# 0054 — A container says which instance started it

## Status
Accepted. Reverses the "nothing unplaceable is removed" rule in
`docs/decisions/0015-a-job-survives-the-daemon-dying.md`'s sweep, for the
containers this instance can prove are its own, and supplies the missing fact
that made removing any of them unsafe.

## Context

The startup sweep finds every container carrying this project's label and tries
to place each against the instance's records. Two kinds cannot be placed: a name
this version does not understand, and a name that parses perfectly and points at
a job the instance has no record of. Both were reported and left, on reasoning
worth quoting because it was right: *a container is where a job's work lives, so
"I did not recognise this, so I deleted it" is the wrong answer when the
instance is the thing that is wrong.*

What that reasoning could not see is the case that makes leaving them wrong too.
**Two instances sharing one container runtime is the ordinary arrangement, not
an exotic one.** `just dev` serves an instance out of a checkout, under its own
key and its own state file, against the same daemon as the real instance in the
platform's data directory. Every container either one starts carries the same
label. So each sees the other's containers as work it has lost, and each would
warn about all of them for ever.

Had the sweep been told to remove what it could not place, the outcome would
have been worse than an accumulating warning: starting the development instance
would destroy the real instance's jobs and foremen, and starting the real one
would return the favour. The label the sweep works from says the container is
*this project's*. Nothing said whose.

## Decision

**Every container this project creates carries the identity of the instance
that created it, and the sweep removes only what that identity names as its
own.**

- **The identity is minted once and kept in the snapshot.** A file that has none
  is given one the moment it is opened, which covers a first run and an upgrade
  in the same path. It travels with the file rather than with the machine, which
  is the opposite of the container runtime's path and for the opposite reason: a
  copied instance file is the same instance, and a second daemon elsewhere has
  none of its containers to confuse.

- **It is not part of the domain.** Nothing in the state reads it, because what
  it answers is a question about containers. It lives on the snapshot and the
  value is held by whatever operates the file. That also keeps the domain
  effect-free: minting needs randomness, and every identifier in that crate
  arrives from outside.

- **Two places ask, not one.** The startup sweep is the one that *removes*,
  and the timer that stops containers nothing is using asks the same question
  for a different reason. That second one was found by this record's own work
  and is the more urgent half: it was stopping containers belonging to other
  instances, mid-turn, roughly once a minute — measured by starting a container
  under a made-up instance beside a running daemon and watching it be stopped
  within thirty seconds. A stopped container ends the agent inside it, so
  another instance's job simply died, reporting that its container had exited.

- **Three answers, not two.** A container labelled with this instance is
  **ours**, and an unplaceable one is removed. A container labelled with another
  instance is **elsewhere**, passed over silently — not ours to touch and not
  ours to report. A container carrying no such label **cannot be attributed**,
  and is reported and left exactly as before.

- **Every uncertainty answers "cannot be attributed."** A runtime that will not
  say, a label that is not an identifier, a container that has gone since the
  listing: all of them mean *do not remove this*. Only a label naming this
  instance earns a removal.

Rejected: **putting the instance in the container's name** rather than in a
label. It needs no new field anywhere and no inspection, because a name from
another instance simply fails to parse as ours. It loses the distinction that
does the work: an unparseable name would then mean both *another instance's* and
*an older version of ours*, and those want opposite treatment — one is passed
over silently and the other is a warning somebody should read.

Rejected: **filtering the listing by the instance label**, so the sweep never
sees anything else. Cheaper, one query instead of one inspection per unplaceable
container. It gives up `docs/conventions.md` §4's bar: a container from before
the label existed would not be listed at all, so the instance would stop
reporting exactly the containers nothing else can account for.

Rejected: **deriving the identity from the snapshot's path.** No new field, no
randomness, stable across restarts, and distinct for a development instance. It
breaks when an operator moves the file, and it breaks silently in the worst
direction available: the instance stops recognising its own containers, so they
become *elsewhere* and are passed over without a word.

Rejected: **leaving the rule as it was.** Honest, and it accumulates. Every
container a lost job leaves behind is warned about on every start for ever,
which is the shape of message people learn to scroll past — and since
`docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md` it
also pins an image that nothing can reclaim.

## Consequences

**A container nothing here can place is left running rather than stopped.**
That is the timer's half of the rule and it errs the other way from the
sweep's: not stopping one costs memory, and stopping one costs somebody a
turn. A job this instance *does* have a record of is stopped whatever its
container says, because being able to place it is better evidence than a label.

**One generation of containers can never be attributed.** Anything created
before this label is left alone for ever, and that is deliberate: removing it
would be a guess, and on a shared runtime the guess destroys somebody else's
work. It is counted separately so an operator can see there is a fixed number of
them to remove by hand, once.

**A snapshot restored from an older backup now loses containers.** That is the
case 0015 protected, and it is protected less: an instance that has forgotten a
job will remove that job's container rather than report it. What makes the trade
acceptable is that the warning still names the container and says work may have
been lost — and that the alternative was warning about it on every start until
somebody removed it by hand anyway. The narrow window where the old rule was
better is a person noticing the warning *and* restoring the right snapshot
before the next start.

**The sweep now spends one inspection per unplaceable container.** Rare by
construction, and the shape both runtimes take — a listing's labels are
formatted differently on each, and an inspection is not.

**A foreman's turn gained a bundle where it had three arguments.** The instance
was the eighth argument to the call that starts a foreman's container, and the
project's own facts were three of the others, so those three became one value.
That is worth noting as a consequence rather than a tidy-up: passing them
abreast let a caller take two from one project and the third from another, and
it would have compiled.

**Reversing** means deleting a label and restoring "report everything". The
field on the snapshot would stay harmlessly, and every container already
labelled would go on carrying one that nothing reads.

**Revisit if** an instance ever needs to adopt another's containers on purpose —
a real migration rather than a copied file — which is the one thing this makes
harder, and which would want a deliberate hand-off rather than a sweep that
guesses.
