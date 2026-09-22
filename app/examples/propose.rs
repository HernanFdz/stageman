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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::sync::Arc;
use std::time::Duration;

use stageman::world::{Asking, Performer};
use stageman_core::{
    Agent, AgentConfig, JobId, Key, Kit, KitConfig, KitName, NONCE_LEN, Platform, Project,
    ProjectId, Secret, State, Timestamp, Uuid,
};
use stageman_instance::{Instance, Request, Response, Seed, Target};
use stageman_vocabulary::Environment;
use stageman_wire::Standing;

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
            foreman_kit: Kit::defaults(Agent::Claude),
            kits: std::collections::BTreeMap::from([(
                KitName::new("Claude").map_err(|error| format!("a kit's name: {error}"))?,
                KitConfig::defaults(Agent::Claude),
            )]),
            credentials,
            channels: std::collections::BTreeMap::new(),
            variables: std::collections::BTreeMap::new(),
            jobs: std::collections::BTreeMap::new(),
            attending: stageman_core::Attending::default(),
            brief: String::new(),
            watched: std::collections::BTreeSet::new(),
            foreman_room: None,
        },
    );

    let scratch = std::env::temp_dir().join(format!("stageman-propose-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).map_err(|error| format!("no scratch directory: {error}"))?;
    let path = scratch.join("state.json");
    let key = throwaway_key();
    std::fs::write(&path, sealed(&state, &key)?)
        .map_err(|error| format!("the instance could not be written: {error}"))?;

    let world = stood_up(&path, &key);

    println!("Proposing against {repository}");
    println!("This runs a real agent and opens a real pull request. It takes a few minutes.");
    println!();

    let asked = world
        .ask(Request::Start {
            project: project.to_string(),
            kit: "Claude".to_owned(),
            work: WORK.to_owned(),
            at: Timestamp::now(),
        })
        .await;
    let job = match asked {
        Some(Response::Jobs(working)) => working
            .jobs
            .first()
            .map(|job| job.id.clone())
            .ok_or("the job was not listed")?,
        Some(Response::Refused(why)) => return Err(format!("the job was refused: {why}")),
        _ => return Err("the instance did not answer".to_owned()),
    };

    let outcome = finished(&world, project, &job).await?;
    let container = Uuid::parse_str(&job)
        .map(JobId::from_uuid)
        .map(stageman_job::container)
        .map_err(|error| format!("the job's identifier: {error}"))?;

    println!();
    println!("  job        {job}");
    println!("  container  {container}");
    println!("  outcome    {outcome:?}");
    println!();
    println!("The container is kept, because nothing retires one yet. Look inside it with:");
    println!("  docker cp {container}:/workspace ./somewhere");
    println!("and remove it with:");
    println!("  docker rm -f {container}");
    Ok(())
}

/// Stands the world up the way the daemon stands it up, less the dashboard:
/// the instance boots from an environment naming its file and its key, takes
/// the address a container reaches its tools on, and is stepped by the loop.
fn stood_up(path: &Path, key: &Key) -> Arc<Asking> {
    let mut environment: Environment = std::env::vars().collect();
    environment.insert(
        stageman_instance::STATE_VARIABLE.to_owned(),
        path.display().to_string(),
    );
    environment.insert(stageman_instance::KEY_VARIABLE.to_owned(), key.to_base64());
    let (instance, effects) = Instance::boot(seed(), environment, Target::compiled());
    let (world, events) = stageman_world::World::new();
    let asking = Asking::new(Arc::clone(&world));
    stageman::world::adopt(Arc::clone(&asking));
    // No pages are served here, and the instance is told a port anyway:
    // it forwards what it cannot place to the presentation server, and
    // nothing in this example visits one.
    asking.send(stageman_instance::AppEvent::Presenting { port: 0 });
    let performer = Performer::new(Arc::clone(&asking));
    stageman_world::run(instance, effects, world, Arc::new(performer), events);
    asking
}

/// Waits for the job to stop working, by asking: nothing else in this process
/// is watching it, and the instance answers between one turn and the next.
async fn finished(world: &Asking, project: ProjectId, job: &str) -> Result<Standing, String> {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let Some(Response::Jobs(working)) = world
            .ask(Request::Jobs {
                project: project.to_string(),
            })
            .await
        else {
            return Err("the instance stopped answering".to_owned());
        };
        let Some(listed) = working.jobs.iter().find(|listed| listed.id == job) else {
            return Err("the job vanished from its project".to_owned());
        };
        if listed.standing != Standing::Working {
            return Ok(listed.standing.clone());
        }
    }
}

/// The state, sealed as the instance would seal it, for the instance to open.
fn sealed(state: &State, key: &Key) -> Result<Vec<u8>, String> {
    let mut nonces = || {
        let mut nonce = [0_u8; NONCE_LEN];
        nonce.copy_from_slice(&Uuid::new_v4().into_bytes()[..NONCE_LEN]);
        nonce
    };
    let snapshot = state
        .seal(key, &mut nonces)
        .map_err(|error| format!("the state could not be sealed: {error}"))?;
    serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("the state could not be encoded: {error}"))
}

/// A seed for the instance, from the same well the identifiers come from.
fn seed() -> Seed {
    let mut seed = [0_u8; 32];
    seed[..16].copy_from_slice(&Uuid::new_v4().into_bytes());
    seed[16..].copy_from_slice(&Uuid::new_v4().into_bytes());
    seed
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

/// A key for an instance that is discarded when this ends.
///
/// Fixed rather than random only because nothing reopens this snapshot. A real
/// instance takes its key from the environment and would be unreadable without
/// it; this one is unreadable because it is deleted.
const fn throwaway_key() -> Key {
    Key::new([0; 32])
}
