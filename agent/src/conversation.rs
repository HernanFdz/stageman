//! The conversation with an agent, as a state machine driven by lines.
//!
//! Since
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
//! the instance holds a turn's conversation and the world only carries
//! lines: the agent's process is kept open, every line it writes arrives as
//! an event, and every line this side says goes out as an effect. So the
//! conversation is a value stepped one line at a time, and it decides
//! everything the connection used to decide across awaits — what to say
//! first, what each answer leads to, how a request of the agent's own is
//! answered, and when it is over.
//!
//! Three types, and every shape on the wire is the protocol library's own,
//! per `docs/decisions/0014-the-protocols-own-sdk-and-our-own-spawning.md`.
//! [`Said`] is one line this side says, as a value: rendered to the line that
//! crosses the pipe and read back from one, so that whatever answers it in a
//! simulation never matches on strings and cannot drift from the renderer.
//! [`Heard`] is one line the agent says, classified as the protocol
//! classifies it, with the constructors a simulated agent renders its
//! answers through. [`Conversation`] is the machine between them.
//!
//! Nothing here formats. A declaration of the tools carries the credential
//! an agent presents to them, so [`Said`] and [`Conversation`] implement
//! neither `Debug` nor `Display`, for the reason `docs/conventions.md` §4
//! gives; what a test prints is the line, and every credential in a test is
//! fake.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, InitializeRequest, InitializeResponse, ListSessionsRequest,
    ListSessionsResponse, LoadSessionRequest, LoadSessionResponse, McpServer, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionId, PermissionOptionKind, PromptRequest,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionConfigId, SessionConfigOption, SessionConfigOptionValue,
    SessionConfigSelectOption, SessionConfigValueId, SessionId, SessionInfo, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, TextContent,
    ToolCall, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
// The envelope is the protocol library's too: its versioned message, its
// request, its response and its notification, which the library itself
// reads every line through.
use agent_client_protocol::schema::v1::{
    JsonRpcMessage as Envelope, Notification, Request, RequestId, Response,
};
use agent_client_protocol::{Error, JsonRpcMessage as _, RawJsonRpcMessage, RawJsonRpcParams};
use serde::Serialize;
use stageman_core::Kit;

use crate::{
    AgentError, Answer, ProtocolVersion, STDERR_LIMIT, StopReason, Tools, WORKSPACE, current,
    declaration, refused, took, wired,
};

/// Whether a conversation starts a session or picks one up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Opening {
    /// Make a new session in a container that has none.
    Fresh,
    /// Find the session already in this container, and load it.
    Resumed,
}

/// One line this side says to an agent, as a value.
///
/// Rendered by [`Said::line`] and read back by [`Said::parse`], and the two
/// are tested against each other: the conversation speaks through the first,
/// and a simulated agent listens through the second.
#[derive(Clone, PartialEq, Eq)]
pub enum Said {
    /// Opens the connection.
    Initialize {
        /// The request's identifier.
        id: i64,
    },
    /// Makes a session in the workspace, declaring the tools if there are
    /// any.
    NewSession {
        /// The request's identifier.
        id: i64,
        /// What the agent is told about the tools, credential included.
        tools: Option<McpServer>,
    },
    /// Asks which sessions the container holds.
    ListSessions {
        /// The request's identifier.
        id: i64,
    },
    /// Loads the session found, declaring the tools again — which is what
    /// lets a resumed container be told where the instance is *now*.
    LoadSession {
        /// The request's identifier.
        id: i64,
        /// Which session.
        session: String,
        /// What the agent is told about the tools, credential included.
        tools: Option<McpServer>,
    },
    /// Sets one option of the kit.
    SetOption {
        /// The request's identifier.
        id: i64,
        /// Which session.
        session: String,
        /// The option, as the adapter names it.
        option: String,
        /// The value, as this crate spells it.
        value: String,
    },
    /// Puts the question.
    Prompt {
        /// The request's identifier.
        id: i64,
        /// Which session.
        session: String,
        /// What the agent is told.
        text: String,
    },
    /// Answers the agent's own request for permission: the option chosen,
    /// or none, which cancels.
    Permitted {
        /// The agent's request, by its own identifier.
        id: RequestId,
        /// Which option, if any.
        option: Option<String>,
    },
    /// Declines a request of the agent's this side does not serve.
    Unserved {
        /// The agent's request, by its own identifier.
        id: RequestId,
        /// What it asked for.
        method: String,
    },
}

/// One line an agent says, as the protocol classifies it.
///
/// A response says nothing about what it answers, so what a result *means*
/// is decided by whoever asked, from the identifier; this only tells the
/// four shapes apart. The constructors below are what a simulated agent
/// renders its half of a conversation through.
#[derive(Clone, PartialEq, Eq)]
pub enum Heard {
    /// The result of something this side asked.
    Answered {
        /// Which request.
        id: RequestId,
        /// What it answered, as the protocol spells it.
        result: serde_json::Value,
    },
    /// A refusal of something this side asked.
    Refused {
        /// Which request.
        id: RequestId,
        /// Why.
        error: Error,
    },
    /// Something the agent said without being asked.
    Notified {
        /// What kind of thing.
        method: String,
        /// The notification, as the protocol spells it.
        params: serde_json::Value,
    },
    /// Something the agent asked this side, and is waiting on.
    Asked {
        /// The agent's identifier for it, to answer with.
        id: RequestId,
        /// What it asked for.
        method: String,
        /// The request, as the protocol spells it.
        params: serde_json::Value,
    },
}

/// What one line from the agent led to.
pub enum Exchange {
    /// The conversation goes on, saying these.
    Continue(Vec<String>),
    /// The conversation is over: the answer, or why there is none.
    Over(Result<Answer, AgentError>),
}

/// Something the agent said or did while answering, as it arrived.
///
/// What the instance posts as the transcript, per
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`:
/// the agent's own text, a piece at a time, and each tool call as it
/// begins. Kept by the conversation until taken with
/// [`Conversation::noticed`], so that the machine stays a value stepped one
/// line at a time and what a line led to is read off it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Noticed {
    /// A piece of the agent's own text.
    Said(String),
    /// A tool call began.
    Called {
        /// What the adapter calls it, which for a command is the command.
        title: String,
        /// What kind of thing it does, as the protocol classifies tools.
        kind: ToolKind,
    },
}

/// Where the conversation has got to.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
enum Stage {
    /// The connection is being opened.
    Initialising { asked: i64 },
    /// A session is being made.
    Opening { asked: i64 },
    /// The container is being asked which sessions it holds.
    Listing { asked: i64 },
    /// The session found is being loaded.
    Loading { asked: i64, session: String },
    /// One option of the kit is being set, with what the session reported
    /// it to be before and the options still to set after it.
    Settling {
        asked: i64,
        session: String,
        option: String,
        value: String,
        before: Option<String>,
        remaining: VecDeque<(String, String)>,
    },
    /// The question has been put.
    Prompting { asked: i64, session: String },
    /// Nothing more will be said.
    Over,
}

/// The conversation with one agent, from opening the connection to the
/// answer.
///
/// Begun with what it is to say, fed every line the agent writes, answering
/// with what to say back, until it is over. The kit is settled between
/// opening the session and the prompt, on every conversation, because a
/// loaded session forgets what it was set to — see
/// `docs/decisions/0048-a-job-runs-on-a-kit.md`.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct Conversation {
    opening: Opening,
    tools: Option<McpServer>,
    kit: Kit,
    question: String,
    /// The next identifier this side mints.
    next: i64,
    stage: Stage,
    /// Everything the agent has said so far, its message text only.
    heard: String,
    /// What the agent has said and done since it was last taken.
    noticed: Vec<Noticed>,
    /// What the session currently reports it can be set to.
    advertised: Vec<SessionConfigOption>,
    /// What the session reported each setting to be, after being set.
    reported: BTreeMap<String, String>,
}

impl Conversation {
    /// Begins a conversation, and says what to send first.
    #[must_use]
    pub fn begin(
        opening: Opening,
        tools: Option<&Tools>,
        kit: Kit,
        question: impl Into<String>,
    ) -> (Self, Vec<String>) {
        let mut conversation = Self {
            opening,
            tools: tools.map(declaration),
            kit,
            question: question.into(),
            next: 1,
            stage: Stage::Over,
            heard: String::new(),
            noticed: Vec::new(),
            advertised: Vec::new(),
            reported: BTreeMap::new(),
        };
        let asked = conversation.mint();
        conversation.stage = Stage::Initialising { asked };
        (conversation, vec![Said::Initialize { id: asked }.line()])
    }

    /// What the conversation is waiting on, for a log line or an error.
    #[must_use]
    pub const fn waiting_for(&self) -> &'static str {
        match self.stage {
            Stage::Initialising { .. } => "the handshake",
            Stage::Opening { .. } => "a session",
            Stage::Listing { .. } => "the sessions it holds",
            Stage::Loading { .. } => "the session to load",
            Stage::Settling { .. } => "a setting to take",
            Stage::Prompting { .. } => "an answer",
            Stage::Over => "nothing",
        }
    }

    /// Whether the conversation has ended.
    #[must_use]
    pub const fn is_over(&self) -> bool {
        matches!(self.stage, Stage::Over)
    }

    /// One line from the agent, and what it leads to.
    ///
    /// A line that is not the protocol is ignored, and so is an answer to
    /// something this side is not waiting on: the conversation waits on one
    /// request at a time, and anything else is the agent's business.
    pub fn heard(&mut self, line: &str) -> Exchange {
        let Some(heard) = Heard::parse(line) else {
            return Exchange::Continue(Vec::new());
        };
        match heard {
            Heard::Notified { method, params } => {
                self.noted(&method, params);
                Exchange::Continue(Vec::new())
            }
            Heard::Asked { id, method, params } => {
                Exchange::Continue(vec![Self::asked(id, &method, params).line()])
            }
            Heard::Answered { id, result } => self.answered(&id, result),
            Heard::Refused { id, error } => self.refused_by(&id, error),
        }
    }

    /// What the agent's process ending before the conversation was over
    /// means.
    ///
    /// A process that failed on its own terms explains itself better than
    /// the silence it left, so its complaint wins where there is one; one
    /// that exited cleanly without answering is reported by what it was
    /// asked.
    #[must_use]
    pub fn stopped(&self, status: Option<i32>, complaints: &str) -> AgentError {
        match status {
            Some(0) => AgentError::Unanswered {
                asked: self.waiting_for(),
            },
            Some(code) => AgentError::Container {
                status: format!("exit status: {code}"),
                message: complaint(complaints),
            },
            None => AgentError::Container {
                status: "ended by a signal".to_owned(),
                message: complaint(complaints),
            },
        }
    }

    /// An identifier for the next request this side makes.
    const fn mint(&mut self) -> i64 {
        let id = self.next;
        // A conversation makes a dozen requests, so coming round is not a
        // case; an identifier only has to differ from the one in flight.
        self.next = self.next.wrapping_add(1); // CLAMP-OK: a handful of requests per conversation.
        id
    }

    /// Says one thing, on the way to whatever it leads to.
    fn say(said: &Said) -> Exchange {
        Exchange::Continue(vec![said.line()])
    }

    /// Ends the conversation with how it ended.
    fn over(&mut self, outcome: Result<Answer, AgentError>) -> Exchange {
        self.stage = Stage::Over;
        Exchange::Over(outcome)
    }

    /// Something the agent said unasked: its message text is kept as the
    /// answer and noticed as narration, and a tool call beginning is noticed
    /// as working. Everything else it reports on the same stream — its plans,
    /// its usage, a tool call's progress — is let go, until something posts
    /// it. Its reasoning would be noticed too, and the pinned adapter was
    /// measured to send none.
    fn noted(&mut self, method: &str, params: serde_json::Value) {
        if !SessionNotification::matches_method(method) {
            return;
        }
        let Ok(notified) = serde_json::from_value::<SessionNotification>(params) else {
            return;
        };
        match notified.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let ContentBlock::Text(said) = chunk.content {
                    self.heard.push_str(&said.text);
                    self.noticed.push(Noticed::Said(said.text));
                }
            }
            SessionUpdate::ToolCall(call) => self.noticed.push(Noticed::Called {
                title: call.title,
                kind: call.kind,
            }),
            _ => {}
        }
    }

    /// Takes what the agent has said and done since this was last asked, in
    /// the order it arrived.
    pub fn noticed(&mut self) -> Vec<Noticed> {
        std::mem::take(&mut self.noticed)
    }

    /// Something the agent asked: permission, which is granted, or anything
    /// else, which this side does not serve and says so rather than leaving
    /// the agent waiting.
    ///
    /// Granted, and not because permission is meaningless. The boundary this
    /// system relies on is the container, chosen in
    /// `docs/decisions/0012-agents-run-in-containers.md` precisely so that it
    /// enforces isolation rather than the agent respecting it — and
    /// `docs/decisions/0010-acp-is-the-agent-contract.md` measured that
    /// agents decide and report rather than genuinely asking. Refusing here
    /// would forbid an agent from doing what it was started to do, inside a
    /// boundary built to make that safe.
    fn asked(id: RequestId, method: &str, params: serde_json::Value) -> Said {
        if !RequestPermissionRequest::matches_method(method) {
            return Said::Unserved {
                id,
                method: method.to_owned(),
            };
        }
        // A request that cannot be read is cancelled rather than granted:
        // there is nothing to choose from.
        let option = serde_json::from_value::<RequestPermissionRequest>(params)
            .ok()
            .and_then(|request| allowing(&request.options));
        Said::Permitted { id, option }
    }

    /// The agent answered something this side asked.
    fn answered(&mut self, id: &RequestId, result: serde_json::Value) -> Exchange {
        if !self.waiting_on(id) {
            return Exchange::Continue(Vec::new());
        }
        match std::mem::replace(&mut self.stage, Stage::Over) {
            Stage::Initialising { .. } => {
                let asked = self.mint();
                match self.opening {
                    Opening::Fresh => {
                        self.stage = Stage::Opening { asked };
                        Self::say(&Said::NewSession {
                            id: asked,
                            tools: self.tools.clone(),
                        })
                    }
                    Opening::Resumed => {
                        self.stage = Stage::Listing { asked };
                        Self::say(&Said::ListSessions { id: asked })
                    }
                }
            }
            Stage::Opening { .. } => {
                let made = match read::<NewSessionResponse>("session", result) {
                    Ok(made) => made,
                    Err(why) => return self.over(Err(why)),
                };
                // An agent that advertises nothing omits the list, and an
                // empty one is the true reading of that rather than a
                // substitute.
                self.advertised = made.config_options.unwrap_or_default();
                self.settle(made.session_id.0.to_string(), None)
            }
            Stage::Listing { .. } => {
                let known = match read::<ListSessionsResponse>("sessions", result) {
                    Ok(known) => known,
                    Err(why) => return self.over(Err(why)),
                };
                // The first, because a job's container holds one. More than
                // one would mean something else made a session here, which
                // is not a case this system produces.
                let Some(found) = known.sessions.into_iter().next() else {
                    return self.over(Err(AgentError::NothingToResume));
                };
                let asked = self.mint();
                let session = found.session_id.0.to_string();
                self.stage = Stage::Loading {
                    asked,
                    session: session.clone(),
                };
                Self::say(&Said::LoadSession {
                    id: asked,
                    session,
                    tools: self.tools.clone(),
                })
            }
            Stage::Loading { session, .. } => {
                let loaded = match read::<LoadSessionResponse>("loaded session", result) {
                    Ok(loaded) => loaded,
                    Err(why) => return self.over(Err(why)),
                };
                self.advertised = loaded.config_options.unwrap_or_default();
                self.settle(session, None)
            }
            Stage::Settling {
                session,
                option,
                value,
                before,
                remaining,
                ..
            } => {
                let reply = match read::<SetSessionConfigOptionResponse>("setting", result) {
                    Ok(reply) => reply,
                    Err(why) => return self.over(Err(why)),
                };
                self.advertised = reply.config_options;
                let after = current(&self.advertised, &option);
                if !took(&value, before.as_deref(), after.as_deref()) {
                    return self.over(Err(AgentError::Ignored { option, value }));
                }
                if let Some(after) = after {
                    self.reported.insert(option, after);
                }
                self.settle(session, Some(remaining))
            }
            Stage::Prompting { .. } => {
                let reply = match read::<PromptResponse>("answer", result) {
                    Ok(reply) => reply,
                    Err(why) => return self.over(Err(why)),
                };
                let answer = Answer {
                    text: self.heard.clone(),
                    stop_reason: reply.stop_reason,
                    reported: self.reported.clone(),
                };
                self.over(Ok(answer))
            }
            Stage::Over => Exchange::Continue(Vec::new()),
        }
    }

    /// The agent refused something this side asked.
    ///
    /// A refused setting is this crate's failure, in the adapter's own words;
    /// anything else refused is the protocol's.
    fn refused_by(&mut self, id: &RequestId, error: Error) -> Exchange {
        if !self.waiting_on(id) {
            return Exchange::Continue(Vec::new());
        }
        match std::mem::replace(&mut self.stage, Stage::Over) {
            Stage::Settling { option, value, .. } => self.over(Err(AgentError::Refused {
                option,
                value,
                message: refused(&error),
            })),
            _ => self.over(Err(AgentError::Protocol(error))),
        }
    }

    /// Whether an identifier is the one this side is waiting to hear about.
    fn waiting_on(&self, id: &RequestId) -> bool {
        let asked = match &self.stage {
            Stage::Initialising { asked }
            | Stage::Opening { asked }
            | Stage::Listing { asked }
            | Stage::Loading { asked, .. }
            | Stage::Settling { asked, .. }
            | Stage::Prompting { asked, .. } => *asked,
            Stage::Over => return false,
        };
        *id == RequestId::Number(asked)
    }

    /// Sets the next option of the kit, or puts the question once every
    /// option has been set.
    ///
    /// `remaining` is what is still to set, or none at the start, when the
    /// whole kit is. The value *before* each set is read off what the
    /// session currently advertises, which is where [`took`] reads the
    /// change from.
    fn settle(
        &mut self,
        session: String,
        remaining: Option<VecDeque<(String, String)>>,
    ) -> Exchange {
        let mut remaining = remaining.unwrap_or_else(|| {
            wired(&self.kit)
                .into_iter()
                .map(|(option, value)| (option.to_owned(), value.to_owned()))
                .collect()
        });
        let asked = self.mint();
        if let Some((option, value)) = remaining.pop_front() {
            let before = current(&self.advertised, &option);
            self.stage = Stage::Settling {
                asked,
                session: session.clone(),
                option: option.clone(),
                value: value.clone(),
                before,
                remaining,
            };
            Self::say(&Said::SetOption {
                id: asked,
                session,
                option,
                value,
            })
        } else {
            self.stage = Stage::Prompting {
                asked,
                session: session.clone(),
            };
            Self::say(&Said::Prompt {
                id: asked,
                session,
                text: self.question.clone(),
            })
        }
    }
}

/// Which of the options a permission request offers to choose: the first
/// that allows, else the first there is, else nothing.
fn allowing(options: &[PermissionOption]) -> Option<String> {
    options
        .iter()
        .find(|option| {
            matches!(
                option.kind,
                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
            )
        })
        .or_else(|| options.first())
        .map(|option| option.option_id.0.to_string())
}

/// A result, read as the type the request it answers was for.
fn read<T: serde::de::DeserializeOwned>(
    what: &'static str,
    result: serde_json::Value,
) -> Result<T, AgentError> {
    serde_json::from_value(result).map_err(|why| AgentError::Unreadable {
        what,
        why: why.to_string(),
    })
}

/// What a process printed, bounded and prefixed for a message, or nothing.
fn complaint(said: &str) -> String {
    let trimmed = said.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(
            " — {}",
            trimmed.chars().take(STDERR_LIMIT).collect::<String>()
        )
    }
}

/// A value, from anything of the protocol's.
///
/// Null where it will not serialise, which nothing here can: every type
/// rendered is the protocol library's own, with string keys throughout. And
/// null is loud rather than silent — a request with null parameters is one
/// the agent refuses.
fn value(message: &impl Serialize) -> serde_json::Value {
    serde_json::to_value(message).unwrap_or(serde_json::Value::Null)
}

/// One line, from a message of the protocol's: the envelope, then the text.
fn line(message: &impl Serialize) -> String {
    value(&Envelope::wrap(message)).to_string()
}

/// One request of this side's, as a line: the method the message names,
/// under the identifier given.
fn requested<M: Serialize + agent_client_protocol::JsonRpcMessage>(id: i64, message: &M) -> String {
    line(&Request {
        id: RequestId::Number(id),
        method: message.method().into(),
        params: Some(value(message)),
    })
}

/// A request's parameters, as one value.
fn parameters(params: Option<RawJsonRpcParams>) -> serde_json::Value {
    match params {
        Some(RawJsonRpcParams::Array(array)) => serde_json::Value::Array(array),
        Some(RawJsonRpcParams::Object(object)) => serde_json::Value::Object(object),
        None => serde_json::Value::Null,
    }
}

/// The text in what was prompted, whole.
fn prompted(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// How a value of an option is spelled on the wire, read back.
///
/// A named value and nothing else: a kit spells its values, and the one
/// toggle the adapter was measured to offer is not exposed, per
/// `docs/decisions/0048-a-job-runs-on-a-kit.md` — so a boolean is not
/// something this side says, and reads as nothing it said.
fn spelled(value: &SessionConfigOptionValue) -> Option<String> {
    match value {
        SessionConfigOptionValue::ValueId { value } => Some(value.0.to_string()),
        _ => None,
    }
}

impl Said {
    /// The credential the tools were declared with on a session request,
    /// if this says one: the inverse of the declaration, for whoever answers
    /// it in a simulation and wants to know what an agent was handed.
    #[must_use]
    pub fn presented(&self) -> Option<String> {
        let (Self::NewSession {
            tools: Some(McpServer::Http(declared)),
            ..
        }
        | Self::LoadSession {
            tools: Some(McpServer::Http(declared)),
            ..
        }) = self
        else {
            return None;
        };
        declared
            .headers
            .iter()
            .find(|header| header.name == "Authorization")
            .and_then(|header| header.value.strip_prefix("Bearer "))
            .map(str::to_owned)
    }

    /// The line that crosses the pipe.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Initialize { id } => requested(*id, &InitializeRequest::new(ProtocolVersion::V1)),
            Self::NewSession { id, tools } => {
                let mut making = NewSessionRequest::new(PathBuf::from(WORKSPACE));
                making.mcp_servers.extend(tools.iter().cloned());
                requested(*id, &making)
            }
            Self::ListSessions { id } => requested(*id, &ListSessionsRequest::new()),
            Self::LoadSession { id, session, tools } => {
                let mut loading = LoadSessionRequest::new(
                    SessionId::new(session.as_str()),
                    PathBuf::from(WORKSPACE),
                );
                loading.mcp_servers.extend(tools.iter().cloned());
                requested(*id, &loading)
            }
            Self::SetOption {
                id,
                session,
                option,
                value,
            } => requested(
                *id,
                &SetSessionConfigOptionRequest::new(
                    SessionId::new(session.as_str()),
                    SessionConfigId::new(option.as_str()),
                    SessionConfigOptionValue::value_id(SessionConfigValueId::new(value.as_str())),
                ),
            ),
            Self::Prompt { id, session, text } => requested(
                *id,
                &PromptRequest::new(
                    SessionId::new(session.as_str()),
                    vec![ContentBlock::Text(TextContent::new(text.clone()))],
                ),
            ),
            Self::Permitted { id, option } => {
                let outcome =
                    option
                        .as_ref()
                        .map_or(RequestPermissionOutcome::Cancelled, |chosen| {
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                                PermissionOptionId::new(chosen.as_str()),
                            ))
                        });
                line(&Response::<serde_json::Value>::Result {
                    id: id.clone(),
                    result: value(&RequestPermissionResponse::new(outcome)),
                })
            }
            Self::Unserved { id, method } => line(&Response::<serde_json::Value>::Error {
                id: id.clone(),
                error: Error::method_not_found().data(serde_json::json!({ "method": method })),
            }),
        }
    }

    /// What a line this side said means, if it is one this side says.
    ///
    /// The inverse of [`Said::line`], for whatever answers in a simulation.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        match serde_json::from_str::<RawJsonRpcMessage>(line).ok()? {
            RawJsonRpcMessage::Request(request) => {
                let RequestId::Number(id) = request.id else {
                    return None;
                };
                let method: &str = &request.method;
                let params = parameters(request.params);
                if InitializeRequest::matches_method(method) {
                    Some(Self::Initialize { id })
                } else if NewSessionRequest::matches_method(method) {
                    let making: NewSessionRequest = serde_json::from_value(params).ok()?;
                    Some(Self::NewSession {
                        id,
                        tools: making.mcp_servers.into_iter().next(),
                    })
                } else if ListSessionsRequest::matches_method(method) {
                    Some(Self::ListSessions { id })
                } else if LoadSessionRequest::matches_method(method) {
                    let loading: LoadSessionRequest = serde_json::from_value(params).ok()?;
                    Some(Self::LoadSession {
                        id,
                        session: loading.session_id.0.to_string(),
                        tools: loading.mcp_servers.into_iter().next(),
                    })
                } else if SetSessionConfigOptionRequest::matches_method(method) {
                    let setting: SetSessionConfigOptionRequest =
                        serde_json::from_value(params).ok()?;
                    Some(Self::SetOption {
                        id,
                        session: setting.session_id.0.to_string(),
                        option: setting.config_id.0.to_string(),
                        value: spelled(&setting.value)?,
                    })
                } else if PromptRequest::matches_method(method) {
                    let prompting: PromptRequest = serde_json::from_value(params).ok()?;
                    Some(Self::Prompt {
                        id,
                        session: prompting.session_id.0.to_string(),
                        text: prompted(&prompting.prompt),
                    })
                } else {
                    None
                }
            }
            RawJsonRpcMessage::Response(Response::Result { id, result }) => {
                let answered: RequestPermissionResponse = serde_json::from_value(result).ok()?;
                let option = match answered.outcome {
                    RequestPermissionOutcome::Selected(chosen) => {
                        Some(chosen.option_id.0.to_string())
                    }
                    RequestPermissionOutcome::Cancelled => None,
                    _ => return None,
                };
                Some(Self::Permitted { id, option })
            }
            RawJsonRpcMessage::Response(Response::Error { id, error }) => {
                if error.code != Error::method_not_found().code {
                    return None;
                }
                let method = error
                    .data
                    .as_ref()
                    .and_then(|data| data.get("method"))
                    .and_then(serde_json::Value::as_str)?
                    .to_owned();
                Some(Self::Unserved { id, method })
            }
            RawJsonRpcMessage::Notification(_) => None,
        }
    }
}

impl Heard {
    /// What a line the agent said is, if it is the protocol at all.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        Some(
            match serde_json::from_str::<RawJsonRpcMessage>(line).ok()? {
                RawJsonRpcMessage::Request(request) => Self::Asked {
                    id: request.id,
                    method: request.method.to_string(),
                    params: parameters(request.params),
                },
                RawJsonRpcMessage::Notification(notification) => Self::Notified {
                    method: notification.method.to_string(),
                    params: parameters(notification.params),
                },
                RawJsonRpcMessage::Response(Response::Result { id, result }) => {
                    Self::Answered { id, result }
                }
                RawJsonRpcMessage::Response(Response::Error { id, error }) => {
                    Self::Refused { id, error }
                }
            },
        )
    }

    /// The line that crosses the pipe, for a simulated agent to say.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Answered { id, result } => line(&Response::<serde_json::Value>::Result {
                id: id.clone(),
                result: result.clone(),
            }),
            Self::Refused { id, error } => line(&Response::<serde_json::Value>::Error {
                id: id.clone(),
                error: error.clone(),
            }),
            Self::Notified { method, params } => line(&Notification {
                method: method.as_str().into(),
                params: (!params.is_null()).then(|| params.clone()),
            }),
            Self::Asked { id, method, params } => line(&Request {
                id: id.clone(),
                method: method.as_str().into(),
                params: (!params.is_null()).then(|| params.clone()),
            }),
        }
    }

    /// The handshake, answered.
    #[must_use]
    pub fn initialized(id: i64) -> Self {
        Self::Answered {
            id: id.into(),
            result: value(&InitializeResponse::new(ProtocolVersion::V1)),
        }
    }

    /// A session made, advertising options at the values given.
    #[must_use]
    pub fn session_made(id: i64, session: &str, options: &[(&str, &str)]) -> Self {
        let mut made = NewSessionResponse::new(SessionId::new(session));
        made.config_options = Some(advertising(options));
        Self::Answered {
            id: id.into(),
            result: value(&made),
        }
    }

    /// The sessions a container holds.
    #[must_use]
    pub fn sessions(id: i64, sessions: &[&str]) -> Self {
        let known = ListSessionsResponse::new(
            sessions
                .iter()
                .map(|session| SessionInfo::new(SessionId::new(*session), WORKSPACE))
                .collect(),
        );
        Self::Answered {
            id: id.into(),
            result: value(&known),
        }
    }

    /// A session loaded, advertising options at the values given — which a
    /// real one does at the agent's defaults, having forgotten what it was
    /// set to.
    #[must_use]
    pub fn loaded(id: i64, options: &[(&str, &str)]) -> Self {
        let mut loaded = LoadSessionResponse::new();
        loaded.config_options = Some(advertising(options));
        Self::Answered {
            id: id.into(),
            result: value(&loaded),
        }
    }

    /// An option set, with the whole list reported again at the values
    /// given.
    #[must_use]
    pub fn set(id: i64, options: &[(&str, &str)]) -> Self {
        Self::Answered {
            id: id.into(),
            result: value(&SetSessionConfigOptionResponse::new(advertising(options))),
        }
    }

    /// The question answered: why the turn ended.
    #[must_use]
    pub fn prompted(id: i64, stop_reason: StopReason) -> Self {
        Self::Answered {
            id: id.into(),
            result: value(&PromptResponse::new(stop_reason)),
        }
    }

    /// A piece of what the agent is saying.
    #[must_use]
    pub fn said(session: &str, text: &str) -> Self {
        let notified = SessionNotification::new(
            SessionId::new(session),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(text.to_owned()),
            ))),
        );
        Self::Notified {
            method: notified.method().to_owned(),
            params: value(&notified),
        }
    }

    /// The agent beginning a tool call, as the pinned adapter announces
    /// one: pending, under a title, running a command.
    #[must_use]
    pub fn called(session: &str, id: &str, title: &str) -> Self {
        let notified = SessionNotification::new(
            SessionId::new(session),
            SessionUpdate::ToolCall(ToolCall::new(id.to_owned(), title).kind(ToolKind::Execute)),
        );
        Self::Notified {
            method: notified.method().to_owned(),
            params: value(&notified),
        }
    }

    /// The agent asking permission, offering the options named, each
    /// allowing or rejecting.
    #[must_use]
    pub fn permission_asked(id: i64, session: &str, options: &[(&str, bool)]) -> Self {
        let asking = RequestPermissionRequest::new(
            SessionId::new(session),
            ToolCallUpdate::new("call-1", ToolCallUpdateFields::default()),
            options
                .iter()
                .map(|(name, allows)| {
                    PermissionOption::new(
                        PermissionOptionId::new(*name),
                        *name,
                        if *allows {
                            PermissionOptionKind::AllowOnce
                        } else {
                            PermissionOptionKind::RejectOnce
                        },
                    )
                })
                .collect(),
        );
        Self::Asked {
            id: id.into(),
            method: asking.method().to_owned(),
            params: value(&asking),
        }
    }

    /// A setting refused, in the shape the pinned adapter was measured to
    /// refuse one: the protocol's generic message, and the sentence naming
    /// the option and the value as data.
    #[must_use]
    pub fn option_refused(id: i64, option: &str, value: &str) -> Self {
        Self::Refused {
            id: id.into(),
            error: Error::internal_error().data(serde_json::json!({
                "details": format!("Invalid value for config option {option}: {value}"),
            })),
        }
    }
}

/// Options advertised at the values given, each offering only that value.
fn advertising(options: &[(&str, &str)]) -> Vec<SessionConfigOption> {
    options
        .iter()
        .map(|(option, current)| {
            SessionConfigOption::select(
                SessionConfigId::new(*option),
                *option,
                SessionConfigValueId::new(*current),
                vec![SessionConfigSelectOption::new(
                    SessionConfigValueId::new(*current),
                    *current,
                )],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Conversation, Exchange, Heard, Noticed, Opening, Said};
    use crate::{AgentError, StopReason, Tools, declaration};
    use agent_client_protocol::schema::v1::{RequestId, ToolKind};
    use stageman_core::{Agent, ClaudeEffort, ClaudeModel, Kit, Secret};

    fn tools() -> Tools {
        Tools::new(
            "http://host.docker.internal:47113/mcp",
            Secret::new("a-warrant".to_owned()),
        )
    }

    /// What was said, read back as values.
    fn said(lines: &[String]) -> Vec<Said> {
        lines
            .iter()
            .map(|line| {
                Said::parse(line).unwrap_or_else(|| panic!("not something this side says: {line}"))
            })
            .collect()
    }

    /// Feeds one line and expects the conversation to go on.
    fn continuing(conversation: &mut Conversation, heard: &Heard) -> Vec<Said> {
        match conversation.heard(&heard.line()) {
            Exchange::Continue(lines) => said(&lines),
            Exchange::Over(Ok(answer)) => panic!("over, with an answer: {}", answer.text),
            Exchange::Over(Err(why)) => panic!("over, with a failure: {why}"),
        }
    }

    /// Feeds one line, and says how the conversation ended if it did.
    fn over(
        conversation: &mut Conversation,
        heard: &Heard,
    ) -> Option<Result<crate::Answer, AgentError>> {
        match conversation.heard(&heard.line()) {
            Exchange::Continue(_) => None,
            Exchange::Over(outcome) => Some(outcome),
        }
    }

    const DEFAULTS: &[(&str, &str)] = &[
        ("mode", "default"),
        ("model", "default"),
        ("effort", "default"),
    ];

    /// What the agent says and does while answering is noticed as it
    /// arrives and taken by whoever asks, in order: text as said, a tool
    /// call as called, and nothing twice. The answer still collects the
    /// text, as it always did.
    #[test]
    fn what_the_agent_says_and_does_is_noticed_once_and_in_order() {
        let (mut conversation, _) = Conversation::begin(
            Opening::Fresh,
            None,
            Kit::defaults(Agent::Claude),
            "look at the parser",
        );
        assert!(conversation.noticed().is_empty(), "nothing yet");

        continuing(&mut conversation, &Heard::said("sess-1", "Looking"));
        continuing(
            &mut conversation,
            &Heard::called("sess-1", "call-1", "cargo test"),
        );
        continuing(&mut conversation, &Heard::said("sess-1", " around."));

        assert_eq!(
            conversation.noticed(),
            vec![
                Noticed::Said("Looking".to_owned()),
                Noticed::Called {
                    title: "cargo test".to_owned(),
                    kind: ToolKind::Execute,
                },
                Noticed::Said(" around.".to_owned()),
            ]
        );
        assert!(conversation.noticed().is_empty(), "taken once");
        assert_eq!(conversation.heard, "Looking around.");
    }

    /// The whole of a fresh conversation, from the handshake to the answer:
    /// what is said, in order, and what the answer is made of.
    #[test]
    fn a_fresh_conversation_opens_a_session_settles_the_kit_and_asks() {
        let (mut conversation, first) = Conversation::begin(
            Opening::Fresh,
            Some(&tools()),
            Kit::defaults(Agent::Claude),
            "hello",
        );
        assert!(said(&first) == [Said::Initialize { id: 1 }]);
        assert_eq!(conversation.waiting_for(), "the handshake");
        assert!(!conversation.is_over());

        assert!(
            continuing(&mut conversation, &Heard::initialized(1))
                == [Said::NewSession {
                    id: 2,
                    tools: Some(declaration(&tools())),
                }],
            "a session is made, with the tools declared on it"
        );
        assert!(
            continuing(
                &mut conversation,
                &Heard::session_made(2, "sess-1", DEFAULTS)
            ) == [Said::SetOption {
                id: 3,
                session: "sess-1".to_owned(),
                option: "mode".to_owned(),
                value: "default".to_owned(),
            }],
            "the mode first, per 0049"
        );
        assert_eq!(conversation.waiting_for(), "a setting to take");
        assert!(
            continuing(&mut conversation, &Heard::set(3, DEFAULTS))
                == [Said::SetOption {
                    id: 4,
                    session: "sess-1".to_owned(),
                    option: "model".to_owned(),
                    value: "default".to_owned(),
                }]
        );
        assert!(
            continuing(&mut conversation, &Heard::set(4, DEFAULTS))
                == [Said::SetOption {
                    id: 5,
                    session: "sess-1".to_owned(),
                    option: "effort".to_owned(),
                    value: "default".to_owned(),
                }]
        );
        assert!(
            continuing(&mut conversation, &Heard::set(5, DEFAULTS))
                == [Said::Prompt {
                    id: 6,
                    session: "sess-1".to_owned(),
                    text: "hello".to_owned(),
                }],
            "every setting taken, the question is put"
        );
        assert_eq!(conversation.waiting_for(), "an answer");

        assert!(continuing(&mut conversation, &Heard::said("sess-1", "Hel")).is_empty());
        assert!(continuing(&mut conversation, &Heard::said("sess-1", "lo.")).is_empty());
        let answer = over(&mut conversation, &Heard::prompted(6, StopReason::EndTurn))
            .expect("the conversation is over")
            .expect("the conversation ends with an answer");
        assert_eq!(answer.text, "Hello.");
        assert_eq!(answer.stop_reason, StopReason::EndTurn);
        assert_eq!(
            answer.reported,
            [
                ("mode".to_owned(), "default".to_owned()),
                ("model".to_owned(), "default".to_owned()),
                ("effort".to_owned(), "default".to_owned()),
            ]
            .into()
        );
        assert!(conversation.is_over());
        assert!(
            matches!(
                conversation.heard(&Heard::prompted(6, StopReason::EndTurn).line()),
                Exchange::Continue(lines) if lines.is_empty()
            ),
            "nothing more is said once it is over"
        );
    }

    /// A resumed conversation finds the session the container holds, loads
    /// it with the tools declared again, and settles the kit again.
    #[test]
    fn a_resumed_conversation_loads_the_session_the_container_holds() {
        let (mut conversation, first) = Conversation::begin(
            Opening::Resumed,
            None,
            Kit::defaults(Agent::Claude),
            "carry on",
        );
        assert!(said(&first) == [Said::Initialize { id: 1 }]);
        assert!(
            continuing(&mut conversation, &Heard::initialized(1)) == [Said::ListSessions { id: 2 }]
        );
        assert!(
            continuing(
                &mut conversation,
                &Heard::sessions(2, &["sess-9", "sess-10"])
            ) == [Said::LoadSession {
                id: 3,
                session: "sess-9".to_owned(),
                tools: None,
            }],
            "the first session, and the tools it has: none"
        );
        assert!(
            continuing(&mut conversation, &Heard::loaded(3, DEFAULTS))
                == [Said::SetOption {
                    id: 4,
                    session: "sess-9".to_owned(),
                    option: "mode".to_owned(),
                    value: "default".to_owned(),
                }],
            "settled again, because a loaded session forgets"
        );
    }

    /// A container with no session has nothing to resume.
    #[test]
    fn a_container_with_no_session_has_nothing_to_resume() {
        let (mut conversation, _) = Conversation::begin(
            Opening::Resumed,
            None,
            Kit::defaults(Agent::Claude),
            "carry on",
        );
        drop(continuing(&mut conversation, &Heard::initialized(1)));
        let why = over(&mut conversation, &Heard::sessions(2, &[]))
            .expect("the conversation is over")
            .expect_err("nothing to resume");
        assert!(matches!(why, AgentError::NothingToResume), "{why}");
    }

    /// A setting the adapter refuses ends the conversation before the
    /// prompt, in the adapter's own words.
    #[test]
    fn a_setting_the_agent_refuses_ends_the_conversation_with_its_words() {
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, Kit::defaults(Agent::Claude), "hello");
        drop(continuing(&mut conversation, &Heard::initialized(1)));
        drop(continuing(
            &mut conversation,
            &Heard::session_made(2, "sess-1", DEFAULTS),
        ));
        let why = over(
            &mut conversation,
            &Heard::option_refused(3, "mode", "default"),
        )
        .expect("the conversation is over")
        .expect_err("refused");
        match why {
            AgentError::Refused {
                option,
                value,
                message,
            } => {
                assert_eq!((option.as_str(), value.as_str()), ("mode", "default"));
                assert_eq!(message, "Invalid value for config option mode: default");
            }
            other => panic!("{other}"),
        }
    }

    /// A setting the adapter accepts and reports unchanged ends the
    /// conversation, and one whose reading moved — however it is spelled —
    /// is recorded as what was reported.
    ///
    /// The case `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
    /// expected the conversation to gain once it was a machine: an adapter
    /// that says yes and does nothing would otherwise run every job on its
    /// defaults with every reply reading as success.
    #[test]
    fn a_setting_accepted_and_reported_unchanged_ends_the_conversation() {
        let opus = Kit::Claude {
            model: ClaudeModel::Opus {
                effort: ClaudeEffort::High,
            },
        };
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, opus.clone(), "hello");
        drop(continuing(&mut conversation, &Heard::initialized(1)));
        assert!(
            continuing(
                &mut conversation,
                &Heard::session_made(2, "sess-1", DEFAULTS)
            ) == [Said::SetOption {
                id: 3,
                session: "sess-1".to_owned(),
                option: "mode".to_owned(),
                value: "default".to_owned(),
            }]
        );
        assert!(
            continuing(&mut conversation, &Heard::set(3, DEFAULTS))
                == [Said::SetOption {
                    id: 4,
                    session: "sess-1".to_owned(),
                    option: "model".to_owned(),
                    value: "opus".to_owned(),
                }]
        );
        let why = over(&mut conversation, &Heard::set(4, DEFAULTS))
            .expect("the conversation is over")
            .expect_err("ignored");
        match why {
            AgentError::Ignored { option, value } => {
                assert_eq!((option.as_str(), value.as_str()), ("model", "opus"));
            }
            other => panic!("{other}"),
        }

        // The same set, reported as having moved to a spelling of the
        // adapter's own, takes, and what is recorded is that spelling.
        let (mut conversation, _) = Conversation::begin(Opening::Fresh, None, opus, "hello");
        drop(continuing(&mut conversation, &Heard::initialized(1)));
        drop(continuing(
            &mut conversation,
            &Heard::session_made(2, "sess-1", DEFAULTS),
        ));
        drop(continuing(&mut conversation, &Heard::set(3, DEFAULTS)));
        let moved = &[
            ("mode", "default"),
            ("model", "opus[1m]"),
            ("effort", "default"),
        ];
        assert!(
            continuing(&mut conversation, &Heard::set(4, moved))
                == [Said::SetOption {
                    id: 5,
                    session: "sess-1".to_owned(),
                    option: "effort".to_owned(),
                    value: "high".to_owned(),
                }]
        );
        let settled = &[
            ("mode", "default"),
            ("model", "opus[1m]"),
            ("effort", "high"),
        ];
        drop(continuing(&mut conversation, &Heard::set(5, settled)));
        let answer = over(&mut conversation, &Heard::prompted(6, StopReason::EndTurn))
            .expect("the conversation is over")
            .expect("answered");
        assert_eq!(
            answer.reported.get("model").map(String::as_str),
            Some("opus[1m]"),
            "what ran, not what was asked"
        );
        assert_eq!(
            answer.reported.get("effort").map(String::as_str),
            Some("high")
        );
    }

    /// A request for permission is answered with the first option that
    /// allows, else the first there is, else cancelled — and the
    /// conversation goes on to its answer afterwards.
    #[test]
    fn a_request_for_permission_is_answered_with_what_allows() {
        let (mut conversation, _) = Conversation::begin(
            Opening::Fresh,
            None,
            Kit::Claude {
                model: ClaudeModel::Haiku,
            },
            "hello",
        );
        drop(continuing(&mut conversation, &Heard::initialized(1)));
        drop(continuing(
            &mut conversation,
            &Heard::session_made(2, "sess-1", DEFAULTS),
        ));
        drop(continuing(&mut conversation, &Heard::set(3, DEFAULTS)));
        assert!(
            continuing(
                &mut conversation,
                &Heard::set(4, &[("mode", "default"), ("model", "haiku")])
            ) == [Said::Prompt {
                id: 5,
                session: "sess-1".to_owned(),
                text: "hello".to_owned(),
            }],
            "haiku has no effort, so two settings and then the question"
        );

        assert!(
            continuing(
                &mut conversation,
                &Heard::permission_asked(
                    7,
                    "sess-1",
                    &[("reject-once", false), ("allow-once", true)]
                )
            ) == [Said::Permitted {
                id: RequestId::Number(7),
                option: Some("allow-once".to_owned()),
            }]
        );
        assert!(
            continuing(
                &mut conversation,
                &Heard::permission_asked(8, "sess-1", &[("reject-once", false)])
            ) == [Said::Permitted {
                id: RequestId::Number(8),
                option: Some("reject-once".to_owned()),
            }],
            "nothing allows, so the first there is"
        );
        assert!(
            continuing(
                &mut conversation,
                &Heard::permission_asked(9, "sess-1", &[])
            ) == [Said::Permitted {
                id: RequestId::Number(9),
                option: None,
            }],
            "nothing to choose from, so cancelled"
        );
        let answer = over(
            &mut conversation,
            &Heard::prompted(5, StopReason::MaxTokens),
        )
        .expect("the conversation is over")
        .expect("still answered afterwards");
        assert_eq!(answer.stop_reason, StopReason::MaxTokens);
    }

    /// A request this side does not serve is declined rather than left
    /// waiting, which is what the agent's own library would do with it.
    #[test]
    fn a_request_this_side_does_not_serve_is_declined() {
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, Kit::defaults(Agent::Claude), "hello");
        let asked = Heard::Asked {
            id: RequestId::Str("fs-1".to_owned()),
            method: "fs/read_text_file".to_owned(),
            params: serde_json::json!({ "path": "/etc/passwd" }),
        };
        assert!(
            continuing(&mut conversation, &asked)
                == [Said::Unserved {
                    id: RequestId::Str("fs-1".to_owned()),
                    method: "fs/read_text_file".to_owned(),
                }]
        );
    }

    /// What is not the protocol, or not what this side is waiting on, is
    /// let go: a stray answer, a notification of another kind, a line that
    /// is not JSON.
    #[test]
    fn what_is_not_the_protocol_or_not_waited_on_is_let_go() {
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, Kit::defaults(Agent::Claude), "hello");
        assert!(matches!(
            conversation.heard("not json at all"),
            Exchange::Continue(lines) if lines.is_empty()
        ));
        assert!(
            continuing(&mut conversation, &Heard::initialized(99)).is_empty(),
            "an answer to something else"
        );
        assert!(
            continuing(&mut conversation, &Heard::option_refused(99, "x", "y")).is_empty(),
            "a refusal of something else"
        );
        assert!(
            continuing(
                &mut conversation,
                &Heard::Notified {
                    method: "session/update".to_owned(),
                    params: serde_json::json!({
                        "sessionId": "sess-1",
                        "update": { "sessionUpdate": "agent_thought_chunk",
                                    "content": { "type": "text", "text": "thinking" } }
                    }),
                }
            )
            .is_empty()
        );
        assert_eq!(
            conversation.waiting_for(),
            "the handshake",
            "still where it was"
        );
        // And the real answer is still taken afterwards.
        assert!(
            continuing(&mut conversation, &Heard::initialized(1))
                == [Said::NewSession { id: 2, tools: None }]
        );
    }

    /// An answer that cannot be read as what was asked for ends the
    /// conversation saying what could not be read.
    #[test]
    fn an_answer_that_cannot_be_read_ends_the_conversation() {
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, Kit::defaults(Agent::Claude), "hello");
        drop(continuing(&mut conversation, &Heard::initialized(1)));
        let why = over(
            &mut conversation,
            &Heard::Answered {
                id: 2.into(),
                result: serde_json::json!({ "not": "a session" }),
            },
        )
        .expect("the conversation is over")
        .expect_err("unreadable");
        match why {
            AgentError::Unreadable { what, .. } => assert_eq!(what, "session"),
            other => panic!("{other}"),
        }
    }

    /// A refusal of anything but a setting is the protocol's failure.
    #[test]
    fn a_refusal_of_the_handshake_is_the_protocols_failure() {
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, Kit::defaults(Agent::Claude), "hello");
        let why = over(&mut conversation, &Heard::option_refused(1, "x", "y"))
            .expect("the conversation is over")
            .expect_err("refused");
        assert!(matches!(why, AgentError::Protocol(_)), "{why}");
    }

    /// An agent whose process ends before it answers is reported by its
    /// complaint where it made one, and by what it was asked otherwise.
    #[test]
    fn an_agent_that_stops_before_answering_is_reported_by_its_complaint_or_by_what_it_was_asked() {
        let (mut conversation, _) =
            Conversation::begin(Opening::Fresh, None, Kit::defaults(Agent::Claude), "hello");
        drop(continuing(&mut conversation, &Heard::initialized(1)));

        match conversation.stopped(Some(1), "  no such image\n") {
            AgentError::Container { status, message } => {
                assert_eq!(status, "exit status: 1");
                assert_eq!(message, " — no such image");
            }
            other => panic!("{other}"),
        }
        match conversation.stopped(None, "") {
            AgentError::Container { status, message } => {
                assert_eq!(status, "ended by a signal");
                assert_eq!(message, "");
            }
            other => panic!("{other}"),
        }
        match conversation.stopped(Some(0), "") {
            AgentError::Unanswered { asked } => assert_eq!(asked, "a session"),
            other => panic!("{other}"),
        }
        let flood = "x".repeat(20_000);
        match conversation.stopped(Some(2), &flood) {
            AgentError::Container { message, .. } => {
                assert_eq!(
                    message.len(),
                    " — ".len() + 8 * 1024,
                    "eight kibibytes of it, which is the bound"
                );
            }
            other => panic!("{other}"),
        }
    }

    /// Every line this side says reads back as itself, and a line this side
    /// never says reads as nothing.
    #[test]
    fn every_line_this_side_says_reads_back_as_itself() {
        let every = [
            Said::Initialize { id: 1 },
            Said::NewSession {
                id: 2,
                tools: Some(declaration(&tools())),
            },
            Said::NewSession { id: 2, tools: None },
            Said::ListSessions { id: 3 },
            Said::LoadSession {
                id: 4,
                session: "sess-9".to_owned(),
                tools: Some(declaration(&tools())),
            },
            Said::SetOption {
                id: 5,
                session: "sess-9".to_owned(),
                option: "model".to_owned(),
                value: "opus".to_owned(),
            },
            Said::Prompt {
                id: 6,
                session: "sess-9".to_owned(),
                text: "Fix the build.".to_owned(),
            },
            Said::Permitted {
                id: RequestId::Number(7),
                option: Some("allow-once".to_owned()),
            },
            Said::Permitted {
                id: RequestId::Str("p-1".to_owned()),
                option: None,
            },
            Said::Unserved {
                id: RequestId::Number(8),
                method: "terminal/create".to_owned(),
            },
        ];
        for said in every {
            let line = said.line();
            assert!(line.starts_with("{\"jsonrpc\":\"2.0\""), "{line}");
            assert!(Said::parse(&line) == Some(said), "{line}");
        }
        assert!(
            Said::parse(
                r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"x"}}"#
            )
            .is_none(),
            "a notification is nothing this side says"
        );
        assert!(
            Said::parse(r#"{"jsonrpc":"2.0","id":1,"method":"session/fork","params":{}}"#)
                .is_none(),
            "a request this side never makes"
        );
        assert!(
            Said::parse(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).is_none(),
            "a response that is not a permission"
        );
        assert!(Said::parse("garbage").is_none());

        // The credential crosses in the declaration, which is the point of
        // declaring the tools at all, and nothing here formats it — and it
        // reads back off the declaration for whoever answers.
        let declared = Said::NewSession {
            id: 2,
            tools: Some(declaration(&tools())),
        };
        let line = declared.line();
        assert!(line.contains("Bearer a-warrant"), "{line}");
        assert!(line.contains(r#""cwd":"/workspace""#), "{line}");
        assert_eq!(declared.presented().as_deref(), Some("a-warrant"));
        assert_eq!(
            Said::LoadSession {
                id: 4,
                session: "sess-9".to_owned(),
                tools: Some(declaration(&tools())),
            }
            .presented()
            .as_deref(),
            Some("a-warrant")
        );
        assert_eq!(Said::NewSession { id: 2, tools: None }.presented(), None);
        assert_eq!(Said::Initialize { id: 1 }.presented(), None);
    }

    /// Every line an agent says reads back as itself, and the constructors
    /// render the shapes the pinned adapter was measured to send.
    #[test]
    fn every_line_the_agent_says_reads_back_as_itself() {
        let every = [
            Heard::initialized(1),
            Heard::session_made(2, "sess-1", &[("model", "default")]),
            Heard::sessions(2, &["sess-1"]),
            Heard::loaded(3, &[]),
            Heard::set(4, &[("model", "opus[1m]")]),
            Heard::prompted(6, StopReason::EndTurn),
            Heard::said("sess-1", "hello"),
            Heard::permission_asked(7, "sess-1", &[("allow-once", true)]),
            Heard::option_refused(5, "effort", "nope"),
            Heard::Asked {
                id: RequestId::Str("t-1".to_owned()),
                method: "terminal/create".to_owned(),
                params: serde_json::Value::Null,
            },
        ];
        for heard in every {
            let line = heard.line();
            assert!(Heard::parse(&line) == Some(heard), "{line}");
        }
        assert!(Heard::parse("garbage").is_none());
        assert!(Heard::parse("{}").is_none(), "not any of the four shapes");

        assert!(
            Heard::prompted(6, StopReason::EndTurn)
                .line()
                .contains(r#""stopReason":"end_turn""#)
        );
        assert!(
            Heard::said("sess-1", "hello")
                .line()
                .contains(r#""sessionUpdate":"agent_message_chunk""#)
        );
        assert!(
            Heard::option_refused(5, "effort", "nope")
                .line()
                .contains(r#""details":"Invalid value for config option effort: nope""#)
        );
    }

    /// Neither the conversation nor what it says formats, because both can
    /// carry the credential an agent presents to the tools. The probe
    /// answers through an inherent method only where `Debug` exists.
    #[test]
    fn a_conversation_and_what_it_says_format_not_at_all() {
        struct Probe<T>(std::marker::PhantomData<T>);
        impl<T: std::fmt::Debug> Probe<T> {
            #[expect(
                clippy::unused_self,
                reason = "a method, so that resolution prefers it to the trait's where it exists"
            )]
            const fn formats(&self) -> bool {
                true
            }
        }
        trait Otherwise {
            fn formats(&self) -> bool {
                false
            }
        }
        impl<T> Otherwise for Probe<T> {}

        assert!(!Probe::<Conversation>(std::marker::PhantomData).formats());
        assert!(!Probe::<Said>(std::marker::PhantomData).formats());
        assert!(
            Probe::<Opening>(std::marker::PhantomData).formats(),
            "the probe tells"
        );
    }
}
