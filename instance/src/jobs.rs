//! Starting a job: the record before the room, the room before the
//! container, and the container before the agent speaks.

use stageman_core::{
    Handout, HandoutError, Job, JobId, Kit, Place, Progress, ProjectId, Room, Secret, Timestamp,
    Waiting,
};

use crate::Running;
use crate::channel::Origin;
use crate::turns::{Run, Turn, speaking_for};
use crate::vocabulary::Speaker;

/// What a job is commissioned with: the kit it runs on, and the three texts
/// about it.
///
/// One value rather than four arguments, so that a caller cannot hand over
/// a reason from one request and the work from another, and so that the
/// texts travel under the names a person reads them by on the dashboard.
#[derive(Debug, Clone)]
pub struct Commission<'a> {
    /// What it runs on, decided by whoever asked for it.
    pub kit: Kit,
    /// Why it exists, in prose for the dashboard.
    pub reason: &'a str,
    /// What its agent is to do.
    pub work: &'a str,
    /// A few words naming it, which its room is named after.
    pub title: &'a str,
}

/// A job could not be recorded.
#[derive(Debug, thiserror::Error)]
pub enum BeginError {
    /// The project is not one this instance watches.
    #[error("no project {0} in this instance")]
    UnknownProject(ProjectId),
    /// The project has no channel bound, so a job on it would have nowhere
    /// to speak. Only a project the last release wrote can be in this state
    /// — see
    /// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`.
    #[error(
        "project {0} has no Slack binding, so a job on it would have nowhere to speak; bind one \
         in the dashboard"
    )]
    NoChannel(ProjectId),
    /// What the job's agent may see could not be decided.
    #[error("what the job's agent may see could not be decided")]
    Handout(#[source] HandoutError),
    /// No name could be minted for it: every one tried was taken. Bounded
    /// rather than tried for ever, and never reached by any seed — see
    /// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
    #[error("no name could be minted for the job: every one tried was taken")]
    Unnamed,
}

/// How many names are minted for a job before giving up. Eight hex clash
/// once in four billion, so the second is never needed; the bound is there
/// so that a loop cannot stand still.
const NAMING_ATTEMPTS: usize = 8;

impl Running {
    /// Records a job on a project and sets it going.
    ///
    /// **The record is written before anything else exists, and the order is
    /// not arbitrary.** Killed after the record lands and before the
    /// container exists, this leaves a job believed to be working with
    /// nothing to run in — which waking recognises and records as lost. The
    /// other order would leave a container naming a job the instance has no
    /// record of, which is the case that needs a person. So the room is
    /// made, and the turn started, only once the record is on the disk.
    ///
    /// The instruction the agent begins from is composed here, from the work,
    /// by the foreman crate. Nothing else composes one. The room's name is
    /// composed by the channel crate, from the project, the title and the
    /// job's identifier — see
    /// `docs/decisions/0061-a-job-has-a-room-of-its-own.md`.
    ///
    /// # Errors
    ///
    /// Fails if the project is unknown, if it has no channel bound, or if a
    /// handout cannot be decided for it.
    pub fn begin(
        &mut self,
        project: ProjectId,
        commission: Commission<'_>,
        origin: Option<Origin>,
        at: Timestamp,
    ) -> Result<JobId, BeginError> {
        let Commission {
            kit,
            reason,
            work,
            title,
        } = commission;
        let (called, repository) = self
            .state
            .projects
            .get(&project)
            .map(|watched| (watched.name.clone(), watched.repository.https()))
            .ok_or(BeginError::UnknownProject(project))?;
        // The channel the job's room is made on. Refused before anything is
        // recorded when there is none, which only a project the last
        // release wrote can lack.
        let channel = self
            .state
            .projects
            .get(&project)
            .and_then(|watched| watched.channels.keys().next().copied())
            .ok_or(BeginError::NoChannel(project))?;
        // Named before the instruction, because the instruction names where
        // this job can be reached and that address is built from the name —
        // and before the room's name, for the same reason.
        let job = self.name_job(title)?;
        // The one thing its container may ask of this instance: its own
        // project's credential, per
        // `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
        // Minted with the job and kept on its record, so that a restart
        // knows what the container was created presenting.
        let warrant = Secret::new(self.unguessable());
        // The kit arrives decided — by a foreman naming one of the project's,
        // or by a person picking one — and is never composed here.
        let handout = Handout::for_job(&self.state, kit, project, warrant.clone())
            .map_err(BeginError::Handout)?;
        let speaking = handout
            .channel(channel)
            .cloned()
            .ok_or(BeginError::NoChannel(project))?;
        // Names and what each is for, never a value: the handout has no
        // method that would hand a value to a prompt.
        let variables: Vec<_> = handout
            .variables_told()
            .map(|(name, note)| (name.clone(), note.to_owned()))
            .collect();
        let kickoff = stageman_foreman::kickoff(
            &repository,
            work,
            &crate::tunnel::address(&self.domain, &job, self.serving),
            &variables,
        );
        let name = stageman_channel::room_name(channel, &called, &job);

        if let Some(watched) = self.state.projects.get_mut(&project) {
            let mut recorded = Job::new(
                handout.kit().clone(),
                reason.to_owned(),
                kickoff,
                at,
                warrant,
            );
            recorded.asked_by = origin.as_ref().and_then(|origin| origin.user.clone());
            watched.jobs.insert(job.clone(), recorded);
            self.dirty = true;
        }

        // A platform that will not make the room fails the job rather than
        // letting it run with nowhere to speak: the kickoff has told this
        // agent it can reach a person, and running it anyway would make
        // that quietly false. It is also the cheapest moment to fail — no
        // container exists yet.
        self.create_room(job.clone(), channel, &speaking, &name, origin);
        Ok(job)
    }

    /// A name for a new job — its title and eight hex minted for it — that
    /// no job of this instance already has, minted again while one does.
    /// Checked across the instance rather than the project, because the
    /// tunnel's host is the instance's — see
    /// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
    fn name_job(&mut self, title: &str) -> Result<JobId, BeginError> {
        for _ in 0..NAMING_ATTEMPTS {
            let named = JobId::named(title, &crate::mint(&mut self.rng));
            if self.state.project_of(&named).is_none() {
                return Ok(named);
            }
        }
        Err(BeginError::Unnamed)
    }

    /// The room a job's conversation will happen in, or why it could not be
    /// made.
    ///
    /// Recorded first, because a reply can only find the job through it.
    /// Then, once that record has landed, the room is described, the person
    /// who asked is invited, its opening is said, the message the job came
    /// from is told where the job is — and the job's first turn begins.
    /// None of the keeping is waited on: what it answers changes nothing
    /// here.
    pub fn room_created(
        &mut self,
        job: &JobId,
        origin: Option<Origin>,
        outcome: Result<String, String>,
    ) {
        let id = match outcome {
            Ok(id) => id,
            Err(why) => {
                tracing::warn!(%job, %why, "the job's room could not be made");
                self.record(
                    job,
                    Progress::Idle(Waiting::Failed(format!(
                        "its room could not be made: {why}"
                    ))),
                );
                return;
            }
        };
        let Some(project) = self.state.project_of(job) else {
            return;
        };
        let Some((channel, speaking, repository, reason)) =
            self.state.projects.get(&project).and_then(|watched| {
                let channel = watched.channels.keys().next().copied()?;
                let recorded = watched.jobs.get(job)?;
                Some((
                    channel,
                    self.state.speaking(project, channel)?,
                    watched.repository.https(),
                    recorded.reason.clone(),
                ))
            })
        else {
            return;
        };
        let room = Room { channel, id };
        if let Some(recorded) = self.state.job_mut(job) {
            recorded.room = Some(room.clone());
            self.dirty = true;
        }

        let tunnel = crate::tunnel::address(&self.domain, job, self.serving);
        let dashboard = crate::tunnel::dashboard(&self.domain, self.serving);
        self.describe_room(
            job.clone(),
            channel,
            &speaking,
            &room.id,
            &reason,
            &format!("Showing at {tunnel} · dashboard at {dashboard}"),
        );
        if let Some(user) = origin.as_ref().and_then(|origin| origin.user.as_deref()) {
            self.invite_into(job.clone(), channel, &speaking, &room.id, user);
        }
        let mention = self.own_mention(project, channel);
        self.say(
            &speaking,
            &Place::root(room.clone()),
            &stageman_foreman::room_opening(&repository, &reason, &mention),
        );
        let link = stageman_channel::room_link(channel, &room.id);
        match origin {
            Some(origin) => self.say(
                &speaking,
                &Place::from(origin.thread),
                &stageman_foreman::started_notice(&link),
            ),
            // A job nobody asked for on a channel — started from the
            // dashboard — is announced where the project's foreman works,
            // if it has such a room yet, per
            // `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
            None => {
                if let Some(foremans) = self
                    .state
                    .projects
                    .get(&project)
                    .and_then(|watched| watched.foreman_room.clone())
                {
                    self.say(
                        &speaking,
                        &Place::root(foremans),
                        &stageman_foreman::started_notice(&link),
                    );
                }
            }
        }
        self.start(job);
    }

    /// Starts a recorded job's first turn, once its record is on the disk.
    ///
    /// The handout is decided again from the record rather than carried
    /// across the room being made: the record holds the kit and the
    /// kickoff, and a handout is what a process is about to be handed, never
    /// state.
    fn start(&mut self, job: &JobId) {
        let Some(project) = self.state.project_of(job) else {
            return;
        };
        let Some((room, kit)) = self.recorded(job) else {
            return;
        };
        // On the record since the job was, so this cannot be reached without
        // one; refused loudly rather than substituted for, all the same.
        let Some(warrant) = self
            .state
            .job(job)
            .and_then(|recorded| recorded.warrant().cloned())
        else {
            tracing::warn!(%job, "the job holds no warrant, so its agent cannot be handed one");
            self.record(
                job,
                Progress::Idle(Waiting::Failed(
                    "it holds no warrant to fetch its credential with".to_owned(),
                )),
            );
            return;
        };
        let handout = match Handout::for_job(&self.state, kit, project, warrant) {
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
        let handout = match room.clone() {
            Some(room) => handout.speaking_in(Place::root(room)),
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
        let speaker = Speaker::Job(job.clone());
        let warrant = self.warrant(&speaker, room.map(Place::root), None);
        let run = Run::Begin {
            container: stageman_job::container(job),
            agent: handout.agent(),
            role: handout.role(),
            environment,
            repository: handout.repository().map(str::to_owned),
            platform: handout
                .reaches(stageman_core::Platform::GitHub)
                .then_some(stageman_core::Platform::GitHub),
            actor: self.actor_for(project),
            kit: handout.kit().clone(),
            warrant,
            tools: self.tools.clone(),
            fetching: Some(self.fetching.clone()),
            kickoff,
        };
        // Told at the root of its room when it ends, if it has one.
        let first = self.turn(speaker, Turn::noticed(run));
        self.defer(first);
        debug_assert!(
            speaking_for(&self.state, job).is_some() || self.state.job(job).is_some(),
            "a job being started is on the record"
        );
    }
}
