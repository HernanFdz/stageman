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

use stageman_agent::{AgentError, Answer, ContainerRuntime};
use stageman_core::{Handout, JobId, Uuid};

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
#[must_use]
fn job_of(container: &str) -> Option<JobId> {
    container
        .strip_prefix(PREFIX)
        .and_then(|rest| Uuid::parse_str(rest).ok())
        .map(JobId::from_uuid)
}

/// How long to wait for a job's tunnel to answer before deciding it is not
/// showing anything.
///
/// Generous for a connection to this machine's own loopback, where the answer
/// arrives in microseconds or not at all, and deliberately so: the cost of
/// waiting is a moment at the end of a turn, and the cost of being impatient
/// is stopping a container somebody is looking at.
const ANSWERING_WITHIN: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether a job is still showing something, and so whether it stays up.
///
/// Named for what it says rather than for what a caller does about it, because
/// the two are different questions and only the first is a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Showing {
    /// Something answered on its tunnel, so the container was left running.
    Still,
    /// Nothing answered, so the container was stopped.
    Nothing,
}

/// Stops a job's container unless its tunnel is still answering.
///
/// The whole of
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
/// in one function, called at each of the three moments that record names: a
/// turn ending, a sweep of what was left up, and startup.
///
/// **Answering is asked of the tunnel, not of the container.** A connection is
/// opened to the port the runtime published, from here, and something has to
/// accept it. That is deliberately stricter than looking for a process bound
/// inside: a server an agent bound to its container's own loopback holds the
/// port and is reachable by nobody, and treating it as showing something would
/// keep a container alive for ever to serve no one.
///
/// **Total, and that is the honest signature rather than a convenience.** Every
/// way this can go wrong means the same thing and admits the same response: a
/// container that is gone, one that never had a mapping, and one the runtime
/// will not answer questions about are all showing nothing, and there is
/// nothing else a caller would do about any of them. A runtime broken badly
/// enough to matter fails loudly everywhere else in the same breath.
pub async fn rest(runtime: &ContainerRuntime, job: JobId) -> Showing {
    let name = container(job);
    if let Ok(Some(port)) = stageman_agent::tunnel_port(runtime, &name).await
        && answering(port).await
    {
        return Showing::Still;
    }
    drop(stageman_agent::halt(runtime, &name).await);
    Showing::Nothing
}

/// Whether anything accepts a connection on a published port.
///
/// Pure of everything but the socket, and separate so that the decision above
/// reads as one sentence. Loopback, because that is where a tunnel is
/// published — see
/// `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`.
async fn answering(port: u16) -> bool {
    let reached = tokio::time::timeout(
        ANSWERING_WITHIN,
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
    )
    .await;
    matches!(reached, Ok(Ok(_)))
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

/// Starts a job: one agent, in one container, on one project.
///
/// The container outlives this call and this process, under a name derived
/// from `job`. Nothing here removes it — that is retention, deliberately
/// unanswered in `docs/open-questions.md` until there is a finished job to
/// retire.
///
/// `kickoff` is composed by the foreman and never here. A job executes
/// instructions it did not write, which is what keeps every prompt in this
/// system reviewable in one place — see `docs/architecture.md` §1.
///
/// # Errors
///
/// Fails if the container cannot be started or the agent will not answer.
pub async fn start(
    runtime: &ContainerRuntime,
    handout: &Handout,
    job: JobId,
    tools: Option<&stageman_agent::Tools>,
    kickoff: &str,
) -> Result<Answer, JobError> {
    stageman_agent::begin(runtime, handout, &container(job), tools, kickoff)
        .await
        .map_err(JobError::Agent)
}

/// Puts a job back to work after its container stopped.
///
/// Takes no handout: what a container was given at creation is part of it, so
/// a restart is already authenticated. It does take the thread, because that
/// is the one thing a container cannot keep — an environment is fixed at
/// creation and a job's thread is written in before each start. `notice` is what the agent is told about
/// having been interrupted, and like every other instruction it is composed by
/// the foreman rather than invented here.
///
/// # Errors
///
/// Fails as [`start`] does, and if the container holds no session to continue
/// — which is what a job stopped before its agent said anything looks like.
/// That job has nothing to resume and needs starting over as a new job, since
/// `docs/conventions.md` §2 has no retry.
pub async fn resume(
    runtime: &ContainerRuntime,
    job: JobId,
    tools: Option<&stageman_agent::Tools>,
    notice: &str,
) -> Result<Answer, JobError> {
    stageman_agent::resume(runtime, &container(job), tools, notice)
        .await
        .map_err(JobError::Agent)
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

/// Every job whose container is running right now.
///
/// The set [`left_behind`] narrows to, and a different question since
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`:
/// a container is no longer stopped by the turn inside it ending, so what is
/// up and what exists have come apart. Only jobs are returned — a container
/// whose name says nothing this version understands has no job to rest, and a
/// foreman's is not a job's to stop.
///
/// # Errors
///
/// Fails if the runtime cannot be run, or refuses the query.
///
/// Skipped by mutation testing: it asks the runtime and maps the answer
/// through the private inverse of [`container`], which is total, reversible
/// and tested directly.
#[mutants::skip]
pub async fn still_running(runtime: &ContainerRuntime) -> Result<Vec<JobId>, JobError> {
    Ok(stageman_agent::running(runtime)
        .await
        .map_err(JobError::Agent)?
        .iter()
        .filter_map(|container| job_of(container))
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
    use super::{Abandoned, ContainerRuntime, answering, container, discard, job_of, left_behind};
    use stageman_core::{JobId, Uuid};

    /// Something listening answers; nothing listening does not.
    ///
    /// The whole of what decides a container's lifetime, and it needs no
    /// container: a port is a port, and what makes this worth a test is that
    /// both answers have to be right. One that never answered would stop every
    /// container the moment its turn ended, which is the behaviour this
    /// replaces; one that always answered would keep every container running
    /// for ever, which is the behaviour it exists to avoid.
    #[tokio::test]
    async fn a_port_answers_only_while_something_is_listening() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();

        assert!(answering(port).await, "something is listening on {port}");

        drop(listener);
        assert!(
            !answering(port).await,
            "nothing is listening on {port} any more",
        );
    }

    fn a_job() -> JobId {
        JobId::from_uuid(Uuid::from_u128(42))
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
