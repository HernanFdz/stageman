# 0052 — A job's state says what somebody does about it

## Status
Accepted. Replaces the three states in
`docs/decisions/0015-a-job-survives-the-daemon-dying.md`'s consequences and the
"deliberately three and not more" reasoning that went with them, and answers
the first half of the retirement question in `docs/open-questions.md` — what a
job that is over *is*, as distinct from who ends one, which is still open.

## Context

A job had three states: working, idle, failed. The set was chosen on a rule
worth keeping — *a state nobody acts on differently is a label rather than a
state* — and two things have since made it too small.

**Idle is four situations wearing one word.** An agent stops when it has asked
a question, when it believes the work is done, when a person stopped it, and
when it simply stopped. A person does something different about each: answer,
review, resume, investigate. The record's own note admitted this and treated it
as unknowable — *nothing here can tell those apart* — which was true while the
only evidence was the protocol's stop reason. It stops being true the moment
the agent is asked, and
`docs/decisions/0034-tools-are-served-not-shipped.md` already gives a job a
tool to answer with.

**Failed is two situations wearing one word, and the words point opposite
ways.** An expired credential and a container that has vanished are both
recorded as failure, and one is a job waiting for a fix while the other is a
job that can never run again. The second was being written as prose — *its
container is gone* — which nothing can branch on, so a reply to such a job was
accepted, resumed, and failed at the runtime with nobody told why.

Both of those are the same defect: a state that names *what happened* rather
than *what to do next*.

## Decision

**Two levels. The outer says what the system does with a job; the inner says
what a person does about it.**

- **Working** — a turn is in it.
- **Idle**, carrying one of five readings: *asked*, *proposed*, *paused*,
  *failed*, *silent*. All five own a container, take a reply, and are resumed
  the same way.
- **Retired**, carrying one of three outcomes: *done*, *discarded*, *lost*. No
  container, no reply, never resumed.

**Nesting rather than eight flat variants**, and the reason is what the callers
ask. Every behavioural caller — the reply gate, the sweep, the project-forget
guard, the query for what is still running — asks an outer question, so each
matches on three arms and stays at three when a reading is added. Flat, each
would need a wildcard or a long alternation, and a wildcard is where a ninth
variant silently lands in the wrong branch. The dashboard is the one place that
wants every case, and it pays one extra match, which is the right place to pay
it.

*Done* and *discarded* prove the rule rather than strain it: the system does
exactly the same thing with both, so they are two readings of one state rather
than two states.

**Failed is not terminal**, and this is the correction that matters most. The
container is still there with the session in it, so a failed job is one waiting
for whatever broke to be fixed, and a reply is what tries again. What makes a
job over is retirement and nothing else.

**Silent must exist.** A turn can end on a token limit, on a refusal, or on an
agent that did not call the tool, and defaulting those to *asked* or *proposed*
would record a claim nobody made. A job landing there is a prompt that was not
followed, and it is visible rather than smoothed over.

**Lost is written by the sweep**, on finding that a job which is not over has
no container. It is terminal because there is nothing left to resume: the
session lived in that container. The sweep now asks this of every unfinished
job rather than only the ones believed to be working — an idle job whose
container has gone is in exactly the same position, and used to look answerable
indefinitely.

Rejected: **eight flat variants.** Simpler to declare and worse at every use.
See above.

Rejected: **`Retired { status }` as the only nesting, with the idle readings
flat.** Half the shape, and it splits on the wrong axis: the readings that most
need grouping are the ones a reply may reach.

Rejected: **inferring the reading from the agent's transcript.** No tool, no
prompt change, and it reads a claim out of prose that was not written to carry
one. The failure would be confident and wrong, which is worse than *silent*.

Rejected: **keeping a job's ending as prose on the failed state.** What exists
today. Nothing can branch on prose, which is how a reply came to be accepted by
a job whose container had gone.

## Consequences

**Every job reads as *silent* until the tool that sets a claim exists.** That
is a real gap and a deliberately narrow one: the dashboard word for it is
*idle*, which is what it said before, so nothing a reader knows changes and the
richer readings arrive when the tool does.

**A reply to a retired job is refused and said so on the thread.** Distinct
from the refusal for a job nobody has, because a person is told something
different: this thread did belong to a job, so replying here was not a mistake
about where to say it.

**The snapshot carries a shape that the last release does not write.** A bridge
reads what it wrote — a bare idle becomes *silent*, since that format had
nowhere to record a claim, and a top-level failure becomes the failed reading.
Everything older than that release was dropped in the same change, per the
support window now recorded in `docs/conventions.md` §4.

**The sweep gained a state it can write and an operator cannot argue with.**
*Lost* is the one outcome nobody chooses, and it overwrites nothing a person
decided: a job that is already retired is passed over entirely.

**Reversing** means collapsing the readings back into one idle and one failure,
which loses every claim already recorded and turns *lost* back into prose. The
snapshot would need a bridge in the other direction, which is the expensive
half — this is the first change here that a downgrade cannot read.

**Revisit if** a reading is ever added that the system treats differently, which
would mean it belongs at the outer level rather than the inner one and the split
is in the wrong place; or if *silent* stops being rare once the tool exists,
which would say the prompt is not being followed and is a prompt problem rather
than a state problem.
