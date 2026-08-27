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

- **What is a workspace, mechanically — a container or a git worktree?**
  `docs/architecture.md` §2 requires that a job cannot reach outside its own
  workspace or into another project. A worktree is nearly free and starts
  instantly, but it shares a kernel, a filesystem and a network with everything
  else on the machine, so the invariant holds only as far as the agent process
  chooses to respect it. A container actually enforces it and costs an image, a
  daemon and a slower start. Settled by writing the escape test from
  `docs/conventions.md` §4 first and seeing which mechanism passes it — the test
  is required either way, so it costs nothing to write it before choosing.
  One argument that used to sit on this scale has been removed: a container
  could not reach an ambient desktop login, but a headless agent credential
  reaches a container as easily as anything else, so authentication no longer
  favours either side — see
  `docs/decisions/0008-one-credential-per-agent.md`. Decide it on isolation
  alone.

- **How is an agent driven — one shared protocol, or an adapter per tool?** The
  Agent Client Protocol is an open standard for precisely this relationship:
  JSON-RPC over a subprocess's standard input and output, with sessions,
  streamed turns, and the agent raising permission requests back to whoever
  launched it. The alternative is an adapter per tool over each agent's own
  headless interface. While only one agent mattered the adapters looked
  cheaper, since a single tool's own interface already offers duplex streaming
  and schema-constrained output.
  `docs/decisions/0006-agents-are-pluggable.md` changes that arithmetic:
  bespoke work per agent, against one protocol. Note what that protocol
  actually costs, because it is easy to overstate — agents do not all speak it
  natively, and at least one is reached through a separate adapter binary
  maintained by a third party and distributed through a package manager this
  project otherwise has no use for. Depending on such an adapter has been
  accepted, which makes the question less about reach and more about shape:
  whether the protocol is the right form for the contract, with each agent
  reaching it natively, through somebody else's adapter, or through one of
  ours. What still has to be checked — and is the narrow thing the spike
  exists for — is whether a permission request surfaces as an event the parent
  can *answer*. Routing it to a human on a channel is the entire requirement,
  and a mode that simply stops asking is not a substitute for one that asks
  somewhere else.

- **Is a third-party orchestrator permitted to run an agent on a personal
  subscription?** Vendors bill agent tools two ways — a subscription meant for
  a person working interactively, and per-token keys meant for automation — and
  the boundary between those is being actively rewritten. At least one vendor
  announced a change to how non-interactive usage is metered and then paused
  it, and secondary sources disagree with that vendor's own documentation about
  what third-party tools may do. None of this can be settled by reasoning about
  it. What makes it survivable rather than fatal is that
  `docs/decisions/0008-one-credential-per-agent.md` treats the credential as
  configuration, so an answer either way is a setting rather than a redesign.
  Settled by reading the terms that actually apply to each configured agent,
  and re-reading them whenever a vendor moves them.

- **What should happen when an agent credential expires while nobody is
  watching?** It will, and it lands on every job at once. The options run from
  failing each job loudly and showing it on the dashboard, to pausing the
  instance and saying so on the channel a human is actually reading. The second
  is more work and is probably right, because a dashboard nobody is looking at
  is exactly where this failure would otherwise sit until morning. Settled by
  deciding what a job that cannot start should look like in general — the same
  question wearing a different hat, and worth answering once.

- **Should a job's environment have an egress allowlist?** Since
  `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`, a job holds
  credentials an agent could be talked into sending somewhere. Restricting
  outbound traffic to the platforms the project actually uses means a persuaded
  agent has nowhere to send them, which is the strongest available mitigation
  and costs nothing at runtime. Deliberately deferred until something works end
  to end, because the shape of the rule is easier to get right against a system
  that runs than one imagined. Settled by choosing the isolation mechanism
  above — the answer is close to free in one of the two options and close to
  impossible in the other.

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

## Next

Intended next steps, in order, each with its reason. Written as intentions, not
progress: "next X, because Y" — never "X is 60% done", which is both derivable
and wrong within a day.

- Next, settle how an agent is driven: a throwaway spike against both
  candidates, testing only the two things `docs/architecture.md` §2 makes
  non-negotiable — and testing them against more than one agent, because a
  contract validated against a single tool is precisely the costume
  `docs/decisions/0006-agents-are-pluggable.md` warns about. It gates
  everything below it, which is why it comes first: the schema cannot settle
  what a job stores until a job's conversation has a shape, and Slack end to
  end *is* that conversation.
- Then the SQLite schema and the encrypted credential handling, because every
  other piece needs somewhere to put a project, and because the redaction bar in
  `docs/conventions.md` §4 is cheap to build in and awkward to add afterwards.
- Then Slack end to end — a signal read, judged and turned into a job whose
  prompt is snapshot-tested, and a blocking question asked and answered without
  anyone touching a terminal. Slack first because it is the escalation path
  rather than the richest source of work: until a job can ask something,
  nothing can safely run unattended, so every other channel is blocked behind
  this one. See `docs/decisions/0005-conversation-happens-on-channels.md`.
