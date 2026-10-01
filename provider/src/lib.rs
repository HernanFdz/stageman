//! The contract every provider is asked on by this daemon, and the adapters
//! that implement it.
//!
//! What a purse is checked with, what the answer means, the shape a
//! credential of each kind has, and where each is minted — as pure
//! functions the **instance** renders and reads and the world carries,
//! dispatched on the provider at this surface so that nothing outside names
//! one. See `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`.
//!
//! Nothing here is performed: the one question asked of a provider — whether
//! it accepts a purse, asked once before the purse is kept — is rendered as
//! a request the world makes, and the provider's answer is read back from
//! what the world carried, on the terms
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
//! sets for a platform's credential.
//!
//! What an agent reads a purse *from* is deliberately not here: the
//! variable is the agent's, per `docs/conventions.md` §3, and lives in the
//! agent crate. This crate knows what a credential looks like, where it
//! comes from and who vouches for it, which is the provider's.

use std::collections::BTreeMap;

use stageman_core::{Provider, Purse, PurseName};

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

/// One request to a provider, as the instance asks the world to make it.
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

/// Renders asking a provider whether it accepts a purse: one read, with the
/// credential in the header its kind travels in, that costs nothing and
/// changes nothing there.
///
/// Dispatched on the purse itself rather than on its provider, so that each
/// provider's module is handed the credential of a purse it hands out and
/// has no other to refuse.
#[must_use]
pub fn check(purse: &Purse) -> Request {
    match purse {
        Purse::AnthropicKey(key) => anthropic::check_key(key),
        Purse::AnthropicSubscription(token) => anthropic::check_subscription(token),
    }
}

/// What the provider's answer to [`check`] means.
///
/// Nothing is read from an answer that accepts: the check is whether the
/// provider answered for the credential, and nothing of it is kept.
///
/// # Errors
///
/// Fails if the provider does not accept the purse, was limiting requests
/// or failing, or answered with something this does not read as any of
/// those.
pub fn checked(purse: PurseName, status: u16, body: &[u8]) -> Result<(), ProviderError> {
    match purse.provider() {
        Provider::Anthropic => anthropic::checked(status, body),
    }
}

/// What a request asks of a provider, read back from what would be sent.
///
/// The inverse of what this crate renders, for a simulated provider to
/// recognise what it is asked and answer as the real one was measured to,
/// without matching on strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// A purse checked, with its credential in the header its kind travels
    /// in.
    Check {
        /// Which purse the credential was sent as.
        purse: PurseName,
    },
}

impl Call {
    /// What a request asks, if it is one this crate renders.
    #[must_use]
    pub fn parse(request: &Request) -> Option<Self> {
        anthropic::call(request)
    }
}

/// A provider would not have a purse, or could not be asked.
///
/// Every message names the provider and reads as the clause after *was not
/// kept:*, which is how a purse's row says it. A clause and never a
/// sentence: none ends in a full stop, the provider's own words included,
/// because the page is what ends its sentences.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// The request never got an answer, or got one that says nothing about
    /// the credential: a limit reached, or the provider's own failure.
    #[error("{} could not be reached: {why}", .provider.name())]
    Unreachable {
        /// Which provider.
        provider: Provider,
        /// What went wrong.
        why: String,
    },
    /// It answered, and does not accept the credential.
    #[error("{} does not accept it: {said}", .provider.name())]
    Refused {
        /// Which provider.
        provider: Provider,
        /// The provider's own sentence, less its full stop: never this
        /// project's guess at one, so an answer that refuses and gives no
        /// sentence is [`ProviderError::Unexpected`] instead.
        said: String,
    },
    /// It answered with a status this does not read as any of the above, or
    /// refused without a sentence this could read.
    #[error("{} answered {status}", .provider.name())]
    Unexpected {
        /// Which provider.
        provider: Provider,
        /// What it answered.
        status: u16,
    },
}

impl ProviderError {
    /// Whether the provider said nothing about the credential, as opposed to
    /// having refused it: what tells *unchecked* from *refused* for whoever
    /// pasted it.
    #[must_use]
    pub const fn unreachable(&self) -> bool {
        matches!(self, Self::Unreachable { .. })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Call, ProviderError, Request, check, checked, example, guide, shaped};
    use stageman_core::{Provider, Purse, PurseName, Secret};

    /// Where the provider lists its models, as the check asks for it.
    const LISTING: &str = "https://api.anthropic.com/v1/models?limit=1";

    /// What the provider answered a made-up key in the key's header, as
    /// measured on 2026-10-01.
    const KEY_REFUSED: &str = r#"{"type":"error","error":{"type":"authentication_error","message":"API key is invalid."},"request_id":null}"#;

    /// What it answered a made-up subscription token sent as a bearer, the
    /// same day.
    const TOKEN_REFUSED: &str = r#"{"type":"error","error":{"type":"authentication_error","message":"OAuth access token is invalid."},"request_id":null}"#;

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn key() -> Purse {
        Purse::AnthropicKey(Secret::new("sk-ant-api03-not-a-real-key".to_owned()))
    }

    fn subscription() -> Purse {
        Purse::AnthropicSubscription(Secret::new("sk-ant-oat01-not-a-real-token".to_owned()))
    }

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
    /// own site over a secure scheme, and the words beside it say something
    /// — that the purse is checked before it is kept among them, since that
    /// is what a person pasting one is about to wait on.
    #[test]
    fn every_purse_is_guided_to_its_providers_own_page() {
        for purse in PurseName::ALL {
            let guide = guide(*purse);
            assert!(guide.link.starts_with("https://"), "{purse:?}: {guide:?}");
            assert!(!guide.label.is_empty(), "{guide:?}");
            let checked = format!(
                "checked against {} before it is kept",
                purse.provider().name()
            );
            assert!(guide.says.contains(&checked), "{guide:?}");
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

    /// A purse is checked by one read of the provider's listing, carrying
    /// the version the provider requires and the credential in the header
    /// its kind travels in, and nothing else: asserted whole, since it is
    /// what a credential is sent with, and read back as what it asked.
    #[test]
    fn a_purse_is_checked_by_one_read_with_its_credential_in_its_kinds_header() {
        let asking = check(&key());
        assert_eq!(asking.method, "GET");
        assert_eq!(asking.url, LISTING);
        assert_eq!(
            asking.headers,
            headers(&[
                ("anthropic-version", "2023-06-01"),
                ("x-api-key", "sk-ant-api03-not-a-real-key"),
            ])
        );
        assert!(asking.body.is_none());
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Check {
                purse: PurseName::AnthropicKey
            })
        );

        let asking = check(&subscription());
        assert_eq!(asking.method, "GET");
        assert_eq!(asking.url, LISTING);
        assert_eq!(
            asking.headers,
            headers(&[
                ("anthropic-version", "2023-06-01"),
                ("authorization", "Bearer sk-ant-oat01-not-a-real-token"),
            ])
        );
        assert!(asking.body.is_none());
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Check {
                purse: PurseName::AnthropicSubscription
            })
        );
    }

    /// Only what this crate renders reads back as a check: another method,
    /// another address, a request carrying no credential, one carrying both,
    /// and an authorisation that is not a bearer are each no call at all.
    #[test]
    fn only_a_listing_carrying_one_credential_reads_back_as_a_check() {
        let asked = |method: &str, url: &str, pairs: &[(&str, &str)]| {
            Call::parse(&Request {
                method: method.to_owned(),
                url: url.to_owned(),
                headers: headers(pairs),
                body: None,
            })
        };
        let key = ("x-api-key", "sk-ant-api03-not-a-real-key");
        let bearer = ("authorization", "Bearer sk-ant-oat01-not-a-real-token");

        assert_eq!(
            asked("GET", LISTING, &[key]),
            Some(Call::Check {
                purse: PurseName::AnthropicKey
            })
        );
        assert_eq!(asked("POST", LISTING, &[key]), None, "another method");
        assert_eq!(
            asked("GET", "https://api.anthropic.com/v1/messages", &[key]),
            None,
            "another address"
        );
        assert_eq!(asked("GET", LISTING, &[]), None, "no credential");
        assert_eq!(asked("GET", LISTING, &[key, bearer]), None, "both");
        assert_eq!(
            asked("GET", LISTING, &[("authorization", "Basic c2stYW50")]),
            None,
            "an authorisation that is not a bearer"
        );
    }

    /// The answer is read as the provider was measured to answer, for either
    /// kind of purse: accepted, and nothing of the listing read; refused,
    /// with the provider's own sentence less its full stop.
    #[test]
    fn the_answer_is_read_as_the_provider_was_measured_to_answer() {
        let provider = Provider::Anthropic;
        for purse in PurseName::ALL {
            assert_eq!(
                checked(*purse, 200, b"a listing, of which nothing is read"),
                Ok(()),
                "{purse:?}"
            );
        }
        assert_eq!(
            checked(PurseName::AnthropicKey, 401, KEY_REFUSED.as_bytes()),
            Err(ProviderError::Refused {
                provider,
                said: "API key is invalid".to_owned(),
            })
        );
        assert_eq!(
            checked(
                PurseName::AnthropicSubscription,
                401,
                TOKEN_REFUSED.as_bytes()
            ),
            Err(ProviderError::Refused {
                provider,
                said: "OAuth access token is invalid".to_owned(),
            })
        );
    }

    /// What was not measured is read by the statuses the provider documents.
    /// Forbidden is a refusal, in the sentence its one error object carries;
    /// a limit reached and the provider's own failure say nothing about the
    /// credential; a refusal with no sentence this can read is reported by
    /// its status rather than given words of this project's; and so is
    /// anything else.
    #[test]
    fn what_the_provider_documents_is_read_by_its_status() {
        let provider = Provider::Anthropic;
        let read =
            |status: u16, body: &str| checked(PurseName::AnthropicKey, status, body.as_bytes());

        assert_eq!(
            read(
                403,
                r#"{"type":"error","error":{"type":"permission_error","message":"Your API key does not have permission to use the specified resource."}}"#
            ),
            Err(ProviderError::Refused {
                provider,
                said: "Your API key does not have permission to use the specified resource"
                    .to_owned(),
            })
        );
        for body in [
            "<html>not the provider's</html>",
            r#"{"type":"error"}"#,
            r#"{"type":"error","error":{"type":"authentication_error"}}"#,
            r#"{"type":"error","error":{"type":"authentication_error","message":""}}"#,
            r#"{"type":"error","error":{"type":"authentication_error","message":" . "}}"#,
        ] {
            assert_eq!(
                read(401, body),
                Err(ProviderError::Unexpected {
                    provider,
                    status: 401,
                }),
                "{body}"
            );
        }
        for status in [429, 500, 529, 599] {
            let failed = read(status, KEY_REFUSED).expect_err("no verdict");
            assert_eq!(
                failed,
                ProviderError::Unreachable {
                    provider,
                    why: format!("it answered {status}"),
                },
                "whatever the body says"
            );
            assert!(failed.unreachable());
        }
        assert_eq!(read(299, ""), Ok(()), "any success is an acceptance");
        for status in [199, 300, 400, 404, 418, 499, 600] {
            let failed = read(status, KEY_REFUSED).expect_err("not an acceptance");
            assert_eq!(
                failed,
                ProviderError::Unexpected { provider, status },
                "whatever the body says"
            );
            assert!(!failed.unreachable());
        }
        assert!(
            !read(401, KEY_REFUSED).expect_err("refused").unreachable(),
            "a refusal is a verdict"
        );
    }

    /// The words a purse's row shows, asserted whole per
    /// `docs/conventions.md` §4: each a clause, and none ending in a full
    /// stop.
    #[test]
    fn every_failure_reads_as_the_clause_after_not_kept() {
        let provider = Provider::Anthropic;
        assert_eq!(
            ProviderError::Refused {
                provider,
                said: "API key is invalid".to_owned(),
            }
            .to_string(),
            "Anthropic does not accept it: API key is invalid"
        );
        assert_eq!(
            ProviderError::Unreachable {
                provider,
                why: "dns error".to_owned(),
            }
            .to_string(),
            "Anthropic could not be reached: dns error"
        );
        assert_eq!(
            ProviderError::Unexpected {
                provider,
                status: 418,
            }
            .to_string(),
            "Anthropic answered 418"
        );
    }
}
