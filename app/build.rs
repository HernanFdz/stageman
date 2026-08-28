//! Guarantees the stylesheet exists before the compiler looks for it.
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

use std::fs;
use std::path::Path;

fn main() {
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
        "/* No stylesheet was built. `just dashboard` compiles the real one;\n   \
         a build without it serves a complete, unstyled page. */\n",
    );
}
