//! A page's own header, which stays in view while the page scrolls.
//!
//! One component rather than a class on each page, so that every page that
//! has a header of its own keeps it the same way, per `docs/conventions.md`
//! §3: what the page is about, and whatever acts on it, in view from
//! wherever a person has scrolled to.

use dioxus::prelude::*;

/// What a page's own header wears to stay in view under the shell's.
///
/// The shell's header is a line of base text with its padding, fourteen
/// units tall, and a hairline under that; this sits one pixel under the
/// hairline, deliberately. A gap between the two would show the page
/// scrolling through it, and an overlap shows nothing, since the shell's
/// header is above.
///
/// The ladder, so that nothing is above what it should be under: what
/// floats within a page — a tooltip, a popover — is at twenty; a page's
/// own header at thirty, so those pass beneath it; the shell's header and
/// status line at forty, above both; and the modal at fifty, above all of
/// it, since it is the one thing that covers a page.
///
/// Pulled up into the page's own top padding and given it back, so the
/// page passes under this and not through a gap above it.
const UNDER_SHELL: &str = "sticky top-14 z-30 -mt-6 flex flex-col gap-1 border-b border-border \
                           bg-background pt-6 pb-3";

/// Properties for [`PageHeader`].
#[derive(Props, PartialEq, Clone)]
pub struct PageHeaderProps {
    /// The header's lines.
    pub children: Element,
}

/// A page's own header.
#[component]
pub fn PageHeader(props: PageHeaderProps) -> Element {
    rsx! {
        div { class: UNDER_SHELL, {props.children} }
    }
}
