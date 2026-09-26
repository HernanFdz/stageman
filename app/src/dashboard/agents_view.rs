//! The agents an instance can run, and what they authenticate with.
//!
//! First of the configuring screens, because nothing else can be configured
//! until one agent exists: a project names one agent for its foreman and
//! a non-empty set its jobs may use, per
//! `docs/decisions/0021-an-instance-starts-empty.md`. An instance with no
//! agents is not broken, it is new — and this is the screen that ends that.
//!
//! **A credential travels one way.** It is sent here and never sent back:
//! nothing on this page carries one, and [`Agent`] has nowhere to put one,
//! which is the invariant in `docs/architecture.md` §2 expressed as a type
//! rather than as care.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::{DashboardError, DashboardResult};
use super::live::{Live, Reading, use_reading};
use crate::ui::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState, Skeleton};

pub use stageman_wire::Agent;

/// Every agent, whether or not it is configured.
///
/// The whole set rather than the configured subset: this screen's job is to
/// let somebody configure one, and a list that hides what is missing cannot.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[get("/api/agents")]
pub async fn agents() -> DashboardResult<Vec<Agent>> {
    match super::ask(Request::Agents).await? {
        Response::Agents(listing) => Ok(listing),
        other => Err(super::unexpected(&other)),
    }
}

/// Gives an agent a credential, or replaces the one it has.
///
/// Replacing rather than refusing when one already exists, because rotating a
/// credential is the ordinary reason to come back to this screen and a
/// separate verb for it would be ceremony.
///
/// # Errors
///
/// Fails if the agent is not one this instance can run, or if the credential
/// is empty.
#[post("/api/agents/configure")]
pub async fn configure(agent: String, credential: String) -> DashboardResult<Vec<Agent>> {
    match super::ask(Request::Configure { agent, credential }).await? {
        Response::Agents(listing) => Ok(listing),
        other => Err(super::unexpected(&other)),
    }
}

/// Removes an agent's credential, if nothing depends on it.
///
/// # Errors
///
/// Fails if the agent is not one this instance can run, or if a project still
/// names it.
#[post("/api/agents/forget")]
pub async fn forget(agent: String) -> DashboardResult<Vec<Agent>> {
    match super::ask(Request::ForgetAgent { agent }).await? {
        Response::Agents(listing) => Ok(listing),
        other => Err(super::unexpected(&other)),
    }
}

/// The agents screen.
#[component]
pub fn AgentsView() -> Element {
    let live = use_context::<Live>();
    let reading = use_reading(live, agents)?;
    let mut failure = use_signal(|| None::<DashboardError>);

    rsx! {
        div { class: "flex flex-col gap-4",
            if let Some(reason) = failure() {
                Card { title: "That did not work",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            }
            match reading {
                Reading::Read(mut listing) => {
                    let agents = listing();
                    rsx! {
                        Card {
                            title: "Agents",
                            note: "An agent needs a credential before a project can name it.",
                            badge: rsx! {
                                Badge { "{agents.iter().filter(|agent| agent.configured).count()} of {agents.len()}" }
                            },
                            if agents.is_empty() {
                                EmptyState {
                                    title: "This build can run no agents at all.",
                                    note: "The set is compiled in, so this is a build problem rather \
                                           than something to configure.",
                                }
                            } else {
                                ul { class: "divide-y divide-border",
                                    for agent in agents {
                                        li { key: "{agent.id}",
                                            AgentRow {
                                                agent,
                                                onchanged: move |outcome: DashboardResult<Vec<Agent>>| {
                                                    match outcome {
                                                        Ok(fresh) => {
                                                            failure.set(None);
                                                            listing.set(fresh);
                                                        }
                                                        Err(reason) => failure.set(Some(reason)),
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Reading::Failed(reason) => rsx! {
                    Card { title: "The agents could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                Reading::NotYet => rsx! { Skeleton {} },
            }
        }
    }
}

/// One agent, with whatever it is currently possible to do to it.
#[component]
fn AgentRow(agent: Agent, onchanged: EventHandler<DashboardResult<Vec<Agent>>>) -> Element {
    let mut credential = use_signal(String::new);
    let removable = agent.used_by.is_empty();
    let identifier = agent.id.clone();

    rsx! {
        div { class: "flex flex-col gap-2 py-3 first:pt-0 last:pb-0",
            div { class: "flex items-baseline gap-3",
                span { class: "text-sm font-medium", "{agent.name}" }
                if agent.configured {
                    Badge { tone: BadgeTone::Idle, "configured" }
                } else {
                    Badge { "no credential" }
                }
                if !agent.used_by.is_empty() {
                    span { class: "ml-auto shrink-0 text-xs text-muted-foreground",
                        "used by {agent.used_by.join(\", \")}"
                    }
                }
            }
            p { class: "max-w-prose text-xs text-muted-foreground", "{agent.description}" }
            div { class: "flex items-center gap-2",
                input {
                    r#type: "password",
                    class: "w-full max-w-sm rounded-md border border-border bg-surface px-2 py-1.5 \
                            font-mono text-xs placeholder:text-faint-foreground focus-visible:outline-none \
                            focus-visible:ring-2 focus-visible:ring-primary",
                    placeholder: if agent.configured { "replace the credential" } else { "paste a credential" },
                    value: "{credential}",
                    oninput: move |event| credential.set(event.value()),
                }
                Button {
                    onclick: {
                        let identifier = identifier.clone();
                        move |_| {
                            let identifier = identifier.clone();
                            let supplied = credential();
                            async move {
                                let outcome = configure(identifier, supplied).await;
                                if outcome.is_ok() {
                                    credential.set(String::new());
                                }
                                onchanged.call(outcome);
                            }
                        }
                    },
                    "Save"
                }
                if agent.configured {
                    Button {
                        variant: ButtonVariant::Danger,
                        disabled: !removable,
                        onclick: move |_| {
                            let identifier = identifier.clone();
                            async move { onchanged.call(forget(identifier).await) }
                        },
                        "Forget"
                    }
                }
            }
        }
    }
}
