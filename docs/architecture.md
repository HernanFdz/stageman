# Architecture

The shape of this project and why it has that shape. Read this before changing
code. `docs/vision.md` is for deciding what to build; this is for deciding where
it goes.

**Backticked paths and identifiers here are verified.** `just drift` resolves
every backticked path against the repository and every backticked Rust
identifier against the source, so a citation that stops being true fails in the
commit that broke it rather than a year later. `docs/conventions.md` above is
one such citation, and it is there so the check has something to resolve from
the first commit — a check reporting a denominator of zero is honest and proves
nothing. `drift-doc-symbols` stays at zero until you name your first identifier;
that is not a pass, it is an absence of evidence.

**Never restate what the repository already knows.** No directory trees, no
dependency lists, no build order, no status. All of it is derivable, all of it
goes stale, and `just drift` cannot save you from a sentence that was true when
it was written.

**Each section says what belongs in it and starts empty.** That prose is a
standing rule, not a placeholder: it stays when the section fills up, because it
governs what gets added next. Only the HTML-comment example and the
`_(none yet)_` marker are disposable.

## 1. The pieces

What each crate or module is *for* — the part `cargo metadata` cannot tell you,
because it is intent rather than structure. One line each is usually enough; the
reasoning goes in §3.

Say which way the dependencies point, and which direction is forbidden. That is
the single most useful sentence in this document, and the one an agent breaks
first.

- **core** — the domain. What a project is, what a job is, the states a job
  moves through, and the vocabulary in `docs/conventions.md` §2 expressed as
  types. No I/O, no async runtime, no platform, no framework.
- **orchestrator** — the deciding. Watches the channels, judges what is worth
  acting on, and creates jobs. It holds every credential, and it is the only
  place a kickoff prompt is composed.
- **job** — the doing. Provisions one isolated workspace, runs one agent process
  inside it, supervises that process to completion, and hosts the tools through
  which that agent reaches the outside world.
- **app** — the Dioxus fullstack binary. Serves the dashboard and runs the
  orchestrator in the same process; running this is what running stageman means.
  It operates the instance — projects, credentials, logs, stopping a job — and
  never talks to one. Conversation with a running job belongs to a channel, so
  no conversational state lives here; see
  `docs/decisions/0005-conversation-happens-on-channels.md`.

Dependencies point inward. **orchestrator** and **job** may both name **core**,
and **core** may name neither. **orchestrator** and **job** may never name each
other — everything they share is a type in **core**, which is what keeps the
deciding and the doing from growing into one another. **app** may name all
three; nothing may name **app**.

The directories are named for the concepts above and the packages are not —
they carry a prefix, and the app is published as the project's own name. That
mismatch is deliberate and one half of it is load-bearing; the reason is in
`docs/conventions.md` §3.

The asymmetry worth noticing: instructions only ever flow one way. The
orchestrator composes the prompt a job starts from, and a job never writes its
own. Every place in the system where an instruction is *authored* is therefore
in one crate, which is what makes §4 of `docs/conventions.md` enforceable at
all.

## 2. Invariants

What must be true at all times, stated so that a reader can tell whether a
change breaks one. These are the properties the type system, the tests, or a
reviewer are defending — write down which, because "invariant" enforced by
nobody is a wish.

- **A job never holds a platform credential.** Credentials live in the
  orchestrator. A job reaches GitHub, Slack or anything else only by calling a
  tool the orchestrator hosts, so text that arrives in a repository cannot carry
  a token back out. *Defended by* the crate boundary — **job** has no path to
  the credential store — and by a reviewer, who has to reject any change that
  hands one across.
- **One job, one workspace, one project.** No two jobs ever share a working
  tree, and a job provisioned for one project cannot reach another project's
  repository, credentials or channels. *Defended by* construction, since a
  workspace is minted per job and scoped to a project, and by the escape test in
  `docs/conventions.md` §4 — construction alone stops holding the moment the
  isolation mechanism changes, and that mechanism is still an open question.
- **No job blocks on a terminal.** A job that needs a human emits the question
  on a channel and stays alive. It never writes to standard output expecting an
  answer on standard input, because nobody is watching that terminal — that is
  the whole point. *Defended by* the agent process being driven
  programmatically rather than interactively, and by a reviewer.

**Deliberately not an invariant: that every job traces to a recorded signal.**
Each job carries a reason (`docs/conventions.md` §2), but nothing enforces it
and no type prevents a job existing without one. Recorded here so the absence
reads as a decision rather than an oversight. Revisit if the reason stops being
enough to answer "why did this run?" — most likely once one signal starts
producing several jobs, or several signals one.

## 3. Why this shape

The forces that produced the structure above, and the shapes that were rejected.
Without this, the next person reads §1 as arbitrary and reorganises it.

For a choice big enough to have consequences, write a record in
`docs/decisions/` and cite it there rather than repeating the argument — a
decision belongs in one place, with its rejected alternative and what would make
it wrong.

The split into deciding and doing is the one real seam in the system, and the
crates exist to make the code map match it — see
`docs/decisions/0003-four-crates-around-a-core.md`, which also records why a
trait-per-boundary design was rejected on a green field.

Everything else follows from two decisions taken before any code existed. That
the agent inside a job is somebody else's product, not ours, is what makes the
**job** crate a supervisor and a tool host rather than an agent loop — see
`docs/decisions/0001-drive-an-existing-coding-agent.md`. That work terminates at
a proposal is what lets a job run unattended at all, and is the reason no part
of this system needs a credential that can merge — see
`docs/decisions/0002-never-merge-never-deploy.md`.

The orchestrator runs inside **app** rather than beside it because the
constraint in `docs/vision.md` §3 is that one operator runs this on their own
machine: a second daemon is a second thing to install, supervise and restart,
and buys nothing while both halves live and die together anyway. The cost is
that the dashboard and the deciding share a process and a fate, which is why
"killing stageman leaves nothing behind" is a quality bar in
`docs/conventions.md` §4 rather than an aspiration.

State is one file for the same reason — see
`docs/decisions/0004-one-encrypted-sqlite-file.md`.
