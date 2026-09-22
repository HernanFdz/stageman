//! More about a field, for whoever asks.
//!
//! One visible line per field and the rest behind a control, per
//! `docs/conventions.md` §3. A disclosure element rather than a script: the
//! browser opens and closes it, the keyboard reaches it, and nothing here
//! runs.

use dioxus::prelude::*;

use super::Icon;

/// Properties for [`Info`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct InfoProps {
    /// The rest of what there is to say.
    pub text: String,
}

/// An information control, and what it discloses.
#[component]
pub fn Info(props: InfoProps) -> Element {
    rsx! {
        details { class: "relative inline-block align-middle",
            summary {
                class: "cursor-pointer list-none rounded text-muted-foreground hover:text-foreground \
                        focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary \
                        [&::-webkit-details-marker]:hidden",
                aria_label: "More about this",
                {Icon::Info.draw(14)}
            }
            div {
                class: "absolute left-0 top-full z-20 mt-1.5 w-80 rounded-md border border-border \
                        bg-surface p-3 text-xs leading-relaxed text-muted-foreground shadow-md",
                "{props.text}"
            }
        }
    }
}
