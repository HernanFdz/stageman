//! How a page stays live: the stream of ticks every page shares, and the
//! mark that says whether it is open.
//!
//! Not a screen, unlike its neighbours: what is here is what every screen has
//! in common, per
//! `docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`. The world
//! tells whoever follows it each time a write of the instance's file has
//! landed; the route below hands each of those to an open page as a tick
//! carrying nothing; the shell opens that stream once, and every page's read
//! follows the ticks, so it re-runs on each. Ticks that land while a page is
//! busy collapse into one, which the channel guarantees rather than anybody.

// The route below is a server function, which the framework requires to be
// `async` whether or not its body awaits — and this one hands a stream back
// without awaiting. At module scope rather than on the function, because the
// macro drops every attribute but the doc comments; and on the daemon's half
// only, because the browser's half of the same function is a generated stub
// that does await.
#![cfg_attr(
    feature = "server",
    expect(
        clippy::unused_async,
        reason = "a server function is async by the framework's contract, and this one only hands a stream back"
    )
)]

use dioxus::fullstack::TextStream;
use dioxus::prelude::*;

use crate::ui::Tooltip;

/// What a tick says on the wire. Nothing, in a word: a page that reads it
/// learns only that something may have changed, and reads again.
const TICK: &str = "tick\n";

/// What the stream says first, as soon as it is open.
///
/// Sent before anything has changed, and that is the whole of its purpose:
/// not every browser resolves a streaming fetch on the head alone — Firefox
/// waits for the first byte of the body — so a stream that stayed silent
/// until something changed would leave a page unable to say it was live.
const OPENING: &str = "open\n";

/// One tick per write that lands, for as long as the page keeps the stream
/// open.
///
/// The framework's own error rather than the dashboard's, and that is a
/// choice: converting a transport failure into the dashboard's error logs
/// it as one, which is right for a route a person pressed and wrong for a
/// stream that fails as a matter of course whenever the daemon restarts
/// under it — a page reopens the stream, and says so at a level nobody
/// scrolls past by default.
///
/// # Errors
///
/// Fails if this process is not operating an instance.
#[get("/api/ticks")]
pub async fn ticks() -> Result<TextStream, ServerFnError> {
    let Some(asking) = crate::asking() else {
        return Err(ServerFnError::new(
            "this process is not operating an instance",
        ));
    };
    let mut following = asking.written();
    Ok(TextStream::spawn(move |ticking| async move {
        if ticking.unbounded_send(OPENING.to_owned()).is_err() {
            return;
        }
        // Each change since the stream was opened, and none before it: the
        // receiver has seen everything up to its making. The loop ends when
        // the page has gone, which is the send failing, or when the world
        // has, which is the change never coming.
        while following.changed().await.is_ok() {
            if ticking.unbounded_send(TICK.to_owned()).is_err() {
                break;
            }
        }
    }))
}

/// What every page shares about being live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Live {
    /// Flipped on every tick. A page reads it inside its own read, so the
    /// read re-runs when it flips; what it is worth means nothing.
    ticked: Signal<bool>,
    /// Whether the stream is open now. Unknown until the page is awake,
    /// because the server does not know and must not guess.
    connected: Signal<Option<bool>>,
}

impl Live {
    /// Nothing known yet, and nothing ticked.
    pub(super) fn new() -> Self {
        Self {
            ticked: Signal::new(false),
            connected: Signal::new(None),
        }
    }

    /// Subscribes the read this is called from to the ticks: that read
    /// re-runs on each. What it answers means nothing.
    // Skipped by mutation testing: reading the signal is the whole act,
    // and the value is discarded by every caller, so a mutant returning
    // either constant is equivalent by design.
    #[mutants::skip]
    #[must_use]
    pub fn follow(self) -> bool {
        (self.ticked)()
    }
}

/// Opens the stream once the page is awake, and keeps it open.
///
/// The browser's half only: the server renders a page and is not one, and a
/// server that followed its own ticks would be reading itself.
// Skipped by mutation testing, as is everything below compiled for the
// browser's half only: mutation testing builds the daemon's half, where this
// is not compiled at all, so a mutant here would pass by never being built.
// The probe drives it in a real browser.
#[cfg(not(feature = "server"))]
#[mutants::skip]
pub(super) fn use_live(live: Live) {
    use_effect(move || {
        spawn(async move { follow(live).await });
    });
}

/// The server's half of the same, which is nothing.
// Skipped by mutation testing: nothing is what it does, by design.
#[cfg(feature = "server")]
#[mutants::skip]
pub(super) const fn use_live(live: Live) {
    let _ = live;
}

/// Follows the ticks for as long as the page is open, reopening the stream
/// whenever it ends.
///
/// The page is live from the stream's opening frame, not from the fetch
/// resolving, because the two are the same moment in one browser and not in
/// another. And it is *not* live only once a retry is due, rather than the
/// moment the stream ends: a page being reloaded has its stream cut first
/// and its paint replaced a moment later, and a mark that flipped in that
/// moment would flash on every reload.
#[cfg(not(feature = "server"))]
#[mutants::skip]
async fn follow(live: Live) {
    let mut live = live;
    let mut again = false;
    loop {
        match ticks().await {
            Ok(mut stream) => {
                while let Some(Ok(frame)) = stream.next().await {
                    if frame.contains(TICK.trim()) {
                        live.ticked.toggle();
                    } else if frame.contains(OPENING.trim()) {
                        live.connected.set(Some(true));
                        // A tick may have been missed between two
                        // connections, and one read is what that costs.
                        if again {
                            live.ticked.toggle();
                        }
                    }
                }
            }
            // Routine: the daemon restarting under the page is the common
            // case, and a page that shouted about it on every retry would
            // bury whatever else the console had to say.
            Err(why) => dioxus::logger::tracing::debug!(
                %why,
                "the stream of changes could not be opened; it is tried again shortly"
            ),
        }
        again = true;
        pause().await;
        live.connected.set(Some(false));
    }
}

/// Waits a while before the stream is opened again, so that a daemon that
/// is down is asked now and then rather than as fast as it refuses.
#[cfg(not(feature = "server"))]
#[mutants::skip]
async fn pause() {
    let mut asked = document::eval(PAUSING);
    let _ = asked.recv::<bool>().await;
}

/// The wait, in the browser's own time. It sends rather than returns, per
/// `docs/conventions.md` §3.
// Evaluated by the browser's half alone, and asserted on by the test below,
// which runs on the daemon's — so on the daemon's half outside a test it is
// read by nothing.
#[cfg_attr(
    all(feature = "server", not(test)),
    expect(
        dead_code,
        reason = "evaluated by the browser's half, and tested on the daemon's"
    )
)]
const PAUSING: &str =
    "await new Promise(function (done) { setTimeout(done, 2000); }); dioxus.send(true);";

/// Whether this page is live, as a mark in the shell — and nothing until the
/// page knows.
///
/// The mark doubles as the diagnostic
/// `docs/decisions/0071-a-page-learns-of-change-from-a-tick.md` promises: a
/// page that never becomes live behind a proxy is a proxy that buffers.
#[component]
pub fn LiveMark(live: Live) -> Element {
    let Some(connected) = (live.connected)() else {
        return VNode::empty();
    };
    let (dot, word, saying) = if connected {
        (
            "bg-working motion-safe:animate-pulse",
            "live",
            "Live: this page updates as the instance changes.",
        )
    } else {
        (
            "bg-faint-foreground",
            "not live",
            "Not live: the stream of changes is closed and will be opened again shortly. \
             Behind a proxy that never opens it, this is the proxy.",
        )
    };

    rsx! {
        Tooltip { text: saying,
            span {
                class: "inline-flex items-center gap-1.5 text-xs text-muted-foreground",
                tabindex: "0",
                aria_label: "{saying}",
                span { aria_hidden: "true", class: "size-1.5 rounded-full {dot}" }
                "{word}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OPENING, PAUSING, TICK};

    /// The wait sends rather than returns, which is the rule for every
    /// script the page evaluates; and each frame is a line, so that a
    /// reader waiting for one can tell where it ends — and the two frames
    /// cannot be mistaken for each other.
    #[test]
    fn the_scripts_and_the_frames_are_shaped_as_the_rules_say() {
        assert!(PAUSING.contains("dioxus.send("), "{PAUSING}");
        assert!(!PAUSING.contains("return"), "{PAUSING}");
        assert!(TICK.ends_with('\n') && OPENING.ends_with('\n'));
        assert!(!TICK.contains(OPENING.trim()) && !OPENING.contains(TICK.trim()));
    }
}
