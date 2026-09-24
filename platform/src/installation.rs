//! GitHub: an installation of the App this instance owns, and what the
//! platform answers about it — see
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
//!
//! Three requests, authenticated one of two ways. The installation is
//! fetched, and a token minted from it, with the App's own key: a JSON Web
//! Token this module signs, issued a minute in the past against clock
//! drift, expiring within ten minutes, issued by the client identifier. An
//! installation that is not this App's cannot be fetched with it, which is
//! the check the platform's documentation asks for, since the identifier
//! the redirect carries is not to be trusted alone. The state the install
//! link carries says nothing about the installation and everything about
//! who asked: it names the page the tab was opened from, per
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`. A minted
//! token lasts an hour and is restricted at minting to named repositories
//! and named permissions. The installation's repositories are listed with
//! such a token. Everything here is what the platform documents, checked
//! rather than remembered.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64_URL};
use stageman_core::{Platform, PlatformApp, RepositoryAddress, Secret, Timestamp};

use crate::github::{Listing, repositories_of};
use crate::{Call, PlatformError, Request};

/// Where the platform answers about an App's installations, with the App's
/// key.
const INSTALLATIONS: &str = "https://api.github.com/app/installations/";

/// Where an installation's repositories are listed, with a token minted
/// from it.
const REPOSITORIES: &str = "https://api.github.com/installation/repositories";

/// Where a person installs an App: its page on the platform.
const APPS: &str = "https://github.com/apps/";

/// What the platform is told this client is.
const STAGEMAN: &str = "stageman";

/// How many repositories one listing asks for: the most the platform gives
/// on a page.
const PER_PAGE: u32 = 100;

/// How far in the past the token is issued, against a clock a little ahead
/// of the platform's, and how long it is good for, both as the platform's
/// documentation recommends.
const ISSUED_BEHIND: i64 = 60;
const GOOD_FOR: i64 = 600;

/// The permissions a token is minted with: exactly what the App was
/// registered with, so that a token never has more than the App does and a
/// project's jobs have what every job needs.
const PERMISSIONS: &str =
    r#"{"contents":"write","issues":"write","pull_requests":"write","metadata":"read"}"#;

/// Where a person installs the App, carrying the state the platform
/// brings back to the setup address with the installation: what names the
/// page the tab was opened from, and nothing about the installation, which
/// the App's key confirms whatever the state says.
#[must_use]
pub fn install_link(slug: &str, state: &str) -> String {
    format!("{APPS}{slug}/installations/new?state={state}")
}

/// The token an App's requests are authenticated with, signed with its
/// key: RS256 over the claims the platform documents, at `now`.
///
/// # Errors
///
/// Fails if the key is not the PEM the platform hands out, or if the clock
/// is somewhere a claim cannot say.
fn jwt(app: &PlatformApp, now: Timestamp) -> Result<String, PlatformError> {
    // Structs rather than literals, so the fields come out in this order
    // whatever a map does with its keys, and a token reads the same twice.
    #[derive(serde::Serialize)]
    struct Header {
        alg: &'static str,
        typ: &'static str,
    }
    #[derive(serde::Serialize)]
    struct Claims<'a> {
        iat: i64,
        exp: i64,
        iss: &'a str,
    }
    let platform = Platform::GitHub;
    let key = |why: String| PlatformError::Key { platform, why };
    let issued = now
        .as_second()
        .checked_sub(ISSUED_BEHIND)
        .ok_or_else(|| key("the clock is before anything a claim can say".to_owned()))?;
    let expires = now
        .as_second()
        .checked_add(GOOD_FOR)
        .ok_or_else(|| key("the clock is beyond anything a claim can say".to_owned()))?;
    let header = serde_json::to_vec(&Header {
        alg: "RS256",
        typ: "JWT",
    })
    .map_err(|why| key(why.to_string()))?;
    let claims = serde_json::to_vec(&Claims {
        iat: issued,
        exp: expires,
        iss: &app.client_id,
    })
    .map_err(|why| key(why.to_string()))?;
    let signing_input = format!(
        "{}.{}",
        BASE64_URL.encode(header),
        BASE64_URL.encode(claims)
    );
    let der = der_of(app.private_key.expose())
        .ok_or_else(|| key("the App's key is not the PEM the platform hands out".to_owned()))?;
    let pair = ring::signature::RsaKeyPair::from_der(&der)
        .map_err(|why| key(format!("the App's key could not be read: {why}")))?;
    let mut signature = vec![0; pair.public().modulus_len()];
    pair.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signing_input.as_bytes(),
        &mut signature,
    )
    .map_err(|_| key("the App's key would not sign".to_owned()))?;
    Ok(format!("{signing_input}.{}", BASE64_URL.encode(signature)))
}

/// The key's DER, out of the PEM the platform hands out: the base64 between
/// the two guard lines, whitespace and all. Nothing for anything else.
pub fn der_of(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("-----"))
        .collect();
    if !pem.contains("-----BEGIN RSA PRIVATE KEY-----") || body.is_empty() {
        return None;
    }
    BASE64.decode(body).ok()
}

/// One request to the platform, authenticated as the platform expects.
fn asking(method: &str, url: String, bearer: &str, body: Option<String>) -> Request {
    let mut headers: std::collections::BTreeMap<String, String> = [
        (
            "accept".to_owned(),
            "application/vnd.github+json".to_owned(),
        ),
        ("authorization".to_owned(), format!("Bearer {bearer}")),
        ("user-agent".to_owned(), STAGEMAN.to_owned()),
        ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
    ]
    .into();
    if body.is_some() {
        headers.insert("content-type".to_owned(), "application/json".to_owned());
    }
    Request {
        method: method.to_owned(),
        url,
        headers,
        body: body.map(String::into_bytes),
    }
}

/// Renders fetching an installation with the App's key: the check that it
/// is this App's, and what account it is on.
///
/// # Errors
///
/// Fails if the App's key cannot sign.
pub fn installation(app: &PlatformApp, id: u64, now: Timestamp) -> Result<Request, PlatformError> {
    Ok(asking(
        "GET",
        format!("{INSTALLATIONS}{id}"),
        &jwt(app, now)?,
        None,
    ))
}

/// An installation of the App, as much of it as this instance reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Its identifier.
    pub id: u64,
    /// The account it is installed on, as the platform spells it.
    pub account: String,
    /// Whether it covers every repository of that account rather than
    /// chosen ones.
    pub every_repository: bool,
}

/// What the platform's answer to [`installation`] means, for the App whose
/// identifier is `app`.
///
/// # Errors
///
/// Fails if the installation is not this App's — which the platform
/// answers `404` to, since the key is the App's, or which an answer naming
/// another App would mean — if the key was refused, if the platform could
/// not be reached, or if it answered with something this does not read as
/// an installation.
pub fn installed(app: u64, status: u16, body: &[u8]) -> Result<Installed, PlatformError> {
    let platform = Platform::GitHub;
    let told = read(status, body)?;
    let field = |name: &str| told.get(name).and_then(serde_json::Value::as_u64);
    if field("app_id") != Some(app) {
        return Err(PlatformError::Foreign { platform });
    }
    Ok(Installed {
        id: field("id").ok_or_else(|| PlatformError::Unreadable {
            platform,
            why: "the installation came without its id".to_owned(),
        })?,
        account: told
            .pointer("/account/login")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| PlatformError::Unreadable {
                platform,
                why: "the installation came without its account".to_owned(),
            })?,
        every_repository: told
            .get("repository_selection")
            .and_then(serde_json::Value::as_str)
            == Some("all"),
    })
}

/// Renders minting a token from an installation with the App's key: for
/// the one repository named, restricted to the App's own permissions, or
/// for everything the installation covers when none is named — which is
/// what lists its repositories, and nothing a job is ever handed.
///
/// # Errors
///
/// Fails if the App's key cannot sign.
pub fn mint(
    app: &PlatformApp,
    id: u64,
    repository: Option<&RepositoryAddress>,
    now: Timestamp,
) -> Result<Request, PlatformError> {
    let body = repository.map(|repository| {
        format!(
            r#"{{"repositories":["{}"],"permissions":{PERMISSIONS}}}"#,
            repository.name
        )
    });
    Ok(asking(
        "POST",
        format!("{INSTALLATIONS}{id}/access_tokens"),
        &jwt(app, now)?,
        body,
    ))
}

/// A token minted from an installation, and when it stops working.
#[derive(Clone, PartialEq, Eq)]
pub struct Minted {
    /// The token.
    pub token: Secret,
    /// When the platform stops accepting it, an hour after minting.
    pub expires: Timestamp,
}

/// What the platform's answer to [`mint`] means.
///
/// # Errors
///
/// Fails if the installation is not this App's, if it does not cover the
/// repository named — which the platform refuses as unprocessable — if
/// the key was refused, if the platform could not be reached, or if it
/// answered with something this does not read as a token.
pub fn minted(status: u16, body: &[u8]) -> Result<Minted, PlatformError> {
    let platform = Platform::GitHub;
    let told = read(status, body)?;
    let token = told
        .get("token")
        .and_then(serde_json::Value::as_str)
        .map(|token| Secret::new(token.to_owned()))
        .ok_or_else(|| PlatformError::Unreadable {
            platform,
            why: "the token came without its text".to_owned(),
        })?;
    let expires = told
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| PlatformError::Unreadable {
            platform,
            why: "the token came without its expiry".to_owned(),
        })?
        .parse::<Timestamp>()
        .map_err(|why| PlatformError::Unreadable {
            platform,
            why: format!("the token's expiry could not be read: {why}"),
        })?;
    Ok(Minted { token, expires })
}

/// Renders listing the repositories an installation covers, with a token
/// minted from it: the first page, which is the most the platform gives.
#[must_use]
pub fn repositories(token: &Secret) -> Request {
    asking(
        "GET",
        format!("{REPOSITORIES}?per_page={PER_PAGE}"),
        token.expose(),
        None,
    )
}

/// What the platform's answer to [`repositories`] means: what the
/// installation covers, as far as one page says, each with its
/// visibility.
///
/// # Errors
///
/// Fails if the token was refused, if the platform could not be reached,
/// or if it answered with something this does not read as a listing.
pub fn listed(status: u16, body: &[u8]) -> Result<Listing, PlatformError> {
    let platform = Platform::GitHub;
    let told = read(status, body)?;
    let listed = told
        .get("repositories")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| PlatformError::Unreadable {
            platform,
            why: "the listing came without its repositories".to_owned(),
        })?;
    let total = told
        .get("total_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    // Compared in the platform's own width; a count this could not hold
    // is one this cannot read, rather than one it guesses at.
    let counted = u64::try_from(listed.len()).map_err(|_| PlatformError::Unreadable {
        platform,
        why: "the listing is longer than can be counted".to_owned(),
    })?;
    Ok(Listing {
        more: total > counted,
        repositories: repositories_of(listed),
    })
}

/// What an answer holds, status by status, as the platform was documented
/// to answer these three requests.
fn read(status: u16, body: &[u8]) -> Result<serde_json::Value, PlatformError> {
    let platform = Platform::GitHub;
    match status {
        200..=299 => {}
        401 => return Err(PlatformError::Refused { platform }),
        404 => return Err(PlatformError::NoSuchInstallation { platform }),
        403 | 422 | 429 => {
            return Err(PlatformError::Forbidden {
                platform,
                why: super::github::message(body),
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
    serde_json::from_slice(body).map_err(|failure| PlatformError::Unreadable {
        platform,
        why: failure.to_string(),
    })
}

/// What a request asks, if it is one this module renders.
pub fn call(request: &Request) -> Option<Call> {
    let platform = Platform::GitHub;
    if let Some(rest) = request.url.strip_prefix(INSTALLATIONS) {
        if let Some(id) = rest.strip_suffix("/access_tokens") {
            if request.method != "POST" {
                return None;
            }
            let repositories = request
                .body
                .as_deref()
                .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
                .and_then(|body| {
                    body.get("repositories")
                        .and_then(serde_json::Value::as_array)
                        .map(|listed| {
                            listed
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                })
                .unwrap_or_default();
            return Some(Call::Mint {
                platform,
                id: id.parse().ok()?,
                repositories,
            });
        }
        if request.method != "GET" {
            return None;
        }
        return Some(Call::Installation {
            platform,
            id: rest.parse().ok()?,
        });
    }
    if request.method == "GET" && request.url.starts_with(REPOSITORIES) {
        return Some(Call::Repositories { platform });
    }
    None
}
