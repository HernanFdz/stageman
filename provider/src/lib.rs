//! The contract every provider is asked on by this daemon, and the adapters
//! that implement it.
//!
//! What a purse is checked with, what the answer means, the shape a
//! credential of each kind has, and where each is minted — as pure
//! functions the **instance** renders and reads and the world carries,
//! dispatched on the provider at this surface so that nothing outside names
//! one. See `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`.
//!
//! What an agent reads a purse *from* is deliberately not here: the
//! variable is the agent's, per `docs/conventions.md` §3, and lives in the
//! agent crate. This crate knows what a credential looks like and where it
//! comes from, which is the provider's.

use stageman_core::{Provider, PurseName};

mod anthropic;

/// Where the provider's own page for minting a purse of this kind is, and
/// what to say beside the link.
///
/// A link and nothing more, on the terms
/// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
/// sets for a platform's form: the credential is minted there, and a copy
/// of that page here would restate it and drift. Composed on the server
/// from tracked text, never in the browser.
#[must_use]
pub const fn guide(purse: PurseName) -> Guide {
    match purse.provider() {
        Provider::Anthropic => anthropic::guide(purse),
    }
}

/// A guide to where a purse is minted: the link, the verb on it, and the
/// sentence a hover away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guide {
    /// The provider's own page.
    pub link: &'static str,
    /// What pressing it opens, in a word or two.
    pub label: &'static str,
    /// What to do there and what to bring back, in a sentence.
    pub says: &'static str,
}

/// Whether what was pasted has the shape of the purse it was pasted as.
///
/// The kind is the box the operator pressed, never the text; this is what
/// refuses the text that belongs in the other box, before anything is kept
/// or asked of the provider. Refused loudly because the alternative was
/// measured: a subscription's token delivered under the key's variable does
/// not fail, it hangs.
///
/// # Errors
///
/// Fails with what the shape ought to be, and never with what was pasted:
/// an error message is a place credentials escape.
pub fn shaped(purse: PurseName, pasted: &str) -> Result<(), Misshapen> {
    match purse.provider() {
        Provider::Anthropic => anthropic::shaped(purse, pasted),
    }
}

/// What was pasted does not have the shape of the purse it was pasted as.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{expected}")]
pub struct Misshapen {
    /// Which purse it was pasted as.
    pub purse: PurseName,
    /// What a credential of that kind looks like, in words a person can act
    /// on.
    pub expected: &'static str,
}

#[cfg(test)]
mod tests {
    use super::{guide, shaped};
    use stageman_core::PurseName;

    /// Every purse has somewhere it is minted, the link is the provider's
    /// own site over a secure scheme, and the words beside it say something.
    #[test]
    fn every_purse_is_guided_to_its_providers_own_page() {
        for purse in PurseName::ALL {
            let guide = guide(*purse);
            assert!(guide.link.starts_with("https://"), "{purse:?}: {guide:?}");
            assert!(
                !guide.label.is_empty() && !guide.says.is_empty(),
                "{guide:?}"
            );
        }
    }

    /// The text that belongs in the other box is refused, and the refusal
    /// says what the box takes rather than what was pasted.
    #[test]
    fn a_credential_pasted_into_the_wrong_box_is_refused_without_being_echoed() {
        let subscription = "sk-ant-oat01-secret-value";
        let key = "sk-ant-api03-secret-value";

        assert_eq!(
            shaped(PurseName::AnthropicSubscription, subscription),
            Ok(())
        );
        assert_eq!(shaped(PurseName::AnthropicKey, key), Ok(()));

        let refused = shaped(PurseName::AnthropicKey, subscription).expect_err("the other box");
        assert_eq!(refused.purse, PurseName::AnthropicKey);
        assert!(!refused.to_string().contains("secret-value"), "{refused}");
        assert!(refused.to_string().contains("sk-ant-api"), "{refused}");

        let refused = shaped(PurseName::AnthropicSubscription, key).expect_err("the other box");
        assert!(!refused.to_string().contains("secret-value"), "{refused}");
        assert!(refused.to_string().contains("sk-ant-oat"), "{refused}");

        assert!(shaped(PurseName::AnthropicKey, "").is_err());
        assert!(shaped(PurseName::AnthropicSubscription, "ghp-not-anthropics").is_err());
    }
}
