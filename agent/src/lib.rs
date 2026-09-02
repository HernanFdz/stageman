//! The contract every coding agent is driven through, and the adapters that
//! implement it.
//!
//! Two shapes of use and one contract. A one-shot structured query is how the
//! foreman thinks; a long-running session in a workspace is how a job
//! works. Both reach a model only by running a configured agent, never through
//! a vendor's own service API — see
//! `docs/decisions/0007-model-work-goes-through-an-agent-cli.md` for why that
//! is a hard rule rather than a preference.
//!
//! Nothing outside an adapter may be specific to one agent. A change that
//! would make the contract fit one vendor more comfortably is the thing this
//! crate exists to catch — see `docs/decisions/0006-agents-are-pluggable.md`,
//! and note that the same record explains why this abstraction was refused
//! until now.
//!
//! The shape that contract takes was settled by a spike rather than by
//! argument; `docs/decisions/0010-acp-is-the-agent-contract.md` records the
//! choice, and rather more usefully, the evidence that outlives it.
//!
//! **Every agent is reached the same way: a container is started, and the
//! protocol is spoken over its standard input and output.** There is no other
//! path, and no host-installed program to find — see
//! `docs/decisions/0012-agents-run-in-containers.md`. The protocol's own
//! vocabulary comes from its Rust library while the container process is
//! started here, which is
//! `docs/decisions/0014-the-protocols-own-sdk-and-our-own-spawning.md` and is
//! the reason a container's arguments are a value this crate builds rather
//! than a command line it hands to somebody else.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    ContentBlock, HttpHeader, InitializeRequest, InitializeResponse, ListSessionsRequest,
    LoadSessionRequest, McpServer, McpServerHttp, NewSessionRequest, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionId, SessionNotification, SessionUpdate, TextContent,
};
use agent_client_protocol::{ByteStreams, Client, ConnectionTo};
use parking_lot::Mutex;
#[cfg(test)]
use stageman_core::Channel;
use stageman_core::{Agent, Handout, Platform, Role, Secret};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};

// The protocol library calls the far end of a connection `Agent` too, and it
// means the role rather than the product. Importing it under another name is
// not decoration: `ConnectionTo<Agent>` would compile against either type and
// read correctly whichever one it resolved to, which is precisely the kind of
// ambiguity `docs/conventions.md` §2 says to spend a word avoiding.
use agent_client_protocol::Agent as AgentRole;

/// Re-exported because they appear in this crate's own signatures.
///
/// `docs/decisions/0014-the-protocols-own-sdk-and-our-own-spawning.md` accepted
/// that protocol types would surface here rather than being wrapped. Accepting
/// that and then not re-exporting them would leave [`Greeting`] and [`Answer`]
/// unreadable to anyone who has not also taken the protocol library as a direct
/// dependency, which is the cost without the benefit.
pub use agent_client_protocol::schema::ProtocolVersion;
pub use agent_client_protocol::schema::v1::StopReason;

/// How much of a failed container's standard error is kept.
///
/// Bounded because the message travels into an error type an operator reads,
/// and an agent that fails by printing megabytes would otherwise turn one
/// unreadable failure into a second one. Kept is not the same as read: see
/// [`printed`], which reads past this and discards the rest.
const STDERR_LIMIT: usize = 8 * 1024;

/// Where a container runtime is looked for, in order.
///
/// Absolute paths and never a search of `PATH`, which is the whole of what
/// `docs/conventions.md` §3 forbids: an inherited variable differs between the
/// shell you tested in and what a service manager supplies, and a list
/// compiled in does not. Deterministic in the same way on every machine is the
/// property being bought.
///
/// Ordered, and the order is a decision rather than an accident. Docker first
/// because it is what most machines that have anything have; the package
/// manager locations before the system ones on each platform, because a
/// hand-installed runtime is the one somebody chose. A machine with both gets
/// the first, and that is the cost of not asking — see
/// `docs/decisions/0023-the-container-runtime-is-discovered-once.md`.
#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/Applications/Docker.app/Contents/Resources/bin/docker",
    "/opt/homebrew/bin/podman",
    "/usr/local/bin/podman",
];

/// Where a container runtime is looked for, in order.
///
/// See the macOS list above for why this is a list of absolute paths.
#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    "/usr/bin/docker",
    "/usr/local/bin/docker",
    "/snap/bin/docker",
    "/usr/bin/podman",
    "/usr/local/bin/podman",
];

/// Where a container runtime is looked for, in order.
///
/// See the macOS list above for why this is a list of absolute paths.
#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    r"C:\Program Files\Docker\Docker\resources\bin\docker.exe",
    r"C:\Program Files\RedHat\Podman\podman.exe",
];

/// Nothing is known about where a runtime lives on this platform.
///
/// An empty list rather than a compile error, so that a platform nobody has
/// tried still builds and fails honestly at startup with "none found" — which
/// is a message somebody can act on, unlike a build that will not finish.
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const CANDIDATES: &[&str] = &[];

/// The first of [`candidates`] that is a file, as a runtime.
///
/// The whole of discovery, and deliberately a pure function of the list rather
/// than a lazy static reading a fixed one. The static lives in the binary,
/// because *what to do when there is none* is a decision about a program that
/// cannot run rather than knowledge about container runtimes — and because a
/// function taking its list can be tested for the absence, which a static
/// reading the real list on a machine that has Docker never can.
///
/// # Examples
///
/// ```
/// use stageman_agent::first_present;
///
/// assert!(first_present(&["/nowhere/at/all/docker"]).is_none());
/// ```
#[must_use]
pub fn first_present(candidates: &[&str]) -> Option<ContainerRuntime> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
        .map(ContainerRuntime::new)
}

/// Every place a runtime was looked for, for a message that has to say.
#[must_use]
pub const fn candidates() -> &'static [&'static str] {
    CANDIDATES
}

/// Where the container runtime lives.
///
/// A located path, never a name to be searched for. Of the two agents
/// installed while this was being designed, one sat in a directory absent from
/// a non-interactive shell's `PATH`, so anything a daemon finds by searching
/// works when tested by hand and fails under a service manager — the rule and
/// its measurement are in `docs/conventions.md` §3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRuntime(PathBuf);

impl ContainerRuntime {
    /// Names the runtime by the path it was configured with.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// The path this runtime was configured with.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Proves the recorded runtime is there and answers.
    ///
    /// The startup check `docs/conventions.md` §3 asks for. A missing runtime
    /// is the kind of failure that makes an instance unusable rather than one
    /// job, so it belongs at startup: the worst moment to discover it is three
    /// in the morning on the first signal that mattered.
    ///
    /// It asks for a version rather than merely testing that the file is
    /// there, because on the runtimes this targets that reaches the daemon —
    /// so a client installed without a daemon running, which looks perfectly
    /// healthy to any check of the filesystem, fails here instead of on the
    /// first job.
    ///
    /// # Errors
    ///
    /// Fails if the path cannot be run at all, or if it runs and reports
    /// failure. The two are separate variants because one means the path is
    /// wrong and the other means the runtime is not working, and an operator
    /// does something different about each.
    pub async fn verify(&self) -> Result<(), AgentError> {
        let reported = tokio::process::Command::new(self.path())
            .arg("version")
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|source| AgentError::Runtime {
                path: self.0.clone(),
                source,
            })?;

        if reported.status.success() {
            return Ok(());
        }
        Err(AgentError::Unusable {
            path: self.0.clone(),
            message: String::from_utf8_lossy(&reported.stderr)
                .trim()
                .chars()
                .take(STDERR_LIMIT)
                .collect(),
        })
    }
}

/// The recipe an agent's image is built from.
///
/// Compiled in, which is the whole of
/// `docs/decisions/0035-an-image-is-built-never-named.md`: what a host needs
/// is a container runtime, and a recipe that ships beside the binary is a
/// second thing to carry and a thing that can be older than the code driving
/// it. It stays a file rather than a string literal so it keeps its comments
/// and its diffs — `include_str!` is what makes it part of the artifact.
///
/// Adapter knowledge, for the same reason the agent set is closed: an image is
/// code.
const fn recipe(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => include_str!("../images/claude/Dockerfile"),
    }
}

/// Which stage of a recipe one role's image is built from.
///
/// The two names are the recipe's and this crate's at once, which is the one
/// place they have to agree; a test below builds both so that renaming a stage
/// in the recipe alone cannot pass. See
/// `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md` for why there are
/// two at all.
const fn stage(role: Role) -> &'static str {
    match role {
        Role::Foreman => "thinking",
        Role::Job => "working",
    }
}

/// An image, named by its content and by nothing else.
///
/// There is no tag, which is the point rather than an omission: an image an
/// operator could name is one they could name wrongly, and an image *nobody*
/// can name cannot be stale, cannot be confused with another instance's, and
/// needs no agreement between this crate and anything that builds it.
///
/// Opaque on purpose. What is inside is whatever the runtime answered with —
/// a digest on one, a bare identifier on the other — and the only thing this
/// project does with it is start a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image(String);

impl Image {
    /// The identifier, as the runtime wants it on a command line.
    #[must_use]
    pub fn as_argument(&self) -> &str {
        &self.0
    }
}

/// How much of a failed build is kept, in lines counted from the end.
///
/// From the end rather than the beginning, which is the opposite of
/// [`printed`] and deliberate: a container that fails says so immediately, and
/// a build that fails says so after however many layers succeeded first.
const BUILD_TAIL: usize = 12;

/// The arguments that build one stage of a recipe, reading it from standard
/// input.
///
/// A build with no context at all — the trailing `-` — because the recipe is
/// the only input there is. `docs/decisions/0034-tools-are-served-not-shipped.md`
/// is what keeps that true: nothing this project writes goes in the image, so
/// there is nothing a context could carry.
///
/// Quiet, so that the identifier is the whole of standard output and can be
/// taken as the answer. What a build has to say about itself still arrives on
/// standard error, which is where the failure message below comes from.
const fn build_arguments(role: Role) -> [&'static str; 5] {
    ["build", "--quiet", "--target", stage(role), "-"]
}

/// Builds the image one container will run, and answers with its identity.
///
/// Run in front of every container rather than once and remembered. A cached
/// rebuild costs about a second and reaches no network, and it compares the
/// recipe's instructions rather than a name — so it is a freshness check that
/// an existence check could not be, and the runtime's own layer cache is a
/// better memo than this process could keep: it survives a restart, and it
/// notices an edit.
///
/// # Errors
///
/// Fails if the runtime cannot be started, or if the build itself does not
/// finish — no network on a machine that has never built this, most often,
/// which is why [`AgentError::Build`] carries what the build said last rather
/// than what it said first.
pub async fn build(
    runtime: &ContainerRuntime,
    agent: Agent,
    role: Role,
) -> Result<Image, AgentError> {
    let mut building = tokio::process::Command::new(runtime.path())
        .args(build_arguments(role))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    let Some(mut writing) = building.stdin.take() else {
        return Err(AgentError::NoChannel);
    };
    // Written whole and then closed, which is safe only because a recipe is a
    // few kilobytes and a pipe buffer is tens of them. A larger one would have
    // to be written while the output is drained, for the reason [`greet`]
    // drains standard error while the exchange happens.
    writing
        .write_all(recipe(agent).as_bytes())
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;
    // A build does not begin until it has the whole recipe, and it learns that
    // from end-of-file. Dropping the handle is what sends one.
    drop(writing);

    let finished = building
        .wait_with_output()
        .await
        .map_err(AgentError::Exit)?;

    outcome(
        finished.status.success(),
        &finished.stdout,
        &finished.stderr,
    )
}

/// What a finished build means.
///
/// Split from [`build`] so that both outcomes can be asserted without a
/// container runtime, which puts it on the same seam
/// [`handshake_arguments`] is on the other side of: running a process is the
/// part a test cannot afford, and deciding what its result meant is the part
/// worth checking.
///
/// Mutation testing is what found this. With the decision inline, a build that
/// failed could have been reported as an image — and the reverse — with every
/// test still green, because the only cases exercising it were `#[ignore]`d.
fn outcome(succeeded: bool, answered: &[u8], complained: &[u8]) -> Result<Image, AgentError> {
    if succeeded {
        Ok(Image(String::from_utf8_lossy(answered).trim().to_owned()))
    } else {
        Err(AgentError::Build {
            message: last_words(complained),
        })
    }
}

/// The end of what a failed build said, as one line.
///
/// By lines rather than by bytes, which is what keeps this off the panic
/// lints: taking the last *n* characters of a string means slicing it at an
/// index nothing guarantees is a character boundary, and the escape from that
/// is exactly the kind `.quality/gate-reference.md` forbids. Lines are already
/// whole.
fn last_words(said: &[u8]) -> String {
    let text = String::from_utf8_lossy(said);
    let mut kept: Vec<&str> = text
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(BUILD_TAIL)
        .collect();
    kept.reverse();
    if kept.is_empty() {
        "it said nothing".to_owned()
    } else {
        kept.join("; ")
    }
}

/// The arguments that start a container just long enough to be greeted.
///
/// Pure, so what a container is started with can be asserted without starting
/// one. No network and no workspace: nothing before the first prompt needs
/// either, and a check that reaches the internet is a check that fails for
/// reasons unrelated to what it tests.
fn handshake_arguments(image: &Image) -> [&str; 6] {
    [
        "run",
        // Leaves nothing behind once the container exits, which is half of the
        // bar in `docs/conventions.md` §4 and the cheap half.
        "--rm",
        // The protocol channel *is* this stream. Without it the container gets
        // no standard input and the agent sees end-of-file immediately.
        "--interactive",
        "--network",
        "none",
        image.as_argument(),
    ]
}

/// What an agent's container said about itself when the connection opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
    /// The protocol version the two ends settled on.
    pub protocol_version: ProtocolVersion,
    /// What answered inside the image, when it said.
    ///
    /// Optional because the protocol still permits an agent to stay quiet
    /// about its own identity. An adapter that does is usable and merely
    /// harder to support, so this is not an error.
    pub adapter: Option<Adapter>,
}

/// The program that answered the protocol inside an image.
///
/// Not the agent — the adapter in front of it, which is what versions
/// independently and what
/// `docs/decisions/0010-acp-is-the-agent-contract.md` warns can lag the agent
/// it speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// What the adapter calls itself.
    pub name: String,
    /// The version it reported.
    pub version: String,
}

impl From<InitializeResponse> for Greeting {
    fn from(response: InitializeResponse) -> Self {
        Self {
            protocol_version: response.protocol_version,
            adapter: response.agent_info.map(|info| Adapter {
                name: info.name,
                version: info.version,
            }),
        }
    }
}

/// An agent in a container could not be reached, or would not answer.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// The container runtime itself could not be started.
    ///
    /// The one failure that makes an instance unusable rather than one job:
    /// nothing here runs without a runtime, so `docs/conventions.md` §3 puts
    /// this in the category that fails at startup rather than in the dashboard.
    #[error("the container runtime at {path} could not be started")]
    Runtime {
        /// Where the runtime was expected.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: io::Error,
    },
    /// The runtime ran and reported that it is not working.
    ///
    /// Distinct from [`AgentError::Runtime`], which means the path is wrong.
    /// A client installed with no daemon behind it passes every test of the
    /// filesystem and fails here, and telling an operator their path is wrong
    /// when their daemon is merely stopped sends them to fix the wrong thing.
    #[error("the container runtime at {path} is not working: {message}")]
    Unusable {
        /// Where the runtime was found.
        path: PathBuf,
        /// What it said about itself, truncated.
        message: String,
    },
    /// The image could not be built from the recipe compiled in here.
    ///
    /// New with `docs/decisions/0035-an-image-is-built-never-named.md`, and it
    /// replaces a failure that used to arrive as [`AgentError::Container`]:
    /// an image nobody had built. That one is now impossible, because a build
    /// runs in front of every container — so what is left is a build that
    /// could not finish, which is a different thing with a different fix. The
    /// first one an operator with no network will meet.
    #[error("the agent's image could not be built: {message}")]
    Build {
        /// The last of what the build said, which is where a build says why.
        message: String,
    },
    /// The runtime started without giving the streams the protocol needs.
    #[error("the container runtime offered no channel to speak the protocol over")]
    NoChannel,
    /// The container itself failed, before or instead of speaking.
    ///
    /// Separate from a protocol failure on purpose. A missing image and a
    /// broken adapter both surface as silence on the connection, and telling
    /// an operator their agent does not speak the protocol when the truth is
    /// that the image was never built is the kind of wrong answer that costs
    /// an evening.
    #[error("the container exited without completing the handshake: {status}{message}")]
    Container {
        /// How the container ended.
        status: String,
        /// What it printed, if anything — prefixed and truncated, or empty.
        message: String,
    },
    /// The container was asked to continue a session it does not have.
    ///
    /// Nothing is written until something is said, so a container stopped
    /// before its agent spoke holds no session to load. Separate from a
    /// protocol failure because the container is fine and the work is simply
    /// not there to continue — starting over is the answer, and pretending to
    /// resume into an empty context would be the worst of the three.
    #[error("that container holds no session to continue")]
    NothingToResume,
    /// The container ran and spoke, but the exchange did not complete.
    #[error("the agent did not complete the protocol handshake")]
    Protocol(#[source] agent_client_protocol::Error),
    /// The container could not be waited on once the exchange was over.
    #[error("the container could not be waited on")]
    Exit(#[source] io::Error),
}

/// Starts a container for `agent` and completes the protocol handshake.
///
/// The cheapest proof that a runtime, an image, an adapter and the stdio
/// channel are all intact, and it needs no credential and no network — the
/// credential boundary sits at the first prompt rather than at the handshake,
/// which `docs/decisions/0014-the-protocols-own-sdk-and-our-own-spawning.md`
/// records as measured rather than assumed.
///
/// The container is started, greeted and shut down. Nothing survives the call.
///
/// # Errors
///
/// Fails if the runtime cannot be started, if the image cannot be built, if
/// the container exits without speaking, or if the exchange itself does not
/// complete. Each is a separate variant because an operator acts on them
/// differently — and since
/// `docs/decisions/0035-an-image-is-built-never-named.md` the second of those
/// has taken over from what used to be the commonest cause of the third.
pub async fn handshake(
    runtime: &ContainerRuntime,
    agent: Agent,
    role: Role,
) -> Result<Greeting, AgentError> {
    let image = build(runtime, agent, role).await?;
    greet(runtime, &handshake_arguments(&image)).await
}

/// Runs the runtime with `arguments` and greets whatever answers.
///
/// Split from [`handshake`] only so that the failure paths can be reached from
/// a test: what a container is started with is the difference between one that
/// greets and one that never starts, and taking the arguments here is what
/// lets a test ask for the second without the agent set having to contain a
/// deliberately broken member.
async fn greet(runtime: &ContainerRuntime, arguments: &[&str]) -> Result<Greeting, AgentError> {
    let mut container = tokio::process::Command::new(runtime.path())
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // So that a dropped future does not leave a container attached to a
        // parent that has stopped reading it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    let (Some(to_agent), Some(from_agent), Some(complaints)) = (
        container.stdin.take(),
        container.stdout.take(),
        container.stderr.take(),
    ) else {
        return Err(AgentError::NoChannel);
    };

    // Jointly, and that is load-bearing rather than tidy. Standard error is a
    // pipe with a small kernel buffer, so an agent chatty enough to fill it
    // blocks on the write and never reaches its own exit — and a container that
    // never exits is one this function would wait on forever. Draining while
    // the exchange happens is what stops a talkative agent from becoming a
    // hang. The exchange finishing drops the transport, which closes the
    // agent's standard input, which is what ends both of these in turn.
    let (spoken, printed) = futures::future::join(
        Client.builder().connect_with(
            ByteStreams::new(to_agent.compat_write(), from_agent.compat()),
            async |connection: ConnectionTo<AgentRole>| {
                connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await
                    .map(Greeting::from)
            },
        ),
        printed(complaints),
    )
    .await;

    // Reached in both outcomes: the reaping that makes "nothing survives the
    // call" true rather than merely intended.
    let status = container.wait().await.map_err(AgentError::Exit)?;

    match spoken {
        Ok(greeting) => Ok(greeting),
        // A container that failed on its own terms explains itself better than
        // the protocol error its silence produced, so it wins when both exist.
        Err(_) if !status.success() => Err(AgentError::Container {
            status: status.to_string(),
            message: printed,
        }),
        Err(protocol) => Err(AgentError::Protocol(protocol)),
    }
}

/// Whatever a container printed, bounded and prefixed for a message.
///
/// Runs on every path rather than only on failure, because its other job is to
/// keep the pipe empty; the result is discarded when there is nothing to
/// explain.
///
/// Deliberately infallible: where this is read from, there is already a failure
/// to report, and losing it in order to report a second one about reading
/// standard error would be a straight downgrade.
async fn printed(mut stderr: tokio::process::ChildStderr) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];

    // Reads to end-of-file even once it has stopped keeping anything. Stopping
    // at the limit instead would leave the pipe to fill, and a container
    // blocked writing to a pipe nobody drains is exactly the hang the joined
    // drain above exists to prevent — the bound would have quietly reinstated
    // it for anything that printed more than this.
    while let Ok(read) = stderr.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        let Some(room) = STDERR_LIMIT.checked_sub(kept.len()) else {
            continue;
        };
        if let Some(head) = chunk.get(..room.min(read)) {
            kept.extend_from_slice(head);
        }
    }

    let said = String::from_utf8_lossy(&kept);
    let trimmed = said.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(" — {trimmed}")
    }
}

/// Where a job's agent works inside its container.
///
/// A directory rather than a mount: nothing delivers a repository, and an
/// agent that needs one clones it here — see
/// `docs/decisions/0016-the-agent-clones-the-repository.md`. The image already
/// makes this its working directory.
const WORKSPACE: &str = "/workspace";

/// The variables one agent's container is started with, and their values.
///
/// This is *delivery*, and the counterpart to the deciding that
/// [`stageman_core::Handout`] does. Which credentials a process may see is a
/// pure question about configuration and lives in the domain crate; what they
/// are called here is knowledge about one agent and lives in its adapter. See
/// `docs/conventions.md` §3.
fn variables(handout: &Handout) -> Vec<(&'static str, Secret)> {
    let mut set = vec![match handout.agent() {
        Agent::Claude => (
            claude_credential_variable(handout.agent_credential()),
            handout.agent_credential().clone(),
        ),
    }];

    for (platform, credential) in handout.platforms() {
        set.push((
            match platform {
                // What the platform's own command-line tool reads, which is how
                // a job reaches it at all — see
                // `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`.
                Platform::GitHub => "GH_TOKEN",
            },
            credential.clone(),
        ));
    }

    // A channel's credential is deliberately absent. It used to travel here,
    // because a program in the container posted with it;
    // `docs/decisions/0034-tools-are-served-not-shipped.md` moved speaking to
    // a tool the instance serves, so the daemon posts and the container has no
    // use for one. That is worth more than tidiness: a job's agent can be
    // talked into sending what it holds somewhere, and the narrowest version
    // of that risk is holding less — which is the mitigation
    // `docs/open-questions.md` is still weighing for the credentials a job
    // does need.

    set
}

/// Which variable this agent's credential belongs in.
///
/// Two exist and they are not interchangeable, which was measured rather than
/// assumed: an OAuth token placed in the API-key variable does not fail, it
/// *hangs* — no error, no refusal, just a turn that never ends. A wrong answer
/// that announces itself is cheap; this one costs however long you wait before
/// suspecting the variable name.
///
/// Sniffing the prefix rather than asking an operator which kind they have:
/// the prefix is unambiguous, and
/// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md` already
/// asks them for a credential on first run, where a second question about its
/// species is friction with no better answer behind it.
fn claude_credential_variable(credential: &Secret) -> &'static str {
    if credential.expose().starts_with("sk-ant-oat") {
        "CLAUDE_CODE_OAUTH_TOKEN"
    } else {
        "ANTHROPIC_API_KEY"
    }
}

/// The arguments that start a container able to reach a model.
///
/// Pure, so what a container is started with can be asserted without starting
/// one — and this is the argument list that carries credentials, so being able
/// to assert it cheaply is the point rather than a convenience.
///
/// Each variable is named but not valued here. `--env NAME` tells the runtime
/// to forward that variable from this process, so the secret travels through an
/// environment rather than through a command line, and never appears in the
/// process table where any user on the machine can read it.
///
/// It takes the list rather than deciding it, so the names forwarded and the
/// values set are the same list by construction. Deciding it twice would let
/// the two drift apart, and a runtime told to forward a variable that is not
/// set says nothing at all — leaving a job that cannot authenticate and no line
/// anywhere explaining why.
fn session_arguments(image: &Image, delivering: &[(&'static str, Secret)]) -> Vec<String> {
    let mut arguments = vec![
        "run".to_owned(),
        "--rm".to_owned(),
        "--interactive".to_owned(),
    ];
    arguments.extend(carrying(image, delivering));
    arguments
}

/// What every container is given, whichever subcommand makes it.
///
/// The tail both argument lists end with, shared rather than assembled twice
/// — the previous shape built one list and edited it into the other by
/// removing a flag and splicing at an index, which the gate is right to call a
/// panic waiting for somebody to reorder the head.
fn carrying(image: &Image, delivering: &[(&'static str, Secret)]) -> Vec<String> {
    let mut arguments = Vec::new();
    for (name, _) in delivering {
        arguments.push("--env".to_owned());
        arguments.push((*name).to_owned());
    }
    // Deliberately no `--network none` here, unlike the handshake: reaching a
    // model needs the network, and so does cloning. Which hosts it *ought* to
    // reach is the egress allowlist still open in `docs/open-questions.md`.
    arguments.push(image.as_argument().to_owned());
    arguments
}

/// What an agent said in reply to one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// Everything the agent said, in order.
    ///
    /// Its message text only. Its private reasoning and its tool calls arrive
    /// on the same stream and are deliberately dropped: this is the answer, not
    /// a transcript, and a caller that wants the working needs a different
    /// shape rather than a fuller string.
    pub text: String,
    /// Why the turn ended.
    ///
    /// Carried rather than collapsed into success, because an answer truncated
    /// by a token limit and one the agent finished are both text and only this
    /// tells them apart.
    pub stop_reason: StopReason,
}

/// Puts one question to an agent and returns what it says.
///
/// The one-shot shape of the contract in `docs/architecture.md` §1 — how the
/// foreman thinks, rather than how a job works. The container is started,
/// asked and destroyed.
///
/// **It starts a container per question, and
/// `docs/decisions/0012-agents-run-in-containers.md` says the foreman's
/// agent should run in one long-lived one.** That is not an oversight and not
/// yet a violation, since nothing calls this yet; reusing a container means a
/// connection outliving a single call, which is machinery this does not build.
/// Tracked in `docs/open-questions.md`.
///
/// # Errors
///
/// Fails if the runtime cannot be started, if the container exits without
/// speaking — a missing image, most often — or if the exchange does not
/// complete. An agent that authenticates badly fails here as a protocol error
/// or as a turn that never ends, which is why choosing the right variable to
/// deliver its credential in is this adapter's problem and not an operator's.
pub async fn ask(
    runtime: &ContainerRuntime,
    handout: &Handout,
    tools: Option<&Tools>,
    question: &str,
) -> Result<Answer, AgentError> {
    let delivering = variables(handout);
    let image = build(runtime, handout.agent(), handout.role()).await?;
    let container = spawn(
        runtime,
        &session_arguments(&image, &delivering),
        &delivering,
    )?;
    converse(container, Opening::Fresh, tools, question).await
}

/// Where an agent reaches the tools this instance serves, and what it presents.
///
/// `docs/decisions/0034-tools-are-served-not-shipped.md` has the instance
/// serve its own tools rather than ship programs that call it, and this is
/// what one agent is told about them. Both halves are needed together: an
/// address nothing may use is no more useful than a credential with nowhere
/// to present it, so they are one value rather than two parameters that could
/// be passed apart.
///
/// **Named on every session and again on every resume**, which is what makes
/// this an address rather than a file. A container told a port once could not
/// be told a different one later, which is why an endpoint was written into
/// it; a session declaration is supplied afresh each time a session is created
/// or loaded, so an instance restarted on another port simply says so again.
#[derive(Clone)]
pub struct Tools {
    /// Where the tools are served.
    endpoint: String,
    /// What authorises this agent to use them, and decides which it is offered.
    credential: Secret,
}

impl Tools {
    /// What to tell an agent about the tools it may use.
    #[must_use]
    pub fn new(endpoint: impl Into<String>, credential: Secret) -> Self {
        Self {
            endpoint: endpoint.into(),
            credential,
        }
    }
}

/// Redacting, because this carries a credential.
///
/// `docs/conventions.md` §4 requires it of anything that can hold one, and the
/// derived version would print whatever `Secret` prints — which is safe today
/// and would stop being so the moment somebody changed that, silently and
/// somewhere else. Written out here so this type's own test pins it.
impl std::fmt::Debug for Tools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tools")
            .field("endpoint", &self.endpoint)
            .field("credential", &"<redacted>")
            .finish()
    }
}

/// What the tools look like on a session request.
///
/// Pure and separate from sending one, so the shape actually put on the wire
/// is asserted in the gate rather than inferred from a container that ran.
/// The transport is HTTP because that is what the adapters advertise: 0034
/// measured the alternative — a server offered over the protocol connection
/// itself — being accepted and silently dropped.
fn declaration(tools: &Tools) -> McpServer {
    McpServer::Http(
        McpServerHttp::new(TOOLS_SERVER, tools.endpoint.clone()).headers(vec![HttpHeader::new(
            "Authorization",
            format!("Bearer {}", tools.credential.expose()),
        )]),
    )
}

/// What this instance calls itself when it serves tools.
///
/// It prefixes every tool name the model sees, so it has to be the same string
/// the endpoint reports about itself. Asserted against that one in the app's
/// own tests rather than shared as a constant, because the two crates are on
/// opposite sides of a boundary that exists to keep the agent's business out
/// of the domain.
const TOOLS_SERVER: &str = "stageman";

/// The label every container this project starts carries.
///
/// How a container that outlives the process which started it is found again.
/// A name addresses one; this finds them all, including the ones whose job the
/// instance has forgotten. That is what makes `docs/conventions.md` §4's
/// "nothing untracked" checkable rather than merely intended — a sweep able to
/// see only what the snapshot already knew would never find the case it exists
/// for.
const OWNER_LABEL: &str = "stageman.job";

/// The port inside a job's container that a tunnel reaches.
///
/// One constant rather than a choice, because a mapping cannot be added to a
/// container that already exists — measured in
/// `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`, and the whole
/// reason nothing asks for a tunnel. Every job's container publishes this one,
/// at creation, whether or not anything ever listens on it.
///
/// **Unusual rather than familiar, on purpose.** Publishing a port does not
/// bind it inside the container, so the risk this number avoids is not a
/// collision: it is an agent that starts a dev server on 3000 to check its own
/// work and finds it published to whoever can reach this instance. A number
/// nobody reaches for by habit is only ever bound deliberately.
pub const TUNNEL_PORT: u16 = 47_201;

/// The arguments that start a container meant to outlive this process.
///
/// The one difference from [`session_arguments`] that matters is the absence
/// of `--rm`. With it, hard-killing the client destroys the container
/// outright; without it, the container is left exited with its filesystem —
/// and therefore its agent's session — intact. Both were measured, and
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` records which one
/// this system needs and why.
fn retained_arguments(
    name: &str,
    image: &Image,
    delivering: &[(&'static str, Secret)],
) -> Vec<String> {
    // Created rather than run, and never removed: the thread has to be put in
    // place before anything starts, and the container outlives this process.
    let mut arguments = vec![
        "create".to_owned(),
        "--interactive".to_owned(),
        // So that one hostname reaches this instance whichever runtime is in
        // use. Measured on both: Docker and Podman each honour it, and
        // without it a container on Linux can reach the host by no name at
        // all.
        "--add-host".to_owned(),
        "host.docker.internal:host-gateway".to_owned(),
        // The tunnel, published here because there is nowhere later: the
        // mapping is fixed when the container is created and no runtime can
        // add one afterwards.
        //
        // Loopback on the host, and an empty host port so the runtime picks a
        // free one atomically — choosing one here by binding and releasing it
        // is a race, and this project has already lost a port that way. What
        // it picked is asked for rather than recorded, because Docker assigns
        // a new one on every start and Podman does not.
        "--publish".to_owned(),
        format!("127.0.0.1::{TUNNEL_PORT}"),
        "--name".to_owned(),
        name.to_owned(),
        "--label".to_owned(),
        format!("{OWNER_LABEL}={name}"),
    ];
    arguments.extend(carrying(image, delivering));
    arguments
}

/// Starts a retained container for an agent and puts the first question to it.
///
/// The container survives this process, under `name`. Nothing removes it —
/// retention is deliberately unanswered in `docs/open-questions.md` until
/// there is a finished job to retire.
///
/// # Errors
///
/// Fails as [`ask`] does, and additionally if a container of this name already
/// exists. That refusal comes from the runtime rather than from here, and is
/// worth leaving to it: names are unique per daemon and enforced against
/// stopped containers too, so a clash is a loud message naming the container in
/// the way rather than a silent reuse of somebody else's.
#[mutants::skip]
pub async fn begin(
    runtime: &ContainerRuntime,
    handout: &Handout,
    name: &str,
    tools: Option<&Tools>,
    question: &str,
) -> Result<Answer, AgentError> {
    let delivering = variables(handout);
    // Which image is decided by the handout rather than passed in beside it.
    // That is what stops a container holding a foreman's credentials from
    // being started on a job's image — see
    // `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`.
    let image = build(runtime, handout.agent(), handout.role()).await?;
    // Created rather than run, so there is a moment between existing and
    // starting in which the thread can be put in place. `run` would have
    // started it immediately and left nowhere to do that.
    let created = tokio::process::Command::new(runtime.path())
        .args(retained_arguments(name, &image, &delivering))
        .envs(
            delivering
                .iter()
                .map(|(named, value)| ((*named).to_owned(), value.expose().to_owned())),
        )
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Container {
            status: "creating the container".to_owned(),
            message: source.to_string(),
        })?;
    if !created.status.success() {
        return Err(AgentError::Container {
            status: created.status.to_string(),
            message: String::from_utf8_lossy(&created.stderr).trim().to_owned(),
        });
    }

    let container = spawn(runtime, &started_arguments(name), &delivering)?;
    converse(container, Opening::Fresh, tools, question).await
}

/// What starts a container that already exists, attached.
///
/// Shared by beginning and resuming, which is what they became once the thread
/// stopped travelling in the environment: both create nothing at this point and
/// both attach to a container that is already there.
fn started_arguments(name: &str) -> Vec<String> {
    vec![
        "start".to_owned(),
        "--interactive".to_owned(),
        name.to_owned(),
    ]
}

/// Restarts a stopped container and continues the session inside it.
///
/// Takes no handout, and that is a property of the runtime rather than an
/// oversight: variables named at creation belong to the container's own
/// configuration, so a restart has the credential it was given without being
/// handed it again. Worth stating for the reason it is uncomfortable — the
/// credential now sits in the runtime's records for as long as the container is
/// kept, where `--rm` used to take it away with everything else.
///
/// `question` is what the resumed agent is told. The measurement in
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` says it works out
/// that it was interrupted unaided; telling it is nearly free, and the
/// alternative is depending on an inference.
///
/// # Errors
///
/// Fails as [`ask`] does, and with [`AgentError::NothingToResume`] if the
/// container holds no session — which is what a container stopped before its
/// agent said anything looks like.
pub async fn resume(
    runtime: &ContainerRuntime,
    name: &str,
    tools: Option<&Tools>,
    question: &str,
) -> Result<Answer, AgentError> {
    settle(runtime, name).await?;
    // Nothing is written into the container before it starts any more. The
    // thread this turn belongs in travels on `tools`, which is the whole of
    // why a container can be told a different one every turn without its
    // environment changing — see
    // `docs/decisions/0034-tools-are-served-not-shipped.md`.
    let container = spawn(runtime, &started_arguments(name), &[])?;
    converse(container, Opening::Resumed, tools, question).await
}

/// Makes sure a container is stopped before anything tries to start it.
///
/// Found by a test rather than by reasoning, and it is a real race rather than
/// a test artefact. Killing the client that was driving a container does not
/// stop the container instantly, and starting one that is still running does
/// not attach — so a resume attempted in that window finds a closed transport
/// and reads as a protocol failure, which is the least informative thing it
/// could be. Stopping first collapses the window: the runtime treats stopping
/// an already-stopped container as success, so this costs nothing in the
/// ordinary case where the container stopped long ago.
async fn settle(runtime: &ContainerRuntime, name: &str) -> Result<(), AgentError> {
    let stopped = tokio::process::Command::new(runtime.path())
        .args(["stop", name])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if stopped.status.success() {
        return Ok(());
    }
    Err(AgentError::Unusable {
        path: runtime.path().to_owned(),
        message: String::from_utf8_lossy(&stopped.stderr).trim().to_owned(),
    })
}

/// Every container this project has left behind, by name.
///
/// Found by label rather than by reading the instance, so that a container
/// whose job the snapshot has lost is still findable.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses the query.
pub async fn abandoned(runtime: &ContainerRuntime) -> Result<Vec<String>, AgentError> {
    let listed = tokio::process::Command::new(runtime.path())
        .args([
            "ps",
            "--all",
            "--filter",
            &format!("label={OWNER_LABEL}"),
            "--format",
            "{{.Names}}",
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if !listed.status.success() {
        return Err(AgentError::Unusable {
            path: runtime.path().to_owned(),
            message: String::from_utf8_lossy(&listed.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Which host port a container's tunnel is reachable on, right now.
///
/// **Asked for rather than remembered, and that is the whole point.** The two
/// runtimes disagree about what happens to an ephemeral published port when a
/// container is stopped and started again — Docker assigns a new one, Podman
/// keeps the old — so anything storing this is correct on one and silently
/// wrong on the other, in the resume path, which is the least observed code
/// here. The runtime is the only thing that knows, so the runtime is asked.
/// Measured in `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`.
///
/// `None` for a container that exists and has no mapping — one created before
/// this project published anything, which is an ordinary thing to meet after
/// an upgrade rather than a failure.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses the query. A container that
/// is not there refuses, which is correct: nothing can be reached on it.
///
/// Skipped by mutation testing, like everything else here that drives the
/// runtime: what it does is spawn a process and hand the output to
/// the private parser beside it, which is where the deciding is and is tested
/// directly. The
/// container behaviour it wraps is pinned by an ignored test below, and an
/// ignored test kills no mutant.
#[mutants::skip]
pub async fn tunnel_port(
    runtime: &ContainerRuntime,
    name: &str,
) -> Result<Option<u16>, AgentError> {
    let reported = tokio::process::Command::new(runtime.path())
        .args(["port", name, &TUNNEL_PORT.to_string()])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if !reported.status.success() {
        return Err(AgentError::Unusable {
            path: runtime.path().to_owned(),
            message: String::from_utf8_lossy(&reported.stderr).trim().to_owned(),
        });
    }
    Ok(published(&String::from_utf8_lossy(&reported.stdout)))
}

/// The host port in what the runtime reported, if it reported one.
///
/// Pure, so every shape either runtime prints can be tested without a
/// container. It takes the port from the *last* colon onwards rather than
/// splitting on colons, because a mapping published on IPv6 is printed as
/// `[::]:64383` and splitting would find the address instead — and both
/// runtimes will print a v6 line alongside the v4 one on a dual-stack host.
///
/// The first line that yields a port wins. Nothing here prefers one family
/// over the other: both reach the same container, and this connects over
/// loopback where both work.
fn published(reported: &str) -> Option<u16> {
    reported
        .lines()
        .filter_map(|line| line.trim().rsplit(':').next())
        .find_map(|port| port.trim().parse().ok())
}

/// Removes a container and everything inside it.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses. A container that is not
/// there is not a failure: what was asked for is that it be gone.
pub async fn discard(runtime: &ContainerRuntime, name: &str) -> Result<(), AgentError> {
    let removed = tokio::process::Command::new(runtime.path())
        .args(["rm", "--force", name])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if removed.status.success() || String::from_utf8_lossy(&removed.stderr).contains("No such") {
        return Ok(());
    }
    Err(AgentError::Unusable {
        path: runtime.path().to_owned(),
        message: String::from_utf8_lossy(&removed.stderr).trim().to_owned(),
    })
}

/// Whether a conversation starts a session or picks one up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opening {
    /// Make a new session in a container that has none.
    Fresh,
    /// Find the session already in this container, and load it.
    Resumed,
}

/// Starts the runtime with `arguments`, with its three streams piped.
fn spawn(
    runtime: &ContainerRuntime,
    arguments: &[String],
    delivering: &[(&'static str, Secret)],
) -> Result<tokio::process::Child, AgentError> {
    tokio::process::Command::new(runtime.path())
        .args(arguments)
        // Set here rather than inherited. Nothing else is forwarded, because
        // the runtime forwards only what `--env` names — which is what makes
        // `docs/conventions.md` §3's "constructed, never inherited" true by
        // mechanism instead of by care.
        .envs(
            delivering
                .iter()
                .map(|(name, secret)| (*name, secret.expose())),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })
}

/// Makes a session, or picks up the one this container already holds.
///
/// Skipped by mutation testing, and the reason is worth stating rather than
/// assumed: every path through this needs a container answering the protocol,
/// so it is covered only by tests the gate cannot run. What it decides that
/// *can* be checked cheaply — the declaration a session carries — is
/// `declaration`, which is pure and has its own test.
///
/// `None` means there was nothing to pick up, which is a container stopped
/// before its agent said anything: sessions are written when something is
/// said, not when one is created.
#[mutants::skip]
async fn open_session(
    connection: &ConnectionTo<AgentRole>,
    opening: Opening,
    tools: Option<&Tools>,
) -> Result<Option<SessionId>, agent_client_protocol::Error> {
    match opening {
        Opening::Fresh => {
            let mut request = NewSessionRequest::new(PathBuf::from(WORKSPACE));
            if let Some(tools) = tools {
                request.mcp_servers.push(declaration(tools));
            }
            Ok(Some(
                connection
                    .send_request(request)
                    .block_task()
                    .await?
                    .session_id,
            ))
        }
        Opening::Resumed => {
            let known = connection
                .send_request(ListSessionsRequest::new())
                .block_task()
                .await?;
            // The first, because a job's container holds one. More than one
            // would mean something else made a session here, which is not a
            // case this system produces.
            let Some(found) = known.sessions.into_iter().next() else {
                return Ok(None);
            };
            // Declared again here, and this is the half that makes the
            // endpoint file unnecessary: a resumed container is told where to
            // reach the instance *now*, so a port that changed between turns
            // is simply named again rather than baked in when the container
            // was created.
            let mut request =
                LoadSessionRequest::new(found.session_id.clone(), PathBuf::from(WORKSPACE));
            if let Some(tools) = tools {
                request.mcp_servers.push(declaration(tools));
            }
            connection.send_request(request).block_task().await?;
            Ok(Some(found.session_id))
        }
    }
}

/// Speaks the protocol to a started container and puts one question to it.
async fn converse(
    mut container: tokio::process::Child,
    opening: Opening,
    tools: Option<&Tools>,
    question: &str,
) -> Result<Answer, AgentError> {
    let (Some(to_agent), Some(from_agent), Some(complaints)) = (
        container.stdin.take(),
        container.stdout.take(),
        container.stderr.take(),
    ) else {
        return Err(AgentError::NoChannel);
    };

    let heard = Arc::new(Mutex::new(String::new()));
    let collecting = Arc::clone(&heard);

    let (spoken, printed_out) = futures::future::join(
        Client
            .builder()
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    if let SessionUpdate::AgentMessageChunk(chunk) = notification.update
                        && let ContentBlock::Text(said) = chunk.content
                    {
                        collecting.lock().push_str(&said.text);
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _cx| {
                    // Approved, and not because permission is meaningless. The
                    // boundary this system relies on is the container, chosen
                    // in `docs/decisions/0012-agents-run-in-containers.md`
                    // precisely so that it enforces isolation rather than the
                    // agent respecting it — and
                    // `docs/decisions/0010-acp-is-the-agent-contract.md`
                    // measured that agents decide and report rather than
                    // genuinely asking. Refusing here would forbid an agent
                    // from doing what it was started to do, inside a boundary
                    // built to make that safe.
                    let allow = request
                        .options
                        .iter()
                        .find(|option| {
                            format!("{:?}", option.kind)
                                .to_lowercase()
                                .contains("allow")
                        })
                        .or_else(|| request.options.first())
                        .map(|option| option.option_id.clone());
                    responder.respond(RequestPermissionResponse::new(
                        allow.map_or(RequestPermissionOutcome::Cancelled, |id| {
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id))
                        }),
                    ))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_with(
                ByteStreams::new(to_agent.compat_write(), from_agent.compat()),
                async |connection: ConnectionTo<AgentRole>| {
                    connection
                        .send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;

                    let Some(session_id) = open_session(&connection, opening, tools).await? else {
                        return Ok(None);
                    };
                    let reply = connection
                        .send_request(PromptRequest::new(
                            session_id,
                            vec![ContentBlock::Text(TextContent::new(question.to_owned()))],
                        ))
                        .block_task()
                        .await?;
                    Ok(Some(reply.stop_reason))
                },
            ),
        printed(complaints),
    )
    .await;

    let status = container.wait().await.map_err(AgentError::Exit)?;

    match spoken {
        Ok(Some(stop_reason)) => Ok(Answer {
            text: heard.lock().clone(),
            stop_reason,
        }),
        Ok(None) => Err(AgentError::NothingToResume),
        // A container that failed on its own terms explains itself better than
        // the protocol error its silence produced, so it wins when both exist.
        Err(_) if !status.success() => Err(AgentError::Container {
            status: status.to_string(),
            message: printed_out,
        }),
        Err(protocol) => Err(AgentError::Protocol(protocol)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What an agent is told about the tools, field by field.
    ///
    /// Asserted on the typed value rather than on serialised JSON, because the
    /// wire format belongs to the protocol library and re-encoding it here
    /// would test that library rather than this decision. What is this crate's
    /// to get right is *which* transport and *what* headers, and both are here.
    ///
    /// The transport is the whole of what
    /// `docs/decisions/0034-tools-are-served-not-shipped.md` measured: a server
    /// offered over the protocol connection is accepted by the pinned adapter
    /// and silently dropped, so this has to be the HTTP one.
    #[test]
    fn the_tools_are_declared_over_http_with_the_credential_in_a_header() {
        let tools = Tools::new(
            "http://host.docker.internal:47113/mcp",
            Secret::new("a-warrant".to_owned()),
        );

        let McpServer::Http(declared) = declaration(&tools) else {
            panic!("the tools must be offered over HTTP, which is what adapters accept");
        };
        assert_eq!(declared.name, TOOLS_SERVER);
        assert_eq!(declared.url, "http://host.docker.internal:47113/mcp");

        let [header] = declared.headers.as_slice() else {
            panic!("exactly one header, carrying the credential");
        };
        assert_eq!(header.name, "Authorization");
        assert_eq!(
            header.value, "Bearer a-warrant",
            "presented as a bearer, which is what the endpoint reads",
        );
    }

    /// The declaration carries a credential, so formatting it must not.
    ///
    /// `docs/conventions.md` §4 requires this of anything able to hold one.
    /// The endpoint is deliberately still printed: it is an address, it is
    /// already reported at startup, and a redacted one would make a container
    /// that cannot reach the instance much harder to diagnose.
    #[test]
    fn the_tools_do_not_print_the_credential_they_carry() {
        let tools = Tools::new(
            "http://host.docker.internal:47113/mcp",
            Secret::new("a-warrant-nobody-should-see".to_owned()),
        );

        let printed = format!("{tools:?}");
        assert!(
            !printed.contains("a-warrant-nobody-should-see"),
            "the credential reached a formatted string: {printed}",
        );
        assert!(printed.contains("redacted"), "{printed}");
        assert!(
            printed.contains("47113"),
            "the address is not a secret and is what a failure to reach it needs: {printed}",
        );
    }

    /// An identifier standing in for one a runtime would have answered with.
    ///
    /// The argument builders take a built image, and building one needs a
    /// runtime — so a test about *arguments* would otherwise need a container
    /// runtime to assert something pure. This is the seam that keeps the
    /// cheap tests cheap.
    fn built() -> Image {
        Image(BUILT.to_owned())
    }

    /// The same identifier as a literal, which is what the assertions compare
    /// against.
    ///
    /// Never `built().as_argument()`, and that is the whole point of it
    /// existing: an assertion that reads the value back through the method
    /// under test compares a mutation to itself and passes. Mutation testing
    /// found exactly that — [`Image::as_argument`] could return an empty
    /// string with every argument test still green.
    const BUILT: &str = "sha256:0123456789abcdef";

    /// Every stage this crate asks for exists in the recipe it asks of.
    ///
    /// The agreement no compiler can make, and the one that replaced a
    /// harder one. It used to be an image tag written in two files, held
    /// together by parsing `project.just` from a test;
    /// `docs/decisions/0035-an-image-is-built-never-named.md` removed the tag
    /// and `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md` put this in
    /// its place — a stage name in the recipe and the same name in [`stage`].
    ///
    /// Strictly cheaper than what it replaces, and that is the compiled-in
    /// recipe paying for itself: the text is in the binary, so this reads no
    /// file and reaches no second directory.
    #[test]
    fn every_stage_the_adapter_asks_for_is_in_the_recipe() {
        let recipe = recipe(Agent::Claude);
        for role in [Role::Foreman, Role::Job] {
            let declared = format!("AS {}", stage(role));
            assert!(
                recipe.contains(&declared),
                "the recipe declares no `{declared}`, so a build for {role:?} would fail \
                 at the runtime rather than here",
            );
        }
    }

    /// The two roles do not build the same thing.
    ///
    /// Worth asserting on its own, because a copy-paste in [`stage`] would
    /// leave the test above perfectly green while handing a foreman a job's
    /// image — which is the whole of what 0036 refuses.
    #[test]
    fn a_foreman_and_a_job_are_built_from_different_stages() {
        assert_ne!(stage(Role::Foreman), stage(Role::Job));
    }

    /// A build reads its recipe from standard input and names no context.
    #[test]
    fn a_build_takes_its_recipe_on_standard_input_and_no_context() {
        let arguments = build_arguments(Role::Job);
        assert_eq!(arguments[0], "build");
        assert!(
            arguments.contains(&"--quiet"),
            "without this the identifier is not the whole of standard output",
        );
        assert_eq!(
            arguments.last(),
            Some(&"-"),
            "the trailing dash is the recipe arriving on standard input",
        );
    }

    #[test]
    fn a_handshake_asks_for_no_network_and_leaves_nothing_behind() {
        let image = built();
        let arguments = handshake_arguments(&image);
        assert_eq!(arguments[0], "run");
        assert!(arguments.contains(&"--rm"));
        assert!(arguments.contains(&"--interactive"));
        assert_eq!(arguments[3..5], ["--network", "none"]);
    }

    #[test]
    fn a_handshake_runs_the_image_that_was_built() {
        let image = built();
        let arguments = handshake_arguments(&image);
        assert_eq!(arguments.last(), Some(&BUILT));
    }

    /// A build that worked is the image it named, and nothing else.
    ///
    /// Trimmed, because both runtimes end that line and an identifier with a
    /// newline in it is not one a container can be started from.
    #[test]
    fn a_build_that_succeeded_is_the_image_it_named() {
        let image =
            outcome(true, b"sha256:abcdef123456\n", b"").expect("a build that worked is an image");
        assert_eq!(image.as_argument(), "sha256:abcdef123456");
    }

    /// A build that failed is not an image, however much it printed.
    ///
    /// The other half of the one above, and the pair is the point: either on
    /// its own would still pass with the test of success inverted.
    #[test]
    fn a_build_that_failed_is_not_an_image() {
        let failure = outcome(false, b"sha256:notthis\n", b"#4 ERROR: exit code 1\n");
        let Err(AgentError::Build { message }) = failure else {
            panic!("expected a build failure, got {failure:?}");
        };
        assert!(message.contains("ERROR: exit code 1"), "{message}");
    }

    /// A build that failed says the end of what it said, not the beginning.
    #[test]
    fn a_failed_build_is_reported_from_its_last_words() {
        let said = b"#4 [2/3] RUN npm install\n#4 0.4 npm error network\n#4 ERROR: exit code 1\n";
        let reported = last_words(said);
        assert!(reported.contains("ERROR: exit code 1"), "{reported}");
        assert!(reported.contains("npm error network"), "{reported}");
    }

    /// A build that said nothing still says something.
    ///
    /// An empty message would render as "the agent's image could not be
    /// built: " and send somebody to read the source.
    #[test]
    fn a_silent_failed_build_still_reports_something() {
        assert!(!last_words(b"").is_empty());
        assert!(!last_words(b"   \n\n  \n").is_empty());
    }

    #[test]
    fn a_runtime_keeps_the_path_it_was_configured_with() {
        let runtime = ContainerRuntime::new(PathBuf::from("/usr/local/bin/docker"));
        assert_eq!(runtime.path(), Path::new("/usr/local/bin/docker"));
    }

    /// The check that a filesystem test cannot make.
    ///
    /// This used to be an integration test driving the whole binary against a
    /// deliberately broken runtime. Discovery took away the ability to point
    /// the binary anywhere, so the check moves to the mechanism it was always
    /// really about: `verify` asks for a version because that reaches the
    /// daemon, and a client installed with nothing behind it looks perfectly
    /// healthy to anything that merely looks for the file.
    #[tokio::test]
    async fn a_runtime_that_runs_and_refuses_is_not_usable() {
        let refusing = ["/usr/bin/false", "/bin/false"]
            .into_iter()
            .map(PathBuf::from)
            .find(|candidate| candidate.exists())
            .expect("a standard utility that always refuses");

        let failure = ContainerRuntime::new(refusing).verify().await;

        assert!(
            matches!(failure, Err(AgentError::Unusable { .. })),
            "{failure:?}"
        );
    }

    #[test]
    fn nothing_installed_is_an_absence_rather_than_a_failure() {
        assert_eq!(first_present(&[]), None);
        assert_eq!(first_present(&["/nowhere/at/all/docker"]), None);
    }

    #[test]
    fn the_first_path_that_is_there_is_the_one_used() {
        let real = ["/usr/bin/false", "/bin/false"]
            .into_iter()
            .find(|candidate| Path::new(candidate).is_file())
            .expect("a standard utility");

        let found = first_present(&["/nowhere/at/all/docker", real]);

        assert_eq!(found, Some(ContainerRuntime::new(PathBuf::from(real))));
    }

    /// A directory is not a runtime, which `exists` would not have caught.
    #[test]
    fn a_directory_where_a_runtime_would_be_is_not_one() {
        assert_eq!(first_present(&["/usr/bin"]), None);
    }

    /// Every candidate is absolute, which is the property that makes this not
    /// a `PATH` search.
    #[test]
    fn nothing_is_looked_for_relative_to_wherever_this_started() {
        assert!(
            !candidates().is_empty(),
            "this platform knows nowhere to look"
        );
        for candidate in candidates() {
            assert!(
                Path::new(candidate).is_absolute(),
                "{candidate} is not an absolute path"
            );
        }
    }

    #[tokio::test]
    async fn a_runtime_that_is_not_there_fails_as_a_runtime_rather_than_a_protocol() {
        let runtime = ContainerRuntime::new(PathBuf::from("/nonexistent/container/runtime"));
        let failure = handshake(&runtime, Agent::Claude, Role::Foreman).await;
        assert!(matches!(failure, Err(AgentError::Runtime { .. })));
    }

    /// The container runtime, found rather than configured.
    ///
    /// Looking it up here is not a breach of the rule this crate states: that
    /// rule is about a daemon which must work under a service manager, and
    /// this is a test which must work on a developer's machine.
    fn located_runtime() -> ContainerRuntime {
        let located = std::process::Command::new("sh")
            .args(["-c", "command -v docker"])
            .output()
            .expect("looking for a container runtime");
        let path = String::from_utf8(located.stdout).expect("a runtime path is text");
        ContainerRuntime::new(PathBuf::from(path.trim()))
    }

    /// Drives a real container, so it needs a runtime and a network.
    ///
    /// Ignored by default rather than absent: `just check` stays a gate you can
    /// run constantly, and nextest still counts this as ignored — which is the
    /// distinction that matters, because a test behind a `cfg` nobody selected
    /// appears nowhere and the total still reads as complete. Run it with
    /// `just image-handshake`.
    ///
    /// It no longer needs an image somebody built first: [`handshake`] builds
    /// one, which means this now covers the build as well as the exchange.
    /// **Both roles**, because two images is the thing 0036 claims and one
    /// greeting proves nothing about the other — and because a foreman's is
    /// the one whose stage could be edited into uselessness without any test
    /// of a job's noticing.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_container_answers_the_protocol() {
        let runtime = located_runtime();

        for role in [Role::Foreman, Role::Job] {
            let greeting = handshake(&runtime, Agent::Claude, role)
                .await
                .unwrap_or_else(|why| panic!("{role:?} answers the handshake: {why}"));

            assert_eq!(greeting.protocol_version, ProtocolVersion::V1);
            let adapter = greeting.adapter.expect("the adapter names itself");
            assert!(!adapter.name.is_empty());
            assert!(!adapter.version.is_empty());
        }
    }

    /// A foreman's container cannot reach a repository, because it has no tool
    /// that could.
    ///
    /// The evidence behind
    /// `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`, and the reason
    /// that record is a decision rather than a preference: the narrowing in
    /// `docs/decisions/0027-a-channel-is-not-a-platform.md` withheld a
    /// credential, and this withholds the capability. Asserted in both
    /// directions, because a split that quietly stopped splitting would leave
    /// a one-sided test green.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn only_a_jobs_image_can_reach_a_repository() {
        let runtime = located_runtime();

        for (role, expected) in [(Role::Foreman, false), (Role::Job, true)] {
            let image = build(&runtime, Agent::Claude, role)
                .await
                .unwrap_or_else(|why| panic!("{role:?} builds: {why}"));
            let looked = tokio::process::Command::new(runtime.path())
                .args([
                    "run",
                    "--rm",
                    "--network",
                    "none",
                    "--entrypoint",
                    "sh",
                    image.as_argument(),
                    "-c",
                    "command -v git && command -v gh",
                ])
                .output()
                .await
                .expect("the runtime runs");
            assert_eq!(
                looked.status.success(),
                expected,
                "{role:?} should{} reach a repository",
                if expected { "" } else { " not" },
            );
        }
    }

    /// A container that fills its error pipe must not become a hang.
    ///
    /// The pipe's kernel buffer is around sixty-four kilobytes and the bound on
    /// what is *kept* is eight, so a container printing two hundred would block
    /// on the write under either of the two mistakes this guards: draining
    /// after the exchange instead of during it, or stopping the drain at the
    /// bound. It reuses the agent's own image with the entry point overridden,
    /// so it asks for nothing the tests above do not already build.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_container_that_floods_its_error_pipe_does_not_hang() {
        let runtime = located_runtime();
        // A foreman's, because this overrides the entry point and so needs the
        // smaller of the two rather than either in particular.
        let flooding = build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");

        let failure = greet(
            &runtime,
            &[
                "run",
                "--rm",
                "--interactive",
                "--network",
                "none",
                "--entrypoint",
                "sh",
                flooding.as_argument(),
                "-c",
                "yes noise | head -c 200000 >&2; exit 3",
            ],
        )
        .await;

        let Err(AgentError::Container { status, message }) = failure else {
            panic!("expected a container failure, got {failure:?}");
        };
        assert!(status.contains('3'), "exit status was {status}");
        assert!(
            message.len() < STDERR_LIMIT * 2,
            "kept {} bytes of a flood",
            message.len()
        );
    }

    /// A container that dies before speaking must not read as an agent that
    /// cannot speak.
    ///
    /// Both fail as silence on the connection, and the error an operator gets
    /// is the only thing that distinguishes "the container did not start" from
    /// "the adapter is broken".
    ///
    /// It used to be named for its commonest cause — an image nobody had built
    /// — and `docs/decisions/0035-an-image-is-built-never-named.md` retired
    /// that cause rather than this test, which is why the name changed and the
    /// assertion did not. What is left is every other way a container fails to
    /// start, and the classification matters just as much for those: the
    /// runtime out of space, an entry point that exits, a daemon that stops
    /// between the build and the run.
    ///
    /// It still asks for an image that does not exist, because that is simply
    /// the cheapest container that reliably dies before speaking.
    #[tokio::test]
    #[ignore = "needs a container runtime; run `just image-handshake`"]
    async fn a_container_that_never_starts_fails_as_a_container_not_a_protocol() {
        let runtime = located_runtime();

        let failure = greet(
            &runtime,
            &[
                "run",
                "--rm",
                "--interactive",
                "--network",
                "none",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ],
        )
        .await;

        assert!(
            matches!(failure, Err(AgentError::Container { .. })),
            "expected a container failure, got {failure:?}"
        );
    }

    use stageman_core::{AgentConfig, Handout, Job, Project, ProjectId, State, Uuid};
    use std::collections::BTreeMap;

    /// An instance configured with one agent and nothing else.
    /// An instance with one agent configured and nothing else.
    fn instance(credential: &str) -> State {
        State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new(credential.to_owned()),
                },
            )]),
            ..State::default()
        }
    }

    fn only_claude() -> std::collections::BTreeSet<Agent> {
        std::collections::BTreeSet::from([Agent::Claude])
    }

    /// An instance with one project, so a handout can carry a platform
    /// credential as well as an agent's own.
    fn instance_with_a_project(credential: &str) -> (State, ProjectId) {
        let mut state = instance(credential);
        let id = ProjectId::from_uuid(Uuid::from_u128(7));
        let mut credentials = BTreeMap::new();
        credentials.insert(
            Platform::GitHub,
            Secret::new("gh-not-a-real-token".to_owned()),
        );
        state.projects.insert(
            id,
            Project {
                name: "example".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                foreman_agent: Agent::Claude,
                job_agents: only_claude(),
                credentials,
                channels: BTreeMap::new(),
                jobs: BTreeMap::<_, Job>::new(),
                attending: stageman_core::Attending::default(),
            },
        );
        (state, id)
    }

    /// The same, with a channel bound.
    ///
    /// Separate from the fixture above rather than replacing it, because both
    /// shapes are real and each is the subject of its own claim: a project with
    /// nothing bound is a working project, per
    /// `docs/decisions/0005-conversation-happens-on-channels.md`.
    fn instance_with_a_channel(credential: &str) -> (State, ProjectId) {
        let (mut state, id) = instance_with_a_project(credential);
        state
            .projects
            .get_mut(&id)
            .expect("the project was just inserted")
            .channels
            .insert(
                Channel::Slack,
                stageman_core::ChannelConfig {
                    address: "C0123456789".to_owned(),
                    credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                    listen_credential: Some(Secret::new("xapp-not-a-real-token".to_owned())),
                },
            );
        (state, id)
    }

    /// A thread never travels as a variable, however narrowed the handout is.
    ///
    /// It used to, and that was a bug waiting for a second turn: a container's
    /// environment is fixed when it is created, so a variable can only carry a
    /// value constant for its whole life. A job's thread is; a foreman's is
    /// not, and one long-lived container answering every message would have
    /// answered all of them in the first message's thread.
    #[test]
    fn a_thread_is_never_delivered_as_a_variable() {
        let (state, project) = instance_with_a_channel("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Agent::Claude, project)
            .expect("a watched project")
            .speaking_in(stageman_core::Thread {
                channel: Channel::Slack,
                id: "1728312345.678901".to_owned(),
            });

        let delivered = variables(&handout);
        let named: Vec<&str> = delivered.iter().map(|(name, _)| *name).collect();

        assert!(
            !named.iter().any(|name| name.contains("THREAD")),
            "the thread goes in a file, not the environment: {named:?}"
        );
        // And nothing carries its value under another name either.
        for (_, value) in &delivered {
            assert_ne!(value.expose(), "1728312345.678901");
        }
        // Nor the channel's credential, since 0034: the daemon posts, so a
        // container has no use for one and holding less is the whole
        // mitigation.
        assert!(!named.contains(&"STAGEMAN_SLACK_CHANNEL"), "{named:?}");
        assert!(!named.contains(&"STAGEMAN_SLACK_TOKEN"), "{named:?}");
    }

    /// A container is given its agent's credential and nothing else it does
    /// not use.
    ///
    /// `docs/decisions/0027-a-channel-is-not-a-platform.md` kept a platform
    /// credential out of a foreman's hands, and that still holds. What changed
    /// is the other half: the channel's credential used to be delivered too,
    /// because a program in the container posted with it. Since
    /// `docs/decisions/0034-tools-are-served-not-shipped.md` the daemon posts,
    /// so neither a foreman nor a job is given one — and a credential a
    /// process never receives is one it cannot be talked into sending
    /// anywhere.
    #[test]
    fn a_container_is_given_no_credential_it_has_no_use_for() {
        let (state, project) = instance_with_a_channel("sk-ant-oat01-xyz");

        for handout in [
            Handout::for_foreman(&state, project).expect("a watched project"),
            Handout::for_job(&state, Agent::Claude, project).expect("a watched project"),
        ] {
            let named: Vec<&str> = variables(&handout).iter().map(|(name, _)| *name).collect();
            assert!(!named.contains(&"STAGEMAN_SLACK_TOKEN"), "{named:?}");
            assert!(!named.contains(&"STAGEMAN_SLACK_CHANNEL"), "{named:?}");
        }

        // And the asymmetry 0027 turns on is unchanged: a foreman watches a
        // channel and still acts on no platform.
        let foreman = Handout::for_foreman(&state, project).expect("a watched project");
        let named: Vec<&str> = variables(&foreman).iter().map(|(name, _)| *name).collect();
        assert!(!named.contains(&"GH_TOKEN"), "{named:?}");
    }

    /// A project with nothing bound is delivered nothing to speak with, rather
    /// than an empty variable — which would leave `stageman-say` unable to tell
    /// an unbound project from a broken one.
    #[test]
    fn a_job_with_no_channel_is_delivered_no_channel_variables() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Agent::Claude, project).expect("a watched project");

        let named: Vec<&str> = variables(&handout).iter().map(|(name, _)| *name).collect();

        assert!(
            !named.iter().any(|name| name.starts_with("STAGEMAN_SLACK")),
            "{named:?}"
        );
    }

    #[test]
    fn an_oauth_token_and_an_api_key_go_to_different_variables() {
        assert_eq!(
            claude_credential_variable(&Secret::new("sk-ant-oat01-xyz".to_owned())),
            "CLAUDE_CODE_OAUTH_TOKEN"
        );
        assert_eq!(
            claude_credential_variable(&Secret::new("sk-ant-api03-xyz".to_owned())),
            "ANTHROPIC_API_KEY"
        );
    }

    #[test]
    fn a_foreman_is_delivered_its_credential_and_nothing_else() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let delivered = variables(&handout);

        assert_eq!(delivered.len(), 1, "{delivered:?}");
        assert_eq!(delivered[0].0, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(delivered[0].1.expose(), "sk-ant-oat01-xyz");
    }

    #[test]
    fn a_job_is_delivered_the_variable_its_platform_tool_reads() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Agent::Claude, project).expect("a watched project");

        let delivered = variables(&handout);
        let named: Vec<&str> = delivered.iter().map(|(name, _)| *name).collect();

        assert!(named.contains(&"CLAUDE_CODE_OAUTH_TOKEN"), "{named:?}");
        assert!(named.contains(&"GH_TOKEN"), "{named:?}");
    }

    /// The one that matters most in this module. A secret on a command line is
    /// readable by every user on the machine through the process table, so the
    /// arguments must *name* each variable and never carry its value.
    #[test]
    fn no_credential_ever_appears_in_a_containers_arguments() {
        let (state, project) = instance_with_a_channel("sk-ant-oat01-secret-value");
        let handout = Handout::for_job(&state, Agent::Claude, project).expect("a watched project");

        let arguments = session_arguments(&built(), &variables(&handout));
        let line = arguments.join(" ");

        assert!(!line.contains("sk-ant-oat01-secret-value"), "{line}");
        assert!(!line.contains("gh-not-a-real-token"), "{line}");
        // The newest credential, and the one a reviewer would not think to
        // check: a channel binding arrived through a different map and a
        // different loop, so it is a second chance to make the same mistake.
        assert!(!line.contains("xoxb-not-a-real-token"), "{line}");
        assert!(line.contains("--env CLAUDE_CODE_OAUTH_TOKEN"), "{line}");
        assert!(line.contains("--env GH_TOKEN"), "{line}");
        // No channel credential is named at all since 0034, because none is
        // delivered: the daemon posts, so a container has no use for one.
        assert!(!line.contains("STAGEMAN_SLACK"), "{line}");
    }

    #[test]
    fn a_session_container_is_not_cut_off_from_the_network() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let arguments = session_arguments(&built(), &variables(&handout));

        assert!(!arguments.iter().any(|a| a == "none"), "{arguments:?}");
        assert_eq!(arguments.last().map(String::as_str), Some(BUILT));
    }

    /// Tests that spend real money, kept in their own module so a filter can
    /// name them as a group rather than one at a time. Run with
    /// `just image-session`; `just image-handshake` deliberately excludes them,
    /// because everything it runs needs only a runtime and a network.

    #[test]
    fn a_retained_container_is_named_labelled_and_survives_its_own_exit() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let arguments = retained_arguments("stageman-job-abc", &built(), &variables(&handout));
        let line = arguments.join(" ");

        assert!(line.contains("--name stageman-job-abc"), "{line}");
        assert!(
            line.contains("--label stageman.job=stageman-job-abc"),
            "{line}"
        );
        assert_eq!(
            arguments.first().map(String::as_str),
            Some("create"),
            "created rather than run, so the thread can be put in before it starts"
        );
        assert_eq!(
            arguments.last().map(String::as_str),
            Some(BUILT),
            "the image has to stay last"
        );
        // The whole difference between a container that survives being killed
        // and one that vanishes with it.
        assert!(!arguments.iter().any(|a| a == "--rm"), "{line}");
        // Stdin has to be opened at creation or nothing can attach to it
        // later, and a session is a conversation over stdin.
        assert!(arguments.iter().any(|a| a == "--interactive"), "{line}");
    }

    /// The tunnel is published here or nowhere: no runtime adds one later.
    ///
    /// Asserted on the exact string rather than on the flag alone, because the
    /// two halves that matter are both in the value. Without the `127.0.0.1`
    /// the runtime publishes on every interface, which would put a server an
    /// agent wrote onto whatever network this machine has joined. Without the
    /// empty host port this would be picking one itself, which is a race.
    #[test]
    fn a_retained_container_publishes_its_tunnel_on_loopback() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let arguments = retained_arguments("stageman-job-abc", &built(), &variables(&handout));
        let line = arguments.join(" ");

        assert!(
            line.contains(&format!("--publish 127.0.0.1::{TUNNEL_PORT}")),
            "{line}"
        );
    }

    /// Both runtimes print an address and a port, and the address may be v6.
    #[test]
    fn a_published_port_is_read_from_the_last_colon() {
        assert_eq!(published("127.0.0.1:64383\n"), Some(64_383));
        assert_eq!(
            published("[::]:64383\n"),
            Some(64_383),
            "splitting on colons would find the address in a v6 mapping",
        );
        assert_eq!(
            published("0.0.0.0:42539\n[::]:42539\n"),
            Some(42_539),
            "a dual-stack host reports both families and either reaches it",
        );
    }

    /// A container with no mapping is not a failure, and says so as `None`.
    ///
    /// What one created before this project published anything reports, which
    /// is an ordinary thing to meet after an upgrade rather than a fault.
    #[test]
    fn a_container_with_no_mapping_reports_no_port() {
        assert_eq!(published(""), None);
        assert_eq!(published("\n  \n"), None);
        assert_eq!(
            published("127.0.0.1:not-a-port\n"),
            None,
            "unparseable is absent rather than a wrong number",
        );
    }

    /// Starting a container that exists names it, attaches, and nothing else.
    ///
    /// The shared tail beginning and resuming both end with, since the thread
    /// stopped travelling in the environment. Worth asserting because it is
    /// the only thing between a created container and a conversation: an
    /// argument list that lost `--interactive` would produce a session with no
    /// stdin, which reads as an agent that will not speak.
    #[test]
    fn starting_an_existing_container_attaches_to_it_by_name() {
        let arguments = started_arguments("stageman-foreman-abc");

        assert_eq!(
            arguments,
            vec![
                "start".to_owned(),
                "--interactive".to_owned(),
                "stageman-foreman-abc".to_owned(),
            ]
        );
    }

    /// A container this project started, found without consulting the instance
    /// and then removed. Needs a runtime and a network, and no credential: it
    /// overrides the entry point rather than running an agent.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_container_this_project_started_is_found_by_label_and_discarded() {
        let runtime = located_runtime();
        let name = "stageman-job-sweep-probe";
        discard(&runtime, name).await.expect("a clean slate");
        let anything = build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");

        let created = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                name,
                "--label",
                &format!("{OWNER_LABEL}={name}"),
                "--network",
                "none",
                "--entrypoint",
                "sh",
                anything.as_argument(),
                "-c",
                "sleep 30",
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );

        let found = abandoned(&runtime).await.expect("the runtime answers");
        assert!(found.iter().any(|left| left == name), "{found:?}");

        discard(&runtime, name).await.expect("it is removable");

        let after = abandoned(&runtime).await.expect("the runtime answers");
        assert!(!after.iter().any(|left| left == name), "{after:?}");
        // Removing what is already gone is the outcome asked for, not a failure.
        discard(&runtime, name).await.expect("idempotent");
    }

    /// A published tunnel is reported by the runtime, and the report is read.
    ///
    /// The seam this feature turns on, and the one place a unit test cannot
    /// reach: the flag is asserted above, the parsing is asserted above, and
    /// what neither can say is whether a runtime asked about this container
    /// answers at all. Needs a runtime and a network, and no credential — it
    /// overrides the entry point rather than running an agent.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_published_tunnel_is_reported_by_the_runtime() {
        let runtime = located_runtime();
        let name = "stageman-job-tunnel-probe";
        discard(&runtime, name).await.expect("a clean slate");
        let anything = build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");

        let created = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                name,
                "--label",
                &format!("{OWNER_LABEL}={name}"),
                // The same mapping `retained_arguments` emits, asserted there
                // as a string and exercised here as a mapping.
                "--publish",
                &format!("127.0.0.1::{TUNNEL_PORT}"),
                "--entrypoint",
                "sh",
                anything.as_argument(),
                "-c",
                "sleep 30",
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );

        let port = tunnel_port(&runtime, name)
            .await
            .expect("the runtime answers");
        assert!(
            port.is_some_and(|port| port != 0),
            "a published mapping has a host port: {port:?}",
        );

        discard(&runtime, name).await.expect("it is removable");
        assert!(
            tunnel_port(&runtime, name).await.is_err(),
            "a container that is gone cannot be reached, and says so",
        );
    }

    mod costs_a_credential {
        use super::*;

        /// The credential, from the gitignored file this project keeps it in.
        ///
        /// Panics rather than skipping when it is absent. A test that quietly
        /// passes because it could not run is the failure mode the ignored
        /// tests above are arranged to avoid, and it would be perverse to
        /// reintroduce it here.
        fn credential() -> Secret {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../.local/anthropic-token");
            let raw = std::fs::read_to_string(path)
                .expect("write an agent credential to .local/anthropic-token (it is gitignored)");
            Secret::new(raw.trim().to_owned())
        }

        fn handout_of() -> (State, Handout) {
            let mut state = State::default();
            state.agents.insert(
                Agent::Claude,
                AgentConfig {
                    auth_token: credential(),
                },
            );
            let project = ProjectId::from_uuid(Uuid::from_u128(3));
            state.projects.insert(
                project,
                Project {
                    name: "probe".to_owned(),
                    repository: "https://example.invalid/repo".to_owned(),
                    foreman_agent: Agent::Claude,
                    job_agents: only_claude(),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
                },
            );
            let handout = Handout::for_foreman(&state, project).expect("a watched project");
            (state, handout)
        }

        #[tokio::test]
        #[ignore = "needs a container runtime, a built image and a credential; run `just image-session`"]
        async fn an_agent_answers_a_question() {
            let runtime = located_runtime();
            let (_state, handout) = handout_of();

            let answer = ask(
                &runtime,
                &handout,
                None,
                "Reply with exactly one word, lowercase, no punctuation: pong",
            )
            .await
            .expect("the agent answers");

            assert_eq!(answer.stop_reason, StopReason::EndTurn);
            assert!(
                answer.text.to_lowercase().contains("pong"),
                "said {:?}",
                answer.text
            );
        }

        /// The measurement `docs/decisions/0015-a-job-survives-the-daemon-dying.md`
        /// rests on, as a test rather than as a paragraph. A container that
        /// has stopped still holds its session, and an agent restarted inside
        /// it still has the conversation.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image and a credential; run `just image-session`"]
        async fn a_session_outlives_the_container_stopping() {
            let runtime = located_runtime();
            let (_state, handout) = handout_of();
            let name = "stageman-job-resume-probe";
            discard(&runtime, name).await.expect("a clean slate");

            let first = begin(
                &runtime,
                &handout,
                name,
                None,
                "Remember this word and reply with it, alone: marmalade",
            )
            .await
            .expect("the agent answers");
            assert!(
                first.text.to_lowercase().contains("marmalade"),
                "said {:?}",
                first.text
            );

            // The container has stopped by now — the conversation ending closes
            // its standard input, which is the same thing a hard kill does.
            let left = abandoned(&runtime).await.expect("the runtime answers");
            assert!(left.iter().any(|c| c == name), "it should still be there");

            let second = resume(
                &runtime,
                name,
                None,
                "What was the word I asked you to remember? Reply with it alone.",
            )
            .await
            .expect("the session is still there");

            assert!(
                second.text.to_lowercase().contains("marmalade"),
                "it did not remember: {:?}",
                second.text
            );
            discard(&runtime, name).await.expect("it is removable");
        }

        /// The harder half, and the one the design actually has to survive:
        /// cut off mid-turn rather than between turns. Dropping the future
        /// kills the client exactly as a hard kill of the daemon would.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image and a credential; run `just image-session`"]
        async fn a_turn_cut_off_partway_can_still_be_picked_up() {
            let runtime = located_runtime();
            let (_state, handout) = handout_of();
            let name = "stageman-job-midturn-probe";
            discard(&runtime, name).await.expect("a clean slate");

            let cut_short = tokio::time::timeout(
                std::time::Duration::from_secs(6),
                begin(
                    &runtime,
                    &handout,
                    name,
                    None,
                    "Count from 1 to 40, one number per line, pausing two seconds between each.",
                ),
            )
            .await;
            assert!(cut_short.is_err(), "it should not have finished in time");

            let picked_up = resume(
                &runtime,
                name,
                None,
                "You were interrupted. In one short line, what were you doing?",
            )
            .await
            .expect("the interrupted session is still there");

            assert_eq!(picked_up.stop_reason, StopReason::EndTurn);
            assert!(
                !picked_up.text.trim().is_empty(),
                "it should be able to say what it was doing"
            );
            discard(&runtime, name).await.expect("it is removable");
        }
    }
}
