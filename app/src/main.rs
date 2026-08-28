//! Running this is what running stageman means.
//!
//! It asks nothing. An instance starts with no agents, no projects and no
//! container runtime, and everything is configured through the dashboard —
//! `docs/decisions/0021-an-instance-starts-empty.md`. What used to be a
//! first-run flow, with a terminal prompt and an environment fallback for the
//! machines that have no terminal, is gone: there is nothing left to ask.
//!
//! Two things still come from the environment, and both are about *where this
//! instance is* rather than what it does: the key its file is encrypted under,
//! and where that file lives. Neither can be stored in the file they describe.
//!
//! **There is no dashboard yet, so this loads, checks and stops.** That is the
//! honest shape of it rather than a placeholder for a server.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use stageman::{LoadError, Store};
use stageman_agent::{AgentError, ContainerRuntime};
use stageman_core::{Key, KeyError, State};

/// The variable the snapshot's encryption key arrives in, as base64.
///
/// From the environment because keeping it beside the file it protects would
/// defeat the encryption — which is the whole of why the file is portable and
/// useless on its own.
const KEY_VARIABLE: &str = "STAGEMAN_KEY";

/// The variable naming the file the instance is kept in.
///
/// Required rather than defaulted to something in the working directory. A
/// daemon started by a service manager has a working directory nobody chose,
/// so a default there would be the same trap as searching `PATH` — it works
/// perfectly when tested by hand and picks a different file in service.
const STATE_VARIABLE: &str = "STAGEMAN_STATE";

/// How much is reported, when the environment does not say.
const DEFAULT_VERBOSITY: &str = "warn";

/// The variable that overrides how much is reported.
const VERBOSITY_VARIABLE: &str = "STAGEMAN_LOG";

/// An instance could not be started.
#[derive(Debug, thiserror::Error)]
enum StartupError {
    /// A variable this needs is not set.
    #[error("{0} is not set")]
    Missing(&'static str),
    /// The key is set but is not key material.
    #[error("{KEY_VARIABLE} is not usable")]
    Key(#[source] KeyError),
    /// The instance could not be opened or created.
    #[error("the instance at {path} could not be opened")]
    Instance {
        /// Where it was looked for.
        path: PathBuf,
        /// Why it could not be opened.
        #[source]
        source: LoadError,
    },
    /// The configured container runtime is missing or not working.
    #[error("the container runtime is not usable")]
    Runtime(#[source] AgentError),
    /// The runtime would not say what containers it has.
    #[error("what the last run left behind could not be established")]
    Sweep(#[source] stageman_job::JobError),
}

fn main() -> ExitCode {
    // Standard error, which is a real answer for a process somebody started
    // and is watching, and a placeholder for the daemon this becomes — see
    // `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md`.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env(VERBOSITY_VARIABLE)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_VERBOSITY)),
        )
        .with_writer(io::stderr)
        .init();

    match tokio::runtime::Runtime::new() {
        Ok(runtime) => match runtime.block_on(start()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(failure) => {
                report(&failure);
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            eprintln!("stageman: no async runtime: {failure}");
            ExitCode::FAILURE
        }
    }
}

/// Prints a failure and everything underneath it.
///
/// Deliberately not through `tracing`, and the distinction is worth keeping:
/// this is the program's last word to whoever ran it, not a record of
/// something that happened. Routing it through a level would let
/// `STAGEMAN_LOG` silence the reason the process exited, which is the one
/// message that must never be filterable.
///
/// The chain rather than the top line alone: every error here wraps a more
/// specific one, and "the instance could not be opened" without the reason
/// underneath is the shape of message that sends somebody to read the source.
fn report(failure: &StartupError) {
    eprintln!("stageman: {failure}");
    let mut cause: Option<&dyn std::error::Error> = std::error::Error::source(failure);
    while let Some(reason) = cause {
        eprintln!("  caused by: {reason}");
        cause = reason.source();
    }
}

async fn start() -> Result<(), StartupError> {
    let key = Key::from_base64(&required(KEY_VARIABLE)?).map_err(StartupError::Key)?;
    let path = PathBuf::from(required(STATE_VARIABLE)?);

    let existing =
        Store::load(path.clone(), key.clone()).map_err(|source| StartupError::Instance {
            path: path.clone(),
            source,
        })?;

    // `Ok(None)` is a first run rather than a failure, and a first run now
    // produces an instance with nothing in it rather than asking questions.
    let store = if let Some(store) = existing {
        store
    } else {
        Store::create(path.clone(), key, State::default())
            .map_err(|source| StartupError::Instance { path, source })?
    };

    // Verified when there is one, and there is nothing to verify before
    // anybody has configured anything. `docs/conventions.md` §3 asks that
    // whatever makes an instance unusable fails at startup; an instance with
    // no projects is not unusable, it is empty. What must not happen is a
    // project created against a runtime nothing has checked, which is a
    // condition of creating one rather than of starting.
    let configured_runtime = store
        .read()
        .container_runtime
        .clone()
        .map(ContainerRuntime::new);
    let swept = if let Some(runtime) = &configured_runtime {
        runtime.verify().await.map_err(StartupError::Runtime)?;
        stageman::reconcile(&store, runtime)
            .await
            .map_err(StartupError::Sweep)?
    } else {
        stageman::Swept::default()
    };

    println!();
    println!("stageman is running.");
    println!(
        "  runtime    {}",
        configured_runtime.as_ref().map_or_else(
            || "not configured yet".to_owned(),
            |runtime| runtime.path().display().to_string()
        )
    );
    println!("  agents     {}", store.read().agents.len());
    println!("  projects   {}", store.read().projects.len());
    println!(
        "  swept      {} resumed, {} failed, {} stranded",
        swept.resumed, swept.failed, swept.stranded
    );
    println!(
        "  left alone {} unidentified, {} naming a forgotten job",
        swept.unidentified, swept.forgotten
    );
    println!();
    println!("There is no dashboard yet, so there is nothing further to run.");
    Ok(())
}

/// The value of a required variable.
fn required(name: &'static str) -> Result<String, StartupError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(StartupError::Missing(name))
}
