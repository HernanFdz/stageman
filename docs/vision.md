# Vision

What this project is for, and what it will never be. Read this before deciding
*what* to build.

Three documents, three questions, and the rule that keeps them from drifting
into one another: if a sentence would change what a user **types**, it belongs
in `README.md`; if it would change what gets **built**, it belongs here; if it
would change where **code goes**, it belongs in `docs/architecture.md`. A
sentence that answers two of those is two sentences.

**Each section says what belongs in it and starts empty.** That prose is a
standing rule, not a placeholder: it stays when the section fills up, because it
governs what gets added next. Only the HTML-comment example and the
`_(none yet)_` marker are disposable.

## 1. The problem

Whose problem this is, and what it costs them today. State it without naming the
solution — a problem defined in terms of its answer cannot rule anything out
later, which is the one job this section has.

<!-- Example:
Small teams keep their runbooks in chat. When the person who wrote one is
asleep, the next person re-derives it from the code, badly, at three in the
morning, and the re-derivation is never written down either.
-->

_(none yet)_

## 2. What it refuses to be

The non-goals, each with its reason.

This is the section that pays for itself. Without it every plausible feature
looks like an oversight, and sooner or later somebody helpfully adds one — a
deliberate absence that is not written down is indistinguishable from a gap.

<!-- Example:
- Not a scheduler. Anything with a clock in it grows a distributed-systems
  problem, and being boring and synchronous is the whole advantage here.
- Not multi-tenant. One team per deployment; isolating tenants would cost more
  than running a second copy.
-->

_(none yet)_

## 3. The constraint that shapes everything

The one fact the downstream decisions keep bumping into — a scale, a deadline, a
platform, a person, a budget. Naming it once here saves re-arguing it in every
decision record, and makes the records shorter for citing it.

<!-- Example:
It runs on the operator's laptop, not on a server. That rules out anything
needing a daemon, and it is why state is a single file rather than a database.
-->

_(none yet)_
