//! Every recorded flow, replayed against a fresh instance.
//!
//! One file, one test, named for the flow it pins. A replay feeds the file's
//! events to an instance constructed from the file's own seed and
//! environment, and compares what it asks for and what it holds after every
//! turn — so nothing here behaves and nothing interprets: a difference is a
//! change of behaviour, and the diff of the file says which. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Its own target, without the usual harness, because the tests are files
//! rather than functions: `libtest-mimic` makes one of each at run time so
//! that a failure names the flow rather than this file.
//!
//! Rewriting one is `just record`, and the diff is the review.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use libtest_mimic::{Arguments, Failed, Trial};
use stageman_instance::Instance;
use stageman_vocabulary::scenario::{Scenario, replay};

/// Where the recorded flows live, flat.
const REPLAYS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/replays");

fn main() -> ExitCode {
    let arguments = Arguments::from_args();
    let found = match recorded() {
        Ok(found) => found,
        Err(why) => {
            eprintln!("the recorded flows could not be read from {REPLAYS}: {why}");
            return ExitCode::FAILURE;
        }
    };
    if found.is_empty() {
        eprintln!("no recorded flows in {REPLAYS}, so nothing was replayed");
        return ExitCode::FAILURE;
    }
    let trials = found
        .into_iter()
        .map(|path| {
            let name = path.file_stem().map_or_else(
                || path.display().to_string(),
                |stem| stem.to_string_lossy().into_owned(),
            );
            Trial::test(name, move || replayed(&path))
        })
        .collect();
    libtest_mimic::run(&arguments, trials).exit_code()
}

/// Every file to replay, in a stable order.
fn recorded() -> Result<Vec<PathBuf>, std::io::Error> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(REPLAYS)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|kind| kind == "json"))
        .collect();
    found.sort();
    Ok(found)
}

/// Replays one file, and says where it first disagrees.
fn replayed(path: &Path) -> Result<(), Failed> {
    let text = std::fs::read_to_string(path)?;
    let scenario: Scenario<Instance> = serde_json::from_str(&text)?;
    replay::<Instance>(&scenario)?;
    Ok(())
}
