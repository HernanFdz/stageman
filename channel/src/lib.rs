//! The contract every channel is spoken on, and the adapters that implement
//! it.
//!
//! One contract: a message posted at the root of an address or in a thread
//! on it, and — once the listener moves here — an event stream opened,
//! frames read and acknowledged. Since
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

use std::collections::BTreeMap;

use stageman_core::{Channel, Speaking};

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

/// Renders posting one message on a channel: at the root of the address
/// when `thread` is none, and in that thread otherwise.
#[must_use]
pub fn post(channel: Channel, speaking: &Speaking, text: &str, thread: Option<&str>) -> Request {
    match channel {
        Channel::Slack => slack::post(speaking, text, thread),
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

/// What a request asks of a platform, read back from what would be sent.
///
/// The inverse of [`post`], for a simulated platform to recognise what it is
/// asked and answer as the real one was measured to, without matching on
/// strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// One message posted.
    Post {
        /// Which channel it goes to.
        channel: Channel,
        /// Where on it.
        address: String,
        /// What.
        text: String,
        /// In which thread, if any.
        thread: Option<String>,
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
}

#[cfg(test)]
mod tests {
    use super::{Call, ChannelError, post, posted};
    use stageman_core::{Channel, Secret, Speaking};

    fn speaking() -> Speaking {
        Speaking {
            address: "C0123456789".to_owned(),
            credential: Secret::new("xoxb-not-a-real-token".to_owned()),
        }
    }

    /// What is rendered reads back as what was asked, at the root and in a
    /// thread, and the credential travels as the platform expects it.
    #[test]
    fn a_post_reads_back_as_what_it_asked() {
        let root = post(Channel::Slack, &speaking(), "a job", None);
        assert_eq!(root.method, "POST");
        assert_eq!(
            root.headers.get("authorization").map(String::as_str),
            Some("Bearer xoxb-not-a-real-token")
        );
        assert_eq!(
            Call::parse(&root),
            Some(Call::Post {
                channel: Channel::Slack,
                address: "C0123456789".to_owned(),
                text: "a job".to_owned(),
                thread: None,
            })
        );

        let reply = post(
            Channel::Slack,
            &speaking(),
            "said",
            Some("1788000000.000001"),
        );
        assert_eq!(
            Call::parse(&reply),
            Some(Call::Post {
                channel: Channel::Slack,
                address: "C0123456789".to_owned(),
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

    /// A request this crate did not render is not a call.
    #[test]
    fn what_this_did_not_render_is_not_a_call() {
        let mut other = post(Channel::Slack, &speaking(), "x", None);
        other.url = "https://example.test/api".to_owned();
        assert_eq!(Call::parse(&other), None);

        let mut bodiless = post(Channel::Slack, &speaking(), "x", None);
        bodiless.body = None;
        assert_eq!(Call::parse(&bodiless), None);
    }
}
