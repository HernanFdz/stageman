# Scaffolded by quality 0.1.0-dev. This file is yours — edit it freely.
#
# Two tiers, split by whether the cost is BOUNDED by the size of the codebase:
#
#   just check   run constantly. Every step costs time proportional to the code
#                that already exists, so it stays predictable as you grow.
#   just verify  run before pushing. Adds `mutants`, whose cost scales with your
#                DIFF, not with the codebase.
#
# For a tighter inner loop than `check`, run `just lint` on its own — clippy runs
# the full compiler frontend, so it catches every compile error; it just does not
# run the tests. On a large project that is the difference between seconds and
# minutes, because `test` dominates `check` once a suite exists.
#
# `deps` is in `check` despite needing the network: a yanked or vulnerable
# dependency is worth hearing about the moment you add it, not at push time.
#
# `msrv` is in `check` because it is normally free. It is skipped outright when
# rust-version equals the pinned toolchain (`lint` already compiled against that
# exact compiler) or is absent (an application pins its toolchain and has no
# consumers). It only builds against a second toolchain when you are making a
# real compatibility promise — which is the one case where it must.

default: check

# ------------------------------------------------------------- what you build
#
# Every configuration this project must build under, ONE PER LINE. Each line is
# the target and feature selection for one pass; the rest of every invocation is
# fixed by the recipes below, so this file is the only place to edit.
#
#   host                                              this machine, default features
#   host --features server                            this machine, plus a feature
#   --target wasm32-unknown-unknown --features web    a specific triple
#
# `host` is a DECLARATION, not a baseline. A project that never runs on the
# machine you develop on deletes that line and the gate stops checking for it —
# a Linux-only service written on a Mac is ordinary, not an edge case. Nothing
# here is implied; the list is exactly what gets checked.
#
# It is a word rather than your own triple because passing `--target <your own
# triple>` is NOT the same as omitting it: cargo then builds into
# `target/<triple>/` and shares no cache with `cargo run`, `cargo test` or
# rust-analyzer. Hardcoding your triple would also break for anyone on another
# machine.
#
# A `--target` line needs that triple listed in `rust-toolchain.toml` under
# `targets`, because targets install PER TOOLCHAIN and this gate pins one.
# Without it the build fails with "can't find crate for `std`", which does not
# sound like a missing target at all.
#
# `--all-targets` below does NOT mean triples — it means lib, bin, test, bench
# and example. Code behind `cfg(target_arch = ...)` or `cfg(feature = ...)` is
# never compiled by a pass that does not select it, so it is never linted.
# Verified: a deliberate violation behind either one passes a single-pass lint
# completely clean.
check_matrix := '''
host
'''

# --------------------------------------------------------------------- check

# `tools` comes first because the gate depends on binaries a default Rust
# install does not ship. Without it a missing tool surfaces as "no such
# command", which reads like a broken justfile rather than a one-line fix — and
# the check would have failed either way, so self-healing costs nothing.
[doc("Full fast gate: fmt, lint, test, doc, drift, deps, msrv. Run constantly.")]
check: tools fmt lint test doc drift deps msrv

[doc("Formatting is clean")]
fmt:
    cargo fmt --all --check

# `cargo clippy` compiles, so a separate `cargo check` step is redundant.
#
# `--all-targets` is MANDATORY. Without it clippy never compiles test, bench or
# example targets, so a library can be green while its own tests do not compile
# at all — and #[cfg(test)] code is never linted for anything.
#
# `xtask` is linted once, on this machine, and excluded from every matrix pass.
# It is host tooling: it cannot compile for wasm, it is never shipped, and no
# consumer builds it. Same exemption `msrv` makes, for the same reason — gate
# infrastructure is not project content.
[doc("Clippy with the full lint set, warnings denied, once per `check_matrix` line")]
lint:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo clippy --package xtask --all-targets --locked -- -D warnings
    # A here-string rather than a pipe: `while read` on the right of a pipe runs
    # in a subshell, and a clippy failure inside it would not reach `set -e`.
    # `read` also trims surrounding whitespace, so lines may be indented.
    while read -r pass; do
      if [ -z "$pass" ]; then continue; fi
      # `if`, not `[ … ] && …`: under `set -e` a false test as the first half of
      # an AND-list exits the script.
      if [ "$pass" = "host" ]; then pass=""; fi
      echo "lint: ${pass:-host, default features}"
      cargo clippy --workspace --exclude xtask --all-targets --locked $pass -- -D warnings
    done <<< "{{check_matrix}}"

# Two commands, because nextest cannot run doctests (a stable-Rust limitation).
# Dropping `test-doc` silently discards them, and they are the one mechanism
# that makes prose and code unable to disagree. Never collapse this into one
# command.
#
# nextest runs a strict subset of what `cargo test` runs, missing exactly the
# doctests — confirm on your own project by diffing `cargo nextest list` against
# `cargo test -- --list`. (Those lists also differ on trybuild compile-fail
# cases: `cargo test` prints one line per UI case, nextest prints only the
# wrapping #[test]. Those still execute.)
#
# Note `cargo test --doc` deliberately has no `--all-targets`: that flag SKIPS
# doctests entirely (verified — a deliberately broken doctest passes under it).
# Runs ONE configuration: this machine, default features. Every other line in
# `check_matrix` is reported afterwards as unexercised, because a silent partial
# run is the worst outcome available here — `cargo test` prints a confident
# count that simply omits whatever was not compiled. A `#[ignore]`d test is at
# least counted as ignored; a test behind a `cfg` that was not selected appears
# NOWHERE, so the number looks complete and is not.
#
# Running the rest needs more than a flag. A foreign triple needs a runner
# (`wasm-bindgen-test-runner`, qemu, a container) wired up as
# `[target.<triple>.runner]`, which is per-project setup the gate cannot supply.
# Feature-only lines could run here today and currently do not; both are open
# questions rather than settled design.
[doc("Unit, integration and doc tests — this machine only; see `check_matrix`")]
test: test-unit test-doc
    #!/usr/bin/env bash
    set -euo pipefail
    while read -r pass; do
      if [ -z "$pass" ] || [ "$pass" = "host" ]; then continue; fi
      echo "test: NOT exercised — $pass"
    done <<< "{{check_matrix}}"

# nextest runs each test in its own process: faster via parallelism, and it
# surfaces hidden inter-test coupling because lazy statics reset per test. That
# isolation is also the migration cost — a suite relying on shared in-process
# state will need rework.
#
# `--show-progress=none` because nextest's progress bar redraws with carriage
# returns padded to the terminal width, and a terminal resize reflows that
# padding into sheared, unreadable scrollback. A gate's output is a log you read
# back, not a display you watch.
#
# `--status-level fail` because one line per passing test is noise in something
# you run constantly — hundreds of lines on a real suite. The summary still
# prints, and a failure still prints in full.
[doc("Unit and integration tests, via nextest")]
test-unit:
    cargo nextest run --workspace --locked --show-progress=none --status-level fail

[doc("Doctests — nextest cannot run these, so cargo test does")]
test-doc:
    cargo test --workspace --locked --doc

# Once per matrix line, for the same reason as `lint`: a broken intra-doc link on
# an item behind a `cfg` is invisible to a pass that does not select it. `xtask`
# is documented once here and excluded from the passes, as in `lint`.
[doc("Docs build with no rustdoc warnings, once per `check_matrix` line")]
doc:
    #!/usr/bin/env bash
    set -euo pipefail
    RUSTDOCFLAGS='-D warnings' cargo doc --package xtask --no-deps --locked
    while read -r pass; do
      if [ -z "$pass" ]; then continue; fi
      if [ "$pass" = "host" ]; then pass=""; fi
      echo "doc: ${pass:-host, default features}"
      RUSTDOCFLAGS='-D warnings' cargo doc --workspace --exclude xtask --no-deps --locked $pass
    done <<< "{{check_matrix}}"

[doc("All four doc/code drift checks")]
drift:
    cargo xtask drift

# ------------------------------------------------------------- drift checks
#
# These four moved out of inline shell into `xtask/` — pure Rust, zero
# dependencies, unit-tested. Not because they are complex (they are small), but
# because shell makes silent failure the default: the same bug shipped three
# separate times here, a `grep` exiting non-zero on no-match which under
# `set -e` killed the recipe before it could print its own error. In Rust that
# mistake does not compile, and `cargo test -p xtask` pins the behaviour.
#
# Each check reports its denominator even when zero — a silent pass on an empty
# input reads as coverage, which is how a check quietly becomes a no-op.

[doc("No tracked file references a gitignored path")]
drift-ignored-refs:
    cargo xtask ignored-refs

[doc("Every file path cited in the docs exists")]
drift-doc-paths:
    cargo xtask doc-paths

[doc("Every Rust symbol named in the docs exists in the source")]
drift-doc-symbols:
    cargo xtask doc-symbols

[doc("No panic-lint escape hatches (saturating_/wrapping_/checked_().unwrap_or)")]
drift-escape-hatches:
    cargo xtask escape-hatches

# Supply chain: advisories, licenses, bans, sources. ~1s warm. This is the one
# step in `check` that touches the network (refreshing the RustSec advisory DB),
# and it earns that: a yanked or vulnerable dependency is worth hearing about
# the moment you add it, not at push time once you have built on top of it.
[doc("Supply chain: advisories, licenses, bans, sources")]
deps:
    # `--deny unmatched-skip` has no deny.toml equivalent — cargo-deny rejects
    # the key — so it has to be here. Without it a `[bans] skip` entry that has
    # outlived its reason reports as a warning and stays forever, which turns
    # the exemption list into the thing it exists to prevent.
    cargo deny check --deny unmatched-skip

# -------------------------------------------------------------------- verify

# Takes the base so that one recipe is the bar everywhere it is enforced — a
# pre-push hook and CI both run this, and CI knows the base of a pull request
# while a laptop does not. Empty means `mutants` works it out, which is what a
# hook wants.
[doc("Everything in `check`, plus mutation testing. The bar for pushing.")]
verify base="": check (mutants base)

# The declared rust-version is decorative unless something builds against it, and
# clippy::incompatible_msrv catches std-API breaks only, never new syntax.
#
# Normally free: skipped outright when rust-version equals the pinned toolchain
# (`lint` already compiled against that exact compiler) or is absent (an
# application pins its toolchain and has no consumers). Only a genuine
# compatibility promise costs a build against a second compiler, because
# consumers build with THEIR toolchain and never see rust-toolchain.toml.
[doc("Workspace compiles against the declared rust-version, per `check_matrix`")]
msrv:
    cargo xtask msrv "{{check_matrix}}"

# Coverage measures which lines RAN. Mutation testing measures whether the tests
# would NOTICE if the code were wrong — the distinction that matters for
# agent-written tests, which tend to be plentiful and shallow.
#
# Budget roughly 3s per mutant, at about one mutant per 20 lines of code. A full
# sweep of a mature codebase therefore runs to tens of minutes — too slow to gate
# every push on. So this mutates only what CHANGED: the existing backlog does not
# block you, but newly added untested logic does.
#
# Equivalent mutants — rewrites that are semantically identical, so no test can
# ever kill them — are real and unavoidable, and are often a sizeable share of
# the survivors. Mark those at the source with `#[mutants::skip]` and a comment
# saying why it is equivalent. Do not loosen a global threshold instead.
[doc("Mutation-test only what changed since the base commit")]
mutants base="":
    cargo xtask mutants {{base}}

[doc("Full mutation sweep over the whole tree. Slow — run deliberately.")]
mutants-full:
    cargo mutants --workspace --no-shuffle

# -------------------------------------------------------------------- hooks

# Git never clones hooks, so this is opt-in per checkout and has to be. What it
# points at is tracked in `.githooks/`, which means the hook is versioned with
# the code it checks — something a hook living in `.git/hooks/` never is.
#
# It is a convenience and not a guarantee: `--no-verify` skips it, and a clone
# that never runs this has no hook. The guarantee is the workflow plus a rule
# that a pull request cannot merge until it passes.
[doc("Point git at this repository's tracked hooks, so the gate runs before a commit")]
hooks:
    git config core.hooksPath .githooks
    @echo "hooks installed: check before each commit, verify before each push"
    @echo "(--no-verify skips either; CI runs verify regardless)"

# ------------------------------------------------------------------- extras

[doc("Tight edit loop with warnings suppressed. NOT a gate — `just check` is.")]
watch:
    RUSTFLAGS=-Awarnings cargo watch -c -x 'run -q'

# Silent when everything is present, so it costs nothing as a dependency of
# `check` in the tight loop. It installs only on a machine that is missing
# something, and only what is missing.
#
# This is the one recipe that keeps its logic here rather than in `xtask`, and
# that is the rule rather than an oversight: shell moves to Rust when it stops
# being readable by inspection. A loop over three names with one conditional
# still is, and it has no way to fail silently — `command -v` and `cargo
# install` both report honestly. `msrv` and `mutants` moved because they parse
# files and resolve git refs, where a non-zero exit is an ordinary answer and
# shell turns that into an abort.
[doc("Install any gate tools this machine is missing")]
tools:
    #!/usr/bin/env bash
    set -euo pipefail
    # A space-separated string rather than an array: bash 3.2 still ships on
    # macOS, and empty-array expansion under `set -u` is an error there.
    missing=""
    for tool in cargo-nextest cargo-deny cargo-mutants; do
      if ! command -v "$tool" >/dev/null 2>&1; then missing="$missing $tool"; fi
    done
    if [ -z "$missing" ]; then exit 0; fi
    # Plain `cargo install` builds from source: native and trustworthy, just
    # slow. binstall is the fast path, but only under two constraints:
    #
    #   --target <host>            Without it, binstall will happily hand an
    #                              Apple Silicon machine an x86_64 binary when
    #                              that is all upstream publishes, relying on
    #                              Rosetta. On a machine without Rosetta that is
    #                              `Bad CPU type in executable`, not a slowdown.
    #   --disable-strategies       Forcing the host target otherwise falls back
    #     quick-install            to a third-party rebuild service. Every other
    #                              binary this project trusts comes from its own
    #                              maintainer or from source, and deny.toml
    #                              refuses unknown registries — a binary from a
    #                              rebuilder would be the odd one out.
    #
    # Together: each tool is the crate author's own release for THIS
    # architecture, or compiled here. Anything else falls through to a source
    # build, which costs minutes once and is the honest price.
    installer="install"
    if command -v cargo-binstall >/dev/null 2>&1; then
      host=$(rustc -vV | awk '/^host:/{print $2}')
      installer="binstall --no-confirm --target $host --disable-strategies quick-install"
    fi
    echo "installing gate tools:$missing"
    for tool in $missing; do cargo $installer "$tool"; done

# Everything here is DERIVED. Nothing about where the project stands is written
# down anywhere, because written status is the largest single source of doc
# drift — it claims to describe a state that moves without it. Ask git instead.
[doc("Orient a fresh session: branch, recent work, open questions")]
brief:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "branch    $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '(no repo)')"
    dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')
    echo "worktree  $([ "$dirty" = 0 ] && echo clean || echo "$dirty uncommitted change(s)")"
    echo
    echo "recent work — commit messages carry the reasoning:"
    git log --oneline -8 2>/dev/null | sed 's/^/  /' || echo "  (no history)"
    echo
    if [ -f docs/open-questions.md ]; then
      echo "open questions and intended next steps — docs/open-questions.md:"
      sed -n '/^## Undecided/,$p' docs/open-questions.md | grep -E '^- ' | sed 's/^/  /' \
        || echo "  (none listed)"
    fi
    if [ -f .local/open-questions.md ]; then
      echo
      echo "private queue (gitignored, not shared) — .local/open-questions.md:"
      sed -n '/^## /,$p' .local/open-questions.md | grep -E '^- ' | sed 's/^/  /'
    fi
    echo
    echo "Run \`just check\` before changing anything: knowing whether the tree was"
    echo "already green is what tells you later whether a failure is yours."

# ------------------------------------------------------------------ project

# Project-specific recipes live there, so that everything above stays identical
# across projects built from this gate and is worth learning once. Optional, so
# deleting it is fine; anything defined there appears in `just --list` alongside
# these.
import? 'project.just'
