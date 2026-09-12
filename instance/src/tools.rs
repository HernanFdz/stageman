//! The tools this instance serves to the agents it runs, and what each call
//! does.
//!
//! `docs/decisions/0034-tools-are-served-not-shipped.md` moves everything an
//! agent does outside its container from a program shipped in the image to a
//! tool served from here: MCP over HTTP, on a listener the world binds. The
//! world forwards each request whole, because everything the endpoint decides
//! is instance state — whether the credential names anyone, what its bearer
//! may be offered, and what each tool does.
//!
//! **What a bearer is offered is decided by the credential it presents**, and
//! that is the whole authorisation mechanism. A foreman may start jobs; a job
//! may not, and that is
//! `docs/decisions/0032-a-foreman-asks-the-instance-by-warrant.md`'s property
//! surviving the move to a per-turn credential.

use stageman_core::{ChannelConfig, Kit, ProjectId, State, Timestamp, Waiting};

use crate::Running;
use crate::foreman::kits_offered;
use crate::vocabulary::{AppEffect, RequestId, Speaker, Warranted};
use crate::{Effect, Emit as _};

/// The protocol version answered when a caller names none.
const PROTOCOL: &str = "2025-06-18";

/// What this instance calls itself to an agent.
///
/// It prefixes every tool name the model sees, so the tools themselves are
/// named for the act alone and never repeat it.
const SERVER: &str = "stageman";

/// What the tool a job ends its turn with is called.
///
/// Named for the moment rather than for an act, because there is no act:
/// nothing changes when it is called. What it does is leave a note that the
/// instance reads when the turn actually ends.
const STOPPING: &str = "stopping";

/// One tool, as an agent is told about it.
///
/// The schema is a value rather than a type because it is a wire document
/// whose shape belongs to the protocol, and modelling it here would buy a
/// second place for it to be wrong.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Tool {
    /// What the model calls it, before the server prefix.
    pub name: &'static str,
    /// What it is for, in the words the model reads.
    pub description: String,
    /// What it takes, as JSON Schema.
    #[serde(rename = "inputSchema")]
    pub schema: serde_json::Value,
}

/// What this bearer may be offered, given the kits its project runs jobs on.
///
/// **The kits are enumerated in the schema rather than described in prose**:
/// a schema can express a closed set where a command line cannot, so a
/// foreman picks from what exists instead of guessing and being corrected.
/// What each kit is *for* is said in the turn's own prompt. An empty set
/// omits the enumeration rather than emitting an empty one: a schema no value
/// can satisfy reads to a model as a broken tool, where a plain string
/// reaches the refusal below and says why.
#[must_use]
pub fn tools(warranted: &Warranted, kits: &[(String, String)]) -> Vec<Tool> {
    // Offered to everything this instance runs, because speaking is the one
    // thing a foreman and a job both do.
    let say = Tool {
        name: "say",
        description: "Say something to the people on this project's channel. \
                      This is the only way anything you write reaches a person: \
                      ordinary output is seen by nobody."
            .to_owned(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "What to say, in your own words.",
                },
            },
            "required": ["message"],
        }),
    };

    if !matches!(warranted.speaker, Speaker::Foreman(_)) {
        // A job may speak and may not start jobs, and it may say why it is
        // stopping, which a foreman may not: a foreman has no state anybody
        // acts on between turns, so a claim from one would be recorded
        // nowhere.
        return vec![say, stopping()];
    }

    let mut kit = serde_json::json!({
        "type": "string",
        "description": "Which kit runs this job: an agent, set a particular way. Pick \
                        by name from what this project offers; what each is for was \
                        said in your instructions.",
    });
    if !kits.is_empty()
        && let Some(fields) = kit.as_object_mut()
    {
        let names: Vec<&str> = kits.iter().map(|(name, _)| name.as_str()).collect();
        fields.insert("enum".to_owned(), serde_json::json!(names));
    }

    vec![
        say,
        Tool {
            name: "start_job",
            description: "Start a job on this project. A job is one agent working in an \
                      isolated container of its own, from kickoff to completion. It \
                      happens once and is not retried."
                .to_owned(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why this job should exist, in your own words. \
                                        A person reads this on the dashboard to \
                                        understand why you decided to start it.",
                    },
                    "instructions": {
                        "type": "string",
                        "description": "What the job's agent is to do. It begins from \
                                        this and never writes its own, so say enough \
                                        that somebody arriving with no other context \
                                        could act on it.",
                    },
                    "kit": kit,
                },
                "required": ["reason", "instructions", "kit"],
            }),
        },
    ]
}

/// The tool a job calls to say why it is stopping.
///
/// **The description is the whole of the feature.** Nothing forces an agent to
/// call this, and a turn that ends without it is recorded as having said
/// nothing — so what decides whether the two readings a person acts on
/// differently ever get recorded is how plainly this asks.
fn stopping() -> Tool {
    let spellings: Vec<&str> = Claim::ALL.iter().map(|claim| claim.spelling()).collect();

    Tool {
        name: STOPPING,
        description: "Call this immediately before you stop, every time, to say why you are \
                      stopping. It is the only way anybody learns whether you are waiting on \
                      them or offering them something to look at: a turn that ends without it \
                      is recorded as having stopped for reasons nobody knows."
            .to_owned(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "because": {
                    "type": "string",
                    "enum": spellings,
                    "description": "\"ready_for_review\" if you have done what was asked and \
                                    there is something for a person to look at. \
                                    \"waiting_for_an_answer\" if you need something from a \
                                    person before you can go on.",
                },
            },
            "required": ["because"],
        }),
    }
}

/// What one request on this endpoint means.
///
/// Deliberately covers the requests that are *not* ours as well: an observed
/// client sends at least one method that is in no specification, and a server
/// that answered such a thing with an error would fail a handshake over a
/// question it was free to ignore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// A handshake, carrying the protocol version to answer with.
    Greeting(String),
    /// Asking what tools exist.
    Listing,
    /// Asking to start a job.
    Starting(Starting),
    /// Asking to say something to a person.
    Saying(String),
    /// Saying why this turn is about to end, spelled as it arrived.
    Stopping(String),
    /// A tool this instance does not serve, by name.
    NoSuchTool(String),
    /// Something needing no answer at all.
    Notification,
    /// Something this instance does not implement and need not.
    Ignored,
}

/// What a job says about why it is about to stop.
///
/// **Two, and never the other three.** A job can say it asked something or
/// that it has something to show; it cannot claim to have failed, because a
/// failure is observed rather than claimed, and it cannot claim to be paused,
/// because that is a person's doing. See
/// `docs/decisions/0055-a-job-says-why-it-stopped.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// It needs an answer from a person before it can go on.
    Asked,
    /// It believes the work is done and there is something to look at.
    Proposed,
}

impl Claim {
    /// How this is spelled by an agent calling the tool: the agent's
    /// vocabulary rather than this project's, because what a model reads has
    /// to say what the state *means*.
    const fn spelling(self) -> &'static str {
        match self {
            Self::Asked => "waiting_for_an_answer",
            Self::Proposed => "ready_for_review",
        }
    }

    /// Every claim, so that the schema and the parser cannot disagree.
    const ALL: &'static [Self] = &[Self::Asked, Self::Proposed];

    /// What an agent's spelling means, if it means anything.
    fn spelled(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|claim| claim.spelling() == text.trim())
    }
}

impl From<Claim> for Waiting {
    fn from(claim: Claim) -> Self {
        match claim {
            Claim::Asked => Self::Asked,
            Claim::Proposed => Self::Proposed,
        }
    }
}

/// What a request to start a job carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Starting {
    /// Why the foreman decided to, in its own words.
    pub reason: String,
    /// What the job's agent is to do.
    pub instructions: String,
    /// Which kit runs it, by the name its project gives it.
    pub kit: String,
}

/// One request, as it arrives.
#[derive(serde::Deserialize)]
struct Incoming {
    /// Absent on a notification, which is what makes one answerable or not.
    #[serde(default)]
    id: Option<serde_json::Value>,
    /// What is being asked.
    method: String,
    /// What it was asked with.
    #[serde(default)]
    params: serde_json::Value,
}

/// What a request means, without performing any of it.
#[must_use]
pub fn decode(method: &str, params: &serde_json::Value) -> Call {
    if method.starts_with("notifications/") {
        return Call::Notification;
    }
    match method {
        "initialize" => Call::Greeting(
            params
                .get("protocolVersion")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(PROTOCOL)
                .to_owned(),
        ),
        "tools/list" => Call::Listing,
        "tools/call" => calling(params),
        _ => Call::Ignored,
    }
}

/// Which tool a call names, and what it was given.
///
/// A missing argument becomes an empty string rather than a refusal here,
/// because the refusal belongs where the job is created: the endpoint answers
/// one way for "you asked for something impossible" whatever made it
/// impossible.
fn calling(params: &serde_json::Value) -> Call {
    let name = params
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let arguments = params.get("arguments");
    let field = |key: &str| {
        arguments
            .and_then(|given| given.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    if name == "say" {
        return Call::Saying(field("message"));
    }
    if name == STOPPING {
        return Call::Stopping(field("because"));
    }
    if name != "start_job" {
        return Call::NoSuchTool(name.to_owned());
    }
    Call::Starting(Starting {
        reason: field("reason"),
        instructions: field("instructions"),
        kit: field("kit"),
    })
}

/// One successful answer, in the protocol's envelope.
///
/// Null rather than absent for a missing identifier: a caller that sent no
/// identifier is answering nothing, and the protocol spells that as an
/// explicit null.
fn answered(id: Option<serde_json::Value>, result: serde_json::Value) -> serde_json::Value {
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "jsonrpc".to_owned(),
        serde_json::Value::String("2.0".to_owned()),
    );
    envelope.insert("id".to_owned(), id.unwrap_or(serde_json::Value::Null));
    envelope.insert("result".to_owned(), result);
    serde_json::Value::Object(envelope)
}

/// A tool that ran and produced text.
fn succeeded(id: Option<serde_json::Value>, said: &str) -> serde_json::Value {
    answered(
        id,
        serde_json::json!({"content": [{"type": "text", "text": said}]}),
    )
}

/// A tool that could not do what was asked, told to the model rather than to
/// its machinery: a protocol error tells the agent's own machinery something
/// went wrong, and a failed tool result tells the model, which is the thing
/// able to pick a different kit and try again.
fn failed(id: Option<serde_json::Value>, why: &str) -> serde_json::Value {
    answered(
        id,
        serde_json::json!({
            "content": [{"type": "text", "text": why}],
            "isError": true,
        }),
    )
}

/// The kit a foreman named, if this project offers one under that name.
///
/// `None` for a name this project does not offer — a refusal rather than a
/// substitution, because silently running a different kit than the one asked
/// for is a wrong answer that looks like a right one.
#[must_use]
pub fn named_kit(state: &State, project: ProjectId, named: &str) -> Option<Kit> {
    let wanted = stageman_core::KitName::new(named).ok()?;
    state
        .projects
        .get(&project)?
        .kits
        .get(&wanted)
        .map(|offered| offered.kit.clone())
}

/// The HTTP status a request is answered with.
const OK: u16 = 200;
const ACCEPTED: u16 = 202;
const BAD_REQUEST: u16 = 400;
const FORBIDDEN: u16 = 403;

impl Running {
    /// Answers one request on the tools endpoint, if whoever asked is allowed
    /// to.
    ///
    /// A bad peer and an unknown credential get the same answer, deliberately
    /// and without detail: anything distinguishing "no such credential" from
    /// "not allowed" is something to guess against.
    pub fn tool_called(
        &mut self,
        id: RequestId,
        at: Timestamp,
        nearby: bool,
        bearer: Option<&str>,
        body: &serde_json::Value,
        effects: &mut Vec<Effect>,
    ) {
        if !nearby {
            tracing::warn!("the tools were reached from beyond this machine");
            self.answer(id, FORBIDDEN, None);
            return;
        }
        let Some(warranted) = bearer.and_then(|presented| self.warrants.get(presented).cloned())
        else {
            tracing::warn!("the tools were reached with a credential this instance does not hold");
            self.answer(id, FORBIDDEN, None);
            return;
        };
        let Ok(incoming) = serde_json::from_value::<Incoming>(body.clone()) else {
            self.answer(id, BAD_REQUEST, None);
            return;
        };

        match decode(&incoming.method, &incoming.params) {
            Call::Notification | Call::Ignored if incoming.id.is_none() => {
                self.answer(id, ACCEPTED, None);
            }
            Call::Notification | Call::Ignored => {
                self.answer(id, OK, Some(answered(incoming.id, serde_json::json!({}))));
            }
            Call::Greeting(protocol) => self.answer(
                id,
                OK,
                Some(answered(
                    incoming.id,
                    serde_json::json!({
                        "protocolVersion": protocol,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": SERVER, "version": crate::release::described()},
                    }),
                )),
            ),
            Call::Listing => {
                let kits = self
                    .project_of(&warranted)
                    .map(|project| kits_offered(&self.state, project))
                    .unwrap_or_default();
                let listed = tools(&warranted, &kits);
                self.answer(
                    id,
                    OK,
                    Some(answered(incoming.id, serde_json::json!({"tools": listed}))),
                );
            }
            Call::NoSuchTool(named) => {
                let why = format!("this instance serves no tool called {named:?}");
                self.answer(id, OK, Some(failed(incoming.id, &why)));
            }
            Call::Starting(starting) => {
                let result = self.starting_a_job(&warranted, &starting, at);
                self.answer(
                    id,
                    OK,
                    Some(result.map_or_else(
                        |why| failed(incoming.id.clone(), &why),
                        |said| succeeded(incoming.id.clone(), &said),
                    )),
                );
            }
            Call::Saying(message) => {
                if let Err(why) = self.saying(id, &warranted, &message, effects) {
                    self.answer(id, OK, Some(failed(incoming.id, &why)));
                } else {
                    // Answered when the platform has, in `posted`. The
                    // identifier the agent sent travels with the request in
                    // the held state until then.
                    self.asking.insert(id, incoming.id);
                }
            }
            Call::Stopping(because) => {
                let result = self.stopping_because(&warranted, &because);
                self.answer(
                    id,
                    OK,
                    Some(result.map_or_else(
                        |why| failed(incoming.id.clone(), &why),
                        |said| succeeded(incoming.id.clone(), said),
                    )),
                );
            }
        }
    }

    /// What the platform said about a message an agent asked to say.
    ///
    /// Said back verbatim rather than summarised: an explanation the agent
    /// inferred is one a person will act on, and it has no way to check it.
    /// The failure this guards against is the worst kind — an agent that
    /// believed it had spoken, stopped as it was told to, and nobody was ever
    /// told anything.
    pub fn posted(&mut self, request: RequestId, outcome: Result<(), String>) {
        let Some(asked) = self.asking.remove(&request) else {
            tracing::debug!(
                ?request,
                "the world answered a post nobody was waiting on; ignored"
            );
            return;
        };
        let body = match outcome {
            Ok(()) => succeeded(asked, "said"),
            Err(why) => {
                tracing::warn!(%why, "saying it failed");
                failed(asked, &format!("it could not be said: {why}"))
            }
        };
        self.answer(request, OK, Some(body));
    }

    /// Answers a request, once whatever this step changed is on the disk.
    fn answer(&mut self, id: RequestId, status: u16, body: Option<serde_json::Value>) {
        self.defer(AppEffect::ToolAnswered { id, status, body });
    }

    /// Which project a bearer belongs to.
    fn project_of(&self, warranted: &Warranted) -> Option<ProjectId> {
        match warranted.speaker {
            Speaker::Foreman(project) => Some(project),
            Speaker::Job(job) => self.state.project_of(job),
        }
    }

    /// Starts a job, or says why it could not be.
    ///
    /// A refusal comes back as a *tool result* marked as an error rather than
    /// as a protocol error, which is the distinction that matters to whoever
    /// reads it: the model is the thing able to pick a different kit.
    fn starting_a_job(
        &mut self,
        warranted: &Warranted,
        starting: &Starting,
        at: Timestamp,
    ) -> Result<String, String> {
        // Refused rather than merely unlisted: a tool nobody was offered can
        // still be called by name, so the check that matters is this one.
        let Speaker::Foreman(project) = warranted.speaker else {
            tracing::warn!("a job asked to start a job");
            return Err("this instance serves no tool called \"start_job\"".to_owned());
        };
        let Some(kit) = named_kit(&self.state, project, &starting.kit) else {
            let offered = kits_offered(&self.state, project)
                .into_iter()
                .map(|(name, description)| format!("{name} — {description}"))
                .collect::<Vec<_>>()
                .join("; ");
            tracing::warn!(%project, asked = %starting.kit, "no such kit on this project");
            return Err(format!(
                "this project offers no kit called {:?}. It offers: {offered}",
                starting.kit
            ));
        };
        match self.begin(project, kit, &starting.reason, &starting.instructions, at) {
            Ok(job) => Ok(format!("started job {job}")),
            Err(why) => {
                tracing::warn!(%project, %why, "the job could not be recorded");
                Err("the job could not be recorded".to_owned())
            }
        }
    }

    /// Says something on the channel, in whichever thread the caller belongs
    /// to.
    ///
    /// **The thread comes from the credential, never from the caller.** A
    /// job's thread is fixed for its life and a foreman's is different every
    /// turn, and an agent choosing its own would be an agent able to speak
    /// into somebody else's conversation.
    ///
    /// # Errors
    ///
    /// Fails, before anything is posted, when there is nothing to say or
    /// nowhere to say it.
    fn saying(
        &self,
        request: RequestId,
        warranted: &Warranted,
        message: &str,
        effects: &mut Vec<Effect>,
    ) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("nothing was said, so nothing was posted".to_owned());
        }
        let Some(thread) = warranted.thread.clone() else {
            // Not a refusal of the agent so much as of this instance: a
            // session declared with nowhere to speak should not have been
            // offered a tool that speaks.
            tracing::warn!("something spoke with no thread to speak in");
            return Err("there is no conversation to say this in".to_owned());
        };
        let speaking = self
            .project_of(warranted)
            .and_then(|project| self.state.projects.get(&project))
            .and_then(|watched| watched.channels.get(&thread.channel))
            .map(ChannelConfig::speaking);
        let Some(speaking) = speaking else {
            tracing::warn!("no channel is bound to say this on");
            return Err(
                "no channel is bound to this project, so there is nobody to say this to".to_owned(),
            );
        };
        effects.emit(AppEffect::Post {
            request,
            speaking: speaking.into(),
            thread,
            text: message.to_owned(),
        });
        Ok(())
    }

    /// Records what a job says about why it is about to stop.
    ///
    /// **Recording a claim is not a state change.** A job stays working until
    /// its agent actually stops, because the reply gate leans on that to keep
    /// two replies from resuming one container; what this writes is consulted
    /// when the turn ends and never before. See
    /// `docs/decisions/0055-a-job-says-why-it-stopped.md`.
    ///
    /// # Errors
    ///
    /// Fails for a foreman, which has no state between turns for a claim to
    /// be recorded against; for a spelling that is not a claim; and for a job
    /// with no turn running, which is what a claim arriving after its own
    /// turn was stopped looks like.
    fn stopping_because(
        &mut self,
        warranted: &Warranted,
        because: &str,
    ) -> Result<&'static str, String> {
        let Speaker::Job(job) = warranted.speaker else {
            tracing::warn!("a foreman said why it was stopping");
            return Err(format!("this instance serves no tool called {STOPPING:?}"));
        };
        let Some(claim) = Claim::spelled(because) else {
            let offered = Claim::ALL
                .iter()
                .map(|claim| format!("{:?}", claim.spelling()))
                .collect::<Vec<_>>()
                .join(" or ");
            return Err(format!(
                "{because:?} is not one of the reasons this takes. Use {offered}."
            ));
        };
        if let Some(turn) = self.turns.get_mut(&Speaker::Job(job)) {
            turn.claimed = Some(claim.into());
            Ok("noted")
        } else {
            tracing::debug!(%job, "said why it was stopping, with no turn running");
            Err("this job has no turn running, so there is nothing to say this about".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Call, Claim, Starting, Tool, calling, decode, named_kit, tools};
    use crate::vocabulary::{Speaker, Warranted};
    use stageman_core::{
        Agent, AgentConfig, Kit, KitConfig, KitName, Project, ProjectId, Secret, State, Thread,
        Uuid,
    };
    use std::collections::BTreeMap;

    fn a_project() -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(1))
    }

    fn a_foreman() -> Warranted {
        Warranted {
            speaker: Speaker::Foreman(a_project()),
            thread: Some(Thread {
                channel: stageman_core::Channel::Slack,
                id: "1788000000.000001".to_owned(),
            }),
        }
    }

    fn a_job() -> Warranted {
        Warranted {
            speaker: Speaker::Job(stageman_core::JobId::from_uuid(Uuid::from_u128(7))),
            thread: None,
        }
    }

    fn one_kit() -> Vec<(String, String)> {
        vec![("Claude".to_owned(), "General-purpose.".to_owned())]
    }

    fn names(listed: &[Tool]) -> Vec<&'static str> {
        listed.iter().map(|tool| tool.name).collect()
    }

    #[test]
    fn a_greeting_echoes_the_version_it_was_given() {
        assert_eq!(
            decode(
                "initialize",
                &serde_json::json!({"protocolVersion": "2024-11-05"})
            ),
            Call::Greeting("2024-11-05".to_owned())
        );
        assert_eq!(
            decode("initialize", &serde_json::json!({})),
            Call::Greeting(super::PROTOCOL.to_owned()),
            "a caller naming none is answered with a version rather than refused"
        );
    }

    #[test]
    fn a_notification_wants_no_answer_and_an_unknown_method_is_ignored() {
        assert_eq!(
            decode("notifications/initialized", &serde_json::Value::Null),
            Call::Notification
        );
        assert_eq!(
            decode("resources/list", &serde_json::Value::Null),
            Call::Ignored
        );
        assert_eq!(
            decode("tools/list", &serde_json::Value::Null),
            Call::Listing
        );
    }

    #[test]
    fn a_call_is_read_as_its_arguments_or_as_no_such_tool() {
        assert_eq!(
            calling(&serde_json::json!({
                "name": "start_job",
                "arguments": {"reason": "why", "instructions": "what", "kit": "Claude"},
            })),
            Call::Starting(Starting {
                reason: "why".to_owned(),
                instructions: "what".to_owned(),
                kit: "Claude".to_owned(),
            })
        );
        assert_eq!(
            calling(&serde_json::json!({"name": "say", "arguments": {"message": "hello"}})),
            Call::Saying("hello".to_owned())
        );
        assert_eq!(
            calling(
                &serde_json::json!({"name": "stopping", "arguments": {"because": "ready_for_review"}})
            ),
            Call::Stopping("ready_for_review".to_owned())
        );
        assert_eq!(
            calling(&serde_json::json!({"name": "delete_everything"})),
            Call::NoSuchTool("delete_everything".to_owned())
        );
        assert_eq!(
            calling(&serde_json::json!({"name": "start_job"})),
            Call::Starting(Starting {
                reason: String::new(),
                instructions: String::new(),
                kit: String::new(),
            }),
            "a missing argument is an empty one, refused where the job is created"
        );
    }

    /// A foreman is offered the tool that starts jobs and a job is not; a
    /// job is offered the tool that says why it stopped and a foreman is not.
    #[test]
    fn what_each_speaker_is_offered() {
        assert_eq!(
            names(&tools(&a_foreman(), &one_kit())),
            ["say", "start_job"]
        );
        assert_eq!(names(&tools(&a_job(), &one_kit())), ["say", "stopping"]);
    }

    /// The kits a foreman may choose are enumerated in the schema, and an
    /// empty set omits the enumeration rather than emitting an empty one.
    #[test]
    fn the_kits_a_foreman_may_choose_are_in_the_schema() {
        let offered = tools(&a_foreman(), &one_kit());
        let start = offered
            .iter()
            .find(|tool| tool.name == "start_job")
            .expect("offered");
        assert_eq!(
            start.schema["properties"]["kit"]["enum"],
            serde_json::json!(["Claude"])
        );

        let none = tools(&a_foreman(), &[]);
        let start = none
            .iter()
            .find(|tool| tool.name == "start_job")
            .expect("offered");
        assert!(start.schema["properties"]["kit"].get("enum").is_none());
    }

    #[test]
    fn every_claim_has_a_spelling_of_its_own_and_the_tool_offers_them() {
        for claim in Claim::ALL {
            assert_eq!(Claim::spelled(claim.spelling()), Some(*claim));
        }
        assert_eq!(Claim::spelled(" ready_for_review "), Some(Claim::Proposed));
        assert_eq!(Claim::spelled("failed"), None);
        let stopping = super::stopping();
        assert_eq!(
            stopping.schema["properties"]["because"]["enum"],
            serde_json::json!(["waiting_for_an_answer", "ready_for_review"])
        );
    }

    /// A kit must be named and offered, or it is refused rather than mended.
    #[test]
    fn a_kit_must_be_named_and_offered_or_it_is_refused() {
        let mut state = State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("agent-token".to_owned()),
                },
            )]),
            ..State::default()
        };
        state.projects.insert(
            a_project(),
            Project {
                name: "example".to_owned(),
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
            },
        );

        assert_eq!(
            named_kit(&state, a_project(), "Claude"),
            Some(Kit::defaults(Agent::Claude))
        );
        assert_eq!(
            named_kit(&state, a_project(), " Claude "),
            Some(Kit::defaults(Agent::Claude)),
            "a name is trimmed on the way in"
        );
        assert_eq!(named_kit(&state, a_project(), "Other"), None);
        assert_eq!(named_kit(&state, a_project(), ""), None);
        assert_eq!(
            named_kit(&state, ProjectId::from_uuid(Uuid::from_u128(404)), "Claude"),
            None
        );
    }
}
