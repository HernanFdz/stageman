# 0053 — A job is stopped or retired by a person

## Status
Accepted. Answers the retirement question
`docs/open-questions.md` has carried since
`docs/decisions/0015-a-job-survives-the-daemon-dying.md` opened it, for the
half a person performs. Depends on
`docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md` for the
states these two acts write, and on
`docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md` for
what a removed container lets go of. Since
`docs/decisions/0069-a-message-reaches-a-working-job.md` a person's
stop of a working job is a cancel the agent answers rather than a closed
process, with the same outcome; a stop that finds the job working with no
turn registered yet is held in memory until the turn is; and what a stop
does with messages the job had not yet been given is that record's.

Since `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`
forgetting a project is one synchronous step: the record goes in that step
and the discards of its containers wait on the write that removes it, so
the window the decision below reports rather than closes no longer exists,
and the refusal is checked once. A container the daemon leaves between that
write and the discard carries this instance's label, per
`docs/decisions/0054-a-container-says-which-instance-started-it.md`, and
names no job, which the waking sweep removes — so nothing is left
untracked, which was the reason for releasing the containers first.

## Context

Nothing ever removed a container. 0015 accepted that deliberately — "when a
finished job's container is removed is deliberately unanswered here" — and
three records since have raised the price of leaving one.
`docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` made a retained
container a *reachable* one, still serving whatever its agent last put up.
`docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md` made
one that is showing something a *running* container, so the cost stopped being
disk and became memory on somebody's laptop. And 0051 tied an image's lifetime
to the containers using it, so a container nobody removes is also an image
nobody can reclaim.

There was no way to end a job and no way to interrupt one. The only thing that
could stop a turn was killing the daemon, which 0015 is built to make
survivable — so the one available remedy was the one guaranteed to put the job
straight back to work at the next start.

## Decision

**Two acts, offered to a person, and never the same press.**

- **Stopping** ends the turn and keeps everything. The future running the agent
  is dropped, which closes the pipe the agent speaks on; the agent exits and
  the container carries on, because since 0043 the agent is not what the
  container runs. The job becomes *paused*, which is a reading of idle, so a
  reply reaches it exactly as it reaches a job that stopped by itself.

- **Retiring** ends the job and keeps nothing but the record. The container is
  removed with the session inside it, the tunnel is forgotten, and the images
  nothing needs are reclaimed. A person supplies the verdict — *done* or
  *discarded* — and *lost* is not offered, because that is the sweep's word for
  a container that went missing and nothing a person presses should be able to
  claim it.

**A working job can only be stopped, and every other job can only be retired.**
Stopping is reversible and retiring is not, so a screen that offered both at
once would put the irreversible one a mis-click from the reversible one. It
also means the destructive act is never a side effect: retiring a working job
is refused rather than quietly stopping it first.

**One place a turn happens.** Three callers ran a turn and recorded it in
nearly the same words; they now share one, which registers the turn as
stoppable, races the agent against the stop signal, and writes down whichever
won. A fourth caller that forgot to register would otherwise have produced a
job nobody could stop, silently.

**The record is written before the container is removed.** Interrupted between
them, this leaves a retired job with a container, which the next sweep finishes
— it looks for exactly that. The other order leaves a job that still looks
answerable with nothing to answer in, which needs a person. It is the same
argument `begin` makes for writing a job's record before creating its
container, in the same direction.

**Forgetting a project releases its containers first**, its jobs' and its
foreman's, and then removes the record. A project's record is what names those
containers, so removing it first would leave them untracked, which
`docs/conventions.md` §4 forbids by name.

Rejected: **retiring a working job by stopping it first.** One press instead of
two, and it makes an irreversible act the second half of something that does
not look irreversible. The refusal costs an operator one extra press at the one
moment that is worth slowing down.

Rejected: **the protocol's own cancellation** instead of dropping the future.
Better in kind: the agent would get a clean stop reason rather than a closed
pipe, and could say what it had been doing. It needs a connection handle shared
across tasks, which is the machinery `docs/open-questions.md` still wants for
holding a foreman's connection open, and building it here would build it twice.
Worth revisiting when that lands.

Rejected: **automatic retirement**, on a timer or on the agent judging itself
finished. The second is the right long-term answer and it is not this decision:
an agent that believes it is done says so — that is what *proposed* is — and
what turns a proposal into an ending is a person, or a conversation on a
channel that does not exist yet. A timer is the wrong shape outright, because
the thing being reclaimed is somebody's unread work.

Rejected: **removing a container without recording anything**, as a pure
cleanup. It loses the one fact worth keeping: whether the work was any good.
The record is the whole reason a retired job stays on the screen.

## Consequences

**A stopped job can lose a half-made workspace.** A job's first turn creates a
container and checks a repository out into it before the agent speaks, and
stopping partway leaves whatever had been done. Accepted rather than prevented:
the window is seconds out of minutes, the container is named after the job so
nothing is untracked, and an agent given that workspace afterwards reports what
it actually finds rather than what it expected.

**A stop signal is held in memory and never written down.** A turn does not
survive the process, so neither should the means of stopping one — a signal
recorded now and acted on after a restart would stop a turn nobody asked about.

**Forgetting a project has a window it reports rather than closes.** The
refusal is checked before and after releasing the containers, because that
release cannot happen with the instance held shut. A job started by this
project's foreman in between keeps its record and loses its container, and the
next sweep records it lost. Narrow, and honest.

**`docs/conventions.md` §4's bar is unchanged and easier to meet.** Nothing
here removes a container this instance cannot name, and the containers it does
remove belong to jobs it has just written a verdict for.

**Reversing** means deleting two routes and the registry behind them. Nothing
migrates, and every job already retired stays retired with its container
already gone — which is the half that cannot be undone, and the reason the
control says so on the way in.

**Revisit if** an agent is ever driven through a connection this project holds
open across turns, which makes a polite cancellation available and turns
*paused* from a closed pipe into something the agent knows about; or when a job
can be retired by a conversation rather than a press, which is the automatic
half this record deliberately leaves out.
