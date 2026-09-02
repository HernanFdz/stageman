//! Where a job shows its work, and how a request reaches it.
//!
//! `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md` gives every
//! job's container one published port, decided when that container is created
//! because no runtime can add one afterwards. This is the other end: the
//! hostname a person visits, and the forwarding that turns it into a
//! connection to that container.
//!
//! **Deciding is split from serving, as everywhere else here.** [`decode`]
//! turns one hostname into what it means and is pure; [`address`] builds what
//! an agent is told and is pure; the middleware below only performs it. That
//! split is what lets the interesting half be tested without a container.
//!
//! **Nothing here is persisted, and that is the design rather than an
//! omission.** The container-side port is a constant, the host-side port is
//! asked of the runtime because the two runtimes disagree about whether it
//! survives a restart, and which jobs have a tunnel is which jobs have a
//! container. There is no map to keep in step with anything.
//!
//! **This project authenticates none of it.** Whatever forwards the domain
//! authenticates every host under it — see the record above, which is where
//! that reasoning lives, because it is a decision rather than a detail.

// Through the framework's re-export rather than a direct dependency, for the
// reason `endpoint`, `serving` and `tooling` do the same: the version that
// matters is whichever one the framework serves with, and naming it twice is
// how the two drift.
use dioxus::server::axum;
use dioxus::server::axum::response::IntoResponse as _;
use stageman_core::JobId;

/// What names the domain this instance answers on.
const DOMAIN_VARIABLE: &str = "STAGEMAN_DOMAIN";

/// The domain assumed when the environment names none.
///
/// Honest on the machine this runs on and a trap on one it is deployed to,
/// which is why the domain in use is printed at startup: an instance that
/// nobody told a domain tells people to look at `<job>.localhost`, and their
/// browser resolves that to their own loopback rather than to this. That fails
/// as a wrong answer rather than as an absent feature, so it has to be visible
/// at boot.
///
/// Chosen as the default rather than requiring one because the alternative
/// makes a deployment fact mandatory for a daemon on somebody's own machine,
/// and `docs/decisions/0021-an-instance-starts-empty.md` is the shape of
/// argument against that. Measured before it was relied on: a name under this
/// one resolves through the system resolver rather than through a browser's
/// courtesy, so it works in whatever the operator already has open.
const DEFAULT_DOMAIN: &str = "localhost";

/// The longest a hostname may be, in the protocol that has to carry it.
const HOSTNAME_LIMIT: usize = 253;

/// The longest one label of a hostname may be.
const LABEL_LIMIT: usize = 63;

/// The domain this instance answers on.
///
/// Read once per process, like the runtime and the job endpoint's port. A
/// value that changed underneath a running daemon would leave a job whose
/// agent has already told somebody where to look pointing at a name that no
/// longer reaches anything.
pub static DOMAIN: std::sync::LazyLock<Domain> =
    std::sync::LazyLock::new(|| chosen_domain(std::env::var(DOMAIN_VARIABLE).ok().as_deref()));

/// What this process has learned about where each job's tunnel is reachable.
///
/// **Deliberately not persisted, and deliberately not authoritative.** The
/// runtime is what knows: Docker assigns a new host port every time a
/// container starts and Podman keeps the old one, both measured, so a value
/// remembered across a restart is right on one runtime and wrong on the other.
/// This is a cache in front of a subprocess, nothing more, and it is allowed
/// to be stale because [`forward`] treats a refused connection as a reason to
/// ask again rather than as a failure.
pub static TUNNELS: std::sync::LazyLock<std::sync::Arc<Tunnels>> =
    std::sync::LazyLock::new(std::sync::Arc::default);

/// The domain an instance answers on.
///
/// A type rather than a `String` because every use of it is a comparison
/// against a hostname somebody else wrote, and the ways those two fail to
/// match are all invisible: a scheme somebody pasted in, a trailing dot a
/// resolver adds, the case a browser does not preserve. Normalising once, on
/// the way in, is the only place that can be got right — and the failure it
/// prevents reads as a permissions problem rather than as a typo, because what
/// happens is that every tunnel request quietly reaches the dashboard instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain(String);

impl Domain {
    /// The domain in `named`, if it is one.
    ///
    /// Lenient about the two things people actually type — a scheme, and a
    /// trailing dot or slash — and strict about everything else. A port is
    /// deliberately *not* accepted: the port belongs to the address a browser
    /// is given, which [`address`] decides, and one written here would end up
    /// in the middle of a hostname comparison where it can only fail.
    #[must_use]
    pub fn parse(named: &str) -> Option<Self> {
        // Lowered first, so that a scheme somebody typed in capitals is still
        // a scheme. Stripping before lowering would leave one in place and
        // refuse the whole value — which falls back silently, where a person
        // who typed it is nowhere near a terminal to be told.
        let lowered = named.trim().to_ascii_lowercase();
        let named = lowered
            .strip_prefix("https://")
            .or_else(|| lowered.strip_prefix("http://"))
            .unwrap_or(&lowered);
        // Everything from the first slash is a path, which a domain does not
        // have. Taken rather than refused because a pasted URL is the ordinary
        // way this arrives wrong.
        let named = named.split('/').next().unwrap_or(named);
        // The root label, which a resolver may add and nothing else uses.
        let named = named.trim_end_matches('.');

        if named.is_empty() || named.len() > HOSTNAME_LIMIT {
            return None;
        }
        if !named.split('.').all(is_label) {
            return None;
        }
        Some(Self(named.to_owned()))
    }

    /// The domain as it is compared and printed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this domain is reached without anything in front of it.
    ///
    /// Decides the scheme and whether the port is named, and those travel
    /// together: a real domain is reachable only through the thing forwarding
    /// it, which terminates TLS on the standard port because it is also what
    /// authenticates. `localhost` is the one case with nothing in front, so it
    /// is the one case that is plain and carries this process's own port.
    #[must_use]
    fn is_local(&self) -> bool {
        self.0 == DEFAULT_DOMAIN || self.0.ends_with(&format!(".{DEFAULT_DOMAIN}"))
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether one dot-separated part of a hostname is a legal label.
///
/// The rule a certificate authority and a resolver both apply, written out
/// rather than approximated with a character class: a label that starts or
/// ends with a hyphen is refused by both, and one that is merely rejected
/// later produces a domain this instance believes in and nothing else does.
fn is_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= LABEL_LIMIT
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// Which domain to answer on, given what the environment said.
///
/// Anything unreadable falls back rather than failing, for the reason the job
/// endpoint's port does the same: a mistyped value should not stop an instance
/// starting, because a tunnel is not what an operator came for. It is reported
/// at startup either way, so the fallback is visible rather than silent — and
/// here that matters more than it does for a port, because the fallback is a
/// working domain rather than an obviously wrong one.
fn chosen_domain(named: Option<&str>) -> Domain {
    named
        .and_then(Domain::parse)
        .unwrap_or_else(|| Domain(DEFAULT_DOMAIN.to_owned()))
}

/// The port this instance's dashboard was actually bound to.
///
/// Set once, where the listener is bound, because a port of zero is an
/// ordinary request for whichever one is free and the answer is only known
/// there. Read here so that what a job is told to show somebody names the port
/// in use rather than the one that was asked for.
pub static SERVING: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

/// Where a person is told to look, for one job of this instance.
///
/// The impure half, kept to one line so that [`address`] — which is the part
/// with anything to get wrong — stays testable without an environment.
///
/// Falls back to the configured port when nothing has bound one yet. That is
/// the right answer rather than a substituted default: before a listener
/// exists the configured port is what one will be asked for, and the only
/// caller that can run this early is a test.
#[must_use]
pub fn showing(job: JobId) -> String {
    let serving = SERVING
        .get()
        .copied()
        .unwrap_or_else(|| dioxus::cli_config::fullstack_address_or_localhost().port());
    address(&DOMAIN, job, serving)
}

/// Where a person is told to look, for one job.
///
/// Pure, so the string an agent is handed is asserted in the gate rather than
/// read out of a container that ran. The port is named only for a local
/// domain, because that is the only case where this process is what a browser
/// reaches: anything else arrives through the thing forwarding the domain,
/// which listens where a browser looks by default.
#[must_use]
pub fn address(domain: &Domain, job: JobId, serving: u16) -> String {
    if domain.is_local() {
        format!("http://{}.{domain}:{serving}", job.as_uuid())
    } else {
        format!("https://{}.{domain}", job.as_uuid())
    }
}

/// What one hostname on this instance means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// The instance itself: its dashboard, and everything the framework
    /// serves.
    Dashboard,
    /// One job's tunnel.
    Job(JobId),
    /// A name under this domain that identifies no job.
    ///
    /// Distinguished from the dashboard rather than folded into it, because
    /// the two mean opposite things to whoever is looking: somebody who
    /// reached the dashboard by asking for a subdomain has been silently sent
    /// somewhere else, and the page they get looks like a working answer to a
    /// question they did not ask.
    Stranger,
}

/// What a request's hostname means, without performing any of it.
///
/// Pure, and the reason this file is testable at all.
///
/// **The `Host` header is authoritative**, and a proxy in front of this has to
/// preserve it. That is a contract rather than a preference, and it is the
/// thing most likely to be got wrong when this is deployed: a proxy that
/// rewrites the header sends every tunnel request to the dashboard, so a
/// person sees this instance where they expected an application and nothing
/// anywhere says why. A forwarded header is deliberately not consulted — it is
/// supplied by whoever is calling, and routing on something a caller controls
/// is a different decision than this one.
#[must_use]
pub fn decode(host: &str, domain: &Domain) -> Routed {
    let named = hostname(host);
    let Some(under) = named.strip_suffix(domain.as_str()) else {
        return Routed::Dashboard;
    };
    // Nothing left means the domain itself. A remainder not ending in a dot
    // means a longer name that merely ends in these characters, which is
    // somebody else's.
    let Some(label) = under.strip_suffix('.') else {
        return Routed::Dashboard;
    };
    // One level, because one level is what a wildcard covers — both in the
    // forwarding rule and in the certificate. A deeper name reaches neither,
    // and needs no check of its own: a label with a dot in it is not an
    // identifier, so it falls out below with everything else that is not one.
    stageman_core::Uuid::parse_str(label)
        .ok()
        .map_or(Routed::Stranger, |job| Routed::Job(JobId::from_uuid(job)))
}

/// The bare hostname in a `Host` header.
///
/// A port if the browser was given one, brackets if the name is a literal
/// address, a trailing dot if something was pedantic, and whatever case
/// somebody typed. All four are removed here so that [`decode`] compares two
/// values that were normalised the same way.
fn hostname(host: &str) -> String {
    let host = host.trim();
    // A literal IPv6 address arrives in brackets, and the colons inside it
    // would otherwise be read as the start of a port.
    let bracketed = host
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(inside, _)| inside);
    let bare = bracketed.unwrap_or_else(|| host.split_once(':').map_or(host, |(name, _)| name));
    bare.trim_end_matches('.').to_ascii_lowercase()
}

/// Where each job's tunnel was last found.
///
/// One entry per job anybody has looked at, which is bounded by the jobs that
/// exist rather than by requests: a second look at the same job replaces its
/// entry instead of adding one.
#[derive(Debug, Default)]
pub struct Tunnels(parking_lot::Mutex<std::collections::HashMap<JobId, u16>>);

impl Tunnels {
    /// Where this job's tunnel was last found, if anything has looked.
    #[must_use]
    pub fn known(&self, job: JobId) -> Option<u16> {
        self.0.lock().get(&job).copied()
    }

    /// Records where a job's tunnel is now.
    pub fn remember(&self, job: JobId, port: u16) {
        self.0.lock().insert(job, port);
    }

    /// Forgets a job's tunnel, so the next look asks the runtime again.
    pub fn forget(&self, job: JobId) {
        self.0.lock().remove(&job);
    }
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
            (
                axum::http::StatusCode::NOT_FOUND,
                "No job answers on this address.",
            )
                .into_response()
        }
        Routed::Job(job) => forward(job, request).await,
    }
}

/// Forwards one request into a job's container, and back.
///
/// The connection is made per request rather than pooled, which costs a
/// handshake to loopback and buys the thing that matters: an upgrade owns its
/// connection for the rest of its life, so a pool would either refuse to give
/// one up or hand a websocket's bytes to somebody else's request.
#[mutants::skip]
async fn forward(job: JobId, request: axum::extract::Request) -> axum::response::Response {
    let container = stageman_job::container(job);
    let Some(port) = reachable(job, &container).await else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            "No job answers on this address.",
        )
            .into_response();
    };

    match relay(port, request).await {
        Ok(response) => response,
        Err(why) => {
            // Forgotten rather than merely reported: the ordinary cause is a
            // container that was restarted and is now on another host port,
            // which is what Docker does on every start. The next request asks
            // the runtime again and finds it.
            TUNNELS.forget(job);
            tracing::debug!(%job, %port, %why, "a job's tunnel did not answer");
            (
                axum::http::StatusCode::BAD_GATEWAY,
                "This job is not showing anything right now.",
            )
                .into_response()
        }
    }
}

/// Which host port this job's tunnel is on, asking the runtime if need be.
#[mutants::skip]
async fn reachable(job: JobId, container: &str) -> Option<u16> {
    if let Some(known) = TUNNELS.known(job) {
        return Some(known);
    }
    match stageman_agent::tunnel_port(&crate::RUNTIME, container).await {
        Ok(Some(port)) => {
            TUNNELS.remember(job, port);
            Some(port)
        }
        // A container that exists with no mapping, or none of that name. Both
        // mean nothing can be reached, and neither is worth a warning: asking
        // about a job that has been retired is an ordinary thing for a stale
        // browser tab to do.
        Ok(None) => None,
        Err(why) => {
            tracing::debug!(%job, %why, "the runtime could not say where a job's tunnel is");
            None
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
    tokio::spawn(connection.with_upgrades());

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
        tokio::spawn(async move {
            let (Ok(upward), Ok(downward)) = tokio::join!(upward, downward) else {
                tracing::debug!(%port, "an upgraded tunnel connection was not established");
                return;
            };
            let mut upward = hyper_util::rt::TokioIo::new(upward);
            let mut downward = hyper_util::rt::TokioIo::new(downward);
            if let Err(why) = tokio::io::copy_bidirectional(&mut upward, &mut downward).await {
                tracing::debug!(%port, %why, "an upgraded tunnel connection ended");
            }
        });
    }

    Ok(response.map(axum::body::Body::new))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_DOMAIN, Domain, Routed, address, chosen_domain, decode};
    use stageman_core::{JobId, Uuid};

    fn a_job() -> JobId {
        JobId::from_uuid(Uuid::from_u128(1))
    }

    fn local() -> Domain {
        Domain(DEFAULT_DOMAIN.to_owned())
    }

    /// The two things people actually type are taken rather than refused.
    #[test]
    fn a_domain_is_normalised_rather_than_demanded_exactly() {
        for named in [
            "Example.Com",
            "https://example.com",
            "http://example.com",
            "example.com.",
            "  example.com  ",
            "https://example.com/",
            // A scheme typed in capitals is still a scheme, and this is where
            // it stops mattering.
            "HTTPS://EXAMPLE.COM",
            "HTTP://Example.Com/",
        ] {
            assert_eq!(
                Domain::parse(named).map(|domain| domain.as_str().to_owned()),
                Some("example.com".to_owned()),
                "{named}",
            );
        }
    }

    /// What is not a hostname is refused, so it can fall back visibly.
    #[test]
    fn a_domain_that_is_not_one_is_refused() {
        for named in [
            "",
            "   ",
            "example..com",
            "-example.com",
            "example-.com",
            "exa mple.com",
            "under_score.com",
            // A port belongs to the address a browser is given, and one here
            // would sit in the middle of a hostname comparison.
            "example.com:8080",
        ] {
            assert_eq!(Domain::parse(named), None, "{named}");
        }
        assert_eq!(
            Domain::parse(&format!("{}.com", "a".repeat(64))),
            None,
            "a label longer than the protocol allows",
        );
    }

    /// The whole name has a limit of its own, and it is not the label's.
    ///
    /// Both sides of it, because a bound only tested from one side is a bound
    /// nothing pins: a limit that was one too small would refuse a name that
    /// is legal everywhere else and look exactly like a typo.
    #[test]
    fn a_domain_is_bounded_by_what_a_hostname_may_be() {
        // Four labels of sixty-three and a fifth taking it to the limit, so
        // every label is legal and only the total is in question.
        let longest = [
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61),
        ]
        .join(".");
        assert_eq!(longest.len(), 253);
        assert_eq!(
            Domain::parse(&longest).map(|domain| domain.as_str().len()),
            Some(253),
            "a name of exactly the limit is legal",
        );

        let longer = format!("{longest}e");
        assert_eq!(longer.len(), 254);
        assert_eq!(Domain::parse(&longer), None, "one past it is not");
    }

    /// What a job is told names that job, whatever the environment says.
    ///
    /// The impure half is one line and this is what pins it: a wrong answer
    /// here is a job telling somebody to look somewhere nothing is, which
    /// nothing else in the system would notice.
    #[test]
    fn what_a_job_is_told_names_that_job() {
        let told = super::showing(a_job());

        assert!(told.contains(&a_job().as_uuid().to_string()), "{told}");
        assert!(told.starts_with("http"), "{told}");
        assert!(told.contains(super::DOMAIN.as_str()), "{told}");
    }

    /// The cache answers what it was told, and forgets when asked.
    ///
    /// Forgetting is the half worth a test: it is what makes a container that
    /// came back on another port reachable again, and a cache that never
    /// forgot would serve one wrong address for the life of the process.
    #[test]
    fn a_remembered_tunnel_can_be_forgotten() {
        let job = JobId::from_uuid(Uuid::from_u128(9));
        let tunnels = super::Tunnels::default();

        assert_eq!(tunnels.known(job), None);
        tunnels.remember(job, 4242);
        assert_eq!(tunnels.known(job), Some(4242));
        tunnels.forget(job);
        assert_eq!(tunnels.known(job), None, "or a moved container stays lost");
    }

    /// A mistyped value falls back rather than stopping the instance.
    #[test]
    fn an_unreadable_domain_falls_back_to_the_default() {
        assert_eq!(chosen_domain(None), local());
        assert_eq!(chosen_domain(Some("not a domain")), local());
        assert_eq!(
            chosen_domain(Some("example.com")),
            Domain("example.com".to_owned()),
        );
    }

    /// The domain itself is the dashboard, and a job's name is that job.
    #[test]
    fn a_hostname_says_whether_it_is_the_dashboard_or_a_job() {
        let domain = Domain("example.com".to_owned());
        let job = a_job();
        let named = job.as_uuid().to_string();

        assert_eq!(decode("example.com", &domain), Routed::Dashboard);
        assert_eq!(
            decode(&format!("{named}.example.com"), &domain),
            Routed::Job(job),
        );
    }

    /// Everything a `Host` header carries besides the name is ignored.
    #[test]
    fn a_hostname_is_compared_without_its_port_case_or_root() {
        let domain = Domain("example.com".to_owned());
        let named = a_job().as_uuid().to_string();

        for host in [
            format!("{named}.example.com:8080"),
            format!("{}.EXAMPLE.COM", named.to_uppercase()),
            format!("{named}.example.com."),
            format!("  {named}.example.com  "),
        ] {
            assert_eq!(decode(&host, &domain), Routed::Job(a_job()), "{host}");
        }
    }

    /// A name this instance does not serve is not silently the dashboard.
    ///
    /// The distinction the `Stranger` variant exists for: somebody sent to the
    /// dashboard by asking for a tunnel gets a page that looks like a working
    /// answer to a question they did not ask.
    #[test]
    fn a_name_under_this_domain_that_names_no_job_is_not_the_dashboard() {
        let domain = Domain("example.com".to_owned());
        let named = a_job().as_uuid().to_string();

        assert_eq!(decode("nope.example.com", &domain), Routed::Stranger);
        assert_eq!(
            decode(&format!("deeper.{named}.example.com"), &domain),
            Routed::Stranger,
            "a wildcard covers one level, in the forwarding and in the certificate",
        );
    }

    /// Somebody else's name is somebody else's, including a near miss.
    #[test]
    fn a_hostname_outside_this_domain_is_the_dashboard() {
        let domain = Domain("example.com".to_owned());

        assert_eq!(decode("127.0.0.1", &domain), Routed::Dashboard);
        assert_eq!(decode("elsewhere.test", &domain), Routed::Dashboard);
        assert_eq!(
            decode("notexample.com", &domain),
            Routed::Dashboard,
            "a longer name merely ending in these characters is not under this domain",
        );
        assert_eq!(decode("", &domain), Routed::Dashboard);
    }

    /// The scheme and the port travel together, and the local case is the odd
    /// one because it is the only one with nothing in front of it.
    #[test]
    fn an_address_names_the_port_only_when_nothing_forwards_to_it() {
        let named = a_job().as_uuid().to_string();

        assert_eq!(
            address(&local(), a_job(), 8080),
            format!("http://{named}.localhost:8080"),
        );
        assert_eq!(
            address(&Domain("dev.localhost".to_owned()), a_job(), 3000),
            format!("http://{named}.dev.localhost:3000"),
            "a name under the local one is still reached directly",
        );
        assert_eq!(
            address(&Domain("example.com".to_owned()), a_job(), 8080),
            format!("https://{named}.example.com"),
            "anything forwarded is reached where a browser looks by default",
        );
    }

    /// What a job is told to look at is what this instance routes back.
    ///
    /// The two functions are separately correct and only useful together: an
    /// address nobody can decode is a job telling somebody to visit nothing.
    #[test]
    fn an_address_decodes_back_to_the_job_it_was_built_for() {
        for domain in ["localhost", "example.com", "stageman.example.com"] {
            let domain = Domain::parse(domain).expect("a domain");
            let built = address(&domain, a_job(), 8080);
            let host = built.split("//").nth(1).expect("a scheme and an authority");
            assert_eq!(decode(host, &domain), Routed::Job(a_job()), "{built}");
        }
    }

    /// Everything above is pure, and none of it proves a request arrives.
    ///
    /// These stand a listener up where a container's published port would be
    /// and drive the real layer against it, because the half that fails in
    /// practice is not the deciding — it is a header read from the wrong
    /// place, a body that never gets forwarded, or an upgrade that is answered
    /// and then dropped. Nothing here needs a container: what a container
    /// contributes is a port, and a port is a port.
    mod forwarding {
        use super::super::{DOMAIN, TUNNELS, route};
        use super::a_job;
        use dioxus::server::axum;
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

        /// What the dashboard answers, so that a misrouted request is obvious.
        const DASHBOARD: &str = "DASHBOARD";

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
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(async move {
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
                    });
                }
            });
            port
        }

        /// This instance, with the layer under test in front of a dashboard.
        async fn an_instance_serving() -> u16 {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let router = axum::Router::new()
                .fallback(|| async { DASHBOARD })
                .layer(axum::middleware::from_fn(route));
            tokio::spawn(async move { axum::serve(listener, router).await });
            port
        }

        /// One request, with a `Host` of this test's choosing.
        ///
        /// Written onto the socket rather than built with a client, because
        /// the whole subject is which host header arrives and a client that
        /// helpfully sets its own would be testing the client.
        async fn asked(port: u16, host: &str) -> (tokio::net::TcpStream, String) {
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
            (stream, said)
        }

        /// A request for a job's name reaches that job, and nothing else does.
        ///
        /// Both halves in one test on purpose: a layer that forwarded
        /// everything would pass the first assertion, and one that forwarded
        /// nothing would pass the second.
        #[tokio::test]
        async fn a_request_for_a_job_reaches_that_job_and_the_rest_reach_the_dashboard() {
            let container = a_container_serving("FROM-THE-JOB", false).await;
            TUNNELS.remember(a_job(), container);
            let instance = an_instance_serving().await;

            let named = format!("{}.{}", a_job().as_uuid(), *DOMAIN);
            let (_, answered) = asked(instance, &named).await;
            assert!(answered.contains("FROM-THE-JOB"), "{answered}");
            assert!(
                answered.contains(&named),
                "the host has to survive the trip, or an application cannot \
                 build its own links: {answered}"
            );

            let (_, dashboard) = asked(instance, DOMAIN.as_str()).await;
            assert!(dashboard.contains(DASHBOARD), "{dashboard}");
        }

        /// A name under this domain that is nobody is refused, not answered.
        ///
        /// The failure this prevents is the quiet one: somebody who asked for
        /// a tunnel and is shown the dashboard has been given a page that
        /// looks like a working answer to a question they did not ask.
        #[tokio::test]
        async fn a_name_that_is_no_job_is_refused_rather_than_shown_the_dashboard() {
            let instance = an_instance_serving().await;

            let (_, answered) = asked(instance, &format!("nobody.{}", *DOMAIN)).await;
            assert!(answered.starts_with("HTTP/1.1 404"), "{answered}");
            assert!(!answered.contains(DASHBOARD), "{answered}");
        }

        /// An upgrade is carried through, and both directions keep moving.
        ///
        /// The reason this is worth its length: a proxy that forwards request
        /// and response and stops there answers the handshake perfectly and
        /// then goes silent, which is a page that renders once and never
        /// updates — the exact thing this feature exists to provide.
        #[tokio::test]
        async fn an_upgraded_connection_keeps_carrying_bytes_both_ways() {
            let job = stageman_core::JobId::from_uuid(stageman_core::Uuid::from_u128(2));
            let container = a_container_serving("", true).await;
            TUNNELS.remember(job, container);
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
