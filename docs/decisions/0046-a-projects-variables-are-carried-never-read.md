# 0046 — A project's variables are carried, never read

## Status
Accepted.

## Context

A project integrates with things this system has never heard of. Its tests need
a payment provider's key in test mode, its build reads a private registry
token, its work has to reach a staging database. None of that is knowable here,
and all of it is ordinary.

Everything a job is handed today comes from a closed set. `Platform` has one
variant and `Channel` has one, and both are closed for the same recorded
reason: reaching one needs *code* — an adapter that knows which variable a
tool reads, an image carrying that tool, a decision about what the credential
authorises. An operator cannot invent a variant, because inventing one would
only postpone the failure to the moment a job runs.

What is being asked for has the opposite property. Nothing here reads it,
nothing here knows what it is for, and supporting one needs no code at all.
That difference is the whole of this record.

It was also anticipated rather than discovered.
`docs/decisions/0019-a-projects-tooling-is-the-projects-business.md` ends by
naming its own revisit trigger — *a credential needed at build time* — on the
grounds that such a thing cannot be declared statically, since declaring it
would mean writing it down. This is that.

**Two things were measured, on Docker and on Podman, and the two agree.**

*A name containing an equals sign is not a name.* Passing one as the name of a
variable to forward sets it inline instead, on both runtimes, so the value
travels in the argument list. That is the exact property the delivery code
exists to protect: a secret in an argument is a secret every user on the
machine can read out of the process table. So validating the name is not
tidiness — it is what keeps that sentence true.

*A name forwarded but never set says nothing at all.* Naming a variable that
nothing set produces a container without it: no error, no warning, no line
anywhere. That is already why the names forwarded and the values set are
derived from one list rather than decided twice, and this record does not get
to weaken it.

## Decision

**A project holds a third map, beside its platform credentials and its channel
bindings: names an operator chose, each with a value this project never
interprets, delivered into the environment of every container its jobs run
in.**

Six things follow. Each is the decision rather than a detail of it.

**They are opaque.** Nothing here parses a value, infers a platform from a
name, or behaves differently because of one. That is what makes this a separate
concept rather than a looser platform credential, and it is what 0019 requires:
stageman still decides nothing about what a project needs. It only carries what
the operator says.

**Every value is a secret.** Sealed in the snapshot, redacted when formatted,
and never rendered in the dashboard — which shows names alone, exactly as it
already does for platforms and channels.

**The name is validated where the domain can defend it, so delivery is total.**
A name is a non-empty run of letters, digits and underscores that does not begin
with a digit. Lowercase is deliberately allowed: the proxy variables an operator
will reach for first are spelled that way. A name entered by hand is refused as
it is entered, and a name a file carries is refused as that file is opened,
which is the work the boundary between a snapshot and a state already exists to
do.

**A name this project already delivers is refused.** An operator who sets
`ANTHROPIC_API_KEY` as a project's variable changes who pays, with no error and
no log line — which is precisely the failure
`docs/decisions/0008-one-credential-per-agent.md` was written to prevent,
arriving through a door that record could not see. Which names those are is an
adapter's knowledge and not the domain's, per `docs/conventions.md` §3, so the
refusal is assembled where both halves are visible: the adapter says what it
may deliver, and **app** — the only crate allowed to name both sides — asks.
The reserved set is every name any compiled-in adapter *could* deliver rather
than only those the project's current agents would, so that adding an agent to
a project later cannot turn a name that was accepted into a collision.

**Jobs only, never a foreman.** A foreman judges signals and touches no
repository, so a project's third-party credentials are the clearest possible
example of something it has no business holding. The asymmetry is the same one
`docs/decisions/0027-a-channel-is-not-a-platform.md` turns on, and a third map
keeps it expressible.

**A container's environment is fixed when it is created, and changing a
project's variables never reaches a job already running.** This is not a
limitation worked around; it is `docs/conventions.md` §2 applied to the
environment. Resuming is not retrying — a resumed job is the *same job
continuing* — and its environment is part of what that job is. A job that turns
out to need a variable nobody set is answered the way §2 already answers every
second attempt: a new job, with its own workspace, after the variable is added.

**The kickoff names them and never their values.** An agent that does not know a
variable is there will not reach for it, which would leave the feature doing
nothing; so one conditional paragraph names them, decided exactly as the
paragraph about speaking on a channel is. Values must never appear, and the
reason is structural rather than cautious: a kickoff is stored on the job and
crosses the snapshot boundary in the clear, because a job holds no credential.
A value in a prompt would write that value to disk unsealed.

## Rejected: another variant on the platform map

The cheap move, and the one 0027 already refused for channels. It fails here
for a sharper reason than it did there: a variant is code, and the entire point
of this concept is that no code is needed. Every third party any project ever
integrates with would need a variant, an adapter arm naming its variable, and a
release of stageman — which is a platform accumulating opinions about projects
it has never seen, the shape 0019 exists to refuse.

## Rejected: a per-variable flag saying whether it is a secret

It buys one real thing: an operator could read a non-secret value back off the
dashboard. It costs two. The operator's mistake is unrecoverable and silent — a
token marked ordinary is written to disk in the clear and printed on a screen —
and it doubles the surface, since two kinds of variable mean two render paths,
two serialised shapes, and a migration the first time anybody flips one.

Treating everything as a secret fails in the safe direction, which is the trade
`docs/conventions.md` §4 asks for everywhere else.

## Rejected: letting an operator change a running job's environment

Refused on the mechanics rather than on the effort. A container's environment
is fixed at creation, so honouring this would mean recreating the container —
which discards the workspace and the agent's session, and is therefore a new
job wearing the old job's name. That is exactly the confusion between resuming
and retrying that `docs/conventions.md` §2 spends a paragraph keeping apart.

## Consequences

**The blast radius of the risk in
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` widens, and
this record does not narrow it.** A leaked job environment used to cost one
repository token. It now costs whatever the operator loaded, for platforms this
project cannot name, cannot scope and cannot rotate. The comment in the
delivery code arguing that the narrowest version of that risk is holding less
stops being true in the commit that implements this. Both mitigations 0009
deferred — an egress allowlist, and credentials scoped per job — are worth more
after this than before it, and both remain in `docs/open-questions.md`.

**The fallback for a job that needed a variable nobody set costs less than it
first appears, and where it does cost something is not where one would guess.**
Said in the job's own thread, a value reaches that job's agent as the text of
its next turn and is written down nowhere: the reply is handed straight to a
resumed container, and a job that is mid-turn has its reply *discarded* with a
notice rather than queued. Nothing of it survives in this instance.

The exposure is one thread over. A message at the **root** of the channel is
for the foreman, and that one becomes an errand — held or queued, and
serialised into the snapshot in the clear, because a message from a person is
not a credential and the sealed form is built on that being true. So the
mistake worth naming is not using this fallback; it is using it in the wrong
place, which looks identical to whoever is typing and is the difference between
a secret that was never stored and one that is on disk unsealed. Slack's own
retention applies either way and is nobody's to fix here.

**Reversal is cheap in this repository and not for an operator.** Removing the
concept is a field, an adapter loop, a form section and a paragraph of prompt.
What it cannot undo is that every instance holding variables loses them, and
the projects depending on them stop working — the same asymmetry 0027 records,
one map further along.

**Revisit if any of three things becomes true.** If an operator needs a value
delivered somewhere other than an environment — a file at a path, which is how
`docs/decisions/0008-one-credential-per-agent.md` already describes one agent's
own credential arriving — then the name of this concept has outgrown it and the
map needs a shape rather than a string. If a variable is wanted across every
project rather than within one, that is a different invariant from the one in
`docs/architecture.md` §2 and needs its own argument, not a wider map. And if
operators start marking values as non-secret in their heads — a version number,
an environment name — because they cannot read them back, the flag rejected
above deserves a second hearing with that evidence behind it.
