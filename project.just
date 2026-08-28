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
[doc("Drive a container from this repository's own code — needs a built image")]
image-handshake:
    # Every ignored test that costs nothing to run. They are ignored for one
    # reason — they need a container runtime — so this selects by exclusion
    # rather than by naming them, and keeps covering the next one somebody
    # writes. `just check` already reports how many it skipped.
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
