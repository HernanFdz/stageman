//! What a message heard on a channel is for, and what a job does with a
//! message: the inbox of
//! `docs/decisions/0069-a-message-reaches-a-working-job.md`.
//!
//! Which job a message is *for* is `State::recipient`, in the domain, where it
//! is tested against every combination rather than the ones a live workspace
//! happens to produce. What is here is what follows from the answer: a
//! message is received in the step it arrives, into the job's inbox — in
//! hand if the job was idle, which is what puts it to work, and behind what
//! it holds otherwise — and delivered from there: by starting a turn, or
//! by being handed to the one running, when its conversation is open and
//! the adapter takes it. One at a time, the next after the previous is
//! answered, so that arrival order holds.
//!
//! **Nothing running means nothing waiting.** A job that is not working has
//! an empty inbox: a turn's end starts the next message's turn at once, a
//! person's stop tells every message still waiting that it will not be
//! delivered, and a turn that never started tells them the same. Kept by
//! those three transitions, in `crate::turns`, and checked by the
//! simulation after every step.

use stageman_core::{
    Arriving, Channel, Errand, JobId, Place, Progress, ProjectId, Recipient, Room, State, Thread,
    Timestamp, Waiting,
};
use stageman_foreman::Finding;

use crate::Running;
use crate::turns::{Delivery, Run, Turn, speaking_for};
use stageman_channel::{Message, Reaction};

use crate::vocabulary::Speaker;

/// What became of a message for a job when it arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    /// The job was idle: the message is in hand and the job is working.
    Started {
        /// Whether a person had stopped the job, in which case the agent is
        /// told it was interrupted rather than that it stopped by itself.
        interrupted: bool,
    },
    /// It was working, so the message waits behind what it holds, or is
    /// handed to the turn if that can be.
    Waiting,
    /// It is over, so nothing can be given to it ever again.
    ///
    /// Apart from [`Accepted::Unknown`] because a person is told something
    /// different: this room did belong to a job, so saying something here
    /// was not a mistake about where to say it.
    Over,
    /// This instance has no such job.
    Unknown,
}

/// Receives a message for a job, in the step it arrives.
///
/// **The check and the transition are one operation on purpose**, and the
/// step it runs in is what serialises them: two messages arriving together
/// are two steps, the first finds the job idle and starts it, and the second
/// finds it working and waits. That is what makes one turn per job at a time
/// the inbox's property, as `docs/decisions/0044-a-listener-only-listens.md`
/// asks of arrival order and 0069 asks of the inbox.
pub fn accepting(state: &mut State, job: &JobId, errand: Errand, now: Timestamp) -> Accepted {
    let Some(recorded) = state.job_mut(job) else {
        return Accepted::Unknown;
    };
    match &recorded.progress {
        Progress::Working => {
            recorded.inbox.receive(errand);
            Accepted::Waiting
        }
        // Over, so there is nothing to resume: the container went with the
        // retirement and the session with it.
        Progress::Retired(_) => Accepted::Over,
        Progress::Idle(waiting) => {
            let interrupted = matches!(waiting, Waiting::Paused);
            recorded.inbox.receive(errand);
            recorded.inbox.give();
            recorded.progress = Progress::Working;
            recorded.since = Some(now);
            Accepted::Started { interrupted }
        }
    }
}

/// A message heard, as a job's inbox holds it: the words, where an answer
/// belongs — the thread it was in, or itself — who said it, and what
/// identifies it, to react on.
fn errand_of(channel: Channel, message: &Message) -> Errand {
    Errand {
        said: message.text.clone(),
        thread: Thread {
            channel,
            room: message.room.clone(),
            id: message.thread.clone().unwrap_or_else(|| message.id.clone()),
        },
        from: message.user.clone(),
        message: Some(message.id.clone()),
        app: message.app.clone(),
    }
}

/// Whether a message in a job's inbox was said in a thread, rather than at
/// the root: where an answer belongs is then not the message itself.
fn in_thread(errand: &Errand) -> bool {
    errand.message.as_deref() != Some(errand.thread.id.as_str())
}

/// What identifies the message in hand, for what is shown of its thread to
/// stop at: the message itself, or the thread it started when nothing said.
fn handling(errand: &Errand) -> &str {
    errand.message.as_deref().unwrap_or(&errand.thread.id)
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
                self.replied(project, &job, channel, message);
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

    /// Receives a message for the job whose room it arrived in.
    ///
    /// Received in this step, whichever way: what put the job to work, or
    /// waiting behind what it holds. Either is a record, and the eyes
    /// reaction waits for it, as a foreman's does. A message that found the
    /// job idle starts a turn — shown its thread first, per
    /// `docs/decisions/0068-a-mention-is-shown-its-thread.md`, when it was
    /// in one — and one that found it working is handed to the turn if that
    /// can be done now. A job that is over refuses, and says so.
    fn replied(&mut self, project: ProjectId, job: &JobId, channel: Channel, message: &Message) {
        let now = self.stamp();
        match accepting(&mut self.state, job, errand_of(channel, message), now) {
            Accepted::Started { interrupted } => {
                // `accepting` wrote the state; this is the one writer that
                // does not go through `record`, so it says so itself.
                self.dirty = true;
                self.react_in(project, channel, &message.room, &message.id, Reaction::Seen);
                let finding = if interrupted {
                    Finding::PartWay
                } else {
                    Finding::AtRest
                };
                self.start_given(project, job, channel, finding);
            }
            Accepted::Waiting => {
                self.dirty = true;
                self.react_in(project, channel, &message.room, &message.id, Reaction::Seen);
                self.try_deliver(project, job, channel);
            }
            Accepted::Over => self.notice(job, stageman_foreman::over_notice()),
            Accepted::Unknown => {
                tracing::warn!(%job, "a reply for a job this instance does not have");
            }
        }
    }

    /// Starts a turn on the message in hand: after its thread is read, when
    /// it was said in one, and at once otherwise.
    pub(crate) fn start_given(
        &mut self,
        project: ProjectId,
        job: &JobId,
        channel: Channel,
        finding: Finding,
    ) {
        let Some(errand) = self
            .state
            .job(job)
            .and_then(|recorded| recorded.inbox.given.last())
            .cloned()
        else {
            return;
        };
        let reading = in_thread(&errand)
            && self.read_thread(
                Speaker::Job(job.clone()),
                crate::threads::Pending::Job {
                    job: job.clone(),
                    finding,
                },
                &errand.thread.room,
                &errand.thread.id,
            );
        if !reading {
            self.resume_job(project, job, channel, finding);
        }
    }

    /// Puts a job back to work on the message in hand, once whatever it was
    /// to be shown of its thread is in hand. The turn speaks where the
    /// message was said, in the thread if it was in one; the root of the
    /// room is told why the turn started, with a link to the message, and
    /// told when it ended, because both are about the job rather than part
    /// of the exchange. The turn waits for every write in flight — the
    /// record that the job is working among them, when it has not landed
    /// yet.
    pub fn resume_job(
        &mut self,
        project: ProjectId,
        job: &JobId,
        channel: Channel,
        finding: Finding,
    ) {
        let Some(errand) = self
            .state
            .job(job)
            .and_then(|recorded| recorded.inbox.given.last())
            .cloned()
        else {
            return;
        };
        let (place, target, link) = self.addressed(project, channel, &errand);
        let speaker = Speaker::Job(job.clone());
        // A job's session is resumed, so it remembers what it was shown.
        let context = self.thread_taken(&speaker).and_then(|read| {
            crate::threads::thread_context(&read, channel, false, handling(&errand))
        });
        let Some((_, kit)) = self.recorded(job) else {
            return;
        };
        self.notice(
            job,
            &stageman_foreman::turn_notice(&stageman_foreman::Because::Message(link.as_deref())),
        );
        let warrant = self.warrant(&speaker, Some(place), None);
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
                text: stageman_foreman::reply(&errand.said, &target, context.as_deref(), finding),
            }),
        );
        self.after_writes(first);
    }

    /// Hands the next message waiting to the job's running turn, if that can
    /// be done now: the turn is talking, nothing is being handed to it, the
    /// adapter has not declined one, no thread is being read for it, and
    /// the adapter said it takes one. Otherwise the message waits, and the
    /// turn's end delivers it.
    ///
    /// The message is in hand before it is handed over, and that is a
    /// record: a crash after the handing finds it in hand and says it again
    /// on resuming, as one that may or may not have been seen, rather than
    /// delivering it twice as new.
    pub(crate) fn try_deliver(&mut self, project: ProjectId, job: &JobId, channel: Channel) {
        let speaker = Speaker::Job(job.clone());
        if self.pending_threads.contains_key(&speaker) {
            return;
        }
        let Some(turn) = self.turns.get(&speaker) else {
            return;
        };
        if turn.delivery != Delivery::Open || !turn.can_steer() {
            return;
        }
        let Some(errand) = self
            .state
            .job_mut(job)
            .and_then(|recorded| recorded.inbox.give())
            .cloned()
        else {
            return;
        };
        self.dirty = true;
        if let Some(turn) = self.turns.get_mut(&speaker) {
            turn.delivery = Delivery::InFlight;
        }
        let reading = in_thread(&errand)
            && self.read_thread(
                speaker,
                crate::threads::Pending::Steer { job: job.clone() },
                &errand.thread.room,
                &errand.thread.id,
            );
        if !reading {
            self.steer_given(project, job, channel);
        }
    }

    /// Hands the message in hand to the job's running turn, framed as an
    /// interruption, once whatever it was to be shown of its thread is in
    /// hand. What the adapter answers comes back through the conversation.
    /// A turn that can no longer take it — ended meanwhile — hands it back
    /// to wait for the next.
    pub(crate) fn steer_given(&mut self, project: ProjectId, job: &JobId, channel: Channel) {
        let speaker = Speaker::Job(job.clone());
        let Some(errand) = self
            .state
            .job(job)
            .and_then(|recorded| recorded.inbox.given.last())
            .cloned()
        else {
            return;
        };
        let (_, target, _) = self.addressed(project, channel, &errand);
        let context = self.thread_taken(&speaker).and_then(|read| {
            crate::threads::thread_context(&read, channel, false, handling(&errand))
        });
        let text =
            stageman_foreman::reply(&errand.said, &target, context.as_deref(), Finding::PartWay);
        let lines = self
            .turns
            .get_mut(&speaker)
            .and_then(|turn| turn.steer(&text));
        if let Some((process, lines)) = lines {
            for line in lines {
                self.after_writes(stageman_vocabulary::Effect::Send { id: process, line });
            }
        } else {
            if let Some(turn) = self.turns.get_mut(&speaker) {
                turn.delivery = Delivery::Open;
            }
            if let Some(recorded) = self.state.job_mut(job) {
                recorded.inbox.hand_back();
                self.dirty = true;
            }
        }
    }

    /// Where a message in a job's inbox was said, as a turn answers there;
    /// how the agent is told to name it; and a link to it, when the
    /// listener has been told where the workspace is.
    pub(crate) fn addressed(
        &self,
        project: ProjectId,
        channel: Channel,
        errand: &Errand,
    ) -> (Place, String, Option<String>) {
        // Answered where it was said: in the thread the person asked in, or
        // at the root of the room.
        let thread = in_thread(errand).then(|| errand.thread.id.clone());
        let place = Place {
            room: Room {
                channel,
                id: errand.thread.room.clone(),
            },
            thread,
        };
        // The thread to answer under, as the agent is shown it: the one the
        // message was in, or the message itself.
        let target = stageman_channel::reference(channel, &errand.thread.room, &errand.thread.id);
        let link = errand.message.as_deref().and_then(|message| {
            self.permalink(
                project,
                channel,
                &errand.thread.room,
                message,
                place.thread.as_deref(),
            )
        });
        (place, target, link)
    }

    /// Says something at the root of a job's room, if it has one, once
    /// whatever this step changed is on the disk — which for a refusal is
    /// nothing, so it goes at once.
    pub(crate) fn notice(&mut self, job: &JobId, text: &str) {
        if let Some((speaking, place)) = speaking_for(&self.state, job) {
            self.say(&speaking, &place, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Accepted, accepting};
    use stageman_core::{
        Agent, AgentConfig, Channel, Errand, Job, JobId, Kit, KitConfig, KitName, Outcome,
        Progress, Project, ProjectId, Secret, State, Thread, Timestamp, Uuid, Waiting,
    };
    use std::collections::BTreeMap;

    /// A message for a job, at the root of its room.
    fn said(text: &str) -> Errand {
        Errand {
            said: text.to_owned(),
            thread: Thread {
                channel: Channel::Slack,
                room: "C0123".to_owned(),
                id: "1788000000.000100".to_owned(),
            },
            from: Some("U0HUMAN".to_owned()),
            message: Some("1788000000.000100".to_owned()),
            app: None,
        }
    }

    fn an_instance_with_a_job() -> (State, JobId) {
        let job = JobId::from_uuid(Uuid::from_u128(12));
        let mut state = State {
            apps: std::collections::BTreeMap::new(),
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
                    job.clone(),
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

    /// The rule that serialises messages: a message finding the job idle
    /// puts it to work with the message in hand, and every one finding it
    /// working waits behind what it holds, in order. Receiving is what
    /// stops a second message starting a turn as well.
    #[test]
    fn a_message_starts_an_idle_job_and_waits_behind_a_working_one() {
        let (mut state, job) = an_instance_with_a_job();

        assert_eq!(
            accepting(&mut state, &job, said("first"), Timestamp::UNIX_EPOCH),
            Accepted::Waiting
        );
        let recorded = state.job(&job).expect("the job");
        assert_eq!(recorded.progress, Progress::Working, "already working");
        assert!(
            recorded.inbox.given.is_empty(),
            "the kickoff is not a message"
        );
        assert_eq!(recorded.inbox.waiting.len(), 1);

        state.job_mut(&job).expect("the job").inbox.drain();
        state.job_mut(&job).expect("the job").progress = Progress::Idle(Waiting::Silent);
        assert_eq!(
            accepting(&mut state, &job, said("second"), Timestamp::UNIX_EPOCH),
            Accepted::Started { interrupted: false }
        );
        let recorded = state.job(&job).expect("the job");
        assert_eq!(recorded.progress, Progress::Working);
        assert_eq!(
            recorded
                .inbox
                .given
                .iter()
                .map(|given| given.said.as_str())
                .collect::<Vec<_>>(),
            vec!["second"],
            "in hand, which is what the turn starts on"
        );
        assert!(recorded.inbox.waiting.is_empty());

        // Starting it once is what stops a second starting it as well.
        assert_eq!(
            accepting(&mut state, &job, said("third"), Timestamp::UNIX_EPOCH),
            Accepted::Waiting
        );
        let recorded = state.job(&job).expect("the job");
        assert_eq!(recorded.inbox.given.len(), 1);
        assert_eq!(
            recorded.inbox.next().map(|next| next.said.as_str()),
            Some("third")
        );
    }

    /// A message starts a job in any reading of idle, and in no other
    /// state; one that a person had stopped is told it was interrupted.
    #[test]
    fn a_message_starts_a_job_in_every_reading_of_idle() {
        for waiting in [
            Waiting::Asked,
            Waiting::Proposed,
            Waiting::Paused,
            Waiting::Silent,
            Waiting::Failed("the credential had expired".to_owned()),
        ] {
            let (mut state, job) = an_instance_with_a_job();
            state.job_mut(&job).expect("the job").progress = Progress::Idle(waiting.clone());

            let interrupted = waiting == Waiting::Paused;
            assert_eq!(
                accepting(&mut state, &job, said("go on"), Timestamp::UNIX_EPOCH),
                Accepted::Started { interrupted },
                "{waiting:?}"
            );
            assert_eq!(
                state.job(&job).expect("the job").progress,
                Progress::Working
            );
        }
    }

    /// A job that is over takes nothing, and keeps the verdict it was given.
    #[test]
    fn a_reply_to_a_job_that_is_over_is_refused_and_changes_nothing() {
        for outcome in [Outcome::Done, Outcome::Discarded, Outcome::Lost] {
            let (mut state, job) = an_instance_with_a_job();
            state.job_mut(&job).expect("the job").progress = Progress::Retired(outcome);

            assert_eq!(
                accepting(&mut state, &job, said("go on"), Timestamp::UNIX_EPOCH),
                Accepted::Over,
                "{outcome:?}"
            );
            let recorded = state.job(&job).expect("the job");
            assert_eq!(recorded.progress, Progress::Retired(outcome));
            assert!(
                recorded.inbox.is_empty(),
                "nothing waits for a job that is over"
            );
        }
    }

    #[test]
    fn a_reply_for_a_job_this_instance_does_not_have_is_unknown() {
        let (mut state, _) = an_instance_with_a_job();

        assert_eq!(
            accepting(
                &mut state,
                &JobId::from_uuid(Uuid::from_u128(99)),
                said("anyone?"),
                Timestamp::UNIX_EPOCH
            ),
            Accepted::Unknown
        );
    }
}
