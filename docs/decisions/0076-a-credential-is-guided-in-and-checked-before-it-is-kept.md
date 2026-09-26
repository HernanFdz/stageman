# 0076 — A credential is guided in and checked before it is kept

## Status

Accepted. Narrows
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`, which
rejected checking a job's claim against the platform: a *credential* is
checked here, once, at the form, and the reasons that record gave do not
reach it — said below, because the two look alike from a distance. Adds a
crate beside **agent** and **channel** for the reason
`docs/decisions/0058-a-channels-adapter-is-a-crate-beside-the-agents.md`
added the second. Takes the setup guides out of `docs/open-questions.md`
§Next, where the dashboard pass listed them last. Taken during that pass,
at the three boxes on the project form that take a credential. Narrowed
by `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`: a repository token is checked when it is set, in a panel of
its own, by a listing of what it can read, and again at save by the read
decided below, against a repository chosen from that listing rather than
typed. Extended by
`docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`: the
check reads whose the token is as well, and keeps that and the token's
expiry beside it.

The Slack app the context looks ahead to is `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`, which found the redirect
half of the flow reaching a laptop and the two pastes the context names
still in it.

## Context

A project is created with three credentials — a token for its repository,
and the two a Slack app is installed with — and each is pasted from a form
on the platform's own site that the operator has to find, fill in and
scope by hand. This instance accepts each on the strength of its not being
empty. The first sign that one is wrong arrives later and elsewhere: a job
failing at its checkout with the platform's words in a log, or a listener
that connects, greets, and hears nobody, which is what a token of the wrong
kind produces. `docs/vision.md` §3 is about the hours nobody is at the
desk, and a credential that was wrong at three in the afternoon is found at
three in the morning.

Both platforms let a link fill their form in, which was checked against
their documentation rather than remembered. GitHub's fine-grained token
form takes a name of forty characters at most, a description, an owner,
an expiry — a number of days up to a year, or none, and thirty when
unsaid — and each permission by name; which repositories the token may
see cannot be given, and choosing the owner on the form is reported to
reset the permissions, so a person still reads the form before pressing.
Slack's app form takes a manifest, YAML or JSON, URL-encoded in the
address, and creates the app from it; what it does not do is mint the
app-level token that opens the event stream, which is generated on the
app's own page and by nothing else.

A flow that sends the operator away and brings them back with the
credential already in hand was looked for, because it is what an operator
wants, and the platforms differ. GitHub has one for an App and none for
a token: the App manifest flow takes a form this page could post,
redirects the browser back with a code the daemon exchanges within the
hour for the App's key, and an installation redirects back too, after
which a token is minted per turn and nothing is ever pasted. Slack has
half of one: an install redirect hands back the bot token and the
workspace, and its settings accepted an address on localhost with a plain
http scheme on 2026-09-23, whatever its documentation says about HTTPS;
but the app itself is created by a person or with a configuration token a
person generates and that lives twelve hours, and the app-level token
that opens the event stream has no API. Both flows belong to an app the
instance owns rather than to a project's pasted credentials, which is the
record after this one.

What each platform answers a wrong credential was measured on 2026-09-23.
Slack answers the two questions the listener already asks — who the
credential is, and where to connect — with a successful status and a body
saying `invalid_auth` for a token it does not know, and documents
`not_allowed_token_type` for a token of the wrong kind and `missing_scope`
for one without the scope. GitHub answers a read of the repository with
`401` for a token it does not accept, `404` for a private repository the
token was not granted, and `200` for a public one whether or not it was —
a fine-grained token reads what anybody can. The `permissions` object in
that answer describes the account and not the token: it said *admin* of a
token granted contents and pull requests alone. So a read can say that a
token is live, and that a private repository was granted to it, and
nothing about what it may write; and a public repository says only the
first.

The instance cannot make a request, per
`docs/decisions/0056-the-instance-decides-and-the-world-performs.md`. The
world makes one for it and answers with an event, and the channel crate
already renders both of Slack's questions and reads both answers, because
the listener asks them on every connection. GitHub had no such renderer
anywhere: no crate here speaks to the platform, since
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` has a job
reach it through its own tools.

The agent's credential is the fourth box, on its own page. The one
adapter's headless credential is a token its vendor mints for its own
client, and whether a request that costs nothing would say if it is live
was not measured, so nothing here decides it; the open question says what
would.

## Decision

**A credential is asked for beside a link that opens its platform's own
form filled in, and a credential that has been pasted is checked against
its platform before it is kept.**

- **A guide is a link and nothing more.** It opens the platform's own form
  with what this project knows filled in: for GitHub, a name — the
  project's, cut to what the form takes — and the three permissions a job
  needs, contents, issues and pull requests, write; for Slack, the manifest
  the app is created from. No description on the token: the name says
  whose it is, and a description would be this project's words kept in
  somebody's token list for a year. Nothing of this project's own stands
  in for either form, because the form is where the credential is minted
  and a copy of it here would restate the platform's fields and drift. The
  link is composed on the server, per 0070's rule that the browser
  composes no address, from the same text that governs everything else:
  the manifest is the channel crate's, and the copy `README.md` shows a
  reader is pinned equal to it by a test. What a form cannot be told, the
  guide's own words say: which repository to choose, and that Slack's
  app-level token is generated under Basic Information after the install.
- **Issues are among the permissions.** A job is as often asked about an
  issue as about a change, and commenting on one or closing it takes the
  same token; the widening is small, and a token without it would fail at
  the first such job with nothing on this page having said so.
- **The repository sits with the token.** The token is granted the
  repository and checked against it, so the two boxes are one decision and
  one card, and a project's name stands alone above them.
- **Expiry is left to the form.** The platform's default stands, and the
  operator lengthens it on a form where the choice is in front of them.
  Filling in a year would make the daemon's convenience the default, in
  the direction `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`
  asks to move away from; the repository is not filled in because it
  cannot be.
- **The check is an effect, and the request waits on it.** A request that
  carries a credential is held rather than answered: every credential in
  it is asked about at once, with the credential itself — the token
  reads the repository the form names, the bot token asks who it is, the
  app-level token asks where to connect — and the request is answered
  when the last answer lands. Refused, it is answered with the platform's
  reason beside the box the credential was typed in, and nothing is
  written. Accepted, it is kept as it was kept before, with the answer
  following the write. Each check has a budget of ten seconds, because a
  person is waiting at a button. What a check proves is exactly what the
  context says it can: that the token is live, and that a private
  repository was granted to it; that the bot token is a bot's; that the
  app-level token opens a stream. What it cannot prove — what the token
  may write — fails where it always did, at the first push, visibly.
- **A platform that cannot be reached is a refusal too.** The credential
  is not kept, and the answer says it could not be checked rather than
  that it was wrong, so the operator knows to try again. A credential kept
  unchecked would be a third state nobody can see.
- **Nothing is kept from a check.** Not the identity Slack answered with,
  not the fact that GitHub answered: the listener asks who it is on every
  connection, as it did, and a held request is held and never kept — a
  daemon dying mid-check answers nobody, and the next start knows nothing
  of it, which is the rule every held thing here is under.
- **A platform's shape lives in a crate beside the agent's and the
  channel's.** What is asked of a platform, what its answer means and where
  its own form is are the **platform** crate's, as pure functions the
  instance renders and reads, dispatched on the platform at its surface,
  with the inverse of what it renders so the simulated platform recognises
  a read without matching on strings. `Platform` is the domain's third
  closed set, closed for the reason the other two are, and 0058's argument
  for the second holds verbatim for it: not core, which holds no
  platform's shape; not the instance, where a third party's quirks would
  touch what decides; not the channel crate, because
  `docs/decisions/0027-a-channel-is-not-a-platform.md`.
- **This is not what 0070 rejected.** That record refused to check a job's
  claim about a pull request, because the probe would have the daemon act
  on the platform in a job's stead, unattended, standing in for the
  person whose reading is the whole of the check under
  `docs/decisions/0002-never-merge-never-deploy.md`. A credential check is
  one read, with the credential the operator just typed, on the operator's
  behalf, at the moment they are at the form and can act on the answer.
  Nothing here acts on a repository, nothing runs while nobody watches,
  and nothing is kept.
- **The agent's credential is not checked**, and the Agents page is as it
  was. The first turn is its check, and 0008 measured it failing in
  seconds with a precise error.

Rejected: **a flow that redirects back with the credential**, for now.
GitHub's is the App the dashboard pass names as the guides' destination,
and it is the record after this one rather than a widening of it: it
changes what a project holds, from a token to an App's key and an
installation. Slack's is the same shape one record later, an app the
instance owns and installs per workspace, with the two manual steps the
context names left in it; what would remove those is an open question.

Rejected: **checking in the browser.** The browser could ask GitHub
itself; the platform's shape would then live in the browser's half, which
`docs/decisions/0022-the-browser-never-sees-the-domain.md` keeps to plain
types, and Slack's answers would need the server anyway.

Rejected: **checking as it is typed.** A request per keystroke carrying a
secret, and a check of half a paste. The check waits for the save, which
is when the operator has finished.

Rejected: **keeping a credential the platform could not be asked about.**
A project created offline would hold a token nobody had checked and a page
that says nothing about it; refusing costs one more press when the network
is back.

Rejected: **checking what the token may write, by writing.** A throwaway
branch would prove the permission and have the daemon act on the
repository, which 0009 and 0070 both refuse and this record keeps refusing.
The platform offers no read that says it, which the context measured.

Rejected: **checking on a timer, or at startup.** That is the open
question about a credential expiring while nobody is watching, and a
different one: it needs a way to say so somewhere a person is reading, and
this record has the person at the form.

Rejected: **a form of this project's own for the token or the app**, with
the platform's fields restated. Each would be a copy of a form somebody
else maintains, wrong the first time that form changes, and the link fails
in the safe direction: a parameter the platform stops taking opens the
form unfilled.

## Consequences

**A save waits on the platform**, up to the budget, and the button says
what it is waiting for.

**The wire grows.** Refusals that point at the box a credential was typed
in, two for each — refused, and unchecked — and a part for the second
Slack box, which had none; a guide per platform on the projects view; and
the token form, named for the project, on each project.

**One more crate**, with a line in the browser pass's exclusion list, a
bullet in `docs/architecture.md` §1 and a clause in the dependency rule
there: **platform** may name **core**, and **instance** may name it.

**The instance holds two more maps**, held and never kept: which request
each check belongs to, and what each held request is waiting on. A crash
mid-check leaves nothing behind but a connection the world drops.

**The simulated platform answers a repository read** as GitHub was
measured to, with a scripted refusal or a scripted silence, so that a
refused token and an unreachable platform are scenarios rather than
prose.

**The request to GitHub names this project in a user-agent header**, which
the platform requires of every client.

**`README.md` says both**: where the links are, and that a pasted
credential is checked.

**Reversing** is deleting the hold, at which point every request is
answered in its own step again, and the crate; the links are a field a
page reads and can stay.

**Revisit if** the GitHub App the dashboard pass names as the guides'
destination lands, which makes the token an installation's and the check
that installation's; if a platform adds a read that says what a
credential may write, which is when the check can say more than *live*;
if operators are found setting an instance up with no network, which is
when the unchecked state above becomes worth a page; or when an adapter
can render a check for an agent's credential that costs nothing, which is
when the Agents page joins the rule.
