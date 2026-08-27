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

- **project** — one repository, together with the channels bound to it and the
  credentials those channels need. One instance manages several. Everything
  else in this list belongs to exactly one project, always.
- **channel** — somewhere the orchestrator watches and a job can speak into.
  Two-directional by definition, which is why it is not called a *source* or a
  *feed*: the same Slack that carries a question out carries the answer back.
  Not *integration* or *connector* either — both describe plumbing, and the
  interesting part is that somebody is on the other end.
- **signal** — one observation on a channel: an issue opened, an alert fired, a
  message posted. Signals are read and judged, not stored or addressed. They
  are deliberately not entities; see **reason** below.
- **job** — one agent, in one isolated workspace, on one project, from kickoff
  to completion. A job happens once. There is no retry and no resume: a second
  attempt is a new job with its own workspace. Not a *worker*, which implies a
  long-lived process pulling from a pool and gets the relationship backwards —
  the job is the work, not the thing that fetches it. Not a *task* or a *run*
  either, since both imply a durable intent that separate attempts belong to,
  and no such thing exists here. A job records which agent ran it, because once
  more than one can, "why did this go badly?" has no answer without it.
- **reason** — the free text the orchestrator writes when it creates a job,
  saying why it decided to. Prose meant for a human reading the dashboard, not
  a key pointing at a signal. It is the whole of a job's provenance, which is
  why `docs/architecture.md` §2 records the structured version as deliberately
  absent.
- **kickoff prompt** — the instruction text the orchestrator composes and the
  job's agent begins from. A job never writes its own.
- **agent** — reserved, always, for a third-party coding agent: the tool that
  gets configured, chosen and run. Never for stageman, never for the
  orchestrator, never for a job. This one matters more than it looks: "the
  agent decided to…" is ambiguous in exactly the place where being wrong is
  expensive, and the sentence still reads fine either way. Note that an agent
  is not only something inside a job — the orchestrator runs one too, to think
  with.

## 3. House rules

Anything someone would otherwise get wrong: framework versions and their
gotchas, the error type this project uses, which module owns which concern, the
external contracts it has to honour, and the patterns that look reasonable and
are wrong here.

State the rule and the reason. A rule without its reason gets discarded the
first time it is inconvenient — usually correctly, because a rule nobody can
justify is usually obsolete.

- **All state is one SQLite file, and credentials in it are encrypted at rest**
  with a key supplied by the environment at startup. Projects, jobs, reasons,
  prompts and credentials all live there; the file is portable and useless
  without the key. The reason is `docs/vision.md` §3 — this runs headless on a
  machine the operator owns, where an OS keychain is awkward and reading
  credentials from the environment alone would contradict editing them from the
  dashboard. What that costs is in
  `docs/decisions/0004-one-encrypted-sqlite-file.md`.
- **No secret is ever written to a log line.** Encryption protects the file, not
  the terminal, and a token escapes through a formatted struct long before it
  escapes through the database. The mechanical half of this rule is §4 below.
- **A child process's environment is constructed, never inherited.** An agent
  process gets exactly the variables it ought to have: its own agent's
  credential, and nothing belonging to any other. This is not tidiness. At
  least one agent resolves credentials by precedence and prefers a per-token
  key when it finds one, so a variable inherited from whatever shell started
  stageman silently changes who pays — no error, no log line, and no way to
  notice before the invoice. The construction is a pure function in the core
  crate precisely so it can be tested without spawning anything; the reasoning
  is in `docs/decisions/0008-one-credential-per-agent.md`.
- **A missing agent, or a credential that does not work, fails at startup,
  loudly.** Nothing works without one — not even watching a channel — so the
  worst possible moment to find out is three in the morning, on the first
  signal that mattered. Note precisely what is being checked: not whether the
  agent is logged in on this machine, which it never needs to be, but whether
  the credential this project holds for it actually works. Agent tools
  generally ship a health check of their own; call theirs, with the environment
  this project would construct, rather than inferring health from the first
  failure of real work.
- **The app crate is an Axum server, and the orchestrator shares its runtime.**
  Dioxus fullstack server functions are Axum handlers, and the orchestrator runs
  in that same process rather than beside it. So orchestrator work must never
  happen on the request path: watching a channel, judging a signal and
  supervising a job all belong on their own tasks. A dashboard that stops
  painting because a job is thinking is the failure this rule exists to prevent.
- **Typed errors per crate, and no `anyhow` in core, orchestrator or job.**
  That is the gate's bar restated only where it bites: **app** is a binary and
  may do as it likes internally, but the other three are libraries whose errors
  cross a boundary, and a boxed error at that boundary makes the caller's
  handling untestable.
- **The agent is third-party, and its quirks stop at the job boundary.** How the
  agent process is launched, spoken to and cleaned up is entirely the **job**
  crate's problem. If a change to that agent's interface would touch **core**,
  the abstraction is in the wrong place — the whole reason the crate boundary is
  there is that the agent is on somebody else's release cadence.
- **Packages carry a prefix; directories do not.** The directories are named
  for the concepts in `docs/architecture.md` §1, and the packages inside them
  are `stageman-core`, `stageman-orchestrator` and `stageman-job`, with the app
  published as `stageman` itself. Exactly one of those prefixes is
  load-bearing: a package whose library target is named `core` **shadows the
  sysroot crate of the same name** in every crate that depends on it, and the
  failure is not an ambiguity error but a silent one — `use core::fmt` reports
  that `fmt` cannot be found in `core`, as though the standard library had
  developed a hole. The other two are prefixed for symmetry, because a naming
  rule with one unexplained exception is a rule nobody remembers.
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
  unrelated.
- **Isolation is tested, not assumed.** The one-job-one-workspace-one-project
  invariant needs a test that genuinely tries to break out — reads another
  project's state, writes outside its own workspace — and fails to. An invariant
  defended only by construction quietly stops holding when the construction
  changes, and the isolation mechanism is an open question, so it will change.
- **Killing stageman leaves nothing behind.** Hard-killing the process is a
  supported operation with a test, not an accident recovered from by hand: no
  stranded workspace, no orphaned agent process, nothing left running. This is a
  long-lived daemon on somebody's own machine, so it *will* be killed
  mid-job — and the failure mode is a silent leak rather than a crash, which is
  exactly the kind nobody notices until there are forty of them.
- **Kickoff prompts are snapshot-tested.** The prompt text the orchestrator
  composes is asserted as literal text, so a change to what a job is told to do
  shows up as a reviewable diff. Prompt text is the highest-leverage code here
  and the only kind that changes behaviour without changing control flow, so it
  is also the only kind that can be rewritten completely without a single test
  going red.
