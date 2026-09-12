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
//! **The disk is written inline and everything else on a task.** A write is
//! the one effect the instance waits on before anything outward-facing, so
//! it is performed where the loop can await it and answered in order; a turn
//! takes minutes, and `docs/conventions.md` §3 keeps that off the loop that
//! answers the dashboard.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use stageman_agent::ContainerRuntime;
use stageman_core::{Channel, JobId, Secret, Speaking, Timestamp};
use stageman_instance::{
    AppEffect, AppEvent, Container, Event, Request, RequestId, Response, Run, Speaker, Stageman,
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

struct Inner {
    /// The runtime every container effect goes through.
    runtime: &'static ContainerRuntime,
    /// Where the instance is kept.
    path: PathBuf,
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
    /// A performer for this runtime, this file and this way in.
    #[must_use]
    pub fn new(
        runtime: &'static ContainerRuntime,
        path: PathBuf,
        endpoint: String,
        asking: Arc<Asking>,
    ) -> Self {
        Self(Arc::new(Inner {
            runtime,
            path,
            endpoint,
            asking,
            turns: parking_lot::Mutex::new(BTreeMap::new()),
        }))
    }

    /// Performs an effect on a task of its own.
    fn spawn<F, Fut>(&self, effect: F)
    where
        F: FnOnce(Arc<Inner>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let inner = Arc::clone(&self.0);
        drop(tokio::spawn(effect(inner)));
    }
}

impl Perform<Stageman> for Performer {
    /// Performs one effect: on a task of its own, or inline where the
    /// instance waits on the answer before doing anything else.
    ///
    /// Skipped by mutation testing, like everything here that drives the
    /// runtime or the network: every arm performs an effect and decides
    /// nothing a test could check without one.
    #[mutants::skip]
    async fn perform(&self, effect: AppEffect) {
        match effect {
            AppEffect::Persist { bytes } => self.persist(bytes.into_inner()).await,
            AppEffect::RunTurn { speaker, run } => self.turn(speaker, run),
            AppEffect::StopTurn { speaker } => {
                // A permit rather than a wake-up, so that a stop arriving in
                // the instant between the turn starting and it waiting is not
                // dropped on the floor.
                if let Some(stopping) = self.0.turns.lock().get(&speaker) {
                    stopping.notify_one();
                }
            }
            AppEffect::Probe { job } => self.probe(job),
            AppEffect::ListRunning => self.list_running(),
            AppEffect::Inspect { container } => self.inspect(container),
            AppEffect::Halt { container } => self.halt(container),
            AppEffect::Discard { container } => self.discard(container),
            AppEffect::Reclaim => self.reclaim(),
            AppEffect::Wake { after, timer } => self.spawn(move |inner| async move {
                tokio::time::sleep(after).await;
                inner.asking.send(AppEvent::Woke { timer });
            }),
            AppEffect::Say {
                speaking,
                thread,
                text,
            } => self.spawn(move |_| async move {
                let speaking: Speaking = speaking.into();
                if let Err(why) = crate::channel::say_in(&speaking, &thread, &text).await {
                    tracing::warn!(%why, "the thread could not be spoken to");
                }
            }),
            AppEffect::OpenThread {
                job,
                speaking,
                announcement,
            } => self.spawn(move |inner| async move {
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
            } => self.spawn(move |inner| async move {
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
            AppEffect::FindPort { job } => self.find_port(job),
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
    /// Asks whether a job's tunnel is answering.
    #[mutants::skip]
    fn probe(&self, job: JobId) {
        self.spawn(move |inner| async move {
            let answering = stageman_job::answering(inner.runtime, job).await;
            inner.asking.send(AppEvent::Probed { job, answering });
        });
    }

    /// Asks which containers are running, and whose each is.
    #[mutants::skip]
    fn list_running(&self) {
        self.spawn(|inner| async move {
            let names = match stageman_agent::running(inner.runtime).await {
                Ok(names) => names,
                Err(why) => {
                    tracing::warn!(%why, "could not ask which containers are running");
                    return;
                }
            };
            let mut running = Vec::with_capacity(names.len());
            for name in names {
                running.push(described(inner.runtime, name, true).await);
            }
            inner.asking.send(AppEvent::Listed { running });
        });
    }

    /// Asks whether a container exists, and which agent it was made for.
    ///
    /// A container that is not there refuses, and so does a runtime that will
    /// not answer; either way nothing can be resumed in it.
    #[mutants::skip]
    fn inspect(&self, container: String) {
        self.spawn(move |inner| async move {
            let (present, agent) = stageman_agent::made_for(inner.runtime, &container)
                .await
                .map_or((false, None), |agent| (true, agent));
            inner.asking.send(AppEvent::Inspected {
                container,
                present,
                agent,
            });
        });
    }

    /// Stops a container, keeping it.
    #[mutants::skip]
    fn halt(&self, container: String) {
        self.spawn(move |inner| async move {
            if let Err(why) = stageman_agent::halt(inner.runtime, &container).await {
                tracing::warn!(%container, %why, "a container could not be stopped");
            }
        });
    }

    /// Removes a container and everything in it.
    ///
    /// Not fatal, and deliberately not retried here: what is left is a
    /// container nothing needs, which is exactly what waking looks for.
    #[mutants::skip]
    fn discard(&self, container: String) {
        self.spawn(move |inner| async move {
            if let Err(why) = stageman_agent::discard(inner.runtime, &container).await {
                tracing::warn!(%container, %why, "a container could not be removed; waking will try again");
            }
        });
    }

    /// Reclaims the images nothing needs.
    ///
    /// Housekeeping rather than work, so a runtime that will not answer is
    /// warned about and nothing else changes.
    #[mutants::skip]
    fn reclaim(&self) {
        self.spawn(|inner| async move {
            match stageman_agent::reclaim(inner.runtime).await {
                Ok(gone) if gone > 0 => {
                    tracing::info!(images = gone, "reclaimed images no container needed");
                }
                Ok(_) => {}
                Err(why) => tracing::warn!(%why, "could not reclaim the images nothing is using"),
            }
        });
    }

    /// Asks the runtime where a job's tunnel is published.
    #[mutants::skip]
    fn find_port(&self, job: JobId) {
        self.spawn(move |inner| async move {
            let port = stageman_agent::tunnel_port(inner.runtime, &stageman_job::container(job))
                .await
                .unwrap_or_else(|why| {
                    tracing::debug!(%job, %why, "the runtime could not say where a job's tunnel is");
                    None
                });
            inner.asking.send(AppEvent::PortFound { job, port });
        });
    }

    /// Writes the instance, and says whether it landed.
    ///
    /// Inline rather than on a task, and awaited: writes are answered in the
    /// order they were asked for, and nothing outward-facing happens on the
    /// strength of one that has not landed. Failure is reported to the
    /// instance rather than to anybody else: no caller can repair a full
    /// disk, the state in memory is still right, and the next change asks
    /// again.
    #[mutants::skip]
    async fn persist(&self, bytes: Vec<u8>) {
        let path = self.0.path.clone();
        let outcome =
            match tokio::task::spawn_blocking(move || write_atomically(&path, &bytes)).await {
                Ok(written) => written.map_err(|why| why.to_string()),
                Err(why) => Err(why.to_string()),
            };
        if let Err(why) = &outcome {
            tracing::error!(%why, "the instance could not be written");
        }
        self.0.asking.send(AppEvent::Persisted { outcome });
    }

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
        self.spawn(move |inner| async move {
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
                            inner.runtime,
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
                            inner.runtime,
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

/// One container, as the instance is told about it: its name, and what its
/// labels say about whose it is and what it was made for.
///
/// Two questions of the runtime per container, because the two runtimes
/// format a listing's labels differently and an inspection is the one shape
/// both take. A label that cannot be read is reported as absent, which is the
/// safe direction: an unlabelled container is left alone.
#[mutants::skip]
pub async fn described(runtime: &ContainerRuntime, name: String, running: bool) -> Container {
    let instance = stageman_agent::started_by(runtime, &name)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!(container = %name, %why, "could not ask which instance started a container");
            None
        });
    let agent = stageman_agent::made_for(runtime, &name)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!(container = %name, %why, "could not ask which agent a container was made for");
            None
        });
    Container {
        name,
        instance,
        agent,
        running,
    }
}

/// Every container this project left behind, as the instance is told about
/// them on waking.
///
/// # Errors
///
/// Fails if the runtime will not say what containers it has, which is worth
/// failing a start over: an instance that cannot see what it left behind
/// cannot keep `docs/conventions.md` §4's bar.
#[mutants::skip]
pub async fn found(runtime: &ContainerRuntime) -> Result<Vec<Container>, stageman_job::JobError> {
    let left = stageman_job::left_behind(runtime).await?;
    let running = stageman_agent::running(runtime)
        .await
        .map_err(stageman_job::JobError::Agent)?;
    let mut containers = Vec::with_capacity(left.len());
    for abandoned in left {
        let up = running.contains(&abandoned.container);
        containers.push(described(runtime, abandoned.container, up).await);
    }
    Ok(containers)
}

/// Replaces a file in one step, so a crash mid-write cannot truncate it.
///
/// Written beside the target rather than in a temporary directory, because
/// renaming across filesystems is not atomic and would silently become a copy.
///
/// # Errors
///
/// Fails if the file cannot be created, written, synced or renamed.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let outcome = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        // Rename is atomic, but only orders against data that has reached the
        // disk. Without this a crash can leave an intact name over empty
        // contents, which is the failure this function exists to prevent.
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if outcome.is_err() {
        // Best effort: the write already failed, and failing to tidy up after
        // it is not worth reporting over the failure itself.
        drop(fs::remove_file(&temporary));
    }
    outcome
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
    use super::{Answered, Asking, Located, Performer, because, write_atomically};
    use stageman_instance::{AppEffect, AppEvent, Event, Request, Response};
    use stageman_world::{Perform as _, World};
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

    /// An effect is performed on a task of its own and comes back as an
    /// event, and a write lands before it is answered.
    #[tokio::test]
    async fn an_effect_is_performed_and_answered_as_an_event() {
        let (world, mut events) = World::new();
        let asking = Asking::new(world);
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("instance.json");
        let runtime: &'static stageman_agent::ContainerRuntime = Box::leak(Box::new(
            stageman_agent::ContainerRuntime::new(std::path::PathBuf::from("/nowhere/docker")),
        ));
        let performer = Performer::new(
            runtime,
            path.clone(),
            "http://host.docker.internal:1/mcp".to_owned(),
            Arc::clone(&asking),
        );

        performer
            .perform(AppEffect::Wake {
                after: Duration::from_millis(1),
                timer: stageman_instance::Timer::Settle,
            })
            .await;
        assert!(matches!(
            soon(events.recv()).await,
            Some(Event::App(AppEvent::Woke {
                timer: stageman_instance::Timer::Settle
            }))
        ));

        performer
            .perform(AppEffect::Persist {
                bytes: stageman_vocabulary::Bytes::new(b"landed".to_vec()),
            })
            .await;
        assert!(matches!(
            soon(events.recv()).await,
            Some(Event::App(AppEvent::Persisted { outcome: Ok(()) }))
        ));
        assert_eq!(std::fs::read(&path).expect("it landed"), b"landed");

        let elsewhere = Performer::new(
            runtime,
            directory.path().join("nowhere").join("instance.json"),
            String::new(),
            Arc::clone(&asking),
        );
        elsewhere
            .perform(AppEffect::Persist {
                bytes: stageman_vocabulary::Bytes::new(b"lost".to_vec()),
            })
            .await;
        assert!(matches!(
            soon(events.recv()).await,
            Some(Event::App(AppEvent::Persisted { outcome: Err(_) }))
        ));
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

    /// A write lands whole, and a failed one leaves nothing beside the file.
    #[test]
    fn a_write_lands_whole_and_leaves_no_temporary_behind() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("instance.json");
        write_atomically(&path, b"first").expect("it writes");
        write_atomically(&path, b"second").expect("it writes again");
        assert_eq!(std::fs::read(&path).expect("it is there"), b"second");
        assert!(!path.with_extension("tmp").exists());

        let missing = directory.path().join("nowhere").join("instance.json");
        assert!(write_atomically(&missing, b"third").is_err());
        assert!(!missing.with_extension("tmp").exists());
    }
}
