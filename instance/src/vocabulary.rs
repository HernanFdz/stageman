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
//! Every family here is a meaning rather than a mechanism — listen, hear —
//! and each moves out of this hole into the generic vocabulary as the
//! instance starts speaking the mechanism instead. A turn already has, and
//! so have the probe and speaking on a channel: the turn is the commands
//! and the process in `crate::turns`, the probe is a port asked of the
//! runtime and read once in `crate::tunnel`, and a message is a request
//! made and read in `crate::channel`.

use serde::{Deserialize, Serialize};
use stageman_core::{Agent, Channel, InstanceId, JobId, ProjectId, Speaking, Thread};
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
    /// Where the presentation server answers, said by the entry point once
    /// it has taken a loopback port for it.
    ///
    /// The application's own fact rather than something asked for: what
    /// serves the pages is the framework, in this process, and the instance
    /// forwards to it exactly as it forwards to a job's container.
    Presenting {
        /// The loopback port it is on.
        port: u16,
    },
    /// Somebody said something on a channel this instance listens to.
    Heard {
        /// Which channel.
        channel: Channel,
        /// What was heard.
        message: Message,
    },
    /// A person asked something of the dashboard. Answered by
    /// [`AppEffect::Respond`], in this step or a later one.
    Request {
        /// What the world is waiting to answer.
        id: RequestId,
        /// What was asked.
        request: crate::requests::Request,
    },
}

impl Named for AppEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Presenting { .. } => "Presenting",
            Self::Heard { .. } => "Heard",
            Self::Request { .. } => "Request",
        }
    }
}

/// One thing the instance asks of the world.
///
/// The doc comment on each says whether it is answered, and by what. An
/// unanswered effect's failure is the world's to log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppEffect {
    /// Answer a person's request. Unanswered.
    Respond {
        /// Which request.
        id: RequestId,
        /// The answer, typed for the screen that asked.
        response: crate::requests::Response,
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
            Self::Respond { .. } => "Respond",
            Self::Listen { .. } => "Listen",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppEffect, AppEvent, Posting, RequestId};
    use stageman_core::{Channel, Uuid};
    use stageman_vocabulary::Named;

    /// Every event and effect of this application's names its kind, which
    /// is the one thing the world may say about one in a log line.
    #[test]
    fn every_kind_is_named() {
        let posting = Posting {
            address: "C0123456789".to_owned(),
            credential: "xoxb-not-a-real-token".to_owned(),
        };
        let events = [
            AppEvent::Presenting { port: 9000 },
            AppEvent::Heard {
                channel: Channel::Slack,
                message: super::Message {
                    address: "C0123456789".to_owned(),
                    id: "1788000000.000002".to_owned(),
                    thread: None,
                    text: "hello".to_owned(),
                    mentions: true,
                    from_us: false,
                },
            },
            AppEvent::Request {
                id: RequestId(1),
                request: crate::requests::Request::Instance,
            },
        ];
        let kinds: Vec<&str> = events.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Presenting", "Heard", "Request"]);

        let effects = [
            AppEffect::Respond {
                id: RequestId(1),
                response: crate::requests::Response::Agents(Vec::new()),
            },
            AppEffect::Listen {
                project: stageman_core::ProjectId::from_uuid(Uuid::from_u128(2)),
                opening: "xapp-not-a-real-token".to_owned(),
                speaking: posting,
            },
        ];
        let kinds: Vec<&str> = effects.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Respond", "Listen"]);
    }

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
        assert!(
            Probe::<String>(std::marker::PhantomData).formats(),
            "the probe tells"
        );
    }
}
