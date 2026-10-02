# 0086 — A kit charges a purse at a provider

## Status

Accepted. Amends `docs/decisions/0008-one-credential-per-agent.md`: its
letter — one credential per configured *agent* — becomes one purse per kind
per *provider*, and every line of its reasoning stands: what a process is
handed is constructed, never inherited, and exactly one credential is ever
present. Extends `docs/decisions/0048-a-job-runs-on-a-kit.md`, which left the
room this fills: a kit's variant now carries the purse it charges, and one
credential is selected by it. Answers two questions from
`docs/open-questions.md`: when credentials move from agents to providers,
and whether an agent's credential is checked before it is kept.
Narrows `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`
by one clause: a container's environment is fixed at creation for a
project's variables, and the purse is not among them. Adds a crate beside
**agent**, **channel** and **platform**, for the reason
`docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
added the third.

## Context

A credential is stored per agent, the kit's tag is the agent, the handout
picks the one credential by that tag, and the adapter decides which
variable to put it in by looking at the token's first characters. That was
right while one agent existed and it authenticated one way. Three facts,
read against the vendors' own documentation on 2026-09-30, make it wrong
for the second agent.

**A subscription is the vendor's own agent's, and a key is every agent's.**
Anthropic's terms reserve its subscription login for its native
applications and forbid a third party from storing or routing through plan
credentials; the one carve-out is the unmodified Claude Code binary signed
in by its own user, which is the path this project takes with the token
`claude setup-token` mints. OpenCode removed the Claude Pro and Max login
in March 2026 at Anthropic's request. So an agent that reaches several
providers charges a *key* at each, metered per token, while a subscription
is charged by one agent alone. The two are different credentials at the
same provider, and an operator may want both at once: the Max subscription
for Claude Code, and an API key for an agent that cannot charge it. A
credential per agent cannot hold that; a credential per provider cannot
either.

**Which variable a credential goes in is knowledge about one agent and one
credential.** Claude Code reads a key from `ANTHROPIC_API_KEY` and a
subscription token from `CLAUDE_CODE_OAUTH_TOKEN`, and 0008 measured that
the token in the key's variable does not fail but *hangs*. An agent that
reaches several providers reads a key per provider under that provider's
own name, and the names are the same across agents for a key and different
for a login. Delivery is therefore a table keyed by agent and credential,
and the domain has to be able to name the credential precisely enough to
index it.

**The credential's kind is decided today by sniffing a prefix**, because
asking the operator seemed like friction. It was friction with a cost: a
key pasted where a subscription token was meant is not refused, it is
delivered under the wrong name and hangs, which is the least visible
failure there is.

Five measurements decide the delivery. Four were made on Docker 29.4.1 and
are pinned by a container test that passes on Podman 6.1.0 as well. A bare
`--env NAME` on `exec` forwards the variable from the client's environment
exactly as it does on `create`, and `inspect` shows only what the container
was created with: so a credential can be given to the agent's process on
every turn rather than to the container once, and never appear in the
container's configuration. A name forwarded on `exec` takes the place of
one the container was created with. And `env` in front of the program,
inside the container, keeps a name the container was created with from
the program it runs, which a runtime has no flag for. The fifth was made
on Docker alone: without that clearing, a process forwarded one purse in a
container made with the other sees both.

One measurement decides the check. Anthropic's models listing answers a
bogus key with `401` and *API key is invalid.*, and no key with `401` and
*x-api-key header is required* — one read, with the credential, costing
nothing, which is exactly the shape 0076 checks a platform credential by.
A real subscription token answers the same listing with `200` when sent as
a bearer token, with or without the beta header the vendor's own client
sends, and with the same `401` as a bogus key when sent in the key's
header. A bogus token sent as a bearer is answered `401` and *OAuth access
token is invalid.*, so a refusal of either kind arrives with a sentence of
the provider's own. So both kinds are checked by one read, differing only
in the header the credential travels in, and the wrong box is refused by
the provider as well as by the shape. Both refusals, and a real token's
`200`, were the same from a client that names itself in no header, which
is how this daemon's own client asks, so the read carries the version
header the provider requires of every request, the credential, and nothing
else. What the provider answers when it is limiting requests or failing
was not measured: the statuses its documentation gives for those are read,
and nothing in their bodies is.

## Decision

**A provider is a closed set. A purse is one credential at one provider, in
the shape that provider hands out, of one kind: a key, metered per token
and honoured by every agent that speaks the provider, or a subscription,
flat per month and honoured by the vendor's own agent alone. The instance
holds at most one purse per kind per provider. A kit names the purse its
work is charged to, from the subset its agent can charge, and the
credential handed to its agent is selected by it.**

Nine things follow, and each is the decision rather than a detail of it.

- **The provider set is closed for the reason `Platform` and `Channel` are.**
  Delivering a credential to an agent is code: the variable it is read
  from, the check it is checked by, the mark it is drawn with. A provider
  an operator could invent would only postpone the failure to the moment a
  job runs. Anthropic is its only member until the agent that needs a
  second lands, because a provider nobody can charge is dead code with a
  mark.

- **The purse is what a kit names, and the kit's payload says which purses
  its agent can charge.** In the domain a kit is still an enumeration
  tagged by the agent, and each payload carries the purse as an enumeration
  of its own — the purses that agent accepts and nothing else — so
  *OpenCode on the Claude subscription* is not a state to check for but a
  sentence that cannot be written. The kit's purse is derived from that
  payload, never stored beside it. A kit is created on a form from what is
  held, checked while a person is present, and the type keeps it true
  afterwards.

- **A purse is held once and referenced by name.** The instance holds its
  purses in one collection keyed by the purse's own name, and the name is
  derived from the purse by the one constructor that inserts, so a purse
  filed under another purse's name is unrepresentable. On disk the purses
  are a list of self-describing entries, each sealed, and a name that
  appears twice refuses the file. A project's kits and its foreman's kit
  name a purse, and the state's one check refuses a kit whose purse is not
  held, exactly where it refuses a kit whose agent is not configured
  today. A purse is not forgotten while a project's foreman, an offered
  kit, or a job that is not over charges it, and the refusal names them —
  a job that would fail at its next turn is as much a dependent as a kit
  that would fail at its next start.

- **The purse travels with every turn, and the container never holds it.**
  The purse's variables are named on the command that runs the agent
  inside the container and carried in the runtime's own environment, on
  the first turn and on every resume, so that the agent's process has them
  and the container's configuration does not. The purse is selected when
  that command is asked, by the kit's own name for it, from what is held
  at that moment; what a container is made with is decided without it, in
  a type with nowhere to put one. The project's variables and
  the job's warrant stay where 0046 put them, given at creation and fixed
  for the job's life, because they are part of what the job *is*. A purse
  is not: it is who pays, and a replaced key reaches a foreman at its next
  turn without losing the session, reaches a job when it resumes, and is
  the answer to a credential that expires while nobody is watching. A
  foreman's container is kept across a change of purse for the same reason
  it is kept across a change of model.

  The same command clears every other variable that agent could read a
  purse from, so that its process sees one purse whatever its container
  was made with. A container made before this record holds the credential
  it was created with, for its life, and an agent that finds a key beside
  a subscription's token charges the key: without the clearing, a foreman
  moved from one purse to the other would go on charging the first, which
  is 0008's failure arriving from the past rather than from a shell.

  A kit whose purse is not held ends its turn failed, saying which, where
  the agent would have been run: after its container exists, so that
  holding the purse and saying something to the job is the whole repair.

- **The kind is declared, the shape is refused before anything is asked,
  and both kinds are checked before they are kept.** The Agents page takes
  a key and a subscription token in different boxes, so the kind is what
  the operator pressed rather than what a prefix suggests, and a value
  whose shape belongs to the other box is refused where it was typed. What
  passes is then checked by the provider's listing, on 0076's terms: one
  read with the credential in the header its kind travels in, refused with
  the provider's own words, or unchecked when the provider could not be
  reached, and kept in neither case.

  Refused is the provider saying no to the credential: unauthorised, or
  forbidden. Unchecked is everything that is a verdict on nothing: no
  answer within the ten seconds a person at a button is given, a limit
  reached, the provider's own failure. Either is said in the purse's own
  row, in the place of the line that says what the purse is for, as the
  rule alone, since the row already names the purse; and a replacement
  that fails either way leaves the purse that was held as it was, because
  nothing is written until the provider has accepted.

- **The names this project delivers are derived from the delivery table.**
  A project's variable may not claim a name any agent reads any purse from,
  and the set is every name the table can produce rather than the ones a
  configured purse would, for the reason 0046 gives: a purse added later
  must not turn an accepted name into a collision.

- **A model stays the agent's.** Nothing here moves the model axis to the
  provider. What a kit may say a model is called is what its agent's
  adapter accepts, checked against the pinned adapter as 0048 decided; a
  provider's own catalogue may one day feed suggestions on a form for an
  agent whose model axis is open, listed when the form asks and kept
  nowhere, and that is a later record's.

- **The file upgrades itself exactly.** A file the last release wrote holds
  one Anthropic credential under the agent's name and kits that name no
  purse. The credential's kind is the true answer from its first
  characters, since that is how it was delivered; it becomes the one purse,
  and every kit and every job's kit that named the agent is read as
  charging it, which is what they did. The agent's map is read and never
  written again, under the one-tag window `docs/conventions.md` §4 sets,
  and a literal older file proves it opens.

- **The dashboard says who pays, and offers nothing that cannot be
  charged.** The Agents page gains a providers section above the roster:
  one card per provider with its mark, and one row per purse kind, each a
  sentence with its actions inline and a guide to where the credential is
  minted. The agent rows lose their credential box and say which purses
  would make each ready. A kit's editor gains one control, *pays with*,
  offering only the purses held that the chosen agent can charge, and
  hidden when exactly one fits. A new kit charges the first held purse in
  its agent's order, and that order puts the subscription first: an
  operator who holds both added the flat one for the vendor's own agent,
  the only one that can charge it, so a kit for that agent starting on the
  key would be the surprise. Each box's placeholder is what a credential of
  its kind begins with, so the box teaches the shape before a refusal does.
  A chip's sentence names the purse, and
  nothing else on a chip changes: a glyph for the provider would say
  *Anthropic* beside *Claude* on every row today, and is worth drawing only
  when the two differ.

Rejected: **a credential per agent**, which is what exists. It cannot hold a
subscription and a key at one provider, and with a second agent the billing
failure 0008 guards against returns through the door that record could not
see: an agent charging a provider's key under a name it shares with another.

Rejected: **one credential per provider.** The map is simpler and a kit
need not say which; it fails on the first operator who wants the Max
subscription for one agent and a key for another, which is the ordinary
case rather than the odd one.

Rejected: **a purse that is a reference the compiler holds** rather than a
name looked up. Serialisation writes the value at every reference, so the
name exists on disk regardless, and the check on the way in is what makes
the lookup total; the idea is kept in the private notes for when enough
sites handle an absence that cannot happen.

Rejected: **the kit naming a purse from the whole set**, with a check that
the agent can charge it. Representable and wrong, and the same shape 0048
refused for an effort on a model that has none.

Rejected: **keeping the prefix sniff.** It is one question fewer on a form
and one hang more in a container, and the form already has to draw two
sentences because the two kinds are minted in different places.

Rejected: **delivering the purse at creation, as today.** Rotation then
costs a container, and a foreman's container is its session; and every
credential sits in the container's inspectable configuration for the
container's life.

Rejected: **selecting the purse when a turn is decided, and carrying it to
where the agent is run.** A turn that resumes is decided in several places
and run in one, and a place that found the purse missing would have to
fail a job before its turn existed, settling its inbox by hand; where the
agent is run, a missing purse is one more step that could not be taken,
and ends the turn as every such step does. The later moment also hands
over a credential replaced while an image was building, which the earlier
one would not.

Rejected: **making again the containers that were made with a purse.** It
would cost a foreman its session for a change this record says keeps it,
and the clearing leaves what such a container holds unread.

Rejected: **emptying the other purse's variable rather than clearing it.**
A runtime can set a variable to nothing on the command it runs, and
whether an agent reads an empty key as no key was not measured; `env`
removing it needs no measurement.

Rejected: **delivering the purse through the protocol's own providers
extension**, which the pinned adapter implements and which would carry a
credential per session in a header. It is a draft the specification may
change, one adapter implements it, and a credential in a request line is a
credential the transcript could log. It is the right delivery for a
gateway held elsewhere, and that is `docs/decisions/0085-the-instances-apps-may-be-held-elsewhere.md`'s later record.

Rejected: **several purses of one kind at one provider**, a key per
project, for instance. Attribution is what the turn records will answer,
and the instance is one operator's; the trigger is named below.

Rejected: **a purse per project.** A purse is the operator's, as an agent's
credential was; per project would be a map on every project holding the
same secret, and the one thing a project's own map exists for — that the
project's jobs reach what the operator loaded for that project — is what
variables are.

## Consequences

**A snapshot migration in three places, all exact.** The agents map becomes
a purse, every kit gains the purse it charged, and every job's kit does too.
None of it is a guess: one credential existed and every kit charged it.
The literal older-file test `docs/conventions.md` §4 requires is extended
rather than replaced.

**The adapter's one environment function becomes two**, one for what a
container is created with and one for what a turn is run with, and the
command that runs the agent names the purse's variables and the ones it
clears. No recorded flow runs a turn, so the replays stand as they are:
the scenarios pin what a container is made with and what its agent's
process is given, and a container test pins what a runtime does with the
command, on each runtime the machine it runs on has.

**A handout has nowhere to put a purse.** What a container is made with
and what a turn is run with are decided by two functions and carried in
two types, so that a container holding the purse is not a state to check
for; and the names an operator may not claim are derived from the table
both deliveries read.

**A container made before this record keeps the credential it was made
with**, in its own configuration, for as long as it lives. Nothing run in
it reads that any more, and nothing here removes it: a job's container
goes when the job is retired, and a foreman's when its project is
forgotten or its agent changes.

**One more crate**, **provider**, with the read that checks a key and the
rule that refuses a shape, a line in the browser pass's exclusion list, a
bullet in `docs/architecture.md` §1 and a clause in its dependency rule:
**provider** may name **core**, and **instance** may name it.

**The wire grows**: the providers and their purses beside the agents, a
purse on a fitted kit and on a shape, and refusals for a purse that is
unknown, misshapen, refused, unchecked, still charged, or not held.

**The gate holds no purse through the binary.** A purse is kept only once
its provider has accepted it, and nothing the gate could paste is one a
provider would accept. So what the binary's own tests ask of the route is
what it does with a paste it refuses, a write that lands comes from
forgetting a purse rather than from holding one, and the simulated
provider is where a purse is kept, refused and left unchecked. A real
purse kept through the binary, and a made-up one refused in the provider's
own words, is a test that needs a credential, run with the others that do.

**The query that says what depends on an agent becomes the query that says
what charges a purse**, and it counts jobs that are not over. It stops being
equivalent under mutation the moment there are two purses, so the
attribute that skipped it goes in this change rather than with the second
agent, which is earlier than 0048 expected and for a reason it did not
have.

**The foreman is told nothing new.** A kit's description is what it chooses
by, and who pays is the operator's to write into that description where it
matters, as cost already is.

**`README.md` says it**: what a purse is, where each kind comes from, and
that a key is checked when it is pasted.

**Reversing** is folding each provider's purses back into a credential per
agent, which loses the second purse of any provider that holds two, and
putting the credential back at creation, which loses rotation.

**Revisit if** an agent's credential is a file rather than a variable —
Codex's and Gemini's logins are — which is when *the shape that provider
hands out* stops being a token and the purse's delivery needs a path; if a
provider's shape has several fields, as a cloud backend's does, which is
the same trigger from the other side; if an operator wants two keys at one
provider, which is the attribution question arriving as a credential
question; if spend ever becomes a decision the instance makes, which is
when a purse needs a budget beside its secret; if a runtime's exec stops
forwarding a bare name, or an image comes without an `env` that clears
one, which the container test says and which is when the per-turn
delivery needs a second path; or if
the protocol's providers extension stabilises and a second adapter
implements it, which is when delivering a purse per session in a header
becomes the smaller thing.
