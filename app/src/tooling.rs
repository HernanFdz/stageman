//! The tools this instance serves to the agents it runs.
//!
//! `docs/decisions/0034-tools-are-served-not-shipped.md` moves everything an
//! agent does outside its container from a program shipped in the image to a
//! tool served from here. This is that endpoint: MCP over HTTP, on the
//! listener `asking` already binds, reached through the one hostname a
//! container has on either runtime.
//!
//! **Deciding is split from serving, as everywhere else here.** [`decode`]
//! turns one request into what it means and is pure; [`tools`] answers what a
//! bearer may be offered and is pure; the handler below only checks who is
//! asking, performs the effect, and writes the envelope back. That split is
//! what lets the interesting half be tested without binding a port.
//!
//! **What a bearer is offered is decided by the credential it presents**, and
//! that is the whole authorisation mechanism rather than a convenience. Under
//! `docs/decisions/0032-a-foreman-asks-the-instance-by-warrant.md` a job could
//! not create jobs because its container held no warrant — nothing to get
//! wrong, and nothing to test either. Here the same property is a decision
//! this module makes and [`tools`] is where it is made, so it is a function
//! with a test rather than an absence.

// Through the framework's re-export rather than a direct dependency, for the
// reason `asking` and `serving` do the same: the version that matters is
// whichever one the framework serves with, and naming it twice is how the two
// drift.
use dioxus::server::axum;
use dioxus::server::axum::response::IntoResponse as _;
use stageman_core::ProjectId;

/// The protocol version answered when a caller names none.
///
/// A caller's own version is echoed when it sends one, which is what every
/// observed client does. This exists for the one that does not, and is a
/// version rather than an error because refusing a handshake over a missing
/// field would be a strict reading nothing benefits from.
const PROTOCOL: &str = "2025-06-18";

/// What this instance calls itself to an agent.
///
/// It prefixes every tool name the model sees — `start_job` is offered as
/// `mcp__stageman__start_job` — so the tools themselves are named for the act
/// alone and never repeat it. That naming rule is the part of
/// `docs/decisions/0028-stageman-ships-the-tool-that-speaks.md` which survives
/// that record being superseded.
const SERVER: &str = "stageman";

/// What a presented credential entitles its bearer to be offered.
///
/// One variant today because one thing holds a credential: a foreman, by its
/// project's warrant. A job gets one when it has a tool to be offered, and the
/// point of naming this now is that adding that variant is where the decision
/// goes, rather than somewhere a reader has to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// A foreman of this project. May start jobs on it.
    Foreman(ProjectId),
}

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

/// What this scope may be offered, given the agents its project runs jobs on.
///
/// **The agents are enumerated in the schema rather than described in prose**,
/// which is the concrete thing typed arguments buy over a command line. The
/// old endpoint took an agent name as text and refused an unknown one with a
/// message listing the alternatives, because a shell command cannot express a
/// closed set. A schema can, so a foreman picks from what exists instead of
/// guessing and being corrected — and `docs/decisions/0006-agents-are-pluggable.md`
/// keeps the choice, which is what an absent field would have taken back.
///
/// An empty set omits the enumeration rather than emitting an empty one: a
/// schema no value can satisfy reads to a model as a broken tool, where a
/// plain string reaches the refusal below and says why.
#[must_use]
pub fn tools(scope: Scope, agents: &[&str]) -> Vec<Tool> {
    let Scope::Foreman(_) = scope;

    let mut agent = serde_json::json!({
        "type": "string",
        "description": "Which agent runs this job. Pick from what this project allows.",
    });
    if !agents.is_empty()
        && let Some(fields) = agent.as_object_mut()
    {
        fields.insert("enum".to_owned(), serde_json::json!(agents));
    }

    vec![Tool {
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
                "agent": agent,
            },
            "required": ["reason", "instructions", "agent"],
        }),
    }]
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
    /// A tool this instance does not serve, by name.
    NoSuchTool(String),
    /// Something needing no answer at all.
    Notification,
    /// Something this instance does not implement and need not.
    Ignored,
}

/// What a request to start a job carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Starting {
    /// Why the foreman decided to, in its own words.
    pub reason: String,
    /// What the job's agent is to do.
    pub instructions: String,
    /// Which agent runs it, as a foreman names it.
    pub agent: String,
}

/// What a request means, without performing any of it.
///
/// Pure, and the reason this file is testable at all: every branch below is
/// reachable from a literal, so the interesting decisions need no port, no
/// container and no model.
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
/// impossible, and splitting that across two layers gives a model two
/// different-sounding failures for one mistake.
fn calling(params: &serde_json::Value) -> Call {
    let name = params
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if name != "start_job" {
        return Call::NoSuchTool(name.to_owned());
    }
    let arguments = params.get("arguments");
    let field = |key: &str| {
        arguments
            .and_then(|given| given.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Call::Starting(Starting {
        reason: field("reason"),
        instructions: field("instructions"),
        agent: field("agent"),
    })
}

/// The credential a request presents, if it presents one.
///
/// Bearer only, and compared nowhere here: this reads the header and the
/// instance decides whether it names anything, so that a malformed header and
/// an unknown credential reach the same refusal by the same path.
#[must_use]
fn presented(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Where a container reaches the tools this instance serves.
///
/// One hostname for both runtimes, which is measured rather than assumed:
/// `--add-host=host.docker.internal:host-gateway` is honoured by Docker and by
/// Podman alike, so nothing here has to know which one is in use.
///
/// The one thing served on that listener, since
/// `docs/decisions/0034-tools-are-served-not-shipped.md` removed the route a
/// shipped program used to post to. Where it binds, and why that is the
/// awkward part, is
/// `docs/decisions/0033-the-job-endpoint-listens-beyond-loopback.md`.
#[must_use]
pub fn endpoint(port: u16) -> String {
    format!("http://host.docker.internal:{port}/mcp")
}

/// One request, as it arrives.
#[derive(serde::Deserialize)]
pub struct Incoming {
    /// Absent on a notification, which is what makes one answerable or not.
    #[serde(default)]
    id: Option<serde_json::Value>,
    /// What is being asked.
    method: String,
    /// What it was asked with.
    #[serde(default)]
    params: serde_json::Value,
}

/// The tools endpoint, for the listener `asking` binds.
///
/// A method router rather than a whole one, so the extension carrying the
/// instance is applied once where the listener is assembled instead of twice.
pub fn served() -> axum::routing::MethodRouter {
    axum::routing::post(called).get(declining).delete(closing)
}

/// Declines the stream a client may offer to open.
///
/// Every tool here answers within its own call, so there is nothing this
/// instance would ever push. Measured: a client offered the stream, was
/// refused, and completed a tool call regardless.
#[mutants::skip]
async fn declining() -> axum::http::StatusCode {
    axum::http::StatusCode::METHOD_NOT_ALLOWED
}

/// Accepts a client hanging up.
///
/// Nothing is held per connection — the credential decides everything and is
/// presented on each request — so this has nothing to release and says so
/// rather than refusing.
#[mutants::skip]
async fn closing() -> axum::http::StatusCode {
    axum::http::StatusCode::NO_CONTENT
}

/// Answers one request, if whoever asked is allowed to.
#[mutants::skip]
async fn called(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    axum::Extension(store): axum::Extension<std::sync::Arc<crate::Store>>,
    headers: axum::http::HeaderMap,
    axum::Json(incoming): axum::Json<Incoming>,
) -> axum::response::Response {
    if !crate::asking::from_nearby(peer.ip()) {
        tracing::warn!(%peer, "the tools were reached from beyond this machine");
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }

    let scope = presented(&headers)
        .and_then(|credential| store.read().warranted(credential))
        .map(Scope::Foreman);
    let Some(scope) = scope else {
        // Deliberately the same answer as a bad peer, and deliberately without
        // detail: anything distinguishing "no such credential" from "not
        // allowed" is something to guess against.
        tracing::warn!(%peer, "the tools were reached with a credential this instance does not hold");
        return axum::http::StatusCode::FORBIDDEN.into_response();
    };

    match decode(&incoming.method, &incoming.params) {
        Call::Notification | Call::Ignored if incoming.id.is_none() => {
            axum::http::StatusCode::ACCEPTED.into_response()
        }
        Call::Notification | Call::Ignored => answered(incoming.id, serde_json::json!({})),
        Call::Greeting(protocol) => answered(
            incoming.id,
            serde_json::json!({
                "protocolVersion": protocol,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
            }),
        ),
        Call::Listing => {
            let Scope::Foreman(project) = scope;
            let agents = allowed_agents(&store.read(), project);
            answered(
                incoming.id,
                serde_json::json!({"tools": tools(scope, &agents)}),
            )
        }
        Call::NoSuchTool(named) => failed(
            incoming.id,
            &format!("this instance serves no tool called {named:?}"),
        ),
        Call::Starting(starting) => starting_a_job(&store, scope, &starting, incoming.id),
    }
}

/// Starts a job, or says why it could not be.
///
/// A refusal comes back as a *tool result* marked as an error rather than as a
/// protocol error, which is the distinction that matters to whoever reads it:
/// a protocol error tells the agent's own machinery something went wrong, and
/// a failed tool result tells the model, which is the thing able to pick a
/// different agent and try again.
#[mutants::skip]
fn starting_a_job(
    store: &std::sync::Arc<crate::Store>,
    scope: Scope,
    starting: &Starting,
    id: Option<serde_json::Value>,
) -> axum::response::Response {
    let Scope::Foreman(project) = scope;

    let Some(agent) = named_agent(&store.read(), project, &starting.agent) else {
        let allowed = allowed_agents(&store.read(), project).join(", ");
        tracing::warn!(%project, asked = %starting.agent, "no such agent for this project");
        return failed(
            id,
            &format!(
                "this project's jobs do not run on {:?}. It runs jobs on: {allowed}",
                starting.agent
            ),
        );
    };

    let started = match crate::begin(
        store,
        project,
        agent,
        &starting.reason,
        &starting.instructions,
    ) {
        Ok(started) => started,
        Err(why) => {
            tracing::warn!(%project, %why, "the job could not be recorded");
            return failed(id, "the job could not be recorded");
        }
    };
    let job = started.job();

    // Answered as soon as the job exists, and supervised on a task of its own:
    // a foreman waiting for a job to finish would be a foreman that cannot
    // answer anything else meanwhile, and jobs take minutes.
    let running = std::sync::Arc::clone(store);
    tokio::spawn(async move {
        drop(crate::supervise(&running, &crate::RUNTIME, started).await);
    });

    succeeded(id, &format!("started job {job}"))
}

/// One successful answer, in the protocol's envelope.
///
/// Assembled rather than written as a literal so that both halves are moved
/// into it: a literal borrows them, which leaves this taking two owned values
/// it never consumes.
#[mutants::skip]
fn answered(id: Option<serde_json::Value>, result: serde_json::Value) -> axum::response::Response {
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "jsonrpc".to_owned(),
        serde_json::Value::String("2.0".to_owned()),
    );
    // Null rather than absent: a caller that sent no identifier is answering
    // nothing, and the protocol spells that as an explicit null.
    envelope.insert("id".to_owned(), id.unwrap_or(serde_json::Value::Null));
    envelope.insert("result".to_owned(), result);
    axum::Json(serde_json::Value::Object(envelope)).into_response()
}

/// A tool that ran and produced text.
#[mutants::skip]
fn succeeded(id: Option<serde_json::Value>, said: &str) -> axum::response::Response {
    answered(
        id,
        serde_json::json!({"content": [{"type": "text", "text": said}]}),
    )
}

/// A tool that could not do what was asked, told to the model rather than to
/// its machinery.
#[mutants::skip]
fn failed(id: Option<serde_json::Value>, why: &str) -> axum::response::Response {
    answered(
        id,
        serde_json::json!({
            "content": [{"type": "text", "text": why}],
            "isError": true,
        }),
    )
}

/// The agent a foreman named, if this project's jobs may run on it.
///
/// Answers `None` for an agent this instance does not run *and* for one it
/// runs but this project does not allow — which is a refusal rather than a
/// substitution, because silently running a different agent than the one asked
/// for is a wrong answer that looks like a right one.
pub fn named_agent(
    state: &stageman_core::State,
    project: stageman_core::ProjectId,
    named: &str,
) -> Option<stageman_core::Agent> {
    state
        .projects
        .get(&project)?
        .job_agents
        .iter()
        .find(|agent| crate::dashboard::wire_name(**agent).0 == named)
        .copied()
}

/// What this project's jobs may run on, as a foreman names them.
///
/// Said back with a refusal, so a foreman that guessed wrong is told what it
/// could have said rather than only that it was wrong.
pub fn allowed_agents(
    state: &stageman_core::State,
    project: stageman_core::ProjectId,
) -> Vec<&'static str> {
    state
        .projects
        .get(&project)
        .map_or_else(Vec::new, |watched| {
            watched
                .job_agents
                .iter()
                .map(|agent| crate::dashboard::wire_name(*agent).0)
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::{
        Call, PROTOCOL, Scope, Starting, allowed_agents, axum, decode, endpoint, named_agent,
        presented, tools,
    };
    use stageman_core::{ProjectId, Uuid};

    fn a_project() -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(1))
    }

    /// A handshake answers in the caller's own version when it named one.
    #[test]
    fn a_greeting_echoes_the_version_it_was_given() {
        assert_eq!(
            decode(
                "initialize",
                &serde_json::json!({"protocolVersion": "2025-11-25"})
            ),
            Call::Greeting("2025-11-25".to_owned()),
        );
        assert_eq!(
            decode("initialize", &serde_json::json!({})),
            Call::Greeting(PROTOCOL.to_owned()),
            "a caller that names no version still gets a handshake",
        );
    }

    /// Anything needing no answer is recognised before anything else.
    #[test]
    fn a_notification_wants_no_answer() {
        for method in ["notifications/initialized", "notifications/cancelled"] {
            assert_eq!(decode(method, &serde_json::json!({})), Call::Notification);
        }
    }

    /// A method this instance does not implement is ignored, not refused.
    ///
    /// Measured rather than supposed: an observed client opens by asking for
    /// `server/discover`, which is in no specification. Refusing it would fail
    /// a handshake over a question nothing needed answered.
    #[test]
    fn a_method_we_do_not_serve_is_ignored_rather_than_refused() {
        assert_eq!(
            decode("server/discover", &serde_json::json!({})),
            Call::Ignored,
        );
        assert_eq!(
            decode("resources/list", &serde_json::json!({})),
            Call::Ignored
        );
    }

    /// A call carries its arguments through, and names an unknown tool.
    #[test]
    fn a_call_is_read_as_its_arguments_or_as_no_such_tool() {
        assert_eq!(
            decode(
                "tools/call",
                &serde_json::json!({
                    "name": "start_job",
                    "arguments": {
                        "reason": "the tests are red on main",
                        "instructions": "find out why and open a pull request",
                        "agent": "claude",
                    },
                }),
            ),
            Call::Starting(Starting {
                reason: "the tests are red on main".to_owned(),
                instructions: "find out why and open a pull request".to_owned(),
                agent: "claude".to_owned(),
            }),
        );

        assert_eq!(
            decode("tools/call", &serde_json::json!({"name": "say"})),
            Call::NoSuchTool("say".to_owned()),
            "a tool this instance does not serve yet is named back",
        );
        assert_eq!(
            decode("tools/call", &serde_json::json!({})),
            Call::NoSuchTool(String::new()),
            "and naming nothing is not naming the only one",
        );

        // An absent argument is carried as empty rather than refused here, so
        // that one impossible request has one refusal rather than two.
        assert_eq!(
            decode("tools/call", &serde_json::json!({"name": "start_job"})),
            Call::Starting(Starting {
                reason: String::new(),
                instructions: String::new(),
                agent: String::new(),
            }),
        );
    }

    /// A foreman is offered the tool that starts jobs, and its agents by name.
    #[test]
    fn what_a_foreman_is_offered_names_the_agents_it_may_choose() {
        let offered = tools(Scope::Foreman(a_project()), &["claude"]);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].name, "start_job");

        let agent = &offered[0].schema["properties"]["agent"];
        assert_eq!(
            agent["enum"],
            serde_json::json!(["claude"]),
            "the closed set is in the schema, so a foreman picks rather than guesses",
        );

        let required = &offered[0].schema["required"];
        assert_eq!(
            required,
            &serde_json::json!(["reason", "instructions", "agent"])
        );
    }

    /// A project with no agents gets a plain string, not an empty enumeration.
    ///
    /// An enumeration no value satisfies reads to a model as a broken tool. A
    /// plain string reaches the refusal that can say what is wrong.
    #[test]
    fn no_agents_omits_the_enumeration_rather_than_emitting_an_empty_one() {
        let offered = tools(Scope::Foreman(a_project()), &[]);
        let agent = &offered[0].schema["properties"]["agent"];
        assert_eq!(agent["type"], "string");
        assert!(agent.get("enum").is_none(), "no enumeration at all");
    }

    /// The endpoint really answers over HTTP, and only to a warrant it holds.
    ///
    /// Served through the same function the daemon serves with, so the route
    /// being registered at all is part of what this covers — the interesting
    /// half above is pure and would pass just as well if nothing mounted it.
    ///
    /// No container, no image and no credential, so this runs in the gate
    /// rather than beside it. What it cannot cover is whether a real agent
    /// likes these answers; that needs the session declaration naming this
    /// endpoint, which does not exist yet.
    #[tokio::test]
    async fn the_endpoint_answers_a_warrant_it_holds_and_refuses_anything_else() {
        use stageman_core::{
            Agent, AgentConfig, Attending, Project, ProjectId, Secret, State, Uuid,
        };
        use std::collections::{BTreeMap, BTreeSet};

        const WARRANT: &str = "a-warrant-that-is-not-a-real-secret";

        let mut state = State::default();
        state.agents.insert(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new("not-a-real-token".to_owned()),
            },
        );
        state.projects.insert(
            ProjectId::from_uuid(Uuid::from_u128(11)),
            Project {
                name: "aviary".to_owned(),
                repository: "https://example.invalid/aviary".to_owned(),
                foreman_agent: Agent::Claude,
                job_agents: BTreeSet::from([Agent::Claude]),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                jobs: BTreeMap::new(),
                warrant: Some(Secret::new(WARRANT.to_owned())),
                attending: Attending::default(),
            },
        );

        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = std::sync::Arc::new(
            crate::Store::create(
                directory.path().join("state.json"),
                stageman_core::Key::new([3; 32]),
                state,
            )
            .expect("it can write"),
        );

        // Port zero, so this never contends with a running instance or with
        // another test — the same reason the endpoint honours it at all.
        let listening = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a port");
        let port = listening.local_addr().expect("a bound address").port();
        let serving = tokio::spawn(crate::asking::serve(
            listening,
            std::sync::Arc::clone(&store),
        ));

        let url = format!("http://127.0.0.1:{port}/mcp");
        let client = reqwest::Client::new();
        let ask = async |warrant: &str, body: serde_json::Value| {
            client
                .post(&url)
                .header("Authorization", format!("Bearer {warrant}"))
                .json(&body)
                .send()
                .await
                .expect("the endpoint answers")
        };

        let greeting = ask(
            WARRANT,
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-11-25"},
            }),
        )
        .await;
        assert!(greeting.status().is_success());
        let greeting: serde_json::Value = greeting.json().await.expect("a JSON answer");
        assert_eq!(greeting["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(greeting["result"]["serverInfo"]["name"], "stageman");
        assert_eq!(greeting["id"], 1, "an answer names what it answers");

        let listed = ask(
            WARRANT,
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .await;
        let listed: serde_json::Value = listed.json().await.expect("a JSON answer");
        assert_eq!(listed["result"]["tools"][0]["name"], "start_job");
        assert_eq!(
            listed["result"]["tools"][0]["inputSchema"]["properties"]["agent"]["enum"],
            serde_json::json!(["claude"]),
            "the project's own agents reach the schema, not a fixed list",
        );

        // The whole authorisation mechanism, exercised rather than reasoned
        // about: what is offered follows the credential, so a credential this
        // instance does not hold is offered nothing at all.
        let stranger = ask(
            "not-the-warrant",
            serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        )
        .await;
        assert_eq!(
            stranger.status(),
            reqwest::StatusCode::FORBIDDEN,
            "an unknown credential is refused without detail",
        );

        // A method in no specification is answered rather than refused, which
        // is what keeps an observed client's opening question from failing the
        // handshake.
        let odd = ask(
            WARRANT,
            serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "server/discover"}),
        )
        .await;
        assert!(odd.status().is_success());

        serving.abort();
    }

    /// The endpoint names the port it was given, and one hostname.
    #[test]
    fn the_endpoint_is_the_same_hostname_whichever_runtime() {
        assert_eq!(endpoint(9001), "http://host.docker.internal:9001/mcp");
        assert!(endpoint(1).starts_with("http://host.docker.internal:"));
    }

    /// Tests that spend real money and reach the network, grouped so a filter
    /// can name them. Run with `just image-session`.
    mod costs_a_credential {
        use stageman_core::{
            Agent, AgentConfig, Attending, Handout, Project, ProjectId, Secret, State, Uuid,
        };
        use std::collections::{BTreeMap, BTreeSet};

        /// What a real agent authenticates with.
        fn credential() -> Secret {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../.local/anthropic-token");
            let raw = std::fs::read_to_string(path)
                .expect("write an agent credential to .local/anthropic-token (it is gitignored)");
            Secret::new(raw.trim().to_owned())
        }

        fn located_runtime() -> stageman_agent::ContainerRuntime {
            stageman_agent::first_present(stageman_agent::candidates())
                .expect("a container runtime is installed")
        }

        /// A real agent is offered the tools this instance serves.
        ///
        /// **The test 0034 asks for, and the shape of it is the point.** An
        /// endpoint a container cannot reach does not fail: session setup
        /// succeeds in under a second and the agent simply has no tools, which
        /// reads exactly like an agent that chose not to use one. So asserting
        /// that a session started would pass on the failure this is for. It
        /// asserts the tool is *there*, by asking the model what it can see.
        ///
        /// Driven through `attend`, which is what the daemon calls, so the
        /// declaration being attached to a real session request is covered
        /// rather than assumed — and bound on every interface, because that is
        /// the only address a container reaches on every platform.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image, a credential and the network; run `just image-session`"]
        async fn a_real_agent_is_offered_the_tools_this_instance_serves() {
            const WARRANT: &str = "a-warrant-for-one-test";

            let runtime = located_runtime();
            let project = ProjectId::from_uuid(Uuid::from_u128(4242));
            let name = stageman_foreman::container(project);
            // A container of this name may survive an earlier failed run, and
            // `attend` would resume it rather than make the session this is
            // about.
            drop(stageman_agent::discard(&runtime, &name).await);

            let mut state = State::default();
            state.agents.insert(
                Agent::Claude,
                AgentConfig {
                    auth_token: credential(),
                },
            );
            state.projects.insert(
                project,
                Project {
                    name: "aviary".to_owned(),
                    repository: "https://example.invalid/aviary".to_owned(),
                    foreman_agent: Agent::Claude,
                    job_agents: BTreeSet::from([Agent::Claude]),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    warrant: Some(Secret::new(WARRANT.to_owned())),
                    attending: Attending::default(),
                },
            );
            let handout = Handout::for_foreman(&state, project).expect("a foreman's handout");

            let directory = tempfile::tempdir().expect("a temporary directory");
            let store = std::sync::Arc::new(
                crate::Store::create(
                    directory.path().join("state.json"),
                    stageman_core::Key::new([3; 32]),
                    state,
                )
                .expect("it can write"),
            );

            // Every interface, not loopback: a container reaches the host
            // through `host.docker.internal`, which does not resolve to
            // `127.0.0.1` on the host's own stack. Port zero so this never
            // contends with a running instance.
            let listening = tokio::net::TcpListener::bind(("0.0.0.0", 0))
                .await
                .expect("a port");
            let port = listening.local_addr().expect("a bound address").port();
            let serving = tokio::spawn(crate::asking::serve(
                listening,
                std::sync::Arc::clone(&store),
            ));

            let answer = stageman_foreman::attend(
                &runtime,
                &handout,
                project,
                "https://example.invalid/aviary",
                &super::super::endpoint(port),
                &[("claude", "a general-purpose coding agent")],
                "Do not start anything. List the names of every tool you have whose name \
                 contains 'stageman', exactly as they are spelled. If you have none, reply \
                 with exactly NO STAGEMAN TOOLS.",
            )
            .await;

            drop(stageman_agent::discard(&runtime, &name).await);
            serving.abort();

            let answer = answer.expect("the foreman answers");
            assert!(
                answer.text.contains("start_job"),
                "the agent was offered no tool it could name, which is what an endpoint it \
                 cannot reach looks like — it said: {:?}",
                answer.text,
            );
        }

        /// A foreman asked for work starts a job by calling the tool.
        ///
        /// The test for the half of 0034 that changed the prompt rather than
        /// the transport. Listing a tool proves it arrived; this proves a
        /// foreman told to use it does, and that what it chose reaches the
        /// instance as typed values rather than a shell command somebody
        /// parses back out.
        ///
        /// Deliberately asserts the *record*, not the answer. What a foreman
        /// says about what it did is a claim; a job in the instance is the
        /// thing that happened.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image, a credential and the network; run `just image-session`"]
        async fn a_foreman_starts_a_job_by_calling_the_tool() {
            const WARRANT: &str = "another-warrant-for-one-test";

            let runtime = located_runtime();
            let project = ProjectId::from_uuid(Uuid::from_u128(4243));
            let name = stageman_foreman::container(project);
            drop(stageman_agent::discard(&runtime, &name).await);

            let mut state = State::default();
            state.agents.insert(
                Agent::Claude,
                AgentConfig {
                    auth_token: credential(),
                },
            );
            state.projects.insert(
                project,
                Project {
                    name: "aviary".to_owned(),
                    repository: "https://example.invalid/aviary".to_owned(),
                    foreman_agent: Agent::Claude,
                    job_agents: BTreeSet::from([Agent::Claude]),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    warrant: Some(Secret::new(WARRANT.to_owned())),
                    attending: Attending::default(),
                },
            );
            let handout = Handout::for_foreman(&state, project).expect("a foreman's handout");

            let directory = tempfile::tempdir().expect("a temporary directory");
            let store = std::sync::Arc::new(
                crate::Store::create(
                    directory.path().join("state.json"),
                    stageman_core::Key::new([3; 32]),
                    state,
                )
                .expect("it can write"),
            );

            let listening = tokio::net::TcpListener::bind(("0.0.0.0", 0))
                .await
                .expect("a port");
            let port = listening.local_addr().expect("a bound address").port();
            let serving = tokio::spawn(crate::asking::serve(
                listening,
                std::sync::Arc::clone(&store),
            ));

            let answer = stageman_foreman::attend(
                &runtime,
                &handout,
                project,
                "https://example.invalid/aviary",
                &super::super::endpoint(port),
                &[("claude", "a general-purpose coding agent")],
                "The README has a broken link in it. Please get that fixed.",
            )
            .await;

            let started: Vec<_> = store
                .read()
                .projects
                .get(&project)
                .expect("the project")
                .jobs
                .values()
                .map(|job| (job.reason.clone(), job.kickoff.clone()))
                .collect();

            // Both containers, whatever happened: the foreman's, and whichever
            // job it started. A test that leaks one leaks it on every run.
            drop(stageman_agent::discard(&runtime, &name).await);
            for left in stageman_agent::abandoned(&runtime)
                .await
                .unwrap_or_default()
            {
                drop(stageman_agent::discard(&runtime, &left).await);
            }
            serving.abort();

            let answer = answer.expect("the foreman answers");
            assert_eq!(
                started.len(),
                1,
                "a foreman asked for work started no job; it said: {:?}",
                answer.text,
            );
            let (reason, kickoff) = started.first().expect("the one job");
            assert!(
                !reason.trim().is_empty(),
                "the reason a person reads on the dashboard is empty",
            );
            assert!(
                kickoff.to_lowercase().contains("readme")
                    || kickoff.to_lowercase().contains("link"),
                "the instruction does not carry what the foreman was asked for: {kickoff:?}",
            );
        }
    }

    /// An agent a project's jobs may not run on is refused, not substituted.
    ///
    /// Silently running a different agent than the one asked for is a wrong
    /// answer that looks like a right one — the job would run, report success,
    /// and have been done by something the foreman did not choose.
    /// `docs/decisions/0006-agents-are-pluggable.md` makes the choice the
    /// foreman's, so overriding it here would take back a decision that record
    /// gave away.
    #[test]
    fn an_agent_must_be_named_and_allowed_or_it_is_refused() {
        use stageman_core::{Agent, ProjectId, State, Uuid};

        let mut state = State::default();
        let project = ProjectId::from_uuid(Uuid::from_u128(1));
        state.projects.insert(project, watched_by([Agent::Claude]));

        assert_eq!(named_agent(&state, project, "claude"), Some(Agent::Claude));
        assert_eq!(
            named_agent(&state, project, "gpt"),
            None,
            "an agent this instance does not run is a refusal, not a substitution"
        );
        assert_eq!(
            named_agent(&state, project, ""),
            None,
            "and naming nothing is not naming the first"
        );
        assert_eq!(
            named_agent(&state, ProjectId::from_uuid(Uuid::from_u128(9)), "claude"),
            None,
            "a project this instance does not watch has no agents"
        );

        // A refusal says what could have been said instead, so a foreman that
        // guessed wrong learns the set rather than only that it was wrong.
        assert_eq!(allowed_agents(&state, project), vec!["claude"]);
        assert!(allowed_agents(&state, ProjectId::from_uuid(Uuid::from_u128(9))).is_empty());
    }

    /// A project running jobs on exactly these agents.
    fn watched_by<const N: usize>(agents: [stageman_core::Agent; N]) -> stageman_core::Project {
        use std::collections::{BTreeMap, BTreeSet};

        stageman_core::Project {
            name: "aviary".to_owned(),
            repository: "https://example.invalid/aviary".to_owned(),
            foreman_agent: stageman_core::Agent::Claude,
            job_agents: BTreeSet::from(agents),
            credentials: BTreeMap::new(),
            channels: BTreeMap::new(),
            warrant: None,
            attending: stageman_core::Attending::default(),
            jobs: BTreeMap::new(),
        }
    }

    /// A credential is read from a bearer header and nothing else.
    #[test]
    fn only_a_bearer_credential_is_read() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(presented(&headers), None, "no header at all");

        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer a-warrant"),
        );
        assert_eq!(presented(&headers), Some("a-warrant"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Basic a-warrant"),
        );
        assert_eq!(presented(&headers), None, "another scheme is not a bearer");
    }
}
