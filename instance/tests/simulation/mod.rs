//! The simulated world: everything the instance is not, in memory, stepped
//! deterministically.
//!
//! It performs effects by changing its own picture of the runtime and
//! scheduling the events that answer them at virtual instants, and it
//! records every event and effect as a trace a test can compare. A crash is
//! a method: turns, timers and unanswered writes die, containers stay as they
//! are, and the disk is what landed.

use std::collections::{BTreeMap, VecDeque};

use stageman_agent::{Answer, StopReason};
use stageman_core::{
    Agent, InstanceId, Key, NONCE_LEN, Nonce, Secret, Snapshot, State, Thread, Uuid,
};
use stageman_instance::{Container, Effect, Event, Instance, Run, Seed, Startup};

/// Virtual milliseconds.
pub type Now = u64;

/// One container as the simulated runtime holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub instance: Option<InstanceId>,
    pub agent: Option<Agent>,
    pub running: bool,
    /// Whether something inside is answering on the tunnel.
    pub serving: bool,
}

pub struct Simulation {
    now: Now,
    seq: u64,
    queue: BTreeMap<(Now, u64), Event>,
    containers: BTreeMap<String, Held>,
    disk: Option<Vec<u8>>,
    /// Writes asked for and not yet landed, in order.
    landing: VecDeque<Vec<u8>>,
    /// How the next turns end, front first; a turn with nothing scripted ends
    /// cleanly having said nothing.
    answers: VecDeque<Result<Answer, String>>,
    /// How long a turn takes.
    turn_takes: Now,
    trace: Vec<String>,
    posts: Vec<(Thread, String)>,
    listening: Vec<stageman_core::ProjectId>,
    reclaims: usize,
    warrants: Vec<Secret>,
    key: Key,
}

/// The identity every simulated instance has.
pub const fn this_instance() -> InstanceId {
    InstanceId::from_uuid(Uuid::from_u128(0xabc))
}

/// A stranger's identity.
pub const fn another_instance() -> InstanceId {
    InstanceId::from_uuid(Uuid::from_u128(0xdef))
}

pub const fn key() -> Key {
    Key::new([7; 32])
}

pub const fn seed(n: u8) -> Seed {
    [n; 32]
}

impl Simulation {
    pub const fn new() -> Self {
        Self {
            now: 0,
            seq: 0,
            queue: BTreeMap::new(),
            containers: BTreeMap::new(),
            disk: None,
            landing: VecDeque::new(),
            answers: VecDeque::new(),
            turn_takes: 1_000,
            trace: Vec::new(),
            posts: Vec::new(),
            listening: Vec::new(),
            reclaims: 0,
            warrants: Vec::new(),
            key: key(),
        }
    }

    /// Puts a state on the disk, sealed as the instance would seal it.
    pub fn holding(&mut self, state: &State) {
        let mut counter: u8 = 0;
        let mut nonces = || {
            counter += 1;
            let nonce: Nonce = [counter; NONCE_LEN];
            nonce
        };
        let mut snapshot = state
            .seal(&self.key, &mut nonces)
            .expect("a well-formed state seals");
        snapshot.instance = Some(this_instance());
        self.disk = Some(serde_json::to_vec_pretty(&snapshot).expect("a snapshot encodes"));
    }

    /// Puts a container in the runtime.
    pub fn container(&mut self, name: &str, held: Held) {
        self.containers.insert(name.to_owned(), held);
    }

    /// A container of this instance's, stopped and showing nothing.
    pub fn ours(name: &str) -> (String, Held) {
        (
            name.to_owned(),
            Held {
                instance: Some(this_instance()),
                agent: Some(Agent::Claude),
                running: false,
                serving: false,
            },
        )
    }

    /// Scripts how the next turn ends.
    pub fn next_turn_ends(&mut self, outcome: Result<Answer, String>) {
        self.answers.push_back(outcome);
    }

    /// What the runtime holds, as the world reports it before the instance
    /// exists.
    pub fn startup(&self) -> Startup {
        Startup {
            containers: self
                .containers
                .iter()
                .map(|(name, held)| Container {
                    name: name.clone(),
                    instance: held.instance,
                    agent: held.agent,
                    running: held.running,
                })
                .collect(),
        }
    }

    /// Opens the instance from the disk and performs what it does on waking.
    pub fn wake(&mut self, seed: Seed) -> Instance {
        let woken = Instance::open(
            self.disk.as_deref(),
            self.key.clone(),
            seed,
            &self.startup(),
        )
        .expect("the file opens");
        self.trace
            .push(format!("{}: woke, swept {:?}", self.now, woken.swept));
        for effect in woken.effects {
            self.perform(effect);
        }
        woken.instance
    }

    /// Kills the daemon and starts it again.
    pub fn crash(&mut self, seed: Seed) -> Instance {
        self.queue.retain(|_, event| {
            !matches!(
                event,
                Event::Persisted { .. }
                    | Event::TurnEnded { .. }
                    | Event::Probed { .. }
                    | Event::Listed { .. }
                    | Event::Woke { .. }
            )
        });
        self.landing.clear();
        self.trace.push(format!("{}: CRASH", self.now));
        self.wake(seed)
    }

    pub fn schedule(&mut self, at: Now, event: Event) {
        self.seq += 1;
        self.queue.insert((at, self.seq), event);
    }

    /// The next event, if there is one. A write lands as its completion is
    /// delivered, never before.
    pub fn next(&mut self) -> Option<Event> {
        let ((at, _), event) = self.queue.pop_first()?;
        self.now = at;
        if matches!(event, Event::Persisted { .. }) {
            self.disk = self.landing.pop_front();
        }
        self.trace.push(format!("{at}: <- {event:?}"));
        Some(event)
    }

    /// Steps the instance until the given virtual instant, leaving later
    /// events queued.
    pub fn run_until(&mut self, instance: &mut Instance, until: Now) {
        while let Some(&(at, _)) = self.queue.keys().next() {
            if at > until {
                break;
            }
            let Some(event) = self.next() else { break };
            for effect in instance.step(event) {
                self.perform(effect);
            }
            self.oracle(instance);
        }
    }

    /// What must hold between the instance and the world after every step.
    fn oracle(&self, instance: &Instance) {
        for job in instance.state().working() {
            assert!(
                self.containers.contains_key(&stageman_job::container(job)),
                "job {job} is working with no container"
            );
        }
    }

    pub fn perform(&mut self, effect: Effect) {
        self.trace.push(format!("{}: -> {effect:?}", self.now));
        match effect {
            Effect::Persist { bytes } => {
                self.landing.push_back(bytes);
                self.schedule(self.now + 1, Event::Persisted { outcome: Ok(()) });
            }
            Effect::RunTurn { speaker, run } => {
                let (container, warrant) = match &run {
                    Run::Begin {
                        container, warrant, ..
                    }
                    | Run::Resume {
                        container, warrant, ..
                    } => (container.clone(), warrant.clone()),
                };
                self.warrants.push(warrant);
                let outcome = match self.containers.get_mut(&container) {
                    Some(held) => {
                        held.running = true;
                        self.answers.pop_front().unwrap_or_else(|| {
                            Ok(Answer {
                                text: "done".to_owned(),
                                stop_reason: StopReason::EndTurn,
                                reported: BTreeMap::new(),
                            })
                        })
                    }
                    None => Err(format!("no such container: {container}")),
                };
                let at = self.now + self.turn_takes;
                self.schedule(at, Event::TurnEnded { speaker, outcome });
            }
            Effect::Probe { job } => {
                let answering = self
                    .containers
                    .get(&stageman_job::container(job))
                    .is_some_and(|held| held.running && held.serving);
                self.schedule(self.now, Event::Probed { job, answering });
            }
            Effect::ListRunning => {
                let running = self
                    .containers
                    .iter()
                    .filter(|(_, held)| held.running)
                    .map(|(name, held)| Container {
                        name: name.clone(),
                        instance: held.instance,
                        agent: held.agent,
                        running: true,
                    })
                    .collect();
                self.schedule(self.now, Event::Listed { running });
            }
            Effect::Halt { container } => {
                if let Some(held) = self.containers.get_mut(&container) {
                    held.running = false;
                }
            }
            Effect::Discard { container } => {
                self.containers.remove(&container);
            }
            Effect::Reclaim => self.reclaims += 1,
            Effect::Wake { after, timer } => {
                let at = self.now + u64::try_from(after.as_millis()).expect("a short wait");
                self.schedule(at, Event::Woke { timer });
            }
            Effect::Say { thread, text, .. } => self.posts.push((thread, text)),
            Effect::Listen { project, .. } => {
                self.listening.push(project);
                self.trace.push(format!(
                    "{}: listening on {} project(s)",
                    self.now,
                    self.listening.len()
                ));
            }
        }
    }

    pub fn trace(&self) -> &[String] {
        &self.trace
    }

    /// The trace without its timestamps, for comparing shapes.
    pub fn shape(&self) -> Vec<String> {
        self.trace
            .iter()
            .map(|line| {
                line.split_once(": ")
                    .map_or_else(|| line.clone(), |(_, rest)| rest.to_owned())
            })
            .collect()
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.containers.get(name).is_some_and(|held| held.running)
    }

    pub fn exists(&self, name: &str) -> bool {
        self.containers.contains_key(name)
    }

    pub fn posts(&self) -> &[(Thread, String)] {
        &self.posts
    }

    pub const fn reclaims(&self) -> usize {
        self.reclaims
    }

    /// What is on the disk, opened.
    pub fn disk(&self) -> Option<State> {
        let bytes = self.disk.as_deref()?;
        let snapshot: Snapshot = serde_json::from_slice(bytes).expect("the disk holds JSON");
        Some(snapshot.open(&self.key).expect("and it opens"))
    }

    /// The credentials handed to turns so far, oldest first.
    pub fn warrants(&self) -> &[Secret] {
        &self.warrants
    }
}

impl Default for Simulation {
    fn default() -> Self {
        Self::new()
    }
}
