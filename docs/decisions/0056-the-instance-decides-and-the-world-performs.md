# 0056 — The instance decides, and the world performs

## Status

Accepted. Supersedes the rejection of ports and adapters in
`docs/decisions/0003-four-crates-around-a-core.md` for one seam — the boundary
between what this project decides and what it asks of the outside — and leaves
that record's crate rule standing. Narrows
`docs/decisions/0011-state-is-a-snapshot-not-a-database.md`: the snapshot is
still written on every change, but the write is asked for rather than made.
Makes structural what `docs/decisions/0044-a-listener-only-listens.md` left to
a reviewer, and turns the ordering a job's record relies on — written before
its container exists — from a comment into a mechanism.

## Context

Everything this project decides is already deterministic, and most of it is
already pure. The domain crate takes timestamps and nonces as arguments, and
mutation testing has pushed decision after decision out of the app crate's
async functions into functions that take a state and answer. What is left in
those async functions is the sequencing — what happens across an await, and
across the daemon dying — and none of it is reachable from the gate. Sixty
functions are skipped by mutation testing, every one because it drives the
container runtime or the network, and they are where the bugs have been: the
listener that awaited the work (0044), the foreman restarted holding a message
that nothing drove again (0045), the sweep that stopped another instance's
container (0054), the job that died when a tool it was told to call was
unreachable (0055). Each was found by running the daemon, and each is a test
nobody could write, because the code holding the bug cannot run without a
container runtime, a channel, and minutes of wall-clock time.

The remedy is not new. Deterministic simulation testing, as practised by
TigerBeetle and by the Trustfall query engine, splits a program into a
deterministic core and a simulator that supplies everything else — time,
randomness, the network, the disk. Test worlds step the core, injecting
failures from a seed or following a scenario, and every failure reproduces
exactly, because in a deterministic world a bug either always happens or never
does. Its price is that the core cannot await anything: every long operation
becomes a request now and a completion later, and the code that sequenced them
becomes explicit state.

Before deciding, a throwaway sketch with no dependencies, on the pinned
toolchain, checked the shape that would fit here: one synchronous value holding
the state, stepped one event at a time and answering with effects as values; a
simulated world with a virtual clock, a seeded generator and a fault plan; a
crash that drops the value and rebuilds it from what was last persisted.
Measured: the same seed produced the same trace on every run and another seed
diverged; a crash injected inside a persistence window lost exactly the write
that had not landed, so the client was never told its request had succeeded
and the job it asked for never existed; and every invariant checked after
every step held across the restart.

## Decision

**The daemon is an instance stepped by a world.** The instance is one
synchronous, single-threaded value holding everything this project knows and
making every decision it makes. The world is everything outside it: the
container runtime and the agents inside it, the channels, the clock,
randomness, the disk, the servers, and the async runtime that drives them. The
two meet through two enumerations of plain data. An event is what the world
tells the instance; an effect is what the instance asks of the world. The
instance is constructed from the bytes of its file, if there is one, the key
that opens them, a seed, and the facts it needs before it can act at all — the
containers the runtime holds, their labels, and whether they run — and answers
with what it does on waking. After that it has one method: one event in,
effects out.

**Effects are values, never calls.** The instance cannot observe the world
except through an event, so determinism is a property of the type rather than
of anybody's discipline. It never reads a clock: the event variants whose
handlers keep a time carry one, and no others do. It never reads entropy: it
owns a generator seeded by the world at construction — from the operating
system in production, from the scenario in a test — and every identifier,
nonce and warrant comes from it. It holds no lock, spawns nothing, awaits
nothing, and keeps every map ordered.

**An effect is answered only where the next decision depends on its outcome.**
Running a turn, probing a tunnel, listing what is running, a timer, opening a
thread and persisting are answered by an event. Stopping a container, posting a
notice and acknowledging a frame are not, and a failure in one of them is the
world's to log. The world's one obligation is the same for every answered
effect: the completion is sent when the effect has actually completed, never
when it has merely been started.

**Persisting is an answered effect, and the instance waits for it before facing
outward.** A step that changes what is kept ends by asking the world to write
the sealed bytes, and every effect of that step that is outward-facing or
irreversible — a response to a client, a turn started, a message posted — is
held back until the world says the bytes have landed. The instance seals,
opens, checks and bridges its own file; the world reads one file before the
instance exists and writes one file atomically afterwards. So a client told
that a change succeeded was told the truth, a failed write can answer a client
with an error rather than a log line, and the record is on the disk before the
container it names exists, where the simulation can see that it is.

**What is kept and what is held are two things.** The kept state is what goes
to disk. The held state is what only this process knows — turns in flight,
warrants minted, a job's tunnel port, when a listener went deaf, effects
waiting on a persist — and a restart begins with none of it. A completion for
a turn started before a crash is therefore ignored by construction.

**The world routes by shape and never decides on state.** The tools endpoint
is its own listener and forwards each request whole. The dashboard's listener
wraps the framework's router in one layer that asks the instance only about
hosts one label below the instance's domain, and the instance answers with a
port to forward to or a response to send; every other request goes to the
framework, whose server functions each hand the instance a typed request and
await its typed response. A request enters as an event carrying an identifier
and is answered by an effect carrying it back, in that step or a later one.
The types that cross to a browser move to a crate of their own, named by both
halves and by the instance, so that the instance can answer a request without
naming the app.

**The instance is a crate.** It names the domain, the foreman, the job and the
agent crates for their types and their pure functions, and nothing that can
perform an effect. The app crate becomes the world: the loop, the listeners,
the servers, the adapters, and the entry point. The rule that dependencies
point inward stands, with this crate between the four and the app.

Rejected: **an asynchronous instance on a single-threaded runtime with a paused
clock, behind trait seams.** A smaller diff, and determinism by discipline:
every await is a point the scheduler decides, every trait method a place where
a synchronous answer could creep in, and a crash can only be injected where an
await already is. It also keeps the shape that produced 0044 and 0045, and
every sequencing feature still queued in `docs/open-questions.md` would be one
more await in it.

Rejected: **a trait per mechanism**, which is what 0003 refused and refuses
still. This is one seam, drawn where the nondeterminism is, with a vocabulary
derived from mechanisms that are now settled — containers by 0012, 0023, 0043,
0047 and 0054, the agent protocol by 0010 and 0014. Its purpose is
determinism, not pluggability, and it names no channel trait and no workspace
trait.

Rejected: **simulating the network.** One library simulates a network of hosts
and another replaces the async runtime; this project's nondeterminism is
subprocesses, one external API and a clock, so both simulate the wrong thing,
and the second is invasive.

Rejected: **a fake container runtime as an executable.** Nothing can be pointed
at it since 0023 compiled discovery in, it costs a process per call, and the
agent's half of a conversation cannot be scripted from a shell.

Rejected: **a read replica for the dashboard.** The world would hold a copy of
the kept state and the code that reads it, which is the world knowing the
instance rather than knowing how to speak to it. Every read is a request
instead, and the loop answers one in microseconds.

Rejected: **forwarding every request to the instance as raw HTTP.** The
framework decodes server-function calls and renders pages, and doing that again
inside the instance would pull the framework into the deterministic core. Only
the two places that decide on instance state — the tools endpoint, and which
job a host names — see a request whole.

## Consequences

**The app crate's orchestration is rewritten as handlers**, and its pure
decisions survive verbatim. The listener, the tooling and the serving modules
become thin. The agent crate is unchanged in substance and becomes the world's
implementation of the runtime effects, tested against the real runtime as it
is today. What is new is the instance crate, the wire crate, a simulated world,
and the scenarios.

**Tests change shape.** A scenario is a starting file and a script of events;
run against the simulated world it produces the trace of effects and the state
after each step, and those are snapshots reviewed like code. A seed is a random
world, and a seed found failing is committed as a scenario. The bars in
`docs/conventions.md` §4 that were prose — nothing untracked after a kill,
nothing running that is not showing something, a turn surviving the daemon
dying — become invariants the simulated world checks after every step. The
container tests stay, as the tests of the world's real implementation: a
simulated runtime encodes a belief about the real one, and 0047 is what a wrong
belief looks like.

**Every state-changing request is answered one write later** than it was,
which is milliseconds. Every request passes through one loop, which is
microseconds, and is the first place to look if the dashboard ever feels slow.

**The instance is built beside the old code and switched in at once.** The
crate grows flow by flow, in the order the constructor forces — startup and the
sweep first, because the instance is born with the runtime's facts; then
replies and the reply gate; the foreman's loop; the tools endpoint; the
dashboard's requests, with the wire crate; the tunnel's routing — each flow
landing with its scenarios, and nothing but those scenarios exercising it
until it covers everything the app does. Then one change makes the app the
world and deletes the old orchestration, the store, the process-wide
registries and the runtime static together. Rejected: moving one flow at a
time through a transitional world in which the store holds the instance
behind its lock and performs each step's effects in place. It keeps every
intermediate state runnable, and it costs glue that is thrown away and a
period in which two shapes carry the same work — which doubles the surface
for exactly the bugs this exists to remove.

**Reversing** is the rewrite in the other direction, and it grows more
expensive with every flow moved. Nothing on disk changes: the file's shape is
the instance's to keep, and it keeps it.

**Revisit if** the simulated runtime and the real one are found to disagree
twice in the same direction, which would mean the belief is wrong and the seam
is at the wrong level; if the loop is ever the thing an operator notices, which
would mean a step has grown work that belongs in an effect; or if a bug is
found inside a turn's conversation that the canned conversations could not
reach, which is the case for moving the protocol itself into the instance, one
event per message.
