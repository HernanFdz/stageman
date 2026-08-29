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

/// Whether a job has anywhere to speak.
///
/// Decides one paragraph of the kickoff, and it has to be decided rather than
/// assumed either way. A prompt naming `stageman-say` to a job whose project
/// has no channel bound teaches an agent to run a command that cannot work; a
/// prompt withholding it from one that has leaves the tool installed and
/// unmentioned, which is the same as not shipping it.
///
/// A two-variant type rather than a `bool`, because the call site is where this
/// is read and `kickoff(repository, work, true)` says nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// A channel is bound, so `stageman-say` reaches somebody.
    Channel,
    /// Nothing is bound. The job can still do work that never needs to speak —
    /// see `docs/decisions/0005-conversation-happens-on-channels.md`.
    Silent,
}

/// The instruction a job begins from.
///
/// Self-contained by necessity: an agent in a fresh container knows nothing
/// about where it came from, so this carries the repository, the work, and the
/// constraint — `docs/conventions.md` §2.
///
/// Three things it says that are not negotiable, each from a decision rather
/// than from taste. Nothing has been checked out, because
/// `docs/decisions/0016-the-agent-clones-the-repository.md` removed every
/// mechanism that would have. The tools are already authenticated, because
/// `docs/decisions/0009-jobs-hold-their-own-platform-credentials.md` hands the
/// job its project's credential rather than proxying its access. And work ends
/// at a proposal, because
/// `docs/decisions/0002-never-merge-never-deploy.md` is what lets a job run
/// with nobody watching at all.
///
/// The proposal instruction is deliberately conditional — *when you have a
/// change to propose*. A job whose work is a question rather than a change has
/// nothing to open, and an unconditional instruction would have it inventing
/// one to comply.
///
/// The last paragraph is conditional for a different reason, and one worth
/// knowing before changing it. `docs/open-questions.md` records that this
/// instruction cannot honestly become *ask and wait* until a reply can reach a
/// running job, and it still does not say that: what changed is where a
/// question goes, not whether the job stops. Both forms end the same way,
/// because with outbound alone nobody answers this session.
#[must_use]
pub fn kickoff(repository: &str, work: &str, voice: Voice) -> String {
    let tools = match voice {
        Voice::Channel => {
            "You have git, gh and stageman-say, and gh is already \
signed in as the account this work belongs to."
        }
        Voice::Silent => {
            "You have git and gh, and gh is already signed in as the \
account this work belongs to."
        }
    };

    let speaking = match voice {
        Voice::Channel => {
            "\
If you need an answer from a person before you can continue, run stageman-say \
with your question as its one argument, then stop. It reaches somebody who can \
answer — but not now, and no reply will arrive in this session, so do not wait \
for one and do not guess. Use it the same way for anything else worth a person \
knowing, such as finishing, or being stuck."
        }
        Voice::Silent => {
            "\
If you need an answer from a person before you can continue, say so plainly \
and stop. Do not guess, and do not wait — nobody is watching this terminal."
        }
    };

    format!(
        "\
You are working on {repository}.

Nothing has been checked out for you. If the work needs the repository, clone \
it into the current directory yourself. {tools}

The work:

{work}

When you have a change to propose, open a pull request and stop there. Do not \
merge it, do not deploy anything, and do not push to the default branch. \
Somebody reads what you propose before it counts for anything, which is what \
lets you work unattended.

{speaking}"
    )
}

#[cfg(test)]
mod tests {
    use super::{Voice, resumption_notice};

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

    /// Asserted whole, per `docs/conventions.md` §4. This is the text that
    /// decides what every job does, and it can be rewritten completely without
    /// a single other test going red — so the diff is the review.
    #[test]
    fn a_kickoff_reads_exactly_as_written() {
        assert_eq!(
            super::kickoff(
                "https://example.invalid/repo",
                "Fix the flaky test in the parser.",
                Voice::Silent
            ),
            "You are working on https://example.invalid/repo.

Nothing has been checked out for you. If the work needs the repository, clone it into the \
current directory yourself. You have git and gh, and gh is already signed in as the account this \
work belongs to.

The work:

Fix the flaky test in the parser.

When you have a change to propose, open a pull request and stop there. Do not merge it, do not \
deploy anything, and do not push to the default branch. Somebody reads what you propose before \
it counts for anything, which is what lets you work unattended.

If you need an answer from a person before you can continue, say so plainly and stop. Do not \
guess, and do not wait — nobody is watching this terminal."
        );
    }

    /// The other half of the same text, and asserted whole for the same reason.
    ///
    /// Two variants means two snapshots. A single one would leave the paragraph
    /// that actually changed — the one naming the tool — as the only prompt
    /// text in the project nothing asserts.
    #[test]
    fn a_kickoff_with_a_channel_bound_reads_exactly_as_written() {
        assert_eq!(
            super::kickoff(
                "https://example.invalid/repo",
                "Fix the flaky test in the parser.",
                Voice::Channel
            ),
            "You are working on https://example.invalid/repo.

Nothing has been checked out for you. If the work needs the repository, clone it into the \
current directory yourself. You have git, gh and stageman-say, and gh is already signed in as \
the account this work belongs to.

The work:

Fix the flaky test in the parser.

When you have a change to propose, open a pull request and stop there. Do not merge it, do not \
deploy anything, and do not push to the default branch. Somebody reads what you propose before \
it counts for anything, which is what lets you work unattended.

If you need an answer from a person before you can continue, run stageman-say with your \
question as its one argument, then stop. It reaches somebody who can answer — but not now, and \
no reply will arrive in this session, so do not wait for one and do not guess. Use it the same \
way for anything else worth a person knowing, such as finishing, or being stuck."
        );
    }

    /// A job with nothing bound is never told to run a command that cannot
    /// work.
    ///
    /// The failure this prevents is an agent doing as it is told and reporting
    /// a tool that fails, which reads as a broken instance rather than as an
    /// unbound project.
    #[test]
    fn a_kickoff_names_the_tool_only_when_there_is_a_channel() {
        let bound = super::kickoff("https://example.invalid/repo", "anything", Voice::Channel);
        let silent = super::kickoff("https://example.invalid/repo", "anything", Voice::Silent);

        assert!(bound.contains("stageman-say"), "{bound}");
        assert!(!silent.contains("stageman-say"), "{silent}");
    }

    /// Neither form tells a job to wait, and that is the constraint
    /// `docs/open-questions.md` puts on this text until a reply can reach a
    /// running job. Outbound alone moves where a question goes, not whether
    /// the job stops.
    #[test]
    fn no_kickoff_tells_a_job_to_wait_for_an_answer() {
        for voice in [Voice::Channel, Voice::Silent] {
            let prompt = super::kickoff("https://example.invalid/repo", "anything", voice);

            assert!(prompt.contains("do not wait"), "{prompt}");
            assert!(prompt.contains("stop"), "{prompt}");
        }
    }

    /// The constraint that lets a job run with nobody watching, and the one
    /// worth a test of its own because losing it is not visible in behaviour
    /// until something has already been merged.
    #[test]
    fn a_kickoff_always_says_to_propose_and_never_to_merge() {
        let prompt = super::kickoff(
            "https://example.invalid/repo",
            "anything at all",
            Voice::Channel,
        );

        assert!(prompt.contains("open a pull request"), "{prompt}");
        assert!(prompt.contains("Do not merge it"), "{prompt}");
        assert!(
            prompt.contains("do not push to the default branch"),
            "{prompt}"
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
