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
    Progress, Project, ProjectId, RepositoryAddress, Secret, State, Variable, VariableName,
};
use stageman_wire::{ChannelDraft, Draft, Ending, KitDraft, Refusal, VariableDraft};

use crate::Effect;
use crate::Running;

use crate::views;
use crate::vocabulary::{AppEffect, RequestId, Speaker};
use crate::{Asked, Command};

/// Why a job started, when a person started it.
///
/// Filled in rather than asked for: a person pressing a button has no
/// separate judgement to record, and asking them to phrase one as well as
/// the work would produce two fields saying one thing.
const BY_HAND: &str = "started by hand from the dashboard";

/// What a person can ask.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Request {
    /// The line at the foot of every page: this machine, and this build.
    Instance,
    /// The first page.
    Home,
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
    /// One job's page.
    Job {
        /// The project, by identifier.
        project: String,
        /// The job, by identifier.
        job: String,
    },
    /// Start a job on a project.
    Start {
        /// The project, by identifier.
        project: String,
        /// Which of its kits, by name.
        kit: String,
        /// What to do, in the operator's own words.
        work: String,
        /// A few words naming it, which its name is made from; the first
        /// words of the work when blank — see
        /// `docs/decisions/0074-a-jobs-identifier-is-its-name.md`.
        title: String,
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
            Self::Home => f.write_str("Home"),
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
            Self::Job { project, job } => f
                .debug_struct("Job")
                .field("project", project)
                .field("job", job)
                .finish(),
            Self::Start {
                project,
                kit,
                work,
                title,
            } => f
                .debug_struct("Start")
                .field("project", project)
                .field("kit", kit)
                .field("work", work)
                .field("title", title)
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Response {
    /// The line at the foot of every page.
    Instance(stageman_wire::Instance),
    /// The first page.
    Home(stageman_wire::Home),
    /// The agents screen.
    Agents(Vec<stageman_wire::Agent>),
    /// The projects screen.
    Projects(stageman_wire::Watching),
    /// One project's screen.
    Jobs(stageman_wire::Working),
    /// One job's page. Boxed, because a page carries the whole instruction
    /// and every other answer is a fraction of its size.
    Job(Box<stageman_wire::JobPage>),
    /// It was not done, and why.
    Refused(Refusal),
}

impl Running {
    /// Answers a request, once whatever it changed is on the disk — or
    /// holds it while the credentials it carries are checked against their
    /// platforms, and answers it once they have, per
    /// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
    pub fn requested(&mut self, id: RequestId, request: Request, effects: &mut Vec<Effect>) {
        match self.hold_for_checks(id, &request, effects) {
            Ok(true) => {}
            Ok(false) => self.respond(id, request, effects),
            Err(refusal) => self.defer(AppEffect::Respond {
                id,
                response: Response::Refused(refusal),
            }),
        }
    }

    /// Answers a request now, once whatever it changed is on the disk.
    pub(crate) fn respond(&mut self, id: RequestId, request: Request, effects: &mut Vec<Effect>) {
        let answered = match request {
            Request::Instance => Ok(Response::Instance(views::instance(
                &self.state,
                &self.runtime.display().to_string(),
                &self.domain,
            ))),
            Request::Home => Ok(Response::Home(views::home(
                &self.state,
                &self.identities(),
                &self.domain,
                self.serving,
            ))),
            Request::Agents => Ok(Response::Agents(views::listed(&self.state))),
            Request::Configure { agent, credential } => self.configure(&agent, &credential),
            Request::ForgetAgent { agent } => self.forget_agent(&agent),
            Request::Projects => Ok(Response::Projects(views::watching_now(
                &self.state,
                &self.identities(),
            ))),
            Request::Create { draft } => self.create(&draft),
            Request::Amend { project, draft } => self.amend(&project, &draft),
            Request::Forget { project } => self.forget(&project),
            Request::Jobs { project } => self.jobs(&project),
            Request::Job { project, job } => self.job_page(&project, &job),
            Request::Start {
                project,
                kit,
                work,
                title,
            } => self.start_by_hand(&project, &kit, &work, &title),
            Request::Stop { project, job } => self.stop(&project, &job, effects),
            Request::Retire {
                project,
                job,
                ending,
            } => self.retire(&project, &job, ending),
        };
        let response = answered.unwrap_or_else(Response::Refused);
        self.defer(AppEffect::Respond { id, response });
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
    /// leaves nothing changed.
    fn create(&mut self, draft: &Draft) -> Result<Response, Refusal> {
        let Drafted {
            name,
            repository,
            foreman_kit,
            credential,
            kits,
            channels,
            variables,
            brief,
        } = drafted(draft, None)?;

        let mut candidate = self.state.clone();
        let created = ProjectId::from_uuid(crate::mint(&mut self.rng));
        candidate.projects.insert(
            created,
            Project {
                name,
                repository: repository.https(),
                foreman_kit,
                kits,
                // Required of a new project, so never empty here; an option
                // only because an amendment may leave the box blank.
                credentials: credential
                    .into_iter()
                    .map(|token| (Platform::GitHub, token))
                    .collect(),
                channels,
                variables,
                jobs: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
                brief,
                watched: std::collections::BTreeSet::new(),
                foreman_room: None,
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
        if let Some(question) = self.listen(created) {
            self.defer(question);
        }
        Ok(Response::Projects(views::watching_now(
            &self.state,
            &self.identities(),
        )))
    }

    /// Changes what a project is, leaving what it has done alone.
    ///
    /// A blank credential means the one it already has, never none: there is
    /// nowhere on the wire for the current value, so the box always starts
    /// empty. The channel is not offered at all, for the same reason.
    fn amend(&mut self, project: &str, draft: &Draft) -> Result<Response, Refusal> {
        let identifier = views::identify(&self.state, project)?;

        let mut candidate = self.state.clone();
        let Some(watched) = candidate.projects.get_mut(&identifier) else {
            return Err(Refusal::UnknownProject {
                id: project.to_owned(),
            });
        };
        let Drafted {
            name,
            repository,
            foreman_kit,
            credential,
            kits,
            variables,
            brief,
            ..
        } = drafted(draft, Some(watched))?;
        watched.variables = variables;
        amended(
            watched,
            name,
            repository.https(),
            foreman_kit,
            kits,
            credential,
            brief,
        );
        candidate
            .check()
            .map_err(|reason| views::from_inconsistent(&reason))?;
        self.state = candidate;
        self.dirty = true;
        Ok(Response::Projects(views::watching_now(
            &self.state,
            &self.identities(),
        )))
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
        let jobs: Vec<JobId> = watched.jobs.keys().cloned().collect();
        for job in &jobs {
            // Its room is archived before its record goes, since the record
            // is what names the room.
            self.archive_room_of(job);
            self.forget_tunnel(job);
            let discard = self.discard(stageman_job::container(job));
            self.defer(discard);
        }
        // The foreman's room too, before the record that names it goes.
        self.archive_foreman_room_of(identifier);
        let discard = self.discard(stageman_foreman::container(identifier));
        self.defer(discard);
        let reclaiming = self.ask(&Command::Images, Asked::Images);
        self.defer(reclaiming);
        // Its channel is no longer listened to, and what is said there from
        // now reaches nobody here.
        for disconnect in self.stop_listening(identifier) {
            self.defer(disconnect);
        }
        self.state.projects.remove(&identifier);
        self.dirty = true;
        Ok(Response::Projects(views::watching_now(
            &self.state,
            &self.identities(),
        )))
    }

    /// One project's screen.
    fn jobs(&self, project: &str) -> Result<Response, Refusal> {
        Ok(Response::Jobs(views::working(
            &self.state,
            &self.identities(),
            project,
            &self.domain,
            self.serving,
        )?))
    }

    /// One job's page.
    fn job_page(&self, project: &str, job: &str) -> Result<Response, Refusal> {
        Ok(Response::Job(Box::new(views::job_page(
            &self.state,
            &self.identities(),
            project,
            job,
            &self.domain,
            self.serving,
        )?)))
    }

    /// Who this instance is on each project's channel, where the channel
    /// has said: held by the listener and never kept, so a page links a room
    /// while the channel is connected and shows its identifier otherwise.
    fn identities(&self) -> views::Identities {
        self.listeners
            .iter()
            .filter_map(|(id, listener)| listener.us.clone().map(|us| (*id, us)))
            .collect()
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
        title: &str,
    ) -> Result<Response, Refusal> {
        let at = self.stamp();
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
        let name = watched.name.clone();
        // A title a person gave, or the first words of the work: the same
        // default the form shows as its placeholder.
        let title = match title.trim() {
            "" => stageman_wire::titled(work),
            given => given.to_owned(),
        };
        let commission = crate::jobs::Commission {
            kit: chosen,
            reason: BY_HAND,
            work,
            title: &title,
        };
        self.begin(identifier, commission, None, at)
            .map_err(|reason| match reason {
                // The one refusal an operator can act on: bind a channel.
                crate::jobs::BeginError::NoChannel(_) => Refusal::ChannelMissing { project: name },
                // Nothing an operator can act on: the project exists and its
                // agents are configured, or the checks above would have
                // refused.
                reason => {
                    tracing::error!(%reason, "a job could not be started");
                    Refusal::Failed
                }
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
        if self.stop_turn(Speaker::Job(named.clone()), effects) {
            tracing::info!(job = %named, "asked to stop a job");
        } else if self
            .state
            .job(&named)
            .is_some_and(|recorded| recorded.progress == Progress::Working)
        {
            // Working with no turn registered: its thread is being read, or
            // its room made. The stop is held until the turn is registered,
            // and takes effect then.
            tracing::info!(job = %named, "asked to stop a job whose turn is not yet registered");
            self.stops_held.insert(named);
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
        let since = self.stamp();
        let Some(recorded) = self.state.job_mut(&named) else {
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
                recorded.since = Some(since);
                self.dirty = true;
            }
        }
        // Its room is archived once the verdict is on the disk: an archived
        // room leaves the sidebar, stays readable, and takes no more posts,
        // which is what makes the conversation over on the platform too.
        self.archive_room_of(&named);
        self.forget_tunnel(&named);
        let discard = self.discard(stageman_job::container(&named));
        self.defer(discard);
        let reclaiming = self.ask(&Command::Images, Asked::Images);
        self.defer(reclaiming);
        self.jobs(project)
    }
}

/// The job this identifier names, if this project has one.
///
/// Both halves matter and the second is the one worth the function: a
/// well-formed identifier belonging to *another* project must not be found
/// here, or a stale page could retire a job it is not looking at.
pub fn identify_job(state: &State, project: ProjectId, job: &str) -> Option<JobId> {
    let named = JobId::parse(job.trim()).ok()?;
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

/// A draft resolved to what a project would hold, before any of it is
/// checked against a platform or kept.
///
/// One resolution for both creating and amending, so that the refusals a
/// draft earns on its own are decided once and in one order, whether the
/// draft goes on to be checked or kept — see
/// `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.
///
/// Deriving `Debug` is safe: the one credential in it redacts itself.
#[derive(Debug)]
pub struct Drafted {
    /// What to call it.
    pub name: String,
    /// Where its jobs work, as an address on the platform.
    pub repository: RepositoryAddress,
    /// How its foreman's agent is set.
    pub foreman_kit: Kit,
    /// The token for the repository: required of a new project, and what
    /// was typed for one that exists — blank means the one already held.
    pub credential: Option<Secret>,
    /// The kits its jobs may run on.
    pub kits: BTreeMap<KitName, KitConfig>,
    /// Its channel bindings, whole. Empty for an amendment, which never
    /// offers them.
    pub channels: BTreeMap<Channel, ChannelConfig>,
    /// Its variables, with a blank value resolved against what it holds.
    pub variables: BTreeMap<VariableName, Variable>,
    /// What its foreman is told every turn.
    pub brief: String,
}

/// Resolves a draft: for a project that does not exist yet when `held` is
/// none, and for the one given otherwise.
///
/// # Errors
///
/// Fails if anything required is missing, if the repository is not an
/// address on the platform, if a kit describes settings this build does not
/// know, if a new project's binding is half given, or if a variable's row
/// is refused.
pub fn drafted(draft: &Draft, held: Option<&Project>) -> Result<Drafted, Refusal> {
    let name = required("name", &draft.name)?;
    let repository = addressed(&draft.repository)?;
    let foreman_kit = views::kit_of(&draft.foreman)?;
    let credential = if held.is_some() {
        let typed = draft.credential.trim();
        (!typed.is_empty()).then(|| typed.to_owned())
    } else {
        Some(required("credential", &draft.credential)?)
    }
    .map(Secret::new);
    let kits = kits_of(&draft.kits)?;
    let channels = match held {
        None => binding(&draft.channel)?,
        Some(_) => BTreeMap::new(),
    };
    let nothing = BTreeMap::new();
    let variables = resolved(
        held.map_or(&nothing, |project| &project.variables),
        &draft.variables,
    )?;
    Ok(Drafted {
        name,
        repository,
        foreman_kit,
        credential,
        kits,
        channels,
        variables,
        brief: draft.brief.trim().to_owned(),
    })
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
    held: &BTreeMap<VariableName, Variable>,
    rows: &[VariableDraft],
) -> Result<BTreeMap<VariableName, Variable>, Refusal> {
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
                .map(|kept| kept.value.clone())
                .ok_or(Refusal::VariableValueMissing)?
        } else {
            Secret::new(given.to_owned())
        };
        // The note is resubmitted whole, like the brief, so blank is blank.
        wanted.insert(
            name,
            Variable {
                value,
                note: row.note.trim().to_owned(),
            },
        );
    }
    Ok(wanted)
}

/// Applies what the form came back with to the project it names.
///
/// No credential means the one already held, never none. A project that
/// had none and is amended with a blank box still has none. The brief is
/// the one text where blank means blank: it is shown in full and
/// resubmitted, so an empty box is an operator taking it away.
pub fn amended(
    watched: &mut Project,
    name: String,
    repository: String,
    foreman_kit: Kit,
    kits: BTreeMap<KitName, KitConfig>,
    credential: Option<Secret>,
    brief: String,
) {
    watched.name = name;
    watched.repository = repository;
    // Replaced whole, both of them, because the form shows and resubmits
    // the whole of each; a change to the foreman's lands at its next turn.
    watched.foreman_kit = foreman_kit;
    watched.kits = kits;
    watched.brief = brief;
    if let Some(token) = credential {
        watched.credentials.insert(Platform::GitHub, token);
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

/// What a project's channel binding is, from the boxes the form offers: one,
/// or a refusal. A project is created with a whole binding, per
/// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`; a
/// partial one looks bound on every screen right up to the moment a job has
/// a question and nowhere to put it, or asks one nobody can answer.
///
/// # Errors
///
/// Fails if either credential is missing.
pub fn binding(channel: &ChannelDraft) -> Result<BTreeMap<Channel, ChannelConfig>, Refusal> {
    let credential = channel.credential.trim();
    let listening = channel.listen_credential.trim();

    if credential.is_empty() || listening.is_empty() {
        return Err(Refusal::ChannelIncomplete);
    }
    Ok(BTreeMap::from([(
        Channel::Slack,
        ChannelConfig {
            credential: Secret::new(credential.to_owned()),
            listen_credential: Secret::new(listening.to_owned()),
        },
    )]))
}

/// The repository, required, and read as an address: an owner and a name
/// on the platform, with what was pasted beside them forgiven — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
/// What is kept is the address as this project writes it.
///
/// # Errors
///
/// Fails if nothing was given, or if what was given is not an address on
/// the platform, saying which rule it broke.
pub fn addressed(given: &str) -> Result<RepositoryAddress, Refusal> {
    let text = required("repository", given)?;
    RepositoryAddress::parse(&text).map_err(|why| Refusal::RepositoryRefused {
        rule: why.to_string(),
    })
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
    use super::{addressed, amended, binding, busy, identify_job, kits_of, offered, resolved};
    use stageman_core::{
        Agent, AgentConfig, Channel, ChannelConfig, ClaudeEffort, ClaudeModel, Job, JobId, Kit,
        KitConfig, KitName, Platform, Progress, Project, ProjectId, Secret, State, Timestamp, Uuid,
        Variable, VariableName, Waiting,
    };
    use stageman_wire::Draft;
    use stageman_wire::{ChannelDraft, Fitted, KitDraft, Refusal, VariableDraft};
    use std::collections::BTreeMap;

    fn drafted(credential: &str, listening: &str) -> ChannelDraft {
        ChannelDraft {
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
            brief: String::new(),
            watched: std::collections::BTreeSet::new(),
            foreman_room: None,
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
            note: String::new(),
        }
    }

    fn holding_variables(pairs: &[(&str, &str)]) -> BTreeMap<VariableName, Variable> {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    VariableName::new(*name).expect("a deliverable name"),
                    Variable::unexplained(Secret::new((*value).to_owned())),
                )
            })
            .collect()
    }

    fn settled(map: &BTreeMap<VariableName, Variable>) -> Vec<(String, String)> {
        map.iter()
            .map(|(name, variable)| (name.to_string(), variable.value.expose().to_owned()))
            .collect()
    }

    /// A note is kept with its variable, trimmed, and blank is blank: it is
    /// shown in full and resubmitted, so nothing is inherited from the
    /// project as a value is.
    #[test]
    fn a_variables_note_is_kept_trimmed_and_blank_is_blank() {
        let held = holding_variables(&[("STRIPE_API_KEY", "sk-test-not-a-real-key")]);
        let explained = resolved(
            &held,
            &[VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: String::new(),
                note: "  the payment provider, in test mode  ".to_owned(),
            }],
        )
        .expect("a name it already has");
        let variable = explained.values().next().expect("the one variable");
        assert_eq!(variable.note, "the payment provider, in test mode");
        assert_eq!(
            variable.value.expose(),
            "sk-test-not-a-real-key",
            "blank keeps the value"
        );

        let unsaid = resolved(&explained, &[row("STRIPE_API_KEY", "")]).expect("kept again");
        assert_eq!(
            unsaid
                .values()
                .next()
                .map(|variable| variable.note.as_str()),
            Some(""),
            "a note left blank is blank, not the one before"
        );
    }

    /// A repository is kept as the address this project writes, whatever was
    /// pasted beside it, and refused when it is not one.
    #[test]
    fn a_repository_is_written_as_an_address_or_refused_by_rule() {
        assert_eq!(
            addressed("https://github.com/HernanFdz/stageman.git/").map(|address| address.https()),
            Ok("https://github.com/HernanFdz/stageman".to_owned())
        );
        assert_eq!(
            addressed("  "),
            Err(Refusal::Incomplete {
                field: "repository".to_owned()
            })
        );
        assert!(matches!(
            addressed("git@github.com:HernanFdz/stageman.git"),
            Err(Refusal::RepositoryRefused { .. })
        ));
        assert!(matches!(
            addressed("https://example.invalid/aviary"),
            Err(Refusal::RepositoryRefused { .. })
        ));
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
                credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                listen_credential: Secret::new("xapp-token".to_owned()),
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
            None,
            "Ignore alerts below error.".to_owned(),
        );
        assert_eq!(project.name, "renamed");
        assert_eq!(project.brief, "Ignore alerts below error.");
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
            Some(Secret::new("ghp-the-new-one".to_owned())),
            String::new(),
        );
        assert_eq!(
            project
                .credentials
                .get(&Platform::GitHub)
                .map(Secret::expose),
            Some("ghp-the-new-one"),
            "typed replaces"
        );
        assert_eq!(project.brief, "", "a blank brief is taken away, not kept");

        let mut none = holding(&[]);
        amended(
            &mut none,
            "aviary".to_owned(),
            "https://example.invalid/aviary".to_owned(),
            Kit::defaults(Agent::Claude),
            one_kit(),
            None,
            String::new(),
        );
        assert!(none.credentials.is_empty(), "blank leaves none as none");
    }

    /// One resolution for both forms: a new project needs its token and a
    /// whole binding, and an existing one takes a blank token as the one
    /// it holds, a typed one as new, and no binding at all.
    #[test]
    fn a_draft_is_resolved_once_for_creating_and_for_amending() {
        let mut draft = Draft {
            name: " aviary ".to_owned(),
            repository: "https://github.com/example/aviary.git".to_owned(),
            foreman: stageman_wire::Fitted {
                agent: "claude".to_owned(),
                model: "default".to_owned(),
                effort: "default".to_owned(),
            },
            kits: vec![kit_row("Claude", "General-purpose.", "default", "default")],
            credential: String::new(),
            channel: ChannelDraft {
                credential: "xoxb-not-a-real-token".to_owned(),
                listen_credential: "xapp-not-a-real-token".to_owned(),
            },
            variables: vec![row("HELD", "")],
            brief: " be brief ".to_owned(),
        };
        assert!(
            matches!(
                super::drafted(&draft, None),
                Err(Refusal::Incomplete { ref field }) if field == "credential"
            ),
            "a new project needs its token"
        );

        let mut held = holding(&[]);
        held.variables = holding_variables(&[("HELD", "kept")]);
        let amending = super::drafted(&draft, Some(&held)).expect("resolved against the project");
        assert_eq!(amending.name, "aviary");
        assert_eq!(
            amending.repository.https(),
            "https://github.com/example/aviary"
        );
        assert!(amending.credential.is_none(), "blank keeps");
        assert!(amending.channels.is_empty(), "never offered when amending");
        assert_eq!(
            settled(&amending.variables),
            vec![("HELD".to_owned(), "kept".to_owned())]
        );
        assert_eq!(amending.brief, "be brief");

        draft.credential = " github_pat_not_a_real_token ".to_owned();
        let creating =
            super::drafted(&draft, None).expect_err("a new project holds no HELD to keep");
        assert_eq!(creating, Refusal::VariableValueMissing);
        draft.variables.clear();
        let creating = super::drafted(&draft, None).expect("a whole draft");
        assert_eq!(
            creating.credential.as_ref().map(Secret::expose),
            Some("github_pat_not_a_real_token"),
            "trimmed, and kept"
        );
        assert!(creating.channels.contains_key(&Channel::Slack));
        assert_eq!(
            super::drafted(&draft, Some(&held))
                .expect("typed replaces")
                .credential
                .as_ref()
                .map(Secret::expose),
            Some("github_pat_not_a_real_token")
        );
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

    /// Both credentials bind, trimmed, and anything less is refused.
    #[test]
    fn a_channel_is_bound_by_both_credentials_or_refused() {
        let bound = binding(&drafted(
            " xoxb-not-a-real-token ",
            " xapp-not-a-real-token ",
        ))
        .expect("both");
        let slack = bound.get(&Channel::Slack).expect("keyed by the channel");
        assert_eq!(slack.credential.expose(), "xoxb-not-a-real-token");
        assert_eq!(slack.listen_credential.expose(), "xapp-not-a-real-token");

        for partial in [
            drafted("", ""),
            drafted("  ", "\t"),
            drafted("xoxb-not-a-real-token", ""),
            drafted("", "xapp-token"),
        ] {
            assert!(
                matches!(binding(&partial), Err(Refusal::ChannelIncomplete)),
                "{partial:?}"
            );
        }
        let shown = format!("{bound:?}");
        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
        assert!(!shown.contains("xapp-not-a-real-token"), "{shown}");
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
            (Request::Home, "Home"),
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
                    title: String::new(),
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
        watched.jobs.insert(mine.clone(), job(Progress::Working));
        state.projects.insert(here, watched);
        let mut other = holding(&[]);
        other.jobs.insert(theirs.clone(), job(Progress::Working));
        state.projects.insert(there, other);

        assert_eq!(
            identify_job(&state, here, &mine.to_string()),
            Some(mine.clone())
        );
        assert_eq!(
            identify_job(&state, here, &format!("  {mine}  ")),
            Some(mine)
        );
        assert_eq!(identify_job(&state, here, &theirs.to_string()), None);
        assert_eq!(identify_job(&state, here, "not-an-identifier"), None);
        assert_eq!(identify_job(&state, here, ""), None);
    }
}
