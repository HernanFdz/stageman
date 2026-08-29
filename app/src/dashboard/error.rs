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

    /// A project names an agent that has no credential.
    ///
    /// The other half of what the domain calls an inconsistent instance, and
    /// the reason the agents screen comes first.
    #[error("{name} has no credential, so a project cannot name it")]
    AgentNotConfigured {
        /// The agent, as the screen names it.
        name: String,
    },

    /// Nothing by that name is a platform this knows about.
    #[error("no platform is called {name}")]
    UnknownPlatform {
        /// What was asked for.
        name: String,
    },

    /// A project cannot be forgotten while its jobs are still running.
    ///
    /// Not tidiness. A running job owns a container named after it, and
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` rests on the
    /// instance being able to name every container it started — forget the
    /// project and those containers become exactly the leak that record
    /// exists to prevent.
    #[error("{name} still has {running} job(s) running")]
    ProjectBusy {
        /// The project, as the screen names it.
        name: String,
        /// How many jobs are still going.
        running: usize,
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
            Self::UnknownAgent { .. }
            | Self::UnknownProject { .. }
            | Self::UnknownPlatform { .. } => StatusCode::NOT_FOUND,
            // Well-formed requests that describe something invalid. The
            // operator can fix all of these by typing something different,
            // which is what separates them from the two above.
            Self::CredentialMissing
            | Self::Incomplete { .. }
            | Self::JobAgentsMissing
            | Self::AgentNotConfigured { .. } => StatusCode::BAD_REQUEST,
            // Not a bad request: the request was well formed and the instance
            // is in a state that forbids it, which is what a conflict means.
            Self::AgentInUse { .. } | Self::ProjectBusy { .. } => StatusCode::CONFLICT,
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
            running: 2,
        };

        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert_eq!(refused.to_string(), "aviary still has 2 job(s) running");
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
