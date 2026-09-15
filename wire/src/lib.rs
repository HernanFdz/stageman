//! What crosses between the dashboard and the instance.
//!
//! Every type here is plain and serialisable, built by the instance from the
//! domain and never the domain itself — see
//! `docs/decisions/0022-the-browser-never-sees-the-domain.md`. What is in the
//! browser is in the browser's hands, so what reaches it is chosen rather
//! than inherited. Note what none of these types can carry: a credential.
//! That is a property of the types rather than of the routes, and a field
//! added here is a field the browser gets.
//!
//! Names are identifiers where a browser sends them back and display names
//! where a person reads them, and the two are deliberately never the same
//! field.

use std::fmt;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- agents

/// One agent this instance could run, as much of it as a page may know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// What the browser names it back as.
    pub id: String,
    /// What it is called on screen.
    pub name: String,
    /// What it is good for.
    pub description: String,
    /// Whether a credential has been supplied for it.
    pub configured: bool,
    /// The projects that would break if it were forgotten. Empty means it
    /// can go; anything else is what a refusal would say, and carrying it
    /// lets a page grey the button *and* explain.
    pub used_by: Vec<String>,
}

// -------------------------------------------------------------- instance

/// One instance, as much of it as a page is allowed to know: counts and
/// names, and nothing that could be a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// Where this machine's container runtime was found. A path rather
    /// than a version, because the operator's next question when something
    /// misbehaves is *which one is it using*.
    pub container_runtime: String,
    /// How many agents are configured. A count and not a list, because an
    /// agent's configuration is a credential.
    pub agents: usize,
    /// The projects this instance watches.
    pub projects: Vec<Project>,
}

// -------------------------------------------------------------- projects

/// One project, as much of it as a page is allowed to know.
///
/// Which platforms have a credential and which channels are bound is here;
/// what any of them is, is not — and a channel's address is withheld on the
/// same terms even though it is not a secret, because nothing on a screen
/// needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// What the browser names it back as.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// How its foreman's agent is set, as the identifiers a browser sends
    /// back — not the names a person reads.
    pub foreman: Fitted,
    /// The kits its jobs may run on, as the form edits them. Never empty in
    /// a valid instance.
    pub kits: Vec<KitDraft>,
    /// The platforms it has a credential for.
    pub platforms: Vec<String>,
    /// The channels bound to it. Empty is valid: a project with nowhere to
    /// escalate can still run work that never needs to ask.
    pub channels: Vec<String>,
    /// The variables its jobs are given, by name. Names and never values.
    pub variables: Vec<String>,
    /// What its operator wrote for its foreman, as the form edits it.
    pub brief: String,
    /// The rooms its foreman watches, by the platform's identifier: shown
    /// and never edited here, since a room is watched by asking the foreman
    /// in it.
    pub watched: Vec<String>,
    /// How many of its jobs are still running.
    pub working: usize,
    /// How many jobs it has had, running or finished.
    pub jobs: usize,
}

impl Project {
    /// Whether nothing of this project's is currently running.
    ///
    /// The screen's version of the rule the instance enforces, and the
    /// reason it is a method: a page that offered a control the instance
    /// would refuse would be lying.
    #[must_use]
    pub const fn idle(&self) -> bool {
        self.working == 0
    }
}

/// What the projects screen needs in order to draw itself.
///
/// One answer rather than two, because the screen cannot offer to create a
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
    /// What each of those can be set to. Built by the instance from the
    /// domain's closed sets, because the browser's half cannot name them.
    pub shapes: Vec<Shape>,
}

/// One agent, set a particular way, as a browser edits it.
///
/// Identifiers throughout. The effort is empty where the model offers none,
/// which is the one shape of absence this type has.
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
    /// The effort is not required here because the screen cannot know
    /// whether the model takes one without the shape, and the instance
    /// refuses a model given none.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        !self.agent.is_empty() && !self.model.is_empty()
    }
}

/// One kit a project offers, as a browser edits it.
///
/// The description is required: it is the whole of what the foreman chooses
/// by — see `docs/decisions/0048-a-job-runs-on-a-kit.md`.
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

/// The boxes that bind a channel, travelling together.
///
/// Both filled binds a channel, and creating a project needs one: anything
/// less is refused, and that rule is written in one place, on the instance.
/// Amending never offers them.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChannelDraft {
    /// What speaks on that channel.
    pub credential: String,
    /// What listens on it.
    pub listen_credential: String,
}

impl fmt::Debug for ChannelDraft {
    /// Names what was given and never the credentials. They are bare
    /// `String`s on the way in from a browser, so nothing under them
    /// redacts, and a derive would print them whole.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelDraft")
            .field("credential", &"<redacted>")
            .field("listen_credential", &"<redacted>")
            .finish()
    }
}

/// What the project form is currently for.
///
/// One value rather than two flags, so that *adding and amending at once*
/// is a sentence that cannot be said. Closed is the absence of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filling {
    /// A project that does not exist yet. Everything is required.
    Creating,
    /// One that already does, named by the identifier it is known by. A
    /// blank credential means the one it already has.
    Amending(String),
}

impl Filling {
    /// Whether this describes a project that does not exist yet.
    #[must_use]
    pub const fn creating(&self) -> bool {
        matches!(self, Self::Creating)
    }
}

/// One row of the variables table, as the browser sends it back.
///
/// **An empty value means the one the project already holds**, exactly as
/// the credential box does: no value ever reaches a browser, so the box
/// always starts empty. Removal is the row being absent.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VariableDraft {
    /// What the variable is called in the container.
    pub name: String,
    /// What it is set to, or empty to keep what the project holds.
    pub value: String,
}

impl fmt::Debug for VariableDraft {
    /// Names it and never says what it is set to.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableDraft")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Everything the project form collects, which is everything a project is.
///
/// A struct rather than one parameter per field, because the form's whole
/// purpose is to be filled in twice — once to create and once to change —
/// and a caller should differ in what it *does* with the answer rather than
/// in how it receives it.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Draft {
    /// What to call it.
    pub name: String,
    /// Where its jobs work.
    pub repository: String,
    /// How its foreman's agent is set.
    pub foreman: Fitted,
    /// The kits its jobs may run on.
    pub kits: Vec<KitDraft>,
    /// What reaches the repository.
    pub credential: String,
    /// Where this project's conversation happens, if anywhere.
    pub channel: ChannelDraft,
    /// What its jobs are given that this project never reads.
    pub variables: Vec<VariableDraft>,
    /// What its foreman is told every turn, in the operator's words. Empty
    /// is none, and says nothing.
    pub brief: String,
}

impl fmt::Debug for Draft {
    /// Names the fields and neither credential.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draft")
            .field("name", &self.name)
            .field("repository", &self.repository)
            .field("foreman", &self.foreman)
            .field("kits", &self.kits)
            .field("credential", &"<redacted>")
            .field("channel", &self.channel)
            .field("variables", &self.variables)
            .field("brief", &self.brief)
            .finish()
    }
}

impl Draft {
    /// Whether this says everything a project needs.
    ///
    /// The same conditions the instance enforces, deliberately: the control
    /// that submits is unavailable until pressing it would succeed. It is
    /// not a second definition of validity — the instance still checks —
    /// but it is the screen refusing to ask the question badly. Creating
    /// needs a credential and a whole channel binding; amending needs
    /// neither, because a blank credential there means the one already held
    /// and the channel is not offered at all. A row of variables needs a
    /// value unless the project already holds that name.
    #[must_use]
    pub fn is_complete(&self, filling: &Filling, held: &[String]) -> bool {
        let described = !self.name.trim().is_empty()
            && !self.repository.trim().is_empty()
            && self.foreman.is_complete()
            && !self.kits.is_empty()
            && self.kits.iter().all(KitDraft::is_complete)
            && distinct(&self.kits);
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
                    && !self.channel.credential.trim().is_empty()
                    && !self.channel.listen_credential.trim().is_empty()
            }
            Filling::Amending(_) => described && named && valued,
        }
    }
}

/// Whether no two kits share a name, once the space around each is gone.
///
/// The domain keys kits on their names and would keep one of two silently,
/// so the screen refuses to submit the pair.
#[must_use]
pub fn distinct(kits: &[KitDraft]) -> bool {
    let names: std::collections::BTreeSet<&str> = kits.iter().map(|kit| kit.name.trim()).collect();
    names.len() == kits.len()
}

// ------------------------------------------------------------------ jobs

/// Where a job has got to, as a page sees it.
///
/// **Flat, where the domain's is two levels.** The domain nests because its
/// outer level decides behaviour and nothing here decides any. Nothing here
/// claims work is finished: `Proposed` is the agent's own account of
/// itself, and `Done` is the only verdict, and it is a person's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "standing", rename_all = "snake_case")]
pub enum Standing {
    /// Its agent has been given something and has not stopped.
    Working,
    /// Its agent asked something and stopped.
    Asked,
    /// Its agent believes the work is done and is inviting somebody to look.
    Proposed,
    /// A person stopped it while it was working.
    Paused,
    /// Its agent stopped and said nothing about why. Called *idle* here and
    /// silent in the domain, because a page is showing a job that simply
    /// stopped.
    Idle,
    /// Its turn ended badly, and this is what went wrong.
    Failed {
        /// Prose for a person, not a code to branch on.
        why: String,
    },
    /// Over, and a person judged it produced what was wanted.
    Done,
    /// Over, and a person judged it did not.
    Discarded,
    /// Over, because its container went missing.
    Lost,
}

impl Standing {
    /// What this is called.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Asked => "asked",
            Self::Proposed => "proposed",
            Self::Paused => "paused",
            Self::Idle => "idle",
            Self::Failed { .. } => "failed",
            Self::Done => "done",
            Self::Discarded => "discarded",
            Self::Lost => "lost",
        }
    }

    /// Whether the job is over: nothing left to stop and nothing to reclaim.
    #[must_use]
    pub const fn is_over(&self) -> bool {
        matches!(self, Self::Done | Self::Discarded | Self::Lost)
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
    /// Beside the kit rather than folded into it, because the two were
    /// measured to differ.
    pub reported: Vec<(String, String)>,
    /// Why it was started, in prose.
    pub reason: String,
    /// What its agent was told to do — the whole instruction, because it is
    /// the only record of what the agent was actually asked.
    pub kickoff: String,
    /// When the record was made.
    pub created_at: String,
    /// Where it has got to.
    pub standing: Standing,
    /// Where to look at whatever it is showing. Always present, and it
    /// promises nothing: the port is published when the container is
    /// created, whether or not the agent ever uses it.
    pub tunnel: String,
}

/// One kit a project offers, as much of it as a page needs to offer it back.
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

/// How a job ends, as a browser asks for it.
///
/// Two of the three outcomes and never the third: *lost* is what waking
/// writes on finding a container gone, and nothing a person presses should
/// be able to claim it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ending {
    /// It produced what was wanted.
    Done,
    /// It did not, and nothing of it is kept.
    Discarded,
}

// -------------------------------------------------------------- refusals

/// What a request can be refused with, in a shape the browser can match on.
///
/// Every variant is safe to send. That is a rule about what may be added
/// here rather than an observation: a failure carrying something an operator
/// should not read is reported as [`Refusal::Failed`] and logged where the
/// operator can see it, because the browser is the one audience that gets
/// no say in who is looking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Refusal {
    /// Nothing by that name can be run. The set of agents is closed and
    /// compiled in, so this is a stale page or a hand-made request.
    #[error("no agent is called {name}")]
    UnknownAgent {
        /// What was asked for.
        name: String,
    },
    /// An agent was configured with nothing.
    #[error("that agent needs a credential")]
    CredentialMissing,
    /// An agent cannot be forgotten while a project names it.
    #[error("{agent} is still used by {}", projects.join(", "))]
    AgentInUse {
        /// The agent that was to be forgotten.
        agent: String,
        /// What would have broken, by name.
        projects: Vec<String>,
    },
    /// Nothing by that identifier is being watched.
    #[error("no project has the identifier {id}")]
    UnknownProject {
        /// What was asked for.
        id: String,
    },
    /// A field that has to say something says nothing.
    #[error("{field} cannot be empty")]
    Incomplete {
        /// The field, named as the screen names it.
        field: String,
    },
    /// A project would have no kit its jobs could run on.
    #[error("a project needs at least one kit its jobs can run on")]
    KitsMissing,
    /// A project was drafted without a whole channel binding.
    #[error("a project needs a Slack bot token and an app-level token")]
    ChannelIncomplete,
    /// A job was asked for on a project that has no channel bound, which only
    /// a project the last release wrote can lack.
    #[error("{project} has no Slack binding, so a job on it would have nowhere to speak")]
    ChannelMissing {
        /// The project, as the screen names it.
        project: String,
    },
    /// A project names an agent that has no credential.
    #[error("{name} has no credential, so a project cannot name it")]
    AgentNotConfigured {
        /// The agent, as the screen names it.
        name: String,
    },
    /// A project's jobs may not run on that kit.
    #[error("{project} offers no kit called {name}")]
    KitNotOnProject {
        /// The kit, as the operator named it.
        name: String,
        /// The project, as the screen names it.
        project: String,
    },
    /// Two of a project's kits were given the same name.
    #[error("two kits are called {name}; a name has to pick one out")]
    KitNameTaken {
        /// The name given twice.
        name: String,
    },
    /// A kit names a setting the agent does not have.
    #[error("no {field} called {value}")]
    UnknownSetting {
        /// Which setting: the model, or the effort.
        field: String,
        /// What was asked for.
        value: String,
    },
    /// A kit asks for an effort on a model that offers no such choice.
    #[error("{model} has no effort to choose")]
    EffortNotOnModel {
        /// The model, as the screen names it.
        model: String,
    },
    /// A variable was given a name a container could not be given. Says
    /// which row and never what was in it: the mistake this most often
    /// catches is a credential pasted into the name box.
    #[error("variable {position} has a name an environment cannot carry: {rule}")]
    VariableNameRefused {
        /// Which row, counting from one.
        position: usize,
        /// Which rule it broke.
        rule: String,
    },
    /// A variable claims a name stageman delivers itself.
    #[error("{name} is a variable stageman sets itself, so a project cannot")]
    VariableReserved {
        /// The reserved name that was claimed.
        name: String,
    },
    /// Two rows give the same name.
    #[error("variable {position} repeats a name given earlier")]
    VariableRepeated {
        /// Which row, counting from one.
        position: usize,
    },
    /// A variable that does not exist yet was given no value.
    #[error("a new variable needs a value")]
    VariableValueMissing,
    /// A project cannot be forgotten while its jobs are still running.
    #[error("{name} still has {working} job(s) working")]
    ProjectBusy {
        /// The project, as the screen names it.
        name: String,
        /// How many jobs are still going.
        working: usize,
    },
    /// The project is watched and holds no job under that identifier.
    #[error("no job here is {id}")]
    UnknownJob {
        /// What was asked for, as it arrived.
        id: String,
    },
    /// A turn is running in that job, so it cannot be retired yet.
    #[error("that job is still working — stop it before retiring it")]
    JobWorking,
    /// Something went wrong that the operator cannot act on from here.
    #[error("that did not work — the server log says why")]
    Failed,
}

impl Refusal {
    /// The HTTP status this answers with.
    ///
    /// A dashboard barely needs these — it reads the variant, not the number
    /// — but anything else speaking to these routes does, and a route that
    /// answers 200 for a refusal is lying to every client that is not this
    /// page.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Failed => 500,
            Self::UnknownAgent { .. } | Self::UnknownProject { .. } | Self::UnknownJob { .. } => {
                404
            }
            // Well-formed requests that describe something invalid, which
            // the operator can fix by typing something different.
            Self::CredentialMissing
            | Self::Incomplete { .. }
            | Self::KitsMissing
            | Self::AgentNotConfigured { .. }
            | Self::KitNotOnProject { .. }
            | Self::KitNameTaken { .. }
            | Self::UnknownSetting { .. }
            | Self::EffortNotOnModel { .. }
            | Self::VariableNameRefused { .. }
            | Self::VariableReserved { .. }
            | Self::VariableRepeated { .. }
            | Self::VariableValueMissing
            | Self::ChannelIncomplete => 400,
            // The request is well formed and the instance is in a state that
            // forbids it, which is what a conflict means.
            Self::AgentInUse { .. }
            | Self::ProjectBusy { .. }
            | Self::JobWorking
            | Self::ChannelMissing { .. } => 409,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelDraft, Draft, Filling, Fitted, KitDraft, Refusal, Standing, VariableDraft, distinct,
    };

    fn as_it_comes() -> Fitted {
        Fitted {
            agent: "claude".to_owned(),
            model: "default".to_owned(),
            effort: "default".to_owned(),
        }
    }

    fn default_kit() -> KitDraft {
        KitDraft {
            name: "Claude".to_owned(),
            description: "General-purpose.".to_owned(),
            fitted: as_it_comes(),
        }
    }

    /// A draft the form would accept, with every box filled including the
    /// two that need not be.
    fn filled() -> Draft {
        Draft {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![default_kit()],
            credential: "ghp-not-a-real-token".to_owned(),
            channel: ChannelDraft {
                credential: "xoxb-not-a-real-token".to_owned(),
                listen_credential: "xapp-not-a-real-token".to_owned(),
            },
            brief: String::new(),
            variables: vec![VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: "sk-test-not-a-real-key".to_owned(),
            }],
        }
    }

    fn without(change: fn(&mut Draft)) -> Draft {
        let mut draft = filled();
        change(&mut draft);
        draft
    }

    const NOTHING_HELD: &[String] = &[];

    fn amending() -> Filling {
        Filling::Amending("a-project".to_owned())
    }

    #[test]
    fn a_draft_with_every_answer_is_complete() {
        assert!(filled().is_complete(&Filling::Creating, NOTHING_HELD));
    }

    /// Every field is required, one at a time.
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
    #[test]
    fn amending_does_not_require_a_credential_and_creating_does() {
        let blank = without(|draft| draft.credential.clear());

        assert!(blank.is_complete(&amending(), NOTHING_HELD));
        assert!(!blank.is_complete(&Filling::Creating, NOTHING_HELD));
    }

    #[test]
    fn amending_still_requires_everything_a_project_is() {
        assert!(!without(|draft| draft.name.clear()).is_complete(&amending(), NOTHING_HELD));
        assert!(!without(|draft| draft.repository.clear()).is_complete(&amending(), NOTHING_HELD));
        assert!(!without(|draft| draft.kits.clear()).is_complete(&amending(), NOTHING_HELD));
    }

    #[test]
    fn only_creating_is_creating() {
        assert!(Filling::Creating.creating());
        assert!(!amending().creating());
    }

    /// A row for a variable the project does not hold needs a value,
    /// whichever caller this is; one it does hold may be left empty; and
    /// renaming a held one makes it new again.
    #[test]
    fn a_variable_needs_a_value_unless_the_project_holds_its_name() {
        let added = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "DATABASE_URL".to_owned(),
                value: String::new(),
            }];
        });
        assert!(!added.is_complete(&amending(), NOTHING_HELD));
        assert!(!added.is_complete(&Filling::Creating, NOTHING_HELD));

        let kept = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "STRIPE_API_KEY".to_owned(),
                value: String::new(),
            }];
        });
        assert!(kept.is_complete(&amending(), &["STRIPE_API_KEY".to_owned()]));

        let renamed = without(|draft| {
            draft.variables = vec![VariableDraft {
                name: "STRIPE_API_KEY_V2".to_owned(),
                value: String::new(),
            }];
        });
        assert!(!renamed.is_complete(&amending(), &["STRIPE_API_KEY".to_owned()]));
    }

    /// Amending never offers a channel, so whatever is left in those boxes
    /// must not make the control unavailable.
    #[test]
    fn amending_ignores_the_channel_boxes_entirely() {
        assert!(
            without(|draft| draft.channel.listen_credential.clear())
                .is_complete(&amending(), NOTHING_HELD)
        );
        assert!(
            without(|draft| draft.channel.credential.clear())
                .is_complete(&amending(), NOTHING_HELD)
        );
    }

    /// A project is created with a whole channel binding, and any part of
    /// one missing is the mistake worth catching on the screen — see
    /// `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`.
    #[test]
    fn a_channel_is_required_whole() {
        assert!(
            !without(|draft| draft.channel = ChannelDraft::default())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
        assert!(
            !without(|draft| draft.channel.credential.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
        assert!(
            !without(|draft| draft.channel.listen_credential.clear())
                .is_complete(&Filling::Creating, NOTHING_HELD)
        );
    }

    #[test]
    fn whitespace_does_not_count_as_an_answer() {
        let mut draft = filled();
        draft.name = "   ".to_owned();
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));

        let mut draft = filled();
        draft.credential = "\t ".to_owned();
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));

        let mut draft = filled();
        draft.channel.listen_credential = "  ".to_owned();
        assert!(!draft.is_complete(&Filling::Creating, NOTHING_HELD));
    }

    /// `docs/conventions.md` §4, for the three credentials a draft holds.
    #[test]
    fn a_draft_does_not_leak_any_credential_when_formatted() {
        let shown = format!("{:?}", filled());

        assert!(!shown.contains("ghp-not-a-real-token"), "{shown}");
        assert!(!shown.contains("xoxb-not-a-real-token"), "{shown}");
        assert!(!shown.contains("xapp-not-a-real-token"), "{shown}");
        assert!(!shown.contains("sk-test-not-a-real-key"), "{shown}");
        assert!(
            shown.contains("aviary"),
            "it should still say what it holds"
        );
        assert!(
            shown.contains("STRIPE_API_KEY"),
            "a name is not a credential"
        );
        assert!(
            shown.contains("ChannelDraft") && shown.contains("<redacted>"),
            "the binding is named, with its credentials redacted rather than dropped: {shown}"
        );
    }

    /// A kit row needs a name and a description, and two rows cannot share
    /// a name.
    #[test]
    fn a_kit_row_needs_a_name_and_a_description_and_names_are_distinct() {
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
        assert!(!twice.is_complete(&Filling::Creating, NOTHING_HELD));
        assert!(!distinct(&twice.kits));
        assert!(distinct(&filled().kits));
    }

    /// Every standing reads as something, and no two read alike.
    #[test]
    fn no_two_standings_read_alike_and_only_the_over_ones_are_over() {
        let every = [
            Standing::Working,
            Standing::Asked,
            Standing::Proposed,
            Standing::Paused,
            Standing::Idle,
            Standing::Failed {
                why: "it did not work".to_owned(),
            },
            Standing::Done,
            Standing::Discarded,
            Standing::Lost,
        ];
        let labels: std::collections::BTreeSet<&str> = every.iter().map(Standing::label).collect();
        assert_eq!(labels.len(), every.len());
        assert!(labels.iter().all(|label| !label.is_empty()));
        let over: Vec<&str> = every
            .iter()
            .filter(|standing| standing.is_over())
            .map(Standing::label)
            .collect();
        assert_eq!(over, ["done", "discarded", "lost"]);
    }

    /// A refusal an operator can fix must not read as a server fault.
    #[test]
    fn a_refusal_is_not_reported_as_a_fault() {
        let refused = Refusal::AgentInUse {
            agent: "claude".to_owned(),
            projects: vec!["aviary".to_owned(), "burrow".to_owned()],
        };
        assert_eq!(refused.status(), 409);
        assert_eq!(
            refused.to_string(),
            "claude is still used by aviary, burrow"
        );

        assert_eq!(
            Refusal::ProjectBusy {
                name: "aviary".to_owned(),
                working: 2,
            }
            .status(),
            409
        );
        assert_eq!(
            Refusal::Incomplete {
                field: "repository".to_owned(),
            }
            .status(),
            400
        );
        assert_eq!(
            Refusal::UnknownJob {
                id: "nope".to_owned()
            }
            .status(),
            404
        );
        assert_eq!(Refusal::Failed.status(), 500);
    }

    #[test]
    fn a_project_with_nothing_running_is_idle() {
        let project = super::Project {
            id: "an-identifier".to_owned(),
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman: as_it_comes(),
            kits: vec![default_kit()],
            platforms: Vec::new(),
            channels: Vec::new(),
            variables: Vec::new(),
            brief: String::new(),
            watched: Vec::new(),
            working: 0,
            jobs: 3,
        };
        assert!(project.idle());
        assert!(
            !super::Project {
                working: 1,
                ..project
            }
            .idle()
        );
    }
}
