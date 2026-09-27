//! The one password the dashboard is entered with, and the sessions it buys
//! — see `docs/decisions/0084-the-instance-authenticates-itself.md`.
//!
//! All of it is decided here and none of it is performed here. The slow
//! part, deriving a hash from what was typed, is an effect the world
//! performs on a thread of its own, because argon2 is slow by design and a
//! step must stay short: a flood of wrong passwords hashed inside the loop
//! would stall every turn and every page behind it. What this module does
//! is compose the derivation, compare its answer, mint a session from the
//! one generator, and decide at the door which requests a session is
//! needed for.
//!
//! **A session is held, never kept.** It lives in memory, minted from the
//! generator that mints warrants, expiring by the step's clock, gone on a
//! restart. An upgrade logs everyone out and no session ever touches the
//! disk.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use stageman_core::{JobId, Progress, Secret};
use stageman_vocabulary::{Answer, Arrival, Bytes, Effect as Generic, EffectId, Now, RequestId};

use crate::Effect;
use crate::tunnel::{address, dashboard};

/// Memory the derivation uses, in kibibytes: nineteen mebibytes, which is
/// the first of the parameter sets the function's own guidance recommends.
pub const MEMORY: u32 = 19_456;

/// Passes over that memory.
pub const ITERATIONS: u32 = 2;

/// Lanes.
pub const PARALLELISM: u32 = 1;

/// How many bytes a hash is.
pub const LENGTH: u32 = 32;

/// How many bytes of salt a password is hashed with.
pub const SALT_LEN: usize = 16;

/// How long a session lasts from its last use: a fortnight.
///
/// Sliding rather than fixed, so that somebody who uses the dashboard every
/// day is never signed out, and somebody who stops is, in time.
pub const SESSION_LIFETIME: Now = 14 * 24 * 60 * 60 * 1000;

/// Where the login page is, and where its form posts.
pub const LOGIN_PATH: &str = "/login";

/// The one path that answers without a session: whether the instance is
/// up, for whatever supervises it, and nothing else.
pub const UP_PATH: &str = "/up";

/// Where the apex grants entry to a job's host, the job's name after it.
///
/// The one place a session for a job's host is bought, since a cookie sent
/// to one host is not sent to another — see
/// `docs/decisions/0084-the-instance-authenticates-itself.md`.
pub const ENTER_PREFIX: &str = "/enter/";

/// Where a job's host takes the grant the apex minted and sets its own
/// cookie.
///
/// A path of the instance's own on a host that otherwise serves whatever a
/// job put there, dotted so that nothing a job serves is likely to sit
/// under it.
pub const ENTRY_PATH: &str = "/.stageman/enter";

/// How long a grant is good for, in milliseconds.
///
/// Long enough for one redirect to land, and no longer, since it travels
/// in an address.
pub const GRANT_LIFETIME: Now = 60_000;

/// What the framework serves that the login page needs: its assets, and the
/// browser's half. Two prefixes, both the framework's, so that a session is
/// not needed to fetch the stylesheet the login page is drawn with or the
/// script that wakes it — measured: a bundle asked for through the gate was
/// answered with the login page, which a browser refuses as a module.
const FRAMEWORK_PREFIXES: [&str; 2] = ["/assets/", "/wasm/"];

/// What a page reads through, answered with a refusal rather than a
/// redirect when there is no session: a page that outlived its session
/// shows an error rather than following a redirect into a login page it
/// cannot parse.
const API_PREFIX: &str = "/api/";

/// The most a login form may be, which is a password and where to go back.
const FORM_LIMIT: usize = 4096;

/// A password's hash, as the instance spells it for the file: which
/// function and parameters, the salt, and the bytes.
///
/// Spelled by this crate and parsed by this crate, so that a change of
/// parameters later verifies an older hash with the parameters it was made
/// with rather than with today's. The function is named in the spelling
/// for the same reason, though only one is ever used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashed {
    /// Memory, in kibibytes.
    pub memory: u32,
    /// Passes.
    pub iterations: u32,
    /// Lanes.
    pub parallelism: u32,
    /// The salt.
    pub salt: Vec<u8>,
    /// The bytes derived.
    pub hash: Vec<u8>,
}

impl Hashed {
    /// Spells this for the file: `argon2id$m$t$p$salt$hash`, the last two
    /// in hex.
    #[must_use]
    pub fn spell(&self) -> String {
        format!(
            "argon2id${}${}${}${}${}",
            self.memory,
            self.iterations,
            self.parallelism,
            hex(&self.salt),
            hex(&self.hash)
        )
    }

    /// Reads a spelling back, or nothing for one this crate did not write.
    #[must_use]
    pub fn parse(spelled: &str) -> Option<Self> {
        let mut parts = spelled.split('$');
        if parts.next()? != "argon2id" {
            return None;
        }
        let memory = parts.next()?.parse().ok()?;
        let iterations = parts.next()?.parse().ok()?;
        let parallelism = parts.next()?.parse().ok()?;
        let salt = unhex(parts.next()?)?;
        let hash = unhex(parts.next()?)?;
        if parts.next().is_some() || salt.is_empty() || hash.is_empty() {
            return None;
        }
        Some(Self {
            memory,
            iterations,
            parallelism,
            salt,
            hash,
        })
    }

    /// The derivation that checks a typed password against this hash, with
    /// the parameters this hash was made with; nothing for a hash too long
    /// to ask for, which no spelling of this crate's produces.
    fn checking(&self, id: EffectId, typed: &str) -> Option<Effect> {
        let length = u32::try_from(self.hash.len()).ok()?;
        Some(Generic::Derive {
            id,
            secret: Bytes::new(typed.as_bytes().to_vec()),
            salt: Bytes::new(self.salt.clone()),
            memory: self.memory,
            iterations: self.iterations,
            parallelism: self.parallelism,
            length,
        })
    }
}

/// The derivation that makes a new hash from a password and a fresh salt,
/// with today's parameters.
#[must_use]
pub fn derivation(id: EffectId, password: &str, salt: &[u8]) -> Effect {
    Generic::Derive {
        id,
        secret: Bytes::new(password.as_bytes().to_vec()),
        salt: Bytes::new(salt.to_vec()),
        memory: MEMORY,
        iterations: ITERATIONS,
        parallelism: PARALLELISM,
        length: LENGTH,
    }
}

/// What a derivation made with today's parameters spells as, once its
/// bytes are back.
#[must_use]
pub fn spelled(salt: &[u8], hash: &[u8]) -> Secret {
    Secret::new(
        Hashed {
            memory: MEMORY,
            iterations: ITERATIONS,
            parallelism: PARALLELISM,
            salt: salt.to_vec(),
            hash: hash.to_vec(),
        }
        .spell(),
    )
}

/// A login whose form is being read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    /// Who is logging in, as an address.
    pub peer: String,
}

/// A typed password being checked against the hash on file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deriving {
    /// The request held for the answer.
    pub request: RequestId,
    /// Who typed it.
    pub peer: String,
    /// Where to send them once it is right.
    pub back: Option<String>,
    /// What the derivation has to come out as.
    pub expected: Vec<u8>,
}

/// A session for one job's host, bought through the apex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entered {
    /// Whose host.
    pub job: JobId,
    /// When it lapses unless used before then.
    pub expires: Now,
}

/// A grant the apex minted for one job's host, until the host takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Whose host.
    pub job: JobId,
    /// Where the person was going on it.
    pub back: Option<String>,
    /// When it is no longer good.
    pub expires: Now,
}

/// What wrong passwords from one address have earned it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failing {
    /// How many in a row.
    pub count: u32,
    /// Until when the next attempt is refused without being checked.
    pub until: Now,
}

/// The cookie a session travels in.
///
/// Prefixed and Secure where the domain is one a certificate stands in
/// front of, so that no page on a job's host can set one the apex
/// receives; plain on a local domain, where there is no certificate for
/// Secure to hold under and nothing between the browser and this process.
#[must_use]
pub const fn cookie_name(local: bool) -> &'static str {
    if local {
        "stageman_session"
    } else {
        "__Host-stageman_session"
    }
}

/// The cookie a job's host is entered with: a session of its own, since a
/// cookie sent to one host is not sent to another, prefixed and Secure on
/// the same terms as the apex's.
#[must_use]
pub const fn tunnel_cookie_name(local: bool) -> &'static str {
    if local {
        "stageman_tunnel"
    } else {
        "__Host-stageman_tunnel"
    }
}

/// Every cookie name of this instance's, in both spellings, which is what a
/// request forwarded to a container is stripped of: what a person presented
/// to this instance is this instance's, and never a job's to read.
#[must_use]
pub fn stripped() -> Vec<String> {
    [
        cookie_name(true),
        cookie_name(false),
        tunnel_cookie_name(true),
        tunnel_cookie_name(false),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The header that sets a cookie for one session, on this host alone.
fn set_cookie(name: &str, local: bool, session: &str) -> String {
    let secure = if local { "" } else { "; Secure" };
    format!("{name}={session}; Path=/; HttpOnly; SameSite=Lax{secure}")
}

/// The cookie a request presents under a name, if it presents one.
fn presented<'a>(request: &'a Arrival, name: &str) -> Option<&'a str> {
    request
        .headers
        .get("cookie")?
        .split(';')
        .map(str::trim)
        .filter_map(|pair| pair.split_once('='))
        .find(|(found, _)| *found == name)
        .map(|(_, value)| value.trim())
}

/// Where a request was going, as the login page is told to send the
/// browser back afterwards: its path and query, or nowhere for a request
/// that was for the login page itself.
fn back_of(request: &Arrival) -> Option<String> {
    let (path, _) = request.path.split_once('?').unwrap_or((&request.path, ""));
    (path != LOGIN_PATH).then(|| request.path.clone())
}

/// Where to send a browser after a login: where it was going if that is a
/// path on this host, and the front page otherwise. A path is one that
/// starts with one slash — two is another host's address in a browser's
/// reading, which is the one thing a value somebody typed must not become.
fn destination(back: Option<&str>) -> String {
    match back {
        Some(back) if back.starts_with('/') && !back.starts_with("//") => back.to_owned(),
        _ => "/".to_owned(),
    }
}

/// The login page, with what it is to say and where to go back.
fn login_page(said: Option<&str>, back: Option<&str>) -> String {
    let mut page = LOGIN_PATH.to_owned();
    let mut separator = '?';
    if let Some(said) = said {
        let _ = write!(page, "{separator}said={said}");
        separator = '&';
    }
    if let Some(back) = back {
        let _ = write!(page, "{separator}back={}", percent_encoded(back));
    }
    page
}

/// One field of a form posted as the browser posts one, decoded.
fn form_field(body: &str, name: &str) -> Option<String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(found, _)| *found == name)
        .map(|(_, value)| percent_decoded(value))
}

/// Text as a browser encodes a form field: pluses are spaces and percent
/// escapes are bytes. A malformed escape is kept as it was, since refusing
/// a password for its spelling is the one thing this must not do.
fn percent_decoded(encoded: &str) -> String {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        rest = after;
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let escaped = after.split_at_checked(2).and_then(|(pair, beyond)| {
                    let pair = std::str::from_utf8(pair).ok()?;
                    Some((u8::from_str_radix(pair, 16).ok()?, beyond))
                });
                match escaped {
                    Some((decoded, beyond)) => {
                        bytes.push(decoded);
                        rest = beyond;
                    }
                    None => bytes.push(b'%'),
                }
            }
            _ => bytes.push(byte),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Text as it goes into a query: everything but the unreserved characters
/// escaped, so that a path with a query of its own survives being carried
/// in another.
fn percent_encoded(plain: &str) -> String {
    let mut encoded = String::with_capacity(plain.len());
    for byte in plain.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Bytes as hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// Hex as bytes, or nothing for text that is not hex.
fn unhex(hex: &str) -> Option<Vec<u8>> {
    let pairs = hex.as_bytes().chunks_exact(2);
    if !pairs.remainder().is_empty() {
        return None;
    }
    pairs
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// Whether two byte strings are the same, in time that depends on their
/// length and on nothing else, so that a comparison's timing says nothing
/// about how much of a guess was right.
fn same(one: &[u8], other: &[u8]) -> bool {
    if one.len() != other.len() {
        return false;
    }
    one.iter()
        .zip(other)
        .fold(0_u8, |differ, (a, b)| differ | (a ^ b))
        == 0
}

/// How long a next attempt is refused for after this many wrong ones in a
/// row, in milliseconds: doubling from a second, and no longer than the
/// last entry, which is where a count past the table lands.
fn wait_after(count: u32) -> Now {
    const SCHEDULE: [Now; 8] = [1_000, 1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 64_000];
    usize::try_from(count)
        .ok()
        .and_then(|count| SCHEDULE.get(count).copied())
        .unwrap_or(64_000) // CLAMP-OK: past the table is the longest wait, by design
}

/// When something that lasts a lifetime from now lapses.
const fn lapsing(now: Now) -> Now {
    now.saturating_add(SESSION_LIFETIME) // CLAMP-OK: past the end of time is the end of time
}

/// The address a request came from, without its port: what a wrong
/// password is counted against.
fn address_of(peer: &str) -> String {
    peer.rsplit_once(':')
        .map_or(peer, |(address, _)| address)
        .trim_matches(['[', ']'])
        .to_owned()
}

impl crate::Running {
    /// Decides at the door whether a request on the dashboard's host needs
    /// a session, and answers it here when it is the login's own or has no
    /// session to show. False means the request goes on to what the door
    /// did before: the instance's own paths, and then the pages.
    pub fn gate(&mut self, id: RequestId, request: &Arrival, effects: &mut Vec<Effect>) -> bool {
        let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
        let local = self.domain.is_local();
        match (request.method.as_str(), path) {
            ("GET" | "HEAD", UP_PATH) => {
                Self::answer_now(id, 200, &[], "up\n", effects);
                true
            }
            ("POST", LOGIN_PATH) => {
                self.logins.insert(
                    id,
                    Login {
                        peer: request.peer.clone(),
                    },
                );
                effects.push(Effect::Answer {
                    id,
                    answer: Answer::Read { limit: FORM_LIMIT },
                });
                true
            }
            ("GET", LOGIN_PATH) => {
                self.pages(id, effects);
                true
            }
            _ if FRAMEWORK_PREFIXES
                .iter()
                .any(|prefix| path.starts_with(prefix)) =>
            {
                self.pages(id, effects);
                true
            }
            // No password set means no door yet: the first run sets one, and
            // until then the dashboard is what it always was.
            _ if self.state.password.is_none() => false,
            _ if !self.session_presented(request, local) => {
                if path.starts_with(API_PREFIX) {
                    Self::answer_now(id, 401, &[], "Sign in to the dashboard first.\n", effects);
                } else {
                    let page = login_page(None, back_of(request).as_deref());
                    Self::answer_now(id, 303, &[("location", &page)], "", effects);
                }
                true
            }
            _ if let Some(named) = path.strip_prefix(ENTER_PREFIX) => {
                self.entry_granted(id, named, query, effects);
                true
            }
            _ => false,
        }
    }

    /// Somebody signed in on the apex asked to enter a job's host: a grant
    /// is minted for that host, good for one redirect, and the browser is
    /// sent there with it.
    fn entry_granted(
        &mut self,
        id: RequestId,
        named: &str,
        query: &str,
        effects: &mut Vec<Effect>,
    ) {
        let job = JobId::parse(named).ok();
        let showing = job
            .as_ref()
            .and_then(|job| self.state.job(job))
            .is_some_and(|recorded| !matches!(recorded.progress, Progress::Retired(_)));
        let (Some(job), true) = (job, showing) else {
            Self::answer_now(id, 404, &[], "No job answers on this address.\n", effects);
            return;
        };
        let back = form_field(query, "back").filter(|back| !back.is_empty());
        self.grants.retain(|_, grant| grant.expires > self.now);
        let token = self.unguessable();
        self.grants.insert(
            token.clone(),
            Grant {
                job: job.clone(),
                back,
                expires: self.now.saturating_add(GRANT_LIFETIME), // CLAMP-OK: past the end of time is the end of time
            },
        );
        let entry = format!(
            "{}{ENTRY_PATH}?t={token}",
            address(&self.domain, &job, self.serving)
        );
        Self::answer_now(id, 303, &[("location", &entry)], "", effects);
    }

    /// Decides at a job's host whether a request may go on to the tunnel:
    /// with a session for that host, yes; at the entry path, with a grant
    /// the apex minted for that host, a session is set and the browser sent
    /// on; otherwise the browser is sent to the apex to be granted one.
    /// False means the request goes on to the tunnel.
    pub fn gate_tunnel(
        &mut self,
        id: RequestId,
        job: &JobId,
        request: &Arrival,
        effects: &mut Vec<Effect>,
    ) -> bool {
        if self.state.password.is_none() {
            return false;
        }
        let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
        let local = self.domain.is_local();
        if path == ENTRY_PATH {
            self.grants.retain(|_, grant| grant.expires > self.now);
            let token = form_field(query, "t").unwrap_or_default();
            let granted = self.grants.remove(&token).filter(|grant| grant.job == *job);
            let Some(grant) = granted else {
                // Spent, lapsed, or somebody else's: the apex mints another,
                // and the person notices nothing but a moment.
                let again = self.entry_of(job, None);
                Self::answer_now(id, 303, &[("location", &again)], "", effects);
                return true;
            };
            let session = self.unguessable();
            self.entered.insert(
                session.clone(),
                Entered {
                    job: job.clone(),
                    expires: lapsing(self.now),
                },
            );
            let cookie = set_cookie(tunnel_cookie_name(local), local, &session);
            let page = destination(grant.back.as_deref());
            Self::answer_now(
                id,
                303,
                &[("location", &page), ("set-cookie", &cookie)],
                "",
                effects,
            );
            return true;
        }
        if self.tunnel_presented(request, job, local) {
            return false;
        }
        let entry = self.entry_of(job, Some(&request.path));
        Self::answer_now(id, 303, &[("location", &entry)], "", effects);
        true
    }

    /// Where a browser is sent to be granted entry to a job's host: the
    /// apex, which is where the session is.
    fn entry_of(&self, job: &JobId, back: Option<&str>) -> String {
        let mut entry = format!(
            "{}{ENTER_PREFIX}{job}",
            dashboard(&self.domain, self.reached)
        );
        if let Some(back) = back {
            let _ = write!(entry, "?back={}", percent_encoded(back));
        }
        entry
    }

    /// Whether the request carries a session for this job's host that has
    /// not lapsed, renewing it when it does.
    fn tunnel_presented(&mut self, request: &Arrival, job: &JobId, local: bool) -> bool {
        let Some(session) = presented(request, tunnel_cookie_name(local)) else {
            return false;
        };
        let Some(entered) = self.entered.get_mut(session) else {
            return false;
        };
        if entered.expires <= self.now || entered.job != *job {
            return false;
        }
        entered.expires = lapsing(self.now);
        true
    }

    /// Whether the request carries a session this instance minted and has
    /// not yet let lapse. A session that is presented is renewed, so that
    /// its lifetime runs from its last use.
    fn session_presented(&mut self, request: &Arrival, local: bool) -> bool {
        let Some(session) = presented(request, cookie_name(local)) else {
            return false;
        };
        let Some(expires) = self.sessions.get_mut(session) else {
            return false;
        };
        if *expires <= self.now {
            self.sessions.remove(session);
            return false;
        }
        *expires = lapsing(self.now);
        true
    }

    /// The login form arrived, or could not be read.
    ///
    /// Nothing is compared here: the typed password is handed to the world
    /// to derive with the parameters and salt the hash on file was made
    /// with, and [`Self::derived`] compares once it is back. An address
    /// that has earned a wait is refused before anything is derived, which
    /// is what keeps a flood cheap.
    pub fn login_read(
        &mut self,
        id: RequestId,
        outcome: &Result<Bytes, String>,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(login) = self.logins.remove(&id) else {
            return false;
        };
        let body = match outcome {
            Ok(body) => body.as_text().unwrap_or("").to_owned(),
            Err(why) => {
                tracing::warn!(%why, "a login's form could not be read");
                Self::answer_now(id, 400, &[], "The form could not be read.\n", effects);
                return true;
            }
        };
        let typed = form_field(&body, "password").unwrap_or_default();
        let back = form_field(&body, "back").filter(|back| !back.is_empty());
        let address = address_of(&login.peer);
        self.failures.retain(|_, failing| failing.until > self.now);
        if self.failures.contains_key(&address) {
            let page = login_page(Some("wait"), back.as_deref());
            Self::answer_now(id, 303, &[("location", &page)], "", effects);
            return true;
        }
        let Some(on_file) = self.state.password.as_ref() else {
            // Nothing to check against: the door is open, and the form was
            // reached by somebody who typed the address by hand.
            let page = destination(back.as_deref());
            Self::answer_now(id, 303, &[("location", &page)], "", effects);
            return true;
        };
        let Some(hashed) = Hashed::parse(on_file.expose()) else {
            tracing::error!(
                "the password on file is not spelled as this instance spells one, so nobody can \
                 sign in until it is set again"
            );
            Self::answer_now(
                id,
                500,
                &[],
                "The password on file cannot be read.\n",
                effects,
            );
            return true;
        };
        if typed.is_empty() {
            self.failed(&address);
            let page = login_page(Some("wrong"), back.as_deref());
            Self::answer_now(id, 303, &[("location", &page)], "", effects);
            return true;
        }
        let effect = self.effect_id();
        let Some(checking) = hashed.checking(effect, &typed) else {
            tracing::error!("the password on file is too long to check against");
            Self::answer_now(
                id,
                500,
                &[],
                "The password on file cannot be read.\n",
                effects,
            );
            return true;
        };
        effects.push(checking);
        self.deriving.insert(
            effect,
            Deriving {
                request: id,
                peer: login.peer,
                back,
                expected: hashed.hash,
            },
        );
        true
    }

    /// The world derived what a typed password hashes to.
    pub fn derived(
        &mut self,
        id: EffectId,
        derived: &Result<Bytes, String>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(deriving) = self.deriving.remove(&id) else {
            tracing::warn!("a derivation was answered that nobody was waiting on; ignored");
            return;
        };
        let address = address_of(&deriving.peer);
        match derived {
            Ok(bytes) if same(bytes.as_slice(), &deriving.expected) => {
                self.failures.remove(&address);
                let session = self.unguessable();
                self.sessions.insert(session.clone(), lapsing(self.now));
                let local = self.domain.is_local();
                let cookie = set_cookie(cookie_name(local), local, &session);
                let page = destination(deriving.back.as_deref());
                Self::answer_now(
                    deriving.request,
                    303,
                    &[("location", &page), ("set-cookie", &cookie)],
                    "",
                    effects,
                );
            }
            Ok(_) => {
                self.failed(&address);
                let page = login_page(Some("wrong"), deriving.back.as_deref());
                Self::answer_now(deriving.request, 303, &[("location", &page)], "", effects);
            }
            Err(why) => {
                tracing::error!(%why, "a typed password could not be hashed to check it");
                Self::answer_now(
                    deriving.request,
                    500,
                    &[],
                    "The password could not be checked.\n",
                    effects,
                );
            }
        }
    }

    /// Counts a wrong password against an address, doubling the wait.
    fn failed(&mut self, address: &str) {
        let before = self
            .failures
            .get(address)
            .map_or(0, |failing| failing.count);
        let count = before.saturating_add(1); // CLAMP-OK: a count that cannot grow stays at the longest wait
        let until = self.now.saturating_add(wait_after(count)); // CLAMP-OK: past the end of time is the end of time
        self.failures
            .insert(address.to_owned(), Failing { count, until });
    }

    /// Answers a request at once, since nothing kept changed.
    fn answer_now(
        id: RequestId,
        status: u16,
        headers: &[(&str, &str)],
        body: &str,
        effects: &mut Vec<Effect>,
    ) {
        let mut sent: BTreeMap<String, String> = headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        if !body.is_empty() {
            sent.insert("content-type".to_owned(), "text/plain".to_owned());
        }
        effects.push(Effect::Answer {
            id,
            answer: Answer::Respond {
                status,
                headers: sent,
                body: Bytes::new(body.as_bytes().to_vec()),
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Hashed, back_of, cookie_name, destination, form_field, login_page, percent_decoded,
        percent_encoded, same, set_cookie, wait_after,
    };
    use stageman_vocabulary::Arrival;

    fn arrival(path: &str, cookie: Option<&str>) -> Arrival {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert("host".to_owned(), "localhost".to_owned());
        if let Some(cookie) = cookie {
            headers.insert("cookie".to_owned(), cookie.to_owned());
        }
        Arrival {
            method: "GET".to_owned(),
            path: path.to_owned(),
            headers,
            peer: "127.0.0.1:50000".to_owned(),
        }
    }

    #[test]
    fn a_hash_spells_and_parses_back() {
        let hashed = Hashed {
            memory: 19_456,
            iterations: 2,
            parallelism: 1,
            salt: vec![0, 1, 254, 255],
            hash: vec![7; 32],
        };
        let spelled = hashed.spell();
        assert!(
            spelled.starts_with("argon2id$19456$2$1$0001feff$0707"),
            "{spelled}"
        );
        assert_eq!(Hashed::parse(&spelled), Some(hashed));
        for wrong in [
            "",
            "bcrypt$1$2$3$00$00",
            "argon2id$x$2$1$00$00",
            "argon2id$1$2$1$0$00",
            "argon2id$1$2$1$zz$00",
            "argon2id$1$2$1$00$",
            "argon2id$1$2$1$00$00$more",
        ] {
            assert_eq!(Hashed::parse(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn a_form_is_decoded_as_a_browser_encodes_one() {
        let body = "password=correct+horse%26battery&back=%2Fprojects%3Fopen%3D1";
        assert_eq!(
            form_field(body, "password").as_deref(),
            Some("correct horse&battery")
        );
        assert_eq!(
            form_field(body, "back").as_deref(),
            Some("/projects?open=1")
        );
        assert_eq!(form_field(body, "other"), None);
        assert_eq!(percent_decoded("a%zz%2"), "a%zz%2");
        assert_eq!(
            percent_decoded(&percent_encoded("/a b?c=d&e")),
            "/a b?c=d&e"
        );
    }

    #[test]
    fn the_login_page_carries_what_it_is_told_and_where_to_go_back() {
        assert_eq!(login_page(None, None), "/login");
        assert_eq!(login_page(Some("wrong"), None), "/login?said=wrong");
        assert_eq!(
            login_page(Some("wait"), Some("/projects?open=1")),
            "/login?said=wait&back=%2Fprojects%3Fopen%3D1"
        );
        assert_eq!(login_page(None, Some("/")), "/login?back=%2F");
    }

    #[test]
    fn where_to_go_back_is_a_path_on_this_host_or_the_front_page() {
        assert_eq!(destination(Some("/projects")), "/projects");
        assert_eq!(destination(Some("//evil.example/")), "/");
        assert_eq!(destination(Some("https://evil.example/")), "/");
        assert_eq!(destination(None), "/");
        assert_eq!(
            back_of(&arrival("/projects?open=1", None)).as_deref(),
            Some("/projects?open=1")
        );
        assert_eq!(back_of(&arrival("/login?said=wrong", None)), None);
    }

    #[test]
    fn the_cookie_is_prefixed_and_secure_where_a_certificate_stands_in_front() {
        assert_eq!(cookie_name(true), "stageman_session");
        assert_eq!(cookie_name(false), "__Host-stageman_session");
        assert_eq!(
            set_cookie(cookie_name(true), true, "abc"),
            "stageman_session=abc; Path=/; HttpOnly; SameSite=Lax"
        );
        assert_eq!(
            set_cookie(cookie_name(false), false, "abc"),
            "__Host-stageman_session=abc; Path=/; HttpOnly; SameSite=Lax; Secure"
        );
        let several = arrival("/", Some("other=1; stageman_session=abc ; more=2"));
        assert_eq!(super::presented(&several, cookie_name(true)), Some("abc"));
        assert_eq!(
            super::presented(&arrival("/", None), cookie_name(true)),
            None
        );
        assert_eq!(
            super::presented(
                &arrival("/", Some("stageman_session=abc")),
                cookie_name(false)
            ),
            None,
            "a plain cookie is not the prefixed one"
        );
        assert_eq!(super::tunnel_cookie_name(true), "stageman_tunnel");
        assert_eq!(super::tunnel_cookie_name(false), "__Host-stageman_tunnel");
        assert_eq!(
            super::stripped().len(),
            4,
            "every name of this instance's, both spellings"
        );
    }

    #[test]
    fn the_wait_doubles_from_a_second_and_stops_growing() {
        assert_eq!(wait_after(1), 1_000);
        assert_eq!(wait_after(2), 2_000);
        assert_eq!(wait_after(7), 64_000);
        assert_eq!(wait_after(8), 64_000);
        assert_eq!(wait_after(200), 64_000);
        assert_eq!(wait_after(u32::MAX), 64_000);
    }

    #[test]
    fn sameness_is_by_every_byte() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
        assert!(same(b"", b""));
        // Two differences that would cancel under a fold that mixed rather
        // than accumulated: swapped bytes, and a pair of bit flips.
        assert!(!same(b"ab", b"ba"));
        assert!(!same(&[1, 2], &[2, 1]));
    }
}
