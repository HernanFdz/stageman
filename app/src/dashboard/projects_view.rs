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
use crate::ui::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState};

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
) -> DashboardResult<Watching> {
    let name = required("name", &name)?;
    let repository = required("repository", &repository)?;
    let orchestrator_agent = super::named(&orchestrator)?;
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
            credentials: std::collections::BTreeMap::new(),
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

    let applied = move |outcome: DashboardResult<Watching>| match outcome {
        Ok(fresh) => {
            failure.set(None);
            reading.set(Some(Ok(fresh)));
        }
        Err(reason) => failure.set(Some(reason)),
    };

    rsx! {
        div { class: "flex flex-col gap-4",
            if let Some(reason) = failure() {
                Card { title: "That did not work",
                    p { class: "text-sm text-failed", "{reason}" }
                }
            }
            match reading.cloned() {
                Some(Ok(watching)) => rsx! {
                    Card {
                        title: "Projects",
                        note: "A project is a repository, the agents that work on it, and the \
                               credentials those agents need.",
                        aside: rsx! {
                            Badge { "{watching.projects.len()}" }
                        },
                        if watching.projects.is_empty() {
                            EmptyState {
                                title: "Nothing is being watched yet.",
                                note: "Add one below. It needs an agent to think with and at \
                                       least one its jobs can run on.",
                            }
                        } else {
                            ul { class: "divide-y divide-border",
                                for project in watching.projects {
                                    li { key: "{project.id}",
                                        WatchedProject { project, onchanged: applied }
                                    }
                                }
                            }
                        }
                    }
                    NewProject { available: watching.available, onchanged: applied }
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

/// One project, and what can be done to it.
#[component]
fn WatchedProject(project: Project, onchanged: EventHandler<DashboardResult<Watching>>) -> Element {
    let mut token = use_signal(String::new);
    let identifier = project.id.clone();
    let idle = project.idle();

    rsx! {
        div { class: "flex flex-col gap-2 py-3 first:pt-0 last:pb-0",
            div { class: "flex items-baseline gap-3",
                span { class: "text-sm font-medium", "{project.name}" }
                span { class: "truncate font-mono text-xs text-faint-foreground",
                    "{project.repository}"
                }
                span { class: "ml-auto flex shrink-0 items-baseline gap-2",
                    if project.running > 0 {
                        Badge { tone: BadgeTone::Running, "{project.running} of {project.jobs} running" }
                    } else {
                        Badge { "{project.jobs} job(s)" }
                    }
                }
            }
            p { class: "text-xs text-muted-foreground",
                "thinks with {project.orchestrator} · runs jobs on {project.job_agents.join(\", \")}"
            }
            div { class: "flex items-center gap-2",
                input {
                    r#type: "password",
                    class: "w-full max-w-sm rounded-md border border-border bg-surface px-2 py-1.5 \
                            font-mono text-xs placeholder:text-faint-foreground focus-visible:outline-none \
                            focus-visible:ring-2 focus-visible:ring-primary",
                    placeholder: if project.platforms.iter().any(|platform| platform == "github") {
                        "replace the GitHub credential"
                    } else {
                        "paste a GitHub credential"
                    },
                    value: "{token}",
                    oninput: move |event| token.set(event.value()),
                }
                Button {
                    onclick: {
                        let identifier = identifier.clone();
                        move |_| {
                            let identifier = identifier.clone();
                            let supplied = token();
                            async move {
                                let outcome =
                                    credential(identifier, "github".to_owned(), supplied).await;
                                if outcome.is_ok() {
                                    token.set(String::new());
                                }
                                onchanged.call(outcome);
                            }
                        }
                    },
                    "Save"
                }
                Button {
                    variant: ButtonVariant::Danger,
                    disabled: !idle,
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

/// The form that starts watching something new.
#[component]
fn NewProject(
    available: Vec<Agent>,
    onchanged: EventHandler<DashboardResult<Watching>>,
) -> Element {
    let mut name = use_signal(String::new);
    let mut repository = use_signal(String::new);
    let mut orchestrator = use_signal(|| {
        available
            .first()
            .map_or_else(String::new, |agent| agent.id.clone())
    });
    let mut chosen = use_signal(|| {
        available
            .first()
            .map(|agent| agent.id.clone())
            .into_iter()
            .collect::<Vec<_>>()
    });

    if available.is_empty() {
        return rsx! {
            Card { title: "Watch a repository",
                EmptyState {
                    title: "No agent has a credential yet.",
                    note: "A project names one agent to think with and at least one its jobs run \
                           on, so configuring an agent comes first.",
                }
            }
        };
    }

    rsx! {
        Card {
            title: "Watch a repository",
            note: "Everything here can be changed afterwards except the repository.",
            div { class: "flex flex-col gap-3",
                Field { label: "Name",
                    input {
                        class: FIELD,
                        placeholder: "what to call it",
                        value: "{name}",
                        oninput: move |event| name.set(event.value()),
                    }
                }
                Field { label: "Repository",
                    input {
                        class: FIELD,
                        placeholder: "https://github.com/…",
                        value: "{repository}",
                        oninput: move |event| repository.set(event.value()),
                    }
                }
                Field { label: "Thinks with",
                    select {
                        class: FIELD,
                        value: "{orchestrator}",
                        onchange: move |event| orchestrator.set(event.value()),
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
                                    checked: chosen().contains(&agent.id),
                                    onchange: {
                                        let picked = agent.id.clone();
                                        move |event: Event<FormData>| {
                                            let picked = picked.clone();
                                            chosen.with_mut(|chosen| {
                                                chosen.retain(|held| held != &picked);
                                                if event.checked() {
                                                    chosen.push(picked);
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
                div {
                    Button {
                        onclick: move |_| async move {
                            let outcome = create(name(), repository(), orchestrator(), chosen())
                                .await;
                            if outcome.is_ok() {
                                name.set(String::new());
                                repository.set(String::new());
                            }
                            onchanged.call(outcome);
                        },
                        "Watch it"
                    }
                }
            }
        }
    }
}

/// What every input on this screen looks like.
///
/// A constant rather than a component, because the thing being shared is the
/// appearance of a box and not its behaviour — a `select` and an `input`
/// differ in everything except how they should look.
const FIELD: &str = "w-full max-w-sm rounded-md border border-border bg-surface px-2 py-1.5 \
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
    use super::Project;

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
