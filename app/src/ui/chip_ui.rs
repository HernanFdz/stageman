//! A chip: one thing shown inline and compact, and usually a link.
//!
//! A chip says which and where and carries no verdict — `docs/conventions.md`
//! §2. The kit's says what a job runs on, or a foreman thinks with, in the
//! space a sentence used to take: the agent's mark, the model's name, and
//! the effort as a meter, with the whole sentence a hover away. The plain
//! one says a number and goes somewhere: a pull request on a job's row.

use dioxus::prelude::*;

use super::{Mark, Tooltip};

/// What every chip looks like.
const CHIP: &str = "inline-flex items-center gap-1.5 rounded-full border border-border bg-surface \
                    px-2 py-0.5 text-xs text-foreground";

/// Properties for [`Chip`].
#[derive(Props, PartialEq, Clone)]
pub struct ChipProps {
    /// What it stands for, in words: the tooltip, and the name for whoever
    /// cannot see it.
    pub says: String,
    /// Where it goes, when it goes somewhere.
    #[props(default)]
    pub link: Option<String>,
    /// What it shows.
    pub children: Element,
}

/// A chip that links where it can, and only says otherwise.
#[component]
pub fn Chip(props: ChipProps) -> Element {
    rsx! {
        Tooltip { text: props.says.clone(),
            if let Some(link) = props.link {
                a {
                    href: "{link}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    class: "{CHIP} hover:border-border-strong hover:bg-surface-muted \
                            focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                    aria_label: "{props.says}",
                    {props.children}
                }
            } else {
                span { class: CHIP, tabindex: "0", aria_label: "{props.says}", {props.children} }
            }
        }
    }
}

/// Properties for [`KitChip`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct KitChipProps {
    /// The kit's own name, where the chip stands for a named kit rather than
    /// for a foreman's setting.
    #[props(default)]
    pub name: Option<String>,
    /// The agent, by the identifier the wire uses.
    pub agent: String,
    /// The agent, as a person reads it.
    pub agent_name: String,
    /// The model, as a person reads it.
    pub model: String,
    /// The effort, as the wire spells it and as a person reads it, where the
    /// model takes one.
    #[props(default)]
    pub effort: Option<(String, String)>,
}

/// A kit, compactly.
#[component]
pub fn KitChip(props: KitChipProps) -> Element {
    let saying = match (&props.name, &props.effort) {
        (Some(name), Some((_, effort))) => {
            format!(
                "{name}: {} · {} · {effort} effort",
                props.agent_name, props.model
            )
        }
        (Some(name), None) => format!("{name}: {} · {}", props.agent_name, props.model),
        (None, Some((_, effort))) => {
            format!("{} · {} · {effort} effort", props.agent_name, props.model)
        }
        (None, None) => format!("{} · {}", props.agent_name, props.model),
    };

    rsx! {
        Tooltip { text: saying.clone(),
            span {
                class: CHIP,
                tabindex: "0",
                aria_label: "{saying}",
                if let Some(name) = props.name {
                    span { class: "font-medium", "{name}" }
                }
                Mark { agent: props.agent, size: 12 }
                span { class: "text-muted-foreground", "{props.model}" }
                // The agent's own default is not a quantity and says nothing
                // beside the model; the sentence a hover away still says it.
                if let Some((id, name)) = props.effort.filter(|(id, _)| id != "default") {
                    Meter { effort: id, name }
                }
            }
        }
    }
}

/// How hard, as a meter, where the effort is one of the steps; as its name
/// where it is not.
#[component]
fn Meter(effort: String, name: String) -> Element {
    level(&effort).map_or_else(
        || {
            rsx! {
                span { class: "text-faint-foreground", "{name}" }
            }
        },
        |level| {
            rsx! {
                span { class: "inline-flex items-center gap-px", aria_hidden: "true",
                    for step in 1..=STEPS {
                        span {
                            key: "{step}",
                            class: if step <= level {
                                "h-2.5 w-1 rounded-sm bg-primary"
                            } else {
                                "h-2.5 w-1 rounded-sm bg-border-strong"
                            },
                        }
                    }
                }
            }
        },
    )
}

/// How many steps the meter has.
const STEPS: u8 = 5;

/// Which step of the meter an effort is, by the wire's spelling of it, and
/// none for a spelling that is not a step — an agent's own default, most of
/// all, which is not a quantity and is shown as its name.
fn level(effort: &str) -> Option<u8> {
    match effort {
        "low" => Some(1),
        "medium" => Some(2),
        "high" => Some(3),
        "xhigh" => Some(4),
        "max" => Some(5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{STEPS, level};

    /// Every step of effort the wire spells is a step on the meter, in
    /// order, and the default is not one.
    #[test]
    fn efforts_climb_the_meter_and_the_default_is_not_a_step() {
        let climbing: Vec<Option<u8>> = ["low", "medium", "high", "xhigh", "max"]
            .iter()
            .map(|effort| level(effort))
            .collect();
        assert_eq!(climbing, [Some(1), Some(2), Some(3), Some(4), Some(5)]);
        assert_eq!(level("max"), Some(STEPS));
        assert_eq!(level("default"), None);
        assert_eq!(level("turbo"), None);
    }
}
