//! A list of rows parted by a hairline, and the one place that owns its item.
//!
//! `docs/conventions.md` §3 wants a list inside a section to be rows parted
//! by a hairline rather than boxes within the box, with the first and last
//! shedding their outer padding, so that the space above the first row and
//! below the last are both the card's own and therefore equal.
//!
//! That shedding has to be said on the list's item. Said on a row's own
//! root it matches every time, because a row's root is the only child of its
//! item and so both its first and its last: every row loses its padding, and
//! the rows sit on their hairlines. A component rather than a pair of class
//! constants, so that a page has nowhere to write it on the wrong element.
//!
//! The list of jobs is the one that does not come through here: its columns
//! are the list's, shared by every row, which needs the item to be a grid of
//! its own kind.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// What every item of the list wears: breathing space above and below what
/// it holds, shed at the list's two ends.
const ROW: &str = "py-3 first:pt-0 last:pb-0";

/// A list of rows, parted by a hairline.
#[component]
pub fn Rows(children: Element) -> Element {
    rsx! {
        ul { class: "divide-y divide-border", {children} }
    }
}

/// Properties for [`Row`].
#[derive(Props, PartialEq, Clone)]
pub struct RowProps {
    /// Extra classes for the item, merged over its own: a roomier padding
    /// for rows of several lines, a tighter one for rows of one.
    #[props(default)]
    pub class: String,
    /// What the row holds.
    pub children: Element,
}

/// One row of [`Rows`]: the item, and the padding that is the item's to
/// carry.
#[component]
pub fn Row(props: RowProps) -> Element {
    rsx! {
        li { class: tw_merge!(ROW, props.class), {props.children} }
    }
}

// On the daemon's half only, for the reason the icon tests give.
#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{Row, Rows};
    use dioxus::prelude::*;

    /// The padding, and the shedding of it at the two ends, are on the
    /// list's items and on nothing inside them: a row's own root is the only
    /// child of its item, so a first- or last-child variant there would
    /// match every row.
    #[test]
    fn the_padding_is_the_items_and_never_the_rows_own() {
        let drawn = dioxus::ssr::render_element(rsx! {
            Rows {
                Row { div { class: "one", "first" } }
                Row { div { class: "two", "second" } }
            }
        });

        assert!(
            drawn.starts_with(r#"<ul class="divide-y divide-border">"#),
            "{drawn}"
        );
        assert_eq!(
            drawn
                .matches(r#"<li class="py-3 first:pt-0 last:pb-0">"#)
                .count(),
            2,
            "{drawn}"
        );
        assert!(drawn.contains(r#"<div class="one">first</div>"#), "{drawn}");
        assert!(
            drawn.contains(r#"<div class="two">second</div>"#),
            "{drawn}"
        );
    }

    /// A list of taller or shorter rows says so on the item, and what it
    /// says replaces the padding rather than joining it, with the shedding
    /// kept.
    #[test]
    fn a_rows_own_padding_replaces_the_usual_one_and_keeps_the_shedding() {
        let drawn = dioxus::ssr::render_element(rsx! {
            Rows {
                Row { class: "py-4", "roomy" }
            }
        });

        assert!(drawn.contains("py-4"), "{drawn}");
        assert!(!drawn.contains("py-3"), "{drawn}");
        assert!(
            drawn.contains("first:pt-0") && drawn.contains("last:pb-0"),
            "{drawn}"
        );
    }
}
