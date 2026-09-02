# 0042 — A job shows its work on a subdomain

## Status

Accepted. Adds the first inbound route into the containers
`docs/decisions/0012-agents-run-in-containers.md` created, and answers the
question `docs/open-questions.md` held about the dashboard needing
authentication before it left `127.0.0.1` — that entry is removed in the same
change. Read the consequences before the decision: who authenticates is the
expensive half of this record, and the tunnel is the cheap one.

Names below that this project does not yet define are unbackticked
deliberately, for the reason
`docs/decisions/0034-tools-are-served-not-shipped.md` gives about protocol
names: `just drift` resolves a backticked identifier against this source, and
a record written before the code it governs would otherwise fail the gate for
being early rather than for being wrong.

## Context

A job's work is invisible until it is proposed. Everything this project does
ends at a pull request — `docs/decisions/0002-never-merge-never-deploy.md` —
and a diff is a poor way to answer *does this look right?* about anything with
a rendered output. The person who has to review it wants to look at the thing
running, and increasingly wants to look at it *while* it is being built rather
than after.

Three measurements decided the shape, and the first one removed the obvious
design outright.

**A port mapping cannot be added to a container that already exists.** `docker
update` has no publish flag and `docker port` only lists. A job's container is
created once, at job start, and retained across turns and across daemon
restarts per `docs/decisions/0015-a-job-survives-the-daemon-dying.md` — so by
the time a job's agent could ask for a tunnel, the container that would need
the mapping already exists and cannot be given one. Recreating it to add one
discards the session, which is the thing 0015 exists to preserve.

**The host port is not durable, and the two runtimes disagree.** One container,
`-p 127.0.0.1::7100`, stopped and started:

| runtime | first start | after stop/start |
|---|---|---|
| Docker 29.4.1 | `127.0.0.1:64383` | `127.0.0.1:64389` |
| Podman (applehv) | `127.0.0.1:42539` | `127.0.0.1:42539` |

Docker reassigns; Podman keeps. So anything persisting a host port is correct
on one runtime and silently wrong on the other — and wrong on the one
continuous integration uses, in the resume path, which is the least observed
code in the system.

**Wildcard `localhost` resolves at the system resolver, not in the browser.**
Measured through getaddrinfo on macOS: `abc.localhost` and
`0d8e1f2a-3b4c.localhost` both answer `127.0.0.1` and `::1`, and a name under
no such rule fails. That matters because it decides whether a default is
honest: this is not a Chrome and Firefox convenience that Safari lacks, it is
the platform resolving the name, so a browser-shaped caveat would have been
wrong.

## Decision

**Every job's container publishes one port at creation, and a job is reachable
at `<job-id>.<domain>`.**

- **One port, one constant, decided at creation.** The container-side port is a
  hard-coded number in the 47_2xx family the endpoint's own default already
  uses, published with `-p 127.0.0.1::<port>` so the runtime picks the host
  side atomically. Unusual rather than familiar on purpose: publishing does not
  bind the port inside the container, so the risk is not a collision but an
  agent that runs a dev server on 3000 to check its own work and finds it
  published.

- **The host side is asked for, never recorded.** The runtime is the authority
  and answers correctly on both, where a snapshot is wrong on one. Nothing
  about a tunnel is persisted at all: the container port is a constant, the
  host port is derived, and which jobs have a tunnel is which jobs have a
  container.

- **The domain is an environment variable read once per process**, defaulting
  to `localhost`, alongside the port variable in `app/src/endpoint.rs` and for
  the same reason. Not a field on the snapshot: `core/src/lib.rs` already
  argues this case where the runtime's path used to live — a snapshot is meant
  to be portable, and a value describing *this host's reachability* is exactly
  what another machine makes wrong.

- **Routing is by the Host header, on the dashboard's listener**, in front of
  the framework's router rather than as a route inside it. The apex is the
  dashboard; anything whose bottom label parses as a `JobId` is that job's
  tunnel. One listener, one port, one thing for an operator to point infra at.

- **Job-only.** A foreman has no workspace and nothing to serve, which makes
  this the mirror of the tool `docs/decisions/0034-tools-are-served-not-shipped.md`
  offers only to a foreman.

- **The kickoff says the port and says to bind every interface**, because a
  server bound inside the container to loopback is unreachable through a
  published port and the agent will verify it with curl and see it working.
  That paragraph belongs with the others in `foreman/src/lib.rs` and is
  snapshot-tested per `docs/conventions.md` §4.

- **Websocket upgrades are forwarded**, not merely proxied. Watching work
  evolve means a dev server with hot reload, so an upgrade that is passed
  through as an ordinary request produces a page that renders once and never
  moves — which is the failure this feature exists to prevent, wearing the
  costume of a working page.

Rejected: **a tool that opens a tunnel on demand.** The design this record
started as, and the first measurement above killed it rather than out-argued
it. Worth recording because it is what anybody would propose next: it is not
that a per-job port is tidier, it is that the runtime cannot do the other
thing.

Rejected: **persisting the host port on the job.** The second measurement.
Correct on Podman, wrong on Docker, and wrong in the resume path.

Rejected: **moving to Podman because of that divergence.** It reads as a
decisive reason and is not one: asking the runtime costs one call and works on
both, and under this design the host port never appears in a URL, so its churn
is unobservable. `docs/open-questions.md` already says what would settle
Podman-only — whether this project's adapter works there at all, which has
never been exercised — and this is not it.

Rejected: **a minted, revocable identifier for the tunnel** instead of the
job's own, at the cost of one defaulted field on a job. It was proposed to buy
two things, and authentication in front of everything took one of them back
before this record was finished: an unguessable identifier defends nothing
that a proxy refusing unauthenticated requests has not already defended. What
it would still buy is an off switch, and that is the honest remaining
argument — declined for simplicity, and named under consequences rather than
argued away.

Rejected: **a third listener of its own.** Symmetrical with
`docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`, which
refused to share the dashboard's, and rejected here because that record's
reason does not carry: it kept a separate listener to avoid dragging the
dashboard off loopback, and this decision moves the dashboard deliberately. A
second public port would be a second thing to forward and a second thing to
protect, for a boundary that is no longer being defended.

Rejected: **making the feature conditional on a domain being configured.** The
argument for it was that the port would then be published only where it could
be used. It does not survive the publishing being loopback-only: reachability
from anywhere else is entirely a function of whether infra forwards
`*.<domain>`, so a conditional publish buys a pair of idle loopback ports per
running job and nothing else. Against that, an unconditional feature has one
URL shape, one code path, and no branch that exists to serve a configuration
nobody wants.

## Consequences

**Authentication is the infra's, for the whole domain, and this project has
none.** `docs/open-questions.md` used to ask whether the dashboard needed
authentication before it left `127.0.0.1`, and answered *the default protects
it*. This record spends that default, so the question is settled here rather
than left open: whatever forwards `*.<domain>` authenticates every host under
it — the apex and every subdomain alike — and stageman authenticates nothing.

Uniform rather than apex-only, which was the first shape proposed and is
wrong. It treats the dashboard as the sensitive half, and a tunnel carries
whatever a job's agent put on it — a build of an unreleased thing, a database
viewer, a page rendered from a repository this instance holds credentials for.
There is no reading on which that deserves less protection than a list of
project names.

Rejected with it: **authenticating in this project.** It buys independence
from whichever proxy an operator chose, and it costs a login page, a session,
and a credential store, in a process whose entire security model to date is
that it is not reachable. The proxy already does this correctly and is being
deployed anyway.

**The defence is real and lives outside this repository**, which is the part
to state rather than assume. A job identifier is not a capability *because
something authenticates in front of it* — not because the identifier is
secret, and not because of anything in this source. Reaching `127.0.0.1:8080`
directly bypasses it, as does widening the dashboard's bind address, so both
are now security-relevant where neither was before. This is the one place
where a property this project relies on is enforced by configuration a fresh
clone cannot see, and it is recorded here because
`docs/conventions.md` §4-style guarantees do not apply to it: no test can
defend it.

**There is no way to close a tunnel** short of removing the container, which
discards the job's session. A container is retained after its job goes idle,
and when a finished job's container is removed is still open, so a job that
showed something in the morning is still serving it at midnight.

**An instance with no domain set advertises the reader's own machine.** The
default is honest on the operator's laptop and a trap on a deployed instance
where nobody set the variable: the agent says to look at
`http://<job-id>.localhost:8080`, and the person's browser resolves that to
their own loopback. It fails as a wrong answer rather than as an absent
feature, so the domain in use goes on the startup block in
`app/src/serving.rs` beside the other facts, where a wrong one is visible at
boot rather than at the first thing anybody was asked to look at.

**The installer has to be told the domain, and cannot ask.** `README.md`
installs by piping a script into `sh`, so stdin is the script and a prompt
would either hang or eat the rest of it. It arrives as an argument, in the
shape `--uninstall` already establishes, and is written into the
`Environment=` block of the unit or the plist that `packaging/install.sh`
already generates.

**The Host header is the whole mechanism, and it fails quietly.** A proxy that
rewrites it sends every tunnel request to the dashboard, so the person sees the
dashboard where they expected their application and nothing anywhere says why.
Which header is authoritative has to be decided rather than assumed, and a
bottom label that parses as an identifier belonging to no job is worth a
diagnostic rather than a silent fallthrough.

**Wildcard TLS needs DNS-01.** A certificate for `*.<domain>` cannot be issued
over HTTP validation, which is the ordinary path and the one somebody will try
first. It is infra rather than this project's, and it is the part most likely
to be discovered late.

**Reversing** costs the publish flag, the routing layer and a kickoff
paragraph, with no data migration, because nothing about a tunnel is stored.
The container of every job running at the time keeps a published port that
nothing routes to, which is inert. Cheaper than most things here, and
deliberately so.

**Revisit if** stageman ever has to run somewhere its operator cannot put a
proxy in front of it, which is the single assumption the whole
authentication answer rests on; if a job ever needs more than
one tunnel, which is the ceiling the single constant port buys the simplicity
with; if a tunnel needs to outlive its job's container, which is the point at
which the derived-not-stored property stops holding; or if an operator needs to
revoke one, which is when the minted identifier rejected above becomes the
right answer rather than the more expensive one.
