//! Where this instance answers something running in a container.
//!
//! A listener of its own, separate from the dashboard's. Separate because of
//! where it has to bind: the dashboard stays on loopback, and this cannot —
//! see `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
//!
//! **Two things stand between a stranger and what is served here**, and only
//! the first is a real barrier. The credential, which decides not merely
//! whether a caller is answered but what it is offered. And the peer check
//! below, which refuses anything that did not come from this machine or its
//! containers.
//!
//! **This module is where it listens; `tooling` is what it serves.** It used
//! to be both, and to serve one bespoke route to a program shipped in the
//! image. `docs/decisions/0034-tools-are-served-not-shipped.md` removed that
//! program and the route with it, so what is left here is the address and who
//! may reach it — which is the half 0033 is about.
// Through the framework's re-export rather than a direct dependency, for the
// reason serving does the same: the version that matters is whichever one the
// framework serves with, and naming it twice is how the two drift.
use dioxus::server::axum;
use std::net::IpAddr;

/// Where the endpoint listens, when the environment does not say.
///
/// High and unusual, because the point is to collide with nothing. It is not
/// configuration in any meaningful sense — nobody needs to know it, and
/// nothing is served there that a person would visit — but a port can always
/// collide with something already running, so there is a way out.
const DEFAULT_PORT: u16 = 47_113;

/// What names a different port.
const PORT_VARIABLE: &str = "STAGEMAN_JOB_PORT";

/// The port a foreman reaches this instance on.
///
/// Read once per process, like the runtime: a value that changed underneath a
/// running daemon would leave containers holding an address that used to work.
pub static PORT: std::sync::LazyLock<u16> =
    std::sync::LazyLock::new(|| chosen_port(std::env::var(PORT_VARIABLE).ok().as_deref()));

/// Which port to listen on, given what the environment said.
///
/// Anything unreadable falls back rather than failing, and that is deliberate:
/// a mistyped port should not stop an instance starting, because the endpoint
/// is not what an operator came for. It is reported at startup either way, so
/// a fallback is visible rather than silent.
///
/// **Zero is honoured, and means whichever port is free.** No container can be
/// told about a port chosen after it was asked for, so nothing real wants
/// this — but a test that runs the binary does, and without it every such test
/// fights over one fixed port, with each other and with whatever instance the
/// operator is running. That is not hypothetical: a leaked mutation-testing
/// process held this port and a real daemon quietly could not bind it.
fn chosen_port(named: Option<&str>) -> u16 {
    named
        .and_then(|named| named.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Whether a request came from somewhere allowed to ask.
///
/// Not a security boundary — the warrant is that — but it costs three lines
/// and removes an entire class of caller. The endpoint binds every interface
/// because that is the only address a container can reach on every platform,
/// and nothing routed from beyond this machine has any business here.
///
/// Loopback for a request from the host itself, and private ranges for one
/// from a container: every container network is private by construction.
#[must_use]
pub fn from_nearby(peer: IpAddr) -> bool {
    match peer {
        IpAddr::V4(address) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        // A container reaching a host over IPv6 arrives from a unique-local
        // address, which is the v6 spelling of private.
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                // A v4 address arriving mapped into v6, which is what a dual
                // stack listener reports for an ordinary v4 peer.
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback() || mapped.is_private())
        }
    }
}

/// Serves what a foreman may reach: the job route, and the tools endpoint.
///
/// **Bound to every interface**, which is not a preference: it is the only
/// address a container can reach on every platform, measured on both runtimes.
/// The dashboard keeps its own listener on loopback and is not served here.
/// See `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
///
/// # Errors
///
/// Fails if the port cannot be bound, which is worth failing over: a foreman
/// that cannot create jobs is a foreman that can only talk, and finding that
/// out at the first message is worse than at startup.
#[mutants::skip]
pub async fn bind() -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(("0.0.0.0", *PORT)).await
}

/// Serves what a foreman may reach, on a listener already bound.
///
/// Bound separately and earlier, so that failing to bind is *reported* rather
/// than logged into a scrolling terminal. A port already taken is the ordinary
/// failure here — another instance, or something of somebody else's — and it
/// leaves a foreman able to talk and unable to work, which is indistinguishable
/// from a foreman that has decided not to.
#[mutants::skip]
pub async fn serve(
    listening: tokio::net::TcpListener,
    store: std::sync::Arc<crate::Store>,
    sessions: std::sync::Arc<crate::tooling::Sessions>,
) -> std::io::Result<()> {
    let router = axum::Router::new()
        .route("/mcp", crate::tooling::served())
        .layer(axum::Extension(store))
        .layer(axum::Extension(sessions));

    axum::serve(
        listening,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PORT, chosen_port, from_nearby};
    use std::net::IpAddr;

    /// The port is what was asked for, or the default.
    #[test]
    fn a_named_port_is_used_and_anything_else_falls_back() {
        assert_eq!(chosen_port(Some("9001")), 9001);
        assert_eq!(chosen_port(Some("  9001  ")), 9001);

        assert_eq!(chosen_port(None), DEFAULT_PORT);
        assert_eq!(chosen_port(Some("")), DEFAULT_PORT);
        assert_eq!(chosen_port(Some("not a port")), DEFAULT_PORT);
        assert_eq!(chosen_port(Some("70000")), DEFAULT_PORT, "beyond a port");

        // Honoured, and the one value nothing real asks for: a port chosen
        // after the fact cannot be written into a container. It exists so that
        // tests running this binary do not contend for one fixed port — with
        // each other, or with an instance somebody is using.
        assert_eq!(chosen_port(Some("0")), 0);
    }

    /// Anything from this machine or its containers may ask; nothing else may.
    #[test]
    fn only_something_on_this_machine_may_ask() {
        for near in [
            "127.0.0.1",
            "::1",
            // The bridge gateway a container arrives from on Linux, and the
            // subnets the common runtimes hand out.
            "172.17.0.1",
            "172.18.0.2",
            "10.88.0.3",
            "192.168.65.1",
            "fd00::1",
            "::ffff:172.17.0.1",
        ] {
            let peer: IpAddr = near.parse().expect("a test address");
            assert!(from_nearby(peer), "{near} is a container or this host");
        }

        for far in ["8.8.8.8", "203.0.113.7", "2606:4700::1111"] {
            let peer: IpAddr = far.parse().expect("a test address");
            assert!(!from_nearby(peer), "{far} came from beyond this machine");
        }
    }
}
