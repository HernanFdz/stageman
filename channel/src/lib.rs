//! The contract every channel is spoken on, and the adapters that implement
//! it.
//!
//! One contract: a message posted at the root of a room or in a thread
//! there, and an event stream opened — who this instance is on the channel,
//! where to connect — with its frames read and acknowledged. Since
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
//! none of it is performed here. What is said to a platform is rendered as a
//! request the world makes, and what the platform answers is read back from
//! what the world carried, both as pure functions the instance calls.
//! Nothing here opens a socket or names an async runtime.
//!
//! **Nothing outside an adapter may be specific to one channel**, which is
//! the rule the agent crate keeps for agents and the reason this crate is
//! beside it rather than inside the foreman's or the instance's — see
//! `docs/decisions/0058-a-channels-adapter-is-a-crate-beside-the-agents.md`.
//! The functions at this level dispatch on the channel and name no platform;
//! the module under each channel does.

mod slack;

use std::collections::{BTreeMap, BTreeSet};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use stageman_core::{Channel, JobId, ProjectId, Secret, Speaking};

/// What a query string carries as it is: the unreserved characters, and
/// nothing else. Everything else is percent-encoded, spaces and line ends
/// included, so a whole manifest survives an address bar.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Who this instance is on a channel, so it can recognise itself.
///
/// Two identifiers, asked for once per connection. The user is what a
/// mention names and what a message from the token's own user carries; the
/// bot is what every post made with the token carries. Together they answer
/// the one question the routing rule asks about the speaker — whether this
/// instance is what said it — by its own identifiers rather than by its
/// being a bot, which stopped being the same thing the moment another bot
/// could be heard.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    /// What a mention of this instance looks up to.
    pub user: String,
    /// What a post made with this instance's credential carries.
    pub bot: String,
    /// Where the workspace is, as an address a person can open: what a link
    /// to a message starts from.
    pub url: String,
}

/// One message heard on a channel, as decoded.
///
/// What routing needs and nothing else: the room it was said in, what
/// identifies it, the thread it was in if any, the words, whether this
/// instance said it, and which app did if one did. That a person's message
/// mentions this instance is not carried, because since
/// `docs/decisions/0060-a-binding-is-a-workspace.md` it is what made the
/// message arrive: a person is read from the platform's own mention event,
/// and nothing a person says without one is decoded from a frame at all. A
/// thread read back is the one place a person's words without a mention are
/// decoded, per `docs/decisions/0068-a-mention-is-shown-its-thread.md`, and
/// which of its messages mention this instance is said beside them, in
/// [`ThreadRead`], rather than on each. Another app's
/// message is decoded whole — its attachments and blocks read into the
/// words — and whether it is read at all is the room's to decide, in the
/// domain's routing rule, asked by the instance. Which job or foreman it is
/// for is not decided here either.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    /// The room it was said in, as the platform names it.
    pub room: String,
    /// What identifies this message, which is the thread a foreman answers
    /// in when the message is at the root.
    pub id: String,
    /// The thread it was in, if it was in one.
    pub thread: Option<String>,
    /// What was said, as the person wrote it.
    pub text: String,
    /// Who said it, as the platform names them, when the platform said.
    pub user: Option<String>,
    /// Whether this instance is what said it.
    pub from_us: bool,
    /// The app that posted it, by the name the platform gives the app, when
    /// another app did. None for a person's message and for this instance's
    /// own. What a signal is framed with, and what says that the room rather
    /// than a mention decides whether it is read — see
    /// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
    pub app: Option<String>,
}

/// What one frame from a channel's event stream means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// The connection is up and nothing needs doing.
    Ready,
    /// The platform is about to close this connection and expects another.
    ///
    /// Not a failure. It says so on a schedule of its own, and a listener
    /// that treated it as one would reconnect just as often while logging
    /// alarm.
    Reconnect,
    /// Somebody said something, and the envelope it arrived in.
    Said {
        /// What has to be acknowledged, whatever is decided about the
        /// message.
        envelope: String,
        /// What was said.
        message: Message,
    },
    /// Something that needs acknowledging and nothing else.
    ///
    /// **Acknowledging is not optional and not politeness.** The platform
    /// redelivers whatever goes unacknowledged, so a frame dropped silently
    /// is the same message arriving again rather than a message lost — and
    /// a message this instance chose not to act on would arrive for ever.
    Acknowledge(String),
    /// A frame this does not understand, carrying nothing to acknowledge.
    Ignore,
}

impl Incoming {
    /// The envelope this frame has to be answered with, if it has one.
    ///
    /// **The most consequential small function here.** The platform
    /// redelivers what goes unacknowledged, so answering nothing means every
    /// message arrives for ever — and answering the wrong envelope means the
    /// right one never stops. Neither shows up as a failure anywhere.
    #[must_use]
    pub fn acknowledging(&self) -> Option<&str> {
        match self {
            Self::Said { envelope, .. } | Self::Acknowledge(envelope) => Some(envelope),
            Self::Ready | Self::Reconnect | Self::Ignore => None,
        }
    }
}

/// One request to a platform, as the instance asks the world to make it.
///
/// Plain data, credential included: what crosses to the world is what the
/// world sends, and a scenario's trace carries it in full — every credential
/// in a test being fake — which is why nothing here formats. See
/// `docs/conventions.md` §4.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    /// The method, as the protocol spells it.
    pub method: String,
    /// Where, scheme and all.
    pub url: String,
    /// The headers, by name.
    pub headers: BTreeMap<String, String>,
    /// The body, if it has one.
    pub body: Option<Vec<u8>>,
}

/// A reaction this instance puts on somebody's message: what it means,
/// with the spelling left to the channel.
///
/// The acknowledgement a foreman gives, since
/// `docs/decisions/0062-what-this-instance-says-is-markdown.md`: a reaction
/// says "received" and "done" without a line in the room's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reaction {
    /// The message has been received: it is being worked on, or waits its
    /// turn.
    Seen,
    /// The message has been handled.
    Done,
}

/// Renders posting one message in a room on a channel, as Markdown: at the
/// room's root when `thread` is none, and in that thread otherwise.
#[must_use]
pub fn post(
    channel: Channel,
    speaking: &Speaking,
    room: &str,
    text: &str,
    thread: Option<&str>,
) -> Request {
    match channel {
        Channel::Slack => slack::post(speaking, room, text, thread),
    }
}

/// Renders editing one message this instance posted, in place: how a
/// message of the transcript grows. Answered as a post is, so [`posted`]
/// reads the answer.
#[must_use]
pub fn update(
    channel: Channel,
    speaking: &Speaking,
    room: &str,
    message: &str,
    text: &str,
) -> Request {
    match channel {
        Channel::Slack => slack::update(speaking, room, message, text),
    }
}

/// Renders asking a channel for a thread: its parent and its most recent
/// replies, up to `at_most`, with the credential that speaks.
#[must_use]
pub fn replies(
    channel: Channel,
    speaking: &Speaking,
    room: &str,
    thread: &str,
    at_most: usize,
) -> Request {
    match channel {
        Channel::Slack => slack::replies(speaking, room, thread, at_most),
    }
}

/// A thread as a channel gave it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRead {
    /// Its messages, oldest first, the parent included.
    pub messages: Vec<Message>,
    /// Which of them mention this instance, by identifier: a person's, read
    /// off the text the way the platform spells a mention, since a thread
    /// read back carries no event saying so.
    pub mentioning: BTreeSet<String>,
    /// Whether older replies were left out.
    pub longer: bool,
}

/// What the platform's answer to [`replies`] means.
///
/// The thread's messages, oldest first, decoded as a frame's are given who
/// this instance is; which of them mention it; and whether older ones were
/// left out.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, or if
/// the channel refused.
pub fn thread_read(
    channel: Channel,
    status: u16,
    body: &[u8],
    room: &str,
    us: &Identity,
) -> Result<ThreadRead, ChannelError> {
    match channel {
        Channel::Slack => slack::thread_read(status, body, room, us),
    }
}

/// Cuts a text into the pieces a channel will accept as posts, in order,
/// which for a text short enough is the one piece it already is.
#[must_use]
pub fn pieces(channel: Channel, text: &str) -> Vec<String> {
    match channel {
        Channel::Slack => slack::pieces(text),
    }
}

/// What the platform's answer to a post means: the identifier of what was
/// posted, as text, or why nothing was.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, if the
/// channel refused the message, or if it accepted one and named no
/// identifier.
pub fn posted(channel: Channel, status: u16, body: &[u8]) -> Result<String, ChannelError> {
    match channel {
        Channel::Slack => slack::posted(status, body),
    }
}

/// Renders creating a room on a channel, under a name the channel allows.
///
/// The room a job's conversation happens in — see
/// `docs/decisions/0061-a-job-has-a-room-of-its-own.md`. Made with the
/// credential that speaks, which is the one that then posts there.
#[must_use]
pub fn create_room(channel: Channel, speaking: &Speaking, name: &str) -> Request {
    match channel {
        Channel::Slack => slack::create_room(speaking, name),
    }
}

/// What the platform's answer to [`create_room`] means: the identifier of
/// the room made, as text.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, if
/// the channel refused — a name already taken is the ordinary refusal — or
/// if it accepted and named no room.
pub fn room_created(channel: Channel, status: u16, body: &[u8]) -> Result<String, ChannelError> {
    match channel {
        Channel::Slack => slack::room_created(status, body),
    }
}

/// Renders setting what a room is for, as the channel shows it beside the
/// room's name.
#[must_use]
pub fn set_purpose(channel: Channel, speaking: &Speaking, room: &str, purpose: &str) -> Request {
    match channel {
        Channel::Slack => slack::set_purpose(speaking, room, purpose),
    }
}

/// Renders setting a room's topic, as the channel shows it at the top of
/// the room.
#[must_use]
pub fn set_topic(channel: Channel, speaking: &Speaking, room: &str, topic: &str) -> Request {
    match channel {
        Channel::Slack => slack::set_topic(speaking, room, topic),
    }
}

/// Renders inviting one person into a room.
#[must_use]
pub fn invite(channel: Channel, speaking: &Speaking, room: &str, user: &str) -> Request {
    match channel {
        Channel::Slack => slack::invite(speaking, room, user),
    }
}

/// Renders putting a reaction on a message in a room.
#[must_use]
pub fn react(
    channel: Channel,
    speaking: &Speaking,
    room: &str,
    message: &str,
    reaction: Reaction,
) -> Request {
    match channel {
        Channel::Slack => slack::react(speaking, room, message, reaction),
    }
}

/// Renders archiving a room: it leaves the sidebar, stays readable, and
/// takes no more posts.
#[must_use]
pub fn archive(channel: Channel, speaking: &Speaking, room: &str) -> Request {
    match channel {
        Channel::Slack => slack::archive(speaking, room),
    }
}

/// What the platform's answer to a request that returns nothing means:
/// that it was done, or why not.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, or if
/// the channel refused.
pub fn done(channel: Channel, status: u16, body: &[u8]) -> Result<(), ChannelError> {
    match channel {
        Channel::Slack => slack::done(status, body),
    }
}

/// The name a job's room is given on a channel: the project's name, folded
/// to what the channel allows, and the job's name whole — see
/// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
///
/// Only the job's name is load-bearing. An archived room keeps its name for
/// ever, so the name has to be unique for ever too, and the job's name is
/// what makes it so; the project's part is for a sidebar.
#[must_use]
pub fn room_name(channel: Channel, project: &str, job: &JobId) -> String {
    match channel {
        Channel::Slack => slack::room_name(project, job),
    }
}

/// The name a project's foreman's room is given on a channel.
///
/// The project, the word foreman, and the project's identifier's prefix,
/// folded to what the channel allows. Only the identifier is load-bearing,
/// for the reason [`room_name`] gives.
#[must_use]
pub fn foreman_room_name(channel: Channel, project: &str, id: ProjectId) -> String {
    match channel {
        Channel::Slack => slack::foreman_room_name(project, id),
    }
}

/// A reference to a room, as the channel renders one inside a message.
#[must_use]
pub fn room_link(channel: Channel, room: &str) -> String {
    match channel {
        Channel::Slack => slack::room_link(room),
    }
}

/// The identifier a message is shown to an agent with, and the one it names
/// a message by.
///
/// The room and the message as one, so that a reply can go into a room its
/// speaker does not own and a thread shown in an earlier turn can be named
/// in a later one — see
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
#[must_use]
pub fn reference(channel: Channel, room: &str, message: &str) -> String {
    match channel {
        Channel::Slack => slack::reference(room, message),
    }
}

/// What an identifier an agent named means: the room and the message, if
/// it is one this crate renders.
#[must_use]
pub fn referenced(channel: Channel, reference: &str) -> Option<(String, String)> {
    match channel {
        Channel::Slack => slack::referenced(reference),
    }
}

/// A link to one message in a room, as a person can open it, from where
/// the channel said its workspace is: the message's own, or its place in a
/// thread when it is a reply.
#[must_use]
pub fn permalink(
    channel: Channel,
    us: &Identity,
    room: &str,
    message: &str,
    thread: Option<&str>,
) -> String {
    match channel {
        Channel::Slack => slack::permalink(us, room, message, thread),
    }
}

/// A link to a room, as a person can open it, from where the channel said
/// its workspace is — what a page links a room by, per
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
#[must_use]
pub fn room_address(channel: Channel, us: &Identity, room: &str) -> String {
    match channel {
        Channel::Slack => slack::room_address(us, room),
    }
}

/// A mention of somebody, as the channel renders one inside a message.
#[must_use]
pub fn mention(channel: Channel, user: &str) -> String {
    match channel {
        Channel::Slack => slack::mention(user),
    }
}

/// The manifest a project's app on a channel is created from, as tracked
/// text: the scopes the adapter's calls need and the events its listener
/// reads.
#[must_use]
pub const fn manifest(channel: Channel) -> &'static str {
    match channel {
        Channel::Slack => slack::MANIFEST,
    }
}

/// Where the channel's own form for a new app is, with the manifest filled
/// in.
///
/// The guide a page offers beside the boxes that take the app's
/// credentials, per
/// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
///
/// `instance` is this instance's own address, scheme and all, which the
/// redirect address the manifest carries hangs off — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
#[must_use]
pub fn app_form(channel: Channel, instance: &str) -> String {
    match channel {
        Channel::Slack => slack::app_form(instance),
    }
}

/// Renders asking a channel who this instance is on it.
///
/// Once per connection rather than once per message, and with the
/// credential that speaks rather than the one that opens the event stream,
/// because the speaking one is what a mention names and what its own posts
/// carry.
#[must_use]
pub fn who_am_i(channel: Channel, speaking: &Speaking) -> Request {
    match channel {
        Channel::Slack => slack::who_am_i(speaking),
    }
}

/// What the platform's answer to [`who_am_i`] means.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, if
/// the channel refused, or if it accepted and named nobody.
pub fn identity(channel: Channel, status: u16, body: &[u8]) -> Result<Identity, ChannelError> {
    match channel {
        Channel::Slack => slack::identity(status, body),
    }
}

/// Renders asking a channel where to connect for its event stream, with the
/// credential that opens one.
#[must_use]
pub fn open_socket(channel: Channel, opening: &Secret) -> Request {
    match channel {
        Channel::Slack => slack::open_socket(opening),
    }
}

/// What the platform's answer to [`open_socket`] means: where to connect.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, if
/// the channel refused, or if it accepted and named nowhere.
pub fn socket_url(channel: Channel, status: u16, body: &[u8]) -> Result<String, ChannelError> {
    match channel {
        Channel::Slack => slack::socket_url(status, body),
    }
}

/// What one frame from the event stream means, given who this instance is
/// on the channel.
#[must_use]
pub fn decode(channel: Channel, frame: &str, us: &Identity) -> Incoming {
    match channel {
        Channel::Slack => slack::decode(frame, us),
    }
}

/// The frame that acknowledges an envelope, as text to send.
#[must_use]
pub fn acknowledgement(channel: Channel, envelope: &str) -> String {
    match channel {
        Channel::Slack => slack::acknowledgement(envelope),
    }
}

/// What a request asks of a platform, read back from what would be sent.
///
/// The inverse of everything this crate renders, for a simulated platform
/// to recognise what it is asked and answer as the real one was measured
/// to, without matching on strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// Who this instance is on the channel.
    WhoAmI {
        /// Which channel.
        channel: Channel,
    },
    /// Where to connect for the event stream.
    OpenSocket {
        /// Which channel.
        channel: Channel,
    },
    /// One message posted.
    Post {
        /// Which channel it goes to.
        channel: Channel,
        /// Which room on it.
        room: String,
        /// What.
        text: String,
        /// In which thread, if any.
        thread: Option<String>,
    },
    /// A thread asked for: its parent and its most recent replies.
    Replies {
        /// Which channel.
        channel: Channel,
        /// Which room.
        room: String,
        /// Which thread, by its parent.
        thread: String,
        /// How many replies at most.
        at_most: usize,
    },
    /// One message edited in place.
    Update {
        /// Which channel it goes to.
        channel: Channel,
        /// Which room on it.
        room: String,
        /// Which message.
        message: String,
        /// What it now says.
        text: String,
    },
    /// A room created.
    CreateRoom {
        /// Which channel.
        channel: Channel,
        /// Under what name.
        name: String,
    },
    /// A room's purpose set.
    SetPurpose {
        /// Which channel.
        channel: Channel,
        /// Which room.
        room: String,
        /// To what.
        purpose: String,
    },
    /// A room's topic set.
    SetTopic {
        /// Which channel.
        channel: Channel,
        /// Which room.
        room: String,
        /// To what.
        topic: String,
    },
    /// Somebody invited into a room.
    Invite {
        /// Which channel.
        channel: Channel,
        /// Which room.
        room: String,
        /// Who.
        user: String,
    },
    /// A room archived.
    Archive {
        /// Which channel.
        channel: Channel,
        /// Which room.
        room: String,
    },
    /// A reaction put on a message.
    React {
        /// Which channel.
        channel: Channel,
        /// Which room.
        room: String,
        /// Which message.
        message: String,
        /// Which reaction.
        reaction: Reaction,
    },
}

impl Call {
    /// What a request asks, if it is one this crate renders.
    #[must_use]
    pub fn parse(request: &Request) -> Option<Self> {
        slack::call(request)
    }
}

/// A channel could not be spoken on.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The request never got an answer, or got one with a status that is
    /// not a success.
    #[error("the channel could not be reached: {0}")]
    Unreachable(String),
    /// It answered with something this cannot read.
    #[error("the channel answered something unreadable: {0}")]
    Unreadable(String),
    /// It answered, and said no.
    #[error("the channel refused it: {0}")]
    Refused(String),
    /// It accepted the message and named no thread, which should not happen.
    #[error("the channel accepted the message and did not say where it went")]
    NoIdentifier,
    /// It accepted the question and did not answer it: who this instance
    /// is, or where to connect.
    #[error("the channel accepted the question and did not answer it")]
    NoAnswer,
    /// The credential that speaks is not a bot's, so nothing it posts could
    /// be told apart from what anybody else says.
    #[error(
        "the credential is not a bot token, so this instance could not recognise its own words"
    )]
    NotABot,
}

#[cfg(test)]
mod tests {
    use super::{
        Call, ChannelError, Identity, Incoming, Reaction, ThreadRead, acknowledgement, app_form,
        archive, create_room, decode, done, foreman_room_name, identity, invite, manifest, mention,
        open_socket, permalink, pieces, post, posted, react, reference, referenced, replies,
        room_address, room_created, room_link, room_name, set_purpose, set_topic, socket_url,
        thread_read, update, who_am_i,
    };
    use stageman_core::{Channel, JobId, ProjectId, Secret, Speaking, Uuid};

    /// The form link carries the whole manifest, encoded so that it
    /// survives an address bar and decodes back to the text it came from.
    #[test]
    fn the_app_form_carries_the_manifest_whole() {
        let link = app_form(Channel::Slack, "http://localhost:8080");
        let (form, carried) = link
            .split_once("&manifest_yaml=")
            .expect("the manifest is the last parameter");
        assert_eq!(form, "https://api.slack.com/apps?new_app=1");
        assert!(
            !carried.contains('\n') && !carried.contains(' '),
            "{carried}"
        );
        assert_eq!(
            percent_encoding::percent_decode_str(carried).decode_utf8_lossy(),
            manifest(Channel::Slack)
        );
    }

    /// The manifest the link carries brings the platform back to the
    /// instance the page was served from, wherever that is, and never to the
    /// example address the file spells.
    #[test]
    fn the_manifest_carries_the_instances_own_redirect_address() {
        let link = app_form(Channel::Slack, "https://stageman.example");
        let (_, carried) = link
            .split_once("&manifest_yaml=")
            .expect("the manifest is the last parameter");
        let decoded = percent_encoding::percent_decode_str(carried).decode_utf8_lossy();
        assert!(
            decoded.contains("- https://stageman.example/instance/apps/slack/installed"),
            "{decoded}"
        );
        assert!(!decoded.contains("localhost:8080"), "{decoded}");
    }

    /// The manifest `README.md` shows a reader is this crate's, word for
    /// word: the link is composed from one text, and the other is pinned
    /// to it rather than kept in step by hand.
    #[test]
    fn the_readme_shows_the_manifest_the_app_is_created_from() {
        let readme = include_str!("../../README.md");
        let (_, after) = readme
            .split_once("## Talking to it on Slack")
            .expect("the README's Slack section");
        let (_, block) = after.split_once("```yaml\n").expect("a YAML block in it");
        let (shown, _) = block.split_once("```").expect("the block ends");
        assert_eq!(shown, manifest(Channel::Slack));
    }

    fn speaking() -> Speaking {
        Speaking {
            credential: Secret::new("xoxb-not-a-real-token".to_owned()),
        }
    }

    /// The room every post here goes to.
    const ROOM: &str = "C0123456789";

    /// What is rendered reads back as what was asked, at the root and in a
    /// thread, and the credential travels as the platform expects it.
    #[test]
    fn a_post_reads_back_as_what_it_asked() {
        let root = post(Channel::Slack, &speaking(), ROOM, "a job", None);
        assert_eq!(root.method, "POST");
        assert_eq!(
            root.headers.get("authorization").map(String::as_str),
            Some("Bearer xoxb-not-a-real-token")
        );
        assert_eq!(
            Call::parse(&root),
            Some(Call::Post {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                text: "a job".to_owned(),
                thread: None,
            })
        );

        let reply = post(
            Channel::Slack,
            &speaking(),
            ROOM,
            "said",
            Some("1788000000.000001"),
        );
        assert_eq!(
            Call::parse(&reply),
            Some(Call::Post {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                text: "said".to_owned(),
                thread: Some("1788000000.000001".to_owned()),
            })
        );
    }

    /// An answer is read through the channel it came from: the identifier
    /// of what was posted, as text, or the platform's own reason.
    #[test]
    fn an_answer_is_read_through_its_channel() {
        assert_eq!(
            posted(
                Channel::Slack,
                200,
                br#"{"ok":true,"ts":"1728312345.678901"}"#
            )
            .expect("accepted"),
            "1728312345.678901"
        );
        assert!(matches!(
            posted(Channel::Slack, 200, br#"{"ok":false,"error":"not_in_channel"}"#),
            Err(ChannelError::Refused(ref why)) if why == "not_in_channel"
        ));
    }

    /// The two questions a listener asks before it can read anything read
    /// back as what they are, and each is answered through its channel.
    #[test]
    fn a_listeners_questions_read_back_and_their_answers_are_read() {
        let asking = who_am_i(Channel::Slack, &speaking());
        assert_eq!(
            Call::parse(&asking),
            Some(Call::WhoAmI {
                channel: Channel::Slack
            })
        );
        assert_eq!(
            asking.headers.get("authorization").map(String::as_str),
            Some("Bearer xoxb-not-a-real-token"),
            "asked with the credential that speaks"
        );
        assert_eq!(
            identity(
                Channel::Slack,
                200,
                br#"{"ok":true,"user_id":"U0BOT","bot_id":"B0SELF","url":"https://example.slack.com/"}"#
            )
            .expect("told"),
            Identity {
                user: "U0BOT".to_owned(),
                bot: "B0SELF".to_owned(), url: "https://example.slack.com/".to_owned(),
            }
        );
        assert!(matches!(
            identity(Channel::Slack, 200, br#"{"ok":false,"error":"invalid_auth"}"#),
            Err(ChannelError::Refused(ref why)) if why == "invalid_auth"
        ));

        let opening = Secret::new("xapp-not-a-real-token".to_owned());
        let locating = open_socket(Channel::Slack, &opening);
        assert_eq!(
            Call::parse(&locating),
            Some(Call::OpenSocket {
                channel: Channel::Slack
            })
        );
        assert_eq!(
            locating.headers.get("authorization").map(String::as_str),
            Some("Bearer xapp-not-a-real-token"),
            "asked with the credential that opens the stream"
        );
        assert_eq!(
            socket_url(
                Channel::Slack,
                200,
                br#"{"ok":true,"url":"wss://example.invalid/link"}"#
            )
            .expect("told"),
            "wss://example.invalid/link"
        );
        assert!(matches!(
            socket_url(Channel::Slack, 200, br#"{"ok":true}"#),
            Err(ChannelError::NoAnswer)
        ));
    }

    /// A frame is read through its channel, and what it carries is
    /// acknowledged with the frame the platform expects.
    #[test]
    fn a_frame_is_read_and_acknowledged_through_its_channel() {
        let us = Identity {
            user: "U0BOT".to_owned(),
            bot: "B0SELF".to_owned(),
            url: "https://example.slack.com/".to_owned(),
        };
        let heard = decode(
            Channel::Slack,
            r#"{"envelope_id":"e-1","payload":{"event":{"type":"app_mention","channel":"C0123",
            "user":"U0HUMAN","text":"<@U0BOT> hello","ts":"1788000001.000001"}}}"#,
            &us,
        );
        let Incoming::Said { envelope, message } = heard else {
            panic!("a message: {heard:?}");
        };
        assert_eq!(envelope, "e-1");
        assert_eq!(message.room, "C0123");
        assert!(!message.from_us);
        assert_eq!(message.app, None, "a person, not an app");
        assert_eq!(
            acknowledgement(Channel::Slack, &envelope),
            r#"{"envelope_id":"e-1"}"#
        );
        assert_eq!(
            decode(Channel::Slack, r#"{"type":"hello"}"#, &us),
            Incoming::Ready
        );
    }

    /// Every request that makes or keeps a room reads back as what it
    /// asked, through the channel it was rendered for, and its answer is
    /// read the same way.
    #[test]
    fn a_rooms_requests_read_back_and_their_answers_are_read() {
        let made = create_room(
            Channel::Slack,
            &speaking(),
            "aviary--fix-the-build--3fa85f64",
        );
        assert_eq!(
            Call::parse(&made),
            Some(Call::CreateRoom {
                channel: Channel::Slack,
                name: "aviary--fix-the-build--3fa85f64".to_owned(),
            })
        );
        assert_eq!(
            room_created(
                Channel::Slack,
                200,
                br#"{"ok":true,"channel":{"id":"C0C1VNX9AA2","name":"aviary--fix-the-build--3fa85f64"}}"#
            )
            .expect("made"),
            "C0C1VNX9AA2"
        );
        assert!(matches!(
            room_created(Channel::Slack, 200, br#"{"ok":false,"error":"name_taken"}"#),
            Err(ChannelError::Refused(ref why)) if why == "name_taken"
        ));
        assert!(matches!(
            room_created(Channel::Slack, 200, br#"{"ok":true}"#),
            Err(ChannelError::NoAnswer)
        ));

        assert_eq!(
            Call::parse(&set_purpose(Channel::Slack, &speaking(), ROOM, "why")),
            Some(Call::SetPurpose {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                purpose: "why".to_owned(),
            })
        );
        assert_eq!(
            Call::parse(&set_topic(Channel::Slack, &speaking(), ROOM, "where")),
            Some(Call::SetTopic {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                topic: "where".to_owned(),
            })
        );
        assert_eq!(
            Call::parse(&invite(Channel::Slack, &speaking(), ROOM, "U0HUMAN")),
            Some(Call::Invite {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                user: "U0HUMAN".to_owned(),
            })
        );
        assert_eq!(
            Call::parse(&archive(Channel::Slack, &speaking(), ROOM)),
            Some(Call::Archive {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
            })
        );
        for reaction in [Reaction::Seen, Reaction::Done] {
            assert_eq!(
                Call::parse(&react(
                    Channel::Slack,
                    &speaking(),
                    ROOM,
                    "1788000000.000001",
                    reaction
                )),
                Some(Call::React {
                    channel: Channel::Slack,
                    room: ROOM.to_owned(),
                    message: "1788000000.000001".to_owned(),
                    reaction,
                })
            );
        }
        assert!(done(Channel::Slack, 200, br#"{"ok":true}"#).is_ok());
        assert!(matches!(
            done(Channel::Slack, 200, br#"{"ok":false,"error":"already_archived"}"#),
            Err(ChannelError::Refused(ref why)) if why == "already_archived"
        ));
    }

    /// A room's name is the project's part folded to what the channel
    /// allows and the job's name whole, which is the part that is always
    /// there — see `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
    #[test]
    fn a_rooms_name_is_the_project_and_the_jobs_name() {
        let minted = Uuid::from_u128(0x3fa8_5f64_5717_4562_b3fc_2c96_3f66_afa6);
        let job = JobId::named("Fix the flaky parser test!", &minted);
        assert_eq!(
            room_name(Channel::Slack, "Closed Loop", &job),
            "closed-loop--fix-the-flaky-parser-test--3fa85f64"
        );
        assert_eq!(
            room_name(Channel::Slack, "", &job),
            "fix-the-flaky-parser-test--3fa85f64",
            "a project with nothing left in its part is left out"
        );
        assert_eq!(
            room_name(Channel::Slack, "aviary", &JobId::named("", &minted)),
            "aviary--job--3fa85f64",
            "a name with nothing to say still says what it is"
        );
        let long = room_name(
            Channel::Slack,
            &"p".repeat(60),
            &JobId::named(&"t ".repeat(120), &minted),
        );
        assert!(long.len() <= 80, "{long}");
        assert!(long.ends_with("--3fa85f64"), "{long}");
        assert_eq!(
            foreman_room_name(
                Channel::Slack,
                "Closed Loop",
                ProjectId::from_uuid(Uuid::from_u128(0x3fa8_5f64_5717_4562_b3fc_2c96_3f66_afa6))
            ),
            "closed-loop--foreman--3fa85f64"
        );
        assert_eq!(room_link(Channel::Slack, "C0C1VNX9AA2"), "<#C0C1VNX9AA2>");
        assert_eq!(mention(Channel::Slack, "U0HUMAN"), "<@U0HUMAN>");
    }

    /// A thread asked for reads back as what it asked, and its answer is
    /// read as a frame's messages are: a person's, this instance's own and
    /// another app's told apart by identifier, which of a person's mention
    /// this instance, and whether older replies were left out.
    #[test]
    fn a_thread_reads_back_and_its_answer_is_read() {
        let asked = replies(Channel::Slack, &speaking(), ROOM, "1788000000.000100", 50);
        assert_eq!(asked.method, "GET");
        assert_eq!(
            asked.headers.get("authorization").map(String::as_str),
            Some("Bearer xoxb-not-a-real-token")
        );
        assert_eq!(
            Call::parse(&asked),
            Some(Call::Replies {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                thread: "1788000000.000100".to_owned(),
                at_most: 50,
            })
        );

        let us = Identity {
            user: "U0BOT".to_owned(),
            bot: "B0SELF".to_owned(),
            url: "https://example.slack.com/".to_owned(),
        };
        let body = br##"{"ok":true,"has_more":true,"messages":[
            {"type":"message","user":"U0HUMAN","text":"<@U0BOT> which database?","ts":"1788000000.000100","thread_ts":"1788000000.000100","reply_count":3},
            {"type":"message","user":"U0BOT","bot_id":"B0SELF","text":"Two options, <@U0BOT> said.","ts":"1788000000.000200","thread_ts":"1788000000.000100"},
            {"type":"message","user":"U0BOT","text":"As a user alone.","ts":"1788000000.000250","thread_ts":"1788000000.000100"},
            {"type":"message","user":"U0GITHUB","bot_id":"B0OTHER","bot_profile":{"name":"GitHub"},"text":"","ts":"1788000000.000300","thread_ts":"1788000000.000100","attachments":[{"pretext":"Issue closed for <@U0BOT>","title":"#9 Done"}]}
        ]}"##;
        let ThreadRead {
            messages,
            mentioning,
            longer,
        } = thread_read(Channel::Slack, 200, body, ROOM, &us).expect("a thread read");
        assert!(longer, "older replies were left out");
        assert_eq!(messages.len(), 4);
        assert_eq!(
            mentioning.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["1788000000.000100"],
            "a person's mention: not this instance naming itself, and not an app doing so"
        );
        assert_eq!(messages[0].user.as_deref(), Some("U0HUMAN"));
        assert!(!messages[0].from_us && messages[0].app.is_none());
        assert_eq!(messages[0].room, ROOM);
        assert_eq!(messages[0].thread.as_deref(), Some("1788000000.000100"));
        assert!(
            messages[1].from_us,
            "this instance's own, by its bot identifier"
        );
        assert!(
            messages[2].from_us && messages[2].app.is_none(),
            "this instance's own, by its user alone"
        );
        assert_eq!(messages[3].app.as_deref(), Some("GitHub"));
        assert!(messages[3].text.contains("#9 Done"), "{}", messages[3].text);
        assert!(matches!(
            thread_read(
                Channel::Slack,
                200,
                br#"{"ok":false,"error":"thread_not_found"}"#,
                ROOM,
                &us
            ),
            Err(ChannelError::Refused(ref why)) if why == "thread_not_found"
        ));
        assert!(matches!(
            thread_read(Channel::Slack, 500, b"", ROOM, &us),
            Err(ChannelError::Unreachable(_))
        ));
    }

    /// An identifier shown to an agent reads back as the room and the
    /// message it names, and anything else is nothing.
    #[test]
    fn a_reference_reads_back_as_the_room_and_the_message() {
        let shown = reference(Channel::Slack, ROOM, "1788000000.000100");
        assert_eq!(shown, "C0123456789/1788000000.000100");
        assert_eq!(
            referenced(Channel::Slack, &shown),
            Some((ROOM.to_owned(), "1788000000.000100".to_owned()))
        );
        assert_eq!(referenced(Channel::Slack, "1788000000.000100"), None);
        assert_eq!(referenced(Channel::Slack, "/1788000000.000100"), None);
        assert_eq!(referenced(Channel::Slack, "C0123456789/"), None);
    }

    /// A link to a message is the workspace's address, the room and the
    /// message's digits; a reply's names its thread as well. As measured on
    /// 2026-09-21 against a real workspace.
    #[test]
    fn a_link_to_a_message_reads_as_the_platform_spells_one() {
        let us = Identity {
            user: "U0BOT".to_owned(),
            bot: "B0SELF".to_owned(),
            url: "https://example.slack.com/".to_owned(),
        };
        assert_eq!(
            permalink(Channel::Slack, &us, ROOM, "1788000000.000100", None),
            "https://example.slack.com/archives/C0123456789/p1788000000000100"
        );
        assert_eq!(
            permalink(
                Channel::Slack,
                &us,
                ROOM,
                "1788000000.000200",
                Some("1788000000.000100")
            ),
            "https://example.slack.com/archives/C0123456789/p1788000000000200\
             ?thread_ts=1788000000.000100&cid=C0123456789"
        );
        assert_eq!(
            permalink(
                Channel::Slack,
                &us,
                ROOM,
                "1788000000.000100",
                Some("1788000000.000100")
            ),
            "https://example.slack.com/archives/C0123456789/p1788000000000100",
            "a thread's parent is linked as itself"
        );
    }

    /// A link to a room is a message's link without the message.
    #[test]
    fn a_link_to_a_room_is_the_workspaces_address_and_the_room() {
        let us = Identity {
            user: "U0BOT".to_owned(),
            bot: "B0SELF".to_owned(),
            url: "https://example.slack.com/".to_owned(),
        };
        assert_eq!(
            room_address(Channel::Slack, &us, ROOM),
            "https://example.slack.com/archives/C0123456789"
        );
    }

    /// An edit reads back as what it asked, and is answered as a post is.
    #[test]
    fn an_edit_reads_back_as_what_it_asked() {
        let edited = update(
            Channel::Slack,
            &speaking(),
            ROOM,
            "1788000000.000001",
            "grown",
        );
        assert_eq!(
            Call::parse(&edited),
            Some(Call::Update {
                channel: Channel::Slack,
                room: ROOM.to_owned(),
                message: "1788000000.000001".to_owned(),
                text: "grown".to_owned(),
            })
        );
        assert_eq!(
            posted(
                Channel::Slack,
                200,
                br#"{"ok":true,"ts":"1788000000.000001"}"#
            )
            .expect("accepted"),
            "1788000000.000001"
        );
    }

    /// A text a post can carry is one piece; a longer one continues in the
    /// next, cut at a line end when there is one late enough, and nothing is
    /// lost between them.
    #[test]
    fn a_long_text_is_cut_into_posts_at_line_ends() {
        assert_eq!(pieces(Channel::Slack, "short"), vec!["short".to_owned()]);
        assert_eq!(pieces(Channel::Slack, ""), vec![String::new()]);

        let line = "x".repeat(99);
        let long = std::iter::repeat_n(line.as_str(), 130)
            .collect::<Vec<_>>()
            .join("\n");
        let cut = pieces(Channel::Slack, &long);
        assert_eq!(
            cut.len(),
            2,
            "{:?}",
            cut.iter().map(String::len).collect::<Vec<_>>()
        );
        assert!(cut[0].chars().count() <= 12_000);
        assert!(cut[0].ends_with(&line), "cut at a line end, not inside one");
        assert!(!cut[1].starts_with('\n'), "the break itself is not carried");
        assert_eq!(cut.join("\n"), long, "nothing lost");

        let unbroken = "y".repeat(12_001);
        let cut = pieces(Channel::Slack, &unbroken);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].chars().count(), 12_000);
        assert_eq!(cut[1], "y");
    }

    /// A request this crate did not render is not a call.
    #[test]
    fn what_this_did_not_render_is_not_a_call() {
        let mut other = post(Channel::Slack, &speaking(), ROOM, "x", None);
        other.url = "https://example.test/api".to_owned();
        assert_eq!(Call::parse(&other), None);

        let mut bodiless = post(Channel::Slack, &speaking(), ROOM, "x", None);
        bodiless.body = None;
        assert_eq!(Call::parse(&bodiless), None);
    }

    /// The cutter's boundaries, each pinned: a text exactly as long as a
    /// post is one piece; a line end just past the limit is not where the
    /// cut goes, the last one inside the post's second half is; and a line
    /// end in the first half is passed over for a cut at a character.
    #[test]
    fn a_text_is_cut_at_its_boundaries_exactly() {
        let exactly = "z".repeat(12_000);
        assert_eq!(pieces(Channel::Slack, &exactly), vec![exactly.clone()]);

        let just_past = format!(
            "{}\n{}\n{}",
            "a".repeat(8_000),
            "b".repeat(3_999),
            "c".repeat(50)
        );
        assert_eq!(
            pieces(Channel::Slack, &just_past),
            vec![
                "a".repeat(8_000),
                format!("{}\n{}", "b".repeat(3_999), "c".repeat(50)),
            ],
            "cut at the line end inside the post, not at the one just past it"
        );

        let early = format!("{}\n{}", "a".repeat(100), "b".repeat(12_000));
        let cut = pieces(Channel::Slack, &early);
        assert_eq!(cut.len(), 2);
        assert_eq!(
            cut[0].chars().count(),
            12_000,
            "a line end in the first half is not where a post is cut"
        );
    }
}
