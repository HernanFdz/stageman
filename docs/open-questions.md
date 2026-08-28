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

- **May a project carry an image of its own?**
  `docs/decisions/0019-a-projects-tooling-is-the-projects-business.md` settled
  who *decides* what a job needs — the project, and never stageman — and left
  who *provides* it open. Declaring a prerequisite does not put a toolchain in
  a container.

  Two shapes remain. An image per project, built from an agent's as a base,
  which is correct and multiplies what somebody has to build and keep current.
  Or a project naming packages installed when its container starts, which
  `docs/decisions/0012-agents-run-in-containers.md` rejected for the agent
  because installing on every start puts minutes in front of every signal — an
  argument that is genuinely weaker here, since a job is not on a signal's
  critical path and starts far less often than triage does.

  What makes this worth answering rather than living with: an agent that cannot
  run a project's checks still opens a pull request. The proposal is honest
  about it — the first one this system made said so unprompted — but honesty
  hands the verification back to a person, which is the work this system exists
  to remove. Settled by the second project this ever runs against, because one
  project's answer is indistinguishable from a special case.

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

- Next, the dashboard — and its first piece proves the shape rather than draws
  a screen. Dioxus is named in `docs/architecture.md` §1 and has never been
  compiled here. It arrives with a server-function model and a client that
  compiles to WebAssembly, and both shape everything built against them, so the
  smallest thing that serves a page and reads real state through a server
  function is worth having before any screen is designed against a guess.

  It also has to settle a gate question in the same breath. `cargo` builds the
  host side only, so a client that does not compile for its own target would
  pass `just check` untouched, and the gate would silently stop covering half
  the application. Deciding that after two screens exist is deciding it too
  late.

- Then move the end-to-end tests out of the crates they test. A test that drives
  a whole flow — a job from kickoff to a cloned repository, a session surviving
  its container stopping — belongs in `tests/`, where it is a separate crate
  that may use only the public API. That is what an end-to-end test should
  exercise, and it is a check nothing else performs: the missing re-exports that
  once made `Greeting` and `Answer` unreadable without taking the protocol
  library as a direct dependency would have shown up immediately as a test that
  would not compile.

  The split is not clean and the rule is what decides it, rather than the word
  *end-to-end*. A test reaching for a private helper — the container-argument
  builders, the label constant, the name parser — is a unit test by definition
  and stays where it is. Roughly half of the ignored tests are in each group,
  so this is a move for some and not a reorganisation of all.

- Then the agents and projects views, which is where configuring an instance
  finally becomes possible at all. Agents first, because
  `docs/decisions/0021-an-instance-starts-empty.md` has a project naming one
  agent for its orchestrator and a non-empty set for its jobs: creating a
  project is impossible until at least one exists, and an agent a project names
  cannot be removed. `State::used_by` is the query that answers both, and it
  names the projects that would break rather than merely refusing — which is
  what lets a dashboard say *why*.

- Then Slack end to end — a signal read, judged and turned into a job whose
  prompt is snapshot-tested, and a question asked and answered without anyone
  touching a terminal. Slack because it is the escalation path
  rather than the richest source of work: until a job can ask something,
  nothing can safely run unattended, so every other channel is blocked behind
  this one. See `docs/decisions/0005-conversation-happens-on-channels.md`.
