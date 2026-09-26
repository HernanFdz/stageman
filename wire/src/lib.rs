//! What crosses between the dashboard and the instance.
//!
//! Every type here is plain and serialisable, built by the instance from the
//! domain and never the domain itself — see
//! `docs/decisions/0022-the-browser-never-sees-the-domain.md`. What is in the
//! browser is in the browser's hands, so what reaches it is chosen rather
//! than inherited. Note what none of these types can carry: a credential.
//! That is a property of the types rather than of the routes, and a field
//! added here is a field the browser gets.
//!
//! Names are identifiers where a browser sends them back and display names
//! where a person reads them, and the two are deliberately never the same
//! field.

use std::fmt;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- agents

/// One agent this instance could run, as much of it as a page may know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// What the browser names it back as.
    pub id: String,
    /// What it is called on screen.
    pub name: String,
    /// What it is good for.
    pub description: String,
    /// Whether a credential has been supplied for it.
    pub configured: bool,
    /// The projects that would break if it were forgotten. Empty means it
    /// can go; anything else is what a refusal would say, and carrying it
    /// lets a page grey the button *and* explain.
    pub used_by: Vec<String>,
}

// ------------------------------------------------------------------ apps

/// What the Instance page shows.
///
/// The Apps this instance owns on each platform, and what the last
/// registration said if it failed — see
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Apps {
    /// The GitHub App, if one is registered.
    pub github: Option<PlatformAppView>,
    /// Why the last registration was not kept, if the last one was not.
    pub failed: Option<String>,
    /// The Slack app, if one is registered — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    pub slack: Option<ChannelAppView>,
    /// Where the platform's form for a new Slack app is, with the manifest
    /// filled in: the guide beside the boxes that register one, and beside
    /// a project's own.
    pub slack_form: String,
}

/// An App the instance owns, as much of it as a page may know: never its
/// key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformAppView {
    /// Its slug, which is what a person reads.
    pub slug: String,
    /// Where it is seen on the platform.
    pub link: String,
    /// Where it is installed, as the instance has learned — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    pub installations: Vec<InstallationView>,
    /// Why the last installation was not kept, if the last one was not.
    pub install_failure: Option<String>,
}

/// A Slack app the instance owns, as much of it as a page may know: never
/// its secrets — see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelAppView {
    /// Its client identifier, which is what a person reads and what tells
    /// it from another app on the platform's page.
    pub client_id: String,
    /// The workspaces it is installed in, as the instance has learned.
    pub workspaces: Vec<WorkspaceView>,
    /// Why the last install was not kept, if the last one was not.
    pub install_failure: Option<String>,
}

/// One workspace the instance's app is installed in, as a page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceView {
    /// Its identifier on the platform.
    pub id: String,
    /// Its name, for a person.
    pub name: String,
    /// The projects that speak through it, by name; empty when none does,
    /// which is when it may be forgotten.
    pub used_by: Vec<String>,
}

/// One installation of the App, as a page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationView {
    /// Its identifier on the platform, which is what a draft names.
    pub id: u64,
    /// The account it is on, as the platform spells it.
    pub account: String,
    /// Whether it covers every repository of that account rather than
    /// chosen ones.
    pub every_repository: bool,
    /// The projects reaching their repository through it, by name: what
    /// forgetting it would leave without access. Empty means it can go.
    pub used_by: Vec<String>,
}

/// The form that registers an App, as the browser posts it.
///
/// Where to, and the manifest it carries, with the state token minted for
/// this one attempt. Composed on the server, like every address a page
/// uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    /// Where the form posts to.
    pub action: String,
    /// The manifest, as the form's one field.
    pub manifest: String,
    /// The state token the platform hands back, for the page to show and
    /// nobody to type.
    pub state: String,
}

// -------------------------------------------------------------- instance

/// One instance, as the line at the foot of every page shows it: this
/// machine and this build. Counts and names, and nothing that could be a
/// credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// Where this machine's container runtime was found. A path rather
    /// than a version, because the operator's next question when something
    /// misbehaves is *which one is it using*.
    pub container_runtime: String,
    /// How many agents are configured. A count and not a list, because an
    /// agent's configuration is a credential.
    pub agents: usize,
    /// The domain the dashboard answers at and jobs are shown under.
    pub domain: String,
    /// Which build this is, as the startup block says it.
    pub version: String,
}

// ------------------------------------------------------------------ home

/// What the first page shows: the idle jobs of every project, the working
/// ones, and the projects — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    /// Every token about to stop working, soonest first, and every one
    /// that has: what a person replaces — see
    /// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    pub expiring: Vec<ExpiringToken>,
    /// Every idle job, longest waiting first: the ones a person does
    /// something about, which is what *idle* means.
    pub needs_you: Vec<ProjectJob>,
    /// Every working job, newest first.
    pub working: Vec<ProjectJob>,
    /// Every project.
    pub projects: Vec<Project>,
}

/// A project's token that is about to stop working, or has: raised on the
/// first page for the person who replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiringToken {
    /// The project, by identifier.
    pub project: String,
    /// The project, by name.
    pub project_name: String,
    /// Whose the token is, where that was read.
    pub owner: Option<String>,
    /// When it expires, as the wire spells a moment.
    pub expires: String,
    /// Whether that moment has passed.
    pub expired: bool,
}

/// One job beside the project it belongs to, for a list that spans projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectJob {
    /// The project, by identifier.
    pub project: String,
    /// The project, by name.
    pub project_name: String,
    /// The job.
    pub job: Job,
}

// -------------------------------------------------------------- projects

/// A repository on the platform, as a page names it: an owner and a name,
/// shown as `owner/name` — see
/// `docs/decisions/0079-a-repository-is-an-owner-and-a-name.md`.
///
/// Two parts rather than text or an address, so that a page shows what a
/// person says and composes nothing: the address a browser opens comes
/// beside it, composed on the server.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct Repository {
    /// Who owns it, as the platform spells it.
    pub owner: String,
    /// What it is called there.
    pub name: String,
}

impl fmt::Display for Repository {
    /// `owner/name`, as the platform says it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// One project, as much of it as a page is allowed to know.
///
/// Which platforms have a credential and which channels are bound is here;
/// what any of them is, is not — and a channel's address is withheld on the
/// same terms even though it is not a secret, because nothing on a screen
/// needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// What the browser names it back as.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Where its jobs work.
    pub repository: Repository,
    /// The same, as an address a browser can open. Composed on the server,
    /// like every address a page links.
    pub repository_link: String,
    /// How its foreman's agent is set, as the identifiers a browser sends
    /// back — not the names a person reads.
    pub foreman: Fitted,
    /// The kits its jobs may run on, as the form edits them. Never empty in
    /// a valid instance.
    pub kits: Vec<KitDraft>,
    /// How it reaches its repository's platform, in one of two shapes, or
    /// not at all — see
    /// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
    /// and `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`. None only for a project the last release wrote without a
    /// token.
    pub access: Option<AccessView>,
    /// How it talks on Slack, in one of two shapes — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    /// None only for a project the last release wrote without a binding,
    /// which can start no job until one is set.
    pub binding: Option<BindingView>,
    /// The variables its jobs are given: names and notes, and never values.
    pub variables: Vec<Variable>,
    /// What its operator wrote for its foreman, as the form edits it.
    pub brief: String,
    /// The rooms its foreman watches, by the platform's identifier: shown
    /// and never edited here, since a room is watched by asking the foreman
    /// in it.
    pub watched: Vec<String>,
    /// The room its foreman's transcript is posted in, by the platform's
    /// identifier, once one has been made.
    pub foreman_room: Option<String>,
    /// The same, as an address a person can open, while the channel is
    /// connected and has said where its workspace is — see
    /// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    pub foreman_room_link: Option<String>,
    /// Whether its foreman is on a message right now.
    pub attending: bool,
    /// How many of its jobs are still running.
    pub working: usize,
    /// How many jobs it has had, running or finished.
    pub jobs: usize,
    /// Where the platform's form for a token is, filled in and named for
    /// this project: the guide beside the box that takes one — see
    /// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
    pub token_form: String,
}

impl Project {
    /// Whether nothing of this project's is currently running.
    ///
    /// The screen's version of the rule the instance enforces, and the
    /// reason it is a method: a page that offered a control the instance
    /// would refuse would be lying.
    #[must_use]
    pub const fn idle(&self) -> bool {
        self.working == 0
    }
}

/// How a project reaches its repository's platform, as a page sees it.
///
/// The shape, and for an installation the account it is on. Never the
/// token, and never the installation's identifier, which is the
/// instance's business — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum AccessView {
    /// A token, pasted and checked, with what the platform said of it —
    /// see `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    Token {
        /// Whose it is, where that was read: an account's name.
        owner: Option<String>,
        /// When the platform stops accepting it, as the wire spells a
        /// moment, where the platform said.
        expires: Option<String>,
        /// Whether that moment has passed, as the instance read the page.
        expired: bool,
    },
    /// An installation of the App.
    Installation {
        /// The account it is on.
        account: String,
    },
}

/// How a project talks on Slack, as a page may know it.
///
/// Through a workspace of the app the instance owns, or through an app of
/// its own — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
/// Never a credential: an app of its own is named by where it speaks, once
/// the channel has said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum BindingView {
    /// An app of the project's own.
    Own {
        /// Where it speaks, as the channel said its workspace is: an
        /// address a person can open, once the connection has been told.
        url: Option<String>,
    },
    /// A workspace of the app the instance owns.
    Workspace {
        /// Its identifier on the platform.
        id: String,
        /// Its name, for a person.
        name: String,
    },
}

/// Whether the workspace a form's tab went out to install on has come back.
///
/// Under the state the tab carried: what the form asks on every tick while
/// its tab is out, and moves onto once it has — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "arrived", rename_all = "snake_case")]
pub enum WorkspaceArrival {
    /// The tab has not come back yet.
    NotYet,
    /// It has, and the workspace is kept beside the app.
    Installed {
        /// Its identifier on the platform.
        id: String,
        /// Its name, for a person.
        name: String,
    },
}

/// What a pair of tokens for an app of a project's own was found to be
/// when the form's panel checked it: accepted, and where it speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bound {
    /// Where the app speaks, as the channel said its workspace is: an
    /// address a person can open.
    pub url: String,
}

/// Where a person installs the App, minted for one press — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
///
/// The link carries the state, and the state is what the page keeps: the
/// installation the platform brings back under it is that page's, and no
/// other's. Composed on the server, like every address a page links.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallLink {
    /// Where the tab goes.
    pub link: String,
    /// What the page asks by, once the tab has come back.
    pub state: String,
}

impl fmt::Debug for InstallLink {
    /// Names neither: the state buys an installation for whoever holds
    /// it, and the link carries the state.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InstallLink { .. }")
    }
}

/// How the form says the repository is reached, as the browser sends it
/// back — see `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
///
/// One of three shapes, with the repository inside the shape: so a form
/// cannot send a token and an installation at once, cannot send a
/// repository with no access to reach it, and drops the repository when
/// it leaves the shape that reached it. An access left unsaid is the one
/// the project holds, which only an existing project has: a token, or an
/// installation. An installation is otherwise named by the state its
/// install came back under, never by its identifier, so a form can name
/// only what its own tab brought back.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum AccessDraft {
    /// Nothing chosen yet. Refused, whichever form this is.
    #[default]
    None,
    /// Through the App, on an installation of it.
    App {
        /// The state the install came back under, or none for the
        /// installation the project holds.
        arrival: Option<String>,
        /// The repository, once one is chosen.
        repository: Option<Repository>,
    },
    /// With a token: one set in the form, or the one the project holds.
    Token {
        /// The token, or none for the one the project holds.
        token: Option<String>,
        /// The repository, once one is chosen.
        repository: Option<Repository>,
    },
}

impl AccessDraft {
    /// The repository this reaches, once one is chosen.
    #[must_use]
    pub const fn repository(&self) -> Option<&Repository> {
        match self {
            Self::None => None,
            Self::App { repository, .. } | Self::Token { repository, .. } => repository.as_ref(),
        }
    }

    /// Whether this says how the repository is reached, for the form given:
    /// an installation come back or a token set, or whichever the project
    /// holds where there is one.
    #[must_use]
    pub fn is_reached(&self, filling: &Filling) -> bool {
        match self {
            Self::None => false,
            Self::App {
                arrival: Some(state),
                ..
            } => !state.trim().is_empty(),
            Self::Token {
                token: Some(token), ..
            } => !token.trim().is_empty(),
            Self::App { arrival: None, .. } | Self::Token { token: None, .. } => {
                !filling.creating()
            }
        }
    }
}

impl fmt::Debug for AccessDraft {
    /// Names the shape and the repository, and neither the token nor the
    /// state, which buys an installation for whoever holds it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::App {
                arrival,
                repository,
            } => f
                .debug_struct("App")
                .field("arrival", &arrival.as_ref().map(|_| "<redacted>"))
                .field("repository", repository)
                .finish(),
            Self::Token { token, repository } => f
                .debug_struct("Token")
                .field("token", &token.as_ref().map(|_| "<redacted>"))
                .field("repository", repository)
                .finish(),
        }
    }
}

/// What a form asks the repositories of.
///
/// The installation its own tab brought back, a token it is about to set,
/// or what a project already holds — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
/// Never the App as a whole: a form is shown what its own access reaches
/// and nothing of the instance's other installations.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "through", rename_all = "snake_case")]
pub enum Through {
    /// The installation that came back under a state minted for this page.
    Arrived {
        /// The state the install link carried.
        state: String,
    },
    /// A token, as pasted and not yet kept.
    Token {
        /// The token.
        token: String,
    },
    /// Whatever the project holds.
    Held {
        /// The project, by identifier.
        project: String,
    },
}

impl fmt::Debug for Through {
    /// Names what is asked and neither the token nor the state.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arrived { .. } => f
                .debug_struct("Arrived")
                .field("state", &"<redacted>")
                .finish(),
            Self::Token { .. } => f
                .debug_struct("Token")
                .field("token", &"<redacted>")
                .finish(),
            Self::Held { project } => f.debug_struct("Held").field("project", project).finish(),
        }
    }
}

/// What an access reaches, as far as one page of the platform says — or
/// why the platform would not say, or that nothing has come back yet.
///
/// An answer either way rather than a refusal, because the question was
/// asked and answered: a token the platform does not accept is what the
/// form wanted to know, and a refusal would have the browser log an error
/// for a check that did its job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reached", rename_all = "snake_case")]
pub enum Reached {
    /// The repositories, each with its visibility.
    Listed {
        /// The account the access is on, where the platform says one:
        /// an installation's, or the account a token was made under.
        account: Option<String>,
        /// When a token expires, as the wire spells a moment, where the
        /// platform said; none for an installation.
        expires: Option<String>,
        /// The rows, by address.
        repositories: Vec<Reachable>,
        /// Whether the platform had more than were listed.
        more: bool,
    },
    /// The platform would not list, and this is why: a clause for the box
    /// the credential was typed in.
    Unlisted {
        /// What went wrong, as a clause.
        why: String,
    },
    /// The state is this instance's and nothing has come back under it:
    /// the tab is still on the platform. Asked again on the next tick.
    NotYet,
}

impl Default for Reached {
    /// Nothing reached, and nothing wrong: what an access that lists
    /// nothing is answered with.
    fn default() -> Self {
        Self::Listed {
            account: None,
            expires: None,
            repositories: Vec::new(),
            more: false,
        }
    }
}

/// One repository an access reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reachable {
    /// The repository, as a draft names it back.
    pub repository: Repository,
    /// Whether it is private. Under a token, a private repository is one
    /// the token was certainly granted, and a public one may not have
    /// been — see `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    pub private: bool,
}

/// What the projects screen needs in order to draw itself.
///
/// One answer rather than two, because the screen cannot offer to create a
/// project without knowing which agents may be named — and two reads could
/// disagree about that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watching {
    /// What is being watched now.
    pub projects: Vec<Project>,
    /// The agents that could be named. Only the configured ones: naming an
    /// agent without a credential is refused, so offering it would be an
    /// invitation to fail.
    pub available: Vec<Agent>,
    /// What each of those can be set to. Built by the instance from the
    /// domain's closed sets, because the browser's half cannot name them.
    pub shapes: Vec<Shape>,
    /// Where each platform's own form is, filled in, for a project that
    /// does not exist yet.
    pub guides: Guides,
    /// Whether an App is registered on the platform, which is what makes
    /// installing it possible from the form. Nothing more of it: where it
    /// is installed is the instance's business, and a form reaches an
    /// installation through the tab it opened — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    pub app_registered: bool,
    /// Whether a Slack app is registered on this instance, which is what
    /// makes installing it on a workspace possible from the form — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    pub slack_app_registered: bool,
}

/// Where the platforms' own forms are, filled in as this project would
/// have them.
///
/// Links composed on the server from tracked text, and never in the
/// browser — see
/// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Guides {
    /// The form that mints a repository token, named for no project.
    pub token_form: String,
    /// The form that creates the channel's app, with the manifest in it.
    pub app_form: String,
}

/// One agent, set a particular way, as a browser edits it.
///
/// Identifiers throughout. The effort is empty where the model offers none,
/// which is the one shape of absence this type has.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Fitted {
    /// Which agent.
    pub agent: String,
    /// Which of its models.
    pub model: String,
    /// How hard it thinks, or empty where the model has no such choice.
    pub effort: String,
}

impl Fitted {
    /// Whether this names an agent and a model.
    ///
    /// The effort is not required here because the screen cannot know
    /// whether the model takes one without the shape, and the instance
    /// refuses a model given none.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.agent.is_empty() && !self.model.is_empty()
    }
}

/// One kit a project offers, as a browser edits it.
///
/// The description is required: it is the whole of what the foreman chooses
/// by — see `docs/decisions/0048-a-job-runs-on-a-kit.md`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KitDraft {
    /// What to call it, and what a foreman says to choose it.
    pub name: String,
    /// What this project wants it for.
    pub description: String,
    /// The agent, set a particular way.
    pub fitted: Fitted,
}

impl KitDraft {
    /// Whether this says enough to be a kit.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.description.trim().is_empty()
            && self.fitted.is_complete()
    }
}

/// What one agent can be set to, as the choices a form offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    /// The agent, by identifier.
    pub agent: String,
    /// Its models, the default first.
    pub models: Vec<ModelChoice>,
    /// Its effort levels, the default first, for the models that take one.
    pub efforts: Vec<Choice>,
}

/// One model a form may choose, and whether choosing it opens an effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// What the browser sends back.
    pub id: String,
    /// What a person reads.
    pub name: String,
    /// Whether this model takes an effort at all.
    pub has_effort: bool,
}

/// One value a form may choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// What the browser sends back.
    pub id: String,
    /// What a person reads.
    pub name: String,
}

/// The two boxes an app of a project's own is set in, travelling together.
///
/// Both filled binds a channel, and anything less is refused, a rule
/// written in one place, on the instance. Since
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`
/// one of [`BindingDraft`]'s two shapes, checked in the form's panel before
/// the form moves onto it and checked again when the form is saved.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChannelDraft {
    /// What speaks on that channel.
    pub credential: String,
    /// What listens on it.
    pub listen_credential: String,
}

impl fmt::Debug for ChannelDraft {
    /// Names what was given and never the credentials. They are bare
    /// `String`s on the way in from a browser, so nothing under them
    /// redacts, and a derive would print them whole.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelDraft")
            .field("credential", &"<redacted>")
            .field("listen_credential", &"<redacted>")
            .finish()
    }
}

/// How a project talks on Slack, as the form says it: one of two shapes,
/// or what the project already holds — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
///
/// The shape is what the form's Slack card is: a sentence saying which
/// the project is in, with the actions inline, on the pattern the GitHub
/// card set. A workspace is named by the state its install came back
/// under, never by its identifier, so a form can name only what its own
/// tab brought back, as an installation of the App is named.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum BindingDraft {
    /// Whatever the project holds, unchanged. Refused for a project that
    /// does not exist yet, which holds nothing.
    #[default]
    Kept,
    /// An app of the project's own, both tokens pasted.
    Own(ChannelDraft),
    /// A workspace of the app the instance owns.
    Workspace {
        /// The state the install came back under, or none for the
        /// workspace the project holds.
        arrival: Option<String>,
    },
}

impl fmt::Debug for BindingDraft {
    /// Names the shape, and neither the tokens, which redact themselves,
    /// nor the state, which buys a workspace for whoever holds it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kept => f.write_str("Kept"),
            Self::Own(pair) => f.debug_tuple("Own").field(pair).finish(),
            Self::Workspace { arrival } => f
                .debug_struct("Workspace")
                .field("arrival", &arrival.as_ref().map(|_| "<redacted>"))
                .finish(),
        }
    }
}

impl BindingDraft {
    /// Whether this says how the project talks: something new in either
    /// shape, or what the project holds where there is a project to hold
    /// it.
    #[must_use]
    pub fn is_settled(&self, filling: &Filling) -> bool {
        match self {
            Self::Kept | Self::Workspace { arrival: None } => !filling.creating(),
            Self::Own(pair) => {
                !pair.credential.trim().is_empty() && !pair.listen_credential.trim().is_empty()
            }
            Self::Workspace {
                arrival: Some(state),
            } => !state.trim().is_empty(),
        }
    }
}

/// What the project form is currently for.
///
/// One value rather than two flags, so that *adding and amending at once*
/// is a sentence that cannot be said. Closed is the absence of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filling {
    /// A project that does not exist yet. Everything is required.
    Creating,
    /// One that already does, named by the identifier it is known by. A
    /// blank credential means the one it already has.
    Amending(String),
}

impl Filling {
    /// Whether this describes a project that does not exist yet.
    #[must_use]
    pub const fn creating(&self) -> bool {
        matches!(self, Self::Creating)
    }
}

/// One row of the variables table, as the browser sends it back.
///
/// **An empty value means the one the project already holds**, exactly as
/// the credential box does: no value ever reaches a browser, so the box
/// always starts empty. Removal is the row being absent.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VariableDraft {
    /// What the variable is called in the container.
    pub name: String,
    /// What it is set to, or empty to keep what the project holds.
    pub value: String,
    /// What it is for, in the operator's words, told to the agent — see
    /// `docs/decisions/0075-a-variable-says-what-it-is-for.md`. Blank is
    /// blank: it is shown in full and resubmitted, like the brief.
    #[serde(default)]
    pub note: String,
}

impl fmt::Debug for VariableDraft {
    /// Names it and never says what it is set to.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableDraft")
            .field("name", &self.name)
            .field("note", &self.note)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Everything the project form collects, which is everything a project is.
///
/// A struct rather than one parameter per field, because the form's whole
/// purpose is to be filled in twice — once to create and once to change —
/// and a caller should differ in what it *does* with the answer rather than
/// in how it receives it.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Draft {
    /// What to call it.
    pub name: String,
    /// How its foreman's agent is set.
    pub foreman: Fitted,
    /// The kits its jobs may run on.
    pub kits: Vec<KitDraft>,
    /// How its repository is reached, in one of two shapes, with the
    /// repository inside the shape.
    pub access: AccessDraft,
    /// How it talks on Slack: one of two shapes, or what it holds.
    pub binding: BindingDraft,
    /// What its jobs are given that this project never reads.
    pub variables: Vec<VariableDraft>,
    /// What its foreman is told every turn, in the operator's words. Empty
    /// is none, and says nothing.
    pub brief: String,
}

impl fmt::Debug for Draft {
    /// Names the fields and neither credential: the access redacts its
    /// own, as the channel does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draft")
            .field("name", &self.name)
            .field("foreman", &self.foreman)
            .field("kits", &self.kits)
            .field("access", &self.access)
            .field("binding", &self.binding)
            .field("variables", &self.variables)
            .field("brief", &self.brief)
            .finish()
    }
}

/// One part of the project form a problem can point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The name.
    Name,
    /// The repository.
    Repository,
    /// What the foreman thinks with.
    Foreman,
    /// The kits, as a set.
    Kits,
    /// One kit, counting from nought.
    Kit(usize),
    /// How the repository is reached: the access, in either shape.
    Access,
    /// The Slack binding, and its bot token in particular.
    Channel,
    /// The Slack binding's app-level token, which listens.
    Listening,
    /// The variables, as a set.
    Variables,
    /// One variable, counting from nought.
    Variable(usize),
}

/// Something the form cannot be saved with, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Which part of the form.
    pub part: Part,
    /// What is wrong with it, for a person.
    pub why: String,
}

impl Draft {
    /// Everything that stops this being saved, each pointing at where.
    ///
    /// The same conditions the instance enforces, deliberately, so the form
    /// can say which box before asking rather than after being refused. It is
    /// not a second definition of validity — the instance still checks — but
    /// it is the screen refusing to ask the question badly. The access is
    /// set in either shape, or kept where there is one to keep; the
    /// repository is chosen; creating needs a whole channel binding, and
    /// amending does not, because the channel is not offered at all. A row
    /// of variables needs a value unless the project already holds that
    /// name.
    #[must_use]
    pub fn problems(&self, filling: &Filling, held: &[String]) -> Vec<Problem> {
        let mut found = Vec::new();
        let mut problem = |part: Part, why: &str| {
            found.push(Problem {
                part,
                why: why.to_owned(),
            });
        };
        if self.name.trim().is_empty() {
            problem(Part::Name, "It needs a name.");
        }
        if !self.access.is_reached(filling) {
            problem(
                Part::Access,
                if matches!(self.access, AccessDraft::Token { .. }) {
                    "Set a token."
                } else {
                    "Choose how the repository is reached."
                },
            );
        }
        if self.access.repository().is_none() {
            problem(Part::Repository, "Choose the repository.");
        }
        if !self.foreman.is_complete() {
            problem(Part::Foreman, "The foreman needs an agent and a model.");
        }
        if self.kits.is_empty() {
            problem(Part::Kits, "It needs at least one kit its jobs can run on.");
        }
        for (position, kit) in self.kits.iter().enumerate() {
            if kit.name.trim().is_empty() {
                problem(Part::Kit(position), "A kit needs a name.");
            }
            if kit.description.trim().is_empty() {
                problem(
                    Part::Kit(position),
                    "Say what this kit is for; the foreman chooses by it.",
                );
            }
            if !kit.fitted.is_complete() {
                problem(Part::Kit(position), "A kit needs an agent and a model.");
            }
        }
        if !distinct(&self.kits) {
            problem(
                Part::Kits,
                "Two kits share a name, and a name has to pick one out.",
            );
        }
        if !self.binding.is_settled(filling) {
            problem(
                Part::Channel,
                if matches!(self.binding, BindingDraft::Own(_)) {
                    "It needs both Slack tokens."
                } else {
                    "Choose how it talks on Slack."
                },
            );
        }
        for (position, row) in self.variables.iter().enumerate() {
            if row.name.trim().is_empty() {
                problem(Part::Variable(position), "A variable needs a name.");
            } else if row.value.trim().is_empty() && !held.iter().any(|had| had == row.name.trim())
            {
                problem(Part::Variable(position), "A new variable needs a value.");
            }
        }
        found
    }

    /// Whether this says everything a project needs: nothing is wrong with
    /// it anywhere.
    #[must_use]
    pub fn is_complete(&self, filling: &Filling, held: &[String]) -> bool {
        self.problems(filling, held).is_empty()
    }
}

/// Whether no two kits share a name, once the space around each is gone.
///
/// The domain keys kits on their names and would keep one of two silently,
/// so the screen refuses to submit the pair.
#[must_use]
pub fn distinct(kits: &[KitDraft]) -> bool {
    let names: std::collections::BTreeSet<&str> = kits.iter().map(|kit| kit.name.trim()).collect();
    names.len() == kits.len()
}

// ------------------------------------------------------------------ jobs

/// A title for a job started by hand, when a person gave none: the first
/// few words of the work, which is what a person would read in a sidebar.
///
/// Here rather than on the server alone because the form that starts a job
/// shows it as the placeholder of the title it asks for, so both halves
/// have to agree on what leaving it blank means — see
/// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
#[must_use]
pub fn titled(work: &str) -> String {
    work.split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where a job has got to, as a page sees it.
///
/// **Flat, where the domain's is two levels.** The domain nests because its
/// outer level decides behaviour and nothing here decides any. Nothing here
/// claims work is finished: `Proposed` is the agent's own account of
/// itself, and `Done` is the only verdict, and it is a person's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "standing", rename_all = "snake_case")]
pub enum Standing {
    /// Its agent has been given something and has not stopped.
    Working,
    /// Its agent asked something and stopped.
    Asked,
    /// Its agent believes the work is done and is inviting somebody to look.
    Proposed,
    /// A person stopped it while it was working.
    Paused,
    /// Its agent stopped and said nothing about why. Called *idle* here and
    /// silent in the domain, because a page is showing a job that simply
    /// stopped.
    Idle,
    /// Its turn ended badly, and this is what went wrong.
    Failed {
        /// Prose for a person, not a code to branch on.
        why: String,
    },
    /// Over, and a person judged it produced what was wanted.
    Done,
    /// Over, and a person judged it did not.
    Discarded,
    /// Over, because its container went missing.
    Lost,
}

impl Standing {
    /// What this is called.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Asked => "asked",
            Self::Proposed => "proposed",
            Self::Paused => "paused",
            Self::Idle => "idle",
            Self::Failed { .. } => "failed",
            Self::Done => "done",
            Self::Discarded => "discarded",
            Self::Lost => "lost",
        }
    }

    /// Whether the job is over: nothing left to stop and nothing to reclaim.
    #[must_use]
    pub const fn is_over(&self) -> bool {
        matches!(self, Self::Done | Self::Discarded | Self::Lost)
    }

    /// What a person does about a job here, as one word, where there is
    /// something to do — and nothing for a job that is working or over.
    ///
    /// The verb the first page puts beside a job, per
    /// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`;
    /// the readings it answers are 0052's, and *look* is the honest word for
    /// a job that stopped without saying why.
    #[must_use]
    pub const fn asks(&self) -> Option<&'static str> {
        match self {
            Self::Asked => Some("Answer"),
            Self::Proposed => Some("Review"),
            Self::Failed { .. } => Some("Fix"),
            Self::Paused => Some("Resume"),
            Self::Idle => Some("Look"),
            Self::Working | Self::Done | Self::Discarded | Self::Lost => None,
        }
    }
}

/// What a job runs on, as a chip is drawn from it: the agent by the
/// identifier the wire uses and by name, the model by name, and the effort
/// by its spelling and its name where the model takes one.
///
/// Resolved on the server, unlike a project's kits, which a browser edits
/// and so carries as identifiers with the shapes to read them by: a job's
/// kit is fixed when the job is made and only ever read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kit {
    /// The agent, by the identifier the wire uses.
    pub agent: String,
    /// The agent, as a person reads it.
    pub agent_name: String,
    /// The model, as a person reads it.
    pub model: String,
    /// The effort, as the wire spells it and as a person reads it, where the
    /// model takes one.
    pub effort: Option<(String, String)>,
}

/// One job, as much of it as a page is allowed to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// Its name, which is what its container, the tail of its room's name
    /// and its tunnel's host are named after — see
    /// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
    pub id: String,
    /// What it runs on, as a chip is drawn from it.
    pub kit: Kit,
    /// What its session reported it was set to, in the adapter's own words.
    /// Beside the kit rather than folded into it, because the two were
    /// measured to differ.
    pub reported: Vec<(String, String)>,
    /// Why it was started, in prose.
    pub reason: String,
    /// What its agent was told to do — the whole instruction, because it is
    /// the only record of what the agent was actually asked.
    pub kickoff: String,
    /// When the record was made.
    pub created_at: String,
    /// Where it has got to.
    pub standing: Standing,
    /// When its standing last changed, where the instance kept the moment;
    /// none for a job written before it did, which says only that it waits
    /// — see `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    pub since: Option<String>,
    /// Where to look at whatever it is showing. Always present, and it
    /// promises nothing: the port is published when the container is
    /// created, whether or not the agent ever uses it.
    pub tunnel: String,
    /// The room its conversation happens in, by the platform's identifier,
    /// once one has been made.
    pub room: Option<String>,
    /// The same, as an address a person can open, while the channel is
    /// connected and has said where its workspace is.
    pub room_link: Option<String>,
    /// The pull requests it said it opened, in order, each with its address
    /// where the repository is one — see
    /// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    pub pull_requests: Vec<PullRequest>,
}

/// One pull request a job said it opened: the number, and where it is.
///
/// The address is composed on the server from the repository and the
/// platform when the repository is an address. Whether it is still open is
/// the platform's to know, and nothing here says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// The number, as the platform counts them.
    pub number: u64,
    /// Where it is, composed on the server from the project's repository.
    pub link: String,
}

/// One job's page: the job, the project it is on, and what the page links to.
///
/// See `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
/// A link is present only where it is true: a repository that is an address,
/// a room while the channel has said where its workspace is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobPage {
    /// The project, by identifier.
    pub project: String,
    /// The project, by name.
    pub project_name: String,
    /// Where its jobs work.
    pub repository: Repository,
    /// The same, as an address a browser can open, composed on the server.
    pub repository_link: String,
    /// The job, as a list shows it: its kit, its room and its tunnel are
    /// on it, since a row shows them too.
    pub job: Job,
}

/// One of a project's variables, as a list shows it: its name and what it
/// is for, and never its value — see
/// `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    /// What it is called in the container.
    pub name: String,
    /// What it is for, in the operator's words; empty when nobody said.
    pub note: String,
}

/// One kit a project offers, as much of it as a page needs to offer it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offered {
    /// What the operator called it, and what the browser names it back as.
    pub name: String,
    /// What this project wants it for, in the operator's words.
    pub description: String,
}

/// One project and its jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Working {
    /// What to call the project.
    pub name: String,
    /// Where its jobs work.
    pub repository: Repository,
    /// The same, as an address a browser can open, composed on the server.
    pub repository_link: String,
    /// The kits its jobs may run on. Never empty in a valid instance.
    pub kits: Vec<Offered>,
    /// Its jobs, newest first.
    pub jobs: Vec<Job>,
}

/// How a job ends, as a browser asks for it.
///
/// Two of the three outcomes and never the third: *lost* is what waking
/// writes on finding a container gone, and nothing a person presses should
/// be able to claim it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ending {
    /// It produced what was wanted.
    Done,
    /// It did not, and nothing of it is kept.
    Discarded,
}

// -------------------------------------------------------------- refusals

/// What a request can be refused with, in a shape the browser can match on.
///
/// Every variant is safe to send. That is a rule about what may be added
/// here rather than an observation: a failure carrying something an operator
/// should not read is reported as [`Refusal::Failed`] and logged where the
/// operator can see it, because the browser is the one audience that gets
/// no say in who is looking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Refusal {
    /// Nothing by that name can be run. The set of agents is closed and
    /// compiled in, so this is a stale page or a hand-made request.
    #[error("no agent is called {name}")]
    UnknownAgent {
        /// What was asked for.
        name: String,
    },
    /// An agent was configured with nothing.
    #[error("that agent needs a credential")]
    CredentialMissing,
    /// An agent cannot be forgotten while a project names it.
    #[error("{agent} is still used by {}", projects.join(", "))]
    AgentInUse {
        /// The agent that was to be forgotten.
        agent: String,
        /// What would have broken, by name.
        projects: Vec<String>,
    },
    /// Nothing by that identifier is being watched.
    #[error("no project has the identifier {id}")]
    UnknownProject {
        /// What was asked for.
        id: String,
    },
    /// A field that has to say something says nothing.
    #[error("{field} cannot be empty")]
    Incomplete {
        /// The field, named as the screen names it.
        field: String,
    },
    /// The repository is not an address on the platform.
    #[error("the repository has to be an address on GitHub: {rule}")]
    RepositoryRefused {
        /// Which rule it broke, in words.
        rule: String,
    },
    /// A project would have no kit its jobs could run on.
    #[error("a project needs at least one kit its jobs can run on")]
    KitsMissing,
    /// A project was drafted without a whole channel binding.
    #[error(
        "a project needs a Slack binding: a workspace of the app this instance owns, or a bot \
         token and an app-level token of its own"
    )]
    ChannelIncomplete,
    /// A project names a workspace by a state no install has come back
    /// under: the tab has not come back, or the instance has started since
    /// — see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    #[error(
        "no workspace has come back for this form, or it came back before the instance last \
         started: press Install on a workspace again"
    )]
    WorkspaceArrivalUnknown,
    /// The platform would not have the token, or could not see the
    /// repository with it — see
    /// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
    #[error("the token was not kept: {why}")]
    TokenRefused {
        /// What the platform said, as a clause for the box.
        why: String,
    },
    /// The platform could not be asked about the token, so it was not
    /// kept: not wrong, and not known to be right.
    #[error("the token was not kept, because it could not be checked: {why}")]
    TokenUnchecked {
        /// What went wrong on the way there.
        why: String,
    },
    /// The channel would not have one of the binding's credentials.
    #[error("the {} token was not kept: {why}", which(*.listening))]
    ChannelRefused {
        /// Whether it was the credential that listens, rather than the one
        /// that speaks.
        listening: bool,
        /// What the channel said, as a clause for the box.
        why: String,
    },
    /// The channel could not be asked about one of the binding's
    /// credentials, so it was not kept.
    #[error(
        "the {} token was not kept, because it could not be checked: {why}",
        which(*.listening)
    )]
    ChannelUnchecked {
        /// Whether it was the credential that listens.
        listening: bool,
        /// What went wrong on the way there.
        why: String,
    },
    /// A job was asked for on a project that has no channel bound, which only
    /// a project the last release wrote can lack.
    #[error("{project} has no Slack binding, so a job on it would have nowhere to speak")]
    ChannelMissing {
        /// The project, as the screen names it.
        project: String,
    },
    /// A project names an agent that has no credential.
    #[error("{name} has no credential, so a project cannot name it")]
    AgentNotConfigured {
        /// The agent, as the screen names it.
        name: String,
    },
    /// A project's jobs may not run on that kit.
    #[error("{project} offers no kit called {name}")]
    KitNotOnProject {
        /// The kit, as the operator named it.
        name: String,
        /// The project, as the screen names it.
        project: String,
    },
    /// Two of a project's kits were given the same name.
    #[error("two kits are called {name}; a name has to pick one out")]
    KitNameTaken {
        /// The name given twice.
        name: String,
    },
    /// A kit names a setting the agent does not have.
    #[error("no {field} called {value}")]
    UnknownSetting {
        /// Which setting: the model, or the effort.
        field: String,
        /// What was asked for.
        value: String,
    },
    /// A kit asks for an effort on a model that offers no such choice.
    #[error("{model} has no effort to choose")]
    EffortNotOnModel {
        /// The model, as the screen names it.
        model: String,
    },
    /// A variable was given a name a container could not be given. Says
    /// which row and never what was in it: the mistake this most often
    /// catches is a credential pasted into the name box.
    #[error("variable {position} has a name an environment cannot carry: {rule}")]
    VariableNameRefused {
        /// Which row, counting from one.
        position: usize,
        /// Which rule it broke.
        rule: String,
    },
    /// A variable claims a name stageman delivers itself.
    #[error("{name} is a variable stageman sets itself, so a project cannot")]
    VariableReserved {
        /// The reserved name that was claimed.
        name: String,
    },
    /// Two rows give the same name.
    #[error("variable {position} repeats a name given earlier")]
    VariableRepeated {
        /// Which row, counting from one.
        position: usize,
    },
    /// A variable that does not exist yet was given no value.
    #[error("a new variable needs a value")]
    VariableValueMissing,
    /// A project cannot be forgotten while its jobs are still running.
    #[error("{name} still has {working} job(s) working")]
    ProjectBusy {
        /// The project, as the screen names it.
        name: String,
        /// How many jobs are still going.
        working: usize,
    },
    /// The project is watched and holds no job under that identifier.
    #[error("no job here is {id}")]
    UnknownJob {
        /// What was asked for, as it arrived.
        id: String,
    },
    /// A turn is running in that job, so it cannot be retired yet.
    #[error("that job is still working — stop it before retiring it")]
    JobWorking,
    /// No App is registered on that platform, so there is nothing to
    /// forget.
    #[error("no App is registered on {platform}")]
    AppMissing {
        /// The platform, as the screen names it.
        platform: String,
    },
    /// The App cannot be forgotten while a project reaches its repository
    /// through an installation of it.
    #[error("the App on {platform} is still installed on {}", projects.join(", "))]
    AppInUse {
        /// The platform, as the screen names it.
        platform: String,
        /// The projects that would be left without access, by name.
        projects: Vec<String>,
    },
    /// A project names an installation the App is not installed as — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    #[error("the App is not installed as {id}")]
    NoSuchInstallation {
        /// The identifier the draft named.
        id: u64,
    },
    /// The form named an install by a state nothing has come back under:
    /// the tab is still on the platform, the state was minted before the
    /// instance last started, or it was spent by a save already.
    #[error(
        "no installation has come back for this form, or it came back before the instance last \
         started: press Install the App again"
    )]
    ArrivalUnknown,
    /// The access does not reach the repository chosen: a token was not
    /// granted it, or the installation does not cover it.
    #[error("the access does not reach {repository}: {why}")]
    NotReached {
        /// The repository, as the platform names it.
        repository: Repository,
        /// What the platform said, as a clause.
        why: String,
    },
    /// The platform would not mint from the installation, so it was not
    /// kept.
    #[error("the installation was not kept: {why}")]
    InstallationRefused {
        /// What the platform said, as a clause for the box.
        why: String,
    },
    /// The platform could not be asked about the installation, so it was
    /// not kept: not wrong, and not known to be right.
    #[error("the installation was not kept, because it could not be checked: {why}")]
    InstallationUnchecked {
        /// What went wrong on the way there.
        why: String,
    },
    /// A Slack app was registered without one of its three values — see
    /// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    #[error("the app needs its client ID, its client secret and an app-level token")]
    ChannelAppIncomplete,
    /// No app is registered on the channel.
    #[error("no {channel} app is registered on this instance")]
    ChannelAppMissing {
        /// The channel, as the screen names it.
        channel: String,
    },
    /// The app on a channel cannot be forgotten while a project speaks
    /// through one of its workspaces.
    #[error("the {channel} app is still used by {}", projects.join(", "))]
    ChannelAppInUse {
        /// The channel, as the screen names it.
        channel: String,
        /// The projects that would be left without a binding, by name.
        projects: Vec<String>,
    },
    /// The app is not installed on the workspace named.
    #[error("the app is not installed on {id}")]
    NoSuchWorkspace {
        /// The workspace's identifier on the platform.
        id: String,
    },
    /// A workspace cannot be forgotten while a project speaks through it.
    #[error("that workspace is still used by {}", projects.join(", "))]
    WorkspaceInUse {
        /// The projects that would be left without a binding, by name.
        projects: Vec<String>,
    },
    /// An installation cannot be forgotten while a project reaches its
    /// repository through it.
    #[error("that installation is still used by {}", projects.join(", "))]
    InstallationInUse {
        /// The projects that would be left without access, by name.
        projects: Vec<String>,
    },
    /// Something went wrong that the operator cannot act on from here.
    #[error("that did not work — the server log says why")]
    Failed,
}

/// Which of a binding's two credentials, as the form labels them.
const fn which(listening: bool) -> &'static str {
    if listening { "app-level" } else { "bot" }
}

impl Refusal {
    /// The HTTP status this answers with.
    ///
    /// A dashboard barely needs these — it reads the variant, not the number
    /// — but anything else speaking to these routes does, and a route that
    /// answers 200 for a refusal is lying to every client that is not this
    /// page.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Failed => 500,
            Self::UnknownAgent { .. }
            | Self::UnknownProject { .. }
            | Self::UnknownJob { .. }
            | Self::AppMissing { .. }
            | Self::ChannelAppMissing { .. } => 404,
            // Well-formed requests that describe something invalid, which
            // the operator can fix by typing something different.
            Self::CredentialMissing
            | Self::Incomplete { .. }
            | Self::RepositoryRefused { .. }
            | Self::KitsMissing
            | Self::AgentNotConfigured { .. }
            | Self::KitNotOnProject { .. }
            | Self::KitNameTaken { .. }
            | Self::UnknownSetting { .. }
            | Self::EffortNotOnModel { .. }
            | Self::VariableNameRefused { .. }
            | Self::VariableReserved { .. }
            | Self::VariableRepeated { .. }
            | Self::VariableValueMissing
            | Self::ChannelIncomplete
            | Self::WorkspaceArrivalUnknown
            | Self::ChannelAppIncomplete
            | Self::NoSuchWorkspace { .. }
            | Self::TokenRefused { .. }
            | Self::NoSuchInstallation { .. }
            | Self::ArrivalUnknown
            | Self::NotReached { .. }
            | Self::InstallationRefused { .. }
            | Self::ChannelRefused { .. } => 400,
            // The platform behind the credential could not be reached, which
            // is what a bad gateway means: not the request's fault, and not
            // this instance's.
            Self::TokenUnchecked { .. }
            | Self::InstallationUnchecked { .. }
            | Self::ChannelUnchecked { .. } => 502,
            // The request is well formed and the instance is in a state that
            // forbids it, which is what a conflict means.
            Self::AgentInUse { .. }
            | Self::AppInUse { .. }
            | Self::ChannelAppInUse { .. }
            | Self::WorkspaceInUse { .. }
            | Self::InstallationInUse { .. }
            | Self::ProjectBusy { .. }
            | Self::JobWorking
            | Self::ChannelMissing { .. } => 409,
        }
    }

    /// Which part of the project form this points at, where it points at
    /// one, so a page can say it beside the box rather than at the top.
    #[must_use]
    pub fn part(&self) -> Option<Part> {
        match self {
            Self::Incomplete { field } => match field.as_str() {
                "name" => Some(Part::Name),
                "repository" => Some(Part::Repository),
                "access" => Some(Part::Access),
                _ => None,
            },
            Self::RepositoryRefused { .. } | Self::NotReached { .. } => Some(Part::Repository),
            Self::KitsMissing | Self::KitNameTaken { .. } => Some(Part::Kits),
            Self::ChannelIncomplete | Self::WorkspaceArrivalUnknown => Some(Part::Channel),
            Self::TokenRefused { .. }
            | Self::TokenUnchecked { .. }
            | Self::NoSuchInstallation { .. }
            | Self::ArrivalUnknown
            | Self::InstallationRefused { .. }
            | Self::InstallationUnchecked { .. } => Some(Part::Access),
            Self::ChannelRefused { listening, .. } | Self::ChannelUnchecked { listening, .. } => {
                Some(if *listening {
                    Part::Listening
                } else {
                    Part::Channel
                })
            }
            Self::VariableNameRefused { position, .. } | Self::VariableRepeated { position } => {
                position.checked_sub(1).map(Part::Variable)
            }
            Self::VariableValueMissing | Self::VariableReserved { .. } => Some(Part::Variables),
            Self::UnknownAgent { .. }
            | Self::CredentialMissing
            | Self::AgentInUse { .. }
            | Self::UnknownProject { .. }
            | Self::ChannelMissing { .. }
            | Self::ChannelAppIncomplete
            | Self::ChannelAppMissing { .. }
            | Self::NoSuchWorkspace { .. }
            | Self::AgentNotConfigured { .. }
            | Self::KitNotOnProject { .. }
            | Self::UnknownSetting { .. }
            | Self::EffortNotOnModel { .. }
            | Self::ProjectBusy { .. }
            | Self::UnknownJob { .. }
            | Self::JobWorking
            | Self::AppMissing { .. }
            | Self::AppInUse { .. }
            | Self::ChannelAppInUse { .. }
            | Self::WorkspaceInUse { .. }
            | Self::InstallationInUse { .. }
            | Self::Failed => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccessDraft, BindingDraft, ChannelDraft, Draft, Filling, Fitted, InstallLink, KitDraft,
        Reached, Refusal, Repository, Standing, Through, VariableDraft, distinct, titled,
    };

    /// A binding of the project's own, with its two boxes as given.
    fn own(credential: &str, listening: &str) -> BindingDraft {
        BindingDraft::Own(ChannelDraft {
            credential: credential.to_owned(),
            listen_credential: listening.to_owned(),
        })
    }

    /// A job started by hand with no title is titled by the first words of
    /// its work, and the form shows the same words as the default.
    #[test]
    fn a_job_started_by_hand_is_titled_by_its_first_words() {
        assert_eq!(
            titled("Fix the flaky parser test before the release ships"),
            "Fix the flaky parser test before"
        );
        assert_eq!(titled("  one   thing  "), "one thing");
        assert_eq!(titled(""), "");
    }

    fn as_it_comes() -> Fitted {
        Fitted {
            agent: "claude".to_owned(),
            model: "default".to_owned(),
            effort: "default".to_owned(),
        }
    }

    fn default_kit() -> KitDraft {
        KitDraft {
            name: "Claude".to_owned(),
            description: "General-purpose.".to_owned(),
            fitted: as_it_comes(),
        }
    }

    /// A draft the form would accept, with every box filled including the
    /// two that need not be.
    fn filled() -> Draft {
        Draft {
            name: "aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![default_kit()],
            access: AccessDraft::Token {
                token: Some("ghp-not-a-real-token".to_owned()),
                repository: Some(aviary()),
            },
            binding: own("xoxb-not-a-real-token", "xapp-not-a-real-token"),
            brief: String::new(),
            variables: vec![VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: "sk-test-not-a-real-key".to_owned(),
                note: String::new(),
            }],
        }
    }

    /// The repository every fixture is on.
    fn aviary() -> Repository {
        Repository {
            owner: "owner".to_owned(),
            name: "aviary".to_owned(),
        }
    }

    fn without(change: fn(&mut Draft)) -> Draft {
        let mut draft = filled();
        change(&mut draft);
        draft
    }

    const NOTHING_HELD: &[String] = &[];

    fn amending() -> Filling {
        Filling::Amending("a-project".to_owned())
    }

    #[test]
    fn a_draft_with_every_answer_is_complete() {
        assert!(filled().is_complete(&Filling::Creating, NOTHING_HELD,));
    }

    /// Every field is required, one at a time.
    #[test]
    fn a_draft_missing_any_answer_is_not() {
        assert!(
            !without(|draft| draft.name.clear()).is_complete(&Filling::Creating, NOTHING_HELD,)
        );
        assert!(
            !without(|draft| draft.access = AccessDraft::Token {
                token: Some("ghp-not-a-real-token".to_owned()),
                repository: None,
            })
            .is_complete(&Filling::Creating, NOTHING_HELD)
        );
        assert!(
            !without(|draft| draft.foreman.agent.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD,)
        );
        assert!(
            !without(|draft| draft.kits.clear()).is_complete(&Filling::Creating, NOTHING_HELD,)
        );
        assert!(
            !without(|draft| draft.access = AccessDraft::None)
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
    }

    /// The one field the two callers disagree about: what the project
    /// holds — its token, or its installation — is an answer only when
    /// amending, nothing chosen is never one, and each shape needs its
    /// repository — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    #[test]
    fn the_access_is_an_answer_by_shape_and_its_repository_is_required() {
        let held = without(|draft| {
            draft.access = AccessDraft::Token {
                token: None,
                repository: Some(aviary()),
            };
        });
        assert!(held.is_complete(&amending(), NOTHING_HELD));
        assert!(!held.is_complete(&Filling::Creating, NOTHING_HELD));
        let held_installation = without(|draft| {
            draft.access = AccessDraft::App {
                arrival: None,
                repository: Some(aviary()),
            };
        });
        assert!(held_installation.is_complete(&amending(), NOTHING_HELD));
        assert!(!held_installation.is_complete(&Filling::Creating, NOTHING_HELD));

        let none = without(|draft| draft.access = AccessDraft::None);
        assert!(!none.is_complete(&amending(), NOTHING_HELD));
        assert!(!none.is_complete(&Filling::Creating, NOTHING_HELD));
        let problems = none.problems(&Filling::Creating, NOTHING_HELD);
        assert_eq!(
            problems
                .iter()
                .map(|problem| (problem.part, problem.why.as_str()))
                .collect::<Vec<_>>(),
            [
                (super::Part::Access, "Choose how the repository is reached."),
                (super::Part::Repository, "Choose the repository."),
            ]
        );

        let on_the_app = without(|draft| {
            draft.access = AccessDraft::App {
                arrival: Some("f00d".to_owned()),
                repository: Some(aviary()),
            };
        });
        assert!(on_the_app.is_complete(&amending(), NOTHING_HELD));
        assert!(on_the_app.is_complete(&Filling::Creating, NOTHING_HELD));
        assert_eq!(on_the_app.access.repository(), Some(&aviary()));
        let unchosen = without(|draft| {
            draft.access = AccessDraft::App {
                arrival: Some("f00d".to_owned()),
                repository: None,
            };
        });
        assert_eq!(
            unchosen
                .problems(&Filling::Creating, NOTHING_HELD)
                .iter()
                .map(|problem| problem.part)
                .collect::<Vec<_>>(),
            [super::Part::Repository]
        );
        let blank_state = without(|draft| {
            draft.access = AccessDraft::App {
                arrival: Some("  ".to_owned()),
                repository: Some(aviary()),
            };
        });
        assert!(!blank_state.is_complete(&Filling::Creating, NOTHING_HELD));

        let unset_token = without(|draft| {
            draft.access = AccessDraft::Token {
                token: Some("  ".to_owned()),
                repository: Some(aviary()),
            };
        });
        assert_eq!(
            unset_token
                .problems(&amending(), NOTHING_HELD)
                .first()
                .map(|problem| (problem.part, problem.why.as_str())),
            Some((super::Part::Access, "Set a token."))
        );

        assert!(
            !without(|draft| draft.binding = own("", "xapp-not-a-real-token"))
                .is_complete(&Filling::Creating, NOTHING_HELD),
            "the Slack binding is still required"
        );
    }

    #[test]
    fn amending_still_requires_everything_a_project_is() {
        assert!(!without(|draft| draft.name.clear()).is_complete(&amending(), NOTHING_HELD,));
        assert!(
            !without(|draft| draft.access = AccessDraft::Token {
                token: None,
                repository: None,
            })
            .is_complete(&amending(), NOTHING_HELD)
        );
        assert!(!without(|draft| draft.kits.clear()).is_complete(&amending(), NOTHING_HELD,));
    }

    #[test]
    fn only_creating_is_creating() {
        assert!(Filling::Creating.creating());
        assert!(!amending().creating());
    }

    /// A row for a variable the project does not hold needs a value,
    /// whichever caller this is; one it does hold may be left empty; and
    /// renaming a held one makes it new again.
    #[test]
    fn a_variable_needs_a_value_unless_the_project_holds_its_name() {
        let added = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "DATABASE_URL".to_owned(),
                value: String::new(),
                note: String::new(),
            }];
        });
        assert!(!added.is_complete(&amending(), NOTHING_HELD,));
        assert!(!added.is_complete(&Filling::Creating, NOTHING_HELD,));

        let kept = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: String::new(),
                note: String::new(),
            }];
        });
        assert!(kept.is_complete(&amending(), &["STRIPE_API_KEY".to_owned()],));

        let renamed = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "STRIPE_API_KEY_V2".to_owned(),
                value: String::new(),
                note: String::new(),
            }];
        });
        assert!(!renamed.is_complete(&amending(), &["STRIPE_API_KEY".to_owned()],));
    }

    /// Amending keeps the binding the project holds, and the workspace it
    /// holds, without either being said again; an app of its own set
    /// while amending still needs both boxes.
    #[test]
    fn amending_keeps_the_binding_the_project_holds() {
        assert!(
            without(|draft| draft.binding = BindingDraft::Kept)
                .is_complete(&amending(), NOTHING_HELD,)
        );
        assert!(
            without(|draft| draft.binding = BindingDraft::Workspace { arrival: None })
                .is_complete(&amending(), NOTHING_HELD,)
        );
        assert!(
            !without(|draft| draft.binding = own("xoxb-not-a-real-token", ""))
                .is_complete(&amending(), NOTHING_HELD,)
        );
    }

    /// A project is created with a whole binding, in either shape: an app
    /// of its own with both tokens, or a workspace its tab brought back;
    /// nothing chosen, or what a project that does not exist holds, is the
    /// mistake worth catching on the screen — see
    /// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`
    /// and `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    #[test]
    fn a_binding_is_required_whole_in_either_shape() {
        for unsettled in [
            BindingDraft::Kept,
            own("", ""),
            own("", "xapp-not-a-real-token"),
            own("xoxb-not-a-real-token", " "),
            BindingDraft::Workspace { arrival: None },
            BindingDraft::Workspace {
                arrival: Some(" ".to_owned()),
            },
        ] {
            let mut drafted = filled();
            drafted.binding = unsettled.clone();
            assert!(
                !drafted.is_complete(&Filling::Creating, NOTHING_HELD),
                "{unsettled:?}"
            );
            let said = drafted
                .problems(&Filling::Creating, NOTHING_HELD)
                .into_iter()
                .find(|problem| problem.part == super::Part::Channel)
                .map(|problem| problem.why);
            let expected = if matches!(unsettled, BindingDraft::Own(_)) {
                "It needs both Slack tokens."
            } else {
                "Choose how it talks on Slack."
            };
            assert_eq!(said.as_deref(), Some(expected), "{unsettled:?}");
        }
        assert!(
            without(|draft| draft.binding = BindingDraft::Workspace {
                arrival: Some("f00d".to_owned())
            })
            .is_complete(&Filling::Creating, NOTHING_HELD)
        );
    }

    /// Each thing wrong points at its own box, in the order the form shows
    /// them, and a refusal from the instance points at a box too where it
    /// can.
    #[test]
    fn every_problem_points_at_where_it_is() {
        use super::{Part, Refusal};

        let mut draft = filled();
        draft.name.clear();
        draft.kits.push(KitDraft {
            name: " Claude ".to_owned(),
            description: String::new(),
            fitted: as_it_comes(),
        });
        draft.variables.push(VariableDraft::default());
        draft.binding = own("xoxb-not-a-real-token", "");

        let problems = draft.problems(&Filling::Creating, NOTHING_HELD);
        let parts: Vec<Part> = problems.iter().map(|problem| problem.part).collect();
        assert_eq!(
            parts,
            [
                Part::Name,
                Part::Kit(1),
                Part::Kits,
                Part::Channel,
                Part::Variable(1),
            ],
            "{problems:?}"
        );
        assert!(problems.iter().all(|problem| !problem.why.is_empty()));
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD,));
        assert!(
            filled()
                .problems(&Filling::Creating, NOTHING_HELD,)
                .is_empty()
        );

        assert_eq!(
            Refusal::Incomplete {
                field: "repository".to_owned()
            }
            .part(),
            Some(Part::Repository)
        );

        assert_eq!(
            Refusal::RepositoryRefused {
                rule: "it has to be on github.com".to_owned()
            }
            .part(),
            Some(Part::Repository)
        );
        assert_eq!(
            Refusal::VariableRepeated { position: 2 }.part(),
            Some(Part::Variable(1)),
            "the instance counts from one and the form from nought"
        );
        assert_eq!(Refusal::VariableRepeated { position: 0 }.part(), None);
        assert_eq!(Refusal::JobWorking.part(), None);
        assert_eq!(
            Refusal::RepositoryRefused {
                rule: "it has to be on github.com".to_owned()
            }
            .status(),
            400
        );
    }

    /// A checked access's refusal points at the access, whichever its
    /// shape, and one about the repository at the repository; the two
    /// Slack boxes apart. Each carries the status its kind answers with.
    #[test]
    fn a_refusal_about_the_access_points_at_the_access_or_the_repository() {
        use super::{Part, Refusal};

        assert_eq!(
            Refusal::TokenRefused {
                why: "GitHub does not accept it".to_owned()
            }
            .part(),
            Some(Part::Access)
        );
        assert_eq!(
            Refusal::TokenUnchecked {
                why: "GitHub could not be reached: dns".to_owned()
            }
            .part(),
            Some(Part::Access)
        );
        assert_eq!(
            Refusal::Incomplete {
                field: "access".to_owned()
            }
            .part(),
            Some(Part::Access)
        );
        assert_eq!(
            Refusal::NoSuchInstallation { id: 77 }.part(),
            Some(Part::Access)
        );
        assert_eq!(
            Refusal::InstallationRefused {
                why: "GitHub refused: no".to_owned()
            }
            .part(),
            Some(Part::Access)
        );
        assert_eq!(
            Refusal::InstallationUnchecked {
                why: "GitHub could not be reached: dns".to_owned()
            }
            .part(),
            Some(Part::Access)
        );
        assert_eq!(
            Refusal::NotReached {
                repository: Repository {
                    owner: "example".to_owned(),
                    name: "a".to_owned(),
                },
                why: "the installation does not cover it".to_owned()
            }
            .part(),
            Some(Part::Repository)
        );
        assert_eq!(
            Refusal::InstallationInUse {
                projects: vec!["aviary".to_owned()]
            }
            .part(),
            None
        );
        assert_eq!(
            Refusal::InstallationInUse {
                projects: vec!["aviary".to_owned()]
            }
            .status(),
            409
        );
        assert_eq!(
            Refusal::NotReached {
                repository: Repository {
                    owner: "example".to_owned(),
                    name: "a".to_owned(),
                },
                why: "the installation does not cover it".to_owned()
            }
            .to_string(),
            "the access does not reach example/a: the installation does not cover it"
        );
        assert_eq!(
            Refusal::ChannelRefused {
                listening: false,
                why: "Slack refused it (invalid_auth)".to_owned()
            }
            .part(),
            Some(Part::Channel)
        );
        assert_eq!(
            Refusal::ChannelUnchecked {
                listening: true,
                why: "Slack could not be reached: dns".to_owned()
            }
            .part(),
            Some(Part::Listening)
        );
        assert_eq!(
            Refusal::RepositoryRefused {
                rule: "it has to be on github.com".to_owned()
            }
            .status(),
            400
        );
    }

    #[test]
    fn whitespace_does_not_count_as_an_answer() {
        let mut draft = filled();
        draft.name = "   ".to_owned();
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD,));

        let mut draft = filled();
        draft.access = AccessDraft::Token {
            token: Some("\t ".to_owned()),
            repository: Some(aviary()),
        };
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));

        let mut draft = filled();
        draft.binding = own("xoxb-not-a-real-token", "  ");
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD,));
    }

    /// `docs/conventions.md` §4, for the three credentials a draft holds,
    /// and for the one a listing is asked with.
    #[test]
    fn a_draft_does_not_leak_any_credential_when_formatted() {
        let shown = format!("{:?}", filled());

        assert!(!shown.contains("ghp-not-a-real-token"), "{shown}");
        assert!(
            shown.contains("Token") && shown.contains("<redacted>"),
            "the access is named by its shape, with the token redacted: {shown}"
        );
        let asked = format!(
            "{:?}",
            Through::Token {
                token: "ghp-not-a-real-token".to_owned()
            }
        );
        assert!(!asked.contains("ghp-not-a-real-token"), "{asked}");
        assert!(asked.contains("<redacted>"), "{asked}");
        // A state buys an installation for whoever holds it, so it is kept
        // out of every formatting too: the link that carries it, the
        // question asked by it, and the draft naming it.
        let minted = format!(
            "{:?}",
            InstallLink {
                link: "https://github.com/apps/x/installations/new?state=f00d".to_owned(),
                state: "f00d".to_owned(),
            }
        );
        assert_eq!(minted, "InstallLink { .. }");
        let arrived = format!(
            "{:?}",
            Through::Arrived {
                state: "f00d".to_owned()
            }
        );
        assert!(
            !arrived.contains("f00d") && arrived.contains("Arrived"),
            "{arrived}"
        );
        let naming = format!(
            "{:?}",
            AccessDraft::App {
                arrival: Some("f00d".to_owned()),
                repository: None,
            }
        );
        assert!(
            !naming.contains("f00d") && naming.contains("App"),
            "{naming}"
        );
        assert_eq!(
            Reached::default(),
            Reached::Listed {
                account: None,
                expires: None,
                repositories: Vec::new(),
                more: false,
            }
        );
        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
        assert!(!shown.contains("xapp-not-a-real-token"), "{shown}");
        assert!(!shown.contains("sk-test-not-a-real-key"), "{shown}");
        assert!(
            shown.contains("aviary"),
            "it should still say what it holds"
        );
        assert!(
            shown.contains("STRIPE_API_KEY"),
            "a name is not a credential"
        );
        assert!(
            shown.contains("ChannelDraft") && shown.contains("<redacted>"),
            "the binding is named, with its credentials redacted rather than dropped: {shown}"
        );
    }

    /// A draft on a workspace names its shape and never the state its
    /// install came back under, which buys the workspace for whoever holds
    /// it, as an installation's does.
    #[test]
    fn a_workspace_drafts_state_does_not_render() {
        let mut draft = filled();
        draft.binding = BindingDraft::Workspace {
            arrival: Some("f00df00df00d".to_owned()),
        };
        let shown = format!("{draft:?}");
        assert!(shown.contains("Workspace"), "{shown}");
        assert!(!shown.contains("f00df00df00d"), "{shown}");
        assert_eq!(format!("{:?}", BindingDraft::Kept), "Kept");
    }

    /// A kit row needs a name and a description, and two rows cannot share
    /// a name.
    #[test]
    fn a_kit_row_needs_a_name_and_a_description_and_names_are_distinct() {
        let mut unnamed = filled();
        unnamed.kits.push(KitDraft {
            name: "  ".to_owned(),
            description: "something".to_owned(),
            fitted: as_it_comes(),
        });
        assert!(!unnamed.is_complete(&Filling::Creating, NOTHING_HELD,));

        let mut undescribed = filled();
        undescribed.kits.push(KitDraft {
            name: "deep".to_owned(),
            description: " ".to_owned(),
            fitted: as_it_comes(),
        });
        assert!(!undescribed.is_complete(&Filling::Creating, NOTHING_HELD,));

        let mut twice = filled();
        twice.kits.push(KitDraft {
            name: " Claude ".to_owned(),
            description: "again".to_owned(),
            fitted: as_it_comes(),
        });
        assert!(!twice.is_complete(&Filling::Creating, NOTHING_HELD,));
        assert!(!distinct(&twice.kits));
        assert!(distinct(&filled().kits));
    }

    /// Every standing reads as something, and no two read alike.
    #[test]
    fn no_two_standings_read_alike_and_only_the_over_ones_are_over() {
        let every = [
            Standing::Working,
            Standing::Asked,
            Standing::Proposed,
            Standing::Paused,
            Standing::Idle,
            Standing::Failed {
                why: "it did not work".to_owned(),
            },
            Standing::Done,
            Standing::Discarded,
            Standing::Lost,
        ];
        let labels: std::collections::BTreeSet<&str> = every.iter().map(Standing::label).collect();
        assert_eq!(labels.len(), every.len());
        assert!(labels.iter().all(|label| !label.is_empty()));
        let over: Vec<&str> = every
            .iter()
            .filter(|standing| standing.is_over())
            .map(Standing::label)
            .collect();
        assert_eq!(over, ["done", "discarded", "lost"]);
    }

    /// Exactly the idle standings ask something of a person, and no two ask
    /// the same thing: a verb shared by two readings would be a reading
    /// nobody could act on differently.
    #[test]
    fn only_an_idle_job_asks_something_and_each_asks_its_own() {
        let every = [
            Standing::Working,
            Standing::Asked,
            Standing::Proposed,
            Standing::Paused,
            Standing::Idle,
            Standing::Failed {
                why: "it did not work".to_owned(),
            },
            Standing::Done,
            Standing::Discarded,
            Standing::Lost,
        ];
        let asking: Vec<&str> = every.iter().filter_map(Standing::asks).collect();
        assert_eq!(asking, ["Answer", "Review", "Resume", "Look", "Fix"]);
        for standing in &every {
            assert_eq!(
                standing.asks().is_some(),
                !standing.is_over() && *standing != Standing::Working,
                "{standing:?}"
            );
        }
    }

    /// A refusal an operator can fix must not read as a server fault.
    /// What is used is refused forgetting as a conflict, naming who uses
    /// it, and points at no box: the page it is on has none.
    #[test]
    fn what_is_used_is_refused_forgetting_as_a_conflict() {
        for (refused, said) in [
            (
                Refusal::ChannelAppInUse {
                    channel: "Slack".to_owned(),
                    projects: vec!["aviary".to_owned(), "burrow".to_owned()],
                },
                "the Slack app is still used by aviary, burrow",
            ),
            (
                Refusal::WorkspaceInUse {
                    projects: vec!["aviary".to_owned()],
                },
                "that workspace is still used by aviary",
            ),
            (
                Refusal::InstallationInUse {
                    projects: vec!["aviary".to_owned()],
                },
                "that installation is still used by aviary",
            ),
        ] {
            assert_eq!(refused.status(), 409, "{said}");
            assert_eq!(refused.part(), None, "{said}");
            assert_eq!(refused.to_string(), said);
        }
    }

    #[test]
    fn a_refusal_is_not_reported_as_a_fault() {
        let refused = Refusal::AgentInUse {
            agent: "claude".to_owned(),
            projects: vec!["aviary".to_owned(), "burrow".to_owned()],
        };
        assert_eq!(refused.status(), 409);
        assert_eq!(
            refused.to_string(),
            "claude is still used by aviary, burrow"
        );

        assert_eq!(
            Refusal::ProjectBusy {
                name: "aviary".to_owned(),
                working: 2,
            }
            .status(),
            409
        );
        assert_eq!(
            Refusal::Incomplete {
                field: "repository".to_owned(),
            }
            .status(),
            400
        );
        assert_eq!(
            Refusal::UnknownJob {
                id: "nope".to_owned()
            }
            .status(),
            404
        );
        assert_eq!(Refusal::Failed.status(), 500);
        assert_eq!(
            Refusal::TokenRefused {
                why: "GitHub does not accept it".to_owned()
            }
            .status(),
            400
        );
        assert_eq!(
            Refusal::ChannelUnchecked {
                listening: true,
                why: "Slack could not be reached: dns".to_owned()
            }
            .status(),
            502
        );
    }

    /// An incomplete field points at its own box where the form has one,
    /// and at nothing where it does not, which is said at the top instead.
    #[test]
    fn an_incomplete_field_points_at_its_box() {
        use super::Part;

        let incomplete = |field: &str| {
            Refusal::Incomplete {
                field: field.to_owned(),
            }
            .part()
        };
        assert_eq!(incomplete("name"), Some(Part::Name));
        assert_eq!(incomplete("repository"), Some(Part::Repository));
        assert_eq!(incomplete("access"), Some(Part::Access));
        assert_eq!(incomplete("kit name"), None);
    }

    /// What a box says when a credential was not kept, asserted whole per
    /// `docs/conventions.md` §4: the box, and then the platform's clause.
    #[test]
    fn a_credential_not_kept_says_which_and_why() {
        assert_eq!(
            Refusal::TokenRefused {
                why: "GitHub does not accept it".to_owned()
            }
            .to_string(),
            "the token was not kept: GitHub does not accept it"
        );
        assert_eq!(
            Refusal::TokenUnchecked {
                why: "GitHub could not be reached: dns error".to_owned()
            }
            .to_string(),
            "the token was not kept, because it could not be checked: GitHub could not be \
             reached: dns error"
        );
        assert_eq!(
            Refusal::ChannelRefused {
                listening: false,
                why: "Slack refused it (invalid_auth)".to_owned()
            }
            .to_string(),
            "the bot token was not kept: Slack refused it (invalid_auth)"
        );
        assert_eq!(
            Refusal::ChannelUnchecked {
                listening: true,
                why: "Slack could not be reached: dns error".to_owned()
            }
            .to_string(),
            "the app-level token was not kept, because it could not be checked: Slack could \
             not be reached: dns error"
        );
    }

    #[test]
    fn a_project_with_nothing_running_is_idle() {
        let project = super::Project {
            id: "an-identifier".to_owned(),
            name: "aviary".to_owned(),
            repository: aviary(),
            repository_link: "https://github.com/owner/aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![default_kit()],
            access: None,
            binding: None,
            variables: Vec::new(),
            brief: String::new(),
            watched: Vec::new(),
            foreman_room: None,
            foreman_room_link: None,
            attending: false,
            working: 0,
            jobs: 3,
            token_form: String::new(),
        };
        assert!(project.idle());
        assert!(
            !super::Project {
                working: 1,
                ..project
            }
            .idle()
        );
    }
}
