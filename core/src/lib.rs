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

use jiff::Timestamp;
use uuid::Uuid;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Platform {
    /// The repository host.
    GitHub,
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
}

#[cfg(test)]
mod tests {
    use super::{Agent, AgentConfig, Job, JobId, Platform, Project, ProjectId, Secret, State};
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

    #[test]
    fn every_agent_says_what_it_is_good_for() {
        // The orchestrator picks an agent by reading this, so an empty one is
        // a silent failure rather than a cosmetic one.
        assert!(!Agent::Claude.description().is_empty());
    }
}
