# 0057 — The world is generic, and the instance boots itself

## Status

Accepted. Amends
`docs/decisions/0056-the-instance-decides-and-the-world-performs.md` in three
places and leaves the rest standing. The events and effects the instance and
the world exchange are no longer derived from this project's mechanisms; they
are a small application-agnostic vocabulary, and the world that performs them
is a crate that names nothing of the domain. The instance is no longer
constructed from its file and the runtime's facts; it is constructed from a
seed and the environment, and learns everything else by asking. And the
container tests no longer stay as the tests of the world: they become a
recorder, and the gate runs entirely in memory. That record's rejection of
forwarding every request to the instance as raw HTTP narrows rather than
reverses: the head of every request now reaches the instance, and pages still
reach the framework, through a proxy rather than a handler.

## Context

0056 moved every decision into the instance and left the world with three
kinds of code, only one of which had to be there. Plumbing with no decision in
it: the async runtime, the router assembly, the listeners, a process pump, an
HTTP client, a websocket client, file writes, timers, and the identifier round
trip that answers a request. Translation that knows a protocol: the agent
adapter composes every container command, parses every output, and holds the
whole conversation with the agent over the container's standard streams, and
the channel listener runs a connection's lifecycle — connect, acknowledge,
refresh before the platform closes, retry, measure the deaf window. And
startup: discovering and verifying the runtime, the key, reading the file,
the sweep's inputs, the first write, and the ordering between them, which is
where two of the switch-over's own bugs were.

Of the fifty functions still skipped by mutation testing after 0056,
forty-five are in the second and third kinds. They are reachable only by
starting a container or a daemon, which is the test nobody runs on every
commit. The aim of this record is a world so small that it needs no test at
all, because it is obviously correct from reading it — and everything that
decides, translates or sequences inside the instance, where a scenario reaches
it in memory.

Two kinds of scenario had been called by one name, and telling them apart is
what makes the rest of this record consistent. A **replay** is a file of
events in and effects out, compared exactly: nothing behaves, nothing
interprets, and any difference in effects or state is a change of behaviour.
An **exploration** is a seed run against a world that answers effects, with
random ordering, faults and crashes, and an invariant checked after every
step. Replay pins; exploration finds. Only the second needs an answering
world, and the harness 0056 built is that world; it is also what records a
replay in the first place.

## Decision

**Two crates, and the world names nothing of the domain.** A vocabulary crate
holds the events and effects, generic over an application hole that carries
whatever the application adds, and names nothing but serialisation. A world
crate performs the vocabulary on the async runtime and is the only crate that
names it; the application's entry point hands it what to perform for the
application's own variants and nothing else. The instance names the
vocabulary and never the world. Both crates are internal to this workspace
and unpublished; the world's reusability is a consequence rather than a goal,
and if it is ever extracted its renaming is a record of its own.

**The vocabulary is mechanisms, not meanings.** Its families are the ways a
process reaches the outside, each answered where an answer exists:

- A file, read — answered with its contents, or that it is absent, or that it
  could not be read, and absent is its own answer because a first run is not
  a failure — and written atomically, with one property the instance may ask
  for: readable by this user only. There is no effect for existence; a read
  that says absent and a spawn that fails cover every use.
- A process run once: a program, its arguments, the whole environment it is
  given, optional bytes on its standard input — answered with the status, the
  output and the complaints, and a spawn that failed distinctly, which is how
  a runtime candidate is found to be missing and how the same command verifies
  it. The program is its own field, so a run with no program is not
  representable and a run with no arguments is.
- A process kept open: opened, its lines arriving as events, lines sent to it
  as effects, closed, and its exit as an event. The conversation with an agent
  runs over this.
- An HTTP request, once. A websocket: connected, frames each way, closed.
- A port probed, with the read-once meaning
  `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
  measured; twenty lines, and not reducible to the others.
- A wake after a duration. A bind of an address, answered with the port it
  got, so that a port of zero — whichever is free, which the tests rely on —
  is learned rather than assumed. A print to standard output. An exit with a
  message, which is how a start refuses.
- For every request that arrives on a bound listener, its head, its peer and
  the time as an event, answered with one of: proxy it to a port, read its
  body and hand that back, or respond now. The body arrives as an event only
  when asked for, and bounded.

**The instance boots itself.** It is constructed from three things that exist
before anything happens: a seed, drawn once by the world from the operating
system; the environment as a map; and which platform this build was made for.
Everything else it asks for: the key,
its own file, each runtime candidate in order, what containers were left
behind, the listeners it wants. The presentation server's port — see below —
arrives as the application's own event, first of all, because it is an
application fact and the map is what the process was actually given.
Everything a start used to refuse with an exit code is an exit effect with
its reason, and everything a start used to order by care — the address line
last, after the first write has landed — is a sequence a replay pins.

**The platform is handed over rather than read**, and it is the third
construction fact for a reason the other rejected ones are not: it is a
property of the binary rather than an answer somebody had to go and find.
Where a container runtime might be and where a program's own files go are
both decided per platform, and reading either from a compile-time condition
would make a start a function of the machine it was compiled for — so a flow
recorded on one would fail to replay on another, which is where the gate
runs. Every list and every rule stays inside, for every platform at once, and
the value says which applies. One compile-time condition remains, in the
entry point, choosing the value to hand over. Two things follow that are
worth having on their own: what a start does on a platform is testable from
any other, which is exactly the branch that shipped wrong once; and
discovering a runtime by looking for the file is gone, because a candidate is
tried by being run.

The build's own identity is the one compile-time read left inside, on the
line naming this binary. It is absent in every build that is not a release,
which is every build that runs a scenario, so it cannot vary underneath one;
what did vary, the target triple, is off that line and stays on `--version`,
which the entry point prints without the instance.

**The seed is never derived from anything guessable.** Every unguessable
value the instance mints comes from it, and warrants are among them: a warrant
is what a container presents to the tools endpoint, and a container runs
somebody else's code. A seed reconstructible from a start time would let one
job forge another speaker's credential.

**Translation moves inside.** The instance renders every command the runtime
is given and parses every output it prints; the agent crate keeps those as
pure functions with their inverses, tested against outputs recorded from the
real runtime. The conversation with an agent becomes a state machine driven
by lines: it emits the initialise and session lines, reads answers and
notifications, answers permission requests, checks that each setting took,
and ends with a stop reason and what was reported. The channel's lifecycle
becomes instance state driven by frames, closes and a timer. Two locks become
held state: the build in flight, and the handle that stops a turn.

**Every arrival was stamped with the time by the world**, because a generic
world cannot know which handlers keep it — which is the argument
`docs/decisions/0073-the-world-tells-the-instance-the-time-with-every-step.md`
made for every event, so the stamp is on the step now and on no arrival.

**The vocabulary serialises in full and formats not at all.** Serialisation
is the contract: a scenario is a file of events, a trace is a file of
effects, and replay needs every byte, credentials included, which are fake in
every file a test writes. The generic types implement neither `Debug` nor
`Display`, so nothing can format one into a log by accident, and the rule in
`docs/conventions.md` §4 is met by absence rather than by a redacting
implementation a generic type cannot write correctly — it cannot know that a
warrant sits inside a line bound for the agent. A method naming the kind is
what the world logs. The instance's own snapshot for scenarios exposes its
state with credentials in the clear, as test support, for the same reason. A
recording taken from a running daemon is the flight-recorder question in
`docs/open-questions.md`, and redaction belongs to whoever writes one.

**The dashboard is a presentation server the instance proxies to.** The
application's entry point starts the framework's server on a loopback port
the kernel chooses, and the instance proxies dashboard requests to it exactly
as it proxies a job's tunnel to a container's port. The world holds no notion
of a service, and the proxy path is exercised on every dashboard request.
Server functions stay the framework's, and each does the one thing it should:
send the instance a typed request through the application hole and return
what it answered. Routing stays on the host, one label below the domain the
instance reads from its environment. Presentation is a third kind of code
beside deciding and performing: it decides nothing about state, needs the
framework's async machinery, and belongs to neither the instance nor the
world.

**A scenario is one typed file.** A title and a description; the seed, the
environment, and the effects and the full state after construction; then one
turn per event, each with the time, the event, the effects it produced, and
the change it made to the state as a standard JSON patch. The runner replays
construction and every turn, compares effects as values, and reconstructs the
state patch by patch, checking it against the instance every time. A flag
rewrites a file from an actual run, and the diff of the file is the review.
Files live flat, named for their flow, one test each.

**Exploration runs outside the gate**, from a recipe with a seed count and a
budget, on a workflow with a manual trigger and a schedule, writing a failing
seed's replay as an artefact to be minimised and committed. One fixed seed
runs in `just verify`, for a few seconds, so that the harness cannot rot
unnoticed.

**The gate runs in memory.** What the container tests check is what the
pinned runtime and the pinned agent actually do, which no in-memory test can
know; that is a recording rather than a test, so they become a recorder: a
recipe that runs reality, captures what it prints and how it answers, and
writes the captures as the fixtures the in-memory tests replay. Assumptions
that cannot be recorded — that a published port accepts and then closes — stay
in that recipe as checks against the real runtime. It runs by hand and when a
pin changes, and the gate never depends on it. The binary tests become
scenarios once boot is inside, and one smoke run in the recorder covers the
entry point's wiring. `just check` stays the pre-commit bar and `just verify`
the pre-push and CI bar; mutation testing is bound by the per-mutant build
and does not belong before a commit.

Rejected: **typed effects that name a command and let the world render it.**
Reusable in principle, and cleaner for the harness, but the world would still
know what a runtime is, and the scenario would not show the argument list
that actually runs. The harness reads the argument list back through the
instance crate's own inverse instead, so it never matches on strings and
cannot drift from the renderer.

Rejected: **the framework's router inside the instance.** Verified against the
sources: the router answers with a future by construction, because the
handler trait requires one, and there is no synchronous handler API; a
handler needs the instance's state to answer, and the step driving the router
is the step that owns that state; the renderer is async and runs inside the
framework's handler on the runtime; and a streamed body or an upgrade cannot
be a function from a whole request to a whole response. Beyond mechanics, it
would make HTML part of the instance's deterministic surface, so every visual
change would alter scenarios that are meant to be about behaviour.

Rejected: **telling the dashboard's requests apart by a cookie.** A person's
first request carries none, so the host rule is needed anyway; clients that
are not the page carry none; cookies are client-controlled state, which
`docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` already refuses
to route on; and a page on a subdomain may set a cookie scoped to its parent,
so a job's own page could reach into the dashboard's routing. The address a
person was given is the whole of what says where it goes.

Rejected: **the presentation port as an entry in the environment map**, which
would make the map something other than what the process was given, and
require every scenario author to know to include it. Rejected likewise:
**construction facts that are answers**, which were how 0056 handed over the
runtime's containers and the bound port; both are asked for now. The platform
is not one of those: nothing goes and finds it.

Rejected, for the platform in particular: **an environment variable naming
where a runtime might be**, which adds configuration
`docs/decisions/0023-the-container-runtime-is-discovered-once.md`
deliberately does without, and would let a start be pointed anywhere.
Rejected: **one merged list of every platform's paths**, which needs no
configuration but spends every start on paths belonging to other platforms
and quietly changes which runtime some machines pick. Rejected: **handing the
instance the list and the rules themselves** rather than a value naming the
platform, which would put that knowledge outside and let a caller substitute
it, where the point of 0023 is that nobody can. Rejected: **recording a file
per platform**, which cannot be done here at all, since nothing in this
project can produce a recording for a machine it is not.

Rejected: **the container tests as examples.** An example says how to use
something; these say what the outside does. Naming them as tests kept them in
the bar for pushing, and naming them as examples would lose their assertions.
A recorder is what they are.

## Consequences

**What leaves the world.** Every container command and every output parser,
the conversation, the channel's lifecycle, startup's sequencing, the domain
the tunnel layer read, and the routers it assembled. What stays is the list
in the first paragraph of the context, each piece generic and each obviously
what it says, plus the entry point: start the presentation server, construct
the instance and the world, step.

**What the scenarios gain.** Boot, in full, including every refusal. The
conversation, including a setting accepted and silently ignored. The
channel's refresh before the platform closes, which
`docs/decisions/0044-a-listener-only-listens.md` could only argue for. The
runtime's commands as the exact argument lists that run, and its outputs as
recorded from the real one.

**The migration is in phases, each with its scenarios**: the two crates and
the application hole first, with the instance's existing effects moved into
the hole and no behaviour changed; then boot and the file and process
families; then the HTTP model — bind, route, print, the presentation server;
then the conversation; then the channel's lifecycle; then the recorder, with
the bar for pushing made in-memory. Each phase moves one family out of the
hole. What the last leaves in it is what the decision above keeps there
deliberately, and it is not nothing: the presentation port's arrival,
because that is an application fact the entry point tells and the
environment map is rejected for it; and a server function's typed request
with its typed answer, because raw HTTP into the instance is rejected and
the identifier round trip that answers one is on the list, in the context
above, of what stays in the world. Neither is a mechanism a generic world
could perform. So the migration is finished when nothing that is a
mechanism is left in the hole, and what the application supplies to perform
its own effects then does one thing, which is to match an answer to whoever
asked.

They were meant to be one branch and one merge, and the first four landed
that way before the branch got long enough that carrying it cost more than
merging it. What makes that safe is a property the phases have anyway: each
one leaves the program working, because a family that has not moved is still
performed where it was. Which families are left is
`docs/open-questions.md`, which is where an intention belongs; this record
says what they are and why, which does not change as they land.

**Reversing** is one rendering step per family: a generic effect is a
semantic one rendered inside rather than outside, so any phase can be undone
alone. The scenario files, being replays, change with the vocabulary and are
rewritten by the recorder.

**Revisit if** the loopback hop per dashboard request is ever noticeable,
which would be the case for handing the presentation server to the world as a
service; if a family turns out to need a meaning the world must know, which
would be a sign the seam is drawn one layer too low for it; or if exploration
runs for a long time without finding anything, which would mean its invariants
are too weak rather than the code too good.
