# stageman

An orchestration platform for sleepless coding agents.

A coding agent can take a well-scoped piece of work from a description to a
finished change. What it cannot do is start. Every unit of work waits for
somebody to notice it needs doing, judge that an agent could handle it, and sit
down to say so — which means the work that gets done is bounded by whatever
attention is left over, and overnight nothing happens at all.

The information needed to make that judgement is already there and already
written down: issues get filed, reviews get left, exceptions get reported,
people describe problems to each other in chat. stageman watches those channels
and closes the loop. When it decides something is worth acting on, it writes
down why, composes the instructions, and starts a **job** — one agent, in one
isolated workspace, on one project. If the job needs a human it asks on Slack,
not in a terminal nobody is watching, and stays alive while it waits.

## It never merges and never deploys

Work terminates at a proposal you review. That is not a setting and there is no
override, which is the point: because the boundary is absolute rather than
conditional, stageman never has to hold a credential that can land code or reach
production. The worst outcome of a bad decision made while you were asleep is a
proposal you decline.

## Running it

Running stageman means running one server executable on a machine you control.
It serves a dashboard and does the watching in the same process, so there is no
second daemon to supervise.

What it does need is a container runtime. stageman runs agents rather than
replacing them, and it runs each one inside a container built with that agent
already installed — so the machine itself needs no coding agent, no repository
tooling, and nothing particular on its path. That holds for the agent the
orchestrator thinks with just as much as for the ones doing the work.

A running stageman reads no credential from the machine's environment. Every
one of them is entered in the dashboard, held encrypted, and handed to a
container as it starts. Obtaining a credential is a one-time step you do
wherever you happen to be, and only its result goes into stageman.

**It asks you nothing to start.** A fresh instance has no agents, no projects
and no container runtime, and that is a perfectly good instance — it simply has
nothing to do yet. You give it those in the dashboard, in that order, because a
project needs an agent to think with and at least one its jobs can run on.

Starting it needs two things named in the environment: `STAGEMAN_KEY`, the
base64 key its file is encrypted under, and `STAGEMAN_STATE`, where that file
lives. Both describe where the instance *is* rather than what it does, which is
why neither can live in the file itself, and neither has a default because a
daemon started by a service manager inherits a working directory nobody chose.

What gets reported is `STAGEMAN_LOG`. It takes the same filter syntax as
`RUST_LOG` and defaults to `warn` — enough to see what needs attention, not a
commentary on things going right.

Which agents are configured is yours to decide, the orchestrator picks one per
job from what you have set up, and the dashboard shows which agent ran each
job. Where an agent can be paid for by a subscription rather than by the token,
that is the path stageman prefers.

The dashboard is where you add projects and set the credentials each one needs,
watch jobs and read their logs, and pause or kill one that has gone wrong. A
single instance manages several projects, and a job belongs to exactly one of
them: it cannot see another project's repository, credentials or channels.

All state lives in one human-readable file, rewritten whenever anything changes,
with credentials encrypted under a key supplied by the environment at startup.
Back up that file and you have backed up the instance; take it to another
machine without the key and it tells you nothing.

## Documentation

`docs/vision.md` is what this is for and what it refuses to be.
`docs/architecture.md` is the shape of the code and the invariants that hold it
together. `docs/conventions.md` is how work is done here, including the words
this codebase uses and the ones it deliberately avoids. `docs/decisions/`
records the choices already taken, each with the alternative it beat and what
would make it wrong.

## A note on the library target

stageman is an application. The library target exists so the binary has
something to test against and so this page has somewhere to live; its public
surface is whatever the binary happens to need, and it will change without
ceremony or a deprecation cycle. Depending on it directly is a mistake.
