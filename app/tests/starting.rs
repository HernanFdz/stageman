//! The binary, run as a binary.
//!
//! There is no first run to test any more. An instance starts with nothing —
//! no agents, no projects, no container runtime — and asks nothing, per
//! `docs/decisions/0021-an-instance-starts-empty.md`. What is left to check is
//! that starting from nothing works, that starting again changes nothing, and
//! that the two things which *do* come from the environment fail clearly when
//! they are wrong.

#![expect(
    clippy::expect_used,
    reason = "test helpers in an integration-test crate are not seen as test code by \
              clippy's allow-expect-in-tests, which only covers #[test] functions and \
              #[cfg(test)] modules; a helper that failed here has nothing to report to"
)]

use std::path::PathBuf;
use std::process::{Command, Output};

use stageman::Store;
use stageman_core::{Key, State};

/// A key, as an operator would supply it: thirty-two bytes of base64.
const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// A utility that runs and fails, standing in for a container runtime that is
/// installed and not working — a client with no daemon behind it.
fn refusing() -> PathBuf {
    ["/usr/bin/false", "/bin/false"]
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.exists())
        .expect("a standard utility that always refuses")
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

fn key() -> Key {
    Key::from_base64(KEY).expect("the test key is well formed")
}

#[test]
fn an_instance_starts_with_nothing_configured() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    assert!(
        finished.status.success(),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
    assert!(snapshot.exists(), "it should have written an instance");
}

/// What removing the first-run flow actually bought: nothing is asked, so
/// nothing has to be answered, so a machine with no terminal stops being a
/// special case rather than being handled as one.
#[test]
fn starting_needs_no_answers_and_no_terminal() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let said = String::from_utf8_lossy(&finished.stdout);

    assert!(finished.status.success());
    assert!(said.contains("not configured yet"), "{said}");
    assert!(said.contains("agents     0"), "{said}");
    assert!(said.contains("projects   0"), "{said}");
}

#[test]
fn starting_again_changes_nothing() {
    let (_kept, snapshot) = scratch();
    assert!(run(&snapshot, &[("STAGEMAN_KEY", KEY)]).status.success());
    let first = std::fs::read_to_string(&snapshot).expect("it wrote an instance");

    assert!(run(&snapshot, &[("STAGEMAN_KEY", KEY)]).status.success());
    let again = std::fs::read_to_string(&snapshot).expect("it is still there");

    // Byte equality is deliberately not a claim about snapshots in general:
    // sealing consumes a fresh nonce per credential per write, so two snapshots
    // of the same configured state differ. With nothing configured there is
    // nothing to seal, which is what makes the comparison meaningful here and
    // misleading anywhere else.
    assert_eq!(first, again);
}

/// A runtime is verified only once something has configured one, and then it
/// must stop the start. The instance is built through the library because the
/// binary no longer has any way to configure one.
#[test]
fn a_configured_runtime_that_is_not_working_stops_the_start() {
    let (_kept, snapshot) = scratch();
    let state = State {
        container_runtime: Some(refusing()),
        ..State::default()
    };
    drop(Store::create(snapshot.clone(), key(), state).expect("it can write"));

    let finished = run(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("container runtime"), "{said}");
    assert!(
        said.contains(&*refusing().to_string_lossy()),
        "it should name the path it tried: {said}"
    );
}

#[test]
fn a_start_without_a_key_says_which_variable_is_missing() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[]);

    assert!(!finished.status.success());
    assert!(
        String::from_utf8_lossy(&finished.stderr).contains("STAGEMAN_KEY"),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
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
