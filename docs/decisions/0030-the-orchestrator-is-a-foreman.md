# 0030 — The orchestrator is a foreman

## Status
Accepted. Renames the concept named in every record numbered below this one:
where those say *orchestrator*, they mean *foreman*, and they are left alone
because a record is append-only and rewriting one would edit reasoning that was
given at the time.

## Context

The per-project thing that reads what a person says and decides what to do
about it has been called the orchestrator since before any of it was built. It
is about to acquire a container, a lifecycle, an inbox and a state machine, and
naming is cheapest before that rather than after.

Two things were wrong with the old word, and only one of them is taste.

It is a **mechanism word for a role**. `docs/conventions.md` §2 prefers words
where somebody is on the other end — that is why a *channel* is not an
*integration*. "Orchestrator" is architecture vocabulary: it describes a box in
a diagram, and it is vague enough to absorb any behaviour added to it, which is
exactly the property that lets a concept drift without anybody noticing.

And it **over-claims**. To orchestrate is to sequence and coordinate. This
thing does not: it reads one message at a time, judges it, and may assign work
to somebody else. The name promised a conductor and delivered a decider.

## Decision

**It is a foreman**, one per project.

A foreman does not do the work; they decide who does. The word pairs with
*job*, which is what it assigns, and it is a role rather than a mechanism, so
"why did the foreman do that?" is a question with an answer. Under a project
called stageman the register is right too: the instance is the management, and
each project has somebody on the floor.

Two words arrive with it, because the lifecycle needs them: an **inbox** of
messages waiting for a foreman, and a **turn** as the unit of one message being
handled. Both are in `docs/conventions.md` §2.

Rejected: **supervisor** and **coordinator**, which describe watching and
sequencing — the same over-claim in a different coat. Rejected: **dispatcher**,
which names only the third of the three things it can do, and would make
answering a person look like the exception. Rejected: **keeping
orchestrator**, on the grounds that a rename is churn — true, and the churn is
one afternoon now against a word that would be load-bearing in every file added
from here.

**triage** goes with it. It was the word for what a foreman does while
thinking, and it survived as `Handout::for_triage` beside `Handout::for_job` —
one naming an activity and one naming an actor, which is the inconsistency that
made it worth noticing. It is `Handout::for_foreman` now.

## Consequences

A crate, a directory, a package, a field on every project, and thirty-eight
sentences of documentation. All of it mechanical, all of it caught by the
compiler except the prose.

**Every decision record numbered below this one keeps the old word.** That is
the append-only rule working rather than a gap: the reasoning in those records
was given when the word was *orchestrator*, and a reader who meets one needs to
know what it meant, not what it would have been called later. This record is
the pointer, and it is why the vocabulary entry names it.

One citation was repointed rather than left: `docs/decisions/0027-a-channel-is-not-a-platform.md`
names `for_foreman` by its identifier, and `just drift` resolves every
backticked identifier in the docs against the source. A citation that no longer
resolves is drift like any other, so the *name* was corrected while the
reasoning around it was not touched. That is the line: an argument is
append-only, a reference to a symbol is a fact about the code and has to stay
true.

Reversing this is the same afternoon in the other direction, and would need its
own record. Revisit if the foreman ever stops assigning work and becomes
something that only answers — at which point it is not a foreman and the word
would be wrong again, in the new direction.
