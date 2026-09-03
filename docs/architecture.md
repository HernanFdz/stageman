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
  piece of logic that looks like plumbing and is not: deciding what a child
  agent process is handed. That is a pure function from configuration to a
  description of exactly what that process should see and nothing more.
  *Delivering* it is an adapter's job and differs per agent — a variable for
  one, a file at an expected path for another — but the deciding stays here and
  stays pure, because it is the only thing standing between an operator and
  silently paying the wrong way, and that is worth being able to test without
  spawning anything.
- **agent** — the contract every coding agent is driven through, and the
  adapters that implement it. Two shapes and one contract: a one-shot
  structured query, and a session bound to a workspace. Nothing outside an
  adapter may be specific to one agent.
- **foreman** — the deciding, for one project. Watches that project's
  channels and judges what each signal deserves. One per project rather than
  one per instance, because watching needs the project's own credentials and a
  shared foreman would hold every project's at once — see
  `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`. A job is one possible reaction and not the only one — doing
  nothing, and answering on the channel, are reactions too. That is worth
  stating because "creates jobs" is the shape this crate drifts into the moment
  nobody writes down that its remit is wider. It holds what it needs in order to
  *watch* a project's channels, and it is the only place a kickoff prompt is
  composed.
- **job** — the doing. Provisions one isolated workspace, runs one agent inside
  it, supervises it to completion, and gives it the credentials its project
  needs. Supervision spans more than one lifetime of this process: a job
  interrupted by the daemon being killed is resumed at startup rather than
  abandoned, per
  `docs/decisions/0015-a-job-survives-the-daemon-dying.md`. It provisions the
  workspace with the project's repository already checked out in it — before
  the agent's first turn, inside the same container, with the platform's own
  tool and the job's own credential, per
  `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`,
  which reversed that half of
  `docs/decisions/0016-the-agent-clones-the-repository.md`. The agent reaches
  platforms through those platforms' own tools rather than through anything
  hosted here — see
  `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`. Which agent
  ran it is recorded on the job.
- **app** — the Dioxus fullstack binary. Serves the dashboard and runs the
  foremen — one per project — in the same process; running this is what
  running stageman means.
  It operates the instance — projects, credentials, logs, stopping a job — and
  never talks to one. Conversation with a running job belongs to a channel, so
  no conversational state lives here; see
  `docs/decisions/0005-conversation-happens-on-channels.md`.

  **This is the one crate compiled for two machines**, and the split runs
  through it rather than around it: the daemon's half may name everything
  below, and the browser's half gets plain serialisable types and nothing else
  — see `docs/decisions/0022-the-browser-never-sees-the-domain.md`. The split
  is drawn by optional dependencies in the manifest rather than by `cfg` in the
  source, because a `cfg` hides code from the compiler and not a dependency
  from cargo.

Dependencies point inward. **core** names nothing. **agent** may name **core**.
**foreman** and **job** may name **core** and **agent** — both run agents,
for different shapes of work — and may never name each other; everything they
share is a type in **core**, which is what keeps the deciding and the doing from
growing into one another. **app** may name all four; nothing may name **app**.

There is one more direction, and it is inside **app** rather than between
crates: **nothing served to a browser may name any of the four.** The rule
looks like the same one and is not — the others are about what a crate is
allowed to know, and this one is about what leaves the machine.

So the foreman does not start a job, despite being the thing that decides
one should exist. It emits a request as a **core** type, and **app** — the only
crate allowed to name both sides — hands that request to **job**. This is worth
a sentence of its own because "the foreman creates jobs" is the natural
reading of the bullet above, and acting on that reading is the first thing that
breaks the rule.

The directories are named for the concepts above and the packages are not —
they carry a prefix, and the app is published as the project's own name. That
mismatch is deliberate and one half of it is load-bearing; the reason is in
`docs/conventions.md` §3.

The asymmetry worth noticing: instructions only ever flow one way. The
foreman composes the prompt a job starts from, and a job never writes its
own. Every place in the system where an instruction is *authored* is therefore
in one crate, which is what makes §4 of `docs/conventions.md` enforceable at
all.

## 2. Invariants

What must be true at all times, stated so that a reader can tell whether a
change breaks one. These are the properties the type system, the tests, or a
reviewer are defending — write down which, because "invariant" enforced by
nobody is a wish.

- **A job holds credentials for its own project, and for no other.** It gets
  what its project needs to reach the platforms that project uses, what it
  needs to speak on that project's channels, whatever its operator gave that
  project to reach everything else, and the credential material of the one
  agent running it — and nothing belonging to any other project, and nothing
  belonging to any other agent. *Defended by* construction, since everything a
  job is handed is selected from the project it belongs to and built by the
  pure function in **core**, and by the escape test in
  `docs/conventions.md` §4.

  Platforms, channels and variables are three selections rather than one, per
  `docs/decisions/0027-a-channel-is-not-a-platform.md` and
  `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`, so this
  invariant has three places to be broken rather than one — which is what the
  escape test checks separately for each. The third is the one whose contents
  this project cannot describe: a variable is carried and never read, so the
  test can only assert that the selection happened, which is the whole of what
  is defensible about it.

  This is the narrowed survivor of a stronger claim. The original invariant was
  that a job held no platform credential at all, which made exfiltration
  structurally impossible rather than merely bounded.
  `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` gave that up
  deliberately, and records what it bought and what it now costs. Read it before
  treating this invariant as the defence, because it is not one: it bounds the
  blast radius of a leak and does nothing to prevent one.
- **One job, one workspace, one project.** No two jobs ever share a container,
  and a job provisioned for one project cannot reach another project's
  repository, credentials or channels. *Defended by* construction, since a
  workspace is minted per job and scoped to a project, and by the escape test in
  `docs/conventions.md` §4. That mechanism is a container — see
  `docs/decisions/0012-agents-run-in-containers.md` — which enforces the
  boundary rather than relying on the agent to respect it. The escape test
  still earns its place: construction is an argument about what the code does,
  and the test is evidence about what the mechanism actually permits.

  This once said *no two jobs share a working tree*, which named the files
  instead of the boundary. Under
  `docs/decisions/0016-the-agent-clones-the-repository.md` a job could have no
  working tree at all and the invariant was untouched — which is the sense in
  which naming the container was always the more accurate claim, and the
  clearest sign that the older wording described an implementation rather than a
  property.
  `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`
  gives every job a working tree again, and the invariant is still about the
  container: the checkout is made inside it, from nothing on the host.
- **Nothing served to a browser can carry a credential.** What the dashboard
  reads is a small set of plain types holding counts, names and paths, built on
  the server from the domain and never the domain itself. *Defended by* those
  types having nowhere to put one, by the manifest keeping **core** out of the
  browser's build entirely, and by a test that configures a credential and
  reads both the rendered page and the route looking for it. See
  `docs/decisions/0022-the-browser-never-sees-the-domain.md`.

  The test passes by construction today, which is exactly why it is worth
  having: construction is what a later field would change, and a redacting
  `Debug` would not notice — `docs/conventions.md` §4's rule is about
  formatting, and this is about the wire.
- **A channel's connection is never unattended.** The task that reads a
  channel reads it, acknowledges, and hands over; it never waits for what it
  handed over to finish. A turn takes minutes and a socket is answered in
  milliseconds, so a task doing both stops answering the platform's pings,
  cannot see its warning that a connection is about to close, and loses
  whatever is said between that close and a replacement — with nothing said
  anywhere, because an ordinary ending is what it looks like from inside. See
  `docs/decisions/0044-a-listener-only-listens.md`. *Defended by* the handing
  over being a synchronous function, so that everything it starts must be
  started on a task of its own, and by a reviewer — an `await` added there
  would compile and would pass every test, since nothing under test is slow
  enough to matter.

  One thing deliberately stays on that task: the state change each recipient
  makes as a message *arrives*. Order is the whole of what a foreman's inbox
  promises, and spawning that would fill it in the order tasks were polled
  rather than the order messages were read.
- **No job blocks on a terminal.** A job that needs a human emits the question
  on a channel and stays alive. It never writes to standard output expecting an
  answer on standard input, because nobody is watching that terminal — that is
  the whole point. *Defended by* the agent process being driven
  programmatically rather than interactively, and by a reviewer.

- **A job runs on the kit it was created with, on every turn.** Not only the
  first: a loaded session comes back with every setting at the agent's
  default, so a job put back to work after this process died would otherwise
  run on something other than its record says, with nothing in any reply
  saying so. *Defended by* the type — a job's kit is given to its constructor
  and has no setter — and by the adapter settling every option before the
  first prompt of every turn, failing the turn if the agent refuses one or
  reports it unchanged. See `docs/decisions/0048-a-job-runs-on-a-kit.md`.

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

A project's foreman runs inside **app** rather than beside it because the
constraint in `docs/vision.md` §3 is that one operator runs this on their own
machine: a second daemon is a second thing to install, supervise and restart,
and buys nothing while both halves live and die together anyway. The cost is
that the dashboard and the deciding share a process and a fate, which is why
"killing stageman leaves nothing running and nothing untracked" is a quality bar
in `docs/conventions.md` §4 rather than an aspiration.

State is one file for the same reason — see
`docs/decisions/0011-state-is-a-snapshot-not-a-database.md`, which supersedes an
earlier choice of an embedded database and is explicit about the one thing that
swap makes expensive to undo.

The fifth crate is the newest part of this shape and the least self-evident. It
exists because being agent-agnostic is a commitment rather than a possibility,
and because both halves genuinely run agents — see
`docs/decisions/0006-agents-are-pluggable.md`, which also records why the same
abstraction was refused earlier and what changed. That a model is reached by
running an agent at all, rather than by calling a vendor's service, is
`docs/decisions/0007-model-work-goes-through-an-agent-cli.md`, and it is a
billing decision at least as much as a technical one.

What shape that contract takes was settled by a spike rather than by argument.
`docs/decisions/0010-acp-is-the-agent-contract.md` records the choice and, more
usefully, the evidence — most of which stays true whichever way the choice had
gone.

The reason the **job** crate hosts nothing, despite an earlier design in which
the foreman hosted the tools an agent used to reach the world, is
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` — the most
consequential reversal taken so far, and the one most worth reading before
changing anything here.
