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
# whatever spawns containers — so it lives here rather than in either.
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
