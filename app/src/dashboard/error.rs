//! What a route can fail with, in a shape the browser can match on.
//!
//! **Compiled for both halves**, which is the whole point. `docs/conventions.md`
//! §3 asks for typed errors wherever one crosses a boundary, and until this
//! existed the widest boundary in the project — the wire — carried a string.
//!
//! What the instance refuses is a [`Refusal`], built in the wire crate so the
//! instance can say it and the browser can read it. This type wraps it with
//! the two failures that are this process's rather than the instance's, and
//! carries the framework's traits, which a type in the wire crate cannot: the
//! framework is not among the things that crate names.
//!
//! Every variant is safe to send. That is a rule about what may be added here
//! rather than an observation: a failure carrying something an operator should
//! not read is reported as [`DashboardError::Failed`] and logged where the
//! operator can see it, because the browser is the one audience that gets no
//! say in who is looking.

use dioxus::fullstack::AsStatusCode;
use dioxus::prelude::{ServerFnError, StatusCode};
use serde::{Deserialize, Serialize};
pub use stageman_wire::Refusal;

/// What every route returns.
pub type DashboardResult<T> = Result<T, DashboardError>;

/// A route could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DashboardError {
    /// This process is not operating an instance.
    ///
    /// A fault in how the server was assembled rather than in anything a
    /// request did, and the one failure here nobody at a browser can act on.
    #[error("this process is not operating an instance")]
    NoInstance,
    /// The instance refused, and this is why.
    ///
    /// Flattened into this type on the wire, so a page reads the refusal's
    /// own reason rather than unwrapping one.
    #[error("{0}")]
    Refused(Refusal),
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
    fn status(&self) -> StatusCode {
        match self {
            // Nothing was asked for that does not exist; this process is
            // wrong.
            Self::NoInstance | Self::Failed => StatusCode::INTERNAL_SERVER_ERROR,
            // A refusal names its own status, and every status it names is a
            // real one: the fallback below is unreachable rather than a
            // substitute, and says so if a refusal ever names a number the
            // protocol does not have.
            Self::Refused(refusal) => {
                StatusCode::from_u16(refusal.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

impl From<Refusal> for DashboardError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
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
    use super::{DashboardError, Refusal, StatusCode};

    /// A refusal an operator can fix must not read as a server fault, and it
    /// keeps the status the instance gave it.
    #[test]
    fn a_refusal_keeps_its_own_status_and_its_own_words() {
        let refused = DashboardError::from(Refusal::AgentInUse {
            agent: "claude".to_owned(),
            projects: vec!["aviary".to_owned(), "burrow".to_owned()],
        });

        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert!(!refused.status().is_server_error());
        assert_eq!(
            refused.to_string(),
            "claude is still used by aviary, burrow"
        );

        assert_eq!(
            DashboardError::from(Refusal::Incomplete {
                field: "repository".to_owned()
            })
            .status(),
            StatusCode::BAD_REQUEST
        );
    }

    /// Only this process's own failures are faults.
    #[test]
    fn a_fault_in_this_process_is_a_server_error() {
        assert_eq!(
            DashboardError::NoInstance.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            DashboardError::Failed.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// A refusal crosses flat, so a page reads its reason directly.
    ///
    /// On the daemon's half only, because the encoder it asserts through is
    /// one of the crates the manifest keeps out of the browser.
    #[cfg(feature = "server")]
    #[test]
    fn a_refusal_crosses_with_its_reason_beside_the_outcome() {
        let served = serde_json::to_string(&DashboardError::from(Refusal::ProjectBusy {
            name: "aviary".to_owned(),
            working: 2,
        }))
        .expect("it serialises");
        assert!(served.contains(r#""outcome":"refused""#), "{served}");
        assert!(served.contains(r#""reason":"project_busy""#), "{served}");
        assert!(served.contains("aviary"), "{served}");
    }
}
