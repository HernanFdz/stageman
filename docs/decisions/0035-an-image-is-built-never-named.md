# 0035 — An image is built, never named

## Status
Accepted. Completes one level down the move
`docs/decisions/0034-tools-are-served-not-shipped.md` began.

## Context

`docs/decisions/0012-agents-run-in-containers.md` put every agent in a
container and stated the property that made it worth doing: *the host needs a
container runtime and nothing else.* That is true of the host and not of the
operator. The image an agent runs in is built from a recipe — which this
record also moves inside the crate that names it, to
`agent/images/claude/Dockerfile`, because a package cannot compile in a file
outside its own directory —
under a tag written twice — once in `project.just` and once as a constant in
the agent crate — held together by a test that parses the justfile. Neither the
recipe nor the command that builds it ships with the binary, so what somebody
actually needs is a container runtime *and a clone of this repository*.

0034 dissolved the same shape one level up, and its sentence was that nothing
this project writes is in the image, so a tool can no longer be older than the
instance driving it. The image itself has that defect unaddressed: one built
weeks ago serves a binary built today, silently, because a tag says nothing
about what it names.

The agent crate already states the principle and stops one step short of
it — an image is code, so an image an operator could name is one they could
name wrongly.

Three measurements decide the shape rather than the feasibility.

**A recipe needs no file and no context.** Docker 29.4.1 and Podman both accept
a recipe on standard input — `build -t <tag> -` — and the result is identical
to a build from a context directory: the same eight layers, the same digests,
the same created timestamp. There is nothing a context could carry, because
0034 decided nothing this project writes goes in the image.

**A cached rebuild costs about a second and no network.** 0.7s from standard
input and 1.7s from a context, against a warm daemon with every layer present.
A rebuild compares instructions rather than names, so it is a freshness check
that an existence check cannot be.

**An image does not need a name.** `build -q` prints a content-addressed
identifier and nothing else, on both runtimes, and a container started from
that identifier speaks the protocol — measured end to end, with the exchange in
`agent/images/claude/handshake.json` answered by an image carrying no tag at
all.

## Decision

**The recipe is compiled into the binary, and every container starts from an
image built immediately beforehand and identified by its content.**

- The recipe stays a file and reaches the binary through `include_str!`. It
  keeps its comments, its highlighting and its diffs; what changes is that it
  is part of the artifact rather than beside it.
- A build runs on the path that creates a container, every time, and is
  remembered nowhere. The runtime's layer cache is the memoisation, and it is
  better than one this project could keep: it survives a restart, and it
  notices a changed recipe.
- **Nothing is tagged.** The build yields an identifier, the identifier starts
  a container, and no name exists for anything to get wrong.

Rejected: **a published image, pulled from a registry.** The fastest first run,
and it puts an artifact this project does not control between an operator and
the thing they were told needed only a container runtime. It also re-couples
release to release — a binary built at one commit needs an image published from
that same commit — which is the coupling a compiled-in recipe removes rather
than manages.

Rejected: **a tag naming the version, built locally.** One literal rather than
two, and it keeps the defect: a recipe edited within a version produces no new
tag, so an existence check selects the old image and says nothing. Only
building unconditionally fixes that, and once the build is unconditional the
tag has no work left to do.

Rejected: **a tag naming a hash of the recipe.** Correct, and the first answer
reached here. It buys exactly what building unconditionally already buys, and
it costs a hash function, a name nobody can read, and an orphaned image per
edit.

Rejected: **provisioning the agent at container start** rather than building at
all. Already refused by 0012, for the reason that still holds: installing puts
minutes in front of every signal. A cached build puts a second there.

## Consequences

**What goes**: the tag constant in the agent crate, the tag literal in
`project.just`, the recipe that builds under it, and the test that parses the
justfile to hold those two together. In its place is a cheaper agreement of the
same kind, and one the compiled-in recipe pays for: the stage names this crate
asks for are checked against the recipe text itself, which reads no file and
reaches no second directory.

**What survives, renamed**: the test asserting that a container which never
starts fails as a container rather than as a protocol. It was named for its
commonest cause — an image nobody had built — and that cause is what this
record retires, not the property. Every other way a container fails to start is
untouched, and the classification is what an operator acts on for all of them.

**A second per container, deliberately.** 0012 measured a container start at a
couple of seconds and called it trivial against the work a job does; this is
the same order again, and it is per container rather than per turn. The sweep
at startup pays it once per resumed job.

**An untagged image is a dangling image**, so a runtime's own pruning removes
it. That degrades the next start to slower and never to wrong: the layer cache
survives, so the rebuild is the same second, and a running container's image
cannot be pruned from under it. Clearing the build cache is the same story one
step further out — a cold build, not a failure.

**A build failure is a new thing to report.** `AgentError` distinguishes a
container that never spoke from an adapter that answered badly, and its own
reasoning says a missing image is the most common cause of the first. Missing
becomes impossible and *unbuildable* takes its place: a different failure with
a different fix, and the first one an operator without a network meets.

**Reversing** means putting a tag back and building it out of band. Small in
code, and it re-opens the question of who builds it and when. Nothing is
persisted, so nothing migrates.

**Revisit if** a recipe ever needs a build context — a file this project writes,
copied in — which ends the no-context property and makes standard input
insufficient. 0034 is what currently forbids that, so the two records revisit
together. Revisit also if a cached rebuild stops costing about a second, most
plausibly on a runtime whose cache is not local.
