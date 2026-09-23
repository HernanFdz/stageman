//! One icon per concept, decided here and nowhere else.
//!
//! A screen says what it means — add, close, stop — and this says what that
//! looks like, so that a foreman is the same shape on every page and one
//! control is never two glyphs on two rows. Every icon is lucide's, per
//! `docs/conventions.md` §3. The text glyphs that used to stand in for some of
//! these are gone: a glyph with an emoji presentation arrives in colour on the
//! platforms that give it one, and two vocabularies on one page read as an
//! accident.
//!
//! The crate compiles icons by category, so an icon that will not resolve
//! usually means a category to add in the manifest rather than an icon that
//! does not exist; the manifest says which are compiled and why.

use dioxus::prelude::*;
use lucide_dioxus::{
    Activity, ArrowLeft, Bot, Calendar, Check, ChevronRight, CircleCheck, CircleOff, CircleX,
    ClipboardPaste, ExternalLink, Eye, EyeOff, GitPullRequest, Hammer, HardHat, Info, LoaderCircle,
    Moon, Pencil, Plus, Square, Sun, SunMoon, Trash2, X,
};

/// A concept a screen can point at, and nothing about how it is drawn.
///
/// Named for meaning rather than shape, as a badge's tone is: a screen that
/// said *plus* would be choosing a glyph, and a screen that says *add* is
/// choosing nothing.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Icon {
    /// One more of something: a project, a kit, a variable, a job.
    Add,
    /// Commit what a form holds.
    Save,
    /// Leave without committing, or dismiss what is over the page.
    Close,
    /// Change what is already there.
    Edit,
    /// Take a row away.
    Remove,
    /// Show what is folded away.
    Reveal,
    /// Fold it away again.
    Hide,
    /// Go and look, somewhere that is not this page.
    Look,
    /// Stop what is running, keeping everything.
    Stop,
    /// A person's verdict that it produced what was wanted.
    Done,
    /// A person's verdict that it did not.
    Discard,
    /// Over, with nothing left to do about it.
    Over,
    /// Still being read.
    Loading,
    /// The look follows the system.
    ThemeSystem,
    /// The light look.
    ThemeLight,
    /// The dark look.
    ThemeDark,
    /// There is more to say about this, for whoever asks.
    Info,
    /// An agent nothing here has a mark for.
    Agent,
    /// The foreman: the one that reads what a person says and decides.
    Foreman,
    /// What a job runs on: its kit, and on a project the kits its jobs may.
    Kit,
    /// Where a job has got to.
    Standing,
    /// When something was made.
    Made,
    /// The way back to where a page was reached from.
    Back,
    /// A pull request a job opened.
    PullRequest,
    /// There is more here, folded away: the trigger of a disclosure, which
    /// turns to point down at what it opened.
    Disclose,
    /// Paste a file's worth of something, rather than typing it a row at
    /// a time.
    Paste,
}

impl Icon {
    /// Every icon there is.
    ///
    /// Listed by hand, which is the weakness the tests below share with the
    /// badge's: a variant added without a line here is a variant nothing
    /// checks.
    pub const ALL: &'static [Self] = &[
        Self::Add,
        Self::Save,
        Self::Close,
        Self::Edit,
        Self::Remove,
        Self::Reveal,
        Self::Hide,
        Self::Look,
        Self::Stop,
        Self::Done,
        Self::Discard,
        Self::Over,
        Self::Loading,
        Self::ThemeSystem,
        Self::ThemeLight,
        Self::ThemeDark,
        Self::Info,
        Self::Agent,
        Self::Foreman,
        Self::Kit,
        Self::Standing,
        Self::Made,
        Self::Back,
        Self::PullRequest,
        Self::Disclose,
        Self::Paste,
    ];

    /// Drawn at `size` pixels, in the current text colour.
    ///
    /// It never shrinks: an icon beside text keeps its size and the text
    /// wraps, which is what a row of controls needs.
    ///
    /// # Errors
    ///
    /// None. An element is a result because a component can fail to render,
    /// and a fixed drawing cannot; the type is the framework's.
    pub fn draw(self, size: usize) -> Element {
        let class = "shrink-0";
        match self {
            Self::Add => rsx! { Plus { size, class } },
            Self::Save => rsx! { Check { size, class } },
            Self::Close => rsx! { X { size, class } },
            Self::Edit => rsx! { Pencil { size, class } },
            Self::Remove => rsx! { Trash2 { size, class } },
            Self::Reveal => rsx! { Eye { size, class } },
            Self::Hide => rsx! { EyeOff { size, class } },
            Self::Look => rsx! { ExternalLink { size, class } },
            Self::Stop => rsx! { Square { size, class } },
            Self::Done => rsx! { CircleCheck { size, class } },
            Self::Discard => rsx! { CircleX { size, class } },
            Self::Over => rsx! { CircleOff { size, class } },
            // Turning, which is the one motion here that says something on
            // its own — and still, under the reduced-motion preference.
            Self::Loading => {
                rsx! { LoaderCircle { size, class: "shrink-0 motion-safe:animate-spin" } }
            }
            Self::ThemeSystem => rsx! { SunMoon { size, class } },
            Self::ThemeLight => rsx! { Sun { size, class } },
            Self::ThemeDark => rsx! { Moon { size, class } },
            Self::Info => rsx! { Info { size, class } },
            Self::Agent => rsx! { Bot { size, class } },
            // A hat, because the word is a role and the hat is what the role
            // wears — see `docs/conventions.md` §2 on why it is a foreman.
            Self::Foreman => rsx! { HardHat { size, class } },
            // A hammer, because a job is the work and a kit is what it is
            // done with, beside the hat that assigns it.
            Self::Kit => rsx! { Hammer { size, class } },
            Self::Standing => rsx! { Activity { size, class } },
            Self::Made => rsx! { Calendar { size, class } },
            Self::Back => rsx! { ArrowLeft { size, class } },
            Self::PullRequest => rsx! { GitPullRequest { size, class } },
            Self::Disclose => rsx! { ChevronRight { size, class } },
            Self::Paste => rsx! { ClipboardPaste { size, class } },
        }
    }
}

// On the daemon's half only, because the renderer these assert through is one
// of the things the manifest keeps out of the browser.
#[cfg(all(test, feature = "server"))]
mod tests {
    use super::Icon;
    use std::collections::BTreeSet;

    fn drawn(icon: Icon) -> String {
        dioxus::ssr::render_element(icon.draw(16))
    }

    #[test]
    fn every_icon_draws_as_a_picture_of_the_size_asked() {
        for icon in Icon::ALL {
            let svg = drawn(*icon);
            assert!(svg.contains("<svg"), "{icon:?} drew nothing: {svg}");
            assert!(
                svg.contains(r#"width="16""#),
                "{icon:?} ignored its size: {svg}"
            );
        }
    }

    /// Two concepts that look identical are two concepts nobody can tell
    /// apart, which is the failure a vocabulary exists to prevent.
    #[test]
    fn no_two_icons_look_the_same() {
        let distinct: BTreeSet<String> = Icon::ALL.iter().map(|icon| drawn(*icon)).collect();

        assert_eq!(
            distinct.len(),
            Icon::ALL.len(),
            "two icons draw identically: {:?}",
            Icon::ALL
        );
    }
}
