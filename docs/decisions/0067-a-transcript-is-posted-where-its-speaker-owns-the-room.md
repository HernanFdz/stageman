# 0067 — A transcript is posted where its speaker owns the room

## Status

Accepted. Nothing below is built yet; this record exists so that the shape is
settled before it is, as
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md` was.

Supersedes one sentence of
`docs/decisions/0034-tools-are-served-not-shipped.md`, inherited from
`docs/decisions/0028-stageman-ships-the-tool-that-speaks.md`: that the tool
which speaks is the only way anything an agent writes reaches a person. The
tool stays, as the way to speak somewhere other than where its speaker's
words already go. Amends
`docs/decisions/0061-a-job-has-a-room-of-its-own.md`, which gave a job a room
and the foreman none, and
`docs/decisions/0062-what-this-instance-says-is-markdown.md`, where it says
nothing here splits a message. Leaves
`docs/decisions/0063-another-app-is-heard-in-a-watched-room.md` standing by
construction rather than by rule: nothing an agent writes reaches a room its
speaker does not own unless it asks. Leaves the mention rule of
`docs/decisions/0031-a-mention-is-what-makes-it-ours.md` untouched.

What the adapter streams was measured on 2026-09-21 against the image this
project builds, driven as the instance drives a turn. What the platform
allows is taken from its documentation, not measured here, and is marked as
such below.

## Context

An agent says a great deal while it works, and a person hears one sentence
of it. The protocol streams everything the agent does over one connection:
the text it writes, its reasoning, every tool it calls and what each
returned. The conversation in the agent crate keeps the text and drops the
rest, on the reasoning that it was the answer and not a transcript, and the
instance then drops the text as well. So the only thing that reaches a
person is what the agent chose to pass to the tool that speaks, and every
instruction says so: ordinary output is seen by nobody.

That was the right first shape, and it has two costs that have become
visible. A person watching a job sees nothing between the kickoff and the
tool call that ends it, so "is it stuck?" has no answer short of the
dashboard, and the dashboard says *working*. And a turn that ends without
the tool call has told nobody anything, which
`docs/decisions/0055-a-job-says-why-it-stopped.md` measured happening to a
tool the agent was told to call every time.

Three things were measured or looked up before deciding what to post and
where.

**What the adapter carries.** A tool call arrives as a pending notification
with a generic title and its kind, is refined at once with the command or
the file as its title, gains a one-line description in the adapter's own
metadata, and ends with a status and its output. Reasoning does not arrive
at all: at every effort level the adapter offers, a task that needed working
out produced text and tool calls and not one thought chunk, because recent
models omit thinking text and the adapter emits nothing for an empty block.

**How much a turn produces.** Forty interactive sessions of the same agent
on this machine, split at each human message: the median turn made five tool
calls and wrote two runs of text, but the heavy tenth made thirty-five calls
or more, up to a few hundred, beside a dozen to ninety runs of text. A job's
one unattended turn is a heavy turn.

**What the platform allows**, from its documentation. An app may post about
one message a second to a room. A Markdown post is limited to twelve
thousand characters. A bot may edit its own messages with Markdown, thread
replies included, and an edit changes the message in place. The client folds
a long message. A thread reply reaches the thread's followers; a plain post
marks the room unread and pings only the members who chose to be told of
everything; a mention pings. Nothing lets an app mute a room for its members,
because muting is each person's own preference.

One more fact shapes the foreman's half. A job's room is its own. A foreman
speaks in rooms people own, and 0063 made silence on a signal a decision: a
foreman that judges a card from another app unworthy of a job says nothing
under it. Anything posted automatically into the room a signal arrived in
would make that silence impossible.

## Decision

**What an agent says and does is posted as it happens, at the root of the
room its speaker owns.** A job's transcript goes to the job's room. A
foreman's goes to a room made for it. Two kinds of message, in the order
they happened:

- **Narration** is the agent's own text: one message per contiguous run of
  it, posted as the run begins and grown by editing until a tool call
  interrupts it or the turn ends.
- **Working** is what the agent does between two runs of narration: one
  message per run, a burst, grown in place as the run continues, with one
  line per tool call giving its kind, its title and its status, and a
  thought, where an adapter carries any, as a quoted line in its place. The
  next narration closes the burst; the next tool call after that opens
  another.

The root of a room is then the transcript of every turn, complete and in
order, and its length is set by how often an agent alternates between
talking and working rather than by how much it works. A thread under either
kind of message is for people: a conversation about what was said, or about
that stretch of working, quoting the line meant.

**The tool that speaks stays, and gains a target.** It takes a message and,
optionally, a message to reply to. Absent, it posts at the root of the
speaker's own room, which is where its narration already goes; present, it
posts in that message's thread. The target is the room and the message
rendered as one identifier, which is how every message shown to an agent is
identified from now on, so that a foreman can reply into a room it does not
own and can name a thread it was shown in an earlier turn. The instance
parses the identifier and checks the room: a job may name only its own, and
a foreman may name any, with the platform refusing one the app is not in.
The tool answers with the identifier of what it posted.

**A notice at the root says why a turn started**, posted by the instance
before anything the agent says: a link to the message that started it, or
that the job began. It gives the narration after it something to be about,
and it is most of what makes a foreman's room legible, where every turn was
started somewhere else.

**A turn started by a mention in a thread that ends without a post in that
thread is told so there**, in one line linking where the answer went. The
tool call this design depends on is one an agent will sometimes miss. A
missed one now leaves an answer at the root rather than silence, and the
line turns that into a signpost.

**A project's foreman has a room of its own**, made before its first turn if
the project has none, recorded on the project, and archived when the project
is forgotten. Public, with nobody invited, since this instance does not know
who the operator is on the platform; linked from the dashboard; named
`<project>--foreman--<8 hex of the project id>`, because an archived name is
taken for ever and a project deleted and recreated under one name would
otherwise collide, which is 0061's reason for the identifier in a job's room
name. A job started from the dashboard, which 0061 left with no
announcement, is announced there. Speaking to people stays explicit: the
foreman's narration goes to its own room, and a thread in somebody else's
room hears it only through the tool, which is what keeps 0063's silence.

**Posting is paced, chained and never load-bearing.** One request in flight
per room, the next sent when the previous is answered, so that posts land in
order. Growth by editing is paced by a timer rather than sent per line. A
message that would pass the platform's limit continues in a second one.
Every one of these posts is a notice in the sense 0061 gives the word: its
failure is logged and changes nothing, and the tool's answer remains the
guarantee for anything a person must see.

**Agents are told.** The instruction that says ordinary output is seen by
nobody is replaced by its opposite: what you write is posted to the people
in your room as you write it, so write for them, and reply to a message by
naming it. The paragraph asking a job to finish by saying what it did goes,
because its last narration is that.

Rejected: **the tool as the only way, as now.** The transcript is thrown
away and a missed call is silence, and both costs are paid on every job.

Rejected: **the working in one thread per turn, narration at the root.**
Quiet, and it loses what observability is for: which calls came before which
narration is recoverable only by merging two views by timestamp. A turn's
first working also has nothing of the agent's to hang under, because an
agent usually reads before it speaks.

Rejected: **tool calls at the root, thoughts in a thread.** Keeps calls in
order against narration and loses them against thoughts, which is the worse
half to lose: a thought is usually the reason for the call that follows it.

Rejected: **every line as its own message.** A heavy turn is a few hundred
messages, which at one post a second takes minutes to deliver and lags the
agent by that much, and the narration a person wants is buried among them.

Rejected: **narration posted where the message being handled was said**, in
its thread when it was in one. It hides a round of work under an aside, so a
reader of the root sees two ready-for-review notices with nothing between
them; it needs a special case for a signal, whose thread must stay silent;
and the answer to the person still depends on the tool call.

Rejected: **renaming the tool to reply.** Absent a target it is not one, and
0028's reason for the name stands: it is named for the act.

Rejected: **telling the agent the identifier of each of its posts**, so that
it could reply under its own earlier narration. Nothing can tell it
mid-turn, and nothing needs to: a person's reply carries the message it
hangs under, and what the agent is shown of that thread is the next
record's to decide.

Rejected: **a per-turn alias for identifiers**, short and hard to mistype.
Not stable across turns, and a foreman linking an earlier thread needs an
identifier its session can carry from one turn to the next.

Rejected: **the dashboard as the transcript's home.**
`docs/decisions/0005-conversation-happens-on-channels.md` keeps conversation
on channels, and a page cannot be replied to. A page may show the same
stream later, as a second reader.

Rejected: **the foreman's narration into the thread it was asked in.** Its
working is noise in a room people own, and on a signal it is the breach of
0063 described above.

## Consequences

**A room's root holds what a person reads.** For a heavy turn that is on the
order of twice its runs of narration rather than a few hundred lines, and a
long burst folds behind the client's control. Not posted at all: usage,
available commands, session information, plans, and the echo of what this
side said.

**The manifest does not change.** Editing needs the scope that posts, and a
foreman's room needs the scope a job's room already has.

**Posting has a budget, and the design spends it on purpose.** Chaining and
pacing keep a busy turn inside one post a second per room; what that costs
is that the root lags a very busy agent by the pace, which is seconds.

**Every post and edit is heard back and dropped**, as this instance's own
words are already. More frames on the socket, and no decision made from
them.

**The bar in `docs/conventions.md` §4 narrows.** Every text this project
composes is still asserted whole: the notices, the framing, the lines a
burst is built from. An agent's own words are posted as they come and are
not this project's to assert.

**Behaviour changes without control flow changing.** A job that finishes
without the tool has still reported, because its last narration is the
report. Whether an agent narrates more once told it is read is not measured,
and is the first thing to watch.

**What the last release wrote opens with no foreman room**, and one is made
at the next turn. Nothing else recorded changes shape.

**Reversing** is stopping the stream, dropping the target from the tool, and
archiving the foremen's rooms; the room recorded on a project can stay. The
prompts go back to saying nobody reads ordinary output, which is then true
again.

**Revisit if** an adapter starts carrying thoughts with text, which decides
whether a quoted line in a burst is enough or reasoning wants a place of its
own; if a turn outruns the per-room budget even paced, which wants a digest
rather than a growing message; if a channel arrives without threads or
edits, on which this layout cannot be built; if narration at the root proves
unreadable for a particular agent, which wants a per-project switch rather
than a rule; or if people turn out to reply under bursts more than under
narration, which says the detail belongs at the root after all.
