//! A choice among a handful of things, all of them visible.
//!
//! What a closed set is chosen from here, per `docs/conventions.md` §3: an
//! agent, a model, an effort. Every option is seen at once and nothing floats
//! over anything. The browser's own select is what this replaces: it draws
//! itself outside the theme, and it hides the options a choice is made
//! between.

use dioxus::prelude::*;

/// Properties for [`Segmented`].
#[derive(Props, PartialEq, Clone)]
pub struct SegmentedProps {
    /// What the group is called, for whoever cannot see it.
    pub label: String,
    /// The options: what the browser sends back, and what a person reads.
    pub options: Vec<(String, String)>,
    /// Which is chosen, by what the browser sends back.
    pub value: String,
    /// Told what the browser sends back for whichever is pressed.
    pub onchange: EventHandler<String>,
}

/// A row of options, one of them chosen.
#[component]
pub fn Segmented(props: SegmentedProps) -> Element {
    let value = props.value;
    let onchange = props.onchange;

    rsx! {
        div {
            role: "radiogroup",
            aria_label: "{props.label}",
            class: "inline-flex flex-wrap gap-0.5 rounded-md border border-border bg-surface p-0.5",
            for (id, name) in props.options {
                button {
                    key: "{id}",
                    r#type: "button",
                    role: "radio",
                    aria_checked: if id == value { "true" } else { "false" },
                    class: if id == value {
                        "rounded bg-surface-muted px-2.5 py-1 text-sm font-medium text-foreground"
                    } else {
                        "rounded px-2.5 py-1 text-sm text-muted-foreground hover:text-foreground \
                         motion-safe:transition-colors"
                    },
                    onclick: move |_| onchange.call(id.clone()),
                    "{name}"
                }
            }
        }
    }
}
