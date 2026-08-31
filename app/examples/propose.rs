//! Drives one real job that proposes a real change, against a real repository.
//!
//! **Deliberately not a test, and the reason is structural rather than
//! stylistic.** `just image-handshake` runs every ignored test that does not
//! cost a credential, precisely so it keeps covering new ones — which means an
//! ignored test that opened a pull request would fire on a command run several
//! times a session. There is no way to write this as a test that the project's
//! own tooling will not eventually run, so it is an example: compiled and
//! linted by the gate's `--all-targets` pass, and executed only when a person
//! types the command.
//!
//! Run it with `just propose <repository-url>`.
//!
//! Credentials come from the gitignored files this project already keeps them
//! in, and never from an argument: anything in a command line is readable from
//! the process table by any user on the machine, which is the same reason
//! containers are given `--env NAME` rather than `--env NAME=value`.

use std::path::PathBuf;
use std::process::ExitCode;

use stageman::Store;
use stageman_agent::ContainerRuntime;
use stageman_core::{Agent, AgentConfig, Key, Platform, Project, ProjectId, Secret, State, Uuid};

/// The work this job is asked to do.
///
/// Chosen to be real, small, and reviewable: the binary reads five environment
/// variables and `README.md` documents two. A documentation change is also the
/// smallest blast radius available for a first run against a live repository.
const WORK: &str = "\
The README documents two environment variables, STAGEMAN_KEY and \
STAGEMAN_STATE, but the binary reads five. The three it does not mention are \
STAGEMAN_AGENT_TOKEN and STAGEMAN_CONTAINER_RUNTIME, which together let a \
first run be provisioned without a terminal, and STAGEMAN_LOG, which sets how \
much is reported.

Document the three that are missing. Read AGENTS.md and docs/conventions.md \
first: this project has strong conventions about how it is written, and a \
change that ignores them is worse than no change. Match the voice of the \
surrounding prose, and do not restate anything the README already says.";

/// Why this job was created, for whoever reads it afterwards.
const REASON: &str = "the first proposal this system has ever made, run by hand";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("STAGEMAN_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        eprintln!("propose: no async runtime");
        return ExitCode::FAILURE;
    };
    match runtime.block_on(propose()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("propose: {failure}");
            ExitCode::FAILURE
        }
    }
}

async fn propose() -> Result<(), String> {
    let repository = std::env::args()
        .nth(1)
        .ok_or("give the repository URL as the only argument")?;

    let agent_token = read_secret("anthropic-token")?;
    let platform_token = read_secret("github-token")?;
    let runtime = ContainerRuntime::new(located_runtime()?);
    runtime
        .verify()
        .await
        .map_err(|error| format!("the container runtime is not usable: {error}"))?;

    // An instance that exists only for this run. Nothing configures a project
    // yet — that is the next step in `docs/open-questions.md` — so this builds
    // one directly, which is exactly what a dashboard will do later.
    let mut state = State::default();
    state.agents.insert(
        Agent::Claude,
        AgentConfig {
            auth_token: agent_token,
        },
    );
    let project = ProjectId::from_uuid(Uuid::new_v4());
    let mut credentials = std::collections::BTreeMap::new();
    credentials.insert(Platform::GitHub, platform_token);
    state.projects.insert(
        project,
        Project {
            name: "stageman".to_owned(),
            repository: repository.clone(),
            foreman_agent: Agent::Claude,
            job_agents: std::collections::BTreeSet::from([Agent::Claude]),
            credentials,
            channels: std::collections::BTreeMap::new(),
            jobs: std::collections::BTreeMap::new(),
            attending: stageman_core::Attending::default(),
        },
    );

    let scratch = std::env::temp_dir().join(format!("stageman-propose-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).map_err(|error| format!("no scratch directory: {error}"))?;
    let store = Store::create(scratch.join("state.json"), throwaway_key(), state)
        .map_err(|error| format!("the instance could not be created: {error}"))?;

    println!("Proposing against {repository}");
    println!("This runs a real agent and opens a real pull request. It takes a few minutes.");
    println!();

    let (job, progress) = stageman::run(&store, &runtime, project, Agent::Claude, REASON, WORK)
        .await
        .map_err(|error| format!("the job could not be created: {error}"))?;

    println!();
    println!("  job        {job}");
    println!("  container  {}", stageman_job::container(job));
    println!("  outcome    {progress:?}");
    println!();
    println!("The container is kept, because nothing retires one yet. Look inside it with:");
    println!(
        "  docker cp {}:/workspace ./somewhere",
        stageman_job::container(job)
    );
    println!("and remove it with:");
    println!("  docker rm -f {}", stageman_job::container(job));
    Ok(())
}

/// A credential, from the gitignored file this project keeps it in.
fn read_secret(name: &str) -> Result<Secret, String> {
    let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.local")).join(name);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("no credential at {}: {error}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{} is empty", path.display()));
    }
    Ok(Secret::new(trimmed.to_owned()))
}

/// Where the container runtime is.
///
/// Looked up when nothing says, which is allowed here for the reason the rule
/// itself gives: `docs/conventions.md` §3 is about a daemon that must work
/// under a service manager, and this is a command a person runs by hand.
fn located_runtime() -> Result<PathBuf, String> {
    if let Ok(configured) = std::env::var("STAGEMAN_CONTAINER_RUNTIME")
        && !configured.trim().is_empty()
    {
        return Ok(PathBuf::from(configured.trim()));
    }
    let located = std::process::Command::new("sh")
        .args(["-c", "command -v docker"])
        .output()
        .map_err(|error| format!("looking for a container runtime: {error}"))?;
    let path = String::from_utf8_lossy(&located.stdout).trim().to_owned();
    if path.is_empty() {
        return Err("no container runtime found; set STAGEMAN_CONTAINER_RUNTIME".to_owned());
    }
    Ok(PathBuf::from(path))
}

/// A key for an instance that is discarded when this ends.
///
/// Fixed rather than random only because nothing reopens this snapshot. A real
/// instance takes its key from the environment and would be unreadable without
/// it; this one is unreadable because it is deleted.
const fn throwaway_key() -> Key {
    Key::new([0; 32])
}
