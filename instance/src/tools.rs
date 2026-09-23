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

use stageman_core::{Kit, Place, ProjectId, Room, State, Thread, Timestamp, Waiting};

use crate::channel::Origin;
use crate::jobs::Commission;

use crate::Running;
use crate::foreman::kits_offered;
use stageman_vocabulary::{Answer, Arrival, Bytes, RequestId};

use crate::vocabulary::{Speaker, Warranted};
use crate::{Called, Effect};

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

/// What the tools that watch and stop watching a room are called.
///
/// Neither takes an argument: the room is the one the turn was asked in,
/// which comes from the credential and never from the caller — see
/// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
const WATCH_ROOM: &str = "watch_room";
const STOP_WATCHING: &str = "stop_watching";

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
        description: say_description(warranted.speaker).to_owned(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "What to say, in Markdown, in your own words.",
                },
                "to": {
                    "type": "string",
                    "description": "The message to reply under, exactly as it was shown to \
                                    you. Leave it out to post at the root of your own room.",
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

    // Both act on the room the turn was asked in, so neither takes a room:
    // a foreman that could name one could be talked into watching a room it
    // was never asked in.
    let watch = Tool {
        name: WATCH_ROOM,
        description: "Watch the room this message was said in. From then on everything \
                      another app posts there — an issue filed, an alert fired, a pull \
                      request opened — reaches you as a signal to judge; people are still \
                      heard only through a mention. Call it when a person asks you, in a \
                      room, to watch that room."
            .to_owned(),
        schema: serde_json::json!({"type": "object", "properties": {}}),
    };
    let stop_watching = Tool {
        name: STOP_WATCHING,
        description: "Stop watching the room this message was said in: nothing another app \
                      posts there reaches you afterwards. Call it when a person asks you, \
                      in a room, to stop watching that room."
            .to_owned(),
        schema: serde_json::json!({"type": "object", "properties": {}}),
    };

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
                    "title": {
                        "type": "string",
                        "description": "A few words naming the job, as a person would read \
                                        them in a sidebar: the room it reports in is named \
                                        after them.",
                    },
                },
                "required": ["reason", "instructions", "kit", "title"],
            }),
        },
        watch,
        stop_watching,
    ]
}

/// What the tool that speaks is for, which differs by who holds it: a job's
/// narration already reaches its room, per
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`,
/// so for a job the tool is for the thread a person asked in; a foreman's
/// reaches a room of its own, where the person who asked is not, so for a
/// foreman the tool is how the person is answered.
const fn say_description(speaker: Speaker) -> &'static str {
    match speaker {
        Speaker::Foreman(_) => {
            "Say something to the people on this project's channel, in Markdown: it \
             is rendered, so headings, lists, code, tables and links all show. It \
             posts under the message you name with `to`, which is how a person is \
             answered where they asked; without one it posts at the root of your \
             own room, where your ordinary output already goes and the person who \
             asked is not."
        }
        Speaker::Job(_) => {
            "Say something to the people in this job's room, in Markdown: it is \
             rendered, so headings, lists, code, tables and links all show. It posts \
             at the root of your room, or under a message when you name it with \
             `to`, as each message is shown to you. Everything you write as ordinary \
             output is posted at the root as well, so use this to answer in a \
             person's thread, or when you need an answer from a person."
        }
    }
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
                      stopping and which pull requests you opened. It is the only way anybody \
                      learns whether you are waiting on them or offering them something to \
                      look at: a turn that ends without it is recorded as having stopped for \
                      reasons nobody knows."
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
                "pull_requests": {
                    "type": "array",
                    "items": { "type": "integer", "minimum": 1 },
                    "description": "The numbers of the pull requests you opened on this \
                                    project's repository, if any: #12 is 12. Give every one \
                                    you opened, whichever reason you are stopping for — a \
                                    draft opened before a question is still something to \
                                    look at.",
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
    /// Asking to say something to a person: what, and under which message,
    /// if any.
    Saying {
        /// What to say.
        message: String,
        /// The message to reply under, as the agent was shown it.
        to: Option<String>,
    },
    /// Saying why this turn is about to end, and which pull requests it
    /// opened.
    Stopping(Stopping),
    /// Asking to watch, or to stop watching, the room this turn was asked in.
    Watching(Watching),
    /// A tool this instance does not serve, by name.
    NoSuchTool(String),
    /// Something needing no answer at all.
    Notification,
    /// Something this instance does not implement and need not.
    Ignored,
}

/// Which way a room's watching is being changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watching {
    /// From now on, another app's message there is a signal.
    Start,
    /// From now on, it is nothing.
    Stop,
}

impl Watching {
    /// What the tool asking for this is called.
    const fn tool(self) -> &'static str {
        match self {
            Self::Start => WATCH_ROOM,
            Self::Stop => STOP_WATCHING,
        }
    }
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

/// What a call saying why a turn is about to end carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopping {
    /// Why, spelled as it arrived.
    pub because: String,
    /// The pull requests it opened, by number — or why the list it gave
    /// cannot be read, which the call is answered with rather than half of
    /// the list being kept.
    pub pull_requests: Result<Vec<u64>, String>,
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
    /// A few words naming it, which its room is named after.
    pub title: String,
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
        let to = arguments
            .and_then(|given| given.get("to"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|named| !named.is_empty())
            .map(str::to_owned);
        return Call::Saying {
            message: field("message"),
            to,
        };
    }
    if name == STOPPING {
        let pull_requests = arguments
            .and_then(|given| given.get("pull_requests"))
            .map_or_else(|| Ok(Vec::new()), pull_requests_of);
        return Call::Stopping(Stopping {
            because: field("because"),
            pull_requests,
        });
    }
    if name == WATCH_ROOM {
        return Call::Watching(Watching::Start);
    }
    if name == STOP_WATCHING {
        return Call::Watching(Watching::Stop);
    }
    if name != "start_job" {
        return Call::NoSuchTool(name.to_owned());
    }
    Call::Starting(Starting {
        reason: field("reason"),
        instructions: field("instructions"),
        kit: field("kit"),
        title: field("title"),
    })
}

/// The pull requests a stopping call names, as positive integers, or why
/// the list cannot be read: the schema says integers from one, and a call
/// that says otherwise is told so rather than having half its list kept.
fn pull_requests_of(given: &serde_json::Value) -> Result<Vec<u64>, String> {
    let Some(listed) = given.as_array() else {
        return Err(format!(
            "pull_requests must be a list of numbers, and {given} is not one"
        ));
    };
    listed
        .iter()
        .map(|item| {
            item.as_u64().filter(|number| *number > 0).ok_or_else(|| {
                format!(
                    "{item} is not a positive integer; a pull request is named by its \
                     number, so #12 is 12"
                )
            })
        })
        .collect()
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
const NO_CONTENT: u16 = 204;
const BAD_REQUEST: u16 = 400;
const FORBIDDEN: u16 = 403;
const NOT_FOUND: u16 = 404;
const METHOD_NOT_ALLOWED: u16 = 405;

/// The one path the tools are served on.
const PATH: &str = "/mcp";

/// How much of a call's body is read before giving up on it.
///
/// A call is a small JSON object, and whoever sends one is somebody else's
/// code running in a container: bounded because a body held whole is held in
/// memory, and generous because the largest of them carries a message a
/// person wrote.
const LIMIT: usize = 1024 * 1024;

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
            Call::Saying { message, to } => {
                if let Err(why) = self.saying(id, &warranted, &message, to.as_deref()) {
                    self.answer(id, OK, Some(failed(incoming.id, &why)));
                } else {
                    // Answered when the platform has, in `posted`. The
                    // identifier the agent sent travels with the request in
                    // the held state until then.
                    self.asking.insert(id, incoming.id);
                }
            }
            Call::Stopping(stopping) => {
                let result = self.stopping_because(&warranted, &stopping);
                self.answer(
                    id,
                    OK,
                    Some(result.map_or_else(
                        |why| failed(incoming.id.clone(), &why),
                        |said| succeeded(incoming.id.clone(), said),
                    )),
                );
            }
            Call::Watching(which) => {
                let result = self.watching(&warranted, which);
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
    pub fn posted(&mut self, request: RequestId, outcome: Result<String, String>) {
        let Some(asked) = self.asking.remove(&request) else {
            tracing::debug!(
                ?request,
                "the world answered a post nobody was waiting on; ignored"
            );
            return;
        };
        let body = match outcome {
            Ok(reference) => succeeded(asked, &reference),
            Err(why) => {
                tracing::warn!(%why, "saying it failed");
                failed(asked, &format!("it could not be said: {why}"))
            }
        };
        self.answer(request, OK, Some(body));
    }

    /// Answers a request, once whatever this step changed is on the disk.
    fn answer(&mut self, id: RequestId, status: u16, body: Option<serde_json::Value>) {
        let (headers, body) = body.map_or_else(
            || (std::collections::BTreeMap::new(), Bytes::new(Vec::new())),
            |body| {
                (
                    [("content-type".to_owned(), "application/json".to_owned())].into(),
                    Bytes::new(serde_json::to_vec(&body).unwrap_or_default()),
                )
            },
        );
        self.defer(Effect::Answer {
            id,
            answer: Answer::Respond {
                status,
                headers,
                body,
            },
        });
    }

    /// A call arrived on the listener the tools are served on.
    ///
    /// Nothing is decided from the head but the shape of the call: what it
    /// presented and where it came from are kept and judged once the body is
    /// there, so that a refusal for a credential nobody holds and one for a
    /// body that is not a call are refused by the same code.
    pub fn called(&mut self, id: RequestId, request: &Arrival, effects: &mut Vec<Effect>) {
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", PATH) => {
                self.calls.insert(
                    id,
                    Called {
                        nearby: nearby(&request.peer),
                        bearer: presented(request),
                        at: stamped(request.at),
                    },
                );
                effects.push(Effect::Answer {
                    id,
                    answer: Answer::Read { limit: LIMIT },
                });
            }
            // Every tool answers within its own call, so there is nothing
            // this instance would ever push: the stream a client may offer
            // to open is declined. Measured — a client offered one, was
            // refused, and completed a tool call regardless.
            ("GET", PATH) => Self::refuse(id, METHOD_NOT_ALLOWED, effects),
            // Nothing is held per connection, because the credential decides
            // everything and is presented on each request, so a client
            // hanging up has nothing to release and is told so.
            ("DELETE", PATH) => Self::refuse(id, NO_CONTENT, effects),
            _ => Self::refuse(id, NOT_FOUND, effects),
        }
    }

    /// The body of a call arrived, or could not be read.
    pub fn read(
        &mut self,
        id: RequestId,
        outcome: Result<Bytes, String>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(called) = self.calls.remove(&id) else {
            tracing::warn!("a body arrived for a request this instance was not reading; ignored");
            return;
        };
        let read = match outcome {
            Ok(read) => read,
            Err(why) => {
                tracing::warn!(%why, "a call's body could not be read");
                Self::refuse(id, BAD_REQUEST, effects);
                return;
            }
        };
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(read.as_slice()) else {
            Self::refuse(id, BAD_REQUEST, effects);
            return;
        };
        self.tool_called(
            id,
            called.at,
            called.nearby,
            called.bearer.as_deref(),
            &body,
        );
    }

    /// Answers now, without waiting on a write, because nothing changed.
    fn refuse(id: RequestId, status: u16, effects: &mut Vec<Effect>) {
        effects.push(Effect::Answer {
            id,
            answer: Answer::Respond {
                status,
                headers: std::collections::BTreeMap::new(),
                body: Bytes::new(Vec::new()),
            },
        });
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
        // Where the job comes from: the thread the foreman is answering in,
        // and who it is answering. A foreman always answers in a thread, so
        // a place without one has nowhere to say where the job is.
        let origin = warranted.place.clone().and_then(|place| {
            let thread = place.thread?;
            Some(Origin {
                thread: Thread {
                    channel: place.room.channel,
                    room: place.room.id,
                    id: thread,
                },
                user: warranted.from.clone(),
            })
        });
        let commission = Commission {
            kit,
            reason: &starting.reason,
            work: &starting.instructions,
            title: &starting.title,
        };
        match self.begin(project, commission, origin, at) {
            Ok(job) => Ok(format!("started job {job}")),
            Err(why) => {
                tracing::warn!(%project, %why, "the job could not be started");
                // Said as it is, so that the foreman can report what was
                // printed rather than what it thinks it meant.
                Err(format!("the job could not be started: {why}"))
            }
        }
    }

    /// Says something on the channel, at whichever place the caller belongs
    /// to.
    ///
    /// **The place comes from the credential, never from the caller.** A
    /// job's is its room, or the thread in it it was asked in, and a
    /// foreman's is different every turn; an agent choosing its own would be
    /// an agent able to speak into somebody else's conversation.
    ///
    /// # Errors
    ///
    /// Fails, before anything is posted, when there is nothing to say or
    /// nowhere to say it.
    fn saying(
        &mut self,
        request: RequestId,
        warranted: &Warranted,
        message: &str,
        to: Option<&str>,
    ) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("nothing was said, so nothing was posted".to_owned());
        }
        // Where the speaker's own words go: the root of the room it owns.
        let own = match warranted.speaker {
            Speaker::Job(job) => crate::turns::speaking_for(&self.state, job),
            Speaker::Foreman(project) => self.state.projects.get(&project).and_then(|watched| {
                let room = watched.foreman_room.clone()?;
                let bound = watched.channels.get(&room.channel)?;
                Some((bound.speaking(), Place::root(room)))
            }),
        };
        let Some((speaking, own_root)) = own else {
            tracing::warn!("something spoke with no room of its own");
            return Err("you have no room of your own to post in".to_owned());
        };
        let channel = own_root.room.channel;
        let place = match to {
            None => own_root,
            Some(named) => {
                let Some((room, under)) = stageman_channel::referenced(channel, named) else {
                    return Err(format!(
                        "{named:?} is not a message as it was shown to you, which reads \
                         <room>/<message>"
                    ));
                };
                // A job speaks in its own room and nowhere else, which is
                // the property the warrant gave the tool before it took a
                // target; a foreman may name any room, and the platform
                // refuses one the app is not in.
                if matches!(warranted.speaker, Speaker::Job(_)) && room != own_root.room.id {
                    return Err("a job may reply only in its own room".to_owned());
                }
                Place {
                    room: Room { channel, id: room },
                    thread: Some(under),
                }
            }
        };
        self.post_for(request, &speaking, &place, message);
        Ok(())
    }

    /// Watches, or stops watching, the room the caller's turn was asked in.
    ///
    /// **The room comes from the credential, never from the caller**, for
    /// the reason a post's place does. Recorded on the project, so it is on
    /// the disk with the next write and survives a restart. Idempotent in
    /// both directions and said so, because an interrupted foreman is told
    /// to check how things stand rather than to assume, and "already
    /// watching" is that answer.
    ///
    /// # Errors
    ///
    /// Fails for a job, which is offered neither tool and may not watch
    /// anything; and for a turn with no place, which has no room to watch.
    fn watching(&mut self, warranted: &Warranted, which: Watching) -> Result<&'static str, String> {
        let Speaker::Foreman(project) = warranted.speaker else {
            tracing::warn!("a job asked to watch a room");
            return Err(format!(
                "this instance serves no tool called {:?}",
                which.tool()
            ));
        };
        let Some(place) = warranted.place.as_ref() else {
            tracing::warn!(%project, "a foreman asked to watch a room with no room to watch");
            return Err(
                "this turn was not asked in a room, so there is nothing to watch".to_owned(),
            );
        };
        let Some(watched) = self.state.projects.get_mut(&project) else {
            return Err(format!("no project {project} in this instance"));
        };
        let room = place.room.clone();
        let said = match which {
            Watching::Start => {
                if watched.watched.insert(room) {
                    self.dirty = true;
                    "watching this room: from now on everything another app posts here reaches \
                     you as a signal"
                } else {
                    "already watching this room"
                }
            }
            Watching::Stop => {
                if watched.watched.remove(&room) {
                    self.dirty = true;
                    "no longer watching this room"
                } else {
                    "this room was not being watched"
                }
            }
        };
        Ok(said)
    }

    /// Records what a job says about why it is about to stop, and which
    /// pull requests it opened.
    ///
    /// **Recording a claim is not a state change.** A job stays working until
    /// its agent actually stops, because the reply gate leans on that to keep
    /// two replies from resuming one container; what this writes is consulted
    /// when the turn ends and never before. See
    /// `docs/decisions/0055-a-job-says-why-it-stopped.md`. The pull requests
    /// are noted against the turn the same way, and kept on the job when it
    /// ends whichever way it ends, per
    /// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
    ///
    /// # Errors
    ///
    /// Fails for a foreman, which has no state between turns for a claim to
    /// be recorded against; for a spelling that is not a claim; for a list
    /// of pull requests that is not one of positive integers; and for a job
    /// with no turn running, which is what a claim arriving after its own
    /// turn was stopped looks like.
    fn stopping_because(
        &mut self,
        warranted: &Warranted,
        stopping: &Stopping,
    ) -> Result<&'static str, String> {
        let Speaker::Job(job) = warranted.speaker else {
            tracing::warn!("a foreman said why it was stopping");
            return Err(format!("this instance serves no tool called {STOPPING:?}"));
        };
        let because = stopping.because.as_str();
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
        let opened = stopping.pull_requests.clone()?;
        if let Some(turn) = self.turns.get_mut(&Speaker::Job(job)) {
            turn.claimed = Some(claim.into());
            turn.pull_requests.extend(opened);
            Ok("noted")
        } else {
            tracing::debug!(%job, "said why it was stopping, with no turn running");
            Err("this job has no turn running, so there is nothing to say this about".to_owned())
        }
    }
}

/// Whether a request came from somewhere allowed to ask.
///
/// Not a security boundary — the warrant is that — but it costs three lines
/// and removes an entire class of caller. The endpoint takes every interface
/// because that is the only address a container can reach on every platform,
/// and nothing routed from beyond this machine has any business here.
///
/// Loopback for a request from the host itself, and private ranges for one
/// from a container: every container network is private by construction.
fn nearby(peer: &str) -> bool {
    use std::net::IpAddr;

    // A peer is an address and a port, and the port is not the question.
    let Some((address, _)) = peer.rsplit_once(':') else {
        return false;
    };
    let Ok(address) = address.trim_matches(['[', ']']).parse::<IpAddr>() else {
        return false;
    };
    match address {
        IpAddr::V4(address) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        // A container reaching a host over IPv6 arrives from a unique-local
        // address, which is the v6 spelling of private.
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                // A v4 address arriving mapped into v6, which is what a dual
                // stack listener reports for an ordinary v4 peer.
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback() || mapped.is_private())
        }
    }
}

/// The credential a request presented, if it presented one.
///
/// Bearer only, and compared nowhere here: this reads the header and the
/// rest decides whether it names anything, so that a malformed header and an
/// unknown credential reach the same refusal by the same path.
fn presented(request: &Arrival) -> Option<String> {
    request
        .headers
        .get("authorization")?
        .strip_prefix("Bearer ")
        .map(|presented| presented.trim().to_owned())
}

/// When a request arrived, as the domain spells a time.
fn stamped(millis: u64) -> Timestamp {
    let Ok(millis) = i64::try_from(millis) else {
        return Timestamp::UNIX_EPOCH;
    };
    // A stamp outside what a timestamp can hold is a clock nobody can act
    // on, and the epoch is the one value that reads as obviously wrong.
    Timestamp::from_millisecond(millis).unwrap_or(Timestamp::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::{nearby, presented, say_description, stamped};
    use stageman_core::{JobId, Timestamp};
    use stageman_vocabulary::Arrival;

    /// A request head, as the world hands one over.
    fn asking(peer: &str, headers: &[(&str, &str)]) -> Arrival {
        Arrival {
            method: "POST".to_owned(),
            path: "/mcp".to_owned(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            peer: peer.to_owned(),
            at: 1_757_000_000_000,
        }
    }

    /// Anything from this machine or its containers may ask; nothing else
    /// may.
    #[test]
    fn only_something_on_this_machine_may_ask() {
        for near in [
            "127.0.0.1:52104",
            "[::1]:52104",
            // The bridge gateway a container arrives from on Linux, and the
            // subnets the common runtimes hand out.
            "172.17.0.1:52104",
            "172.18.0.2:52104",
            "10.88.0.3:52104",
            "192.168.65.1:52104",
            "[fd00::1]:52104",
            "[::ffff:172.17.0.1]:52104",
        ] {
            assert!(nearby(near), "{near} is a container or this host");
        }

        for far in [
            "8.8.8.8:52104",
            "203.0.113.7:52104",
            "[2606:4700::1111]:443",
        ] {
            assert!(!nearby(far), "{far} came from beyond this machine");
        }

        // A peer this cannot read is not one it may trust.
        assert!(!nearby(""), "nothing is not an address");
        assert!(!nearby("not-an-address:1"), "nor is that");
    }

    /// The credential is read from the one header and the one scheme.
    #[test]
    fn only_a_bearer_credential_is_presented() {
        assert_eq!(presented(&asking("127.0.0.1:1", &[])), None);
        assert_eq!(
            presented(&asking(
                "127.0.0.1:1",
                &[("authorization", "Bearer  not-a-real-credential ")]
            ))
            .as_deref(),
            Some("not-a-real-credential")
        );
        assert_eq!(
            presented(&asking(
                "127.0.0.1:1",
                &[("authorization", "Basic bm90LWEtcmVhbC1jcmVkZW50aWFs")]
            )),
            None,
            "another scheme presents nothing"
        );
    }

    /// A stamp is the time it names, and one nothing can name is the epoch.
    #[test]
    fn a_stamp_is_the_time_the_world_said() {
        assert_eq!(
            stamped(1_757_000_000_000).to_string(),
            "2025-09-04T15:33:20Z"
        );
        assert_eq!(stamped(0), Timestamp::UNIX_EPOCH);
        assert_eq!(
            stamped(u64::MAX),
            Timestamp::UNIX_EPOCH,
            "a clock nobody can act on reads as obviously wrong"
        );
    }

    use super::{
        Call, Claim, Starting, Stopping, Tool, Watching, calling, decode, named_kit, tools,
    };
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
            place: Some(stageman_core::Place::from(Thread {
                channel: stageman_core::Channel::Slack,
                room: "C0123456789".to_owned(),
                id: "1788000000.000001".to_owned(),
            })),
            from: Some("U0HUMAN".to_owned()),
        }
    }

    fn a_job() -> Warranted {
        Warranted {
            speaker: Speaker::Job(stageman_core::JobId::from_uuid(Uuid::from_u128(7))),
            place: None,
            from: None,
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
                "arguments": {"reason": "why", "instructions": "what", "kit": "Claude", "title": "a title"},
            })),
            Call::Starting(Starting {
                reason: "why".to_owned(),
                instructions: "what".to_owned(),
                kit: "Claude".to_owned(),
                title: "a title".to_owned(),
            })
        );
        assert_eq!(
            calling(&serde_json::json!({"name": "say", "arguments": {"message": "hello"}})),
            Call::Saying {
                message: "hello".to_owned(),
                to: None,
            }
        );
        assert_eq!(
            calling(&serde_json::json!({
                "name": "say",
                "arguments": {"message": "hello", "to": " C0123/1788000000.000100 "},
            })),
            Call::Saying {
                message: "hello".to_owned(),
                to: Some("C0123/1788000000.000100".to_owned()),
            },
            "named as shown, whitespace aside"
        );
        assert_eq!(
            calling(
                &serde_json::json!({"name": "say", "arguments": {"message": "hello", "to": ""}})
            ),
            Call::Saying {
                message: "hello".to_owned(),
                to: None,
            },
            "nothing named is nothing"
        );
        assert_eq!(
            calling(
                &serde_json::json!({"name": "stopping", "arguments": {"because": "ready_for_review"}})
            ),
            Call::Stopping(Stopping {
                because: "ready_for_review".to_owned(),
                pull_requests: Ok(Vec::new()),
            })
        );
        assert_eq!(
            calling(&serde_json::json!({"name": "watch_room"})),
            Call::Watching(Watching::Start)
        );
        assert_eq!(
            calling(&serde_json::json!({"name": "stop_watching", "arguments": {}})),
            Call::Watching(Watching::Stop)
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
                title: String::new(),
            }),
            "a missing argument is an empty one, refused where the job is created"
        );
    }

    /// A stopping call names the pull requests it opened as positive
    /// integers, in the order given, or is told exactly why its list cannot
    /// be read — so that half a list is never kept.
    #[test]
    fn a_stopping_call_names_its_pull_requests_or_is_told_why_not() {
        assert_eq!(
            calling(&serde_json::json!({
                "name": "stopping",
                "arguments": {"because": "waiting_for_an_answer", "pull_requests": [12, 3]}
            })),
            Call::Stopping(Stopping {
                because: "waiting_for_an_answer".to_owned(),
                pull_requests: Ok(vec![12, 3]),
            }),
            "the numbers as given; the job sorts and unites them"
        );
        for (given, why) in [
            (
                serde_json::json!({"pull_requests": [12, 0]}),
                "0 is not a positive integer",
            ),
            (
                serde_json::json!({"pull_requests": ["#12"]}),
                "\"#12\" is not a positive integer",
            ),
            (serde_json::json!({"pull_requests": 12}), "12 is not one"),
        ] {
            let mut arguments = given;
            arguments["because"] = serde_json::json!("ready_for_review");
            let Call::Stopping(stopping) =
                calling(&serde_json::json!({"name": "stopping", "arguments": arguments}))
            else {
                panic!("a stopping call");
            };
            let refused = stopping.pull_requests.expect_err("refused");
            assert!(refused.contains(why), "{refused}");
        }
    }

    /// A foreman is offered the tools that start jobs and watch rooms and a
    /// job is not; a job is offered the tool that says why it stopped and a
    /// foreman is not.
    #[test]
    fn what_each_speaker_is_offered() {
        assert_eq!(
            names(&tools(&a_foreman(), &one_kit())),
            ["say", "start_job", "watch_room", "stop_watching"]
        );
        assert_eq!(names(&tools(&a_job(), &one_kit())), ["say", "stopping"]);
    }

    /// Neither tool that watches takes a room, because the room is the
    /// credential's to say.
    #[test]
    fn the_tools_that_watch_take_no_room() {
        for tool in tools(&a_foreman(), &one_kit()) {
            if tool.name == "watch_room" || tool.name == "stop_watching" {
                assert_eq!(
                    tool.schema["properties"],
                    serde_json::json!({}),
                    "{}",
                    tool.name
                );
            }
        }
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
                brief: String::new(),
                watched: std::collections::BTreeSet::new(),
                foreman_room: None,
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

    /// The tool that speaks is described to each speaker as it works for
    /// them, asserted whole per `docs/conventions.md` §4: what an agent is
    /// told about its tools is prompt text.
    #[test]
    fn the_speaking_tool_is_described_to_each_speaker_exactly() {
        assert_eq!(
            say_description(Speaker::Foreman(ProjectId::from_uuid(Uuid::from_u128(1)))),
            "Say something to the people on this project's channel, in Markdown: it is \
             rendered, so headings, lists, code, tables and links all show. It posts under \
             the message you name with `to`, which is how a person is answered where they \
             asked; without one it posts at the root of your own room, where your ordinary \
             output already goes and the person who asked is not."
        );
        assert_eq!(
            say_description(Speaker::Job(JobId::from_uuid(Uuid::from_u128(2)))),
            "Say something to the people in this job's room, in Markdown: it is rendered, \
             so headings, lists, code, tables and links all show. It posts at the root of \
             your room, or under a message when you name it with `to`, as each message is \
             shown to you. Everything you write as ordinary output is posted at the root as \
             well, so use this to answer in a person's thread, or when you need an answer \
             from a person."
        );
    }
}
