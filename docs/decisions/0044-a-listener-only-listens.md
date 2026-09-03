# 0044 — A listener only listens

## Status

Accepted. Narrows the transport half of
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md`, which chose the
socket and said nothing about what may be done on it.

## Context

Messages addressed to a foreman at the root of a channel went missing. Not
delayed — missing: no answer, no notice that one had been received, and nothing
in the log at any level an operator runs. The report correlated the losses with
a job being under way, which turned out to be the right instinct for a reason
narrower than it sounds.

**The task that read the socket was also the task that did the work.** A frame
was decoded, acknowledged, and then handled to completion before the next was
read. Handling a message for a foreman means draining that foreman's inbox, one
agent session per message; handling a reply in a job's thread means resuming
that job's container and running a turn in it. Both are minutes. For those
minutes nothing polled the socket, and three things followed from that one
fact.

- **Nothing answered a ping.** The websocket library is driven entirely by
  polling: receiving a ping is what queues the pong, and there is no task
  behind it. An unpolled connection therefore does not merely go quiet, it
  stops meeting its end of the protocol. The platform's own client library
  documents a thirty-second budget for hearing from the server and a
  five-second one for being answered, so it is not ambiguous about what it
  expects of a peer.
- **The warning could not be used.** The platform says it is about to close a
  connection roughly ten seconds ahead, which is ample and is the entire point
  of the warning. Behind a four-minute turn it expired unread.
- **The recovery was slow and read as routine.** When the loop came back and
  found the socket gone, that ending is the ordinary one — a long-lived
  connection is closed on a schedule of its own — so it was logged as such and
  a replacement was opened after a five-second wait. Nothing distinguished it
  from the healthy case, because from inside the read loop it was not
  distinguishable.

Events are not replayed to a connection opened afterwards. So the window from
the connection dying to the replacement being up is a window in which anything
said reaches nobody, permanently.

Two smaller things compounded it. The five-second wait was taken on the
*scheduled* refresh too, so even a perfectly healthy instance was deaf for at
least five seconds several times a day. And the instance knew it had no
connection open and never said so — the whole story sits at a level below the
default, so a clean log was not evidence the message had arrived, it was
evidence that neither answer was visible.

## Decision

**The task that reads a channel does nothing else.** Concretely, three things.

- **Handling is dispatched, not awaited.** `act` is synchronous and every await
  it used to make now happens on a task of its own, so the read loop returns to
  the socket immediately.

  **With one deliberate exception, and it is the interesting half.** The state
  change each recipient makes *on arrival* stays on the reading task: the
  foreman's inbox taking the message (`arriving`), and a job accepting or
  refusing a reply (`accepting_reply`). Both were already one operation under
  one lock, so moving them would not have broken anything by racing — it would
  have broken the *order*. Two frames read back to back would be two spawned
  tasks, and the runtime polls the most recently spawned first, so the usual
  outcome would be the second message taking the inbox ahead of the first. An
  inbox whose only promise is arrival order cannot be filled in polling order.

- **A replacement is opened before the connection it replaces is let go.** The
  ten-second warning and the platform's allowance of several concurrent
  connections exist together for exactly this, and using them costs nothing: on
  a refresh the old socket is handed to a task that keeps reading it while the
  new one is opened. There is no wait, because nothing went wrong. The wait
  (`BEFORE_TRYING_AGAIN`) survives only where it belongs, after a failure.

- **A window with no connection is a warning, with its length.** Not the
  connection ending, which is ordinary and stays where it was. The anomaly is
  the gap, an operator can act on it, and it is the only thing in here that
  explains a message nobody answered.

Rejected: **raising the existing lines to warn, or shipping a unit that sets
the log level to info.** The first builds the thing this codebase already
argues against in the function that classifies an ordinary ending — a warning
that fires several times a day on the healthy path is one people learn to
scroll past. The second contradicts `README.md`, which commits to a default of
warn and to it not being "a commentary on things going right", and would put a
line in the journal per frame. Neither is what was missing: the gap was not a
quiet line, it was no line at all.

Rejected: **one handling task per project, fed by a queue.** It fixes the
socket completely and preserves order exactly, with no change outside this
file, and it was close. It loses on what it keeps: a reply to one job would
still wait behind another job's turn, which is the same serialisation moved one
step away from the socket rather than removed. Recipients are already
serialised by their own state, so there is nothing left for a second queue to
protect.

Rejected: **spawning the arrival transition along with the work**, which is the
obvious reading of "hand it off" and is wrong for the reason above. Worth
recording because it is what a later reader will try to simplify this into, and
because it would fail in the direction nobody tests: two messages sent a second
apart never race, and the bug would only ever appear under exactly the load
that makes it hardest to see.

Rejected: **holding the connection open with a timer that pongs independently.**
It treats the symptom, leaves the socket unread for minutes, and would keep a
connection alive precisely so that messages could accumulate behind a loop that
is not reading them.

## Consequences

**Messages for different recipients are now handled concurrently.** That is not
a new hazard: the two transitions this could have raced on are the ones kept on
the reading task, and everything after them is already guarded by the recipient
that owns it — a foreman's loop is driven only by the arrival that found it
idle, and a second reply to a working job is refused rather than admitted. What
changes is that those guards now do work they were written for and never had to
perform.

**A refresh costs nothing, and an unscheduled ending still costs something.**
There is no way to overlap a connection nobody warned about, so the gap is
reduced to what it must be — noticing, and opening another — rather than
eliminated. It is now measured and reported, which is the honest version of a
limit that cannot be removed.

**Two connections are briefly open per project.** Within the platform's
allowance, and each event is still delivered once, to one of them; both are
read by this process, so nothing is duplicated.

**The read loop's not awaiting is a property nobody can see.** `act` being
synchronous is what enforces it, and that is worth stating because it is
invisible in the way that matters: adding an `await` there would compile,
behave perfectly under test, and reintroduce exactly this. `docs/architecture.md`
§2 carries it as an invariant so that it is defended by a reviewer who has been
told what to look for.

**Reversing** is small and well isolated — recombining two halves in
`app/src/instance.rs` and awaiting in `app/src/listening.rs` — and there is no
data to migrate, because none of this is recorded anywhere.

**Revisit if** a channel arrives whose transport redelivers what it could not
deliver, which would make the gap warning noise rather than news; or if the
number of concurrent handlings becomes the thing an operator notices first,
which would be a bound on work in flight arriving rather than this being wrong.
