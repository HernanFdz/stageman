//! Reconciling what the runtime holds against what the instance believes:
//! once on waking, from the facts the world was born with, and every so often
//! afterwards, from a listing it asks for.
//!
//! The decisions are pure functions over names and records, and the tests
//! below pin them without a runtime. What they decide reaches the world as
//! effects; nothing here is removed, stopped or resumed directly.

use stageman_core::{InstanceId, JobId, Outcome, Progress, ProjectId, State};

use crate::turns::{Run, Turn};
use crate::vocabulary::{Container, Speaker};
use crate::{Asked, Command};
use crate::{Effect, Running, SETTLING_INTERVAL};

/// One container, placed as far as its name allows.
///
/// The name is what removes it; the job it names, if any, is what resumes it;
/// the label says whose it is when the name says nothing this instance knows.
pub struct Left<'a> {
    /// Its name.
    pub name: &'a str,
    /// The job it belongs to, if its name says so.
    pub job: Option<JobId>,
    /// Which instance started it, if its label says.
    pub instance: Option<InstanceId>,
    /// Whether it is up.
    pub running: bool,
}

impl<'a> Left<'a> {
    pub fn of(container: &'a Container) -> Self {
        Self {
            name: &container.name,
            job: stageman_job::job_of(&container.name),
            instance: container.instance,
            running: container.running,
        }
    }
}

/// Why a container could not be matched to work this instance knows about.
///
/// Two cases and not one, because an operator does something different about
/// each: a name that says nothing is an older version's, odd and benign; a
/// name that reads perfectly and points at a job the instance has lost means
/// work exists that this instance no longer knows it asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unplaceable<'a> {
    /// Its name says nothing this version understands.
    Unidentified(&'a str),
    /// Its name says which job, and this instance has no such job.
    Forgotten(&'a str, JobId),
}

impl Unplaceable<'_> {
    /// The container's name, whichever way it could not be placed.
    pub const fn named(&self) -> &str {
        match self {
            Self::Unidentified(name) | Self::Forgotten(name, _) => name,
        }
    }
}

/// Who a container belongs to, as far as this instance can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whose {
    /// This instance started it, so it is this instance's to remove.
    Ours,
    /// Another instance started it. Not ours to touch.
    Elsewhere,
    /// It says nothing, so nobody can tell.
    ///
    /// A container made before instances were told apart. Left alone for
    /// ever, which is the honest answer: removing it would be guessing, and
    /// on a shared runtime the guess destroys somebody else's work.
    Unlabelled,
}

/// What a container's label means, given whose instance is asking.
///
/// **Every uncertainty answers `Unlabelled`**, which is the direction that
/// cannot destroy anything. Inverted, this instance would remove every
/// container except its own — see
/// `docs/decisions/0054-a-container-says-which-instance-started-it.md`.
pub const fn belonging(started: Option<InstanceId>, instance: InstanceId) -> Whose {
    match started {
        Some(named) if named.as_uuid().as_u128() == instance.as_uuid().as_u128() => Whose::Ours,
        Some(_) => Whose::Elsewhere,
        None => Whose::Unlabelled,
    }
}

/// Everything the runtime has that the instance cannot account for.
///
/// A foreman's container is placed rather than reported: it belongs to a
/// project this instance watches and is exactly where that foreman's session
/// lives. One whose project is gone is the same loss as a forgotten job's.
pub fn unplaceable<'a>(left: &[Left<'a>], state: &State) -> Vec<Unplaceable<'a>> {
    left.iter()
        .filter_map(|container| match container.job {
            None => match stageman_foreman::project_of(container.name) {
                Some(project) if state.projects.contains_key(&project) => None,
                Some(_) | None => Some(Unplaceable::Unidentified(container.name)),
            },
            Some(job) if state.job(job).is_none() => {
                Some(Unplaceable::Forgotten(container.name, job))
            }
            Some(_) => None,
        })
        .collect()
}

/// Which instance started a container of that name, if the listing said.
fn labelled(left: &[Left<'_>], name: &str) -> Option<InstanceId> {
    left.iter()
        .find(|container| container.name == name)
        .and_then(|container| container.instance)
}

/// Every container belonging to a job that is over.
///
/// What a retirement leaves behind when it is interrupted, and what the next
/// waking finishes. A job the instance has no record of is not here — that is
/// a container to report rather than one to remove.
pub fn over(left: &[Left<'_>], state: &State) -> Vec<JobId> {
    left.iter()
        .filter_map(|container| container.job)
        .filter(|job| {
            state
                .job(*job)
                .is_some_and(|recorded| recorded.progress.is_retired())
        })
        .collect()
}

/// Whether anything left behind is this job's container.
pub fn has_container(left: &[Left<'_>], job: JobId) -> bool {
    left.iter().any(|container| container.job == Some(job))
}

/// Which of the jobs whose containers are up should be asked whether they are
/// still showing something, split by whether this instance can place them.
///
/// **A job believed to be working is passed over**, because its container is
/// up for the other reason in
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`:
/// there is a turn in it. Stopping that container would end an agent
/// mid-turn, which is the one outcome this must never produce.
///
/// **A job the instance has no record of is answered separately.** It cannot
/// be working *here*, and on a shared runtime it is most likely another
/// instance's job, mid-turn. Whose it is decides what happens to it.
pub fn resting(up: &[JobId], state: &State) -> (Vec<JobId>, Vec<JobId>) {
    let mut placed = Vec::new();
    let mut unplaced = Vec::new();
    for job in up.iter().copied() {
        match state.job(job) {
            Some(recorded) if matches!(recorded.progress, Progress::Working) => {}
            Some(_) => placed.push(job),
            None => unplaced.push(job),
        }
    }
    (placed, unplaced)
}

/// What waking found, and what it asked for.
///
/// Counts of what was *asked* rather than of what came of it: a turn put
/// back to work is answered later, and how it went is recorded on the job.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Swept {
    /// Jobs put back to work.
    pub resumed: usize,
    /// Jobs whose container was gone, and which are now over.
    pub lost: usize,
    /// Containers of jobs that are over, asked to be removed.
    pub cleared: usize,
    /// Containers of this instance's whose names it does not understand,
    /// asked to be removed.
    pub unidentified: usize,
    /// Containers naming a job this instance has no record of, asked to be
    /// removed. Counted apart from the above because it means something
    /// worse: work exists that this instance no longer knows it asked for.
    pub forgotten: usize,
    /// Containers carrying no instance label, left exactly where they were.
    pub unclaimed: usize,
    /// Containers belonging to another instance, passed over.
    pub elsewhere: usize,
}

/// Counts what waking did.
///
/// Zipped rather than indexed, because the unplaceable containers and the
/// decisions about them are built in one pass and the pairing is what makes a
/// count mean anything: a container is only counted as removed if its label
/// said it was ours.
fn tallied(
    resumed: usize,
    lost: usize,
    cleared: usize,
    unplaceable: &[Unplaceable<'_>],
    disowned: &[Whose],
) -> Swept {
    let paired = || unplaceable.iter().zip(disowned.iter());
    let ours = |wanted: fn(&Unplaceable<'_>) -> bool| {
        paired()
            .filter(|(container, whose)| **whose == Whose::Ours && wanted(container))
            .count()
    };
    let counted = |wanted: Whose| disowned.iter().filter(|whose| **whose == wanted).count();

    Swept {
        resumed,
        lost,
        cleared,
        unidentified: ours(|container| matches!(container, Unplaceable::Unidentified(_))),
        forgotten: ours(|container| matches!(container, Unplaceable::Forgotten(..))),
        unclaimed: counted(Whose::Unlabelled),
        elsewhere: counted(Whose::Elsewhere),
    }
}

impl Running {
    /// The timer that asks, every so often, which containers still deserve
    /// to be up. A generic wake, remembered by its identifier as this one.
    pub fn settle_later(&mut self) -> Effect {
        let id = self.effect_id();
        self.timers.insert(id, crate::Timer::Settling);
        Effect::Wake {
            id,
            after: SETTLING_INTERVAL,
        }
    }
}

impl Running {
    /// What the instance does on waking, given what the runtime holds.
    ///
    /// The last piece of
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`. It walks two
    /// directions, because they answer different questions: from containers
    /// to jobs, anything the instance cannot place is removed if it is ours
    /// and reported otherwise; from jobs to containers, every job that is not
    /// over either has a container or is lost, and every working one is put
    /// back to work.
    pub fn waking(&mut self, containers: &[Container]) -> (Vec<Effect>, Swept) {
        let mut effects = Vec::new();
        let left: Vec<Left<'_>> = containers.iter().map(Left::of).collect();

        // First, because everything below reasons about containers a job
        // still needs, and these belong to nothing this instance knows.
        let (unplaceable, disowned) = self.disowned(&left, &mut effects);

        // A retirement writes the record before removing the container, so
        // one left here is a retirement interrupted, and this finishes it.
        let cleared = over(&left, &self.state);
        for job in &cleared {
            tracing::info!(%job, "removing the container of a job that is over");
            let discard = self.discard(stageman_job::container(*job));
            effects.push(discard);
        }

        // Anything with nothing to run in is over: the session lived in that
        // container. Asked of every job that is not over, not only the ones
        // believed to be working — an idle job whose container has gone would
        // otherwise look answerable for ever.
        let lost: Vec<JobId> = self
            .state
            .unfinished()
            .filter(|job| !has_container(&left, *job))
            .collect();
        for job in &lost {
            tracing::warn!(%job, "its container is gone, so the job is lost");
            self.record(*job, Progress::Retired(Outcome::Lost));
        }

        // Every working job left has a container, and its record is already
        // on the disk, so resuming waits for nothing.
        let resuming: Vec<JobId> = self.state.working().collect();
        for job in &resuming {
            let Some((thread, kit)) = self.recorded(*job) else {
                continue;
            };
            let speaker = Speaker::Job(*job);
            let warrant = self.warrant(speaker, thread);
            let first = self.turn(
                speaker,
                Turn::quiet(Run::Resume {
                    container: stageman_job::container(*job),
                    kit,
                    warrant,
                    tools: self.tools.clone(),
                    text: stageman_foreman::resumption_notice().to_owned(),
                }),
            );
            effects.push(first);
        }

        // A hard kill leaves containers running, and nothing of this
        // project's ran to stop them. Every idle job's container that is up
        // is asked whether it is still showing something.
        let up: Vec<JobId> = left
            .iter()
            .filter(|container| container.running)
            .filter_map(|container| container.job)
            .collect();
        let (placed, _) = resting(&up, &self.state);
        for job in placed {
            self.probe(job, &mut effects);
        }

        let reclaiming = self.ask(&Command::Images, Asked::Images);
        effects.push(reclaiming);
        // Every bound channel is listened to from now: a project that
        // listens is one whose people can reach its jobs.
        let watched: Vec<ProjectId> = self.state.projects.keys().copied().collect();
        for project in watched {
            if let Some(question) = self.listen(project) {
                effects.push(question);
            }
        }
        let settling = self.settle_later();
        effects.push(settling);

        // A foreman found holding a message was interrupted mid-turn, and
        // nothing else would ever drive it again: only an arrival that finds
        // it idle does, and it is not idle. See
        // `docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`.
        for project in crate::foreman::interrupted(&self.state) {
            tracing::info!(%project, "its foreman was interrupted mid-turn; picking it up");
            self.pick_up(project);
        }

        (
            effects,
            tallied(
                resuming.len(),
                lost.len(),
                cleared.len(),
                &unplaceable,
                &disowned,
            ),
        )
    }

    /// Deals with every container the instance cannot account for.
    ///
    /// A phase of its own because it is the one that *destroys* something,
    /// and it answers a question none of the others ask: not "what should this
    /// job do next" but "is this container mine at all". Answers with what it
    /// found and what it decided about each, in the same order, so the tally
    /// can pair them.
    fn disowned<'a>(
        &mut self,
        left: &[Left<'a>],
        effects: &mut Vec<Effect>,
    ) -> (Vec<Unplaceable<'a>>, Vec<Whose>) {
        let unplaceable = unplaceable(left, &self.state);
        let mut disowned = Vec::with_capacity(unplaceable.len());
        for container in &unplaceable {
            let whose = belonging(labelled(left, container.named()), self.id);
            match (whose, container) {
                (Whose::Ours, Unplaceable::Unidentified(name)) => tracing::warn!(
                    container = %name,
                    "a container of this instance's, under a name it does not understand; removed"
                ),
                (Whose::Ours, Unplaceable::Forgotten(name, job)) => tracing::warn!(
                    container = %name,
                    %job,
                    "a container of this instance's naming a job it has no record of — work may \
                     have been lost; removed"
                ),
                (Whose::Elsewhere, _) => tracing::debug!(
                    container = %container.named(),
                    "a container belonging to another instance; left alone"
                ),
                (Whose::Unlabelled, _) => tracing::warn!(
                    container = %container.named(),
                    "a container this project started before instances were told apart, so it \
                     cannot be attributed; left alone rather than removed"
                ),
            }
            if whose == Whose::Ours {
                let discard = self.discard(container.named().to_owned());
                effects.push(discard);
            }
            disowned.push(whose);
        }
        (unplaceable, disowned)
    }

    /// What to do with the containers the world says are up.
    ///
    /// The other half of
    /// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`:
    /// a container held open because its tunnel answered does not stop when
    /// that server does, so something has to ask again. Jobs believed to be
    /// working are passed over. Anything this instance has no record of is
    /// asked about only if its label says it is ours — another instance's
    /// container is very likely mid-turn, and one that cannot say is not worth
    /// guessing about when the cost of guessing wrong is somebody's work.
    pub fn listed(&mut self, running: &[Container], effects: &mut Vec<Effect>) {
        let up: Vec<JobId> = running
            .iter()
            .filter_map(|container| stageman_job::job_of(&container.name))
            .collect();
        let (placed, unplaced) = resting(&up, &self.state);
        for job in placed {
            self.probe(job, effects);
        }
        for job in unplaced {
            let started = running
                .iter()
                .find(|container| stageman_job::job_of(&container.name) == Some(job))
                .and_then(|container| container.instance);
            if belonging(started, self.id) == Whose::Ours {
                self.probe(job, effects);
            } else {
                tracing::debug!(
                    %job,
                    "a container up for work this instance has no record of; left running"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Left, Swept, Unplaceable, Whose, belonging, has_container, over, resting, tallied,
        unplaceable,
    };
    use stageman_core::{
        Agent, AgentConfig, InstanceId, Job, JobId, Kit, KitConfig, KitName, Outcome, Progress,
        Project, ProjectId, Secret, State, Timestamp, Uuid, Waiting,
    };
    use std::collections::BTreeMap;

    fn instance() -> InstanceId {
        InstanceId::from_uuid(Uuid::from_u128(1))
    }

    /// An instance watching one project, with one job on it, running.
    fn with_a_running_job() -> (State, ProjectId, JobId) {
        let project = ProjectId::from_uuid(Uuid::from_u128(1));
        let job = JobId::from_uuid(Uuid::from_u128(2));
        let mut state = State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("agent-token".to_owned()),
                },
            )]),
            ..State::default()
        };
        state.projects.insert(
            project,
            Project {
                name: "example".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: BTreeMap::from([(
                    KitName::new("Claude").expect("a name"),
                    KitConfig::defaults(Agent::Claude),
                )]),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                jobs: BTreeMap::from([(
                    job,
                    Job::new(
                        Kit::defaults(Agent::Claude),
                        "an issue was opened".to_owned(),
                        "work on it".to_owned(),
                        Timestamp::UNIX_EPOCH,
                    ),
                )]),
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
            },
        );
        (state, project, job)
    }

    fn left(name: &str) -> Left<'_> {
        Left {
            name,
            job: stageman_job::job_of(name),
            instance: None,
            running: false,
        }
    }

    /// A container is removed only when its label names this instance.
    ///
    /// The comparison the whole sweep rests on. Inverted, an instance would
    /// remove every container except its own, which on a shared runtime is
    /// the worst outcome available.
    #[test]
    fn only_a_container_this_instance_labelled_is_ours_to_remove() {
        let theirs = InstanceId::from_uuid(Uuid::from_u128(2));

        assert_eq!(belonging(Some(instance()), instance()), Whose::Ours);
        assert_eq!(belonging(Some(theirs), instance()), Whose::Elsewhere);
        assert_eq!(
            belonging(None, instance()),
            Whose::Unlabelled,
            "a container that says nothing must never be taken for ours",
        );
    }

    /// The distinction that decides what an operator does next.
    #[test]
    fn a_name_that_cannot_be_read_and_one_naming_a_lost_job_are_told_apart() {
        let (state, _, known) = with_a_running_job();
        let lost = JobId::from_uuid(Uuid::from_u128(404));
        let lost_name = stageman_job::container(lost);
        let known_name = stageman_job::container(known);
        let containers = [
            left("stageman-job-from-an-older-scheme"),
            left(&lost_name),
            left(&known_name),
        ];

        assert_eq!(
            unplaceable(&containers, &state),
            vec![
                Unplaceable::Unidentified("stageman-job-from-an-older-scheme"),
                Unplaceable::Forgotten(&lost_name, lost),
            ],
            "a container for a job the instance still has is placeable"
        );
    }

    /// A foreman's container is placed, not reported.
    #[test]
    fn a_foremans_container_is_placed_rather_than_reported() {
        let (mut state, watched, _) = with_a_running_job();
        let named = stageman_foreman::container(watched);

        assert_eq!(unplaceable(&[left(&named)], &state), vec![]);

        let gone = stageman_foreman::container(ProjectId::from_uuid(Uuid::from_u128(404)));
        assert_eq!(
            unplaceable(&[left(&gone)], &state),
            vec![Unplaceable::Unidentified(&gone)],
            "one whose project is gone is a loss, like a forgotten job's"
        );

        state.projects.clear();
        assert_eq!(
            unplaceable(&[left(&named)], &state),
            vec![Unplaceable::Unidentified(&named)],
            "which makes this about the record rather than the name"
        );
    }

    /// Only containers of jobs that are over are cleared, and every one is.
    #[test]
    fn only_the_containers_of_jobs_that_are_over_are_cleared() {
        let (mut state, project, busy) = with_a_running_job();
        let ended = JobId::from_uuid(Uuid::from_u128(3));
        let idle = JobId::from_uuid(Uuid::from_u128(4));
        for (id, progress) in [
            (ended, Progress::Retired(Outcome::Done)),
            (idle, Progress::Idle(Waiting::Asked)),
        ] {
            let mut job = Job::new(
                Kit::defaults(Agent::Claude),
                "a reason".to_owned(),
                "some work".to_owned(),
                Timestamp::UNIX_EPOCH,
            );
            job.progress = progress;
            state
                .projects
                .get_mut(&project)
                .expect("the project")
                .jobs
                .insert(id, job);
        }
        let names = [
            stageman_job::container(ended),
            stageman_job::container(idle),
            stageman_job::container(busy),
            stageman_job::container(JobId::from_uuid(Uuid::from_u128(0x77))),
            "stageman-foreman-something".to_owned(),
        ];
        let containers: Vec<Left<'_>> = names.iter().map(|name| left(name)).collect();

        assert_eq!(over(&containers, &state), vec![ended]);
    }

    #[test]
    fn a_job_is_only_matched_to_a_container_that_names_it() {
        let job = JobId::from_uuid(Uuid::from_u128(7));
        let other = JobId::from_uuid(Uuid::from_u128(8));
        let other_name = stageman_job::container(other);
        let containers = [left("stageman-job-unidentified"), left(&other_name)];

        assert!(!has_container(&containers, job));
        assert!(has_container(&containers, other));
        assert!(!has_container(&[], job));
    }

    /// A container holding a turn is never asked to stop; every other one is.
    #[test]
    fn settling_asks_about_everything_up_except_a_job_still_working() {
        let (mut state, project, working) = with_a_running_job();
        let idle = JobId::from_uuid(Uuid::from_u128(13));
        let unknown = JobId::from_uuid(Uuid::from_u128(99));
        let mut resting_job = state.job(working).expect("the running job").clone();
        resting_job.progress = Progress::Idle(Waiting::Silent);
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .jobs
            .insert(idle, resting_job);

        let (placed, unplaced) = resting(&[working, idle, unknown], &state);
        assert_eq!(placed, vec![idle]);
        assert_eq!(unplaced, vec![unknown]);

        let (placed, unplaced) = resting(&[working], &state);
        assert!(placed.is_empty() && unplaced.is_empty());
    }

    #[test]
    fn a_tally_counts_each_kind_separately() {
        let unplaceable = [
            Unplaceable::Unidentified("older-scheme"),
            Unplaceable::Forgotten("a-name", JobId::from_uuid(Uuid::from_u128(9))),
            Unplaceable::Forgotten("another", JobId::from_uuid(Uuid::from_u128(10))),
            Unplaceable::Unidentified("nobody-knows"),
        ];
        let disowned = [
            Whose::Ours,
            Whose::Ours,
            Whose::Elsewhere,
            Whose::Unlabelled,
        ];

        assert_eq!(
            tallied(2, 1, 3, &unplaceable, &disowned),
            Swept {
                resumed: 2,
                lost: 1,
                cleared: 3,
                unidentified: 1,
                forgotten: 1,
                unclaimed: 1,
                elsewhere: 1,
            }
        );
    }

    #[test]
    fn a_waking_that_found_nothing_counts_nothing() {
        assert_eq!(tallied(0, 0, 0, &[], &[]), Swept::default());
    }

    #[test]
    fn an_unplaceable_container_answers_with_its_own_name() {
        let job = JobId::from_uuid(Uuid::from_u128(7));

        assert_eq!(
            Unplaceable::Unidentified("older-scheme").named(),
            "older-scheme"
        );
        assert_eq!(Unplaceable::Forgotten("a-name", job).named(), "a-name");
    }
}
