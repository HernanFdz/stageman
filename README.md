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
It serves a dashboard and does the watching in the same process — there is no
second daemon to supervise and nothing to install alongside it.

The dashboard is where you add projects and set the credentials each one needs,
watch jobs and read their logs, and pause or kill one that has gone wrong. A
single instance manages several projects, and a job belongs to exactly one of
them: it cannot see another project's repository, credentials or channels.

All state lives in one `SQLite` file, with credentials encrypted at rest under a
key supplied by the environment at startup. Back up the file and you have backed
up the instance; take it to another machine without the key and it tells you
nothing.

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
