# 0015 — A job survives the daemon dying

## Status
Accepted. Supersedes the "no retry and no resume" clause in
`docs/conventions.md` §2, and narrows the bar in §4 from *nothing behind* to
*nothing running and nothing untracked*.

Its **mechanism** is amended by
`docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`: a
turn is no longer a stopped container being started but an agent run inside one
that is already up, and a container is no longer stopped by the turn inside it
ending. What survives here is the decision — that a job outlives the process
supervising it, and that resuming is not retrying. What does not is the
measurement below, which was taken against a mechanism that no longer exists.

## Context

§2 said a job happens once, with no retry and no resume, and that a second
attempt is a new job. That was right while a job's work stayed inside its own
workspace: nothing outside had changed, so nothing outside had to be reconciled.

`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` ended that. A
job holds its project's credentials and acts on those platforms directly, so by
the time a daemon is killed a job may have opened a pull request, commented on
an issue, or asked a question on a channel that a human has since answered.
Discarding it and starting a new one does not repeat work — it duplicates
outward-facing state and abandons an answer somebody already gave. That is a
correctness problem rather than a cost one, and it was the argument that moved
this: the earlier reasoning weighed only tokens and time, which are the things
that do not matter here.

`docs/vision.md` §3 has this running as a long-lived daemon on somebody's own
machine, so it will be killed mid-job. That is the case to design for, not the
exception.

Four things were measured, on Docker Desktop on macOS, against the adapter
pinned in the image:

**What a hard kill leaves depends on one flag.** With a container's standard
input held cleanly open, hard-killing the attached client ends the container
either way. With `--rm` nothing remains, not even a stopped container. Without
it, the container is left exited with its filesystem intact.

**A stopped container restarts and speaks.** Starting it again runs the entry
point afresh and the protocol handshake completes, so what survives is the
filesystem and never the process.

**A session with content survives the stop.** After a restart, listing sessions
returned the prior one with its identifier, working directory, a generated
title and a timestamp; loading it succeeded. An *empty* session does not
survive, because nothing is written until something is said — which is why an
earlier probe wrongly suggested sessions were lost.

**The resumed agent has its context, and checks rather than assumes.** Asked
after a restart what single word it had replied with before the stop, it
answered correctly. Killed mid-tool-call and restarted, it stated the task it
had been in the middle of and reported the target file's real state — created
and empty — rather than the state its interrupted plan implied. Told only to
report, it reported; left unconstrained, it finished the work.

## Decision

A job's container is not removed when it exits. Killing the daemon leaves
stopped containers, and startup restarts them and resumes their sessions.

Nothing is cleaned up on the way down. Not because a leak is acceptable but
because no code runs on a hard kill: the container stopping is the runtime's
doing, and recovery is startup's job. A clean shutdown and a hard kill
therefore take the same path.

A container is addressed by a name derived from its job's identifier, and
carries a label naming the instance so orphans can be found without the
snapshot. Nothing records a session identifier — listing sessions inside the
restarted container returns it.

On resume the agent is told it was interrupted. The measurement above says it
works this out unaided; the notice is nearly free, and the alternative is
depending on an inference.

The handshake keeps `--rm`. It is a throwaway probe with no state worth
keeping, and that is the one place the flag is right.

## Rejected: starting a new job instead

What §2 said, and correct until 0009. It loses on outward-facing state, above.
Worth keeping in view if a job's effects are ever confined again — that would
restore the original reasoning intact.

## Rejected: recording a container identifier on the job

An identifier is assigned by the runtime, so it can only be recorded *after*
the container exists — and a kill inside that window leaves a container nothing
knows about, which is precisely the untracked leak this record is trying to
prevent. A derived name is known before the container exists, so the window
does not exist.

The objection to names is collision, and the runtime answers it: names are
unique per daemon and enforced against stopped containers too, so a clash is a
loud refusal that names the conflicting container rather than a silent
mix-up. Identifiers are unique across daemons, which is a scope this project
never has, since it talks to exactly one.

## Rejected: resuming only after a clean shutdown

Tempting, because a shutdown we control could let an agent reach a safe point
first, and a mid-turn kill cannot. It loses because the recovery path turns out
to be the same one, and building two means the path that matters — the
unplanned one — is the path exercised least. The measurement that killed this
option is the mid-tool-call test: the agent reconciled against the filesystem
rather than trusting its own plan, which is the behaviour the clean-shutdown
path was supposed to guarantee by construction.

## Consequences

A job can now span more than one lifetime of the daemon. What stays true is
that a job happens *once*: resuming is not retrying, a second attempt is still
a new job with its own container, and nothing records an attempt count.

**Resumption is per-agent behaviour and the contract cannot promise it.** It
depends on the agent persisting its session into the container's filesystem,
which is adapter knowledge in the sense
`docs/decisions/0006-agents-are-pluggable.md` means. The first agent that fails
this needs a different answer for that agent — most likely failing the job
loudly, since a job that silently resumes into an empty context is worse than
one that stops.

**A retained container keeps its credential at rest, where `--rm` used to take
it away.** Variables named at creation belong to the runtime's own record of a
container, so a *stopped* container's configuration still holds the credential
in the clear — verified by inspecting one. Nothing about that is created by
this decision except its duration: the record used to vanish when the container
did, and now lasts as long as the container is kept. It is not the snapshot's
problem, since
`docs/decisions/0011-state-is-a-snapshot-not-a-database.md` seals what it
stores, and it is the runtime's storage rather than this project's — but it is
a place a credential now sits that it did not before, and whoever answers
retention is also answering how long it sits there.

Containers accumulate, one writable layer per job. When a finished job's
container is removed is deliberately unanswered here and tracked in
`docs/open-questions.md`, because it cannot be answered before a job exists to
retire.

§4's bar narrows. *Nothing behind* becomes *nothing running and nothing
untracked*: a stopped container the instance can name is retained on purpose,
and only an untracked one is a leak. The test that bar demands gets harder
rather than easier, since it now has to distinguish the two.

Reversing is cheap in code — restore `--rm` and delete the startup sweep — and
expensive in behaviour, because it reintroduces duplicated pull requests and
abandoned answers as a supported outcome.

Revisit if a platform's own tooling grows a way to make a job's effects
idempotent, so that starting over becomes safe again. That would not make this
wrong, but it would make the simpler design available.
