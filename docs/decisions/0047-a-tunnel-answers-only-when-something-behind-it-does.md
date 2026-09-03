# 0047 — A tunnel answers only when something behind it does

## Status

Accepted. Repairs the mechanism
`docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
chose, and changes none of what that record decided. The rule is still that a
container lives while a turn runs in it or its tunnel answers; what was wrong
was *answering*.

## Context

Every job's container was being left running for ever, including the ones
showing nothing at all — which is most of them. Reported from ordinary use
rather than found by a test, and the containers accumulate silently, so the
first symptom is a machine with less memory than it had.

0043 defines answering as a connection this daemon can open to the port the
runtime published, and argues that asking from outside is "deliberately
stricter than looking for a process bound inside". The first half is right and
worth keeping. The second half is exactly backwards, and that is the whole
defect.

**A published port is not a bare port.** Both runtimes publish by putting a
proxy on the host side, and that proxy binds the host port when the container
is created and accepts every connection to it for as long as the container
runs — then discovers there is nothing inside to forward to, and closes.
Accepting is the proxy's, not the container's. So a probe that only connects
succeeds against every running container, `rest` answers `Showing::Still`
every time, and `halt` is never reached at any of the three moments 0043 names.
Measured on both:

| | connect | first read |
|---|---|---|
| Docker 29.4.1, nothing inside | succeeds | closed after ~2ms |
| Podman, nothing inside | succeeds | closed after ~195ms |
| something listening inside | succeeds | held open, or data |
| no container at all | refused | — |

Three things kept this invisible, and each looked like care at the time.

The record's end-to-end evidence was two jobs that were *both serving* — a page
somebody watched change, and a websocket. That is the case that works. The case
that fails is a job that shows nothing, which no manual test thought to try
because there is nothing to look at.

The unit test names this failure and cannot see it. It says in its own words
that "one that always answered would keep every container running for ever,
which is the behaviour it exists to avoid", and then tests a socket it binds
itself, on the stated reasoning that *a port is a port*. That premise is the
bug. A bare port has nobody answering for it.

And the revisit trigger points the other way: 0043 says to revisit if the
daemon is ever somewhere it *cannot* open a connection to a published port. The
assumption that broke is the mirror image — it can always open one.

## Decision

**Connect, then read once, and let the far side prove it exists.** The proxy
owns the near side and can fake accepting; what it cannot fake is what happens
next.

- **Closed at once** — nothing behind it. A clean end-of-file and a reset are
  the same event through different runtimes, and both mean this.
- **Said something** — serving, plainly.
- **Held open and silent** — serving. This is the case that decides the shape:
  an HTTP server sends nothing until it is asked, so a probe demanding bytes
  would stop exactly the containers the feature exists to keep. Treating
  silence as absence is the failure that looks most like rigour.

Nothing is written to find out. The far side is somebody else's server, and a
request this project made up is not ours to send.

`ANSWERING_WITHIN` becomes a budget for the proxy admitting the container is
empty rather than for a network round trip on loopback, and 500ms is chosen
against the slower runtime measured above with room to spare. Too short is the
dangerous direction now: a window shorter than that close reads every empty
container as one that is showing something, which is this bug again.

Rejected: **asking the runtime what is bound inside the container.** 0043
rejected it and both its reasons still hold — the image carries no `ss`,
`netstat` or `lsof`, and a process bound to the container's own loopback is
reachable by nobody while looking perfectly alive from in there. The point of
probing from outside was always to answer "can a person reach this?", and
reading is what makes it actually answer that.

Rejected: **treating a container's own port mapping as the signal**, i.e.
keeping it up while a mapping exists. That is what the code effectively did,
and it is why every container lived for ever.

Rejected: **sending a minimal HTTP request and requiring a response.** It
distinguishes the cases sharply and it is the wrong instrument: it assumes the
tunnel speaks HTTP, when 0042's whole argument is that an agent may put
anything behind it, and it writes invented bytes into a server this project
does not own.

Rejected: **shortening the read window to keep the sweep quick.** The measured
spread between the two runtimes is two orders of magnitude, so a window tuned
to the fast one silently restores the bug on the slow one — and it would do so
on somebody else's machine rather than on the machine that tuned it.

## Consequences

**A silent-but-serving container now costs the full window per sweep**, where
before every container returned instantly and wrongly. That is the price of the
answer being correct, it is bounded by the number of containers actually up,
and it is off the request path already.

**A server that accepts and hangs up immediately reads as nothing.** Correct
for anything a person could look at, and worth writing down as the one shape
this deliberately calls absent.

**The unit test that missed this is kept and told what it does not cover.** It
still checks both answers for an unproxied socket, which is real; the claim it
carried about being "the whole of what decides a container's lifetime" is
removed, because that claim is what made the gap invisible. The case it cannot
see is now a test needing a real published port, which is the only kind that
meets a proxy at all.

**`docs/conventions.md` §4's bar stops being vacuous.** "Nothing running that
is not holding a live tunnel" was trivially satisfied while everything appeared
to be holding one; it now has content, and the container test is what gives it
teeth.

**Reversing** is four lines in one function, and there is nothing recorded to
migrate. The wrong version is cheap to restore and expensive to run.

**Revisit if** a runtime publishes ports without a proxy in front — connections
would then be refused outright when nothing is inside, the read would become
redundant rather than wrong, and the window could shrink. Docker with
`userland-proxy` disabled is exactly that case, so this is a live possibility on
somebody's host rather than a hypothetical; it is safe there today, and only the
cost changes.
