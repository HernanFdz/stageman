//! The foreman's loop: a message arrives, it is taken or queued, and the
//! foreman works until its inbox is empty.
//!
//! One turn is not the unit; draining is. A foreman that took a message,
//! answered it and stopped would leave whatever arrived meanwhile waiting for
//! the next arrival to wake it. So a turn ending picks up the next message,
//! and only an empty inbox returns a foreman to idle — which the shape of
//! `Attending` makes the only way to be idle at all.
//!
//! Whether a turn starts a session or continues one is asked of the runtime,
//! never believed: a container is the truth about whether a session exists.

use stageman_core::{
    Agent, Channel, ChannelConfig, Errand, Handout, ProjectId, State, Taken, Thread,
};
use stageman_foreman::Starting;

use crate::Instance;
use crate::turns::Turn;
use crate::vocabulary::{AppEffect, Message, Run, Speaker};
use crate::{Effect, Emit as _};

/// Every project whose foreman was working when this process last stopped.
///
/// A foreman that is idle is not here, and a foreman that is idle *while
/// something waits* is a state the domain has no way to write down.
pub fn interrupted(state: &State) -> Vec<ProjectId> {
    state
        .projects
        .iter()
        .filter(|(_, watched)| watched.attending.on().is_some())
        .map(|(project, _)| *project)
        .collect()
}

/// What the foreman should be working on now, if anything.
pub fn waiting_on(state: &State, project: ProjectId) -> Option<Errand> {
    state
        .projects
        .get(&project)
        .and_then(|watched| watched.attending.on().cloned())
}

/// What this project's jobs may run on: each kit's name and what it is for.
///
/// Said in the turn's prompt every turn, because a project's kits are edited
/// from the dashboard and a session outlives those edits.
pub fn kits_offered(state: &State, project: ProjectId) -> Vec<(String, String)> {
    state
        .projects
        .get(&project)
        .map_or_else(Vec::new, |watched| {
            watched
                .kits
                .iter()
                .map(|(name, offered)| (name.to_string(), offered.description.clone()))
                .collect()
        })
}

/// Whether a foreman's existing container is kept for the agent it now wants.
///
/// Kept when it was made for that agent, and kept when it cannot say — a
/// container from before the label existed could only have been made for the
/// agent its project named then. Replaced only when the label names a
/// different agent, which is the one case where keeping it would run the
/// wrong image.
///
/// Skipped by mutation testing, and equivalent rather than untested: there is
/// one agent, so a label can never name a different one and this is `true`
/// for every input there is. **Delete this attribute in the commit that adds
/// a second agent**, and give the test below its replaced case.
#[mutants::skip]
fn keeps(made_for: Option<Agent>, wanted: Agent) -> bool {
    made_for.is_none_or(|made| made == wanted)
}

impl Instance {
    /// A message at the root of a project's channel, for its foreman.
    ///
    /// Taken or queued in this step, in the order messages arrive — which is
    /// the whole of what an inbox promises — and acknowledged on its thread
    /// once the inbox is on the disk, so that "got it" is never said of a
    /// message a crash could lose. The message that found the foreman idle
    /// is the one that starts its loop.
    pub fn for_foreman(&mut self, project: ProjectId, channel: Channel, message: &Message) {
        // A message at the root is the parent of the thread its answer
        // belongs under; one in a thread is answered where it was said.
        let thread = Thread {
            channel,
            id: message.thread.clone().unwrap_or_else(|| message.id.clone()),
        };
        let Some(watched) = self.state.projects.get_mut(&project) else {
            return;
        };
        let taken = watched.attending.take(Errand {
            said: message.text.clone(),
            thread: thread.clone(),
        });
        // Counting the one in hand: from outside, everything not yet
        // answered is ahead of this.
        let ahead = match taken {
            Taken::Started => 0,
            Taken::Waiting => watched.attending.waiting(),
        };
        self.dirty = true;
        self.notice_in(project, &thread, &stageman_foreman::received_notice(ahead));
        if taken == Taken::Started {
            self.look_before_turning(project);
        }
    }

    /// Puts a foreman found holding a message on waking back to work.
    ///
    /// Nothing is taken and nothing is re-acknowledged: the arrival already
    /// happened and was already answered when it did. What the thread is
    /// told instead is that the wait had a reason.
    pub fn pick_up(&mut self, project: ProjectId) {
        let Some(errand) = waiting_on(&self.state, project) else {
            return;
        };
        self.notice_in(project, &errand.thread, stageman_foreman::resumed_notice());
        self.interrupted.insert(project);
        self.look_before_turning(project);
    }

    /// Asks the runtime about the foreman's container before turning.
    ///
    /// Held back behind whatever this step changed, so that a turn never
    /// starts on the strength of an inbox that is not on the disk.
    fn look_before_turning(&mut self, project: ProjectId) {
        self.defer(AppEffect::Inspect {
            container: stageman_foreman::container(project),
        });
    }

    /// What the runtime said about a foreman's container, and the turn that
    /// follows.
    ///
    /// The session is continued when the container is there and was made for
    /// the agent the project now names; begun afresh otherwise, replacing a
    /// container made for another agent. The opening is sent only when a
    /// session is being made.
    pub fn inspected(
        &mut self,
        container: &str,
        present: bool,
        agent: Option<Agent>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(project) = stageman_foreman::project_of(container) else {
            tracing::debug!(%container, "inspected a container that is no foreman's; ignored");
            return;
        };
        let speaker = Speaker::Foreman(project);
        if self.turns.contains_key(&speaker) {
            tracing::debug!(%project, "the foreman is already working; ignored");
            return;
        }
        let Some(errand) = waiting_on(&self.state, project) else {
            tracing::debug!(%project, "nothing is in hand for this foreman; ignored");
            return;
        };
        let (repository, handout) = match self.for_project(project, &errand.thread) {
            Ok(decided) => decided,
            Err(why) => {
                // Logged and moved past rather than retried. A message that
                // cannot be handled must not become a message that is handled
                // for ever, and the person who sent it is told.
                tracing::warn!(%project, %why, "the foreman's turn could not be decided");
                self.notice_in(project, &errand.thread, stageman_foreman::stuck_notice());
                self.move_on(project);
                return;
            }
        };

        let starting = if self.interrupted.remove(&project) {
            Starting::Interrupted
        } else {
            Starting::Fresh
        };
        let kits = kits_offered(&self.state, project);
        let kits: Vec<(&str, &str)> = kits
            .iter()
            .map(|(name, description)| (name.as_str(), description.as_str()))
            .collect();
        let asked = stageman_foreman::asked(
            stageman_foreman::Turn {
                said: &errand.said,
                starting,
            },
            &kits,
        );
        let warrant = self.warrant(speaker, Some(errand.thread.clone()));
        self.turns.insert(speaker, Turn::quiet());

        let run = if present && keeps(agent, handout.agent()) {
            // The kit goes with every turn, not only the first: a loaded
            // session forgets what it was set to.
            Run::Resume {
                container: container.to_owned(),
                kit: handout.kit().clone(),
                warrant,
                text: asked,
            }
        } else {
            if present {
                // Made for another agent, which is another image: it goes,
                // and a fresh session is begun, with the memory as the price.
                effects.emit(AppEffect::Discard {
                    container: container.to_owned(),
                });
            }
            let environment = match crate::rendered(&handout) {
                Ok(environment) => environment,
                Err(why) => {
                    tracing::warn!(%project, %why, "the foreman's environment could not be decided");
                    self.notice_in(project, &errand.thread, stageman_foreman::stuck_notice());
                    self.move_on(project);
                    return;
                }
            };
            // The opening and the first message together, because a session
            // that was told who it is and then asked nothing would have spent
            // a turn saying hello.
            Run::Begin {
                container: container.to_owned(),
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
                kickoff: format!("{}\n\n{asked}", stageman_foreman::opening(&repository)),
            }
        };
        effects.emit(AppEffect::RunTurn { speaker, run });
    }

    /// A foreman's turn ended: put the message down, pick up the next.
    pub fn foreman_ended(
        &mut self,
        project: ProjectId,
        outcome: Result<stageman_agent::Answer, String>,
    ) {
        if let Err(why) = outcome {
            tracing::warn!(%project, %why, "the foreman's turn did not finish");
            if let Some(errand) = waiting_on(&self.state, project) {
                self.notice_in(project, &errand.thread, stageman_foreman::stuck_notice());
            }
        }
        self.move_on(project);
    }

    /// Puts down the message in hand and starts on the next, if any.
    ///
    /// Whatever was queued was never begun, so it is fresh however the loop
    /// was entered.
    fn move_on(&mut self, project: ProjectId) {
        let next = self
            .state
            .projects
            .get_mut(&project)
            .is_some_and(|watched| watched.attending.finish().is_some());
        self.dirty = true;
        self.interrupted.remove(&project);
        if next {
            self.look_before_turning(project);
        }
    }

    /// What a foreman's turn is decided from: the repository it speaks
    /// about and what its agent is handed, narrowed to the thread it answers
    /// in.
    fn for_project(
        &self,
        project: ProjectId,
        thread: &Thread,
    ) -> Result<(String, Handout), stageman_core::HandoutError> {
        let repository = self
            .state
            .projects
            .get(&project)
            .map(|watched| watched.repository.clone())
            .ok_or(stageman_core::HandoutError::UnknownProject(project))?;
        let handout = Handout::for_foreman(&self.state, project)?.speaking_in(thread.clone());
        Ok((repository, handout))
    }

    /// Says something in a thread on the instance's own behalf, once whatever
    /// this step changed is on the disk.
    pub fn notice_in(&mut self, project: ProjectId, thread: &Thread, text: &str) {
        let Some(speaking) = self
            .state
            .projects
            .get(&project)
            .and_then(|watched| watched.channels.get(&thread.channel))
            .map(ChannelConfig::speaking)
        else {
            return;
        };
        self.defer(AppEffect::Say {
            speaking: speaking.into(),
            thread: thread.clone(),
            text: text.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{interrupted, keeps, kits_offered, waiting_on};
    use stageman_core::{
        Agent, AgentConfig, Attending, Channel, Errand, Kit, KitConfig, KitName, Project,
        ProjectId, Secret, State, Taken, Thread, Uuid,
    };
    use std::collections::BTreeMap;

    fn watching() -> (State, ProjectId) {
        let project = ProjectId::from_uuid(Uuid::from_u128(11));
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
                kits: BTreeMap::from([
                    (
                        KitName::new("Claude").expect("a name"),
                        KitConfig::defaults(Agent::Claude),
                    ),
                    (
                        KitName::new("Narrow").expect("a name"),
                        KitConfig {
                            description: "For small things.".to_owned(),
                            ..KitConfig::defaults(Agent::Claude)
                        },
                    ),
                ]),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                jobs: BTreeMap::new(),
                variables: BTreeMap::new(),
                attending: Attending::default(),
            },
        );
        (state, project)
    }

    fn a_message() -> Errand {
        Errand {
            said: "look at the parser".to_owned(),
            thread: Thread {
                channel: Channel::Slack,
                id: "1700000000.000100".to_owned(),
            },
        }
    }

    /// A foreman holding a message is found; an idle one is not.
    ///
    /// Both halves, because the two failures are opposite and both are
    /// silent: missing a foreman that was working leaves it wedged for ever,
    /// and naming an idle one starts a turn on a message that does not exist.
    #[test]
    fn a_foreman_holding_a_message_is_the_one_put_back_to_work() {
        let (mut state, project) = watching();
        assert_eq!(interrupted(&state), Vec::new());

        let watched = state.projects.get_mut(&project).expect("the project");
        assert_eq!(watched.attending.take(a_message()), Taken::Started);
        assert_eq!(watched.attending.take(a_message()), Taken::Waiting);

        assert_eq!(
            interrupted(&state),
            vec![project],
            "named once, however many wait"
        );
    }

    #[test]
    fn what_a_foreman_is_working_on_comes_from_its_inbox() {
        let (mut state, project) = watching();
        assert_eq!(waiting_on(&state, project), None, "idle holds nothing");

        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .attending
            .take(a_message());
        assert_eq!(waiting_on(&state, project), Some(a_message()));
        assert_eq!(
            waiting_on(&state, ProjectId::from_uuid(Uuid::from_u128(404))),
            None,
            "a project this instance does not watch has no inbox"
        );
    }

    /// The kits are named with what the operator wrote each one is for.
    #[test]
    fn the_kits_a_project_offers_are_said_with_their_descriptions() {
        let (state, project) = watching();

        let offered = kits_offered(&state, project);
        assert_eq!(offered.len(), 2);
        assert!(offered.contains(&("Narrow".to_owned(), "For small things.".to_owned())));
        assert!(kits_offered(&state, ProjectId::from_uuid(Uuid::from_u128(404))).is_empty());
    }

    /// A foreman's container is kept unless it was made for another agent.
    #[test]
    fn a_foremans_container_is_kept_unless_it_was_made_for_another_agent() {
        assert!(keeps(Some(Agent::Claude), Agent::Claude));
        assert!(
            keeps(None, Agent::Claude),
            "a container from before the label could only be this agent's"
        );
        // The one agent there is cannot be another; this is the line to
        // extend when a second exists.
        assert_eq!(Agent::ALL, &[Agent::Claude]);
    }
}
