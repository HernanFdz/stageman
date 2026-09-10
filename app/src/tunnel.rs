//! Where a job shows its work, and how a request reaches it.
//!
//! `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` gives every
//! job's container one published port, decided when that container is created
//! because no runtime can add one afterwards. This is the other end: the
//! forwarding that turns a request for a job's name into a connection to that
//! container.
//!
//! **Deciding is the instance's.** What a hostname means is decided by shape,
//! with the instance crate's own `decode`, and only a name one label below
//! the domain reaches the instance at all — as a question, answered with a
//! port or with nothing. Where a job's tunnel was last found is held there
//! too, so nothing here remembers anything: this layer reads a header, asks,
//! and forwards.
//!
//! **This project authenticates none of it.** Whatever forwards the domain
//! authenticates every host under it — see the record above, which is where
//! that reasoning lives, because it is a decision rather than a detail.

use std::future::Future;
use std::pin::Pin;

// Through the framework's re-export rather than a direct dependency, for the
// reason `endpoint`, `serving` and `tooling` do the same: the version that
// matters is whichever one the framework serves with, and naming it twice is
// how the two drift.
use dioxus::server::axum;
use dioxus::server::axum::response::IntoResponse as _;
use stageman_core::JobId;
use stageman_instance::{Domain, Event, Routed, decode};

use crate::world::Located;

/// What names the domain this instance answers on.
const DOMAIN_VARIABLE: &str = "STAGEMAN_DOMAIN";

/// The domain this instance answers on.
///
/// Read once per process, like the runtime and the job endpoint's port: a
/// value that changed underneath a running daemon would leave jobs holding
/// addresses that used to work. The instance is told it on waking, and this
/// is where the world reads it from the environment.
pub static DOMAIN: std::sync::LazyLock<Domain> =
    std::sync::LazyLock::new(|| chosen_domain(std::env::var(DOMAIN_VARIABLE).ok().as_deref()));

/// Which domain to answer on, given what the environment said.
///
/// Anything unreadable falls back rather than failing, for the reason the job
/// endpoint's port does the same: a mistyped value should not stop an instance
/// starting, because a tunnel is not what an operator came for. It is reported
/// at startup either way, so the fallback is visible rather than silent — and
/// here that matters more than it does for a port, because the fallback is a
/// working domain rather than an obviously wrong one.
fn chosen_domain(named: Option<&str>) -> Domain {
    named.and_then(Domain::parse).unwrap_or_else(Domain::local)
}

/// Where a job's tunnel is, as somebody answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    /// Forward to this host port.
    At(u16),
    /// Nothing answers on that name.
    Nowhere,
    /// Nothing can answer at all: this process has no instance.
    Unanswerable,
}

/// Something that can say where a job's tunnel is.
///
/// A function rather than the world itself, so that the layer can be driven
/// against a listener a test stood up without an instance behind it.
type Locator = fn(JobId) -> Pin<Box<dyn Future<Output = Found> + Send>>;

/// Where a job's tunnel is, asked of the instance.
fn asking_the_instance(job: JobId) -> Pin<Box<dyn Future<Output = Found> + Send>> {
    Box::pin(async move {
        let Some(world) = crate::world() else {
            return Found::Unanswerable;
        };
        match world.tunnel(job).await {
            Some(Located::At(port)) => Found::At(port),
            Some(Located::Nowhere) => Found::Nowhere,
            None => Found::Unanswerable,
        }
    })
}

/// Routes one request: to a job's tunnel, or on to the dashboard.
///
/// A layer rather than a route, because what decides is the hostname and the
/// framework's router owns every path. It has to run outside that router: a
/// tunnel serves an application somebody else's agent wrote, and putting that
/// through the server-function and static-file machinery would let a path
/// collision decide which one answers.
pub async fn route(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    forwarded(request, next, asking_the_instance).await
}

/// Routes one request, asking `locate` where a job is.
///
/// **The `Host` header is authoritative**, and a proxy in front of this has to
/// preserve it. A proxy that rewrites the header sends every tunnel request to
/// the dashboard, so a person sees this instance where they expected an
/// application and nothing anywhere says why. A forwarded header is
/// deliberately not consulted — it is supplied by whoever is calling, and
/// routing on something a caller controls is a different decision than this
/// one.
async fn forwarded(
    request: axum::extract::Request,
    next: axum::middleware::Next,
    locate: Locator,
) -> axum::response::Response {
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        // HTTP/2 carries the authority in the target rather than in a header,
        // and a client that speaks it to this process directly is not
        // hypothetical once something is forwarding to it.
        .or_else(|| request.uri().host().map(str::to_owned))
        .unwrap_or_default();

    match decode(&host, &DOMAIN) {
        Routed::Dashboard => next.run(request).await,
        Routed::Stranger => {
            tracing::warn!(
                %host,
                "a name under this instance's domain identifies no job — the domain may be \
                 set to something other than what is forwarded here"
            );
            nobody()
        }
        Routed::Job(job) => forward(job, request, locate).await,
    }
}

/// What a name nothing answers on is told.
fn nobody() -> axum::response::Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        "No job answers on this address.",
    )
        .into_response()
}

/// Forwards one request into a job's container, and back.
///
/// The connection is made per request rather than pooled, which costs a
/// handshake to loopback and buys the thing that matters: an upgrade owns its
/// connection for the rest of its life, so a pool would either refuse to give
/// one up or hand a websocket's bytes to somebody else's request.
///
/// A connection that does not go through is reported to the instance, which
/// forgets where it found the tunnel: the ordinary cause is a container that
/// was restarted and is now on another host port, which is what the runtime
/// does on every start, and the next request asks again and finds it.
#[mutants::skip]
async fn forward(
    job: JobId,
    request: axum::extract::Request,
    locate: Locator,
) -> axum::response::Response {
    let port = match locate(job).await {
        Found::At(port) => port,
        Found::Nowhere => return nobody(),
        Found::Unanswerable => {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    match relay(port, request).await {
        Ok(response) => response,
        Err(why) => {
            tracing::debug!(%job, %port, %why, "a job's tunnel did not answer");
            if let Some(world) = crate::world() {
                world.send(Event::TunnelFailed {
                    job,
                    why: why.to_string(),
                });
            }
            (
                axum::http::StatusCode::BAD_GATEWAY,
                "This job is not showing anything right now.",
            )
                .into_response()
        }
    }
}

/// Speaks HTTP to a published port and hands back what it said.
///
/// Upgrades are carried through in both directions. Both handles are taken
/// before anything is awaited on them, because an upgrade is only available
/// until the message it belongs to is consumed — and once the response is
/// returned, this function no longer has the request to take one from.
async fn relay(
    port: u16,
    mut request: axum::extract::Request,
) -> Result<axum::response::Response, Box<dyn std::error::Error + Send + Sync>> {
    let upward = request
        .extensions_mut()
        .remove::<hyper::upgrade::OnUpgrade>();

    let stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await?;
    // With upgrades, so that a 101 leaves the connection available to be taken
    // over rather than closed underneath it.
    drop(tokio::spawn(connection.with_upgrades()));

    // Whatever framing the incoming request carried is the framing of a body
    // this process has already decoded, and the client re-derives it from what
    // it is given. Left in place, the two disagree and the request is refused.
    request
        .headers_mut()
        .remove(axum::http::header::TRANSFER_ENCODING);

    let mut response = sender.send_request(request).await?;

    if response.status() == axum::http::StatusCode::SWITCHING_PROTOCOLS
        && let Some(upward) = upward
    {
        let downward = hyper::upgrade::on(&mut response);
        // On a task of its own, because it lives as long as the websocket
        // does and the response has to be returned now for the handshake to
        // complete at all.
        drop(tokio::spawn(async move {
            let (Ok(upward), Ok(downward)) = tokio::join!(upward, downward) else {
                tracing::debug!(%port, "an upgraded tunnel connection was not established");
                return;
            };
            let mut upward = hyper_util::rt::TokioIo::new(upward);
            let mut downward = hyper_util::rt::TokioIo::new(downward);
            if let Err(why) = tokio::io::copy_bidirectional(&mut upward, &mut downward).await {
                tracing::debug!(%port, %why, "an upgraded tunnel connection ended");
            }
        }));
    }

    Ok(response.map(axum::body::Body::new))
}

#[cfg(test)]
mod tests {
    use super::{Domain, chosen_domain};

    /// A mistyped value falls back rather than stopping the instance.
    #[test]
    fn an_unreadable_domain_falls_back_to_the_default() {
        assert_eq!(chosen_domain(None), Domain::local());
        assert_eq!(chosen_domain(Some("not a domain")), Domain::local());
        assert_eq!(
            chosen_domain(Some("example.com")),
            Domain::parse("example.com").expect("a domain"),
        );
    }

    /// Everything the instance decides is pure and tested there, and none of
    /// it proves a request arrives.
    ///
    /// These stand a listener up where a container's published port would be
    /// and drive the real layer against it, because the half that fails in
    /// practice is not the deciding — it is a header read from the wrong
    /// place, a body that never gets forwarded, or an upgrade that is answered
    /// and then dropped. Nothing here needs a container: what a container
    /// contributes is a port, and a port is a port. Nor an instance: where a
    /// job is comes from a table this test writes.
    mod forwarding {
        use super::super::{DOMAIN, Found, forwarded};
        use dioxus::server::axum;
        use stageman_core::{JobId, Uuid};
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

        /// What the dashboard answers, so that a misrouted request is obvious.
        const DASHBOARD: &str = "DASHBOARD";

        /// Where each job's tunnel is, as this test says.
        static TABLE: std::sync::LazyLock<
            parking_lot::Mutex<std::collections::HashMap<JobId, u16>>,
        > = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

        fn from_the_table(
            job: JobId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Found> + Send>> {
            let found = TABLE
                .lock()
                .get(&job)
                .map_or(Found::Nowhere, |port| Found::At(*port));
            Box::pin(async move { found })
        }

        /// Reads one request's headers and nothing more.
        ///
        /// Enough to know a request arrived and to reply to it. The body is
        /// deliberately not read: these assert on what came back.
        async fn request_read(
            reader: &mut tokio::io::BufReader<tokio::net::TcpStream>,
        ) -> Vec<String> {
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                if line.trim().is_empty() {
                    break;
                }
                lines.push(line.trim().to_owned());
            }
            lines
        }

        /// A listener answering as a job's own server would, on loopback.
        ///
        /// Raw rather than built with the framework, so that what goes over
        /// the socket is exactly what this test says and an upgrade can be
        /// answered without a websocket library agreeing to it.
        async fn a_container_serving(said: &'static str, upgrading: bool) -> u16 {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("a port");
            let port = listener.local_addr().expect("an address").port();
            drop(tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    drop(tokio::spawn(async move {
                        let mut reader = tokio::io::BufReader::new(stream);
                        let asked = request_read(&mut reader).await;
                        let answer = if upgrading {
                            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: probe\r\n\
                             Connection: Upgrade\r\n\r\n"
                                .to_owned()
                        } else {
                            // The host it was asked for comes back in the
                            // body, which is how the test sees whether the
                            // header survived the trip.
                            let host = asked
                                .iter()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("host: ")
                                        .map(str::to_owned)
                                })
                                .unwrap_or_default();
                            let body = format!("{said}|{host}");
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            )
                        };
                        let stream = reader.get_mut();
                        if stream.write_all(answer.as_bytes()).await.is_err() {
                            return;
                        }
                        if !upgrading {
                            return;
                        }
                        // Past the upgrade, everything is bytes. Echoed back
                        // so the test can prove both directions still move.
                        let mut buffer = [0_u8; 64];
                        while let Ok(read) =
                            tokio::io::AsyncReadExt::read(stream, &mut buffer).await
                        {
                            if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                                return;
                            }
                        }
                    }));
                }
            }));
            port
        }

        /// This instance, with the layer under test in front of a dashboard.
        async fn an_instance_serving() -> u16 {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let router = axum::Router::new().fallback(|| async { DASHBOARD }).layer(
                axum::middleware::from_fn(|request, next| forwarded(request, next, from_the_table)),
            );
            drop(tokio::spawn(
                async move { axum::serve(listener, router).await },
            ));
            port
        }

        /// One request, with a `Host` of this test's choosing.
        ///
        /// Written onto the socket rather than built with a client, because
        /// the whole subject is which host header arrives and a client that
        /// helpfully sets its own would be testing the client.
        async fn asked(port: u16, host: &str) -> String {
            let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .expect("the instance");
            stream
                .write_all(
                    format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .expect("a request");
            let mut said = String::new();
            tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut said)
                .await
                .expect("an answer");
            said
        }

        /// A request for a job's name reaches that job, and nothing else does.
        ///
        /// Both halves in one test on purpose: a layer that forwarded
        /// everything would pass the first assertion, and one that forwarded
        /// nothing would pass the second.
        #[tokio::test]
        async fn a_request_for_a_job_reaches_that_job_and_the_rest_reach_the_dashboard() {
            let job = JobId::from_uuid(Uuid::from_u128(1));
            let container = a_container_serving("FROM-THE-JOB", false).await;
            TABLE.lock().insert(job, container);
            let instance = an_instance_serving().await;

            let named = format!("{}.{}", job.as_uuid(), *DOMAIN);
            let answered = asked(instance, &named).await;
            assert!(answered.contains("FROM-THE-JOB"), "{answered}");
            assert!(
                answered.contains(&named),
                "the host has to survive the trip, or an application cannot \
                 build its own links: {answered}"
            );

            let dashboard = asked(instance, DOMAIN.as_str()).await;
            assert!(dashboard.contains(DASHBOARD), "{dashboard}");
        }

        /// A name under this domain that is nobody is refused, not answered,
        /// and so is a job's name nothing answers on.
        ///
        /// The failure this prevents is the quiet one: somebody who asked for
        /// a tunnel and is shown the dashboard has been given a page that
        /// looks like a working answer to a question they did not ask.
        #[tokio::test]
        async fn a_name_that_is_no_job_is_refused_rather_than_shown_the_dashboard() {
            let instance = an_instance_serving().await;

            let answered = asked(instance, &format!("nobody.{}", *DOMAIN)).await;
            assert!(answered.starts_with("HTTP/1.1 404"), "{answered}");
            assert!(!answered.contains(DASHBOARD), "{answered}");

            let unknown = JobId::from_uuid(Uuid::from_u128(99));
            let answered = asked(instance, &format!("{}.{}", unknown.as_uuid(), *DOMAIN)).await;
            assert!(answered.starts_with("HTTP/1.1 404"), "{answered}");
        }

        /// An upgrade is carried through, and both directions keep moving.
        ///
        /// The reason this is worth its length: a proxy that forwards request
        /// and response and stops there answers the handshake perfectly and
        /// then goes silent, which is a page that renders once and never
        /// updates — the exact thing this feature exists to provide.
        #[tokio::test]
        async fn an_upgraded_connection_keeps_carrying_bytes_both_ways() {
            let job = JobId::from_uuid(Uuid::from_u128(2));
            let container = a_container_serving("", true).await;
            TABLE.lock().insert(job, container);
            let instance = an_instance_serving().await;

            let mut stream =
                tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, instance))
                    .await
                    .expect("the instance");
            stream
                .write_all(
                    format!(
                        "GET / HTTP/1.1\r\nHost: {}.{}\r\nConnection: Upgrade\r\n\
                         Upgrade: probe\r\n\r\n",
                        job.as_uuid(),
                        *DOMAIN
                    )
                    .as_bytes(),
                )
                .await
                .expect("a request");

            let mut reader = tokio::io::BufReader::new(stream);
            let answered = request_read(&mut reader).await;
            assert!(
                answered.first().is_some_and(|line| line.contains("101")),
                "{answered:?}",
            );

            // The half a handshake alone would not prove.
            let stream = reader.get_mut();
            stream.write_all(b"still-here").await.expect("a write");
            let mut buffer = [0_u8; 10];
            // Bounded, because the failure this is looking for is a
            // connection that was upgraded and then abandoned — which does not
            // error, it simply never says anything again.
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::io::AsyncReadExt::read_exact(stream, &mut buffer),
            )
            .await
            .expect("the echo, rather than a connection nothing is carrying")
            .expect("the echo");
            assert_eq!(&buffer, b"still-here");
        }
    }
}
