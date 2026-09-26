//! The line at the foot of every page: this machine, and this build.
//!
//! Not a page.
//! `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`
//! retired the page that showed the machine, because nobody acts on a
//! runtime path from a page: it is worth seeing everywhere and worth reading
//! nowhere in particular, which is what a footer is for.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use stageman_instance::{Request, Response};

use super::error::DashboardResult;
use super::live::{Live, Reading, use_reading};

pub use stageman_wire::Instance;

/// This machine and this build, as much of it as a page may know.
///
/// # Errors
///
/// Fails if this process is not operating an instance, which is a fault in
/// this process rather than in anything a request did.
#[get("/api/instance")]
pub async fn instance() -> DashboardResult<Instance> {
    match super::ask(Request::Instance).await? {
        Response::Instance(shown) => Ok(shown),
        other => Err(super::unexpected(&other)),
    }
}

/// The status line.
///
/// Says nothing while it has nothing to say: a footer reporting its own
/// failure to load would be the least useful line on the page, and every
/// page above it already reports the instance being unreadable.
#[component]
pub fn Status() -> Element {
    let live = use_context::<Live>();
    let Reading::Read(read) = use_reading(live, instance)? else {
        return VNode::empty();
    };
    let shown = read();

    rsx! {
        // In view at the foot of the window, under the same rule as the
        // shell's header — `docs/conventions.md` §3 — and opaque, so that
        // the page passes under it rather than through it.
        footer { class: "sticky bottom-0 z-40 border-t border-border bg-surface",
            div { class: "mx-auto flex max-w-5xl flex-wrap items-baseline gap-x-5 gap-y-1 px-6 py-4 \
                          text-xs text-muted-foreground",
                Fact { label: "runtime", value: shown.container_runtime }
                Fact { label: "domain", value: shown.domain }
                Fact { label: "version", value: shown.version }
                Fact { label: "agents", value: shown.agents.to_string() }
            }
        }
    }
}

/// One fact on the line: what it is called, and what it is.
#[component]
fn Fact(label: String, value: String) -> Element {
    rsx! {
        span { class: "inline-flex items-baseline gap-1.5",
            span { "{label}" }
            span { class: "font-mono text-foreground", "{value}" }
        }
    }
}
