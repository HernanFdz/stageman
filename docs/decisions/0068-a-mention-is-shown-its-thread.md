# 0068 — A mention is shown its thread

## Status

Accepted. Nothing below is built yet; this record exists so that the shape is
settled before it is.

Amends `docs/decisions/0031-a-mention-is-what-makes-it-ours.md` and
`docs/decisions/0060-a-binding-is-a-workspace.md` in one respect: what a
mention is *shown*, where those records decide what is *read*. What wakes an
agent does not change. Depends on
`docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
for the identifier every message is shown with, and for the thread a reply
most often hangs under, which since that record is something the agent wrote
and never saw answered. Leaves
`docs/decisions/0063-another-app-is-heard-in-a-watched-room.md` standing: a
signal in a thread is shown the thread as well.

What the platform returns for a thread was measured on 2026-09-21 against a
real workspace, with the credential that speaks, on threads a person, this
instance and another app had written in.

## Context

A turn is shown the message that started it and nothing else: the words,
framed as a person's or as an app's, with where it was said travelling on the
warrant. 0031 keeps people talking in a thread without waking the agent, and
0060 lets a person go on talking to the foreman in one because the session
remembers. What the session does not remember is anything said in that
thread without a mention, and since 0067 the thread a reply hangs under is,
most often, one whose parent the agent wrote and whose replies it has never
seen.

So the case a person hits first is this. Two people discuss a job's work
under one of its messages, and one of them then asks the agent about it.
The agent is handed the question and none of the discussion, and either
asks what was meant or guesses. The same happens when a person replies
under an earlier message of their own, and when a person quotes a line of
the agent's working and asks why.

**What the platform returns**, measured. Asked for a thread, it answers with
the parent first, carrying how many replied and who, then every reply in
order, each with its own identifier and who said it: a person by user, this
instance by the bot identifier the identity call already gives, another app
by its own, and a follow-up broadcast by its subtype. One page holds up to a
cap the caller sets and says whether there is more. The scope it needs is
the one the manifest grants to hear a room, and its budget is documented as
about fifty such reads a minute. The event that carries a mention carries the
thread's identifier and nothing of the thread.

## Decision

**A mention in a thread is shown that thread, before its turn starts.** The
instance asks the platform for the thread, one request answered before the
turn begins, the way a room is made before a job starts, and frames the turn
with the parent message and everything said in the thread since this
instance last posted there, in order. Each message is shown with who said it
and its identifier: a person by the platform's own mention of them, this
instance as its own words, another app by name. The mention itself is the
last entry, marked as the one addressed to the agent.

**"Since this instance last posted there" is derived, never kept.** An agent
remembers what it said and what it was shown when it said it, so everything
after its own last post in a thread is exactly what it has not seen. The
fetched thread says which posts are this instance's by the bot identifier,
which is the rule the frame decoder already applies, so nothing is recorded
to know it. When the session is fresh rather than resumed, because the
container was recreated, the whole thread is shown, since the memory the
rule relies on is gone.

**Capped**, at one page of the fifty most recent messages, and the frame
says when a thread was longer. A bound rather than a design.

**A root mention is shown nothing.** The message is its own context, and
nothing else in the room is read for it.

**A signal in a thread is shown the thread the same way.** A follow-up under
an earlier card means nothing without what it follows, and the platform
carries only the root on it.

**A thread that cannot be fetched does not stop the turn.** The message goes
alone, the frame says the thread could not be read, and the refusal is
logged.

**What is read to wake anybody does not change.** A person is read from the
mention event and from nothing else; nothing in a fetched thread wakes an
agent, costs a turn, or is answered because it was fetched. The rule 0031
keeps, that people can talk under a job without waking it, holds exactly.
What changes is that when one of them pulls the agent in, it reads what a
colleague pulled into that thread would read.

Rejected: **the parent and the mention only.** Cheaper, and it needs no rule
about what was already seen. It breaks precisely the case above: the
discussion is the context.

Rejected: **the whole thread every time.** It duplicates what the session
already holds, grows with the thread, and re-reads on every mention what
people said once.

Rejected: **keeping, per thread, how far the agent was shown.** Kept state
for what the fetched thread already says, and wrong in the one case where it
would matter, a session that was lost.

Rejected: **the room's recent history on a root mention.** It widens what the
agent has read beyond what a person pointing at a thread expects, with no
bound a person can predict. If it ever comes, it is a tool the agent calls
with a bounded window, so that a person can see it chose to look; the revisit
below names it.

Rejected: **reading untagged messages as they arrive**, so that the context
is always there. Refused by 0031 and still refused: a turn per message or
held state per room, and people who cannot talk in a room without being read.

## Consequences

**One request per threaded mention**, before the turn, against a budget
nothing here approaches.

**The manifest does not change.** The scope that reads a thread is the one
that hears the room.

**Words with no mention reach a model for the first time.** Said in a room
the app was invited into, shown only when somebody in that thread mentions
the agent, and only from that thread; a person who does not want a thread
read does not mention the agent in it. Written down because it is a change
in what people can assume, however small.

**People are shown by the platform's markup, not by name.** A name costs a
scope the manifest does not grant, which 0063 already declined for a room's
name; the markup is what a mention's own text already carries, and it renders
as a name wherever the agent repeats it.

**The frame is snapshot-tested**, as every text this project composes is; a
thread's messages are quoted inside it and are not this project's.

**Nothing recorded changes shape.**

**Reversing** is deleting the fetch, its reader in the channel crate, and the
frame.

**Revisit if** threads longer than the cap turn out to be where questions get
asked, which wants paging; if root mentions keep arriving that assume the
room's discussion was read, which wants the on-demand tool above; if a
project's people object to untagged messages being shown, which wants a
per-project switch; or if the platform starts carrying the thread on the
event, which retires the fetch.
