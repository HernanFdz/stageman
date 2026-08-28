# 0024 — The agent provisions what a project needs, inside its own container

## Status
Accepted. Completes
`docs/decisions/0019-a-projects-tooling-is-the-projects-business.md`, which
settled who decides and left who provides open.

## Context

0019 said that stageman does not decide what tools a project needs and does not
install them: a project declares its own prerequisites, in the place whoever is
about to work on it will read. That is a rule about *deciding*, and it left a
gap its own text names — declaring a prerequisite does not put a toolchain in a
container.

The gap was recorded as an open question and stayed open because nothing had
run against a second project. What it asked was whether a project may carry an
image of its own, and it listed two shapes: an image per project built from an
agent's as a base, or a project naming packages that get installed when its
container starts.

`docs/decisions/0012-agents-run-in-containers.md` is the other constraint. It
put agents in images because installing on every start puts minutes in front of
every signal — an argument about *triage*, which is on a signal's critical
path, and which the open question already noted is weaker for a job, since a
job is not.

## Decision

**One image, project-agnostic. Starting a job means starting a container from
it, and the agent inside sets up whatever that project needs before working on
it.**

Nothing about a project is baked into an image, and nothing about a project is
installed by stageman. Both halves of 0019 now say the same thing: the project
declares, and the agent — which reads the declaration, because that is what
reading a repository is — acts on it.

The distinction from the second shape the open question listed is worth
stating, because they look alike and are not. A project naming packages that
*stageman* installs at container start is stageman provisioning per project,
which is what 0019 forbids; it merely moves the list from an image to a field.
What happens instead is that the agent does it, as part of the work, from the
repository's own instructions — which is what a person picking the project up
would do.

## Rejected: an image per project, built from the agent's as a base

The shape that makes a job start fast, and the one the open question called
correct-but-multiplying.

It loses on who maintains them. Every project gets an image somebody has to
build, keep current with its agent's base, and rebuild when either moves. That
is a second artefact per project, kept in step by hand, and stageman has no
place to store one or any business building it. It also reintroduces per-project
knowledge into this system in exactly the form 0019 removed.

Worth naming the thing it was right about: a job that installs a toolchain pays
for it every time, and an image pays once. That cost is real and is accepted
below.

## Rejected: a project declares packages, stageman installs them at container start

The other shape the open question listed, and the closer of the two.

It loses because it is 0019's rule with the word *stageman* quietly back in it.
Something here would have to know what a package is, which package manager to
use, and what to do when installation fails — three things that differ per
project and per ecosystem, and that a project's own instructions already
describe for humans. It buys speed over the chosen shape only by moving work
from the agent to the platform, and the platform is the side that cannot read.

## Consequences

**A job pays installation time, every job.** That is the cost 0012 refused for
triage and accepts here, on the reasoning 0012 itself gives: triage is on a
signal's critical path and a job is not. Revisit if job startup ever becomes
the thing somebody is waiting on.

**An agent that cannot set the project up still opens a pull request.** This is
the concern the open question raised and it is not resolved by answering the
question — it is inherited. What stands today is that the proposal says so: the
first one this system made was unprompted about what it had not been able to
verify. That is honesty rather than a mechanism, and it hands verification back
to a person, which is the work this system exists to remove. It is the strongest
argument for a per-project image and it did not win, so it is written down here
rather than lost.

**A project whose prerequisite is a container runtime cannot be provisioned
this way.** This one is not hypothetical: it is stageman. Since
`docs/decisions/0023-the-container-runtime-is-discovered-once.md`, `just check`
requires a container runtime, so an agent working on this repository must run
containers inside its own — and installing a client is not the same as having a
runtime, which was measured: a container with the Docker CLI installed answers
`docker --version` and fails `docker version`, because the second reaches a
daemon and the first does not.

What that costs, and how to pay it, is a question this record deliberately does
not answer; it is in `docs/open-questions.md` with the measurements. This
decision is about who provisions, and the answer does not change because one
project's prerequisite is hard to provide.

Reversing means building an image per project, which is a build pipeline and a
place to keep images rather than a code change — expensive in operations,
cheap in this repository, since nothing here would have to be deleted.

Revisit when job startup time is what somebody complains about, or when a
project's setup is expensive enough that repeating it per job is absurd — a
large compiled dependency tree is the obvious candidate. The answer then is
probably a cache rather than an image, because a cache keeps this decision and
an image reverses it.
