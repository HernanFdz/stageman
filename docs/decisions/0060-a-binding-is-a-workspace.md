# 0060 — A binding is a workspace, and a mention anywhere reaches the foreman

## Status

Accepted. Supersedes the routing rule of
`docs/decisions/0031-a-mention-is-what-makes-it-ours.md` where it concerned a
thread belonging to no job, and the part of
`docs/decisions/0027-a-channel-is-not-a-platform.md` that made the address the
place a project is listened to. Keeps
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md`'s one app per project
and hardens it from a preference into a must. Depends on
`docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`.

Every claim below about what the platform does was measured on 2026-09-15
against a real workspace, with the app installed and a person typing.

## Context

A project was bound to one Slack channel, and that channel was the whole of
where it listened: a message anywhere else went to nobody, and the routing
rule found the project by matching the message's channel against every
binding. So talking to a project meant going to its channel, and a job's
thread had to hang from a message posted there.

Three measurements changed what the rule could be.

**Every room the app has been invited to arrives on the same socket.** The
event stream is the app's, not a channel's, so listening in any room costs
nothing beyond somebody typing `/invite`. A mention in a room the app is
*not* in makes the platform offer the person an invite, and if they accept,
the message that prompted it is delivered — but only as an `app_mention`
event, never on the message subscription.

**A person's mention arrives twice.** Once as a `message` event and once as an
`app_mention` event, with the same identifier, and the second carries the
thread identifier when the mention was inside a thread. What this instance
posts itself arrives once, as a `message` event carrying its own bot
identifier.

**Two connections on one app split the events between them.** A second
project sharing an app-level token would hear half of what is said, and so
would the first, with nothing anywhere saying so.

## Decision

**The project a message belongs to is the socket it arrived on.** A binding is
an app installed in a workspace, that app hears every room it has been
invited to, and nothing about a message's room decides whose it is.

**People are read from `app_mention` and nothing else.** A person's copy on
the message subscription is acknowledged and dropped as the duplicate it is.
What this instance said is recognised by its bot identifier, asked of the
platform once per connection beside its user identifier — not by being a bot,
which is what the reader assumed while no other bot was ever read.

**A mention in a job's thread is that job's. A mention anywhere else, at the
root or inside a thread, is the foreman's, and it is answered where it was
said.** A foreman's inbox already records the thread each message arrived in,
so an answer lands in the right place however many turns behind it is, and a
foreman's session remembers earlier turns, so a person can go on talking to it
in one thread.

**The address survives as the home room**, where a job's thread is opened,
and is consulted for nothing else. It leaves with the record that gives a job
a room of its own; carrying it one more step is what lets a job started from
the dashboard have somewhere to speak until then.

**One app per project is a must.** The setup instructions say so, and the
reason is the third measurement.

Rejected: **reading people from the message subscription, as before.** It is
what exists, it needs no new event, and it loses the one message a new room
sees first — the mention that prompted the invite — which is exactly the
moment a person decides whether this works. Reading both would deliver every
mention twice, and telling the copies apart needs the instance to remember
what it has recently heard, which is held state for a problem the split
removes outright.

Rejected: **keeping 0031's fixed answer for a thread belonging to no job.**
That record's reason was that a foreman "cannot be held in conversation",
because by the time it answers it may be several turns on. It can: the errand
carries its thread and the session carries its memory, and the objection
described a foreman without either. What the fixed answer actually taught was
that replying to the foreman where it answered you does not work, which is the
most natural move available and the one this rule now makes work.

Rejected: **an app for the whole instance**, again. 0029 rejected it as a
credential concentration; it is now also measured to split the stream.

## Consequences

**The manifest must subscribe to `app_mention` with the scope that goes with
it**, or people are never heard: the app connects, greets, and hears nobody,
with nothing said anywhere. `README.md` carries the manifest for that reason,
and the instance cannot check a subscription — the platform offers no way to
ask.

**A fourth routing outcome disappears.** There is no thread belonging to
nobody any more, so nothing is refused with a fixed sentence, and the
acknowledgement a foreman posts stops teaching a rule that is no longer true.

**Two things the last release wrote are dropped on opening, and their owners
are kept.** A binding without the credential that listens becomes no binding,
per 0059. A job's thread recorded without its room becomes no thread — the job
keeps its record and loses its place to be answered in, which is the loss
`docs/conventions.md` §4 permits and the one the next record would inflict
anyway. A foreman's inbox recorded without rooms is emptied for the same
reason, and the person whose message was in it is not told; that window is a
restart during an upgrade with a message in hand, and it is accepted.

**Reversing** is the routing rule, the reader of a frame, and the manifest;
nothing recorded changes shape back.

**Revisit if** a way of talking to this instance arrives that has no mention —
a direct message, most obviously — which needs a second reason for a
person's message to be read; or when a job has a room of its own, which is
planned and takes the home room with it.
