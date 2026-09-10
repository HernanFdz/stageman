//! The doing: one isolated workspace, and one agent running inside it with the
//! credentials its project needs.
//!
//! Two invariants in `docs/architecture.md` §2 are this crate's to keep. A job
//! has one workspace and one project, and holds credentials for that project
//! and no other — it can reach neither another job's workspace nor anything
//! belonging to another project. And a job never blocks on a terminal: when it
//! needs a human it asks on a channel and stays alive, because nobody is
//! watching that terminal.
//!
//! How an agent is driven is not this crate's business — that contract lives
//! in `stageman-agent`, and which agent ran a given job is recorded on the job
//! itself. What belongs here is everything around the agent: the workspace it
//! runs in, the credentials it is handed, and the supervision that ends it.

use stageman_agent::{AgentError, ContainerRuntime};
use stageman_core::{JobId, Uuid};

/// What every one of this project's containers is named for.
///
/// A job's container is named from its identifier rather than recorded
/// anywhere, which is what closes the window a stored name would leave open:
/// the name is known before the container exists, so there is no instant at
/// which one is running and nothing can say whose it is. See
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`.
const PREFIX: &str = "stageman-job-";

/// The container a job runs in.
///
/// Total and reversible: every job has exactly one name and every such name
/// says which job it belongs to, which is what lets a sweep work from what the
/// runtime reports rather than from what the instance remembers.
#[must_use]
pub fn container(job: JobId) -> String {
    format!("{PREFIX}{}", job.as_uuid())
}

/// Which job a container belongs to, if its name says so.
///
/// `None` for a container carrying this project's label under a name this
/// version does not understand — an older naming scheme, or something that
/// borrowed the label. Worth distinguishing rather than ignoring: it is still
/// ours to clean up, and it is not ours to resume.
///
/// Public because placing a container is the instance's decision, and a
/// listing reaches it as names.
#[must_use]
pub fn job_of(container: &str) -> Option<JobId> {
    container
        .strip_prefix(PREFIX)
        .and_then(|rest| Uuid::parse_str(rest).ok())
        .map(JobId::from_uuid)
}

/// How long to wait for a job's tunnel to answer before deciding it is not
/// showing anything.
///
/// **This is a budget for the runtime's proxy, not for the network.** The
/// connection itself is to this machine's own loopback and resolves in
/// microseconds; what takes time is the proxy in front of a published port
/// admitting that there is nothing behind it, which it does by accepting first
/// and closing afterwards — see
/// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`.
/// Measured at about two milliseconds on Docker and about two hundred on
/// Podman, so this is generous against the slower of the two rather than
/// against a network.
///
/// Being impatient is the expensive direction, and now more so than before: a
/// window shorter than that close takes to arrive reads every empty container
/// as one that is showing something, which is the bug this constant's users
/// exist to avoid.
const ANSWERING_WITHIN: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether a job's tunnel is answering: something is behind the port the
/// runtime published for it, and so the container is showing something.
///
/// The question `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
/// asks at each of the three moments it names; what to do about the answer is
/// the instance's, which is why this only asks.
///
/// **Answering is asked of the tunnel, not of the container.** A connection is
/// opened to the port the runtime published, from here, and something has to
/// be behind it. Asking from outside is what makes this stricter than looking
/// for a process bound inside: a server an agent bound to its container's own
/// loopback holds the port and is reachable by nobody, and treating it as
/// showing something would keep a container alive for ever to serve no one.
///
/// **Accepting is not being behind it**, and that distinction is the whole of
/// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`.
/// A published port is not a bare one: both runtimes put a proxy on the host
/// side, and it accepts for as long as the container runs, whether or not
/// anything inside is listening. So this reads as well as connects.
///
/// **Total, and that is the honest signature rather than a convenience.** Every
/// way this can go wrong means the same thing: a container that is gone, one
/// that never had a mapping, and one the runtime will not answer questions
/// about are all showing nothing. A runtime broken badly enough to matter
/// fails loudly everywhere else in the same breath.
#[mutants::skip]
pub async fn answering(runtime: &ContainerRuntime, job: JobId) -> bool {
    let name = container(job);
    match stageman_agent::tunnel_port(runtime, &name).await {
        Ok(Some(port)) => reachable(port).await,
        Ok(None) | Err(_) => false,
    }
}

/// Whether anything is behind a published port, rather than merely in front
/// of it.
///
/// Pure of everything but the socket, and separate so that the decision above
/// reads as one sentence. Loopback, because that is where a tunnel is
/// published — see
/// `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`.
///
/// **Connecting proves nothing, so this also reads.** The runtime's proxy owns
/// the host port for as long as the container runs and accepts every
/// connection to it; what it cannot fake is the far side. So the three answers
/// a read gives are the three states worth telling apart, and only the first
/// means nothing is there:
///
/// - **closed at once** — the proxy accepted, found nothing inside to forward
///   to, and hung up. This is every job that never showed anything.
/// - **said something** — plainly serving.
/// - **held open and silent** — also serving, and the case that makes reading
///   worth the wait rather than a trick: an HTTP server says nothing at all
///   until it is asked, so a probe that demanded bytes would stop exactly the
///   containers this exists to keep. Nothing is written to find out, because
///   the far side is somebody else's server and a made-up request is not ours
///   to send.
async fn reachable(port: u16) -> bool {
    use tokio::io::AsyncReadExt as _;

    let Ok(Ok(mut tunnel)) = tokio::time::timeout(
        ANSWERING_WITHIN,
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
    )
    .await
    else {
        return false;
    };

    let mut first = [0_u8; 1];
    match tokio::time::timeout(ANSWERING_WITHIN, tunnel.read(&mut first)).await {
        // Nothing behind the proxy. A clean close and a reset are the same
        // event seen through different runtimes, and neither is a fault.
        Ok(Ok(0) | Err(_)) => false,
        // Something behind it, whether it spoke or is waiting to be asked.
        Ok(Ok(_)) | Err(_) => true,
    }
}

/// A container this project started and has not removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abandoned {
    /// The container's name, which is what removes it.
    pub container: String,
    /// The job it belongs to, if its name says so.
    pub job: Option<JobId>,
}

/// A job could not be run.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// The agent's container could not be reached, or would not answer.
    #[error("the job's agent could not be run")]
    Agent(#[source] AgentError),
}

/// Every container this project has left behind.
///
/// Read from the runtime rather than from the instance, which is the whole
/// point: a container the snapshot has forgotten is exactly the one worth
/// finding, and `docs/conventions.md` §4 asks that nothing be left *untracked*
/// rather than that nothing be left.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses the query.
pub async fn left_behind(runtime: &ContainerRuntime) -> Result<Vec<Abandoned>, JobError> {
    Ok(stageman_agent::abandoned(runtime)
        .await
        .map_err(JobError::Agent)?
        .into_iter()
        .map(|container| Abandoned {
            job: job_of(&container),
            container,
        })
        .collect())
}

/// Removes a job's container and everything in it.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses.
pub async fn discard(runtime: &ContainerRuntime, job: JobId) -> Result<(), JobError> {
    stageman_agent::discard(runtime, &container(job))
        .await
        .map_err(JobError::Agent)
}

#[cfg(test)]
mod tests {
    use super::{Abandoned, ContainerRuntime, container, discard, job_of, left_behind, reachable};
    use stageman_core::{JobId, Uuid};

    /// Something listening answers; nothing listening does not.
    ///
    /// **A bare port, and that is the limit of what this proves.** It once
    /// claimed to be "the whole of what decides a container's lifetime" on the
    /// grounds that a port is a port. That premise is false, and it is what let
    /// every container run for ever with this test green: a *published* port
    /// has the runtime's proxy in front of it, and the proxy accepts on the
    /// container's behalf. See
    /// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`.
    ///
    /// It is kept because both answers still have to be right for a socket
    /// nobody is proxying, and it is cheap. What it cannot see is covered by
    /// [`a_published_port_with_nothing_inside_answers_for_nobody`], which needs
    /// a runtime.
    ///
    /// Worth knowing what the first assertion covers for free: this listener is
    /// bound and never accepted on, so the handshake completes in the kernel's
    /// backlog and the read that follows blocks. That is the *held open and
    /// silent* case — the shape an HTTP server has before it is asked
    /// anything — so a probe demanding bytes would fail here rather than only
    /// in production.
    #[tokio::test]
    async fn a_port_answers_only_while_something_is_listening() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();

        assert!(reachable(port).await, "something is listening on {port}");

        drop(listener);
        assert!(
            !reachable(port).await,
            "nothing is listening on {port} any more",
        );
    }

    /// A published port answers for nobody when nothing is inside.
    ///
    /// **The test the bug got past, and it needs a real container to exist at
    /// all.** The one above binds a socket and asks about it; this publishes a
    /// port the way a job's container does and asks about *that*, which is the
    /// only version that meets the runtime's proxy. With the probe this
    /// replaces — a connection and nothing more — the first assertion here
    /// fails on both runtimes, which is precisely how every job container came
    /// to run for ever.
    ///
    /// Both directions, because a probe that answered `false` unconditionally
    /// would pass the half that matters and destroy the feature: it would stop
    /// a container somebody is looking at.
    ///
    /// Needs a runtime and no credential and no image of ours — anything that
    /// stays up will do, and the entry point is overridden so nothing is run.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_published_port_with_nothing_inside_answers_for_nobody() {
        let runtime = located_runtime();
        let job = a_showing_job();
        let name = container(job);
        discard(&runtime, job).await.expect("a clean slate");
        let anything = stageman_agent::build(
            &runtime,
            stageman_core::Agent::Claude,
            stageman_core::Role::Foreman,
        )
        .await
        .expect("the image builds");

        // Nothing inside is listening: the proxy is the only thing on the host
        // port, and it is what a bare connection would find.
        let empty = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                &name,
                "--label",
                &format!("stageman.job={name}"),
                "--publish",
                &format!("127.0.0.1::{}", stageman_agent::TUNNEL_PORT),
                "--entrypoint",
                "sh",
                anything.as_argument(),
                "-c",
                "sleep 30",
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            empty.status.success(),
            "{}",
            String::from_utf8_lossy(&empty.stderr)
        );

        let port = stageman_agent::tunnel_port(&runtime, &name)
            .await
            .expect("the runtime answers")
            .expect("a mapping was published");
        // Reported rather than asserted, and only if this fails. Whether a
        // bare connection succeeds is a property of the host: both runtimes
        // proxy a published port by default and it does, which is the trap
        // this test exists for, but Docker with `userland-proxy` disabled
        // refuses instead and the assertion below then holds for a simpler
        // reason. Saying which makes a failure here diagnosable rather than
        // puzzling.
        let trapped = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok();
        assert!(
            !reachable(port).await,
            "nothing is listening inside, so the proxy on {port} answers for nobody \
             (a bare connection to it succeeds here: {trapped})"
        );

        discard(&runtime, job).await.expect("it is removable");

        // And the other direction, on a container that genuinely serves, so
        // that this cannot pass by answering `false` to everything.
        //
        // Node rather than a networking tool, because the base image is
        // node and has no `nc`, `socat` or `python3` — the same absence
        // `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
        // records about `ss` and `lsof`. It binds every interface, not
        // loopback, or the proxy would have nothing to forward to; and it
        // never writes, which makes it the held-open-and-silent case rather
        // than the easy one.
        let serving = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                &name,
                "--label",
                &format!("stageman.job={name}"),
                "--publish",
                &format!("127.0.0.1::{}", stageman_agent::TUNNEL_PORT),
                "--entrypoint",
                "node",
                anything.as_argument(),
                "-e",
                &format!(
                    "require('net').createServer().listen({}, '0.0.0.0',                      () => console.log('listening'))",
                    stageman_agent::TUNNEL_PORT
                ),
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            serving.status.success(),
            "{}",
            String::from_utf8_lossy(&serving.stderr)
        );

        // Waited for, and waited for by asking the container rather than the
        // port. `--detach` returns once the container is started, which is
        // before node has bound anything — so probing straight away finds the
        // proxy with nothing behind it yet and reads exactly like the empty
        // case above. That is what failed in continuous integration and passed
        // on the machine that wrote it, which is the shape of every race worth
        // the name.
        ready(&runtime, &name, "listening").await;

        let port = stageman_agent::tunnel_port(&runtime, &name)
            .await
            .expect("the runtime answers")
            .expect("a mapping was published");
        assert!(
            reachable(port).await,
            "something is listening inside, so {port} is showing it"
        );

        discard(&runtime, job).await.expect("it is removable");
    }

    /// Waits until a container has said the thing that means it is ready.
    ///
    /// **Asks the container, never the port.** Waiting on `answering` would
    /// make the assertion that follows pass whenever that function said yes,
    /// which is the bug it is there to catch — a readiness check must not be
    /// the thing under test.
    async fn ready(runtime: &ContainerRuntime, name: &str, marker: &str) {
        // Thirty seconds, counted in polls rather than measured against a
        // deadline: adding a duration to an instant is arithmetic that can
        // overflow, the gate rightly refuses it, and there is nothing here
        // that needs a clock.
        let mut printed = String::new();
        for _ in 0..300 {
            let said = std::process::Command::new(runtime.path())
                .args(["logs", name])
                .output()
                .expect("the runtime reports what a container printed");
            printed = format!(
                "{}{}",
                String::from_utf8_lossy(&said.stdout),
                String::from_utf8_lossy(&said.stderr)
            );
            if printed.contains(marker) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("{name} never said {marker:?}; it said: {printed}");
    }

    fn a_job() -> JobId {
        JobId::from_uuid(Uuid::from_u128(42))
    }

    /// A different job from [`a_job`], and it has to be.
    ///
    /// A container's name is derived from its job's identifier and from
    /// nothing else, which is the property `two_jobs_never_share_a_container_name`
    /// exists for. Two tests sharing an identifier therefore share a container,
    /// and the test runner runs them at once: the second to start is refused
    /// the name and fails for a reason that has nothing to do with what it
    /// asserts. Found exactly that way.
    fn a_showing_job() -> JobId {
        JobId::from_uuid(Uuid::from_u128(43))
    }

    #[test]
    fn a_jobs_container_name_says_which_job_it_is() {
        assert_eq!(job_of(&container(a_job())), Some(a_job()));
    }

    /// The property the sweep rests on. Two jobs must never be able to address
    /// the same container, since the runtime would refuse the second and the
    /// first would be resumed with the wrong work.
    #[test]
    fn two_jobs_never_share_a_container_name() {
        let one = JobId::from_uuid(Uuid::from_u128(1));
        let other = JobId::from_uuid(Uuid::from_u128(2));

        assert_ne!(container(one), container(other));
    }

    #[test]
    fn a_container_this_project_did_not_name_belongs_to_no_job() {
        assert_eq!(job_of("something-else"), None);
        assert_eq!(job_of("stageman-job-not-an-identifier"), None);
        assert_eq!(job_of(""), None);
    }

    /// A container whose name says nothing is still ours to remove, so it has
    /// to survive the sweep as a value rather than being filtered away.
    #[test]
    fn an_unrecognised_container_is_still_reported() {
        let left = Abandoned {
            container: "stageman-job-from-an-older-scheme".to_owned(),
            job: job_of("stageman-job-from-an-older-scheme"),
        };

        assert_eq!(left.job, None);
        assert!(!left.container.is_empty());
    }

    /// The two halves of a sweep, checked against a real runtime.
    ///
    /// Worth being exact about what this defends, because it is not the path
    /// that resumes work. Putting a job back is a lookup from job to container:
    /// derive the name, start it, continue the session. This is the opposite
    /// direction — from container back to job — and it exists for the other
    /// half of `docs/conventions.md` §4's bar, *nothing untracked*. Walking the
    /// instance's jobs can only ever find containers the instance already knows
    /// about; one it has lost is invisible that way and leaks silently.
    ///
    /// So the label is how they are found and the name is how they are
    /// identified, and the failure this catches is the label key changing on
    /// one side: the filter stops matching, orphans stop being found, and an
    /// instance with several reads exactly like a clean one.
    ///
    /// Needs a runtime and a network and no credential, because it never runs
    /// an agent — it overrides the entry point, so any image will do and a
    /// foreman's is the cheaper one to build.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_container_named_for_a_job_is_swept_up_as_that_job() {
        let runtime = located_runtime();
        let job = a_job();
        let name = container(job);
        discard(&runtime, job).await.expect("a clean slate");
        let anything = stageman_agent::build(
            &runtime,
            stageman_core::Agent::Claude,
            stageman_core::Role::Foreman,
        )
        .await
        .expect("the image builds");

        let created = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                &name,
                "--label",
                &format!("stageman.job={name}"),
                "--network",
                "none",
                "--entrypoint",
                "sh",
                anything.as_argument(),
                "-c",
                "sleep 30",
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );

        let left = left_behind(&runtime).await.expect("the runtime answers");
        let found = left.iter().find(|abandoned| abandoned.container == name);

        assert_eq!(
            found.map(|abandoned| abandoned.job),
            Some(Some(job)),
            "the sweep should have recognised it: {left:?}"
        );

        discard(&runtime, job).await.expect("it is removable");
        let after = left_behind(&runtime).await.expect("the runtime answers");
        assert!(
            !after.iter().any(|abandoned| abandoned.container == name),
            "{after:?}"
        );
    }

    /// The container runtime, found rather than configured — allowed in a test
    /// for the reason the rule itself gives: it is about a daemon under a
    /// service manager, and this runs on somebody's machine.
    fn located_runtime() -> ContainerRuntime {
        let located = std::process::Command::new("sh")
            .args(["-c", "command -v docker"])
            .output()
            .expect("looking for a container runtime");
        let path = String::from_utf8(located.stdout).expect("a runtime path is text");
        ContainerRuntime::new(std::path::PathBuf::from(path.trim()))
    }
}
