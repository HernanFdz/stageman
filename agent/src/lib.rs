//! The contract every coding agent is driven through, and the adapters that
//! implement it.
//!
//! One contract: a conversation with an agent over a session in a workspace,
//! which is how a foreman thinks and how a job works alike. Both reach a
//! model only by running a configured agent, never through a vendor's own
//! service API — see
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
//! vocabulary comes from its Rust library, which is
//! `docs/decisions/0014-the-protocols-own-sdk-and-our-own-spawning.md`, and
//! since
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
//! nothing here starts a process or speaks to one: every command a turn runs
//! is a value rendered here and read back here, and the conversation is a
//! machine — [`Conversation`] — that the instance steps one line at a time.
//! What is left that drives a runtime is the probe's, and the container
//! tests', which drive the same argument lists the instance renders.

mod conversation;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub use conversation::{Conversation, Exchange, Heard, Noticed, Opening, Said, Steered};

use agent_client_protocol::schema::v1::{
    HttpHeader, McpServer, McpServerHttp, SessionConfigKind, SessionConfigOption,
};
use sha2::{Digest as _, Sha256};
#[cfg(test)]
use stageman_core::Channel;
use stageman_core::{
    Agent, ClaudeEffort, ClaudeModel, Handout, InstanceId, Kit, Platform, Role, Secret, Uuid,
};
use tokio::io::AsyncWriteExt as _;

/// Re-exported because they appear in this crate's own signatures.
///
/// `docs/decisions/0014-the-protocols-own-sdk-and-our-own-spawning.md` accepted
/// that protocol types would surface here rather than being wrapped. Accepting
/// that and then not re-exporting them would leave [`Answer`] unreadable to
/// anyone who has not also taken the protocol library as a direct dependency,
/// which is the cost without the benefit.
pub use agent_client_protocol::schema::ProtocolVersion;
pub use agent_client_protocol::schema::v1::StopReason;
pub use agent_client_protocol::schema::v1::ToolCallStatus;
pub use agent_client_protocol::schema::v1::ToolKind;

/// How much of a failed agent's standard error is kept in what is recorded.
///
/// Bounded because the message travels into a record an operator reads, and
/// an agent that fails by printing megabytes would otherwise turn one
/// unreadable failure into a second one. The world keeps more than this and
/// reads past even that, so nothing here is what stops a flooded pipe.
pub(crate) const STDERR_LIMIT: usize = 8 * 1024;

/// Which platform this binary was made for, as far as anything here needs to
/// care.
///
/// Handed to the instance at construction rather than read inside it, so that
/// what a start does is a function of what it was given: a flow recorded on
/// one platform replays on another by handing the replay the target the file
/// names, and a test on any machine can ask what a start would do on a
/// platform this is not. See
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    /// A mac.
    MacOs,
    /// A Linux machine.
    Linux,
    /// A Windows machine.
    Windows,
    /// Anything else, which knows nowhere to look for a runtime and says so
    /// rather than failing to build.
    Unknown,
}

impl Target {
    /// What this binary was made for.
    ///
    /// The one place a compile-time condition decides anything about a
    /// platform. Everything downstream branches on the value instead, which
    /// is what lets the same build answer for a platform it is not running
    /// on.
    #[must_use]
    pub const fn compiled() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::Unknown
        }
    }
}

/// Where a container runtime is looked for on a mac, in order.
const MACOS_CANDIDATES: &[&str] = &[
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/Applications/Docker.app/Contents/Resources/bin/docker",
    "/opt/homebrew/bin/podman",
    "/usr/local/bin/podman",
];

/// Where a container runtime is looked for on a Linux machine, in order.
const LINUX_CANDIDATES: &[&str] = &[
    "/usr/bin/docker",
    "/usr/local/bin/docker",
    "/snap/bin/docker",
    "/usr/bin/podman",
    "/usr/local/bin/podman",
];

/// Where a container runtime is looked for on a Windows machine, in order.
const WINDOWS_CANDIDATES: &[&str] = &[
    r"C:\Program Files\Docker\Docker\resources\bin\docker.exe",
    r"C:\Program Files\RedHat\Podman\podman.exe",
];

/// Every place a runtime is looked for on that platform, in order, for a
/// start that has to try them and for a refusal that has to say where it
/// looked.
///
/// Absolute paths and never a search of `PATH`, which is the whole of what
/// `docs/conventions.md` §3 forbids: an inherited variable differs between
/// the shell you tested in and what a service manager supplies, and a list
/// compiled in does not.
///
/// Ordered, and the order is a decision rather than an accident. Docker
/// first because it is what most machines that have anything have; the
/// package manager locations before the system ones on each platform,
/// because a hand-installed runtime is the one somebody chose. A machine
/// with both gets the first, and that is the cost of not asking — see
/// `docs/decisions/0023-the-container-runtime-is-discovered-once.md`.
///
/// A platform nothing here knows gets an empty list, so it still builds and
/// refuses honestly at startup with "none found", which is a message
/// somebody can act on unlike a build that will not finish.
#[must_use]
pub const fn candidates(target: Target) -> &'static [&'static str] {
    match target {
        Target::MacOs => MACOS_CANDIDATES,
        Target::Linux => LINUX_CANDIDATES,
        Target::Windows => WINDOWS_CANDIDATES,
        Target::Unknown => &[],
    }
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
}

/// The base every recipe starts from.
///
/// Shared by every agent rather than chosen per agent, and that is a decision
/// rather than a convenience — `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`.
/// Everything composed after it assumes what is in it, so an adapter free to
/// bring its own base would invalidate every other fragment and every
/// container test that establishes they work together.
const BASE: &str = include_str!("../images/base.Dockerfile");

/// What holds a container open once the agent that ran in it has stopped.
///
/// Composed after an agent's fragment and never before it, which is what makes
/// its command the last one any recipe names: an adapter's fragment cannot
/// replace it, because a later `CMD` is what would have to. A property of the
/// order rather than a rule anybody keeps.
const HOLDING: &str = include_str!("../images/holding.fragment");

/// What a job needs in order to reach a repository, and a foreman does not.
///
/// Last, so that a job's recipe is a foreman's with this appended. The
/// instructions before it are then identical byte for byte, which is what
/// makes the two images share every layer up to this one and the second build
/// a cache hit throughout — measured, and the property
/// `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md` used two stages to
/// get.
const PLATFORM: &str = include_str!("../images/platform.fragment");

/// What installs one agent's adapter.
///
/// The only fragment that is genuinely one agent's, which is what the split
/// into fragments is for. Adapter knowledge, for the same reason the agent set
/// is closed: an image is code.
const fn installing(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => include_str!("../images/claude/install.fragment"),
    }
}

/// The recipe one role's image is built from, composed from its fragments.
///
/// Composed here rather than written out per agent, because the alternative is
/// a near-identical copy of the base, the holding command and the platform
/// layer for every adapter — and copies drift on exactly the pinned versions
/// that pinning exists to hold. Concatenated rather than templated: a
/// placeholder would move a Dockerfile fact into this file, and every fragment
/// is meant to stay text a runtime could build.
///
/// Nothing separates the pieces, so each fragment ends in a newline and a test
/// says so. Gluing two instructions into one line is the failure that would
/// otherwise be silent until a build fails somewhere unrelated.
///
/// Public because the instance builds: since
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
/// a build is a command rendered there with this on its standard input.
#[must_use]
pub fn recipe(agent: Agent, role: Role) -> String {
    let mut composed = String::from(BASE);
    composed.push_str(installing(agent));
    composed.push_str(HOLDING);
    if matches!(role, Role::Job) {
        composed.push_str(PLATFORM);
    }
    composed
}

/// The repository every image this project builds is named under.
///
/// One name for all of them, with the recipe's digest as the tag, so that a
/// listing of what this project has built is one query and a name of ours can
/// never collide with anything else on the daemon.
const REPOSITORY: &str = "stageman";

/// An image, named by the recipe it is built from.
///
/// The name is the digest of the exact bytes handed to the build, so two
/// containers wanting the same recipe want the same image and get it — which
/// is the whole of
/// `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`.
/// It is still not a name an operator chooses: nothing can be stale under it,
/// because a changed recipe is a changed name.
///
/// Opaque on purpose. The only things this project does with one are start a
/// container from it and ask whether it is there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image(String);

impl Image {
    /// The name, as the runtime wants it on a command line.
    #[must_use]
    pub fn as_argument(&self) -> &str {
        &self.0
    }
}

/// What one recipe's image is called.
///
/// Over the recipe rather than over the fragments or anything this crate
/// arranges, so the name has a definition outside this binary: it is the
/// digest of what the build was given, and nothing else goes into it.
///
/// The digest formats itself as hexadecimal, which is a trait the digest
/// crate's own dependency provides. Reached for rather than reimplemented: the
/// alternatives are a formatting loop whose only error is one that cannot
/// happen, or a table whose only failure is an index that cannot be out of
/// range, and both are the shape `.quality/gate-reference.md` warns about —
/// code pretending to handle something impossible.
#[must_use]
pub fn named(recipe: &str) -> Image {
    Image(format!(
        "{REPOSITORY}:{:x}",
        Sha256::digest(recipe.as_bytes())
    ))
}

/// How much of a failed build is kept, in lines counted from the end.
///
/// From the end rather than the beginning, which is the opposite of
/// [`printed`] and deliberate: a container that fails says so immediately, and
/// a build that fails says so after however many layers succeeded first.
const BUILD_TAIL: usize = 12;

/// The arguments that build a recipe under the name it hashes to.
///
/// A build with no context at all — the trailing `-` — because the recipe is
/// the only input there is. `docs/decisions/0034-tools-are-served-not-shipped.md`
/// is what keeps that true: nothing this project writes goes in the image, so
/// there is nothing a context could carry.
///
/// Quiet, because the name is already known here and the identifier a build
/// prints is of no further use. What a build has to say about itself still
/// arrives on standard error, which is where the failure message comes from.
fn build_arguments(image: &Image) -> Vec<String> {
    Command::Build {
        image: image.as_argument().to_owned(),
    }
    .arguments()
}

/// The arguments that ask whether an image is already here.
fn present_arguments(image: &Image) -> Vec<String> {
    Command::Present {
        image: image.as_argument().to_owned(),
    }
    .arguments()
}

/// Whether the runtime already holds this image.
///
/// Total, and that is the honest signature rather than a convenience: a
/// runtime that cannot answer is one the build a moment later fails on loudly,
/// so a second error path here would report the same thing twice and earlier.
///
/// Skipped by mutation testing, like everything here that drives the runtime:
/// what it does is spawn a process and read an exit status.
#[mutants::skip]
async fn present(runtime: &ContainerRuntime, image: &Image) -> bool {
    tokio::process::Command::new(runtime.path())
        .args(present_arguments(image))
        .kill_on_drop(true)
        .output()
        .await
        .is_ok_and(|asked| asked.status.success())
}

/// One build at a time, for as long as this process lives.
///
/// Two containers starting together would otherwise both find their image
/// absent and build it, and the second build would move the name onto its own
/// copy and leave the first's unreferenced — which is measured to *delete* it,
/// taking the image record out from under a container created from it a moment
/// earlier. Serialising also makes the second build free rather than
/// duplicated, because it finds what the first one left.
///
/// It serialises two builds of *different* images too, which is a cost worth
/// naming: a foreman and a job starting at once wait for one another. They
/// share every layer but the last, so the second is a cache hit either way.
static BUILDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Makes sure the image one container will run exists, and answers with its
/// name.
///
/// **Built only if it is not already here**, which is the reversal
/// `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`
/// records. Freshness is not given up with the unconditional build: the name
/// *is* the recipe, so an image found under it was built from these exact
/// bytes and an edited recipe asks for a name nothing has. What building
/// anyway would cost is measured and severe — the name moves to a new copy,
/// and the old one is deleted from under every container already using it.
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
    let recipe = recipe(agent, role);
    let image = named(&recipe);

    // Held across the check and the build both, because the two are one
    // decision: a gap between them is where the second builder gets its
    // answer wrong.
    let _building = BUILDING.lock().await;
    if present(runtime, &image).await {
        return Ok(image);
    }

    let mut building = tokio::process::Command::new(runtime.path())
        .args(build_arguments(&image))
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
        return Err(AgentError::Build {
            message: "the runtime offered no standard input for the recipe".to_owned(),
        });
    };
    // Written whole and then closed, which is safe only because a recipe is a
    // few kilobytes and a pipe buffer is tens of them. A larger one would have
    // to be written while the output is drained, for the reason [`greet`]
    // drains standard error while the exchange happens.
    writing
        .write_all(recipe.as_bytes())
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
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    outcome(finished.status.success(), image, &finished.stderr)
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
fn outcome(succeeded: bool, image: Image, complained: &[u8]) -> Result<Image, AgentError> {
    if succeeded {
        Ok(image)
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
///
/// Public because the instance reads a build's outcome, and this is what a
/// failed one says.
#[must_use]
pub fn last_words(said: &[u8]) -> String {
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

/// The arguments that list every image this project has built.
///
/// By reference rather than by label, because an image carries no label of
/// this project's: what it carries is a name, and the name is ours by its
/// repository. Both runtimes match every tag under a repository named this
/// way — measured, because a filter that silently matched only one tag would
/// make the sweep below reclaim nothing and say it swept.
fn ours_arguments() -> Vec<String> {
    vec![
        "images".to_owned(),
        "--filter".to_owned(),
        format!("reference={REPOSITORY}"),
        "--format".to_owned(),
        "{{.Repository}}:{{.Tag}}".to_owned(),
    ]
}

/// The image names in what a listing reported.
///
/// Pure, so what a sweep works from can be tested without a runtime. Two
/// things are dropped rather than carried: a blank line, which is what a
/// runtime prints when it found nothing, and anything still carrying the
/// runtime's word for *no name*, which is an image that lost its name between
/// the listing and now and is not one this project can address.
///
/// Public because the instance reads it: the listing is asked for there, and
/// this is what its lines mean.
#[must_use]
pub fn tagged(reported: &str) -> Vec<String> {
    reported
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.contains("<none>"))
        .map(str::to_owned)
        .collect()
}

/// Every image this build would use, which is what a sweep keeps.
///
/// Kept rather than reclaimed and rebuilt, for two reasons and the second is
/// the one that decides it. A rebuild is wasted work when the next container
/// wants exactly this. And a sweep that removed them would race the path that
/// has just built one and not yet created its container, which is a job
/// failing on an image that was there a moment ago.
///
/// Public, with [`kept`], because the instance decides what a sweep removes:
/// since
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
/// the listing is asked for there and read there, and what is left is this —
/// knowledge about images rather than about one run.
#[must_use]
pub fn keeping() -> Vec<Image> {
    Agent::ALL
        .iter()
        .copied()
        .flat_map(|agent| {
            Role::ALL
                .iter()
                .copied()
                .map(move |role| named(&recipe(agent, role)))
        })
        .collect()
}

/// Whether a name is one of the images this build would use.
///
/// Named and public, for the reason [`keeping`] is: the sweep around it is
/// the instance's now, and this is the decision that says whether anything
/// is removed at all. Inverted, a sweep reclaims exactly the images the next
/// container wants and keeps the ones nothing will ask for again.
#[must_use]
pub fn kept(keeping: &[Image], image: &str) -> bool {
    keeping.iter().any(|keep| keep.as_argument() == image)
}

/// An agent in a container could not be reached, or would not answer.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// A project's variable claims a name this project delivers itself.
    ///
    /// Refused rather than resolved, because both ways of resolving it are
    /// silently wrong: discarding the operator's variable loses something they
    /// set, and honouring it can change which account an agent bills — see
    /// `docs/decisions/0008-one-credential-per-agent.md` and
    /// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.
    ///
    /// The dashboard refuses such a name as it is entered, so this is what a
    /// hand-edited snapshot reaches. It fails the job rather than the
    /// instance, which is the distinction `docs/conventions.md` §3 draws:
    /// an operator can act on it, and everything else still works.
    ///
    /// Carries the name and never a value. A name is not a credential.
    #[error("the variable {name} is one stageman delivers itself")]
    ReservedVariable {
        /// The name that collided.
        name: String,
    },
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
    /// The repository could not be checked out before the agent's first turn.
    ///
    /// Its own variant rather than [`AgentError::Container`], because the
    /// container is fine and the agent never ran: this is a credential that
    /// does not open the repository, or a URL that names nothing, failing
    /// before a model turn is spent — which is the point of doing it first,
    /// per
    /// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    #[error("the repository {repository} could not be checked out: {message}")]
    Checkout {
        /// What was asked for.
        repository: String,
        /// What the tool printed.
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
    /// The agent refused a setting its kit asked for.
    ///
    /// Before the first prompt, and in the adapter's own words, which name the
    /// option and the value. Every spelling this crate sends is held to what
    /// the pinned adapter accepts by a container test, so reaching this means
    /// the pin moved without the spelling following — see
    /// `docs/decisions/0048-a-job-runs-on-a-kit.md`. It fails the job rather
    /// than the instance: nothing else is wrong, and an operator can act on it.
    #[error("the agent refused to set {option} to {value}: {message}")]
    Refused {
        /// The option, as the adapter names it.
        option: String,
        /// The value asked for, as this crate spells it.
        value: String,
        /// What the adapter said.
        message: String,
    },
    /// The agent accepted a setting and then reported it unchanged.
    ///
    /// The failure the read-back exists to catch. An adapter that says yes and
    /// does nothing would run every job on whatever it defaults to, with every
    /// reply reading as success — the shape
    /// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
    /// was written about. Not this adapter, which was measured to change what
    /// it reports; the next one need not be so honest.
    #[error("the agent accepted {option} = {value} and then reported it unchanged")]
    Ignored {
        /// The option, as the adapter names it.
        option: String,
        /// The value asked for, as this crate spells it.
        value: String,
    },
    /// The container ran and spoke, but the exchange did not complete.
    #[error("the agent did not complete the protocol handshake")]
    Protocol(#[source] agent_client_protocol::Error),
    /// The agent's process ended cleanly without answering what it had been
    /// asked.
    ///
    /// Distinct from [`AgentError::Container`], which is a process that
    /// failed and said why, and from [`AgentError::Protocol`], which is a
    /// refusal: this one simply stopped talking, and the only thing to say
    /// about it is what it was asked.
    #[error("the agent stopped before answering {asked}")]
    Unanswered {
        /// What it was being asked, as the conversation names it.
        asked: &'static str,
    },
    /// The agent answered, and the answer could not be read as what was
    /// asked for.
    #[error("the agent's {what} could not be read: {why}")]
    Unreadable {
        /// What was asked for, as the conversation names it.
        what: &'static str,
        /// Why not.
        why: String,
    },
}

/// Where a job's agent works inside its container.
///
/// A directory rather than a mount: nothing delivers a repository, and an
/// agent that needs one clones it here — see
/// `docs/decisions/0016-the-agent-clones-the-repository.md`. The image already
/// makes this its working directory.
pub(crate) const WORKSPACE: &str = "/workspace";

/// The variables one agent's container is started with, and their values.
///
/// This is *delivery*, and the counterpart to the deciding that
/// [`stageman_core::Handout`] does. Which credentials a process may see is a
/// pure question about configuration and lives in the domain crate; what they
/// are called here is knowledge about one agent and lives in its adapter. See
/// `docs/conventions.md` §3.
/// Exactly the environment a container running this handout is given, in the
/// order it is set: the agent's own credential under the variable its
/// adapter reads, the platform credentials under the variables their tools
/// read, and the project's variables last, refused on collision.
///
/// Pure, so that whoever decides what a process is handed can decide this
/// too, and the world only sets it.
///
/// # Errors
///
/// Fails if a project's variable claims a name this project delivers itself,
/// which would change who pays — `docs/decisions/0008-one-credential-per-agent.md`.
pub fn environment(handout: &Handout) -> Result<Vec<(String, Secret)>, AgentError> {
    let mut set: Vec<(String, Secret)> = vec![match handout.agent() {
        Agent::Claude => (
            claude_credential_variable(handout.agent_credential()).to_owned(),
            handout.agent_credential().clone(),
        ),
    }];

    for (platform, credential) in handout.platforms() {
        set.push((
            match platform {
                // What the platform's own command-line tool reads, which is how
                // a job reaches it at all — see
                // `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`.
                Platform::GitHub => "GH_TOKEN".to_owned(),
            },
            credential.clone(),
        ));
    }

    // The project's own, last and refused on collision. Refused rather than
    // ordered around, because either order is a silent wrong answer: ours
    // winning discards a variable the operator set and said nothing, and
    // theirs winning changes who pays — which is
    // `docs/decisions/0008-one-credential-per-agent.md`'s failure arriving
    // through a door that record could not see.
    //
    // The dashboard refuses such a name when it is typed, so reaching this is
    // a snapshot that was hand-edited. Failing the job loudly is what
    // `docs/conventions.md` §3 asks for in that case: an operator can act on
    // it, and nothing else about the instance is broken.
    for (name, value) in handout.variables() {
        if RESERVED.contains(&name.as_str()) {
            return Err(AgentError::ReservedVariable {
                name: name.to_string(),
            });
        }
        set.push((name.to_string(), value.clone()));
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

    Ok(set)
}

/// Every name this project may deliver on its own account.
///
/// A project's variables may not claim one of these — see
/// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`. It
/// lives here rather than in the domain because what a credential is *called*
/// is knowledge about one agent, which is the rule `docs/conventions.md` §3
/// states; **app** is the crate allowed to see both halves and is what asks.
///
/// It names every variable any compiled-in adapter *could* deliver rather than
/// the ones a given handout will, and the difference matters: an agent added to
/// a project later must not turn a name that was accepted into a collision.
/// Both of Claude's are here for the same reason — which one is used depends on
/// the shape of the credential, so reserving only the one in force would make
/// the rule depend on a token an operator has not supplied yet.
///
/// Adding an agent means adding its names here. Nothing makes that automatic,
/// and the test below is what notices: it asserts that everything a real
/// handout delivers is in this list.
pub const RESERVED: &[&str] = &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY", "GH_TOKEN"];

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

/// The option the adapter calls its permission mode, and the value it is
/// pinned to.
///
/// Pinned rather than chosen, per
/// `docs/decisions/0049-the-permission-mode-is-pinned-not-chosen.md`: no mode
/// the adapter offers allows everything, the two that never prompt both deny,
/// and a value inherited from the adapter's own default is one an adapter
/// release can move underneath every job at once. Approval stays in the
/// client's answer to each request; this keeps the requests coming.
const MODE_OPTION: (&str, &str) = ("mode", "default");

/// What one kit is spelled as on the wire: each option the adapter names, the
/// value to set it to, and the order to set them in.
///
/// The delivery half of a kit, and the counterpart to the deciding
/// [`stageman_core::Kit`] does — the same seam [`delivered`] sits on for
/// credentials. Pure, so that every spelling can be asserted without a
/// container, and so that the container test holding these to the pinned
/// adapter has one list to walk.
///
/// The mode comes first and is not the kit's; see [`MODE_OPTION`]. The model
/// comes before the effort because the effort exists only on some models —
/// measured, and the reason it lives inside the model's variant rather than
/// beside it.
pub(crate) fn wired(kit: &Kit) -> Vec<(&'static str, &'static str)> {
    let mut set = vec![MODE_OPTION];
    match kit {
        Kit::Claude { model } => {
            set.push(("model", claude_model(*model)));
            if let Some(effort) = model.effort() {
                set.push(("effort", claude_effort(effort)));
            }
        }
    }
    set
}

/// The alias the adapter knows a model by.
///
/// Aliases rather than dated identifiers, because the adapter refuses the
/// latter outright — measured in `docs/decisions/0048-a-job-runs-on-a-kit.md`.
const fn claude_model(model: ClaudeModel) -> &'static str {
    match model {
        ClaudeModel::Default { .. } => "default",
        ClaudeModel::Sonnet { .. } => "sonnet",
        ClaudeModel::Opus { .. } => "opus",
        ClaudeModel::Haiku => "haiku",
    }
}

/// The name the adapter knows an effort level by.
const fn claude_effort(effort: ClaudeEffort) -> &'static str {
    match effort {
        ClaudeEffort::Default => "default",
        ClaudeEffort::Low => "low",
        ClaudeEffort::Medium => "medium",
        ClaudeEffort::High => "high",
        ClaudeEffort::XHigh => "xhigh",
        ClaudeEffort::Max => "max",
    }
}

/// What a session reports one option to be, as text.
///
/// `None` when the option is not in the list, and when it is of a kind this
/// version of the protocol library cannot read — an option that cannot be read
/// back is one that did not demonstrably take, which is what the caller needs
/// to know.
pub(crate) fn current(options: &[SessionConfigOption], id: &str) -> Option<String> {
    let option = options.iter().find(|option| &*option.id.0 == id)?;
    match &option.kind {
        SessionConfigKind::Select(select) => Some(select.current_value.0.to_string()),
        SessionConfigKind::Boolean(toggle) => Some(toggle.current_value.to_string()),
        _ => None,
    }
}

/// Whether a set took, given what was reported before it and after it.
///
/// By change rather than by spelling. The adapter was measured to report an
/// alias back with a suffix where an account is entitled to a larger context,
/// so requiring the reply to spell the value as it was asked would fail a set
/// that worked. What can be required is that the reading moved — unless what
/// was asked for is what was already reported, in which case nothing had to.
/// Not reported back at all is never having taken: the option just set has to
/// be in the reply.
pub(crate) fn took(asked: &str, before: Option<&str>, after: Option<&str>) -> bool {
    after.is_some_and(|after| before == Some(asked) || Some(after) != before)
}

/// What an adapter said when it refused a setting, as one line.
///
/// The detail travels in the error's data rather than its message — the
/// message is the protocol's generic *internal error*, measured — so the data
/// is read first and the message is the fallback.
pub(crate) fn refused(error: &agent_client_protocol::Error) -> String {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("details"))
        .and_then(|details| details.as_str())
        .map_or_else(|| error.message.clone(), str::to_owned)
}

/// What an agent said in reply to one question.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// What the session reported each setting to be, after being set.
    ///
    /// In the adapter's own spelling and keyed by its option identifier. It was
    /// measured to differ from what was asked — an account entitled to a larger
    /// context has the same alias reported back with a suffix — so this is what
    /// actually ran, where the kit is what was asked for. Recorded on the job
    /// by whoever ran the turn; see `docs/decisions/0048-a-job-runs-on-a-kit.md`.
    pub reported: BTreeMap<String, String>,
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
pub(crate) fn declaration(tools: &Tools) -> McpServer {
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

/// The agent program inside the image, which used to be its entry point.
///
/// Named here because the container no longer runs it as itself: it is run
/// inside a container that is already up, so something has to say what to run.
/// That is this adapter's business and nowhere else's — `docs/conventions.md`
/// §3 keeps the agent's quirks behind this crate's boundary, and which program
/// implements the protocol is the most agent-specific fact there is.
const AGENT_PROGRAM: &str = "claude-agent-acp";

/// The label every container this project starts carries.
///
/// How a container that outlives the process which started it is found again.
/// A name addresses one; this finds them all, including the ones whose job the
/// instance has forgotten. That is what makes `docs/conventions.md` §4's
/// "nothing untracked" checkable rather than merely intended — a sweep able to
/// see only what the snapshot already knew would never find the case it exists
/// for.
const OWNER_LABEL: &str = "stageman.job";

/// The label saying which instance started a container.
///
/// **What makes a sweep safe to let remove anything.** [`OWNER_LABEL`] says a
/// container is this *project's*; this says it is this *instance's*, and the
/// two differ whenever a daemon is shared — a development instance served out
/// of a checkout beside the real one is the ordinary case. Without it, either
/// instance sees the other's containers as work it has lost and, if it removed
/// them, would take the other's jobs with them.
///
/// A container carrying none was made before this existed. That is knowably
/// different from one carrying somebody else's, and the sweep treats it
/// differently — see
/// `docs/decisions/0054-a-container-says-which-instance-started-it.md`.
const INSTANCE_LABEL: &str = "stageman.instance";

/// The label saying which agent a container was made for.
///
/// Read at a foreman's turn boundary and nowhere else: a job's kit cannot
/// change, so a job's container is never asked. See [`made_for`].
const AGENT_LABEL: &str = "stageman.agent";

/// How an agent is spelled in a container's label.
///
/// This crate's spelling and nobody else's — the dashboard has its own for the
/// browser, and the two are separate contracts that happen to agree today.
const fn agent_label(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
    }
}

/// The agent a label names, if it names one this build knows.
/// The agent a label names, if this build knows it.
#[must_use]
pub fn labelled(text: &str) -> Option<Agent> {
    Agent::ALL
        .iter()
        .copied()
        .find(|agent| agent_label(*agent) == text)
}

/// Which agent a container was made for, if it says.
///
/// `None` for a container carrying no such label — one made before the label
/// existed, or by something else — and for a label naming an agent this build
/// does not know. Both mean the same to the one caller: it cannot tell, and
/// has to decide what that means rather than have it decided here.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses — a container that does not
/// exist being the ordinary refusal.
#[mutants::skip]
pub async fn made_for(runtime: &ContainerRuntime, name: &str) -> Result<Option<Agent>, AgentError> {
    let asked = tokio::process::Command::new(runtime.path())
        .args(
            Command::Label {
                name: name.to_owned(),
                label: Label::Agent,
            }
            .arguments(),
        )
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if !asked.status.success() {
        return Err(AgentError::Container {
            status: asked.status.to_string(),
            message: String::from_utf8_lossy(&asked.stderr).trim().to_owned(),
        });
    }
    Ok(labelled(String::from_utf8_lossy(&asked.stdout).trim()))
}

/// Which instance started a container, if it says.
///
/// `None` for a container carrying no such label, which means one made before
/// the label existed rather than one belonging to nobody. Asked of a container
/// at a time rather than read from a listing, because the two runtimes format
/// a listing's labels differently and an inspection is the one shape both
/// take.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses — a container that does not
/// exist being the ordinary refusal.
#[mutants::skip]
pub async fn started_by(
    runtime: &ContainerRuntime,
    name: &str,
) -> Result<Option<InstanceId>, AgentError> {
    let asked = tokio::process::Command::new(runtime.path())
        .args(
            Command::Label {
                name: name.to_owned(),
                label: Label::Instance,
            }
            .arguments(),
        )
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if !asked.status.success() {
        return Err(AgentError::Container {
            status: asked.status.to_string(),
            message: String::from_utf8_lossy(&asked.stderr).trim().to_owned(),
        });
    }
    Ok(minted(&String::from_utf8_lossy(&asked.stdout)))
}

/// The instance a label names, if it names one this build can read.
///
/// Pure, so every shape a runtime prints can be tested without a container. An
/// empty answer is what both print for a label that is not there, and anything
/// that is not an identifier is treated the same way: unreadable rather than
/// somebody else's, because guessing the other way round would let a sweep
/// remove a container it could not actually place.
/// The instance a label names, if it names one this build can read.
#[must_use]
pub fn minted(text: &str) -> Option<InstanceId> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Uuid::parse_str(trimmed).ok().map(InstanceId::from_uuid)
}

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
#[cfg(test)]
fn retained_arguments(
    name: &str,
    image: &Image,
    agent: Agent,
    instance: InstanceId,
    delivering: &[(String, Secret)],
) -> Vec<String> {
    Command::Create {
        name: name.to_owned(),
        image: image.as_argument().to_owned(),
        agent,
        instance,
        variables: delivering.iter().map(|(named, _)| named.clone()).collect(),
    }
    .arguments()
}

/// What makes sure a container is up, without attaching to it.
///
/// Nothing attaches to the container itself any more — the agent is run inside
/// it — so this only has to leave it running. Starting one that is already
/// running is success on both runtimes, which is what lets beginning and
/// resuming share it without asking first: a container held open because its
/// tunnel answers is already up, and one that was stopped is not.
#[cfg(test)]
fn holding_arguments(name: &str) -> Vec<String> {
    Command::Start {
        name: name.to_owned(),
    }
    .arguments()
}

/// What runs the agent inside a container that is already up.
///
/// The pipes belong to this process rather than to the container, which is the
/// whole change: the end of a turn closes them and ends the agent, and the
/// container carries on. Nothing is forwarded with `--env`, because the
/// variables were named when the container was created and everything run
/// inside it inherits them.
#[cfg(test)]
fn agent_arguments(name: &str) -> Vec<String> {
    Command::Exec {
        name: name.to_owned(),
    }
    .arguments()
}

/// The variable the checkout step reads the repository from.
///
/// Not a secret, and not forwarded from this process the way credentials are:
/// it is set on the one command that needs it, so a URL is on that command
/// line and nothing else is.
const REPOSITORY_VARIABLE: &str = "STAGEMAN_REPOSITORY";

/// What checks the repository out into a container's workspace, before its
/// agent is run for the first time — see
/// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
///
/// Pure, so what runs in the container can be asserted without one. The
/// repository travels in a variable rather than in the script, so it needs no
/// quoting and the script is the same text for every job.
#[cfg(test)]
fn checkout_arguments(name: &str, repository: &str, platform: Option<Platform>) -> Vec<String> {
    Command::Checkout {
        name: name.to_owned(),
        repository: repository.to_owned(),
        platform,
    }
    .arguments()
}

/// The checkout itself, as the shell inside the container runs it.
///
/// Two shapes, decided by whether the job holds a credential for the
/// repository's platform. With one, the platform's own tool makes the clone
/// with the credential already in the environment, configures git to push
/// with it — the tool leaves no helper behind on its own, measured — and sets
/// the commit identity to the account the credential belongs to, spelled the
/// way the platform spells a private address. Without one, plain git clones
/// what is public, and there is no account to be.
///
/// Asserted as literal text, because this is the one place the project types
/// commands into a container on a job's behalf.
const fn checkout_script(platform: Option<Platform>) -> &'static str {
    match platform {
        Some(Platform::GitHub) => {
            "set -eu\n\
             gh repo clone \"$STAGEMAN_REPOSITORY\" .\n\
             gh auth setup-git --hostname github.com\n\
             account=\"$(gh api user --jq '\"\\(.id)+\\(.login)\"')\"\n\
             git config --global user.name \"${account#*+}\"\n\
             git config --global user.email \"${account}@users.noreply.github.com\"\n"
        }
        None => "set -eu\ngit clone \"$STAGEMAN_REPOSITORY\" .\n",
    }
}

/// Runs the checkout in a container that is up.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or if the checkout does — a credential
/// that does not open the repository, a URL that names nothing, or a
/// workspace that already holds something — with what the tool printed,
/// before any model turn is spent.
#[mutants::skip]
#[cfg(test)]
async fn check_out(
    runtime: &ContainerRuntime,
    name: &str,
    repository: &str,
    platform: Option<Platform>,
) -> Result<(), AgentError> {
    let done = tokio::process::Command::new(runtime.path())
        .args(checkout_arguments(name, repository, platform))
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;
    if done.status.success() {
        return Ok(());
    }
    Err(AgentError::Checkout {
        repository: repository.to_owned(),
        message: String::from_utf8_lossy(&done.stderr).trim().to_owned(),
    })
}

/// Leaves a container running, whether or not it already was.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses — a container that does not
/// exist being the ordinary refusal.
#[mutants::skip]
#[cfg(test)]
async fn hold(runtime: &ContainerRuntime, name: &str) -> Result<(), AgentError> {
    let started = tokio::process::Command::new(runtime.path())
        .args(holding_arguments(name))
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| AgentError::Runtime {
            path: runtime.path().to_owned(),
            source,
        })?;

    if started.status.success() {
        return Ok(());
    }
    Err(AgentError::Container {
        status: started.status.to_string(),
        message: String::from_utf8_lossy(&started.stderr).trim().to_owned(),
    })
}

/// Stops a container, leaving it and everything in it where it is.
///
/// The other half of `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`:
/// a container is no longer stopped by the turn inside it ending, so something
/// has to stop it, and that something is a caller who has decided nothing is
/// being shown. Stopping one already stopped is success on both runtimes, so a
/// caller need not ask first.
///
/// Not [`discard`], which removes it: what is inside is a job's work and the
/// session it would be resumed from.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses — a container that does not
/// exist being the ordinary refusal.
pub async fn halt(runtime: &ContainerRuntime, name: &str) -> Result<(), AgentError> {
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

/// One question the runtime is asked, as a value.
///
/// Rendered to the arguments the runtime is given and read back from them,
/// and the two directions are tested against each other, so that whatever
/// decides to ask — the instance, since
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
/// — and whatever answers in a simulation cannot drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Whether the runtime answers at all, which is also how a candidate for
    /// one is found to be present.
    Version,
    /// The names of every container this project started, or only the
    /// running ones.
    Containers {
        /// Only those up right now.
        running_only: bool,
    },
    /// What one label on a container says.
    Label {
        /// The container.
        name: String,
        /// Which label.
        label: Label,
    },
    /// Stop a container, leaving it where it is.
    Halt {
        /// The container.
        name: String,
    },
    /// Remove a container and everything inside it.
    ///
    /// Forced, because a container that is still running is one this has
    /// decided is finished with, and stopping it first would be two commands
    /// with a window between them.
    Discard {
        /// The container.
        name: String,
    },
    /// Where a container's tunnel is published on the host, if anywhere.
    Port {
        /// The container.
        name: String,
    },
    /// Every image this project has built and still holds.
    Images,
    /// Remove one image.
    RemoveImage {
        /// Its name and tag.
        image: String,
    },
    /// Whether an image is already here, which is how a build is skipped.
    Present {
        /// Its name and tag.
        image: String,
    },
    /// Build an image under the name its recipe hashes to, from a recipe on
    /// standard input and no other context: nothing this project writes
    /// goes in an image, per
    /// `docs/decisions/0034-tools-are-served-not-shipped.md`, so there is
    /// nothing a context could carry. Quiet, because the name is already
    /// known; what a build has to say still arrives on standard error.
    Build {
        /// Its name and tag.
        image: String,
    },
    /// Create a container meant to outlive this process, from an image,
    /// with its tunnel published and the variables named forwarded from
    /// the environment the runtime is given.
    ///
    /// Created rather than run, so there is a moment between existing and
    /// starting in which a thread can be put in place; and never `--rm`,
    /// so that hard-killing the daemon leaves the container and its session
    /// intact — see
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`.
    Create {
        /// The container, named before it exists.
        name: String,
        /// The image it is made from.
        image: String,
        /// Which agent it is made for, on its label.
        agent: Agent,
        /// Which instance made it, on its label.
        instance: InstanceId,
        /// The variables forwarded into it, by name. Never valued here:
        /// `--env NAME` tells the runtime to take the value from the
        /// environment the command is given, so a secret never appears in
        /// the process table.
        variables: Vec<String>,
    },
    /// Start a container, or leave one already running as it is: success
    /// on both runtimes either way, which is what lets beginning and
    /// resuming share it.
    Start {
        /// The container.
        name: String,
    },
    /// Check the repository out into a container's workspace, with the
    /// platform's own tool where a credential for one is held and plain git
    /// otherwise — see
    /// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    Checkout {
        /// The container.
        name: String,
        /// What is checked out, travelling in a variable rather than the
        /// script so that it needs no quoting.
        repository: String,
        /// Whose tool makes the clone, if any's.
        platform: Option<Platform>,
    },
    /// Run the agent inside a container that is up, with its standard
    /// streams piped to this process: the pipes are what a turn holds, and
    /// closing them ends the agent while the container carries on.
    Exec {
        /// The container.
        name: String,
    },
}

/// The labels a container of this project's carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Label {
    /// Which instance started it.
    Instance,
    /// Which agent it was made for.
    Agent,
}

impl Label {
    const fn key(self) -> &'static str {
        match self {
            Self::Instance => INSTANCE_LABEL,
            Self::Agent => AGENT_LABEL,
        }
    }

    fn of(key: &str) -> Option<Self> {
        match key {
            INSTANCE_LABEL => Some(Self::Instance),
            AGENT_LABEL => Some(Self::Agent),
            _ => None,
        }
    }
}

impl Command {
    /// The arguments that ask it, which both runtimes take.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        match self {
            Self::Version => vec!["version".to_owned()],
            Self::Containers { running_only } => {
                let mut arguments = vec!["ps".to_owned()];
                if !running_only {
                    arguments.push("--all".to_owned());
                }
                arguments.extend([
                    "--filter".to_owned(),
                    format!("label={OWNER_LABEL}"),
                    "--format".to_owned(),
                    "{{.Names}}".to_owned(),
                ]);
                arguments
            }
            Self::Label { name, label } => vec![
                "inspect".to_owned(),
                "--format".to_owned(),
                format!("{{{{index .Config.Labels \"{}\"}}}}", label.key()),
                name.clone(),
            ],
            Self::Halt { name } => vec!["stop".to_owned(), name.clone()],
            Self::Discard { name } => {
                vec!["rm".to_owned(), "--force".to_owned(), name.clone()]
            }
            Self::Port { name } => vec!["port".to_owned(), name.clone(), TUNNEL_PORT.to_string()],
            Self::Images => ours_arguments(),
            Self::RemoveImage { image } => vec!["rmi".to_owned(), image.clone()],
            Self::Present { image } => vec![
                "image".to_owned(),
                "inspect".to_owned(),
                "--format".to_owned(),
                "{{.Id}}".to_owned(),
                image.clone(),
            ],
            Self::Build { image } => vec![
                "build".to_owned(),
                "--quiet".to_owned(),
                "--tag".to_owned(),
                image.clone(),
                "-".to_owned(),
            ],
            Self::Create {
                name,
                image,
                agent,
                instance,
                variables,
            } => {
                let mut arguments = vec![
                    "create".to_owned(),
                    "--interactive".to_owned(),
                    // The runtime's own init as process one, which does two
                    // things this needs. It reaps what an agent orphans — and
                    // since
                    // `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
                    // a container outlives the agent that filled it, so there
                    // is now time to accumulate them. And it passes on the
                    // signal that stops a container, which process one
                    // otherwise ignores, so stopping takes an instant rather
                    // than a timeout.
                    "--init".to_owned(),
                    // So that one hostname reaches this instance whichever
                    // runtime is in use. Measured on both: Docker and Podman
                    // each honour it, and without it a container on Linux can
                    // reach the host by no name at all.
                    "--add-host".to_owned(),
                    "host.docker.internal:host-gateway".to_owned(),
                    // The tunnel, published here because there is nowhere
                    // later: the mapping is fixed when the container is
                    // created and no runtime can add one afterwards.
                    //
                    // Loopback on the host, and an empty host port so the
                    // runtime picks a free one atomically — choosing one here
                    // by binding and releasing it is a race, and this project
                    // has already lost a port that way. What it picked is
                    // asked for rather than recorded, because Docker assigns a
                    // new one on every start and Podman does not.
                    "--publish".to_owned(),
                    format!("127.0.0.1::{TUNNEL_PORT}"),
                    "--name".to_owned(),
                    name.clone(),
                    "--label".to_owned(),
                    format!("{OWNER_LABEL}={name}"),
                    // Which agent this container was made for, so that a turn
                    // boundary can ask. A foreman's container is long-lived
                    // and its project's kit can change underneath it; the
                    // same agent on other settings keeps the container,
                    // because settings are settled again every turn, and a
                    // different agent is a different image — see
                    // `docs/decisions/0048-a-job-runs-on-a-kit.md`. The
                    // image's identity cannot answer this, since nothing is
                    // tagged.
                    "--label".to_owned(),
                    format!("{AGENT_LABEL}={}", agent_label(*agent)),
                    // Which instance this belongs to, so that a sweep on a
                    // shared daemon can tell its own abandoned work from
                    // somebody else's containers.
                    "--label".to_owned(),
                    format!("{INSTANCE_LABEL}={instance}"),
                ];
                for variable in variables {
                    arguments.push("--env".to_owned());
                    arguments.push(variable.clone());
                }
                // Deliberately no `--network none` here, unlike the
                // handshake: reaching a model needs the network, and so does
                // cloning. Which hosts it *ought* to reach is the egress
                // allowlist still open in `docs/open-questions.md`.
                arguments.push(image.clone());
                arguments
            }
            Self::Start { name } => vec!["start".to_owned(), name.clone()],
            Self::Checkout {
                name,
                repository,
                platform,
            } => vec![
                "exec".to_owned(),
                "--env".to_owned(),
                format!("{REPOSITORY_VARIABLE}={repository}"),
                name.clone(),
                "sh".to_owned(),
                "-c".to_owned(),
                checkout_script(*platform).to_owned(),
            ],
            Self::Exec { name } => vec![
                "exec".to_owned(),
                "--interactive".to_owned(),
                name.clone(),
                AGENT_PROGRAM.to_owned(),
            ],
        }
    }

    /// The question these arguments ask, if they ask one this build renders.
    #[must_use]
    pub fn parse(arguments: &[String]) -> Option<Self> {
        let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["version"] => Some(Self::Version),
            ["ps", "--all", "--filter", filter, "--format", "{{.Names}}"]
                if *filter == format!("label={OWNER_LABEL}") =>
            {
                Some(Self::Containers {
                    running_only: false,
                })
            }
            ["ps", "--filter", filter, "--format", "{{.Names}}"]
                if *filter == format!("label={OWNER_LABEL}") =>
            {
                Some(Self::Containers { running_only: true })
            }
            ["inspect", "--format", template, name] => {
                let key = template
                    .strip_prefix("{{index .Config.Labels \"")?
                    .strip_suffix("\"}}")?;
                Some(Self::Label {
                    name: (*name).to_owned(),
                    label: Label::of(key)?,
                })
            }
            ["stop", name] => Some(Self::Halt {
                name: (*name).to_owned(),
            }),
            ["rm", "--force", name] => Some(Self::Discard {
                name: (*name).to_owned(),
            }),
            ["port", name, published] if *published == TUNNEL_PORT.to_string() => {
                Some(Self::Port {
                    name: (*name).to_owned(),
                })
            }
            [
                "images",
                "--filter",
                filter,
                "--format",
                "{{.Repository}}:{{.Tag}}",
            ] if *filter == format!("reference={REPOSITORY}") => Some(Self::Images),
            ["rmi", image] => Some(Self::RemoveImage {
                image: (*image).to_owned(),
            }),
            _ => Self::parse_turn(&words),
        }
    }

    /// The question these arguments ask, among the ones a turn asks.
    ///
    /// Split from [`Command::parse`] by the line budget and nothing else:
    /// a turn is six commands, and every one has an argument list worth
    /// reading back exactly.
    fn parse_turn(words: &[&str]) -> Option<Self> {
        match words {
            ["image", "inspect", "--format", "{{.Id}}", image] => Some(Self::Present {
                image: (*image).to_owned(),
            }),
            ["build", "--quiet", "--tag", image, "-"] => Some(Self::Build {
                image: (*image).to_owned(),
            }),
            [
                "create",
                "--interactive",
                "--init",
                "--add-host",
                "host.docker.internal:host-gateway",
                "--publish",
                published,
                "--name",
                name,
                "--label",
                owner,
                "--label",
                agent,
                "--label",
                instance,
                rest @ ..,
            ] if *published == format!("127.0.0.1::{TUNNEL_PORT}")
                && *owner == format!("{OWNER_LABEL}={name}") =>
            {
                let agent = labelled(agent.strip_prefix(&format!("{AGENT_LABEL}="))?)?;
                let instance = minted(instance.strip_prefix(&format!("{INSTANCE_LABEL}="))?)?;
                let (image, forwarded) = rest.split_last()?;
                let mut variables = Vec::new();
                for pair in forwarded.chunks(2) {
                    let ["--env", variable] = pair else {
                        return None;
                    };
                    variables.push((*variable).to_owned());
                }
                Some(Self::Create {
                    name: (*name).to_owned(),
                    image: (*image).to_owned(),
                    agent,
                    instance,
                    variables,
                })
            }
            ["start", name] => Some(Self::Start {
                name: (*name).to_owned(),
            }),
            ["exec", "--env", repository, name, "sh", "-c", script] => {
                let repository = repository.strip_prefix(&format!("{REPOSITORY_VARIABLE}="))?;
                let platform = if *script == checkout_script(Some(Platform::GitHub)) {
                    Some(Platform::GitHub)
                } else if *script == checkout_script(None) {
                    None
                } else {
                    return None;
                };
                Some(Self::Checkout {
                    name: (*name).to_owned(),
                    repository: repository.to_owned(),
                    platform,
                })
            }
            ["exec", "--interactive", name, program] if *program == AGENT_PROGRAM => {
                Some(Self::Exec {
                    name: (*name).to_owned(),
                })
            }
            _ => None,
        }
    }
}

/// Every container this project has ever started that the runtime still
/// holds, by name.
///
/// The names a container carries are the only thing a listing is asked for:
/// the two runtimes format a listing's labels differently, and an inspection
/// is the one shape both take, so labels are asked per container.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses the query.
#[mutants::skip]
pub async fn abandoned(runtime: &ContainerRuntime) -> Result<Vec<String>, AgentError> {
    listed(
        runtime,
        &Command::Containers {
            running_only: false,
        },
    )
    .await
}

/// Every container this project started that is up right now, by name.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses the query.
#[mutants::skip]
pub async fn running(runtime: &ContainerRuntime) -> Result<Vec<String>, AgentError> {
    listed(runtime, &Command::Containers { running_only: true }).await
}

#[mutants::skip]
async fn listed(runtime: &ContainerRuntime, query: &Command) -> Result<Vec<String>, AgentError> {
    let listed = tokio::process::Command::new(runtime.path())
        .args(query.arguments())
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
    Ok(names(&String::from_utf8_lossy(&listed.stdout)))
}

/// The names in a listing, one per line, however the runtime spaced them.
#[must_use]
pub fn names(reported: &str) -> Vec<String> {
    reported
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
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
/// Public because the instance reads it: since
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
/// the command is rendered there and its output parsed there, and this is
/// the parser.
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
#[must_use]
pub fn published(reported: &str) -> Option<u16> {
    reported
        .lines()
        .filter_map(|line| line.trim().rsplit(':').next())
        .find_map(|port| port.trim().parse().ok())
}

/// Removes a container and everything inside it.
///
/// Skipped by mutation testing, like everything else here that drives the
/// runtime: what it decides is reached only by the container tests, and an
/// ignored test kills no mutant. The instance renders its own removal since
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`;
/// this is what the tests tidy up with.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses. A container that is not
/// there is not a failure: what was asked for is that it be gone.
#[mutants::skip]
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

    /// Every kit is spelled with the mode first and the model before the
    /// effort, and a model with no effort sends none.
    ///
    /// The order is load-bearing: the effort exists only on some models, so a
    /// spelling that set it first could set it on a model about to be changed
    /// to one that refuses it.
    #[test]
    fn a_kit_is_spelled_mode_first_and_effort_only_where_there_is_one() {
        assert_eq!(
            wired(&Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            }),
            vec![("mode", "default"), ("model", "opus"), ("effort", "xhigh")],
        );
        assert_eq!(
            wired(&Kit::Claude {
                model: ClaudeModel::Haiku
            }),
            vec![("mode", "default"), ("model", "haiku")],
        );
        assert_eq!(
            wired(&Kit::defaults(Agent::Claude)),
            vec![
                ("mode", "default"),
                ("model", "default"),
                ("effort", "default")
            ],
            "the agent's own default is a value it spells, and is sent as one",
        );
    }

    /// A container's label names its agent both ways, and an unknown label
    /// names nobody.
    /// Every question renders to arguments and reads back as itself, and
    /// arguments that ask nothing this build renders read as nothing.
    #[test]
    fn every_command_reads_back_from_its_own_arguments() {
        let every = [
            Command::Version,
            Command::Containers {
                running_only: false,
            },
            Command::Containers { running_only: true },
            Command::Label {
                name: "stageman-job-1".to_owned(),
                label: Label::Instance,
            },
            Command::Label {
                name: "stageman-foreman-2".to_owned(),
                label: Label::Agent,
            },
            Command::Halt {
                name: "stageman-job-1".to_owned(),
            },
            Command::Discard {
                name: "stageman-job-1".to_owned(),
            },
            Command::Port {
                name: "stageman-job-1".to_owned(),
            },
            Command::Images,
            Command::RemoveImage {
                image: "stageman:0123".to_owned(),
            },
        ];
        for command in every {
            assert_eq!(
                Command::parse(&command.arguments()),
                Some(command.clone()),
                "{command:?}"
            );
        }
        assert_eq!(Command::parse(&["rm".to_owned(), "-f".to_owned()]), None);
        assert_eq!(
            Command::parse(&[
                "ps".to_owned(),
                "--all".to_owned(),
                "--filter".to_owned(),
                "label=other".to_owned(),
                "--format".to_owned(),
                "{{.Names}}".to_owned()
            ]),
            None,
            "another project's containers are not this one's question"
        );
        assert_eq!(
            Command::parse(&[
                "ps".to_owned(),
                "--filter".to_owned(),
                "label=other".to_owned(),
                "--format".to_owned(),
                "{{.Names}}".to_owned()
            ]),
            None,
            "and no more so when only the running ones are asked for"
        );
        assert_eq!(
            Command::parse(&[
                "inspect".to_owned(),
                "--format".to_owned(),
                "{{index .Config.Labels \"other\"}}".to_owned(),
                "x".to_owned()
            ]),
            None
        );
        assert_eq!(
            Command::parse(&[
                "port".to_owned(),
                "stageman-job-1".to_owned(),
                "80".to_owned()
            ]),
            None,
            "another port is another question"
        );
        assert_eq!(
            Command::parse(&[
                "images".to_owned(),
                "--filter".to_owned(),
                "reference=other".to_owned(),
                "--format".to_owned(),
                "{{.Repository}}:{{.Tag}}".to_owned()
            ]),
            None,
            "another project's images are not this one's question"
        );
        assert_eq!(names(" a \n\nb\n"), vec!["a".to_owned(), "b".to_owned()]);
    }

    /// Every command a turn runs renders to arguments and reads back as
    /// itself, with and without what varies: the variables forwarded, and
    /// whose tool makes the checkout.
    #[test]
    fn every_command_a_turn_runs_reads_back_from_its_own_arguments() {
        let every = [
            Command::Present {
                image: "stageman:0123".to_owned(),
            },
            Command::Build {
                image: "stageman:0123".to_owned(),
            },
            Command::Create {
                name: "stageman-job-1".to_owned(),
                image: "stageman:0123".to_owned(),
                agent: Agent::Claude,
                instance: an_instance(),
                variables: vec!["ANTHROPIC_API_KEY".to_owned(), "GH_TOKEN".to_owned()],
            },
            Command::Create {
                name: "stageman-foreman-2".to_owned(),
                image: "stageman:0123".to_owned(),
                agent: Agent::Claude,
                instance: an_instance(),
                variables: Vec::new(),
            },
            Command::Start {
                name: "stageman-job-1".to_owned(),
            },
            Command::Checkout {
                name: "stageman-job-1".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                platform: Some(Platform::GitHub),
            },
            Command::Checkout {
                name: "stageman-job-1".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                platform: None,
            },
            Command::Exec {
                name: "stageman-job-1".to_owned(),
            },
        ];
        for command in every {
            assert_eq!(
                Command::parse(&command.arguments()),
                Some(command.clone()),
                "{command:?}"
            );
        }
    }

    /// The turn's commands, asked another way, are not this build's
    /// questions: a script that is not the checkout, a program that is not
    /// the agent, a label naming an agent this build does not know, a
    /// variable not introduced by `--env`.
    #[test]
    fn a_turns_commands_asked_another_way_are_not_this_builds_questions() {
        let mut other_script = Command::Checkout {
            name: "stageman-job-1".to_owned(),
            repository: "https://example.invalid/repo".to_owned(),
            platform: None,
        }
        .arguments();
        other_script.pop();
        other_script.push("rm -rf /".to_owned());
        assert_eq!(Command::parse(&other_script), None);
        assert_eq!(
            Command::parse(&[
                "exec".to_owned(),
                "--interactive".to_owned(),
                "stageman-job-1".to_owned(),
                "sh".to_owned()
            ]),
            None
        );
        let mut other_agent = Command::Create {
            name: "stageman-job-1".to_owned(),
            image: "stageman:0123".to_owned(),
            agent: Agent::Claude,
            instance: an_instance(),
            variables: Vec::new(),
        }
        .arguments();
        let labelled_agent = other_agent
            .iter()
            .position(|argument| argument == "stageman.agent=claude")
            .expect("the agent's label");
        other_agent[labelled_agent] = "stageman.agent=gpt".to_owned();
        assert_eq!(Command::parse(&other_agent), None);
        let mut odd_variables = Command::Create {
            name: "stageman-job-1".to_owned(),
            image: "stageman:0123".to_owned(),
            agent: Agent::Claude,
            instance: an_instance(),
            variables: vec!["ONE".to_owned()],
        }
        .arguments();
        odd_variables.insert(odd_variables.len() - 1, "TWO".to_owned());
        assert_eq!(
            Command::parse(&odd_variables),
            None,
            "a variable not introduced by --env is not a variable"
        );

        // The two halves of the guard, each wrong on its own: a tunnel
        // published somewhere other than loopback, and an owner label naming
        // another container than the one being made.
        let creating = || {
            Command::Create {
                name: "stageman-job-1".to_owned(),
                image: "stageman:0123".to_owned(),
                agent: Agent::Claude,
                instance: an_instance(),
                variables: Vec::new(),
            }
            .arguments()
        };
        let mut elsewhere = creating();
        let published = elsewhere
            .iter()
            .position(|argument| argument.starts_with("127.0.0.1::"))
            .expect("the tunnel's mapping");
        elsewhere[published] = format!("0.0.0.0::{TUNNEL_PORT}");
        assert_eq!(
            Command::parse(&elsewhere),
            None,
            "a tunnel published beyond loopback is not this build's question"
        );
        let mut disowned = creating();
        let owner = disowned
            .iter()
            .position(|argument| argument == "stageman.job=stageman-job-1")
            .expect("the owner label");
        disowned[owner] = "stageman.job=stageman-job-2".to_owned();
        assert_eq!(
            Command::parse(&disowned),
            None,
            "an owner label naming another container is not this build's question"
        );
    }

    #[test]
    fn an_agents_label_reads_back_as_that_agent() {
        for agent in Agent::ALL {
            assert_eq!(labelled(agent_label(*agent)), Some(*agent));
        }
        // The literal, because containers already made carry it: a label
        // spelled differently by a later build would read every existing
        // foreman's container as made for nobody.
        assert_eq!(agent_label(Agent::Claude), "claude");
        assert_eq!(labelled(""), None, "no label is no agent");
        assert_eq!(labelled("gpt"), None, "a label this build does not know");
        assert_eq!(
            Command::Label {
                name: "stageman-foreman-x".to_owned(),
                label: Label::Agent,
            }
            .arguments(),
            vec![
                "inspect",
                "--format",
                "{{index .Config.Labels \"stageman.agent\"}}",
                "stageman-foreman-x",
            ],
        );
    }

    /// No two levels, and no two models, share a spelling.
    ///
    /// A shared one would set the wrong thing and read back as having taken.
    #[test]
    fn every_model_and_every_effort_has_a_spelling_of_its_own() {
        let efforts: std::collections::BTreeSet<&str> = ClaudeEffort::ALL
            .iter()
            .map(|effort| claude_effort(*effort))
            .collect();
        assert_eq!(efforts.len(), ClaudeEffort::ALL.len());

        let effort = ClaudeEffort::Default;
        let models: std::collections::BTreeSet<&str> = [
            ClaudeModel::Default { effort },
            ClaudeModel::Sonnet { effort },
            ClaudeModel::Opus { effort },
            ClaudeModel::Haiku,
        ]
        .into_iter()
        .map(claude_model)
        .collect();
        assert_eq!(models.len(), 4);
    }

    /// Whether a set took is decided by the reading moving, not by spelling.
    ///
    /// The first case is the measured one that rules out exact comparison: an
    /// account entitled to a larger context has `opus` reported back as
    /// `opus[1m]`, and that set worked.
    #[test]
    fn a_setting_took_when_the_reading_moved_or_was_already_what_was_asked() {
        assert!(
            took("opus", Some("default"), Some("opus[1m]")),
            "a different spelling is still a change"
        );
        assert!(
            took("default", Some("default"), Some("default")),
            "already what was asked, so nothing had to move"
        );
        assert!(
            took("high", None, Some("high")),
            "an option that was not advertised before it was set"
        );
        assert!(
            !took("opus", Some("default"), Some("default")),
            "accepted and unchanged is the failure this exists to catch"
        );
        assert!(
            !took("opus", Some("default"), None),
            "not reported back at all is not having taken"
        );
    }

    /// A refusal's detail is in the error's data, and the message is only the
    /// fallback.
    ///
    /// Measured: the message is the protocol's generic *internal error*, and
    /// the sentence naming the option and the value travels as data.
    #[test]
    fn a_refusal_is_read_from_the_errors_data_before_its_message() {
        let detailed = agent_client_protocol::Error::new(-32603, "Internal error")
            .data(serde_json::json!({"details": "Invalid value for config option model: nope"}));
        assert_eq!(
            refused(&detailed),
            "Invalid value for config option model: nope"
        );

        let bare = agent_client_protocol::Error::new(-32603, "Internal error");
        assert_eq!(refused(&bare), "Internal error");
    }

    /// The current value of an option, read off the kinds this crate knows.
    #[test]
    fn an_options_current_value_is_read_as_text() {
        let advertised: Vec<SessionConfigOption> = serde_json::from_value(serde_json::json!([
            {"id": "model", "name": "Model", "type": "select", "currentValue": "opus[1m]",
             "options": [{"value": "opus[1m]", "name": "Opus"}]},
            {"id": "fast", "name": "Fast", "type": "boolean", "currentValue": false}
        ]))
        .expect("the protocol's own shape parses");

        assert_eq!(current(&advertised, "model").as_deref(), Some("opus[1m]"));
        assert_eq!(current(&advertised, "fast").as_deref(), Some("false"));
        assert_eq!(current(&advertised, "effort"), None, "not advertised");
    }

    /// Every kit the domain can spell for Claude.
    fn every_claude_kit() -> Vec<Kit> {
        let mut kits = vec![Kit::Claude {
            model: ClaudeModel::Haiku,
        }];
        for effort in ClaudeEffort::ALL.iter().copied() {
            for model in [
                ClaudeModel::Default { effort },
                ClaudeModel::Sonnet { effort },
                ClaudeModel::Opus { effort },
            ] {
                kits.push(Kit::Claude { model });
            }
        }
        kits
    }

    /// Every spelling this crate sends is one the pinned adapter accepts, and
    /// the one option it does not offer is the one the domain cannot ask for.
    ///
    /// The test `docs/decisions/0048-a-job-runs-on-a-kit.md` promises: a pin
    /// bump that removes or renames a value fails here rather than in a job
    /// at three in the morning. Every kit is settled on one session, said
    /// and read exactly as a conversation says and reads it, so what is
    /// checked is what runs. It needs no credential and no network — a
    /// session opens without either, measured — which is why it sits with
    /// the tests that only need a runtime rather than with the ones that
    /// cost a credential.
    ///
    /// The last assertion is about the domain's shape rather than a spelling:
    /// Haiku has no effort in the domain because the adapter offers none, and
    /// if the adapter started offering one this is what would say so.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn every_spelling_is_one_the_pinned_adapter_accepts() {
        use agent_client_protocol::schema::v1::{
            NewSessionResponse, SetSessionConfigOptionResponse,
        };

        let runtime = located_runtime();
        let image = build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");
        let mut adapter = Pump::start(&runtime, &alone(&image));

        adapter.say(&Said::Initialize { id: 1 }).await;
        adapter.answer(1).await.expect("the handshake completes");
        adapter.say(&Said::NewSession { id: 2, tools: None }).await;
        let made: NewSessionResponse = serde_json::from_value(
            adapter
                .answer(2)
                .await
                .expect("a fresh session always opens"),
        )
        .expect("a session");
        let session = made.session_id.0.to_string();
        let mut advertised = made.config_options.unwrap_or_default();

        let mut id = 3;
        for kit in every_claude_kit() {
            for (option, value) in wired(&kit) {
                id += 1;
                let before = current(&advertised, option);
                adapter
                    .say(&Said::SetOption {
                        id,
                        session: session.clone(),
                        option: option.to_owned(),
                        value: value.to_owned(),
                    })
                    .await;
                let result = adapter.answer(id).await.unwrap_or_else(|why| {
                    panic!(
                        "{kit:?} is not accepted by the pinned adapter: {option} = {value}: {why}"
                    )
                });
                let reply: SetSessionConfigOptionResponse =
                    serde_json::from_value(result).expect("a setting's reply");
                advertised = reply.config_options;
                let after = current(&advertised, option);
                assert!(
                    took(value, before.as_deref(), after.as_deref()),
                    "{kit:?}: {option} = {value} was accepted and reported unchanged"
                );
            }
        }

        // Back on haiku, which has no effort in the domain because the
        // adapter offers none there: asked for one anyway, it refuses.
        adapter
            .say(&Said::SetOption {
                id: id + 1,
                session: session.clone(),
                option: "model".to_owned(),
                value: claude_model(ClaudeModel::Haiku).to_owned(),
            })
            .await;
        adapter.answer(id + 1).await.expect("haiku settles");
        adapter
            .say(&Said::SetOption {
                id: id + 2,
                session,
                option: "effort".to_owned(),
                value: "default".to_owned(),
            })
            .await;
        assert!(
            adapter.answer(id + 2).await.is_err(),
            "the adapter offers no effort on haiku, which is why the domain cannot ask for one",
        );
    }

    /// The arguments that start a container just long enough to be spoken
    /// to, for the tests here: no network and no workspace, since nothing
    /// before the first prompt needs either, and nothing left behind.
    ///
    /// Named, because the image holds a container open by default since
    /// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`,
    /// so a run that names no command starts a container that sleeps and
    /// never speaks.
    fn alone(image: &Image) -> Vec<String> {
        vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--interactive".to_owned(),
            "--network".to_owned(),
            "none".to_owned(),
            image.as_argument().to_owned(),
            AGENT_PROGRAM.to_owned(),
        ]
    }

    /// An agent process the tests speak to line by line, the way the world
    /// does: what is said goes down its standard input, what it says comes
    /// back up its standard output, and its standard error is drained so a
    /// talkative agent cannot block.
    struct Pump {
        child: tokio::process::Child,
        input: tokio::process::ChildStdin,
        output: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
        complaints: Option<tokio::task::JoinHandle<Vec<u8>>>,
    }

    impl Pump {
        /// Starts the runtime with `arguments` and pipes its three streams.
        fn start(runtime: &ContainerRuntime, arguments: &[String]) -> Self {
            use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _};
            let mut child = tokio::process::Command::new(runtime.path())
                .args(arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .expect("the runtime starts");
            let input = child.stdin.take().expect("piped");
            let output = tokio::io::BufReader::new(child.stdout.take().expect("piped")).lines();
            let mut stderr = child.stderr.take().expect("piped");
            let complaints = tokio::spawn(async move {
                let mut kept = Vec::new();
                drop(stderr.read_to_end(&mut kept).await);
                kept
            });
            Self {
                child,
                input,
                output,
                complaints: Some(complaints),
            }
        }

        /// Writes one line down the agent's standard input.
        async fn write(&mut self, line: &str) {
            let mut line = line.to_owned();
            line.push('\n');
            self.input
                .write_all(line.as_bytes())
                .await
                .expect("the agent is still reading");
        }

        /// Says one thing.
        async fn say(&mut self, said: &Said) {
            self.write(&said.line()).await;
        }

        /// What the agent printed, once it has stopped, with its status.
        async fn stopped(&mut self) -> (Option<i32>, String) {
            let status = self.child.wait().await.expect("it can be waited on");
            let complaints = match self.complaints.take() {
                Some(draining) => draining.await.unwrap_or_default(),
                None => Vec::new(),
            };
            (
                status.code(),
                String::from_utf8_lossy(&complaints).into_owned(),
            )
        }

        /// The agent's answer to one request, skipping whatever it says
        /// unasked on the way — a session, once made, is announced with
        /// what it can be told — and failing on a request of the agent's
        /// own, which nothing here answers.
        async fn answer(&mut self, id: i64) -> Result<serde_json::Value, AgentError> {
            loop {
                match self.hear().await {
                    Heard::Answered { id: whose, result } if whose == id.into() => {
                        return Ok(result);
                    }
                    Heard::Refused { id: whose, error } if whose == id.into() => {
                        return Err(AgentError::Protocol(error));
                    }
                    Heard::Notified { .. } | Heard::Answered { .. } | Heard::Refused { .. } => {}
                    Heard::Asked { method, .. } => panic!("the agent asked for {method}"),
                }
            }
        }

        /// Hears the next thing the agent says that is the protocol, or
        /// fails with what it printed if it stopped speaking first.
        async fn hear(&mut self) -> Heard {
            loop {
                let Ok(Some(line)) = self.output.next_line().await else {
                    let (status, complaints) = self.stopped().await;
                    panic!("the agent stopped speaking: {status:?}{complaints}");
                };
                if let Some(heard) = Heard::parse(&line) {
                    return heard;
                }
            }
        }

        /// Drives a whole conversation to its end, as the instance does.
        async fn talk(
            &mut self,
            mut conversation: Conversation,
            first: Vec<String>,
        ) -> Result<Answer, AgentError> {
            for line in first {
                self.write(&line).await;
            }
            loop {
                let Ok(Some(line)) = self.output.next_line().await else {
                    let (status, complaints) = self.stopped().await;
                    return Err(conversation.stopped(status, &complaints));
                };
                match conversation.heard(&line) {
                    Exchange::Continue(lines) => {
                        for line in lines {
                            self.write(&line).await;
                        }
                    }
                    Exchange::Over(outcome) => return outcome,
                }
            }
        }
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

    /// A stand-in instance, for the same reason [`built`] is one: the label
    /// has to be asserted, and minting an identity needs randomness this crate
    /// deliberately does not take.
    fn an_instance() -> InstanceId {
        InstanceId::from_uuid(Uuid::from_u128(0x5747))
    }

    /// The same identifier as a literal, which is what the assertions compare
    /// against.
    ///
    /// Never `built().as_argument()`, and that is the whole point of it
    /// existing: an assertion that reads the value back through the method
    /// under test compares a mutation to itself and passes. Mutation testing
    /// found exactly that — [`Image::as_argument`] could return an empty
    /// string with every argument test still green.
    const BUILT: &str = "stageman:0123456789abcdef";

    /// Every fragment ends in a newline, so composing cannot glue two
    /// instructions into one.
    ///
    /// Nothing is inserted between fragments — see [`recipe`] — so this is the
    /// whole of what keeps the seams valid. The failure it prevents is the
    /// worst shape available: a recipe that builds something subtly different,
    /// reported by the runtime as a syntax error in a line no file contains.
    #[test]
    fn every_fragment_ends_where_the_next_can_begin() {
        for (named, fragment) in [
            ("the base", BASE),
            ("the holding command", HOLDING),
            ("the platform layer", PLATFORM),
            ("claude's adapter", installing(Agent::Claude)),
        ] {
            assert!(
                fragment.ends_with('\n'),
                "{named} does not end in a newline, so whatever follows it \
                 would continue its last line",
            );
        }
    }

    /// A composed recipe declares exactly one image to build on, first.
    ///
    /// Both halves matter and each fails differently. A second `FROM` starts a
    /// second stage, and the build would answer with that one; a recipe whose
    /// first instruction is not a `FROM` is refused outright. This is what
    /// makes the composition order in [`recipe`] checkable without a runtime.
    #[test]
    fn every_recipe_builds_on_exactly_one_base_named_first() {
        for role in Role::ALL.iter().copied() {
            let composed = recipe(Agent::Claude, role);
            let instructions: Vec<&str> = composed
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect();

            assert_eq!(
                instructions
                    .iter()
                    .filter(|line| line.starts_with("FROM"))
                    .count(),
                1,
                "{role:?} composes more than one stage",
            );
            assert!(
                instructions
                    .first()
                    .is_some_and(|first| first.starts_with("FROM")),
                "{role:?} names something before the image it builds on",
            );
        }
    }

    /// A job's recipe is a foreman's with a layer appended, byte for byte.
    ///
    /// The property `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`
    /// used two stages to get, and the reason the platform fragment is last:
    /// identical instructions up to the split are what make the two images
    /// share every layer but one, and the second build a cache hit throughout.
    /// Asserted here because the alternative is noticing it as a slow build.
    #[test]
    fn a_jobs_recipe_begins_with_the_whole_of_a_foremans() {
        let foreman = recipe(Agent::Claude, Role::Foreman);
        let job = recipe(Agent::Claude, Role::Job);

        assert!(
            job.starts_with(&foreman),
            "a job's recipe has to extend a foreman's rather than rearrange it",
        );
        assert_ne!(job, foreman, "a job's recipe adds nothing");
    }

    /// Only the fragment that exists to name a command names one.
    ///
    /// The order in [`recipe`] puts the holding command after an adapter's
    /// fragment so that nothing can override it. That protection is worth
    /// nothing if a fragment further along names one too, and a `CMD` in the
    /// platform layer would be a container that starts perfectly and never
    /// speaks — the failure
    /// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
    /// spends a paragraph on.
    #[test]
    fn the_command_is_named_once_and_by_the_fragment_that_holds_a_container_open() {
        let names_a_command = |fragment: &str| {
            fragment
                .lines()
                .map(str::trim)
                .any(|line| line.starts_with("CMD") || line.starts_with("ENTRYPOINT"))
        };

        assert!(names_a_command(HOLDING));
        for (named, fragment) in [
            ("the base", BASE),
            ("the platform layer", PLATFORM),
            ("claude's adapter", installing(Agent::Claude)),
        ] {
            assert!(
                !names_a_command(fragment),
                "{named} names a command, which would replace the one that holds \
                 a container open",
            );
        }
    }

    /// The two roles are two recipes, so they are two images.
    ///
    /// What used to be a copy-paste in `stage` handing a foreman a job's
    /// image — which is the whole of what 0036 refuses — is now a copy-paste
    /// producing one name for both.
    #[test]
    fn a_foreman_and_a_job_are_not_named_the_same_image() {
        let foreman = named(&recipe(Agent::Claude, Role::Foreman));
        let job = named(&recipe(Agent::Claude, Role::Job));
        assert_ne!(foreman, job);
    }

    /// A name is this project's repository and the recipe's digest.
    ///
    /// The shape rather than the value, because the value is whatever the
    /// fragments currently say and pinning it here would make every recipe
    /// edit a failing test that teaches nobody anything.
    #[test]
    fn an_image_is_named_for_the_repository_and_the_digest_of_its_recipe() {
        let image = named(&recipe(Agent::Claude, Role::Job));
        let (repository, digest) = image
            .as_argument()
            .split_once(':')
            .expect("a name is a repository and a tag");

        assert_eq!(repository, "stageman");
        assert_eq!(digest.len(), 64, "a sha256 is 64 hexadecimal characters");
        assert!(digest.chars().all(|each| each.is_ascii_hexdigit()));
    }

    /// The same recipe is the same name, every time.
    ///
    /// The property the whole design rests on, and the one the runtime does
    /// not have: two builds of identical bytes were measured to produce two
    /// images with different identifiers, which is what an image per container
    /// was made of.
    #[test]
    fn one_recipe_is_one_name_however_often_it_is_asked_for() {
        let recipe = recipe(Agent::Claude, Role::Job);
        assert_eq!(named(&recipe), named(&recipe));
    }

    /// What a sweep keeps is one image per agent per role.
    ///
    /// Guards the list rather than the arithmetic: a role or an agent added
    /// without being reachable here is an image reclaimed at every startup and
    /// rebuilt at the next container, which is slow rather than broken and so
    /// would not be noticed.
    #[test]
    fn what_a_sweep_keeps_is_every_image_this_build_would_use() {
        let keeping = keeping();
        assert_eq!(keeping.len(), Agent::ALL.len() * Role::ALL.len());

        for agent in Agent::ALL.iter().copied() {
            for role in Role::ALL.iter().copied() {
                assert!(
                    keeping.contains(&named(&recipe(agent, role))),
                    "{agent:?} as {role:?} would be reclaimed and rebuilt",
                );
            }
        }
    }

    /// A listing is the names in it, and never a name that is not one.
    ///
    /// An image whose name went between the listing and the read is reported
    /// The listing asks the runtime for this project's images, by name.
    ///
    /// Every part is load-bearing and none is checked by anything else: the
    /// wrong subcommand lists containers, a missing filter lists the whole
    /// machine, and a format that is not the repository and tag produces lines
    /// no removal can address. A sweep built on any of those reclaims nothing
    /// and says it swept.
    #[test]
    fn a_listing_asks_for_this_projects_images_and_their_names() {
        let arguments = ours_arguments();

        assert_eq!(arguments.first().map(String::as_str), Some("images"));
        assert!(
            arguments.iter().any(|each| each == "reference=stageman"),
            "{arguments:?}",
        );
        assert!(
            arguments
                .iter()
                .any(|each| each.contains("{{.Repository}}") && each.contains("{{.Tag}}")),
            "{arguments:?}",
        );
    }

    /// What a sweep keeps is exactly what it would build, by name.
    ///
    /// The decision that says whether anything is reclaimed. Inverted, a sweep
    /// removes the image the next container wants and keeps every one nothing
    /// will ask for again — which is slow rather than broken, and so would go
    /// unnoticed.
    #[test]
    fn an_image_this_build_would_use_is_kept_and_any_other_is_not() {
        let keeping = keeping();
        let current = named(&recipe(Agent::Claude, Role::Job));

        assert!(kept(&keeping, current.as_argument()));
        assert!(
            !kept(&keeping, &format!("{REPOSITORY}:{}", "0".repeat(64))),
            "a name nothing would build is not kept",
        );
        assert!(
            !kept(&[], current.as_argument()),
            "nothing is kept from nothing"
        );
    }

    /// with the runtime's word for *no name*, and passing that to a removal
    /// would address something other than what was meant.
    #[test]
    fn an_image_listing_drops_what_is_not_a_name() {
        let reported = "stageman:abc\n\n<none>:<none>\n  stageman:def  \n";
        assert_eq!(tagged(reported), vec!["stageman:abc", "stageman:def"]);
        assert!(tagged("\n  \n").is_empty());
    }

    /// A build reads its recipe from standard input and names no context.
    #[test]
    fn a_build_takes_its_recipe_on_standard_input_and_no_context() {
        let image = built();
        let arguments = build_arguments(&image);
        assert_eq!(arguments[0], "build");
        assert_eq!(
            arguments.last(),
            Some(&"-".to_owned()),
            "the trailing dash is the recipe arriving on standard input",
        );
    }

    /// A build names what it is building, and asks about that same name.
    ///
    /// The pair is the point. A build that tagged nothing would leave an image
    /// nobody can find, and a presence check asking about anything else would
    /// answer for a different image — either way every container gets its own,
    /// which is the state this replaced.
    #[test]
    fn a_build_and_the_question_before_it_name_one_image() {
        let image = built();
        assert!(build_arguments(&image).iter().any(|word| word == "--tag"));
        assert!(build_arguments(&image).iter().any(|word| word == BUILT));
        assert_eq!(present_arguments(&image).last(), Some(&BUILT.to_owned()));
    }

    /// A build that worked is the image it was told to build.
    ///
    /// Nothing is read back from the build to learn that. The name was decided
    /// before the build began, which is what lets a container be started from
    /// an image the build did not have to create.
    #[test]
    fn a_build_that_succeeded_is_the_image_it_named() {
        let image = outcome(true, built(), b"").expect("a build that worked is an image");
        assert_eq!(image.as_argument(), BUILT);
    }

    /// A build that failed is not an image, however much it printed.
    ///
    /// The other half of the one above, and the pair is the point: either on
    /// its own would still pass with the test of success inverted.
    #[test]
    fn a_build_that_failed_is_not_an_image() {
        let failure = outcome(false, built(), b"#4 ERROR: exit code 1\n");
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

    #[test]
    fn nothing_is_looked_for_relative_to_wherever_this_started() {
        // Every platform's list, on whichever machine this runs, which is
        // the point of the lists being values rather than one compiled
        // arm: what a mac would look for is checkable from a Linux box.
        for target in [Target::MacOs, Target::Linux, Target::Windows] {
            let looked = candidates(target);
            assert!(!looked.is_empty(), "{target:?} knows nowhere to look");
            for candidate in looked {
                let absolute = if target == Target::Windows {
                    // A drive letter, which is what absolute means there and
                    // what `Path` cannot tell from a Unix machine.
                    candidate.starts_with(r"C:\")
                } else {
                    Path::new(candidate).is_absolute()
                };
                assert!(absolute, "{candidate} is not an absolute path");
            }
        }
        assert!(
            candidates(Target::Unknown).is_empty(),
            "a platform nothing knows looks nowhere"
        );
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
    /// It builds the image it greets, so it covers the build as well as the
    /// exchange. **Both roles**, because two images is the thing 0036 claims
    /// and one greeting proves nothing about the other — and because a
    /// foreman's is the one whose stage could be edited into uselessness
    /// without any test of a job's noticing. What is said and heard is what
    /// a conversation says and hears, rendered and read the same way.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_container_answers_the_protocol() {
        use agent_client_protocol::schema::v1::InitializeResponse;

        let runtime = located_runtime();

        for role in [Role::Foreman, Role::Job] {
            let image = build(&runtime, Agent::Claude, role)
                .await
                .unwrap_or_else(|why| panic!("{role:?} builds: {why}"));
            let mut adapter = Pump::start(&runtime, &alone(&image));
            adapter.say(&Said::Initialize { id: 1 }).await;
            let result = adapter
                .answer(1)
                .await
                .unwrap_or_else(|why| panic!("{role:?} answers the handshake: {why}"));
            let greeting: InitializeResponse =
                serde_json::from_value(result).expect("the protocol's own greeting");
            assert_eq!(greeting.protocol_version, ProtocolVersion::V1);
            let adapter = greeting.agent_info.expect("the adapter names itself");
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

    use stageman_core::{AgentConfig, Handout, Job, Project, ProjectId, State, Uuid};
    use std::collections::BTreeMap;

    /// An instance configured with one agent and nothing else.
    /// An instance with one agent configured and nothing else.
    fn instance(credential: &str) -> State {
        State {
            apps: std::collections::BTreeMap::new(),
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new(credential.to_owned()),
                },
            )]),
            ..State::default()
        }
    }

    fn only_claude() -> std::collections::BTreeMap<stageman_core::KitName, stageman_core::KitConfig>
    {
        std::collections::BTreeMap::from([(
            stageman_core::KitName::new("Claude").expect("a name"),
            stageman_core::KitConfig::defaults(Agent::Claude),
        )])
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
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: only_claude(),
                credentials,
                channels: BTreeMap::new(),
                variables: BTreeMap::new(),
                jobs: BTreeMap::<_, Job>::new(),
                attending: stageman_core::Attending::default(),
                brief: String::new(),
                watched: std::collections::BTreeSet::new(),
                foreman_room: None,
            },
        );
        (state, id)
    }

    /// The same, with a channel bound.
    ///
    /// Separate from the fixture above rather than replacing it, because both
    /// shapes are real and each is the subject of its own claim: a project
    /// with nothing bound is what the last release could write, and what its
    /// agent is handed still has to be decidable.
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
                    credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                    listen_credential: Secret::new("xapp-not-a-real-token".to_owned()),
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
    /// The names one handout is delivered, in order.
    ///
    /// Shared because several tests assert names and not values, and because
    /// the delivery helper is fallible now — unwrapping it in six places would
    /// say less than naming the expectation once.
    fn names_of(handout: &Handout) -> Vec<String> {
        environment(handout)
            .expect("a handout with no reserved name")
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[test]
    fn a_thread_is_never_delivered_as_a_variable() {
        let (state, project) = instance_with_a_channel("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
            .expect("a watched project")
            .speaking_in(stageman_core::Place::from(stageman_core::Thread {
                channel: Channel::Slack,
                room: "C0123456789".to_owned(),
                id: "1728312345.678901".to_owned(),
            }));

        let delivering = environment(&handout).expect("a handout with no reserved name");
        let named = names_of(&handout);

        assert!(
            !named.iter().any(|name| name.contains("THREAD")),
            "the thread goes in a file, not the environment: {named:?}"
        );
        // And nothing carries its value under another name either.
        for (_, value) in &delivering {
            assert_ne!(value.expose(), "1728312345.678901");
        }
        // Nor the channel's credential, since 0034: the daemon posts, so a
        // container has no use for one and holding less is the whole
        // mitigation.
        assert!(
            !named.iter().any(|name| name == "STAGEMAN_SLACK_CHANNEL"),
            "{named:?}"
        );
        assert!(
            !named.iter().any(|name| name == "STAGEMAN_SLACK_TOKEN"),
            "{named:?}"
        );
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
            Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
                .expect("a watched project"),
        ] {
            let named = names_of(&handout);
            assert!(
                !named.iter().any(|name| name == "STAGEMAN_SLACK_TOKEN"),
                "{named:?}"
            );
            assert!(
                !named.iter().any(|name| name == "STAGEMAN_SLACK_CHANNEL"),
                "{named:?}"
            );
        }

        // And the asymmetry 0027 turns on is unchanged: a foreman watches a
        // channel and still acts on no platform.
        let foreman = Handout::for_foreman(&state, project).expect("a watched project");
        let named = names_of(&foreman);
        assert!(!named.iter().any(|name| name == "GH_TOKEN"), "{named:?}");
    }

    /// A project with nothing bound is delivered nothing to speak with, rather
    /// than an empty variable — which would leave `stageman-say` unable to tell
    /// an unbound project from a broken one.
    #[test]
    fn a_job_with_no_channel_is_delivered_no_channel_variables() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
            .expect("a watched project");

        let named = names_of(&handout);

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

        let delivered = environment(&handout).expect("a handout with no reserved name");

        assert_eq!(delivered.len(), 1, "{delivered:?}");
        assert_eq!(delivered[0].0, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(delivered[0].1.expose(), "sk-ant-oat01-xyz");
    }

    #[test]
    fn a_job_is_delivered_the_variable_its_platform_tool_reads() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
            .expect("a watched project");

        let named = names_of(&handout);

        assert!(
            named.iter().any(|name| name == "CLAUDE_CODE_OAUTH_TOKEN"),
            "{named:?}"
        );
        assert!(named.iter().any(|name| name == "GH_TOKEN"), "{named:?}");
    }

    /// A project's own variables reach its jobs' containers.
    ///
    /// The whole of what the feature does, from the delivery side.
    #[test]
    fn a_job_is_delivered_its_projects_variables() {
        let (mut state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .variables
            .insert(
                stageman_core::VariableName::new("STRIPE_API_KEY").expect("a deliverable name"),
                stageman_core::Variable::unexplained(Secret::new(
                    "sk-test-not-a-real-key".to_owned(),
                )),
            );
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
            .expect("a watched project");

        let delivering = environment(&handout).expect("no reserved name here");
        let found = delivering
            .iter()
            .find(|(name, _)| name == "STRIPE_API_KEY")
            .expect("the project's variable is delivered");

        assert_eq!(found.1.expose(), "sk-test-not-a-real-key");
    }

    /// And a foreman's container is given none of them.
    #[test]
    fn a_foreman_is_delivered_no_variable_of_its_projects() {
        let (mut state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .variables
            .insert(
                stageman_core::VariableName::new("STRIPE_API_KEY").expect("a deliverable name"),
                stageman_core::Variable::unexplained(Secret::new(
                    "sk-test-not-a-real-key".to_owned(),
                )),
            );
        let foreman = Handout::for_foreman(&state, project).expect("a watched project");

        let named = names_of(&foreman);

        assert!(
            !named.iter().any(|name| name == "STRIPE_API_KEY"),
            "{named:?}"
        );
    }

    /// The refusal that keeps an operator from silently changing who pays.
    ///
    /// A variable named for the agent's own credential would otherwise be set
    /// twice, and the last one wins — with no error and no log line, which is
    /// exactly the failure `docs/decisions/0008-one-credential-per-agent.md`
    /// exists to prevent. The dashboard refuses this when it is typed; this is
    /// what a hand-edited snapshot meets.
    #[test]
    fn a_variable_claiming_a_name_this_project_delivers_is_refused() {
        for claimed in RESERVED {
            let (mut state, project) = instance_with_a_project("sk-ant-oat01-xyz");
            state
                .projects
                .get_mut(&project)
                .expect("the project")
                .variables
                .insert(
                    stageman_core::VariableName::new(*claimed).expect("a deliverable name"),
                    stageman_core::Variable::unexplained(Secret::new(
                        "somebody-elses-account".to_owned(),
                    )),
                );
            let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
                .expect("a watched project");

            let refused = environment(&handout).expect_err("that name is ours");

            assert!(
                matches!(refused, AgentError::ReservedVariable { ref name } if name == claimed),
                "{refused:?}"
            );
        }
    }

    /// What keeps [`RESERVED`] honest as agents are added.
    ///
    /// The list is written by hand, so nothing makes it follow the adapters it
    /// describes. This is what notices: everything a real handout delivers on
    /// this project's own account has to be in it, so an agent whose credential
    /// goes in a new variable fails here until somebody adds it — rather than
    /// silently letting an operator claim that name.
    ///
    /// Both of Claude's credential variables are covered because the two
    /// fixtures below differ in the shape of the token, which is what chooses
    /// between them.
    #[test]
    fn every_name_this_project_delivers_is_one_it_reserves() {
        for credential in ["sk-ant-oat01-xyz", "sk-ant-api03-xyz"] {
            let (state, project) = instance_with_a_project(credential);
            let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
                .expect("a watched project");

            for name in names_of(&handout) {
                assert!(
                    RESERVED.contains(&name.as_str()),
                    "{name} is delivered but not reserved, so an operator could claim it",
                );
            }
        }
    }

    /// The one that matters most in this module. A secret on a command line is
    /// readable by every user on the machine through the process table, so the
    /// arguments must *name* each variable and never carry its value.
    #[test]
    fn no_credential_ever_appears_in_a_containers_arguments() {
        let (state, project) = instance_with_a_channel("sk-ant-oat01-secret-value");
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
            .expect("a watched project");

        let arguments = retained_arguments(
            "stageman-job-abc",
            &built(),
            Agent::Claude,
            an_instance(),
            &environment(&handout).expect("a handout with no reserved name"),
        );
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

    /// A container an agent works in reaches the network, and is not the
    /// agent: it holds itself open, and the agent is run inside it.
    #[test]
    fn a_retained_container_is_not_cut_off_from_the_network_and_is_not_the_agent() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let arguments = retained_arguments(
            "stageman-job-abc",
            &built(),
            Agent::Claude,
            an_instance(),
            &environment(&handout).expect("a handout with no reserved name"),
        );

        assert!(!arguments.iter().any(|a| a == "none"), "{arguments:?}");
        assert!(
            !arguments.iter().any(|a| a == AGENT_PROGRAM),
            "the image holds the container open; the agent is run inside it: {arguments:?}",
        );
    }

    /// Tests that spend real money, kept in their own module so a filter can
    /// name them as a group rather than one at a time. Run with
    /// `just image-session`; `just image-handshake` deliberately excludes them,
    /// because everything it runs needs only a runtime and a network.

    #[test]
    fn a_retained_container_is_named_labelled_and_survives_its_own_exit() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let arguments = retained_arguments(
            "stageman-job-abc",
            &built(),
            Agent::Claude,
            an_instance(),
            &environment(&handout).expect("a handout with no reserved name"),
        );
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

        let arguments = retained_arguments(
            "stageman-job-abc",
            &built(),
            Agent::Claude,
            an_instance(),
            &environment(&handout).expect("a handout with no reserved name"),
        );
        let line = arguments.join(" ");

        assert!(
            line.contains(&format!("--publish 127.0.0.1::{TUNNEL_PORT}")),
            "{line}"
        );
    }

    /// Every container this project creates says which instance made it.
    ///
    /// The label a sweep is allowed to remove things on the strength of, so
    /// its absence is not a cosmetic loss: without it every container looks
    /// like this instance's own abandoned work.
    #[test]
    fn a_retained_container_says_which_instance_started_it() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), project)
            .expect("a watched project");

        let arguments = retained_arguments(
            "stageman-job-abc",
            &built(),
            Agent::Claude,
            an_instance(),
            &environment(&handout).expect("a handout with no reserved name"),
        );

        let line = arguments.join(" ");
        assert!(
            line.contains(&format!("stageman.instance={}", an_instance())),
            "{arguments:?}",
        );
    }

    /// A label is read back as the instance it names, and anything else is
    /// nobody.
    ///
    /// Every uncertainty has to answer *nobody*, because that is the answer a
    /// sweep leaves alone. Reading an unparseable label as an instance would
    /// be inventing an owner; reading a missing one as this instance would let
    /// a sweep remove containers it cannot actually place.
    #[test]
    fn an_instance_label_reads_back_only_when_it_is_one() {
        assert_eq!(minted(&an_instance().to_string()), Some(an_instance()));
        assert_eq!(
            minted(&format!("  {}  ", an_instance())),
            Some(an_instance())
        );
        assert_eq!(minted(""), None, "a container carrying no such label");
        assert_eq!(minted("   \n"), None, "what a runtime prints for one");
        assert_eq!(
            minted("not-a-uuid"),
            None,
            "a label that is not an identity"
        );
    }

    /// The question is asked of the named container, and of nothing else.
    #[test]
    fn asking_which_instance_started_a_container_names_that_container() {
        let arguments = Command::Label {
            name: "stageman-job-abc".to_owned(),
            label: Label::Instance,
        }
        .arguments();
        assert_eq!(arguments[0], "inspect");
        assert_eq!(
            arguments.last().map(String::as_str),
            Some("stageman-job-abc")
        );
        assert!(
            arguments.iter().any(|each| each.contains(INSTANCE_LABEL)),
            "{arguments:?}",
        );
    }

    /// A listing is the names in it, and nothing blank.
    ///
    /// The blank matters more than it looks: a runtime that found nothing
    /// prints an empty line, and an empty name reaches a subcommand that would
    /// then address something other than what was meant.
    #[test]
    fn a_listing_is_the_names_in_it_and_nothing_blank() {
        assert_eq!(
            names("stageman-job-one\n  stageman-job-two  \n"),
            vec!["stageman-job-one".to_owned(), "stageman-job-two".to_owned()],
        );
        assert!(names("").is_empty(), "a runtime that found nothing");
        assert!(
            names("\n  \n\n").is_empty(),
            "and the blank lines it prints"
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

    /// Making sure a container is up names it and does not attach.
    ///
    /// The absence is the assertion. Attaching here is what this stopped
    /// doing: the agent runs inside the container rather than being it, so a
    /// stray `--interactive` would hold this process against a container that
    /// never exits and the turn would never start.
    #[test]
    fn holding_a_container_open_names_it_and_attaches_to_nothing() {
        assert_eq!(
            holding_arguments("stageman-foreman-abc"),
            vec!["start".to_owned(), "stageman-foreman-abc".to_owned()],
        );
    }

    /// The agent is run inside the container, over pipes this process owns.
    ///
    /// `--interactive` moved here, and it is the only thing between a running
    /// container and a conversation: without it the agent gets no standard
    /// input, which reads as an agent that will not speak.
    #[test]
    fn the_agent_runs_inside_the_container_with_a_pipe_of_its_own() {
        assert_eq!(
            agent_arguments("stageman-job-abc"),
            vec![
                "exec".to_owned(),
                "--interactive".to_owned(),
                "stageman-job-abc".to_owned(),
                AGENT_PROGRAM.to_owned(),
            ],
        );
    }

    /// A container is created with an init, or nothing reaps what it orphans.
    ///
    /// Cheap to lose and expensive to notice: without it the process holding
    /// the container open is process one, which reaps nothing and ignores the
    /// signal that stops a container — so a long job accumulates zombies and
    /// every stop waits for a timeout first.
    #[test]
    fn a_retained_container_is_created_with_an_init() {
        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");

        let arguments = retained_arguments(
            "stageman-job-abc",
            &built(),
            Agent::Claude,
            an_instance(),
            &environment(&handout).expect("a handout with no reserved name"),
        );

        assert!(arguments.iter().any(|a| a == "--init"), "{arguments:?}");
    }

    /// Runs one of the commands the instance renders, against a real
    /// runtime.
    ///
    /// What the instance asks the world for is an argument list, so a test
    /// that drives the same list is evidence about the thing it will meet.
    fn asking(runtime: &ContainerRuntime, command: &Command) -> std::process::Output {
        std::process::Command::new(runtime.path())
            .args(command.arguments())
            .output()
            .expect("the runtime runs")
    }

    /// Every image of ours the runtime holds right now.
    fn ours_now(runtime: &ContainerRuntime) -> Vec<String> {
        let listed = asking(runtime, &Command::Images);
        assert!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        tagged(&String::from_utf8_lossy(&listed.stdout))
    }

    /// A sweep removes an image nothing needs, keeps the ones a container is
    /// using, and keeps the ones the next container will want.
    ///
    /// The only function here that destroys something an operator would miss,
    /// so it is worth the minutes: everything it decides is decided against a
    /// live runtime, and the refusal that protects an image in use is the
    /// runtime's rather than this crate's — which means no unit test can
    /// stand in for it.
    ///
    /// The image that ought to go is made by naming an existing one a second
    /// time, which is what a recipe edit leaves behind: a name under this
    /// project's repository that nothing will ask for again.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_sweep_reclaims_what_nothing_needs_and_keeps_what_something_does() {
        let runtime = located_runtime();
        let name = "stageman-job-reclaim-probe";
        discard(&runtime, name).await.expect("a clean slate");

        let current = build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");

        // A name of ours that no container will ever want: the same image
        // under a second name, which is exactly what an edited recipe leaves.
        let stale = format!("{REPOSITORY}:{}", "0".repeat(64));
        let named_twice = std::process::Command::new(runtime.path())
            .args(["tag", current.as_argument(), &stale])
            .output()
            .expect("the runtime runs");
        assert!(
            named_twice.status.success(),
            "{}",
            String::from_utf8_lossy(&named_twice.stderr)
        );

        // And a container holding the current one, so the sweep has something
        // it must refuse to take.
        let created = std::process::Command::new(runtime.path())
            .args([
                "create",
                "--name",
                name,
                "--label",
                &format!("{OWNER_LABEL}={name}"),
                current.as_argument(),
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );

        // The sweep as the instance performs it: ask which images are ours,
        // keep the ones this build would only rebuild, remove the rest. The
        // deciding is pure and tested in memory; what needs a live runtime
        // is what the two commands actually do.
        let keeping = keeping();
        for image in ours_now(&runtime) {
            if kept(&keeping, &image) {
                continue;
            }
            drop(asking(&runtime, &Command::RemoveImage { image }));
        }

        // Asserted on what is left rather than on how many went, because
        // another test may be sweeping the same daemon at the same moment and
        // the count is the one thing that is genuinely theirs to change. What
        // is left is not: an image nothing needs is gone whoever removed it,
        // and one a container needs survives either way.
        let left = ours_now(&runtime);
        assert!(
            !left.contains(&stale),
            "a name nothing needs is still here: {left:?}",
        );
        assert!(
            left.contains(&current.as_argument().to_owned()),
            "the image the next container wants was reclaimed: {left:?}",
        );

        // And what a container is holding is still startable, which is the
        // property the unforced removal exists to keep.
        assert!(
            present(&runtime, &current).await,
            "the image a container is using was taken from under it",
        );

        discard(&runtime, name).await.expect("it is removable");
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

    /// A container outlives a process run inside it, and stops when told.
    ///
    /// The mechanism this whole decision turns on, and the one thing no unit
    /// test can reach: that a container created from this image holds itself
    /// open, that running something inside it and letting that finish does not
    /// take it down, and that stopping it does. Every one of those is a
    /// property of the runtime and the image together.
    ///
    /// It runs `true` rather than the agent, which is the point — a credential
    /// would only make the process inside more interesting, and what is being
    /// asserted is what happens to the container when *any* process in it
    /// ends. `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
    /// replaced a measurement with this one.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_container_outlives_what_runs_inside_it() {
        let runtime = located_runtime();
        let name = "stageman-job-lifetime-probe";
        discard(&runtime, name).await.expect("a clean slate");

        let (state, project) = instance_with_a_project("sk-ant-oat01-xyz");
        let handout = Handout::for_foreman(&state, project).expect("a watched project");
        let delivering = environment(&handout).expect("a handout with no reserved name");
        let image = build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");

        let created = tokio::process::Command::new(runtime.path())
            .args(retained_arguments(
                name,
                &image,
                Agent::Claude,
                an_instance(),
                &delivering,
            ))
            .envs(
                delivering
                    .iter()
                    .map(|(named, value)| (named.clone(), value.expose().to_owned())),
            )
            .output()
            .await
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        // The label a foreman's turn boundary reads, asserted on a container
        // made the way every retained container is made.
        assert_eq!(
            made_for(&runtime, name).await.expect("the runtime answers"),
            Some(Agent::Claude),
            "a created container says which agent it was made for",
        );

        hold(&runtime, name).await.expect("it starts");
        assert!(
            running(&runtime)
                .await
                .expect("a listing")
                .iter()
                .any(|up| up == name),
            "the image has to hold the container open with nothing running in it",
        );

        // Something that finishes at once. Under the mechanism this replaced,
        // a process ending was the container ending.
        let inside = tokio::process::Command::new(runtime.path())
            .args(["exec", name, "true"])
            .output()
            .await
            .expect("the runtime runs");
        assert!(
            inside.status.success(),
            "{}",
            String::from_utf8_lossy(&inside.stderr)
        );
        assert!(
            running(&runtime)
                .await
                .expect("a listing")
                .iter()
                .any(|up| up == name),
            "a container has to outlive a process that ran inside it",
        );

        halt(&runtime, name).await.expect("it stops");
        let up = running(&runtime).await.expect("a listing");
        let left = abandoned(&runtime).await.expect("a listing");
        assert!(!up.iter().any(|running| running == name), "{up:?}");
        assert!(
            left.iter().any(|behind| behind == name),
            "stopping keeps the container and its session: {left:?}",
        );

        discard(&runtime, name).await.expect("it is removable");
    }

    /// Asserted whole, the way a kickoff is: this is the one place the
    /// project types commands into a container on a job's behalf, and a
    /// change to it changes what every job starts from without any other
    /// test noticing.
    #[test]
    fn the_checkout_reads_exactly_as_written() {
        assert_eq!(
            checkout_script(Some(Platform::GitHub)),
            "set -eu\n\
             gh repo clone \"$STAGEMAN_REPOSITORY\" .\n\
             gh auth setup-git --hostname github.com\n\
             account=\"$(gh api user --jq '\"\\(.id)+\\(.login)\"')\"\n\
             git config --global user.name \"${account#*+}\"\n\
             git config --global user.email \"${account}@users.noreply.github.com\"\n"
        );
        assert_eq!(
            checkout_script(None),
            "set -eu\ngit clone \"$STAGEMAN_REPOSITORY\" .\n"
        );
    }

    /// The repository travels on the one command that needs it and never in
    /// the script, so the script is the same text for every job and a URL
    /// needs no quoting.
    #[test]
    fn the_checkout_runs_in_the_named_container_with_the_repository_in_its_environment() {
        let arguments = checkout_arguments(
            "stageman-job-1",
            "https://example.invalid/repo.git",
            Some(Platform::GitHub),
        );

        assert_eq!(
            &arguments[..4],
            [
                "exec",
                "--env",
                "STAGEMAN_REPOSITORY=https://example.invalid/repo.git",
                "stageman-job-1",
            ]
        );
        assert_eq!(&arguments[4..6], ["sh", "-c"]);
        assert_eq!(arguments[6], checkout_script(Some(Platform::GitHub)));
        assert_eq!(arguments.len(), 7);
        assert!(
            !arguments[6].contains("example.invalid"),
            "the script must not carry the URL: {}",
            arguments[6]
        );
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

    /// The checkout is made in a container that is up, before any agent runs,
    /// and lands at the workspace root — where the adapter looks for a
    /// project's settings, per
    /// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    ///
    /// Needs a runtime and the network, and no credential: with no platform
    /// credential the checkout is plain git against a public repository,
    /// which is the path a job with nothing to authenticate with takes. What
    /// a signed-in checkout adds — the helper and the identity — is asserted
    /// as text above and exercised by the job that runs end to end under
    /// `just image-session`.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_repository_is_checked_out_before_any_agent_runs() {
        let runtime = located_runtime();
        let name = "stageman-job-checkout-probe";
        discard(&runtime, name).await.expect("a clean slate");
        let image = build(&runtime, Agent::Claude, Role::Job)
            .await
            .expect("the image builds");

        let created = std::process::Command::new(runtime.path())
            .args(retained_arguments(
                name,
                &image,
                Agent::Claude,
                an_instance(),
                &[],
            ))
            .output()
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        hold(&runtime, name).await.expect("it starts");

        check_out(&runtime, name, PUBLIC_REPOSITORY, None)
            .await
            .expect("a public repository checks out without a credential");

        let inspected = std::process::Command::new(runtime.path())
            .args([
                "exec",
                name,
                "git",
                "-C",
                WORKSPACE,
                "rev-parse",
                "--show-toplevel",
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            inspected.status.success(),
            "{}",
            String::from_utf8_lossy(&inspected.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&inspected.stdout).trim(),
            WORKSPACE,
            "the checkout is the workspace itself, not a directory inside it",
        );

        // A workspace that already holds a checkout is refused loudly rather
        // than nested into or overwritten.
        let again = check_out(&runtime, name, PUBLIC_REPOSITORY, None).await;
        assert!(
            matches!(again, Err(AgentError::Checkout { .. })),
            "{again:?}"
        );

        discard(&runtime, name).await.expect("it is removable");
    }

    /// A repository anybody can clone, for the checkout above.
    const PUBLIC_REPOSITORY: &str = "https://github.com/octocat/Hello-World";

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
                    foreman_kit: Kit::defaults(Agent::Claude),
                    kits: only_claude(),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    variables: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
                    brief: String::new(),
                    watched: std::collections::BTreeSet::new(),
                    foreman_room: None,
                },
            );
            let handout = Handout::for_foreman(&state, project).expect("a watched project");
            (state, handout)
        }

        /// A container for a foreman's handout, made and started the way
        /// the instance makes and starts one, with the credential forwarded.
        async fn made(runtime: &ContainerRuntime, name: &str, handout: &Handout) {
            discard(runtime, name).await.expect("a clean slate");
            let delivering = environment(handout).expect("a handout with no reserved name");
            let image = build(runtime, Agent::Claude, Role::Foreman)
                .await
                .expect("the image builds");
            let created = tokio::process::Command::new(runtime.path())
                .args(retained_arguments(
                    name,
                    &image,
                    Agent::Claude,
                    an_instance(),
                    &delivering,
                ))
                .envs(
                    delivering
                        .iter()
                        .map(|(named, value)| (named.clone(), value.expose().to_owned())),
                )
                .output()
                .await
                .expect("the runtime runs");
            assert!(
                created.status.success(),
                "{}",
                String::from_utf8_lossy(&created.stderr)
            );
            hold(runtime, name).await.expect("it starts");
        }

        /// One conversation with the agent in a container that is up, driven
        /// as the instance drives one.
        async fn talk(
            runtime: &ContainerRuntime,
            name: &str,
            opening: Opening,
            kit: &Kit,
            question: &str,
        ) -> Result<Answer, AgentError> {
            let (conversation, first) = Conversation::begin(opening, None, kit.clone(), question);
            let mut agent = Pump::start(runtime, &agent_arguments(name));
            agent.talk(conversation, first).await
        }

        #[tokio::test]
        #[ignore = "needs a container runtime, a built image and a credential; run `just image-session`"]
        async fn an_agent_answers_a_question() {
            let runtime = located_runtime();
            let (_state, handout) = handout_of();
            let name = "stageman-job-question-probe";
            made(&runtime, name, &handout).await;

            let answer = talk(
                &runtime,
                name,
                Opening::Fresh,
                handout.kit(),
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
            assert!(
                answer.reported.contains_key("model"),
                "what the session reported it was set to: {:?}",
                answer.reported
            );
            discard(&runtime, name).await.expect("it is removable");
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
            made(&runtime, name, &handout).await;

            let first = talk(
                &runtime,
                name,
                Opening::Fresh,
                handout.kit(),
                "Remember this word and reply with it, alone: marmalade",
            )
            .await
            .expect("the agent answers");
            assert!(
                first.text.to_lowercase().contains("marmalade"),
                "said {:?}",
                first.text
            );

            // Stopped, as a container is when nothing answers on its tunnel,
            // and started again, as a container is when a reply arrives.
            halt(&runtime, name).await.expect("it stops");
            let left = abandoned(&runtime).await.expect("the runtime answers");
            assert!(left.iter().any(|c| c == name), "it should still be there");
            hold(&runtime, name).await.expect("it starts again");

            let second = talk(
                &runtime,
                name,
                Opening::Resumed,
                handout.kit(),
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
        /// cut off mid-turn rather than between turns. Dropping the process
        /// kills the client exactly as a hard kill of the daemon would.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image and a credential; run `just image-session`"]
        async fn a_turn_cut_off_partway_can_still_be_picked_up() {
            let runtime = located_runtime();
            let (_state, handout) = handout_of();
            let name = "stageman-job-midturn-probe";
            made(&runtime, name, &handout).await;

            let cut_short = tokio::time::timeout(
                std::time::Duration::from_secs(6),
                talk(
                    &runtime,
                    name,
                    Opening::Fresh,
                    handout.kit(),
                    "Count from 1 to 40, one number per line, pausing two seconds between each.",
                ),
            )
            .await;
            assert!(cut_short.is_err(), "it should not have finished in time");

            let picked_up = talk(
                &runtime,
                name,
                Opening::Resumed,
                handout.kit(),
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
