# 0058 — A channel's adapter is a crate beside the agent's

## Status

Accepted. Decides where the translation
`docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
moves out of the app crate goes for a channel, which that record left open;
adds a crate to the shape in
`docs/decisions/0003-four-crates-around-a-core.md` for the same reason the
agent crate is one.

## Context

0057 moves every piece of code that knows a protocol inside the seam, as pure
functions the instance renders and reads: the agent adapter's commands and
its conversation went to the agent crate, where the adapters for coding
agents already were. The channel's translation — what a message posted to a
platform looks like, what its answer means, how a frame decodes and how it
is acknowledged — lived in the app crate beside the tasks that performed it,
and with those tasks gone it needs a home that the instance can name and
that performs nothing.

Three candidates were in the tree. The foreman crate, on the reading that it
"holds what it needs in order to watch a project's channels". The core
crate, which already holds the channel's domain types — a binding, a thread,
the speaking half of a binding — and the routing rule. The instance, which
sequences everything and could render what it sequences.

`Channel` and `Agent` are the two closed sets in the domain, and they are
closed for the same stated reason: reaching one needs code, and code is not
something an operator supplies. The code that reaches an agent has a crate
of its own, named by the instance for both shapes of work an agent does.

## Decision

**A channel crate beside the agent crate.** It holds the contract every
channel is spoken on and the adapters that implement it — what is sent to a
platform, what its answers mean, what a frame carries, and what has to be
acknowledged — as pure functions, dispatched on `Channel` at the crate's
surface so that nothing outside names a platform. It may name **core** and
nothing that performs an effect. The instance names it for both directions,
speaking and listening; the app names it only while the listener is still
performed there, and not afterwards. It ships with the inverse of what it
renders, so that a simulated platform recognises what it is asked without
matching on strings, the way the agent crate's commands are read back.

Rejected: **a module in the foreman crate.** The foreman crate is the
deciding for one project, and watching a channel is its remit; but a job
speaks on a channel too, through the tools endpoint, and the rule that
foreman and job never name each other would have every job's words rendered
by the crate that decides what a foreman does. Translation that knows a
platform is a different kind of code from judging what a signal deserves,
and the agent crate is the precedent for keeping them apart.

Rejected: **the core crate.** It holds the channel's types because they are
the domain's — a thread is where one job's conversation happens whatever the
platform — and it holds no platform's shape on purpose: no I/O, no async
runtime, no platform, no framework, so that the routing rule can be tested
against every combination. A request body and a status code are a
platform's shape.

Rejected: **the instance.** `docs/conventions.md` §3 has a third party's
quirks stopping at an adapter's boundary, so that a change to that party's
interface touches one crate. A platform is on somebody else's release
cadence exactly as an agent is.

## Consequences

**One more crate in the workspace**, with a manifest line in the browser
pass's exclusion list, a bullet in `docs/architecture.md` §1, and a
sentence in the dependency rule there: **channel** may name **core**, and
**instance** may name it. It has no README, like every internal crate.

**The instance dispatches nothing on a platform.** A post is rendered by
naming the channel, and an answer is read by naming it; a second channel is
a second module under this crate and a second arm in each function at its
surface, and nothing else moves.

**The simulated platform answers as the real one was measured to.** It
recognises a request through this crate's inverse and answers with the
platform's own shape — including a refusal arriving with a successful
status, which is the trap the reader guards against and can now be a
scenario rather than only a unit test.

**Reversing** is moving two modules into another crate, and there is nothing
recorded to migrate.

**Revisit if** a second channel arrives whose contract does not fit posting
at an address and reading a thread — a channel with no threads, or one that
is read by polling rather than over a socket — which would be the case for
the contract at this crate's surface growing a second shape rather than a
second arm.
