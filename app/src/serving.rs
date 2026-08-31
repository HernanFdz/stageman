//! Starting an instance, and serving the dashboard from it.
//!
//! This is the daemon's half of the binary and never reaches the browser. It
//! lives here rather than in `main.rs` because that file now holds two entry
//! points — `dx` builds the same binary for both halves — and an entry point
//! carrying a hundred lines of startup would make it impossible to see at a
//! glance which half you are reading.
//!
//! Serving is plain `axum` around a router Dioxus assembles, rather than
//! `dioxus::serve`, which builds a runtime of its own and never returns. The
//! difference is what `docs/conventions.md` §3 asks for: whatever makes an
//! instance unusable has to fail at startup, with an exit code and a reason,
//! and that is not available inside a function that cannot return.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, LazyLock};

use dioxus::prelude::{DioxusRouterExt as _, ServeConfig};
use dioxus::server::axum;
use etcetera::BaseStrategy as _;
use rand::rngs::{StdRng, SysRng};
use rand::{Rng as _, SeedableRng as _};
use stageman_agent::{AgentError, ContainerRuntime};
use stageman_core::{Key, KeyError, State};

use crate::Dashboard;
use crate::{LoadError, Store};

/// The variable the snapshot's encryption key arrives in, as base64.
///
/// An override rather than a requirement since
/// `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md`, and
/// still what a deliberate deployment sets — a service manager passing a
/// secret in has somewhere to put it, and nothing about that changed.
///
/// What it is *not* is the only way of saying. Requiring it made a downloaded
/// binary refuse to start until somebody generated thirty-two bytes by hand,
/// which is the last thing between "put this somewhere and run it" and the
/// truth.
const KEY_VARIABLE: &str = "STAGEMAN_KEY";

/// What the generated key is called, in the platform's configuration
/// directory.
///
/// A different directory from the instance, not merely a different name. The
/// rule it answers is that a key beside the file it protects protects nothing,
/// and 0037 records exactly how much of that survives per platform: two
/// directories on Linux and macOS, one on Windows, where the platform defines
/// its configuration directory as its data directory. The property `README.md`
/// actually claims is about the *file*, and a separate file keeps it
/// everywhere.
const KEY_FILE: &str = "key";

/// How a generated key file is created, where the platform has an opinion.
///
/// Owner read and write and nothing else. It does not make the key private
/// from anything running as this user — 0037 is explicit that nothing can,
/// and that a variable is no better — but a key file readable by every account
/// on a shared machine would be worse than what it replaced, and that is worth
/// one constant.
#[cfg(unix)]
const KEY_PERMISSIONS: u32 = 0o600;

/// The variable naming the file the instance is kept in.
///
/// An override rather than a requirement, and it used to be the only way of
/// saying. Where an instance lives is an operational detail rather than a
/// choice anybody should have to make, so there is now a per-platform default
/// and this exists for the cases that genuinely differ: a second instance on
/// one machine, and a test that must not touch the real one.
///
/// The reasoning that made it mandatory is intact and is what rules out
/// defaulting to the *working directory*. A daemon started by a service
/// manager has one nobody chose, so a relative default would be the same trap
/// as searching `PATH` — right when tested by hand, wrong in service. What
/// replaced it is absolute and derived from the platform rather than from
/// wherever the process happens to have been started.
const STATE_VARIABLE: &str = "STAGEMAN_STATE";

/// The directory this instance's file goes in, under the platform's own.
const INSTANCE_DIRECTORY: &str = "stageman";

/// What the instance's file is called.
const INSTANCE_FILE: &str = "instance.json";

/// How much is reported, when the environment does not say.
const DEFAULT_VERBOSITY: &str = "warn";

/// The variable that overrides how much is reported.
const VERBOSITY_VARIABLE: &str = "STAGEMAN_LOG";

/// An instance could not be started.
#[derive(Debug, thiserror::Error)]
enum StartupError {
    /// The key is set but is not key material.
    #[error("the instance key is not usable")]
    Key(#[source] KeyError),
    /// The key could not be read from, or written to, where it is kept.
    ///
    /// The same class as an instance file that cannot be opened: an instance
    /// cannot run without one, so `docs/conventions.md` §3 puts it at startup
    /// rather than in the dashboard. It says the path, because the repair is
    /// almost always a permission on that directory.
    #[error("the instance key at {path} could not be read or written")]
    KeyFile {
        /// Where it is kept.
        path: PathBuf,
        /// Why it could not be.
        #[source]
        source: io::Error,
    },
    /// The instance could not be opened or created.
    #[error("the instance at {path} could not be opened")]
    Instance {
        /// Where it was looked for.
        path: PathBuf,
        /// Why it could not be opened.
        #[source]
        source: LoadError,
    },
    /// There is no container runtime on this machine.
    ///
    /// Several lines, because it is the one startup failure that is a fact
    /// about the machine rather than about this instance, and the useful part
    /// of it is the list of places that were looked in.
    #[error(
        "no container runtime found.\n  Every agent runs in a container, including the \
         one a foreman thinks with,\n  so nothing here can run without one. Install Docker \
         or Podman.\n  Looked in:\n{0}"
    )]
    NoRuntime(String),
    /// The container runtime is there and not working.
    #[error("the container runtime is not usable")]
    Runtime(#[source] AgentError),
    /// The runtime would not say what containers it has.
    #[error("what the last run left behind could not be established")]
    Sweep(#[source] stageman_job::JobError),
    /// The address the dashboard would be served on could not be taken.
    #[error("the dashboard cannot listen on {address}")]
    Listen {
        /// What it tried to bind.
        address: SocketAddr,
        /// Why it could not.
        #[source]
        source: io::Error,
    },
    /// Serving stopped on something other than being asked to.
    #[error("serving the dashboard stopped")]
    Serving(#[source] io::Error),
    /// There is no randomness to generate a key from.
    ///
    /// Refused rather than substituted. A predictable key is worse than no
    /// key, because it encrypts and looks like it worked.
    #[error("no source of randomness, so an instance key cannot be generated")]
    NoRandomness,
    /// There is no home directory to put an instance under.
    #[error("no home directory, so there is nowhere to keep an instance — set {STATE_VARIABLE}")]
    NoHome(#[source] etcetera::HomeDirError),
    /// The directory the instance goes in could not be made.
    #[error("the directory for the instance at {path} could not be created")]
    Directory {
        /// Where it tried to put it.
        path: PathBuf,
        /// Why it could not.
        #[source]
        source: io::Error,
    },
}

/// The container runtime this process uses.
///
/// A value rather than an `Option`, which is the point of it being here: every
/// reader — startup, a server function, whatever comes next — gets a runtime
/// and none of them writes a branch for a state that
/// `docs/decisions/0023-the-container-runtime-is-discovered-once.md` makes
/// impossible. A machine with no runtime cannot run stageman at all, so there
/// is nothing a later caller could do about it.
///
/// **There is exactly one moment at which that guarantee is not yet true**, and
/// it is between this being initialised and startup checking it. Discovery
/// finding nothing yields a runtime with an empty path, and one private
/// function in this module is the definition of what that means; startup asks
/// before doing anything else and stops if the answer is yes, so nothing
/// downstream ever holds one. Anything reading this without a start in front of
/// it is outside that guarantee — which is why nothing does.
///
/// Read once per process. A runtime installed while this is running is picked
/// up by restarting, which is the answer for everything else checked at
/// startup too.
pub static RUNTIME: LazyLock<ContainerRuntime> = LazyLock::new(|| {
    stageman_agent::first_present(stageman_agent::candidates())
        .unwrap_or_else(|| ContainerRuntime::new(PathBuf::new()))
});

/// Which credential means what, for the sessions running right now.
///
/// A process-wide value like [`RUNTIME`] above, and for a plainer reason: it
/// is written where a session is declared and read where a tool is called, and
/// those are on opposite sides of the daemon with nothing but the request
/// between them. Threading it through every caller would put an argument in a
/// dozen signatures to reach two of them.
///
/// Ephemeral by design — see `crate::tooling::Sessions`. Nothing here is
/// persisted, so restarting invalidates every credential outstanding, and each
/// container is handed a current one on its next turn.
pub static SESSIONS: LazyLock<Arc<crate::tooling::Sessions>> =
    LazyLock::new(|| Arc::new(crate::tooling::Sessions::default()));

/// Whether discovery came back with nothing.
///
/// The empty path is a sentinel, and this is the only place that knows it. A
/// function rather than a comparison written at each site, because a sentinel
/// nobody names is one somebody eventually forgets to check — and the whole
/// cost of this shape is that the compiler will not remind them.
fn missing(runtime: &ContainerRuntime) -> bool {
    runtime.path().as_os_str().is_empty()
}

/// Starts an instance and serves its dashboard until the process is stopped.
///
/// The whole of what running stageman means, and the reason this returns an
/// [`ExitCode`] rather than a `Result`: a failure here is the program's last
/// word to whoever ran it, so it is printed in full and turned into a status
/// rather than handed to a caller who has nothing better to do with it.
#[must_use]
pub fn serve() -> ExitCode {
    // Standard error, which is a real answer for a process somebody started
    // and is watching, and a placeholder for the daemon this becomes — see
    // `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md`.
    //
    // Installed before anything else, and that ordering is load-bearing now
    // that Dioxus is here: `dioxus_logger` installs a subscriber of its own
    // unless one is already set, so being second would mean losing
    // `STAGEMAN_LOG` to a default nobody chose.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env(VERBOSITY_VARIABLE)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_VERBOSITY)),
        )
        .with_writer(io::stderr)
        .init();

    // Before the async runtime, the instance file, or anything else that could
    // fail first. A machine with no container runtime cannot run stageman, and
    // the useful thing to say about that is the only thing worth saying — so
    // it is said before any other failure can get in front of it.
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
    // First, before the instance file or anything configurable. A machine with
    // no container runtime cannot run stageman at all, so that is the failure
    // worth reporting even when something else is also wrong: telling somebody
    // their key is unset, when the answer is that they need Docker, sends them
    // to fix the wrong thing.
    let runtime: &ContainerRuntime = &RUNTIME;
    if missing(runtime) {
        return Err(StartupError::NoRuntime(
            stageman_agent::candidates()
                .iter()
                .map(|candidate| format!("    {candidate}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }

    let (key, source) = instance_key()?;
    let path = instance_path()?;

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
        Store::create(path.clone(), key, State::default()).map_err(|source| {
            StartupError::Instance {
                path: path.clone(),
                source,
            }
        })?
    };

    // Being found is not being usable, and both are checked. `RUNTIME` has
    // already established that something is installed — see the touch in
    // `serve` — and this establishes that it answers. The two are different
    // failures and an operator does something different about each: nothing
    // installed, versus a client installed with no daemon behind it, which
    // looks perfectly healthy to anything that merely looks for the file.
    //
    // That second one is not hypothetical here. A container with the Docker
    // client installed and no daemon reachable answers `--version` happily and
    // fails `version`, which is why this asks for the latter.
    runtime.verify().await.map_err(StartupError::Runtime)?;

    let swept = crate::reconcile(&store, runtime)
        .await
        .map_err(StartupError::Sweep)?;

    // Bound before anything is printed, so that the address reported is the
    // one in use rather than the one intended — a port of zero is an ordinary
    // request for whichever port is free, and the answer is only known here.
    // It is also what makes the summary below a readiness signal: the socket
    // is already accepting by the time the line naming it appears.
    let address = dioxus::cli_config::fullstack_address_or_localhost();
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|source| StartupError::Listen { address, source })?;
    let serving = listener
        .local_addr()
        .map_err(|source| StartupError::Listen { address, source })?;

    // Handed to the dashboard's route rather than reached for through a
    // global. One process operates one instance, so a global would even be
    // true — and it would make a route's dependencies invisible at the point
    // somebody has to test one.
    let store = Arc::new(store);

    // Two routers, and which one is a question about how this binary was
    // built rather than about how it is configured. `cargo build` produces a
    // server and no client, and that is a working thing to run — the page is
    // rendered here and arrives complete, it just does not come alive
    // afterwards. Anything that serves the bundle it does not have would be
    // serving nothing.
    let bundle = bundle().filter(|path| path.is_dir());
    let router = axum::Router::new();
    let router = if bundle.is_some() {
        router.serve_dioxus_application(ServeConfig::new(), Dashboard)
    } else {
        router.serve_api_application(ServeConfig::new(), Dashboard)
    };
    // Reached by the dashboard's route as an extension rather than as a
    // context, because a context only exists while a page is being rendered —
    // see `crate::dashboard::instance`. A layer is on every request, which is
    // both paths.
    let router = router.layer(axum::Extension(Arc::clone(&store)));

    // One task per project that has somewhere to listen. Spawned rather than
    // awaited, and after the sweep rather than before: a reply arriving for a
    // job the sweep has not yet placed would be routed against a state that is
    // still being repaired.
    //
    // `docs/conventions.md` §3 keeps this off the request path, which matters
    // more here than anywhere — a socket is open for as long as the process is,
    // and awaiting one would mean the dashboard never starts.
    crate::listen(&store, runtime);

    // Where a foreman asks for a job. Its own listener, on its own port and
    // every interface — the dashboard stays on loopback, and this cannot,
    // because a container reaches nothing else on every platform. See
    // `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
    //
    // Spawned rather than awaited, for the reason everything else here is: it
    // runs for as long as the process does.
    // Bound before anything is announced, so that a port already taken is
    // reported on the startup block rather than logged into a scrolling
    // terminal. That failure leaves a foreman able to talk and unable to work,
    // which reads exactly like a foreman that decided not to — and it is not
    // hypothetical: a leaked test process held this port and a real instance
    // quietly could not have it.
    let tools = crate::endpoint::bind().await;

    let kept = Kept {
        file: path,
        key: source,
    };
    announce(
        runtime,
        &store,
        &swept,
        serving,
        bundle.as_deref(),
        &kept,
        tools
            .as_ref()
            .ok()
            .and_then(|bound| bound.local_addr().ok()),
    )?;

    match tools {
        Ok(listening) => {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                if let Err(why) =
                    crate::endpoint::serve(listening, store, Arc::clone(&SESSIONS)).await
                {
                    tracing::error!(%why, "the job endpoint stopped");
                }
            });
        }
        Err(why) => tracing::error!(%why, "no foreman can ask for a job"),
    }

    axum::serve(listener, router)
        .await
        .map_err(StartupError::Serving)
}

/// How many channels this instance can hear replies on.
///
/// A channel bound without the credential that listens is counted as nothing,
/// which is the point: it is the ordinary shape of a project that speaks and
/// is not answered, and it is also what a half-finished setup looks like.
fn listening(state: &stageman_core::State) -> usize {
    state
        .projects
        .values()
        .filter(|project| {
            project
                .channels
                .values()
                .any(|bound| bound.listen_credential.is_some())
        })
        .count()
}

#[cfg(test)]
mod listening_tests {
    use super::listening;
    use stageman_core::{Agent, Channel, ChannelConfig, Project, ProjectId, Secret, State, Uuid};
    use std::collections::{BTreeMap, BTreeSet};

    /// A project with a channel, and a credential to listen with or not.
    fn watching(name: &str, listens: bool) -> Project {
        Project {
            name: name.to_owned(),
            repository: "https://example.invalid/repo".to_owned(),
            foreman_agent: Agent::Claude,
            job_agents: BTreeSet::from([Agent::Claude]),
            credentials: BTreeMap::new(),
            channels: BTreeMap::from([(
                Channel::Slack,
                ChannelConfig {
                    address: format!("C-{name}"),
                    credential: Secret::new("xoxb-token".to_owned()),
                    listen_credential: listens.then(|| Secret::new("xapp-token".to_owned())),
                },
            )]),
            jobs: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
        }
    }

    /// The count says what can be heard, not what is bound.
    ///
    /// The distinction this line exists for: a channel bound with no
    /// credential to listen with is a project that speaks and is not answered,
    /// which looks exactly like a half-finished setup and produces no warning
    /// either way.
    #[test]
    fn only_a_channel_with_somewhere_to_listen_is_counted() {
        let mut state = State::default();
        assert_eq!(listening(&state), 0);

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(1)),
            watching("a", false),
        );
        assert_eq!(listening(&state), 0, "bound is not the same as listening");

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(2)),
            watching("b", true),
        );
        assert_eq!(listening(&state), 1);

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(3)),
            watching("c", true),
        );
        assert_eq!(listening(&state), 2);
    }
}

/// Says what was found and where the dashboard is, on standard output.
///
/// Not through `tracing`, for the same reason [`report`] is not: this is what
/// somebody who typed the command is waiting to read, and a verbosity setting
/// must not be able to withhold it.
fn announce(
    runtime: &ContainerRuntime,
    store: &Store,
    swept: &crate::Swept,
    serving: SocketAddr,
    bundle: Option<&Path>,
    kept: &Kept,
    tools: Option<SocketAddr>,
) -> Result<(), StartupError> {
    println!();
    println!("stageman is running.");
    println!("  instance   {}", kept.file.display());
    // Printed for the reason the instance path is: an instance opened under a
    // key nobody meant to use looks exactly like an instance that lost its
    // projects, and this is the one line that tells those apart.
    println!("  key        {}", kept.key);
    println!("  runtime    {}", runtime.path().display());
    println!("  agents     {}", store.read().agents.len());
    println!("  projects   {}", store.read().projects.len());
    // Printed rather than logged, because the failure it answers is a channel
    // bound with no credential to listen with — which is not an error, produces
    // no warning, and is indistinguishable from a platform sending nothing.
    // The count is the cheapest thing that tells them apart.
    println!("  listening  {} channel(s)", listening(&store.read()));
    // Printed because it is a port open on every interface, and because it
    // can fail to open at all — a port already taken leaves a foreman able to
    // talk and unable to work, and that has to be visible here rather than in
    // a log line nobody was watching.
    match tools {
        Some(bound) => println!("  tools      {bound}"),
        None => println!(
            "  tools      NOT SERVED — port {} is taken, so no agent can reach a tool and \
             none will say so",
            *crate::endpoint::PORT
        ),
    }
    println!(
        "  swept      {} resumed, {} failed, {} stranded",
        swept.resumed, swept.failed, swept.stranded
    );
    println!(
        "  left alone {} unidentified, {} naming a forgotten job",
        swept.unidentified, swept.forgotten
    );
    println!(
        "  client     {}",
        bundle.map_or_else(
            || "not built — the dashboard is rendered here and stays still".to_owned(),
            |path| path.display().to_string()
        )
    );
    println!();
    // Last, and that ordering is load-bearing rather than cosmetic: this line
    // is what anything supervising a start waits for, so everything worth
    // reading has to be above it. The integration tests stop reading here.
    println!("  dashboard  http://{serving}");
    println!();
    // Rust's standard output is line-buffered rather than terminal-aware, so
    // the lines above have already left. Flushed anyway because the line
    // naming the address is what anything supervising this process waits for,
    // and depending on a buffering policy for that would be depending on an
    // implementation detail.
    io::stdout().flush().map_err(StartupError::Serving)
}

/// Where this instance is kept, and what opens it.
///
/// One value rather than two arguments, and the grouping is the honest one: a
/// file opened at the right path under the wrong key is indistinguishable from
/// an instance that lost its projects, so the summary names both or the line
/// naming one of them is a trap.
struct Kept {
    /// The file this instance lives in.
    file: PathBuf,
    /// Where the key that opens it came from.
    key: KeySource,
}

/// Where the instance key came from, for the line that says so at startup.
///
/// Worth reporting rather than assuming, and the reason is the failure it
/// prevents: an operator who believes they set the variable, and did not,
/// otherwise sees an instance that opens perfectly and holds none of their
/// projects — because it was opened under a different key and created afresh.
/// Naming the source makes that one line of output instead of an evening.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeySource {
    /// Supplied in the environment, which is what a service manager does.
    Environment,
    /// Read from where a previous start generated it.
    Kept(PathBuf),
    /// Generated by this start, because there was none.
    Generated(PathBuf),
}

impl fmt::Display for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment => write!(f, "{KEY_VARIABLE}"),
            Self::Kept(path) => write!(f, "{}", path.display()),
            Self::Generated(path) => write!(f, "{} (generated just now)", path.display()),
        }
    }
}

/// The key this instance's file is encrypted under, and where it came from.
///
/// The environment first, because an operator who said so meant it and because
/// that is the path a service manager takes. Otherwise the platform's
/// configuration directory, where a previous start left one or where this
/// start puts one — see
/// `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md` for what
/// that buys and what it gives up.
///
/// Generating is not a fallback hiding a failure, which is the distinction
/// `.quality/gate-reference.md` cares about. A first run genuinely has no key,
/// and thirty-two fresh bytes are the true answer rather than a substituted
/// default: what would be wrong is generating a *second* one over an instance
/// that already has a file, and that cannot happen here because a key is only
/// ever created when the file holding it does not exist.
///
/// # Errors
///
/// Fails if the variable is set to something that is not key material, if
/// there is no home directory to keep one under, or if the file cannot be read
/// or written. A key that cannot be established is an instance that cannot
/// open, so all three stop the start.
fn instance_key() -> Result<(Key, KeySource), StartupError> {
    if let Some(said) = optional(KEY_VARIABLE) {
        return Ok((
            Key::from_base64(&said).map_err(StartupError::Key)?,
            KeySource::Environment,
        ));
    }

    let path = configuration_directory()?.join(KEY_FILE);
    if let Some(text) = kept(fs::read_to_string(&path), &path)? {
        return Ok((
            Key::from_base64(text.trim()).map_err(StartupError::Key)?,
            KeySource::Kept(path),
        ));
    }
    let key = minted()?;
    write_key(&path, &key)?;
    Ok((key, KeySource::Generated(path)))
}

/// What a read of the key file meant: the key, or that there is not one yet.
///
/// Deciding, split from reading, for the reason [`missing`] above is a named
/// function rather than a comparison at its one call site — except that this
/// one earns it twice over. The distinction is the whole of what stands
/// between "this is a first run" and "your key file cannot be read", and those
/// want opposite answers: generating over the second would report that the
/// file already exists, which is true, useless, and points away from the
/// permission that actually failed.
///
/// Taking the read rather than performing it is what makes that assertable. A
/// test can hand this a permission failure; a test that had to *produce* one
/// on a real filesystem would depend on not being run as root.
///
/// Mutation testing found this, and then found it again: naming the comparison
/// left the guard that used it still uncovered, because nothing exercised the
/// site. Only moving the decision somewhere a test could reach it closed both.
fn kept(read: Result<String, io::Error>, path: &Path) -> Result<Option<String>, StartupError> {
    match read {
        Ok(text) => Ok(Some(text)),
        Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StartupError::KeyFile {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Thirty-two bytes from the operating system.
///
/// The randomness is supplied here rather than inside the domain crate, which
/// is the same split `State::seal` already makes for nonces: what a key is
/// belongs to the type, and where entropy comes from is a property of the
/// machine this happens to run on.
fn minted() -> Result<Key, StartupError> {
    let mut rng = StdRng::try_from_rng(&mut SysRng).map_err(|_| StartupError::NoRandomness)?;
    let mut material = [0_u8; 32];
    rng.fill_bytes(&mut material);
    Ok(Key::new(material))
}

/// Writes a generated key where it will be looked for next time.
///
/// Created with an explicit mode where the platform has one, rather than
/// written and then adjusted: a file that is briefly world-readable and then
/// tightened is readable for as long as it takes, and the window is the whole
/// of what this is guarding.
fn write_key(path: &Path, key: &Key) -> Result<(), StartupError> {
    let failed = |source| StartupError::KeyFile {
        path: path.to_owned(),
        source,
    };
    let mut opening = fs::OpenOptions::new();
    opening.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opening, KEY_PERMISSIONS);
    let mut file = opening.open(path).map_err(failed)?;
    file.write_all(key.to_base64().as_bytes()).map_err(failed)?;
    file.write_all(b"\n").map_err(failed)?;
    file.sync_all().map_err(failed)
}

/// The platform's configuration directory for this instance, created if absent.
///
/// Separate from the data directory, which is where the instance file goes.
/// On Windows the platform defines those as the same place and this returns
/// it, which 0037 records as a documented consequence rather than something to
/// work around.
fn configuration_directory() -> Result<PathBuf, StartupError> {
    let directory = etcetera::choose_base_strategy()
        .map_err(StartupError::NoHome)?
        .config_dir()
        .join(INSTANCE_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|source| StartupError::Directory {
        path: directory.clone(),
        source,
    })?;
    Ok(directory)
}

/// Where this instance is kept.
///
/// The platform's own data directory unless [`STATE_VARIABLE`] says otherwise
/// — the XDG data directory under a home on Linux *and on macOS*, the roaming
/// application data directory on Windows — according to the machine rather
/// than to a list maintained here.
///
/// macOS is named explicitly because the obvious guess is wrong and this said
/// it for a while: `choose_base_strategy` is the CLI convention and returns
/// XDG everywhere except Windows, so an instance lands beside the Linux one
/// rather than under Application Support. The sibling function that returns
/// Apple's own directories is the one this deliberately does not call.
///
/// Written without the literal Linux path on purpose: this repository has a
/// gitignored scratch directory whose name is a prefix of it, and `just drift`
/// cannot tell a home directory from a repository-relative one. It reports the
/// false positive rather than missing the real case, which is the right way
/// round.
///
/// The directory is created, because the alternative is a first run that fails
/// on a path the operator never chose and cannot be expected to have made.
fn instance_path() -> Result<PathBuf, StartupError> {
    if let Some(said) = optional(STATE_VARIABLE) {
        return Ok(PathBuf::from(said));
    }
    let directory = etcetera::choose_base_strategy()
        .map_err(StartupError::NoHome)?
        .data_dir()
        .join(INSTANCE_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|source| StartupError::Directory {
        path: directory.clone(),
        source,
    })?;
    Ok(directory.join(INSTANCE_FILE))
}

/// Where a client bundle would be, if this binary was built with one.
///
/// The same rule Dioxus applies, restated because the function that applies it
/// is private and because it panics on a directory that is not there. Asking
/// first turns "built without a client" from a crash into the ordinary state
/// of anything `cargo build` produces.
///
/// Restating somebody else's rule is drift waiting to happen, and what keeps
/// it honest is that the integration tests run this binary: a Dioxus upgrade
/// that moved the directory would fail them here rather than in production.
fn bundle() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DIOXUS_PUBLIC_PATH") {
        return Some(PathBuf::from(path));
    }
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|beside| beside.join("public"))
}

/// The value of a variable, when it is set to something.
///
/// Set-but-empty counts as unset, because a variable cleared by a wrapper
/// script is meant as *do not use this* rather than as a path of no
/// characters.
fn optional(name: &'static str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{RUNTIME, missing};
    use stageman_agent::ContainerRuntime;
    use std::path::PathBuf;

    /// Only a key file that is not there means there is no key yet.
    ///
    /// All three branches, because the failure this guards is one-sided: a
    /// decision that called every failure a first run would silently mint a
    /// second key over an unreadable one, and one that called none of them a
    /// first run would make a first run impossible.
    #[test]
    fn only_a_missing_key_file_means_there_is_no_key_yet() {
        use super::{StartupError, kept};
        use std::io::{Error, ErrorKind};

        let path = PathBuf::from("/nowhere/stageman/key");

        assert_eq!(
            kept(Ok("a key".to_owned()), &path).expect("a key that read is not a failure"),
            Some("a key".to_owned()),
        );
        assert_eq!(
            kept(Err(Error::from(ErrorKind::NotFound)), &path)
                .expect("a first run is not a failure"),
            None,
            "a key file that is not there is a first run",
        );
        for refused in [ErrorKind::PermissionDenied, ErrorKind::IsADirectory] {
            assert!(
                matches!(
                    kept(Err(Error::from(refused)), &path),
                    Err(StartupError::KeyFile { .. })
                ),
                "{refused:?} must not read as a first run — it would mint a second key",
            );
        }
    }

    /// The sentinel means what the one function that reads it says it means.
    ///
    /// Worth a test precisely because the compiler cannot check it: an empty
    /// path is a perfectly ordinary value, so nothing but this says that it is
    /// how discovery reports having found nothing.
    #[test]
    fn an_empty_path_is_how_a_missing_runtime_is_spelled() {
        assert!(missing(&ContainerRuntime::new(PathBuf::new())));
        assert!(!missing(&ContainerRuntime::new(PathBuf::from(
            "/usr/local/bin/docker"
        ))));
    }

    /// On a machine that can run these tests, discovery found something.
    ///
    /// Not a tautology: `just check` now requires a container runtime, so this
    /// asserts the requirement is actually met rather than merely declared —
    /// and it is the only test that would fail on a machine without one for a
    /// reason that names the cause.
    #[test]
    fn the_gate_runs_where_a_runtime_was_found() {
        assert!(
            !missing(&RUNTIME),
            "no container runtime on this machine — see docs/conventions.md §5"
        );
    }
}
