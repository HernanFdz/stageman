//! This application's half of the world: what performs its own effects, and
//! how whoever needs an answer waits for it.
//!
//! The loop, the channel and the generic mechanisms are the world crate's.
//! What is here is what only stageman knows how to perform — a turn, a
//! probe, a container's fate, a channel's posting — and the one thing the
//! generic world cannot do for it: match an answer to whoever asked. A server
//! function, the tools endpoint and the tunnel layer each send an event
//! carrying an identifier and wait for the effect that carries it back, and
//! the map below is where they wait. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Everything here runs on a task of its own: a turn takes minutes, and
//! `docs/conventions.md` §3 keeps that off the loop that answers the
//! dashboard. The disk is the world crate's, written inline there.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use stageman_agent::ContainerRuntime;
use stageman_core::{Channel, JobId, Secret, Speaking, Timestamp};
use stageman_instance::{
    AppEffect, AppEvent, Event, Request, RequestId, Response, Run, Speaker, Stageman,
};
use stageman_world::{Perform, World};

/// The application's way in, once an instance has been opened.
///
/// A process-wide value, because the things that send to it — a server
/// function, a request on the tools endpoint, the tunnel layer — are handed
/// nothing by anybody: each is called by a framework with a request and
/// nothing else. One process operates one instance, so this is even true.
static ASKING: OnceLock<Arc<Asking>> = OnceLock::new();

/// The way in, if an instance has been opened in this process.
#[must_use]
pub fn asking() -> Option<&'static Arc<Asking>> {
    ASKING.get()
}

/// Makes a way in the one this process uses.
///
/// Once. A second is a fault in startup rather than a request to honour, and
/// is said rather than silently replaced.
pub fn adopt(asking: Arc<Asking>) {
    if ASKING.set(asking).is_err() {
        tracing::error!("this process already has a way in; the second was not adopted");
    }
}

/// Where a job's tunnel is, as the instance answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Located {
    /// Forward to this host port.
    At(u16),
    /// Nothing answers on that name.
    Nowhere,
}

/// Somebody waiting for an answer, by what they asked.
enum Answering {
    /// A person, through a server function.
    Person(tokio::sync::oneshot::Sender<Response>),
    /// An agent, through the tools endpoint.
    Tool(tokio::sync::oneshot::Sender<(u16, Option<serde_json::Value>)>),
    /// A browser, through the tunnel layer.
    Tunnel(tokio::sync::oneshot::Sender<Located>),
}

/// What the instance answered, by what was asked.
enum Answered {
    Person(Response),
    Tool(u16, Option<serde_json::Value>),
    Tunnel(Located),
}

/// Sends the instance events, and matches its answers to whoever asked.
pub struct Asking {
    /// The loop's way in.
    world: Arc<World<Stageman>>,
    /// The next request identifier. Never reused while this process runs.
    next: AtomicU64,
    /// Everyone waiting on an answer, by the identifier it will carry.
    waiting: parking_lot::Mutex<BTreeMap<RequestId, Answering>>,
}

impl Asking {
    /// A way in to this world.
    #[must_use]
    pub fn new(world: Arc<World<Stageman>>) -> Arc<Self> {
        Arc::new(Self {
            world,
            next: AtomicU64::new(1),
            waiting: parking_lot::Mutex::new(BTreeMap::new()),
        })
    }

    /// Tells the instance something happened.
    pub fn send(&self, event: impl Into<Event>) {
        self.world.send(event);
    }

    /// A request identifier nothing else in this process has.
    fn minted(&self) -> RequestId {
        RequestId(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Asks the instance something on a person's behalf.
    ///
    /// `None` if the instance stopped before answering, which is a fault in
    /// this process rather than a refusal.
    pub async fn ask(&self, request: Request) -> Option<Response> {
        let (answer, waiting) = tokio::sync::oneshot::channel();
        let id = self.minted();
        self.waiting.lock().insert(id, Answering::Person(answer));
        self.send(AppEvent::Request { id, request });
        waiting.await.ok()
    }

    /// Hands the instance a call on the tools endpoint, whole.
    ///
    /// The status and the body the endpoint answers with, or `None` if the
    /// instance stopped before answering.
    pub async fn call(
        &self,
        at: Timestamp,
        nearby: bool,
        bearer: Option<String>,
        body: serde_json::Value,
    ) -> Option<(u16, Option<serde_json::Value>)> {
        let (answer, waiting) = tokio::sync::oneshot::channel();
        let id = self.minted();
        self.waiting.lock().insert(id, Answering::Tool(answer));
        self.send(AppEvent::ToolCalled {
            id,
            at,
            nearby,
            bearer,
            body,
        });
        waiting.await.ok()
    }

    /// Asks where a job's tunnel is.
    pub async fn tunnel(&self, job: JobId) -> Option<Located> {
        let (answer, waiting) = tokio::sync::oneshot::channel();
        let id = self.minted();
        self.waiting.lock().insert(id, Answering::Tunnel(answer));
        self.send(AppEvent::TunnelAsked { id, job });
        waiting.await.ok()
    }

    /// Delivers an answer to whoever asked.
    ///
    /// Nobody waiting is not an error: a browser that gave up has already
    /// dropped its end, and the answer has nowhere to go.
    fn answered(&self, id: RequestId, answered: Answered) {
        let Some(waiting) = self.waiting.lock().remove(&id) else {
            tracing::debug!(?id, "an answer arrived for a request nobody is waiting on");
            return;
        };
        match (waiting, answered) {
            (Answering::Person(reply), Answered::Person(response)) => drop(reply.send(response)),
            (Answering::Tool(reply), Answered::Tool(status, body)) => {
                drop(reply.send((status, body)));
            }
            (Answering::Tunnel(reply), Answered::Tunnel(located)) => drop(reply.send(located)),
            _ => tracing::error!(
                ?id,
                "the instance answered a request with the wrong kind of answer"
            ),
        }
    }
}

/// What performs this application's own effects.
///
/// Cheap to clone and handed whole to every task it spawns, which is why the
/// state sits behind one shared inner value.
#[derive(Clone)]
pub struct Performer(Arc<Inner>);

/// The runtime every container effect goes through, once the instance has
/// found one and said so.
///
/// A static rather than a field, for the reason the domain beside it is one:
/// what borrows it is a task that outlives the call that spawned it, and a
/// process has exactly one running instance to have found one. On its way
/// out either way — every effect that needs a runtime will carry its path.
static RUNTIME: OnceLock<ContainerRuntime> = OnceLock::new();

struct Inner {
    /// Where a container reaches the tools this instance serves.
    endpoint: String,
    /// Where events go back, and where answers are matched to askers.
    asking: Arc<Asking>,
    /// The turns running right now, by whose they are, each with the handle
    /// that stops it. In memory and never written down: a turn does not
    /// survive this process, so neither should anything about one.
    turns: parking_lot::Mutex<BTreeMap<Speaker, Arc<tokio::sync::Notify>>>,
}

impl Performer {
    /// A performer for this way in.
    #[must_use]
    pub fn new(endpoint: String, asking: Arc<Asking>) -> Self {
        Self(Arc::new(Inner {
            endpoint,
            asking,
            turns: parking_lot::Mutex::new(BTreeMap::new()),
        }))
    }

    /// Performs an effect on a task of its own, with the runtime the
    /// instance found; one asked for before that is a fault in the instance
    /// and is said rather than performed.
    ///
    /// Skipped by mutation testing, like the performer it serves: what it
    /// does is spawn, and everything decided is inside the effect that task
    /// performs, which cannot run without a container runtime.
    #[mutants::skip]
    fn spawn<F, Fut>(&self, effect: F)
    where
        F: FnOnce(Arc<Inner>, &'static ContainerRuntime) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let Some(runtime) = RUNTIME.get() else {
            tracing::error!("asked to drive a runtime before the instance found one");
            return;
        };
        drop(tokio::spawn(effect(Arc::clone(&self.0), runtime)));
    }

    /// Performs an effect on a task of its own, needing no runtime.
    ///
    /// Skipped for the reason [`Performer::spawn`] is: it spawns, and what
    /// the task does reaches a channel over the network.
    #[mutants::skip]
    fn spawn_plain<F, Fut>(&self, effect: F)
    where
        F: FnOnce(Arc<Inner>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let inner = Arc::clone(&self.0);
        drop(tokio::spawn(effect(inner)));
    }
}

impl Perform<Stageman> for Performer {
    /// Performs one of the application's own effects, on a task of its own.
    ///
    /// Skipped by mutation testing, like everything here that drives the
    /// runtime or the network: every arm performs an effect and decides
    /// nothing a test could check without one.
    #[mutants::skip]
    async fn perform(&self, effect: AppEffect) {
        match effect {
            AppEffect::Booted { runtime, domain } => {
                if RUNTIME.set(ContainerRuntime::new(runtime)).is_err() {
                    tracing::error!("the instance booted twice; the second runtime is ignored");
                }
                crate::tunnel::adopt(&domain);
            }
            AppEffect::RunTurn { speaker, run } => self.turn(speaker, run),
            AppEffect::StopTurn { speaker } => {
                // A permit rather than a wake-up, so that a stop arriving in
                // the instant between the turn starting and it waiting is not
                // dropped on the floor.
                if let Some(stopping) = self.0.turns.lock().get(&speaker) {
                    stopping.notify_one();
                }
            }
            AppEffect::Probe { job } => self.spawn(move |inner, runtime| async move {
                let answering = stageman_job::answering(runtime, job).await;
                inner.asking.send(AppEvent::Probed { job, answering });
            }),
            AppEffect::Say {
                speaking,
                thread,
                text,
            } => self.spawn_plain(move |_| async move {
                let speaking: Speaking = speaking.into();
                if let Err(why) = crate::channel::say_in(&speaking, &thread, &text).await {
                    tracing::warn!(%why, "the thread could not be spoken to");
                }
            }),
            AppEffect::OpenThread {
                job,
                speaking,
                announcement,
            } => self.spawn_plain(move |inner| async move {
                let speaking: Speaking = speaking.into();
                let outcome = crate::channel::open_thread(&speaking, Channel::Slack, &announcement)
                    .await
                    .map_err(|why| why.to_string());
                inner.asking.send(AppEvent::ThreadOpened { job, outcome });
            }),
            AppEffect::Post {
                request,
                speaking,
                thread,
                text,
            } => self.spawn_plain(move |inner| async move {
                let speaking: Speaking = speaking.into();
                let outcome = crate::channel::say_in(&speaking, &thread, &text)
                    .await
                    .map_err(|why| why.to_string());
                inner.asking.send(AppEvent::Posted { request, outcome });
            }),
            AppEffect::Listen {
                project,
                opening,
                speaking,
            } => crate::listening::listen_to(
                Arc::clone(&self.0.asking),
                crate::listening::Listening {
                    project,
                    opening: Secret::new(opening),
                    speaking: speaking.into(),
                },
            ),
            AppEffect::Respond { id, response } => {
                self.0.asking.answered(id, Answered::Person(response));
            }
            AppEffect::ToolAnswered { id, status, body } => {
                self.0.asking.answered(id, Answered::Tool(status, body));
            }
            AppEffect::Route { id, port } => self.0.asking.answered(
                id,
                Answered::Tunnel(port.map_or(Located::Nowhere, Located::At)),
            ),
        }
    }
}

impl Performer {
    /// Runs one turn on a task of its own, and reports how it ended.
    ///
    /// **Asking rather than killing** is what stopping means: the future
    /// running the agent is dropped, which closes the pipe the agent is
    /// speaking on and ends it, and the container carries on because the
    /// agent is no longer what it runs — see
    /// `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`.
    #[mutants::skip]
    fn turn(&self, speaker: Speaker, run: Run) {
        let stopping = Arc::new(tokio::sync::Notify::new());
        self.0.turns.lock().insert(speaker, Arc::clone(&stopping));
        self.spawn(move |inner, runtime| async move {
            let tools = |warrant: String| {
                stageman_agent::Tools::new(inner.endpoint.clone(), Secret::new(warrant))
            };
            let running = async {
                match run {
                    Run::Begin {
                        container,
                        instance,
                        agent,
                        role,
                        environment,
                        repository,
                        platform,
                        kit,
                        warrant,
                        kickoff,
                    } => {
                        let launch = stageman_agent::Launch {
                            agent,
                            role,
                            environment: environment
                                .into_iter()
                                .map(|(name, value)| (name, Secret::new(value)))
                                .collect(),
                            repository,
                            platform,
                            kit,
                        };
                        stageman_agent::begin(
                            runtime,
                            &launch,
                            &container,
                            instance,
                            Some(&tools(warrant)),
                            &kickoff,
                        )
                        .await
                    }
                    Run::Resume {
                        container,
                        kit,
                        warrant,
                        text,
                    } => {
                        stageman_agent::resume(
                            runtime,
                            &container,
                            &kit,
                            Some(&tools(warrant)),
                            &text,
                        )
                        .await
                    }
                }
            };
            let outcome = tokio::select! {
                answered = running => answered.map_err(|why| because(&why)),
                () = stopping.notified() => Err("stopped".to_owned()),
            };
            inner.turns.lock().remove(&speaker);
            inner.asking.send(AppEvent::TurnEnded { speaker, outcome });
        });
    }
}

/// A failure and everything underneath it, as one line of prose.
///
/// `to_string` on an error renders only its outermost line, and every error
/// that reaches a job's record wraps a more specific one — so recording the
/// outer line alone throws away the only part that says what actually went
/// wrong. One line rather than several, because this goes into a record a
/// dashboard shows as prose.
fn because(failure: &dyn std::error::Error) -> String {
    let mut told = failure.to_string();
    let mut cause = std::error::Error::source(failure);
    while let Some(reason) = cause {
        told.push_str(": ");
        told.push_str(&reason.to_string());
        cause = reason.source();
    }
    told
}

#[cfg(test)]
mod tests {
    use super::{Answered, Asking, Located, because};
    use stageman_instance::{AppEvent, Event, Request, Response};
    use stageman_world::World;
    use std::sync::Arc;
    use std::time::Duration;

    /// A bounded wait, because the failure these look for is an answer that
    /// never arrives — which does not error, it hangs.
    async fn soon<T>(waiting: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("an answer within the wait")
    }

    /// Every kind of request is answered by the effect carrying its
    /// identifier, and by nothing else.
    #[tokio::test]
    async fn a_request_is_answered_by_the_effect_carrying_its_identifier() {
        let (world, mut events) = World::new();
        let asking = Asking::new(world);
        let job = stageman_core::JobId::from_uuid(stageman_core::Uuid::from_u128(7));

        let asked = {
            let asking = Arc::clone(&asking);
            tokio::spawn(async move { asking.ask(Request::Instance).await })
        };
        let Some(Event::App(AppEvent::Request {
            id,
            request: Request::Instance,
        })) = soon(events.recv()).await
        else {
            panic!("the request, as an event");
        };
        asking.answered(id, Answered::Person(Response::Agents(Vec::new())));
        assert_eq!(
            soon(asked).await.expect("the task"),
            Some(Response::Agents(Vec::new()))
        );

        let calling = {
            let asking = Arc::clone(&asking);
            tokio::spawn(async move {
                asking
                    .call(
                        stageman_core::Timestamp::UNIX_EPOCH,
                        true,
                        Some("not-a-real-credential".to_owned()),
                        serde_json::json!({"method": "ping"}),
                    )
                    .await
            })
        };
        let Some(Event::App(AppEvent::ToolCalled {
            id, nearby, bearer, ..
        })) = soon(events.recv()).await
        else {
            panic!("the call, as an event");
        };
        assert!(nearby);
        assert_eq!(bearer.as_deref(), Some("not-a-real-credential"));
        asking.answered(id, Answered::Tool(202, None));
        assert_eq!(soon(calling).await.expect("the task"), Some((202, None)));

        let locating = {
            let asking = Arc::clone(&asking);
            tokio::spawn(async move { asking.tunnel(job).await })
        };
        let Some(Event::App(AppEvent::TunnelAsked { id, job: asked })) = soon(events.recv()).await
        else {
            panic!("the question, as an event");
        };
        assert_eq!(asked, job);
        asking.answered(id, Answered::Tunnel(Located::At(4242)));
        assert_eq!(
            soon(locating).await.expect("the task"),
            Some(Located::At(4242))
        );
    }

    /// An answer of the wrong kind reaches nobody, and the one waiting is
    /// told there is no answer rather than left waiting.
    #[tokio::test]
    async fn an_answer_of_the_wrong_kind_answers_nobody() {
        let (world, mut events) = World::new();
        let asking = Asking::new(world);
        let asked = {
            let asking = Arc::clone(&asking);
            tokio::spawn(async move { asking.ask(Request::Agents).await })
        };
        let Some(Event::App(AppEvent::Request { id, .. })) = soon(events.recv()).await else {
            panic!("the request, as an event");
        };
        asking.answered(id, Answered::Tunnel(Located::Nowhere));
        assert_eq!(soon(asked).await.expect("the task"), None);
    }

    /// The chain is what there is to read.
    #[test]
    fn a_failure_is_recorded_with_everything_underneath_it() {
        let inner = std::io::Error::other("the disk is full");
        let outer = stageman_agent::AgentError::Runtime {
            path: std::path::PathBuf::from("/usr/local/bin/docker"),
            source: inner,
        };
        let told = because(&outer);
        assert!(told.contains("the disk is full"), "{told}");
        assert!(told.contains(": "), "{told}");
    }
}
