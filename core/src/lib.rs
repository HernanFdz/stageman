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

use std::collections::BTreeMap;
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

/// One agent, in one workspace, working on one project.
///
/// A job happens once. There is no retry and no resume: a second attempt is a
/// new job with its own workspace, which is why nothing here records an
/// attempt count.
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
    /// What its jobs are handed, one credential per platform.
    ///
    /// A map rather than a list so that two credentials for one platform is
    /// unrepresentable, and ordered rather than hashed so the snapshot does not
    /// reshuffle itself between writes.
    pub credentials: BTreeMap<Platform, Secret>,
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
#[derive(Debug, Clone)]
pub struct State {
    /// The agents this instance can run, and what each authenticates with.
    pub agents: BTreeMap<Agent, AgentConfig>,
    /// The projects it watches.
    pub projects: BTreeMap<ProjectId, Project>,
    /// Which agent the orchestrator thinks with.
    ///
    /// Not optional, and guaranteed to appear in `agents` — by construction
    /// here, and by the check a snapshot passes on its way back in. That is
    /// what lets every later caller look it up without handling an absence
    /// that a working instance never has.
    pub orchestrator_agent: Agent,
}

impl State {
    /// Builds the state of a freshly configured instance.
    ///
    /// Taking the agent and its configuration together is what makes the
    /// invariant above hold by construction: there is no moment at which an
    /// instance exists without something to think with.
    #[must_use]
    pub fn new(orchestrator_agent: Agent, config: AgentConfig) -> Self {
        let mut agents = BTreeMap::new();
        agents.insert(orchestrator_agent, config);
        Self {
            agents,
            projects: BTreeMap::new(),
            orchestrator_agent,
        }
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
                Ok((
                    *id,
                    SealedProject {
                        name: project.name.clone(),
                        repository: project.repository.clone(),
                        credentials,
                        jobs: project.jobs.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SealError>>()?;

        Ok(Snapshot {
            agents,
            projects,
            orchestrator_agent: self.orchestrator_agent,
        })
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

/// Sealing a credential failed.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// The cipher rejected the input.
    #[error("a credential could not be sealed")]
    Cipher,
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
    /// The orchestrator's agent has no configuration.
    #[error("the orchestrator's agent {0:?} has no configuration in this snapshot")]
    UnconfiguredOrchestratorAgent(Agent),
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

/// A project as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedProject {
    /// What to call it.
    pub name: String,
    /// Where the repository lives.
    pub repository: String,
    /// Its sealed credentials.
    pub credentials: BTreeMap<Platform, SealedSecret>,
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
    /// Which agent the orchestrator thinks with.
    pub orchestrator_agent: Agent,
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
        let Self {
            agents,
            projects,
            orchestrator_agent,
        } = self;

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
                Ok((
                    id,
                    Project {
                        name: project.name,
                        repository: project.repository,
                        credentials,
                        jobs: project.jobs,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;

        if !agents.contains_key(&orchestrator_agent) {
            return Err(OpenError::UnconfiguredOrchestratorAgent(orchestrator_agent));
        }

        Ok(State {
            agents,
            projects,
            orchestrator_agent,
        })
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
}

impl Handout {
    /// What the agent the orchestrator thinks with is handed.
    ///
    /// Its own credential, and no platform credential at all: triage has no
    /// project, no repository and no workspace, so there is nothing it could
    /// legitimately reach a platform for — see
    /// `docs/decisions/0012-agents-run-in-containers.md`.
    ///
    /// # Errors
    ///
    /// Fails if the orchestrator's agent has no configuration — which the
    /// invariant on [`State`] says cannot happen, since it holds at
    /// construction and is checked again when a snapshot is loaded.
    ///
    /// The signature admits it anyway, and deliberately. The alternative is a
    /// total function that substitutes an empty credential for a missing one,
    /// which converts a state that cannot occur into an authentication failure
    /// somewhere else entirely — the exact trade `.quality/gate-reference.md`
    /// forbids, where a loud failure is replaced by a silent wrong value. An
    /// unreachable error variant costs one `?`; a fabricated credential costs
    /// an afternoon.
    pub fn for_triage(state: &State) -> Result<Self, HandoutError> {
        let agent = state.orchestrator_agent;
        let config = state
            .agents
            .get(&agent)
            .ok_or(HandoutError::UnconfiguredAgent(agent))?;
        Ok(Self {
            agent,
            agent_credential: config.auth_token.clone(),
            platforms: BTreeMap::new(),
        })
    }

    /// What a job's agent is handed: its own credential, plus the platform
    /// credentials of the one project the job belongs to.
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
        Agent, AgentConfig, BASE64, Handout, HandoutError, Job, JobId, Key, NONCE_LEN, Nonce,
        OpenError, Platform, Project, ProjectId, Secret, Snapshot, State,
    };
    use base64::Engine as _;
    use jiff::Timestamp;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    const TOKEN: &str = "ghp-not-a-real-token";

    fn configured() -> State {
        State::new(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new("agent-token".to_owned()),
            },
        )
    }

    fn a_project_with_a_job() -> Project {
        let mut credentials = BTreeMap::new();
        credentials.insert(Platform::GitHub, Secret::new(TOKEN.to_owned()));
        let mut jobs = BTreeMap::new();
        jobs.insert(
            JobId::from_uuid(Uuid::from_u128(9)),
            Job {
                agent: Agent::Claude,
                reason: "an issue was opened".to_owned(),
                kickoff: "work on it".to_owned(),
                created_at: Timestamp::UNIX_EPOCH,
            },
        );
        Project {
            name: "example".to_owned(),
            repository: "https://example.invalid/repo".to_owned(),
            credentials,
            jobs,
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
        assert!(!format!("{:?}", a_project_with_a_job()).contains(TOKEN));
    }

    #[test]
    fn a_secret_still_yields_its_value_when_asked() {
        assert_eq!(Secret::new(TOKEN.to_owned()).expose(), TOKEN);
    }

    #[test]
    fn a_configured_instance_can_look_up_the_agent_it_thinks_with() {
        let state = configured();
        assert!(state.agents.contains_key(&state.orchestrator_agent));
        assert!(state.projects.is_empty());
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
        assert_ne!(agent_nonce, project_nonce);
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
    fn a_snapshot_naming_an_unconfigured_orchestrator_agent_is_refused() {
        // The check that lets every later caller look that agent up without
        // handling an absence. A file is untrusted input; this is where that
        // stops being true.
        let mut snapshot = sealed();
        snapshot.agents.clear();
        assert!(matches!(
            snapshot.open(&key()),
            Err(OpenError::UnconfiguredOrchestratorAgent(Agent::Claude))
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
        state.projects.insert(theirs, other);

        (state, mine, theirs)
    }

    #[test]
    fn triage_is_handed_its_credential_and_no_platform_at_all() {
        let state = configured();
        let handout = Handout::for_triage(&state).expect("a configured instance");

        assert_eq!(handout.agent(), Agent::Claude);
        assert_eq!(handout.agent_credential().expose(), "agent-token");
        assert_eq!(handout.platforms().count(), 0);
        assert!(handout.platform(Platform::GitHub).is_none());
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
        assert!(Handout::for_triage(&state).is_err());
    }

    #[test]
    fn a_handout_does_not_leak_a_credential_when_formatted() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Agent::Claude, mine).expect("a watched project");

        let shown = format!("{handout:?}");

        assert!(!shown.contains("agent-token"), "{shown}");
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(
            shown.contains("GitHub"),
            "it should still say what it holds"
        );
    }
}
