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
  credentials those channels need, the variables its jobs are given, and the
  agents it runs on: one for its foreman to think with, and a non-empty set its
  jobs may use. One
  instance manages several. Everything else in this list belongs to exactly one
  project, always — including the foreman, which is a project's rather
  than an instance's, per
  `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`.
- **foreman** — the one thing per project that reads what a person says and
  decides what to do about it: answer, do nothing, or start a job. It runs an
  agent to think with, in one container of its own — kept for as long as the
  project is, and running only while a turn runs in it, per
  `docs/decisions/0066-a-foremans-container-runs-only-while-a-turn-runs-in-it.md`
  — and it is the only thing that composes an instruction: a job never writes
  its own.

  Called a foreman because the word is a *role* and the job is the work it
  assigns: "why did the foreman do that?" is a question with an answer, in a
  way that the word this replaced never managed. It was **orchestrator** until
  `docs/decisions/0030-the-orchestrator-is-a-foreman.md`, which is why every
  record numbered below that one says the old word and means this. Not a
  *supervisor* or a *coordinator*, which describe watching rather than
  deciding, and not a *dispatcher*, which is only the third of the three
  things it can do.

- **inbox** — the messages waiting for a project's foreman, or for a job
  since `docs/decisions/0069-a-message-reaches-a-working-job.md`, in the
  order they arrived. It exists only while its owner is working: a foreman
  or a job with nothing to do has nothing waiting. For a foreman that is a
  property of the type rather than a rule anybody keeps; for a job, whose
  inbox is a field beside its progress, it is kept by every transition the
  instance makes — a turn's end starts the next message's turn at once, a
  stop tells what was waiting and drops it — and checked by the simulation
  after every step. A job's is delivered into the turn that is running, by
  steering, when that turn's conversation is open, and waits otherwise. Not
  a *queue*, which names the structure instead of what is in it, and would
  invite a second one somewhere else.

  It outlives this process, the way a job does. A message in hand when the
  daemon is killed is still in hand when it starts again, and startup is what
  puts that foreman back to work — see
  `docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`, which
  exists because for a while nothing did, and a foreman that had been
  interrupted accepted messages for ever and answered none of them.

- **turn** — one message, handled from being handed to a foreman or a job until
  its agent stops, together with whatever a job's agent was steered while it
  ran — see `docs/decisions/0069-a-message-reaches-a-working-job.md`. The
  protocol's own word, and the unit everything else is scoped to: a turn is
  what an inbox entry buys or is delivered into, what a thread collects, and
  what "idle" means the absence of.

- **mention** — how somebody says they mean stageman rather than each other.
  It is the whole of what makes a person's message ours: nothing without one
  is read, in a thread or at the root — see
  `docs/decisions/0031-a-mention-is-what-makes-it-ours.md`. Since
  `docs/decisions/0068-a-mention-is-shown-its-thread.md`
  the thread a mention was said in is *shown* to the turn it starts,
  untagged messages included, which is a different thing from being read:
  nothing there wakes anybody or costs a turn. Since
  `docs/decisions/0060-a-binding-is-a-workspace.md` the platform's own
  mention event is what is read, in every room the app has been invited to,
  so a mention is also what makes a person's message *arrive* at all. Worth a
  word of its own because it is the only rule an operator has to hold in
  their head, and the only one whose failure is silence. It is the rule for
  people and not for apps: since
  `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md` another
  app's message in a watched room is read with no mention at all, because an
  app mentions nobody.

- **channel** — somewhere the foreman watches and a job can speak into.
  Two-directional by definition, which is why it is not called a *source* or a
  *feed*: the same Slack that carries a question out carries the answer back.
  Not *integration* or *connector* either — both describe plumbing, and the
  interesting part is that somebody is on the other end. Every project is
  bound to one, with the credential that speaks and the one that listens
  both given, per
  `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`; for
  Slack the binding is one app installed in one workspace, and the app hears
  every room it has been invited to.
- **room** — one Slack channel, as a person sees it in the sidebar. The word
  this project uses because *channel* is taken: `Channel::Slack` names the
  platform, and Slack's own word would make "a job's channel" mean two
  things in one sentence. A job has a room of its own, made with the job
  and archived with it, and a mention anywhere in it is that job's — see
  `docs/decisions/0061-a-job-has-a-room-of-its-own.md`. The foreman has a
  room of its own too, since
  `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`,
  where what it says and does is posted; it is still heard in every room
  the app is in — see
  `docs/decisions/0060-a-binding-is-a-workspace.md`. Not *conversation*,
  refused under **thread** for the same reason. Not *address*, which is
  what a binding used to carry when a project was listened to in one room,
  and which nothing carries now; and not *channel* for a job's room, for
  the reason above.
- **watched room** — a room a person has told the foreman to watch, by
  asking it there. In one, every message another app posts is a signal for
  the foreman; a person's message is read exactly as anywhere else, through
  a mention. Recorded on the project, so a restart watches what it watched,
  and shown on the dashboard by the platform's identifier, because a name
  costs a scope the manifest does not grant — see
  `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`. Not a
  *subscription*, which is the platform's word for what an app receives and
  would suggest this project asks the platform for something: it asks
  nothing, and watching only decides what is read of what arrives anyway.
  Not a *source*, refused under **channel** for describing plumbing. Not an
  *allowlist*, which names the finer design that record rejects and would
  make a room sound like a list of apps.
- **signal** — one observation on a channel: an issue opened, an alert fired, a
  message posted. Signals are read and judged, not stored or addressed. They
  are deliberately not entities; see **reason** below. Since
  `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md` the one
  kind there is has a shape: a message another app posted in a watched room,
  read into text by the channel crate and handed to the foreman as a turn,
  framed as the app's rather than as a person's. It waits in the inbox like
  any message and is gone once the turn ends; nothing keeps a signal after
  it has been judged, which is the sense in which it is still not an entity.
  Since `docs/decisions/0068-a-mention-is-shown-its-thread.md` a
  signal that follows an earlier message is shown that thread too.
- **job** — one agent, in one isolated workspace, on one project, from kickoff
  to completion. A job happens once, and there is no retry: a second attempt is
  a new job with its own workspace. It may, however, outlive the process
  supervising it — the daemon being killed leaves a job's container behind
  rather than ending the job, and startup puts it back to work. Behind and
  *stopped*, unless its tunnel is answering, in which case it is left running
  and whatever it was showing stays reachable — see
  `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`.
  **Resuming is not
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
- **working, idle, retired** — where a job has got to, and the only three
  things this system *does* about one: run a turn in it, leave it alone with
  its container, or reclaim what it was holding. Idle carries a reading of why
  its agent stopped — **asked**, **proposed**, **paused**, **failed**,
  **silent** — and retired carries how it ended — **done**, **discarded**,
  **lost**. The outer word is what code branches on and the inner word is what
  a person acts on; see
  `docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md`.

  Three near-misses worth recording. **Failed is not an ending**: the container
  and its session are still there, so a failed job is waiting for whatever
  broke to be fixed, and only retirement is terminal. **Proposed is a claim,
  not a verdict** — the agent's own account of itself, where **done** is a
  person's judgement of it, which is why they are different words at different
  levels. And **silent** is not a synonym for idle: it is the honest residual
  for a turn that ended without the agent saying which reading applied, and a
  job landing there is a prompt that was not followed. A screen says *idle* for
  it, because that is what a reader of a job list expects and what it has
  always been called there.

  Not *completed*, which claimed the work had ended and was the old name for
  idle. Not *cancelled* for paused, which implies the job is over when the
  whole point is that it can be resumed. Not *archived* for retired, which
  every product a person has used means something reversible by, where this
  removes the container and the session with it.

- **workspace** — the isolated place a job's agent works: the container it runs
  in, with the project's repository checked out in it before the agent's first
  turn, for as long as that job lasts — see
  `docs/decisions/0012-agents-run-in-containers.md` and
  `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
  One job, one workspace: no two ever share one, which is an invariant rather
  than an aspiration (`docs/architecture.md` §2). Not a *checkout*, which names
  the files and misses the boundary that makes them isolated. The checkout has
  been in and out of this definition, and the history is worth keeping: it once
  read "the filesystem the repository is checked out into", from a design in
  which something delivered that repository;
  `docs/decisions/0016-the-agent-clones-the-repository.md` removed every such
  mechanism and had the agent clone it if the work needed one, so that a job
  with no repository was an ordinary case; and 0050 put the checkout back —
  made inside the container by this project, before the agent speaks — because
  a coding agent reads a project's instructions when it starts, and no job
  without a repository ever came. The foreman's agent has no workspace, for the
  plainest reason — a workspace belongs to a job, and triage is not one.
- **thread** — a reply chain under one message in a room: where the foreman
  answers, and where a job answers when it was asked in one. It was where a
  job's whole conversation happened, under an announcement in the project's
  one room, until `docs/decisions/0061-a-job-has-a-room-of-its-own.md` gave
  a job a room instead; what routes a reply now is the room, and a thread
  only says where in it to answer. Not a *conversation*, which is the thing
  that happens in one rather than the place it happens in, and would leave
  nothing to call the identifier. The foreman has none of its own to be
  addressed in: a mention anywhere that is not a job's room is for it, and
  it answers in a thread under the message, or in the one the message was
  already in. See `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`
  and `docs/decisions/0060-a-binding-is-a-workspace.md`.

  Its identifier is opaque to this project and **must stay text**. For Slack it
  is the parent message's timestamp, which looks like a number and is not one:
  parsed as one it loses the microseconds and addresses no message, and the
  failure reads like a permissions problem. It names its room as well,
  because an identifier is only unique within one room and the app hears
  more than one. Since `docs/decisions/0060-a-binding-is-a-workspace.md` a
  person can go on talking to the foreman in the thread it answered in,
  because each message there is its own turn and the session remembers.
  Since `docs/decisions/0068-a-mention-is-shown-its-thread.md`
  a mention in a thread is shown the thread: its parent, and everything
  said there from the last message that was given to this instance, its own
  words among them.

  A job's room is named `<project>--<title>--<8 hex of the job id>`, and
  only the last part is load-bearing: an archived room keeps its name for
  ever on the platform, so the name has to be unique for ever too, and the
  identifier is what makes it so. The rest is for a sidebar.

- **transcript** — everything an agent says and does in a turn, as the
  protocol streams it: its narration, its working, and the notifications
  nothing here posts. Since
  `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
  it is posted as it happens, at the root of the room its speaker owns. Not
  *output*, which is what the instructions called the text nobody saw. Not
  *log*, which is a diagnostic of this project's own, per
  `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md`, and goes
  somewhere else entirely. Not *answer* either, which is the agent crate's
  word for the text alone.
- **narration** — the agent's own text, written to be read: one message per
  contiguous run of it, at the root of its room. Not *answer*, since nothing
  need have been asked; not *message*, which is a person's word on a
  channel; and not *reply*, which is what the tool that speaks does when
  given a target.
- **working** — what the agent does between two runs of narration: its tool
  calls, and its thoughts where an adapter carries any. Posted as a burst.
  Not *transcript*, which is the whole; not *trace*, which is a scenario's
  word for effects; not *log*, refused under **transcript**.
- **burst** — one run of working, posted as one message that grows in place
  until the next narration closes it. Not *batch*, which implies a size
  somebody chose, and not *turn*, which holds many.
- **steering** — how a message reaches a job's agent while a turn runs in
  it: delivered into the running turn at once, pre-empting whatever the
  agent was doing, rather than waiting for the turn to end — see
  `docs/decisions/0069-a-message-reaches-a-working-job.md`. The adapter's
  own word for its extension, kept because the mechanism is somebody else's
  and renaming it would hide which one. Not *interrupt*, which is what a
  person's stop does and ends the turn; not *inject*, which names the wire
  and not what a person sees; and not a second *prompt*, which the adapter
  accepts and which was measured to lose the first prompt's answer.

- **tunnel** — the way in to what a job has put up for somebody to look at:
  one port published from its container when that container is created, and
  the address that reaches it. One per job, always, and never asked for — a
  job has one because it is a job, which is what lets nothing about it be
  stored. See
  `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`.

  The word names the mechanism, and §2 usually rejects one that does — the
  argument against *checkout*. It survives here because the mechanism is
  genuinely the concept: nothing decides that a tunnel exists, nothing decides
  what is on it, and this project never knows whether anything is listening.
  A word implying intent would claim all three. Not *preview*, which is the
  near-miss worth recording: it says something finished is being shown for
  approval, and half of what this is for is watching unfinished work move.
  Not *port forward* either, which is the same mechanism named one layer down,
  where the interesting part — that somebody is at the other end of it — has
  disappeared.

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

- **kit** — one agent, set the way one job runs it: which model, how hard it
  thinks, and whatever else that agent's adapter can be told. A job runs on
  exactly one, fixed when the job is created and settled again at the start of
  every turn — see `docs/decisions/0048-a-job-runs-on-a-kit.md`, which also
  decides that a project names the kits its jobs may run on. In the code its
  tag *is* the agent, so a kit cannot hold settings for an agent other than
  its own; and what the adapter reports back after being set is kept on the
  job beside the kit rather than derived from it, because the two were
  measured to differ.

  Not *profile* or *preset*, both of which name a saved form rather than the
  thing a job actually runs under. Not *assignment*, which names the act of a
  job receiving one and would leave nothing to call the thing received. Not
  *configuration* or *settings* either — and note that the objection recorded
  under **variable**, that those words imply something here reads one, does
  not apply, which is exactly the difference between the two concepts: a kit is
  read, by the adapter, on every turn. What rules those words out is that they
  name a bag of values, and a kit is a decision about one job.

- **brief** — free text an operator writes for a project's foreman, said to
  it on every turn beside the kits: the operator's standing instructions,
  and the one place policy lives — which alerts to ignore, what a filed
  issue deserves, which account a job acts as. Nothing here enforces one;
  the foreman reads it. See `docs/decisions/0064-a-project-has-a-brief.md`.
  Not *instructions*, which is a job's word for the one thing it is told to
  do, where a brief is standing and about no task in particular. Not a
  *system prompt*, which names a mechanism this is not: it is said with each
  message rather than fixed at the session's start, because a session
  outlives every edit to it. Not *policy* either, which reads as enforced,
  and the near-miss worth recording: a brief is followed by judgement, and
  a person correcting the foreman in a room is still a message.

- **variable** — one name and one value an operator gives a project, set in the
  environment of every container that project's jobs run in. What makes it a
  concept of its own rather than a loose platform credential is that **this
  project never reads it**: nothing here parses the value, infers anything from
  the name, or needs code in order to support one — which is precisely what a
  platform and a channel do need, and why both of those are closed sets. See
  `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.

  The word names the mechanism, which this section usually rejects — the
  argument against *checkout*. It survives for the reason **tunnel** does, and
  for one more that is particular to it: an environment variable genuinely is
  the concept here, because the operator is the one choosing the mechanism.
  Everything else in a handout is delivered however its adapter sees fit — a
  variable for one agent, a file at a path for another — and this is the only
  thing a handout carries whose delivery is the operator's decision rather than
  the adapter's.

  The near-miss to record is inside this repository rather than beside it. The
  adapter has a wider set under the same word: everything a container is started
  with, including the credentials this project decides on its own. A project's
  variables are the subset it knows nothing about, and the two must not be
  allowed to blur. Not *setting* or *configuration*, both of which imply
  something here reads one.

- **handout** — exactly what one agent process is allowed to see: its agent's
  own credential, and — of the one project it works for — that project's
  platform credentials, its variables, and the *speaking* half of its channel
  bindings. Half,
  because a binding holds a second credential that opens an event stream, and
  a job has no use for one: the handout carries a narrower type with nowhere
  to put it, so that is a property rather than a rule somebody applies. Nothing else, and nothing
  inherited. The three project parts are not interchangeable and a handout can
  carry one without the others: a foreman's gets the channels and neither a
  platform credential nor a variable, because watching a channel is its remit
  and acting on anything else is not. See
  `docs/decisions/0027-a-channel-is-not-a-platform.md`, and
  `docs/decisions/0046-a-projects-variables-are-carried-never-read.md` for the
  third. It is *decided* in the domain crate as a
  pure function and *delivered* by an adapter, because which secrets a process
  may see is a question about configuration while what they are called is
  knowledge about one agent. Not *environment*, and that near-miss is the whole
  reason this word exists: an environment names a delivery mechanism, and
  delivery is precisely the half that differs — a variable for one agent, a file
  at an expected path for another — so a word presuming variables would make the
  wrong half sound settled. Not *credentials* either, which is a bag of secrets
  rather than a decision about one process, and loses the part that matters:
  a handout is scoped to somebody, and the scoping is the point.

- **instance** — everything one running stageman knows and every decision it
  makes, as one synchronous value: the projects and their jobs, the foremen's
  inboxes, what is in flight, and the rule for what to do about each thing
  that happens. It is stepped one event at a time and answers with effects,
  and it can neither read a clock nor perform an effect of its own — see
  `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`. What
  it *keeps* goes to the disk; what it merely *holds* — turns in flight,
  warrants, a tunnel's port — a restart begins without. The word was already
  in use for a running stageman and its file, and the two are one thing seen
  from outside and from inside: what an operator installs is a daemon, and
  what the daemon runs is the instance. Not *model*, the first name proposed
  and the near-miss worth recording: a kit already has a model, and "the
  model decided to start a job" is ambiguous in exactly the place this
  section exists to keep clear. Not *core* either, which is a crate.

- **world** — everything the instance is not: the container runtime and the
  agents inside it, the channels, the clock, randomness, the disk, the
  servers, and the async runtime that drives them all. There are two. The
  real one is the app crate; the simulated one is what a scenario steps, and
  it is where a fault is injected and a crash is chosen. A world routes by
  the shape of a request and never decides on the instance's state, and it
  answers an effect only once the effect has actually completed. Not
  *simulator*, which is the literature's word and names only the second;
  not *environment*, refused under **handout** for naming a delivery
  mechanism; not *adapter*, which is one part of a world — the piece that
  speaks one outside system's protocol — and not the whole.

- **event** — one thing the world tells the instance: a message heard, a
  turn ended, a request arrived, a timer gone off, bytes landed on the disk.
  Plain data, and the only way anything reaches the instance at all. An
  event never carries a time: the world tells the instance the time with
  every **step**, stamped as it hands the event over, and a fact's own time
  — a platform's timestamp — is data inside the payload; see
  `docs/decisions/0073-the-world-tells-the-instance-the-time-with-every-step.md`.
  Not *message*, which is what a person says on a channel, and not
  *command*, which would suggest the world tells the instance what to do
  rather than what happened.

- **effect** — one thing the instance asks of the world: run a turn, stop a
  container, post a message, write these bytes, answer this request, wake me
  later. A value and never a call, so that a scenario's effects are a trace
  that can be compared and the instance can learn nothing from making one.
  An effect is *answered* by an event only where the instance's next decision
  depends on the outcome; the rest are unanswered, and a failure in one of
  them is the world's to log. Whatever faces outward waits for the persist of
  the step that justified it. Not *action*, which reads as done rather than
  asked for, and not *side effect*, which is what this design exists to have
  none of inside the instance.

- **scenario** — a scripted world: the file an instance starts from and the
  sequence of events it is fed, run against the simulated world, whose trace
  of effects and state after each step are snapshots. Two kinds, and telling
  them apart matters. A **replay** is a file of events in and effects out,
  compared exactly, in which nothing behaves; an **exploration** is a
  **seed**, a random world under the same simulation, answering effects with
  faults and crashes while an invariant is checked after every step. Replay
  pins and exploration finds, and a seed found failing is committed as a
  replay, so that the bug costs one run to rule out for ever — see
  `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
  Not *test case*, which does not say that the world is simulated.

- **recorder** — what runs the real container runtime and the real agent, and
  writes down what they print and how they answer, as the fixtures the
  in-memory tests replay. Not a *test*: what it checks is what the outside
  does, not what this project does, and it runs by hand and when a pin
  changes rather than in the gate. Not an *example* either, which says how to
  use something rather than what something else is like.

- **chip** — one thing shown inline and compact, and usually a link: a kit
  on a project's row, a pull request on a job's. A chip says which and
  where, and carries no verdict. Not a *badge*, which carries one word of
  state and never links; not a *tag* or a *label*, which are what a platform
  calls its own; not a *pill*, which names the shape rather than the thing.

- **mark** — somebody's own symbol, drawn inline at icon size: an agent's,
  one per agent this build can run, keyed by the identifier the wire uses
  for it, with a generic one for an identifier this build does not know;
  and since the job page, a platform's and a channel's, for the links a
  page makes to them, keyed the same way. A mark says *whose*; an icon says
  *what*. Not a *logo*, which is a brand's full lockup and belongs to nobody
  here; not an *icon*, which is reserved for the one set §3 names.

- **tick** — what an open page is told when a write of the instance's file
  has landed: that something may have changed, and nothing else — see
  `docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`. Not an
  *event*, which is what the world tells the instance; not a *notification*,
  which is what a channel posts to a person; not an *update*, which would
  claim to carry the change.

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

  Since `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`
  the instance *asks* for that write rather than making it — an effect,
  answered when the bytes have landed — and nothing outward-facing leaves the
  step until they have. The write is still on every change; what moved is who
  performs it, and where the guarantee that a client was told the truth lives.
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

  **A project's variables can reach that same failure from the other side, and
  are refused for it.** An operator naming one `ANTHROPIC_API_KEY` would change
  who pays exactly as an inherited variable would, so a name this project
  already delivers is rejected when it is entered. Which names those are is the
  adapter's knowledge and not the domain's — the sentence above is why — so the
  question is put to the adapter and asked by **app**, which is the only crate
  allowed to see both halves. See
  `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.
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

  Since 0056 this is the shape of the program rather than a rule anybody
  keeps: the instance's step is synchronous and short, everything that takes
  time is an effect the world performs on a task of its own, and a request is
  an event the loop answers between two others.

  **And the server is behind a door rather than at it.** Since
  `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
  the address a person types is taken by the instance, which decides on every
  request's host and forwards: a job's name to that job's container, and
  everything else to the framework, on a loopback port the kernel chose. So
  the app crate is still an Axum server and is no longer *the* listener, and
  a server function is the one thing that still asks the instance directly —
  everything else arrives as a request the instance answers itself.
- **The app's `server` feature is a contract with the framework, not a name.**
  The server-function macro emits `#[cfg(feature = "server")]` literally, so a
  feature spelled anything else silently moves every server function's body to
  the client. Everything the daemon needs is an optional dependency behind it —
  including the internal crates — because a `cfg` hides code from the
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
  would silently stop covering half the application. The line excludes the
  internal crates by name; a crate added later fails there until somebody says
  which side of the split it is on, which is the right default.
- **Two things `dx` produces are corrected rather than accepted, and a `dx`
  upgrade is the moment to re-check both.** Neither is a broken build. Both are
  a line in the console of a page that otherwise works perfectly — which is
  exactly why they are written down here, because nothing else would ever bring
  anybody back to them.

  **The index is not byte-for-byte what the bundler wrote.** It preloads the
  browser's half with `rel="preload" as="script"` and then loads that same file
  with `<script type="module">`; a browser fetches a module in its own mode, so
  the preloaded copy matches nothing and is thrown away — sixty kilobytes
  fetched twice, and Firefox saying so. One literal is rewritten as the index
  is written out, and nothing else is touched.

  **A release's wasm carries no name section**, because Firefox reads one that
  is present and *empty* by running off the end of it into the next custom
  section and reporting the module as validated with a warning. Emptying it is
  the bindgen step's doing and preserving the husk is the optimizer's, so the
  fix is `--debug-symbols false` in the build recipe. Not
  `[web.wasm_opt] debug` in `app/Dioxus.toml`, which is the obvious place and
  does nothing: the flag overwrites that field rather than reading it.

  Both fail in the safe direction, which is the whole argument for doing either
  to somebody else's output. A substitution that stops matching leaves a page
  that works and a console line that came back; a flag that stops being
  accepted stops the build and says which one.
- **Typed errors per crate, and no `anyhow` in any crate but app.** That is
  the gate's bar restated only where it bites: **app** is a binary and may do
  as it likes internally, but the others are libraries whose errors cross a
  boundary, and a boxed error at that boundary makes the caller's handling
  untestable.
- **The agent is third-party, and its quirks stop at the agent crate's
  boundary.** What a container is started with, what is said to the agent and
  what its answers mean are entirely the **agent** crate's business, rendered
  and read there as pure functions; the **instance** only sequences them, and
  the world only carries them. If a change to that agent's interface would
  touch **core** or the instance, the abstraction is in the wrong place — the
  whole reason the crate boundary is there is that the agent is on somebody
  else's release cadence. The same holds for a channel, for the same reason:
  what is sent to a platform and what its answers mean are the **channel**
  crate's, per
  `docs/decisions/0058-a-channels-adapter-is-a-crate-beside-the-agents.md`.
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

- **Nothing posted is written in the platform's own markup.** Since
  `docs/decisions/0062-what-this-instance-says-is-markdown.md` the adapter
  posts everything as Markdown through the platform's own Markdown parameter,
  and agents are told to write Markdown. The platform's dialect — single
  asterisks for bold, angle brackets for links — looks close enough to pass
  review and renders wrong, which is why it is worth a rule: a text that
  needs a link or a mention gets it from the channel crate, which is the
  one place that spells the platform's references.
- **One Slack app per project is a must, and its manifest must subscribe to
  `app_mention`.** Measured, both: Socket Mode hands each event to *one* of an
  app's open connections, so two projects sharing an app-level token each
  hear half of what is said, with nothing anywhere saying so; and since
  `docs/decisions/0060-a-binding-is-a-workspace.md` a person is read from the
  platform's own mention event and from nothing else, so an app whose
  manifest lacks that subscription connects, greets, and hears nobody. The
  same message also arrives on the message subscription, and that copy is
  acknowledged and dropped rather than read twice. `README.md` carries the
  manifest so that neither has to be remembered.

- **Dependency versions live in `Cargo.toml` and are not restated here.** They
  are derivable, they go stale, and `just drift` cannot catch a version number
  in prose. Gotchas belong here; numbers do not.

- **Nothing inside the instance performs an effect, reads a clock, or draws on
  entropy of its own.** No I/O, no async, no threads, no locks, and every map
  ordered. Time arrives with every step, from the world; randomness comes
  from a generator the world seeded at construction; the outside is reached by
  returning an effect and heard from by being handed an event. The reason is
  that determinism is then a property of the type rather than of anybody's
  discipline: a scenario reproduces because there is nothing in the instance
  that could make it otherwise. The manifest is where it is enforced — the
  instance crate names no async runtime and nothing that opens a socket or a
  file — and the review question for any change there is whether a line could
  answer differently on another run. See
  `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`.

  **On another machine, too, and that is the part most easily missed.** A
  compile-time condition inside the instance is as much a hidden input as a
  clock: it makes a start a function of the machine the binary was made for,
  and a flow recorded on one then fails to replay on another. So which
  platform this build is for arrives at construction beside the seed and the
  environment, every platform's lists and rules are held for all of them at
  once, and the branch is on the value. The one compile-time condition left
  is in the entry point, choosing which value to hand over. A `cfg` or a
  `cfg!` anywhere in the instance crate is the smell this rule exists to
  catch; see
  `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.

- **The world knows no domain, and answers an effect only when it has
  completed.** It performs mechanisms — a file, a process, a request, a
  socket, a port, a timer — and hands the application's own effects to
  whatever the entry point supplied. Which host a request is for, which
  command a runtime is given and what its output means are all the
  instance's; an `if` on domain data in the world is a smell to move, and so
  is a string the world composes for anything but a log line. The completion
  rule is the whole of what the durability guarantee rests on: the instance
  holds back a client's answer and a turn's start until the world says the
  write landed, so a world that answered a persist when the write was merely
  started would make every one of those promises false. See
  `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.

- **A panic ends the process.** The release profile aborts rather than
  unwinds, per `docs/decisions/0065-a-panic-aborts-the-daemon.md`, so nothing
  here may rely on catching one: a task's join error saying it panicked, a
  poisoned lock, anything wrapped to catch an unwind — each is a branch the
  shipped binary never takes and a development build does, which is exactly
  the divergence a rule is for. The reason is what a caught panic looked like
  before: the instance steps on a task nobody joins, so a panic there left a
  daemon that accepted every connection and answered none, and a page waiting
  on it waited for ever. An abort is the kill this daemon already survives,
  under the bar §4 sets for one, and a service manager restarts it.

- **A room is posted to one request at a time, and a burst grows by
  editing.** Two posts in flight to one room land in whichever order the
  platform receives them, so the next is sent when the previous is answered,
  and the order of a transcript is a property of the chain rather than of
  luck. A post per line looks simplest and is a few hundred posts per heavy
  turn against a budget of about one a second per room, so a burst is one
  message edited as it grows, paced by a timer — see
  `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
  Nothing posted this way is load-bearing: a failure is logged, and the
  tool's answer is what guarantees a person sees what they must.

- **Never a second prompt while one is open.** The adapter accepts one and
  queues it, and the first prompt's answer is then lost without a word: it
  was measured resolving with no output at all while the model answered only
  the second. A prompt starts a turn and nothing else. A message for a job
  that is working is delivered by steering when its conversation is open and
  waits in the job's inbox otherwise — see
  `docs/decisions/0069-a-message-reaches-a-working-job.md`.

- **One icon set, one icon per concept, decided in one module.** Every icon
  is lucide's. The text glyphs the dashboard once used for add, save, close
  and edit are refused: a glyph arrives in colour on the platforms that give
  it an emoji presentation, and two vocabularies on one page read as an
  accident. Which icon means which concept is decided once, in one module of
  the app crate, so a foreman is the same shape on every page. The crate
  compiles icons by category, so an icon that will not resolve usually means
  a category to add rather than an icon that does not exist, and the cost of a
  category is compile time rather than bundle size, since a browser build
  strips what nothing draws. An agent, a platform and a channel are shown by
  their marks, per §2, vendored as inline drawings rather than fetched from
  anywhere: a brand's symbol is not the icon set's to draw, and a link that
  leaves the page says whose it goes to at a glance, where a word would take
  a glance and a half. A reference that leaves the page is a mark or an
  icon with the address a hover away, never the address written out — an
  address is read character by character and a row of them is a wall.

- **A closed set is a control, never a dropdown.** Every set a form here
  chooses from has a handful of members — an agent, a model, an effort, a
  kit — and a set that small is a segmented control, or a row of cards with
  their descriptions, in the page, where every option is seen at once and
  nothing floats over anything. The browser's own select was what the form
  used and is refused for two reasons: it draws itself, outside the theme,
  and it hides the options a choice is made between. A listbox that floats is
  for a set that is open or long; none exists here, and when one does it is
  lifted from the components the framework's own registry publishes rather
  than depended on, because the crate published under that name was a
  placeholder when this was written.

- **A theme is a token block, and a component never names one.** Since
  `docs/decisions/0072-the-dashboard-has-a-dark-theme.md` there are two,
  under one class on the root, applied before paint by the one script the
  dashboard writes by hand. A component that seems to need a dark variant is
  one whose colour has no token yet, and the token is the fix. The three
  state colours are chosen per theme rather than tinted from one.

- **A script the page evaluates sends its answer and never returns it.** The
  framework wraps what it evaluates in a function that closes the script's
  channel after it, so a `return` at the top level skips the close: the
  channel leaks, and Firefox reports code after a return on every page that
  did it. The answer goes through the channel the wrapper hands the script,
  and the Rust side receives it, which is the same length and reads the same.

- **Motion is a transition, honours the reduced-motion preference, and needs
  no library.** What moves: hover and focus, a panel or a popover entering
  and leaving, the mark that says working, and a row arriving once a page is
  live. Nothing moves for its own sake, and everything that moves is still
  under the preference, because a person who asked for stillness asked for
  it everywhere.

- **A page is live through a tick, never through polling.** The shell opens
  one stream, and every page restarts its own read on a tick — see
  `docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`. A page that
  read on a timer would read while nothing changes and still be stale between
  reads. The mark in the shell says whether the page is live, and a page
  without it behind a proxy is a proxy that buffers.

- **A time is shown relative, exact on hover, and drawn after the page
  wakes.** The server renders the page and the browser hydrates it, and a
  relative time computed twice from two clocks is two different strings,
  which the framework reports as a mismatch. So the exact time is what
  arrives, and the relative one is drawn in the browser.

- **One visible line per field, and the rest behind a control.** A form's
  copy is the highest-leverage text an operator reads and the easiest to
  stop reading: a paragraph under every field is six paragraphs, and a
  person skips all six. A field gets a label that is a noun, a placeholder
  that is an example rather than an instruction, and one line saying what it
  is for; anything longer is said by an info control beside the label, to
  whoever hovers or focuses it.

  **The line goes under the label, whatever the field holds.** Label, line,
  control, in that order, so that every field reads as a titled thing with
  its line as the subtitle; a line under the control reads as belonging to
  whatever is above it, which for a list is its last row. A problem takes
  the line's place rather than adding to it, so a field that is wrong says
  one thing, where the eye already looks. What adds to a list sits at the
  end of the label's line, where an action belongs, rather than under the
  list, where it moves as the list grows and takes a row for one small
  control. A control beside a box is the box's height and square, and a
  list inside a section is rows parted by a hairline rather than boxes
  within the box: a shorter control reads as a misalignment, and a nested
  box pads its rows in from the edge every other control on the page sits
  at.

- **A tooltip, and what the info control beside a label says, show on
  hover and on a focus that came from the keyboard, never for a click.** A
  control that is clicked keeps its focus, so a tooltip shown for focus
  stays until the next click lands somewhere that takes it, and reads as
  stuck. Shown for `focus-visible` instead, it appears for a person on a
  keyboard, who cannot hover, and for nobody else. The info control was a
  disclosure — opened by a click, closed by nothing but a second click, so
  that several stood open at once and each stood for ever — and is a
  tooltip now, with two differences a paragraph earns over a repeated
  name: it is the control's description to assistive technology, because
  it says what the label does not; and Escape dismisses it while the
  control has focus, until the pointer leaves or focus moves on, which is
  the one thing the browser's own states cannot do and the only script in
  either. One difference it does not earn, and the near-miss worth
  recording: neither holds still for a pointer that moves onto the text.
  It was tried, because a paragraph is read rather than glanced at, and it
  put the text between the pointer and whatever is under the label — the
  next label's own control was unreachable beneath it. So the text lets
  the pointer through, and a person who wants it to hold has the keyboard,
  where focus keeps it until Escape or the next Tab. On a touch screen a
  tap stands in for hover and a tap elsewhere for leaving, which is the
  browser's own doing, and nothing said this way is essential on a screen
  without a pointer, because the line under the label carries what
  matters. Not a disclosure, which is for content that stays; not a
  popover, which is for something a person acts in.

- **The shell's header and status line stay in view, and so does a page's
  own header, through one component.** The settings page is longer than a
  screen, and what a person reaches for after editing the bottom of it is
  Save, which
  `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` put
  at the top; a control that has to be scrolled to is a control somebody
  scrolls past. A job's page is longer than a screen too, and what its
  header says — which job, and where it is talking and showing — is worth
  keeping in view over the instruction. The navigation, the live mark and
  the machine are worth seeing from wherever a person has scrolled to,
  which was the point of making the machine a line rather than a page. A
  page that has a header of its own gives it to the one component that
  stays in view, so that no page sticks while another does not. Whatever
  stays in view is opaque and above what scrolls under it, so a tooltip or
  a popover passes beneath, and below the modal, which is the one thing
  that covers a page.

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
- **Killing stageman leaves nothing untracked, and nothing running that is not
  holding a live tunnel.** Hard-killing
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

  `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
  narrowed it a second time, in the same direction and further. A container
  showing something a person can reach is left *running*, and on a hard kill
  that is not a choice: nothing of this project's runs on a kill, so there is no
  shutdown path in which it could be otherwise. What the test has to tell apart
  is now three things rather than two — a container held open because its tunnel
  answers, one retained stopped because its job can be resumed, and one nothing
  can name, which is the only leak. Count anything and it passes on the last.

  **Something now removes containers, and the bar is what says which ones.**
  `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md` gives an
  operator a way to end a job, and the sweep a way to finish one that was
  interrupted. What goes is a container belonging to a job whose verdict has
  just been written, or to a project being forgotten. A test that counted what
  was left behind would now pass on all three of the cases above and on this
  one too, which is the same reason it could never have been a count.

  **And the sweep may now remove what it cannot place, but only its own.**
  `docs/decisions/0054-a-container-says-which-instance-started-it.md` puts the
  creating instance's identity on every container, because two instances
  sharing one runtime is the ordinary arrangement here — `just dev` serves one
  out of the checkout beside the real one. So the bar has a third thing to tell
  apart, and it is the one worth testing against a live runtime rather than by
  construction: a container this instance started, a container another instance
  started, and a container from before either could say. Only the first may be
  removed, and a test that cannot tell the second from the first is a test that
  passes while a development instance destroys the real one's work.

  **Whatever asks whether a tunnel answers must ask it of a *published* port.**
  A port this project binds itself has nobody answering for it; a published one
  has the runtime's proxy in front, which accepts on the container's behalf
  whether or not anything is inside. So a test that binds its own socket proves
  the probe can tell a listener from silence and nothing about the case that
  decides a container's life — and while that was the only test, every
  container ran for ever with the bar above reading as satisfied. See
  `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`.
  This is why the container tests earn their minutes: the gap was not in the
  reasoning, it was in what the cheap test could reach.

  **A foreman's container is under the same rule, with the tunnel half
  vacuous.** Nothing is ever told where a foreman's port is, so its container
  runs while a turn runs in it and is stopped when its inbox empties, at the
  same three moments — see
  `docs/decisions/0066-a-foremans-container-runs-only-while-a-turn-runs-in-it.md`,
  which exists because for a while nothing stopped one at all, and every
  foreman's container ran for ever with this bar reading as satisfied.

  Since `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`
  the running half of this is an invariant the simulated world checks after
  every step of every scenario, once the instance is awake and has swept: no
  container of this instance's that it has listed or made is running with
  nothing in it — no turn in flight for it, no message waiting for its
  foreman or for it, nothing answering on its tunnel, and no question about
  it in flight; and, since
  `docs/decisions/0069-a-message-reaches-a-working-job.md`, no job that is
  not working holds a message. The other half, that nothing is left the
  instance cannot name, is the waking sweep's, pinned by its scenario rather
  than by the oracle. The container tests are what tie the simulated runtime
  to the real one, and both are needed: the simulation reaches the crash
  between two steps that no container test can, and the container test
  reaches the proxy that no simulation would have imagined.
- **What a snapshot must still open is what the last release wrote, and
  nothing older.** Compatibility is a window of one tag, not a growing pile:
  when a released version exists, a schema change carries a bridge from *that*
  version's shape and drops every shim older than it in the same commit. The
  reasoning is that a shim's value decays to nothing while its cost does not —
  each one is a branch nothing exercises, a test pinning a file no clone can
  produce, and a second shape the type has to keep meaning. Five such shims
  were dropped in the change that added this rule, and every one of them
  described a release nobody was running.

  **The defaults stay even where the last release writes the field.** They cost
  nothing, they cannot produce a wrong value — absent genuinely means none —
  and they are the pattern the next added field follows. What goes is the
  *renaming* and *reshaping* compatibility, which is where the cost is.

  This project is not stable and has no outside consumers, so the window is a
  choice rather than an obligation. Widening it later is free; what would not
  be free is discovering that a shim nobody could test had rotted.

  **What must survive that window is the instance, not every job in it.** The
  configured agents and the projects, with their credentials and bindings,
  must open on the release after the one that wrote them, because losing
  them is losing everything an operator typed. A job, a foreman's inbox, or
  the half of a binding that cannot be carried honestly may be dropped on
  opening, with a line at startup saying so, when carrying it would mean
  inventing a value for it — a job mid-flight across an upgrade is a job the
  operator can retire and restart, and a message in hand is one they can
  send again. Never the other way round: a file must not fail to open over
  a job.

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

- **What this project can spell, the pinned adapter must accept.** A kit's
  values are variants in the domain and spellings in the adapter, and the
  adapter's version is pinned in the image compiled into the binary — so the
  set is a fact about this build, and a container test settles every kit the
  domain can spell on one real session and fails on the pin bump that removes
  or renames a value. It needs no credential and no network, because a session
  opens with neither, which is why it sits with the handshake tests rather
  than with the ones that cost a credential. What it cannot see is an *added*
  value, which passes silently and is a feature to add rather than a defect.
  See `docs/decisions/0048-a-job-runs-on-a-kit.md`, and note that the same
  record's read-back cannot be exact either: an account's entitlements change
  how a value is spelled in a reply, so what is checked is that the reading
  moved.
- **Every text this system posts is snapshot-tested, and it is Markdown.**
  The prompt text the foreman composes, and every notice this instance says
  on its own behalf, is asserted as literal text, so a change to what a job
  is told to do or what a person is told shows up as a reviewable diff.
  Prompt text is the highest-leverage code here and the only kind that
  changes behaviour without changing control flow, so it is also the only
  kind that can be rewritten completely without a single test going red.
  Since `docs/decisions/0062-what-this-instance-says-is-markdown.md` every
  such text is Markdown and goes out as Markdown: a notice written in the
  platform's own markup would render its asterisks literally, and a
  translator between the two is exactly what that record refuses. Since
  `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
  an agent's own words are posted as they come, and they are not this
  project's to assert; what is asserted whole is every text composed around
  them — the notices, the framing, the lines a burst is built from.

- **Sequencing is tested by scenarios and seeds, and their snapshots are
  reviewed as behaviour.** Every flow that crosses an await today — a turn
  ending, a sweep, a reply arriving while a turn runs, the daemon dying
  part-way — is a scenario against the simulated world, and its trace of
  effects and state after each step are snapshots whose diff somebody reads
  before it merges. A seed found failing is committed as a replay. One fixed
  seed runs in `just verify`, so the harness cannot rot; exploration proper
  runs outside the gate, on its own trigger. Every event and effect is a
  value: it clones, it compares, and it serialises in full, and it formats
  not at all — the generic vocabulary implements neither `Debug` nor
  `Display`, so a credential inside one can reach a file a test wrote, where
  every credential is fake, and never a log. A channel or a callback added
  to a variant would end the comparison that makes any of this work.

## 5. What this project needs installed

Beyond the Rust toolchain and `just` that `AGENTS.md` names for every project
built from this gate, this one needs:

- **A container runtime** — Docker or Podman. Every agent runs inside a
  container, including the one the foreman thinks with, so nothing here
  runs an agent without one. See
  `docs/decisions/0012-agents-run-in-containers.md`.
  There is deliberately nothing here about building an image. The fragments a
  recipe is composed from are compiled into the binary and built on demand, per
  `docs/decisions/0035-an-image-is-built-never-named.md` and
  `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`, so
  the only thing to install is the runtime that builds them. Images this
  project builds are named for the recipe they came from and shared by every
  container built from it; ones left by a version before that carry no name at
  all, so they are dangling images and `docker image prune` is what reclaims
  them. That is a command to run rather than one this project runs, because it
  would take every other dangling image on the machine with it. The container tests still skip under
  `just check` rather than failing — the first of them costs minutes and a
  network — so a green gate on a machine that has never built one is not
  evidence the containers work; `just image-handshake` is.
- **`dx`, the Dioxus CLI** — `cargo install dioxus-cli`, needed to build the
  browser half into a bundle. `just dev` serves both halves with reloading and
  `just build` produces the single file that ships — the browser's half is
  compiled into it, per
  `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`, which is why
  that recipe builds twice; `just check` needs neither them
  nor a bundle, because the wasm pass is `cargo` against a target and that is a
  toolchain fact.

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

**The runtime is needed for `just check`; `dx` is not.** That split used to be
simpler — nothing beyond a toolchain — and
`docs/decisions/0023-the-container-runtime-is-discovered-once.md` gave up the
runtime half deliberately, on the grounds that something required in production
and optional in the tests is a difference that gets discovered late. Seven
integration tests run the binary, and the binary refuses to start without a
runtime, so there is no version of this where the gate passes on a machine that
could not run the program.

Building an image stays out of the gate, because a present runtime is not a
built image: the first build costs minutes and a network, so the tests that
need one belong to `just verify` — the bar for pushing — rather than to the
gate you run constantly. What changed with 0035 is only who builds it. Nobody
runs a recipe by hand any more; the tests build what they need from the recipe
compiled into the crate they are testing, which is the same code the daemon
runs.

**Three variables are read at build time rather than at run time**, and the
distinction matters more than it looks: everything else spelled `STAGEMAN_*` is
configuration a running daemon reads, while `STAGEMAN_BUILD_VERSION`,
`STAGEMAN_BUILD_COMMIT` and `STAGEMAN_BUILD_DATE` are implanted into a binary
when it is compiled and mean nothing to one that is already running. The word
`BUILD` is in the name for that reason. Setting the first is what makes a build
a release; the build script refuses if the other two are then missing, because
a release that cannot say where it came from is broken rather than partial. See
`docs/decisions/0039-a-release-is-a-tagged-binary.md`, and note that
`just release` sets all three from a tag and from git, so nobody sets them by
hand.

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
nothing asks you for it: `just dev` generates one on first use, to
encrypt the development instance it serves. Losing it costs a file with nothing
in it, and an instance that will not open is repaired by deleting both.

That recipe generates its own rather than letting the binary do it, and the
difference is the point: since
`docs/decisions/0037-the-instance-key-is-generated-on-first-run.md` a binary
with no key generates one under the platform's configuration directory, and a
development instance served out of a checkout must no more write there than it
may write to the real instance file. Both overrides are set for the same
reason, in the same place.

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
