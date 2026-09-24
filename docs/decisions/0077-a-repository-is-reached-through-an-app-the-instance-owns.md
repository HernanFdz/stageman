# 0077 — A repository is reached through an App the instance owns

## Status

Accepted. Builds the second route
`docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
named as the guides' destination, and answers the open question on scoped,
short-lived platform credentials for the one platform there is. Amends
four records without reversing any:
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`, whose
job now *obtains* a credential rather than being handed one, and obtains a
short one;
`docs/decisions/0032-a-foreman-asks-the-instance-by-warrant.md`, whose
"a job's container is never given one" gains a warrant that buys exactly
one thing, the job's own credential;
`docs/decisions/0034-tools-are-served-not-shipped.md`, whose "nothing this
project writes goes in the image" stays true while one thing this project
writes goes into a *container*, at creation, from the instance driving it;
and `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`,
whose checkout now reaches the platform through the same door every later
command does. Gives
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` a
fourth navigation entry, for what the instance configures about itself.

## Context

Route 1 asks an operator to mint a token on the platform, scope it by
hand, paste it, and hold it for as long as the project lives. 0076 made
that as short as a pasted token can be. The token is still long-lived,
reaches every repository the operator grants it, and sits in each job's
environment for the job's life, which is the exposure 0009 accepted and
asked to narrow.

What GitHub offers instead was checked against its documentation rather
than remembered, and three things decide the shape.

**An App is registered by a form the operator's browser posts.** The
manifest flow takes a form with a manifest and a state token, creates the
App, and sends the browser back to the manifest's redirect address with a
code. One unauthenticated request converts the code into the App's
identifier, slug, client credentials, webhook secret and private key. The
manifest declares the App's permissions, whether it may be installed on
any account or only its owner's, and, if the App is to have a webhook, its
address. The code is good for an hour. Measured on 2026-09-24 with the
operator's browser: the platform refuses a webhook address on localhost
as unreachable over the public Internet even with the hook declared
inactive, which the documentation does not say, so the manifest declares
no webhook at all; and it did not refuse the redirect and setup addresses
on localhost in the same breath, which the first build's retry settles.

**An installation is what a project holds, and it says which repositories
it covers.** Installing the App is a link on the platform, with a state
token the platform hands back, and after it the browser is sent to the
App's setup address with the installation's identifier. The documentation
says not to trust that identifier from the redirect alone; fetching the
installation with the App's own key is the check, since an installation
that is not this App's cannot be fetched. That fetch names the account and
whether the installation covers all repositories or chosen ones, and a
second lists the repositories. So a project's repository is never typed:
it is the installation's one repository, or one chosen from its list.

**A token is minted per use, for an hour, for one repository.** A JSON Web
Token signed with the App's key — RS256, issued a minute in the past
against clock drift, expiring within ten minutes, issued by the client
identifier — buys an installation token that expires after an hour and can
be restricted at minting to named repositories and named permissions. That
is the credential 0009's open question asked for: minted per job, expiring,
and good for one repository.

Two facts about this project constrain where the token goes. A container's
environment is fixed when it is created and the container lives across
turns and across restarts of the daemon, per
`docs/decisions/0046-a-projects-variables-are-carried-never-read.md`, so an
hourly token cannot travel the way a pasted one does today. And the tools
endpoint of 0034 already authenticates every caller from inside a container
by a credential minted here and checked here, on a listener the world
binds, which is a door a container can ask a credential through.

The private key has to be the operator's. A public App owned by this
project's maintainers would put a key able to mint tokens for every
operator's repositories in one place, which `docs/vision.md` §3 refuses by
keeping everything on the operator's own machine.

## Decision

**The instance owns one GitHub App, registered from the dashboard by the
platform's manifest flow; a project reaches its repository either by a
pasted token or by an installation of that App; and whichever it holds, a
job fetches its credential from this instance when a command needs one,
and never carries it.**

- **One App per instance, registered from the Instance page.** The page
  posts the manifest to the platform: named *stageman*, which the
  platform's own form lets the operator change if the name is taken;
  homepage, redirect and setup addresses on this instance; contents,
  issues and pull requests write, with metadata read; no webhook, since
  signals arrive through Slack and the platform refuses an address it
  cannot reach; redirect on installation updates; and installable on any
  account or on the owner's only, which the operator chooses on the page.
  A private App installs on the account that registered it and nowhere
  else; a public one is installable by anyone, related to the owner or
  not, which is what an organisation's repository or a collaborator's
  needs, and it grants whoever installs it nothing on its own — only this
  instance holds the key, and only a project here can use an installation.
  An organisation's App is registered from the organisation's form,
  which is a box on the page. The browser comes back with a code and the
  state the instance minted; the instance holds that request as 0076 holds
  a check, exchanges the code, and keeps the App's identifier, slug, client
  identifier and private key, sealed as every credential is. The client
  secret and the webhook secret are not kept: nothing here authorises a
  user or receives a webhook, and a secret kept for nothing is a secret to
  leak for nothing.
- **Access to a repository is one of two things.** A token, pasted and
  checked per 0076; or an installation, by identifier, learned from the
  setup redirect and confirmed with the App's key. The project's GitHub
  card offers both: paste a token, or install the App, which leaves the
  page for the platform and comes back with the repository filled in — or
  with the installation's repositories to choose from, when it covers more
  than one. A project written by the last release holds a token and opens
  as one.
- **A credential is fetched, never carried.** A job's container is created
  with no platform token in its environment. In its place the instance
  writes, at creation and beside the checkout, a small wrapper ahead of the
  platform's tool on the path. When any command runs the tool, the wrapper
  asks the tools endpoint for a credential and runs the real tool with it
  for that one process; git asks the tool, as the checkout already
  arranges, so a push takes the same door. The endpoint answers with the
  project's pasted token, or with an installation token minted for the
  project's one repository and the App's permissions and kept until
  shortly before its hour is up, so a project mints about once an hour
  however many commands its jobs run. Both routes are served the same way,
  so no job holds a platform credential in its environment whichever its
  project uses, and the checkout of 0050 is the first command to take the
  door.
- **The wrapper is the instance's, not the image's.** It is written into
  the container by the instance driving it, at creation, which keeps 0034's
  rule that an image carries nothing this project writes and its reason:
  what a container runs is never older than the instance that made it.
- **A job is given a warrant for one thing.** The credential route is
  answered only to a bearer presenting a job's own warrant, minted when
  the job is, kept sealed on the job so that a restart knows it, and
  delivered into the container's environment at creation. It buys the
  job's own project's credential and nothing else: not the tools of a turn,
  which keep their own per-turn warrant, and never another project's.
- **The Instance page is the fourth navigation entry.** Home, Projects,
  Agents, Instance: what the instance configures about itself, which today
  is this App and its installations, and later the Slack app the next
  record builds. Named for what it holds rather than for who may touch
  it: nothing here has a second person yet, per `docs/vision.md` §3, and
  when one arrives what is restricted will be restricted by what it is.
  The status line of 0070 stays a line; the page holds what an operator
  changes. A new instance's first steps become agents, the App if the
  one-press route is wanted, projects, and Home's empty-state checklist
  says so.
- **Forgetting.** The App is forgotten from the page and refused while a
  project installs through it, as an agent is refused while a project
  names it. The registration on the platform is left for the operator to
  delete, linked from the page: this instance acts on the platform only to
  mint what it was installed to mint. An installation removed on the
  platform is a token that stops minting, which fails the next job's
  command visibly, as an expired pasted token does today.

Rejected: **a token in the environment at creation, refreshed by a timer.**
The instance would rewrite a file in every running container of a project
before each hour was up, and a token would rest in the container for an
hour with nothing asking for it. Fetching on demand has no timer and
nothing at rest.

Rejected: **the wrapper in the image.** It is the program 0034 removed from
the image, back in the image; the instance writing it per container is
what keeps it at the instance's version.

Rejected: **asking the agent to refresh a token**, through a tool it calls
on an authorisation error. A model managing secrets is fragile, and any
token that passes through a tool result lands in the transcript, which
since
`docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
is posted to the job's room as it happens.

Rejected: **user-to-server tokens through the device flow**, which need no
redirect and would work anywhere. They act as the operator rather than as
the App, reach every repository the operator can reach, and need the
client secret to refresh. A token that is the App's is scoped by minting.

Rejected: **a public App owned by this project.** The key would be the
maintainers', for the reason the context gives.

Rejected: **keeping the client secret and the webhook secret.** The first
serves user authorisation this record refuses, the second webhooks it
declares inactive.

Rejected: **webhooks for installation events**, which need an address the
platform can reach. The setup redirect on updates says what a webhook
would, through the operator's own browser.

## Consequences

**The platform crate grows a second question and a signing key.** It
renders the manifest, the exchange, the installation fetch, the repository
listing and the token minting, reads each answer, and signs the JSON Web
Token with the same cryptography the world already links for TLS. Signing
draws randomness for blinding and changes no byte of what it produces, so
a scenario's trace stays exact.

**Two paths under the dashboard's host are the instance's own.** The
manifest's redirect and the setup redirect are answered by the instance
before the proxy, as 0057's request path allows, each held while the
platform is asked and then answered with a redirect to the page that sent
the operator away.

**The file grows and bridges.** The App, sealed; the installations the
instance has learned; and a project's access as one of two shapes, with a
bare sealed token opening as the first. The older-file test of
`docs/conventions.md` §4 carries each.

**The handout changes shape.** It carries no platform credential and
carries the job's warrant instead; the adapter delivers the warrant and the
wrapper. The isolation bar of `docs/conventions.md` §4 gains a container
test: a job's wrapper fetches its own project's credential and is refused
another's.

**The tools endpoint gains one route**, answered by the project's kind of
access, and held while a token is minted.

**Scenarios pin the flows**: a registration exchanged and kept, a
registration the platform refuses, a setup redirect with a foreign
installation refused, a repository derived from an installation of one and
offered from an installation of several, a token minted once and served
twice within its hour, and a daemon dying between the redirect and the
exchange, which keeps nothing.

**The Instance page and the settings page's GitHub card change**, the
navigation gains an entry, Home's checklist a line, and `README.md` says
what an operator does for the one-press route.

**Vocabulary gains three words** in `docs/conventions.md` §2: the *App*, the
*installation*, and a project's *access*.

**Reversing** is keeping every pasted token as it is, deleting the App and
its installations from the file, and delivering the token into the
environment again. A project on an installation would have to be given a
token by hand, which is the migration this record avoids by keeping both
shapes.

**Revisit if** the platform starts carrying the repositories in the setup
redirect, which removes a fetch; if a turn is found running commands
across the hour so often that minting shows in a log, which is when the
cache wants a longer horizon; if an operator's repositories span accounts
the public switch does not cover, which is when a second App per instance
becomes right; or when a second person operates one instance, which is
when the Instance page is the first thing to restrict.
