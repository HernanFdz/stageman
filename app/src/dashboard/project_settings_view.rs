//! A project's settings, and a new project, which is the same page with
//! nothing filled in.
//!
//! A page rather than a panel over one, per
//! `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`: the
//! form has as many fields as a page, it has sections, and it has an address
//! to come back to. Both pages read what the projects screen reads, because
//! creating needs the agents that may be named and the shape of each, and
//! amending needs those and the project as it is.
//!
//! **Validity is asked, not restated.** What makes an instance valid is
//! `State::check` and nothing else; this page builds the state it would
//! produce, asks, and reports the answer beside the box it concerns where it
//! can. What it does first is refuse to ask badly: the wire knows which boxes
//! are empty, and the page says so beside each once a save has been tried.
//!
//! Every closed set here is a row of options rather than a dropdown, and
//! every longer text a box that grows, per `docs/conventions.md` §3. The
//! one long set — the repositories an access reaches — is a box that
//! filters with its list in the page, per
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`,
//! which is also why the GitHub card is the enum it is: the access first,
//! in one of two shapes, and the repository chosen from what it reaches.

use dioxus::prelude::*;

use super::agents_view::Agent;
use super::error::DashboardError;
use super::instance_view::install_link;
use super::instance_view::workspace_link;
use super::live::Live;
use super::projects_view::{amend, binds, create, forget, projects, reaches, workspace_arrival};
use crate::ui::{
    BESIDE, Button, ButtonVariant, Card, Combobox, ComboboxItem, FIELD, Field, Guide, Icon, Modal,
    PageHeader, Segmented, Skeleton, TextArea, Tooltip, When,
};

pub use stageman_wire::{
    AccessDraft, AccessView, BindingDraft, BindingView, ChannelDraft, Draft, Filling, Fitted,
    KitDraft, Part, Problem, Reachable, Reached, Repository, Shape, Through, VariableDraft,
    Watching, WorkspaceArrival,
};

/// The project a page is for, where it is for one that exists.
///
/// One lookup for everything the page reads off the project — its draft,
/// what it holds, its name, its token form — so that the comparison it
/// turns on is tested once rather than inverted in the component where
/// nothing could notice.
fn watched<'a>(watching: &'a Watching, filling: &Filling) -> Option<&'a stageman_wire::Project> {
    match filling {
        Filling::Amending(id) => watching.projects.iter().find(|project| &project.id == id),
        Filling::Creating => None,
    }
}

/// What to say beside a part of the form: the first thing wrong with it, or
/// the instance's refusal where that points at the part, or nothing.
fn beside(
    problems: &[Problem],
    refused: Option<&str>,
    pointed: Option<Part>,
    part: Part,
) -> Option<String> {
    problems
        .iter()
        .find(|problem| problem.part == part)
        .map(|problem| problem.why.clone())
        .or_else(|| {
            if pointed == Some(part) {
                refused.map(str::to_owned)
            } else {
                None
            }
        })
}

/// Which shape the access is in — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AccessShape {
    /// Nothing chosen yet.
    #[default]
    None,
    /// Through the App.
    App,
    /// With a token, set or held.
    Token,
}

/// What can be done about the access, from the sentence that says where
/// it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Open the platform to install the App; the form moves onto it when
    /// the tab comes back.
    Install,
    /// Open the panel a token is set in, for the first or in place of one.
    UseToken,
    /// Put the form back on the token it remembers: the project's, or
    /// one set here before.
    BackToToken,
    /// Put the form back on the installation it remembers: the project's,
    /// or one its tab brought back before.
    BackToApp,
}

/// One piece of the sentence: words, words that do something, or a
/// moment drawn once the page is awake.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// Words.
    Text(String),
    /// Words that do something when pressed.
    Act(Action, String),
    /// A moment, as the wire spells it, read as how far off it is.
    When(String),
}

/// What the form holds for the App shape, live or remembered: which
/// installation, and what it reaches once listed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppSlot {
    /// The state the install came back under; none for the installation
    /// the project holds.
    arrival: Option<String>,
    /// The account it is on.
    account: String,
    /// What it reaches, once the instance has listed it.
    listing: Option<Result<Reached, String>>,
}

/// What the form holds for the token shape, live or remembered: which
/// token, and what it reaches once listed.
#[derive(Clone, PartialEq, Eq)]
struct TokenSlot {
    /// The token set in this form; none for the one the project holds.
    token: Option<String>,
    /// Whose it is, where the platform has said — see
    /// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    owner: Option<String>,
    /// When the platform stops accepting it, as the wire spells a moment,
    /// where the platform has said.
    expires: Option<String>,
    /// Whether that moment has passed, as the instance last read it. Never
    /// for a token set here, which the platform just accepted.
    expired: bool,
    /// What it can read, once the instance has listed it.
    listing: Option<Result<Reached, String>>,
}

impl std::fmt::Debug for TokenSlot {
    /// Names the listing and never the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSlot")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("owner", &self.owner)
            .field("expires", &self.expires)
            .field("expired", &self.expired)
            .field("listing", &self.listing)
            .finish()
    }
}

/// The GitHub card as the form holds it: which shape it is in, what it
/// holds for each shape — the current one live, the other remembered, so
/// that going back finds what was there — the repository, and whether
/// the repository was carried in from another access and is not yet
/// settled against what this one reaches. What the wire is sent is
/// derived from it by [`Access::draft`], so a form cannot send what it
/// does not hold — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Access {
    /// The shape the form is in.
    shape: AccessShape,
    /// The App shape's slot, where the form has been on it.
    app: Option<AppSlot>,
    /// The token shape's slot, where the form has been on it.
    token: Option<TokenSlot>,
    /// The repository, once one is chosen.
    repository: Option<Repository>,
    /// Whether the repository came from another access than the one the
    /// form is on now, and waits to be settled against its listing.
    carried: bool,
}

impl Access {
    /// What the form starts on: the shape the project holds, with its
    /// repository, and nothing for a project that does not exist yet.
    fn starting(project: Option<&stageman_wire::Project>) -> Self {
        let Some(project) = project else {
            return Self::default();
        };
        let repository = Some(project.repository.clone());
        match &project.access {
            Some(AccessView::Installation { account }) => Self {
                shape: AccessShape::App,
                app: Some(AppSlot {
                    arrival: None,
                    account: account.clone(),
                    listing: None,
                }),
                repository,
                ..Self::default()
            },
            Some(AccessView::Token {
                owner,
                expires,
                expired,
            }) => Self {
                shape: AccessShape::Token,
                token: Some(TokenSlot {
                    token: None,
                    owner: owner.clone(),
                    expires: expires.clone(),
                    expired: *expired,
                    listing: None,
                }),
                repository,
                ..Self::default()
            },
            None => Self::default(),
        }
    }

    /// What the wire is sent: the shape, what names its access, and the
    /// repository.
    fn draft(&self) -> AccessDraft {
        match self.shape {
            AccessShape::None => AccessDraft::None,
            AccessShape::App => AccessDraft::App {
                arrival: self.app.as_ref().and_then(|slot| slot.arrival.clone()),
                repository: self.repository.clone(),
            },
            AccessShape::Token => AccessDraft::Token {
                token: self.token.as_ref().and_then(|slot| slot.token.clone()),
                repository: self.repository.clone(),
            },
        }
    }

    /// The current shape's listing, where the shape has one and it has
    /// come.
    fn listing(&self) -> Option<&Result<Reached, String>> {
        match self.shape {
            AccessShape::None => None,
            AccessShape::App => self.app.as_ref().and_then(|slot| slot.listing.as_ref()),
            AccessShape::Token => self.token.as_ref().and_then(|slot| slot.listing.as_ref()),
        }
    }

    /// The rows the current shape reaches, once listed.
    fn rows(&self) -> &[Reachable] {
        match self.listing() {
            Some(Ok(Reached::Listed { repositories, .. })) => repositories,
            _ => &[],
        }
    }

    /// Whether the current shape's listing is still being asked for.
    fn busy(&self) -> bool {
        self.shape != AccessShape::None && self.listing().is_none()
    }

    /// Why the current shape could not be listed, where it could not.
    fn unlisted(&self) -> Option<String> {
        match self.listing() {
            Some(Ok(Reached::Unlisted { why }) | Err(why)) => {
                Some(format!("Could not list what it reaches: {why}"))
            }
            Some(Ok(Reached::Listed { .. } | Reached::NotYet)) | None => None,
        }
    }

    /// Whether the platform had more than it listed.
    fn more(&self) -> bool {
        matches!(self.listing(), Some(Ok(Reached::Listed { more: true, .. })))
    }

    /// The account the current access is on, where the platform names one.
    fn account(&self) -> Option<&str> {
        match self.shape {
            AccessShape::App => self
                .app
                .as_ref()
                .map(|slot| slot.account.as_str())
                .filter(|account| !account.is_empty()),
            AccessShape::None | AccessShape::Token => None,
        }
    }

    /// The form moved onto a shape, or onto another access in the same
    /// shape: the repository is carried until the listing settles it.
    const fn moved(&mut self, shape: AccessShape) {
        self.shape = shape;
        self.carried = true;
    }

    /// Settles the repository against what the current access reaches,
    /// once listed, and says what to show over the field's line.
    ///
    /// A repository carried in from another access is kept where this one
    /// reaches it and otherwise dropped and said, never another put in its
    /// place, since a repository the person did not choose is not theirs
    /// to save. One chosen under this access, or the project's own, is
    /// kept and said where the listing no longer reaches it, so an edit
    /// to anything else still saves. Where none is chosen, the one
    /// repository the listing holds fills in where it holds exactly one.
    fn settle(&mut self) -> Option<String> {
        let Some(Ok(Reached::Listed { .. })) = self.listing() else {
            return None;
        };
        let reached =
            |rows: &[Reachable], have: &Repository| rows.iter().any(|row| &row.repository == have);
        let rows = self.rows().to_vec();
        match self.repository.take() {
            Some(have) if reached(&rows, &have) => {
                self.repository = Some(have);
                self.carried = false;
                None
            }
            Some(have) if self.carried => {
                self.carried = false;
                Some(not_reached_words(&have))
            }
            Some(have) => {
                let said = not_reached_words(&have);
                self.repository = Some(have);
                Some(said)
            }
            None => {
                if let [row] = rows.as_slice() {
                    self.repository = Some(row.repository.clone());
                }
                self.carried = false;
                None
            }
        }
    }
}

/// The sentence the Access control is, by where the form stands: which
/// shape it is in, or that none is chosen, and what can be done about it,
/// inline — the same three sentences whether what it holds is the
/// project's or was set in this form, since the whole form is unsaved
/// until Save. Nothing of the instance's other installations: a form is
/// told the account its own access is on and no more.
fn sentence(access: &Access, app_registered: bool) -> Vec<Segment> {
    let text = |words: &str| Segment::Text(words.to_owned());
    let act = |action: Action, words: &str| Segment::Act(action, words.to_owned());
    let mut said = Vec::new();
    match access.shape {
        AccessShape::None => {
            said.push(text("Not chosen yet. "));
            if app_registered {
                said.push(act(Action::Install, "Install the App"));
                said.push(text(" on the repository's account, or "));
                said.push(act(Action::UseToken, "use a token"));
                said.push(text("."));
            } else {
                said.push(act(Action::UseToken, "Use a token"));
                said.push(text(" granted the one repository."));
            }
        }
        AccessShape::App => {
            said.push(text(&access.account().map_or_else(
                || "Through the App. ".to_owned(),
                |account| format!("Through the App, installed on {account}. "),
            )));
            said.push(act(Action::Install, "Install it elsewhere"));
            said.push(text(", or "));
            if access.token.is_some() {
                said.push(act(Action::BackToToken, "go back to the token"));
            } else {
                said.push(act(Action::UseToken, "use a token"));
            }
            said.push(text(" instead."));
        }
        AccessShape::Token => {
            // Whose, and until when, where the platform has said: the two
            // facts kept beside a token, per
            // `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
            let slot = access.token.as_ref();
            let whose = slot.and_then(|slot| slot.owner.as_deref()).map_or_else(
                || "With a token".to_owned(),
                |owner| format!("With {owner}'s token"),
            );
            match slot.and_then(|slot| slot.expires.clone()) {
                Some(at) if slot.is_some_and(|slot| slot.expired) => {
                    said.push(text(&format!("{whose}, which expired ")));
                    said.push(Segment::When(at));
                    said.push(text(". "));
                }
                Some(at) => {
                    said.push(text(&format!("{whose}, expiring ")));
                    said.push(Segment::When(at));
                    said.push(text(". "));
                }
                None => said.push(text(&format!("{whose}. "))),
            }
            said.push(act(Action::UseToken, "Replace it"));
            if app_registered {
                said.push(text(", or "));
                if access.app.is_some() {
                    said.push(act(Action::BackToApp, "go back to the App"));
                } else {
                    said.push(act(Action::Install, "install the App"));
                }
                said.push(text(" instead."));
            } else {
                said.push(text("."));
            }
        }
    }
    said
}

/// What the Repository box says while nothing is chosen or typed, by
/// where the listing has got to.
const fn placeholder(shape: AccessShape, busy: bool, listed: usize) -> &'static str {
    match (shape, busy, listed) {
        (AccessShape::None, _, _) => "choose the access first",
        (_, true, _) => "asking GitHub…",
        (AccessShape::App, false, 0) => "nothing reached yet",
        (AccessShape::Token, false, 0) => "set a token first",
        (_, false, _) => "type to filter",
    }
}

/// What the Repository field says over its line for a repository the
/// listing does not hold.
fn not_reached_words(repository: &Repository) -> String {
    format!("{repository} is not reached this way.")
}

/// The rows a listing gives the box: `owner/name` as what is sent back and
/// read, the owner as the group where the rows span more than one, and
/// the visibility as the mark.
fn items_of(rows: &[Reachable]) -> Vec<ComboboxItem> {
    let owners: std::collections::BTreeSet<&str> = rows
        .iter()
        .map(|row| row.repository.owner.as_str())
        .collect();
    rows.iter()
        .map(|row| ComboboxItem {
            id: row.repository.to_string(),
            label: row.repository.to_string(),
            group: (owners.len() > 1).then(|| row.repository.owner.clone()),
            icon: Some(if row.private {
                (Icon::Private, "private".to_owned())
            } else {
                (Icon::Public, "public".to_owned())
            }),
        })
        .collect()
}

/// Changes the card's state, settles the repository against what the
/// access now reaches, hands the draft what the wire is sent, and says
/// what the settling said. Every change to the card goes through here, so
/// the three cannot drift apart.
// Skipped by mutation testing: it writes three signals of the page, which
// only a running page has, and what it composes — `settle` and `draft` —
// is tested on its own. The probe drives it in a real browser.
#[mutants::skip]
fn apply(
    mut access: Signal<Access>,
    mut draft: Signal<Draft>,
    mut not_reached: Signal<Option<String>>,
    change: impl FnOnce(&mut Access),
) {
    let said = {
        let mut current = access.write();
        change(&mut current);
        current.settle()
    };
    draft.write().access = access.peek().draft();
    not_reached.set(said);
}

/// Which shape the Slack card is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BindingShape {
    /// Nothing chosen yet.
    #[default]
    None,
    /// An app of the project's own.
    Own,
    /// A workspace of the app the instance owns.
    Workspace,
}

/// What can be done about the binding, from the sentence that says where
/// it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingAction {
    /// Open Slack to install the instance's app on a workspace; the form
    /// moves onto it when the tab comes back.
    Install,
    /// Open the panel an app of its own is set in.
    UseOwn,
    /// Put the form back on the app of its own it remembers: the
    /// project's, or one set here before.
    BackToOwn,
    /// Put the form back on the workspace it remembers: the project's, or
    /// one its tab brought back before.
    BackToWorkspace,
}

/// One piece of the Slack sentence: words, or words that do something.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Said {
    /// Words.
    Text(String),
    /// Words that do something when pressed.
    Act(BindingAction, String),
}

/// What the form holds for an app of its own: the pair set here, or none
/// for the project's, which never reaches a browser; and where it speaks,
/// once the channel has said.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnSlot {
    /// The two tokens, set in the panel and checked there.
    pair: Option<ChannelDraft>,
    /// Where the app speaks, as an address a person can open.
    url: Option<String>,
}

/// What the form holds for the workspace shape: the state its install
/// came back under, or none for the project's, and the workspace's name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceSlot {
    /// The state the install came back under; none for the workspace the
    /// project holds.
    arrival: Option<String>,
    /// The workspace's name, for the sentence.
    name: String,
}

/// The Slack card, as the form holds it — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`:
/// which shape it is on, and what it remembers of each, so that going
/// back is going back to something.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Binding {
    /// The shape the form is in.
    shape: BindingShape,
    /// The app of its own, where the form has been on one.
    own: Option<OwnSlot>,
    /// The workspace, where the form has been on one.
    workspace: Option<WorkspaceSlot>,
}

impl Binding {
    /// What the form starts on: the shape the project holds, and nothing
    /// for a project that does not exist yet.
    fn starting(project: Option<&stageman_wire::Project>) -> Self {
        match project.and_then(|project| project.binding.as_ref()) {
            Some(BindingView::Own { url }) => Self {
                shape: BindingShape::Own,
                own: Some(OwnSlot {
                    pair: None,
                    url: url.clone(),
                }),
                workspace: None,
            },
            Some(BindingView::Workspace { name, .. }) => Self {
                shape: BindingShape::Workspace,
                own: None,
                workspace: Some(WorkspaceSlot {
                    arrival: None,
                    name: name.clone(),
                }),
            },
            None => Self::default(),
        }
    }

    /// What the wire is sent: the shape, and what names it — the pair set
    /// here, the state a workspace came back under, or what the project
    /// holds.
    fn draft(&self) -> BindingDraft {
        match self.shape {
            BindingShape::None => BindingDraft::Kept,
            BindingShape::Own => self
                .own
                .as_ref()
                .and_then(|slot| slot.pair.clone())
                .map_or(BindingDraft::Kept, BindingDraft::Own),
            BindingShape::Workspace => BindingDraft::Workspace {
                arrival: self
                    .workspace
                    .as_ref()
                    .and_then(|slot| slot.arrival.clone()),
            },
        }
    }

    /// The form moved onto a shape.
    const fn moved(&mut self, shape: BindingShape) {
        self.shape = shape;
    }
}

/// Changes the Slack card and hands the draft what the wire is sent.
// Skipped by mutation testing for the reason `apply` is: it writes two
// signals of the page, which only a running page has, and what it composes
// — `Binding::draft` — is tested on its own. The probe drives it in a real
// browser.
#[mutants::skip]
fn apply_binding(
    mut binding: Signal<Binding>,
    mut draft: Signal<Draft>,
    change: impl FnOnce(&mut Binding),
) {
    change(&mut binding.write());
    draft.write().binding = binding.peek().draft();
}

/// The workspace an address is on, as a person reads it: the address
/// without its scheme and its trailing slash.
fn host_of(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

/// The sentence the Slack card's control is, by where the form stands:
/// which shape the project is in, or that none is chosen, and what can be
/// done about it, inline — on the pattern the GitHub card set.
fn slack_sentence(binding: &Binding, app_registered: bool) -> Vec<Said> {
    let text = |words: &str| Said::Text(words.to_owned());
    let act = |action: BindingAction, words: &str| Said::Act(action, words.to_owned());
    let mut said = Vec::new();
    match binding.shape {
        BindingShape::None => {
            said.push(text("Not chosen yet. "));
            if app_registered {
                said.push(act(BindingAction::Install, "Install the instance's app"));
                said.push(text(" on a workspace, or "));
                said.push(act(BindingAction::UseOwn, "use an app of its own"));
                said.push(text("."));
            } else {
                said.push(act(BindingAction::UseOwn, "Use an app of its own"));
                said.push(text(
                    "; the instance's app can be installed on a workspace once one is \
                     registered on the Instance page.",
                ));
            }
        }
        BindingShape::Workspace => {
            let name = binding
                .workspace
                .as_ref()
                .map(|slot| slot.name.as_str())
                .filter(|name| !name.is_empty());
            said.push(text(&name.map_or_else(
                || "Through the instance's app. ".to_owned(),
                |name| format!("Through the instance's app, on {name}. "),
            )));
            said.push(act(
                BindingAction::Install,
                "Install it on another workspace",
            ));
            said.push(text(", or "));
            if binding.own.is_some() {
                said.push(act(
                    BindingAction::BackToOwn,
                    "go back to the app of its own",
                ));
            } else {
                said.push(act(BindingAction::UseOwn, "use an app of its own"));
            }
            said.push(text(" instead."));
        }
        BindingShape::Own => {
            let host = binding
                .own
                .as_ref()
                .and_then(|slot| slot.url.as_deref())
                .map(host_of)
                .filter(|host| !host.is_empty());
            said.push(text(&host.map_or_else(
                || "Through an app of its own. ".to_owned(),
                |host| format!("Through an app of its own, on {host}. "),
            )));
            said.push(act(BindingAction::UseOwn, "Replace it"));
            if app_registered {
                said.push(text(", or "));
                if binding.workspace.is_some() {
                    said.push(act(
                        BindingAction::BackToWorkspace,
                        "go back to the workspace",
                    ));
                } else {
                    said.push(act(BindingAction::Install, "install the instance's app"));
                    said.push(text(" on a workspace"));
                }
                said.push(text(" instead."));
            } else {
                said.push(text("."));
            }
        }
    }
    said
}

/// Where a refusal of the panel's pair is said: beside the bot token's
/// box, beside the app-level token's, or over both when it points at
/// neither.
fn placed_own(
    refused: Option<&DashboardError>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(why) = refused else {
        return (None, None, None);
    };
    let said = why.to_string();
    match why {
        DashboardError::Refused(refusal) => match refusal.part() {
            Some(Part::Channel) => (Some(said), None, None),
            Some(Part::Listening) => (None, Some(said), None),
            _ => (None, None, Some(said)),
        },
        DashboardError::NoInstance | DashboardError::Failed => (None, None, Some(said)),
    }
}

/// Whether a save stops before asking the instance: the page refuses to
/// ask badly, and says so beside each box instead.
fn refused_before_asking(draft: &Draft, filling: &Filling, held: &[String]) -> bool {
    !draft.is_complete(filling, held)
}

/// An agent as it comes: the shape's first model, and its first effort where
/// that model takes one.
///
/// What a new kit starts on, and what a fitted agent moves to when its agent
/// changes — nothing carries over between agents, because a model is one
/// agent's and not another's.
pub(super) fn seeded(shape: &Shape) -> Fitted {
    let model = shape.models.first();
    Fitted {
        agent: shape.agent.clone(),
        model: model.map(|model| model.id.clone()).unwrap_or_default(),
        effort: model
            .filter(|model| model.has_effort)
            .and_then(|_| shape.efforts.first())
            .map(|effort| effort.id.clone())
            .unwrap_or_default(),
    }
}

/// The shape describing an agent, if the server sent one for it.
///
/// A function rather than a `find` at each of its call sites, so that the
/// comparison it turns on is tested once — mutation testing inverted it inside
/// the component, where nothing could notice.
pub(super) fn shape_for<'a>(shapes: &'a [Shape], agent: &str) -> Option<&'a Shape> {
    shapes.iter().find(|shape| shape.agent == agent)
}

/// Whether a model takes an effort, as its agent's shape says.
///
/// False for a model the shape does not list at all, which the form cannot
/// produce and a request written by hand can: the far side refuses it by name,
/// and offering an effort for it here would be offering a second thing to
/// refuse.
fn takes_effort(shape: &Shape, model: &str) -> bool {
    shape
        .models
        .iter()
        .any(|choice| choice.id == model && choice.has_effort)
}

/// A fitted agent moved to another agent.
///
/// That agent's defaults, or — for an identifier no shape describes, which the
/// form cannot produce — the identifier alone, so that the refusal on the far
/// side names it rather than a substitute.
fn with_agent(shapes: &[Shape], agent: &str) -> Fitted {
    shape_for(shapes, agent).map_or_else(
        || Fitted {
            agent: agent.to_owned(),
            ..Fitted::default()
        },
        seeded,
    )
}

/// A fitted agent moved to another of its models.
///
/// The effort is kept where the new model takes one, cleared where it does
/// not, and given the first where the old model had none — so what the form
/// shows is always something the far side will accept.
fn with_model(fitted: &Fitted, shape: &Shape, model: &str) -> Fitted {
    let effort = if !takes_effort(shape, model) {
        String::new()
    } else if fitted.effort.is_empty() {
        shape
            .efforts
            .first()
            .map(|effort| effort.id.clone())
            .unwrap_or_default()
    } else {
        fitted.effort.clone()
    };
    Fitted {
        agent: fitted.agent.clone(),
        model: model.to_owned(),
        effort,
    }
}

/// The draft a page starts from: what the project holds, or — for one that
/// does not exist yet — the first configured agent as it comes, for the
/// foreman and for one starting kit named and described after the agent, so
/// that a project with nothing particular to say can be saved as it opens.
///
/// The credential and the channel cannot be seeded: neither ever reaches a
/// browser. A variable is seeded as its name with an empty value, which the
/// wire reads as *keep*.
fn starting(watching: &Watching, filling: &Filling) -> Draft {
    match filling {
        Filling::Creating => {
            let first = watching.shapes.first().map(|shape| {
                let fitted = seeded(shape);
                let agent = watching
                    .available
                    .iter()
                    .find(|agent| agent.id == shape.agent);
                KitDraft {
                    name: agent.map(|agent| agent.name.clone()).unwrap_or_default(),
                    description: agent
                        .map(|agent| agent.description.clone())
                        .unwrap_or_default(),
                    fitted,
                }
            });
            Draft {
                foreman: first
                    .as_ref()
                    .map(|kit| kit.fitted.clone())
                    .unwrap_or_default(),
                kits: first.into_iter().collect(),
                ..Draft::default()
            }
        }
        Filling::Amending(_) => watched(watching, filling)
            .map(|project| Draft {
                name: project.name.clone(),
                foreman: project.foreman.clone(),
                kits: project.kits.clone(),
                access: Access::starting(Some(project)).draft(),
                binding: Binding::starting(Some(project)).draft(),
                variables: project
                    .variables
                    .iter()
                    .map(|variable| VariableDraft {
                        name: variable.name.clone(),
                        value: String::new(),
                        note: variable.note.clone(),
                    })
                    .collect(),
                brief: project.brief.clone(),
            })
            .unwrap_or_default(),
    }
}

/// The page for a project that does not exist yet.
#[component]
pub fn ProjectNewView() -> Element {
    let live = use_context::<Live>();
    let reading = use_server_future(move || {
        let _ = live.follow();
        projects()
    })?;

    rsx! {
        match reading.cloned() {
            Some(Ok(watching)) => rsx! { Editing { watching, filling: Filling::Creating } },
            Some(Err(reason)) => rsx! {
                Card { title: "The projects could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            None => rsx! { Skeleton {} },
        }
    }
}

/// The page for a project that exists.
#[component]
pub fn ProjectSettingsView(project: String) -> Element {
    let live = use_context::<Live>();
    let reading = use_server_future(use_reactive!(|project| {
        let _ = live.follow();
        let _ = project;
        projects()
    }))?;

    rsx! {
        match reading.cloned() {
            Some(Ok(watching)) => {
                if watching.projects.iter().any(|known| known.id == project) {
                    rsx! { Editing { watching, filling: Filling::Amending(project) } }
                } else {
                    rsx! {
                        Card { title: "No such project",
                            p { class: "text-sm text-muted-foreground",
                                "Nothing is watched under this identifier. It may have been forgotten."
                            }
                        }
                    }
                }
            }
            Some(Err(reason)) => rsx! {
                Card { title: "The project could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            None => rsx! { Skeleton {} },
        }
    }
}

/// The form, in sections, with what commits it at the top.
///
/// **Controlled by this page**: the draft is one signal and every section
/// writes into it, so that the control which saves can read the whole. A
/// problem is shown beside its box once a save has been tried, and not
/// before: a form that opens red is a form that scolds before anybody has
/// typed. What the instance refuses is shown beside its box too, where the
/// refusal says which, and at the top where it does not.
#[component]
fn Editing(watching: Watching, filling: Filling) -> Element {
    let seed = starting(&watching, &filling);
    // What the access starts as, which is what the project holds: a save
    // that changed it is checked against GitHub, and says so.
    let seed_access = seed.access.clone();
    let mut draft = use_signal(move || seed);
    let mut tried = use_signal(|| false);
    let mut refused = use_signal(|| None::<DashboardError>);
    let mut forgetting = use_signal(|| false);
    // A file being pasted into the variables: the box, its text, and what
    // could not be read from it — see
    // `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
    let mut pasting = use_signal(|| false);
    let mut pasted = use_signal(String::new);
    let mut unread = use_signal(|| None::<String>);
    // Whether a save is out, so the control says what it waits for and
    // cannot be pressed twice: a credential is checked against its platform
    // before it is kept, which takes a moment — see
    // `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
    let mut saving = use_signal(|| false);
    let creating = filling.creating();

    // What the project holds now, which decides whether an empty value box
    // means *keep*. Empty while creating, which is the true answer: a
    // project that does not exist yet holds nothing.
    let project = watched(&watching, &filling);
    // Whether an App is registered, which is what makes installing it
    // possible; nothing more of it reaches a form.
    let app_registered = watching.app_registered;
    let project_id = project.map(|project| project.id.clone());
    // The same, for the panel that checks a pair for this project.
    let bound_for = project_id.clone();
    // Whether a Slack app is registered, which is what makes installing it
    // on a workspace possible.
    let slack_app_registered = watching.slack_app_registered;
    // The Slack card, as the form holds it — see `Binding`.
    let starting_binding = Binding::starting(project);
    let binding = use_signal(move || starting_binding);
    // What the Binding field says over its line: an install that could not
    // begin, or a tab that came back to nothing.
    let mut binding_problem = use_signal(|| None::<String>);
    let holds_something = project.is_some_and(|project| project.access.is_some());
    // The GitHub card, as the form holds it — see `Access`. Every change
    // goes through `apply`, which settles the repository and hands the
    // draft what the wire is sent.
    let starting_access = Access::starting(project);
    let access = use_signal(move || starting_access);
    // What the Repository field says over its line, when the listing does
    // not reach the repository the person had.
    let not_reached = use_signal(|| None::<String>);
    // What the Access field says over its line: an install that could not
    // begin, or a tab that came back to nothing.
    let mut access_problem = use_signal(|| None::<String>);
    // What the project holds is listed once, for the form to choose from.
    use_future(move || {
        let project_id = project_id.clone();
        async move {
            if let Some(project) = project_id
                && holds_something
            {
                let listed = reaches(Through::Held { project })
                    .await
                    .map_err(|why| why.to_string());
                apply(access, draft, not_reached, |access| match access.shape {
                    AccessShape::App => {
                        if let Some(slot) = &mut access.app {
                            slot.listing = Some(listed);
                        }
                    }
                    AccessShape::Token => {
                        if let Some(slot) = &mut access.token {
                            slot.listing = Some(listed);
                        }
                    }
                    AccessShape::None => {}
                });
            }
        }
    });
    // The state an install was pressed under, while its tab is out. Asked
    // about on every tick, since the tick is how the form learns the tab
    // came back; the form moves onto the App when it has — see
    // `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    let live = use_context::<Live>();
    let mut pending = use_signal(|| None::<String>);
    let _arriving = use_resource(move || {
        let _ = live.follow();
        let state = pending();
        async move {
            let Some(state) = state else {
                return;
            };
            match reaches(Through::Arrived {
                state: state.clone(),
            })
            .await
            {
                Ok(Reached::NotYet) => {}
                Ok(listed @ Reached::Listed { .. }) => {
                    pending.set(None);
                    access_problem.set(None);
                    let account = match &listed {
                        Reached::Listed { account, .. } => account.clone().unwrap_or_default(),
                        Reached::Unlisted { .. } | Reached::NotYet => String::new(),
                    };
                    apply(access, draft, not_reached, |access| {
                        access.app = Some(AppSlot {
                            arrival: Some(state),
                            account,
                            listing: Some(Ok(listed)),
                        });
                        access.moved(AccessShape::App);
                    });
                }
                Ok(Reached::Unlisted { why }) => {
                    pending.set(None);
                    access_problem.set(Some(why));
                }
                Err(why) => {
                    pending.set(None);
                    access_problem.set(Some(why.to_string()));
                }
            }
        }
    });
    // The state an install of the instance's app was pressed under, while
    // its tab is out, asked about on every tick as the App's is — see
    // `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    let live_for_slack = use_context::<Live>();
    let mut pending_workspace = use_signal(|| None::<String>);
    let _workspace_arriving = use_resource(move || {
        let _ = live_for_slack.follow();
        let state = pending_workspace();
        async move {
            let Some(state) = state else {
                return;
            };
            match workspace_arrival("slack".to_owned(), state.clone()).await {
                Ok(WorkspaceArrival::NotYet) => {}
                Ok(WorkspaceArrival::Installed { name, .. }) => {
                    pending_workspace.set(None);
                    binding_problem.set(None);
                    apply_binding(binding, draft, |binding| {
                        binding.workspace = Some(WorkspaceSlot {
                            arrival: Some(state),
                            name,
                        });
                        binding.moved(BindingShape::Workspace);
                    });
                }
                Err(why) => {
                    pending_workspace.set(None);
                    binding_problem.set(Some(why.to_string()));
                }
            }
        }
    });
    // The panel an app of its own is set in, and what it holds while it is
    // open.
    let mut own_panel = use_signal(|| false);
    let mut own_credential = use_signal(String::new);
    let mut own_listening = use_signal(String::new);
    let mut own_refused = use_signal(|| None::<DashboardError>);
    let mut checking_own = use_signal(|| false);
    // The panel a token is set in, and what it holds while it is open.
    let mut token_panel = use_signal(|| false);
    let mut token_text = use_signal(String::new);
    let mut token_problem = use_signal(|| None::<String>);
    let mut checking_token = use_signal(|| false);
    let shape = access.read().shape;
    let busy = access.read().busy() || checking_token();
    let rows: Vec<Reachable> = access.read().rows().to_vec();
    let unlisted = access.read().unlisted();
    let more = access.read().more();
    let items = items_of(&rows);
    let held: Vec<String> = project
        .map(|project| {
            project
                .variables
                .iter()
                .map(|variable| variable.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let name = project
        .map(|project| project.name.clone())
        .unwrap_or_default();
    // Where the platforms' own forms are, filled in: the token's is named
    // for this project where it exists, and for none where it does not
    // yet. Composed on the server, like every address a page links.
    let token_form = project.map_or_else(
        || watching.guides.token_form.clone(),
        |project| project.token_form.clone(),
    );
    let app_form = watching.guides.app_form.clone();

    let problems = if tried() {
        draft().problems(&filling, &held)
    } else {
        Vec::new()
    };
    let refusal = refused();
    let pointed = refusal.as_ref().and_then(|reason| match reason {
        DashboardError::Refused(refusal) => refusal.part(),
        DashboardError::NoInstance | DashboardError::Failed => None,
    });
    // What to say beside a part: the first thing wrong with it, or the
    // instance's refusal where that points here.
    let refusal_said = refusal.as_ref().map(ToString::to_string);
    let saying = move |part: Part| -> Option<String> {
        beside(&problems, refusal_said.as_deref(), pointed, part)
    };
    let unplaced = refused().filter(|_| pointed.is_none());

    let shapes = watching.shapes.clone();
    let available = watching.available;
    let starting_kit = shapes.first().map(seeded).unwrap_or_default();
    let back = match &filling {
        Filling::Creating => super::Route::ProjectsView {},
        Filling::Amending(id) => super::Route::ProjectJobsView {
            project: id.clone(),
        },
    };

    let save = {
        let filling = filling.clone();
        let held = held.clone();
        move |_| {
            tried.set(true);
            let asked = draft();
            if refused_before_asking(&asked, &filling, &held) {
                return;
            }
            let filling = filling.clone();
            let navigator = navigator();
            saving.set(true);
            spawn(async move {
                let answered = match &filling {
                    Filling::Creating => create(asked).await,
                    Filling::Amending(project) => amend(project.clone(), asked).await,
                };
                saving.set(false);
                match answered {
                    Ok(_) => {
                        refused.set(None);
                        // Where the project is: its own page once it has
                        // one, and the list when it has just been made,
                        // since nothing names a project uniquely but the
                        // identifier the instance minted.
                        match filling {
                            Filling::Creating => {
                                navigator.push(super::Route::ProjectsView {});
                            }
                            Filling::Amending(project) => {
                                navigator.push(super::Route::ProjectJobsView { project });
                            }
                        }
                    }
                    Err(reason) => refused.set(Some(reason)),
                }
            });
        }
    };

    rsx! {
        div { class: "flex flex-col gap-4",
            // In view while the page scrolls, because what commits the form
            // is here and the form is longer than a screen —
            // `docs/conventions.md` §3.
            PageHeader {
                div { class: "flex items-baseline gap-3",
                    h1 { class: "text-base font-semibold",
                        if creating { "New project" } else { "{name}" }
                    }
                    if !creating {
                        span { class: "text-sm text-muted-foreground", "settings" }
                    }
                    span { class: "ml-auto flex items-center gap-2",
                        Link {
                            to: back,
                            class: ButtonVariant::Secondary.styled(""),
                            "Cancel"
                        }
                        Button {
                            disabled: saving(),
                            onclick: save,
                            // Named for the wait while it lasts: a token or a
                            // binding is being asked about, and that is what
                            // takes the time.
                            // Named for the wait: an access set is checked
                            // against the platform, and so is one kept
                            // under a repository that moved.
                            if saving() {
                                if draft().access != seed_access {
                                    "Checking…"
                                } else {
                                    "Saving…"
                                }
                            } else if creating {
                                "Create"
                            } else {
                                "Save"
                            }
                        }
                    }
                }
            }
            if let Some(reason) = unplaced {
                p { role: "alert", class: "text-sm text-failed", "{reason}" }
            }

            Card { title: "About",
                Field { label: "Name", problem: saying(Part::Name),
                    input {
                        class: FIELD,
                        placeholder: "Closed Loop",
                        value: "{draft().name}",
                        oninput: move |event| draft.with_mut(|draft| draft.name = event.value()),
                    }
                }
            }

            Card {
                title: "Agents",
                note: "One to think with, and the kits its jobs may run on.",
                div { class: "flex flex-col gap-4",
                    Field {
                        label: "Thinks with",
                        note: "The foreman's own agent. A change lands at its next turn.",
                        info: "A change here lands when the foreman next picks up a message, \
                               never in the middle of one. Changing the agent starts its memory \
                               over; changing only the model or the effort keeps it.",
                        problem: saying(Part::Foreman),
                        FittedEditor {
                            fitted: draft().foreman,
                            shapes: shapes.clone(),
                            available: available.clone(),
                            onchange: move |fitted| draft.with_mut(|draft| draft.foreman = fitted),
                        }
                    }
                    Field {
                        label: "Runs jobs on",
                        note: "A kit is an agent set a particular way. The foreman picks one per job by what you say it is for.",
                        info: "Say what each kit is for — a cheap one for small fixes and \
                               questions, a strong one for work that touches many files. The \
                               foreman reads these lines when it chooses, so they are the \
                               most useful thing on this page to write well.",
                        problem: saying(Part::Kits),
                        aside: rsx! {
                            Tooltip { text: "Add a kit",
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    class: BESIDE,
                                    aria_label: "Add a kit",
                                    onclick: move |_| {
                                        let fitted = starting_kit.clone();
                                        draft.with_mut(|draft| {
                                            draft.kits.push(KitDraft {
                                                fitted,
                                                ..KitDraft::default()
                                            });
                                        });
                                    },
                                    {Icon::Add.draw(16)}
                                }
                            }
                        },
                        div { class: "flex flex-col divide-y divide-border",
                            for (position, row) in draft().kits.iter().enumerate() {
                                Kit {
                                    key: "{position}",
                                    position,
                                    row: row.clone(),
                                    shapes: shapes.clone(),
                                    available: available.clone(),
                                    problem: saying(Part::Kit(position)),
                                    onchange: move |changed: KitDraft| {
                                        draft.with_mut(|draft| {
                                            if let Some(row) = draft.kits.get_mut(position) {
                                                *row = changed;
                                            }
                                        });
                                    },
                                    onremove: move |()| {
                                        draft.with_mut(|draft| {
                                            if position < draft.kits.len() {
                                                draft.kits.remove(position);
                                            }
                                        });
                                    },
                                }
                            }
                        }
                    }
                }
            }

            Card { title: "Brief",
                Field {
                    label: "Brief",
                    note: "Standing instructions for the foreman, said to it on every turn.",
                    info: "Said to the foreman every time it is asked anything or a watched room \
                           hears another app, in your words, so this is where policy lives: \
                           which alerts to ignore, what a filed issue deserves, which account its \
                           jobs act as. It is followed by judgement rather than enforced, and every \
                           line costs tokens on every turn. Ask the foreman in a channel to watch \
                           a room; what it watches is listed on the project's row.",
                    TextArea {
                        class: "min-h-24",
                        placeholder: "Ignore alerts below error. A filed issue gets a job. Act as the release account.",
                        value: draft().brief,
                        oninput: move |event: FormEvent| draft.with_mut(|draft| draft.brief = event.value()),
                    }
                }
            }

            // The access first, in one of three shapes, and the repository
            // chosen from what it reaches: the card is the enum, per
            // `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
            // Installing opens the platform in a tab of its own, which
            // closes itself when the platform brings it back, and the form
            // moves onto the App through the tick.
            Card {
                title: "GitHub",
                note: "How its jobs reach the repository, and which one.",
                div { class: "flex flex-col gap-4",
                    Field {
                        label: "Access",
                        note: "How its jobs reach GitHub: through the App this instance owns, or with a token.",
                        info: "A token is pasted, checked, and held for as long as the project is; \
                               the App mints a token per hour for the one repository, and nothing \
                               is pasted. Either way no job carries a credential: each command \
                               fetches it from this instance. Whatever the sentence below says \
                               replaces what the project holds when you save, and never before.",
                        problem: saying(Part::Access).or_else(|| access_problem.read().clone()),
                        // A sentence saying where things stand, with what
                        // can be done about it inline — `docs/conventions.md`
                        // §3.
                        p { class: "text-sm text-foreground",
                            for (position, segment) in sentence(&access.read(), app_registered).into_iter().enumerate() {
                                match segment {
                                    Segment::Text(words) => rsx! { span { key: "{position}", "{words}" } },
                                    Segment::When(at) => rsx! {
                                        When { key: "{position}", at, ahead: true, class: "text-sm text-foreground".to_owned() }
                                    },
                                    Segment::Act(action, words) => rsx! {
                                        button {
                                            key: "{position}",
                                            r#type: "button",
                                            class: "rounded underline decoration-border-strong underline-offset-2 \
                                                    hover:decoration-foreground focus-visible:outline-none \
                                                    focus-visible:ring-2 focus-visible:ring-primary",
                                            onclick: move |_| match action {
                                                Action::Install => {
                                                    // The tab in the press, the address after it:
                                                    // a browser opens a tab for a press and
                                                    // refuses one for what comes later, and the
                                                    // address is minted by the instance.
                                                    super::open_a_tab();
                                                    spawn(async move {
                                                        match install_link("github".to_owned()).await {
                                                            Ok(minted) => {
                                                                super::send_the_tab(&minted.link);
                                                                access_problem.set(None);
                                                                pending.set(Some(minted.state));
                                                            }
                                                            Err(why) => {
                                                                super::close_the_tab();
                                                                access_problem.set(Some(why.to_string()));
                                                            }
                                                        }
                                                    });
                                                }
                                                Action::UseToken => {
                                                    token_problem.set(None);
                                                    token_panel.set(true);
                                                }
                                                Action::BackToToken => {
                                                    apply(access, draft, not_reached, |access| access.moved(AccessShape::Token));
                                                }
                                                Action::BackToApp => {
                                                    apply(access, draft, not_reached, |access| access.moved(AccessShape::App));
                                                }
                                            },
                                            "{words}"
                                        }
                                    },
                                }
                            }
                        }
                    }
                    Field {
                        label: "Repository",
                        note: "One the access reaches, chosen from GitHub's own list.",
                        info: "Through the App, exactly what its installation covers. With a \
                               token, everything GitHub lists for it: a private repository here was \
                               granted to the token, and a public one is listed because anybody can \
                               read it, so a push to one you did not grant fails there. Either way \
                               the choice is checked against GitHub when you save.",
                        problem: saying(Part::Repository).or_else(|| unlisted.clone()).or_else(|| not_reached.read().clone()),
                        Combobox {
                            label: "Repository",
                            items: items.clone(),
                            value: access.read().repository.as_ref().map(ToString::to_string).unwrap_or_default(),
                            placeholder: placeholder(shape, busy, items.len()).to_owned(),
                            disabled: items.is_empty(),
                            busy,
                            onchange: {
                                move |picked: String| {
                                    let Some(row) = rows.iter().find(|row| row.repository.to_string() == picked) else {
                                        return;
                                    };
                                    let repository = row.repository.clone();
                                    apply(access, draft, not_reached, |access| {
                                        access.repository = Some(repository);
                                        access.carried = false;
                                    });
                                }
                            },
                        }
                        if more {
                            p { class: "text-xs text-faint-foreground",
                                "GitHub listed a hundred, and the rest is not shown."
                            }
                        }
                    }
                }
            }

            // The panel a token is set in: one action, which lists what the
            // token can read before the panel closes, so that a token the
            // platform does not accept never reaches the draft. The token
            // lives in the form until the save checks it once more and
            // keeps it.
            // An app of its own is set in a panel of two boxes, checked there
            // before the form moves onto it, and never shown back.
            if own_panel() {
                {
                    let check = Callback::new(move |()| {
                        let pair = ChannelDraft {
                            credential: own_credential().trim().to_owned(),
                            listen_credential: own_listening().trim().to_owned(),
                        };
                        if pair.credential.is_empty() || pair.listen_credential.is_empty() || checking_own() {
                            return;
                        }
                        checking_own.set(true);
                        own_refused.set(None);
                        let project = bound_for.clone();
                        spawn(async move {
                            let answered = binds(project, pair.clone()).await;
                            checking_own.set(false);
                            match answered {
                                Ok(bound) => {
                                    apply_binding(binding, draft, |binding| {
                                        binding.own = Some(OwnSlot {
                                            pair: Some(pair),
                                            url: Some(bound.url),
                                        });
                                        binding.moved(BindingShape::Own);
                                    });
                                    own_credential.set(String::new());
                                    own_listening.set(String::new());
                                    own_panel.set(false);
                                }
                                Err(why) => own_refused.set(Some(why)),
                            }
                        });
                    });
                    let (beside_credential, beside_listening, unplaced_own) = placed_own(own_refused().as_ref());
                    let complete_own = !own_credential().trim().is_empty() && !own_listening().trim().is_empty();
                    rsx! {
                        Modal {
                            title: "Use an app of its own",
                            onclose: move |()| {
                                own_panel.set(false);
                                own_refused.set(None);
                            },
                            actions: rsx! {
                                Button {
                                    disabled: !complete_own,
                                    onclick: move |_| check.call(()),
                                    if checking_own() { "Checking…" } else { "Use it" }
                                }
                            },
                            div { class: "flex flex-col gap-3",
                                if let Some(why) = unplaced_own {
                                    p { role: "alert", class: "text-sm text-failed", "{why}" }
                                }
                                Field {
                                    label: "Bot token",
                                    note: "What speaks. Starts with xoxb; on OAuth & Permissions once the app is installed.",
                                    problem: beside_credential,
                                    aside: rsx! {
                                        Guide {
                                            mark: "slack",
                                            label: "New app",
                                            says: "Opens Slack's form with the app's manifest filled in. Install \
                                                   the app to the workspace and copy its bot token; then generate \
                                                   an app-level token under Basic Information, with the \
                                                   connections:write scope, and copy that.",
                                            link: app_form,
                                        }
                                    },
                                    input {
                                        r#type: "password",
                                        class: "{FIELD} font-mono",
                                        placeholder: "xoxb-…",
                                        value: "{own_credential}",
                                        oninput: move |event| own_credential.set(event.value()),
                                    }
                                }
                                Field {
                                    label: "App-level token",
                                    note: "What listens. Starts with xapp, with the connections:write scope.",
                                    info: "Generated by hand on the app's Basic Information page, under \
                                           App-Level Tokens, with the connections:write scope: the manifest \
                                           cannot mint one, and neither can anything but a person. Both tokens \
                                           are checked against Slack before the form moves onto them, and an \
                                           app another project already speaks through is refused, since Slack \
                                           hands each event to one connection.",
                                    problem: beside_listening,
                                    input {
                                        r#type: "password",
                                        class: "{FIELD} font-mono",
                                        placeholder: "xapp-…",
                                        value: "{own_listening}",
                                        oninput: move |event| own_listening.set(event.value()),
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if token_panel() {
                {
                    // Pressed on the button and on Enter in the box alike.
                    let check = Callback::new(move |()| {
                        let token = token_text().trim().to_owned();
                        if token.is_empty() || checking_token() {
                            return;
                        }
                        checking_token.set(true);
                        token_problem.set(None);
                        spawn(async move {
                            let listed = reaches(Through::Token { token: token.clone() }).await;
                            checking_token.set(false);
                            match listed {
                                Ok(listed @ Reached::Listed { .. }) => {
                                    let (owner, expires) = match &listed {
                                        Reached::Listed { account, expires, .. } => (account.clone(), expires.clone()),
                                        Reached::Unlisted { .. } | Reached::NotYet => (None, None),
                                    };
                                    apply(access, draft, not_reached, |access| {
                                        access.token = Some(TokenSlot {
                                            token: Some(token),
                                            owner,
                                            expires,
                                            expired: false,
                                            listing: Some(Ok(listed)),
                                        });
                                        access.moved(AccessShape::Token);
                                    });
                                    token_text.set(String::new());
                                    token_panel.set(false);
                                }
                                Ok(Reached::Unlisted { why }) => {
                                    token_problem.set(Some(format!("The token was not kept: {why}.")));
                                }
                                Ok(Reached::NotYet) => {
                                    token_problem.set(Some("The token was not checked.".to_owned()));
                                }
                                Err(why) => token_problem.set(Some(why.to_string())),
                            }
                        });
                    });
                    rsx! {
                        Modal {
                            title: "Use a token",
                            onclose: move |()| {
                                token_panel.set(false);
                                token_problem.set(None);
                            },
                            actions: rsx! {
                                // Not disabled while the platform is asked, though it
                                // says so: a control disabled under the pointer drops
                                // its focus, and Escape would then reach nothing. The
                                // guard in the check is what stops a second press.
                                Button {
                                    disabled: token_text().trim().is_empty(),
                                    onclick: move |_| check.call(()),
                                    if checking_token() { "Checking…" } else { "Use it" }
                                }
                            },
                            Field {
                                label: "Token",
                                note: "A fine-grained token, granted the one repository, with contents, issues and pull requests write.",
                                info: "Every job on this project fetches this token from the instance when \
                                       a command needs it, and no container carries it — so a token that \
                                       reaches more than the one repository is still a token every job \
                                       could misuse. Grant it the one repository, with contents, issues and \
                                       pull requests write and nothing else. GitHub is asked what it can \
                                       read before this closes, and asked again about the repository you \
                                       choose when you save.",
                                problem: token_problem(),
                                // The platform's own form, filled in — see
                                // `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
                                aside: rsx! {
                                    Guide {
                                        mark: "github",
                                        label: "New token",
                                        says: "Opens GitHub's form with the name and the permissions filled \
                                               in. Choose the repository there; the form cannot be told which.",
                                        link: token_form,
                                    }
                                },
                                input {
                                    r#type: "password",
                                    class: "{FIELD} font-mono",
                                    placeholder: "github_pat_…",
                                    value: "{token_text}",
                                    // Focused by script once it exists: the autofocus
                                    // attribute is honoured only until the person has
                                    // focused anything, and the control that opened this
                                    // panel took their click.
                                    onmounted: move |event: MountedEvent| {
                                        spawn(async move {
                                            // Nothing to do for a focus that failed: the
                                            // panel's own focus, set the same way, still
                                            // takes the keys.
                                            let _ = event.set_focus(true).await;
                                        });
                                    },
                                    oninput: move |event| token_text.set(event.value()),
                                    onkeydown: move |event: KeyboardEvent| {
                                        if event.key() == Key::Enter {
                                            event.prevent_default();
                                            check.call(());
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }

            // The sentence, in either shape, whether creating or amending: a
            // binding's credentials never reach the browser, so an app of
            // its own is named by where it speaks, and set again only
            // through the panel — see
            // `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
            Card {
                title: "Slack",
                note: "How it talks on Slack: through the app this instance owns, on a workspace, or through an app of its own.",
                Field {
                    label: "Binding",
                    note: "Where its foreman listens and its jobs speak.",
                    info: "A workspace of the instance's app hears every channel the app is \
                           invited to there, and nothing is pasted. An app of its own is set \
                           with its bot token and an app-level token, checked against Slack \
                           in the panel and again when you save, and never shown back. \
                           Whatever the sentence below says replaces what the project holds \
                           when you save, and never before.",
                    problem: saying(Part::Channel)
                        .or_else(|| saying(Part::Listening))
                        .or_else(|| binding_problem.read().clone()),
                    p { class: "text-sm text-foreground",
                        for (position, segment) in slack_sentence(&binding.read(), slack_app_registered).into_iter().enumerate() {
                            match segment {
                                Said::Text(words) => rsx! { span { key: "{position}", "{words}" } },
                                Said::Act(action, words) => rsx! {
                                    button {
                                        key: "{position}",
                                        r#type: "button",
                                        class: "rounded underline decoration-border-strong underline-offset-2 \
                                                hover:decoration-foreground focus-visible:outline-none \
                                                focus-visible:ring-2 focus-visible:ring-primary",
                                        onclick: move |_| match action {
                                            BindingAction::Install => {
                                                // The tab in the press, the address after it,
                                                // as the App's install does.
                                                super::open_a_tab();
                                                spawn(async move {
                                                    match workspace_link("slack".to_owned()).await {
                                                        Ok(minted) => {
                                                            super::send_the_tab(&minted.link);
                                                            binding_problem.set(None);
                                                            pending_workspace.set(Some(minted.state));
                                                        }
                                                        Err(why) => {
                                                            super::close_the_tab();
                                                            binding_problem.set(Some(why.to_string()));
                                                        }
                                                    }
                                                });
                                            }
                                            BindingAction::UseOwn => {
                                                own_refused.set(None);
                                                own_panel.set(true);
                                            }
                                            BindingAction::BackToOwn => {
                                                apply_binding(binding, draft, |binding| binding.moved(BindingShape::Own));
                                            }
                                            BindingAction::BackToWorkspace => {
                                                apply_binding(binding, draft, |binding| binding.moved(BindingShape::Workspace));
                                            }
                                        },
                                        "{words}"
                                    }
                                },
                            }
                        }
                    }
                }
            }

            Card {
                title: "Environment",
                note: "Set in every container this project's jobs run in. stageman never reads one.",
                Field {
                    label: "Variables",
                    note: "Names and values, as an environment carries them, and what each is for. Removing a row takes the variable away.",
                    info: "Told to the agent by name, with your note beside it, so it knows what \
                           is there and what it is for; never read here. Leaving a value empty \
                           keeps the one already stored. Paste a .env file to add many at once.",
                    problem: saying(Part::Variables),
                    aside: rsx! {
                        div { class: "flex items-center gap-2",
                            Tooltip { text: "Paste a .env file",
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    class: BESIDE,
                                    aria_label: "Paste a .env file",
                                    onclick: move |_| {
                                        pasting.set(true);
                                        unread.set(None);
                                    },
                                    {Icon::Paste.draw(16)}
                                }
                            }
                            Tooltip { text: "Add a variable",
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    class: BESIDE,
                                    aria_label: "Add a variable",
                                    onclick: move |_| {
                                        draft.with_mut(|draft| draft.variables.push(VariableDraft::default()));
                                    },
                                    {Icon::Add.draw(16)}
                                }
                            }
                        }
                    },
                    div { class: "flex flex-col gap-2",
                        for (position, row) in draft().variables.iter().enumerate() {
                            Variable {
                                key: "{position}",
                                position,
                                row: row.clone(),
                                kept: held.iter().any(|had| had == row.name.trim()),
                                problem: saying(Part::Variable(position)),
                                onchange: move |changed: VariableDraft| {
                                    draft.with_mut(|draft| {
                                        if let Some(row) = draft.variables.get_mut(position) {
                                            *row = changed;
                                        }
                                    });
                                },
                                onremove: move |()| {
                                    draft.with_mut(|draft| {
                                        if position < draft.variables.len() {
                                            draft.variables.remove(position);
                                        }
                                    });
                                },
                            }
                        }
                    }
                }
            }

            // A file, pasted, in a dialog of its own — as starting a job is:
            // a transaction with one action, taken and gone. Every line it
            // can read becomes a row in the list, and what it cannot read
            // stays in the box, as it was, with the dialog open, so nothing
            // is dropped unseen. The browser only fills rows; what is refused
            // is refused as before, by row, when the form is saved — see
            // `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
            if pasting() {
                Modal {
                    title: "Paste a .env file",
                    onclose: move |()| {
                        pasting.set(false);
                        unread.set(None);
                    },
                    actions: rsx! {
                        Tooltip { text: "Add them as variables",
                            Button {
                                class: "px-2",
                                aria_label: "Add them as variables",
                                disabled: pasted().trim().is_empty(),
                                onclick: move |_| {
                                    let read = super::env_file::read(&pasted());
                                    if read.variables.is_empty() {
                                        unread.set(Some("No NAME=value line in it.".to_owned()));
                                        return;
                                    }
                                    draft.with_mut(|draft| draft.variables.extend(read.variables));
                                    if read.left.is_empty() {
                                        pasted.set(String::new());
                                        unread.set(None);
                                        pasting.set(false);
                                    } else {
                                        unread.set(Some(format!(
                                            "{} line(s) that are not NAME=value stay here.",
                                            read.left.len()
                                        )));
                                        pasted.set(read.left.join("\n"));
                                    }
                                },
                                {Icon::Save.draw(16)}
                            }
                        }
                    },
                    Field {
                        label: "The file",
                        note: "One NAME=value per line. A comment above a line becomes its note.",
                        problem: unread(),
                        TextArea {
                            class: "min-h-48 font-mono",
                            placeholder: "# the payment provider, in test mode\nSTRIPE_API_KEY=sk_test_not_a_real_key\nexport DATABASE_URL=\"postgres://…\"",
                            value: pasted(),
                            oninput: move |event: FormEvent| pasted.set(event.value()),
                        }
                    }
                }
            }

            if let Filling::Amending(id) = filling {
                Card {
                    title: "Forget this project",
                    note: "Stops watching the repository and removes every job's container. The repository itself is untouched.",
                    Button {
                        variant: ButtonVariant::Danger,
                        onclick: move |_| forgetting.set(true),
                        "Forget…"
                    }
                }
                if forgetting() {
                    Modal {
                        title: "Forget {name}?",
                        onclose: move |()| forgetting.set(false),
                        actions: rsx! {
                            Button {
                                variant: ButtonVariant::Danger,
                                onclick: move |_| {
                                    {
                                        let id = id.clone();
                                        let navigator = navigator();
                                        spawn(async move {
                                            match forget(id).await {
                                                Ok(_) => {
                                                    navigator.push(super::Route::ProjectsView {});
                                                }
                                                Err(reason) => {
                                                    forgetting.set(false);
                                                    refused.set(Some(reason));
                                                }
                                            }
                                        });
                                    }
                                },
                                "Forget"
                            }
                        },
                        p { class: "text-sm text-muted-foreground",
                            "Every job's container and session go with it. A job still working \
                             stops this, so stop it first."
                        }
                    }
                }
            }
        }
    }
}

/// One kit's row: its name, what it is for, and how its agent is set.
#[component]
fn Kit(
    position: usize,
    row: KitDraft,
    shapes: Vec<Shape>,
    available: Vec<Agent>,
    problem: Option<String>,
    onchange: EventHandler<KitDraft>,
    onremove: EventHandler<()>,
) -> Element {
    rsx! {
        // A row parted from the next by a hairline rather than a box within
        // the box, so that its remove control stands on the same edge as
        // every other control in the section — `docs/conventions.md` §3.
        div { class: "flex flex-col gap-2 py-3 first:pt-0 last:pb-0",
            div { class: "flex items-center gap-2",
                input {
                    class: FIELD,
                    placeholder: "a name, e.g. quick",
                    aria_label: "Name of kit {position + 1}",
                    value: "{row.name}",
                    // Moved rather than cloned: the last thing on the row
                    // that wants it, in the order the macro evaluates.
                    oninput: move |event| onchange.call(KitDraft { name: event.value(), ..row.clone() }),
                }
                // Removing the row is how a kit is taken away.
                Tooltip { text: "Remove",
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: BESIDE,
                        aria_label: "Remove kit {position + 1}",
                        onclick: move |_| onremove.call(()),
                        {Icon::Remove.draw(16)}
                    }
                }
            }
            TextArea {
                class: "min-h-12",
                placeholder: "what this project wants it for, e.g. small fixes and questions",
                aria_label: "What kit {position + 1} is for",
                value: row.description.clone(),
                oninput: {
                    let row = row.clone();
                    move |event: FormEvent| onchange.call(KitDraft { description: event.value(), ..row.clone() })
                },
            }
            FittedEditor {
                fitted: row.fitted.clone(),
                shapes,
                available,
                onchange: {
                    let row = row.clone();
                    move |fitted| onchange.call(KitDraft { fitted, ..row.clone() })
                },
            }
            if let Some(problem) = problem {
                p { role: "alert", class: "text-xs text-failed", "{problem}" }
            }
        }
    }
}

/// One variable's row: its name, its value, what it is for, and the way to
/// take it away.
#[component]
fn Variable(
    position: usize,
    row: VariableDraft,
    kept: bool,
    problem: Option<String>,
    onchange: EventHandler<VariableDraft>,
    onremove: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-1",
            div { class: "flex items-center gap-2",
                input {
                    class: "{FIELD} font-mono",
                    placeholder: "STRIPE_API_KEY",
                    aria_label: "Name of variable {position + 1}",
                    value: "{row.name}",
                    oninput: {
                        let row = row.clone();
                        move |event| onchange.call(VariableDraft { name: event.value(), ..row.clone() })
                    },
                }
                input {
                    r#type: "password",
                    class: FIELD,
                    // Per row rather than per form, because *this row* is
                    // what decides it: a box says "keep" only where there
                    // is something to keep, which is a name the project
                    // already holds.
                    placeholder: if kept { "leave empty to keep" } else { "its value" },
                    aria_label: "Value of variable {position + 1}",
                    value: "{row.value}",
                    oninput: {
                        let row = row.clone();
                        move |event| onchange.call(VariableDraft { value: event.value(), ..row.clone() })
                    },
                }
                // What it is for, told to the agent beside the name — see
                // `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
                input {
                    class: FIELD,
                    placeholder: "what it is for",
                    aria_label: "Note of variable {position + 1}",
                    value: "{row.note}",
                    oninput: move |event| {
                        onchange.call(VariableDraft { note: event.value(), ..row.clone() });
                    },
                }
                Tooltip { text: "Remove",
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: BESIDE,
                        aria_label: "Remove variable {position + 1}",
                        onclick: move |_| onremove.call(()),
                        {Icon::Remove.draw(16)}
                    }
                }
            }
            if let Some(problem) = problem {
                p { role: "alert", class: "text-xs text-failed", "{problem}" }
            }
        }
    }
}

/// The three choices that set an agent: which one, which model, and how
/// hard it thinks where the model allows a choice — each a row of options,
/// because each is a handful and a person should see them all.
///
/// **Controlled**, like the form around it: it emits the whole [`Fitted`] on
/// every change rather than writing anywhere, so that the parent decides which
/// row it lands in. Moving to another agent starts from that agent's defaults;
/// moving to another model keeps the effort where the new one takes it and
/// clears it where it does not — see [`with_agent`] and [`with_model`], which
/// are pure so that both rules can be tested without a browser.
///
/// The effort appears only where the model takes one. A choice for a setting
/// the model does not have would be offering something the far side refuses.
#[component]
fn FittedEditor(
    fitted: Fitted,
    shapes: Vec<Shape>,
    available: Vec<Agent>,
    onchange: EventHandler<Fitted>,
) -> Element {
    let shape = shape_for(&shapes, &fitted.agent).cloned();
    let with_effort = shape
        .as_ref()
        .is_some_and(|shape| takes_effort(shape, &fitted.model));

    rsx! {
        div { class: "flex flex-wrap items-center gap-2",
            Segmented {
                label: "Agent",
                options: available.iter().map(|agent| (agent.id.clone(), agent.name.clone())).collect::<Vec<_>>(),
                value: fitted.agent.clone(),
                onchange: move |agent: String| onchange.call(with_agent(&shapes, &agent)),
            }
            if let Some(shape) = shape {
                Segmented {
                    label: "Model",
                    options: shape.models.iter().map(|model| (model.id.clone(), model.name.clone())).collect::<Vec<_>>(),
                    value: fitted.model.clone(),
                    onchange: {
                        let fitted = fitted.clone();
                        let shape = shape.clone();
                        move |model: String| onchange.call(with_model(&fitted, &shape, &model))
                    },
                }
                if with_effort {
                    Segmented {
                        label: "Effort",
                        options: shape.efforts.iter().map(|effort| (effort.id.clone(), effort.name.clone())).collect::<Vec<_>>(),
                        value: fitted.effort.clone(),
                        onchange: move |effort: String| {
                            onchange.call(Fitted {
                                effort,
                                ..fitted.clone()
                            });
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::agents_view::Agent;
    use super::super::error::DashboardError;
    use super::{
        Access, AccessDraft, AccessShape, AccessView, Action, AppSlot, Binding, BindingDraft,
        BindingShape, BindingView, Draft, Filling, Fitted, Icon, KitDraft, OwnSlot, Part, Problem,
        Reachable, Reached, Repository, Said, Segment, Shape, TokenSlot, Watching, WorkspaceSlot,
        beside, host_of, items_of, not_reached_words, placed_own, placeholder,
        refused_before_asking, seeded, sentence, shape_for, slack_sentence, starting, takes_effort,
        watched, with_agent, with_model,
    };
    use stageman_wire::{Choice, ModelChoice};

    /// What a box says: the first problem with its part, else the
    /// instance's refusal when that points at the part, else nothing.
    #[test]
    fn a_box_says_its_problem_before_the_instances_refusal() {
        let problems = vec![Problem {
            part: Part::Name,
            why: "It needs a name.".to_owned(),
        }];
        assert_eq!(
            beside(&problems, Some("refused"), Some(Part::Name), Part::Name),
            Some("It needs a name.".to_owned())
        );
        assert_eq!(
            beside(&[], Some("refused"), Some(Part::Name), Part::Name),
            Some("refused".to_owned())
        );
        assert_eq!(
            beside(&[], Some("refused"), Some(Part::Name), Part::Repository),
            None
        );
        assert_eq!(beside(&[], Some("refused"), None, Part::Name), None);
        assert_eq!(beside(&problems, None, None, Part::Repository), None);
    }

    /// A save stops at the page for a draft with anything wrong, and goes
    /// to the instance for one with nothing wrong.
    #[test]
    fn a_save_stops_at_the_page_for_a_draft_with_a_problem() {
        let blank = Draft::default();
        assert!(refused_before_asking(&blank, &Filling::Creating, &[]));
        let whole = Draft {
            name: "aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![KitDraft {
                name: "Claude".to_owned(),
                description: "does the work".to_owned(),
                fitted: as_it_comes(),
            }],
            access: AccessDraft::Token {
                token: Some("github_pat_not_a_real_token".to_owned()),
                repository: Some(named("owner/aviary")),
            },
            binding: BindingDraft::Own(stageman_wire::ChannelDraft {
                credential: "xoxb-not-a-real-token".to_owned(),
                listen_credential: "xapp-not-a-real-token".to_owned(),
            }),
            variables: Vec::new(),
            brief: String::new(),
        };
        assert!(!refused_before_asking(&whole, &Filling::Creating, &[]));
    }

    /// The Slack sentence, as words.
    fn slack_words(said: &[Said]) -> String {
        said.iter()
            .map(|segment| match segment {
                Said::Text(words) | Said::Act(_, words) => words.clone(),
            })
            .collect()
    }

    /// The Slack card's sentence says which shape the project is in, or
    /// that none is chosen, with what can be done about it inline; and
    /// what can be done depends on whether a Slack app is registered and
    /// on what the form remembers.
    #[test]
    fn the_slack_sentence_says_the_shape_and_what_can_be_done_about_it() {
        let nothing = Binding::default();
        assert_eq!(
            slack_words(&slack_sentence(&nothing, true)),
            "Not chosen yet. Install the instance's app on a workspace, or use an app of its own."
        );
        assert_eq!(
            slack_words(&slack_sentence(&nothing, false)),
            "Not chosen yet. Use an app of its own; the instance's app can be installed on a \
             workspace once one is registered on the Instance page."
        );

        let project = stageman_wire::Project {
            binding: Some(BindingView::Workspace {
                id: "T0TEAM".to_owned(),
                name: "Acme".to_owned(),
            }),
            ..a_project()
        };
        let on_workspace = Binding::starting(Some(&project));
        assert_eq!(
            on_workspace.draft(),
            BindingDraft::Workspace { arrival: None }
        );
        assert_eq!(
            slack_words(&slack_sentence(&on_workspace, true)),
            "Through the instance's app, on Acme. Install it on another workspace, or use an app \
             of its own instead."
        );

        let project = stageman_wire::Project {
            binding: Some(BindingView::Own {
                url: Some("https://acme.slack.com/".to_owned()),
            }),
            ..a_project()
        };
        let mut own = Binding::starting(Some(&project));
        assert_eq!(
            own.draft(),
            BindingDraft::Kept,
            "the project's own app, kept"
        );
        assert_eq!(
            slack_words(&slack_sentence(&own, true)),
            "Through an app of its own, on acme.slack.com. Replace it, or install the instance's \
             app on a workspace instead."
        );
        assert_eq!(
            slack_words(&slack_sentence(&own, false)),
            "Through an app of its own, on acme.slack.com. Replace it."
        );

        // Moved onto a workspace its tab brought back, and back again.
        own.workspace = Some(WorkspaceSlot {
            arrival: Some("f00d".to_owned()),
            name: "Acme".to_owned(),
        });
        own.moved(BindingShape::Workspace);
        assert_eq!(
            own.draft(),
            BindingDraft::Workspace {
                arrival: Some("f00d".to_owned())
            }
        );
        assert_eq!(
            slack_words(&slack_sentence(&own, true)),
            "Through the instance's app, on Acme. Install it on another workspace, or go back to \
             the app of its own instead."
        );
        own.moved(BindingShape::Own);
        assert_eq!(
            slack_words(&slack_sentence(&own, true)),
            "Through an app of its own, on acme.slack.com. Replace it, or go back to the \
             workspace instead."
        );
        // A pair set in the panel is what the wire is sent.
        own.own = Some(OwnSlot {
            pair: Some(stageman_wire::ChannelDraft {
                credential: "xoxb-not-a-real-token".to_owned(),
                listen_credential: "xapp-not-a-real-token".to_owned(),
            }),
            url: None,
        });
        assert!(matches!(own.draft(), BindingDraft::Own(_)));
        assert_eq!(
            slack_words(&slack_sentence(&own, true)),
            "Through an app of its own. Replace it, or go back to the workspace instead."
        );
    }

    /// A refusal of the panel's pair lands beside the box it points at,
    /// and over both otherwise.
    #[test]
    fn a_refusal_of_the_pair_is_placed_beside_its_box() {
        assert_eq!(placed_own(None), (None, None, None));
        let speaking = DashboardError::Refused(stageman_wire::Refusal::ChannelRefused {
            listening: false,
            why: "it is already aviary's app".to_owned(),
        });
        let (credential, listening, unplaced) = placed_own(Some(&speaking));
        assert!(credential.is_some_and(|said| said.contains("already aviary's app")));
        assert_eq!(listening, None);
        assert_eq!(unplaced, None);
        let listening_refused = DashboardError::Refused(stageman_wire::Refusal::ChannelUnchecked {
            listening: true,
            why: "Slack could not be reached: dns".to_owned(),
        });
        let (credential, listening, unplaced) = placed_own(Some(&listening_refused));
        assert_eq!(credential, None);
        assert!(listening.is_some());
        assert_eq!(unplaced, None);
        let (credential, listening, unplaced) = placed_own(Some(&DashboardError::Failed));
        assert_eq!((credential, listening), (None, None));
        assert!(unplaced.is_some());
        assert_eq!(host_of("https://acme.slack.com/"), "acme.slack.com");
        assert_eq!(host_of("http://x.example/"), "x.example");
        assert_eq!(host_of(""), "");
    }

    fn words(said: &[Segment]) -> String {
        said.iter()
            .map(|segment| match segment {
                Segment::Text(words) | Segment::Act(_, words) => words.clone(),
                Segment::When(at) => format!("<{at}>"),
            })
            .collect()
    }

    fn actions(said: &[Segment]) -> Vec<(Action, &str)> {
        said.iter()
            .filter_map(|segment| match segment {
                Segment::Act(action, words) => Some((*action, words.as_str())),
                Segment::Text(_) | Segment::When(_) => None,
            })
            .collect()
    }

    fn row(full_name: &str, private: bool) -> Reachable {
        Reachable {
            repository: named(full_name),
            private,
        }
    }

    fn named(full_name: &str) -> Repository {
        let (owner, name) = full_name.split_once('/').expect("owner/name");
        Repository {
            owner: owner.to_owned(),
            name: name.to_owned(),
        }
    }

    fn listed(rows: &[Reachable]) -> Reached {
        Reached::Listed {
            account: None,
            expires: None,
            repositories: rows.to_vec(),
            more: false,
        }
    }

    fn on_the_app(account: &str, arrival: Option<&str>) -> AppSlot {
        AppSlot {
            arrival: arrival.map(str::to_owned),
            account: account.to_owned(),
            listing: None,
        }
    }

    fn with_a_token(token: Option<&str>) -> TokenSlot {
        TokenSlot {
            token: token.map(str::to_owned),
            owner: None,
            expires: None,
            expired: false,
            listing: None,
        }
    }

    /// The sentence says where the form stands and offers what can be done,
    /// by shape, the same whether what it holds is the project's or was
    /// set here: nothing chosen, against whether an App is registered; the
    /// App, naming its own account and no other; a token; and the way back
    /// to the other shape where the form remembers one. An action's words
    /// are the verb alone, never the comma that leads into it.
    #[test]
    fn the_sentence_says_the_shape_and_offers_its_actions() {
        let said = sentence(&Access::default(), false);
        assert_eq!(
            words(&said),
            "Not chosen yet. Use a token granted the one repository."
        );
        assert_eq!(actions(&said), [(Action::UseToken, "Use a token")]);
        let said = sentence(&Access::default(), true);
        assert_eq!(
            words(&said),
            "Not chosen yet. Install the App on the repository's account, or use a token."
        );
        assert_eq!(
            actions(&said),
            [
                (Action::Install, "Install the App"),
                (Action::UseToken, "use a token")
            ]
        );

        let on_acme = Access {
            shape: AccessShape::App,
            app: Some(on_the_app("acme", Some("f00d"))),
            ..Access::default()
        };
        let said = sentence(&on_acme, true);
        assert_eq!(
            words(&said),
            "Through the App, installed on acme. Install it elsewhere, or use a token instead."
        );
        assert_eq!(
            actions(&said),
            [
                (Action::Install, "Install it elsewhere"),
                (Action::UseToken, "use a token")
            ]
        );
        let remembering_a_token = Access {
            token: Some(with_a_token(None)),
            ..on_acme.clone()
        };
        assert_eq!(
            words(&sentence(&remembering_a_token, true)),
            "Through the App, installed on acme. Install it elsewhere, or go back to the token \
             instead."
        );
        assert_eq!(
            actions(&sentence(&remembering_a_token, true))[1].0,
            Action::BackToToken
        );
        let unnamed = Access {
            app: Some(on_the_app("", None)),
            ..on_acme
        };
        assert_eq!(
            words(&sentence(&unnamed, true)),
            "Through the App. Install it elsewhere, or use a token instead."
        );

        let token = Access {
            shape: AccessShape::Token,
            token: Some(with_a_token(Some("github_pat_not_a_real_token"))),
            ..Access::default()
        };
        let said = sentence(&token, true);
        assert_eq!(
            words(&said),
            "With a token. Replace it, or install the App instead."
        );
        assert_eq!(
            actions(&said),
            [
                (Action::UseToken, "Replace it"),
                (Action::Install, "install the App")
            ]
        );
        assert_eq!(words(&sentence(&token, false)), "With a token. Replace it.");
        let remembering_the_app = Access {
            app: Some(on_the_app("acme", None)),
            ..token
        };
        let said = sentence(&remembering_the_app, true);
        assert_eq!(
            words(&said),
            "With a token. Replace it, or go back to the App instead."
        );
        assert_eq!(actions(&said)[1], (Action::BackToApp, "go back to the App"));
        assert!(
            format!("{remembering_the_app:?}").contains("<redacted>"),
            "a slot names its token to nobody"
        );
    }

    /// The token's sentence says whose it is and when it expires, where the
    /// platform has said either: the moment drawn once the page is awake,
    /// and *which expired* once it has passed — see
    /// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
    #[test]
    fn the_sentence_says_whose_the_token_is_and_when_it_expires() {
        let token = Access {
            shape: AccessShape::Token,
            token: Some(with_a_token(Some("github_pat_not_a_real_token"))),
            ..Access::default()
        };
        let owned = Access {
            token: Some(TokenSlot {
                owner: Some("acme".to_owned()),
                expires: Some("2026-10-24T12:00:00Z".to_owned()),
                ..with_a_token(None)
            }),
            ..token
        };
        assert_eq!(
            words(&sentence(&owned, false)),
            "With acme's token, expiring <2026-10-24T12:00:00Z>. Replace it.",
            "whose, and until when, drawn as a moment"
        );
        let expired = Access {
            token: Some(TokenSlot {
                expired: true,
                ..owned.token.clone().expect("the slot")
            }),
            ..owned.clone()
        };
        assert_eq!(
            words(&sentence(&expired, false)),
            "With acme's token, which expired <2026-10-24T12:00:00Z>. Replace it."
        );
        let owner_only = Access {
            token: Some(TokenSlot {
                expires: None,
                ..owned.token.clone().expect("the slot")
            }),
            ..owned
        };
        assert_eq!(
            words(&sentence(&owner_only, false)),
            "With acme's token. Replace it."
        );
    }

    /// The form starts on what the project holds, with its repository, and
    /// on nothing for a project that does not exist yet; and what the wire
    /// is sent names the access the way the instance resolves it — the
    /// state for an installation come back, none for the one held.
    /// A project on an installation of the App, with no binding: what the
    /// cards' tests start from.
    fn a_project() -> stageman_wire::Project {
        stageman_wire::Project {
            id: "p".to_owned(),
            name: "aviary".to_owned(),
            repository: named("acme/aviary"),
            repository_link: "https://github.com/acme/aviary".to_owned(),
            foreman: as_it_comes(),
            kits: Vec::new(),
            access: Some(AccessView::Installation {
                account: "acme".to_owned(),
            }),
            binding: None,
            variables: Vec::new(),
            brief: String::new(),
            watched: Vec::new(),
            foreman_room: None,
            foreman_room_link: None,
            attending: false,
            working: 0,
            jobs: 0,
            token_form: String::new(),
        }
    }

    #[test]
    fn the_card_starts_on_what_the_project_holds_and_drafts_what_the_wire_needs() {
        let mut project = a_project();
        let starting = Access::starting(Some(&project));
        assert_eq!(starting.shape, AccessShape::App);
        assert_eq!(starting.app, Some(on_the_app("acme", None)));
        assert_eq!(starting.token, None);
        assert_eq!(starting.repository, Some(named("acme/aviary")));
        assert!(!starting.carried);
        assert_eq!(
            starting.draft(),
            AccessDraft::App {
                arrival: None,
                repository: Some(named("acme/aviary")),
            }
        );
        project.access = Some(AccessView::Token {
            owner: Some("acme".to_owned()),
            expires: Some("2026-10-24T12:00:00Z".to_owned()),
            expired: false,
        });
        let starting = Access::starting(Some(&project));
        assert_eq!(starting.shape, AccessShape::Token);
        assert_eq!(
            starting.token,
            Some(TokenSlot {
                owner: Some("acme".to_owned()),
                expires: Some("2026-10-24T12:00:00Z".to_owned()),
                ..with_a_token(None)
            })
        );
        assert_eq!(
            starting.draft(),
            AccessDraft::Token {
                token: None,
                repository: Some(named("acme/aviary")),
            }
        );
        project.access = None;
        assert_eq!(Access::starting(Some(&project)), Access::default());
        assert_eq!(Access::starting(None).draft(), AccessDraft::None);

        let arrived = Access {
            shape: AccessShape::App,
            app: Some(on_the_app("acme", Some("f00d"))),
            token: Some(with_a_token(Some("github_pat_not_a_real_token"))),
            repository: None,
            carried: true,
        };
        assert_eq!(
            arrived.draft(),
            AccessDraft::App {
                arrival: Some("f00d".to_owned()),
                repository: None,
            }
        );
        let back_on_the_token = Access {
            shape: AccessShape::Token,
            ..arrived
        };
        assert_eq!(
            back_on_the_token.draft(),
            AccessDraft::Token {
                token: Some("github_pat_not_a_real_token".to_owned()),
                repository: None,
            }
        );
    }

    /// A listing settles the repository: one carried in from another
    /// access is kept where reached and otherwise dropped and said, never
    /// another put in its place; the project's own, or one chosen here,
    /// is kept and said where the listing no longer reaches it; and where
    /// none is chosen the one row fills in. Nothing settles while the
    /// listing has not come, or could not be made.
    #[test]
    fn a_listing_settles_the_repository_by_where_it_came_from() {
        let a_and_b = [row("acme/a", true), row("acme/b", false)];
        let only_a = [row("acme/a", true)];
        let on_acme =
            |rows: Option<&[Reachable]>, repository: Option<&str>, carried: bool| Access {
                shape: AccessShape::App,
                app: Some(AppSlot {
                    listing: rows.map(|rows| Ok(listed(rows))),
                    ..on_the_app("acme", None)
                }),
                token: None,
                repository: repository.map(named),
                carried,
            };

        let mut still_listing = on_acme(None, Some("acme/c"), true);
        assert_eq!(still_listing.settle(), None);
        assert_eq!(
            still_listing.repository,
            Some(named("acme/c")),
            "nothing settles before the listing"
        );
        assert!(still_listing.carried);

        let mut carried_and_reached = on_acme(Some(&a_and_b), Some("acme/b"), true);
        assert_eq!(carried_and_reached.settle(), None);
        assert_eq!(carried_and_reached.repository, Some(named("acme/b")));
        assert!(!carried_and_reached.carried, "settled");

        let mut carried_and_not = on_acme(Some(&only_a), Some("acme/c"), true);
        assert_eq!(
            carried_and_not.settle(),
            Some("acme/c is not reached this way.".to_owned())
        );
        assert_eq!(
            carried_and_not.repository, None,
            "dropped, and the one row is not put in its place"
        );
        assert!(!carried_and_not.carried);

        let mut held_and_not = on_acme(Some(&only_a), Some("acme/c"), false);
        assert_eq!(
            held_and_not.settle(),
            Some("acme/c is not reached this way.".to_owned())
        );
        assert_eq!(
            held_and_not.repository,
            Some(named("acme/c")),
            "the project's own is kept, and said"
        );

        let mut none_and_one = on_acme(Some(&only_a), None, true);
        assert_eq!(none_and_one.settle(), None);
        assert_eq!(
            none_and_one.repository,
            Some(named("acme/a")),
            "the one row fills in where nothing was chosen"
        );
        let mut none_and_two = on_acme(Some(&a_and_b), None, true);
        assert_eq!(none_and_two.settle(), None);
        assert_eq!(none_and_two.repository, None, "two rows wait for a choice");

        let mut unlisted = on_acme(None, Some("acme/c"), true);
        unlisted.app = Some(AppSlot {
            listing: Some(Ok(Reached::Unlisted {
                why: "GitHub does not accept it".to_owned(),
            })),
            ..on_the_app("acme", None)
        });
        assert_eq!(unlisted.settle(), None);
        assert_eq!(
            unlisted.unlisted().as_deref(),
            Some("Could not list what it reaches: GitHub does not accept it")
        );
        assert!(unlisted.rows().is_empty());
        assert!(!unlisted.busy());
        assert!(!unlisted.more());
        let mut a_page_of = on_acme(Some(&a_and_b), None, false);
        a_page_of.app = Some(AppSlot {
            listing: Some(Ok(Reached::Listed {
                account: None,
                expires: None,
                repositories: a_and_b.to_vec(),
                more: true,
            })),
            ..on_the_app("acme", None)
        });
        assert!(a_page_of.more(), "the platform had more than it listed");
        assert!(on_acme(None, None, false).busy(), "listing not come");
        assert!(!Access::default().busy(), "nothing to list");

        let mut moved = on_acme(Some(&a_and_b), Some("acme/a"), false);
        moved.moved(AccessShape::Token);
        assert_eq!(moved.shape, AccessShape::Token);
        assert!(
            moved.carried,
            "carried until the token's listing settles it"
        );
        assert_eq!(
            not_reached_words(&named("acme/d")),
            "acme/d is not reached this way."
        );
    }

    #[test]
    fn the_box_says_what_it_waits_for() {
        assert_eq!(
            placeholder(AccessShape::None, false, 0),
            "choose the access first"
        );
        assert_eq!(placeholder(AccessShape::App, true, 0), "asking GitHub…");
        assert_eq!(placeholder(AccessShape::Token, true, 3), "asking GitHub…");
        assert_eq!(
            placeholder(AccessShape::App, false, 0),
            "nothing reached yet"
        );
        assert_eq!(
            placeholder(AccessShape::Token, false, 0),
            "set a token first"
        );
        assert_eq!(placeholder(AccessShape::App, false, 2), "type to filter");
        assert_eq!(placeholder(AccessShape::Token, false, 9), "type to filter");
    }

    /// A listing becomes the box's rows: the address sent back, the owner
    /// and name read, the visibility marked, and the owner as the group
    /// only where the rows span more than one.
    #[test]
    fn a_listing_becomes_items() {
        let rows = vec![row("acme/site", true), row("example/pub", false)];
        let items = items_of(&rows);
        assert_eq!(
            items
                .iter()
                .map(|item| (item.id.as_str(), item.label.as_str(), item.group.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("acme/site", "acme/site", Some("acme")),
                ("example/pub", "example/pub", Some("example")),
            ]
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.icon.clone())
                .collect::<Vec<_>>(),
            [
                Some((Icon::Private, "private".to_owned())),
                Some((Icon::Public, "public".to_owned())),
            ]
        );
        let one_owner = items_of(&[row("acme/site", true), row("acme/other", false)]);
        assert!(
            one_owner.iter().all(|item| item.group.is_none()),
            "no group where every row is one owner's"
        );
    }

    /// The agent's defaults, as a browser holds them.
    fn as_it_comes() -> Fitted {
        Fitted {
            agent: "claude".to_owned(),
            model: "default".to_owned(),
            effort: "default".to_owned(),
        }
    }

    /// The shape the server would send for Claude, as the form sees it.
    fn claude() -> Shape {
        let model = |id: &str, has_effort: bool| ModelChoice {
            id: id.to_owned(),
            name: id.to_owned(),
            has_effort,
        };
        let effort = |id: &str| Choice {
            id: id.to_owned(),
            name: id.to_owned(),
        };
        Shape {
            agent: "claude".to_owned(),
            models: vec![
                model("default", true),
                model("sonnet", true),
                model("opus", true),
                model("haiku", false),
            ],
            efforts: vec![effort("default"), effort("low"), effort("high")],
        }
    }

    /// Moving between models keeps, clears or seeds the effort as the new
    /// model demands, so the form never holds a pair the far side refuses.
    #[test]
    fn changing_the_model_keeps_the_effort_only_where_the_new_model_takes_one() {
        let shape = claude();
        let on_opus = with_model(&as_it_comes(), &shape, "opus");
        assert_eq!(on_opus.model, "opus");
        assert_eq!(on_opus.effort, "default", "kept, since opus takes one");

        let on_haiku = with_model(&on_opus, &shape, "haiku");
        assert_eq!(on_haiku.model, "haiku");
        assert_eq!(on_haiku.effort, "", "cleared, since haiku takes none");

        let back = with_model(&on_haiku, &shape, "sonnet");
        assert_eq!(
            back.effort, "default",
            "seeded with the first, since there was none"
        );

        let chosen = Fitted {
            effort: "high".to_owned(),
            ..as_it_comes()
        };
        assert_eq!(
            with_model(&chosen, &shape, "sonnet").effort,
            "high",
            "a chosen effort survives a change of model"
        );
    }

    /// The two comparisons the form turns on, asserted in both directions.
    ///
    /// Mutation testing inverted each of these inside the component and no
    /// test noticed, which is why they are functions now.
    #[test]
    fn a_shape_is_found_by_its_agent_and_says_which_models_take_an_effort() {
        let shapes = vec![claude()];
        assert_eq!(shape_for(&shapes, "claude"), Some(&claude()));
        assert_eq!(shape_for(&shapes, "gpt"), None);
        assert_eq!(shape_for(&[], "claude"), None);

        assert!(takes_effort(&claude(), "opus"));
        assert!(takes_effort(&claude(), "default"));
        assert!(!takes_effort(&claude(), "haiku"), "the one model with none");
        assert!(
            !takes_effort(&claude(), "gpt-5"),
            "a model the shape does not list takes nothing"
        );
    }

    /// Moving to another agent starts from that agent's defaults, and an
    /// agent no shape describes is carried as itself for the far side to
    /// refuse by name.
    #[test]
    fn changing_the_agent_starts_from_its_defaults() {
        let shapes = vec![claude()];
        assert_eq!(with_agent(&shapes, "claude"), as_it_comes());
        assert_eq!(seeded(&claude()), as_it_comes());
        assert_eq!(
            with_agent(&shapes, "gpt"),
            Fitted {
                agent: "gpt".to_owned(),
                ..Fitted::default()
            }
        );

        let mut effortless = claude();
        effortless.models.rotate_left(3);
        assert_eq!(
            seeded(&effortless).effort,
            "",
            "an agent whose first model takes no effort is seeded with none"
        );
    }

    /// A new project starts on the first agent as it comes, with one kit
    /// named after it, so it can be saved as it opens; an existing one
    /// starts as it is, with its credentials blank, because none ever
    /// reaches a browser.
    #[test]
    fn a_page_starts_from_what_is_true() {
        let watching = Watching {
            projects: vec![stageman_wire::Project {
                id: "p".to_owned(),
                name: "aviary".to_owned(),
                repository: named("owner/aviary"),
                repository_link: "https://github.com/owner/aviary".to_owned(),
                foreman: as_it_comes(),
                kits: vec![KitDraft {
                    name: "deep".to_owned(),
                    description: "big work".to_owned(),
                    fitted: as_it_comes(),
                }],
                access: Some(AccessView::Token {
                    owner: None,
                    expires: None,
                    expired: false,
                }),
                binding: Some(BindingView::Own { url: None }),
                variables: vec![stageman_wire::Variable {
                    name: "STRIPE_API_KEY".to_owned(),
                    note: "the payment provider, in test mode".to_owned(),
                }],
                brief: "be careful".to_owned(),
                watched: Vec::new(),
                foreman_room: None,
                foreman_room_link: None,
                attending: false,
                working: 0,
                jobs: 0,
                token_form: String::new(),
            }],
            available: vec![Agent {
                id: "claude".to_owned(),
                name: "Claude".to_owned(),
                description: "does the work".to_owned(),
                configured: true,
                used_by: Vec::new(),
            }],
            shapes: vec![claude()],
            guides: stageman_wire::Guides::default(),
            app_registered: false,
            slack_app_registered: false,
        };

        let fresh = starting(&watching, &Filling::Creating);
        assert_eq!(fresh.foreman, as_it_comes());
        assert_eq!(fresh.kits.len(), 1);
        assert_eq!(
            fresh.kits.first().map(|kit| kit.name.as_str()),
            Some("Claude")
        );
        assert!(fresh.name.is_empty());
        assert_eq!(fresh.access, AccessDraft::None, "nothing chosen yet");

        let existing = starting(&watching, &Filling::Amending("p".to_owned()));
        assert_eq!(existing.name, "aviary");
        assert_eq!(existing.brief, "be careful");
        assert_eq!(
            existing.kits.first().map(|kit| kit.name.as_str()),
            Some("deep")
        );
        assert_eq!(
            existing.access,
            AccessDraft::Token {
                token: None,
                repository: Some(named("owner/aviary")),
            },
            "the token held, left unsaid, on its repository"
        );
        assert_eq!(
            existing
                .variables
                .first()
                .map(|row| (row.name.as_str(), row.value.as_str())),
            Some(("STRIPE_API_KEY", "")),
            "the name, with an empty value that means keep"
        );

        let unknown = starting(&watching, &Filling::Amending("q".to_owned()));
        assert_eq!(unknown, Draft::default());

        assert_eq!(
            watched(&watching, &Filling::Amending("p".to_owned()))
                .map(|project| project.name.as_str()),
            Some("aviary")
        );
        assert!(watched(&watching, &Filling::Amending("q".to_owned())).is_none());
        assert!(watched(&watching, &Filling::Creating).is_none());
    }
}
