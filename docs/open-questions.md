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

- **Should a message reach a job that is already running?** Today it cannot,
  and that is a limitation accepted deliberately rather than a gap nobody
  noticed. A reply is delivered by resuming a stopped container, so it lands
  only between turns — which covers the case the design was built for, an agent
  that asked something and stopped. It does not cover the case a person will
  hit first: realising *after* starting a job that the agent needs a piece of
  context it was never given.

  The symptom is "cannot message a running agent" and the cause is further
  down. The protocol delivers a prompt to a session and the agent works until
  its turn ends, so a mid-turn message needs the connection held open — the
  question below about holding a foreman's container open — *and* an agent that
  will read something while working. The second half is not this project's to
  decide: it is a property of whichever agent is running, and neither adapter
  examined has been checked for it.

  Settled by finding out what an agent does with a second prompt mid-turn.
  Until then the daemon refuses such a message and says so on the thread, which
  is the honest version of not supporting it. Note what that refusal is worth
  keeping even afterwards: it is also what stops two replies resuming one
  container at once.

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

- **Does a container behave the same way on every runtime this has to run on,
  now that a turn is a process rather than a restart?**
  `docs/decisions/0015-a-job-survives-the-daemon-dying.md` rests on one
  measurement taken on Docker Desktop on macOS: hard-killing the attached
  client leaves the container exited with its filesystem intact, and starting it
  again resumes the session. The mechanism was not identified, so it is evidence
  rather than a guarantee — and the whole resume design now depends on it, which
  makes this the most load-bearing unverified claim in the repository.

  `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
  replaces the half that was measured. A turn is no longer a stopped container
  being started; it is an agent run inside one that is already up, and a hard
  kill now ends that agent while leaving the container going. Neither half of
  the original observation describes it, so the claim has to be taken again
  rather than carried over — and the new one is about whether an agent run this
  way resumes its session at all.

  Settled by running the probe, in its new shape, on a Linux engine — which is
  where CI runs — and on a rootless runtime. If it does not generalise, the fallback is not obvious and
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

  `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` raises the price
  of leaving one. A retained container is no longer only a writable layer
  nobody reclaims: it is a reachable one, serving whatever its agent last put
  up, for as long as it exists. Retirement is now the only way to close a
  tunnel, which turns this from housekeeping into the answer to a question that
  record could not answer for itself.

  `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
  raises it again, and changes what is being reclaimed. A container showing
  something is not stopped, so an agent that leaves a server bound holds a
  *running* container indefinitely — this stops being about disk and becomes
  about memory on somebody's laptop. The cheap answer in the meantime needs no
  design at all: ask the agent to stop what it is showing, which is a message
  to a job, and that already works.

- **How is the foreman's long-lived container held open?**
  `docs/decisions/0012-agents-run-in-containers.md` puts the agent the
  foreman thinks with in one long-lived container, on the reasoning that
  per-signal containers buy nothing and cost a start every time. What is built
  starts one per question, which is a gap rather than a violation only because
  nothing calls it yet — it becomes a violation the moment the foreman
  does. The obstacle is shape, not effort: the protocol library scopes a
  connection to a closure, so a connection outliving one call means a task that
  owns it and a channel to speak through, and that task's failure and shutdown
  become things somebody has to handle. Settled by building it, and worth
  building before the foreman has a second caller rather than after.

  **It has one consumer, not two, and the correction is worth keeping.** This
  entry used to claim that a job which can be *answered* needed the same
  mechanism. It does not. A job's agent asks and stops, so its turn is over
  and its container is merely stopped, with the session intact — and
  `stageman_agent::resume` already restarts that container, loads the session
  and delivers new text to it. A reply is that call with the reply as its text.

  A held-open connection would only be needed if an agent had to block inside
  a turn waiting for an answer, which is the design
  `docs/architecture.md` §2 forbids outright. So the honesty rule that produced
  *ask and stop* removed this blocker as a side effect, and what is left here
  is the foreman's own case: not paying a container start per signal,
  which is a cost rather than an obstacle.

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

- **How is a wildcard certificate obtained for an instance's domain?** A
  certificate covering `*.<domain>` cannot be issued over HTTP validation, so
  the ordinary path — and the one somebody will try first — does not work. It
  needs DNS-01, which means the issuing client holds a credential for the
  domain's DNS.

  Not this project's code, and recorded here anyway because it is the step
  most likely to be discovered late and the only one in
  `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`'s deployment
  story that cannot be improvised on the day. Note that a tunnel-style
  provider avoids it entirely by terminating TLS on its own certificate, which
  may make the question moot rather than answered.

  Settled by picking the forwarding infra, since the answer is a property of
  that choice rather than an independent decision.

- **How does a page find out that something changed?** Everything the dashboard
  shows is read once, while the page is rendered. A job finishing, a signal
  arriving, a foreman deciding — none of it reaches a browser that is
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

- **Should the browser's bundle live inside the binary?** What ships is a
  server executable and a `public/` directory of static assets beside it, which
  is two things where the rest of the deployment story is now one:
  `docs/decisions/0035-an-image-is-built-never-named.md` puts the agent's
  recipe in the binary, and nothing else is carried alongside. Embedding the
  bundle the same way would make the artifact a single file somebody can copy
  anywhere.

  What makes it a question rather than an obvious next step is who owns that
  directory. The framework decides its layout and reads it from disk, and the
  function that finds it already restates a private rule of the framework's
  and says in its own comment that restating somebody else's rule is drift
  waiting to happen. Embedding means owning that rule outright, and serving the
  files ourselves rather than letting the framework do it.

  Settled by finding out how much of the framework's serving has to be
  reimplemented to do it — if the answer is a route that reads from a compiled-in
  map instead of a directory, it is cheap; if it is the asset pipeline, it is
  not.

- **When do credentials move from agents to providers?**
  `docs/decisions/0048-a-job-runs-on-a-kit.md` decides that they will. With an
  agent that reaches several providers, the credential is the provider's rather
  than the agent's, and a job must be handed exactly the one its kit names or
  the billing failure `docs/decisions/0008-one-credential-per-agent.md` guards
  against returns through a new door. A provider is platform-shaped — a closed
  set, because naming the variable an agent reads it from is code — and
  anything stranger is already a project variable.

  What is undecided is only when. Moving them before any configured agent has
  more than one provider is a snapshot migration for a distinction nothing yet
  has, so the honest answer is: with the first adapter that does. Settled by
  that adapter landing, and worth deciding in the same change as its kit
  variant, since that variant is where the provider has to be named.

## Next

Intended next steps, in order, each with its reason. Written as intentions, not
progress: "next X, because Y" — never "X is 60% done", which is both derivable
and wrong within a day.

- Next, implement `docs/decisions/0034-tools-are-served-not-shipped.md`,
  because everything queued behind it inherits the shape and one of those
  things is how a job speaks. The instance serves its own tools over the
  listener it already has; the two programs, the endpoint file, the thread
  file and the mechanism that copies them into a stopped container all go; and
  the warrant becomes per session rather than per container, delivered on the
  session declaration and re-supplied on every resume.

  **The one thing worth building carefully is the failure.** An endpoint the
  container cannot reach does not error — session creation succeeds and the
  agent simply has no tools, which reads exactly like an agent that chose not
  to use them. That is the shape this record was written to avoid inheriting,
  so it wants a test that asserts the tools are *there*, not merely that a
  session started.

- Next, cover Slack against regression, because it works and nothing in the
  repository would notice if it stopped. Both directions have been driven end
  to end against a real workspace — a job speaks on its project's channel, and
  a reply reaches it through the thread it was said in. So the open item is
  not whether it works. It is that the only evidence lives outside the
  repository, and a clone cannot re-run it.

  Every automated test of either half stops at a blocked network, which proves
  both names were found and says nothing about whether Slack accepts them —
  the half that fails in front of a person. There is no `slack-token` in
  `.local` and no test that spends one, which is the same shape as
  `just image-session` and `just propose` and probably wants the same answer.

  Note the interaction with
  `docs/decisions/0034-tools-are-served-not-shipped.md`: the speaking half
  moves from a program in the image to a tool the instance serves, so a test
  written against the program would be written twice. Worth doing after that
  lands rather than before.

- Then have the daemon post into a job's thread when the **agent or its
  container fails**, which is the one case the agent cannot report on: a
  crashed agent says nothing, so a job that dies is silent everywhere except
  the dashboard.

  Deliberately only that case. Reporting what a job *did* is the agent's, and
  the reasoning is worth keeping because the opposite was argued first: having
  the daemon post every outcome would be more reliable, and it would put a
  second author of channel text beside the agent, duplicating whatever the
  agent already said. The split that survives is that each says what only it
  can — the agent knows what happened, and the instance knows when the agent
  stopped being able to say so.

  Note what this does not fix. A job's answer still reaches nobody but the
  channel: `Progress::Idle` carries no text, so the dashboard shows that a
  job ended and never what it said. That is a separate gap and probably wants
  the answer recorded on the job.

  One coupling that decides the order. The instruction a job begins from
  currently ends by telling it to ask and *stop* — "do not wait, nobody is
  watching this terminal". Slack makes that false, but it cannot honestly
  become "ask and wait" until inbound works. So the prompt changes with the
  second piece, not the first, and `docs/conventions.md` §4 makes that a
  reviewable diff.

  Slack first among channels because it is the escalation path rather than the
  richest source of work: until a job can ask something, nothing can safely run
  unattended, so every other channel is blocked behind this one. See
  `docs/decisions/0005-conversation-happens-on-channels.md`.

- Then move the end-to-end tests out of the crates they test. A test that drives
  a whole flow — a job from kickoff to a cloned repository, a session surviving
  its container stopping — belongs in `tests/`, where it is a separate crate
  that may use only the public API. That is what an end-to-end test should
  exercise, and it is a check nothing else performs: the missing re-exports that
  once made `Greeting` and `Answer` unreadable without taking the protocol
  library as a direct dependency would have shown up immediately as a test that
  would not compile.

  It has since grown a second reason, and the more pressing one.
  `app/tests/starting.rs` now carries two concerns: a binary that starts, and
  the routes that binary serves. Its own module documentation says so and
  points here. They share a harness rather than duplicating one, which is the
  right trade until the move happens and the wrong one afterwards — the move is
  where they part company.

  The split is not clean and the rule is what decides it, rather than the word
  *end-to-end*. A test reaching for a private helper — the container-argument
  builders, the label constant, the name parser — is a unit test by definition
  and stays where it is. Roughly half of the ignored tests are in each group,
  so this is a move for some and not a reorganisation of all.


