//! Speaking on a project's channel, from the daemon rather than from a job.
//!
//! **The other half of `stageman-say`**, and it exists because a thread has no
//! independent existence: there is no call that creates an empty one, so a
//! thread is a message plus the replies hanging from it. Somebody has to post
//! that message before a job's container starts, because the container is
//! given the thread it speaks in at creation and never afterwards.
//!
//! That somebody cannot be the job. It has not started yet, and if it posted
//! its own root message the instance would not learn the identifier — which is
//! the one thing the instance needs, because a reply arrives naming a thread
//! and nothing else. See
//! `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.
//!
//! Only the daemon reads this module. A job's agent never gets an HTTP client
//! or an endpoint; it gets one command with one argument, per
//! `docs/decisions/0028-stageman-ships-the-tool-that-speaks.md`.

use stageman_core::{Channel, Speaking, Thread};

/// Where Slack takes a message.
const POST_MESSAGE: &str = "https://slack.com/api/chat.postMessage";

/// Opens a thread by posting the message the rest of it hangs from.
///
/// # Errors
///
/// Fails if the channel cannot be reached, or refuses.
pub async fn open_thread(
    bound: &Speaking,
    channel: Channel,
    text: &str,
) -> Result<Thread, ChannelError> {
    let posted = match channel {
        Channel::Slack => slack_post(bound, text, None).await?,
    };
    Ok(Thread {
        channel,
        id: posted,
    })
}

/// Says something in a thread that already exists.
///
/// What the daemon uses to speak for itself — a notice that a job's agent has
/// stopped, or that a reply could not be delivered. A job's *own* words never
/// come through here: those go out through the tool in its container, per
/// `docs/decisions/0028-stageman-ships-the-tool-that-speaks.md`, and keeping
/// the two paths apart is what stops this becoming a second way for a job to
/// talk.
///
/// # Errors
///
/// Fails if the channel cannot be reached, or refuses.
///
/// Skipped by mutation testing for the reason [`slack_post`] is: it chooses
/// nothing. Everything that could be got wrong about an answer is in
/// [`understood`], and reaching this needs a network.
#[mutants::skip]
pub async fn say_in(bound: &Speaking, thread: &Thread, text: &str) -> Result<(), ChannelError> {
    match thread.channel {
        Channel::Slack => slack_post(bound, text, Some(&thread.id)).await?,
    };
    Ok(())
}

/// Posts one message and hands the answer to [`understood`].
///
/// **Does the sending and decides nothing.** Everything that could be got
/// wrong is in `understood`, which is pure and tested; what is left here is a
/// request and two moves of a string. Reaching this at all needs a network, and
/// a test with one would be testing Slack rather than this.
#[mutants::skip]
async fn slack_post(
    bound: &Speaking,
    text: &str,
    thread: Option<&str>,
) -> Result<String, ChannelError> {
    let answer = reqwest::Client::new()
        .post(POST_MESSAGE)
        .bearer_auth(bound.credential.expose())
        .json(&Posting {
            channel: &bound.address,
            text,
            thread_ts: thread,
        })
        .send()
        .await
        .map_err(|failure| ChannelError::Unreachable(failure.to_string()))?;

    let status = answer.status().as_u16();
    let body = answer
        .text()
        .await
        .map_err(|failure| ChannelError::Unreachable(failure.to_string()))?;

    understood(status, &body)
}

/// What an answer from the channel means.
///
/// Pure, so that the guard below can be tested without a network — the split
/// the rest of this project uses wherever an I/O function would otherwise hide
/// a decision. It was not split when it was written, and mutation testing
/// deleted the negation from that guard without a single test noticing.
///
/// Returns the identifier **as text**, never parsed. It looks like a number and
/// is not one: through an `f64` it loses the microseconds and comes back
/// addressing no message, and the failure that produces reads like a
/// permissions problem rather than a rounding one.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, if the
/// channel refused the message, or if it accepted one and named no thread.
fn understood(status: u16, body: &str) -> Result<String, ChannelError> {
    if !(200..300).contains(&status) {
        return Err(ChannelError::Unreachable(format!(
            "the channel answered {status}"
        )));
    }

    let answered: Answered = serde_json::from_str(body)
        .map_err(|failure| ChannelError::Unreadable(failure.to_string()))?;

    // **A refusal arrives as 200.** Slack reports a bad token or an unknown
    // channel in the body with a successful status, so a check of the status
    // alone calls every one of them a message delivered. The tool in the image
    // guards the same thing for the same reason; the two are separate because
    // the failure would be separate.
    if !answered.ok {
        return Err(ChannelError::Refused(
            answered
                .error
                .unwrap_or_else(|| "no reason given".to_owned()),
        ));
    }
    answered.ts.ok_or(ChannelError::NoIdentifier)
}

/// One message, on its way out.
///
/// A type rather than a `json!` literal, because building it by indexing into
/// a `Value` is an operation the gate is right to call a panic: it is only
/// infallible as long as the thing being indexed stays an object, and nothing
/// says it must.
#[derive(serde::Serialize)]
struct Posting<'a> {
    channel: &'a str,
    text: &'a str,
    /// Absent when this opens a thread, and the parent's identifier when it
    /// speaks in one. Skipped rather than sent as null, because the platform
    /// reads a present-but-empty field as an address rather than as no address.
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}

/// What Slack says back.
#[derive(serde::Deserialize)]
struct Answered {
    ok: bool,
    /// Present when it worked, and the identifier of the thread just opened.
    ts: Option<String>,
    /// Present when it did not.
    error: Option<String>,
}

/// A channel could not be spoken on.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The request never got an answer.
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
    use super::{ChannelError, understood};

    /// The guard that exists because a refusal arrives as 200.
    ///
    /// The whole reason this module has a pure half: without it, deleting the
    /// negation from that check turns every bad token and every wrong channel
    /// into a message reported as delivered — and the job then stops as
    /// instructed, having told nobody anything.
    #[test]
    fn a_refusal_with_a_successful_status_is_still_a_refusal() {
        let refused = understood(200, r#"{"ok":false,"error":"not_in_channel"}"#);

        assert!(
            matches!(refused, Err(ChannelError::Refused(ref why)) if why == "not_in_channel"),
            "{refused:?}"
        );
    }

    /// A refusal that names no reason is still a refusal.
    #[test]
    fn a_refusal_without_a_reason_says_so_rather_than_reading_as_success() {
        let refused = understood(200, r#"{"ok":true is not valid"#);
        assert!(
            matches!(refused, Err(ChannelError::Unreadable(_))),
            "{refused:?}"
        );

        let refused = understood(200, r#"{"ok":false}"#);
        assert!(
            matches!(refused, Err(ChannelError::Refused(ref why)) if why == "no reason given"),
            "{refused:?}"
        );
    }

    /// The identifier comes back exactly as it arrived.
    #[test]
    fn an_accepted_message_answers_with_its_identifier_as_text() {
        assert_eq!(
            understood(200, r#"{"ok":true,"ts":"1728312345.678901"}"#).expect("it was accepted"),
            // Not 1728312345.678901_f64, which is a different string on the
            // way back out and addresses no message.
            "1728312345.678901"
        );
    }

    /// Accepted and unidentified should not read as accepted.
    #[test]
    fn a_message_accepted_without_a_thread_is_not_a_thread() {
        assert!(
            matches!(
                understood(200, r#"{"ok":true}"#),
                Err(ChannelError::NoIdentifier)
            ),
            "an answer naming no thread is not somewhere to speak"
        );
    }

    /// A status outside the successful range never reaches the body.
    #[test]
    fn an_unsuccessful_status_is_unreachable_rather_than_refused() {
        for status in [400, 429, 500, 503] {
            let answer = understood(status, r#"{"ok":true,"ts":"1.1"}"#);
            assert!(
                matches!(answer, Err(ChannelError::Unreachable(_))),
                "{status} should not have been read as an answer: {answer:?}"
            );
        }
        // And the boundaries of that range are the boundaries, so a redirect
        // is not quietly treated as a success.
        assert!(understood(299, r#"{"ok":true,"ts":"1.1"}"#).is_ok());
        assert!(matches!(
            understood(300, r#"{"ok":true,"ts":"1.1"}"#),
            Err(ChannelError::Unreachable(_))
        ));
    }
}
