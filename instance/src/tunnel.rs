//! Where a job shows its work, as a name: the domain an instance answers on,
//! the address a job is told, and what a hostname arriving here means.
//!
//! All of it is pure. `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`
//! gives every job's container one published port; the forwarding that turns
//! a request into a connection is the world's, and this is the deciding half.
//! So is whether anything is behind that port: the world connects and reads
//! once and says what the port did, and what that means — measured in
//! `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
//! for the proxy a runtime puts in front of a published port — is read here.

use std::time::Duration;

use stageman_core::{JobId, Progress};

use stageman_vocabulary::{Answer, Arrival, Bytes, Effect as Generic, EffectId, Probed, RequestId};

use crate::Effect;
use crate::vocabulary::Speaker;
use crate::{Asked, Command};

/// How long a probe gives a job's tunnel to say something, or to close,
/// before it is taken to be showing something in silence.
///
/// **A budget for the runtime's proxy, not for the network.** The connection
/// is to this machine's own loopback and resolves in microseconds; what takes
/// time is the proxy in front of a published port admitting that there is
/// nothing behind it, which it does by accepting first and closing afterwards
/// — see
/// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`,
/// which measured both runtimes and chose this against the slower with room
/// to spare.
///
/// Being impatient is the expensive direction: a window shorter than that
/// close takes to arrive reads every empty container as one that is showing
/// something, which is the bug this constant exists to avoid.
pub const ANSWERING_WITHIN: Duration = Duration::from_millis(500);

/// Whether anything is behind a probed port, rather than merely in front of
/// it.
///
/// The meaning
/// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
/// measured, and the reason a probe reads as well as connects. A published
/// port is not a bare one: both runtimes put a proxy on the host side, and it
/// accepts every connection to it for as long as the container runs, whether
/// or not anything inside is listening. What the proxy cannot fake is what
/// happens next, and only the first of the three things it can do means
/// nothing is there:
///
/// - **closed at once** — the proxy accepted, found nothing inside to forward
///   to, and hung up. This is every job that never showed anything.
/// - **said something** — plainly serving.
/// - **held open and silent** — also serving, and the case that decides the
///   shape: an HTTP server says nothing at all until it is asked, so a probe
///   that demanded bytes would stop exactly the containers this exists to
///   keep. Treating silence as absence is the failure that looks most like
///   rigour.
///
/// A connection refused outright is the fourth, and means what the first
/// does. It is what an empty port does on a runtime that publishes without a
/// proxy in front, which the record names as the case to revisit for: safe
/// here already, and only the cost changes.
#[must_use]
pub const fn answering(probed: Probed) -> bool {
    matches!(probed, Probed::Spoke | Probed::Silent)
}

/// Which container to stop, now it is known whether a job's tunnel answers.
///
/// Answering means the container is left running for whoever is looking;
/// nothing behind it means the container is stopped, keeping it and the
/// session in it for the next reply.
///
/// It answers with the container to stop rather than stopping one, so the
/// deciding is a function anything can call and the asking stays with the
/// instance, which is what mints an identifier for the answer.
fn halting(job: &JobId, answering: bool) -> Option<String> {
    if answering {
        tracing::info!(
            %job,
            "its container is left running, because something is still answering on its tunnel"
        );
        None
    } else {
        Some(stageman_job::container(job))
    }
}

/// The domain assumed when nothing names one.
///
/// Honest on the machine this runs on and a trap on one it is deployed to,
/// which is why the domain in use is printed at startup. Measured before it
/// was relied on: a name under this one resolves through the system resolver
/// rather than through a browser's courtesy, so it works in whatever the
/// operator already has open.
pub const DEFAULT_DOMAIN: &str = "localhost";

/// The longest a hostname may be, in the protocol that has to carry it.
const HOSTNAME_LIMIT: usize = 253;

/// The longest one label of a hostname may be.
const LABEL_LIMIT: usize = 63;

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
        // a scheme.
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

    /// The default domain, for a world nobody told one.
    #[must_use]
    pub fn local() -> Self {
        Self(DEFAULT_DOMAIN.to_owned())
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
    /// authenticates. The local one is the one case with nothing in front, so
    /// it is the one case that is plain and carries this process's own port.
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

/// Where a person is told to look, for one job.
///
/// The port is named only for a local domain, because that is the only case
/// where this process is what a browser reaches: anything else arrives
/// through the thing forwarding the domain, which listens where a browser
/// looks by default.
#[must_use]
pub fn address(domain: &Domain, job: &JobId, serving: u16) -> String {
    if domain.is_local() {
        format!("http://{job}.{domain}:{serving}")
    } else {
        format!("https://{job}.{domain}")
    }
}

/// Where a person is told to look for the dashboard: the apex of the same
/// domain, on the same terms as a job's address.
#[must_use]
pub fn dashboard(domain: &Domain, serving: u16) -> String {
    if domain.is_local() {
        format!("http://{domain}:{serving}")
    } else {
        format!("https://{domain}")
    }
}

/// What a person sees when the address they used names no job of this
/// instance's, or one that is over.
const NOBODY: &str = "No job answers on this address.";

/// What a person sees when a job's container is there and nothing behind
/// its tunnel answers.
const SHOWING_NOTHING: &str = "This job is not showing anything right now.";

/// What a person sees if the presentation server is not answering, which is
/// this process failing to reach itself.
const DASHBOARD_SILENT: &str = "The dashboard is not answering.";

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

/// What a request's hostname means.
///
/// **The `Host` header is authoritative**, and a proxy in front of this has to
/// preserve it. A proxy that rewrites the header sends every tunnel request to
/// the dashboard, so a person sees this instance where they expected an
/// application and nothing anywhere says why. A forwarded header is
/// deliberately not consulted — it is supplied by whoever is calling, and
/// routing on something a caller controls is a different decision than this
/// one.
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
    // forwarding rule and in the certificate. A label with a dot in it is not
    // a name, so a deeper name falls out below with everything else that is
    // not one. What is one is read by the one grammar a job's name has, per
    // `docs/decisions/0074-a-jobs-identifier-is-its-name.md`, so a stranger
    // is a label no name could be; a name no job has is found out below.
    JobId::parse(label).map_or(Routed::Stranger, Routed::Job)
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

impl crate::Running {
    /// Answers where a job's tunnel is: from what was last found, or from
    /// the runtime once it has been asked.
    ///
    /// A job this instance has no record of, or one that is over, is
    /// answered at once with nothing, without asking the runtime: a retired
    /// job's container is gone by decision, and a stale browser tab asking
    /// after one is ordinary. Requests that arrive while the runtime is being
    /// asked wait for the one answer rather than each asking again.
    /// Somebody visited an address this instance answers on.
    ///
    /// Routed by the name they typed and nothing else: a label under this
    /// instance's domain is a job's tunnel, and everything else is the
    /// dashboard — see
    /// `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`. The
    /// header carries it, which is enough because what serves this speaks
    /// HTTP/1.1, where the authority is always a header.
    pub fn visited(&mut self, id: RequestId, request: &Arrival, effects: &mut Vec<Effect>) {
        let host = request.headers.get("host").map_or("", String::as_str);
        match decode(host, &self.domain) {
            // Two paths under the dashboard's host are the instance's own:
            // where the platform sends the browser back after a
            // registration, answered here before the proxy — see
            // `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
            Routed::Dashboard => {
                if !self.came_back(id, request, effects) {
                    effects.push(Effect::Answer {
                        id,
                        answer: Answer::Proxy {
                            port: self.presenting,
                            refused: Bytes::new(DASHBOARD_SILENT.as_bytes().to_vec()),
                        },
                    });
                }
            }
            Routed::Stranger => {
                tracing::warn!(
                    %host,
                    "a name under this instance's domain identifies no job — the domain may be \
                     set to something other than what is forwarded here"
                );
                Self::nobody(id, effects);
            }
            Routed::Job(job) => self.tunnel_asked(id, job, effects),
        }
    }

    /// Answers whoever asked, now that where a job's tunnel is, is known.
    fn routed(id: RequestId, port: Option<u16>, effects: &mut Vec<Effect>) {
        match port {
            Some(port) => effects.push(Effect::Answer {
                id,
                answer: Answer::Proxy {
                    port,
                    refused: Bytes::new(SHOWING_NOTHING.as_bytes().to_vec()),
                },
            }),
            None => Self::nobody(id, effects),
        }
    }

    /// What a person sees when the address names no job that is showing.
    fn nobody(id: RequestId, effects: &mut Vec<Effect>) {
        effects.push(Effect::Answer {
            id,
            answer: Answer::Respond {
                status: 404,
                headers: [("content-type".to_owned(), "text/plain".to_owned())].into(),
                body: Bytes::new(NOBODY.as_bytes().to_vec()),
            },
        });
    }

    pub fn tunnel_asked(&mut self, id: RequestId, job: JobId, effects: &mut Vec<Effect>) {
        let Some(recorded) = self.state.job(&job) else {
            // The same diagnostic a stranger gets, because since a name is
            // read by a grammar most labels are names, and a name no job
            // has is the likelier sign that the domain is not what is
            // forwarded here.
            tracing::warn!(
                %job,
                "a name under this instance's domain identifies no job — a stale link, or the \
                 domain set to something other than what is forwarded here"
            );
            Self::nobody(id, effects);
            return;
        };
        if matches!(recorded.progress, Progress::Retired(_)) {
            Self::nobody(id, effects);
            return;
        }
        if let Some(port) = self.tunnels.get(&job).copied() {
            Self::routed(id, Some(port), effects);
            return;
        }
        let waiting = self.routing.entry(job.clone()).or_default();
        waiting.push(id);
        if waiting.len() == 1 {
            let looking = self.ask(
                &Command::Port {
                    name: stageman_job::container(&job),
                },
                Asked::Port { job },
            );
            effects.push(looking);
        }
    }

    /// Records where a job's tunnel was found, and answers everyone waiting.
    pub fn port_found(&mut self, job: &JobId, port: Option<u16>, effects: &mut Vec<Effect>) {
        if let Some(port) = port {
            self.tunnels.insert(job.clone(), port);
        }
        for id in self.routing.remove(job).unwrap_or_default() {
            Self::routed(id, port, effects);
        }
    }

    /// Asks whether anything is behind a job's tunnel, at one of the three
    /// moments
    /// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
    /// names.
    ///
    /// Two questions rather than one. The runtime is asked where the tunnel
    /// is published — every time, and never from what was last found,
    /// because the port can have moved under a container somebody else
    /// restarted, and a probe of the old one would find nothing there and
    /// stop a container that is showing something on the new. Then the port
    /// is probed, and what it did is read for what it means.
    pub fn probe(&mut self, job: JobId, effects: &mut Vec<Effect>) {
        let looking = self.ask(
            &Command::Port {
                name: stageman_job::container(&job),
            },
            Asked::Probing { job },
        );
        effects.push(looking);
    }

    /// The runtime said where a job's tunnel is, on the way to probing it.
    ///
    /// Nothing published is nothing to reach, and so is a container the
    /// runtime will not answer about: both are a tunnel with nothing behind
    /// it, and neither is probed.
    pub fn probing(&mut self, job: &JobId, port: Option<u16>, effects: &mut Vec<Effect>) {
        let Some(port) = port else {
            self.tunnel_answered(job, false, effects);
            return;
        };
        let id = self.effect_id();
        self.probes.insert(id, job.clone());
        effects.push(Generic::Probe {
            id,
            port,
            within: ANSWERING_WITHIN,
        });
    }

    /// The world said what a probed port did.
    pub fn probed(&mut self, id: EffectId, probed: Probed, effects: &mut Vec<Effect>) {
        let Some(job) = self.probes.remove(&id) else {
            tracing::warn!("a port was probed that this instance did not ask about; ignored");
            return;
        };
        self.tunnel_answered(&job, answering(probed), effects);
    }

    /// What is done about a job's tunnel, now it is known whether anything
    /// is behind it. Inward-facing, so it waits on no write.
    ///
    /// A turn started for the job since the probe was asked — a message
    /// arriving just after the turn the probe followed ended, which
    /// `docs/decisions/0069-a-message-reaches-a-working-job.md` makes
    /// ordinary — needs the container, so a silent tunnel is not halted
    /// under it: the race
    /// `docs/decisions/0066-a-foremans-container-runs-only-while-a-turn-runs-in-it.md`
    /// admits, closed for a job.
    fn tunnel_answered(&mut self, job: &JobId, answering: bool, effects: &mut Vec<Effect>) {
        if !answering && self.turns.contains_key(&Speaker::Job(job.clone())) {
            tracing::debug!(%job, "its tunnel answers nobody, but a turn has started in it since");
            return;
        }
        if !answering {
            // Halted, so the port it was on reaches nothing.
            self.forget_tunnel(job);
        }
        if let Some(container) = halting(job, answering) {
            let halt = self.ask(
                &Command::Halt {
                    name: container.clone(),
                },
                Asked::Halted { container },
            );
            effects.push(halt);
        }
    }

    /// Forgets where a job's tunnel was, so that a look afterwards asks the
    /// runtime rather than trusting a port that has moved.
    ///
    /// At every moment the container is stopped, restarted or removed, and
    /// at every probe that finds nothing answering. It used to be forgotten
    /// on a failed forward too, which is no longer something this hears
    /// about: the world answers a forward to nothing itself, with the words
    /// it was given. What that costs is the case where a container was
    /// restarted by somebody else and moved — and the settling probe finds
    /// that within its interval, which is the same recovery arriving a
    /// minute later rather than a new one being needed.
    pub fn forget_tunnel(&mut self, job: &JobId) {
        self.tunnels.remove(job);
    }
}

#[cfg(test)]
mod tests {
    use super::{Domain, Probed, Routed, address, answering, dashboard, decode, halting};
    use stageman_core::{JobId, Uuid};

    fn a_job() -> JobId {
        JobId::from_uuid(Uuid::from_u128(1))
    }

    /// What a port did is read for whether anything is behind it, and the
    /// case that decides the shape is the one where it said nothing.
    #[test]
    fn a_port_that_refused_or_closed_has_nothing_behind_it_and_one_held_open_has() {
        assert!(!answering(Probed::Refused), "nothing accepted");
        assert!(!answering(Probed::Closed), "the proxy accepted for nobody");
        assert!(answering(Probed::Spoke));
        assert!(
            answering(Probed::Silent),
            "an HTTP server says nothing until it is asked"
        );
    }

    /// The whole of what deciding a container's life looks like from here:
    /// answering keeps it, silence stops it.
    #[test]
    fn a_tunnel_that_answers_keeps_its_container_and_silence_stops_it() {
        let job = JobId::from_uuid(Uuid::from_u128(5));

        assert_eq!(halting(&job, true), None, "answering keeps the container");
        assert_eq!(
            halting(&job, false),
            Some(stageman_job::container(&job)),
            "silence stops it, and that container and no other"
        );
    }

    /// The dashboard is told the way a job's address is: on the port for a
    /// local domain, where this process is what a browser reaches, and on
    /// the domain alone otherwise, where whatever forwards it listens.
    #[test]
    fn the_dashboard_is_told_on_the_same_terms_as_a_jobs_address() {
        assert_eq!(dashboard(&Domain::local(), 8080), "http://localhost:8080");
        let forwarded = Domain::parse("stageman.example.com").expect("a domain");
        assert_eq!(dashboard(&forwarded, 8080), "https://stageman.example.com");
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
    /// nothing pins.
    #[test]
    fn a_domain_is_bounded_by_what_a_hostname_may_be() {
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
        assert_eq!(Domain::parse(&longer), None, "one past it is not");
    }

    /// The domain itself is the dashboard, and a job's name is that job.
    #[test]
    fn a_hostname_says_whether_it_is_the_dashboard_or_a_job() {
        let domain = Domain::parse("example.com").expect("a domain");
        let named = a_job().to_string();

        assert_eq!(decode("example.com", &domain), Routed::Dashboard);
        assert_eq!(
            decode(&format!("{named}.example.com"), &domain),
            Routed::Job(a_job()),
        );
    }

    /// Everything a `Host` header carries besides the name is ignored.
    #[test]
    fn a_hostname_is_compared_without_its_port_case_or_root() {
        let domain = Domain::parse("example.com").expect("a domain");
        let named = a_job().to_string();

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
    /// Since `docs/decisions/0074-a-jobs-identifier-is-its-name.md` a label
    /// that fits a name's grammar is read as a name and found to belong to
    /// no job further on; a stranger is a label no name could be.
    #[test]
    fn a_name_under_this_domain_that_names_no_job_is_not_the_dashboard() {
        let domain = Domain::parse("example.com").expect("a domain");
        let named = a_job().to_string();

        assert_eq!(
            decode("nope.example.com", &domain),
            Routed::Job(JobId::parse("nope").expect("a name")),
            "a well-formed label is a name, whether or not a job has it"
        );
        assert_eq!(decode("not_a_name.example.com", &domain), Routed::Stranger);
        assert_eq!(
            decode(&format!("deeper.{named}.example.com"), &domain),
            Routed::Stranger,
            "a wildcard covers one level, in the forwarding and in the certificate",
        );
    }

    /// Somebody else's name is somebody else's, including a near miss.
    #[test]
    fn a_hostname_outside_this_domain_is_the_dashboard() {
        let domain = Domain::parse("example.com").expect("a domain");

        assert_eq!(decode("127.0.0.1", &domain), Routed::Dashboard);
        assert_eq!(decode("elsewhere.test", &domain), Routed::Dashboard);
        assert_eq!(
            decode("notexample.com", &domain),
            Routed::Dashboard,
            "a longer name merely ending in these characters is not under this domain",
        );
        assert_eq!(decode("", &domain), Routed::Dashboard);
    }

    /// A bracketed literal address is a hostname too, and its colons are not
    /// a port.
    #[test]
    fn a_bracketed_address_is_read_whole() {
        let domain = Domain::parse("example.com").expect("a domain");

        assert_eq!(decode("[::1]:8080", &domain), Routed::Dashboard);
    }

    /// The scheme and the port travel together, and the local case is the odd
    /// one because it is the only one with nothing in front of it.
    #[test]
    fn an_address_names_the_port_only_when_nothing_forwards_to_it() {
        let named = a_job().to_string();

        assert_eq!(
            address(&Domain::local(), &a_job(), 8080),
            format!("http://{named}.localhost:8080"),
        );
        assert_eq!(
            address(
                &Domain::parse("dev.localhost").expect("a domain"),
                &a_job(),
                3000
            ),
            format!("http://{named}.dev.localhost:3000"),
            "a name under the local one is still reached directly",
        );
        assert_eq!(
            address(
                &Domain::parse("example.com").expect("a domain"),
                &a_job(),
                8080
            ),
            format!("https://{named}.example.com"),
            "anything forwarded is reached where a browser looks by default",
        );
    }

    /// What a job is told to look at is what this instance routes back.
    #[test]
    fn an_address_decodes_back_to_the_job_it_was_built_for() {
        for domain in ["localhost", "example.com", "stageman.example.com"] {
            let domain = Domain::parse(domain).expect("a domain");
            let built = address(&domain, &a_job(), 8080);
            let host = built.split("//").nth(1).expect("a scheme and an authority");
            assert_eq!(decode(host, &domain), Routed::Job(a_job()), "{built}");
        }
    }

    /// The default prints as what it is.
    #[test]
    fn the_local_domain_is_the_default_and_says_so() {
        assert_eq!(Domain::local().to_string(), "localhost");
        assert_eq!(
            Domain::local(),
            Domain::parse("localhost").expect("a domain")
        );
    }
}
