//! What this instance says to the world and hears from it, in the
//! application hole of the vocabulary — and the plain types it holds that
//! more than one module names.
//!
//! Everything here is plain data. An [`AppEvent`] is what the world tells the
//! instance and an [`AppEffect`] is what the instance asks of the world, and
//! neither carries a call, a channel, a callback or a domain secret. Neither
//! enumeration formats, for the reason `docs/conventions.md` §4 gives. See
//! `docs/decisions/0056-the-instance-decides-and-the-world-performs.md` and
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! **Three things are in the hole, and they are what 0057 keeps there
//! deliberately.** The presentation port's arrival, which is an application
//! fact the entry point tells rather than something asked for; and a
//! person's request with its typed answer, which is the one thing that
//! still asks the instance directly. Neither is a mechanism a generic world
//! could perform. Everything else that was here was a meaning rather than a
//! mechanism — a turn, a probe, a message posted, a channel listened to —
//! and each left as the instance started speaking the mechanism instead: the
//! turn is the commands and the process in `crate::turns`, the probe is a
//! port asked of the runtime and read once in `crate::tunnel`, a message is
//! a request made and read in `crate::channel`, and a channel is a socket
//! driven in `crate::listening`.
//!
//! The other types here — a container as the runtime reports it, whose turn
//! it is, what a credential entitles its bearer to — are not the hole's.
//! They are what the instance holds, kept here because more than one module
//! names each and none owns it.

use serde::{Deserialize, Serialize};
use stageman_core::{Agent, InstanceId, JobId, Place, ProjectId};
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
/// before a crash names nothing. The place is where anything the bearer says
/// lands: a job's room, or the thread in it a person asked in; a foreman's
/// thread, per turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warranted {
    /// Who holds it.
    pub speaker: Speaker,
    /// Where they speak, if anywhere.
    pub place: Option<Place>,
    /// Who the message being answered is from, when the platform said: what
    /// a job started in this turn records as who asked for it.
    pub from: Option<String>,
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
    /// A person asked something of the dashboard. Answered by
    /// [`AppEffect::Respond`], in this step or a later one.
    Request {
        /// What the world is waiting to answer.
        id: RequestId,
        /// What was asked. Boxed because a request is as wide as the
        /// widest screen a person can draft on, and the other event here
        /// is a port.
        request: Box<crate::requests::Request>,
    },
}

impl Named for AppEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Presenting { .. } => "Presenting",
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
}

impl Named for AppEffect {
    fn kind(&self) -> &'static str {
        match self {
            Self::Respond { .. } => "Respond",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppEffect, AppEvent, RequestId};
    use stageman_vocabulary::Named;

    /// Every event and effect of this application's names its kind, which
    /// is the one thing the world may say about one in a log line.
    #[test]
    fn every_kind_is_named() {
        let events = [
            AppEvent::Presenting { port: 9000 },
            AppEvent::Request {
                id: RequestId(1),
                request: Box::new(crate::requests::Request::Instance),
            },
        ];
        let kinds: Vec<&str> = events.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Presenting", "Request"]);

        let effects = [AppEffect::Respond {
            id: RequestId(1),
            response: crate::requests::Response::Agents(Vec::new()),
        }];
        let kinds: Vec<&str> = effects.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Respond"]);
    }

    /// This application's own events and effects format no more than the
    /// vocabulary does, and for the same reason the rule exists even now that
    /// nothing in them carries a credential: the probe answers through an
    /// inherent method only where `Debug` exists.
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
