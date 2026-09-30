# 0086 — A kit charges a purse at a provider

## Status

Accepted. Amends `docs/decisions/0008-one-credential-per-agent.md`: its
letter — one credential per configured *agent* — becomes one purse per kind
per *provider*, and every line of its reasoning stands: what a process is
handed is constructed, never inherited, and exactly one credential is ever
present. Extends `docs/decisions/0048-a-job-runs-on-a-kit.md`, which left the
room this fills: a kit's variant now carries the purse it charges, and the
handout selects one credential by it. Answers two questions from
`docs/open-questions.md`: when credentials move from agents to providers,
and, for a key, whether an agent's credential is checked before it is kept.
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

Two measurements decide the delivery. On Docker 29.4.1, a bare `--env NAME`
on `exec` forwards the variable from the client's environment exactly as it
does on `create`, and `inspect` shows only what the container was created
with. So a credential can be given to the agent's process on every turn
rather than to the container once, and never appear in the container's
configuration. Podman documents the same semantics for both verbs and was
not measured, which the container test that pins this must do on both.

One measurement decides the check. Anthropic's models listing answers a
bogus key with `401` and *API key is invalid.*, and no key with `401` and
*x-api-key header is required* — one read, with the credential, costing
nothing, which is exactly the shape 0076 checks a platform credential by.
A real subscription token answers the same listing with `200` when sent as
a bearer token, with or without the beta header the vendor's own client
sends, and with the same `401` as a bogus key when sent in the key's
header. So both kinds are checked by one read, differing only in the
header the credential travels in, and the wrong box is refused by the
provider as well as by the shape.

## Decision

**A provider is a closed set. A purse is one credential at one provider, in
the shape that provider hands out, of one kind: a key, metered per token
and honoured by every agent that speaks the provider, or a subscription,
flat per month and honoured by the vendor's own agent alone. The instance
holds at most one purse per kind per provider. A kit names the purse its
work is charged to, from the subset its agent can charge, and the handout
selects the credential by it.**

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
  and the container's configuration does not. The project's variables and
  the job's warrant stay where 0046 put them, given at creation and fixed
  for the job's life, because they are part of what the job *is*. A purse
  is not: it is who pays, and a replaced key reaches a foreman at its next
  turn without losing the session, reaches a job when it resumes, and is
  the answer to a credential that expires while nobody is watching. A
  foreman's container is kept across a change of purse for the same reason
  it is kept across a change of model.

- **The kind is declared, the shape is refused before anything is asked,
  and both kinds are checked before they are kept.** The Agents page takes
  a key and a subscription token in different boxes, so the kind is what
  the operator pressed rather than what a prefix suggests, and a value
  whose shape belongs to the other box is refused where it was typed. What
  passes is then checked by the provider's listing, on 0076's terms: one
  read with the credential in the header its kind travels in, refused with
  the provider's own words, or unchecked when the provider could not be
  reached, and kept in neither case.

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
  hidden when exactly one fits. A chip's sentence names the purse, and
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
command that runs the agent names the purse's variables. The replays
re-record, because an effect's argument list changed, and the diff is
reviewed as behaviour.

**One more crate**, **provider**, with the read that checks a key and the
rule that refuses a shape, a line in the browser pass's exclusion list, a
bullet in `docs/architecture.md` §1 and a clause in its dependency rule:
**provider** may name **core**, and **instance** may name it.

**The wire grows**: the providers and their purses beside the agents, a
purse on a fitted kit and on a shape, and refusals for a purse that is
unknown, misshapen, refused, unchecked, still charged, or not held.

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
when a purse needs a budget beside its secret; if Podman's exec is measured
to differ, which is when the per-turn delivery needs a second path; or if
the protocol's providers extension stabilises and a second adapter
implements it, which is when delivering a purse per session in a header
becomes the smaller thing.
