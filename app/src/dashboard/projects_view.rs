//! The projects an instance watches, and what each one needs in order to work.
//!
//! The screen that makes an instance able to *do* something. It comes after
//! agents because a project names one agent for its orchestrator and a
//! non-empty set its jobs may use, and both have to be configured before a
//! project may name them — `docs/decisions/0021-an-instance-starts-empty.md`.
//!
//! **Validity is asked, not restated.** What makes an instance valid is
//! `State::check` and nothing else; this screen builds the state it would
//! produce, asks, and reports the answer. Re-deciding here would be a second
//! definition of valid that could drift from the first, which is the trap 0021
//! chose a single function to avoid.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use dioxus::server::axum::Extension;
use serde::{Deserialize, Serialize};

use super::agents_view::Agent;
use super::error::{DashboardError, DashboardResult};
use crate::ui::{Badge, BadgeTone, Button, Card, EmptyState, Modal};

/// One project, as much of it as a page is allowed to know.
///
/// Note what is absent and cannot be added: the credentials. Which platforms
/// have one is here; what any of them is, is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// What the browser names it back as.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// The agent its orchestrator thinks with.
    pub orchestrator: String,
    /// The agents its jobs may run on. Never empty in a valid instance.
    pub job_agents: Vec<String>,
    /// The platforms it has a credential for.
    pub platforms: Vec<String>,
    /// How many of its jobs are still running.
    pub running: usize,
    /// How many jobs it has had, running or finished.
    pub jobs: usize,
}

impl Project {
    /// Whether nothing of this project's is currently running.
    ///
    /// The screen's version of the rule the route enforces, and the reason it
    /// is a method: a page that offered a button the server would refuse would
    /// be lying, and this is the only thing keeping the two answers the same.
    #[must_use]
    pub const fn idle(&self) -> bool {
        self.running == 0
    }
}

/// What the projects screen needs in order to draw itself.
///
/// One route rather than two, because the screen cannot offer to create a
/// project without knowing which agents may be named — and two reads could
/// disagree about that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watching {
    /// What is being watched now.
    pub projects: Vec<Project>,
    /// The agents that could be named. Only the configured ones: naming an
    /// agent without a credential is refused, so offering it would be an
    /// invitation to fail.
    pub available: Vec<Agent>,
}

/// Everything the projects screen shows.
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
#[get("/api/projects", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn projects() -> DashboardResult<Watching> {
    let state = instance.0.read();
    Ok(Watching {
        projects: super::watching(&state),
        available: super::listed(&state)
            .into_iter()
            .filter(|agent| agent.configured)
            .collect(),
    })
}

/// Starts watching a repository.
///
/// # Errors
///
/// Fails if anything required is missing, if an agent named is not one this
/// instance runs, or if the result would not be a valid instance.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/projects/create", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn create(
    name: String,
    repository: String,
    orchestrator: String,
    job_agents: Vec<String>,
    credential: String,
) -> DashboardResult<Watching> {
    let name = required("name", &name)?;
    let repository = required("repository", &repository)?;
    let orchestrator_agent = super::named(&orchestrator)?;
    let credential = required("credential", &credential)?;
    if job_agents.is_empty() {
        return Err(DashboardError::JobAgentsMissing);
    }
    let job_agents = job_agents
        .iter()
        .map(|agent| super::named(agent))
        .collect::<DashboardResult<_>>()?;

    let mut state = instance.0.update();

    // Asked of a copy before it is asked of the instance. `State::check` is
    // the one definition of valid, and the store consults it on write — where
    // it *logs* a refusal rather than returning one. Mutating first would
    // therefore leave an instance that is invalid in memory and correct on
    // disk, with only a log line saying so.
    let mut candidate = state.clone();
    candidate.projects.insert(
        stageman_core::ProjectId::from_uuid(uuid::Uuid::new_v4()),
        stageman_core::Project {
            name,
            repository,
            orchestrator_agent,
            job_agents,
            // One platform, so one field. A second would make this a list
            // here and on the form, and the closed set in the domain is what
            // would force both.
            credentials: std::collections::BTreeMap::from([(
                stageman_core::Platform::GitHub,
                stageman_core::Secret::new(credential),
            )]),
            jobs: std::collections::BTreeMap::new(),
        },
    );
    candidate
        .check()
        .map_err(|reason| DashboardError::from_inconsistent(&reason, super::shown))?;

    *state = candidate;
    let watching = watching_now(&state);
    drop(state);

    Ok(watching)
}

/// Gives a project the credential it needs to reach a platform.
///
/// # Errors
///
/// Fails if the project or platform is unknown, or the credential is empty.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/projects/credential", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn credential(
    project: String,
    platform: String,
    secret: String,
) -> DashboardResult<Watching> {
    let platform = super::named_platform(&platform)?;
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(DashboardError::CredentialMissing);
    }

    let mut state = instance.0.update();
    let identifier = identify(&state, &project)?;
    let Some(watched) = state.projects.get_mut(&identifier) else {
        drop(state);
        return Err(DashboardError::UnknownProject { id: project });
    };
    watched
        .credentials
        .insert(platform, stageman_core::Secret::new(secret.to_owned()));
    let watching = watching_now(&state);
    drop(state);

    Ok(watching)
}

/// Stops watching a repository, if nothing of its is still running.
///
/// # Errors
///
/// Fails if the project is unknown, or any of its jobs is still running.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/projects/forget", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn forget(project: String) -> DashboardResult<Watching> {
    let mut state = instance.0.update();
    let identifier = identify(&state, &project)?;

    let Some(watched) = state.projects.get(&identifier) else {
        drop(state);
        return Err(DashboardError::UnknownProject { id: project });
    };
    if let Some(running) = busy(watched) {
        let name = watched.name.clone();
        drop(state);
        return Err(DashboardError::ProjectBusy { name, running });
    }

    state.projects.remove(&identifier);
    let watching = watching_now(&state);
    drop(state);

    Ok(watching)
}

/// How many of a project's jobs are still going, if any are.
///
/// A function rather than a comparison inside the route, so that the rule can
/// be tested: a fixture cannot contain a running job, because startup
/// reconciles what the instance believes against what the runtime has and
/// records a job with no container as failed — see
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`. So the only place
/// this can be checked at all is here.
#[cfg(feature = "server")]
fn busy(project: &stageman_core::Project) -> Option<usize> {
    let running = project
        .jobs
        .values()
        .filter(|job| job.progress == stageman_core::Progress::Running)
        .count();

    (running > 0).then_some(running)
}

/// The identifier a browser sent back, as this instance knows it.
///
/// Compared as text rather than parsed, so that a malformed identifier and an
/// unknown one are the same answer — which they are, from the operator's side.
///
/// # Errors
///
/// Fails if nothing is watched under it.
#[cfg(feature = "server")]
fn identify(
    state: &stageman_core::State,
    identifier: &str,
) -> DashboardResult<stageman_core::ProjectId> {
    state
        .projects
        .keys()
        .find(|known| known.to_string() == identifier)
        .copied()
        .ok_or_else(|| DashboardError::UnknownProject {
            id: identifier.to_owned(),
        })
}

/// A field that has to say something.
///
/// # Errors
///
/// Fails if it says nothing.
#[cfg(feature = "server")]
fn required(field: &str, given: &str) -> DashboardResult<String> {
    let trimmed = given.trim();
    if trimmed.is_empty() {
        return Err(DashboardError::Incomplete {
            field: field.to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

/// What every route here answers with.
#[cfg(feature = "server")]
fn watching_now(state: &stageman_core::State) -> Watching {
    Watching {
        projects: super::watching(state),
        available: super::listed(state)
            .into_iter()
            .filter(|agent| agent.configured)
            .collect(),
    }
}

/// The projects screen.
#[component]
pub fn ProjectsView() -> Element {
    let mut reading = use_server_future(projects)?;
    let mut failure = use_signal(|| None::<DashboardError>);
    let mut adding = use_signal(|| false);
    let mut draft = use_signal(Draft::default);

    rsx! {
        div { class: "flex flex-col gap-4",
            match reading.cloned() {
                Some(Ok(watching)) => rsx! {
                    Card {
                        title: "Projects",
                        note: "A project is a repository, the agents that work on it, and the \
                               credential those agents need to reach it.",
                        badge: rsx! {
                            Badge { "{watching.projects.len()}" }
                        },
                        aside: rsx! {
                            Button {
                                // A glyph, because the card's title already
                                // says what is being added and repeating it on
                                // the control is the longest thing on the row
                                // saying the least. Named for anyone not
                                // looking at it.
                                class: "px-2.5 text-base leading-none",
                                aria_label: "New project",
                                title: "New project",
                                disabled: watching.available.is_empty(),
                                onclick: {
                                    // The first configured agent, which is
                                    // what both fields start on. Taken here so
                                    // the handler does not need the whole list.
                                    let first = watching
                                        .available
                                        .first()
                                        .map(|agent| agent.id.clone());
                                    move |_| {
                                        // Emptied on the way in rather than on
                                        // the way out, so that a modal
                                        // abandoned half-filled does not
                                        // reopen holding what was abandoned.
                                        draft.set(Draft {
                                            orchestrator: first.clone().unwrap_or_default(),
                                            job_agents: first.clone().into_iter().collect(),
                                            ..Draft::default()
                                        });
                                        failure.set(None);
                                        adding.set(true);
                                    }
                                },
                                "+"
                            }
                        },
                        if watching.projects.is_empty() {
                            EmptyState {
                                title: "Nothing is being watched yet.",
                                note: if watching.available.is_empty() {
                                    "A project names one agent to think with and at least one its \
                                     jobs run on, so configuring an agent comes first."
                                } else {
                                    "Add one. It needs a repository, the agents that work on it, \
                                     and a credential to reach it with."
                                },
                            }
                        } else {
                            ul { class: "divide-y divide-border",
                                for project in watching.projects {
                                    li { key: "{project.id}", WatchedProject { project } }
                                }
                            }
                        }
                    }
                    if adding() {
                        Modal {
                            title: "New project",
                            onclose: move |()| adding.set(false),
                            actions: rsx! {
                                Button {
                                    // Unavailable until pressing it would
                                    // work, which is this screen's whole
                                    // answer to an incomplete form for now —
                                    // per-field messages are the better
                                    // answer and are not this change.
                                    class: "px-2.5 text-base leading-none",
                                    aria_label: "Add",
                                    title: "Add",
                                    disabled: !draft().is_complete(),
                                    onclick: move |_| async move {
                                        let asked = draft();
                                        match create(
                                                asked.name,
                                                asked.repository,
                                                asked.orchestrator,
                                                asked.job_agents,
                                                asked.credential,
                                            )
                                            .await
                                        {
                                            Ok(fresh) => {
                                                failure.set(None);
                                                reading.set(Some(Ok(fresh)));
                                                adding.set(false);
                                            }
                                            // Left open, deliberately: closing
                                            // would throw away what was typed,
                                            // and what is wrong is almost
                                            // always in one field of it.
                                            Err(reason) => failure.set(Some(reason)),
                                        }
                                    },
                                    "✓"
                                }
                            },
                            // Shown here rather than behind the modal, which is
                            // where it used to be. Anything the screen could
                            // have seen is caught by the control above being
                            // unavailable, so what reaches this is a refusal
                            // only the instance could make.
                            if let Some(reason) = failure() {
                                p { class: "mb-3 text-sm text-failed", "{reason}" }
                            }
                            ProjectForm { draft, available: watching.available }
                        }
                    }
                },
                Some(Err(reason)) => rsx! {
                    Card { title: "The projects could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                None => rsx! {
                    p { class: "text-sm text-muted-foreground", "Reading the projects…" }
                },
            }
        }
    }
}

/// One project, as the list shows it.
///
/// Read-only, and that is a decision rather than a gap. What a project *is* is
/// decided in one place — the form — and a list that also edited would be a
/// second place, disagreeing about which fields matter and which are required.
/// Changing one is the same form with different initial values, which is what
/// [`ProjectForm`] takes.
#[component]
fn WatchedProject(project: Project) -> Element {
    rsx! {
        // Roomier than the rows on the agents screen, and deliberately: an
        // agent is one line and a project is three, so the same padding reads
        // as cramped here.
        //
        // The first and last shed their outer padding entirely, so the space
        // above the first row and below the last are both the card's own and
        // therefore equal. Anything else makes the top gap the sum of two
        // paddings and the eye reads it as a mistake.
        div { class: "flex flex-col gap-1.5 py-4 first:pt-0 last:pb-0",
            div { class: "flex items-baseline gap-3",
                span { class: "text-sm font-medium", "{project.name}" }
                span { class: "truncate font-mono text-xs text-faint-foreground",
                    "{project.repository}"
                }
                span { class: "ml-auto shrink-0",
                    if project.running > 0 {
                        Badge { tone: BadgeTone::Running, "{project.running} of {project.jobs} running" }
                    } else {
                        Badge { "{project.jobs} job(s)" }
                    }
                }
            }
            p { class: "text-xs text-muted-foreground",
                "thinks with {project.orchestrator} · runs jobs on {project.job_agents.join(\", \")}"
                if project.platforms.is_empty() {
                    " · no credential"
                }
            }
        }
    }
}

/// Everything the form collects, which is everything a project is.
///
/// A struct rather than five handlers, because the form's whole purpose is to
/// be filled in twice — once to create and once to change — and a caller
/// should differ in what it *does* with the answer rather than in how it
/// receives it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    /// What to call it.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// The agent its orchestrator thinks with.
    pub orchestrator: String,
    /// The agents its jobs may run on.
    pub job_agents: Vec<String>,
    /// What reaches the repository.
    pub credential: String,
}

impl Draft {
    /// Whether this says everything a project needs.
    ///
    /// The same conditions the route enforces, and deliberately so: the point
    /// is that the control which submits is unavailable until pressing it
    /// would succeed, so the operator is never told off for something the
    /// screen could see. It is not a second definition of validity — the route
    /// still checks, because a browser is not a place to enforce anything —
    /// but it is the screen refusing to ask the question badly.
    ///
    /// What it deliberately does *not* check is anything the domain decides:
    /// whether the agents named are configured is `State::check`'s to answer,
    /// and the form only offers agents that are.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.repository.trim().is_empty()
            && !self.orchestrator.is_empty()
            && !self.job_agents.is_empty()
            && !self.credential.trim().is_empty()
    }
}

/// The form that describes a project.
///
/// **Controlled**: the caller owns the draft and this writes into it. That is
/// what lets the control which submits live outside the form — in the modal's
/// header, beside the way out — and it is what an edit will need too, since
/// editing is this form over different initial values with a different handler.
///
/// It renders no submit of its own. A form that both collected and committed
/// would have to be told where its button goes, which is the caller's business
/// and not its own.
#[component]
fn ProjectForm(draft: Signal<Draft>, available: Vec<Agent>) -> Element {
    let mut draft = draft;

    rsx! {
        div { class: "flex flex-col gap-3",
            Field { label: "Name",
                input {
                    class: FIELD,
                    placeholder: "what to call it",
                    value: "{draft().name}",
                    oninput: move |event| draft.with_mut(|draft| draft.name = event.value()),
                }
            }
            Field { label: "Repository",
                input {
                    class: FIELD,
                    placeholder: "https://github.com/…",
                    value: "{draft().repository}",
                    oninput: move |event| draft.with_mut(|draft| draft.repository = event.value()),
                }
            }
            Field { label: "Thinks with",
                select {
                    class: FIELD,
                    value: "{draft().orchestrator}",
                    onchange: move |event| {
                        draft.with_mut(|draft| draft.orchestrator = event.value());
                    },
                    for agent in available.iter() {
                        option { key: "{agent.id}", value: "{agent.id}", "{agent.name}" }
                    }
                }
            }
            Field { label: "Runs jobs on",
                div { class: "flex flex-wrap gap-3",
                    for agent in available.iter() {
                        label { key: "{agent.id}", class: "flex items-center gap-1.5 text-sm",
                            input {
                                r#type: "checkbox",
                                checked: draft().job_agents.contains(&agent.id),
                                onchange: {
                                    let picked = agent.id.clone();
                                    move |event: Event<FormData>| {
                                        let picked = picked.clone();
                                        draft
                                            .with_mut(|draft| {
                                                draft.job_agents.retain(|held| held != &picked);
                                                if event.checked() {
                                                    draft.job_agents.push(picked);
                                                }
                                            });
                                    }
                                },
                            }
                            "{agent.name}"
                        }
                    }
                }
            }
            Field { label: "GitHub credential",
                input {
                    r#type: "password",
                    class: FIELD,
                    placeholder: "a token scoped to this repository",
                    value: "{draft().credential}",
                    oninput: move |event| draft.with_mut(|draft| draft.credential = event.value()),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Scoped to this repository, with contents and pull requests write. A token that \
                 reaches more than this project is a token every job on it could misuse."
            }
        }
    }
}

/// What every input on this screen looks like.
///
/// A constant rather than a component, because the thing being shared is the
/// appearance of a box and not its behaviour — a `select` and an `input`
/// differ in everything except how they should look.
const FIELD: &str = "w-full rounded-md border border-border bg-surface px-2 py-1.5 \
                     text-sm placeholder:text-faint-foreground focus-visible:outline-none \
                     focus-visible:ring-2 focus-visible:ring-primary";

/// A labelled row in the form.
#[component]
fn Field(label: String, children: Element) -> Element {
    rsx! {
        label { class: "flex flex-col gap-1",
            span { class: "text-xs font-medium text-muted-foreground", "{label}" }
            {children}
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::{DashboardError, busy, identify};
    use stageman_core::{Agent, Job, JobId, Progress, Project, ProjectId, State, Timestamp};
    use std::collections::{BTreeMap, BTreeSet};

    /// One job, in whatever state the caller needs it.
    fn job(progress: Progress) -> Job {
        Job {
            agent: Agent::Claude,
            reason: "because a test said so".to_owned(),
            kickoff: "do the thing".to_owned(),
            created_at: Timestamp::UNIX_EPOCH,
            progress,
        }
    }

    /// A project holding exactly these jobs.
    fn holding(jobs: &[Progress]) -> Project {
        Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            orchestrator_agent: Agent::Claude,
            job_agents: BTreeSet::from([Agent::Claude]),
            credentials: BTreeMap::new(),
            // Freshly minted rather than derived from a position, which would
            // need a conversion that can fail — and the gate is right that
            // defaulting such a conversion would silently give two jobs the
            // same identifier, which is the one property this fixture needs.
            jobs: jobs
                .iter()
                .map(|progress| {
                    (
                        JobId::from_uuid(uuid::Uuid::new_v4()),
                        job(progress.clone()),
                    )
                })
                .collect(),
        }
    }

    /// Nothing running is what lets a project be forgotten.
    #[test]
    fn a_project_with_nothing_running_is_not_busy() {
        assert_eq!(busy(&holding(&[])), None);
        assert_eq!(
            busy(&holding(&[
                Progress::Completed,
                Progress::Failed("it did not work".to_owned())
            ])),
            None
        );
    }

    /// The count is of running jobs, not of jobs.
    ///
    /// The distinction the whole refusal rests on: a project with a hundred
    /// finished jobs may be forgotten, and one with a single running job may
    /// not — because that job owns a container the instance would stop being
    /// able to name.
    #[test]
    fn a_project_is_busy_for_exactly_its_running_jobs() {
        assert_eq!(busy(&holding(&[Progress::Running])), Some(1));
        assert_eq!(
            busy(&holding(&[
                Progress::Running,
                Progress::Completed,
                Progress::Running,
            ])),
            Some(2)
        );
    }

    /// An instance watching one project under a known identifier.
    fn watching(id: ProjectId) -> State {
        State {
            agents: BTreeMap::new(),
            projects: BTreeMap::from([(
                id,
                Project {
                    name: "aviary".to_owned(),
                    repository: "https://example.invalid/aviary".to_owned(),
                    orchestrator_agent: Agent::Claude,
                    job_agents: BTreeSet::from([Agent::Claude]),
                    credentials: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                },
            )]),
        }
    }

    /// An identifier the browser sends back finds the project it came from.
    #[test]
    fn an_identifier_finds_the_project_it_names() {
        let id = ProjectId::from_uuid(uuid::Uuid::from_u128(9));
        let state = watching(id);

        assert_eq!(identify(&state, &id.to_string()), Ok(id));
    }

    /// Anything else is not found rather than matched to whatever is nearest.
    #[test]
    fn an_identifier_naming_nothing_finds_nothing() {
        let state = watching(ProjectId::from_uuid(uuid::Uuid::from_u128(9)));
        let other = ProjectId::from_uuid(uuid::Uuid::from_u128(10)).to_string();

        assert_eq!(
            identify(&state, &other),
            Err(DashboardError::UnknownProject { id: other.clone() })
        );
        assert!(identify(&state, "not-an-identifier").is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::{Draft, Project};

    /// One project, with however many jobs running.
    fn watched(running: usize, jobs: usize) -> Project {
        Project {
            id: "an-identifier".to_owned(),
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            orchestrator: "Claude".to_owned(),
            job_agents: vec!["Claude".to_owned()],
            platforms: Vec::new(),
            running,
            jobs,
        }
    }

    /// A draft the form would accept.
    fn filled() -> Draft {
        Draft {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            orchestrator: "claude".to_owned(),
            job_agents: vec!["claude".to_owned()],
            credential: "ghp-not-a-real-token".to_owned(),
        }
    }

    #[test]
    fn a_draft_with_every_answer_is_complete() {
        assert!(filled().is_complete());
    }

    /// Every field is required, and the test says so one at a time.
    ///
    /// Written out rather than looped because the point is which field, and a
    /// loop over closures would say it less clearly than five lines do.
    #[test]
    fn a_draft_missing_any_answer_is_not() {
        let without = |change: fn(&mut Draft)| {
            let mut draft = filled();
            change(&mut draft);
            draft
        };

        assert!(!without(|draft| draft.name.clear()).is_complete());
        assert!(!without(|draft| draft.repository.clear()).is_complete());
        assert!(!without(|draft| draft.orchestrator.clear()).is_complete());
        assert!(!without(|draft| draft.job_agents.clear()).is_complete());
        assert!(!without(|draft| draft.credential.clear()).is_complete());
    }

    /// Whitespace is not an answer.
    ///
    /// The route trims before judging, so a form that accepted spaces would
    /// offer a control that fails — which is the one thing this check exists
    /// to prevent.
    #[test]
    fn whitespace_does_not_count_as_an_answer() {
        let mut draft = filled();
        draft.name = "   ".to_owned();
        assert!(!draft.is_complete());

        let mut draft = filled();
        draft.credential = "\t ".to_owned();
        assert!(!draft.is_complete());
    }

    /// The screen must not offer what the route would refuse.
    #[test]
    fn a_project_with_a_running_job_is_not_idle() {
        assert!(!watched(1, 3).idle());
        assert!(!watched(3, 3).idle());
    }

    #[test]
    fn a_project_whose_jobs_have_all_finished_is_idle() {
        assert!(watched(0, 3).idle());
        assert!(watched(0, 0).idle());
    }
}
