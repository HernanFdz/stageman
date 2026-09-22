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
//! subscription is read for two things: what this instance said itself,
//! which is how its own posts are told apart, and what another app said,
//! which is read whole and left to the room to decide about; a person's copy
//! there is acknowledged and dropped. See
//! `docs/decisions/0060-a-binding-is-a-workspace.md` and
//! `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
//!
//! **Another app's words are wherever the app put them.** Measured: the
//! GitHub app posts an empty text and everything in an attachment, an app
//! posting blocks puts a notification fallback in the text and the content
//! in the blocks, and a follow-up broadcast under an earlier message carries
//! the app's profile on its root and not on itself. So what an app said is
//! read from the blocks when there are any, else the text, and then from
//! every attachment — in the platform's own markup, untranslated, as a
//! person's mention already arrives.
//!
//! **A thread has no independent existence**, which is why a job's thread
//! is opened by the daemon rather than by the job. There is no call that
//! creates an empty one: a thread is a message plus the replies hanging from
//! it, so somebody posts that message before the job's container starts,
//! and the identifier it comes back with is what a reply names — see
//! `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.

use stageman_core::{Channel, JobId, ProjectId, Secret, Speaking, Uuid};

use crate::{Call, ChannelError, Identity, Incoming, Message, Reaction, Request, ThreadRead};

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

/// Where Slack takes an edit to a message.
const UPDATE_MESSAGE: &str = "https://slack.com/api/chat.update";

/// Where Slack gives a thread back: its parent and its replies.
const REPLIES: &str = "https://slack.com/api/conversations.replies";

/// The most a room's name may be, in characters.
const NAME_AT_MOST: usize = 80;

/// What separates the parts of a room's name: two hyphens, because titles
/// contain single ones.
const SEPARATOR: &str = "--";

/// The most a room's purpose or topic may be, in characters: the platform
/// refuses anything longer outright, so what is sent is cut to fit.
const DESCRIPTION_AT_MOST: usize = 250;

/// The most a Markdown post may be, in characters, per the platform's
/// documentation: a longer one is refused outright, so a text that would
/// pass it is posted in pieces that continue one another — see
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
const POST_AT_MOST: usize = 12_000;

/// Cuts a text into pieces the platform accepts, in order: each as long as
/// a post may be, broken at the last line end in its second half where
/// there is one and at a character otherwise, so that a list or a table is
/// split between lines when it can be.
pub fn pieces(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(cut) = cut_point(rest) {
        // Progress, whatever the cut point says: a loop that could stand
        // still would hang under a fault rather than fail, and a hang is
        // what no test can see.
        if cut == 0 {
            break;
        }
        let (piece, remaining) = rest.split_at(cut);
        pieces.push(piece.to_owned());
        rest = remaining.strip_prefix('\n').unwrap_or(remaining);
    }
    pieces.push(rest.to_owned());
    pieces
}

/// Where a text longer than a post is cut, in bytes: at the last line end
/// in the second half of what a post holds, and at the post's length
/// otherwise. Nothing for a text a post carries whole.
fn cut_point(rest: &str) -> Option<usize> {
    if rest.chars().count() <= POST_AT_MOST {
        return None;
    }
    let limit = rest
        .char_indices()
        .nth(POST_AT_MOST)
        .map_or(rest.len(), |(at, _)| at);
    let cut = rest
        .char_indices()
        .take_while(|(at, _)| *at < limit)
        .filter(|(at, c)| *c == '\n' && *at >= limit / 2)
        .last()
        .map_or(limit, |(at, _)| at);
    Some(cut)
}

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
    named(project, title, job.as_uuid())
}

/// The name a project's foreman's room is given: `<project>--foreman--<identifier>`,
/// the project's identifier making it unique for ever, as a job's does a
/// job's room — see
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
pub fn foreman_room_name(project: &str, id: ProjectId) -> String {
    named(project, "foreman", id.as_uuid())
}

/// A room's name from its three parts, folded to what Slack allows.
fn named(project: &str, title: &str, identifier: &Uuid) -> String {
    let identifier: String = identifier
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
    // Where the workspace is, which every answer to this question carries
    // and a link to a message starts from.
    let url = told
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or(ChannelError::NoAnswer)?;
    Ok(Identity {
        user: user.to_owned(),
        bot: bot.to_owned(),
        url: url.to_owned(),
    })
}

/// The identifier a message is shown to an agent with: the room and the
/// message's own identifier, joined by a slash, which neither contains.
pub fn reference(room: &str, message: &str) -> String {
    format!("{room}/{message}")
}

/// What an identifier an agent named means, if it is one [`reference`]
/// renders: both parts present, and nothing else.
pub fn referenced(reference: &str) -> Option<(String, String)> {
    let (room, message) = reference.trim().split_once('/')?;
    if room.is_empty() || message.is_empty() || message.contains('/') {
        return None;
    }
    Some((room.to_owned(), message.to_owned()))
}

/// Renders asking for a thread: its parent and its replies, the most recent
/// `at_most` of them, oldest first, with a flag saying whether older ones
/// were left out — measured on 2026-09-21 against a real workspace, where a
/// limit of two gave the parent and the two most recent replies. A read
/// rather than a change, so it is asked as one, with its arguments in the
/// address — see `docs/decisions/0068-a-mention-is-shown-its-thread.md`.
pub fn replies(speaking: &Speaking, room: &str, thread: &str, at_most: usize) -> Request {
    Request {
        method: "GET".to_owned(),
        url: format!("{REPLIES}?channel={room}&ts={thread}&limit={at_most}"),
        headers: [(
            "authorization".to_owned(),
            format!("Bearer {}", speaking.credential.expose()),
        )]
        .into(),
        body: None,
    }
}

/// What the platform's answer to [`replies`] means: the thread's messages
/// as decoded, oldest first, which of them mention this instance, and
/// whether older ones were left out. Each is read as a frame's message is,
/// given who this instance is, so that its own words and another app's are
/// told apart the same way; the room is the one asked, since a thread's
/// messages do not name it. A mention is a person's message carrying the
/// markup for this instance: the event that says so when one arrives is not
/// part of a thread read back.
pub fn thread_read(
    status: u16,
    body: &[u8],
    room: &str,
    us: &Identity,
) -> Result<ThreadRead, ChannelError> {
    if !(200..300).contains(&status) {
        return Err(ChannelError::Unreachable(format!(
            "the channel answered {status}"
        )));
    }
    let told: Fetched = serde_json::from_slice(body)
        .map_err(|failure| ChannelError::Unreadable(failure.to_string()))?;
    if !told.ok {
        return Err(ChannelError::Refused(
            told.error.unwrap_or_else(|| "no reason given".to_owned()),
        ));
    }
    let messages: Vec<Message> = told
        .messages
        .into_iter()
        .filter_map(|said| said.message_in(room, us))
        .collect();
    let named = mention(&us.user);
    let mentioning = messages
        .iter()
        .filter(|message| {
            !message.from_us && message.app.is_none() && message.text.contains(&named)
        })
        .map(|message| message.id.clone())
        .collect();
    Ok(ThreadRead {
        messages,
        mentioning,
        longer: told.has_more,
    })
}

/// A thread, as the platform gives one back.
#[derive(serde::Deserialize)]
struct Fetched {
    ok: bool,
    error: Option<String>,
    #[serde(default)]
    messages: Vec<Said>,
    #[serde(default)]
    has_more: bool,
}

/// A link to one message, as Slack spells one — measured on 2026-09-21
/// against a real workspace, through the platform's own permalink call: the
/// workspace's address, the room, and the message's identifier with its
/// dot removed; and for a reply, the thread it is in and the room again.
pub fn permalink(us: &Identity, room: &str, message: &str, thread: Option<&str>) -> String {
    let digits: String = message.chars().filter(|c| *c != '.').collect();
    let base = format!("{}/archives/{room}/p{digits}", us.url.trim_end_matches('/'));
    match thread {
        Some(parent) if parent != message => format!("{base}?thread_ts={parent}&cid={room}"),
        _ => base,
    }
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

    // Recognised by this instance's own identifiers, never by being a bot:
    // a post made with the token carries the bot identifier, and one from
    // the user the token belongs to carries the user's. Either alone is
    // enough, since the two do not always arrive together. Anything else
    // carrying a bot identifier is another app.
    let ours = said.bot_id.as_deref() == Some(us.bot.as_str())
        || said.user.as_deref() == Some(us.user.as_str());
    let another_app = if ours { None } else { said.bot_id.as_deref() };
    let plain = said.subtype.is_none() || said.subtype.as_deref() == Some("bot_message");
    let broadcast = said.subtype.as_deref() == Some("thread_broadcast");
    let (from_us, app) = match said.kind.as_deref() {
        // Somebody mentioning this instance, as its own event. It is what
        // makes a person's message arrive at all, and the one delivery that
        // reaches a room the app was invited into by that very mention. An
        // app that mentions this instance — whether the platform delivers
        // one as this event was not measured — is still an app, so that the
        // answer cannot let one past the rule for apps either way.
        Some("app_mention") => (ours, another_app.map(|bot| said.app_name(bot))),
        // The message subscription is read for what this instance said
        // itself and for what another app said. A person's message here is
        // the copy of a mention that arrives as its own event above, and is
        // dropped. An edit is not a thing said; a broadcast is, when it is
        // an app's follow-up under an earlier message, and nothing of this
        // instance's own.
        Some("message") if ours && plain => (true, None),
        Some("message") if another_app.is_some() && (plain || broadcast) => {
            (false, another_app.map(|bot| said.app_name(bot)))
        }
        _ => return Incoming::Acknowledge(id),
    };

    // An app's words are wherever the app put them; a person's are the
    // text. An app's user is the app's bot user, which is nobody to invite
    // into a room or to name, so it is not carried.
    let (text, user) = if app.is_some() {
        (said.reading(), None)
    } else {
        (said.text, said.user)
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
            text,
            room,
            thread: said.thread_ts,
            user,
            app,
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
    /// What the platform says about the app that posted it, when an app
    /// did.
    bot_profile: Option<Profile>,
    /// A classic bot's name, on a message with the `bot_message` subtype.
    username: Option<String>,
    /// The message a broadcast follows, which carries the profile the
    /// broadcast itself lacks.
    root: Option<Root>,
    #[serde(default)]
    attachments: Vec<Attachment>,
    /// Read as values rather than as types, because a block is a layout
    /// document whose shapes belong to the platform, and only the text in
    /// a few of them is wanted.
    #[serde(default)]
    blocks: Vec<serde_json::Value>,
}

impl Said {
    /// This message as decoded, in the room named, given who this instance
    /// is: what a frame's message becomes, read from a thread instead — by
    /// the same rule, so that the words this instance said are its own and
    /// another app's are the app's, whichever way they arrived. Nothing
    /// without an identifier, since nothing could name it.
    fn message_in(self, room: &str, us: &Identity) -> Option<Message> {
        let ours = self.bot_id.as_deref() == Some(us.bot.as_str())
            || self.user.as_deref() == Some(us.user.as_str());
        let app = if ours {
            None
        } else {
            self.bot_id.as_deref().map(|bot| self.app_name(bot))
        };
        let (text, user) = if app.is_some() {
            (self.reading(), None)
        } else {
            (self.text, self.user)
        };
        Some(Message {
            room: room.to_owned(),
            id: self.ts?,
            thread: self.thread_ts,
            text,
            user,
            from_us: ours,
            app,
        })
    }

    /// What the platform calls the app that posted this: its profile's
    /// name, its root's for a broadcast, a classic bot's username, and the
    /// bot identifier when it has none of those.
    fn app_name(&self, bot: &str) -> String {
        self.bot_profile
            .as_ref()
            .and_then(|profile| profile.name.clone())
            .or_else(|| {
                self.root
                    .as_ref()
                    .and_then(|root| root.bot_profile.as_ref())
                    .and_then(|profile| profile.name.clone())
            })
            .or_else(|| self.username.clone())
            .unwrap_or_else(|| bot.to_owned())
    }

    /// What an app said, as text: the blocks when there are any, else the
    /// text; then every attachment, each read for what it carries.
    fn reading(&self) -> String {
        let mut parts = Vec::new();
        let body = if self.blocks.is_empty() {
            self.text.trim().to_owned()
        } else {
            blocks_text(&self.blocks)
        };
        if !body.is_empty() {
            parts.push(body);
        }
        parts.extend(
            self.attachments
                .iter()
                .map(Attachment::reading)
                .filter(|read| !read.is_empty()),
        );
        parts.join("\n\n")
    }
}

#[derive(serde::Deserialize)]
struct Profile {
    name: Option<String>,
}

#[derive(serde::Deserialize)]
struct Root {
    bot_profile: Option<Profile>,
}

/// One attachment, as much of it as reads as words: what an app built from
/// the older attachment shape says, which is what the GitHub app posts.
#[derive(serde::Deserialize)]
struct Attachment {
    fallback: Option<String>,
    pretext: Option<String>,
    title: Option<String>,
    title_link: Option<String>,
    text: Option<String>,
    #[serde(default)]
    fields: Vec<Field>,
    footer: Option<String>,
}

#[derive(serde::Deserialize)]
struct Field {
    title: Option<String>,
    value: Option<String>,
}

impl Attachment {
    /// Its pretext, title, text, fields and footer, one per line, or its
    /// fallback when it has none of those: the fallback is the platform's
    /// notification line, and is what an app put there to be read where
    /// nothing else can be shown.
    fn reading(&self) -> String {
        let mut lines = Vec::new();
        push_line(&mut lines, self.pretext.as_deref());
        match (self.title.as_deref(), self.title_link.as_deref()) {
            // A title with its link beside it rather than inside it is
            // spelled the way the platform spells a link, so that the two
            // shapes read alike.
            (Some(title), Some(link)) if !title.contains('<') => {
                lines.push(format!("<{link}|{}>", title.trim()));
            }
            _ => push_line(&mut lines, self.title.as_deref()),
        }
        push_line(&mut lines, self.text.as_deref());
        for field in &self.fields {
            match (field.title.as_deref(), field.value.as_deref()) {
                (Some(title), Some(value)) if !title.trim().is_empty() => {
                    lines.push(format!("{}: {}", title.trim(), value.trim()));
                }
                (_, value) => push_line(&mut lines, value),
            }
        }
        push_line(&mut lines, self.footer.as_deref());
        if lines.is_empty() {
            push_line(&mut lines, self.fallback.as_deref());
        }
        lines.join("\n")
    }
}

/// Keeps a line that says something.
fn push_line(lines: &mut Vec<String>, text: Option<&str>) {
    if let Some(text) = text
        && !text.trim().is_empty()
    {
        lines.push(text.trim().to_owned());
    }
}

/// The words in a message's blocks: headings, sections and their fields,
/// context lines, and rich text, one block per line. Anything else — a
/// divider, an image, a button — has no words to read.
fn blocks_text(blocks: &[serde_json::Value]) -> String {
    let mut lines = Vec::new();
    for block in blocks {
        match block.get("type").and_then(serde_json::Value::as_str) {
            Some("header" | "section") => {
                push_line(
                    &mut lines,
                    block
                        .pointer("/text/text")
                        .and_then(serde_json::Value::as_str),
                );
                for field in elements_of(block, "fields") {
                    push_line(
                        &mut lines,
                        field.get("text").and_then(serde_json::Value::as_str),
                    );
                }
            }
            Some("context") => {
                let said: Vec<&str> = elements_of(block, "elements")
                    .filter_map(|element| element.get("text").and_then(serde_json::Value::as_str))
                    .collect();
                push_line(&mut lines, Some(&said.join(" ")));
            }
            Some("rich_text") => {
                for section in elements_of(block, "elements") {
                    push_line(&mut lines, Some(&rich_text(section)));
                }
            }
            _ => {}
        }
    }
    lines.join("\n")
}

/// The values under one of a block's list fields, or nothing.
fn elements_of<'a>(
    block: &'a serde_json::Value,
    field: &str,
) -> impl Iterator<Item = &'a serde_json::Value> {
    block
        .get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
}

/// The words in one rich text section, with the platform's own spelling
/// for what is not plain text: a link, a person, a room, an emoji, a
/// broadcast. A nested section — a list's items, a quote — is read the same
/// way, on lines of its own.
fn rich_text(section: &serde_json::Value) -> String {
    let mut text = String::new();
    for element in elements_of(section, "elements") {
        let field = |name: &str| element.get(name).and_then(serde_json::Value::as_str);
        let piece = match element.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => field("text").unwrap_or("").to_owned(),
            Some("link") => match (field("url"), field("text")) {
                (Some(url), Some(label)) => format!("<{url}|{label}>"),
                (Some(url), None) => format!("<{url}>"),
                (None, _) => String::new(),
            },
            Some("user") => field("user_id").map_or_else(String::new, mention),
            Some("channel") => field("channel_id").map_or_else(String::new, room_link),
            Some("emoji") => field("name").map_or_else(String::new, |name| format!(":{name}:")),
            Some("broadcast") => {
                field("range").map_or_else(String::new, |range| format!("<!{range}>"))
            }
            _ => {
                // A nested section — a list's item — goes on a line of its
                // own; one with nothing to say adds nothing, not a line.
                let nested = rich_text(element);
                if nested.is_empty() {
                    String::new()
                } else if text.is_empty() {
                    nested
                } else {
                    format!("\n{nested}")
                }
            }
        };
        text.push_str(&piece);
    }
    text
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

/// Renders editing one message this instance posted, in place, as
/// Markdown: how a message of the transcript grows, per
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
/// The platform answers as it answers a post, naming the message, so
/// [`posted`] reads the answer.
pub fn update(speaking: &Speaking, room: &str, message: &str, text: &str) -> Request {
    let mut editing = serde_json::Map::new();
    editing.insert("channel".to_owned(), room.into());
    editing.insert("ts".to_owned(), message.into());
    editing.insert("markdown_text".to_owned(), text.into());
    telling(UPDATE_MESSAGE, speaking, editing)
}

/// What a request asks, if it is one this module rendered.
pub fn call(request: &Request) -> Option<Call> {
    if request.method == "GET" {
        return read_call(request);
    }
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
        POST_MESSAGE | UPDATE_MESSAGE | CREATE_ROOM | SET_PURPOSE | SET_TOPIC | INVITE
        | ARCHIVE | REACT => {}
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
        UPDATE_MESSAGE => Call::Update {
            channel,
            room: told.channel?,
            message: told.ts?,
            text: told.markdown_text?,
        },
        _ => Call::Post {
            channel,
            room: told.channel?,
            text: told.markdown_text?,
            thread: told.thread_ts,
        },
    })
}

/// What a read asks, if it is one this module renders: a thread, by the
/// arguments in its address.
fn read_call(request: &Request) -> Option<Call> {
    let (url, query) = request.url.split_once('?')?;
    if url != REPLIES {
        return None;
    }
    let mut room = None;
    let mut thread = None;
    let mut at_most = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("channel", value)) => room = Some(value.to_owned()),
            Some(("ts", value)) => thread = Some(value.to_owned()),
            Some(("limit", value)) => at_most = value.parse().ok(),
            _ => {}
        }
    }
    Some(Call::Replies {
        channel: Channel::Slack,
        room: room?,
        thread: thread?,
        at_most: at_most?,
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
    /// The message an edit names.
    ts: Option<String>,
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
            url: "https://example.slack.com/".to_owned(),
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

    /// Another app's message is read as that app's and never as this
    /// instance's, whatever shape it arrives in, and its name is wherever
    /// the platform put it: the bot identifier when there is no profile, a
    /// classic bot's username, and a broadcast's root's profile.
    ///
    /// The reader used to take any bot for this one, which was safe while
    /// nothing else ever posted where it listened and is the loop guard
    /// letting every other app through the moment one does. What is done
    /// about another app's message is the room's to decide, in the domain.
    #[test]
    fn another_apps_message_is_read_as_the_apps_and_never_as_ours() {
        let named = |frame: &str| match said(frame) {
            Incoming::Said { message, .. } => {
                assert!(!message.from_us, "{frame}");
                assert_eq!(message.user, None, "an app's bot user is nobody to name");
                message.app.expect("an app's message names the app")
            }
            other => panic!("expected an app's message: {other:?}"),
        };

        // A post from another app with no profile: a user and a bot
        // identifier, neither of them this instance's.
        assert_eq!(
            named(
                r#"{"envelope_id":"e-8","payload":{"event":{"type":"message",
                "channel":"C0123","user":"U0GITHUB","bot_id":"B0OTHER","ts":"1788000008.000008","text":"filed"}}}"#,
            ),
            "B0OTHER"
        );
        // A classic integration's post, named by its username.
        assert_eq!(
            named(
                r#"{"envelope_id":"e-9","payload":{"event":{"type":"message","subtype":"bot_message",
                "channel":"C0123","bot_id":"B0OTHER","username":"deploybot","ts":"1788000009.000009","text":"deployed"}}}"#,
            ),
            "deploybot"
        );
        // Another app's follow-up, broadcast from a thread to the room,
        // named by its root.
        assert_eq!(
            named(
                r#"{"envelope_id":"e-10","payload":{"event":{"type":"message","subtype":"thread_broadcast",
                "channel":"C0123","user":"U0GITHUB","bot_id":"B0OTHER","ts":"1788000010.000010",
                "thread_ts":"1788000008.000008","text":"","root":{"bot_profile":{"name":"GitHub"}},
                "attachments":[{"fallback":"closed"}]}}}"#,
            ),
            "GitHub"
        );
    }

    /// The GitHub app's message that an issue was opened, as captured from
    /// a real workspace on 2026-09-15 with the icons and buttons taken out:
    /// an empty text, and everything in one attachment. Read as the app's,
    /// with its words in the order the card shows them and in the
    /// platform's own markup.
    #[test]
    fn an_issue_opened_by_the_github_app_is_read_from_its_attachment() {
        let heard = said(
            r#"{"envelope_id":"ccfe6354-a5b5-467a-bcc0-b9aea79083f8","payload":{"team_id":"T0BTGA3HFRB","context_team_id":"T0BTGA3HFRB","context_enterprise_id":null,"api_app_id":"A0BT53849TR","event":{"type":"message","user":"U0C1X0K98DQ","ts":"1789480508.740559","bot_id":"B0C1V4509PG","text":"","bot_profile":{"id":"B0C1V4509PG","name":"GitHub"},"attachments":[{"fallback":"[HernanFdz/stageman-test] Issue opened by HernanFdz","text":"Opened to capture the frames the GitHub Slack app produces for an issue: opened, commented, closed and reopened. Safe to close.","pretext":"Issue created by <https://github.com/HernanFdz|HernanFdz>","title":"<https://github.com/HernanFdz/stageman-test/issues/9|#9 Probe: hearing the GitHub app in a watched room>","footer":"<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>"}],"metadata":{"event_type":"github_notification"},"channel":"C0BT53FM079","event_ts":"1789480508.740559","channel_type":"group"},"type":"event_callback","is_ext_shared_channel":false},"type":"events_api","accepts_response_payload":false,"retry_attempt":0,"retry_reason":""}"#,
        );
        let Incoming::Said { envelope, message } = heard else {
            panic!("an app's message: {heard:?}");
        };
        assert_eq!(envelope, "ccfe6354-a5b5-467a-bcc0-b9aea79083f8");
        assert_eq!(message.room, "C0BT53FM079");
        assert_eq!(message.id, "1789480508.740559");
        assert_eq!(message.thread, None);
        assert_eq!(message.app.as_deref(), Some("GitHub"));
        assert_eq!(message.user, None);
        assert!(!message.from_us);
        assert_eq!(
            message.text,
            "Issue created by <https://github.com/HernanFdz|HernanFdz>
<https://github.com/HernanFdz/stageman-test/issues/9|#9 Probe: hearing the GitHub app in a watched room>
Opened to capture the frames the GitHub Slack app produces for an issue: opened, commented, closed and reopened. Safe to close.
<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>"
        );
    }

    /// The same app's follow-up that the issue was closed, as captured: a
    /// broadcast under the original, carrying the original's identifier as
    /// its thread, no profile of its own and its root's, and an attachment
    /// with no text. Read as the app's, in that thread.
    #[test]
    fn a_follow_up_from_the_github_app_is_read_in_the_thread_it_follows() {
        let heard = said(
            r#"{"envelope_id":"d3c6fa82-c577-4748-9325-0e2009d1929e","payload":{"team_id":"T0BTGA3HFRB","context_team_id":"T0BTGA3HFRB","context_enterprise_id":null,"api_app_id":"A0BT53849TR","event":{"type":"message","subtype":"thread_broadcast","bot_id":"B0C1V4509PG","thread_ts":"1789480508.740559","root":{"user":"U0C1X0K98DQ","type":"message","ts":"1789480508.740559","bot_id":"B0C1V4509PG","text":"","bot_profile":{"id":"B0C1V4509PG","name":"GitHub"},"thread_ts":"1789480508.740559","attachments":[{"fallback":"[HernanFdz/stageman-test] Issue opened by HernanFdz","text":"Opened to capture the frames the GitHub Slack app produces for an issue: opened, commented, closed and reopened. Safe to close.","pretext":"Issue created by <https://github.com/HernanFdz|HernanFdz>","title":"<https://github.com/HernanFdz/stageman-test/issues/9|#9 Probe: hearing the GitHub app in a watched room>","footer":"<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>"}],"metadata":{"event_type":"github_notification"}},"user":"U0C1X0K98DQ","ts":"1789480543.379269","text":"","attachments":[{"fallback":"[HernanFdz/stageman-test] Issue closed as completed by HernanFdz","pretext":"Issue closed as completed by <https://github.com/HernanFdz|HernanFdz>","title":"<https://github.com/HernanFdz/stageman-test/issues/9|#9 Probe: hearing the GitHub app in a watched room>","footer":"<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>"}],"metadata":{"event_type":"github_notification"},"channel":"C0BT53FM079","event_ts":"1789480543.379269","channel_type":"group"},"type":"event_callback","is_ext_shared_channel":false},"type":"events_api","accepts_response_payload":false,"retry_attempt":0,"retry_reason":""}"#,
        );
        let Incoming::Said { message, .. } = heard else {
            panic!("an app's message: {heard:?}");
        };
        assert_eq!(message.id, "1789480543.379269");
        assert_eq!(message.thread.as_deref(), Some("1789480508.740559"));
        assert_eq!(message.app.as_deref(), Some("GitHub"));
        assert_eq!(
            message.text,
            "Issue closed as completed by <https://github.com/HernanFdz|HernanFdz>
<https://github.com/HernanFdz/stageman-test/issues/9|#9 Probe: hearing the GitHub app in a watched room>
<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>"
        );

        // A pull request arrives in the same shape, and reads the same way.
        let Incoming::Said { message, .. } = said(
            r#"{"envelope_id":"0e18e2dc-1068-4d89-8397-e5a3b711a45b","payload":{"team_id":"T0BTGA3HFRB","context_team_id":"T0BTGA3HFRB","context_enterprise_id":null,"api_app_id":"A0BT53849TR","event":{"type":"message","user":"U0C1X0K98DQ","ts":"1789480575.595269","bot_id":"B0C1V4509PG","text":"","bot_profile":{"id":"B0C1V4509PG","name":"GitHub"},"attachments":[{"fallback":"[HernanFdz/stageman-test] Pull request opened by HernanFdz","text":"Opened to capture the frame the GitHub Slack app produces for a pull request. Safe to close.","pretext":"Pull request opened by <https://github.com/HernanFdz|HernanFdz>","title":"<https://github.com/HernanFdz/stageman-test/pull/10|#10 Probe: a pull request notification>","footer":"<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>"}],"metadata":{"event_type":"github_notification"},"channel":"C0BT53FM079","event_ts":"1789480575.595269","channel_type":"group"},"type":"event_callback","is_ext_shared_channel":false},"type":"events_api","accepts_response_payload":false,"retry_attempt":0,"retry_reason":""}"#,
        ) else {
            panic!("an app's message");
        };
        assert_eq!(message.app.as_deref(), Some("GitHub"));
        assert!(
            message.text.starts_with("Pull request opened by"),
            "{}",
            message.text
        );
    }

    /// The edits that arrive beside every follow-up — the original's card
    /// and the broadcast's, recoloured — are edits, and are dropped as edits
    /// always were.
    #[test]
    fn the_edits_beside_an_apps_follow_up_are_dropped() {
        assert!(matches!(
            said(
                r#"{"envelope_id":"8cd2e7a2-7b28-4426-9d2a-efc2df1765d1","payload":{"team_id":"T0BTGA3HFRB","context_team_id":"T0BTGA3HFRB","context_enterprise_id":null,"api_app_id":"A0BT53849TR","event":{"type":"message","subtype":"message_changed","message":{"user":"U0C1X0K98DQ","type":"message","bot_id":"B0C1V4509PG","text":"","bot_profile":{"id":"B0C1V4509PG","name":"GitHub"},"thread_ts":"1789480508.740559","attachments":[{"fallback":"[HernanFdz/stageman-test] Issue opened by HernanFdz","text":"Opened to capture the frames the GitHub Slack app produces for an issue: opened, commented, closed and reopened. Safe to close.","pretext":"Issue created by <https://github.com/HernanFdz|HernanFdz>","title":"<https://github.com/HernanFdz/stageman-test/issues/9|#9 Probe: hearing the GitHub app in a watched room>","footer":"<https://github.com/HernanFdz/stageman-test|HernanFdz/stageman-test>","fields":[{"value":"1","title":"Comments","short":true}]}],"metadata":{"event_type":"github_notification"},"ts":"1789480508.740559","source_team":"T0BTGA3HFRB","user_team":"T0BTGA3HFRB"},"channel":"C0BT53FM079","hidden":true,"ts":"1789480543.002100","event_ts":"1789480543.002100","channel_type":"group"},"type":"event_callback","is_ext_shared_channel":false},"type":"events_api","accepts_response_payload":false,"retry_attempt":0,"retry_reason":""}"#
            ),
            Incoming::Acknowledge(_)
        ));
    }

    /// A message built from blocks is read from the blocks — the heading,
    /// the section and its fields, the context line — and not from its
    /// text, which is the notification fallback the app put there.
    /// Captured from a real workspace on 2026-09-15 by posting one, and
    /// carried here as another app's.
    #[test]
    fn a_message_built_from_blocks_is_read_from_its_blocks() {
        let Incoming::Said { message, .. } = said(
            r#"{"envelope_id":"f2b63490-9196-4eb0-9127-fa873942e58b","payload":{"team_id":"T0BTGA3HFRB","context_team_id":"T0BTGA3HFRB","context_enterprise_id":null,"api_app_id":"A0BT53849TR","event":{"type":"message","user":"U0OTHER","ts":"1789480774.072789","bot_id":"B0OTHER","text":"fallback text for a block message","bot_profile":{"id":"B0OTHER","name":"Alerts"},"blocks":[{"type":"header","text":{"type":"plain_text","text":"Probe: a block message","emoji":true}},{"type":"section","text":{"type":"mrkdwn","text":"A *section* with a <https://example.com|link> and a mention of <@U0BUEQZHMB2>.","verbatim":false},"fields":[{"type":"mrkdwn","text":"*Severity*\nerror","verbatim":false},{"type":"mrkdwn","text":"*Service*\napi","verbatim":false}]},{"type":"context","elements":[{"type":"mrkdwn","text":"context line, _safe to delete_","verbatim":false}]},{"type":"divider"}],"channel":"C0BT53FM079","event_ts":"1789480774.072789","channel_type":"group"},"type":"event_callback","is_ext_shared_channel":false},"type":"events_api","accepts_response_payload":false,"retry_attempt":0,"retry_reason":""}"#,
        ) else {
            panic!("an app's message");
        };
        assert_eq!(message.app.as_deref(), Some("Alerts"));
        assert_eq!(
            message.text,
            "Probe: a block message
A *section* with a <https://example.com|link> and a mention of <@U0BUEQZHMB2>.
*Severity*
error
*Service*
api
context line, _safe to delete_"
        );
    }

    /// Rich text is read with the platform's own spelling for what is not
    /// plain text, and a nested section lands on a line of its own.
    #[test]
    fn rich_text_is_read_with_the_platforms_spelling_for_references() {
        let frame = r#"{"envelope_id":"e-20","payload":{"event":{"type":"message","channel":"C0123",
            "user":"U0OTHER","bot_id":"B0OTHER","ts":"1788000020.000020","text":"fallback",
            "blocks":[{"type":"rich_text","elements":[
                {"type":"rich_text_section","elements":[
                    {"type":"text","text":"see "},{"type":"link","url":"https://example.com","text":"this"},
                    {"type":"text","text":" and "},{"type":"link","url":"https://example.org"},
                    {"type":"text","text":" "},{"type":"user","user_id":"U0HUMAN"},{"type":"text","text":" in "},
                    {"type":"channel","channel_id":"C0999"},{"type":"text","text":" "},{"type":"emoji","name":"eyes"},
                    {"type":"text","text":" "},{"type":"broadcast","range":"here"}]},
                {"type":"rich_text_list","style":"bullet","elements":[
                    {"type":"rich_text_section","elements":[{"type":"text","text":"one"}]},
                    {"type":"rich_text_section","elements":[{"type":"text","text":"two"}]}]}]},
              {"type":"divider"}]}}}"#;
        let Incoming::Said { message, .. } = said(frame) else {
            panic!("an app's message");
        };
        assert_eq!(
            message.text,
            "see <https://example.com|this> and <https://example.org> <@U0HUMAN> in <#C0999> :eyes: <!here>
one
two"
        );
    }

    /// An attachment with nothing but a fallback is read as that, a title
    /// with its link beside it is spelled as the platform spells a link, a
    /// title that is already a link is left as it is, and fields read as
    /// name and value — or as the value alone when the name is blank.
    #[test]
    fn an_attachment_is_read_for_what_it_carries() {
        let frame = r#"{"envelope_id":"e-21","payload":{"event":{"type":"message","channel":"C0123",
            "user":"U0OTHER","bot_id":"B0OTHER","ts":"1788000021.000021","text":"",
            "attachments":[
                {"fallback":"only a fallback"},
                {"title":"Alert fired","title_link":"https://example.com/alert/1","fields":[
                    {"title":"Severity","value":"error"},{"value":"unnamed"},
                    {"title":"  ","value":"blank name"}],"footer":"alerts"},
                {"title":"<https://example.com/alert/2|Already a link>","title_link":"https://example.com/alert/2"}]}}}"#;
        let Incoming::Said { message, .. } = said(frame) else {
            panic!("an app's message");
        };
        assert_eq!(
            message.text,
            "only a fallback

<https://example.com/alert/1|Alert fired>
Severity: error
unnamed
blank name
alerts

<https://example.com/alert/2|Already a link>"
        );
    }

    /// An element with nothing to say — a colour swatch, an unknown kind —
    /// adds nothing between the words around it, and a list item that
    /// comes first opens no blank line.
    #[test]
    fn an_element_with_nothing_to_say_adds_nothing() {
        let frame = r#"{"envelope_id":"e-22","payload":{"event":{"type":"message","channel":"C0123",
            "user":"U0OTHER","bot_id":"B0OTHER","ts":"1788000022.000022","text":"fallback",
            "blocks":[{"type":"rich_text","elements":[
                {"type":"rich_text_section","elements":[
                    {"type":"text","text":"a"},{"type":"color","value":"ff0000"},{"type":"text","text":"b"}]},
                {"type":"rich_text_list","style":"bullet","elements":[
                    {"type":"rich_text_section","elements":[]},
                    {"type":"rich_text_section","elements":[{"type":"text","text":"one"}]}]}]}]}}}"#;
        let Incoming::Said { message, .. } = said(frame) else {
            panic!("an app's message");
        };
        assert_eq!(message.text, "ab\none");
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
            identity(200, br#"{"ok":true,"user_id":"U0BOT","bot_id":"B0SELF","url":"https://example.slack.com/"}"#).expect("a bot"),
            Identity {
                user: "U0BOT".to_owned(),
                bot: "B0SELF".to_owned(), url: "https://example.slack.com/".to_owned(),
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
