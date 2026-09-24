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
mod installation;
mod registration;

use std::collections::BTreeMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use stageman_core::{Platform, PlatformApp, RepositoryAddress, Secret, Timestamp};

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

pub use github::{Listing, Owned, Repository};
pub use installation::{Installed, Minted};
pub use registration::Registered;

/// Where a person installs the App, carrying a state the platform brings
/// back with the installation.
///
/// See
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
/// The state names the page the tab was opened from and nothing about the
/// installation, per
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`:
/// the App's key confirms whatever comes back.
#[must_use]
pub fn install_link(platform: Platform, slug: &str, state: &str) -> String {
    match platform {
        Platform::GitHub => installation::install_link(slug, state),
    }
}

/// Renders listing what a token can read — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
#[must_use]
pub fn readable(platform: Platform, credential: &Secret) -> Request {
    match platform {
        Platform::GitHub => github::readable(credential),
    }
}

/// What the platform's answer to [`readable`] means.
///
/// # Errors
///
/// Fails if the platform does not accept the token, refused to say, could
/// not be reached, or answered with something this does not read as a
/// listing.
pub fn readable_listed(
    platform: Platform,
    status: u16,
    body: &[u8],
) -> Result<Listing, PlatformError> {
    match platform {
        Platform::GitHub => github::readable_listed(status, body),
    }
}

/// Renders asking whose a token is — see
/// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
#[must_use]
pub fn owner(platform: Platform, credential: &Secret) -> Request {
    match platform {
        Platform::GitHub => github::owner(credential),
    }
}

/// What the platform's answer to [`owner`] means: the account, and when
/// the token expires where the platform says.
///
/// # Errors
///
/// Fails if the platform does not accept the token, refused to say, could
/// not be reached, or answered with something this does not read.
pub fn owned(
    platform: Platform,
    status: u16,
    headers: &std::collections::BTreeMap<String, String>,
    body: &[u8],
) -> Result<Owned, PlatformError> {
    match platform {
        Platform::GitHub => github::owned(status, headers, body),
    }
}

/// What the platform calls the App's own account, which commits made with
/// an installation's token are attributed to: the slug, marked as a bot.
#[must_use]
pub fn bot_name(platform: Platform, slug: &str) -> String {
    match platform {
        Platform::GitHub => format!("{slug}[bot]"),
    }
}

/// The path under the instance's address the browser comes back to after
/// an installation, with the installation's identifier.
#[must_use]
pub const fn installed_path(platform: Platform) -> &'static str {
    match platform {
        Platform::GitHub => registration::INSTALLED_PATH,
    }
}

/// Renders fetching an installation with the App's key, at `now`.
///
/// # Errors
///
/// Fails if the App's key cannot sign.
pub fn installation(
    platform: Platform,
    app: &PlatformApp,
    id: u64,
    now: Timestamp,
) -> Result<Request, PlatformError> {
    match platform {
        Platform::GitHub => installation::installation(app, id, now),
    }
}

/// What the platform's answer to [`installation`] means, for this App.
///
/// # Errors
///
/// Fails if the installation is not this App's, if the key was refused,
/// if the platform could not be reached, or if the answer does not read as
/// an installation.
pub fn installed(
    platform: Platform,
    app: u64,
    status: u16,
    body: &[u8],
) -> Result<Installed, PlatformError> {
    match platform {
        Platform::GitHub => installation::installed(app, status, body),
    }
}

/// Renders minting a token from an installation with the App's key: for
/// the one repository named, or for everything the installation covers
/// when none is.
///
/// # Errors
///
/// Fails if the App's key cannot sign.
pub fn mint(
    platform: Platform,
    app: &PlatformApp,
    id: u64,
    repository: Option<&RepositoryAddress>,
    now: Timestamp,
) -> Result<Request, PlatformError> {
    match platform {
        Platform::GitHub => installation::mint(app, id, repository, now),
    }
}

/// What the platform's answer to [`mint`] means.
///
/// # Errors
///
/// Fails if the installation is not this App's or does not cover the
/// repository, if the key was refused, if the platform could not be
/// reached, or if the answer does not read as a token.
pub fn minted(platform: Platform, status: u16, body: &[u8]) -> Result<Minted, PlatformError> {
    match platform {
        Platform::GitHub => installation::minted(status, body),
    }
}

/// Renders listing the repositories an installation covers, with a token
/// minted from it.
#[must_use]
pub fn repositories(platform: Platform, token: &Secret) -> Request {
    match platform {
        Platform::GitHub => installation::repositories(token),
    }
}

/// What the platform's answer to [`repositories`] means.
///
/// # Errors
///
/// Fails if the token was refused, if the platform could not be reached,
/// or if the answer does not read as a listing.
pub fn listed(platform: Platform, status: u16, body: &[u8]) -> Result<Listing, PlatformError> {
    match platform {
        Platform::GitHub => installation::listed(status, body),
    }
}

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
    /// An installation fetched with the App's key.
    Installation {
        /// Which platform.
        platform: Platform,
        /// The installation's identifier.
        id: u64,
    },
    /// A token minted from an installation with the App's key.
    Mint {
        /// Which platform.
        platform: Platform,
        /// The installation's identifier.
        id: u64,
        /// The repositories it is restricted to, by name; none for a token
        /// covering everything the installation does.
        repositories: Vec<String>,
    },
    /// The repositories an installation covers, listed with a token
    /// minted from it.
    Repositories {
        /// Which platform.
        platform: Platform,
    },
    /// What a token can read, listed with it.
    Readable {
        /// Which platform.
        platform: Platform,
    },
    /// Whose a token is, asked with it.
    Owner {
        /// Which platform.
        platform: Platform,
    },
}

impl Call {
    /// What a request asks, if it is one this crate renders.
    #[must_use]
    pub fn parse(request: &Request) -> Option<Self> {
        github::call(request)
            .or_else(|| registration::call(request))
            .or_else(|| installation::call(request))
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
    /// The App's key could not sign, or could not be read.
    #[error("the App's key on {} could not be used: {why}", shown(*.platform))]
    Key {
        /// Which platform.
        platform: Platform,
        /// What was wrong with it.
        why: String,
    },
    /// It answered, and knows no installation by that identifier for this
    /// App: the identifier the redirect carried is not one of this App's,
    /// or the installation has since been removed.
    #[error("{} knows no installation with that identifier for this App", shown(*.platform))]
    NoSuchInstallation {
        /// Which platform.
        platform: Platform,
    },
    /// It answered with an installation that names another App.
    #[error("that installation belongs to another App on {}", shown(*.platform))]
    Foreign {
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
        Call, PlatformError, app_link, bot_name, exchange, install_link, installation, installed,
        installed_path, listed, manifest, mint, minted, owned, owner, reach, reached, readable,
        readable_listed, register_form, registered, repositories, shown, token_form,
    };
    use stageman_core::{Platform, PlatformApp, RepositoryAddress, Secret, Timestamp};

    /// A key made for these tests and used for nothing, in the shape the
    /// platform hands out: what the simulated App signs with.
    pub const TEST_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
         MIIEpAIBAAKCAQEAv48aiq9x2RBccn267zi6TArEnVXppTczV2jP6z4mRT06Pk4n\n\
         s9QqKi+t/cSfX+9cgVFj/UHvB43UwZm8ZbcnyZhg5BdOF8m+POt79O4AbJWtlCQF\n\
         fdBhmAvn685Mak+9mI+VQbboU86Xf2bhJl48kuaiqP6YPpo8MCA9xnjMn1+8OiOr\n\
         Er5X/hmMVv7tIIaipeEAl6WEYifX+SD1B5WU8TsYImB/2pviNKhdZ4m5hCZ3z1fU\n\
         zJkv/0eSiJtq1hZ8c3BoY4d4sAfzETnNmmhVIc2E8Zfvs8tyZuZocyudOiqlz7O3\n\
         7Z4DwDjHc6WM1f4GFblxWMpq9jR/lb1E6WBtUwIDAQABAoIBAQCsuIeiDNeGdO4m\n\
         fZ+UG34/GmZ1xwVI5yDv652t6vfu7moZy7aYuvDZ4OvtKODbS6QJJi4WKOEx2ny/\n\
         o7Lvs9m4OCEFCM5tPIa/v0ShcAgJ4FwGewRIkR+uTO3s/LKCGSxG5xAZlKafCmQn\n\
         h8fzJH1Rp4t6/TShHcivTCLnVfyKpeRe1LPFCWm0M6smwzaQZZ5zwjes3OemrHBm\n\
         gVkmh656n14ESmAu5n1htza/J3nlsa4l4UitRbPhHRnaTyr80oQgwy6eeioNtdy6\n\
         63w29/Q9Thc4koUGsd09nIKlaFUCDMVUU3CtwAtli10Lf5tr3Ivgj3t3gOjlejlr\n\
         5/nUxPihAoGBAN/9BCjjYiLhjkuYqK0VF8rI74BjseSJZJTDW2bpjL9ugRtNRahW\n\
         OiXhBSQ0KbgjjZWiKlEVFMDicN1p6MXfuPncJcm+Yp5+vnnIVtl3fi6SGWn0w9VE\n\
         LB0XIBEJArkE8B6ZRppyzNwhD7iKrfQNWzUiw4k9PMYMjc2rsuZJSYvdAoGBANrv\n\
         mzDnpeU8TUma+ccBAAwRsI3y4QurRiJtpl1WQ2JQve+Tyl1GAOIV5GDN55UMmxW/\n\
         zNfAQhOfAl0Ok9hlBWxTkrXtjt5IXpZOp/hEcgNBw+yn6Ml8x6P8ta/wlaMgpsjZ\n\
         ijku+4lMQ6Wbb09CbHlP3Rdv8Ya7k8+tEkObOaLvAoGAIcHLH7JtNt6RiHkgar10\n\
         EX7JAauEwvGl8/mhS9hE+xDXalrx9ZXRO6Y3FSa7ZuIM05FWGVQ5BXzbD7OHflLi\n\
         WN3B4C7ORB7L7CSyWiH1JWWlaN+XqAuXLmcu0QJvo5zH54SoLFzC3SYqbWCRKOfe\n\
         aBquJ3/QKfT4ZhfLZYOEDw0CgYEAyBMdnLSlK3dPHgvNZWppk5363c3ukU5lGoNf\n\
         /H4fuFIXMUC7N0AJAJOHEFw63UAW3epYlXYyLGIss8PlomS3bwZ01WMSI9q47d1V\n\
         rRFHq+hG1xefKbqpaxg/JVjUNq5ZHMWIhreD0TXrwATq1ODb5oTwhEGd1EXJT4lX\n\
         XocVResCgYBam6iJqyhk3ExJ0eu4MT5sWdeqliLd5NRZ/XLIBQ2fuiZU1nFSbgIz\n\
         nFaZU3jL1az+IsGuqjcB+GwqFt0P9HolFKfaK4y2xiH9UaEI08AqEZqDRB2VMNOv\n\
         W3w9IvDekeaSfWp+tELfYbY3v+k+BDpTlHTKyIX9QiKEv+UAjjRo3A==\n\
         -----END RSA PRIVATE KEY-----\n";

    fn app() -> PlatformApp {
        PlatformApp {
            id: 4242,
            slug: "stageman-sim".to_owned(),
            client_id: "Iv1.sim".to_owned(),
            private_key: Secret::new(TEST_KEY.to_owned()),
            installations: std::collections::BTreeMap::new(),
        }
    }

    fn at() -> Timestamp {
        Timestamp::from_second(1_790_000_000).expect("a time")
    }

    /// The bearer an App's request carries.
    fn bearer(request: &super::Request) -> String {
        request
            .headers
            .get("authorization")
            .and_then(|header| header.strip_prefix("Bearer "))
            .expect("a bearer")
            .to_owned()
    }

    /// The token is three parts the platform documents, signed with the
    /// App's key so that its own public half verifies it, and the same
    /// twice for the same instant.
    #[test]
    fn a_token_is_signed_for_the_app_and_verifies_with_its_key() {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let asking = installation(Platform::GitHub, &app(), 77, at()).expect("signs");
        let token = bearer(&asking);
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "{token}");
        assert_eq!(
            String::from_utf8(URL_SAFE_NO_PAD.decode(parts[0]).expect("base64")).expect("text"),
            r#"{"alg":"RS256","typ":"JWT"}"#
        );
        assert_eq!(
            String::from_utf8(URL_SAFE_NO_PAD.decode(parts[1]).expect("base64")).expect("text"),
            r#"{"iat":1789999940,"exp":1790000600,"iss":"Iv1.sim"}"#,
            "issued a minute back, good for ten, by the client identifier"
        );
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).expect("base64");
        let der = super::installation::der_of(TEST_KEY).expect("the test key is a PEM");
        let pair = ring::signature::RsaKeyPair::from_der(&der).expect("the test key reads");
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            pair.public().as_ref(),
        )
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("the App's own public key verifies what its private key signed");

        let again = installation(Platform::GitHub, &app(), 77, at()).expect("signs");
        assert_eq!(bearer(&again), token, "signing is deterministic");
    }

    /// A key that is not the PEM the platform hands out cannot sign, and
    /// says so before anything is asked.
    #[test]
    fn a_key_that_is_not_the_platforms_pem_is_refused() {
        for broken in [
            "-----BEGIN RSA PRIVATE KEY-----\nk\n-----END RSA PRIVATE KEY-----\n",
            "not a key",
            "",
        ] {
            let mut app = app();
            app.private_key = Secret::new(broken.to_owned());
            assert!(
                matches!(
                    installation(Platform::GitHub, &app, 77, at()),
                    Err(PlatformError::Key { .. })
                ),
                "{broken:?}"
            );
        }
        assert_eq!(
            PlatformError::Key {
                platform: Platform::GitHub,
                why: "it would not sign".to_owned()
            }
            .to_string(),
            "the App's key on GitHub could not be used: it would not sign"
        );
    }

    /// An installation is fetched with the key, reads back as what it
    /// asked, and is read as this App's or refused: by the platform, which
    /// knows no installation of another App's under the key, or by the
    /// answer naming another App.
    #[test]
    fn an_installation_is_fetched_with_the_key_and_read_as_this_apps_or_refused() {
        let asking = installation(Platform::GitHub, &app(), 77, at()).expect("signs");
        assert_eq!(asking.method, "GET");
        assert_eq!(asking.url, "https://api.github.com/app/installations/77");
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Installation {
                platform: Platform::GitHub,
                id: 77,
            })
        );
        let read =
            |status: u16, body: &str| installed(Platform::GitHub, 4242, status, body.as_bytes());
        assert_eq!(
            read(
                200,
                r#"{"id":77,"app_id":4242,"account":{"login":"example"},"repository_selection":"selected"}"#
            ),
            Ok(super::Installed {
                id: 77,
                account: "example".to_owned(),
                every_repository: false,
            })
        );
        assert_eq!(
            read(
                200,
                r#"{"id":77,"app_id":9999,"account":{"login":"example"},"repository_selection":"all"}"#
            ),
            Err(PlatformError::Foreign {
                platform: Platform::GitHub
            })
        );
        assert_eq!(
            read(404, r#"{"message":"Not Found"}"#),
            Err(PlatformError::NoSuchInstallation {
                platform: Platform::GitHub
            })
        );
        assert_eq!(
            read(401, r#"{"message":"Bad credentials"}"#),
            Err(PlatformError::Refused {
                platform: Platform::GitHub
            })
        );
        assert_eq!(
            read(200, r#"{"app_id":4242}"#),
            Err(PlatformError::Unreadable {
                platform: Platform::GitHub,
                why: "the installation came without its id".to_owned(),
            })
        );
        assert_eq!(
            PlatformError::NoSuchInstallation {
                platform: Platform::GitHub
            }
            .to_string(),
            "GitHub knows no installation with that identifier for this App"
        );
        assert_eq!(
            PlatformError::Foreign {
                platform: Platform::GitHub
            }
            .to_string(),
            "that installation belongs to another App on GitHub"
        );
    }

    /// A token is minted for the one repository named with the App's own
    /// permissions, or for everything the installation covers when none
    /// is, and read with its expiry.
    #[test]
    fn a_token_is_minted_for_one_repository_and_read() {
        let asking = mint(Platform::GitHub, &app(), 77, Some(&repository()), at()).expect("signs");
        assert_eq!(asking.method, "POST");
        assert_eq!(
            asking.url,
            "https://api.github.com/app/installations/77/access_tokens"
        );
        assert_eq!(
            asking.body.as_deref().map(String::from_utf8_lossy),
            Some(
                r#"{"repositories":["name"],"permissions":{"contents":"write","issues":"write","pull_requests":"write","metadata":"read"}}"#
                    .into()
            )
        );
        assert_eq!(
            asking.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Mint {
                platform: Platform::GitHub,
                id: 77,
                repositories: vec!["name".to_owned()],
            })
        );
        let everything = mint(Platform::GitHub, &app(), 77, None, at()).expect("signs");
        assert_eq!(everything.body, None);
        assert_eq!(
            Call::parse(&everything),
            Some(Call::Mint {
                platform: Platform::GitHub,
                id: 77,
                repositories: Vec::new(),
            })
        );

        let read = minted(
            Platform::GitHub,
            201,
            br#"{"token":"ghs_not_a_real_token","expires_at":"2026-09-24T08:00:00Z","permissions":{"contents":"write"}}"#,
        )
        .expect("a token");
        assert_eq!(read.token.expose(), "ghs_not_a_real_token");
        assert_eq!(
            read.expires,
            "2026-09-24T08:00:00Z".parse::<Timestamp>().expect("a time")
        );
        assert_eq!(
            minted(
                Platform::GitHub,
                422,
                br#"{"message":"There is at least one repository that does not exist or is not accessible to the parent installation."}"#
            )
            .err(),
            Some(PlatformError::Forbidden {
                platform: Platform::GitHub,
                why: "There is at least one repository that does not exist or is not accessible to the parent installation.".to_owned(),
            })
        );
        assert!(matches!(
            minted(Platform::GitHub, 201, br#"{"token":"ghs_x"}"#).err(),
            Some(PlatformError::Unreadable { .. })
        ));
    }

    /// The repositories an installation covers are listed with a token
    /// minted from it, one page at most, and read as addresses with their
    /// visibility, skipping what is not an address and saying whether
    /// more were left out.
    #[test]
    fn the_installations_repositories_are_listed_and_read() {
        let asking = repositories(Platform::GitHub, &Secret::new("ghs_x".to_owned()));
        assert_eq!(asking.method, "GET");
        assert_eq!(
            asking.url,
            "https://api.github.com/installation/repositories?per_page=100"
        );
        assert_eq!(bearer(&asking), "ghs_x");
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Repositories {
                platform: Platform::GitHub
            })
        );
        let read = listed(
            Platform::GitHub,
            200,
            br#"{"total_count":4,"repositories":[{"html_url":"https://github.com/example/a","private":true},{"html_url":"https://github.com/example/b","private":false},{"html_url":"not-an-address"}]}"#,
        )
        .expect("a listing");
        assert_eq!(
            read.repositories
                .iter()
                .map(|repository| (repository.address.https(), repository.private))
                .collect::<Vec<_>>(),
            [
                ("https://github.com/example/a".to_owned(), true),
                ("https://github.com/example/b".to_owned(), false)
            ]
        );
        assert!(read.more, "four covered, three listed, two read");
        let whole = listed(
            Platform::GitHub,
            200,
            br#"{"total_count":1,"repositories":[{"html_url":"https://github.com/example/a"}]}"#,
        )
        .expect("a listing");
        assert!(!whole.more);
        assert!(matches!(
            listed(Platform::GitHub, 200, b"{}").err(),
            Some(PlatformError::Unreadable { .. })
        ));
    }

    /// What a token can read is listed with it, one page at most, and
    /// read as addresses with their visibility; a full page says there
    /// was more, and a token the platform does not accept is refused.
    #[test]
    fn what_a_token_can_read_is_listed_and_read() {
        let asking = readable(
            Platform::GitHub,
            &Secret::new("github_pat_not_a_real_token".to_owned()),
        );
        assert_eq!(asking.method, "GET");
        assert_eq!(asking.url, "https://api.github.com/user/repos?per_page=100");
        assert_eq!(bearer(&asking), "github_pat_not_a_real_token");
        assert_eq!(
            Call::parse(&asking),
            Some(Call::Readable {
                platform: Platform::GitHub
            })
        );
        let read = readable_listed(
            Platform::GitHub,
            200,
            br#"[{"html_url":"https://github.com/example/a","private":true},{"html_url":"https://github.com/example/b","private":false},{"html_url":"not-an-address"}]"#,
        )
        .expect("a listing");
        assert_eq!(
            read.repositories
                .iter()
                .map(|repository| (repository.address.https(), repository.private))
                .collect::<Vec<_>>(),
            [
                ("https://github.com/example/a".to_owned(), true),
                ("https://github.com/example/b".to_owned(), false)
            ]
        );
        assert!(!read.more, "two listed, of a page of a hundred");
        let page: Vec<String> = (0..100)
            .map(|n| format!(r#"{{"html_url":"https://github.com/example/r{n}","private":false}}"#))
            .collect();
        let full = readable_listed(
            Platform::GitHub,
            200,
            format!("[{}]", page.join(",")).as_bytes(),
        )
        .expect("a full page");
        assert!(full.more, "a full page says there was more");
        assert_eq!(
            readable_listed(Platform::GitHub, 401, br#"{"message":"Bad credentials"}"#),
            Err(PlatformError::Refused {
                platform: Platform::GitHub
            })
        );
        assert!(matches!(
            readable_listed(Platform::GitHub, 200, b"{}").err(),
            Some(PlatformError::Unreadable { .. })
        ));
    }

    /// Where a person installs the App and where they come back to, and
    /// what the App's own account is called.
    /// Whose a token is, and when it expires: the read of the account,
    /// and its answer with the expiry header spelled as the platform was
    /// measured to spell it — absent for a token that does not expire,
    /// and refused where this cannot read it — see
    /// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    #[test]
    fn whose_a_token_is_and_when_it_expires_are_read_off_the_account() {
        let asking = owner(
            Platform::GitHub,
            &Secret::new("github_pat_not_a_real_token".to_owned()),
        );
        assert_eq!(asking.method, "GET");
        assert_eq!(asking.url, "https://api.github.com/user");
        assert_eq!(
            asking.headers.get("authorization").map(String::as_str),
            Some("Bearer github_pat_not_a_real_token")
        );
        let body = br#"{"login":"HernanFdz","id":1}"#;
        let spelled = |expiry: Option<&str>| {
            expiry
                .map(|spelled| {
                    (
                        "github-authentication-token-expiration".to_owned(),
                        spelled.to_owned(),
                    )
                })
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let read = owned(
            Platform::GitHub,
            200,
            &spelled(Some("2026-09-27 08:42:20 UTC")),
            body,
        )
        .expect("read");
        assert_eq!(read.login, "HernanFdz");
        assert_eq!(
            read.expires.map(|at| at.to_string()),
            Some("2026-09-27T08:42:20Z".to_owned()),
            "the header's spelling, read as the moment it names"
        );
        assert_eq!(
            owned(Platform::GitHub, 200, &spelled(None), body)
                .expect("read")
                .expires,
            None,
            "no header is no expiry"
        );
        assert!(matches!(
            owned(Platform::GitHub, 200, &spelled(Some("tomorrow")), body),
            Err(PlatformError::Unreadable { .. })
        ));
        assert!(matches!(
            owned(Platform::GitHub, 200, &spelled(None), br#"{"id":1}"#),
            Err(PlatformError::Unreadable { .. })
        ));
        assert!(matches!(
            owned(Platform::GitHub, 401, &spelled(None), b"{}"),
            Err(PlatformError::Refused { .. })
        ));
    }

    #[test]
    fn the_install_link_names_the_app() {
        assert_eq!(
            install_link(Platform::GitHub, "stageman-sim", "f00d"),
            "https://github.com/apps/stageman-sim/installations/new?state=f00d"
        );
        assert_eq!(
            installed_path(Platform::GitHub),
            "/instance/apps/github/installed"
        );
        assert_eq!(
            bot_name(Platform::GitHub, "stageman-sim"),
            "stageman-sim[bot]"
        );
    }

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
        assert_eq!(
            Call::parse(&other),
            Some(Call::Owner {
                platform: Platform::GitHub
            }),
            "whose a token is, asked with it"
        );
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
