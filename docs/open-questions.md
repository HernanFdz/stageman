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

- **Should a job's tools be a prerequisite rather than a courtesy?**
  `docs/decisions/0055-a-job-says-why-it-stopped.md` measured something that
  changes what the endpoint is for: with it unreachable, a job told to call a
  tool failed outright about half the time rather than carrying on without
  tools. `docs/decisions/0034-tools-are-served-not-shipped.md` recorded the
  opposite — an unreachable endpoint "does not error" — and that was true while
  nothing insisted on a tool.

  Nothing is broken today, because the daemon serves the endpoint before it
  runs anything. What is undecided is whether a job should *check* before it
  starts, and what it should do when the check fails. The cheap answer is to
  reach the endpoint from inside the container once, before the first turn, and
  fail the job loudly rather than let its agent die halfway through with a
  message about a container exiting. Settled by deciding what a job that cannot
  start should look like in general, which is the same question the expiring
  credential above is wearing a different hat of.

- **When does a job retire itself?**
  `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md` answers the
  half a person performs: an operator stops a working job or ends an idle one
  with a verdict, and ending it removes the container, the session in it and
  the images nothing else needs. What is left is the half nobody presses.

  An agent that believes it is finished already says so — that is what
  *proposed* means in
  `docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md` — and
  what turns a proposal into an ending is a person. So the question is whether
  a job should be able to ask for its own retirement on a channel and act on
  the answer, which is the same shape as everything else under
  `docs/decisions/0005-conversation-happens-on-channels.md` and falls out of
  the channel work rather than needing a design of its own.

  Deliberately not a timer, and worth writing down so nobody adds one: the
  thing being reclaimed is somebody's unread work, and the only signal that it
  has been read is a person saying so.

  Note what has stopped being a reason to hurry. A container left behind used
  to accumulate an image with it, and since
  `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`
  every container from one recipe shares one image. What a retained container
  costs is its own writable layer, and memory if its tunnel is still answering.

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

  Since `docs/decisions/0066-a-foremans-container-runs-only-while-a-turn-runs-in-it.md`
  the container half is settled the other way from what 0043 left: a
  foreman's container exists for as long as its project does and runs only
  while a turn runs in it, so a message after idleness pays a container
  start, measured at a tenth of a second. What a held connection would save
  is the session load every turn pays, not the start.

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

- **How is a purse's expiry noticed while nobody is watching?** It will
  expire, and it lands on every job charging it at once. Since
  `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md` the repair
  is a paste: a replaced credential reaches a foreman at its next message
  and a job when it resumes, with no container recreated. What is
  undecided is how the expiry is noticed and where it is said. The options
  run from failing each job loudly and showing it on the dashboard, to
  pausing the instance and saying so on the channel a human is actually
  reading. The second is more work and is probably right, because a
  dashboard nobody is looking at is exactly where this failure would
  otherwise sit until morning. Settled by deciding what a job that cannot
  start should look like in general — the same question wearing a different
  hat, and worth answering once.

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

- **Should the instance keep a flight recorder?** Everything it does is a
  function of what it was constructed from and the events it was fed since,
  per `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`,
  so a daemon that kept that sequence in memory and wrote it out when
  something went wrong would turn any production failure into a scenario
  that reproduces exactly. The design makes it cheap. What makes it a
  question is that the sequence holds what people said on channels and the
  bodies of tool calls, and the file it started from holds sealed
  credentials — a second place secrets could live, with a retention and a
  redaction question of its own. Settled by the first failure a log line
  could not explain, and not before: noted so that it is not forgotten,
  deliberately not built.

- **Should the image sweep be scoped by instance?**
  `docs/decisions/0054-a-container-says-which-instance-started-it.md` labels
  every container with the instance that made it, so a sweep removes only
  its own. An image is named by its recipe, per
  `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`,
  and shared by every instance on the runtime — and the sweep that removes
  an image nothing needs asks only this instance's containers whether they
  need it. Observed with two instances on one runtime, which is the ordinary
  arrangement of `just dev` beside the real daemon: the second instance,
  waking, tried to remove the images the first one's containers were built
  from, and was refused only because those containers still held them. An
  image the other instance's *next* job needs is held by nothing, so a
  development instance can cost the real one a rebuild, minutes and a
  network, on its next job. Settled by deciding whose an image is: the
  runtime's, pruned by a person as `docs/conventions.md` §5 already says of
  dangling ones, which means the sweep stops removing images at all; or each
  instance's, labelled as containers are, which means two instances build
  the same image twice.

- **Should a job's kickoff be kept as its parts rather than as one text?**
  The instruction a job's agent begins from is composed by the foreman crate
  from parts with different jobs: the work, the repository, where the tunnel
  is, which variables are in the environment, and the rules that never
  change — nothing is checked out, the tools are signed in, stop at a
  proposal. The record keeps only the rendered text, so a page can show it
  only whole, folded under the reason since the dashboard pass, and a reader
  looking for the work has to find it inside the boilerplate. Kept as typed
  parts, the page could lead with the work and fold the rest, and a change
  to the standing text would not rewrite what every older job was told.
  Against: the domain would carry the foreman crate's structure, which
  `docs/conventions.md` §3 keeps behind that crate's boundary; the snapshot
  tests that assert the literal text would move to the parts; and every job
  the last release wrote holds one text, which is a bridge or a job shown
  whole. Settled by whether a page reading the parts is worth a record's
  structure crossing that boundary.

- **Does activating public distribution accept a redirect address on
  localhost?** `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md` installs the instance's Slack app on a second workspace
  by the platform's redirect, which needs public distribution activated on
  the app's page once, and the platform's checklist for it says the redirect
  addresses must be SSL. The authorisation step honoured a plain address on
  localhost, measured on 2026-09-25, and the checklist was not tried. Settled
  by pressing it on a laptop instance; if it refuses, the Instance page has
  to say that a second workspace on a laptop instance takes an app of its
  own on the project, since installing on workspaces other than the app's
  own is what a domain-hosted instance holding several people's projects
  does, and that instance has the address the checklist wants.

- **Should a project moved to another Slack app bring its rooms along?**
  `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`
  lets a project move between an app of its own and the instance's app,
  and a move moves nothing on the platform: the new app's bot is in none
  of the rooms the old one made or was invited to, so until a person
  invites it, the foreman's transcript fails to post, a live job's room
  hears nothing, and a watched room sends no signal. The old app is still
  installed and still in those rooms at the moment of the move, and its
  token has the scope that invites, so it could invite the new bot into
  each of them itself, within one workspace. Settled by measuring whether
  the platform lets one app's bot invite another app's bot user; if it
  does, the move invites, and the README's sentence about inviting by hand
  goes.

- **Should a kit be allowed to leave a model or an effort to the agent's
  default?** `docs/decisions/0048-a-job-runs-on-a-kit.md` lets a kit say
  *default* for both, and the adapter resolves it at each turn, so what ran
  is known only from what the session reported. The chip then reads
  *Default*, which says nothing to a person, and a job's record says what
  was asked rather than what ran. Requiring a value would make every chip a
  name and every record exact; it would also make every kit a thing to
  re-pin when the agent's models change, which the default absorbs today,
  and every kit the last release wrote one to bridge or refuse. Settled with
  a record amending 0048 either way.

- **Should where the App is installed be refreshed from the platform?**
  It is learned from the platform's redirects and kept, per `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`, so an
  installation removed on the platform stays listed here until a mint
  fails and somebody forgets it. The platform lists an App's
  installations for the App's key in one request, which a control on the
  Instance page could ask on demand and reconcile against. Worth doing
  when a ghost is met, and not before: a request on every page read is
  what the record refused.

- **Should a project's containers run on a runtime reached over a network?**
  `docs/decisions/0083-hosting-is-one-instance-per-customer.md` parked this
  rather than deciding it, and named what it costs so that it is not
  re-derived. The control path is free: the world spawns the runtime with
  the daemon's own environment minus the `STAGEMAN_` names, so a host named
  in the runtime's own variable already redirects every command the instance
  renders, and docker's SSH transport refuses a password in the address,
  runs its own client on the remote, and shares one connection through the
  user's ssh configuration. Two paths are host-to-host and break: the
  tunnel port is published on the runtime host's loopback and the probe of
  `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
  and the proxy connect to this machine's, with the probe's budget inverting
  over a network so that a slow close reads as answering; and every
  container reaches the tools endpoint and the credential route through the
  host gateway name, which is the remote host. Three ways across are known.
  An SSH connection kept open by the world, carrying a forward for the
  probe and the proxy and a reverse forward for the tools, which on a Linux
  runtime host needs the host's sshd to bind the reverse forward on the
  bridge address. The tools endpoint exposed over TLS on an address of its
  own, warrant-authenticated, which makes it a real service. Or everything
  riding the runtime's own control channel: a relay written into the
  container at creation and on every resume, as
  `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
  writes the wrapper, driven by a second kept-open exec, so that tools and
  the credential route reach the instance with no inbound network at all,
  which would also retire the listener on every interface for a local
  runtime; the adapter's translation has a branch for a stdio tool server,
  measured to exist and not to work by
  `docs/decisions/0034-tools-are-served-not-shipped.md`. Two smaller edges:
  runtime commands have no timeout, so a stalled transport hangs a step
  rather than a request; and the endpoint belongs in the environment the
  instance hands the process, never in the arguments, since an argument
  change costs every replay fixture. Settled by a customer asking for the
  dashboard hosted and the compute their own, and not before.

- **What are Slack's acknowledgement window and retry schedule under Socket
  Mode?** `docs/decisions/0085-the-instances-apps-may-be-held-elsewhere.md`
  rests its relay's floor on the rules Slack documents for its HTTP
  delivery — three seconds, three retries at once, a minute and five, and an
  app's subscriptions disabled above 95 percent of deliveries failing in an
  hour — and on the Socket Mode page saying an event is acknowledged so that
  Slack knows whether to retry, and that the rules apply to both. Whether
  the window and the schedule are exactly those over a socket was not
  measured. Settled by the probe tooling this repository does not track:
  open a connection, withhold one acknowledgement, and record when and how
  often the envelope comes back and what its retry fields say.

- **How does a relay route an event that names several workspaces?** An
  event in a channel shared between workspaces carries more than one
  authorisation, per the envelopes 0081 recorded, so a relay forwarding by
  workspace has to choose: the event's own team, or every bound team. Two
  instances receiving one event is the duplication 0085 refuses for one
  instance; one instance missing an event its bot can see is the other
  failure. Settled when the relay is built, from recorded envelopes rather
  than from the documentation, which does not say.

- **Should an instance cap how many jobs it runs at once?** Nothing limits
  how many jobs a foreman has working, so a small machine overcommits: the
  runtime kills the largest process in a container and the turn fails
  visibly, or the machine's own killer takes the daemon and
  `docs/decisions/0065-a-panic-aborts-the-daemon.md`'s service manager
  restarts it. A hosted plan is sized by its machine, per 0083, so a cap is
  the first thing a plan would set, and a self-hosting convenience. What a
  foreman is told when the cap is reached is the design: the tool that
  starts a job refuses with a reason the foreman can say on the channel,
  since a queue is what `docs/vision.md` §2 refuses. Settled by the first
  hosted plan, or the first laptop that swaps.

- **Should a purse be a gateway's?** 0085 decides that a model credential
  can be held elsewhere the way an app can, and defers the shape: a purse
  would become an address and a key beside the provider's own kinds, held
  or in custody, delivered as the two variables each agent documents for a
  gateway — or, once the protocol's providers extension stabilises, in a
  header per session, which
  `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md` names as the
  delivery to prefer for a gateway — and the base-address variable would
  join the names
  `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`
  refuses, since one overriding it redirects an agent's traffic exactly as
  an inherited key changes who pays. Three facts read from the agent's
  documentation on 2026-09-26: a gateway credential replaces a subscription
  login, so what goes through a gateway is metered usage billed to the
  upstream key's owner; a gateway must forward whatever the agent sends,
  and the agent grows with each release, so it is maintained infrastructure;
  and the vendor ships a self-hosted gateway of its own. What is unread is
  the vendor's terms on resale. Settled by reading the terms, and by the
  first customer who brings no credential of their own.

- **Should a platform's or a channel's credential be refused by its shape
  before it is sent?**
  `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md` refuses a
  purse whose text has the other box's shape before anything is asked of
  the provider.
  `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
  checks a project's token and its Slack tokens by sending them, so
  whatever is in the box reaches the platform: a paste of the wrong thing,
  or, until a box that takes a secret stopped being a password field per
  `docs/conventions.md` §3, the dashboard's own password filled in by a
  browser. Each of those credentials has a documented beginning, so the
  same refusal is available, as a function per platform in the crates that
  already own what each credential is checked with. What is undecided is
  whether a beginning is stable enough to refuse on: a platform that
  mints a new kind of token would be refused here until this project
  learned of it, which is a failure that checking by asking does not
  have. Settled by reading how each platform documents its token formats,
  and whether it promises them.

## Next

Intended next steps, in order, each with its reason. Written as intentions, not
progress: "next X, because Y" — never "X is 60% done", which is both derivable
and wrong within a day.

- Next, what a turn cost, as a record of its own: every turn, a job's and
  the foreman's, kept with its kit, what the session reported and the
  usage the adapter sent — tokens everywhere, a cost where the agent says
  one, never a total and never a price of this project's own. Before the
  second agent, because a second agent is where *what did that cost, and
  on which purse?* starts having more than one answer. The adapter's pin
  moves with it, since a later release names the model beside the usage.

- Then the second agent, as a record of its own, because being agnostic
  about agents has been a commitment since
  `docs/decisions/0006-agents-are-pluggable.md` and one adapter cannot
  show whether the contract is the protocol's shape or that adapter's. The
  one that reaches several providers first, since it is what the purse was
  shaped for; then the vendors' own agents, each bringing a provider the
  first can also charge.

- Then the instance's apps held elsewhere, per
  `docs/decisions/0085-the-instances-apps-may-be-held-elsewhere.md`, Slack
  first because that is where the setup cost is — ten minutes per project
  under 0081 — and GitHub second. In chunks: the field and the Instance
  page's sentence; the four calls against an address and the install link
  from the holder; the scenarios that pin an install, a mint, an open and a
  refused second connection; the bridge from the older file.

- Then the cap on concurrent jobs, once its open question above is
  answered, because it is the first thing a hosted plan sizes.

- Then the small things the dashboard pass of
  `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` left:
  the projects list's rows padding themselves on a rule that always
  matches, which the job rows had and fixed on the list item; and whether
  the dialog takes focus when it opens in a headed browser, which a
  headless one said it did not.

- Then make the container tests a recorder, because what they check is what
  the pinned runtime and the pinned agent actually do, which is a recording
  rather than a test. A recipe runs reality, captures what it prints and how
  it answers, and writes the captures as the fixtures the in-memory tests
  replay; the assumptions that cannot be recorded — that a published port
  accepts and then closes — stay in that recipe as checks against the real
  runtime. The gate then runs entirely in memory, which is what
  `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
  asks for and what makes the bar cheap enough to keep raising.

- Then add exploration, because replay pins what somebody thought of and
  nothing yet finds what nobody did. A seed is a random world under the same
  simulation, answering effects with faults and crashes and checking an
  invariant after every step; a seed found failing is committed as a replay,
  so the bug costs one run to rule out for ever. It belongs outside the gate,
  on a recipe with a seed count and a budget and a workflow with a manual
  trigger and a schedule, with one fixed seed in `just verify` so the harness
  cannot rot unnoticed.

- Then cover Slack against regression, because it works and nothing in the
  repository would notice if it stopped. Both directions have been driven end
  to end against a real workspace — a job speaks in a room of its own, and
  a reply reaches it in the room it was given. So the open item is
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

- Then the smaller things the room design made cheap, in no particular
  order: a direct message with the app as a room where no mention is needed;
  a private room for a project whose own rooms are private.

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

  The scenarios of 0056 belong there too, and the simulated world with them:
  test support that names the instance and nothing in the app, which is the
  same rule from the other direction.


