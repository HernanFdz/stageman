//! A labelled row of a form: the label, one line about it, the control, and
//! what is wrong with it.
//!
//! One visible line per field, under the label, per `docs/conventions.md`
//! §3, so that every field reads as a titled thing with its line as the
//! subtitle. What adds to a list sits at the end of the label's line. A
//! problem takes the line's place rather than adding to it, so a field that
//! is wrong says one thing, where the eye already looks.

use dioxus::prelude::*;

use super::Info;

/// What every text box and area looks like.
///
/// A constant rather than a component, because the thing being shared is the
/// appearance of a box and not its behaviour — an input and a textarea differ
/// in everything except how they should look.
pub const FIELD: &str = "w-full rounded-md border border-border bg-surface px-2 py-1.5 text-sm \
                         placeholder:text-faint-foreground focus-visible:outline-none \
                         focus-visible:ring-2 focus-visible:ring-primary";

/// What a control beside a box wears, and what sits at the end of a label's
/// line: the box's height, and square.
///
/// The height is [`FIELD`]'s — a line of small text, its padding twice, and
/// a border each side — and has to move with it, because a shorter control
/// beside a box reads as a misalignment rather than as a smaller control.
pub const BESIDE: &str = "size-8.5 shrink-0 p-0";

/// Properties for [`Field`].
#[derive(Props, PartialEq, Clone)]
pub struct FieldProps {
    /// What the control is called. A noun.
    pub label: String,
    /// One line saying what it is for, under the label.
    #[props(default)]
    pub note: Option<String>,
    /// The rest, behind a control.
    #[props(default)]
    pub info: Option<String>,
    /// What is wrong with it, if anything, in the note's place.
    #[props(default)]
    pub problem: Option<String>,
    /// What adds to it, at the end of the label's line.
    #[props(default)]
    pub aside: Option<Element>,
    /// The control.
    pub children: Element,
}

/// A labelled row.
#[component]
pub fn Field(props: FieldProps) -> Element {
    let line = said(props.problem, props.note);

    rsx! {
        div { class: "flex flex-col gap-1",
            div { class: "flex items-center justify-between gap-3",
                div { class: "flex flex-col gap-1",
                    div { class: "flex items-center gap-1.5",
                        span { class: "text-xs font-medium text-muted-foreground", "{props.label}" }
                        if let Some(info) = props.info {
                            Info { text: info }
                        }
                    }
                    if let Some(line) = line {
                        {line.draw()}
                    }
                }
                if let Some(aside) = props.aside {
                    div { class: "shrink-0", {aside} }
                }
            }
            {props.children}
        }
    }
}

/// The one line a field shows, and which kind it is.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// What the field is for.
    Said(String),
    /// What is wrong with it.
    Problem(String),
}

impl Line {
    fn draw(self) -> Element {
        match self {
            Self::Said(text) => rsx! {
                p { class: "text-xs text-faint-foreground", "{text}" }
            },
            Self::Problem(text) => rsx! {
                p { role: "alert", class: "text-xs text-failed", "{text}" }
            },
        }
    }
}

/// The one line the field shows: what is wrong with it where something is,
/// and what it is for otherwise — never both, so that a field that is wrong
/// says one thing.
fn said(problem: Option<String>, note: Option<String>) -> Option<Line> {
    problem.map(Line::Problem).or_else(|| note.map(Line::Said))
}

#[cfg(test)]
mod tests {
    use super::{Line, said};

    /// A problem takes the note's place rather than adding to it, and a
    /// field with nothing wrong says what it is for.
    #[test]
    fn a_problem_takes_the_notes_place() {
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(
            said(None, some("what it is for")),
            Some(Line::Said("what it is for".to_owned()))
        );
        assert_eq!(
            said(some("wrong"), some("what it is for")),
            Some(Line::Problem("wrong".to_owned()))
        );
        assert_eq!(
            said(some("wrong"), None),
            Some(Line::Problem("wrong".to_owned()))
        );
        assert_eq!(said(None, None), None);
    }
}
