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

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::{DashboardError, DashboardResult};
use super::live::Live;
use crate::ui::{
    BESIDE, Button, ButtonVariant, Card, EmptyState, Field, Icon, Mark, Modal, Reference,
    Segmented, Skeleton, Tooltip,
};

pub use stageman_wire::{Apps, InstallLink, InstallationView, PlatformAppView, Registration};

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
