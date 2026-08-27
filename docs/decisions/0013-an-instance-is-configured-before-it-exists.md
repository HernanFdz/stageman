# 0013 — An instance is configured before it exists

## Status
Accepted

## Context

The agent the orchestrator thinks with is not optional. Making it so would have
spread option handling across every caller in order to catch the weaker half of
a failure the startup check in `docs/conventions.md` §3 has to catch properly
anyway: that check can tell an operator their credential is dead, and a missing
value cannot.

That left the question of where a valid instance comes from. Three answers were
available, and two were tried on paper before the third became obvious.

## Decision

There is no empty instance. The state type has no `Default`, and one is either
loaded from a snapshot or built already configured.

On a first run — no snapshot on disk — the process asks, in the terminal,
before the dashboard starts: which agent to configure, and what credential it
should use. It then has a valid instance and proceeds. Every agent after the
first is added through the dashboard, as before.

Rejected: **making the field optional.** Discussed above; it buys handling for a
state a working instance never occupies.

Rejected: **seeding a placeholder agent.** This was written and then removed. It
produced an entry that *looked* configured and was not: an empty credential, and
a program named rather than located, which `docs/conventions.md` §3 forbids
resolving through the environment. Keeping the rule then meant rejecting the
placeholder at startup — so the design created an invalid value on purpose and
added a check to catch it, when it could simply never create one. A test had to
exist purely to pin the placeholder's shape, which is the smell that named the
problem.

## Consequences

The invariant that the orchestrator's agent appears among the configured agents
holds by construction at creation, and is *also* checked when a snapshot is
loaded. That is not redundancy: a file on disk is untrusted input — hand-edited,
half-written, or written by an older version — so the two guards answer
different questions. One says this instance was built correctly; the other says
this file can be believed.

A first run cannot currently be provisioned unattended, because it wants a
terminal. `docs/vision.md` §3 explicitly contemplates a machine nobody sits at,
so this will need the same values accepted from the environment, prompting only
when interactive. Not built yet, and recorded here so that it is a known gap
rather than a surprise on the first server install.

`README.md` changes shape slightly: the first credential is entered in the
terminal, the rest in the dashboard.

Reversal is cheap — the flow is one function and a constructor — but it would
mean reintroducing a state the rest of the code has stopped handling, which is
the expensive half.

Revisit when the first non-interactive install happens, which is the trigger for
the environment fallback rather than for changing this.
