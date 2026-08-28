# Gate reference

Read this when `just check` or `just verify` fails. It is **not** loaded by
default and does not need to be — the gate is the trigger, this file is the
answer.

Every heading is the exact name the tool prints, so grep for that rather than
reading end to end. If this ever grows past comfortable loading, split it by
heading — but not before: an index of sections is a list the filesystem already
knows, and neighbouring entries are frequently the ones you need, since the
wrong fix for one lint is usually the subject of another.

It deliberately does **not** restate what the tool already printed. Its only job
is the part the tool cannot say: **which fix is correct, and which fixes look
correct but pass the gate while being wrong.**

---

## `clippy::arithmetic_side_effects`

Any `+ - * / %` that can overflow, underflow, or divide by zero.

**Correct fix — propagate.**

```rust
let next = self.0.checked_add(1).ok_or(Error::CounterExhausted)?;
```

**Correct fix for property-test oracles.** A property test must compute its
expected answer independently, in plain arithmetic (`(x + y) % MODULUS`), and
`clippy.toml` has `allow-{unwrap,expect,panic,indexing-slicing}-in-tests` but
**no arithmetic equivalent**. Add to the *crate root*:

```rust
#![cfg_attr(test, expect(
    clippy::arithmetic_side_effects,
    reason = "property-test oracle arithmetic; overflow fails the test, not production"
))]
```

The `cfg_attr(test, …)` scoping is not optional. `--all-targets` compiles the
lib target *without* `cfg(test)`, so production code stays fully checked —
verified by injecting `a + b` into a non-test function and watching it still
fail. An **unscoped** crate-root `expect` is satisfied by a single firing, so it
never self-deletes and it hides every future violation in the crate.

**Wrong fixes that pass the gate.** Each replaces a loud failure with a silent
wrong answer. A panic is a correct program halting; these are incorrect programs
continuing.

| Written | What actually happens |
|---|---|
| `self.0.saturating_add(1)` | the counter sticks at `MAX` forever |
| `i.checked_rem(cap).unwrap_or(0)` | writes slot 0 instead of failing |
| `u64::try_from(x).unwrap_or(0)` | invents a value out of nothing |
| `a.wrapping_sub(b)` | silently wraps to a huge number |

`just drift-escape-hatches` greps for these. If a clamp genuinely *is* the
intended semantics, say so on the line: `// CLAMP-OK: <why this default is correct>`.

---

## `clippy::indexing_slicing`, `clippy::string_slice`

**Correct fix:** `.get(i).ok_or(…)?` / `.get(a..b).ok_or(…)?`.

**Wrong fix that passes the gate:**

```rust
if let Some(slot) = slots.get_mut(idx) { *slot = v; }   // no else — write silently dropped
```

No lint can catch this; it is legitimate code most of the time. If you take the
`if let` path, handle the `None` arm.

Note `string_slice` is about UTF-8: `&s[0..3]` panics mid-character. Use
`char_indices()` or `.get(..)`.

---

## `clippy::unwrap_used`, `expect_used`, `panic`, `panic_in_result_fn`

**Correct fix:** propagate with `?`, or return `Err`. In a proc-macro crate,
report through `syn::Error::new(span, "…")` — a panic there produces a span-less
diagnostic pointing into macro internals rather than at the caller's invocation.

**Wrong fix that passes the gate:** `.unwrap_or_default()`. This is widely
recommended, and it is correct *only* when the default is the semantically
intended value for that failure — `"abc".parse::<i32>().unwrap_or_default()` is
fine if unparseable genuinely means zero. It is wrong everywhere else, and the
distinction is a judgement made per site, never a mechanical substitution.

These are already allowed in tests; you should not need an exemption there —
with one gap worth knowing before you go looking for a knob that does not
exist. `allow-expect-in-tests` and its siblings recognise `#[test]` functions
and `#[cfg(test)]` modules. A file under `tests/` is its own crate, so a *helper*
there — a fixture builder, a command runner — is not inside either and the lint
fires on it while firing on nothing in the tests themselves. Put the
`#[expect(…, reason = "…")]` at the top of that file rather than moving working
code into the test bodies to satisfy a detection rule.

---

## `clippy::as_conversions`

**Correct fix:** `u64::try_from(x)?`. Note the panic lints make the lazy escape
(`try_from(x).unwrap()`) fail too, which is intentional — the two lints
reinforce each other into real error handling.

Expect this to be the most-suppressed lint in the set. It overlaps heavily with
pedantic's targeted cast lints, and is kept because "never `as`" is a uniform
rule that is followed reliably, while "`as` only when lossless" requires a
judgement call at every site.

---

## `unfulfilled_lint_expectations`

The suppression is **dead** — the lint it silences no longer fires. Delete the
whole attribute. Do not convert it back to `#[allow]`; that is what it is there
to prevent.

This is the mechanism that stops a strict config decaying into a wall of dead
suppressions, so treat a firing as the system working, not as an obstacle.

## `clippy::allow_attributes` / `allow_attributes_without_reason`

Replace `#[allow(lint)]` with `#[expect(lint, reason = "…")]`. The `reason` is
required and should say *why the lint does not apply here*, not what the lint is.

Converting `allow` → `expect` sometimes immediately trips
`unfulfilled_lint_expectations`. That means the suppression was already dead —
delete it rather than reaching for `allow` again.

---

## `clippy::incompatible_msrv` / `just verify-msrv`

Something uses a `std` API newer than the declared `rust-version`. Either raise
`rust-version` deliberately, or use an older API. Note clippy catches std-API
breaks only, not new *syntax* — that is why `just verify-msrv` compiles against
the actual toolchain.

## `clippy::cargo_common_metadata`

Missing `description` / `repository` / `keywords` / `categories` / `readme`.
Fill them in. Do not add a blanket `allow`: this lint fires on every project
that has not filled in its metadata, and a permanently-allowed lint is a check
you have trained yourself to ignore.

---

## `just verify-deps` reports a wildcard dependency

`deny.toml` sets `allow-wildcard-paths = true`, which exempts intra-workspace
path dependencies. If a wildcard is *still* reported, cargo-deny has found a
real defect, not noise: crates.io forbids bare path dependencies, so a
**publishable** crate depending on a sibling by path with no `version` cannot be
published at all. Add `version = "…"` alongside the path.

## `just drift-ignored-refs`

A tracked file references a gitignored path, so a fresh clone will not build.
Either track the file, or — if it is a build artifact — add it to
`.quality/generated-paths` along with the command that generates it, and make
sure that command actually runs as part of `just check`.

## `just drift-doc-symbols` flags a name that is not yours

The check treats a backticked identifier as a claim about *this* crate's source.
The common false positive is prose naming a **std method you do not call** —
`` `saturating_add` `` in a passage explaining why you avoid it, for example.

Prefer rewording so the name is not backticked: an unbackticked mention is not a
claim about your code. Reach for `.quality/doc-symbols-ignore` only for names
that genuinely recur, and keep that list short — if it grows, your docs are
probably describing another codebase, which is its own rule violation.

## `just drift-doc-paths` passes locally and fails in CI

**Cause:** a backticked path in `docs/` that exists on your machine and in no
clone. The check resolves citations against the working tree, so a gitignored
file — a credential, a scratch note, anything under a local-only directory —
satisfies it where it was written and nowhere else. Every reviewer sees green;
every fresh checkout sees red.

**Correct fix:** do not cite it as a path. `is_path_like` needs a `/`, so
naming the file and its directory separately (`` `token-file` ``, in the
`.local` directory) says the same thing and claims nothing about the
repository.

**Wrong fix:** adding it to `.quality/generated-paths`. That list is for build
artifacts with a generator, and each entry is a promise that some command
creates the file before anything needs it. A credential has no generator, so
the promise would be false and the next fresh-clone failure would be silent.

**Worth knowing:** the check reads `docs/` only. The same citation in
`AGENTS.md` or `README.md` is not examined, so moving a sentence between them
changes whether it is checked.

---

## `just drift-doc-paths` reports `0 path(s) cited`

Not a failure. It means the check examined nothing, and it says so on purpose: a
silent pass on an empty denominator reads as coverage. If your docs cite symbols
rather than file paths, this check is inert for you and the drift you are
actually exposed to needs a different check.

---

## Two traps in the gate itself

**`cargo clippy` needs `--all-targets`; `cargo test` must not have it.**

Without `--all-targets`, clippy never compiles test/bench/example targets — so a
library can be green while its own tests do not compile, and `#[cfg(test)]` code
is never linted for anything. (This is also why the article's "unwraps are
allowed in tests by default" holds for plain `cargo clippy`: not because clippy
exempts them, but because it never looks.)

With `--all-targets`, `cargo test` **silently skips doctests** — verified: a
deliberately broken doctest passes under `--all-targets` and fails without it.
Doctests are the one mechanism that makes prose and code unable to disagree, so
losing them silently removes the primary anti-drift guarantee.

**A lint run that ended in a compilation error has not linted anything
downstream of it.** If a dependency crate fails, the crates that depend on it
were never checked — and a truncated run is indistinguishable from a clean one.
Before concluding a crate is clean, confirm it actually produced diagnostics.
