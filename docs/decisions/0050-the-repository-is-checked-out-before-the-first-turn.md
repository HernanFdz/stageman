# 0050 — The repository is checked out before the agent's first turn

## Status
Accepted. Reverses half of
`docs/decisions/0016-the-agent-clones-the-repository.md` — who makes the
clone, and when — and keeps the other half: nothing mounts or shares a
repository, and the checkout is made inside the container, with the platform's
own tool and the job's own credential. Changes what *workspace* means in
`docs/conventions.md` §2 for the second time.

## Context

0016 had the agent clone the repository itself, on the reasoning that anything
delivering one would make a repository mandatory, and a job that needs none —
one settling a question about a platform, say — should be an ordinary case.
That reasoning was sound about the mechanisms it rejected, a mount and a
volume, and it was right that nothing in this system should share files across
the boundary a container draws. It missed something about the agent.

**A coding agent is built to be started inside a repository, and everything it
reads about a project is read when it starts.** Measured against the pinned
adapter, in its own source: a session is created with the workspace as its
working directory, and at that moment the agent resolves its settings from the
user, the project and the local tiers, loads the project's memory files,
registers its hooks and discovers its skills. The watcher that would notice a
project's settings appearing later is installed only on a directory that
already exists. So a session created in an empty workspace has none of the
project's instructions, and a clone made inside the first turn does not bring
them: the memory file is not read, the hooks do not run, the skills are not
there, and the permission defaults the repository asked for are not in force —
until the next turn, when a resumed session starts over with the checkout
present and picks all of it up.

That is not hypothetical. The first job this system ran on a second project
opened a pull request whose commit carried no scope where that repository's
convention requires one, and whose description was the agent's own default
shape rather than the repository's template. The first job it ever ran, on
this repository, knew about the gate — because it went looking and found
`AGENTS.md`, which is luck rather than mechanism. An agent that reads a
repository's instructions only if it thinks to is one whose behaviour on a
project depends on curiosity.

**Two more things were being improvised on every job, and both cost a
detour.** Git refuses to commit without an identity, so the agent discovered
the refusal, worked out which account its token belongs to, and configured one
— correctly, twice out of twice, and by inference each time. And the platform's
tool clones without leaving a credential helper behind (measured: the clone
writes the remote and nothing else), so pushing needed a second discovery. A
step this project runs can do all three deterministically before any model
turn is spent.

**And the case 0016 kept open never arrived.** A project cannot be created
without a repository, the foreman's tool always names its project's, and every
job so far has cloned. The revisit trigger 0016 wrote for itself — jobs without
a repository would have to be forbidden again, and by then some will exist —
did not fire.

## Decision

**Before a job's agent is run for the first time, this project checks the
repository out into the workspace, inside the job's own container, with the
platform's own tool and the credential the job already holds; configures git
to push with that credential; and sets the commit identity to the account the
credential belongs to. The kickoff then says the repository is there.**

Five things follow, each of them the decision rather than a detail of it.

**Inside the boundary, with the job's credential.** The clone is one command
run in the container that already holds the credential in its environment —
the same command the agent would have typed. Nothing is mounted, nothing is
shared, and the host's filesystem is no more part of the boundary than before.
What changes is who runs the command and when.

**Before the first session, not during it.** The checkout exists when the
session is created, so the agent starts the way it is built to start: memory
files loaded, hooks registered, skills discovered, permission defaults in
force, from the first turn.

**Failure is loud and cheap.** A token that does not open the repository, or a
URL that names nothing, fails the job before a model turn is spent, with the
tool's own message rather than an agent's paraphrase of it.

**Identity from the credential.** The commit author is the account the
credential belongs to, spelled the way the platform spells a private address,
so a machine user's token gives a machine user's commits — which is what lets
a repository's rules tell a job's work from a person's.

**Once, at creation.** The checkout is made when the container is, never on a
resume: a resumed job is the same job continuing, per `docs/conventions.md`
§2, and its workspace is part of what it is.

Rejected: **a sentence in the kickoff telling the agent to read the
repository's instructions first.** Cheapest, and it fixes the memory file only:
hooks, skills and permission defaults are loaded by the adapter at session
creation, and no instruction to the model changes what the adapter already
did.

Rejected: **a first turn that clones and stops, then a second that works.**
Keeps 0016 intact and spends a model turn on what one exec does, adds a state
to the job between the two, and trusts the agent to stop after cloning — which
is a thing agents are not reliably good at. It also leaves the identity and the
credential helper to be improvised as before.

Rejected: **a mount or a volume**, for 0016's reasons, which stand.

Rejected: **cloning into a subdirectory** rather than the workspace itself. The
session's working directory is where the adapter looks for a project's
settings; a checkout one level down is found by the agent and not by the
adapter, which is the situation this record exists to end.

## Consequences

**A job has a repository, always.** The vocabulary changes back, and
`docs/conventions.md` §2 is updated in the same commit: a workspace is the
container a job runs in, with the project's repository checked out in it
before the agent's first turn. What was given up is exactly the case 0016
preserved and nobody used.

**The handout carries the repository.** Deciding what a job is handed stays the
pure function in the domain crate, and the repository joins it there for a job
and is absent for a foreman by construction — a foreman has no workspace and
its image has no repository tool, per
`docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`. Delivering it is the
adapter's, which is the seam credentials already use. With one platform, having
its credential is what decides which tool makes the clone; when a second lands,
the repository has to say which host it is on, and that is a change to this
record's delivery rather than to its decision.

**The kickoff changes, and the snapshot tests with it.** "Nothing has been
checked out for you" becomes a statement that the repository is in the current
directory and git is signed in. Per `docs/conventions.md` §4 the diff is the
review.

**A daemon killed mid-clone leaves a container with no session and a partial
checkout.** Startup finds a job that cannot be resumed and fails it, which is
what already happened to a job killed between its container being created and
its agent speaking — and `docs/conventions.md` §2 has no retry, so the answer
is a new job. Nothing is removed automatically, per the retention question
still open in `docs/open-questions.md`.

**Every job pays a clone before its first turn, on this project's account
rather than the agent's.** The cost is the one 0016 already accepted; what
moves is who waits for it — and a job now waits for it exactly once rather
than however many turns it took the agent to notice.

**The end-to-end proof changes meaning.** The test that copies a workspace out
of a finished container and looks for a checkout used to prove the agent
cloned. It now proves the checkout was there for the agent, and the assertion
moves to the workspace root, which is where the adapter looks.

**Reversing** is deleting one step and one paragraph of prompt, and putting
0016's kickoff sentence back. Nothing is persisted, so nothing migrates. What
it would cost is every consequence above, back again, on the first job.

**Revisit if** a job genuinely needs no repository, which is the case 0016
guarded and this record judges absent; if a repository becomes too large to
clone per job, which is 0016's own revisit trigger and is answered here by a
shallower clone rather than by a different design; or if a platform worth
supporting has no command-line tool, in which case the checkout is plain git
with a credential helper this project configures, and the identity comes from
somewhere else.
