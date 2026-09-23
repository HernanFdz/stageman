//! A guide: a link that opens a platform's own form, filled in, beside the
//! box that takes what the form mints.
//!
//! A mark and a verb, with what it does a hover away and the address never
//! written out, per `docs/conventions.md` §3. It leaves the page, so it
//! opens in a tab of its own and the form it was pressed from stays as it
//! was, which is what makes pasting back into it a matter of switching
//! tabs. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.

use dioxus::prelude::*;

use super::{ButtonVariant, Mark, Tooltip};

/// Properties for [`Guide`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct GuideProps {
    /// Whose form, by the identifier the wire uses for the platform or
    /// channel.
    pub mark: String,
    /// The verb beside the mark: what pressing it opens.
    pub label: String,
    /// What it does, in a sentence, for whoever hovers or focuses it, and
    /// the control's description to assistive technology.
    pub says: String,
    /// Where the form is.
    pub link: String,
}

/// A link to a platform's form, filled in.
#[component]
pub fn Guide(props: GuideProps) -> Element {
    rsx! {
        // At the end of a line, always, so the sentence hangs from the
        // control's end and runs back over the page.
        Tooltip { text: props.says.clone(), wrap: true, at_end: true,
            a {
                href: "{props.link}",
                target: "_blank",
                rel: "noopener noreferrer",
                // The height of the box beside it, per the rule a control
                // beside a box is under, and small text because it is a
                // verb on a label's line rather than an action of the page.
                class: ButtonVariant::Secondary.styled("h-8.5 gap-1.5 px-2 text-xs"),
                aria_label: "{props.label}",
                "aria-description": "{props.says}",
                Mark { agent: props.mark, size: 14 }
                "{props.label}"
            }
        }
    }
}
