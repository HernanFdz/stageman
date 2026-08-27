# 0010 — The Agent Client Protocol is the contract's shape

## Status
Accepted

## Context

`docs/decisions/0006-agents-are-pluggable.md` settled that agents are pluggable
and that a crate owns the contract. It did not settle what shape that contract
takes. Two candidates: an open protocol spoken over a subprocess's standard
input and output, or an adapter per tool written against each agent's own
headless interface.

A throwaway spike drove two agents from different vendors both ways. What it
found matters more than the conclusion, because most of it is durable
regardless of which option had won.

**Both agents can be driven headlessly and both produce schema-constrained
output**, by different flags — one takes a schema inline, the other by file
path. So the one-shot half of the contract is genuinely vendor-neutral, which
was the half in most doubt.

**Neither agent's headless interface will round-trip a permission request.**
Asked to write outside its working directory, one agent auto-denied, reported
the denial as an event, and finished the turn — in one-shot mode and in duplex
mode alike, with the parent's input stream held open for over a minute. It does
not ask. It decides and reports.

That mattered less than expected, because it exposed a conflation: a job that
needs a human is nearly always asking a *question*, not requesting a tool
permission, and a question is answered by resuming the session with the answer.

**The protocol normalises session vocabulary, and that is what it is for.** Both
agents' adapters advertise resumable sessions as negotiated capabilities, using
one vocabulary, where their native interfaces express the same idea as a flag on
one and a subcommand on the other. They differ at the edges — one offers forking
a session, the other closing one — which is capability negotiation working
rather than failing.

**The protocol's transport survives isolation.** A containerised process
answered three interleaved requests over standard input and output with
networking disabled entirely, driven from an uncontainerised parent on a
platform whose containers run in a virtual machine.

## Decision

The contract takes the protocol's shape, and adapters implement it. Where an
agent speaks the protocol through a separate adapter binary, depending on that
binary is acceptable.

Rejected: **an adapter per tool over each agent's own headless interface.** Both
agents can do everything needed this way — this loses on cost, not capability.
It means tracking two vendors' flags, output formats and session models forever,
and being wrong per agent rather than once. The spike found the two agents
disagreeing on how to express resumption before a single line of adapter code
existed, which is a preview of that maintenance.

## Consequences

A dependency on adapter binaries this project does not build, distributed
through a package manager it otherwise has no use for — so an image that runs an
agent carries that toolchain too. Both adapters were observed migrating out of
one editor vendor's namespace into the protocol's own organisation during the
spike, which reduces the concern that they serve somebody else's roadmap; it
does not remove it.

Adapters **under-declare**. One advertised only an interactive login among its
authentication methods, while the binary beneath it demonstrably accepts
credentials from its environment. So what an agent advertises is useful where
present and is not authoritative — per-agent knowledge stays in the adapter,
as `docs/decisions/0008-one-credential-per-agent.md` already assumed.

Neither adapter supports tools served over the protocol connection. That would
have forced an inbound path into a job's environment, and it is the reason this
record does not claim the transport result extends to hosted tools. It is now
moot: `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` removes
hosted tools entirely.

Reversing means writing the per-tool adapters that were rejected — the cost of
the rejected option, paid later, with the protocol's vocabulary as a head start
on what those adapters would need to express anyway.

Revisit if an agent worth supporting has no adapter and writing one against the
protocol is harder than writing one against its own interface, or if adapter
releases lag their agents badly enough to block work that matters.
