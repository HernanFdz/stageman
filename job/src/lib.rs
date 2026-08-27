//! The doing: one isolated workspace, one agent process inside it, and the
//! tools that agent calls to reach anything outside.
//!
//! Three invariants in `docs/architecture.md` §2 are this crate's to keep. A
//! job holds no platform credential. A job has one workspace and one project,
//! and can reach neither another job's nor another project's. And a job never
//! blocks on a terminal — when it needs a human it asks on a channel and stays
//! alive, because nobody is watching that terminal.
//!
//! How an agent is driven is not this crate's business — that contract lives
//! in `stageman-agent`, and which agent ran a given job is recorded on the job
//! itself. What belongs here is everything around the agent: the workspace it
//! runs in, the supervision that ends it, and the tools it reaches the world
//! through.
