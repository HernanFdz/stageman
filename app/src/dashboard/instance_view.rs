//! What the instance configures about itself: the Apps it owns on each
//! platform, registered from here by the platform's own flow — see
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
//!
//! Named for what it holds rather than for who may touch it: nothing here
//! has a second person yet, and when one arrives what is restricted will be
//! restricted by what it is. The status line under every page stays a line;
//! this page holds what an operator changes.
//!
//! **Registering is a form the browser posts to the platform**, not a
//! request to this instance: the platform's flow wants a navigation with
//! the manifest as a field, so the form is a plain one, with its action and
//! its manifest composed on the server and minted for one attempt. The
//! browser comes back to a path the instance answers itself, and lands
//! here again with the App kept or the refusal said.
//!
//! **Installing is a link in a new tab**, and the browser lands here again
//! with the installation kept beside the App, or the refusal said — see
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
//! Where the App is installed is listed here, each installation
//! forgettable while no project reaches its repository through it.
//!
//! **The Slack app is registered by pasting three values**, since the
//! platform's own flow stops short of what a redirect could carry: the
//! app-level token is checked against Slack before anything is kept, the
//! client pair is kept as pasted and checked by the first install — see
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use stageman_wire::Part;

use super::error::{DashboardError, DashboardResult};
use super::live::Live;
use crate::ui::{
    BESIDE, Button, ButtonVariant, Card, EmptyState, FIELD, Field, Guide, Icon, Mark, Modal,
    Reference, Segmented, Skeleton, Tooltip,
};

pub use stageman_wire::{
    Apps, ChannelAppView, InstallLink, InstallationView, PlatformAppView, Registration,
};

/// The Apps this instance owns, and what the last registration said.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[get("/api/instance/apps")]
pub async fn apps() -> DashboardResult<Apps> {
    match super::ask(Request::Apps).await? {
        Response::Apps(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// A form to register an App with, minted for one attempt.
///
/// # Errors
///
/// Fails if the platform is not one this build knows.
#[post("/api/instance/apps/registration")]
pub async fn registration(platform: String, anywhere: bool) -> DashboardResult<Registration> {
    match super::ask(Request::Registration { platform, anywhere }).await? {
        Response::Registration(form) => Ok(form),
        other => Err(super::unexpected(&other)),
    }
}

/// Where to install the App, minted for one press: the state in the link
/// is what the page asks by once the tab has come back — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
///
/// # Errors
///
/// Fails if no App is registered on the platform, or if the platform is
/// not one this build knows.
#[post("/api/instance/apps/install-link")]
pub async fn install_link(platform: String) -> DashboardResult<InstallLink> {
    match super::ask(Request::InstallLink { platform }).await? {
        Response::InstallLink(minted) => Ok(minted),
        other => Err(super::unexpected(&other)),
    }
}

/// Forgets the App on a platform.
///
/// # Errors
///
/// Fails if no App is registered there.
#[post("/api/instance/apps/forget")]
pub async fn forget_app(platform: String) -> DashboardResult<Apps> {
    match super::ask(Request::ForgetApp { platform }).await? {
        Response::Apps(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// Forgets one installation of the App on a platform.
///
/// # Errors
///
/// Fails if no App is registered there, if it holds no such installation,
/// or if a project reaches its repository through it.
#[post("/api/instance/apps/forget-installation")]
pub async fn forget_installation(platform: String, id: u64) -> DashboardResult<Apps> {
    match super::ask(Request::ForgetInstallation { platform, id }).await? {
        Response::Apps(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// Registers the app this instance owns on a channel, from three values
/// pasted — see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
///
/// # Errors
///
/// Fails if a value is blank, if the channel refuses the app-level token,
/// or if the channel could not be asked.
#[post("/api/instance/apps/register-channel-app")]
pub async fn register_channel_app(
    channel: String,
    client_id: String,
    client_secret: String,
    app_token: String,
) -> DashboardResult<Apps> {
    match super::ask(Request::RegisterChannelApp {
        channel,
        client_id,
        client_secret,
        app_token,
    })
    .await?
    {
        Response::Apps(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// Forgets the app this instance owns on a channel.
///
/// # Errors
///
/// Fails if no app is registered there.
#[post("/api/instance/apps/forget-channel-app")]
pub async fn forget_channel_app(channel: String) -> DashboardResult<Apps> {
    match super::ask(Request::ForgetChannelApp { channel }).await? {
        Response::Apps(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// The Instance page.
#[component]
pub fn InstanceView() -> Element {
    let live = use_context::<Live>();
    let mut reading = use_server_future(move || {
        let _ = live.follow();
        apps()
    })?;
    let mut failure = use_signal(|| None::<DashboardError>);

    rsx! {
        div { class: "flex flex-col gap-4",
            if let Some(reason) = failure() {
                Card { title: "That did not work",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            }
            match reading.cloned() {
                Some(Ok(shown)) => rsx! {
                    GitHubApp {
                        app: shown.github,
                        failed: shown.failed,
                        onchanged: move |outcome: DashboardResult<Apps>| match outcome {
                            Ok(fresh) => {
                                failure.set(None);
                                reading.set(Some(Ok(fresh)));
                            }
                            Err(reason) => failure.set(Some(reason)),
                        },
                    }
                    SlackApp {
                        app: shown.slack,
                        form: shown.slack_form,
                        onchanged: move |outcome: DashboardResult<Apps>| match outcome {
                            Ok(fresh) => {
                                failure.set(None);
                                reading.set(Some(Ok(fresh)));
                            }
                            Err(reason) => failure.set(Some(reason)),
                        },
                    }
                },
                Some(Err(reason)) => rsx! {
                    Card { title: "The instance could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                None => rsx! { Skeleton {} },
            }
        }
    }
}

/// The GitHub App: registered, with the way to forget it; or the form to
/// register one.
#[component]
fn GitHubApp(
    app: Option<PlatformAppView>,
    failed: Option<String>,
    onchanged: EventHandler<DashboardResult<Apps>>,
) -> Element {
    let mut forgetting = use_signal(|| false);
    // Why the last press of Install could not open the platform, if the
    // last one could not: a link the instance would not mint.
    let mut not_opened = use_signal(|| None::<String>);

    rsx! {
        Card {
            title: "GitHub App",
            note: "One App this instance owns, which a project installs on its repository \
                   instead of pasting a token.",
            info: "Registered under your account by GitHub's own form, which comes back here \
                   with the App's key; the key stays sealed in the instance's file. A project \
                   that installs it runs its jobs on tokens minted per turn, good for an hour \
                   and for that one repository. Contents, issues and pull requests write, \
                   metadata read; no webhook.",
            if let Some(why) = failed {
                p { role: "alert", class: "mb-3 text-sm text-failed",
                    "The App was not registered: {why}"
                }
            }
            match app {
                Some(app) => rsx! {
                    div { class: "flex flex-col gap-4",
                        div { class: "flex items-center gap-3",
                            Reference {
                                mark: "github",
                                says: "The App on GitHub",
                                link: Some(app.link.clone()),
                            }
                            span { class: "font-mono text-sm", "{app.slug}" }
                            span { class: "ml-auto flex items-center gap-2",
                                Button {
                                    variant: ButtonVariant::Danger,
                                    onclick: move |_| forgetting.set(true),
                                    "Forget…"
                                }
                            }
                        }
                        if let Some(why) = app.install_failure.clone() {
                            p { role: "alert", class: "text-sm text-failed",
                                "The App was not installed: {why}"
                            }
                        }
                        if let Some(why) = not_opened() {
                            p { role: "alert", class: "text-sm text-failed", "{why}" }
                        }
                        // Where it is installed, as the platform has told
                        // this instance; the way onto the platform to
                        // install it sits at the end of the label's line,
                        // where what adds to a list belongs.
                        Field {
                            label: "Installed on",
                            note: "Where the App is installed, as GitHub brought you back to say. A project's repository is chosen from what these reach.",
                            info: "Installing opens GitHub in a new tab: choose the account, and all of \
                                   its repositories or some, and GitHub brings you back here. A form \
                                   left open on a project fills its list in when you do. An \
                                   installation nothing uses can be forgotten here; one a project \
                                   reaches its repository through cannot, and says which.",
                            aside: rsx! {
                                // By script rather than a link, so that the tab
                                // can close itself when GitHub brings it back —
                                // see `docs/conventions.md` §3.
                                Tooltip {
                                    text: "Opens GitHub in a tab of its own to install this instance's \
                                           App on an account, for some or all of its repositories. The \
                                           tab closes itself when GitHub brings it back, and this list \
                                           fills in.",
                                    wrap: true,
                                    at_end: true,
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        class: "h-8.5 gap-1.5 px-2 text-xs",
                                        aria_label: "Install the App",
                                        // The tab in the press, the address after
                                        // it, for the reason `open_a_tab` gives.
                                        onclick: move |_| {
                                            super::open_a_tab();
                                            spawn(async move {
                                                match install_link("github".to_owned()).await {
                                                    Ok(minted) => {
                                                        super::send_the_tab(&minted.link);
                                                        not_opened.set(None);
                                                    }
                                                    Err(why) => {
                                                        super::close_the_tab();
                                                        not_opened.set(Some(why.to_string()));
                                                    }
                                                }
                                            });
                                        },
                                        Mark { agent: "github".to_owned(), size: 14 }
                                        "Install the App"
                                    }
                                }
                            },
                            if app.installations.is_empty() {
                                p { class: "py-2 text-sm text-muted-foreground",
                                    "Nowhere yet."
                                }
                            } else {
                                ul { class: "divide-y divide-border",
                                    for installation in app.installations.iter().cloned() {
                                        li { key: "{installation.id}",
                                            Installed { installation, onchanged }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if forgetting() {
                        Modal {
                            title: "Forget the App?",
                            onclose: move |()| forgetting.set(false),
                            actions: rsx! {
                                Button {
                                    variant: ButtonVariant::Danger,
                                    onclick: move |_| {
                                        spawn(async move {
                                            let outcome = forget_app("github".to_owned()).await;
                                            forgetting.set(false);
                                            onchanged.call(outcome);
                                        });
                                    },
                                    "Forget"
                                }
                            },
                            p { class: "text-sm text-muted-foreground",
                                "Its key is removed from this instance. The App itself stays \
                                 registered on GitHub until you delete it there. Refused while a \
                                 project is installed on through it."
                            }
                        }
                    }
                },
                None => rsx! { Registering {} },
            }
        }
    }
}

/// One installation of the App: the account, what it covers, which
/// projects reach their repository through it, and the way to forget it
/// where nothing does.
#[component]
fn Installed(
    installation: InstallationView,
    onchanged: EventHandler<DashboardResult<Apps>>,
) -> Element {
    let id = installation.id;
    let in_use = !installation.used_by.is_empty();
    let covers = if installation.every_repository {
        "all repositories"
    } else {
        "chosen repositories"
    };
    let used_by = installation.used_by.join(", ");
    let forgetting_says = if in_use {
        format!("Used by {used_by}, so it cannot be forgotten")
    } else {
        format!("Forget the installation on {}", installation.account)
    };

    rsx! {
        div { class: "flex items-center gap-3 py-2 first:pt-0 last:pb-0",
            Mark { agent: "github".to_owned(), size: 16 }
            span { class: "text-sm font-medium", "{installation.account}" }
            span { class: "text-xs text-muted-foreground", "{covers}" }
            if in_use {
                span { class: "text-xs text-muted-foreground", "used by {used_by}" }
            }
            span { class: "ml-auto flex items-center",
                Tooltip { text: forgetting_says.clone(), wrap: in_use, at_end: true,
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: BESIDE,
                        disabled: in_use,
                        aria_label: "{forgetting_says}",
                        onclick: move |_| {
                            spawn(async move {
                                onchanged.call(forget_installation("github".to_owned(), id).await);
                            });
                        },
                        {Icon::Remove.draw(16)}
                    }
                }
            }
        }
    }
}

/// The form that registers the App: one choice, then a press that leaves
/// for the platform.
#[component]
fn Registering() -> Element {
    let mut anywhere = use_signal(|| false);
    let form =
        use_resource(move || async move { registration("github".to_owned(), anywhere()).await });

    rsx! {
        div { class: "flex flex-col gap-3",
            Field {
                label: "Installable on",
                note: "A private App installs on the account that registered it and nowhere else.",
                info: "Choose any account when a repository lives under an organisation, or \
                       under somebody else's account: GitHub then lets anyone install the App, \
                       related to you or not. Installing it grants them nothing on its own, \
                       since only this instance holds the key and only a project here can use \
                       an installation.",
                Segmented {
                    label: "Installable on",
                    options: vec![
                        ("mine".to_owned(), "My account only".to_owned()),
                        ("any".to_owned(), "Any account".to_owned()),
                    ],
                    value: if anywhere() { "any".to_owned() } else { "mine".to_owned() },
                    onchange: move |chosen: String| anywhere.set(chosen == "any"),
                }
            }
            match form.read().as_ref() {
                Some(Ok(form)) => rsx! {
                    // A plain form, posted by the browser to the platform:
                    // the flow wants a navigation with the manifest as a
                    // field, and nothing here composes the address.
                    form { method: "post", action: "{form.action}",
                        input { r#type: "hidden", name: "manifest", value: "{form.manifest}" }
                        button {
                            r#type: "submit",
                            class: ButtonVariant::Primary.styled(""),
                            "Register the App on GitHub"
                        }
                    }
                },
                Some(Err(reason)) => rsx! {
                    p { role: "alert", class: "text-sm text-failed", "{reason}" }
                },
                None => rsx! {
                    EmptyState { title: "Preparing the form…" }
                },
            }
        }
    }
}

/// The Slack app: registered, with where it is installed and the way to
/// forget it; or the form to register one — see
/// `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
#[component]
fn SlackApp(
    app: Option<ChannelAppView>,
    form: String,
    onchanged: EventHandler<DashboardResult<Apps>>,
) -> Element {
    let mut forgetting = use_signal(|| false);

    rsx! {
        Card {
            title: "Slack app",
            note: "One app this instance owns, which a workspace installs so that a project can \
                   talk there without an app of its own.",
            info: "Created from Slack's form with the manifest filled in, then registered here by \
                   pasting its client ID, its client secret and an app-level token. The \
                   app-level token opens the one event stream every workspace's messages arrive \
                   on and never leaves this instance; both secrets stay sealed in the instance's \
                   file. A project speaks through a workspace of it, or through an app of its \
                   own, as before.",
            match app {
                Some(app) => rsx! {
                    div { class: "flex flex-col gap-4",
                        div { class: "flex items-center gap-3",
                            Mark { agent: "slack".to_owned(), size: 16 }
                            span { class: "text-sm text-muted-foreground", "Client ID" }
                            span { class: "font-mono text-sm", "{app.client_id}" }
                            span { class: "ml-auto flex items-center gap-2",
                                Button {
                                    variant: ButtonVariant::Danger,
                                    onclick: move |_| forgetting.set(true),
                                    "Forget…"
                                }
                            }
                        }
                        Field {
                            label: "Installed on",
                            note: "The workspaces the app is installed in, as Slack brought you back to say.",
                            if app.workspaces.is_empty() {
                                p { class: "py-2 text-sm text-muted-foreground",
                                    "Nowhere yet."
                                }
                            } else {
                                ul { class: "divide-y divide-border",
                                    for workspace in app.workspaces.iter().cloned() {
                                        li { key: "{workspace.id}",
                                            div { class: "flex items-center gap-3 py-2 first:pt-0 last:pb-0",
                                                Mark { agent: "slack".to_owned(), size: 16 }
                                                span { class: "text-sm font-medium", "{workspace.name}" }
                                                span { class: "font-mono text-xs text-muted-foreground", "{workspace.id}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if forgetting() {
                        Modal {
                            title: "Forget the Slack app?",
                            onclose: move |()| forgetting.set(false),
                            actions: rsx! {
                                Button {
                                    variant: ButtonVariant::Danger,
                                    onclick: move |_| {
                                        spawn(async move {
                                            let outcome = forget_channel_app("slack".to_owned()).await;
                                            forgetting.set(false);
                                            onchanged.call(outcome);
                                        });
                                    },
                                    "Forget"
                                }
                            },
                            p { class: "text-sm text-muted-foreground",
                                "Its secrets are removed from this instance. The app itself stays on \
                                 Slack until you delete it there."
                            }
                        }
                    }
                },
                None => rsx! { RegisteringSlack { form, onchanged } },
            }
        }
    }
}

/// The form that registers the Slack app: the guide onto the platform's
/// form, three boxes, and a press that asks the instance to check the
/// app-level token and keep the three.
#[component]
fn RegisteringSlack(form: String, onchanged: EventHandler<DashboardResult<Apps>>) -> Element {
    let mut client_id = use_signal(String::new);
    let mut client_secret = use_signal(String::new);
    let mut app_token = use_signal(String::new);
    let mut refused = use_signal(|| None::<DashboardError>);
    let complete = complete(&client_id(), &client_secret(), &app_token());
    let (beside_token, unplaced) = placed(refused().as_ref());

    rsx! {
        div { class: "flex flex-col gap-3",
            if let Some(why) = unplaced {
                p { role: "alert", class: "text-sm text-failed", "{why}" }
            }
            Field {
                label: "Client ID",
                note: "On the app's Basic Information page, under App Credentials.",
                aside: rsx! {
                    Guide {
                        mark: "slack",
                        label: "New app",
                        says: "Opens Slack's form with the app's manifest filled in, the address it \
                               brings you back to included. Create the app there, generate an \
                               app-level token under Basic Information with the connections:write \
                               scope, and paste the three values here.",
                        link: form,
                    }
                },
                input {
                    class: "{FIELD} font-mono",
                    placeholder: "1234567890.1234567890123",
                    value: "{client_id}",
                    oninput: move |event| client_id.set(event.value()),
                }
            }
            Field {
                label: "Client secret",
                note: "Beside the client ID. Kept sealed, and checked by the first install.",
                input {
                    r#type: "password",
                    class: "{FIELD} font-mono",
                    placeholder: "…",
                    value: "{client_secret}",
                    oninput: move |event| client_secret.set(event.value()),
                }
            }
            Field {
                label: "App-level token",
                note: "What listens. Starts with xapp, with the connections:write scope; checked \
                       against Slack before it is kept.",
                problem: beside_token,
                input {
                    r#type: "password",
                    class: "{FIELD} font-mono",
                    placeholder: "xapp-…",
                    value: "{app_token}",
                    oninput: move |event| app_token.set(event.value()),
                }
            }
            div {
                Button {
                    disabled: !complete,
                    onclick: move |_| {
                        spawn(async move {
                            match register_channel_app(
                                "slack".to_owned(),
                                client_id(),
                                client_secret(),
                                app_token(),
                            )
                            .await
                            {
                                Ok(fresh) => {
                                    refused.set(None);
                                    onchanged.call(Ok(fresh));
                                }
                                Err(why) => refused.set(Some(why)),
                            }
                        });
                    },
                    "Register the app"
                }
            }
        }
    }
}

/// Whether the three boxes of the registration form hold something to
/// send: what enables the press.
fn complete(client_id: &str, client_secret: &str, app_token: &str) -> bool {
    !client_id.trim().is_empty() && !client_secret.trim().is_empty() && !app_token.trim().is_empty()
}

/// Where a refusal is said on the registration form: beside the token's
/// box where it names the token, and above the form otherwise. Of the
/// three values, only the token has a check of its own, so only a refusal
/// of the token has a box to be said beside.
fn placed(refused: Option<&DashboardError>) -> (Option<String>, Option<String>) {
    let Some(why) = refused else {
        return (None, None);
    };
    let pointed = match why {
        DashboardError::Refused(refusal) => refusal.part(),
        DashboardError::NoInstance | DashboardError::Failed => None,
    };
    let said = Some(why.to_string());
    if pointed == Some(Part::Listening) {
        (said, None)
    } else {
        (None, said)
    }
}

#[cfg(test)]
mod tests {
    use super::{DashboardError, complete, placed};
    use stageman_wire::Refusal;

    /// The press waits for all three values, whichever is missing.
    #[test]
    fn the_press_waits_for_all_three_values() {
        assert!(complete("1234.5678", "s3cret", "xapp-1"));
        assert!(!complete(" ", "s3cret", "xapp-1"));
        assert!(!complete("1234.5678", "", "xapp-1"));
        assert!(!complete("1234.5678", "s3cret", "\t"));
    }

    /// A refusal of the app-level token is said beside its box; a refusal
    /// of anything else, and a failure that is nobody's box, above the
    /// form; and nothing is said of nothing.
    #[test]
    fn a_refusal_is_said_beside_the_token_where_it_names_it_and_above_otherwise() {
        let listening = DashboardError::from(Refusal::ChannelRefused {
            listening: true,
            why: "Slack refused it (invalid_auth)".to_owned(),
        });
        assert_eq!(
            placed(Some(&listening)),
            (
                Some(
                    "the app-level token was not kept: Slack refused it (invalid_auth)".to_owned()
                ),
                None
            )
        );
        let speaking = DashboardError::from(Refusal::ChannelRefused {
            listening: false,
            why: "Slack refused it (invalid_auth)".to_owned(),
        });
        assert_eq!(placed(Some(&speaking)), (None, Some(speaking.to_string())));
        let blank = DashboardError::from(Refusal::ChannelAppIncomplete);
        assert_eq!(placed(Some(&blank)), (None, Some(blank.to_string())));
        assert_eq!(
            placed(Some(&DashboardError::Failed)),
            (None, Some(DashboardError::Failed.to_string()))
        );
        assert_eq!(placed(None), (None, None));
    }
}
