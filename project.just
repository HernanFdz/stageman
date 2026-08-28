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

# The image tag is a fact two places need to agree on — this recipe and
# whatever spawns containers. The second place now exists and is a Rust
# constant, which `just` cannot read and which cannot be generated without
# putting a build step in front of every check. So the two literals stay, and a
# test in the agent crate asserts they are the same string; it parses the line
# below, so keep the `name := "value"` shape.
image_tag := "stageman/claude:dev"

[doc("Build the agent image that jobs and triage run inside")]
image:
    docker build -t {{ image_tag }} images/claude

[doc("Check a built image still answers the protocol handshake")]
image-check:
    # Needs no credential and no network: `initialize` is answered before any
    # authentication happens, so this is the cheapest possible proof that the
    # image, the adapter and the stdio channel are all intact. Worth reaching
    # for before anything that needs a token.
    docker run -i --rm --network none {{ image_tag }} < images/claude/handshake.json

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
# The build stays out of `check` rather than out of the bar: it costs minutes
# and a network, and `check` is the command you run constantly.
[doc("Build the image and run every container test — what `verify` adds")]
containers: image image-handshake

[doc("Drive a container from this repository's own code — needs a built image")]
image-handshake:
    # Every ignored test that costs nothing to run. They are ignored for one
    # reason — they need the image built above, which `check` must not spend
    # minutes building — so this selects by exclusion rather than by naming
    # them, and keeps covering the next one somebody writes. `just check`
    # already reports how many it skipped.
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
# server build, so `just dashboard --port 8123` appended at the end is a `dx`
# error rather than a port.
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
[doc("Serve the dashboard, both halves, with reloading — needs `dx`")]
dashboard *arguments:
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
    echo "instance    $STAGEMAN_STATE"
    dx serve --fullstack --package stageman {{ arguments }} \
        @client --no-default-features --features web \
        @server --features server

# The same two halves, built rather than served. What this produces is what
# would ship: a server binary with the browser's bundle in `public/` beside it,
# which is the layout the server looks for and reports at startup.
[doc("Build both halves into a bundle — needs `dx`")]
dashboard-build:
    dx build --fullstack --package stageman \
        @client --no-default-features --features web \
        @server --features server
