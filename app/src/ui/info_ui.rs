//! More about a field, for whoever hovers or focuses the control beside its
//! label.
//!
//! A tooltip rather than a disclosure, per `docs/conventions.md` §3. It was
//! a `details` element — opened by a click, closed by nothing but a second
//! click, so that several stood open at once and each stood for ever. Now
//! the browser's own states show it: a pointer that hovers, and a focus
//! that came from the keyboard. Two things a paragraph earns over the
//! tooltip that repeats a control's name: it is the control's description
//! to assistive technology, because it says what the label does not; and
//! Escape dismisses it while the control has focus, until the pointer
//! leaves or focus moves on, which is the one thing the browser's states
//! cannot do and the only script here. One thing it does not earn: the
//! text lets the pointer through rather than holding it, because it sits
//! over whatever is under the label, and a pointer moving down to reach
//! that would find the text in the way — measured, with the next label's
//! own control unreachable beneath it. A person who wants it to hold has
//! the keyboard, where focus keeps it until Escape or the next Tab. On a
//! touch screen a tap stands in for hover and a tap elsewhere for leaving,
//! which is the browser's doing.

use dioxus::prelude::*;

use super::Icon;

/// Properties for [`Info`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct InfoProps {
    /// The rest of what there is to say.
    pub text: String,
}

/// An information control, and what it says on hover or keyboard focus.
#[component]
pub fn Info(props: InfoProps) -> Element {
    // Escape hides what hover or focus is showing, until the pointer leaves
    // or focus moves on: the one thing the browser's own states cannot do.
    let mut dismissed = use_signal(|| false);

    rsx! {
        span {
            class: "group relative inline-flex",
            onmouseleave: move |_| dismissed.set(false),
            button {
                r#type: "button",
                class: "rounded text-muted-foreground hover:text-foreground focus-visible:outline-none \
                        focus-visible:ring-2 focus-visible:ring-primary",
                aria_label: "More about this",
                // The text again, as the control's description, so that a
                // screen reader says it on reaching the control; what is
                // drawn below is hidden from one, since it would be twice.
                "aria-description": "{props.text}",
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        dismissed.set(true);
                    }
                },
                onblur: move |_| dismissed.set(false),
                {Icon::Info.draw(14)}
            }
            div {
                aria_hidden: "true",
                // Invisible rather than merely transparent while hidden, so
                // that it is read by nobody; and no pointer target even
                // while shown, so that it never stands between the pointer
                // and what is under it. Shown a moment late, for the reason
                // a tooltip is, and hidden at once.
                class: if dismissed() {
                    "pointer-events-none invisible absolute left-0 top-full z-20 mt-1.5 opacity-0"
                } else {
                    "pointer-events-none invisible absolute left-0 top-full z-20 mt-1.5 opacity-0 \
                     motion-safe:transition-[opacity,visibility] motion-safe:duration-150 \
                     group-hover:visible group-hover:opacity-100 group-hover:delay-150 \
                     group-has-[:focus-visible]:visible group-has-[:focus-visible]:opacity-100"
                },
                div {
                    class: "w-80 rounded-md border border-border bg-surface p-3 text-xs \
                            leading-relaxed text-muted-foreground shadow-md",
                    "{props.text}"
                }
            }
        }
    }
}
