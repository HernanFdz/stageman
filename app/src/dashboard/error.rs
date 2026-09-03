//! What a route can fail with, in a shape the browser can match on.
//!
//! **Compiled for both halves**, which is the whole point. `docs/conventions.md`
//! §3 asks for typed errors wherever one crosses a boundary, and until this
//! existed the widest boundary in the project — the wire — carried a string.
//! A page that wants to behave differently for "you asked for an agent that
//! does not exist" than for "that agent is still in use" had to read prose to
//! find out which it had.
//!
//! Every variant is safe to send. That is a rule about what may be added here
//! rather than an observation: a failure carrying something an operator should
//! not read is reported as [`DashboardError::Failed`] and logged where the
//! operator can see it, because the browser is the one audience that gets no
//! say in who is looking.

use dioxus::fullstack::AsStatusCode;
use dioxus::prelude::{ServerFnError, StatusCode};
use serde::{Deserialize, Serialize};

/// What every route returns.
pub type DashboardResult<T> = Result<T, DashboardError>;

/// A route could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DashboardError {
    /// This process is not operating an instance.
    ///
    /// A fault in how the server was assembled rather than in anything a
    /// request did, and unreachable through the binary — which is why it says
    /// so rather than suggesting a remedy.
    #[error("this server is not operating an instance")]
    NoInstance,

    /// Nothing by that name can be run.
    ///
    /// The set of agents is closed and compiled in, per
    /// `docs/decisions/0006-agents-are-pluggable.md`, so this is a stale page
    /// or a hand-made request rather than a typo an operator can fix.
    #[error("no agent is called {name}")]
    UnknownAgent {
        /// What was asked for.
        name: String,
    },

    /// An agent was configured with nothing.
    ///
    /// Separate from a *wrong* credential on purpose: nothing here can tell
    /// whether a credential works without spending it, and refusing an empty
    /// one is the only check available before then.
    #[error("that agent needs a credential")]
    CredentialMissing,

    /// An agent cannot be forgotten while a project names it.
    ///
    /// Carries the projects rather than merely refusing, which is what
    /// `State::used_by` exists for and what lets a page say *why* — see
    /// `docs/decisions/0021-an-instance-starts-empty.md`.
    #[error("{agent} is still used by {}", projects.join(", "))]
    AgentInUse {
        /// The agent that was to be forgotten.
        agent: String,
        /// What would have broken. Names, because an identifier means nothing
        /// to whoever is reading the screen.
        projects: Vec<String>,
    },

    /// Nothing by that identifier is being watched.
    ///
    /// A stale page, most likely: somebody forgot a project in one tab and
    /// acted on it in another.
    #[error("no project has the identifier {id}")]
    UnknownProject {
        /// What was asked for.
        id: String,
    },

    /// A field that has to say something says nothing.
    ///
    /// One variant rather than one per field, because the page knows which box
    /// is empty and the operator is looking at it. What the wire has to carry
    /// is which one, not a sentence about it.
    #[error("{field} cannot be empty")]
    Incomplete {
        /// The field, named as the screen names it.
        field: String,
    },

    /// A project would have no agent its jobs could run on.
    ///
    /// Refused rather than stored, because a project whose jobs cannot run
    /// cannot do the one thing a project is for — the domain says so in
    /// `State::check`, and this is that refusal reaching a screen.
    #[error("a project needs at least one agent its jobs can run on")]
    JobAgentsMissing,

    /// Another project already listens where this one would.
    ///
    /// Two projects on one channel makes routing ambiguous in the one place it
    /// cannot be: a message at the root belongs to whichever foreman the
    /// search reaches first, which is an ordering rather than an answer. If
    /// both listen it is worse — both hear every message, and one job's reply
    /// is delivered twice.
    #[error("{project} is already bound to that channel")]
    ChannelAlreadyBound {
        /// The project that has it, as the screen names it.
        project: String,
    },

    /// A channel was given an address without a credential, or the reverse.
    ///
    /// A binding is two values and neither half works alone: an address with
    /// nothing to authenticate with cannot be reached, and a credential with
    /// nowhere to point has nowhere to speak. Refused rather than stored,
    /// because a half-bound channel looks bound on every screen and fails at
    /// the one moment it is needed — which is a job with a question and
    /// nowhere to put it.
    ///
    /// Distinct from [`DashboardError::Incomplete`], which says a required
    /// field is empty. Both of these fields are optional; what is refused is
    /// the combination.
    #[error("a channel needs an address and a credential, or neither")]
    ChannelIncomplete,

    /// A project names an agent that has no credential.
    ///
    /// The other half of what the domain calls an inconsistent instance, and
    /// the reason the agents screen comes first.
    #[error("{name} has no credential, so a project cannot name it")]
    AgentNotConfigured {
        /// The agent, as the screen names it.
        name: String,
    },

    /// A project's jobs may not run on that agent.
    ///
    /// Distinct from an agent having no credential: this one is configured and
    /// simply is not among the ones this project named. Refusing here rather
    /// than letting the handout fail keeps the answer about the request rather
    /// than about an instance that has stopped making sense.
    #[error("{project} does not run its jobs on {name}")]
    AgentNotOnProject {
        /// The agent, as the screen names it.
        name: String,
        /// The project, as the screen names it.
        project: String,
    },

    /// A variable was given a name a container could not be given.
    ///
    /// **Says which row and never what was in it**, and that is the whole
    /// reason this carries a position rather than the name. The mistake this
    /// most often catches is a credential pasted into the name box — a token
    /// with an equals sign in it is exactly the shape that fails here — so an
    /// error repeating the name would be an error repeating a secret, and
    /// `docs/conventions.md` §4 does not care that it arrived in the wrong
    /// field.
    ///
    /// The reason comes from the domain, which is written to the same rule.
    #[error("variable {position} has a name an environment cannot carry: {rule}")]
    VariableNameRefused {
        /// Which row, counting from one, as the screen shows them.
        position: usize,
        /// Which rule it broke. Named `rule` rather than `reason` because this
        /// enum is serialised with `reason` as its tag, and a field of that
        /// name is refused by the derive.
        rule: String,
    },

    /// A variable claims a name stageman delivers itself.
    ///
    /// Refused here rather than at delivery, so an operator finds out while
    /// looking at the box rather than when a job fails. The name is safe to
    /// repeat, unlike the one above: it is necessarily one of a handful of
    /// compiled-in constants, so it cannot be anything an operator typed
    /// except by naming it exactly.
    ///
    /// It matters more than a name clash usually would. One of these would
    /// change which account an agent bills, with no error anywhere — see
    /// `docs/decisions/0008-one-credential-per-agent.md`.
    #[error("{name} is a variable stageman sets itself, so a project cannot")]
    VariableReserved {
        /// The reserved name that was claimed.
        name: String,
    },

    /// Two rows give the same name.
    ///
    /// Refused rather than resolved, because either resolution is a silent
    /// wrong answer: taking the last discards a value the operator typed, and
    /// taking the first discards the one they typed second and probably meant.
    ///
    /// By position, for the reason above — a valid name can still be a pasted
    /// credential, since a token of letters and underscores passes every rule
    /// there is.
    #[error("variable {position} repeats a name given earlier")]
    VariableRepeated {
        /// Which row, counting from one.
        position: usize,
    },

    /// A variable that does not exist yet was given no value.
    ///
    /// An empty box means *keep what is there* — there is no way to show an
    /// operator the value a project already holds, so blank has to mean
    /// unchanged. That reading has nothing to fall back on for a name the
    /// project has never had, and storing an empty credential would leave
    /// something that reads as configured and authenticates as nothing.
    #[error("variable {position} is new, so it needs a value")]
    VariableValueMissing {
        /// Which row, counting from one.
        position: usize,
    },

    /// A project cannot be forgotten while its jobs are still running.
    ///
    /// Not tidiness. A running job owns a container named after it, and
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` rests on the
    /// instance being able to name every container it started — forget the
    /// project and those containers become exactly the leak that record
    /// exists to prevent.
    #[error("{name} still has {working} job(s) working")]
    ProjectBusy {
        /// The project, as the screen names it.
        name: String,
        /// How many jobs are still going.
        working: usize,
    },

    /// Something went wrong that the operator cannot act on from here.
    ///
    /// Deliberately opaque and deliberately singular. Anything with a cause
    /// worth reading is logged with that cause; what reaches the browser is
    /// that it did not work, because the alternative is deciding case by case
    /// which internal detail is safe to publish, and that decision gets made
    /// wrong eventually.
    #[error("that did not work — the server log says why")]
    Failed,
}

impl DashboardError {
    /// The status this failure answers with.
    ///
    /// A dashboard barely needs these — it reads the variant, not the number —
    /// but anything else speaking to these routes does, and a route that
    /// answers 200 for a refusal is lying to every client that is not this
    /// page.
    const fn status(&self) -> StatusCode {
        match self {
            // Nothing was asked for that does not exist; this process is
            // wrong.
            Self::NoInstance | Self::Failed => StatusCode::INTERNAL_SERVER_ERROR,
            Self::UnknownAgent { .. } | Self::UnknownProject { .. } => StatusCode::NOT_FOUND,
            // Well-formed requests that describe something invalid. The
            // operator can fix all of these by typing something different,
            // which is what separates them from the two above.
            Self::CredentialMissing
            | Self::Incomplete { .. }
            | Self::JobAgentsMissing
            | Self::AgentNotConfigured { .. }
            | Self::AgentNotOnProject { .. }
            | Self::VariableNameRefused { .. }
            | Self::VariableReserved { .. }
            | Self::VariableRepeated { .. }
            | Self::VariableValueMissing { .. }
            | Self::ChannelIncomplete => StatusCode::BAD_REQUEST,
            // The request is well formed and the instance is in a state that
            // forbids it, which is what a conflict means.
            // Not a bad request: the request was well formed and the instance
            // is in a state that forbids it, which is what a conflict means.
            Self::AgentInUse { .. }
            | Self::ProjectBusy { .. }
            | Self::ChannelAlreadyBound { .. } => StatusCode::CONFLICT,
        }
    }
}

impl AsStatusCode for DashboardError {
    fn as_status_code(&self) -> StatusCode {
        self.status()
    }
}

/// Transport failures collapse to the opaque variant.
///
/// A network that dropped, a response that would not decode: real, and none of
/// them something the operator can act on beyond trying again. The original is
/// logged rather than sent, for the reason [`DashboardError::Failed`] gives.
impl From<ServerFnError> for DashboardError {
    fn from(failure: ServerFnError) -> Self {
        // Through the framework's re-export rather than a direct dependency,
        // because this type is compiled for the browser too and `tracing` is
        // one of the crates the manifest keeps out of that half. It is the
        // same crate underneath, so a failure on the daemon still reaches the
        // subscriber `serve` installed, and one in the browser reaches the
        // console.
        dioxus::logger::tracing::error!(?failure, "a dashboard route failed in transport");
        Self::Failed
    }
}

/// The domain's verdict on a state, in terms a screen can show.
///
/// The mapping is the whole point: `State::check` is the one definition of
/// what a valid instance is — `docs/decisions/0021-an-instance-starts-empty.md`
/// settled that deliberately — so a route asks it rather than re-deciding, and
/// this turns its answer into something with a field the operator recognises.
#[cfg(feature = "server")]
impl DashboardError {
    /// Translates a refusal from the domain.
    pub(super) fn from_inconsistent(
        reason: &stageman_core::Inconsistent,
        naming: impl Fn(stageman_core::Agent) -> String,
    ) -> Self {
        match reason {
            stageman_core::Inconsistent::NoJobAgents(_) => Self::JobAgentsMissing,
            stageman_core::Inconsistent::UnconfiguredProjectAgent { agent, .. } => {
                Self::AgentNotConfigured {
                    name: naming(*agent),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DashboardError, StatusCode};

    /// A refusal an operator can fix must not read as a server fault.
    ///
    /// The distinction this is really defending: `AgentInUse` is the one
    /// failure here that is *correct behaviour*, and answering 500 for it
    /// would tell every client that stageman had broken.
    #[test]
    fn a_refusal_is_not_reported_as_a_fault() {
        let refused = DashboardError::AgentInUse {
            agent: "claude".to_owned(),
            projects: vec!["aviary".to_owned()],
        };

        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert!(!refused.status().is_server_error());
    }

    /// A project that is busy is a refusal too, not a fault.
    #[test]
    fn a_busy_project_is_a_refusal_rather_than_a_fault() {
        let refused = DashboardError::ProjectBusy {
            name: "aviary".to_owned(),
            working: 2,
        };

        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert_eq!(refused.to_string(), "aviary still has 2 job(s) working");
    }

    /// Something the operator can retype is not a not-found.
    #[test]
    fn an_incomplete_field_asks_for_a_correction_rather_than_reporting_absence() {
        let refused = DashboardError::Incomplete {
            field: "repository".to_owned(),
        };

        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    }

    /// The refusal says what would break, because a page has to show it.
    #[test]
    fn a_refusal_names_what_would_break() {
        let refused = DashboardError::AgentInUse {
            agent: "claude".to_owned(),
            projects: vec!["aviary".to_owned(), "burrow".to_owned()],
        };

        assert_eq!(
            refused.to_string(),
            "claude is still used by aviary, burrow"
        );
    }
}
