# 0022 — The browser never sees the domain

## Status
Accepted.

## Context

`docs/architecture.md` §1 has named the app crate a Dioxus fullstack binary
since `docs/decisions/0003-four-crates-around-a-core.md`, and nothing had
compiled it. Doing so forces a question that reads like packaging and is not:
**one crate is compiled twice, for two machines**, and one of those machines is
a browser tab.

The daemon's half may name everything. The browser's half is a bundle served
to whoever opens the page, and what is in it is in their hands: readable,
patchable, and running in a process this project does not control.

**core** holds `Secret`, `Key`, and the cipher that opens a snapshot. None of
it has any business in a page. Nothing there would leak a credential merely by
being compiled — the values live in a file on the daemon's disk — but shipping
the type whose whole purpose is not to leak, and the routine that undoes the
encryption, to the one place with no reason for either is the kind of thing
that is only ever noticed afterwards.

The mechanical half is sharper, and it is what makes this a decision rather
than a preference. **A `cfg` does not stop cargo building a dependency.** Code
can be hidden from the compiler by a feature; a dependency is only removed by
the manifest. So a crate that gates its server code with `#[cfg]` and leaves
`stageman-core` in `[dependencies]` compiles core, and the cipher, and the
async runtime, for `wasm32-unknown-unknown` — and finds out whether they build
there at all.

They often do not. The `uuid` crate routes randomness through `getrandom` from
1.20, which refuses to build for wasm without a JavaScript backend; this
workspace pins 1.26 in two crates and would have met it.

## Decision

**The browser gets one module of plain serialisable types, and nothing else.**

Every server-side dependency of the app crate is optional and activated by a
`server` feature: the four internal crates, the async runtime, the cipher's
neighbours, everything. The browser's half activates `web`, which names the
renderer and no more.

What crosses between them is declared in one module. Those types are counts,
names and paths — converted from the domain **on the server**, by code that is
itself compiled only there. A field added to one of them is a field the browser
gets, and the type is the only place to check.

The feature is called `server` because the server-function macro emits
`#[cfg(feature = "server")]` literally. That is a contract with the framework
rather than a naming preference, and renaming it silently moves every server
function's body to the client.

**The gate builds both halves.** `check_matrix` gains a wasm line, so a client
that does not compile fails the same command that everything else fails. It
excludes the four internal crates by name: they are not for the browser, and a
crate added later that is also not for the browser fails loudly until somebody
says which side it is on. That is the intended default — the alternative is a
list that silently stops covering things.

`just check` needs no `dx` and no bundle. The wasm pass is `cargo clippy`
against a target, which is a toolchain fact; building an actual bundle is not,
and the gate stays runnable on a machine that has only a toolchain.

## Rejected: one crate, `cfg` alone, no optional dependencies

The obvious shape, and it is what the `cfg`s in the source look like they are
already doing.

It loses on the mechanical point above: the dependencies would still be built.
Every crate in this workspace, and the cipher, would have to compile for wasm
and keep compiling for it — a constraint on **core** imposed entirely by a
decision about the dashboard, which is exactly backwards from the dependency
direction `docs/architecture.md` §1 sets out.

## Rejected: a fifth crate holding the shared view types

The tidy answer, and the one to reach for if the view grows: a `view` crate
depended on by both halves, with the conversions in the app.

It loses **for now** on cost rather than on principle. It adds a crate, a
manifest, and a place for the dependency rules to be got wrong, in exchange for
a boundary the feature already draws. The rule it would enforce — the browser
sees only these types — is enforced today by there being one module and by the
gate compiling it for wasm.

Revisit when something other than the app needs those types, which is the point
at which a crate is buying something rather than describing something.

## Rejected: serving the domain types directly and trusting the redaction

`Secret` already refuses to render, per `docs/conventions.md` §4, so a project
serialised whole would not obviously spill anything.

It loses because it makes a page's contents depend on a redaction being right
in every type it transitively touches, forever, rather than on there being
nowhere to put a secret. `docs/conventions.md` §4 asks that secrets never
render *and* never serialise; this decision removes the question from the wire
entirely, which is the stronger of the two and does not need testing per field.

It would also drag the domain's shape into the view. A dashboard wants a count
of running jobs; the domain has a map of jobs keyed by identifier. Serving the
second and computing the first in the browser puts the deciding on the wrong
side of the boundary.

## Consequences

The app crate has two entry points and one binary. `dx` compiles the same
target twice, so `main.rs` forks on the feature and holds nothing else;
everything either half does is in the library.

**A `cargo build` binary has a server and no client.** That is a working thing
to run rather than a broken one: the page is rendered on the server and arrives
complete, it just does not come alive afterwards. Anything that expects a
bundle beside the binary and does not find one serves the application without
static assets and says so on startup, rather than failing.

Lint expectations have to be written per half, and one of them cannot be
written on the item at all: the server-function macro re-emits doc comments and
drops every other attribute, so an expectation belonging to the generated
client code lives at module scope. That is recorded where it is written, and it
is the kind of thing that costs an hour to rediscover.

Reversing means making the internal crates unconditional and deleting the wasm
line from `check_matrix`. Cheap in edits, and it would be noticed immediately —
by the build, on the first dependency that cannot compile for a browser.

Revisit when a second thing needs the view types, when the view grows past one
module, or if a future dashboard genuinely needs something the domain knows and
this rule forbids — at which point the question is what to *add* to the wire
types, not whether to send the domain.
