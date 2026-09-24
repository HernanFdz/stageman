//! A choice from a long list: a box that filters and holds the choice, and
//! rows shown while it is typed in.
//!
//! What the one long set here is chosen from, per `docs/conventions.md`
//! §3 and
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`:
//! the repositories an access reaches. Its list is in the page rather than
//! floating, a handful tall and scrolling beyond, grouped where the items
//! say a group. The browser's own select is refused for the reason the
//! segmented control refuses it, and the registry's combobox for the price
//! the record measured.
//!
//! **Controlled**, like every control here: it says which item was chosen
//! and writes nowhere. What it keeps for itself is what is being typed and
//! which row the keyboard is on, which are nobody else's business.

use dioxus::prelude::*;

use super::{FIELD, Icon};

/// One thing a combobox offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComboboxItem {
    /// What the browser sends back.
    pub id: String,
    /// What a person reads, and what the filter matches.
    pub label: String,
    /// Which group it is under, where the list has groups.
    pub group: Option<String>,
    /// An icon beside it, and what the icon says for whoever cannot see it.
    pub icon: Option<(Icon, String)>,
}

/// Properties for [`Combobox`].
#[derive(Props, PartialEq, Clone)]
pub struct ComboboxProps {
    /// What the choice is called, for whoever cannot see it.
    pub label: String,
    /// What can be chosen, in the order it is shown.
    pub items: Vec<ComboboxItem>,
    /// Which is chosen, by what the browser sends back; empty for none.
    pub value: String,
    /// What the box says while nothing is chosen or typed.
    #[props(default)]
    pub placeholder: String,
    /// Refuses a choice, and says so.
    #[props(default = false)]
    pub disabled: bool,
    /// Whether the list is still being asked for, which the box says with
    /// the mark that says working.
    #[props(default = false)]
    pub busy: bool,
    /// Told what the browser sends back for whichever is chosen.
    pub onchange: EventHandler<String>,
}

/// How many rows are in view before the list scrolls.
const IN_VIEW: &str = "max-h-64";

/// A box that filters a list, and the list.
#[component]
pub fn Combobox(props: ComboboxProps) -> Element {
    // What is being typed, while the list is open; the choice's label is
    // what the box shows otherwise.
    let mut query = use_signal(String::new);
    let mut open = use_signal(|| false);
    // Which of the rows shown the keyboard is on, from nought.
    let mut active = use_signal(|| 0_usize);
    let onchange = props.onchange;
    let items = props.items.clone();
    // What the box shows while nothing is typed: the choice's label, or
    // the value as it stands where no item carries it yet — a page
    // rendered before the list arrived still says what is held.
    let chosen = props
        .items
        .iter()
        .find(|item| item.id == props.value)
        .map_or_else(|| props.value.clone(), |item| item.label.clone());
    let shown: Vec<ComboboxItem> = if open() {
        filtered(&props.items, &query())
    } else {
        Vec::new()
    };
    let rows = shown.len();
    // Each row with whether it heads its group: the group's name is shown
    // once, above its first row, since the rows come grouped.
    let mut headed: Vec<(usize, ComboboxItem, bool)> = Vec::with_capacity(rows);
    let mut last_group: Option<String> = None;
    for (position, item) in shown.iter().enumerate() {
        let heads = item.group.is_some() && item.group != last_group;
        last_group.clone_from(&item.group);
        headed.push((position, item.clone(), heads));
    }
    let listing = open() && !props.disabled;
    let list_id = format!("combobox-{}", props.label.to_lowercase().replace(' ', "-"));
    // Copied into every row's press and the box's key handler alike, which
    // a callback is and a closure is not.
    let pick = Callback::new(move |id: String| {
        onchange.call(id);
        open.set(false);
        query.set(String::new());
    });

    rsx! {
        div { class: "flex flex-col gap-1",
            div { class: "relative",
            input {
                r#type: "text",
                role: "combobox",
                class: "{FIELD} font-mono pr-8",
                aria_label: "{props.label}",
                aria_autocomplete: "list",
                aria_expanded: if listing { "true" } else { "false" },
                aria_controls: "{list_id}",
                aria_activedescendant: if listing && rows > 0 { format!("{list_id}-{}", active()) } else { String::new() },
                placeholder: "{props.placeholder}",
                disabled: props.disabled,
                autocomplete: "off",
                value: if open() { query() } else { chosen },
                onfocus: move |_| {
                    query.set(String::new());
                    active.set(0);
                    open.set(true);
                },
                onblur: move |_| open.set(false),
                oninput: move |event| {
                    query.set(event.value());
                    active.set(0);
                    open.set(true);
                },
                onkeydown: {
                    move |event: KeyboardEvent| {
                        let shown = filtered(&items, &query());
                        match event.key() {
                            Key::ArrowDown => {
                                event.prevent_default();
                                open.set(true);
                                active.set(next(active(), shown.len()));
                            }
                            Key::ArrowUp => {
                                event.prevent_default();
                                active.set(previous(active(), shown.len()));
                            }
                            Key::Enter => {
                                if let Some(item) = shown.get(active()) {
                                    event.prevent_default();
                                    pick.call(item.id.clone());
                                }
                            }
                            Key::Escape => {
                                open.set(false);
                                query.set(String::new());
                            }
                            _ => {}
                        }
                    }
                },
            }
            if props.busy {
                span {
                    class: "pointer-events-none absolute inset-y-0 right-2 inline-flex items-center \
                            text-muted-foreground",
                    aria_hidden: "true",
                    {Icon::Loading.draw(14)}
                }
            }
            }
            if listing {
                ul {
                    id: "{list_id}",
                    role: "listbox",
                    aria_label: "{props.label}",
                    class: "{IN_VIEW} overflow-y-auto rounded-md border border-border bg-surface \
                            py-1 text-sm",
                    if rows == 0 {
                        li { role: "presentation", class: "px-2 py-1 text-xs text-faint-foreground",
                            "Nothing matches."
                        }
                    }
                    for (position, item, heads) in headed.iter().cloned() {
                        // One keyed node per row, holding the group's name
                        // where the row heads its group.
                        Fragment { key: "{item.id}",
                            if heads {
                                li {
                                    role: "presentation",
                                    class: "px-2 pb-0.5 pt-1.5 text-xs font-medium text-muted-foreground",
                                    "{item.group.clone().unwrap_or_default()}"
                                }
                            }
                            li {
                                id: "{list_id}-{position}",
                            role: "option",
                            aria_selected: if item.id == props.value { "true" } else { "false" },
                            class: if position == active() {
                                "bg-surface-muted"
                            } else {
                                ""
                            },
                            button {
                                r#type: "button",
                                tabindex: "-1",
                                class: "flex w-full items-center gap-2 px-2 py-1 text-left font-mono \
                                        hover:bg-surface-muted",
                                // Pressed rather than clicked, and the press
                                // swallowed, so the box keeps its focus and
                                // the list is not closed by the blur a click
                                // would cause first.
                                onmousedown: move |event| event.prevent_default(),
                                onclick: {
                                    let id = item.id.clone();
                                    move |_| pick.call(id.clone())
                                },
                                if let Some((icon, says)) = item.icon.clone() {
                                    span { class: "inline-flex text-muted-foreground", aria_label: "{says}", title: "{says}",
                                        {icon.draw(14)}
                                    }
                                }
                                span { class: if item.id == props.value { "font-medium" } else { "" },
                                    "{item.label}"
                                }
                            }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The items whose label or group holds what is typed, case apart; every
/// item while nothing is.
fn filtered(items: &[ComboboxItem], query: &str) -> Vec<ComboboxItem> {
    let wanted = query.trim().to_lowercase();
    items
        .iter()
        .filter(|item| {
            wanted.is_empty()
                || item.label.to_lowercase().contains(&wanted)
                || item
                    .group
                    .as_ref()
                    .is_some_and(|group| group.to_lowercase().contains(&wanted))
        })
        .cloned()
        .collect()
}

/// The row after this one, wrapping at the end; nought where there are
/// none.
const fn next(active: usize, rows: usize) -> usize {
    match (active.checked_add(1), rows.checked_sub(1)) {
        (Some(after), Some(last)) if after <= last => after,
        _ => 0,
    }
}

/// The row before this one, wrapping at the start; nought where there are
/// none.
fn previous(active: usize, rows: usize) -> usize {
    match (active.checked_sub(1), rows.checked_sub(1)) {
        (Some(before), Some(last)) => before.min(last),
        (None, Some(last)) => last,
        (_, None) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{ComboboxItem, filtered, next, previous};

    fn item(id: &str, group: Option<&str>) -> ComboboxItem {
        ComboboxItem {
            id: id.to_owned(),
            label: id.to_owned(),
            group: group.map(str::to_owned),
            icon: None,
        }
    }

    /// The filter matches the label or the group, case apart, and matches
    /// everything while nothing is typed.
    #[test]
    fn the_filter_matches_label_or_group_and_everything_when_blank() {
        let items = vec![
            item("acme/site", Some("acme")),
            item("example/aviary", Some("example")),
            item("example/burrow", Some("example")),
        ];
        let ids = |query: &str| {
            filtered(&items, query)
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(""), ["acme/site", "example/aviary", "example/burrow"]);
        assert_eq!(ids("  "), ["acme/site", "example/aviary", "example/burrow"]);
        assert_eq!(ids("AVI"), ["example/aviary"]);
        assert_eq!(ids("acme"), ["acme/site"]);
        assert!(ids("nothing").is_empty());
    }

    /// The keyboard wraps at both ends, and stays put with no rows.
    #[test]
    fn the_keyboard_wraps_at_both_ends() {
        assert_eq!(next(0, 3), 1);
        assert_eq!(next(2, 3), 0);
        assert_eq!(next(5, 0), 0);
        assert_eq!(previous(1, 3), 0);
        assert_eq!(previous(0, 3), 2);
        assert_eq!(previous(0, 0), 0);
    }
}
