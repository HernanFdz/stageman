# 0005 — Conversation happens on channels, not in the dashboard

## Status
Accepted

## Context

Two surfaces face a human. The dashboard operates the instance: add a project,
set its credentials, watch jobs, read logs, kill one that has gone wrong. Slack
is where a job escalates when it hits something it cannot decide, because that
is where the human already is.

What was left open was whether both could answer. A job that asks a blocking
question is idle until somebody replies, and the dashboard is right there in the
browser — so letting it reply looks like an obvious convenience rather than a
second design.

## Decision

A job's questions go out on a channel and are answered on that channel. The
dashboard never carries a conversation with a running job.

Stated as the durable rule rather than the current instance: **conversation
belongs to channels; the dashboard is a console.** Slack is the first channel
implemented and currently the only conversational one, but the boundary is about
which surface owns a conversation, not about which vendor is on the other end.

Rejected: **both surfaces can answer.** A conversation with two front doors
needs the same state behind both — what was asked, what was already said, who
replied first, what happens when two people answer at once. That cost is paid
forever, on every future control, in exchange for saving a human one click
during the few hours a day they are at the desk anyway.

Rejected: **the dashboard answers and Slack only notifies.** It inverts the
constraint in `docs/vision.md` §1: the whole point is that work continues while
nobody is watching a screen, and this puts the reply behind the screen.

## Consequences

Slack is not an optional integration. It is the escalation path, and without it
a job that hits a question it cannot answer has nowhere to put it — which is why
it is the first channel implemented rather than the most useful one. A project
with no conversational channel bound to it can only run work that never needs to
ask.

The dashboard stays a console, and that is now a design rule rather than a
current limitation: a control that turns out to need a conversation is a signal
the control is in the wrong place. This is what
`docs/architecture.md` §1 means when it says the app crate operates the instance
and does not talk to jobs.

Reversing this is not a refactor. It means conversational state in the app
crate, a resolution rule for concurrent answers, and a second implementation of
every escalation the channels already handle.

Revisit if a second conversational channel arrives — the rule that survives is
one conversational surface per job, and at that point "which channel does *this*
project talk on?" becomes a real question with a real answer. Revisit also if an
operator cannot use Slack at all, which makes the escalation path, not the
dashboard, the thing that needs a second implementation.
