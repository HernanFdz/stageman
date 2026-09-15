//! What a page is allowed to know, built from the domain.
//!
//! The conversion from domain to wire, kept in one place: every identifier
//! a browser sends back and every name a person reads is decided here, so
//! that adding an agent to the domain stops this compiling until somebody
//! decides what the browser calls it. A wire name is a contract, and
//! deciding it deliberately is the point.

use stageman_core::{
    Agent, Channel, ClaudeEffort, ClaudeModel, Inconsistent, Kit, Outcome, Platform, Progress,
    Project, ProjectId, State, Waiting,
};
use stageman_wire::{Choice, Fitted, KitDraft, ModelChoice, Refusal, Shape, Standing};

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

/// A kit in the words a person reads on a job's row: the agent, and whatever
/// differs from the agent's own defaults.
pub fn described(kit: &Kit) -> String {
    match kit {
        Kit::Claude { model } => {
            let mut parts = vec![wire_name(Agent::Claude).1.to_owned()];
            if !matches!(model, ClaudeModel::Default { .. }) {
                parts.push(wire_model(*model).1.to_owned());
            }
            if let Some(effort) = model.effort()
                && effort != ClaudeEffort::Default
            {
                parts.push(wire_effort(effort).1.to_lowercase());
            }
            parts.join(" · ")
        }
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

/// What a screen calls a channel.
const fn wire_channel(channel: Channel) -> &'static str {
    match channel {
        Channel::Slack => "Slack",
    }
}

/// One project, as the browser sees it: identifiers where it sends them back,
/// names where a person reads them, and never a credential.
pub fn projected(id: ProjectId, project: &Project) -> stageman_wire::Project {
    stageman_wire::Project {
        id: id.to_string(),
        name: project.name.clone(),
        repository: project.repository.clone(),
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
        platforms: project
            .credentials
            .keys()
            .map(|platform| wire_platform(*platform).to_owned())
            .collect(),
        channels: project
            .channels
            .keys()
            .map(|channel| wire_channel(*channel).to_owned())
            .collect(),
        variables: project
            .variables
            .keys()
            .map(std::string::ToString::to_string)
            .collect(),
        brief: project.brief.clone(),
        watched: project.watched.iter().map(|room| room.id.clone()).collect(),
        working: project
            .jobs
            .values()
            .filter(|job| job.progress == Progress::Working)
            .count(),
        jobs: project.jobs.len(),
    }
}

/// Every project this instance watches.
pub fn watching(state: &State) -> Vec<stageman_wire::Project> {
    state
        .projects
        .iter()
        .map(|(id, project)| projected(*id, project))
        .collect()
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

    let mut jobs: Vec<stageman_wire::Job> = watched
        .jobs
        .iter()
        .map(|(id, job)| stageman_wire::Job {
            id: id.to_string(),
            kit: described(job.kit()),
            reported: job
                .reported
                .iter()
                .map(|(option, value)| (option.clone(), value.clone()))
                .collect(),
            reason: job.reason.clone(),
            kickoff: job.kickoff.clone(),
            created_at: job.created_at.to_string(),
            standing: standing(&job.progress),
            tunnel: address(domain, *id, serving),
        })
        .collect();
    jobs.sort_by(|one, other| other.created_at.cmp(&one.created_at));

    Ok(stageman_wire::Working {
        name: watched.name.clone(),
        repository: watched.repository.clone(),
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

/// What the instance screen shows.
pub fn overview(state: &State, runtime: &str) -> stageman_wire::Instance {
    stageman_wire::Instance {
        container_runtime: runtime.to_owned(),
        agents: state.agents.len(),
        projects: watching(state),
    }
}

/// What the projects screen shows: the projects, the agents that may be
/// named, and the shape of each of those.
pub fn watching_now(state: &State) -> stageman_wire::Watching {
    stageman_wire::Watching {
        projects: watching(state),
        available: listed(state)
            .into_iter()
            .filter(|agent| agent.configured)
            .collect(),
        shapes: Agent::ALL
            .iter()
            .filter(|agent| state.agents.contains_key(agent))
            .map(|agent| shape_of(*agent))
            .collect(),
    }
}

/// The domain's verdict on a state, in terms a screen can show.
pub fn from_inconsistent(reason: &Inconsistent) -> Refusal {
    match reason {
        Inconsistent::NoKits(_) => Refusal::KitsMissing,
        Inconsistent::UnconfiguredProjectAgent { agent, .. } => Refusal::AgentNotConfigured {
            name: shown(*agent),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Domain, dependents, described, fitted, identify, kit_of, listed, named, shape_of, shown,
        standing, wire_channel, wire_name, wire_platform, working,
    };
    use stageman_core::{
        Agent, AgentConfig, ClaudeEffort, ClaudeModel, Job, JobId, Kit, KitConfig, KitName,
        Outcome, Progress, Project, ProjectId, Secret, State, Timestamp, Uuid, Waiting,
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

    #[test]
    fn a_kit_is_described_by_what_differs_from_the_defaults() {
        assert_eq!(described(&Kit::defaults(Agent::Claude)), "Claude");
        assert_eq!(
            described(&Kit::Claude {
                model: ClaudeModel::Opus {
                    effort: ClaudeEffort::XHigh,
                },
            }),
            "Claude · Opus · extra high"
        );
        assert_eq!(
            described(&Kit::Claude {
                model: ClaudeModel::Haiku,
            }),
            "Claude · Haiku"
        );
        assert_eq!(
            described(&Kit::Claude {
                model: ClaudeModel::Default {
                    effort: ClaudeEffort::Low,
                },
            }),
            "Claude · low"
        );
    }

    fn watching(name: &str) -> State {
        State {
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
                    repository: "https://example.invalid/repo".to_owned(),
                    foreman_kit: Kit::defaults(Agent::Claude),
                    kits: BTreeMap::from([(
                        KitName::new("Claude").expect("a name"),
                        KitConfig::defaults(Agent::Claude),
                    )]),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    variables: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
                    brief: String::new(),
                    watched: std::collections::BTreeSet::new(),
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
            );
            job.progress = progress;
            watched
                .jobs
                .insert(JobId::from_uuid(Uuid::from_u128(which)), job);
        }

        let shown = super::projected(ProjectId::from_uuid(Uuid::nil()), watched);
        assert_eq!(shown.working, 1);
        assert_eq!(shown.jobs, 3);
        assert_eq!(shown.name, "aviary");
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

        let shown = super::projected(ProjectId::from_uuid(Uuid::nil()), watched);
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
            );
            job.progress = Progress::Idle(Waiting::Silent);
            watched
                .jobs
                .insert(JobId::from_uuid(Uuid::from_u128(which)), job);
        }
        state.projects.insert(project, watched);

        let domain = Domain::parse("example.com").expect("a domain");
        let shown = working(&state, &project.to_string(), &domain, 8080).expect("watched");
        assert_eq!(shown.jobs.len(), 2);
        for job in &shown.jobs {
            assert_eq!(job.tunnel, format!("https://{}.example.com", job.id));
            assert_eq!(job.standing, Standing::Idle);
        }
        assert_eq!(shown.kits.len(), 1);
        assert!(
            working(&state, "nope", &domain, 8080).is_err(),
            "a project nobody watches"
        );
    }
}
