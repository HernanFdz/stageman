# 0051 — An image is named by the recipe it is built from

## Status
Accepted. Reverses the *nothing is tagged* half of
`docs/decisions/0035-an-image-is-built-never-named.md`, and keeps everything
else that record decided. It also replaces the mechanism in
`docs/decisions/0036-a-foremans-image-is-not-a-jobs.md` — two stages of one
file become two compositions of four fragments — while keeping that record's
decision intact.

## Context

0035 rejected exactly this design, in one paragraph:

> Rejected: **a tag naming a hash of the recipe.** Correct, and the first
> answer reached here. It buys exactly what building unconditionally already
> buys, and it costs a hash function, a name nobody can read, and an orphaned
> image per edit.

The first clause is the one that was wrong, and it was wrong about the runtime
rather than about the reasoning. *An orphaned image per edit* assumed that two
builds of identical bytes produce one image. They do not.

**Measured, on Docker 29.4.1 with the containerd image store.** Three cached
builds of the same recipe, each taking about 0.15s and reaching no network,
produced three different image identifiers. Setting the reproducible-build
timestamp the frontend honours — as an environment variable and as a build
argument — changed nothing. The images
differ only in an export timestamp and a builder reference, and the runtime
deduplicates neither. So the cost was never an orphan per *edit*. It was an
orphan per *container*, which is what an operator running this for a fortnight
actually found: 37 untagged images, one per job ever started, each holding
about 24MB of its own on top of the layers they all share.

**Two further measurements decide the shape rather than the direction.**

*Asking whether an image exists costs 0.02s* against 0.15s for a cached build,
on a warm daemon. Cheap enough that neither number decides anything, which
matters because the choice below is not about speed.

*Rebuilding under a name that already exists is destructive.* With a container
created from image A under name N, building N again moves the name to a new
image B **and deletes A**, leaving that container's recorded image resolving to
nothing. A subsequent unforced removal of N then succeeds, because the
container no longer appears to be using it — where the same removal attempted
before the rebuild is refused by name. So a name plus an unconditional build is
worse than either alone: it severs containers from their images and switches
off the runtime's own protection against removing an image in use.

## Decision

**Every image is named `stageman:<sha256 of its recipe>`, built only if no
image of that name is already here, and a recipe is composed from fragments
rather than written per agent.**

- **The name is the digest of exactly the bytes handed to the build.** Nothing
  else goes into it, so the name has a definition outside this binary: `cat`
  the fragments into `shasum -a 256` and you have it. One repository for all
  of them, so a listing of what this project has built is one query.

- **A build happens only when that name is absent.** This is the reversal, and
  it gives up nothing 0035 wanted. That record built unconditionally because a
  rebuild *compares instructions rather than a name*, so it noticed an edited
  recipe where an existence check on a fixed tag could not. A name derived from
  the instructions makes the two the same test: an edited recipe asks for a
  name nothing has, and an image found under a name was built from those bytes.
  Building anyway is not merely wasteful but destructive, per the measurement
  above.

- **Builds are serialised within a process.** Two containers starting together
  would otherwise both find the image absent, and the second build would delete
  the first's image from under a container just created from it.

- **A recipe is the concatenation of four fragments**: a base, the fragment
  installing one agent's adapter, the fragment that holds a container open,
  and — for a job only — the fragment that reaches a repository. Concatenated
  with nothing inserted, so a job's recipe is a foreman's byte for byte plus
  one layer, which is what makes the two images share every layer but the last.

- **The base is this project's, and one base serves every agent.** An adapter
  brings what installs it and nothing about the operating system.

- **What nothing needs is reclaimed.** At startup, after the containers are
  settled, every image under this project's name that no container is using is
  removed — unforced, so the refusal is the runtime's own and is decided
  atomically against containers this instance cannot see. The images the
  current fragments hash to are kept.

Rejected: **keeping 0035 exactly, and pruning dangling images instead.** The
runtime's own `image prune` does reclaim them, which is what 0035 relied on.
It is refused because it is not ours to run: it removes every dangling image on
the machine, including ones belonging to work that has nothing to do with this
project, and a daemon that quietly deletes an operator's other build artifacts
is worse than one that accumulates its own.

Rejected: **a name per agent and role, such as `stageman:claude-job`.** Shorter,
readable, and it reintroduces precisely the defect 0035 removed: a recipe
edited within a version produces no new name, so the existence check selects a
stale image and says nothing. Under a content-derived name that cannot happen.

Rejected: **a template with substitutions instead of fragments.** One file per
agent with a placeholder for the base image. It moves a Dockerfile fact into
Rust, and the tracked file stops being something a runtime could build or a
linter could read until it is rendered.

Rejected: **a base image chosen per agent.** The honest version of "an adapter
knows what it needs to run on", and it multiplies the container tests that
matter most. The fragments after the base assume what is in it — a shell, a
package manager reading Debian's lists, the roots the platform layer installs
onto — so a base per agent makes every one of those assumptions a thing to
re-establish per agent, and the tests establishing that the composition works
would run once per agent rather than once. The adapter's own tests multiply by
agent either way; these need not.

## Consequences

**An image per recipe rather than per container.** Every container built from
the same fragments shares one image, which is the whole point. What still
accumulates is one image per *recipe edit*, and the sweep is what reclaims
those.

**Existing untagged images stay, and are the operator's to remove.** This
project cannot recognise them: they carry no name, and the containers pinning
them are the only evidence they were ever ours. `docker image prune` reclaims
the ones nothing is using, and it is a command a person runs rather than
something this does on their behalf, for the reason the rejected alternative
above gives.

**0035's freshness property survives, moved.** A stale image is impossible
because a name cannot outlive its contents, rather than because a build runs
every time. What is given up is the second-per-container that record was
willing to pay; what is gained is that the payment bought an orphan.

**Two of the four fragments are not valid Dockerfiles on their own**, since
they name no image to build on. That costs editor support: tooling that claims
a file by name will report a fragment as a broken Dockerfile, so the fragments
are named so that it does not claim them, and they lose highlighting with it.
The base keeps both, and it is the one most likely to be edited by somebody who
is not sure what they are doing.

**No single file shows a whole image any more.** A reader assembles one from
four, and the order lives in code. The order is checked without a runtime —
exactly one `FROM` and it comes first, a job's recipe extending a foreman's,
every fragment ending where the next can begin, and the holding command named
by the one fragment that exists to name it.

**The `--target` flag and the two stage names go**, with the test that held the
stage names in the recipe and the crate in agreement. Their replacement is
cheaper and of the same kind: the composition is checked against itself, and a
composition in the wrong order fails at the runtime immediately, because a
recipe whose first instruction is not a `FROM` is refused.

**Reversing** means deleting the `--tag`, restoring the unconditional build and
putting the stages back. Nothing is persisted, so nothing migrates — but every
image built under a name would be left named, and every container created from
one would go on referring to it.

**Revisit if** an adapter appears that cannot live on the shared base, which is
the trigger for changing the base for everybody rather than for forking it; if
a runtime is met that deduplicates identical builds, which would make the name
unnecessary rather than wrong; or if the number of images a recipe edit leaves
behind stops being reclaimed by the sweep, which would mean containers are
being kept far longer than the recipes they were built from — the retirement
question in `docs/open-questions.md` arriving from a new direction.
