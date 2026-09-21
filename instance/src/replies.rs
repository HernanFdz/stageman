//! What a message heard on a channel is for, and what a job does with a
//! reply.
//!
//! Which job a message is *for* is `State::recipient`, in the domain, where it
//! is tested against every combination rather than the ones a live workspace
//! happens to produce. What is here is what follows from the answer.

use stageman_core::{Arriving, Channel, JobId, Place, Progress, ProjectId, Recipient, Room, State};

use crate::Running;
use crate::turns::{Run, Turn, speaking_for};
use stageman_channel::Message;

use crate::vocabulary::Speaker;

/// What the gate decided when a reply arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    /// The job was idle and is now working.
    Taken,
    /// It was already working, so nothing was taken.
    Busy,
    /// It is over, so nothing can be given to it ever again.
    ///
    /// Apart from [`Accepted::Unknown`] because a person is told something
    /// different: this thread did belong to a job, so saying something here
    /// was not a mistake about where to say it.
    Over,
    /// This instance has no such job.
    Unknown,
}

/// Whether a job can take a reply now, taking it if so.
///
/// **The check and the transition are one operation on purpose**, and the
/// step it runs in is what serialises them: two replies arriving together are
/// two steps, the second finds the job working, and it is refused. That makes
/// the refusal a person sees for a genuinely busy job and the refusal that
/// prevents two turns resuming one container the same code, which is right —
/// from the outside they are the same situation.
pub fn accepting(state: &mut State, job: JobId) -> Accepted {
    let Some(recorded) = state.job_mut(job) else {
        return Accepted::Unknown;
    };
    match recorded.progress {
        Progress::Working => Accepted::Busy,
        // Over, so there is nothing to resume: the container went with the
        // retirement and the session with it.
        Progress::Retired(_) => Accepted::Over,
        Progress::Idle(_) => {
            recorded.progress = Progress::Working;
            Accepted::Taken
        }
    }
}

impl Running {
    /// Hands one message heard on a project's socket to whoever it is for.
    pub fn heard(&mut self, project: ProjectId, channel: Channel, message: &Message) {
        let arriving = Arriving {
            room: &message.room,
            id: &message.id,
            thread: message.thread.as_deref(),
            from_us: message.from_us,
            from_app: message.app.is_some(),
        };
        match self.state.recipient(project, channel, &arriving) {
            Recipient::Job(job) => {
                tracing::info!(%job, "handing a reply to the job whose room it is in");
                self.replied(project, job, channel, message);
            }
            Recipient::Foreman(project) => {
                if let Some(app) = &message.app {
                    tracing::info!(%project, %app, "a signal for the foreman");
                } else {
                    tracing::info!(%project, "a message for the foreman");
                }
                self.for_foreman(project, channel, message);
            }
            // Ordinary: what this instance said itself, heard back, or
            // another app posting in a room nobody watches.
            Recipient::Nobody => tracing::debug!("nobody that message was for"),
        }
    }

    /// Gives a reply to the job whose room it arrived in, if it can take
    /// one, and says which otherwise.
    ///
    /// A taken reply is a turn, and the turn waits for the record that the
    /// job is working to land: the record before the container, as
    /// everywhere. A refusal changes nothing, so the notice of it is said at
    /// once. The turn speaks where the reply was said, in the thread if it
    /// was in one; the root of the room is told why the turn started, with
    /// a link to the reply, and told when it ended, because both are about
    /// the job rather than part of the exchange.
    fn replied(&mut self, project: ProjectId, job: JobId, channel: Channel, message: &Message) {
        // Answered where it was said: in the thread the person asked in, or
        // at the root of the room.
        let place = Place {
            room: Room {
                channel,
                id: message.room.clone(),
            },
            thread: message.thread.clone(),
        };
        let said = message.text.as_str();
        // The thread to answer under, as the agent is shown it: the one the
        // reply was in, or the reply itself.
        let target = stageman_channel::reference(
            channel,
            &message.room,
            message.thread.as_deref().unwrap_or(&message.id),
        );
        match accepting(&mut self.state, job) {
            Accepted::Taken => {
                // `accepting` wrote the state; this is the one writer that
                // does not go through `record`, so it says so itself.
                self.dirty = true;
                let Some((_, kit)) = self.recorded(job) else {
                    return;
                };
                let link = self.permalink(
                    project,
                    channel,
                    &message.room,
                    &message.id,
                    message.thread.as_deref(),
                );
                self.notice(
                    job,
                    &stageman_foreman::turn_notice(&stageman_foreman::Because::Message(
                        link.as_deref(),
                    )),
                );
                let speaker = Speaker::Job(job);
                let warrant = self.warrant(speaker, Some(place), None);
                // Resuming starts the container, which publishes its tunnel
                // on a fresh port.
                self.forget_tunnel(job);
                let first = self.turn(
                    speaker,
                    Turn::noticed(Run::Resume {
                        container: stageman_job::container(job),
                        kit,
                        warrant,
                        tools: self.tools.clone(),
                        text: stageman_foreman::reply(said, &target),
                    }),
                );
                self.defer(first);
            }
            Accepted::Busy => self.notice(job, stageman_foreman::busy_notice()),
            Accepted::Over => self.notice(job, stageman_foreman::over_notice()),
            Accepted::Unknown => {
                tracing::warn!(%job, "a reply for a job this instance does not have");
            }
        }
    }

    /// Says something at the root of a job's room, if it has one, once
    /// whatever this step changed is on the disk — which for a refusal is
    /// nothing, so it goes at once.
    pub(crate) fn notice(&mut self, job: JobId, text: &str) {
        if let Some((speaking, place)) = speaking_for(&self.state, job) {
            self.say(&speaking, &place, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Accepted, accepting};
    use stageman_core::{
        Agent, AgentConfig, Job, JobId, Kit, KitConfig, KitName, Outcome, Progress, Project,
        ProjectId, Secret, State, Timestamp, Uuid, Waiting,
    };
    use std::collections::BTreeMap;

    fn an_instance_with_a_job() -> (State, JobId) {
        let job = JobId::from_uuid(Uuid::from_u128(12));
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
            ProjectId::from_uuid(Uuid::from_u128(11)),
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
                        "started by hand".to_owned(),
                        "do the thing".to_owned(),
                        Timestamp::UNIX_EPOCH,
                    ),
                )]),
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
                brief: String::new(),
                watched: std::collections::BTreeSet::new(),
                foreman_room: None,
            },
        );
        (state, job)
    }

    /// The rule that serialises replies, and the one the user meets.
    #[test]
    fn a_reply_is_taken_only_by_a_job_that_is_not_working() {
        let (mut state, job) = an_instance_with_a_job();

        assert_eq!(accepting(&mut state, job), Accepted::Busy);
        assert_eq!(
            state.job(job).expect("the job").progress,
            Progress::Working,
            "a refused reply must not move the job"
        );

        state.job_mut(job).expect("the job").progress = Progress::Idle(Waiting::Silent);
        assert_eq!(accepting(&mut state, job), Accepted::Taken);
        assert_eq!(state.job(job).expect("the job").progress, Progress::Working);

        // Taking it once is what stops a second taking it as well.
        assert_eq!(accepting(&mut state, job), Accepted::Busy);
    }

    /// A reply is taken by any reading of idle, and by no other state.
    #[test]
    fn a_reply_is_taken_by_every_reading_of_idle() {
        for waiting in [
            Waiting::Asked,
            Waiting::Proposed,
            Waiting::Paused,
            Waiting::Silent,
            Waiting::Failed("the credential had expired".to_owned()),
        ] {
            let (mut state, job) = an_instance_with_a_job();
            state.job_mut(job).expect("the job").progress = Progress::Idle(waiting.clone());

            assert_eq!(accepting(&mut state, job), Accepted::Taken, "{waiting:?}");
            assert_eq!(state.job(job).expect("the job").progress, Progress::Working);
        }
    }

    /// A job that is over takes nothing, and keeps the verdict it was given.
    #[test]
    fn a_reply_to_a_job_that_is_over_is_refused_and_changes_nothing() {
        for outcome in [Outcome::Done, Outcome::Discarded, Outcome::Lost] {
            let (mut state, job) = an_instance_with_a_job();
            state.job_mut(job).expect("the job").progress = Progress::Retired(outcome);

            assert_eq!(accepting(&mut state, job), Accepted::Over, "{outcome:?}");
            assert_eq!(
                state.job(job).expect("the job").progress,
                Progress::Retired(outcome)
            );
        }
    }

    #[test]
    fn a_reply_for_a_job_this_instance_does_not_have_is_unknown() {
        let (mut state, _) = an_instance_with_a_job();

        assert_eq!(
            accepting(&mut state, JobId::from_uuid(Uuid::from_u128(99))),
            Accepted::Unknown
        );
    }
}
