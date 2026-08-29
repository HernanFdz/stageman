# 0029 — A reply is routed by its thread, and arrives over a socket

## Status
Accepted. Outbound threading is built; the transport and the router this
record decides are not, and this exists so that the shape they need is settled
before either is written.

## Context

`docs/decisions/0005-conversation-happens-on-channels.md` makes a channel the
place a job escalates, and `docs/decisions/0028-stageman-ships-the-tool-that-speaks.md`
gives a job a way to speak. Neither says how anything comes back.

A reply arrives naming a channel and a thread and nothing else. So two
questions have to be answered before a router can exist: how the message
reaches this process at all, and how a message becomes the job it is meant for.

The first is constrained by where this runs. `docs/vision.md` §3 has a daemon
on somebody's own machine, and the dashboard defaults to `127.0.0.1`.

## Decision

**Events arrive over Socket Mode**, an outbound websocket this process opens.
No inbound port, no public hostname, no tunnel.

Rejected: **the Events API**, where the platform posts to a URL. It needs a
public HTTPS endpoint, which contradicts a daemon on a laptop outright — every
operator would need a tunnel or a hosted relay before the first message, and
the thing being run is a program on their own machine.

**One Slack app per project.** Socket Mode needs an app-level token, and an
app-level token belongs to the app rather than to a channel.

Rejected: **one app for the whole instance.** It is less setup, and it puts one
credential in a position to open every project's event stream — which is the
concentration `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`
exists to prevent, arriving by a different door. One app per project also gives
each project a distinct identity in the chat client, which is a legibility win
rather than a cost.

So a binding is **two credentials, and only one of them ever reaches a job.**
The bot token posts and goes in the handout; the app-level token opens the
socket and stays in the daemon. A leaked job credential can post in one
channel, which is the whole of what it could already do.

**Routing.** A message in a thread belonging to a job is for that job. Any
other message — at the root, or in a thread belonging to nothing — is for the
orchestrator, **and only when it mentions this project's bot.** Nothing else is
read at all. Messages the bot itself posted are never read, under any of these
rules.

Rejected: **root means the orchestrator, threads mean jobs, and nothing else
matters.** Cleaner to state and wrong in use: replying inside a thread is how a
person answers a specific message, so somebody answering the orchestrator will
thread their reply under it — and that rule sends the message most clearly
meant for the orchestrator to nobody.

Rejected: **answering every unrecognised thread with a fixed message.** It
sounds helpful and is mostly noise: a job's record outlives the job, so a
finished job's thread is still recognised, and the common unrecognised thread
is two people talking to each other in a project's channel. A bot that
interrupts those is the most irritating member of the room. The narrow case it
was meant for survives as a fixed reply: a thread whose job went with a project
that was forgotten, which is the one case the orchestrator genuinely cannot
explain.

## Consequences

**The mention is a convention, not a mechanism**, and it is the one thing an
operator has to know. Every chat bot works this way, so it is a habit people
already have rather than something to teach — but it is the cost of the routing
rule, and somebody who does not know it will post at the root and be ignored.

**A thread identifier has to survive the process dying**, which is why a job
records the thread it speaks in even though nothing about *speaking* needs it —
a container keeps what it was created with, so a resumed job is already holding
its thread. The record exists for the lookup in the other direction. It is
`JobId` by way of the project's jobs, so the map is a search rather than an
index; at the scale in `docs/vision.md` §3 that is the smaller thing.

**Filtering out the bot's own messages is load-bearing**, not hygiene. Without
it, an agent posting a question produces an event routed back to that agent,
which answers, which produces another. The loop costs real model tokens per lap
and would be discovered on an invoice.

**Nothing here is built.** Forwarding a reply into a running job needs the
agent session to outlive the turn that started it, which is the long-lived
connection question in `docs/open-questions.md` and is now that question's
second consumer.

Reversing the transport is contained — the router does not care how a message
arrived. Reversing one-app-per-project is not: it is a credential shape, so it
is a change to `ChannelConfig`, to the form, and to a snapshot every instance
already has.

Revisit the transport if this ever runs somewhere with a public address, where
the Events API stops being a burden and starts being simpler. Revisit the
mention rule if a project's channel turns out to be used only by stageman, in
which case requiring it buys nothing.
