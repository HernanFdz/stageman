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

use stageman_core::{Channel, JobId, Secret, Speaking};

use crate::{Call, ChannelError, Identity, Incoming, Message, Reaction, Request};

/// Where Slack takes a message.
const POST_MESSAGE: &str = "https://slack.com/api/chat.postMessage";

/// Where Slack says who a credential is.
const WHO_AM_I: &str = "https://slack.com/api/auth.test";

/// Where Slack says where to connect for the event stream.
const OPEN_SOCKET: &str = "https://slack.com/api/apps.connections.open";

/// Where Slack makes a room.
const CREATE_ROOM: &str = "https://slack.com/api/conversations.create";

/// Where Slack takes a room's purpose.
const SET_PURPOSE: &str = "https://slack.com/api/conversations.setPurpose";

/// Where Slack takes a room's topic.
const SET_TOPIC: &str = "https://slack.com/api/conversations.setTopic";

/// Where Slack takes an invitation into a room.
const INVITE: &str = "https://slack.com/api/conversations.invite";

/// Where Slack archives a room.
const ARCHIVE: &str = "https://slack.com/api/conversations.archive";

/// Where Slack takes a reaction.
const REACT: &str = "https://slack.com/api/reactions.add";

/// The most a room's name may be, in characters.
const NAME_AT_MOST: usize = 80;

/// What separates the parts of a room's name: two hyphens, because titles
/// contain single ones.
const SEPARATOR: &str = "--";

/// The most a room's purpose or topic may be, in characters: the platform
/// refuses anything longer outright, so what is sent is cut to fit.
const DESCRIPTION_AT_MOST: usize = 250;

/// How much of a project's name a room's name carries, at most.
const PROJECT_AT_MOST: usize = 24;

/// How much of a job's identifier a room's name carries: enough that two
/// jobs of one project never collide in practice, short enough to read.
const IDENTIFIER_CHARS: usize = 8;

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

/// A request with a JSON body, made with the credential that speaks: the
/// shape of everything that changes something on the platform.
fn telling(
    url: &str,
    speaking: &Speaking,
    body: serde_json::Map<String, serde_json::Value>,
) -> Request {
    Request {
        method: "POST".to_owned(),
        url: url.to_owned(),
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
        body: Some(serde_json::Value::Object(body).to_string().into_bytes()),
    }
}

/// Renders making a room under a name.
pub fn create_room(speaking: &Speaking, name: &str) -> Request {
    let mut body = serde_json::Map::new();
    body.insert("name".to_owned(), name.into());
    telling(CREATE_ROOM, speaking, body)
}

/// The room made, from the answer to [`create_room`].
pub fn room_created(status: u16, body: &[u8]) -> Result<String, ChannelError> {
    let told = accepted(status, body)?;
    told.get("channel")
        .and_then(|room| room.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(ChannelError::NoAnswer)
}

/// Renders setting a room's purpose, cut to what the platform takes.
pub fn set_purpose(speaking: &Speaking, room: &str, purpose: &str) -> Request {
    let mut body = serde_json::Map::new();
    body.insert("channel".to_owned(), room.into());
    body.insert("purpose".to_owned(), fitting(purpose).into());
    telling(SET_PURPOSE, speaking, body)
}

/// Renders setting a room's topic, cut to what the platform takes.
pub fn set_topic(speaking: &Speaking, room: &str, topic: &str) -> Request {
    let mut body = serde_json::Map::new();
    body.insert("channel".to_owned(), room.into());
    body.insert("topic".to_owned(), fitting(topic).into());
    telling(SET_TOPIC, speaking, body)
}

/// A description cut to what the platform takes, by character rather than
/// by byte, so that a reason ending in a multi-byte character is cut
/// between characters and not inside one.
fn fitting(text: &str) -> String {
    text.chars().take(DESCRIPTION_AT_MOST).collect()
}

/// Renders inviting one person into a room.
pub fn invite(speaking: &Speaking, room: &str, user: &str) -> Request {
    let mut body = serde_json::Map::new();
    body.insert("channel".to_owned(), room.into());
    body.insert("users".to_owned(), user.into());
    telling(INVITE, speaking, body)
}

/// Renders archiving a room.
pub fn archive(speaking: &Speaking, room: &str) -> Request {
    let mut body = serde_json::Map::new();
    body.insert("channel".to_owned(), room.into());
    telling(ARCHIVE, speaking, body)
}

/// Renders putting a reaction on a message.
pub fn react(speaking: &Speaking, room: &str, message: &str, reaction: Reaction) -> Request {
    let mut body = serde_json::Map::new();
    body.insert("channel".to_owned(), room.into());
    body.insert("timestamp".to_owned(), message.into());
    body.insert("name".to_owned(), spelled(reaction).into());
    telling(REACT, speaking, body)
}

/// How Slack spells a reaction, and the inverse.
///
/// Slack's names for its emoji, which is why they live here and nowhere
/// else: what a reaction means is the instance's, and what it is called is
/// the platform's.
const fn spelled(reaction: Reaction) -> &'static str {
    match reaction {
        Reaction::Seen => "eyes",
        Reaction::Done => "white_check_mark",
    }
}

fn reaction_of(spelling: &str) -> Option<Reaction> {
    match spelling {
        "eyes" => Some(Reaction::Seen),
        "white_check_mark" => Some(Reaction::Done),
        _ => None,
    }
}

/// Whether a request that returns nothing was done.
pub fn done(status: u16, body: &[u8]) -> Result<(), ChannelError> {
    accepted(status, body).map(|_| ())
}

/// The name a job's room is given: `<project>--<title>--<identifier>`, in
/// what Slack allows — lowercase letters, digits, hyphens and underscores,
/// eighty characters at most.
///
/// Double hyphens separate the parts because titles contain single ones.
/// A part with nothing left in it is left out rather than left empty, and a
/// name with neither still says what it is. Measured: consecutive hyphens
/// survive creation, and an archived room's name is still taken, which is
/// why the identifier is the one part always present.
pub fn room_name(project: &str, title: &str, job: JobId) -> String {
    let identifier: String = job
        .as_uuid()
        .simple()
        .to_string()
        .chars()
        .take(IDENTIFIER_CHARS)
        .collect();
    let project = slug(project, PROJECT_AT_MOST);
    // What is left for the title once the other parts and their separators
    // are counted, so that the whole is never over the limit. Summed rather
    // than added because the gate is right that an addition can overflow,
    // and these cannot: every term is a small constant or bounded by one.
    let spent: usize = [
        project.len(),
        IDENTIFIER_CHARS,
        SEPARATOR.len(),
        if project.is_empty() {
            0
        } else {
            SEPARATOR.len()
        },
    ]
    .into_iter()
    .sum();
    // The subtrahend is bounded by construction, far below the limit, so
    // the subtraction never clamps; saturating is only the type saying what
    // would happen if it did.
    let title = slug(title, NAME_AT_MOST.saturating_sub(spent)); // CLAMP-OK: bounded, never reached.
    let mut parts: Vec<&str> = [project.as_str(), title.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        parts.push("job");
    }
    parts.push(&identifier);
    parts.join(SEPARATOR)
}

/// Folds text to a piece of a room's name: lowercase, with every run of
/// anything else as one hyphen, no hyphen at either end, and no longer than
/// `at_most`.
fn slug(text: &str, at_most: usize) -> String {
    let mut folded = String::new();
    for character in text.chars() {
        if folded.len() >= at_most {
            break;
        }
        let lowered = character.to_ascii_lowercase();
        if lowered.is_ascii_alphanumeric() {
            folded.push(lowered);
        } else if !folded.is_empty() && !folded.ends_with('-') {
            folded.push('-');
        }
    }
    folded.trim_end_matches('-').to_owned()
}

/// A reference to a room, which the platform renders as its name.
pub fn room_link(room: &str) -> String {
    format!("<#{room}>")
}

/// A mention of somebody, which the platform renders as their name and
/// notifies them of.
pub fn mention(user: &str) -> String {
    format!("<@{user}>")
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
            user: said.user,
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

/// Renders one message posted in a room, as Markdown: at its root, or in a
/// thread there.
///
/// Through the platform's own Markdown parameter, which it translates into
/// its native blocks and writes the notification fallback for — measured,
/// with no special feature enabled, per
/// `docs/decisions/0062-what-this-instance-says-is-markdown.md`. It cannot
/// be combined with the plain text field, so nothing here sends one.
pub fn post(speaking: &Speaking, room: &str, text: &str, thread: Option<&str>) -> Request {
    // Built as a map rather than indexed into as a value: indexing into a
    // value is the operation the gate is right to call a panic, and a map
    // has nothing to fail on. The thread is left out rather than sent as
    // null, because the platform reads a present-but-empty field as an
    // address rather than as no address.
    let mut posting = serde_json::Map::new();
    posting.insert("channel".to_owned(), room.into());
    posting.insert("markdown_text".to_owned(), text.into());
    if let Some(thread) = thread {
        posting.insert("thread_ts".to_owned(), thread.into());
    }
    telling(POST_MESSAGE, speaking, posting)
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
        POST_MESSAGE | CREATE_ROOM | SET_PURPOSE | SET_TOPIC | INVITE | ARCHIVE | REACT => {}
        _ => return None,
    }
    let told: Told = serde_json::from_slice(request.body.as_deref()?).ok()?;
    let channel = Channel::Slack;
    Some(match request.url.as_str() {
        CREATE_ROOM => Call::CreateRoom {
            channel,
            name: told.name?,
        },
        SET_PURPOSE => Call::SetPurpose {
            channel,
            room: told.channel?,
            purpose: told.purpose?,
        },
        SET_TOPIC => Call::SetTopic {
            channel,
            room: told.channel?,
            topic: told.topic?,
        },
        INVITE => Call::Invite {
            channel,
            room: told.channel?,
            user: told.users?,
        },
        ARCHIVE => Call::Archive {
            channel,
            room: told.channel?,
        },
        REACT => Call::React {
            channel,
            room: told.channel?,
            message: told.timestamp?,
            reaction: reaction_of(&told.name?)?,
        },
        _ => Call::Post {
            channel,
            room: told.channel?,
            text: told.markdown_text?,
            thread: told.thread_ts,
        },
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

/// What one request told the platform, read back: every field any of them
/// carries, each present only where its request sends it.
#[derive(serde::Deserialize)]
struct Told {
    channel: Option<String>,
    markdown_text: Option<String>,
    thread_ts: Option<String>,
    name: Option<String>,
    purpose: Option<String>,
    topic: Option<String>,
    users: Option<String>,
    timestamp: Option<String>,
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
    use super::{accepted, decode, fitting, identity, posted, room_name, slug};
    use crate::{ChannelError, Identity, Incoming};
    use stageman_core::{JobId, Uuid};

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

    /// A description longer than the platform takes is cut to fit, by
    /// character, and one that fits is left alone.
    #[test]
    fn a_description_is_cut_to_what_the_platform_takes() {
        assert_eq!(fitting("why"), "why");
        let long: String = "é".repeat(300);
        let cut = fitting(&long);
        assert_eq!(cut.chars().count(), 250);
        assert!(cut.chars().all(|c| c == 'é'), "cut between characters");
    }

    /// A room's name is folded to the platform's alphabet, part by part,
    /// and the whole never exceeds its limit.
    #[test]
    fn a_rooms_name_is_folded_to_what_the_platform_allows() {
        assert_eq!(slug("Closed Loop!", 80), "closed-loop");
        assert_eq!(slug("  --x--  ", 80), "x");
        assert_eq!(
            slug("ÀB", 80),
            "b",
            "what is not the platform's alphabet folds to a hyphen"
        );
        assert_eq!(slug("abcdef", 3), "abc");
        assert_eq!(slug("ab-cdef", 3), "ab", "a cut never ends in a hyphen");

        let job = JobId::from_uuid(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
        let name = room_name(&"long".repeat(20), &"title ".repeat(30), job);
        assert!(name.len() <= 80, "{name}");
        assert!(name.ends_with("--01234567"), "{name}");
        assert!(!name.contains("---"), "{name}");
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
