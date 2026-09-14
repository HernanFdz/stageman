//! What an instance looks like from the outside: this machine, and what it is
//! watching.
//!
//! The screen somebody lands on, and the only one that says anything about the
//! machine rather than about the work.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::DashboardResult;
use crate::ui::{Badge, BadgeTone, Card, EmptyState};

pub use stageman_wire::Instance;

/// Everything the dashboard shows, read from the instance this process is
/// operating.
///
/// One route rather than one per pane, because there is one instance behind
/// all of it and splitting it would mean two reads that could disagree.
///
/// # Errors
///
/// Fails if this process is not operating an instance, which is a fault in
/// this process rather than in anything a request did.
#[get("/api/instance")]
pub async fn instance() -> DashboardResult<Instance> {
    match super::ask(Request::Instance).await? {
        Response::Instance(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// The instance screen.
///
/// [`use_server_future`] rather than `use_resource`, and the difference is the
/// whole reason this exists: it runs on the server during the render, ships
/// the answer with the page, and hands the client the same value rather than a
/// second request. So a page arrives with the instance already on it, and the
/// hydrated client agrees with the HTML it hydrated.
#[component]
pub fn InstanceView() -> Element {
    let reading = use_server_future(instance)?;

    rsx! {
        match reading.cloned() {
            Some(Ok(instance)) => rsx! { Summary { instance } },
            // Shown rather than logged. A blank page would send whoever hit it
            // to read the source.
            Some(Err(reason)) => rsx! {
                Card { title: "This instance could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            // Unreachable once the future above has resolved, and written out
            // rather than unwrapped because "unreachable" is a claim about
            // somebody else's code.
            None => rsx! {
                p { class: "text-sm text-muted-foreground", "Reading the instance…" }
            },
        }
    }
}

/// One instance, rendered.
///
/// Two cards, because an instance is two things an operator asks about
/// separately: what this machine can do, and what it is watching. Deliberately
/// dense — this is a console, not a landing page, and the thing being
/// optimised for is scanning several projects rather than admiring one.
#[component]
fn Summary(instance: Instance) -> Element {
    rsx! {
        div { class: "flex flex-col gap-4",
            Card {
                title: "This machine",
                note: "Found at startup, and not configurable — every agent runs in a container.",
                dl { class: "grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 text-sm",
                    dt { class: "text-muted-foreground", "runtime" }
                    dd { class: "font-mono text-xs", "{instance.container_runtime}" }
                    dt { class: "text-muted-foreground", "agents" }
                    dd { "{instance.agents}" }
                }
            }
            Card {
                title: "Projects",
                aside: rsx! {
                    Badge { "{instance.projects.len()}" }
                },
                if instance.projects.is_empty() {
                    EmptyState {
                        title: "Nothing is being watched yet.",
                        note: "A project needs an agent to think with and at least one its jobs \
                               can run on, so agents come first.",
                    }
                } else {
                    ul { class: "divide-y divide-border",
                        // Keyed by position rather than by name, because
                        // nothing makes a project's name unique — an operator
                        // types it. A duplicate key is not a warning in
                        // Dioxus, it is two list entries the renderer believes
                        // are the same one.
                        for (position , project) in instance.projects.iter().enumerate() {
                            li { key: "{position}", class: "flex items-baseline gap-3 py-2 first:pt-0 last:pb-0",
                                span { class: "text-sm font-medium", "{project.name}" }
                                span { class: "truncate font-mono text-xs text-faint-foreground",
                                    "{project.repository}"
                                }
                                span { class: "ml-auto shrink-0",
                                    if project.working > 0 {
                                        Badge { tone: BadgeTone::Working,
                                            "{project.working} of {project.jobs} working"
                                        }
                                    } else {
                                        Badge { "{project.jobs} job(s)" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
