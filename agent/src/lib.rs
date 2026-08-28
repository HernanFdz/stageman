//! The contract every coding agent is driven through, and the adapters that
//! implement it.
//!
//! Two shapes of use and one contract. A one-shot structured query is how the
//! orchestrator thinks; a long-running session in a workspace is how a job
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
    ContentBlock, InitializeRequest, InitializeResponse, ListSessionsRequest, LoadSessionRequest,
    NewSessionRequest, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionId, SessionNotification,
    SessionUpdate, TextContent,
};
use agent_client_protocol::{ByteStreams, Client, ConnectionTo};
use parking_lot::Mutex;
use stageman_core::{Agent, Handout, Platform, Secret};
use tokio::io::AsyncReadExt as _;
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

/// The image tag an agent's container is started from.
///
/// Adapter knowledge, and compiled in for the same reason the agent set is
/// closed: an image is code, so an image an operator could name is one they
/// could name wrongly. `project.just` builds under this tag, and a test in
/// this module holds the two together.
#[must_use]
pub const fn image(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "stageman/claude:dev",
    }
}

/// The arguments that start a container just long enough to be greeted.
///
/// Pure, so what a container is started with can be asserted without starting
/// one. No network and no workspace: nothing before the first prompt needs
/// either, and a check that reaches the internet is a check that fails for
/// reasons unrelated to what it tests.
const fn handshake_arguments(agent: Agent) -> [&'static str; 6] {
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
        image(agent),
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
/// Fails if the runtime cannot be started, if the container exits without
/// speaking — a missing image, most often — or if the exchange itself does not
/// complete. The three are separate variants because an operator acts on them
/// differently.
pub async fn handshake(runtime: &ContainerRuntime, agent: Agent) -> Result<Greeting, AgentError> {
    greet(runtime, &handshake_arguments(agent)).await
}

/// Runs the runtime with `arguments` and greets whatever answers.
///
/// Split from [`handshake`] only so that the failure paths can be reached from
/// a test: which image is run is the difference between a container that
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
fn session_arguments(agent: Agent, delivering: &[(&'static str, Secret)]) -> Vec<String> {
    let mut arguments = vec![
        "run".to_owned(),
        "--rm".to_owned(),
        "--interactive".to_owned(),
    ];
    for (name, _) in delivering {
        arguments.push("--env".to_owned());
        arguments.push((*name).to_owned());
    }
    // Deliberately no `--network none` here, unlike the handshake: reaching a
    // model needs the network, and so does cloning. Which hosts it *ought* to
    // reach is the egress allowlist still open in `docs/open-questions.md`.
    arguments.push(image(agent).to_owned());
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
/// orchestrator thinks, rather than how a job works. The container is started,
/// asked and destroyed.
///
/// **It starts a container per question, and
/// `docs/decisions/0012-agents-run-in-containers.md` says the orchestrator's
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
    question: &str,
) -> Result<Answer, AgentError> {
    let delivering = variables(handout);
    let container = spawn(
        runtime,
        &session_arguments(handout.agent(), &delivering),
        &delivering,
    )?;
    converse(container, Opening::Fresh, question).await
}

/// The label every container this project starts carries.
///
/// How a container that outlives the process which started it is found again.
/// A name addresses one; this finds them all, including the ones whose job the
/// instance has forgotten. That is what makes `docs/conventions.md` §4's
/// "nothing untracked" checkable rather than merely intended — a sweep able to
/// see only what the snapshot already knew would never find the case it exists
/// for.
const OWNER_LABEL: &str = "stageman.job";

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
    agent: Agent,
    delivering: &[(&'static str, Secret)],
) -> Vec<String> {
    let mut arguments = session_arguments(agent, delivering);
    arguments.retain(|argument| argument != "--rm");
    // Spliced in after `run` rather than pushed: everything between the
    // subcommand and the image is a flag, and the image has to stay last.
    arguments.splice(
        1..1,
        [
            "--name".to_owned(),
            name.to_owned(),
            "--label".to_owned(),
            format!("{OWNER_LABEL}={name}"),
        ],
    );
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
pub async fn begin(
    runtime: &ContainerRuntime,
    handout: &Handout,
    name: &str,
    question: &str,
) -> Result<Answer, AgentError> {
    let delivering = variables(handout);
    let container = spawn(
        runtime,
        &retained_arguments(name, handout.agent(), &delivering),
        &delivering,
    )?;
    converse(container, Opening::Fresh, question).await
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
    question: &str,
) -> Result<Answer, AgentError> {
    settle(runtime, name).await?;
    let arguments = [
        "start".to_owned(),
        "--interactive".to_owned(),
        name.to_owned(),
    ];
    let container = spawn(runtime, &arguments, &[])?;
    converse(container, Opening::Resumed, question).await
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
/// `None` means there was nothing to pick up, which is a container stopped
/// before its agent said anything: sessions are written when something is
/// said, not when one is created.
async fn open_session(
    connection: &ConnectionTo<AgentRole>,
    opening: Opening,
) -> Result<Option<SessionId>, agent_client_protocol::Error> {
    match opening {
        Opening::Fresh => Ok(Some(
            connection
                .send_request(NewSessionRequest::new(PathBuf::from(WORKSPACE)))
                .block_task()
                .await?
                .session_id,
        )),
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
            connection
                .send_request(LoadSessionRequest::new(
                    found.session_id.clone(),
                    PathBuf::from(WORKSPACE),
                ))
                .block_task()
                .await?;
            Ok(Some(found.session_id))
        }
    }
}

/// Speaks the protocol to a started container and puts one question to it.
async fn converse(
    mut container: tokio::process::Child,
    opening: Opening,
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

                    let Some(session_id) = open_session(&connection, opening).await? else {
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

    /// The tag the image is built under, read from the recipe that builds it.
    ///
    /// Two files have to agree on this string and no compiler can make them:
    /// `project.just` cannot read a Rust constant, and the constant cannot be
    /// generated without putting a build step in front of every check. So the
    /// agreement is asserted instead, which is the same trade the drift checks
    /// make everywhere else.
    fn tag_in_the_recipe() -> String {
        let recipe =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../project.just"))
                .expect("project.just sits beside this crate");
        recipe
            .lines()
            .find_map(|line| line.strip_prefix("image_tag :="))
            .map(|value| value.trim().trim_matches('"').to_owned())
            .expect("project.just declares image_tag")
    }

    #[test]
    fn the_recipe_builds_the_tag_the_adapter_runs() {
        assert_eq!(image(Agent::Claude), tag_in_the_recipe());
    }

    #[test]
    fn a_handshake_asks_for_no_network_and_leaves_nothing_behind() {
        let arguments = handshake_arguments(Agent::Claude);
        assert_eq!(arguments[0], "run");
        assert!(arguments.contains(&"--rm"));
        assert!(arguments.contains(&"--interactive"));
        assert_eq!(arguments[3..5], ["--network", "none"]);
    }

    #[test]
    fn a_handshake_runs_the_image_that_agent_lives_in() {
        let arguments = handshake_arguments(Agent::Claude);
        assert_eq!(arguments.last(), Some(&image(Agent::Claude)));
    }

    #[test]
    fn a_runtime_keeps_the_path_it_was_configured_with() {
        let runtime = ContainerRuntime::new(PathBuf::from("/usr/local/bin/docker"));
        assert_eq!(runtime.path(), Path::new("/usr/local/bin/docker"));
    }

    #[tokio::test]
    async fn a_runtime_that_is_not_there_fails_as_a_runtime_rather_than_a_protocol() {
        let runtime = ContainerRuntime::new(PathBuf::from("/nonexistent/container/runtime"));
        let failure = handshake(&runtime, Agent::Claude).await;
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

    /// Drives a real container, so it needs a runtime and a built image.
    ///
    /// Ignored by default rather than absent: `just check` stays a gate you can
    /// run constantly, and nextest still counts this as ignored — which is the
    /// distinction that matters, because a test behind a `cfg` nobody selected
    /// appears nowhere and the total still reads as complete. Run it with
    /// `just image-handshake`.
    #[tokio::test]
    #[ignore = "needs a container runtime and a built image; run `just image-handshake`"]
    async fn a_container_answers_the_protocol() {
        let runtime = located_runtime();

        let greeting = handshake(&runtime, Agent::Claude)
            .await
            .expect("the image answers the handshake");

        assert_eq!(greeting.protocol_version, ProtocolVersion::V1);
        let adapter = greeting.adapter.expect("the adapter names itself");
        assert!(!adapter.name.is_empty());
        assert!(!adapter.version.is_empty());
    }

    /// A container that fills its error pipe must not become a hang.
    ///
    /// The pipe's kernel buffer is around sixty-four kilobytes and the bound on
    /// what is *kept* is eight, so a container printing two hundred would block
    /// on the write under either of the two mistakes this guards: draining
    /// after the exchange instead of during it, or stopping the drain at the
    /// bound. It reuses the agent's own image with the entry point overridden,
    /// so it needs nothing built that the tests above do not already need.
    #[tokio::test]
    #[ignore = "needs a container runtime and a built image; run `just image-handshake`"]
    async fn a_container_that_floods_its_error_pipe_does_not_hang() {
        let runtime = located_runtime();

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
                image(Agent::Claude),
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

    /// An image nobody ever built must not read as an agent that cannot speak.
    ///
    /// Both fail as silence on the connection, and the error an operator gets
    /// is the only thing that distinguishes "build the image" from "the
    /// adapter is broken". This needs a runtime but deliberately no image,
    /// which is the whole point of it.
    #[tokio::test]
    #[ignore = "needs a container runtime; run `just image-handshake`"]
    async fn an_image_that_was_never_built_fails_as_a_container_not_a_protocol() {
        let runtime = located_runtime();

        let failure = greet(
            &runtime,
            &[
                "run",
                "--rm",
                "--interactive",
                "--network",
                "none",
                "stageman/no-such-image-was-ever-built:dev",
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
    fn instance(credential: &str) -> State {
        State::new(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new(credential.to_owned()),
            },
            PathBuf::from("/usr/local/bin/container-runtime"),
        )
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
                credentials,
                jobs: BTreeMap::<_, Job>::new(),
            },
        );
        (state, id)
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
    fn triage_is_delivered_its_credential_and_nothing_else() {
        let state = instance("sk-ant-oat01-xyz");
        let handout = Handout::for_triage(&state).expect("a configured instance");

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
        let (state, project) = instance_with_a_project("sk-ant-oat01-secret-value");
        let handout = Handout::for_job(&state, Agent::Claude, project).expect("a watched project");

        let arguments = session_arguments(handout.agent(), &variables(&handout));
        let line = arguments.join(" ");

        assert!(!line.contains("sk-ant-oat01-secret-value"), "{line}");
        assert!(!line.contains("gh-not-a-real-token"), "{line}");
        assert!(line.contains("--env CLAUDE_CODE_OAUTH_TOKEN"), "{line}");
        assert!(line.contains("--env GH_TOKEN"), "{line}");
    }

    #[test]
    fn a_session_container_is_not_cut_off_from_the_network() {
        let state = instance("sk-ant-oat01-xyz");
        let handout = Handout::for_triage(&state).expect("a configured instance");

        let arguments = session_arguments(handout.agent(), &variables(&handout));

        assert!(!arguments.iter().any(|a| a == "none"), "{arguments:?}");
        assert_eq!(
            arguments.last().map(String::as_str),
            Some(image(Agent::Claude))
        );
    }

    /// Tests that spend real money, kept in their own module so a filter can
    /// name them as a group rather than one at a time. Run with
    /// `just image-session`; `just image-handshake` deliberately excludes them,
    /// because everything it runs needs only a runtime and an image.

    #[test]
    fn a_retained_container_is_named_labelled_and_survives_its_own_exit() {
        let state = instance("sk-ant-oat01-xyz");
        let handout = Handout::for_triage(&state).expect("a configured instance");

        let arguments = retained_arguments("stageman-job-abc", Agent::Claude, &variables(&handout));
        let line = arguments.join(" ");

        assert!(line.contains("--name stageman-job-abc"), "{line}");
        assert!(
            line.contains("--label stageman.job=stageman-job-abc"),
            "{line}"
        );
        assert_eq!(
            arguments.first().map(String::as_str),
            Some("run"),
            "the subcommand has to stay first"
        );
        assert_eq!(
            arguments.last().map(String::as_str),
            Some(image(Agent::Claude)),
            "the image has to stay last"
        );
        // The whole difference between a container that survives being killed
        // and one that vanishes with it.
        assert!(!arguments.iter().any(|a| a == "--rm"), "{line}");
    }

    /// A container this project started, found without consulting the instance
    /// and then removed. Needs a runtime and an image, and no credential: it
    /// overrides the entry point rather than running an agent.
    #[tokio::test]
    #[ignore = "needs a container runtime and a built image; run `just image-handshake`"]
    async fn a_container_this_project_started_is_found_by_label_and_discarded() {
        let runtime = located_runtime();
        let name = "stageman-job-sweep-probe";
        discard(&runtime, name).await.expect("a clean slate");

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
                image(Agent::Claude),
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

        fn handout_of(runtime: &ContainerRuntime) -> (State, Handout) {
            let state = State::new(
                Agent::Claude,
                AgentConfig {
                    auth_token: credential(),
                },
                runtime.path().to_owned(),
            );
            let handout = Handout::for_triage(&state).expect("a configured instance");
            (state, handout)
        }

        #[tokio::test]
        #[ignore = "needs a container runtime, a built image and a credential; run `just image-session`"]
        async fn an_agent_answers_a_question() {
            let runtime = located_runtime();
            let (_state, handout) = handout_of(&runtime);

            let answer = ask(
                &runtime,
                &handout,
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
            let (_state, handout) = handout_of(&runtime);
            let name = "stageman-job-resume-probe";
            discard(&runtime, name).await.expect("a clean slate");

            let first = begin(
                &runtime,
                &handout,
                name,
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
            let (_state, handout) = handout_of(&runtime);
            let name = "stageman-job-midturn-probe";
            discard(&runtime, name).await.expect("a clean slate");

            let cut_short = tokio::time::timeout(
                std::time::Duration::from_secs(6),
                begin(
                    &runtime,
                    &handout,
                    name,
                    "Count from 1 to 40, one number per line, pausing two seconds between each.",
                ),
            )
            .await;
            assert!(cut_short.is_err(), "it should not have finished in time");

            let picked_up = resume(
                &runtime,
                name,
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
