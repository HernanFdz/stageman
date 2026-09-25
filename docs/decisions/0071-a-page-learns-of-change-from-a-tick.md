# 0071 — A page learns of change from a tick

## Status

Accepted. Answers the question `docs/open-questions.md` carried since the
first page was served: how a page finds out that something changed. Taken
beside `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`,
whose first page is a list of things that change while somebody is looking
at it. Amended by
`docs/decisions/0082-a-page-keeps-its-reading-while-it-re-reads.md`: the
read a page restarts on a tick runs in the background and the page keeps
its last reading until the next has landed, because a page that suspended
on every tick was measured to be torn down and rebuilt on each.

## Context

Everything a page shows is read once, while the page renders. A job that
finishes, a foreman that decides, a message that lands — none of it reaches a
page that is already open, and a job's standing on the development instance
stays what it was until somebody reloads. A console whose whole purpose is
the hours nobody is at the desk cannot be one that has to be refreshed by
hand when they return.

The mechanisms were known when the question was written: polling on a timer,
a stream from the server, or a socket in both directions. Two things have
been settled since and decide between them.

**Authentication is no longer a reason to wait.** The question noted that
anything long-lived is a connection somebody has to authenticate.
`docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` made the
operator's proxy authenticate every host under the domain, so a long-lived
request is protected by exactly what protects a short one.

**The world already knows the one moment a change is real.** Under
`docs/decisions/0056-the-instance-decides-and-the-world-performs.md` nothing
outward-facing leaves a step until the write of that step has landed, and the
world answers a write only once it has. So the completion of a write is the
moment after which a page's next read is guaranteed to see the change, and
the world observes that moment without knowing what was written.

Two facts about the plumbing were checked rather than assumed. The framework's
server functions can return a stream, consumed on the page as a stream. And
the forwarder `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
put in front of the framework hands a response body back as the stream it
arrived as, and carries an upgrade through, so a long-lived response reaches
the browser through it unbuffered.

## Decision

**The world keeps a version, bumps it when a write of the instance's file
completes, and a streaming route hands every bump to every open page as a
tick that carries nothing.**

- **A tick says that something may have changed, and nothing else.** No view,
  no identifier, no data: a page that receives one re-runs the read it already
  makes. So nothing can leak through a tick, and no view logic moves to a
  second place.
- **Bumps coalesce.** A page reads at most once per tick it consumes, and
  bumps that land while a page is still reading collapse into one, which is a
  property of the channel the version is kept in rather than a rule anybody
  keeps. A turn that writes often costs a page nothing more than reading at
  its own pace.
- **The instance has no part in it.** No event, no effect, no request: the
  tick is the world's, made from a mechanism the world already performs, and
  the instance can neither learn of a page being open nor be replayed
  differently for it. Every scenario is untouched.
- **A page says whether it is live**, with a mark in the shell, and reopens
  the stream when it ends, reading once as it does, so a tick missed between
  two connections costs one read.

Rejected: **polling.** Wrong at any interval, and a tick is cheaper than one
poll: nothing is sent while nothing changes.

Rejected: **pushing each view as it changes.** The world would have to know
which view a write concerns, which is domain knowledge in the crate that has
none, or the instance would have to say so with an effect, which is an effect
for the world's own bookkeeping in every trace of every scenario. A tick
carries no such question. Revisit below.

Rejected: **a socket in both directions.** Everything a person does already
goes through a server function; the second direction would carry nothing.

Rejected: **a version the instance persists.** The version is the world's
count of completed writes in this process's lifetime, and a page that
outlives the process reconnects and reads anyway.

## Consequences

**The shell reads the stream once for every page**, and each page's read is
what it restarts; a page adds nothing to be live beyond the read it already
has.

**The proxy in front of an instance must pass a streaming response through.**
One that buffers it delivers no tick, ever, and the page is the snapshot it
was before. The live mark is what says so, which is why it exists: a mark
that is missing is a proxy to look at.

**A route to test.** A write followed by a tick on an open stream, driven
through the binary as the routes already are.

**Reversing** is deleting one route, one hook and one channel; nothing is
persisted.

**Revisit if** a page grows a list long enough that re-reading it on every
tick is felt, which is when a tick wants to name what changed and the
rejected per-view push becomes the cheaper design; or if the instance ever
needs to know that somebody is watching, which would make a page an event
and this record wrong.
