# 0069 — A message reaches a working job

## Status

Accepted. Nothing below is built yet; this record exists so that the shape is
settled before it is.

Answers the question `docs/open-questions.md` carried since
`docs/decisions/0015-a-job-survives-the-daemon-dying.md`: whether a message
should reach a job that is already running. Retires the refusal a working job
gave a reply, which
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md` left waiting on a
session that outlived a turn. Gives a job an inbox of the shape
`docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md` gave a
foreman. Amends `docs/decisions/0055-a-job-says-why-it-stopped.md` for a
claim made before a message lands, and
`docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md` for how a
person's stop reaches the agent. Depends on
`docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
for the notice that says a message landed and the identifier it is shown
with, and on `docs/decisions/0068-a-mention-is-shown-its-thread.md` for what
it is shown of its thread.

Everything below about the adapter was measured on 2026-09-21 against the
image this project builds, driven as the instance drives a turn, with the
adapter pinned in `agent/images/claude/install.fragment`.

## Context

A reply to a working job is refused: the job is told to say it again once it
stops. That was honest while nothing could deliver one, and it covered the
case the design was built for, an agent that asked something and stopped. It
does not cover the case a person hits first, realising after starting a job
that the agent needs something it was not given, and since 0067 it would be
hit constantly: a room whose root shows the agent working is a room people
reply in.

The open question said what would settle it: finding out what an agent does
with a second prompt mid-turn. The protocol itself says only that a client
may send another prompt once a turn completes, and spells one way to reach a
running turn, cancelling it. The pinned adapter was measured on all of it.

**A second prompt mid-turn is accepted, queued, and loses the first turn's
answer.** Both prompts resolved as ended, in order. The first was settled the
moment its tool result came back, with no output at all, and the model
answered only the second. Nothing on the wire said so.

**Steering works.** The adapter advertises an extension at initialise and
serves it: a message handed to a running turn was acknowledged as injected
at once, the agent reacted to it about a second later, and the running
prompt resolved once, as an ordinary end of turn, under two seconds after
the message was sent. The injected message is not echoed on the update
stream. A tool call in flight when the message landed was aborted: a
two-second command had started, the agent answered a second later, and no
completion for that call ever arrived.

**Without an opt-in, steering an idle session starts a turn nobody asked
for.** The agent spoke with no request outstanding to answer. With the
adapter's idle behaviour set to prompt-required, the same call answers that
no turn is running and nothing happens.

**Cancelling is instant and the session survives it.** A cancel during a
tool call resolved the turn as cancelled within milliseconds, and the next
prompt was answered from memory of where the interrupted work had got to.

**The adapter's queue is in memory**, inside the container, and the
extension's name carries the prefix its protocol reserves for what is not
yet in the specification.

## Decision

**A job has an inbox**, the shape 0045 gave a foreman: a message in hand and
the rest in the order they arrived, kept with the job's record and written
with everything else, so a message in hand when this process dies is still
in hand when it starts again. A message for a job is taken or queued in the
step it arrives, given the eyes reaction once the inbox holding it is on the
disk, and the check mark when the turn that absorbed it ends, as a foreman's
messages are. The refusal goes: a message to a job is always received.

**A message is delivered by steering when the job's conversation is open,
and waits otherwise.** A message finding the job idle starts a turn, as a
reply does today. One finding a turn in flight with its conversation open is
steered into it, after the thread it was said in has been fetched per 0068:
the adapter answers *injected*, and the message is done when that turn ends.
One finding a turn in flight before its conversation has opened, or after it
has closed, waits, and the turn's end delivers it by starting the next. The
steering request always carries the idle behaviour that refuses to start a
turn, and *prompt required* is read as *wait*. One message is delivered at a
time, the next after the previous is answered, so arrival order holds as
`docs/decisions/0044-a-listener-only-listens.md` promises.

**A steered message is framed as an interruption.** Who said it, where, and
its identifier, as any message is; and that whatever the agent was doing may
not have finished, so it should check before assuming, in the terms the
resumption notice already uses. The notice 0067 posts at the root says when
it landed, linking it, since nothing on the stream would.

**A claim is cleared when a message lands.** A job that said it was waiting
for an answer and then received one mid-turn is no longer waiting: the note
registered against the turn is dropped, and the frame tells the agent to
say again why it is stopping before it does.

**A tool call in flight when a message lands is marked interrupted** in the
working, because no completion for it will ever arrive.

**Never a second prompt while one is open.** A prompt starts a turn and
nothing else; what reaches a running turn is a steer or a cancel.

**A person's stop is a cancel.** 0053's stop of a working job closes the
agent's process. It becomes the cancel the protocol spells, which the agent
answers and its session survives, and the outcome 0053 decides, paused, is
unchanged. The next message resumes the job with the interruption frame.

**The foreman is not steered.** Its inbox stays as it is and a second
message waits for the next turn: its turns are short, each message is its
own turn with its own frame, kits and brief, and folding two people's
mentions into one turn would answer the second under the first's framing.

**The capability is checked.** The conversation reads whether the adapter
advertises steering at initialise; without it a message waits for the turn
to end, which is the inbox path, and nothing else changes.

Rejected: **steering alone, with no inbox.** It works exactly when a
conversation is open, and a job spends much of its life with none: idle,
starting its container and session, closing, dead with the daemon, or
receiving two messages at once. Each of those needs somewhere for a message
to wait, and the adapter's own queue is memory inside a container.

Rejected: **a second prompt, since the adapter queues it.** Measured to lose
the first turn's answer without a word, which is the worst failure available
here: an agent that was working, stopped short, and nobody told.

Rejected: **refusing, as now.** With a transcript at the root inviting
replies, a person told to say it again in twenty minutes is told so most
times they speak.

Rejected: **closing the process and resuming with the message, for every
message.** It loses more of the work in flight than a steer does, costs a
session load, and is what a person's stop already is; it stays as that.

Rejected: **letting a person choose whether a message interrupts or waits.**
A rule people must learn, for a judgement the agent can make once it has
the message: one that says "when you are done" is read and acted on when it
is done.

Rejected: **steering the foreman too.** The reason above, and nothing a
foreman's turn does takes long enough to be worth interrupting.

## Consequences

**The busy notice goes.** A message to a working job is received and
delivered, and a person sees the eyes reaction rather than a refusal.

**Two prompts change**: the reply frame gains its interruption form, and the
paragraph about saying why a job stops says to say it again after an
interruption. Both are asserted whole, as every text this project composes
is.

**One field is added to a job's record**, its inbox, defaulted empty for what
the last release wrote.

**The reply gate's other job survives.** Refusing a second reply was also
what stopped two turns resuming one container; one turn per job at a time
is now the inbox's property, as it is the foreman's.

**A tool call can be aborted by a message.** A person who replies while an
edit is being applied may interrupt it. The frame is what makes that safe,
and it is a cost: the adapter's steering pre-empts and does not wait.

**The extension is not in the specification.** The pinned protocol crate
does not spell it, so the conversation renders the request itself, and a pin
bump re-measures it; the capability check is what keeps a bump that removes
it from breaking anything but the immediacy.

**Reversing** is the refusal back, the inbox field ignored, and two prompt
paragraphs; a message already in an inbox is delivered or dropped on
opening, as `docs/conventions.md` §4 permits for a message in hand.

**Revisit if** a foreman's turns grow long enough for a second message to
wait noticeably, which wants the same delivery there; if the adapter's
steering changes shape or gains a priority that waits for the current step,
which would make the abort above a choice; if aborting a tool call turns out
to cost real work often, which wants that choice; or if people want a
message to wait rather than interrupt, which wants the per-message choice
rejected above.
