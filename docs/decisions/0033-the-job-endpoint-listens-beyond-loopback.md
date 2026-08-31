# 0033 — The job endpoint listens beyond loopback

## Status
Accepted. Completes `docs/decisions/0032-a-foreman-asks-the-instance-by-warrant.md`,
which decided how a foreman is authorised and deliberately left where it is
heard undecided. Where it listens is unchanged by
`docs/decisions/0034-tools-are-served-not-shipped.md`; *what it serves* is not —
see that record for why "one route is served there and nothing else" below no
longer holds.

## Context

A foreman creates jobs by asking the instance, which means the instance has to
be reachable from inside a container. Where it listens is not a detail, because
the answer differs by platform. Measured on this machine, and the difference is
the whole problem:

| bind address | Docker Desktop | Linux |
|---|---|---|
| `127.0.0.1` | reachable from a container | **not reachable** |
| the bridge gateway | no such address on the host | reachable |
| `0.0.0.0` | reachable | reachable |

Linux is where continuous integration runs and where anything deployed will
run. So designing against what a Mac does would produce a foreman that creates
jobs on a laptop and cannot in the place that matters — which is the shape of
failure this project keeps meeting, and the reason
`docs/conventions.md` §3 records a container runtime's path rather than
searching for it.

## Decision

**A listener of its own, on `0.0.0.0`, serving one route.**

Its own, because the dashboard's stays on loopback. Sharing one would drag the
dashboard out with it, and `docs/open-questions.md` still has authenticating
that as an open question — this decision must not answer it by accident.

`0.0.0.0` because it is the only address reachable on every platform. That is
the whole reason, and it is not a preference.

**One hostname reaches it from either runtime.** Containers are created with
`--add-host=host.docker.internal:host-gateway`, which Docker and Podman both
honour — measured — so nothing has to know which runtime is in use.

**The address is written into the container rather than set as a variable**,
like the thread and for the same reason: the port can be named by the
environment, so an instance restarted with a different one would otherwise
leave every existing container asking somewhere nothing is listening.

Rejected: **binding loopback and reaching it some other way.** On Linux that
means `--network host`, which gives the container the host's network namespace
and gives up the isolation
`docs/decisions/0012-agents-run-in-containers.md` rests on. A foreman that can
create jobs is not worth a foreman that can see every socket on the machine.

Rejected: **discovering the bridge gateway and binding that.** It is the
narrowest address that works on Linux, and it does not exist on Docker Desktop,
varies with custom networks, and differs again under rootless Podman. Three
platform-specific paths to maintain, each failing somewhere nobody tested.

## Consequences

**A port is open on every interface**, and on a laptop that includes whatever
network it is joined to. Three things stand behind it and only the first is a
barrier: the warrant, which nothing but a foreman's container holds; a refusal
of any peer that is not loopback or a private address, which removes routed
traffic outright and is not a security boundary; and that one route is served
there and nothing else.

It is printed at startup for that reason. Nobody needs to configure it, and
nobody should have to discover it by reading source — the same argument that
put the count of channels on that line.

The port is a constant with an environment override, read once per process like
the runtime. Not configuration in any meaningful sense, but a port can collide
with something already running, so there is a way out that costs nothing when
it is not needed. An unreadable value falls back rather than failing: a
mistyped port should not stop an instance starting, and the startup line makes
the fallback visible.

Revisit if stageman ever runs somewhere with untrusted neighbours on the same
network — a shared host, a corporate LAN with no client isolation. The warrant
holds there too, but the calculus of an open port changes, and the answer is
probably the narrow bridge-address binding this record rejected as too
platform-specific for a laptop.
