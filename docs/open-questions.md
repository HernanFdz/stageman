# Open questions

A **queue, not a status report**. That distinction is the whole design: a status
report claims to describe current state, so it rots the moment state moves — the
single largest source of doc drift there is. A queue entry is either still open
or it has been removed, and "still open" is verifiable by reading it.

Entries leave one of two ways:

- **Answered** → write it up in `docs/decisions/` and delete the entry here.
- **Done** → it is in git history; delete the entry here.

Nothing derivable belongs in this file. What is built, what is committed, what
is green — `just brief` and `just check` answer those from the repository, and
they cannot go stale. This file is only for what the repository cannot tell you:
a question waiting on a human, or an intention not yet acted on.

## Undecided

Questions blocking or shaping work, each with enough context to answer without
re-deriving it. If you cannot state what would settle it, it is not a question
yet — it is unease, and belongs in your own notes until it sharpens.

- **Should a job's environment have an egress allowlist?** Since
  `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`, a job holds
  credentials an agent could be talked into sending somewhere. Restricting
  outbound traffic to the platforms the project actually uses means a persuaded
  agent has nowhere to send them, which is the strongest available mitigation
  and costs nothing at runtime. Deliberately deferred until something works end
  to end, because the shape of the rule is easier to get right against a system
  that runs than one imagined. What was going to settle it — the choice of
  isolation mechanism — has since been settled the other way round by
  `docs/decisions/0012-agents-run-in-containers.md`, which makes this
  straightforward rather than theoretical. So the remaining question is not
  whether it can be done but which hosts belong on the list, and that is
  answered by watching what a job actually reaches for.

- **Should a job's platform credentials be scoped and short-lived?** The other
  mitigation from 0009, and independent of the first. A credential limited to
  the one repository a job is working on, minted per job and expiring, turns a
  leak from an estate-wide problem into a bounded one. Deferred for the same
  reason. Settled by finding out what the platforms actually support: a token
  narrow enough to be worth minting per job, and an issuing path that does not
  need a human. Note the interaction with
  `docs/decisions/0002-never-merge-never-deploy.md` — whether a scope exists
  that permits opening a pull request but not merging one is the same question
  wearing a different hat, and answering it once answers both.

- **Where does a log line go?**
  `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md` settled how
  one is *emitted* and deliberately settled nothing about where it ends up.
  Standard error is a real answer for a process an operator started and is
  watching, and a placeholder for the daemon this becomes:
  `docs/decisions/0005-conversation-happens-on-channels.md` has the dashboard
  showing logs, and a failure nobody sees is close to one that was swallowed.

  The hard part is that **instance-wide and project-level output are two
  concepts, not one with two sources.** A snapshot that will not write is a
  fact about the installation. What a job's agent says is about one project's
  work. They differ in who reads them, when, and what for — so answering both
  with one destination is the way this gets built wrong, and calling them the
  same has to be a decision somebody takes rather than a default nobody
  noticed. Spans are how they could be routed apart, and no span exists yet
  because nothing has run long enough to need one.

  Settled by the first thing that needs to *read* a log rather than write one,
  which is still the dashboard.

- **Does a stopped container behave the same way on every runtime this has to
  run on?** `docs/decisions/0015-a-job-survives-the-daemon-dying.md` rests on
  one measurement taken on Docker Desktop on macOS: hard-killing the attached
  client leaves the container exited with its filesystem intact, and starting it
  again resumes the session. The mechanism was not identified, so it is evidence
  rather than a guarantee — and the whole resume design now depends on it, which
  makes this the most load-bearing unverified claim in the repository. Settled
  by running the same probe on a Linux engine, which is where CI runs, and on a
  rootless runtime. If it does not generalise, the fallback is not obvious and
  is worth thinking about before the probe rather than after: most likely
  recording enough to recreate the container rather than restart it, which is a
  different design and not a patch to this one.

- **When is a finished job's container removed?** Nothing does it today, so
  `docs/decisions/0015-a-job-survives-the-daemon-dying.md` accumulates one
  writable layer per job — the leak that record's own narrowed bar permits by
  name. Deliberately unanswered until a job exists to retire, because the shape
  of the rule depends on what finishing looks like. The leading candidate is
  that an operator retires a job from the dashboard and that removes its
  container, with automatic retirement later: an agent that judges itself
  finished asks for confirmation on a channel and retires on the answer.

  One part is now settled by default rather than by decision, which is worth
  saying out loud: the startup sweep removes *nothing*. A container it cannot
  place is reported and left, because a container is where a job's work lives
  and "I did not recognise this, so I deleted it" is the wrong answer when the
  instance is the thing that is wrong — a snapshot restored from an older
  backup, most obviously. So nothing is ever removed automatically today, and
  the question below is entirely about when that should change. Note
  what that second half already is — a job asking a question and waiting for a
  human, which is
  `docs/decisions/0005-conversation-happens-on-channels.md` and the last item
  under Next. So this is not a separate feature to design; it falls out of the
  channel work, and the cheap thing to record now is that the two are the same
  problem.

- **How is the orchestrator's long-lived container held open?**
  `docs/decisions/0012-agents-run-in-containers.md` puts the agent the
  orchestrator thinks with in one long-lived container, on the reasoning that
  per-signal containers buy nothing and cost a start every time. What is built
  starts one per question, which is a gap rather than a violation only because
  nothing calls it yet — it becomes a violation the moment the orchestrator
  does. The obstacle is shape, not effort: the protocol library scopes a
  connection to a closure, so a connection outliving one call means a task that
  owns it and a channel to speak through, and that task's failure and shutdown
  become things somebody has to handle. Settled by building it, and worth
  building before the orchestrator has a second caller rather than after.

- **Which tools does a job's image need, and who decides?**
  `docs/decisions/0012-agents-run-in-containers.md` has the image carrying "the
  agent and the platform tools a job needs", installed at build time. The first
  real job this system ran proposed a change to this repository and said so
  itself, in its own pull request: it could not run `just check`, because
  neither `just` nor `cargo` is in the image. Verified afterwards — the image
  has git and gh and no Rust toolchain at all.

  That is not a defect in the image, it is a question 0012 did not ask. The
  tools a job needs are a property of the **project**, and the image is built
  per **agent**. A Rust project needs a toolchain, a Python one needs something
  else, and both need the same agent — so one axis is being asked to carry two.

  Three shapes are available and none is obviously right. An image per project,
  built from an agent base, which is correct and multiplies the images somebody
  has to build. A project declaring packages installed when its container
  starts, which 0012 rejected for the agent on the grounds that installing on
  every start puts minutes in front of every signal — an argument that is
  weaker for a job than it was for triage, since a job is not on a signal's
  critical path. Or leaving it, which is what happens today.

  Leaving it is worse than it sounds, and that is the part worth deciding on.
  An agent that cannot run the tests still opens a pull request, and the whole
  purpose of the gate is that nobody has to trust a change which has not passed
  it. A proposal that says "I could not verify this" is honest and still shifts
  the work back to a human, which is the thing this system exists to avoid.
  Settled by deciding whether a project may carry an image of its own.

- **What should happen when an agent credential expires while nobody is
  watching?** It will, and it lands on every job at once. The options run from
  failing each job loudly and showing it on the dashboard, to pausing the
  instance and saying so on the channel a human is actually reading. The second
  is more work and is probably right, because a dashboard nobody is looking at
  is exactly where this failure would otherwise sit until morning. Settled by
  deciding what a job that cannot start should look like in general — the same
  question wearing a different hat, and worth answering once.

## Next

Intended next steps, in order, each with its reason. Written as intentions, not
progress: "next X, because Y" — never "X is 60% done", which is both derivable
and wrong within a day.

- Next, the half of the proof of concept still missing: a change actually
  proposed. A job now runs end to end — a project watched, a container named
  for it, a kickoff it did not write, and a repository it cloned itself — and
  it stops short of opening a pull request. That step is the first in this
  project that cannot be taken against something invented: it needs a
  repository somebody owns and a credential that can push to it, and the thing
  it produces is visible to other people. Deliberately still before the two
  mitigations above, which are far easier to design against something that runs.
- Then a way to configure a project at all. One is currently built in a test,
  and an operator has no way to add one, because adding projects is the
  dashboard's job and there is no dashboard. Worth stating as its own step
  rather than letting it hide inside the dashboard's: the first-run flow shows
  that asking for configuration in a terminal is cheap, and a project is a
  repository and a credential rather than anything a dashboard is uniquely good
  at.
- Then Slack end to end — a signal read, judged and turned into a job whose
  prompt is snapshot-tested, and a question asked and answered without anyone
  touching a terminal. Slack because it is the escalation path
  rather than the richest source of work: until a job can ask something,
  nothing can safely run unattended, so every other channel is blocked behind
  this one. See `docs/decisions/0005-conversation-happens-on-channels.md`.
