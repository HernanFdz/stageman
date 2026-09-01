//! Pure text scanning. No I/O, no process spawning — everything here is a
//! function from `&str` to a decision, which is what makes it unit-testable.
//!
//! This module exists because the shell versions of these checks shipped the
//! same class of bug three times: a `grep` that exits non-zero on no-match,
//! which under `set -e` killed the recipe *before* it could report anything.
//! Silent failure was the default. Here the equivalent mistake does not
//! compile, and the tests below pin the behaviour that used to be implicit.
//!
//! CLAMP-OK-FILE: this module NAMES the clamping patterns rather than using
//! them — a pattern table plus its tests. Without a file-level exemption the
//! escape-hatch check flags its own implementation.

use std::collections::BTreeSet;

/// Backtick-quoted spans in a Markdown document, excluding fenced code blocks.
///
/// Fence-awareness is a deliberate improvement on the shell version, which
/// happily scraped identifiers out of ` ```rust ` examples and then complained
/// that they did not exist.
pub fn backticked(markdown: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Splitting on the delimiter puts quoted spans at the odd positions.
        found.extend(line.split('`').skip(1).step_by(2));
    }
    found
}

/// Whether a token is a repo-relative path rather than prose.
///
/// Conservative in two ways. A `/` is required, so a bare `Cargo.toml` mention
/// is ignored. And a **leading** `/` disqualifies it: an app's docs are full of
/// backticked URL routes (`/games/:game_id`, `/login`), which are not files and
/// must not be resolved as if they were.
pub fn is_path_like(token: &str) -> bool {
    // Trim first, then require a separator in what remains. That keeps
    // `bases/error/` (a real directory the docs claim exists) while dropping
    // `api/` — prose shorthand for "the api subdirectory of each module", which
    // has no single path to resolve against.
    let trimmed = token.trim_end_matches('/');
    trimmed.contains('/')
        && !trimmed.contains(char::is_whitespace)
        && !trimmed.starts_with('/')
        && !trimmed.contains("://")
        && !trimmed.contains('*')
        && !trimmed.starts_with("http")
}

/// Whether a token is a claim about a Rust item in this codebase.
///
/// Requires an underscore or a camelCase transition, which skips ordinary words
/// in backticks (`Locked`, `true`) at the cost of also skipping short type names
/// (`Api`, `Bit`). Under-reporting beats crying wolf: a check people learn to
/// ignore is worse than no check.
pub fn is_symbol_like(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    if token.contains('_') {
        return true;
    }
    token
        .chars()
        .zip(token.chars().skip(1))
        .any(|(a, b)| a.is_ascii_lowercase() && b.is_ascii_uppercase())
}

/// The panic-lint escape hatch used on this line, if any.
///
/// Each of these silences a denied lint by producing a wrong value instead of
/// failing: a panic is a correct program halting, these are incorrect programs
/// continuing. `// CLAMP-OK:` marks a clamp that genuinely is the intent.
pub fn escape_hatch(line: &str) -> Option<&'static str> {
    if line.contains("CLAMP-OK:") {
        return None;
    }
    for pattern in [
        "saturating_add",
        "saturating_sub",
        "saturating_mul",
        "wrapping_add",
        "wrapping_sub",
        "wrapping_mul",
    ] {
        if line.contains(pattern) {
            return Some(pattern);
        }
    }
    // `as_conversions` is denied, so the lazy escape from it is
    // `u64::try_from(x).unwrap_or(0)`. Blocking `unwrap()` alone leaves it open.
    if line.contains(".unwrap_or")
        && (line.contains("checked_") || line.contains("try_from") || line.contains("try_into"))
    {
        return Some("checked_/try_* followed by .unwrap_or(default)");
    }
    None
}

/// Whether a token could name a file inside this repository.
///
/// Test data below deliberately avoids `tmp/`: it is a near-universal
/// gitignore entry, so a literal using it makes this very file a tracked file
/// naming an ignored path — which is the violation `ignored-refs` reports.
///
/// Worth its own function because `git check-ignore` aborts *fatally* on a
/// malformed path, which takes the whole check down with it — a URL tokenised
/// out of `Cargo.toml` did exactly that, and the check silently reported
/// nothing instead of the defect it was pointed at.
pub fn plausible_repo_path(token: &str) -> bool {
    // A leading dot is allowed only when a separator follows it. Without that
    // distinction this check could not see dotted directories AT ALL — the
    // commonest shape of gitignored path there is — while still printing a
    // denominator that read as coverage.
    //
    // A bare dotfile (`.env`) stays out of reach on purpose: it is
    // indistinguishable from an extension named in prose, and a page mentioning
    // `.log` in a project ignoring `*.log` would be reported as a violation.
    // Under-reporting beats crying wolf.
    // `..` must stay out: `git check-ignore` calls a path outside the
    // repository a FATAL error, and with `--stdin` that abort discards the rest
    // of the batch — every later candidate goes unchecked and the check reports
    // a clean result. Letting `../other/src` through did exactly that, and only
    // the broken fixture noticed.
    //
    // A token holding a variable expansion is somebody else's filesystem, not
    // this repository's, and `git check-ignore` cannot tell the difference: it
    // reads `${HOME}` as an ordinary directory name, so `${HOME}/.local/share`
    // matches a `.local/` entry and is reported as a violation of a rule about
    // fresh clones. A shipped shell script naming a path on the machine it
    // installs to is the case that found this, and the reasoning generalises to
    // every `$PWD`, `%h` and `${{ }}` a workflow or recipe contains. The check's
    // own principle applies — under-reporting beats crying wolf.
    let leading_dot_ok = token
        .strip_prefix('.')
        .is_none_or(|rest| rest.contains('/') && !rest.starts_with('.'));
    token.len() > 2
        && token.contains('.')
        && leading_dot_ok
        && !token.starts_with('/')
        && !token.starts_with('-')
        && !token.contains("//")
        && !token.contains('*')
        && !token.contains('$')
}

/// The `check_matrix` from the justfile, as one argument list per pass.
///
/// One line per configuration, blanks ignored, surrounding whitespace trimmed.
/// The word `host` means *this machine with default features*, and it expands to
/// no flags at all rather than to a triple — passing `--target <your own
/// triple>` makes cargo build into `target/<triple>/`, sharing no cache with
/// `cargo run` or rust-analyzer, so "the host" has to be spelled as the absence
/// of `--target`.
///
/// `host` is stripped only as a leading word, so `--features hosted` survives
/// intact.
///
/// An empty matrix yields no passes, and the caller is expected to say so rather
/// than report success: checking nothing is not the same as finding nothing.
pub fn matrix_passes(matrix: &str) -> Vec<Vec<String>> {
    matrix
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let flags = if line == "host" {
                ""
            } else {
                line.strip_prefix("host ").unwrap_or(line)
            };
            flags.split_whitespace().map(str::to_owned).collect()
        })
        .collect()
}

/// Paths the gate itself defines as deliberately untracked.
///
/// Gate infrastructure is not project content — the same exemption `.quality/`
/// already has, for the same reason, recorded here at the exemption. `.local/`
/// is the gate's own scratch and private-queue convention, and `AGENTS.md`
/// documents it, so **every** scaffolded project names it in a tracked file by
/// construction. Flagging that would fail a freshly scaffolded project on its
/// first run, which is the one thing this gate must never do.
///
/// The rule being relaxed also does not apply: the ignored-path check exists so
/// that a fresh clone BUILDS, and nothing is ever compiled out of `.local/`.
/// This is not the allowlist's job — `.quality/generated-paths` promises that a
/// named command generates the file before the compiler needs it, and no
/// command generates a private notes directory.
pub fn gate_scratch(path: &str) -> bool {
    path.trim_end_matches('/') == ".local" || path.starts_with(".local/")
}

/// Whether a tracked file's contents should be searched for path references.
///
/// Two kinds of file legitimately name ignored paths and must not be treated as
/// violations. `.gitignore` does so as its entire purpose. `.quality/` holds the
/// gate's own configuration and documentation — the allowlist is a list of
/// ignored paths by definition, and the reference explains them by example.
/// Neither is project content a fresh clone has to compile.
pub fn scannable_for_paths(path: &str) -> bool {
    !path.ends_with(".gitignore") && !path.starts_with(".quality/")
}

/// A top-level `key = "value"` from a TOML file, without a TOML parser.
///
/// Deliberately strict about the `=`, so `rust-version` does not match a
/// hypothetical `rust-version-policy`.
pub fn toml_string_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().map(str::trim).find_map(|line| {
        let rest = line.strip_prefix(key)?;
        if !rest.trim_start().starts_with('=') {
            return None;
        }
        rest.split('"').nth(1)
    })
}

/// Whether a declared MSRV names the same compiler as the pinned toolchain.
///
/// If it does, `lint` has already compiled the whole workspace with that exact
/// compiler under `--all-targets`, so verifying it again proves nothing.
///
/// The prefix must end at a component boundary: `1.9` is NOT `1.95.0`, and a
/// naive `starts_with` would silently skip a real MSRV check.
pub fn msrv_is_pinned(msrv: &str, channel: &str) -> bool {
    if msrv.is_empty() || channel.is_empty() {
        return false;
    }
    channel == msrv
        || channel
            .strip_prefix(msrv)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// What a docs-versus-source symbol scan found.
pub struct SymbolScan {
    /// How many symbols the docs claimed. Reported even when zero.
    pub cited: usize,
    /// Those the source does not define.
    pub missing: Vec<String>,
}

/// Symbols the docs claim exist but the source never defines.
///
/// Pure so it can be tested: the caller does the reading, this does the
/// deciding. Splitting the source into identifier-shaped words up front gives
/// whole-word matching for free — `bits_one` must not match `bits_one_extra` —
/// and turns N scans into one.
pub fn missing_symbols(docs: &str, source: &str, ignored: &[String]) -> SymbolScan {
    let words: BTreeSet<&str> = source
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect();
    let cited: BTreeSet<&str> = backticked(docs)
        .into_iter()
        .filter(|token| is_symbol_like(token) && !ignored.iter().any(|entry| entry == token))
        .collect();
    SymbolScan {
        cited: cited.len(),
        missing: cited
            .into_iter()
            .filter(|symbol| !words.contains(symbol))
            .map(str::to_owned)
            .collect(),
    }
}

/// Escape-hatch hits in one file, as (1-based line number, pattern).
///
/// A file that NAMES these patterns rather than using them — a linter, a
/// pattern table, documentation — opts out wholesale with `CLAMP-OK-FILE:`.
pub fn escape_hatch_hits(text: &str) -> Vec<(usize, &'static str)> {
    if text.contains("CLAMP-OK-FILE:") {
        return Vec::new();
    }
    text.lines()
        .zip(1usize..)
        .filter_map(|(line, number)| escape_hatch(line).map(|pattern| (number, pattern)))
        .collect()
}

/// Entries not covered by an allowlist.
pub fn unallowed(hits: Vec<String>, allowed: &[String]) -> Vec<String> {
    hits.into_iter()
        .filter(|hit| !allowed.iter().any(|entry| entry == hit))
        .collect()
}

/// One allowlist entry per line, `#` comments and blanks removed.
pub fn allowlist(contents: &str) -> Vec<&str> {
    contents
        .lines()
        .map(|line| match line.split_once('#') {
            Some((before, _)) => before.trim(),
            None => line.trim(),
        })
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        allowlist, backticked, escape_hatch, escape_hatch_hits, gate_scratch, is_path_like,
        is_symbol_like, matrix_passes, missing_symbols, msrv_is_pinned, plausible_repo_path,
        scannable_for_paths, toml_string_value, unallowed,
    };

    #[test]
    fn a_matrix_line_becomes_one_pass_of_flags() {
        let passes = matrix_passes("host\n--target wasm32-unknown-unknown --features web\n");
        assert_eq!(passes.len(), 2);
        assert!(passes[0].is_empty(), "`host` means no flags, not a triple");
        assert_eq!(
            passes[1],
            ["--target", "wasm32-unknown-unknown", "--features", "web"]
        );
    }

    #[test]
    fn blank_lines_and_indentation_are_ignored() {
        // The justfile writes this as a `'''` block, so it arrives with a
        // leading newline and possibly indented lines.
        let passes = matrix_passes("\n  host  \n\n\thost --features server\n");
        assert_eq!(passes.len(), 2);
        assert!(passes[0].is_empty());
        assert_eq!(passes[1], ["--features", "server"]);
    }

    #[test]
    fn host_is_stripped_only_as_a_leading_word() {
        // A blanket replace would eat this and silently change the build.
        assert_eq!(
            matrix_passes("--features hosted")[0],
            ["--features", "hosted"]
        );
        assert_eq!(
            matrix_passes("--target host-thing")[0],
            ["--target", "host-thing"]
        );
    }

    #[test]
    fn an_empty_matrix_yields_no_passes() {
        // The caller must treat this as "nothing was checked" rather than as
        // success — an empty denominator reads as coverage.
        assert!(matrix_passes("\n  \n").is_empty());
    }

    #[test]
    fn the_gates_own_scratch_directory_is_exempt() {
        // AGENTS.md documents `.local/`, so every scaffolded project names it in
        // a tracked file. Without this the gate fails a project it just created.
        assert!(gate_scratch(".local"));
        assert!(gate_scratch(".local/"));
        assert!(gate_scratch(".local/open-questions.md"));
        // Not a licence for anything else that happens to start similarly.
        assert!(!gate_scratch(".localstack/config.yml"));
        assert!(!gate_scratch("docs/notes.md"));
        // Assembled rather than written out: the literal would make this very
        // file a tracked file naming an ignored path — the violation the check
        // reports — exactly as the warning on `plausible_repo_path` says.
        let nested = format!("src/{}/x.rs", ".local");
        assert!(!gate_scratch(&nested), "exempt only at the root");
    }

    #[test]
    fn url_fragments_are_not_repo_paths() {
        // A repository URL in Cargo.toml tokenises into this, and feeding it to
        // `git check-ignore` aborts the entire check.
        assert!(!plausible_repo_path("//example.invalid/quality"));
        assert!(!plausible_repo_path("/etc/passwd"));
        assert!(!plausible_repo_path("*.rs"));
        assert!(plausible_repo_path("assets/generated.css"));
        assert!(plausible_repo_path("styles.css"));
    }

    #[test]
    fn dotted_directories_are_repo_paths_but_bare_dotfiles_are_not() {
        // The blind spot this closes: a leading dot disqualified everything, so
        // the check whose whole job is finding references to gitignored paths
        // could not see `.local/`, `.cache/` or `.idea/` — while reporting a
        // denominator in the hundreds.
        assert!(plausible_repo_path(".local/open-questions.md"));
        assert!(plausible_repo_path(".github/workflows/gate.yml"));
        // Deliberately still rejected: indistinguishable from an extension
        // named in prose, and a project ignoring `*.log` would see a false
        // positive on every page that mentions one.
        assert!(!plausible_repo_path(".rs"));
        assert!(!plausible_repo_path(".env"));
        assert!(!plausible_repo_path(".gitignore"));
        // Fatal to `git check-ignore`, which aborts on a path outside the
        // repository and abandons every candidate after it in the batch.
        assert!(!plausible_repo_path("../other-project/src"));
        assert!(!plausible_repo_path("../../etc/passwd"));
    }

    /// A path built from a variable belongs to whatever machine expands it.
    ///
    /// Not a hypothetical: `git check-ignore` reads `${HOME}` as a directory
    /// name like any other, so a shipped installer naming
    /// `${HOME}/.local/share/stageman` is reported as a tracked file naming an
    /// ignored path — a rule about fresh clones, applied to somebody else's
    /// home directory.
    #[test]
    fn a_variable_expansion_is_not_a_path_in_this_repository() {
        assert!(!plausible_repo_path("${HOME}/.local/share/stageman"));
        assert!(!plausible_repo_path("$HOME/.config/stageman/key"));
        assert!(!plausible_repo_path(
            "${{ github.workspace }}/dist/install.sh"
        ));
        assert!(
            plausible_repo_path("packaging/install.sh"),
            "the same file named relative to the repository is still checked"
        );
    }

    #[test]
    fn backticked_spans_are_extracted() {
        assert_eq!(backticked("use `foo` and `bar` here"), vec!["foo", "bar"]);
    }

    #[test]
    fn fenced_code_blocks_are_skipped() {
        let doc = "before `real`\n```rust\nlet `fake` = 1;\n```\nafter `also_real`";
        assert_eq!(backticked(doc), vec!["real", "also_real"]);
    }

    #[test]
    fn unclosed_backtick_does_not_capture_the_rest() {
        assert_eq!(
            backticked("a `open and then nothing"),
            vec!["open and then nothing"]
        );
    }

    #[test]
    fn paths_need_a_separator_and_no_spaces() {
        assert!(is_path_like("src/lib.rs"));
        assert!(
            !is_path_like("Cargo.toml"),
            "no separator: prose, not a path"
        );
        assert!(!is_path_like("and / or"));
        assert!(!is_path_like("https://example.invalid/x"));
    }

    #[test]
    fn url_routes_are_not_file_paths() {
        // A web app's docs backtick its route table. These are not files, and
        // resolving them as such buries the real findings in noise.
        assert!(!is_path_like("/games/:game_id"));
        assert!(!is_path_like("/login"));
        assert!(!is_path_like("/*"));
    }

    #[test]
    fn bare_directory_mentions_and_globs_are_skipped() {
        // `api/` in prose means "the api subdirectory of each module" — there is
        // no one path to resolve it against, and treating it as a file buries
        // the genuine findings under noise.
        assert!(!is_path_like("api/"), "single-segment shorthand");
        assert!(!is_path_like("mods/*/api"));
        assert!(!is_path_like("Path=/"), "a cookie attribute, not a path");
        assert!(
            is_path_like("bases/error/"),
            "a trailing slash on a real multi-segment path is still a claim"
        );
        // A sibling project's absolute path IS reportable: tracked docs must be
        // self-contained, so naming another checkout is a violation, not noise.
        assert!(is_path_like("../other-project/src"));
    }

    #[test]
    fn symbols_need_an_underscore_or_a_case_transition() {
        assert!(is_symbol_like("bits_one"));
        assert!(is_symbol_like("GameLoadState"));
        assert!(!is_symbol_like("Locked"), "an ordinary capitalised word");
        assert!(
            !is_symbol_like("Api"),
            "deliberately skipped: too short to be sure"
        );
        assert!(!is_symbol_like("src/lib.rs"));
        assert!(!is_symbol_like(""));
        assert!(!is_symbol_like("9lives"));
    }

    #[test]
    fn clamping_and_wrapping_are_flagged() {
        assert_eq!(
            escape_hatch("self.0.saturating_add(1)"),
            Some("saturating_add")
        );
        assert_eq!(escape_hatch("a.wrapping_sub(b)"), Some("wrapping_sub"));
    }

    #[test]
    fn checked_and_try_followed_by_a_default_are_flagged() {
        assert!(escape_hatch("i.checked_rem(cap).unwrap_or(0)").is_some());
        assert!(escape_hatch("u64::try_from(x).unwrap_or(0)").is_some());
    }

    #[test]
    fn a_justified_clamp_is_allowed() {
        assert_eq!(
            escape_hatch("let n = a.saturating_sub(b); // CLAMP-OK: zero is correct"),
            None
        );
    }

    #[test]
    fn honest_code_is_not_flagged() {
        assert_eq!(
            escape_hatch("let n = a.checked_add(b).ok_or(Error::Overflow)?;"),
            None
        );
        assert_eq!(escape_hatch("let n = maybe.unwrap_or(fallback);"), None);
    }

    #[test]
    fn files_whose_job_is_naming_ignored_paths_are_skipped() {
        assert!(!scannable_for_paths(".gitignore"));
        assert!(!scannable_for_paths("crates/foo/.gitignore"));
        assert!(!scannable_for_paths(".quality/generated-paths"));
        assert!(!scannable_for_paths(".quality/gate-reference.md"));
        assert!(scannable_for_paths("src/lib.rs"));
        assert!(scannable_for_paths("Cargo.toml"));
    }

    #[test]
    fn toml_values_need_an_equals_sign() {
        let manifest = "[package]\nrust-version = \"1.85\"\nname = \"x\"\n";
        assert_eq!(toml_string_value(manifest, "rust-version"), Some("1.85"));
        assert_eq!(toml_string_value(manifest, "edition"), None);
        assert_eq!(
            toml_string_value("rust-version-policy = \"strict\"\n", "rust-version"),
            None,
            "a longer key must not match"
        );
    }

    #[test]
    fn msrv_matches_the_pin_only_at_a_component_boundary() {
        assert!(msrv_is_pinned("1.95", "1.95.0"));
        assert!(msrv_is_pinned("1.95.0", "1.95.0"));
        assert!(!msrv_is_pinned("1.85", "1.95.0"));
        // The trap: a naive starts_with would call this a match and silently
        // skip a genuine MSRV verification.
        assert!(!msrv_is_pinned("1.9", "1.95.0"));
        assert!(!msrv_is_pinned("", "1.95.0"));
        assert!(!msrv_is_pinned("1.95", ""));
    }

    // The three below exist because mutation testing found them missing: a
    // leading underscore, the length boundary, and the empty-input guard each
    // had a surviving mutant in code that already had tests.

    #[test]
    fn a_leading_underscore_is_still_an_identifier() {
        assert!(is_symbol_like("_private"));
        assert!(!is_symbol_like("9lives"));
    }

    #[test]
    fn repo_paths_need_more_than_two_characters() {
        assert!(plausible_repo_path("a.b"), "three characters is enough");
        // `a.` satisfies every OTHER condition — contains a dot, no leading dot,
        // no slash prefix — so it isolates the length bound. An earlier version
        // of this test used `.b`, which is rejected for having a leading dot and
        // therefore never exercised the comparison at all. Mutation testing
        // caught that: the test passed for the wrong reason.
        assert!(!plausible_repo_path("a."), "two is not");
    }

    #[test]
    fn an_empty_msrv_never_matches_a_pin() {
        // Contrived, but it pins the guard: without it, an empty MSRV would
        // prefix-match any channel beginning with a dot.
        assert!(!msrv_is_pinned("", ".5"));
    }

    #[test]
    fn missing_symbols_reports_only_undefined_ones() {
        let docs = "Uses `checked_increment` and `never_written`.";
        let source = "pub fn checked_increment() {}";
        let scan = missing_symbols(docs, source, &[]);
        assert_eq!(scan.cited, 2);
        assert_eq!(scan.missing, vec!["never_written"]);
    }

    #[test]
    fn missing_symbols_matches_whole_words_and_honours_the_ignore_list() {
        let scan = missing_symbols("`bits_one`", "fn bits_one_extra() {}", &[]);
        assert_eq!(scan.missing, vec!["bits_one"], "must not match a prefix");

        let ignored = vec!["not_ours".to_owned()];
        let scan = missing_symbols("`not_ours`", "", &ignored);
        assert_eq!(scan.cited, 0);
        assert!(scan.missing.is_empty());
    }

    #[test]
    fn escape_hatch_hits_are_numbered_from_one() {
        let text = "fine\nlet n = a.saturating_add(b);\nfine\n";
        assert_eq!(escape_hatch_hits(text), vec![(2, "saturating_add")]);
    }

    #[test]
    fn a_file_that_only_names_the_patterns_opts_out() {
        let text = "// CLAMP-OK-FILE: a pattern table\nlet n = a.saturating_add(b);\n";
        assert!(escape_hatch_hits(text).is_empty());
    }

    #[test]
    fn unallowed_removes_exactly_the_allowlisted() {
        let hits = vec!["assets/x.css".to_owned(), "other/y.rs".to_owned()];
        let allowed = vec!["assets/x.css".to_owned()];
        assert_eq!(unallowed(hits, &allowed), vec!["other/y.rs"]);
    }

    #[test]
    fn allowlist_strips_comments_and_blanks() {
        let raw = "# a heading\n\njsQR      # a JavaScript library\nassets/styles.css\n";
        assert_eq!(allowlist(raw), vec!["jsQR", "assets/styles.css"]);
    }
}
