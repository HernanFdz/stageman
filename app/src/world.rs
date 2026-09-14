//! This application's half of the world: what performs its own effects, and
//! how whoever needs an answer waits for it.
//!
//! The loop, the channel and the generic mechanisms are the world crate's.
//! What is here is what only stageman knows how to perform — a probe, a
//! channel's posting — and the one thing the generic world cannot do for
//! it: match an answer to whoever asked. A server function sends an event
//! carrying an identifier and waits for the effect that carries it back,
//! and the map below is where it waits. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Everything here runs on a task of its own: a probe waits on a socket,
//! and `docs/conventions.md` §3 keeps that off the loop that answers the
//! dashboard. The disk and the agent's process are the world crate's.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use stageman_agent::ContainerRuntime;
use stageman_core::{Channel, Secret, Speaking};
use stageman_instance::{AppEffect, AppEvent, Event, Request, RequestId, Response, Stageman};
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

/// Sends the instance events, and matches its answers to whoever asked.
pub struct Asking {
    /// The loop's way in.
    world: Arc<World<Stageman>>,
    /// The next request identifier. Never reused while this process runs.
    next: AtomicU64,
    /// Everyone waiting on an answer, by the identifier it will carry.
    waiting: parking_lot::Mutex<BTreeMap<RequestId, tokio::sync::oneshot::Sender<Response>>>,
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
        self.waiting.lock().insert(id, answer);
        self.send(AppEvent::Request { id, request });
        waiting.await.ok()
    }

    /// Delivers an answer to whoever asked.
    ///
    /// Nobody waiting is not an error: a browser that gave up has already
    /// dropped its end, and the answer has nowhere to go.
    fn answered(&self, id: RequestId, response: Response) {
        let Some(reply) = self.waiting.lock().remove(&id) else {
            tracing::debug!(?id, "an answer arrived for a request nobody is waiting on");
            return;
        };
        drop(reply.send(response));
    }
}

/// What performs this application's own effects.
///
/// Cheap to clone and handed whole to every task it spawns, which is why the
/// state sits behind one shared inner value.
#[derive(Clone)]
pub struct Performer(Arc<Inner>);

/// The runtime the probe goes through, once the instance has found one and
/// said so.
///
/// A static rather than a field: what borrows it is a task that outlives the
/// call that spawned it, and a process has exactly one running instance to
/// have found one. On its way out with the probe, which is the one effect
/// left that needs it.
static RUNTIME: OnceLock<ContainerRuntime> = OnceLock::new();

struct Inner {
    /// Where events go back, and where answers are matched to askers.
    asking: Arc<Asking>,
}

impl Performer {
    /// A performer for this way in.
    #[must_use]
    pub fn new(asking: Arc<Asking>) -> Self {
        Self(Arc::new(Inner { asking }))
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
            AppEffect::Booted { runtime } => {
                if RUNTIME.set(ContainerRuntime::new(runtime)).is_err() {
                    tracing::error!("the instance booted twice; the second runtime is ignored");
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
                self.0.asking.answered(id, response);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Asking;
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
        asking.answered(id, Response::Agents(Vec::new()));
        assert_eq!(
            soon(asked).await.expect("the task"),
            Some(Response::Agents(Vec::new()))
        );
    }
}
