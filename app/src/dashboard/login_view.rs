//! The door: one password, entered once, for a session the instance mints
//! — see `docs/decisions/0084-the-instance-authenticates-itself.md`.
//!
//! A page and a form and nothing else. The form posts to a path the
//! instance answers itself, at the door, because a server function's
//! answer cannot set a cookie: its request reaches the instance as a typed
//! value with no headers, and its answer goes back the same way. So this
//! page is drawn by the framework, for the stylesheet and the theme, and
//! the credential never passes through it.
//!
//! Outside the shell, deliberately: a person with no session is shown no
//! navigation, no live mark and no status line, since every one of those
//! reads through routes that refuse them.

use dioxus::prelude::*;

use super::Head;
use crate::ui::{Button, FIELD, Field};

/// Where the form posts: the path the instance answers at the door. A
/// literal rather than the instance's constant, because this half is
/// compiled for the browser, where the instance is not.
const LOGIN_PATH: &str = "/login";

/// The login page.
///
/// `said` is what the last attempt earned, carried in the query by the
/// instance when it sends the browser back here — *wrong*, or *wait* — and
/// `back` is where the person was going, carried through the form so that
/// a session sends them there rather than to the front page.
#[component]
pub fn LoginView(said: String, back: String) -> Element {
    let problem = match said.as_str() {
        "wrong" => Some("That is not the password.".to_owned()),
        "wait" => Some("Too many tries in a row. Wait a moment, then try again.".to_owned()),
        _ => None,
    };

    rsx! {
        Head {}
        main { class: "flex min-h-screen items-center justify-center bg-background px-6 font-sans text-foreground",
            form {
                method: "post",
                action: LOGIN_PATH,
                class: "flex w-full max-w-sm flex-col gap-5 rounded-lg border border-border bg-surface p-6",
                h1 { class: "text-base font-semibold tracking-tight", "stageman" }
                Field {
                    label: "Password".to_owned(),
                    note: Some("The one this instance was given.".to_owned()),
                    problem,
                    input {
                        r#type: "password",
                        name: "password",
                        autofocus: true,
                        autocomplete: "current-password",
                        class: FIELD,
                    }
                }
                input { r#type: "hidden", name: "back", value: back }
                Button { r#type: "submit", "Sign in" }
            }
        }
    }
}
