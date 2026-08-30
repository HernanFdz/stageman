//! What an instance looks like from the outside: this machine, and what it is
//! watching.
//!
//! The screen somebody lands on, and the only one that says anything about the
//! machine rather than about the work.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use dioxus::server::axum::Extension;
use serde::{Deserialize, Serialize};

use super::Project;
use super::error::DashboardResult;
use crate::ui::{Badge, BadgeTone, Card, EmptyState};

/// One instance, as much of it as a page is allowed to know.
///
/// Counts and names, and nothing that could be a credential. That is a
/// property of this type rather than of the function below: a field added here
/// is a field the browser gets, and there is no second place to check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// Where this machine's container runtime was found.
    ///
    /// A path rather than a version, because the operator's next question when
    /// something misbehaves is *which one is it using*. Not optional: nothing
    /// gets far enough to serve this page without one, per
    /// `docs/decisions/0023-the-container-runtime-is-discovered-once.md`.
    ///
    /// It comes from the machine rather than from the instance, so it is the
    /// one field here that is not derived from the state below it — and it is
    /// a `String` rather than an `Option<String>` because by the time anything
    /// can ask, a runtime has been found and proved to answer.
    pub container_runtime: String,
    /// How many agents are configured.
    ///
    /// A count and not a list, because an agent's configuration is a
    /// credential and listing them is the agents view's problem rather than
    /// this one's.
    pub agents: usize,
    /// The projects this instance watches.
    pub projects: Vec<Project>,
}

/// Everything the dashboard shows, read from the instance this process is
/// operating.
///
/// One route rather than one per pane, because there is one snapshot behind
/// all of it and splitting it would mean two reads that could disagree.
///
/// The instance arrives as an `axum` extension, declared in the attribute
/// rather than in the signature: what the macro adds there exists on the
/// server and is absent from what the client calls. That is deliberate over
/// the alternative — `ServeConfig`'s context providers reach the virtual DOM
/// and so are present while a page is rendered and *missing* when the client
/// calls the same route afterwards. One mechanism that works on both paths
/// beats two that each work on one, and the failure mode of getting this wrong
/// is a route that passes every server-rendering test and fails the first time
/// a browser calls it.
///
/// # Errors
///
/// Fails if the server was assembled without an instance behind it, which is a
/// fault in this process rather than in anything a request did.
// Required by the macro and unused by this body, which reads a lock rather
// than waiting for anything. Scoped to the feature because the client's half
// of this function *is* full of awaits, so an unconditional expectation would
// go unfulfilled there — and `unfulfilled_lint_expectations` is denied.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[get("/api/instance", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn instance() -> DashboardResult<Instance> {
    Ok(Instance::of(&instance.0.read()))
}

#[cfg(feature = "server")]
impl Instance {
    /// What to show, for this instance on this machine.
    ///
    /// Deliberately not a `From` implementation, which it was until the
    /// runtime stopped being part of the state: `From` promises a function of
    /// its input, and this reads a process-wide discovery as well — see
    /// `docs/decisions/0023-the-container-runtime-is-discovered-once.md`. A
    /// trait implementation that quietly depends on a global is the kind of
    /// thing that reads correctly and is wrong.
    ///
    /// Everything the browser is shown is assembled here, in one place that
    /// can be read as a list. On the server only: the domain type it reads
    /// from is not compiled for the browser at all, which is the point.
    fn of(state: &stageman_core::State) -> Self {
        Self {
            container_runtime: crate::RUNTIME.path().display().to_string(),
            agents: state.agents.len(),
            // The same projection the projects screen reads, rather than a
            // second one. This screen shows fewer of its fields; that is a
            // decision about the view and not a reason for another type.
            projects: super::watching(state),
        }
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
