# 0018 — Diagnostics are emitted through tracing; where they go is still open

## Status
Accepted. Answers one half of the logging question in `docs/open-questions.md`
and deliberately leaves the other half open, which is the point of it.

## Context

Two places now produce diagnostics with nowhere to send them. A snapshot write
that fails inside a `Drop` can only be reported, never returned. A startup
sweep finds a container nothing in the instance claims, and an operator is the
only one who can act on it. A third would make this a habit rather than a pair
of placeholders, and both currently go to standard error described in a comment
as not yet a choice.

The open question named `tracing` as the leading candidate and said the harder
half is *where output goes* — `docs/decisions/0005-conversation-happens-on-channels.md`
has the dashboard showing logs, so writing to a terminal nobody is attached to
is not enough. It said this would be settled by the first thing that needs to
*read* a log rather than write one.

That is still true, and it has since been sharpened into the reason it is hard:
**instance-wide and project-level output are two concepts, not one with two
sources.** A snapshot that will not write is a fact about the installation. What
a job's agent says is about one project's work. They differ in who reads them,
when, and what for.

Both of those are questions about *routing*. Neither is a question about how a
call site says something happened.

## Decision

Diagnostics are emitted through `tracing`. A subscriber writes to standard
error. Where output goes in general stays open.

The distinction that makes this safe to take now is the one `tracing` is built
around: emitting and routing are separate. A call site states what happened and
in what context; a subscriber decides where that goes. So adopting the library
settles nothing about the dashboard, nothing about whether the two concepts
share a mechanism, and nothing about channels — and it is what makes those
answerable later without revisiting every place that ever logged anything.

## Rejected: standard error at every call site until the dashboard exists

What is there now, and it survives exactly as long as there is one such site.
It hardcodes the routing *into* each call site, so the deferred decision gets
paid for by finding and rewriting all of them — which is the cost of deferring
badly rather than the cost of deferring. Deferring well means the call sites do
not encode the answer.

## Rejected: the `log` facade

Smaller, and adequate for a line of text. It loses on the thing the deferred
question will need: `log` has no spans, and a span is what carries *which job,
which project* alongside the message. Routing project-level output apart from
instance-wide output means having that context at the point of routing, and a
facade that discards it would have to be replaced to answer the question this
record is deliberately postponing.

It is also not a smaller dependency in practice. `tracing` is already in this
project's tree, pulled in by the protocol library, so `log` would add a second
vocabulary rather than avoid a first.

## Rejected: deciding the routing at the same time

Tempting, because a subscriber has to be installed anyway and it feels
half-finished to install one without deciding what it is for. It loses for the
reason the open question gave originally: the reader does not exist yet.
Designing where output goes against an imagined dashboard is how the two
concepts get answered with one mechanism by accident.

## Consequences

Standard error is a real answer for a process an operator started and is
watching, which is what the binary is today, and a placeholder for a daemon
nobody is attached to, which is what it becomes. Those are different claims
about the same line of code, and only the second is deferred.

**Adopting one library is not adopting one destination.** Instance-wide and
project-level output remain two concepts; spans and targets are how they can be
routed apart later, and nothing here says they should share a subscriber. A
record that quietly turned "we log through tracing" into "logs are one thing"
would have decided the harder half by omission.

A span per job is the shape the routing question will want, since it is what
attaches a message to the work that produced it. Not built here, because
nothing yet runs a job long enough to need one, and recorded so that the first
thing that does knows where it is going.

Reversal is mechanical: call sites say what happened rather than where it goes,
which is the property being bought.

Revisit when something needs to *read* a log — the dashboard, or a channel
carrying a job's question to a human. The question then is routing, and this
record is deliberately not an answer to it.
