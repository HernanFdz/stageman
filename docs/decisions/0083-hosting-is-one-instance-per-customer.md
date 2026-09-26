# 0083 — Hosting is one instance per customer

## Status

Accepted. Amends `docs/vision.md` §2 and §3: the non-goal that the instance
is not multi-tenant stands, and the constraint that it runs on infrastructure
the operator controls gains two delegations an operator may choose and take
back. Answers the question a managed offering asks of this repository, which
is what the binary must gain for it — two things, each a record of its own:
`docs/decisions/0084-the-instance-authenticates-itself.md` and
`docs/decisions/0085-the-instances-apps-may-be-held-elsewhere.md`. Parks a
remote container runtime as an open question rather than deciding it.

## Context

Somebody will pay to have stageman run for them: a domain, a certificate,
updates, backups, a job's work reachable at a public address, and nothing
to install. The question is what shape that takes, and the shape decides
what this repository has to change.

Two facts about the code decided it, both read from the source on
2026-09-26 rather than assumed.

**The instance is the target, not the containers.** Every credential an
operator has entered is in one process's memory in the clear, in one file
under one key, behind three surfaces that face the network: the door with
its tunnel proxy, the tools endpoint with the credential route, and the
Slack sockets. The containers are isolated by construction; the process
holding every customer's secrets would not be. Rust removes memory bugs and
not logic bugs, and
`docs/decisions/0034-tools-are-served-not-shipped.md` already records that a
bug in the warrant check is a privilege escalation — in a shared instance it
would cross customers.

**A remote container runtime is three network paths, not a flag.** The
world spawns the runtime with the daemon's own environment minus the
`STAGEMAN_` names, so a host named in the runtime's own variable already
redirects every command the instance renders, the exec that carries the
conversation and the build fed from standard input included; docker's SSH
transport refuses a password in the address, runs its own client on the
remote, and shares one connection through the user's ssh configuration. But
two paths are host-to-host today. The tunnel port is published on the
runtime host's loopback, and the probe of
`docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
and the proxy connect to this machine's; over a network the probe's budget
inverts, so a slow close reads as answering and a container is never
stopped. And every container reaches the tools endpoint and the credential
route through the host gateway name, which on a remote runtime is the
remote host — the case
`docs/decisions/0034-tools-are-served-not-shipped.md` names as turning a
link between neighbours into a real network service.

`docs/vision.md` §2 had already weighed the alternative: isolating tenants
costs more than running a second copy. And
`docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
had refused an App owned by this project's maintainers, because one key able
to mint tokens for every operator's repositories in one place is exactly what
a shared instance's App would be.

## Decision

**A hosted offering runs one unchanged instance per customer, on a machine
provisioned for them that they never see, and the instance stays what it
is: one operator's, single-tenant, holding its own credentials and deciding
everything.** Hosting is a copy, not a tenant.

- **The binary does not know it is hosted.** The installer already takes a
  domain and, with one set, the instance spells every address as https
  without a port; the tunnel and the dashboard route by that domain. The
  domain in the environment is the whole of what differs between a laptop
  and a hosted machine, and it stays the only place that difference lives.
- **Two delegations, both the operator's to choose and to take back.** The
  machine may be somebody else's to provision, which is this record. And an
  app's secret may be somebody else's to hold and act with on the instance's
  behalf, which is
  `docs/decisions/0085-the-instances-apps-may-be-held-elsewhere.md`. Neither
  moves a decision out of the instance.
- **The machine is the boundary between customers.** A container escape
  lands in that customer's own machine, holding that customer's own data.
  The open question on a job running containers of its own already records
  that a shared kernel is an escape away; a machine per customer is the
  answer to it that costs nothing in this repository.
- **The file key is the host's to inject**, through the service manager, as
  `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md` already
  says a deliberate deployment does. A copied disk is then useless without
  the host, moving a customer between machines is copying a file, and
  leaving is being handed the file and the key.
- **What the binary gains for hosting is two things and nothing else**: it
  authenticates itself, because an instance facing the open Internet cannot
  rely on a proxy the operator did not put there, per 0084; and its apps
  may be held elsewhere, so that a hosted customer gets a one-press install
  without a secret of the host's ever entering their machine, per 0085.

Rejected: **organisations and users inside the instance.** The natural
first sketch, and the largest change this repository could take: every
request variant, every view, the routing that shares a Slack workspace
among projects, the uniqueness of watched rooms and of job names, the file's
shape, and about a hundred and sixty places the state is touched, each
gaining a check. Its worst cost is not the size: the instance's own App and
Slack app become one key for every customer, which 0077 refused; and one
process aborts for everyone on a bad day.

Rejected: **one shared instance with each customer's containers on a remote
runtime.** It keeps the shared instance as the target, and it needs the
remote runtime's three paths besides. What it uniquely offers — this
project's dashboard on somebody else's compute — nobody has asked for.

Rejected, for now: **the remote runtime as a feature on its own.** Real,
and parked: the control path is free, the other two are a record-sized
design, and no hosted customer needs it. `docs/open-questions.md` names the
three paths so that it is not re-derived.

## Consequences

**`docs/vision.md` changes in two places**, in the same commit: the
non-goal in §2 gains the sentence that a hosted offering is a copy, and §3
gains the two delegations and what each costs to take back.

**A cap on concurrent jobs becomes wanted.** Nothing limits how many jobs a
foreman has working at once, and a small machine overcommits into an abort.
That is the first thing a hosted plan would size, and a self-hosting
convenience besides; it is an open question rather than part of this
record.

**Two open questions become pressing**: where a log line goes, because a
hosted customer's failure is read by somebody who does not sit at the
machine; and the flight recorder, because a hosted failure captured as the
exact event sequence replays as a scenario, which is a support capability
this architecture gets for nearly nothing.

**Nothing here is a change to code.** The two records this one points at
are, and each carries its own cost.

**Reversing** is a sentence in the vision, because nothing in the binary
knows.

**Revisit if** a customer asks for the dashboard hosted and the compute
their own, which is the remote runtime arriving with a reason; if a machine
per customer stops being cheap against the work a job does, which is the
scale at which sharing would pay; or if a second person needs to operate one
instance, which is the trigger several records already name and is a
feature of the instance rather than of hosting.
