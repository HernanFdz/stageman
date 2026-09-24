# 0055 — A job says why it stopped

## Status
Accepted. Completes
`docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md`, which
defined five readings of an idle job and left four of them unreachable, and
spends the mechanism
`docs/decisions/0034-tools-are-served-not-shipped.md` built. Since
`docs/decisions/0069-a-message-reaches-a-working-job.md` a claim made
in a turn is cleared when a message is steered into it, and the job is told
to claim again before it stops. Since
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` the same
tool takes the pull requests the job opened, by number, which the job keeps
as the union of everything ever claimed, whichever way the turn ends and
however many messages were steered into it — a pull request is a fact about
the platform, not about the turn — and which the notice below carries.

## Context

0052 split *idle* into readings a person acts on differently: asked, proposed,
paused, failed, silent. Two of those are written by things outside the job — a
person pauses one, and a turn that goes wrong is observed to have failed. The
other two are claims only the agent is in a position to make, and nothing could
make them. So every job that finished cleanly landed in *silent*, which is
honest and useless.

The evidence for which reading applies exists and is not readable. An agent
that stopped because it asked a question and one that stopped because it opened
a pull request end their turns identically: the protocol reports *end turn* for
both. 0052's own rejected alternative covers reading it out of the transcript,
and the objection stands — the failure would be confident and wrong.

## Decision

**A job is given a tool that says why it is stopping, and its instructions tell
it to call the tool immediately before it stops.**

- **Two spellings and no more**: *ready for review* and *waiting for an answer*.
  A job cannot claim to have failed, because failure is observed rather than
  claimed, and cannot claim to be paused, because that is a person's doing.

- **Calling it changes no state.** It leaves a note against the turn, and the
  instance reads that note when the turn actually ends. A job stays *working*
  until its agent stops, which the reply gate leans on to stop two replies
  resuming one container.

- **The note belongs to the turn, not the job.** It is registered when a turn
  begins and consumed when it ends, so a claim can never be recorded against
  the turn after the one it was made during — and a claim arriving after its
  own turn was stopped is refused, because there is nothing left to describe.

- **A claim is ignored when the turn did not end cleanly.** An agent that said
  it was ready for review and then ran out of tokens did not finish, whatever
  it believed a moment earlier.

- **Silent stays.** A turn that ends without a claim is recorded as one, and
  the dashboard word for it is the one it always used. Nothing forces an agent
  to call anything, so the residual is not a defect to design out.

Rejected: **inferring the reading from what the agent wrote.** 0052 rejected it
and nothing has changed: it reads a claim out of prose not written to carry one.

Rejected: **making the claim a state change**, so that calling the tool moves
the job immediately. It would let an agent put its own job somewhere a reply
could reach while its turn was still running, which is the collision the reply
gate exists to prevent.

Rejected: **offering the tool to a foreman as well.** A foreman has no state
between turns for a claim to be recorded against, so a foreman calling it would
be describing something nothing reads.

Rejected: **defaulting a missing claim to *proposed***, on the grounds that a
turn that ended cleanly probably finished. It records something no agent said,
and it hides the case worth seeing — an instruction that is not being followed.

## Consequences

**It works, and it is not certain.** Driven end to end against a real agent: a
job was recorded as *proposed* because its agent said so. Also observed
finishing without calling the tool at all, on the same instruction. So the
readings arrive most of the time and *silent* is a real outcome rather than a
theoretical one, which is the reason it exists.

**An agent told to use a tool it cannot reach can die.** Measured while
building this: with the tools endpoint unreachable, a job that was told to call
this one failed outright about half the time rather than proceeding without
tools. 0034 records that an unreachable endpoint "does not error"; that was
true while nothing insisted on a tool. The daemon always serves the endpoint,
so this is a statement about what now depends on it rather than a new failure
mode in normal operation — but it means the endpoint's reachability has stopped
being merely a feature and become a prerequisite.

**A test that runs a job now has to serve the tools.** The harness that drives a
job end to end used to leave the endpoint unserved, because nothing a job did
needed it. It is the daemon's own port, and an operator's daemon is very likely
holding it, so `just image-session` names a port of its own.

**The kickoff gained a closing paragraph, and it is unconditional.** Saying
something needs a channel; saying why you stopped does not, because it is
recorded on the job rather than posted anywhere. So a job on a project with no
channel bound is told to call it too.

**Reversing** means deleting a tool, a paragraph and a registry. Every claim
already recorded stays on its job and goes on reading correctly, because the
states outlive the thing that wrote them.

**Revisit if** *silent* stops being rare, which would say the instruction is not
being followed and is a prompt problem rather than a state problem; or if an
agent is ever driven through a connection held open across turns, which would
make a claim deliverable as part of ending a turn rather than as a tool call
that happens to come last.
