//! GitHub: whether a token reaches a repository, what a token can read,
//! and where the form that mints one is.
//!
//! One read, measured on 2026-09-23 against the real platform: a token it
//! does not accept is answered `401`, a private repository the token was not
//! granted `404`, and a public repository `200` whether or not it was
//! granted — a fine-grained token reads what anybody can. The permissions
//! object in a `200` describes the account rather than the token, so nothing
//! in a read says what the token may write, and nothing here claims to. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
//!
//! One listing, measured on 2026-09-24: what a fine-grained token can read
//! is every public repository the operator owns or belongs to an
//! organisation for, and the private ones the token was granted — so a
//! private entry is certainly granted and a public one may not be, which
//! is what the listing crosses with each entry's visibility for. See
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.

use percent_encoding::utf8_percent_encode;
use stageman_core::{Platform, RepositoryAddress, Secret, Timestamp};

use crate::{Call, PlatformError, QUERY, Request};

/// Where the platform answers about a repository.
const REPOSITORIES: &str = "https://api.github.com/repos/";

/// Where the platform lists what a token can read.
const READABLE: &str = "https://api.github.com/user/repos";

/// Where the platform says whose a token is.
const USER: &str = "https://api.github.com/user";

/// The header the platform answers every request with when the token it
/// was made with expires, carrying when — see
/// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
const EXPIRATION: &str = "github-authentication-token-expiration";

/// How many repositories one listing asks for: the most the platform gives
/// on a page.
const PER_PAGE: u32 = 100;

/// Where a fine-grained token is minted. Its form takes from the address a
/// name and each permission by name, per the platform's documentation;
/// which repositories the token may see cannot be given, and the expiry is
/// left to the form on purpose.
const TOKEN_FORM: &str = "https://github.com/settings/personal-access-tokens/new";

/// The most a token's name may be, in characters, per the platform's
/// documentation.
const NAME_AT_MOST: usize = 40;

/// What the platform is told this client is, which it requires of every
/// client, and what a token is named when no project names it.
const STAGEMAN: &str = "stageman";

/// Renders asking whether a token reaches a repository: a read of the
/// repository with the token, and nothing that changes anything there.
pub fn reach(credential: &Secret, repository: &RepositoryAddress) -> Request {
    Request {
        method: "GET".to_owned(),
        url: format!("{REPOSITORIES}{}/{}", repository.owner, repository.name),
        headers: [
            (
                "accept".to_owned(),
                "application/vnd.github+json".to_owned(),
            ),
            (
                "authorization".to_owned(),
                format!("Bearer {}", credential.expose()),
            ),
            ("user-agent".to_owned(), STAGEMAN.to_owned()),
            ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
        ]
        .into(),
        body: None,
    }
}

/// What the platform's answer to [`reach`] means, status by status as
/// measured.
pub fn reached(
    repository: &RepositoryAddress,
    status: u16,
    body: &[u8],
) -> Result<(), PlatformError> {
    let platform = Platform::GitHub;
    match status {
        200..=299 => Ok(()),
        401 => Err(PlatformError::Refused { platform }),
        404 => Err(PlatformError::NotGranted {
            platform,
            repository: format!("{}/{}", repository.owner, repository.name),
        }),
        403 | 429 => Err(PlatformError::Forbidden {
            platform,
            why: message(body),
        }),
        // The platform's own trouble rather than a verdict on the token.
        500..=599 => Err(PlatformError::Unreachable {
            platform,
            why: format!("it answered {status}"),
        }),
        other => Err(PlatformError::Unexpected {
            platform,
            status: other,
        }),
    }
}

/// Renders listing what a token can read: the first page, which is the
/// most the platform gives.
pub fn readable(credential: &Secret) -> Request {
    Request {
        method: "GET".to_owned(),
        url: format!("{READABLE}?per_page={PER_PAGE}"),
        headers: [
            (
                "accept".to_owned(),
                "application/vnd.github+json".to_owned(),
            ),
            (
                "authorization".to_owned(),
                format!("Bearer {}", credential.expose()),
            ),
            ("user-agent".to_owned(), STAGEMAN.to_owned()),
            ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
        ]
        .into(),
        body: None,
    }
}

/// Renders asking whose a token is: the account it was made under, which
/// is also the request whose answer carries when the token expires.
pub fn owner(credential: &Secret) -> Request {
    Request {
        method: "GET".to_owned(),
        url: USER.to_owned(),
        headers: [
            (
                "accept".to_owned(),
                "application/vnd.github+json".to_owned(),
            ),
            (
                "authorization".to_owned(),
                format!("Bearer {}", credential.expose()),
            ),
            ("user-agent".to_owned(), STAGEMAN.to_owned()),
            ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
        ]
        .into(),
        body: None,
    }
}

/// What the platform said of a token: whose it is, and when it expires
/// where it does.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Owned {
    /// The account the token was made under, as the platform spells it.
    pub login: String,
    /// When the platform will stop accepting it, where the platform said:
    /// the header is absent for a token that does not expire.
    pub expires: Option<Timestamp>,
}

/// What the platform's answer to [`owner`] means: the account, and the
/// expiry read off the header the platform sends beside every answer to a
/// token that expires, spelled `2026-09-27 08:42:20 UTC` as measured.
///
/// # Errors
///
/// Fails if the platform does not accept the token, refused to say, could
/// not be reached, answered with something this does not read as an
/// account, or sent an expiry this does not read as a moment — the last
/// refused rather than dropped, since a fact this instance acts on has to
/// be the platform's word or absent.
pub fn owned(
    status: u16,
    headers: &std::collections::BTreeMap<String, String>,
    body: &[u8],
) -> Result<Owned, PlatformError> {
    let platform = Platform::GitHub;
    match status {
        200..=299 => {}
        401 => return Err(PlatformError::Refused { platform }),
        403 | 429 => {
            return Err(PlatformError::Forbidden {
                platform,
                why: message(body),
            });
        }
        500..=599 => {
            return Err(PlatformError::Unreachable {
                platform,
                why: format!("it answered {status}"),
            });
        }
        other => {
            return Err(PlatformError::Unexpected {
                platform,
                status: other,
            });
        }
    }
    let told: serde_json::Value =
        serde_json::from_slice(body).map_err(|failure| PlatformError::Unreadable {
            platform,
            why: failure.to_string(),
        })?;
    let login = told
        .get("login")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PlatformError::Unreadable {
            platform,
            why: "the account came without its login".to_owned(),
        })?;
    let expires = headers
        .get(EXPIRATION)
        .map(|spelled| expiry(spelled))
        .transpose()?;
    Ok(Owned { login, expires })
}

/// The expiry header's moment: `YYYY-MM-DD HH:MM:SS UTC`, as the platform
/// spells it, read as the moment it names.
fn expiry(spelled: &str) -> Result<Timestamp, PlatformError> {
    let unreadable = || PlatformError::Unreadable {
        platform: Platform::GitHub,
        why: format!("the token's expiry could not be read: {spelled:?}"),
    };
    let (date, clock) = spelled
        .trim()
        .strip_suffix(" UTC")
        .and_then(|rest| rest.split_once(' '))
        .ok_or_else(unreadable)?;
    format!("{date}T{clock}Z")
        .parse::<Timestamp>()
        .map_err(|_| unreadable())
}

/// Repositories listed by the platform, as far as one page says, each
/// with its visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The repositories listed, as addresses, with whether each is private.
    pub repositories: Vec<Repository>,
    /// Whether there were more than were listed.
    pub more: bool,
}

/// One repository as a listing names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Its address.
    pub address: RepositoryAddress,
    /// Whether it is private: for a token's listing, whether the token was
    /// certainly granted it.
    pub private: bool,
}

/// The repositories in a listing's items, skipping what is not an address
/// on the platform.
pub fn repositories_of(items: &[serde_json::Value]) -> Vec<Repository> {
    items
        .iter()
        .filter_map(|item| {
            let address = item
                .get("html_url")
                .and_then(serde_json::Value::as_str)
                .and_then(|url| RepositoryAddress::parse(url).ok())?;
            Some(Repository {
                address,
                private: item
                    .get("private")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// What the platform's answer to [`readable`] means, status by status as
/// measured.
///
/// # Errors
///
/// Fails if the platform does not accept the token, refused to say, could
/// not be reached, or answered with something this does not read as a
/// listing.
pub fn readable_listed(status: u16, body: &[u8]) -> Result<Listing, PlatformError> {
    let platform = Platform::GitHub;
    match status {
        200..=299 => {}
        401 => return Err(PlatformError::Refused { platform }),
        403 | 429 => {
            return Err(PlatformError::Forbidden {
                platform,
                why: message(body),
            });
        }
        500..=599 => {
            return Err(PlatformError::Unreachable {
                platform,
                why: format!("it answered {status}"),
            });
        }
        other => {
            return Err(PlatformError::Unexpected {
                platform,
                status: other,
            });
        }
    }
    let told: serde_json::Value =
        serde_json::from_slice(body).map_err(|failure| PlatformError::Unreadable {
            platform,
            why: failure.to_string(),
        })?;
    let items = told.as_array().ok_or_else(|| PlatformError::Unreadable {
        platform,
        why: "the listing is not a list".to_owned(),
    })?;
    // One page is asked for; a full page means the platform had more.
    let full = u32::try_from(items.len()).map_err(|_| PlatformError::Unreadable {
        platform,
        why: "the listing is longer than can be counted".to_owned(),
    })?;
    Ok(Listing {
        repositories: repositories_of(items),
        more: full >= PER_PAGE,
    })
}

/// What the platform said, when it said anything: the message every
/// refusal of its carries, or that it carried none.
pub fn message(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|told| {
            told.get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "no reason given".to_owned())
}

/// Where the form that mints a token is, filled in: named for the project
/// where one is named, cut to what the form takes, and given the three
/// permissions a job needs — contents, issues and pull requests, write.
pub fn token_form(project: Option<&str>) -> String {
    let name: String = project
        .map_or_else(
            || STAGEMAN.to_owned(),
            |project| format!("{STAGEMAN}: {project}"),
        )
        .chars()
        .take(NAME_AT_MOST)
        .collect();
    format!(
        "{TOKEN_FORM}?name={}&contents=write&issues=write&pull_requests=write",
        utf8_percent_encode(&name, QUERY)
    )
}

/// What a request asks, if it is one this module renders.
pub fn call(request: &Request) -> Option<Call> {
    if request.method != "GET" {
        return None;
    }
    if request.url.starts_with(READABLE) {
        return Some(Call::Readable {
            platform: Platform::GitHub,
        });
    }
    if request.url == USER {
        return Some(Call::Owner {
            platform: Platform::GitHub,
        });
    }
    let rest = request.url.strip_prefix(REPOSITORIES)?;
    let (owner, name) = rest.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(Call::Repository {
        platform: Platform::GitHub,
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}
