//! The Apps this instance owns on each platform: registered from the
//! dashboard by the platform's own flow, kept sealed, and forgotten from
//! the same page — see
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
//!
//! A registration is three steps across two processes. The page asks for a
//! form: the instance mints a state token, holds it, and answers with where
//! the form posts to and the manifest it carries. The browser posts it to
//! the platform, which creates the App and sends the browser back to a path
//! of this instance's own with a code and the state. The instance checks
//! the state, holds the browser's request, asks the world to make the one
//! request that converts the code — rendered and read by the platform
//! crate, so the world knows nothing of it — keeps what the platform
//! answered once the write has landed, and sends the browser on to the
//! page. Everything held here is held and
//! never kept: a daemon dying mid-registration answers nobody, and the
//! operator presses once more.

use std::collections::VecDeque;

use std::collections::BTreeMap;

use stageman_core::{Channel, ChannelApp, Platform, PlatformApp, Secret};
use stageman_vocabulary::{
    Answer, Arrival, Bytes, Effect as Generic, EffectId, RequestId, Responded,
};
use stageman_wire::{Apps, ChannelAppView, PlatformAppView, Refusal, Registration, WorkspaceView};

use crate::checks::CHECKED_WITHIN;
use crate::requests::Response;
use crate::{Effect, Running};

/// How many registrations begun and not finished are remembered: a page
/// re-read on every tick asks for a form each time, and only the last few
/// can still be the one a browser comes back with.
const REMEMBERED: usize = 8;

/// One registration begun: what the form was asked for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Registering {
    /// Which platform's App.
    pub platform: Platform,
}

/// The page the browser is sent on to, once the platform has answered.
const PAGE: &str = "/instance";

/// What a person sees when the browser came back with a state nobody
/// minted: a registration begun elsewhere, or before a restart.
const NOT_BEGUN_HERE: &str = "This registration was not begun by this instance, or it was begun \
                              before the instance restarted. Go back to the dashboard and press \
                              again.";

impl Running {
    /// Mints a registration form for the page: a state token held for the
    /// browser's return, and where the form posts to with what it carries.
    ///
    /// # Errors
    ///
    /// Fails if the manifest would not serialise, which is a fault in this
    /// process and reported as one.
    pub fn registration(
        &mut self,
        platform: Platform,
        anywhere: bool,
    ) -> Result<Registration, Refusal> {
        let instance = crate::tunnel::dashboard(&self.domain, self.serving);
        let manifest =
            stageman_platform::manifest(platform, &instance, anywhere).map_err(|why| {
                tracing::error!(%why, "the App's manifest could not be composed");
                Refusal::Failed
            })?;
        let state = crate::mint(&mut self.rng).simple().to_string();
        self.registrations
            .push_back((state.clone(), Registering { platform }));
        // One in, at most one out: each press adds one, so one drop keeps
        // the bound, and a comparison that went wrong could not loop.
        if self.registrations.len() > REMEMBERED {
            self.registrations.pop_front();
        }
        Ok(Registration {
            action: stageman_platform::register_form(platform, &state, None),
            manifest,
            state,
        })
    }

    /// The Apps this instance owns, as the page shows them: each with
    /// where it is installed, where to install it, and what came of the
    /// last installation if it was not kept.
    #[must_use]
    pub fn apps(&self) -> Apps {
        let platform = Platform::GitHub;
        let channel = Channel::Slack;
        let instance = crate::tunnel::dashboard(&self.domain, self.serving);
        Apps {
            github: self.state.apps.get(&platform).map(|app| PlatformAppView {
                slug: app.slug.clone(),
                link: stageman_platform::app_link(platform, &app.slug),
                installations: self.installations_view(platform),
                install_failure: self.install_failure.clone(),
            }),
            failed: self.app_failure.clone(),
            slack: self
                .state
                .channel_apps
                .get(&channel)
                .map(|app| ChannelAppView {
                    client_id: app.client_id.clone(),
                    workspaces: app
                        .workspaces
                        .iter()
                        .map(|(id, workspace)| WorkspaceView {
                            id: id.clone(),
                            name: workspace.name.clone(),
                            used_by: Vec::new(),
                        })
                        .collect(),
                }),
            slack_form: stageman_channel::app_form(channel, &instance),
        }
    }

    /// Keeps the app this instance owns on a channel, from the three values
    /// pasted on the Instance page — see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
    ///
    /// Its app-level token was checked before this is reached, as a
    /// binding's is; the client pair cannot be, and is kept as pasted.
    /// Registering over an app already kept replaces its values, since
    /// rotating a credential is the ordinary reason to come back to the
    /// page; the workspaces stay when the app is the same one, by its
    /// client identifier, and go when it is another's, since an install of
    /// one app is not an install of another.
    ///
    /// # Errors
    ///
    /// Fails if any of the three values is blank.
    pub fn register_channel_app(
        &mut self,
        channel: Channel,
        client_id: &str,
        client_secret: &str,
        app_token: &str,
    ) -> Result<Response, Refusal> {
        let (client_id, client_secret, app_token) =
            three_values(client_id, client_secret, app_token)?;
        let workspaces = match self.state.channel_apps.remove(&channel) {
            Some(had) if had.client_id == client_id => had.workspaces,
            _ => BTreeMap::new(),
        };
        self.state.channel_apps.insert(
            channel,
            ChannelApp {
                client_id: client_id.to_owned(),
                client_secret: Secret::new(client_secret.to_owned()),
                app_token: Secret::new(app_token.to_owned()),
                workspaces,
            },
        );
        self.dirty = true;
        Ok(Response::Apps(self.apps()))
    }

    /// Forgets the app on a channel.
    ///
    /// Left on the platform for the operator to delete, as the App is.
    ///
    /// # Errors
    ///
    /// Fails if no app is registered on that channel.
    pub fn forget_channel_app(&mut self, channel: Channel) -> Result<Response, Refusal> {
        if self.state.channel_apps.remove(&channel).is_none() {
            return Err(Refusal::ChannelAppMissing {
                channel: crate::views::wire_channel(channel).to_owned(),
            });
        }
        self.dirty = true;
        Ok(Response::Apps(self.apps()))
    }

    /// Forgets the App on a platform.
    ///
    /// Left on the platform for the operator to delete: this instance acts
    /// there only to mint what it was installed to mint. Refused while a
    /// project reaches its repository through it, as an agent is refused
    /// while a project names it.
    ///
    /// # Errors
    ///
    /// Fails if no App is registered on that platform, or if a project is
    /// installed on through it.
    pub fn forget_app(&mut self, platform: Platform) -> Result<Response, Refusal> {
        if !self.state.apps.contains_key(&platform) {
            return Err(Refusal::AppMissing {
                platform: stageman_platform::shown(platform).to_owned(),
            });
        }
        let installed_on = self.installed_on(platform);
        if !installed_on.is_empty() {
            return Err(Refusal::AppInUse {
                platform: stageman_platform::shown(platform).to_owned(),
                projects: installed_on,
            });
        }
        self.state.apps.remove(&platform);
        self.app_failure = None;
        self.forget_installations();
        self.dirty = true;
        Ok(Response::Apps(self.apps()))
    }

    /// The browser came back from the platform, if the path is the one a
    /// registration comes back to. False for any other path, which is the
    /// dashboard's to serve.
    pub fn came_back(
        &mut self,
        id: RequestId,
        request: &Arrival,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let platform = Platform::GitHub;
        let Some(query) = request
            .path
            .strip_prefix(stageman_platform::registered_path(platform))
        else {
            return false;
        };
        let query = query.strip_prefix('?').unwrap_or(query);
        let (code, state) = (parameter(query, "code"), parameter(query, "state"));
        let begun = state.as_ref().and_then(|state| {
            self.registrations
                .iter()
                .position(|(minted, _)| minted == state)
        });
        let (Some(code), Some(at)) = (code, begun) else {
            tracing::warn!("a registration came back with a state this instance did not mint");
            effects.push(Effect::Answer {
                id,
                answer: Answer::Respond {
                    status: 400,
                    headers: [("content-type".to_owned(), "text/plain".to_owned())].into(),
                    body: Bytes::new(NOT_BEGUN_HERE.as_bytes().to_vec()),
                },
            });
            return true;
        };
        // Spent: a state buys one exchange, as the code does.
        let (_, registering) = self
            .registrations
            .remove(at)
            .unwrap_or((String::new(), Registering { platform }));
        let effect = self.effect_id();
        self.exchanging.insert(effect, (id, registering));
        let rendered = stageman_platform::exchange(platform, &code);
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

    /// The platform answered about a code. False when the answer was to no
    /// exchange of this instance's.
    pub fn exchanged(
        &mut self,
        id: EffectId,
        responded: &Responded,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some((request, registering)) = self.exchanging.remove(&id) else {
            return false;
        };
        let platform = registering.platform;
        let outcome = match responded {
            Responded::Answered { status, body, .. } => {
                stageman_platform::registered(platform, *status, body.as_slice())
                    .map_err(|why| why.to_string())
            }
            Responded::Failed(why) => Err(format!(
                "{} could not be reached: {why}",
                stageman_platform::shown(platform)
            )),
        };
        match outcome {
            Ok(registered) => {
                self.state.apps.insert(
                    platform,
                    PlatformApp {
                        id: registered.id,
                        slug: registered.slug,
                        client_id: registered.client_id,
                        private_key: registered.private_key,
                        // Installed nowhere yet: what the setup redirect
                        // brings, per
                        // `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
                        installations: std::collections::BTreeMap::new(),
                    },
                );
                self.app_failure = None;
                self.dirty = true;
            }
            Err(why) => {
                tracing::warn!(%why, "an App was not registered");
                self.app_failure = Some(why);
            }
        }
        // Sent on to the page either way, after the write where there is
        // one, so that the page it lands on shows what was kept.
        self.defer(Effect::Answer {
            id: request,
            answer: Answer::Respond {
                status: 303,
                headers: [("location".to_owned(), PAGE.to_owned())].into(),
                body: Bytes::new(Vec::new()),
            },
        });
        let _ = effects;
        true
    }
}

/// The three values a channel app is registered from, trimmed, or the
/// refusal for one left blank.
///
/// One function for the two places that need it — the check that asks the
/// platform about the app-level token, which must refuse before asking,
/// and the keeper, which must refuse if reached any other way — so that
/// neither can drift from the other.
///
/// # Errors
///
/// Fails if any of the three is blank.
pub fn three_values<'a>(
    client_id: &'a str,
    client_secret: &'a str,
    app_token: &'a str,
) -> Result<(&'a str, &'a str, &'a str), Refusal> {
    let trimmed = (client_id.trim(), client_secret.trim(), app_token.trim());
    if trimmed.0.is_empty() || trimmed.1.is_empty() || trimmed.2.is_empty() {
        return Err(Refusal::ChannelAppIncomplete);
    }
    Ok(trimmed)
}

/// One parameter of a query string, where it is made of the characters a
/// code or a state token is made of: letters, digits and the three marks
/// the platform uses. Anything else is not the parameter.
pub fn parameter(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
        .filter(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .map(str::to_owned)
}

/// What an awake instance holds about registrations, for the snapshot.
pub type Registrations = VecDeque<(String, Registering)>;

/// What is held while a code is converted: the browser's request, as the
/// world holds it open, and what the registration was for.
pub type Exchanging = std::collections::BTreeMap<EffectId, (RequestId, Registering)>;

#[cfg(test)]
mod tests {
    use super::parameter;

    /// A parameter is read by name from wherever it is in the query, and
    /// refused when it carries anything a code or a state could not.
    /// Each value blank on its own is refused, whichever it is, and all
    /// three given are handed back trimmed.
    #[test]
    fn a_registration_needs_each_of_its_three_values() {
        for (client_id, client_secret, app_token) in [
            ("", "s3cret", "xapp-1"),
            ("1234.5678", "  ", "xapp-1"),
            ("1234.5678", "s3cret", "\t"),
        ] {
            assert_eq!(
                super::three_values(client_id, client_secret, app_token),
                Err(stageman_wire::Refusal::ChannelAppIncomplete),
                "{client_id:?} {client_secret:?} {app_token:?}"
            );
        }
        assert_eq!(
            super::three_values(" 1234.5678 ", "s3cret\n", " xapp-1"),
            Ok(("1234.5678", "s3cret", "xapp-1"))
        );
    }

    #[test]
    fn a_query_parameter_is_read_by_name_and_only_when_well_formed() {
        assert_eq!(
            parameter("code=a1b2&state=f00d", "code").as_deref(),
            Some("a1b2")
        );
        assert_eq!(
            parameter("code=a1b2&state=f00d", "state").as_deref(),
            Some("f00d")
        );
        assert_eq!(parameter("code=a1b2", "state"), None);
        assert_eq!(parameter("code=", "code"), None);
        assert_eq!(parameter("code=a%20b", "code"), None);
        assert_eq!(parameter("code=../x", "code"), None);
        assert_eq!(parameter("", "code"), None);
    }
}
