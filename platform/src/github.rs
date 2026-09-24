//! GitHub: whether a token reaches a repository, and where the form that
//! mints one is.
//!
//! One read, measured on 2026-09-23 against the real platform: a token it
//! does not accept is answered `401`, a private repository the token was not
//! granted `404`, and a public repository `200` whether or not it was
//! granted — a fine-grained token reads what anybody can. The permissions
//! object in a `200` describes the account rather than the token, so nothing
//! in a read says what the token may write, and nothing here claims to. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.

use percent_encoding::utf8_percent_encode;
use stageman_core::{Platform, RepositoryAddress, Secret};

use crate::{Call, PlatformError, QUERY, Request};

/// Where the platform answers about a repository.
const REPOSITORIES: &str = "https://api.github.com/repos/";

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
