//! Light, dark, or whatever the system says.
//!
//! The dark theme is the same tokens with other values under one class on the
//! root, chosen in three states, kept in the browser, and applied before the
//! first paint — see `docs/decisions/0072-the-dashboard-has-a-dark-theme.md`.
//! All of that mechanism is here: the one script the dashboard writes by
//! hand, the choice, and the control that cycles it.

use dioxus::prelude::*;

use super::{Button, ButtonVariant, Icon, Tooltip};

/// The script the shell puts in the head, run before the body is parsed.
///
/// It reads the choice, sets the class the dark tokens hang off, and leaves a
/// small object on the window for the control below to ask and tell. Kept to
/// what it is: anything more belongs in Rust, and a component that seems to
/// need more script is a component that needs a token.
pub const SCRIPT: &str = r#"(function () {
  var key = "stageman-theme";
  var system = window.matchMedia("(prefers-color-scheme: dark)");
  function chosen() { return localStorage.getItem(key) || "system"; }
  function apply(choice) {
    var dark = choice === "dark" ? true : choice === "light" ? false : system.matches;
    document.documentElement.classList.toggle("dark", dark);
  }
  window.stagemanTheme = {
    chosen: chosen,
    choose: function (choice) {
      if (choice === "system") { localStorage.removeItem(key); } else { localStorage.setItem(key, choice); }
      apply(choice);
    }
  };
  apply(chosen());
  system.addEventListener("change", function () { apply(chosen()); });
})();"#;

/// What the control asks the browser, once the page is awake.
///
/// Sent through the channel rather than returned, and that is a rule rather
/// than a style — see `docs/conventions.md` §3: the framework closes the
/// channel after the script, so a `return` skips the close, leaks the
/// channel, and has Firefox warn about unreachable code on every page.
const ASKING: &str =
    r#"dioxus.send(window.stagemanTheme ? window.stagemanTheme.chosen() : "system");"#;

/// The three ways the look can be chosen.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    /// Whatever the platform prefers, changing when it does.
    #[default]
    System,
    /// Light, whatever the platform prefers.
    Light,
    /// Dark, likewise.
    Dark,
}

impl Theme {
    /// Every choice there is, in the order the control goes round them.
    pub const ALL: &'static [Self] = &[Self::System, Self::Light, Self::Dark];

    /// How the script spells it, and what the browser keeps.
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The choice the browser spelled, if it is one.
    #[must_use]
    pub fn spelled(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|theme| theme.spelling() == text)
    }

    /// The one after this, round the circle.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    /// What to call it on screen.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The icon that says which this is.
    #[must_use]
    pub const fn icon(self) -> Icon {
        match self {
            Self::System => Icon::ThemeSystem,
            Self::Light => Icon::ThemeLight,
            Self::Dark => Icon::ThemeDark,
        }
    }
}

/// The control that cycles the look: system, light, dark, and round again.
///
/// It starts on *system* whatever the browser holds, because the server
/// renders it and cannot know. The browser is asked once the page is awake,
/// and the answer moves the icon — only the icon, since the script has
/// already applied the choice by then.
#[component]
pub fn ThemeToggle() -> Element {
    let mut theme = use_signal(Theme::default);

    use_effect(move || {
        spawn(async move {
            // On the server there is no evaluator and this is an error, which
            // leaves the default: the server does not know, and the script
            // decides.
            let mut asked = document::eval(ASKING);
            if let Ok(spelling) = asked.recv::<String>().await
                && let Some(chosen) = Theme::spelled(&spelling)
            {
                theme.set(chosen);
            }
        });
    });

    let current = theme();
    let coming = current.next();

    rsx! {
        Tooltip { text: "Theme: {current.label()}. Click for {coming.label()}.",
            Button {
                variant: ButtonVariant::Ghost,
                class: "px-1.5 py-1",
                aria_label: "Theme: {current.label()}",
                onclick: move |_| {
                    let chosen = theme().next();
                    theme.set(chosen);
                    // The script keeps the choice and applies it; this only
                    // says which.
                    let spelling = chosen.spelling();
                    let _told = document::eval(&format!("window.stagemanTheme.choose(\"{spelling}\");"));
                },
                {current.icon().draw(16)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ASKING, SCRIPT, Theme};
    use std::collections::BTreeSet;

    /// The script and the control name the same things, so a rename on
    /// either side fails here rather than in a browser.
    #[test]
    fn the_script_and_the_control_agree() {
        assert!(SCRIPT.contains("window.stagemanTheme = {"), "{SCRIPT}");
        assert!(ASKING.contains("window.stagemanTheme.chosen()"), "{ASKING}");
        assert!(
            ASKING.starts_with("dioxus.send(") && !ASKING.contains("return"),
            "an answer is sent, never returned: {ASKING}"
        );
        assert!(
            SCRIPT.contains(r#"classList.toggle("dark""#),
            "the class the dark tokens hang off: {SCRIPT}"
        );
        for theme in Theme::ALL {
            let quoted = format!("\"{}\"", theme.spelling());
            assert!(
                SCRIPT.contains(&quoted),
                "the script never spells {theme:?}"
            );
        }
    }

    #[test]
    fn every_choice_spells_and_reads_back_and_reads_apart() {
        for theme in Theme::ALL {
            assert_eq!(Theme::spelled(theme.spelling()), Some(*theme));
            assert!(!theme.label().is_empty());
        }
        let labels: BTreeSet<&str> = Theme::ALL.iter().map(|theme| theme.label()).collect();
        assert_eq!(labels.len(), Theme::ALL.len(), "two choices read alike");
        let icons: BTreeSet<String> = Theme::ALL
            .iter()
            .map(|theme| format!("{:?}", theme.icon()))
            .collect();
        assert_eq!(icons.len(), Theme::ALL.len(), "two choices look alike");
    }

    /// The control goes round all three and comes back.
    #[test]
    fn the_choices_go_round() {
        assert_eq!(Theme::System.next(), Theme::Light);
        assert_eq!(Theme::Light.next(), Theme::Dark);
        assert_eq!(Theme::Dark.next(), Theme::System);
        assert_eq!(Theme::default(), Theme::System, "the server's answer");
    }

    /// What the browser did not say, or said wrongly, is not a choice.
    #[test]
    fn a_spelling_that_is_not_a_choice_reads_as_none() {
        assert_eq!(Theme::spelled(""), None);
        assert_eq!(Theme::spelled("purple"), None);
        assert_eq!(Theme::spelled("Dark"), None, "spellings are exact");
    }
}
