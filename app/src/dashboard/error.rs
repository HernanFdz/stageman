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
            Self::UnknownAgent { .. } => StatusCode::NOT_FOUND,
            Self::CredentialMissing => StatusCode::BAD_REQUEST,
            // Not a bad request: the request was well formed and the instance
            // is in a state that forbids it, which is what a conflict means.
            Self::AgentInUse { .. } => StatusCode::CONFLICT,
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
