//! Listening on a project's channel: the connection's lifecycle as state
//! driven by frames, closes and a timer.
//!
//! The transport half of
//! `docs/decisions/0029-a-reply-is-routed-by-its-thread.md` — an event
//! stream this process opens outward — and the property
//! `docs/decisions/0044-a-listener-only-listens.md` decided: the connection
//! is never unattended. Since
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
//! that is a property of the type rather than of a task's discipline. The
//! world's socket task does nothing but turn frames into events and send
//! what it is handed, and everything the lifecycle decides — who this
//! instance is, where to connect, what a frame means, that it is
//! acknowledged before anything else, that a replacement is opened before
//! the platform closes the old connection, and how long a gap with no
//! connection lasted — is decided here, in the step the event arrives in.
//!
//! **A replacement is opened before the connection it replaces is let go.**
//! The platform warns about ten seconds ahead and allows several
//! connections at once; on the warning the old socket goes on being read
//! until it closes, and the new one is asked for at once, with no wait,
//! because nothing went wrong. An unscheduled end has no such warning, so
//! it does leave a gap, and the gap is measured and reported when the next
//! connection greets: a message said into it reached nobody, and the
//! platform does not send it again.

use std::time::Duration;

use stageman_channel::{Identity, Incoming};
use stageman_core::{Channel, ProjectId, Secret, Speaking};
use stageman_vocabulary::{Disconnected, Effect as Generic, EffectId, Responded};

use crate::channel::{Purpose, Question, Sent, request};
use crate::turns::listening_on;
use crate::{Effect, Running, Timer};

/// How long to wait before trying a connection that failed again.
///
/// A fixed wait rather than a backoff. Reconnecting is the ordinary case
/// here, so the common path must not grow a delay that compounds; a
/// credential that is simply wrong retries at this rate for ever, which is
/// cheap and visible in the log. It is only ever waited *after* something
/// went wrong — a connection the platform replaced on schedule does not pass
/// through it.
pub const BEFORE_TRYING_AGAIN: Duration = Duration::from_secs(5);

/// One project's channel being listened to: what opens the stream, what
/// speaks on it, who this instance is on it, and where its connection has
/// got to.
///
/// Held and never kept. A restart begins with none of these and listens
/// again from what the projects say, which is why nothing here survives a
/// crash and nothing needs to.
#[derive(Debug, Clone)]
pub struct Listener {
    /// Which channel.
    pub channel: Channel,
    /// What opens the event stream. Never enters a container.
    pub opening: Secret,
    /// What speaks, and what asks the platform who this instance is.
    pub speaking: Speaking,
    /// Who this instance is on the channel, once told.
    pub us: Option<Identity>,
    /// Where the connection has got to.
    pub phase: Phase,
    /// When this project stopped having a connection, if it has none: what
    /// the window nothing heard is measured from, in the world's
    /// milliseconds. Kept across failed attempts rather than replaced, so
    /// that a run of them is reported as the one window it is.
    pub deaf_since: Option<u64>,
}

/// Where a listener's connection has got to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Phase {
    /// Asking the platform who this instance is.
    Introducing {
        /// The question, on its answer.
        asked: EffectId,
    },
    /// Asking where to connect.
    Locating {
        /// The question, on its answer.
        asked: EffectId,
    },
    /// The socket is opening, and the platform has not greeted on it yet.
    Connecting {
        /// The socket.
        socket: EffectId,
    },
    /// Reading frames.
    Listening {
        /// The socket.
        socket: EffectId,
    },
    /// Waiting before trying again, after something went wrong.
    Waiting {
        /// The timer.
        timer: EffectId,
    },
}

impl Running {
    /// Starts listening on a project's channel, if it has one to listen on
    /// and is not already listened to.
    ///
    /// The first question is handed back rather than pushed, so that the
    /// caller decides whether it is asked now or once a record has landed:
    /// waking asks at once, and binding a channel waits for the write.
    pub fn listen(&mut self, project: ProjectId) -> Option<Effect> {
        if self.listeners.contains_key(&project) {
            tracing::debug!(%project, "already listening on its channel");
            return None;
        }
        let (channel, opening, speaking) =
            self.state.projects.get(&project).and_then(listening_on)?;
        let (asked, question) = self.introducing(project, channel, &speaking);
        self.listeners.insert(
            project,
            Listener {
                channel,
                opening,
                speaking,
                us: None,
                phase: Phase::Introducing { asked },
                deaf_since: None,
            },
        );
        Some(question)
    }

    /// Asks the platform who this instance is, which is where every
    /// connection begins.
    fn introducing(
        &mut self,
        project: ProjectId,
        channel: Channel,
        speaking: &Speaking,
    ) -> (EffectId, Effect) {
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Question(Question::Introducing { project }),
                room: None,
            },
        );
        (
            id,
            request(id, stageman_channel::who_am_i(channel, speaking)),
        )
    }

    /// Begins another connection for a listener that has one to replace, or
    /// has lost one.
    fn again(&mut self, project: ProjectId, effects: &mut Vec<Effect>) {
        let Some((channel, speaking)) = self
            .listeners
            .get(&project)
            .map(|listener| (listener.channel, listener.speaking.clone()))
        else {
            return;
        };
        let (asked, question) = self.introducing(project, channel, &speaking);
        if let Some(listener) = self.listeners.get_mut(&project) {
            listener.phase = Phase::Introducing { asked };
        }
        effects.push(question);
    }

    /// What the platform said to a listener's question.
    pub fn questioned(
        &mut self,
        channel: Channel,
        question: Question,
        responded: &Responded,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        let unreachable = |why: &str| format!("the channel could not be reached: {why}");
        match question {
            Question::Introducing { project } => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::identity(channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.introduced(project, outcome, at, effects);
            }
            Question::Locating { project } => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::socket_url(channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.located(project, outcome, at, effects);
            }
        }
    }

    /// Told who this instance is on a project's channel, or why not: the
    /// next question is where to connect.
    fn introduced(
        &mut self,
        project: ProjectId,
        outcome: Result<Identity, String>,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(listener) = self.listeners.get_mut(&project) else {
            tracing::debug!(%project, "told who it is on a channel it no longer listens to; ignored");
            return;
        };
        let us = match outcome {
            Ok(us) => us,
            Err(why) => {
                self.could_not_listen(project, &why, at, effects);
                return;
            }
        };
        listener.us = Some(us);
        let (channel, opening) = (listener.channel, listener.opening.clone());
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Question(Question::Locating { project }),
                room: None,
            },
        );
        if let Some(listener) = self.listeners.get_mut(&project) {
            listener.phase = Phase::Locating { asked: id };
        }
        effects.push(request(
            id,
            stageman_channel::open_socket(channel, &opening),
        ));
    }

    /// Told where to connect for a project's channel, or why not: the
    /// socket is opened, and the platform's greeting on it is what says it
    /// is up.
    fn located(
        &mut self,
        project: ProjectId,
        outcome: Result<String, String>,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        if !self.listeners.contains_key(&project) {
            tracing::debug!(%project, "told where to connect for a channel it no longer listens to; ignored");
            return;
        }
        let url = match outcome {
            Ok(url) => url,
            Err(why) => {
                self.could_not_listen(project, &why, at, effects);
                return;
            }
        };
        let id = self.effect_id();
        self.sockets.insert(id, project);
        if let Some(listener) = self.listeners.get_mut(&project) {
            listener.phase = Phase::Connecting { socket: id };
        }
        effects.push(Generic::Connect { id, url });
    }

    /// A connection could not be opened: said, the window nothing hears
    /// noted from now if it was not already, and tried again later.
    fn could_not_listen(
        &mut self,
        project: ProjectId,
        why: &str,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        tracing::warn!(%project, %why, "the channel could not be listened to");
        if let Some(listener) = self.listeners.get_mut(&project) {
            listener.deaf_since.get_or_insert(at);
        }
        self.try_again_later(project, effects);
    }

    /// Waits before trying a project's connection again.
    fn try_again_later(&mut self, project: ProjectId, effects: &mut Vec<Effect>) {
        let id = self.effect_id();
        self.timers.insert(id, Timer::Reconnecting { project });
        if let Some(listener) = self.listeners.get_mut(&project) {
            listener.phase = Phase::Waiting { timer: id };
        }
        effects.push(Generic::Wake {
            id,
            after: BEFORE_TRYING_AGAIN,
        });
    }

    /// The wait before trying again is over.
    pub fn try_again(&mut self, project: ProjectId, effects: &mut Vec<Effect>) {
        self.again(project, effects);
    }

    /// A question a listener asked was never made, because the write it
    /// waited on never landed: tried again later, as a failed attempt is.
    pub fn unasked(&mut self, project: ProjectId, effects: &mut Vec<Effect>) {
        if self.listeners.contains_key(&project) {
            self.try_again_later(project, effects);
        }
    }

    /// A frame arrived on a socket.
    ///
    /// Acknowledged before anything else, whatever is decided about it: the
    /// platform redelivers what goes unacknowledged, and what follows can
    /// take as long as an agent takes. A frame on a connection being drained
    /// is read exactly like one on the connection that replaced it, which is
    /// the whole point of draining.
    pub fn frame(&mut self, id: EffectId, text: &str, at: u64, effects: &mut Vec<Effect>) {
        let Some(project) = self.sockets.get(&id).copied() else {
            tracing::warn!("a frame arrived on a socket this instance did not open; ignored");
            return;
        };
        let Some(listener) = self.listeners.get(&project) else {
            tracing::debug!(%project, "a frame on the socket of a channel no longer listened to; ignored");
            return;
        };
        let Some(us) = listener.us.clone() else {
            tracing::warn!(%project, "a frame arrived before the platform said who this instance is; ignored");
            return;
        };
        let channel = listener.channel;
        let heard = stageman_channel::decode(channel, text, &us);
        // Every frame, at a level nobody runs by default. It is the only
        // place that can say whether the platform is sending anything at
        // all, which is the first question when a reply does not arrive.
        tracing::debug!(frame = %text, "heard a frame");
        if let Some(envelope) = heard.acknowledging() {
            effects.push(Generic::Transmit {
                id,
                text: stageman_channel::acknowledgement(channel, envelope),
            });
        }
        match heard {
            Incoming::Ready => self.greeted(project, id, at),
            Incoming::Reconnect => self.refresh(project, id, effects),
            Incoming::Said { message, .. } => {
                // Anything this instance said is the loop guard working,
                // and there is one of these for every message it sends.
                // Said at debug so that what is left at info is somebody
                // talking to it.
                if message.from_us {
                    tracing::debug!(room = %message.room, "heard itself, and ignored it");
                } else {
                    tracing::info!(
                        room = %message.room,
                        thread = message.thread.as_deref().unwrap_or("(root)"),
                        "heard somebody speak"
                    );
                }
                self.heard(project, channel, &message);
            }
            Incoming::Acknowledge(_) | Incoming::Ignore => {}
        }
    }

    /// The platform greeted on a socket: the connection is up.
    ///
    /// Only on the socket the listener is waiting on. A greeting on one
    /// being drained is nothing, and so is one on a socket the listener has
    /// already moved past.
    fn greeted(&mut self, project: ProjectId, socket: EffectId, at: u64) {
        let Some(listener) = self.listeners.get_mut(&project) else {
            return;
        };
        if listener.phase != (Phase::Connecting { socket }) {
            return;
        }
        listener.phase = Phase::Listening { socket };
        // Said at all because the alternative was silence. A listener that
        // connects and is sent nothing looks exactly like one discarding
        // every frame, and the difference is entirely on the platform's
        // side — an event subscription that was never added.
        tracing::info!(
            %project,
            as_user = listener.us.as_ref().map_or("?", |us| us.user.as_str()),
            "listening"
        );
        // Warned rather than logged, and only when there was genuinely
        // nothing listening: this is the whole of what a lost message looks
        // like from in here — a window, its length, and nothing else.
        if let Some(since) = listener.deaf_since.take() {
            if let Some(deaf_for) = at.checked_sub(since) {
                tracing::warn!(
                    %project,
                    deaf_for_ms = deaf_for,
                    "nothing was listening on this project's channel for that long; anything \
                     said in the window was not heard, and the platform does not send it again"
                );
            } else {
                tracing::warn!(
                    %project,
                    "nothing was listening on this project's channel for a while; anything said \
                     in the window was not heard, and the platform does not send it again"
                );
            }
        }
    }

    /// The platform said it is about to close a connection: a replacement
    /// is opened at once, and the old one is read until it closes.
    ///
    /// Only for the connection the listener is on. The platform says this
    /// about ten seconds before it closes, and those ten seconds are what
    /// the replacement is opened inside of; a warning on a connection
    /// already being drained is nothing.
    fn refresh(&mut self, project: ProjectId, socket: EffectId, effects: &mut Vec<Effect>) {
        let listening = self
            .listeners
            .get(&project)
            .is_some_and(|listener| listener.phase == Phase::Listening { socket });
        if !listening {
            return;
        }
        self.draining.insert(socket);
        self.again(project, effects);
    }

    /// A socket ended.
    ///
    /// One being drained had already been replaced, so its end costs
    /// nothing. The connection a listener is on ending is the ordinary
    /// case when the far side closed it, and a warning otherwise; either
    /// way it is the start of a window nothing hears, and the connection is
    /// tried again after a wait.
    pub fn disconnected(
        &mut self,
        id: EffectId,
        disconnected: &Disconnected,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(project) = self.sockets.remove(&id) else {
            tracing::warn!("a socket this instance did not open ended; ignored");
            return;
        };
        if self.draining.remove(&id) {
            tracing::debug!(%project, "a replaced connection ended");
            return;
        }
        let Some(listener) = self.listeners.get_mut(&project) else {
            tracing::debug!(%project, "the socket of a channel no longer listened to ended");
            return;
        };
        match disconnected {
            // An ordinary end, which is most of them: the platform closes a
            // long-lived connection on a schedule of its own and does not
            // always say goodbye first, so this is the common path rather
            // than a fault — and reporting it as one is how somebody learns
            // to ignore what this reports.
            Disconnected::Closed => {
                tracing::info!(%project, "the connection ended; opening another");
            }
            Disconnected::Failed(why) => {
                tracing::warn!(%project, %why, "the channel stopped being readable");
            }
        }
        listener.deaf_since.get_or_insert(at);
        self.try_again_later(project, effects);
    }

    /// Stops listening on a project's channel: every socket of its is
    /// disconnected, and its timer forgotten.
    ///
    /// A question in flight answers to nobody afterwards, and the sockets
    /// stay known until each says it has ended, so that their ends are
    /// placed rather than wondered about.
    pub fn stop_listening(&mut self, project: ProjectId) -> Vec<Effect> {
        let Some(listener) = self.listeners.remove(&project) else {
            return Vec::new();
        };
        if let Phase::Waiting { timer } = listener.phase {
            self.timers.remove(&timer);
        }
        self.sockets
            .iter()
            .filter(|(_, whose)| **whose == project)
            .map(|(id, _)| Generic::Disconnect { id: *id })
            .collect()
    }
}
