# 0020 — The orchestrator belongs to a project

## Status
Accepted. Supersedes the claim in
`docs/decisions/0012-agents-run-in-containers.md` that triage is a singleton
the instance owns.

## Context

0012 justified running the orchestrator's agent in one long-lived container
like this:

> triage is a singleton service the instance owns, with no workspace, no
> repository and **no project credentials**

The last clause is what carried it, and it is not true. `docs/architecture.md`
§1 says the orchestrator *"holds what it needs in order to watch a project's
channels"*, and the orchestrator crate says the same thing in its own words:
what it holds in order to **watch** is the counterpart of what a job is handed
in order to **act**, and both come from the same project configuration.
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` is where that
arrangement was settled.

So a singleton orchestrator holds *every* project's watching credentials, in
one long-lived container, for as long as the instance runs. That is exactly the
concentration `docs/architecture.md` §2 forbids one level down — a job holds
credentials for its own project and no other, defended by construction — and
nothing defended it here, because nobody had noticed the orchestrator held any.

The isolation the whole design rests on was complete everywhere except the one
component watching all of it at once.

## Decision

An orchestrator belongs to a project. One per project, watching that project's
channels with that project's credentials, in its own container.

## Rejected: keeping the singleton and handing it credentials per signal

One container still, given only the credentials of the project whose signal it
is judging, for the duration of that judgement.

It loses because a long-lived process keeps what it has been handed — in
memory, in whatever the agent caches, in a session that outlives the turn.
Isolation would then rest on the agent forgetting, which is the same courtesy
0012 refused to rely on when it chose containers over worktrees. Rejecting that
argument at the job level and accepting it here would be inconsistent in the
one place that watches everything.

## Rejected: no container until a signal arrives

Start one per signal and stop it afterwards, so nothing is long-lived and
nothing accumulates.

0012 rejected this on cost — a start in front of every signal — and that
reasoning survives. Worth recording that its target was per-*signal* starts,
not per-*project* ones, so this decision does not disturb it.

## Consequences

Idle containers now scale with projects rather than being one. At the scale
`docs/vision.md` §3 describes — one operator, their own machine — that is a
handful, and 0012's cost argument is untouched because the multiplier changed
rather than the reasoning.

**Revisit that if an instance ever watches enough projects for idle containers
to matter.** The answer then is to start one on demand and stop it when idle,
which pays a start only for projects quiet enough that the delay does not
matter — the cost lands exactly where it is cheapest, which is why it is worth
having as the escape hatch rather than as the design.

The instance no longer has an agent to think with, so an instance with no
projects needs nothing configured at all. That reopens a question
`docs/decisions/0013-an-instance-is-configured-before-it-exists.md` closed, and
`docs/decisions/0021-an-instance-starts-empty.md` answers it.

`app` runs orchestrators rather than an orchestrator. Their failures and their
shutdown become several things to supervise instead of one, which is real work
and not merely a plural.

Reversing is cheap in code and would reintroduce a credential concentration
nothing else in this design tolerates.

Revisit if watching a channel ever stops needing a credential — a public feed,
or a webhook an instance receives rather than polls. Then 0012's premise
becomes true rather than assumed, and a singleton becomes defensible again.
This decision rests entirely on watching being authenticated, and it is worth
knowing that it does.
