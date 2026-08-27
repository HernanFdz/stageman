//! The doing: one isolated workspace, one agent process inside it, and the
//! tools that agent calls to reach anything outside.
//!
//! Three invariants in `docs/architecture.md` §2 are this crate's to keep. A
//! job holds no platform credential. A job has one workspace and one project,
//! and can reach neither another job's nor another project's. And a job never
//! blocks on a terminal — when it needs a human it asks on a channel and stays
//! alive, because nobody is watching that terminal.
//!
//! The agent is somebody else's product on somebody else's release cadence, so
//! its quirks stop at this boundary: if a change to it would reach
//! `stageman-core`, the abstraction is in the wrong place.
