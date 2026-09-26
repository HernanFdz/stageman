//! The domain: what a project is, what a job is, and the states a job moves
//! through.
//!
//! No I/O, no async runtime, no platform and no framework. This crate names
//! nothing else in the workspace, which is what lets both of the crates above
//! it depend on it without depending on each other — see
//! `docs/architecture.md` §1 for the rule and why it is the one worth
//! defending.
//!
//! The vocabulary this crate exists to express is fixed in
//! `docs/conventions.md` §2, including the words it deliberately avoids.
//!
//! One thing here looks like plumbing and is not: deciding what an agent
//! process is handed. It lives in a crate with no I/O because it is a pure
//! function from configuration to a description of what that process should
//! see, and because it is the only thing standing between an operator and
//! silently paying the wrong way — which makes being able to test it without
//! spawning a process the whole point rather than a convenience. Delivering
//! that description is an adapter's job, and differs per agent.
//!
//! **Nothing here reads a clock, mints an identifier, or generates a nonce.**
//! All three are effects, and all three would make values non-deterministic to
//! construct, so all three are supplied by the caller — which is why creating a
//! job takes a timestamp rather than asking the operating system for one. The
//! crates that are allowed effects do that; this one stays a set of values a
//! test can build exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key as CipherKey, Nonce as CipherNonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

// Re-exported because both appear in this crate's public signatures: a caller
// building a job needs a timestamp and an identifier, and making it depend on
// these crates separately would make matching their versions its problem.
pub use jiff::Timestamp;
pub use uuid::Uuid;

/// Bytes of the nonce a single sealing operation consumes.
pub const NONCE_LEN: usize = 12;

/// A nonce: unique per sealing operation, never reused under one key.
pub type Nonce = [u8; NONCE_LEN];

/// A credential, in memory.
///
/// Formatting is redacted in both `Debug` and `Display`, because the usual way
/// a token reaches a log is a structure printed whole while somebody is
/// debugging something else entirely.
///
/// **It deliberately does not implement serialisation.** State is persisted by
/// converting it to a separate sealed form, and a `Serialize` here would let
/// this type reach a file in the clear by accident. The bar is in
/// `docs/conventions.md` §4.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a credential.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Yields the credential in the clear.
    ///
    /// Named for what it does rather than for what it returns, so that every
    /// call site reads as a decision someone made instead of an accessor
    /// nobody noticed.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Identifies a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProjectId(Uuid);

/// Identifies a job, and names it.
///
/// One string, and it is the job's name wherever the job is met: the key
/// here, the segment of its page's address, the suffix of its container's
/// name, the bottom label of its tunnel's host, and the tail of its room's
/// name — see `docs/decisions/0074-a-jobs-identifier-is-its-name.md`. It is
/// the title the job was given, folded by [`slug`] and capped at
/// [`JobId::TITLE_AT_MOST`], then [`JobId::SEPARATOR`] and
/// [`JobId::SUFFIX_CHARS`] of hex minted for it. A job the last release
/// wrote is named by its UUID, which fits the same grammar as it stands.
///
/// **Nothing parses one.** The title in it is for a reader and the hex is
/// what made it unique; the only reader of its shape is [`JobId::parse`],
/// which says whether a text is a name and never what it means.
///
/// Text rather than a UUID, so it is cloned where the UUID was copied.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct JobId(String);

impl JobId {
    /// The most a name may be, in characters: a title at its cap, the
    /// separator, and the suffix. Under a DNS label's sixty-three, and
    /// beside a project's part under a room's eighty.
    pub const AT_MOST: usize = 50;
    /// The most of a title a name carries.
    pub const TITLE_AT_MOST: usize = 40;
    /// What parts the title from the suffix: two hyphens, because a title
    /// contains single ones.
    pub const SEPARATOR: &'static str = "--";
    /// How much of the minted hex a name carries: enough that two jobs of
    /// one instance never collide in practice, short enough to read.
    pub const SUFFIX_CHARS: usize = 8;
    /// What a title with nothing left in it after folding reads as, so that
    /// every name has its two parts.
    const NAMELESS: &'static str = "job";

    /// The name of a job identified by a UUID: the shape the last release
    /// wrote, and the one tests mint from a number.
    #[must_use]
    pub fn from_uuid(value: Uuid) -> Self {
        Self(value.hyphenated().to_string())
    }

    /// A name from a title and the hex minted for it.
    ///
    /// There is no constructor that mints: doing so needs randomness, and
    /// this crate takes no effects. See the crate documentation.
    #[must_use]
    pub fn named(title: &str, minted: &Uuid) -> Self {
        let mut name = slug(title, Self::TITLE_AT_MOST);
        if name.is_empty() {
            name.push_str(Self::NAMELESS);
        }
        name.push_str(Self::SEPARATOR);
        name.extend(minted.simple().to_string().chars().take(Self::SUFFIX_CHARS));
        Self(name)
    }

    /// Reads a name, refusing anything the grammar does not allow: lowercase
    /// ASCII letters and digits in runs parted by hyphens, none at either
    /// end, and no longer than [`JobId::AT_MOST`].
    ///
    /// # Errors
    ///
    /// Says which rule the text broke.
    pub fn parse(text: &str) -> Result<Self, InvalidJobId> {
        if text.is_empty() {
            return Err(InvalidJobId::Empty);
        }
        if text.chars().count() > Self::AT_MOST {
            return Err(InvalidJobId::TooLong);
        }
        if text.starts_with('-') || text.ends_with('-') {
            return Err(InvalidJobId::HyphenAtAnEnd);
        }
        if let Some(character) = text
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            return Err(InvalidJobId::Character(character));
        }
        Ok(Self(text.to_owned()))
    }

    /// The name, as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for JobId {
    type Error = InvalidJobId;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<JobId> for String {
    fn from(name: JobId) -> Self {
        name.0
    }
}

/// Why a text is not a job's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidJobId {
    /// Nothing at all.
    #[error("a job's name cannot be empty")]
    Empty,
    /// More than a name may be.
    #[error("a job's name is at most {} characters", JobId::AT_MOST)]
    TooLong,
    /// A hyphen where a DNS label refuses one.
    #[error("a job's name neither starts nor ends with a hyphen")]
    HyphenAtAnEnd,
    /// Something outside the alphabet every place a name goes shares.
    #[error("a job's name is lowercase letters, digits and hyphens, and {0:?} is none of those")]
    Character(char),
}

/// Folds text to a piece of a name.
///
/// Lowercase, with every run of anything but an ASCII letter or digit as
/// one hyphen, no hyphen at either end, and no longer than `at_most` — cut
/// back to the last whole word that fits when the cut would land inside
/// one, and cut where it is when what is left is one word longer than the
/// cap.
///
/// The one fold for everything named here: a job, and the project's part of
/// a room's name in the channel crate — see
/// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
#[must_use]
pub fn slug(text: &str, at_most: usize) -> String {
    let mut folded = String::new();
    for character in text.chars() {
        let lowered = character.to_ascii_lowercase();
        if lowered.is_ascii_alphanumeric() {
            folded.push(lowered);
        } else if !folded.is_empty() && !folded.ends_with('-') {
            folded.push('-');
        }
    }
    // Every character kept is ASCII, so a length is a count and a cut at
    // `at_most` lands on a character boundary. Whether there is a character
    // at the cut is whether the fold is longer than the cap.
    if let Some(at_the_cut) = folded.chars().nth(at_most) {
        let inside_a_word = at_the_cut != '-';
        folded.truncate(at_most);
        if inside_a_word
            && folded.ends_with(|c: char| c != '-')
            && let Some(last_break) = folded.rfind('-')
        {
            folded.truncate(last_break);
        }
    }
    folded.trim_end_matches('-').to_owned()
}

/// Identifies one instance of this program.
///
/// **Not a fact about the work, and here anyway.** What it exists for is one
/// question a container runtime cannot answer on its own: given a container
/// carrying this project's label, did *this* instance create it? Two instances
/// sharing a daemon is the ordinary case rather than an exotic one — a
/// development instance served out of a checkout, and the real one — and
/// without this each sees the other's containers as its own abandoned work.
///
/// It travels with the snapshot rather than with the machine, which is the
/// opposite of the container runtime's path and for the opposite reason: a
/// copied instance file is the same instance, and a second daemon on another
/// machine has none of its containers to confuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InstanceId(Uuid);

macro_rules! identifier {
    ($name:ident) => {
        impl $name {
            /// Wraps an identifier minted elsewhere.
            ///
            /// There is no constructor that generates one: doing so needs
            /// randomness, and this crate takes no effects. See the crate
            /// documentation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Borrows the underlying identifier.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

identifier!(ProjectId);
identifier!(InstanceId);

/// A coding agent this project knows how to run.
///
/// A closed set rather than a list an operator can extend, because every agent
/// needs an adapter and an image, both of which are code — so the set of
/// supportable agents was always bounded by what is compiled in, and a value
/// an operator could invent only postponed the failure to runtime. Adding one
/// is a compile error everywhere it is not yet handled, which is the point.
///
/// Naming the set here is not the same as being specific to one, which
/// `docs/decisions/0006-agents-are-pluggable.md` forbids outside an adapter:
/// this crate knows *which* agents exist, and adapters know how each behaves.
///
/// A job stores this **by value**, never as a reference into configuration, so
/// that removing an agent's configuration cannot rewrite the history of jobs
/// that used it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Agent {
    /// Anthropic's coding agent.
    Claude,
}

impl Agent {
    /// Every agent that exists.
    ///
    /// The one place adding a variant has to be updated by hand, and worth
    /// being explicit about why that is tolerable: every *behavioural* site is
    /// a match and so fails to compile until the new agent is handled, which is
    /// the property the closed set exists for. A list is not a match, so
    /// forgetting this one costs a missing menu entry rather than a wrong
    /// answer — the cheapest failure of the set, and the only one available
    /// without a derive this crate would otherwise have no use for.
    pub const ALL: &'static [Self] = &[Self::Claude];

    /// What to call this agent, for a person.
    ///
    /// In the domain rather than only on a screen because the domain needs it
    /// once: a project written before kits existed named agents, and each
    /// becomes a kit named after its agent when that file is opened — see
    /// [`KitConfig::defaults`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
        }
    }

    /// What this agent is good for, in prose.
    ///
    /// Not decoration and not operator-editable: it describes the agent rather
    /// than the installation, so it lives in code — see
    /// `docs/decisions/0006-agents-are-pluggable.md`. What the foreman reasons
    /// over when it chooses is a kit's description, which is the operator's
    /// and describes what one project wants that kit for; this is what a kit
    /// made of an agent's defaults says until somebody writes that.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Claude => {
                "General-purpose coding agent. Reads a repository, makes changes \
                 across files, runs commands, and explains what it did."
            }
        }
    }
}

/// What an operator supplies in order to run an agent.
///
/// A credential and nothing else. There is deliberately no path here: agents
/// run in containers built with them already installed, so where the program
/// lives is decided by an image rather than by the machine this happens to run
/// on — see `docs/decisions/0012-agents-run-in-containers.md`.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// What the agent authenticates with.
    ///
    /// One credential per agent, never one per role — the foreman and a
    /// job running the same agent use the same one. See
    /// `docs/decisions/0008-one-credential-per-agent.md`.
    pub auth_token: Secret,
}

/// One agent, set the way one job runs it.
///
/// The variants are the agents and each payload is that agent's own settings,
/// so a job holding settings for an agent other than the one it ran on is not
/// a state to check for but a sentence that cannot be written — there is no
/// second field to disagree with the first. See
/// `docs/decisions/0048-a-job-runs-on-a-kit.md`. This crate owns the *shape*
/// of each agent's settings and the adapter owns their *spelling* on the wire,
/// which is the seam [`Handout`] already draws for credentials: deciding is a
/// pure question about configuration, and what a value is called is knowledge
/// about one agent.
///
/// Each payload is a closed set because the adapter it is spelled for is
/// pinned in the image compiled into the binary, so what that adapter accepts
/// is a fact about this build rather than about the world. The adapter crate
/// holds every spelling here to what the pinned adapter accepts, in a test
/// that runs a container, so a pin bump that removes or renames a value fails
/// there instead of rotting.
///
/// Read back through [`Job`], which also accepts the bare agent name every job
/// recorded before kits existed was written with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kit {
    /// Anthropic's coding agent, on one of the models its adapter offers.
    Claude {
        /// Which model, and — where the model has one — how hard it thinks.
        model: ClaudeModel,
    },
}

impl Kit {
    /// An agent exactly as it comes: every setting left to the agent's own
    /// default.
    ///
    /// What every job ran on before kits existed, which is why reading an
    /// older record produces this rather than a guess — a job written with a
    /// bare agent name genuinely ran on that agent's defaults, so this is the
    /// true answer, per `docs/conventions.md` §4.
    #[must_use]
    pub const fn defaults(agent: Agent) -> Self {
        match agent {
            Agent::Claude => Self::Claude {
                model: ClaudeModel::Default {
                    effort: ClaudeEffort::Default,
                },
            },
        }
    }

    /// Which agent this is a kit for.
    ///
    /// Derived from the variant rather than stored beside it, which is the
    /// whole point of the shape: there is no way for the two to disagree.
    #[must_use]
    pub const fn agent(&self) -> Agent {
        match self {
            Self::Claude { .. } => Agent::Claude,
        }
    }
}

/// Which of Claude's models a kit runs on, and how hard it thinks where that
/// is a choice at all.
///
/// The effort lives *inside* the variants that have one rather than beside
/// the model, because the pinned adapter offers no effort on Haiku and refuses
/// one asked of it as an unknown option — measured in
/// `docs/decisions/0048-a-job-runs-on-a-kit.md`. Two fields side by side would
/// let that combination be written down; this shape cannot.
///
/// The variants are the adapter's *aliases*: it refuses a dated model
/// identifier outright, and an alias follows the vendor's releases on its own,
/// so this set is small and moves only when the pinned adapter does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaudeModel {
    /// Whatever the adapter recommends, which it describes as Sonnet.
    Default {
        /// How hard it thinks.
        effort: ClaudeEffort,
    },
    /// Efficient for routine work, in the adapter's words.
    Sonnet {
        /// How hard it thinks.
        effort: ClaudeEffort,
    },
    /// Best for complex work, in the adapter's words, and the most expensive.
    Opus {
        /// How hard it thinks.
        effort: ClaudeEffort,
    },
    /// Fastest for quick answers, and the one with no effort to choose.
    Haiku,
}

impl ClaudeModel {
    /// How hard this model is asked to think, if that is a choice on it.
    #[must_use]
    pub const fn effort(self) -> Option<ClaudeEffort> {
        match self {
            Self::Default { effort } | Self::Sonnet { effort } | Self::Opus { effort } => {
                Some(effort)
            }
            Self::Haiku => None,
        }
    }
}

/// How hard one of Claude's models is asked to think.
///
/// The adapter's own levels, `Default` included: "no preference" is a value
/// the agent spells and means something by, not an absence this crate would
/// add on top — see `docs/decisions/0048-a-job-runs-on-a-kit.md` on why this
/// is not an `Option`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ClaudeEffort {
    /// Whatever the adapter picks for the model.
    Default,
    /// The least.
    Low,
    /// Between.
    Medium,
    /// More.
    High,
    /// More still.
    XHigh,
    /// The most the adapter offers.
    Max,
}

impl ClaudeEffort {
    /// Every level, the default first and then rising.
    ///
    /// A list rather than a derive, for the reason [`Agent::ALL`] gives:
    /// forgetting to add a level here costs a missing menu entry, which is the
    /// cheapest failure available.
    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::XHigh,
        Self::Max,
    ];
}

/// The name an operator gives one of a project's kits.
///
/// Trimmed and non-empty, and nothing more: it is what a foreman says back in
/// order to choose a kit and what a person reads on a form, so the only rule is
/// that there is something to say. A project keys its kits on it, so two kits
/// under one name on one project is unrepresentable rather than checked.
///
/// Like [`VariableName`] it implements no deserialisation. A name a file
/// carries is checked as that file is opened rather than trusted because
/// something once checked it — the boundary a credential crosses through
/// [`SealedSecret`], applied to text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KitName(String);

impl KitName {
    /// Accepts a name if there is one, with the whitespace around it removed.
    ///
    /// Trimmed rather than refused for the space, because the difference
    /// between `deep` and `deep ` is one nobody typed on purpose, and a form
    /// refusing it would be refusing a name it could plainly see.
    ///
    /// # Errors
    ///
    /// Fails if nothing is left once the whitespace is gone.
    pub fn new(name: impl Into<String>) -> Result<Self, KitNameError> {
        let name = name.into();
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(KitNameError::Empty);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The name, as a person or a foreman would say it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KitName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A kit was given no name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KitNameError {
    /// Nothing but whitespace, or nothing at all.
    #[error("a kit needs a name")]
    Empty,
}

/// What an operator supplies in order to offer one kit on a project.
///
/// The kit, and what this project wants it for. The description is the
/// operator's rather than code's, and the difference is the one
/// `docs/decisions/0006-agents-are-pluggable.md` draws for an agent's: that
/// one describes the agent and so lives in code, while this one describes what
/// *this project* wants the kit for, which nothing in code could know. It is
/// what the foreman reasons over when it chooses a kit — see
/// `docs/decisions/0048-a-job-runs-on-a-kit.md`.
///
/// Holds no credential, so it crosses the snapshot boundary as itself, the way
/// a [`Job`] does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitConfig {
    /// What this project wants the kit for, in the operator's words.
    pub description: String,
    /// The kit itself.
    pub kit: Kit,
}

impl KitConfig {
    /// An agent's defaults, described as the agent describes itself.
    ///
    /// What a project written before kits existed is read as offering, one per
    /// agent it named — the default that is the true answer, per
    /// `docs/conventions.md` §4, since nothing else existed to run on — and
    /// what a project offers for an agent nobody has yet written a kit for.
    #[must_use]
    pub fn defaults(agent: Agent) -> Self {
        Self {
            description: agent.description().to_owned(),
            kit: Kit::defaults(agent),
        }
    }
}

/// A platform a project's jobs act on.
///
/// One variant for now, which is the one a job cannot work without: cloning the
/// repository, pushing a branch and opening a pull request are all the same
/// credential. See
/// `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` for why a
/// job holds these at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Platform {
    /// The repository host.
    GitHub,
}

/// An App this instance owns on a platform.
///
/// What it registered there once, from the dashboard, and mints tokens with
/// from then on — see
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
/// One per platform, held by the instance rather than by a project, because
/// a project installs it and does not own it. The client secret and the
/// webhook secret the registration also answered with are not kept: nothing
/// here authorises a user or receives a webhook, and a secret kept for
/// nothing is a secret to leak for nothing.
///
/// Deriving `Debug` is safe and deliberate: the one credential in it redacts
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformApp {
    /// The App's identifier on the platform.
    pub id: u64,
    /// Its slug, which every address on the platform is made from.
    pub slug: String,
    /// Its client identifier, which the platform takes as the issuer of the
    /// token this instance signs.
    pub client_id: String,
    /// Its private key, PEM, which signs that token.
    pub private_key: Secret,
    /// Where it is installed, by the installation's identifier on the
    /// platform: learned from the platform's setup redirect, confirmed with
    /// the key, and kept here rather than on any project — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    /// A project's access names one of these, and a form lists what they
    /// cover from the platform when it asks.
    pub installations: BTreeMap<u64, Installation>,
}

/// One installation of the App: on which account, and whether on every
/// repository of it or on chosen ones.
///
/// What the platform says of an installation when it is fetched with the
/// App's key, and all this instance keeps of one. Which repositories it
/// covers is deliberately not here: it goes stale the moment a repository
/// is added on the platform's own page, so it is listed when a form asks
/// and kept nowhere. Nothing in it is a credential, so it travels in the
/// clear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installation {
    /// The account it is installed on, as the platform spells it.
    pub account: String,
    /// Whether it covers every repository of that account rather than
    /// chosen ones.
    pub every_repository: bool,
}

/// A Slack app this instance owns.
///
/// What an operator pasted once from the platform's own page, and the
/// workspaces it has been installed on since — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
/// One per channel, held by the instance rather than by a project, because
/// a project speaks through a workspace of it and does not own it. Three
/// values and no more: the client identifier an install names and the
/// exchange of a code is made with, the client secret that exchange is made
/// with, and the app-level token that opens the one event stream every
/// workspace's events arrive on. Not the signing secret, which verifies
/// requests this instance never receives, and not the verification token,
/// which the platform deprecated: a secret kept for nothing is a secret to
/// leak for nothing.
///
/// Deriving `Debug` is safe and deliberate: every credential in it redacts
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelApp {
    /// Its client identifier on the platform: not a secret, and what the
    /// install link names.
    pub client_id: String,
    /// Its client secret, which the exchange of an install's code is made
    /// with.
    pub client_secret: Secret,
    /// Its app-level token, which opens the event stream and never leaves
    /// the daemon.
    pub app_token: Secret,
    /// Where it is installed, by the workspace's identifier on the platform:
    /// learned from the platform's redirect and kept here rather than on any
    /// project, as the App's installations are. A project's binding names
    /// one of these.
    pub workspaces: BTreeMap<String, Workspace>,
}

/// One workspace the instance's app is installed in: what the platform
/// answered when the install's code was exchanged, and all this instance
/// keeps of one.
///
/// The bot token is the credential that speaks there, handed to a job
/// exactly as a binding's own is; the bot user is what a mention names.
///
/// Deriving `Debug` is safe and deliberate: the one credential in it
/// redacts itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The workspace's name, as the platform spells it: for a person.
    pub name: String,
    /// The bot user the install made, which a mention there names.
    pub bot_user: String,
    /// The bot token minted for the install: what speaks in that workspace.
    pub bot_token: Secret,
}

/// How a project reaches its repository's platform — see
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
///
/// One of two shapes, and a project holds one per platform. Whichever it
/// holds, a job never carries it: the credential route serves a pasted
/// token as it is, and mints one from an installation for the project's
/// repository and an hour, so the shape decides what the instance does to
/// answer a job and nothing about what the job sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// A token pasted and checked, per
    /// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`,
    /// held for as long as the project is, with what the platform said of
    /// it when it was checked — see
    /// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    Token {
        /// The token itself.
        secret: Secret,
        /// Whose it is, as the platform spells the account: read when the
        /// token was checked, and none for a token the last release kept.
        owner: Option<String>,
        /// When the platform will stop accepting it: read from the answer
        /// when the token was checked, and none where the platform says
        /// none, or for a token the last release kept.
        expires: Option<Timestamp>,
    },
    /// An installation of the App this instance owns, by its identifier on
    /// the platform, learned from the platform's setup redirect and
    /// confirmed with the App's key. Not a credential: what reaches the
    /// repository is minted from it, per project, when a job asks.
    Installation {
        /// The installation's identifier on the platform.
        id: u64,
    },
}

/// Where a project's repository is, as the platform names it: an owner and
/// a name on GitHub, and what a project holds — see
/// `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
///
/// Two parts rather than text, so that everything composed from it — the
/// address a browser opens, a pull request's by number, the name a token is
/// minted for — is composed from an address and never from text that was
/// not one, per
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
/// Written to the file as the address this project spells and parsed back
/// on opening; text there that is not an address refuses the file, naming
/// the project, rather than opening as something nothing can compose from.
///
/// Serialises, because the instance holds one while a credential is
/// checked against it and a scenario's snapshot walks what is held; it is
/// an address and never a secret. Shown as `owner/name`, which is how the
/// platform and a person both say it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RepositoryAddress {
    /// Who owns it, as the platform spells it.
    pub owner: String,
    /// What it is called there.
    pub name: String,
}

impl fmt::Display for RepositoryAddress {
    /// `owner/name`, as the platform says it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl RepositoryAddress {
    /// An address from its two parts, each checked to be something the
    /// platform would call an owner or a repository.
    ///
    /// # Errors
    ///
    /// Fails if either part is empty or holds a character the platform does
    /// not allow.
    pub fn new(owner: &str, name: &str) -> Result<Self, RepositoryError> {
        if !is_slug(owner) || !is_slug(name) {
            return Err(RepositoryError::NotOwnerAndName);
        }
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Parses an https address on GitHub, forgiving the two things people
    /// paste along with one: a trailing `.git`, and a trailing slash.
    ///
    /// # Errors
    ///
    /// Fails if it is not https, not on GitHub, or not an owner and a name
    /// and nothing more.
    pub fn parse(text: &str) -> Result<Self, RepositoryError> {
        let Some(rest) = text.trim().strip_prefix("https://") else {
            return Err(RepositoryError::NotHttps);
        };
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            return Err(RepositoryError::NotOnGitHub);
        }
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let mut parts = path.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(owner), Some(name), None) if is_slug(owner) && is_slug(name) => Ok(Self {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            _ => Err(RepositoryError::NotOwnerAndName),
        }
    }

    /// The address as this project writes it: https, no suffix, no slash.
    #[must_use]
    pub fn https(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}

/// Whether one part of a path is something the platform would call an owner
/// or a repository: letters, digits, and the three marks it allows.
fn is_slug(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Why some text is not a repository's address.
///
/// Says which rule was broken rather than merely refusing, because the
/// operator is looking at the box they typed it into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryError {
    /// It does not start with the one scheme accepted.
    #[error("it has to start with https://")]
    NotHttps,
    /// It is on some other host.
    #[error("it has to be on github.com")]
    NotOnGitHub,
    /// Its path is not an owner and a name.
    #[error("it has to name an owner and a repository, and nothing more")]
    NotOwnerAndName,
}

/// The name of one variable a project gives its jobs.
///
/// Validated on the way in, so that everything downstream is total: an adapter
/// receiving one of these never has to ask whether it can be delivered, and
/// there is no path by which an undeliverable name reaches a container's
/// argument list. See
/// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.
///
/// **The rule is not cosmetic, and the equals sign is why.** A container
/// runtime told to forward a variable whose *name* contains one sets it inline
/// instead — measured on Docker and on Podman, which agree — so the value stops
/// travelling through an environment and starts travelling through the command
/// line, where any user on the machine can read it out of the process table.
/// Refusing the name here is what keeps that from being possible to type.
///
/// Lowercase is allowed deliberately. The rule is what an environment can
/// carry rather than a house style, and the proxy variables an operator will
/// reach for first are spelled in lower case.
///
/// **It implements no deserialisation, and that is the point.** A snapshot is
/// untrusted input, so a name arriving from one is checked as the file is
/// opened rather than trusted because it was once checked — the same boundary
/// [`Secret`] crosses through [`SealedSecret`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariableName(String);

impl VariableName {
    /// Accepts a name if a container could be given one.
    ///
    /// # Errors
    ///
    /// Fails if it is empty, begins with a digit, or holds anything but
    /// letters, digits and underscores.
    pub fn new(name: impl Into<String>) -> Result<Self, VariableNameError> {
        let name = name.into();
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return Err(VariableNameError::Empty);
        };
        if first.is_ascii_digit() {
            return Err(VariableNameError::LeadingDigit);
        }
        if !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(VariableNameError::NotAName);
        }
        Ok(Self(name))
    }

    /// The name, for whoever is about to deliver it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VariableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A name a container could not be given.
///
/// Says which rule was broken rather than merely refusing, because the
/// operator is looking at the box they typed it into. It says nothing about a
/// *value*, which is a credential and never appears in an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VariableNameError {
    /// Nothing was given.
    #[error("a variable needs a name")]
    Empty,
    /// It begins with a digit, which no environment accepts.
    #[error("a variable's name cannot begin with a digit")]
    LeadingDigit,
    /// It holds something other than letters, digits and underscores.
    ///
    /// An equals sign is the one worth knowing about: a runtime reads a name
    /// containing one as an inline assignment, which would put the value on
    /// the command line for anybody on the machine to read.
    #[error("a variable's name may hold only letters, digits and underscores")]
    NotAName,
}

/// Somewhere the foreman watches and a job can speak into.
///
/// Two-directional by definition, which is the whole reason this is not a
/// variant of [`Platform`] — see
/// `docs/decisions/0027-a-channel-is-not-a-platform.md`. A platform is
/// something a job *acts on*; a channel is where a conversation happens, and
/// the foreman is on it as much as a job is.
///
/// A closed set for the same reason [`Agent`] is: reaching one needs code, and
/// code is not something an operator supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Channel {
    /// The team chat, and the escalation path — see
    /// `docs/decisions/0005-conversation-happens-on-channels.md`.
    Slack,
}

/// What an operator supplies in order to bind one channel to a project.
///
/// Two credentials, held by different processes, which is why channels do
/// not share the platform map: a platform's entry is one secret a job is
/// handed, and a binding is one a job is handed and one it never sees. It
/// used to carry an address as well — the one room a project was listened
/// to in, and then the room a job's thread opened in — and carries none since
/// `docs/decisions/0061-a-job-has-a-room-of-its-own.md`: the app hears every
/// room it is invited to, and a job has a room of its own.
///
/// Deriving `Debug` is safe and deliberate. Both credentials redact
/// themselves, so there is nothing here for a hand-written formatter to hide
/// that the field types do not already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConfig {
    /// What the channel is reached with.
    ///
    /// Belongs to the project rather than the instance, for the reason
    /// `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md` gives:
    /// watching a project's channels needs that project's credentials, and one
    /// holder of every project's at once is the shape being avoided.
    pub credential: Secret,
    /// What listening on that channel needs.
    ///
    /// A second credential rather than a wider first one, because the two
    /// authorise different things and are held by different processes.
    /// [`Handout`] delivers `credential` above to a job's container and never
    /// this: posting is what a job does, and opening an event stream is not.
    /// A leaked job credential can therefore post in one channel, which is all
    /// it could ever do — see
    /// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.
    ///
    /// Required, since
    /// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`:
    /// a project that speaks and is never answered is a project whose jobs
    /// wait for answers that have no way to arrive.
    pub listen_credential: Secret,
}

/// How a project is bound to a channel.
///
/// Through an app of its own, or through a workspace the instance's app on
/// that channel is installed on — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
/// One of two shapes, and a project holds one per channel. Whichever it
/// holds, a job is handed the credential that speaks and never the one that
/// listens: [`State::speaking`] resolves the first through the app where
/// the binding is a workspace, and the second belongs to the app and never
/// to a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// An app the project owns, with both credentials pasted: what every
    /// project had before the instance owned an app.
    Own(ChannelConfig),
    /// A workspace the instance's app is installed on, by the workspace's
    /// identifier on the platform: the credential that speaks is the
    /// workspace's bot token, kept beside the app, and the one that listens
    /// is the app's.
    Workspace(String),
}

impl Binding {
    /// The app of its own, where the binding is one.
    #[must_use]
    pub const fn own(&self) -> Option<&ChannelConfig> {
        match self {
            Self::Own(config) => Some(config),
            Self::Workspace(_) => None,
        }
    }

    /// The workspace, where the binding is one.
    #[must_use]
    pub fn workspace(&self) -> Option<&str> {
        match self {
            Self::Own(_) => None,
            Self::Workspace(team) => Some(team),
        }
    }
}

/// One room on a channel: for Slack, one Slack channel.
///
/// A job has one of its own, made with the job and archived with it — see
/// `docs/decisions/0061-a-job-has-a-room-of-its-own.md` — and a message
/// arriving in it is that job's. Named for the concept in
/// `docs/conventions.md` §2 rather than the platform's word, because
/// [`Channel`] is the platform.
///
/// The identifier is opaque here and stays text, like a thread's. Ordered
/// so that a project can hold a set of them — the rooms it watches — and
/// the snapshot does not reshuffle it between writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Room {
    /// The channel it is a room on.
    pub channel: Channel,
    /// What identifies it there.
    pub id: String,
}

/// Where one turn speaks: a room, and a thread in it when the message being
/// answered was in one.
///
/// A job's conversation happens in its room, and a person may ask it
/// something in a thread there, which is where the answer then belongs; a
/// foreman always answers in a thread. So a turn is narrowed to a room and
/// perhaps a thread, and a [`Thread`] is the case where there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    /// The room.
    pub room: Room,
    /// The thread in it, if the conversation is in one.
    pub thread: Option<String>,
}

impl Place {
    /// The root of a room.
    #[must_use]
    pub const fn root(room: Room) -> Self {
        Self { room, thread: None }
    }
}

impl From<Thread> for Place {
    fn from(thread: Thread) -> Self {
        Self {
            room: Room {
                channel: thread.channel,
                id: thread.room,
            },
            thread: Some(thread.id),
        }
    }
}

/// A thread: a reply chain under one message in a room.
///
/// Where the foreman answers, and where a job answers when it was asked in
/// one. It was where a job's whole conversation happened, hanging from an
/// announcement in the project's one room, until
/// `docs/decisions/0061-a-job-has-a-room-of-its-own.md` gave a job a room
/// instead; what routes a reply now is the room.
///
/// The identifier is opaque here and **must stay text**. For Slack it is the
/// parent message's timestamp, which looks like a number and is not one:
/// parsing it loses the microseconds and yields an identifier that addresses
/// no message. The domain does not need to know that, and does need to not
/// convert it.
///
/// It names its channel and its room as well as the thread, because an
/// identifier is only unique within one room, and since
/// `docs/decisions/0060-a-binding-is-a-workspace.md` the app hears more than
/// one: a foreman answers wherever it was mentioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// The channel it is a thread on.
    pub channel: Channel,
    /// The room it is in, as the platform names it.
    pub room: String,
    /// What identifies it there.
    pub id: String,
}

/// One agent, in one workspace, working on one project.
///
/// A job happens once and there is no retry: a second attempt is a new job with
/// its own workspace, which is why nothing here records an attempt count. It
/// may outlive the process supervising it, and resuming is not retrying — the
/// same job carries on, which is the distinction
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` turns on.
///
/// Nothing here names the container it runs in. The name is derived from the
/// job's identifier, so there is no moment at which a container exists and the
/// value naming it has not been written down — a field would have that gap, and
/// a container nothing can name is the one leak 0015 has to prevent.
///
/// It held no credential until
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
/// gave it a warrant, and so crossed the snapshot boundary unchanged; it now
/// crosses it as [`SealedJob`], like every other type that carries one.
#[derive(Debug, Clone)]
pub struct Job {
    /// What it runs on: which agent, set how.
    ///
    /// The one field nothing may change once the record exists, and the only
    /// private one here for that reason. A resumed job is the same job
    /// continuing, so what it runs on is part of what it *is*, and the record
    /// of it has to stay true for "why did this go badly?" to have an answer
    /// — see `docs/decisions/0048-a-job-runs-on-a-kit.md`. Everything else on
    /// this type is written to as the job goes; this is read through
    /// [`Job::kit`] and set only by [`Job::new`].
    ///
    /// Stored by value, so this stays true after an operator removes that
    /// agent's configuration.
    ///
    /// Written as `kit`, and read as the bare agent name too. Every job
    /// recorded before kits existed says `"agent": "Claude"`, and such a job
    /// genuinely ran on that agent's defaults, so that is what it reads as —
    /// the default that is the true answer, per `docs/conventions.md` §4,
    /// rather than the substituted one it forbids.
    kit: Kit,
    /// Why the foreman started it, in prose.
    ///
    /// The whole of a job's provenance, deliberately — see
    /// `docs/architecture.md` §2 on why the structured version is absent.
    pub reason: String,
    /// The instruction the agent begins from.
    ///
    /// Self-contained by necessity: an agent in a fresh workspace knows nothing
    /// about where it came from, so this carries the repository, the work, and
    /// the constraint that it proposes rather than merges.
    pub kickoff: String,
    /// When the record was created.
    ///
    /// Named for the record rather than for the work, so it stays true once
    /// there is a gap between a job existing and an agent starting.
    pub created_at: Timestamp,
    /// Where it has got to.
    ///
    /// Read through a bridge on the sealed form, because the shape on disk
    /// changed: a private deserialiser beside it reads what the last release
    /// wrote as well as what this one does.
    pub progress: Progress,
    /// When its progress last changed — what a job that needs a person has
    /// been waiting since, per
    /// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    ///
    /// Written whenever the progress is, from the time the world told the
    /// instance with the step, per
    /// `docs/decisions/0073-the-world-tells-the-instance-the-time-with-every-step.md`.
    /// None for every job the last release wrote, which is why it is
    /// defaulted: such a job says only that it waits, and it has waited
    /// longer than any job that carries a moment, since its last change was
    /// before this build's first stamp.
    pub since: Option<Timestamp>,
    /// The room its conversation happens in, once there is one.
    ///
    /// Recorded for the lookup in the other direction: a reply arrives naming
    /// a room and nothing else, so the instance needs to know which job that
    /// is, and that lookup has to survive the process dying — see
    /// `docs/decisions/0061-a-job-has-a-room-of-its-own.md`.
    ///
    /// Absent for a job whose room could not be created, and for every job
    /// the last release wrote — which is why it is defaulted, per
    /// `docs/conventions.md` §4. What the last release wrote instead was a
    /// `thread`, which is read past and dropped: a thread is not somewhere
    /// this rule can answer, so the job keeps its record and loses its place
    /// to be answered in, which is the loss §4 permits.
    pub room: Option<Room>,
    /// Who asked for it, as the platform names them, when a person's message
    /// is what started it.
    ///
    /// Carried so that the person is invited into the job's room and can be
    /// named when it needs them. None for a job started from the dashboard,
    /// and for every job the last release wrote, which is why it is
    /// defaulted.
    pub asked_by: Option<String>,
    /// What the agent's session reported it was set to, after being set.
    ///
    /// Beside the kit rather than derived from it, because the two are
    /// different facts: the kit is what was asked, and this is what the
    /// adapter said in reply, in its own spelling. The two were measured to
    /// differ — an account entitled to a larger context has the same alias
    /// reported back with a suffix — so this is the only record of what
    /// actually ran, and the kit alone is not it. See
    /// `docs/decisions/0048-a-job-runs-on-a-kit.md`.
    ///
    /// Keyed by the adapter's own option identifier, so an entry means nothing
    /// without knowing the agent, which the kit says. Overwritten on every
    /// turn, since every turn sets and reads back. Empty for a job that has not
    /// had a turn, and for every job recorded before this existed, which is why
    /// it is defaulted.
    pub reported: BTreeMap<String, String>,
    /// The messages it has been sent and has not finished with — see
    /// `docs/decisions/0069-a-message-reaches-a-working-job.md`. Kept with
    /// the record, so that a message in hand when this process dies is
    /// still in hand when it starts again. Empty for every job the last
    /// release wrote, which is why it is defaulted.
    pub inbox: Inbox,
    /// The pull requests it said it opened, by number on the project's
    /// repository — see
    /// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    ///
    /// The union of every number ever claimed, sorted, so that a forgetful
    /// later turn cannot erase an earlier one; whether any is still open is
    /// the platform's to know. A number names a pull request on this
    /// project's repository and nowhere else, which is why it is a number
    /// and not an address. Empty for a job that claimed none, and for every
    /// job the last release wrote, which is why it is defaulted.
    pub pull_requests: BTreeSet<u64>,
    /// What its container presents to this instance to fetch its project's
    /// platform credential with, and nothing else — see
    /// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
    ///
    /// Minted when the job is and fixed for its life, like the kit, and
    /// private for the same reason: it is delivered into the container's
    /// environment at creation, so a value that changed would be one the
    /// container could no longer present. None only for a job the last
    /// release wrote, whose container was created with the credential itself
    /// in its environment and keeps what it has, since resuming is not
    /// retrying. Read through [`Job::warrant`] and set only by [`Job::new`].
    warrant: Option<Secret>,
}

/// The messages a job has been sent and has not finished with.
///
/// The shape `docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`
/// gave a foreman, widened in the one way a job needs: what is in hand is
/// plural, because a message handed to a running turn joins the one that
/// started it, and both are finished with when that turn ends. Everything
/// not yet given waits in the order it arrived, which is the only order that
/// makes sense to give it in.
///
/// **Nothing running means nothing waiting**, for a job as for a foreman: a
/// job that is not working has an empty inbox. A foreman's shape makes the
/// contrary unsayable; this is a field beside the job's progress instead,
/// because `docs/decisions/0069-a-message-reaches-a-working-job.md` fixed it
/// as one, defaulted for what the last release wrote, and because a job
/// works with nothing in hand — on its kickoff turn, and on a resume — where
/// a foreman never does. So the rule is kept by every transition the
/// instance makes and checked by the simulation after every step, rather
/// than by the type; see `docs/conventions.md` §2. What a turn's end, a
/// person's stop and a turn that never started each do with what is here is
/// that record's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Inbox {
    /// What the turn in flight has been given: the message that started it,
    /// and every one handed to it since.
    #[serde(default)]
    pub given: Vec<Errand>,
    /// What has not been given yet, front first.
    #[serde(default)]
    pub waiting: std::collections::VecDeque<Errand>,
}

impl Inbox {
    /// An inbox with nothing in it.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            given: Vec::new(),
            waiting: std::collections::VecDeque::new(),
        }
    }

    /// Takes a message, behind whatever already waits.
    pub fn receive(&mut self, errand: Errand) {
        self.waiting.push_back(errand);
    }

    /// What would be given next, if anything waits.
    #[must_use]
    pub fn next(&self) -> Option<&Errand> {
        self.waiting.front()
    }

    /// Gives the next message to the turn in flight: it stops waiting and
    /// is in hand. Answers with it, and with nothing when nothing waited.
    pub fn give(&mut self) -> Option<&Errand> {
        let next = self.waiting.pop_front()?;
        self.given.push(next);
        self.given.last()
    }

    /// The turn in flight ended: everything it had been given is finished
    /// with, and is what this answers with.
    pub fn finish(&mut self) -> Vec<Errand> {
        std::mem::take(&mut self.given)
    }

    /// The last message given was not taken — handed to a running turn that
    /// had ended by the time it arrived — so it waits again, ahead of
    /// whatever arrived since.
    pub fn hand_back(&mut self) {
        if let Some(errand) = self.given.pop() {
            self.waiting.push_front(errand);
        }
    }

    /// Everything, given and then waiting, in the order it was received:
    /// what a person's stop, or a turn that never started, has to say
    /// something about. Leaves nothing.
    pub fn drain(&mut self) -> Vec<Errand> {
        let mut all = std::mem::take(&mut self.given);
        all.extend(std::mem::take(&mut self.waiting));
        all
    }

    /// Whether nothing is in hand and nothing waits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.given.is_empty() && self.waiting.is_empty()
    }
}

impl Job {
    /// A job as it is created: working, on `kit`, with nowhere yet to speak.
    ///
    /// The only way to give a job its kit, which is what makes "fixed when the
    /// job is created" a property of the type rather than a rule — see
    /// `docs/decisions/0048-a-job-runs-on-a-kit.md`. Takes the timestamp
    /// rather than reading a clock, like everything else here.
    ///
    /// Takes the warrant as well, so that every job created from now on holds
    /// one: the only job without one is a job read from a file the last
    /// release wrote.
    #[must_use]
    pub const fn new(
        kit: Kit,
        reason: String,
        kickoff: String,
        created_at: Timestamp,
        warrant: Secret,
    ) -> Self {
        Self {
            kit,
            reason,
            kickoff,
            created_at,
            progress: Progress::Working,
            since: Some(created_at),
            room: None,
            asked_by: None,
            reported: BTreeMap::new(),
            inbox: Inbox::new(),
            pull_requests: BTreeSet::new(),
            warrant: Some(warrant),
        }
    }

    /// What this job runs on, for its whole life.
    #[must_use]
    pub const fn kit(&self) -> &Kit {
        &self.kit
    }

    /// What its container presents to fetch its project's credential with,
    /// or nothing for a job the last release wrote.
    #[must_use]
    pub const fn warrant(&self) -> Option<&Secret> {
        self.warrant.as_ref()
    }
}

/// Where a job has got to.
///
/// The states `docs/architecture.md` §1 says this crate holds, and the shape
/// is two levels rather than one because two different questions are being
/// answered — see
/// `docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md`.
///
/// **The outer level is what the system does with a job**: resume it, leave it
/// alone, or reclaim what it was holding. Three, and every caller that asks a
/// behavioural question matches on exactly these three and stays at three when
/// a reading is added. **The inner level is what a person does about it**, and
/// only the dashboard and whoever reads it care.
///
/// A state nobody acts on differently is still a label rather than a state,
/// and that rule is what puts *done* and *discarded* inside one variant rather
/// than beside each other at the top: the system does the same thing with
/// both.
///
/// Note what is absent. There is no *interrupted*, although a job's container
/// is stopped every time the daemon dies. That is a fact about the runtime and
/// not about the work: the job is still running, and startup's job is to make
/// the containers match rather than to move the job somewhere new — see
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, where resuming is
/// the same job continuing. A state for it would have to be left behind on
/// every resume, and the first crash between the two would strand a job in a
/// state nothing clears.
///
/// There is also no `Default`. A job is created running, so the value is never
/// in doubt at the one moment a default would be consulted — and a job whose
/// progress was filled in by nobody is exactly the kind of record that later
/// reads as fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Progress {
    /// Its agent has been given something and has not stopped.
    ///
    /// What a sweep looks for: a working job whose container is stopped is one
    /// to restart, and a container whose job is not working is not.
    ///
    /// Believed rather than observed, which is why it is not called *running*:
    /// a job is in this state while the daemon that was supervising it is
    /// dead and no process is running anywhere — see
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`. The word is
    /// about the work, not about a process.
    Working,
    /// Its agent stopped, and it can be given something.
    ///
    /// **Says nothing about how it went**, and the payload says what it is
    /// waiting for — which are two different claims and only the second is
    /// ever a guess. This used to be called *completed*, which claimed the
    /// work had ended, and that is a claim this system cannot make.
    Idle(Waiting),
    /// It is over, and nothing more will be given to it.
    ///
    /// The container is gone with everything in it, and the record is all that
    /// is left. See
    /// `docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md`.
    Retired(Outcome),
}

impl Progress {
    /// Whether this job is over.
    ///
    /// The question nearly every caller actually asks, and worth a method
    /// rather than a pattern at each of them: a retired job takes no reply, is
    /// never resumed, holds no container and is not swept.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        matches!(self, Self::Retired(_))
    }
}

/// What an idle job is waiting for.
///
/// **Every one of these is a job somebody could give something to**, which is
/// what makes them one state with five readings rather than five states: the
/// system does exactly the same thing with all of them, and a person does
/// something different about each. That split — the outer level for what the
/// system does, the inner for what a person does — is the whole shape of
/// [`Progress`].
///
/// Four of the five are the agent's own account of why it stopped, delivered
/// by a tool it is asked to call before the turn ends. The fifth is what a
/// turn that ended without one becomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Waiting {
    /// Its agent asked something and stopped, and an answer is what unblocks
    /// it.
    Asked,
    /// Its agent believes the work is done and is inviting somebody to look.
    ///
    /// A claim rather than a fact, and deliberately so — nothing here can
    /// check it, and `docs/decisions/0002-never-merge-never-deploy.md` means a
    /// person reads what it proposed before any of it counts for anything.
    Proposed,
    /// A person stopped it while it was working.
    ///
    /// Distinct from every other reading because it is the only one nothing
    /// inside the job decided. Nothing went wrong and nothing was finished:
    /// somebody wanted it to stop.
    Paused,
    /// Its turn ended badly, and this is what went wrong.
    ///
    /// **Not an ending.** The container is still there with the session in it,
    /// so this is a job waiting for whatever broke to be fixed — an expired
    /// credential being the case that will arrive first. A reply is what
    /// tries again. What makes a job *over* is [`Progress::Retired`] and
    /// nothing here.
    ///
    /// Prose for a person reading the dashboard, like `reason` — not a code to
    /// branch on.
    Failed(String),
    /// Its turn ended, and the agent said nothing about which of these it was.
    ///
    /// The honest residual, and it must exist: a turn can end on a token
    /// limit, on a refusal, or on an agent that simply did not call the tool.
    /// Defaulting those to *asked* or *proposed* would record a claim nobody
    /// made, which is the one failure this whole shape exists to avoid. A job
    /// landing here is a prompt that was not followed, and it is visible.
    Silent,
}

/// How a job that is over ended.
///
/// Three because a person does something different about each, and no more:
/// this is the level `docs/conventions.md` §2 warns about, where a label that
/// nobody acts on differently gets added because it reads well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// A person judged that it produced what was wanted.
    Done,
    /// A person judged that it did not, and kept nothing.
    ///
    /// Not a failure: the job ran, and what it produced was read and turned
    /// down. The distinction from [`Waiting::Failed`] is who decided and
    /// whether anything can still be done about it.
    Discarded,
    /// Its container went missing before it reached an ending of its own.
    ///
    /// The one outcome nobody chooses. A job is put here by the sweep, on
    /// finding that what it was recorded as still needing is not there — most
    /// plausibly removed by hand, or by a runtime that was reset. Terminal
    /// because there is nothing left to resume: the session lived in that
    /// container.
    ///
    /// It also covers the narrow case of a job killed between its record being
    /// written and its container being created, which never had one. That is
    /// the ordering `begin` chooses deliberately, and this is the state that
    /// resolves it.
    Lost,
}

/// Reads a job's progress, in this shape or in the one before it.
///
/// The bridge `docs/conventions.md` §4 requires for a value that goes on disk.
/// What the last release wrote is `Working`, a bare `Idle`, or a `Failed`
/// carrying prose; all three still parse, and the two that have moved arrive
/// as the reading that is true of them. A bare `Idle` says the agent stopped
/// and nothing more, which is exactly [`Waiting::Silent`] — the older format
/// could not carry a claim, so no claim is invented for it.
///
/// The current shape is tried first and the two cannot be confused: this
/// shape's `Idle` carries a payload and the older one does not, and nothing
/// here spells `Failed` at the top level any more.
fn progress_or_older<'de, D>(deserializer: D) -> Result<Progress, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        Now(Progress),
        Then(Older),
    }

    /// Only the spellings that have moved. `Working` is unchanged, so it
    /// parses as itself above and never reaches here.
    #[derive(Deserialize)]
    enum Older {
        Idle,
        Failed(String),
    }

    Ok(match Written::deserialize(deserializer)? {
        Written::Now(progress) => progress,
        Written::Then(Older::Idle) => Progress::Idle(Waiting::Silent),
        Written::Then(Older::Failed(why)) => Progress::Idle(Waiting::Failed(why)),
    })
}

/// Reads a foreman's inbox, in this shape or in any other.
///
/// The bridge `docs/conventions.md` §4 requires. An inbox the last release
/// wrote holds errands whose threads name no room, so it cannot be carried:
/// a thread in a room this cannot name is nowhere to answer. It opens as
/// idle, and so does anything else that is not this release's shape, because
/// §4 says a file must not fail to open over an inbox — what is lost is a
/// message in hand across an upgrade, which the person can send again.
fn attending_or_older<'de, D>(deserializer: D) -> Result<Attending, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        Now(Attending),
        Then(serde::de::IgnoredAny),
    }

    Ok(match Written::deserialize(deserializer)? {
        Written::Now(attending) => attending,
        Written::Then(_) => Attending::Idle,
    })
}

/// One message waiting for, or held by, a project's foreman — or a job,
/// since `docs/decisions/0069-a-message-reaches-a-working-job.md`.
///
/// Carries where to answer as well as what was said, because a foreman answers
/// in the thread its message arrived under — see
/// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`. Working that out
/// when the turn starts rather than when the message arrives would mean
/// looking it up from a message that may be hours old by then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Errand {
    /// What was said, as the person wrote it.
    pub said: String,
    /// Where an answer belongs.
    pub thread: Thread,
    /// Who said it, as the platform names them, so that a job started for
    /// it can invite them. None when the platform did not say, and for every
    /// errand the last release wrote, which is why it is defaulted.
    #[serde(default)]
    pub from: Option<String>,
    /// What identifies the message itself, to react on: the mention's own
    /// identifier, where the thread above names the message it hangs under.
    /// None for every errand the last release wrote, which is why it is
    /// defaulted; such a message is simply not reacted to.
    #[serde(default)]
    pub message: Option<String>,
    /// The app that posted it, by the name the platform gives the app, when
    /// it was another app's message in a watched room rather than a
    /// person's — see
    /// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`. What
    /// the foreman's turn is framed with. None for a person's, and for every
    /// errand the last release wrote, which is why it is defaulted.
    #[serde(default)]
    pub app: Option<String>,
}

/// What a project's foreman is doing, and what is waiting behind it.
///
/// **The shape is the invariant.** A foreman that is idle while messages wait
/// is a state nothing should ever produce, and the way to be sure is for it to
/// have no way of being written down: the queue exists only inside [`Working`],
/// so "idle with something waiting" is not a bug to avoid but a sentence that
/// cannot be said.
///
/// It also removes the case that would otherwise have to be handled and could
/// not occur — taking from a queue that is known to be non-empty, and answering
/// a `None` the compiler can see and a reader cannot.
///
/// [`Working`]: Attending::Working
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Attending {
    /// Nothing in hand and nothing waiting.
    #[default]
    Idle,
    /// One message in hand, and the rest in the order they arrived.
    Working {
        /// What its agent was given.
        on: Errand,
        /// What it will be given next, front first.
        ///
        /// Deliberately not a set and not keyed: two identical messages from a
        /// person are two messages, and the order they arrived in is the only
        /// order that makes sense to answer them in.
        waiting: std::collections::VecDeque<Errand>,
    },
}

impl Attending {
    /// Takes a message, either to start on or to leave waiting.
    ///
    /// The whole of the arrival rule, and **one operation rather than a check
    /// followed by an act** — which is what makes two messages arriving
    /// together unable to both start a turn. Whichever reaches this first
    /// finds `Idle`; the second cannot, because the first already changed it.
    pub fn take(&mut self, errand: Errand) -> Taken {
        match self {
            Self::Idle => {
                *self = Self::Working {
                    on: errand,
                    waiting: std::collections::VecDeque::new(),
                };
                Taken::Started
            }
            Self::Working { waiting, .. } => {
                waiting.push_back(errand);
                Taken::Waiting
            }
        }
    }

    /// Puts down the message in hand and picks up the next, if there is one.
    ///
    /// Answers with what to start next, and `None` only when nothing is left —
    /// which is the one way this becomes [`Attending::Idle`]. A foreman
    /// therefore cannot go idle while anything waits, without anybody having to
    /// remember that.
    pub fn finish(&mut self) -> Option<&Errand> {
        if let Self::Working { mut waiting, .. } = std::mem::take(self)
            && let Some(on) = waiting.pop_front()
        {
            *self = Self::Working { on, waiting };
        }
        self.on()
    }

    /// What its agent is working on, if anything.
    #[must_use]
    pub const fn on(&self) -> Option<&Errand> {
        match self {
            Self::Working { on, .. } => Some(on),
            Self::Idle => None,
        }
    }

    /// How many messages are waiting behind the one in hand.
    #[must_use]
    pub fn waiting(&self) -> usize {
        match self {
            Self::Working { waiting, .. } => waiting.len(),
            Self::Idle => 0,
        }
    }
}

/// What became of a message handed to a foreman.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// The foreman was idle, and this message is now in hand.
    Started,
    /// It was already working, so this is waiting behind what it has.
    Waiting,
}

/// A repository this instance watches, and everything belonging to it.
#[derive(Debug, Clone)]
pub struct Project {
    /// What to call it in the dashboard.
    pub name: String,
    /// Where the repository lives, as an address on the platform — see
    /// `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
    ///
    /// Every kickoff embeds this, because an agent has no other way to find it.
    pub repository: RepositoryAddress,
    /// The kit this project's foreman thinks with.
    ///
    /// Per project rather than per instance, because watching a project's
    /// channels needs that project's credentials and a shared foreman
    /// would hold every project's at once — see
    /// `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`.
    ///
    /// A kit rather than an agent since
    /// `docs/decisions/0048-a-job-runs-on-a-kit.md`, and a change to it lands
    /// at the next turn boundary rather than during a turn. Its settings are
    /// settled again at the start of every turn regardless, so a change of
    /// model or effort keeps the container and the session; a change of agent
    /// is a change of image, and only that replaces the container.
    pub foreman_kit: Kit,
    /// The kits this project's jobs may run on, each under the name the
    /// operator gave it.
    ///
    /// Never empty in a valid instance, and checked rather than made
    /// unrepresentable — see [`State::check`], which is the one definition of
    /// what valid means and is consulted wherever a state could have stopped
    /// being it.
    ///
    /// Named kits rather than a set of agents, per
    /// `docs/decisions/0048-a-job-runs-on-a-kit.md`: a job is started on one
    /// of these and on nothing else, whether the foreman starts it or a person
    /// does, and the operator's judgement about what this project wants each
    /// one *for* is the description the foreman reasons over when it chooses.
    pub kits: BTreeMap<KitName, KitConfig>,
    /// What the operator wrote for its foreman: standing instructions, said
    /// on every turn beside the kits and for the same reason — a session
    /// outlives every edit to it. Where policy lives, per
    /// `docs/decisions/0064-a-project-has-a-brief.md`, and empty for a
    /// project nobody has written one for, which says nothing.
    ///
    /// Not a secret, deliberately: it is written to be read, like a kit's
    /// description, and travels in the clear on the same terms.
    pub brief: String,
    /// How it reaches its repository on each platform: a token, or an
    /// installation of the App this instance owns — see [`Access`].
    ///
    /// A map rather than a list so that two for one platform is
    /// unrepresentable, and ordered rather than hashed so the snapshot does not
    /// reshuffle itself between writes. Empty only for a project the last
    /// release wrote without a token, or one created to be installed on
    /// from its settings page: a job on such a project checks out what is
    /// public and reaches nothing else.
    pub access: BTreeMap<Platform, Access>,
    /// Where its conversations happen, one binding per channel.
    ///
    /// Separate from `access` above rather than folded into it — see
    /// `docs/decisions/0027-a-channel-is-not-a-platform.md`, which is mostly an
    /// argument about the two lines this splits into.
    ///
    /// **Empty only for a project the last release wrote without one.**
    /// Every project is created with a binding since
    /// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`,
    /// and [`State::check`] deliberately does not require it: refusing to
    /// open a file over a missing binding would put the repair behind the
    /// door it locks. Such a project is named at startup and can start no
    /// job until one is bound.
    pub channels: BTreeMap<Channel, Binding>,
    /// The rooms its foreman watches: where another app's message is a
    /// signal rather than nothing, per
    /// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
    ///
    /// A set rather than a list, because watching a room twice is not a
    /// thing; ordered, so the snapshot does not reshuffle it. Empty for
    /// most projects, and for every project the last release wrote.
    pub watched: BTreeSet<Room>,
    /// The room its foreman's transcript is posted in, once one has been
    /// made: before its first turn, per
    /// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
    /// None until then, and for every project the last release wrote.
    pub foreman_room: Option<Room>,
    /// What its jobs are given that this project never reads.
    ///
    /// A third map beside the two above rather than a wider version of either,
    /// because it differs from both in the property that matters: reaching a
    /// platform or a channel needs code, and this needs none — nothing here
    /// parses a value or infers anything from a name. See
    /// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.
    ///
    /// Every value is a [`Secret`], including the ones that are not secret.
    /// The alternative was an operator marking which are, and the mistake in
    /// that direction is unrecoverable and silent — a token marked ordinary is
    /// written to disk in the clear and printed on a screen.
    ///
    /// May be empty, which is most projects.
    pub variables: BTreeMap<VariableName, Variable>,
    /// Its jobs, past and present.
    ///
    /// Nested rather than held globally so that "a job belongs to exactly one
    /// project" is structural instead of a field somebody has to keep true.
    pub jobs: BTreeMap<JobId, Job>,
    /// What its foreman is doing, and what is waiting behind it.
    ///
    /// Per project because a foreman is, and persisted because a message
    /// somebody sent must not be lost to a restart — the same reasoning
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` applies to
    /// work already begun.
    pub attending: Attending,
}

/// Everything one instance knows.
///
/// The whole of what gets snapshotted and the whole of what is loaded back —
/// see `docs/decisions/0011-state-is-a-snapshot-not-a-database.md`.
///
/// There is deliberately no `Default`. An instance with nothing to think with
/// is not a state worth representing, so one is either loaded from a snapshot
/// or built by the first-run flow, and never conjured empty — see
/// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// The agents this instance can run, and what each authenticates with.
    ///
    /// May be empty. An instance with no projects needs nothing to think with,
    /// which is what lets one start with nothing configured at all — see
    /// `docs/decisions/0021-an-instance-starts-empty.md`.
    ///
    /// An agent may not be removed while a project names it. Nothing here
    /// prevents that directly, and three things catch it: [`State::used_by`]
    /// is the query a caller consults first, sealing refuses a state that has
    /// broken the rule, and opening refuses a file that has.
    pub agents: BTreeMap<Agent, AgentConfig>,
    /// The projects it watches.
    pub projects: BTreeMap<ProjectId, Project>,
    /// The Apps it owns on each platform, at most one per platform. May be
    /// empty: a project reaches its repository with a pasted token until an
    /// App is registered, and after.
    pub apps: BTreeMap<Platform, PlatformApp>,
    /// The apps it owns on each channel, at most one per channel. May be
    /// empty: a project speaks through an app of its own until one is
    /// registered, and after — see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    pub channel_apps: BTreeMap<Channel, ChannelApp>,
}

// Deliberately absent: where the container runtime lives. It was a field here
// until `docs/decisions/0023-the-container-runtime-is-discovered-once.md`
// replaced it with discovery, and the reason it does not come back is that it
// describes the *machine* rather than the work. A snapshot is meant to be
// portable — copy the file, carry it to another machine, supply the key — and
// a recorded absolute path is the one thing in it that a different machine
// makes wrong.

impl State {
    /// Which projects depend on an agent's configuration.
    ///
    /// The query to consult before removing one. Empty means the agent can go;
    /// anything else names what would break, which is what a dashboard needs in
    /// order to say *why* rather than merely refusing.
    ///
    /// A project's *past* jobs are not considered and must not be: a job stores
    /// its agent by value precisely so that removing a configuration cannot
    /// rewrite the record of work already done — `docs/conventions.md` §2.
    ///
    /// Skipped by mutation testing, and equivalent rather than untested:
    /// [`Agent`] has one member and a project's job agents are never empty, so
    /// both sides of this condition are true for every project. Inverting the
    /// comparison or replacing the `or` with an `and` changes nothing any test
    /// could observe. **Delete this attribute in the commit that adds a second
    /// agent** — a project naming one agent for its foreman and another for its jobs
    /// is what makes this falsifiable, and it is the first thing that will
    /// exist once there are two.
    #[mutants::skip]
    pub fn used_by(&self, agent: Agent) -> impl Iterator<Item = ProjectId> + '_ {
        self.projects
            .iter()
            .filter(move |(_, project)| {
                project.foreman_kit.agent() == agent
                    || project
                        .kits
                        .values()
                        .any(|offered| offered.kit.agent() == agent)
            })
            .map(|(id, _)| *id)
    }

    /// The credential that speaks for a project on a channel, whichever
    /// shape its binding is.
    ///
    /// Its own app's, or its workspace's bot token from the instance's app —
    /// see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    /// None where the project has no binding on the channel, or names a
    /// workspace the app does not hold, which [`State::check`] refuses.
    #[must_use]
    pub fn speaking(&self, project: ProjectId, channel: Channel) -> Option<Speaking> {
        match self.projects.get(&project)?.channels.get(&channel)? {
            Binding::Own(config) => Some(config.speaking()),
            Binding::Workspace(team) => {
                self.channel_apps
                    .get(&channel)?
                    .workspaces
                    .get(team)
                    .map(|workspace| Speaking {
                        credential: workspace.bot_token.clone(),
                    })
            }
        }
    }

    /// The projects bound to a workspace of the instance's app on a
    /// channel: the candidates a message from that workspace is routed
    /// among.
    pub fn bound_to(&self, channel: Channel, team: &str) -> impl Iterator<Item = ProjectId> + '_ {
        let team = team.to_owned();
        self.projects.iter().filter_map(move |(id, project)| {
            (project.channels.get(&channel)?.workspace()? == team).then_some(*id)
        })
    }

    /// Whether this describes an instance that can exist.
    ///
    /// The invariant `docs/decisions/0021-an-instance-starts-empty.md` moved
    /// down from the instance to the project: a project names one agent for its
    /// foreman and at least one its jobs may run on, and every one of them
    /// is configured. And since
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`,
    /// a project reaching its repository through an installation names one
    /// the App holds, which is the same shape of reference.
    ///
    /// Checked rather than made unrepresentable. A type that could not hold an
    /// empty set would enforce half of this and leave the other half — an agent
    /// removed while a project still named it — needing a check anyway, so the
    /// wrapper bought ceremony at every construction site in exchange for one
    /// of two conditions. One function, asked wherever a state might have
    /// stopped being valid, is the smaller thing.
    ///
    /// It says nothing about *when* to ask. A file is checked as it is read,
    /// because it is untrusted input; a state is checked before it is written,
    /// because a file that will not open is worse than a write that refused.
    /// Neither belongs to the domain, so neither is decided here.
    ///
    /// # Errors
    ///
    /// Names the first project that is wrong and how, because an operator
    /// repairing a file needs to know which one.
    pub fn check(&self) -> Result<(), Inconsistent> {
        for (id, project) in &self.projects {
            if project.kits.is_empty() {
                return Err(Inconsistent::NoKits(*id));
            }
            for named in std::iter::once(project.foreman_kit.agent())
                .chain(project.kits.values().map(|offered| offered.kit.agent()))
            {
                if !self.agents.contains_key(&named) {
                    return Err(Inconsistent::UnconfiguredProjectAgent {
                        project: *id,
                        agent: named,
                    });
                }
            }
            for (platform, access) in &project.access {
                if let Access::Installation { id: installation } = access
                    && !self
                        .apps
                        .get(platform)
                        .is_some_and(|app| app.installations.contains_key(installation))
                {
                    return Err(Inconsistent::UnknownInstallation {
                        project: *id,
                        installation: *installation,
                    });
                }
            }
            for (channel, binding) in &project.channels {
                if let Binding::Workspace(team) = binding
                    && !self
                        .channel_apps
                        .get(channel)
                        .is_some_and(|app| app.workspaces.contains_key(team))
                {
                    return Err(Inconsistent::UnknownWorkspace {
                        project: *id,
                        workspace: team.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Every job this instance believes is still running.
    ///
    /// What a sweep puts back to work after the process supervising them died.
    /// Believed rather than observed: this says what the instance recorded, and
    /// whether a container is actually up is a question for the runtime — see
    /// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, where
    /// reconciling the two is startup's job rather than a state of its own.
    pub fn working(&self) -> impl Iterator<Item = JobId> + '_ {
        self.projects.values().flat_map(|project| {
            project
                .jobs
                .iter()
                .filter(|(_, job)| matches!(job.progress, Progress::Working))
                .map(|(id, _)| id.clone())
        })
    }

    /// Every job that is not over.
    ///
    /// What a sweep has to account for, which is wider than [`working`]: a job
    /// that is merely idle still owns a container, and a container that has
    /// gone missing means the same thing whether or not a turn was running in
    /// it. Retired jobs are excluded because there is nothing left of them to
    /// reconcile against.
    ///
    /// [`working`]: State::working
    pub fn unfinished(&self) -> impl Iterator<Item = JobId> + '_ {
        self.projects.values().flat_map(|project| {
            project
                .jobs
                .iter()
                .filter(|(_, job)| !job.progress.is_retired())
                .map(|(id, _)| id.clone())
        })
    }

    /// A job, whichever project it belongs to.
    ///
    /// Jobs are keyed inside their project, because a job belongs to exactly
    /// one and `docs/architecture.md` §2 leans on that. A sweep works from a
    /// container's name, which says the job and not the project, so it needs
    /// the search this does.
    #[must_use]
    pub fn job(&self, job: &JobId) -> Option<&Job> {
        self.projects
            .values()
            .find_map(|project| project.jobs.get(job))
    }

    /// Which project a job belongs to.
    ///
    /// The companion to [`State::job`], and needed for the same reason: a job
    /// is keyed inside its project, so anything arriving with only a job's
    /// identifier — a container's name, a thread a reply came in — has to
    /// search for the rest. Whoever speaks on that job's behalf needs the
    /// project, because the channel binding is the project's.
    #[must_use]
    pub fn project_of(&self, job: &JobId) -> Option<ProjectId> {
        self.projects
            .iter()
            .find(|(_, project)| project.jobs.contains_key(job))
            .map(|(id, _)| *id)
    }

    /// Which job holds a warrant, if one that is not over does.
    ///
    /// What the credential route asks of a bearer — see
    /// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
    /// A retired job's is refused here rather than removed from its record:
    /// its container is gone with everything in it, so nothing can present
    /// the warrant, and the record stays as it was written.
    #[must_use]
    pub fn job_with_warrant(&self, presented: &str) -> Option<(ProjectId, JobId)> {
        self.projects.iter().find_map(|(id, project)| {
            project
                .jobs
                .iter()
                .find(|(_, recorded)| {
                    !recorded.progress.is_retired()
                        && recorded
                            .warrant
                            .as_ref()
                            .is_some_and(|warrant| warrant.expose() == presented)
                })
                .map(|(job, _)| (*id, job.clone()))
        })
    }

    /// A job, for recording what became of it.
    pub fn job_mut(&mut self, job: &JobId) -> Option<&mut Job> {
        self.projects
            .values_mut()
            .find_map(|project| project.jobs.get_mut(job))
    }

    /// Who a message arriving on a channel is for.
    ///
    /// The rule in `docs/decisions/0031-a-mention-is-what-makes-it-ours.md`,
    /// as `docs/decisions/0060-a-binding-is-a-workspace.md` and
    /// `docs/decisions/0061-a-job-has-a-room-of-its-own.md` narrow it. The
    /// project is given rather than found: it is whichever project's socket
    /// the message arrived on. What is left to decide is whether the room it
    /// was said in is one of that project's jobs' — then it is that job's —
    /// and otherwise it is the foreman's, at the root or in a thread alike.
    /// Another app's message is the foreman's when the room is one the
    /// project watches, and nobody's otherwise — see
    /// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
    ///
    /// That a person's message mentions this instance is not consulted,
    /// because it is what made the message arrive: the reader hands over a
    /// person's words only when the platform delivered them as a mention, and
    /// the mention is still required inside a job's own thread, for the
    /// reason 0031 gives — people need to talk under a job without waking its
    /// agent. What *is* consulted, and first, is whether this instance said
    /// it, before anything else can match, so that a job's own thread is no
    /// exception.
    ///
    /// A room is matched on its channel as well as its identifier, because
    /// an identifier is only unique within one channel. Whether the message
    /// was in a thread decides nothing here: a job's room is that job's
    /// throughout, and where in it to answer is the deliverer's to carry.
    ///
    /// Note what is *not* consulted: whether the job is still running. A
    /// finished job's room still routes to it, and that is deliberate — the
    /// room is that job's conversation, and a person speaking in it a day
    /// later means the job. What happens when a job cannot take the message is
    /// a question for whoever delivers it, not for this.
    #[must_use]
    pub fn recipient(
        &self,
        project: ProjectId,
        channel: Channel,
        arriving: &Arriving<'_>,
    ) -> Recipient {
        self.recipient_among(&[project], channel, arriving)
    }

    /// Who a message is for, among the projects it could be for: one, for
    /// a message on a project's own app; every project bound to the
    /// workspace it came from, for one on the instance's app — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    ///
    /// The room decides among them, as it does for one: a job's room is
    /// that job's, a foreman's room and a watched room are that project's
    /// foreman's, and a room none of them owns is the foreman's where there
    /// is one project, and [`Recipient::Unowned`] where there are several.
    /// An app's message is a watching project's foreman's and nobody's
    /// otherwise, whatever the count.
    #[must_use]
    pub fn recipient_among(
        &self,
        candidates: &[ProjectId],
        channel: Channel,
        arriving: &Arriving<'_>,
    ) -> Recipient {
        // Nothing this instance said, before anything else can match, so that
        // a job's own room is no exception.
        if arriving.from_us {
            return Recipient::Nobody;
        }
        let in_room = |room: &Room| room.channel == channel && room.id == arriving.room;
        let projects: Vec<(ProjectId, &Project)> = candidates
            .iter()
            .filter_map(|id| Some((*id, self.projects.get(id)?)))
            .collect();
        // Another app's message is the foreman's exactly when the room is
        // watched, whatever room it is, and nobody's otherwise: an app
        // mentions nobody, so the room is the whole of the rule for it.
        if arriving.from_app {
            return projects
                .iter()
                .find(|(_, watched)| watched.watched.iter().any(in_room))
                .map_or(Recipient::Nobody, |(id, _)| Recipient::Foreman(*id));
        }
        for (id, watched) in &projects {
            if let Some((job, _)) = watched
                .jobs
                .iter()
                .find(|(_, job)| job.room.as_ref().is_some_and(in_room))
            {
                return Recipient::Job(job.clone());
            }
            if watched.foreman_room.as_ref().is_some_and(in_room)
                || watched.watched.iter().any(in_room)
            {
                return Recipient::Foreman(*id);
            }
        }
        match projects.as_slice() {
            [] => Recipient::Nobody,
            [(id, _)] => Recipient::Foreman(*id),
            several => Recipient::Unowned {
                among: several.iter().map(|(id, _)| *id).collect(),
            },
        }
    }

    /// Converts to the form that goes on disk, sealing every credential.
    ///
    /// Takes a source of nonces rather than generating them, because
    /// randomness is an effect and this crate takes none. A **fresh** nonce is
    /// consumed per credential per write, always: with this cipher, reusing one
    /// under the same key leaks the authentication key rather than merely one
    /// plaintext, so there is no such thing as a cheap reuse.
    ///
    /// A consequence worth knowing: because sealing happens on the way out,
    /// every write produces different ciphertext even when no credential
    /// changed. The values are opaque anyway, so what
    /// `docs/decisions/0011-state-is-a-snapshot-not-a-database.md` wanted from
    /// a readable file survives — but two snapshots are never byte-identical.
    ///
    /// # Errors
    ///
    /// Fails only if the cipher rejects an input, which for a well-formed key
    /// and nonce does not happen in practice.
    pub fn seal(
        &self,
        key: &Key,
        nonces: &mut impl FnMut() -> Nonce,
    ) -> Result<Snapshot, SealError> {
        let agents = self
            .agents
            .iter()
            .map(|(agent, config)| {
                Ok((
                    *agent,
                    SealedAgentConfig {
                        auth_token: config.auth_token.seal(key, nonces())?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SealError>>()?;

        let apps = sealed_apps(&self.apps, key, nonces)?;

        let projects = self
            .projects
            .iter()
            .map(|(id, project)| {
                let credentials = project
                    .access
                    .iter()
                    .map(|(platform, access)| Ok((*platform, access.seal(key, nonces)?)))
                    .collect::<Result<BTreeMap<_, _>, SealError>>()?;
                let channels = project
                    .channels
                    .iter()
                    .map(|(channel, binding)| {
                        Ok((
                            *channel,
                            match binding {
                                Binding::Own(config) => SealedBinding::Own(SealedChannelConfig {
                                    credential: config.credential.seal(key, nonces())?,
                                    listen_credential: Some(
                                        config.listen_credential.seal(key, nonces())?,
                                    ),
                                }),
                                Binding::Workspace(team) => SealedBinding::Workspace {
                                    workspace: team.clone(),
                                },
                            },
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, SealError>>()?;
                // The name travels in the clear beside its sealed value, on
                // the same terms a channel's address does: a name is not a
                // credential, and sealing it would cost a nonce per write to
                // hide something the operator typed in order to read it back.
                let variables = project
                    .variables
                    .iter()
                    .map(|(name, variable)| {
                        Ok((
                            name.to_string(),
                            SealedVariable {
                                value: variable.value.seal(key, nonces())?,
                                note: variable.note.clone(),
                            },
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, SealError>>()?;
                let jobs = project
                    .jobs
                    .iter()
                    .map(|(job, recorded)| Ok((job.clone(), recorded.seal(key, nonces)?)))
                    .collect::<Result<BTreeMap<_, _>, SealError>>()?;
                Ok((
                    *id,
                    SealedProject {
                        name: project.name.clone(),
                        repository: project.repository.https(),
                        foreman_kit: project.foreman_kit.clone(),
                        // Names in the clear beside their kits, on the terms a
                        // variable's name travels: a name is not a credential,
                        // and the operator typed it in order to read it back.
                        kits: project
                            .kits
                            .iter()
                            .map(|(name, offered)| (name.to_string(), offered.clone()))
                            .collect(),
                        credentials,
                        channels,
                        variables,
                        brief: project.brief.clone(),
                        watched: project.watched.clone(),
                        foreman_room: project.foreman_room.clone(),
                        jobs,
                        attending: project.attending.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SealError>>()?;

        let channel_apps = sealed_channel_apps(&self.channel_apps, key, nonces)?;

        Ok(Snapshot {
            // Left for whoever holds the file to fill in, for the reason the
            // field itself gives: this crate has no identity of its own to
            // write here, and inventing one would need randomness.
            instance: None,
            agents,
            projects,
            apps,
            channel_apps,
        })
    }
}

/// The key a snapshot's credentials are sealed under.
///
/// Supplied by the environment at startup, and never stored beside the file it
/// protects — that is what makes the file portable and useless on its own.
/// Redacts when formatted, for exactly the reason a credential does.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl Key {
    /// Wraps key material.
    #[must_use]
    pub const fn new(material: [u8; 32]) -> Self {
        Self(material)
    }

    /// Parses key material supplied as base64.
    ///
    /// Parsing is pure, so it lives with the type rather than with whatever
    /// reads the environment — which keeps the one place that knows what a key
    /// looks like from being the same place that knows where it comes from.
    ///
    /// # Errors
    ///
    /// Fails if the text is not base64, or does not decode to exactly the
    /// right number of bytes. Neither message repeats the input.
    pub fn from_base64(text: &str) -> Result<Self, KeyError> {
        let decoded = BASE64.decode(text).map_err(|_| KeyError::Encoding)?;
        let material: [u8; 32] = decoded.try_into().map_err(|_| KeyError::Length)?;
        Ok(Self(material))
    }

    /// The key as base64, which is the one form anything outside this crate
    /// ever writes down.
    ///
    /// The exact inverse of [`Key::from_base64`], and here rather than beside
    /// whoever stores one for the same reason: what a key looks like is this
    /// type's business, and a second encoder somewhere else is a second thing
    /// that could disagree about padding or alphabet.
    ///
    /// It hands back the material in the clear, which is why this is a named
    /// method and not a [`fmt::Display`] — that one redacts, and must, since
    /// `docs/conventions.md` §4 makes formatting the place secrets escape.
    /// Writing a key down is a deliberate act with exactly one caller: the
    /// startup that generated it, per
    /// `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md`.
    #[must_use]
    pub fn to_base64(&self) -> String {
        BASE64.encode(self.0)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&CipherKey::<Aes256Gcm>::from(self.0))
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(<redacted>)")
    }
}

/// Key material could not be read.
///
/// Deliberately says nothing about the input, since a malformed key is often a
/// nearly-correct one and an error message is a place secrets escape.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// The text is not valid base64.
    #[error("the key is not valid base64")]
    Encoding,
    /// The text decoded to the wrong number of bytes.
    #[error("the key must decode to exactly 32 bytes")]
    Length,
}

/// A credential could not be sealed.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// The cipher rejected the input.
    #[error("a credential could not be sealed")]
    Cipher,
}

/// An instance's state is not internally consistent.
///
/// One type for every way a state can be wrong, because there is one definition
/// of valid and several places that need to ask: a file on the way in, a state
/// on the way out, and every operation that could break it. Two definitions
/// would eventually disagree, and the one that let something through would be
/// the one nobody was reading.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Inconsistent {
    /// A project has no kit its jobs can run on.
    ///
    /// A project whose jobs cannot run cannot do the one thing a project is
    /// for, which is why this is a broken instance rather than an unusual one.
    #[error("project {0} has no kit its jobs can run on")]
    NoKits(ProjectId),
    /// A project names an agent this instance does not configure.
    #[error("project {project} names agent {agent:?}, which is not configured")]
    UnconfiguredProjectAgent {
        /// The project holding the dangling reference.
        project: ProjectId,
        /// The agent it names.
        agent: Agent,
    },
    /// A project reaches its repository through an installation the App
    /// does not hold — or holds no App at all.
    #[error(
        "project {project} names installation {installation}, which the App is not installed as"
    )]
    UnknownInstallation {
        /// The project holding the dangling reference.
        project: ProjectId,
        /// The installation it names.
        installation: u64,
    },
    /// A project speaks through a workspace the instance's app is not
    /// installed on — or holds no app at all — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    #[error("project {project} names workspace {workspace}, which the app is not installed on")]
    UnknownWorkspace {
        /// The project holding the dangling reference.
        project: ProjectId,
        /// The workspace it names.
        workspace: String,
    },
}

/// A snapshot could not be turned back into state.
///
/// Every variant is deliberately vague about *which* credential, and says
/// nothing about its contents: an error message is a place secrets escape.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// A stored credential is not valid base64.
    #[error("a stored credential is not valid base64")]
    Encoding,
    /// A stored nonce is the wrong length.
    #[error("a stored nonce is the wrong length")]
    NonceLength,
    /// Decryption failed.
    #[error("a credential could not be decrypted: wrong key, or the file was altered")]
    Cipher,
    /// A credential decrypted to something that is not text.
    #[error("a credential decrypted to bytes that are not text")]
    NotText,
    /// The snapshot decrypted, and describes an instance that cannot be.
    ///
    /// A file is untrusted input — hand-edited, half-written, or written by an
    /// older version — so this is where believing it stops.
    #[error("the snapshot describes an instance that is not internally consistent")]
    Inconsistent(#[source] Inconsistent),
    /// A project holds text where an address on the platform has to be —
    /// see `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
    ///
    /// Names the project and the rule broken, and not the text, on the
    /// terms every variant here keeps: a file is untrusted input.
    #[error("the project {project} holds a repository that is not an address on GitHub: {why}")]
    Repository {
        /// The project, by name.
        project: String,
        /// Which rule the text broke.
        #[source]
        why: RepositoryError,
    },
    /// A project names a variable a container could not be given.
    ///
    /// Names the rule and never the value, for the reason every variant here
    /// is vague about which credential: an error message is a place secrets
    /// escape.
    #[error("the snapshot names a variable that could not be delivered")]
    VariableName(#[source] VariableNameError),
    /// A project offers a kit under a name that is not one.
    ///
    /// The same boundary as the variable above: a name in a file is checked
    /// as the file is opened, because a file may have been edited by hand.
    #[error("the snapshot names a kit under a name that is not one")]
    KitName(#[source] KitNameError),
}

/// One of a project's variables: its value, and what it is for.
///
/// The value is a [`Secret`] and never read here, per
/// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.
/// The note is the operator's words on what the variable is for, told to a
/// job's agent beside the name and read by nothing here either — see
/// `docs/decisions/0075-a-variable-says-what-it-is-for.md`. It is kept in
/// the clear, on the terms the name is: prose the operator typed in order
/// to have it read back, and never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// What it is set to.
    pub value: Secret,
    /// What it is for, in the operator's words; empty when nobody said.
    pub note: String,
}

impl Variable {
    /// A variable with a value and nothing said about it.
    #[must_use]
    pub const fn unexplained(value: Secret) -> Self {
        Self {
            value,
            note: String::new(),
        }
    }
}

/// A variable as it appears on disk: its value sealed, its note in the
/// clear beside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedVariable {
    /// The value, sealed.
    pub value: SealedSecret,
    /// What it is for, as the operator wrote it.
    #[serde(default)]
    pub note: String,
}

/// A variable in either shape a snapshot may hold: with its note, or as the
/// bare sealed value the last release wrote, which opens as one with no
/// note — the bridge `docs/conventions.md` §4 asks for, from that release's
/// shape and no older.
#[derive(Deserialize)]
#[serde(untagged)]
enum SealedVariableOrOlder {
    /// The shape written now.
    Noted(SealedVariable),
    /// The shape the last release wrote: the sealed value alone.
    Bare(SealedSecret),
}

/// Reads a project's variables in either shape.
fn variables_or_older<'de, D>(deserializer: D) -> Result<BTreeMap<String, SealedVariable>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let read: BTreeMap<String, SealedVariableOrOlder> = BTreeMap::deserialize(deserializer)?;
    Ok(read
        .into_iter()
        .map(|(name, either)| {
            let variable = match either {
                SealedVariableOrOlder::Noted(variable) => variable,
                SealedVariableOrOlder::Bare(value) => SealedVariable {
                    value,
                    note: String::new(),
                },
            };
            (name, variable)
        })
        .collect())
}

/// A credential as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecret {
    /// The nonce this was sealed with, base64.
    pub nonce: String,
    /// The ciphertext and its authentication tag, base64.
    pub ciphertext: String,
}

impl Secret {
    /// Seals this credential for storage.
    ///
    /// # Errors
    ///
    /// Fails only if the cipher rejects the input.
    pub fn seal(&self, key: &Key, nonce: Nonce) -> Result<SealedSecret, SealError> {
        let ciphertext = key
            .cipher()
            .encrypt(&CipherNonce::from(nonce), self.0.as_bytes())
            .map_err(|_| SealError::Cipher)?;
        Ok(SealedSecret {
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(ciphertext),
        })
    }
}

impl SealedSecret {
    /// Recovers the credential.
    ///
    /// # Errors
    ///
    /// Fails if the stored encoding is malformed, or if decryption fails —
    /// which means the key is wrong or the file was altered. The cipher
    /// authenticates, so a tampered snapshot is a failure rather than a
    /// plausible-looking wrong answer.
    pub fn open(&self, key: &Key) -> Result<Secret, OpenError> {
        let nonce = BASE64
            .decode(&self.nonce)
            .map_err(|_| OpenError::Encoding)?;
        let nonce: Nonce = nonce.try_into().map_err(|_| OpenError::NonceLength)?;
        let ciphertext = BASE64
            .decode(&self.ciphertext)
            .map_err(|_| OpenError::Encoding)?;
        let plaintext = key
            .cipher()
            .decrypt(&CipherNonce::from(nonce), ciphertext.as_slice())
            .map_err(|_| OpenError::Cipher)?;
        String::from_utf8(plaintext)
            .map(Secret::new)
            .map_err(|_| OpenError::NotText)
    }
}

/// How a project reaches a platform, as it appears on disk: a token
/// sealed, or an installation by identifier.
///
/// Untagged, and told apart by shape: a token with what was read of it
/// has its secret under a name, a token as the last release wrote it is
/// the bare sealed secret, and an installation is the one other object,
/// with its one field. The bare form is read and never written, so a
/// file upgrades itself on its first change, per `docs/conventions.md`
/// §4; the two facts beside a token are defaulted, since a token kept
/// before they were read genuinely has none.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SealedAccess {
    /// A token, sealed, with what the platform said of it.
    Token {
        /// The token, sealed.
        secret: SealedSecret,
        /// Whose it is, in the clear: an account's name is not a secret.
        #[serde(default)]
        owner: Option<String>,
        /// When the platform will stop accepting it, in the clear.
        #[serde(default)]
        expires: Option<Timestamp>,
    },
    /// A token as the last release wrote one: the bare sealed secret.
    Bare(SealedSecret),
    /// An installation of the App, by identifier, which needs no sealing.
    Installation {
        /// The installation's identifier on the platform.
        installation: u64,
    },
}

impl Access {
    /// Converts to the form that goes on disk, sealing a token; an
    /// installation is its identifier and needs no sealing.
    ///
    /// # Errors
    ///
    /// Fails only if the cipher rejects the input.
    pub fn seal(
        &self,
        key: &Key,
        nonces: &mut impl FnMut() -> Nonce,
    ) -> Result<SealedAccess, SealError> {
        Ok(match self {
            Self::Token {
                secret,
                owner,
                expires,
            } => SealedAccess::Token {
                secret: secret.seal(key, nonces())?,
                owner: owner.clone(),
                expires: *expires,
            },
            Self::Installation { id } => SealedAccess::Installation { installation: *id },
        })
    }
}

impl SealedAccess {
    /// Recovers the access, opening a token.
    ///
    /// # Errors
    ///
    /// Fails if a token cannot be recovered, which means the key is wrong or
    /// the file was altered.
    pub fn open(self, key: &Key) -> Result<Access, OpenError> {
        Ok(match self {
            Self::Token {
                secret,
                owner,
                expires,
            } => Access::Token {
                secret: secret.open(key)?,
                owner,
                expires,
            },
            Self::Bare(sealed) => Access::Token {
                secret: sealed.open(key)?,
                owner: None,
                expires: None,
            },
            Self::Installation { installation } => Access::Installation { id: installation },
        })
    }
}

/// An agent's configuration as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedAgentConfig {
    /// The sealed credential.
    pub auth_token: SealedSecret,
}

/// An App this instance owns, as it appears on disk: everything in the
/// clear but the key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedPlatformApp {
    /// The App's identifier on the platform.
    pub id: u64,
    /// Its slug.
    pub slug: String,
    /// Its client identifier.
    pub client_id: String,
    /// Its private key, sealed.
    pub private_key: SealedSecret,
    /// Where it is installed, in the clear. Defaulted, because a file
    /// written before installations were kept described an App installed
    /// nowhere this instance knew of, which is the true answer — see
    /// `docs/conventions.md` §4.
    #[serde(default)]
    pub installations: BTreeMap<u64, Installation>,
}

/// A Slack app this instance owns, as it appears on disk.
///
/// The client identifier in the clear, its two secrets sealed, and its
/// workspaces under it — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedChannelApp {
    /// Its client identifier.
    pub client_id: String,
    /// Its client secret, sealed.
    pub client_secret: SealedSecret,
    /// Its app-level token, sealed.
    pub app_token: SealedSecret,
    /// Where it is installed. Defaulted, because a file written before
    /// workspaces were kept described an app installed nowhere this
    /// instance knew of, which is the true answer — see
    /// `docs/conventions.md` §4.
    #[serde(default)]
    pub workspaces: BTreeMap<String, SealedWorkspace>,
}

/// One workspace the app is installed in, as it appears on disk: its name
/// and bot user in the clear, its bot token sealed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedWorkspace {
    /// The workspace's name.
    pub name: String,
    /// The bot user the install made.
    pub bot_user: String,
    /// The bot token minted for the install, sealed.
    pub bot_token: SealedSecret,
}

/// A binding as it appears on disk.
///
/// An app of the project's own, written as the last release wrote it, or a
/// workspace of the instance's app by its identifier — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
/// Untagged, so that the pair the last release wrote reads as the first
/// shape with nothing added, and the second is told apart by the one field
/// it has.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SealedBinding {
    /// An app of the project's own.
    Own(SealedChannelConfig),
    /// A workspace of the instance's app.
    Workspace {
        /// The workspace's identifier on the platform.
        workspace: String,
    },
}

/// A channel binding as it appears on disk.
///
/// The last release wrote an `address` beside these, the one room a project
/// was listened to in; it is read past and dropped, because nothing names a
/// room for a project any more — see
/// `docs/decisions/0061-a-job-has-a-room-of-its-own.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedChannelConfig {
    /// The sealed credential.
    pub credential: SealedSecret,
    /// The sealed credential for listening.
    ///
    /// Always written, since
    /// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`,
    /// and still read as optional, because the last release wrote none for a
    /// project that did not listen — `docs/conventions.md` §4. A binding read
    /// without one is dropped on opening rather than carried, and the project
    /// is kept.
    #[serde(default)]
    pub listen_credential: Option<SealedSecret>,
}

/// A job as it appears on disk: everything in the clear but the warrant.
///
/// The bridges from what the last release wrote live here rather than on
/// [`Job`], because this is the shape a file is read into: a job's
/// progress under its older spellings, and every field a job written before
/// it existed lacks, defaulted per `docs/conventions.md` §4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedJob {
    /// What it runs on. Written as `kit`, and read as the bare agent name
    /// too, for the reason [`Job`] gives.
    pub kit: Kit,
    /// Why the foreman started it.
    pub reason: String,
    /// The instruction the agent begins from.
    pub kickoff: String,
    /// When the record was created.
    pub created_at: Timestamp,
    /// Where it has got to, read through the bridge from the last release's
    /// spellings.
    #[serde(deserialize_with = "progress_or_older")]
    pub progress: Progress,
    /// When its progress last changed. None for every job the last release
    /// wrote, which is why it is defaulted.
    #[serde(default)]
    pub since: Option<Timestamp>,
    /// The room its conversation happens in. Absent for a job whose room
    /// could not be created, and for every job the last release wrote, which
    /// is why it is defaulted; the `thread` that release wrote instead is
    /// read past and dropped.
    #[serde(default)]
    pub room: Option<Room>,
    /// Who asked for it. None for a job started from the dashboard, and for
    /// every job the last release wrote, which is why it is defaulted.
    #[serde(default)]
    pub asked_by: Option<String>,
    /// What the agent's session reported it was set to. Empty for a job
    /// that has not had a turn, and for every job recorded before this
    /// existed, which is why it is defaulted.
    #[serde(default)]
    pub reported: BTreeMap<String, String>,
    /// The messages it has been sent and has not finished with. Empty for
    /// every job the last release wrote, which is why it is defaulted.
    #[serde(default)]
    pub inbox: Inbox,
    /// The pull requests it said it opened. Empty for a job that claimed
    /// none, and for every job the last release wrote, which is why it is
    /// defaulted.
    #[serde(default)]
    pub pull_requests: BTreeSet<u64>,
    /// Its warrant, sealed. None for every job the last release wrote,
    /// which is the true answer rather than a substitute for one: such a
    /// job's container was created with the credential in its environment,
    /// and there is no warrant to invent for it.
    #[serde(default)]
    pub warrant: Option<SealedSecret>,
}

impl Job {
    /// Converts to the form that goes on disk, sealing the warrant.
    ///
    /// # Errors
    ///
    /// Fails only if the cipher rejects the input.
    pub fn seal(
        &self,
        key: &Key,
        nonces: &mut impl FnMut() -> Nonce,
    ) -> Result<SealedJob, SealError> {
        Ok(SealedJob {
            kit: self.kit.clone(),
            reason: self.reason.clone(),
            kickoff: self.kickoff.clone(),
            created_at: self.created_at,
            progress: self.progress.clone(),
            since: self.since,
            room: self.room.clone(),
            asked_by: self.asked_by.clone(),
            reported: self.reported.clone(),
            inbox: self.inbox.clone(),
            pull_requests: self.pull_requests.clone(),
            warrant: self
                .warrant
                .as_ref()
                .map(|warrant| warrant.seal(key, nonces()))
                .transpose()?,
        })
    }
}

impl SealedJob {
    /// Recovers the record, its warrant opened.
    ///
    /// # Errors
    ///
    /// Fails if the warrant cannot be recovered, which means the key is
    /// wrong or the file was altered.
    pub fn open(self, key: &Key) -> Result<Job, OpenError> {
        Ok(Job {
            kit: self.kit,
            reason: self.reason,
            kickoff: self.kickoff,
            created_at: self.created_at,
            progress: self.progress,
            since: self.since,
            room: self.room,
            asked_by: self.asked_by,
            reported: self.reported,
            inbox: self.inbox,
            pull_requests: self.pull_requests,
            warrant: self.warrant.map(|sealed| sealed.open(key)).transpose()?,
        })
    }
}

/// A project as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedProject {
    /// What to call it.
    pub name: String,
    /// Where the repository lives, as the address this project spells.
    /// Text on disk, as the last release wrote it, and an address once
    /// opened: text that is not one refuses the file — see
    /// `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
    pub repository: String,
    /// The kit its foreman thinks with.
    ///
    /// Read under three spellings. The oldest files say `orchestrator_agent`,
    /// from before `docs/decisions/0030-the-orchestrator-is-a-foreman.md`;
    /// the ones after say `foreman_agent`; and both hold a bare agent name,
    /// which reads as that agent's defaults through the same reader a job's
    /// kit uses and for the same reason — nothing else existed to think with.
    /// A rename of a serialised name is never free, per `docs/conventions.md`
    /// §4, which gained that rule from this exact field failing to open a
    /// real instance. Written under this name from now on, so a file upgrades
    /// itself on its first change.
    #[serde()]
    pub foreman_kit: Kit,
    /// The kits its jobs may run on, keyed by plain text.
    ///
    /// Plain text rather than the validated name, for the reason the
    /// variables below are: a file is untrusted input, so a name is checked as
    /// the snapshot is opened rather than trusted because something once
    /// checked it. Defaulted because it was added after snapshots existed,
    /// and empty is the true answer for a file that carries `job_agents`
    /// instead — the open path reads one from the other.
    #[serde(default)]
    pub kits: BTreeMap<String, KitConfig>,
    /// The operator's brief for its foreman, in the clear: written to be
    /// read, like a kit's description.
    ///
    /// Defaulted, because it was added after snapshots existed, and empty
    /// is the true answer for a file from before: nobody had written one.
    #[serde(default)]
    pub brief: String,
    /// How it reaches each platform: a token, sealed, or an installation
    /// by identifier. Under the name the last release wrote, whose bare
    /// sealed token still opens as one.
    pub credentials: BTreeMap<Platform, SealedAccess>,
    /// Its channel bindings, each with its credential sealed.
    ///
    /// **Defaulted, because this field was added after snapshots existed.**
    /// `docs/decisions/0011-state-is-a-snapshot-not-a-database.md` versions
    /// nothing and says what that costs: an added field is free *with a
    /// default*, and without one an existing file stops loading — which loses
    /// everything, since there is only the one file.
    ///
    /// This is not the substituted default `.quality/gate-reference.md`
    /// forbids. That rule is about replacing a failure with a guess; here the
    /// empty map is the true answer, because a file written before channels
    /// existed described a project that had none, and a project with none is
    /// valid.
    #[serde(default)]
    pub channels: BTreeMap<Channel, SealedBinding>,
    /// The rooms its foreman watches, which hold nothing needing sealing.
    ///
    /// Defaulted, because it was added after snapshots existed, and empty
    /// is the true answer for a file from before: nothing was watched.
    #[serde(default)]
    pub watched: BTreeSet<Room>,
    /// The room its foreman's transcript is posted in, which holds nothing
    /// needing sealing. Defaulted, because it was added after snapshots
    /// existed, and none is the true answer for a file from before: no such
    /// room had been made.
    #[serde(default)]
    pub foreman_room: Option<Room>,
    /// Its variables, each with its value sealed and its name in the clear.
    ///
    /// Keyed by plain text rather than by the validated name, deliberately:
    /// a file is untrusted input, so a name is checked as the snapshot is
    /// opened rather than trusted because something once checked it. That is
    /// the same boundary a credential crosses, and it is what lets
    /// `VariableName` implement no deserialisation at all.
    ///
    /// **Defaulted, because this field was added after snapshots existed.**
    /// `docs/decisions/0011-state-is-a-snapshot-not-a-database.md` versions
    /// nothing: an added field is free *with* a default and loses every
    /// existing instance without one. The empty map is the true answer rather
    /// than a substitute for one, because a file written before variables
    /// existed described a project that had none. Read in either shape: the
    /// value with its note, or the bare sealed value the last release wrote.
    #[serde(default, deserialize_with = "variables_or_older")]
    pub variables: BTreeMap<String, SealedVariable>,
    /// Its jobs, each with its warrant sealed.
    pub jobs: BTreeMap<JobId, SealedJob>,
    /// What its foreman was doing, which holds nothing needing sealing either:
    /// a message from a person is not a credential.
    ///
    /// Defaulted, because every snapshot written before foremen had an inbox
    /// has no such field — `docs/conventions.md` §4. Read through a bridge as
    /// well: an inbox the last release wrote holds threads that name no
    /// room, and opens as idle. The messages in it are lost, which §4
    /// permits — a restart during an upgrade with a message in hand is the
    /// whole of the window — and the alternative was a file that would not
    /// open.
    #[serde(default, deserialize_with = "attending_or_older")]
    pub attending: Attending,
}

/// Everything one instance knows, as it appears on disk.
///
/// A separate type from [`State`] rather than the same one behind a flag,
/// because the boundary between them does real work: a file is untrusted input
/// — hand-edited, half-written, or written by an older version — and turning
/// one into state is the moment to find out whether it can be believed. What
/// comes out the far side has already been checked, so nothing downstream
/// handles a reference that does not resolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Which instance this file belongs to.
    ///
    /// **Deliberately not on [`State`].** Nothing in the domain reads it: it
    /// answers a question about *containers*, which is the runtime's world and
    /// not this crate's, and putting it in the state would mean every place
    /// that builds one has to invent an identity it will never look at. So it
    /// lives on the file, and whoever operates the file holds the value.
    ///
    /// Optional because a file written before this existed has none, and
    /// because this crate cannot mint one — that needs randomness, and the
    /// rule against effects here is what keeps every identifier arriving from
    /// outside. Whoever opens such a file gives it one, and the next write
    /// keeps it.
    #[serde(default)]
    pub instance: Option<InstanceId>,
    /// The configured agents, sealed.
    pub agents: BTreeMap<Agent, SealedAgentConfig>,
    /// The projects, sealed.
    pub projects: BTreeMap<ProjectId, SealedProject>,
    /// The Apps the instance owns, sealed. Defaulted, because a file the
    /// last release wrote has none, which is the true answer: it had none.
    #[serde(default)]
    pub apps: BTreeMap<Platform, SealedPlatformApp>,
    /// The apps the instance owns on each channel, sealed. Defaulted,
    /// because a file the last release wrote has none, which is the true
    /// answer.
    #[serde(default)]
    pub channel_apps: BTreeMap<Channel, SealedChannelApp>,
}

/// The Apps of a file, their keys opened.
fn opened_apps(
    apps: BTreeMap<Platform, SealedPlatformApp>,
    key: &Key,
) -> Result<BTreeMap<Platform, PlatformApp>, OpenError> {
    apps.into_iter()
        .map(|(platform, sealed)| {
            Ok((
                platform,
                PlatformApp {
                    id: sealed.id,
                    slug: sealed.slug,
                    client_id: sealed.client_id,
                    private_key: sealed.private_key.open(key)?,
                    installations: sealed.installations,
                },
            ))
        })
        .collect()
}

/// The Apps of a state, their keys sealed for the file.
fn sealed_apps(
    apps: &BTreeMap<Platform, PlatformApp>,
    key: &Key,
    nonces: &mut impl FnMut() -> Nonce,
) -> Result<BTreeMap<Platform, SealedPlatformApp>, SealError> {
    apps.iter()
        .map(|(platform, app)| {
            Ok((
                *platform,
                SealedPlatformApp {
                    id: app.id,
                    slug: app.slug.clone(),
                    client_id: app.client_id.clone(),
                    private_key: app.private_key.seal(key, nonces())?,
                    installations: app.installations.clone(),
                },
            ))
        })
        .collect()
}

/// The channel apps of a state, their secrets sealed for the file.
fn sealed_channel_apps(
    apps: &BTreeMap<Channel, ChannelApp>,
    key: &Key,
    nonces: &mut impl FnMut() -> Nonce,
) -> Result<BTreeMap<Channel, SealedChannelApp>, SealError> {
    apps.iter()
        .map(|(channel, app)| {
            let workspaces = app
                .workspaces
                .iter()
                .map(|(id, workspace)| {
                    Ok((
                        id.clone(),
                        SealedWorkspace {
                            name: workspace.name.clone(),
                            bot_user: workspace.bot_user.clone(),
                            bot_token: workspace.bot_token.seal(key, nonces())?,
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, SealError>>()?;
            Ok((
                *channel,
                SealedChannelApp {
                    client_id: app.client_id.clone(),
                    client_secret: app.client_secret.seal(key, nonces())?,
                    app_token: app.app_token.seal(key, nonces())?,
                    workspaces,
                },
            ))
        })
        .collect()
}

/// The channel apps of a file, their secrets opened.
fn opened_channel_apps(
    apps: BTreeMap<Channel, SealedChannelApp>,
    key: &Key,
) -> Result<BTreeMap<Channel, ChannelApp>, OpenError> {
    apps.into_iter()
        .map(|(channel, sealed)| {
            let workspaces = sealed
                .workspaces
                .into_iter()
                .map(|(id, workspace)| {
                    Ok((
                        id,
                        Workspace {
                            name: workspace.name,
                            bot_user: workspace.bot_user,
                            bot_token: workspace.bot_token.open(key)?,
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
            Ok((
                channel,
                ChannelApp {
                    client_id: sealed.client_id,
                    client_secret: sealed.client_secret.open(key)?,
                    app_token: sealed.app_token.open(key)?,
                    workspaces,
                },
            ))
        })
        .collect()
}

impl SealedProject {
    /// Decrypts and validates one project: every credential recovered,
    /// every name and the repository's address believed only once checked.
    ///
    /// # Errors
    ///
    /// Fails if a credential cannot be recovered, if a variable's or a
    /// kit's name could not be delivered, or if the repository is text that
    /// is not an address on the platform.
    pub fn open(self, key: &Key) -> Result<Project, OpenError> {
        let access = self
            .credentials
            .into_iter()
            .map(|(platform, sealed)| Ok((platform, sealed.open(key)?)))
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
        // A binding the last release wrote without the credential that
        // listens is no binding, per 0059: the project is kept and told so
        // at startup, rather than the file refused.
        let channels = self
            .channels
            .into_iter()
            .filter_map(|(channel, sealed)| match sealed {
                SealedBinding::Own(sealed) => {
                    let listening = sealed.listen_credential?;
                    Some((channel, Some((sealed.credential, listening)), None))
                }
                SealedBinding::Workspace { workspace } => Some((channel, None, Some(workspace))),
            })
            .map(|(channel, own, workspace)| {
                Ok((
                    channel,
                    match (own, workspace) {
                        (Some((credential, listening)), _) => Binding::Own(ChannelConfig {
                            credential: credential.open(key)?,
                            listen_credential: listening.open(key)?,
                        }),
                        (None, Some(team)) => Binding::Workspace(team),
                        (None, None) => return Err(OpenError::Encoding),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
        // Where a name stops being believed. A snapshot may be hand-edited,
        // and a name that cannot be delivered has to be refused here rather
        // than reaching an argument list.
        let variables = self
            .variables
            .into_iter()
            .map(|(name, sealed)| {
                let name = VariableName::new(name).map_err(OpenError::VariableName)?;
                Ok((
                    name,
                    Variable {
                        value: sealed.value.open(key)?,
                        note: sealed.note,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
        // Where a kit's name stops being believed, on the same terms.
        let kits = self
            .kits
            .into_iter()
            .map(|(name, offered)| {
                let name = KitName::new(name).map_err(OpenError::KitName)?;
                Ok((name, offered))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
        let jobs = self
            .jobs
            .into_iter()
            .map(|(job, sealed)| Ok((job, sealed.open(key)?)))
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;
        // Where the repository's text stops being believed: an address is
        // what everything downstream composes from — see
        // `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
        let repository =
            RepositoryAddress::parse(&self.repository).map_err(|why| OpenError::Repository {
                project: self.name.clone(),
                why,
            })?;
        Ok(Project {
            name: self.name,
            repository,
            foreman_kit: self.foreman_kit,
            kits,
            access,
            channels,
            variables,
            brief: self.brief,
            watched: self.watched,
            foreman_room: self.foreman_room,
            jobs,
            attending: self.attending,
        })
    }
}

impl Snapshot {
    /// Decrypts and validates, yielding state that can be relied on.
    ///
    /// # Errors
    ///
    /// Fails if any credential cannot be recovered, or if the snapshot is
    /// internally inconsistent — currently, if the agent it names as the
    /// foreman's has no configuration. That check is what lets every
    /// later caller look that agent up without handling an absence.
    pub fn open(self, key: &Key) -> Result<State, OpenError> {
        let Self {
            instance: _,
            agents,
            projects,
            apps,
            channel_apps,
        } = self;

        let agents = agents
            .into_iter()
            .map(|(agent, config)| {
                Ok((
                    agent,
                    AgentConfig {
                        auth_token: config.auth_token.open(key)?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;

        let apps = opened_apps(apps, key)?;
        let channel_apps = opened_channel_apps(channel_apps, key)?;

        let projects = projects
            .into_iter()
            .map(|(id, project)| Ok((id, project.open(key)?)))
            .collect::<Result<BTreeMap<_, _>, OpenError>>()?;

        let state = State {
            agents,
            projects,
            apps,
            channel_apps,
        };
        // A file is untrusted input, so this is where believing it stops.
        state.check().map_err(OpenError::Inconsistent)?;
        Ok(state)
    }
}

/// What one process is given in order to speak on a channel.
///
/// A narrower thing than [`ChannelConfig`], and narrower on purpose: a binding
/// holds two credentials and only one of them belongs anywhere near a job.
/// [`Handout`] carries this rather than the binding, so the credential that
/// opens an event stream has nowhere to travel to — which is the property
/// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md` claims, made true
/// by there being no field for it rather than by remembering to strip one.
#[derive(Debug, Clone)]
pub struct Speaking {
    /// What to authenticate with. Where to speak is not here: since
    /// `docs/decisions/0060-a-binding-is-a-workspace.md` a room is named by
    /// the conversation, not by the binding, and a thread carries its own.
    pub credential: Secret,
}

impl ChannelConfig {
    /// The half of this binding a process may be handed.
    #[must_use]
    pub fn speaking(&self) -> Speaking {
        Speaking {
            credential: self.credential.clone(),
        }
    }
}

/// A message that arrived on a channel, as much of it as routing needs.
///
/// Deliberately not the platform's own event type. What decides where a message
/// goes is three facts, and taking only those keeps the deciding in this crate
/// — which has no I/O and can therefore be tested against every combination
/// rather than against whichever ones a live workspace happens to produce.
///
/// That a person's message mentions this instance is not among them, since
/// `docs/decisions/0060-a-binding-is-a-workspace.md`: it is what made the
/// message arrive, because a person is read from the platform's own mention
/// event and from nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arriving<'a> {
    /// The room it was said in, as the platform names it.
    pub room: &'a str,
    /// What identifies this message itself.
    ///
    /// Needed because a message at the root *is* the parent of the thread a
    /// foreman answers in — there is nothing to create. A job's thread had to
    /// be opened by posting a message for it to hang from; a foreman's arrives
    /// already made, and this is its identifier.
    pub id: &'a str,
    /// The thread it was in, if it was in one at all.
    pub thread: Option<&'a str>,
    /// Whether this instance is what said it.
    ///
    /// Carried into the decision rather than filtered before it, because
    /// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md` calls this
    /// load-bearing and a line in an I/O function is the easiest kind to
    /// delete. An agent posting a question produces an event; routed back to
    /// that agent it answers, producing another. The loop costs a model call
    /// per lap and would be found on an invoice.
    pub from_us: bool,
    /// Whether another app is what said it.
    ///
    /// An app's message is read by a different rule from a person's: the
    /// room decides, not a mention — see
    /// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
    pub from_app: bool,
}

/// Who an arriving message is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    /// The job whose room it arrived in.
    Job(JobId),
    /// The foreman of the project whose socket it arrived on: a mention in
    /// any room that is no job's, at the root or in a thread — which is
    /// where a person lands by replying to something the foreman said, and
    /// which used to be answered with a fixed sentence.
    Foreman(ProjectId),
    /// Nobody at all, and this is the ordinary answer rather than a failure:
    /// what this instance said itself, heard back, or a message for a project
    /// this instance does not watch.
    Nobody,
    /// A person's mention in a room none of several projects owns, on a
    /// workspace they share: answered where it was said with a notice naming
    /// each project's foreman room, and costing no turn — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`. Never for one project alone, whose foreman every unowned
    /// mention is for.
    Unowned {
        /// The projects sharing the workspace, in the order they are kept.
        among: Vec<ProjectId>,
    },
}

/// Which of the two things that run an agent this is for.
///
/// The vocabulary in `docs/conventions.md` §2 has exactly two, and they differ
/// in what they are allowed to reach rather than in degree: a foreman judges
/// signals and never touches a repository, a job does the work and must.
///
/// It lives on a [`Handout`] because that is already the value which says what
/// one process may see, and because the two constructors below are the only
/// places the answer is known. Carrying it means an adapter can build the
/// narrower image for a foreman without being told separately — and being told
/// separately is what would let the image and the credentials disagree, which
/// is the failure `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`
/// exists to close.
///
/// Deliberately not serialised, for the reason a handout is not: this
/// describes a process about to be started, never anything kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The one long-lived agent a project's foreman thinks with.
    Foreman,
    /// One agent doing one piece of work, in one workspace.
    Job,
}

impl Role {
    /// Every role there is.
    ///
    /// For anything that has to consider all of them rather than the one it
    /// was handed — today, the sweep that reclaims images, which keeps the one
    /// each role would currently be built from. Written out rather than
    /// derived, like [`Agent::ALL`], and forgetting to add a role here costs
    /// an image kept that could have been reclaimed: the cheapest failure
    /// available, and the reason this is a list rather than a derive this
    /// crate would otherwise have no use for.
    pub const ALL: &'static [Self] = &[Self::Foreman, Self::Job];
}

/// Exactly what one agent process is allowed to see, and nothing more.
///
/// The piece of logic `docs/architecture.md` §1 says looks like plumbing and
/// is not. It lives in a crate with no I/O because it is a pure function from
/// configuration to a description: *deciding* is here, *delivering* belongs to
/// an adapter and differs per agent — a variable for one, a file at an
/// expected path for another.
///
/// Being able to test it without spawning a process is the whole point rather
/// than a convenience. At least one agent resolves credentials by precedence
/// and prefers a per-token key when it finds one, so a variable inherited from
/// whatever shell started the daemon silently changes who pays: no error, no
/// log line, and no way to notice before the invoice arrives. See
/// `docs/decisions/0008-one-credential-per-agent.md`, and
/// `docs/conventions.md` §3 for the rule this exists to keep.
///
/// **Nothing here is inherited.** A handout is built by selecting from one
/// project, which is how the invariant in `docs/architecture.md` §2 — a job
/// holds credentials for its own project and no other — holds by construction
/// rather than by review.
///
/// It carries credentials, so like [`Secret`] it redacts when formatted and
/// deliberately implements no serialisation: a handout is what a process is
/// about to be handed, never state, and nothing should be able to write one to
/// disk.
#[derive(Clone)]
pub struct Handout {
    kit: Kit,
    role: Role,
    // Where the job's repository lives, and nothing for a foreman. Not a
    // secret — every kickoff embeds it — but decided here beside the
    // credentials, because the checkout is made from this value and a foreman
    // must have nothing to check out; see
    // `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    repository: Option<String>,
    agent_credential: Secret,
    // Which platforms the job reaches, and never a credential for one: since
    // `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
    // a job fetches its project's credential with the warrant below when a
    // command needs one, and carries none. What is decided here is only
    // which platforms its project holds access to, which is what shapes the
    // checkout.
    platforms: BTreeSet<Platform>,
    // A job's, for that one thing; a foreman has none, having nothing to
    // fetch.
    warrant: Option<Secret>,
    variables: BTreeMap<VariableName, Variable>,
    channels: BTreeMap<Channel, Speaking>,
    place: Option<Place>,
}

impl Handout {
    /// What the agent a project's foreman thinks with is handed.
    ///
    /// Its own credential, and no platform credential at all: a foreman judges
    /// signals rather than acting on them, so it has no repository to reach and
    /// nothing to authenticate against — see
    /// `docs/decisions/0012-agents-run-in-containers.md`.
    ///
    /// It does get the project's channel bindings, and that asymmetry is the
    /// point rather than an inconsistency. Watching a project's channels is the
    /// whole of what a foreman does — `docs/architecture.md` §1 — and
    /// answering on one is a reaction it is allowed to take, so a foreman
    /// that cannot reach a channel cannot do its job. A single map for both
    /// kinds could not express this, which is the argument
    /// `docs/decisions/0027-a-channel-is-not-a-platform.md` turns on.
    ///
    /// Per project rather than per instance, because the channels it watches
    /// belong to a project and a shared foreman would hold every
    /// project's credentials at once —
    /// `docs/decisions/0020-the-orchestrator-belongs-to-a-project.md`.
    ///
    /// # Errors
    ///
    /// Fails if the project is not one this instance watches, or if its
    /// foreman's agent has no configuration — which the invariant in
    /// `docs/decisions/0021-an-instance-starts-empty.md` says cannot happen,
    /// since it holds at construction and is checked again on the way in and
    /// out of a snapshot.
    ///
    /// The signature admits it anyway, and deliberately: the alternative is a
    /// total function substituting an empty credential for a missing one, which
    /// turns a state that cannot occur into an authentication failure somewhere
    /// else entirely. `.quality/gate-reference.md` forbids exactly that trade.
    pub fn for_foreman(state: &State, project: ProjectId) -> Result<Self, HandoutError> {
        let watching = state
            .projects
            .get(&project)
            .ok_or(HandoutError::UnknownProject(project))?;
        let agent = watching.foreman_kit.agent();
        let config = state
            .agents
            .get(&agent)
            .ok_or(HandoutError::UnconfiguredAgent(agent))?;
        Ok(Self {
            kit: watching.foreman_kit.clone(),
            role: Role::Foreman,
            // No repository, for the reason there is no platform credential: a
            // foreman has no workspace, and its image carries no tool that
            // could check anything out into one — see
            // `docs/decisions/0036-a-foremans-image-is-not-a-jobs.md`.
            repository: None,
            agent_credential: config.auth_token.clone(),
            platforms: BTreeSet::new(),
            // A foreman fetches nothing, so it is given nothing to fetch
            // with.
            warrant: None,
            // None, for the reason there is no platform credential here: a
            // foreman judges signals rather than acting on them, so a
            // project's credentials for third parties are the clearest
            // possible example of something it has no business holding.
            variables: BTreeMap::new(),
            channels: speaking(state, project),
            // Narrowed by [`Handout::speaking_in`] to the thread of the
            // message being answered, every turn; nothing is fixed here.
            place: None,
        })
    }

    /// What a job's agent is handed: its own credential, the job's own
    /// warrant, and the variables and channel bindings of the one project
    /// the job belongs to.
    ///
    /// No platform credential, since
    /// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`:
    /// the warrant is what the job's container presents to this instance to
    /// fetch its project's credential when a command needs one, and it buys
    /// that and nothing else. It is taken rather than read from the record
    /// because a handout is decided before the record exists, to refuse a
    /// job that could not be handed anything before anything is written.
    ///
    /// The channels are how a job speaks without a terminal, which
    /// `docs/architecture.md` §2 makes an invariant: a job that needs a human
    /// says so on a channel and stays alive.
    ///
    /// # Errors
    ///
    /// Fails if the project is not one this instance watches, or if the agent
    /// has no configuration. Both are refusals rather than empty handouts: a
    /// process started with nothing to authenticate with fails later, further
    /// from the cause, and `docs/conventions.md` §3 would rather that be a
    /// visible job failure than a mystery.
    pub fn for_job(
        state: &State,
        kit: Kit,
        project: ProjectId,
        warrant: Secret,
    ) -> Result<Self, HandoutError> {
        let agent = kit.agent();
        let config = state
            .agents
            .get(&agent)
            .ok_or(HandoutError::UnconfiguredAgent(agent))?;
        let watched = state
            .projects
            .get(&project)
            .ok_or(HandoutError::UnknownProject(project))?;
        Ok(Self {
            kit,
            role: Role::Job,
            repository: Some(watched.repository.https()),
            agent_credential: config.auth_token.clone(),
            platforms: watched.access.keys().copied().collect(),
            warrant: Some(warrant),
            variables: watched.variables.clone(),
            channels: speaking(state, project),
            // Narrowed by [`Handout::speaking_in`] once the job has a room.
            place: None,
        })
    }

    /// Which agent this was built for.
    ///
    /// Carried so that an adapter can refuse a handout meant for another
    /// agent. The invariant says nothing belonging to any other agent, and a
    /// value that cannot be checked defends nothing.
    #[must_use]
    pub const fn agent(&self) -> Agent {
        self.kit.agent()
    }

    /// How that agent is to be set, which an adapter spells out and applies
    /// at the start of every turn — see
    /// `docs/decisions/0048-a-job-runs-on-a-kit.md`.
    #[must_use]
    pub const fn kit(&self) -> &Kit {
        &self.kit
    }

    /// Which of the two things that run an agent this was built for.
    ///
    /// Read by an adapter to decide which image to build, and by nothing else.
    /// It is derived from the constructor rather than passed in, so a handout
    /// carrying a foreman's credentials cannot be built for a job's image.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// What the agent authenticates with.
    #[must_use]
    pub const fn agent_credential(&self) -> &Secret {
        &self.agent_credential
    }

    /// Where the repository this job works on lives, or nothing for a foreman.
    ///
    /// Decided by the constructor and not by a caller: a job's handout carries
    /// its project's repository and a foreman's carries none, by construction,
    /// because a foreman has no workspace to check anything out into. An
    /// adapter reads this to make the checkout before the agent's first turn —
    /// see
    /// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    #[must_use]
    pub fn repository(&self) -> Option<&str> {
        self.repository.as_deref()
    }

    /// Whether this job reaches a platform: whether its project holds access
    /// to it, which is what its wrapper fetches a credential for.
    #[must_use]
    pub fn reaches(&self, platform: Platform) -> bool {
        self.platforms.contains(&platform)
    }

    /// Every platform this job reaches.
    pub fn platforms(&self) -> impl Iterator<Item = Platform> + '_ {
        self.platforms.iter().copied()
    }

    /// What this job's container presents to fetch its project's credential
    /// with, or nothing for a foreman.
    #[must_use]
    pub const fn warrant(&self) -> Option<&Secret> {
        self.warrant.as_ref()
    }

    /// Every variable this project gives its jobs, in the order a snapshot
    /// holds them.
    ///
    /// Ordered because the map is, which is what lets an adapter's argument
    /// list be asserted as literal text rather than as a set.
    pub fn variables(&self) -> impl Iterator<Item = (&VariableName, &Secret)> {
        self.variables
            .iter()
            .map(|(name, variable)| (name, &variable.value))
    }

    /// The names alone, for whatever must never carry a value.
    ///
    /// Separate from [`Handout::variables`] because the caller is different in
    /// kind: a prompt names them and must never carry a value, since a kickoff
    /// is stored on the job and crosses the snapshot boundary in the clear.
    /// A method returning only names is what makes that impossible to get
    /// wrong at the call site rather than merely easy to get right.
    pub fn variable_names(&self) -> impl Iterator<Item = &VariableName> {
        self.variables.keys()
    }

    /// The names with what each is for, for the instruction a job begins
    /// from: the note the operator wrote, and never the value, on the same
    /// terms as [`Handout::variable_names`] — see
    /// `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
    pub fn variables_told(&self) -> impl Iterator<Item = (&VariableName, &str)> {
        self.variables
            .iter()
            .map(|(name, variable)| (name, variable.note.as_str()))
    }

    /// How this process reaches one channel, if it is bound to one.
    #[must_use]
    pub fn channel(&self, channel: Channel) -> Option<&Speaking> {
        self.channels.get(&channel)
    }

    /// Every channel this handout can speak on.
    pub fn channels(&self) -> impl Iterator<Item = (Channel, &Speaking)> {
        self.channels.iter().map(|(c, speaking)| (*c, speaking))
    }

    /// Narrows this to one place, so the process speaks there: a job's room,
    /// or the thread a foreman is answering in.
    ///
    /// Taken separately from the rest, and the asymmetry is honest rather than
    /// awkward: everything else here is decided from configuration and can be
    /// computed before anything happens, while a room is made at the moment
    /// a job starts and cannot exist earlier. So the description is built in
    /// two steps, and this is the second.
    ///
    /// Its absence is meaningful and not a default: a handout with no place
    /// has nowhere to speak, and a process handed one is offered nothing to
    /// speak with.
    #[must_use]
    pub fn speaking_in(mut self, place: Place) -> Self {
        self.place = Some(place);
        self
    }

    /// The place this process speaks in, if it was narrowed to one.
    #[must_use]
    pub const fn place(&self) -> Option<&Place> {
        self.place.as_ref()
    }
}

impl fmt::Debug for Handout {
    /// Names what is present and never its contents.
    ///
    /// Written out rather than derived, per `docs/conventions.md` §4. The
    /// fields redact themselves, so deriving would in fact be safe here — and
    /// that is exactly the reasoning which stops being true the first time
    /// somebody adds a `String` field, which is why the rule has no exception
    /// worth taking.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handout")
            .field("kit", &self.kit)
            .field("role", &self.role)
            // A URL rather than a secret, and every kickoff embeds it already.
            .field("repository", &self.repository)
            .field("agent_credential", &"<redacted>")
            .field("platforms", &self.platforms)
            .field("warrant", &self.warrant.as_ref().map(|_| "<redacted>"))
            // Names, never values. A name is not a credential — the operator
            // typed it in order to read it back — and naming what is present
            // is the whole use of a `Debug` on this type.
            .field("variables", &self.variables.keys().collect::<Vec<_>>())
            .field("channels", &self.channels.keys().collect::<Vec<_>>())
            .field("place", &self.place)
            .finish()
    }
}

/// The speaking half of every channel a project binds.
///
/// A function rather than a `clone`, because a clone is what carried the
/// listening credential into a handout in the first place.
fn speaking(state: &State, project: ProjectId) -> BTreeMap<Channel, Speaking> {
    state
        .projects
        .get(&project)
        .into_iter()
        .flat_map(|watched| watched.channels.keys())
        .filter_map(|channel| Some((*channel, state.speaking(project, *channel)?)))
        .collect()
}

/// A handout could not be decided.
#[derive(Debug, thiserror::Error)]
pub enum HandoutError {
    /// The agent has no configuration in this instance.
    #[error("the agent {0:?} has no configuration in this instance")]
    UnconfiguredAgent(Agent),
    /// The project is not one this instance watches.
    #[error("no project {0} in this instance")]
    UnknownProject(ProjectId),
}

#[cfg(test)]
mod tests {
    use super::{
        Access, Agent, AgentConfig, Arriving, Attending, BASE64, Binding, Channel, ChannelApp,
        ChannelConfig, ClaudeEffort, ClaudeModel, Errand, Handout, HandoutError, Inbox,
        Inconsistent, Installation, Job, JobId, Key, Kit, KitConfig, KitName, KitNameError,
        NONCE_LEN, Nonce, OpenError, Outcome, Place, Platform, PlatformApp, Progress, Project,
        ProjectId, Recipient, RepositoryAddress, RepositoryError, Room, SealedAccess,
        SealedBinding, SealedChannelApp, SealedJob, SealedPlatformApp, Secret, Snapshot, State,
        Taken, Thread, Variable, VariableName, VariableNameError, Waiting, Workspace,
    };
    use base64::Engine as _;
    use jiff::Timestamp;
    use std::collections::{BTreeMap, BTreeSet};
    use uuid::Uuid;

    const TOKEN: &str = "ghp-not-a-real-token";
    /// The fixture's job's warrant: what its container presents to fetch
    /// `TOKEN` with, and never `TOKEN` itself.
    const WARRANT: &str = "warrant-of-the-fixtures-job";
    /// Another job's, on another project, so that a handout carrying the
    /// wrong one can say whose it was.
    const ALIEN_WARRANT: &str = "warrant-that-is-not-yours";

    fn warrant() -> Secret {
        Secret::new(WARRANT.to_owned())
    }
    /// Distinct from `TOKEN`, so a test that finds a credential where it should
    /// not can say which map it escaped from.
    const CHANNEL_TOKEN: &str = "xoxb-not-a-real-token";
    /// Not a secret, and asserted to travel in the clear.
    /// A room people talk in, which no job owns.
    const CHANNEL_ADDRESS: &str = "C0123456789";
    /// The room the fixture's job owns.
    const JOB_ROOM: &str = "C0JOBROOM01";
    /// A second project's channel credential, so that a handout carrying the
    /// wrong one is detectable rather than merely non-empty.
    const ALIEN_CHANNEL_TOKEN: &str = "xoxb-belongs-to-somebody-else";
    /// The second credential, which opens an event stream rather than posting.
    /// Distinct again, so a handout carrying it is detectable.
    const LISTEN_TOKEN: &str = "xapp-not-a-real-token";

    /// One of a project's own variables, and the value in it.
    ///
    /// A third-party credential rather than something innocuous, because that
    /// is what the concept is for and because the escape test below has to be
    /// able to tell one project's from another's.
    const VARIABLE: &str = "STRIPE_API_KEY";
    const VARIABLE_VALUE: &str = "sk-test-not-a-real-key";

    /// The same variable on the other project, holding somebody else's value.
    const ALIEN_VARIABLE_VALUE: &str = "sk-test-belongs-to-somebody-else";
    /// An instance with an agent configured and nothing else.
    fn configured() -> State {
        State {
            apps: std::collections::BTreeMap::new(),
            channel_apps: std::collections::BTreeMap::new(),
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("agent-token".to_owned()),
                },
            )]),
            ..State::default()
        }
    }

    /// The kits a project's jobs may run on, for a project offering one: its
    /// agent's defaults, under the agent's name — which is exactly what a
    /// project written before kits existed opens as.
    fn default_kits() -> BTreeMap<KitName, KitConfig> {
        BTreeMap::from([(
            KitName::new("Claude").expect("a name"),
            KitConfig::defaults(Agent::Claude),
        )])
    }

    fn a_project_with_a_job() -> Project {
        let mut access = BTreeMap::new();
        access.insert(
            Platform::GitHub,
            Access::Token {
                secret: Secret::new(TOKEN.to_owned()),
                owner: Some("example".to_owned()),
                expires: Some(Timestamp::from_second(4_102_444_800).expect("a time")),
            },
        );
        let channels = BTreeMap::from([(Channel::Slack, a_slack_binding())]);
        let mut jobs = BTreeMap::new();
        jobs.insert(
            JobId::from_uuid(Uuid::from_u128(9)),
            Job::new(
                Kit::defaults(Agent::Claude),
                "an issue was opened".to_owned(),
                "work on it".to_owned(),
                Timestamp::UNIX_EPOCH,
                Secret::new(WARRANT.to_owned()),
            ),
        );
        Project {
            name: "example".to_owned(),
            repository: RepositoryAddress::new("example", "repo").expect("an address"),
            foreman_kit: Kit::defaults(Agent::Claude),
            kits: default_kits(),
            access,
            channels,
            variables: BTreeMap::from([(
                VariableName::new(VARIABLE).expect("a deliverable name"),
                Variable {
                    value: Secret::new(VARIABLE_VALUE.to_owned()),
                    note: "the staging database, read-only".to_owned(),
                },
            )]),
            jobs,
            attending: Attending::default(),
            brief: String::new(),
            watched: BTreeSet::new(),
            foreman_room: None,
        }
    }

    fn a_slack_binding() -> Binding {
        Binding::Own(ChannelConfig {
            credential: Secret::new(CHANNEL_TOKEN.to_owned()),
            listen_credential: Secret::new(LISTEN_TOKEN.to_owned()),
        })
    }

    /// The room a job of the fixture speaks in.
    fn a_room() -> Room {
        Room {
            channel: Channel::Slack,
            id: JOB_ROOM.to_owned(),
        }
    }

    #[test]
    fn debug_does_not_leak_a_secret() {
        let secret = Secret::new(TOKEN.to_owned());
        assert!(!format!("{secret:?}").contains(TOKEN));
    }

    #[test]
    fn display_does_not_leak_a_secret() {
        let secret = Secret::new(TOKEN.to_owned());
        assert!(!format!("{secret}").contains(TOKEN));
    }

    #[test]
    fn a_secret_nested_in_a_structure_does_not_leak() {
        // The failure this guards is not formatting a secret directly — nobody
        // does that. It is printing whatever happens to contain one.
        //
        // A project now nests two kinds, and the second is the one a derive
        // could get wrong: `ChannelConfig` holds a `String` beside its
        // credential, so this is what says the derive on it is still safe.
        let shown = format!("{:?}", a_project_with_a_job());
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains(CHANNEL_TOKEN), "{shown}");
    }

    #[test]
    fn a_secret_still_yields_its_value_when_asked() {
        assert_eq!(Secret::new(TOKEN.to_owned()).expose(), TOKEN);
    }

    /// The state an instance is in before anybody has configured anything,
    /// which `docs/decisions/0021-an-instance-starts-empty.md` made valid
    /// again.
    #[test]
    fn a_fresh_instance_has_nothing_and_is_still_a_state() {
        let empty = State::default();

        assert!(empty.agents.is_empty());
        assert!(empty.projects.is_empty());
    }

    #[test]
    fn a_project_names_the_agent_its_foreman_thinks_with() {
        let state = populated();
        let project = state.projects.values().next().expect("one project");

        assert!(state.agents.contains_key(&project.foreman_kit.agent()));
        assert!(
            project
                .kits
                .values()
                .any(|offered| offered.kit.agent() == Agent::Claude)
        );
    }

    /// The rule a dashboard has to enforce, and the query it enforces it with.
    #[test]
    fn an_agent_a_project_names_reports_which_projects_would_break() {
        let state = populated();
        let depending: Vec<ProjectId> = state.used_by(Agent::Claude).collect();

        assert_eq!(depending.len(), 1, "{depending:?}");
        assert_eq!(configured().used_by(Agent::Claude).count(), 0);
    }

    /// One definition of valid, asked directly. Everything that persists or
    /// loads a state consults this rather than repeating the rule.
    #[test]
    fn a_state_that_lost_an_agent_a_project_names_is_not_consistent() {
        let mut state = populated();
        state.agents.clear();

        assert!(matches!(
            state.check(),
            Err(Inconsistent::UnconfiguredProjectAgent { .. })
        ));
    }

    #[test]
    fn a_project_with_no_kit_for_its_jobs_is_not_consistent() {
        let mut state = populated();
        for project in state.projects.values_mut() {
            project.kits.clear();
        }

        assert!(matches!(state.check(), Err(Inconsistent::NoKits(_))));
    }

    /// A kit's name is what was typed, less the space around it, or nothing.
    #[test]
    fn a_kits_name_is_trimmed_and_never_empty() {
        assert_eq!(KitName::new("  deep ").expect("a name").as_str(), "deep");
        assert_eq!(KitName::new("   "), Err(KitNameError::Empty));
        assert_eq!(KitName::new(""), Err(KitNameError::Empty));
    }

    /// A kit an operator wrote survives the snapshot boundary as itself: its
    /// name, its description and every setting.
    #[test]
    fn a_named_kit_survives_the_snapshot_boundary() {
        let mut state = populated();
        let deep = KitConfig {
            description: "for refactors touching many files; costs several times more".to_owned(),
            kit: Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            },
        };
        for project in state.projects.values_mut() {
            project
                .kits
                .insert(KitName::new("deep").expect("a name"), deep.clone());
            project.foreman_kit = Kit::Claude {
                model: ClaudeModel::Haiku,
            };
        }

        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        let project = reopened.projects.values().next().expect("the project");

        assert_eq!(
            project.kits.get(&KitName::new("deep").expect("a name")),
            Some(&deep)
        );
        assert_eq!(project.kits.len(), 2, "the default kit is still there too");
        assert_eq!(
            project.foreman_kit,
            Kit::Claude {
                model: ClaudeModel::Haiku
            }
        );
    }

    /// A name that is not one is refused as the file is opened.
    #[test]
    fn a_kit_under_a_blank_name_is_refused_as_the_file_is_opened() {
        let mut sealed = sealed();
        for project in sealed.projects.values_mut() {
            project
                .kits
                .insert("   ".to_owned(), KitConfig::defaults(Agent::Claude));
        }

        assert!(matches!(sealed.open(&key()), Err(OpenError::KitName(_))));
    }

    /// A project with nowhere to escalate is a working project.
    ///
    /// `docs/decisions/0005-conversation-happens-on-channels.md` says it can
    /// still run work that never needs to ask, so requiring a binding would
    /// make the escalation path a prerequisite for work that never escalates.
    /// Asserted because the natural next change to [`State::check`] is to
    /// demand one.
    #[test]
    fn a_project_with_no_channel_bound_is_still_consistent() {
        let mut state = populated();
        for project in state.projects.values_mut() {
            project.channels.clear();
        }

        assert_eq!(state.check(), Ok(()));
    }

    #[test]
    fn an_instance_with_nothing_in_it_is_consistent() {
        assert_eq!(State::default().check(), Ok(()));
        assert_eq!(populated().check(), Ok(()));
    }

    #[test]
    fn a_jobs_agent_survives_that_agent_being_deconfigured() {
        // The reason a job stores an agent by value rather than by reference:
        // removing configuration is ordinary housekeeping, and it must not
        // rewrite the record of work already done.
        let mut state = configured();
        let project = a_project_with_a_job();
        state
            .projects
            .insert(ProjectId::from_uuid(Uuid::from_u128(3)), project);
        state.agents.remove(&Agent::Claude);

        let still_recorded = state
            .projects
            .values()
            .flat_map(|project| project.jobs.values())
            .all(|job| job.kit().agent() == Agent::Claude);
        assert!(still_recorded);
    }

    /// An instance with one project and one job in a room of its own — the
    /// smallest state a message can be routed against.
    fn listening() -> (State, ProjectId, JobId) {
        let mut state = configured();
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        state.projects.insert(project, a_project_with_a_job());
        let job = state
            .projects
            .get(&project)
            .expect("just inserted")
            .jobs
            .keys()
            .next()
            .expect("a job")
            .clone();
        state.job_mut(&job).expect("the job").room = Some(a_room());
        (state, project, job)
    }

    /// A person's message, as much of it as routing sees.
    fn arriving<'a>(room: &'a str, thread: Option<&'a str>) -> Arriving<'a> {
        Arriving {
            room,
            id: "1788000000.000000",
            thread,
            from_us: false,
            from_app: false,
        }
    }

    /// Another app's message, as much of it as routing sees.
    fn posted_by_an_app<'a>(room: &'a str, thread: Option<&'a str>) -> Arriving<'a> {
        Arriving {
            from_app: true,
            ..arriving(room, thread)
        }
    }

    /// Among several projects on one workspace the room decides: a job's
    /// room is that job's, a foreman's room and a watched room are that
    /// project's foreman's, an app's message is a watching project's or
    /// nobody's, and a person's mention in a room none of them owns is
    /// nobody's to answer but the notice; with one project it is that
    /// foreman's, and with none it is nobody's.
    #[test]
    fn among_several_projects_the_room_decides_and_an_unowned_mention_is_pointed() {
        let mut state = populated();
        let mine = *state.projects.keys().next().expect("a project");
        let other = ProjectId::from_uuid(Uuid::from_u128(77));
        let mut theirs = state.projects.get(&mine).expect("the project").clone();
        theirs.jobs.clear();
        theirs.foreman_room = Some(Room {
            channel: Channel::Slack,
            id: "C0THEIRFOREMAN".to_owned(),
        });
        theirs.watched.insert(Room {
            channel: Channel::Slack,
            id: "C0THEIRWATCH".to_owned(),
        });
        state.projects.insert(other, theirs);
        let job = state
            .projects
            .get(&mine)
            .expect("the project")
            .jobs
            .keys()
            .next()
            .expect("a job")
            .clone();
        state.job_mut(&job).expect("the job").room = Some(Room {
            channel: Channel::Slack,
            id: "C0MYJOB".to_owned(),
        });
        let arriving = |room: &'static str, from_app: bool| Arriving {
            room,
            id: "1788000000.000100",
            thread: None,
            from_us: false,
            from_app,
        };
        let both = [mine, other];
        assert_eq!(
            state.recipient_among(&both, Channel::Slack, &arriving("C0MYJOB", false)),
            Recipient::Job(job)
        );
        assert_eq!(
            state.recipient_among(&both, Channel::Slack, &arriving("C0THEIRFOREMAN", false)),
            Recipient::Foreman(other)
        );
        assert_eq!(
            state.recipient_among(&both, Channel::Slack, &arriving("C0THEIRWATCH", false)),
            Recipient::Foreman(other)
        );
        assert_eq!(
            state.recipient_among(&both, Channel::Slack, &arriving("C0THEIRWATCH", true)),
            Recipient::Foreman(other)
        );
        assert_eq!(
            state.recipient_among(&both, Channel::Slack, &arriving("C0NOBODYS", true)),
            Recipient::Nobody
        );
        assert_eq!(
            state.recipient_among(&both, Channel::Slack, &arriving("C0NOBODYS", false)),
            Recipient::Unowned {
                among: vec![mine, other]
            }
        );
        assert_eq!(
            state.recipient_among(&[mine], Channel::Slack, &arriving("C0NOBODYS", false)),
            Recipient::Foreman(mine)
        );
        assert_eq!(
            state.recipient_among(&[], Channel::Slack, &arriving("C0NOBODYS", false)),
            Recipient::Nobody
        );
        assert_eq!(
            state.recipient_among(
                &both,
                Channel::Slack,
                &Arriving {
                    room: "C0NOBODYS",
                    id: "1788000000.000100",
                    thread: None,
                    from_us: true,
                    from_app: false,
                }
            ),
            Recipient::Nobody,
            "what this instance said itself is nobody's, among any number"
        );
    }

    /// Another app's message is the foreman's in a room the project watches,
    /// at the root and in a thread — a follow-up under an earlier message —
    /// and nobody's in a room it does not, even the room people talk in.
    #[test]
    fn another_apps_message_is_the_foremans_exactly_in_a_watched_room() {
        let (mut state, project, _) = listening();
        for thread in [None, Some("1728312345.678901")] {
            assert_eq!(
                state.recipient(
                    project,
                    Channel::Slack,
                    &posted_by_an_app(CHANNEL_ADDRESS, thread)
                ),
                Recipient::Nobody,
                "not watched, so not read: {thread:?}"
            );
        }

        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .watched
            .insert(Room {
                channel: Channel::Slack,
                id: CHANNEL_ADDRESS.to_owned(),
            });
        for thread in [None, Some("1728312345.678901")] {
            assert_eq!(
                state.recipient(
                    project,
                    Channel::Slack,
                    &posted_by_an_app(CHANNEL_ADDRESS, thread)
                ),
                Recipient::Foreman(project),
                "watched, so the foreman's: {thread:?}"
            );
        }
        assert_eq!(
            state.recipient(project, Channel::Slack, &posted_by_an_app(JOB_ROOM, None)),
            Recipient::Nobody,
            "a job's room is not watched, and an app's message there is not a reply"
        );
        assert_eq!(
            state.recipient(project, Channel::Slack, &arriving(CHANNEL_ADDRESS, None)),
            Recipient::Foreman(project),
            "a person's mention in a watched room is read as anywhere else"
        );
    }

    /// A mention in a job's room is that job's, at the root and in a thread.
    #[test]
    fn a_mention_anywhere_in_a_jobs_room_is_for_that_job() {
        let (state, project, job) = listening();

        for thread in [None, Some("1728312345.678901")] {
            assert_eq!(
                state.recipient(project, Channel::Slack, &arriving(JOB_ROOM, thread)),
                Recipient::Job(job.clone()),
                "{thread:?}"
            );
        }
    }

    /// A mention in a room that is no job's is the foreman's, at the root
    /// and in a thread alike.
    ///
    /// The thread case is where a person lands by replying to something a
    /// foreman said, which is the most natural move available.
    /// `docs/decisions/0031-a-mention-is-what-makes-it-ours.md` answered it
    /// with a fixed sentence, and
    /// `docs/decisions/0060-a-binding-is-a-workspace.md` makes it work: the
    /// errand carries its thread, so the answer lands where the person is.
    #[test]
    fn a_mention_in_a_room_that_is_no_jobs_is_for_the_foreman() {
        let (state, project, _) = listening();

        for thread in [None, Some("1111111111.000000")] {
            assert_eq!(
                state.recipient(project, Channel::Slack, &arriving(CHANNEL_ADDRESS, thread)),
                Recipient::Foreman(project),
                "{thread:?}"
            );
        }
    }

    /// Nothing this instance said is ever routed anywhere.
    ///
    /// The loop guard, asserted at the point it is decided. Every other rule
    /// would otherwise match: this is a message in a job's room, which is the
    /// strongest match there is.
    #[test]
    fn nothing_this_instance_said_is_routed_back_to_it() {
        let (state, project, _) = listening();

        for thread in [Some("1728312345.678901"), None] {
            assert_eq!(
                state.recipient(
                    project,
                    Channel::Slack,
                    &Arriving {
                        room: JOB_ROOM,
                        id: "1788000000.000000",
                        thread,
                        from_us: true,
                        from_app: false,
                    }
                ),
                Recipient::Nobody,
                "an instance answering itself is a loop that bills per lap"
            );
        }
    }

    /// A message for a project this instance does not watch is nobody's.
    ///
    /// Not a case a socket can produce, since every socket is a project's;
    /// asserted so that the answer to an impossible question is silence
    /// rather than a foreman that does not exist.
    #[test]
    fn a_message_for_a_project_this_instance_does_not_watch_is_nobodys() {
        let (state, _, _) = listening();

        assert_eq!(
            state.recipient(
                ProjectId::from_uuid(Uuid::from_u128(404)),
                Channel::Slack,
                &arriving(CHANNEL_ADDRESS, None)
            ),
            Recipient::Nobody
        );
    }

    /// Only the socket's own project's jobs are looked at.
    ///
    /// Two projects' apps can share a room, and each hears its own copy of a
    /// mention there. Another project's job's room is, for this one, a room
    /// belonging to no job — and so its foreman's, not silence and not the
    /// other project's job.
    #[test]
    fn another_projects_jobs_room_is_this_projects_foremans() {
        let (mut state, _, _) = listening();
        let other = ProjectId::from_uuid(Uuid::from_u128(4));
        state.projects.insert(other, a_project_with_a_job());

        assert_eq!(
            state.recipient(other, Channel::Slack, &arriving(JOB_ROOM, None)),
            Recipient::Foreman(other)
        );
    }

    /// An idle job's room still routes to it.
    ///
    /// Deliberate: the room is that job's conversation, and somebody
    /// speaking in it a day later means that job. Whether it can still take
    /// the message is the deliverer's problem, not this one's.
    #[test]
    fn an_idle_jobs_room_still_routes_to_it() {
        let (mut state, project, job) = listening();
        state.job_mut(&job).expect("the job").progress = Progress::Idle(Waiting::Silent);

        assert_eq!(
            state.recipient(project, Channel::Slack, &arriving(JOB_ROOM, None)),
            Recipient::Job(job)
        );
    }

    fn key() -> Key {
        Key::new([7; 32])
    }

    /// Deterministic stand-in for the randomness a caller normally supplies.
    ///
    /// A range rather than a counter, so the arithmetic happens inside the
    /// iterator instead of in code that would then need a suppression. Real
    /// nonces come from the operating system; all a test needs is that
    /// successive ones differ.
    fn counting_nonces() -> impl FnMut() -> Nonce {
        let mut supply = (0_u8..u8::MAX).map(|byte| [byte; NONCE_LEN]);
        move || supply.next().unwrap_or([u8::MAX; NONCE_LEN])
    }

    fn populated() -> State {
        let mut state = configured();
        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(3)),
            a_project_with_a_job(),
        );
        state
    }

    fn sealed() -> Snapshot {
        populated()
            .seal(&key(), &mut counting_nonces())
            .expect("sealing cannot fail for a well-formed key and nonce")
    }

    #[test]
    fn a_sealed_snapshot_carries_no_plaintext_credential() {
        // The whole point of the exercise. If this ever fails, every token this
        // instance holds is sitting in a file in the clear.
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");
        assert!(!json.contains(TOKEN));
        assert!(!json.contains("agent-token"));
        assert!(!json.contains(CHANNEL_TOKEN), "{json}");
        assert!(!json.contains(LISTEN_TOKEN), "{json}");
        // A variable's value is sealed like any other credential; its name is
        // not, and must not be — an operator reads a name back in order to
        // know what a project is carrying.
        assert!(!json.contains(VARIABLE_VALUE), "{json}");
        assert!(
            json.contains(VARIABLE),
            "a variable's name travels in the clear: {json}"
        );
    }

    /// A snapshot says nothing about the machine it was written on.
    ///
    /// This replaces a test asserting the opposite — that a snapshot remembered
    /// where the container runtime lived. That field is gone, per
    /// `docs/decisions/0023-the-container-runtime-is-discovered-once.md`, and
    /// what it protected is worth keeping as a property: the file is meant to
    /// be copied to another machine, so anything machine-specific in it is
    /// wrong there. Asserted against the serialised form rather than the type,
    /// because a field added back would compile perfectly and only show up
    /// here.
    #[test]
    fn a_snapshot_holds_nothing_that_belongs_to_one_machine() {
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");

        assert!(!json.contains("runtime"), "{json}");
        assert!(!json.contains("/usr/"), "{json}");
    }

    #[test]
    fn a_snapshot_round_trips_through_json_and_back_into_state() {
        // Also proves the map keys survive: JSON object keys must be strings,
        // so an enum or an identifier used as one has to serialise as text.
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");
        let parsed: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let state = parsed.open(&key()).expect("and opens with the right key");
        let project = state
            .projects
            .values()
            .next()
            .expect("the project survived");
        assert!(matches!(
            project.access.get(&Platform::GitHub),
            Some(Access::Token { secret, .. }) if secret.expose() == TOKEN
        ));
        let bound = project
            .channels
            .get(&Channel::Slack)
            .and_then(Binding::own)
            .expect("the binding survived, as an app of its own");
        assert_eq!(bound.credential.expose(), CHANNEL_TOKEN);
        assert_eq!(
            bound.listen_credential.expose(),
            LISTEN_TOKEN,
            "both credentials cross the boundary, not only the one that posts"
        );
    }

    #[test]
    fn each_credential_is_sealed_with_its_own_nonce() {
        // Guards the failure that would be catastrophic rather than merely
        // wrong: with this cipher, reusing a nonce under one key leaks the
        // authentication key. Hoisting one nonce out of the loop would look
        // like a tidy-up.
        let snapshot = sealed();
        let agent_nonce = &snapshot
            .agents
            .get(&Agent::Claude)
            .expect("the agent is configured")
            .auth_token
            .nonce;
        let project = snapshot
            .projects
            .values()
            .next()
            .expect("the project is there");
        let Some(SealedAccess::Token {
            secret: sealed_token,
            ..
        }) = project.credentials.get(&Platform::GitHub)
        else {
            panic!("the credential is there, as a token");
        };
        let project_nonce = &sealed_token.nonce;
        let Some(SealedBinding::Own(sealed_binding)) = project.channels.get(&Channel::Slack) else {
            panic!("the binding is there, as an app of its own");
        };
        let channel_nonce = &sealed_binding.credential.nonce;
        assert_ne!(agent_nonce, project_nonce);
        assert_ne!(project_nonce, channel_nonce);
        assert_ne!(agent_nonce, channel_nonce);
    }

    /// A job records where its conversation happens, across the snapshot.
    #[test]
    fn a_jobs_thread_survives_the_snapshot_boundary() {
        let mut state = populated();
        let job = state
            .projects
            .values()
            .next()
            .expect("a project")
            .jobs
            .keys()
            .next()
            .expect("a job")
            .clone();
        state.job_mut(&job).expect("the job").room = Some(a_room());

        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");

        let room = reopened
            .job(&job)
            .expect("the job survived")
            .room
            .clone()
            .expect("and so did its room");
        assert_eq!(room.channel, Channel::Slack);
        assert_eq!(room.id, JOB_ROOM);
    }

    /// A job as the last release wrote it still opens, in every state it
    /// could have been in.
    ///
    /// The literal-older-file test `docs/conventions.md` §4 demands, and the
    /// window it covers is one release rather than every release there has
    /// been: what is supported is what the latest tag wrote, and everything
    /// older is dropped in the same change that would have had to carry it.
    /// The writer always emits the current spelling, so only a file from
    /// before the change can produce the input that breaks.
    ///
    /// The two states that moved arrive as the reading that is true of them.
    /// A bare `Idle` becomes `Silent` rather than a guess: that format had
    /// nowhere to record why an agent stopped, so no claim is invented for it.
    #[test]
    fn a_job_written_by_the_last_release_still_opens_in_every_state() {
        for (written, expected) in [
            ("\"Working\"", Progress::Working),
            ("\"Idle\"", Progress::Idle(Waiting::Silent)),
            (
                r#"{"Failed": "the credential had expired"}"#,
                Progress::Idle(Waiting::Failed("the credential had expired".to_owned())),
            ),
        ] {
            let older = format!(
                r#"{{
                  "kit": {{ "Claude": {{ "model": {{ "Default": {{ "effort": "Default" }} }} }} }},
                  "reason": "an issue was opened",
                  "kickoff": "work on it",
                  "created_at": "1970-01-01T00:00:00Z",
                  "progress": {written},
                  "thread": null,
                  "reported": {{}}
                }}"#
            );

            let job = serde_json::from_str::<SealedJob>(&older)
                .unwrap_or_else(|why| panic!("{written} must still parse: {why}"))
                .open(&key())
                .expect("and opens, holding no warrant to unseal");
            assert_eq!(job.progress, expected);
            assert!(
                job.warrant().is_none(),
                "a job written before warrants existed has none"
            );
            assert!(
                job.pull_requests.is_empty(),
                "a job written before it could claim one claimed none"
            );
            assert_eq!(
                job.since, None,
                "a job written before the moment was kept says only that it waits"
            );
        }
    }

    /// What is written back is the current spelling, so a file upgrades itself
    /// the first time anything changes rather than carrying both for ever.
    #[test]
    fn a_state_is_written_in_the_shape_this_build_reads() {
        let written = serde_json::to_value(Progress::Idle(Waiting::Asked)).expect("it serialises");
        assert_eq!(written, serde_json::json!({ "Idle": "Asked" }));

        let over = serde_json::to_value(Progress::Retired(Outcome::Lost)).expect("it serialises");
        assert_eq!(over, serde_json::json!({ "Retired": "Lost" }));

        assert_eq!(
            serde_json::to_value(Progress::Working).expect("it serialises"),
            serde_json::json!("Working"),
        );
    }

    /// Every reading round-trips as itself, including the one carrying prose.
    #[test]
    fn every_reading_survives_being_written_and_read_back() {
        for progress in [
            Progress::Working,
            Progress::Idle(Waiting::Asked),
            Progress::Idle(Waiting::Proposed),
            Progress::Idle(Waiting::Paused),
            Progress::Idle(Waiting::Silent),
            Progress::Idle(Waiting::Failed("it ran out of tokens".to_owned())),
            Progress::Retired(Outcome::Done),
            Progress::Retired(Outcome::Discarded),
            Progress::Retired(Outcome::Lost),
        ] {
            let job = Job {
                progress: progress.clone(),
                ..Job::new(
                    Kit::defaults(Agent::Claude),
                    "a reason".to_owned(),
                    "some work".to_owned(),
                    Timestamp::UNIX_EPOCH,
                    warrant(),
                )
            };
            let written = serde_json::to_string(
                &job.seal(&key(), &mut counting_nonces())
                    .expect("sealing cannot fail"),
            )
            .expect("a job serialises");
            let read = serde_json::from_str::<SealedJob>(&written)
                .expect("and parses back")
                .open(&key())
                .expect("and opens");
            assert_eq!(read.progress, progress);
        }
    }

    /// Only a retired job is over, and every reading of idle is not.
    ///
    /// The question every behavioural caller asks, so inverting it would let a
    /// reply reach a job whose container is gone and stop one reaching a job
    /// that is merely waiting.
    #[test]
    fn a_job_is_over_only_once_it_is_retired() {
        assert!(Progress::Retired(Outcome::Done).is_retired());
        assert!(Progress::Retired(Outcome::Lost).is_retired());
        assert!(!Progress::Working.is_retired());
        assert!(!Progress::Idle(Waiting::Silent).is_retired());
        assert!(
            !Progress::Idle(Waiting::Failed("broken".to_owned())).is_retired(),
            "a failed job still has its container and can be given another go",
        );
    }

    /// A kit that says more than the defaults survives the snapshot boundary
    /// as itself, and so does what the agent reported back.
    #[test]
    fn a_jobs_kit_and_what_was_reported_survive_the_snapshot_boundary() {
        let mut state = populated();
        let project = *state.projects.keys().next().expect("a project");
        let job = JobId::from_uuid(Uuid::from_u128(11));
        let mut recorded = Job::new(
            Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            },
            "a refactor touching many files".to_owned(),
            "do it carefully".to_owned(),
            Timestamp::UNIX_EPOCH,
            warrant(),
        );
        recorded
            .reported
            .insert("model".to_owned(), "opus[1m]".to_owned());
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .jobs
            .insert(job.clone(), recorded);

        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        let survived = reopened.job(&job).expect("the job survived");

        assert_eq!(
            *survived.kit(),
            Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            }
        );
        assert_eq!(
            survived.reported.get("model").map(String::as_str),
            Some("opus[1m]"),
            "what the adapter said is kept in its own spelling",
        );
    }

    /// The model that has no effort says so, and the ones that do say which.
    ///
    /// Small, and worth having because the adapter's spelling is built from
    /// this answer: a model reporting an effort it does not have would send
    /// an option the adapter refuses as unknown.
    #[test]
    fn only_the_models_with_an_effort_report_one() {
        assert_eq!(ClaudeModel::Haiku.effort(), None);
        for effort in ClaudeEffort::ALL {
            assert_eq!(
                ClaudeModel::Default { effort: *effort }.effort(),
                Some(*effort)
            );
            assert_eq!(
                ClaudeModel::Sonnet { effort: *effort }.effort(),
                Some(*effort)
            );
            assert_eq!(
                ClaudeModel::Opus { effort: *effort }.effort(),
                Some(*effort)
            );
        }
        assert_eq!(Kit::defaults(Agent::Claude).agent(), Agent::Claude);
    }

    /// A snapshot as the last release wrote it still opens, whole.
    ///
    /// The literal-older-file test `docs/conventions.md` §4 asks for, and the
    /// only one: what this build promises to read is what the latest tag
    /// wrote, and everything older was dropped in the change that would have
    /// had to keep carrying it. Two tests used to sit here, pinning files from
    /// before threads and before channels, and both described releases this
    /// one no longer reads.
    ///
    /// Written as literal text rather than by round-tripping, deliberately.
    /// The current writer always emits every field, so a round trip cannot
    /// produce the input that breaks — only a file from another release can,
    /// and this is one.
    ///
    /// **Nothing in here may be renamed to match the source.** These are the
    /// names the last release wrote, not the names the types happen to use
    /// now, and a search-and-replace across the crate will silently update
    /// them and leave this passing against a file nothing ever produced.
    ///
    /// A file as the last release wrote it, holding two projects.
    ///
    /// Two, because the last release wrote two shapes of binding and two
    /// shapes of thread, and what survives differs per `docs/conventions.md`
    /// §4. The first project speaks and does not listen, holds a job in a
    /// thread that names no room, and has a message in hand. The second
    /// listens.
    ///
    /// **Nothing in here may be renamed to match the source.** These are the
    /// names the last release wrote, not the names the types happen to use
    /// now, and a search-and-replace across the crate will silently update
    /// them and leave this passing against a file nothing ever produced.
    /// Note what is absent: the field naming the agents a project's jobs
    /// could run on. The last release read one and never wrote one, so a
    /// file it wrote has kits and nothing else.
    fn written_by_the_last_release() -> String {
        let sealed = serde_json::to_string(
            &Secret::new("agent-token".to_owned())
                .seal(&key(), [1; NONCE_LEN])
                .expect("sealing a well-formed secret"),
        )
        .expect("a sealed secret serialises");
        format!(
            r#"{{
              "agents": {{
                "Claude": {{ "auth_token": {sealed} }}
              }},
              "projects": {{
                "00000000-0000-0000-0000-000000000003": {{
                  "name": "example",
                  "repository": "https://github.com/example/repo",
                  "foreman_kit": {{ "Claude": {{ "model": {{ "Default": {{ "effort": "Default" }} }} }} }},
                  "kits": {{
                    "Claude": {{
                      "kit": {{ "Claude": {{ "model": {{ "Default": {{ "effort": "Default" }} }} }} }},
                      "description": "its defaults"
                    }}
                  }},
                  "credentials": {{ "GitHub": {sealed} }},
                  "channels": {{
                    "Slack": {{ "address": "C0123456789", "credential": {sealed} }}
                  }},
                  "variables": {{ "STRIPE_API_KEY": {sealed} }},
                  "jobs": {{
                    "00000000-0000-0000-0000-000000000009": {{
                      "kit": {{ "Claude": {{ "model": {{ "Default": {{ "effort": "Default" }} }} }} }},
                      "reason": "an issue was opened",
                      "kickoff": "work on it",
                      "created_at": "1970-01-01T00:00:00Z",
                      "progress": "Idle",
                      "thread": {{ "channel": "Slack", "id": "1728312345.678901" }},
                      "reported": {{}}
                    }}
                  }},
                  "attending": {{
                    "Working": {{
                      "on": {{
                        "said": "look at the parser",
                        "thread": {{ "channel": "Slack", "id": "1728312345.678901" }}
                      }},
                      "waiting": []
                    }}
                  }}
                }},
                "00000000-0000-0000-0000-000000000004": {{
                  "name": "listening",
                  "repository": "https://github.com/example/other",
                  "foreman_kit": {{ "Claude": {{ "model": {{ "Default": {{ "effort": "Default" }} }} }} }},
                  "kits": {{
                    "Claude": {{
                      "kit": {{ "Claude": {{ "model": {{ "Default": {{ "effort": "Default" }} }} }} }},
                      "description": "its defaults"
                    }}
                  }},
                  "credentials": {{}},
                  "channels": {{
                    "Slack": {{
                      "address": "C0123456789",
                      "credential": {sealed},
                      "listen_credential": {sealed}
                    }}
                  }},
                  "variables": {{}},
                  "jobs": {{}},
                  "attending": "Idle"
                }}
              }}
            }}"#
        )
    }

    /// A snapshot as the last release wrote it still opens, and what cannot
    /// be carried is dropped rather than refused: a binding without the
    /// credential that listens, a thread that names no room, and an inbox
    /// whose threads name none. The project and its job are kept.
    #[test]
    fn a_snapshot_written_by_the_last_release_still_opens() {
        // The last release's form did not check the text, so a file may
        // hold something that is not an address: refused with the project
        // named, rather than opened as something nothing can compose from —
        // see `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
        let not_an_address = written_by_the_last_release().replace(
            "https://github.com/example/repo",
            "https://example.invalid/repo",
        );
        let parsed: Snapshot = serde_json::from_str(&not_an_address).expect("still parses");
        let refused = parsed.open(&key()).expect_err("refused on opening");
        assert_eq!(
            refused.to_string(),
            "the project example holds a repository that is not an address on GitHub: it has to \
             be on github.com"
        );

        let parsed: Snapshot = serde_json::from_str(&written_by_the_last_release())
            .expect("an older file still parses");
        let state = parsed.open(&key()).expect("and still opens");
        let project = state
            .projects
            .get(&ProjectId::from_uuid(Uuid::from_u128(3)))
            .expect("the project survived");

        assert_eq!(project.name, "example");
        assert_eq!(
            project.repository,
            RepositoryAddress::new("example", "repo").expect("an address"),
            "the text the last release wrote opens as the address it spelled"
        );
        assert_eq!(
            project
                .kits
                .keys()
                .map(KitName::to_string)
                .collect::<Vec<_>>(),
            vec!["Claude".to_owned()],
        );
        // The last release wrote a variable as its sealed value alone, which
        // opens as one with nothing said about it — the bridge
        // `docs/decisions/0075-a-variable-says-what-it-is-for.md` keeps.
        let variable = project
            .variables
            .get(&VariableName::new("STRIPE_API_KEY").expect("a deliverable name"))
            .expect("the variable survived");
        assert_eq!(variable.value.expose(), "agent-token");
        assert_eq!(variable.note, "", "a bare value has no note");
        assert!(
            project.channels.is_empty(),
            "a binding without the credential that listens is no binding"
        );
        assert_eq!(
            project.attending,
            Attending::Idle,
            "an inbox whose threads name no room opens idle"
        );

        let job = project.jobs.values().next().expect("and its job");
        assert_eq!(job.reason, "an issue was opened");
        assert_eq!(
            job.progress,
            Progress::Idle(Waiting::Silent),
            "a state that could not say why becomes the reading that says so",
        );
        assert_eq!(
            job.room, None,
            "a thread, which is what the last release wrote, is not a room"
        );
        assert_eq!(job.asked_by, None);
        assert!(job.inbox.is_empty(), "no job had an inbox to write");
        assert!(job.warrant().is_none(), "no job had a warrant to write");
        assert_eq!(project.brief, "", "nobody had written one");
        assert!(project.watched.is_empty(), "nothing was watched");
        // The last release wrote a token as a bare sealed secret, which is
        // the one shape of access it knew and still the shape a token is
        // written in.
        assert!(matches!(
            project.access.get(&Platform::GitHub),
            Some(Access::Token {
                secret,
                owner: None,
                expires: None
            }) if secret.expose() == "agent-token"
        ));
    }

    /// An installation is written by its identifier alone, needing no
    /// sealing, and comes back as itself; a token is still written as the
    /// bare sealed secret, so a file holding tokens is unchanged by the
    /// second shape existing.
    #[test]
    fn an_installation_survives_the_file_as_its_identifier() {
        let mut state = populated();
        state
            .apps
            .insert(Platform::GitHub, an_app(&[(77, "example", false)]));
        let project = *state.projects.keys().next().expect("a project");
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .access
            .insert(Platform::GitHub, Access::Installation { id: 77 });

        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        assert!(json.contains(r#""GitHub":{"installation":77}"#), "{json}");

        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        assert_eq!(
            reopened
                .projects
                .get(&project)
                .and_then(|watched| watched.access.get(&Platform::GitHub)),
            Some(&Access::Installation { id: 77 })
        );
    }

    /// An App this instance owns, installed as given: identifier, account,
    /// and whether on every repository. The key is a placeholder, since
    /// nothing here signs with it.
    fn an_app(installations: &[(u64, &str, bool)]) -> PlatformApp {
        PlatformApp {
            id: 4242,
            slug: "stageman-test".to_owned(),
            client_id: "Iv1.test".to_owned(),
            private_key: Secret::new("not-a-real-key".to_owned()),
            installations: installations
                .iter()
                .map(|(id, account, every_repository)| {
                    (
                        *id,
                        Installation {
                            account: (*account).to_owned(),
                            every_repository: *every_repository,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Where the App is installed travels through the file in the clear,
    /// beside its sealed key, and a file written before installations were
    /// kept opens with none — the literal older shape, per
    /// `docs/conventions.md` §4.
    #[test]
    fn an_apps_installations_survive_the_file_and_default_when_absent() {
        let mut state = populated();
        state.apps.insert(
            Platform::GitHub,
            an_app(&[(77, "example", false), (78, "acme", true)]),
        );
        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        assert!(
            json.contains(
                r#""installations":{"77":{"account":"example","every_repository":false},"78":{"account":"acme","every_repository":true}}"#
            ),
            "{json}"
        );
        assert!(
            !json.contains("not-a-real-key"),
            "the key is sealed: {json}"
        );
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        assert_eq!(
            reopened
                .apps
                .get(&Platform::GitHub)
                .map(|app| app.installations.clone()),
            state
                .apps
                .get(&Platform::GitHub)
                .map(|app| app.installations.clone())
        );

        // As the App was written before installations were kept: the four
        // fields and nothing else.
        let sealed_key = serde_json::to_string(
            &Secret::new("not-a-real-key".to_owned())
                .seal(&key(), [1; NONCE_LEN])
                .expect("sealing a well-formed secret"),
        )
        .expect("a sealed secret serialises");
        let older: SealedPlatformApp = serde_json::from_str(&format!(
            r#"{{"id":4242,"slug":"stageman-test","client_id":"Iv1.test","private_key":{sealed_key}}}"#
        ))
        .expect("an App written before installations still parses");
        assert!(older.installations.is_empty());
    }

    /// A channel app's client identifier and its workspaces' names travel
    /// through the file in the clear, its secrets sealed; a file written
    /// before the instance owned one opens with none, as does an app written
    /// before workspaces were kept.
    #[test]
    fn a_channel_apps_secrets_are_sealed_and_a_file_without_one_opens() {
        let mut state = populated();
        state.channel_apps.insert(
            Channel::Slack,
            ChannelApp {
                client_id: "1234.5678".to_owned(),
                client_secret: Secret::new("not-a-real-secret".to_owned()),
                app_token: Secret::new("xapp-not-a-real-token".to_owned()),
                workspaces: BTreeMap::from([(
                    "T0TEAM".to_owned(),
                    Workspace {
                        name: "Acme".to_owned(),
                        bot_user: "U0BOT".to_owned(),
                        bot_token: Secret::new("xoxb-not-a-real-token".to_owned()),
                    },
                )]),
            },
        );
        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        assert!(json.contains(r#""client_id":"1234.5678""#), "{json}");
        assert!(
            json.contains(r#""T0TEAM":{"name":"Acme","bot_user":"U0BOT","bot_token":{"#),
            "{json}"
        );
        for secret in [
            "not-a-real-secret",
            "xapp-not-a-real-token",
            "xoxb-not-a-real-token",
        ] {
            assert!(!json.contains(secret), "{secret} is sealed: {json}");
        }
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        assert_eq!(reopened.channel_apps, state.channel_apps);

        // As the last release wrote the file: no channel apps at all.
        let older: Snapshot =
            serde_json::from_str(&written_by_the_last_release()).expect("the older file parses");
        assert!(older.channel_apps.is_empty());

        // As an app would be written before workspaces were kept.
        let sealed = serde_json::to_string(
            &Secret::new("not-a-real-secret".to_owned())
                .seal(&key(), [1; NONCE_LEN])
                .expect("sealing a well-formed secret"),
        )
        .expect("a sealed secret serialises");
        let bare: SealedChannelApp = serde_json::from_str(&format!(
            r#"{{"client_id":"1234.5678","client_secret":{sealed},"app_token":{sealed}}}"#
        ))
        .expect("an app written before workspaces still parses");
        assert!(bare.workspaces.is_empty());
    }

    /// A binding through a workspace of the instance's app travels through
    /// the file as the workspace's identifier alone, in the clear, and comes
    /// back as that shape; the pair the last release wrote still comes back
    /// as an app of the project's own; and a project naming a workspace the
    /// app is not installed on is refused, with no app and with the app
    /// installed elsewhere.
    #[test]
    fn a_binding_through_a_workspace_is_written_by_its_identifier_and_checked() {
        let mut state = populated();
        let project = *state.projects.keys().next().expect("a project");
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .channels
            .insert(Channel::Slack, Binding::Workspace("T0TEAM".to_owned()));
        assert_eq!(
            state.check(),
            Err(Inconsistent::UnknownWorkspace {
                project,
                workspace: "T0TEAM".to_owned()
            })
        );
        state.channel_apps.insert(
            Channel::Slack,
            ChannelApp {
                client_id: "1234.5678".to_owned(),
                client_secret: Secret::new("not-a-real-secret".to_owned()),
                app_token: Secret::new("xapp-not-a-real-token".to_owned()),
                workspaces: BTreeMap::from([(
                    "T0OTHER".to_owned(),
                    Workspace {
                        name: "Other".to_owned(),
                        bot_user: "U0BOT".to_owned(),
                        bot_token: Secret::new("xoxb-not-a-real-token".to_owned()),
                    },
                )]),
            },
        );
        assert!(matches!(
            state.check(),
            Err(Inconsistent::UnknownWorkspace { .. })
        ));
        state
            .channel_apps
            .get_mut(&Channel::Slack)
            .expect("the app")
            .workspaces
            .insert(
                "T0TEAM".to_owned(),
                Workspace {
                    name: "Acme".to_owned(),
                    bot_user: "U0ACMEBOT".to_owned(),
                    bot_token: Secret::new("xoxb-acme-not-real".to_owned()),
                },
            );
        assert_eq!(state.check(), Ok(()));
        assert_eq!(
            state
                .speaking(project, Channel::Slack)
                .map(|speaking| speaking.credential.expose().to_owned()),
            Some("xoxb-acme-not-real".to_owned()),
            "what speaks is the workspace's bot token, from the app"
        );
        assert_eq!(
            state.bound_to(Channel::Slack, "T0TEAM").collect::<Vec<_>>(),
            vec![project]
        );
        assert!(state.bound_to(Channel::Slack, "T0OTHER").next().is_none());

        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        assert!(
            json.contains(r#""channels":{"Slack":{"workspace":"T0TEAM"}}"#),
            "{json}"
        );
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        assert_eq!(
            reopened
                .projects
                .get(&project)
                .map(|watched| watched.channels.clone()),
            state
                .projects
                .get(&project)
                .map(|watched| watched.channels.clone())
        );

        // The pair the last release wrote is an app of the project's own.
        let sealed_key = serde_json::to_string(
            &Secret::new("xoxb-not-a-real-token".to_owned())
                .seal(&key(), [1; NONCE_LEN])
                .expect("sealing a well-formed secret"),
        )
        .expect("a sealed secret serialises");
        let older: SealedBinding = serde_json::from_str(&format!(
            r#"{{"address":"C0123456789","credential":{sealed_key},"listen_credential":{sealed_key}}}"#
        ))
        .expect("the last release's pair still parses");
        assert!(matches!(older, SealedBinding::Own(_)));
        let workspace: SealedBinding =
            serde_json::from_str(r#"{"workspace":"T0TEAM"}"#).expect("the new shape parses");
        assert!(
            matches!(workspace, SealedBinding::Workspace { ref workspace } if workspace == "T0TEAM")
        );
    }

    /// A project reaching its repository through an installation names one
    /// the App holds, or the state is refused: with no App, and with an App
    /// installed elsewhere.
    #[test]
    fn a_project_naming_an_installation_the_app_does_not_hold_is_refused() {
        let mut state = populated();
        let project = *state.projects.keys().next().expect("a project");
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .access
            .insert(Platform::GitHub, Access::Installation { id: 77 });
        assert_eq!(
            state.check(),
            Err(Inconsistent::UnknownInstallation {
                project,
                installation: 77
            })
        );

        state
            .apps
            .insert(Platform::GitHub, an_app(&[(78, "example", false)]));
        assert!(matches!(
            state.check(),
            Err(Inconsistent::UnknownInstallation {
                installation: 77,
                ..
            })
        ));

        state
            .apps
            .insert(Platform::GitHub, an_app(&[(77, "example", false)]));
        assert_eq!(state.check(), Ok(()));
        assert_eq!(
            Inconsistent::UnknownInstallation {
                project,
                installation: 77
            }
            .to_string(),
            format!("project {project} names installation 77, which the App is not installed as")
        );
    }

    /// A brief and the watched rooms travel through the file whole, in the
    /// clear, and an errand from an app keeps saying which app.
    #[test]
    fn a_brief_and_the_watched_rooms_survive_the_file() {
        let mut state = populated();
        let (id, project) = state.projects.iter_mut().next().expect("a project");
        let id = *id;
        project.brief = "Ignore alerts below error.".to_owned();
        project.watched.insert(Room {
            channel: Channel::Slack,
            id: "C0BT53FM079".to_owned(),
        });
        project.attending.take(Errand {
            app: Some("GitHub".to_owned()),
            from: None,
            ..errand("an issue was opened")
        });

        let mut nonces = counting_nonces();
        let sealed = state.seal(&key(), &mut nonces).expect("seals");
        let written = serde_json::to_string(&sealed).expect("encodes");
        assert!(
            written.contains("Ignore alerts below error."),
            "in the clear"
        );
        let reopened: Snapshot = serde_json::from_str(&written).expect("parses");
        let opened = reopened.open(&key()).expect("opens");
        let project = opened.projects.get(&id).expect("the project");
        assert_eq!(project.brief, "Ignore alerts below error.");
        assert_eq!(
            project
                .watched
                .iter()
                .map(|room| room.id.as_str())
                .collect::<Vec<_>>(),
            vec!["C0BT53FM079"]
        );
        assert_eq!(
            project
                .attending
                .on()
                .and_then(|errand| errand.app.as_deref()),
            Some("GitHub")
        );
    }

    /// A job's inbox gives what waits in the order it arrived, keeps what a
    /// turn was given until that turn ends, hands back the last thing given
    /// when the turn did not take it, ahead of whatever arrived since, and
    /// drains in the order received.
    #[test]
    fn a_jobs_inbox_gives_in_order_and_hands_back_what_was_not_taken() {
        let mut inbox = Inbox::new();
        assert!(inbox.is_empty());
        assert_eq!(inbox.next(), None);
        assert_eq!(inbox.give(), None, "nothing waits");

        inbox.receive(errand("first"));
        inbox.receive(errand("second"));
        inbox.receive(errand("third"));
        assert_eq!(inbox.next().map(|next| next.said.as_str()), Some("first"));
        assert_eq!(inbox.give().map(|given| given.said.as_str()), Some("first"));
        assert_eq!(
            inbox.give().map(|given| given.said.as_str()),
            Some("second")
        );
        assert_eq!(inbox.next().map(|next| next.said.as_str()), Some("third"));
        assert!(!inbox.is_empty());
        let mut in_hand_only = Inbox::new();
        in_hand_only.receive(errand("alone"));
        assert!(in_hand_only.give().is_some());
        assert!(!in_hand_only.is_empty(), "something in hand is something");

        // The second was not taken: it waits again, ahead of the third, and
        // the first stays in hand.
        inbox.hand_back();
        assert_eq!(
            inbox
                .given
                .iter()
                .map(|given| given.said.as_str())
                .collect::<Vec<_>>(),
            vec!["first"]
        );
        assert_eq!(
            inbox
                .waiting
                .iter()
                .map(|waiting| waiting.said.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "third"]
        );

        // The turn ended: what it was given is finished with, in order.
        assert!(inbox.give().is_some());
        let finished = inbox.finish();
        assert_eq!(
            finished
                .iter()
                .map(|done| done.said.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert!(inbox.given.is_empty());
        assert_eq!(inbox.next().map(|next| next.said.as_str()), Some("third"));
        assert!(inbox.finish().is_empty(), "nothing is finished twice");

        // A stop drains everything, given first, and leaves nothing.
        assert!(inbox.give().is_some());
        inbox.receive(errand("fourth"));
        assert_eq!(
            inbox
                .drain()
                .iter()
                .map(|drained| drained.said.as_str())
                .collect::<Vec<_>>(),
            vec!["third", "fourth"]
        );
        assert!(inbox.is_empty());
        inbox.hand_back();
        assert!(inbox.is_empty(), "nothing to hand back");
    }

    /// A job's inbox travels through the file whole: what is in hand and
    /// what waits, in order.
    #[test]
    fn a_jobs_inbox_survives_the_file() {
        let mut state = populated();
        let job = state
            .projects
            .values_mut()
            .next()
            .and_then(|project| project.jobs.values_mut().next())
            .expect("a job");
        job.inbox.receive(errand("in hand"));
        job.inbox.receive(errand("waiting"));
        assert!(job.inbox.give().is_some());
        let before = job.inbox.clone();

        let mut nonces = counting_nonces();
        let sealed = state.seal(&key(), &mut nonces).expect("seals");
        let written = serde_json::to_string(&sealed).expect("encodes");
        let reopened: Snapshot = serde_json::from_str(&written).expect("parses");
        let opened = reopened.open(&key()).expect("opens");
        let job = opened
            .projects
            .values()
            .next()
            .and_then(|project| project.jobs.values().next())
            .expect("the job");
        assert_eq!(job.inbox, before);
        assert_eq!(job.inbox.given.len(), 1);
        assert_eq!(job.inbox.waiting.len(), 1);
    }

    /// A binding the last release wrote with both credentials comes through
    /// whole, which is the shape every real instance has been in.
    #[test]
    fn a_binding_with_both_credentials_written_by_the_last_release_comes_through_whole() {
        let parsed: Snapshot = serde_json::from_str(&written_by_the_last_release())
            .expect("an older file still parses");
        let state = parsed.open(&key()).expect("and still opens");

        let listening = state
            .projects
            .get(&ProjectId::from_uuid(Uuid::from_u128(4)))
            .expect("the other project survived");
        let bound = listening
            .channels
            .get(&Channel::Slack)
            .and_then(Binding::own)
            .expect("a binding with both credentials comes through whole");
        assert_eq!(bound.credential.expose(), "agent-token");
        assert_eq!(bound.listen_credential.expose(), "agent-token");
    }

    /// Where a name stops being believed.
    ///
    /// A snapshot is hand-editable by design, and its variable *names* travel
    /// in the clear — so editing one to something a runtime would read as an
    /// inline assignment costs nothing and needs no key. Refusing it here is
    /// what stops it reaching an argument list, and it is refused rather than
    /// dropped: silently discarding a variable would leave a job failing to
    /// authenticate against something, with nothing anywhere saying why.
    #[test]
    fn a_snapshot_naming_a_variable_that_could_not_be_delivered_is_refused() {
        let mut snapshot = sealed();
        let project = snapshot
            .projects
            .values_mut()
            .next()
            .expect("a project to edit");
        let sealed_value = project
            .variables
            .remove(VARIABLE)
            .expect("the fixture has one");
        project
            .variables
            .insert("NOT A NAME=oops".to_owned(), sealed_value);

        assert!(matches!(
            snapshot.open(&key()),
            Err(OpenError::VariableName(VariableNameError::NotAName))
        ));
    }

    #[test]
    fn the_wrong_key_does_not_open_a_snapshot() {
        assert!(matches!(
            sealed().open(&Key::new([8; 32])),
            Err(OpenError::Cipher)
        ));
    }

    #[test]
    fn an_altered_snapshot_is_refused_rather_than_misread() {
        // The cipher authenticates, so tampering is a failure rather than a
        // plausible-looking wrong answer. That is why it is an AEAD and not
        // just encryption.
        let mut snapshot = sealed();
        let sealed_token = &mut snapshot
            .agents
            .get_mut(&Agent::Claude)
            .expect("the agent is configured")
            .auth_token;
        let mut raw = BASE64
            .decode(&sealed_token.ciphertext)
            .expect("we wrote valid base64");
        raw[0] ^= 0xFF;
        sealed_token.ciphertext = BASE64.encode(raw);

        assert!(matches!(snapshot.open(&key()), Err(OpenError::Cipher)));
    }

    #[test]
    fn a_snapshot_naming_an_agent_it_does_not_configure_is_refused() {
        // The check that lets every later caller resolve a project's agents
        // without handling an absence. A file is untrusted input — hand-edited,
        // half-written, or written by an older version — and this is where that
        // stops being assumed.
        let mut snapshot = sealed();
        snapshot.agents.clear();
        assert!(matches!(
            snapshot.open(&key()),
            Err(OpenError::Inconsistent(
                Inconsistent::UnconfiguredProjectAgent {
                    agent: Agent::Claude,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn a_key_does_not_leak_when_formatted() {
        assert!(!format!("{:?}", key()).contains('7'));
    }

    /// Writing a key down and reading it back is the identity, and the text is
    /// the one everybody else already writes.
    ///
    /// The literal rather than a round-trip alone, which is the whole reason
    /// this encoder lives on the type: a round-trip would pass just as
    /// happily against a different alphabet or padding, as long as both halves
    /// agreed. What must not change is the *text*, because the environment
    /// variable and the generated file hold the same thing and are read by the
    /// same parser — see
    /// `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md`.
    #[test]
    fn a_key_written_down_is_the_key_that_comes_back() {
        let written = key().to_base64();

        assert_eq!(written, "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=");
        assert_eq!(
            Key::from_base64(&written).expect("what this wrote, it can read"),
            key()
        );
    }

    #[test]
    fn every_agent_says_what_it_is_good_for() {
        // The foreman picks an agent by reading this, so an empty one is
        // a silent failure rather than a cosmetic one.
        assert!(!Agent::Claude.description().is_empty());
    }

    /// Two projects whose credentials differ, so that "the wrong one" is
    /// detectable rather than merely absent.
    fn two_projects() -> (State, ProjectId, ProjectId) {
        let mut state = configured();
        let mine = ProjectId::from_uuid(Uuid::from_u128(3));
        let theirs = ProjectId::from_uuid(Uuid::from_u128(4));

        state.projects.insert(mine, a_project_with_a_job());

        let mut other = a_project_with_a_job();
        other.name = "somebody else".to_owned();
        other.access.insert(
            Platform::GitHub,
            Access::Token {
                secret: Secret::new("not-yours-and-never-was".to_owned()),
                owner: None,
                expires: None,
            },
        );
        other.channels.insert(
            Channel::Slack,
            Binding::Own(ChannelConfig {
                credential: Secret::new(ALIEN_CHANNEL_TOKEN.to_owned()),
                listen_credential: Secret::new("xapp-not-yours-either".to_owned()),
            }),
        );
        // The same name holding a different value, which is what makes the
        // escape test below able to fail: two projects whose variables merely
        // had different names would pass it by accident.
        other.variables.insert(
            VariableName::new(VARIABLE).expect("a deliverable name"),
            Variable::unexplained(Secret::new(ALIEN_VARIABLE_VALUE.to_owned())),
        );
        state.projects.insert(theirs, other);

        (state, mine, theirs)
    }

    #[test]
    fn a_foreman_is_handed_its_credential_and_no_platform_at_all() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_foreman(&state, mine).expect("a watched project");

        assert_eq!(handout.agent(), Agent::Claude);
        assert_eq!(handout.agent_credential().expose(), "agent-token");
        assert_eq!(handout.platforms().count(), 0);
        assert!(!handout.reaches(Platform::GitHub));
        assert!(
            handout.warrant().is_none(),
            "nothing to fetch, so nothing to fetch with"
        );
    }

    /// The asymmetry `docs/decisions/0027-a-channel-is-not-a-platform.md` is
    /// built on, asserted beside the test that establishes the other half.
    ///
    /// A foreman watches its project's channels — that is the whole of
    /// its remit — so a handout that withheld them the way it withholds
    /// platform credentials would leave it unable to work.
    #[test]
    fn a_foreman_is_handed_the_channels_it_has_to_watch() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_foreman(&state, mine).expect("a watched project");

        let watching = handout
            .channel(Channel::Slack)
            .expect("the binding came through");
        assert_eq!(watching.credential.expose(), CHANNEL_TOKEN);
        assert_eq!(handout.channels().count(), 1);
    }

    /// A job is handed its own warrant and told which platforms it reaches,
    /// and no platform credential at all: since
    /// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
    /// the credential is fetched with the warrant when a command needs it,
    /// and the handout has nowhere to put one.
    #[test]
    fn a_job_is_handed_a_warrant_and_no_platform_credential() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");

        assert_eq!(handout.agent_credential().expose(), "agent-token");
        assert!(handout.reaches(Platform::GitHub));
        assert_eq!(handout.platforms().collect::<Vec<_>>(), [Platform::GitHub]);
        assert_eq!(handout.warrant().map(Secret::expose), Some(WARRANT));
        // And the state it was built from does hold the token, so this is
        // about the handout rather than an empty fixture.
        assert!(matches!(
            state
                .projects
                .get(&mine)
                .and_then(|project| project.access.get(&Platform::GitHub)),
            Some(Access::Token { secret, .. }) if secret.expose() == TOKEN
        ));
    }

    /// The checkout is made from the handout, so the repository has to travel
    /// on it — for a job. A foreman has no workspace and nothing to check out,
    /// and that is decided by the constructor rather than left to a caller;
    /// see `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`.
    #[test]
    fn a_job_is_handed_its_projects_repository_and_a_foreman_is_not() {
        let (state, mine, _) = two_projects();
        let expected = state
            .projects
            .get(&mine)
            .map(|project| project.repository.https())
            .expect("a watched project");

        let job = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");
        assert_eq!(
            job.repository(),
            Some(expected.as_str()),
            "the address, as the checkout clones it"
        );

        let foreman = Handout::for_foreman(&state, mine).expect("a watched project");
        assert_eq!(foreman.repository(), None);
    }

    /// How a job speaks without a terminal — the invariant in
    /// `docs/architecture.md` §2 needs both halves of the binding, because an
    /// agent that holds the credential and not the address has nowhere to put
    /// the question.
    #[test]
    fn a_job_is_handed_the_channel_it_speaks_on() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");

        let speaking = handout
            .channel(Channel::Slack)
            .expect("the binding came through");
        assert_eq!(speaking.credential.expose(), CHANNEL_TOKEN);
    }

    /// The escape test `docs/conventions.md` §4 asks for, at the level where
    /// the selection actually happens: a handout built for one project must
    /// carry nothing belonging to another, and the two are distinguishable
    /// because their credentials differ rather than because one is empty.
    #[test]
    fn a_jobs_handout_carries_nothing_belonging_to_another_project() {
        let (state, mine, theirs) = two_projects();

        let ours = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");
        let alien = Handout::for_job(
            &state,
            Kit::defaults(Agent::Claude),
            theirs,
            Secret::new(ALIEN_WARRANT.to_owned()),
        )
        .expect("a watched project");

        // Neither carries a platform credential since 0077: each reaches its
        // project's platform through a warrant of its own, and the warrants
        // are what must not be confused.
        assert!(ours.reaches(Platform::GitHub) && alien.reaches(Platform::GitHub));
        assert_eq!(alien.warrant().map(Secret::expose), Some(ALIEN_WARRANT));
        assert_ne!(ours.warrant().map(Secret::expose), Some(ALIEN_WARRANT));

        // The same claim for the second map. Selection happens once per map,
        // so a map added without being selected from is exactly the mistake
        // this catches.
        assert_eq!(
            alien.channel(Channel::Slack).map(|c| c.credential.expose()),
            Some(ALIEN_CHANNEL_TOKEN)
        );
        for (_, bound) in ours.channels() {
            assert_ne!(bound.credential.expose(), ALIEN_CHANNEL_TOKEN);
        }

        // And the third. Both projects name the same variable, so this can
        // only pass if the selection actually happened — which is the whole of
        // what is defensible about a value this project never reads.
        assert!(
            alien
                .variables()
                .any(|(_, value)| value.expose() == ALIEN_VARIABLE_VALUE),
            "the other project's own value should be in its own handout"
        );
        for (_, value) in ours.variables() {
            assert_ne!(value.expose(), ALIEN_VARIABLE_VALUE);
        }
    }

    /// A job is handed what its operator gave the project for everything this
    /// system has never heard of.
    #[test]
    fn a_job_is_handed_its_projects_variables() {
        let (state, mine, _) = two_projects();

        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");
        let delivered: Vec<(&str, &str)> = handout
            .variables()
            .map(|(name, value)| (name.as_str(), value.expose()))
            .collect();

        assert_eq!(delivered, vec![(VARIABLE, VARIABLE_VALUE)]);
    }

    /// And a foreman is handed none, on the same terms it is handed no
    /// platform credential: it judges signals rather than acting on them.
    ///
    /// Asserted rather than assumed, because the two constructors are the only
    /// places this is decided and a field copied into the wrong one would look
    /// exactly like the right one.
    #[test]
    fn a_foreman_is_handed_no_variable_at_all() {
        let (state, mine, _) = two_projects();

        let handout = Handout::for_foreman(&state, mine).expect("a watched project");

        assert_eq!(handout.variables().count(), 0);
        assert_eq!(handout.variable_names().count(), 0);
    }

    /// The names are readable on their own, because a prompt names them and
    /// must never carry a value — a kickoff is stored on the job and crosses
    /// the snapshot boundary in the clear.
    #[test]
    fn a_handouts_variable_names_can_be_read_without_their_values() {
        let (state, mine, _) = two_projects();

        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");
        let named: Vec<&str> = handout.variable_names().map(VariableName::as_str).collect();

        assert_eq!(named, vec![VARIABLE]);
        // What the kickoff is told: the name with its note, and never the
        // value — see `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
        let told: Vec<(&str, &str)> = handout
            .variables_told()
            .map(|(name, note)| (name.as_str(), note))
            .collect();
        assert_eq!(told, vec![(VARIABLE, "the staging database, read-only")]);
    }

    /// The one grammar a name is read by, at each of its edges: the cap,
    /// the ends, the alphabet — and a name reads back as the text it was.
    #[test]
    fn a_jobs_name_is_read_by_the_one_grammar_at_its_edges() {
        let longest = "a".repeat(JobId::AT_MOST);
        let read = JobId::parse(&longest).expect("exactly the cap is a name");
        assert_eq!(read.as_str(), longest);
        assert_eq!(read.to_string(), longest);
        assert_eq!(
            JobId::parse(&format!("{longest}a")),
            Err(super::InvalidJobId::TooLong)
        );
        assert_eq!(JobId::parse(""), Err(super::InvalidJobId::Empty));
        assert_eq!(
            JobId::parse("-fix"),
            Err(super::InvalidJobId::HyphenAtAnEnd)
        );
        assert_eq!(
            JobId::parse("fix-"),
            Err(super::InvalidJobId::HyphenAtAnEnd)
        );
        assert_eq!(
            JobId::parse("Fix"),
            Err(super::InvalidJobId::Character('F'))
        );
        assert_eq!(
            JobId::parse("fix_it"),
            Err(super::InvalidJobId::Character('_'))
        );
        assert_eq!(
            JobId::parse("fix-login-timeout--3f9a2c1b").map(|name| name.to_string()),
            Ok("fix-login-timeout--3f9a2c1b".to_owned())
        );
    }

    /// The fold: lowercase, every run of anything else one hyphen, none at
    /// either end, and a cut that lands inside a word steps back to the
    /// last whole one — unless what is left is one word, which is cut where
    /// it is.
    #[test]
    fn a_title_folds_to_a_slug_and_is_cut_on_a_word() {
        assert_eq!(super::slug("Fix: the LOGIN!!", 40), "fix-the-login");
        assert_eq!(super::slug("--fix  it--", 40), "fix-it");
        assert_eq!(super::slug("", 40), "");
        // Exactly the cap is left alone; one more is cut on a word.
        assert_eq!(super::slug("fix the login", 13), "fix-the-login");
        assert_eq!(super::slug("fix the login", 12), "fix-the");
        assert_eq!(super::slug("fix the login", 9), "fix-the");
        // A cut that lands on a hyphen, or just after one, keeps the words
        // before it whole.
        assert_eq!(super::slug("fix the login", 7), "fix-the");
        assert_eq!(super::slug("fix the login", 8), "fix-the");
        // One word longer than the cap is cut where it is; a word after a
        // whole one is dropped.
        assert_eq!(super::slug("abcdefghij", 5), "abcde");
        assert_eq!(super::slug("abc defghij", 5), "abc");
    }

    /// `docs/conventions.md` §4, for the map added last.
    ///
    /// The name is expected to show, because naming what is present is what a
    /// `Debug` on this type is for. The value is not.
    #[test]
    fn a_handout_does_not_leak_a_variables_value_when_formatted() {
        let (state, mine, _) = two_projects();

        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");
        let shown = format!("{handout:?}");

        assert!(!shown.contains(VARIABLE_VALUE), "{shown}");
        assert!(shown.contains(VARIABLE), "{shown}");
    }

    /// A handout has nowhere to speak until it is narrowed to a place.
    ///
    /// The absence is what makes a handout before narrowing mean "offered
    /// nothing to speak with", so it is asserted rather than assumed.
    #[test]
    fn a_handout_has_nowhere_to_speak_until_it_is_given_a_place() {
        let (state, mine, _) = two_projects();

        let job = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");
        assert!(job.place().is_none());
        assert!(
            Handout::for_foreman(&state, mine)
                .expect("a watched project")
                .place()
                .is_none(),
            "a foreman is narrowed to the thread it answers in, every turn"
        );

        let narrowed = job.speaking_in(Place::root(a_room()));
        assert_eq!(
            narrowed.place().map(|place| place.room.id.as_str()),
            Some(JOB_ROOM)
        );
        assert_eq!(
            narrowed.place().and_then(|place| place.thread.as_deref()),
            None,
            "a job speaks at the root of its room until asked in a thread"
        );
        // Narrowing changes where it speaks and nothing about what it holds.
        assert_eq!(narrowed.channels().count(), 1);
    }

    /// A warrant is sealed like every other credential, and survives.
    /// The credential that listens never reaches a handout at all.
    ///
    /// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md` says the token
    /// that opens an event stream stays in the daemon, and this is what makes
    /// that a property rather than a promise: a handout carries [`Speaking`],
    /// which has nowhere to put it.
    ///
    /// Worth a test even so, because the first version of this cloned the whole
    /// binding and carried the listening credential into every job's handout —
    /// where nothing delivered it onwards, so nothing failed and nothing said
    /// so.
    #[test]
    fn a_handout_never_carries_the_credential_that_listens() {
        let (state, mine, _) = two_projects();

        for handout in [
            Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
                .expect("a watched project"),
            Handout::for_foreman(&state, mine).expect("a watched project"),
        ] {
            let bound = handout.channel(Channel::Slack).expect("a binding");
            assert_eq!(bound.credential.expose(), CHANNEL_TOKEN);

            // The whole structure, in case a field is added later that does
            // carry it. Formatting is redacted, so this reads the values.
            let carried: Vec<&str> = handout
                .channels()
                .map(|(_, speaking)| speaking.credential.expose())
                .collect();
            assert!(!carried.contains(&LISTEN_TOKEN), "{carried:?}");
        }

        // And the state it was built from does hold it, so the test above is
        // about the handout rather than about an empty fixture.
        assert_eq!(
            state
                .projects
                .get(&mine)
                .expect("the project")
                .channels
                .get(&Channel::Slack)
                .and_then(Binding::own)
                .expect("its binding, an app of its own")
                .listen_credential
                .expose(),
            LISTEN_TOKEN
        );
    }

    #[test]
    fn a_handout_for_a_project_this_instance_does_not_watch_is_refused() {
        let (state, _, _) = two_projects();
        let stranger = ProjectId::from_uuid(Uuid::from_u128(99));

        let refused = Handout::for_job(&state, Kit::defaults(Agent::Claude), stranger, warrant());

        assert!(matches!(refused, Err(HandoutError::UnknownProject(id)) if id == stranger));
    }

    #[test]
    fn a_handout_for_an_agent_with_no_configuration_is_refused() {
        let (mut state, mine, _) = two_projects();
        state.agents.clear();

        let refused = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant());

        assert!(matches!(
            refused,
            Err(HandoutError::UnconfiguredAgent(Agent::Claude))
        ));
        assert!(Handout::for_foreman(&state, mine).is_err());
    }

    #[test]
    fn a_handout_does_not_leak_a_credential_when_formatted() {
        let (state, mine, _) = two_projects();
        let handout = Handout::for_job(&state, Kit::defaults(Agent::Claude), mine, warrant())
            .expect("a watched project");

        let shown = format!("{handout:?}");

        assert!(!shown.contains("agent-token"), "{shown}");
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains(CHANNEL_TOKEN), "{shown}");
        assert!(!shown.contains(WARRANT), "{shown}");
        assert!(
            shown.contains("warrant"),
            "it should still say it holds one"
        );
        assert!(
            shown.contains("GitHub"),
            "it should still say what it holds"
        );
        assert!(shown.contains("Slack"), "{shown}");
    }

    /// What an environment can carry, which is what this rule is about.
    ///
    /// Lowercase is in the list deliberately: the rule is not a house style,
    /// and the proxy variables an operator reaches for first are spelled that
    /// way. A digit is fine anywhere but first.
    #[test]
    fn a_name_a_container_could_be_given_is_accepted() {
        for name in [
            "STRIPE_API_KEY",
            "http_proxy",
            "PATH",
            "_leading_underscore",
            "S3_BUCKET_2",
            "a",
        ] {
            assert!(
                VariableName::new(name).is_ok(),
                "{name} is a name a container could be given"
            );
        }
    }

    /// The refusal that keeps a credential out of the process table.
    ///
    /// A runtime told to forward a variable whose name contains an equals sign
    /// sets it inline instead — measured on Docker and on Podman — so the
    /// value would travel through the command line rather than through an
    /// environment. This is the only thing standing between an operator typing
    /// that and it happening.
    #[test]
    fn a_name_holding_an_equals_sign_is_refused() {
        assert_eq!(
            VariableName::new("STRIPE_API_KEY=sk-test-not-a-real-key"),
            Err(VariableNameError::NotAName)
        );
    }

    /// The rest of the rule, each with the answer that says which part broke.
    #[test]
    fn a_name_an_environment_could_not_carry_is_refused() {
        assert_eq!(VariableName::new(""), Err(VariableNameError::Empty));
        assert_eq!(
            VariableName::new("2_MANY"),
            Err(VariableNameError::LeadingDigit)
        );
        for name in ["has space", "has-dash", "has.dot", "has\0nul", "é"] {
            assert_eq!(
                VariableName::new(name),
                Err(VariableNameError::NotAName),
                "{name} is not a name an environment could carry"
            );
        }
    }

    /// A refusal says which rule broke and never what was in the box.
    ///
    /// The name is not a credential, but an operator pasting a value into the
    /// What is unfinished is everything that is not over, and nothing else.
    ///
    /// Both halves matter and each fails differently. Missing a job leaves a
    /// container nothing reconciles against; including a retired one has the
    /// sweep reporting the same casualty on every start for ever.
    #[test]
    fn unfinished_is_every_job_that_is_not_over() {
        let mut state = populated();
        let project = *state.projects.keys().next().expect("a project");
        let jobs = &mut state.projects.get_mut(&project).expect("the project").jobs;
        jobs.clear();

        let going = JobId::from_uuid(Uuid::from_u128(21));
        let waiting = JobId::from_uuid(Uuid::from_u128(22));
        let over = JobId::from_uuid(Uuid::from_u128(23));
        for (id, progress) in [
            (going.clone(), Progress::Working),
            (waiting.clone(), Progress::Idle(Waiting::Asked)),
            (over, Progress::Retired(Outcome::Done)),
        ] {
            let mut job = Job::new(
                Kit::defaults(Agent::Claude),
                "a reason".to_owned(),
                "some work".to_owned(),
                Timestamp::UNIX_EPOCH,
                warrant(),
            );
            job.progress = progress;
            jobs.insert(id, job);
        }

        let mut unfinished: Vec<JobId> = state.unfinished().collect();
        unfinished.sort();
        assert_eq!(unfinished, vec![going, waiting]);
    }

    /// wrong box is exactly the mistake this type exists to catch — so the
    /// error has to be safe to log even when the "name" is a token.
    #[test]
    fn a_refused_name_does_not_repeat_what_was_typed() {
        let pasted = "sk-test-not-a-real-key=oops";
        let refused = VariableName::new(pasted).expect_err("that is not a name");

        assert!(!format!("{refused}").contains(pasted));
        assert!(!format!("{refused:?}").contains(pasted));
    }

    #[test]
    fn a_jobs_progress_survives_the_snapshot_boundary() {
        let mut state = populated();
        let project = state
            .projects
            .values_mut()
            .next()
            .expect("the populated state has one");
        let job = project
            .jobs
            .values_mut()
            .next()
            .expect("that project has one job");
        job.progress = Progress::Idle(Waiting::Failed("the credential had expired".to_owned()));

        let recovered = state
            .seal(&key(), &mut counting_nonces())
            .expect("sealing succeeds")
            .open(&key())
            .expect("and opens again");

        let carried = recovered
            .projects
            .values()
            .next()
            .and_then(|project| project.jobs.values().next())
            .map(|job| job.progress.clone());

        assert_eq!(
            carried,
            Some(Progress::Idle(Waiting::Failed(
                "the credential had expired".to_owned()
            )))
        );
    }

    #[test]
    fn working_finds_a_job_wherever_its_project_is() {
        let state = populated();
        let found: Vec<JobId> = state.working().collect();

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(state.job(&found[0]).is_some());
    }

    #[test]
    fn a_job_that_has_finished_is_not_running() {
        let mut state = populated();
        let id = state.working().next().expect("one to start with");

        state.job_mut(&id).expect("it is there").progress = Progress::Idle(Waiting::Silent);

        assert_eq!(state.working().count(), 0);
        assert!(
            state.job(&id).is_some(),
            "finishing is not forgetting: the record stays"
        );
    }

    fn errand(said: &str) -> Errand {
        Errand {
            said: said.to_owned(),
            thread: Thread {
                channel: Channel::Slack,
                room: CHANNEL_ADDRESS.to_owned(),
                id: format!("{said}.thread"),
            },
            from: Some("U0HUMAN".to_owned()),
            message: Some(format!("{said}.thread")),
            app: None,
        }
    }

    /// An idle foreman starts on what arrives; a working one queues it.
    #[test]
    fn the_first_message_starts_a_turn_and_the_rest_wait() {
        let mut attending = Attending::default();
        assert_eq!(attending, Attending::Idle);

        assert_eq!(attending.take(errand("first")), Taken::Started);
        assert_eq!(attending.on().map(|e| e.said.as_str()), Some("first"));
        assert_eq!(attending.waiting(), 0);

        // Everything after it waits, however many arrive.
        for said in ["second", "third"] {
            assert_eq!(attending.take(errand(said)), Taken::Waiting);
        }
        assert_eq!(attending.on().map(|e| e.said.as_str()), Some("first"));
        assert_eq!(attending.waiting(), 2);
    }

    /// Two messages arriving together cannot both start a turn.
    ///
    /// Not a test of locking — that is the caller's — but of the reason
    /// locking is enough: taking is one operation, so whichever runs first
    /// leaves a state the second cannot mistake for idle.
    #[test]
    fn only_one_message_can_ever_start_a_turn() {
        let mut attending = Attending::Idle;
        let started = [errand("a"), errand("b")]
            .into_iter()
            .filter(|_| true)
            .map(|e| attending.take(e))
            .filter(|taken| *taken == Taken::Started)
            .count();

        assert_eq!(started, 1, "exactly one of them may begin");
    }

    /// Finishing picks up the next, in the order they arrived.
    #[test]
    fn messages_are_picked_up_in_the_order_they_arrived() {
        let mut attending = Attending::Idle;
        for said in ["first", "second", "third"] {
            attending.take(errand(said));
        }

        assert_eq!(attending.finish().map(|e| e.said.as_str()), Some("second"));
        assert_eq!(attending.waiting(), 1);
        assert_eq!(attending.finish().map(|e| e.said.as_str()), Some("third"));
        assert_eq!(attending.waiting(), 0);
    }

    /// A foreman goes idle only when nothing is left.
    ///
    /// The invariant the shape exists for: there is no way to reach `Idle`
    /// while anything waits, because `Idle` has nowhere to keep it. This
    /// asserts the behaviour; the type is what makes it true.
    #[test]
    fn a_foreman_goes_idle_only_with_an_empty_inbox() {
        let mut attending = Attending::Idle;
        attending.take(errand("only"));

        assert_eq!(attending.finish(), None);
        assert_eq!(attending, Attending::Idle);
        assert_eq!(attending.waiting(), 0);

        // And finishing when there was nothing in hand changes nothing.
        assert_eq!(attending.finish(), None);
        assert_eq!(attending, Attending::Idle);
    }

    /// What is waiting survives the snapshot, because a person sent it.
    #[test]
    fn an_inbox_survives_the_snapshot_boundary() {
        let mut state = populated();
        let project = *state.projects.keys().next().expect("a project");
        let attending = &mut state
            .projects
            .get_mut(&project)
            .expect("the project")
            .attending;
        attending.take(errand("in hand"));
        attending.take(errand("waiting"));

        let json = serde_json::to_string(
            &state
                .seal(&key(), &mut counting_nonces())
                .expect("sealing cannot fail"),
        )
        .expect("a snapshot serialises");
        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");

        let attending = &reopened
            .projects
            .get(&project)
            .expect("the project survived")
            .attending;
        assert_eq!(attending.on().map(|e| e.said.as_str()), Some("in hand"));
        assert_eq!(attending.waiting(), 1);
    }

    /// A job's warrant is sealed in the file like every other credential,
    /// never written in the clear, and comes back as itself.
    #[test]
    fn a_jobs_warrant_is_sealed_in_the_file_and_survives_it() {
        let json = serde_json::to_string(&sealed()).expect("a snapshot serialises");
        assert!(!json.contains(WARRANT), "{json}");

        let reopened: Snapshot = serde_json::from_str(&json).expect("and parses back");
        let reopened = reopened.open(&key()).expect("and opens");
        let job = reopened
            .projects
            .values()
            .next()
            .and_then(|project| project.jobs.values().next())
            .expect("the job survived");

        assert_eq!(job.warrant().map(Secret::expose), Some(WARRANT));
    }

    /// A warrant names the job that holds it, and only while that job is
    /// not over: a retired job's container is gone with everything in it,
    /// so nothing can present its warrant, and the route that asks this
    /// refuses it.
    #[test]
    fn a_warrant_names_its_job_until_that_job_is_over() {
        let mut state = populated();
        let project = *state.projects.keys().next().expect("a project");
        let job = state
            .projects
            .get(&project)
            .and_then(|watched| watched.jobs.keys().next())
            .cloned()
            .expect("a job");

        assert_eq!(
            state.job_with_warrant(WARRANT),
            Some((project, job.clone()))
        );
        assert_eq!(state.job_with_warrant("not-a-warrant"), None);
        assert_eq!(state.job_with_warrant(""), None);

        state.job_mut(&job).expect("the job").progress = Progress::Retired(Outcome::Done);
        assert_eq!(
            state.job_with_warrant(WARRANT),
            None,
            "a retired job's warrant buys nothing"
        );
    }

    /// A job's project is findable from the job alone.
    #[test]
    fn a_job_names_the_project_it_belongs_to() {
        let (state, project, job) = listening();

        assert_eq!(state.project_of(&job), Some(project));
        assert_eq!(
            state.project_of(&JobId::from_uuid(Uuid::from_u128(99))),
            None
        );
    }

    #[test]
    fn a_job_this_instance_never_had_is_not_found() {
        let state = populated();

        assert!(state.job(&JobId::from_uuid(Uuid::from_u128(404))).is_none());
    }

    /// A file describing an instance that cannot exist is refused where it is
    /// read, rather than believed and acted on.
    #[test]
    fn a_snapshot_giving_a_project_no_kits_is_refused() {
        let mut snapshot = sealed();
        for project in snapshot.projects.values_mut() {
            project.kits.clear();
        }

        assert!(matches!(
            snapshot.open(&key()),
            Err(OpenError::Inconsistent(Inconsistent::NoKits(_)))
        ));
    }

    /// An address is what an operator pastes, with what they paste beside it
    /// forgiven, and nothing else.
    #[test]
    fn a_repository_address_is_https_on_github_with_an_owner_and_a_name() {
        let parsed = RepositoryAddress::parse(" https://github.com/HernanFdz/stageman.git/ ")
            .expect("an address");
        assert_eq!(parsed.owner, "HernanFdz");
        assert_eq!(parsed.name, "stageman");
        assert_eq!(parsed.https(), "https://github.com/HernanFdz/stageman");
        assert_eq!(
            RepositoryAddress::parse("https://WWW.GitHub.com/owner/name").map(|a| a.https()),
            Ok("https://github.com/owner/name".to_owned()),
            "the host is not case-sensitive"
        );

        assert_eq!(
            RepositoryAddress::parse("http://github.com/owner/name"),
            Err(RepositoryError::NotHttps)
        );
        assert_eq!(
            RepositoryAddress::parse("git@github.com:owner/name.git"),
            Err(RepositoryError::NotHttps)
        );
        assert_eq!(
            RepositoryAddress::new("HernanFdz", "stageman")
                .expect("an address")
                .to_string(),
            "HernanFdz/stageman",
            "shown as the platform says it"
        );
        assert_eq!(
            RepositoryAddress::parse("https://example.invalid/owner/name"),
            Err(RepositoryError::NotOnGitHub)
        );
        for path in [
            "",
            "owner",
            "owner/",
            "owner/name/tree/main",
            "owner/na me",
            "/name",
        ] {
            assert_eq!(
                RepositoryAddress::parse(&format!("https://github.com/{path}")),
                Err(RepositoryError::NotOwnerAndName),
                "{path:?}"
            );
        }
    }
}
