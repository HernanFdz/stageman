# 0041 — Installing is a script published with the release

## Status
Accepted. Builds on
`docs/decisions/0040-a-release-is-built-on-the-machine-it-is-for.md`, which is
what makes a single command true on more than one machine.

## Context

A release is two binaries at a stable address, and using one means downloading
it, making it executable, deciding where to put it, writing a service unit, and
knowing that the executable's directory has to be writable or the daemon
refuses to start. Every one of those is documented and none of them is
interesting. What people expect from a project like this is one command.

Three things about *this* daemon shape what that command has to do, and a
generic installer gets all three wrong.

**The obvious install directory is the one that breaks it.** Since
`docs/decisions/0038-the-browsers-half-lives-in-the-binary.md` the binary
writes the browser half's index beside its own executable at startup and
refuses to start when it cannot. Installed to a system directory and run as an
ordinary user, that is a hard failure whose message reads like a permissions
bug.

**A container runtime is required and must not be installed here.**
`docs/decisions/0023-the-container-runtime-is-discovered-once.md` stops the
process when there is none. Under a service manager that is a crash loop, and
the installer will have already said it was done.

**Everything else is already handled.**
`docs/decisions/0037-the-instance-key-is-generated-on-first-run.md` generates
the key and `docs/decisions/0021-an-instance-starts-empty.md` means there is
nothing to configure — so the command really can end with something running,
rather than with a list of next steps.

## Decision

**One script, published as an asset of each release, pinned to that release.**
`packaging/install.sh` is the tracked template; `just installer` substitutes
the version and the release attaches the result.

- **Published per release rather than served from a branch.** A script read
  from `HEAD` describes whatever that branch is now and installs whatever
  `latest` is then — two moving parts that can disagree, and no way to
  reproduce an installation from last month. This is 0039's argument about the
  binary, one layer out. The property is checked rather than described: the
  recipe fails unless exactly one line differs from the template, so anybody
  can diff the published script against the tag it came from.
- **A system unit running as the person who installed it**, and a launchd
  agent on macOS. The instance and key stay in that account's home, where the
  operator can back them up without privilege, and the container runtime is
  reached through their own group membership.
- **The binary goes in a system directory and the unit sets the public path**,
  which is what makes that directory safe. One variable, in a file this script
  writes.
- **Root is refused.** Running the whole thing as root would install a service
  for root and put the instance where the person who typed the command cannot
  reach it — which looks exactly like success. Privilege is asked for the two
  files that need it.
- **Nothing checks for a container runtime.** The service is started and, if it
  did not stay up, the daemon's own message is printed. The daemon already
  names every path it tried; a second copy of that list in shell would be a
  second thing to keep true, and nothing in this repository can check a script
  that has already been released.
- **Re-running it is the update.** The published script is fetched from
  `latest`, so it is a newer script pinned to a newer release. One mechanism,
  and the installed version is read from the binary so it can say what changed.
- **Uninstalling removes the service and the binary and keeps the instance.**

**What replaced the checksum is running the binary.** 0040 removed the
published hash because one from the same release is not evidence about the
binary. The script downloads to a temporary file and asks it for its version
before anything is replaced. That catches a truncated download, a proxy
answering with an error page, and a binary for the wrong machine — the last of
which a matching checksum would have waved straight through.

Rejected: **a `stageman self-update` subcommand.** It puts a network fetch and
a binary-replacement path inside the product, needs an API call to discover
what is current, and becomes a second mechanism that can disagree with the
first about where things live.

Rejected: **a user service with lingering, on Linux.** It was the first
proposal, on the grounds that it needs no privilege. It does not survive
contact with the common case: a user manager stops when its last session ends,
so a service started over SSH dies at disconnect unless lingering is enabled —
and enabling it over SSH is itself privileged, because a session with no seat
is not active as far as the policy is concerned. So the privilege is not
avoided, and what is bought instead is a mechanism that can be silently absent,
whose failure is the daemon disappearing at logout.

Rejected: **a dedicated service account.** It is what one reaches for on a
server, and it is disqualified by code rather than by preference: the key and
the instance path are both derived from the account's home, so an account
without a usable one fails at startup. Rescuing it means the installer setting
the state path *and* generating a key, which is exactly what 0037 took out of
everybody's hands.

Rejected: **installing a container runtime.** It needs root for something that
is not stageman, differs per distribution, and every package manager already
does it properly. `docs/vision.md` §3 has this running on infrastructure the
operator controls; configuring stageman is in scope and configuring their
machine is not.

Rejected: **binding the dashboard beyond loopback when the install looks
remote.** Tempting, since a server install cannot reach `127.0.0.1` from
anywhere useful — and it would quietly answer the open question about
authentication with "none, exposed". The script does not guess where it is
running.

## Consequences

**The installer is shell, and shell is not covered by the gate.** What holds it
honest is narrow and worth stating plainly rather than overselling: the recipe
parses the published artifact with `sh -n` and asserts the substitution, the
template refuses to run if it is ever published unsubstituted, and it is POSIX
`sh` wrapped in a function called on the last line, so a truncated download
cannot execute half of it. None of that is a test that it installs anything.
Running the published script end to end on a fresh machine is not done, and
until it is, this is the least-verified thing in the repository.

**The `latest` address only exists once something is published.** There is no
install path from `main`, which is 0039 refusing a rolling prerelease rather
than a gap here.

**Two files on the machine are ours and are not tracked anywhere**: the unit
and, on macOS, the agent. An operator who edits one — to change the address the
dashboard listens on, most obviously — has it overwritten by the next update.

**Reversing** is deleting a directory, a recipe and three workflow steps. A
release already published keeps working, because its script is its own.

**Revisit if** somebody wants the dashboard reachable from another machine,
which is the open question about authentication and not an installer flag; if
Linux arm64 gets a binary, which is one line in the target detection; or if the
unit needs to survive being edited, at which point the update path has to learn
the difference between a file it wrote and one somebody changed.
