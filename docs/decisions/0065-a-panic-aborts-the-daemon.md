# 0065 — A panic aborts the daemon

## Status

Accepted. Follows
`docs/decisions/0015-a-job-survives-the-daemon-dying.md` and
`docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`, which
make the daemon dying a thing this project survives, and
`docs/decisions/0056-the-instance-decides-and-the-world-performs.md`, whose
shape is what made an unwinding panic worse than a death.

## Context

The lints deny every explicit panic in this workspace's source, and the
release profile keeps one panic path on purpose: `overflow-checks` turns an
overflow the lints could only warn about into a panic, as the backstop. The
dependencies and the standard library keep their own. So a panic in the
shipped binary is rare and designed, and never impossible — the question is
what the daemon does when one happens.

What it did was worse than dying. The instance steps on a task the world
spawns and never joins, in `world/src/lib.rs`. An unwinding panic there is
caught by the async runtime and stored in a handle nobody holds; the task
ends and nothing observes it. The process stays up: the listener accepts,
every later event is logged as lost, and a server function waiting on an
answer waits for ever, because the sending half of its channel stays in the
map of the waiting in `app/src/world.rs`. A dashboard that renders and never
responds, under a service manager that sees nothing wrong. The doc comment
on that wait promises `None`; the code hangs.

Measured on the server binary as `cargo` builds it, without the browser's
half, which neither setting touches: unwinding, 14.6 MB; aborting, 10.4 MB;
aborting with the symbol table stripped as well, 8.1 MB. The test build was
measured too, because `cargo` documents that a test harness ignores the
setting: the harness and the crates it depends on are built unwinding, and
the binary the integration tests run is built aborting. A profile that
aborts therefore compiles every crate the harness and the binary share
twice, once per strategy.

## Decision

**The release profile aborts on panic.** The panic hook still prints the
message and the backtrace; then the process is gone, which is the kill 0015
and 0045 already survive and the one a service manager restarts. A panic is
a correct program halting, and an abort is the halt.

**Release only.** The test harness unwinds whatever the profile says, and a
development profile that aborts would compile the shared crates twice on
every run of the gate for a binary it never ships. Under `just dev` a panic
still unwinds, where somebody is watching the log.

**The symbol table stays.** `strip` keeps the level it had: debug
information goes and symbols do not, because once a panic ends the process
the log it printed is the only evidence, and the symbol table is what makes
the backtrace in it readable. A panic's own file and line survive any
stripping, being a static string; the chain of calls that led there does not.

Rejected: **unwinding, and joining the loop's task** so that its panic is
raised again in the main thread. That fixes the one task and leaves every
other the world spawns — a wake, a read, a run, a connection — swallowing its
own panic exactly as before. One policy for every task, costing no code, is
better than a policy per task, and the runtime's own switch for the same
thing is behind an unstable flag.

Rejected: **catching the panic and stepping on.** The instance's state after
a panic part-way through a step is one nobody has reasoned about, and
continuing from it is the incorrect program continuing that the panic rule
in `AGENTS.md` forbids.

Rejected: **aborting in every profile.** The cost is in the gate, on every
run, and the harness would unwind regardless.

Rejected: **stripping the symbol table alongside.** The size it saves is
metadata and not code, and it is the metadata the only evidence needs.

## Consequences

**Nothing may rely on catching a panic.** A join error that says a task
panicked, a poisoned lock, anything wrapped to catch an unwind — each is a
branch the shipped binary never takes and a development build does, which
is the divergence `docs/conventions.md` §3 now names.

**Destructors do not run on a panic.** Already the case on a kill, which
nothing of this project's runs on, under the bar `docs/conventions.md` §4
sets for what a kill may leave behind: a container the instance can still
name, and nothing else.

**The browser's half is compiled under the same profile** and changes
nothing there: that target has no unwinder.

**Reversing** is one line in the manifest.

**Revisit if** something here ever needs to catch a panic — a boundary with
code this project does not trust, such as a plugin — or if the loop grows a
supervisor that restarts a stepped instance from its snapshot, at which
point unwinding is the mechanism it needs and the reasons above want
re-reading.
