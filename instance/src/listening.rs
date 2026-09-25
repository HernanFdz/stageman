//! Listening on a channel: the connection's lifecycle as state driven by
//! frames, closes and a timer.
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
//!
//! **One connection per app-level token.** A project's own app is listened
//! to for that project, as it always was; the instance's own app on a
//! channel is listened to once, however many workspaces it is installed on
//! and however many projects speak through them, and a frame on it names
//! its workspace, which is what says which projects it can be for — see
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
//! Who this instance is differs per workspace, since each install makes a
//! bot user of its own, so that listener holds a voice per workspace and
//! asks who it is with each before it connects.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use stageman_channel::{Identity, Incoming};
use stageman_core::{Binding, Channel, ProjectId, Secret, Speaking};
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

/// What a listener listens with: a project's own app, or the instance's
/// app on a channel.
///
/// The key every connection, question and timer is placed by. A project's
/// own app is its project's; the instance's is nobody's in particular, and
/// what a frame on it is for is decided by the workspace the frame names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Listening {
    /// A project's own app: one connection for that project.
    Own(ProjectId),
    /// The instance's app on a channel: one connection for every workspace
    /// it is installed on.
    App(Channel),
}

impl fmt::Display for Listening {
    /// The project's identifier for its own app, as the snapshot always
    /// keyed a listener; the channel's name for the instance's.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Own(project) => write!(f, "{project}"),
            Self::App(channel) => write!(f, "app:{channel:?}"),
        }
    }
}

/// One voice on a connection: what speaks with it, and who this instance
/// is with it, once told.
#[derive(Debug, Clone)]
pub struct Voice {
    /// What speaks, and what asks the platform who this instance is.
    pub speaking: Speaking,
    /// Who this instance is with that credential, once told.
    pub us: Option<Identity>,
}

/// The voices a listener hears with: the one of a project's own app, or one
/// per workspace of the instance's.
#[derive(Debug, Clone)]
pub enum Voices {
    /// A project's own app speaks with one credential.
    Own(Voice),
    /// The instance's app speaks with a workspace's bot token in each
    /// workspace it is installed on, by the workspace's identifier.
    Workspaces(BTreeMap<String, Voice>),
}

impl Voices {
    /// The voice for a workspace, or the only voice where there is one; the
    /// first voice for a frame naming no workspace, which is a greeting or
    /// a disconnect and needs none.
    fn voice(&self, team: Option<&str>) -> Option<&Voice> {
        match (self, team) {
            (Self::Own(voice), _) => Some(voice),
            (Self::Workspaces(voices), Some(team)) => voices.get(team),
            (Self::Workspaces(voices), None) => voices.values().next(),
        }
    }

    /// The same, to be told who this instance is with it.
    fn voice_mut(&mut self, team: Option<&str>) -> Option<&mut Voice> {
        match (self, team) {
            (Self::Own(voice), _) => Some(voice),
            (Self::Workspaces(voices), Some(team)) => voices.get_mut(team),
            (Self::Workspaces(_), None) => None,
        }
    }

    /// Every voice, by the workspace it is for, with what speaks for it.
    fn each(&self) -> Vec<(Option<String>, Speaking)> {
        match self {
            Self::Own(voice) => vec![(None, voice.speaking.clone())],
            Self::Workspaces(voices) => voices
                .iter()
                .map(|(team, voice)| (Some(team.clone()), voice.speaking.clone()))
                .collect(),
        }
    }

    /// Who this instance is with each voice that has been told, by the
    /// workspace the voice is for.
    fn identities(&self) -> Vec<(Option<&str>, &Identity)> {
        match self {
            Self::Own(voice) => voice.us.iter().map(|us| (None, us)).collect(),
            Self::Workspaces(voices) => voices
                .iter()
                .filter_map(|(team, voice)| Some((Some(team.as_str()), voice.us.as_ref()?)))
                .collect(),
        }
    }

    /// Whether there is nothing left to hear with.
    fn is_empty(&self) -> bool {
        matches!(self, Self::Workspaces(voices) if voices.is_empty())
    }
}

/// One app being listened to: what opens the stream, the voices it is
/// heard with, and where its connection has got to.
///
/// Held and never kept. A restart begins with none of these and listens
/// again from what the projects and the app say, which is why nothing here
/// survives a crash and nothing needs to.
#[derive(Debug, Clone)]
pub struct Listener {
    /// Which channel.
    pub channel: Channel,
    /// What opens the event stream. Never enters a container.
    pub opening: Secret,
    /// What speaks on it, and who this instance is with each, once told.
    pub voices: Voices,
    /// Where the connection has got to.
    pub phase: Phase,
    /// When this listener stopped having a connection, if it has none:
    /// what the window nothing heard is measured from, in the world's
    /// milliseconds. Kept across failed attempts rather than replaced, so
    /// that a run of them is reported as the one window it is.
    pub deaf_since: Option<u64>,
}

/// Where a listener's connection has got to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Phase {
    /// Asking the platform who this instance is, with every voice.
    Introducing {
        /// The questions, on their answers.
        asked: BTreeSet<EffectId>,
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
    /// Starts listening with an app, if there is something to listen with
    /// and it is not already listened to: the questions to ask first,
    /// handed back rather than pushed so that the caller decides whether
    /// they are asked now or once a record has landed — waking asks at
    /// once, and binding a channel waits for the write.
    ///
    /// A project's own app is listened to when its binding is one; the
    /// instance's app when it is registered and installed somewhere, with a
    /// voice per workspace.
    pub fn listen(&mut self, listening: Listening) -> Vec<Effect> {
        if self.listeners.contains_key(&listening) {
            tracing::debug!(%listening, "already listening");
            return Vec::new();
        }
        let (channel, opening, voices) = match listening {
            Listening::Own(project) => {
                let Some((channel, opening, speaking)) =
                    self.state.projects.get(&project).and_then(listening_on)
                else {
                    return Vec::new();
                };
                (channel, opening, Voices::Own(Voice { speaking, us: None }))
            }
            Listening::App(channel) => {
                let Some(app) = self.state.channel_apps.get(&channel) else {
                    return Vec::new();
                };
                if app.workspaces.is_empty() {
                    return Vec::new();
                }
                let voices = app
                    .workspaces
                    .iter()
                    .map(|(team, workspace)| {
                        (
                            team.clone(),
                            Voice {
                                speaking: Speaking {
                                    credential: workspace.bot_token.clone(),
                                },
                                us: None,
                            },
                        )
                    })
                    .collect();
                (channel, app.app_token.clone(), Voices::Workspaces(voices))
            }
        };
        let (asked, questions) = self.introducing_each(listening, channel, &voices);
        self.listeners.insert(
            listening,
            Listener {
                channel,
                opening,
                voices,
                phase: Phase::Introducing { asked },
                deaf_since: None,
            },
        );
        questions
    }

    /// Asks the platform who this instance is with every voice, which is
    /// where every connection begins.
    fn introducing_each(
        &mut self,
        listening: Listening,
        channel: Channel,
        voices: &Voices,
    ) -> (BTreeSet<EffectId>, Vec<Effect>) {
        let mut asked = BTreeSet::new();
        let mut questions = Vec::new();
        for (team, speaking) in voices.each() {
            let (id, question) = self.introducing(listening, channel, team, &speaking);
            asked.insert(id);
            questions.push(question);
        }
        (asked, questions)
    }

    /// Asks the platform who this instance is with one voice.
    fn introducing(
        &mut self,
        listening: Listening,
        channel: Channel,
        team: Option<String>,
        speaking: &Speaking,
    ) -> (EffectId, Effect) {
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Question(Question::Introducing { listening, team }),
                room: None,
            },
        );
        (
            id,
            request(id, stageman_channel::who_am_i(channel, speaking)),
        )
    }

    /// Begins another connection for a listener that has one to replace, or
    /// has lost one: every voice introduced again, since a bot user can
    /// have changed under a credential meanwhile.
    fn again(&mut self, listening: Listening, effects: &mut Vec<Effect>) {
        let Some((channel, voices)) = self
            .listeners
            .get(&listening)
            .map(|listener| (listener.channel, listener.voices.clone()))
        else {
            return;
        };
        let (asked, questions) = self.introducing_each(listening, channel, &voices);
        if let Some(listener) = self.listeners.get_mut(&listening) {
            listener.phase = Phase::Introducing { asked };
        }
        effects.extend(questions);
    }

    /// What the platform said to a listener's question.
    pub fn questioned(
        &mut self,
        id: EffectId,
        channel: Channel,
        question: Question,
        responded: &Responded,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        let unreachable = |why: &str| format!("the channel could not be reached: {why}");
        match question {
            Question::Introducing { listening, team } => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::identity(channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.introduced(listening, team.as_deref(), id, outcome, at, effects);
            }
            Question::Locating { listening } => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::socket_url(channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.located(listening, outcome, at, effects);
            }
        }
    }

    /// Told who this instance is with one voice, or why not: once every
    /// voice has been told, the next question is where to connect. A voice
    /// introduced while the connection is up — a workspace installed
    /// meanwhile — is told and that is all.
    fn introduced(
        &mut self,
        listening: Listening,
        team: Option<&str>,
        id: EffectId,
        outcome: Result<Identity, String>,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(listener) = self.listeners.get_mut(&listening) else {
            tracing::debug!(%listening, "told who it is on a channel it no longer listens to; ignored");
            return;
        };
        let us = match outcome {
            Ok(us) => us,
            Err(why) => {
                self.could_not_listen(listening, &why, at, effects);
                return;
            }
        };
        if let Some(voice) = listener.voices.voice_mut(team) {
            voice.us = Some(us);
        }
        let Phase::Introducing { asked } = &mut listener.phase else {
            return;
        };
        asked.remove(&id);
        if !asked.is_empty() {
            return;
        }
        let (channel, opening) = (listener.channel, listener.opening.clone());
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Question(Question::Locating { listening }),
                room: None,
            },
        );
        if let Some(listener) = self.listeners.get_mut(&listening) {
            listener.phase = Phase::Locating { asked: id };
        }
        effects.push(request(
            id,
            stageman_channel::open_socket(channel, &opening),
        ));
    }

    /// Told where to connect, or why not: the socket is opened, and the
    /// platform's greeting on it is what says it is up.
    fn located(
        &mut self,
        listening: Listening,
        outcome: Result<String, String>,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        if !self.listeners.contains_key(&listening) {
            tracing::debug!(%listening, "told where to connect for a channel it no longer listens to; ignored");
            return;
        }
        let url = match outcome {
            Ok(url) => url,
            Err(why) => {
                self.could_not_listen(listening, &why, at, effects);
                return;
            }
        };
        let id = self.effect_id();
        self.sockets.insert(id, listening);
        if let Some(listener) = self.listeners.get_mut(&listening) {
            listener.phase = Phase::Connecting { socket: id };
        }
        effects.push(Generic::Connect { id, url });
    }

    /// A connection could not be opened: said, the window nothing hears
    /// noted from now if it was not already, and tried again later.
    fn could_not_listen(
        &mut self,
        listening: Listening,
        why: &str,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        tracing::warn!(%listening, %why, "the channel could not be listened to");
        if let Some(listener) = self.listeners.get_mut(&listening) {
            listener.deaf_since.get_or_insert(at);
        }
        self.try_again_later(listening, effects);
    }

    /// Waits before trying a connection again.
    fn try_again_later(&mut self, listening: Listening, effects: &mut Vec<Effect>) {
        let id = self.effect_id();
        self.timers.insert(id, Timer::Reconnecting { listening });
        if let Some(listener) = self.listeners.get_mut(&listening) {
            listener.phase = Phase::Waiting { timer: id };
        }
        effects.push(Generic::Wake {
            id,
            after: BEFORE_TRYING_AGAIN,
        });
    }

    /// The wait before trying again is over.
    pub fn try_again(&mut self, listening: Listening, effects: &mut Vec<Effect>) {
        self.again(listening, effects);
    }

    /// A question a listener asked was never made, because the write it
    /// waited on never landed: tried again later, as a failed attempt is.
    pub fn unasked(&mut self, listening: Listening, effects: &mut Vec<Effect>) {
        if self.listeners.contains_key(&listening) {
            self.try_again_later(listening, effects);
        }
    }

    /// A frame arrived on a socket.
    ///
    /// Acknowledged before anything else, whatever is decided about it: the
    /// platform redelivers what goes unacknowledged, and what follows can
    /// take as long as an agent takes. A frame on a connection being drained
    /// is read exactly like one on the connection that replaced it, which is
    /// the whole point of draining. On the instance's app, the workspace the
    /// frame names is what picks the voice it is read with and the projects
    /// it can be for.
    pub fn frame(&mut self, id: EffectId, text: &str, at: u64, effects: &mut Vec<Effect>) {
        let Some(listening) = self.sockets.get(&id).copied() else {
            tracing::warn!("a frame arrived on a socket this instance did not open; ignored");
            return;
        };
        let Some(listener) = self.listeners.get(&listening) else {
            tracing::debug!(%listening, "a frame on the socket of a channel no longer listened to; ignored");
            return;
        };
        let channel = listener.channel;
        let team = match listening {
            Listening::Own(_) => None,
            Listening::App(channel) => stageman_channel::workspace_of(channel, text),
        };
        let Some(us) = listener
            .voices
            .voice(team.as_deref())
            .and_then(|voice| voice.us.clone())
        else {
            tracing::warn!(%listening, "a frame arrived before the platform said who this instance is; ignored");
            return;
        };
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
            Incoming::Ready => self.greeted(listening, id, at),
            Incoming::Reconnect => self.refresh(listening, id, effects),
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
                match listening {
                    Listening::Own(project) => self.heard(project, channel, &message),
                    Listening::App(channel) => {
                        self.heard_on_workspace(channel, team.as_deref(), &message);
                    }
                }
            }
            Incoming::Acknowledge(_) | Incoming::Ignore => {}
        }
    }

    /// The platform greeted on a socket: the connection is up.
    ///
    /// Only on the socket the listener is waiting on. A greeting on one
    /// being drained is nothing, and so is one on a socket the listener has
    /// already moved past.
    fn greeted(&mut self, listening: Listening, socket: EffectId, at: u64) {
        let Some(listener) = self.listeners.get_mut(&listening) else {
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
        let as_user = match &listener.voices {
            Voices::Own(voice) => voice
                .us
                .as_ref()
                .map_or("?", |us| us.user.as_str())
                .to_owned(),
            Voices::Workspaces(voices) => format!("{} workspace(s)", voices.len()),
        };
        tracing::info!(%listening, %as_user, "listening");
        // Warned rather than logged, and only when there was genuinely
        // nothing listening: this is the whole of what a lost message looks
        // like from in here — a window, its length, and nothing else.
        if let Some(since) = listener.deaf_since.take() {
            if let Some(deaf_for) = at.checked_sub(since) {
                tracing::warn!(
                    %listening,
                    deaf_for_ms = deaf_for,
                    "nothing was listening on this channel for that long; anything said in the \
                     window was not heard, and the platform does not send it again"
                );
            } else {
                tracing::warn!(
                    %listening,
                    "nothing was listening on this channel for a while; anything said in the \
                     window was not heard, and the platform does not send it again"
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
    fn refresh(&mut self, listening: Listening, socket: EffectId, effects: &mut Vec<Effect>) {
        let up = self
            .listeners
            .get(&listening)
            .is_some_and(|listener| listener.phase == Phase::Listening { socket });
        if !up {
            return;
        }
        self.draining.insert(socket);
        self.again(listening, effects);
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
        let Some(listening) = self.sockets.remove(&id) else {
            tracing::warn!("a socket this instance did not open ended; ignored");
            return;
        };
        if self.draining.remove(&id) {
            tracing::debug!(%listening, "a replaced connection ended");
            return;
        }
        let Some(listener) = self.listeners.get_mut(&listening) else {
            tracing::debug!(%listening, "the socket of a channel no longer listened to ended");
            return;
        };
        match disconnected {
            // An ordinary end, which is most of them: the platform closes a
            // long-lived connection on a schedule of its own and does not
            // always say goodbye first, so this is the common path rather
            // than a fault — and reporting it as one is how somebody learns
            // to ignore what this reports.
            Disconnected::Closed => {
                tracing::info!(%listening, "the connection ended; opening another");
            }
            Disconnected::Failed(why) => {
                tracing::warn!(%listening, %why, "the channel stopped being readable");
            }
        }
        listener.deaf_since.get_or_insert(at);
        self.try_again_later(listening, effects);
    }

    /// Stops listening with an app: every socket of its is disconnected,
    /// and its timer forgotten.
    ///
    /// A question in flight answers to nobody afterwards, and the sockets
    /// stay known until each says it has ended, so that their ends are
    /// placed rather than wondered about.
    pub fn stop_listening(&mut self, listening: Listening) -> Vec<Effect> {
        let Some(listener) = self.listeners.remove(&listening) else {
            return Vec::new();
        };
        if let Phase::Waiting { timer } = listener.phase {
            self.timers.remove(&timer);
        }
        self.sockets
            .iter()
            .filter(|(_, whose)| **whose == listening)
            .map(|(id, _)| Generic::Disconnect { id: *id })
            .collect()
    }

    /// Who this instance is on a project's channel, once its listener has
    /// been told: its own app's identity, or its workspace's on the
    /// instance's app.
    #[must_use]
    pub fn identity_of(&self, project: ProjectId, channel: Channel) -> Option<&Identity> {
        let (listening, team) = match self.state.projects.get(&project)?.channels.get(&channel)? {
            Binding::Own(_) => (Listening::Own(project), None),
            Binding::Workspace(team) => (Listening::App(channel), Some(team.as_str())),
        };
        self.listeners
            .get(&listening)?
            .voices
            .voice(team)?
            .us
            .as_ref()
    }

    /// Whose app a bot identity already is, where a listener hears with
    /// it: a project's own, by the project's name, or the instance's, by
    /// the workspace it is installed on. What refuses a binding of a
    /// project's own that would make a second connection on one app, per
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`:
    /// the platform hands each event to one connection, so the second
    /// would hear half of what is said. A project's own app heard again for
    /// that project is not another's, which is what `except` says.
    #[must_use]
    pub fn already_heard_as(
        &self,
        channel: Channel,
        us: &Identity,
        except: Option<ProjectId>,
    ) -> Option<String> {
        self.listeners
            .iter()
            .filter(|(_, listener)| listener.channel == channel)
            .filter(
                |(listening, _)| !matches!(listening, Listening::Own(own) if Some(*own) == except),
            )
            .find_map(|(listening, listener)| {
                let (team, _) = listener
                    .voices
                    .identities()
                    .into_iter()
                    .find(|(_, known)| known.bot == us.bot)?;
                Some(match listening {
                    Listening::Own(project) => {
                        let name = self
                            .state
                            .projects
                            .get(project)
                            .map_or_else(|| project.to_string(), |watched| watched.name.clone());
                        format!("{name}'s app")
                    }
                    Listening::App(_) => {
                        let workspace = team
                            .and_then(|team| {
                                self.state.channel_apps.get(&channel)?.workspaces.get(team)
                            })
                            .map_or("a workspace", |workspace| workspace.name.as_str());
                        format!("the instance's own app, installed on {workspace}")
                    }
                })
            })
    }

    /// A workspace was installed on the instance's app: heard from now,
    /// with a voice of its own, and without the connection being made
    /// again — the socket is the app's, and a voice only needs to be told
    /// who it is. The first workspace is what starts the listener.
    pub fn workspace_added(
        &mut self,
        channel: Channel,
        team: &str,
        speaking: &Speaking,
    ) -> Vec<Effect> {
        let listening = Listening::App(channel);
        let Some(listener) = self.listeners.get_mut(&listening) else {
            return self.listen(listening);
        };
        let Voices::Workspaces(voices) = &mut listener.voices else {
            return Vec::new();
        };
        voices.insert(
            team.to_owned(),
            Voice {
                speaking: speaking.clone(),
                us: None,
            },
        );
        let (id, question) = self.introducing(listening, channel, Some(team.to_owned()), speaking);
        if let Some(listener) = self.listeners.get_mut(&listening)
            && let Phase::Introducing { asked } = &mut listener.phase
        {
            asked.insert(id);
        }
        vec![question]
    }

    /// A workspace was forgotten: its voice with it, and the connection
    /// when no voice is left to hear with.
    pub fn workspace_dropped(&mut self, channel: Channel, team: &str) -> Vec<Effect> {
        let listening = Listening::App(channel);
        let Some(listener) = self.listeners.get_mut(&listening) else {
            return Vec::new();
        };
        if let Voices::Workspaces(voices) = &mut listener.voices {
            voices.remove(team);
        }
        if listener.voices.is_empty() {
            self.stop_listening(listening)
        } else {
            Vec::new()
        }
    }
}
