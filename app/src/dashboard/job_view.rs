//! One job's page: what it is, what it was told, and where it is talking
//! and showing.
//!
//! The page `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`
//! gives a job: its reason, the kit it runs on, what its session reported,
//! the instruction it began from, when it was made, and every reference as a
//! link where the link is true. It links to where the conversation is and
//! never carries one, which is what
//! `docs/decisions/0005-conversation-happens-on-channels.md` decided and
//! 0070 kept. Icons lead and words follow, per `docs/conventions.md` §3: a
//! reference that leaves the page is a mark with the address a hover away.
//! It is headed by the job's name, per
//! `docs/decisions/0074-a-jobs-identifier-is-its-name.md`; the reason and
//! the instruction are the kickoff, one card, with the instruction folded
//! away until asked for.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::{DashboardError, DashboardResult};
use super::jobs_view::{JobControls, Showing, Toned as _};
use super::live::Live;
use crate::ui::{Badge, Card, Chip, Icon, Info, KitChip, PageHeader, Reference, Skeleton, When};

pub use stageman_wire::JobPage;
use stageman_wire::{Standing, Working};

/// One job's page.
///
/// # Errors
///
/// Fails if the project or the job is unknown.
#[get("/api/projects/{project}/jobs/{job}")]
pub async fn job_page(project: String, job: String) -> DashboardResult<JobPage> {
    match super::ask(Request::Job { project, job }).await? {
        Response::Job(page) => Ok(*page),
        other => Err(super::unexpected(&other)),
    }
}

/// One job's screen.
#[component]
pub fn ProjectJobView(project: String, job: String) -> Element {
    // `use_reactive!` for the reason the project's screen gives: a plain
    // value is read once, and the screen would keep showing the first job
    // it was opened on.
    let live = use_context::<Live>();
    let reading = use_server_future(use_reactive!(|project, job| {
        let _ = live.follow();
        job_page(project, job)
    }))?;
    let failure = use_signal(|| None::<DashboardError>);

    rsx! {
        match reading.cloned() {
            Some(Ok(page)) => rsx! { Shown { page, failure } },
            Some(Err(reason)) => rsx! {
                Card { title: "This job could not be read",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            },
            None => rsx! { Skeleton {} },
        }
    }
}

/// The page, once read.
///
/// The header says which job and where it is talking and showing, and
/// stays in view over the kickoff. The summary says where the job has got
/// to, with the verbs on it, what it runs on, and when it was made. The
/// controls are the row's, so that a job is stopped and retired the same
/// way wherever it is met; the page is live through the tick, so it
/// re-reads itself once whatever a control did has landed, and only what a
/// control refused is kept here.
#[component]
fn Shown(page: JobPage, failure: Signal<Option<DashboardError>>) -> Element {
    let mut failure = failure;
    let job = page.job.clone();
    let reported = job
        .reported
        .iter()
        .map(|(option, value)| format!("{option} {value}"))
        .collect::<Vec<_>>()
        .join(" · ");

    rsx! {
        div { class: "flex flex-col gap-4",
            PageHeader {
                div { class: "flex items-center gap-2",
                    Link {
                        to: super::Route::ProjectJobsView { project: page.project.clone() },
                        class: "inline-flex items-center gap-1 text-sm text-muted-foreground \
                                hover:text-foreground hover:underline",
                        {Icon::Back.draw(14)}
                        "{page.project_name}"
                    }
                    Reference {
                        mark: "github",
                        says: page.repository.clone(),
                        link: page.repository_link.clone(),
                    }
                }
                div { class: "flex items-center gap-2",
                    // Its name, in the face identifiers wear here.
                    h1 { class: "font-mono text-base font-semibold", "{job.id}" }
                    // The room is a link while the channel has said where
                    // its workspace is, and its identifier otherwise.
                    if let Some(room) = job.room.clone() {
                        Reference {
                            mark: "slack",
                            says: "Its room, {room}",
                            link: job.room_link.clone(),
                        }
                    }
                    // Nothing can answer on the tunnel of a job that is
                    // over, so none is offered.
                    if !job.standing.is_over() {
                        Showing { tunnel: job.tunnel.clone() }
                    }
                }
            }
            if let Some(reason) = failure() {
                p { role: "alert", class: "text-sm text-failed", "{reason}" }
            }

            Card { title: "Summary",
                div { class: "flex flex-col gap-3",
                    // Where it has got to, with what a person does about it
                    // beside it.
                    Row { icon: Icon::Standing, says: "Where it has got to",
                        Badge { tone: job.standing.tone(), "{job.standing.label()}" }
                        // Since when; a job the last release wrote says
                        // only that it waits.
                        if let Some(since) = job.since.clone() {
                            When { at: since }
                        }
                        if let Standing::Failed { why } = &job.standing {
                            span { class: "text-sm text-failed", "{why}" }
                        }
                        JobControls {
                            project: page.project,
                            job: job.clone(),
                            onchanged: move |answered: Result<Working, DashboardError>| {
                                match answered {
                                    Ok(_) => failure.set(None),
                                    Err(reason) => failure.set(Some(reason)),
                                }
                            },
                        }
                    }
                    // What it runs on, and — a hover away — what the session
                    // said it was set to, in the adapter's spelling: the one
                    // case worth seeing is the two disagreeing.
                    Row { icon: Icon::Kit, says: "What it runs on",
                        KitChip {
                            agent: job.kit.agent.clone(),
                            agent_name: job.kit.agent_name.clone(),
                            model: job.kit.model.clone(),
                            effort: job.kit.effort.clone(),
                        }
                        if !reported.is_empty() {
                            Info { text: "Its session reported: {reported}" }
                        }
                    }
                    Row { icon: Icon::Made, says: "When it was made",
                        When { at: job.created_at.clone() }
                    }
                    // Every pull request it ever said it opened, by number,
                    // linked where the repository is an address; whether
                    // any is still open is the platform's to say.
                    if !job.pull_requests.is_empty() {
                        Row { icon: Icon::PullRequest, says: "The pull requests it opened",
                            for opened in job.pull_requests.iter() {
                                PullRequestChip { key: "{opened.number}", number: opened.number, link: opened.link.clone() }
                            }
                        }
                    }
                }
            }

            // Why it was started and what its agent was told, together: the
            // reason is a paragraph a person reads, and the instruction is
            // the whole record of what was asked, folded until asked for.
            // The trigger is where the instruction appears, under the
            // reason, and says what it opens; a disclosure rather than a
            // script, so it works before the page wakes and from the
            // keyboard.
            Card {
                title: "Kickoff",
                note: "Why it was started, and what its agent began from.",
                div { class: "flex flex-col gap-3",
                    p { class: "text-sm", "{job.reason}" }
                    details { class: "group",
                        summary {
                            class: "flex w-fit cursor-pointer list-none items-center gap-1.5 rounded-md text-xs \
                                    font-medium text-muted-foreground hover:text-foreground \
                                    focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary \
                                    focus-visible:ring-offset-2 focus-visible:ring-offset-background \
                                    [&::-webkit-details-marker]:hidden",
                            span {
                                class: "inline-flex motion-safe:transition-transform group-open:rotate-90",
                                aria_hidden: "true",
                                {Icon::Disclose.draw(14)}
                            }
                            "The whole instruction"
                        }
                        pre { class: "mt-3 max-h-[32rem] overflow-auto whitespace-pre-wrap rounded-md bg-surface-muted p-3 font-mono text-xs text-muted-foreground",
                            "{job.kickoff}"
                        }
                    }
                }
            }
        }
    }
}

/// One pull request, as a chip: the number, going where it is.
#[component]
pub(super) fn PullRequestChip(number: u64, link: Option<String>) -> Element {
    let says = link
        .clone()
        .unwrap_or_else(|| format!("Pull request {number}"));
    rsx! {
        Chip { says, link,
            {Icon::PullRequest.draw(12)}
            "#{number}"
        }
    }
}

/// One line of the summary: an icon saying what, and the thing itself.
#[component]
fn Row(icon: Icon, says: String, children: Element) -> Element {
    rsx! {
        div { class: "flex items-center gap-3",
            span {
                class: "inline-flex shrink-0 items-center text-muted-foreground",
                role: "img",
                aria_label: "{says}",
                title: "{says}",
                {icon.draw(16)}
            }
            div { class: "flex min-w-0 flex-wrap items-center gap-2", {children} }
        }
    }
}
