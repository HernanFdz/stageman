# 0017 — The container runtime's path is recorded in the instance

## Status
Accepted. Extends the first-run flow in
`docs/decisions/0013-an-instance-is-configured-before-it-exists.md` by one
question.

## Context

`docs/conventions.md` §3 has said from the start that the container runtime's
location is configuration and never a lookup on `PATH`, and it says why with a
measurement: of two agents installed while this was being designed, one sat in
a directory absent from a non-interactive shell's search path. Anything a
daemon finds by searching therefore works perfectly when tested by hand and
fails when a service manager starts it — the worst of the available outcomes,
because the two runs differ in something nobody was looking at.

What the rule did not say is *where* the path is recorded. Nothing needed one
until now: the agent crate takes it as a parameter, and its tests supply it. A
binary has to get it from somewhere.

## Decision

The path is part of the instance, kept in the snapshot beside everything else
the instance knows, asked for on a first run and verified at startup.

## Rejected: an environment variable, like the encryption key

Superficially attractive because a key already arrives that way, and because a
machine-specific value sits oddly in a file whose whole appeal is that it can be
copied.

It fails on the reasoning that produced the rule. `PATH` was rejected because a
daemon's environment is not the environment you tested in, and a service manager
supplies a different one; moving the value to a *different* variable in that
same untrusted environment is the identical failure wearing a new name. The key
is not a precedent, because it is in the environment for a reason that does not
generalise: storing it beside the file it encrypts would defeat the encryption,
which has nothing to do with locating a program.

## Rejected: discovering it, and recording what was found

Check a fixed list of well-known absolute paths at first run and store the
first that exists. This is *not* a `PATH` search — a fixed list is
deterministic where an inherited variable is not — and it is rejected only as
the sole mechanism, because a machine that keeps its runtime somewhere unusual
would have no way to say so. It survives as a *suggestion*: the first run
proposes what it found and the operator accepts or overrides it, so the value
is still theirs and the common case costs a keystroke.

## Consequences

A first run asks a third question. 0013 named two — which agent, and what
credential — and `README.md` already speaks of *those questions* in the plural,
so this extends that flow rather than changing its shape.

**A snapshot restored on a different machine may name a runtime that is not
there.** It fails at startup, loudly, naming the path it looked for. That is
§3's existing rule applying rather than a new trap: a missing runtime is
already in the category that must refuse to start, because nothing works
without one and the dashboard could not repair it anyway. The repair is one
field in a human-readable file, which is among the things
`docs/decisions/0011-state-is-a-snapshot-not-a-database.md` chose that format
for.

`README.md`'s claim that backing up the file backs up the instance stays true,
and gains a caveat it did not have: the file is portable, and one value in it
describes the machine. Worth knowing before a restore rather than during one.

Reversing means reading the value from somewhere else, which is a few lines and
one field. What it would not undo is the first-run question, since an operator
who has answered it once expects it to have been remembered.

Revisit if a second machine-specific value ever appears. Two would be a
pattern, and a pattern deserves somewhere deliberate to live rather than being
scattered through a snapshot that is otherwise about the work.
