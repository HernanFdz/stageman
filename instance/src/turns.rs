//! A turn: what the instance remembers about one while it runs, the steps
//! it takes to run it, and what it records when it ends.
//!
//! Since
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
//! a turn is not asked of the world as one thing. It is the commands the
//! runtime is given, one after another — whether the image is there, a build
//! if it is not, a container created, started, its repository checked out —
//! and then a process kept open, over which the conversation with the agent
//! runs line by line. Every step is a generic effect the instance renders,
//! and every answer is routed back to the turn by the identifier it carries,
//! so a scenario sees the argument list that actually runs and can answer
//! it.
//!
//! Every wait but the conversation is a command run once and answered by
//! its end; the conversation is lines in and lines out, and one end. A
//! person stopping a turn closes its process if it is talking, and otherwise
//! takes effect at the next step: a build cannot be interrupted, and a
//! container half-created is one nothing can name.
//!
//! **One build at a time per image, for as long as this process lives.**
//! Two turns starting together would otherwise both find their image absent
//! and build it, and the second build would move the name onto its own copy
//! and leave the first's unreferenced — which was measured to *delete* it,
//! taking the image record out from under a container created from it a
//! moment earlier. So the second waits on the first, and finds what it
//! left. This used to be a lock in the world; it is held state here.

use std::collections::BTreeMap;

use stageman_agent::{
    AgentError, Answer, Command, Conversation, Exchange, Opening, StopReason, Tools,
};
use stageman_core::{
    Agent, JobId, Kit, Platform, Progress, Project, Role, Secret, Speaking, State, Thread, Waiting,
};
use stageman_vocabulary::{Effect as Generic, EffectId, Ended, Finished};

use crate::vocabulary::{AppEffect, Speaker};
use crate::{Asked, Effect, Running, complaint};

/// Whether a turn begins a session or continues the one its container
/// holds, and everything the container is started with.
///
/// Everything an agent process is about to be handed, decided when the turn
/// is and carried as plain data: the environment its container is given is
/// rendered from the handout by the instance, so that what a container sees
/// is decided in the one place that decides. Credentials in the clear, for
/// the reason `crate::vocabulary` gives, which is why this formats not at
/// all.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub enum Run {
    /// Make the container and the session, and put the first question.
    Begin {
        /// The container to make, named before it exists.
        container: String,
        /// Which agent runs in it.
        agent: Agent,
        /// What it runs as, which decides the image.
        role: Role,
        /// Exactly the environment the container is given, and nothing
        /// inherited.
        environment: BTreeMap<String, String>,
        /// The repository checked out before the agent speaks, for a job.
        repository: Option<String>,
        /// The platform whose tool makes the checkout, if a credential for
        /// one is held.
        platform: Option<Platform>,
        /// What the agent runs on.
        kit: Kit,
        /// What the agent presents to the tools endpoint.
        warrant: String,
        /// Where that endpoint is, as a container reaches it: the port
        /// actually taken rather than the one asked for.
        tools: String,
        /// The instruction it begins from.
        kickoff: String,
    },
    /// Continue the session the container holds, settling the kit again.
    Resume {
        /// The container.
        container: String,
        /// What the job runs on, settled again because a loaded session
        /// forgets it.
        kit: Kit,
        /// What the agent presents to the tools endpoint, minted afresh.
        warrant: String,
        /// Where that endpoint is, as a container reaches it.
        tools: String,
        /// What the resumed agent is told.
        text: String,
    },
}

impl Run {
    /// The container the turn runs in.
    fn container(&self) -> &str {
        match self {
            Self::Begin { container, .. } | Self::Resume { container, .. } => container,
        }
    }

    /// The kit the agent runs on.
    const fn kit(&self) -> &Kit {
        match self {
            Self::Begin { kit, .. } | Self::Resume { kit, .. } => kit,
        }
    }

    /// What the agent is told about the tools: where they are, and what it
    /// presents to them.
    fn tools(&self) -> Tools {
        match self {
            Self::Begin { tools, warrant, .. } | Self::Resume { tools, warrant, .. } => {
                Tools::new(tools.clone(), Secret::new(warrant.clone()))
            }
        }
    }

    /// Whether the conversation makes a session or loads one.
    const fn opening(&self) -> Opening {
        match self {
            Self::Begin { .. } => Opening::Fresh,
            Self::Resume { .. } => Opening::Resumed,
        }
    }

    /// What the agent is told once the session is open.
    fn question(&self) -> &str {
        match self {
            Self::Begin { kickoff, .. } => kickoff,
            Self::Resume { text, .. } => text,
        }
    }
}

/// Where a turn has got to: which answer it is waiting on.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub enum Stage {
    /// Whether its image is there.
    Looking,
    /// A build of its image, its own or one already in flight.
    Building,
    /// Its container being created.
    Creating,
    /// Its container being started.
    Starting,
    /// Its repository being checked out.
    CheckingOut,
    /// The agent, over a process kept open.
    Talking {
        /// The process, by the identifier it was opened under.
        process: EffectId,
        /// The conversation with it. Boxed because it is most of what a
        /// turn holds, and every other stage holds nothing.
        conversation: Box<Conversation>,
    },
    /// The process's end, the conversation being over and the process told
    /// to close, with what will be recorded when it has.
    Closing {
        /// The process.
        process: EffectId,
        /// What the conversation came to.
        outcome: Result<Answer, String>,
    },
}

/// One turn in flight: what it can be told, and what it has said.
///
/// Both per turn rather than per job, which is why this is a fresh value each
/// time: a stop asked of a turn that has ended would stop the next one, and a
/// claim left over would be recorded against work the agent never described.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
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
    /// What the turn was asked to do, kept until its conversation opens: the
    /// steps before it each need a piece of it.
    run: Run,
    /// Which answer it is waiting on.
    stage: Stage,
}

impl Turn {
    /// A turn nobody is told about when it ends.
    pub const fn quiet(run: Run) -> Self {
        Self {
            stopping: false,
            claimed: None,
            notify: false,
            stage: Self::first_stage(&run),
            run,
        }
    }

    /// A turn whose ending is said on the job's thread.
    pub const fn noticed(run: Run) -> Self {
        Self {
            stopping: false,
            claimed: None,
            notify: true,
            stage: Self::first_stage(&run),
            run,
        }
    }

    /// What a turn waits on first: whether its image is there, for one that
    /// begins, and its container starting, for one that resumes.
    const fn first_stage(run: &Run) -> Stage {
        match run {
            Run::Begin { .. } => Stage::Looking,
            Run::Resume { .. } => Stage::Starting,
        }
    }

    /// The process this turn is talking to, if it is.
    const fn process(&self) -> Option<EffectId> {
        match self.stage {
            Stage::Talking { process, .. } | Stage::Closing { process, .. } => Some(process),
            _ => None,
        }
    }
}

/// The image a role's agent runs in, by name.
fn image_of(agent: Agent, role: Role) -> String {
    stageman_agent::named(&stageman_agent::recipe(agent, role))
        .as_argument()
        .to_owned()
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

/// A failure and everything underneath it, as one line of prose.
///
/// `to_string` on an error renders only its outermost line, and every error
/// that reaches a job's record wraps a more specific one — so recording the
/// outer line alone throws away the only part that says what actually went
/// wrong. One line rather than several, because this goes into a record a
/// dashboard shows as prose.
pub fn because(failure: &dyn std::error::Error) -> String {
    let mut told = failure.to_string();
    let mut cause = std::error::Error::source(failure);
    while let Some(reason) = cause {
        told.push_str(": ");
        told.push_str(&reason.to_string());
        cause = reason.source();
    }
    told
}

/// Why a container command failed, as the failure a turn records.
///
/// The runtime's own complaint, trimmed, behind a status that says which
/// command: a container that could not be created and one that could not be
/// started are different repairs.
fn container_failure(doing: &str, finished: &Finished) -> String {
    let message = complaint(finished).unwrap_or_default();
    because(&AgentError::Container {
        status: doing.to_owned(),
        message: if message.is_empty() {
            message
        } else {
            format!(" — {message}")
        },
    })
}

/// What a build that failed said, as the failure a turn records.
fn build_failure(finished: &Finished) -> String {
    let message = match finished {
        Finished::Exited { stderr, .. } => stageman_agent::last_words(stderr.as_slice()),
        Finished::NotFound | Finished::Failed(_) => {
            complaint(finished).unwrap_or_else(|| "it said nothing".to_owned())
        }
    };
    because(&AgentError::Build { message })
}

/// What a process ending mid-conversation means, as the failure a turn
/// records.
fn ended_early(conversation: &Conversation, ended: &Ended) -> String {
    because(&match ended {
        Ended::Exited { status, stderr } => {
            conversation.stopped(*status, stderr.as_text().unwrap_or(""))
        }
        Ended::NotFound => AgentError::Container {
            status: "the runtime could not be run".to_owned(),
            message: String::new(),
        },
        Ended::Failed(why) => AgentError::Container {
            status: "the agent could not be started".to_owned(),
            message: format!(" — {why}"),
        },
    })
}

impl Running {
    /// Puts a turn in flight for a speaker, and asks for its first step.
    ///
    /// The turn is held from this moment, so a stop arriving before its first
    /// answer has something to mark. What is answered with is the first
    /// command of the run — whether the image is there, or the container
    /// started — and the caller decides whether that waits for a write.
    pub fn turn(&mut self, speaker: Speaker, turn: Turn) -> Effect {
        let first = match &turn.run {
            Run::Begin { agent, role, .. } => self.ask(
                &Command::Present {
                    image: image_of(*agent, *role),
                },
                Asked::Present { speaker },
            ),
            Run::Resume { container, .. } => self.ask(
                &Command::Start {
                    name: container.clone(),
                },
                Asked::Started { speaker },
            ),
        };
        self.turns.insert(speaker, turn);
        first
    }

    /// A person asked for a speaker's turn to stop. Whether there was one.
    ///
    /// **Asking rather than killing** is what stopping means for the agent:
    /// its process is closed, which closes the pipe it speaks on and ends
    /// it, and the container carries on because the agent is no longer what
    /// it runs — see
    /// `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`. A
    /// turn that is not yet talking stops at its next step instead.
    pub fn stop_turn(&mut self, speaker: Speaker, effects: &mut Vec<Effect>) -> bool {
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return false;
        };
        turn.stopping = true;
        if let Stage::Talking { process, .. } = turn.stage {
            effects.push(Generic::Close { id: process });
        }
        true
    }

    /// Whether a turn was stopped before this step, in which case it ends
    /// here rather than going on to the next.
    fn stopped_before(&mut self, speaker: Speaker, effects: &mut Vec<Effect>) -> bool {
        let stopping = self.turns.get(&speaker).is_some_and(|turn| turn.stopping);
        if stopping {
            self.ended(speaker, Err("stopped".to_owned()), effects);
        }
        stopping
    }

    /// The runtime said whether a turn's image is there.
    ///
    /// Nothing to complain about is the whole of what "it is there" means;
    /// anything else is an image to build.
    pub fn looked(&mut self, speaker: Speaker, finished: &Finished, effects: &mut Vec<Effect>) {
        if self.stopped_before(speaker, effects) {
            return;
        }
        let Some(Run::Begin { agent, role, .. }) = self.turns.get(&speaker).map(|turn| &turn.run)
        else {
            return;
        };
        let (agent, role) = (*agent, *role);
        if complaint(finished).is_none() {
            self.make(speaker, effects);
        } else {
            self.build(speaker, agent, role, effects);
        }
    }

    /// Asks for a turn's image to be built, or waits on the build already in
    /// flight for it.
    fn build(&mut self, speaker: Speaker, agent: Agent, role: Role, effects: &mut Vec<Effect>) {
        let recipe = stageman_agent::recipe(agent, role);
        let image = stageman_agent::named(&recipe).as_argument().to_owned();
        if let Some(turn) = self.turns.get_mut(&speaker) {
            turn.stage = Stage::Building;
        }
        if let Some(waiting) = self.building.get_mut(&image) {
            tracing::info!(%image, "waiting on a build already in flight");
            waiting.push(speaker);
            return;
        }
        self.building.insert(image.clone(), vec![speaker]);
        let building = self.asking(
            &Command::Build {
                image: image.clone(),
            },
            Asked::Built { image },
            self.runtime_environment.clone(),
            Some(recipe.into()),
        );
        effects.push(building);
    }

    /// A build finished, for every turn waiting on it.
    pub fn built(&mut self, image: &str, finished: &Finished, effects: &mut Vec<Effect>) {
        // Nobody waiting is the true reading of a build nothing asked for.
        let waiting = self.building.remove(image).unwrap_or_default();
        let failed = complaint(finished).map(|_| build_failure(finished));
        for speaker in waiting {
            match &failed {
                None => self.make(speaker, effects),
                Some(why) => {
                    tracing::warn!(%image, %why, "the image could not be built");
                    self.ended(speaker, Err(why.clone()), effects);
                }
            }
        }
    }

    /// Asks for a turn's container to be made, with exactly the environment
    /// its handout decided.
    fn make(&mut self, speaker: Speaker, effects: &mut Vec<Effect>) {
        if self.stopped_before(speaker, effects) {
            return;
        }
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        let Run::Begin {
            container,
            agent,
            role,
            environment,
            ..
        } = &turn.run
        else {
            return;
        };
        turn.stage = Stage::Creating;
        let command = Command::Create {
            name: container.clone(),
            image: image_of(*agent, *role),
            agent: *agent,
            instance: self.id,
            variables: environment.keys().cloned().collect(),
        };
        // The runtime is given the credentials in its environment, and
        // told by name which to forward — so a secret travels through an
        // environment rather than a command line, and never appears in the
        // process table.
        let mut given = self.runtime_environment.clone();
        given.extend(environment.clone());
        let creating = self.asking(&command, Asked::Created { speaker }, given, None);
        effects.push(creating);
    }

    /// The runtime said whether a turn's container was made.
    pub fn made(&mut self, speaker: Speaker, finished: &Finished, effects: &mut Vec<Effect>) {
        if self.stopped_before(speaker, effects) {
            return;
        }
        if complaint(finished).is_some() {
            let why = container_failure("creating the container", finished);
            self.ended(speaker, Err(why), effects);
            return;
        }
        self.hold(speaker, effects);
    }

    /// Asks for a turn's container to be started, or left running if it is.
    fn hold(&mut self, speaker: Speaker, effects: &mut Vec<Effect>) {
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        turn.stage = Stage::Starting;
        let name = turn.run.container().to_owned();
        let starting = self.ask(&Command::Start { name }, Asked::Started { speaker });
        effects.push(starting);
    }

    /// The runtime said whether a turn's container is up.
    ///
    /// A job's first turn fills the workspace before the agent is run, and
    /// inside the container it will run in: a coding agent reads a
    /// project's instructions when its session starts, so a checkout made
    /// during the first turn is one the agent's own machinery never loads —
    /// see
    /// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    pub fn held(&mut self, speaker: Speaker, finished: &Finished, effects: &mut Vec<Effect>) {
        if self.stopped_before(speaker, effects) {
            return;
        }
        if complaint(finished).is_some() {
            let why = container_failure("starting the container", finished);
            self.ended(speaker, Err(why), effects);
            return;
        }
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        let Run::Begin {
            container,
            repository: Some(repository),
            platform,
            ..
        } = &turn.run
        else {
            self.open(speaker, effects);
            return;
        };
        turn.stage = Stage::CheckingOut;
        let command = Command::Checkout {
            name: container.clone(),
            repository: repository.clone(),
            platform: *platform,
        };
        let checking_out = self.ask(&command, Asked::CheckedOut { speaker });
        effects.push(checking_out);
    }

    /// The runtime said whether a turn's repository was checked out.
    pub fn checked_out(
        &mut self,
        speaker: Speaker,
        finished: &Finished,
        effects: &mut Vec<Effect>,
    ) {
        if self.stopped_before(speaker, effects) {
            return;
        }
        if let Some(message) = complaint(finished) {
            let repository = match self.turns.get(&speaker).map(|turn| &turn.run) {
                Some(Run::Begin {
                    repository: Some(repository),
                    ..
                }) => repository.clone(),
                _ => String::new(),
            };
            let why = because(&AgentError::Checkout {
                repository,
                message,
            });
            self.ended(speaker, Err(why), effects);
            return;
        }
        self.open(speaker, effects);
    }

    /// Runs the agent inside a turn's container and opens the conversation
    /// with it: the process, and the first line it is sent, in one step.
    fn open(&mut self, speaker: Speaker, effects: &mut Vec<Effect>) {
        let process = self.effect_id();
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        let run = &turn.run;
        let (conversation, lines) = Conversation::begin(
            run.opening(),
            Some(&run.tools()),
            run.kit().clone(),
            run.question(),
        );
        effects.push(Generic::Open {
            id: process,
            program: self.runtime.clone(),
            arguments: Command::Exec {
                name: run.container().to_owned(),
            }
            .arguments(),
            environment: self.runtime_environment.clone(),
        });
        for line in lines {
            effects.push(Generic::Send { id: process, line });
        }
        turn.stage = Stage::Talking {
            process,
            conversation: Box::new(conversation),
        };
        self.talking.insert(process, speaker);
    }

    /// A line from a turn's agent.
    ///
    /// What the conversation says back goes out at once, and the conversation
    /// being over closes the process: its end is what ends the turn, so that
    /// there is one place a turn ends.
    pub fn line(&mut self, process: EffectId, line: &str, effects: &mut Vec<Effect>) {
        let Some(speaker) = self.talking.get(&process).copied() else {
            tracing::warn!(
                "a line arrived from a process this instance is not talking to; ignored"
            );
            return;
        };
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        let Stage::Talking { conversation, .. } = &mut turn.stage else {
            // Closing: whatever the agent says after the conversation is
            // over is its own business.
            return;
        };
        match conversation.heard(line) {
            Exchange::Continue(lines) => {
                for line in lines {
                    effects.push(Generic::Send { id: process, line });
                }
            }
            Exchange::Over(outcome) => {
                let outcome = outcome.map_err(|why| because(&why));
                turn.stage = Stage::Closing { process, outcome };
                effects.push(Generic::Close { id: process });
            }
        }
    }

    /// A turn's agent process ended, on its own or because it was closed.
    ///
    /// The process is forgotten by the turn ending, which is the one place a
    /// turn's process is forgotten; a process whose turn is already gone is
    /// forgotten here, so that nothing is held for nobody.
    pub fn process_ended(&mut self, process: EffectId, ended: &Ended, effects: &mut Vec<Effect>) {
        let Some(speaker) = self.talking.get(&process).copied() else {
            tracing::warn!("a process this instance did not open ended; ignored");
            return;
        };
        let Some(turn) = self.turns.get(&speaker) else {
            self.talking.remove(&process);
            return;
        };
        let outcome = match &turn.stage {
            Stage::Closing { outcome, .. } => outcome.clone(),
            Stage::Talking { conversation, .. } => Err(ended_early(conversation, ended)),
            _ => Err("its process ended before it was spoken to".to_owned()),
        };
        self.ended(speaker, outcome, effects);
    }

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
        if let Some(process) = turn.process() {
            self.talking.remove(&process);
        }
        self.warrants.retain(|_, known| known.speaker != speaker);
        let job = match speaker {
            Speaker::Foreman(project) => {
                self.foreman_ended(project, outcome);
                return;
            }
            Speaker::Job(job) => job,
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
        said_about(job, &progress);
        self.record(job, progress);

        // The container is asked whether it is still showing something, at
        // one of the three moments 0043 names. Inward-facing, so it need not
        // wait for the record to land.
        self.probe(job, effects);

        // Said whichever way it went: the agent has already reported for
        // itself if it could, and this says the one thing the agent cannot,
        // which is that it has stopped and a reply now reaches it. Outward,
        // so it waits for the record.
        if turn.notify
            && let Some((speaking, thread)) = speaking_for(&self.state, job)
        {
            self.defer(AppEffect::Say {
                speaking: speaking.into(),
                thread,
                text: stageman_foreman::attention_notice().to_owned(),
            });
        }
    }

    /// What becomes of a turn whose first step was dropped with the write it
    /// waited on: it never started, and the job it was for is recorded as
    /// failed for that reason.
    ///
    /// The alternative is a job that says working with nothing running in
    /// it, which a reply could never reach. The record is a change of its
    /// own, so the next write carries it — and if that one lands, the job
    /// can be given something again.
    pub fn abandoned(&mut self, speaker: Speaker) {
        self.turns.remove(&speaker);
        self.warrants.retain(|_, known| known.speaker != speaker);
        if let Speaker::Job(job) = speaker {
            self.record(
                job,
                Progress::Idle(Waiting::Failed(
                    "the instance could not be written, so the turn was not started".to_owned(),
                )),
            );
        }
    }
}

/// Says what became of a turn, where it is worth saying.
///
/// Skipped by mutation testing: it chooses a diagnostic and decides nothing,
/// and a test of which level a line is logged at would be a test of the
/// logging.
#[mutants::skip]
fn said_about(job: JobId, progress: &Progress) {
    match progress {
        Progress::Idle(Waiting::Failed(why)) => {
            tracing::warn!(%job, %why, "the turn did not finish");
        }
        Progress::Idle(Waiting::Paused) => tracing::info!(%job, "a person stopped it"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Run, Turn, because, build_failure, container_failure, image_of, outcome, speaking_for,
    };
    use stageman_agent::{Answer, StopReason};
    use stageman_core::{
        Agent, AgentConfig, Channel, ChannelConfig, Job, JobId, Kit, KitConfig, KitName, Progress,
        Project, ProjectId, Role, Secret, State, Thread, Timestamp, Uuid, Waiting,
    };
    use stageman_vocabulary::{Bytes, Finished};
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

    /// The chain is what there is to read.
    #[test]
    fn a_failure_is_recorded_with_everything_underneath_it() {
        let inner = std::io::Error::other("the disk is full");
        let outer = stageman_agent::AgentError::Runtime {
            path: std::path::PathBuf::from("/usr/local/bin/docker"),
            source: inner,
        };
        let told = because(&outer);
        assert!(told.contains("the disk is full"), "{told}");
        assert!(told.contains(": "), "{told}");
    }

    /// What a failed command says reaches the record, behind which command
    /// it was; a build says its last words rather than its first.
    #[test]
    fn a_failed_command_is_recorded_by_what_it_said_and_which_it_was() {
        let refused = Finished::Exited {
            status: Some(1),
            stdout: Bytes::new(Vec::new()),
            stderr: Bytes::new(b"Error: No such container: x\n".to_vec()),
        };
        let told = container_failure("starting the container", &refused);
        assert!(told.contains("starting the container"), "{told}");
        assert!(told.contains("No such container: x"), "{told}");

        let silent = Finished::Exited {
            status: Some(1),
            stdout: Bytes::new(Vec::new()),
            stderr: Bytes::new(Vec::new()),
        };
        assert!(
            !container_failure("creating the container", &silent).contains(" — "),
            "nothing said, nothing quoted"
        );

        let build = Finished::Exited {
            status: Some(1),
            stdout: Bytes::new(Vec::new()),
            stderr: Bytes::new(b"step 1\nstep 2\nfailed to fetch\n".to_vec()),
        };
        let told = build_failure(&build);
        assert!(told.contains("failed to fetch"), "{told}");
        assert!(told.contains("could not be built"), "{told}");
        assert!(
            build_failure(&Finished::NotFound).contains("could not be run"),
            "{}",
            build_failure(&Finished::NotFound)
        );
    }

    /// A turn begins by looking for its image and resumes by starting its
    /// container, and the image is the one its role's recipe hashes to.
    #[test]
    fn a_turn_waits_first_on_what_its_run_needs_first() {
        let begin = Turn::quiet(Run::Begin {
            container: "stageman-foreman-x".to_owned(),
            agent: Agent::Claude,
            role: Role::Foreman,
            environment: BTreeMap::new(),
            repository: None,
            platform: None,
            kit: Kit::defaults(Agent::Claude),
            warrant: "w".to_owned(),
            tools: "http://host.docker.internal:47113/mcp".to_owned(),
            kickoff: "hello".to_owned(),
        });
        assert!(matches!(begin.stage, super::Stage::Looking));
        assert!(!begin.notify);
        let resume = Turn::noticed(Run::Resume {
            container: "stageman-job-x".to_owned(),
            kit: Kit::defaults(Agent::Claude),
            warrant: "w".to_owned(),
            tools: "http://host.docker.internal:47113/mcp".to_owned(),
            text: "carry on".to_owned(),
        });
        assert!(matches!(resume.stage, super::Stage::Starting));
        assert!(resume.notify);
        assert_eq!(begin.process(), None);

        assert_ne!(
            image_of(Agent::Claude, Role::Foreman),
            image_of(Agent::Claude, Role::Job),
            "a foreman's image is not a job's"
        );
        assert!(image_of(Agent::Claude, Role::Job).starts_with("stageman:"));
    }

    /// A turn holds credentials, so it formats not at all.
    #[test]
    fn a_turn_formats_not_at_all() {
        struct Probe<T>(std::marker::PhantomData<T>);
        impl<T: std::fmt::Debug> Probe<T> {
            #[expect(
                clippy::unused_self,
                reason = "a method, so that resolution prefers it to the trait's where it exists"
            )]
            const fn formats(&self) -> bool {
                true
            }
        }
        trait Otherwise {
            fn formats(&self) -> bool {
                false
            }
        }
        impl<T> Otherwise for Probe<T> {}

        assert!(!Probe::<Turn>(std::marker::PhantomData).formats());
        assert!(!Probe::<Run>(std::marker::PhantomData).formats());
        assert!(
            Probe::<String>(std::marker::PhantomData).formats(),
            "the probe tells"
        );
    }
}
