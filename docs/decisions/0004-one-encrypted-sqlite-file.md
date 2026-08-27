# 0004 — One encrypted SQLite file

## Status
Superseded by `docs/decisions/0011-state-is-a-snapshot-not-a-database.md`. The
credential reasoning below is carried forward by that record unchanged — what it
supersedes is the choice of an embedded database as the store.

## Context

An instance manages several projects, and each carries credentials for the
platforms it watches. Those credentials are editable from the dashboard, which
means this project stores secrets rather than merely reading them.

`docs/vision.md` §3 fixes the environment: a long-lived process on a machine the
operator owns, quite possibly headless and quite possibly not the machine it was
configured on.

## Decision

One SQLite file holds everything — projects, jobs, reasons, prompts and
credentials. Credentials are encrypted at rest with a key supplied by the
environment at startup, so the file is portable and useless without it.

Rejected: **state in SQLite, secrets in the OS keychain.** The cleanest
separation, and it removes key handling entirely. It lost on the deployment
shape: keychain access differs per platform and is awkward-to-hostile on a
headless box, which is precisely where a self-hosted daemon ends up living.

Rejected: **state in SQLite, credentials read from the environment only.** The
simplest and hardest to get wrong — the database could then never leak a secret
it does not hold. It lost because it contradicts a requirement rather than a
preference: editing a project's credentials from the dashboard is part of what
managing several projects in one place means, and this would demote that surface
to read-only status reporting.

## Consequences

Key handling, rotation, and the question of what happens when the key is lost
are now this project's problem. Losing the key means re-entering every
credential; it does not mean losing job history, which is the reason those are
stored separately inside the same file rather than encrypted wholesale.

Encryption protects the file and nothing else, which is why "no secret is ever
written to a log line" is a house rule in `docs/conventions.md` §3 and why
redaction is a testable bar in §4. A token escapes through a formatted struct
long before it escapes through a database file.

Cheap to reverse while the schema is small — moving credentials out to a
keychain or the environment later is a migration and a code path, not a
redesign.

Revisit above the point where one file stops being enough: concurrent instances
against shared state, or a job history large enough that queries matter. Both
are a long way from one operator and a handful of projects, and neither is
worth pre-paying for.
