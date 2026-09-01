# 0039 — A release is a tagged binary, and never a package

## Status
Accepted.

## Context

`docs/decisions/0038-the-browsers-half-lives-in-the-binary.md` made what ships
one file. Nothing yet says how that file reaches anybody, or what it is once it
has.

Two facts shape the answer.

**The binary cannot say what it is.** The manifest version reaches exactly one
place — the tool server's own description of itself, which only an agent ever
reads. Nothing is printed at startup and there is no flag to ask. A distributed
binary that cannot be identified makes "which build is this?" unanswerable, and
a bug report unattributable.

**A package cannot carry a browser half.** Building one needs `dx`, and a
registry builds with `cargo` alone. So a package install produces a binary with
no client: a dashboard that renders and never responds, which
0038 records as the failure most easily mistaken for success. Whatever else is
true, that channel cannot produce a working product.

## Decision

**A release is cut by pushing a tag, and produces binaries. This project is
never published to a registry.**

- **Publication is refused mechanically**, not by convention: every package
  carries `publish = false`, and every version is `0.0.0`. The first stops
  `cargo publish`; the second says why anybody looking at the manifest should
  not expect a number to mean anything.
- **The version is implanted at build time and belongs to the tag.** Nothing in
  the source names a version, so there is no second place for one to disagree
  with. A build told nothing produces a binary that says so.
- **What it is told is validated when it is told.** A build given a version and
  no commit fails rather than producing a binary that quietly reports itself as
  no release at all.
- **The target is derived rather than supplied**, because it is the one part of
  a build's provenance the build itself knows and whoever invoked it could get
  wrong.

Rejected: **publishing on every merge to main.** Zero ceremony, and the cost is
not runner time — this repository is public. What it costs is that every merge
becomes a promise nobody decided to make, and that a version has to be
synthesised from a commit because no human named one.
`docs/decisions/0002-never-merge-never-deploy.md` puts a person at the boundary
where work leaves the system, and calls that boundary the reason the rest is
safe to leave running; this is the same boundary one level out.

Rejected: **a rolling prerelease overwritten on every merge.** It buys "the
latest is always downloadable" and gives up the thing a release is for — after
it, "I am running the main build" names nothing that can be looked up.

Rejected: **carrying the version in the manifest and asserting the tag matches
it.** That was the design until `publish = false` made a manifest version
meaningless. Two places holding one fact needs a check to keep them honest;
one place needs nothing.

Rejected: **stamping the build's own date.** It answers when a job happened to
run rather than how old the code is, and it makes a rebuild of one tag produce
a binary claiming a different date — so a version would name two things. The
commit's date is a property of the release rather than of the moment it was
built.

Rejected: **carrying the commit message.** Unbounded prose, in a string that
goes in a log line and a page.

## Consequences

**Only Linux is built.** Not a scope decision so much as Apple's: linking for a
Darwin target needs a software development kit whose licence permits use on
Apple hardware only, so a Linux runner cannot produce a macOS binary however
well it cross-compiles in the other direction — which this project does, and
proved, in the direction that is freely redistributable. Somebody on macOS
builds their own, which they can, because they have the toolchain that runs it.

**Every target is named**, and `host` is never used for something published: a
file whose name depends on which machine happened to build it is a file nobody
can identify later.

**A release binary is one file with a name that says what it is** —
`stageman-<os>-<arch>` — and the release attaches it under the version too.

**Adding an architecture is four lines**: a target the toolchain installs, a
cross linker the runner installs, one variable naming that linker, and one name
in the build recipe. The linker stays out of this repository for the reason
`docs/conventions.md` §5 gives, and continuous integration proves that reason
rather than working around it — a tracked linker entry for the runner's own
triple would break the very job that builds the release.

**Reversing** is deleting a workflow and a module. Nothing is persisted, and no
binary already downloaded stops working.

**Revisit if** macOS becomes a target somebody downloads rather than builds,
which needs a second runner and cannot be done any other way; or if a registry
ever becomes able to build the browser half, which would make a package a real
channel rather than a trap.
