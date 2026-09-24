# 0079 — A repository is an owner and a name

## Status

Accepted. Finishes what
`docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md` began:
that record typed a repository's address in order to compose links from it
and left the project holding the text it was parsed from; this one makes
the project hold the address, and that record's status says so. Narrows
`docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`
in one place: the box the repository is chosen in shows *owner/name* and
never an address.

## Context

A project's repository was text: whatever the operator typed, kept as
typed, and since 0070 checked at the form to be an https address on the
platform. Everything that needed the address parsed the text again at the
point of use — the link on a project's row, a pull request's address by
its number, the token minted for the one repository, the checkout — and
each of those had a branch for text that does not parse: a link absent, a
pull request shown bare, a mint refused with a sentence of its own. None
of those branches could be reached from the form since 0070, and every one
of them was a place the same fact was decided again.

0078 changed where the text comes from. A repository is chosen from the
platform's own list, not typed, so what a project holds is an address by
construction; but the wire carried the address as text and a link beside
it derived from the same text, and the box showed a whole address for a
repository the access no longer reached, because that was what it held.

The file is the one place text has to be believed. The last release,
v0.6.2, wrote what its form accepted, and its form accepted anything, so a
file it wrote may hold a repository that is not an address — in theory;
the instances this project knows of hold addresses.

## Decision

**A project's repository is an address on the platform: an owner and a
name, held as such, composed into the platform's spelling only at the
edges, and shown as *owner/name*.**

- **The domain holds the address.** A project's repository is the typed
  address 0070 introduced, with a constructor from its two parts that
  checks each is something the platform would call an owner or a
  repository, and shown as *owner/name*, which is how the platform and a
  person both say it. Everything that composes from it — the link, a pull
  request's address, the name a token is minted for, the address a
  checkout clones — composes from the address, and the branches for text
  that was not one go.

- **The file's shape does not change.** A project is written as the https
  address this project spells, as the last release wrote it, and parsed
  back on opening. Text there that is not an address refuses the file,
  naming the project and the rule broken, rather than opening as
  something nothing can compose from: an instance running on a repository
  it cannot mint for, link to or clone is not an instance that survived,
  and the refusal names what to fix in a field that is plain text in the
  file. A literal file the last release wrote is opened both ways in a
  test: with an address, and with the text its form would have accepted.

- **The wire carries the two parts.** A page is given the owner and the
  name and composes nothing, per
  `docs/decisions/0022-the-browser-never-sees-the-domain.md`; the address a
  browser opens comes beside them, composed on the server, and is always
  there, as a pull request's link is always there. A draft names its
  repository by the two parts, so does a row of a listing, and so does the
  refusal that says the access does not reach one.

- **Prompts and the checkout are told the address as text**, composed at
  that edge, because a prompt is text and a clone takes a URL. What the
  foreman and the job crates render does not change.

Rejected: **keeping the text and parsing at each use**, which was the
state of things and the branches above.

Rejected: **a host in the type.** One platform exists, and the platform
set is closed. A second platform makes a repository's address a question
of which platform it is on, which is a platform on the address or a type
per platform, and is that platform's record to decide.

Rejected: **opening a file whose text is not an address and dropping the
project, with a line at startup.** `docs/conventions.md` §4 says a
project must survive the release window, and this looks like the way to
honour it; but a project without an address is not a project that
survived, and a dropped project loses what an operator typed where a
refusal keeps it in the file for them to fix.

Rejected: **refusing only where something composes from it**, at the
mint or the link. That is the branch this record removes, kept.

## Consequences

**The domain's address gains a constructor and a display**, and the
project holds it. The sealed form is unchanged, and opening it can refuse
one more thing.

**The wire says a repository as two parts** on a project, a job's page,
a project's jobs, a draft's access, a listing's row and the refusal for a
repository not reached, with the address beside it where a page links it;
a pull request's link is always present.

**Every fixture moves onto the platform's form of address.** The
placeholder host the tests used for a repository is gone from every
project a test builds, since a project can no longer hold one; what the
prompts and the Slack fixtures say about other things is untouched.

**Reversing** is the text back on the project and the branches back at
each use.

**Revisit if** a second platform is taken up, which is when the address
needs to say which platform it is on; or if a file the last release wrote
is met holding text that is not an address, which is when the refusal
above is the wrong cost and a bridge that keeps the project and marks it
is the right one.
