//! Where the App is installed, what an access reaches, and the tokens a
//! project's jobs run on — see
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`
//! and
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
//!
//! Three flows. A page asks for an install link and is answered one
//! carrying a state minted for that press and held; the platform's setup
//! redirect brings the browser back with an installation's identifier
//! and the state: the request is held while the installation is fetched
//! with the App's key, which is the whole of the check, the installation
//! is kept beside the App — its account, and whether it covers every
//! repository — and the state is marked as come back under it, before
//! the tab is answered with the page that closes it. A form asks what an
//! access reaches — the installation its own state came back under, a
//! token about to be set, or what a project holds — and is held while the
//! platform lists it: a token minted per installation for listing and
//! held until its hour is nearly up, then that one installation's
//! repositories; or one read of what a token can read. Nothing of a
//! listing is kept, and no form is shown an installation but its own or
//! its project's. And a job's wrapper asking for its credential is
//! answered from a token minted for the project's one repository and held
//! until shortly before its hour is up, so a project mints about once an
//! hour however many commands its jobs run.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stageman_core::{Access, Platform, ProjectId, Timestamp};
use stageman_platform::{Minted, PlatformError};
use stageman_vocabulary::{
    Answer, Arrival, Bytes, Effect as Generic, EffectId, RequestId, Responded,
};
use stageman_wire::{InstallLink, InstallationView, Reachable, Reached, Refusal, Through};

use crate::apps::parameter;
use crate::checks::CHECKED_WITHIN;
use crate::requests::Response;
use crate::views;
use crate::vocabulary::{AppEffect, RequestId as Asking};
use crate::{Effect, Running};

/// How long before a minted token's hour is up it stops being served, so
/// that a command begun on it finishes on it.
const RENEWED_BEFORE_SECONDS: i64 = 300;

/// The status a wrapper is answered with when a token could not be minted:
/// the platform behind the credential, not the request, is what failed.
const BAD_GATEWAY: u16 = 502;

/// What a person sees when the browser came back to the installation's
/// path with nothing naming one.
const NO_INSTALLATION: &str = "Nothing in this address names an installation. Press Install the \
                               App in stageman and let GitHub bring you back.";

/// How many install links minted and not come back are remembered: a
/// press mints one, and only the last few can still be the one a tab
/// comes back with.
const REMEMBERED: usize = 16;

/// The page the tab the platform brings back is answered with, around one
/// of the two sentences below: outside the dashboard, since the instance
/// answers it before the proxy, and as small as the sentence it carries.
const LANDING: (&str, &str) = (
    "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" \
     content=\"width=device-width, initial-scale=1\">\n<meta name=\"color-scheme\" content=\"light \
     dark\">\n<title>stageman</title>\n<style>body{font:15px/1.5 system-ui,sans-serif;margin:3rem \
     auto;max-width:36rem;padding:0 1rem}</style>\n</head>\n<body>\n",
    "</body>\n</html>\n",
);

/// What the tab says when the installation was kept and the page that
/// opened the tab will have it, before it closes itself: a tab opened by
/// script may be closed by script, which is why the install is opened
/// that way — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
const INSTALLED: (&str, &str) = (
    "<p>The App is installed on <b>",
    "</b>. Back in stageman, the page you left has it.</p>\n<p>This tab closes itself; if it \
     stays, close it.</p>\n<script>window.close();</script>\n",
);

/// What the tab says when the installation was kept but came back under a
/// state this instance did not mint, or minted before it last started:
/// the page that opened the tab will not learn of it, so the tab stays
/// open to say what to do.
const INSTALLED_UNANNOUNCED: (&str, &str) = (
    "<p>The App is installed on <b>",
    "</b>, but this tab was opened before stageman last started, so the page you left will not \
     learn of it.</p>\n<p>Close this tab and press Install the App again there: GitHub will \
     bring you straight back.</p>\n",
);

/// What the tab says when the installation was not kept, and stays open
/// to say it.
const NOT_INSTALLED: (&str, &str) = (
    "<p>The App was not installed: ",
    ".</p>\n<p>Close this tab and press Install the App again.</p>\n",
);

/// What the tab the platform brought back is told.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Landing {
    /// Kept, and the page that opened the tab will have it: the tab
    /// closes.
    Announced {
        /// The account the App is installed on.
        account: String,
    },
    /// Kept, and no page of this instance's is waiting on it: the tab
    /// stays, saying what to do.
    Unannounced {
        /// The account the App is installed on.
        account: String,
    },
    /// Not kept, and the tab stays to say why.
    Refused {
        /// Why, as a clause.
        why: String,
    },
}

/// Text as a page can carry it: the three characters that would read as
/// markup, escaped.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The page the tab is answered with: that the App is installed on the
/// account and the tab closes; that it is, but nothing here will say so,
/// and the tab stays; or why it is not, and the tab stays.
fn landing(landed: &Landing) -> String {
    let (opening, closing) = LANDING;
    let said = |around: (&str, &str), it: &str| format!("{}{}{}", around.0, escaped(it), around.1);
    let sentence = match landed {
        Landing::Announced { account } => said(INSTALLED, account),
        Landing::Unannounced { account } => said(INSTALLED_UNANNOUNCED, account),
        Landing::Refused { why } => said(NOT_INSTALLED, why),
    };
    format!("{opening}{sentence}{closing}")
}

/// One setup redirect being answered: the browser's request held, which
/// installation the platform was asked about, and the state the tab came
/// back under, if any.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Installing {
    /// The browser's request, as the world holds it open.
    pub request: RequestId,
    /// The installation, by the identifier the redirect carried.
    pub installation: u64,
    /// The state the redirect carried, which names the page that opened
    /// the tab.
    pub state: Option<String>,
}

/// One install begun from a page: for which platform's App, and which
/// installation came back under its state, once one has.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Install {
    /// Which platform's App.
    pub platform: Platform,
    /// The installation that came back, once one has: kept beside the App
    /// already, and named here so that the page holding the state can
    /// reach it and no other page can.
    pub installation: Option<u64>,
}

/// A form's listing being assembled: the request held, what has been
/// listed so far, and which answers are still awaited.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Reaching {
    /// The account the access is on, where the platform says one: an
    /// installation's, or the one a token was made under.
    pub account: Option<String>,
    /// When a token expires, where the platform said.
    pub expires: Option<Timestamp>,
    /// What has been listed so far.
    pub repositories: Vec<Reachable>,
    /// Whether any listing had more than it gave.
    pub more: bool,
    /// The platform's answers not yet in.
    pub outstanding: BTreeSet<EffectId>,
    /// Why the platform would not list, if it would not: what the request
    /// is answered with, whatever the rest say.
    pub unlisted: Option<String>,
}

/// What one platform answer is for, within a listing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ReachStage {
    /// A token minted for listing an installation.
    Minting {
        /// Which installation.
        installation: u64,
    },
    /// An installation's repositories.
    Listing {
        /// Which installation.
        installation: u64,
    },
    /// What a token can read.
    Readable,
    /// Whose a token is, and when it expires.
    Owner,
}

/// Setup redirects being answered, by the identifier the platform's answer
/// carries.
pub type Installs = BTreeMap<EffectId, Installing>;
/// Installs begun, by the state their link carried, oldest first: what a
/// page asks by once its tab has come back.
pub type Begun = VecDeque<(String, Install)>;

/// The installation that came back under a state, if the state is one
/// minted here and one has.
pub fn arrived(begun: &Begun, state: &str) -> Option<u64> {
    begun
        .iter()
        .find(|(minted, _)| minted == state)
        .and_then(|(_, install)| install.installation)
}
/// Listings being assembled for forms, by the person's request held.
pub type Reachings = BTreeMap<Asking, Reaching>;
/// Which listing each platform answer is for, and what it is of.
pub type Reaches = BTreeMap<EffectId, (Asking, ReachStage)>;
/// Tokens minted for listing what an installation covers, per
/// installation, until shortly before each hour is up. Never a job's.
pub type ListingTokens = BTreeMap<u64, Minted>;
/// Tokens minted for projects' jobs, until shortly before each hour is up.
pub type Tokens = BTreeMap<ProjectId, Minted>;
/// Tokens being minted, by the identifier the answer carries: for which
/// project.
pub type Mintings = BTreeMap<EffectId, ProjectId>;
/// Wrappers' requests waiting on a token being minted, per project.
pub type Awaiting = BTreeMap<ProjectId, Vec<RequestId>>;

/// Whether a minted token is still worth serving: its hour is not within
/// the margin of being up.
fn still_good(minted: &Minted, now: Timestamp) -> bool {
    minted
        .expires
        .as_second()
        .checked_sub(RENEWED_BEFORE_SECONDS)
        .is_some_and(|edge| edge > now.as_second())
}

/// One request to the platform, as the world makes it.
fn platform_request(id: EffectId, rendered: stageman_platform::Request) -> Effect {
    Generic::Request {
        id,
        method: rendered.method,
        url: rendered.url,
        headers: rendered.headers,
        body: rendered.body.map(Bytes::new),
        within: CHECKED_WITHIN,
    }
}

/// What the platform said of a token, from its answer to the read of the
/// account — or why the answer is no use.
fn owned_of(responded: &Responded) -> Result<stageman_platform::Owned, PlatformError> {
    let platform = Platform::GitHub;
    match responded {
        Responded::Answered {
            status,
            headers,
            body,
        } => stageman_platform::owned(platform, *status, headers, body.as_slice()),
        Responded::Failed(why) => Err(PlatformError::Unreachable {
            platform,
            why: why.clone(),
        }),
    }
}

/// A listing's repositories as the rows a form is given.
fn rows_of(listing: stageman_platform::Listing) -> Vec<Reachable> {
    listing
        .repositories
        .into_iter()
        .map(|repository| Reachable {
            repository: views::wire_repository(&repository.address),
            private: repository.private,
        })
        .collect()
}

impl Running {
    /// Mints an install link for one press: the state in it is held, and
    /// is what the page asks by once the platform has brought the tab
    /// back — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    ///
    /// # Errors
    ///
    /// Fails if no App is registered on the platform.
    pub fn install_link(&mut self, platform: Platform) -> Result<Response, Refusal> {
        let Some(app) = self.state.apps.get(&platform) else {
            return Err(Refusal::AppMissing {
                platform: stageman_platform::shown(platform).to_owned(),
            });
        };
        let state = crate::mint(&mut self.rng).simple().to_string();
        let link = stageman_platform::install_link(platform, &app.slug, &state);
        self.begun.push_back((
            state.clone(),
            Install {
                platform,
                installation: None,
            },
        ));
        // One in, at most one out, for the reason the registrations give.
        if self.begun.len() > REMEMBERED {
            self.begun.pop_front();
        }
        Ok(Response::InstallLink(InstallLink { link, state }))
    }

    /// What came back under a state, if the state is this instance's:
    /// nothing yet, or an installation.
    fn arrival(&self, state: &str) -> Option<&Install> {
        self.begun
            .iter()
            .find(|(minted, _)| minted == state)
            .map(|(_, install)| install)
    }

    /// An installation came back under a state: named there, if the state
    /// is this instance's. Whether it was.
    fn arrival_returned(&mut self, state: &str, installation: u64) -> bool {
        match self.begun.iter_mut().find(|(minted, _)| minted == state) {
            Some((_, install)) => {
                install.installation = Some(installation);
                true
            }
            None => false,
        }
    }

    /// A state was spent by the save that named it: what came back under
    /// it is the project's now, and the state buys nothing more.
    pub(crate) fn spend_arrival(&mut self, state: &str) {
        self.begun.retain(|(minted, _)| minted != state);
    }

    /// Where the App on a platform is installed, as a page shows it, each
    /// with the projects reaching their repository through it.
    pub(crate) fn installations_view(&self, platform: Platform) -> Vec<InstallationView> {
        self.state
            .apps
            .get(&platform)
            .map(|app| {
                app.installations
                    .iter()
                    .map(|(id, installation)| InstallationView {
                        id: *id,
                        account: installation.account.clone(),
                        every_repository: installation.every_repository,
                        used_by: self.using(platform, *id),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The projects reaching their repository through an installation, by
    /// name.
    fn using(&self, platform: Platform, installation: u64) -> Vec<String> {
        self.state
            .projects
            .values()
            .filter(|watched| {
                watched.access.get(&platform) == Some(&Access::Installation { id: installation })
            })
            .map(|watched| watched.name.clone())
            .collect()
    }

    /// The projects reaching their repository through any installation of
    /// the App on a platform, by name: what forgetting the App would leave
    /// without access.
    pub(crate) fn installed_on(&self, platform: Platform) -> Vec<String> {
        self.state
            .projects
            .values()
            .filter(|watched| {
                matches!(
                    watched.access.get(&platform),
                    Some(Access::Installation { .. })
                )
            })
            .map(|watched| watched.name.clone())
            .collect()
    }

    /// The projects screen, saying whether an App is registered.
    pub(crate) fn projects_screen(&self) -> stageman_wire::Watching {
        views::watching_now(
            &self.state,
            &self.identities(),
            self.state.apps.contains_key(&Platform::GitHub),
            &crate::tunnel::dashboard(&self.domain, self.serving),
            self.stamp(),
        )
    }

    /// The browser came back from installing the App, if the path is the
    /// one an installation comes back to. False for any other path.
    ///
    /// Nothing about the identifier is trusted: it is fetched with the
    /// App's key before anything is kept, which is the check the
    /// platform's documentation asks for and the whole of it. The state
    /// the redirect carries names the page the tab was opened from and is
    /// no part of the check — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    pub fn came_back_installed(
        &mut self,
        id: RequestId,
        request: &Arrival,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let platform = Platform::GitHub;
        let Some(query) = request
            .path
            .strip_prefix(stageman_platform::installed_path(platform))
        else {
            return false;
        };
        let query = query.strip_prefix('?').unwrap_or(query);
        let Some(installation) =
            parameter(query, "installation_id").and_then(|id| id.parse::<u64>().ok())
        else {
            tracing::warn!("the browser came back to the installation's path naming none");
            effects.push(Effect::Answer {
                id,
                answer: Answer::Respond {
                    status: 400,
                    headers: [("content-type".to_owned(), "text/plain".to_owned())].into(),
                    body: Bytes::new(NO_INSTALLATION.as_bytes().to_vec()),
                },
            });
            return true;
        };
        let state = parameter(query, "state");
        let Some(app) = self.state.apps.get(&platform).cloned() else {
            self.install_failed(id, "no App is registered on this instance".to_owned());
            return true;
        };
        match stageman_platform::installation(platform, &app, installation, self.stamp()) {
            Ok(rendered) => {
                let effect = self.effect_id();
                self.installs.insert(
                    effect,
                    Installing {
                        request: id,
                        installation,
                        state,
                    },
                );
                effects.push(platform_request(effect, rendered));
            }
            Err(why) => self.install_failed(id, why.to_string()),
        }
        true
    }

    /// The platform answered about an installation being set up. False
    /// when the answer was to nothing of this module's.
    ///
    /// Kept beside the App once confirmed, on an update as on an install;
    /// the tokens minted on it dropped, so that a repository removed from
    /// it fails the next command loudly rather than an hour late; and the
    /// state the tab came back under, where it is this instance's, told
    /// which installation, so that the page holding it can ask.
    pub fn installed_answered(
        &mut self,
        id: EffectId,
        responded: &Responded,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(Installing { request, state, .. }) = self.installs.remove(&id) else {
            return false;
        };
        let platform = Platform::GitHub;
        let Some(app_id) = self.state.apps.get(&platform).map(|app| app.id) else {
            self.install_failed(request, "the App was forgotten meanwhile".to_owned());
            return true;
        };
        let outcome = match responded {
            Responded::Answered { status, body, .. } => {
                stageman_platform::installed(platform, app_id, *status, body.as_slice())
                    .map_err(|why| why.to_string())
            }
            Responded::Failed(why) => Err(format!(
                "{} could not be reached: {why}",
                stageman_platform::shown(platform)
            )),
        };
        match outcome {
            Ok(installed) => {
                if let Some(app) = self.state.apps.get_mut(&platform) {
                    app.installations.insert(
                        installed.id,
                        stageman_core::Installation {
                            account: installed.account.clone(),
                            every_repository: installed.every_repository,
                        },
                    );
                }
                self.install_failure = None;
                self.listing_tokens.remove(&installed.id);
                let on_it: Vec<ProjectId> = self
                    .state
                    .projects
                    .iter()
                    .filter(|(_, watched)| {
                        watched.access.get(&platform)
                            == Some(&Access::Installation { id: installed.id })
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for project in on_it {
                    self.minted.remove(&project);
                }
                self.dirty = true;
                // A tab that came back under no state at all — an update made
                // from the platform's own settings page — is nobody's to
                // announce, and closing it is right where it was opened by
                // script and harmless where it was not.
                let announced =
                    state.is_none_or(|state| self.arrival_returned(&state, installed.id));
                let account = installed.account;
                self.landed(
                    request,
                    &if announced {
                        Landing::Announced { account }
                    } else {
                        Landing::Unannounced { account }
                    },
                );
            }
            Err(why) => self.install_failed(request, why),
        }
        let _ = effects;
        true
    }

    /// An installation was not kept: why is said on the Instance page, and
    /// the tab that came back stays open saying it.
    fn install_failed(&mut self, request: RequestId, why: String) {
        tracing::warn!(%why, "an installation was not kept");
        self.install_failure = Some(why.clone());
        self.landed(request, &Landing::Refused { why });
    }

    /// Answers the tab that came back with the landing page, after the
    /// write where there is one, so that a tab that closes itself closes
    /// on a form the tick has already told.
    fn landed(&mut self, request: RequestId, landed: &Landing) {
        self.defer(Effect::Answer {
            id: request,
            answer: Answer::Respond {
                status: 200,
                headers: [(
                    "content-type".to_owned(),
                    "text/html; charset=utf-8".to_owned(),
                )]
                .into(),
                body: Bytes::new(landing(landed).into_bytes()),
            },
        });
    }

    /// Forgets an installation of the App on a platform, if no project
    /// reaches its repository through it.
    ///
    /// # Errors
    ///
    /// Fails if no App is registered there, if it holds no such
    /// installation, or if a project names it.
    pub fn forget_installation(
        &mut self,
        platform: Platform,
        installation: u64,
    ) -> Result<Response, Refusal> {
        let Some(app) = self.state.apps.get(&platform) else {
            return Err(Refusal::AppMissing {
                platform: stageman_platform::shown(platform).to_owned(),
            });
        };
        if !app.installations.contains_key(&installation) {
            return Err(Refusal::NoSuchInstallation { id: installation });
        }
        let used_by = self.using(platform, installation);
        if !used_by.is_empty() {
            return Err(Refusal::InstallationInUse { projects: used_by });
        }
        if let Some(app) = self.state.apps.get_mut(&platform) {
            app.installations.remove(&installation);
        }
        self.listing_tokens.remove(&installation);
        self.dirty = true;
        Ok(Response::Apps(self.apps()))
    }

    /// Forgets everything held for the App's installations: what a forget
    /// of the App leaves behind.
    pub(crate) fn forget_installations(&mut self) {
        self.listing_tokens.clear();
        self.minted.clear();
        self.install_failure = None;
        self.begun.clear();
    }

    /// A project's access changed, or its repository did: a token minted
    /// for what it held before is not for what it holds now.
    pub(crate) fn access_amended(&mut self, project: ProjectId) {
        self.minted.remove(&project);
    }

    /// A form asked what an access reaches: held while the platform lists
    /// it, and answered when the last listing lands — with the rows, or
    /// with why the platform would not list, either as an answer — see
    /// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    /// A state nothing has come back under yet is answered so, to be asked
    /// again on the next tick; one this instance does not hold is answered
    /// with why, as the form's cue to stop waiting. Refused only where the
    /// question itself is wrong: an empty token, or a project nothing is
    /// watched under.
    pub(crate) fn reaches(&mut self, id: Asking, through: Through, effects: &mut Vec<Effect>) {
        let mut reaching = Reaching {
            account: None,
            expires: None,
            repositories: Vec::new(),
            more: false,
            outstanding: BTreeSet::new(),
            unlisted: None,
        };
        let asked = match through {
            Through::Held { project } => match self.held_access(&project) {
                Ok(Some(Access::Token { secret, .. })) => {
                    self.reach_token(id, secret.expose(), &mut reaching, effects)
                }
                Ok(Some(Access::Installation { id: installation })) => {
                    self.reach_installation(id, installation, &mut reaching, effects)
                }
                Ok(None) => Ok(()),
                Err(refusal) => Err(refusal),
            },
            Through::Token { token } => self.reach_token(id, &token, &mut reaching, effects),
            Through::Arrived { state } => {
                match self.arrival(&state).map(|install| install.installation) {
                    Some(Some(installation)) => {
                        self.reach_installation(id, installation, &mut reaching, effects)
                    }
                    Some(None) => {
                        self.defer(AppEffect::Respond {
                            id,
                            response: Response::Reached(Reached::NotYet),
                        });
                        return;
                    }
                    None => {
                        reaching.unlisted = Some(Refusal::ArrivalUnknown.to_string());
                        Ok(())
                    }
                }
            }
        };
        if let Err(refusal) = asked {
            self.defer(AppEffect::Respond {
                id,
                response: Response::Refused(refusal),
            });
        } else if reaching.outstanding.is_empty() {
            self.reached(id, reaching);
        } else {
            self.reaching.insert(id, reaching);
        }
    }

    /// How a project reaches the platform, by the identifier a browser
    /// sent back: nothing where it holds nothing.
    fn held_access(&self, project: &str) -> Result<Option<Access>, Refusal> {
        let identifier = views::identify(&self.state, project)?;
        let watched =
            self.state
                .projects
                .get(&identifier)
                .ok_or_else(|| Refusal::UnknownProject {
                    id: project.to_owned(),
                })?;
        Ok(watched.access.get(&Platform::GitHub).cloned())
    }

    /// Asks what a token can read, and whose it is, into a listing being
    /// assembled: two reads at once, or the refusal an empty token earns
    /// without one.
    fn reach_token(
        &mut self,
        id: Asking,
        token: &str,
        reaching: &mut Reaching,
        effects: &mut Vec<Effect>,
    ) -> Result<(), Refusal> {
        let token = token.trim();
        if token.is_empty() {
            return Err(Refusal::Incomplete {
                field: "access".to_owned(),
            });
        }
        let secret = stageman_core::Secret::new(token.to_owned());
        for (stage, rendered) in [
            (
                ReachStage::Readable,
                stageman_platform::readable(Platform::GitHub, &secret),
            ),
            (
                ReachStage::Owner,
                stageman_platform::owner(Platform::GitHub, &secret),
            ),
        ] {
            let effect = self.effect_id();
            self.reaches.insert(effect, (id, stage));
            reaching.outstanding.insert(effect);
            effects.push(platform_request(effect, rendered));
        }
        Ok(())
    }

    /// Asks what one installation of the App reaches, into a listing being
    /// assembled: listed with the token held for it while that is good,
    /// and with one minted otherwise. An installation the App no longer
    /// holds reaches nothing, said as the platform would say it.
    fn reach_installation(
        &mut self,
        id: Asking,
        installation: u64,
        reaching: &mut Reaching,
        effects: &mut Vec<Effect>,
    ) -> Result<(), Refusal> {
        let platform = Platform::GitHub;
        let Some(app) = self.state.apps.get(&platform).cloned() else {
            return Err(Refusal::AppMissing {
                platform: stageman_platform::shown(platform).to_owned(),
            });
        };
        let Some(kept) = app.installations.get(&installation) else {
            reaching.unlisted = Some(Refusal::NoSuchInstallation { id: installation }.to_string());
            return Ok(());
        };
        reaching.account = Some(kept.account.clone());
        let now = self.stamp();
        let (stage, rendered) = self
            .listing_tokens
            .get(&installation)
            .filter(|minted| still_good(minted, now))
            .map_or_else(
                || {
                    (
                        ReachStage::Minting { installation },
                        stageman_platform::mint(platform, &app, installation, None, now),
                    )
                },
                |minted| {
                    (
                        ReachStage::Listing { installation },
                        Ok(stageman_platform::repositories(platform, &minted.token)),
                    )
                },
            );
        match rendered {
            Ok(rendered) => {
                let effect = self.effect_id();
                self.reaches.insert(effect, (id, stage));
                reaching.outstanding.insert(effect);
                effects.push(platform_request(effect, rendered));
            }
            Err(why) => {
                reaching.unlisted.get_or_insert_with(|| why.to_string());
            }
        }
        Ok(())
    }

    /// The platform answered a listing for a form. False when the answer
    /// was to no listing of this instance's.
    pub fn reached_answered(
        &mut self,
        id: EffectId,
        responded: &Responded,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some((request, stage)) = self.reaches.remove(&id) else {
            return false;
        };
        let platform = Platform::GitHub;
        let Some(mut reaching) = self.reaching.remove(&request) else {
            tracing::warn!("a listing landed for a request no longer held; ignored");
            return true;
        };
        reaching.outstanding.remove(&id);
        let answered = match responded {
            Responded::Answered { status, body, .. } => Ok((*status, body.as_slice())),
            Responded::Failed(why) => Err(PlatformError::Unreachable {
                platform,
                why: why.clone(),
            }),
        };
        let mut unlisted = |why: PlatformError| {
            reaching.unlisted.get_or_insert_with(|| why.to_string());
        };
        match stage {
            ReachStage::Owner => match owned_of(responded) {
                Ok(owned) => {
                    reaching.account = Some(owned.login);
                    reaching.expires = owned.expires;
                }
                Err(why) => unlisted(why),
            },
            ReachStage::Minting { installation } => {
                match answered
                    .and_then(|(status, body)| stageman_platform::minted(platform, status, body))
                {
                    Ok(minted) => {
                        let rendered = stageman_platform::repositories(platform, &minted.token);
                        self.listing_tokens.insert(installation, minted);
                        let effect = self.effect_id();
                        self.reaches
                            .insert(effect, (request, ReachStage::Listing { installation }));
                        reaching.outstanding.insert(effect);
                        effects.push(platform_request(effect, rendered));
                    }
                    Err(why) => unlisted(why),
                }
            }
            ReachStage::Listing { .. } => {
                match answered
                    .and_then(|(status, body)| stageman_platform::listed(platform, status, body))
                {
                    Ok(listing) => {
                        reaching.more |= listing.more;
                        reaching.repositories.extend(rows_of(listing));
                    }
                    Err(why) => unlisted(why),
                }
            }
            ReachStage::Readable => {
                match answered.and_then(|(status, body)| {
                    stageman_platform::readable_listed(platform, status, body)
                }) {
                    Ok(listing) => {
                        reaching.more |= listing.more;
                        reaching.repositories.extend(rows_of(listing));
                    }
                    Err(why) => unlisted(why),
                }
            }
        }
        if reaching.outstanding.is_empty() {
            self.reached(request, reaching);
        } else {
            self.reaching.insert(request, reaching);
        }
        true
    }

    /// Answers a form's listing, every answer in: the rows by owner and
    /// name, with the account the access is on; or why the platform would
    /// not list, as an answer all the same.
    fn reached(&mut self, request: Asking, mut reaching: Reaching) {
        let reached = if let Some(why) = reaching.unlisted {
            Reached::Unlisted { why }
        } else {
            reaching
                .repositories
                .sort_by(|one, other| one.repository.cmp(&other.repository));
            Reached::Listed {
                account: reaching.account,
                expires: reaching.expires.map(|at| at.to_string()),
                repositories: reaching.repositories,
                more: reaching.more,
            }
        };
        self.defer(AppEffect::Respond {
            id: request,
            response: Response::Reached(reached),
        });
    }

    /// A job's wrapper asked for its credential, and its project reaches
    /// the platform through an installation: answered from the token
    /// minted for the project while it is still good, and otherwise held
    /// while one is minted — one minting at a time per project, every
    /// request waiting on it answered when it lands.
    pub(crate) fn credential_from_installation(
        &mut self,
        id: RequestId,
        project: ProjectId,
        installation: u64,
        effects: &mut Vec<Effect>,
    ) {
        let platform = Platform::GitHub;
        let now = self.stamp();
        if let Some(minted) = self.minted.get(&project)
            && still_good(minted, now)
        {
            tracing::debug!(%project, "handed a token minted earlier and still good");
            Self::say_now(id, 200, minted.token.expose(), effects);
            return;
        }
        self.awaiting_tokens.entry(project).or_default().push(id);
        if self.minting.values().any(|minting| *minting == project) {
            return;
        }
        let Some(app) = self.state.apps.get(&platform).cloned() else {
            self.token_waiting_answered(
                project,
                &Err("no App is registered, so nothing can mint a token".to_owned()),
                effects,
            );
            return;
        };
        let Some(repository) = self
            .state
            .projects
            .get(&project)
            .map(|watched| watched.repository.clone())
        else {
            self.token_waiting_answered(
                project,
                &Err("the project is not watched, so no token can be minted for it".to_owned()),
                effects,
            );
            return;
        };
        match stageman_platform::mint(platform, &app, installation, Some(&repository), now) {
            Ok(rendered) => {
                let effect = self.effect_id();
                self.minting.insert(effect, project);
                effects.push(platform_request(effect, rendered));
            }
            Err(why) => self.token_waiting_answered(project, &Err(why.to_string()), effects),
        }
    }

    /// The platform answered a minting for a project's jobs. False when
    /// the answer was to no minting of this instance's.
    pub fn minted_answered(
        &mut self,
        id: EffectId,
        responded: &Responded,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(project) = self.minting.remove(&id) else {
            return false;
        };
        let platform = Platform::GitHub;
        let outcome = match responded {
            Responded::Answered { status, body, .. } => {
                stageman_platform::minted(platform, *status, body.as_slice())
                    .map_err(|why| why.to_string())
            }
            Responded::Failed(why) => Err(format!(
                "{} could not be reached: {why}",
                stageman_platform::shown(platform)
            )),
        };
        match outcome {
            Ok(minted) => {
                tracing::debug!(%project, "minted a token for the project's jobs, kept for most of its hour");
                let token = minted.token.expose().to_owned();
                self.minted.insert(project, minted);
                self.token_waiting_answered(project, &Ok(token), effects);
            }
            Err(why) => {
                tracing::warn!(%project, %why, "a token could not be minted for the project's jobs");
                self.token_waiting_answered(project, &Err(why), effects);
            }
        }
        true
    }

    /// Answers every wrapper waiting on a project's token: with the token,
    /// or with why there is none, which fails the command loudly.
    fn token_waiting_answered(
        &mut self,
        project: ProjectId,
        outcome: &Result<String, String>,
        effects: &mut Vec<Effect>,
    ) {
        for id in self.awaiting_tokens.remove(&project).unwrap_or_default() {
            match outcome {
                Ok(token) => Self::say_now(id, 200, token, effects),
                Err(why) => Self::say_now(
                    id,
                    BAD_GATEWAY,
                    &format!("a token could not be minted for this job: {why}"),
                    effects,
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Landing, RENEWED_BEFORE_SECONDS, landing, still_good};
    use stageman_core::{Secret, Timestamp};
    use stageman_platform::Minted;

    /// A token is served until the margin before its hour, and not after.
    #[test]
    fn a_minted_token_is_good_until_the_margin_before_its_hour() {
        let expires = Timestamp::from_second(3_600).expect("a time");
        let minted = Minted {
            token: Secret::new("ghs_x".to_owned()),
            expires,
        };
        let at = |second: i64| Timestamp::from_second(second).expect("a time");
        assert!(still_good(&minted, at(0)));
        assert!(still_good(&minted, at(3_600 - RENEWED_BEFORE_SECONDS - 1)));
        assert!(!still_good(&minted, at(3_600 - RENEWED_BEFORE_SECONDS)));
        assert!(!still_good(&minted, at(3_600)));
        assert!(!still_good(&minted, at(7_200)));
    }

    /// The three pages the tab that comes back is answered with, asserted
    /// whole per `docs/conventions.md` §4: the one that closes the tab,
    /// the one that stays because no page here will learn of the
    /// installation, and the one that stays to say why it was not kept —
    /// with the platform's words made safe for a page.
    #[test]
    fn the_landing_page_closes_itself_when_announced_and_stays_otherwise() {
        assert_eq!(
            landing(&Landing::Announced {
                account: "acme".to_owned()
            }),
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta \
             name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta \
             name=\"color-scheme\" content=\"light dark\">\n<title>stageman</title>\n<style>body{font:15px/1.5 \
             system-ui,sans-serif;margin:3rem auto;max-width:36rem;padding:0 \
             1rem}</style>\n</head>\n<body>\n<p>The App is installed on <b>acme</b>. Back in \
             stageman, the page you left has it.</p>\n<p>This tab closes itself; if it stays, \
             close it.</p>\n<script>window.close();</script>\n</body>\n</html>\n"
        );
        let unannounced = landing(&Landing::Unannounced {
            account: "acme".to_owned(),
        });
        assert!(
            unannounced.contains(
                "<p>The App is installed on <b>acme</b>, but this tab was opened before stageman \
                 last started, so the page you left will not learn of it.</p>\n<p>Close this tab \
                 and press Install the App again there: GitHub will bring you straight \
                 back.</p>\n"
            ),
            "{unannounced}"
        );
        assert!(!unannounced.contains("window.close"), "{unannounced}");
        let refused = landing(&Landing::Refused {
            why: "GitHub said <no> & meant it".to_owned(),
        });
        assert!(refused.contains(
            "<p>The App was not installed: GitHub said &lt;no&gt; &amp; meant it.</p>\n<p>Close \
             this tab and press Install the App again.</p>\n"
        ));
        assert!(!refused.contains("window.close"), "{refused}");
    }
}
