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

use stageman_agent::ToolCallStatus;
use stageman_agent::{Answer, Command, Heard, Label, Said, StopReason};
use stageman_channel::{Call, Reaction};
use stageman_core::{
    Agent, AgentConfig, Channel, ChannelConfig, Errand, InstanceId, Job, JobId, Key, Kit,
    KitConfig, KitName, NONCE_LEN, Nonce, Place, Progress, Project, ProjectId, Room, Secret,
    Snapshot, State, Thread, Timestamp, Uuid,
};
use stageman_instance::{
    AppEffect, AppEvent, Effect, Event, Instance, Request, RequestId, Response, Seed, Target,
};
use stageman_platform::Call as PlatformCall;
use stageman_vocabulary::scenario::{Meta, Recorder};
use stageman_vocabulary::{
    Answer as Answering, Arrival, Bytes, Disconnected, EffectId, Ended, Environment, Finished,
    Named as _, Probed, RequestId as Asked, Responded,
};

/// Virtual milliseconds: the vocabulary's own count, which every step is
/// told.
pub use stageman_vocabulary::Now;

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
    /// What it says and does before ending, when a transcript was scripted
    /// for this turn; otherwise it says the outcome's text in one piece.
    narrates: Option<(Vec<Utterance>, u64)>,
    /// Which conversation this is, in the record of them.
    talk: usize,
    /// The prompt in flight, by its request, and when its scripted answer
    /// lands: what a message handed to the turn is measured against, and
    /// what a cancel ends early.
    prompt: Option<(i64, Now)>,
}

/// One thing a scripted agent says or does while answering, in the order
/// scripted: what the instance notices as narration and as working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Utterance {
    /// A piece of its own text.
    Says(&'static str),
    /// A piece of its reasoning.
    Thinks(&'static str),
    /// A tool call beginning, under this title.
    Calls(&'static str),
    /// The last call under this title ending well.
    Done(&'static str),
    /// The last call under this title ending badly.
    Fails(&'static str),
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
    /// Every message handed to the turn that the adapter took, in order:
    /// what the agent was told mid-turn, as the pinned adapter was measured
    /// to take it and never echo it.
    pub steered: Vec<String>,
    /// Whether the prompt was cancelled, which ends it as cancelled.
    pub cancelled: bool,
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

/// One scripted utterance, as the line the agent would send for it. A call
/// is remembered under its title, so that its ending can name it.
fn uttered(
    session: &str,
    n: usize,
    utterance: &Utterance,
    calls: &mut Vec<(&'static str, String)>,
) -> Heard {
    match utterance {
        Utterance::Says(text) => Heard::said(session, text),
        Utterance::Thinks(text) => Heard::thought(session, text),
        Utterance::Calls(title) => {
            let call = format!("call-{n}");
            calls.push((title, call.clone()));
            Heard::called(session, &call, title)
        }
        Utterance::Done(title) | Utterance::Fails(title) => {
            let status = if matches!(utterance, Utterance::Done(_)) {
                ToolCallStatus::Completed
            } else {
                ToolCallStatus::Failed
            };
            let call = calls
                .iter()
                .rev()
                .find(|(named, _)| named == title)
                .map(|(_, call)| call.clone())
                .expect("a call under that title was scripted");
            Heard::call_changed(session, &call, None, Some(status))
        }
    }
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
    /// Why the platform refuses the next posts, front first.
    post_failures: VecDeque<String>,
    /// What each request was answered with, by identifier.
    tool_answers: BTreeMap<Asked, (u16, Option<serde_json::Value>)>,
    /// What each person's request was answered with, by identifier.
    responses: BTreeMap<RequestId, Response>,
    /// Where each tunnel request was sent, by identifier.
    routes: BTreeMap<Asked, Sent>,
    /// The last host port handed out.
    ports: u16,
    /// Jobs whose container the world has been asked to make. A job is on
    /// the record before its container exists, so only after this may the
    /// oracle expect the container.
    begun: BTreeSet<JobId>,
    /// Every container the instance has been told about or asked for: what
    /// a listing named, and what it made. The oracle judges only these. A
    /// container that appeared behind the instance's back is its business
    /// from the next listing, which is the sweep's own promise.
    seen: BTreeSet<String>,
    /// What each listing asked for named, by the identifier its answer
    /// carries: what the instance is told of when that answer lands, and
    /// judged on from then.
    listings: BTreeMap<EffectId, Vec<String>>,
    /// How long a turn takes.
    turn_takes: Now,
    trace: Vec<String>,
    posts: Vec<(Place, String)>,
    /// What the next turns say and do before ending, front first, for the
    /// turns a scenario scripts a transcript for.
    transcripts: VecDeque<(Vec<Utterance>, u64)>,
    /// Every edit made to a post, by the message's identifier, in order.
    edits: Vec<(String, String)>,
    /// Every message the platform holds, as it would give one back in a
    /// thread: what people and apps said in frames, and what was posted.
    said: Vec<serde_json::Value>,
    /// Why the next thread reads are refused, front first.
    thread_failures: VecDeque<String>,
    /// Whether the adapters say at the handshake that a running turn can be
    /// handed a message, as the pinned adapter was measured to.
    steerable: bool,
    /// How many of the next messages handed to a turn the adapter declines.
    steers_declined: u32,
    /// How many of the next cancels the adapter ignores: a turn that goes
    /// on as if nothing was said.
    cancels_ignored: u32,
    /// Rooms made so far, so each is named apart.
    rooms_made: u32,
    /// Every room made: its identifier, and the name asked for.
    rooms: Vec<(String, String)>,
    /// Every description a room was given: the room, and the text.
    described: Vec<(String, String)>,
    /// Everyone invited into a room: the room, and who.
    invited: Vec<(String, String)>,
    /// Every room archived.
    archived: Vec<String>,
    /// Every reaction put on a message: the room, the message, and which.
    reactions: Vec<(String, String, Reaction)>,
    /// Why the platform refuses the next rooms, front first.
    room_failures: VecDeque<String>,
    /// The sockets the platform holds, by the identifier each was connected
    /// under, and whether each is still open.
    sockets: BTreeMap<EffectId, bool>,
    /// How many event streams have been opened, so each is addressed apart.
    streams_opened: u32,
    /// How many envelopes have been delivered, so each is named apart.
    envelopes: u32,
    /// Every envelope acknowledged, in order.
    acked: Vec<String>,
    /// Why the platform refuses the next questions a listener asks, front
    /// first.
    listen_failures: VecDeque<String>,
    /// Why the platform refuses the next requests for where to connect in
    /// particular, front first, read before the queue above: what tells a
    /// wrong app-level token from a wrong bot token.
    locate_failures: VecDeque<String>,
    /// Why the next sockets cannot be opened, front first.
    socket_failures: VecDeque<String>,
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
    /// Every request to a platform asked for, as the channel crate reads it
    /// back and where in the trace it was asked — so a test names a call
    /// rather than a string.
    calls: Vec<(usize, Call)>,
    /// Every read of a platform asked for, as the platform crate reads it
    /// back and where in the trace it was asked.
    platform_calls: Vec<(usize, PlatformCall)>,
    /// What the platform answers the next reads of a repository, front
    /// first: a status and a body, as scripted. Unscripted, a read is
    /// answered as the real platform was measured to answer a token that
    /// reaches a private repository.
    platform_answers: VecDeque<(u16, String)>,
    /// Why the next reads of a repository get no answer at all, front
    /// first.
    platform_failures: VecDeque<String>,
    key: Key,
    /// What this flow is recorded as, where it is recorded at all: the file
    /// it is written to, and what that file says it pins.
    recording: Option<(String, Meta)>,
    /// The recording itself, once an instance has been constructed for it
    /// to begin from.
    recorder: Option<Recorder<Instance>>,
}

/// Who a simulated frame is from, and on which subscription.
///
/// Three shapes, as measured on the real platform: a person's mention
/// arrives as its own event, a person's message arrives on the message
/// subscription — whether the copy of a mention or plain talk — and what
/// this instance posted comes back carrying its own identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spoken {
    /// A person mentioning this instance, delivered as a mention event.
    Mention,
    /// A person's message on the message subscription.
    Plain,
    /// Something this instance posted itself.
    Ours,
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
pub fn job(n: u128) -> JobId {
    JobId::from_uuid(Uuid::from_u128(n))
}

/// A thread in the room people talk in, named by number.
pub fn thread(n: u32) -> Thread {
    Thread {
        channel: Channel::Slack,
        room: CHANNEL.to_owned(),
        id: format!("1788000000.{n:06}"),
    }
}

/// What identifies a person's message at the root of a job's room, as the
/// simulation numbers one.
pub const SAID_IN_ROOM: &str = "1788000099.000004";

/// What identifies a person's message in a thread of a job's room, as the
/// simulation numbers one.
pub const SAID_IN_ROOMS_THREAD: &str = "1788000099.000005";

/// Who this instance is on the simulated platform, as the platform answers.
pub fn us() -> stageman_channel::Identity {
    stageman_channel::Identity {
        user: "U0BOT".to_owned(),
        bot: "B0SELF".to_owned(),
        url: "https://example.slack.com/".to_owned(),
    }
}

/// A link to a message in a room, as the notices spell one.
pub fn link_to(room: &str, message: &str, thread: Option<&str>) -> String {
    stageman_channel::permalink(Channel::Slack, &us(), room, message, thread)
}

/// A job's room, named by number: the n-th room the platform made.
pub fn room(n: u32) -> Room {
    Room {
        channel: Channel::Slack,
        id: format!("C-job-{n:03}"),
    }
}

/// The root of a job's room, as a place to speak.
pub fn in_room(n: u32) -> Place {
    Place::root(room(n))
}

/// A thread in the room people talk in, as a place to speak.
pub fn in_thread(n: u32) -> Place {
    Place::from(thread(n))
}

fn a_job(progress: &Progress, room: Option<Room>) -> Job {
    let mut job = Job::new(
        Kit::defaults(Agent::Claude),
        "a reason".to_owned(),
        "some work".to_owned(),
        Timestamp::UNIX_EPOCH,
    );
    job.progress = progress.clone();
    job.room = room;
    job
}

fn a_project(jobs: BTreeMap<JobId, Job>, bound: bool) -> Project {
    let mut channels = BTreeMap::new();
    if bound {
        channels.insert(
            Channel::Slack,
            ChannelConfig {
                credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                listen_credential: Secret::new("xapp-not-a-real-token".to_owned()),
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
        brief: String::new(),
        watched: std::collections::BTreeSet::new(),
        foreman_room: None,
    }
}

fn configured(project: Project) -> State {
    State {
        apps: std::collections::BTreeMap::new(),
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
            .map(|(id, progress)| (id.clone(), a_job(progress, None)))
            .collect(),
        false,
    ))
}

/// An instance watching one project with a channel bound, each job in the
/// room numbered for it.
pub fn watching_a_channel(jobs: &[(JobId, Progress, u32)]) -> State {
    configured(a_project(
        jobs.iter()
            .map(|(id, progress, n)| (id.clone(), a_job(progress, Some(self::room(*n)))))
            .collect(),
        true,
    ))
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
            from: Some("U0HUMAN".to_owned()),
            message: Some(thread(n).id),
            app: None,
        });
}

/// Marks a room as one the project's foreman watches, as a person asking
/// it there would have.
pub fn watching_a_room(state: &mut State, room: &str) {
    state
        .projects
        .get_mut(&project())
        .expect("the project")
        .watched
        .insert(Room {
            channel: Channel::Slack,
            id: room.to_owned(),
        });
}

/// Gives the project a brief, as its operator would on the form.
pub fn briefed(state: &mut State, brief: &str) {
    brief.clone_into(
        &mut state
            .projects
            .get_mut(&project())
            .expect("the project")
            .brief,
    );
}

/// The name a job's tunnel answers on, under the domain every scenario uses.
pub fn tunnel_host(job: &JobId) -> String {
    format!("{job}.localhost")
}

/// A person asking something of the dashboard.
pub fn request(id: u64, request: Request) -> Event {
    Event::App(AppEvent::Request {
        id: RequestId(id),
        request: Box::new(request),
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
            begun: BTreeSet::new(),
            seen: BTreeSet::new(),
            listings: BTreeMap::new(),
            turn_takes: 1_000,
            trace: Vec::new(),
            posts: Vec::new(),
            transcripts: VecDeque::new(),
            edits: Vec::new(),
            said: Vec::new(),
            thread_failures: VecDeque::new(),
            steerable: true,
            steers_declined: 0,
            cancels_ignored: 0,
            rooms_made: 0,
            rooms: Vec::new(),
            described: Vec::new(),
            invited: Vec::new(),
            archived: Vec::new(),
            reactions: Vec::new(),
            room_failures: VecDeque::new(),
            sockets: BTreeMap::new(),
            streams_opened: 0,
            envelopes: 0,
            acked: Vec::new(),
            listen_failures: VecDeque::new(),
            locate_failures: VecDeque::new(),
            socket_failures: VecDeque::new(),
            images: vec!["stageman:unneeded".to_owned()],
            listeners: BTreeMap::new(),
            bodies: BTreeMap::new(),
            asking_next: 1,
            commands: Vec::new(),
            calls: Vec::new(),
            platform_calls: Vec::new(),
            platform_answers: VecDeque::new(),
            platform_failures: VecDeque::new(),
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

    /// Scripts the next post to be refused by the platform, with that
    /// reason — as the platform refuses, which is with a successful status.
    pub fn next_post_fails(&mut self, why: &str) {
        self.post_failures.push_back(why.to_owned());
    }

    /// Scripts the next room to be refused by the platform, with that
    /// reason: a name already taken is the ordinary one.
    pub fn next_room_fails(&mut self, why: &str) {
        self.room_failures.push_back(why.to_owned());
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

    /// How many connections the platform holds open for this instance.
    pub fn listening(&self) -> usize {
        self.sockets_open().len()
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

    /// Says the browser arrived at a path on the dashboard's listener, as
    /// a platform's redirect brings it there.
    pub fn visits_path(&mut self, at: Now, path: &str) -> Asked {
        self.arriving(
            self.listener(false),
            at,
            "GET",
            path,
            &[("host", "localhost")],
            "127.0.0.1:50000",
            "",
        )
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

    /// Where in the trace a platform was first asked something of a kind.
    pub fn first_call(&self, wanted: impl Fn(&Call) -> bool) -> Option<usize> {
        self.calls
            .iter()
            .find(|(_, call)| wanted(call))
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

    /// Scripts what the next turn's agent says and does before it ends, in
    /// order, instead of saying its outcome's text in one piece.
    pub fn next_turn_narrates(&mut self, script: Vec<Utterance>) {
        self.transcripts.push_back((script, 0));
    }

    /// Scripts what the next turn's agent says and does, each utterance this
    /// many milliseconds after the previous, so that pacing has time to
    /// come round between them.
    pub fn next_turn_narrates_over(&mut self, script: Vec<Utterance>, apart: u64) {
        self.transcripts.push_back((script, apart));
    }

    /// Scripts the adapters to say nothing at the handshake about handing a
    /// running turn a message: an adapter that cannot be steered.
    pub const fn adapters_unsteerable(&mut self) {
        self.steerable = false;
    }

    /// Scripts the adapter to decline the next message handed to a turn.
    pub const fn next_steer_declined(&mut self) {
        self.steers_declined += 1;
    }

    /// Scripts the adapter to ignore the next cancel: the prompt goes on as
    /// if nothing was said.
    pub const fn next_cancel_ignored(&mut self) {
        self.cancels_ignored += 1;
    }

    /// Every edit made to a post, by the message's identifier, in order.
    pub fn edits(&self) -> &[(String, String)] {
        &self.edits
    }

    /// Every post made, with where in the trace it was asked for.
    pub fn post_calls(&self) -> Vec<(usize, String)> {
        self.calls
            .iter()
            .filter_map(|(at, call)| match call {
                Call::Post { text, .. } => Some((*at, text.clone())),
                _ => None,
            })
            .collect()
    }

    /// Where in the trace each answer from a platform arrived.
    pub fn responded_at(&self) -> Vec<usize> {
        self.shape()
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with("<- Responded"))
            .map(|(at, _)| at)
            .collect()
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
                    | Event::Responded { .. }
                    | Event::Frame { .. }
                    | Event::Disconnected { .. }
                    | Event::App(AppEvent::Presenting { .. } | AppEvent::Request { .. })
            )
        });
        self.landing.clear();
        self.begun.clear();
        // The agent processes die with the daemon that held their pipes;
        // what they wrote into their containers stays. So do the sockets:
        // the platform sees them close, and delivers nothing to them again.
        self.adapters.clear();
        self.sockets.clear();
        // Nothing asked before the crash is answered after it, and the next
        // boot lists what is there.
        self.listings.clear();
        self.seen.clear();
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
        // A listing's answer landing is when the instance is told what is
        // there, and from then on it is judged on it.
        if let Event::Ran { id, .. } = &event
            && let Some(named) = self.listings.remove(id)
        {
            self.seen.extend(named);
        }
        let effects = instance.step(self.now, event.clone());
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

    /// What must hold after every step: the bar in `docs/conventions.md` §4,
    /// as far as a simulated runtime can see it.
    ///
    /// A working job has a container, unless its first turn has not been
    /// asked for yet: the record is written before the container exists, on
    /// purpose, and the container is made by the turn. And once the instance
    /// is awake and has swept, nothing of its own that it has listed or made
    /// is running with nothing in it — no turn in flight for it, no message
    /// waiting for its foreman, nothing answering on its tunnel, and no
    /// question about it in flight. Another instance's container is not judged,
    /// and neither is one that appeared behind this instance's back until a
    /// listing has named it, which is the sweep's own promise. See
    /// `docs/decisions/0066-a-foremans-container-runs-only-while-a-turn-runs-in-it.md`,
    /// which is the state this fails on.
    fn oracle(&self, instance: &Instance) {
        for job in instance.state().working() {
            assert!(
                self.containers.contains_key(&stageman_job::container(&job))
                    || !self.begun.contains(&job),
                "job {job} is working with no container"
            );
        }
        // Nothing running means nothing waiting: a job that is not working
        // has an empty inbox, per
        // `docs/decisions/0069-a-message-reaches-a-working-job.md`, kept by
        // the instance's transitions rather than by the type.
        for (job, recorded) in instance
            .state()
            .projects
            .values()
            .flat_map(|project| project.jobs.iter())
        {
            assert!(
                recorded.progress == Progress::Working || recorded.inbox.is_empty(),
                "job {job} is not working and holds messages: {:?}",
                self.trace.iter().rev().take(8).collect::<Vec<_>>()
            );
        }
        if instance.swept().is_none() {
            return;
        }
        let turning = instance.turning();
        let reading = instance.reading_for();
        let asking = instance.asking_about();
        for (name, held) in &self.containers {
            if !held.running || held.instance != Some(this_instance()) || !self.seen.contains(name)
            {
                continue;
            }
            let talking = self
                .adapters
                .values()
                .any(|adapter| adapter.container == *name);
            let deciding = asking.iter().any(|about| about == name);
            let busy = match (
                stageman_job::job_of(name),
                stageman_foreman::project_of(name),
            ) {
                // A thread being read for a job's next turn is a question
                // about it in flight, for the one step between a turn's end
                // and the next turn's registration.
                (Some(job), _) => {
                    turning.contains(&stageman_instance::Speaker::Job(job.clone()))
                        || reading.contains(&stageman_instance::Speaker::Job(job))
                        || held.serving
                        || deciding
                }
                (None, Some(project)) => {
                    turning.contains(&stageman_instance::Speaker::Foreman(project))
                        || deciding
                        || instance
                            .state()
                            .projects
                            .get(&project)
                            .is_some_and(|watched| watched.attending.on().is_some())
                }
                // A name this version cannot read is the waking sweep's to
                // remove, in the step that finds it.
                (None, None) => false,
            };
            assert!(
                talking || busy,
                "container {name} is running with nothing in it, after: {:?}",
                self.trace.iter().rev().take(8).collect::<Vec<_>>()
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
            steered: Vec::new(),
            cancelled: false,
        });
        let talk = self.talks.len() - 1;
        self.talking.insert(id, talk);
        let narrates = self.transcripts.pop_front();
        self.adapters.insert(
            id,
            Adapter {
                container: name,
                session: None,
                options: Vec::new(),
                outcome,
                narrates,
                talk,
                prompt: None,
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
            Said::Initialize { id: request } => {
                let handshake = if self.steerable {
                    Heard::initialized(request)
                } else {
                    Heard::initialized_unsteerable(request)
                };
                self.schedule(
                    reply_at,
                    Event::Line {
                        id,
                        line: handshake.line(),
                    },
                );
            }
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
            Said::Steer {
                id: request, text, ..
            } => self.steered(id, talk, request, text),
            Said::Cancel { .. } => self.cancelled(id),
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
        let narrates = adapter.narrates.clone();
        match adapter.outcome.clone() {
            Ok(answer) => {
                // What it says and does, as scripted and as far apart as
                // scripted, or the outcome's text in one piece: either way
                // before the answer that ends it.
                let mut ends_at = at;
                match narrates {
                    Some((script, apart)) => {
                        let mut calls: Vec<(&'static str, String)> = Vec::new();
                        for (n, utterance) in script.iter().enumerate() {
                            let when = at + u64::try_from(n).expect("a short script") * apart;
                            ends_at = when;
                            let line = uttered(&session, n, utterance, &mut calls);
                            self.schedule(
                                when,
                                Event::Line {
                                    id,
                                    line: line.line(),
                                },
                            );
                        }
                    }
                    None => self.schedule(
                        at,
                        Event::Line {
                            id,
                            line: Heard::said(&session, &answer.text).line(),
                        },
                    ),
                }
                self.schedule(
                    ends_at,
                    Event::Line {
                        id,
                        line: Heard::prompted(request, answer.stop_reason).line(),
                    },
                );
                if let Some(adapter) = self.adapters.get_mut(&id) {
                    adapter.prompt = Some((request, ends_at));
                }
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

    /// A message handed to a turn, answered as the pinned adapter was
    /// measured to: taken into the prompt in flight, which goes on as
    /// scripted and ends once; refused when no prompt is in flight, or when
    /// a scenario scripted the adapter to decline. What was taken is
    /// remembered on the conversation, since the adapter never echoes it.
    fn steered(&mut self, id: EffectId, talk: usize, request: i64, text: String) {
        // One scripted refusal is spent per message handed over.
        let declined = match self.steers_declined.checked_sub(1) {
            Some(left) => {
                self.steers_declined = left;
                true
            }
            None => false,
        };
        let Some(adapter) = self.adapters.get(&id) else {
            return;
        };
        let running = adapter
            .prompt
            .is_some_and(|(_, ends_at)| ends_at > self.now);
        let landed = running && !declined;
        if landed && let Some(talk) = self.talks.get_mut(talk) {
            talk.steered.push(text);
        }
        self.schedule(
            self.now + 1,
            Event::Line {
                id,
                line: Heard::steered(request, landed).line(),
            },
        );
    }

    /// A cancel, answered as measured: the prompt in flight ends at once as
    /// cancelled, whatever it had left to say, and the session survives. One
    /// with no prompt in flight is nothing.
    fn cancelled(&mut self, id: EffectId) {
        if let Some(left) = self.cancels_ignored.checked_sub(1) {
            self.cancels_ignored = left;
            return;
        }
        let Some(adapter) = self.adapters.get_mut(&id) else {
            return;
        };
        let Some((request, ends_at)) = adapter.prompt else {
            return;
        };
        if ends_at <= self.now {
            return;
        }
        adapter.prompt = None;
        let talk = adapter.talk;
        self.queue
            .retain(|_, event| !matches!(event, Event::Line { id: whose, .. } if *whose == id));
        if let Some(talk) = self.talks.get_mut(talk) {
            talk.cancelled = true;
        }
        self.schedule(
            self.now + 1,
            Event::Line {
                id,
                line: Heard::prompted(request, StopReason::Cancelled).line(),
            },
        );
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
            self.seen.insert(name.to_owned());
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
                Some(Command::Containers { running_only }) => {
                    let named: Vec<String> = self
                        .containers
                        .iter()
                        .filter(|(_, held)| held.running || !running_only)
                        .map(|(name, _)| name.clone())
                        .collect();
                    self.listings.insert(id, named.clone());
                    exited(named.iter().fold(String::new(), |mut listed, name| {
                        listed.push_str(name);
                        listed.push('\n');
                        listed
                    }))
                }
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
        if let Effect::Request {
            method,
            url,
            headers,
            body,
            ..
        } = &effect
            && let Some(call) = Call::parse(&stageman_channel::Request {
                method: method.clone(),
                url: url.clone(),
                headers: headers.clone(),
                body: body.as_ref().map(|bytes| bytes.as_slice().to_vec()),
            })
        {
            self.calls.push((self.trace.len(), call));
        }
        if let Effect::Request {
            method,
            url,
            headers,
            body,
            ..
        } = &effect
            && let Some(call) = PlatformCall::parse(&stageman_platform::Request {
                method: method.clone(),
                url: url.clone(),
                headers: headers.clone(),
                body: body.as_ref().map(|bytes| bytes.as_slice().to_vec()),
            })
        {
            self.platform_calls.push((self.trace.len(), call));
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
            AppEffect::Respond { id, response } => {
                self.responses.insert(id, response);
            }
        }
    }

    /// Takes a post, unless the next one was scripted to be refused:
    /// numbered as the platform numbers one, remembered as the platform
    /// would give it back in a thread — this instance's own, by its
    /// identifiers — and kept as what was posted where. What the platform
    /// answers either way.
    fn posted(&mut self, place: Place, text: String) -> String {
        if let Some(error) = self.post_failures.pop_front() {
            return format!(r#"{{"ok":false,"error":"{error}"}}"#);
        }
        let identifier = format!("1788000000.9{:05}", self.posts.len());
        let mut held = serde_json::Map::new();
        held.insert("type".to_owned(), "message".into());
        held.insert("ts".to_owned(), identifier.clone().into());
        held.insert("channel".to_owned(), place.room.id.clone().into());
        held.insert("user".to_owned(), "U0BOT".into());
        held.insert("bot_id".to_owned(), "B0SELF".into());
        held.insert("text".to_owned(), text.clone().into());
        if let Some(thread) = &place.thread {
            held.insert("thread_ts".to_owned(), thread.clone().into());
        }
        self.said.push(serde_json::Value::Object(held));
        self.posts.push((place, text));
        format!(r#"{{"ok":true,"ts":"{identifier}"}}"#)
    }

    /// A thread as the platform gives it back, or the scripted refusal.
    fn thread_of(&mut self, room: &str, thread: &str, at_most: usize) -> String {
        if let Some(error) = self.thread_failures.pop_front() {
            return format!(r#"{{"ok":false,"error":"{error}"}}"#);
        }
        let field = |held: &serde_json::Value, name: &str| {
            held.get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let in_room: Vec<&serde_json::Value> = self
            .said
            .iter()
            .filter(|held| field(held, "channel").as_deref() == Some(room))
            .collect();
        let parent = in_room
            .iter()
            .find(|held| field(held, "ts").as_deref() == Some(thread))
            .copied();
        let replies: Vec<&serde_json::Value> = in_room
            .iter()
            .filter(|held| {
                field(held, "thread_ts").as_deref() == Some(thread)
                    && field(held, "ts").as_deref() != Some(thread)
            })
            .copied()
            .collect();
        let has_more = replies.len() > at_most;
        let recent: Vec<&serde_json::Value> =
            replies.iter().rev().take(at_most).rev().copied().collect();
        let messages: Vec<&serde_json::Value> = parent.into_iter().chain(recent).collect();
        serde_json::json!({"ok": true, "has_more": has_more, "messages": messages}).to_string()
    }

    /// Scripts the platform to refuse the next thread asked for.
    pub fn next_thread_read_fails(&mut self, why: &str) {
        self.thread_failures.push_back(why.to_owned());
    }

    /// Somebody saying something without mentioning this instance, at the
    /// root of a room or in a thread of it: dropped by the reader, and
    /// remembered by the platform.
    pub fn person_says(&mut self, at: Now, room: &str, id: &str, thread: Option<&str>, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(socket, room, id, thread, text, Spoken::Plain);
        self.schedule(at, event);
    }

    /// Something this instance said once, heard back as its own: what the
    /// platform remembers of it in a thread.
    pub fn we_said(&mut self, at: Now, room: &str, id: &str, thread: Option<&str>, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(socket, room, id, thread, text, Spoken::Ours);
        self.schedule(at, event);
    }

    /// Takes an edit to a post: the post reads as edited, and the platform
    /// answers naming the message again, or says it knows no such message.
    fn edited(&mut self, message: &str, text: String) -> String {
        let index = message
            .strip_prefix("1788000000.9")
            .and_then(|digits| digits.parse::<usize>().ok());
        match index.and_then(|index| self.posts.get_mut(index)) {
            Some((_, posted)) => {
                posted.clone_from(&text);
                self.edits.push((message.to_owned(), text));
                format!(r#"{{"ok":true,"ts":"{message}"}}"#)
            }
            None => r#"{"ok":false,"error":"message_not_found"}"#.to_owned(),
        }
    }

    /// Answers a request to a platform as the real one was measured to,
    /// recognising what it asks through the channel crate's own inverse.
    ///
    /// A post is taken and named, at the root or in a thread — unless the
    /// next one was scripted to be refused, in which case the refusal
    /// arrives as the platform sends it: a successful status with a body
    /// that says no, which is the trap the reader exists for. A request this
    /// crate did not render is not one the simulation can answer.
    fn requested(
        &mut self,
        id: EffectId,
        method: String,
        url: String,
        headers: BTreeMap<String, String>,
        body: Option<Bytes>,
    ) {
        // A platform's read before a channel's calls: the two crates read
        // different requests, and a repository read is nobody's post.
        if self.read_repository(
            id,
            &stageman_platform::Request {
                method: method.clone(),
                url: url.clone(),
                headers: headers.clone(),
                body: body.as_ref().map(|bytes| bytes.as_slice().to_vec()),
            },
        ) {
            return;
        }
        let request = stageman_channel::Request {
            method,
            url,
            headers,
            body: body.map(Bytes::into_inner),
        };
        // As the platform answers everything: a successful status, and the
        // body saying whether it was.
        let answered = |body: String| Responded::Answered {
            status: 200,
            headers: [("content-type".to_owned(), "application/json".to_owned())].into(),
            body: body.into(),
        };
        let edited = matches!(Call::parse(&request), Some(Call::Update { .. }));
        let responded = match Call::parse(&request) {
            Some(Call::Post {
                channel,
                room,
                text,
                thread,
            }) => {
                let place = Place {
                    room: Room { channel, id: room },
                    thread,
                };
                answered(self.posted(place, text))
            }
            // An edit to a post this simulation took: the post reads as
            // edited, and the platform answers naming the message again.
            Some(Call::Update { message, text, .. }) => answered(self.edited(&message, text)),
            // A thread given back as measured: its parent, then its most
            // recent replies up to the limit, oldest first, and whether
            // older ones were left out — unless the next read was scripted
            // to be refused.
            Some(Call::Replies {
                room,
                thread,
                at_most,
                ..
            }) => answered(self.thread_of(&room, &thread, at_most)),
            // A room made, named as the platform names it: by number, in the
            // order made, unless the next one was scripted to be refused.
            Some(Call::CreateRoom { name, .. }) => {
                let body = if let Some(error) = self.room_failures.pop_front() {
                    format!(r#"{{"ok":false,"error":"{error}"}}"#)
                } else {
                    self.rooms_made += 1;
                    let made = room(self.rooms_made);
                    self.rooms.push((made.id.clone(), name.clone()));
                    format!(
                        r#"{{"ok":true,"channel":{{"id":"{}","name":"{name}"}}}}"#,
                        made.id
                    )
                };
                answered(body)
            }
            Some(Call::SetPurpose { room, purpose, .. }) => {
                self.described.push((room, purpose));
                answered(r#"{"ok":true}"#.to_owned())
            }
            Some(Call::SetTopic { room, topic, .. }) => {
                self.described.push((room, topic));
                answered(r#"{"ok":true}"#.to_owned())
            }
            Some(Call::Invite { room, user, .. }) => {
                self.invited.push((room, user));
                answered(r#"{"ok":true}"#.to_owned())
            }
            Some(Call::Archive { room, .. }) => {
                self.archived.push(room);
                answered(r#"{"ok":true}"#.to_owned())
            }
            Some(Call::React {
                room,
                message,
                reaction,
                ..
            }) => {
                self.reactions.push((room, message, reaction));
                answered(r#"{"ok":true}"#.to_owned())
            }
            Some(question @ (Call::WhoAmI { .. } | Call::OpenSocket { .. })) => {
                answered(self.listener_answered(&question))
            }
            None => Responded::Failed("the simulation does not know this request".to_owned()),
        };
        // An edit is answered a tick later, as a line, a write and a probe
        // are: it takes time in the real world, and an instance that edited
        // again the moment it was answered would then run out of time rather
        // than hang, which a scenario can see. Everything else is answered
        // at once, which is what the scenarios' timings were written to.
        let at = if edited { self.now + 1 } else { self.now };
        self.schedule(at, Event::Responded { id, responded });
    }

    /// Answers one of the two questions a listener asks — and a check of a
    /// binding's credentials asks the same two — as the platform does,
    /// unless the next was scripted to be refused. A wrong app-level token
    /// refuses only the request for where to connect, as measured.
    fn listener_answered(&mut self, question: &Call) -> String {
        let refused = match question {
            Call::OpenSocket { .. } => self
                .locate_failures
                .pop_front()
                .or_else(|| self.listen_failures.pop_front()),
            _ => self.listen_failures.pop_front(),
        };
        if let Some(error) = refused {
            format!(r#"{{"ok":false,"error":"{error}"}}"#)
        } else if matches!(question, Call::WhoAmI { .. }) {
            r#"{"ok":true,"user_id":"U0BOT","bot_id":"B0SELF","url":"https://example.slack.com/"}"#
                .to_owned()
        } else {
            self.streams_opened += 1;
            format!(
                r#"{{"ok":true,"url":"wss://sim.slack/link/{}"}}"#,
                self.streams_opened
            )
        }
    }

    /// Answers a read of a repository, if the request is one: as the real
    /// platform was measured to answer a token that reaches a private
    /// repository, unless the next answer was scripted otherwise. False for
    /// a request that is not a platform's read.
    fn read_repository(&mut self, id: EffectId, request: &stageman_platform::Request) -> bool {
        let Some(call) = PlatformCall::parse(request) else {
            return false;
        };
        let responded = if let Some(why) = self.platform_failures.pop_front() {
            Responded::Failed(why)
        } else {
            let (status, body) = self.platform_answers.pop_front().unwrap_or_else(|| match call {
                PlatformCall::Repository { owner, name, .. } => (
                    200,
                    format!(r#"{{"full_name":"{owner}/{name}","private":true}}"#),
                ),
                // An App created from a manifest, as the platform answers:
                // the App object with the key beside it.
                PlatformCall::Exchange { code, .. } => (
                    201,
                    format!(
                        r#"{{"id":4242,"slug":"stageman-sim","client_id":"Iv1.sim{code}","pem":"-----BEGIN RSA PRIVATE KEY-----\nsim\n-----END RSA PRIVATE KEY-----\n","client_secret":"not-kept","webhook_secret":"not-kept","html_url":"https://github.com/apps/stageman-sim"}}"#
                    ),
                ),
            });
            Responded::Answered {
                status,
                headers: [(
                    "content-type".to_owned(),
                    "application/json; charset=utf-8".to_owned(),
                )]
                .into(),
                body: body.into(),
            }
        };
        self.schedule(self.now, Event::Responded { id, responded });
        true
    }

    /// Opens a socket the instance asked for: the platform greets on it at
    /// once, which is what says it is up — unless the next one was scripted
    /// not to open.
    fn connected(&mut self, id: EffectId) {
        if let Some(why) = self.socket_failures.pop_front() {
            self.schedule(
                self.now,
                Event::Disconnected {
                    id,
                    disconnected: Disconnected::Failed(why),
                },
            );
            return;
        }
        self.sockets.insert(id, true);
        self.schedule(
            self.now,
            Event::Frame {
                id,
                text: r#"{"type":"hello","num_connections":1}"#.to_owned(),
            },
        );
    }

    /// A frame sent on a socket: an acknowledgement, written down so a test
    /// can say what was acknowledged. One sent on a socket that has ended
    /// goes nowhere, as it would.
    fn transmitted(&mut self, id: EffectId, text: &str) {
        if !self.sockets.get(&id).copied().unwrap_or(false) {
            return;
        }
        let acknowledged: serde_json::Value =
            serde_json::from_str(text).expect("a frame this instance sends is JSON");
        if let Some(envelope) = acknowledged
            .get("envelope_id")
            .and_then(serde_json::Value::as_str)
        {
            self.acked.push(envelope.to_owned());
        }
    }

    /// A socket ends, closed, at an instant: as the platform closes one, or
    /// as the instance's disconnect is answered.
    fn closes(&mut self, id: EffectId, at: Now) {
        let Some(open) = self.sockets.get_mut(&id) else {
            return;
        };
        if !*open {
            return;
        }
        *open = false;
        self.schedule(
            at,
            Event::Disconnected {
                id,
                disconnected: Disconnected::Closed,
            },
        );
    }

    /// The socket the platform delivers on now: the newest still open.
    fn live_socket(&self) -> EffectId {
        self.sockets
            .iter()
            .rev()
            .find(|(_, open)| **open)
            .map(|(id, _)| *id)
            .expect("nothing is listening, so nothing can be said")
    }

    /// One message as the platform delivers it: a frame on a socket, in an
    /// envelope of its own.
    fn frame_on(
        &mut self,
        socket: EffectId,
        room: &str,
        id: &str,
        in_thread: Option<&str>,
        text: &str,
        spoken: Spoken,
    ) -> Event {
        self.envelopes += 1;
        let envelope = format!("e-{}", self.envelopes);
        let thread_ts =
            in_thread.map_or_else(String::new, |thread| format!(r#","thread_ts":"{thread}""#));
        // As measured: a person's mention is its own event, a person's
        // message is the copy of one on the message subscription, and what
        // this instance posted comes back as a message carrying both of its
        // identifiers.
        let (kind, speaker) = match spoken {
            Spoken::Mention => ("app_mention", r#""user":"U0HUMAN""#),
            Spoken::Plain => ("message", r#""user":"U0HUMAN""#),
            Spoken::Ours => ("message", r#""bot_id":"B0SELF","user":"U0BOT""#),
        };
        // Remembered as the platform would give it back in a thread, by the
        // same identifiers the frame carries.
        let mut held = serde_json::Map::new();
        held.insert("type".to_owned(), "message".into());
        held.insert("ts".to_owned(), id.into());
        held.insert("channel".to_owned(), room.into());
        held.insert("text".to_owned(), text.into());
        if let Some(thread) = in_thread {
            held.insert("thread_ts".to_owned(), thread.into());
        }
        match spoken {
            Spoken::Mention | Spoken::Plain => {
                held.insert("user".to_owned(), "U0HUMAN".into());
            }
            Spoken::Ours => {
                held.insert("user".to_owned(), "U0BOT".into());
                held.insert("bot_id".to_owned(), "B0SELF".into());
            }
        }
        self.said.push(serde_json::Value::Object(held));
        let text = serde_json::Value::String(text.to_owned()).to_string();
        Event::Frame {
            id: socket,
            text: format!(
                r#"{{"envelope_id":"{envelope}","type":"events_api","payload":{{"event":{{"type":"{kind}","channel":"{room}",{speaker},"text":{text},"ts":"{id}"{thread_ts}}}}}}}"#
            ),
        }
    }

    /// Another app posting in a room, as the GitHub app was measured to: an
    /// empty text, everything in one attachment, and a profile naming the
    /// app. At the root when `under` is none; a follow-up under an earlier
    /// message otherwise, which arrives as a broadcast carrying the app's
    /// profile on its root and not on itself.
    fn app_frame(
        &mut self,
        room: &str,
        id: &str,
        under: Option<&str>,
        headline: &str,
        body: &str,
    ) -> Event {
        let socket = self.live_socket();
        self.envelopes += 1;
        let envelope = format!("e-{}", self.envelopes);
        let profile = serde_json::json!({"id": "B0OTHER", "name": "GitHub"});
        let mut card = serde_json::Map::new();
        card.insert(
            "fallback".to_owned(),
            format!("[example/repo] {headline}").into(),
        );
        card.insert("pretext".to_owned(), headline.into());
        card.insert(
            "title".to_owned(),
            "<https://example.invalid/repo/issues/1|#1 The parser is flaky>".into(),
        );
        card.insert(
            "footer".to_owned(),
            "<https://example.invalid/repo|example/repo>".into(),
        );
        let mut event = serde_json::Map::new();
        event.insert("type".to_owned(), "message".into());
        event.insert("channel".to_owned(), room.into());
        event.insert("user".to_owned(), "U0GITHUB".into());
        event.insert("bot_id".to_owned(), "B0OTHER".into());
        event.insert("text".to_owned(), "".into());
        event.insert("ts".to_owned(), id.into());
        match under {
            None => {
                event.insert("bot_profile".to_owned(), profile);
                card.insert("text".to_owned(), body.into());
            }
            Some(parent) => {
                event.insert("subtype".to_owned(), "thread_broadcast".into());
                event.insert("thread_ts".to_owned(), parent.into());
                event.insert(
                    "root".to_owned(),
                    serde_json::json!({"bot_profile": profile}),
                );
            }
        }
        event.insert(
            "attachments".to_owned(),
            serde_json::Value::Array(vec![serde_json::Value::Object(card)]),
        );
        // Remembered as the platform would give it back in a thread: the
        // event itself, which is what a thread's messages are shaped like.
        self.said.push(serde_json::Value::Object(event.clone()));
        Event::Frame {
            id: socket,
            text: serde_json::json!({
                "envelope_id": envelope,
                "type": "events_api",
                "payload": {"event": serde_json::Value::Object(event)},
            })
            .to_string(),
        }
    }

    /// Another app posting at the root of a room, at an instant.
    pub fn app_posts(&mut self, at: Now, room: &str, id: &str, headline: &str, body: &str) {
        let event = self.app_frame(room, id, None, headline, body);
        self.schedule(at, event);
    }

    /// Another app following up under an earlier message of its own, at an
    /// instant.
    pub fn app_follows_up(&mut self, at: Now, room: &str, id: &str, under: &str, headline: &str) {
        let event = self.app_frame(room, id, Some(under), headline, "");
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance in a thread, said at an instant.
    pub fn says_in(&mut self, at: Now, thread: u32, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(
            socket,
            CHANNEL,
            "1788000099.000001",
            Some(&self::thread(thread).id),
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance at the root of the project's home
    /// room, said at an instant. Each is its own message, so each opens its
    /// own thread.
    pub fn says_at_root(&mut self, at: Now, n: u32, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(
            socket,
            CHANNEL,
            &self::thread(n).id,
            None,
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance at the root, on the socket named
    /// rather than the newest: what a message on a connection being
    /// replaced looks like.
    pub fn says_at_root_on(&mut self, socket: EffectId, at: Now, n: u32, text: &str) {
        let event = self.frame_on(
            socket,
            CHANNEL,
            &self::thread(n).id,
            None,
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance at the root of a job's room, said
    /// at an instant.
    pub fn says_in_room(&mut self, at: Now, n: u32, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(
            socket,
            &room(n).id,
            "1788000099.000004",
            None,
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance at the root of a job's room, under
    /// the identifier given: what two messages to one job need, so that
    /// each is reacted to and linked apart.
    pub fn says_in_room_as(&mut self, at: Now, n: u32, id: &str, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(
            socket,
            &room(n).id,
            id,
            None,
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance in a thread inside a job's room,
    /// under the identifier given.
    pub fn says_in_rooms_thread_as(&mut self, at: Now, n: u32, thread: &str, id: &str, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(
            socket,
            &room(n).id,
            id,
            Some(thread),
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Somebody mentioning this instance in a thread inside a job's room,
    /// said at an instant.
    pub fn says_in_rooms_thread(&mut self, at: Now, n: u32, thread: &str, text: &str) {
        let socket = self.live_socket();
        let event = self.frame_on(
            socket,
            &room(n).id,
            "1788000099.000005",
            Some(thread),
            &format!("<@U0BOT> {text}"),
            Spoken::Mention,
        );
        self.schedule(at, event);
    }

    /// Something said in a job's room that is not a person's mention: plain
    /// talk, the copy of a mention on the message subscription, or this
    /// instance's own post.
    pub fn said_in_room(&mut self, n: u32, text: &str, spoken: Spoken) -> Event {
        let socket = self.live_socket();
        self.frame_on(socket, &room(n).id, "1788000099.000006", None, text, spoken)
    }

    /// The platform warns, at an instant, that it is about to close the
    /// connection it delivers on now.
    pub fn platform_warns(&mut self, at: Now) {
        let socket = self.live_socket();
        self.schedule(
            at,
            Event::Frame {
                id: socket,
                text: r#"{"type":"disconnect","reason":"warning"}"#.to_owned(),
            },
        );
    }

    /// The platform closes, at an instant, the connection it delivers on
    /// now, with no warning.
    pub fn platform_closes(&mut self, at: Now) {
        let socket = self.live_socket();
        self.closes(socket, at);
    }

    /// The platform closes the connection named, at an instant.
    pub fn platform_closes_socket(&mut self, socket: EffectId, at: Now) {
        self.closes(socket, at);
    }

    /// Scripts the platform to refuse the next question a listener asks.
    pub fn next_listen_fails(&mut self, why: &str) {
        self.listen_failures.push_back(why.to_owned());
    }

    /// Scripts the platform to refuse the next request for where to
    /// connect, and that one alone: a wrong app-level token, as measured.
    pub fn next_locate_fails(&mut self, why: &str) {
        self.locate_failures.push_back(why.to_owned());
    }

    /// Scripts the platform's answer to the next read of a repository: a
    /// status, and the body it came with, as the real platform was measured
    /// to answer.
    pub fn next_platform_answers(&mut self, status: u16, body: &str) {
        self.platform_answers.push_back((status, body.to_owned()));
    }

    /// Scripts the next read of a repository to get no answer at all.
    pub fn next_platform_fails(&mut self, why: &str) {
        self.platform_failures.push_back(why.to_owned());
    }

    /// Every request to a channel asked for, with where in the trace each
    /// was asked.
    pub fn channel_calls(&self) -> &[(usize, Call)] {
        &self.calls
    }

    /// Every read of a platform asked for, with where in the trace each
    /// was asked.
    pub fn platform_calls(&self) -> &[(usize, PlatformCall)] {
        &self.platform_calls
    }

    /// Scripts the next socket not to open.
    pub fn next_socket_fails(&mut self, why: &str) {
        self.socket_failures.push_back(why.to_owned());
    }

    /// The sockets still open, oldest first.
    pub fn sockets_open(&self) -> Vec<EffectId> {
        self.sockets
            .iter()
            .filter(|(_, open)| **open)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Every envelope acknowledged so far, in order.
    pub fn acked(&self) -> &[String] {
        &self.acked
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
            // Answered a tick later, as a line or a write is: a probe takes
            // time in the real world, and what arrives in that time is
            // exactly what the race
            // `docs/decisions/0069-a-message-reaches-a-working-job.md`
            // closes is about.
            Effect::Probe { id, port, .. } => {
                let probed = self.probed(port);
                self.schedule(self.now + 1, Event::Probed { id, probed });
            }
            Effect::Request {
                id,
                method,
                url,
                headers,
                body,
                ..
            } => self.requested(id, method, url, headers, body),
            Effect::Connect { id, .. } => self.connected(id),
            Effect::Transmit { id, text } => self.transmitted(id, &text),
            Effect::Disconnect { id } => {
                let now = self.now;
                self.closes(id, now);
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

    pub fn posts(&self) -> &[(Place, String)] {
        &self.posts
    }

    /// Every room the platform was asked to make, with the name asked for.
    pub fn rooms(&self) -> &[(String, String)] {
        &self.rooms
    }

    /// Every description a room was given.
    pub fn described(&self) -> &[(String, String)] {
        &self.described
    }

    /// Everyone invited into a room.
    pub fn invited(&self) -> &[(String, String)] {
        &self.invited
    }

    /// Every room archived.
    pub fn archived(&self) -> &[String] {
        &self.archived
    }

    /// Every reaction put on a message, in order.
    pub fn reactions(&self) -> &[(String, String, Reaction)] {
        &self.reactions
    }

    /// The messages a reaction was put on, in order, for one reaction.
    pub fn reacted(&self, wanted: Reaction) -> Vec<String> {
        self.reactions
            .iter()
            .filter(|(_, _, reaction)| *reaction == wanted)
            .map(|(_, message, _)| message.clone())
            .collect()
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
