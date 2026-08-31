# 0036 — A foreman's image is not a job's

## Status
Accepted. Depends on
`docs/decisions/0035-an-image-is-built-never-named.md`, which is what makes a
second image cost nothing to ship.

## Context

One image serves both halves of `docs/architecture.md` §1. It carries the
agent's adapter, and then a certificate bundle, a download tool, `git` and the
repository host's own tool — everything a job needs in order to reach a
repository, because nothing outside the container delivers one.

A foreman has no repository. It has no workspace at all: `docs/conventions.md`
§2 says a workspace belongs to a job, and triage is not one. And
`docs/decisions/0027-a-channel-is-not-a-platform.md` gives its handout no
platform credential whatsoever, deliberately — watching a channel is its remit
and acting on a platform is not.

So a foreman's container ships a tool for reaching a repository host and
nothing that could authenticate it. The image and the handout disagree, and the
handout is the one that was designed.

Measured, because the split is only worth making if the halves genuinely
differ. `node:22-slim` ships no `git`, no download tool, no repository tool and
**no certificate bundle**. The recipe installs all four in one layer — and
installs the adapter over HTTPS *before* that layer, which is the proof that
the runtime brings its own roots and never needed the bundle. Everything in
that layer exists for a job.

## Decision

**One recipe per agent, two images: what a foreman thinks with, and what a job
works in.** The job's image is the foreman's plus the layer that reaches a
repository, expressed as two stages in one file and selected by target.

Which image a container gets is decided by what that container is for, and is
never configuration. A foreman that could be given a job's image would be an
operator's opportunity to hand triage a capability its handout holds no
credential for.

Rejected: **one image for both**, which is what exists today. It costs nothing
to keep and it is the cheaper thing to have written. What it costs is that the
narrowing in 0027 stops at the process boundary: a handout carrying no platform
credential, delivered into a container that can reach a platform, is a bounded
blast radius rather than a closed door — and
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` already spent
that argument once and records what it cost.

Rejected: **two recipes, one per role.** The job's is the foreman's plus a
layer, so two files would duplicate the head and let the halves drift on the
version they pin — which is the failure that pinning exists to prevent.

## Consequences

**A second build, and almost no second image.** The stages share every layer up
to the split, so the job's image costs the layer that reaches a repository and
nothing else, and building it after the foreman's is a cache hit throughout.

**A foreman's cold build gets much shorter** — no package index, no certificate
bundle, no release download from a repository host. That is also the first
container an operator ever starts, which is where minutes are most expensive.

**Least privilege here becomes structural rather than asserted.** A foreman's
container cannot run the repository host's tool because it is not installed,
which is a stronger claim than a credential it was not given:
`docs/conventions.md` §4's escape test is evidence about what a container
permits, and this narrows what there is to permit.

**Reversing** is deleting a stage. Nothing is persisted, and the only thing that
outlives the change is a long-lived foreman's container, which would be
recreated and would discard its session — the same cost 0034 already records
for the same reason.

**Revisit if** a foreman ever needs to reach a platform, which is the trigger
0027 already names for collapsing the distinction underneath this one; the two
records fall together. Revisit also if an agent appears whose adapter needs the
tools a job needs, which would make the two stages identical and leave nothing
to split.
