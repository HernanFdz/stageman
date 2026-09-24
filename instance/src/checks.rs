//! A credential checked against its platform before it is kept, and the
//! request held open while it is.
//!
//! A request that carries a credential — a project created, or amended
//! with a new token — is not answered in its own step. Every credential in
//! it is asked about at once, with the credential itself: the token reads
//! the repository the form names, the bot token asks who it is, the
//! app-level token asks where to connect. Each is one request the world
//! makes, rendered by the platform's or the channel's crate and read back
//! by it, and the held request is answered when the last answer lands:
//! refused with the platform's reason beside the box the credential was
//! typed in, or answered as it would have been in the first place, with
//! nothing kept from the check. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
//!
//! Held and never kept: a daemon dying mid-check answers nobody, since the
//! world drops the connection with it, and the next start knows nothing of
//! it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use stageman_channel::ChannelError;
use stageman_core::{Channel, Platform, RepositoryAddress};
use stageman_platform::PlatformError;
use stageman_vocabulary::{Bytes, Effect as Generic, EffectId, Responded};
use stageman_wire::Refusal;

use crate::requests::{Drafted, Request, Response, drafted};
use crate::views;
use crate::vocabulary::{AppEffect, RequestId};
use crate::{Effect, Running};

/// How long a platform is given to answer about a credential.
///
/// Shorter than a channel's budget, because a person is waiting at a
/// button: an answer that takes longer than this is one they would rather
/// be told about than wait for.
pub const CHECKED_WITHIN: Duration = Duration::from_secs(10);

/// A request held open while the credentials it carries are checked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Held {
    /// What was asked, answered once every check has been.
    request: Request,
    /// The checks not yet answered.
    outstanding: BTreeSet<EffectId>,
    /// The first check that failed, if one has: what the request is
    /// answered with, whatever the rest say.
    refused: Option<Refusal>,
}

/// What one check is of.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Check {
    /// A project's token on its platform, read against the repository the
    /// form names.
    Token {
        /// Which platform.
        platform: Platform,
        /// Which repository the read was of, for the refusal to name.
        repository: RepositoryAddress,
    },
    /// The credential that speaks on a channel.
    Speaking {
        /// Which channel.
        channel: Channel,
    },
    /// The credential that opens a channel's event stream.
    Listening {
        /// Which channel.
        channel: Channel,
    },
}

/// One request to check a credential with, whichever crate rendered it.
///
/// The two adapter crates render the same four fields and may not name
/// each other, so the join is here, where both are named.
struct Asking {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Option<Vec<u8>>,
}

impl From<stageman_channel::Request> for Asking {
    fn from(rendered: stageman_channel::Request) -> Self {
        Self {
            method: rendered.method,
            url: rendered.url,
            headers: rendered.headers,
            body: rendered.body,
        }
    }
}

impl From<stageman_platform::Request> for Asking {
    fn from(rendered: stageman_platform::Request) -> Self {
        Self {
            method: rendered.method,
            url: rendered.url,
            headers: rendered.headers,
            body: rendered.body,
        }
    }
}

impl Running {
    /// Holds a request whose credentials have to be checked, asking every
    /// platform concerned at once. False when the request carries nothing
    /// to check, and is to be answered now.
    ///
    /// # Errors
    ///
    /// Fails if the request is refused before any platform is asked, which
    /// is every refusal a draft earns on its own: nothing is asked about a
    /// project that would be refused anyway.
    pub fn hold_for_checks(
        &mut self,
        id: RequestId,
        request: &Request,
        effects: &mut Vec<Effect>,
    ) -> Result<bool, Refusal> {
        let checks = self.checks_for(request)?;
        if checks.is_empty() {
            return Ok(false);
        }
        let mut outstanding = BTreeSet::new();
        for (check, asking) in checks {
            let effect = self.effect_id();
            self.checks.insert(effect, (id, check));
            outstanding.insert(effect);
            // Nothing to wait on: a check changes nothing, so it goes now.
            effects.push(Generic::Request {
                id: effect,
                method: asking.method,
                url: asking.url,
                headers: asking.headers,
                body: asking.body.map(Bytes::new),
                within: CHECKED_WITHIN,
            });
        }
        self.checking.insert(
            id,
            Held {
                request: request.clone(),
                outstanding,
                refused: None,
            },
        );
        Ok(true)
    }

    /// The checks a request needs, each with the request that makes it.
    fn checks_for(&self, request: &Request) -> Result<Vec<(Check, Asking)>, Refusal> {
        let resolved = match request {
            Request::Create { draft } => drafted(draft, None)?,
            Request::Amend { project, draft } => {
                let identifier = views::identify(&self.state, project)?;
                let watched = self.state.projects.get(&identifier).ok_or_else(|| {
                    Refusal::UnknownProject {
                        id: project.clone(),
                    }
                })?;
                drafted(draft, Some(watched))?
            }
            _ => return Ok(Vec::new()),
        };
        Ok(checks_of(&resolved))
    }

    /// A platform answered about a credential. False when the answer was to
    /// no check of this instance's, and is somebody else's to read.
    pub fn checked(
        &mut self,
        id: EffectId,
        responded: &Responded,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some((request, check)) = self.checks.remove(&id) else {
            return false;
        };
        let outcome = verdict(&check, responded);
        let Some(held) = self.checking.get_mut(&request) else {
            tracing::warn!("a credential was checked for a request no longer held; ignored");
            return true;
        };
        held.outstanding.remove(&id);
        if let Err(refusal) = outcome
            && held.refused.is_none()
        {
            held.refused = Some(refusal);
        }
        if !held.outstanding.is_empty() {
            return true;
        }
        let Some(held) = self.checking.remove(&request) else {
            return true;
        };
        match held.refused {
            Some(refusal) => self.defer(AppEffect::Respond {
                id: request,
                response: Response::Refused(refusal),
            }),
            None => self.respond(request, held.request, effects),
        }
        true
    }
}

/// The checks a resolved draft needs: its token, if it gives one, on the
/// repository it names; and both credentials of every binding it gives.
fn checks_of(resolved: &Drafted) -> Vec<(Check, Asking)> {
    let mut checks = Vec::new();
    if let Some(credential) = &resolved.credential {
        let platform = Platform::GitHub;
        checks.push((
            Check::Token {
                platform,
                repository: resolved.repository.clone(),
            },
            stageman_platform::reach(platform, credential, &resolved.repository).into(),
        ));
    }
    for (channel, bound) in &resolved.channels {
        checks.push((
            Check::Speaking { channel: *channel },
            stageman_channel::who_am_i(*channel, &bound.speaking()).into(),
        ));
        checks.push((
            Check::Listening { channel: *channel },
            stageman_channel::open_socket(*channel, &bound.listen_credential).into(),
        ));
    }
    checks
}

/// What a platform's answer to one check means for the box the credential
/// was typed in.
fn verdict(check: &Check, responded: &Responded) -> Result<(), Refusal> {
    match check {
        Check::Token {
            platform,
            repository,
        } => match responded {
            Responded::Answered { status, body, .. } => {
                stageman_platform::reached(*platform, repository, *status, body.as_slice())
                    .map_err(|why| token_refusal(&why))
            }
            Responded::Failed(why) => Err(Refusal::TokenUnchecked {
                why: format!(
                    "{} could not be reached: {why}",
                    stageman_platform::shown(*platform)
                ),
            }),
        },
        Check::Speaking { channel } => spoken(*channel, false, responded, |status, body| {
            stageman_channel::identity(*channel, status, body).map(|_| ())
        }),
        Check::Listening { channel } => spoken(*channel, true, responded, |status, body| {
            stageman_channel::socket_url(*channel, status, body).map(|_| ())
        }),
    }
}

/// A token's refusal: unchecked where the platform was never asked, and
/// refused where it answered.
fn token_refusal(why: &PlatformError) -> Refusal {
    if why.unreachable() {
        Refusal::TokenUnchecked {
            why: why.to_string(),
        }
    } else {
        Refusal::TokenRefused {
            why: why.to_string(),
        }
    }
}

/// What a channel's answer to one of a binding's credentials means, read
/// by the question the credential was asked.
fn spoken(
    channel: Channel,
    listening: bool,
    responded: &Responded,
    read: impl Fn(u16, &[u8]) -> Result<(), ChannelError>,
) -> Result<(), Refusal> {
    let name = views::wire_channel(channel);
    match responded {
        Responded::Answered { status, body, .. } => {
            read(*status, body.as_slice()).map_err(|why| match why {
                ChannelError::Unreachable(what) => Refusal::ChannelUnchecked {
                    listening,
                    why: format!("{name} could not be reached: {what}"),
                },
                ChannelError::Refused(word) => Refusal::ChannelRefused {
                    listening,
                    why: format!("{name} refused it ({word})"),
                },
                ChannelError::NotABot => Refusal::ChannelRefused {
                    listening,
                    why: "it is not a bot token".to_owned(),
                },
                ChannelError::NoAnswer | ChannelError::NoIdentifier => Refusal::ChannelRefused {
                    listening,
                    why: format!("{name} accepted it and answered nothing"),
                },
                ChannelError::Unreadable(what) => Refusal::ChannelRefused {
                    listening,
                    why: format!("{name} answered something unreadable: {what}"),
                },
            })
        }
        Responded::Failed(why) => Err(Refusal::ChannelUnchecked {
            listening,
            why: format!("{name} could not be reached: {why}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{Check, verdict};
    use stageman_core::{Channel, Platform, RepositoryAddress};
    use stageman_vocabulary::{Bytes, Responded};
    use stageman_wire::Refusal;

    fn answered(status: u16, body: &str) -> Responded {
        Responded::Answered {
            status,
            headers: std::collections::BTreeMap::new(),
            body: Bytes::new(body.as_bytes().to_vec()),
        }
    }

    fn token() -> Check {
        Check::Token {
            platform: Platform::GitHub,
            repository: RepositoryAddress::parse("https://github.com/owner/name")
                .expect("an address"),
        }
    }

    /// The platform's verdict on a token lands on the token's box, as the
    /// clause the platform crate wrote; a platform never reached says so
    /// rather than refusing.
    #[test]
    fn a_tokens_verdict_is_the_platforms_clause_on_its_box() {
        assert_eq!(
            verdict(&token(), &answered(200, r#"{"full_name":"owner/name"}"#)),
            Ok(())
        );
        assert_eq!(
            verdict(&token(), &answered(401, r#"{"message":"Bad credentials"}"#)),
            Err(Refusal::TokenRefused {
                why: "GitHub does not accept it".to_owned()
            })
        );
        assert_eq!(
            verdict(&token(), &answered(503, "")),
            Err(Refusal::TokenUnchecked {
                why: "GitHub could not be reached: it answered 503".to_owned()
            })
        );
        assert_eq!(
            verdict(&token(), &Responded::Failed("dns error".to_owned())),
            Err(Refusal::TokenUnchecked {
                why: "GitHub could not be reached: dns error".to_owned()
            })
        );
    }

    /// Each of a binding's credentials is answered on its own box, with
    /// the channel's word where it gave one and the reason a user token is
    /// refused where the channel accepted one.
    #[test]
    fn a_bindings_credentials_are_answered_on_their_own_boxes() {
        let speaking = Check::Speaking {
            channel: Channel::Slack,
        };
        let listening = Check::Listening {
            channel: Channel::Slack,
        };
        assert_eq!(
            verdict(
                &speaking,
                &answered(
                    200,
                    r#"{"ok":true,"user_id":"U1","bot_id":"B1","url":"https://x.slack.com/"}"#
                )
            ),
            Ok(())
        );
        assert_eq!(
            verdict(
                &speaking,
                &answered(200, r#"{"ok":false,"error":"invalid_auth"}"#)
            ),
            Err(Refusal::ChannelRefused {
                listening: false,
                why: "Slack refused it (invalid_auth)".to_owned()
            })
        );
        assert_eq!(
            verdict(
                &speaking,
                &answered(
                    200,
                    r#"{"ok":true,"user_id":"U1","url":"https://x.slack.com/"}"#
                )
            ),
            Err(Refusal::ChannelRefused {
                listening: false,
                why: "it is not a bot token".to_owned()
            })
        );
        assert_eq!(
            verdict(
                &listening,
                &answered(200, r#"{"ok":true,"url":"wss://wss.slack.com/link/1"}"#)
            ),
            Ok(())
        );
        assert_eq!(
            verdict(
                &listening,
                &answered(200, r#"{"ok":false,"error":"not_allowed_token_type"}"#)
            ),
            Err(Refusal::ChannelRefused {
                listening: true,
                why: "Slack refused it (not_allowed_token_type)".to_owned()
            })
        );
        assert_eq!(
            verdict(&listening, &answered(200, "<html>")),
            Err(Refusal::ChannelRefused {
                listening: true,
                why: "Slack answered something unreadable: expected value at line 1 column 1"
                    .to_owned()
            })
        );
        assert_eq!(
            verdict(&listening, &answered(200, r#"{"ok":true}"#)),
            Err(Refusal::ChannelRefused {
                listening: true,
                why: "Slack accepted it and answered nothing".to_owned()
            })
        );
        assert_eq!(
            verdict(&listening, &answered(502, "")),
            Err(Refusal::ChannelUnchecked {
                listening: true,
                why: "Slack could not be reached: the channel answered 502".to_owned()
            })
        );
        assert_eq!(
            verdict(&speaking, &Responded::Failed("dns error".to_owned())),
            Err(Refusal::ChannelUnchecked {
                listening: false,
                why: "Slack could not be reached: dns error".to_owned()
            })
        );
    }
}
