# 0001 — Drive an existing coding agent, and host its tools

## Status
Accepted. The rejection of a pluggable worker contract below is superseded by
`docs/decisions/0006-agents-are-pluggable.md` — that record fires the revisit
condition this one names, rather than contradicting it. The decision to host the
agent's tools is superseded by
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`. What remains
accepted is the part the title names: drive an existing coding agent rather than
write one.

## Context

The missing piece in `docs/vision.md` §1 is initiative, not capability. A coding
agent can already take a well-scoped piece of work to a finished change; nothing
starts it. So the question at the outset was how much of the agent this project
should own in order to reach the part that is actually absent.

Coding agents are also the fastest-moving software in this stack. Their
interfaces, their tool sets and their failure modes change on a cadence nobody
here sets.

## Decision

An agent inside a job is a third-party product, driven as a process. This
project implements no agent loop, no context management and no tool-calling
cycle of its own.

It does own the *tools* that agent calls to reach the outside world. GitHub,
Slack and anything else is reached through a tool this project hosts, never
through a credential handed to the agent — which is what makes the first
invariant in `docs/architecture.md` §2 possible at all.

Rejected: **writing our own agent loop.** It would buy exact control over
stopping conditions, budget and escalation, and it was tempting precisely
because the default behaviour of an agent CLI is wrong here — those stop early
to talk to a terminal nobody is watching. It lost because it means owning the
largest and fastest-moving component in order to fix the smallest, and because
the same escalation problem is solvable at the tool boundary.

Also rejected: **a pluggable worker contract**, an interface any agent could
satisfy. Designing an abstraction over one implementation produces an
abstraction shaped exactly like that implementation, wearing a costume. If a
second agent is ever worth supporting, the seam will be visible then and not
before.

## Consequences

The agent's stopping conditions, budget behaviour and failure modes are
inherited rather than chosen. Steering happens through the prompt, the
environment and the hosted tools — which is why prompt text is treated as
load-bearing in `docs/conventions.md` §4 rather than as strings.

Reversing this means writing an agent, so it is not a refactor. What is cheap to
change is *which* agent and *how* it is spoken to; that boundary is confined to
the job crate on purpose, and the protocol question is still open in
`docs/open-questions.md`.

Revisit if the interface a job needs — pushing a message into a running agent,
and getting a blocking question answered somewhere other than a terminal —
turns out not to exist in any agent worth driving. That is a capability
question, not a preference, and it would invalidate this outright.
