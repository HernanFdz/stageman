//! Where a foreman asks this instance for something.
//!
//! A listener of its own, separate from the dashboard's. Separate because of
//! where it has to bind: the dashboard stays on loopback, and this cannot —
//! see `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
//!
//! **Two things stand between a stranger and creating jobs**, and only the
//! first is a real barrier. The warrant, which nothing but a foreman's
//! container is given. And the peer check below, which refuses anything that
//! did not come from this machine or its containers.
//!
//! It used to be three: this listener served one route and nothing else.
//! `docs/decisions/0034-tools-are-served-not-shipped.md` puts the tools
//! endpoint here too, which is a wider surface behind the same two barriers —
//! that record says why the trade is worth it, and `tooling` is the other
//! route. This one is on its way out with the program that calls it; the
//! barriers are shared rather than duplicated in the meantime.

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

/// Where a container reaches this instance.
///
/// One hostname for both runtimes, which is measured rather than assumed:
/// `--add-host=host.docker.internal:host-gateway` is honoured by Docker and by
/// Podman alike, so nothing here has to know which one is in use.
#[must_use]
pub fn endpoint(port: u16) -> String {
    format!("http://host.docker.internal:{port}/job")
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

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PORT, chosen_port, endpoint, from_nearby};
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

    /// The endpoint names the port it was given, and one hostname.
    #[test]
    fn the_endpoint_is_the_same_hostname_whichever_runtime() {
        assert_eq!(endpoint(9001), "http://host.docker.internal:9001/job");
        assert!(endpoint(DEFAULT_PORT).ends_with("/job"));
    }

    /// An agent a project's jobs may not run on is refused, not substituted.
    ///
    /// Silently running a different agent than the one asked for is a wrong
    /// answer that looks like a right one — the job would run, report success,
    /// and have been done by something the foreman did not choose.
    /// `docs/decisions/0006-agents-are-pluggable.md` makes the choice the
    /// foreman's, so overriding it here would take back a decision that record
    /// gave away.
    #[test]
    fn an_agent_must_be_named_and_allowed_or_it_is_refused() {
        use stageman_core::{Agent, ProjectId, State, Uuid};

        let mut state = State::default();
        let project = ProjectId::from_uuid(Uuid::from_u128(1));
        state.projects.insert(project, watched_by([Agent::Claude]));

        assert_eq!(
            super::named_agent(&state, project, "claude"),
            Some(Agent::Claude)
        );
        assert_eq!(
            super::named_agent(&state, project, "gpt"),
            None,
            "an agent this instance does not run is a refusal, not a substitution"
        );
        assert_eq!(
            super::named_agent(&state, project, ""),
            None,
            "and naming nothing is not naming the first"
        );
        assert_eq!(
            super::named_agent(&state, ProjectId::from_uuid(Uuid::from_u128(9)), "claude"),
            None,
            "a project this instance does not watch has no agents"
        );

        // A refusal says what could have been said instead, so a foreman that
        // guessed wrong learns the set rather than only that it was wrong.
        assert_eq!(super::allowed_agents(&state, project), vec!["claude"]);
        assert!(super::allowed_agents(&state, ProjectId::from_uuid(Uuid::from_u128(9))).is_empty());
    }

    /// A project running jobs on exactly these agents.
    fn watched_by<const N: usize>(agents: [stageman_core::Agent; N]) -> stageman_core::Project {
        use std::collections::{BTreeMap, BTreeSet};

        stageman_core::Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_agent: stageman_core::Agent::Claude,
            job_agents: BTreeSet::from(agents),
            credentials: BTreeMap::new(),
            channels: BTreeMap::new(),
            warrant: None,
            attending: stageman_core::Attending::default(),
            jobs: BTreeMap::new(),
        }
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

/// What a foreman sends to ask for a job.
#[derive(serde::Deserialize)]
pub struct Asking {
    /// What proves the asking came from a foreman this instance started.
    warrant: String,
    /// Why this job should exist, in the foreman's words.
    reason: String,
    /// What the job's agent is to do.
    work: String,
    /// Which agent runs it.
    ///
    /// **Required, with no default.** `docs/decisions/0006-agents-are-pluggable.md`
    /// makes this the foreman's judgement, and a default would take it back —
    /// silently, by choosing whichever agent happened to sort first. The
    /// foreman is told the set it may pick from on every turn, so it always
    /// knows what it is choosing between.
    ///
    /// Optional *here* and refused below, which is not the same as defaulted.
    /// A body without it is the signature of a container built from an image
    /// older than this instance, and letting the field be absent is what makes
    /// it possible to say so. Left required, the framework refuses the body
    /// before any of this runs and answers a bare 422 — which a foreman cannot
    /// act on, and which one duly explained to a person by inventing a rule
    /// about jobs running one at a time.
    agent: Option<String>,
}

/// What it gets back.
#[derive(serde::Serialize)]
pub struct Started {
    /// What names the job, so the foreman can speak about it afterwards.
    job: String,
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
) -> std::io::Result<()> {
    use axum::routing::post;

    let router = axum::Router::new()
        .route("/job", post(asked))
        .route("/mcp", crate::tooling::served())
        .layer(axum::Extension(store));

    axum::serve(
        listening,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
}

/// Creates a job, if whoever asked is allowed to.
#[mutants::skip]
async fn asked(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    axum::Extension(store): axum::Extension<std::sync::Arc<crate::Store>>,
    axum::Json(asking): axum::Json<Asking>,
) -> Result<axum::Json<Started>, (axum::http::StatusCode, String)> {
    if !from_nearby(peer.ip()) {
        tracing::warn!(%peer, "a job was asked for from beyond this machine");
        return Err((axum::http::StatusCode::FORBIDDEN, String::new()));
    }

    let project = {
        let state = store.read();
        let found = state.warranted(&asking.warrant);
        drop(state);
        found
    };
    let Some(project) = project else {
        // Deliberately the same answer as a bad peer, and deliberately without
        // detail: anything that distinguishes "no such warrant" from "not
        // allowed" is something to guess against.
        tracing::warn!(%peer, "a job was asked for with a warrant this instance does not hold");
        return Err((axum::http::StatusCode::FORBIDDEN, String::new()));
    };

    let Some(asked_for) = asking.agent.as_deref() else {
        tracing::warn!(
            %project,
            "a job was asked for without naming an agent, which means an image older than this instance"
        );
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "no agent was named. This container was built from an image older than this \
             instance of stageman and cannot start jobs; it has to be recreated."
                .to_owned(),
        ));
    };

    let agent = named_agent(&store.read(), project, asked_for).ok_or_else(|| {
        let allowed = allowed_agents(&store.read(), project).join(", ");
        tracing::warn!(%project, asked = %asked_for, "no such agent for this project");
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("this project's jobs do not run on {asked_for}. It runs jobs on: {allowed}"),
        )
    })?;

    let started =
        crate::begin(&store, project, agent, &asking.reason, &asking.work).map_err(|why| {
            tracing::warn!(%project, %why, "the job could not be recorded");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, String::new())
        })?;
    let job = started.job();

    // Answered as soon as the job exists, and supervised on a task of its own:
    // a foreman waiting for a job to finish would be a foreman that cannot
    // answer anything else meanwhile, and jobs take minutes.
    let running = std::sync::Arc::clone(&store);
    tokio::spawn(async move {
        drop(crate::supervise(&running, &crate::RUNTIME, started).await);
    });

    Ok(axum::Json(Started {
        job: job.to_string(),
    }))
}

/// The agent a foreman named, if this project's jobs may run on it.
///
/// Answers `None` for an agent this instance does not run *and* for one it
/// runs but this project does not allow — which is a refusal rather than a
/// substitution, because silently running a different agent than the one asked
/// for is a wrong answer that looks like a right one.
pub fn named_agent(
    state: &stageman_core::State,
    project: stageman_core::ProjectId,
    named: &str,
) -> Option<stageman_core::Agent> {
    state
        .projects
        .get(&project)?
        .job_agents
        .iter()
        .find(|agent| crate::dashboard::wire_name(**agent).0 == named)
        .copied()
}

/// What this project's jobs may run on, as a foreman names them.
///
/// Said back with a refusal, so a foreman that guessed wrong is told what it
/// could have said rather than only that it was wrong.
pub fn allowed_agents(
    state: &stageman_core::State,
    project: stageman_core::ProjectId,
) -> Vec<&'static str> {
    state
        .projects
        .get(&project)
        .map_or_else(Vec::new, |watched| {
            watched
                .job_agents
                .iter()
                .map(|agent| crate::dashboard::wire_name(*agent).0)
                .collect()
        })
}
