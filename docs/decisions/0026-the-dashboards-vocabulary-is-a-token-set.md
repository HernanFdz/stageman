# 0026 — The dashboard's vocabulary is a token set, named for meaning

## Status
Accepted.

## Context

The dashboard needs a look, and the first component makes that decision for
every component after it. Three things get chosen at once and only one of them
is aesthetic: what a colour is *called*, where that name is defined, and
whether a component may ever write a colour directly.

Tailwind is the framework already implied by the toolchain — `dx` compiles it
without a package manager — and its version 4 defines a project's design tokens
in a `@theme` block in ordinary CSS, which generates the utility classes.

## Decision

**Every colour, font and spacing decision is a named token, and a component
names the role rather than the value.**

A component writes `bg-surface` and `text-muted-foreground`. It never writes
`bg-white` or `text-gray-500`, and never a hex value. The tokens live in one
`@theme` block, which is the only file that knows what any of them look like.

The names follow the convention the Dioxus and Tailwind component registries
emit — `background`, `foreground`, `surface`, `muted-foreground`, `border`,
`primary` — rather than a vocabulary invented here. That is a practical choice
and not a preference: those registries are copy-paste by design, and a
component lifted from one of them compiles against these names unchanged.
Inventing a private vocabulary makes every such lift a rewrite, forever, in
exchange for nothing a reader gains.

**One token per job state, and exactly three**, matching the three in
`docs/conventions.md` §2. A fourth colour here would be an invitation to invent
a fourth state in a view rather than in the domain, which is where states are
allowed to be added.

**Monospace is a decision, not decoration.** Most of what this dashboard shows
is an identifier, a path or a container name, and those are read character by
character; they get the mono token, and prose does not.

## Rejected: a vocabulary named for this project

The alternative that reads better in isolation — `ink` on `canvas`, `accent`,
`surface-alt` — and which has a real argument behind it: `text` collides
awkwardly with the utility Tailwind generates from it, and a paper metaphor
gives a designer somewhere to stand.

It loses on the lifting cost above. It is the right choice for a product whose
look is part of what it is; this is an operator console whose look should get
out of the way, and whose components are worth acquiring rather than
authoring.

## Rejected: raw utility classes, no token layer

Fewer moving parts, and for a dashboard this small it would work today.

It loses the first time anything changes. A colour used directly is a colour
that has to be found everywhere it was used, and the find is a text search
rather than a compile error. It also makes a second theme — dark, most
obviously — an edit to every component instead of a second block in one file.

## Consequences

A component may not reach for a colour that has no name. When a screen needs
one, the token is added first, which is a moment where somebody asks what the
colour *means* — and that question is the whole benefit.

The token set is small on purpose and will grow. What must not grow is the
number of places that define one.

**Nothing here reaches the domain.** Primitives take strings and numbers, which
is what keeps them on the right side of
`docs/decisions/0022-the-browser-never-sees-the-domain.md` without anybody
having to think about it.

Reversing is a search-and-replace over class strings, and it gets more
expensive per component added — which is the argument for taking the decision
now, at four components, rather than at forty.

Revisit if this ever ships a look somebody chose deliberately, rather than one
that stays out of the way. A product with an identity wants a vocabulary that
carries it, and at that point the lifting cost is worth paying.
