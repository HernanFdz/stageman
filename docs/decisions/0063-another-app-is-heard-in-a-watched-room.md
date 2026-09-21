# 0063 — Another app is heard in a watched room

## Status

Accepted. Adds the one way a message is read without a mention, beside the
rule `docs/decisions/0031-a-mention-is-what-makes-it-ours.md` and
`docs/decisions/0060-a-binding-is-a-workspace.md` keep for people, which is
untouched. Depends on `docs/decisions/0061-a-job-has-a-room-of-its-own.md`
for where a job started from a signal is announced, and on
`docs/decisions/0062-what-this-instance-says-is-markdown.md` for the reaction
that says a message was received. What the foreman is told to make of a
signal is `docs/decisions/0064-a-project-has-a-brief.md`. Since
`docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
an agent's words are posted only in the room its speaker owns, so the
silence this record decides for a signal holds by construction.

Every claim below about what the platform does was measured on 2026-09-15
against a real workspace, with the GitHub app subscribed to a repository and
an issue and a pull request opened, commented on, closed and reopened.

## Context

The loop `docs/vision.md` §1 exists to close begins with a signal nobody
addressed: an issue filed, an alert fired, a pull request opened. On a
channel those arrive as messages posted by an app, and an app mentions
nobody. Since 0060 a person is read from the platform's own mention event
and from nothing else, and another app's message was acknowledged and
dropped — deliberately, with a note in the reader that what to do about
other apps was a decision of its own. So the foreman could be asked to do
anything and told about nothing.

What the platform delivers was measured.

**Another app's message arrives on the message subscription**, in a public
room and a private one alike, carrying the app's bot identifier, a user
identifier that is the app's bot user, and a bot profile with the app's
name. The GitHub app's messages carry an empty text and everything in one
attachment: a fallback line, a pretext, a title that is a link, the issue's
body as text on the message that opened it, and a footer that links the
repository. A pull request arrives in the same shape. The platform's own
metadata on the message names the notification's kind, number and URL in the
app's own vocabulary.

**A follow-up arrives as a broadcast under the original.** An issue closed
or reopened, or a pull request closed, is a message with the
`thread_broadcast` subtype, its own identifier, the original's identifier as
its thread, a copy of the original as its root, and an attachment with a
pretext and a title and no text. The broadcast itself carries no bot
profile; its root does. Each follow-up arrives beside two edits — the
original's card and the broadcast's recoloured — delivered as
`message_changed` events. A comment on the issue produced no message at all:
what a room hears is what the app chooses to post.

**A message built from blocks carries its content in the blocks and a
fallback in its text**, measured by posting one: the text field holds what
was given as the notification fallback, and the headings, sections, fields
and context lines are in the blocks.

**This instance can act on another app's message with the scopes it already
has.** A reaction added with the bot token lands on an app's message at the
root and on its broadcast in a thread, and a Markdown post with the
original's identifier as its thread lands under it. Nothing in the manifest
changes.

**A room cannot be named with those scopes.** Listing rooms and asking a
room's name both need scopes the manifest does not grant, and an app gains a
scope only by being reinstalled.

## Decision

**A room is watched when a person tells the foreman so, in it.** The foreman
is offered two tools, `watch_room` and `stop_watching`, each acting on the
room the message being answered was said in and taking no argument: the
place comes from the turn's credential, as everything a tool does to a room
does, so a foreman cannot be talked into watching a room it was not asked
in. Which rooms a project watches is recorded on the project, kept across
restarts, and shown on the dashboard as the platform's identifiers, read-only.

**In a watched room, every message from another app is a signal for the
foreman.** It is read into text by the channel crate — the blocks when there
are any, else the text; then each attachment's pretext, title, text, fields
and footer, or its fallback when it has none of those — in the platform's own
markup, untranslated, as a person's mention already arrives. It is handed to
the foreman as a turn of its own, framed as the app's rather than as a
person's: *GitHub posted this in a room you watch*. The name is the one the
platform gives the app — its bot profile's, its root's for a broadcast, a
classic bot's username, and its bot identifier when it has none of those. A
broadcast under an earlier message is a signal in that thread, so the
foreman's answer, if it gives one, lands under the message the follow-up
followed. An edit is not a thing said, and the edits that arrive beside every
follow-up are dropped as edits always were.

**People are still read only through a mention, everywhere.** A person's
plain message in a watched room reaches nobody, exactly as before. What a
watched room changes is what is done with an app's message, and nothing
else.

**A signal is received the way a person's message is**: an eyes reaction
once the inbox holding it is on the disk, and a check mark when the turn
ends. The foreman is told that the reaction already says it looked, and to
speak only if it acted or a person needs to know something. A job started
from a signal records nobody as having asked for it, invites nobody into its
room, and is announced in the signal's thread, which is where a person
following the app's post will look.

**Whether an app can mention this instance was not measured**, and the
reader does not depend on the answer: a mention event carrying another app's
bot identifier is read as that app's message, subject to the same rule, so
that an app cannot reach the foreman past it either way.

Rejected: **an allowlist of apps per project.** Finer, and the obvious
design — hear GitHub and not the deploy bot. It needs a name for each app,
and what an app is called is in a profile that a follow-up broadcast does not
carry, so the list would have to be kept by bot identifier, which nobody can
read off a screen. A room is what a person can point at. This is the layer to
add if one room ever mixes an app worth hearing with one worth ignoring, and
until then the brief can say which to ignore.

Rejected: **hearing every app wherever the app is invited.** No tool and no
state, and silent spend: every signal is a turn of the foreman's agent, so a
room with a chatty bot in it would bill per message from the moment somebody
invited the app to ask it a question there.

Rejected: **reading the platform's metadata on the message.** The GitHub
app's carries the kind, number and URL of what it is about, structured. It
is one app's vocabulary, the title already carries the link, and a reader of
it would put an app's shape into this project, which is the thing
`docs/conventions.md` §3 keeps behind the channel crate's boundary.

Rejected: **typing a room's identifier into the project form.** A person has
to find the identifier, and a form cannot tell them the app is not in that
room. Being asked in the room proves the app is there and hands over the
identifier without anybody reading it.

Rejected: **showing a watched room by name on the dashboard.** Measured to
need scopes the manifest does not grant and every existing app to be
reinstalled, for a label. The identifier is shown, and the room's name is
what the platform shows wherever the identifier is linked.

## Consequences

**No scope changes.** An app installed from the manifest as it stands hears
other apps in the rooms it is in, reacts to their messages and answers in
their threads.

**A signal costs a turn**, which is what the brief is for: which alerts to
ignore is a sentence there, not a rule here.

**What a watched room hears is what the app posts**, which is the app's
subscription and not this project's. The GitHub app posted nothing for a
comment, so a comment is not a signal until somebody subscribes the app to
comments.

**Two states of the same fact cannot drift**: the room a signal came from is
compared against the watched set by channel and identifier, and a job's room
is never watched, because the tool is only ever called from a room the
foreman was mentioned in and a mention in a job's room is that job's.

**What the last release wrote opens unchanged**: no room watched, and every
message in an inbox a person's.

**Reversing** is the reader of a frame, one branch of the routing rule, two
tools and a set on the project; nothing recorded changes shape back.

**Revisit if** one room mixes an app to hear with one to ignore, which wants
the allowlist above; if an identifier on the dashboard is not enough to tell
watched rooms apart, which wants the listing scopes and a reinstall; if a
room produces more signals than a turn each can absorb, which wants a digest
rather than a turn per message; or if something worth hearing arrives on the
platform as something other than a message, which the reader has nowhere to
put.
