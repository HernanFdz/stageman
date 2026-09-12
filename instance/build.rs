//! What this build knows about itself, implanted at compile time.
//!
//! Three variables make a build a release — see
//! `docs/decisions/0039-a-release-is-a-tagged-binary.md` — and the target is
//! always known. Read here rather than trusted to change detection, because
//! this decides whether to fail, and a decision made on a stale value is
//! worse than no decision.

/// What makes a build a release.
const VERSION: &str = "STAGEMAN_BUILD_VERSION";

/// What a release must also say about itself.
const PROVENANCE: &[&str] = &["STAGEMAN_BUILD_COMMIT", "STAGEMAN_BUILD_DATE"];

/// The target every build is told.
const TARGET: &str = "STAGEMAN_BUILD_TARGET";

fn main() {
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
