//! A box that takes a secret: a credential, a token, a variable's value.
//!
//! Not a password field, and that is the whole of why this is a component —
//! see `docs/conventions.md` §3. The dashboard has a password of its own, so
//! a browser holds a saved login for its address, and a password-typed box
//! anywhere on it is that login's to fill. Measured, the dashboard's password
//! arrived in the box for a purse and in the box for a project's token, which
//! is sent to its platform to be checked; and a key pasted into one was
//! followed by an offer to replace the saved password with it. So the box is
//! a text box, what keeps a secret off the screen is a style, and what keeps
//! it out of the browser's own keeping is said attribute by attribute below.
//!
//! One component, so that a page cannot build the box some other way: the
//! test at the foot of this file refuses a password-typed box anywhere but
//! the sign-in and the password's own form.

use dioxus::prelude::*;
use tw_merge::tw_merge;

use super::FIELD;

/// What a secret's box adds to a box: monospace, because what is pasted
/// there is checked character by character against where it came from, and
/// the mask, which `tailwind.css` defines.
const SECRET: &str = "font-mono masked";

/// Properties for [`SecretBox`].
#[derive(Props, PartialEq, Clone)]
pub struct SecretBoxProps {
    /// What it holds. Controlled: the caller keeps it, and this shows it,
    /// masked.
    pub value: String,
    /// An example of what goes there — what one begins with, most often —
    /// shown while it is empty, and never masked.
    #[props(default)]
    pub placeholder: String,
    /// Extra classes, merged over the box's own.
    #[props(default)]
    pub class: String,
    /// Told what was typed.
    pub oninput: EventHandler<FormEvent>,
    /// Told of a key pressed in it, for a box that Enter acts in.
    pub onkeydown: Option<EventHandler<KeyboardEvent>>,
    /// Told once the box exists, for one that takes the focus.
    pub onmounted: Option<EventHandler<MountedEvent>>,
    /// Anything else a caller wants on the element — a name for whoever
    /// cannot see it, most often. Not its type, which is decided below: a
    /// spread of the same name renders a second attribute, and a browser
    /// keeps the first.
    #[props(extends = input, extends = GlobalAttributes)]
    pub attributes: Vec<Attribute>,
}

/// A box that takes a secret.
#[component]
pub fn SecretBox(props: SecretBoxProps) -> Element {
    let oninput = props.oninput;
    let onkeydown = props.onkeydown;
    let onmounted = props.onmounted;

    rsx! {
        input {
            // Text, never password: a password field is a saved login's to
            // fill, and to offer to save.
            r#type: "text",
            class: tw_merge!(FIELD, SECRET, props.class),
            // Nothing typed here is the browser's to complete, or to keep
            // for the Back button to restore.
            autocomplete: "off",
            // Nor to underline, which in some browsers means sending it to
            // a spelling service; nor to capitalise or correct.
            spellcheck: "false",
            autocapitalize: "off",
            "autocorrect": "off",
            placeholder: props.placeholder,
            value: "{props.value}",
            oninput: move |event| oninput.call(event),
            onkeydown: move |event| {
                if let Some(handler) = onkeydown {
                    handler.call(event);
                }
            },
            onmounted: move |event| {
                if let Some(handler) = onmounted {
                    handler.call(event);
                }
            },
            ..props.attributes,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    /// Every source file under a directory, with its text.
    fn sources(directory: &Path, found: &mut Vec<(String, String)>) {
        let entries = std::fs::read_dir(directory).expect("the source tree is there");
        for entry in entries {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                sources(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text = std::fs::read_to_string(&path).expect("a source file reads");
                found.push((path.to_string_lossy().into_owned(), text));
            }
        }
    }

    /// The only password-typed boxes are the sign-in's and the three that
    /// change the password. Everything else that takes a secret is the
    /// component above, and a box built some other way fails here — which
    /// has to be a reading of the source, because the boxes that matter
    /// appear only after a press, where no test of a page reaches them.
    #[test]
    fn nothing_types_a_password_but_the_sign_in_and_the_passwords_own_form() {
        // In two halves, so that this file does not hold what it looks for.
        let typed = concat!("r#type: ", "\"password\"");
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut found = Vec::new();
        sources(root, &mut found);
        assert!(found.len() > 10, "the walk found the source tree");

        let typing: BTreeMap<String, usize> = found
            .iter()
            .map(|(path, text)| (path, text.matches(typed).count()))
            .filter(|(_, count)| *count > 0)
            .map(|(path, count)| {
                let relative = path
                    .strip_prefix(&*root.to_string_lossy())
                    .unwrap_or(path)
                    .trim_start_matches('/')
                    .to_owned();
                (relative, count)
            })
            .collect();

        assert_eq!(
            typing,
            BTreeMap::from([
                ("dashboard/instance_view.rs".to_owned(), 3),
                ("dashboard/login_view.rs".to_owned(), 1),
            ]),
            "a box that takes a secret is a SecretBox, per docs/conventions.md §3"
        );
    }

    // On the daemon's half only, for the reason the icon tests give.
    #[cfg(feature = "server")]
    mod drawn {
        use super::super::SecretBox;
        use dioxus::prelude::*;

        /// A page holding one box, so that its handler is made where a
        /// handler can be: inside something being rendered.
        fn page() -> Element {
            rsx! {
                SecretBox {
                    placeholder: "sk-ant-api03-…",
                    value: "sk-ant-api03-a-pasted-secret",
                    aria_label: "The API key",
                    oninput: |_| {},
                }
            }
        }

        /// What a browser is told about the box: that it is text and not a
        /// password, masked, and none of the browser's to complete, correct
        /// or check the spelling of.
        #[test]
        fn a_secrets_box_is_text_masked_and_not_the_browsers_to_keep() {
            let mut dom = VirtualDom::new(page);
            dom.rebuild_in_place();
            let drawn = dioxus::ssr::render(&dom);

            assert!(drawn.contains(r#"type="text""#), "{drawn}");
            assert!(!drawn.contains("password"), "{drawn}");
            assert!(drawn.contains("masked"), "{drawn}");
            assert!(drawn.contains(r#"autocomplete="off""#), "{drawn}");
            assert!(drawn.contains(r#"spellcheck="false""#), "{drawn}");
            assert!(drawn.contains(r#"autocapitalize="off""#), "{drawn}");
            assert!(drawn.contains(r#"autocorrect="off""#), "{drawn}");
            assert!(drawn.contains(r#"placeholder="sk-ant-api03-…""#), "{drawn}");
            assert!(drawn.contains(r#"aria-label="The API key""#), "{drawn}");
        }
    }
}
