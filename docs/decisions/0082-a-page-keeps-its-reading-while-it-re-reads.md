# 0082 — A page keeps its reading while it re-reads

## Status

Accepted. Amends
`docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`, which said
that a page re-runs its read on every tick and not how: the read runs in
the background, the page keeps its last reading until the next has landed,
and nothing in the browser is ever suspended.

## Context

The browser's console logged *cannot reclaim ElementId*, from the
framework's own arena, many times over while a dashboard was open beside a
running job. The line means an element id freed twice, and the framework
logs it rather than failing, so a page that logged it looked fine.

Every page read through the framework's server future, which is what
carries a reading made on the server into the page, and each read followed
the ticks of 0071 so that it re-ran on every write. Measured, with the
framework's core instrumented and a tick fired at a page in a headless
browser, that combination did this on every tick:

- The server future suspends the page whenever its read is pending, so on
  a tick every page suspended, and the status line under it suspended
  with it — two suspended reads under one boundary.
- The dashboard declares no suspense boundary, so a suspended page lands
  on the one the framework wraps every application in. That boundary
  answers a suspension by moving its children out of the document: every
  element id in the page is freed, every node removed, and the mounts
  kept with the freed ids in them. The page was blank until the slowest
  read had landed — a few milliseconds beside the instance, a visible gap
  behind a slow proxy — and then rebuilt from scratch, which is where
  focus went on every tick, why the Kickoff card's disclosure closed on
  every tick, and why the tagged heading of a page was a different element
  after each.
- When one read landed while the other was still out, the page rendered
  again in the background, without a writer. Anything that render removed
  — a badge gone, a row gone, a control that a standing no longer offers
  — freed the ids of what it removed a second time, and the framework
  logged each. A page beside a job whose rows change on every write logged
  a handful per tick.

The framework's tracker carries this as an open report against its 0.7
line, its number 5025, and a guard for it only on the unreleased 0.8 line.
So the log line is the framework's, and the teardown is the framework's
answer to a page that suspends; what is this project's is that a page
suspended on every tick when it had a reading to show.

One more thing was measured on the way and is worth keeping: the
framework's wasm layer writes every level, errors included, as a styled
`console.log` line. A browser check that counts the console's own error
entries or `console.error` calls counts none of it, which is why every
earlier check reported no errors on pages that logged one per element.

## Decision

**A page's read is made once with the page and again on every tick in the
background, and the page keeps its last reading until the next has
landed. Nothing in the browser is suspended: a page with nothing yet shows
its skeleton, and a page whose address carries a parameter is keyed by it,
so another address is another page.**

- **One hook, in the shell's module.** Every page reads through it, and it
  is built on the framework's loader rather than its server future: the
  loader suspends once, on the first read, and reloads in the background
  after, which is precisely the contract a page under ticks wants. The
  hook suspends only on the server, where waiting is how the reading gets
  into the page; in the browser it answers *not yet*, and the page is
  rendered again when the reading lands.
- **The skeleton arm is live.** Every page already drew its skeleton for a
  reading it did not have, and no page ever reached that arm: the browser
  either hydrated a reading or suspended. A page reached within the
  browser now shows it until its first reading lands.
- **A page is its address.** The three pages whose address names a project
  or a job render their screen under a key made of it. A page keeping its
  reading while it re-reads would otherwise show the last project's jobs
  under the next project's address for the length of one read, which is
  the exact thing `use_reactive!` was there to prevent; a key makes the
  next address a fresh page, with a fresh read and none of the last page's
  state — a settings page no longer carries one project's draft to
  another's.
- **A route that changes something still shows its answer at once.** The
  loader is writable, and a page that holds a newer listing writes it in,
  as it did before, rather than waiting for the tick that follows.

Rejected: **a suspense boundary around the page slot, with the skeleton as
its fallback.** The framework's own advice, and it would keep the shell in
place. But the skeleton would then flash on every tick, since the page
would still suspend on each, and the second free would still happen on
every navigation in this version of the framework: the page's placeholder
is created with an id, freed when the boundary moves it out, and freed
again when the page resolves in the background. Isolating the damage is
not removing its cause.

Rejected: **keeping the server future and stashing its handle.** A hook
that keeps the resource handle from a render that had one and reads its
value on a render that suspended would work, and it would be a copy of
what the loader already is, kept in this project instead of the framework.

Rejected: **carrying a patched core crate.** The framework's guard on its
0.8 line could be ported. It would put this project on a fork of a pinned
dependency, against `docs/conventions.md` §3's reason for the pin, and it
would leave the teardown on every tick exactly as it was, since the guard
silences the line and not the cause.

## Consequences

**A tick costs the page a diff of what changed** rather than a rebuild of
the document: focus, scroll, selection and the open disclosures survive
it. The error card and the skeleton are reachable in the browser now, so
what each says is worth reading as a person would.

**A read's failure reaches the page as its words.** The loader carries an
error as a captured one, its message and no more, where the server future
carried the typed error; no page ever read more than the words, so nothing
is lost, and a page that ever needs the kind of failure has the wire's
refusal to ask for.

**Reversing** is one hook, nine call sites and three keys; nothing is
persisted and nothing crosses the wire differently.

**Revisit if** the framework's loader changes its contract and suspends
after the first load, which the hook's tests do not see and a browser check
under ticks does; or if the framework guards the second free and the
teardown on its own, when a boundary around the page slot becomes the
cheaper way to a loading look on navigation than the skeleton arm — though
the arm would still be right.
