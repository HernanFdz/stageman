//! This application's half of the world: what performs its own effects, and
//! how whoever needs an answer waits for it.
//!
//! The loop, the channel and the generic mechanisms are the world crate's.
//! What is here is what only stageman knows how to perform — a channel's
//! posting and listening — and the one thing the generic world cannot do
//! for it: match an answer to whoever asked. A server function sends an
//! event carrying an identifier and waits for the effect that carries it
//! back, and the map below is where it waits. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Everything here runs on a task of its own: a post waits on the network,
//! and `docs/conventions.md` §3 keeps that off the loop that answers the
//! dashboard. The disk, the agent's process and the probe of a job's tunnel
//! are the world crate's; nothing here names a container runtime any more.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

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

    /// Performs an effect on a task of its own.
    ///
    /// Skipped by mutation testing, like the performer it serves: what it
    /// does is spawn, and what the task does reaches a channel over the
    /// network.
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
    use stageman_agent::{ContainerRuntime, TUNNEL_PORT};
    use stageman_core::{Agent, JobId, Role, Uuid};
    use stageman_instance::{ANSWERING_WITHIN, AppEvent, Event, Request, Response, answering};
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

    /// The container runtime, found rather than configured — allowed in a
    /// test for the reason the rule itself gives: it is about a daemon under
    /// a service manager, and this runs on somebody's machine.
    fn located_runtime() -> ContainerRuntime {
        let located = std::process::Command::new("sh")
            .args(["-c", "command -v docker"])
            .output()
            .expect("looking for a container runtime");
        let path = String::from_utf8(located.stdout).expect("a runtime path is text");
        ContainerRuntime::new(std::path::PathBuf::from(path.trim()))
    }

    /// Waits until a container has said the thing that means it is ready.
    ///
    /// **Asks the container, never the port.** Waiting on the probe would
    /// make the assertion that follows pass whenever the probe said yes,
    /// which is the bug it is there to catch — a readiness check must not be
    /// the thing under test.
    async fn ready(runtime: &ContainerRuntime, name: &str, marker: &str) {
        // Thirty seconds, counted in polls rather than measured against a
        // deadline: adding a duration to an instant is arithmetic that can
        // overflow, the gate rightly refuses it, and there is nothing here
        // that needs a clock.
        let mut printed = String::new();
        for _ in 0..300 {
            let said = std::process::Command::new(runtime.path())
                .args(["logs", name])
                .output()
                .expect("the runtime reports what a container printed");
            printed = format!(
                "{}{}",
                String::from_utf8_lossy(&said.stdout),
                String::from_utf8_lossy(&said.stderr)
            );
            if printed.contains(marker) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("{name} never said {marker:?}; it said: {printed}");
    }

    /// Whether a job's tunnel answers, asked the way the instance asks it:
    /// the world probes the port with the instance's budget, and the
    /// instance reads what the port did.
    async fn answers(port: u16) -> (bool, stageman_vocabulary::Probed) {
        let probed = stageman_world::probe(port, ANSWERING_WITHIN).await;
        (answering(probed), probed)
    }

    /// A published port answers for nobody when nothing is inside, and for
    /// somebody when something is.
    ///
    /// **Here because this is the one crate that names both halves.** The
    /// probe is the world's and knows no runtime; what its answer means is
    /// the instance's; and only a port a real runtime published meets the
    /// proxy that decides a container's life — `docs/conventions.md` §4. The
    /// world crate's own test binds a socket and proves the probe can tell
    /// the four things a port can do apart, and nothing about which of them
    /// a proxy with nothing behind it does. With a probe that connected and
    /// nothing more, the first assertion below fails on both runtimes, which
    /// is precisely how every job container came to run for ever. It is the
    /// recorder's, once that lands.
    ///
    /// Both directions, because a probe that answered nothing to everything
    /// would pass the half that matters and destroy the feature: it would
    /// stop a container somebody is looking at.
    ///
    /// Needs a runtime and no credential and no image of ours — anything
    /// that stays up will do, and the entry point is overridden so nothing
    /// is run. The job's identifier is shared with no other container test,
    /// because two tests naming one container run at once and the second is
    /// refused the name.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_published_port_with_nothing_inside_answers_for_nobody() {
        let runtime = located_runtime();
        let name = stageman_job::container(JobId::from_uuid(Uuid::from_u128(43)));
        stageman_agent::discard(&runtime, &name)
            .await
            .expect("a clean slate");
        let anything = stageman_agent::build(&runtime, Agent::Claude, Role::Foreman)
            .await
            .expect("the image builds");

        // Nothing inside is listening: the proxy is the only thing on the host
        // port, and it is what a bare connection would find.
        let empty = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                &name,
                "--label",
                &format!("stageman.job={name}"),
                "--publish",
                &format!("127.0.0.1::{TUNNEL_PORT}"),
                "--entrypoint",
                "sh",
                anything.as_argument(),
                "-c",
                "sleep 30",
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            empty.status.success(),
            "{}",
            String::from_utf8_lossy(&empty.stderr)
        );

        let port = stageman_agent::tunnel_port(&runtime, &name)
            .await
            .expect("the runtime answers")
            .expect("a mapping was published");
        // Reported rather than asserted, and only if this fails. Whether a
        // bare connection succeeds is a property of the host: both runtimes
        // proxy a published port by default and it does, which is the trap
        // this test exists for, but Docker with `userland-proxy` disabled
        // refuses instead and the assertion below then holds for a simpler
        // reason. Saying which makes a failure here diagnosable rather than
        // puzzling.
        let trapped = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok();
        let (answering, probed) = answers(port).await;
        assert!(
            !answering,
            "nothing is listening inside, so the proxy on {port} answers for nobody \
             (the port {probed:?}; a bare connection to it succeeds here: {trapped})"
        );

        stageman_agent::discard(&runtime, &name)
            .await
            .expect("it is removable");

        // And the other direction, on a container that genuinely serves, so
        // that this cannot pass by answering `false` to everything.
        //
        // Node rather than a networking tool, because the base image is
        // node and has no `nc`, `socat` or `python3` — the same absence
        // `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
        // records about `ss` and `lsof`. It binds every interface, not
        // loopback, or the proxy would have nothing to forward to; and it
        // never writes, which makes it the held-open-and-silent case rather
        // than the easy one.
        let serving = std::process::Command::new(runtime.path())
            .args([
                "run",
                "--detach",
                "--name",
                &name,
                "--label",
                &format!("stageman.job={name}"),
                "--publish",
                &format!("127.0.0.1::{TUNNEL_PORT}"),
                "--entrypoint",
                "node",
                anything.as_argument(),
                "-e",
                &format!(
                    "require('net').createServer().listen({TUNNEL_PORT}, '0.0.0.0', \
                     () => console.log('listening'))"
                ),
            ])
            .output()
            .expect("the runtime runs");
        assert!(
            serving.status.success(),
            "{}",
            String::from_utf8_lossy(&serving.stderr)
        );

        // Waited for, and waited for by asking the container rather than the
        // port. `--detach` returns once the container is started, which is
        // before node has bound anything — so probing straight away finds the
        // proxy with nothing behind it yet and reads exactly like the empty
        // case above. That is what failed in continuous integration and passed
        // on the machine that wrote it, which is the shape of every race worth
        // the name.
        ready(&runtime, &name, "listening").await;

        let port = stageman_agent::tunnel_port(&runtime, &name)
            .await
            .expect("the runtime answers")
            .expect("a mapping was published");
        let (answering, probed) = answers(port).await;
        assert!(
            answering,
            "something is listening inside, so {port} is showing it (the port {probed:?})"
        );

        stageman_agent::discard(&runtime, &name)
            .await
            .expect("it is removable");
    }
}
