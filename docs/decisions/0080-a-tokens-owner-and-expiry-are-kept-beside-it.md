# 0080 — A token's owner and expiry are kept beside it

## Status

Accepted. Extends
`docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`,
whose check reads one repository with the token and keeps nothing of
the answer: the check now reads whose the token is as well, and keeps
two facts the platform states beside the token. Extends
`docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`
in the same way at the panel: the listing that checks a token before the
panel closes carries the same two facts, so the form says them before
anything is saved.

## Context

A fine-grained token on the platform is made under one account and, for
every one made so far, expires on a date fixed when it is made: the form
that makes one takes an expiry, and regenerating one is a new secret. The
platform states both facts on every answer to a request made with the
token: the account from its own endpoint for the user, and the expiry in
a header, `github-authentication-token-expiration`, spelled
`2026-09-27 08:42:20 UTC` — measured on 2026-09-24 against the token this
repository's own tests run with — and absent for a token that does not
expire.

This instance kept neither. A token was a secret and nothing else, so
the settings page said *with a token* where the App's side said *installed
on acme*, and the first sign a token had expired was a job failing at
its next command, at whatever hour that fell. `docs/vision.md` §3 is about
the hours nobody is at the desk; an expiry is the one failure that is
known a week in advance.

## Decision

**A token is kept with what the platform said of it when it was checked:
whose it is, and when it expires. The settings page says both, and the
first page raises a token a week before it expires and after.**

- **Two facts beside the secret.** A project's token access carries the
  account it was made under and, where the platform said one, the moment
  it expires. Both are facts about the secret and cannot change under it,
  which is why keeping them is safe: a regenerated token is a new secret,
  set anew, and read anew. On disk they sit beside the sealed secret in
  the clear, since an account's name is not a secret and a moment is not
  one either; the bare sealed secret the last release wrote still opens,
  as a token nothing has been said of.

- **Read whenever the token is checked, and refused if unreadable.** The
  check of 0076 gains a second read, of the account, made with the token
  at the same time as the read of the repository; the expiry is taken off
  that answer's header. The panel's listing of 0078 makes the same two
  reads, so the form says whose the token is and when it expires as soon
  as it is set. An expiry the platform sends and this instance cannot read
  refuses the token, as a listing it cannot read would: a fact this
  instance acts on is the platform's word or absent, never a parse it
  shrugged at, and a header format that changes is found at the form
  rather than by a token that expired unannounced.

- **Said where the token is, and raised where a person looks.** The
  settings page's sentence becomes *With acme's token, expiring in three
  days* — the moment drawn in the browser, as every moment here is, per
  `docs/conventions.md` §3 — or *which expired three days ago*. The first
  page's *Needs you* lists every token within a week of expiring, and
  every one past, soonest first, before the jobs, each with the way to the
  page it is replaced on; whether a token is within the week is decided by
  the instance with the time it is given, on every read.

Rejected: **reading the expiry only, from the repository check's own
header.** The account is worth as much: it is the token's counterpart to
the installation's *installed on acme*, and it says which person's token
a project runs on, which is what a person asks when a token is about to
expire. One more read at a moment the platform is being asked anyway.

Rejected: **refreshing the two facts on a timer.** They cannot change
under the secret. What changes is the time, which the instance is given
with every step.

Rejected: **dropping an unreadable expiry and keeping the token.** The
easy path, and the one that turns a format change on the platform into a
token that expires with nothing said.

Rejected: **a notification on a channel** when a token is about to
expire. The dashboard is where a token is replaced, per `docs/conventions.md`
§3, and a token is a thing an operator fixes, not something a foreman is
told about. Worth revisiting once a token has expired in the night.

## Consequences

**The domain's token access is a struct**, with its two facts defaulted
on disk, and the bare form the last release wrote read as before. The
platform crate renders one more read and reads its answer, header
included, and the simulated platform answers it with an account and a
header where one is scripted.

**The check learns something for the first time.** A held check carries
what the account read said, and the request is answered with it kept
beside the token. The wire's token view carries the account, the moment
and whether it has passed; the listing carries the account and the moment
for a token as it does the account for an installation; the first page
carries the tokens raised.

**The moment component reads ahead** as well as behind, since an expiry
is the first moment the dashboard shows that lies in the future.

**Reversing** is the two fields dropped from the variant and the second
read from the check, with the bare form still opening.

**Revisit if** the platform is found to change a token's expiry without
changing the secret, which is when the facts have to be refreshed; if a
token expires in the night and nobody sees the first page, which is when
a notice on a channel earns its place; or if a platform whose tokens do
not expire is taken up, which is when the expiry is that platform's
absence rather than the token's.
