# 0014 — The protocol's own SDK, and our own spawning

## Status
Accepted

## Context

`docs/decisions/0010-acp-is-the-agent-contract.md` settled that the contract
takes the protocol's shape. It did not settle how this project speaks it. The
protocol is JSON-RPC over a subprocess's standard input and output, and the
surface needed today is small — `initialize`, `authenticate`, a session, a
prompt, and the notifications a session emits — so writing the wire format by
hand was a genuine option rather than a straw man.

A throwaway spike drove the image from `docs/decisions/0012-agents-run-in-containers.md`
through the protocol's official Rust SDK. What it found:

**The SDK drives a container unmodified.** `docker run -i --rm --network none`,
spawned as the agent process, answered `initialize` and negotiated protocol
version 1 against adapter 0.70.0. Nothing about the transport cared that the
subprocess was a container rather than a program.

**A session can be created with no credential and no network.** Creating one
returned a session identifier over `--network none` with nothing supplied to
authenticate with. So the credential boundary sits at the prompt, not at the
handshake — which means a test can drive a real container as far as a session
without a token, and that is the difference between this contract being
exercised in the gate and being asserted in prose.

**The adapter advertises no authentication methods at all.** Its list of them
came back empty, from a binary that demonstrably accepts a credential from its
environment. 0010 predicted this and called it under-declaring; it is now
observed rather than expected, and it is the reason per-agent knowledge stays
in an adapter.

**The SDK's own transport brings a second async runtime.** It spawns through
`async-process`, and `docs/conventions.md` §3 fixes this project's runtime as
the one Axum is already using. The SDK's `Lines` transport is public and
generic over a sink and a stream, so the process can be spawned with the
runtime this project already has and handed to the SDK.

**The SDK's tokio companion crate is a major version behind its core.** At the
time of writing the companion depends on a 0.x release of a crate that has
since reached 2.x, so taking both would mean compiling two incompatible copies
of the protocol. It is not usable, and the twenty lines it would have saved are
written here instead.

## Decision

Depend on the protocol's official Rust SDK for the vocabulary and the
connection, and spawn the container process here rather than through the SDK.

## Rejected: writing the JSON-RPC by hand

Tempting because the surface needed *today* is small, and it would leave this
project with no dependency on somebody else's release cadence — the concern
0010 recorded and did not resolve.

It loses on what the surface becomes rather than what it is. 0010 chose the
protocol because it *"normalises session vocabulary, and that is what it is
for"*: content blocks, session updates, tool calls, permission requests and
stop reasons are the bulk of the protocol and the bulk of its value. Owning
that by hand is the rejected option from 0010 — being wrong per vendor rather
than once — moved one layer down and paid per protocol revision instead of per
agent.

## Rejected: the SDK's spawner as well as its protocol

Three lines instead of forty, and proven working in the spike.

It loses on two counts, and the second is the one that decides it. It puts a
second reactor and its thread pool beside the one `docs/conventions.md` §3
already commits to. And it puts the child process's lifetime in a dependency,
when *"killing stageman leaves nothing behind"* is a stated bar in
`docs/conventions.md` §4 — a container is a process this project must be able
to reason about killing, and that reasoning cannot live in a crate whose drop
semantics are not ours to change.

## Consequences

A dependency under a licence this project's dependency policy already allows,
on a crate whose version history is visibly fast-moving — 0.11 to 2.0 inside
one release cycle, with its own companion crate left behind. That is the cost
0010 flagged, now taken deliberately rather than inherited.

Protocol types appear in this project's own signatures wherever the contract is
expressed, so an SDK upgrade is a change to the contract's shape rather than to
one adapter. That is the price of not owning the vocabulary, and it is the same
price 0010 accepted when it chose the protocol at all.

Spawning is ours, so the container's arguments are a value this project
constructs — which makes them a pure function, testable without a container,
and the place `docs/conventions.md` §3's rule about the runtime never being a
PATH lookup is actually enforced.

Reversing means writing the wire format, which is the rejected option's cost
paid later. The transport boundary makes this smaller than it sounds: the
process spawning stays either way, and what would have to be written is the
schema and the request correlation.

Revisit if an SDK upgrade breaks this project's build twice in a row without
the protocol itself having changed, which would mean the dependency is tracking
its authors' refactoring rather than the specification.
