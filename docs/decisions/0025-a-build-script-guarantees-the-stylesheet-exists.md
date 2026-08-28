# 0025 — A build script guarantees the stylesheet exists

## Status
Accepted.

## Context

The dashboard needs CSS. Tailwind produces it, `dx` compiles it — fetching the
Tailwind binary itself, so no package manager is involved — and the output is a
build artefact, gitignored like any other.

Attaching it runs into two facts that only look compatible.

**`asset!` resolves at compile time and fails when the file is absent.**
Measured rather than assumed: pointing it at a path nothing had generated
reported *"Asset at … doesn't exist"* and the crate did not compile. So an
`asset!` aimed at a gitignored artefact makes a fresh clone fail to build
unless something generates it first.

`.quality/generated-paths` exists for that arrangement and states the condition
in its own words: an entry "is a promise that the listed command runs before
the compiler needs the file", and one whose command is not part of `just check`
is "a latent fresh-clone failure". Here `dx` is that command, and it is not
part of the gate — `docs/conventions.md` §5 keeps that to a toolchain and a
container runtime, and a Tailwind compile is neither.

**The obvious way out does not exist.** The plan was to skip `asset!`, put the
output in the configured asset directory, and link it by URL — nothing at
compile time, nothing to promise. That was built and then measured, and the
measurement killed it: a file placed in that directory **is not copied into the
bundle**. Assets are bundled because something referenced them, not because
they sit somewhere. The page linked a path that was never served, and the
server answered with the page again.

## Decision

**The stylesheet is an ordinary compile-time asset, and a build script
guarantees the file is there.**

The dashboard references it with `asset!`. `app/build.rs` creates an empty one
if none exists and **never overwrites**, because `dx` writes the real
stylesheet to that path before invoking the compiler and a later plain build
must not blank it.

This satisfies the allowlist's condition honestly rather than by exemption: the
thing that produces the file before the compiler needs it is a build script,
which runs in every build including `just check`. The entry is a promise that
is kept.

An empty stylesheet is not a fallback hiding a failure. It is what a build with
no browser half *should* have, and it produces a page that is complete and
unstyled — the state that binary was already in and already reports at startup,
per `docs/decisions/0022-the-browser-never-sees-the-domain.md`.

## Rejected: linking the stylesheet by URL from the asset directory

The design this record was first written to describe, before it was run.

It loses on a fact, not an argument: the asset directory is not copied into the
bundle. Kept here because the reasoning was sound and the premise was false,
and because the next person will have the same idea — assets are bundled by
being *referenced*, and a directory of files nothing references is copied
nowhere.

## Rejected: tracking the compiled stylesheet

Makes `asset!` work, keeps a fresh clone building, needs no build script.

It loses because a tracked artefact that nothing regenerates goes stale the
first time somebody adds a class and does not rebuild. The symptom is a
component silently unstyled everywhere except the machine that wrote it. A
checked-in artefact with no check that it is current is worse than an absent
one.

## Rejected: building the stylesheet as part of the gate

The honest version of doing nothing: put `dx` in `just check` and the problem
evaporates.

It loses on what the gate is for. It runs constantly and its cost is meant to
track the size of the code; a Tailwind compile and a binary download are
neither. It would also make `dx` a hard prerequisite for the gate.
`docs/decisions/0023-the-container-runtime-is-discovered-once.md` took one such
prerequisite deliberately, and the argument that carried it was that the
program cannot *work* without a runtime. It works fine without a stylesheet.

## Consequences

**This crate has a build script, and that is a cost.** It runs on every build
of the package. It is the smallest one that could work — an existence check and,
at most, one write.

A page served by a `cargo`-built binary is unstyled, and the tests assert on
its text rather than its appearance. They already did, and that is the right
thing for them to assert on regardless.

**The gate now creates a file.** `just check` on a fresh clone leaves an empty
stylesheet behind. It is gitignored and listed in `.quality/generated-paths`,
so nothing is surprised by it.

Reversing means deleting the build script and choosing one of the rejected
options above — each of which is a paragraph of edits and a different cost.

Revisit if `dx` ever bundles a directory as well as a reference, which would
make the first rejected option work and remove the build script. It is the
tidier shape and it is one upstream change away.
