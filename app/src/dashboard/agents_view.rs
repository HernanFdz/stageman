//! The agents an instance can run, and what they authenticate with.
//!
//! First of the configuring screens, because nothing else can be configured
//! until one agent exists: a project names one agent for its foreman and
//! a non-empty set its jobs may use, per
//! `docs/decisions/0021-an-instance-starts-empty.md`. An instance with no
//! agents is not broken, it is new — and this is the screen that ends that.
//!
//! **A credential travels one way.** It is sent here and never sent back:
//! nothing on this page carries one, and [`Agent`] below has nowhere to put
//! one, which is the invariant in `docs/architecture.md` §2 expressed as a
//! type rather than as care.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use dioxus::server::axum::Extension;
use serde::{Deserialize, Serialize};

use super::error::{DashboardError, DashboardResult};
use crate::ui::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState};

/// One agent this instance could run, as much of it as a page may know.
///
/// Note what is absent and cannot be added: the credential. This type is the
/// enforcement, not the route below it.
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
    /// The projects that would break if it were forgotten.
    ///
    /// Empty means it can go. Anything else is what a refusal would say, and
    /// carrying it here means the page can grey the button *and* explain,
    /// rather than letting somebody find out by pressing it.
    pub used_by: Vec<String>,
}

/// Every agent, whether or not it is configured.
///
/// The whole set rather than the configured subset: this screen's job is to
/// let somebody configure one, and a list that hides what is missing cannot.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[get("/api/agents", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn agents() -> DashboardResult<Vec<Agent>> {
    Ok(super::listed(&instance.0.read()))
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
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/agents/configure", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn configure(agent: String, credential: String) -> DashboardResult<Vec<Agent>> {
    let named = super::named(&agent)?;
    let credential = credential.trim();
    if credential.is_empty() {
        return Err(DashboardError::CredentialMissing);
    }

    let mut state = instance.0.update();
    state.agents.insert(
        named,
        stageman_core::AgentConfig {
            auth_token: stageman_core::Secret::new(credential.to_owned()),
        },
    );
    let listing = super::listed(&state);
    // Explicitly, because releasing this guard is what writes the snapshot —
    // the borrow ending *is* the save, and leaving it to the end of the
    // function would put the most consequential line of this route in the one
    // place nobody reads.
    drop(state);

    Ok(listing)
}

/// Removes an agent's credential, if nothing depends on it.
///
/// The check happens **before** the change and not after, which matters more
/// than it looks: the store validates on write and logs a refusal rather than
/// returning it, so mutating first would leave an instance that is invalid in
/// memory and correct on disk. Asking `used_by` first is what
/// `docs/decisions/0021-an-instance-starts-empty.md` intends by it.
///
/// # Errors
///
/// Fails if the agent is not one this instance can run, or if a project still
/// names it.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/agents/forget", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn forget(agent: String) -> DashboardResult<Vec<Agent>> {
    let named = super::named(&agent)?;

    // Checked while holding the guard that would write, so that nothing can
    // start depending on this agent between the question and the answer.
    let mut state = instance.0.update();
    let dependents = super::dependents(&state, named);
    if !dependents.is_empty() {
        drop(state);
        return Err(DashboardError::AgentInUse {
            agent,
            projects: dependents,
        });
    }

    state.agents.remove(&named);
    let listing = super::listed(&state);
    drop(state);

    Ok(listing)
}

/// The agents screen.
#[component]
pub fn AgentsView() -> Element {
    let mut listing = use_server_future(agents)?;
    let mut failure = use_signal(|| None::<DashboardError>);

    rsx! {
        div { class: "flex flex-col gap-4",
            if let Some(reason) = failure() {
                Card { title: "That did not work",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            }
            match listing.cloned() {
                Some(Ok(agents)) => rsx! {
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
                                                        listing.set(Some(Ok(fresh)));
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
                },
                Some(Err(reason)) => rsx! {
                    Card { title: "The agents could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                None => rsx! {
                    p { class: "text-sm text-muted-foreground", "Reading the agents…" }
                },
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
                    Badge { tone: BadgeTone::Completed, "configured" }
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
