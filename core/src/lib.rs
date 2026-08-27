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
//! **Nothing here reads a clock or mints an identifier.** Both are effects, and
//! both would make every value non-deterministic to construct, so both are
//! supplied by the caller — which is why creating a job takes a timestamp
//! rather than asking the operating system for one. The crates that are allowed
//! effects do that; this one stays a set of values a test can build exactly.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use jiff::Timestamp;
use uuid::Uuid;

/// A credential, in memory.
///
/// Formatting is redacted in both `Debug` and `Display`, because the usual way
/// a token reaches a log is a structure printed whole while somebody is
/// debugging something else entirely.
///
/// **It deliberately does not implement serialisation yet.** State is persisted
/// by serialising the very structure credentials live in, so a `Serialize` that
/// wrote the value in the clear would put every token on disk on the next
/// change — the same bug as a derived `Debug`, wearing a different hat. Adding
/// it is the same commit as adding encryption, never an earlier one. The bar is
/// in `docs/conventions.md` §4.
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

/// Identifies a configured agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentId(Uuid);

/// Identifies a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(Uuid);

/// Identifies a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

identifier!(AgentId);
identifier!(ProjectId);
identifier!(JobId);

/// A platform a project's jobs act on.
///
/// One variant for now, which is the one a job cannot work without: cloning the
/// repository, pushing a branch and opening a pull request are all the same
/// credential. See
/// `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` for why a
/// job holds these at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Platform {
    /// The repository host.
    GitHub,
}

/// How to start an agent.
///
/// The program is a path rather than a name on purpose. Agents install where a
/// login shell can find them and a service manager cannot, so resolving one by
/// searching the environment works when you test it by hand and fails when
/// anything else starts it — see `docs/conventions.md` §3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Absolute path to the program to run.
    pub program: PathBuf,
    /// Arguments passed before anything this project adds.
    pub args: Vec<String>,
}

/// A coding agent this instance can run.
///
/// Always a third-party tool, never this project and never a job — the word is
/// reserved in `docs/conventions.md` §2 precisely because "the agent decided
/// to…" reads fine while meaning two different things.
#[derive(Debug, Clone)]
pub struct Agent {
    /// What to call it in the dashboard.
    pub name: String,
    /// What it is good for, in prose.
    ///
    /// Not decoration: the orchestrator chooses which agent runs a job, and
    /// this is what it reasons over. See
    /// `docs/decisions/0006-agents-are-pluggable.md`.
    pub description: String,
    /// How to start it.
    pub launch: Launch,
    /// What it authenticates with.
    ///
    /// One credential per agent, never one per role — the orchestrator and a
    /// job running the same agent use the same one. See
    /// `docs/decisions/0008-one-credential-per-agent.md`.
    pub credential: Secret,
}

/// One agent, in one workspace, working on one project.
///
/// A job happens once. There is no retry and no resume: a second attempt is a
/// new job with its own workspace, which is why nothing here records an
/// attempt count.
#[derive(Debug, Clone)]
pub struct Job {
    /// Which agent ran it.
    ///
    /// Recorded because once more than one agent can, "why did this go badly?"
    /// has no answer without it.
    pub agent: AgentId,
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

/// The agent a fresh instance is seeded with.
///
/// A fixed value rather than a minted one, so that the entry a first run
/// creates is the same entry on every machine and can be referred to before
/// anything has been configured.
pub const SEEDED_AGENT: AgentId = AgentId::from_uuid(Uuid::from_u128(1));

/// Everything one instance knows.
///
/// The whole of what gets snapshotted, and the whole of what is loaded back —
/// see `docs/decisions/0011-state-is-a-snapshot-not-a-database.md`. A default
/// value is a first run rather than an error.
#[derive(Debug, Clone)]
pub struct State {
    /// The agents this instance can run.
    pub agents: BTreeMap<AgentId, Agent>,
    /// The projects it watches.
    pub projects: BTreeMap<ProjectId, Project>,
    /// Which agent the orchestrator thinks with.
    ///
    /// Not optional. An instance with nothing to think with is not a state
    /// worth representing, and making it representable would spread option
    /// handling across every caller in exchange for catching the weaker half
    /// of a failure the startup check in `docs/conventions.md` §3 has to catch
    /// properly anyway — that check can tell an operator the path is wrong or
    /// the credential is dead, and a missing value cannot.
    pub orchestrator_agent: AgentId,
}

impl Default for State {
    /// A first run: one agent, unconfigured, and nothing else.
    ///
    /// The seeded entry is a **template**, not a working configuration. It
    /// cannot know where the program lives on this machine or what credential
    /// to use, so it carries a legible placeholder for the first and an empty
    /// value for the second, and the startup check refuses to run until both
    /// are real. In particular the program is a bare name, which
    /// `docs/conventions.md` §3 forbids resolving through the environment —
    /// the rule is kept by rejecting a non-absolute path at startup rather
    /// than by quietly searching for one.
    fn default() -> Self {
        let mut agents = BTreeMap::new();
        agents.insert(
            SEEDED_AGENT,
            Agent {
                name: "Claude Code".to_owned(),
                description: "General-purpose coding agent. Reads a repository, \
                              makes changes across files, runs commands, and \
                              explains what it did."
                    .to_owned(),
                launch: Launch {
                    program: PathBuf::from("claude"),
                    args: Vec::new(),
                },
                credential: Secret::new(String::new()),
            },
        );
        Self {
            agents,
            projects: BTreeMap::new(),
            orchestrator_agent: SEEDED_AGENT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentId, Platform, Project, Secret, State};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    const TOKEN: &str = "ghp-not-a-real-token";

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
        let mut credentials = BTreeMap::new();
        credentials.insert(Platform::GitHub, Secret::new(TOKEN.to_owned()));
        let project = Project {
            name: "example".to_owned(),
            repository: "https://example.invalid/repo".to_owned(),
            credentials,
            jobs: BTreeMap::new(),
        };
        assert!(!format!("{project:?}").contains(TOKEN));
    }

    #[test]
    fn a_secret_still_yields_its_value_when_asked() {
        assert_eq!(Secret::new(TOKEN.to_owned()).expose(), TOKEN);
    }

    #[test]
    fn a_fresh_instance_can_name_the_agent_it_thinks_with() {
        let state = State::default();
        assert!(state.projects.is_empty());
        // The point of seeding rather than leaving this absent: the reference
        // always resolves, so no caller has to handle a state that a working
        // instance never occupies.
        assert!(state.agents.contains_key(&state.orchestrator_agent));
    }

    #[test]
    fn the_seeded_agent_is_a_template_and_not_a_working_configuration() {
        // Guards the thing most likely to be "tidied up" later: the seed looks
        // configured, and is not. Startup is what refuses it — see
        // conventions §3 — so this test pins the shape that check relies on.
        let state = State::default();
        let agent = state
            .agents
            .get(&state.orchestrator_agent)
            .expect("the seeded agent is present by construction");
        assert!(agent.credential.expose().is_empty());
        assert!(!agent.launch.program.is_absolute());
    }

    #[test]
    fn identifiers_of_different_kinds_wrap_the_same_value_distinctly() {
        // Distinct types over one representation: the point is that a project
        // identifier cannot be passed where a job's is expected.
        let raw = Uuid::from_u128(7);
        assert_eq!(AgentId::from_uuid(raw).as_uuid(), &raw);
    }
}
