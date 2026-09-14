//! What the instance and the world say to each other.
//!
//! An [`Event`] is what the world tells the instance and an [`Effect`] is
//! what the instance asks of the world. Both are plain data: the mechanisms a
//! process reaches the outside through — a file, a process run once, a
//! process kept open and spoken to line by line, a request served and a
//! request made, a socket opened and spoken over, a port probed, a timer,
//! its own standard output, its own exit — and one hole an [`App`] fills
//! with its own events and effects, which the world never interprets and
//! hands to whatever the application supplied to perform them. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! **Everything here serialises in full and formats not at all.** A scenario
//! is a file of events and a trace is a file of effects, and replaying one
//! needs every byte, credentials included — which are fake in every file a
//! test writes. Neither enumeration implements `Debug` or `Display`, so that
//! nothing can format one into a log by accident; what the world logs is the
//! kind, from [`Named`]. That absence is a rule rather than an omission, and
//! `docs/conventions.md` §4 says why.

pub mod scenario;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// What the world seeds the instance's randomness with, once.
///
/// From the operating system in production and from the scenario in a test,
/// which is the whole difference between the two. Never from anything
/// guessable: every unguessable value the instance mints comes from it.
pub type Seed = [u8; 32];

/// The environment the process was given, as the instance is constructed
/// with it.
///
/// What it was actually given and nothing synthesised, so that a scenario's
/// author knows exactly what to write: the application's own facts arrive as
/// the application's own events.
pub type Environment = BTreeMap<String, String>;

/// Something whose kind can be named, for a log line.
///
/// The one thing the world may say about an event or an effect without
/// formatting it, and so the one thing it does say.
pub trait Named {
    /// What kind of thing this is, as one word.
    fn kind(&self) -> &'static str;
}

/// What an application adds to the vocabulary.
///
/// The world never looks inside either type. It carries an event of the
/// application's to the instance like any other, and hands an effect of the
/// application's to whatever the entry point supplied for them.
pub trait App: Send + Sync + 'static {
    /// What the application's own world tells the instance.
    type Event: Serialize + DeserializeOwned + Clone + PartialEq + Named + Send + 'static;
    /// What the instance asks of the application's own world.
    type Effect: Serialize + DeserializeOwned + Clone + PartialEq + Named + Send + 'static;
}

/// What identifies a request the world is holding open for an answer.
///
/// Minted by the world rather than by the instance, which is the opposite of
/// every other identifier here and for the reason they go the other way:
/// whoever holds the thing open is whoever can name it, and a request is a
/// connection the world accepted. The instance is told one arrived and
/// answers with the identifier it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequestId(pub u64);

/// A request, as much of it as arrives before anything is decided.
///
/// The head and no body: a body is read only where the answer depends on
/// one, and then only up to a limit somebody set.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arrival {
    /// What it asks to do.
    pub method: String,
    /// What it asks about, query and all.
    pub path: String,
    /// Its headers, by lowercased name, joined with commas where a name
    /// arrived more than once — which is what reading them as a map costs,
    /// and what makes a request a value a scenario can compare.
    pub headers: BTreeMap<String, String>,
    /// Who sent it, as an address.
    pub peer: String,
    /// When it arrived, in milliseconds since the epoch.
    ///
    /// Stamped by the world, because a generic world cannot know which
    /// handlers keep a time and which ignore it.
    pub at: u64,
}

/// What to do about a request the world is holding open.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// Answer it now, and close the matter.
    Respond {
        /// The status.
        status: u16,
        /// What to send with it, by header name.
        headers: BTreeMap<String, String>,
        /// The body.
        body: Bytes,
    },
    /// Forward it to a port on this machine's loopback, whole, and forward
    /// back whatever answers — including an upgrade, so that a websocket
    /// through this is still a websocket.
    Proxy {
        /// Where.
        port: u16,
        /// What to say if nothing answers there, as the body of a gateway
        /// refusal. Carried rather than composed here, because what it
        /// means for nothing to answer is the deciding half's to know.
        refused: Bytes,
    },
    /// Read its body and hand it back, then ask again.
    ///
    /// Bounded, because whoever sent it is not always somebody trusted and
    /// a body held whole is held in memory.
    Read {
        /// How many bytes to take before giving up on it.
        limit: usize,
    },
}

/// What identifies an effect the instance is waiting to have answered.
///
/// Minted by the instance, from a counter, so that it is as deterministic as
/// everything else it does; carried on the effect and echoed on the event
/// that answers it. Opaque to the world.
///
/// It has to be unique among the effects still *waiting*, and nothing more
/// than that: an identifier is spent the moment its answer arrives, and a
/// handful are outstanding at a time. So the counter is cyclic rather than
/// bounded, and there is no exhausting it — coming round would take more
/// effects than a run can perform, and even then it could only meet an
/// identifier answered long before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EffectId(pub u64);

/// How a process that was run once came to an end.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finished {
    /// It ran and exited.
    Exited {
        /// Its exit status, or none if a signal ended it.
        status: Option<i32>,
        /// Everything it wrote to its standard output.
        stdout: Bytes,
        /// Everything it wrote to its standard error.
        stderr: Bytes,
    },
    /// There is no such program, which is an answer of its own: it is how a
    /// candidate for something is found to be absent.
    NotFound,
    /// It could not be started for some other reason.
    Failed(String),
}

/// How a process that was kept open came to an end.
///
/// Its output is not here, because it already arrived line by line; what is
/// here is what a process run once would have answered with less that: the
/// status, and what it wrote to its standard error, which is where a process
/// that fails says why.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ended {
    /// It ran and exited, on its own or because it was closed.
    Exited {
        /// Its exit status, or none if a signal ended it — which is what
        /// being closed looks like.
        status: Option<i32>,
        /// What it wrote to its standard error, from the beginning and up to
        /// a bound the world keeps: a process that fails says so at once,
        /// and one that runs for an hour says a great deal that is not that.
        stderr: Bytes,
    },
    /// There is no such program.
    NotFound,
    /// It could not be started, or could not be waited on, for some other
    /// reason.
    Failed(String),
}

/// How a request made once came to an end.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Responded {
    /// It was answered, with whatever status: a refusal is an answer, and
    /// what a status means is the deciding half's to know.
    Answered {
        /// The status.
        status: u16,
        /// Its headers, by lowercased name, joined with commas where a name
        /// arrived more than once — as a served request's are.
        headers: BTreeMap<String, String>,
        /// The whole body.
        body: Bytes,
    },
    /// It got no answer: the name did not resolve, nothing accepted, the
    /// connection broke, or the far side never finished.
    Failed(String),
}

/// How a socket came to an end.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Disconnected {
    /// The far side ended it, with a closing handshake or without one — a
    /// peer that vanished is the same ending seen one layer lower, and which
    /// arrives is a matter of timing rather than of anything worth telling
    /// apart.
    Closed,
    /// It could not be opened, or it broke: the protocol was violated, or
    /// the transport failed in a way that is not a peer going away.
    Failed(String),
}

/// What a port did when it was probed: connected to and read from once,
/// with nothing written.
///
/// The observation and not its meaning. What each of these says about
/// whether anything is behind the port is the deciding half's to know —
/// `docs/decisions/0047-a-tunnel-answers-only-when-something-behind-it-does.md`
/// measured it for a container runtime's proxy, and a world that knows no
/// runtime cannot know that a connection accepted and then closed means
/// nothing was there. Four rather than a yes or no, so that a scenario can
/// answer with exactly what a runtime was measured to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Probed {
    /// Nothing accepted the connection.
    Refused,
    /// It accepted, and closed before saying anything.
    Closed,
    /// It accepted, and said something.
    Spoke,
    /// It accepted, and said nothing for as long as it was given.
    Silent,
}

/// One thing the world tells the instance.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Event<A: App> {
    /// Answers [`Effect::Read`]: the file's contents, or that there is no
    /// such file, or why it could not be read. Absent is its own answer
    /// rather than a failure, because a first run is not a failure.
    Read {
        /// Which read.
        id: EffectId,
        /// The contents, none for a file that is not there, or the reason.
        contents: Result<Option<Bytes>, String>,
    },
    /// Answers [`Effect::Write`]: the bytes reached the disk, or did not.
    Written {
        /// Which write.
        id: EffectId,
        /// Why not, if not.
        outcome: Result<(), String>,
    },
    /// Answers [`Effect::Run`]: how the process came to an end.
    Ran {
        /// Which run.
        id: EffectId,
        /// How.
        finished: Finished,
    },
    /// Answers [`Effect::Bind`]: the address was taken, or could not be.
    Bound {
        /// Which bind.
        id: EffectId,
        /// The port it got, or why it got none.
        outcome: Result<u16, String>,
    },
    /// A request arrived on a bound address and is being held open for an
    /// answer. Answered by [`Effect::Answer`].
    Arrived {
        /// Which listener it arrived on.
        listener: EffectId,
        /// What answers it.
        id: RequestId,
        /// The request, head only.
        request: Arrival,
    },
    /// Answers [`Answer::Read`]: the body of a request that was asked for.
    Body {
        /// Which request.
        id: RequestId,
        /// Its body, or why it could not be read — which includes being
        /// longer than the limit it was asked for under.
        outcome: Result<Bytes, String>,
    },
    /// Answers [`Effect::Wake`].
    Woke {
        /// Which wake.
        id: EffectId,
    },
    /// A process kept open wrote one line to its standard output.
    ///
    /// Every line arrives, in order, and every one arrives before
    /// [`Event::Ended`] for the same process.
    Line {
        /// Which process, by the identifier it was opened under.
        id: EffectId,
        /// The line, without its newline.
        line: String,
    },
    /// A process kept open came to an end, on its own or because it was
    /// closed. Answers [`Effect::Open`], eventually, and [`Effect::Close`].
    Ended {
        /// Which process, by the identifier it was opened under.
        id: EffectId,
        /// How.
        ended: Ended,
    },
    /// Answers [`Effect::Probe`]: what the port did.
    Probed {
        /// Which probe.
        id: EffectId,
        /// What it did.
        probed: Probed,
    },
    /// Answers [`Effect::Request`]: how the request came to an end.
    Responded {
        /// Which request.
        id: EffectId,
        /// How.
        responded: Responded,
        /// When, in milliseconds since the epoch, stamped by the world for
        /// the same reason a served request is: a failure to reach a
        /// platform is what a gap with no connection can begin with.
        at: u64,
    },
    /// A socket received one text frame.
    ///
    /// Every text frame arrives, in order, and every one arrives before
    /// [`Event::Disconnected`] for the same socket. Text only: nothing here
    /// speaks a binary frame, so one that arrives is dropped where it does.
    Frame {
        /// Which socket, by the identifier it was connected under.
        id: EffectId,
        /// The frame's text.
        text: String,
        /// When it arrived, in milliseconds since the epoch, stamped by the
        /// world for the same reason a served request is.
        at: u64,
    },
    /// A socket came to an end, on its own or because it was disconnected.
    /// Answers [`Effect::Connect`], eventually, and [`Effect::Disconnect`].
    Disconnected {
        /// Which socket, by the identifier it was connected under.
        id: EffectId,
        /// How.
        disconnected: Disconnected,
        /// When, in milliseconds since the epoch: what a gap with no
        /// connection is measured from.
        at: u64,
    },
    /// Something of the application's own.
    App(A::Event),
}

// By hand rather than derived, because a derive would demand the bounds of
// the application marker itself, which is a type with nothing in it.
impl<A: App> Clone for Event<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Read { id, contents } => Self::Read {
                id: *id,
                contents: contents.clone(),
            },
            Self::Written { id, outcome } => Self::Written {
                id: *id,
                outcome: outcome.clone(),
            },
            Self::Ran { id, finished } => Self::Ran {
                id: *id,
                finished: finished.clone(),
            },
            Self::Bound { id, outcome } => Self::Bound {
                id: *id,
                outcome: outcome.clone(),
            },
            Self::Arrived {
                listener,
                id,
                request,
            } => Self::Arrived {
                listener: *listener,
                id: *id,
                request: request.clone(),
            },
            Self::Body { id, outcome } => Self::Body {
                id: *id,
                outcome: outcome.clone(),
            },
            Self::Woke { id } => Self::Woke { id: *id },
            Self::Line { id, line } => Self::Line {
                id: *id,
                line: line.clone(),
            },
            Self::Ended { id, ended } => Self::Ended {
                id: *id,
                ended: ended.clone(),
            },
            Self::Probed { id, probed } => Self::Probed {
                id: *id,
                probed: *probed,
            },
            Self::Responded { id, responded, at } => Self::Responded {
                id: *id,
                responded: responded.clone(),
                at: *at,
            },
            Self::Frame { id, text, at } => Self::Frame {
                id: *id,
                text: text.clone(),
                at: *at,
            },
            Self::Disconnected {
                id,
                disconnected,
                at,
            } => Self::Disconnected {
                id: *id,
                disconnected: disconnected.clone(),
                at: *at,
            },
            Self::App(event) => Self::App(event.clone()),
        }
    }
}

impl<A: App> PartialEq for Event<A> {
    fn eq(&self, other: &Self) -> bool {
        // Through the one representation both sides share, which is what a
        // scenario compares anyway — and field by field is where a
        // comparison silently starts agreeing about two different things.
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl<A: App> Named for Event<A> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Read { .. } => "Read",
            Self::Written { .. } => "Written",
            Self::Ran { .. } => "Ran",
            Self::Bound { .. } => "Bound",
            Self::Arrived { .. } => "Arrived",
            Self::Body { .. } => "Body",
            Self::Woke { .. } => "Woke",
            Self::Line { .. } => "Line",
            Self::Ended { .. } => "Ended",
            Self::Probed { .. } => "Probed",
            Self::Responded { .. } => "Responded",
            Self::Frame { .. } => "Frame",
            Self::Disconnected { .. } => "Disconnected",
            Self::App(event) => event.kind(),
        }
    }
}

/// One thing the instance asks of the world.
///
/// The doc comment on each says whether it is answered, and by what. An
/// unanswered effect's failure is the world's to log.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Effect<A: App> {
    /// Read a file whole. Answered by [`Event::Read`].
    Read {
        /// Which read, on the answer.
        id: EffectId,
        /// The file.
        path: PathBuf,
    },
    /// Write a file whole and atomically — a temporary beside it, flushed,
    /// then renamed over it — making its directory if need be. Answered by
    /// [`Event::Written`], in the order writes were asked for.
    Write {
        /// Which write, on the answer.
        id: EffectId,
        /// The file.
        path: PathBuf,
        /// Its whole new contents.
        bytes: Bytes,
        /// Whether nobody but this user may read it, where the platform can
        /// say so. A property rather than a mode, because a mode is a
        /// mechanism.
        private: bool,
    },
    /// Run a program once, to its end. Answered by [`Event::Ran`].
    Run {
        /// Which run, on the answer.
        id: EffectId,
        /// The program, by path.
        program: PathBuf,
        /// Its arguments, none being a legitimate number of them.
        arguments: Vec<String>,
        /// Exactly the environment it is given, and nothing inherited.
        environment: Environment,
        /// What it is given on its standard input, then end of file.
        stdin: Option<Bytes>,
    },
    /// Start a program and keep it open: every line it writes to its
    /// standard output is an [`Event::Line`], every [`Effect::Send`] is a
    /// line on its standard input, and how it ends is an [`Event::Ended`].
    ///
    /// Not answered by anything of its own. A program that could not be
    /// started ends at once, distinctly, and one that could is spoken to
    /// from the next effect on — so the lines that open a conversation are
    /// asked for in the same step as the process.
    Open {
        /// Which process, on every line and on its end.
        id: EffectId,
        /// The program, by path.
        program: PathBuf,
        /// Its arguments.
        arguments: Vec<String>,
        /// Exactly the environment it is given, and nothing inherited.
        environment: Environment,
    },
    /// Write one line to a process kept open, newline included by the world.
    /// Unanswered: a process that has ended is not written to, and its end
    /// is the event that says so.
    Send {
        /// Which process.
        id: EffectId,
        /// The line, without its newline.
        line: String,
    },
    /// End a process kept open: its standard input is closed and it is
    /// killed, and its end arrives as [`Event::Ended`] like any other.
    ///
    /// Both at once rather than the first and then the second after a grace,
    /// because a grace is a wait nothing here can decide the length of, and
    /// the process on the far side of the pipe is somebody else's — see
    /// `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`,
    /// which measured that a closed pipe is what ends an agent.
    Close {
        /// Which process.
        id: EffectId,
    },
    /// Wake the instance later. Answered by [`Event::Woke`].
    Wake {
        /// Which wake, on the answer.
        id: EffectId,
        /// How long from now.
        after: Duration,
    },
    /// Take an address, so that requests arriving on it are events.
    ///
    /// Answered with the port it got, which is how a port of zero — give me
    /// whichever is free — becomes a port anything can be told about.
    Bind {
        /// What answers it.
        id: EffectId,
        /// The address, as a person would type it.
        address: String,
    },
    /// What to do about a request that arrived. Unanswered, except by
    /// [`Event::Body`] when what was asked for is the body.
    Answer {
        /// Which request.
        id: RequestId,
        /// What to do about it.
        answer: Answer,
    },
    /// Make one request and read its whole answer. Answered by
    /// [`Event::Responded`].
    ///
    /// The method, the address, the headers and the body are all the
    /// deciding half's: what is asked of whom, and what an answer means, are
    /// meanings, and this is only the asking.
    Request {
        /// Which request, on the answer.
        id: EffectId,
        /// The method, as the protocol spells it.
        method: String,
        /// Where, scheme and all.
        url: String,
        /// The headers, by name.
        headers: BTreeMap<String, String>,
        /// The body, if it has one.
        body: Option<Bytes>,
        /// How long the whole exchange is given before it is taken to have
        /// failed. The deciding half's to set, because how long an answer
        /// is worth waiting for is a question about what is being asked.
        within: Duration,
    },
    /// Open a socket to an address and keep it open: every text frame it
    /// receives is an [`Event::Frame`], every [`Effect::Transmit`] is a
    /// frame sent on it, and how it ends is an [`Event::Disconnected`].
    ///
    /// Not answered by anything of its own, like a process kept open: one
    /// that could not be opened ends at once, distinctly, and one that could
    /// is heard from as frames arrive. Which is why the first thing a far
    /// side says is what tells the deciding half it is connected.
    Connect {
        /// Which socket, on every frame and on its end.
        id: EffectId,
        /// Where, scheme and all.
        url: String,
    },
    /// Send one text frame on a socket. Unanswered: a socket that has ended
    /// is not sent to, and its end is the event that says so.
    Transmit {
        /// Which socket.
        id: EffectId,
        /// The frame's text.
        text: String,
    },
    /// End a socket: a closing handshake is begun and the connection let go
    /// of, and its end arrives as [`Event::Disconnected`] like any other.
    Disconnect {
        /// Which socket.
        id: EffectId,
    },
    /// Connect to a port on this machine's loopback and read from it once,
    /// writing nothing, to learn what is there. Answered by
    /// [`Event::Probed`] with what the port did.
    ///
    /// Read as well as connected to, because connecting alone proves
    /// nothing where something accepts on another's behalf; and nothing
    /// written, because whatever is behind the port is somebody else's and
    /// bytes this process made up are not its to send.
    Probe {
        /// Which probe, on the answer.
        id: EffectId,
        /// The port.
        port: u16,
        /// How long to give it to say something, or to close, before it is
        /// taken to be holding the connection open in silence. The deciding
        /// half's to set, because what the far side is and how long it
        /// takes to admit to being empty are things only it knows.
        within: Duration,
    },
    /// Write to the process's standard output, which is where whoever
    /// started it is reading. Unanswered.
    Print {
        /// What to write, whole; a trailing newline is the instance's to
        /// include.
        text: String,
    },
    /// Stop the process, with the reason as its last word. Unanswered, since
    /// there is nobody left to answer.
    Exit {
        /// The reason, for whoever started it.
        message: String,
    },
    /// Something of the application's own.
    App(A::Effect),
}

impl<A: App> Clone for Effect<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Read { id, path } => Self::Read {
                id: *id,
                path: path.clone(),
            },
            Self::Write {
                id,
                path,
                bytes,
                private,
            } => Self::Write {
                id: *id,
                path: path.clone(),
                bytes: bytes.clone(),
                private: *private,
            },
            Self::Run {
                id,
                program,
                arguments,
                environment,
                stdin,
            } => Self::Run {
                id: *id,
                program: program.clone(),
                arguments: arguments.clone(),
                environment: environment.clone(),
                stdin: stdin.clone(),
            },
            Self::Open {
                id,
                program,
                arguments,
                environment,
            } => Self::Open {
                id: *id,
                program: program.clone(),
                arguments: arguments.clone(),
                environment: environment.clone(),
            },
            Self::Send { id, line } => Self::Send {
                id: *id,
                line: line.clone(),
            },
            Self::Close { id } => Self::Close { id: *id },
            Self::Wake { id, after } => Self::Wake {
                id: *id,
                after: *after,
            },
            Self::Bind { id, address } => Self::Bind {
                id: *id,
                address: address.clone(),
            },
            Self::Answer { id, answer } => Self::Answer {
                id: *id,
                answer: answer.clone(),
            },
            Self::Request {
                id,
                method,
                url,
                headers,
                body,
                within,
            } => Self::Request {
                id: *id,
                method: method.clone(),
                url: url.clone(),
                headers: headers.clone(),
                body: body.clone(),
                within: *within,
            },
            Self::Connect { id, url } => Self::Connect {
                id: *id,
                url: url.clone(),
            },
            Self::Transmit { id, text } => Self::Transmit {
                id: *id,
                text: text.clone(),
            },
            Self::Disconnect { id } => Self::Disconnect { id: *id },
            Self::Probe { id, port, within } => Self::Probe {
                id: *id,
                port: *port,
                within: *within,
            },
            Self::Print { text } => Self::Print { text: text.clone() },
            Self::Exit { message } => Self::Exit {
                message: message.clone(),
            },
            Self::App(effect) => Self::App(effect.clone()),
        }
    }
}

impl<A: App> PartialEq for Effect<A> {
    fn eq(&self, other: &Self) -> bool {
        // Through the one representation both sides share, which is what a
        // scenario compares anyway.
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl<A: App> Named for Effect<A> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Read { .. } => "Read",
            Self::Write { .. } => "Write",
            Self::Run { .. } => "Run",
            Self::Open { .. } => "Open",
            Self::Send { .. } => "Send",
            Self::Close { .. } => "Close",
            Self::Wake { .. } => "Wake",
            Self::Bind { .. } => "Bind",
            Self::Answer { .. } => "Answer",
            Self::Request { .. } => "Request",
            Self::Connect { .. } => "Connect",
            Self::Transmit { .. } => "Transmit",
            Self::Disconnect { .. } => "Disconnect",
            Self::Probe { .. } => "Probe",
            Self::Print { .. } => "Print",
            Self::Exit { .. } => "Exit",
            Self::App(effect) => effect.kind(),
        }
    }
}

/// Something the world steps: constructed from a seed and an environment,
/// then one event in and effects out, for as long as the process runs.
///
/// The only way anything reaches the deciding half, and the only way it
/// answers. The world calls these and nothing else.
pub trait Deciding: Sized {
    /// The application whose events and effects this speaks.
    type App: App;

    /// Which platform this build was made for.
    ///
    /// The third fact that exists before anything happens, and the only one
    /// that is a property of the binary rather than of the run. It is handed
    /// over rather than read so that nothing inside branches on the machine
    /// it was compiled for: a recording made on one platform is replayed on
    /// another by handing the replay the target the file names. Which
    /// platforms there are is the application's to say, which is why this is
    /// a hole rather than a set named here.
    type Target: Serialize + DeserializeOwned + Clone;

    /// Constructs the deciding half from the facts that exist before
    /// anything happens, and answers with what it asks for first.
    ///
    /// Everything else — a key, a file, what is installed — it asks for
    /// through effects and learns from their answers.
    fn boot(
        seed: Seed,
        environment: Environment,
        target: Self::Target,
    ) -> (Self, Vec<Effect<Self::App>>);

    /// Handles one event and answers with what to do about it.
    fn step(&mut self, event: Event<Self::App>) -> Vec<Effect<Self::App>>;

    /// Everything it holds, as a value a scenario compares and a reviewer
    /// reads, credentials in the clear.
    fn snapshot(&self) -> serde_json::Value;
}

/// Bytes that cross the vocabulary, readable where they are text.
///
/// Serialised as the text they spell when they are valid UTF-8, and as hex
/// otherwise, so that a file's contents in a scenario read as the file and
/// a trace never carries an array of numbers. Compared and stored as the
/// bytes themselves.
#[derive(Clone, PartialEq, Eq)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    /// Wraps bytes.
    #[must_use]
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// The bytes, owned.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }

    /// How many.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The bytes as text, where they are text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<String> for Bytes {
    fn from(text: String) -> Self {
        Self(text.into_bytes())
    }
}

/// The two spellings bytes take on the wire.
#[derive(Serialize, Deserialize)]
enum Spelled {
    #[serde(rename = "text")]
    Text(String),
    #[serde(rename = "hex")]
    Hex(String),
}

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let spelled = std::str::from_utf8(&self.0).map_or_else(
            |_| {
                use std::fmt::Write as _;
                Spelled::Hex(self.0.iter().fold(String::new(), |mut hex, byte| {
                    // Writing to a string cannot fail, so the result says nothing.
                    let _ = write!(hex, "{byte:02x}");
                    hex
                }))
            },
            |text| Spelled::Text(text.to_owned()),
        );
        spelled.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Spelled::deserialize(deserializer)? {
            Spelled::Text(text) => Ok(Self(text.into_bytes())),
            Spelled::Hex(hex) => {
                let digits: Vec<u8> = hex.as_bytes().to_vec();
                if !digits.len().is_multiple_of(2) {
                    return Err(serde::de::Error::custom("hex with an odd number of digits"));
                }
                digits
                    .chunks(2)
                    .map(|pair| {
                        let text = std::str::from_utf8(pair)
                            .map_err(|_| serde::de::Error::custom("hex that is not ASCII"))?;
                        u8::from_str_radix(text, 16).map_err(|_| {
                            serde::de::Error::custom("hex with a digit that is not one")
                        })
                    })
                    .collect::<Result<Vec<u8>, D::Error>>()
                    .map(Self)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod doorbell {
    //! The smallest application there is, for the tests here: a bell that
    //! counts its rings and asks the world to print each one.

    use super::{App, Deciding, Effect, EffectId, Environment, Event, Named, Seed};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Told {
        Rang { times: u32 },
    }

    impl Named for Told {
        fn kind(&self) -> &'static str {
            match self {
                Self::Rang { .. } => "Rang",
            }
        }
    }

    pub struct Doorbell;

    impl App for Doorbell {
        type Event = Told;
        type Effect = Told;
    }

    /// Counts rings, and reads a file of its own name at boot to know how
    /// many it had heard before.
    #[derive(Serialize)]
    pub struct Bell {
        pub heard: u32,
        pub next: u64,
        pub booted: bool,
    }

    impl Deciding for Bell {
        type App = Doorbell;
        type Target = ();

        fn boot(
            _seed: Seed,
            environment: Environment,
            (): Self::Target,
        ) -> (Self, Vec<Effect<Doorbell>>) {
            let path = environment
                .get("BELL")
                .map_or_else(|| "bell".to_owned(), Clone::clone);
            (
                Self {
                    heard: 0,
                    next: 2,
                    booted: false,
                },
                vec![Effect::Read {
                    id: EffectId(1),
                    path: path.into(),
                }],
            )
        }

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "a bell that counted past its type should fail this crate's tests rather than clamp"
        )]
        fn step(&mut self, event: Event<Doorbell>) -> Vec<Effect<Doorbell>> {
            match event {
                Event::Read { contents, .. } => {
                    self.heard = contents
                        .ok()
                        .flatten()
                        .and_then(|bytes| bytes.as_text().and_then(|text| text.parse().ok()))
                        .unwrap_or(0);
                    self.booted = true;
                    Vec::new()
                }
                Event::App(Told::Rang { times }) => {
                    self.heard += times;
                    let id = EffectId(self.next);
                    self.next += 1;
                    vec![
                        Effect::Print {
                            text: format!("rang {times}, heard {} in all\n", self.heard),
                        },
                        Effect::Write {
                            id,
                            path: "bell".into(),
                            bytes: self.heard.to_string().into(),
                            private: false,
                        },
                    ]
                }
                Event::Written { .. }
                | Event::Ran { .. }
                | Event::Woke { .. }
                | Event::Bound { .. }
                | Event::Arrived { .. }
                | Event::Body { .. }
                | Event::Line { .. }
                | Event::Ended { .. }
                | Event::Probed { .. }
                | Event::Responded { .. }
                | Event::Frame { .. }
                | Event::Disconnected { .. } => Vec::new(),
            }
        }

        fn snapshot(&self) -> serde_json::Value {
            serde_json::to_value(self).expect("a bell serialises")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::doorbell::{Doorbell, Told};
    use super::{
        Answer, Arrival, Bytes, Disconnected, Effect, EffectId, Ended, Event, Finished, Named,
        Probed, RequestId, Responded,
    };

    /// The hole carries the application's own, and the kind is what a log
    /// may say.
    #[test]
    fn an_applications_own_event_crosses_whole_and_names_its_kind() {
        let event: Event<Doorbell> = Event::App(Told::Rang { times: 2 });
        let served = serde_json::to_string(&event).expect("it serialises");
        assert_eq!(served, r#"{"App":{"Rang":{"times":2}}}"#);
        let back: Event<Doorbell> = serde_json::from_str(&served).expect("and back");
        assert!(back == event);
        assert_eq!(event.kind(), "Rang");

        let effect: Effect<Doorbell> = Effect::App(Told::Rang { times: 1 });
        assert_eq!(effect.kind(), "Rang");
        let served = serde_json::to_string(&effect).expect("it serialises");
        let back: Effect<Doorbell> = serde_json::from_str(&served).expect("and back");
        assert!(back == effect);
    }

    /// Every mechanism crosses whole and names its kind.
    #[test]
    fn every_mechanism_crosses_whole_and_names_its_kind() {
        let effects: Vec<Effect<Doorbell>> = vec![
            Effect::Read {
                id: EffectId(1),
                path: "/etc/hostname".into(),
            },
            Effect::Write {
                id: EffectId(2),
                path: "/tmp/x".into(),
                bytes: Bytes::new(b"x".to_vec()),
                private: true,
            },
            Effect::Run {
                id: EffectId(3),
                program: "/usr/bin/true".into(),
                arguments: vec!["--version".to_owned()],
                environment: [("HOME".to_owned(), "/home/x".to_owned())].into(),
                stdin: None,
            },
            Effect::Open {
                id: EffectId(11),
                program: "/usr/bin/docker".into(),
                arguments: vec!["exec".to_owned(), "-i".to_owned(), "x".to_owned()],
                environment: [("HOME".to_owned(), "/home/x".to_owned())].into(),
            },
            Effect::Send {
                id: EffectId(11),
                line: r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.to_owned(),
            },
            Effect::Close { id: EffectId(11) },
            Effect::Wake {
                id: EffectId(4),
                after: std::time::Duration::from_secs(1),
            },
            Effect::Bind {
                id: EffectId(5),
                address: "127.0.0.1:0".to_owned(),
            },
            Effect::Answer {
                id: RequestId(1),
                answer: Answer::Respond {
                    status: 200,
                    headers: [("content-type".to_owned(), "text/plain".to_owned())].into(),
                    body: Bytes::new(b"ok".to_vec()),
                },
            },
            Effect::Answer {
                id: RequestId(2),
                answer: Answer::Proxy {
                    port: 8080,
                    refused: Bytes::new(b"nothing there".to_vec()),
                },
            },
            Effect::Answer {
                id: RequestId(3),
                answer: Answer::Read { limit: 1024 },
            },
            Effect::Probe {
                id: EffectId(6),
                port: 64_383,
                within: std::time::Duration::from_millis(500),
            },
            Effect::Print {
                text: "hello\n".to_owned(),
            },
            Effect::Exit {
                message: "bye".to_owned(),
            },
        ];
        let kinds: Vec<&str> = effects.iter().map(Named::kind).collect();
        assert_eq!(
            kinds,
            [
                "Read", "Write", "Run", "Open", "Send", "Close", "Wake", "Bind", "Answer",
                "Answer", "Answer", "Probe", "Print", "Exit"
            ]
        );
        crosses_whole(&effects);
    }

    /// Everything round-trips through the one representation both sides
    /// share, and comes back equal.
    fn crosses_whole<T>(values: &[T])
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone + PartialEq,
    {
        for value in values {
            let served = serde_json::to_string(value).expect("it serialises");
            let back: T = serde_json::from_str(&served).expect("and back");
            assert!(back == *value, "{served}");
            assert!(back.clone() == *value);
        }
    }

    /// A request made and a socket spoken over cross whole and name their
    /// kind, the same way.
    #[test]
    fn a_request_and_a_socket_cross_whole_and_name_their_kind() {
        let effects: Vec<Effect<Doorbell>> = vec![
            Effect::Request {
                id: EffectId(7),
                method: "POST".to_owned(),
                url: "https://example.test/api/say".to_owned(),
                headers: [("authorization".to_owned(), "Bearer x".to_owned())].into(),
                body: Some(Bytes::new(b"{\"text\":\"hi\"}".to_vec())),
                within: std::time::Duration::from_secs(30),
            },
            Effect::Connect {
                id: EffectId(8),
                url: "wss://example.test/socket".to_owned(),
            },
            Effect::Transmit {
                id: EffectId(8),
                text: r#"{"envelope_id":"e-1"}"#.to_owned(),
            },
            Effect::Disconnect { id: EffectId(8) },
        ];
        let kinds: Vec<&str> = effects.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Request", "Connect", "Transmit", "Disconnect"]);
        crosses_whole(&effects);

        let events: Vec<Event<Doorbell>> = vec![
            Event::Responded {
                id: EffectId(7),
                responded: Responded::Answered {
                    status: 200,
                    headers: [("content-type".to_owned(), "application/json".to_owned())].into(),
                    body: Bytes::new(b"{\"ok\":true}".to_vec()),
                },
                at: 1_757_000_000_004,
            },
            Event::Responded {
                id: EffectId(7),
                responded: Responded::Failed("the name did not resolve".to_owned()),
                at: 1_757_000_000_004,
            },
            Event::Frame {
                id: EffectId(8),
                text: r#"{"type":"hello"}"#.to_owned(),
                at: 1_757_000_000_001,
            },
            Event::Disconnected {
                id: EffectId(8),
                disconnected: Disconnected::Closed,
                at: 1_757_000_000_002,
            },
            Event::Disconnected {
                id: EffectId(9),
                disconnected: Disconnected::Failed("the handshake was refused".to_owned()),
                at: 1_757_000_000_003,
            },
        ];
        let kinds: Vec<&str> = events.iter().map(Named::kind).collect();
        assert_eq!(
            kinds,
            [
                "Responded",
                "Responded",
                "Frame",
                "Disconnected",
                "Disconnected"
            ]
        );
        crosses_whole(&events);
    }

    /// Every event crosses whole and names its kind, the same way.
    #[test]
    fn every_event_crosses_whole_and_names_its_kind() {
        let events: Vec<Event<Doorbell>> = vec![
            Event::Read {
                id: EffectId(1),
                contents: Ok(None),
            },
            Event::Written {
                id: EffectId(2),
                outcome: Err("full".to_owned()),
            },
            Event::Ran {
                id: EffectId(3),
                finished: Finished::Exited {
                    status: Some(0),
                    stdout: Bytes::new(Vec::new()),
                    stderr: Bytes::new(Vec::new()),
                },
            },
            Event::Ran {
                id: EffectId(3),
                finished: Finished::NotFound,
            },
            Event::Woke { id: EffectId(4) },
            Event::Line {
                id: EffectId(11),
                line: r#"{"jsonrpc":"2.0","id":1,"result":{}}"#.to_owned(),
            },
            Event::Ended {
                id: EffectId(11),
                ended: Ended::Exited {
                    status: None,
                    stderr: Bytes::new(b"closed\n".to_vec()),
                },
            },
            Event::Ended {
                id: EffectId(12),
                ended: Ended::NotFound,
            },
            Event::Ended {
                id: EffectId(13),
                ended: Ended::Failed("no permission".to_owned()),
            },
            Event::Bound {
                id: EffectId(5),
                outcome: Ok(47_201),
            },
            Event::Arrived {
                listener: EffectId(5),
                id: RequestId(1),
                request: Arrival {
                    method: "GET".to_owned(),
                    path: "/jobs?open=1".to_owned(),
                    headers: [("host".to_owned(), "stageman.test".to_owned())].into(),
                    peer: "127.0.0.1:53124".to_owned(),
                    at: 1_757_000_000_000,
                },
            },
            Event::Body {
                id: RequestId(1),
                outcome: Ok(Bytes::new(b"{}".to_vec())),
            },
            Event::Probed {
                id: EffectId(6),
                probed: Probed::Closed,
            },
        ];
        let kinds: Vec<&str> = events.iter().map(Named::kind).collect();
        assert_eq!(
            kinds,
            [
                "Read", "Written", "Ran", "Ran", "Woke", "Line", "Ended", "Ended", "Ended",
                "Bound", "Arrived", "Body", "Probed"
            ]
        );
        crosses_whole(&events);
        assert!(events[0] != events[4]);
    }

    /// Neither enumeration formats, and this is what keeps it so: an
    /// inherent method exists only where `Debug` does, and the trait
    /// method underneath answers otherwise, so deriving `Debug` on either
    /// flips the answer and fails here.
    #[test]
    fn the_vocabulary_formats_not_at_all() {
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

        assert!(!Probe::<Event<Doorbell>>(std::marker::PhantomData).formats());
        assert!(!Probe::<Effect<Doorbell>>(std::marker::PhantomData).formats());
        assert!(!Probe::<Bytes>(std::marker::PhantomData).formats());
        assert!(
            Probe::<String>(std::marker::PhantomData).formats(),
            "the probe tells"
        );
    }

    /// Text reads as text, and anything else is spelled so it survives.
    #[test]
    fn bytes_read_as_the_text_they_spell_and_survive_otherwise() {
        let text = Bytes::new(b"{\"kept\": true}".to_vec());
        assert_eq!(
            serde_json::to_string(&text).expect("it serialises"),
            r#"{"text":"{\"kept\": true}"}"#
        );
        let back: Bytes = serde_json::from_str(r#"{"text":"{\"kept\": true}"}"#).expect("back");
        assert!(back == text);
        assert_eq!(back.as_text(), Some("{\"kept\": true}"));

        let binary = Bytes::new(vec![0xff, 0x00, 0x7f]);
        let served = serde_json::to_string(&binary).expect("it serialises");
        assert_eq!(served, r#"{"hex":"ff007f"}"#);
        let back: Bytes = serde_json::from_str(&served).expect("back");
        assert!(back == binary);
        assert_eq!(back.len(), 3);
        assert!(!back.is_empty());
        assert_eq!(back.as_text(), None);

        assert!(serde_json::from_str::<Bytes>(r#"{"hex":"abc"}"#).is_err());
        assert!(serde_json::from_str::<Bytes>(r#"{"hex":"zz"}"#).is_err());

        // What comes out is what went in, both ways round: these are what a
        // file is written from and what a program's output is read through,
        // so bytes invented here would be bytes on somebody's disk.
        assert_eq!(binary.as_slice(), [0xff, 0x00, 0x7f]);
        assert_eq!(binary.clone().into_inner(), vec![0xff, 0x00, 0x7f]);
        let none = Bytes::new(Vec::new());
        assert!(none.is_empty(), "nothing is empty");
        assert!(!binary.is_empty(), "and something is not");
        assert_eq!(none.len(), 0);
        assert!(none.as_slice().is_empty());
        assert_eq!(none.into_inner(), Vec::<u8>::new());
    }

    /// One field apart is a different value, on both enumerations.
    ///
    /// The whole of what a replay rests on: a comparison satisfied by an
    /// identifier alone would call an answer to one effect the answer to
    /// another, and a file would agree with a run that had done something
    /// else. Written as pairs differing in exactly one place, because that
    /// is the comparison a weakened one gets wrong.
    #[test]
    fn one_field_apart_is_not_the_same_value() {
        let read = |id: u64, contents: Result<Option<Bytes>, String>| Event::<Doorbell>::Read {
            id: EffectId(id),
            contents,
        };
        let some = || Ok(Some(Bytes::new(b"x".to_vec())));
        assert!(read(1, Ok(None)) == read(1, Ok(None)));
        assert!(
            read(1, Ok(None)) != read(1, some()),
            "the same read, answered differently"
        );
        assert!(
            read(1, Ok(None)) != read(2, Ok(None)),
            "a different read, answered the same"
        );

        let written = |id: u64, outcome: Result<(), String>| Event::<Doorbell>::Written {
            id: EffectId(id),
            outcome,
        };
        assert!(written(1, Ok(())) == written(1, Ok(())));
        assert!(written(1, Ok(())) != written(1, Err("full".to_owned())));
        assert!(written(1, Ok(())) != written(2, Ok(())));

        let ran = |id: u64, finished: Finished| Event::<Doorbell>::Ran {
            id: EffectId(id),
            finished,
        };
        assert!(ran(1, Finished::NotFound) == ran(1, Finished::NotFound));
        assert!(ran(1, Finished::NotFound) != ran(1, Finished::Failed("no".to_owned())));
        assert!(ran(1, Finished::NotFound) != ran(2, Finished::NotFound));

        let line = |id: u64, line: &str| Event::<Doorbell>::Line {
            id: EffectId(id),
            line: line.to_owned(),
        };
        assert!(line(1, "a") == line(1, "a"));
        assert!(
            line(1, "a") != line(1, "b"),
            "the same process, another line"
        );
        assert!(
            line(1, "a") != line(2, "a"),
            "another process, the same line"
        );

        let ended = |id: u64, status: Option<i32>| Event::<Doorbell>::Ended {
            id: EffectId(id),
            ended: Ended::Exited {
                status,
                stderr: Bytes::new(Vec::new()),
            },
        };
        assert!(ended(1, Some(0)) == ended(1, Some(0)));
        assert!(
            ended(1, Some(0)) != ended(1, None),
            "exiting and being killed are different ends"
        );
        assert!(ended(1, Some(0)) != ended(2, Some(0)));

        let probed = |id: u64, probed: Probed| Event::<Doorbell>::Probed {
            id: EffectId(id),
            probed,
        };
        assert!(probed(1, Probed::Closed) == probed(1, Probed::Closed));
        assert!(
            probed(1, Probed::Closed) != probed(1, Probed::Silent),
            "closing at once and holding open in silence are different answers"
        );
        assert!(probed(1, Probed::Closed) != probed(2, Probed::Closed));

        // Effects compare through the one representation both sides share,
        // so the field that differs here is the one a reader would miss.
        let write = |private: bool| Effect::<Doorbell>::Write {
            id: EffectId(1),
            path: "/tmp/x".into(),
            bytes: Bytes::new(b"x".to_vec()),
            private,
        };
        assert!(write(true) == write(true));
        assert!(
            write(true) != write(false),
            "a private write is not a public one"
        );
    }

    /// One field apart is a different value for a request's answer and a
    /// socket's frame too, including the time a frame arrived.
    #[test]
    fn one_field_apart_is_not_the_same_answer_or_frame() {
        let responded = |id: u64, status: u16| Event::<Doorbell>::Responded {
            id: EffectId(id),
            responded: Responded::Answered {
                status,
                headers: BTreeMap::new(),
                body: Bytes::new(Vec::new()),
            },
            at: 5,
        };
        assert!(responded(1, 200) == responded(1, 200));
        assert!(
            responded(1, 200) != responded(1, 429),
            "the same request, answered differently"
        );
        assert!(responded(1, 200) != responded(2, 200));

        let frame = |id: u64, text: &str, at: u64| Event::<Doorbell>::Frame {
            id: EffectId(id),
            text: text.to_owned(),
            at,
        };
        assert!(frame(1, "a", 5) == frame(1, "a", 5));
        assert!(frame(1, "a", 5) != frame(1, "b", 5), "another frame");
        assert!(frame(1, "a", 5) != frame(2, "a", 5), "another socket");
        assert!(
            frame(1, "a", 5) != frame(1, "a", 6),
            "the same frame at another time"
        );
    }
}
