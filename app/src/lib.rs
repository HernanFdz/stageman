#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! This crate serves the dashboard and runs the orchestrator in the same
//! process. It operates the instance and never talks to a job: conversation
//! belongs to a channel, so no conversational state lives here — see
//! `docs/decisions/0005-conversation-happens-on-channels.md`.
//!
//! There is no binary target yet. It arrives with the server, and adding one
//! now would mean a `main` that starts nothing.
