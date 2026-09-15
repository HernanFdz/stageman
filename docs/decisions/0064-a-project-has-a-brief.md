# 0064 — A project has a brief, and the foreman is told it every turn

## Status

Accepted. Goes with
`docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`, which is
what makes a place for policy necessary, and follows
`docs/decisions/0048-a-job-runs-on-a-kit.md` in saying a project's
configuration to the foreman on every turn rather than once.

## Context

Everything a foreman was told about how to behave was a prompt line this
project hard-codes, the same for every project: decide rather than ask,
start a job when something needs doing, choose a kit deliberately. That was
enough while the only thing a foreman heard was a person asking for
something. A signal is different: an issue filed may or may not deserve a
job, an alert below some level may be noise, a pull request a person opened
may or may not be one to review, and a job may need to act as a particular
account. None of that is this project's to decide, and none of it is the
same from one project to the next.

The foreman's session lasts as long as the project does, and the kits are
said on every turn for exactly that reason: said once at the start, a list
is right until somebody edits it and wrong thereafter, with nothing to
notice.

## Decision

**A project has a brief: free text the operator writes for its foreman**,
edited on the project form, and said to the foreman on every turn beside the
kits, as the operator's standing instructions for whatever the turn is
about. An empty brief says nothing, and a project with none gets a prompt
byte-for-byte what it was before briefs existed.

**It is where policy lives.** "Our jobs act as the machine user X", "review
every pull request a person opens", "ignore alerts below error", "start
nothing for an issue labelled question" — sentences this project cannot
write, said in the operator's words and followed by the foreman's judgement.
Nothing here enforces a brief; the foreman reads it.

**It travels in the clear.** A brief is written to be read, like a kit's
description, and is kept beside them rather than sealed. An operator who
puts a credential into one has put a credential into a prompt, which nothing
this project does could protect.

Rejected: **prompt lines per project in code.** A policy that differs per
project is configuration, and a line of code cannot differ per project
without becoming one.

Rejected: **saying the brief once, in the opening.** It goes stale the way a
kit list said once would, and a brief is the thing an operator edits most:
the first signal that was judged wrong is the moment the brief changes.

Rejected: **a brief per room, or per kind of signal.** Finer than anything
has asked for, and a brief in prose can already say "in the alerts room,
ignore anything below error". Structure is the layer to add when prose stops
being enough.

Rejected: **a file in the repository the foreman reads.** A foreman has no
checkout, by `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`, and
a repository's documents are a job's business. The brief also has to exist
before any job has run, which a file in a repository nothing has cloned
cannot.

Rejected: **putting the brief into every kickoff as well.** A job's
instruction is composed by the foreman, which has read the brief, so what a
job needs of it arrives through the foreman's judgement; carried whole, every
job would be told policy meant for the deciding, and told it in words the
operator wrote for a different reader.

## Consequences

**Every turn grows by the brief**, so a long brief costs tokens on every
message and every signal. That is the operator's choice, and the form says
so.

**A wrong brief is followed.** It is an instruction, and the foreman is told
it outranks nothing but is standing: a person correcting the foreman in a
room is still a message, and still its own turn.

**What the last release wrote opens with an empty brief**, which is the true
answer: no operator has written one.

**Reversing** is one field on the project, one on the form, and one
paragraph of a prompt.

**Revisit if** the brief outgrows a text box, which wants structure; if every
project on an instance wants the same sentences, which wants an instance
brief beneath the project's; or if a job turns out to need the brief as
written, which puts it in the kickoff after all.
