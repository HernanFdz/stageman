//! A bounded region with a title, which is most of what a dashboard is.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// Properties for [`Card`].
#[derive(Props, PartialEq, Clone)]
pub struct CardProps {
    /// What this region is about.
    pub title: String,
    /// An optional line under the title, for what the title cannot say.
    #[props(default)]
    pub note: Option<String>,
    /// Something aligned to the right of the title — a count, a badge, an
    /// action.
    #[props(default)]
    pub aside: Option<Element>,
    /// Extra classes, merged over the ones below.
    #[props(default)]
    pub class: String,
    /// The contents.
    pub children: Element,
}

/// A titled region.
#[component]
pub fn Card(props: CardProps) -> Element {
    rsx! {
        section {
            class: tw_merge!(
                "rounded-lg border border-border bg-surface",
                props.class
            ),
            header { class: "flex items-baseline justify-between gap-3 border-b border-border px-4 py-3",
                div {
                    h2 { class: "text-sm font-semibold text-foreground", "{props.title}" }
                    if let Some(note) = props.note {
                        p { class: "mt-0.5 text-xs text-muted-foreground", "{note}" }
                    }
                }
                if let Some(aside) = props.aside {
                    div { class: "shrink-0", {aside} }
                }
            }
            div { class: "px-4 py-3", {props.children} }
        }
    }
}
