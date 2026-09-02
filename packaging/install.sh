#!/bin/sh
#
# **This file is a template and does not run.** The version below is
# substituted by `just installer` when a release is cut, and the result is
# attached to that release as `install.sh`. That published copy is the one
# people fetch, and it differs from this file by exactly one line — so you can
# diff them and see the whole of what a release added.
#
# Why it is published per release rather than served from this repository: a
# script read from a branch describes whatever that branch looks like right now
# and installs whatever `latest` happens to be, which is two moving parts that
# can disagree and no way to reproduce an installation from last month. Pinned
# to its own release, the script is one artifact whose version was settled when
# it was built — the same argument
# `docs/decisions/0039-a-release-is-a-tagged-binary.md` makes about the binary,
# one layer out. See
# `docs/decisions/0041-installing-is-a-script-published-with-the-release.md`.
#
# POSIX `sh`, not bash: `/bin/sh` is dash on Debian and an old bash on macOS,
# and this has to be identical on both. Everything is inside functions with the
# last line calling `main`, so a download cut short cannot execute half an
# install — the shell reads and runs as bytes arrive, and an incomplete file
# simply never reaches the call.
#
# Tests are `if` rather than `[ … ] && …` throughout. An AND-list that fails
# takes its status from the test, and where that is a function's or a script's
# last command it becomes the exit status — which under `set -e` reads as an
# unexplained failure at the end of a run that worked.

set -eu

# ---------------------------------------------------------------- the release
#
# Substituted by `just installer`. Bare, without the leading `v`, because that
# is the spelling the binary itself reports and comparing the two is how an
# update decides whether there is anything to do.
VERSION="@VERSION@"

REPOSITORY="https://github.com/HernanFdz/stageman"

# One place, on the default path of every account on both platforms, which is
# what makes `stageman --version` work for the person who just installed it.
# It is not writable by the user the service runs as, and that is handled
# rather than avoided — see `public_directory` below.
BINARY="/usr/local/bin/stageman"

SERVICE="stageman"
UNIT="/etc/systemd/system/${SERVICE}.service"
LABEL="io.github.hernanfdz.stageman"

# --------------------------------------------------------------------- saying

say() { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }

# Never called from inside a command substitution, deliberately: `exit` there
# would leave a subshell rather than this script, and the install would carry
# on with an empty variable.
fail() {
    printf '\nstageman install: %s\n' "$1" >&2
    shift
    for line in "$@"; do printf '  %s\n' "${line}" >&2; done
    exit 1
}

# ------------------------------------------------------------- this machine

# Sets TARGET to the name this release publishes a binary under.
#
# Every case is named, including the ones with no binary, because "unsupported"
# and "not built yet" are different sentences with different next steps, and a
# single catch-all would tell somebody with an arm64 server to give up.
detect_target() {
    system="$(uname -s)"
    machine="$(uname -m)"
    case "${system} ${machine}" in
        'Linux x86_64')
            TARGET='linux-x64'
            ;;
        'Darwin arm64')
            TARGET='macos-arm64'
            ;;
        'Linux aarch64' | 'Linux arm64')
            fail "there is no Linux arm64 binary yet." \
                "Nothing is wrong with your machine — this release publishes x86-64 only." \
                "Building one needs a cross linker rather than a decision; until somebody" \
                "has run what it produces, an absent binary is more honest than an untried one." \
                "You can build your own: ${REPOSITORY}"
            ;;
        'Darwin x86_64')
            fail "there is no Intel macOS binary." \
                "Releases are built on Apple silicon. Building your own works and is documented:" \
                "${REPOSITORY}"
            ;;
        *)
            fail "no binary is published for ${system} on ${machine}." \
                "stageman is built for Linux on x86-64 and macOS on Apple silicon."
            ;;
    esac
}

# ------------------------------------------------------------------ privilege

# Runs one command as root, and only the commands that need it: writing the
# binary into a system directory and, on Linux, the unit beside it. Everything
# the daemon itself touches — its instance, its key, its log — is written by
# the service as the operator, in their own home.
as_root() { sudo "$@"; }

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "$1 is needed and was not found." "$2"
    fi
}

# -------------------------------------------------------------------- the file

# Where the daemon may write the browser half's index while it starts.
#
# It writes that file beside its own executable unless told otherwise, and
# refuses to start when it cannot — a binary carrying a browser half and unable
# to place its index would serve a page that renders and never comes alive,
# which is nearly indistinguishable from one that works. The executable lives
# in a system directory the service user cannot write, so the unit points this
# somewhere they can, under the same directory the instance goes in. So
# everything stageman writes on this machine is in one place.
PUBLIC="${HOME}/.local/share/stageman/public"
LOG="${HOME}/.local/share/stageman/stageman.log"
PLIST="${HOME}/Library/LaunchAgents/${LABEL}.plist"

# What is installed right now, or nothing.
#
# Reads `version` out of what the binary prints. A build somebody made
# themselves reports `none`, which can never equal a release and is therefore
# treated as something to replace rather than as an error.
installed_version() {
    if [ -x "${BINARY}" ]; then
        "${BINARY}" --version 2>/dev/null | awk '$1 == "version" { print $2; exit }'
    fi
}

# Fetches the release binary and proves it runs before anything is replaced.
#
# **This is what a checksum would have been for, and it is a better test.** A
# hash published in the same release, fetched over the same connection from the
# same account, is not evidence about the binary — anybody able to replace one
# could replace both. Running the thing is evidence: it catches a truncated
# download, a proxy that served an error page with a 200, and a binary for the
# wrong architecture, which is exactly the case a matching checksum would have
# waved through and `exec` would have reported days later as a format error.
fetch() {
    url="${REPOSITORY}/releases/download/v${VERSION}/stageman-${TARGET}"
    say "    ${url}"
    # `--fail` is not optional: without it an HTTP error page is written to the
    # file as cheerfully as a binary would be, and the install carries on.
    if ! curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
        --output "${DOWNLOAD}" "${url}"; then
        fail "the download failed." \
            "If this release is very new, its assets may still be uploading."
    fi
    chmod +x "${DOWNLOAD}"
    if ! "${DOWNLOAD}" --version >/dev/null 2>&1; then
        fail "the downloaded file does not run on this machine." \
            "It is not a working ${TARGET} binary. Nothing has been changed."
    fi
}

# ---------------------------------------------------------------- the service

# What a person typed, as a domain.
#
# Lenient about the one thing everybody pastes — a scheme, and the trailing
# slash that comes with it — because a URL is what a browser shows and a domain
# is what this wants. Everything left is refused here rather than accepted and
# quietly ignored later: the daemon falls back to its default on a value it
# cannot read, which is the right behaviour for a service already running and
# the wrong one for a person standing at a terminal who can be told.
cleaned_domain() {
    given="${1#https://}"
    given="${given#http://}"
    given="${given%/}"
    case "${given}" in
        '' | */* | *:* | *' '*)
            fail "that is not a domain: ${1}" \
                "Give the name on its own, like --domain stageman.example.com."
            ;;
    esac
    printf '%s' "${given}"
}

# The domain this instance was last installed with.
#
# Read back so that re-running the installer — which is how you update — does
# not silently drop it. That failure would look like nothing at all until the
# next job told somebody to visit a name that stopped resolving.
existing_domain() {
    case "${PLATFORM}" in
        linux)
            [ -f "${UNIT}" ] || return 0
            sed -n 's/^Environment=STAGEMAN_DOMAIN=//p' "${UNIT}" | head -1
            ;;
        macos)
            [ -f "${PLIST}" ] || return 0
            /usr/libexec/PlistBuddy -c \
                'Print :EnvironmentVariables:STAGEMAN_DOMAIN' "${PLIST}" 2>/dev/null || true
            ;;
    esac
}


write_systemd_unit() {
    # Absent rather than empty when there is none: the daemon's own default is
    # what an unset variable means, and a variable set to nothing is a value it
    # would have to decide what to do with.
    DOMAIN_LINE=''
    [ -n "${DOMAIN}" ] && DOMAIN_LINE="Environment=STAGEMAN_DOMAIN=${DOMAIN}"
    # `After=docker.service` with no matching `Wants=`, which is the whole
    # point: `After` only orders against a unit that is being started anyway,
    # so it costs nothing on a machine running Podman and having no unit by
    # that name. A `Wants` would try to pull Docker in and fail there.
    #
    # The restart limit is deliberate rather than stingy. A missing container
    # runtime is not something a restart fixes, and a unit retrying for ever
    # sits in `activating` — which reports as neither working nor broken, and
    # is the state hardest to notice.
    as_root tee "${UNIT}" >/dev/null <<UNITFILE
[Unit]
Description=stageman
Documentation=${REPOSITORY}
After=network-online.target docker.service
Wants=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=3

[Service]
Type=simple
User=${OPERATOR}
Environment=HOME=${HOME}
Environment=DIOXUS_PUBLIC_PATH=${PUBLIC}
${DOMAIN_LINE}
ExecStart=${BINARY}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
UNITFILE
    as_root systemctl daemon-reload
}

write_launch_agent() {
    # Absent rather than empty when there is none, for the reason the unit's is.
    DOMAIN_KEY=''
    [ -n "${DOMAIN}" ] && DOMAIN_KEY="    <key>STAGEMAN_DOMAIN</key>
    <string>${DOMAIN}</string>
"
    mkdir -p "${HOME}/Library/LaunchAgents"
    cat > "${PLIST}" <<PLISTFILE
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${BINARY}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>DIOXUS_PUBLIC_PATH</key>
    <string>${PUBLIC}</string>
${DOMAIN_KEY}  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>${LOG}</string>
  <key>StandardErrorPath</key>
  <string>${LOG}</string>
</dict>
</plist>
PLISTFILE
}

stop_service() {
    case "${PLATFORM}" in
        linux)
            # Asked rather than stopped unconditionally, so a first install
            # does not report a failure for a unit that was never there.
            if systemctl is-active --quiet "${SERVICE}" 2>/dev/null; then
                as_root systemctl stop "${SERVICE}"
            fi
            ;;
        macos)
            if [ -f "${PLIST}" ]; then
                launchctl bootout "gui/$(id -u)/${LABEL}" >/dev/null 2>&1 || true
            fi
            ;;
    esac
}

start_service() {
    case "${PLATFORM}" in
        linux)
            as_root systemctl enable --now "${SERVICE}"
            ;;
        macos)
            # Emptied first, so that what `settled` finds in it can only have
            # come from this start. launchd appends across restarts, and a
            # startup line left by the version being replaced would report the
            # new one as running before it had said anything.
            : > "${LOG}"
            launchctl bootstrap "gui/$(id -u)" "${PLIST}"
            ;;
    esac
}

# Whether it is still up a moment later, which is a different question from
# whether it started. Both service managers return as soon as the process has
# been forked, so a daemon that exits immediately looks like a successful start.
#
# Nothing here checks for a container runtime, and that is deliberate: the
# daemon already looks in a list of absolute paths compiled into it and says
# which ones it tried. A second copy of that list in shell would be a second
# thing to keep true, and no check in this repository can see into a script
# that has already been released. So this reports what the daemon said rather
# than predicting it.
settled() {
    attempt=0
    while [ "${attempt}" -lt 10 ]; do
        case "${PLATFORM}" in
            linux)
                if systemctl is-active --quiet "${SERVICE}"; then return 0; fi
                if systemctl is-failed --quiet "${SERVICE}"; then return 1; fi
                ;;
            macos)
                # The line the daemon prints last, rather than whether launchd
                # has the job: `launchctl print` answers about registration,
                # and a job that is registered and exiting on a loop answers it
                # the same way one that is working does.
                if grep -q 'stageman is running' "${LOG}" 2>/dev/null; then return 0; fi
                ;;
        esac
        attempt=$((attempt + 1))
        sleep 1
    done
    return 1
}

why_it_did_not_start() {
    say ""
    say "stageman was installed but did not stay running. What it said:"
    say ""
    case "${PLATFORM}" in
        linux) as_root journalctl --unit "${SERVICE}" --lines 20 --no-pager || true ;;
        macos) if [ -f "${LOG}" ]; then tail -n 20 "${LOG}"; fi ;;
    esac
    say ""
    say "The usual cause is no container runtime. stageman runs every agent in a"
    say "container, including the one it thinks with, so Docker or Podman has to be"
    say "installed and running. Installing one is deliberately not this script's job:"
    say "it needs root for something that is not stageman, and your machine's package"
    say "manager already does it properly."
    say ""
    say "Fix that, then start it again:"
    case "${PLATFORM}" in
        linux) say "  sudo systemctl start ${SERVICE}" ;;
        macos) say "  launchctl kickstart -k gui/$(id -u)/${LABEL}" ;;
    esac
    exit 1
}

# ------------------------------------------------------------------ uninstall

uninstall() {
    step "stopping and removing the service"
    stop_service
    case "${PLATFORM}" in
        linux)
            as_root systemctl disable "${SERVICE}" >/dev/null 2>&1 || true
            as_root rm -f "${UNIT}"
            as_root systemctl daemon-reload
            ;;
        macos)
            rm -f "${PLIST}"
            ;;
    esac
    as_root rm -f "${BINARY}"

    # Left alone, and said out loud so nobody has to wonder. The instance file
    # is the whole of what stageman knows — projects, jobs, credentials — and a
    # script that removed it would make "uninstall" and "lose everything" the
    # same command. Removing them is one line each, and yours to type.
    say ""
    say "stageman is uninstalled."
    say ""
    say "Your instance and its key were left where they are. They are the whole of"
    say "what stageman knew, and deleting them is not an uninstaller's decision:"
    say "  ${HOME}/.local/share/stageman/"
    say "  ${HOME}/.config/stageman/"
    exit 0
}

# ----------------------------------------------------------------------- main

usage() {
    say "stageman ${VERSION} — installer"
    say ""
    say "  sh install.sh                     install or update, and run it as a service"
    say "  sh install.sh --domain <domain>   the domain this instance answers on"
    say "  sh install.sh --uninstall         remove the service and the binary, keep the instance"
    say ""
    say "A domain is what makes a job's work reachable: each job is served at"
    say "<job-id>.<domain>, and the dashboard at the domain itself. Point whatever"
    say "forwards *.<domain> at this machine, and have it authenticate — nothing here"
    say "does. Without one, jobs are shown at <job-id>.localhost, which works on this"
    say "machine and nowhere else."
    say ""
    say "Re-running this is how you update. It is the same command either way, and a"
    say "domain already set is kept unless --domain says otherwise."
    exit 0
}

main() {
    case "${VERSION}" in
        *@*)
            fail "this is the tracked template, not the installer." \
                "The published script is attached to each release:" \
                "${REPOSITORY}/releases/latest/download/install.sh"
            ;;
    esac

    while [ "$#" -gt 0 ]; do
        case "${1}" in
            --uninstall) WANTS_UNINSTALL='yes' ;;
            --domain)
                shift
                [ "$#" -gt 0 ] || fail "--domain needs a value, like --domain stageman.example.com."
                DOMAIN="$(cleaned_domain "${1}")"
                ;;
            --domain=*) DOMAIN="$(cleaned_domain "${1#--domain=}")" ;;
            -h | --help) usage ;;
            *) fail "unknown option ${1}." "Try --help." ;;
        esac
        shift
    done

    # Refused rather than accommodated, and this is the one piece of policy in
    # the script. stageman runs as the person who operates it and keeps its
    # instance in that account's home, so a run as root would install a service
    # for root and put the instance somewhere the person who typed the command
    # cannot reach — which looks exactly like it worked. Sudo is asked for the
    # two files that need it and nothing else.
    if [ "$(id -u)" -eq 0 ]; then
        fail "run this as yourself, not as root." \
            "stageman runs as the person who operates it, and keeps its instance in that" \
            "account's home. It will ask for sudo for the two files that need it." \
            "On a machine with only a root account, make yourself one first."
    fi
    OPERATOR="$(id -un)"
    if [ -z "${HOME:-}" ] || [ ! -d "${HOME}" ]; then
        fail "there is no home directory to keep an instance in."
    fi

    case "$(uname -s)" in
        Linux) PLATFORM='linux' ;;
        Darwin) PLATFORM='macos' ;;
        *) PLATFORM='unknown' ;;
    esac

    require curl "Install it with your package manager and run this again."
    require sudo "The binary goes in a system directory, which needs it."
    case "${PLATFORM}" in
        linux) require systemctl "This installs a systemd service." ;;
        macos) require launchctl "This installs a launchd agent." ;;
        *) fail "neither systemd nor launchd is here, so there is no service to install." ;;
    esac

    if [ "${WANTS_UNINSTALL}" = 'yes' ]; then
        uninstall
    fi

    detect_target

    # Reported, and never acted on. Matching versions do not short-circuit the
    # download, deliberately: re-running is also how somebody repairs an
    # install — a deleted unit, a half-finished update, a binary that was moved
    # — and a run that decided there was nothing to do would be useless in
    # exactly those cases. The cost is one download of a file already present.
    present="$(installed_version)"
    if [ -n "${present}" ]; then
        say "stageman ${present} is installed; this is ${VERSION}."
    fi

    step "downloading stageman ${VERSION} for ${TARGET}"
    DOWNLOAD="$(mktemp)"
    # Runs on every exit, including the failures below that leave a file behind.
    trap 'rm -f "${DOWNLOAD}"' EXIT INT TERM
    fetch

    step "installing to ${BINARY}"
    # Stopped before the file is replaced. Renaming over a running executable
    # does work where copying onto one does not, but the service has to be
    # restarted regardless, and stopping first makes the order obvious rather
    # than clever.
    stop_service
    as_root install -m 755 "${DOWNLOAD}" "${BINARY}"
    mkdir -p "${PUBLIC}"

    # Kept rather than rewritten. Re-running this is how you update, so a
    # domain set once and not repeated has to survive — dropping it would take
    # every job's address away with no error anywhere, and the next thing an
    # agent told somebody to look at would resolve to their own machine.
    [ -n "${DOMAIN}" ] || DOMAIN="$(existing_domain)"

    step "setting it up to run as a service, as ${OPERATOR}"
    case "${PLATFORM}" in
        linux) write_systemd_unit ;;
        macos) write_launch_agent ;;
    esac
    start_service

    if ! settled; then
        why_it_did_not_start
    fi

    case "${PLATFORM}" in
        linux)
            reading="journalctl --unit ${SERVICE} --lines 20"
            configured_in="${UNIT}"
            ;;
        macos)
            reading="tail -n 20 ${LOG}"
            configured_in="${PLIST}"
            ;;
    esac

    # **Says nothing about what is in the instance**, and that is the fix rather
    # than the wording. This script never opens one. What used to be here said
    # the instance had no agents and no projects yet — true of a first install,
    # a plain guess otherwise, and read as a statement of fact by somebody
    # updating a machine with real projects on it. The daemon names the instance
    # it opened, in the lines below, for the same reason nothing here goes
    # looking for a container runtime: it already knows, and a second answer can
    # only be the wrong one.
    step "stageman ${VERSION} is running"
    say ""
    say "Which instance it opened, where the dashboard is listening and which"
    say "container runtime it found are all printed at startup:"
    say ""
    say "  ${reading}"
    say ""
    say "An instance already on this machine is opened as it was — nothing here"
    say "writes to one. A new one starts with no agents and no projects, which you"
    say "add in the dashboard."
    say ""
    say "It listens on 127.0.0.1:8080 by default. To change that, or to reach it"
    say "from another machine, edit ${configured_in}."
    say ""
    say "Re-run this script to update. Run it with --uninstall to remove it."
}

WANTS_UNINSTALL='no'
# Empty means the daemon's own default, which is `localhost`.
DOMAIN=''
main "$@"
