//! The two enumerations the instance and the world meet through, and what the
//! instance is born knowing.
//!
//! Everything here is a value. An [`Event`] is what the world tells the
//! instance and an [`Effect`] is what the instance asks of the world, and
//! neither can carry a call, a channel or a callback: a scenario's effects
//! are a trace that can be compared, and the instance can learn nothing from
//! making one. See
//! `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`.

use std::fmt;
use std::time::Duration;

use stageman_agent::Answer;
use stageman_core::{
    Agent, Channel, Handout, InstanceId, JobId, Kit, ProjectId, Secret, Speaking, Thread, Timestamp,
};

use crate::tunnel::Domain;

/// What the world seeds the instance's randomness with, once.
///
/// From the operating system in production and from the scenario in a test,
/// which is the whole difference between the two.
pub type Seed = [u8; 32];

/// What the world knows before the instance exists.
///
/// The facts the first decisions need and no later event could supply,
/// because absence is only visible in a complete listing: a working job with
/// *no* container is lost, and nothing per container could say so.
#[derive(Debug, Clone)]
pub struct Startup {
    /// Every container the runtime holds that this project started, running
    /// or not, with the labels it was given.
    pub containers: Vec<Container>,
    /// The domain this instance answers on, which a job is told so that what
    /// it shows can be reached.
    pub domain: Domain,
    /// The port the dashboard is actually bound to, for the same reason.
    pub serving: u16,
    /// What this build calls itself, for whoever asks the tools endpoint.
    pub build: String,
}

/// What identifies one request the world is waiting to answer.
///
/// Minted by the world, carried in and echoed back, and opaque here: the
/// instance decides nothing from its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// One container the runtime holds, as the world reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// Its name, which is what addresses it.
    pub name: String,
    /// Which instance started it, if its label says.
    pub instance: Option<InstanceId>,
    /// Which agent it was made for, if its label says.
    pub agent: Option<Agent>,
    /// Whether it is up right now.
    pub running: bool,
}

/// Who a turn belongs to.
///
/// A turn is taken by one job or by one project's foreman, and at most one
/// runs for each at a time, which is why a turn needs no identifier of its
/// own: the speaker is the key, and a completion for a speaker with no turn
/// in flight is one from before a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Speaker {
    /// The one agent a project's foreman thinks with.
    Foreman(ProjectId),
    /// One job's agent.
    Job(JobId),
}

/// What a timer the instance asked for was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// Time to ask which containers are still showing something.
    Settle,
}

/// What a credential presented to the tools endpoint entitles its bearer to.
///
/// Minted when a turn starts and forgotten when it ends, so a warrant from
/// before a crash names nothing. The thread is where anything the bearer says
/// lands: a job's for its whole life, a foreman's per turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warranted {
    /// Who holds it.
    pub speaker: Speaker,
    /// Where they speak, if anywhere.
    pub thread: Option<Thread>,
}

/// One message heard on a channel, as the world decoded it.
///
/// What routing needs and nothing else: where it was said, what identifies
/// it, the thread it was in if any, the words, and the two facts about the
/// speaker the rule in
/// `docs/decisions/0031-a-mention-is-what-makes-it-ours.md` turns on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Where it was said.
    pub address: String,
    /// What identifies this message, which is the thread a foreman answers
    /// in when the message is at the root.
    pub id: String,
    /// The thread it was in, if it was in one.
    pub thread: Option<String>,
    /// What was said, as the person wrote it.
    pub text: String,
    /// Whether it named this instance.
    pub mentions: bool,
    /// Whether this instance is what said it.
    pub from_us: bool,
}

/// One thing the world tells the instance.
///
/// Time appears only where a handler keeps it, which in this set is nowhere:
/// nothing here is recorded with a timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Answers [`Effect::Persist`]: the bytes reached the disk, or did not.
    ///
    /// Answered in the order the persists were asked for, which is the one
    /// ordering obligation the world has beyond answering only when done.
    Persisted {
        /// Why the write failed, if it did.
        outcome: Result<(), String>,
    },
    /// Answers [`Effect::RunTurn`]: the agent stopped, or could not be run.
    TurnEnded {
        /// Whose turn it was.
        speaker: Speaker,
        /// What the agent said and why it stopped, or why it could not.
        outcome: Result<Answer, String>,
    },
    /// Answers [`Effect::Probe`]: whether something is behind a job's tunnel.
    Probed {
        /// Which job's tunnel was probed.
        job: JobId,
        /// Whether anything answered behind it.
        answering: bool,
    },
    /// Answers [`Effect::ListRunning`]: the containers up right now.
    Listed {
        /// Every running container this project started, with its labels.
        running: Vec<Container>,
    },
    /// Answers [`Effect::Wake`].
    Woke {
        /// Which timer.
        timer: Timer,
    },
    /// Somebody said something on a channel this instance listens to.
    Heard {
        /// Which channel.
        channel: Channel,
        /// What was heard.
        message: Message,
    },
    /// Answers [`Effect::Inspect`]: whether a container exists, and for
    /// which agent it was made.
    Inspected {
        /// Its name.
        container: String,
        /// Whether the runtime holds it.
        present: bool,
        /// Which agent it was made for, if it is there and its label says.
        agent: Option<Agent>,
    },
    /// An agent called the tools endpoint. Answered by
    /// [`Effect::ToolAnswered`], in this step or a later one.
    ///
    /// The whole request, because everything the endpoint decides is
    /// instance state: whether the credential names anyone, what its bearer
    /// may be offered, and what each tool does.
    ToolCalled {
        /// What the world is waiting to answer.
        id: RequestId,
        /// When it arrived. Kept on a job the call creates.
        at: Timestamp,
        /// Whether the caller is on this machine, which is the only place a
        /// container of ours can be.
        nearby: bool,
        /// The bearer credential presented, if any.
        bearer: Option<String>,
        /// The request body, as JSON.
        body: serde_json::Value,
    },
    /// Answers [`Effect::OpenThread`]: where a job's conversation happens,
    /// or why it could not be opened.
    ThreadOpened {
        /// Whose thread.
        job: JobId,
        /// The thread, or why not.
        outcome: Result<Thread, String>,
    },
    /// Answers [`Effect::Post`]: whether the platform took the message.
    Posted {
        /// Which request it was said for.
        request: RequestId,
        /// Why not, if not.
        outcome: Result<(), String>,
    },
}

/// One thing the instance asks of the world.
///
/// The doc comment on each says whether it is answered, and by what. An
/// unanswered effect's failure is the world's to log.
#[derive(Clone)]
pub enum Effect {
    /// Write these bytes over the instance's file, atomically. Answered by
    /// [`Event::Persisted`], in order.
    Persist {
        /// The sealed snapshot, whole.
        bytes: Vec<u8>,
    },
    /// Run one turn of an agent. Answered by [`Event::TurnEnded`].
    RunTurn {
        /// Whose turn.
        speaker: Speaker,
        /// Starting a session or continuing one.
        run: Run,
    },
    /// Ask whether anything is behind a job's tunnel. Answered by
    /// [`Event::Probed`].
    Probe {
        /// Which job.
        job: JobId,
    },
    /// Ask which containers are up. Answered by [`Event::Listed`].
    ListRunning,
    /// Ask whether a container exists and for which agent it was made.
    /// Answered by [`Event::Inspected`].
    ///
    /// Asked before every foreman's turn rather than remembered, because a
    /// container is the truth about whether a session exists: a foreman that
    /// believed it had one and did not would fail every turn until somebody
    /// looked.
    Inspect {
        /// Its name.
        container: String,
    },
    /// Stop a container, keeping it and everything in it. Unanswered.
    Halt {
        /// Its name.
        container: String,
    },
    /// Remove a container and everything in it. Unanswered.
    Discard {
        /// Its name.
        container: String,
    },
    /// Reclaim the images nothing needs any more. Unanswered.
    Reclaim,
    /// Wake the instance later. Answered by [`Event::Woke`].
    Wake {
        /// How long from now.
        after: Duration,
        /// What for.
        timer: Timer,
    },
    /// Post on a channel, in a thread, on the instance's own behalf.
    /// Unanswered: this is a notice about an outcome, and the outcome does
    /// not change because the notice of it did not arrive.
    Say {
        /// The channel and the credential that posts on it.
        speaking: Speaking,
        /// Where in it.
        thread: Thread,
        /// What.
        text: String,
    },
    /// Answer a request the world is waiting on. Unanswered.
    ToolAnswered {
        /// Which request.
        id: RequestId,
        /// The HTTP status to answer with.
        status: u16,
        /// The body, if the status carries one.
        body: Option<serde_json::Value>,
    },
    /// Open the thread a job's conversation happens in, by posting its
    /// announcement at the root of the channel. Answered by
    /// [`Event::ThreadOpened`].
    OpenThread {
        /// Whose thread.
        job: JobId,
        /// The channel and the credential that posts on it.
        speaking: Speaking,
        /// What the thread hangs from.
        announcement: String,
    },
    /// Post on a channel on an agent's behalf. Answered by [`Event::Posted`],
    /// because the agent is told whether it was heard.
    Post {
        /// Which request is waiting on it.
        request: RequestId,
        /// The channel and the credential that posts on it.
        speaking: Speaking,
        /// Where in it.
        thread: Thread,
        /// What.
        text: String,
    },
    /// Listen on a project's channel for what people say. Unanswered: what
    /// is heard arrives as events of its own.
    Listen {
        /// Whose channel.
        project: ProjectId,
        /// What opens the event stream. Never enters a container.
        opening: Secret,
        /// What posts, and what asks the platform who this instance is.
        speaking: Speaking,
    },
}

/// Whether a turn starts a session or continues the one its container holds.
#[derive(Clone)]
pub enum Run {
    /// Make the container and the session, and put the first question.
    Begin {
        /// The container to make, named before it exists.
        container: String,
        /// Exactly what the agent may see.
        handout: Handout,
        /// Which instance made it, for the label.
        instance: InstanceId,
        /// What the agent presents to the tools endpoint.
        warrant: Secret,
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
        warrant: Secret,
        /// What the resumed agent is told.
        text: String,
    },
}

impl fmt::Debug for Effect {
    /// The trace's rendering. Written out rather than derived for one variant:
    /// the sealed bytes are ciphertext, and a trace line holding a whole file
    /// would say nothing a reader could use.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persist { bytes } => f
                .debug_struct("Persist")
                .field("bytes", &bytes.len())
                .finish(),
            Self::RunTurn { speaker, run } => f
                .debug_struct("RunTurn")
                .field("speaker", speaker)
                .field("run", run)
                .finish(),
            Self::Probe { job } => f.debug_struct("Probe").field("job", job).finish(),
            Self::ListRunning => f.write_str("ListRunning"),
            Self::Inspect { container } => f
                .debug_struct("Inspect")
                .field("container", container)
                .finish(),
            Self::Halt { container } => f
                .debug_struct("Halt")
                .field("container", container)
                .finish(),
            Self::Discard { container } => f
                .debug_struct("Discard")
                .field("container", container)
                .finish(),
            Self::Reclaim => f.write_str("Reclaim"),
            Self::Wake { after, timer } => f
                .debug_struct("Wake")
                .field("after", after)
                .field("timer", timer)
                .finish(),
            Self::Say {
                speaking,
                thread,
                text,
            } => f
                .debug_struct("Say")
                .field("address", &speaking.address)
                .field("thread", thread)
                .field("text", text)
                .finish(),
            Self::Listen {
                project, speaking, ..
            } => f
                .debug_struct("Listen")
                .field("project", project)
                .field("address", &speaking.address)
                .finish(),
            Self::ToolAnswered { id, status, body } => f
                .debug_struct("ToolAnswered")
                .field("id", id)
                .field("status", status)
                .field("body", body)
                .finish(),
            Self::OpenThread {
                job,
                speaking,
                announcement,
            } => f
                .debug_struct("OpenThread")
                .field("job", job)
                .field("address", &speaking.address)
                .field("announcement", announcement)
                .finish(),
            Self::Post {
                request,
                speaking,
                thread,
                text,
            } => f
                .debug_struct("Post")
                .field("request", request)
                .field("address", &speaking.address)
                .field("thread", thread)
                .field("text", text)
                .finish(),
        }
    }
}

impl fmt::Debug for Run {
    /// Names what a turn is given and never a credential: the handout
    /// redacts itself, and the warrant is one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Begin {
                container,
                handout,
                instance,
                kickoff,
                ..
            } => f
                .debug_struct("Begin")
                .field("container", container)
                .field("handout", handout)
                .field("instance", instance)
                .field("kickoff", kickoff)
                .finish(),
            Self::Resume {
                container,
                kit,
                text,
                ..
            } => f
                .debug_struct("Resume")
                .field("container", container)
                .field("kit", kit)
                .field("text", text)
                .finish(),
        }
    }
}
