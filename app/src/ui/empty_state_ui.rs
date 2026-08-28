//! What a region says when it has nothing in it.
//!
//! Worth a component rather than a paragraph because an empty dashboard is the
//! *first* thing anybody sees — `docs/decisions/0021-an-instance-starts-empty.md`
//! makes a fresh instance empty by design — so the emptiest screen is the one
//! that has to explain itself best.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// Properties for [`EmptyState`].
#[derive(Props, PartialEq, Clone)]
pub struct EmptyStateProps {
    /// What is not here, in a few words.
    pub title: String,
    /// What to do about it. Optional, because sometimes nothing is wrong and
    /// there is simply nothing yet.
    #[props(default)]
    pub note: Option<String>,
    /// An action that would fix it.
    #[props(default)]
    pub action: Option<Element>,
    /// Extra classes, merged over the ones below.
    #[props(default)]
    pub class: String,
}

/// Nothing here, and why.
#[component]
pub fn EmptyState(props: EmptyStateProps) -> Element {
    rsx! {
        div {
            class: tw_merge!(
                "flex flex-col items-start gap-1 py-6 text-left",
                props.class
            ),
            p { class: "text-sm text-muted-foreground", "{props.title}" }
            if let Some(note) = props.note {
                p { class: "max-w-prose text-xs text-faint-foreground", "{note}" }
            }
            if let Some(action) = props.action {
                div { class: "mt-3", {action} }
            }
        }
    }
}
