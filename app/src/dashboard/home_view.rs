//! The first page: what needs a person, what is working, and the projects.
//!
//! The page an operator lands on, and the one question it exists to answer
//! is *what needs me?* — see
//! `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`. The
//! set it shows first is the domain's *idle*, which
//! `docs/decisions/0052-a-jobs-state-says-what-somebody-does-about-it.md`
//! defined as the state a person does something about, with the reading
//! saying what. So each row carries the verb its reading wants, and nothing
//! here decides anything: the instance partitions and orders, and this draws.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::DashboardResult;
use super::jobs_view::{JobRow, ROW, ROWS};
use super::live::Live;
use crate::ui::{
    Badge, BadgeTone, Card, EmptyState, Icon, Mark, Reference, Skeleton, Tooltip, When,
};

pub use stageman_wire::{ExpiringToken, Home, Project, ProjectJob};

/// Everything the first page shows, read from the instance this process is
/// operating.
///
/// One route rather than one per region, because there is one instance
/// behind all of it and three reads could disagree.
///
/// # Errors
///
/// Fails if this process is not operating an instance, which is a fault in
/// this process rather than in anything a request did.
#[get("/api/home")]
pub async fn home() -> DashboardResult<Home> {
    match super::ask(Request::Home).await? {
        Response::Home(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// The first page.
///
/// [`use_server_future`] rather than `use_resource`, and the difference is the
/// whole reason this exists: it runs on the server during the render, ships
/// the answer with the page, and hands the client the same value rather than a
/// second request. So a page arrives with everything already on it, and the
/// hydrated client agrees with the HTML it hydrated.
#[component]
pub fn HomeView() -> Element {
    let live = use_context::<Live>();
    let reading = use_server_future(move || {
        let _ = live.follow();
        home()
    })?;

    rsx! {
        match reading.cloned() {
            Some(Ok(home)) => rsx! { Overview { home } },
            // Shown rather than logged. A blank page would send whoever hit it
            // to read the source.
            Some(Err(reason)) => rsx! {
                Card { title: "This instance could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            None => rsx! { Skeleton {} },
        }
    }
}

/// The three regions, in the order the record gives them.
#[component]
fn Overview(home: Home) -> Element {
    rsx! {
        div { class: "flex flex-col gap-4",
            Card {
                title: "Needs you",
                note: "Tokens about to stop working, then jobs waiting on a person, longest waiting first.",
                badge: rsx! { Badge { "{home.needs_you.len() + home.expiring.len()}" } },
                if home.needs_you.is_empty() && home.expiring.is_empty() {
                    EmptyState {
                        title: "Nothing needs you.",
                        note: "A job that asks, proposes, fails or is paused lands here, with \
                               what to do about it; so does a token a week from expiring.",
                    }
                } else {
                    ul { class: ROWS,
                        // A token about to expire is raised before the jobs,
                        // because every job on its project stops with it —
                        // see `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
                        for token in home.expiring {
                            li { key: "token-{token.project}", class: ROW,
                                Expiring { token }
                            }
                        }
                        for placed in home.needs_you {
                            li { key: "{placed.job.id}", class: ROW,
                                Placed { placed }
                            }
                        }
                    }
                }
            }
            Card {
                title: "Working now",
                badge: rsx! { Badge { "{home.working.len()}" } },
                if home.working.is_empty() {
                    EmptyState {
                        title: "Nothing is working.",
                        note: "A job is listed here while its agent has something to do, \
                               whether a foreman or you started it.",
                    }
                } else {
                    ul { class: ROWS,
                        for placed in home.working {
                            li { key: "{placed.job.id}", class: ROW,
                                Placed { placed }
                            }
                        }
                    }
                }
            }
            Card {
                title: "Projects",
                badge: rsx! { Badge { "{home.projects.len()}" } },
                if home.projects.is_empty() {
                    // The checklist a new instance needs, in the order
                    // `docs/decisions/0021-an-instance-starts-empty.md`
                    // requires: nothing can be named before it exists.
                    EmptyState {
                        title: "Nothing is being watched yet.",
                        note: "Four steps, in this order; the second is optional.",
                        action: rsx! {
                            ol { class: "list-decimal space-y-1 pl-5 text-xs text-muted-foreground",
                                li {
                                    Link { to: super::Route::AgentsView {}, class: "underline", "Give an agent a credential" }
                                    ", so a project has something to think with."
                                }
                                li {
                                    Link { to: super::Route::InstanceView {}, class: "underline", "Register the App on GitHub" }
                                    " and install it where your repositories are, if you would rather not paste a token."
                                }
                                li {
                                    Link { to: super::Route::ProjectsView {}, class: "underline", "Add a project" }
                                    ": how it reaches GitHub, the repository chosen from what that reaches, and the Slack app it talks through."
                                }
                                li { "Invite the app into a channel, and mention it." }
                            }
                        },
                    }
                } else {
                    ul { class: "divide-y divide-border",
                        for project in home.projects {
                            li { key: "{project.id}", Watched { project } }
                        }
                    }
                }
            }
        }
    }
}

/// One token about to stop working, or stopped: whose it is, on which
/// project, when, and the way to the page it is replaced on.
#[component]
fn Expiring(token: ExpiringToken) -> Element {
    let ExpiringToken {
        project,
        project_name,
        owner,
        expires,
        expired,
    } = token;
    let whose = owner.map_or_else(
        || "its token".to_owned(),
        |owner| format!("{owner}'s token"),
    );
    let verb = if expired { "expired" } else { "expires" };

    rsx! {
        div { class: "flex items-center gap-3",
            Badge { tone: if expired { BadgeTone::Failed } else { BadgeTone::Idle },
                if expired { "expired" } else { "expiring" }
            }
            Mark { agent: "github".to_owned(), size: 16 }
            span { class: "text-sm",
                span { class: "font-medium", "{project_name}" }
                ": {whose} {verb} "
                When { at: expires, ahead: true, class: "text-sm".to_owned() }
            }
            span { class: "ml-auto",
                Link {
                    to: super::Route::ProjectSettingsView { project },
                    class: "text-sm underline decoration-border-strong underline-offset-2 hover:decoration-foreground",
                    "Replace it"
                }
            }
        }
    }
}

/// One job on the first page: the row every list draws, with its project
/// before its name and the verb its standing wants at the end.
///
/// The verb leads to the job's page, where its controls, its instruction
/// and its links are; Home finds, and the page acts.
#[component]
fn Placed(placed: ProjectJob) -> Element {
    let ProjectJob {
        project,
        project_name,
        job,
    } = placed;
    let to_job = super::Route::ProjectJobView {
        project: project.clone(),
        job: job.id.clone(),
    };
    let verb = job.standing.asks();

    rsx! {
        JobRow {
            job,
            project,
            project_name: Some(project_name),
            aside: rsx! {
                if let Some(verb) = verb {
                    Link {
                        to: to_job,
                        class: "text-xs font-medium text-primary hover:underline",
                        "{verb}"
                    }
                }
            },
        }
    }
}

/// One project on the first page: where it is, how much it has going, and
/// whether its foreman is on something.
#[component]
fn Watched(project: Project) -> Element {
    rsx! {
        div { class: "flex items-baseline gap-3 py-2 first:pt-0 last:pb-0",
            Link {
                to: super::Route::ProjectJobsView { project: project.id.clone() },
                class: "text-sm font-medium hover:underline",
                "{project.name}"
            }
            Reference {
                mark: "github",
                says: project.repository.to_string(),
                link: Some(project.repository_link.clone()),
            }
            span { class: "ml-auto flex shrink-0 items-center gap-2",
                if project.attending {
                    Tooltip { text: "The foreman is on a message",
                        Badge { tone: BadgeTone::Working,
                            {Icon::Foreman.draw(12)}
                            "foreman"
                        }
                    }
                }
                if project.working > 0 {
                    Badge { tone: BadgeTone::Working, "{project.working} of {project.jobs} working" }
                } else {
                    Badge { "{project.jobs} job(s)" }
                }
            }
        }
    }
}
