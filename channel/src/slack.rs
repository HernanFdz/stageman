//! Slack: one message posted, and what it answers.
//!
//! **A thread has no independent existence**, which is why a job's thread
//! is opened by the daemon rather than by the job. There is no call that
//! creates an empty one: a thread is a message plus the replies hanging from
//! it, so somebody posts that message before the job's container starts,
//! and the identifier it comes back with is what a reply names — see
//! `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.

use stageman_core::{Channel, Speaking};

use crate::{Call, ChannelError, Request};

/// Where Slack takes a message.
const POST_MESSAGE: &str = "https://slack.com/api/chat.postMessage";

/// Renders one message posted: at the root of the address, or in a thread.
pub fn post(speaking: &Speaking, text: &str, thread: Option<&str>) -> Request {
    // Built as a map rather than indexed into as a value: indexing into a
    // value is the operation the gate is right to call a panic, and a map
    // has nothing to fail on. The thread is left out rather than sent as
    // null, because the platform reads a present-but-empty field as an
    // address rather than as no address.
    let mut posting = serde_json::Map::new();
    posting.insert("channel".to_owned(), speaking.address.as_str().into());
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

/// What a request asks, if it is a post this module rendered.
pub fn call(request: &Request) -> Option<Call> {
    if request.method != "POST" || request.url != POST_MESSAGE {
        return None;
    }
    let posted: Posted = serde_json::from_slice(request.body.as_deref()?).ok()?;
    Some(Call::Post {
        channel: Channel::Slack,
        address: posted.channel,
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
    use super::posted;
    use crate::ChannelError;

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
