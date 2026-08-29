//! The domain: what a project is, what a job is, and the states a job moves
//! through.
//!
//! No I/O, no async runtime, no platform and no framework. This crate names
//! nothing else in the workspace, which is what lets both of the crates above
//! it depend on it without depending on each other — see
//! `docs/architecture.md` §1 for the rule and why it is the one worth
//! defending.
//!
//! The vocabulary this crate exists to express is fixed in
//! `docs/conventions.md` §2, including the words it deliberately avoids.
//!
//! One thing here looks like plumbing and is not: deciding what an agent
//! process is handed. It lives in a crate with no I/O because it is a pure
//! function from configuration to a description of what that process should
//! see, and because it is the only thing standing between an operator and
//! silently paying the wrong way — which makes being able to test it without
//! spawning a process the whole point rather than a convenience. Delivering
//! that description is an adapter's job, and differs per agent.
//!
//! **Nothing here reads a clock, mints an identifier, or generates a nonce.**
//! All three are effects, and all three would make values non-deterministic to
//! construct, so all three are supplied by the caller — which is why creating a
//! job takes a timestamp rather than asking the operating system for one. The
//! crates that are allowed effects do that; this one stays a set of values a
//! test can build exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key as CipherKey, Nonce as CipherNonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

// Re-exported because both appear in this crate's public signatures: a caller
// building a job needs a timestamp and an identifier, and making it depend on
// these crates separately would make matching their versions its problem.
pub use jiff::Timestamp;
pub use uuid::Uuid;

/// Bytes of the nonce a single sealing operation consumes.
pub const NONCE_LEN: usize = 12;

/// A nonce: unique per sealing operation, never reused under one key.
pub type Nonce = [u8; NONCE_LEN];

/// A credential, in memory.
///
/// Formatting is redacted in both `Debug` and `Display`, because the usual way
/// a token reaches a log is a structure printed whole while somebody is
/// debugging something else entirely.
///
/// **It deliberately does not implement serialisation.** State is persisted by
/// converting it to a separate sealed form, and a `Serialize` here would let
/// this type reach a file in the clear by accident. The bar is in
/// `docs/conventions.md` §4.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a credential.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Yields the credential in the clear.
    ///
    /// Named for what it does rather than for what it returns, so that every
    /// call site reads as a decision someone made instead of an accessor
    /// nobody noticed.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Identifies a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProjectId(Uuid);

/// Identifies a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct JobId(Uuid);

macro_rules! identifier {
    ($name:ident) => {
        impl $name {
            /// Wraps an identifier minted elsewhere.
            ///
            /// There is no constructor that generates one: doing so needs
            /// randomness, and this crate takes no effects. See the crate
            /// documentation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Borrows the underlying identifier.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

identifier!(ProjectId);
identifier!(JobId);

/// A coding agent this project knows how to run.
///
/// A closed set rather than a list an operator can extend, because every agent
/// needs an adapter and an image, both of which are code — so the set of
/// supportable agents was always bounded by what is compiled in, and a value
/// an operator could invent only postponed the failure to runtime. Adding one
/// is a compile error everywhere it is not yet handled, which is the point.
///
/// Naming the set here is not the same as being specific to one, which
/// `docs/decisions/0006-agents-are-pluggable.md` forbids outside an adapter:
/// this crate knows *which* agents exist, and adapters know how each behaves.
///
/// A job stores this **by value**, never as a reference into configuration, so
/// that removing an agent's configuration cannot rewrite the history of jobs
/// that used it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Agent {
    /// Anthropic's coding agent.
    Claude,
}

impl Agent {
    /// Every agent that exists.
    ///
    /// The one place adding a variant has to be updated by hand, and worth
    /// being explicit about why that is tolerable: every *behavioural* site is
    /// a match and so fails to compile until the new agent is handled, which is
    /// the property the closed set exists for. A list is not a match, so
    /// forgetting this one costs a missing menu entry rather than a wrong
    /// answer — the cheapest failure of the set, and the only one available
    /// without a derive this crate would otherwise have no use for.
    pub const ALL: &'static [Self] = &[Self::Claude];

    /// What this agent is good for, in prose.
    ///
    /// Not decoration and not operator-editable: the orchestrator chooses which
    /// agent runs a job, and this is what it reasons over — see
    /// `docs/decisions/0006-agents-are-pluggable.md`. It lives in code because
    /// it describes the agent rather than the installation.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Claude => {
                "General-purpose coding agent. Reads a repository, makes changes \
                 across files, runs commands, and explains what it did."
            }
        }
    }
}

/// What an operator supplies in order to run an agent.
///
/// A credential and nothing else. There is deliberately no path here: agents
/// run in containers built with them already installed, so where the program
/// lives is decided by an image rather than by the machine this happens to run
/// on — see `docs/decisions/0012-agents-run-in-containers.md`.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// What the agent authenticates with.
    ///
    /// One credential per agent, never one per role — the orchestrator and a
    /// job running the same agent use the same one. See
    /// `docs/decisions/0008-one-credential-per-agent.md`.
    pub auth_token: Secret,
}

/// A platform a project's jobs act on.
///
/// One variant for now, which is the one a job cannot work without: cloning the
/// repository, pushing a branch and opening a pull request are all the same
/// credential. See
/// `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` for why a
/// job holds these at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Platform {
    /// The repository host.
    GitHub,
}

/// Somewhere the orchestrator watches and a job can speak into.
///
/// Two-directional by definition, which is the whole reason this is not a
/// variant of [`Platform`] — see
/// `docs/decisions/0027-a-channel-is-not-a-platform.md`. A platform is
/// something a job *acts on*; a channel is where a conversation happens, and
/// the orchestrator is on it as much as a job is.
///
/// A closed set for the same reason [`Agent`] is: reaching one needs code, and
/// code is not something an operator supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Channel {
    /// The team chat, and the escalation path — see
    /// `docs/decisions/0005-conversation-happens-on-channels.md`.
    Slack,
}

/// What an operator supplies in order to bind one channel to a project.
///
/// Two values rather than one, which is the second half of why channels do not
/// share the platform map: an address has nowhere to live in a map of bare
/// credentials.
///
/// Deriving `Debug` is safe and deliberate. The credential redacts itself and
/// the address is not one — it is a public identifier a person could read off
/// the chat client — so there is nothing here for a hand-written formatter to
/// hide that the field types do not already.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// Where on that channel this project's conversation happens.
    ///
    /// For Slack, the identifier of the channel a project's threads hang from
    /// — one channel per project, one thread per job.
    ///
    /// Called an address rather than a *destination* because
    /// `docs/conventions.md` §2 rejects one-directional words for this concept:
    /// the orchestrator watches this exact place, and a job posts to it.
    pub address: String,
    /// What the channel is reached with.
    ///
    /// Belongs to the project rather than the instance, for the reason
    /// `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md` gives:
    /// watching a project's channels needs that project's credentials, and one
    /// holder of every project's at once is the shape being avoided.
    pub credential: Secret,
}

/// One agent, in one workspace, working on one project.
///
/// A job happens once and there is no retry: a second attempt is a new job with
/// its own workspace, which is why nothing here records an attempt count. It
/// may outlive the process supervising it, and resuming is not retrying — the
/// same job carries on, which is the distinction
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` turns on.
///
/// Nothing here names the container it runs in. The name is derived from the
/// job's identifier, so there is no moment at which a container exists and the
/// value naming it has not been written down — a field would have that gap, and
/// a container nothing can name is the one leak 0015 has to prevent.
///
/// It holds no credential, which is why it crosses the snapshot boundary
/// unchanged while the types around it need a sealed counterpart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Which agent ran it.
    ///
    /// Stored by value, so this stays true after an operator removes that
    /// agent's configuration. Recorded at all because once more than one agent
    /// can run a job, "why did this go badly?" has no answer without it.
    pub agent: Agent,
    /// Why the orchestrator started it, in prose.
    ///
    /// The whole of a job's provenance, deliberately — see
    /// `docs/architecture.md` §2 on why the structured version is absent.
    pub reason: String,
    /// The instruction the agent begins from.
    ///
    /// Self-contained by necessity: an agent in a fresh workspace knows nothing
    /// about where it came from, so this carries the repository, the work, and
    /// the constraint that it proposes rather than merges.
    pub kickoff: String,
    /// When the record was created.
    ///
    /// Named for the record rather than for the work, so it stays true once
    /// there is a gap between a job existing and an agent starting.
    pub created_at: Timestamp,
    /// Where it has got to.
    pub progress: Progress,
}

/// Where a job has got to.
///
/// The states `docs/architecture.md` §1 says this crate holds. Deliberately
/// three and not more: they are what somebody has to *act* on, and a state
/// nobody acts on differently is a label rather than a state.
///
/// Note what is absent. There is no *interrupted*, although a job's container
/// is stopped every time the daemon dies. That is a fact about the runtime and
/// not about the work: the job is still running, and startup's job is to make
/// the containers match rather than to move the job somewhere new — see
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, where resuming is
/// the same job continuing. A state for it would have to be left behind on
/// every resume, and the first crash between the two would strand a job in a
/// state nothing clears.
///
/// There is also no `Default`. A job is created running, so the value is never
/// in doubt at the one moment a default would be consulted — and a job whose
/// progress was filled in by nobody is exactly the kind of record that later
/// reads as fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Progress {
    /// Started, and nothing has said it is over.
    ///
    /// What a sweep looks for: a running job whose container is stopped is one
    /// to restart, and a container whose job is not running is not.
    Running,
    /// The agent finished its turn.
    ///
    /// Says the work ended, not that it succeeded — nothing here can see
    /// whether a proposal was any good, and
    /// `docs/decisions/0002-never-merge-never-deploy.md` means a human reads it
    /// before it counts for anything. Claiming more would be a claim this
    /// system is not in a position to make.
    Completed,
    /// It could not be finished, and this is what went wrong.
    ///
    /// Prose for a person reading the dashboard, like `reason` — not a code to
    /// branch on. What a job that fails ought to do beyond this is
    /// `docs/open-questions.md`'s question about credentials expiring, wearing
    /// a different hat.
    Failed(String),
}

/// A repository this instance watches, and everything belonging to it.
#[derive(Debug, Clone)]
pub struct Project {
    /// What to call it in the dashboard.
    pub name: String,
    /// Where the repository lives.
    ///
    /// Every kickoff embeds this, because an agent has no other way to find it.
    pub repository: String,
    /// The agent this project's orchestrator thinks with.
    ///
    /// Per project rather than per instance, because watching a project's
    /// channels needs that project's credentials and a shared orchestrator
    /// would hold every project's at once — see
    /// `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`.
    pub orchestrator_agent: Agent,
    /// The agents this project's jobs may run on.
    ///
    /// Never empty in a valid instance, and checked rather than made
    /// unrepresentable — see [`State::check`], which is the one definition of
    /// what valid means and is consulted wherever a state could have stopped
    /// being it.
    pub job_agents: BTreeSet<Agent>,
    /// What its jobs are handed, one credential per platform.
    ///
    /// A map rather than a list so that two credentials for one platform is
    /// unrepresentable, and ordered rather than hashed so the snapshot does not
    /// reshuffle itself between writes.
    pub credentials: BTreeMap<Platform, Secret>,
    /// Where its conversations happen, one binding per channel.
    ///
    /// Separate from `credentials` above rather than folded into it — see
    /// `docs/decisions/0027-a-channel-is-not-a-platform.md`, which is mostly an
    /// argument about the two lines this splits into.
    ///
    /// **May be empty, and that is a working project rather than a broken
    /// one.** [`State::check`] deliberately does not require a binding:
    /// `docs/decisions/0005-conversation-happens-on-channels.md` says a project
    /// with no conversational channel can still run work that never needs to
    /// ask, and refusing to start one would make the escalation path a
    /// prerequisite for work that never escalates.
    pub channels: BTreeMap<Channel, ChannelConfig>,
    /// Its jobs, past and present.
    ///
    /// Nested rather than held globally so that "a job belongs to exactly one
    /// project" is structural instead of a field somebody has to keep true.
    pub jobs: BTreeMap<JobId, Job>,
}

/// Everything one instance knows.
///
/// The whole of what gets snapshotted and the whole of what is loaded back —
/// see `docs/decisions/0011-state-is-a-snapshot-not-a-database.md`.
///
/// There is deliberately no `Default`. An instance with nothing to think with
/// is not a state worth representing, so one is either loaded from a snapshot
/// or built by the first-run flow, and never conjured empty — see
/// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// The agents this instance can run, and what each authenticates with.
    ///
    /// May be empty. An instance with no projects needs nothing to think with,
    /// which is what lets one start with nothing configured at all — see
    /// `docs/decisions/0021-an-instance-starts-empty.md`.
    ///
    /// An agent may not be removed while a project names it. Nothing here
    /// prevents that directly, and three things catch it: [`State::used_by`]
    /// is the query a caller consults first, sealing refuses a state that has
    /// broken the rule, and opening refuses a file that has.
    pub agents: BTreeMap<Agent, AgentConfig>,
    /// The projects it watches.
    pub projects: BTreeMap<ProjectId, Project>,
}

// Deliberately absent: where the container runtime lives. It was a field here
// until `docs/decisions/0023-the-container-runtime-is-discovered-once.md`
// replaced it with discovery, and the reason it does not come back is that it
// describes the *machine* rather than the work. A snapshot is meant to be
// portable — copy the file, carry it to another machine, supply the key — and
// a recorded absolute path is the one thing in it that a different machine
// makes wrong.

impl State {
    /// Which projects depend on an agent's configuration.
    ///
    /// The query to consult before removing one. Empty means the agent can go;
    /// anything else names what would break, which is what a dashboard needs in
    /// order to say *why* rather than merely refusing.
    ///
    /// A project's *past* jobs are not considered and must not be: a job stores
    /// its agent by value precisely so that removing a configuration cannot
    /// rewrite the record of work already done — `docs/conventions.md` §2.
    ///
    /// Skipped by mutation testing, and equivalent rather than untested:
    /// [`Agent`] has one member and a project's job agents are never empty, so
    /// both sides of this condition are true for every project. Inverting the
    /// comparison or replacing the `or` with an `and` changes nothing any test
    /// could observe. **Delete this attribute in the commit that adds a second
    /// agent** — a project naming one agent for triage and another for its jobs
    /// is what makes this falsifiable, and it is the first thing that will
    /// exist once there are two.
    #[mutants::skip]
    pub fn used_by(&self, agent: Agent) -> impl Iterator<Item = ProjectId> + '_ {
        self.projects
            .iter()
            .filter(move |(_, project)| {
                project.orchestrator_agent == agent || project.job_agents.contains(&agent)
            })
            .map(|(id, _)| *id)
    }

    /// Whether this describes an instance that can exist.
    ///
    /// The invariant `docs/decisions/0021-an-instance-starts-empty.md` moved
    /// down from the instance to the project: a project names one agent for its
    /// orchestrator and at least one its jobs may run on, and every one of them
    /// is configured.
    ///
    /// Checked rather than made unrepresentable. A type that could not hold an
    /// empty set would enforce half of this and leave the other half — an agent
    /// removed while a project still named it — needing a check anyway, so the
    /// wrapper bought ceremony at every construction site in exchange for one
    /// of two conditions. One function, asked wherever a state might have
    /// stopped being valid, is the smaller thing.
    ///
    /// It says nothing about *when* to ask. A file is checked as it is read,
    /// because it is untrusted input; a state is checked before it is written,
    /// because a file that will not open is worse than a write that refused.
    /// Neither belongs to the domain, so neither is decided here.
    ///
    /// # Errors
    ///
    /// Names the first project that is wrong and how, because an operator
    /// repairing a file needs to know which one.
    pub fn check(&self) -> Result<(), Inconsistent> {
        for (id, project) in &self.projects {
            if project.job_agents.is_empty() {
                return Err(Inconsistent::NoJobAgents(*id));
            }
            for named in std::iter::once(project.orchestrator_agent)
                .chain(project.job_agents.iter().copied())
            {
                if !self.agents.contains_key(&named) {
                    return Err(Inconsistent::UnconfiguredProjectAgent {
                        project: *id,
                        agent: named,
                    });
                }
            }
        }
        Ok(())
    }

    /// Every job this instance believes is still running.
    ///
    /// What a sweep puts back to work after the process supervising them died.
    /// Believed rather than observed: this says what the instance recorded, and
    /// whether a container is actually up is a question for the runtime — see
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, where
    /// reconciling the two is startup's job rather than a state of its own.
    pub fn running(&self) -> impl Iterator<Item = JobId> + '_ {
        self.projects.values().flat_map(|project| {
            project
                .jobs
                .iter()
                .filter(|(_, job)| job.progress == Progress::Running)
                .map(|(id, _)| *id)
        })
    }

    /// A job, whichever project it belongs to.
    ///
    /// Jobs are keyed inside their project, because a job belongs to exactly
    /// one and `docs/architecture.md` §2 leans on that. A sweep works from a
    /// container's name, which says the job and not the project, so it needs
    /// the search this does.
    #[must_use]
    pub fn job(&self, job: JobId) -> Option<&Job> {
        self.projects
            .values()
            .find_map(|project| project.jobs.get(&job))
    }

    /// A job, for recording what became of it.
    pub fn job_mut(&mut self, job: JobId) -> Option<&mut Job> {
        self.projects
            .values_mut()
            .find_map(|project| project.jobs.get_mut(&job))
    }

    /// Converts to the form that goes on disk, sealing every credential.
    ///
    /// Takes a source of nonces rather than generating them, because
    /// randomness is an effect and this crate takes none. A **fresh** nonce is
    /// consumed per credential per write, always: with this cipher, reusing one
    /// under the same key leaks the authentication key rather than merely one
    /// plaintext, so there is no such thing as a cheap reuse.
    ///
    /// A consequence worth knowing: because sealing happens on the way out,
    /// every write produces different ciphertext even when no credential
    /// changed. The values are opaque anyway, so what
    /// `docs/decisions/0011-state-is-a-snapshot-not-a-database.md` wanted from
    /// a readable file survives — but two snapshots are never byte-identical.
    ///
    /// # Errors
    ///
    /// Fails only if the cipher rejects an input, which for a well-formed key
    /// and nonce does not happen in practice.
    pub fn seal(
        &self,
        key: &Key,
        nonces: &mut impl FnMut() -> Nonce,
    ) -> Result<Snapshot, SealError> {
        let agents = self
            .agents
            .iter()
            .map(|(agent, config)| {
                Ok((
                    *agent,
                    SealedAgentConfig {
                        auth_token: config.auth_token.seal(key, nonces())?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SealError>>()?;

        let projects = self
            .projects
            .iter()
            .map(|(id, project)| {
                let credentials = project
                    .credentials
                    .iter()
                    .map(|(platform, secret)| Ok((*platform, secret.seal(key, nonces())?)))
                    .collect::<Result<BTreeMap<_, _>, SealError>>()?;
                let channels = project
                    .channels
                    .iter()
                    .map(|(channel, config)| {
                        Ok((
                            *channel,
                            SealedChannelConfig {
                                address: config.address.clone(),
                                credential: config.credential.seal(key, nonces())?,
                            },
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, SealError>>()?;
                Ok((
                    *id,
                    SealedProject {
                        name: project.name.clone(),
                        repository: project.repository.clone(),
                        orchestrator_agent: project.orchestrator_agent,
                        job_agents: project.job_agents.clone(),
                        credentials,
                        channels,
                        jobs: project.jobs.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SealError>>()?;

        Ok(Snapshot { agents, projects })
    }
}

/// The key a snapshot's credentials are sealed under.
///
/// Supplied by the environment at startup, and never stored beside the file it
/// protects — that is what makes the file portable and useless on its own.
/// Redacts when formatted, for exactly the reason a credential does.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl Key {
    /// Wraps key material.
    #[must_use]
    pub const fn new(material: [u8; 32]) -> Self {
        Self(material)
    }

    /// Parses key material supplied as base64.
    ///
    /// Parsing is pure, so it lives with the type rather than with whatever
    /// reads the environment — which keeps the one place that knows what a key
    /// looks like from being the same place that knows where it comes from.
    ///
    /// # Errors
    ///
    /// Fails if the text is not base64, or does not decode to exactly the
    /// right number of bytes. Neither message repeats the input.
    pub fn from_base64(text: &str) -> Result<Self, KeyError> {
        let decoded = BASE64.decode(text).map_err(|_| KeyError::Encoding)?;
        let material: [u8; 32] = decoded.try_into().map_err(|_| KeyError::Length)?;
        Ok(Self(material))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&CipherKey::<Aes256Gcm>::from(self.0))
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(<redacted>)")
    }
}

/// Key material could not be read.
///
/// Deliberately says nothing about the input, since a malformed key is often a
/// nearly-correct one and an error message is a place secrets escape.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// The text is not valid base64.
    #[error("the key is not valid base64")]
    Encoding,
    /// The text decoded to the wrong number of bytes.
    #[error("the key must decode to exactly 32 bytes")]
    Length,
}

/// A credential could not be sealed.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// The cipher rejected the input.
    #[error("a credential could not be sealed")]
    Cipher,
}

/// An instance's state is not internally consistent.
///
/// One type for every way a state can be wrong, because there is one definition
/// of valid and several places that need to ask: a file on the way in, a state
/// on the way out, and every operation that could break it. Two definitions
/// would eventually disagree, and the one that let something through would be
/// the one nobody was reading.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Inconsistent {
    /// A project has no agent its jobs can run on.
    ///
    /// A project whose jobs cannot run cannot do the one thing a project is
    /// for, which is why this is a broken instance rather than an unusual one.
    #[error("project {0} has no agent its jobs can run on")]
    NoJobAgents(ProjectId),
    /// A project names an agent this instance does not configure.
    #[error("project {project} names agent {agent:?}, which is not configured")]
    UnconfiguredProjectAgent {
        /// The project holding the dangling reference.
        project: ProjectId,
        /// The agent it names.
        agent: Agent,
    },
}

/// A snapshot could not be turned back into state.
///
/// Every variant is deliberately vague about *which* credential, and says
/// nothing about its contents: an error message is a place secrets escape.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// A stored credential is not valid base64.
    #[error("a stored credential is not valid base64")]
    Encoding,
    /// A stored nonce is the wrong length.
    #[error("a stored nonce is the wrong length")]
    NonceLength,
    /// Decryption failed.
    #[error("a credential could not be decrypted: wrong key, or the file was altered")]
    Cipher,
    /// A credential decrypted to something that is not text.
    #[error("a credential decrypted to bytes that are not text")]
    NotText,
    /// The snapshot decrypted, and describes an instance that cannot be.
    ///
    /// A file is untrusted input — hand-edited, half-written, or written by an
    /// older version — so this is where believing it stops.
    #[error("the snapshot describes an instance that is not internally consistent")]
    Inconsistent(#[source] Inconsistent),
}

/// A credential as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecret {
    /// The nonce this was sealed with, base64.
    pub nonce: String,
    /// The ciphertext and its authentication tag, base64.
    pub ciphertext: String,
}

impl Secret {
    /// Seals this credential for storage.
    ///
    /// # Errors
    ///
    /// Fails only if the cipher rejects the input.
    pub fn seal(&self, key: &Key, nonce: Nonce) -> Result<SealedSecret, SealError> {
        let ciphertext = key
            .cipher()
            .encrypt(&CipherNonce::from(nonce), self.0.as_bytes())
            .map_err(|_| SealError::Cipher)?;
        Ok(SealedSecret {
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(ciphertext),
        })
    }
}

impl SealedSecret {
    /// Recovers the credential.
    ///
    /// # Errors
    ///
    /// Fails if the stored encoding is malformed, or if decryption fails —
    /// which means the key is wrong or the file was altered. The cipher
    /// authenticates, so a tampered snapshot is a failure rather than a
    /// plausible-looking wrong answer.
    pub fn open(&self, key: &Key) -> Result<Secret, OpenError> {
        let nonce = BASE64
            .decode(&self.nonce)
            .map_err(|_| OpenError::Encoding)?;
        let nonce: Nonce = nonce.try_into().map_err(|_| OpenError::NonceLength)?;
        let ciphertext = BASE64
            .decode(&self.ciphertext)
            .map_err(|_| OpenError::Encoding)?;
        let plaintext = key
            .cipher()
            .decrypt(&CipherNonce::from(nonce), ciphertext.as_slice())
            .map_err(|_| OpenError::Cipher)?;
        String::from_utf8(plaintext)
            .map(Secret::new)
            .map_err(|_| OpenError::NotText)
    }
}

/// An agent's configuration as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedAgentConfig {
    /// The sealed credential.
    pub auth_token: SealedSecret,
}

/// A channel binding as it appears on disk.
///
/// The address travels in the clear beside its sealed credential, because it is
/// not a secret and sealing it would cost a nonce per write to hide a value the
/// chat client shows anybody in the room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedChannelConfig {
    /// Where on that channel this project's conversation happens.
    pub address: String,
    /// The sealed credential.
    pub credential: SealedSecret,
}

/// A project as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedProject {
    /// What to call it.
    pub name: String,
    /// Where the repository lives.
    pub repository: String,
    /// The agent its orchestrator thinks with.
    pub orchestrator_agent: Agent,
    /// The agents its jobs may run on.
    pub job_agents: BTreeSet<Agent>,
    /// Its sealed credentials.
    pub credentials: BTreeMap<Platform, SealedSecret>,
    /// Its channel bindings, each with its credential sealed.
    pub channels: BTreeMap<Channel, SealedChannelConfig>,
    /// Its jobs, which hold nothing needing sealing.
    pub jobs: BTreeMap<JobId, Job>,
}

/// Everything one instance knows, as it appears on disk.
///
/// A separate type from [`State`] rather than the same one behind a flag,
/// because the boundary between them does real work: a file is untrusted input
/// — hand-edited, half-written, or written by an older version — and turning
/// one into state is the moment to find out whether it can be believed. What
/// comes out the far side has already been checked, so nothing downstream
/// handles a reference that does not resolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The configured agents, sealed.
    pub agents: BTreeMap<Agent, SealedAgentConfig>,
    /// The projects, sealed.
    pub projects: BTreeMap<ProjectId, SealedProject>,
}

impl Snapshot {
    /// Decrypts and validates, yielding state that can be relied on.
    ///
    /// # Errors
    ///
    /// Fails if any credential cannot be recovered, or if the snapshot is
    /// internally inconsistent — currently, if the agent it names as the
    /// orchestrator's has no configuration. That check is what lets every
    /// later caller look that agent up without handling an absence.
    pub fn open(self, key: &Key) -> Result<State, OpenError> {
        let Self { agents, projects } = self;

        let agents = agents
            .into_iter()
            .map(|(agent, config)| {
                Ok((
                    agent,
                    AgentConfig {
                        auth_token: config.auth_token.open(key)?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;

        let projects = projects
            .into_iter()
            .map(|(id, project)| {
                let credentials = project
                    .credentials
                    .into_iter()
                    .map(|(platform, sealed)| Ok((platform, sealed.open(key)?)))
                    .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
                let channels = project
                    .channels
                    .into_iter()
                    .map(|(channel, sealed)| {
                        Ok((
                            channel,
                            ChannelConfig {
                                address: sealed.address,
                                credential: sealed.credential.open(key)?,
                            },
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
                Ok((
                    id,
                    Project {
                        name: project.name,
                        repository: project.repository,
                        orchestrator_agent: project.orchestrator_agent,
                        job_agents: project.job_agents,
                        credentials,
                        channels,
                        jobs: project.jobs,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;

        let state = State { agents, projects };
        // A file is untrusted input, so this is where believing it stops.
        state.check().map_err(OpenError::Inconsistent)?;
        Ok(state)
    }
}

/// Exactly what one agent process is allowed to see, and nothing more.
///
/// The piece of logic `docs/architecture.md` §1 says looks like plumbing and
/// is not. It lives in a crate with no I/O because it is a pure function from
/// configuration to a description: *deciding* is here, *delivering* belongs to
/// an adapter and differs per agent — a variable for one, a file at an
/// expected path for another.
///
/// Being able to test it without spawning a process is the whole point rather
/// than a convenience. At least one agent resolves credentials by precedence
/// and prefers a per-token key when it finds one, so a variable inherited from
/// whatever shell started the daemon silently changes who pays: no error, no
/// log line, and no way to notice before the invoice arrives. See
/// `docs/decisions/0008-one-credential-per-agent.md`, and
/// `docs/conventions.md` §3 for the rule this exists to keep.
///
/// **Nothing here is inherited.** A handout is built by selecting from one
/// project, which is how the invariant in `docs/architecture.md` §2 — a job
/// holds credentials for its own project and no other — holds by construction
/// rather than by review.
///
/// It carries credentials, so like [`Secret`] it redacts when formatted and
/// deliberately implements no serialisation: a handout is what a process is
/// about to be handed, never state, and nothing should be able to write one to
/// disk.
#[derive(Clone)]
pub struct Handout {
    agent: Agent,
    agent_credential: Secret,
    platforms: BTreeMap<Platform, Secret>,
    channels: BTreeMap<Channel, ChannelConfig>,
}

impl Handout {
    /// What the agent a project's orchestrator thinks with is handed.
    ///
    /// Its own credential, and no platform credential at all: triage judges
    /// signals rather than acting on them, so it has no repository to reach and
    /// nothing to authenticate against — see
    /// `docs/decisions/0012-agents-run-in-containers.md`.
    ///
    /// It does get the project's channel bindings, and that asymmetry is the
    /// point rather than an inconsistency. Watching a project's channels is the
    /// whole of what an orchestrator does — `docs/architecture.md` §1 — and
    /// answering on one is a reaction it is allowed to take, so an orchestrator
    /// that cannot reach a channel cannot do its job. A single map for both
    /// kinds could not express this, which is the argument
    /// `docs/decisions/0027-a-channel-is-not-a-platform.md` turns on.
    ///
    /// Per project rather than per instance, because the channels it watches
    /// belong to a project and a shared orchestrator would hold every
    /// project's credentials at once —
    /// `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`.
    ///
    /// # Errors
    ///
    /// Fails if the project is not one this instance watches, or if its
    /// orchestrator's agent has no configuration — which the invariant in
    /// `docs/decisions/0021-an-instance-starts-empty.md` says cannot happen,
    /// since it holds at construction and is checked again on the way in and
    /// out of a snapshot.
    ///
    /// The signature admits it anyway, and deliberately: the alternative is a
    /// total function substituting an empty credential for a missing one, which
    /// turns a state that cannot occur into an authentication failure somewhere
    /// else entirely. `.quality/gate-reference.md` forbids exactly that trade.
    pub fn for_triage(state: &State, project: ProjectId) -> Result<Self, HandoutError> {
        let watching = state
            .projects
            .get(&project)
            .ok_or(HandoutError::UnknownProject(project))?;
        let agent = watching.orchestrator_agent;
        let config = state
            .agents
            .get(&agent)
            .ok_or(HandoutError::UnconfiguredAgent(agent))?;
        Ok(Self {
            agent,
            agent_credential: config.auth_token.clone(),
            platforms: BTreeMap::new(),
            channels: watching.channels.clone(),
        })
    }

    /// What a job's agent is handed: its own credential, plus the platform
    /// credentials and channel bindings of the one project the job belongs to.
    ///
    /// The channels are how a job speaks without a terminal, which
    /// `docs/architecture.md` §2 makes an invariant: a job that needs a human
    /// says so on a channel and stays alive.
    ///
    /// # Errors
    ///
    /// Fails if the project is not one this instance watches, or if the agent
    /// has no configuration. Both are refusals rather than empty handouts: a
    /// process started with nothing to authenticate with fails later, further
    /// from the cause, and `docs/conventions.md` §3 would rather that be a
    /// visible job failure than a mystery.
    pub fn for_job(state: &State, agent: Agent, project: ProjectId) -> Result<Self, HandoutError> {
        let config = state
            .agents
            .get(&agent)
            .ok_or(HandoutError::UnconfiguredAgent(agent))?;
        let project = state
            .projects
            .get(&project)
            .ok_or(HandoutError::UnknownProject(project))?;
        Ok(Self {
            agent,
            agent_credential: config.auth_token.clone(),
            platforms: project.credentials.clone(),
            channels: project.channels.clone(),
        })
    }

    /// Which agent this was built for.
    ///
    /// Carried so that an adapter can refuse a handout meant for another
    /// agent. The invariant says nothing belonging to any other agent, and a
    /// value that cannot be checked defends nothing.
    #[must_use]
    pub const fn agent(&self) -> Agent {
        self.agent
    }

    /// What the agent authenticates with.
    #[must_use]
    pub const fn agent_credential(&self) -> &Secret {
        &self.agent_credential
    }

    /// What this job reaches one platform with, if it has anything for it.
    #[must_use]
    pub fn platform(&self, platform: Platform) -> Option<&Secret> {
        self.platforms.get(&platform)
    }

    /// Every platform credential in this handout.
    pub fn platforms(&self) -> impl Iterator<Item = (Platform, &Secret)> {
        self.platforms.iter().map(|(p, s)| (*p, s))
    }

    /// How this process reaches one channel, if it is bound to one.
    #[must_use]
    pub fn channel(&self, channel: Channel) -> Option<&ChannelConfig> {
        self.channels.get(&channel)
    }

    /// Every channel binding in this handout.
    pub fn channels(&self) -> impl Iterator<Item = (Channel, &ChannelConfig)> {
        self.channels.iter().map(|(c, config)| (*c, config))
    }
}

impl fmt::Debug for Handout {
    /// Names what is present and never its contents.
    ///
    /// Written out rather than derived, per `docs/conventions.md` §4. The
    /// fields redact themselves, so deriving would in fact be safe here — and
    /// that is exactly the reasoning which stops being true the first time
    /// somebody adds a `String` field, which is why the rule has no exception
    /// worth taking.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handout")
            .field("agent", &self.agent)
            .field("agent_credential", &"<redacted>")
            .field("platforms", &self.platforms.keys().collect::<Vec<_>>())
            .field("channels", &self.channels.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A handout could not be decided.
#[derive(Debug, thiserror::Error)]
pub enum HandoutError {
    /// The agent has no configuration in this instance.
    #[error("the agent {0:?} has no configuration in this instance")]
    UnconfiguredAgent(Agent),
    /// The project is not one this instance watches.
    #[error("no project {0} in this instance")]
    UnknownProject(ProjectId),
}

#[cfg(test)]
mod tests {
    use super::{
        Agent, AgentConfig, BASE64, Channel, ChannelConfig, Handout, HandoutError, Inconsistent,
        Job, JobId, Key, NONCE_LEN, Nonce, OpenError, Platform, Progress, Project, ProjectId,
        Secret, Snapshot, State,
    };
    use base64::Engine as _;
    use jiff::Timestamp;
    use std::collections::{BTreeMap, BTreeSet};
    use uuid::Uuid;

    const TOKEN: &str = "ghp-not-a-real-token";
    /// Distinct from `TOKEN`, so a test that finds a credential where it should
    /// not can say which map it escaped from.
    const CHANNEL_TOKEN: &str = "xoxb-not-a-real-token";
    /// Not a secret, and asserted to travel in the clear.
    const CHANNEL_ADDRESS: &str = "C0123456789";
    /// A second project's channel credential, so that a handout carrying the
    /// wrong one is detectable rather than merely non-empty.
    const ALIEN_CHANNEL_TOKEN: &str = "xoxb-belongs-to-somebody-else";

    /// An instance with an agent configured and nothing else.
    fn configured() -> State {
        State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("agent-token".to_owned()),
                },
            )]),
            ..State::default()
        }
    }

    /// The agents a project's jobs may run on, for a project that only has one.
    fn only_claude() -> BTreeSet<Agent> {
        BTreeSet::from([Agent::Claude])
    }

    fn a_project_with_a_job() -> Project {
        let mut credentials = BTreeMap::new();
        credentials.insert(Platform::GitHub, Secret::new(TOKEN.to_owned()));
        let channels = BTreeMap::from([(Channel::Slack, a_slack_binding())]);
        let mut jobs = BTreeMap::new();
        jobs.insert(
            JobId::from_uuid(Uuid::from_u128(9)),
            Job {
                agent: Agent::Claude,
                reason: "an issue was opened".to_owned(),
                kickoff: "work on it".to_owned(),
                created_at: Timestamp::UNIX_EPOCH,
                progress: Progress::Running,
            },
        );
        Project {
            name: "example".to_owned(),
            repository: "https://example.invalid/repo".to_owned(),
            orchestrator_agent: Agent::Claude,
            job_agents: only_claude(),
            credentials,
            channels,
            jobs,
        }
    }

    fn a_slack_binding() -> ChannelConfig {
        ChannelConfig {
            address: CHANNEL_ADDRESS.to_owned(),
            credential: Secret::new(CHANNEL_TOKEN.to_owned()),
        }
    }

    #[test]
    fn debug_does_not_leak_a_secret() {
        let secret = Secret::new(TOKEN.to_owned());
        assert!(!format!("{secret:?}").contains(TOKEN));
    }

    #[test]
    fn display_does_not_leak_a_secret() {
        let secret = Secret::new(TOKEN.to_owned());
        assert!(!format!("{secret}").contains(TOKEN));
    }

    #[test]
    fn a_secret_nested_in_a_structure_does_not_leak() {
        // The failure this guards is not formatting a secret directly — nobody
        // does that. It is printing whatever happens to contain one.
        //
        // A project now nests two kinds, and the second is the one a derive
        // could get wrong: `ChannelConfig` holds a `String` beside its
        // credential, so this is what says the derive on it is still safe.
        let shown = format!("{:?}", a_project_with_a_job());
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains(CHANNEL_TOKEN), "{shown}");
    }

    #[test]
    fn a_secret_still_yields_its_value_when_asked() {
        assert_eq!(Secret::new(TOKEN.to_owned()).expose(), TOKEN);
    }

    /// The state an instance is in before anybody has configured anything,
    /// which `docs/decisions/0021-an-instance-starts-empty.md` made valid
    /// again.
    #[test]
    fn a_fresh_instance_has_nothing_and_is_still_a_state() {
        let empty = State::default();

        assert!(empty.agents.is_empty());
        assert!(empty.projects.is_empty());
    }

    #[test]
    fn a_project_names_the_agent_its_orchestrator_thinks_with() {
        let state = populated();
        let project = state.projects.values().next().expect("one project");

        assert!(state.agents.contains_key(&project.orchestrator_agent));
        assert!(project.job_agents.contains(&Agent::Claude));
    }

    /// The rule a dashboard has to enforce, and the query it enforces it with.
    #[test]
    fn an_agent_a_project_names_reports_which_projects_would_break() {
        let state = populated();
        let depending: Vec<ProjectId> = state.used_by(Agent::Claude).collect();

        assert_eq!(depending.len(), 1, "{depending:?}");
        assert_eq!(configured().used_by(Agent::Claude).count(), 0);
    }

    /// One definition of valid, asked directly. Everything that persists or
    /// loads a state consults this rather than repeating the rule.
    #[test]
    fn a_state_that_lost_an_agent_a_project_names_is_not_consistent() {
        let mut state = populated();
        state.agents.clear();

        assert!(matches!(
            state.check(),
            Err(Inconsistent::UnconfiguredProjectAgent { .. })
        ));
    }

    #[test]
    fn a_project_with_no_agent_for_its_jobs_is_not_consistent() {
        let mut state = populated();
        for project in state.projects.values_mut() {
            project.job_agents.clear();
        }

        assert!(matches!(state.check(), Err(Inconsistent::NoJobAgents(_))));
    }

    /// A project with nowhere to escalate is a working project.
    ///
    /// `docs/decisions/0005-conversation-happens-on-channels.md` says it can
    /// still run work that never needs to ask, so requiring a binding would
    /// make the escalation path a prerequisite for work that never escalates.
    /// Asserted because the natural next change to [`State::check`] is to
    /// demand one.
    #[test]
    fn a_project_with_no_channel_bound_is_still_consistent() {
        let mut state = populated();
        for project in state.projects.values_mut() {
            project.channels.clear();
        }

        assert_eq!(state.check(), Ok(()));
    }

    #[test]
    fn an_instance_with_nothing_in_it_is_consistent() {
        assert_eq!(State::default().check(), Ok(()));
        assert_eq!(populated().check(), Ok(()));
    }

    #[test]
    fn a_jobs_agent_survives_that_agent_being_deconfigured() {
        // The reason a job stores an agent by value rather than by reference:
        // removing configuration is ordinary housekeeping, and it must not
        // rewrite the record of work already done.
        let mut state = configured();
        let project = a_project_with_a_job();
        state
            .projects
            .insert(ProjectId::from_uuid(Uuid::from_u128(3)), project);
        state.agents.remove(&Agent::Claude);

        let still_recorded = state
            .projects
            .values()
            .flat_map(|project| project.jobs.values())
            .all(|job| job.agent == Agent::Claude);
        assert!(still_recorded);
    }

    fn key() -> Key {
        Key::new([7; 32])
    }

    /// Deterministic stand-in for the randomness a caller normally supplies.
    ///
    /// A range rather than a counter, so the arithmetic happens inside the
    /// iterator instead of in code that would then need a suppression. Real
    /// nonces come from the operating system; all a test needs is that
    /// successive ones differ.
    fn counting_nonces() -> impl FnMut() -> Nonce {
        let mut supply = (0_u8..u8::MAX).map(|byte| [byte; NONCE_LEN]);
        move || supply.next().unwrap_or([u8::MAX; NONCE_LEN])
    }

    fn populated() -> State {
        let mut state = configured();
        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(3)),
            a_project_with_a_job(),
        );
        state
    }

    fn sealed() -> Snapshot {
        populated()
            .seal(&key(), &mut counting_nonces())
            .expect("sealing cannot fail for a well-formed key and nonce")
    }

    #[test]
    fn a_sealed_snapshot_carries_no_plaintext_credential() {
        // The whole point of the exercise. If this ever fails, every token this
        // instance holds is sitting in a file in the clear.
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");
        assert!(!json.contains(TOKEN));
        assert!(!json.contains("agent-token"));
        assert!(!json.contains(CHANNEL_TOKEN), "{json}");
    }

    /// The other half of the sealing decision, asserted rather than assumed.
    ///
    /// A channel's address is not a credential — it is an identifier the chat
    /// client shows everybody in the room — so it is written in the clear, and
    /// sealing it would spend a nonce per write to hide nothing. Worth a test
    /// because the cautious-looking change is to seal it, and nothing else
    /// would object.
    #[test]
    fn a_channels_address_is_not_sealed() {
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");
        assert!(json.contains(CHANNEL_ADDRESS), "{json}");
    }

    /// A snapshot says nothing about the machine it was written on.
    ///
    /// This replaces a test asserting the opposite — that a snapshot remembered
    /// where the container runtime lived. That field is gone, per
    /// `docs/decisions/0023-the-container-runtime-is-discovered-once.md`, and
    /// what it protected is worth keeping as a property: the file is meant to
    /// be copied to another machine, so anything machine-specific in it is
    /// wrong there. Asserted against the serialised form rather than the type,
    /// because a field added back would compile perfectly and only show up
    /// here.
    #[test]
    fn a_snapshot_holds_nothing_that_belongs_to_one_machine() {
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");

        assert!(!json.contains("runtime"), "{json}");
        assert!(!json.contains("/usr/"), "{json}");
    }

    #[test]
    fn a_snapshot_round_trips_through_json_and_back_into_state() {
        // Also proves the map keys survive: JSON object keys must be strings,
        // so an enum or an identifier used as one has to serialise as text.
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");
        let parsed: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let state = parsed.open(&key()).expect("and opens with the right key");
        let project = state
            .projects
            .values()
            .next()
            .expect("the project survived");
        assert_eq!(
            project
                .credentials
                .get(&Platform::GitHub)
                .map(Secret::expose),
            Some(TOKEN)
        );
        let bound = project
            .channels
            .get(&Channel::Slack)
            .expect("the binding survived");
        assert_eq!(bound.address, CHANNEL_ADDRESS);
        assert_eq!(bound.credential.expose(), CHANNEL_TOKEN);
    }

    #[test]
    fn each_credential_is_sealed_with_its_own_nonce() {
        // Guards the failure that would be catastrophic rather than merely
        // wrong: with this cipher, reusing a nonce under one key leaks the
        // authentication key. Hoisting one nonce out of the loop would look
        // like a tidy-up.
        let snapshot = sealed();
        let agent_nonce = &snapshot
            .agents
            .get(&Agent::Claude)
            .expect("the agent is configured")
            .auth_token
            .nonce;
        let project = snapshot
            .projects
            .values()
            .next()
            .expect("the project is there");
        let project_nonce = &project
            .credentials
            .get(&Platform::GitHub)
            .expect("the credential is there")
            .nonce;
        let channel_nonce = &project
            .channels
            .get(&Channel::Slack)
            .expect("the binding is there")
            .credential
            .nonce;
        assert_ne!(agent_nonce, project_nonce);
        assert_ne!(project_nonce, channel_nonce);
        assert_ne!(agent_nonce, channel_nonce);
    }

    #[test]
    fn the_wrong_key_does_not_open_a_snapshot() {
        assert!(matches!(
            sealed().open(&Key::new([8; 32])),
            Err(OpenError::Cipher)
        ));
    }

    #[test]
    fn an_altered_snapshot_is_refused_rather_than_misread() {
        // The cipher authenticates, so tampering is a failure rather than a
        // plausible-looking wrong answer. That is why it is an AEAD and not
        // just encryption.
        let mut snapshot = sealed();
        let sealed_token = &mut snapshot
            .agents
            .get_mut(&Agent::Claude)
            .expect("the agent is configured")
            .auth_token;
        let mut raw = BASE64
            .decode(&sealed_token.ciphertext)
            .expect("we wrote valid base64");
        raw[0] ^= 0xFF;
        sealed_token.ciphertext = BASE64.encode(raw);

        assert!(matches!(snapshot.open(&key()), Err(OpenError::Cipher)));
    }

    #[test]
    fn a_snapshot_naming_an_agent_it_does_not_configure_is_refused() {
        // The check that lets every later caller resolve a project's agents
        // without handling an absence. A file is untrusted input — hand-edited,
        // half-written, or written by an older version — and this is where that
        // stops being assumed.
        let mut snapshot = sealed();
        snapshot.agents.clear();
        assert!(matches!(
            snapshot.open(&key()),
            Err(OpenError::Inconsistent(
                Inconsistent::UnconfiguredProjectAgent {
                    agent: Agent::Claude,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn a_key_does_not_leak_when_formatted() {
        assert!(!format!("{:?}", key()).contains('7'));
    }

    #[test]
    fn every_agent_says_what_it_is_good_for() {
        // The orchestrator picks an agent by reading this, so an empty one is
        // a silent failure rather than a cosmetic one.
        assert!(!Agent::Claude.description().is_empty());
    }

    /// Two projects whose credentials differ, so that "the wrong one" is
    /// detectable rather than merely absent.
    fn two_projects() -> (State, ProjectId, ProjectId) {
        let mut state = configured();
        let mine = ProjectId::from_uuid(Uuid::from_u128(3));
        let theirs = ProjectId::from_uuid(Uuid::from_u128(4));

        state.projects.insert(mine, a_project_with_a_job());

        let mut other = a_project_with_a_job();
        other.name = "somebody else".to_owned();
        other.credentials.insert(
            Platform::GitHub,
            Secret::new("not-yours-and-never-was".to_owned()),
        );
        other.channels.insert(
            Channel::Slack,
            ChannelConfig {
                address: "C9999999999".to_owned(),
                credential: Secret::new(ALIEN_CHANNEL_TOKEN.to_owned()),
            },
        );
        state.projects.insert(theirs, other);

        (state, mine, theirs)
    }

    #[test]
    fn triage_is_handed_its_credential_and_no_platform_at_all() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_triage(&state, mine).expect("a watched project");

        assert_eq!(handout.agent(), Agent::Claude);
        assert_eq!(handout.agent_credential().expose(), "agent-token");
        assert_eq!(handout.platforms().count(), 0);
        assert!(handout.platform(Platform::GitHub).is_none());
    }

    /// The asymmetry `docs/decisions/0027-a-channel-is-not-a-platform.md` is
    /// built on, asserted beside the test that establishes the other half.
    ///
    /// An orchestrator watches its project's channels — that is the whole of
    /// its remit — so a handout that withheld them the way it withholds
    /// platform credentials would leave it unable to work.
    #[test]
    fn triage_is_handed_the_channels_it_has_to_watch() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_triage(&state, mine).expect("a watched project");

        let watching = handout
            .channel(Channel::Slack)
            .expect("the binding came through");
        assert_eq!(watching.address, CHANNEL_ADDRESS);
        assert_eq!(watching.credential.expose(), CHANNEL_TOKEN);
        assert_eq!(handout.channels().count(), 1);
    }

    #[test]
    fn a_job_is_handed_its_own_projects_credentials() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Agent::Claude, mine).expect("a watched project");

        assert_eq!(handout.agent_credential().expose(), "agent-token");
        assert_eq!(
            handout.platform(Platform::GitHub).map(Secret::expose),
            Some(TOKEN)
        );
    }

    /// How a job speaks without a terminal — the invariant in
    /// `docs/architecture.md` §2 needs both halves of the binding, because an
    /// agent that holds the credential and not the address has nowhere to put
    /// the question.
    #[test]
    fn a_job_is_handed_the_channel_it_speaks_on() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Agent::Claude, mine).expect("a watched project");

        let speaking = handout
            .channel(Channel::Slack)
            .expect("the binding came through");
        assert_eq!(speaking.address, CHANNEL_ADDRESS);
        assert_eq!(speaking.credential.expose(), CHANNEL_TOKEN);
    }

    /// The escape test `docs/conventions.md` §4 asks for, at the level where
    /// the selection actually happens: a handout built for one project must
    /// carry nothing belonging to another, and the two are distinguishable
    /// because their credentials differ rather than because one is empty.
    #[test]
    fn a_jobs_handout_carries_nothing_belonging_to_another_project() {
        let (state, mine, theirs) = two_projects();

        let ours = Handout::for_job(&state, Agent::Claude, mine).expect("a watched project");
        let alien = Handout::for_job(&state, Agent::Claude, theirs).expect("a watched project");

        let leaked = alien.platform(Platform::GitHub).map(Secret::expose);
        assert_eq!(leaked, Some("not-yours-and-never-was"));
        assert_ne!(ours.platform(Platform::GitHub).map(Secret::expose), leaked);

        for (_, secret) in ours.platforms() {
            assert_ne!(secret.expose(), "not-yours-and-never-was");
        }

        // The same claim for the second map. Selection happens once per map,
        // so a map added without being selected from is exactly the mistake
        // this catches.
        assert_eq!(
            alien.channel(Channel::Slack).map(|c| c.credential.expose()),
            Some(ALIEN_CHANNEL_TOKEN)
        );
        for (_, bound) in ours.channels() {
            assert_ne!(bound.credential.expose(), ALIEN_CHANNEL_TOKEN);
        }
    }

    #[test]
    fn a_handout_for_a_project_this_instance_does_not_watch_is_refused() {
        let (state, _, _) = two_projects();
        let stranger = ProjectId::from_uuid(Uuid::from_u128(99));

        let refused = Handout::for_job(&state, Agent::Claude, stranger);

        assert!(matches!(refused, Err(HandoutError::UnknownProject(id)) if id == stranger));
    }

    #[test]
    fn a_handout_for_an_agent_with_no_configuration_is_refused() {
        let (mut state, mine, _) = two_projects();
        state.agents.clear();

        let refused = Handout::for_job(&state, Agent::Claude, mine);

        assert!(matches!(
            refused,
            Err(HandoutError::UnconfiguredAgent(Agent::Claude))
        ));
        assert!(Handout::for_triage(&state, mine).is_err());
    }

    #[test]
    fn a_handout_does_not_leak_a_credential_when_formatted() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Agent::Claude, mine).expect("a watched project");

        let shown = format!("{handout:?}");

        assert!(!shown.contains("agent-token"), "{shown}");
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains(CHANNEL_TOKEN), "{shown}");
        assert!(
            shown.contains("GitHub"),
            "it should still say what it holds"
        );
        assert!(shown.contains("Slack"), "{shown}");
    }

    #[test]
    fn a_jobs_progress_survives_the_snapshot_boundary() {
        let mut state = populated();
        let project = state
            .projects
            .values_mut()
            .next()
            .expect("the populated state has one");
        let job = project
            .jobs
            .values_mut()
            .next()
            .expect("that project has one job");
        job.progress = Progress::Failed("the credential had expired".to_owned());

        let recovered = state
            .seal(&key(), &mut counting_nonces())
            .expect("sealing succeeds")
            .open(&key())
            .expect("and opens again");

        let carried = recovered
            .projects
            .values()
            .next()
            .and_then(|project| project.jobs.values().next())
            .map(|job| job.progress.clone());

        assert_eq!(
            carried,
            Some(Progress::Failed("the credential had expired".to_owned()))
        );
    }

    #[test]
    fn running_finds_a_job_wherever_its_project_is() {
        let state = populated();
        let found: Vec<JobId> = state.running().collect();

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(state.job(found[0]).is_some());
    }

    #[test]
    fn a_job_that_has_finished_is_not_running() {
        let mut state = populated();
        let id = state.running().next().expect("one to start with");

        state.job_mut(id).expect("it is there").progress = Progress::Completed;

        assert_eq!(state.running().count(), 0);
        assert!(
            state.job(id).is_some(),
            "finishing is not forgetting: the record stays"
        );
    }

    #[test]
    fn a_job_this_instance_never_had_is_not_found() {
        let state = populated();

        assert!(state.job(JobId::from_uuid(Uuid::from_u128(404))).is_none());
    }

    /// A file describing an instance that cannot exist is refused where it is
    /// read, rather than believed and acted on.
    #[test]
    fn a_snapshot_giving_a_project_no_job_agents_is_refused() {
        let mut snapshot = sealed();
        for project in snapshot.projects.values_mut() {
            project.job_agents.clear();
        }

        assert!(matches!(
            snapshot.open(&key()),
            Err(OpenError::Inconsistent(Inconsistent::NoJobAgents(_)))
        ));
    }
}
