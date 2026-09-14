//! Asking a platform something with one credential and no arguments, from
//! the daemon.
//!
//! What is left here of speaking on a channel: the two calls the listener
//! makes before it can read anything — where to connect, and who this
//! instance is — are both of this shape, and the listener is what still
//! makes them from the app. Posting is the instance's since
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`,
//! rendered and read by the channel crate and made by the world; this goes
//! the same way with the listener.

pub use stageman_channel::ChannelError;

/// Asks the platform something that takes no arguments, with one credential.
///
/// # Errors
///
/// Fails if the platform cannot be reached, or refuses.
#[mutants::skip]
pub async fn ask(
    endpoint: &str,
    credential: &stageman_core::Secret,
) -> Result<serde_json::Value, ChannelError> {
    let answer = reqwest::Client::new()
        .post(endpoint)
        .bearer_auth(credential.expose())
        .send()
        .await
        .map_err(|failure| ChannelError::Unreachable(failure.to_string()))?;

    let status = answer.status().as_u16();
    let body = answer
        .text()
        .await
        .map_err(|failure| ChannelError::Unreachable(failure.to_string()))?;

    understood_value(status, &body)
}

/// What an answer to [`ask`] means, stopping at the whole answer.
///
/// The guard that matters — a refusal arriving as 200 — is the same guard
/// the channel crate keeps for a post, and it is kept here too because this
/// is separate code that would fail in a way nobody would attribute to a
/// missing check: it would connect to a URL that is not there.
///
/// # Errors
///
/// Fails if the status was not a success, if the body cannot be read, or if
/// the platform refused.
fn understood_value(status: u16, body: &str) -> Result<serde_json::Value, ChannelError> {
    if !(200..300).contains(&status) {
        return Err(ChannelError::Unreachable(format!(
            "the channel answered {status}"
        )));
    }

    let answered: serde_json::Value = serde_json::from_str(body)
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

#[cfg(test)]
mod tests {
    use super::{ChannelError, understood_value};

    /// A refusal that arrives as 200 is a refusal here too.
    #[test]
    fn a_refusal_is_a_refusal_for_this_reader_too() {
        assert!(matches!(
            understood_value(200, r#"{"ok":false,"error":"invalid_auth"}"#),
            Err(ChannelError::Refused(ref why)) if why == "invalid_auth"
        ));
        // Accepted and saying nothing is not a failure to read.
        assert!(matches!(
            understood_value(200, r#"{"ok":true}"#).map(|told| told
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)),
            Ok(None)
        ));
        assert_eq!(
            understood_value(200, r#"{"ok":true,"url":"wss://example.invalid/link"}"#)
                .expect("accepted")
                .get("url")
                .and_then(serde_json::Value::as_str),
            Some("wss://example.invalid/link")
        );
        assert!(matches!(
            understood_value(500, r#"{"ok":true}"#),
            Err(ChannelError::Unreachable(_))
        ));
    }
}
