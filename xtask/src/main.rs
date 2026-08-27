//! Repo automation for the quality gate.
//!
//! **Checks** count problems: `ignored-refs`, `doc-paths`, `doc-symbols`,
//! `escape-hatches`. `drift` runs all four.
//! **Actions** do a thing that may fail: `msrv`, `mutants`.
//!
//! All of it began as inline shell in the justfile. It moved here because
//! shell's error semantics make silent failure the default: the same bug shipped
//! three separate times — a `grep` exiting non-zero on no-match, which under
//! `set -e` killed the recipe before it could print its own error. Every
//! instance failed quietly or misleadingly, the worst behaviour a gate can have.
//!
//! The rule those bugs taught is encoded in [`git_lenient`]: a command that
//! legitimately exits non-zero when it finds nothing must say so in its type,
//! not in a comment.
//!
//! What stayed in the justfile stayed on principle, not by omission — a recipe
//! moves here when it stops being readable by inspection, and `tools` (a loop
//! over three names) has not.

mod scan;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use scan::{
    allowlist, backticked, escape_hatch_hits, gate_scratch, is_path_like, matrix_passes,
    missing_symbols, msrv_is_pinned, plausible_repo_path, scannable_for_paths, toml_string_value,
    unallowed,
};

/// Name plus entry point for one check. Aliased because `clippy::type_complexity`
/// is denied and a bare `&[(&str, fn() -> Report)]` trips it.
type Check = (&'static str, fn() -> Report);

/// What a check found. `checked` is reported even when zero, because a check
/// that examined nothing must say so — a silent pass on an empty denominator
/// reads as coverage.
struct Report {
    unit: &'static str,
    checked: usize,
    problems: Vec<String>,
    advice: &'static str,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let requested: Vec<&str> = args.iter().map(String::as_str).collect();

    let checks: &[Check] = &[
        ("ignored-refs", check_ignored_refs),
        ("doc-paths", check_doc_paths),
        ("doc-symbols", check_doc_symbols),
        ("escape-hatches", check_escape_hatches),
    ];

    // Actions rather than checks: they do a thing and may fail, instead of
    // counting problems. They live here rather than as shell in the justfile
    // because both parse files and resolve refs, and both had already shipped
    // the shell failure mode this crate exists to eliminate.
    match requested.first().copied() {
        Some("msrv") => return report(run_msrv(requested.get(1).copied().unwrap_or_default())),
        Some("mutants") => return report(run_mutants(requested.get(1).copied())),
        _ => {}
    }

    // Every check reads the tracked file list from git, and the invariant is
    // "git knows about files" — NOT "a repository exists". `git ls-files` reads
    // the index, so a freshly initialised repo with nothing staged is as empty
    // as no repo at all, and all four checks would report `0 checked` and pass.
    // Reporting a denominator is no defence when the denominator is a lie.
    if let Err(reason) = git_knows_about_files() {
        eprintln!(
            "error: {reason}\n  \
             Every drift check would report `0 checked` and pass, which is\n  \
             indistinguishable from a project that genuinely has nothing to check."
        );
        return ExitCode::FAILURE;
    }

    let selected: Vec<&Check> = match requested.first() {
        None | Some(&"drift") => checks.iter().collect(),
        Some(name) => checks.iter().filter(|(n, _)| n == name).collect(),
    };

    if selected.is_empty() {
        eprintln!(
            "unknown command. checks: drift (all), {}. actions: msrv, mutants",
            names(checks)
        );
        return ExitCode::FAILURE;
    }

    let mut failed = false;
    for (name, run) in selected {
        let report = run();
        println!("{name}: {} {} checked", report.checked, report.unit);
        if report.problems.is_empty() {
            continue;
        }
        failed = true;
        for problem in &report.problems {
            eprintln!("  {problem}");
        }
        eprintln!("{}", report.advice);
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn report(outcome: Result<String, String>) -> ExitCode {
    match outcome {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn names(checks: &[Check]) -> String {
    let mut out = String::new();
    for (name, _) in checks {
        if !out.is_empty() {
            out.push_str(", ");
        }
        let _ = write!(out, "{name}");
    }
    out
}

// ---------------------------------------------------------------- git access

/// Runs git and returns stdout, treating a non-zero exit as "found nothing"
/// rather than as an error.
///
/// `git check-ignore` exits 1 when nothing matches, which is the *normal* case.
/// The shell version needed a `|| true` for exactly this, and omitting it is
/// what made three separate recipes die before reporting anything.
fn git_lenient(args: &[&str]) -> String {
    Command::new("git").args(args).output().map_or_else(
        |_| String::new(),
        |out| String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Whether git can supply a file list, and why not if it cannot.
///
/// [`git_lenient`] deliberately turns a non-zero exit into "found nothing",
/// which is right for `check-ignore` and wrong for `ls-files`: the first
/// genuinely means no matches, the second can also mean no repository — or a
/// repository with an empty index. This separates those cases so the caller can
/// refuse to run rather than report a denominator it has no basis for.
fn git_knows_about_files() -> Result<(), String> {
    let in_repo = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success());
    if !in_repo {
        return Err(
            "not a git repository, so the tracked file list cannot be read.\n  \
                    Run `git init`."
                .to_owned(),
        );
    }
    if tracked_files().is_empty() {
        return Err(
            "this repository has no tracked files — `git ls-files` reads the\n  \
                    index, so nothing staged means nothing to check. Run `git add -A`."
                .to_owned(),
        );
    }
    Ok(())
}

fn tracked_files() -> Vec<PathBuf> {
    git_lenient(&["ls-files"])
        .lines()
        .map(PathBuf::from)
        .collect()
}

/// Which candidates git considers ignored, or why the answer is unusable.
///
/// `check-ignore` exits 0 when something matched and 1 when nothing did — both
/// ordinary, and conflating them is what `git_lenient` exists to prevent. Any
/// other status is a FATAL abort partway through the stream, and under
/// `--stdin` it discards the whole rest of the batch: every candidate after the
/// offending one goes unchecked while the command still prints a tidy, short
/// answer. That is indistinguishable from a clean result, which is how this
/// check quietly became a no-op once already.
///
/// So the status is inspected rather than swallowed. A batch that aborted is an
/// error, not an empty result.
fn ignored_among(candidates: &BTreeSet<String>) -> Result<Vec<String>, String> {
    let mut child = Command::new("git")
        .args(["check-ignore", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("running `git check-ignore`: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        for candidate in candidates {
            let _ = writeln!(stdin, "{candidate}");
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|error| format!("reading from `git check-ignore`: {error}"))?;
    match out.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_owned)
            .collect()),
        other => Err(format!(
            "`git check-ignore` aborted (exit {other:?}), so everything after the\n  \
             offending path went unchecked and this result cannot be trusted:\n  {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

// ------------------------------------------------------------------ locating

fn docs_dir() -> Option<PathBuf> {
    ["docs", "doc"]
        .into_iter()
        .map(PathBuf::from)
        .find(|dir| dir.is_dir())
}

/// Every plausible root a doc might write paths relative to.
///
/// Docs cite module paths relative to a crate's `src/`, not to the repo root —
/// `bases/realtime/room.rs` means `src/bases/realtime/room.rs`. Resolving only
/// from the repo root makes most such citations look missing.
fn source_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(".")];
    for parent in [".", "crates"] {
        let Ok(entries) = fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let src = entry.path().join("src");
            if src.is_dir() {
                roots.push(src);
            }
        }
    }
    if Path::new("src").is_dir() {
        roots.push(PathBuf::from("src"));
    }
    roots
}

fn read_docs() -> Vec<(PathBuf, String)> {
    let Some(dir) = docs_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_markdown(&dir, &mut out);
    out
}

fn collect_markdown(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md")
            && let Ok(text) = fs::read_to_string(&path)
        {
            out.push((path, text));
        }
    }
}

fn read_allowlist(name: &str) -> Vec<String> {
    fs::read_to_string(Path::new(".quality").join(name)).map_or_else(
        |_| Vec::new(),
        |text| allowlist(&text).into_iter().map(str::to_owned).collect(),
    )
}

// ------------------------------------------------------------------- actions

/// A git rev, or `None` if it does not resolve.
///
/// `git rev-parse` exits non-zero for an unknown ref, which is an ordinary
/// answer here rather than an error — the shape [`git_lenient`] exists for.
fn git_rev(spec: &str) -> Option<String> {
    let resolved = git_lenient(&["rev-parse", "--verify", "-q", spec]);
    let trimmed = resolved.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// The workspace compiles against the MSRV it promises.
fn run_msrv(matrix: &str) -> Result<String, String> {
    let manifest =
        fs::read_to_string("Cargo.toml").map_err(|e| format!("reading Cargo.toml: {e}"))?;
    let Some(msrv) = toml_string_value(&manifest, "rust-version") else {
        return Ok("msrv: no rust-version declared — nothing to verify.\n  \
             Correct for an application: rust-toolchain.toml pins the exact compiler,\n  \
             and nobody consumes your crate as a dependency."
            .to_owned());
    };
    let toolchain = fs::read_to_string("rust-toolchain.toml").unwrap_or_default();
    let channel = toml_string_value(&toolchain, "channel").unwrap_or_default();

    if msrv_is_pinned(msrv, channel) {
        return Ok(format!(
            "msrv: rust-version ({msrv}) is the pinned toolchain ({channel}) — \
             already verified by `lint`."
        ));
    }

    // A genuine compatibility promise: consumers build with THEIR toolchain and
    // never see rust-toolchain.toml, so this must compile against a second
    // compiler. Installing it is best-effort; if it is already present, or
    // rustup is not in use, the build below is what actually reports.
    println!("msrv: verifying {msrv} (pinned toolchain is {channel})");
    let _ = Command::new("rustup")
        .args(["toolchain", "install", msrv, "--profile", "minimal"])
        .output();
    // `--exclude xtask`: gate infrastructure is not project content, the same
    // exemption `.quality/` and `gate_scratch` already carry. This crate is
    // never published, nobody consumes it, and it is only ever compiled by the
    // pinned toolchain — so it makes no compatibility promise to verify.
    //
    // Without this the scaffolder breaks any project whose declared MSRV is
    // older than the features xtask happens to use: enrolling xtask puts it in
    // the workspace, `--workspace` then compiles it against the ADOPTER's
    // floor, and it fails on code they did not write. Found on a real adopter
    // declaring 1.85, where xtask's let-chains need 1.88.
    //
    // Once per matrix pass, because an MSRV promise is per configuration: an API
    // stabilised in a later release can sit behind a `cfg` the host pass never
    // compiles, and the promise would be unverified for exactly the platform
    // that needs it.
    let passes = matrix_passes(matrix);
    if passes.is_empty() {
        return Err(
            "msrv: `check_matrix` is empty, so nothing was verified.\n  \
             An empty matrix is not a pass — declare at least one configuration."
                .to_owned(),
        );
    }
    for pass in &passes {
        let label = if pass.is_empty() {
            "host, default features".to_owned()
        } else {
            pass.join(" ")
        };
        println!("msrv: {label}");
        let status = Command::new("cargo")
            .arg(format!("+{msrv}"))
            .args([
                "check",
                "--workspace",
                "--exclude",
                "xtask",
                "--all-targets",
                "--locked",
            ])
            .args(pass)
            .status()
            .map_err(|e| format!("running cargo +{msrv}: {e}"))?;
        if !status.success() {
            return Err(format!(
                "msrv: does not compile against the declared rust-version ({msrv}) for\n  \
                 `{label}`. Either raise rust-version deliberately, or stop using the\n  \
                 newer API on that configuration."
            ));
        }
    }
    Ok(format!(
        "msrv: verified against {msrv} for {} configuration(s)",
        passes.len()
    ))
}

/// Mutation-test what changed, so the backlog does not block but new untested
/// logic does.
fn run_mutants(base_arg: Option<&str>) -> Result<String, String> {
    let base = base_arg
        .filter(|spec| !spec.is_empty())
        .map(str::to_owned)
        .or_else(|| git_rev("origin/HEAD"))
        .or_else(|| git_rev("HEAD~1"));

    let Some(base) = base else {
        println!("mutants: no base commit to diff against — running the full sweep");
        return finish(
            Command::new("cargo")
                .args(["mutants", "--workspace", "--no-shuffle"])
                .status(),
            "mutants: no surviving mutants",
        );
    };

    let diff = git_lenient(&["diff", &base]);
    if diff.trim().is_empty() {
        return Ok(format!("mutants: no changes since {base}"));
    }
    let path = std::env::temp_dir().join(format!("xtask-mutants-{}.diff", std::process::id()));
    fs::write(&path, &diff).map_err(|e| format!("writing {}: {e}", path.display()))?;
    println!("mutants: mutating only what changed since {base}");
    let status = Command::new("cargo")
        .args(["mutants", "--workspace", "--no-shuffle", "--in-diff"])
        .arg(&path)
        .status();
    let _ = fs::remove_file(&path);
    finish(status, "mutants: every mutant in the diff was caught")
}

fn finish(status: std::io::Result<std::process::ExitStatus>, ok: &str) -> Result<String, String> {
    match status {
        Ok(status) if status.success() => Ok(ok.to_owned()),
        Ok(_) => Err(
            "mutants: surviving mutants — the tests would not notice this code \
                      being wrong. Write a test that fails under the mutation, or mark a \
                      genuinely equivalent mutant with #[mutants::skip] and say why."
                .to_owned(),
        ),
        Err(error) => Err(format!("running cargo mutants: {error}")),
    }
}

// -------------------------------------------------------------------- checks

/// A fresh clone must build, so no tracked file may name a gitignored path.
fn check_ignored_refs() -> Report {
    let mut candidates = BTreeSet::new();
    for path in tracked_files() {
        if !scannable_for_paths(&path.to_string_lossy()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for token in text.split([
            '"', '`', '(', ')', '\'', ' ', '\t', '\n', ',', ';', ':', '=',
        ]) {
            let token = token.trim();
            if plausible_repo_path(token) {
                candidates.insert(token.to_owned());
            }
        }
    }
    let allowed = read_allowlist("generated-paths");
    let problems: Vec<String> = match ignored_among(&candidates) {
        Ok(hits) => unallowed(hits, &allowed)
            .into_iter()
            .filter(|hit| !gate_scratch(hit))
            .map(|hit| format!("{hit} is gitignored but named by a tracked file"))
            .collect(),
        // A batch that aborted is reported as a problem rather than as no
        // problems, because those two look identical from the outside.
        Err(message) => vec![message],
    };

    Report {
        unit: "path token(s)",
        checked: candidates.len(),
        problems,
        advice: "A fresh clone will not build. Either track the file, or add it to\n\
                 .quality/generated-paths with the command that generates it — and make\n\
                 sure that command runs as part of `just check`.",
    }
}

/// Every repo-relative path cited in the docs must exist.
fn check_doc_paths() -> Report {
    let roots = source_roots();
    let mut cited = BTreeSet::new();
    for (_, text) in read_docs() {
        for token in backticked(&text) {
            if is_path_like(token) {
                cited.insert(token.trim_end_matches('/').to_owned());
            }
        }
    }
    let problems: Vec<String> = cited
        .iter()
        .filter(|path| !roots.iter().any(|root| root.join(path).exists()))
        .map(|path| format!("{path} — cited in the docs, exists under no source root"))
        .collect();

    Report {
        unit: "path(s) cited",
        checked: cited.len(),
        problems,
        advice: "Either the path moved and the doc is stale, or the doc describes code\n\
                 that was never written. Fix whichever one is wrong.",
    }
}

/// Every Rust symbol named in the docs must exist somewhere in the source.
fn check_doc_symbols() -> Report {
    let ignored = read_allowlist("doc-symbols-ignore");
    let mut docs = String::new();
    for (_, text) in read_docs() {
        docs.push_str(&text);
        docs.push('\n');
    }
    let mut source = String::new();
    for path in tracked_files() {
        if path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(text) = fs::read_to_string(&path)
        {
            source.push_str(&text);
            source.push('\n');
        }
    }
    // The deciding is in `scan`, where it is unit-tested; this function only
    // does the reading. That split is why mutation testing can reach it.
    let scan = missing_symbols(&docs, &source, &ignored);

    Report {
        unit: "symbol(s) cited",
        checked: scan.cited,
        problems: scan
            .missing
            .into_iter()
            .map(|symbol| format!("{symbol} — named in the docs, defined nowhere in the source"))
            .collect(),
        advice: "Either the symbol was renamed and the doc is stale, or the doc specifies\n\
                 something never implemented. If the name is not yours — a std method, a\n\
                 JS library — prefer rewording so it is not backticked; a backticked\n\
                 identifier is a claim about THIS crate's source.",
    }
}

/// Panic lints must not be silenced by producing a wrong value.
fn check_escape_hatches() -> Report {
    let sources: Vec<PathBuf> = tracked_files()
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();

    let mut problems = Vec::new();
    for path in &sources {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (number, pattern) in escape_hatch_hits(&text) {
            problems.push(format!(
                "{}:{number} uses {pattern} — silently produces a wrong value",
                path.display()
            ));
        }
    }

    Report {
        unit: "Rust file(s)",
        checked: sources.len(),
        problems,
        advice: "Use checked_* and propagate with `?`. If the clamp IS the intended\n\
                 semantics, annotate the line: // CLAMP-OK: <why this default is correct>",
    }
}
