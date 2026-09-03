# 0048 — A job runs on a kit, and a project says which kits there are

## Status

Accepted. Extends `docs/decisions/0006-agents-are-pluggable.md`, which records
which agent ran a job and has the foreman choose one: the thing recorded and
chosen is now a *kit*, an agent set a particular way, and the agent alone is no
longer enough to say what ran. Leaves
`docs/decisions/0008-one-credential-per-agent.md` in force and names, under
Consequences, the change that would revise it.

## Context

A job records which agent ran it and nothing about how that agent was set. The
two settings that most change what a job costs and how well it does — which
model, and how hard it thinks — are chosen by nobody, so every job runs on
whatever the adapter defaults to. The foreman cannot send a typo fix to the
cheap model and a cross-cutting refactor to the expensive one, and an operator
cannot say that a project's jobs never run on the expensive one at all. Both
are the ordinary things to want from something that dispatches work unattended.

What makes this harder than adding two fields is that **the settings, their
values, and which of them exist at all depend on the agent** — and, it turns
out, on each other. A design that fixes a flat set of fields in the domain is
wrong for the second agent, and a design that takes free text is wrong for the
first.

**The protocol already has the concept, in the version this project speaks.**
An agent answers session/new with a list of configuration options, each with an
identifier, a human name, a category — the specification names *mode*, *model*,
*model configuration* and *thought level* — and a payload that is either a
select over named values or a boolean. The client sets one with
session/set_config_option, and the reply carries the whole list again, updated.
`docs/decisions/0010-acp-is-the-agent-contract.md` chose the protocol because it
normalises session vocabulary across agents; this is that property paying out a
second time.

**Measured against the pinned Claude adapter, with a real subscription token.**

| probe | result |
|---|---|
| options advertised on session/new | `mode`, `model`, `effort`, each currently `default` |
| the model's values | `default`, `sonnet`, `opus`, `haiku` |
| the effort's values | `default`, `low`, `medium`, `high`, `xhigh`, `max` |
| set the model to a name it did not offer | refused, before any prompt, with an error naming the option and the value |
| set the model to a full dated model identifier | refused the same way |
| set the effort to a name it did not offer | refused the same way |
| set the model to `opus` | accepted, and a boolean `fast` option appeared that was not there before |
| read the list back after each set | the current value had changed exactly as asked |

Four things follow from that table, and each shapes the decision.

*Validation at the door is free and loud.* A value the agent will not take is
refused before the first prompt, so a job asking for the impossible fails
before its container has done anything a person can see, with the agent's own
words saying why. `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
is the record of what it costs when a call succeeding is mistaken for the thing
having happened; here the thing happening can be checked in the same reply.

*The model axis is a small closed set of aliases, not an open string.* The
adapter accepts `opus` and refuses the dated identifier behind it. An alias
tracks the vendor's releases on its own, so the set is small and moves slowly.

*The set is a function of this binary, not of the world.* The recipe pins the
adapter's version and is compiled in, per
`docs/decisions/0035-an-image-is-built-never-named.md`, so what the adapter
advertises cannot change underneath an instance without a release of this
project. A pin bump is the moment to re-check, exactly as `docs/conventions.md`
§3 already says of the Dioxus CLI.

*Options depend on each other.* Choosing a model changed what else was offered.
Any shape that treats settings as independent fields is wrong on the first
agent, not merely on some later one.

**Three more measurements shaped the type and the check.** A session opens
with no credential and no network, and its options can be set, so the test
below needs neither. Without the credential the adapter advertised the model
`opus` as `opus[1m]`, accepted `opus` anyway, and reported `opus[1m]` back — so
the advertised list depends on what an account is entitled to, and a value
asked for is not always spelled the same in the reply. And choosing `haiku`
removed the effort option from the reply altogether, in both environments;
asked to set an effort on it, the adapter refused the option as unknown. Haiku
has no effort, and a type that let one be written for it would be sending a
value the adapter will not take.

**The mechanism is the protocol's rather than one adapter's.** The Codex
adapter the protocol's own library names refuses to open a session without a
credential, and none was to hand — but its bundled source carries the *model*,
*thought level* and *mode* categories, so it advertises the same shapes.

**A loaded session does not keep what it was set to.** Measured the way this
project actually resumes: one long-lived container, the agent run inside it
twice. The first run set the model to `opus` and the effort to `high` and sent a
prompt, so the session was written. The second run listed that session, loaded
it, and read the options back — every one was `default` again, and a further
set confirmed the reading. So the mechanism that lets a job survive this
process dying, per `docs/decisions/0015-a-job-survives-the-daemon-dying.md`,
also silently puts it back on the agent's defaults, and nothing in the reply
says so. Whatever this record decides has to be done on every turn and not
once.

## Decision

**A job runs on a kit: one agent, set the way that job runs it. A project
names the kits its jobs may run on, each with a name and a description an
operator wrote, and every job — the foreman's and a person's alike — picks one
of those and nothing else.**

Seven things follow, and each is the decision rather than a detail of it.

**The tag is the agent.** In the domain a kit is an enumeration whose variants
are the agents and whose payloads are each agent's own settings. A job holding
settings for an agent other than the one it ran on is therefore not a state to
check for but a sentence that cannot be written — there is no second field to
disagree with the first. The domain owns the *shape* of each agent's settings
and the adapter owns their *spelling* on the wire, which is the seam
`Handout` already draws for credentials: deciding is a pure question about
configuration, and what a value is called is knowledge about one agent.

**Each axis is closed where the agent's is, and open where it is not.** For the
Claude adapter both axes are variants — the aliases the adapter advertises, and
nothing else — because the adapter's version is pinned in the image and the set
is knowable at compile time. One container test opens a session and asserts
that what the adapter advertises equals what this project can spell, so a pin
bump that changes the set fails there rather than rotting. An agent whose model
axis is genuinely open — one that names a provider's own model identifiers —
gets a shape-validated name in its variant, the way `VariableName` is validated,
and for that axis the session's refusal is the only check there is. The
enumeration lets both kinds coexist without pretending they are alike.

**The agent's own default is a variant, never an absence.** The adapter offers
`default` as a value with a meaning of its own, so "no preference" is a member
of the set the agent spells, not a second notion of absence layered on top by
this project.

**A dependent option lives inside the variant it depends on.** The effort is a
field of the models that have one and absent from the one that does not, so
"Haiku at high effort" is not a combination to refuse but a sentence that cannot
be written. The `fast` toggle that appeared only under one model belongs in
that variant's payload if it is ever exposed, and it is not exposed yet.

**Set at the door, check by change, and record what was reported.** At the
start of every turn, before the first prompt, every option this project has an
opinion on is set, and a refusal fails the job with the adapter's own message.
Each reply is then read back — and read for *change* rather than for spelling,
because the adapter was measured to report `opus` back as `opus[1m]` after a
set that worked. What is required is that the reported value moved, unless what
was asked for is what was already reported; a set the adapter accepted and
reported unchanged fails the job, which is what would catch an adapter that says
yes and does nothing. Not this one, but the next one need not be so honest. The
values reported are recorded on the job beside the kit, because the two are
different facts: the kit is what was asked, the reported values are what ran,
and `opus[1m]` is the proof that the second is not always derivable from the
first. On a resumed turn the whole sequence runs again, so that a job put back
to work after this process died runs on what it was created with rather than on
whatever the agent defaults to.

**Fixed when the job is created, for the job's whole life.** The protocol
allows a setting to change mid-session and this project declines to use that.
The reasoning is `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`'s
for a job's environment, applied to its agent: a kit is part of what a job
*is*, a resumed job is the same job continuing, and the record of what ran has
to stay true. The lever for wanting something different is the one every other
"change something about a running job" already has — a new job. The type has
no setter, so this is structural.

**A project's kits are the only kits.** A project used to name which agents
its jobs may run on; it now names kits, each carrying a name and a description
the operator wrote, and the set is never empty in a valid instance, checked
where the agent set was. The foreman's tool enumerates the names exactly as it
enumerated agents, and the dashboard's form offers the same list. **No job is
started on a kit its project does not name**, not even by hand from the
dashboard — one path, and adding a kit takes seconds. Three things recommend
this over letting each job choose freely across the grid, and the third is the
one that decides it.

- *Validation happens while a person is present.* A kit is created on a form,
  by an operator who can read a refusal and fix it; every later use is a
  selection from something already valid. Checking at job start would move
  the failure to the moment nobody is watching.
- *The foreman reasons over prose it can use.* "Deep — for refactors touching
  many files; costs several times more" is a better basis for a judgement than
  a vendor's alias list. `docs/decisions/0006-agents-are-pluggable.md` keeps
  an agent's description in code because it describes the agent; a kit's
  description is the operator's because it describes what *this project* wants
  that kit for, which nothing in code could know.
- *The interface does not grow with the axes.* Once an agent has a provider, a
  model and an effort, an editor of allowed values per axis is a grid nobody
  can read, and the foreman's choice becomes a point in it. A kit is one row
  with a name.

**A foreman's kit changes at a turn boundary, never during one.** A project's
foreman has a kit too, set by the operator, and its session is long-lived.
Changing it must not cost a message: a turn in progress finishes on the kit it
started with, and the change lands when the next message is picked up.
Nothing waiting is lost, because the inbox is the snapshot's per
`docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`. What the
boundary costs depends on what changed. Settings alone are re-set at the start
of every turn regardless, so the same agent on a different model keeps its
container and its memory. A different *agent* is a different image, so the
container is replaced and the session with it — the same price
`docs/decisions/0034-tools-are-served-not-shipped.md` records for the same
reason. The boundary can ask which agent a container was made for because the
container is labelled with it at creation, the way it already carries the label
that lets a sweep find it.

Rejected: **an open string for the model, validated by the session alone.**
Loses the compile-time check for nothing, since the set is knowable from a
version this project pins. Kept only for an agent whose axis really is open.

Rejected: **discovering the set at runtime and keeping it in the instance.**
State that can be stale, a refresh nobody owns, and a question about when to
ask — all to learn something the binary already knows.

Rejected: **probing a container to fill a form.** A container start on a
request path, for a list the enumeration already holds.

Rejected: **a free choice per job across every axis.** The most expressive and
the least safe: the autonomous chooser picks from a vendor catalogue with
nothing an operator wrote to steer it, cost control has nowhere to live, and
the tool schema grows a field per axis per agent.

Rejected: **allowed values per axis on the project.** Cost control without a
name to reason over, and an editor that grows with the axes — the grid above,
with a checkbox on every cell.

Rejected: **an ad-hoc kit for a job started by hand.** A second path to keep
consistent with the first, defended by "the operator is present" — which is
true, and buys a few seconds against maintaining two ways for a job to come
into being.

Rejected: **an optional model, with absence meaning the agent's default.** The
agent already spells that meaning as a value, and an `Option` would give
"unset" two owners.

Rejected: **one flat settings structure with every field any agent has.** Most
combinations are meaningless, the type says nothing about which, and the first
dependent option makes it wrong for the first agent.

## Consequences

**A snapshot migration in two places, both defaulted and both honest.** A
job's recorded agent becomes its kit, and the reader accepts the old bare
spelling — a job that ran before kits existed genuinely ran on its agent's
defaults, so the default is the true answer rather than a guess. A project's
set of agents becomes a set of kits, each agent becoming one kit on defaults
named after it. Both want the literal older-file test `docs/conventions.md` §4
requires, and both write the new shape, so a file upgrades itself on its first
change.

**The domain grows an enumeration per agent, and the adapter grows the
spelling of each.** Adding an agent is a compile error in both places and in
the dashboard's form, which is the property
`docs/decisions/0006-agents-are-pluggable.md` wants from the closed set. The
container test that holds the spelling to the adapter settles every kit the
domain can spell on one real session, through the same function every turn
uses, and needs neither a credential nor a network — so it sits with the
handshake tests rather than the ones that cost a credential. It fails on the
pin bump that removes or renames a value. A bump that *adds* one passes
silently, and that is the right shape: an alias this project does not yet spell
is a feature to add, not a defect to catch.

**Exact verification is not available, and the check is honest about it.** A
reply's spelling can differ from the request's for a reason as innocent as an
account's entitlements, so the read-back cannot say "this is exactly what was
asked". It can say the value moved, and that the adapter did not refuse — and
between the adapter refusing every value it does not know and the read-back
catching a value it accepted and dropped, what is left uncaught is an adapter
that accepts a value and changes the reading to something *else* it was not
asked for. No adapter measured does that, and the reported record is what would
show it.

**The foreman's tool and the dashboard's form both enumerate kit names**, and
the tool's descriptions carry the operator's prose, so the prompt snapshot
tests move. A kickoff does not tell a job which kit it is on; the agent knows.

**Providers come later, and the shape already has room.** With an agent that
reaches several providers, the credential belongs to the provider rather than
the agent, and a job must be handed exactly the one its kit names or the
billing failure `docs/decisions/0008-one-credential-per-agent.md` was written
against returns through a new door. A provider is platform-shaped — a closed
set, because naming its variable is code — and anything stranger is already
served by a project's variables. So credentials will move from per-agent to
per-provider, which is a snapshot migration and a revision of 0008's letter
that keeps its reasoning. It is not needed until a second agent lands; what is
needed now is that a variant can carry a provider and a handout can select one
credential by it, and an enumeration does that with a field. Queued in
`docs/open-questions.md`.

**The permission mode is not a kit field.** It is offered through the same
mechanism and it is not a per-job choice: how this project drives an agent is
the adapter's business and the same for every job. Decided separately in
`docs/decisions/0049-the-permission-mode-is-pinned-not-chosen.md`.

**Reversing** means collapsing each variant back to its tag and deleting a
form, and it costs every instance its kits and every project its cost
controls — the asymmetry
`docs/decisions/0046-a-projects-variables-are-carried-never-read.md` records,
one concept further along.

**Revisit if** an agent worth supporting advertises no options at all, which
makes its variant a bare tag and is fine; or advertises them and ignores a
value it accepted, which the read-back is there to catch and which would make
that adapter unfit rather than this record wrong; or if a project ends up with
so many kits that naming them stops being cheaper than choosing per axis —
that is the evidence the per-axis alternative above would need, and the
threshold is a screen an operator cannot scan.
