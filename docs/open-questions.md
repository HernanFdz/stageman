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

- **Should the runtime be Podman only?** The list compiled in by
  `docs/decisions/0023-the-container-runtime-is-discovered-once.md` is ordered
  Docker first, so that is what continuous integration finds and what most
  machines will. The question is whether it should name Podman and nothing
  else.

  The argument for is the one measured under the question below: rootless
  Podman was the only nesting configuration that kept every capability off and
  seccomp filtering on. Choosing one runtime everywhere also makes what is
  tested and what is run the same thing, which an ordered list of two does not.

  The argument against is that Docker is what is already installed on most
  machines, and a list naming only Podman means somebody with a working
  container runtime is told they have none. That is a poor first impression for
  a reason they did not choose.

  Two things would settle it and neither is expensive. Whether this project's
  adapter works against Podman at all — it shells out for a version, a run, a
  listing and a label, none of which has ever been exercised there, and the
  answer is one afternoon. And whether anything in the nesting answer below
  actually requires it, rather than merely preferring it.

- **How does a job run containers of its own?** Answered for provisioning in
  general by
  `docs/decisions/0024-the-agent-provisions-what-a-project-needs.md` — the
  agent sets a project up inside its own container — and this is the one
  prerequisite that answer cannot supply, because it is the container itself.

  It is not hypothetical: it is this project. Since
  `docs/decisions/0023-the-container-runtime-is-discovered-once.md`,
  `just check` needs a container runtime, so an agent working on this
  repository needs one inside its container. Installing a client is not having
  a runtime — measured: a container with the Docker CLI answers
  `docker --version` and fails `docker version`, since only the second reaches
  a daemon.

  **Three shapes, and two of them were measured rather than reasoned about.**

  *Mounting the host's container socket* is what most continuous integration
  does, and it is disqualified here rather than merely risky. It breaks the
  invariant in `docs/architecture.md` §2 directly: a container holding that
  socket listed its siblings, read another container's environment — which is
  where `docs/decisions/0008-one-credential-per-agent.md` puts an agent's
  credential — and executed inside it as root. That is every other project's
  credentials, from any one job.

  *A privileged container* running its own daemon works, and costs the whole
  boundary: every Linux capability, the host's block devices, and the host's
  root filesystem readable through the raw device. It has no socket to abuse
  and reaches the same place by a longer road, so it is not the safer of the
  two despite looking like it.

  *A rootless container running rootless Podman* also works, and is the one
  worth building on. Measured in that configuration: no capabilities at all,
  seccomp filtering still active, no host devices, and the innermost root two
  user namespaces away from any real user. It needs `/dev/fuse`, a non-root
  user in the container, and — on a distribution that enforces SELinux — its
  labelling relaxed.

  What that does *not* buy is worth being equally clear about: the kernel is
  still shared, so an escape is still a kernel bug away. The difference is
  where an attacker starts, not whether the door exists.

  Settled by trying the third shape on the machine it has to work on. What is
  measured was measured on one architecture, under a virtual machine, on a
  distribution that enforces SELinux; continuous integration is none of those,
  and the SELinux flag in particular may be unnecessary there. Note also that
  the requirement itself is a consequence of 0023 rather than a fact of nature
  — a gate that did not need a runtime would not need any of this, and that is
  the cheapest available answer if the rest turns out to be expensive.

- **Is the environment the right place for the encryption key?** It is where
  `STAGEMAN_KEY` arrives today, for the reason
  `docs/decisions/0017-the-runtimes-path-is-recorded-in-the-instance.md` gives
  in passing: storing it beside the file it encrypts would defeat the
  encryption. That says where it must *not* be and settles nothing about where
  it should be.

  Get the threat model right before designing for it, because the obvious
  statement of it is wrong. A process environment is not world-readable: on
  Linux `/proc/<pid>/environ` is readable only by the same user and by root,
  and macOS has no equivalent at all for another user's processes. So the
  exposure is to **anything already running as the same user** — which on a
  developer's own machine is a large set, and is exactly the machine
  `docs/vision.md` §3 has this running on. It is also inherited by children:
  the container runtime client is spawned with the daemon's environment, so
  the key is in that process too. Not in the *container* — what crosses that
  boundary is only what `--env` names, which is the mechanism
  `docs/conventions.md` §3 relies on — but the client is one more process
  holding it.

  The candidates are per-platform and none is portable, which is the hard
  part. A service manager is the leading shape: a systemd unit's
  `LoadCredential=` passes a secret through a file descriptor rather than the
  environment, which is strictly better than `EnvironmentFile=` and better
  than what happens today. The analogues are launchd with the Keychain, and a
  Windows service with the credential manager. All three arrive with
  installation rather than with the program.

  Settled by distribution, and deliberately not before: what installs this
  decides what can hold a secret for it, and building a keychain integration
  against a guess about packaging is how that gets built twice.

- **Does the dashboard need authentication before it leaves `127.0.0.1`?**
  Nothing authenticates a request today, and the default address makes that
  survivable rather than correct: anything that can reach the port can read
  every project's name and, once the views exist, change what this instance
  runs. The address is configurable, so the protection is a default and not a
  boundary.

  Worth stating what it is *not*: no credential is served — see
  `docs/decisions/0022-the-browser-never-sees-the-domain.md` and the invariant
  in `docs/architecture.md` §2 — so this is about who may operate the instance
  rather than about what leaks from reading it.

  Settled by deciding whether this is ever reached from another machine.
  `docs/vision.md` §3 has a daemon on somebody's own machine, and if that holds
  the answer is to document the default loudly and stop. The moment somebody
  wants it from a phone, the answer is a real one and the cheapest real one is
  probably a reverse proxy that already does this, rather than a login page
  here.

- **How does a page find out that something changed?** Everything the dashboard
  shows is read once, while the page is rendered. A job finishing, a signal
  arriving, an orchestrator deciding — none of it reaches a browser that is
  already open, and this is the first thing anybody will notice.

  The mechanisms are known and the choice between them is not: polling the
  route on a timer, which is trivial and wrong at any interval you pick;
  server-sent events, which fit one-directional updates exactly; or the
  framework's websocket support, which fits and costs more. What actually
  decides it is what the *first* changing view needs, and none exists yet.

  Note the interaction with the question above it. Anything long-lived is a
  connection somebody has to authenticate, so answering that one first is
  cheaper than answering it twice.

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

- Next, move the end-to-end tests out of the crates they test. A test that drives
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
