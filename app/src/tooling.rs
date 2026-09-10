//! The tools this instance serves to the agents it runs.
//!
//! `docs/decisions/0034-tools-are-served-not-shipped.md` moves everything an
//! agent does outside its container from a program shipped in the image to a
//! tool served from here. This is that endpoint: MCP over HTTP, on the
//! listener `endpoint` already binds, reached through the one hostname a
//! container has on either runtime.
//!
//! **Nothing here decides.** The whole request — who it came from, what it
//! presented, and what it asked — is handed to the instance as one event, and
//! what comes back is a status and a body. Which credential names whom, what
//! a bearer may be offered, and what each tool does are all the instance's,
//! where they are tested against scenarios rather than against a port.

// Through the framework's re-export rather than a direct dependency, for the
// reason `endpoint` and `serving` do the same: the version that matters is
// whichever one the framework serves with, and naming it twice is how the two
// drift.
use dioxus::server::axum;
use dioxus::server::axum::response::IntoResponse as _;

/// The credential a request presented, if it presented one.
///
/// Bearer only, and compared nowhere here: this reads the header and the
/// instance decides whether it names anything, so that a malformed header and
/// an unknown credential reach the same refusal by the same path.
#[must_use]
fn presented(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Where a container reaches the tools this instance serves.
///
/// One hostname for both runtimes, which is measured rather than assumed:
/// `--add-host=host.docker.internal:host-gateway` is honoured by Docker and by
/// Podman alike, so nothing here has to know which one is in use.
///
/// The one thing served on that listener, since
/// `docs/decisions/0034-tools-are-served-not-shipped.md` removed the route a
/// shipped program used to post to. Where it binds, and why that is the
/// awkward part, is
/// `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
#[must_use]
pub fn endpoint(port: u16) -> String {
    format!("http://host.docker.internal:{port}/mcp")
}

/// The tools endpoint, for the listener `endpoint` binds.
pub fn served() -> axum::routing::MethodRouter {
    axum::routing::post(called).get(declining).delete(closing)
}

/// Declines the stream a client may offer to open.
///
/// Every tool here answers within its own call, so there is nothing this
/// instance would ever push. Measured: a client offered the stream, was
/// refused, and completed a tool call regardless.
#[mutants::skip]
async fn declining() -> axum::http::StatusCode {
    axum::http::StatusCode::METHOD_NOT_ALLOWED
}

/// Accepts a client hanging up.
///
/// Nothing is held per connection — the credential decides everything and is
/// presented on each request — so this has nothing to release and says so
/// rather than refusing.
#[mutants::skip]
async fn closing() -> axum::http::StatusCode {
    axum::http::StatusCode::NO_CONTENT
}

/// Hands one request to the instance, and answers with what it said.
///
/// The peer check is made here because only this layer can see the peer, and
/// carried across as a fact rather than acted on: refusing is the instance's,
/// so that a request from beyond this machine and one presenting a credential
/// nobody holds are refused by the same code with the same answer.
#[mutants::skip]
async fn called(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let nearby = crate::endpoint::from_nearby(peer.ip());
    if !nearby {
        tracing::warn!(%peer, "the tools were reached from beyond this machine");
    }
    let bearer = presented(&headers).map(str::to_owned);
    let Some(world) = crate::world() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match world
        .call(stageman_core::Timestamp::now(), nearby, bearer, body)
        .await
    {
        Some((status, body)) => {
            let status = axum::http::StatusCode::from_u16(status)
                .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
            body.map_or_else(
                || status.into_response(),
                |body| (status, axum::Json(body)).into_response(),
            )
        }
        None => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{axum, endpoint, presented};

    /// The credential is read from the one header and the one scheme.
    #[test]
    fn only_a_bearer_credential_is_presented() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(presented(&headers), None);

        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer  not-a-real-credential ".parse().expect("a header"),
        );
        assert_eq!(presented(&headers), Some("not-a-real-credential"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Basic bm90LWEtcmVhbC1jcmVkZW50aWFs"
                .parse()
                .expect("a header"),
        );
        assert_eq!(presented(&headers), None, "another scheme presents nothing");
    }

    /// The address a container is told is the one hostname both runtimes
    /// resolve to the host.
    #[test]
    fn the_endpoint_names_the_host_as_a_container_sees_it() {
        assert_eq!(endpoint(47_113), "http://host.docker.internal:47113/mcp");
    }
}
