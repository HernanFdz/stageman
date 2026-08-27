//! The domain: what a project is, what a job is, and the states a job moves
//! through.
//!
//! No I/O, no async runtime, no platform and no framework. This crate names
//! nothing else in the workspace, which is what lets both of the crates above
//! it depend on it without depending on each other — see
//! `docs/architecture.md` §1 for the rule and why it is the one worth
//! defending.
//!
//! The vocabulary this crate exists to express is fixed in
//! `docs/conventions.md` §2, including the words it deliberately avoids.
//!
//! One thing here looks like plumbing and is not: deciding what an agent
//! process is handed. It lives in a crate with no I/O because it is a pure
//! function from configuration to a description of what that process should
//! see, and because it is the only thing standing between an operator and
//! silently paying the wrong way — which makes being able to test it without
//! spawning a process the whole point rather than a convenience. Delivering
//! that description is an adapter's job, and differs per agent.
