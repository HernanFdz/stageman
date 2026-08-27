# Architecture

The shape of this project and why it has that shape. Read this before changing
code. `docs/vision.md` is for deciding what to build; this is for deciding where
it goes.

**Backticked paths and identifiers here are verified.** `just drift` resolves
every backticked path against the repository and every backticked Rust
identifier against the source, so a citation that stops being true fails in the
commit that broke it rather than a year later. `docs/conventions.md` above is
one such citation, and it is there so the check has something to resolve from
the first commit — a check reporting a denominator of zero is honest and proves
nothing. `drift-doc-symbols` stays at zero until you name your first identifier;
that is not a pass, it is an absence of evidence.

**Never restate what the repository already knows.** No directory trees, no
dependency lists, no build order, no status. All of it is derivable, all of it
goes stale, and `just drift` cannot save you from a sentence that was true when
it was written.

**Each section says what belongs in it and starts empty.** That prose is a
standing rule, not a placeholder: it stays when the section fills up, because it
governs what gets added next. Only the HTML-comment example and the
`_(none yet)_` marker are disposable.

## 1. The pieces

What each crate or module is *for* — the part `cargo metadata` cannot tell you,
because it is intent rather than structure. One line each is usually enough; the
reasoning goes in §3.

Say which way the dependencies point, and which direction is forbidden. That is
the single most useful sentence in this document, and the one an agent breaks
first.

<!-- Example:
- core — the domain types and their invariants. No I/O, no async, no framework.
- api — the HTTP surface. Every handler is a thin adapter over core; a business
  rule that reaches a handler is a bug, not a shortcut.
Dependencies point inward only: api may name core, core may never name api.
-->

_(none yet)_

## 2. Invariants

What must be true at all times, stated so that a reader can tell whether a
change breaks one. These are the properties the type system, the tests, or a
reviewer are defending — write down which, because "invariant" enforced by
nobody is a wish.

<!-- Example:
An identifier is unique for the lifetime of a deployment. The type is opaque and
constructed in one place; nothing else may mint one.
-->

_(none yet)_

## 3. Why this shape

The forces that produced the structure above, and the shapes that were rejected.
Without this, the next person reads §1 as arbitrary and reorganises it.

For a choice big enough to have consequences, write a record in
`docs/decisions/` and cite it here rather than repeating the argument — a
decision belongs in one place, with its rejected alternative and what would make
it wrong.

<!-- Example:
Kept as one process rather than split into services. The split was tried and
reverted: every boundary needed the same transaction, so it bought network calls
and bought nothing. Revisit if a component's write rate stops fitting on one
machine — see docs/decisions/0003.
-->

_(none yet)_
