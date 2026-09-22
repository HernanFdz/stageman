//! What a region shows while it is still being read.
//!
//! Bars where the words will be, moving gently, so that a page which has not
//! finished reading looks like one about to rather than one that is broken.
//! Under the reduced-motion preference they hold still, per
//! `docs/conventions.md` §3.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// Properties for [`Skeleton`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct SkeletonProps {
    /// How many lines of it.
    #[props(default = 3)]
    pub lines: usize,
    /// Extra classes, merged over the ones below.
    #[props(default)]
    pub class: String,
}

/// Lines that are still being read.
#[component]
pub fn Skeleton(props: SkeletonProps) -> Element {
    // Three widths, cycled, so the bars read as lines of text rather than as
    // one grey block.
    const WIDTHS: [&str; 3] = ["w-2/3", "w-full", "w-1/2"];

    rsx! {
        div {
            role: "status",
            aria_label: "Reading…",
            class: tw_merge!("flex flex-col gap-2 py-1 motion-safe:animate-pulse", props.class),
            for (position, width) in WIDTHS.iter().cycle().take(props.lines).enumerate() {
                div { key: "{position}", class: "h-3 rounded bg-surface-muted {width}" }
            }
        }
    }
}
