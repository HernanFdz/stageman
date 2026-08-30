//! One project, and the jobs it has had.
//!
//! The first screen that makes something *happen* rather than configuring what
//! could. Everything before it described an instance; this starts a job, which
//! is one agent in one container doing work on one repository until it is done.
//!
//! **What an operator types is the work, never the instruction.** The agent's
//! instruction is composed by the foreman from the work and the
//! repository, and carries three things that are not negotiable — nothing is
//! checked out, the tools are already authenticated, and work ends at a
//! proposal. `docs/architecture.md` §1 puts every place an instruction is
//! authored in that one crate, which is what makes the snapshot-testing rule
//! in `docs/conventions.md` §4 mean anything at all. A form collecting a
//! finished instruction would route around all of it.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use dioxus::server::axum::Extension;
use serde::{Deserialize, Serialize};

use super::agents_view::Agent;
use super::error::{DashboardError, DashboardResult};
use crate::ui::{Badge, BadgeTone, Button, Card, EmptyState, Modal};

/// Why a job started, when a person started it.
///
/// Filled in rather than asked for. The vocabulary in `docs/conventions.md` §2
/// calls a reason "why the foreman decided to" — and an foreman has
/// a reason distinct from the work because it is judging a signal. A person
/// pressing a button has no separate judgement to record: the provenance *is*
/// that a person asked, and asking them to phrase that as well as the work
/// would produce two fields saying one thing.
#[cfg(feature = "server")]
const BY_HAND: &str = "started by hand from the dashboard";

/// Where a job has got to, as a page sees it.
///
/// The three in `docs/conventions.md` §2 and no more. `Idle` says its agent
/// stopped rather than that the work is done — nothing here can tell a job
/// that finished from one that asked a question, and
/// `docs/decisions/0002-never-merge-never-deploy.md` means a person reads what
/// it proposed before any of it counts for anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "standing", rename_all = "snake_case")]
pub enum Standing {
    /// Its agent has been given something and has not stopped.
    Working,
    /// Its agent stopped, and nothing has been given to it since.
    Idle,
    /// It could not be finished, and this is what went wrong.
    Failed {
        /// Prose for a person, not a code to branch on.
        why: String,
    },
}

impl Standing {
    /// How this reads on a badge.
    #[must_use]
    pub const fn tone(&self) -> BadgeTone {
        match self {
            Self::Working => BadgeTone::Working,
            Self::Idle => BadgeTone::Idle,
            Self::Failed { .. } => BadgeTone::Failed,
        }
    }

    /// What this is called.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Failed { .. } => "failed",
        }
    }
}

/// One job, as much of it as a page is allowed to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// What names it, and what its container is named after.
    pub id: String,
    /// Which agent ran it.
    pub agent: String,
    /// Why it was started, in prose.
    pub reason: String,
    /// What its agent was told to do.
    ///
    /// The whole instruction, including the parts the foreman composed
    /// rather than the part somebody typed. Shown because it is the only
    /// record of what the agent was actually asked, and a job that went wrong
    /// is usually a job that was asked badly.
    pub kickoff: String,
    /// When the record was made.
    pub created_at: String,
    /// Where it has got to.
    pub standing: Standing,
}

/// One project and its jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Working {
    /// What to call the project.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// The agents its jobs may run on. Never empty in a valid instance.
    pub agents: Vec<Agent>,
    /// Its jobs, newest first.
    pub jobs: Vec<Job>,
}

/// Everything one project's screen shows.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[get("/api/projects/{project}/jobs", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn jobs(project: String) -> DashboardResult<Working> {
    let state = instance.0.read();

    working(&state, &project)
}

/// Starts a job on a project.
///
/// Answers as soon as the job exists, and supervises it on a task of its own.
/// `docs/conventions.md` §3 forbids the second half from happening on the
/// request path, and the split is what lets the answer already contain the job
/// — a request that spawned everything would return a list without the thing
/// it had just created, and nothing yet tells an open page otherwise.
///
/// # Errors
///
/// Fails if the project is unknown, if the work is empty, or if the agent is
/// not one that project's jobs may run on.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/projects/{project}/jobs/start", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn start(project: String, agent: String, work: String) -> DashboardResult<Working> {
    let named = super::named(&agent)?;
    let work = work.trim();
    if work.is_empty() {
        return Err(DashboardError::Incomplete {
            field: "work".to_owned(),
        });
    }

    // Every refusal is decided while the instance is held, and the guard is
    // released before any of them is returned — a read guard living across a
    // `?` would hold the instance shut for as long as the error took to
    // travel.
    let identifier = {
        let state = instance.0.read();
        let found = super::identify(&state, &project);
        // Asked before starting rather than after failing: a project's jobs
        // may run on a set of agents, and one outside it is a request this
        // instance should refuse rather than a handout it cannot decide.
        let refusal = found.as_ref().ok().and_then(|found| {
            let watched = state.projects.get(found)?;
            forbids(watched, named).then(|| DashboardError::AgentNotOnProject {
                name: super::shown(named),
                project: watched.name.clone(),
            })
        });
        drop(state);

        if let Some(refusal) = refusal {
            return Err(refusal);
        }
        found?
    };

    let started =
        crate::begin(&instance.0, identifier, named, BY_HAND, work).map_err(|reason| {
            // Nothing an operator can act on: the project exists and its agents
            // are configured, or the checks above would have refused. What is left
            // is an instance that has stopped being consistent, which belongs in a
            // log rather than on a screen.
            dioxus::logger::tracing::error!(?reason, "a job could not be started");
            DashboardError::Failed
        })?;

    let answer = {
        let state = instance.0.read();
        let answer = working(&state, &project)?;
        drop(state);
        answer
    };

    // The job outlives the request, which is the point. Nothing awaits this:
    // the record already exists, the outcome is written when it arrives, and a
    // process killed in between leaves a job the sweep puts back to work.
    let store = std::sync::Arc::clone(&instance.0);
    tokio::spawn(async move {
        drop(crate::supervise(&store, &crate::RUNTIME, started).await);
    });

    Ok(answer)
}

/// Whether this project's jobs may *not* run on that agent.
///
/// Phrased as the refusal rather than the permission so that the negation
/// lives here, where it can be tested, rather than at the one call site where
/// it cannot: a project's set of job agents always contains the only agent
/// there is today, so no request can reach the refusing branch through the
/// domain. A fixture can hold a set a valid instance could not, which is the
/// whole reason this is a function.
///
/// It exists for the version of this with two agents, where a project running
/// jobs on one and an operator asking for the other is an ordinary mistake
/// rather than an impossible one.
#[cfg(feature = "server")]
fn forbids(project: &stageman_core::Project, agent: stageman_core::Agent) -> bool {
    !project.job_agents.contains(&agent)
}

/// One project's screen, from the instance.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier.
#[cfg(feature = "server")]
fn working(state: &stageman_core::State, project: &str) -> DashboardResult<Working> {
    let identifier = super::identify(state, project)?;
    let watched =
        state
            .projects
            .get(&identifier)
            .ok_or_else(|| DashboardError::UnknownProject {
                id: project.to_owned(),
            })?;

    // Newest first, which is the order somebody looks in. Jobs are keyed by an
    // identifier carrying no order of its own, so this sorts by when the
    // record was made.
    let mut jobs: Vec<Job> = watched
        .jobs
        .iter()
        .map(|(id, job)| Job {
            id: id.to_string(),
            agent: super::shown(job.agent),
            reason: job.reason.clone(),
            kickoff: job.kickoff.clone(),
            created_at: job.created_at.to_string(),
            standing: standing(&job.progress),
        })
        .collect();
    jobs.sort_by(|one, other| other.created_at.cmp(&one.created_at));

    Ok(Working {
        name: watched.name.clone(),
        repository: watched.repository.clone(),
        agents: super::listed(state)
            .into_iter()
            .filter(|agent| {
                super::named(&agent.id).is_ok_and(|named| watched.job_agents.contains(&named))
            })
            .collect(),
        jobs,
    })
}

/// The domain's progress, as a page sees it.
///
/// The failure's prose crosses with it. It is the only thing a person has to
/// go on when a job goes wrong, and `docs/conventions.md` §2 keeps it as prose
/// rather than a code precisely so that it can be read.
#[cfg(feature = "server")]
fn standing(progress: &stageman_core::Progress) -> Standing {
    match progress {
        stageman_core::Progress::Working => Standing::Working,
        stageman_core::Progress::Idle => Standing::Idle,
        stageman_core::Progress::Failed(why) => Standing::Failed { why: why.clone() },
    }
}

/// One project's screen.
#[component]
pub fn ProjectJobsView(project: String) -> Element {
    // `use_reactive!` because `project` is a plain value rather than a signal:
    // without it this resource keeps its first identifier when the route
    // changes, and the screen shows another project's jobs while claiming to
    // be this one.
    let mut reading = use_server_future(use_reactive!(|project| jobs(project)))?;
    let mut failure = use_signal(|| None::<DashboardError>);
    let mut starting = use_signal(|| false);
    let mut draft = use_signal(Wanted::default);
    let identifier = project;

    rsx! {
        div { class: "flex flex-col gap-4",
            match reading.cloned() {
                Some(Ok(working)) => rsx! {
                    Card {
                        title: working.name.clone(),
                        note: working.repository.clone(),
                        badge: rsx! {
                            Badge { "{working.jobs.len()}" }
                        },
                        aside: rsx! {
                            Button {
                                class: "px-2.5 text-base leading-none",
                                aria_label: "Start a job",
                                title: "Start a job",
                                onclick: {
                                    let first = working.agents.first().map(|agent| agent.id.clone());
                                    move |_| {
                                        draft.set(Wanted {
                                            agent: first.clone().unwrap_or_default(),
                                            work: String::new(),
                                        });
                                        failure.set(None);
                                        starting.set(true);
                                    }
                                },
                                "+"
                            }
                        },
                        if working.jobs.is_empty() {
                            EmptyState {
                                title: "Nothing has run on this project yet.",
                                note: "Describe a piece of work and an agent will do it in a \
                                       container of its own, stopping at a proposal.",
                            }
                        } else {
                            ul { class: "divide-y divide-border",
                                for job in working.jobs {
                                    li { key: "{job.id}", RanJob { job } }
                                }
                            }
                        }
                    }
                    if starting() {
                        Modal {
                            title: "Start a job",
                            onclose: move |()| starting.set(false),
                            actions: rsx! {
                                Button {
                                    class: "px-2.5 text-base leading-none",
                                    aria_label: "Start",
                                    title: "Start",
                                    disabled: !draft().is_complete(),
                                    onclick: move |_| {
                                        let identifier = identifier.clone();
                                        let asked = draft();
                                        async move {
                                            match start(identifier, asked.agent, asked.work).await {
                                                Ok(fresh) => {
                                                    failure.set(None);
                                                    reading.set(Some(Ok(fresh)));
                                                    starting.set(false);
                                                }
                                                Err(reason) => failure.set(Some(reason)),
                                            }
                                        }
                                    },
                                    "✓"
                                }
                            },
                            if let Some(reason) = failure() {
                                p { class: "mb-3 text-sm text-failed", "{reason}" }
                            }
                            JobForm { draft, agents: working.agents }
                        }
                    }
                },
                Some(Err(reason)) => rsx! {
                    Card { title: "This project could not be read",
                        p { class: "text-sm text-failed", "{reason}" }
                    }
                },
                None => rsx! {
                    p { class: "text-sm text-muted-foreground", "Reading the project…" }
                },
            }
        }
    }
}

/// One job, as the list shows it.
#[component]
fn RanJob(job: Job) -> Element {
    let mut showing = use_signal(|| false);

    rsx! {
        div { class: "flex flex-col gap-1.5 py-4 first:pt-0 last:pb-0",
            div { class: "flex items-baseline gap-3",
                Badge { tone: job.standing.tone(), "{job.standing.label()}" }
                span { class: "text-sm", "{job.reason}" }
                span { class: "ml-auto shrink-0 font-mono text-xs text-faint-foreground",
                    "{job.agent} · {job.created_at}"
                }
            }
            if let Standing::Failed { why } = &job.standing {
                p { class: "text-xs text-failed", "{why}" }
            }
            div {
                button {
                    r#type: "button",
                    class: "text-xs text-muted-foreground hover:text-foreground",
                    onclick: move |_| showing.toggle(),
                    if showing() { "hide what it was told" } else { "what it was told" }
                }
                if showing() {
                    pre { class: "mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap rounded-md \
                                  bg-surface-muted p-3 font-mono text-xs text-muted-foreground",
                        "{job.kickoff}"
                    }
                }
            }
        }
    }
}

/// What starting a job asks for.
///
/// The work and which agent, and nothing else. Not the instruction: that is
/// composed from this, and composing it here would put an author of
/// instructions outside the one crate allowed to be one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wanted {
    /// Which of the project's agents should do it.
    pub agent: String,
    /// What to do, in the operator's own words.
    pub work: String,
}

impl Wanted {
    /// Whether this says enough to start a job.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.agent.is_empty() && !self.work.trim().is_empty()
    }
}

/// The form that describes a piece of work.
///
/// Controlled, like the project form and for the same reason: the control that
/// submits lives in the modal's header, and cannot read state the form owns.
#[component]
fn JobForm(draft: Signal<Wanted>, agents: Vec<Agent>) -> Element {
    let mut draft = draft;

    rsx! {
        div { class: "flex flex-col gap-3",
            // Only when there is a choice. A project with one agent has
            // already made this decision, and a select with one option asks a
            // question that has no other answer.
            if agents.len() > 1 {
                label { class: "flex flex-col gap-1",
                    span { class: "text-xs font-medium text-muted-foreground", "Runs on" }
                    select {
                        class: FIELD,
                        value: "{draft().agent}",
                        onchange: move |event| draft.with_mut(|draft| draft.agent = event.value()),
                        for agent in agents.iter() {
                            option { key: "{agent.id}", value: "{agent.id}", "{agent.name}" }
                        }
                    }
                }
            }
            label { class: "flex flex-col gap-1",
                span { class: "text-xs font-medium text-muted-foreground", "The work" }
                textarea {
                    class: "{FIELD} min-h-40 resize-y",
                    placeholder: "What needs doing, in your own words. Say what \"done\" looks \
                                  like, and name anything the agent should read first.",
                    value: "{draft().work}",
                    oninput: move |event| draft.with_mut(|draft| draft.work = event.value()),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "The agent is told where the repository is, that nothing is checked out, that \
                 its tools are already signed in, and to stop at a proposal rather than merge \
                 anything. You are describing the work, not writing the instruction."
            }
        }
    }
}

/// What every input on this screen looks like.
const FIELD: &str = "w-full rounded-md border border-border bg-surface px-2 py-1.5 \
                     text-sm placeholder:text-faint-foreground focus-visible:outline-none \
                     focus-visible:ring-2 focus-visible:ring-primary";

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::{Standing, forbids, standing};
    use stageman_core::{Agent, Progress};
    use std::collections::{BTreeMap, BTreeSet};

    /// A failure's prose is the only thing a person has to go on.
    #[test]
    fn a_failure_carries_why_it_failed_across() {
        let crossed = standing(&Progress::Failed("its container is gone".to_owned()));

        assert_eq!(
            crossed,
            Standing::Failed {
                why: "its container is gone".to_owned()
            }
        );
    }

    /// A project running jobs on these agents.
    fn running_on(agents: BTreeSet<Agent>) -> stageman_core::Project {
        stageman_core::Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_agent: Agent::Claude,
            job_agents: agents,
            credentials: BTreeMap::new(),
            channels: BTreeMap::new(),
            jobs: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
        }
    }

    /// An agent the project names is allowed, and one it does not is refused.
    ///
    /// The second half needs a project naming no agents, which a valid
    /// instance cannot contain — `State::check` refuses one. That is why this
    /// is tested here against a fixture rather than through a request.
    #[test]
    fn a_project_forbids_exactly_the_agents_it_does_not_name() {
        assert!(!forbids(
            &running_on(BTreeSet::from([Agent::Claude])),
            Agent::Claude
        ));
        assert!(forbids(&running_on(BTreeSet::new()), Agent::Claude));
    }

    #[test]
    fn the_other_two_cross_as_themselves() {
        assert_eq!(standing(&Progress::Working), Standing::Working);
        assert_eq!(standing(&Progress::Idle), Standing::Idle);
    }
}

#[cfg(test)]
mod tests {
    use super::{Standing, Wanted};
    use crate::ui::BadgeTone;

    /// Every standing there is.
    ///
    /// Listed by hand: a variant added without a line here is one nothing
    /// below checks.
    fn every() -> Vec<Standing> {
        vec![
            Standing::Working,
            Standing::Idle,
            Standing::Failed {
                why: "it did not work".to_owned(),
            },
        ]
    }

    /// A running job and a failed one being indistinguishable is the failure
    /// a tone exists to prevent, and this is where the two are decided.
    #[test]
    fn no_two_standings_look_or_read_alike() {
        let tones: Vec<BadgeTone> = every().iter().map(Standing::tone).collect();
        let labels: Vec<&str> = every().iter().map(Standing::label).collect();

        for (position, tone) in tones.iter().enumerate() {
            assert!(
                !tones
                    .iter()
                    .skip(position)
                    .skip(1)
                    .any(|other| other == tone),
                "two standings share a tone: {tones:?}"
            );
        }
        assert!(labels.iter().all(|label| !label.is_empty()));
        assert_eq!(
            labels
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            labels.len(),
            "two standings share a label: {labels:?}"
        );
    }

    /// The control that starts a job is unavailable until pressing it would
    /// work, so this is the whole of that guard.
    #[test]
    fn a_request_needs_both_an_agent_and_some_work() {
        let complete = Wanted {
            agent: "claude".to_owned(),
            work: "document the three missing variables".to_owned(),
        };
        assert!(complete.is_complete());

        let mut without_agent = complete.clone();
        without_agent.agent.clear();
        assert!(!without_agent.is_complete());

        let mut without_work = complete;
        without_work.work.clear();
        assert!(!without_work.is_complete());
    }

    /// Whitespace is not a description of work.
    ///
    /// The route trims before judging, so a form accepting spaces would offer
    /// a control that fails.
    #[test]
    fn whitespace_is_not_work() {
        let asked = Wanted {
            agent: "claude".to_owned(),
            work: "  \n\t ".to_owned(),
        };

        assert!(!asked.is_complete());
    }
}
