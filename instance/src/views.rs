//! What a page is allowed to know, built from the domain.
//!
//! The conversion from domain to wire, kept in one place: every identifier
//! a browser sends back and every name a person reads is decided here, so
//! that adding an agent to the domain stops this compiling until somebody
//! decides what the browser calls it. A wire name is a contract, and
//! deciding it deliberately is the point.

use std::collections::BTreeMap;

use stageman_channel::Identity;
use stageman_core::{
    Access, Agent, Attending, Channel, ClaudeEffort, ClaudeModel, Inconsistent, Installation, Job,
    JobId, Kit, Outcome, Platform, Progress, Project, ProjectId, RepositoryAddress, Room, State,
    Timestamp, Waiting,
};
use stageman_wire::{AccessView, Choice, Fitted, KitDraft, ModelChoice, Refusal, Shape, Standing};

use crate::tunnel::{Domain, address};

/// The agent named by a wire identifier.
///
/// # Errors
///
/// Fails if nothing is called that.
pub fn named(identifier: &str) -> Result<Agent, Refusal> {
    match identifier {
        "claude" => Ok(Agent::Claude),
        _ => Err(Refusal::UnknownAgent {
            name: identifier.to_owned(),
        }),
    }
}

/// What the browser calls an agent, and what to show for it.
pub const fn wire_name(agent: Agent) -> (&'static str, &'static str) {
    match agent {
        Agent::Claude => ("claude", "Claude"),
    }
}

/// What the browser calls one of Claude's models, and what to show for it.
///
/// The browser's vocabulary rather than the adapter's, which spells the same
/// models for the wire in the agent crate. The two happen to agree today and
/// are separate contracts.
const fn wire_model(model: ClaudeModel) -> (&'static str, &'static str) {
    match model {
        ClaudeModel::Default { .. } => ("default", "Default"),
        ClaudeModel::Sonnet { .. } => ("sonnet", "Sonnet"),
        ClaudeModel::Opus { .. } => ("opus", "Opus"),
        ClaudeModel::Haiku => ("haiku", "Haiku"),
    }
}

/// What the browser calls an effort level, and what to show for it.
const fn wire_effort(effort: ClaudeEffort) -> (&'static str, &'static str) {
    match effort {
        ClaudeEffort::Default => ("default", "Default"),
        ClaudeEffort::Low => ("low", "Low"),
        ClaudeEffort::Medium => ("medium", "Medium"),
        ClaudeEffort::High => ("high", "High"),
        ClaudeEffort::XHigh => ("xhigh", "Extra high"),
        ClaudeEffort::Max => ("max", "Max"),
    }
}

/// One of each of Claude's models, for enumerating what a browser may choose.
/// The effort on the ones that carry one is a placeholder.
const CLAUDE_MODELS: [ClaudeModel; 4] = [
    ClaudeModel::Default {
        effort: ClaudeEffort::Default,
    },
    ClaudeModel::Sonnet {
        effort: ClaudeEffort::Default,
    },
    ClaudeModel::Opus {
        effort: ClaudeEffort::Default,
    },
    ClaudeModel::Haiku,
];

/// A kit, as a browser edits it.
pub fn fitted(kit: &Kit) -> Fitted {
    match kit {
        Kit::Claude { model } => Fitted {
            agent: wire_name(Agent::Claude).0.to_owned(),
            model: wire_model(*model).0.to_owned(),
            effort: model
                .effort()
                .map_or_else(String::new, |effort| wire_effort(effort).0.to_owned()),
        },
    }
}

/// The kit a browser described, if every part of it is one this build knows.
///
/// Refuses rather than mends: a model this build does not know is not mapped
/// onto the nearest one, and an effort asked of a model that has none is not
/// dropped, because the domain cannot hold that combination.
///
/// # Errors
///
/// Fails if the agent, the model or the effort is not one this build knows,
/// if a model that takes an effort was given none, or if one that takes none
/// was given one.
pub fn kit_of(fitted: &Fitted) -> Result<Kit, Refusal> {
    match named(&fitted.agent)? {
        Agent::Claude => {
            let kind = CLAUDE_MODELS
                .iter()
                .copied()
                .find(|model| wire_model(*model).0 == fitted.model)
                .ok_or_else(|| Refusal::UnknownSetting {
                    field: "model".to_owned(),
                    value: fitted.model.clone(),
                })?;
            let effort = || -> Result<ClaudeEffort, Refusal> {
                if fitted.effort.is_empty() {
                    return Err(Refusal::Incomplete {
                        field: "effort".to_owned(),
                    });
                }
                ClaudeEffort::ALL
                    .iter()
                    .copied()
                    .find(|effort| wire_effort(*effort).0 == fitted.effort)
                    .ok_or_else(|| Refusal::UnknownSetting {
                        field: "effort".to_owned(),
                        value: fitted.effort.clone(),
                    })
            };
            let model = match kind {
                ClaudeModel::Haiku => {
                    if !fitted.effort.is_empty() {
                        return Err(Refusal::EffortNotOnModel {
                            model: wire_model(kind).1.to_owned(),
                        });
                    }
                    ClaudeModel::Haiku
                }
                ClaudeModel::Default { .. } => ClaudeModel::Default { effort: effort()? },
                ClaudeModel::Sonnet { .. } => ClaudeModel::Sonnet { effort: effort()? },
                ClaudeModel::Opus { .. } => ClaudeModel::Opus { effort: effort()? },
            };
            Ok(Kit::Claude { model })
        }
    }
}

/// What one agent can be set to, as the choices a form offers. The first
/// model and the first effort are the agent's defaults.
pub fn shape_of(agent: Agent) -> Shape {
    match agent {
        Agent::Claude => Shape {
            agent: wire_name(agent).0.to_owned(),
            models: CLAUDE_MODELS
                .iter()
                .map(|model| {
                    let (id, name) = wire_model(*model);
                    ModelChoice {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        has_effort: model.effort().is_some(),
                    }
                })
                .collect(),
            efforts: ClaudeEffort::ALL
                .iter()
                .map(|effort| {
                    let (id, name) = wire_effort(*effort);
                    Choice {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    }
                })
                .collect(),
        },
    }
}

/// A kit as a chip is drawn from it: the agent by identifier and by name,
/// the model by name, and the effort by spelling and name where the model
/// takes one — resolved here, because a job's kit is only ever read.
pub fn kit_shown(kit: &Kit) -> stageman_wire::Kit {
    match kit {
        Kit::Claude { model } => stageman_wire::Kit {
            agent: wire_name(Agent::Claude).0.to_owned(),
            agent_name: wire_name(Agent::Claude).1.to_owned(),
            model: wire_model(*model).1.to_owned(),
            effort: model.effort().map(|effort| {
                let (spelled, name) = wire_effort(effort);
                (spelled.to_owned(), name.to_owned())
            }),
        },
    }
}

/// The identifier a browser sent back, as this instance knows it.
///
/// Compared as text rather than parsed, so that a malformed identifier and
/// an unknown one are the same answer.
///
/// # Errors
///
/// Fails if nothing is watched under it.
pub fn identify(state: &State, identifier: &str) -> Result<ProjectId, Refusal> {
    state
        .projects
        .keys()
        .find(|known| known.to_string() == identifier)
        .copied()
        .ok_or_else(|| Refusal::UnknownProject {
            id: identifier.to_owned(),
        })
}

/// Who this instance is on each project's channel, where the channel has
/// said: what a room is linked from, and held rather than kept — see the
/// amendment to
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
pub type Identities = BTreeMap<ProjectId, Identity>;

/// A room as an address a person can open, from where the channel said its
/// workspace is. A caller with no identity to hand links nothing, so that a
/// page links only what is true.
fn room_address(us: &Identity, room: &Room) -> String {
    stageman_channel::room_address(room.channel, us, &room.id)
}

/// The repository as an address a browser can open, when what a project
/// holds is one; a project written before addresses were checked may hold
/// text that is not, which is shown and linked to nothing.
/// A repository as a page names it: its two parts, and nothing composed.
pub fn wire_repository(address: &RepositoryAddress) -> stageman_wire::Repository {
    stageman_wire::Repository {
        owner: address.owner.clone(),
        name: address.name.clone(),
    }
}

/// What a screen calls an agent.
pub fn shown(agent: Agent) -> String {
    wire_name(agent).1.to_owned()
}

/// The projects that would break if this agent were forgotten, by name.
pub fn dependents(state: &State, agent: Agent) -> Vec<String> {
    state
        .used_by(agent)
        .filter_map(|project| state.projects.get(&project))
        .map(|project| project.name.clone())
        .collect()
}

/// What the browser calls a platform.
const fn wire_platform(platform: Platform) -> &'static str {
    match platform {
        Platform::GitHub => "github",
    }
}

/// The platform named by a wire identifier.
///
/// # Errors
///
/// Fails if nothing is called that.
pub fn platform_named(identifier: &str) -> Result<Platform, Refusal> {
    if identifier == wire_platform(Platform::GitHub) {
        Ok(Platform::GitHub)
    } else {
        Err(Refusal::AppMissing {
            platform: identifier.to_owned(),
        })
    }
}

/// What the wire calls a channel when a request names one: the platform's
/// own lowercase spelling, as a platform's is.
pub const fn channel_identifier(channel: Channel) -> &'static str {
    match channel {
        Channel::Slack => "slack",
    }
}

/// The channel named by a wire identifier.
///
/// # Errors
///
/// Fails if nothing is called that.
pub fn channel_named(identifier: &str) -> Result<Channel, Refusal> {
    if identifier == channel_identifier(Channel::Slack) {
        Ok(Channel::Slack)
    } else {
        Err(Refusal::ChannelAppMissing {
            channel: identifier.to_owned(),
        })
    }
}

/// What a screen calls a channel.
pub const fn wire_channel(channel: Channel) -> &'static str {
    match channel {
        Channel::Slack => "Slack",
    }
}

/// One project, as the browser sees it: identifiers where it sends them back,
/// names where a person reads them, and never a credential. The App's
/// installations name the account an installation is on, where the
/// project reaches its repository through one.
pub fn projected(
    id: ProjectId,
    project: &Project,
    us: Option<&Identity>,
    installations: Option<&BTreeMap<u64, Installation>>,
    now: Timestamp,
) -> stageman_wire::Project {
    stageman_wire::Project {
        id: id.to_string(),
        name: project.name.clone(),
        repository: wire_repository(&project.repository),
        repository_link: project.repository.https(),
        foreman: fitted(&project.foreman_kit),
        kits: project
            .kits
            .iter()
            .map(|(name, offered)| KitDraft {
                name: name.to_string(),
                description: offered.description.clone(),
                fitted: fitted(&offered.kit),
            })
            .collect(),
        access: match project.access.get(&Platform::GitHub) {
            Some(Access::Token { owner, expires, .. }) => Some(AccessView::Token {
                owner: owner.clone(),
                expires: expires.map(|at| at.to_string()),
                expired: expires.is_some_and(|at| at <= now),
            }),
            Some(stageman_core::Access::Installation { id }) => Some(AccessView::Installation {
                account: installations
                    .and_then(|known| known.get(id))
                    .map(|installation| installation.account.clone())
                    .unwrap_or_default(),
            }),
            None => None,
        },
        channels: project
            .channels
            .keys()
            .map(|channel| wire_channel(*channel).to_owned())
            .collect(),
        // Names and notes, never values — see
        // `docs/decisions/0075-a-variable-says-what-it-is-for.md`.
        variables: project
            .variables
            .iter()
            .map(|(name, variable)| stageman_wire::Variable {
                name: name.to_string(),
                note: variable.note.clone(),
            })
            .collect(),
        brief: project.brief.clone(),
        watched: project.watched.iter().map(|room| room.id.clone()).collect(),
        foreman_room: project.foreman_room.as_ref().map(|room| room.id.clone()),
        foreman_room_link: project
            .foreman_room
            .as_ref()
            .zip(us)
            .map(|(room, us)| room_address(us, room)),
        attending: !matches!(project.attending, Attending::Idle),
        working: project
            .jobs
            .values()
            .filter(|job| job.progress == Progress::Working)
            .count(),
        jobs: project.jobs.len(),
        // Composed here and never in the browser, like every address a
        // page links — see
        // `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
        token_form: stageman_platform::token_form(Platform::GitHub, Some(&project.name)),
    }
}

/// Every project this instance watches.
pub fn watching(
    state: &State,
    identities: &Identities,
    now: Timestamp,
) -> Vec<stageman_wire::Project> {
    let installations = state
        .apps
        .get(&Platform::GitHub)
        .map(|app| &app.installations);
    state
        .projects
        .iter()
        .map(|(id, project)| projected(*id, project, identities.get(id), installations, now))
        .collect()
}

/// How long before a token expires it is raised on the first page: a week,
/// which is long enough to mint another and short enough not to nag.
const RAISED_BEFORE_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Every token about to expire or expired, soonest first — see
/// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
fn expiring(state: &State, now: Timestamp) -> Vec<stageman_wire::ExpiringToken> {
    let mut raised: Vec<(Timestamp, stageman_wire::ExpiringToken)> = state
        .projects
        .iter()
        .filter_map(
            |(id, project)| match project.access.get(&Platform::GitHub) {
                Some(Access::Token {
                    owner,
                    expires: Some(at),
                    ..
                }) if at
                    .as_second()
                    .checked_sub(now.as_second())
                    .is_some_and(|left| left <= RAISED_BEFORE_SECONDS) =>
                {
                    Some((
                        *at,
                        stageman_wire::ExpiringToken {
                            project: id.to_string(),
                            project_name: project.name.clone(),
                            owner: owner.clone(),
                            expires: at.to_string(),
                            expired: *at <= now,
                        },
                    ))
                }
                _ => None,
            },
        )
        .collect();
    raised.sort_by_key(|(at, _)| *at);
    raised.into_iter().map(|(_, token)| token).collect()
}

/// Every agent this build can run, as the browser sees them — the whole set,
/// because a screen that hid the unconfigured ones could not be used to
/// configure one.
pub fn listed(state: &State) -> Vec<stageman_wire::Agent> {
    Agent::ALL
        .iter()
        .map(|agent| {
            let (id, name) = wire_name(*agent);
            stageman_wire::Agent {
                id: id.to_owned(),
                name: name.to_owned(),
                description: agent.description().to_owned(),
                configured: state.agents.contains_key(agent),
                used_by: dependents(state, *agent),
            }
        })
        .collect()
}

/// The domain's progress, as a page sees it. The failure's prose crosses
/// with it, because it is the only thing a person has to go on.
pub fn standing(progress: &Progress) -> Standing {
    match progress {
        Progress::Working => Standing::Working,
        Progress::Idle(Waiting::Asked) => Standing::Asked,
        Progress::Idle(Waiting::Proposed) => Standing::Proposed,
        Progress::Idle(Waiting::Paused) => Standing::Paused,
        Progress::Idle(Waiting::Silent) => Standing::Idle,
        Progress::Idle(Waiting::Failed(why)) => Standing::Failed { why: why.clone() },
        Progress::Retired(Outcome::Done) => Standing::Done,
        Progress::Retired(Outcome::Discarded) => Standing::Discarded,
        Progress::Retired(Outcome::Lost) => Standing::Lost,
    }
}

/// One project's screen, newest job first.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier.
pub fn working(
    state: &State,
    identities: &Identities,
    project: &str,
    domain: &Domain,
    serving: u16,
) -> Result<stageman_wire::Working, Refusal> {
    let identifier = identify(state, project)?;
    let watched = state
        .projects
        .get(&identifier)
        .ok_or_else(|| Refusal::UnknownProject {
            id: project.to_owned(),
        })?;
    let us = identities.get(&identifier);

    let mut jobs: Vec<stageman_wire::Job> = watched
        .jobs
        .iter()
        .map(|(id, job)| job_view(id, job, &watched.repository, us, domain, serving))
        .collect();
    jobs.sort_by(|one, other| other.created_at.cmp(&one.created_at));

    Ok(stageman_wire::Working {
        name: watched.name.clone(),
        repository: wire_repository(&watched.repository),
        repository_link: watched.repository.https(),
        kits: watched
            .kits
            .iter()
            .map(|(name, offered)| stageman_wire::Offered {
                name: name.to_string(),
                description: offered.description.clone(),
            })
            .collect(),
        jobs,
    })
}

/// One job's page — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
///
/// # Errors
///
/// Fails if nothing is watched under that identifier, or if the project
/// holds no job under the other.
pub fn job_page(
    state: &State,
    identities: &Identities,
    project: &str,
    job: &str,
    domain: &Domain,
    serving: u16,
) -> Result<stageman_wire::JobPage, Refusal> {
    let identifier = identify(state, project)?;
    let watched = state
        .projects
        .get(&identifier)
        .ok_or_else(|| Refusal::UnknownProject {
            id: project.to_owned(),
        })?;
    let unknown = || Refusal::UnknownJob { id: job.to_owned() };
    let named = crate::requests::identify_job(state, identifier, job).ok_or_else(unknown)?;
    let recorded = watched.jobs.get(&named).ok_or_else(unknown)?;
    Ok(stageman_wire::JobPage {
        project: identifier.to_string(),
        project_name: watched.name.clone(),
        repository: wire_repository(&watched.repository),
        repository_link: watched.repository.https(),
        job: job_view(
            &named,
            recorded,
            &watched.repository,
            identities.get(&identifier),
            domain,
            serving,
        ),
    })
}

/// Where a pull request is, when the repository is an address: composed
/// here and never in the browser, per
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`,
/// because the shape of an address is the platform's knowledge.
pub fn pull_request_link(repository: &RepositoryAddress, number: u64) -> String {
    format!("{}/pull/{number}", repository.https())
}

/// One job, as a page sees it: with its room as a link where the channel
/// has said where its workspace is, and as its identifier otherwise.
fn job_view(
    id: &JobId,
    job: &Job,
    repository: &RepositoryAddress,
    us: Option<&Identity>,
    domain: &Domain,
    serving: u16,
) -> stageman_wire::Job {
    stageman_wire::Job {
        id: id.to_string(),
        kit: kit_shown(job.kit()),
        reported: job
            .reported
            .iter()
            .map(|(option, value)| (option.clone(), value.clone()))
            .collect(),
        reason: job.reason.clone(),
        kickoff: job.kickoff.clone(),
        created_at: job.created_at.to_string(),
        standing: standing(&job.progress),
        since: job.since.map(|moment| moment.to_string()),
        tunnel: address(domain, id, serving),
        room: job.room.as_ref().map(|room| room.id.clone()),
        room_link: job
            .room
            .as_ref()
            .zip(us)
            .map(|(room, us)| room_address(us, room)),
        pull_requests: job
            .pull_requests
            .iter()
            .map(|number| stageman_wire::PullRequest {
                number: *number,
                link: pull_request_link(repository, *number),
            })
            .collect(),
    }
}

/// The first page: every idle job, longest waiting first, then every
/// working one, newest first, then the projects — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
///
/// Ordered by the moment each job's standing last changed. A job with no
/// moment kept changed its standing before this build's first stamp, so it
/// has waited longer than any job that carries one: it comes first among
/// those waiting and last among those working, and among its own kind by
/// when it was made.
pub fn home(
    state: &State,
    identities: &Identities,
    domain: &Domain,
    serving: u16,
    now: Timestamp,
) -> stageman_wire::Home {
    let mut idle: Vec<(ProjectId, &Project, &JobId, &Job)> = Vec::new();
    let mut running: Vec<(ProjectId, &Project, &JobId, &Job)> = Vec::new();
    for (project_id, project) in &state.projects {
        for (id, job) in &project.jobs {
            match &job.progress {
                Progress::Idle(_) => idle.push((*project_id, project, id, job)),
                Progress::Working => running.push((*project_id, project, id, job)),
                Progress::Retired(_) => {}
            }
        }
    }
    // A moment that is none sorts before every moment that is some, and an
    // earlier moment before a later one, which is longest waiting first.
    let changed = |job: &Job| (job.since, job.created_at);
    idle.sort_by_key(|placed| changed(placed.3));
    running.sort_by_key(|placed| std::cmp::Reverse(changed(placed.3)));
    let placed = |(project_id, project, id, job): (ProjectId, &Project, &JobId, &Job)| {
        stageman_wire::ProjectJob {
            project: project_id.to_string(),
            project_name: project.name.clone(),
            job: job_view(
                id,
                job,
                &project.repository,
                identities.get(&project_id),
                domain,
                serving,
            ),
        }
    };
    stageman_wire::Home {
        expiring: expiring(state, now),
        needs_you: idle.into_iter().map(placed).collect(),
        working: running.into_iter().map(placed).collect(),
        projects: watching(state, identities, now),
    }
}

/// The line at the foot of every page: this machine, and this build.
pub fn instance(state: &State, runtime: &str, domain: &Domain) -> stageman_wire::Instance {
    stageman_wire::Instance {
        container_runtime: runtime.to_owned(),
        agents: state.agents.len(),
        domain: domain.to_string(),
        version: crate::release::described(),
    }
}

/// What the projects screen shows: the projects, the agents that may be
/// named, the shape of each of those, and whether an App is registered.
pub fn watching_now(
    state: &State,
    identities: &Identities,
    app_registered: bool,
    instance: &str,
    now: Timestamp,
) -> stageman_wire::Watching {
    stageman_wire::Watching {
        projects: watching(state, identities, now),
        available: listed(state)
            .into_iter()
            .filter(|agent| agent.configured)
            .collect(),
        shapes: Agent::ALL
            .iter()
            .filter(|agent| state.agents.contains_key(agent))
            .map(|agent| shape_of(*agent))
            .collect(),
        guides: stageman_wire::Guides {
            token_form: stageman_platform::token_form(Platform::GitHub, None),
            app_form: stageman_channel::app_form(Channel::Slack, instance),
        },
        app_registered,
    }
}

/// The domain's verdict on a state, in terms a screen can show.
pub fn from_inconsistent(reason: &Inconsistent) -> Refusal {
    match reason {
        Inconsistent::NoKits(_) => Refusal::KitsMissing,
        Inconsistent::UnconfiguredProjectAgent { agent, .. } => Refusal::AgentNotConfigured {
            name: shown(*agent),
        },
        Inconsistent::UnknownInstallation { installation, .. } => {
            Refusal::NoSuchInstallation { id: *installation }
        }
        Inconsistent::UnknownWorkspace { workspace, .. } => Refusal::NoSuchWorkspace {
            id: workspace.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Domain, Identities, dependents, fitted, identify, kit_of, kit_shown, listed, named,
        shape_of, shown, standing, wire_channel, wire_name, wire_platform, working,
    };
    use stageman_core::{
        Agent, AgentConfig, ClaudeEffort, ClaudeModel, Job, JobId, Kit, KitConfig, KitName,
        Outcome, Progress, Project, ProjectId, RepositoryAddress, Secret, State, Timestamp, Uuid,
        Waiting,
    };
    use stageman_wire::{Fitted, Refusal, Standing};
    use std::collections::BTreeMap;

    fn every_claude_kit() -> Vec<Kit> {
        let mut kits = vec![Kit::Claude {
            model: ClaudeModel::Haiku,
        }];
        for effort in ClaudeEffort::ALL.iter().copied() {
            for model in [
                ClaudeModel::Default { effort },
                ClaudeModel::Sonnet { effort },
                ClaudeModel::Opus { effort },
            ] {
                kits.push(Kit::Claude { model });
            }
        }
        kits
    }

    /// Every kit crosses to the browser and back as itself: the whole grid,
    /// because a spelling that did not round-trip would send a job on a kit
    /// other than the one the operator saw on the form.
    #[test]
    fn every_kit_survives_the_trip_to_the_browser_and_back() {
        let kits = every_claude_kit();
        assert_eq!(kits.len(), 1 + 3 * ClaudeEffort::ALL.len());
        for kit in kits {
            assert_eq!(kit_of(&fitted(&kit)), Ok(kit.clone()), "{kit:?}");
        }
    }

    #[test]
    fn the_shape_of_claude_offers_an_effort_on_every_model_but_haiku() {
        let shape = shape_of(Agent::Claude);
        assert_eq!(shape.agent, "claude");
        let without: Vec<&str> = shape
            .models
            .iter()
            .filter(|model| !model.has_effort)
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(without, vec!["haiku"]);
        assert_eq!(
            shape.models.first().map(|model| model.id.as_str()),
            Some("default")
        );
        assert_eq!(
            shape.efforts.first().map(|effort| effort.id.as_str()),
            Some("default")
        );
        assert_eq!(shape.efforts.len(), ClaudeEffort::ALL.len());
    }

    /// What the domain cannot hold is refused rather than mended.
    #[test]
    fn a_kit_a_browser_describes_badly_is_refused_rather_than_mended() {
        let claude = |model: &str, effort: &str| Fitted {
            agent: "claude".to_owned(),
            model: model.to_owned(),
            effort: effort.to_owned(),
        };

        assert_eq!(
            kit_of(&claude("haiku", "high")),
            Err(Refusal::EffortNotOnModel {
                model: "Haiku".to_owned()
            })
        );
        assert_eq!(
            kit_of(&claude("opus", "")),
            Err(Refusal::Incomplete {
                field: "effort".to_owned()
            })
        );
        assert_eq!(
            kit_of(&claude("gpt-5", "high")),
            Err(Refusal::UnknownSetting {
                field: "model".to_owned(),
                value: "gpt-5".to_owned()
            })
        );
        assert_eq!(
            kit_of(&claude("opus", "ultra")),
            Err(Refusal::UnknownSetting {
                field: "effort".to_owned(),
                value: "ultra".to_owned()
            })
        );
        assert_eq!(
            kit_of(&Fitted {
                agent: "gpt".to_owned(),
                model: "default".to_owned(),
                effort: "default".to_owned(),
            }),
            Err(Refusal::UnknownAgent {
                name: "gpt".to_owned()
            })
        );
    }

    /// A job's kit is shown with every name a chip needs, resolved here:
    /// the agent by identifier and name, the model by name, and the effort
    /// by spelling and name where the model takes one.
    #[test]
    fn a_kit_is_shown_with_its_names_resolved() {
        let shown = kit_shown(&Kit::Claude {
            model: ClaudeModel::Opus {
                effort: ClaudeEffort::XHigh,
            },
        });
        assert_eq!(shown.agent, "claude");
        assert_eq!(shown.agent_name, "Claude");
        assert_eq!(shown.model, "Opus");
        assert_eq!(
            shown.effort,
            Some(("xhigh".to_owned(), "Extra high".to_owned()))
        );
        assert_eq!(
            kit_shown(&Kit::defaults(Agent::Claude)).effort,
            Some(("default".to_owned(), "Default".to_owned())),
            "the agent's own default is a spelling the chip knows to leave unmetered"
        );
        assert_eq!(
            kit_shown(&Kit::Claude {
                model: ClaudeModel::Haiku,
            })
            .effort,
            None,
            "a model that takes no effort shows none"
        );
    }

    fn watching(name: &str) -> State {
        State {
            apps: std::collections::BTreeMap::new(),
            channel_apps: std::collections::BTreeMap::new(),
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("not-a-real-credential".to_owned()),
                },
            )]),
            projects: BTreeMap::from([(
                ProjectId::from_uuid(Uuid::nil()),
                Project {
                    name: name.to_owned(),
                    repository: stageman_core::RepositoryAddress::new("example", "repo")
                        .expect("an address"),
                    foreman_kit: Kit::defaults(Agent::Claude),
                    kits: BTreeMap::from([(
                        KitName::new("Claude").expect("a name"),
                        KitConfig::defaults(Agent::Claude),
                    )]),
                    access: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    variables: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
                    brief: String::new(),
                    watched: std::collections::BTreeSet::new(),
                    foreman_room: None,
                },
            )]),
        }
    }

    #[test]
    fn every_agent_has_an_identifier_that_round_trips_and_a_name() {
        for agent in Agent::ALL {
            let (id, name) = wire_name(*agent);
            assert_eq!(named(id), Ok(*agent));
            assert!(!id.is_empty() && !name.is_empty());
            assert_eq!(shown(*agent), name);
        }
        assert_eq!(
            named("gpt"),
            Err(Refusal::UnknownAgent {
                name: "gpt".to_owned()
            })
        );
    }

    /// The wire names, asserted as the literal text: with no parser on the
    /// other side, the literal is the whole of the contract.
    #[test]
    fn a_platform_and_a_channel_are_named_by_something_that_says_something() {
        assert_eq!(wire_platform(stageman_core::Platform::GitHub), "github");
        assert_eq!(wire_channel(stageman_core::Channel::Slack), "Slack");
    }

    #[test]
    fn an_identifier_finds_the_project_it_names_and_nothing_else() {
        let id = ProjectId::from_uuid(Uuid::from_u128(9));
        let mut state = watching("aviary");
        let project = state
            .projects
            .values()
            .next()
            .cloned()
            .expect("the project");
        state.projects.clear();
        state.projects.insert(id, project);

        assert_eq!(identify(&state, &id.to_string()), Ok(id));
        let other = ProjectId::from_uuid(Uuid::from_u128(10)).to_string();
        assert_eq!(
            identify(&state, &other),
            Err(Refusal::UnknownProject { id: other.clone() })
        );
        assert!(identify(&state, "not-an-identifier").is_err());
    }

    /// A project counts its running jobs apart from all of them.
    #[test]
    fn a_project_is_shown_with_its_working_jobs_counted_apart() {
        let mut state = watching("aviary");
        let watched = state
            .projects
            .get_mut(&ProjectId::from_uuid(Uuid::nil()))
            .expect("the project");
        for (which, progress) in [
            (1_u128, Progress::Working),
            (2, Progress::Idle(Waiting::Silent)),
            (3, Progress::Retired(Outcome::Done)),
        ] {
            let mut job = Job::new(
                Kit::defaults(Agent::Claude),
                "because".to_owned(),
                "do the thing".to_owned(),
                Timestamp::UNIX_EPOCH,
                stageman_core::Secret::new("warrant-of-a-test-job".to_owned()),
            );
            job.progress = progress;
            watched
                .jobs
                .insert(JobId::from_uuid(Uuid::from_u128(which)), job);
        }

        let shown = super::projected(
            ProjectId::from_uuid(Uuid::nil()),
            watched,
            None,
            None,
            Timestamp::UNIX_EPOCH,
        );
        assert_eq!(shown.working, 1);
        assert_eq!(shown.jobs, 3);
        assert_eq!(shown.name, "aviary");
        assert!(!shown.attending, "nothing in hand");
        assert_eq!(
            shown.repository,
            stageman_wire::Repository {
                owner: "example".to_owned(),
                name: "repo".to_owned()
            }
        );
        assert_eq!(shown.repository_link, "https://github.com/example/repo");

        // On a message, and on another repository.
        watched.attending.take(stageman_core::Errand {
            said: "fix the build".to_owned(),
            thread: stageman_core::Thread {
                channel: stageman_core::Channel::Slack,
                room: "C0123456789".to_owned(),
                id: "1788000000.000001".to_owned(),
            },
            from: None,
            message: None,
            app: None,
        });
        watched.repository = RepositoryAddress::new("owner", "aviary").expect("an address");
        let shown = super::projected(
            ProjectId::from_uuid(Uuid::nil()),
            watched,
            None,
            None,
            Timestamp::UNIX_EPOCH,
        );
        assert!(shown.attending);
        assert_eq!(shown.repository_link, "https://github.com/owner/aviary");
    }

    /// The first page lists what a person does something about, longest
    /// waiting first; then what is working, newest first; and never what is
    /// over. The partition is the domain's own outer state, so this is the
    /// one place it is turned into an order — by the moment a standing
    /// changed, and a job with no moment kept, which changed before this
    /// build's first stamp, comes first among those waiting and last among
    /// those working whatever its record says it was made at.
    #[test]
    fn the_first_page_partitions_jobs_by_what_the_system_does_with_them() {
        let mut state = watching("aviary");
        let watched = state
            .projects
            .get_mut(&ProjectId::from_uuid(Uuid::nil()))
            .expect("the project");
        for (which, second, progress, kept) in [
            (1_u128, 30_i64, Progress::Idle(Waiting::Proposed), true),
            (2, 10, Progress::Idle(Waiting::Asked), true),
            (3, 20, Progress::Working, true),
            (4, 40, Progress::Working, true),
            (5, 50, Progress::Retired(Outcome::Done), true),
            (6, 60, Progress::Idle(Waiting::Silent), false),
            (7, 70, Progress::Working, false),
        ] {
            let mut job = Job::new(
                Kit::defaults(Agent::Claude),
                "because".to_owned(),
                "do the thing".to_owned(),
                Timestamp::from_second(second).expect("a time"),
                stageman_core::Secret::new("warrant-of-a-test-job".to_owned()),
            );
            job.progress = progress;
            if !kept {
                job.since = None;
            }
            watched
                .jobs
                .insert(JobId::from_uuid(Uuid::from_u128(which)), job);
        }
        let domain = Domain::parse("example.com").expect("a domain");

        let shown = super::home(
            &state,
            &super::Identities::new(),
            &domain,
            8080,
            Timestamp::UNIX_EPOCH,
        );

        let named = |placed: &[stageman_wire::ProjectJob]| {
            placed
                .iter()
                .map(|placed| placed.job.id.clone())
                .collect::<Vec<_>>()
        };
        let id = |which: u128| JobId::from_uuid(Uuid::from_u128(which)).to_string();
        assert_eq!(
            named(&shown.needs_you),
            [id(6), id(2), id(1)],
            "longest waiting first, and a job with no moment kept first of all"
        );
        assert_eq!(
            named(&shown.working),
            [id(4), id(3), id(7)],
            "newest first, and a job with no moment kept last of all"
        );
        assert_eq!(shown.needs_you[0].job.since, None);
        assert_eq!(
            shown.needs_you[1].job.since.as_deref(),
            Some("1970-01-01T00:00:10Z"),
            "the moment crosses as the wire spells a time"
        );
        assert_eq!(shown.projects.len(), 1);
        assert!(
            shown
                .needs_you
                .iter()
                .chain(&shown.working)
                .all(|placed| placed.project_name == "aviary"),
            "every job says which project it is on"
        );
    }

    /// The brief crosses as written, and the watched rooms cross as the
    /// platform's identifiers, which is all a screen can show of them.
    #[test]
    fn the_brief_and_the_watched_rooms_cross_as_text() {
        let mut state = watching("aviary");
        let watched = state
            .projects
            .get_mut(&ProjectId::from_uuid(Uuid::nil()))
            .expect("it");
        watched.brief = "Ignore alerts below error.".to_owned();
        watched.watched.insert(stageman_core::Room {
            channel: stageman_core::Channel::Slack,
            id: "C0BT53FM079".to_owned(),
        });

        let shown = super::projected(
            ProjectId::from_uuid(Uuid::nil()),
            watched,
            None,
            None,
            Timestamp::UNIX_EPOCH,
        );
        assert_eq!(shown.brief, "Ignore alerts below error.");
        assert_eq!(shown.watched, vec!["C0BT53FM079".to_owned()]);
    }

    /// The query the whole removal guard rests on.
    #[test]
    fn a_project_naming_an_agent_is_named_as_depending_on_it() {
        assert_eq!(
            dependents(&watching("aviary"), Agent::Claude),
            vec!["aviary".to_owned()]
        );
        assert!(dependents(&State::default(), Agent::Claude).is_empty());
    }

    /// The listing is of every agent, not of the configured ones, and never
    /// carries a credential.
    #[test]
    fn every_agent_is_listed_and_never_its_credential() {
        let empty = listed(&State::default());
        assert_eq!(empty.len(), Agent::ALL.len());
        assert!(empty.iter().all(|agent| !agent.configured));

        let listing = listed(&watching("aviary"));
        let claude = listing.first().expect("Claude is listed");
        assert!(claude.configured);
        assert_eq!(claude.used_by, vec!["aviary".to_owned()]);
        let served = serde_json::to_string(&listing).expect("it serialises");
        assert!(!served.contains("not-a-real-credential"), "{served}");
    }

    /// Every reading crosses as its own standing, and none as another's.
    #[test]
    fn every_reading_crosses_as_a_standing_of_its_own() {
        let crossings = [
            (Progress::Working, Standing::Working),
            (Progress::Idle(Waiting::Asked), Standing::Asked),
            (Progress::Idle(Waiting::Proposed), Standing::Proposed),
            (Progress::Idle(Waiting::Paused), Standing::Paused),
            (Progress::Idle(Waiting::Silent), Standing::Idle),
            (
                Progress::Idle(Waiting::Failed("its container is gone".to_owned())),
                Standing::Failed {
                    why: "its container is gone".to_owned(),
                },
            ),
            (Progress::Retired(Outcome::Done), Standing::Done),
            (Progress::Retired(Outcome::Discarded), Standing::Discarded),
            (Progress::Retired(Outcome::Lost), Standing::Lost),
        ];
        for (progress, expected) in &crossings {
            assert_eq!(standing(progress), *expected, "{progress:?}");
        }
        let reached: std::collections::BTreeSet<&str> = crossings
            .iter()
            .map(|(_, standing)| standing.label())
            .collect();
        assert_eq!(reached.len(), crossings.len());
    }

    /// Every job crosses with the address that reaches that job, newest
    /// first.
    #[test]
    fn a_job_crosses_with_the_address_that_reaches_it() {
        let project = ProjectId::from_uuid(Uuid::from_u128(7));
        let mut state = watching("aviary");
        let mut watched = state
            .projects
            .remove(&ProjectId::from_uuid(Uuid::nil()))
            .expect("it");
        for which in 1..=2_u128 {
            let mut job = Job::new(
                Kit::defaults(Agent::Claude),
                "because".to_owned(),
                "do the thing".to_owned(),
                Timestamp::UNIX_EPOCH,
                stageman_core::Secret::new("warrant-of-a-test-job".to_owned()),
            );
            job.progress = Progress::Idle(Waiting::Silent);
            watched
                .jobs
                .insert(JobId::from_uuid(Uuid::from_u128(which)), job);
        }
        state.projects.insert(project, watched);

        let domain = Domain::parse("example.com").expect("a domain");
        let shown = working(
            &state,
            &Identities::new(),
            &project.to_string(),
            &domain,
            8080,
        )
        .expect("watched");
        assert_eq!(shown.jobs.len(), 2);
        for job in &shown.jobs {
            assert_eq!(job.tunnel, format!("https://{}.example.com", job.id));
            assert_eq!(job.standing, Standing::Idle);
        }
        assert_eq!(shown.kits.len(), 1);
        assert!(
            working(&state, &Identities::new(), "nope", &domain, 8080).is_err(),
            "a project nobody watches"
        );
    }

    /// A pull request's address is the repository's with the number, and a
    /// repository crosses to a page as its two parts with its address
    /// beside it.
    #[test]
    fn a_pull_request_is_addressed_on_the_repository() {
        let named = RepositoryAddress::new("owner", "name").expect("an address");
        assert_eq!(
            super::pull_request_link(&named, 12),
            "https://github.com/owner/name/pull/12"
        );
        assert_eq!(
            super::wire_repository(&named),
            stageman_wire::Repository {
                owner: "owner".to_owned(),
                name: "name".to_owned()
            }
        );
    }
}
