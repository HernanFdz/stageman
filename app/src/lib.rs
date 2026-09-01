#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! This crate serves the dashboard and runs the foreman in the same
//! process. It operates the instance and never talks to a job: conversation
//! belongs to a channel, so no conversational state lives here — see
//! `docs/decisions/0005-conversation-happens-on-channels.md`.
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
pub(crate) mod endpoint;
#[cfg(feature = "server")]
mod instance;
#[cfg(feature = "server")]
mod listening;
#[cfg(feature = "server")]
pub mod release;
#[cfg(feature = "server")]
mod serving;
#[cfg(feature = "server")]
pub(crate) mod tooling;

pub mod dashboard;
pub mod ui;

pub use dashboard::Dashboard;

#[cfg(feature = "server")]
pub use instance::{
    LoadError, RunError, SaveError, Started, StateGuard, StateRef, Store, Swept, attend, begin,
    deliver, reconcile, run, supervise,
};
#[cfg(feature = "server")]
pub use listening::{listen, listen_to, listening_on};
#[cfg(feature = "server")]
pub use serving::{RUNTIME, SESSIONS, serve};
