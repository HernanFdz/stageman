//! Anthropic: where its two purses are minted, and what each looks like.
//!
//! Read from the vendor's own documentation on 2026-09-30. A key is minted
//! on the Console's keys page and begins `sk-ant-api`; a subscription's
//! token is minted by `claude setup-token`, for a year, and begins
//! `sk-ant-oat`. The two are told apart by the box they are pasted into,
//! and this module is what refuses the text that belongs in the other.

use stageman_core::PurseName;

use crate::{Guide, Misshapen};

/// What a key begins with.
const KEY_PREFIX: &str = "sk-ant-api";

/// What a subscription's token begins with.
const SUBSCRIPTION_PREFIX: &str = "sk-ant-oat";

/// Where each purse is minted, and what to say beside the link.
pub const fn guide(purse: PurseName) -> Guide {
    match purse {
        PurseName::AnthropicKey => Guide {
            link: "https://console.anthropic.com/settings/keys",
            label: "New key",
            says: "Opens the Anthropic Console at its keys page. Create a key there and paste it \
                   here; it is checked against Anthropic before it is kept.",
        },
        PurseName::AnthropicSubscription => Guide {
            link: "https://code.claude.com/docs/en/authentication#generate-a-long-lived-token",
            label: "How to mint one",
            says: "Opens Claude Code's own page on long-lived tokens. Run `claude setup-token` \
                   where Claude Code is signed in to the subscription, and paste the token it \
                   prints here; it lasts a year.",
        },
    }
}

/// Whether the text begins as a purse of that kind does.
pub fn shaped(purse: PurseName, pasted: &str) -> Result<(), Misshapen> {
    let (prefix, expected) = match purse {
        PurseName::AnthropicKey => (
            KEY_PREFIX,
            "an Anthropic API key begins with sk-ant-api; a subscription's token goes in the \
             other box",
        ),
        PurseName::AnthropicSubscription => (
            SUBSCRIPTION_PREFIX,
            "a Claude subscription's token, from `claude setup-token`, begins with sk-ant-oat; \
             an API key goes in the other box",
        ),
    };
    if pasted.starts_with(prefix) {
        Ok(())
    } else {
        Err(Misshapen { purse, expected })
    }
}
