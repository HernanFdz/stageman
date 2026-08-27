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

- **Which logging does this use, and where does its output go?** The store has
  the first thing that needs one: a snapshot write that fails inside a `Drop`
  can only be reported, never returned, and it currently goes to standard error
  as a placeholder rather than a choice. `tracing` with `tracing-subscriber` is
  the leading candidate — structured, and its span model suits a system whose
  work is naturally nested inside a job. The harder half is where output goes:
  `docs/decisions/0005-conversation-happens-on-channels.md` has the dashboard
  showing logs, so writing to a terminal nobody is attached to is not enough,
  and a failure an operator never sees is close to a failure that was
  swallowed. Settled by the first thing that needs to *read* a log rather than
  write one, which is the dashboard.

- **Does a container die with the process that started it, on every runtime
  this has to work on?** `docs/conventions.md` §4 makes "killing stageman
  leaves nothing behind" a bar with a test rather than an aspiration, and a
  container is now the thing that would be left. Measured on Docker Desktop on
  macOS, with a container's standard input held cleanly open: hard-killing the
  attached client ends the container either way, and `--rm` decides what
  remains of it. With that flag, nothing — not even a stopped container.
  Without it, the container is left exited, with its filesystem intact. The
  handshake passes it because a throwaway probe has no state worth keeping;
  anything that does will want the other behaviour. That is one runtime on one
  platform and the mechanism was not identified, so it is evidence and not a
  guarantee; the same probe on a Linux engine and on a rootless runtime is what
  would settle it. Settled by running it where CI runs, since that is the
  second platform this has to hold on anyway.

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

- Next, a workspace and a platform credential arriving at container start,
  which is the first time
  `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` is
  exercised rather than described. The handshake reaches a container with
  neither, so this is the step that makes the arguments vary rather than being
  one fixed list.
- Then the app grows a binary, so that the first-run prompt in
  `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`, and the
  key read from the environment, have somewhere to live.
- Then one job end to end, as the proof of concept: a project configured, an
  agent started in its own container, and a change proposed. Deliberately
  before the two mitigations above rather than after — they are far easier to
  design against something that runs than against something imagined.
- Then Slack end to end — a signal read, judged and turned into a job whose
  prompt is snapshot-tested, and a question asked and answered without anyone
  touching a terminal. Slack because it is the escalation path
  rather than the richest source of work: until a job can ask something,
  nothing can safely run unattended, so every other channel is blocked behind
  this one. See `docs/decisions/0005-conversation-happens-on-channels.md`.
