//! The deciding: watch the channels, judge what is worth acting on, and create
//! jobs.
//!
//! A job is one possible reaction and not the only one: doing nothing, and
//! answering on the channel, are reactions too. Judging is the work here;
//! spawning is a consequence of one particular judgement.
//!
//! One thing lives here and nowhere else: every kickoff prompt, because a job
//! executes instructions it did not write, which is what keeps prompt text
//! reviewable in one place rather than scattered across the system.
//!
//! Credentials are no longer in that set. This crate holds what it needs in
//! order to *watch* a project's channels; a job is handed what it needs in
//! order to *act* on them. Both come from the same project configuration. See
//! `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` for what
//! that gave up, and what it bought.
//!
//! To judge at all, this crate runs an agent itself, the same way a job does
//! and through the same contract — one-shot and structured rather than a
//! session in a workspace.
//!
//! Both are load-bearing rather than tidy: see `docs/architecture.md` §2 for
//! the credential invariant and `docs/conventions.md` §4 for why prompts are
//! held to a test.

/// What a resumed job's agent is told about having been interrupted.
///
/// Composed here because it is an instruction, and every instruction in this
/// system is authored in this crate — `docs/architecture.md` §1 makes that the
/// property which keeps prompt text reviewable in one place. A job never
/// writes its own, and that holds for the second thing it is told as much as
/// for the first.
///
/// It exists because `docs/decisions/0015-a-job-survives-the-daemon-dying.md`
/// measured that an agent works out it was interrupted unaided, and decided to
/// tell it anyway: the notice is nearly free and the alternative is depending
/// on an inference.
///
/// The middle paragraph is the load-bearing one. A job holds its project's
/// credentials since
/// `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md`, so by the
/// time it is interrupted it may already have pushed a branch or posted a
/// comment — and an agent that assumes its last step failed will do that twice.
/// Checking beats assuming in either direction.
const RESUMPTION: &str = "\
You were interrupted: the process supervising you stopped, and you have just \
been restarted. Your instructions have not changed.

Something you had begun may have finished, half-finished, or never started — \
including work outside this workspace, such as a branch pushed or a comment \
posted. Check how things actually stand before you act. Do not assume your \
last step completed, and do not assume it did not.

Then carry on with the work you were given.";

/// What a resumed job's agent is told about having been interrupted.
#[must_use]
pub const fn resumption_notice() -> &'static str {
    RESUMPTION
}

#[cfg(test)]
mod tests {
    use super::resumption_notice;

    /// Asserted as literal text, per `docs/conventions.md` §4. Prompt text is
    /// the only kind of code here that changes behaviour without changing
    /// control flow, so it is also the only kind that can be rewritten
    /// completely without a single other test going red.
    #[test]
    fn the_resumption_notice_reads_exactly_as_written() {
        assert_eq!(
            resumption_notice(),
            "You were interrupted: the process supervising you stopped, and you have just been \
restarted. Your instructions have not changed.

Something you had begun may have finished, half-finished, or never started — including work \
outside this workspace, such as a branch pushed or a comment posted. Check how things actually \
stand before you act. Do not assume your last step completed, and do not assume it did not.

Then carry on with the work you were given."
        );
    }

    /// The paragraph that exists because a job acts on platforms it can reach.
    #[test]
    fn the_notice_warns_that_work_outside_the_workspace_may_already_have_happened() {
        let notice = resumption_notice();

        assert!(notice.contains("outside this workspace"), "{notice}");
        assert!(
            notice.contains("Check how things actually stand"),
            "{notice}"
        );
    }
}
