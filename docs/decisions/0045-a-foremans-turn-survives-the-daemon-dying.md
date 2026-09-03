# 0045 — A foreman's turn survives the daemon dying

## Status

Accepted. The half `docs/decisions/0015-a-job-survives-the-daemon-dying.md` did
not cover, and did not cover for an ordinary reason: a foreman had no inbox
when that record was written, so there was nothing of the deciding half to
resume.

## Context

A project's foreman holds one message and queues the rest, and that structure
is part of the snapshot — it is a person's words, not a credential, and it is
written with everything else on every change. So a message in hand when this
process stops is still in hand when it comes back.

Nothing picked it up. A foreman's loop is driven by exactly one thing: the
arrival that finds it idle. After a restart it is not idle — it is holding the
message it was interrupted on — so no arrival ever drives it again. The project
is wedged permanently, and the shape of the failure is worse than silence:
every later message is accepted, acknowledged on its thread, and told how many
are ahead of it, with that number climbing for ever. It looks like a busy
system. The only repair is to edit the instance file by hand.

It went unnoticed because the two things that would have caught it both look
elsewhere. The sweep at startup reconciles jobs against containers and has no
opinion about inboxes. And the invariant that makes the inbox trustworthy —
that a foreman cannot be idle while something waits — is about the type, which
is as true after a restart as before. Nothing was inconsistent. Nothing was
running either.

Two things make picking the message up genuinely possible rather than merely
desirable. A foreman's session lives in one long-lived container per project,
and since `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
that container is not stopped by a turn ending — so after an ordinary kill it
is still there, and the turn resumes with everything the agent already knew. And
the honest answer to "did the interrupted turn finish?" is that this process
cannot know: whatever it did happened outside itself.

## Decision

**At startup, every project whose foreman is holding a message is put back to
work**, on a task of its own, and the turn that picks that message up is told
it was interrupted.

- **Nothing is taken and nothing is re-acknowledged.** The arrival already
  happened and was already answered on its thread when it did, so repeating
  `received_notice` would tell somebody their message had been received twice.
  What the thread is told instead is `resumed_notice` — that the instance
  restarted and is picking it up — because the person has been waiting since
  before the restart, and a wait nobody explains is indistinguishable from
  having been forgotten.

- **The interrupted turn is told, in the same terms a resumed job is told.**
  `RESUMPTION` already establishes the rule and the reason: an agent that
  assumes its last step failed does it twice, and a foreman's last step may
  have been starting a job or speaking on the channel. So the notice says what
  cannot be assumed in either direction rather than guessing, and it goes
  *first*, before the instructions — an agent that read what to do before
  hearing it was interrupted has already decided.

- **Only the first turn is interrupted.** Whatever was queued behind it was
  never begun, so it is fresh however the loop was entered.

Rejected: **dropping the message in hand and starting from what waited behind
it.** It is the smaller change and it makes the wedge impossible, and it loses
the one message somebody is definitely waiting on — silently, which is the
failure this whole area is being fixed for.

Rejected: **not persisting the inbox at all**, so a restart always begins idle.
Simpler again, honest about what it does, and it throws away every message in
flight rather than one. It also gives up something already paid for: the state
is written on every change, so the inbox surviving costs nothing and only the
resuming was missing.

Rejected: **recording how far a turn got, so that resuming could be precise.**
There is nothing to record. The turn's effects are an agent's, in a container,
against services this process does not observe; anything written here would be
a guess about somebody else's progress, and a confident one. Asking the agent
to check is the only account of what happened that is actually informed.

Rejected: **resetting a held message to idle at startup and leaving the person
to send it again.** It unwedges the instance and puts the cost on somebody who
did nothing wrong, and it needs them to notice — which the acknowledgement they
already received actively discourages.

## Consequences

**An interrupted turn may do part of its work twice.** The notice is what makes
that tolerable rather than what makes it impossible, and it is a real cost: an
agent that checks badly starts a second job. It is the same trade 0015 made
deliberately for jobs, and it is made here knowing the outward-facing thing a
foreman can repeat is larger.

**Its strength depends on the session container surviving**, which after an
ordinary kill it does, and after a host reboot or a pruned runtime it does not.
In that case the session is gone, the opening is sent again, and the agent picks
the message up with no memory of having seen it — with the notice as its only
warning. That is the weakest case and it is not detected separately, because
from in here a missing container and a fresh project look identical.

**A restart now speaks on threads it did not speak on before.** One line per
project that was mid-turn, and only then. A restart with nothing in flight says
nothing, which is nearly every restart.

**The wedge is fixed forward, not repaired backward.** An instance already
holding a stuck message is unwedged the first time it starts a build carrying
this, because that is exactly the state this looks for.

**Reversing** is deleting one call at startup and one branch in the prompt; no
data changes, because this reads state that was already being written.

**Revisit if** a foreman's turn ever gains an effect it cannot check — anything
this system does that leaves no trace an agent could look up. The whole decision
rests on "check rather than assume" being advice somebody can act on, and an
unobservable effect would make it advice that cannot be followed.
