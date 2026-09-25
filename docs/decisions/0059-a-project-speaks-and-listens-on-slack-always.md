# 0059 — A project speaks and listens on Slack, always

## Status

Accepted. Narrows `docs/decisions/0027-a-channel-is-not-a-platform.md`, which
let a project go without a binding, and
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md`, which let a binding
go without the credential that listens. Neither is reversed: a channel is
still not a platform, and a binding is still two credentials of which one
never reaches a job.

Since `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md` a binding may be a workspace of the instance's app, its two
credentials held by the app and by the workspace rather than both by the
project; a project is still created with one, in either shape.

## Context

`docs/decisions/0005-conversation-happens-on-channels.md` made Slack the
escalation path and, in the same breath, optional: a project with no channel
"can only run work that never needs to ask", and that was allowed. 0029 then
made the app-level token optional too, on the grounds that a project which
speaks and is never answered "is exactly what existed before".

Three things since have made both allowances cost more than they buy.

`docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md` made
*failed* a state a job waits in rather than ends in, and a reply on its thread
the only thing that puts it back to work. A job on a project that cannot be
answered is therefore a job nobody can repair, and the dashboard would need a
control of its own to do what a reply already does.

The instruction a job begins from had two forms, one naming the tool that
speaks and one not, each held to a snapshot of its own — twice the text for
a case nothing real ever exercised, since every project this has been run
against has been bound with both credentials from the day it was created.

And the routing rule was carrying an unbound project as a case: a message on a
room nobody binds went to nobody, which was also what a mention in the wrong
room looked like, so one branch served two meanings.

## Decision

**A project is created with a Slack binding, and the binding carries both
credentials.** The form refuses anything less, the instance refuses a project
drafted without one, and starting a job on a project that has none is refused
rather than begun with nowhere to speak. The kickoff has one form.

**Opening an instance file does not enforce it.** A file written by the last
release may hold a project with no binding, or a binding with no credential to
listen with. Such a project is kept, its incomplete binding is dropped, and
the daemon says so at startup by name, because the dashboard is where the
repair happens and refusing to open the file would put the repair behind the
door it locks — the rule in `docs/conventions.md` §3.

Rejected: **requiring the binding in `State::check`**, so that an instance
without one is inconsistent. It is the cheap way to make the type say what is
true, and it is exactly the trap §3 names: an operator upgrading an instance
with one unbound project would find nothing starts, projects and agents
included, over the one thing they could have fixed in a minute.

Rejected: **keeping the credential that listens optional.** A one-way project
was a working configuration when a job asked and stopped and nothing carried
an answer back. It is not one now: a job that asks on such a project waits
for an answer that has no way to arrive, and looks exactly like a job whose
answer is on its way.

Rejected: **a second conversational channel instead of requiring this one.**
0005 already names that as the answer for an operator who cannot use Slack at
all, and nothing has changed about it; but it is a different feature, and
requiring what exists does not foreclose adding what does not.

## Consequences

**A project can no longer run silent work.** Every job has somewhere to
speak, so the paragraph of the kickoff that hedged on it is gone, and a job
that stops with a question is always a job somebody can answer.

**The dashboard's form gains no field and loses two words.** Both credentials
were already asked for; what changes is that neither is marked optional and
the control that submits stays disabled until all three are given.

**A job refused for lack of a binding says so.** That is a new way to fail
to start, and it can only happen on a project that predates this record.

**Reversing** is an `Option` back on the binding, one branch in the form and
one in the routing rule; no data changes, because the file's shape is the
same either way.

**Revisit if** an operator genuinely cannot use Slack, which is 0005's own
trigger and makes a second channel, not optionality, the thing to build; or
if a kind of work appears that is better done with nobody to ask, which so
far nothing has been.
