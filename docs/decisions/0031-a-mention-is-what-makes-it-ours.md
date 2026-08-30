# 0031 — A mention is what makes a message ours

## Status
Accepted. Supersedes the routing rule in
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md`; everything else that
record decided — Socket Mode, one app per project, a reply routed by its thread
— stands unchanged.

## Context

0029 decided that a message in a thread belonging to a job was for that job,
with no mention needed, and that a mention was required only outside one. The
reasoning was that inside a job's thread there is no ambiguity about who is
being addressed, because the thread names them.

That was built, and it worked. What it missed is a use nobody had tried yet:
**people need to talk under a job.** A job's thread is where its announcement,
its questions and its results appear, which makes it the obvious place for two
colleagues to discuss the change it proposed — and under 0029 every one of
those messages woke the agent and paid for a turn.

A thread stageman opened is still a room.

## Decision

**Nothing without a mention is read at all.** Then:

- A mention in a thread belonging to a job is for that job.
- A mention anywhere else on a bound channel is for that project's foreman.
- A mention in a thread belonging to *no* job is answered, saying so and
  naming where to go instead.
- Anything this instance said is never read, before any of the above.

The mention is now uniform, which is the property worth having: "stageman reads
what mentions it" is a rule a person can hold in their head, where "it reads
threads it started, and mentions elsewhere" is one they have to be taught.

Rejected: **keeping 0029's asymmetry.** It costs less typing in the case that
matters most — answering an agent's question — and it makes a job's thread
unusable for anything else. The typing is a habit every chat bot already
teaches; the unusable thread is a room nobody can talk in.

Rejected: **routing a mention in an unowned thread to the foreman.** This is
what 0029 did, and it is where somebody lands by replying to a foreman's own
message — the most natural move available. Sending it to the foreman would
work, once, and teach a person that a foreman can be held in conversation. It
cannot: by the time it answers it may be several messages further on, in a
different thread. So that case is answered rather than routed, and the answer
says where a foreman does listen.

Rejected: **silence for an unowned thread.** They addressed this instance. A
system that is quiet when spoken to is indistinguishable from one that is
broken, and the person is left waiting on a reply that was never coming.

## Consequences

**A fourth outcome exists**, and it is the one that speaks. Routing now answers
*this belongs to no job* separately from *this is not for us*, because the
first needs a sentence and the second needs nothing. The compiler found every
place that had to change, which is why it is a variant rather than a flag.

`Arriving` carries the arriving message's own identifier. A message at the root
*is* the thread a foreman answers under — there is nothing to open, unlike a
job's thread, which had to be created by posting the message it hangs from.

**A person must remember the mention**, and that is the whole cost. Every chat
bot works this way, so it is a habit rather than a lesson, but somebody who
does not know it will post at the root and be ignored with no indication why.
The notice above is the only place this is taught, which is thin. Revisit if
that turns out to be where people get stuck.

Revisit the whole rule if a project's channel is ever used only by stageman —
no colleagues, no side conversation — because then the mention buys nothing and
0029's asymmetry was right after all.
