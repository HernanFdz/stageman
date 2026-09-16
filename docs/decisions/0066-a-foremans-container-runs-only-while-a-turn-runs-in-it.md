# 0066 — A foreman's container runs only while a turn runs in it

## Status

Accepted. Amends
`docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`:
brings a foreman's container under the rule that record states, and reverses
the consequence there that such a container persists. Leaves
`docs/decisions/0012-agents-run-in-containers.md` standing, with *long-lived*
read as it was meant — the container exists for as long as its project does
and keeps the session inside it — rather than as it was built. Changes nothing
`docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md` rests on,
because that record needs the container to be there, not to be up.

## Context

0043 made a job's container run while a turn runs in it or its tunnel
answers, and applied that rule at three moments: when a turn ends, on a timer,
and at startup. A foreman's container fell outside the rule by omission rather
than by decision. It is held open the same way, by the image's holding
command, and it is filtered out of all three moments: a foreman's turn ending
returns before the probe is reached, and both sweeps keep only the containers
named for a job. So a foreman's container had no path to being stopped while
its project existed.

0043 noticed, called it "not the aim", and then called it the right direction,
on the reading that 0012 asked for a long-lived container. 0012 asked for the
container to *exist* across turns and keep its session, against the
alternative of a new container per signal. It never asked for it to run.

Seen on the machine this was found on: a foreman's container up for nineteen
hours, holding the runtime's init and the holding command and nothing else,
with no daemon running. Nothing of this project's runs on a hard kill, and
nothing stopped it afterwards either.

What that costs is not the process, which is a sleep. It is that the runtime
never becomes idle. Docker Desktop reclaims most of its virtual machine's
memory once no container has been running for a few minutes, and one
permanently running container per project means it never does — on exactly
the machine `docs/vision.md` §3 says this runs on. The bar in
`docs/conventions.md` §4, nothing running that is not holding a live tunnel,
was also simply false for every foreman, and that section's claim that the
simulated world checks it after every step described a check that did not
exist: the oracle checked only that a working job has a container.

What stopping costs was measured before deciding. On Docker Desktop, stopping
a container held open this way takes a tenth of a second and starting it
again takes less, against tens of seconds of model time per turn. A start
issued while a stop of the same container was still in flight — the race a
stop at the end of one turn opens against a message arriving a moment later —
was serialised by the runtime: the stop finished, the container was started
again, and a process run inside it afterwards succeeded. Podman was not
measured.

## Decision

**0043's rule is every container's.** A container runs while a turn runs in
it or its tunnel answers. A foreman's tunnel answers nobody — nothing ever
tells anyone where it is — so a foreman's container runs while a turn runs in
it, and is stopped at the same three moments a job's is probed:

- **When its inbox empties.** A foreman that finishes a message with nothing
  queued behind it asks for its container to be stopped, in the same step and
  without waiting for a write, the way a job's probe is asked. Between queued
  messages nothing stops, so draining an inbox costs one start.
- **On waking.** A foreman's container found running is stopped unless its
  project is holding a message, in which case the message is picked up in it,
  per 0045.
- **On the settling timer.** A foreman's container found running with no turn
  in flight and nothing in its inbox is stopped. This is what catches a hard
  kill between a turn ending and its stop landing.

Not probed. Every container publishes a port, but a foreman's is nobody's
address, and probing it would make a stray listener started by a foreman's
agent permanent — the wrong direction for a container with no workspace.

The next message starts the container again and resumes the session inside
it, which is the path a job's reply already takes and the path the scenarios
already pinned for a foreman whose container was stopped. Nothing about
resuming changes.

Rejected: **making the agent the container's command again, for the foreman's
image only**, so that the container stops when its agent exits. That is the
mechanism this project ran every turn by before 0043, which 0015 measured, and
it makes the lifetime a property of the mechanism rather than a rule kept in
three places. It also stops the container on a hard kill mid-turn, which no
rule kept here can reach. It lost on four costs. It is a second way to run a
turn, for ever: exec into a held container for a job, attach to the
container's own process for a foreman, in the instance's turn sequence, the
runtime vocabulary, the simulation and the recorder. It brings back the attach
race the older code guarded with a stop before every start — a start on a
container still stopping does not attach, and reads as a protocol failure. It
rests the bar on the adapter exiting at end of file, which is measured for the
pinned adapter and is somebody else's property. And a container's command is
fixed when it is created, so every existing foreman's container would have to
be found and recreated, losing its session once. What remains in its favour
is the kill mid-turn, and a foreman killed mid-turn is holding a message and
needs its container up again at the next start anyway. Revisit if the turn
sequence is ever split by role for another reason, or if a kill leaving a
foreman's container running turns out to matter in practice; nothing here
gets in that design's way.

Rejected: **a new container per message.** 0012's rejected option, still
rejected: the session is the foreman's memory of the conversation, and 0045
is built on it.

Rejected: **leaving it running and exempting the foreman from the bar**, which
is what 0043's consequence amounted to. The cost above says no, and a bar
with an exemption for the one container that never shows anything reads
backwards.

Rejected: **stopping only after several settling sweeps have found it idle**,
so that a burst of messages a minute apart pays no start. No clock is needed
for it, a count of sweeps is enough, but it is held state and a rule of its
own, bought for a tenth of a second per message. The settling sweep already
bounds how long a container left up by a crash runs; nothing else needs the
delay.

## Consequences

**The simulated world now checks what §4 said it checked.** After every step
of every scenario, once the instance is awake and has swept: no container of
this instance's that it has listed or made is running with nothing in it — no
turn in flight for it, no message waiting for its foreman, nothing answering
on its tunnel, and no question about it in flight. The check fails on the
state this record fixes, which is the argument for having it. What it does not
check is the other half of the bar, that nothing is left the instance cannot
name; that half is the waking sweep's, and its scenario pins it.

**A message after idleness pays a container start.** A tenth of a second on
the runtime measured, before the session load every turn already pays.

**A hard kill mid-turn still leaves the container running** until the next
start, exactly as it does for a job, and for the same reason: nothing of this
project's runs on a kill. The settling sweep bounds the other window — a kill
between a turn ending and its stop landing — to one interval.

**The race at a turn's end is left to the runtime.** A message arriving in the
tenth of a second a stop takes asks for a start while the stop is in flight.
Docker serialises the two and the container comes up; the same race already
existed for a job, between its probe's stop and a reply's start, and was
accepted without being noticed. If Podman is measured to behave otherwise, the
instance can hold the stops in flight and look again when one lands, which is
a held set and one branch.

**Reversing** is deleting the stop at the three moments and the oracle's
foreman branch. Nothing is recorded, and every foreman's container in
existence is either stopped, which the next message starts, or running, which
the old code never minded.

**Revisit if** a foreman's turn is ever driven through a connection held open
across turns, which is the open question this record deliberately does not
touch: an agent process that lives between turns is a container that runs
between turns, and this rule would then have to say why that one may.
