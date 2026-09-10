//! The simulated world: everything the instance is not, in memory, stepped
//! deterministically.
//!
//! It performs effects by changing its own picture of the runtime and
//! scheduling the events that answer them at virtual instants, and it
//! records every event and effect as a trace a test can compare. A crash is
//! a method: turns, timers and unanswered writes die, containers stay as they
//! are, and the disk is what landed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stageman_agent::{Answer, StopReason};
use stageman_core::{
    Agent, AgentConfig, Channel, ChannelConfig, Errand, InstanceId, Job, JobId, Key, Kit,
    KitConfig, KitName, NONCE_LEN, Nonce, Progress, Project, ProjectId, Secret, Snapshot, State,
    Thread, Timestamp, Uuid,
};
use stageman_instance::{
    Container, Domain, Effect, Event, Instance, Message, Request, RequestId, Response, Run, Seed,
    Startup,
};

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
    /// The host port its tunnel is published on, once it has been started.
    /// A new one on every start, as the runtime does.
    pub port: Option<u16>,
}

pub struct Simulation {
    now: Now,
    seq: u64,
    queue: BTreeMap<(Now, u64), Event>,
    containers: BTreeMap<String, Held>,
    disk: Option<Vec<u8>>,
    /// Writes asked for and not yet answered, in order; `None` is one that
    /// will fail rather than land.
    landing: VecDeque<Option<Vec<u8>>>,
    /// Why the next writes fail, front first.
    write_failures: VecDeque<String>,
    /// How the next turns end, front first; a turn with nothing scripted ends
    /// cleanly having said nothing.
    answers: VecDeque<Result<Answer, String>>,
    /// Why the next posts on an agent's behalf fail, front first.
    post_failures: VecDeque<String>,
    /// What each request was answered with, by identifier.
    tool_answers: BTreeMap<RequestId, (u16, Option<serde_json::Value>)>,
    /// What each person's request was answered with, by identifier.
    responses: BTreeMap<RequestId, Response>,
    /// Where each tunnel request was sent, by identifier.
    routes: BTreeMap<RequestId, Sent>,
    /// The last host port handed out.
    ports: u16,
    /// Threads opened so far, so each gets a number of its own.
    threads_opened: u32,
    /// Jobs whose first turn the world has been asked to run. A job is on the
    /// record before its container exists, so only after this may the oracle
    /// expect the container.
    begun: BTreeSet<JobId>,
    /// How long a turn takes.
    turn_takes: Now,
    trace: Vec<String>,
    posts: Vec<(Thread, String)>,
    listening: Vec<stageman_core::ProjectId>,
    reclaims: usize,
    warrants: Vec<Secret>,
    key: Key,
}

/// Where a tunnel request was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// Nothing answers on that name.
    Nowhere,
    /// Forwarded to this host port.
    To(u16),
}

/// The identity every simulated instance has.
pub const fn this_instance() -> InstanceId {
    InstanceId::from_uuid(Uuid::from_u128(0xabc))
}

/// A stranger's identity.
pub const fn another_instance() -> InstanceId {
    InstanceId::from_uuid(Uuid::from_u128(0xdef))
}

/// The one project every scenario watches.
pub const fn project() -> ProjectId {
    ProjectId::from_uuid(Uuid::from_u128(11))
}

/// Where that project's channel is.
pub const CHANNEL: &str = "C0123456789";

/// A job of that project, by number.
pub const fn job(n: u128) -> JobId {
    JobId::from_uuid(Uuid::from_u128(n))
}

/// A thread on the project's channel, named by number.
pub fn thread(n: u32) -> Thread {
    Thread {
        channel: Channel::Slack,
        id: format!("1788000000.{n:06}"),
    }
}

fn a_job(progress: &Progress, thread: Option<Thread>) -> Job {
    let mut job = Job::new(
        Kit::defaults(Agent::Claude),
        "a reason".to_owned(),
        "some work".to_owned(),
        Timestamp::UNIX_EPOCH,
    );
    job.progress = progress.clone();
    job.thread = thread;
    job
}

fn a_project(jobs: BTreeMap<JobId, Job>, bound: bool) -> Project {
    let mut channels = BTreeMap::new();
    if bound {
        channels.insert(
            Channel::Slack,
            ChannelConfig {
                address: CHANNEL.to_owned(),
                credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                listen_credential: Some(Secret::new("xapp-not-a-real-token".to_owned())),
            },
        );
    }
    Project {
        name: "example".to_owned(),
        repository: "https://example.invalid/repo".to_owned(),
        foreman_kit: Kit::defaults(Agent::Claude),
        kits: BTreeMap::from([(
            KitName::new("Claude").expect("a name"),
            KitConfig::defaults(Agent::Claude),
        )]),
        credentials: BTreeMap::new(),
        channels,
        jobs,
        variables: BTreeMap::new(),
        attending: stageman_core::Attending::default(),
    }
}

fn configured(project: Project) -> State {
    State {
        agents: BTreeMap::from([(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new("agent-token".to_owned()),
            },
        )]),
        projects: BTreeMap::from([(self::project(), project)]),
    }
}

/// An instance watching one project with no channel, whose jobs are in the
/// given states.
pub fn watching(jobs: &[(JobId, Progress)]) -> State {
    configured(a_project(
        jobs.iter()
            .map(|(id, progress)| (*id, a_job(progress, None)))
            .collect(),
        false,
    ))
}

/// An instance watching one project with a channel bound, each job speaking
/// in the thread numbered for it.
pub fn watching_a_channel(jobs: &[(JobId, Progress, u32)]) -> State {
    configured(a_project(
        jobs.iter()
            .map(|(id, progress, thread)| (*id, a_job(progress, Some(self::thread(*thread)))))
            .collect(),
        true,
    ))
}

/// Somebody mentioning this instance in a thread on the project's channel.
pub fn said_in(thread: u32, text: &str) -> Event {
    Event::Heard {
        channel: Channel::Slack,
        message: Message {
            address: CHANNEL.to_owned(),
            id: "1788000099.000001".to_owned(),
            thread: Some(self::thread(thread).id),
            text: text.to_owned(),
            mentions: true,
            from_us: false,
        },
    }
}

/// Somebody mentioning this instance at the root of the project's channel.
/// Each is its own message, so each opens its own thread.
pub fn said_at_root(n: u32, text: &str) -> Event {
    Event::Heard {
        channel: Channel::Slack,
        message: Message {
            address: CHANNEL.to_owned(),
            id: thread(n).id,
            thread: None,
            text: text.to_owned(),
            mentions: true,
            from_us: false,
        },
    }
}

/// Puts a message in the project's foreman's hands, as a daemon that died
/// mid-turn would have left it.
pub fn holding_a_message(state: &mut State, n: u32, text: &str) {
    state
        .projects
        .get_mut(&project())
        .expect("the project")
        .attending
        .take(Errand {
            said: text.to_owned(),
            thread: thread(n),
        });
}

/// A request arriving for a job's name under the domain.
pub const fn tunnel_asked(id: u64, job: JobId) -> Event {
    Event::TunnelAsked {
        id: RequestId(id),
        job,
    }
}

/// A person asking something of the dashboard.
pub const fn request(id: u64, request: Request) -> Event {
    Event::Request {
        id: RequestId(id),
        request,
    }
}

/// A call on the tools endpoint from this machine, presenting a credential.
pub fn tool_call(id: u64, bearer: &Secret, body: serde_json::Value) -> Event {
    Event::ToolCalled {
        id: RequestId(id),
        at: Timestamp::UNIX_EPOCH,
        nearby: true,
        bearer: Some(bearer.expose().to_owned()),
        body,
    }
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
            write_failures: VecDeque::new(),
            answers: VecDeque::new(),
            post_failures: VecDeque::new(),
            tool_answers: BTreeMap::new(),
            responses: BTreeMap::new(),
            routes: BTreeMap::new(),
            ports: 40_000,
            threads_opened: 100,
            begun: BTreeSet::new(),
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
                port: None,
            },
        )
    }

    /// Scripts the next write to fail.
    pub fn next_write_fails(&mut self, why: &str) {
        self.write_failures.push_back(why.to_owned());
    }

    /// Scripts the next post on an agent's behalf to fail.
    pub fn next_post_fails(&mut self, why: &str) {
        self.post_failures.push_back(why.to_owned());
    }

    /// What a request was answered with, if it has been.
    pub fn tool_answer(&self, id: RequestId) -> Option<&(u16, Option<serde_json::Value>)> {
        self.tool_answers.get(&id)
    }

    /// What a person's request was answered with, if it has been.
    pub fn response(&self, id: u64) -> Option<&Response> {
        self.responses.get(&RequestId(id))
    }

    /// Where a tunnel request was sent, if it has been answered.
    pub fn route(&self, id: u64) -> Option<Sent> {
        self.routes.get(&RequestId(id)).copied()
    }

    /// The host port a container's tunnel is on now, if it is running.
    pub fn port_of(&self, name: &str) -> Option<u16> {
        self.containers
            .get(name)
            .filter(|held| held.running)
            .and_then(|held| held.port)
    }

    /// The virtual instant.
    pub const fn now(&self) -> Now {
        self.now
    }

    /// The projects listened on so far, in the order listening began.
    pub fn listening(&self) -> &[ProjectId] {
        &self.listening
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
            domain: Domain::local(),
            serving: 8080,
            build: "a test build".to_owned(),
            runtime: "/usr/local/bin/docker".to_owned(),
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
                    | Event::Inspected { .. }
                    | Event::ThreadOpened { .. }
                    | Event::Posted { .. }
                    | Event::Woke { .. }
                    | Event::Request { .. }
                    | Event::TunnelAsked { .. }
                    | Event::PortFound { .. }
                    | Event::TunnelFailed { .. }
            )
        });
        self.landing.clear();
        self.begun.clear();
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
        if matches!(event, Event::Persisted { .. })
            && let Some(Some(bytes)) = self.landing.pop_front()
        {
            self.disk = Some(bytes);
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
            // A working job has a container, unless its first turn has not
            // been asked for yet: the record is written before the container
            // exists, on purpose, and the container is made by the turn.
            assert!(
                self.containers.contains_key(&stageman_job::container(job))
                    || !self.begun.contains(&job),
                "job {job} is working with no container"
            );
        }
    }

    /// Runs a turn: beginning makes the container, resuming needs one, and
    /// how it ends is the next scripted answer or a clean ending.
    fn run_turn(&mut self, speaker: stageman_instance::Speaker, run: &Run) {
        if let stageman_instance::Speaker::Job(job) = speaker {
            self.begun.insert(job);
        }
        let (container, warrant) = match &run {
            Run::Begin {
                container, warrant, ..
            }
            | Run::Resume {
                container, warrant, ..
            } => (container.clone(), warrant.clone()),
        };
        self.warrants.push(warrant);
        if matches!(run, Run::Begin { .. }) {
            // Beginning makes the container, named before it exists.
            self.containers.insert(
                container.clone(),
                Held {
                    instance: Some(this_instance()),
                    agent: Some(Agent::Claude),
                    running: true,
                    serving: false,
                    port: None,
                },
            );
        }
        // A start publishes the tunnel on a fresh host port, as the runtime
        // does, whether the container is new or restarted.
        self.ports += 1;
        let port = self.ports;
        let outcome = match self.containers.get_mut(&container) {
            Some(held) => {
                held.running = true;
                held.port = Some(port);
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

    /// Answers whether a container exists, and whose it is.
    fn inspect(&mut self, container: String) {
        let (present, agent) = self
            .containers
            .get(&container)
            .map_or((false, None), |held| (true, held.agent));
        self.schedule(
            self.now,
            Event::Inspected {
                container,
                present,
                agent,
            },
        );
    }

    /// Answers with every container that is running.
    fn list_running(&mut self) {
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

    /// Ends the agent process, so the turn it was running ends with that
    /// rather than with whatever the agent would have said.
    fn stop_turn(&mut self, speaker: stageman_instance::Speaker) {
        self.queue.retain(|_, event| {
            !matches!(event, Event::TurnEnded { speaker: whose, .. } if *whose == speaker)
        });
        self.schedule(
            self.now,
            Event::TurnEnded {
                speaker,
                outcome: Err("stopped".to_owned()),
            },
        );
    }

    /// Accepts a write: it lands as its completion is delivered, unless it
    /// was scripted to fail.
    fn write(&mut self, bytes: Vec<u8>) {
        let outcome = if let Some(why) = self.write_failures.pop_front() {
            self.landing.push_back(None);
            Err(why)
        } else {
            self.landing.push_back(Some(bytes));
            Ok(())
        };
        self.schedule(self.now + 1, Event::Persisted { outcome });
    }

    pub fn perform(&mut self, effect: Effect) {
        self.trace.push(format!("{}: -> {effect:?}", self.now));
        match effect {
            Effect::Persist { bytes } => self.write(bytes),
            Effect::RunTurn { speaker, run } => self.run_turn(speaker, &run),
            Effect::Probe { job } => {
                let answering = self
                    .containers
                    .get(&stageman_job::container(job))
                    .is_some_and(|held| held.running && held.serving);
                self.schedule(self.now, Event::Probed { job, answering });
            }
            Effect::Inspect { container } => self.inspect(container),
            Effect::ListRunning => self.list_running(),
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
            Effect::ToolAnswered { id, status, body } => {
                self.tool_answers.insert(id, (status, body));
            }
            Effect::Respond { id, response } => {
                self.responses.insert(id, response);
            }
            Effect::Route { id, port } => {
                self.routes.insert(id, port.map_or(Sent::Nowhere, Sent::To));
            }
            Effect::FindPort { job } => {
                let port = self.port_of(&stageman_job::container(job));
                self.schedule(self.now, Event::PortFound { job, port });
            }
            Effect::StopTurn { speaker } => self.stop_turn(speaker),
            Effect::OpenThread {
                job, announcement, ..
            } => {
                self.threads_opened += 1;
                let opened = thread(self.threads_opened);
                self.posts.push((opened.clone(), announcement));
                self.schedule(
                    self.now,
                    Event::ThreadOpened {
                        job,
                        outcome: Ok(opened),
                    },
                );
            }
            Effect::Post {
                request,
                thread,
                text,
                ..
            } => {
                let outcome = if let Some(why) = self.post_failures.pop_front() {
                    Err(why)
                } else {
                    self.posts.push((thread, text));
                    Ok(())
                };
                self.schedule(self.now, Event::Posted { request, outcome });
            }
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
