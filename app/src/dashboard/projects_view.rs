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

use std::fmt;

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
/// have one and which channels are bound is here; what any of them is, is not
/// — and a channel's address is withheld on the same terms even though it is
/// not a secret, because nothing on a screen needs it.
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
    /// The channels bound to it. Empty is valid: a project with nowhere to
    /// escalate can still run work that never needs to ask — see
    /// `docs/decisions/0005-conversation-happens-on-channels.md`.
    pub channels: Vec<String>,
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
/// instance runs, if a channel was given one half of a binding, or if the
/// result would not be a valid instance.
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
    channel: ChannelDraft,
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
    let channels = binding(&channel)?;

    let mut state = instance.0.update();

    // Asked of a copy before it is asked of the instance. `State::check` is
    // the one definition of valid, and the store consults it on write — where
    // it *logs* a refusal rather than returning one. Mutating first would
    // therefore leave an instance that is invalid in memory and correct on
    // disk, with only a log line saying so.
    // Checked here rather than in `State::check`, and the distinction matters.
    // `State::check` is consulted when a snapshot is *read*, so a rule added
    // there refuses to open an instance that already breaks it — putting the
    // repair behind the door it just locked, which is the trap
    // `docs/conventions.md` §3 names. An instance with two projects on one
    // channel is ambiguous rather than unusable, so it is refused at the point
    // somebody creates one and tolerated in a file that already has it.
    if let Some(taken) = already_bound(&state, &channels) {
        drop(state);
        return Err(DashboardError::ChannelAlreadyBound { project: taken });
    }

    let mut candidate = state.clone();
    let created = stageman_core::ProjectId::from_uuid(uuid::Uuid::new_v4());
    candidate.projects.insert(
        created,
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
            channels,
            jobs: std::collections::BTreeMap::new(),
        },
    );
    candidate
        .check()
        .map_err(|reason| DashboardError::from_inconsistent(&reason, super::shown))?;

    *state = candidate;
    let watching = watching_now(&state);
    // Started here rather than only at startup. Binding a channel with a
    // credential to listen with used to do nothing until the daemon was
    // restarted, and nothing said so — which is indistinguishable from the
    // platform sending nothing, and cost an evening to tell apart.
    let listening = crate::listening_on(&state, created);
    drop(state);

    if let Some(listening) = listening {
        crate::listen_to(&instance.0, &crate::RUNTIME, listening);
    }

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
    let identifier = super::identify(&state, &project)?;
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
    let identifier = super::identify(&state, &project)?;

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

/// What a project's channel bindings are, from the two boxes the form offers.
///
/// Empty, one, or a refusal — and the refusal is the reason this is a function
/// rather than two lines in the route. A binding is two values, neither half
/// works alone, and a half-bound channel looks bound on every screen right up
/// to the moment a job has a question and nowhere to put it.
///
/// One channel, so one pair of fields, in the same way one platform means one
/// credential box above. A second would make this a list here and on the form,
/// and the closed set in the domain is what would force both.
///
/// # Errors
///
/// Fails if exactly one half was given.
#[cfg(feature = "server")]
fn binding(
    channel: &ChannelDraft,
) -> DashboardResult<std::collections::BTreeMap<stageman_core::Channel, stageman_core::ChannelConfig>>
{
    let address = channel.address.trim();
    let credential = channel.credential.trim();
    let listening = channel.listen_credential.trim();

    match (address.is_empty(), credential.is_empty()) {
        // Nothing bound. A credential to listen with and nowhere to listen is
        // the one combination that cannot mean anything, so it is refused
        // rather than dropped — silently ignoring a filled box is how somebody
        // concludes the feature is broken.
        (true, true) if listening.is_empty() => Ok(std::collections::BTreeMap::new()),
        (false, false) => Ok(std::collections::BTreeMap::from([(
            stageman_core::Channel::Slack,
            stageman_core::ChannelConfig {
                address: address.to_owned(),
                credential: stageman_core::Secret::new(credential.to_owned()),
                listen_credential: (!listening.is_empty())
                    .then(|| stageman_core::Secret::new(listening.to_owned())),
            },
        )])),
        _ => Err(DashboardError::ChannelIncomplete),
    }
}

/// The project already bound where these bindings would go, if any is.
///
/// Names it rather than merely refusing, because an operator looking at a
/// channel identifier they just pasted cannot otherwise tell which of their
/// projects has it.
#[cfg(feature = "server")]
fn already_bound(
    state: &stageman_core::State,
    wanted: &std::collections::BTreeMap<stageman_core::Channel, stageman_core::ChannelConfig>,
) -> Option<String> {
    state
        .projects
        .values()
        .find(|project| {
            wanted.iter().any(|(channel, binding)| {
                project
                    .channels
                    .get(channel)
                    .is_some_and(|held| held.address == binding.address)
            })
        })
        .map(|project| project.name.clone())
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
                                                asked.channel,
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
                Link {
                    to: super::Route::ProjectJobsView {
                        project: project.id.clone(),
                    },
                    class: "text-sm font-medium hover:underline",
                    "{project.name}"
                }
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
                // Absence only, in the same way the credential above is. A
                // bound channel needs no announcement; one that is missing
                // changes what the project can be asked to do.
                if project.channels.is_empty() {
                    " · no channel"
                }
            }
        }
    }
}

/// The two boxes that bind a channel, travelling together.
///
/// One type rather than two parameters, and the reason is not tidiness: a
/// binding is two values that are only meaningful together, and a signature
/// taking them apart invites a caller to pass one. It also keeps [`create`]
/// within the argument count the gate allows, which is the lint noticing the
/// same thing.
///
/// Both empty means no channel. Both filled means one. Exactly one filled is
/// refused by [`binding`], which is the only place that rule is written.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChannelDraft {
    /// Where on the channel this project's conversation happens.
    pub address: String,
    /// What reaches that channel.
    pub credential: String,
    /// What listens on it, if this project is to be answered at all.
    ///
    /// Optional where the two above are both-or-neither: speaking without
    /// listening is what this did before there was any listening, and is a
    /// project that escalates and reads its answers somewhere else. Listening
    /// without speaking is not a thing — there would be nothing to reply to.
    pub listen_credential: String,
}

impl fmt::Debug for ChannelDraft {
    /// Names what was given and never the credential.
    ///
    /// Hand-written per `docs/conventions.md` §4. This one is not a case where
    /// deriving would happen to be safe: the credential is a bare `String` on
    /// the way in from a browser, so nothing under it redacts, and a derive
    /// would print it whole the first time somebody logs a request.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelDraft")
            .field("address", &self.address)
            .field("credential", &"<redacted>")
            .field("listen_credential", &"<redacted>")
            .finish()
    }
}

/// Everything the form collects, which is everything a project is.
///
/// A struct rather than five handlers, because the form's whole purpose is to
/// be filled in twice — once to create and once to change — and a caller
/// should differ in what it *does* with the answer rather than in how it
/// receives it.
#[derive(Clone, PartialEq, Eq, Default)]
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
    /// Where this project's conversation happens, if anywhere. Optional, and
    /// the only part of a draft that may be left blank.
    pub channel: ChannelDraft,
}

impl fmt::Debug for Draft {
    /// Names the fields and neither credential.
    ///
    /// `docs/conventions.md` §4 again, and this type is why the rule has no
    /// exception for "it is only the browser's": a draft holds two credentials
    /// as bare `String`s, and a derive would print both. That was true of the
    /// repository credential before the channel arrived; adding a second is
    /// what made it worth fixing rather than noting.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draft")
            .field("name", &self.name)
            .field("repository", &self.repository)
            .field("orchestrator", &self.orchestrator)
            .field("job_agents", &self.job_agents)
            .field("credential", &"<redacted>")
            .field("channel", &self.channel)
            .finish()
    }
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
    ///
    /// The channel is the one part that is *optional* rather than required,
    /// and it is still checked: both halves or neither, which is the rule
    /// [`binding`] enforces on the far side. A pair where one box is filled is
    /// the mistake this catches, and it is worth catching on the screen
    /// because the operator is looking at the empty box.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.repository.trim().is_empty()
            && !self.orchestrator.is_empty()
            && !self.job_agents.is_empty()
            && !self.credential.trim().is_empty()
            && self.channel.address.trim().is_empty() == self.channel.credential.trim().is_empty()
            // Listening needs somewhere to listen. The reverse is fine.
            && (self.channel.listen_credential.trim().is_empty()
                || !self.channel.address.trim().is_empty())
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
            Field { label: "Slack channel (optional)",
                input {
                    class: FIELD,
                    placeholder: "C0123456789",
                    value: "{draft().channel.address}",
                    oninput: move |event| {
                        draft.with_mut(|draft| draft.channel.address = event.value());
                    },
                }
            }
            Field { label: "Slack credential (optional)",
                input {
                    r#type: "password",
                    class: FIELD,
                    placeholder: "a bot token that can post there",
                    value: "{draft().channel.credential}",
                    oninput: move |event| {
                        draft.with_mut(|draft| draft.channel.credential = event.value());
                    },
                }
            }
            Field { label: "Slack app-level token (optional)",
                input {
                    r#type: "password",
                    class: FIELD,
                    placeholder: "xapp-… , so replies reach the job",
                    value: "{draft().channel.listen_credential}",
                    oninput: move |event| {
                        draft.with_mut(|draft| draft.channel.listen_credential = event.value());
                    },
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Where a job asks when it hits something it cannot decide. Leave both empty and \
                 this project can only run work that never needs to ask. Give one without the \
                 other and it is refused: neither half works alone."
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
    use super::{ChannelDraft, DashboardError, binding, busy};
    use stageman_core::{ProjectId, State};

    /// The two boxes that bind a channel, plus the optional third.
    fn drafted(address: &str, credential: &str, listening: &str) -> ChannelDraft {
        ChannelDraft {
            address: address.to_owned(),
            credential: credential.to_owned(),
            listen_credential: listening.to_owned(),
        }
    }
    use stageman_core::{Agent, Channel, Job, JobId, Progress, Project, Secret, Timestamp};
    use std::collections::{BTreeMap, BTreeSet};

    /// One job, in whatever state the caller needs it.
    fn job(progress: Progress) -> Job {
        Job {
            agent: Agent::Claude,
            reason: "because a test said so".to_owned(),
            kickoff: "do the thing".to_owned(),
            created_at: Timestamp::UNIX_EPOCH,
            progress,
            thread: None,
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
            channels: BTreeMap::new(),
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

    /// Both boxes empty is a project with nowhere to escalate, which is a
    /// project rather than a failure.
    #[test]
    fn a_project_may_bind_no_channel_at_all() {
        assert!(
            binding(&drafted("", "", ""))
                .expect("neither half is not a refusal")
                .is_empty()
        );
        assert!(
            binding(&drafted("  ", "\t", ""))
                .expect("whitespace is nothing")
                .is_empty()
        );
    }

    /// Both halves reach the domain, on the one channel there is.
    #[test]
    fn both_halves_bind_the_channel() {
        let bound = binding(&drafted(" C0123456789 ", " xoxb-not-a-real-token ", ""))
            .expect("both halves are a binding");

        let slack = bound.get(&Channel::Slack).expect("keyed by the channel");
        // Trimmed, because the boxes either side of a pasted token are the
        // usual way one arrives and a credential with a space is a credential
        // that fails somewhere unhelpful.
        assert_eq!(slack.address, "C0123456789");
        assert_eq!(slack.credential.expose(), "xoxb-not-a-real-token");
        assert_eq!(bound.len(), 1);
    }

    /// Two projects on one channel is refused, and the holder is named.
    ///
    /// The invariant inbound rests on. A message at the root would otherwise
    /// belong to whichever orchestrator the search reached first — an ordering
    /// rather than an answer — and if both listened, both would hear every
    /// message and one job's reply would be delivered twice.
    #[test]
    fn a_channel_another_project_already_binds_is_refused() {
        let mut state = State::default();
        state.projects.insert(
            ProjectId::from_uuid(uuid::Uuid::from_u128(1)),
            Project {
                name: "aviary".to_owned(),
                repository: "https://example.invalid/aviary".to_owned(),
                orchestrator_agent: Agent::Claude,
                job_agents: BTreeSet::from([Agent::Claude]),
                credentials: BTreeMap::new(),
                channels: BTreeMap::from([(
                    Channel::Slack,
                    stageman_core::ChannelConfig {
                        address: "C0123456789".to_owned(),
                        credential: Secret::new("xoxb-token".to_owned()),
                        listen_credential: None,
                    },
                )]),
                jobs: BTreeMap::new(),
            },
        );

        let wanted = binding(&drafted("C0123456789", "xoxb-other", "")).expect("a binding");
        assert_eq!(
            super::already_bound(&state, &wanted),
            Some("aviary".to_owned()),
            "it must name the project holding it, not merely refuse"
        );

        // A different channel on the same instance is fine, which is what
        // stops this refusing every second project.
        let elsewhere = binding(&drafted("C9999999999", "xoxb-other", "")).expect("a binding");
        assert_eq!(super::already_bound(&state, &elsewhere), None);

        // And binding nothing collides with nothing.
        let none = binding(&drafted("", "", "")).expect("no binding");
        assert_eq!(super::already_bound(&state, &none), None);
    }

    /// Listening is optional; listening with nowhere to listen is not.
    ///
    /// The asymmetry is the point. Speaking without listening is what every
    /// project did before there was any listening. A token to listen with and
    /// no channel cannot mean anything, and dropping it silently is how
    /// somebody concludes the feature is broken.
    #[test]
    fn listening_is_optional_but_needs_somewhere_to_listen() {
        let bound = binding(&drafted("C0123456789", "xoxb-token", ""))
            .expect("speaking without listening is a binding");
        assert_eq!(
            bound
                .get(&Channel::Slack)
                .and_then(|slack| slack.listen_credential.as_ref()),
            None
        );

        let listening = binding(&drafted("C0123456789", "xoxb-token", "xapp-token"))
            .expect("both is a binding too");
        assert_eq!(
            listening
                .get(&Channel::Slack)
                .and_then(|slack| slack.listen_credential.as_ref())
                .map(Secret::expose),
            Some("xapp-token")
        );

        assert!(
            matches!(
                binding(&drafted("", "", "xapp-token")),
                Err(DashboardError::ChannelIncomplete)
            ),
            "a credential to listen with and nowhere to listen is refused"
        );
    }

    /// Half a binding is refused rather than stored.
    ///
    /// The failure it prevents is quiet: a project holding an address with no
    /// credential looks bound on every screen, and finds out otherwise at the
    /// one moment it matters — a job with a question and nowhere to put it.
    #[test]
    fn half_a_binding_is_refused() {
        // Matched rather than compared: `ChannelConfig` has no `PartialEq`,
        // and giving a secret-bearing type one so that a test could use
        // `assert_eq!` would widen the domain for the convenience of this
        // line. Only `Secret` carries that in this codebase.
        assert!(matches!(
            binding(&drafted("C0123456789", "", "")),
            Err(DashboardError::ChannelIncomplete)
        ));
        assert!(matches!(
            binding(&drafted("", "xoxb-not-a-real-token", "")),
            Err(DashboardError::ChannelIncomplete)
        ));
    }

    /// A credential given to this never comes back out of it in the clear.
    ///
    /// `docs/conventions.md` §4 asks that secrets never render, and a binding
    /// built here is put straight into a project — so this is where a wrapper
    /// that had been forgotten would first be visible.
    #[test]
    fn a_bound_credential_does_not_render() {
        let bound =
            binding(&drafted("C0123456789", "xoxb-not-a-real-token", "")).expect("a binding");
        let shown = format!("{bound:?}");

        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
        assert_eq!(
            bound
                .get(&Channel::Slack)
                .map(|slack| Secret::expose(&slack.credential)),
            Some("xoxb-not-a-real-token"),
            "and is still readable when asked"
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
}

#[cfg(test)]
mod tests {
    use super::{ChannelDraft, Draft, Project};

    /// One project, with however many jobs running.
    fn watched(running: usize, jobs: usize) -> Project {
        Project {
            id: "an-identifier".to_owned(),
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            orchestrator: "Claude".to_owned(),
            job_agents: vec!["Claude".to_owned()],
            platforms: Vec::new(),
            channels: Vec::new(),
            running,
            jobs,
        }
    }

    /// A draft the form would accept, with every box filled including the two
    /// that need not be.
    fn filled() -> Draft {
        Draft {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            orchestrator: "claude".to_owned(),
            job_agents: vec!["claude".to_owned()],
            credential: "ghp-not-a-real-token".to_owned(),
            channel: ChannelDraft {
                address: "C0123456789".to_owned(),
                credential: "xoxb-not-a-real-token".to_owned(),
                listen_credential: "xapp-not-a-real-token".to_owned(),
            },
        }
    }

    /// Applies one change to an otherwise complete draft.
    fn without(change: fn(&mut Draft)) -> Draft {
        let mut draft = filled();
        change(&mut draft);
        draft
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
        assert!(!without(|draft| draft.name.clear()).is_complete());
        assert!(!without(|draft| draft.repository.clear()).is_complete());
        assert!(!without(|draft| draft.orchestrator.clear()).is_complete());
        assert!(!without(|draft| draft.job_agents.clear()).is_complete());
        assert!(!without(|draft| draft.credential.clear()).is_complete());
    }

    /// The channel is the one thing a project may go without.
    ///
    /// `docs/decisions/0005-conversation-happens-on-channels.md` says a
    /// project with nothing bound can still run work that never needs to ask,
    /// so a form demanding one would refuse a project the domain accepts.
    #[test]
    fn a_draft_binding_no_channel_is_still_complete() {
        assert!(
            without(|draft| {
                draft.channel = ChannelDraft::default();
            })
            .is_complete()
        );
    }

    /// Half a binding is the mistake worth catching on the screen.
    ///
    /// Neither half works alone, and the operator is looking at the box they
    /// left empty — so the control that submits goes unavailable rather than
    /// the instance refusing after the fact.
    #[test]
    fn a_draft_binding_half_a_channel_is_not_complete() {
        assert!(!without(|draft| draft.channel.address.clear()).is_complete());
        assert!(!without(|draft| draft.channel.credential.clear()).is_complete());
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

        // And a channel half-filled with spaces is half-filled, which is what
        // the route decides after trimming. A screen that judged before
        // trimming would offer a control the instance then refuses.
        let mut draft = filled();
        draft.channel.address = "  ".to_owned();
        assert!(!draft.is_complete());
    }

    /// `docs/conventions.md` §4, for the two credentials a draft holds.
    ///
    /// The one place in this crate where a credential sits in a bare `String`
    /// rather than behind `Secret` — it has just arrived from a form and has
    /// not reached the domain yet — so nothing underneath redacts and the
    /// formatter is the whole of the defence.
    #[test]
    fn a_draft_does_not_leak_either_credential_when_formatted() {
        let shown = format!("{:?}", filled());

        assert!(!shown.contains("ghp-not-a-real-token"), "{shown}");
        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
        assert!(
            shown.contains("aviary"),
            "it should still say what it holds"
        );
        assert!(
            shown.contains("C0123456789"),
            "an address is not a credential"
        );
    }

    /// The same, for the pair on its own — which is what crosses the wire.
    #[test]
    fn a_channel_draft_does_not_leak_its_credential_when_formatted() {
        let shown = format!("{:?}", filled().channel);

        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
        assert!(shown.contains("C0123456789"), "{shown}");
    }

    /// Listening without a channel is refused on the screen too.
    ///
    /// The same rule as the route, so the control that submits goes
    /// unavailable rather than the instance refusing after the fact.
    #[test]
    fn a_draft_listening_with_nowhere_to_listen_is_not_complete() {
        assert!(
            !without(|draft| {
                draft.channel.address.clear();
                draft.channel.credential.clear();
            })
            .is_complete(),
            "a token to listen with and no channel is not a complete draft"
        );

        // And dropping only the listening token is fine, since speaking
        // without listening is an ordinary project.
        assert!(without(|draft| draft.channel.listen_credential.clear()).is_complete());
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
