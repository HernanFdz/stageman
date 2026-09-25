//! The projects an instance watches, and what each one needs in order to work.
//!
//! The screen that makes an instance able to *do* something. It comes after
//! agents because a project names one agent for its foreman and a
//! non-empty set its jobs may use, and both have to be configured before a
//! project may name them — `docs/decisions/0021-an-instance-starts-empty.md`.
//!
//! This is the list. What a project *is* is decided on its settings page,
//! which is where a new one is made too — see
//! `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`. The
//! routes that make and change one live here beside the one that lists them,
//! because they answer with the same listing.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::agents_view::Agent;
use super::error::DashboardResult;
use super::live::{Live, Reading, use_reading};
use crate::ui::{
    Badge, BadgeTone, ButtonVariant, Card, EmptyState, Icon, KitChip, Reference, Skeleton, Tooltip,
};

pub use stageman_wire::{
    Bound, ChannelDraft, Choice, Draft, Fitted, KitDraft, ModelChoice, Project, Reached, Shape,
    Through, Watching, WorkspaceArrival,
};

/// Everything the projects screen shows.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[get("/api/projects")]
pub async fn projects() -> DashboardResult<Watching> {
    match super::ask(Request::Projects).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// Starts watching a repository.
///
/// # Errors
///
/// Fails if anything required is missing, if the repository is not an
/// address on the platform, if a kit describes settings this build does not
/// know, if the channel is half given, or if the instance would not be
/// consistent with this project in it.
#[post("/api/projects/create")]
pub async fn create(draft: Draft) -> DashboardResult<Watching> {
    match super::ask(Request::Create { draft }).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// Changes what a project is, leaving what it has done alone.
///
/// A blank credential means the one it already has, never none: there is
/// nowhere on the wire for the current value, so the box always starts empty.
/// The channel is not offered at all, for the same reason.
///
/// # Errors
///
/// Fails as [`create`] does, and if nothing is watched under that identifier.
#[post("/api/projects/amend")]
pub async fn amend(project: String, draft: Draft) -> DashboardResult<Watching> {
    match super::ask(Request::Amend { project, draft }).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// What an access reaches: the repositories the form chooses from, listed
/// from the platform and kept nowhere — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
///
/// # Errors
///
/// Fails if the platform does not accept the token, could not be asked, or
/// if no App is registered where the App was asked about.
#[post("/api/projects/reaches")]
pub async fn reaches(through: Through) -> DashboardResult<Reached> {
    match super::ask(Request::Reaches { through }).await? {
        Response::Reached(reached) => Ok(reached),
        other => Err(super::unexpected(&other)),
    }
}

/// Checks a pair of tokens for an app of a project's own, for the form's
/// panel, and keeps nothing: where the app speaks, once Slack has accepted
/// both — see `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
///
/// # Errors
///
/// Fails if either token is blank, if Slack does not accept one, if Slack
/// could not be asked, or if the pair is an app another project already
/// speaks through.
#[post("/api/projects/binds")]
pub async fn binds(project: Option<String>, binding: ChannelDraft) -> DashboardResult<Bound> {
    match super::ask(Request::Binds { project, binding }).await? {
        Response::Bound(bound) => Ok(bound),
        other => Err(super::unexpected(&other)),
    }
}

/// Whether the workspace a form's tab went out to install on has come
/// back under the state the tab carried, and which it is.
///
/// # Errors
///
/// Fails if no install was begun under that state, or if the workspace
/// that came back is no longer held.
#[post("/api/projects/workspace-arrival")]
pub async fn workspace_arrival(
    channel: String,
    state: String,
) -> DashboardResult<WorkspaceArrival> {
    match super::ask(Request::WorkspaceArrived { channel, state }).await? {
        Response::WorkspaceArrival(arrival) => Ok(arrival),
        other => Err(super::unexpected(&other)),
    }
}

/// Stops watching a repository, and reclaims everything it was holding.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier, or if any of its jobs
/// is still working.
#[post("/api/projects/forget")]
pub async fn forget(project: String) -> DashboardResult<Watching> {
    match super::ask(Request::Forget { project }).await? {
        Response::Projects(watching) => Ok(watching),
        other => Err(super::unexpected(&other)),
    }
}

/// What a project's row notes under its name: absence for the two things a
/// project cannot work without, and what the foreman watches — a watched
/// room by the platform's identifier, because a name costs a scope the
/// manifest does not grant. A function rather than a paragraph in the
/// component, so that each condition is tested once.
fn noted(project: &Project) -> String {
    let mut notes = Vec::new();
    if project.access.is_none() {
        notes.push("no access to the repository".to_owned());
    }
    if project.binding.is_none() {
        notes.push("no Slack binding".to_owned());
    }
    if !project.watched.is_empty() {
        notes.push(format!("watching {}", project.watched.join(", ")));
    }
    notes.join(" · ")
}

/// The projects screen.
#[component]
pub fn ProjectsView() -> Element {
    let live = use_context::<Live>();
    let reading = use_reading(live, projects)?;

    rsx! {
        div { class: "flex flex-col gap-4",
            match reading {
                Reading::Read(read) => {
                    let watching = read();
                    rsx! {
                        Card {
                            title: "Projects",
                            note: "A repository, the agents that work on it, and what they need to reach it.",
                            badge: rsx! {
                                Badge { "{watching.projects.len()}" }
                            },
                            // Offered only once an agent could be named: a page
                            // that let somebody fill a form the instance must
                            // refuse would be inviting the refusal.
                            aside: if watching.available.is_empty() { None } else {
                                Some(rsx! {
                                    Tooltip { text: "New project",
                                        Link {
                                            to: super::Route::ProjectNewView {},
                                            class: ButtonVariant::Primary.styled("px-2"),
                                            aria_label: "New project",
                                            {Icon::Add.draw(16)}
                                        }
                                    }
                                })
                            },
                            if watching.projects.is_empty() {
                                EmptyState {
                                    title: "Nothing is being watched yet.",
                                    note: if watching.available.is_empty() {
                                        "A project names one agent to think with and at least one its \
                                         jobs run on, so configuring an agent comes first."
                                    } else {
                                        "Add one. It needs the agents that work on it, a way to reach \
                                         GitHub, and the repository chosen from what that reaches."
                                    },
                                }
                            } else {
                                ul { class: "divide-y divide-border",
                                    for project in watching.projects.iter().cloned() {
                                        li { key: "{project.id}",
                                            WatchedProject {
                                                project,
                                                available: watching.available.clone(),
                                                shapes: watching.shapes.clone(),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Reading::Failed(reason) => rsx! {
                    Card { title: "The projects could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                Reading::NotYet => rsx! { Skeleton {} },
            }
        }
    }
}

/// What to show for an agent the browser named by its identifier.
///
/// A function rather than a closure inside the row, so that it can be asserted:
/// mutation testing found the comparison inside it unguarded, which is fair —
/// a project carries identifiers and the row shows names, and nothing else
/// notices if the two stop lining up.
///
/// An identifier this build does not know is shown as it stands rather than
/// hidden. An instance naming an agent that is gone is worth seeing, and
/// `State::check` refuses one anyway.
fn shown_as(available: &[Agent], identifier: &str) -> String {
    available
        .iter()
        .find(|agent| agent.id == identifier)
        .map_or_else(|| identifier.to_owned(), |agent| agent.name.clone())
}

/// What a fitted agent reads as: the model's name, and the effort's
/// spelling and name where the model takes one — from the shape the server
/// sent, or the identifiers as they stand where it sent none.
pub(super) fn read_as(shapes: &[Shape], fitted: &Fitted) -> (String, Option<(String, String)>) {
    let shape = shapes.iter().find(|shape| shape.agent == fitted.agent);
    let model = shape
        .and_then(|shape| shape.models.iter().find(|model| model.id == fitted.model))
        .map_or_else(|| fitted.model.clone(), |model| model.name.clone());
    let effort = (!fitted.effort.is_empty()).then(|| {
        let name = shape
            .and_then(|shape| {
                shape
                    .efforts
                    .iter()
                    .find(|effort| effort.id == fitted.effort)
            })
            .map_or_else(|| fitted.effort.clone(), |effort| effort.name.clone());
        (fitted.effort.clone(), name)
    });
    (model, effort)
}

/// One project, as the list shows it.
///
/// It shows and it links; it never edits. What a project *is* stays decided
/// in one place, its settings page, and a row that edited in place would be a
/// second place, disagreeing about which fields matter and which are required.
///
/// It takes the available agents and their shapes in order to *render*: a
/// project carries the identifiers a browser sends back, and this is where
/// they become the names a person reads and the chips a person scans.
#[component]
fn WatchedProject(project: Project, available: Vec<Agent>, shapes: Vec<Shape>) -> Element {
    let (foreman_model, foreman_effort) = read_as(&shapes, &project.foreman);
    let notes = noted(&project);
    // The variables a hover away, by name and by what each is for, and
    // never a value.
    let variables = project
        .variables
        .iter()
        .map(|variable| {
            if variable.note.is_empty() {
                variable.name.clone()
            } else {
                format!("{} — {}", variable.name, variable.note)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let named = project
        .variables
        .iter()
        .map(|variable| variable.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let counted = format!("{} variable(s)", project.variables.len());

    rsx! {
        // Roomier than the rows on the agents screen, and deliberately: an
        // agent is one line and a project is three, so the same padding reads
        // as cramped here.
        //
        // The first and last shed their outer padding entirely, so the space
        // above the first row and below the last are both the card's own and
        // therefore equal. Anything else makes the top gap the sum of two
        // paddings and the eye reads it as a mistake.
        div { class: "flex flex-col gap-2 py-4 first:pt-0 last:pb-0",
            div { class: "flex items-center gap-3",
                Link {
                    to: super::Route::ProjectJobsView {
                        project: project.id.clone(),
                    },
                    class: "text-sm font-medium hover:underline",
                    "{project.name}"
                }
                // Where it is, as marks with the address a hover away: the
                // repository, and the foreman's room once there is one —
                // a link where the link is true, per
                // `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
                Reference {
                    mark: "github",
                    says: project.repository.to_string(),
                    link: Some(project.repository_link.clone()),
                }
                if let Some(room) = project.foreman_room.clone() {
                    Reference {
                        mark: "slack",
                        says: "The foreman's room, {room}",
                        link: project.foreman_room_link.clone(),
                    }
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
                    if !project.variables.is_empty() {
                        Tooltip { text: variables,
                            Badge { tabindex: "0", aria_label: "{counted}: {named}",
                                "{counted}"
                            }
                        }
                    }
                    if project.working > 0 {
                        Badge { tone: BadgeTone::Working, "{project.working} of {project.jobs} working" }
                    } else {
                        Badge { "{project.jobs} job(s)" }
                    }
                    // Offered whatever the project is doing, unlike
                    // forgetting one: amending changes what the next job is
                    // given and cannot reach into a container that already
                    // exists. Named with the project, because a screen of
                    // these reads out as a column of identical "Settings"
                    // otherwise.
                    Tooltip { text: "Settings",
                        Link {
                            to: super::Route::ProjectSettingsView {
                                project: project.id.clone(),
                            },
                            class: ButtonVariant::Secondary.styled("px-1.5 py-1"),
                            aria_label: "Settings of {project.name}",
                            {Icon::Edit.draw(14)}
                        }
                    }
                }
            }
            // Who thinks and who works, as chips rather than a sentence: the
            // hat says which is the foreman's, the hammer which are the
            // jobs', a hairline parts the two, and each kit says its name,
            // whose it is, which model, and how hard.
            div { class: "flex flex-wrap items-center gap-1.5",
                Tooltip { text: "The foreman thinks with this",
                    span { class: "inline-flex items-center text-muted-foreground", {Icon::Foreman.draw(14)} }
                }
                KitChip {
                    agent: project.foreman.agent.clone(),
                    agent_name: shown_as(&available, &project.foreman.agent),
                    model: foreman_model,
                    effort: foreman_effort,
                }
                span { class: "mx-1 h-4 w-px bg-border-strong", aria_hidden: "true" }
                Tooltip { text: "Its jobs run on these",
                    span { class: "inline-flex items-center text-muted-foreground", {Icon::Kit.draw(14)} }
                }
                for kit in project.kits.iter() {
                    {
                        let (model, effort) = read_as(&shapes, &kit.fitted);
                        rsx! {
                            KitChip {
                                key: "{kit.name}",
                                name: kit.name.clone(),
                                agent: kit.fitted.agent.clone(),
                                agent_name: shown_as(&available, &kit.fitted.agent),
                                model,
                                effort,
                            }
                        }
                    }
                }
            }
            if !notes.is_empty() {
                p { class: "text-xs text-muted-foreground", "{notes}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    fn a_project_with_everything() -> super::Project {
        super::Project {
            id: "p".to_owned(),
            name: "aviary".to_owned(),
            repository: stageman_wire::Repository {
                owner: "owner".to_owned(),
                name: "aviary".to_owned(),
            },
            repository_link: "https://github.com/owner/aviary".to_owned(),
            foreman: stageman_wire::Fitted::default(),
            kits: Vec::new(),
            access: Some(stageman_wire::AccessView::Token {
                owner: None,
                expires: None,
                expired: false,
            }),
            binding: Some(stageman_wire::BindingView::Own { url: None }),
            variables: Vec::new(),
            brief: String::new(),
            watched: Vec::new(),
            foreman_room: None,
            foreman_room_link: None,
            attending: false,
            working: 0,
            jobs: 0,
            token_form: String::new(),
        }
    }

    /// A row notes what is missing and what is watched, each only when it
    /// applies, and nothing when nothing does.
    #[test]
    fn a_row_notes_absences_and_watched_rooms() {
        assert_eq!(super::noted(&a_project_with_everything()), "");
        let mut bare = a_project_with_everything();
        bare.access = None;
        bare.binding = None;
        bare.watched = vec!["C1".to_owned(), "C2".to_owned()];
        assert_eq!(
            super::noted(&bare),
            "no access to the repository · no Slack binding · watching C1, C2"
        );
        let mut watching = a_project_with_everything();
        watching.watched = vec!["C1".to_owned()];
        assert_eq!(super::noted(&watching), "watching C1");
    }
    use super::super::agents_view::Agent;
    use super::{Choice, Fitted, ModelChoice, Shape, read_as, shown_as};

    /// A project carries identifiers and a row shows names, so something has
    /// to map one to the other — and nothing else would notice if it stopped.
    ///
    /// Found by mutation testing: inverting the comparison inside the row's
    /// lookup broke nothing any test could see, which is exactly the shape of
    /// a screen that renders confidently and wrongly.
    #[test]
    fn an_agent_is_shown_by_its_name_and_not_the_identifier_it_arrived_as() {
        let available = vec![Agent {
            id: "claude".to_owned(),
            name: "Claude".to_owned(),
            description: "does the work".to_owned(),
            configured: true,
            used_by: Vec::new(),
        }];

        assert_eq!(shown_as(&available, "claude"), "Claude");
    }

    /// An agent this build does not know is shown as it stands.
    ///
    /// The other half, and the reason the lookup falls back rather than
    /// hiding: an instance naming an agent that is gone is worth seeing.
    #[test]
    fn an_agent_this_build_does_not_know_is_shown_as_it_arrived() {
        assert_eq!(shown_as(&[], "something-else"), "something-else");
    }

    /// A chip reads names off the shape where the shape has them, and the
    /// identifiers as they stand where it does not, and shows no effort for
    /// a model that takes none.
    #[test]
    fn a_fitted_agent_reads_as_names_where_the_shape_has_them() {
        let shape = Shape {
            agent: "claude".to_owned(),
            models: vec![ModelChoice {
                id: "opus".to_owned(),
                name: "Opus".to_owned(),
                has_effort: true,
            }],
            efforts: vec![Choice {
                id: "xhigh".to_owned(),
                name: "Extra high".to_owned(),
            }],
        };
        let fitted = Fitted {
            agent: "claude".to_owned(),
            model: "opus".to_owned(),
            effort: "xhigh".to_owned(),
        };
        assert_eq!(
            read_as(std::slice::from_ref(&shape), &fitted),
            (
                "Opus".to_owned(),
                Some(("xhigh".to_owned(), "Extra high".to_owned()))
            )
        );
        assert_eq!(
            read_as(&[], &fitted),
            (
                "opus".to_owned(),
                Some(("xhigh".to_owned(), "xhigh".to_owned()))
            ),
            "as they stand, with no shape to read from"
        );
        let effortless = Fitted {
            effort: String::new(),
            ..fitted
        };
        assert_eq!(read_as(&[shape], &effortless).1, None);
    }
}
