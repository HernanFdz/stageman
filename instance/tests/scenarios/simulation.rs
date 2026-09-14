//! The simulated world: everything the instance is not, in memory, stepped
//! deterministically.
//!
//! It performs effects by changing its own picture of the disk and the
//! runtime and scheduling the events that answer them at virtual instants,
//! and it records every event and effect as a trace a test can compare. The
//! runtime's questions are recognised through the agent crate's own inverse
//! of their rendering, never by matching on strings, and so are the lines
//! the instance says to an agent: a process kept open here is a simulated
//! adapter that reads each line back as what it asks and answers as the
//! pinned adapter was measured to. A crash is a method: turns, timers and
//! unanswered writes die, containers stay as they are, and the disk is what
//! landed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use stageman_agent::{Answer, Command, Heard, Label, Said, StopReason};
use stageman_core::{
    Agent, AgentConfig, Channel, ChannelConfig, Errand, InstanceId, Job, JobId, Key, Kit,
    KitConfig, KitName, NONCE_LEN, Nonce, Progress, Project, ProjectId, Secret, Snapshot, State,
    Thread, Timestamp, Uuid,
};
use stageman_instance::{
    AppEffect, AppEvent, Effect, Event, Instance, Message, Request, RequestId, Response, Seed,
    Target,
};
use stageman_vocabulary::scenario::{Meta, Recorder};
use stageman_vocabulary::{
    Answer as Answering, Arrival, Bytes, EffectId, Ended, Environment, Finished, Named as _,
    Probed, RequestId as Asked,
};

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
    /// The session its agent has written, if one has said anything: what a
    /// resumed conversation finds, and what a fresh one leaves.
    pub session: Option<String>,
    /// The variables it was made with, valued: what the runtime was given
    /// and told to forward.
    pub environment: BTreeMap<String, String>,
}

/// One agent process the simulation keeps open: the adapter's half of a
/// conversation, answering what it is sent.
struct Adapter {
    /// Which container it runs in.
    container: String,
    /// The session it serves, once one is made or loaded.
    session: Option<String>,
    /// What the session currently reports its options to be.
    options: Vec<(String, String)>,
    /// How the prompt ends: the next scripted outcome, or a clean ending
    /// having said "done".
    outcome: Result<Answer, String>,
    /// Which conversation this is, in the record of them.
    talk: usize,
}

/// One conversation the instance opened, as the simulation saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Talk {
    /// Which container the agent was run in.
    pub container: String,
    /// Whether a session was made rather than the container's own loaded,
    /// once the conversation got that far.
    pub fresh: Option<bool>,
    /// What the agent was asked, once it was.
    pub prompt: Option<String>,
    /// The credential the tools were declared with, once they were.
    pub warrant: Option<String>,
    /// Where in the trace the process was opened.
    pub opened_at: usize,
    /// Where in the trace its end was delivered, once it was.
    pub ended_at: Option<usize>,
}

impl Talk {
    /// Whether this conversation began a session, which is known once it
    /// got as far as one.
    pub fn began(&self) -> bool {
        self.fresh == Some(true)
    }

    /// Whether this conversation resumed a session.
    pub fn resumed(&self) -> bool {
        self.fresh == Some(false)
    }

    /// Whether the agent was told something containing the words.
    pub fn was_told(&self, words: &str) -> bool {
        self.prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains(words))
    }
}

/// What a session advertises before anything is set: the agent's defaults.
fn defaults() -> Vec<(String, String)> {
    ["mode", "model", "effort"]
        .into_iter()
        .map(|option| (option.to_owned(), "default".to_owned()))
        .collect()
}

/// Options as the constructors take them.
fn pairs(options: &[(String, String)]) -> Vec<(&str, &str)> {
    options
        .iter()
        .map(|(option, current)| (option.as_str(), current.as_str()))
        .collect()
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
    /// Why the next addresses cannot be taken, front first.
    bind_failures: VecDeque<String>,
    /// How the next turns end, front first; a turn with nothing scripted ends
    /// cleanly having said "done".
    answers: VecDeque<Result<Answer, String>>,
    /// Why the next builds fail, front first.
    build_failures: VecDeque<String>,
    /// Why the next checkouts fail, front first.
    checkout_failures: VecDeque<String>,
    /// The agent processes kept open, by the identifier each was opened
    /// under, for as long as each runs.
    adapters: BTreeMap<EffectId, Adapter>,
    /// Every conversation opened, in order, and which process each was.
    talks: Vec<Talk>,
    talking: BTreeMap<EffectId, usize>,
    /// How many sessions have been made, so each is named apart.
    sessions_made: u32,
    /// Why the next posts on an agent's behalf fail, front first.
    post_failures: VecDeque<String>,
    /// What each request was answered with, by identifier.
    tool_answers: BTreeMap<Asked, (u16, Option<serde_json::Value>)>,
    /// What each person's request was answered with, by identifier.
    responses: BTreeMap<RequestId, Response>,
    /// Where each tunnel request was sent, by identifier.
    routes: BTreeMap<Asked, Sent>,
    /// The last host port handed out.
    ports: u16,
    /// Threads opened so far, so each gets a number of its own.
    threads_opened: u32,
    /// Jobs whose container the world has been asked to make. A job is on
    /// the record before its container exists, so only after this may the
    /// oracle expect the container.
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
    /// The addresses taken, by the bind that asked for each.
    listeners: BTreeMap<EffectId, String>,
    /// What each request said, until whoever is deciding asks to read it.
    bodies: BTreeMap<Asked, Vec<u8>>,
    /// The next request identifier, minted here because the world holds a
    /// request open.
    asking_next: u64,
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

/// The name a job's tunnel answers on, under the domain every scenario uses.
pub fn tunnel_host(job: JobId) -> String {
    format!("{}.localhost", job.as_uuid())
}

/// A person asking something of the dashboard.
pub const fn request(id: u64, request: Request) -> Event {
    Event::App(AppEvent::Request {
        id: RequestId(id),
        request,
    })
}

/// A call on the tools endpoint from this machine, presenting a credential.
/// Where a caller of the tools is, as the world reports a peer.
pub const NEARBY: &str = "192.168.65.1:52104";

/// Somewhere beyond this machine, which is nowhere a container is.
pub const FARAWAY: &str = "203.0.113.7:52104";

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
            bind_failures: VecDeque::new(),
            answers: VecDeque::new(),
            build_failures: VecDeque::new(),
            checkout_failures: VecDeque::new(),
            adapters: BTreeMap::new(),
            talks: Vec::new(),
            talking: BTreeMap::new(),
            sessions_made: 0,
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
            listeners: BTreeMap::new(),
            bodies: BTreeMap::new(),
            asking_next: 1,
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
    ///
    /// One that is up has its tunnel published on a host port, as the
    /// runtime would have it, unless the caller said which.
    pub fn container(&mut self, name: &str, mut held: Held) {
        if held.running && held.port.is_none() {
            self.ports += 1;
            held.port = Some(self.ports);
        }
        self.containers.insert(name.to_owned(), held);
    }

    /// A container of this instance's, stopped and showing nothing, whose
    /// agent has written a session: what a job or a foreman leaves behind.
    pub fn ours(name: &str) -> (String, Held) {
        (
            name.to_owned(),
            Held {
                instance: Some(this_instance()),
                agent: Some(Agent::Claude),
                running: false,
                serving: false,
                port: None,
                session: Some(format!("sess-{name}")),
                environment: BTreeMap::new(),
            },
        )
    }

    /// A container up right now, of this instance's, showing something or
    /// not, with a session in it.
    pub fn up(serving: bool) -> Held {
        Held {
            instance: Some(this_instance()),
            agent: Some(Agent::Claude),
            running: true,
            serving,
            port: None,
            session: Some("sess-left".to_owned()),
            environment: BTreeMap::new(),
        }
    }

    /// A container of somebody else's, up: another instance's, or one made
    /// before instances were told apart.
    pub fn theirs(instance: Option<InstanceId>, running: bool) -> Held {
        Held {
            instance,
            agent: instance.map(|_| Agent::Claude),
            running,
            serving: false,
            port: None,
            session: None,
            environment: BTreeMap::new(),
        }
    }

    /// Scripts the next write to fail.
    /// The next address asked for cannot be taken, and says why.
    ///
    /// The first one asked for is the dashboard's, which is the one whose
    /// refusal stops a start.
    pub fn next_bind_fails(&mut self, why: &str) {
        self.bind_failures.push_back(why.to_owned());
    }

    pub fn next_write_fails(&mut self, why: &str) {
        self.write_failures.push_back(why.to_owned());
    }

    /// Scripts the next post on an agent's behalf to fail.
    pub fn next_post_fails(&mut self, why: &str) {
        self.post_failures.push_back(why.to_owned());
    }

    /// Scripts the next build to fail, saying why.
    pub fn next_build_fails(&mut self, why: &str) {
        self.build_failures.push_back(why.to_owned());
    }

    /// Scripts the next checkout to fail, saying why.
    pub fn next_checkout_fails(&mut self, why: &str) {
        self.checkout_failures.push_back(why.to_owned());
    }

    /// Every conversation opened so far, in order.
    pub fn talks(&self) -> &[Talk] {
        &self.talks
    }

    /// The conversations opened in one container, in order.
    pub fn talks_in(&self, container: &str) -> Vec<&Talk> {
        self.talks
            .iter()
            .filter(|talk| talk.container == container)
            .collect()
    }

    /// The variables a container was made with, valued, if it was made
    /// here.
    pub fn environment_of(&self, name: &str) -> Option<&BTreeMap<String, String>> {
        self.containers.get(name).map(|held| &held.environment)
    }

    /// Where in the trace a turn was first asked for: the first command a
    /// turn runs, which is whether its image is there for one that begins
    /// and its container starting for one that resumes.
    pub fn first_turn(&self) -> Option<usize> {
        self.first_asking(|command| {
            matches!(command, Command::Present { .. } | Command::Start { .. })
        })
    }

    /// What a request was answered with, if it has been.
    pub fn tool_answer(&self, id: Asked) -> Option<&(u16, Option<serde_json::Value>)> {
        self.tool_answers.get(&id)
    }

    /// What a person's request was answered with, if it has been.
    pub fn response(&self, id: u64) -> Option<&Response> {
        self.responses.get(&RequestId(id))
    }

    /// Where a tunnel request was sent, if it has been answered.
    pub fn route(&self, id: Asked) -> Option<Sent> {
        self.routes.get(&id).copied()
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
    /// What to do about a request being held open.
    fn answered(&mut self, id: Asked, answer: Answering) {
        match answer {
            Answering::Respond { status, body, .. } => {
                // A refusal to whoever visited a name, or an answer to a
                // call: which it is, is which listener it arrived on, and a
                // test knows because it said.
                if status == 404 && !body.is_empty() {
                    self.routes.insert(id, Sent::Nowhere);
                }
                let body = serde_json::from_slice(body.as_slice()).ok();
                self.tool_answers.insert(id, (status, body));
            }
            Answering::Read { limit } => {
                let said = self.bodies.remove(&id).unwrap_or_default();
                // The limit is honoured here as a real listener honours it:
                // what is longer is not half-read, it is not read.
                let outcome = if said.len() > limit {
                    Err("the body is longer than the limit it was read under".to_owned())
                } else {
                    Ok(Bytes::new(said))
                };
                self.schedule(self.now, Event::Body { id, outcome });
            }
            // Nothing is forwarded yet: the tunnel is the next family.
            Answering::Proxy { port, .. } => {
                self.routes.insert(id, Sent::To(port));
            }
        }
    }

    /// Which listener is which, by the address it was taken on: the tools
    /// are served on every interface and the dashboard is not.
    fn listener(&self, tools: bool) -> EffectId {
        self.listeners
            .iter()
            .find(|(_, address)| address.starts_with("0.0.0.0:") == tools)
            .map(|(id, _)| *id)
            .expect("an address was taken before anything arrived on it")
    }

    /// Says a request arrived on the tools' listener, and holds its body for
    /// whenever it is asked for.
    pub fn arrives(
        &mut self,
        at: Now,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        peer: &str,
        body: &str,
    ) -> Asked {
        self.arriving(self.listener(true), at, method, path, headers, peer, body)
    }

    /// Says a request arrived on one of them.
    #[expect(
        clippy::too_many_arguments,
        reason = "a request is a method, a path, headers, a peer and a body, and a test that \
                  named them in a struct would read worse at every call"
    )]
    fn arriving(
        &mut self,
        listener: EffectId,
        at: Now,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        peer: &str,
        body: &str,
    ) -> Asked {
        let id = Asked(self.asking_next);
        self.asking_next += 1;
        self.bodies.insert(id, body.as_bytes().to_vec());
        self.schedule(
            at,
            Event::Arrived {
                listener,
                id,
                request: Arrival {
                    method: method.to_owned(),
                    path: path.to_owned(),
                    headers: headers
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                        .collect(),
                    peer: peer.to_owned(),
                    at: 1_757_000_000_000,
                },
            },
        );
        id
    }

    /// Says somebody visited a name this instance answers on.
    pub fn visits(&mut self, at: Now, host: &str) -> Asked {
        self.arriving(
            self.listener(false),
            at,
            "GET",
            "/",
            &[("host", host)],
            NEARBY,
            "",
        )
    }

    /// Says a tool call arrived, as an agent's client would make one.
    pub fn calls(&mut self, at: Now, bearer: &str, body: &serde_json::Value) -> Asked {
        self.arrives(
            at,
            "POST",
            "/mcp",
            &[("authorization", &format!("Bearer {bearer}"))],
            NEARBY,
            &serde_json::to_string(body).expect("a call is JSON"),
        )
    }

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

    /// Scripts how the next turn ends: what its agent says and reports and
    /// why it stops, or why its process dies instead of answering.
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
        self.stepped(&mut instance, AppEvent::Presenting { port: 9000 }.into());
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
                    | Event::Line { .. }
                    | Event::Ended { .. }
                    | Event::Probed { .. }
                    | Event::App(
                        AppEvent::Presenting { .. }
                            | AppEvent::ThreadOpened { .. }
                            | AppEvent::Posted { .. }
                            | AppEvent::Request { .. }
                    )
            )
        });
        self.landing.clear();
        self.begun.clear();
        // The agent processes die with the daemon that held their pipes;
        // what they wrote into their containers stays.
        self.adapters.clear();
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
        if let Event::Ended { id, .. } = &event
            && let Some(&talk) = self.talking.get(id)
            && let Some(talk) = self.talks.get_mut(talk)
        {
            talk.ended_at = Some(self.trace.len());
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

    /// Keeps an agent process open: the adapter's half of a conversation,
    /// in a container that has to be up.
    fn opened(&mut self, id: EffectId, arguments: &[String]) {
        let Some(Command::Exec { name }) = Command::parse(arguments) else {
            self.schedule(
                self.now,
                Event::Ended {
                    id,
                    ended: Ended::Failed("the simulation keeps no such process open".to_owned()),
                },
            );
            return;
        };
        if !self.is_running(&name) {
            self.schedule(
                self.now,
                Event::Ended {
                    id,
                    ended: Ended::Exited {
                        status: Some(1),
                        stderr: format!(
                            "Error response from daemon: container {name} is not running\n"
                        )
                        .into(),
                    },
                },
            );
            return;
        }
        let outcome = self.answers.pop_front().unwrap_or_else(|| {
            Ok(Answer {
                text: "done".to_owned(),
                stop_reason: StopReason::EndTurn,
                reported: BTreeMap::new(),
            })
        });
        self.talks.push(Talk {
            container: name.clone(),
            fresh: None,
            prompt: None,
            warrant: None,
            opened_at: self.trace.len(),
            ended_at: None,
        });
        let talk = self.talks.len() - 1;
        self.talking.insert(id, talk);
        self.adapters.insert(
            id,
            Adapter {
                container: name,
                session: None,
                options: Vec::new(),
                outcome,
                talk,
            },
        );
    }

    /// One line said to an agent process, answered as the pinned adapter
    /// was measured to answer: read back as what it asks, never matched as
    /// a string.
    fn sent(&mut self, id: EffectId, line: &str) {
        let Some(said) = Said::parse(line) else {
            return;
        };
        let presented = said.presented();
        let Some(adapter) = self.adapters.get_mut(&id) else {
            // A process that has ended is not written to.
            return;
        };
        let talk = adapter.talk;
        let reply_at = self.now + 1;
        match said {
            Said::Initialize { id: request } => self.schedule(
                reply_at,
                Event::Line {
                    id,
                    line: Heard::initialized(request).line(),
                },
            ),
            Said::NewSession { id: request, .. } => {
                self.session_opened(id, talk, request, None, presented);
            }
            Said::ListSessions { id: request } => {
                let held = self
                    .containers
                    .get(&adapter.container)
                    .and_then(|held| held.session.clone());
                let known: Vec<&str> = held.iter().map(String::as_str).collect();
                self.schedule(
                    reply_at,
                    Event::Line {
                        id,
                        line: Heard::sessions(request, &known).line(),
                    },
                );
            }
            Said::LoadSession {
                id: request,
                session,
                ..
            } => self.session_opened(id, talk, request, Some(session), presented),
            Said::SetOption {
                id: request,
                option,
                value,
                ..
            } => {
                // Taken, and reported as what the script says the adapter
                // reports where it says, else as asked.
                let reported = adapter
                    .outcome
                    .as_ref()
                    .ok()
                    .and_then(|answer| answer.reported.get(&option).cloned())
                    .unwrap_or(value);
                match adapter
                    .options
                    .iter_mut()
                    .find(|(named, _)| *named == option)
                {
                    Some((_, current)) => *current = reported,
                    None => adapter.options.push((option, reported)),
                }
                let line = Heard::set(request, &pairs(&adapter.options)).line();
                self.schedule(reply_at, Event::Line { id, line });
            }
            Said::Prompt {
                id: request, text, ..
            } => self.prompted(id, talk, request, text),
            Said::Permitted { .. } | Said::Unserved { .. } => {}
        }
    }

    /// A session made or loaded: made under a fresh name and written into
    /// the container, or loaded as the one named; either way advertising
    /// the agent's defaults, as measured — a loaded session forgets what it
    /// was set to.
    fn session_opened(
        &mut self,
        id: EffectId,
        talk: usize,
        request: i64,
        loading: Option<String>,
        presented: Option<String>,
    ) {
        let Some(adapter) = self.adapters.get_mut(&id) else {
            return;
        };
        let fresh = loading.is_none();
        let session = loading.unwrap_or_else(|| {
            self.sessions_made += 1;
            format!("sess-{}", self.sessions_made)
        });
        if fresh && let Some(held) = self.containers.get_mut(&adapter.container) {
            held.session = Some(session.clone());
        }
        adapter.session = Some(session.clone());
        adapter.options = defaults();
        let advertised = pairs(&adapter.options);
        let line = if fresh {
            Heard::session_made(request, &session, &advertised).line()
        } else {
            Heard::loaded(request, &advertised).line()
        };
        if let Some(talk) = self.talks.get_mut(talk) {
            talk.fresh = Some(fresh);
            talk.warrant = presented;
        }
        self.schedule(self.now + 1, Event::Line { id, line });
    }

    /// The question put: answered as scripted after the turn's length, or
    /// the process dies instead of answering.
    fn prompted(&mut self, id: EffectId, talk: usize, request: i64, text: String) {
        let Some(adapter) = self.adapters.get(&id) else {
            return;
        };
        if let Some(talk) = self.talks.get_mut(talk) {
            talk.prompt = Some(text);
        }
        let session = adapter.session.clone().unwrap_or_default();
        let at = self.now + self.turn_takes;
        match adapter.outcome.clone() {
            Ok(answer) => {
                self.schedule(
                    at,
                    Event::Line {
                        id,
                        line: Heard::said(&session, &answer.text).line(),
                    },
                );
                self.schedule(
                    at,
                    Event::Line {
                        id,
                        line: Heard::prompted(request, answer.stop_reason).line(),
                    },
                );
            }
            Err(why) => {
                // The agent dies instead of answering, and its process ends
                // with the complaint.
                self.adapters.remove(&id);
                self.schedule(
                    at,
                    Event::Ended {
                        id,
                        ended: Ended::Exited {
                            status: Some(1),
                            stderr: format!("{why}\n").into(),
                        },
                    },
                );
            }
        }
    }

    /// Makes a container: from an image that is there, under a name nothing
    /// holds, with the variables named taken from the environment the
    /// runtime was given — which is what forwarding means.
    fn made(
        &mut self,
        name: &str,
        image: &str,
        agent: Agent,
        instance: InstanceId,
        variables: &[String],
        environment: &Environment,
    ) -> Finished {
        let exited = |stdout: String| Finished::Exited {
            status: Some(0),
            stdout: stdout.into(),
            stderr: Bytes::new(Vec::new()),
        };
        let refused = |stderr: String| Finished::Exited {
            status: Some(1),
            stdout: Bytes::new(Vec::new()),
            stderr: stderr.into(),
        };
        if !self.images.iter().any(|held| held == image) {
            refused(format!("Unable to find image '{image}' locally\n"))
        } else if self.containers.contains_key(name) {
            refused(format!(
                "Error response from daemon: Conflict. The container name \"/{name}\" is already in use\n"
            ))
        } else {
            let given = variables
                .iter()
                .filter_map(|variable| {
                    environment
                        .get(variable)
                        .map(|value| (variable.clone(), value.clone()))
                })
                .collect();
            self.containers.insert(
                name.to_owned(),
                Held {
                    instance: Some(instance),
                    agent: Some(agent),
                    running: false,
                    serving: false,
                    port: None,
                    session: None,
                    environment: given,
                },
            );
            if let Some(job) = stageman_job::job_of(name) {
                self.begun.insert(job);
            }
            exited("0123456789abcdef\n".to_owned())
        }
    }

    /// The commands a turn runs, answered from what the simulation holds.
    ///
    /// Split from [`Simulation::ran`] by the line budget and nothing else.
    fn ran_for_a_turn(
        &mut self,
        command: Command,
        environment: &Environment,
        stdin: Option<&Bytes>,
    ) -> Finished {
        let exited = |stdout: String| Finished::Exited {
            status: Some(0),
            stdout: stdout.into(),
            stderr: Bytes::new(Vec::new()),
        };
        let refused = |stderr: String| Finished::Exited {
            status: Some(1),
            stdout: Bytes::new(Vec::new()),
            stderr: stderr.into(),
        };
        match Some(command) {
            Some(Command::Present { image }) => {
                if self.images.contains(&image) {
                    exited("sha256:simulated\n".to_owned())
                } else {
                    refused(format!(
                        "Error response from daemon: No such image: {image}\n"
                    ))
                }
            }
            // A build reads its recipe from standard input, and one given
            // none has nothing to build.
            Some(Command::Build { image }) => {
                if stdin.is_none_or(Bytes::is_empty) {
                    refused("a build was given no recipe\n".to_owned())
                } else if let Some(why) = self.build_failures.pop_front() {
                    refused(why)
                } else {
                    if !self.images.contains(&image) {
                        self.images.push(image);
                    }
                    exited("sha256:simulated\n".to_owned())
                }
            }
            // Made from an image that is there, under a name nothing
            // holds, with the variables named taken from the environment
            // the runtime was given — which is what forwarding means.
            Some(Command::Create {
                name,
                image,
                agent,
                instance,
                variables,
            }) => self.made(&name, &image, agent, instance, &variables, environment),
            // A start publishes the tunnel on a fresh host port, as the
            // runtime does, whether the container is new or restarted.
            Some(Command::Start { name }) => {
                self.ports += 1;
                let port = self.ports;
                match self.containers.get_mut(&name) {
                    Some(held) => {
                        held.running = true;
                        held.port = Some(port);
                        exited(format!("{name}\n"))
                    }
                    None => refused(format!(
                        "Error response from daemon: No such container: {name}\n"
                    )),
                }
            }
            Some(Command::Checkout { name, .. }) => {
                if !self.is_running(&name) {
                    refused(format!(
                        "Error response from daemon: container {name} is not running\n"
                    ))
                } else if let Some(why) = self.checkout_failures.pop_front() {
                    refused(why)
                } else {
                    exited(String::new())
                }
            }
            Some(Command::Exec { .. }) => {
                Finished::Failed("the agent is run kept open, not once".to_owned())
            }
            None
            | Some(
                Command::Version
                | Command::Containers { .. }
                | Command::Label { .. }
                | Command::Halt { .. }
                | Command::Discard { .. }
                | Command::Port { .. }
                | Command::Images
                | Command::RemoveImage { .. },
            ) => Finished::Failed("not a turn's command".to_owned()),
        }
    }

    /// Ends an agent process: whatever it was about to say is never said,
    /// and its end arrives at once, killed.
    fn closed(&mut self, id: EffectId) {
        if self.adapters.remove(&id).is_none() {
            return;
        }
        self.queue
            .retain(|_, event| !matches!(event, Event::Line { id: whose, .. } if *whose == id));
        self.schedule(
            self.now,
            Event::Ended {
                id,
                ended: Ended::Exited {
                    status: None,
                    stderr: Bytes::new(Vec::new()),
                },
            },
        );
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
    fn ran(
        &mut self,
        id: EffectId,
        program: &Path,
        arguments: &[String],
        environment: &Environment,
        stdin: Option<&Bytes>,
    ) {
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
                // A turn's commands take a moment each, so that a turn is a
                // sequence a test can stop between the steps of, rather
                // than one instant.
                Some(
                    command @ (Command::Present { .. }
                    | Command::Build { .. }
                    | Command::Create { .. }
                    | Command::Start { .. }
                    | Command::Checkout { .. }
                    | Command::Exec { .. }),
                ) => {
                    let finished = self.ran_for_a_turn(command, environment, stdin);
                    self.schedule(self.now + 1, Event::Ran { id, finished });
                    return;
                }
                None => Finished::Failed("the simulation does not know this command".to_owned()),
            }
        } else {
            Finished::NotFound
        };
        self.schedule(self.now, Event::Ran { id, finished });
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
            AppEffect::Say { thread, text, .. } => self.posts.push((thread, text)),
            AppEffect::Respond { id, response } => {
                self.responses.insert(id, response);
            }
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

    /// What a probe of a port finds, as the runtime's proxy was measured to
    /// behave: a running container's published port accepts and closes at
    /// once when nothing inside is serving, holds the connection open in
    /// silence when something is, and a port no running container is
    /// published on refuses.
    fn probed(&self, port: u16) -> Probed {
        match self
            .containers
            .values()
            .find(|held| held.running && held.port == Some(port))
        {
            Some(held) if held.serving => Probed::Silent,
            Some(_) => Probed::Closed,
            None => Probed::Refused,
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
                environment,
                stdin,
            } => self.ran(id, &program, &arguments, &environment, stdin.as_ref()),
            Effect::Wake { id, after } => {
                let at = self.now + u64::try_from(after.as_millis()).expect("a short wait");
                self.schedule(at, Event::Woke { id });
            }
            Effect::Bind { id, address } => {
                if let Some(why) = self.bind_failures.pop_front() {
                    self.schedule(
                        self.now,
                        Event::Bound {
                            id,
                            outcome: Err(why),
                        },
                    );
                    return None;
                }
                self.listeners.insert(id, address.clone());
                // Whichever port was asked for, taken: a port of zero is
                // answered with one nothing else here uses, as a real one
                // would be.
                let asked = address
                    .rsplit_once(':')
                    .and_then(|(_, port)| port.parse::<u16>().ok())
                    .unwrap_or(0);
                let taken = if asked == 0 { 47_999 } else { asked };
                self.schedule(
                    self.now,
                    Event::Bound {
                        id,
                        outcome: Ok(taken),
                    },
                );
            }
            Effect::Answer { id, answer } => self.answered(id, answer),
            Effect::Probe { id, port, .. } => {
                let probed = self.probed(port);
                self.schedule(self.now, Event::Probed { id, probed });
            }
            Effect::Print { text } => self.printed.push(text),
            Effect::Exit { message } => self.exited = Some(message),
            Effect::Open { id, arguments, .. } => self.opened(id, &arguments),
            Effect::Send { id, line } => self.sent(id, &line),
            Effect::Close { id } => self.closed(id),
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

    /// The credentials the tools were declared with to agents so far,
    /// oldest first: what each agent was actually handed, read back off its
    /// session request.
    pub fn warrants(&self) -> Vec<String> {
        self.talks
            .iter()
            .filter_map(|talk| talk.warrant.clone())
            .collect()
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
