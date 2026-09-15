# 0062 — What this instance says is Markdown, and a notice says what it knows

## Status

Accepted. Changes how every message this instance posts is rendered, and
what each of its own notices says; changes nothing about where they go,
which `docs/decisions/0061-a-job-has-a-room-of-its-own.md` settles.

Every claim below about what the platform does was measured on 2026-09-15
against a real workspace.

## Context

Everything this instance posted went out as plain text in the platform's
own markup, a dialect that looks like Markdown and is not: single asterisks
for bold, angle brackets for links, no headings, no lists, no tables. The
agents this project runs write Markdown by habit, because everything they
were trained on does, so what an agent said arrived with its emphasis as
literal asterisks and its headings as pound signs. Nothing translated,
because a translator between two dialects is a second thing to keep right.

The notices this instance posts on its own behalf were written when it knew
less. The one that says an agent stopped was written to say nothing about
how it went, on the reasoning that the instance could not tell an answer
from a question from an agent giving up; that stopped being true with
`docs/decisions/0055-a-job-says-why-it-stopped.md`, which records the
agent's own account of why, and the notice never caught up. The
acknowledgement a foreman posts on receiving a message was a whole message
because a thread had to be opened by posting something, which
`docs/decisions/0060-a-binding-is-a-workspace.md` made untrue: the message
a person posts is the thread, and the acknowledgement is one more line in
a room's timeline that says nothing a person acts on.

Measured: a message posted through the platform's Markdown parameter, from
this app with no special feature enabled, is translated by the platform
into its native blocks — headings, lists, a table, a highlighted code block
— with references to rooms and people rendered as such; the platform
writes the notification fallback itself; the limit is twelve thousand
characters per message. A reaction added by the app appears on the message
it names.

## Decision

**Everything this instance posts is Markdown, and it goes out as Markdown.**
The adapter posts every message through the platform's Markdown parameter,
whether the text was composed here or by an agent, and nothing translates.
Agents are told, where they are told about the tool that speaks, to write
Markdown because it is rendered.

**A notice says what the instance knows, and nothing it does not.** The
notice that a job stopped carries the reading the agent gave — waiting for
an answer, ready for review, stopped by an operator, failed with the reason,
or stopped without saying — and names the person who asked for the job when
it is waiting on them. The notice that a foreman could not handle a message
carries the reason. Every notice says how to reach the job or the foreman
from where it is read, since since 0061 that is one rule everywhere: mention
it.

**Receipt is a reaction, not a message.** A message for a foreman gets an
eyes reaction the moment the inbox holding it is on the disk, and a check
mark when the turn that handled it ends. A room's timeline then holds what
people said and what the foreman answered, and nothing else. The one
exception is a restart: a person waiting since before this instance died is
told so in a line, because a reaction cannot say that.

Rejected: **translating Markdown to the platform's dialect.** It is what
would have been built had the platform not rendered Markdown, and it was
measured unnecessary. It would be a second dialect to keep right, wrong in
the corners — nested lists, tables, code in links — that agents reach for
most.

Rejected: **building the platform's block structure by hand for notices.**
A layout language for eight sentences, and a second rendering path beside
the one every agent's words take. One path, and a notice is Markdown like
everything else.

Rejected: **keeping the acknowledgement as a message.** It said "got it",
which a reaction says as well, and a rule about where to say the next thing,
which is no longer true. What it cost was a line under every message a
person sends, for ever.

Rejected: **a reaction for the stopped notice too.** A reaction cannot
carry a failure's reason or name the person being waited on, and a change
in a job's state is worth a line in the job's own timeline.

## Consequences

**The manifest needs the scope that reacts**, and an app without it is
told so in the log on every message and heard nothing worse: a reaction
that fails is a notice that fails, and changes nothing.

**A message over the platform's limit is refused by the platform**, and an
agent whose tool call was refused is told so in the tool's answer, which is
the same thing that happens to a message the platform refuses for any other
reason. Nothing here splits a long message, because nothing this instance
composes is long, and an agent told the limit can keep to it.

**Every text is still asserted whole**, per `docs/conventions.md` §4, which
now says so of every text this system posts rather than of kickoffs alone.

**Reversing** is the shape of one request body and a handful of strings;
nothing recorded changes.

**Revisit if** a channel arrives that renders no Markdown of its own, which
puts a translation in that channel's adapter and nowhere else; or if
reactions turn out to be missed by the people who need them, which would
put a short line back where the reaction is.
