//! The world: everything the instance is not, performed.
//!
//! One loop owns the deciding half and steps it, one event at a time, on a
//! task of its own. Everything that happens is an event sent to that loop,
//! and everything it asks for comes back as an effect this crate performs —
//! the generic ones as the mechanism each names, the application's own by
//! handing them to what the entry point supplied. Nothing here decides; see
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Small enough to be read rather than tested, which is the point of it
//! being a crate of its own: what is here is a channel, a loop, and one
//! performer per mechanism, and none of them holds a domain type.

use std::sync::Arc;

use stageman_vocabulary::{App, Deciding, Effect, Event, Named};

/// The way in: events go to the loop, and nothing comes back this way.
///
/// Whoever needs an answer to what they sent waits on the application's own
/// effect carrying the identifier back, which the application's performer
/// delivers.
pub struct World<A: App> {
    events: tokio::sync::mpsc::UnboundedSender<Event<A>>,
}

impl<A: App> World<A> {
    /// A world nothing is stepping yet, and the events it will send.
    #[must_use]
    pub fn new() -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<Event<A>>) {
        let (events, receiving) = tokio::sync::mpsc::unbounded_channel();
        (Arc::new(Self { events }), receiving)
    }

    /// Tells the instance something happened.
    pub fn send(&self, event: impl Into<Event<A>>) {
        let event = event.into();
        if self.events.send(event).is_err() {
            tracing::error!("the instance is no longer stepping, so an event was lost");
        }
    }
}

/// What performs the application's own effects.
///
/// The one thing the entry point supplies. The world calls it for every
/// effect in the application hole and for nothing else.
pub trait Perform<A: App>: Send + Sync + 'static {
    /// Performs one of the application's effects.
    fn perform(&self, effect: A::Effect) -> impl std::future::Future<Output = ()> + Send;
}

/// Steps the deciding half for as long as this process runs.
///
/// `pending` is what it asked for on waking, performed before the first event
/// is read. Each effect is performed before the next is looked at, which is
/// what keeps an answered effect answered in the order it was asked for; a
/// performer that has nothing to wait on returns at once.
#[mutants::skip]
pub fn run<D, P>(
    deciding: D,
    pending: Vec<Effect<D::App>>,
    performer: Arc<P>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<Event<D::App>>,
) where
    D: Deciding + Send + 'static,
    P: Perform<D::App>,
{
    drop(tokio::spawn(async move {
        let mut deciding = deciding;
        for effect in pending {
            perform(&performer, effect).await;
        }
        while let Some(event) = events.recv().await {
            tracing::trace!(kind = event.kind(), "stepping");
            for effect in deciding.step(event) {
                perform(&performer, effect).await;
            }
        }
        tracing::error!("the world stopped sending events, so the instance stopped stepping");
    }));
}

/// Performs one effect: a generic one as the mechanism it names, and one of
/// the application's by handing it over.
#[mutants::skip]
async fn perform<A: App, P: Perform<A>>(performer: &Arc<P>, effect: Effect<A>) {
    tracing::trace!(kind = effect.kind(), "performing");
    match effect {
        Effect::App(effect) => performer.perform(effect).await,
    }
}
