# 0043 — A container lives as long as its tunnel answers

## Status

Accepted. Amends the mechanism in
`docs/decisions/0015-a-job-survives-the-daemon-dying.md` — a turn stops being a
container restarted and becomes a process run inside one that is already up —
and makes true a promise
`docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` made and could not
keep. It also restates `docs/conventions.md` §4's bar on what a hard kill may
leave behind, which is the expensive half.

Names this project does not yet define are unbackticked, for the reason 0042
gives.

## Context

0042 shipped and was driven end to end against a real project: a job served a
page that changed while somebody watched it, and a second job pushed messages
over a websocket through the same tunnel. Both worked. Both were also *turns in
progress*, which is what hid the defect.

**A tunnel is reachable only while its job's turn is running**, and the reason
is mechanical. The agent is the container's entry point, the daemon speaks to
it over that container's standard input, and a turn ends by that pipe closing.
The agent sees end-of-file and exits; it is process one, so the container exits
with it, taking down whatever the agent had left listening.

That breaks exactly half of what the kickoff offers. *A dev server while you
work* is fine — the agent is mid-turn throughout. *A built result for somebody
to look at before you propose it* is not: the agent says where to look, stops,
and stopping is what removes the page. The instruction is answered by an agent
doing precisely as it was told, which is the worst way for a prompt to be
wrong.

The cost of simply keeping every container up is the thing to avoid. Most jobs
show nothing at all, and a container per job running for ever — holding memory
on somebody's laptop — to serve pages that do not exist is a poor trade for a
feature only some jobs use.

## Decision

**A job's container runs while either is true: a turn is running in it, or its
tunnel answers.** Neither holds for a job that never showed anything, so those
behave exactly as they do today.

- **The image stops running the agent by default** and instead runs something
  that does not exit and is not this project's to write — the image already has
  one. Each turn runs the agent *inside* the container that is already up,
  rather than by starting a stopped one. Containers are created with the
  runtime's own init, so that process one reaps what an agent orphans; without
  it a long job accumulates zombies.

  **The image's default *command*, not its entry point**, and the difference is
  worth the sentence because getting it wrong fails silently. A command named
  on the command line replaces a default command and is *appended* to an entry
  point — so with an entry point, the paths that still want the container to be
  one agent would ask for the holding command and the agent's name together,
  and get a container that starts perfectly, sleeps, and never speaks.

- **Answering is a connection the daemon makes to the published port**, from
  outside, and not a check for something bound inside. Those differ in one case
  and it is the case that matters: a server an agent bound to loopback inside
  its container is holding the port and is reachable by nobody. Defined the
  other way it would keep a container alive for ever to serve no one — turning
  the single mistake agents most reliably make into an immortal process.

- **One rule, applied at three moments**: when a turn ends, on a timer for
  containers already left up, and at startup. Startup matters most, because
  that is where a daemon that was killed rediscovers what it left.

Rejected: **a supervisor loop as the entry point**, deciding this from inside.
It has to answer the same question with no tooling to answer it with — the
image has no `ss`, `netstat` or `lsof` — so it ends up reading the kernel's own
table and matching a port number. That number would then live in the image,
where an image built before it changed watches the wrong one in silence, which
is the drift
`docs/decisions/0034-tools-are-served-not-shipped.md` removed from this project
at some cost. It also gets the loopback case wrong, because from inside a
server bound to loopback looks exactly like one anybody can reach.

Rejected: **an entry point that forwards standard input through to the agent**,
which was the first shape proposed and is the one that does not work. A second
turn would have to attach fresh input to a process already running as process
one; the runtime allows one attachment at a time and hands back the same
stream, so it fails intermittently rather than cleanly. Running the agent as a
new process per turn gives each one its own pipes and needs nothing forwarded.

Rejected: **a watchdog inside the container that exits when it loses the
daemon.** It is the only thing that could make a hard kill tidy up, and it
contradicts 0015 outright: a job that dies with the daemon is the behaviour that
record exists to prevent.

Rejected: **a grace period on the end-of-turn check**, for a server that is
still coming up when its turn ends. Real, and deliberately not handled, because
the failure is benign in a way that is worth stating: the container stops
exactly as it does today, and the agent starts it again on the next turn.
Nothing is lost that the current behaviour would have kept, so the simpler rule
wins.

Rejected: **copying a job's output out of its container and serving that
instead**, which needs no lifetime change at all. Genuinely cheaper, and
narrower in the way that decides it — the limit is not what stays alive but
what can be shown at all. Copied files serve static content and nothing else,
where the point of a tunnel is that an agent may put *anything* behind it: an
application, with whatever server logic and infrastructure the work turned out
to need. Serving a job's output answers a smaller question that happens to
resemble this one.

## Consequences

**A hard kill leaves containers running, and no design here can prevent it.**
On a kill nothing of this project's runs — that is what the signal means — so
there is no shutdown path to write. What stops a container today does so
indirectly: the daemon dies, the kernel closes its end of the pipe, the agent
reads end-of-file and exits, and the container follows because the agent is
process one. Once the agent is no longer process one, the same end-of-file ends
the agent and nothing else.

**The side of that which is not a cost:** the process the agent left listening
is inside a container that never stopped, so it never died. A tunnel stays
reachable straight through the daemon being killed and started again, and
somebody looking at a page does not notice. Only a reboot of the host loses it.

**`docs/conventions.md` §4's bar is restated rather than broken.** "Killing
stageman leaves nothing running" becomes "leaves nothing running that is not
holding a live tunnel, and nothing that cannot be named". That is the same
narrowing 0015 already performed once, when it made a retained *stopped*
container deliberate rather than a leak — and it makes the test harder for the
same reason, because a suite that counted what was left behind would now pass
on the leak and fail on the feature.

**The sweep gains a legal state it does not currently have.** An idle job whose
container is running is neither of the two cases `reconcile` reasons about —
a working job whose container stopped, and a container whose job is not
working — so a sweep that has not been told about it will either put the job
back to work or report it as unplaceable. Both are wrong and both are quiet.

**0015's measurement needs taking again.** That record rests on one
observation: hard-killing the attached client leaves the container exited with
its session intact, and starting it again resumes. Neither half describes what
happens here, so the claim has to be re-established for a container that is
never stopped and an agent that is run inside it — on both runtimes, since
`docs/open-questions.md` already records that the original was taken on one.

**A foreman's container persists too, and that was not the aim.** One image
carries both, so an entry point that does not exit changes both — and nothing
stops a foreman's, because the rule above is a job's and a foreman has no
tunnel to answer on. It is the right direction rather than a regression:
`docs/decisions/0012-agents-run-in-containers.md` asks for the foreman's agent
to live in one long-lived container and the built thing started one per
question, which that record's own reading calls a gap. This closes the half
that is about a *container*. It does not touch the half
`docs/open-questions.md` is actually about, which is holding a *connection*
open across turns — that is still a task owning it and a channel to speak
through, and still unbuilt.

**A forgotten server is immortal.** An agent that leaves something bound keeps
its container alive indefinitely, and nothing reclaims it. That is the accepted
cost of the rule rather than an oversight: it lands on the retirement question
in `docs/open-questions.md`, which stops being about disk and becomes about
memory. The cheap answer in the meantime is to ask the agent to stop it, which
is a message to a job that already works.

**One thing gets simpler.** 0042 records that Docker assigns a new host port
every time a container starts and Podman does not, which is why nothing stores
one. Under this decision a container that is showing something does not stop,
so the reassignment stops arising in practice. The rule stands as written —
deriving it is still correct and still cheaper than storing it — but the case it
was defending against becomes rare rather than routine.

**The resumption notice changes, and not in the obvious direction.** A job put
back to work after an interruption is told what it cannot assume. It would be
wrong to tell it that whatever it launched has crashed: after a kill it is very
likely still running, and only a reboot makes that true. The honest form is the
one the notice already uses about everything else — that it may or may not still
be running, and to check rather than assume either way.

**Reversing** means putting the agent back as the entry point and starting
stopped containers again. No data migration, because none of this is recorded,
but every container in existence at the time would have to be recreated, which
discards its session. Cheaper before there are long-lived jobs than after.

**Revisit if** an agent is ever run somewhere the daemon cannot open a
connection to its published port, which is the one assumption the whole rule
rests on; or if the number of containers left holding tunnels becomes the thing
an operator notices first, which is the retirement question arriving rather than
this one being wrong.
