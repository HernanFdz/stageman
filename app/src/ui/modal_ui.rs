//! A panel over the page, for one decision at a time.
//!
//! Rendered by the caller rather than holding its own open state — a modal
//! that knew whether it was open would need telling twice, once to exist and
//! once to show, and the second is the one that gets forgotten. Callers write
//! `if showing { Modal { … } }`, which is the same condition the rest of the
//! screen already has to consult.
//!
//! Not the `dialog` element, deliberately: its modal behaviour is opened by a
//! method call rather than by an attribute, so it needs a hand into the DOM
//! after render. That is a real mechanism and this does not yet need what it
//! buys — the top layer, and the browser's own dismissal — where it would cost
//! an effect on every open.

use dioxus::prelude::*;
use tw_merge::tw_merge;

/// Properties for [`Modal`].
#[derive(Props, PartialEq, Clone)]
pub struct ModalProps {
    /// What this is asking about, shown and announced.
    pub title: String,
    /// What to do when the reader asks to leave.
    ///
    /// Called for every way out that is not the panel's own contents: the
    /// backdrop, the close control, and the escape key. A caller closes by
    /// acting on this rather than by guessing which one happened.
    pub onclose: EventHandler<()>,
    /// What finishes this, sitting beside the way out of it.
    ///
    /// In the header rather than at the foot of the panel, so that the thing
    /// which commits and the thing which abandons are next to each other and
    /// read as the pair they are.
    #[props(default)]
    pub actions: Option<Element>,
    /// Extra classes for the panel, merged over the ones below.
    #[props(default)]
    pub class: String,
    /// What is being decided.
    pub children: Element,
}

/// A panel over the page.
#[component]
pub fn Modal(props: ModalProps) -> Element {
    let onclose = props.onclose;

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-start justify-center overflow-y-auto \
                    bg-foreground/20 p-4 sm:p-8",
            // The backdrop dismisses, which is the convention everywhere and
            // therefore what a reader will try first.
            onclick: move |_| onclose.call(()),
            div {
                role: "dialog",
                aria_modal: "true",
                aria_label: "{props.title}",
                // Focused so the key handler below receives anything typed
                // before the reader has clicked into a field.
                tabindex: "-1",
                autofocus: true,
                class: tw_merge!(
                    "w-full max-w-lg rounded-lg border border-border bg-surface shadow-lg \
                     focus-visible:outline-none",
                    props.class
                ),
                // Without this, a click anywhere inside the panel reaches the
                // backdrop above and closes what the reader is filling in.
                onclick: move |event| event.stop_propagation(),
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        onclose.call(());
                    }
                },
                header { class: "flex items-baseline justify-between gap-3 border-b border-border px-4 py-3",
                    h2 { class: "text-sm font-semibold text-foreground", "{props.title}" }
                    div { class: "flex shrink-0 items-center gap-2",
                        if let Some(actions) = props.actions {
                            {actions}
                        }
                        button {
                            r#type: "button",
                            class: "text-base leading-none text-muted-foreground hover:text-foreground",
                            aria_label: "Close",
                            onclick: move |_| onclose.call(()),
                            "×"
                        }
                    }
                }
                div { class: "px-4 py-4", {props.children} }
            }
        }
    }
}
