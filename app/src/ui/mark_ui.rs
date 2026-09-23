//! Whose something is, at icon size: an agent's mark, a platform's, a
//! channel's.
//!
//! A mark says *whose*; an icon says *what* — the distinction
//! `docs/conventions.md` §2 draws. One per agent this build can run, keyed by
//! the identifier the wire uses for it, and a generic one for an identifier
//! this build does not know, so that an instance naming an agent that is gone
//! still draws something rather than nothing; and one per platform and per
//! channel this build knows, keyed the same way, for the links a page makes
//! to them.
//!
//! Vendored as inline drawings rather than fetched, per §3: a mark arrives
//! with the page, and the page reaches out for nothing. Each is the
//! monochrome mark from the Simple Icons collection, released under CC0; the
//! marks themselves are Anthropic's, GitHub's and Slack's, drawn here to say
//! whose something is and for nothing else.
//!
//! Its callers arrive with the chips
//! `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`
//! describes. It lands ahead of them because the vocabulary is decided once,
//! and a mark is part of it.

use dioxus::prelude::*;

use super::Icon;

/// Claude's mark, as one path in a square of twenty-four.
const CLAUDE: &str = "m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z";

/// GitHub's mark, as one path in a square of twenty-four.
const GITHUB: &str = "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12";

/// Slack's mark, as one path in a square of twenty-four.
const SLACK: &str = "M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zm1.271 0a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zm0 1.271a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zm10.122 2.521a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zm-1.268 0a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zm-2.523 10.122a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zm0-1.268a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z";

/// Properties for [`Mark`].
#[derive(Props, PartialEq, Eq, Clone)]
pub struct MarkProps {
    /// Whose: an agent, a platform or a channel, by the identifier the wire
    /// uses for it.
    pub agent: String,
    /// How big, in pixels.
    #[props(default = 16)]
    pub size: usize,
}

/// A mark, or the generic one for an identifier this build cannot name.
#[component]
pub fn Mark(props: MarkProps) -> Element {
    let (name, path) = match props.agent.as_str() {
        "claude" => ("Claude", CLAUDE),
        "github" => ("GitHub", GITHUB),
        "slack" => ("Slack", SLACK),
        _ => return Icon::Agent.draw(props.size),
    };
    rsx! {
        svg {
            role: "img",
            "aria-label": "{name}",
            "viewBox": "0 0 24 24",
            "width": "{props.size}",
            "height": "{props.size}",
            "fill": "currentColor",
            "class": "shrink-0",
            path { "d": path }
        }
    }
}

// On the daemon's half only, for the reason the icon tests give.
#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{CLAUDE, GITHUB, Mark, SLACK};
    use dioxus::prelude::*;

    fn drawn(agent: &str) -> String {
        dioxus::ssr::render_element(rsx! { Mark { agent, size: 20 } })
    }

    /// The identifier is the wire's, so this is where the two are tied.
    #[test]
    fn a_known_agent_draws_its_own_mark() {
        let claude = drawn("claude");
        assert!(claude.contains(CLAUDE), "{claude}");
        assert!(claude.contains(r#"width="20""#), "{claude}");
    }

    /// A platform and a channel have marks of their own, so that a link to
    /// either says whose it is at a glance.
    #[test]
    fn a_platform_and_a_channel_draw_their_own_marks() {
        let github = drawn("github");
        assert!(github.contains(GITHUB), "{github}");
        assert!(github.contains(r#"aria-label="GitHub""#), "{github}");
        let slack = drawn("slack");
        assert!(slack.contains(SLACK), "{slack}");
        assert!(slack.contains(r#"aria-label="Slack""#), "{slack}");
    }

    /// An instance naming an agent this build does not know still draws
    /// something, and not somebody else's mark.
    #[test]
    fn an_unknown_agent_draws_the_generic_mark() {
        let unknown = drawn("something-else");
        assert!(unknown.contains("<svg"), "{unknown}");
        assert!(!unknown.contains(CLAUDE), "{unknown}");
    }
}
