# 0006 — Agents are pluggable, and the contract has its own crate

## Status
Accepted. Supersedes the rejection of a pluggable agent contract in
`docs/decisions/0001-drive-an-existing-coding-agent.md`, and the crate count in
`docs/decisions/0003-four-crates-around-a-core.md`.

## Context

0001 rejected a pluggable agent contract and, unusually, named the condition
under which to build one: *"if a second agent is ever worth supporting, the
seam will be visible then and not before."*

That condition has arrived — at design time rather than later. Working with
several coding agents is a product commitment rather than a possibility: one is
tried first, others follow, and which one runs a given piece of work is a choice
the system makes rather than a constant compiled into it.

Two crates need to run an agent, for different shapes of work. The orchestrator
asks one-shot questions and needs a structured answer. A job runs a long
session inside a workspace and needs to surface what that session asks for.

## Decision

Nothing outside an adapter is specific to one agent, and the contract plus its
adapters live in their own crate.

The contract covers both shapes: a one-shot structured query, and a session
bound to a workspace. Configured agents are instance configuration, and each
carries a description of what it is good for — not decoration, but the thing
the orchestrator reasons over when choosing one. Which agent ran a job is
recorded on the job, because "why did this go badly?" is unanswerable without
it once more than one can run.

Rejected: **keeping the abstraction out until a second agent actually ships.**
That is what 0001 chose, and it was right while a second agent was a
possibility rather than a plan. It loses now for the reason 0001 itself gave —
the seam is visible, because we already know two consumers and several
implementations exist.

Rejected: **putting agent invocation in the job crate and routing the
orchestrator's triage through it.** It avoids a new crate, and it fails on the
rule in `docs/architecture.md` §1: the orchestrator would have to name the job
crate, which is the one direction that must stay closed.

## Consequences

Five crates rather than four. 0003's reasoning is untouched — dependencies
still point inward, and the orchestrator and job still cannot name each other —
only its count is superseded.

The abstraction can still end up shaped like the first agent, wearing a
costume; that risk was real when 0001 named it and is not cancelled by having
decided to take it. The mitigation is a second adapter early rather than
eventually, while the contract is still cheap to move.

Reversing means merging one crate back and inlining the adapter, which is
mechanical. What is expensive to reverse is the *commitment* — being agnostic
is a promise to users, and withdrawing it is a product change, not a refactor.

**Two clarifications added once this met code.** *"Nothing outside an adapter is
specific to one agent"* is about behaviour, not about knowing the roster: the
domain crate names the closed set of agents that exist, and adapters know how
each behaves. The set has to be closed because every agent needs an adapter and
an image, both compiled in — so a value an operator could invent only postponed
the failure to runtime, while an enumeration makes adding one a compile error
everywhere it is not yet handled. And the description an agent carries lives in
code rather than in configuration: it describes the agent, not the
installation, and nothing an operator could edit would make it more true.

Revisit if, after two adapters exist, the contract has become a union of vendor
quirks rather than a shape they share. That would mean the seam is in the wrong
place, and it is better answered by moving the boundary than by widening the
contract again.
