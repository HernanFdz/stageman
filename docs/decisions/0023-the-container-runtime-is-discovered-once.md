# 0023 — The container runtime is discovered once, and is required

## Status
Accepted. Supersedes
`docs/decisions/0017-the-runtimes-path-is-recorded-in-the-instance.md`, and
narrows `docs/decisions/0021-an-instance-starts-empty.md`.

## Context

0017 put the runtime's path in the instance, asked for on a first run and
verified at startup. 0021 then removed the first run, so nothing asked; the
path stayed in the snapshot as something the dashboard would one day set, and
an instance without one started anyway on the reasoning that an instance with
nothing configured is empty rather than unusable.

Two things are wrong with where that left it, and they pull in the same
direction.

**It is the only machine-specific value in a portable file.**
`docs/decisions/0011-state-is-a-snapshot-not-a-database.md` sells the snapshot
on being copyable: take the file, carry it, supply the key. An absolute path to
a program is exactly what a different machine makes wrong, and 0017 accepted
that cost explicitly — a restored snapshot "may name a runtime that is not
there".

**It is not a choice anybody wants to make.** An operator does not have a
preference about where `docker` is; they have one about whether stageman works.
0017 saw this — its own rejected-alternatives section keeps discovery alive "as
a suggestion", proposing what was found and letting the operator override. That
was written when a first run existed to do the proposing. Without one, the
suggestion has nowhere to happen, and what is left is a field somebody must
fill in by hand before anything runs.

The rule underneath both is unchanged and is not in question:
`docs/conventions.md` §3 forbids locating the runtime by searching `PATH`,
because an inherited variable differs between the shell you tested in and what
a service manager supplies. That rule is about `PATH`, not about discovery. **A
fixed list of absolute paths compiled into the binary is deterministic in
exactly the way `PATH` is not** — 0017 says so itself.

## Decision

**The runtime is found from a list of absolute paths compiled in, once per
process, and a machine without one cannot run stageman.**

The list is per-platform and ordered, and the order is a decision: Docker
before Podman, package-manager locations before system ones. A machine with
both gets the first, and that is the price of not asking.

Discovery itself is a pure function in the agent crate, taking the list and
returning what it found. **The policy is a lazily-initialised static in the
binary, and it holds a runtime rather than an optional one.** When there is
none it prints what it looked for and stops the process; startup forces it
before anything else, so that is the first thing that can fail and the last
thing that needs saying.

The split is deliberate: *where a runtime might be* is knowledge about
container runtimes and belongs in the library, while *what to do when there is
none* is a decision about a program that cannot run and belongs in the program.
It also leaves discovery testable — a function taking its list can be asked
about the empty case, which a static reading the real list on a machine that
has Docker never can.

**Nothing records it.** The field is gone from the state, from the sealed
snapshot, and from the dashboard's view of the instance. The snapshot now holds
only work, so it is portable in the way 0011 claimed rather than nearly.

**A missing runtime is fatal at startup, and this is the narrowing of 0021.**
That record's argument — that an instance with nothing configured is empty
rather than unusable — was about things an *operator* configures, and a runtime
is no longer one of them. Every agent runs in a container, including the one
triage thinks with, so an instance without one cannot do anything at all. It is
a missing prerequisite, like a missing libc, and not an empty field.

Being found is not being usable, and both are checked. `verify` asks the
runtime for a version, which reaches the daemon — so a client installed with no
daemon behind it, which looks perfectly healthy to anything that merely looks
for the file, fails at startup rather than on the first job.

## Rejected: the static holds an optional runtime and each reader branches

Tried first, and wrong, and worth recording because the argument for it sounds
right. It went: a panic out of a lazy initialiser cannot carry the error report
this binary already builds — the message and every cause under it and a chosen
exit code — so hold the absence and let startup turn it into a proper failure.
Nothing else, the argument continued, would ever have to handle it.

**That last claim was false, and the code disproved it within the hour.** The
dashboard's own route reads this value, and holding an optional one made it
invent a string — "none found" — for a state startup has already proved
impossible. One reader, one meaningless branch, and every reader after it would
have added another.

The premise was wrong too. Nothing stops the initialiser printing before it
stops the process, and that is what it does: the operator sees the sentence and
the list of paths, in the same place they would have seen any other startup
failure. What is lost is the exit code, which becomes a panic's rather than a
chosen one, and the panic's own line after the useful text. That is a small
price for a value no reader has to interrogate.

The one property genuinely given up is that a missing runtime is no longer a
`StartupError` variant alongside the others, so it is not reported through the
same chain. It does not need to be: every other startup failure is something an
operator might repair in place, and this one means the program cannot run here
at all.

## Rejected: keeping the field and using discovery only as a default

0017's own surviving suggestion, updated for a world with no first run: fill
the field from discovery, let the dashboard change it.

It loses because it keeps the machine-specific value in the portable file for
the sake of a case nobody has met — a runtime somewhere none of the compiled-in
paths name. When that case appears it will be a new path on the list, which
helps everyone on that platform, rather than a field one operator fills in.
Records the cost honestly: somebody with a runtime in an unusual place now has
no way to say so, and their fix is a pull request rather than a text field.

## Consequences

**`just check` now requires a container runtime.** Seven integration tests
start the binary, and the binary now refuses to start without one, so a machine
with only a toolchain can no longer run the gate. `docs/conventions.md` §5 says
so, with installation instructions, and the continuous integration workflow
builds the image before running the gate rather than after. This is a real cost
and it was taken deliberately: the alternative was a runtime that is required
in production and optional in the tests, which is the kind of difference that
is discovered late.

**The container tests stop being a separate step.** They were ignored because
`just check` had to run without a runtime; that reason is gone. They move into
`just verify` rather than `just check`, because building the image is a
container build costing minutes and a network — a present runtime is not a
built image, and the constantly-run gate stays proportional to the code.

**One test could not survive.** A configured runtime that runs and refuses used
to be tested by pointing the binary at `/usr/bin/false`, and there is no longer
any way to point the binary anywhere. The check moved to the agent crate,
against `verify` directly, which is where it could always have been.

Reversing means putting the field back and reading it before consulting the
static. The snapshot format changes in both directions, and an instance written
by either version is readable by the other, because what is at stake is a field
being absent rather than a format changing shape.

Revisit when a machine appears that keeps its runtime somewhere the list does
not name — the answer is a longer list — or if stageman ever runs somewhere the
runtime is reached over a socket rather than by executing a program, which is a
different design and not a longer list.
