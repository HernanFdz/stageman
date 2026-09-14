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
mod channel;
mod file;
mod foreman;
mod jobs;
mod listening;
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
use stageman_agent::Command;
use stageman_core::{
    Agent, InstanceId, JobId, Key, Kit, Progress, ProjectId, State, Thread, Timestamp, Uuid,
};
use stageman_vocabulary::{Effect as Generic, EffectId, Environment, Finished};

pub use boot::KeySource;
pub use file::LoadError;
pub use paths::{DOMAIN_VARIABLE, KEY_VARIABLE, STATE_VARIABLE};
pub use requests::{Request, Response};
/// Which platform this build was made for, handed to [`Instance::boot`].
///
/// The agent crate's, because that is where what a platform means is known:
/// where a container runtime might be on one. Re-exported because the entry
/// point names it and has no other reason to know that crate exists.
pub use stageman_agent::Target;
pub use stageman_vocabulary::Seed;
pub use sweep::Swept;
pub use tunnel::{ANSWERING_WITHIN, DEFAULT_DOMAIN, Domain, Routed, address, answering, decode};
pub use vocabulary::{AppEffect, AppEvent, Container, RequestId, Speaker, Warranted};

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

/// What a wake was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Timer {
    /// The settling sweep: which containers still deserve to be up.
    Settling,
    /// A project's channel, tried again after something went wrong.
    Reconnecting {
        /// Whose.
        project: ProjectId,
    },
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
    pub fn boot(seed: Seed, environment: Environment, target: Target) -> (Self, Vec<Effect>) {
        let (booting, effects) = boot::Boot::new(seed, environment, target);
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
    type Target = Target;

    fn boot(seed: Seed, environment: Environment, target: Target) -> (Self, Vec<Effect>) {
        Self::boot(seed, environment, target)
    }

    fn step(&mut self, event: Event) -> Vec<Effect> {
        Self::step(self, event)
    }

    fn snapshot(&self) -> serde_json::Value {
        Self::snapshot(self)
    }
}

/// What a tool call said about itself, while its body is being read.
///
/// Held between the head arriving and the body landing, which is the one
/// place a request is in two halves: what may be decided from the head is
/// decided when the body is there, so that a refusal for a credential
/// nobody holds and one for a body that is not a call read the same.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Called {
    /// Whether it came from somewhere allowed to ask.
    nearby: bool,
    /// What it presented, if it presented anything.
    bearer: Option<String>,
    /// When it arrived.
    at: Timestamp,
}

/// What the runtime was asked, and therefore what its answer means.
///
/// The awake half of what booting's phases do: a command is a value with an
/// identifier, and the identifier comes back on the answer, so this is what
/// says which question was being answered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Asked {
    /// A container told to stop, which is a job's tunnel gone quiet.
    Halted {
        /// The container.
        container: String,
    },
    /// A container told to go, with everything in it.
    Discarded {
        /// The container.
        container: String,
    },
    /// Where a job's tunnel is published on the host, for whoever is
    /// waiting to be forwarded there.
    Port {
        /// Whose.
        job: JobId,
    },
    /// Where a job's tunnel is published on the host, on the way to probing
    /// it. Apart from [`Asked::Port`] because the answers go different
    /// ways: one is told to whoever is waiting, and this one is probed.
    Probing {
        /// Whose.
        job: JobId,
    },
    /// Whether a container is there, and which agent it was made for.
    Inspected {
        /// The container.
        container: String,
    },
    /// Which containers are up, for the settling sweep.
    Listing,
    /// What one label on one container of a listing says.
    Labelled {
        /// The container.
        name: String,
        /// Which label.
        label: stageman_agent::Label,
    },
    /// Which images this project has built and still holds.
    Images,
    /// One image told to go, in the housekeeping that follows a removal.
    Reclaimed {
        /// Its name and tag.
        image: String,
    },
    /// Whether a turn's image is there.
    Present {
        /// Whose turn.
        speaker: Speaker,
    },
    /// A build of an image, for every turn waiting on it.
    Built {
        /// Its name and tag.
        image: String,
    },
    /// Whether a turn's container was made.
    Created {
        /// Whose turn.
        speaker: Speaker,
    },
    /// Whether a turn's container is up.
    Started {
        /// Whose turn.
        speaker: Speaker,
    },
    /// Whether a turn's repository was checked out.
    CheckedOut {
        /// Whose turn.
        speaker: Speaker,
    },
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
    /// What every command that runtime is given gets for an environment.
    pub runtime_environment: Environment,
    /// The port the tools are served on, which is what a container is told.
    pub tools: u16,
    /// The listener they arrive on, where one was taken.
    pub tools_listener: Option<EffectId>,
    /// The listener a person's requests arrive on.
    pub dashboard_listener: Option<EffectId>,
    /// The loopback port the presentation server answers on.
    pub presenting: u16,
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
    /// What every command it is given gets for an environment: this
    /// process's own, less this project's, decided once while booting.
    runtime_environment: Environment,
    /// Where a container reaches the tools this instance serves, composed
    /// from the port that was actually taken.
    tools: String,
    /// Which listener a tool call arrives on, where one was taken.
    tools_listener: Option<EffectId>,
    /// Which listener a person's requests arrive on.
    dashboard_listener: Option<EffectId>,
    /// Where the presentation server is, which is where everything that is
    /// not a job's tunnel is forwarded.
    presenting: u16,
    /// What each tool call said about itself before its body was read.
    calls: BTreeMap<stageman_vocabulary::RequestId, Called>,
    /// What a listing being assembled has learned so far, by container.
    ///
    /// A listing is one question and then two more per container it names,
    /// so this is what the answers accumulate into until the last arrives.
    listing: BTreeMap<String, (Option<InstanceId>, Option<Agent>)>,
    /// What the runtime was asked, by the identifier its answer will carry.
    ///
    /// Held and never kept: an answer arriving after this process dies is
    /// answered to nobody, and the next start asks again whatever still
    /// matters.
    asked: BTreeMap<EffectId, Asked>,
    /// The one generator.
    rng: StdRng,
    /// The next effect identifier.
    next: u64,
    /// The turns in flight, by whose they are.
    turns: BTreeMap<Speaker, Turn>,
    /// Whose turn each open agent process belongs to, by the identifier the
    /// process was opened under: what routes a line back to its conversation.
    talking: BTreeMap<EffectId, Speaker>,
    /// The builds in flight, by image, with every turn waiting on each. One
    /// build at a time per image, for the reason `crate::turns` gives.
    building: BTreeMap<String, Vec<Speaker>>,
    /// The credentials minted for those turns.
    warrants: BTreeMap<String, Warranted>,
    /// The foremen found mid-turn on waking, until each is picked up.
    interrupted: BTreeSet<ProjectId>,
    /// Requests on the tools endpoint waiting on a post, by the identifier
    /// the world holds each one open under.
    asking: BTreeMap<stageman_vocabulary::RequestId, Option<serde_json::Value>>,
    /// Where each job's tunnel was last found.
    tunnels: BTreeMap<JobId, u16>,
    /// Requests waiting for the runtime to say where a job's tunnel is, by
    /// the identifier the world holds each one open under.
    routing: BTreeMap<JobId, Vec<stageman_vocabulary::RequestId>>,
    /// Tunnels being asked whether anything is behind them, by the
    /// identifier the answer carries: whose each probe is.
    probes: BTreeMap<EffectId, JobId>,
    /// Requests made to a channel, by the identifier the answer carries:
    /// what each was sent for.
    sent: BTreeMap<EffectId, channel::Sent>,
    /// Effects waiting on a write, by the write they wait on, in order.
    deferred: VecDeque<(EffectId, Vec<Effect>)>,
    /// The wakes asked for that have not gone off, and what each was for.
    timers: BTreeMap<EffectId, Timer>,
    /// Every project whose channel is being listened to, and where its
    /// connection has got to.
    listeners: BTreeMap<ProjectId, listening::Listener>,
    /// Every socket open, by the identifier the world knows it under: whose
    /// channel each is, for as long as it is open.
    sockets: BTreeMap<EffectId, ProjectId>,
    /// Sockets the platform said it would close, read until they do while
    /// their replacements are opened.
    draining: BTreeSet<EffectId>,
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
            runtime_environment,
            tools,
            tools_listener,
            dashboard_listener,
            presenting,
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
            runtime_environment,
            tools: paths::tools_endpoint(tools),
            tools_listener,
            dashboard_listener,
            presenting,
            calls: BTreeMap::new(),
            asked: BTreeMap::new(),
            listing: BTreeMap::new(),
            rng,
            next,
            turns: BTreeMap::new(),
            talking: BTreeMap::new(),
            building: BTreeMap::new(),
            warrants: BTreeMap::new(),
            interrupted: BTreeSet::new(),
            asking: BTreeMap::new(),
            tunnels: BTreeMap::new(),
            routing: BTreeMap::new(),
            probes: BTreeMap::new(),
            sent: BTreeMap::new(),
            deferred: VecDeque::new(),
            timers: BTreeMap::new(),
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            draining: BTreeSet::new(),
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

    /// What waking asks for: the sweep, and the first write.
    pub(crate) fn waking_up(&mut self, containers: &[Container]) -> Vec<Effect> {
        let (mut effects, tally) = self.waking(containers);
        self.swept = Some(tally);
        self.flush(&mut effects);
        effects
    }

    /// Asks the runtime something, and remembers what for.
    ///
    /// Every command is rendered here rather than by whoever performs it, so
    /// what a scenario shows is the argument list that actually runs — and
    /// the answer is routed by the identifier it carries back.
    fn ask(&mut self, command: &Command, asked: Asked) -> Effect {
        self.asking(command, asked, self.runtime_environment.clone(), None)
    }

    /// Asks the runtime something with an environment and an input of its
    /// own: the one that makes a container carries its credentials, and the
    /// one that builds an image carries its recipe.
    fn asking(
        &mut self,
        command: &Command,
        asked: Asked,
        environment: Environment,
        stdin: Option<stageman_vocabulary::Bytes>,
    ) -> Effect {
        let id = self.effect_id();
        self.asked.insert(id, asked);
        Generic::Run {
            id,
            program: self.runtime.clone(),
            arguments: command.arguments(),
            environment,
            stdin,
        }
    }

    /// Asks for a container to go, with everything in it.
    ///
    /// Named because four places want it and the pairing of the command with
    /// what its answer means should be written once.
    fn discard(&mut self, container: String) -> Effect {
        self.ask(
            &Command::Discard {
                name: container.clone(),
            },
            Asked::Discarded { container },
        )
    }

    /// What the runtime said about something this instance asked it.
    fn ran(&mut self, id: EffectId, finished: &Finished, effects: &mut Vec<Effect>) {
        let Some(asked) = self.asked.remove(&id) else {
            tracing::warn!("a program finished that this instance did not ask about; ignored");
            return;
        };
        match asked {
            // Housekeeping, so a failure is said and nothing else changes:
            // what is left is a container nothing needs, which is exactly
            // what the next sweep looks for.
            Asked::Halted { container } => {
                if let Some(why) = complaint(finished) {
                    tracing::warn!(%container, %why, "a container could not be stopped");
                }
            }
            Asked::Discarded { container } => {
                if let Some(why) = complaint(finished) {
                    tracing::warn!(
                        %container,
                        %why,
                        "a container could not be removed; waking will try again"
                    );
                }
            }
            // A container that is not there refuses, and so does a runtime
            // that will not answer; either way nothing can be resumed in it,
            // which is the one thing the answer decides.
            Asked::Inspected { container } => {
                // Nothing to complain about is the whole of what "it is
                // there" means: a container that is not there makes the
                // runtime say so, and so does a runtime that will not
                // answer at all.
                let present = complaint(finished).is_none();
                let agent = stageman_agent::labelled(said(finished).trim());
                self.inspected(&container, present, agent, effects);
            }
            // What a listing of images leads to is a decision about which
            // this build would only rebuild, which is knowledge about
            // images and lives with them; what is left here is the asking.
            Asked::Images => {
                if let Some(why) = complaint(finished) {
                    tracing::warn!(%why, "could not ask which images are ours");
                    return;
                }
                let keeping = stageman_agent::keeping();
                for image in stageman_agent::tagged(said(finished)) {
                    if stageman_agent::kept(&keeping, &image) {
                        continue;
                    }
                    let removal = self.ask(
                        &Command::RemoveImage {
                            image: image.clone(),
                        },
                        Asked::Reclaimed { image },
                    );
                    effects.push(removal);
                }
            }
            Asked::Reclaimed { image } => {
                if let Some(why) = complaint(finished) {
                    tracing::warn!(%image, %why, "an image nothing needs could not be removed");
                } else {
                    tracing::info!(%image, "reclaimed an image no container needed");
                }
            }
            Asked::Listing => self.listing(finished, effects),
            Asked::Labelled { name, label } => self.labelled(&name, label, finished, effects),
            // A turn's steps, each answered to the turn it belongs to.
            Asked::Present { speaker } => self.looked(speaker, finished, effects),
            Asked::Built { image } => self.built(&image, finished, effects),
            Asked::Created { speaker } => self.made(speaker, finished, effects),
            Asked::Started { speaker } => self.held(speaker, finished, effects),
            Asked::CheckedOut { speaker } => self.checked_out(speaker, finished, effects),
            Asked::Port { job } => {
                if let Some(why) = complaint(finished) {
                    tracing::debug!(%job, %why, "the runtime could not say where a job's tunnel is");
                }
                // Nothing published reads as no line at all, which is what a
                // container with no tunnel prints, so an empty answer and a
                // failed one mean the same thing here: nowhere to send them.
                let port = stageman_agent::published(said(finished));
                self.port_found(job, port, effects);
            }
            Asked::Probing { job } => {
                if let Some(why) = complaint(finished) {
                    tracing::debug!(%job, %why, "the runtime could not say where a job's tunnel is");
                }
                let port = stageman_agent::published(said(finished));
                self.probing(job, port, effects);
            }
        }
    }

    /// A request arrived on one of the addresses this instance took.
    ///
    /// Which listener it came in on is the whole of the routing at this
    /// level: one serves the tools an agent calls, and the other serves
    /// whoever typed an address.
    fn arrived(
        &mut self,
        listener: EffectId,
        id: stageman_vocabulary::RequestId,
        request: &stageman_vocabulary::Arrival,
        effects: &mut Vec<Effect>,
    ) {
        if Some(listener) == self.tools_listener {
            self.called(id, request, effects);
        } else if Some(listener) == self.dashboard_listener {
            self.visited(id, request, effects);
        } else {
            tracing::warn!("a request arrived on a listener this instance did not take; refused");
            effects.push(Generic::Answer {
                id,
                answer: stageman_vocabulary::Answer::Respond {
                    status: 404,
                    headers: BTreeMap::new(),
                    body: stageman_vocabulary::Bytes::new(Vec::new()),
                },
            });
        }
    }

    /// The runtime said which containers are up.
    ///
    /// A listing is assembled here rather than by whoever runs the commands:
    /// the names come back first, and each is then asked what its labels
    /// say, which is the shape booting uses before it is awake.
    fn listing(&mut self, finished: &Finished, effects: &mut Vec<Effect>) {
        if let Some(why) = complaint(finished) {
            tracing::warn!(%why, "could not ask which containers are running");
        }
        self.listing.clear();
        let mut asking = Vec::new();
        for name in stageman_agent::names(said(finished)) {
            self.listing.insert(name.clone(), (None, None));
            for label in [
                stageman_agent::Label::Instance,
                stageman_agent::Label::Agent,
            ] {
                asking.push(self.ask(
                    &Command::Label {
                        name: name.clone(),
                        label,
                    },
                    Asked::Labelled {
                        name: name.clone(),
                        label,
                    },
                ));
            }
        }
        if asking.is_empty() {
            self.listed(&[], effects);
        } else {
            effects.extend(asking);
        }
    }

    /// The runtime said what one label on one listed container says.
    ///
    /// The listing is finished when nothing is left to answer, which is what
    /// says every container in it has been placed.
    fn labelled(
        &mut self,
        name: &str,
        label: stageman_agent::Label,
        finished: &Finished,
        effects: &mut Vec<Effect>,
    ) {
        if complaint(finished).is_some() {
            tracing::warn!(container = %name, "could not read a container's label");
        }
        let text = said(finished);
        if let Some(entry) = self.listing.get_mut(name) {
            match label {
                stageman_agent::Label::Instance => entry.0 = stageman_agent::minted(text),
                stageman_agent::Label::Agent => entry.1 = stageman_agent::labelled(text.trim()),
            }
        }
        if self
            .asked
            .values()
            .any(|asked| matches!(asked, Asked::Labelled { .. }))
        {
            return;
        }
        let running: Vec<Container> = std::mem::take(&mut self.listing)
            .into_iter()
            .map(|(name, (instance, agent))| Container {
                name,
                instance,
                agent,
                // Only the running ones were asked for.
                running: true,
            })
            .collect();
        self.listed(&running, effects);
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
            Event::Ran { id, finished } => self.ran(id, &finished, &mut effects),
            // Nothing here asks for a file or a listener while awake yet;
            // both arrive as families land, and until then an answer to
            // something nobody asked is said rather than acted on.
            Event::Arrived {
                listener,
                id,
                request,
            } => self.arrived(listener, id, &request, &mut effects),
            Event::Body { id, outcome } => self.read(id, outcome, &mut effects),
            Event::Line { id, line } => self.line(id, &line, &mut effects),
            Event::Ended { id, ended } => self.process_ended(id, &ended, &mut effects),
            Event::Probed { id, probed } => self.probed(id, probed, &mut effects),
            Event::Responded { id, responded, at } => {
                self.responded(id, &responded, at, &mut effects);
            }
            Event::Frame { id, text, at } => self.frame(id, &text, at, &mut effects),
            Event::Disconnected {
                id,
                disconnected,
                at,
            } => self.disconnected(id, &disconnected, at, &mut effects),
            Event::Read { .. } | Event::Bound { .. } => {
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
            AppEvent::Presenting { .. } => {
                tracing::debug!("told again where the presentation server is; ignored");
            }
            AppEvent::Request { id, request } => self.requested(id, *request, effects),
        }
    }

    /// A timer went off.
    fn woke(&mut self, id: EffectId, effects: &mut Vec<Effect>) {
        match self.timers.remove(&id) {
            Some(Timer::Settling) => {
                let listing = self.ask(&Command::Containers { running_only: true }, Asked::Listing);
                effects.push(listing);
                let settling = self.settle_later();
                effects.push(settling);
            }
            Some(Timer::Reconnecting { project }) => self.try_again(project, effects),
            None => tracing::warn!("woken for a timer this instance did not set; ignored"),
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
                    self.dropped(waiting, effects);
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
    /// A command the runtime was to be given is not given, and forgotten as
    /// asked, so its answer is not waited for. A turn whose first step that
    /// was is not started, and the job it was for is recorded as failed for
    /// that reason: the alternative is a job that says working with nothing
    /// running in it, which a reply could never reach. The record is a
    /// change of its own, so the next write carries it — and if that one
    /// lands, the job can be given something again. A request to a channel
    /// is not made, and whatever waited on its answer hears that instead:
    /// a notice is simply not said, and a job whose thread was never opened
    /// is failed the same way a turn never started is.
    fn dropped(&mut self, dropped: Vec<Effect>, effects: &mut Vec<Effect>) {
        tracing::warn!(
            dropped = dropped.len(),
            "what waited on the write is dropped"
        );
        for effect in dropped {
            match effect {
                Effect::Run { id, .. } => {
                    if let Some(Asked::Present { speaker } | Asked::Started { speaker }) =
                        self.asked.remove(&id)
                    {
                        self.abandoned(speaker);
                    }
                }
                Effect::Request { id, .. } => self.unsent(id, effects),
                _ => {}
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
                self.dropped(staged, effects);
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

/// What a runtime command printed, when it ran and was happy.
///
/// Anything else printed nothing worth reading: a command that failed has
/// its reason on the other stream, which is [`complaint`].
fn said(finished: &Finished) -> &str {
    match finished {
        Finished::Exited {
            status: Some(0),
            stdout,
            ..
        } => stdout.as_text().unwrap_or(""),
        _ => "",
    }
}

/// Why a runtime command failed, if it did.
///
/// A command that was run and exited cleanly says nothing; anything else is
/// worth a line, and the line is the runtime's own complaint where there is
/// one.
fn complaint(finished: &Finished) -> Option<String> {
    match finished {
        Finished::Exited {
            status: Some(0), ..
        } => None,
        Finished::Exited { stderr, .. } => Some(stderr.as_text().unwrap_or("").trim().to_owned()),
        Finished::NotFound => Some("the runtime could not be run".to_owned()),
        Finished::Failed(why) => Some(why.clone()),
    }
}
