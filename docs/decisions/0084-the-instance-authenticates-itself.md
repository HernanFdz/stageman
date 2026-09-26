# 0084 — The instance authenticates itself

## Status

Accepted. Supersedes the authentication clause of
`docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` — that this
project authenticates nothing and whatever forwards the domain does — and
leaves that record's tunnel and routing standing. Needed by
`docs/decisions/0083-hosting-is-one-instance-per-customer.md`. Amends the
request path of
`docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
in two places: the door answers a fourth path itself, and a proxy answer
says which cookies to strip. Narrows
`docs/decisions/0021-an-instance-starts-empty.md` by one thing a first run
now sets.

Names below that this project does not yet define are unbackticked
deliberately, for the reason 0042 gives: `just drift` resolves a backticked
identifier against this source.

## Context

0042 spent the loopback default and answered the authentication question
with the infra: whatever forwards `*.<domain>` authenticates every host
under it, uniformly, and this project none. It rejected authenticating here
as a login page, a session and a credential store "in a process whose entire
security model to date is that it is not reachable", and named its own
revisit trigger: if stageman ever has to run somewhere its operator cannot
put a proxy in front of it. 0083 is that: a hosted instance faces the open
Internet on a machine the customer never sees, and a self-hoster who gives
the daemon a domain today is told in `README.md` to put authentication in
front and mostly does not.

Four facts about the code decide the shape, read on 2026-09-26.

**The door sees every request whole and already answers paths of its own.**
Since 0057 the instance takes the address a person types, reads the head of
every request, and either proxies it or responds; three paths — the two
GitHub landings and the Slack one — are answered by the instance itself,
with any status and any headers.

**A server function cannot set a cookie.** Its request reaches the instance
as a typed value with no headers, and its answer goes back the same way, so
nothing a page calls can establish a session.

**The relay forwards the Cookie header to a container unchanged.** A cookie
the browser sends to a job's host reaches whatever the agent left listening
there.

**The generator is a cryptographically secure stream seeded from the
operating system**, and it already mints every warrant, per 0057.

Three facts about browsers decide the cookies. A cookie whose name starts
with `__Host-` must be Secure, carry no domain attribute and have the root
path, so it is sent to exactly one host and no page on another host can set
one that it receives. A page on a job's host may otherwise set a cookie for
its parent domain, which every host under that domain receives. And a
cookie marked SameSite Strict is not sent on a top-level navigation from
another site, which is exactly what the platform landings are; Lax is sent
on those and withheld from a cross-site form post.

## Decision

**The instance authenticates every request on the dashboard's host and on
every job's host with a session it minted, established by one password per
instance, checked at the door.**

- **One secret, no username.** Minted strong, and changed on the Instance
  page against the current one. There is no second person to tell apart;
  when one arrives, the user is the thing that gets added, and the password
  becomes theirs.
- **Hashed with argon2id and sealed** in the file like everything
  credential-shaped, so the rule stays one rule. Hashing is an effect the
  world performs, never work inside a step: argon2 is slow by design, the
  step must stay short, and a flood of wrong passwords hashed inside the
  loop would stall every turn and every page. It is deterministic given its
  salt, so a scenario answers it and replays stay exact.
- **How it is first set.** A variable named STAGEMAN_PASSWORD, read at boot
  and honoured only while the file holds no hash, which is how a machine
  provisioned for somebody sets it without a person; the line that carried
  it is the provisioner's to remove once the daemon has started. A first run
  with neither prints a one-time setup link on the startup block, which
  opens the dashboard signed in and asks for a password. The environment path
  is the key's precedent from 0037; the link is the first-run story
  `README.md` has wanted.
- **The login lives at the door.** The login page is a page the framework
  renders, proxied without a session so that it is styled with the rest;
  its form posts to a path the instance answers itself, with the cookie and
  a redirect. Every other request on the dashboard's host without a valid
  session is answered with the login page, and the framework's routes are
  protected wholesale because the door is in front of them.
- **The cookie.** Named with the `__Host-` prefix, Secure, HttpOnly, SameSite
  Lax, root path. Lax rather than Strict, because the platform landings are
  top-level navigations from another site and Strict would strip the cookie
  exactly there; Lax already withholds it from a cross-site form post.
- **Sessions are held, not kept.** In memory, minted from the generator that
  mints warrants, expiring by the step's clock, gone on a restart. An
  upgrade logs everyone out and no session ever touches the disk.
- **A job's host is gated too**, uniformly, as 0042 insists, and needs a
  cookie of its own because a host-only cookie on the apex is not sent
  there. A request to a job's host without one is redirected to the apex,
  which checks the session and sends the browser back with a one-time
  token; the job's host exchanges it for its own `__Host-` cookie and
  redirects to where the person was going. The proxy answer gains a list of
  cookie names to strip before forwarding, so the container never sees the
  session and the world stays ignorant of what a cookie means. An upgrade
  to a websocket carries cookies and takes the same check.
- **A modest backoff per source**, by the step's clock, for wrong passwords.
  With a minted password guessing is moot, and the effect already bounds the
  cost of hashing; the backoff is there so that a flood is refused cheaply
  before it is hashed at all.
- **One path answers without a session**: whether the instance is up, for
  whatever supervises it, saying nothing else.
- **The same rule on a laptop.** A local domain is not exempt; the setup
  link makes the first login one click, and `just dev` sets the variable.

Rejected: **keeping the proxy as the only answer.** It still works — a proxy
that authenticates in front of an instance that also does is harmless — but
a product that faces the Internet cannot rely on a proxy the operator did
not put there, and 0042's own trigger said so.

Rejected: **basic authentication.** No logout, cached by the browser for
the session's life, no rate limiting, and a job's host would receive it on
every request.

Rejected: **a username with the password.** There is nobody to tell apart,
and a second field with one valid value is a lie about the model.

Rejected: **sessions kept in the file.** Survives a restart at the price of
session identifiers on disk beside the credentials they open, for a
convenience an upgrade would spend anyway.

Rejected: **hashing inside the step**, for the stall above.

Rejected: **one cookie with a domain attribute for every host.** It would
reach every job's container on every request, which is the one place a
session must never go.

Rejected, for now: **an identity provider.** The right answer for a second
person and the trigger below; a password is the whole of what one operator
needs.

## Consequences

**`README.md` changes when this is built**: the paragraph saying stageman
authenticates nothing goes, the first run gains the setup link, and the
domain paragraph stops asking the operator to authenticate in front.

**`docs/conventions.md` gains** the word *session* in §2, and in §3 the
rule that a session is a host-only cookie, never sent to a container, and
that a job's host is entered through the apex.

**The vocabulary gains** an effect for deriving a hash, answered by the
world, and the proxy answer gains what to strip.

**The file grows one sealed field**, bridged: an older file has no hash, and
opens as an instance whose password is not yet set.

**Scenarios pin**: a login, a wrong password and the backoff, a page without
a session answered with the login page, a job's host entered through the
apex with the token spent once, the cookie stripped before the proxy, a
restart forgetting every session, a first run setting the password from
the variable, and a first run printing the link.

**The hosted edge does only TLS**, which removes authentication from the
provisioning story entirely and makes a self-hosted and a hosted deployment
the same thing behind a certificate.

**Reversing** is removing the check at the door and the field from the file;
every session was held, so nothing else is on disk to migrate.

**Revisit if** a second person operates one instance, which is when a user
is added and the Instance page is the first thing to restrict, as 0077 and
0081 already say; if an operator wants to sign in with an identity they
already have, which is the identity provider above; or if a job's page is
found reaching the apex's session by a route this record did not foresee,
which is the one thing the cookie rules exist to make impossible and the
first thing to test.
