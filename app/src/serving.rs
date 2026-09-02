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
    /// The index could not be put where the framework will read it.
    ///
    /// Fatal rather than a warning, and that is the judgement worth recording:
    /// a binary carrying a browser half and unable to place its index serves a
    /// page that renders and never comes alive, which is almost indistinguishable
    /// from one that works. `docs/conventions.md` §3 puts what an operator can
    /// act on in the dashboard — and this cannot be, because the dashboard is
    /// the thing that would not be working.
    #[error(
        "the browser half could not be placed at {path}.\n  This binary carries \
         one, and the directory it must be written to is not writable.\n  Either \
         run from a directory you can write to, or set DIOXUS_PUBLIC_PATH to one."
    )]
    Bundle {
        /// Where it tried to write.
        path: PathBuf,
        /// Why it could not.
        #[source]
        source: io::Error,
    },
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
    // Every field on its own line: this is the whole output, so it has the room
    // that the startup block does not.
    //
    // Before the subscriber, the runtime, the instance and the runtime check,
    // because it is a question about this file rather than about this machine
    // — and it has to be answerable on a machine where none of the rest would
    // work. Asking a binary what it is must never require it to be able to run.
    if asked_what_it_is(std::env::args().skip(1)) {
        print!("{}", crate::release::detailed());
        return ExitCode::SUCCESS;
    }

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

/// Whether the only thing wanted is what this binary is.
///
/// Hand-rolled rather than parsed, because this is the whole command line
/// there is: stageman takes its configuration from the environment and the
/// dashboard, so a parser would be a dependency in front of one question. Both
/// spellings, because both are what people type.
///
/// Anything else is ignored rather than refused. A binary a service manager
/// starts with an argument nobody meant is better serving than exiting, and
/// there is no argument it could be given that means something else.
///
/// It takes the arguments rather than reading them, because a test cannot
/// choose what a process was started with — and the two spellings and the
/// comparison between them are exactly what is worth asserting.
fn asked_what_it_is(mut arguments: impl Iterator<Item = String>) -> bool {
    arguments.any(|argument| argument == "--version" || argument == "-V")
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

    // The work matters and the tally does not: everything it finds worth acting
    // on, it warns about by name as it goes.
    crate::reconcile(&store, runtime)
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
    // Recorded rather than recomputed, so that the address a job is told to
    // show somebody names the port in use. A port of zero is an ordinary
    // request for whichever one is free, and only this knows the answer.
    crate::tunnel::SERVING.get_or_init(|| serving.port());

    // Handed to the dashboard's route rather than reached for through a
    // global. One process operates one instance, so a global would even be
    // true — and it would make a route's dependencies invisible at the point
    // somebody has to test one.
    let store = Arc::new(store);

    // Three states, and which one holds is a question about how this binary
    // was built rather than about how it is configured.
    //
    // *Carrying its own* — what `just build` produces. The index is written
    // where the framework looks, read once, and removed; every other file is
    // served from memory by a route of ours, so the framework is asked for a
    // rendering that serves no static files at all.
    //
    // *A bundle beside it* — what `dx serve` arranges during development, and
    // what a hand-assembled directory looks like. The framework serves it.
    //
    // *Neither* — what `cargo build` produces, and a working thing to run: the
    // page is rendered here and arrives complete, it just does not come alive
    // afterwards.
    let carried = crate::bundle::CARRIED.index();
    let configured = configuration(carried, public_directory())?;

    let beside = public_directory().filter(|path| path.is_dir());
    let router = axum::Router::new();
    let router = if carried.is_some() {
        serving_embedded(router.serve_api_application(configured, Dashboard))
    } else if beside.is_some() {
        router.serve_dioxus_application(configured, Dashboard)
    } else {
        router.serve_api_application(configured, Dashboard)
    };
    // Warned rather than printed, because it is an anomaly rather than a fact:
    // every build that ships carries one. What it produces is a dashboard that
    // renders and never responds, which is the state most easily mistaken for
    // one that works — so it is worth saying, and worth saying only when true.
    if clientless(carried, beside.as_deref()) {
        tracing::warn!(
            "this build carries no browser half and none is beside it — the dashboard \
             will render and never respond. `just build` produces one that does"
        );
    }

    // The same treatment, for the same reason. This used to be a count of
    // channels with somewhere to listen, which told an operator that one of
    // three was misconfigured without telling them which. A binding with no
    // credential to listen with produces no error and looks exactly like a
    // platform that has sent nothing, so it has to be said — by name.
    // Bound before the loop so the read guard is dropped at this statement
    // rather than held across it: warning is not a reason to keep the instance
    // locked, and the gate is right to say so.
    let deaf = unheard(&store.read());
    for project in deaf {
        tracing::warn!(
            %project,
            "a channel is bound with no credential to listen with, so nothing it says \
             will be heard — which is indistinguishable from nobody saying anything"
        );
    }

    // Reached by the dashboard's route as an extension rather than as a
    // context, because a context only exists while a page is being rendered —
    // see `crate::dashboard::instance`. A layer is on every request, which is
    // both paths.
    let router = router.layer(axum::Extension(Arc::clone(&store)));

    // Outermost, and that is the whole of it: a job's tunnel serves an
    // application somebody else's agent wrote, so it must not pass through the
    // server-function and static-file machinery on its way — a path collision
    // would otherwise decide which of the two answers. Applied last because a
    // layer added last is the one that runs first. See
    // `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`.
    let router = router.layer(axum::middleware::from_fn(crate::tunnel::route));

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
    announce(runtime, serving, &kept)?;

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

/// Whether this build has no browser half anywhere.
///
/// Both halves of the question, and a named function rather than a condition
/// written where it is used, for the reason [`missing`] above is one: the two
/// sources are not interchangeable and getting the combination wrong is
/// silent. Warning when only one is absent would fire on every ordinary
/// development build, which is how a warning stops being read.
const fn clientless(carried: Option<&[u8]>, beside: Option<&Path>) -> bool {
    carried.is_none() && beside.is_none()
}

/// Every project whose channels it cannot hear replies on.
///
/// By name rather than by count, which is the whole change: an operator told
/// that two of three projects are listening still has to work out which one is
/// not. A binding with no credential to listen with is not an error and
/// produces no warning of its own — it looks exactly like a platform that has
/// sent nothing — so this is the only thing that tells the two apart.
fn unheard(state: &stageman_core::State) -> Vec<String> {
    state
        .projects
        .values()
        .filter(|project| {
            project
                .channels
                .values()
                .any(|bound| bound.listen_credential.is_none())
        })
        .map(|project| project.name.clone())
        .collect()
}

#[cfg(test)]
mod unheard_tests {
    use super::unheard;
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

    /// It names the projects that cannot be heard, and only those.
    ///
    /// By name, which is the point of it: this replaced a count, and a count
    /// told an operator that something was misconfigured without telling them
    /// what. Bound is not the same as listening — a channel with no credential
    /// to listen with is a project that speaks and is never answered, and it
    /// looks exactly like a platform that has sent nothing.
    #[test]
    fn only_a_project_that_cannot_be_heard_is_named() {
        let mut state = State::default();
        assert!(unheard(&state).is_empty());

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(1)),
            watching("heard", true),
        );
        assert!(
            unheard(&state).is_empty(),
            "a listening channel is not a problem"
        );

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(2)),
            watching("deaf", false),
        );
        assert_eq!(unheard(&state), vec!["deaf".to_owned()]);

        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(3)),
            watching("also-deaf", false),
        );
        let mut named = unheard(&state);
        named.sort();
        assert_eq!(named, vec!["also-deaf".to_owned(), "deaf".to_owned()]);
    }
}

/// Says what this is and where the dashboard is, on standard output.
///
/// Not through `tracing`, for the same reason [`report`] is not: this is what
/// somebody who typed the command is waiting to read, and a verbosity setting
/// must not be able to withhold it.
///
/// **Five facts and an address, and nothing that is merely true right now.**
/// It used to count agents, projects, listening channels, swept jobs and
/// containers left alone. Those are state at one arbitrary moment — no more
/// worth printing than the same numbers a second later — and every one of them
/// that an operator could act on is already a warning naming the thing it is
/// about, which a count never could. What is left is what does not change
/// while the process runs: what this binary is, what it needs, what opens its
/// instance, where that instance is, and what it answers to.
fn announce(
    runtime: &ContainerRuntime,
    serving: SocketAddr,
    kept: &Kept,
) -> Result<(), StartupError> {
    println!();
    println!("stageman is running.");
    // Outward from the program to this instance's data: what it is, what it
    // needs in order to work, what opens its file, and where that file is.
    println!("  version    {}", crate::release::described());
    println!("  runtime    {}", runtime.path().display());
    println!("  key        {}", kept.key);
    println!("  instance   {}", kept.file.display());
    // Last of the facts, and the one most likely to be wrong on a machine
    // this was deployed to: an instance nobody told a domain tells people to
    // look at a name that resolves to their own loopback. It fails as a wrong
    // answer rather than as an absent feature, so it is said out loud.
    println!("  domain     {}", *crate::tunnel::DOMAIN);
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

/// The directory the framework resolves a browser half from.
///
/// The same rule Dioxus applies, restated because the function that applies it
/// is private and because it panics on a directory that is not there. Asking
/// first turns "built without a client" from a crash into the ordinary state
/// of anything `cargo build` produces.
///
/// Restating somebody else's rule is drift waiting to happen, and what keeps
/// it honest is that the integration tests run this binary: a Dioxus upgrade
/// that moved the directory would fail them here rather than in production.
///
/// **It is now read for two reasons rather than one**, and that is why it is
/// named for the directory rather than for the bundle that may be in it: it
/// says where a bundle would be found, and it says where this binary must put
/// its own index so the framework will find *that* — see
/// [`materialise`] and
/// `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`. One rule,
/// read twice, so the two can never point at different directories.
fn public_directory() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DIOXUS_PUBLIC_PATH") {
        return Some(PathBuf::from(path));
    }
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|beside| beside.join("public"))
}

/// Builds the serving configuration, putting a carried index in front of it.
///
/// The whole of the arrangement in
/// `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`, in the order
/// that makes it safe: write the index where the framework looks, build the
/// configuration — which is the one call that reads it — then take it away.
///
/// With nothing carried this is the configuration on its own, which is what
/// every build that is not `just build` produces.
///
/// It takes the directory rather than resolving one, for the reason
/// [`materialise`] does: the gate compiles this binary carrying nothing, so a
/// test can only reach the interesting half by being handed somewhere to put
/// it.
///
/// # Errors
///
/// Fails if there is nowhere to derive a directory from, or the index cannot be
/// written there. Both are refusals rather than a configuration without a
/// client: a binary carrying a browser half that could not place it would serve
/// a page which renders and never responds, and the dashboard is the thing an
/// operator would otherwise be sent to fix it in.
fn configuration(
    carried: Option<&[u8]>,
    directory: Option<PathBuf>,
) -> Result<ServeConfig, StartupError> {
    let Some(index) = carried else {
        return Ok(ServeConfig::new());
    };
    let directory = directory.ok_or_else(|| StartupError::Bundle {
        path: PathBuf::new(),
        source: io::Error::other("this program has no location to derive one from"),
    })?;
    let written = materialise(&directory, index)?;
    let configured = ServeConfig::new();
    withdraw(&written);
    Ok(configured)
}

/// Puts the carried index where the framework will read it, and says where.
///
/// The framework accepts an index only as a path. The one call that takes a
/// parsed index in memory needs a type its own crate does not export, checked
/// in three releases — so this writes the file, and the caller removes it as
/// soon as the configuration has been built. It is on disk for the length of
/// one read.
///
/// Written unconditionally rather than deferring to a file already there.
/// Assets are named by a hash of their contents and this binary's index names
/// the hashes this binary carries, so an index left by some other build would
/// send a browser looking for files nothing here has. Overwriting is the safe
/// direction.
///
/// It takes the directory rather than resolving one, which is what lets a test
/// hand it somewhere harmless. Resolving is [`public_directory`]'s job and the
/// caller's to ask for, so the two questions — *where does the framework look*
/// and *can this be written there* — are answered in different places.
///
/// # Errors
///
/// Fails if the directory cannot be made or the file cannot be written.
fn materialise(directory: &Path, index: &[u8]) -> Result<PathBuf, StartupError> {
    let written = directory.join(crate::bundle::INDEX);
    let failed = |source| StartupError::Bundle {
        path: written.clone(),
        source,
    };
    fs::create_dir_all(directory).map_err(failed)?;
    fs::write(&written, modulepreloaded(index)).map_err(failed)?;
    Ok(written)
}

/// How the bundler asks a browser to preload the browser's half.
///
/// It emits this and then loads the same file with `<script type="module">`,
/// and to a browser those are two different requests: a module is fetched in
/// its own mode, so the preloaded copy never matches the one the page goes on
/// to ask for. Firefox says so — *preloaded with link preload was not used* —
/// having fetched sixty kilobytes twice to find out.
const PRELOADED: &str = r#"rel="preload" as="script""#;

/// What it should have said.
///
/// `modulepreload` is the tag whose fetch matches a module's, so the preload
/// is used rather than raced. `as` goes with it because the relationship
/// already implies the destination, and the `href` and `crossorigin` either
/// side of this are left exactly as they were.
const MODULEPRELOADED: &str = r#"rel="modulepreload""#;

/// The carried index, with the browser's half preloaded as the module it is.
///
/// A substitution over somebody else's generated file, which is worth being
/// plain about rather than burying. [`PRELOADED`] is a literal the bundler
/// writes, so a version of it spelling that differently is a version this
/// quietly stops correcting — and what comes back then is the console line it
/// was written for, on a page that still works. That asymmetry is the whole
/// argument for patching generated output here: the failure of the patch is
/// the state before it.
///
/// Everything else is left alone, deliberately. This is not an HTML rewriter
/// and must not become one — the index is the renderer's input, and a
/// transformation that reflowed it would change what the framework parses.
///
/// Bytes that are not text come back untouched rather than lossily converted.
/// An index this could not read is not one it should be editing, and a build
/// carrying no browser half has no such tag in the first place, so leaving it
/// alone is the ordinary case rather than a failure.
fn modulepreloaded(index: &[u8]) -> Vec<u8> {
    std::str::from_utf8(index).map_or_else(
        |_| index.to_vec(),
        |text| text.replace(PRELOADED, MODULEPRELOADED).into_bytes(),
    )
}

/// Removes what [`materialise`] wrote, so nothing is left on disk.
///
/// Best effort and deliberately silent. The configuration has already been
/// built by the time this runs, so a file that cannot be removed changes
/// nothing about whether the dashboard works — and the next start overwrites
/// it. Refusing to start over an undeletable temporary file would be trading a
/// working instance for tidiness.
///
/// **The directory goes too, but only if it is empty**, and `remove_dir` is
/// what makes that safe to attempt rather than something to reason about
/// first. It is `rmdir` underneath: emptiness and removal are one operation,
/// so there is no window in which a directory tested as empty gains a file
/// before it is taken away. Trying and being refused *is* the check.
///
/// That the refusal is silent is the point rather than laziness. A directory
/// with anything else in it belongs to somebody else — an operator who pointed
/// `DIOXUS_PUBLIC_PATH` at a real bundle, most obviously — and leaving it is
/// the correct outcome, not a failure to report. Only a directory this created
/// and emptied is one nothing else wants, and that one is recreated on the
/// next start anyway.
fn withdraw(written: &Path) {
    drop(fs::remove_file(written));
    if let Some(directory) = written.parent() {
        drop(fs::remove_dir(directory));
    }
}

/// Adds a route per carried file, so the bundle is served from memory.
///
/// One exact route each rather than a wildcard under a prefix. There is no
/// path to normalise and no way to ask for something outside the table, so the
/// class of bug where a crafted path escapes the directory cannot occur — it
/// is not defended against, it is absent.
///
/// The index is skipped: it is the renderer's input rather than a response,
/// and serving the unrendered template would hand back a page that boots
/// without the state the server had already put in it.
/// What each carried file is served as, and under what path.
///
/// Split from registering it so that the two decisions here — which files get
/// a route, and what each is called — can be asserted without building a
/// router or driving one. It takes the table rather than reading the one
/// compiled in, and that is the difference between a test and a tautology: the
/// gate embeds nothing, so a function reading the real table would return
/// nothing and agree with every mutation of itself.
fn routed(carried: crate::bundle::Bundle) -> Vec<(String, &'static str, &'static [u8])> {
    carried
        .entries()
        .iter()
        .filter(|(served, _)| *served != crate::bundle::INDEX)
        .map(|(served, bytes)| {
            (
                format!("/{served}"),
                crate::bundle::content_type(served),
                *bytes,
            )
        })
        .collect()
}

/// Registers one route per entry [`routed`] names.
///
/// Skipped by mutation testing, and untestable rather than untested: what it
/// does beyond `routed` is one framework call per entry, and reaching it needs
/// a binary with a bundle compiled into it — which the gate never builds,
/// because the directory it would come from is empty in a fresh clone. What
/// can be checked is checked, one function up.
#[mutants::skip]
fn serving_embedded(mut router: axum::Router) -> axum::Router {
    for (path, kind, body) in routed(crate::bundle::CARRIED) {
        router = router.route(
            &path,
            axum::routing::get(move || async move {
                ([(axum::http::header::CONTENT_TYPE, kind)], body)
            }),
        );
    }
    router
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

    /// A browser half from either source is a browser half.
    ///
    /// All four combinations, because the failure is one-sided in both
    /// directions: a condition that warned when either was absent would fire
    /// on every development build until nobody read it, and one that warned
    /// when neither was would never fire at all.
    #[test]
    fn only_a_build_with_no_browser_half_anywhere_is_clientless() {
        use super::clientless;
        use std::path::Path;

        let beside = Path::new("/somewhere/public");

        assert!(clientless(None, None), "nothing carried and nothing beside");
        assert!(
            !clientless(Some(b"<html></html>"), None),
            "carried is enough"
        );
        assert!(!clientless(None, Some(beside)), "beside is enough");
        assert!(!clientless(Some(b"<html></html>"), Some(beside)));
    }

    /// Both spellings ask, and nothing else does.
    ///
    /// Every part of this is worth asserting: that either spelling is enough,
    /// that both are required to *match* rather than to coincide, and that an
    /// ordinary argument is not mistaken for one. A binary that read `--help`
    /// as a version request would exit instead of serving.
    #[test]
    fn only_the_two_spellings_ask_what_it_is() {
        use super::asked_what_it_is;

        let given =
            |arguments: &[&str]| asked_what_it_is(arguments.iter().map(|a| (*a).to_owned()));

        assert!(given(&["--version"]));
        assert!(given(&["-V"]));
        assert!(given(&["--serve", "--version"]), "position does not matter");

        assert!(!given(&[]), "no arguments is not a question");
        assert!(!given(&["--help"]));
        assert!(!given(&["-v"]), "lowercase is not the flag");
        assert!(!given(&["--versions"]));
        assert!(!given(&["version"]));
    }

    /// Nothing of the index survives, the directory it needed included.
    ///
    /// The directory is the half that was missed first time round: the file
    /// went and an empty `public/` stayed beside the binary, which is exactly
    /// the residue this whole arrangement exists to avoid.
    #[test]
    fn withdrawing_takes_the_directory_it_needed_with_it() {
        use super::{materialise, withdraw};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");

        let written = materialise(&at, b"<html></html>").expect("it is writable");
        assert!(at.is_dir(), "it should have made the directory");

        withdraw(&written);

        assert!(
            !written.exists(),
            "the index outlived the read it existed for"
        );
        assert!(
            !at.exists(),
            "an empty directory was left beside the binary"
        );
    }

    /// A directory holding anything else is left exactly as it was.
    ///
    /// The case an operator creates by pointing `DIOXUS_PUBLIC_PATH` at a real
    /// bundle. Removing it would take their files with it, so being refused is
    /// the outcome asked for rather than an error — which is why nothing here
    /// reports one.
    #[test]
    fn a_directory_holding_anything_else_survives() {
        use super::{materialise, withdraw};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");
        std::fs::create_dir_all(&at).expect("the directory is made");
        let theirs = at.join("something-else.js");
        std::fs::write(&theirs, "not ours").expect("their file is written");

        let written = materialise(&at, b"<html></html>").expect("it is writable");
        withdraw(&written);

        assert!(!written.exists(), "ours should still be removed");
        assert!(at.is_dir(), "their directory should have survived");
        assert_eq!(
            std::fs::read_to_string(&theirs).expect("their file is still there"),
            "not ours",
        );
    }

    /// A carried index is written, read, and taken away again.
    ///
    /// Asserted through a sentinel rather than by looking at what comes back,
    /// because the configuration is opaque: a stale index is put there first,
    /// and what proves the whole sequence ran is that it is gone afterwards.
    /// Nothing else could have removed it.
    #[test]
    fn a_carried_index_is_written_read_and_taken_away() {
        use super::configuration;

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");
        std::fs::create_dir_all(&at).expect("the directory is made");
        let stale = at.join("index.html");
        std::fs::write(&stale, "STALE").expect("the sentinel is written");

        configuration(Some(b"<html>ours</html>"), Some(at)).expect("it is configurable");

        assert!(
            !stale.exists(),
            "the index should have been overwritten and then removed",
        );
    }

    /// A binary carrying nothing touches nothing.
    ///
    /// The other half, and the one that keeps `just dev` working: a build with
    /// no bundle of its own must leave a directory that has one alone.
    #[test]
    fn carrying_nothing_leaves_the_directory_alone() {
        use super::configuration;

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");
        std::fs::create_dir_all(&at).expect("the directory is made");
        let theirs = at.join("index.html");
        std::fs::write(&theirs, "THEIRS").expect("the sentinel is written");

        configuration(None, Some(at)).expect("it is configurable");

        assert_eq!(
            std::fs::read_to_string(&theirs).expect("it is still there"),
            "THEIRS",
            "a build carrying nothing must not disturb a bundle beside it",
        );
    }

    /// The index is written where it was asked for, with what it was given.
    ///
    /// The whole of what the framework needs from us, and the one step that
    /// touches a disk — so it is worth asserting rather than assuming, and
    /// asserting somewhere harmless rather than beside a real binary.
    #[test]
    fn the_index_is_written_where_the_framework_will_look() {
        use super::materialise;

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let nested = elsewhere.path().join("public");

        let written = materialise(&nested, b"<html>carried</html>").expect("it is writable");

        assert_eq!(written, nested.join("index.html"));
        assert_eq!(
            std::fs::read_to_string(&written).expect("it is there"),
            "<html>carried</html>",
            "the framework would parse whatever this wrote",
        );
    }

    /// The browser's half is preloaded as the module the page then loads.
    ///
    /// Written against the tag the bundler actually emits rather than a
    /// paraphrase of it, because a paraphrase would keep passing after the
    /// real one changed shape — and the substitution missing is silent by
    /// design, so this test is the only thing that would say so.
    #[test]
    fn a_module_is_preloaded_as_a_module() {
        use super::modulepreloaded;

        let emitted = concat!(
            r#"<link rel="preload" as="script" href="/./assets/x-dxh0.js" crossorigin>"#,
            r#"<script type="module" async src="/./assets/x-dxh0.js"></script>"#,
        );

        let corrected =
            String::from_utf8(modulepreloaded(emitted.as_bytes())).expect("text in, text out");

        assert_eq!(
            corrected,
            concat!(
                r#"<link rel="modulepreload" href="/./assets/x-dxh0.js" crossorigin>"#,
                r#"<script type="module" async src="/./assets/x-dxh0.js"></script>"#,
            ),
            "the link should preload a module, and nothing else should move",
        );
    }

    /// An index with nothing to correct is passed through untouched.
    ///
    /// Two of them, and they are different failures rather than the same one
    /// twice: a build carrying no browser half has no such tag, and bytes that
    /// are not text are not an index this has any business editing. Both must
    /// arrive at the framework exactly as they left.
    #[test]
    fn an_index_with_no_such_tag_is_left_alone() {
        use super::modulepreloaded;

        assert_eq!(
            modulepreloaded(b"<html>no client here</html>"),
            b"<html>no client here</html>",
        );
        assert_eq!(
            modulepreloaded(&[0xff, 0xfe, 0x00]),
            &[0xff, 0xfe, 0x00],
            "bytes that are not text are not rewritten lossily",
        );
    }

    /// A directory that cannot be written is a refusal, not an empty success.
    ///
    /// The failure this guards is the expensive one: a binary carrying a
    /// browser half that quietly serves a page which renders and never
    /// responds. Provoked with a path under a file, which cannot be a
    /// directory on any platform.
    #[test]
    fn an_index_that_cannot_be_written_is_a_failure() {
        use super::{StartupError, materialise};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let blocked = elsewhere.path().join("a-file");
        std::fs::write(&blocked, "not a directory").expect("the file is written");

        let refused = materialise(&blocked.join("public"), b"<html></html>");

        assert!(
            matches!(refused, Err(StartupError::Bundle { .. })),
            "expected a refusal, got {refused:?}",
        );
    }

    /// What was written is taken away again.
    ///
    /// The point of writing it at all is that it does not stay, so this is the
    /// assertion that the whole arrangement rests on.
    #[test]
    fn what_was_written_does_not_stay() {
        use super::{materialise, withdraw};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let written = materialise(elsewhere.path(), b"<html></html>").expect("it is writable");
        assert!(written.exists());

        withdraw(&written);

        assert!(
            !written.exists(),
            "the index outlived the read it existed for"
        );
        // Removing what is already gone is the outcome asked for rather than a
        // failure, and a second start must not trip over the first.
        withdraw(&written);
    }

    /// The index gets no route, and everything else gets one under its path.
    ///
    /// Vacuous when nothing is embedded, which is the gate's case — the table
    /// is a compile-time input, so no test can put anything in it. What this
    /// catches is the filter inverting, which would serve the unrendered
    /// template and nothing else.
    #[test]
    fn the_index_is_not_routed_and_the_rest_are() {
        use super::routed;

        // A table of this shape rather than the one compiled in. The gate
        // embeds nothing, so asserting against the real table would assert
        // that nothing maps to nothing.
        let carrying = crate::bundle::Bundle::of(&[
            ("index.html", b"<html></html>"),
            ("assets/app-abc.js", b"console.log(1)"),
            ("assets/app-abc.wasm", b"\0asm"),
        ]);

        let routes = routed(carrying);

        assert_eq!(
            routes.len(),
            2,
            "the index must not get a route: {routes:?}"
        );
        let paths: Vec<&str> = routes.iter().map(|(path, ..)| path.as_str()).collect();
        assert!(paths.contains(&"/assets/app-abc.js"), "{paths:?}");
        assert!(paths.contains(&"/assets/app-abc.wasm"), "{paths:?}");
        assert!(!paths.contains(&"/index.html"), "{paths:?}");

        let wasm = routes
            .iter()
            .find(|(path, ..)| path == "/assets/app-abc.wasm")
            .expect("the wasm is routed");
        assert_eq!(
            wasm.1, "application/wasm",
            "a browser refuses a module served as anything else",
        );
        assert_eq!(wasm.2, b"\0asm", "the route serves the bytes it was given");
    }

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
