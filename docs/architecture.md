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
  types. No I/O, no async runtime, no platform, no framework. It also owns one
  piece of logic that looks like plumbing and is not: building the environment
  a child agent process is handed. That is a pure function from configuration
  to a set of variables, it is the only thing standing between an operator and
  silently paying the wrong way, and being pure is exactly what lets it be
  tested without spawning anything.
- **agent** — the contract every coding agent is driven through, and the
  adapters that implement it. Two shapes and one contract: a one-shot
  structured query, and a session bound to a workspace. Nothing outside an
  adapter may be specific to one agent.
- **orchestrator** — the deciding. Watches the channels and judges what each
  signal deserves. A job is one possible reaction and not the only one — doing
  nothing, and answering on the channel, are reactions too. That is worth
  stating because "creates jobs" is the shape this crate drifts into the moment
  nobody writes down that its remit is wider. It holds every platform
  credential, and it is the only place a kickoff prompt is composed.
- **job** — the doing. Provisions one isolated workspace, runs one agent inside
  it, supervises it to completion, and hosts the tools through which that agent
  reaches the outside world. Which agent ran it is recorded on the job.
- **app** — the Dioxus fullstack binary. Serves the dashboard and runs the
  orchestrator in the same process; running this is what running stageman means.
  It operates the instance — projects, credentials, logs, stopping a job — and
  never talks to one. Conversation with a running job belongs to a channel, so
  no conversational state lives here; see
  `docs/decisions/0005-conversation-happens-on-channels.md`.

Dependencies point inward. **core** names nothing. **agent** may name **core**.
**orchestrator** and **job** may name **core** and **agent** — both run agents,
for different shapes of work — and may never name each other; everything they
share is a type in **core**, which is what keeps the deciding and the doing from
growing into one another. **app** may name all four; nothing may name **app**.

So the orchestrator does not start a job, despite being the thing that decides
one should exist. It emits a request as a **core** type, and **app** — the only
crate allowed to name both sides — hands that request to **job**. This is worth
a sentence of its own because "the orchestrator creates jobs" is the natural
reading of the bullet above, and acting on that reading is the first thing that
breaks the rule.

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

- **A job never holds a *platform* credential.** Platform means the outside
  world a job could act on: the repository host, the chat workspace, the error
  reporter. Those live in the orchestrator, and a job reaches them only by
  calling a tool the orchestrator hosts, so text arriving in a repository
  cannot carry a token back out. *Defended by* the crate boundary — **job** has
  no path to that credential store — and by a reviewer, who has to reject any
  change that hands one across.

  The **agent** credential is deliberately outside this invariant. A job's
  environment contains one by construction, because an agent that cannot
  authenticate cannot think, and no arrangement exists in which it does not. It
  is a different kind of secret: it buys work on the operator's account, and
  buys nothing at all in their repositories or channels. Its rule is separate
  and narrower — an agent process receives exactly its own agent's credential
  variables and none belonging to any other — and lives in
  `docs/decisions/0008-one-credential-per-agent.md`. Writing the carve-out down
  is the point: an invariant with a silent exception is worse than one with a
  stated boundary, because only the second can be checked.
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

The fifth crate is the newest part of this shape and the least self-evident. It
exists because being agent-agnostic is a commitment rather than a possibility,
and because both halves genuinely run agents — see
`docs/decisions/0006-agents-are-pluggable.md`, which also records why the same
abstraction was refused earlier and what changed. That a model is reached by
running an agent at all, rather than by calling a vendor's service, is
`docs/decisions/0007-model-work-goes-through-an-agent-cli.md`, and it is a
billing decision at least as much as a technical one.
