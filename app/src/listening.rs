//! Hearing what a channel says back.
//!
//! The transport half of `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`:
//! this process opens a websocket outward and events arrive on it, because the
//! alternative wants a public HTTPS endpoint and `docs/vision.md` §3 has this
//! running on somebody's own machine.
//!
//! **Deciding is split from listening, as everywhere else here.** [`decode`]
//! turns one frame into what should happen and is pure; the loop below only
//! reads, acknowledges and hands over. Which job a message is *for* is not
//! decided here at all — that is `State::recipient`, in the domain, where it
//! can be tested against every combination rather than the ones a live
//! workspace happens to produce.

use stageman_core::Arriving;

/// Who this instance is on a channel, so it can recognise itself.
///
/// One identifier, fetched once when a connection opens. It answers both
/// questions the routing rule asks about the speaker: whether a message
/// mentions this instance, and whether this instance is what said it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// What a mention of this instance looks up to.
    pub user: String,
}

/// What one frame means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// The connection is up and nothing needs doing.
    Ready,
    /// The platform is about to close this connection and expects another.
    ///
    /// Not a failure. It says so on a schedule of its own, and a listener that
    /// treated it as one would reconnect just as often while logging alarm.
    Reconnect,
    /// Somebody said something, and the envelope it arrived in.
    Said {
        /// What has to be acknowledged, whatever is decided about the message.
        envelope: String,
        /// What identifies this message, which is the thread a foreman answers
        /// in when the message is at the root.
        id: String,
        /// What was said.
        text: String,
        /// Where it was said, and in which thread if any.
        address: String,
        /// The thread, if it was in one.
        thread: Option<String>,
        /// Whether it named this instance.
        mentions: bool,
        /// Whether this instance is what said it.
        from_us: bool,
    },
    /// Something that needs acknowledging and nothing else.
    ///
    /// **Acknowledging is not optional and not politeness.** The platform
    /// redelivers whatever goes unacknowledged, so a frame dropped silently is
    /// the same message arriving again rather than a message lost — and a
    /// message this instance chose not to act on would arrive for ever.
    Acknowledge(String),
    /// A frame this does not understand, carrying nothing to acknowledge.
    Ignore,
}

/// What a frame from the socket means.
///
/// Pure, so that every shape the platform sends can be tested without one.
#[must_use]
pub fn decode(frame: &str, us: &Identity) -> Incoming {
    let Ok(envelope) = serde_json::from_str::<Envelope>(frame) else {
        return Incoming::Ignore;
    };

    match envelope.kind.as_deref() {
        Some("hello") => return Incoming::Ready,
        Some("disconnect") => return Incoming::Reconnect,
        _ => {}
    }

    let Some(id) = envelope.envelope_id else {
        return Incoming::Ignore;
    };
    let Some(said) = envelope.payload.and_then(|payload| payload.event) else {
        return Incoming::Acknowledge(id);
    };

    // Only a plain message is a message. An edit, a deletion, somebody joining
    // — all arrive as this type with a subtype, and none of them is somebody
    // talking to a job. A bot's own message is the one subtype worth reading,
    // because recognising it is how the loop below is prevented.
    let plain = said.subtype.is_none();
    let ours = said.bot_id.is_some() || said.subtype.as_deref() == Some("bot_message");
    if said.kind.as_deref() != Some("message") || !(plain || ours) {
        return Incoming::Acknowledge(id);
    }

    let Some(address) = said.channel else {
        return Incoming::Acknowledge(id);
    };

    let Some(spoken) = said.ts else {
        // Every message has one. Without it there is nothing to answer under,
        // so it is acknowledged and dropped rather than routed to a thread
        // that cannot be addressed.
        return Incoming::Acknowledge(id);
    };

    Incoming::Said {
        envelope: id,
        mentions: said.text.contains(&format!("<@{}>", us.user)),
        from_us: ours || said.user.as_deref() == Some(us.user.as_str()),
        id: spoken,
        text: said.text,
        address,
        thread: said.thread_ts,
    }
}

impl Incoming {
    /// The message as the routing rule wants it, if this is one.
    #[must_use]
    pub fn arriving(&self) -> Option<Arriving<'_>> {
        match self {
            Self::Said {
                id,
                address,
                thread,
                mentions,
                from_us,
                ..
            } => Some(Arriving {
                address,
                id,
                thread: thread.as_deref(),
                mentions: *mentions,
                from_us: *from_us,
            }),
            _ => None,
        }
    }
}

/// One frame, as much of it as anything here reads.
#[derive(serde::Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    /// Named by the platform rather than by this project, which is why the
    /// lint about repeating the type's name is answered here rather than
    /// obeyed: renaming it would need a `serde` attribute saying the real name
    /// anyway, and then the wire name would appear twice.
    #[expect(
        clippy::struct_field_names,
        reason = "the wire chooses this name, not this project"
    )]
    envelope_id: Option<String>,
    payload: Option<Payload>,
}

#[derive(serde::Deserialize)]
struct Payload {
    event: Option<Said>,
}

#[derive(serde::Deserialize)]
struct Said {
    #[serde(rename = "type")]
    kind: Option<String>,
    subtype: Option<String>,
    channel: Option<String>,
    user: Option<String>,
    bot_id: Option<String>,
    #[serde(default)]
    text: String,
    ts: Option<String>,
    thread_ts: Option<String>,
}

/// Where a project's replies arrive from, and what it needs to get there.
///
/// Held together because a listener needs all three and none of them alone:
/// the app-level credential opens the socket, the speaking half answers on it,
/// and the project is what a message is routed against.
#[cfg(feature = "server")]
pub struct Listening {
    /// The project whose channel this is.
    pub project: stageman_core::ProjectId,
    /// What opens the socket. Never leaves this process.
    pub opening: stageman_core::Secret,
    /// What posts, and what asks the platform who this instance is.
    pub speaking: stageman_core::Speaking,
}

/// Asks the platform for somewhere to connect, and for who this instance is.
///
/// Two calls rather than one, because they answer with different credentials:
/// the app-level one opens a socket and knows nothing about a channel, and the
/// bot one is what a mention names. Both happen once per connection rather than
/// once per message.
///
/// # Errors
///
/// Fails if either call cannot be reached or is refused.
#[cfg(feature = "server")]
#[mutants::skip]
pub async fn introduce(listening: &Listening) -> Result<Identity, crate::channel::ChannelError> {
    let told = crate::channel::ask(
        "https://slack.com/api/auth.test",
        &listening.speaking.credential,
    )
    .await?;
    let user = told
        .get("user_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(crate::channel::ChannelError::NoIdentifier)?;
    Ok(Identity {
        user: user.to_owned(),
    })
}

/// Where to connect, for this project.
///
/// # Errors
///
/// Fails if the platform cannot be reached or refuses the credential.
#[cfg(feature = "server")]
#[mutants::skip]
async fn open_socket(
    opening: &stageman_core::Secret,
) -> Result<String, crate::channel::ChannelError> {
    let told = crate::channel::ask("https://slack.com/api/apps.connections.open", opening).await?;
    told.get("url")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(crate::channel::ChannelError::NoIdentifier)
}

/// Opens one connection and reads it until it ends.
///
/// **Skipped by mutation testing, and thin enough to justify it.** Every
/// decision is in [`decode`] and in `State::recipient`, both pure and both
/// tested; what is here is a request, a socket, and a loop that hands over.
///
/// Returns when the platform asks for a new connection or the socket closes,
/// which are the same thing to a caller: open another.
///
/// # Errors
///
/// Fails if the connection cannot be opened or the socket breaks.
#[cfg(feature = "server")]
#[mutants::skip]
pub async fn attend(
    store: &crate::Store,
    runtime: &stageman_agent::ContainerRuntime,
    listening: &Listening,
    us: &Identity,
) -> Result<(), crate::channel::ChannelError> {
    use futures_util::{SinkExt as _, StreamExt as _};

    let url = open_socket(&listening.opening).await?;
    let (mut socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|failure| crate::channel::ChannelError::Unreachable(failure.to_string()))?;
    // Said at all because the alternative was silence. A listener that
    // connects and is sent nothing looked exactly like one discarding every
    // frame, and the difference is entirely on the platform's side — an event
    // subscription that was never added. Neither is a failure, so neither
    // warned, so nothing was written anywhere.
    tracing::info!(
        project = %listening.project,
        address = %listening.speaking.address,
        as_user = %us.user,
        "listening"
    );

    while let Some(frame) = socket.next().await {
        let frame = match frame {
            Ok(frame) => frame,
            // An ordinary end, which is most of them. The platform closes a
            // long-lived connection on a schedule of its own and does not
            // always say goodbye first, so this is the common path rather than
            // a fault — and reporting it as one is how somebody learns to
            // ignore what this reports.
            Err(why) if ordinary_end(&why) => {
                tracing::info!(%why, "the connection ended; opening another");
                return Ok(());
            }
            Err(why) => {
                return Err(crate::channel::ChannelError::Unreachable(why.to_string()));
            }
        };
        let tokio_tungstenite::tungstenite::Message::Text(text) = frame else {
            continue;
        };

        let heard = decode(text.as_str(), us);
        // Every frame, at a level nobody runs by default. It is the only place
        // that can say whether the platform is sending anything at all, which
        // is the first question when a reply does not arrive.
        tracing::debug!(frame = %text.as_str(), "heard a frame");
        // Before acting, always. The platform redelivers what is not
        // acknowledged, and acting can take as long as an agent takes.
        if let Some(envelope) = acknowledging(&heard) {
            let answer = serde_json::json!({ "envelope_id": envelope });
            socket
                .send(tokio_tungstenite::tungstenite::Message::text(
                    answer.to_string(),
                ))
                .await
                .map_err(|failure| {
                    crate::channel::ChannelError::Unreachable(failure.to_string())
                })?;
        }

        match heard {
            Incoming::Reconnect => return Ok(()),
            Incoming::Said {
                ref text,
                ref address,
                ref thread,
                mentions,
                from_us,
                ..
            } => {
                // Anything this instance said is the loop guard working, and
                // there is one of these for every message it sends. Said at
                // debug so that what is left at info is somebody talking to
                // it.
                if from_us {
                    tracing::debug!(%address, "heard itself, and ignored it");
                } else {
                    tracing::info!(
                        %address,
                        thread = thread.as_deref().unwrap_or("(root)"),
                        mentions,
                        "heard somebody speak"
                    );
                }
                let text = text.clone();
                act(store, runtime, listening, &heard, &text).await;
            }
            Incoming::Ready | Incoming::Acknowledge(_) | Incoming::Ignore => {}
        }
    }
    Ok(())
}

/// Whether a socket ending this way is the ordinary case.
///
/// **Most endings are.** A long-lived connection is closed by the platform on
/// its own schedule, and it does not always complete a closing handshake
/// first — a reset arrives as a protocol error and means nothing has gone
/// wrong. Told apart from a real failure so that a warning stays worth
/// reading: this is logged as the routine thing it is, and anything else is
/// not.
fn ordinary_end(why: &tokio_tungstenite::tungstenite::Error) -> bool {
    use tokio_tungstenite::tungstenite::{Error, error::ProtocolError};

    match why {
        // A close the platform completed, and one it did not bother to —
        // the same event, and which arrives depends on timing rather than on
        // anything worth telling apart.
        Error::ConnectionClosed
        | Error::AlreadyClosed
        | Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
        // A peer that vanished mid-read is the same event seen one layer
        // lower, which is which depending on timing rather than on anything
        // meaningful.
        Error::Io(failure) => matches!(
            failure.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
        ),
        _ => false,
    }
}

/// The envelope a frame has to be answered with, if it has one.
fn acknowledging(heard: &Incoming) -> Option<&str> {
    match heard {
        Incoming::Said { envelope, .. } | Incoming::Acknowledge(envelope) => Some(envelope),
        Incoming::Ready | Incoming::Reconnect | Incoming::Ignore => None,
    }
}

/// Hands one message to whoever it is for.
#[cfg(feature = "server")]
#[mutants::skip]
async fn act(
    store: &crate::Store,
    runtime: &stageman_agent::ContainerRuntime,
    listening: &Listening,
    heard: &Incoming,
    said: &str,
) {
    let Some(arriving) = heard.arriving() else {
        return;
    };
    let destination = {
        let state = store.read();
        let found = state.recipient(stageman_core::Channel::Slack, &arriving);
        drop(state);
        found
    };

    match destination {
        stageman_core::Recipient::Job(job) => {
            tracing::info!(%job, "handing a reply to the job whose thread it is in");
            let framed = stageman_foreman::reply(said);
            drop(crate::deliver(store, runtime, job, &framed).await);
        }
        stageman_core::Recipient::Foreman(project) => {
            // A message at the root is the parent of the thread its answer
            // belongs under; one in a thread is answered where it was said.
            // Either way the thread is decided here, when the message arrives,
            // rather than when its turn starts — by then it may be several
            // turns back.
            let thread = stageman_core::Thread {
                channel: stageman_core::Channel::Slack,
                id: arriving.thread.unwrap_or(arriving.id).to_owned(),
            };
            tracing::info!(%project, "a message for the foreman");
            crate::attend(
                store,
                runtime,
                project,
                stageman_core::Errand {
                    said: said.to_owned(),
                    thread,
                },
            )
            .await;
        }
        // Answered rather than ignored. They addressed this instance, so
        // silence would read as broken — and this is where somebody lands by
        // replying to a foreman's own message.
        stageman_core::Recipient::NoSuchJob(project) => {
            let Some(thread) = arriving.thread.map(|id| stageman_core::Thread {
                channel: stageman_core::Channel::Slack,
                id: id.to_owned(),
            }) else {
                return;
            };
            tracing::info!(%project, "a message in a thread belonging to no job");
            if let Err(why) = crate::channel::say_in(
                &listening.speaking,
                &thread,
                stageman_foreman::no_such_job_notice(),
            )
            .await
            {
                tracing::warn!(%why, "the thread could not be answered");
            }
        }
        // Ordinary — most of what is said in a project's channel is people
        // talking to each other. Said at debug so that "nothing happened" can
        // be told apart from "nothing arrived".
        stageman_core::Recipient::Nobody => {
            tracing::debug!("nobody that message was for");
        }
    }
}

/// Starts listening on every project that has somewhere to listen.
///
/// One task per project, each reopening its own connection: the platform ends
/// a connection on a schedule of its own, so reconnecting is the ordinary case
/// rather than error handling. A project whose credential has stopped working
/// keeps trying and says so in the log, because
/// `docs/conventions.md` §3 puts a broken credential in front of an operator
/// rather than stopping the instance that would let them fix it.
///
/// Reads the projects once, which is all a startup needs. A project bound
/// *afterwards* gets its own listener from [`listen_to`], called where the
/// binding is made — because a listener that only ever started at startup made
/// binding a channel appear to do nothing until the daemon was restarted, and
/// nothing said so.
#[cfg(feature = "server")]
#[mutants::skip]
pub fn listen(
    store: &std::sync::Arc<crate::Store>,
    runtime: &'static stageman_agent::ContainerRuntime,
) {
    let listening: Vec<Listening> = {
        let state = store.read();
        let found = state
            .projects
            .keys()
            .filter_map(|project| listening_on(&state, *project))
            .collect();
        drop(state);
        found
    };

    for one in listening {
        listen_to(store, runtime, one);
    }
}

/// Listens on one project's channel, reopening its connection for ever.
///
/// Split out so that binding a channel can start listening on it immediately.
/// The platform ends a connection on a schedule of its own, so reopening is
/// the ordinary case rather than error handling.
#[cfg(feature = "server")]
#[mutants::skip]
pub fn listen_to(
    store: &std::sync::Arc<crate::Store>,
    runtime: &'static stageman_agent::ContainerRuntime,
    one: Listening,
) {
    let store = std::sync::Arc::clone(store);
    tokio::spawn(async move {
        let project = one.project;
        loop {
            match introduce(&one).await {
                Ok(us) => {
                    if let Err(why) = attend(&store, runtime, &one, &us).await {
                        tracing::warn!(%project, %why, "the channel stopped being readable");
                    }
                }
                Err(why) => {
                    tracing::warn!(%project, %why, "the channel could not be listened to");
                }
            }
            // A fixed wait rather than a backoff. Reconnecting is the ordinary
            // case here, so the common path must not grow a delay that
            // compounds; a credential that is simply wrong retries at this
            // rate for ever, which is cheap and visible in the log.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}

/// What to listen to on one project, if there is anything.
///
/// The same selection [`listen`] makes across every project, as a function so
/// that a project bound after startup is heard by exactly the rule that would
/// have heard it at startup — rather than by a second one written beside it.
#[cfg(feature = "server")]
#[must_use]
pub fn listening_on(
    state: &stageman_core::State,
    project: stageman_core::ProjectId,
) -> Option<Listening> {
    let bound = state
        .projects
        .get(&project)?
        .channels
        .get(&stageman_core::Channel::Slack)?;
    Some(Listening {
        project,
        opening: bound.listen_credential.clone()?,
        speaking: bound.speaking(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Identity, Incoming, decode};

    fn us() -> Identity {
        Identity {
            user: "U0BOT".to_owned(),
        }
    }

    fn said(frame: &str) -> Incoming {
        decode(frame, &us())
    }

    /// The two frames that are about the connection rather than a message.
    #[test]
    fn a_greeting_is_ready_and_a_disconnect_is_a_reconnection() {
        assert_eq!(
            said(r#"{"type":"hello","num_connections":1}"#),
            Incoming::Ready
        );
        assert_eq!(
            said(r#"{"type":"disconnect","reason":"warning"}"#),
            Incoming::Reconnect
        );
    }

    /// A plain message from a person, in a thread.
    #[test]
    fn a_reply_in_a_thread_carries_everything_routing_needs() {
        let frame = r#"{"envelope_id":"e-1","type":"events_api","payload":{"event":{
            "type":"message","channel":"C0123","user":"U0HUMAN",
            "text":"use postgres","ts":"1788000001.000001","thread_ts":"1728312345.678901"}}}"#;

        let Incoming::Said {
            envelope,
            id,
            text,
            address,
            thread,
            mentions,
            from_us,
        } = said(frame)
        else {
            panic!("expected a message, got {:?}", said(frame));
        };

        assert_eq!(envelope, "e-1");
        // The message's own identifier, which is what a foreman would answer
        // under if this had been at the root.
        assert_eq!(id, "1788000001.000001");
        assert_eq!(text, "use postgres");
        assert_eq!(address, "C0123");
        // A string, never a number: parsed as one it addresses no message.
        assert_eq!(thread.as_deref(), Some("1728312345.678901"));
        assert!(!mentions);
        assert!(!from_us);
    }

    /// A mention is recognised by identifier rather than by name.
    ///
    /// The rendered form is the identifier, so somebody typing the bot's
    /// display name is not a mention and does not become one.
    #[test]
    fn a_mention_is_the_rendered_identifier() {
        let frame = |text: &str| {
            format!(
                r#"{{"envelope_id":"e-2","payload":{{"event":{{
                "type":"message","channel":"C0123","user":"U0HUMAN","ts":"1788000002.000002","text":"{text}"}}}}}}"#
            )
        };

        let mentioned = said(&frame("<@U0BOT> what is happening"));
        assert!(
            mentioned.arriving().expect("a message").mentions,
            "{mentioned:?}"
        );

        for missing in ["stageman what is happening", "<@U0SOMEBODYELSE> hello"] {
            let plain = said(&frame(missing));
            assert!(
                !plain.arriving().expect("a message").mentions,
                "{missing} is not a mention"
            );
        }
    }

    /// Anything this instance said is marked as its own.
    ///
    /// Two ways of telling, because a message from this app arrives carrying a
    /// bot identifier and the subtype that goes with it, and a message from
    /// the user the token belongs to arrives as an ordinary one.
    #[test]
    fn anything_this_instance_said_is_recognised_as_its_own() {
        let by_bot = said(
            r#"{"envelope_id":"e-3","payload":{"event":{"type":"message","subtype":"bot_message",
            "channel":"C0123","bot_id":"B0SELF","ts":"1788000003.000003","text":"⚠️ Check this out."}}}"#,
        );
        assert!(by_bot.arriving().expect("a message").from_us);

        let by_user = said(
            r#"{"envelope_id":"e-4","payload":{"event":{"type":"message",
            "channel":"C0123","user":"U0BOT","ts":"1788000004.000004","text":"hello"}}}"#,
        );
        assert!(by_user.arriving().expect("a message").from_us);

        // Either marker alone is enough, and both have to be, because they do
        // not always arrive together: a message posted through the API carries
        // the identifier, and one the platform renders as a bot carries the
        // subtype. Requiring both would let one shape through and reopen the
        // loop this exists to close.
        let identifier_only = said(
            r#"{"envelope_id":"e-9","payload":{"event":{"type":"message",
            "channel":"C0123","bot_id":"B0SELF","ts":"1788000005.000005","text":"posted"}}}"#,
        );
        assert!(
            identifier_only.arriving().expect("a message").from_us,
            "{identifier_only:?}"
        );

        let subtype_only = said(
            r#"{"envelope_id":"e-10","payload":{"event":{"type":"message",
            "subtype":"bot_message","channel":"C0123","ts":"1788000006.000006","text":"rendered"}}}"#,
        );
        assert!(
            subtype_only.arriving().expect("a message").from_us,
            "{subtype_only:?}"
        );
    }

    /// Everything with an envelope is acknowledged, and nothing else is.
    ///
    /// **The most consequential small function here.** The platform redelivers
    /// what goes unacknowledged, so answering nothing means every message
    /// arrives for ever — and answering the wrong envelope means the right one
    /// never stops. Neither shows up as a failure anywhere.
    #[test]
    fn exactly_what_carries_an_envelope_is_acknowledged() {
        use super::acknowledging;

        let reply = said(
            r#"{"envelope_id":"e-11","payload":{"event":{"type":"message",
            "channel":"C0123","user":"U0HUMAN","ts":"1788000007.000007","text":"hello"}}}"#,
        );
        assert_eq!(acknowledging(&reply), Some("e-11"));

        assert_eq!(
            acknowledging(&Incoming::Acknowledge("e-12".to_owned())),
            Some("e-12")
        );

        // Nothing to answer, and answering anyway would be answering an
        // envelope that does not exist.
        for nothing in [Incoming::Ready, Incoming::Reconnect, Incoming::Ignore] {
            assert_eq!(acknowledging(&nothing), None, "{nothing:?}");
        }
    }

    /// Everything else is acknowledged and nothing more.
    ///
    /// Acknowledging is the load-bearing half: the platform redelivers what is
    /// not acknowledged, so a frame this chooses to ignore has to be answered
    /// or it arrives for ever.
    #[test]
    fn what_is_not_acted_on_is_still_acknowledged() {
        for frame in [
            // An edit, which is not somebody talking to a job.
            r#"{"envelope_id":"e-5","payload":{"event":{"type":"message","subtype":"message_changed","channel":"C0123"}}}"#,
            // Somebody joining.
            r#"{"envelope_id":"e-6","payload":{"event":{"type":"member_joined_channel","channel":"C0123"}}}"#,
            // A message with nowhere attached to it.
            r#"{"envelope_id":"e-7","payload":{"event":{"type":"message","user":"U0HUMAN","text":"hi"}}}"#,
            // An envelope carrying no event at all.
            r#"{"envelope_id":"e-8","payload":{}}"#,
        ] {
            assert!(
                matches!(said(frame), Incoming::Acknowledge(_)),
                "{frame} must still be acknowledged"
            );
        }
    }

    /// A project is listened to only when it has both halves.
    #[test]
    fn a_project_is_listened_to_only_when_it_can_be() {
        use stageman_core::{
            Agent, Channel, ChannelConfig, Project, ProjectId, Secret, State, Uuid,
        };
        use std::collections::{BTreeMap, BTreeSet};

        let bound = |listens: bool| Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_agent: Agent::Claude,
            job_agents: BTreeSet::from([Agent::Claude]),
            credentials: BTreeMap::new(),
            channels: BTreeMap::from([(
                Channel::Slack,
                ChannelConfig {
                    address: "C0123456789".to_owned(),
                    credential: Secret::new("xoxb-token".to_owned()),
                    listen_credential: listens.then(|| Secret::new("xapp-token".to_owned())),
                },
            )]),
            jobs: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
        };
        let id = ProjectId::from_uuid(Uuid::from_u128(1));

        let mut state = State::default();
        assert!(
            super::listening_on(&state, id).is_none(),
            "a project this instance does not have"
        );

        state.projects.insert(id, bound(false));
        assert!(
            super::listening_on(&state, id).is_none(),
            "bound is not the same as listening"
        );

        state.projects.insert(id, bound(true));
        let listening = super::listening_on(&state, id).expect("somewhere to listen");
        assert_eq!(listening.project, id);
        assert_eq!(listening.speaking.address, "C0123456789");
        // The speaking half, so the credential that opens the socket cannot
        // travel with the one that posts.
        assert_eq!(listening.speaking.credential.expose(), "xoxb-token");
        assert_eq!(listening.opening.expose(), "xapp-token");
    }

    /// A connection ending is usually nothing, and has to be told apart.
    ///
    /// **Found by reading a real log.** A reset arrived as a protocol error
    /// every few minutes, was reported as a warning, and the listener quietly
    /// reconnected and carried on — so the warning meant nothing, which is how
    /// somebody learns to skip past the one that does.
    #[test]
    fn an_ordinary_disconnection_is_not_a_failure() {
        use tokio_tungstenite::tungstenite::{Error, error::ProtocolError};

        for ending in [
            Error::ConnectionClosed,
            Error::AlreadyClosed,
            Error::Protocol(ProtocolError::ResetWithoutClosingHandshake),
            Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
            Error::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
        ] {
            assert!(super::ordinary_end(&ending), "{ending:?} is how they end");
        }

        for broken in [
            Error::Protocol(ProtocolError::HandshakeIncomplete),
            Error::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            Error::AttackAttempt,
        ] {
            assert!(
                !super::ordinary_end(&broken),
                "{broken:?} is worth somebody reading"
            );
        }
    }

    /// A frame with nothing to acknowledge is dropped rather than guessed at.
    #[test]
    fn a_frame_that_is_not_understood_is_ignored() {
        for frame in ["not json at all", "{}", r#"{"type":"something_new"}"#] {
            assert_eq!(said(frame), Incoming::Ignore, "{frame}");
        }
    }
}
