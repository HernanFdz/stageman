# stageman

An orchestration platform for sleepless coding agents.

A coding agent can take a well-scoped piece of work from a description to a
finished change. What it cannot do is start. Every unit of work waits for
somebody to notice it needs doing, judge that an agent could handle it, and sit
down to say so — which means the work that gets done is bounded by whatever
attention is left over, and overnight nothing happens at all.

The information needed to make that judgement is already there and already
written down: issues get filed, reviews get left, exceptions get reported,
people describe problems to each other in chat. stageman watches those channels
and closes the loop. When it decides something is worth acting on, it writes
down why, composes the instructions, and starts a **job** — one agent, in one
isolated workspace, on one project. If the job needs a human it asks on Slack,
not in a terminal nobody is watching, and stays alive while it waits.

## It never merges and never deploys

Work terminates at a proposal you review. That is not a setting and there is no
override, which is the point: because the boundary is absolute rather than
conditional, stageman never has to hold a credential that can land code or reach
production. The worst outcome of a bad decision made while you were asleep is a
proposal you decline.

## Running it

Running stageman means running one server executable on a machine you control.
It serves a dashboard and does the watching in the same process, so there is no
second daemon to supervise.

What it does need is a container runtime — Docker or Podman, installed the
ordinary way, and running. stageman runs agents rather than replacing them, and
it runs each one inside a container built with that agent already installed —
so the machine itself needs no coding agent, no repository tooling, and nothing
particular on its path. That holds for the agent the foreman thinks with
just as much as for the ones doing the work.

You do not tell it where that runtime is. It looks in the places each installer
puts one, checks that what it finds actually answers rather than merely exists,
and prints the path it settled on. A machine without one is a machine stageman
cannot work on, so it says which paths it tried and stops rather than starting
into a state where nothing can run.

A running stageman reads no credential from the machine's environment. Every
one of them is entered in the dashboard, held encrypted, and handed to a
container as it starts. Obtaining a credential is a one-time step you do
wherever you happen to be, and only its result goes into stageman.

**It asks you nothing to start.** A fresh instance has no agents and no
projects, and that is a perfectly good instance — it simply has nothing to do
yet. You give it those in the dashboard, in that order, because a project needs
an agent to think with and at least one its jobs can run on.

**It needs nothing named in the environment either.** Its file is encrypted
under a key, and a key cannot live in the file it protects — so on a first run
stageman generates one, keeps it in the ordinary place for configuration on
your platform, and says at startup where it came from. Set `STAGEMAN_KEY` to
the base64 key yourself and that wins instead, which is what a service manager
passing a secret in should do.

What the encryption buys is worth being exact about: the instance file is
useless to anybody who has it and not the key. It is not a defence against
somebody already running programs as you, and nothing kept on your own machine
could be.

Where that file goes is not your problem. It lands in the ordinary place for
application data on your platform, the directory is created if it is not there,
and the path is printed at startup so it is never a guess. Set `STAGEMAN_STATE`
if you want a different one — a second instance on one machine is the case that
needs it. What it will never be is relative to wherever the process happened to
start: a daemon under a service manager has a working directory nobody chose.

What gets reported is `STAGEMAN_LOG`. It takes the same filter syntax as
`RUST_LOG` and defaults to `warn` — enough to see what needs attention, not a
commentary on things going right.

## Installing it

One command. It works out which binary this machine needs, installs it, sets it
up to run as a service as you, and starts it:

```sh
curl -fsSL https://github.com/HernanFdz/stageman/releases/latest/download/install.sh | sh
```

It asks for `sudo` for the two files that need it — the binary, and on Linux
the unit beside it — and refuses to run as root outright. stageman runs as the
person who operates it and keeps its instance in that account's home, so a run
as root would install a service for root and put the instance somewhere you
cannot reach, which looks exactly like it worked.

**Re-running it is how you update.** There is no separate update command and no
`self-update` subcommand: the address above always serves the newest script,
that script is pinned to the release it was published with, and it replaces
whatever is installed and restarts the service. It tells you which version it
found and which one it is installing.

**Removing it** keeps your instance, which is the whole of what stageman knows:

```sh
curl -fsSL https://github.com/HernanFdz/stageman/releases/latest/download/install.sh | sh -s -- --uninstall
```

Each release publishes its own copy of that script, and the copy is fixed to
that release's binaries rather than to whatever is latest at the time it runs.
So an installation is reproducible: the script from `v0.1.0` installs `v0.1.0`
today and next year. `packaging/install.sh` is what it is built from, and it
differs by exactly one line — the version — which the build asserts rather than
promises.

The script needs a container runtime to already be there, and deliberately will
not install one: that needs root for something that is not stageman, and your
package manager does it properly. If stageman does not stay running, the script
prints what the daemon said, which names every path it looked in.

### Or do it by hand

Each release carries one binary per machine it is built on — Linux on x86-64,
macOS on Apple silicon — and the latest of each is always at the same address:

```sh
curl -Lo stageman https://github.com/HernanFdz/stageman/releases/latest/download/stageman-linux-x64
chmod +x stageman
./stageman --version
```

```sh
curl -Lo stageman https://github.com/HernanFdz/stageman/releases/latest/download/stageman-macos-arm64
chmod +x stageman
./stageman --version
```

**The asset is named for the machine and the file you keep is not**, which is
the same split the version already gets: an address has to say which binary it
is handing you, because nothing else there can, while a file sitting on the
machine that will run it restates what you already know. Ask it instead —
`--version` answers with the release and the target it was built for, without
starting anything, which is also the cheapest proof the download arrived
whole. Neither address ever changes.

**Fetch it with `curl`, not with a browser.** The macOS binary is not signed,
and macOS refuses to run an unsigned executable that arrived carrying a
download marker — which a browser attaches and `curl` does not. The message
blames a developer who cannot be verified and reads exactly like a corrupt
file, so the `--version` line above is where you would meet it. If you have
already downloaded one that way, `xattr -d com.apple.quarantine stageman`
removes the marker.

There is no checksum to download beside these, and that is deliberate rather
than missing: one published in the same release, fetched over the same
connection from the same account, is not evidence about the binary — anybody
able to replace one could replace both. GitHub records a SHA-256 digest for
every asset and shows it on the release, which is the same assurance without a
step that resembles a stronger one.

Nothing is built for Linux on arm64 yet. Building one needs a cross linker
rather than a decision, and until somebody has run what it produces, an absent
binary is a more honest answer than an untried one.

Releases are cut from the Actions tab — *release* → **Run workflow** — by
naming a version and, if it should not be the head of `main`, a commit to build
from. Pushing a tag by hand does nothing. `stageman --version`
answers without starting anything, so a downloaded file can always say which
release it is and what it was built for. A binary you built yourself says that
instead of a version, which is the honest answer. There is no package to
install: building the browser's half needs the Dioxus tooling, which a registry
does not run, so an installed package would be a dashboard that renders and
never responds.

The browser's half is inside the executable, so what you copy is one file.
Starting it writes that half's `index.html` beside the executable for as long
as it takes to read — the framework will only accept it as a path — and removes
it again, so nothing accumulates. If the executable lives somewhere it may not
write, `/usr/local/bin` being the obvious case, set `DIOXUS_PUBLIC_PATH` to a
directory it may, which is one line in a service unit. It refuses to start
rather than serving a page that would render and never respond.

Where the dashboard listens is `IP` and `PORT`, defaulting to `127.0.0.1:8080`.
Those two names are generic, and they are what they are because the Dioxus
tooling sets them: a binary that read its own pair would need translating every
time it was run for development. Ask for port zero and the operating system
picks; either way the address actually taken is printed at startup, along with
where the browser's half came from. A build without one still serves the
dashboard — the page is rendered on the server and arrives complete, it just
does not update itself afterwards.

Which agents are configured is yours to decide, the foreman picks one per
job from what you have set up, and the dashboard shows which agent ran each
job. Where an agent can be paid for by a subscription rather than by the token,
that is the path stageman prefers.

The dashboard is where you add projects and set the credentials each one needs,
watch jobs and read their logs, and pause or kill one that has gone wrong. A
single instance manages several projects, and a job belongs to exactly one of
them: it cannot see another project's repository, credentials or channels.

All state lives in one human-readable file, rewritten whenever anything changes,
with credentials encrypted under the key described above. Back up that file and
you have backed up the instance; take it to another machine without the key and
it tells you nothing.

## Documentation

`docs/vision.md` is what this is for and what it refuses to be.
`docs/architecture.md` is the shape of the code and the invariants that hold it
together. `docs/conventions.md` is how work is done here, including the words
this codebase uses and the ones it deliberately avoids. `docs/decisions/`
records the choices already taken, each with the alternative it beat and what
would make it wrong.

## A note on the library target

stageman is an application. The library target exists so the binary has
something to test against and so this page has somewhere to live; its public
surface is whatever the binary happens to need, and it will change without
ceremony or a deprecation cycle. Depending on it directly is a mistake.
