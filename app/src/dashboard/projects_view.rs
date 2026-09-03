//! The projects an instance watches, and what each one needs in order to work.
//!
//! The screen that makes an instance able to *do* something. It comes after
//! agents because a project names one agent for its foreman and a
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
use crate::ui::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState, Modal};

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
    /// How its foreman's agent is set, as the identifiers a browser sends
    /// back — not the names a person reads. See `projected`, which says why.
    pub foreman: Fitted,
    /// The kits its jobs may run on, as the form edits them. Never empty in a
    /// valid instance. Whole rather than named, because the form has to open
    /// showing what is true, and a kit holds nothing a browser may not see.
    pub kits: Vec<KitDraft>,
    /// The platforms it has a credential for.
    pub platforms: Vec<String>,
    /// The channels bound to it. Empty is valid: a project with nowhere to
    /// escalate can still run work that never needs to ask — see
    /// `docs/decisions/0005-conversation-happens-on-channels.md`.
    pub channels: Vec<String>,
    /// The variables its jobs are given, by name.
    ///
    /// Names and never values, on the same terms as `platforms` above: a value
    /// is a credential, and there is nowhere on this type to put one. The
    /// names are what lets an edit form show what a project carries — which is
    /// why they are here and a channel's address is not, since nothing on a
    /// screen needed that.
    pub variables: Vec<String>,
    /// How many of its jobs are still running.
    pub working: usize,
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
        self.working == 0
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
    /// What each of those can be set to: the models it offers, which of them
    /// take an effort, and the efforts. Built on the server from the domain's
    /// closed sets, because the browser's half cannot name them — see
    /// `docs/decisions/0022-the-browser-never-sees-the-domain.md` — and a form
    /// that listed them itself would be a second copy nothing holds to the
    /// first.
    pub shapes: Vec<Shape>,
}

/// One agent, set a particular way, as a browser edits it.
///
/// Identifiers throughout, on the terms [`Project`]'s agent has always used:
/// these are what a browser sends back, and the names a person reads come from
/// the [`Shape`] the server sent alongside. The effort is empty where the model
/// offers none, which is the one shape of absence this type has.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Fitted {
    /// Which agent.
    pub agent: String,
    /// Which of its models.
    pub model: String,
    /// How hard it thinks, or empty where the model has no such choice.
    pub effort: String,
}

impl Fitted {
    /// Whether this names an agent and a model.
    ///
    /// The effort is not required here because the screen cannot know whether
    /// the model takes one without the shape, and the far side refuses a model
    /// given none. What the screen can see is that nothing was chosen at all.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.agent.is_empty() && !self.model.is_empty()
    }
}

/// One kit a project offers, as a browser edits it.
///
/// The name and the description are the operator's, and the description is
/// required: it is the whole of what the foreman chooses by — see
/// `docs/decisions/0048-a-job-runs-on-a-kit.md` — so a kit without one is half
/// a kit.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KitDraft {
    /// What to call it, and what a foreman says to choose it.
    pub name: String,
    /// What this project wants it for.
    pub description: String,
    /// The agent, set a particular way.
    pub fitted: Fitted,
}

impl KitDraft {
    /// Whether this says enough to be a kit.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.description.trim().is_empty()
            && self.fitted.is_complete()
    }
}

/// What one agent can be set to, as the choices a form offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    /// The agent, by identifier.
    pub agent: String,
    /// Its models, the default first.
    pub models: Vec<ModelChoice>,
    /// Its effort levels, the default first, for the models that take one.
    pub efforts: Vec<Choice>,
}

/// One model a form may choose, and whether choosing it opens an effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// What the browser sends back.
    pub id: String,
    /// What a person reads.
    pub name: String,
    /// Whether this model takes an effort at all.
    pub has_effort: bool,
}

/// One value a form may choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// What the browser sends back.
    pub id: String,
    /// What a person reads.
    pub name: String,
}

/// An agent as it comes: the shape's first model, and its first effort where
/// that model takes one.
///
/// What a new kit starts on, and what a fitted agent moves to when its agent
/// changes — nothing carries over between agents, because a model is one
/// agent's and not another's.
fn seeded(shape: &Shape) -> Fitted {
    let model = shape.models.first();
    Fitted {
        agent: shape.agent.clone(),
        model: model.map(|model| model.id.clone()).unwrap_or_default(),
        effort: model
            .filter(|model| model.has_effort)
            .and_then(|_| shape.efforts.first())
            .map(|effort| effort.id.clone())
            .unwrap_or_default(),
    }
}

/// The shape describing an agent, if the server sent one for it.
///
/// A function rather than a `find` at each of its three call sites, so that the
/// comparison it turns on is tested once — mutation testing inverted it inside
/// the component, where nothing could notice.
fn shape_for<'a>(shapes: &'a [Shape], agent: &str) -> Option<&'a Shape> {
    shapes.iter().find(|shape| shape.agent == agent)
}

/// Whether a model takes an effort, as its agent's shape says.
///
/// False for a model the shape does not list at all, which the form cannot
/// produce and a request written by hand can: the far side refuses it by name,
/// and offering an effort select for it here would be offering a second thing
/// to refuse.
fn takes_effort(shape: &Shape, model: &str) -> bool {
    shape
        .models
        .iter()
        .any(|choice| choice.id == model && choice.has_effort)
}

/// A fitted agent moved to another agent.
///
/// That agent's defaults, or — for an identifier no shape describes, which the
/// form cannot produce — the identifier alone, so that the refusal on the far
/// side names it rather than a substitute.
fn with_agent(shapes: &[Shape], agent: &str) -> Fitted {
    shape_for(shapes, agent).map_or_else(
        || Fitted {
            agent: agent.to_owned(),
            ..Fitted::default()
        },
        seeded,
    )
}

/// A fitted agent moved to another of its models.
///
/// The effort is kept where the new model takes one, cleared where it does
/// not, and given the first where the old model had none — so what the form
/// shows is always something the far side will accept.
fn with_model(fitted: &Fitted, shape: &Shape, model: &str) -> Fitted {
    let effort = if !takes_effort(shape, model) {
        String::new()
    } else if fitted.effort.is_empty() {
        shape
            .efforts
            .first()
            .map(|effort| effort.id.clone())
            .unwrap_or_default()
    } else {
        fitted.effort.clone()
    };
    Fitted {
        agent: fitted.agent.clone(),
        model: model.to_owned(),
        effort,
    }
}

/// Whether no two kits share a name, once the space around each is gone.
///
/// The domain keys kits on their names and would keep one of two silently, so
/// the screen refuses to submit the pair — the far side refuses it too, with
/// the name, for a request the form did not make.
fn distinct(kits: &[KitDraft]) -> bool {
    let names: std::collections::BTreeSet<&str> = kits.iter().map(|kit| kit.name.trim()).collect();
    names.len() == kits.len()
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
    Ok(watching_now(&state))
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
pub async fn create(draft: Draft) -> DashboardResult<Watching> {
    let name = required("name", &draft.name)?;
    let repository = required("repository", &draft.repository)?;
    let foreman_kit = super::kit_of(&draft.foreman)?;
    let credential = required("credential", &draft.credential)?;
    let kits = kits_of(&draft.kits)?;
    let channels = binding(&draft.channel)?;
    // Nothing is held yet, so every row here has to carry its own value —
    // which `resolved` says by refusing a new name with an empty one.
    let variables = resolved(&std::collections::BTreeMap::new(), &draft.variables)?;

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
            foreman_kit,
            kits,
            // One platform, so one field. A second would make this a list
            // here and on the form, and the closed set in the domain is what
            // would force both.
            credentials: std::collections::BTreeMap::from([(
                stageman_core::Platform::GitHub,
                stageman_core::Secret::new(credential),
            )]),
            channels,
            variables,
            jobs: std::collections::BTreeMap::new(),
            attending: stageman_core::Attending::default(),
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

/// Changes what a project is, leaving what it has done alone.
///
/// The second caller of the one form, and the reason [`Draft`] exists rather
/// than five parameters. It replaces a narrower route that set a platform
/// credential and nothing else: that route had no caller on any screen, so an
/// expiring token could not in fact be replaced from the dashboard, which is
/// the whole gap this closes. Folding it in here rather than leaving both is
/// the same instinct as `State::check` — two ways to change one field
/// eventually disagree, and the one nobody is reading is the one that lets
/// something through.
///
/// **A blank credential means the one it already has, never none.** There is
/// nowhere on the wire for the current value — `Project` has no field for a
/// credential and must not grow one — so the box an operator sees always
/// starts empty, and treating empty as *clear it* would silently disarm a
/// project every time somebody corrected its name.
///
/// Its jobs, its channel binding and its inbox are untouched. The channel is
/// deliberate rather than pending: a binding's address never reaches the
/// browser, so an edit form cannot show what it would be replacing, and three
/// empty boxes that mean *unchanged* are indistinguishable from three that
/// mean *unbind*.
///
/// Nothing here refuses a project with work in flight, unlike [`forget`]. A
/// running job already holds everything it was given — its container's
/// environment was fixed when it was created — so amending changes what the
/// *next* job gets and cannot reach into one that is going.
///
/// # Errors
///
/// Fails if the project is unknown, if anything required is missing, if an
/// agent named is not one this instance runs, or if the result would not be a
/// valid instance.
#[cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "the shape a server function is required to have"
    )
)]
#[post("/api/projects/amend", instance: Extension<std::sync::Arc<crate::Store>>)]
pub async fn amend(project: String, draft: Draft) -> DashboardResult<Watching> {
    let name = required("name", &draft.name)?;
    let repository = required("repository", &draft.repository)?;
    let foreman_kit = super::kit_of(&draft.foreman)?;
    let kits = kits_of(&draft.kits)?;
    let credential = draft.credential.trim();
    // The channel arrives and is ignored, because amending does not offer one
    // — see this route's own note on why an empty box could not mean
    // *unchanged* there the way it does for a credential.

    let mut state = instance.0.update();
    let identifier = super::identify(&state, &project)?;

    // Asked of a copy before it is asked of the instance, for the reason
    // `create` gives: the store consults `State::check` on write and *logs* a
    // refusal rather than returning one, so mutating first would leave an
    // instance invalid in memory and correct on disk.
    let mut candidate = state.clone();
    let Some(watched) = candidate.projects.get_mut(&identifier) else {
        drop(state);
        return Err(DashboardError::UnknownProject { id: project });
    };
    // Decided against what the project already holds, which is what lets an
    // empty box mean *keep this one* rather than *clear it*.
    watched.variables = resolved(&watched.variables, &draft.variables)?;
    amended(watched, name, repository, foreman_kit, kits, credential);
    candidate
        .check()
        .map_err(|reason| DashboardError::from_inconsistent(&reason, super::shown))?;

    *state = candidate;
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
    if let Some(working) = busy(watched) {
        let name = watched.name.clone();
        drop(state);
        return Err(DashboardError::ProjectBusy { name, working });
    }

    state.projects.remove(&identifier);
    let watching = watching_now(&state);
    drop(state);

    Ok(watching)
}

/// What a project's variables become, from the rows the form came back with.
///
/// A function rather than lines inside two routes, for the reason `binding` is
/// one: every rule worth having lives here, and both callers get the same
/// answer by construction rather than by being written twice.
///
/// Four refusals, and each is a silent wrong answer avoided. A name a
/// container could not be given would reach an argument list — see
/// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md` on
/// what an equals sign does there. A name stageman delivers itself would
/// change which account an agent bills, with nothing anywhere saying so. Two
/// rows with one name have no resolution that does not throw away something
/// somebody typed. And a *new* variable with no value would store an empty
/// credential, which reads as configured and authenticates as nothing.
///
/// Everything not named in `rows` is dropped, which is how removal is said —
/// there is no other way to say it, because an empty value already means
/// *keep*.
///
/// # Errors
///
/// Any of the four above, each naming the row rather than its contents. That
/// is deliberate: the mistake this most often catches is a credential pasted
/// into a name box, and an error repeating it would put it in a log.
#[cfg(feature = "server")]
fn resolved(
    held: &std::collections::BTreeMap<stageman_core::VariableName, stageman_core::Secret>,
    rows: &[VariableDraft],
) -> DashboardResult<std::collections::BTreeMap<stageman_core::VariableName, stageman_core::Secret>>
{
    let mut wanted = std::collections::BTreeMap::new();

    // Counted from one, because the operator is looking at a list rather than
    // at an index — and counted by the range rather than by adding to an
    // index, so there is no arithmetic here to be wrong about.
    for (position, row) in (1..).zip(rows) {
        let name = stageman_core::VariableName::new(row.name.trim()).map_err(|rule| {
            DashboardError::VariableNameRefused {
                position,
                rule: rule.to_string(),
            }
        })?;
        if stageman_agent::RESERVED.contains(&name.as_str()) {
            return Err(DashboardError::VariableReserved {
                name: name.to_string(),
            });
        }
        if wanted.contains_key(&name) {
            return Err(DashboardError::VariableRepeated { position });
        }

        let given = row.value.trim();
        let value = if given.is_empty() {
            held.get(&name)
                .cloned()
                .ok_or(DashboardError::VariableValueMissing)?
        } else {
            stageman_core::Secret::new(given.to_owned())
        };
        wanted.insert(name, value);
    }

    Ok(wanted)
}

/// Applies what the form came back with to the project it names.
///
/// A function rather than six lines inside the route, for the same reason
/// `busy` is one: the rule worth testing is what a **blank credential** means,
/// and a server function cannot be reached without a request. Leaving it
/// inline would make the one rule here that could quietly disarm a project the
/// only one with no test.
///
/// Blank means the credential already held, never none — see [`amend`], which
/// is where that is argued. A project that had none and is amended with a
/// blank box still has none, which is the same rule and not a special case.
#[cfg(feature = "server")]
fn amended(
    watched: &mut stageman_core::Project,
    name: String,
    repository: String,
    foreman_kit: stageman_core::Kit,
    kits: std::collections::BTreeMap<stageman_core::KitName, stageman_core::KitConfig>,
    credential: &str,
) {
    watched.name = name;
    watched.repository = repository;
    // Replaced whole, both of them, because the form shows and resubmits the
    // whole of each: a kit that survives is one the operator left on the
    // form, and a change to the foreman's lands at its next turn boundary —
    // see `docs/decisions/0048-a-job-runs-on-a-kit.md`.
    watched.foreman_kit = foreman_kit;
    watched.kits = kits;
    if !credential.is_empty() {
        watched.credentials.insert(
            stageman_core::Platform::GitHub,
            stageman_core::Secret::new(credential.to_owned()),
        );
    }
}

/// The kits a form described, refusing what the domain would silently mend.
///
/// The domain keys kits on their names, so two rows under one name would keep
/// one and lose the other without a word; a blank name is not a name; and a
/// description is required because it is the whole of what a foreman chooses
/// by — `docs/decisions/0048-a-job-runs-on-a-kit.md`. Each agent's settings
/// are checked by `kit_of`, which refuses rather than mends for the same
/// reason.
///
/// # Errors
///
/// Fails if there are no kits, if one has no name or no description, if two
/// share a name, or if one describes settings this build does not know.
#[cfg(feature = "server")]
fn kits_of(
    drafts: &[KitDraft],
) -> DashboardResult<std::collections::BTreeMap<stageman_core::KitName, stageman_core::KitConfig>> {
    if drafts.is_empty() {
        return Err(DashboardError::KitsMissing);
    }
    let mut kits = std::collections::BTreeMap::new();
    for draft in drafts {
        let name = stageman_core::KitName::new(draft.name.as_str()).map_err(|_| {
            DashboardError::Incomplete {
                field: "kit name".to_owned(),
            }
        })?;
        let description = required("kit description", &draft.description)?;
        let kit = super::kit_of(&draft.fitted)?;
        if kits
            .insert(name.clone(), stageman_core::KitConfig { description, kit })
            .is_some()
        {
            return Err(DashboardError::KitNameTaken {
                name: name.to_string(),
            });
        }
    }
    Ok(kits)
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
        .filter(|job| job.progress == stageman_core::Progress::Working)
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
        // For the same agents `available` lists, so the form never meets an
        // agent it has no shape for.
        shapes: stageman_core::Agent::ALL
            .iter()
            .filter(|agent| state.agents.contains_key(agent))
            .map(|agent| super::shape_of(*agent))
            .collect(),
    }
}

/// The projects screen.
#[component]
pub fn ProjectsView() -> Element {
    let mut reading = use_server_future(projects)?;
    let mut failure = use_signal(|| None::<DashboardError>);
    // What the form is open for, if it is open at all. One signal rather than
    // a flag per purpose — see [`Filling`].
    let mut filling = use_signal(|| None::<Filling>);
    let mut draft = use_signal(Draft::default);

    rsx! {
        div { class: "flex flex-col gap-4",
            match reading.cloned() {
                Some(Ok(watching)) => {
                    // What the project being amended holds now, which is what
                    // decides whether an empty value box means *keep*. Derived
                    // once, so the hint on a box and the control that submits
                    // cannot disagree about the same question — and outside
                    // the markup below, because that is the one place a `let`
                    // cannot go.
                    //
                    // Empty while creating, and that is the true answer rather
                    // than a missing one: a project that does not exist yet
                    // holds nothing.
                    let held: Vec<String> = match filling() {
                        Some(Filling::Amending(ref id)) => watching
                            .projects
                            .iter()
                            .find(|project| &project.id == id)
                            .map(|project| project.variables.clone())
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    rsx! {
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
                                    // The first configured agent as it comes,
                                    // which is what the foreman and the one
                                    // starting kit are seeded with. The kit is
                                    // named and described after the agent, so
                                    // a project with nothing particular to say
                                    // can be saved as it opens — and a kit
                                    // that says more is a row edited rather
                                    // than a row invented.
                                    let first = watching
                                        .shapes
                                        .first()
                                        .map(|shape| {
                                            let fitted = seeded(shape);
                                            let agent = watching
                                                .available
                                                .iter()
                                                .find(|agent| agent.id == shape.agent);
                                            KitDraft {
                                                name: agent
                                                    .map(|agent| agent.name.clone())
                                                    .unwrap_or_default(),
                                                description: agent
                                                    .map(|agent| agent.description.clone())
                                                    .unwrap_or_default(),
                                                fitted,
                                            }
                                        });
                                    move |_| {
                                        // Emptied on the way in rather than on
                                        // the way out, so that a modal
                                        // abandoned half-filled does not
                                        // reopen holding what was abandoned.
                                        draft.set(Draft {
                                            foreman: first
                                                .as_ref()
                                                .map(|kit| kit.fitted.clone())
                                                .unwrap_or_default(),
                                            kits: first.clone().into_iter().collect(),
                                            ..Draft::default()
                                        });
                                        failure.set(None);
                                        filling.set(Some(Filling::Creating));
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
                                for project in watching.projects.iter().cloned() {
                                    li { key: "{project.id}",
                                        WatchedProject {
                                            project: project.clone(),
                                            available: watching.available.clone(),
                                            // Seeded from what the row already
                                            // holds, so the form opens showing
                                            // what is true rather than blank.
                                            // The credential and the channel
                                            // cannot be among them: neither
                                            // ever reaches a browser.
                                            onedit: {
                                                move |()| {
                                                    draft
                                                        .set(Draft {
                                                            name: project.name.clone(),
                                                            repository: project.repository.clone(),
                                                            foreman: project.foreman.clone(),
                                                            kits: project.kits.clone(),
                                                            credential: String::new(),
                                                            channel: ChannelDraft::default(),
                                                            // Names with empty
                                                            // values: the row
                                                            // says keep, and
                                                            // deleting it says
                                                            // remove.
                                                            variables: project
                                                                .variables
                                                                .iter()
                                                                .map(|name| VariableDraft {
                                                                    name: name.clone(),
                                                                    value: String::new(),
                                                                })
                                                                .collect(),
                                                        });
                                                    failure.set(None);
                                                    filling.set(Some(Filling::Amending(project.id.clone())));
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(open) = filling() {
                        Modal {
                            title: if open == Filling::Creating { "New project" } else { "Edit project" },
                            onclose: move |()| filling.set(None),
                            actions: rsx! {
                                Button {
                                    // Unavailable until pressing it would
                                    // work, which is this screen's whole
                                    // answer to an incomplete form for now —
                                    // per-field messages are the better
                                    // answer and are not this change.
                                    class: "px-2.5 text-base leading-none",
                                    aria_label: "Save",
                                    title: "Save",
                                    disabled: !draft().is_complete(&open, &held),
                                    onclick: {
                                        let open = open.clone();
                                        move |_| {
                                            let open = open.clone();
                                            async move {
                                                let asked = draft();
                                                // The one place the two callers
                                                // differ: same draft, same
                                                // failure handling, different
                                                // route.
                                                let answered = match open {
                                                    Filling::Creating => {
                                                        create(asked).await
                                                    }
                                                    Filling::Amending(project) => {
                                                        amend(project, asked).await
                                                    }
                                                };
                                                match answered {
                                                    Ok(fresh) => {
                                                        failure.set(None);
                                                        reading.set(Some(Ok(fresh)));
                                                        filling.set(None);
                                                    }
                                                    // Left open, deliberately:
                                                    // closing would throw away
                                                    // what was typed, and what
                                                    // is wrong is almost always
                                                    // in one field of it.
                                                    Err(reason) => failure.set(Some(reason)),
                                                }
                                            }
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
                            ProjectForm {
                                draft,
                                available: watching.available,
                                shapes: watching.shapes,
                                filling: open,
                                held,
                            }
                        }
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

/// One project, as the list shows it.
///
/// It shows and it opens the form; it never edits. What a project *is* stays
/// decided in one place, and a row that edited in place would be a second
/// place, disagreeing about which fields matter and which are required.
/// Changing one is that same form over different initial values, which is what
/// [`ProjectForm`] has always taken and what [`Filling`] now selects between.
///
/// It takes the available agents in order to *render*: a project carries the
/// identifiers a browser sends back, and this is where they become the names a
/// person reads. An identifier this build does not know is shown as it stands
/// rather than hidden — an instance naming an agent that is gone is worth
/// seeing, and `State::check` refuses one anyway.
#[component]
fn WatchedProject(project: Project, available: Vec<Agent>, onedit: EventHandler<()>) -> Element {
    let foreman = shown_as(&available, &project.foreman.agent);
    let offers = project
        .kits
        .iter()
        .map(|kit| kit.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
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
                span { class: "ml-auto flex shrink-0 items-center gap-2",
                    if project.working > 0 {
                        Badge { tone: BadgeTone::Working, "{project.working} of {project.jobs} working" }
                    } else {
                        Badge { "{project.jobs} job(s)" }
                    }
                    // Offered whatever the project is doing, unlike forgetting
                    // one: amending changes what the next job is given and
                    // cannot reach into a container that already exists.
                    //
                    // A glyph on the same terms as the `+` that adds a project
                    // — the row already says which project this is, so a word
                    // here would be the longest thing on it saying the least.
                    // Named for anyone not looking at it, and named with the
                    // project, because a screen of these reads out as a column
                    // of identical "Edit"s otherwise.
                    //
                    // U+270E and deliberately not U+270F, which is the pencil
                    // most editors offer: that one has an emoji presentation
                    // and most platforms take it, so it would arrive in colour
                    // beside the monochrome `×`, `+` and `✓` this dashboard
                    // already uses. The glyph vocabulary here is text, and one
                    // emoji in it looks like a mistake rather than a choice.
                    Button {
                        // Secondary, because this sits beside a badge on every
                        // row: the enum's own word for it is "an ordinary
                        // action sitting beside others", and a solid accent
                        // repeated down the list would out-shout the one
                        // primary action the screen has.
                        variant: ButtonVariant::Secondary,
                        class: "px-2 py-1 text-sm leading-none",
                        aria_label: "Edit {project.name}",
                        title: "Edit",
                        onclick: move |_| onedit.call(()),
                        "✎"
                    }
                }
            }
            p { class: "text-xs text-muted-foreground",
                "thinks with {foreman} · offers {offers}"
                if project.platforms.is_empty() {
                    " · no credential"
                }
                // Absence only, in the same way the credential above is. A
                // bound channel needs no announcement; one that is missing
                // changes what the project can be asked to do.
                if project.channels.is_empty() {
                    " · no channel"
                }
                // Presence, unlike the two above, and the asymmetry is the
                // point: a project with no variables is the ordinary case and
                // says nothing, while one carrying third-party credentials is
                // worth seeing at a glance. Counted rather than named, because
                // the row is already three facts long.
                if !project.variables.is_empty() {
                    " · {project.variables.len()} variable(s)"
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

/// What the form on this screen is currently for.
///
/// One value rather than two flags, so that *adding and amending at once* is a
/// sentence that cannot be said — the shape `Attending` uses in the domain for
/// the same reason. Closed is the absence of one, so the screen holds an
/// `Option` and never a third variant meaning nothing.
///
/// It carries the project being amended, because the handler needs it and
/// nothing else on the screen knows it by then. [`ProjectForm`] reads only
/// which of the two this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filling {
    /// A project that does not exist yet. Everything is required.
    Creating,
    /// One that already does, named by the identifier it is known by. A blank
    /// credential means the one it already has — see [`amend`].
    Amending(String),
}

/// One row of the variables table, as the browser sends it back.
///
/// **An empty value means the one the project already holds**, exactly as the
/// credential box does and for the same reason: no value ever reaches a
/// browser, so the box always starts empty and the other reading would wipe a
/// project's variables every time somebody corrected its name.
///
/// Removal is therefore not an empty value — it is the row being absent. The
/// form sends the complete list of names a project should end up with, so
/// leaving one out is what takes it away. That is the question
/// `docs/open-questions.md` recorded as the one genuinely new thing in this
/// screen, and this is the answer: emptiness says *keep*, absence says *drop*.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VariableDraft {
    /// What the variable is called in the container.
    pub name: String,
    /// What it is set to, or empty to keep what the project holds.
    pub value: String,
}

impl fmt::Debug for VariableDraft {
    /// Names it and never says what it is set to.
    ///
    /// `docs/conventions.md` §4. The value is a bare `String` on its way in
    /// from a form, so nothing underneath redacts and this is the whole of the
    /// defence — the same position [`ChannelDraft`] is in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableDraft")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Everything the form collects, which is everything a project is.
///
/// A struct rather than five handlers, because the form's whole purpose is to
/// be filled in twice — once to create and once to change — and a caller
/// should differ in what it *does* with the answer rather than in how it
/// receives it.
///
/// **It crosses the wire whole**, which it did not always. Both routes took
/// this apart into one parameter per field, and adding variables pushed
/// [`create`] to eight — past what the gate allows, which is the lint noticing
/// what [`ChannelDraft`]'s own reasoning already said: a value that is only
/// meaningful together should travel together. Passing the draft is also what
/// makes the two routes differ in what they *do* rather than in what they
/// take, which is what this type was for.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Draft {
    /// What to call it.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// How its foreman's agent is set.
    pub foreman: Fitted,
    /// The kits its jobs may run on. A list of rows, like the variables,
    /// because an operator names and describes each one.
    pub kits: Vec<KitDraft>,
    /// What reaches the repository.
    pub credential: String,
    /// Where this project's conversation happens, if anywhere. Optional, and
    /// the only part of a draft that may be left blank.
    pub channel: ChannelDraft,
    /// What its jobs are given that this project never reads.
    ///
    /// A list rather than a fixed set of boxes, because unlike a platform or a
    /// channel there is no closed set to draw from — an operator names these
    /// themselves, which is the whole of what makes them a separate concept.
    pub variables: Vec<VariableDraft>,
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
            .field("foreman", &self.foreman)
            .field("kits", &self.kits)
            .field("credential", &"<redacted>")
            .field("channel", &self.channel)
            .field("variables", &self.variables)
            .finish()
    }
}

impl Filling {
    /// Whether this describes a project that does not exist yet.
    ///
    /// A method rather than a comparison at each site, because the form asks
    /// this three times — for the credential's label, its placeholder, and
    /// whether a channel is offered at all — and three copies of one question
    /// is three places for it to be answered differently. Mutation testing is
    /// what said so: the comparison it replaced had no test that noticed.
    #[must_use]
    pub const fn creating(&self) -> bool {
        matches!(self, Self::Creating)
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
    /// and the form only offers agents that are. Nor whether a kit's settings
    /// are ones the agent has — the form offers only those, and the far side
    /// refuses the rest by name.
    ///
    /// The channel is the one part that is *optional* rather than required,
    /// and it is still checked: both halves or neither, which is the rule
    /// [`binding`] enforces on the far side. A pair where one box is filled is
    /// the mistake this catches, and it is worth catching on the screen
    /// because the operator is looking at the empty box.
    ///
    /// It takes the [`Filling`] because the two routes genuinely require
    /// different things, and one function saying so is better than two that
    /// could drift: creating needs a credential and may bind a channel,
    /// amending needs neither, because a blank credential there means the one
    /// already held and the channel is not offered at all.
    #[must_use]
    pub fn is_complete(&self, filling: &Filling, held: &[String]) -> bool {
        let described = !self.name.trim().is_empty()
            && !self.repository.trim().is_empty()
            && self.foreman.is_complete()
            && !self.kits.is_empty()
            && self.kits.iter().all(KitDraft::is_complete)
            && distinct(&self.kits);

        // Every row needs a name, whichever caller this is, and a row needs a
        // value unless the project already holds that name — which is exactly
        // what `resolved` asks on the far side. Not a `Filling` split any
        // more: amending a project can add a variable, and that new row has
        // nothing to keep either.
        //
        // The screen can answer this and cannot answer whether a name is
        // *deliverable*, and the difference is not arbitrary. Which names a
        // project holds crosses the wire; the rule about what an environment
        // can carry lives in the domain, which the browser's half deliberately
        // cannot name — see
        // `docs/decisions/0022-the-browser-never-sees-the-domain.md`. So this
        // one is caught before the operator presses anything, and that one is
        // a refusal naming the row.
        let named = self.variables.iter().all(|row| !row.name.trim().is_empty());
        let valued = self.variables.iter().all(|row| {
            !row.value.trim().is_empty() || held.iter().any(|had| had == row.name.trim())
        });

        match filling {
            Filling::Creating => {
                described
                    && named
                    && valued
                    && !self.credential.trim().is_empty()
                    && self.channel.address.trim().is_empty()
                        == self.channel.credential.trim().is_empty()
                    // Listening needs somewhere to listen. The reverse is fine.
                    && (self.channel.listen_credential.trim().is_empty()
                        || !self.channel.address.trim().is_empty())
            }
            Filling::Amending(_) => described && named && valued,
        }
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
fn ProjectForm(
    draft: Signal<Draft>,
    available: Vec<Agent>,
    shapes: Vec<Shape>,
    filling: Filling,
    held: Vec<String>,
) -> Element {
    let mut draft = draft;
    let creating = filling.creating();
    // What a kit added to the form starts on: the first agent as it comes.
    let starting = shapes.first().map(seeded).unwrap_or_default();

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
                FittedEditor {
                    fitted: draft().foreman,
                    shapes: shapes.clone(),
                    available: available.clone(),
                    onchange: move |fitted| draft.with_mut(|draft| draft.foreman = fitted),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "A change here lands when the foreman next picks up a message, never in the \
                 middle of one. Changing the agent starts its memory over; changing only the \
                 model or the effort keeps it."
            }
            Field { label: "Runs jobs on",
                div { class: "flex flex-col gap-2",
                    for (position, row) in draft().kits.iter().enumerate() {
                        div { key: "{position}", class: "flex flex-col gap-2 rounded-md border border-border p-2",
                            div { class: "flex items-center gap-2",
                                input {
                                    class: FIELD,
                                    placeholder: "a name, e.g. quick",
                                    value: "{row.name}",
                                    oninput: move |event| {
                                        draft
                                            .with_mut(|draft| {
                                                if let Some(row) = draft.kits.get_mut(position) {
                                                    row.name = event.value();
                                                }
                                            });
                                    },
                                }
                                // Removing the row is how a kit is taken away.
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    class: "shrink-0 px-2 py-1 text-sm leading-none",
                                    aria_label: "Remove kit {position + 1}",
                                    title: "Remove",
                                    onclick: move |_| {
                                        draft.with_mut(|draft| { draft.kits.remove(position); });
                                    },
                                    "×"
                                }
                            }
                            input {
                                class: FIELD,
                                placeholder: "what this project wants it for, e.g. small fixes and questions",
                                value: "{row.description}",
                                oninput: move |event| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.kits.get_mut(position) {
                                                row.description = event.value();
                                            }
                                        });
                                },
                            }
                            FittedEditor {
                                fitted: row.fitted.clone(),
                                shapes: shapes.clone(),
                                available: available.clone(),
                                onchange: move |fitted| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.kits.get_mut(position) {
                                                row.fitted = fitted;
                                            }
                                        });
                                },
                            }
                        }
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: "self-start px-2.5 py-1 text-sm leading-none",
                        aria_label: "Add a kit",
                        title: "Add a kit",
                        onclick: {
                            move |_| {
                                let fitted = starting.clone();
                                draft.with_mut(|draft| {
                                    draft.kits.push(KitDraft {
                                        fitted,
                                        ..KitDraft::default()
                                    });
                                });
                            }
                        },
                        "+"
                    }
                }
            }
            p { class: "text-xs text-faint-foreground",
                "A kit is an agent set a particular way. The foreman picks one per job by what \
                 you say it is for, so say what each is for — a cheap one for small fixes and \
                 questions, a strong one for work that touches many files."
            }
            Field { label: if creating { "GitHub credential" } else { "GitHub credential (optional)" },
                input {
                    r#type: "password",
                    class: FIELD,
                    // The one place this form tells the operator what an empty
                    // box means, because it is the one place where empty is not
                    // the same as absent. There is nothing to prefill it with:
                    // no credential ever reaches the browser.
                    placeholder: if creating {
                        "a token scoped to this repository"
                    } else {
                        "leave empty to keep the current one"
                    },
                    value: "{draft().credential}",
                    oninput: move |event| draft.with_mut(|draft| draft.credential = event.value()),
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Scoped to this repository, with contents and pull requests write. A token that \
                 reaches more than this project is a token every job on it could misuse."
            }
            // Only when creating. A binding's address never reaches the
            // browser, so there is nothing to show here for a project that has
            // one — and an empty box that meant *unbind* would disconnect a
            // project every time somebody corrected its name.
            if creating {
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
            Field { label: "Variables (optional)",
                div { class: "flex flex-col gap-2",
                    for (position, row) in draft().variables.iter().enumerate() {
                        div { key: "{position}", class: "flex items-center gap-2",
                            input {
                                class: "{FIELD} font-mono",
                                placeholder: "STRIPE_API_KEY",
                                value: "{row.name}",
                                oninput: move |event| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.variables.get_mut(position) {
                                                row.name = event.value();
                                            }
                                        });
                                },
                            }
                            input {
                                r#type: "password",
                                class: FIELD,
                                // Per row rather than per form, because *this
                                // row* is what decides it: a box says "keep"
                                // only where there is something to keep, which
                                // is a name the project already holds. A row
                                // just added holds nothing, and neither does
                                // one whose name has been typed over — and
                                // both change the moment the name does, which
                                // is the behaviour an operator expects from a
                                // hint about the box beside it.
                                placeholder: if held.iter().any(|had| had == row.name.trim()) {
                                    "leave empty to keep"
                                } else {
                                    "its value"
                                },
                                value: "{row.value}",
                                oninput: move |event| {
                                    draft
                                        .with_mut(|draft| {
                                            if let Some(row) = draft.variables.get_mut(position) {
                                                row.value = event.value();
                                            }
                                        });
                                },
                            }
                            // Removing the row is how a variable is taken
                            // away: an empty value already means keep, so
                            // absence is the only thing left to mean drop.
                            Button {
                                variant: ButtonVariant::Secondary,
                                class: "shrink-0 px-2 py-1 text-sm leading-none",
                                aria_label: "Remove variable {position + 1}",
                                title: "Remove",
                                onclick: move |_| {
                                    draft.with_mut(|draft| { draft.variables.remove(position); });
                                },
                                "×"
                            }
                        }
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        class: "self-start px-2.5 py-1 text-sm leading-none",
                        aria_label: "Add a variable",
                        title: "Add a variable",
                        onclick: move |_| {
                            draft.with_mut(|draft| draft.variables.push(VariableDraft::default()));
                        },
                        "+"
                    }
                }
            }
            p { class: "text-xs text-faint-foreground",
                "Set in every container this project's jobs run in, and named to the agent so it \
                 knows they are there. stageman never reads one, so what they mean is the \
                 repository's business. Removing a row takes the variable away; leaving its value \
                 empty keeps the one already stored."
            }
        }
    }
}

/// The three selects that set an agent: which one, which model, and how hard
/// it thinks where the model allows a choice.
///
/// **Controlled**, like the form around it: it emits the whole [`Fitted`] on
/// every change rather than writing anywhere, so that the parent decides which
/// row it lands in. Moving to another agent starts from that agent's defaults;
/// moving to another model keeps the effort where the new one takes it and
/// clears it where it does not — see [`with_agent`] and [`with_model`], which
/// are pure so that both rules can be tested without a browser.
///
/// The effort select appears only where the model takes one. A select for a
/// setting the model does not have would offer a choice the far side refuses.
#[component]
fn FittedEditor(
    fitted: Fitted,
    shapes: Vec<Shape>,
    available: Vec<Agent>,
    onchange: EventHandler<Fitted>,
) -> Element {
    let shape = shape_for(&shapes, &fitted.agent).cloned();
    let with_effort = shape
        .as_ref()
        .is_some_and(|shape| takes_effort(shape, &fitted.model));

    rsx! {
        div { class: "flex flex-wrap gap-2",
            select {
                class: "{FIELD} basis-40 grow",
                value: "{fitted.agent}",
                onchange: move |event: Event<FormData>| {
                    onchange.call(with_agent(&shapes, &event.value()));
                },
                for agent in available.iter() {
                    option { key: "{agent.id}", value: "{agent.id}", "{agent.name}" }
                }
            }
            if let Some(shape) = shape {
                select {
                    class: "{FIELD} basis-40 grow",
                    value: "{fitted.model}",
                    onchange: {
                        let fitted = fitted.clone();
                        move |event: Event<FormData>| {
                            onchange.call(with_model(&fitted, &shape, &event.value()));
                        }
                    },
                    for model in shape.models.iter() {
                        option { key: "{model.id}", value: "{model.id}", "{model.name}" }
                    }
                }
                if with_effort {
                    select {
                        class: "{FIELD} basis-40 grow",
                        value: "{fitted.effort}",
                        onchange: {
                            let fitted = fitted.clone();
                            move |event: Event<FormData>| {
                                onchange.call(Fitted {
                                    effort: event.value(),
                                    ..fitted.clone()
                                });
                            }
                        },
                        for effort in shape.efforts.iter() {
                            option { key: "{effort.id}", value: "{effort.id}", "{effort.name}" }
                        }
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
    use super::{
        ChannelDraft, DashboardError, Fitted, KitDraft, VariableDraft, amended, binding, busy,
        kits_of, resolved,
    };
    use stageman_core::{ProjectId, State};

    /// The two boxes that bind a channel, plus the optional third.
    fn drafted(address: &str, credential: &str, listening: &str) -> ChannelDraft {
        ChannelDraft {
            address: address.to_owned(),
            credential: credential.to_owned(),
            listen_credential: listening.to_owned(),
        }
    }
    use stageman_core::{
        Agent, Channel, ClaudeEffort, ClaudeModel, Job, JobId, Kit, KitConfig, KitName, Progress,
        Project, Secret, Timestamp,
    };
    use std::collections::BTreeMap;

    /// The one kit a fixture offers: its agent's defaults, under the agent's
    /// name.
    #[expect(
        clippy::expect_used,
        reason = "a fixture kit that cannot be named is a broken test, and should say so"
    )]
    fn one_kit() -> BTreeMap<KitName, KitConfig> {
        BTreeMap::from([(
            KitName::new("Claude").expect("a name"),
            KitConfig::defaults(Agent::Claude),
        )])
    }

    /// Amending replaces both the foreman's kit and the kits whole, because
    /// the form shows and resubmits the whole of each.
    ///
    /// The fixture's own kit is asserted first, so that a fixture offering
    /// nothing cannot make the rest pass vacuously — which mutation testing
    /// showed it could.
    #[test]
    fn amending_replaces_the_foremans_kit_and_the_kits_whole() {
        let mut watched = holding(&[]);
        assert_eq!(watched.kits.len(), 1, "the fixture offers one kit");
        assert!(
            watched
                .kits
                .contains_key(&KitName::new("Claude").expect("a name")),
            "named after its agent"
        );

        let deep = KitConfig {
            description: "refactors touching many files".to_owned(),
            kit: Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            },
        };
        amended(
            &mut watched,
            "aviary".to_owned(),
            "https://example.invalid/aviary".to_owned(),
            Kit::Claude {
                model: ClaudeModel::Haiku,
            },
            BTreeMap::from([(KitName::new("deep").expect("a name"), deep.clone())]),
            "",
        );

        assert_eq!(
            watched.foreman_kit,
            Kit::Claude {
                model: ClaudeModel::Haiku
            }
        );
        assert_eq!(
            watched.kits,
            BTreeMap::from([(KitName::new("deep").expect("a name"), deep)]),
            "the old kit is gone, because the form did not resubmit it"
        );
    }

    /// A kit row as the form would send it.
    fn kit_row(name: &str, description: &str, model: &str, effort: &str) -> KitDraft {
        KitDraft {
            name: name.to_owned(),
            description: description.to_owned(),
            fitted: Fitted {
                agent: "claude".to_owned(),
                model: model.to_owned(),
                effort: effort.to_owned(),
            },
        }
    }

    /// What a form describes becomes the project's kits, and what the domain
    /// would silently mend is refused instead.
    ///
    /// Two rows under one name is the case that matters: the domain keys on
    /// the name and would keep one without a word, which is the failure this
    /// route exists to stop.
    #[test]
    fn a_forms_kits_are_kept_whole_and_refused_where_the_domain_would_mend() {
        let kits = kits_of(&[
            kit_row("quick", "small fixes", "haiku", ""),
            kit_row(" deep ", "refactors touching many files", "opus", "xhigh"),
        ])
        .expect("two well-formed kits");
        assert_eq!(kits.len(), 2);
        let deep = kits
            .get(&KitName::new("deep").expect("a name"))
            .expect("named as typed, less the space around it");
        assert_eq!(deep.description, "refactors touching many files");
        assert_eq!(
            deep.kit,
            Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            }
        );

        assert_eq!(kits_of(&[]), Err(DashboardError::KitsMissing));
        assert_eq!(
            kits_of(&[kit_row("  ", "described", "default", "default")]),
            Err(DashboardError::Incomplete {
                field: "kit name".to_owned()
            }),
            "a blank name is not a name"
        );
        assert_eq!(
            kits_of(&[kit_row("quick", "  ", "default", "default")]),
            Err(DashboardError::Incomplete {
                field: "kit description".to_owned()
            }),
            "a kit is chosen by its description, so it needs one"
        );
        assert_eq!(
            kits_of(&[
                kit_row("quick", "one", "haiku", ""),
                kit_row("quick ", "another", "sonnet", "low"),
            ]),
            Err(DashboardError::KitNameTaken {
                name: "quick".to_owned()
            }),
            "two rows under one name, however spaced, are refused rather than merged"
        );
        assert_eq!(
            kits_of(&[kit_row("quick", "small fixes", "haiku", "high")]),
            Err(DashboardError::EffortNotOnModel {
                model: "Haiku".to_owned()
            }),
            "the far side refuses what the form would never send"
        );
    }

    /// One job, in whatever state the caller needs it.
    fn job(progress: Progress) -> Job {
        let mut job = Job::new(
            Kit::defaults(Agent::Claude),
            "because a test said so".to_owned(),
            "do the thing".to_owned(),
            Timestamp::UNIX_EPOCH,
        );
        job.progress = progress;
        job
    }

    /// A project holding exactly these jobs.
    #[expect(
        clippy::expect_used,
        reason = "a fixture kit that cannot be named is a broken test, and should say so"
    )]
    fn holding(jobs: &[Progress]) -> Project {
        Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_kit: Kit::defaults(Agent::Claude),
            kits: BTreeMap::from([(
                KitName::new("Claude").expect("a name"),
                KitConfig::defaults(Agent::Claude),
            )]),
            credentials: BTreeMap::new(),
            channels: BTreeMap::new(),
            variables: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
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

    /// One row of the variables table.
    fn row(name: &str, value: &str) -> VariableDraft {
        VariableDraft {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    /// What a project already carries.
    ///
    /// The suppression is needed here and not in the sibling `tests` module
    /// below, which writes the same call happily. `clippy.toml` sets
    /// `allow-expect-in-tests`, and that allowance does not reach this module
    /// because it is gated on `cfg(all(test, feature = "server"))` rather than
    /// on `test` alone. Written out rather than worked around: the alternative
    /// is a fixture that swallows a bad name and yields an empty map, which
    /// would make every test here pass vacuously.
    #[expect(
        clippy::expect_used,
        reason = "a fixture name that is not deliverable is a broken test, and should say so"
    )]
    fn holding_variables(pairs: &[(&str, &str)]) -> BTreeMap<stageman_core::VariableName, Secret> {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    stageman_core::VariableName::new(*name).expect("a deliverable name"),
                    Secret::new((*value).to_owned()),
                )
            })
            .collect()
    }

    /// What came out, as plain pairs, so an assertion reads as one line.
    fn settled(map: &BTreeMap<stageman_core::VariableName, Secret>) -> Vec<(String, String)> {
        map.iter()
            .map(|(name, value)| (name.to_string(), value.expose().to_owned()))
            .collect()
    }

    /// The rule an empty box follows, and the reason the whole feature is safe
    /// to edit: no value ever reaches a browser, so blank has to mean keep.
    #[test]
    fn a_blank_value_keeps_what_the_project_holds() {
        let held = holding_variables(&[("STRIPE_API_KEY", "sk-test-not-a-real-key")]);

        let kept = resolved(&held, &[row("STRIPE_API_KEY", "")]).expect("a name it already has");

        assert_eq!(
            settled(&kept),
            vec![(
                "STRIPE_API_KEY".to_owned(),
                "sk-test-not-a-real-key".to_owned()
            )]
        );
    }

    /// And a value that was typed replaces it, which is how one gets rotated.
    #[test]
    fn a_value_that_was_typed_replaces_what_is_held() {
        let held = holding_variables(&[("STRIPE_API_KEY", "sk-test-the-old-one")]);

        let now = resolved(&held, &[row("STRIPE_API_KEY", "sk-test-the-new-one")])
            .expect("a name it already has");

        assert_eq!(
            settled(&now),
            vec![(
                "STRIPE_API_KEY".to_owned(),
                "sk-test-the-new-one".to_owned()
            )]
        );
    }

    /// Removal is a row that is not there.
    ///
    /// The question `docs/open-questions.md` recorded as the new one in this
    /// screen, answered: emptiness already means keep, so absence is the only
    /// thing left to mean drop.
    #[test]
    fn a_variable_left_out_is_taken_away() {
        let held = holding_variables(&[
            ("STRIPE_API_KEY", "sk-test-not-a-real-key"),
            ("DATABASE_URL", "postgres://nowhere"),
        ]);

        let now = resolved(&held, &[row("DATABASE_URL", "")]).expect("a name it already has");

        assert_eq!(
            settled(&now),
            vec![("DATABASE_URL".to_owned(), "postgres://nowhere".to_owned())]
        );
    }

    /// A new name with no value has nothing to keep, and storing an empty
    /// credential would read as configured and authenticate as nothing.
    #[test]
    fn a_new_variable_with_no_value_is_refused() {
        let refused = resolved(&BTreeMap::new(), &[row("STRIPE_API_KEY", "")])
            .expect_err("nothing is held under that name");

        assert_eq!(refused, DashboardError::VariableValueMissing);
    }

    /// The refusal that keeps a credential out of the process table.
    ///
    /// A runtime reads a name containing an equals sign as an inline
    /// assignment, so the value would travel on the command line. The domain
    /// refuses the name; this is that refusal reaching a screen, and it names
    /// the row rather than the name — the mistake it most often catches is a
    /// credential pasted into the wrong box.
    #[test]
    fn a_name_a_container_could_not_be_given_is_refused_by_position() {
        let refused = resolved(&BTreeMap::new(), &[row("NOT A NAME=oops", "anything")])
            .expect_err("that is not a name");

        assert!(
            matches!(
                refused,
                DashboardError::VariableNameRefused { position: 1, .. }
            ),
            "{refused:?}"
        );
        assert!(
            !format!("{refused}").contains("oops"),
            "a refusal must not repeat what was typed: {refused}"
        );
    }

    /// A name stageman delivers itself is refused before it can change which
    /// account an agent bills — `docs/decisions/0008-one-credential-per-agent.md`.
    ///
    /// Over the whole reserved set, so an agent added later is covered here
    /// the moment its name joins that list.
    #[test]
    fn a_name_stageman_delivers_itself_is_refused() {
        for claimed in stageman_agent::RESERVED {
            let refused = resolved(&BTreeMap::new(), &[row(claimed, "somebody-elses-account")])
                .expect_err("that name is stageman's");

            assert_eq!(
                refused,
                DashboardError::VariableReserved {
                    name: (*claimed).to_owned()
                }
            );
        }
    }

    /// Two rows with one name have no resolution that keeps both, so neither
    /// is chosen.
    #[test]
    fn the_same_name_twice_is_refused() {
        let refused = resolved(
            &BTreeMap::new(),
            &[row("STRIPE_API_KEY", "one"), row("STRIPE_API_KEY", "two")],
        )
        .expect_err("one name, two rows");

        assert_eq!(refused, DashboardError::VariableRepeated { position: 2 });
    }

    /// Whitespace is not a name and not a value.
    ///
    /// The route trims before judging, so a form that accepted spaces would
    /// offer a control the instance then refuses.
    #[test]
    fn whitespace_is_neither_a_name_nor_a_value() {
        assert!(resolved(&BTreeMap::new(), &[row("  ", "anything")]).is_err());

        let held = holding_variables(&[("STRIPE_API_KEY", "sk-test-not-a-real-key")]);
        let kept = resolved(&held, &[row(" STRIPE_API_KEY ", "   ")])
            .expect("a name it already has, spaces and all");

        assert_eq!(
            settled(&kept),
            vec![(
                "STRIPE_API_KEY".to_owned(),
                "sk-test-not-a-real-key".to_owned()
            )],
            "a value of spaces is an empty box, so it keeps"
        );
    }

    /// A project holding one GitHub credential, to amend against.
    fn credentialled(token: &str) -> Project {
        let mut project = holding(&[]);
        project.credentials.insert(
            stageman_core::Platform::GitHub,
            Secret::new(token.to_owned()),
        );
        project
    }

    /// What a project's GitHub credential is now, if it has one.
    fn token_of(project: &Project) -> Option<String> {
        project
            .credentials
            .get(&stageman_core::Platform::GitHub)
            .map(|secret| secret.expose().to_owned())
    }

    /// The rule that could quietly disarm a project, asserted directly.
    ///
    /// No credential ever reaches the browser, so the box an operator sees is
    /// always empty — which means *keep it* rather than *clear it*. Were this
    /// the other way round, correcting a project's name would silently remove
    /// the token every one of its jobs needs, and nothing on any screen would
    /// say so until the next job failed to clone.
    #[test]
    fn a_blank_credential_keeps_the_one_already_held() {
        let mut project = credentialled("ghp-not-a-real-token");

        amended(
            &mut project,
            "renamed".to_owned(),
            "https://example.invalid/renamed".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            "",
        );

        assert_eq!(token_of(&project).as_deref(), Some("ghp-not-a-real-token"));
        // And the kits given are now the project's. Asserted here on the
        // fixture's content, because a fixture offering nothing would let
        // every use of it above pass without saying anything.
        assert_eq!(project.kits.len(), 1);
        assert!(
            project
                .kits
                .contains_key(&KitName::new("Claude").expect("a name"))
        );
    }

    /// And a credential that was typed replaces it, which is the whole point
    /// of offering the box — an expiring token was the gap this closes.
    #[test]
    fn a_credential_that_was_typed_replaces_the_one_held() {
        let mut project = credentialled("ghp-the-old-one");

        amended(
            &mut project,
            "aviary".to_owned(),
            "https://example.invalid/aviary".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            "ghp-the-new-one",
        );

        assert_eq!(token_of(&project).as_deref(), Some("ghp-the-new-one"));
    }

    /// The same rule where there is nothing to keep. Worth its own test
    /// because the obvious implementation of *keep* — insert whatever came in
    /// — would put an empty credential here, which reads as configured and
    /// authenticates as nothing.
    #[test]
    fn a_blank_credential_leaves_a_project_that_had_none_with_none() {
        let mut project = holding(&[]);

        amended(
            &mut project,
            "aviary".to_owned(),
            "https://example.invalid/aviary".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            "",
        );

        assert_eq!(token_of(&project), None);
    }

    /// Everything an amendment is actually for, and the things it must not
    /// touch. A project's jobs are its history, and its channel binding cannot
    /// be shown on the form that produced this — so both have to survive.
    #[test]
    fn amending_replaces_what_a_project_is_and_leaves_what_it_has_done() {
        let mut project = credentialled("ghp-not-a-real-token");
        project.channels.insert(
            Channel::Slack,
            stageman_core::ChannelConfig {
                address: "C0123456789".to_owned(),
                credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                listen_credential: None,
            },
        );
        project
            .jobs
            .insert(JobId::from_uuid(uuid::Uuid::new_v4()), job(Progress::Idle));

        amended(
            &mut project,
            "renamed".to_owned(),
            "https://example.invalid/renamed".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            "",
        );

        assert_eq!(project.name, "renamed");
        assert_eq!(project.repository, "https://example.invalid/renamed");
        assert_eq!(project.jobs.len(), 1, "its history is not an amendment");
        assert!(
            project.channels.contains_key(&Channel::Slack),
            "a binding the form could not show must survive one",
        );
    }

    /// Nothing running is what lets a project be forgotten.
    #[test]
    fn a_project_with_nothing_running_is_not_busy() {
        assert_eq!(busy(&holding(&[])), None);
        assert_eq!(
            busy(&holding(&[
                Progress::Idle,
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
    /// belong to whichever foreman the search reached first — an ordering
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
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: BTreeMap::from([(
                    KitName::new("Claude").expect("a name"),
                    KitConfig::defaults(Agent::Claude),
                )]),
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
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
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
        assert_eq!(busy(&holding(&[Progress::Working])), Some(1));
        assert_eq!(
            busy(&holding(&[
                Progress::Working,
                Progress::Idle,
                Progress::Working,
            ])),
            Some(2)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::agents_view::Agent;
    use super::{
        ChannelDraft, Choice, Draft, Filling, Fitted, KitDraft, ModelChoice, Project, Shape,
        VariableDraft, distinct, seeded, shape_for, shown_as, takes_effort, with_agent, with_model,
    };

    /// The agent's defaults, as a browser holds them.
    fn as_it_comes() -> Fitted {
        Fitted {
            agent: "claude".to_owned(),
            model: "default".to_owned(),
            effort: "default".to_owned(),
        }
    }

    /// One kit on the agent's defaults, named and described after it.
    fn default_kit() -> KitDraft {
        KitDraft {
            name: "Claude".to_owned(),
            description: "General-purpose.".to_owned(),
            fitted: as_it_comes(),
        }
    }

    /// The shape the server would send for Claude, as the form sees it.
    fn claude() -> Shape {
        let model = |id: &str, has_effort: bool| ModelChoice {
            id: id.to_owned(),
            name: id.to_owned(),
            has_effort,
        };
        let effort = |id: &str| Choice {
            id: id.to_owned(),
            name: id.to_owned(),
        };
        Shape {
            agent: "claude".to_owned(),
            models: vec![
                model("default", true),
                model("sonnet", true),
                model("opus", true),
                model("haiku", false),
            ],
            efforts: vec![effort("default"), effort("low"), effort("high")],
        }
    }

    /// One project, with however many jobs running.
    fn watched(working: usize, jobs: usize) -> Project {
        Project {
            id: "an-identifier".to_owned(),
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![default_kit()],
            platforms: Vec::new(),
            channels: Vec::new(),
            variables: Vec::new(),
            working,
            jobs,
        }
    }

    /// A draft the form would accept, with every box filled including the two
    /// that need not be.
    fn filled() -> Draft {
        Draft {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![default_kit()],
            credential: "ghp-not-a-real-token".to_owned(),
            channel: ChannelDraft {
                address: "C0123456789".to_owned(),
                credential: "xoxb-not-a-real-token".to_owned(),
                listen_credential: "xapp-not-a-real-token".to_owned(),
            },
            variables: vec![VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: "sk-test-not-a-real-key".to_owned(),
            }],
        }
    }

    /// Applies one change to an otherwise complete draft.
    fn without(change: fn(&mut Draft)) -> Draft {
        let mut draft = filled();
        change(&mut draft);
        draft
    }

    /// A project carrying no variables yet, which is what a draft is judged
    /// against unless a test says otherwise.
    const NOTHING_HELD: &[String] = &[];

    /// The project an amendment names. Its value never matters to
    /// [`Draft::is_complete`]; which variant it is, does.
    fn amending() -> Filling {
        Filling::Amending("a-project".to_owned())
    }

    #[test]
    fn a_draft_with_every_answer_is_complete() {
        assert!(filled().is_complete(&Filling::Creating, NOTHING_HELD));
    }

    /// Every field is required, and the test says so one at a time.
    ///
    /// Written out rather than looped because the point is which field, and a
    /// loop over closures would say it less clearly than five lines do.
    #[test]
    fn a_draft_missing_any_answer_is_not() {
        assert!(!without(|draft| draft.name.clear()).is_complete(&Filling::Creating, NOTHING_HELD));
        assert!(
            !without(|draft| draft.repository.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
        assert!(
            !without(|draft| draft.foreman.agent.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
        assert!(!without(|draft| draft.kits.clear()).is_complete(&Filling::Creating, NOTHING_HELD));
        assert!(
            !without(|draft| draft.credential.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
    }

    /// The one field the two callers disagree about.
    ///
    /// Creating needs a credential because a project with none can reach
    /// nothing; amending does not, because there is no way to show the one
    /// already held and an empty box therefore means *keep it*. A single rule
    /// for both would have to pick one of those, and either choice is wrong
    /// half the time.
    #[test]
    fn amending_does_not_require_a_credential_and_creating_does() {
        let blank = without(|draft| draft.credential.clear());

        assert!(blank.is_complete(&amending(), NOTHING_HELD));
        assert!(!blank.is_complete(&Filling::Creating, NOTHING_HELD));
    }

    /// Everything else is required of both, which is what stops the exception
    /// above from being read as *amending checks nothing*.
    #[test]
    fn amending_still_requires_everything_a_project_is() {
        assert!(!without(|draft| draft.name.clear()).is_complete(&amending(), NOTHING_HELD));
        assert!(!without(|draft| draft.repository.clear()).is_complete(&amending(), NOTHING_HELD));
        assert!(
            !without(|draft| draft.foreman.agent.clear()).is_complete(&amending(), NOTHING_HELD)
        );
        assert!(!without(|draft| draft.kits.clear()).is_complete(&amending(), NOTHING_HELD));
    }

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

    /// The one question the form asks three times.
    ///
    /// Mutation testing found the comparison it replaced unguarded, which is
    /// how a form could have started offering a channel while amending and
    /// calling a credential optional while creating.
    #[test]
    fn only_creating_is_creating() {
        assert!(Filling::Creating.creating());
        assert!(!amending().creating());
    }

    /// `docs/conventions.md` §4, for the row that holds a value.
    ///
    /// Its own test rather than a line in the draft's, because the draft's
    /// formatter delegates here — so a derived `Debug` on this type would
    /// leak through a test that reads as though it covered it. Mutation
    /// testing found exactly that: emptying this formatter changed nothing
    /// any assertion noticed.
    #[test]
    fn a_variable_row_does_not_leak_its_value_when_formatted() {
        let shown = format!(
            "{:?}",
            VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: "sk-test-not-a-real-key".to_owned(),
            }
        );

        assert!(!shown.contains("sk-test-not-a-real-key"), "{shown}");
        assert!(
            shown.contains("STRIPE_API_KEY"),
            "it should still say which variable it is: {shown}"
        );
    }

    /// A row for a variable the project does not hold needs a value, whichever
    /// caller this is — including an amendment, which is where it was wrong.
    ///
    /// The screen used to require values only when creating, so adding a
    /// variable to an existing project offered a control that the route then
    /// refused. Being told off for something the screen could see is exactly
    /// what [`Draft::is_complete`] exists to prevent.
    #[test]
    fn a_variable_the_project_does_not_hold_needs_a_value() {
        let added = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "DATABASE_URL".to_owned(),
                value: String::new(),
            }];
        });

        assert!(!added.is_complete(&amending(), NOTHING_HELD));
        assert!(!added.is_complete(&Filling::Creating, NOTHING_HELD));
    }

    /// And one the project does hold does not, because an empty box there
    /// means keep — which is the whole reason the held names reach the screen.
    #[test]
    fn a_variable_the_project_already_holds_may_be_left_empty() {
        let kept = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: String::new(),
            }];
        });

        assert!(kept.is_complete(&amending(), &["STRIPE_API_KEY".to_owned()]));
    }

    /// Typing over the name of a held variable makes it a new one again.
    ///
    /// The case that makes this per row rather than per form: the row was
    /// seeded from the project, so it began as something with a value to keep,
    /// and renaming it leaves nothing behind that name at all.
    #[test]
    fn renaming_a_held_variable_makes_it_need_a_value_again() {
        let renamed = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "STRIPE_API_KEY_V2".to_owned(),
                value: String::new(),
            }];
        });

        assert!(!renamed.is_complete(&amending(), &["STRIPE_API_KEY".to_owned()]));
    }

    /// A half-bound channel cannot block an amendment, because amending never
    /// offers one. The form hides those boxes, so a draft carrying whatever
    /// was left in them must not make the control unavailable.
    #[test]
    fn amending_ignores_the_channel_boxes_entirely() {
        assert!(
            without(|draft| draft.channel.address.clear()).is_complete(&amending(), NOTHING_HELD)
        );
        assert!(
            without(|draft| draft.channel.credential.clear())
                .is_complete(&amending(), NOTHING_HELD)
        );
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
            .is_complete(&Filling::Creating, NOTHING_HELD)
        );
    }

    /// Half a binding is the mistake worth catching on the screen.
    ///
    /// Neither half works alone, and the operator is looking at the box they
    /// left empty — so the control that submits goes unavailable rather than
    /// the instance refusing after the fact.
    #[test]
    fn a_draft_binding_half_a_channel_is_not_complete() {
        assert!(
            !without(|draft| draft.channel.address.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
        assert!(
            !without(|draft| draft.channel.credential.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
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
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));

        let mut draft = filled();
        draft.credential = "\t ".to_owned();
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));

        // And a channel half-filled with spaces is half-filled, which is what
        // the route decides after trimming. A screen that judged before
        // trimming would offer a control the instance then refuses.
        let mut draft = filled();
        draft.channel.address = "  ".to_owned();
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));
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
        // A variable's value is the third credential a draft holds, and it
        // reaches this formatter through the row's own — so a derive on either
        // one leaks here.
        assert!(!shown.contains("sk-test-not-a-real-key"), "{shown}");
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
            .is_complete(&Filling::Creating, NOTHING_HELD),
            "a token to listen with and no channel is not a complete draft"
        );

        // And dropping only the listening token is fine, since speaking
        // without listening is an ordinary project.
        assert!(
            without(|draft| draft.channel.listen_credential.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
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

    /// A kit row needs a name and a description, and two rows cannot share a
    /// name — the screen refuses what the far side would refuse, before the
    /// operator presses anything.
    #[test]
    fn a_kit_row_needs_a_name_and_a_description_and_names_are_distinct() {
        assert!(filled().is_complete(&Filling::Creating, NOTHING_HELD));

        let mut unnamed = filled();
        unnamed.kits.push(KitDraft {
            name: "  ".to_owned(),
            description: "something".to_owned(),
            fitted: as_it_comes(),
        });
        assert!(!unnamed.is_complete(&Filling::Creating, NOTHING_HELD));

        let mut undescribed = filled();
        undescribed.kits.push(KitDraft {
            name: "deep".to_owned(),
            description: " ".to_owned(),
            fitted: as_it_comes(),
        });
        assert!(!undescribed.is_complete(&Filling::Creating, NOTHING_HELD));

        let mut twice = filled();
        twice.kits.push(KitDraft {
            name: " Claude ".to_owned(),
            description: "again".to_owned(),
            fitted: as_it_comes(),
        });
        assert!(
            !twice.is_complete(&Filling::Creating, NOTHING_HELD),
            "the same name with space around it is the same name"
        );
        assert!(!distinct(&twice.kits));
        assert!(distinct(&filled().kits));
    }

    /// Moving between models keeps, clears or seeds the effort as the new
    /// model demands, so the form never holds a pair the far side refuses.
    #[test]
    fn changing_the_model_keeps_the_effort_only_where_the_new_model_takes_one() {
        let shape = claude();
        let on_opus = with_model(&as_it_comes(), &shape, "opus");
        assert_eq!(on_opus.model, "opus");
        assert_eq!(on_opus.effort, "default", "kept, since opus takes one");

        let on_haiku = with_model(&on_opus, &shape, "haiku");
        assert_eq!(on_haiku.model, "haiku");
        assert_eq!(on_haiku.effort, "", "cleared, since haiku takes none");

        let back = with_model(&on_haiku, &shape, "sonnet");
        assert_eq!(
            back.effort, "default",
            "seeded with the first, since there was none"
        );

        let chosen = Fitted {
            effort: "high".to_owned(),
            ..as_it_comes()
        };
        assert_eq!(
            with_model(&chosen, &shape, "sonnet").effort,
            "high",
            "a chosen effort survives a change of model"
        );
    }

    /// The two comparisons the form turns on, asserted in both directions.
    ///
    /// Mutation testing inverted each of these inside the component and no
    /// test noticed, which is why they are functions now.
    #[test]
    fn a_shape_is_found_by_its_agent_and_says_which_models_take_an_effort() {
        let shapes = vec![claude()];
        assert_eq!(shape_for(&shapes, "claude"), Some(&claude()));
        assert_eq!(shape_for(&shapes, "gpt"), None);
        assert_eq!(shape_for(&[], "claude"), None);

        assert!(takes_effort(&claude(), "opus"));
        assert!(takes_effort(&claude(), "default"));
        assert!(!takes_effort(&claude(), "haiku"), "the one model with none");
        assert!(
            !takes_effort(&claude(), "gpt-5"),
            "a model the shape does not list takes nothing"
        );
    }

    /// Moving to another agent starts from that agent's defaults, and an
    /// agent no shape describes is carried as itself for the far side to
    /// refuse by name.
    #[test]
    fn changing_the_agent_starts_from_its_defaults() {
        let shapes = vec![claude()];
        assert_eq!(with_agent(&shapes, "claude"), as_it_comes());
        assert_eq!(seeded(&claude()), as_it_comes());
        assert_eq!(
            with_agent(&shapes, "gpt"),
            Fitted {
                agent: "gpt".to_owned(),
                ..Fitted::default()
            }
        );

        let mut effortless = claude();
        effortless.models.rotate_left(3);
        assert_eq!(
            seeded(&effortless).effort,
            "",
            "an agent whose first model takes no effort is seeded with none"
        );
    }
}
