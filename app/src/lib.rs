#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! This crate serves the dashboard and runs the foreman in the same
//! process. It operates the instance and never talks to a job: conversation
//! belongs to a channel, so no conversational state lives here — see
//! `docs/decisions/0005-conversation-happens-on-channels.md`.
//!
//! **It is the world.** The instance decides and this crate performs — see
//! `docs/decisions/0056-the-instance-decides-and-the-world-performs.md` —
//! so what is here is a loop, the listeners and servers that turn what
//! happens into events, and the adapters that perform effects.
//!
//! **It is compiled twice, for two machines.** The daemon gets everything;
//! the browser gets [`dashboard`] and nothing else. That split is a feature
//! selection in `Cargo.toml` rather than a `cfg` here, because a `cfg` hides
//! code from the compiler and not a dependency from cargo — see
//! `docs/decisions/0022-the-browser-never-sees-the-domain.md`.

#[cfg(feature = "server")]
mod bundle;
#[cfg(feature = "server")]
mod channel;
#[cfg(feature = "server")]
#[cfg(feature = "server")]
mod listening;
#[cfg(feature = "server")]
mod serving;
#[cfg(feature = "server")]
#[cfg(feature = "server")]
#[cfg(feature = "server")]
pub mod world;

pub mod dashboard;
pub mod ui;

pub use dashboard::Dashboard;

#[cfg(feature = "server")]
#[cfg(feature = "server")]
pub use serving::serve;
#[cfg(feature = "server")]
pub use stageman_instance::release;
#[cfg(feature = "server")]
#[cfg(feature = "server")]
pub use world::asking;
