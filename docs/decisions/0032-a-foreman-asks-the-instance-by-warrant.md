# 0032 — A foreman asks the instance by warrant

## Status
Accepted. Decides how a foreman is *authorised* to create jobs; where the
endpoint listens is deliberately not decided here and is open in
`docs/open-questions.md`.

## Context

A foreman that can only talk is not much use. What makes it a foreman is that
it can assign work, and `docs/architecture.md` §1 says the app crate is the one
allowed to name both the deciding and the doing — so creating a job means the
foreman asking the instance for one.

The shape of the asking was decided against the alternative first. A foreman's
turn could have *answered* with what it wanted done, and the daemon acted on
it: no network, no new mechanism, and closest to what §1 already describes.
That was rejected on a failure it cannot handle — nothing guarantees an agent
ends its turn with parseable structure, and a turn that ends in prose has done
nothing with no recourse. A command either works or hands the agent an error it
can act on, which is error handling rather than ergonomics.

So a foreman runs a command that reaches the daemon. Which raises the question
this record answers: **what stops anything else doing the same?**

Not much, it turns out. Measured rather than assumed: an unrelated `alpine`
container reaches a service on the host exactly as easily as a foreman's image
does, on both Docker and Podman. Being in a container this instance started
proves nothing, and every job's agent runs in one.

## Decision

**A warrant**: a secret held per project, presented with every request a
foreman makes of this instance, and delivered only to a foreman's handout.

`Handout::for_foreman` carries it and `Handout::for_job` does not. That is not
a precaution, it is the whole mechanism: a job's container is never given one,
so a job's agent cannot create jobs however well it reaches the daemon.

Not called a *token*, and the near-miss is worth recording because it was the
obvious name. This codebase already uses that word for what an agent
authenticates with, what a job reaches a platform with, and what a channel is
posted on — three unrelated things. A warrant authorises one bearer to ask one
thing of *this instance* and never leaves the machine, which none of the others
describe.

Minted lazily, when a foreman first needs one, rather than when a project is
created. Every project that already exists has none, and a project whose
foreman never runs never needs one.

Rejected: **no authorisation at all**, on the grounds that the endpoint is only
reachable from inside. It is reachable from every container on the machine,
which is the measurement above.

Rejected: **one secret for the whole instance.** Cheaper, and it puts every
project's authority in one value — the concentration
`docs/decisions/0020-the-orchestrator-belongs-to-a-project.md` exists to
prevent, arriving by a different door. Per project means a leaked warrant costs
one project.

Rejected: **reusing the agent's credential.** It is already in the container,
so it costs nothing to check against — and it authenticates a *model provider*,
not this instance, so a foreman and a job hold the same one. It would authorise
exactly the thing this is meant to withhold.

## Consequences

It is a credential, so it is sealed in the snapshot and redacted when
formatted, like every other. It is also the first credential this project
*mints* rather than being given, which is why it comes from the same source the
crate already uses for identifiers rather than introducing an encoding and a
generator for one field.

**Rotation is unaddressed.** A warrant lasts as long as the project, and
changing one means the foreman's container holds a stale value in an
environment that cannot be changed — so rotating means recreating the
container, which discards the session. That is a real cost and nothing needs it
yet.

**Where the endpoint listens is not decided here**, and it is the harder half.
The daemon binds `127.0.0.1`, which containers reach on Docker Desktop and do
not reach on Linux, where CI runs — measured. A warrant makes it safe to listen
somewhere reachable; it does not choose where.

Revisit if a job ever needs to ask this instance for something. The answer is
probably a warrant of its own with a narrower remit rather than sharing this
one, and at that point "a foreman's warrant" becomes "a warrant, scoped".
