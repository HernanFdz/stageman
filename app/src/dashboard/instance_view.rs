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

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::{DashboardError, DashboardResult};
use super::live::Live;
use crate::ui::{
    Button, ButtonVariant, Card, EmptyState, Field, Modal, Reference, Segmented, Skeleton,
};

pub use stageman_wire::{Apps, PlatformAppView, Registration};

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
                                 registered on GitHub until you delete it there."
                            }
                        }
                    }
                },
                None => rsx! { Registering {} },
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
