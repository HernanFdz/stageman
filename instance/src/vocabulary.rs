//! What this instance says to the world and hears from it, in the
//! application hole of the vocabulary.
//!
//! Everything here is plain data. An [`AppEvent`] is what the world tells the
//! instance and an [`AppEffect`] is what the instance asks of the world, and
//! neither carries a call, a channel, a callback or a domain secret: a
//! credential crosses as the string it is, so that a scenario's trace
//! serialises in full and the instance can learn nothing from making an
//! effect. Neither enumeration formats, for the reason
//! `docs/conventions.md` §4 gives. See
//! `docs/decisions/0056-the-instance-decides-and-the-world-performs.md` and
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Every family here is a meaning rather than a mechanism — run a turn, probe
//! a tunnel, say something — and each moves out of this hole into the generic
//! vocabulary as the instance starts speaking the mechanism instead.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use stageman_agent::Answer;
use stageman_core::{
    Agent, Channel, InstanceId, JobId, Kit, Platform, ProjectId, Role, Speaking, Thread, Timestamp,
};
use stageman_vocabulary::Named;

/// What identifies one request the world is waiting to answer.
///
/// Minted by the world, carried in and echoed back, and opaque here: the
/// instance decides nothing from its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequestId(pub u64);

/// One container the runtime holds, as the world reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Speaker {
    /// The one agent a project's foreman thinks with.
    Foreman(ProjectId),
    /// One job's agent.
    Job(JobId),
}

/// What a credential presented to the tools endpoint entitles its bearer to.
///
/// Minted when a turn starts and forgotten when it ends, so a warrant from
/// before a crash names nothing. The thread is where anything the bearer says
/// lands: a job's for its whole life, a foreman's per turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// What posts on a channel, as plain data: where, and with what.
///
/// The domain's own type for this holds the credential as a secret, which
/// deliberately does not serialise; this is that type as it crosses to the
/// world, credential in the clear, for the reason the module says.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    /// Where on the channel.
    pub address: String,
    /// What reaches it.
    pub credential: String,
}

impl From<Speaking> for Posting {
    fn from(speaking: Speaking) -> Self {
        Self {
            address: speaking.address,
            credential: speaking.credential.expose().to_owned(),
        }
    }
}

impl From<Posting> for Speaking {
    fn from(posting: Posting) -> Self {
        Self {
            address: posting.address,
            credential: stageman_core::Secret::new(posting.credential),
        }
    }
}

/// One thing the world tells the instance.
///
/// Time appears only where a handler keeps it, which in this set is nowhere:
/// nothing here is recorded with a timestamp.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppEvent {
    /// Where the dashboard is being served, once the world has bound it.
    ///
    /// The one application fact that exists before the instance and is not
    /// in its environment: a port of zero there is a request for whichever
    /// is free, and only the bind knows the answer. Until the listener is
    /// the instance's own to bind, the world says.
    Serving {
        /// The address, as a person would type it after the scheme.
        address: String,
        /// The port, for what a job is told about its tunnel.
        port: u16,
    },
    /// Answers [`AppEffect::RunTurn`]: the agent stopped, or could not be run.
    TurnEnded {
        /// Whose turn it was.
        speaker: Speaker,
        /// What the agent said and why it stopped, or why it could not.
        outcome: Result<Answer, String>,
    },
    /// Answers [`AppEffect::Probe`]: whether something is behind a job's
    /// tunnel.
    Probed {
        /// Which job's tunnel was probed.
        job: JobId,
        /// Whether anything answered behind it.
        answering: bool,
    },
    /// Answers [`AppEffect::ListRunning`]: the containers up right now.
    Listed {
        /// Every running container this project started, with its labels.
        running: Vec<Container>,
    },
    /// Somebody said something on a channel this instance listens to.
    Heard {
        /// Which channel.
        channel: Channel,
        /// What was heard.
        message: Message,
    },
    /// Answers [`AppEffect::Inspect`]: whether a container exists, and for
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
    /// [`AppEffect::ToolAnswered`], in this step or a later one.
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
    /// Answers [`AppEffect::OpenThread`]: where a job's conversation
    /// happens, or why it could not be opened.
    ThreadOpened {
        /// Whose thread.
        job: JobId,
        /// The thread, or why not.
        outcome: Result<Thread, String>,
    },
    /// Answers [`AppEffect::Post`]: whether the platform took the message.
    Posted {
        /// Which request it was said for.
        request: RequestId,
        /// Why not, if not.
        outcome: Result<(), String>,
    },
    /// A person asked something of the dashboard. Answered by
    /// [`AppEffect::Respond`], in this step or a later one.
    Request {
        /// What the world is waiting to answer.
        id: RequestId,
        /// What was asked.
        request: crate::requests::Request,
    },
    /// A request arrived for a name one label below the domain, and that
    /// label is a job's identifier. Answered by [`AppEffect::Route`], in
    /// this step or once the runtime has said where the tunnel is. The
    /// world decodes the hostname, which is shape; whether the job is one of
    /// this instance's is state, and so is asked here.
    TunnelAsked {
        /// What the world is waiting to answer.
        id: RequestId,
        /// The job the name identifies.
        job: JobId,
    },
    /// Answers [`AppEffect::FindPort`]: where the job's tunnel is published,
    /// if its container is running with one.
    PortFound {
        /// Which job.
        job: JobId,
        /// The host port, or none if nothing can be reached.
        port: Option<u16>,
    },
    /// A connection to where a job's tunnel was last found did not go
    /// through. Only failures are reported: a relay that worked needs no
    /// decision, and one per request would be noise.
    TunnelFailed {
        /// Which job.
        job: JobId,
        /// What went wrong, for the log.
        why: String,
    },
}

impl Named for AppEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Serving { .. } => "Serving",
            Self::TurnEnded { .. } => "TurnEnded",
            Self::Probed { .. } => "Probed",
            Self::Listed { .. } => "Listed",
            Self::Heard { .. } => "Heard",
            Self::Inspected { .. } => "Inspected",
            Self::ToolCalled { .. } => "ToolCalled",
            Self::ThreadOpened { .. } => "ThreadOpened",
            Self::Posted { .. } => "Posted",
            Self::Request { .. } => "Request",
            Self::TunnelAsked { .. } => "TunnelAsked",
            Self::PortFound { .. } => "PortFound",
            Self::TunnelFailed { .. } => "TunnelFailed",
        }
    }
}

/// One thing the instance asks of the world.
///
/// The doc comment on each says whether it is answered, and by what. An
/// unanswered effect's failure is the world's to log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppEffect {
    /// What booting found, for the world's own performers: which runtime
    /// answers, and which domain this instance answers on. Unanswered, and
    /// on its way out: every effect that needs the runtime will carry its
    /// path, and the domain is the instance's to decide on once it binds
    /// its own listeners.
    Booted {
        /// The container runtime that answered.
        runtime: PathBuf,
        /// The domain, as it is compared and printed.
        domain: String,
    },
    /// Run one turn of an agent. Answered by [`AppEvent::TurnEnded`].
    RunTurn {
        /// Whose turn.
        speaker: Speaker,
        /// Starting a session or continuing one.
        run: Run,
    },
    /// Ask whether anything is behind a job's tunnel. Answered by
    /// [`AppEvent::Probed`].
    Probe {
        /// Which job.
        job: JobId,
    },
    /// Ask which containers are up. Answered by [`AppEvent::Listed`].
    ListRunning,
    /// Ask whether a container exists and for which agent it was made.
    /// Answered by [`AppEvent::Inspected`].
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
    /// Post on a channel, in a thread, on the instance's own behalf.
    /// Unanswered: this is a notice about an outcome, and the outcome does
    /// not change because the notice of it did not arrive.
    Say {
        /// The channel and the credential that posts on it.
        speaking: Posting,
        /// Where in it.
        thread: Thread,
        /// What.
        text: String,
    },
    /// Answer a tunnel request: where to forward it, or that nothing
    /// answers on that name. Unanswered, except by [`AppEvent::TunnelFailed`]
    /// when the forwarding does not go through.
    Route {
        /// Which request.
        id: RequestId,
        /// The host port to forward to, or none for a name nothing answers on.
        port: Option<u16>,
    },
    /// Ask the runtime where a job's tunnel is published. Answered by
    /// [`AppEvent::PortFound`].
    FindPort {
        /// Which job.
        job: JobId,
    },
    /// Answer a person's request. Unanswered.
    Respond {
        /// Which request.
        id: RequestId,
        /// The answer, typed for the screen that asked.
        response: crate::requests::Response,
    },
    /// Stop the turn running for a speaker: the agent process is ended and
    /// the container carries on. Answered by [`AppEvent::TurnEnded`], like
    /// the turn it stops.
    StopTurn {
        /// Whose turn.
        speaker: Speaker,
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
    /// [`AppEvent::ThreadOpened`].
    OpenThread {
        /// Whose thread.
        job: JobId,
        /// The channel and the credential that posts on it.
        speaking: Posting,
        /// What the thread hangs from.
        announcement: String,
    },
    /// Post on a channel on an agent's behalf. Answered by
    /// [`AppEvent::Posted`], because the agent is told whether it was heard.
    Post {
        /// Which request is waiting on it.
        request: RequestId,
        /// The channel and the credential that posts on it.
        speaking: Posting,
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
        opening: String,
        /// What posts, and what asks the platform who this instance is.
        speaking: Posting,
    },
}

impl Named for AppEffect {
    fn kind(&self) -> &'static str {
        match self {
            Self::Booted { .. } => "Booted",
            Self::RunTurn { .. } => "RunTurn",
            Self::Probe { .. } => "Probe",
            Self::ListRunning => "ListRunning",
            Self::Inspect { .. } => "Inspect",
            Self::Halt { .. } => "Halt",
            Self::Discard { .. } => "Discard",
            Self::Reclaim => "Reclaim",
            Self::Say { .. } => "Say",
            Self::Route { .. } => "Route",
            Self::FindPort { .. } => "FindPort",
            Self::Respond { .. } => "Respond",
            Self::StopTurn { .. } => "StopTurn",
            Self::ToolAnswered { .. } => "ToolAnswered",
            Self::OpenThread { .. } => "OpenThread",
            Self::Post { .. } => "Post",
            Self::Listen { .. } => "Listen",
        }
    }
}

/// Whether a turn starts a session or continues the one its container holds.
///
/// Everything an agent process is about to be handed, decided here and
/// carried as plain data: the environment it is given is rendered from the
/// handout by the instance, so that what a container sees is decided in the
/// one place that decides, and the world only sets it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Run {
    /// Make the container and the session, and put the first question.
    Begin {
        /// The container to make, named before it exists.
        container: String,
        /// Which instance made it, for the label.
        instance: InstanceId,
        /// Which agent runs in it.
        agent: Agent,
        /// What it runs as, which decides the image.
        role: Role,
        /// Exactly the environment the container is given, and nothing
        /// inherited. Credentials in the clear, for the reason the module
        /// says.
        environment: BTreeMap<String, String>,
        /// The repository checked out before the agent speaks, for a job.
        repository: Option<String>,
        /// The platform whose tool makes the checkout, if a credential for
        /// one is held.
        platform: Option<Platform>,
        /// What the agent runs on.
        kit: Kit,
        /// What the agent presents to the tools endpoint.
        warrant: String,
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
        warrant: String,
        /// What the resumed agent is told.
        text: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{AppEffect, AppEvent, Run};

    /// This application's own events and effects format no more than the
    /// vocabulary does, because a credential crosses them in the clear: the
    /// probe answers through an inherent method only where `Debug` exists.
    #[test]
    fn the_applications_own_vocabulary_formats_not_at_all() {
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

        assert!(!Probe::<AppEvent>(std::marker::PhantomData).formats());
        assert!(!Probe::<AppEffect>(std::marker::PhantomData).formats());
        assert!(!Probe::<Run>(std::marker::PhantomData).formats());
        assert!(
            Probe::<String>(std::marker::PhantomData).formats(),
            "the probe tells"
        );
    }
}
