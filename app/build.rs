//! Guarantees the two generated inputs exist before the compiler looks for
//! them, and turns one of them into a table the binary carries.
//!
//! Both follow the same rule and it is worth stating once: a directory that
//! only exists after `dx` has run cannot be a hard requirement of `cargo
//! build`, because `just check` never runs `dx` and a fresh clone has not
//! either. So each is created empty when absent, and empty is a meaningful
//! state rather than a papered-over failure.
//!
//! The dashboard attaches its stylesheet with `asset!`, which resolves at
//! compile time and fails the build when the file is not there. The file is
//! Tailwind's output — a build artefact, produced by `dx`, and gitignored like
//! any other. Those two facts do not fit together on a fresh clone, where
//! nothing has run `dx` yet and `just check` deliberately never will.
//!
//! So this creates an empty one if there is none. It is not a fallback hiding
//! a failure: an empty stylesheet is exactly what a build with no browser half
//! *should* have, and the page it produces is complete and unstyled, which is
//! the state that binary was already in. Reasoning in
//! `docs/decisions/0025-a-build-script-guarantees-the-stylesheet-exists.md`.
//!
//! It never overwrites. `dx` writes the real stylesheet to this path before
//! invoking the compiler, and a later plain `cargo build` must leave that
//! alone rather than blank the page.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// What a release build is told its version is.
///
/// The presence of this is what makes a build a release: there is no separate
/// flag, because two variables that can disagree are worse than one that
/// cannot. See `docs/decisions/0039-a-release-is-a-tagged-binary.md`.
const VERSION: &str = "STAGEMAN_BUILD_VERSION";

/// The commit a release was built from, and when that commit was made.
///
/// Required alongside the version rather than optional. The date is the
/// commit's rather than this build's, so that rebuilding one tag cannot
/// produce a binary claiming a different one.
const PROVENANCE: &[&str] = &["STAGEMAN_BUILD_COMMIT", "STAGEMAN_BUILD_DATE"];

/// The triple this is being built for, which the compiler already knows.
const TARGET: &str = "STAGEMAN_BUILD_TARGET";

/// Where `just build` leaves the browser's half for this to embed.
const BUNDLE: &str = "bundle";

/// What the generated table is called inside `OUT_DIR`.
const TABLE: &str = "bundle.rs";

fn main() {
    stylesheet();
    embed();
    provenance();
}

/// Guarantees the stylesheet exists before `asset!` looks for it.
///
/// Split from [`main`] so that its early return means "there is already a
/// stylesheet" and nothing more. It used to return from `main` itself, which
/// silently made everything after it conditional on the stylesheet being
/// absent — a shape that works exactly once, on a fresh clone.
fn stylesheet() {
    // Only the directory, not the file: the file is gitignored, and asking
    // cargo to watch it would make every `dx` rebuild invalidate this crate.
    println!("cargo::rerun-if-changed=assets");

    let stylesheet = Path::new("assets/styles.css");
    if stylesheet.exists() {
        return;
    }

    // Best effort, and deliberately silent about failing. If the file cannot
    // be created, `asset!` reports that it is missing and names the path,
    // which is a better message than anything this could print — and it stops
    // the build either way, so nothing proceeds on a false assumption.
    let _ = fs::create_dir_all("assets");
    let _ = fs::write(
        stylesheet,
        "/* No stylesheet was built. `just dev` compiles the real one;\n   \
         a build without it serves a complete, unstyled page. */\n",
    );
}

/// Writes a table naming every file in the bundle directory and its bytes.
///
/// The browser's half, compiled in so that what ships is one file — see
/// `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`. The
/// directory is populated by the first pass of `just build` and emptied by
/// `just dev`, so in an ordinary `cargo build` it is empty and the table has
/// no entries. That is the state a fresh clone is in, and the binary it
/// produces behaves exactly as it did before any of this existed.
///
/// Written by hand rather than by a crate. What a crate would offer is a macro
/// over a directory walk, and this is the walk — against a dependency that
/// would need auditing, licence-checking and pinning for ever.
fn embed() {
    // The directory rather than its contents, for the reason the stylesheet
    // above watches its directory: naming the files would mean naming files
    // that do not exist yet on the run that creates them.
    println!("cargo::rerun-if-changed={BUNDLE}");

    let root = Path::new(BUNDLE);
    let _ = fs::create_dir_all(root);

    let mut found = Vec::new();
    collect(root, root, &mut found);
    // Sorted, so that the table is the same for the same directory. An
    // unordered walk would produce a different file on every build and make
    // every downstream artefact differ for no reason anybody could see.
    found.sort();

    let mut table = String::from(
        "/// Every file of the browser's half, as (path, bytes).\n\
         ///\n\
         /// Generated by this crate's build script. Empty when nothing has\n\
         /// been built into the bundle directory, which is the ordinary state\n\
         /// of a plain `cargo build`.\n\
         pub static EMBEDDED: &[(&str, &[u8])] = &[\n",
    );
    for (served, absolute) in &found {
        // Both are written as string literals, so a path containing a quote or
        // a backslash would produce a table that does not compile. Rejected
        // rather than escaped: an asset filename is written by the bundler and
        // is a hash, and a name that needs escaping means something is wrong
        // further back than here.
        if served.contains(['"', '\\']) || absolute.contains(['"', '\\']) {
            println!("cargo::warning=skipping bundle file with an unquotable name: {served}");
            continue;
        }
        // `write!` rather than pushing a formatted string, which allocates a
        // second time for no reason the gate is willing to overlook.
        let _ = writeln!(table, "    (\"{served}\", include_bytes!(\"{absolute}\")),");
    }
    table.push_str("];\n");

    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let _ = fs::write(Path::new(&out).join(TABLE), table);
}

/// Every file under `at`, as the path it is served under and its absolute path.
///
/// Served paths use forward slashes whatever the platform, because they are
/// URLs rather than filenames: the browser asks for what `index.html` names,
/// and that was written by a bundler that does not know about Windows.
fn collect(root: &Path, at: &Path, found: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, found);
        } else if let Ok(relative) = path.strip_prefix(root)
            && let Ok(absolute) = fs::canonicalize(&path)
        {
            let served = relative
                .components()
                .filter_map(|part| part.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join("/");
            if let Some(absolute) = absolute.to_str() {
                found.push((served, absolute.to_owned()));
            }
        }
    }
}

/// Settles what this binary will say about itself.
///
/// Two jobs, and they are here rather than in the source for opposite reasons.
///
/// The target is *derived*: cargo tells a build script what it is building for
/// and tells the crate nothing, so this is the only place it can come from —
/// which is also the right place, because it is the one part of a release's
/// provenance that whoever invoked the build could get wrong and the build
/// cannot.
///
/// The rest is *checked*: `option_env!` in the source would let a release
/// built without its commit fall back to reporting no release at all, quietly.
/// A release missing its provenance is broken rather than partial, so it stops
/// here instead.
fn provenance() {
    // Read through `env::var` rather than trusted to change detection: this
    // decides whether to fail, and a decision made on a stale value is worse
    // than no decision. `option_env!` in the source is separately tracked by
    // the compiler, which is why nothing else here has to be.
    println!("cargo::rerun-if-env-changed={VERSION}");
    for named in PROVENANCE {
        println!("cargo::rerun-if-env-changed={named}");
    }

    if std::env::var_os(VERSION).is_some() {
        for named in PROVENANCE {
            assert!(
                std::env::var_os(named).is_some(),
                "{VERSION} is set, so this is a release build, and {named} is not set.\n\
                 A release that cannot say which commit it came from is broken rather \
                 than partial,\n  so this refuses rather than building a binary that \
                 reports itself as no release at all.",
            );
        }
    }

    // Always set, so the source reads it with `env!` rather than testing for
    // it: every build knows its own target, release or not.
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo::rustc-env={TARGET}={target}");
}
