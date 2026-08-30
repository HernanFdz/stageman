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

use stageman_agent::{AgentError, Answer, ContainerRuntime};
use stageman_core::{Handout, JobId, ProjectId};

/// What every foreman's container is named for.
///
/// Parallel to the job crate's, and named from the project rather than
/// recorded anywhere for the same reason: the name is known before the
/// container exists, so there is no instant at which one is running and
/// nothing can say whose it is — see
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`.
const PREFIX: &str = "stageman-foreman-";

/// The container a project's foreman thinks in.
///
/// One per project and long-lived, where a job's is one per job and
/// ephemeral. That difference is the whole distinction between the two —
/// `docs/decisions/0012-agents-run-in-containers.md` — and it is why this
/// name is derived from the project: a foreman has no identifier of its own
/// because there is exactly one per project, so the project *is* its identity.
#[must_use]
pub fn container(project: ProjectId) -> String {
    format!("{PREFIX}{}", project.as_uuid())
}

/// Which project a container belongs to, if its name says so.
///
/// The reverse of [`container`], and it exists for the sweep rather than for
/// this crate: a container carrying this project's label has to be placed as
/// somebody's, and a foreman's would otherwise be counted as a name this
/// version cannot read — which is reported as odd and benign, and would be
/// neither.
#[must_use]
pub fn project_of(container: &str) -> Option<ProjectId> {
    container
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.parse().ok())
        .map(ProjectId::from_uuid)
}

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

/// Starts a foreman's session, or continues the one it already has.
///
/// **Which of the two is asked of the runtime, never of the instance.** A
/// container is the truth about whether a session exists —
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md` makes that the
/// rule for jobs and it holds no less here, where the container outlives every
/// turn and the snapshot it would otherwise be remembered in. A foreman that
/// believed it had a session and did not would fail every turn until somebody
/// looked.
///
/// The opening is sent only when a session is being made. After that every
/// turn is a message on the same session, which is what lets a foreman
/// remember what it was already asked.
///
/// # Errors
///
/// Fails if the runtime cannot be reached, or the agent cannot be run.
pub async fn attend(
    runtime: &ContainerRuntime,
    handout: &Handout,
    project: ProjectId,
    repository: &str,
    said: &str,
) -> Result<Answer, ForemanError> {
    let name = container(project);
    let existing = stageman_agent::abandoned(runtime)
        .await
        .map_err(ForemanError::Agent)?;

    if continuing(&existing, &name) {
        // The thread comes from the handout every turn, which is the whole
        // point: one container, a different thread each time.
        stageman_agent::resume(runtime, &name, handout.thread(), &asked(said))
            .await
            .map_err(ForemanError::Agent)
    } else {
        // The opening and the first message together, because a session that
        // was told who it is and then asked nothing would have spent a turn
        // saying hello.
        let first = format!("{}\n\n{}", opening(repository), asked(said));
        stageman_agent::begin(runtime, handout, &name, &first)
            .await
            .map_err(ForemanError::Agent)
    }
}

/// Whether this foreman already has a session to carry on.
///
/// A comparison, and therefore worth extracting: mutation testing inverted it
/// without a test noticing, and inverting it is the worst available outcome —
/// every foreman that had a session would be told the opening again as though
/// it were new, and every foreman that had none would be asked to resume one
/// that does not exist.
#[must_use]
fn continuing(existing: &[String], name: &str) -> bool {
    existing.iter().any(|found| found == name)
}

/// A foreman's turn could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum ForemanError {
    /// The agent could not be run, or would not answer.
    #[error("the foreman's agent could not be run")]
    Agent(#[source] AgentError),
}

/// The first thing a project's foreman is ever told.
///
/// Said once, at the start of a session that then lasts as long as the project
/// does: every message after this is a turn on the same session, so the
/// foreman remembers what it has already been asked and what it already did.
///
/// **The autonomy paragraph is the one that differs from a job's**, and it is
/// not a stylistic difference. A job may ask a person something and stop,
/// because the answer comes back into that job's own thread. A foreman cannot:
/// by the time somebody answers, it may be several messages further on, and
/// the answer arrives as a new turn in a different thread. So it is told to
/// decide, and told what that costs.
#[must_use]
pub fn opening(repository: &str) -> String {
    format!(
        "\
You are the foreman for {repository}.

People talk to you on a channel. Each message they send you arrives as its own \
turn, and the only way to answer is to **run the command-line program \
`stageman-say`**, which is installed in this container. It is not a tool you \
can call — run it in a shell, the way you would run `git`:

    stageman-say 'what you want to say'

Nothing you write as ordinary output is seen by anybody. What you pass to \
`stageman-say` lands in the thread of the message you are answering, so a \
person can always see which of their messages you meant.

**Decide rather than ask.** You may say anything you like, but nothing you say \
comes back to you in this turn, and a person answering you starts a *new* turn \
that may be behind several others. So never end a turn waiting for a reply: if \
you need a judgement nobody has given you, make the most reasonable one \
available and say plainly what you chose and why. Somebody reading the channel \
can correct you, and that correction is its own message.

You remember everything from earlier turns, so do not ask again for what you \
have already been told."
    )
}

/// What a foreman is told when a person sends it a message.
///
/// Framed rather than passed through, for the reason a job's reply is: what
/// arrives is somebody's words, and a session that has been running for days
/// has no other way to tell those from an instruction it wrote itself.
#[must_use]
pub fn asked(said: &str) -> String {
    format!(
        "\
A person said this to you on the channel:

{said}

Do what it asks, or answer it, or both. Then run `stageman-say` with your \
answer before you finish: a turn that ends without running it has told nobody \
anything, however much you wrote."
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

/// What a thread is told when a foreman's turn could not be taken at all.
///
/// **Not the same as a foreman deciding it cannot help.** That is something it
/// says for itself, in its own words, and this is what is said when it never
/// got to speak — its agent would not run, or the turn ended without
/// finishing. A person who asked for something and hears nothing has no way to
/// tell that from being ignored.
///
/// The message is not retried. A message that cannot be handled must not
/// become one that is handled for ever, blocking every message behind it.
#[must_use]
pub const fn stuck_notice() -> &'static str {
    "Something went wrong handling that, so it did not get done. The server log says what. \
Send it again once that is fixed — it has not been kept."
}

/// What a thread is told when somebody addresses this instance in one that
/// belongs to no job."""
///
/// **Said rather than ignored**, because they asked. Silence here is
/// indistinguishable from being broken, and this is the case a person reaches
/// by the most natural move available: replying to something a foreman said.
///
/// It names where to go instead, because a rule nobody was told is a rule
/// nobody can follow.
#[must_use]
pub const fn no_such_job_notice() -> &'static str {
    "No job belongs to this thread — it may have been retired. If you meant the foreman, \
say so at the root of the channel instead: it does not read replies here."
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
    use super::{JobId, ProjectId, Voice, resumption_notice};

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
            super::stuck_notice(),
            "Something went wrong handling that, so it did not get done. The server log says \
what. Send it again once that is fixed — it has not been kept."
        );
        assert_eq!(
            super::no_such_job_notice(),
            "No job belongs to this thread — it may have been retired. If you meant the foreman, \
say so at the root of the channel instead: it does not read replies here."
        );
        assert_eq!(
            super::busy_notice(),
            "This job is still working, so that did not reach it. Wait until it stops, then say \
it again."
        );
    }

    /// The stuck notice must say the message is gone, not that it is queued.
    ///
    /// It is deliberately not retried — a message that cannot be handled must
    /// not become one handled for ever, blocking everything behind it — so a
    /// person has to be told to send it again. A notice that merely apologised
    /// would leave them waiting for a turn that is never coming.
    #[test]
    fn the_stuck_notice_says_the_message_was_not_kept() {
        let said = super::stuck_notice();

        assert!(said.contains("Send it again"), "{said}");
        assert!(said.contains("not been kept"), "{said}");
    }

    /// The notice for an unowned thread must say where to go instead.
    ///
    /// Its whole reason for existing is that silence reads as broken. A notice
    /// that said only "nothing here" would leave a person exactly as stuck,
    /// having been answered — which is worse, because now they know they were
    /// heard and still cannot get anywhere.
    #[test]
    fn the_unowned_thread_notice_names_somewhere_to_go() {
        let said = super::no_such_job_notice();

        assert!(said.contains("root of the channel"), "{said}");
        assert!(said.contains("foreman"), "{said}");
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

    /// Whether a session is carried on is decided from what the runtime has.
    ///
    /// Inverting this is the worst outcome available: every foreman with a
    /// session would be greeted as new, and every foreman without one asked to
    /// resume nothing. Mutation testing found it untested.
    #[test]
    fn a_session_is_continued_only_when_its_container_is_there() {
        let project = ProjectId::from_uuid(stageman_core::Uuid::from_u128(7));
        let mine = super::container(project);
        let another = super::container(ProjectId::from_uuid(stageman_core::Uuid::from_u128(8)));

        assert!(super::continuing(std::slice::from_ref(&mine), &mine));
        assert!(super::continuing(&[another.clone(), mine.clone()], &mine));

        assert!(
            !super::continuing(&[], &mine),
            "nothing running is a new session"
        );
        assert!(
            !super::continuing(&[another], &mine),
            "another project's foreman is not this one's session"
        );
        // A job's container is not a session to carry on either, however many
        // of them are about.
        assert!(!super::continuing(
            &["stageman-job-00000000-0000-0000-0000-000000000007".to_owned()],
            &mine
        ));
    }

    /// A foreman's container is named for its project, both ways.
    ///
    /// Total and reversible, like a job's: every project has exactly one such
    /// name and every such name says whose it is, which is what lets a sweep
    /// place one from what the runtime reports rather than from what the
    /// instance remembers.
    #[test]
    fn a_foremans_container_is_named_for_its_project_and_says_so() {
        let project = ProjectId::from_uuid(stageman_core::Uuid::from_u128(7));
        let named = super::container(project);

        assert_eq!(
            named,
            "stageman-foreman-00000000-0000-0000-0000-000000000007"
        );
        assert_eq!(super::project_of(&named), Some(project));

        // A job's container is not a foreman's, which is the distinction the
        // sweep needs in order to place either.
        assert_eq!(
            super::project_of("stageman-job-00000000-0000-0000-0000-000000000007"),
            None
        );
        assert_eq!(super::project_of("something-else-entirely"), None);
        assert_eq!(super::project_of("stageman-foreman-not-a-uuid"), None);
    }

    /// Asserted whole, per `docs/conventions.md` §4.
    #[test]
    fn a_foremans_opening_reads_exactly_as_written() {
        assert_eq!(
            super::opening("https://example.invalid/repo"),
            "You are the foreman for https://example.invalid/repo.

People talk to you on a channel. Each message they send you arrives as its own turn, and the \
only way to answer is to **run the command-line program `stageman-say`**, which is installed in \
this container. It is not a tool you can call — run it in a shell, the way you would run `git`:

    stageman-say 'what you want to say'

Nothing you write as ordinary output is seen by anybody. What you pass to `stageman-say` lands \
in the thread of the message you are answering, so a person can always see which of their \
messages you meant.

**Decide rather than ask.** You may say anything you like, but nothing you say comes back to you \
in this turn, and a person answering you starts a *new* turn that may be behind several others. \
So never end a turn waiting for a reply: if you need a judgement nobody has given you, make the \
most reasonable one available and say plainly what you chose and why. Somebody reading the \
channel can correct you, and that correction is its own message.

You remember everything from earlier turns, so do not ask again for what you have already been \
told."
        );
    }

    /// Asserted whole, and framed as somebody speaking.
    #[test]
    fn a_message_to_a_foreman_reads_exactly_as_written() {
        assert_eq!(
            super::asked("look at the parser"),
            "A person said this to you on the channel:

look at the parser

Do what it asks, or answer it, or both. Then run `stageman-say` with your answer before you \
finish: a turn that ends without running it has told nobody anything, however much you wrote."
        );
    }

    /// The instruction has to say `stageman-say` is a program, not a tool.
    ///
    /// **Found by running it.** The first version said "you answer with
    /// stageman-say", which reads perfectly and behaved wrong: the agent
    /// searched for a tool by that name, found none, and answered in ordinary
    /// output that nobody sees. The job kickoff never had this problem because
    /// it lists the program beside `git` and `gh`, which an agent already
    /// knows are commands.
    #[test]
    fn a_foreman_is_told_that_saying_something_means_running_a_command() {
        let told = super::opening("https://example.invalid/repo");

        assert!(told.contains("command-line program"), "{told}");
        assert!(told.contains("run it in a shell"), "{told}");
        assert!(
            told.contains("not a tool you can call"),
            "the mistake it made was looking for a tool: {told}"
        );
        // And that ordinary output reaches nobody, which is why the agent
        // believed it had answered when it had not.
        assert!(
            told.contains("Nothing you write as ordinary output"),
            "{told}"
        );
    }

    /// A foreman is never told it may wait, which a job is allowed to do.
    ///
    /// The one instruction that must differ between the two. A job asks and
    /// stops because the answer returns to its own thread; a foreman cannot,
    /// because by the time somebody answers it may be several turns further
    /// on. A prompt that let it wait would produce a foreman that stops
    /// working and nobody could tell why.
    #[test]
    fn a_foreman_is_told_to_decide_rather_than_wait() {
        let told = super::opening("https://example.invalid/repo");

        assert!(told.contains("Decide rather than ask"), "{told}");
        assert!(told.contains("never end a turn waiting"), "{told}");
        assert!(
            told.contains("say plainly what you chose"),
            "deciding without saying so is invisible: {told}"
        );
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
