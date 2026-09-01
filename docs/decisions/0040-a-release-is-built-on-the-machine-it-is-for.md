# 0040 — A release is built on the machine it is for

## Status
Accepted. Narrows the "only Linux is built" consequence of
`docs/decisions/0039-a-release-is-a-tagged-binary.md`, and removes the checksum
that record's workflow published beside each binary.

## Context

0039 built one binary and said why: linking for a Darwin target needs a
software development kit licensed for Apple hardware only, so a Linux runner
cannot produce a macOS binary however well it cross-compiles the other way.
That reasoning is intact and is not being reversed. It is a fact about a
*runner*, not about this project, and 0039 named the thing that would make it
worth paying for a second one — "revisit if macOS becomes a target somebody
downloads rather than builds, which needs a second runner and cannot be done
any other way".

An installation script is what makes that true. Its whole promise is one
command on whatever machine you are sitting at, and a script that detects macOS
correctly can only detect it correctly enough to apologise. The apology is also
aimed at the wrong person: this project is developed on macOS, so the first
machine the command is typed on has no binary to fetch.

Three things were already in place, which is what makes this small rather than
a project.

- **The build recipe already names the target.** `project.just` has
  `macos-arm64` and has had it since before there was anything to publish.
- **The daemon can already find a runtime there.** The path list compiled into
  `agent/src/lib.rs` carries Homebrew and Docker Desktop locations, so a macOS
  binary reaches a working instance rather than a startup failure.
- **The existing job is already the shape this needs.** It passes `--target`
  for what is, on that runner, its own host triple — so "build for the machine
  you are on, through the same recipe" is the case that has been proven every
  time a release was cut.

## Decision

**A release is built once per target, on a runner of that target's own
platform, and drafted once from everything those builds produced.**
`macos-arm64` joins `linux-x64`.

Nothing is cross-compiled. Each target is the host triple of the runner
building it, so no cross linker is configured, no target is installed, and
`docs/conventions.md` §5's reason for keeping linkers out of this repository
stays untested by this change rather than worked around.

The workflow becomes three jobs where it was one, and the split is not
cosmetic:

- **settle** decides the version and the commit and refuses everything
  refusable. Its own job, so a mistyped version fails before two runners
  install a toolchain rather than after one has.
- **build** is a matrix over `(runner, target)`. Each uploads its one file as
  an artifact.
- **draft** collects them and creates the release. One job writes to the
  release, which is what makes a partial one impossible: a release naming two
  binaries and carrying one would be worse than a failed build, and only a
  single writer rules it out.

**`macos-latest` rather than a pinned image, and the produced file is checked
against the name it will ship under.** A pinned label is the more determinate
choice everywhere except here: runner images are retired on GitHub's schedule,
and the failure would land on the one workflow whose failure is most expensive
to discover. What the pin was protecting against — the label moving back to
Intel and silently producing a file called `arm64` — is instead asserted
directly, by reading the architecture out of the built binary. That is a
stronger check than the pin was, because it holds against every way the name
could come to be wrong rather than the one that was anticipated.

**No checksum is published.** The workflow used to attach a `.sha256` beside
the binary, and it bought nothing: it came from the same release, over the same
connection, from the same account, so anybody able to replace the binary could
replace its checksum in the same breath. A verification step whose reference
value shares a trust domain with the thing it verifies is ceremony, and
ceremony that resembles a security control is worse than none — it invites the
belief that downloads are verified. GitHub already records a SHA-256 digest for
every asset and serves it through its own API, so ours was also a third copy of
a fact nobody asked for.

Rejected: **each build attaching its own file to the same draft.** One less
job, and two jobs writing to one release is a race — the action's own guard
against a missing file sees only the half it was given, so the failure mode is
a published release quietly missing a platform.

Rejected: **cross-compiling macOS from Linux.** Not a cost, an impossibility:
0039 measured this and the constraint is a licence rather than a toolchain.

Rejected: **`linux-arm64` in the same change.** It is genuinely the "four
lines" 0039 describes — a target, a cross linker on the runner, a variable and
a name — but it is a *cross* build, so it reintroduces the linker question this
decision otherwise leaves alone. An unbuilt target costs nothing as long as
whatever installs names it rather than guessing.

Rejected: **signing the macOS binary.** It would need a paid developer account,
a certificate held somewhere, and notarisation on every release. What it buys
is a smoother path for somebody who downloaded through a browser, and the
supported path does not go through one — see below.

## Consequences

**The macOS binary is unsigned and unnotarised, and how you fetch it decides
whether that matters.** A file downloaded by a browser carries
`com.apple.quarantine`, and Gatekeeper refuses to run an unsigned executable
that has it — the message names a developer who cannot be verified, which reads
like a broken download. `curl` sets no such attribute, so a binary fetched the
way this project documents runs. That asymmetry has to be written down wherever
somebody is told to download one, because the failing path is the one that
looks more ordinary.

**A release costs two runners instead of one.** Free here, because the
repository is public; on a private one, macOS minutes bill at a multiple.

**Reversing is deleting a matrix entry**, and no binary already downloaded
stops working.

**Revisit if** a Linux arm64 machine is somebody's actual target, which needs
the cross linker this deliberately avoided; if Intel macOS is asked for, which
is a third runner and not a flag; or if a binary ever needs to be fetched by
something that sets the quarantine attribute, at which point signing stops
being ceremony and becomes the cheapest answer.
