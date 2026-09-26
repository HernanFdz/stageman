# 0085 — The instance's apps may be held elsewhere

## Status

Accepted. Needed by
`docs/decisions/0083-hosting-is-one-instance-per-customer.md`. Amends
`docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`:
its rejection of a public App owned by this project's maintainers stands
for an App whose key would be *here*, and is answered on its own terms for
one whose key is held elsewhere and never enters this machine. Amends
`docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`:
the one connection per app-level token may be opened by whoever holds the
app, and a project then listens through that holder's relay in Slack's own
protocol. Leaves
`docs/decisions/0029-a-reply-is-routed-by-its-thread.md`'s rejection of the
Events API standing for the instance; a holder may use it upstream.

Names below that this project does not yet define are unbackticked
deliberately, for the reason 0042 gives.

## Context

A hosted customer should get the Slack app and the GitHub App with one
press each, the way every integration they already use installs, instead of
the three pastes and the manifest flow a self-hoster performs. That means an
app the host owns, installed on many customers' workspaces and accounts.
Where its secrets live decides everything, and five facts settle it.

**The customer's machine cannot hold a secret of the host's**, however
invisible the machine is. It runs the customer's untrusted code by design,
and a container escape lands as root beside the file and the key. A field
marking a secret as the host's, never shown, is a fence the attacker is
already past — and the App's key on every customer's machine would mint
tokens for every customer's repositories, which is 0077's rejected
concentration multiplied by the number of customers.

**A shared Slack app can be listened to by exactly one connection.** Slack
allows an app up to ten connections and sends each payload to any one of
them, which is the halving 0060 measured. So whoever holds the app-level
token must route, and the instance cannot hold it.

**A browser must return to an address registered on the app.** Verified
against both platforms' documentation on 2026-09-26: Slack's redirect must
match, or sit under, a redirect address registered on the app, over HTTPS,
on the same domain; GitHub's setup address is fixed when the App is
registered. Both carry a state through, so a press can be recognised on
return.

**What the instance does with each secret today is small.** The app-level
token makes one call, the open, which is also how the token is checked
before it is kept. The client pair makes one, the exchange. The App's key
signs the token for two, confirming an installation and minting a token for
one repository. Everything else — listing repositories, looking up the App's
bot, the checkout, posting, rooms, reactions, the socket itself — uses the
bot token or the installation token, which are the workspace's and the
account's own.

**Slack counts an unacknowledged event against the app, not the listener.**
Its documentation, read on 2026-09-26: acknowledge within three seconds; an
unacknowledged event is retried three times, at once, after a minute and
after five; and when more than 95 percent of deliveries fail within an
hour the app's subscriptions are disabled, with apps under a thousand
events an hour exempt — stated for both delivery methods. GitHub versions
its API by a header on each request, defaults to a fixed version without
one, and keeps a version alive for at least two years after the next.

## Decision

**An app has a holder: whoever holds its secret and acts with it. The
instance is the holder of an app of its own, as today. An app may instead
be held by a custodian, a service elsewhere that keeps the secret, serves
the platform's own calls under it, and decides only whose they are. The
instance then holds stand-ins for the secrets, in the platform's own shapes,
and nothing of the custodian's ever enters this machine.**

- **Stand-in secrets in the platform's own shapes.** For GitHub the
  custodian mints a key pair per instance: the instance holds the private
  half where it holds an App's key and signs the same token it signs today;
  the custodian verifies with the public half and reads the issuer to know
  whose it is, so the issuer is per instance rather than the App's. For
  Slack the stand-ins are an app-level token and a client secret, opaque
  strings the custodian looks up. Each is sealed like the secret it stands
  in for, useless anywhere but at the custodian, and revocable there.
- **One field per app: where its API is.** Absent means the platform's own,
  so every older file opens. The Instance page derives *held elsewhere*
  from it, names the holder by that address, hides the registration and the
  paste forms, and drops the sentence about deleting a registration on the
  platform.
- **An install link comes from its holder.** An app of the instance's own
  composes it locally, as 0078 and 0081 do. One held elsewhere asks its
  address for one, per press, sending where the browser should come back
  to; the link names the custodian's return address, since that is the one
  the platform registered. The instance's landing is unchanged and receives
  the platform's own query, forwarded whole.
- **Four calls change their address and nothing else changes.** GitHub's
  confirm and mint, in the App's token; Slack's open and exchange, with the
  stand-ins in the secrets' places. The version header travels with the
  request and the custodian forwards it, so an old instance keeps the shape
  it was written against. Every other call, and the socket, are
  byte-for-byte what they are.
- **The relay speaks Slack's protocol.** The open call answers with a socket
  address on the custodian's relay; hello, envelopes, acknowledgements and
  disconnects are Slack's, so the listening of 0044 and 0081 is untouched
  and a relay draining for a restart says what Slack says before a refresh.
  The custodian forwards each envelope to the instance whose workspace it
  names and passes the acknowledgement through while that instance answers
  in time, which is the normal path and stateless. When it does not — gone,
  or silent past two seconds — the custodian acknowledges on its behalf
  before the window closes and holds the envelope for the platform's own
  retry horizon, retrying toward the instance and then dropping it. The
  floor exists because one dead instance would otherwise count against
  every instance's subscription; below the platform's exemption threshold
  pass-through alone is safe, and the floor costs a few dozen lines.
- **One live instance per stand-in.** Two instances on one credential
  would split the stream and diverge two files, which 0011 already refuses.
  The custodian refuses the second connection in the open call's own error
  envelope, and the page shows the platform's word as it shows any refusal.
  Which instance is live is a control of the holder's, never a flag of the
  instance's.
- **The exchange is made at the custodian's landing**, where the code
  arrives, and answered from memory to the instance's later call, so the
  landing can name the workspace before it forwards.
- **Unbinding mirrors the platforms' own.** Slack's uninstall with the
  client pair, and GitHub's delete of an installation under the App's
  token, both at the custodian's address, unbind and pass through. A
  workspace reinstalled keeps its identifier, so without this an instance
  that started over could never reconnect it.
- **The install hijack is an accepted risk, bounded three ways.** A shared
  app has a failure an app of one's own cannot: whoever holds a stand-in
  can send a person a link to a flow they started, and if that person
  installs, their account or workspace is bound to the sender's instance.
  The defence every service takes is to check the returning person's
  identity at the landing, and there is no defence that does not, since
  anything the sender's browser can do the sender can make the victim's
  browser do, except present the victim's own credentials. Deferred, on the
  reading that a person installing an App from a link they did not initiate
  is rare, with these bounds: the custodian binds only an account or a
  workspace not already bound, so an existing instance is never taken over;
  its landing says which account was connected to which instance's address
  and takes one press before it forwards; and every binding is recorded
  with the account, the instance's address and the time.
- **Nothing of the custodian's is versioned.** The platforms' own shapes and
  versions carry through. What is the custodian's own is the link endpoint
  and the landing, under a path with a number that costs nothing. The
  instance names its release in its user agent so that a custodian can
  refuse one broken release with a message, which is a switch and not a
  policy.
- **A model credential can be held the same way, later.** An agent's
  credential would become a gateway's address and a key, each agent
  independently held or in custody, and the base-address variable would
  join the names 0046 refuses for a project's variables, since one
  overriding it redirects an agent's traffic exactly as an inherited key
  changes who pays. Recorded in `docs/open-questions.md` rather than here,
  because its terms are unread.

Rejected: **the host's secrets on the customer's machine, marked as the
host's.** The fence above.

Rejected: **a credential type of the custodian's own**, a bearer presented
where the platform's secret would be. It is a second authentication in the
platform crate's rendering and a second kind of credential in the file, for
nothing a stand-in in the platform's own shape does not do.

Rejected: **a grant of the custodian's own** exchanged for the workspace
record, and **a call registering a state** with the custodian. The
platform's own code forwarded whole, and the link asked of the holder, do
both with no new shape.

Rejected: **a signed state**, letting the instance compose the link itself.
It puts cryptography into a value that is a nonce everywhere else, to save
one call the holder abstraction makes natural.

Rejected: **a proxy for every GitHub call.** Two calls need the key; the
rest use a token the instance already holds, and a proxy for them would put
customer data through the custodian for nothing.

Rejected: **a framing of the custodian's own on the relay.** Slack's
protocol is what the instance already speaks, and a relay that speaks it
changes one address and no code.

Rejected: **acknowledging every envelope on the instance's behalf and
forwarding from a queue.** It makes the relay hold state for every event
and makes it better than Slack rather than transparent; the instance
acknowledges in the step the frame arrives, which is inside any window.

Rejected: **an app per customer created through Slack's manifest API with
events delivered over HTTP to the instance.** No API creates the app-level
token, so the instance would need an events listener it lacks; the manifest
API is beta and needs a configuration token a person generates; and a
second workspace still needs a press on the platform, per 0081.

## Consequences

**The file grows one field per app**, absent for every app the last release
wrote, and the stand-ins sit where the secrets sit.

**The platform and channel crates render four requests against an address
that may not be the platform's**, and read the same answers they read
today. The fixtures recorded from the real platforms serve both shapes.

**The Instance page says who holds each app**, offers *use an App of your
own* beside it, and shows a refusal from the holder with the holder's word.

**A holder elsewhere has a contract, and it is the platforms' own
documentation plus this record.** It keeps, per instance, the public half
and the stand-ins; it keeps the bindings of accounts and workspaces to
instances, and the last return address each instance gave, which is where
the browser goes after an installation update that carries no state; it
parses nothing beyond need — the token for identity, the workspace for
routing, the installation for binding — so that it stays a translator and
never grows into a second instance.

**Scenarios pin**: an install through a holder, from the press to the
landing's forwarded query; a mint and a confirm through a holder; an open
through a holder and the socket that follows; the refusal of a second
connection; an unbind; and the older file.

**Reversing** is dropping the field: every app held elsewhere becomes one
the instance must register or paste as its own, which is the setup a
self-hoster performs today.

**Revisit if** the App is listed publicly or a custodian serves more than a
handful of instances, which is when the returning person's identity at the
landing stops being deferrable; if Slack changes how it distributes
payloads among connections, which is the fact the relay exists for; if an
agent consumes tools over the protocol connection, per 0034, which changes
nothing here but is worth checking against; or if a platform adds an API
for what the holder does by hand, which may remove a mirrored call.
