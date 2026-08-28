//! Running this is what running stageman means.
//!
//! It does three things before anything else can happen, in this order: finds
//! the key and the snapshot, configures the instance if there is not one yet,
//! and proves the container runtime works. All three are the category
//! `docs/conventions.md` §3 calls fail-at-startup — an instance missing any of
//! them cannot do anything at all, and the worst moment to discover that is
//! three in the morning on the first signal that mattered.
//!
//! **There is no dashboard yet, so this configures and checks and stops.** That
//! is the honest shape of it today rather than a placeholder for a server: the
//! first-run flow in
//! `docs/decisions/0013-an-instance-is-configured-before-it-exists.md` and the
//! startup checks are real work with nothing depending on them, and running
//! them is how an operator finds out their machine is ready.

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use stageman::{LoadError, Store};
use stageman_agent::{AgentError, ContainerRuntime};
use stageman_core::{Agent, AgentConfig, Key, KeyError, Secret, State};

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
/// perfectly when you test it by hand and picks a different file in service.
const STATE_VARIABLE: &str = "STAGEMAN_STATE";

/// The variable a first run can take the agent's credential from.
///
/// Present so an instance can be provisioned with nobody watching, which
/// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md` recorded
/// as a known gap and `docs/vision.md` §3 needs eventually — a machine nobody
/// sits at cannot answer a prompt. Set it and nothing is asked; leave it and
/// the terminal is.
///
/// There is deliberately no variable for *which* agent. The set is closed and
/// holds one, so it would be a question with a single possible answer; it
/// becomes worth adding at the same moment a second agent does.
const TOKEN_VARIABLE: &str = "STAGEMAN_AGENT_TOKEN";

/// The variable a first run can take the container runtime's path from.
///
/// The unattended counterpart to the suggestion below, and the same value
/// either way — once given it is recorded in the instance, per
/// `docs/decisions/0017-the-runtimes-path-is-recorded-in-the-instance.md`. That
/// it can arrive through the environment while a *running* instance never reads
/// it there is the distinction that matters: this is answered once, at
/// configuration time, not resolved afresh on every start.
const RUNTIME_VARIABLE: &str = "STAGEMAN_CONTAINER_RUNTIME";

/// Where a container runtime is commonly installed.
///
/// Only ever a *suggestion* offered at first run, and deliberately not a
/// search: `docs/conventions.md` §3 forbids resolving the runtime through the
/// environment, and this list is fixed and absolute where `PATH` is neither.
/// The operator confirms or overrides whatever is proposed, so the recorded
/// value is theirs — see
/// `docs/decisions/0017-the-runtimes-path-is-recorded-in-the-instance.md`.
const RUNTIME_SUGGESTIONS: [&str; 5] = [
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/usr/bin/docker",
    "/usr/local/bin/podman",
    "/usr/bin/podman",
];

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
    /// The terminal could not be read from or written to.
    ///
    /// Reached on a first run with nothing attached — which
    /// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`
    /// records as a known gap rather than a surprise, since
    /// `docs/vision.md` §3 contemplates a machine nobody sits at.
    #[error("a first run needs a terminal to ask its questions on")]
    Console(#[source] io::Error),
    /// Nothing was entered where something was required.
    #[error("{0} is needed and nothing was entered")]
    Empty(&'static str),
    /// The container runtime is missing or not working.
    #[error("the container runtime is not usable")]
    Runtime(#[source] AgentError),
    /// The runtime would not say what containers it has.
    ///
    /// Fatal at startup rather than merely reported: an instance that cannot
    /// see its own containers cannot tell a job worth resuming from one that
    /// is gone, and would quietly start a second container for work already
    /// running.
    #[error("what the last run left behind could not be established")]
    Sweep(#[source] stageman_job::JobError),
}

/// How much is reported, when the environment does not say.
///
/// A default rather than a policy. `warn` is what an operator who set nothing
/// should see: things they can act on, and not a commentary on things going
/// right.
const DEFAULT_VERBOSITY: &str = "warn";

/// The variable that overrides how much is reported.
const VERBOSITY_VARIABLE: &str = "STAGEMAN_LOG";

fn main() -> ExitCode {
    // Standard error, which is a real answer for a process somebody started
    // and is watching, and a placeholder for the daemon this becomes — see
    // `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md`, which
    // takes the first and defers the second on purpose.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env(VERBOSITY_VARIABLE)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_VERBOSITY)),
        )
        .with_writer(std::io::stderr)
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

    // `Ok(None)` is a first run rather than a failure, which is the whole
    // reason `Store::load` distinguishes the two.
    let store = if let Some(store) = existing {
        store
    } else {
        let state = configure()?;
        Store::create(path.clone(), key, state)
            .map_err(|source| StartupError::Instance { path, source })?
    };

    let runtime = ContainerRuntime::new(store.read().container_runtime.clone());
    runtime.verify().await.map_err(StartupError::Runtime)?;

    // Before anything else could act on the instance: what the last run left
    // is reconciled with what this one believes, per
    // `docs/decisions/0015-a-job-survives-the-daemon-dying.md`.
    let swept = stageman::reconcile(&store, &runtime)
        .await
        .map_err(StartupError::Sweep)?;

    println!();
    println!("stageman is configured and its container runtime answers.");
    println!("  agent      {:?}", store.read().orchestrator_agent);
    println!("  runtime    {}", runtime.path().display());
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

/// Asks, in the terminal, everything a new instance needs to exist.
///
/// The first-run flow of
/// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`, plus
/// the runtime path
/// `docs/decisions/0017-the-runtimes-path-is-recorded-in-the-instance.md`
/// added. It returns a state that is already valid, because there is no such
/// thing here as an instance waiting to be finished.
fn configure() -> Result<State, StartupError> {
    let unattended = supplied(TOKEN_VARIABLE).is_some() && supplied(RUNTIME_VARIABLE).is_some();
    if !unattended {
        println!("No instance here yet, so a few questions — once.");
        println!();
    }

    let agent = choose_agent(unattended)?;
    let auth_token = Secret::new(match supplied(TOKEN_VARIABLE) {
        Some(given) => given,
        None => rpassword::prompt_password(format!("  credential for {agent:?}: "))
            .map_err(StartupError::Console)?
            .trim()
            .to_owned(),
    });
    if auth_token.expose().is_empty() {
        return Err(StartupError::Empty("a credential"));
    }
    let container_runtime = match supplied(RUNTIME_VARIABLE) {
        Some(given) => PathBuf::from(given),
        None => choose_runtime()?,
    };

    if !unattended {
        println!();
    }
    Ok(State::new(
        agent,
        AgentConfig { auth_token },
        container_runtime,
    ))
}

/// The value of an optional variable, if it holds anything.
///
/// Blank counts as absent. A variable set to the empty string is nearly always
/// an unset one that went through a shell, and treating it as an answer would
/// configure an instance with a credential of no characters — which fails much
/// later, on the first job, with nothing pointing back to here.
fn supplied(name: &'static str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Which agent the orchestrator will think with.
///
/// A menu even while only one agent exists, because the question is the same
/// one either way and a flow that appears when a second arrives is a flow
/// nobody has run.
fn choose_agent(quietly: bool) -> Result<Agent, StartupError> {
    let agents = Agent::ALL;

    let Some((only, [])) = agents.split_first() else {
        if !quietly {
            println!("  Which agent should the orchestrator think with?");
            // Numbered by zipping a range rather than by adding to an index:
            // that addition is the kind the gate denies outright, and counting
            // from one is not worth an escape hatch.
            for (position, agent) in (1_usize..).zip(agents) {
                println!("    {position}) {agent:?} — {}", agent.description());
            }
        }
        let chosen = ask("  agent: ")?;
        // `checked_sub` rather than a saturating one: entering zero is a
        // mistake, and saturating would answer it with the first agent instead
        // of saying so.
        return chosen
            .parse::<usize>()
            .ok()
            .and_then(|position| agents.get(position.checked_sub(1)?))
            .copied()
            .ok_or(StartupError::Empty("an agent"));
    };
    if !quietly {
        println!("  Agent: {only:?} — {}", only.description());
        println!("  It is the only one compiled in, so there is nothing to choose.");
    }
    Ok(*only)
}

/// Where the container runtime lives, suggested if it can be found.
fn choose_runtime() -> Result<PathBuf, StartupError> {
    let found = RUNTIME_SUGGESTIONS
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.exists());

    let prompt = found.as_ref().map_or_else(
        || "  container runtime, full path: ".to_owned(),
        |suggestion| format!("  container runtime [{}]: ", suggestion.display()),
    );

    let answered = ask(&prompt)?;
    if answered.is_empty() {
        return found.ok_or(StartupError::Empty("a path to a container runtime"));
    }
    Ok(PathBuf::from(answered))
}

/// Puts a question and reads the answer.
fn ask(prompt: &str) -> Result<String, StartupError> {
    print!("{prompt}");
    io::stdout().flush().map_err(StartupError::Console)?;
    let mut answered = String::new();
    io::stdin()
        .read_line(&mut answered)
        .map_err(StartupError::Console)?;
    Ok(answered.trim().to_owned())
}
