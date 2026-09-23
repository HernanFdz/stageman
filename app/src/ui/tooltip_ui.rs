//! A few words that appear on hover, or on a focus that came from the
//! keyboard, under a control that has no visible label.
//!
//! Stylesheet alone: it is shown by the browser's own states and runs
//! nothing, which is what keeps it honest on a page that was rendered on
//! the server and has not woken yet. Hover, and `focus-visible` rather than
//! focus, per `docs/conventions.md` §3: a control that is clicked keeps its
//! focus, so a tooltip shown for focus stayed until the next click landed
//! somewhere that took it, and read as stuck. It is hidden from assistive
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
    /// Whether the text wraps, at the width a short paragraph reads well
    /// at, rather than staying on its lines: for a reason, never for a
    /// name, which is one line by nature.
    #[props(default = false)]
    pub wrap: bool,
    /// Whether it hangs from the control's end rather than its centre: for
    /// a control at the end of a line, so that the text runs back over the
    /// page instead of off it.
    #[props(default = false)]
    pub at_end: bool,
    /// The control it sits under.
    pub children: Element,
}

/// A few words on hover, or on keyboard focus.
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
                // Kept on the lines it was given and never wrapped: one
                // line for a name, and a line per item where a caller
                // lists a few.
                class: tw_merge!(
                    "pointer-events-none absolute left-1/2 top-full z-20 mt-1.5 -translate-x-1/2 \
                     whitespace-pre rounded-md border border-border bg-surface px-2 py-1 \
                     text-xs text-foreground shadow-md opacity-0 delay-150 \
                     motion-safe:transition-opacity group-hover:opacity-100 \
                     group-has-[:focus-visible]:opacity-100",
                    if props.wrap {
                        "w-max max-w-sm whitespace-normal text-left"
                    } else {
                        ""
                    },
                    if props.at_end {
                        "left-auto right-0 translate-x-0"
                    } else {
                        ""
                    }
                ),
                "{props.text}"
            }
        }
    }
}
