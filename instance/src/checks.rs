//! A credential checked against its platform before it is kept, and the
//! request held open while it is.
//!
//! A request that carries a credential — a project created, or amended
//! with its access set, or with its repository moved under the access it
//! holds — is not answered in its own step. Everything in it is asked
//! about at once, with the credential itself: a token reads the repository
//! the form chose, an installation mints a token restricted to it, which
//! the platform refuses where the installation does not cover it, the bot
//! token asks who it is, the app-level token asks where to connect. Each
//! is one request the world makes, rendered by the platform's or the
//! channel's crate and read back by it, and the held request is answered
//! when the last answer lands: refused with the platform's reason beside
//! the box it concerns, or answered as it would have been in the first
//! place, with nothing kept from the check. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`
//! and `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
//!
//! Held and never kept: a daemon dying mid-check answers nobody, since the
//! world drops the connection with it, and the next start knows nothing of
//! it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use stageman_channel::ChannelError;
use stageman_core::{Access, Channel, Platform, RepositoryAddress, Secret};
use stageman_platform::{Owned, PlatformError};
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
    /// What the platform said of the token, once it has: kept beside the
    /// token when the request is answered — see
    /// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    learned: Option<Owned>,
}

/// What one check is of.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Check {
    /// A project's token on its platform, read against the repository the
    /// form chose.
    Token {
        /// Which platform.
        platform: Platform,
        /// Which repository the read was of, for the refusal to name.
        repository: RepositoryAddress,
    },
    /// A project's token asked whose it is, which is also the answer that
    /// carries when it expires: what is kept beside the token.
    Owner {
        /// Which platform.
        platform: Platform,
    },
    /// An installation of the App, asked to mint a token restricted to the
    /// repository the form chose: refused by the platform where the
    /// installation does not cover it, and the token discarded either way.
    Covered {
        /// Which platform.
        platform: Platform,
        /// Which installation.
        installation: u64,
        /// Which repository, for the refusal to name.
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
                learned: None,
            },
        );
        Ok(true)
    }

    /// The checks a request needs, each with the request that makes it.
    fn checks_for(&self, request: &Request) -> Result<Vec<(Check, Asking)>, Refusal> {
        let resolved = match request {
            Request::Create { draft } => drafted(draft, None, &self.begun)?,
            Request::Amend { project, draft } => {
                let identifier = views::identify(&self.state, project)?;
                let watched = self.state.projects.get(&identifier).ok_or_else(|| {
                    Refusal::UnknownProject {
                        id: project.clone(),
                    }
                })?;
                drafted(draft, Some(watched), &self.begun)?
            }
            // The app-level token of an app the instance owns, asked where
            // to connect, as a binding's is; the client pair cannot be
            // checked — the exchange refuses a bogus code before it looks at
            // the pair, measured — and is kept unchecked, per
            // `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
            Request::RegisterChannelApp {
                channel,
                client_id,
                client_secret,
                app_token,
            } => {
                let channel = views::channel_named(channel)?;
                let app_token = app_token.trim();
                if client_id.trim().is_empty()
                    || client_secret.trim().is_empty()
                    || app_token.is_empty()
                {
                    return Err(Refusal::ChannelAppIncomplete);
                }
                let opening = Secret::new(app_token.to_owned());
                return Ok(vec![(
                    Check::Listening { channel },
                    stageman_channel::open_socket(channel, &opening).into(),
                )]);
            }
            _ => return Ok(Vec::new()),
        };
        let platform = Platform::GitHub;
        let mut checks = Vec::new();
        // The access is checked against the repository where either is not
        // what the project holds; what it holds was checked when kept.
        match (&resolved.access, resolved.reach_changed) {
            (_, false) => {}
            (Access::Token { secret, .. }, true) => {
                checks.push((
                    Check::Token {
                        platform,
                        repository: resolved.repository.clone(),
                    },
                    stageman_platform::reach(platform, secret, &resolved.repository).into(),
                ));
                // Whose it is and when it expires, read whenever the token
                // is checked: two facts about the secret, which cannot
                // change under it.
                checks.push((
                    Check::Owner { platform },
                    stageman_platform::owner(platform, secret).into(),
                ));
            }
            (Access::Installation { id }, true) => {
                let id = *id;
                let app = self
                    .state
                    .apps
                    .get(&platform)
                    .filter(|app| app.installations.contains_key(&id))
                    .ok_or(Refusal::NoSuchInstallation { id })?;
                let rendered = stageman_platform::mint(
                    platform,
                    app,
                    id,
                    Some(&resolved.repository),
                    self.stamp(),
                )
                .map_err(|why| Refusal::InstallationRefused {
                    why: why.to_string(),
                })?;
                checks.push((
                    Check::Covered {
                        platform,
                        installation: id,
                        repository: resolved.repository.clone(),
                    },
                    rendered.into(),
                ));
            }
        }
        checks.extend(channel_checks(&resolved));
        Ok(checks)
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
        match outcome {
            Ok(Some(learned)) => held.learned = Some(learned),
            Ok(None) => {}
            Err(refusal) => {
                if held.refused.is_none() {
                    held.refused = Some(refusal);
                }
            }
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
            None => self.respond(request, held.request, held.learned, effects),
        }
        true
    }
}

/// The checks a resolved draft's bindings need: both credentials of every
/// binding it gives.
fn channel_checks(resolved: &Drafted) -> Vec<(Check, Asking)> {
    let mut checks = Vec::new();
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
/// was typed in, and what the one check that learns something learned.
fn verdict(check: &Check, responded: &Responded) -> Result<Option<Owned>, Refusal> {
    match check {
        Check::Owner { platform } => match responded {
            Responded::Answered {
                status,
                headers,
                body,
            } => stageman_platform::owned(*platform, *status, headers, body.as_slice())
                .map(Some)
                .map_err(|why| owner_refusal(&why)),
            Responded::Failed(why) => Err(Refusal::TokenUnchecked {
                why: format!(
                    "{} could not be reached: {why}",
                    stageman_platform::shown(*platform)
                ),
            }),
        },
        Check::Token {
            platform,
            repository,
        } => match responded {
            Responded::Answered { status, body, .. } => {
                stageman_platform::reached(*platform, repository, *status, body.as_slice())
                    .map(|()| None)
                    .map_err(|why| token_refusal(&why, repository))
            }
            Responded::Failed(why) => Err(Refusal::TokenUnchecked {
                why: format!(
                    "{} could not be reached: {why}",
                    stageman_platform::shown(*platform)
                ),
            }),
        },
        Check::Covered {
            platform,
            repository,
            ..
        } => match responded {
            Responded::Answered { status, body, .. } => {
                stageman_platform::minted(*platform, *status, body.as_slice())
                    .map(|_| None)
                    .map_err(|why| covered_refusal(&why, repository))
            }
            Responded::Failed(why) => Err(Refusal::InstallationUnchecked {
                why: format!(
                    "{} could not be reached: {why}",
                    stageman_platform::shown(*platform)
                ),
            }),
        },
        Check::Speaking { channel } => spoken(*channel, false, responded, |status, body| {
            stageman_channel::identity(*channel, status, body).map(|_| ())
        })
        .map(|()| None),
        Check::Listening { channel } => spoken(*channel, true, responded, |status, body| {
            stageman_channel::socket_url(*channel, status, body).map(|_| ())
        })
        .map(|()| None),
    }
}

/// The owner read's refusal: unchecked where the platform was never asked
/// or could not be, and on the token where it answered anything else.
fn owner_refusal(why: &PlatformError) -> Refusal {
    match why {
        PlatformError::Unreachable { .. } => Refusal::TokenUnchecked {
            why: why.to_string(),
        },
        _ => Refusal::TokenRefused {
            why: why.to_string(),
        },
    }
}

/// A token's refusal: on the repository where the platform cannot see it
/// with the token, unchecked where the platform was never asked, and on
/// the token where it answered anything else.
fn token_refusal(why: &PlatformError, repository: &RepositoryAddress) -> Refusal {
    match why {
        PlatformError::NotGranted { platform, .. } => Refusal::NotReached {
            repository: views::wire_repository(repository),
            why: format!(
                "{} cannot see it with the token — a fine-grained token has to be granted that \
                 repository",
                stageman_platform::shown(*platform)
            ),
        },
        PlatformError::Unreachable { .. } => Refusal::TokenUnchecked {
            why: why.to_string(),
        },
        _ => Refusal::TokenRefused {
            why: why.to_string(),
        },
    }
}

/// An installation's refusal: on the repository where the platform would
/// not mint for it, which is what it answers when the installation does
/// not cover it; unchecked where the platform was never asked; and on the
/// installation where the platform refused the App's key or knows no such
/// installation.
fn covered_refusal(why: &PlatformError, repository: &RepositoryAddress) -> Refusal {
    match why {
        PlatformError::Forbidden { .. } => Refusal::NotReached {
            repository: views::wire_repository(repository),
            why: why.to_string(),
        },
        PlatformError::Unreachable { .. } => Refusal::InstallationUnchecked {
            why: why.to_string(),
        },
        _ => Refusal::InstallationRefused {
            why: why.to_string(),
        },
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
            Ok(None)
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
        assert_eq!(
            verdict(&token(), &answered(404, r#"{"message":"Not Found"}"#)),
            Err(Refusal::NotReached {
                repository: stageman_wire::Repository {
                    owner: "owner".to_owned(),
                    name: "name".to_owned()
                },
                why: "GitHub cannot see it with the token — a fine-grained token has to be \
                      granted that repository"
                    .to_owned()
            }),
            "a repository the token cannot see is the repository's refusal"
        );
    }

    /// An installation's verdict is read off the restricted mint: minted
    /// is covered, unprocessable is the repository's refusal with the
    /// platform's words, an unknown installation is the installation's,
    /// and a platform never reached says so.
    #[test]
    fn an_installations_verdict_is_read_off_the_restricted_mint() {
        let covered = Check::Covered {
            platform: Platform::GitHub,
            installation: 77,
            repository: RepositoryAddress::parse("https://github.com/owner/name")
                .expect("an address"),
        };
        assert_eq!(
            verdict(
                &covered,
                &answered(
                    201,
                    r#"{"token":"ghs_not_a_real_token","expires_at":"2026-09-24T08:00:00Z"}"#
                )
            ),
            Ok(None)
        );
        assert_eq!(
            verdict(
                &covered,
                &answered(
                    422,
                    r#"{"message":"There is at least one repository that does not exist or is not accessible to the parent installation."}"#
                )
            ),
            Err(Refusal::NotReached {
                repository: stageman_wire::Repository {
                    owner: "owner".to_owned(),
                    name: "name".to_owned()
                },
                why: "GitHub refused: There is at least one repository that does not exist or \
                      is not accessible to the parent installation."
                    .to_owned()
            })
        );
        assert_eq!(
            verdict(&covered, &answered(404, r#"{"message":"Not Found"}"#)),
            Err(Refusal::InstallationRefused {
                why: "GitHub knows no installation with that identifier for this App".to_owned()
            })
        );
        assert_eq!(
            verdict(&covered, &Responded::Failed("dns error".to_owned())),
            Err(Refusal::InstallationUnchecked {
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
            Ok(None)
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
            Ok(None)
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
