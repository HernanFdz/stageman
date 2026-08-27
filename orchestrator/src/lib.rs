//! The deciding: watch the channels, judge what is worth acting on, and create
//! jobs.
//!
//! Two things live here and nowhere else. Every credential, because a job that
//! cannot hold one cannot leak one. And every kickoff prompt, because a job
//! executes instructions it did not write — which is what makes prompt text
//! reviewable in one place rather than scattered across the system.
//!
//! Both are load-bearing rather than tidy: see `docs/architecture.md` §2 for
//! the credential invariant and `docs/conventions.md` §4 for why prompts are
//! held to a test.
