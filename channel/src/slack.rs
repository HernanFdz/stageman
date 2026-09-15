//! Slack: one message posted, an event stream opened, and what each
//! answers.
//!
//! The event stream is Socket Mode, an outbound websocket this process opens
//! — the alternative wants a public HTTPS endpoint, and this is a daemon on
//! somebody's own machine, per
//! `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`. Two questions
//! come before it: who this instance is, asked with the credential that
//! speaks, because that is what a mention names and what its own posts
//! carry; and where to connect, asked with the app-level credential, because
//! that is the one that opens a stream and it never leaves the daemon.
//!
//! **A person is read from the platform's own mention event and from nothing
//! else.** A mention arrives twice — once on the message subscription and
//! once as an `app_mention` — and only the second is delivered when the
//! mention is what got the app invited into the room. So the message
//! subscription is read for one thing, what this instance said itself, which
//! is how its own posts are told apart; a person's copy there is
//! acknowledged and dropped. See
//! `docs/decisions/0060-a-binding-is-a-workspace.md`.
//!
//! **A thread has no independent existence**, which is why a job's thread
//! is opened by the daemon rather than by the job. There is no call that
//! creates an empty one: a thread is a message plus the replies hanging from
//! it, so somebody posts that message before the job's container starts,
//! and the identifier it comes back with is what a reply names — see
//! `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.

use stageman_core::{Channel, Secret, Speaking};

use crate::{Call, ChannelError, Identity, Incoming, Message, Request};

/// Where Slack takes a message.
const POST_MESSAGE: &str = "https://slack.com/api/chat.postMessage";

/// Where Slack says who a credential is.
const WHO_AM_I: &str = "https://slack.com/api/auth.test";

/// Where Slack says where to connect for the event stream.
const OPEN_SOCKET: &str = "https://slack.com/api/apps.connections.open";

/// A question with one credential and no arguments, which is the shape of
/// both calls a listener makes before it can read anything.
fn asking(url: &str, credential: &Secret) -> Request {
    Request {
        method: "POST".to_owned(),
        url: url.to_owned(),
        headers: [(
            "authorization".to_owned(),
            format!("Bearer {}", credential.expose()),
        )]
        .into(),
        body: None,
    }
}

/// Renders asking who this instance is, with the credential that speaks.
pub fn who_am_i(speaking: &Speaking) -> Request {
    asking(WHO_AM_I, &speaking.credential)
}

/// Renders asking where to connect, with the credential that opens a stream.
pub fn open_socket(opening: &Secret) -> Request {
    asking(OPEN_SOCKET, opening)
}

/// Who this instance is, from the answer to [`who_am_i`].
///
/// Both identifiers, and the second is what makes the credential a bot's: a
/// user token is answered without one, and a user token could not tell its
/// own posts from anybody else's, so it is refused here rather than
/// connected and left to read its own words back.
pub fn identity(status: u16, body: &[u8]) -> Result<Identity, ChannelError> {
    let told = accepted(status, body)?;
    let user = told
        .get("user_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(ChannelError::NoAnswer)?;
    let bot = told
        .get("bot_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(ChannelError::NotABot)?;
    Ok(Identity {
        user: user.to_owned(),
        bot: bot.to_owned(),
    })
}

/// Where to connect, from the answer to [`open_socket`].
pub fn socket_url(status: u16, body: &[u8]) -> Result<String, ChannelError> {
    let told = accepted(status, body)?;
    told.get("url")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(ChannelError::NoAnswer)
}

/// What an answer to a question means, stopping at the whole answer.
///
/// The guard that matters — a refusal arriving as 200 — is the same guard
/// [`posted`] keeps, and it is kept here too because this is separate code
/// that would fail in a way nobody would attribute to a missing check: it
/// would connect to a URL that is not there.
fn accepted(status: u16, body: &[u8]) -> Result<serde_json::Value, ChannelError> {
    if !(200..300).contains(&status) {
        return Err(ChannelError::Unreachable(format!(
            "the channel answered {status}"
        )));
    }
    let answered: serde_json::Value = serde_json::from_slice(body)
        .map_err(|failure| ChannelError::Unreadable(failure.to_string()))?;
    if answered.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(ChannelError::Refused(
            answered
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no reason given")
                .to_owned(),
        ));
    }
    Ok(answered)
}

/// The frame that acknowledges an envelope.
pub fn acknowledgement(envelope: &str) -> String {
    serde_json::json!({ "envelope_id": envelope }).to_string()
}

/// What a frame from the socket means.
///
/// Pure, so that every shape the platform sends can be tested without one.
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

    let from_us = match said.kind.as_deref() {
        // Somebody mentioning this instance, as its own event. It is what
        // makes a person's message arrive at all, and the one delivery that
        // reaches a room the app was invited into by that very mention.
        Some("app_mention") => said.user.as_deref() == Some(us.user.as_str()),
        // The message subscription is read for what this instance said
        // itself and for nothing else. A person's message here is the copy
        // of a mention that arrives as its own event above, and another
        // bot's is not read. Recognised by this instance's own identifiers,
        // never by being a bot: a bot's post carries the bot identifier,
        // and one from the user the token belongs to carries the user's.
        // Either alone is enough, since the two do not always arrive
        // together, and an edit or a broadcast is not a thing said.
        Some("message") => {
            let ours = said.bot_id.as_deref() == Some(us.bot.as_str())
                || said.user.as_deref() == Some(us.user.as_str());
            let plain = said.subtype.is_none() || said.subtype.as_deref() == Some("bot_message");
            if !(ours && plain) {
                return Incoming::Acknowledge(id);
            }
            true
        }
        _ => return Incoming::Acknowledge(id),
    };

    let Some(room) = said.channel else {
        return Incoming::Acknowledge(id);
    };

    let Some(spoken) = said.ts else {
        // Every message has one. Without it there is nothing to answer
        // under, so it is acknowledged and dropped rather than routed to a
        // thread that cannot be addressed.
        return Incoming::Acknowledge(id);
    };

    Incoming::Said {
        envelope: id,
        message: Message {
            from_us,
            id: spoken,
            text: said.text,
            room,
            thread: said.thread_ts,
        },
    }
}

#[derive(serde::Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    /// Named by the platform rather than by this project, which is why the
    /// lint about repeating the type's name is answered here rather than
    /// obeyed: renaming it would need a `serde` attribute saying the real
    /// name anyway, and then the wire name would appear twice.
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

/// Renders one message posted in a room: at its root, or in a thread there.
pub fn post(speaking: &Speaking, room: &str, text: &str, thread: Option<&str>) -> Request {
    // Built as a map rather than indexed into as a value: indexing into a
    // value is the operation the gate is right to call a panic, and a map
    // has nothing to fail on. The thread is left out rather than sent as
    // null, because the platform reads a present-but-empty field as an
    // address rather than as no address.
    let mut posting = serde_json::Map::new();
    posting.insert("channel".to_owned(), room.into());
    posting.insert("text".to_owned(), text.into());
    if let Some(thread) = thread {
        posting.insert("thread_ts".to_owned(), thread.into());
    }
    Request {
        method: "POST".to_owned(),
        url: POST_MESSAGE.to_owned(),
        headers: [
            (
                "authorization".to_owned(),
                format!("Bearer {}", speaking.credential.expose()),
            ),
            (
                "content-type".to_owned(),
                "application/json; charset=utf-8".to_owned(),
            ),
        ]
        .into(),
        body: Some(serde_json::Value::Object(posting).to_string().into_bytes()),
    }
}

/// What a request asks, if it is one this module rendered.
pub fn call(request: &Request) -> Option<Call> {
    if request.method != "POST" {
        return None;
    }
    match request.url.as_str() {
        WHO_AM_I => {
            return Some(Call::WhoAmI {
                channel: Channel::Slack,
            });
        }
        OPEN_SOCKET => {
            return Some(Call::OpenSocket {
                channel: Channel::Slack,
            });
        }
        POST_MESSAGE => {}
        _ => return None,
    }
    let posted: Posted = serde_json::from_slice(request.body.as_deref()?).ok()?;
    Some(Call::Post {
        channel: Channel::Slack,
        room: posted.channel,
        text: posted.text,
        thread: posted.thread_ts,
    })
}

/// What an answer from Slack means.
///
/// Pure, so that the guard below can be tested without a network — the split
/// the rest of this project uses wherever an I/O function would otherwise
/// hide a decision. It was not split when it was written, and mutation
/// testing deleted the negation from that guard without a single test
/// noticing.
///
/// Returns the identifier **as text**, never parsed. It looks like a number
/// and is not one: through a float it loses the microseconds and comes back
/// addressing no message, and the failure that produces reads like a
/// permissions problem rather than a rounding one.
pub fn posted(status: u16, body: &[u8]) -> Result<String, ChannelError> {
    if !(200..300).contains(&status) {
        return Err(ChannelError::Unreachable(format!(
            "the channel answered {status}"
        )));
    }

    let answered: Answered = serde_json::from_slice(body)
        .map_err(|failure| ChannelError::Unreadable(failure.to_string()))?;

    // **A refusal arrives as 200.** Slack reports a bad token or an unknown
    // channel in the body with a successful status, so a check of the status
    // alone calls every one of them a message delivered.
    if !answered.ok {
        return Err(ChannelError::Refused(
            answered
                .error
                .unwrap_or_else(|| "no reason given".to_owned()),
        ));
    }
    answered.ts.ok_or(ChannelError::NoIdentifier)
}

/// One message, as it was sent, read back.
#[derive(serde::Deserialize)]
struct Posted {
    channel: String,
    text: String,
    thread_ts: Option<String>,
}

/// What Slack says back.
#[derive(serde::Deserialize)]
struct Answered {
    ok: bool,
    /// Present when it worked, and the identifier of what was posted.
    ts: Option<String>,
    /// Present when it did not.
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{accepted, decode, identity, posted};
    use crate::{ChannelError, Identity, Incoming};

    fn us() -> Identity {
        Identity {
            user: "U0BOT".to_owned(),
            bot: "B0SELF".to_owned(),
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

    /// A person mentioning this instance in a thread, as the platform
    /// delivers it: its own event, carrying the thread it was said in.
    #[test]
    fn a_mention_in_a_thread_carries_everything_routing_needs() {
        let frame = r#"{"envelope_id":"e-1","type":"events_api","payload":{"event":{
            "type":"app_mention","channel":"C0123","user":"U0HUMAN",
            "text":"<@U0BOT> use postgres","ts":"1788000001.000001","thread_ts":"1728312345.678901"}}}"#;

        let Incoming::Said { envelope, message } = said(frame) else {
            panic!("expected a message, got {:?}", said(frame));
        };

        assert_eq!(envelope, "e-1");
        // The message's own identifier, which is what a foreman would
        // answer under if this had been at the root.
        assert_eq!(message.id, "1788000001.000001");
        assert_eq!(message.text, "<@U0BOT> use postgres");
        assert_eq!(message.room, "C0123");
        // A string, never a number: parsed as one it addresses no message.
        assert_eq!(message.thread.as_deref(), Some("1728312345.678901"));
        assert!(!message.from_us);
    }

    /// A person is read from the mention event, and the copy of the same
    /// message on the message subscription is acknowledged and dropped.
    ///
    /// Measured: a mention arrives on both, with the same identifier, and
    /// only the mention event is delivered when the mention is what got the
    /// app invited. Reading both would hand every mention over twice.
    #[test]
    fn a_person_is_read_from_the_mention_event_and_not_from_the_message_subscription() {
        let as_mention = r#"{"envelope_id":"e-2","payload":{"event":{"type":"app_mention",
            "channel":"C0123","user":"U0HUMAN","ts":"1788000002.000002","text":"<@U0BOT> hello"}}}"#;
        let as_message = r#"{"envelope_id":"e-3","payload":{"event":{"type":"message",
            "channel":"C0123","user":"U0HUMAN","ts":"1788000002.000002","text":"<@U0BOT> hello"}}}"#;

        assert!(matches!(said(as_mention), Incoming::Said { .. }));
        assert_eq!(said(as_message), Incoming::Acknowledge("e-3".to_owned()));
    }

    /// Anything this instance said is marked as its own, by its own
    /// identifiers.
    ///
    /// Two ways of telling, because a post made with the token carries the
    /// bot identifier, and a message from the user the token belongs to
    /// carries the user's. Either alone is enough, and both have to be,
    /// because they do not always arrive together: requiring both would let
    /// one shape through and reopen the loop this exists to close.
    #[test]
    fn anything_this_instance_said_is_recognised_as_its_own() {
        let from_us = |frame: &str| match said(frame) {
            Incoming::Said { message, .. } => message.from_us,
            other => panic!("expected a message: {other:?}"),
        };

        // As measured: a post made with the token, carrying both.
        assert!(from_us(
            r#"{"envelope_id":"e-4","payload":{"event":{"type":"message",
            "channel":"C0123","user":"U0BOT","bot_id":"B0SELF","ts":"1788000004.000004","text":"posted"}}}"#,
        ));
        assert!(from_us(
            r#"{"envelope_id":"e-5","payload":{"event":{"type":"message","subtype":"bot_message",
            "channel":"C0123","bot_id":"B0SELF","ts":"1788000005.000005","text":"⚠️ Check this out."}}}"#,
        ));
        assert!(from_us(
            r#"{"envelope_id":"e-6","payload":{"event":{"type":"message",
            "channel":"C0123","user":"U0BOT","ts":"1788000006.000006","text":"hello"}}}"#,
        ));
        // A mention this instance made of itself is still its own.
        assert!(from_us(
            r#"{"envelope_id":"e-7","payload":{"event":{"type":"app_mention",
            "channel":"C0123","user":"U0BOT","ts":"1788000007.000007","text":"<@U0BOT> hi"}}}"#,
        ));
    }

    /// Another bot's message is not this instance's, and is not read.
    ///
    /// The reader used to take any bot for this one, which was safe while
    /// nothing else ever posted where it listened and is the loop guard
    /// letting every other app through the moment one does. Not read yet
    /// either: what is done about other bots is a decision of its own.
    #[test]
    fn another_bots_message_is_neither_ours_nor_read() {
        for frame in [
            // A post from another app, as measured: a user and a bot
            // identifier, neither of them this instance's.
            r#"{"envelope_id":"e-8","payload":{"event":{"type":"message",
            "channel":"C0123","user":"U0GITHUB","bot_id":"B0OTHER","ts":"1788000008.000008","text":""}}}"#,
            // A classic integration's post.
            r#"{"envelope_id":"e-9","payload":{"event":{"type":"message","subtype":"bot_message",
            "channel":"C0123","bot_id":"B0OTHER","ts":"1788000009.000009","text":"deployed"}}}"#,
            // Another app's follow-up, broadcast from a thread to the room.
            r#"{"envelope_id":"e-10","payload":{"event":{"type":"message","subtype":"thread_broadcast",
            "channel":"C0123","user":"U0GITHUB","bot_id":"B0OTHER","ts":"1788000010.000010",
            "thread_ts":"1788000008.000008","text":""}}}"#,
        ] {
            assert!(
                matches!(said(frame), Incoming::Acknowledge(_)),
                "{frame} is somebody else's, and is not read"
            );
        }
    }

    /// Everything with an envelope is acknowledged, and nothing else is.
    #[test]
    fn exactly_what_carries_an_envelope_is_acknowledged() {
        let reply = said(
            r#"{"envelope_id":"e-11","payload":{"event":{"type":"app_mention",
            "channel":"C0123","user":"U0HUMAN","ts":"1788000011.000011","text":"<@U0BOT> hello"}}}"#,
        );
        assert_eq!(reply.acknowledging(), Some("e-11"));
        assert_eq!(
            Incoming::Acknowledge("e-12".to_owned()).acknowledging(),
            Some("e-12")
        );
        // Nothing to answer, and answering anyway would be answering an
        // envelope that does not exist.
        for nothing in [Incoming::Ready, Incoming::Reconnect, Incoming::Ignore] {
            assert_eq!(nothing.acknowledging(), None, "{nothing:?}");
        }
    }

    /// Everything else is acknowledged and nothing more.
    ///
    /// Acknowledging is the load-bearing half: the platform redelivers what
    /// is not acknowledged, so a frame this chooses to ignore has to be
    /// answered or it arrives for ever.
    #[test]
    fn what_is_not_acted_on_is_still_acknowledged() {
        for frame in [
            // People talking to each other, which is most of a room.
            r#"{"envelope_id":"e-13","payload":{"event":{"type":"message","channel":"C0123",
            "user":"U0HUMAN","ts":"1788000013.000013","text":"lunch?"}}}"#,
            // An edit, which is not somebody talking to a job — including
            // an edit of something this instance said.
            r#"{"envelope_id":"e-14","payload":{"event":{"type":"message","subtype":"message_changed","channel":"C0123"}}}"#,
            r#"{"envelope_id":"e-15","payload":{"event":{"type":"message","subtype":"message_changed",
            "channel":"C0123","bot_id":"B0SELF","ts":"1788000015.000015","text":"edited"}}}"#,
            // Somebody joining.
            r#"{"envelope_id":"e-16","payload":{"event":{"type":"member_joined_channel","channel":"C0123"}}}"#,
            // A mention with nowhere attached to it.
            r#"{"envelope_id":"e-17","payload":{"event":{"type":"app_mention","user":"U0HUMAN","text":"<@U0BOT> hi"}}}"#,
            // An envelope carrying no event at all.
            r#"{"envelope_id":"e-18","payload":{}}"#,
            // Somebody joining, with everything a message would have but
            // being one: it is what is joined, not what is said, and a
            // reader that only checked for a subtype would read it aloud.
            r#"{"envelope_id":"e-19","payload":{"event":{"type":"member_joined_channel",
            "channel":"C0123","user":"U0HUMAN","ts":"1788000019.000019"}}}"#,
        ] {
            assert!(
                matches!(said(frame), Incoming::Acknowledge(_)),
                "{frame} must still be acknowledged"
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

    /// Who this instance is needs a bot token, because only a bot's posts
    /// can be told apart by the identifier they carry.
    #[test]
    fn who_this_instance_is_needs_a_bot_token() {
        assert_eq!(
            identity(200, br#"{"ok":true,"user_id":"U0BOT","bot_id":"B0SELF"}"#).expect("a bot"),
            Identity {
                user: "U0BOT".to_owned(),
                bot: "B0SELF".to_owned(),
            }
        );
        assert!(matches!(
            identity(200, br#"{"ok":true,"user_id":"U0PERSON"}"#),
            Err(ChannelError::NotABot)
        ));
        assert!(matches!(
            identity(200, br#"{"ok":true,"bot_id":"B0SELF"}"#),
            Err(ChannelError::NoAnswer)
        ));
    }

    /// The same refusal guard, on the reader of a question's answer.
    ///
    /// Tested apart from a post's because it is separate code, and the one
    /// that opens a socket fails in a way nobody would attribute to a
    /// missing check: it would connect to a URL that is not there.
    #[test]
    fn a_refusal_is_a_refusal_for_a_questions_answer_too() {
        assert!(matches!(
            accepted(200, br#"{"ok":false,"error":"invalid_auth"}"#),
            Err(ChannelError::Refused(ref why)) if why == "invalid_auth"
        ));
        assert!(matches!(
            accepted(200, br#"{"ok":true is not valid"#),
            Err(ChannelError::Unreadable(_))
        ));
        assert!(matches!(
            accepted(500, br#"{"ok":true}"#),
            Err(ChannelError::Unreachable(_))
        ));
        assert!(accepted(200, br#"{"ok":true}"#).is_ok());
    }

    /// The guard that exists because a refusal arrives as 200.
    ///
    /// The whole reason this module is pure: without it, deleting the
    /// negation from that check turns every bad token and every wrong
    /// channel into a message reported as delivered — and the job then
    /// stops as instructed, having told nobody anything.
    #[test]
    fn a_refusal_with_a_successful_status_is_still_a_refusal() {
        let refused = posted(200, br#"{"ok":false,"error":"not_in_channel"}"#);

        assert!(
            matches!(refused, Err(ChannelError::Refused(ref why)) if why == "not_in_channel"),
            "{refused:?}"
        );
    }

    /// A refusal that names no reason is still a refusal, and an answer that
    /// cannot be read is not a success.
    #[test]
    fn a_refusal_without_a_reason_says_so_rather_than_reading_as_success() {
        let unreadable = posted(200, br#"{"ok":true is not valid"#);
        assert!(
            matches!(unreadable, Err(ChannelError::Unreadable(_))),
            "{unreadable:?}"
        );

        let refused = posted(200, br#"{"ok":false}"#);
        assert!(
            matches!(refused, Err(ChannelError::Refused(ref why)) if why == "no reason given"),
            "{refused:?}"
        );
    }

    /// The identifier comes back exactly as it arrived.
    #[test]
    fn an_accepted_message_answers_with_its_identifier_as_text() {
        assert_eq!(
            posted(200, br#"{"ok":true,"ts":"1728312345.678901"}"#).expect("it was accepted"),
            // Not a float, which is a different string on the way back out
            // and addresses no message.
            "1728312345.678901"
        );
    }

    /// Accepted and unidentified should not read as accepted.
    #[test]
    fn a_message_accepted_without_an_identifier_is_not_a_thread() {
        assert!(
            matches!(
                posted(200, br#"{"ok":true}"#),
                Err(ChannelError::NoIdentifier)
            ),
            "an answer naming no thread is not somewhere to speak"
        );
    }

    /// A status outside the successful range never reaches the body, and
    /// the boundaries of that range are the boundaries.
    #[test]
    fn an_unsuccessful_status_is_unreachable_rather_than_refused() {
        for status in [400, 429, 500, 503] {
            let answer = posted(status, br#"{"ok":true,"ts":"1.1"}"#);
            assert!(
                matches!(answer, Err(ChannelError::Unreachable(_))),
                "{status} should not have been read as an answer: {answer:?}"
            );
        }
        assert!(posted(299, br#"{"ok":true,"ts":"1.1"}"#).is_ok());
        assert!(matches!(
            posted(300, br#"{"ok":true,"ts":"1.1"}"#),
            Err(ChannelError::Unreachable(_))
        ));
    }
}
