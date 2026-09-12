//! The deciding and the doing as one deterministic value.
//!
//! An [`Instance`] holds everything one running stageman knows and makes every
//! decision it makes. It is constructed from a seed and the environment, and
//! learns everything else — whether a runtime answers, its key, its own file,
//! what the runtime holds — by asking the world and hearing back. After that
//! it has one method, which takes one [`Event`] and answers with [`Effect`]s.
//! It never reads a clock, never draws on entropy beyond the generator it was
//! seeded with, and performs nothing: the world does, and reports back. See
//! `docs/decisions/0056-the-instance-decides-and-the-world-performs.md` and
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! **What it keeps and what it holds are two things.** The kept state goes to
//! the disk, sealed by this crate. The held state — turns in flight, warrants
//! minted, effects waiting on a write — is what only this process knows, and a
//! restart begins with none of it.
//!
//! **Persisting is answered, and everything outward-facing waits for it.** A
//! step that changes the kept state ends by asking the world to write, and any
//! effect of that step that faces outward is held back until the world says
//! the bytes have landed. So a client told a change succeeded was told the
//! truth, and a job's record is on the disk before its container exists. The
//! first write is the one a start depends on: it is what says the file can be
//! written at all, and the address is announced only once it has landed.

mod boot;
mod file;
mod foreman;
mod jobs;
mod paths;
pub mod release;
mod replies;
mod requests;
mod snapshot;
mod sweep;
mod tools;
mod tunnel;
mod turns;
mod views;
mod vocabulary;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use rand::Rng as _;
use rand::rngs::StdRng;
use stageman_core::{InstanceId, JobId, Key, Kit, Progress, ProjectId, State, Thread, Uuid};
use stageman_vocabulary::{Effect as Generic, EffectId, Environment};

pub use boot::KeySource;
pub use file::LoadError;
pub use paths::{DOMAIN_VARIABLE, KEY_VARIABLE, STATE_VARIABLE};
pub use requests::{Request, Response};
pub use stageman_vocabulary::Seed;
pub use sweep::Swept;
pub use tunnel::{DEFAULT_DOMAIN, Domain, Routed, address, decode};
pub use vocabulary::{
    AppEffect, AppEvent, Container, Message, Posting, RequestId, Run, Speaker, Warranted,
};

/// This application, as the vocabulary sees it: what fills its hole.
pub struct Stageman;

impl stageman_vocabulary::App for Stageman {
    type Event = AppEvent;
    type Effect = AppEffect;
}

/// One thing the world tells this instance.
pub type Event = stageman_vocabulary::Event<Stageman>;

/// One thing this instance asks of the world.
pub type Effect = stageman_vocabulary::Effect<Stageman>;

impl From<AppEvent> for Event {
    fn from(event: AppEvent) -> Self {
        Self::App(event)
    }
}

impl From<AppEffect> for Effect {
    fn from(effect: AppEffect) -> Self {
        Self::App(effect)
    }
}

/// Pushing an effect of this application's onto a step's effects, without
/// wrapping it at every site.
trait Emit {
    /// Adds an effect.
    fn emit(&mut self, effect: impl Into<Effect>);
}

impl Emit for Vec<Effect> {
    fn emit(&mut self, effect: impl Into<Effect>) {
        self.push(effect.into());
    }
}

/// Exactly the environment a container is given, rendered from what its
/// handout decides, credentials in the clear for the wire.
///
/// # Errors
///
/// Fails if a project's variable claims a name this project delivers itself.
fn rendered(
    handout: &stageman_core::Handout,
) -> Result<BTreeMap<String, String>, stageman_agent::AgentError> {
    Ok(stageman_agent::environment(handout)?
        .into_iter()
        .map(|(name, value)| (name, value.expose().to_owned()))
        .collect())
}

use turns::Turn;

/// Says so when a write was answered out of order.
///
/// Skipped by mutation testing because it is equivalent under one: what the
/// comparison decides is which line is logged, and a log line is not
/// something a test can see. What keeps the order is the queue, and that is
/// tested.
#[mutants::skip]
fn out_of_order(asked: EffectId, id: EffectId) {
    if asked != id {
        tracing::error!("writes were answered out of order; carrying on in the order asked");
    }
}

/// How often the instance asks which containers still deserve to be up.
///
/// A server an agent left running does not say when it stops, so the only
/// way to notice is to look — see
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`.
const SETTLING_INTERVAL: Duration = Duration::from_mins(1);

/// The instance: booting, and then awake.
///
/// Two stages rather than one struct of optional facts, so that everything
/// an awake instance needs is a plain field of [`Running`] and nothing has
/// to be checked for absence at every use.
pub struct Instance {
    stage: Stage,
}

enum Stage {
    Booting(Box<boot::Boot>),
    Awake(Box<Running>),
}

impl Instance {
    /// Constructs an instance from the two facts that exist before anything
    /// happens, and answers with what it asks for first.
    #[must_use]
    pub fn boot(seed: Seed, environment: Environment) -> (Self, Vec<Effect>) {
        let (booting, effects) = boot::Boot::new(seed, environment);
        (
            Self {
                stage: Stage::Booting(Box::new(booting)),
            },
            effects,
        )
    }

    /// Handles one event and answers with what to do about it.
    ///
    /// While booting, an answer moves booting along and anything of the
    /// application's own waits; the moment the instance is awake, whatever
    /// waited is handled in the order it arrived.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        match &mut self.stage {
            Stage::Awake(running) => running.step(event),
            Stage::Booting(booting) => match booting.step(event) {
                boot::Booting::Asking(effects) => effects,
                boot::Booting::Awake(mut running, mut effects, waiting) => {
                    for event in waiting {
                        effects.extend(running.step(Event::App(event)));
                    }
                    self.stage = Stage::Awake(running);
                    effects
                }
            },
        }
    }

    /// What this instance knows: nothing until it is awake.
    #[must_use]
    pub fn state(&self) -> &State {
        match &self.stage {
            Stage::Booting(booting) => booting.state(),
            Stage::Awake(running) => &running.state,
        }
    }

    /// Which instance this is, once it knows.
    #[must_use]
    pub fn id(&self) -> Option<InstanceId> {
        match &self.stage {
            Stage::Booting(_) => None,
            Stage::Awake(running) => Some(running.id),
        }
    }

    /// Who holds a credential presented to the tools endpoint, if anyone.
    #[must_use]
    pub fn warranted(&self, presented: &str) -> Option<&Warranted> {
        match &self.stage {
            Stage::Booting(_) => None,
            Stage::Awake(running) => running.warrants.get(presented),
        }
    }

    /// What waking found, once it has.
    #[must_use]
    pub fn swept(&self) -> Option<&Swept> {
        match &self.stage {
            Stage::Booting(_) => None,
            Stage::Awake(running) => running.swept.as_ref(),
        }
    }

    /// Everything this instance holds, as a value.
    #[must_use]
    pub fn snapshot(&self) -> serde_json::Value {
        match &self.stage {
            Stage::Booting(booting) => serde_json::json!({ "booting": booting.phase_name() }),
            Stage::Awake(running) => snapshot::of(running),
        }
    }
}

impl stageman_vocabulary::Deciding for Instance {
    type App = Stageman;

    fn boot(seed: Seed, environment: Environment) -> (Self, Vec<Effect>) {
        Self::boot(seed, environment)
    }

    fn step(&mut self, event: Event) -> Vec<Effect> {
        Self::step(self, event)
    }

    fn snapshot(&self) -> serde_json::Value {
        Self::snapshot(self)
    }
}

/// What booting hands an awake instance.
pub struct Facts {
    /// What the file held, or nothing on a first run.
    pub state: State,
    /// What the file named itself, if it did.
    pub named: Option<InstanceId>,
    /// What opens the file.
    pub key: Key,
    /// Where the key came from.
    pub source: KeySource,
    /// The one generator, carried on.
    pub rng: StdRng,
    /// The next effect identifier, carried on.
    pub next: u64,
    /// The domain this instance answers on.
    pub domain: Domain,
    /// Where the file is.
    pub path: PathBuf,
    /// The runtime that answered.
    pub runtime: PathBuf,
    /// Where the dashboard is served, as a person would type it.
    pub address: String,
    /// The port it is served on.
    pub port: u16,
}

/// An awake instance: everything it knows, and everything it holds.
pub struct Running {
    /// What is kept.
    state: State,
    /// Which instance this is.
    id: InstanceId,
    /// What seals the file.
    key: Key,
    /// Where the key came from, for the startup block.
    source: KeySource,
    /// Where the file is.
    path: PathBuf,
    /// The domain this instance answers on.
    domain: Domain,
    /// The port the dashboard is served on.
    serving: u16,
    /// The address the dashboard is served on, for the startup block.
    address: String,
    /// The runtime that answered.
    runtime: PathBuf,
    /// The one generator.
    rng: StdRng,
    /// The next effect identifier.
    next: u64,
    /// The turns in flight, by whose they are.
    turns: BTreeMap<Speaker, Turn>,
    /// The credentials minted for those turns.
    warrants: BTreeMap<String, Warranted>,
    /// The foremen found mid-turn on waking, until each is picked up.
    interrupted: BTreeSet<ProjectId>,
    /// Requests on the tools endpoint waiting on a post.
    asking: BTreeMap<RequestId, Option<serde_json::Value>>,
    /// Where each job's tunnel was last found.
    tunnels: BTreeMap<JobId, u16>,
    /// Tunnel requests waiting for the runtime to say where a job is.
    routing: BTreeMap<JobId, Vec<RequestId>>,
    /// Effects waiting on a write, by the write they wait on, in order.
    deferred: VecDeque<(EffectId, Vec<Effect>)>,
    /// The wakes asked for that have not gone off, each one the settling
    /// timer; a second kind of timer is what would make this a map again.
    timers: BTreeSet<EffectId>,
    /// Whether this step changed what is kept.
    dirty: bool,
    /// Effects of this step held back until the write lands.
    staged: Vec<Effect>,
    /// Whether the startup block has been printed, which happens once the
    /// first write has landed.
    announced: bool,
    /// What waking found.
    swept: Option<Swept>,
}

impl Running {
    /// An awake instance, from what booting learned.
    ///
    /// A file that had no identity has one from the moment it is opened.
    pub(crate) fn woken(facts: Facts) -> Self {
        let Facts {
            state,
            named,
            key,
            source,
            mut rng,
            next,
            domain,
            path,
            runtime,
            address,
            port,
        } = facts;
        let id = named.unwrap_or_else(|| {
            let minted = InstanceId::from_uuid(mint(&mut rng));
            tracing::info!(instance = %minted, "this instance had no identity, so it was given one");
            minted
        });
        Self {
            state,
            id,
            key,
            source,
            path,
            domain,
            serving: port,
            address,
            runtime,
            rng,
            next,
            turns: BTreeMap::new(),
            warrants: BTreeMap::new(),
            interrupted: BTreeSet::new(),
            asking: BTreeMap::new(),
            tunnels: BTreeMap::new(),
            routing: BTreeMap::new(),
            deferred: VecDeque::new(),
            timers: BTreeSet::new(),
            // Written once on waking, before anything can depend on this
            // instance: a first run has a file at all, a file that had no
            // identity has one from the moment it is opened, and a path that
            // cannot be written fails at startup rather than at the first
            // change — `docs/conventions.md` §3.
            dirty: true,
            staged: Vec::new(),
            announced: false,
            swept: None,
        }
    }

    /// What waking asks for: the world told what booting found, the sweep,
    /// and the first write.
    pub(crate) fn waking_up(&mut self, containers: &[Container]) -> Vec<Effect> {
        let mut effects = vec![Effect::App(AppEffect::Booted {
            runtime: self.runtime.clone(),
            domain: self.domain.to_string(),
        })];
        let (swept, tally) = self.waking(containers);
        effects.extend(swept);
        self.swept = Some(tally);
        self.flush(&mut effects);
        effects
    }

    /// An identifier for an effect this instance will be answered about.
    const fn effect_id(&mut self) -> EffectId {
        let id = EffectId(self.next);
        // An identifier only has to be unique among the effects still
        // waiting to be answered — the writes not yet landed and the timers
        // not yet gone off — so coming round could only meet one long spent.
        self.next = self.next.wrapping_add(1); // CLAMP-OK: the cycle is `EffectId`'s contract.
        id
    }

    /// Handles one event and answers with what to do about it.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            Event::Written { id, outcome } => self.written(id, outcome, &mut effects),
            Event::Woke { id } => self.woke(id, &mut effects),
            Event::Read { .. } | Event::Ran { .. } => {
                tracing::warn!(
                    "answered something this instance did not ask for while awake; ignored"
                );
            }
            Event::App(event) => self.told(event, &mut effects),
        }
        self.flush(&mut effects);
        debug_assert!(
            self.state.check().is_ok(),
            "a step left the state inconsistent"
        );
        effects
    }

    /// Handles one thing the application's own world said.
    fn told(&mut self, event: AppEvent, effects: &mut Vec<Effect>) {
        match event {
            AppEvent::Serving { .. } => {
                tracing::debug!("told again where the dashboard is served; ignored");
            }
            AppEvent::TurnEnded { speaker, outcome } => self.ended(speaker, outcome, effects),
            AppEvent::Probed { job, answering } => {
                if !answering {
                    // Halted, so the port it was on reaches nothing.
                    self.forget_tunnel(job);
                }
                probed(job, answering, effects);
            }
            AppEvent::Listed { running } => self.listed(&running, effects),
            AppEvent::Heard { channel, message } => self.heard(channel, &message, effects),
            AppEvent::Inspected {
                container,
                present,
                agent,
            } => self.inspected(&container, present, agent, effects),
            AppEvent::ToolCalled {
                id,
                at,
                nearby,
                bearer,
                body,
            } => self.tool_called(id, at, nearby, bearer.as_deref(), &body, effects),
            AppEvent::ThreadOpened { job, outcome } => self.thread_opened(job, outcome),
            AppEvent::Posted { request, outcome } => self.posted(request, outcome),
            AppEvent::Request { id, request } => self.requested(id, request, effects),
            AppEvent::TunnelAsked { id, job } => self.tunnel_asked(id, job, effects),
            AppEvent::PortFound { job, port } => self.port_found(job, port, effects),
            AppEvent::TunnelFailed { job, why } => self.tunnel_failed(job, &why),
        }
    }

    /// A timer went off.
    fn woke(&mut self, id: EffectId, effects: &mut Vec<Effect>) {
        if self.timers.remove(&id) {
            effects.emit(AppEffect::ListRunning);
            let settling = self.settle_later();
            effects.push(settling);
        } else {
            tracing::warn!("woken for a timer this instance did not set; ignored");
        }
    }

    /// What the world said about a write.
    ///
    /// Writes are answered in the order they were asked for, so the front of
    /// the queue is the one being answered. A write that landed releases what
    /// waited on it; the first to land is what the address is announced
    /// after, because it is what says the file can be written at all. A write
    /// that failed drops what waited on it: the state in memory is still
    /// right, the next change asks again, and nothing outward-facing happens
    /// on the strength of a record that is not on the disk — except at the
    /// start, where a file that cannot be written is a start that refuses.
    fn written(&mut self, id: EffectId, outcome: Result<(), String>, effects: &mut Vec<Effect>) {
        let Some((asked, waiting)) = self.deferred.pop_front() else {
            tracing::warn!("the world answered a write nobody asked for; ignored");
            return;
        };
        out_of_order(asked, id);
        match outcome {
            Ok(()) => {
                effects.extend(waiting);
                if !self.announced {
                    self.announced = true;
                    effects.push(Generic::Print {
                        text: self.announcement(),
                    });
                }
            }
            Err(why) => {
                tracing::error!(%why, "the instance could not be written");
                if self.announced {
                    self.dropped(waiting);
                } else {
                    effects.push(Generic::Exit {
                        message: format!(
                            "the instance at {} could not be written\n  caused by: {why}",
                            self.path.display()
                        ),
                    });
                }
            }
        }
    }

    /// The startup block: every field on its own line, the address last.
    ///
    /// Outward from the program to this instance's data: what it is, what it
    /// needs in order to work, what opens its file, where that file is, and
    /// the domain — the one most likely to be wrong on a machine this was
    /// deployed to, so it is said out loud. The address is last, and that
    /// ordering is load-bearing: it is what anything supervising a start
    /// waits for, so everything worth reading has to be above it.
    fn announcement(&self) -> String {
        format!(
            "\nstageman is running.\n  version    {}\n  runtime    {}\n  key        {}\n  \
             instance   {}\n  domain     {}\n\n  dashboard  http://{}\n\n",
            release::described(),
            self.runtime.display(),
            self.source,
            self.path.display(),
            self.domain,
            self.address,
        )
    }

    /// What becomes of effects that waited on a write that never landed.
    ///
    /// Notices are simply not said. A turn that was to start is not started,
    /// and the job it was for is recorded as failed for that reason: the
    /// alternative is a job that says working with nothing running in it,
    /// which a reply could never reach. The record is a change of its own,
    /// so the next write carries it — and if that one lands, the job can be
    /// given something again.
    fn dropped(&mut self, effects: Vec<Effect>) {
        tracing::warn!(
            dropped = effects.len(),
            "what waited on the write is dropped"
        );
        for effect in effects {
            if let Effect::App(AppEffect::RunTurn {
                speaker: speaker @ Speaker::Job(job),
                ..
            }) = effect
            {
                self.turns.remove(&speaker);
                self.warrants.retain(|_, known| known.speaker != speaker);
                self.record(
                    job,
                    Progress::Idle(stageman_core::Waiting::Failed(
                        "the instance could not be written, so the turn was not started".to_owned(),
                    )),
                );
            }
        }
    }

    /// The step's last act.
    ///
    /// A changed state is sealed and asked to be written, and whatever the
    /// step held back waits on that write; an unchanged state releases the
    /// held-back effects at once, because there is nothing to wait for.
    fn flush(&mut self, effects: &mut Vec<Effect>) {
        let staged = std::mem::take(&mut self.staged);
        if !self.dirty {
            effects.extend(staged);
            return;
        }
        self.dirty = false;
        match file::sealed(&self.state, self.id, &self.key, &mut self.rng) {
            Ok(bytes) => {
                let id = self.effect_id();
                self.deferred.push_back((id, staged));
                effects.push(Generic::Write {
                    id,
                    path: self.path.clone(),
                    bytes: bytes.into(),
                    private: false,
                });
            }
            Err(why) => {
                tracing::error!(%why, "the instance could not be sealed, so nothing is written");
                self.dropped(staged);
            }
        }
    }

    /// Holds an effect back until the state this step changed is on the disk.
    fn defer(&mut self, effect: impl Into<Effect>) {
        self.staged.push(effect.into());
    }

    /// Writes what became of a job.
    fn record(&mut self, job: JobId, progress: Progress) {
        if let Some(recorded) = self.state.job_mut(job) {
            recorded.progress = progress;
            self.dirty = true;
        }
    }

    /// Writes down what a job's session reported it was set to, this turn.
    fn noted(&mut self, job: JobId, reported: BTreeMap<String, String>) {
        if let Some(recorded) = self.state.job_mut(job) {
            recorded.reported = reported;
            self.dirty = true;
        }
    }

    /// What a job resuming needs from its record: where it speaks, and what
    /// it runs on.
    fn recorded(&self, job: JobId) -> Option<(Option<Thread>, Kit)> {
        self.state
            .job(job)
            .map(|recorded| (recorded.thread.clone(), recorded.kit().clone()))
    }

    /// Mints the credential a turn presents to the tools endpoint, forgetting
    /// whatever that speaker held before.
    ///
    /// Unguessable from the instance's own generator, so there is one answer
    /// to where an unguessable value comes from. Bounded by construction: one
    /// entry per turn in flight rather than one per turn ever taken.
    fn warrant(&mut self, speaker: Speaker, thread: Option<Thread>) -> String {
        let credential = format!(
            "{}{}",
            mint(&mut self.rng).simple(),
            mint(&mut self.rng).simple()
        );
        self.warrants.retain(|_, known| known.speaker != speaker);
        self.warrants
            .insert(credential.clone(), Warranted { speaker, thread });
        credential
    }
}

fn mint(rng: &mut StdRng) -> Uuid {
    let mut bytes = [0_u8; 16];
    rng.fill_bytes(&mut bytes);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// What to do about a job's tunnel, now the world has looked.
///
/// Answering means the container is left running for whoever is looking;
/// nothing behind it means the container is stopped, keeping it and the
/// session in it for the next reply.
fn probed(job: JobId, answering: bool, effects: &mut Vec<Effect>) {
    if answering {
        tracing::info!(
            %job,
            "its container is left running, because something is still answering on its tunnel"
        );
    } else {
        effects.emit(AppEffect::Halt {
            container: stageman_job::container(job),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{AppEffect, Effect, probed};
    use stageman_core::{JobId, Uuid};

    /// The whole of what deciding a container's life looks like from here:
    /// answering keeps it, silence stops it.
    #[test]
    fn a_tunnel_that_answers_keeps_its_container_and_silence_stops_it() {
        let job = JobId::from_uuid(Uuid::from_u128(5));

        let mut effects = Vec::new();
        probed(job, true, &mut effects);
        assert!(effects.is_empty(), "answering keeps the container");

        probed(job, false, &mut effects);
        assert!(
            matches!(effects.as_slice(), [Effect::App(AppEffect::Halt { container })] if *container == stageman_job::container(job)),
            "silence stops it, and nothing else happens"
        );
    }
}
