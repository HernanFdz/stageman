//! Something to press.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// How much weight a button carries on its screen.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
#[non_exhaustive]
pub enum ButtonVariant {
    /// The action the screen exists for. One per screen, at most.
    #[default]
    Primary,
    /// An ordinary action sitting beside others.
    Secondary,
    /// Something destructive, which should look like it.
    Danger,
    /// An action drawn as its icon alone, in a row of others: no fill and no
    /// border, so the row stays a row, and hover to say it can be pressed.
    Ghost,
}

impl ButtonVariant {
    /// The classes a control in this variant wears: what every button wears,
    /// this variant's own, and the caller's, merged. Public so that a link
    /// which should look like a button can wear them without being one — a
    /// button inside a link is not a thing a browser accepts.
    #[must_use]
    pub fn styled(self, extra: &str) -> String {
        tw_merge!(BASE, self.class(), extra)
    }

    /// The classes this variant renders with.
    const fn class(self) -> &'static str {
        match self {
            Self::Primary => "bg-primary text-primary-foreground hover:bg-primary/90",
            Self::Secondary => {
                "bg-surface text-foreground border border-border hover:bg-surface-muted"
            }
            Self::Danger => "bg-failed text-primary-foreground hover:bg-failed/90",
            Self::Ghost => "text-muted-foreground hover:bg-surface-muted hover:text-foreground",
        }
    }
}

/// What every button wears, whatever its variant.
const BASE: &str = "inline-flex items-center justify-center gap-2 rounded-md px-3 py-1.5 text-sm \
                    font-medium transition-colors disabled:pointer-events-none disabled:opacity-50 \
                    focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary \
                    focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// Properties for [`Button`].
#[derive(Props, PartialEq, Clone)]
pub struct ButtonProps {
    /// How much weight it carries.
    #[props(default)]
    pub variant: ButtonVariant,
    /// Refuses to be pressed, and says so.
    #[props(default = false)]
    pub disabled: bool,
    /// Extra classes, merged over the ones above.
    #[props(default)]
    pub class: String,
    /// What pressing it does.
    pub onclick: Option<EventHandler<MouseEvent>>,
    /// Anything else a caller wants on the element.
    #[props(extends = button, extends = GlobalAttributes)]
    pub attributes: Vec<Attribute>,
    /// The label.
    pub children: Element,
}

/// A button.
#[component]
pub fn Button(props: ButtonProps) -> Element {
    let disabled = props.disabled;
    rsx! {
        button {
            // Explicit, because a button inside a form defaults to submitting
            // it — which is a navigation nobody asked for.
            r#type: "button",
            class: props.variant.styled(&props.class),
            disabled,
            onclick: move |event| {
                // Checked here as well as by the attribute: `disabled` stops a
                // real click, and not a handler fired some other way.
                if !disabled && let Some(handler) = props.onclick {
                    handler.call(event);
                }
            },
            ..props.attributes,
            {props.children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ButtonVariant;
    use std::collections::BTreeSet;

    /// Every variant there is.
    ///
    /// Listed by hand, with the same weakness as the equivalent list for
    /// tones: a variant added without a line here is unchecked.
    const EVERY: &[ButtonVariant] = &[
        ButtonVariant::Primary,
        ButtonVariant::Secondary,
        ButtonVariant::Danger,
        ButtonVariant::Ghost,
    ];

    /// A styled control wears what every button wears, its variant's own,
    /// and the caller's, merged — which a link dressed as a button relies
    /// on as much as a button does.
    #[test]
    fn a_styled_control_wears_all_three() {
        let worn = ButtonVariant::Secondary.styled("px-2");
        assert!(worn.contains("inline-flex"), "{worn}");
        assert!(worn.contains("border-border"), "{worn}");
        assert!(worn.contains("px-2") && !worn.contains("px-3"), "{worn}");
        assert!(ButtonVariant::Primary.styled("").contains("bg-primary"));
    }

    #[test]
    fn every_variant_renders_as_something() {
        for variant in EVERY {
            assert!(
                !variant.class().is_empty(),
                "{variant:?} renders as nothing"
            );
        }
    }

    /// A destructive action that looks like an ordinary one is the failure
    /// this prevents, and it is the reason variants exist at all.
    ///
    /// Counted through a set, for the reason given on the equivalent test for
    /// tones.
    #[test]
    fn no_two_variants_look_the_same() {
        let distinct: BTreeSet<&str> = EVERY.iter().map(|variant| variant.class()).collect();

        assert_eq!(
            distinct.len(),
            EVERY.len(),
            "two variants render identically: {EVERY:?} produced {distinct:?}"
        );
    }
}
