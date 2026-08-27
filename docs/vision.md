# Vision

What this project is for, and what it will never be. Read this before deciding
*what* to build.

Three documents, three questions, and the rule that keeps them from drifting
into one another: if a sentence would change what a user **types**, it belongs
in `README.md`; if it would change what gets **built**, it belongs here; if it
would change where **code goes**, it belongs in `docs/architecture.md`. A
sentence that answers two of those is two sentences.

**Each section says what belongs in it and starts empty.** That prose is a
standing rule, not a placeholder: it stays when the section fills up, because it
governs what gets added next. Only the HTML-comment example and the
`_(none yet)_` marker are disposable.

## 1. The problem

Whose problem this is, and what it costs them today. State it without naming the
solution — a problem defined in terms of its answer cannot rule anything out
later, which is the one job this section has.

The loop is still open.

A coding agent can carry a well-scoped piece of work from description to
finished change. What it cannot do is start. Every unit of work begins with a
person noticing that something needs doing, judging that an agent could handle
it, and then sitting down to say so. Capability is high and initiative is zero,
so the amount of work that gets done is bounded by how much attention a human
has left over — not by what the agent could have done.

The information needed to make that judgement is already being produced, and
already being written down. Issues get filed, pull requests get reviewed,
exceptions get reported, people describe problems to each other in chat. A
person reads those and decides what is worth doing next. Nothing else does. So
the signal sits there, fully formed, until somebody has time to act on it — and
overnight, when nobody does, nothing happens at all.

## 2. What it refuses to be

The non-goals, each with its reason.

This is the section that pays for itself. Without it every plausible feature
looks like an oversight, and sooner or later somebody helpfully adds one — a
deliberate absence that is not written down is indistinguishable from a gap.

- **Not a deployment system.** Work terminates at a proposal a human reviews.
  Nothing is merged or shipped unattended, and this is not a configurable
  policy — it is the reason the rest is safe to leave running. Because the
  boundary is absolute rather than conditional, no credential that can land code
  or reach production has to exist anywhere in the system, so the worst outcome
  of a bad autonomous decision is a bad proposal nobody accepts.
- **Not a coding agent.** The agent that does the work is somebody else's
  product, on somebody else's release cadence. Building one would mean owning
  the largest and fastest-moving part of the problem in order to reach the part
  that is actually missing, which is the deciding rather than the doing.
- **Not multi-tenant, and not a hosted service.** One operator, one instance,
  their own machine and their own credentials. Several *projects* per instance
  is a requirement; several *customers* per instance is not, because isolating
  tenants costs more than running a second copy.
- **Not a queue.** Work is never retried and never resumed. A queue's guarantees
  rest on re-executing a unit of work being safe, and an agent editing a
  repository is not that: a second attempt starts from a world the first one
  already changed.
- **Not a chat client.** The dashboard operates the instance and never becomes a
  second place to talk to a running agent. A conversation with two front doors
  needs the same state behind both, forever, in exchange for saving one click
  during the hours somebody is at the desk anyway — and the entire point is the
  hours when nobody is. See
  `docs/decisions/0005-conversation-happens-on-channels.md`.

## 3. The constraint that shapes everything

The one fact the downstream decisions keep bumping into — a scale, a deadline, a
platform, a person, a budget. Naming it once here saves re-arguing it in every
decision record, and makes the records shorter for citing it.

**It runs on infrastructure the operator controls** — a long-lived process on
their own machine, holding their own credentials. Nothing is delegated to a
service somebody else operates.

That one fact settles a surprising number of arguments before they start. There
is a long-lived process at all because something has to keep watching while
nobody is looking. State is a local file rather than a managed database, and
credentials are held rather than referenced, because there is no remote side to
hold them. There is no account, no tenancy model and no billing boundary,
because there is exactly one operator and they already have root on the box. And
the process will be killed underneath running work — by a reboot, an upgrade, a
closed laptop lid — which makes surviving that a design requirement rather than
an edge case.

Revisit if it ever has to serve people who do not administer the machine it runs
on. At that point the honest answer is a different product, not a flag.
