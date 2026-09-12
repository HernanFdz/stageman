//! The deciding and the doing as one deterministic value.
//!
//! An [`Instance`] holds everything one running stageman knows and makes every
//! decision it makes. It is built from the bytes of its file, if there is one,
//! and from what the runtime holds; after that it has one method, which takes
//! one [`Event`] and answers with [`Effect`]s. It never reads a clock, never
//! draws on entropy beyond the generator it was seeded with, and performs
//! nothing: the world does, and reports back as events. See
//! `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`.
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
//! truth, and a job's record is on the disk before its container exists.

mod file;
mod foreman;
mod jobs;
mod replies;
mod requests;
mod sweep;
mod tools;
mod tunnel;
mod turns;
mod views;
mod vocabulary;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng as _, SeedableRng as _};
use stageman_core::{InstanceId, JobId, Key, Kit, Progress, ProjectId, State, Thread, Uuid};

pub use file::LoadError;
pub use requests::{Request, Response};
pub use stageman_vocabulary::Seed;
pub use sweep::Swept;
pub use tunnel::{DEFAULT_DOMAIN, Domain, Routed, address, decode};
pub use vocabulary::{
    AppEffect, AppEvent, Container, Message, Posting, RequestId, Run, Speaker, Startup, Timer,
    Warranted,
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

impl stageman_vocabulary::Deciding for Instance {
    type App = Stageman;

    fn step(&mut self, event: Event) -> Vec<Effect> {
        Self::step(self, event)
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

/// How often the instance asks which containers still deserve to be up.
///
/// A server an agent left running does not say when it stops, so the only
/// way to notice is to look — see
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`.
const SETTLING_INTERVAL: Duration = Duration::from_mins(1);

/// Everything one running stageman knows, and every decision it makes.
pub struct Instance {
    /// What goes to the disk.
    state: State,
    /// Which instance this is, for as long as this file is the instance.
    id: InstanceId,
    /// What seals the file.
    key: Key,
    /// The domain this instance answers on.
    domain: Domain,
    /// The port the dashboard is bound to.
    serving: u16,
    /// What this build calls itself.
    build: String,
    /// Where the container runtime was found, for the dashboard.
    runtime: String,
    /// The only randomness there is.
    rng: StdRng,
    /// The turns running right now, by whose they are.
    turns: BTreeMap<Speaker, Turn>,
    /// Which credential the tools endpoint may be shown, and by whom.
    warrants: BTreeMap<String, Warranted>,
    /// Projects whose foreman was found holding a message on waking, and
    /// whose next turn is therefore told it was interrupted.
    interrupted: BTreeSet<ProjectId>,
    /// Tool calls waiting on the platform before they can be answered, with
    /// the identifier the agent sent, to answer under.
    asking: BTreeMap<RequestId, Option<serde_json::Value>>,
    /// Where each job's tunnel was last found. Held, not kept: the runtime
    /// publishes on a fresh port at every start, so what survives a restart
    /// is wrong by construction.
    tunnels: BTreeMap<JobId, u16>,
    /// Tunnel requests waiting for the runtime to say where a job is.
    routing: BTreeMap<JobId, Vec<RequestId>>,
    /// Effects waiting for a write to land, one entry per write asked for.
    deferred: VecDeque<Vec<Effect>>,
    /// Whether the kept state changed since it was last written.
    dirty: bool,
    /// Effects this step wants held back until its write lands.
    staged: Vec<Effect>,
}

/// An instance that has just been opened, and what it does about it.
pub struct Woken {
    /// The instance.
    pub instance: Instance,
    /// What it asks of the world on waking.
    pub effects: Vec<Effect>,
    /// What waking found.
    pub swept: Swept,
}

impl Instance {
    /// Opens an instance from its file, or starts one where there is no file,
    /// and reconciles it with what the runtime holds.
    ///
    /// Nothing can reach [`Instance::step`] before this has run, which is why
    /// the facts the sweep needs are an argument rather than a first event.
    ///
    /// # Errors
    ///
    /// Fails if the file is not JSON, cannot be opened with `key`, or
    /// describes an instance that cannot exist.
    pub fn open(
        file: Option<&[u8]>,
        key: Key,
        seed: Seed,
        startup: &Startup,
    ) -> Result<Woken, LoadError> {
        let (state, named) = file::opened(file, &key)?;
        let mut rng = StdRng::from_seed(seed);
        let id = named.unwrap_or_else(|| {
            let minted = InstanceId::from_uuid(mint(&mut rng));
            tracing::info!(instance = %minted, "this instance had no identity, so it was given one");
            minted
        });
        let mut instance = Self {
            state,
            id,
            key,
            domain: startup.domain.clone(),
            serving: startup.serving,
            build: startup.build.clone(),
            runtime: startup.runtime.clone(),
            rng,
            turns: BTreeMap::new(),
            warrants: BTreeMap::new(),
            interrupted: BTreeSet::new(),
            asking: BTreeMap::new(),
            tunnels: BTreeMap::new(),
            routing: BTreeMap::new(),
            deferred: VecDeque::new(),
            // Written once on waking, before anything can depend on this
            // instance: a first run has a file at all, a file that had no
            // identity has one from the moment it is opened, and a path that
            // cannot be written fails at startup rather than at the first
            // change — `docs/conventions.md` §3.
            dirty: true,
            staged: Vec::new(),
        };
        let (mut effects, swept) = instance.waking(startup);
        instance.flush(&mut effects);
        Ok(Woken {
            instance,
            effects,
            swept,
        })
    }

    /// Which instance this is.
    #[must_use]
    pub const fn id(&self) -> InstanceId {
        self.id
    }

    /// What this instance knows, for reading.
    ///
    /// The world reads this to say what it is serving; it decides nothing
    /// from it, because deciding is what [`Instance::step`] is for.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// What a presented credential entitles its bearer to, if this instance
    /// minted it and the turn it was minted for is still running.
    #[must_use]
    pub fn warranted(&self, presented: &str) -> Option<&Warranted> {
        self.warrants.get(presented)
    }

    /// One event in, effects out.
    ///
    /// The whole of the instance's interface after construction. Every
    /// decision the daemon makes is made in here, on one thread, in the order
    /// events arrive.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        let Event::App(event) = event;
        match event {
            AppEvent::Persisted { outcome } => self.persisted(outcome, &mut effects),
            AppEvent::TurnEnded { speaker, outcome } => self.ended(speaker, outcome, &mut effects),
            AppEvent::Probed { job, answering } => {
                if !answering {
                    // Halted, so the port it was on reaches nothing.
                    self.forget_tunnel(job);
                }
                probed(job, answering, &mut effects);
            }
            AppEvent::Listed { running } => self.listed(&running, &mut effects),
            AppEvent::Woke {
                timer: Timer::Settle,
            } => {
                effects.emit(AppEffect::ListRunning);
                effects.push(sweep::settle_later());
            }
            AppEvent::Heard { channel, message } => self.heard(channel, &message, &mut effects),
            AppEvent::Inspected {
                container,
                present,
                agent,
            } => self.inspected(&container, present, agent, &mut effects),
            AppEvent::ToolCalled {
                id,
                at,
                nearby,
                bearer,
                body,
            } => self.tool_called(id, at, nearby, bearer.as_deref(), &body, &mut effects),
            AppEvent::ThreadOpened { job, outcome } => self.thread_opened(job, outcome),
            AppEvent::Posted { request, outcome } => self.posted(request, outcome),
            AppEvent::Request { id, request } => self.requested(id, request, &mut effects),
            AppEvent::TunnelAsked { id, job } => self.tunnel_asked(id, job, &mut effects),
            AppEvent::PortFound { job, port } => self.port_found(job, port, &mut effects),
            AppEvent::TunnelFailed { job, why } => self.tunnel_failed(job, &why),
        }
        self.flush(&mut effects);
        debug_assert!(
            self.state.check().is_ok(),
            "a step left the state inconsistent"
        );
        effects
    }

    /// What the world said about a write.
    ///
    /// Persists are answered in the order they were asked for, so the front
    /// of the queue is the one being answered. A write that failed drops what
    /// waited on it: the state in memory is still right, the next change asks
    /// again, and nothing outward-facing happens on the strength of a record
    /// that is not on the disk.
    fn persisted(&mut self, outcome: Result<(), String>, effects: &mut Vec<Effect>) {
        let Some(waiting) = self.deferred.pop_front() else {
            tracing::warn!("the world answered a write nobody asked for; ignored");
            return;
        };
        match outcome {
            Ok(()) => effects.extend(waiting),
            Err(why) => {
                tracing::error!(%why, "the instance could not be written");
                self.dropped(waiting);
            }
        }
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
                self.deferred.push_back(staged);
                effects.emit(AppEffect::Persist {
                    bytes: stageman_vocabulary::Bytes::new(bytes),
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

/// A fresh identifier from the instance's own randomness.
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
