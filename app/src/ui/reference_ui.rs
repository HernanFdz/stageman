//! A reference that leaves the page, as a mark with the address a hover away.
//!
//! Per `docs/conventions.md` §3: a link to a platform or a channel says
//! whose it goes to at a glance, and never writes the address out, which is
//! read character by character. It is a link where the link is true, and a
//! quiet mark otherwise, still saying what it stands for, so that a page
//! links only what is true and hides nothing.

use dioxus::prelude::*;

use super::{Mark, Tooltip};

/// Properties for [`Reference`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct ReferenceProps {
    /// Whose, by the identifier the wire uses for the platform or channel.
    pub mark: String,
    /// What it stands for, in words: the tooltip, and the name for whoever
    /// cannot see the mark.
    pub says: String,
    /// Where it goes, when it goes somewhere.
    #[props(default)]
    pub link: Option<String>,
}

/// A mark that links, or one that only says.
#[component]
pub fn Reference(props: ReferenceProps) -> Element {
    rsx! {
        Tooltip { text: props.says.clone(),
            if let Some(link) = props.link {
                a {
                    href: "{link}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    class: "inline-flex items-center rounded text-muted-foreground hover:text-foreground \
                            focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                    aria_label: "{props.says}",
                    Mark { agent: props.mark, size: 16 }
                }
            } else {
                span {
                    class: "inline-flex items-center text-faint-foreground",
                    tabindex: "0",
                    aria_label: "{props.says}",
                    Mark { agent: props.mark, size: 16 }
                }
            }
        }
    }
}
