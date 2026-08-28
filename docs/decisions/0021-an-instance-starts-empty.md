# 0021 — An instance starts empty

## Status
Accepted. Supersedes
`docs/decisions/0013-an-instance-is-configured-before-it-exists.md`.

## Context

0013 held that there is no empty instance: the state type has no `Default`, and
one is either loaded from a snapshot or built already configured. Its reason
was that the agent the orchestrator thinks with is not optional, and that
making it so would spread option handling across every caller in order to
catch the weaker half of a failure a startup check has to catch properly
anyway.

That reasoning was sound and it rested entirely on there being an
instance-wide orchestrator. `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`
removed one. With no projects there is no orchestrator, and therefore nothing
for the instance to think with — so there is nothing an empty instance is
missing.

0013 also recorded a gap: a first run wants a terminal, while `docs/vision.md`
§3 contemplates a machine nobody sits at. That was later closed by accepting
the same answers from the environment — a workaround, for a question that
turns out not to need asking.

## Decision

An instance starts with nothing: no agents, no projects, and no container
runtime. Everything is configured through the dashboard, and `State` has a
`Default` again.

The invariant 0013 protected is not lost, it moves. **A project names one agent
for its orchestrator and a non-empty set of agents its jobs may use, and every
one of them is configured.**

It is *checked* rather than made unrepresentable, and that was settled the
second way round. A wrapper type refusing an empty set was written first and
then removed: it enforced one of the two conditions and left the other — an
agent removed while a project still named it — needing a check anyway, so it
bought ceremony at every construction site in exchange for half the property.
One function, asked wherever a state might have stopped being valid, is smaller
than a type plus a check — and it leaves one definition of *valid* rather than
two that can drift apart.

Where it is asked is not the domain's business, and the first attempt got that
wrong by making sealing refuse. Sealing is cryptography; whether a state makes
sense is a different question, and conflating them had `seal` returning an
error that has nothing to do with a cipher. A file is checked as it is read,
because it is untrusted input. A state is checked before it is written, by the
store rather than by the domain, because a file that will not open is a worse
outcome than a write that refused.

An agent's configuration may not be removed while a project names it.
Historical jobs are unaffected, because `docs/conventions.md` §2 already has a
job storing its agent by value so that removal cannot rewrite the record of
work already done.

## Rejected: keeping a first run for the container runtime alone

The runtime's path is machine-specific and
`docs/decisions/0017-the-runtimes-path-is-recorded-in-the-instance.md` keeps it
in the instance, so there is a case for asking about it before anything starts.

It loses because it keeps an entire flow alive — a terminal prompt, an
environment fallback, and the headless gap that comes with both — for a single
value the dashboard can ask for as easily. And it makes "an instance starts
with nothing configured" *nearly* true, which is worse than either extreme: a
rule with one exception is a rule every reader has to remember the exception
to.

## Rejected: making the instance's agent optional

What 0013 rejected, for the reason it gave: an `Option` spreads handling across
every caller for a state a working instance never occupies. Still right, and
now moot — the field is gone rather than optional.

## Consequences

The first-run flow goes, and the environment variables that provisioned it
unattended go with it. 0013's headless gap closes by ceasing to exist rather
than by being handled, which is the better of the two ways to close a gap.

**`docs/conventions.md` §3's startup rule narrows.** A missing container runtime
fails at startup *when one is configured*; an instance that has none is not
unusable, it is empty. What must not happen is a project being created against
a runtime nothing has verified, so that check moves to where the requirement
begins rather than being dropped.

The set of agents a project may run jobs with is separate from the one its
orchestrator thinks with, and **the orchestrator's agent need not be among
them.** Triage and work are different tasks: a cheap agent judging signals and
a capable one doing the work is a configuration somebody will want, and a
constraint forbidding it would be an accident rather than a decision.

Reversing means asking a question on a fresh machine again, which is cheap in
code and reintroduces something nobody can answer before they have seen the
dashboard.

Revisit if an instance ever needs something to be true before any project
exists. That would be a fact about the installation rather than about the work,
and it would want somewhere deliberate to live rather than being asked for at
the first opportunity.
