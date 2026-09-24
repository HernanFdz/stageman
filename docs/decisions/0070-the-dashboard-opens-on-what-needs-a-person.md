# 0070 — The dashboard opens on what needs a person

## Status

Accepted. Gives the console
`docs/decisions/0005-conversation-happens-on-channels.md` describes a shape:
which page comes first, what each page is for, and what is a page rather than
a panel over one. Retires the instance page. Takes the link from the
dashboard to a job's room out of `docs/open-questions.md`, where it waited
among the smaller things the room design made cheap.
`docs/decisions/0071-a-page-learns-of-change-from-a-tick.md` and
`docs/decisions/0072-the-dashboard-has-a-dark-theme.md` are taken beside it;
the first is what makes any page here worth leaving open.

Amended when the job page was built. The workspace's address is what the
channel already asks on every connection and holds, rather than something
kept on the binding: a room is a link while the channel is connected and
text otherwise, which is the rule the tunnel is under and costs no field.
And the tunnel is shown whether or not something answers on it, for now:
the instance probes an idle job's tunnel to decide its container's fate and
keeps no fact of the answer, and never probes a working one, so showing it
only while something answers waits on the instance keeping that fact —
taken up with the moment a standing changed, which is the other fact this
record asks it to keep. Since
`docs/decisions/0074-a-jobs-identifier-is-its-name.md` a job's identifier
is its name, so the addresses that keep it read as one; the argument below
against a readable slug still holds for a project, whose name has no suffix
to make it unique. With it, a job's row and its page are titled by that
name, the reason is a hover away on the row and a paragraph of the page's
kickoff card, where the instruction is folded under it, and what the
session reported it was set to is shown on the page alone; the rules a row
and the page keep are in `docs/conventions.md` §3. A job that is over
shows no tunnel: the standing itself is the fact the probe never kept,
since nothing is behind the tunnel of a job whose container is gone. And
the panel below is kept for a confirmation *and* for a transaction that
is over in one action — starting a job, pasting a file of variables per
`docs/decisions/0075-a-variable-says-what-it-is-for.md` — which is not
configuration and has nothing to return to; configuration is still a
page.

Narrowed by
`docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`,
which checks a credential against its platform before it is kept. The
rejection below of checking a *claim* stands: that probe would have the
daemon act in a job's stead, unattended, in place of a person's reading. A
credential check is one read with the credential the operator just typed,
on their behalf, while they are at the form, and keeps nothing.

## Context

Four pages, flat: the instance, the agents, the projects, and one project's
jobs. Every one reads the instance once, while it renders. Read against the
questions an operator arrives with, on the development instance, they answer
none of them.

- **Nothing says what needs a person.** A job that asked, proposed, failed,
  was paused or fell silent is the reason to open the console at all, and
  finding one means opening each project in turn.
- **The landing page is the projects list without its controls**, beside a
  runtime path and a count of agents.
- **A project is edited in a panel over the list**: a name, a repository, the
  foreman's agent with its editor, a kit with an editor each, a brief, three
  credentials, the variables, and six paragraphs saying what each is for. A
  panel a few hundred pixels wide holds it badly, and a panel has no address.
- **Rows carry data as it is stored.** A timestamp as written, a kit line
  that names an agent twice because a kit seeded from an agent is named after
  it, a tunnel link whether or not anything answers, and a reason as the
  title even when the reason is that somebody pressed a button.
- **Retiring is one press on an icon**, and it removes the container and the
  session in it.
- **Rooms exist for the foreman and for every job**, since
  `docs/decisions/0061-a-job-has-a-room-of-its-own.md` and
  `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`,
  and nothing here links to one.

One fact about the domain decides the first page.
`docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md` made
*idle* the state a person does something about, with the reading saying
what: answer, review, fix, resume, look. So "what needs a person" is a set the
domain already has, across every project, and showing it needs no new concept
and no rule anybody keeps.

## Decision

**Home opens on the idle jobs of every project**, ordered by how long each
has waited, each with the verb its reading wants beside it. Then the working
jobs, then the projects with their counts and whether the foreman is
attending. On screen the first region is called *needs you*; in the domain
the set is *idle*; they are one set, by 0052's own definition. Ordering by
waiting needs the moment a job's standing last changed, which is recorded
from this change on and defaulted for every older job, so that an older file
opens and an older job says only that it waits.

**The machine is a line, not a page.** The runtime, the domain and the
version go on a status line in the shell, visible from every page, and the
instance page goes. All three are known at boot and cross the wire as plain
text.

**Navigation is Home, Projects, Agents.** Agents stays at the top because it
is the first step on a new instance, per
`docs/decisions/0021-an-instance-starts-empty.md`, and Home's empty state is
that checklist: an agent, a project, the invitation.

**Configuration is a page, never a panel.** A project has a settings page in
sections, and New project is that page with nothing filled in, at an address
of its own. The router prefers a static segment to a dynamic one, so the
address does not collide with a project's. A panel over a page is kept for
one thing, a confirmation, because a confirmation is a decision and not a
form.

**A job has a page**: its reason, the kit it runs on, what its session
reported, the instruction it began from, when it was made and when its
standing last changed, and the pull requests it claimed to have opened, per
the amendment this makes to
`docs/decisions/0055-a-job-says-why-it-stopped.md` below.

**Every reference is a link, and only a true one.** The repository; the
foreman's room and a job's room, through the platform's own redirect, which
needs the workspace's identifier, asked of the platform once at bind time and
kept on the binding; the tunnel, shown only while something answers on it,
which the instance already asks for the container's sake; and each pull
request, as the number a person reads,
with the address composed on the server from the repository and the platform.
The browser composes no address, per
`docs/decisions/0022-the-browser-never-sees-the-domain.md`: what shape an
address has is the platform's knowledge, and the platform set is closed.

**A job says which pull requests it opened, by number.** The tool of 0055
gains an optional list of positive integers, allowed with either reading,
because a draft opened before a question is still something to look at. A
number names a pull request on the project's repository and nowhere else; one
opened elsewhere has nowhere to be said, which is the right kind of
impossible. The job keeps the union of every number ever claimed, sorted, so a
forgetful later turn cannot erase an earlier one; whether any is still open is
the platform's to know, and the label says nothing about it. Composing the
address needs a repository that is one, so the repository becomes a typed
value: an address on the platform, parsed to its owner and name, with the
trailing suffix and slash stripped, refused otherwise. Today it is text
nothing checks.

**Retiring is confirmed and stopping is not.** One is reversible and one
removes a container, which is the distinction
`docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md` draws, now
drawn on the page as well.

**Addresses keep the identifier.** Names are not unique and the domain does
not make them so; a readable slug would be a rule on names taken for the sake
of an address bar.

Rejected: **keeping the instance page as the landing page.** It shows what
the projects page shows, without the controls, and two facts nobody acts on
from a page.

Rejected: **an area of settings holding the agents.** Tidy, and it hides the
first thing a new operator has to do behind a gear.

Rejected: **a panel for the project form, made bigger.** The form has as many
fields as a page and will gain more with each integration; a panel has no
address to link to, no place for a section, and nothing to return to.

Rejected: **inferring a job's pull requests from its transcript or from the
platform.** The first reads a claim out of prose not written to carry one,
which 0052 and 0055 both refused. The second puts a platform credential and a
network call on the daemon's side, the direction
`docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` reversed.

Rejected: **checking a claim against the platform.** A probe would say that a
number exists and nothing about whose it is; it would have the daemon act on
the platform, the same reversal of 0009; and a private repository answers an
unauthenticated probe with nothing. A wrong number is a link that does not
resolve, which a person notices, and a person reading the proposal is already
the whole of the check under
`docs/decisions/0002-never-merge-never-deploy.md`.

Rejected: **a branch or pull-request naming convention in the kickoff**, so
that a person could tell on the platform which job a pull request came from.
A repository's conventions are the project's business, per
`docs/decisions/0019-a-projects-tooling-is-the-projects-business.md`, and an
operator who wants one has the brief, per
`docs/decisions/0064-a-project-has-a-brief.md`.

Rejected: **a free list of links** on the same tool. It would point anywhere,
be validated as nothing, and render as an address instead of a label. It can
be added beside the numbers if a use ever arrives, and nothing here forecloses
it.

Rejected: **a page that talks to a job.** 0005 refused it and nothing has
changed: a job's page links to where the conversation is and never carries
one.

## Consequences

**The wire grows, and every addition is a field a page reads.** A view for
Home spanning projects; a version and a domain; the moment a standing
changed; a room's identifier and the workspace's; whether a tunnel answers;
the pull requests with their addresses. Each is plain text or a number, and
the test that reads the page looking for a credential covers them as it
covers everything else.

**Three defaulted fields land in the snapshot**: when a standing changed,
the workspace identifier on a binding, and a job's pull requests. All three
mean *unknown* when absent, which is the true answer for a file written
before them, so the older-file test of `docs/conventions.md` §4 is the whole
of the migration.

**The repository grows a type**, and a project whose repository is not an
address on its platform is refused at the form. An older file carrying one is
opened and the project shown, with the addresses that need it left out,
rather than refused over a field a person can fix on the settings page.

**The tool's schema and the prompt change**, so the snapshot tests of
`docs/conventions.md` §4 move.

**The integration test moves with the pages.** It pins the landing page
naming a project and the navigation marking the current page; both are still
true and both are asserted in a different place.

**Reversing** is a route table and a handful of pages. The defaulted fields
stay readable either way, which is why they are defaulted.

**Revisit if** a second person operates one instance, which is a different
product per `docs/vision.md` §3 and would want a page per person before a
page per job; if the projects outgrow one screen, which is when Home wants
a filter rather than a list; or if a job comes to carry more than a page can
show, which is when a page wants sections of its own.
