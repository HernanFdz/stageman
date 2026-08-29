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

use stageman_core::JobId;

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

/// What a project's channel is told when a job starts.
///
/// The message a job's thread hangs from, so it is written for whoever is
/// reading the channel rather than for the agent. Composed here because this
/// crate authors the text this system emits — `docs/architecture.md` §1 — and
/// held to the same snapshot test as the rest, since it is read by a person
/// and nothing else would notice it changing.
///
/// **It does not promise a reply reaches anybody.** Saying so would be the
/// natural sentence to write and is not true yet: a job speaks and stops, and
/// nothing carries an answer back until inbound is built. A channel message
/// making a promise the system does not keep is worse than a terse one.
#[must_use]
pub fn announcement(repository: &str, reason: &str, job: JobId) -> String {
    format!(
        "\
Starting a job on {repository}.

{reason}

Whatever it has to say appears in this thread. Job {job}."
    )
}

/// What a job's thread is told when its agent stops.
///
/// **Deliberately says nothing about how it went.** The instance knows the
/// turn ended and cannot know whether that was an answer, a question, or an
/// agent giving up — deciding would mean reading what it said, and reading it
/// properly would mean another model call. A notice that guessed would be
/// wrong often enough to be worse than one that does not try.
///
/// What it does carry is the fact worth having: the agent has stopped, so a
/// reply now reaches it. While
/// `docs/open-questions.md` has messaging a *running* job unsupported, that is
/// the difference between a thread somebody can act on and one they cannot.
#[must_use]
pub const fn attention_notice() -> &'static str {
    "⚠️ Check this out. The agent has stopped; a reply in this thread will reach it."
}

/// What a thread is told when a reply arrives for a job that is still working.
///
/// The honest version of a limitation rather than a silence. A person who
/// replies and hears nothing concludes the reply was read, which is the one
/// wrong belief available here — they would then wait for an answer that is
/// not coming.
#[must_use]
pub const fn busy_notice() -> &'static str {
    "This job is still working, so that did not reach it. Wait until it stops, then say it again."
}

/// What a job's agent is told when a person replies on its thread.
///
/// Framed rather than passed through, and the frame is the whole of it: what
/// arrives is a person's words, and an agent picking up a session hours later
/// has no way to tell those from a new instruction it should follow to the
/// letter. Saying who is speaking is what makes the difference legible.
///
/// Composed here because every text this system emits is, per
/// `docs/architecture.md` §1 — including the ones that merely wrap somebody
/// else's.
#[must_use]
pub fn reply(said: &str) -> String {
    format!(
        "\
A person replied on the channel:

{said}

Carry on from there. The same rules still hold: propose rather than merge, and \
say what you did when you finish."
    )
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
Finish by saying what you did, using stageman-say, which takes what you want \
to say as its one argument. Nobody reads this terminal, so anything you do not \
say there is lost — including the answer, if the work was a question. Say what \
you found, what you changed, or what you could not do.

Use it during the work as well, whenever you need an answer from a person: say \
what you need, then stop. It reaches somebody who can answer, but not now — no \
reply arrives in this session, so do not wait for one and do not guess."
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
    use super::{JobId, Voice, resumption_notice};

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

Finish by saying what you did, using stageman-say, which takes what you want to say as its one \
argument. Nobody reads this terminal, so anything you do not say there is lost — including the \
answer, if the work was a question. Say what you found, what you changed, or what you could not \
do.

Use it during the work as well, whenever you need an answer from a person: say what you need, \
then stop. It reaches somebody who can answer, but not now — no reply arrives in this session, \
so do not wait for one and do not guess."
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

    /// Asserted whole, per `docs/conventions.md` §4. Read by a person rather
    /// than an agent, which is exactly why nothing else would notice it
    /// changing.
    #[test]
    fn an_announcement_reads_exactly_as_written() {
        assert_eq!(
            super::announcement(
                "https://example.invalid/repo",
                "an issue was opened",
                JobId::from_uuid(stageman_core::Uuid::from_u128(9))
            ),
            "Starting a job on https://example.invalid/repo.

an issue was opened

Whatever it has to say appears in this thread. Job \
00000000-0000-0000-0000-000000000009."
        );
    }

    /// It must not promise something the system does not do yet.
    ///
    /// The natural sentence to write here is that replying reaches the agent,
    /// and nothing carries a reply back until inbound is built —
    /// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`. A channel
    /// message making a promise the system does not keep is worse than a terse
    /// one, and this is what stops somebody adding it back.
    #[test]
    fn an_announcement_does_not_promise_a_reply() {
        let said = super::announcement(
            "https://example.invalid/repo",
            "an issue was opened",
            JobId::from_uuid(stageman_core::Uuid::from_u128(9)),
        );

        assert!(!said.contains("repl"), "{said}");
        assert!(!said.contains("answer"), "{said}");
    }

    /// Reporting is the ending, not the exception — and this is why.
    ///
    /// The first version of this paragraph opened with *if you need an answer
    /// from a person*, and put finishing in a subordinate clause at the end of
    /// it. An agent given read-only work then has no question, no change to
    /// propose, and no reason to speak: it answers into a session nothing
    /// keeps, and the channel stays empty. That is not hypothetical — it is
    /// what the first real job did.
    ///
    /// Asserted alongside the snapshot rather than left to it, because a
    /// snapshot is updated wholesale by whoever changes the text and records
    /// no opinion about which sentence mattered.
    #[test]
    fn a_kickoff_with_a_channel_makes_reporting_the_ending() {
        let prompt = super::kickoff("https://example.invalid/repo", "anything", Voice::Channel);

        let reporting = prompt
            .find("Finish by saying")
            .expect("it must say to report at the end");
        let asking = prompt
            .find("whenever you need an answer")
            .expect("and still offer the tool during the work");

        assert!(
            reporting < asking,
            "reporting must come first, or it reads as a special case of asking: {prompt}"
        );
    }

    /// Asserted as literal text, per `docs/conventions.md` §4.
    ///
    /// Both of these are read by a person on a channel and by nothing else, so
    /// a rewrite would change what an operator sees and no other test would go
    /// red.
    #[test]
    fn the_notices_read_exactly_as_written() {
        assert_eq!(
            super::attention_notice(),
            "⚠️ Check this out. The agent has stopped; a reply in this thread will reach it."
        );
        assert_eq!(
            super::busy_notice(),
            "This job is still working, so that did not reach it. Wait until it stops, then say \
it again."
        );
    }

    /// The attention notice must not claim to know how it went.
    ///
    /// The instance sees a turn end and cannot tell an answer from a question
    /// from an agent giving up. Every word suggesting otherwise is one an
    /// operator would reasonably act on, so this is what stops a well-meant
    /// "finished" being added later.
    #[test]
    fn the_attention_notice_says_nothing_about_how_it_went() {
        let said = super::attention_notice().to_lowercase();

        for claim in ["finish", "done", "complete", "success", "fail", "error"] {
            assert!(!said.contains(claim), "it must not claim {claim}: {said}");
        }
    }

    /// Asserted whole, per `docs/conventions.md` §4.
    #[test]
    fn a_reply_reads_exactly_as_written() {
        assert_eq!(
            super::reply("use postgres"),
            "A person replied on the channel:

use postgres

Carry on from there. The same rules still hold: propose rather than merge, and say what you did \
when you finish."
        );
    }

    /// A reply is framed as somebody speaking, not as a fresh instruction.
    ///
    /// An agent picking up a session hours later cannot otherwise tell a
    /// person's words from something it must follow literally, and the words
    /// are whatever the person typed.
    #[test]
    fn a_reply_says_who_is_speaking_before_it_says_what() {
        let framed = super::reply("delete everything");

        assert!(framed.starts_with("A person replied"), "{framed}");
        assert!(framed.contains("propose rather than merge"), "{framed}");
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
