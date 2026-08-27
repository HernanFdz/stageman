# 0012 — Agents run in containers, including the orchestrator's own

## Status
Accepted. Answers the isolation question that was open in
`docs/open-questions.md`.

## Context

That question asked what a workspace is mechanically — a container, or a git
worktree — and said it should be decided on isolation alone. Evidence
accumulated on one side without ever quite closing it: a containerised process
answers requests over standard input and output with networking disabled
entirely; an image can pin an agent's version against tools that update
themselves; and both mitigations deferred by
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` are far
easier inside a container than outside one.

What closed it was a smaller problem. An agent's program lives at a path that
differs per machine, so its location has to be configuration — a rule
`docs/conventions.md` §3 states because a daemon that searches for its agents
works when tested by hand and fails when a service manager starts it. Inside a
container built with the agent already installed, that path is decided by the
image and stops being a property of the host at all.

## Decision

Every agent runs in a container. Not only a job's agent — the one the
orchestrator thinks with, too.

The image carries the agent and the platform tools a job needs, installed at
build time rather than provisioned at spawn; installing on every start would
put minutes in front of every signal. Credentials and the workspace arrive at
`run`, because baking a credential into an image puts it in a layer and shares
it with every project.

**The host needs a container runtime and nothing else.** No agent installed, no
`gh`, no toolchain — which is the largest single simplification in this
decision, and it is a deployment property rather than an isolation one.

The orchestrator's agent runs in one **long-lived** container; a job's runs in
one **per job**. That is not an inconsistency needing an excuse: triage is a
singleton service the instance owns, with no workspace, no repository and no
project credentials, so per-signal containers would buy nothing and cost a
start every time. A job is an ephemeral unit of work whose isolation is the
whole point.

Rejected: **git worktrees.** Nearly free and instant, and they leave the
invariant in `docs/architecture.md` §2 resting on the agent choosing to respect
it — the agent's own boundary check, observed during a spike, is exactly that:
a courtesy. They also leave the path problem unsolved and both deferred
mitigations impractical.

Rejected: **containers for jobs, the host for triage.** It keeps triage fast and
reintroduces every host-specific concern for one component — an installed
agent, a machine-specific path, an agent that updates itself underneath a
long-running process. Paying that for one of the two consumers is worse than
paying it for neither.

## Consequences

`docs/conventions.md` §3's rule does not disappear, it retargets: the thing
whose location must be configured is now the container runtime.

A job pays a container start — a couple of seconds, measured. Trivial against
the work a job does, and worth stating because it is per job rather than
amortised.

An image has to exist before anything runs, so the proof of concept is larger
than it would have been: build an image, run a container, speak the protocol
through it. That is the real thing rather than a throwaway, and the transport
was proven before this was decided rather than after.

Bind-mounted filesystem performance is meaningfully worse on a platform whose
containers run in a virtual machine, which is most developer machines. It may
be better to clone a repository inside the container than to mount one from
outside; that is an implementation question, not a decision.

The egress allowlist deferred in 0009 becomes straightforward rather than
theoretical, which moves the remaining unmitigated risk closer to being
addressed than it was.

Reversing this means reintroducing host-installed agents and per-machine paths,
which is the rejected option's cost paid later.

Revisit if container start ever stops being trivial against the work a job
does — the shape of that would be many small jobs rather than few large ones,
which is not what this system is for.
