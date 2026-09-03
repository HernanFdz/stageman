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
use lucide_dioxus::{ExternalLink, Eye, EyeOff};
use serde::{Deserialize, Serialize};

use super::error::{DashboardError, DashboardResult};
use crate::ui::{Badge, BadgeTone, Button, Card, EmptyState, Modal};

/// Why a job started, when a person started it.
///
/// Filled in rather than asked for. The vocabulary in `docs/conventions.md` §2
/// calls a reason "why the foreman decided to" — and a foreman has
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
    /// What it ran on, in the words a person reads: the agent, and whatever
    /// of its settings differs from that agent's own defaults.
    pub kit: String,
    /// What its session reported it was set to, in the adapter's own words.
    ///
    /// Beside the kit rather than folded into it, because the two were
    /// measured to differ — see `docs/decisions/0048-a-job-runs-on-a-kit.md`.
    /// Empty for a job that has not had a turn, and for every job recorded
    /// before this existed.
    pub reported: Vec<(String, String)>,
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
    /// Where to look at whatever it is showing.
    ///
    /// Built on the server rather than assembled here, because it is made from
    /// the domain this instance answers on and the browser has no business
    /// knowing that — a page that composed its own would be a second place for
    /// the rule in `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`
    /// to live, and the one nobody would update.
    ///
    /// **Always present, and it promises nothing.** Nothing here knows whether
    /// a job has anything listening: the port is published when its container
    /// is created, whether or not its agent ever uses it. So this is an address
    /// rather than a claim, and following it when there is nothing there says
    /// so plainly.
    pub tunnel: String,
}

/// One kit a project offers, as much of it as a page needs to offer it back.
///
/// The name is what a browser sends to start a job on it, and the description
/// is what a person chooses by — the same two things the foreman is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offered {
    /// What the operator called it, and what the browser names it back as.
    pub name: String,
    /// What this project wants it for, in the operator's words.
    pub description: String,
}

/// One project and its jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Working {
    /// What to call the project.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// The kits its jobs may run on. Never empty in a valid instance.
    pub kits: Vec<Offered>,
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
/// Fails if the project is unknown, if the work is empty, or if the project
/// offers no kit under that name.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/projects/{project}/jobs/start", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn start(project: String, kit: String, work: String) -> DashboardResult<Working> {
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
    let (identifier, chosen) = {
        let state = instance.0.read();
        let found = super::identify(&state, &project);
        // Asked before starting rather than after failing: a project's kits
        // are the only kits — `docs/decisions/0048-a-job-runs-on-a-kit.md`,
        // and by hand as much as by the foreman — so a name outside them is a
        // request this instance refuses rather than a handout it cannot
        // decide.
        let decided = found.and_then(|found| {
            let watched =
                state
                    .projects
                    .get(&found)
                    .ok_or_else(|| DashboardError::UnknownProject {
                        id: project.clone(),
                    })?;
            offered(watched, &kit)
                .map(|chosen| (found, chosen))
                .ok_or_else(|| DashboardError::KitNotOnProject {
                    name: kit.clone(),
                    project: watched.name.clone(),
                })
        });
        drop(state);
        decided?
    };

    let started =
        crate::begin(&instance.0, identifier, chosen, BY_HAND, work).map_err(|reason| {
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

/// The kit this project offers under a name, if it offers one.
///
/// A function rather than a lookup at the call site so that the refusal can be
/// tested against a fixture: through a request, a project always offers at
/// least one kit, and the interesting case is the name it does not offer. The
/// name is read the way `KitName` reads it, so a name with space around it
/// still names the kit — and a blank one names nothing.
#[cfg(feature = "server")]
fn offered(project: &stageman_core::Project, name: &str) -> Option<stageman_core::Kit> {
    let wanted = stageman_core::KitName::new(name).ok()?;
    project.kits.get(&wanted).map(|offered| offered.kit.clone())
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
            kit: super::described(job.kit()),
            reported: job
                .reported
                .iter()
                .map(|(option, value)| (option.clone(), value.clone()))
                .collect(),
            reason: job.reason.clone(),
            kickoff: job.kickoff.clone(),
            created_at: job.created_at.to_string(),
            standing: standing(&job.progress),
            tunnel: crate::tunnel::showing(*id),
        })
        .collect();
    jobs.sort_by(|one, other| other.created_at.cmp(&one.created_at));

    Ok(Working {
        name: watched.name.clone(),
        repository: watched.repository.clone(),
        kits: watched
            .kits
            .iter()
            .map(|(name, offered)| Offered {
                name: name.to_string(),
                description: offered.description.clone(),
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
                                    // The first kit the project offers, which
                                    // is the one a select with a single option
                                    // would have chosen anyway.
                                    let first = working.kits.first().map(|kit| kit.name.clone());
                                    move |_| {
                                        draft.set(Wanted {
                                            kit: first.clone().unwrap_or_default(),
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
                                            match start(identifier, asked.kit, asked.work).await {
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
                            JobForm { draft, kits: working.kits }
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
                    "{job.kit} · {job.created_at}"
                }
            }
            if let Standing::Failed { why } = &job.standing {
                p { class: "text-xs text-failed", "{why}" }
            }
            // What the session said it was set to, in the adapter's spelling,
            // beside what was asked for above. Shown whenever there is
            // anything, because the one case worth seeing is the two
            // disagreeing — and a reader cannot spot a disagreement that is
            // only shown when it occurs.
            if !job.reported.is_empty() {
                p { class: "font-mono text-xs text-faint-foreground",
                    "reported "
                    {job.reported.iter().map(|(option, value)| format!("{option} {value}")).collect::<Vec<_>>().join(" · ")}
                }
            }
            // Icons rather than words, because a row of jobs is a list and a
            // list reads better as shapes. Both carry an accessible name and a
            // tooltip: an icon-only control with neither is a puzzle, and the
            // tooltip is the only thing that says which address the second one
            // goes to.
            //
            // The glyphs are deliberately not hidden from assistive technology
            // and do not need to be. A label on the control replaces whatever
            // its contents would have computed, so an unnamed drawing inside
            // one contributes nothing to say twice.
            div { class: "flex items-center gap-1",
                button {
                    r#type: "button",
                    // Padded and pulled back, so the target is bigger than the
                    // shape without moving anything around it.
                    class: "-m-1 rounded p-1 text-muted-foreground hover:text-foreground \
                            focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                    onclick: move |_| showing.toggle(),
                    aria_label: if showing() { "Hide what it was told" } else { "What it was told" },
                    title: if showing() { "Hide what it was told" } else { "What it was told" },
                    if showing() {
                        EyeOff { size: 16, class: "shrink-0" }
                    } else {
                        Eye { size: 16, class: "shrink-0" }
                    }
                }
                // In a tab of its own, and told to carry nothing there. What
                // is on the other side is an application this instance's agent
                // wrote, so it gets neither a handle on the page that opened
                // it nor the address that page was at.
                //
                // An arrow leaving a frame rather than an eye, and the
                // distinction is worth keeping: an eye means *reveal this*, as
                // the control beside it does, and this one navigates away.
                a {
                    class: "-m-1 rounded p-1 text-muted-foreground hover:text-foreground \
                            focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                    href: "{job.tunnel}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    aria_label: "Look at what it is showing",
                    title: "Look at what it is showing — {job.tunnel}",
                    ExternalLink { size: 16, class: "shrink-0" }
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
/// The work and which kit, and nothing else. Not the instruction: that is
/// composed from this, and composing it here would put an author of
/// instructions outside the one crate allowed to be one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wanted {
    /// Which of the project's kits should do it, by name.
    pub kit: String,
    /// What to do, in the operator's own words.
    pub work: String,
}

impl Wanted {
    /// Whether this says enough to start a job.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.kit.trim().is_empty() && !self.work.trim().is_empty()
    }
}

/// The form that describes a piece of work.
///
/// Controlled, like the project form and for the same reason: the control that
/// submits lives in the modal's header, and cannot read state the form owns.
#[component]
fn JobForm(draft: Signal<Wanted>, kits: Vec<Offered>) -> Element {
    let mut draft = draft;

    rsx! {
        div { class: "flex flex-col gap-3",
            // Only when there is a choice. A project with one kit has already
            // made this decision, and a select with one option asks a question
            // that has no other answer.
            if kits.len() > 1 {
                label { class: "flex flex-col gap-1",
                    span { class: "text-xs font-medium text-muted-foreground", "Runs on" }
                    select {
                        class: FIELD,
                        value: "{draft().kit}",
                        onchange: move |event| draft.with_mut(|draft| draft.kit = event.value()),
                        for kit in kits.iter() {
                            option {
                                key: "{kit.name}",
                                value: "{kit.name}",
                                title: "{kit.description}",
                                "{kit.name} — {kit.description}"
                            }
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
    use super::{Standing, offered, standing};
    use stageman_core::{Agent, Kit, KitConfig, KitName, Progress};
    use std::collections::BTreeMap;

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

    /// A project offering these kits.
    fn running_on(kits: BTreeMap<KitName, KitConfig>) -> stageman_core::Project {
        stageman_core::Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_kit: Kit::defaults(Agent::Claude),
            kits,
            credentials: BTreeMap::new(),
            channels: BTreeMap::new(),
            jobs: BTreeMap::new(),
            variables: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
        }
    }

    /// A kit the project offers is found by its name, and a name it does not
    /// offer finds nothing.
    ///
    /// The second half is what a request cannot reach through a valid
    /// instance, since the form only offers names the project has — so it is
    /// tested here against a fixture. The trimmed case is the one a select
    /// cannot produce and a hand-written request can.
    #[test]
    fn a_project_offers_exactly_the_kits_it_names() {
        let quick = KitConfig {
            description: "for small fixes".to_owned(),
            kit: Kit::Claude {
                model: stageman_core::ClaudeModel::Haiku,
            },
        };
        let project = running_on(BTreeMap::from([(
            KitName::new("quick").expect("a name"),
            quick.clone(),
        )]));

        assert_eq!(offered(&project, "quick"), Some(quick.kit.clone()));
        assert_eq!(
            offered(&project, " quick "),
            Some(quick.kit),
            "named, if untidily"
        );
        assert_eq!(offered(&project, "deep"), None);
        assert_eq!(offered(&project, ""), None, "a blank names nothing");
        assert_eq!(offered(&running_on(BTreeMap::new()), "quick"), None);
    }

    #[test]
    fn the_other_two_cross_as_themselves() {
        assert_eq!(standing(&Progress::Working), Standing::Working);
        assert_eq!(standing(&Progress::Idle), Standing::Idle);
    }

    /// Every job crosses with the address that reaches that job.
    ///
    /// Worth asserting on the way across rather than trusting the builder: the
    /// address is made from an identifier, and a page showing one job's link
    /// under another's is a person looking at the wrong thing and being sure
    /// they are looking at the right one.
    #[test]
    fn a_job_crosses_with_the_address_that_reaches_it() {
        let mut watched = running_on(BTreeMap::from([(
            KitName::new("Claude").expect("a name"),
            KitConfig::defaults(Agent::Claude),
        )]));
        for which in 1..=2_u128 {
            let job = stageman_core::JobId::from_uuid(stageman_core::Uuid::from_u128(which));
            watched.jobs.insert(
                job,
                stageman_core::Job::new(
                    stageman_core::Kit::defaults(Agent::Claude),
                    "because".to_owned(),
                    "do the thing".to_owned(),
                    stageman_core::Timestamp::now(),
                ),
            );
        }

        let project = stageman_core::ProjectId::from_uuid(stageman_core::Uuid::from_u128(7));
        let mut state = stageman_core::State::default();
        state.agents.insert(
            Agent::Claude,
            stageman_core::AgentConfig {
                auth_token: stageman_core::Secret::new("a-credential".to_owned()),
            },
        );
        state.projects.insert(project, watched);

        let shown = super::working(&state, &project.to_string()).expect("a watched project");

        assert_eq!(shown.jobs.len(), 2);
        for job in &shown.jobs {
            assert!(
                job.tunnel
                    .contains(&format!("{}.{}", job.id, *crate::tunnel::DOMAIN)),
                "a job's address has to name that job: {job:?}",
            );
        }
        assert_ne!(
            shown.jobs[0].tunnel, shown.jobs[1].tunnel,
            "two jobs never share one",
        );
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
    fn a_request_needs_both_a_kit_and_some_work() {
        let complete = Wanted {
            kit: "Claude".to_owned(),
            work: "document the three missing variables".to_owned(),
        };
        assert!(complete.is_complete());

        let mut without_kit = complete.clone();
        without_kit.kit.clear();
        assert!(!without_kit.is_complete());

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
            kit: "Claude".to_owned(),
            work: "  \n\t ".to_owned(),
        };

        assert!(!asked.is_complete());
    }
}
