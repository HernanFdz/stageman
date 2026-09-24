# 0073 — The world tells the instance the time with every step

## Status

Accepted. Amends one sentence of
`docs/decisions/0056-the-instance-decides-and-the-world-performs.md` — that
the event variants whose handlers keep a time carry one, and no others do —
and one of
`docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`,
that every arrival is stamped. Taken while building the moment a job's
standing last changed, which
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` asks for.

## Context

The instance reads no clock, per 0056. Time reached it on four events, each
stamped by the world with its clock at the moment the world observed the
fact: a served request's arrival, a made request's response, a frame heard
on a socket, and a socket's end. The rule was that a variant carries a time
only where its handler keeps one, so that a value carries nothing nobody
reads.

Three things showed the rule had drifted before the moment a standing
changed asked for a fifth time.

- **Arrivals were stamped wholesale.** 0057 stamps every served request
  "because a generic world cannot know which handlers keep it", which is the
  argument for stamping every event, made about one variant.
- **The timer carried no time.** A wake said which wake fired and not when,
  so a handler woken to pace something had no clock, and a timer that fired
  late was invisible to it.
- **The dashboard read the clock itself.** Starting a job by hand carried a
  time the server function filled with the application's own clock, because
  the application's request event had none: a clock read outside both the
  instance and the world's stamping, kept as data in every replay.

And the moment a standing changed would have needed a time on the turn's
end, the probe's answer, every request, the sweep's listing, and on whatever
writes a job's progress next — a rule kept by remembering.

Two clocks were considered, because every event does have two moments: the
one at which the world observed the fact, and the later one at which the
instance was handed it, after whatever was queued ahead of it.

## Decision

**The world tells the instance the time with every step, stamped as it hands
the event over, and no event carries a time of its own.**

- **Time is a property of the step.** The instance is stepped with the time
  and an event, and every handler reads the step's time. A job's progress is
  recorded through one function, which writes the moment the standing
  changed from it; so does whatever writes progress next, without anybody
  remembering to.
- **Stamped at delivery, at one point.** The world's loop reads its clock as
  it dequeues, so the sequence of times the instance sees never goes
  backwards, which times stamped by several tasks at observation did not
  promise: a frame observed first can be queued second. A step told a time
  earlier than the last is a fault in the world, said in the log and taken
  as given.
- **A fact's own time is data.** A message's timestamp is the platform's and
  a pull request's dates are the platform's; they travel inside the payload
  where they are facts, and never as a stamp.
- **The domain converts at its edge.** The vocabulary's time is the plain
  count of milliseconds it always was; a job's moments are the domain's
  timestamps, made from it where the instance records one.
- **The moment a standing changed is defaulted.** A job the last release
  wrote has none, and says only that it waits. It changed its standing before
  this build made its first stamp, so it has waited longer than any job that
  carries one: a list ordered by waiting puts such jobs first, among
  themselves by when they were made.

Rejected: **a time on each variant whose handler keeps one**, the rule this
replaces. It failed by omission on the timer, the turn's end and every
request, and would have failed again on the next handler to keep one.

Rejected: **the moment observed and the moment delivered, both.** They are
one clock read twice with a lag between them that no decision of the
instance's turns on; the lag is the world's to log, and the world knows both
moments without telling the instance either. The simulation makes the cost
plain: in a discrete-event world an event has exactly one time, the virtual
moment it is delivered, and a second stamp would be a number the simulator
invents and no invariant checks.

Rejected: **the instance reading a clock.** 0056's rejection stands, for the
reason it gave: determinism as a property of the type.

## Consequences

**Four fields go rather than one arriving**: the three variants' stamps, the
arrival's, and the time on starting a job by hand, with the application's
clock read behind it. The listener code that threaded a time through six
functions reads the step's instead.

**The harness already thought this way.** A replay records each turn as the
time, the event, the effects and what changed, on the recording world's
clock, and no recorded event carried a time of its own. The instance was
the only party not told. The replays are re-recorded once, and their diff
is the review.

**A monotone clock is something a scenario can check**, and something a
handler can lean on: a gap with no connection is never negative, and a list
ordered by waiting is ordered.

**Reversing** is putting four fields back and a parameter away; nothing is
persisted that depends on it except the moments recorded, which are read
either way.

**Revisit if** the instance ever has a decision that turns on how late it
learned something, which would make the observed moment a fact worth
carrying; or if a second clock has to be reconciled with this one, which is
when a platform's timestamps would stop being mere data.
