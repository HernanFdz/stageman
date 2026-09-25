//! GitHub: an App registered from the dashboard by the platform's manifest
//! flow, and what the platform answers.
//!
//! The dashboard posts a form to the platform with a manifest and a state
//! token; the platform creates the App and sends the browser back to the
//! manifest's redirect address with a code; one unauthenticated request
//! converts the code into the App's identifier, slug, client credentials
//! and private key, within the hour the code is good for.
//! Everything here is what the platform documents, checked rather than
//! remembered — see
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.

use stageman_core::{Platform, Secret};

use crate::{Call, PlatformError, Request};

/// Where a personal account's App is registered from a manifest.
const REGISTER: &str = "https://github.com/settings/apps/new";

/// Where an organisation's is: the organisation's name goes in the middle.
const REGISTER_FOR_ORGANISATION: (&str, &str) =
    ("https://github.com/organizations/", "/settings/apps/new");

/// Where a code is converted into the App, with no authentication.
const CONVERSIONS: &str = "https://api.github.com/app-manifests/";

/// Where an App is seen on the platform.
const APPS: &str = "https://github.com/apps/";

/// What the platform is told this client is.
const STAGEMAN: &str = "stageman";

/// The path under this instance's address the browser comes back to with
/// the code, and the one it comes back to after an installation. Both are
/// the instance's own, answered before the proxy.
pub const REGISTERED_PATH: &str = "/instance/apps/github/registered";
pub const INSTALLED_PATH: &str = "/instance/apps/github/installed";

/// What the App says it is for, on the platform's page.
const DESCRIPTION: &str = "Installed on a repository so that stageman's jobs can clone it, push a \
                           branch, open a pull request and work an issue. Owned by whoever runs \
                           the instance; nothing else can use it.";

/// The form the browser posts to register the App: where it goes, with the
/// state token the platform hands back.
#[must_use]
pub fn register_form(state: &str, organisation: Option<&str>) -> String {
    let (before, after) = REGISTER_FOR_ORGANISATION;
    organisation.map_or_else(
        || format!("{REGISTER}?state={state}"),
        |organisation| format!("{before}{organisation}{after}?state={state}"),
    )
}

/// The manifest the form carries: the App's name, where it comes back to,
/// and what it may do, with no webhook at all.
///
/// No webhook rather than one switched off: the platform refuses a webhook
/// address it cannot reach over the public Internet even when the hook is
/// declared inactive, measured on 2026-09-24 with an address on localhost,
/// and signals reach this instance through Slack anyway.
///
/// `instance` is this instance's own address, scheme and all, which the
/// redirect and setup addresses hang off. `anywhere` is whether the App
/// may be installed on any account rather than its owner's only.
///
/// # Errors
///
/// Fails only if a struct of strings and booleans would not serialise,
/// which is a fault in this code rather than anything the caller did.
pub fn manifest(instance: &str, anywhere: bool) -> Result<String, serde_json::Error> {
    // A struct rather than a JSON literal, so that the fields come out in
    // this order whatever a map does with its keys, and the text can be
    // asserted whole.
    #[derive(serde::Serialize)]
    struct Manifest<'a> {
        name: &'a str,
        url: &'a str,
        description: &'a str,
        redirect_url: String,
        setup_url: String,
        setup_on_update: bool,
        public: bool,
        default_permissions: Permissions<'a>,
        default_events: &'a [&'a str],
    }
    #[derive(serde::Serialize)]
    struct Permissions<'a> {
        contents: &'a str,
        issues: &'a str,
        pull_requests: &'a str,
        metadata: &'a str,
    }
    let manifest = Manifest {
        name: STAGEMAN,
        url: instance,
        description: DESCRIPTION,
        redirect_url: format!("{instance}{REGISTERED_PATH}"),
        setup_url: format!("{instance}{INSTALLED_PATH}"),
        setup_on_update: true,
        public: anywhere,
        default_permissions: Permissions {
            contents: "write",
            issues: "write",
            pull_requests: "write",
            metadata: "read",
        },
        default_events: &[],
    };
    serde_json::to_string(&manifest)
}

/// Renders converting the code the browser came back with into the App.
#[must_use]
pub fn exchange(code: &str) -> Request {
    Request {
        method: "POST".to_owned(),
        url: format!("{CONVERSIONS}{code}/conversions"),
        headers: [
            (
                "accept".to_owned(),
                "application/vnd.github+json".to_owned(),
            ),
            ("user-agent".to_owned(), STAGEMAN.to_owned()),
            ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
        ]
        .into(),
        body: None,
    }
}

/// What the platform registered, as much of it as this instance keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    /// The App's identifier.
    pub id: u64,
    /// Its slug.
    pub slug: String,
    /// Its client identifier, the issuer of the token this instance signs.
    pub client_id: String,
    /// Its private key, PEM.
    pub private_key: Secret,
}

/// What the platform's answer to [`exchange`] means.
///
/// A created App answers `201` with the App object and, beside it, the
/// client secret, the webhook secret and the key; a code the platform does
/// not know, or one older than its hour, answers `404`.
///
/// # Errors
///
/// Fails if the platform did not create the App, could not be reached, or
/// answered with something this does not read as an App.
pub fn registered(status: u16, body: &[u8]) -> Result<Registered, PlatformError> {
    let platform = Platform::GitHub;
    match status {
        200..=299 => {}
        404 => return Err(PlatformError::Spent { platform }),
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
    let told: serde_json::Value =
        serde_json::from_slice(body).map_err(|failure| PlatformError::Unreadable {
            platform,
            why: failure.to_string(),
        })?;
    let field = |name: &str| {
        told.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| PlatformError::Unreadable {
                platform,
                why: format!("the App came without its {name}"),
            })
    };
    let id = told
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| PlatformError::Unreadable {
            platform,
            why: "the App came without its id".to_owned(),
        })?;
    Ok(Registered {
        id,
        slug: field("slug")?,
        client_id: field("client_id")?,
        private_key: Secret::new(field("pem")?),
    })
}

/// Where an App is seen on the platform, by its slug.
#[must_use]
pub fn app_link(slug: &str) -> String {
    format!("{APPS}{slug}")
}

/// What a request asks, if it is one this module renders.
pub fn call(request: &Request) -> Option<Call> {
    if request.method != "POST" {
        return None;
    }
    let rest = request.url.strip_prefix(CONVERSIONS)?;
    let code = rest.strip_suffix("/conversions")?;
    if code.is_empty() || code.contains('/') {
        return None;
    }
    Some(Call::Exchange {
        platform: Platform::GitHub,
        code: code.to_owned(),
    })
}
