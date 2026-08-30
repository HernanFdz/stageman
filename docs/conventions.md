# Conventions

**This is the file that differs.** `AGENTS.md` and the justfile are identical
across every project built from this gate, which is what makes them worth
learning once; everything particular to *this* project belongs here instead, so
that the shared half stays comparable and the local half stays findable.

It is the project's brief for whoever works on it next: orientation first, then
the rules nothing enforces mechanically. What the project *does* is in
`README.md` and is not repeated here — one fact in two files is where drift
starts.

Nothing outside this repository will ever rewrite any of it. `quality init`
creates a project and lets go — there is no version file, no sync, and no
update to wait for — so a stale claim here survives until somebody notices it.

**Each section says what belongs in it and starts empty.** That prose is a
standing rule, not a placeholder: it stays when the section fills up, because it
governs what gets added next. Only two things are disposable — the HTML-comment
example, and the `_(none yet)_` marker, which means nobody has written it rather
than that there is nothing to write. Append new sections at the end rather than
renumbering — a `§3` cited in a commit message or a code comment breaks
silently, and no check catches it.

## 1. Where to start reading

The order below is what every project scaffolded from this gate starts with. Add
a document, add its line here, in the same commit — an entry point nobody
maintains sends the next reader to a guess, and a guess is invisible to you: it
produces confident work built on the wrong model, with nothing in the output
saying so.

1. `README.md` — what this does, for whoever is deciding whether to use it.
2. `docs/vision.md` — what it is for and what it refuses to be. Read before
   deciding what to build.
3. `docs/architecture.md` — the pieces, the invariants, and why this shape.
   Read before changing code.
4. `docs/decisions/` — the choices already taken, each with its rejected
   alternative and what would make it wrong.
5. `docs/open-questions.md` — what is still undecided, and the intended next
   step.

Layout lives in `docs/architecture.md` §1 and is deliberately not repeated here.
This file is loaded into every session; that one is read when you are about to
move code. One fact, one home.

## 2. Vocabulary

The words this codebase uses and what each means *here*, including the ones it
deliberately does not use.

The highest-value section in this file and the most often skipped. Someone who
reaches for a plausible synonym writes code that reads correctly and names the
wrong thing — and review does not catch it precisely because it reads correctly.
Record the near-miss too: the term you rejected, and what it would have implied.

- **project** — one repository, together with the channels bound to it, the
  credentials those channels need, and the agents it runs on: one for its
  foreman to think with, and a non-empty set its jobs may use. One
  instance manages several. Everything else in this list belongs to exactly one
  project, always — including the foreman, which is a project's rather
  than an instance's, per
  `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`.
- **foreman** — the one thing per project that reads what a person says and
  decides what to do about it: answer, do nothing, or start a job. It runs an
  agent to think with, in one long-lived container of its own, and it is the
  only thing that composes an instruction — a job never writes its own.

  Called a foreman because the word is a *role* and the job is the work it
  assigns: "why did the foreman do that?" is a question with an answer, in a
  way that the word this replaced never managed. It was **orchestrator** until
  `docs/decisions/0030-the-orchestrator-is-a-foreman.md`, which is why every
  record numbered below that one says the old word and means this. Not a
  *supervisor* or a *coordinator*, which describe watching rather than
  deciding, and not a *dispatcher*, which is only the third of the three
  things it can do.

- **inbox** — the messages waiting for a project's foreman, in the order they
  arrived. It exists only while the foreman is working: a foreman with nothing
  to do has nothing waiting, which is a property of the type rather than a rule
  anybody keeps. Not a *queue*, which names the structure instead of what is in
  it, and would invite a second one somewhere else.

- **turn** — one message, handled from being handed to a foreman or a job until
  its agent stops. The protocol's own word, and the unit everything else is
  scoped to: a turn is what an inbox entry buys, what a thread collects, and
  what "idle" means the absence of.

- **channel** — somewhere the foreman watches and a job can speak into.
  Two-directional by definition, which is why it is not called a *source* or a
  *feed*: the same Slack that carries a question out carries the answer back.
  Not *integration* or *connector* either — both describe plumbing, and the
  interesting part is that somebody is on the other end.
- **signal** — one observation on a channel: an issue opened, an alert fired, a
  message posted. Signals are read and judged, not stored or addressed. They
  are deliberately not entities; see **reason** below.
- **job** — one agent, in one isolated workspace, on one project, from kickoff
  to completion. A job happens once, and there is no retry: a second attempt is
  a new job with its own workspace. It may, however, outlive the process
  supervising it — the daemon being killed stops a job's container rather than
  ending the job, and startup puts it back to work. **Resuming is not
  retrying**, and the distinction is the whole of it: a resumed job is the same
  job continuing, which is why nothing records an attempt count and why the
  outward-facing things it already did are not done twice. See
  `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, which reverses an
  earlier "no resume" and says what changed. Not a *worker*, which implies a
  long-lived process pulling from a pool and gets the relationship backwards —
  the job is the work, not the thing that fetches it. Not a *task* or a *run*
  either, since both imply a durable intent that separate attempts belong to,
  and no such thing exists here. A job records which agent ran it, because once
  more than one can, "why did this go badly?" has no answer without it.
- **workspace** — the isolated place a job's agent works: the container it runs
  in, for as long as that job lasts — see
  `docs/decisions/0012-agents-run-in-containers.md`. One job, one workspace: no
  two ever share one, which is an invariant rather than an aspiration
  (`docs/architecture.md` §2). Not a *checkout*, which names the files and
  misses the boundary that makes them isolated — and the files are no longer
  even part of the definition. This once read "the filesystem the repository is
  checked out into", from a design in which something delivered that
  repository. Nothing does:
  `docs/decisions/0016-the-agent-clones-the-repository.md` has the agent clone
  it if the work needs one, so what is inside a workspace is the job's business
  and a job with no repository at all is an ordinary case rather than an empty
  mount. The foreman's agent still has no workspace, and now for a plainer
  reason than having no repository — a workspace belongs to a job, and triage
  is not one.
- **thread** — where one job's conversation happens on a channel. A channel
  belongs to a project and everything it runs shares it, so a thread is what
  narrows a conversation down to one job — which is also what makes a reply
  routable, since a message arriving names its thread and nothing else. Not a
  *conversation*, which is the thing that happens in one rather than the place
  it happens in, and would leave nothing to call the identifier. The
  foreman has none, deliberately: it speaks at the root of the channel,
  and that is what makes a message there addressed to it rather than to any
  job. See `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.

  Its identifier is opaque to this project and **must stay text**. For Slack it
  is the parent message's timestamp, which looks like a number and is not one:
  parsed as one it loses the microseconds and addresses no message, and the
  failure reads like a permissions problem.

- **reason** — the free text the foreman writes when it creates a job,
  saying why it decided to. Prose meant for a human reading the dashboard, not
  a key pointing at a signal. It is the whole of a job's provenance, which is
  why `docs/architecture.md` §2 records the structured version as deliberately
  absent.
- **kickoff prompt** — the instruction text the foreman composes and the
  job's agent begins from. A job never writes its own.
- **agent** — reserved, always, for a third-party coding agent: the tool that
  gets configured, chosen and run. Never for stageman, never for the
  foreman, never for a job. This one matters more than it looks: "the
  agent decided to…" is ambiguous in exactly the place where being wrong is
  expensive, and the sentence still reads fine either way. Note that an agent
  is not only something inside a job — the foreman runs one too, to think
  with. In the code it is a closed set rather than an open list: the agents that
  can be run are the ones compiled in, because each needs an adapter and an
  image and both are code. What an operator supplies per agent is an
  `AgentConfig` — a credential, and nothing else, since where the program lives
  is decided by an image rather than by this machine. A job stores its agent by
  value, so removing that configuration later cannot rewrite the record of work
  already done.

  One near-miss is imported rather than invented: the protocol library uses the
  same word for the *role* at the far end of a connection. Both types are in
  scope in an adapter, and `ConnectionTo<Agent>` compiles and reads correctly
  against either one, so the protocol's is aliased at the import rather than
  used bare. This is the one place in the codebase where the wrong meaning of
  this word type-checks.

- **handout** — exactly what one agent process is allowed to see: its agent's
  own credential, and — of the one project it works for — that project's
  platform credentials and the *speaking* half of its channel bindings. Half,
  because a binding holds a second credential that opens an event stream, and
  a job has no use for one: the handout carries a narrower type with nowhere
  to put it, so that is a property rather than a rule somebody applies. Nothing else, and nothing
  inherited. The two project halves are not interchangeable and a handout can
  carry one without the other: an foreman's gets the channels and no
  platform credential at all, because watching a channel is its remit and
  acting on a platform is not. See
  `docs/decisions/0027-a-channel-is-not-a-platform.md`. It is *decided* in the domain crate as a
  pure function and *delivered* by an adapter, because which secrets a process
  may see is a question about configuration while what they are called is
  knowledge about one agent. Not *environment*, and that near-miss is the whole
  reason this word exists: an environment names a delivery mechanism, and
  delivery is precisely the half that differs — a variable for one agent, a file
  at an expected path for another — so a word presuming variables would make the
  wrong half sound settled. Not *credentials* either, which is a bag of secrets
  rather than a decision about one process, and loses the part that matters:
  a handout is scoped to somebody, and the scoping is the point.

## 3. House rules

Anything someone would otherwise get wrong: framework versions and their
gotchas, the error type this project uses, which module owns which concern, the
external contracts it has to honour, and the patterns that look reasonable and
are wrong here.

State the rule and the reason. A rule without its reason gets discarded the
first time it is inconvenient — usually correctly, because a rule nobody can
justify is usually obsolete.

- **State lives in memory and is snapshotted to one file on every change.**
  Projects, jobs, reasons, prompts and credentials are one structure, serialised
  with serde and written atomically — temporary file, flush, rename — whenever
  it changes. Not at shutdown: `docs/vision.md` §3 commits to surviving the
  process being killed, and a shutdown-only snapshot survives only a clean exit.
  Credentials inside it stay encrypted under a key from the environment, so the
  file is portable and useless without it. Reasoning and reversal cost are in
  `docs/decisions/0011-state-is-a-snapshot-not-a-database.md`.
- **No secret is ever written to a log line.** Encryption protects the file, not
  the terminal, and a token escapes through a formatted struct long before it
  escapes through the database. The mechanical half of this rule is §4 below.
- **What a child process is handed is constructed, never inherited.** An agent
  process receives exactly the credential material it ought to have — its own
  agent's, and nothing belonging to any other. Delivery differs per agent: a
  variable for one, a file at an expected path for another. What never differs
  is that this project decides, and that nothing arrives by accident. This is
  not tidiness. At least one agent resolves credentials by precedence and
  prefers a per-token key when it finds one, so a variable inherited from
  whatever shell started the daemon silently changes who pays — no error, no
  log line, and no way to notice before the invoice arrives. Deciding what goes
  where is a pure function in the core crate so it can be tested without
  spawning anything; delivering it belongs to the adapter. Reasoning in
  `docs/decisions/0008-one-credential-per-agent.md`.
- **The container runtime's location is configuration, never a PATH lookup.**
  This rule used to be about agents; agents now live in images, so it retargets
  rather than retires — see
  `docs/decisions/0012-agents-run-in-containers.md`. The reasoning is
  unchanged and was measured: of the two agents installed while this was being
  designed, one sat in a directory absent from a non-interactive shell's PATH.
  Anything a daemon locates by searching therefore works perfectly when you
  test it by hand and fails when a service manager starts it, which is the
  worst of the available outcomes. Record the path, and verify it with the
  startup check above.
- **Fail at startup for whatever makes the instance unusable; surface the rest
  in the dashboard.** A missing container runtime and a snapshot that cannot be
  written are the first kind — nothing works without them, and the worst moment
  to discover it is three in the morning on the first signal that mattered.
  Both are built: the snapshot is written once at startup, and the recorded
  runtime is asked for its version, which reaches the daemon rather than merely
  finding the file — a client installed with nothing behind it looks perfectly
  healthy to any check of the filesystem.

  Both of those are conditional on there being something to check. An instance
  starts with nothing configured — no agents, no projects, no runtime — and an
  instance with nothing to run is not unusable, it is empty; see
  `docs/decisions/0021-an-instance-starts-empty.md`. So a runtime is verified
  when one has been configured, and what must not happen is a project created
  against a runtime nothing has checked. The rule is unchanged: the check moves
  to where the requirement begins.

  A credential that has stopped working is the *second* kind, and refusing to
  start over one would be a trap: the dashboard is where credentials get fixed,
  so an instance that will not start puts the repair behind the door it just
  locked. Those fail the job that needs them, visibly, and leave the instance
  running so an operator can do something about it. The distinction is whether
  the operator could act on it — not how serious it looks.
- **The app crate is an Axum server, and the foreman shares its runtime.**
  Dioxus fullstack server functions are Axum handlers, and the foreman runs
  in that same process rather than beside it. So foreman work must never
  happen on the request path: watching a channel, judging a signal and
  supervising a job all belong on their own tasks. A dashboard that stops
  painting because a job is thinking is the failure this rule exists to prevent.
- **The app's `server` feature is a contract with the framework, not a name.**
  The server-function macro emits `#[cfg(feature = "server")]` literally, so a
  feature spelled anything else silently moves every server function's body to
  the client. Everything the daemon needs is an optional dependency behind it —
  including the four internal crates — because a `cfg` hides code from the
  compiler and only the manifest hides a dependency from cargo. Reasoning in
  `docs/decisions/0022-the-browser-never-sees-the-domain.md`.

  Two consequences worth knowing before meeting them. **A server function's
  dependencies arrive as an axum extension declared in the macro attribute**,
  not through the framework's serve configuration: that reaches the virtual DOM,
  so it exists while a page is rendered and is missing when the browser calls
  the same route afterwards — which passes every server-rendering test and
  fails the first real click. And **a lint expectation written on a server function does not
  survive**: the macro re-emits doc comments and drops every other attribute,
  so anything the generated client code needs goes at module scope.
- **The gate builds the browser's half too, and it is a line in
  `check_matrix`.** `cargo` builds the host side only, so without that line a
  client that does not compile would pass `just check` untouched and the gate
  would silently stop covering half the application. The line excludes the four
  internal crates by name; a crate added later fails there until somebody says
  which side of the split it is on, which is the right default.
- **Typed errors per crate, and no `anyhow` in core, agent, foreman or
  job.** That is the gate's bar restated only where it bites: **app** is a
  binary and may do as it likes internally, but the other four are libraries
  whose errors cross a boundary, and a boxed error at that boundary makes the
  caller's handling untestable.
- **The agent is third-party, and its quirks stop at the job boundary.** How the
  agent process is launched, spoken to and cleaned up is entirely the **job**
  crate's problem. If a change to that agent's interface would touch **core**,
  the abstraction is in the wrong place — the whole reason the crate boundary is
  there is that the agent is on somebody else's release cadence.
- **Packages carry a prefix; directories do not.** The directories are named
  for the concepts in `docs/architecture.md` §1, and the packages inside them
  are `stageman-core`, `stageman-foreman` and `stageman-job`, with the app
  published as `stageman` itself. Exactly one of those prefixes is
  load-bearing: a package whose library target is named `core` **shadows the
  sysroot crate of the same name** in every crate that depends on it, and the
  failure is not an ambiguity error but a silent one — `use core::fmt` reports
  that `fmt` cannot be found in `core`, as though the standard library had
  developed a hole. The other two are prefixed for symmetry, because a naming
  rule with one unexplained exception is a rule nobody remembers.
- **`main` is protected: a change lands through a pull request, or not at all.**
  Pushing to it is refused by the forge, for whoever is pushing — a person, an
  agent, or this project running against itself. The gate runs there as a
  required check, so a branch that cannot pass cannot merge, which is the only
  barrier in the three this project has that a laptop cannot skip. The other
  two, `just hooks` and `just check`, are conveniences that find the problem
  sooner.

  This is worth stating rather than discovering: the first thing anybody does
  with finished work is try to push it, and a rule enforced only by a remote's
  refusal teaches itself expensively.

- **Dependency versions live in `Cargo.toml` and are not restated here.** They
  are derivable, they go stale, and `just drift` cannot catch a version number
  in prose. Gotchas belong here; numbers do not.

## 4. Quality bar beyond the gate

`AGENTS.md` carries the bar the gate enforces mechanically. This is for the part
it cannot: what "done" means here, what must have a property test, what must
never panic even where the lints would allow it, what needs a benchmark before
it lands.

- **Secrets never render.** Any type that can hold a credential gets a redacting
  `Debug` and `Display`, with a test asserting the real value does not appear in
  the formatted output. A derived `Debug` on a secret-bearing type is a bug, not
  a style preference. No lint in the gate catches this, and the usual way a
  token reaches a log is a struct printed whole while somebody debugs something
  unrelated. The same applies to serialisation, and for a sharper reason: state
  is persisted by serialising the very structure those credentials live in, so a
  type that formats safely and serialises in the clear writes the secret to disk
  on the next change. One wrapper type should carry both behaviours, and one
  test should cover both, so neither can be added without the other.
- **Isolation is tested, not assumed.** The one-job-one-workspace-one-project
  invariant needs a test that genuinely tries to break out — reads another
  project's state, writes outside its own workspace — and fails to. An invariant
  defended only by construction quietly stops holding when the construction
  changes. The mechanism is no longer an open question — it is a container, per
  `docs/decisions/0012-agents-run-in-containers.md` — which raises the bar
  rather than retiring it: the test is now evidence about what that container
  actually permits, and construction is only an argument about what the code
  asks for.
- **Killing stageman leaves nothing running and nothing untracked.** Hard-killing
  the process is a supported operation with a test, not an accident recovered
  from by hand. This is a long-lived daemon on somebody's own machine, so it
  *will* be killed mid-job — and the failure mode is a silent leak rather than a
  crash, which is exactly the kind nobody notices until there are forty of them.

  This used to read "leaves nothing behind", and
  `docs/decisions/0015-a-job-survives-the-daemon-dying.md` narrowed it: a
  stopped container the instance can still name is retained deliberately, so
  that a job which has already opened a pull request or asked a question on a
  channel is resumed rather than duplicated. Only a container nothing can name
  is a leak. That makes the test *harder* rather than laxer, because it now has
  to tell the two apart — a suite that merely counts what is left behind would
  pass on the leak and fail on the feature.
- **A field added to the sealed form is defaulted, and a literal older file
  proves it.** `docs/decisions/0011-state-is-a-snapshot-not-a-database.md`
  versions nothing and says what that costs: an added field is free *with a
  default*, and without one every existing snapshot stops loading — which
  loses all of it, because there is only the one file. The gate cannot catch
  this, and neither can a round-trip test: the current writer always emits
  every field, so the input that breaks can only come from before the change.
  Write the older file out as literal text and open it.

  **A renamed variant is the other half of that sentence, and it is not free
  at all.** A default cannot help: the old spelling is already on disk and
  parsing it is the only thing that opens the file. So a renamed value that is
  serialised keeps its old name as a `serde` alias, read-only, and a test
  parses the old spelling literally. Writing uses the new name, so a snapshot
  upgrades itself the first time anything changes rather than carrying both
  for ever. This was learned by renaming a job's states and watching the test
  above go red, which is the cheapest place it could have happened.

  This is not the substituted default `.quality/gate-reference.md` forbids.
  That rule is about replacing a failure with a guess; here the default is the
  true answer, because a file written before the field existed described
  something that genuinely did not have it. If that is *not* true of some
  future field — if absence and emptiness would mean different things — then a
  default is the wrong tool and the version field 0011 already names is the
  right one.

  Written down because it was learned the expensive way: the channel map was
  added without a default, and the first thing anybody did with the build was
  fail to open an instance holding five real projects.

- **Kickoff prompts are snapshot-tested.** The prompt text the foreman
  composes is asserted as literal text, so a change to what a job is told to do
  shows up as a reviewable diff. Prompt text is the highest-leverage code here
  and the only kind that changes behaviour without changing control flow, so it
  is also the only kind that can be rewritten completely without a single test
  going red.

## 5. What this project needs installed

Beyond the Rust toolchain and `just` that `AGENTS.md` names for every project
built from this gate, this one needs:

- **A container runtime** — Docker or Podman. Every agent runs inside a
  container, including the one the foreman thinks with, so nothing here
  runs an agent without one. See
  `docs/decisions/0012-agents-run-in-containers.md`.
- **The agent image, built** — `just image`. The container tests skip without
  it rather than failing, so a green `just check` on a machine that has never
  built it is not evidence the containers work; `just image-handshake` is.
- **`dx`, the Dioxus CLI** — `cargo install dioxus-cli`, needed to build the
  browser half into a bundle. `just dashboard` serves both halves with
  reloading; `just check` needs neither it nor a bundle, because the wasm pass
  is `cargo` against a target and that is a toolchain fact.

  **It has to be the same version as the `dioxus` dependency**, which is in
  `Cargo.toml` and is deliberately not repeated here. `dx` generates the glue
  the runtime hydrates against, so the two are one thing shipped as two
  packages. A mismatch is not fatal today — it serves, and says so in red on
  every start — which is exactly what makes it worth writing down: a warning
  that does not stop anything is one people learn to scroll past.
  `cargo install dioxus-cli@<the version in Cargo.toml>` fixes it.

  A binary built by `cargo` alone therefore has a server and no client, and
  that is a working thing to run rather than a broken one — the page is
  rendered on the server and arrives complete, it just does not come alive
  afterwards. Which one you have is printed at startup, so this is never a
  guess.

**The runtime is needed for `just check`; the image and `dx` are not.** That
split used to be simpler — nothing beyond a toolchain — and
`docs/decisions/0023-the-container-runtime-is-discovered-once.md` gave up the
runtime half deliberately, on the grounds that something required in production
and optional in the tests is a difference that gets discovered late. Seven
integration tests run the binary, and the binary refuses to start without a
runtime, so there is no version of this where the gate passes on a machine that
could not run the program.

Building the image stays out, because a present runtime is not a built image: it
is a container build costing minutes and a network, so it belongs to
`just verify` — the bar for pushing — rather than to the gate you run
constantly.

**Two credentials, if you want to run the tests that cost money.**
`just image-session` drives a real agent against a real model, and
`just propose` opens a real pull request. Both read from files this repository
ignores rather than from the environment, so nothing inherits them by accident:

- **`anthropic-token`** — what the agent authenticates with. Needed by
  `just image-session`, which is the only thing exercising session resumption
  and a job running end to end.
- **`github-token`** — needed by `just propose` alone. A fine-grained token
  scoped to this one repository, with contents and pull requests write, and
  nothing else.

A third file, `instance-key`, sits beside them and is not in that list because
nothing asks you for it: `just dashboard` generates one on first use, to
encrypt the development instance it serves. Losing it costs a file with nothing
in it, and an instance that will not open is repaired by deleting both.

All of these live in the gitignored `.local` directory, and the credentials are
named here without it on purpose. `just drift` resolves every backticked path in this directory
against the repository, so citing one that exists on the machine writing the
sentence and in no clone passes locally and fails everywhere else. That is not
hypothetical: it is what this paragraph did when it was first written, and the
check caught it in continuous integration rather than here.

Neither is needed for `just check` or `just image-handshake`, and neither is
run by continuous integration — which is why those tests report as skipped
there and why that number is not zero.

**What a *job's* container needs is a different question and not this one.**
That is about the project a job works on rather than about this repository, and
`docs/decisions/0019-a-projects-tooling-is-the-projects-business.md` says why
stageman does not answer it.

