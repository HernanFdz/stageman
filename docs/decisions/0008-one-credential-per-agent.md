# 0008 — One credential per configured agent

## Status
Accepted

## Context

Agents authenticate differently, and there is no arrangement in which one
credential serves all of them. Within a single agent there can also be more
than one way to pay: a subscription, or a per-token key billed by usage.

Two facts make the handling of this delicate rather than routine.

The first is that at least one agent resolves credentials from its environment
by precedence, and prefers a per-token key when one is present. An operator who
intends to run on a subscription, on a machine where an API key happens to be
exported, gets per-token billing with no error and no warning — the difference
appears on an invoice weeks later.

The second is that a subscription's credential is normally ambient: a login
belonging to the machine's user, held wherever that platform keeps secrets. On
one developer's desktop that is invisible and convenient. It is also
unreachable from a container, absent on a server, and different on every
operating system — which would quietly make the choice of credential a function
of the choice of isolation mechanism, two decisions that have nothing to do
with each other.

## Decision

Each configured agent carries exactly one credential, and every process that
runs that agent is handed **constructed** credential material: exactly what
that agent needs in order to authenticate as itself, and nothing belonging to
any other. Nothing is inherited from the environment stageman itself was
started in.

"Material" rather than "variables" is deliberate, and was learned rather than
foreseen. One agent takes a token from an environment variable; another keeps
its subscription credential in a file and expects to find it at a path. How it
is delivered differs per agent and is an adapter's business. What does not
differ is that this project decides what a process gets, and that nothing
arrives by accident.

Because exactly one credential is ever present, precedence between credential
kinds is never consulted and cannot silently change underneath us.

There is no orchestrator-versus-job dimension to this. An agent is configured
once and whatever runs it — triage or work — uses that agent's credential.

The operator enters that credential once and it lives in the encrypted store
from then on. It never has to exist in the environment of the machine this runs
on, and the agent never has to be logged in there: obtaining the credential is
a one-time step performed wherever the operator happens to be, and only its
result is handed over. That is what makes a headless host no different from a
desktop, and it is the practical payoff of refusing ambient auth below.

Where an agent offers a headless subscription credential, that is the default
path. Claude Code's is a long-lived token minted by `claude setup-token`, which
works from a container and on a server and requires no platform keychain
support at all.

Rejected: **one instance-wide credential.** Cannot work once agents come from
different vendors.

Rejected: **reading the host's ambient login.** It works beautifully on the
machine it was developed on and nowhere else, and it couples auth to isolation
for no reason.

Rejected: **setting the intended credential and relying on documented
precedence to ignore the other.** Correct until the day the precedence changes
or an operator exports something in a shell profile, and wrong silently and
expensively when it does.

## Consequences

Stageman now stores long-lived credentials rather than merely reading them,
which is what `docs/decisions/0004-one-encrypted-sqlite-file.md` already
provides for.

A job's environment contains a credential **by construction**, so the first
invariant in `docs/architecture.md` §2 needs its scope stated rather than left
to be read generously. That is done there, not here.

Credentials expire, and one will expire while nobody is watching. The behaviour
that failure should produce is not yet decided — see
`docs/open-questions.md`.

The blast radius of a leak is bounded but real: an agent credential buys
somebody else's work on the operator's account. It buys no access to the
repositories or channels, because those credentials are held elsewhere and by
different rules.

Revisit if an agent worth supporting offers no headless credential at all. That
would force a choice between excluding it and reintroducing ambient auth for
that one adapter, and the second option should not be taken quietly.
