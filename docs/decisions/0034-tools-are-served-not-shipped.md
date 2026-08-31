# 0034 — Tools are served, not shipped

## Status
Accepted. Supersedes
`docs/decisions/0028-stageman-ships-the-tool-that-speaks.md`, rescopes the
warrant in `docs/decisions/0032-a-foreman-asks-the-instance-by-warrant.md`, and
changes what the listener in
`docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md` serves.

## Context

Everything an agent did outside its container went through a command-line
program this project writes and ships in the image: `stageman-say` posts on a
channel, `stageman-job` asks for work. Behind them sat a warrant, a listener on
every interface, a hostname that works on two runtimes, and two files copied
into a stopped container before every start.

All of it rested on one sentence in
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`: *neither
agent adapter examined supports tools served over the protocol connection, and
both want them over HTTP.* That was read as closing both halves. It closed
neither — it was a claim about two adapters at one moment, and the second half
was never tried at all.

Both halves were measured, against the adapter pinned in
`agent/images/claude/Dockerfile`. Protocol names below are unbackticked
deliberately: they belong to a dependency, and `just drift` resolves a
backticked identifier against this source.

**Over the protocol connection: no, and it fails silently.** The adapter
advertises mcpCapabilities of http and sse, and no acp. Offering an
acp-transport server in session/new is nevertheless *accepted* — and so is a
deliberately invented transport name, and a malformed http entry with no url,
so acceptance carries no information whatsoever. No mcp/connect ever arrives.
The adapter's own translation loop has a branch for http and sse, a branch for
a bare stdio entry, and no else, so the declaration is dropped in silence. That
version was the latest published, and no agent implementation examined declares
the capability.

**Over HTTP: yes, end to end.** A container reaches a server on the host through
the add-host flag it is already created with; a per-session credential travels
in an Authorization header on the declaration; the model discovers the tool and
calls it with typed arguments; and the permission request arrives as
session/request_permission, which the app already answers.

Two further measurements decide the shape rather than the feasibility.

**A resumed container picks up a new address.** One container, one session,
stopped between turns. The first turn's endpoint was killed and a second
started on another port; the resumed turn — loaded exactly as this project
resumes, by listing sessions and then loading one, naming the new address —
reached the new endpoint. Session loading carries the same server declarations
session creation does, and the adapter applies both through one path.

**Authority travels with the credential.** The same session offered a
foreman's credential is served a job-creating tool and uses it; offered a job's,
it is served only the speaking tool, searches, and reports it has none.

## Decision

**The instance serves its own tools, as an MCP server over HTTP, declared per
session.**

- **The image carries no program this project writes.** Only the agent, `git`
  and `gh` — things it cannot serve.
- **Every session names the endpoint**, on creation and again on every resume,
  and carries a credential minted for that session.
- **What the tool listing returns is decided by that credential.** A foreman's
  session is offered the tool that creates jobs. A job's is not.
- **Handlers run in the process that owns the state**, so a call arrives as
  typed values rather than a shell command to parse, and a reason or an
  instruction stops being a string quoted through a command line.

Rejected: **forking the adapter to consume tools over the protocol
connection.** It is genuinely small — the adapter already builds an in-process
MCP server for its own file-change audit, so the pattern is in its source — and
the licence permits it, and its dependencies are exactly pinned, so a frozen
fork would keep working. It was rejected on what freezing *means*: the pinned
adapter pins the agent SDK exactly, and that SDK carries the coding agent
itself. Forking therefore freezes the agent every job runs, permanently, in a
project whose entire value is running a good one. Bumping the agent under a
frozen fork is the untested pairing upstream pins against. That is a product
cost wearing a maintenance costume, and it buys only the two items in the next
paragraph.

Rejected: **shipping one MCP server binary in the image** instead of two
command-line programs. Measured working, and it buys the typed arguments. It
keeps a program in the image that can be older than the instance driving it,
which is the drift this record dissolves, and keeps the endpoint file.

Rejected: **a Unix socket bind-mounted per container**, which would have made
authority structural again — possession of a socket rather than a checked
credential — and removed the listener with it. Measured on both runtimes on
macOS: Docker refuses to connect with ENOTSUP, and Podman does not surface the
socket in the mount at all. It works on a Linux host, and that is exactly what
makes it the wrong answer — a mechanism that works where this is deployed and
not where it is developed.

## Consequences

**What goes**: both programs, their argument handling, the endpoint file, the
thread file, and the mechanism that copies values into a stopped container.
Nothing this project writes is in the image, so a tool can no longer be older
than the instance driving it — the question of delivering tools rather than
baking them in dissolves rather than being answered.

**What stays**: the listener on every interface and the one hostname that
reaches it from either runtime. Only serving tools over the protocol connection
removes those, and no agent can consume that yet.

**The listener stops serving one route.** 0033 records "one route is served
there and nothing else" as a thing standing behind an open port, and that
sentence is now false: the endpoint speaks a general protocol, which is more
surface than one bespoke route. The credential in front of it and the refusal
of non-private peers are unchanged. The endpoint must also tolerate methods it
does not know, because the client sends at least one that is not in the spec.

**The warrant changes character, and the trade is not one-sided.** It was
structural: a job's container was never given one, so a job could not create
jobs however well it reached the daemon, and there was nothing to get wrong. It
is now a check the daemon makes against a per-session credential — so a bug in
that check is a privilege escalation where previously no code existed to have a
bug in. Against that, it is now *testable* by driving a real session and
observing what the model is offered, where the old property could only be
asserted by noting a file's absence. 0032 predicted this exactly: its revisit
trigger says that when a job needs to ask the instance for something, "a
foreman's warrant" becomes "a warrant, scoped". This is that.

**An unreachable endpoint is silent, and this is the sharpest edge here.**
Measured: session creation succeeds in under a second and the agent simply has
no tools — no error on either side, and a model that behaves as though the
tools were never mentioned. It is the same failure shape as the dropped
protocol-transport declaration above, and it is now this project's to prevent
rather than a third party's. The daemon serving the tools is the process that
would refuse to start without them, so `docs/conventions.md` §3's startup rule
covers birth; nothing yet covers an endpoint that stops answering later.

**Tools arrive deferred rather than loaded.** The model finds them by searching
before calling. Both observed sessions did this and both succeeded, and the
listing response is ours, so asking for them to be loaded up front is available
without a protocol change.

**Reversing** means putting the programs back in the image, the files back in
the container, and rebuilding — no data migration, because none of this is
persisted, but every existing container would need recreating, which discards
its session. Cheaper before there are long-lived foremen than after.

**Revisit if** an agent worth running consumes tools over the protocol
connection, which retires the listener and the hostname and is the only thing
that can; or if stageman ever runs where the daemon and the container are not
on the same host, which turns a link between neighbours into a real network
service; or if a tool needs to stream, or to take longer than the client is
willing to wait for a call.
