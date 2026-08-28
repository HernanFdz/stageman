# 0019 — A project's tooling is the project's business

## Status
Accepted. Answers half of the open question about what a job's image needs, and
narrows the other half to a single question.

## Context

The first real job this system ran proposed a change to this repository, and
said so in its own pull request: it could not run the gate, because neither
`just` nor `cargo` was in the image. That was true — the image carries an
agent, git and the repository host's tool, and no language toolchain at all.

The tempting reading is that the image should carry more. It is the wrong
reading, and the reason is already written down one level away.
`docs/decisions/0001-drive-an-existing-coding-agent.md` refused to become a
coding agent, on the grounds that the interesting work is orchestration and the
agent is somebody else's product. The same argument applies to a project's
toolchain: what a project needs in order to be built and checked is the
project's own domain, changes on its own schedule, and is knowable by exactly
one party — the project.

## Decision

stageman does not decide what tools a project needs, and does not install them.
A project declares its own prerequisites, in the place whoever is about to work
on it will read.

That declaration is ordinary practice rather than anything to do with agents. A
repository that does not say what it needs is hard for a person to pick up too;
it is only that a person will ask somebody, and an agent will not.

## Rejected: carrying a common set of tools in the image

Install a language toolchain and the usual build tools, so that most jobs can
verify their work.

It loses on which set. This project is Rust, so it would be a Rust toolchain —
and the next project is Python, and the one after needs a database to run its
tests. A platform that accumulates opinions about projects it has never seen is
the shape 0001 refused, arrived at from the other direction. The set that is
*common* is never the set that is *needed*, and the gap is invisible until a
job fails inside it.

It also makes the image grow without bound in exchange for being wrong more
slowly.

## Rejected: inferring the toolchain from the repository

Read the manifest, recognise the ecosystem, install accordingly. Clever, and it
would have worked for the job that exposed this.

It loses on the failures it cannot see. A project with an unusual build gets a
confident wrong answer instead of an honest absence, and a confident wrong
answer is the more expensive of the two — the job proceeds, produces something
plausible, and nobody learns why it was wrong. It also makes this system's
behaviour depend on parsing files it does not own and cannot version.

## Consequences

**The declaration has to be somewhere the reader actually looks.** For this
project that is `AGENTS.md`, which is the first thing an agent is told to read
and which, until this change, opened by telling it to run `just check` without
ever saying how to have `just`. Fixed in the same change, because a decision
about where prerequisites go is worth nothing if the project taking it does not
follow it.

**Declaring is not providing, and provision is still open.** A project saying
it needs a toolchain does not put one in a container. What remains of the
question in `docs/open-questions.md` is exactly one thing: may a project carry
an image of its own, built from an agent's? This record does not answer that,
and deliberately: it says who decides, which is the half that was confused.

**Until it is answered, an agent may propose a change it could not verify.**
That is a real cost and it is not paid by the image. It is paid by the barriers
around the change — a gate that runs in continuous integration, and a rule that
a pull request cannot merge until it passes. Those are the project's to set up,
which is the same sentence as the decision above.

Reversing means putting tools in the image after all, which is mechanical. What
it would not undo is the expectation that stageman knows what a project needs,
and that expectation is expensive to retract once anybody has relied on it.

Revisit if a project ever cannot express its prerequisites — something that
must be installed differently per machine, or a credential needed at build
time. That would mean the declaration cannot be static, which is a different
problem from this one.
