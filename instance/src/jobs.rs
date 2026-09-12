//! Starting a job: the record before the thread, the thread before the
//! container, and the container before the agent speaks.

use stageman_core::{
    Handout, HandoutError, Job, JobId, Kit, Progress, ProjectId, Thread, Timestamp, Waiting,
};
use stageman_foreman::Voice;

use crate::Instance;
use crate::turns::{Turn, speaking_for};
use crate::vocabulary::{AppEffect, Run, Speaker};

/// A job could not be recorded.
#[derive(Debug, thiserror::Error)]
pub enum BeginError {
    /// The project is not one this instance watches.
    #[error("no project {0} in this instance")]
    UnknownProject(ProjectId),
    /// What the job's agent may see could not be decided.
    #[error("what the job's agent may see could not be decided")]
    Handout(#[source] HandoutError),
}

impl Instance {
    /// Records a job on a project and sets it going.
    ///
    /// **The record is written before anything else exists, and the order is
    /// not arbitrary.** Killed after the record lands and before the
    /// container exists, this leaves a job believed to be working with
    /// nothing to run in — which waking recognises and records as lost. The
    /// other order would leave a container naming a job the instance has no
    /// record of, which is the case that needs a person. So the thread is
    /// opened, and the turn started, only once the record is on the disk.
    ///
    /// The instruction the agent begins from is composed here, from the work,
    /// by the foreman crate. Nothing else composes one.
    ///
    /// # Errors
    ///
    /// Fails if the project is unknown, or if a handout cannot be decided
    /// for it.
    pub fn begin(
        &mut self,
        project: ProjectId,
        kit: Kit,
        reason: &str,
        work: &str,
        at: Timestamp,
    ) -> Result<JobId, BeginError> {
        let repository = self
            .state
            .projects
            .get(&project)
            .map(|watched| watched.repository.clone())
            .ok_or(BeginError::UnknownProject(project))?;
        // The kit arrives decided — by a foreman naming one of the project's,
        // or by a person picking one — and is never composed here.
        let handout = Handout::for_job(&self.state, kit, project).map_err(BeginError::Handout)?;

        // What the job can be told depends on what it was handed, so the
        // prompt and the environment the container is started with are
        // decided from one value.
        let voice = if handout.channels().next().is_some() {
            Voice::Channel
        } else {
            Voice::Silent
        };
        // Minted before the instruction, because the instruction names where
        // this job can be reached and that address is built from the
        // identifier.
        let job = JobId::from_uuid(crate::mint(&mut self.rng));
        let variables: Vec<_> = handout.variable_names().cloned().collect();
        let kickoff = stageman_foreman::kickoff(
            &repository,
            work,
            voice,
            &crate::tunnel::address(&self.domain, job, self.serving),
            &variables,
        );
        let announcement = stageman_foreman::announcement(&repository, reason, job);

        if let Some(watched) = self.state.projects.get_mut(&project) {
            watched.jobs.insert(
                job,
                Job::new(handout.kit().clone(), reason.to_owned(), kickoff, at),
            );
            self.dirty = true;
        }

        // A channel that will not take a message fails the job rather than
        // letting it run speaking at the root: the kickoff has told this
        // agent it can reach a person, and running it anyway would make that
        // quietly false. It is also the cheapest moment to fail — no
        // container exists yet.
        match handout.channels().next() {
            Some((_, speaking)) => self.defer(AppEffect::OpenThread {
                job,
                speaking: speaking.clone().into(),
                announcement,
            }),
            None => self.start(job),
        }
        Ok(job)
    }

    /// Where a job's conversation will happen, or why it could not be opened.
    pub fn thread_opened(&mut self, job: JobId, outcome: Result<Thread, String>) {
        match outcome {
            Ok(thread) => {
                if let Some(recorded) = self.state.job_mut(job) {
                    recorded.thread = Some(thread);
                    self.dirty = true;
                }
                self.start(job);
            }
            Err(why) => {
                tracing::warn!(%job, %why, "the job's thread could not be opened");
                self.record(
                    job,
                    Progress::Idle(Waiting::Failed(format!(
                        "its channel could not be reached: {why}"
                    ))),
                );
            }
        }
    }

    /// Starts a recorded job's first turn, once its record is on the disk.
    ///
    /// The handout is decided again from the record rather than carried
    /// across the thread being opened: the record holds the kit and the
    /// kickoff, and a handout is what a process is about to be handed, never
    /// state.
    fn start(&mut self, job: JobId) {
        let Some(project) = self.state.project_of(job) else {
            return;
        };
        let Some((thread, kit)) = self.recorded(job) else {
            return;
        };
        let handout = match Handout::for_job(&self.state, kit, project) {
            Ok(handout) => handout,
            Err(why) => {
                tracing::warn!(%job, %why, "the job's handout could not be decided");
                self.record(
                    job,
                    Progress::Idle(Waiting::Failed(format!(
                        "what its agent may see could not be decided: {why}"
                    ))),
                );
                return;
            }
        };
        let handout = match thread.clone() {
            Some(thread) => handout.speaking_in(thread),
            None => handout,
        };
        let Some(kickoff) = self.state.job(job).map(|recorded| recorded.kickoff.clone()) else {
            return;
        };
        let environment = match crate::rendered(&handout) {
            Ok(environment) => environment,
            Err(why) => {
                tracing::warn!(%job, %why, "the job's environment could not be decided");
                self.record(
                    job,
                    Progress::Idle(Waiting::Failed(format!(
                        "what its agent may see could not be decided: {why}"
                    ))),
                );
                return;
            }
        };
        let speaker = Speaker::Job(job);
        let warrant = self.warrant(speaker, thread);
        self.turns.insert(speaker, Turn::noticed());
        self.defer(AppEffect::RunTurn {
            speaker,
            run: Run::Begin {
                container: stageman_job::container(job),
                instance: self.id,
                agent: handout.agent(),
                role: handout.role(),
                environment,
                repository: handout.repository().map(str::to_owned),
                platform: handout
                    .platform(stageman_core::Platform::GitHub)
                    .map(|_| stageman_core::Platform::GitHub),
                kit: handout.kit().clone(),
                warrant,
                kickoff,
            },
        });
        debug_assert!(
            speaking_for(&self.state, job).is_some() || self.state.job(job).is_some(),
            "a job being started is on the record"
        );
    }
}
