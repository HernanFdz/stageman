# Project-specific recipes.
#
# The split is for readers, not for tooling. The main `justfile` is the same in
# every project built from this gate, so it is worth learning once and anything
# unfamiliar in it is a real difference; this file is where the differences are
# supposed to be. Adding a project recipe to the main justfile is not an error,
# it just makes the shared half stop being shared.
#
# The main justfile ends with `import? 'project.just'`, so whatever you define
# here appears in `just --list` alongside the gate's own recipes. The `?` makes
# the import optional, so deleting this file is also fine.
#
# Example:
#
#   [doc("Run the app against the local fixture data")]
#   demo:
#       cargo run -- --fixtures tests/data

# The recipe is compiled into the binary and built on demand, so there is no
# tag here and no recipe that builds one — see
# `docs/decisions/0035-an-image-is-built-never-named.md`. The two stages are
# the only strings this file and the source both name, and the agent crate has
# a test asserting that every stage it asks for is declared in the recipe.

[doc("Check the image still answers the protocol handshake, without compiling anything")]
image-check:
    #!/usr/bin/env bash
    set -euo pipefail
    # Needs no credential: `initialize` is answered before any authentication
    # happens, so this is the cheapest possible proof that the image, the
    # adapter and the stdio channel are all intact. Worth reaching for before
    # anything that needs a token.
    #
    # Shell rather than Rust on purpose, and that is the whole point of keeping
    # it: this holds when the repository does not compile, which is exactly
    # when you most want to know whether the container half still works. It
    # therefore builds the recipe itself rather than asking the binary to.
    #
    # A foreman's stage, because it is the smaller of the two and the adapter
    # it proves is in both.
    image="$(docker build --quiet --target thinking - < agent/images/claude/Dockerfile)"
    docker run -i --rm --network none "$image" < agent/images/claude/handshake.json

# Both of these greet the image, and the difference is what is on trial. The
# recipe above tests the image with a redirect and a file, so it holds when this
# repository does not compile. This one tests *this repository's* code, and is
# the ignored test from the agent crate — ignored so that `just check` needs
# neither a runtime nor a built image, and named here so that skipping it stays
# a choice rather than an accident.
# What `just verify` reaches for, and the reason these are no longer a step
# somebody has to remember. Until
# `docs/decisions/0023-the-container-runtime-is-discovered-once.md` the gate had
# to run on a machine with only a toolchain, so anything needing a runtime was
# ignored and run by hand; a runtime is now a prerequisite, and the only thing
# still separating these from the rest is the image build.
#
# That build has no recipe of its own any more: the tests build what they need,
# from the recipe compiled into the crate they are testing. It stays out of
# `check` rather than out of the bar for the reason it always did — the first
# one costs minutes and a network, and `check` is the command you run
# constantly.
[doc("Run every container test — what `verify` adds")]
containers: image-handshake

[doc("Drive a container from this repository's own code — builds what it needs")]
image-handshake:
    # Every ignored test that costs nothing but time. They are ignored for one
    # reason — the first of them builds an image, which `check` must not spend
    # minutes on — so this selects by exclusion rather than by naming them, and
    # keeps covering the next one somebody writes. `just check` already reports
    # how many it skipped.
    cargo nextest run --workspace --locked --run-ignored ignored-only \
        -E 'not test(/costs_a_credential/)'

# Split from the recipe above because these spend real money and reach the
# network, which makes them a different thing to reach for. They are grouped by
# module rather than by name so the filter keeps working as more are added, and
# they fail loudly rather than skipping when no credential is present: a test
# that passes because it could not run is worse than one that is honestly
# absent.
[doc("Drive an agent that actually answers — needs a built image and a credential")]
image-session:
    cargo nextest run --workspace --locked --run-ignored ignored-only \
        -E 'test(/costs_a_credential/)'

# Not a test, and deliberately so: `just image-handshake` runs every ignored
# test that costs no credential, so an ignored test which opened a pull request
# would fire on a command run several times a session. An example is compiled
# and linted by the gate and executed only when somebody types this.
[doc("Run one real job that proposes a real change — opens a pull request")]
propose repository:
    cargo run --package stageman --example propose -- {{ repository }}

# `dx` builds the same binary twice, for two machines, and needs telling what
# each half is: the browser's has to have `server` off, or it drags the domain
# crate — and a cipher, and an async runtime — into a wasm build that cannot
# have them. `docs/decisions/0022-the-browser-never-sees-the-domain.md` says
# why that must not merely fail, but be impossible to ask for.
#
# `just check` does not need this recipe or the `dx` it runs: the gate compiles
# the browser's half with plain cargo against the wasm target, which is a
# toolchain fact. What needs `dx` is a *bundle*, which is what makes the page
# come alive rather than merely arrive.
# Arguments go before `@client`, and that position is the whole of why this
# takes them at all: `dx` reads everything after `@server` as an argument to the
# server build, so `just dev --port 8123` appended at the end is a `dx` error
# rather than a port.
#
# The two variables the daemon requires are supplied here, and that is not a
# softening of the rule that it defaults neither. That rule is about a daemon a
# service manager starts in a working directory nobody chose; this is a command
# a person runs by hand from the repository root, which is the same exemption
# `propose` already takes for locating a container runtime.
#
# The key is generated on first use rather than asked for. It protects a
# development instance, so losing it costs a file with nothing in it — and an
# instance you cannot open is repaired by deleting it.
#
# The instance is put in the scratch directory rather than left to the default,
# which is deliberate and is the case that variable exists for: serving this
# out of a checkout must not touch the real instance on the machine, and
# noticing that it had would take a while.
[doc("Serve both halves with reloading — the development loop. Needs `dx`")]
dev *arguments:
    #!/usr/bin/env bash
    set -euo pipefail
    scratch="{{ justfile_directory() }}/.local"
    key="$scratch/instance-key"
    if [ ! -f "$key" ]; then
      mkdir -p "$scratch"
      # `/dev/urandom` rather than `openssl`, which is a coin flip on any
      # given machine, and 32 bytes because that is the key size — a shorter
      # one is refused at startup rather than padded.
      head -c 32 /dev/urandom | base64 > "$key"
      echo "generated a development instance key in .local/instance-key"
    fi
    export STAGEMAN_KEY="$(cat "$key")"
    export STAGEMAN_STATE="${STAGEMAN_STATE:-$scratch/instance.json}"
    # Emptied, because the bundle directory is a compile-time input and a
    # binary carrying one prefers it over the live tree `dx` is serving. A
    # stale bundle embedded here would look exactly like hot reloading having
    # stopped working, which is a long way to walk for a directory nobody
    # meant to leave behind. `just build` empties it too, so this is a second
    # line of defence rather than the only one.
    rm -rf "{{ justfile_directory() }}/app/bundle"
    mkdir -p "{{ justfile_directory() }}/app/bundle"
    # Deliberately not echoed here. The binary prints where its instance is as
    # part of the summary it prints anyway, so saying it first only produces
    # the same line twice, several seconds apart, with a build between them.
    dx serve --fullstack --package stageman {{ arguments }} \
        @client --no-default-features --features web \
        @server --features server

# The same two halves, built rather than served: a server binary with the
# browser's bundle in `public/` beside it, which is the layout the server looks
# for and reports at startup. Copy that directory anywhere and run it.
#
# **Always `--release`, and there is no way to ask for anything else.** A debug
# bundle is not merely a larger one: `dioxus-web` gates its hot-reload client on
# `debug_assertions`, so it carries a websocket client that retries against a
# dev server that is not there, for ever, on every open tab. That client cannot
# be switched off by feature selection — `dioxus`'s `fullstack` feature enables
# it explicitly, and cargo features are additive. The profile is the only lever,
# so this recipe holds it rather than offering it. The wasm being far smaller
# is the lesser half of the reason.
#
# **`--debug-symbols false` on the browser's half is not the tautology it
# looks like.** The flag defaults to *true*, and it wins over
# `[web.wasm_opt] debug` in `app/Dioxus.toml`, because the tooling overwrites
# that field with this flag rather than reading it — so this is the only place
# the question can be answered at all, and the obvious place is the wrong one.
#
# What it buys is not size. `strip = "debuginfo"` in the release profile leaves
# no debug information for it to keep, so the only thing it was preserving was
# an *empty* name section: the bindgen step is told to remove the names and
# leaves the section behind it, and `--debuginfo` then carries the husk
# through. Firefox reads a name section that is present and empty by running
# off the end of it into the next custom section, misreading that section's
# header as a subsection of this one, and reports the module as validated with
# a warning on every single load. Off, the optimizer strips the section
# instead of preserving it — and there was never anything in it to lose.
#
# Targets are named `<os>-<arch>` rather than by triple, because this is the
# layer where commands are spelled and nobody should have to type
# `x86_64-unknown-linux-gnu`. The list is deliberately short: a name appears
# here once somebody has a linker for it and has run what it produced, so an
# unlisted name is an honest "nobody has tried this" rather than a build that
# fails at the link step.
#
# `host` is a word rather than this machine's triple, and that is the same
# distinction `check_matrix` in the main justfile draws for the same reason:
# passing `--target <your own triple>` makes cargo build into `target/<triple>/`
# and share no cache with `cargo run`, `cargo test` or rust-analyzer. Omitting
# it is not a shorthand for naming it, so building for this machine has to mean
# passing no target at all.
#
# **Deliberately no bare-OS aliases.** `linux` meaning x64 would be a guess
# about an architecture, and when such a guess is wrong the failure is
# `Exec format error` on the machine you copied to — found after deploying
# rather than at build time. Naming the architecture costs four characters and
# cannot be wrong.
#
# The cross-compilation linker is NOT configured in this repository, and that
# is also deliberate. `[target.<triple>].linker` is read whenever cargo builds
# for that triple, host or not — so a tracked entry for `x86_64-unknown-linux-gnu`
# breaks an ordinary native build on any x86_64 Linux machine, continuous
# integration included. It belongs in `~/.cargo/config.toml`, per machine.
#
# What it produces is copied out of `dx`'s nested output tree into a directory
# named for the target, which answers in one path the two questions a built
# bundle raises: where is it, and which machine is it for. `dx` writes every
# target to the same place, so without this the only thing telling two bundles
# apart would be which was written last — and that is a difference nothing
# reports, whose symptom is `Exec format error` at the far end.
[doc("Build the deployable bundle into dist/<target>: host, macos-arm64, linux-x64")]
build target="host":
    #!/usr/bin/env bash
    set -euo pipefail
    # Expanded unquoted below, which is deliberate: a triple contains no
    # whitespace and no glob, so word-splitting is exactly the behaviour
    # wanted — two arguments when there is a target, and none when there is
    # not.
    case "{{ target }}" in
      host)        targeting="" ;;
      macos-arm64) targeting="--target aarch64-apple-darwin" ;;
      linux-x64)   targeting="--target x86_64-unknown-linux-gnu" ;;
      *)
        echo "unknown build target '{{ target }}'" >&2
        echo "known: host, macos-arm64, linux-x64" >&2
        exit 1
        ;;
    esac
    built="{{ justfile_directory() }}/target/dx/stageman/release/web"
    shipping="{{ justfile_directory() }}/dist/stageman-{{ target }}"
    embed="{{ justfile_directory() }}/app/bundle"

    # Twice, and the reason is ordering rather than caution. The server embeds
    # the browser's half at compile time, and the tooling builds both halves in
    # parallel — its one ordering flag runs the server *first*, which is the
    # wrong way round. So the first pass exists to produce a client, and the
    # second is the one whose server carries it. See
    # `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`.
    #
    # Emptied rather than copied over, here and below. An asset's filename
    # carries a hash of its contents, so a stale file survives a rebuild under
    # a name the new `index.html` never mentions — a bundle that grows one dead
    # file per build and never says so.
    # Emptied before anything else, because `dx` does not do it: an asset's
    # filename carries a hash of its contents, so every build leaves its
    # predecessors behind under names the new `index.html` never mentions.
    # Left alone, this directory accumulates a dead wasm module per build —
    # measured at four of them, some nine megabytes, all of which would have
    # been compiled into the binary and served by nothing. The same reasoning
    # applies here as to the destination below; the difference is that this one
    # is somebody else's directory, so nothing but doing it ourselves fixes it.
    rm -rf "$built"

    echo "==> pass 1 of 2: building the browser's half"
    dx build --release --fullstack --package stageman \
        @client --no-default-features --features web --debug-symbols false \
        @server --features server $targeting
    rm -rf "$embed"
    mkdir -p "$embed"
    cp -R "$built/public/." "$embed/"

    echo "==> pass 2 of 2: building the server that carries it"
    dx build --release --fullstack --package stageman \
        @client --no-default-features --features web --debug-symbols false \
        @server --features server $targeting

    # One file, named for what it is. `dx` calls it `server`, which says
    # nothing once it is somewhere else — and a name that depends on which
    # machine built it is a name nobody can identify later, which is why
    # `host` is never what a published artefact is called.
    mkdir -p "{{ justfile_directory() }}/dist"
    rm -f "$shipping"
    cp "$built/server" "$shipping"

    # Left as a fresh clone has it. The directory is a compile-time input, so
    # leaving it full would make an ordinary `cargo build` — and therefore
    # `just check` — produce a different binary here than it does in continuous
    # integration, which is the kind of difference that is only ever noticed
    # from the far side.
    rm -rf "$embed"
    mkdir -p "$embed"

    # Said out loud, because a build's two questions are what it made and what
    # it will run on, and the second is the one whose wrong answer is only
    # discovered on the machine you copied it to.
    echo
    echo "built: $shipping"
    # What it will run on comes from the file, because that is the question a
    # cross build gets wrong and the one whose wrong answer surfaces only on
    # the machine you copied it to. What it says it is comes from asking it,
    # which only works when it can run here — so it is attempted rather than
    # relied on.
    if command -v file >/dev/null 2>&1; then
      file -b "$shipping"
    fi
    "$shipping" --version 2>/dev/null || true


# What continuous integration runs when a tag is pushed, and what you run to
# reproduce one by hand. The derivation lives here rather than in the workflow
# for the reason the main justfile states: every rule lives in the justfile, so
# the forge runs the identical thing and the two cannot drift apart.
#
# The version comes from the tag and nowhere else — no manifest holds one, per
# `docs/decisions/0039-a-release-is-a-tagged-binary.md`, so there is nothing for
# it to disagree with. The commit and its date come from git, which is the only
# thing that actually knows them; the date is the commit's rather than this
# moment's, so that rebuilding one tag cannot produce a binary claiming a
# different one.
[doc("Build a release artefact with its provenance implanted: `just release v0.2.0`")]
release version target="linux-x64":
    #!/usr/bin/env bash
    set -euo pipefail
    tag="{{ version }}"
    # A tag is written `v0.2.0` and a version is `0.2.0`. Stripped in one place
    # so that nothing downstream has to know the convention.
    export STAGEMAN_BUILD_VERSION="${tag#v}"
    export STAGEMAN_BUILD_COMMIT="$(git rev-parse --short=12 HEAD)"
    # Committer date, as a plain date. `%cs` is already ISO 8601 and already
    # has no time in it, which is the precision this claim deserves.
    export STAGEMAN_BUILD_DATE="$(git show -s --format=%cs HEAD)"
    echo "==> releasing $STAGEMAN_BUILD_VERSION from $STAGEMAN_BUILD_COMMIT ($STAGEMAN_BUILD_DATE)"
    just build {{ target }}

# What turns the tracked template into the script a release publishes.
#
# Here rather than in the release workflow for the reason the main justfile
# states and `.github/workflows/release.yml` repeats: every rule lives in the
# justfile, so continuous integration runs the identical thing and the two
# cannot drift. It also means the published installer can be reproduced by hand
# and diffed against what a release actually attached.
#
# One substitution, and the recipe asserts that. The whole argument for
# publishing this per release rather than serving it from a branch is that the
# script is pinned to one set of assets — so "it differs from the template by
# exactly one line" is the property worth checking rather than describing, and
# a second placeholder appearing later fails here instead of shipping.
#
# It deliberately does not read anything out of `dist`. There was a version
# where it hashed the binaries and templated the checksums in, and
# `docs/decisions/0040-a-release-is-built-on-the-machine-it-is-for.md` records
# why that went: a checksum from the same release is not evidence about the
# binary. What replaced it is in the script — it runs what it downloaded before
# installing it — which needs nothing from here.
[doc("Template the published installer for a version: `just installer v0.2.0`")]
installer version:
    #!/usr/bin/env bash
    set -euo pipefail
    tag="{{ version }}"
    # Stripped exactly as `release` strips it, and for a sharper reason here:
    # the script compares its own version against what an installed binary
    # reports, and a binary reports `0.2.0` rather than `v0.2.0`.
    bare="${tag#v}"
    template="{{ justfile_directory() }}/packaging/install.sh"
    shipping="{{ justfile_directory() }}/dist/install.sh"

    if ! grep -q '@VERSION@' "$template"; then
      echo "no @VERSION@ placeholder in $template" >&2
      echo "Substituting nothing would publish a script that refuses to run." >&2
      exit 1
    fi

    mkdir -p "{{ justfile_directory() }}/dist"
    sed "s/@VERSION@/${bare}/" "$template" > "$shipping"

    # Anything still spelled like a placeholder is one nobody substituted. The
    # script's own guard catches `@VERSION@` at run time; this catches the
    # second placeholder somebody adds without adding a `sed` for it, which the
    # guard would not.
    if grep -n '@[A-Z_][A-Z_]*@' "$shipping"; then
      echo "the lines above still hold a placeholder" >&2
      exit 1
    fi

    # Exactly one line differs, which is the property the decision record
    # claims and this is what makes it true rather than asserted. `diff` exits
    # non-zero on a difference, which is the expected case, so its status is
    # not what is being read.
    changed="$(diff "$template" "$shipping" | grep -c '^[<>]' || true)"
    if [ "$changed" != "2" ]; then
      echo "$changed changed line(s) between the template and the published script; expected 2" >&2
      diff "$template" "$shipping" || true
      exit 1
    fi

    # Parsed as the shell that will run it, on the artifact that ships rather
    # than on the template. Costs nothing, needs nothing installed, and a
    # syntax error here would otherwise be discovered by whoever piped it into
    # a shell.
    sh -n "$shipping"
    chmod +x "$shipping"
    echo "built: $shipping (stageman ${bare})"
