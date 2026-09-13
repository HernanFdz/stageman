//! The simulated world: everything the instance is not, in memory, stepped
//! deterministically.
//!
//! It performs effects by changing its own picture of the disk and the
//! runtime and scheduling the events that answer them at virtual instants,
//! and it records every event and effect as a trace a test can compare. The
//! runtime's questions are recognised through the agent crate's own inverse
//! of their rendering, never by matching on strings. A crash is a method:
//! turns, timers and unanswered writes die, containers stay as they are, and
//! the disk is what landed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use stageman_agent::{Answer, Command, Label, StopReason};
use stageman_core::{
    Agent, AgentConfig, Channel, ChannelConfig, Errand, InstanceId, Job, JobId, Key, Kit,
    KitConfig, KitName, NONCE_LEN, Nonce, Progress, Project, ProjectId, Secret, Snapshot, State,
    Thread, Timestamp, Uuid,
};
use stageman_instance::{
    AppEffect, AppEvent, Effect, Event, Instance, Message, Request, RequestId, Response, Run, Seed,
    Target,
};
use stageman_vocabulary::scenario::{Meta, Recorder};
use stageman_vocabulary::{Bytes, EffectId, Environment, Finished, Named as _};

/// Virtual milliseconds.
pub type Now = u64;

/// Where every simulated instance keeps its file.
const INSTANCE_FILE: &str = "/sim/instance.json";

/// Which platform every scenario is played on.
///
/// One platform for all of them, and a value rather than this machine's, so
/// that a flow answers the same wherever it is run and a recording of one
/// replays anywhere. What each platform does about a path is a unit test of
/// the paths themselves, where all of them are reachable.
pub const TARGET: Target = Target::Linux;

/// What says this run is meant to rewrite the replay files it records.
///
/// Off by default, and that is the whole of the arrangement: a committed
/// file is what the replay runner checks a fresh instance against, so
/// rewriting one is a deliberate act whose diff somebody reads. See
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
const RECORD_VARIABLE: &str = "STAGEMAN_RECORD";

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

/// Where a tunnel request was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// Nothing answers on that name.
    Nowhere,
    /// Forwarded to this host port.
    To(u16),
}

pub struct Simulation {
    now: Now,
    seq: u64,
    queue: BTreeMap<(Now, u64), Event>,
    containers: BTreeMap<String, Held>,
    /// The disk: every file by path.
    files: BTreeMap<PathBuf, Vec<u8>>,
    /// Writes asked for and not yet answered, in order; `None` is one that
    /// will fail rather than land.
    landing: VecDeque<Option<(PathBuf, Vec<u8>)>>,
    /// Whether a container runtime is installed.
    has_runtime: bool,
    /// Everything printed to standard output, in order.
    printed: Vec<String>,
    /// The reason the process was told to stop, if it was.
    exited: Option<String>,
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
    listening: Vec<ProjectId>,
    /// Every image this project has built, as the runtime holds them.
    ///
    /// One that nothing needs is what a sweep reclaims, and what it leaves
    /// is what the next container would have rebuilt anyway.
    images: Vec<String>,
    warrants: Vec<String>,
    /// Every runtime command asked for, as the agent crate reads it back and
    /// where in the trace it was asked — so a test names a question rather
    /// than a string, and can still say what came before it.
    commands: Vec<(usize, Command)>,
    key: Key,
    /// What this flow is recorded as, where it is recorded at all: the file
    /// it is written to, and what that file says it pins.
    recording: Option<(String, Meta)>,
    /// The recording itself, once an instance has been constructed for it
    /// to begin from.
    recorder: Option<Recorder<Instance>>,
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
    Event::App(AppEvent::Heard {
        channel: Channel::Slack,
        message: Message {
            address: CHANNEL.to_owned(),
            id: "1788000099.000001".to_owned(),
            thread: Some(self::thread(thread).id),
            text: text.to_owned(),
            mentions: true,
            from_us: false,
        },
    })
}

/// Somebody mentioning this instance at the root of the project's channel.
/// Each is its own message, so each opens its own thread.
pub fn said_at_root(n: u32, text: &str) -> Event {
    Event::App(AppEvent::Heard {
        channel: Channel::Slack,
        message: Message {
            address: CHANNEL.to_owned(),
            id: thread(n).id,
            thread: None,
            text: text.to_owned(),
            mentions: true,
            from_us: false,
        },
    })
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
    Event::App(AppEvent::TunnelAsked {
        id: RequestId(id),
        job,
    })
}

/// A person asking something of the dashboard.
pub const fn request(id: u64, request: Request) -> Event {
    Event::App(AppEvent::Request {
        id: RequestId(id),
        request,
    })
}

/// A call on the tools endpoint from this machine, presenting a credential.
pub fn tool_call(id: u64, bearer: &str, body: serde_json::Value) -> Event {
    Event::App(AppEvent::ToolCalled {
        id: RequestId(id),
        at: Timestamp::UNIX_EPOCH,
        nearby: true,
        bearer: Some(bearer.to_owned()),
        body,
    })
}

pub const fn key() -> Key {
    Key::new([7; 32])
}

pub const fn seed(n: u8) -> Seed {
    [n; 32]
}

impl Simulation {
    pub fn new() -> Self {
        Self {
            now: 0,
            seq: 0,
            queue: BTreeMap::new(),
            containers: BTreeMap::new(),
            files: BTreeMap::new(),
            landing: VecDeque::new(),
            has_runtime: true,
            printed: Vec::new(),
            exited: None,
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
            images: vec!["stageman:unneeded".to_owned()],
            warrants: Vec::new(),
            commands: Vec::new(),
            key: key(),
            recording: None,
            recorder: None,
        }
    }

    /// Puts a state on the disk, sealed as the instance would seal it.
    /// Puts bytes where the instance keeps its file, for the starts that
    /// turn on what is already on the disk rather than on what it says.
    pub fn holding_bytes(&mut self, bytes: &[u8]) {
        self.files
            .insert(PathBuf::from(INSTANCE_FILE), bytes.to_vec());
    }

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
        self.files.insert(
            PathBuf::from(INSTANCE_FILE),
            serde_json::to_vec_pretty(&snapshot).expect("a snapshot encodes"),
        );
    }

    /// Takes the runtime away, so that booting finds no candidate answering.
    pub const fn without_a_runtime(&mut self) {
        self.has_runtime = false;
    }

    /// The environment every simulated instance is constructed with: a home,
    /// its key, and where its file is.
    pub fn environment() -> Environment {
        [
            ("HOME".to_owned(), "/sim/home".to_owned()),
            ("STAGEMAN_KEY".to_owned(), key().to_base64()),
            ("STAGEMAN_STATE".to_owned(), INSTANCE_FILE.to_owned()),
        ]
        .into()
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

    /// Everything printed to standard output so far.
    /// Every runtime command asked for, in order.
    pub fn commands(&self) -> Vec<Command> {
        self.commands
            .iter()
            .map(|(_, command)| command.clone())
            .collect()
    }

    /// Every runtime command asked after a point in the trace.
    ///
    /// Booting asks several of the same questions an awake instance does, so
    /// a test about what waking leads to says where waking was.
    pub fn commands_after(&self, at: usize) -> Vec<Command> {
        self.commands
            .iter()
            .filter(|(asked, _)| *asked > at)
            .map(|(_, command)| command.clone())
            .collect()
    }

    /// Where in the trace the runtime was first asked something of a kind.
    pub fn first_asking(&self, wanted: impl Fn(&Command) -> bool) -> Option<usize> {
        self.commands
            .iter()
            .find(|(_, command)| wanted(command))
            .map(|(at, _)| *at)
    }

    pub fn printed(&self) -> &[String] {
        &self.printed
    }

    /// Why the process was told to stop, if it was.
    pub fn exited(&self) -> Option<&str> {
        self.exited.as_deref()
    }

    /// Scripts how the next turn ends.
    pub fn next_turn_ends(&mut self, outcome: Result<Answer, String>) {
        self.answers.push_back(outcome);
    }

    /// Boots an instance and answers everything it asks on the way to being
    /// awake, which is everything scheduled for this instant.
    pub fn wake(&mut self, seed: Seed) -> Instance {
        self.wake_given(seed, Self::environment())
    }

    /// Boots with an environment of the caller's own, for the starts that
    /// turn on what a variable says.
    pub fn wake_given(&mut self, seed: Seed, environment: Environment) -> Instance {
        let (mut instance, effects) = Instance::boot(seed, environment.clone(), TARGET);
        self.trace.push(format!("{}: booted", self.now));
        if let Some((_, meta)) = &self.recording {
            assert!(
                self.recorder.is_none(),
                "a replay is one instance's life, so a flow that wakes twice needs a file each"
            );
            self.recorder = Some(Recorder::started(
                meta.clone(),
                seed,
                environment,
                TARGET,
                effects.clone(),
                instance.snapshot(),
            ));
        }
        for effect in effects {
            self.perform(effect);
        }
        self.stepped(
            &mut instance,
            AppEvent::Serving {
                address: "127.0.0.1:8080".to_owned(),
                port: 8080,
            }
            .into(),
        );
        let now = self.now;
        self.run_until(&mut instance, now);
        instance
    }

    /// Kills the daemon and starts it again.
    pub fn crash(&mut self, seed: Seed) -> Instance {
        self.queue.retain(|_, event| {
            !matches!(
                event,
                Event::Read { .. }
                    | Event::Written { .. }
                    | Event::Ran { .. }
                    | Event::Woke { .. }
                    | Event::App(
                        AppEvent::Serving { .. }
                            | AppEvent::TurnEnded { .. }
                            | AppEvent::Probed { .. }
                            | AppEvent::ThreadOpened { .. }
                            | AppEvent::Posted { .. }
                            | AppEvent::Request { .. }
                            | AppEvent::TunnelAsked { .. }
                            | AppEvent::TunnelFailed { .. }
                    )
            )
        });
        self.landing.clear();
        self.begun.clear();
        self.trace.push(format!("{}: CRASH", self.now));
        self.wake(seed)
    }

    pub fn schedule(&mut self, at: Now, event: impl Into<Event>) {
        self.seq += 1;
        self.queue.insert((at, self.seq), event.into());
    }

    /// The next event, if there is one. A write lands as its completion is
    /// delivered, never before.
    pub fn next(&mut self) -> Option<Event> {
        let ((at, _), event) = self.queue.pop_first()?;
        self.now = at;
        if matches!(event, Event::Written { .. })
            && let Some(Some((path, bytes))) = self.landing.pop_front()
        {
            self.files.insert(path, bytes);
        }
        self.trace
            .push(format!("{at}: <- {}{}", event.kind(), serialised(&event)));
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
            self.stepped(instance, event);
            self.oracle(instance);
        }
    }

    /// Steps the instance once: what it asks for is performed, and the turn
    /// is written down where a recording is being made.
    fn stepped(&mut self, instance: &mut Instance, event: Event) {
        let effects = instance.step(event.clone());
        if let Some(recorder) = &mut self.recorder {
            recorder.turned(self.now, event, effects.clone(), instance.snapshot());
        }
        for effect in effects {
            self.perform(effect);
        }
    }

    /// Records this flow as a replay file under the name given.
    ///
    /// Asked for per flow rather than always, because a recording costs a
    /// snapshot of everything the instance holds on every step, and because
    /// which flows are pinned as files is a decision worth reading in the
    /// test that makes it.
    pub fn recording(&mut self, name: &str, title: &str) {
        self.recording = Some((
            name.to_owned(),
            Meta {
                title: title.to_owned(),
                description: String::new(),
            },
        ));
    }

    /// Writes what was recorded, when this run was asked to rewrite files.
    ///
    /// Otherwise nothing at all: what checks the committed file is the
    /// replay runner, which drives it against a fresh instance, and a test
    /// that quietly rewrote its own expectation would check nothing.
    pub fn recorded(&mut self) {
        let (Some((name, _)), Some(recorder)) = (self.recording.take(), self.recorder.take())
        else {
            panic!("nothing was recorded: `recording` is asked for before an instance wakes");
        };
        if std::env::var_os(RECORD_VARIABLE).is_none() {
            return;
        }
        let directory = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/replays"));
        std::fs::create_dir_all(directory).expect("the replays directory");
        let mut written =
            serde_json::to_string_pretty(&recorder.finished()).expect("a scenario serialises");
        written.push('\n');
        std::fs::write(directory.join(format!("{name}.json")), written).expect("the replay lands");
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
        self.schedule(at, AppEvent::TurnEnded { speaker, outcome });
    }

    /// Accepts a write: it lands as its completion is delivered, unless it
    /// was scripted to fail.
    fn write(&mut self, id: EffectId, path: PathBuf, bytes: Vec<u8>) {
        let outcome = if let Some(why) = self.write_failures.pop_front() {
            self.landing.push_back(None);
            Err(why)
        } else {
            self.landing.push_back(Some((path, bytes)));
            Ok(())
        };
        self.schedule(self.now + 1, Event::Written { id, outcome });
    }

    /// Runs a program: the runtime's own questions are answered from what the
    /// simulation holds, recognised through the agent crate's own inverse
    /// rather than by matching on strings.
    fn ran(&mut self, id: EffectId, program: &Path, arguments: &[String]) {
        let exited = |stdout: String| Finished::Exited {
            status: Some(0),
            stdout: stdout.into(),
            stderr: Bytes::new(Vec::new()),
        };
        let finished = if self.has_runtime {
            match Command::parse(arguments) {
                Some(Command::Version) => {
                    exited(format!("{} version 0.0-simulated\n", program.display()))
                }
                Some(Command::Containers { running_only }) => exited(
                    self.containers
                        .iter()
                        .filter(|(_, held)| held.running || !running_only)
                        .fold(String::new(), |mut listed, (name, _)| {
                            listed.push_str(name);
                            listed.push('\n');
                            listed
                        }),
                ),
                Some(Command::Label { name, label }) => self.containers.get(&name).map_or_else(
                    || Finished::Exited {
                        status: Some(1),
                        stdout: Bytes::new(Vec::new()),
                        stderr: format!("Error: No such object: {name}\n").into(),
                    },
                    |held| {
                        exited(match label {
                            Label::Instance => held
                                .instance
                                .map_or_else(String::new, |instance| format!("{instance}\n")),
                            Label::Agent => held
                                .agent
                                .map_or_else(String::new, |_| "claude\n".to_owned()),
                        })
                    },
                ),
                Some(Command::Halt { name }) => {
                    if let Some(held) = self.containers.get_mut(&name) {
                        held.running = false;
                    }
                    exited(String::new())
                }
                Some(Command::Discard { name }) => {
                    self.containers.remove(&name);
                    exited(String::new())
                }
                // As the runtime prints it: the host side of the mapping,
                // one line, and nothing at all when there is no mapping.
                Some(Command::Images) => {
                    exited(self.images.iter().fold(String::new(), |mut listed, image| {
                        listed.push_str(image);
                        listed.push('\n');
                        listed
                    }))
                }
                Some(Command::RemoveImage { image }) => {
                    self.images.retain(|held| *held != image);
                    exited(String::new())
                }
                Some(Command::Port { name }) => self.port_of(&name).map_or_else(
                    || exited(String::new()),
                    |port| exited(format!("127.0.0.1:{port}\n")),
                ),
                None => Finished::Failed("the simulation does not know this command".to_owned()),
            }
        } else {
            Finished::NotFound
        };
        self.schedule(self.now, Event::Ran { id, finished });
    }

    /// Ends the agent process, so the turn it was running ends with that
    /// rather than with whatever the agent would have said.
    fn stop_turn(&mut self, speaker: stageman_instance::Speaker) {
        self.queue.retain(|_, event| {
            !matches!(event, Event::App(AppEvent::TurnEnded { speaker: whose, .. }) if *whose == speaker)
        });
        self.schedule(
            self.now,
            AppEvent::TurnEnded {
                speaker,
                outcome: Err("stopped".to_owned()),
            },
        );
    }

    pub fn perform(&mut self, effect: Effect) {
        // Before the line is pushed, so the position recorded is its own.
        if let Effect::Run { arguments, .. } = &effect
            && let Some(command) = Command::parse(arguments)
        {
            self.commands.push((self.trace.len(), command));
        }
        self.trace.push(format!(
            "{}: -> {}{}",
            self.now,
            effect.kind(),
            serialised(&effect)
        ));
        let Some(effect) = self.generic(effect) else {
            return;
        };
        match effect {
            AppEffect::Booted { .. } => {}
            AppEffect::RunTurn { speaker, run } => self.run_turn(speaker, &run),
            AppEffect::Probe { job } => {
                let answering = self
                    .containers
                    .get(&stageman_job::container(job))
                    .is_some_and(|held| held.running && held.serving);
                self.schedule(self.now, AppEvent::Probed { job, answering });
            }
            AppEffect::Say { thread, text, .. } => self.posts.push((thread, text)),
            AppEffect::ToolAnswered { id, status, body } => {
                self.tool_answers.insert(id, (status, body));
            }
            AppEffect::Respond { id, response } => {
                self.responses.insert(id, response);
            }
            AppEffect::Route { id, port } => {
                self.routes.insert(id, port.map_or(Sent::Nowhere, Sent::To));
            }
            AppEffect::StopTurn { speaker } => self.stop_turn(speaker),
            AppEffect::OpenThread {
                job, announcement, ..
            } => {
                self.threads_opened += 1;
                let opened = thread(self.threads_opened);
                self.posts.push((opened.clone(), announcement));
                self.schedule(
                    self.now,
                    AppEvent::ThreadOpened {
                        job,
                        outcome: Ok(opened),
                    },
                );
            }
            AppEffect::Post {
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
                self.schedule(self.now, AppEvent::Posted { request, outcome });
            }
            AppEffect::Listen { project, .. } => {
                self.listening.push(project);
                self.trace.push(format!(
                    "{}: listening on {} project(s)",
                    self.now,
                    self.listening.len()
                ));
            }
        }
    }

    /// Performs a generic effect, and hands back an application one.
    fn generic(&mut self, effect: Effect) -> Option<AppEffect> {
        match effect {
            Effect::Read { id, path } => {
                let contents = Ok(self.files.get(&path).cloned().map(Bytes::new));
                self.schedule(self.now, Event::Read { id, contents });
            }
            Effect::Write {
                id, path, bytes, ..
            } => self.write(id, path, bytes.into_inner()),
            Effect::Run {
                id,
                program,
                arguments,
                ..
            } => self.ran(id, &program, &arguments),
            Effect::Wake { id, after } => {
                let at = self.now + u64::try_from(after.as_millis()).expect("a short wait");
                self.schedule(at, Event::Woke { id });
            }
            Effect::Print { text } => self.printed.push(text),
            Effect::Exit { message } => self.exited = Some(message),
            Effect::App(effect) => return Some(effect),
        }
        None
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

    pub fn reclaims(&self) -> usize {
        self.commands
            .iter()
            .filter(|(_, command)| matches!(command, Command::Images))
            .count()
    }

    /// What is on the disk, opened.
    pub fn disk(&self) -> Option<State> {
        let bytes = self.files.get(Path::new(INSTANCE_FILE))?;
        let snapshot: Snapshot = serde_json::from_slice(bytes).expect("the disk holds JSON");
        Some(snapshot.open(&self.key).expect("and it opens"))
    }

    /// The credentials handed to turns so far, oldest first.
    pub fn warrants(&self) -> &[String] {
        &self.warrants
    }
}

/// A value's fields as one line of JSON after a space, which is what a trace
/// holds beside the kind: the hole and the variant are unwrapped, and a
/// variant with no fields is nothing at all.
fn serialised(value: &(impl serde::Serialize + stageman_vocabulary::Named)) -> String {
    let kind = value.kind();
    let mut value = serde_json::to_value(value).expect("everything in the vocabulary serialises");
    for wrapper in ["App", kind] {
        value = match value {
            serde_json::Value::Object(mut wrapped)
                if wrapped.len() == 1 && wrapped.contains_key(wrapper) =>
            {
                wrapped.remove(wrapper).expect("just checked")
            }
            other => other,
        };
    }
    match value {
        serde_json::Value::String(_) | serde_json::Value::Null => String::new(),
        fields => format!(
            " {}",
            serde_json::to_string(&fields).expect("a value serialises")
        ),
    }
}

impl Default for Simulation {
    fn default() -> Self {
        Self::new()
    }
}
