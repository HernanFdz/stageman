//! The contract every platform is asked on by this daemon, and the adapters
//! that implement it.
//!
//! One question so far: whether a credential reaches a repository, asked
//! once before the credential is kept, and where the platform's own form for
//! minting one is — see
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
//! Nothing here is performed: what is asked is rendered as a request the
//! world makes, and what the platform answers is read back from what the
//! world carried, both as pure functions the instance calls. A job reaches
//! a platform through that platform's own tools and never through this
//! crate, per
//! `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`; this
//! is the one read the daemon makes for itself, on an operator's behalf.
//!
//! **Nothing outside an adapter may be specific to one platform**, which is
//! the rule the agent and channel crates keep and the reason this crate is
//! beside them. The functions at this level dispatch on the platform and
//! name none; the module under each platform does.

mod github;
mod registration;

use std::collections::BTreeMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use stageman_core::{Platform, RepositoryAddress, Secret};

/// What a query string carries as it is: the unreserved characters, and
/// nothing else. Everything else is percent-encoded, spaces included, so a
/// sentence survives an address bar.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// One request to a platform, as the instance asks the world to make it.
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

/// Renders asking a platform whether a credential reaches a repository: a
/// read of the repository with the credential, and nothing that changes
/// anything there.
#[must_use]
pub fn reach(platform: Platform, credential: &Secret, repository: &RepositoryAddress) -> Request {
    match platform {
        Platform::GitHub => github::reach(credential, repository),
    }
}

/// What the platform's answer to [`reach`] means.
///
/// # Errors
///
/// Fails if the platform does not accept the credential, cannot see the
/// repository with it, refused to say, could not be reached, or answered
/// with something this does not read as any of those.
pub fn reached(
    platform: Platform,
    repository: &RepositoryAddress,
    status: u16,
    body: &[u8],
) -> Result<(), PlatformError> {
    match platform {
        Platform::GitHub => github::reached(repository, status, body),
    }
}

/// Where the platform's own form for minting a credential is, filled in.
///
/// Named for the project where one is named, cut to what the form takes. A
/// link and nothing more, per the record: the form is the platform's, and
/// what it cannot be told is left for the person at it.
#[must_use]
pub fn token_form(platform: Platform, project: Option<&str>) -> String {
    match platform {
        Platform::GitHub => github::token_form(project),
    }
}

/// The form the browser posts to register an App the instance owns.
///
/// Where it goes, with the state token the platform hands back — see
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
#[must_use]
pub fn register_form(platform: Platform, state: &str, organisation: Option<&str>) -> String {
    match platform {
        Platform::GitHub => registration::register_form(state, organisation),
    }
}

/// The manifest that form carries, for an instance at `instance` — scheme
/// and all — and an App installable anywhere or on its owner's account
/// only.
///
/// # Errors
///
/// Fails only if the manifest would not serialise, which is a fault in
/// this code rather than anything the caller did.
pub fn manifest(
    platform: Platform,
    instance: &str,
    anywhere: bool,
) -> Result<String, serde_json::Error> {
    match platform {
        Platform::GitHub => registration::manifest(instance, anywhere),
    }
}

/// The path under the instance's address the browser comes back to with
/// the registration's code.
#[must_use]
pub const fn registered_path(platform: Platform) -> &'static str {
    match platform {
        Platform::GitHub => registration::REGISTERED_PATH,
    }
}

/// Renders converting the code the browser came back with into the App.
#[must_use]
pub fn exchange(platform: Platform, code: &str) -> Request {
    match platform {
        Platform::GitHub => registration::exchange(code),
    }
}

/// What the platform's answer to [`exchange`] means.
///
/// # Errors
///
/// Fails if the platform did not create the App, could not be reached, or
/// answered with something this does not read as an App.
pub fn registered(
    platform: Platform,
    status: u16,
    body: &[u8],
) -> Result<Registered, PlatformError> {
    match platform {
        Platform::GitHub => registration::registered(status, body),
    }
}

/// Where an App is seen on the platform, by its slug.
#[must_use]
pub fn app_link(platform: Platform, slug: &str) -> String {
    match platform {
        Platform::GitHub => registration::app_link(slug),
    }
}

pub use registration::Registered;

/// What a person calls the platform.
#[must_use]
pub const fn shown(platform: Platform) -> &'static str {
    match platform {
        Platform::GitHub => "GitHub",
    }
}

/// What a request asks of a platform, read back from what would be sent.
///
/// The inverse of what this crate renders, for a simulated platform to
/// recognise what it is asked and answer as the real one was measured to,
/// without matching on strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// A repository read with a credential.
    Repository {
        /// Which platform.
        platform: Platform,
        /// Who owns it, as the platform spells it.
        owner: String,
        /// What it is called there.
        name: String,
    },
    /// A registration's code converted into the App.
    Exchange {
        /// Which platform.
        platform: Platform,
        /// The code the browser came back with.
        code: String,
    },
}

impl Call {
    /// What a request asks, if it is one this crate renders.
    #[must_use]
    pub fn parse(request: &Request) -> Option<Self> {
        github::call(request).or_else(|| registration::call(request))
    }
}

/// A platform would not have a credential, or could not be asked.
///
/// Every message names the platform and reads as the clause after *the
/// token was not kept:*, which is how a screen shows it beside the box the
/// credential was typed in.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// The request never got an answer, or got one that says nothing about
    /// the credential.
    #[error("{} could not be reached: {why}", shown(*.platform))]
    Unreachable {
        /// Which platform.
        platform: Platform,
        /// What went wrong.
        why: String,
    },
    /// It answered, and does not accept the credential.
    #[error("{} does not accept it", shown(*.platform))]
    Refused {
        /// Which platform.
        platform: Platform,
    },
    /// It answered, and cannot see the repository with the credential: for
    /// a private repository, the credential was not granted it.
    #[error(
        "{} cannot see {repository} with it — a fine-grained token has to be granted that \
         repository",
        shown(*.platform)
    )]
    NotGranted {
        /// Which platform.
        platform: Platform,
        /// The repository, as the platform names it.
        repository: String,
    },
    /// It answered, and would not say: a limit reached, or an
    /// organisation's rule in the way.
    #[error("{} refused: {why}", shown(*.platform))]
    Forbidden {
        /// Which platform.
        platform: Platform,
        /// What it said.
        why: String,
    },
    /// It answered with a status this does not read as any of the above.
    #[error("{} answered {status}", shown(*.platform))]
    Unexpected {
        /// Which platform.
        platform: Platform,
        /// What it answered.
        status: u16,
    },
    /// It answered, and does not know the code a registration came back
    /// with: one is good for an hour, and once.
    #[error("{} does not know that code — one is good for an hour, and once", shown(*.platform))]
    Spent {
        /// Which platform.
        platform: Platform,
    },
    /// It answered with something this cannot read.
    #[error("{} answered something unreadable: {why}", shown(*.platform))]
    Unreadable {
        /// Which platform.
        platform: Platform,
        /// What could not be read.
        why: String,
    },
}

impl PlatformError {
    /// Whether the platform was never asked, as opposed to having answered:
    /// what tells *unchecked* from *refused* for whoever typed the
    /// credential.
    #[must_use]
    pub const fn unreachable(&self) -> bool {
        matches!(self, Self::Unreachable { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Call, PlatformError, app_link, exchange, manifest, reach, reached, register_form,
        registered, shown, token_form,
    };
    use stageman_core::{Platform, RepositoryAddress, Secret};

    /// The manifest, asserted whole per `docs/conventions.md` §4: a
    /// document this project composes. No webhook in it, for the reason the
    /// module gives; the two addresses hang off the instance's own.
    #[test]
    fn the_manifest_names_the_instance_and_declares_no_webhook() {
        assert_eq!(
            manifest(Platform::GitHub, "http://localhost:8080", false).expect("serialises"),
            r#"{"name":"stageman","url":"http://localhost:8080","description":"Installed on a repository so that stageman's jobs can clone it, push a branch, open a pull request and work an issue. Owned by whoever runs the instance; nothing else can use it.","redirect_url":"http://localhost:8080/instance/apps/github/registered","setup_url":"http://localhost:8080/instance/apps/github/installed","setup_on_update":true,"public":false,"default_permissions":{"contents":"write","issues":"write","pull_requests":"write","metadata":"read"},"default_events":[]}"#
        );
        assert!(
            manifest(Platform::GitHub, "https://stageman.example.com", true)
                .expect("serialises")
                .contains(r#""public":true"#)
        );
        assert_eq!(
            register_form(Platform::GitHub, "f00d", None),
            "https://github.com/settings/apps/new?state=f00d"
        );
        assert_eq!(
            register_form(Platform::GitHub, "f00d", Some("acme")),
            "https://github.com/organizations/acme/settings/apps/new?state=f00d"
        );
        assert_eq!(
            app_link(Platform::GitHub, "stageman-acme"),
            "https://github.com/apps/stageman-acme"
        );
    }

    /// The exchange reads back as what it asked, and the platform's answer
    /// is read status by status: the App with its key, a spent code, and
    /// an App that came without a field.
    #[test]
    fn an_exchange_reads_back_and_its_answer_is_read() {
        let asking = exchange(Platform::GitHub, "c0de");
        assert_eq!(asking.method, "POST");
        assert_eq!(
            asking.url,
            "https://api.github.com/app-manifests/c0de/conversions"
        );
        assert!(asking.headers.contains_key("user-agent"));
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Exchange {
                platform: Platform::GitHub,
                code: "c0de".to_owned(),
            })
        );
        let created = r#"{"id":7,"slug":"stageman-acme","client_id":"Iv1.abc","pem":"-----BEGIN RSA PRIVATE KEY-----\nk\n-----END RSA PRIVATE KEY-----\n","client_secret":"s","webhook_secret":"w","html_url":"https://github.com/apps/stageman-acme"}"#;
        let read = registered(Platform::GitHub, 201, created.as_bytes()).expect("an App");
        assert_eq!(
            (read.id, read.slug.as_str(), read.client_id.as_str()),
            (7, "stageman-acme", "Iv1.abc")
        );
        assert!(read.private_key.expose().starts_with("-----BEGIN"));
        assert_eq!(
            registered(Platform::GitHub, 404, b"{}"),
            Err(PlatformError::Spent {
                platform: Platform::GitHub
            })
        );
        assert_eq!(
            registered(
                Platform::GitHub,
                201,
                br#"{"id":7,"slug":"s","client_id":"c"}"#
            ),
            Err(PlatformError::Unreadable {
                platform: Platform::GitHub,
                why: "the App came without its pem".to_owned(),
            })
        );
        assert_eq!(
            PlatformError::Spent {
                platform: Platform::GitHub
            }
            .to_string(),
            "GitHub does not know that code — one is good for an hour, and once"
        );
    }

    fn repository() -> RepositoryAddress {
        RepositoryAddress::parse("https://github.com/owner/name").expect("an address")
    }

    /// The read carries the credential as the platform expects it, names
    /// this client as the platform requires, and reads back as what it
    /// asked.
    #[test]
    fn a_read_of_the_repository_reads_back_as_what_it_asked() {
        let asking = reach(
            Platform::GitHub,
            &Secret::new("github_pat_not_a_real_token".to_owned()),
            &repository(),
        );
        assert_eq!(asking.method, "GET");
        assert_eq!(asking.url, "https://api.github.com/repos/owner/name");
        assert_eq!(
            asking.headers.get("authorization").map(String::as_str),
            Some("Bearer github_pat_not_a_real_token")
        );
        assert_eq!(
            asking.headers.get("user-agent").map(String::as_str),
            Some("stageman")
        );
        assert!(asking.body.is_none());
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Repository {
                platform: Platform::GitHub,
                owner: "owner".to_owned(),
                name: "name".to_owned(),
            })
        );

        let mut other = asking;
        other.method = "POST".to_owned();
        assert_eq!(Call::parse(&other), None);
        other.method = "GET".to_owned();
        other.url = "https://api.github.com/repos/owner/name/pulls".to_owned();
        assert_eq!(Call::parse(&other), None);
        other.url = "https://api.github.com/user".to_owned();
        assert_eq!(Call::parse(&other), None);
        other.url = "https://api.github.com/repos//name".to_owned();
        assert_eq!(Call::parse(&other), None, "no owner is no repository");
        other.url = "https://api.github.com/repos/owner/".to_owned();
        assert_eq!(Call::parse(&other), None, "no name is no repository");
    }

    /// Each status the platform was measured to answer means one thing,
    /// and every meaning says which platform.
    #[test]
    fn the_answer_is_read_as_the_platform_was_measured_to_answer() {
        let read = |status: u16, body: &str| {
            reached(Platform::GitHub, &repository(), status, body.as_bytes())
        };
        assert_eq!(read(200, r#"{"full_name":"owner/name"}"#), Ok(()));
        assert_eq!(
            read(401, r#"{"message":"Bad credentials"}"#),
            Err(PlatformError::Refused {
                platform: Platform::GitHub
            })
        );
        assert_eq!(
            read(404, r#"{"message":"Not Found"}"#),
            Err(PlatformError::NotGranted {
                platform: Platform::GitHub,
                repository: "owner/name".to_owned(),
            })
        );
        assert_eq!(
            read(403, r#"{"message":"API rate limit exceeded"}"#),
            Err(PlatformError::Forbidden {
                platform: Platform::GitHub,
                why: "API rate limit exceeded".to_owned(),
            })
        );
        assert_eq!(
            read(403, "not json"),
            Err(PlatformError::Forbidden {
                platform: Platform::GitHub,
                why: "no reason given".to_owned(),
            })
        );
        assert_eq!(
            read(502, ""),
            Err(PlatformError::Unreachable {
                platform: Platform::GitHub,
                why: "it answered 502".to_owned(),
            })
        );
        assert_eq!(
            read(418, ""),
            Err(PlatformError::Unexpected {
                platform: Platform::GitHub,
                status: 418,
            })
        );
        assert!(read(502, "").is_err_and(|why| why.unreachable()));
        assert!(read(401, "").is_err_and(|why| !why.unreachable()));
    }

    /// The words a box shows, asserted whole per `docs/conventions.md` §4.
    #[test]
    fn every_failure_reads_as_the_clause_after_not_kept() {
        let said = |error: PlatformError| error.to_string();
        assert_eq!(
            said(PlatformError::Refused {
                platform: Platform::GitHub
            }),
            "GitHub does not accept it"
        );
        assert_eq!(
            said(PlatformError::NotGranted {
                platform: Platform::GitHub,
                repository: "owner/name".to_owned(),
            }),
            "GitHub cannot see owner/name with it — a fine-grained token has to be granted that \
             repository"
        );
        assert_eq!(
            said(PlatformError::Forbidden {
                platform: Platform::GitHub,
                why: "API rate limit exceeded".to_owned(),
            }),
            "GitHub refused: API rate limit exceeded"
        );
        assert_eq!(
            said(PlatformError::Unreachable {
                platform: Platform::GitHub,
                why: "dns error".to_owned(),
            }),
            "GitHub could not be reached: dns error"
        );
        assert_eq!(
            said(PlatformError::Unexpected {
                platform: Platform::GitHub,
                status: 418,
            }),
            "GitHub answered 418"
        );
        assert_eq!(shown(Platform::GitHub), "GitHub");
    }

    /// The form is filled in through its address, with every word encoded
    /// so the address survives being pasted, and named for the project
    /// where there is one, cut to the forty characters the form takes.
    /// Asserted whole, since it is a text this project composes.
    #[test]
    fn the_token_form_is_filled_in_through_its_address() {
        assert_eq!(
            token_form(Platform::GitHub, None),
            "https://github.com/settings/personal-access-tokens/new?name=stageman\
             &contents=write&issues=write&pull_requests=write"
        );
        assert!(
            token_form(Platform::GitHub, Some("Closed Loop"))
                .contains("?name=stageman%3A%20Closed%20Loop&"),
            "named for the project"
        );
        let long = token_form(
            Platform::GitHub,
            Some("a project whose name runs on and on and on"),
        );
        assert!(
            long.contains("?name=stageman%3A%20a%20project%20whose%20name%20runs%20on%20a&"),
            "cut to what the form takes: {long}"
        );
    }
}
