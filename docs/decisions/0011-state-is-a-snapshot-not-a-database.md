# 0011 — State is a snapshot, not a database

## Status
Accepted. Supersedes `docs/decisions/0004-one-encrypted-sqlite-file.md`. The
credential handling in that record is carried forward unchanged; what is
superseded is the choice of an embedded database as the store.

## Context

The state this system keeps is small, and its shape is still moving: projects
with their channels and credentials, jobs with the reason they exist, the prompt
they were given, the agent that ran them, and enough to resume them. One process
owns all of it, because the app crate runs the orchestrator in-process, so there
is no second writer to coordinate with. It is read constantly and written on
discrete events a person could count.

A database earns its cost through queries, concurrent writers, partial loads and
transactions. None of those are in evidence here. Its schema-and-migrations tax,
meanwhile, is paid from the first commit — at exactly the point the shape is
least settled and changes most.

## Decision

All application state lives in memory while the process runs, in one structure.
That structure is serialised to a single file and read back at startup; when no
file exists the instance starts empty, and that is a first run rather than an
error.

Serialisation is serde, and the format is JSON. Self-describing, greppable,
diffable and hand-editable beats compact while the shape is still moving, and
changing the format later is close to a one-line change.

Two amendments are what make this safe rather than merely tidy, and neither is
optional:

**The snapshot is written on state change, not at shutdown.** Writing only on a
clean exit would survive a clean exit and nothing else, and
`docs/vision.md` §3 already commits to the opposite: the process *will* be
killed underneath running work, by a reboot or an upgrade or a closed lid, and
surviving that is a requirement rather than an edge case. A shutdown-only
snapshot loses every project, every job and every agent session on a panic or a
power cut, which turns in-flight work unresumable. Each write is atomic —
temporary file, flush, rename — so a crash during one cannot truncate the last
good state.

**Credentials remain encrypted at rest**, under a key supplied by the
environment at startup, exactly as 0004 required. A plain snapshot would put
tokens in cleartext on disk, which is a regression rather than a simplification.
This composes with the redaction bar in `docs/conventions.md` §4: the wrapper
type that gives a credential its redacting formatting is the same place its
encrypting serialisation belongs, so the two properties are added together or
not at all.

Rejected: **an embedded database.** It buys queries, partial loads and
transactional writes, none of which this needs, and charges a schema and a
migration path from the first commit onwards.

Rejected: **snapshotting at shutdown only.** Simpler and quite wrong — see
above.

## Consequences

Reads are free, since state is already in memory. Writes rewrite the whole file,
which is nothing at the scale in `docs/vision.md` §3 and would not stay nothing
forever.

**Reversal cost is asymmetric, and the asymmetry is the thing to understand.**
Changing the *format* is trivial — serde makes JSON to a binary encoding close
to one line. Changing the *architecture* is not: "the whole state is a structure
in memory" is an assumption that spreads into every access, none of which is
written to expect a query, a partial load or a transaction. Adopting a database
later is not swapping a backend, it is revisiting every read. That is the bet
being taken, and it is being taken deliberately at a scale where it is cheap.

Nothing here versions the snapshot, and that gap is taken knowingly rather than
overlooked. Being rid of migrations was part of the point, but a serialised
structure evolves too: an added field is free with a default, while a rename or
a removal makes an existing file fail to load — and failing to load means losing
all of it, since there is only the one file. For now the answer is to delete it
and start again, which is honest while the only thing it holds is a proof of
concept and indefensible the moment it holds something an operator would miss. A
version field is the intended fix; recognising when that moment has arrived is
the hard part, and the trigger is the first time somebody would rather migrate
than start over.

Revisit when any of three things becomes true: job history stops fitting
comfortably in memory, rewriting the whole file per change stops being cheap, or
a second writer appears — the last of which would most likely arrive as somebody
wanting to run two instances against one state, and is worth refusing on other
grounds first.
