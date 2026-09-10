//! A turn: what the instance remembers about one while it runs, and what it
//! records when it ends.

use stageman_agent::{Answer, StopReason};
use stageman_core::{JobId, Progress, Project, Secret, Speaking, State, Thread, Waiting};

use crate::Instance;
use crate::vocabulary::{Effect, Speaker};

/// One turn in flight: what it can be told, and what it has said.
///
/// Both per turn rather than per job, which is why this is a fresh value each
/// time: a stop asked of a turn that has ended would stop the next one, and a
/// claim left over would be recorded against work the agent never described.
pub struct Turn {
    /// Whether a person asked this turn to stop.
    pub stopping: bool,
    /// What the agent said about why it is stopping, if it has said.
    pub claimed: Option<Waiting>,
    /// Whether the job's thread is told when this turn ends.
    ///
    /// A turn a person caused is; one waking put back to work is not, since
    /// nothing changed that the person could act on.
    pub notify: bool,
}

impl Turn {
    /// A turn nobody is told about when it ends.
    pub const fn quiet() -> Self {
        Self {
            stopping: false,
            claimed: None,
            notify: false,
        }
    }

    /// A turn whose ending is said on the job's thread.
    pub const fn noticed() -> Self {
        Self {
            stopping: false,
            claimed: None,
            notify: true,
        }
    }
}

/// What an agent's answer means for the job that produced it.
///
/// Anything short of finishing the turn is a failure, and the stop reason is
/// carried into the message rather than collapsed: a turn cut off by a token
/// limit and one the agent refused are both "not finished", and an operator
/// does something different about each.
///
/// A claim is ignored when the turn did not end cleanly: an agent that said
/// it was ready for review and then ran out of tokens did not finish,
/// whatever it believed a moment earlier. A turn that ended without a claim
/// is *silent*, which is the honest residual rather than a substituted
/// default.
pub fn outcome(answer: &Answer, claimed: Option<Waiting>) -> Progress {
    if answer.stop_reason == StopReason::EndTurn {
        Progress::Idle(claimed.unwrap_or(Waiting::Silent))
    } else {
        Progress::Idle(Waiting::Failed(format!(
            "the agent stopped: {:?}",
            answer.stop_reason
        )))
    }
}

/// Where to speak on a job's behalf, if it has anywhere.
///
/// The several ways of having nowhere — no project, no thread, a thread on a
/// channel the project no longer binds — all answer nothing.
pub fn speaking_for(state: &State, job: JobId) -> Option<(Speaking, Thread)> {
    let project = state.project_of(job)?;
    let thread = state.job(job)?.thread.clone()?;
    let bound = state
        .projects
        .get(&project)?
        .channels
        .get(&thread.channel)?
        .speaking();
    Some((bound, thread))
}

/// What to listen to on one project, if there is anything.
///
/// A binding with no credential to listen with is not listened to, and it is
/// not an error: it looks exactly like a platform that has sent nothing.
pub fn listening_on(project: &Project) -> Option<(Secret, Speaking)> {
    let bound = project.channels.get(&stageman_core::Channel::Slack)?;
    Some((bound.listen_credential.clone()?, bound.speaking()))
}

impl Instance {
    /// What a turn ending means, and what follows from it.
    ///
    /// A completion for a speaker with no turn in flight is one from before a
    /// crash, and is ignored: the registry of turns is held rather than kept,
    /// so a restart begins with none.
    pub fn ended(
        &mut self,
        speaker: Speaker,
        outcome: Result<Answer, String>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(turn) = self.turns.remove(&speaker) else {
            tracing::debug!(
                ?speaker,
                "a turn this instance did not start ended; ignored"
            );
            return;
        };
        self.warrants.retain(|_, known| known.speaker != speaker);
        let Speaker::Job(job) = speaker else {
            tracing::info!(?speaker, "a foreman's turn ended");
            return;
        };

        let progress = if turn.stopping {
            Progress::Idle(Waiting::Paused)
        } else {
            match outcome {
                Ok(answer) => {
                    self.noted(job, answer.reported.clone());
                    self::outcome(&answer, turn.claimed)
                }
                Err(why) => Progress::Idle(Waiting::Failed(why)),
            }
        };
        match &progress {
            Progress::Idle(Waiting::Failed(why)) => {
                tracing::warn!(%job, %why, "the turn did not finish");
            }
            Progress::Idle(Waiting::Paused) => tracing::info!(%job, "a person stopped it"),
            _ => {}
        }
        self.record(job, progress);

        // The container is asked whether it is still showing something, at
        // one of the three moments 0043 names. Inward-facing, so it need not
        // wait for the record to land.
        effects.push(Effect::Probe { job });

        // Said whichever way it went: the agent has already reported for
        // itself if it could, and this says the one thing the agent cannot,
        // which is that it has stopped and a reply now reaches it. Outward,
        // so it waits for the record.
        if turn.notify
            && let Some((speaking, thread)) = speaking_for(&self.state, job)
        {
            self.defer(Effect::Say {
                speaking,
                thread,
                text: stageman_foreman::attention_notice().to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{outcome, speaking_for};
    use stageman_agent::{Answer, StopReason};
    use stageman_core::{
        Agent, AgentConfig, Channel, ChannelConfig, Job, JobId, Kit, KitConfig, KitName, Progress,
        Project, ProjectId, Secret, State, Thread, Timestamp, Uuid, Waiting,
    };
    use std::collections::BTreeMap;

    fn answered(stop_reason: StopReason) -> Answer {
        Answer {
            text: "whatever it said".to_owned(),
            stop_reason,
            reported: BTreeMap::new(),
        }
    }

    /// Finishing the turn is the only thing that counts as having finished,
    /// and finishing without a claim is silent rather than anything more
    /// flattering.
    #[test]
    fn only_a_finished_turn_counts_as_a_completed_job() {
        assert_eq!(
            outcome(&answered(StopReason::EndTurn), None),
            Progress::Idle(Waiting::Silent)
        );
    }

    #[test]
    fn a_finished_turn_is_recorded_as_what_its_agent_claimed() {
        for claimed in [Waiting::Asked, Waiting::Proposed] {
            assert_eq!(
                outcome(&answered(StopReason::EndTurn), Some(claimed.clone())),
                Progress::Idle(claimed),
            );
        }
    }

    /// A claim is ignored when the turn did not end cleanly.
    #[test]
    fn a_claim_does_not_survive_a_turn_that_went_wrong() {
        assert!(matches!(
            outcome(&answered(StopReason::MaxTokens), Some(Waiting::Proposed)),
            Progress::Idle(Waiting::Failed(_)),
        ));
    }

    /// Every other way a turn can end is a failure, each checked rather than
    /// one standing in for the rest.
    #[test]
    fn every_other_ending_is_a_failure() {
        for ending in [
            StopReason::MaxTokens,
            StopReason::MaxTurnRequests,
            StopReason::Refusal,
            StopReason::Cancelled,
        ] {
            assert!(
                matches!(
                    outcome(&answered(ending), None),
                    Progress::Idle(Waiting::Failed(_))
                ),
                "{ending:?} should not read as success"
            );
        }
    }

    /// The stop reason survives into the message.
    #[test]
    fn a_failure_says_how_the_turn_ended() {
        let Progress::Idle(Waiting::Failed(why)) = outcome(&answered(StopReason::Refusal), None)
        else {
            panic!("a refusal is not a completed job");
        };

        assert!(why.contains("Refusal"), "{why}");
    }

    /// Where to speak, and the several ways of having nowhere.
    #[test]
    fn a_job_with_no_thread_has_nowhere_to_be_spoken_to() {
        let project = ProjectId::from_uuid(Uuid::from_u128(11));
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
                        "started by hand".to_owned(),
                        "do the thing".to_owned(),
                        Timestamp::UNIX_EPOCH,
                    ),
                )]),
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
            },
        );

        assert!(speaking_for(&state, job).is_none(), "no thread");

        state.job_mut(job).expect("the job").thread = Some(Thread {
            channel: Channel::Slack,
            id: "1728312345.678901".to_owned(),
        });
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .channels
            .insert(
                Channel::Slack,
                ChannelConfig {
                    address: "C0123456789".to_owned(),
                    credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                    listen_credential: None,
                },
            );
        let (bound, thread) = speaking_for(&state, job).expect("somewhere to speak");
        assert_eq!(bound.address, "C0123456789");
        assert_eq!(thread.id, "1728312345.678901");

        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .channels
            .clear();
        assert!(
            speaking_for(&state, job).is_none(),
            "a thread naming a channel the project no longer binds is nowhere again"
        );
    }
}
