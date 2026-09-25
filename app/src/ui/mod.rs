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
mod chip_ui;
mod combobox_ui;
mod empty_state_ui;
mod field_ui;
mod guide_ui;
mod icon_ui;
mod info_ui;
mod mark_ui;
mod modal_ui;
mod page_header_ui;
mod reference_ui;
mod segmented_ui;
mod skeleton_ui;
mod text_area_ui;
mod theme_ui;
mod tooltip_ui;
mod when_ui;

pub use badge_ui::{Badge, BadgeTone};
pub use button_ui::{Button, ButtonVariant};
pub use card_ui::Card;
pub use chip_ui::{Chip, KitChip};
pub use combobox_ui::{Combobox, ComboboxItem};
pub use empty_state_ui::EmptyState;
pub use field_ui::{BESIDE, FIELD, Field};
pub use guide_ui::Guide;
pub use icon_ui::Icon;
pub use info_ui::Info;
pub use mark_ui::Mark;
pub use modal_ui::Modal;
pub use page_header_ui::PageHeader;
pub use reference_ui::Reference;
pub use segmented_ui::Segmented;
pub use skeleton_ui::Skeleton;
pub use text_area_ui::TextArea;
pub use theme_ui::{SCRIPT as THEME_SCRIPT, Theme, ThemeToggle};
pub use tooltip_ui::Tooltip;
pub use when_ui::When;
