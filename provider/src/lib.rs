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

/// What a purse of this kind looks like, for the box it is pasted into:
/// what the shape check will require, said as a placeholder, so that the box
/// teaches the shape before a refusal does.
#[must_use]
pub const fn example(purse: PurseName) -> &'static str {
    match purse.provider() {
        Provider::Anthropic => anthropic::example(purse),
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
/// an error message is a place credentials escape. Nor with where the paste
/// belongs instead, which is the page's to say — the page is what has a box
/// per purse.
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
    /// on, and nothing about where else a paste might belong.
    pub expected: &'static str,
}

#[cfg(test)]
mod tests {
    use super::{example, guide, shaped};
    use stageman_core::PurseName;

    /// What a box shows as an example has the shape its check requires, and
    /// not the other box's: a placeholder that passed both checks would teach
    /// nothing, and one that passed neither would teach a refusal.
    #[test]
    fn every_example_has_the_shape_its_own_box_requires_and_no_other() {
        for purse in PurseName::ALL {
            assert_eq!(shaped(*purse, example(*purse)), Ok(()), "{purse:?}");
            for other in PurseName::ALL.iter().filter(|other| *other != purse) {
                assert!(
                    shaped(*other, example(*purse)).is_err(),
                    "{purse:?}'s example passes as {other:?}"
                );
            }
        }
    }

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
