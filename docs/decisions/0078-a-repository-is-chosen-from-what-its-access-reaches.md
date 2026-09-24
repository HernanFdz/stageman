# 0078 — A repository is chosen from what its access reaches

## Status

Accepted. Amends
`docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`,
whose settings card offered a token box beside a repository box and a link
onto the App minted per project: the link and its state, the repository
rule applied on its return, the repositories offered on the card and
creation without a token are replaced by what is decided here, and that
record's status says so. Narrows
`docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
in one place: a token is checked when it is set, in the form, as well as
at save. Takes up the moment `docs/conventions.md` §3 named for the first
long set a form here chooses from, and decides against lifting the
registry's control for it.

## Context

What 0077 built works and reads wrong. The GitHub card is a repository
box and a token box, with a link onto the App as the card's aside: the
operator types an address, then pastes a token or leaves for the platform,
and the shape the project is in — a token, or an installation — is a
sentence under the card's title rather than anything the form shows.
Three things were found wrong with it at the desk, and one on the
platform.

**The form does not say what the project holds.** A project's access is
one of two shapes, in the domain since 0077, and a form of two boxes and a
link says neither which shape it is in nor that the two exclude each
other. A token pasted under a line saying the project is reached through
the App replaces the installation without a word.

**The repository is typed, and could be chosen.** Whichever shape a
project is in, the platform can say which repositories the access
reaches. An installation's token lists what the installation covers,
exactly. A token's listing of the operator's repositories was measured on
2026-09-24: for a fine-grained token granted one public repository it
listed thirty-one, every public repository the operator owns or belongs
to an organisation for and none of the account's thirteen private ones.
A read of a public repository the token was not granted answered `200`
with the same account-derived permissions block as the granted one, seven
further reads behaved identically on both, and a write to the ungranted
one — a dangling blob, the smallest write the platform has — answered
`403 Resource not accessible by personal access token`. A token granted
one private repository listed it, measured the same day. So a fine-grained
token cannot be asked which *public* repositories it was granted and can
be asked which private ones: a list of what a token can read always holds
the right answer, and for a public entry cannot say whether it is
granted.

**Creating and amending are not one form.** 0077 let a project be created
without a token once an App was registered, to be installed on from its
settings afterwards, because an install is a trip to the platform and back
and a project being created has nowhere to come back to. The platform's
documentation settles why: the setup address is one field on the App's
registration — "enter the URL to redirect users to after they install
your app" — and the install link carries only a `state`, which can name a
project and cannot name one that does not exist yet.

**An installation is per account.** On the platform the App is installed
on an account, for some or all of its repositories, and two projects on
two repositories of one account share the installation. 0077 minted a
state per project and routed the return by it. What the platform does
when the App is already installed on the account the operator picks is
undocumented, and whether the state survives an update made from the
platform's own settings page is not stated either; a design that depends
on either is one a click can break.

The mechanism this instance already has for a page learning that
something changed is the tick of
`docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`: a write of
the file, told to every open page, which re-reads. A form's draft lives in
a hook the re-read does not reset, so a page can learn of a change without
losing what is typed in it.

## Decision

**A project's access is set before its repository is chosen, and the
repository is chosen from what the access reaches. Where the App is
installed is kept beside the App, learned from the platform's redirects,
so that an install is a tab opened from the form, which comes back under
a state minted for that press, and the form learns of the return through
the tick. A form is shown what its own access reaches and nothing of the
instance's other installations.**

- **The GitHub card is the enum.** A project's access, as the form holds
  it, is one of three things: nothing chosen, the App with a repository
  chosen on the installation the form may name, or a token with a
  repository chosen from what it can read — the repository inside the
  shape, so that a repository with no access cannot be said, and none
  once the shape no longer reaches it. The Access field's control is a
  sentence that says which of the three the form is in and offers,
  inline, what can be done about it: *install the App*, *use a token*,
  *replace it*, *install it elsewhere*, and *go back* to the other shape
  where the form remembers one. One sentence per shape, the same whether
  what the form holds is the project's or was set in it, since the whole
  form is unsaved until Save and no other field says so; the App's names
  the account the form's own installation is on and no other. Under it
  the Repository field is a box that filters what the access reaches,
  disabled until it has something to list. A new project starts with
  nothing chosen. The lines under both labels say what each field is for
  and never change; it is the control that says where things stand.

- **The form remembers the other shape.** What the form holds for each
  shape — the installation its tab brought back or the project's own, the
  token set here or the project's own, with what each reaches — stays
  while the form is on the other, so that going back finds it as it was
  rather than asking for it again. The current shape's is live and the
  other's is a memory; the repository is one field, under the rule below,
  whichever way the form moves.

- **The repository follows the shape.** Leaving a shape drops the
  repository; arriving at one fills it back in where the shape's listing
  holds it, and otherwise says that the repository the person had is not
  reached this way and waits for a choice — never putting another in its
  place, since a repository the person did not choose is not theirs to
  save. Where they had none — a new project, or an install just made from
  the form — the one repository the listing holds fills in where it holds
  exactly one. The same rule runs when a listing arrives later, through
  the tick, so an install made from the form ends with the form set up.
  A project opened whose access has stopped reaching its repository — an
  installation narrowed on the platform, a token re-granted — is told the
  same over the field's line and is not changed, so an edit to anything
  else still saves; the repository is fixed when somebody chooses one.

- **Where the App is installed is a fact kept beside it, and which page
  an install came back to is a state's.** The platform's setup redirect
  carries an installation's identifier; the instance holds the browser's
  request, fetches the installation with the App's key — which is the
  check, since an installation of another App cannot be fetched with it —
  and keeps the identifier, the account, and whether it covers every
  repository or chosen ones, under the App, before the tab is answered.
  The update redirect refreshes the same record and drops the tokens
  minted on the installation, so a repository removed from it fails the
  next command loudly rather than an hour late. The Instance page lists
  the installations, each forgettable while no project names it. The
  install link is minted per press and carries a state, which says
  nothing about the installation — the key is the check, whatever the
  state — and everything about who asked: the instance holds it, and the
  installation that comes back under it is named there, so the page that
  pressed can ask what its own tab brought back and no other page can.
  A form never names an installation by its identifier and is never shown
  the instance's installations: what it lists is the one that came back
  under its state, or the one its project holds. The platform's own rule
  that only an account's administrator can install there is what makes
  the arrival an entitlement, which listing the instance's installations
  to a form would have thrown away. A save spends the state; one the
  instance does not hold — never minted, minted before it last started,
  or spent — is refused before anything is asked. Nothing here makes two
  projects share an account, and nothing forbids it: the second presses
  *install*, the platform shows the account as already installed, and
  saving there brings the tab back the same way.

- **An install opens a tab of its own, which closes itself when the
  platform brings it back, and the form learns of the return through the
  tick.** "Install the App" opens the platform in a tab opened by script
  rather than by a link, for one reason: a tab opened by script may be
  closed by script, and one opened by a link may not once the platform
  has moved it through its pages. The redirect lands on the instance's
  own path, which answers with a page of the instance's own — outside the
  dashboard, one sentence and one line of script — that says the App is
  installed and closes the tab, so the person is back where they were;
  where the platform refused, the page stays open and says why; and where
  the installation was kept under a state the instance does not hold —
  a tab opened before the daemon last started — it stays open too,
  saying to press again, since no page will learn of it. The write of the
  installation is the tick: the form that pressed asks on each tick what
  came back under its state, and moves onto the App when something has,
  listing what that installation reaches, with the draft as it was typed.
  Until then the form stays as it was, so a tab closed without installing
  changes nothing, as a panel cancelled changes nothing. The tab is
  opened in the press and sent to the platform once the link is minted,
  because a browser opens a tab for a press and refuses one for what
  comes later. So creating and amending are one form: neither leaves the
  page, and a project needs no token at creation because it needs its
  access, in either shape, before its repository. The Instance page
  lists the installations all the same, for whoever installs from there,
  and nothing sends a person to it.

- **What an access reaches is listed on demand, and kept nowhere.** A
  form asks the instance what its access reaches — the installation that
  came back under its state, a token it is about to set, or what the
  project holds — and the request is held as 0076 holds a check: for an
  installation, a token minted for it for listing and held until its hour
  is nearly up, then that installation's repositories, with the account
  it is on; for a token, one read of what it can read. Each repository
  crosses with whether it is private. Nothing about coverage is kept:
  what is kept is where the App is installed, and coverage is asked of
  the platform while a person is at the form and can act on the answer.

- **A token is set in a modal, checked there, and checked again at
  save.** "Use a token" opens a panel of one action, which
  `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`
  allows: the box, the guide onto the platform's form, and a submit that
  lists what the token can read before the panel closes. A token the
  platform does not accept is said beside the box — as the answer to the
  question the panel asked, not as a refusal of the request, so that the
  browser logs nothing for it; one it does closes the panel, becomes the
  draft's access, and fills the repository list.
  The token lives in the browser's draft until save, when the instance
  checks it once more against the repository chosen — the read of 0076 —
  and keeps it. Two reads per token, and a request written by hand cannot
  skip the first, because the second is what keeps.

- **The list says what it can and no more.** Every row carries the
  repository's visibility as a mark: a lock for private, a globe for
  public. Under the App the list is exact. Under a token the field's line
  says the list is what the token can read: a lock row is certainly
  granted, because it is listed for that reason alone, and a globe row may
  not have been. The check at save refuses a private repository the token
  was not granted, and a public one it was not granted fails at the first
  push, as it did before this record. The mark is what makes the list
  honest where a sentence would be skipped.

- **The save refuses what the access does not reach.** Whichever the
  shape, the instance checks the repository against the access before
  anything is kept: a token by the read of 0076, an installation by
  minting a token restricted to the repository, which the platform
  refuses as unprocessable when the installation does not cover it. A
  kept access is checked again when only the repository changed. An
  installation the App does not hold, or a state nothing has come back
  under, is refused before anything is asked.

- **The repository's control is this project's own.** The registry the
  framework publishes has a combobox, and lifting it was measured to mean
  a git dependency on the primitives crate, an icon crate of its own and a
  theme file of its own, in the browser's half. So the one long set here
  gets a control written in the token set, with its list in the page
  rather than floating: a box that filters and holds the choice, and rows
  shown while it is typed in, a handful tall and scrolling beyond.

Rejected: **the token's listing as what the token was granted.** Measured
otherwise, above; the list is labelled for what it is.

Rejected: **holding a creating form's draft on the server** under the
install link's state, and restoring it on return. The draft carries the
binding's two tokens, which would be sent back to a browser, against
`docs/decisions/0022-the-browser-never-sees-the-domain.md`.

Rejected: **polling from the form** for the installation's arrival.
`docs/conventions.md` §3 refuses a page that reads on a timer, and a tick
needs a write, which is what keeping the installation supplies.

Rejected: **a popup that reports back** to the form through its opener.
The platform sets no opener policy, measured on 2026-09-24, so it would
work; but it is a protocol between two windows to carry news the tick
already carries. What is kept of it is the window opened by script, for
the one thing that buys: the landing page can close it.

Rejected: **landing on the Instance page** after an install, which the
first build of this record did. It assumes whoever creates a project may
see that page, which a second person on one instance need not, and it is
not where the person was.

Rejected: **listing the instance's installations to a project form**,
grouped by account, with the row chosen naming the installation, which
the first build of this record also did. It shows every account's
repositories to whoever can open a form, and lets a form name any
installation the App holds, which is wrong the moment a second person
operates the instance; and it put the accounts in the sentence for
nothing chosen, where a person has no business seeing them. The
state-bound arrival makes the platform's own administrator check the
entitlement, at no cost to the person who installs.

Rejected: **an installation per project, with a state routing its
return.** The platform's own model is per account, the setup address is
one, and a project being created has no identifier for a state to name.
The state kept here names a press, which every page can make.

Rejected: **a state minted with the page's read**, ready for the press.
A page reads on every tick, and a state per read is a state per write of
the instance's file; minted for the press, the tab is opened blank in
the press and sent on when the link arrives, which costs a blank tab for
the length of one request.

Rejected: **keeping what each installation covers.** It goes stale the
moment a repository is added on the platform's own page by somebody whose
browser does not land here, and the save re-lists anyway.

Rejected: **hiding the repository control** until the access is set. A
control nobody has seen is one nobody learns exists; disabled, with its
line, is the form saying its own order.

Rejected: **a typed repository box under the token shape.** It has the
listing's weakness with infinitely many wrong answers rather than a few
marked ones.

Rejected: **lifting the registry's combobox**, for the price measured
above. `docs/conventions.md` §3 said to when the moment came, and the
moment found the price.

## Consequences

**The file grows.** The App carries its installations, defaulted for a
file written before it did; a project's access is as 0077 left it. The
domain refuses a project naming an installation the App does not hold.

**The wire says the shapes.** A draft's access is one of nothing, the App
— on the installation that came back under a state, or the one held — or
a token — the one held, or one set — each with the repository chosen, so
that a repository without an access cannot be sent and an installation is
never named by its identifier; a project crosses with its access as a
token or as an installation on an account; the projects screen says
whether an App is registered and no more of it; an install link is
minted per press with its state; a form asks what an access reaches and
is answered with rows carrying visibility and the account, with why the
platform would not list, or with nothing yet, as an answer; and the
refusals name the box, the access or the repository.

**The instance holds five more things**, held and never kept: installs
begun, by state; setup redirects being answered; forms' listings being
assembled; the tokens minted for listing per installation; and why the
last installation was not kept. The per-project link, the offered rows
and the per-project failures go.

**The platform crate renders one more read**, of what a token can read,
and reads visibility off both listings.

**The dashboard changes** on three pages: the settings page's card, the
Instance page's App card, and the projects list's note. One control joins
the primitives, and two icons the set; and the instance serves one page
of its own, for the tab that comes back.

**Scenarios pin**: the form told whether an App is registered; a link
minted per press with a state of its own, and an installation come back
under it confirmed, kept under the App, named under the state and listed
on the Instance page; what came back listed for the form holding the
state and not the App's other installations, with the listing token held
through its hour, nothing yet answered before the tab is back, and a
state nobody minted answered with why; a foreign installation refused
and said where it came back; an arrival naming none refused without
asking; an update refreshing the record and dropping the tokens minted
on it; an arrival under a state nobody here minted kept with the tab
staying; what a project holds listed for its own form; a token's reach
listed and a bad token answered with why; a project created on the
installation its tab brought back with the coverage checked by a
restricted mint, the state spent by the save, and refused where not
covered or where the state is nobody's; a kept access checked again when
the repository changes; the shape and the repository required; an
installation refused forgetting while a project names it; and a daemon
dying mid-arrival keeping nothing and forgetting the states minted
before it.

**Reversing** is 0077's card back: a token box beside a repository box,
the per-project link, and the offered rows. The installations kept under
the App can stay, since they are facts; the landing page goes with the
script that opens the tab, and the states with it.

**Revisit if** the platform starts saying which public repositories a
fine-grained token was granted, which is when the token's list becomes
exact and the mark can carry a verdict; if an installation is found gone
on the platform and still listed here often enough to matter, which is
when the Instance page wants a refresh from the platform's own list of
the App's installations; if an installation covering more than one page of repositories is met,
which 0077 already names; if the platform is found to drop the state on
the path through an account it is already installed on, which is when a
second project on that account needs another way onto it; or when a
second person operates one instance, which is when an installation come
back under a state should be that person's rather than the page's, and a
project's installation checked as theirs at save — the state names a
press today, and nothing checks whose the press was.
