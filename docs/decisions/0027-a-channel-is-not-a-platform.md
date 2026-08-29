# 0027 — A channel is not a platform, and does not share its map

## Status
Accepted.

## Context

`docs/decisions/0005-conversation-happens-on-channels.md` makes Slack the
escalation path rather than an optional integration, and
`docs/open-questions.md` puts it at the head of the queue. Before any of it can
be built the domain needs somewhere for a channel to live, because a project
today has no field one could go in.

The cheap move is already sitting there. A project carries
`credentials: BTreeMap<Platform, Secret>`, the handout carries the same map, and
an adapter turns each entry into an environment variable. Adding `Slack` to
`Platform` costs one variant and one match arm, and every other piece —
sealing, opening, delivery, the dashboard's credential route — starts working
without being touched.

## Decision

**Channels are a second closed set with their own map, their own binding type,
and their own place in a handout.** A project holds
`channels: BTreeMap<Channel, ChannelConfig>` alongside its platform
credentials, and never inside them.

A `ChannelConfig` carries two things rather than one: the credential, and an
`address` — the Slack channel a project's conversation happens in. It is called
that, and not `destination`, because `docs/conventions.md` §2 defines a channel
as two-directional and rejects *source* and *feed* for naming only one
direction. `destination` makes the same mistake pointing the other way: the
orchestrator watches that address as much as a job posts to it.

Rejected: **`Platform::Slack`, one map for both.** Two things break, and the
first is the one that matters.

*The two handouts differ, and one map cannot say how.*
`Handout::for_triage` deliberately carries no platform credential at all —
triage judges signals rather than acting on them — while an orchestrator that
cannot reach a channel cannot watch one, which is the whole of its remit per
`docs/architecture.md` §1. Sharing the map forces a choice between handing
triage every platform credential it has no business holding, or handing it no
channel credential and leaving it unable to work.

*A binding is not one value.* The address has nowhere to go in a
`BTreeMap<Platform, Secret>`, so this shape needs a second parallel map for it
anyway — keyed by a variant that is meaningless for `GitHub` and every platform
after it. That is the same two maps, minus the type telling anybody which
combinations are real.

## Consequences

Two maps on a project, two on its sealed form, two in a handout and two loops in
the adapter that delivers one. That duplication is the price, it is visible, and
it is what keeps `for_triage` able to state what it means.

Reversing this is small today and stops being small quickly: merging the maps
means a snapshot migration, since both are serialised, plus re-deciding what the
orchestrator is handed. Doing it before Slack ships costs an afternoon; doing it
after a project has bound one costs a migration for every instance.

Revisit if the orchestrator ever needs a platform credential, or a job ever
needs to act on a channel the way it acts on a platform. Either one collapses
the distinction this record is built on, and at that point one map is the
smaller thing. Revisit also if a project needs two bindings for one channel —
two Slack workspaces, most obviously — which the map's key forbids by
construction and would need a different shape rather than a second entry.
