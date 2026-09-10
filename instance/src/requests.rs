//! What a person asks of the dashboard, and what they are answered.
//!
//! Every request enters as an event carrying an identifier and is answered by
//! an effect carrying it back, once whatever the request changed is on the
//! disk. What a page is told is a wire type, built here from the domain;
//! what it is refused is a [`Refusal`], which is safe to send by
//! construction.
//!
//! **Validity is asked, not restated.** What makes an instance valid is
//! `State::check` and nothing else; a request builds the state it would
//! produce, asks, and reports the answer.

use std::collections::BTreeMap;
use std::fmt;

use stageman_core::{
    AgentConfig, Channel, ChannelConfig, JobId, Kit, KitConfig, KitName, Outcome, Platform,
    Progress, Project, ProjectId, Secret, State, Timestamp, VariableName,
};
use stageman_wire::{ChannelDraft, Draft, Ending, KitDraft, Refusal, VariableDraft};

use crate::Instance;
use crate::turns::listening_on;
use crate::views;
use crate::vocabulary::{Effect, RequestId, Speaker};

/// Why a job started, when a person started it.
///
/// Filled in rather than asked for: a person pressing a button has no
/// separate judgement to record, and asking them to phrase one as well as
/// the work would produce two fields saying one thing.
const BY_HAND: &str = "started by hand from the dashboard";

/// What a person can ask.
#[derive(Clone, PartialEq, Eq)]
pub enum Request {
    /// The instance screen.
    Instance,
    /// Every agent, whether or not it is configured.
    Agents,
    /// Give an agent a credential, or replace the one it has.
    Configure {
        /// The agent, by wire identifier.
        agent: String,
        /// The credential.
        credential: String,
    },
    /// Remove an agent's credential, if nothing depends on it.
    ForgetAgent {
        /// The agent, by wire identifier.
        agent: String,
    },
    /// The projects screen.
    Projects,
    /// Start watching a repository.
    Create {
        /// Everything a project is.
        draft: Draft,
    },
    /// Change what a project is, leaving what it has done alone.
    Amend {
        /// The project, by identifier.
        project: String,
        /// Everything a project is.
        draft: Draft,
    },
    /// Stop watching a repository, and reclaim everything it was holding.
    Forget {
        /// The project, by identifier.
        project: String,
    },
    /// One project's screen.
    Jobs {
        /// The project, by identifier.
        project: String,
    },
    /// Start a job on a project.
    Start {
        /// The project, by identifier.
        project: String,
        /// Which of its kits, by name.
        kit: String,
        /// What to do, in the operator's own words.
        work: String,
        /// When it was asked.
        at: Timestamp,
    },
    /// Stop the turn running in a job, keeping everything.
    Stop {
        /// The project, by identifier.
        project: String,
        /// The job, by identifier.
        job: String,
    },
    /// End a job, and reclaim everything it was holding.
    Retire {
        /// The project, by identifier.
        project: String,
        /// The job, by identifier.
        job: String,
        /// The verdict.
        ending: Ending,
    },
}

impl fmt::Debug for Request {
    /// Names what was asked and never a credential. A draft redacts itself;
    /// the one bare credential here is an agent's.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Instance => f.write_str("Instance"),
            Self::Agents => f.write_str("Agents"),
            Self::Configure { agent, .. } => f
                .debug_struct("Configure")
                .field("agent", agent)
                .field("credential", &"<redacted>")
                .finish(),
            Self::ForgetAgent { agent } => {
                f.debug_struct("ForgetAgent").field("agent", agent).finish()
            }
            Self::Projects => f.write_str("Projects"),
            Self::Create { draft } => f.debug_struct("Create").field("draft", draft).finish(),
            Self::Amend { project, draft } => f
                .debug_struct("Amend")
                .field("project", project)
                .field("draft", draft)
                .finish(),
            Self::Forget { project } => f.debug_struct("Forget").field("project", project).finish(),
            Self::Jobs { project } => f.debug_struct("Jobs").field("project", project).finish(),
            Self::Start {
                project,
                kit,
                work,
                at,
            } => f
                .debug_struct("Start")
                .field("project", project)
                .field("kit", kit)
                .field("work", work)
                .field("at", at)
                .finish(),
            Self::Stop { project, job } => f
                .debug_struct("Stop")
                .field("project", project)
                .field("job", job)
                .finish(),
            Self::Retire {
                project,
                job,
                ending,
            } => f
                .debug_struct("Retire")
                .field("project", project)
                .field("job", job)
                .field("ending", ending)
                .finish(),
        }
    }
}

/// What a person is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// The instance screen.
    Instance(stageman_wire::Instance),
    /// The agents screen.
    Agents(Vec<stageman_wire::Agent>),
    /// The projects screen.
    Projects(stageman_wire::Watching),
    /// One project's screen.
    Jobs(stageman_wire::Working),
    /// It was not done, and why.
    Refused(Refusal),
}

impl Instance {
    /// Answers a request, once whatever it changed is on the disk.
    pub fn requested(&mut self, id: RequestId, request: Request, effects: &mut Vec<Effect>) {
        let answered = match request {
            Request::Instance => Ok(Response::Instance(views::overview(
                &self.state,
                &self.runtime,
            ))),
            Request::Agents => Ok(Response::Agents(views::listed(&self.state))),
            Request::Configure { agent, credential } => self.configure(&agent, &credential),
            Request::ForgetAgent { agent } => self.forget_agent(&agent),
            Request::Projects => Ok(Response::Projects(views::watching_now(&self.state))),
            Request::Create { draft } => self.create(&draft),
            Request::Amend { project, draft } => self.amend(&project, &draft),
            Request::Forget { project } => self.forget(&project),
            Request::Jobs { project } => self.jobs(&project),
            Request::Start {
                project,
                kit,
                work,
                at,
            } => self.start_by_hand(&project, &kit, &work, at),
            Request::Stop { project, job } => self.stop(&project, &job, effects),
            Request::Retire {
                project,
                job,
                ending,
            } => self.retire(&project, &job, ending),
        };
        let response = answered.unwrap_or_else(Response::Refused);
        self.defer(Effect::Respond { id, response });
    }

    /// Gives an agent a credential, or replaces the one it has.
    ///
    /// Replacing rather than refusing when one already exists, because
    /// rotating a credential is the ordinary reason to come back to this
    /// screen.
    fn configure(&mut self, agent: &str, credential: &str) -> Result<Response, Refusal> {
        let named = views::named(agent)?;
        let credential = credential.trim();
        if credential.is_empty() {
            return Err(Refusal::CredentialMissing);
        }
        self.state.agents.insert(
            named,
            AgentConfig {
                auth_token: Secret::new(credential.to_owned()),
            },
        );
        self.dirty = true;
        Ok(Response::Agents(views::listed(&self.state)))
    }

    /// Removes an agent's credential, if nothing depends on it.
    fn forget_agent(&mut self, agent: &str) -> Result<Response, Refusal> {
        let named = views::named(agent)?;
        let dependents = views::dependents(&self.state, named);
        if !dependents.is_empty() {
            return Err(Refusal::AgentInUse {
                agent: agent.to_owned(),
                projects: dependents,
            });
        }
        self.state.agents.remove(&named);
        self.dirty = true;
        Ok(Response::Agents(views::listed(&self.state)))
    }

    /// Starts watching a repository.
    ///
    /// Asked of a copy before it is asked of the instance, so that a refusal
    /// leaves nothing changed. Two projects on one channel are refused here
    /// rather than in `State::check`, because a rule there would refuse to
    /// open an instance that already breaks it — putting the repair behind
    /// the door it just locked.
    fn create(&mut self, draft: &Draft) -> Result<Response, Refusal> {
        let name = required("name", &draft.name)?;
        let repository = required("repository", &draft.repository)?;
        let foreman_kit = views::kit_of(&draft.foreman)?;
        let credential = required("credential", &draft.credential)?;
        let kits = kits_of(&draft.kits)?;
        let channels = binding(&draft.channel)?;
        let variables = resolved(&BTreeMap::new(), &draft.variables)?;

        if let Some(taken) = already_bound(&self.state, &channels) {
            return Err(Refusal::ChannelAlreadyBound { project: taken });
        }

        let mut candidate = self.state.clone();
        let created = ProjectId::from_uuid(crate::mint(&mut self.rng));
        candidate.projects.insert(
            created,
            Project {
                name,
                repository,
                foreman_kit,
                kits,
                credentials: BTreeMap::from([(Platform::GitHub, Secret::new(credential))]),
                channels,
                variables,
                jobs: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
            },
        );
        candidate
            .check()
            .map_err(|reason| views::from_inconsistent(&reason))?;
        self.state = candidate;
        self.dirty = true;

        // Listened to from now, not from the next restart: binding a channel
        // used to do nothing until the daemon was restarted, and nothing said
        // so.
        if let Some((opening, speaking)) = self.state.projects.get(&created).and_then(listening_on)
        {
            self.defer(Effect::Listen {
                project: created,
                opening,
                speaking,
            });
        }
        Ok(Response::Projects(views::watching_now(&self.state)))
    }

    /// Changes what a project is, leaving what it has done alone.
    ///
    /// A blank credential means the one it already has, never none: there is
    /// nowhere on the wire for the current value, so the box always starts
    /// empty. The channel is not offered at all, for the same reason.
    fn amend(&mut self, project: &str, draft: &Draft) -> Result<Response, Refusal> {
        let name = required("name", &draft.name)?;
        let repository = required("repository", &draft.repository)?;
        let foreman_kit = views::kit_of(&draft.foreman)?;
        let kits = kits_of(&draft.kits)?;
        let identifier = views::identify(&self.state, project)?;

        let mut candidate = self.state.clone();
        let Some(watched) = candidate.projects.get_mut(&identifier) else {
            return Err(Refusal::UnknownProject {
                id: project.to_owned(),
            });
        };
        watched.variables = resolved(&watched.variables, &draft.variables)?;
        amended(
            watched,
            name,
            repository,
            foreman_kit,
            kits,
            draft.credential.trim(),
        );
        candidate
            .check()
            .map_err(|reason| views::from_inconsistent(&reason))?;
        self.state = candidate;
        self.dirty = true;
        Ok(Response::Projects(views::watching_now(&self.state)))
    }

    /// Stops watching a repository, and reclaims everything it was holding.
    ///
    /// Its containers are removed and its record with them, in one step. A
    /// container that outlives the record — the daemon dying between the
    /// write landing and the removal — is one of this instance's naming a job
    /// it has no record of, which waking removes.
    fn forget(&mut self, project: &str) -> Result<Response, Refusal> {
        let identifier = views::identify(&self.state, project)?;
        let watched =
            self.state
                .projects
                .get(&identifier)
                .ok_or_else(|| Refusal::UnknownProject {
                    id: project.to_owned(),
                })?;
        if let Some(working) = busy(watched) {
            return Err(Refusal::ProjectBusy {
                name: watched.name.clone(),
                working,
            });
        }
        let jobs: Vec<JobId> = watched.jobs.keys().copied().collect();
        for job in &jobs {
            self.forget_tunnel(*job);
            self.defer(Effect::Discard {
                container: stageman_job::container(*job),
            });
        }
        self.defer(Effect::Discard {
            container: stageman_foreman::container(identifier),
        });
        self.defer(Effect::Reclaim);
        self.state.projects.remove(&identifier);
        self.dirty = true;
        Ok(Response::Projects(views::watching_now(&self.state)))
    }

    /// One project's screen.
    fn jobs(&self, project: &str) -> Result<Response, Refusal> {
        Ok(Response::Jobs(views::working(
            &self.state,
            project,
            &self.domain,
            self.serving,
        )?))
    }

    /// Starts a job on a project, by hand.
    ///
    /// A project's kits are the only kits, by hand as much as by the foreman,
    /// so a name outside them is a request this instance refuses rather than
    /// a handout it cannot decide.
    fn start_by_hand(
        &mut self,
        project: &str,
        kit: &str,
        work: &str,
        at: Timestamp,
    ) -> Result<Response, Refusal> {
        let work = work.trim();
        if work.is_empty() {
            return Err(Refusal::Incomplete {
                field: "work".to_owned(),
            });
        }
        let identifier = views::identify(&self.state, project)?;
        let watched =
            self.state
                .projects
                .get(&identifier)
                .ok_or_else(|| Refusal::UnknownProject {
                    id: project.to_owned(),
                })?;
        let chosen = offered(watched, kit).ok_or_else(|| Refusal::KitNotOnProject {
            name: kit.to_owned(),
            project: watched.name.clone(),
        })?;
        self.begin(identifier, chosen, BY_HAND, work, at)
            .map_err(|reason| {
                // Nothing an operator can act on: the project exists and its
                // agents are configured, or the checks above would have
                // refused.
                tracing::error!(%reason, "a job could not be started");
                Refusal::Failed
            })?;
        self.jobs(project)
    }

    /// Stops the turn running in a job, keeping everything.
    ///
    /// A job whose turn ended while the request was in flight is not an
    /// error: it is already where stopping would have left it. The job stays
    /// working until the world says the turn ended, which is what keeps the
    /// reply gate honest.
    fn stop(
        &mut self,
        project: &str,
        job: &str,
        effects: &mut Vec<Effect>,
    ) -> Result<Response, Refusal> {
        let identifier = views::identify(&self.state, project)?;
        let named = identify_job(&self.state, identifier, job)
            .ok_or_else(|| Refusal::UnknownJob { id: job.to_owned() })?;
        let speaker = Speaker::Job(named);
        if let Some(turn) = self.turns.get_mut(&speaker) {
            turn.stopping = true;
            effects.push(Effect::StopTurn { speaker });
            tracing::info!(job = %named, "asked to stop a job");
        } else {
            tracing::debug!(job = %named, "asked to stop a job with no turn in it");
        }
        self.jobs(project)
    }

    /// Ends a job, and reclaims everything it was holding.
    ///
    /// A verdict already recorded is never overwritten, and the container is
    /// asked to go either way, which is what makes this safe to press twice.
    fn retire(&mut self, project: &str, job: &str, ending: Ending) -> Result<Response, Refusal> {
        let identifier = views::identify(&self.state, project)?;
        let named = identify_job(&self.state, identifier, job)
            .ok_or_else(|| Refusal::UnknownJob { id: job.to_owned() })?;
        let Some(recorded) = self.state.job_mut(named) else {
            return Err(Refusal::UnknownJob { id: job.to_owned() });
        };
        match recorded.progress {
            Progress::Working => return Err(Refusal::JobWorking),
            Progress::Retired(_) => {}
            Progress::Idle(_) => {
                recorded.progress = Progress::Retired(match ending {
                    Ending::Done => Outcome::Done,
                    Ending::Discarded => Outcome::Discarded,
                });
                self.dirty = true;
            }
        }
        self.forget_tunnel(named);
        self.defer(Effect::Discard {
            container: stageman_job::container(named),
        });
        self.defer(Effect::Reclaim);
        self.jobs(project)
    }
}

/// The job this identifier names, if this project has one.
///
/// Both halves matter and the second is the one worth the function: a
/// well-formed identifier belonging to *another* project must not be found
/// here, or a stale page could retire a job it is not looking at.
pub fn identify_job(state: &State, project: ProjectId, job: &str) -> Option<JobId> {
    let named = JobId::from_uuid(stageman_core::Uuid::parse_str(job.trim()).ok()?);
    state
        .projects
        .get(&project)?
        .jobs
        .contains_key(&named)
        .then_some(named)
}

/// The kit this project offers under a name, if it offers one. The name is
/// read the way a kit's name is, so a name with space around it still names
/// the kit — and a blank one names nothing.
pub fn offered(project: &Project, name: &str) -> Option<Kit> {
    let wanted = KitName::new(name).ok()?;
    project.kits.get(&wanted).map(|offered| offered.kit.clone())
}

/// What a project's variables become, from the rows the form came back with.
///
/// Four refusals, each a silent wrong answer avoided: a name a container
/// could not be given, a name stageman delivers itself, two rows with one
/// name, and a *new* variable with no value. Everything not named in `rows`
/// is dropped, which is how removal is said — an empty value already means
/// *keep*.
///
/// # Errors
///
/// Any of the four above, each naming the row rather than its contents,
/// because the mistake this most often catches is a credential pasted into a
/// name box.
pub fn resolved(
    held: &BTreeMap<VariableName, Secret>,
    rows: &[VariableDraft],
) -> Result<BTreeMap<VariableName, Secret>, Refusal> {
    let mut wanted = BTreeMap::new();
    // Counted from one, because the operator is looking at a list; by the
    // range rather than by adding to an index.
    for (position, row) in (1..).zip(rows) {
        let name =
            VariableName::new(row.name.trim()).map_err(|rule| Refusal::VariableNameRefused {
                position,
                rule: rule.to_string(),
            })?;
        if stageman_agent::RESERVED.contains(&name.as_str()) {
            return Err(Refusal::VariableReserved {
                name: name.to_string(),
            });
        }
        if wanted.contains_key(&name) {
            return Err(Refusal::VariableRepeated { position });
        }
        let given = row.value.trim();
        let value = if given.is_empty() {
            held.get(&name)
                .cloned()
                .ok_or(Refusal::VariableValueMissing)?
        } else {
            Secret::new(given.to_owned())
        };
        wanted.insert(name, value);
    }
    Ok(wanted)
}

/// Applies what the form came back with to the project it names.
///
/// Blank means the credential already held, never none. A project that had
/// none and is amended with a blank box still has none.
pub fn amended(
    watched: &mut Project,
    name: String,
    repository: String,
    foreman_kit: Kit,
    kits: BTreeMap<KitName, KitConfig>,
    credential: &str,
) {
    watched.name = name;
    watched.repository = repository;
    // Replaced whole, both of them, because the form shows and resubmits
    // the whole of each; a change to the foreman's lands at its next turn.
    watched.foreman_kit = foreman_kit;
    watched.kits = kits;
    if !credential.is_empty() {
        watched
            .credentials
            .insert(Platform::GitHub, Secret::new(credential.to_owned()));
    }
}

/// The kits a form described, refusing what the domain would silently mend.
///
/// # Errors
///
/// Fails if there are no kits, if one has no name or no description, if two
/// share a name, or if one describes settings this build does not know.
pub fn kits_of(drafts: &[KitDraft]) -> Result<BTreeMap<KitName, KitConfig>, Refusal> {
    if drafts.is_empty() {
        return Err(Refusal::KitsMissing);
    }
    let mut kits = BTreeMap::new();
    for draft in drafts {
        let name = KitName::new(draft.name.as_str()).map_err(|_| Refusal::Incomplete {
            field: "kit name".to_owned(),
        })?;
        let description = required("kit description", &draft.description)?;
        let kit = views::kit_of(&draft.fitted)?;
        if kits
            .insert(name.clone(), KitConfig { description, kit })
            .is_some()
        {
            return Err(Refusal::KitNameTaken {
                name: name.to_string(),
            });
        }
    }
    Ok(kits)
}

/// How many of a project's jobs are still going, if any are.
pub fn busy(project: &Project) -> Option<usize> {
    let running = project
        .jobs
        .values()
        .filter(|job| job.progress == Progress::Working)
        .count();
    (running > 0).then_some(running)
}

/// What a project's channel bindings are, from the boxes the form offers:
/// empty, one, or a refusal. A half-bound channel looks bound on every screen
/// right up to the moment a job has a question and nowhere to put it.
///
/// # Errors
///
/// Fails if exactly one half was given, or a credential to listen with and
/// nowhere to listen.
pub fn binding(channel: &ChannelDraft) -> Result<BTreeMap<Channel, ChannelConfig>, Refusal> {
    let address = channel.address.trim();
    let credential = channel.credential.trim();
    let listening = channel.listen_credential.trim();

    match (address.is_empty(), credential.is_empty()) {
        (true, true) if listening.is_empty() => Ok(BTreeMap::new()),
        (false, false) => Ok(BTreeMap::from([(
            Channel::Slack,
            ChannelConfig {
                address: address.to_owned(),
                credential: Secret::new(credential.to_owned()),
                listen_credential: (!listening.is_empty())
                    .then(|| Secret::new(listening.to_owned())),
            },
        )])),
        _ => Err(Refusal::ChannelIncomplete),
    }
}

/// The project already bound where these bindings would go, if any is,
/// named so that an operator can tell which of their projects has it.
pub fn already_bound(state: &State, wanted: &BTreeMap<Channel, ChannelConfig>) -> Option<String> {
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
pub fn required(field: &str, given: &str) -> Result<String, Refusal> {
    let trimmed = given.trim();
    if trimmed.is_empty() {
        return Err(Refusal::Incomplete {
            field: field.to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{already_bound, amended, binding, busy, identify_job, kits_of, offered, resolved};
    use stageman_core::{
        Agent, AgentConfig, Channel, ChannelConfig, ClaudeEffort, ClaudeModel, Job, JobId, Kit,
        KitConfig, KitName, Platform, Progress, Project, ProjectId, Secret, State, Timestamp, Uuid,
        VariableName, Waiting,
    };
    use stageman_wire::{ChannelDraft, Fitted, KitDraft, Refusal, VariableDraft};
    use std::collections::BTreeMap;

    fn drafted(address: &str, credential: &str, listening: &str) -> ChannelDraft {
        ChannelDraft {
            address: address.to_owned(),
            credential: credential.to_owned(),
            listen_credential: listening.to_owned(),
        }
    }

    fn one_kit() -> BTreeMap<KitName, KitConfig> {
        BTreeMap::from([(
            KitName::new("Claude").expect("a name"),
            KitConfig::defaults(Agent::Claude),
        )])
    }

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
    fn holding(jobs: &[Progress]) -> Project {
        Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_kit: Kit::defaults(Agent::Claude),
            kits: one_kit(),
            credentials: BTreeMap::new(),
            channels: BTreeMap::new(),
            variables: BTreeMap::new(),
            attending: stageman_core::Attending::default(),
            jobs: jobs
                .iter()
                .zip(1_u128..)
                .map(|(progress, n)| (JobId::from_uuid(Uuid::from_u128(n)), job(progress.clone())))
                .collect(),
        }
    }

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

    fn row(name: &str, value: &str) -> VariableDraft {
        VariableDraft {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    fn holding_variables(pairs: &[(&str, &str)]) -> BTreeMap<VariableName, Secret> {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    VariableName::new(*name).expect("a deliverable name"),
                    Secret::new((*value).to_owned()),
                )
            })
            .collect()
    }

    fn settled(map: &BTreeMap<VariableName, Secret>) -> Vec<(String, String)> {
        map.iter()
            .map(|(name, value)| (name.to_string(), value.expose().to_owned()))
            .collect()
    }

    /// Amending replaces both the foreman's kit and the kits whole, keeps a
    /// credential when the box is blank, replaces it when typed, and leaves
    /// what the project has done alone.
    #[test]
    fn amending_replaces_what_a_project_is_and_leaves_what_it_has_done() {
        let mut project = holding(&[Progress::Idle(Waiting::Silent)]);
        project
            .credentials
            .insert(Platform::GitHub, Secret::new("ghp-the-old-one".to_owned()));
        project.channels.insert(
            Channel::Slack,
            ChannelConfig {
                address: "C0123456789".to_owned(),
                credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                listen_credential: None,
            },
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
            &mut project,
            "renamed".to_owned(),
            "https://example.invalid/renamed".to_owned(),
            Kit::Claude {
                model: ClaudeModel::Haiku,
            },
            BTreeMap::from([(KitName::new("deep").expect("a name"), deep.clone())]),
            "",
        );
        assert_eq!(project.name, "renamed");
        assert_eq!(
            project.foreman_kit,
            Kit::Claude {
                model: ClaudeModel::Haiku
            }
        );
        assert_eq!(
            project.kits,
            BTreeMap::from([(KitName::new("deep").expect("a name"), deep)])
        );
        assert_eq!(
            project
                .credentials
                .get(&Platform::GitHub)
                .map(Secret::expose),
            Some("ghp-the-old-one"),
            "blank keeps"
        );
        assert_eq!(project.jobs.len(), 1, "its history is not an amendment");
        assert!(project.channels.contains_key(&Channel::Slack));

        amended(
            &mut project,
            "renamed".to_owned(),
            "https://example.invalid/renamed".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            "ghp-the-new-one",
        );
        assert_eq!(
            project
                .credentials
                .get(&Platform::GitHub)
                .map(Secret::expose),
            Some("ghp-the-new-one"),
            "typed replaces"
        );

        let mut none = holding(&[]);
        amended(
            &mut none,
            "aviary".to_owned(),
            "https://example.invalid/aviary".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            "",
        );
        assert!(none.credentials.is_empty(), "blank leaves none as none");
    }

    /// What a form describes becomes the project's kits, and what the domain
    /// would silently mend is refused instead.
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

        assert_eq!(kits_of(&[]), Err(Refusal::KitsMissing));
        assert_eq!(
            kits_of(&[kit_row("  ", "described", "default", "default")]),
            Err(Refusal::Incomplete {
                field: "kit name".to_owned()
            })
        );
        assert_eq!(
            kits_of(&[kit_row("quick", "  ", "default", "default")]),
            Err(Refusal::Incomplete {
                field: "kit description".to_owned()
            })
        );
        assert_eq!(
            kits_of(&[
                kit_row("quick", "one", "haiku", ""),
                kit_row("quick ", "another", "sonnet", "low"),
            ]),
            Err(Refusal::KitNameTaken {
                name: "quick".to_owned()
            })
        );
        assert_eq!(
            kits_of(&[kit_row("quick", "small fixes", "haiku", "high")]),
            Err(Refusal::EffortNotOnModel {
                model: "Haiku".to_owned()
            })
        );
    }

    /// Blank keeps, typed replaces, absence removes, new needs a value.
    #[test]
    fn a_variables_value_keeps_replaces_or_is_required() {
        let held = holding_variables(&[
            ("STRIPE_API_KEY", "sk-test-not-a-real-key"),
            ("DATABASE_URL", "postgres://nowhere"),
        ]);

        let kept = resolved(&held, &[row("STRIPE_API_KEY", ""), row("DATABASE_URL", "")])
            .expect("names it already has");
        assert_eq!(settled(&kept).len(), 2);
        assert_eq!(
            settled(&kept).first().map(|(_, value)| value.as_str()),
            Some("postgres://nowhere")
        );

        let rotated = resolved(&held, &[row("STRIPE_API_KEY", "sk-test-the-new-one")])
            .expect("a name it already has");
        assert_eq!(
            settled(&rotated),
            vec![(
                "STRIPE_API_KEY".to_owned(),
                "sk-test-the-new-one".to_owned()
            )],
            "typed replaces, and the row left out is gone"
        );

        assert_eq!(
            resolved(&BTreeMap::new(), &[row("STRIPE_API_KEY", "")]),
            Err(Refusal::VariableValueMissing)
        );
        let spaced = resolved(&held, &[row(" STRIPE_API_KEY ", "   ")]).expect("spaces are empty");
        assert_eq!(settled(&spaced).len(), 1);
    }

    /// The three refusals that keep a credential out of the process table,
    /// the billing account and a merge.
    #[test]
    fn a_variables_name_is_refused_by_position_and_never_repeated() {
        let refused = resolved(&BTreeMap::new(), &[row("NOT A NAME=oops", "anything")])
            .expect_err("that is not a name");
        assert!(matches!(
            refused,
            Refusal::VariableNameRefused { position: 1, .. }
        ));
        assert!(!format!("{refused}").contains("oops"));

        for claimed in stageman_agent::RESERVED {
            assert_eq!(
                resolved(&BTreeMap::new(), &[row(claimed, "somebody-elses-account")]),
                Err(Refusal::VariableReserved {
                    name: (*claimed).to_owned()
                })
            );
        }

        assert_eq!(
            resolved(
                &BTreeMap::new(),
                &[row("STRIPE_API_KEY", "one"), row("STRIPE_API_KEY", "two")],
            ),
            Err(Refusal::VariableRepeated { position: 2 })
        );
        assert!(resolved(&BTreeMap::new(), &[row("  ", "anything")]).is_err());
    }

    /// Both halves bind, neither binds nothing, one alone is refused, and
    /// listening needs somewhere to listen.
    #[test]
    fn a_channel_is_bound_by_both_halves_or_neither() {
        assert!(binding(&drafted("", "", "")).expect("nothing").is_empty());
        assert!(
            binding(&drafted("  ", "\t", ""))
                .expect("whitespace")
                .is_empty()
        );

        let bound =
            binding(&drafted(" C0123456789 ", " xoxb-not-a-real-token ", "")).expect("both halves");
        let slack = bound.get(&Channel::Slack).expect("keyed by the channel");
        assert_eq!(slack.address, "C0123456789");
        assert_eq!(slack.credential.expose(), "xoxb-not-a-real-token");
        assert!(slack.listen_credential.is_none());

        let listening =
            binding(&drafted("C0123456789", "xoxb-token", "xapp-token")).expect("all three");
        assert_eq!(
            listening
                .get(&Channel::Slack)
                .and_then(|slack| slack.listen_credential.as_ref())
                .map(Secret::expose),
            Some("xapp-token")
        );

        for half in [
            drafted("C0123456789", "", ""),
            drafted("", "xoxb-not-a-real-token", ""),
            drafted("", "", "xapp-token"),
        ] {
            assert!(matches!(binding(&half), Err(Refusal::ChannelIncomplete)));
        }
        let shown = format!("{bound:?}");
        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
    }

    /// Two projects on one channel is refused, and the holder is named.
    #[test]
    fn a_channel_another_project_already_binds_is_refused() {
        let mut state = State::default();
        let mut watched = holding(&[]);
        watched.channels.insert(
            Channel::Slack,
            ChannelConfig {
                address: "C0123456789".to_owned(),
                credential: Secret::new("xoxb-token".to_owned()),
                listen_credential: None,
            },
        );
        state
            .projects
            .insert(ProjectId::from_uuid(Uuid::from_u128(1)), watched);

        let wanted = binding(&drafted("C0123456789", "xoxb-other", "")).expect("a binding");
        assert_eq!(already_bound(&state, &wanted), Some("aviary".to_owned()));
        let elsewhere = binding(&drafted("C9999999999", "xoxb-other", "")).expect("a binding");
        assert_eq!(already_bound(&state, &elsewhere), None);
        assert_eq!(already_bound(&state, &BTreeMap::new()), None);
    }

    /// The count is of running jobs, not of jobs.
    #[test]
    fn a_project_is_busy_for_exactly_its_running_jobs() {
        assert_eq!(busy(&holding(&[])), None);
        assert_eq!(
            busy(&holding(&[
                Progress::Idle(Waiting::Silent),
                Progress::Idle(Waiting::Failed("it did not work".to_owned()))
            ])),
            None
        );
        assert_eq!(busy(&holding(&[Progress::Working])), Some(1));
        assert_eq!(
            busy(&holding(&[
                Progress::Working,
                Progress::Idle(Waiting::Silent),
                Progress::Working,
            ])),
            Some(2)
        );
    }

    /// A kit the project offers is found by its name, however spaced, and a
    /// name it does not offer finds nothing.
    #[test]
    fn a_project_offers_exactly_the_kits_it_names() {
        let quick = KitConfig {
            description: "for small fixes".to_owned(),
            kit: Kit::Claude {
                model: ClaudeModel::Haiku,
            },
        };
        let mut project = holding(&[]);
        project.kits = BTreeMap::from([(KitName::new("quick").expect("a name"), quick.clone())]);

        assert_eq!(offered(&project, "quick"), Some(quick.kit.clone()));
        assert_eq!(offered(&project, " quick "), Some(quick.kit));
        assert_eq!(offered(&project, "deep"), None);
        assert_eq!(offered(&project, ""), None);
    }

    /// A request formats as what was asked, and never as a credential.
    ///
    /// `docs/conventions.md` §4, for the one bare credential a request
    /// carries and the two a draft does.
    #[test]
    fn a_request_names_what_was_asked_and_never_a_credential() {
        use super::Request;
        use stageman_wire::Draft;

        let shown = format!(
            "{:?}",
            Request::Configure {
                agent: "claude".to_owned(),
                credential: "not-a-real-credential".to_owned(),
            }
        );
        assert!(shown.contains("Configure"), "{shown}");
        assert!(shown.contains("claude"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(!shown.contains("not-a-real-credential"), "{shown}");

        let shown = format!(
            "{:?}",
            Request::Create {
                draft: Draft {
                    name: "aviary".to_owned(),
                    credential: "ghp-not-a-real-token".to_owned(),
                    ..Draft::default()
                },
            }
        );
        assert!(shown.contains("Create"), "{shown}");
        assert!(shown.contains("aviary"), "{shown}");
        assert!(!shown.contains("ghp-not-a-real-token"), "{shown}");

        let named = [
            (Request::Instance, "Instance"),
            (Request::Agents, "Agents"),
            (Request::Projects, "Projects"),
            (
                Request::ForgetAgent {
                    agent: "claude".to_owned(),
                },
                "ForgetAgent",
            ),
            (
                Request::Amend {
                    project: "p".to_owned(),
                    draft: Draft::default(),
                },
                "Amend",
            ),
            (
                Request::Forget {
                    project: "p".to_owned(),
                },
                "Forget",
            ),
            (
                Request::Jobs {
                    project: "p".to_owned(),
                },
                "Jobs",
            ),
            (
                Request::Start {
                    project: "p".to_owned(),
                    kit: "Claude".to_owned(),
                    work: "fix it".to_owned(),
                    at: Timestamp::UNIX_EPOCH,
                },
                "Start",
            ),
            (
                Request::Stop {
                    project: "p".to_owned(),
                    job: "j".to_owned(),
                },
                "Stop",
            ),
            (
                Request::Retire {
                    project: "p".to_owned(),
                    job: "j".to_owned(),
                    ending: stageman_wire::Ending::Done,
                },
                "Retire",
            ),
        ];
        for (request, name) in named {
            let shown = format!("{request:?}");
            assert!(shown.contains(name), "{shown}");
        }
    }

    /// A job is found on its own project and on no other.
    #[test]
    fn a_job_is_found_on_its_own_project_and_nowhere_else() {
        let mut state = State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("a-credential".to_owned()),
                },
            )]),
            ..State::default()
        };
        let here = ProjectId::from_uuid(Uuid::from_u128(7));
        let there = ProjectId::from_uuid(Uuid::from_u128(8));
        let mine = JobId::from_uuid(Uuid::from_u128(1));
        let theirs = JobId::from_uuid(Uuid::from_u128(2));
        let mut watched = holding(&[]);
        watched.jobs.insert(mine, job(Progress::Working));
        state.projects.insert(here, watched);
        let mut other = holding(&[]);
        other.jobs.insert(theirs, job(Progress::Working));
        state.projects.insert(there, other);

        assert_eq!(identify_job(&state, here, &mine.to_string()), Some(mine));
        assert_eq!(
            identify_job(&state, here, &format!("  {mine}  ")),
            Some(mine)
        );
        assert_eq!(identify_job(&state, here, &theirs.to_string()), None);
        assert_eq!(identify_job(&state, here, "not-an-identifier"), None);
        assert_eq!(identify_job(&state, here, ""), None);
    }
}
