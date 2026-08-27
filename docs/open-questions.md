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

- **How does the orchestrator talk to the agent inside a job?** Two candidates.
  The Agent Client Protocol is an open, editor-oriented standard: JSON-RPC over
  a subprocess's standard input and output, with sessions, streamed turns, and —
  the part that matters here — the agent raising permission requests back to
  whoever launched it. It is a wire protocol rather than a library, so a Rust
  process can speak it directly, and the agents worth driving already implement
  it. The alternative is the chosen agent's own headless interface, which is
  narrower but has no second specification between us and it. Settled by
  checking one thing against both: whether a message can be pushed into a
  *running* agent, and whether a blocking question can be answered from
  somewhere other than a terminal. `docs/architecture.md` §2 makes that
  non-negotiable, and an interface that cannot do it disqualifies itself.

## Next

Intended next steps, in order, each with its reason. Written as intentions, not
progress: "next X, because Y" — never "X is 60% done", which is both derivable
and wrong within a day.

- Next, settle how the orchestrator talks to the agent inside a job: a
  throwaway spike against both candidates, testing only the two things
  `docs/architecture.md` §2 makes non-negotiable. It gates everything below it,
  which is why it comes first — the schema cannot settle what a job stores
  until a job's conversation has a shape, and Slack end to end *is* that
  conversation.
- Then the SQLite schema and the encrypted credential handling, because every
  other piece needs somewhere to put a project, and because the redaction bar in
  `docs/conventions.md` §4 is cheap to build in and awkward to add afterwards.
- Then Slack end to end — a signal read, judged and turned into a job whose
  prompt is snapshot-tested, and a blocking question asked and answered without
  anyone touching a terminal. Slack first because it is the escalation path
  rather than the richest source of work: until a job can ask something,
  nothing can safely run unattended, so every other channel is blocked behind
  this one. See `docs/decisions/0005-conversation-happens-on-channels.md`.
