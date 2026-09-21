# 0061 — A job has a room of its own

## Status

Accepted. Supersedes the thread-per-job half of
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md` — a reply is routed
by its room now — and takes the home room away that
`docs/decisions/0060-a-binding-is-a-workspace.md` kept for one step, which
removes the last thing
`docs/decisions/0027-a-channel-is-not-a-platform.md` called an address.
Depends on `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`
for the moment a room is archived, and on
`docs/decisions/0034-tools-are-served-not-shipped.md` for why the credential
that creates one never enters a container. Since
`docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
a job's room also carries what its agent says and does as it happens, the
foreman has a room of its own, and a job started from the dashboard is
announced there.

Every claim below about what the platform does was measured on 2026-09-15
against a real workspace.

## Context

A job's conversation was a thread hanging from an announcement in the
project's one channel. That was chosen, before any of it was built, for two
reasons: a channel per job "needs a token that can create channels where
posting needs only to post, and widening a job's credential cuts against
narrowing it", and it "costs a channel per job for ever, in a workspace
nobody can garbage-collect".

Both premises have since gone. 0034 keeps the bot token in the daemon, so a
job's container holds nothing that could create anything, and widening what
the daemon may do widens nothing a job holds. And 0053 gives a job an ending
a person performs, which is exactly the moment a room can be archived.

What a thread costs in use has become visible since the first real jobs ran.
A job's whole life collapses under one announcement in a channel shared by
every other job; a person has to find it; what a job shows and what it says
are buried; and two jobs' threads interleave, so a reader of the channel sees
neither.

What the platform allows was measured. A bot token with `channels:manage`
creates a room and is its first member; a name is lowercase letters, digits,
hyphens and underscores, eighty characters at most, and an archived room's
name is still taken; posting into an archived room is refused, saying so; a bot token can archive a room, set its topic and purpose,
invite people into it — and unarchive it, so archiving is not final; and
consecutive hyphens survive.

## Decision

**A job's conversation happens in a room of its own**, created by this
instance before the job's container, once the job's record is on the disk —
the order `begin` already keeps: record, then somewhere to speak, then the
container. The room's identifier is recorded on the job where the thread
was, and a reply is routed by its room.

**Named `<project>--<title>--<8 hex of the job id>`.** The project's name
and a title the foreman gives the job, or the first words of the work when
a person starts one by hand, both folded to what a name may contain; and the
identifier's prefix, which is what makes the name unique for ever and the
only part that is load-bearing. Double hyphens separate the three parts
because titles contain single ones.

**Described, so that the sidebar says what the room is for.** The purpose is
the job's reason and the topic is where it shows its work and where the
dashboard is, each within the platform's limit. Both are notices: a failure
is logged and changes nothing.

**Public, with the creator in it and the person who asked.** A job started
from a person's message records who that was, carried by the warrant of the
turn that started it the way the thread already is, and that person is
invited. A job started by a bot's signal or from the dashboard records
nobody. Anyone else joins, because the room is public.

**Announced where the request came from.** A job started from a message is
announced as a thread reply under that message, linking the room. A job
started from the dashboard has no source room, and the room is its own
announcement.

**A mention anywhere in a job's room is that job's**, at the root or in a
thread, and it is answered where it was said. The notice that the job has
stopped goes to the root of the room whatever thread the exchange was in,
because it is about the job rather than part of the exchange: the root is
the job's timeline, which is what somebody glancing at the room reads, and
a thread would fold the state change away. Nothing else about routing
changes: a mention anywhere else is the foreman's.

**Retiring a job archives its room, and forgetting a project archives every
one of its jobs' rooms.** A failed or idle job keeps its room, because it is
not over. An archived room refuses a post, so the notice that a job is over
is reached only when archiving failed or somebody unarchived the room by
hand, and it stays for that.

**The address goes.** A binding is the two credentials and nothing else.

Rejected: **a private room per job.** It needs a second scope and a
membership list somebody maintains, and the case for it — a project whose
own rooms are private — has not arrived. Public is what "somebody may want
to look" needs, and private can be added for that project when it comes.

Rejected: **naming a room by the job identifier alone.** Unique and
derivable, like a container's name, and unreadable in a sidebar holding a
dozen of them. A title costs one field in the tool that starts a job.

Rejected: **deleting a room on retirement.** No bot can; archiving is what
exists, and it keeps the conversation searchable, which is what a retired
job's record is for.

Rejected: **keeping the thread.** The two reasons for it are gone, and the
costs above are what is left.

## Consequences

**One more scope in the manifest, and a reinstall for an app that lacks
it.** `channels:manage` covers creating, archiving, inviting, the topic and
the purpose.

**A job costs a few more requests before its container exists**, and any of
them failing fails the job the way a refused announcement does today,
visibly and before anything runs.

**A name can only collide on its suffix**, and when it does the platform
says `name_taken` and the job fails saying so.

**What the last release wrote is bridged the way 0060 bridged threads.** A
job's thread, roomed or not, opens as no room: it was a thread, and a
thread is not somewhere this rule can answer. The job keeps its record. A
foreman's errand keeps its thread, because a foreman still answers in one.

**Reversing** means the thread model back and a record for each job that
already has a room; rooms cannot become threads, so the reversal keeps them
as rooms and stops making new ones.

**Revisit if** a project's own rooms are private, which wants the private
variant; if a channel arrives with no notion of a room, for which a thread
is the unit again; or if the number of rooms a project makes in a day
outgrows what archiving keeps tidy, which is the sidebar's problem before
it is this project's.
