//! Anthropic: where its two purses are minted, what each looks like, and
//! the one read either is checked by.
//!
//! Read from the vendor's own documentation on 2026-09-30. A key is minted
//! on the Console's keys page and begins `sk-ant-api`; a subscription's
//! token is minted by `claude setup-token`, for a year, and begins
//! `sk-ant-oat`. The two are told apart by the box they are pasted into;
//! this module is what shows each box what it takes, and what refuses the
//! text that belongs in the other.
//!
//! The check is a listing of models, measured against the real provider on
//! 2026-09-30 and 2026-10-01. A key travels in the key's own header and a
//! subscription's token as a bearer, and each is answered `200` when it is
//! good; a made-up one of either kind is answered `401` with one error
//! object carrying a sentence — *API key is invalid.* for a key, *OAuth
//! access token is invalid.* for a token. The subscription's token needs no
//! header beyond the bearer, though the vendor's own client sends one, and
//! neither a refusal nor a good token's answer depended on the client
//! naming itself, so the request carries the version the provider requires,
//! the credential, and nothing else. See
//! `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`.

use stageman_core::{Provider, PurseName, Secret};

use crate::{Call, Guide, Misshapen, ProviderError, Request};

/// What a key begins with.
const KEY_PREFIX: &str = "sk-ant-api";

/// What a subscription's token begins with.
const SUBSCRIPTION_PREFIX: &str = "sk-ant-oat";

/// What a key looks like, as far as a placeholder should say.
const KEY_EXAMPLE: &str = "sk-ant-api03-…";

/// What a subscription's token looks like, as far as a placeholder should
/// say.
const SUBSCRIPTION_EXAMPLE: &str = "sk-ant-oat01-…";

/// Where the provider lists its models: the read a purse is checked by. One
/// model is asked for, the fewest the listing gives, since nothing in it is
/// read.
const LISTING: &str = "https://api.anthropic.com/v1/models?limit=1";

/// The header naming which version of its API a request is written against,
/// which the provider requires of every request.
const VERSION_HEADER: &str = "anthropic-version";

/// The version the check was measured against.
const VERSION: &str = "2023-06-01";

/// The header a key travels in.
const KEY_HEADER: &str = "x-api-key";

/// The header a subscription's token travels in.
const TOKEN_HEADER: &str = "authorization";

/// What comes before a subscription's token in its header.
const BEARER: &str = "Bearer ";

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
            says: "Opens Claude Code's own page on long-lived tokens. Run claude setup-token where \
                   Claude Code is signed in to the subscription, and paste the token it prints \
                   here; it lasts a year, and is checked against Anthropic before it is kept.",
        },
    }
}

/// What each purse looks like, for the box it is pasted into.
pub const fn example(purse: PurseName) -> &'static str {
    match purse {
        PurseName::AnthropicKey => KEY_EXAMPLE,
        PurseName::AnthropicSubscription => SUBSCRIPTION_EXAMPLE,
    }
}

/// Whether the text begins as a purse of that kind does.
pub fn shaped(purse: PurseName, pasted: &str) -> Result<(), Misshapen> {
    let (prefix, expected) = match purse {
        PurseName::AnthropicKey => (KEY_PREFIX, "an Anthropic API key begins with sk-ant-api"),
        PurseName::AnthropicSubscription => (
            SUBSCRIPTION_PREFIX,
            "a Claude subscription's token, from claude setup-token, begins with sk-ant-oat",
        ),
    };
    if pasted.starts_with(prefix) {
        Ok(())
    } else {
        Err(Misshapen { purse, expected })
    }
}

/// Renders checking a key: the listing, with the key in its own header.
pub fn check_key(key: &Secret) -> Request {
    listing(KEY_HEADER, key.expose().to_owned())
}

/// Renders checking a subscription's token: the listing, with the token as
/// a bearer.
pub fn check_subscription(token: &Secret) -> Request {
    listing(TOKEN_HEADER, format!("{BEARER}{}", token.expose()))
}

/// The listing, asked with one credential in the header given.
fn listing(header: &str, credential: String) -> Request {
    Request {
        method: "GET".to_owned(),
        url: LISTING.to_owned(),
        headers: [
            (VERSION_HEADER.to_owned(), VERSION.to_owned()),
            (header.to_owned(), credential),
        ]
        .into(),
        body: None,
    }
}

/// What the provider's answer to the listing means, for a purse of either
/// kind: by the statuses measured, and by the ones its documentation gives
/// for what was not.
pub fn checked(status: u16, body: &[u8]) -> Result<(), ProviderError> {
    let provider = Provider::Anthropic;
    match status {
        200..=299 => Ok(()),
        // The provider saying no to the credential — unauthorised, or
        // forbidden — in its own sentence. A refusal that carries no
        // sentence this can read may not be the provider's at all, and is
        // reported by its status rather than given words of this project's.
        401 | 403 => Err(said(body)
            .map_or(ProviderError::Unexpected { provider, status }, |said| {
                ProviderError::Refused { provider, said }
            })),
        // A limit reached, or the provider's own trouble: a verdict on
        // nothing, so the purse was not checked rather than refused.
        429 | 500..=599 => Err(ProviderError::Unreachable {
            provider,
            why: format!("it answered {status}"),
        }),
        other => Err(ProviderError::Unexpected {
            provider,
            status: other,
        }),
    }
}

/// The provider's own sentence in a refusal, where it gave one: every error
/// of its API is one object carrying a kind and a message. Less the full
/// stop it ends with, because what this crate says is a clause and the page
/// is what ends its sentences.
fn said(body: &[u8]) -> Option<String> {
    let told: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = told.get("error")?.get("message")?.as_str()?;
    let clause = message.trim().trim_end_matches('.').trim_end();
    (!clause.is_empty()).then(|| clause.to_owned())
}

/// What a request asks, if it is one this module renders: the listing,
/// carrying one credential, which says which kind of purse it was sent as.
pub fn call(request: &Request) -> Option<Call> {
    if request.method != "GET" || request.url != LISTING {
        return None;
    }
    let key = request.headers.contains_key(KEY_HEADER);
    let bearer = request
        .headers
        .get(TOKEN_HEADER)
        .is_some_and(|sent| sent.starts_with(BEARER));
    let purse = match (key, bearer) {
        (true, false) => PurseName::AnthropicKey,
        (false, true) => PurseName::AnthropicSubscription,
        // No credential is no check, and two is nothing this renders.
        (false, false) | (true, true) => return None,
    };
    Some(Call::Check { purse })
}
