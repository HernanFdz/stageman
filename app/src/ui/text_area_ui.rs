//! A box that grows with what is typed, holding what the caller keeps.
//!
//! A component rather than a constant, unlike the box beside it, because
//! there is a behaviour to share and not only a look. The framework writes
//! a `value` as an attribute, which an input reads as its initial value and
//! a textarea does not: a textarea's initial value is its text. And a page
//! rendered on the server is hydrated rather than rendered again, which
//! applies no attribute at all. So a textarea rendered on the server with
//! its value in an attribute arrives empty and stays empty until something
//! changes it, while the same box reached by navigating within the page is
//! filled — which is exactly the kind of difference nobody notices until a
//! reload. This writes the value as the element's text as well, once, from
//! what the box first held; every change after that reaches the browser
//! through the attribute, which the browser's half sets as the property.

use dioxus::prelude::*;
use tw_merge::tw_merge;

use super::FIELD;

/// What a text area adds to a box: it grows with what is typed, and can be
/// pulled taller by hand.
const AREA: &str = "field-sizing-content min-h-16 resize-y";

/// Properties for [`TextArea`].
#[derive(Props, PartialEq, Clone)]
pub struct TextAreaProps {
    /// What it holds. Controlled: the caller keeps it, and this shows it.
    pub value: String,
    /// An example of what goes there, shown while it is empty.
    #[props(default)]
    pub placeholder: String,
    /// Extra classes, merged over the box's own.
    #[props(default)]
    pub class: String,
    /// Whether what it holds is secret — a file of variables, say — and so
    /// none of the browser's to complete, to restore, or to check the
    /// spelling of, per `docs/conventions.md` §3. Readable all the same,
    /// where a credential's box is masked: what is pasted here is checked
    /// by eye.
    #[props(default = false)]
    pub holds_secrets: bool,
    /// Told what was typed.
    pub oninput: EventHandler<FormEvent>,
    /// Anything else a caller wants on the element — a name for whoever
    /// cannot see it, most often.
    #[props(extends = textarea, extends = GlobalAttributes)]
    pub attributes: Vec<Attribute>,
}

/// A box that grows.
#[component]
pub fn TextArea(props: TextAreaProps) -> Element {
    // What the box first held, and never anything later: the text is only
    // read by a browser before anybody has typed, and a row that changes
    // what it holds does so through the attribute below.
    let first = use_hook(|| props.value.clone());
    let oninput = props.oninput;

    rsx! {
        textarea {
            class: tw_merge!(FIELD, AREA, props.class),
            // Said only by a box of secrets, and left to the browser by every
            // other: a brief is prose, and wants its spelling checked.
            autocomplete: if props.holds_secrets { "off" },
            spellcheck: if props.holds_secrets { "false" },
            autocapitalize: if props.holds_secrets { "off" },
            "autocorrect": if props.holds_secrets { "off" },
            placeholder: props.placeholder,
            value: "{props.value}",
            dangerous_inner_html: as_text(&first),
            oninput: move |event| oninput.call(event),
            ..props.attributes,
        }
    }
}

/// Text as a textarea's content.
///
/// A textarea's content is read as text rather than markup, so only two
/// characters mean anything there: an ampersand starts a reference, and a
/// less-than could start the tag that ends the element. Both are escaped
/// and nothing else is, on the server and in the browser alike.
fn as_text(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::as_text;

    // On the daemon's half only, for the reason the icon tests give.
    #[cfg(feature = "server")]
    mod drawn {
        use super::super::TextArea;
        use dioxus::prelude::*;

        /// A page holding one box of each kind, so that their handlers are
        /// made where a handler can be: inside something being rendered.
        fn page() -> Element {
            rsx! {
                TextArea { value: "a brief", oninput: |_| {} }
                TextArea { value: "KEY=value", holds_secrets: true, oninput: |_| {} }
            }
        }

        /// A box of secrets tells the browser to keep out of it, and any
        /// other box says nothing, so that prose keeps its spelling checked.
        #[test]
        fn only_a_box_of_secrets_is_kept_from_the_browser() {
            let mut dom = VirtualDom::new(page);
            dom.rebuild_in_place();
            let drawn = dioxus::ssr::render(&dom);
            let (prose, secrets) = drawn
                .split_once("</textarea>")
                .expect("two boxes, one after the other");

            for said in [
                "autocomplete",
                "spellcheck",
                "autocapitalize",
                "autocorrect",
            ] {
                assert!(!prose.contains(said), "{said} on a box of prose: {prose}");
            }
            assert!(secrets.contains(r#"autocomplete="off""#), "{secrets}");
            assert!(secrets.contains(r#"spellcheck="false""#), "{secrets}");
            assert!(secrets.contains(r#"autocapitalize="off""#), "{secrets}");
            assert!(secrets.contains(r#"autocorrect="off""#), "{secrets}");
        }
    }

    /// Whatever a person typed comes back as they typed it, and nothing
    /// they typed can end the box or be read as a reference.
    #[test]
    fn what_a_box_holds_cannot_end_it_or_be_read_as_markup() {
        assert_eq!(as_text("a & b"), "a &amp; b");
        assert_eq!(as_text("</textarea><b>"), "&lt;/textarea>&lt;b>");
        assert_eq!(as_text("&lt;"), "&amp;lt;", "a reference typed is text");
        assert_eq!(
            as_text("plain, with \"quotes\" and 'more'"),
            "plain, with \"quotes\" and 'more'"
        );
    }
}
