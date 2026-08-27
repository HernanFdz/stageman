//! The doing: one isolated workspace, and one agent running inside it with the
//! credentials its project needs.
//!
//! Two invariants in `docs/architecture.md` §2 are this crate's to keep. A job
//! has one workspace and one project, and holds credentials for that project
//! and no other — it can reach neither another job's workspace nor anything
//! belonging to another project. And a job never blocks on a terminal: when it
//! needs a human it asks on a channel and stays alive, because nobody is
//! watching that terminal.
//!
//! How an agent is driven is not this crate's business — that contract lives
//! in `stageman-agent`, and which agent ran a given job is recorded on the job
//! itself. What belongs here is everything around the agent: the workspace it
//! runs in, the credentials it is handed, and the supervision that ends it.
