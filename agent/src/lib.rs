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

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{InitializeRequest, InitializeResponse};
use agent_client_protocol::{ByteStreams, Client, ConnectionTo};
use stageman_core::Agent;
use tokio::io::AsyncReadExt as _;
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};

// The protocol library calls the far end of a connection `Agent` too, and it
// means the role rather than the product. Importing it under another name is
// not decoration: `ConnectionTo<Agent>` would compile against either type and
// read correctly whichever one it resolved to, which is precisely the kind of
// ambiguity `docs/conventions.md` §2 says to spend a word avoiding.
use agent_client_protocol::Agent as AgentRole;

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

/// An agent's container could not be reached.
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
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
pub async fn handshake(
    runtime: &ContainerRuntime,
    agent: Agent,
) -> Result<Greeting, HandshakeError> {
    greet(runtime, &handshake_arguments(agent)).await
}

/// Runs the runtime with `arguments` and greets whatever answers.
///
/// Split from [`handshake`] only so that the failure paths can be reached from
/// a test: which image is run is the difference between a container that
/// greets and one that never starts, and taking the arguments here is what
/// lets a test ask for the second without the agent set having to contain a
/// deliberately broken member.
async fn greet(runtime: &ContainerRuntime, arguments: &[&str]) -> Result<Greeting, HandshakeError> {
    let mut container = tokio::process::Command::new(runtime.path())
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // So that a dropped future does not leave a container attached to a
        // parent that has stopped reading it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| HandshakeError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    let (Some(to_agent), Some(from_agent), Some(complaints)) = (
        container.stdin.take(),
        container.stdout.take(),
        container.stderr.take(),
    ) else {
        return Err(HandshakeError::NoChannel);
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
    let status = container.wait().await.map_err(HandshakeError::Exit)?;

    match spoken {
        Ok(greeting) => Ok(greeting),
        // A container that failed on its own terms explains itself better than
        // the protocol error its silence produced, so it wins when both exist.
        Err(_) if !status.success() => Err(HandshakeError::Container {
            status: status.to_string(),
            message: printed,
        }),
        Err(protocol) => Err(HandshakeError::Protocol(protocol)),
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
        assert!(matches!(failure, Err(HandshakeError::Runtime { .. })));
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

        let Err(HandshakeError::Container { status, message }) = failure else {
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
            matches!(failure, Err(HandshakeError::Container { .. })),
            "expected a container failure, got {failure:?}"
        );
    }
}
