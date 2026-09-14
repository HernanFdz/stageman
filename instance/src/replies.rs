//! What a message heard on a channel is for, and what a job does with a
//! reply.
//!
//! Which job a message is *for* is `State::recipient`, in the domain, where it
//! is tested against every combination rather than the ones a live workspace
//! happens to produce. What is here is what follows from the answer.

use stageman_core::{Arriving, Channel, ChannelConfig, JobId, Progress, Recipient, State, Thread};

use crate::Running;
use crate::turns::{Run, Turn, speaking_for};
use crate::vocabulary::{AppEffect, Message, Speaker};
use crate::{Effect, Emit as _};

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
    /// Hands one message to whoever it is for.
    pub fn heard(&mut self, channel: Channel, message: &Message, effects: &mut Vec<Effect>) {
        let arriving = Arriving {
            address: &message.address,
            id: &message.id,
            thread: message.thread.as_deref(),
            mentions: message.mentions,
            from_us: message.from_us,
        };
        match self.state.recipient(channel, &arriving) {
            Recipient::Job(job) => {
                tracing::info!(%job, "handing a reply to the job whose thread it is in");
                self.replied(job, &message.text);
            }
            Recipient::Foreman(project) => {
                tracing::info!(%project, "a message for the foreman");
                self.for_foreman(project, channel, message);
            }
            // Answered rather than ignored. They addressed this instance, so
            // silence would read as broken — and this is where somebody lands
            // by replying to a foreman's own message.
            Recipient::NoSuchJob(project) => {
                tracing::info!(%project, "a message in a thread belonging to no job");
                let Some(id) = message.thread.as_deref() else {
                    return;
                };
                let Some(speaking) = self
                    .state
                    .projects
                    .get(&project)
                    .and_then(|watched| watched.channels.get(&channel))
                    .map(ChannelConfig::speaking)
                else {
                    return;
                };
                effects.emit(AppEffect::Say {
                    speaking: speaking.into(),
                    thread: Thread {
                        channel,
                        id: id.to_owned(),
                    },
                    text: stageman_foreman::no_such_job_notice().to_owned(),
                });
            }
            // Ordinary — most of what is said in a project's channel is people
            // talking to each other.
            Recipient::Nobody => tracing::debug!("nobody that message was for"),
        }
    }

    /// Gives a reply to the job whose thread it arrived in, if it can take
    /// one, and says which otherwise.
    ///
    /// A taken reply is a turn, and the turn waits for the record that the
    /// job is working to land: the record before the container, as
    /// everywhere. A refusal changes nothing, so the notice of it is said at
    /// once.
    fn replied(&mut self, job: JobId, said: &str) {
        match accepting(&mut self.state, job) {
            Accepted::Taken => {
                // `accepting` wrote the state; this is the one writer that
                // does not go through `record`, so it says so itself.
                self.dirty = true;
                let Some((thread, kit)) = self.recorded(job) else {
                    return;
                };
                let speaker = Speaker::Job(job);
                let warrant = self.warrant(speaker, thread);
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
                        text: stageman_foreman::reply(said),
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

    /// Says something in a job's thread, if it has one, without waiting for
    /// anything: a notice about a refusal changes no state.
    fn notice(&mut self, job: JobId, text: &str) {
        if let Some((speaking, thread)) = speaking_for(&self.state, job) {
            self.defer(AppEffect::Say {
                speaking: speaking.into(),
                thread,
                text: text.to_owned(),
            });
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
