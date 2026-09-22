//! A few words that appear on hover or focus, under a control that has no
//! visible label.
//!
//! Stylesheet alone: it is shown by the browser's own hover and focus states
//! and runs nothing, which is what keeps it honest on a page that was rendered
//! on the server and has not woken yet. It is hidden from assistive
//! technology, because the control it sits under carries its own name and
//! saying it twice helps nobody — so a caller gives the control an accessible
//! name, and this repeats it for eyes.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// Properties for [`Tooltip`].
#[derive(Props, PartialEq, Clone)]
pub struct TooltipProps {
    /// What to say.
    pub text: String,
    /// Extra classes for the wrapper, merged over the ones below.
    #[props(default)]
    pub class: String,
    /// The control it sits under.
    pub children: Element,
}

/// A few words on hover or focus.
#[component]
pub fn Tooltip(props: TooltipProps) -> Element {
    rsx! {
        span { class: tw_merge!("group relative inline-flex", props.class),
            {props.children}
            span {
                aria_hidden: "true",
                // Below the control, centred on it, and a moment late: a
                // tooltip that appears the instant a pointer crosses a row of
                // controls is five tooltips in a row.
                class: "pointer-events-none absolute left-1/2 top-full z-20 mt-1.5 -translate-x-1/2 \
                        whitespace-nowrap rounded-md border border-border bg-surface px-2 py-1 \
                        text-xs text-foreground shadow-md opacity-0 delay-150 \
                        motion-safe:transition-opacity group-hover:opacity-100 \
                        group-focus-within:opacity-100",
                "{props.text}"
            }
        }
    }
}
