//! Where the instance's own app on a channel is installed: a workspace,
//! learned from the platform's redirect and kept beside the app — see
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
//!
//! An install is a state-bound arrival, on the shape
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`
//! gave the App's: a press mints a state and a link onto the platform; the
//! platform brings the tab back to a path of this instance's own with a
//! code and the state; the instance holds the browser's request while the
//! code is exchanged for the workspace's bot token, keeps the workspace
//! once the write has landed, names it under the state so that the page
//! that pressed can ask, and answers the tab with a page that closes it.
//! Everything held here is held and never kept: a daemon dying
//! mid-exchange answers nobody, and the operator presses once more.

use std::collections::{BTreeMap, VecDeque};

use stageman_channel::ChannelError;
use stageman_core::{Channel, Workspace};
use stageman_vocabulary::{
    Answer, Arrival, Bytes, Effect as Generic, EffectId, RequestId, Responded,
};
use stageman_wire::{InstallLink, Refusal};

use crate::apps::parameter;
use crate::checks::CHECKED_WITHIN;
use crate::installations::{LANDING, escaped};
use crate::requests::Response;
use crate::views::wire_channel;
use crate::{Effect, Running};

/// How many installs begun and not come back are remembered: every press
/// mints one, and only the last few can still be the one a tab comes back
/// with.
const REMEMBERED: usize = 16;

/// What the tab says when the workspace was kept and the page that opened
/// the tab will have it, before it closes itself.
const INSTALLED: (&str, &str) = (
    "<p>The app is installed on <b>",
    "</b>. Back in stageman, the page you left has it.</p>\n<p>This tab closes itself; if it \
     stays, close it.</p>\n<script>window.close();</script>\n",
);

/// What the tab says when the workspace was kept but came back under a
/// state this instance did not mint, or minted before it last started: the
/// page that opened the tab will not learn of it, so the tab stays open to
/// say what to do.
const INSTALLED_UNANNOUNCED: (&str, &str) = (
    "<p>The app is installed on <b>",
    "</b>, but this tab was opened before stageman last started, so the page you left will not \
     learn of it.</p>\n<p>Close this tab and press Install on a workspace again there: Slack \
     will bring you straight back.</p>\n",
);

/// What the tab says when the workspace was not kept, and stays open to
/// say it.
const NOT_INSTALLED: (&str, &str) = (
    "<p>The app was not installed: ",
    ".</p>\n<p>Close this tab and press Install on a workspace again.</p>\n",
);

/// What the tab is told when it came back with neither a code nor the
/// platform's word: not the platform's redirect at all.
const NO_CODE: &str = "The browser came back to the install's path with no code from Slack. Go \
                       back to the dashboard and press Install on a workspace again.";

/// What the tab the platform brought back is told.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Landing {
    /// Kept, and the page that opened the tab will have it: the tab closes.
    Announced {
        /// The workspace's name.
        name: String,
    },
    /// Kept, and no page of this instance's is waiting on it: the tab
    /// stays, saying what to do.
    Unannounced {
        /// The workspace's name.
        name: String,
    },
    /// Not kept, and the tab stays to say why.
    Refused {
        /// Why, as a clause.
        why: String,
    },
}

/// The page the tab is answered with, around one of the three sentences.
fn landing(landed: &Landing) -> String {
    let (opening, closing) = LANDING;
    let said = |around: (&str, &str), it: &str| format!("{}{}{}", around.0, escaped(it), around.1);
    let sentence = match landed {
        Landing::Announced { name } => said(INSTALLED, name),
        Landing::Unannounced { name } => said(INSTALLED_UNANNOUNCED, name),
        Landing::Refused { why } => said(NOT_INSTALLED, why),
    };
    format!("{opening}{sentence}{closing}")
}

/// One install begun from a page: on which channel, and which workspace
/// came back under its state, once one has.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Install {
    /// Which channel's app.
    pub channel: Channel,
    /// The workspace that came back, by its identifier, once one has: kept
    /// beside the app already, and named here so that the page holding
    /// the state can reach it and no other page can.
    pub workspace: Option<String>,
}

/// Installs begun, by the state their link carried, oldest first: what a
/// page asks by once its tab has come back.
pub type Begun = VecDeque<(String, Install)>;

/// One exchange being answered: the browser's request held, which
/// channel, and the state the tab came back under, if any.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Exchanging {
    /// The browser's request, as the world holds it open.
    pub request: RequestId,
    /// Which channel's app.
    pub channel: Channel,
    /// The state the redirect carried, which names the page that opened
    /// the tab.
    pub state: Option<String>,
}

/// Exchanges being answered, by the identifier the platform's answer
/// carries.
pub type Exchanges = BTreeMap<EffectId, Exchanging>;

/// What a channel's refusal of an exchange says on the tab.
fn refusal_words(channel: Channel, why: &ChannelError) -> String {
    let name = wire_channel(channel);
    match why {
        ChannelError::Unreachable(what) => format!("{name} could not be reached: {what}"),
        ChannelError::Refused(word) => format!("{name} refused it ({word})"),
        ChannelError::NoAnswer | ChannelError::NoIdentifier => {
            format!("{name} accepted it and named no workspace")
        }
        ChannelError::NotABot => format!("{name} answered with something that is not a bot token"),
        ChannelError::Unreadable(what) => format!("{name} answered something unreadable: {what}"),
    }
}

impl Running {
    /// Mints where to install the app on a workspace, for one press: the
    /// state in the link is what the page asks by once the tab has come
    /// back.
    ///
    /// # Errors
    ///
    /// Fails if no app is registered on the channel.
    pub fn workspace_link(&mut self, channel: Channel) -> Result<Response, Refusal> {
        let Some(app) = self.state.channel_apps.get(&channel) else {
            return Err(Refusal::ChannelAppMissing {
                channel: wire_channel(channel).to_owned(),
            });
        };
        let state = crate::mint(&mut self.rng).simple().to_string();
        let instance = crate::tunnel::dashboard(&self.domain, self.reached);
        let link = stageman_channel::install_link(channel, &app.client_id, &state, &instance);
        self.workspaces_begun.push_back((
            state.clone(),
            Install {
                channel,
                workspace: None,
            },
        ));
        // One in, at most one out, for the reason the registrations give.
        if self.workspaces_begun.len() > REMEMBERED {
            self.workspaces_begun.pop_front();
        }
        Ok(Response::InstallLink(InstallLink { link, state }))
    }

    /// The browser came back from installing the app on a workspace, if
    /// the path is the one an install comes back to. False for any other
    /// path, which is the dashboard's to serve.
    ///
    /// Nothing the redirect carries is trusted but the code, and the code
    /// only as far as the platform honours it: the exchange is the check,
    /// since a code the platform did not issue buys nothing. The state
    /// names the page the tab was opened from and is no part of it.
    pub fn came_back_workspace(
        &mut self,
        id: RequestId,
        request: &Arrival,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let channel = Channel::Slack;
        let Some(query) = request
            .path
            .strip_prefix(stageman_channel::installed_path(channel))
        else {
            return false;
        };
        let query = query.strip_prefix('?').unwrap_or(query);
        let state = parameter(query, "state");
        // A person who did not allow: the platform brings the tab back
        // with its word in the code's place.
        if let Some(error) = parameter(query, "error") {
            self.workspace_refused(id, format!("{} said {error}", wire_channel(channel)));
            return true;
        }
        let Some(code) = parameter(query, "code") else {
            tracing::warn!("the browser came back to the install's path with no code");
            effects.push(Effect::Answer {
                id,
                answer: Answer::Respond {
                    status: 400,
                    headers: [("content-type".to_owned(), "text/plain".to_owned())].into(),
                    body: Bytes::new(NO_CODE.as_bytes().to_vec()),
                },
            });
            return true;
        };
        let Some(app) = self.state.channel_apps.get(&channel) else {
            self.workspace_refused(id, "no Slack app is registered on this instance".to_owned());
            return true;
        };
        let instance = crate::tunnel::dashboard(&self.domain, self.reached);
        let rendered = stageman_channel::exchange(
            channel,
            &app.client_id,
            &app.client_secret,
            &code,
            &instance,
        );
        let effect = self.effect_id();
        self.workspace_exchanges.insert(
            effect,
            Exchanging {
                request: id,
                channel,
                state,
            },
        );
        effects.push(Generic::Request {
            id: effect,
            method: rendered.method,
            url: rendered.url,
            headers: rendered.headers,
            body: rendered.body.map(Bytes::new),
            within: CHECKED_WITHIN,
        });
        true
    }

    /// The platform answered about an install's code. False when the
    /// answer was to no exchange of this module's.
    ///
    /// Kept beside the app once answered, on a second install of the same
    /// workspace as on the first; and the state the tab came back under,
    /// where it is this instance's, told which workspace, so that the page
    /// holding it can ask.
    pub fn workspace_exchanged(
        &mut self,
        id: EffectId,
        responded: &Responded,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(Exchanging {
            request,
            channel,
            state,
        }) = self.workspace_exchanges.remove(&id)
        else {
            return false;
        };
        let outcome = match responded {
            Responded::Answered { status, body, .. } => {
                stageman_channel::installed(channel, *status, body.as_slice())
                    .map_err(|why| refusal_words(channel, &why))
            }
            Responded::Failed(why) => Err(format!(
                "{} could not be reached: {why}",
                wire_channel(channel)
            )),
        };
        match outcome {
            Ok(installed) => {
                let Some(app) = self.state.channel_apps.get_mut(&channel) else {
                    self.workspace_refused(request, "the app was forgotten meanwhile".to_owned());
                    return true;
                };
                app.workspaces.insert(
                    installed.team.clone(),
                    Workspace {
                        name: installed.name.clone(),
                        bot_user: installed.bot_user,
                        bot_token: installed.bot_token,
                    },
                );
                self.workspace_failure = None;
                self.dirty = true;
                // A tab that came back under no state at all is nobody's
                // to announce, and closing it is right where it was opened
                // by script and harmless where it was not.
                let announced =
                    state.is_none_or(|state| self.workspace_returned(&state, &installed.team));
                let name = installed.name;
                self.workspace_landed(
                    request,
                    &if announced {
                        Landing::Announced { name }
                    } else {
                        Landing::Unannounced { name }
                    },
                );
            }
            Err(why) => self.workspace_refused(request, why),
        }
        let _ = effects;
        true
    }

    /// A workspace came back under a state: named there, if the state is
    /// this instance's. Whether it was.
    fn workspace_returned(&mut self, state: &str, team: &str) -> bool {
        match self
            .workspaces_begun
            .iter_mut()
            .find(|(minted, _)| minted == state)
        {
            Some((_, install)) => {
                install.workspace = Some(team.to_owned());
                true
            }
            None => false,
        }
    }

    /// A workspace was not kept: why is said on the Instance page, and the
    /// tab that came back stays open saying it.
    fn workspace_refused(&mut self, request: RequestId, why: String) {
        tracing::warn!(%why, "a workspace was not kept");
        self.workspace_failure = Some(why.clone());
        self.workspace_landed(request, &Landing::Refused { why });
    }

    /// Answers the tab that came back with the landing page, after the
    /// write where there is one, so that a tab that closes itself closes
    /// on a form the tick has already told.
    fn workspace_landed(&mut self, request: RequestId, landed: &Landing) {
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

    /// Forgets a workspace the app is installed on.
    ///
    /// # Errors
    ///
    /// Fails if no app is registered on the channel, or if it is not
    /// installed on that workspace.
    pub fn forget_workspace(&mut self, channel: Channel, id: &str) -> Result<Response, Refusal> {
        let Some(app) = self.state.channel_apps.get_mut(&channel) else {
            return Err(Refusal::ChannelAppMissing {
                channel: wire_channel(channel).to_owned(),
            });
        };
        if app.workspaces.remove(id).is_none() {
            return Err(Refusal::NoSuchWorkspace { id: id.to_owned() });
        }
        self.dirty = true;
        Ok(Response::Apps(self.apps()))
    }

    /// Forgets everything held for the app's workspaces: what a forget of
    /// the app leaves behind.
    pub fn forget_workspaces(&mut self) {
        self.workspaces_begun.clear();
        self.workspace_failure = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{Landing, landing, refusal_words};
    use stageman_channel::ChannelError;
    use stageman_core::Channel;

    /// The three pages the tab that comes back is answered with, asserted
    /// whole per `docs/conventions.md` §4: the one that closes the tab,
    /// the one that stays because no page here will learn of the
    /// workspace, and the one that stays to say why it was not kept, with
    /// the platform's words made safe for a page.
    #[test]
    fn the_landing_page_closes_itself_when_announced_and_stays_otherwise() {
        assert_eq!(
            landing(&Landing::Announced {
                name: "Acme".to_owned()
            }),
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta \
             name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta \
             name=\"color-scheme\" content=\"light dark\">\n<title>stageman</title>\n<style>body{font:15px/1.5 \
             system-ui,sans-serif;margin:3rem auto;max-width:36rem;padding:0 \
             1rem}</style>\n</head>\n<body>\n<p>The app is installed on <b>Acme</b>. Back in \
             stageman, the page you left has it.</p>\n<p>This tab closes itself; if it stays, \
             close it.</p>\n<script>window.close();</script>\n</body>\n</html>\n"
        );
        let unannounced = landing(&Landing::Unannounced {
            name: "Acme".to_owned(),
        });
        assert!(
            unannounced.contains(
                "<p>The app is installed on <b>Acme</b>, but this tab was opened before stageman \
                 last started, so the page you left will not learn of it.</p>\n<p>Close this tab \
                 and press Install on a workspace again there: Slack will bring you straight \
                 back.</p>\n"
            ),
            "{unannounced}"
        );
        assert!(!unannounced.contains("window.close"), "{unannounced}");
        let refused = landing(&Landing::Refused {
            why: "Slack said <no> & meant it".to_owned(),
        });
        assert!(refused.contains(
            "<p>The app was not installed: Slack said &lt;no&gt; &amp; meant it.</p>\n<p>Close \
             this tab and press Install on a workspace again.</p>\n"
        ));
        assert!(!refused.contains("window.close"), "{refused}");
    }

    /// What the tab says of each way an exchange can fail: the platform's
    /// word where it gave one, and what went wrong on the way otherwise.
    #[test]
    fn a_refused_exchange_is_said_in_the_platforms_words() {
        let words = |why| refusal_words(Channel::Slack, &why);
        assert_eq!(
            words(ChannelError::Refused("invalid_code".to_owned())),
            "Slack refused it (invalid_code)"
        );
        assert_eq!(
            words(ChannelError::Unreachable("dns".to_owned())),
            "Slack could not be reached: dns"
        );
        assert_eq!(
            words(ChannelError::NoAnswer),
            "Slack accepted it and named no workspace"
        );
        assert_eq!(
            words(ChannelError::Unreadable("not json".to_owned())),
            "Slack answered something unreadable: not json"
        );
        assert_eq!(
            words(ChannelError::NotABot),
            "Slack answered with something that is not a bot token"
        );
    }
}
