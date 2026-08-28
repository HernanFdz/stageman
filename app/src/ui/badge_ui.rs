//! A small label carrying one word of state.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// What a badge is saying, which decides how it looks.
///
/// Named for meaning rather than colour, so a screen says what it knows and
/// this decides how to show it. The three that name job states are deliberate
/// and are the three in `docs/conventions.md` §2 — a fourth here would be an
/// invitation to invent a fourth state in a view rather than in the domain.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
#[non_exhaustive]
pub enum BadgeTone {
    /// Something is happening now.
    Running,
    /// Something finished. Not that it succeeded — nothing here can see that.
    Completed,
    /// Something went wrong and a person will want to know why.
    Failed,
    /// Everything else: a count, a name, a label carrying no verdict.
    #[default]
    Neutral,
}

impl BadgeTone {
    /// The classes this tone renders with.
    const fn class(self) -> &'static str {
        match self {
            Self::Running => "bg-running/10 text-running",
            Self::Completed => "bg-completed/10 text-completed",
            Self::Failed => "bg-failed/10 text-failed",
            Self::Neutral => "bg-surface-muted text-muted-foreground",
        }
    }
}

/// Properties for [`Badge`].
#[derive(Props, PartialEq, Clone)]
pub struct BadgeProps {
    /// What it is saying.
    #[props(default)]
    pub tone: BadgeTone,
    /// Extra classes, merged over the ones above.
    #[props(default)]
    pub class: String,
    /// Anything else a caller wants on the element.
    #[props(extends = span, extends = GlobalAttributes)]
    pub attributes: Vec<Attribute>,
    /// The label.
    pub children: Element,
}

/// One word of state, in a pill.
#[component]
pub fn Badge(props: BadgeProps) -> Element {
    rsx! {
        span {
            class: tw_merge!(
                "inline-flex items-center gap-1 whitespace-nowrap rounded-full px-2 py-0.5 \
                 text-xs font-medium",
                props.tone.class(),
                props.class
            ),
            ..props.attributes,
            {props.children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BadgeTone;
    use std::collections::BTreeSet;

    /// Every tone there is.
    ///
    /// Listed by hand, which is the weakness of this test and worth knowing:
    /// a variant added without a line here is a variant nothing below checks.
    const EVERY: &[BadgeTone] = &[
        BadgeTone::Running,
        BadgeTone::Completed,
        BadgeTone::Failed,
        BadgeTone::Neutral,
    ];

    #[test]
    fn every_tone_renders_as_something() {
        for tone in EVERY {
            assert!(!tone.class().is_empty(), "{tone:?} renders as nothing");
        }
    }

    /// Two states that look identical are two states nobody can tell apart.
    ///
    /// The real subject of this test rather than a technicality: a running job
    /// and a failed one are the two things an operator scans a dashboard for,
    /// and the whole value of a tone is that they cannot be confused.
    ///
    /// Counted through a set rather than compared pairwise, which says the
    /// same thing without indexing or arithmetic — both of which this project
    /// denies. The first attempt at this test reached for a clamping addition
    /// to get around one, and the gate was right to refuse it.
    #[test]
    fn no_two_tones_look_the_same() {
        let distinct: BTreeSet<&str> = EVERY.iter().map(|tone| tone.class()).collect();

        assert_eq!(
            distinct.len(),
            EVERY.len(),
            "two tones render identically: {EVERY:?} produced {distinct:?}"
        );
    }
}
