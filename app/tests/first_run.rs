//! The binary, run as a binary.
//!
//! `docs/decisions/0013-an-instance-is-configured-before-it-exists.md` records
//! that a first run wants a terminal, and that provisioning one unattended was
//! a known gap. Closing that gap is what makes these tests possible at all: a
//! flow only a human can drive is a flow nothing can check, and the first-run
//! path is the one place an instance is brought into existence.
//!
//! Nothing here needs a container runtime. It stands in for one with a utility
//! that always succeeds, which is enough because what is under test is the
//! configuration flow and the startup check's *verdict*, not the runtime.

#![expect(
    clippy::expect_used,
    reason = "test helpers in an integration-test crate are not seen as test code by \
              clippy's allow-expect-in-tests, which only covers #[test] functions and \
              #[cfg(test)] modules; a helper that failed here has nothing to report to"
)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// A key, as an operator would supply it: thirty-two bytes of base64.
const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// Stands in for the credential, and is asserted absent from what gets written.
const TOKEN: &str = "sk-ant-not-a-real-token-0123456789";

/// A utility that runs and succeeds whatever it is asked, standing in for a
/// working container runtime.
fn accepting() -> PathBuf {
    stand_in(["/usr/bin/true", "/bin/true"])
}

/// A utility that runs and fails, standing in for one that is installed but not
/// working — a client with no daemon behind it.
fn refusing() -> PathBuf {
    stand_in(["/usr/bin/false", "/bin/false"])
}

fn stand_in(candidates: [&str; 2]) -> PathBuf {
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.exists())
        .expect("a standard utility to stand in for a container runtime")
}

/// Runs the binary with exactly the environment given and nothing else.
///
/// `env_clear` is the point rather than tidiness: `docs/conventions.md` §3 says
/// what a process is handed is constructed and never inherited, and a test that
/// let the surrounding shell through could pass because of a variable nobody
/// meant to set.
fn run(snapshot: &PathBuf, variables: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stageman"));
    command.env_clear().env("STAGEMAN_STATE", snapshot);
    for (name, value) in variables {
        command.env(name, value);
    }
    command.output().expect("the binary runs")
}

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let snapshot = directory.path().join("state.json");
    (directory, snapshot)
}

#[test]
fn an_unattended_first_run_configures_an_instance() {
    let (_kept, snapshot) = scratch();
    let runtime = accepting();

    let finished = run(
        &snapshot,
        &[
            ("STAGEMAN_KEY", KEY),
            ("STAGEMAN_AGENT_TOKEN", TOKEN),
            ("STAGEMAN_CONTAINER_RUNTIME", &runtime.to_string_lossy()),
        ],
    );

    assert!(
        finished.status.success(),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
    assert!(snapshot.exists(), "it should have written an instance");
}

/// The bar in `docs/conventions.md` §4 applied to the file that actually gets
/// written, rather than to a formatted struct: a credential that survives a
/// round trip in the clear is the same bug whichever way it escaped.
#[test]
fn a_first_run_never_writes_the_credential_it_was_given() {
    let (_kept, snapshot) = scratch();
    let runtime = accepting();

    run(
        &snapshot,
        &[
            ("STAGEMAN_KEY", KEY),
            ("STAGEMAN_AGENT_TOKEN", TOKEN),
            ("STAGEMAN_CONTAINER_RUNTIME", &runtime.to_string_lossy()),
        ],
    );

    let written = std::fs::read_to_string(&snapshot).expect("it wrote an instance");
    assert!(!written.contains(TOKEN), "the credential is in the file");
    assert!(
        written.contains("ciphertext"),
        "it should be there, sealed: {written}"
    );
}

/// The second start is the one an operator does every day, and it must want
/// nothing but the key and the file.
#[test]
fn a_second_start_asks_for_nothing_and_needs_no_further_variables() {
    let (_kept, snapshot) = scratch();
    let runtime = accepting();
    let first = run(
        &snapshot,
        &[
            ("STAGEMAN_KEY", KEY),
            ("STAGEMAN_AGENT_TOKEN", TOKEN),
            ("STAGEMAN_CONTAINER_RUNTIME", &runtime.to_string_lossy()),
        ],
    );
    assert!(first.status.success());

    let again = run(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
}

#[test]
fn a_runtime_that_is_installed_but_not_working_stops_the_start() {
    let (_kept, snapshot) = scratch();
    let runtime = refusing();

    let finished = run(
        &snapshot,
        &[
            ("STAGEMAN_KEY", KEY),
            ("STAGEMAN_AGENT_TOKEN", TOKEN),
            ("STAGEMAN_CONTAINER_RUNTIME", &runtime.to_string_lossy()),
        ],
    );

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("container runtime"), "{said}");
    assert!(
        said.contains(&*runtime.to_string_lossy()),
        "it should name the path it tried: {said}"
    );
}

#[test]
fn a_start_without_a_key_says_which_variable_is_missing() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[]);

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("STAGEMAN_KEY"), "{said}");
}

/// A wrong key must be refused rather than half-read, and the message must not
/// repeat what it was given.
#[test]
fn a_key_that_is_not_key_material_is_refused_without_echoing_it() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[("STAGEMAN_KEY", "far-too-short")]);

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("STAGEMAN_KEY"), "{said}");
    assert!(!said.contains("far-too-short"), "it echoed the key: {said}");
}
