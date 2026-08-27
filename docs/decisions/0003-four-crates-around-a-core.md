# 0003 — Four crates around a core

## Status
Accepted. The crate count is superseded by
`docs/decisions/0006-agents-are-pluggable.md`; the dependency rule and the
rejected alternatives below still stand.

## Context

There is one real seam in this system: deciding what is worth doing, and doing
it. They run differently — one watches continuously and holds every credential,
the other is spun up per unit of work inside an isolated workspace. They also
fail differently, and an agent editing code is far more likely to blur them than
a human is.

Separately, running this is meant to mean running one executable that also
serves a dashboard, which puts a user interface in the picture before any of the
above exists.

## Decision

Four crates. **core** holds the domain and no I/O. **orchestrator** watches,
judges and composes prompts. **job** provisions a workspace, supervises one
agent process and hosts its tools. **app** is the Dioxus fullstack binary,
serving the dashboard and running the orchestrator in the same process.

Dependencies point inward, and the forbidden direction is stated in
`docs/architecture.md` §1: orchestrator and job may never name each other.
Everything they share is a type in core, which is what stops the deciding and
the doing from growing into one another.

Rejected: **ports and adapters** — a core with traits, every channel and every
workspace mechanism an implementation behind one. It is where this lands if the
isolation mechanism or the agent protocol turns over a few times, and both are
open questions today. It lost on timing: an interface designed against one
implementation ends up shaped like that implementation, and the two questions in
`docs/open-questions.md` are exactly the ones that would tell us what the traits
should look like. Deferring costs a refactor later; guessing costs a wrong
abstraction that outlives its reason.

Rejected: **one crate with modules**, splitting when a boundary needs enforcing.
Cheapest to start and to move code around in, and the boundary rules would be
identical. It lost because those rules would then be held by review alone, and
this is the specific boundary an agent breaks first — a helpful import from
orchestrator into job compiles, reads fine, and is exactly the change that
matters.

## Consequences

Four crates before there is code to put in them, which is real overhead: a
workspace to keep coherent, and moments early on where a type has no obvious
home and gets one anyway.

Cheap to reverse in either direction. Collapsing crates into modules is
mechanical, and splitting further later is only expensive if the boundary has
been violated in the meantime — which is the thing this arrangement exists to
prevent.

Revisit if core starts accumulating types that exist only to let orchestrator
and job talk past each other. That is the signature of a seam in the wrong
place, and it is the point at which the ports-and-adapters shape earns its cost.
