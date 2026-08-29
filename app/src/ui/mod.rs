//! The dashboard's primitives — the small set every screen is built from.
//!
//! **Compiled for both halves**, like everything a page touches, so nothing
//! here may name the domain — see
//! `docs/decisions/0022-the-browser-never-sees-the-domain.md`. In practice
//! that is easy to honour: a primitive takes strings and numbers and knows
//! nothing about what they mean.
//!
//! One file per component, named for it, and each is a variant enum plus a
//! function. Styling is Tailwind classes chosen from the token set in
//! `tailwind.css` and never a raw colour, so that what a thing *is* survives
//! changing what it looks like —
//! `docs/decisions/0026-the-dashboards-vocabulary-is-a-token-set.md`.
//!
//! The bar for adding one is that a second screen needs it. A primitive with
//! one caller is that caller's markup wearing a costume, and the cost is paid
//! by everyone reading the file looking for the shape it abstracts.

mod badge_ui;
mod button_ui;
mod card_ui;
mod empty_state_ui;
mod modal_ui;

pub use badge_ui::{Badge, BadgeTone};
pub use button_ui::{Button, ButtonVariant};
pub use card_ui::Card;
pub use empty_state_ui::EmptyState;
pub use modal_ui::Modal;
