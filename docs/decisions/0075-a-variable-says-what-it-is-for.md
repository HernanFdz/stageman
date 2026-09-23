# 0075 — A variable says what it is for

## Status

Accepted. Amends
`docs/decisions/0046-a-projects-variables-are-carried-never-read.md`, whose
variables now carry a note beside the value, and keeps its rule: nothing
here reads a value, and the note is prose for the agent, not a fact this
project acts on. Taken during the dashboard pass of
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`, at
the form that takes a project's variables.

## Context

A project's variables reach a job as its environment and reach its agent
as a sentence in the kickoff: the names, and "nothing here knows what any
of them is for, so follow whatever the repository says about them". That
was honest, because 0046 made a value opaque here, and it was also the
least an agent could be told: the operator who typed `DATABASE_URL` knew
it was the staging database and read-only, and had nowhere to say so. A
repository sometimes says; the person who set the variable always could.

The form took variables one row at a time, a name and a value each. What
an operator has is a file: a `.env`, one `NAME=value` per line, some
quoted, some prefixed with `export`, with comments above the lines that
matter. Typing a dozen of those into a dozen rows is where a value lands
in a name box, which is the mistake the refusals by row exist to catch.

Two things were checked before deciding how a file gets in. The
framework's paste event carries no text, so a paste cannot be read as it
happens; and the clipboard's own reading interface asks a permission that
each browser grants differently, which is a prompt in the middle of a
form. What a browser does reliably is put pasted text into a text area.

## Decision

**A variable carries a note saying what it is for, told to the agent
beside its name; and a project's variables can be pasted as a `.env`
file, comments above a line becoming its note.**

- **The note is prose for the agent.** It travels in the kickoff, one
  line per variable — `DATABASE_URL — the staging database, read-only` —
  in place of the sentence that said nothing here knew. It is never read
  by this project, never delivered into the container, and never a value:
  the value stays sealed and the note stays in the clear beside the name,
  which is the boundary a name already crosses. A variable with no note is
  named alone, as before.
- **A file is pasted into a dialog of its own**, opened from the end of
  the variables' label line beside the control that adds one row — a
  dialog as starting a job is, because a paste is a transaction with one
  action, taken and gone, and not something that stays on the page. Each
  line that is `NAME=value` becomes a row: `export` before the name is
  dropped, a value in double quotes is read with its escapes and one in
  single quotes as it stands, a comment may follow either, an unquoted
  value ends at ` #`, and the comment lines just above a line are its note,
  joined; a blank line parts a comment from what follows it, so a section's
  heading is nobody's note. What the dialog cannot take is left in it, as
  it was, with the dialog open, so nothing is dropped without being seen.
- **The browser only fills rows.** What is refused is refused as before,
  by row, once the form is saved: a name a container cannot be given, a
  name this project delivers itself, two rows with one name, a new
  variable with no value. A paste changes how rows are filled and nothing
  about what is accepted.

Rejected: **reading the clipboard.** The paste event carries no text, and
the reading interface is a permission prompt whose shape is each
browser's; a text area asks nothing of anybody.

Rejected: **a box folded into the page**, under the variables' line and
parted from the rows by a hairline. Built first, and wrong for the same
reason a disclosure was right for a job's instruction: that is content
that stays, and this is a transaction; a box that stays open beside the
rows it filled is a box somebody has to close.

Rejected: **the note as a kind or a flag.** 0046 refused a per-variable
flag saying whether one is secret, for a silent and unrecoverable
mistake; a note is words, carries no rule, and changes nothing here.

Rejected: **telling the agent a value, or inferring a note from one.**
A kickoff is kept on the job in the clear. Nothing that could hold a
value reaches it, which is why the handout hands the kickoff names and
notes and has no method that hands it a value.

Rejected: **taking more of the `.env` dialect** — multi-line quoted
values, interpolation of one variable into another, a `.env.local`
beside it. Each is a rule a person would have to know this form keeps;
a value that needs one can be typed into its row.

## Consequences

**The sealed form changes shape, and a bridge reads the last release's.**
A variable on disk was a sealed value; it is a sealed value and a note
now, and a bare sealed value opens as one with no note. The older-file
test carries one, which is what proves it.

**The kickoff's paragraph moves**, and its snapshot with it: a list of
names with their notes where anybody wrote one, then the credential
warning that has not changed.

**The wire carries the note both ways**: on a draft's row, and on a
project's variables as the list shows them, by name and note and never
by value. The row gains a third box.

**Reversing** is dropping the note: the sealed form reads either shape
already, and a file written with notes opens without them once the field
is gone.

**Revisit if** a note needs to differ per job rather than per project,
which is a job-level thing the kickoff would take from the work instead;
or if operators arrive with files the parser leaves half in the box,
which is when the dialect above is too small.
