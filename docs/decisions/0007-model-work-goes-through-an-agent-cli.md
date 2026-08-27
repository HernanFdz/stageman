# 0007 — All model work goes through an agent, never a vendor service API

## Status
Accepted

## Context

Both halves of the system need a model. A job needs one to do the work. The
orchestrator needs one to judge a signal, decide how to react, write the reason
and compose the kickoff prompt.

For the job, running the agent is the whole point. For the orchestrator it is
not obvious: asking a question and getting an answer is exactly what a vendor's
service API is for, and calling it directly is fewer moving parts than spawning
a process.

The deciding fact is billing. A subscription is reachable only by running the
vendor's own agent tool; a service API takes a per-token key, always. So the
choice of transport silently decides what the operator pays, and choosing
differently in the two halves would mean paying two ways at once.

## Decision

Every model interaction runs a configured agent — job work and orchestrator
triage alike. No crate reaches a vendor's service API directly.

This is practical rather than a workaround because agent tools expose
schema-constrained output for headless use: triage gets an answer validated
against a schema, not prose to be parsed hopefully.

Note what this decision is *not* about. It fixes the transport, not the
credential — a configured agent may be paid for by a subscription or by a
per-token key, and that is `docs/decisions/0008-one-credential-per-agent.md`.
Conflating the two produced an earlier draft of this record that would have
forced the operator to configure the orchestrator and jobs separately, which is
exactly the complexity nobody wanted.

Rejected: **a service API for triage, an agent for jobs.** Two transports, two
credential stories, and — the part that makes it indefensible — triage silently
running off-subscription while jobs run on it, with nothing in the system
saying so.

## Consequences

A hard runtime dependency: at least one agent must be installed and
authenticated for anything at all to happen, including watching. That failure
belongs at startup, loudly, rather than at three in the morning on the first
signal that arrives — see `docs/conventions.md` §3.

Triage costs a process spawn rather than a request. At the scale in
`docs/vision.md` §3 that is not a consideration; at some other scale it would
be, and that is what would make this wrong.

Triage and job work draw on the same account, so watching competes with doing
for the same budget. Deliberately not solved by filtering signals before
judging them — see `docs/vision.md` §2.

Cheap to reverse for triage specifically, and expensive for jobs, where the
agent *is* the product. Revisit if agent tools stop exposing schema-constrained
headless output, which is the single property that makes triage through one
tolerable.
