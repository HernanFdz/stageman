//! What this binary says it is.
//!
//! Nothing in the source names a version. A release is a tagged binary rather
//! than a package — `docs/decisions/0039-a-release-is-a-tagged-binary.md` — so
//! the version belongs to the tag, is implanted when the tag is built, and is
//! simply absent otherwise.
//!
//! **Absent is a real answer and not a degraded one.** A build somebody made
//! on their own machine is not a release and should not claim to be one. What
//! it can still say is what it was built for, which every build knows.
//!
//! The variables are read here with `option_env!` and checked in the build
//! script. That split is deliberate: the compiler tracks an `option_env!` and
//! rebuilds when it changes, so reading is safe here — but a *missing* one
//! would silently produce [`None`], and a release that reports itself as no
//! release is exactly the failure that must not be quiet. So the refusal lives
//! where it can fail a build.

use std::fmt;

/// What a release knows about itself.
///
/// All four together or none of them. Fields are not individually optional,
/// because a release that knows its version and not its commit is not a
/// partially-known release — it is a broken one, and the build script refuses
/// to produce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    /// The tag this was cut from, without its leading `v`.
    pub version: &'static str,
    /// The commit that tag named.
    pub commit: &'static str,
    /// When that commit was made — not when this was built.
    ///
    /// The distinction is the point: a rebuild of one tag must not produce a
    /// binary claiming a different date, or the version names two things.
    pub date: &'static str,
    /// The triple it was built for.
    pub target: &'static str,
}

/// The triple this was built for, which is known whether or not it is a
/// release.
///
/// Set by the build script, because cargo tells a build script its target and
/// tells the crate nothing. That also makes it the one piece of provenance
/// nobody invoking a build can get wrong.
pub const TARGET: &str = env!("STAGEMAN_BUILD_TARGET");

/// What was implanted at build time, if anything was.
///
/// The catch-all arm is reachable only when nothing was supplied at all. A
/// build given some of it and not the rest never gets here, because the build
/// script stops first.
pub const RELEASE: Option<Release> = assembled(
    option_env!("STAGEMAN_BUILD_VERSION"),
    option_env!("STAGEMAN_BUILD_COMMIT"),
    option_env!("STAGEMAN_BUILD_DATE"),
);

/// A release from three things a build was told, or nothing.
///
/// Split from [`RELEASE`] so that it can be handed values, which is the only
/// way any of this is assertable: what the constant reads is decided when this
/// crate is compiled, and the gate compiles it having been told nothing — so a
/// test against the constant would be a test that nothing produces nothing.
///
/// All three or none. A build given some of them never reaches here, because
/// the build script refuses first; what this handles is the ordinary case of a
/// build told nothing at all.
const fn assembled(
    version: Option<&'static str>,
    commit: Option<&'static str>,
    date: Option<&'static str>,
) -> Option<Release> {
    match (version, commit, date) {
        (Some(version), Some(commit), Some(date)) => Some(Release {
            version,
            commit,
            date,
            target: TARGET,
        }),
        _ => None,
    }
}

/// How a release describes itself in one line.
impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} {}) {}",
            self.version, self.commit, self.date, self.target
        )
    }
}

/// One line naming this binary, for a summary that has other things to say.
///
/// Used on the startup block, where it is one fact among a dozen, and by the
/// tool server describing itself.
#[must_use]
pub fn described() -> String {
    RELEASE.map_or_else(
        || format!("none — not a release build, for {TARGET}"),
        |release| release.to_string(),
    )
}

/// How wide the labels are, so that these lines and the startup block's align.
///
/// The two are read together often enough that a reader should not have to
/// notice they were produced by different code.
const LABEL: usize = 10;

/// Every field on its own line, for somebody who asked and nothing else.
///
/// A different shape from [`described`] rather than a different truth: both
/// read the same constant, so they cannot disagree about what this binary is,
/// only about how much room they have to say it. `--version` has the whole
/// terminal; the startup block has one line among a dozen.
///
/// A build that is not a release says so and stops, rather than printing empty
/// labels. There is no commit it is declining to mention — there is none.
#[must_use]
pub fn detailed() -> String {
    let mut said = String::new();
    match RELEASE {
        Some(release) => {
            said.push_str(&line("version", release.version));
            said.push_str(&line("commit", release.commit));
            said.push_str(&line("date", release.date));
        }
        None => said.push_str(&line("version", "none — not a release build")),
    }
    // Named `target` rather than `arch` because that is what the value is: an
    // architecture is `aarch64`, and this also carries the vendor, the system
    // and the binary interface, each of which decides whether the file will
    // run somewhere.
    said.push_str(&line("target", TARGET));
    said
}

/// One labelled line, padded to the width the startup block uses.
fn line(label: &str, value: &str) -> String {
    format!("  {label:<LABEL$} {value}\n")
}

#[cfg(test)]
mod tests {
    use super::{RELEASE, Release, TARGET, described};

    /// A build knows its target whether or not it is a release.
    #[test]
    fn the_target_is_always_known() {
        assert!(!TARGET.is_empty());
        assert_ne!(
            TARGET, "unknown",
            "the build script could not read a target"
        );
    }

    /// Three things make a release, and any two do not.
    ///
    /// Every combination, because the failure is one-sided in both directions:
    /// something that accepted two would produce a release which cannot say
    /// where it came from, and something that accepted none would make every
    /// build claim to be one.
    #[test]
    fn a_release_needs_all_three_or_none() {
        use super::assembled;

        let whole = assembled(Some("0.2.0"), Some("abc123"), Some("2026-09-01"))
            .expect("all three make a release");
        assert_eq!(whole.version, "0.2.0");
        assert_eq!(whole.commit, "abc123");
        assert_eq!(whole.date, "2026-09-01");
        assert_eq!(
            whole.target, TARGET,
            "the target is never supplied, only derived"
        );

        assert!(
            assembled(None, None, None).is_none(),
            "a local build is not one"
        );
        assert!(assembled(None, Some("abc123"), Some("2026-09-01")).is_none());
        assert!(assembled(Some("0.2.0"), None, Some("2026-09-01")).is_none());
        assert!(assembled(Some("0.2.0"), Some("abc123"), None).is_none());
    }

    /// Every line is labelled, and the labels line up.
    ///
    /// Alignment is not decoration here: this output and the startup block are
    /// read together, and a reader should not have to notice they came from
    /// different code.
    #[test]
    fn what_it_says_in_detail_is_labelled_and_aligned() {
        use super::detailed;

        let said = detailed();
        let lines: Vec<&str> = said.lines().collect();

        assert!(!lines.is_empty(), "it must say something");
        assert!(
            said.ends_with('\n'),
            "the last line needs its terminator: {said:?}",
        );
        for line in &lines {
            assert!(line.starts_with("  "), "not indented: {line:?}");
            let value = line.find(char::is_whitespace).map(|_| ());
            assert!(value.is_some(), "no label and value: {line:?}");
        }
        // Every value begins at the same column, whatever the label's length.
        let columns: Vec<usize> = lines
            .iter()
            .filter_map(|line| {
                line.rfind("  ")
                    .map(|_| line.len() - line.trim_start().len())
            })
            .collect();
        assert!(columns.iter().all(|start| *start == 2), "{columns:?}");
        assert!(
            lines.iter().any(|line| line.contains("target")),
            "a build always knows what it was built for: {said}",
        );
    }

    /// Detail and summary cannot disagree about what this binary is.
    ///
    /// They are different shapes on purpose, so what has to hold is that every
    /// fact in the short one appears in the long one.
    #[test]
    fn the_two_shapes_say_the_same_things() {
        use super::detailed;

        let detail = detailed();
        assert!(detail.contains(TARGET), "{detail}");
        match RELEASE {
            Some(release) => {
                assert!(detail.contains(release.version), "{detail}");
                assert!(detail.contains(release.commit), "{detail}");
                assert!(detail.contains(release.date), "{detail}");
            }
            None => assert!(detail.contains("not a release build"), "{detail}"),
        }
    }

    /// What is said matches whether anything was implanted.
    ///
    /// Asserted against whichever this build happens to be, because the gate
    /// compiles one that is not a release and a tagged build compiles one that
    /// is — so a test naming either state would pass in one and fail in the
    /// other.
    #[test]
    fn what_it_says_follows_what_it_carries() {
        let said = described();
        assert!(
            said.contains(TARGET),
            "it should always name its target: {said}"
        );
        match RELEASE {
            Some(release) => {
                assert!(said.contains(release.version), "{said}");
                assert!(said.contains(release.commit), "{said}");
                assert!(!said.contains("not a release"), "{said}");
            }
            None => assert!(
                said.contains("not a release build"),
                "a build carrying nothing must not imply it is one: {said}",
            ),
        }
    }

    /// A release says every part of itself, in one line.
    ///
    /// Built here rather than read from the compiled-in one, which is empty in
    /// the gate — an assertion against that would assert nothing.
    #[test]
    fn a_release_names_all_four_things() {
        let release = Release {
            version: "0.2.0",
            commit: "abc123def456",
            date: "2026-09-01",
            target: "x86_64-unknown-linux-gnu",
        };

        let said = release.to_string();

        assert!(said.contains("0.2.0"), "{said}");
        assert!(said.contains("abc123def456"), "{said}");
        assert!(said.contains("2026-09-01"), "{said}");
        assert!(said.contains("x86_64-unknown-linux-gnu"), "{said}");
        assert!(!said.contains('\n'), "a version is one line: {said}");
    }
}
