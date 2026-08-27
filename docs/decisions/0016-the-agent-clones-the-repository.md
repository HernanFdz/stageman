# 0016 — The agent clones the repository; nothing delivers it

## Status
Accepted. Answers the workspace-delivery question
`docs/decisions/0012-agents-run-in-containers.md` left open, and changes what
the word *workspace* means in `docs/conventions.md` §2.

## Context

0012 settled that a job runs in a container and explicitly deferred how the
repository's files get inside one, calling it an implementation question. It
also recorded the one measurement that bears on it: bind-mounted filesystem
performance is meaningfully worse on a platform whose containers run in a
virtual machine, which is most developer machines.

Three ways were available. Mount the repository from the host. Put it in a
volume the runtime manages. Or have nothing deliver it at all, and let the
job's agent clone it with the platform's own command-line tool, using the
credential
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` already gives
it.

The first two were being compared on performance and, after
`docs/decisions/0015-a-job-survives-the-daemon-dying.md`, on whether the files
survive a restart. Both are the wrong question.

## Decision

Nothing delivers a repository. A job that needs one clones it inside its own
container, with the platform's own tool and the credential it already holds.

## Rejected: mounting the repository from the host

Slower where it matters most, per 0012's measurement. It also makes the host's
filesystem part of a job's boundary, which is exactly the boundary 0012 chose a
container to draw — a mount is a hole in it, and the isolation invariant in
`docs/architecture.md` §2 would then rest on the mount being scoped correctly
rather than on the container.

## Rejected: a volume the runtime manages

Faster than a mount, and it survives a restart as readily as the container's
own layer does. It loses on a second lifetime to manage beside the container's:
volumes outlive the containers that used them and are orphaned easily, which
would add a second kind of leak to the one 0015 already has to sweep for.

## The flaw both share, which is the one that decides it

Either mechanism makes a repository **mandatory**. Every job would have a
checkout because the machinery gives it one, whether or not the work needs it.

That is not merely wasteful, it is untrue to what a job is. A job that settles
whether some third-party platform supports a feature, or reads an API's
documentation to answer a question a channel is waiting on, has no repository
in it at all — and today that job cannot be expressed. Cloning is the only
option of the three where *not having a repository* is an ordinary case rather
than an empty mount.

It also sits correctly with 0009, which already has a job reaching platforms
through those platforms' own tools rather than through anything hosted here.
Provisioning a checkout on the agent's behalf would be this project doing by
mechanism the one thing 0009 says the agent does with its credential.

## Consequences

**The vocabulary changes, and §2 is updated in the same commit.** A workspace
was defined as the filesystem the repository is checked out into together with
the container around it. The filesystem is now *contents* rather than
definition: a workspace is the container a job runs in, and what is inside it
is the job's business. The invariant in `docs/architecture.md` §2 gets cleaner
rather than weaker — the boundary was always the container, and 0012 said so.

A job pays a clone. Small against a container start and against the work a job
does, and per job rather than amortised — the same shape of cost 0012 accepted
for the start itself, and worth stating for the same reason.

Cloning needs the network, so the egress allowlist deferred in 0009 must permit
the repository host. That sharpens the open question rather than blocking it:
the list now has a member nobody has to guess at.

The credential's scope does not widen. It already had to clone, push and open a
pull request, which 0009 records as one credential precisely because those are
the same one.

**A kickoff prompt now has to say whether there is a repository and what to do
with it.** Instructions are authored in one crate — `docs/architecture.md` §1
— so this lands entirely in the orchestrator, and it is covered by the
snapshot-tested prompt bar in `docs/conventions.md` §4 rather than needing
anything new.

Reversing means adding a mount, which is small. What is not small is that jobs
without a repository would have to be forbidden again, and by then some will
exist.

Revisit if cloning stops being cheap — a very large repository cloned per job
is the shape of that. The answer then is a shallower clone or a cache the
agent is pointed at, which is a change to what a prompt says rather than to
this decision.
