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
    kickoff: &str,
) -> Result<Answer, JobError> {
    stageman_agent::begin(runtime, handout, &container(job), kickoff)
        .await
        .map_err(JobError::Agent)
}

/// Puts a job back to work after its container stopped.
///
/// Takes no handout: what a container was given at creation is part of it, so
/// a restart is already authenticated. `notice` is what the agent is told about
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
    notice: &str,
) -> Result<Answer, JobError> {
    stageman_agent::resume(runtime, &container(job), notice)
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
    use super::{Abandoned, ContainerRuntime, container, discard, job_of, left_behind};
    use stageman_core::{JobId, Uuid};

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
    /// Needs a runtime and an image and no credential, because it never runs
    /// an agent.
    #[tokio::test]
    #[ignore = "needs a container runtime and a built image; run `just image-handshake`"]
    async fn a_container_named_for_a_job_is_swept_up_as_that_job() {
        let runtime = located_runtime();
        let job = a_job();
        let name = container(job);
        discard(&runtime, job).await.expect("a clean slate");

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
                stageman_agent::image(stageman_core::Agent::Claude),
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
