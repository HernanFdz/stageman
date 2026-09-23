# 0074 — A job's identifier is its name

## Status

Accepted. Amends `docs/decisions/0061-a-job-has-a-room-of-its-own.md`,
whose room is named after the identifier alone now rather than after a
title and a prefix of it; amends
`docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`, whose bottom
label is read by this record's grammar rather than as a UUID; and keeps
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` to its
word that addresses keep the identifier, by making the identifier worth
reading. Taken during the dashboard pass that record began, when a job's
row was to be given a title and there was nothing to give it.

## Context

A job was identified by a UUID, minted from the seeded generator, and that
identifier was everywhere a job is met: the key in the instance's file, the
suffix of its container's name, the bottom label of its tunnel's host, and
the segment of its page's address. Three places read it back — the waking
sweep, from a container's name; the router, from a request's host; the
file, from its keys — and each read it as a UUID.

The one readable name a job had was spent as it was made. The foreman's
tool requires a title, "a few words naming the job, as a person would read
them in a sidebar", and a job started by hand takes the first words of its
work; 0061 folds that title into the room's name between the project's and
eight hex of the identifier, and nothing keeps it. So a person met the same
job as `closed-loop--fix-login-timeout--3f9a2c1b` in the sidebar, as
`3f9a2c1b-…` in the address bar, in `docker ps` and in the tunnel's host,
and as its reason on the dashboard, which is prose about why and reads
badly as a title. A message on Slack naming a job had no word to name it
by.

Four things constrain what one string can be, if it is to be the same
everywhere. A room's name is lowercase letters, digits, hyphens and
underscores, eighty characters at most, and 0061 already gives the
project's part twenty-four. A DNS label is letters, digits and hyphens,
sixty-three at most, with none at either end. A container's name is the
same alphabet and more, with no limit that matters. And a UUID, lowercase,
is hex and single hyphens — which is the observation the compatibility
below rests on.

Measured on 2026-09-23: `fix-login-timeout--3f9a2c1b.localhost` and
`ab--3f9a2c1b.localhost` both resolve at the system resolver to the
loopback addresses, the second with a hyphen pair in its third and fourth
characters, which is the shape internationalised names reserve.

## Decision

**A job's identifier is its name: the title folded to a slug, two hyphens,
and eight hex minted for it** — `fix-login-timeout--3f9a2c1b`. One string
is the key in the file, the container's name after its prefix, the tunnel's
host label, the tail of the room's name, the address of the page, and the
word a message uses.

- **The grammar is one, and it is the intersection.** Lowercase ASCII
  letters and digits, in runs parted by one or more hyphens, with no hyphen
  at either end, and no longer than fifty characters. Every identifier the
  last release wrote is a UUID, which is thirty-six characters of lowercase
  hex and single hyphens, and so is a name under this grammar as it stands.
  Nothing is renamed and nothing is bridged: the type accepts one shape and
  both fit it.
- **Nothing parses it.** The title in it is for a reader and the hex is
  what made it unique when it was minted; no code extracts either. The one
  reader of its shape is the validator at each boundary — a request's host,
  a container's name, a page's address, the file — which says whether a
  string is a name and never what it means.
- **The title's slug is capped at forty, on a word.** The fold is the one
  0061 gave the room — lowercase, every run of anything else one hyphen,
  none at either end — and a title longer than forty is cut back to the
  last whole word that fits, so a name never ends in half of one; a title
  that is one long word is cut where it is. Forty, and the suffix's ten,
  keep the whole under a DNS label's sixty-three and, beside a project's
  twenty-four, under a room's eighty. A title with nothing left in it after
  folding reads as `job`, so that every name has its two parts.
- **Unique by the suffix, checked at minting.** The eight hex come from the
  seeded generator, and a name already in the instance is minted again — a
  bounded number of times, and a job that cannot be named fails saying so,
  which no seed has reached — so that a scenario reproduces and a clash
  never reaches the file. On the platform an archived room keeps its name
  for ever, and a clash there is still answered as 0061 says: the job
  fails, saying so.
- **The room is named `<project>--<identifier>`.** The project's part as
  before; the title and the prefix of the identifier that stood beside it
  are now inside the identifier. The foreman's room is untouched.
- **A job started by hand is titled by the first words of its work, unless
  a person gives one.** The form that starts a job gains a title, optional,
  with the default shown as its placeholder as the work is typed.

Rejected: **a title kept beside the identifier**, shown wherever a job is
met and left out of the address, the container and the host. Cheaper, and
it keeps two names for one thing — with the opaque one in exactly the
places a person copies from: the sidebar, `docker ps` and the address bar.

Rejected: **the name from the reason.** The reason is prose about why, and
its first words are "a person asked in", which names nothing. The title is
the foreman's one answer to *what would you call it*, and it costs a field
the tool already has.

Rejected: **the slug without the suffix.** It would need names to be
unique, which is the rule on names 0070 refused for a project's sake; the
suffix makes a name unique without asking anything of the title.

Rejected: **renaming a job the last release wrote.** Its title was spent on
its room and is nowhere to be read back from; and a container, a room and
a bookmark carry the old name and would each need finding. A job named by
a UUID is shown as one, and is gone within the compatibility window.

## Consequences

**The type stops being a UUID and stops being `Copy`.** A name is text,
cloned where it was copied; the core crate validates it on construction
and on reading the file, and a file with an identifier outside the grammar
is refused, which only a hand-edited file can be.

**The router's stranger shrinks.** A bottom label that fits the grammar is
read as a name, and one naming no job is answered as an unknown job
already was, with a line in the log; the case 0042 called a stranger is
left to a label no name could be, such as one with a dot or an underscore.

**The channel crate takes a name and no title**, and the fold moves to the
core crate, where the name is minted; the channel keeps only the budget
for the project's part.

**The wire carries the name as the identifier it already carried**, so a
page's addresses change shape and nothing about how a page reads them.
Every text that names a job — the tool's own description of a title, the
answer it gives — says the name, and the snapshot tests of
`docs/conventions.md` §4 move.

**The older-file test already opens a job keyed by a UUID**, which is now
the test that the old shape is a name.

**Reversing** is a type that is a UUID again, and a title beside it; every
name minted under this record would then have to be read past, which is
the migration this record avoids by accepting both shapes.

**Revisit if** a job's name ever has to change after it is made, which is
when a name beside the identifier becomes right; if a second source of
jobs arrives with no title to give, which is when the reason or the work
would have to name it; or if two instances sharing a workspace are seen to
clash on a suffix, which is when eight hex stopped being enough.
