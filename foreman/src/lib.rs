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

use stageman_agent::{ToolCallStatus, ToolKind};
use stageman_core::{ProjectId, VariableName, Waiting};

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
posted, and including anything you left running, such as whatever you were \
showing. Check how things actually stand before you act. Do not assume your \
last step completed, and do not assume it did not.

Then carry on with the work you were given.";

/// What a resumed job's agent is told about having been interrupted.
#[must_use]
pub const fn resumption_notice() -> &'static str {
    RESUMPTION
}

/// The first message in a job's room, for whoever finds the room.
///
/// Written for a person reading the channel rather than for the agent, and
/// composed here because this crate authors the text this system emits —
/// `docs/architecture.md` §1 — and held to the same snapshot test as the
/// rest, since nothing else would notice it changing.
///
/// It teaches the one rule a person has to know in the room, because the
/// room is the first place a newcomer meets it: the job reads nothing that
/// does not mention it, so that people can talk to each other here without
/// waking it. The mention is rendered by the channel, since how one is
/// spelled is the platform's business.
#[must_use]
pub fn room_opening(repository: &str, reason: &str, mention: &str) -> String {
    format!(
        "\
**A job on {repository}**

{reason}

_Everything it has to say appears here. Mention {mention} to talk to it; \
anything else said here is between people._"
    )
}

/// What the message a job came from is told, in its thread: where the job
/// is.
///
/// The link is rendered by the channel, because a reference to a room is
/// the platform's to spell.
#[must_use]
pub fn started_notice(link: &str) -> String {
    format!("Started a job for this: {link}.")
}

/// Why a turn started, as the notice at the root of a room says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Because<'a> {
    /// A person's message, linked when the channel can link it.
    Message(Option<&'a str>),
    /// Another app's post in a watched room, by the app's name.
    Signal {
        /// The app, as the platform names it.
        app: &'a str,
        /// A link to what it posted, when the channel can link it.
        link: Option<&'a str>,
    },
    /// This process restarted with a turn in hand, linked to what the turn
    /// was on when the channel can link it.
    Restart(Option<&'a str>),
}

/// What the root of a room is told when a turn starts there.
///
/// Why, with a link to what started it. Posted by the instance before
/// anything the agent says, so that what follows is about something — see
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
#[must_use]
pub fn turn_notice(because: &Because<'_>) -> String {
    match because {
        Because::Message(Some(link)) => format!("▶️ Handling {link}."),
        Because::Message(None) => "▶️ Handling a message.".to_owned(),
        Because::Signal {
            app,
            link: Some(link),
        } => format!("▶️ Judging what {app} posted: {link}."),
        Because::Signal { app, link: None } => format!("▶️ Judging what {app} posted."),
        Because::Restart(Some(link)) => format!("▶️ Picking up {link} again after a restart."),
        Because::Restart(None) => "▶️ Picking up again after a restart.".to_owned(),
    }
}

/// What a thread is told when the job it asked in answered elsewhere: at the
/// root of its room, where its transcript goes. The signpost of 0067, for
/// the turn that missed the tool call.
#[must_use]
pub fn answered_elsewhere_notice(room: &str) -> String {
    format!("↩️ Answered at the root of {room}.")
}

/// What a thread is told when the foreman handled the message without
/// answering there: where its notes went, when it has a room to link.
#[must_use]
pub fn handled_elsewhere_notice(room: Option<&str>) -> String {
    room.map_or_else(
        || "↩️ Handled without answering here.".to_owned(),
        |room| format!("↩️ Handled without answering here; its notes are in {room}."),
    )
}

/// What a foreman's room is for, as the sidebar shows it beside the name.
#[must_use]
pub fn foreman_room_purpose(project: &str) -> String {
    format!("Where {project}'s foreman thinks: what it is handling, what it decided, and why.")
}

/// The first message in a foreman's room, for whoever finds the room.
///
/// It teaches the one rule that matters there: the foreman is talked to by
/// a mention, anywhere, and this room is where its work shows. The mention
/// is rendered by the channel, since how one is spelled is the platform's
/// business.
#[must_use]
pub fn foreman_room_opening(project: &str, mention: &str) -> String {
    format!(
        "\
**{project}'s foreman.**

_Everything it says and does as it works appears here. Mention {mention} anywhere to \
talk to it; anything else said here is between people._"
    )
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
turn, and the only way to answer is to **call the `say` tool**, in Markdown, \
which is rendered.

Everything you write as ordinary output is posted in a room of your own, \
where anybody can watch you work — but the person who asked is not there. \
What you pass to `say` lands under the message you name with `to` — each \
message is shown to you with its identifier — so a person can always see \
which of their messages you meant; without one, it lands at the root of your \
own room.

**You do not do the work yourself.** You have no copy of the repository and no \
credentials to reach it, and that is deliberate rather than something missing: \
reaching a repository is a job's business, not yours. When something needs \
doing, **call the `start_job` tool**.

A job is one agent in a container of its own, holding this project's \
credentials, which can clone the repository, change it and open a pull \
request. It reports in a room of its own, named after the `title` you give \
it, and whoever asked for it is invited there. Its `reason` is prose a person \
reads on the dashboard; its `instructions` are the whole instruction that \
job's agent is given — it cannot see this conversation, so say everything it \
needs.

You can also be asked to watch a room. When a person asks you, in a room, to \
watch it, **call the `watch_room` tool** there: from then on everything another \
app posts in that room — an issue filed, an alert fired, a pull request opened \
— reaches you as a signal to judge, framed as that app's. `stop_watching`, \
asked in the same room, undoes it. People are only ever heard through a \
mention, whether a room is watched or not.

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

/// How a turn on a message is starting.
///
/// **Not a detail of delivery.** A foreman's inbox is part of the snapshot, so
/// a message in hand when this process stopped is still in hand when it comes
/// back — and the turn that picks it up has to be told, because everything it
/// may already have done happened outside this process and cannot be asked
/// about afterwards. The same reasoning as
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, applied to the
/// half that record did not cover; see
/// `docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`.
///
/// Deliberately not called an attempt. `docs/conventions.md` §2 keeps that
/// word out of this project because it implies a count and a retry, and this
/// is neither: it is one turn, continuing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Starting {
    /// Nothing has been said to the agent about this message yet.
    Fresh,
    /// A turn on this message was cut short by this process stopping.
    Interrupted,
}

/// One turn: the message being handled, and how the handling is starting.
///
/// The two things that differ from one turn to the next — everything else a
/// foreman is handed describes its project or its session and is the same
/// every time. Named for `docs/conventions.md` §2's word, which says a turn is
/// one message handled until its agent stops, because that is exactly what
/// this is the description of.
#[derive(Debug, Clone, Copy)]
pub struct Turn<'a> {
    /// What was said, as the person wrote it or as the app's message reads.
    pub said: &'a str,
    /// The message to answer under, as the channel identifies it to an
    /// agent: the thread the message was in, or the message itself.
    pub target: &'a str,
    /// What the turn is shown of the thread the message was said in, when
    /// it was in one: composed by [`thread_shown`], or [`thread_unread`].
    pub thread: Option<&'a str>,
    /// Whether a turn on it was already begun and cut short.
    pub starting: Starting,
    /// The app that posted it, by name, when it is a signal from a watched
    /// room rather than a person's message — see
    /// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
    /// What the framing says, because a foreman has no other way to tell a
    /// notification from a request.
    pub app: Option<&'a str>,
}

/// The project a foreman is thinking about, as it needs to know it.
///
/// Three values that have always travelled together and were three arguments
/// until the instance became a fourth: which project this is, the repository
/// its jobs work on, and the kits they may run on. Bundled rather than passed
/// abreast because a caller that got two of them from one project and the
/// third from another would compile — and would have a foreman start a job on
/// somebody else's repository.
#[derive(Debug, Clone, Copy)]
pub struct Watching<'a> {
    /// Which project this foreman belongs to.
    pub project: ProjectId,
    /// Where its jobs work.
    pub repository: &'a str,
    /// The kits its jobs may run on, each with what the operator wants it for.
    pub kits: &'a [(&'a str, &'a str)],
}

/// What a foreman is told when a turn is picking up an interrupted one.
///
/// Its own paragraph rather than a clause in the message, and prepended rather
/// than woven in, so that the message underneath is byte-for-byte the one the
/// person wrote. The middle sentence is the load-bearing one, for the reason
/// [`RESUMPTION`] gives about a job: a foreman may already have started a job
/// or spoken on the channel, and one that assumes otherwise does it twice.
const INTERRUPTION: &str = "\
You were interrupted: the process running you stopped part-way through \
handling the message below, and you have just been restarted.

You may have finished it, half-finished it, or never begun — including \
things that outlive you, such as a job you started or something you said \
on the channel. Check how things actually stand before you act. Do not \
assume your last step completed, and do not assume it did not.";

/// What a thread is told when the message it is waiting on was interrupted.
///
/// Said because only the instance can say it. The person has already been told
/// their message was received, and then heard nothing for as long as this
/// instance was down — which is indistinguishable from having been forgotten,
/// and forgotten is the thing it must not be mistaken for.
#[must_use]
pub const fn resumed_notice() -> &'static str {
    "stageman restarted while working on this, and is picking it up again."
}

/// What a foreman is told when a person sends it a message, or when another
/// app posts in a room it watches.
///
/// Framed rather than passed through, for the reason a job's reply is: what
/// arrives is somebody's words, and a session that has been running for days
/// has no other way to tell those from an instruction it wrote itself. A
/// signal is framed as the app's, and differently: nobody asked anything,
/// so the foreman is told to judge rather than to answer, and to speak only
/// if it acted or a person needs to know — the reaction on the message
/// already says it looked. See
/// `docs/decisions/0063-another-app-is-heard-in-a-watched-room.md`.
///
/// **The kits are named here rather than in the opening**, and that is the
/// point of saying them every turn: a project's kits are edited from the
/// dashboard, and a session outlives those edits. Said once at the start, the
/// list would be right until somebody changed it and wrong thereafter, with
/// nothing to notice. Each comes with the description its operator wrote,
/// which is what the choice is made on — see
/// `docs/decisions/0048-a-job-runs-on-a-kit.md`. **The brief is said here
/// for the same reason**, and an empty one says nothing, so a project with
/// none gets the prompt it always got — see
/// `docs/decisions/0064-a-project-has-a-brief.md`.
#[must_use]
pub fn asked(turn: Turn<'_>, kits: &[(&str, &str)], brief: &str) -> String {
    let Turn {
        said,
        target,
        thread,
        starting,
        app,
    } = turn;
    let choices = kits
        .iter()
        .map(|(named, described)| format!("  {named} — {described}"))
        .collect::<Vec<_>>()
        .join("\n");

    // First, and on its own, because it changes what every line after it
    // means. An interrupted turn that read its instructions before hearing it
    // was interrupted has already decided what to do.
    let interruption = match starting {
        Starting::Fresh => String::new(),
        Starting::Interrupted => format!("{INTERRUPTION}\n\n"),
    };
    // Before the message, so that it reads as what the message follows.
    let context = thread.map_or_else(String::new, |shown| format!("{shown}\n\n"));

    let framed = app.map_or_else(
        || {
            format!(
                "\
A person said this to you on the channel:

{said}

Answer it, or start a job for it with the `start_job` tool, or both. Then \
call `say` before you finish, with `to` set to `{target}`, so that your answer \
lands under their message: a turn that ends without calling it has told \
nobody anything, however much you wrote."
            )
        },
        |app| {
            format!(
                "\
{app} posted this in a room you watch:

{said}

Nobody asked you anything: this is a signal, and yours to judge. Decide what it \
deserves — nothing, a job started with the `start_job` tool, or a word to the \
people in that room — and do that. Call `say` only if you acted on it or a \
person needs to know something, with `to` set to `{target}` to speak under \
what {app} posted; the reaction on the message already says you looked, so a \
turn that ends in silence is a decision, not a failure."
            )
        },
    );

    // Inside this value rather than in the template, so that a project with
    // no brief gets a prompt byte-for-byte what it was before briefs existed.
    let briefed = if brief.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nThe operator's brief for this project — standing instructions, in their own \
words, that apply to every message and every signal:\n\n{}",
            brief.trim()
        )
    };

    format!(
        "\
{interruption}{context}{framed}{briefed}

The kits this project's jobs may run on — each an agent, set a particular way — \
and what each is for:

{choices}

Choose one deliberately and name it first. It is your judgement to make — \
`docs/decisions/0048-a-job-runs-on-a-kit.md` — and the list is said here \
rather than at the start of this session because it can change while you are \
still running.

If a command fails, say what it printed rather than what you think it meant. \
An explanation you inferred is one a person will act on, and you have no way \
to check it."
    )
}

/// What a job's room is told when its agent stops: which reading of idle
/// the job landed in, and what to do about it.
///
/// It used to say nothing about how it went, because the instance could
/// not tell an answer from a question from an agent giving up. Since
/// `docs/decisions/0055-a-job-says-why-it-stopped.md` the agent says, and
/// this passes that on — the agent's own claim, never a guess — with the
/// one thing the agent cannot say, which is that a mention now reaches it.
/// A job waiting for an answer names the person who asked for it, rendered
/// by the channel, so that they are notified. Said at the root of the job's
/// room, whatever thread the exchange was in, because it is about the job
/// and the root is the job's timeline. See
/// `docs/decisions/0062-what-this-instance-says-is-markdown.md`.
#[must_use]
pub fn stopped_notice(waiting: &Waiting, asked_by: Option<&str>, mention: &str) -> String {
    match waiting {
        Waiting::Asked => asked_by.map_or_else(
            || format!("❓ **Waiting for an answer.** Mention {mention} here to reply."),
            |who| format!("❓ **Waiting for an answer**, {who}. Mention {mention} here to reply."),
        ),
        Waiting::Proposed => {
            format!("✅ **Ready for review.** Mention {mention} here to send it back for changes.")
        }
        Waiting::Paused => {
            format!("⏸️ **Stopped by an operator.** Mention {mention} here to carry on.")
        }
        Waiting::Failed(why) => format!(
            "❌ **Failed:** `{why}`. Mention {mention} here to try again once that is fixed."
        ),
        Waiting::Silent => {
            format!("⏹️ **Stopped without saying why.** Mention {mention} here to carry on.")
        }
    }
}

/// What a thread is told when a reply arrives for a job that is still working.
///
/// The honest version of a limitation rather than a silence. A person who
/// replies and hears nothing concludes the reply was read, which is the one
/// wrong belief available here — they would then wait for an answer that is
/// not coming.
#[must_use]
pub const fn busy_notice() -> &'static str {
    "⏳ Still working, so that did not reach it. Say it again once it stops."
}

/// What a thread is told when a reply arrives for a job that is over.
///
/// Distinct from the notice for a job that was never here, and the difference
/// is the one thing a person needs: this job existed and this thread was its
/// thread, so a reply landing here is not a mistake about where to say it. It
/// is a conversation that has ended, and there is nothing to reopen — the
/// container went with the retirement, and the session with it.
#[must_use]
pub const fn over_notice() -> &'static str {
    "This job is over, so nothing more reaches it. Start a new job if there is still work here."
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
pub fn reply(said: &str, target: &str, thread: Option<&str>) -> String {
    // Before the reply, so that it reads as what the reply follows.
    let context = thread.map_or_else(String::new, |shown| format!("{shown}\n\n"));
    format!(
        "\
{context}A person replied on the channel:

{said}

To answer them where they asked, call `say` with `to` set to `{target}`; \
whatever you write without it is posted at the root of your room. Carry on \
from there. The same rules still hold: propose rather than merge, and say what \
you did when you finish."
    )
}

/// Who said one message of a thread, as an agent is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice<'a> {
    /// A person, by the channel's own mention of them.
    Person(&'a str),
    /// This instance: the agent's own earlier words, or a notice of its.
    Us,
    /// Another app, by the name the platform gives it.
    App(&'a str),
}

/// One message of a thread, as an agent is shown it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown<'a> {
    /// The identifier the agent would name it by.
    pub id: &'a str,
    /// Who said it.
    pub voice: Voice<'a>,
    /// What was said.
    pub text: &'a str,
}

/// What a turn is told of the thread its message was said in.
///
/// Said before the message itself: what came before, oldest first, each
/// entry with who said it and its identifier — see
/// `docs/decisions/0068-a-mention-is-shown-its-thread.md`. From the last
/// message the agent was given there when its session remembers what came
/// before that, and from the start when it does not; and told when the
/// thread was longer than what is shown.
#[must_use]
pub fn thread_shown(shown: &[Shown<'_>], since: bool, longer: bool) -> String {
    let lead = if since {
        "This was said in a thread. Below are its first message and everything said there from \
         the last message you were given onwards, your own words among them, oldest first, each \
         with who said it and its identifier:"
    } else {
        "This was said in a thread. What was said there before it, oldest first, each with who \
         said it and its identifier:"
    };
    let entries = shown
        .iter()
        .map(|entry| {
            let who = match entry.voice {
                Voice::Person(mention) => mention,
                Voice::Us => "You",
                Voice::App(app) => app,
            };
            format!("{who} ({}):\n{}", entry.id, entry.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let tail = if longer {
        "\n\nThe thread is longer than this: only its most recent messages are shown."
    } else {
        ""
    };
    format!("{lead}\n\n{entries}{tail}")
}

/// What a turn is told when the thread its message was said in could not
/// be read: that it was not, so that nothing is assumed about it.
#[must_use]
pub const fn thread_unread() -> &'static str {
    "This was said in a thread that could not be read, so what came before it is not shown."
}

/// What a thread is told when a foreman's turn could not be taken at all,
/// and why.
///
/// **Not the same as a foreman deciding it cannot help.** That is something it
/// says for itself, in its own words, and this is what is said when it never
/// got to speak — its agent would not run, or the turn ended without
/// finishing. A person who asked for something and hears nothing has no way to
/// tell that from being ignored, and one told only that something went wrong
/// has no way to fix it, which is why the reason is said rather than left
/// to the log.
///
/// The message is not retried. A message that cannot be handled must not
/// become one that is handled for ever, blocking every message behind it.
#[must_use]
pub fn stuck_notice(why: &str) -> String {
    format!(
        "❌ That could not be handled: `{why}`. It will not be retried — send it again once \
that is fixed."
    )
}

/// One line of a burst of working: a tool call as a person reads it, marked
/// with where it has got to.
///
/// Composed here because every text this system posts is, and asserted whole
/// below — see
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
/// The kind is the protocol's classification of tools, said as what the
/// agent did; the title is the adapter's, which for a command is the
/// command. A call not yet ended, or one whose ending the adapter has not
/// classified, reads as still running.
#[must_use]
pub fn working_line(kind: ToolKind, title: &str, status: Option<ToolCallStatus>) -> String {
    let mark = match status {
        Some(ToolCallStatus::Completed) => "✅",
        Some(ToolCallStatus::Failed) => "❌",
        _ => "⏳",
    };
    let did = match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edited",
        ToolKind::Delete => "deleted",
        ToolKind::Move => "moved",
        ToolKind::Search => "searched",
        ToolKind::Execute => "ran",
        ToolKind::Think => "thought about",
        ToolKind::Fetch => "fetched",
        _ => "did",
    };
    format!("{mark} {did} `{title}`")
}

/// A thought in a burst of working, as a quoted line, where an adapter
/// carries any: the agent's reasoning, told apart from what it did.
#[must_use]
pub fn thought_line(text: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(n, line)| {
            if n == 0 {
                format!("> 💭 {line}")
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The instruction a job begins from.
///
/// Self-contained by necessity: an agent in a fresh container knows nothing
/// about where it came from, so this carries the repository, the work, and the
/// constraint — `docs/conventions.md` §2.
///
/// Three things it says that are not negotiable, each from a decision rather
/// than from taste. The repository is already checked out, because
/// `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`
/// puts it there before this is read. The tools are already authenticated, because
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
/// **Every job is told it can speak, and every job is told to say why it
/// stopped.** The first used to be conditional on a channel being bound, and
/// since `docs/decisions/0059-a-project-speaks-and-listens-on-slack-always.md`
/// there is no job without one. The second was never conditional, because
/// why a job stopped is recorded on the job and read from the dashboard — see
/// `docs/decisions/0055-a-job-says-why-it-stopped.md`.
///
/// The paragraph about asking is conditional for a different reason, and one
/// worth knowing before changing it. `docs/open-questions.md` records that
/// this instruction cannot honestly become *ask and wait* until a reply can
/// reach a running job, and it still does not say that: a job that asks
/// stops, because nobody answers this session while it runs.
#[must_use]
pub fn kickoff(repository: &str, work: &str, tunnel: &str, variables: &[VariableName]) -> String {
    let port = stageman_agent::TUNNEL_PORT;
    // Two things this has to get across, and the second is the one that fails
    // silently. A server bound inside the container to loopback is reachable
    // from nowhere else, and an agent checking its own work with curl sees it
    // answering perfectly — so the instruction is repeated rather than
    // mentioned, because being wrong about it costs a whole session.
    let showing = format!(
        "\
You can show people what you are doing. Anything you serve inside this container on port \
{port} is reachable at {tunnel} — a dev server while you work, or a built result for somebody \
to look at before you propose it. Bind it to 0.0.0.0 and not to localhost: a server on \
localhost answers you from inside this container and is reachable from nowhere else. Say where \
to look, because nobody finds that address on their own."
    );

    let tools = "You have git and gh, both signed in as the account this work \
belongs to, and the `say` tool for talking to people.";

    // What a job is told about being read changed with
    // `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`:
    // its narration reaches the room as it is written, so the paragraph that
    // asked it to finish by saying what it did is gone, and the tool is for
    // the thread a person asked in.
    let speaking = "\
Everything you write is posted to the people in this job's room as you write \
it, in Markdown, which is rendered — so write for them: what you found, what \
you changed, or what you could not do, and the answer, if the work was a \
question.

The `say` tool posts at the root of your room, or under a message when you \
name it with `to`, as each message is shown to you. Use it whenever you need \
an answer from a person: say what you need, then stop. It reaches somebody who \
can answer, but not now — no reply arrives in this session, so do not wait for \
one and do not guess.";

    // Named and never valued, and the type is what enforces it: a
    // `VariableName` cannot hold a credential, so there is no call site at
    // which a value could be passed here by accident. It matters more than it
    // looks — a kickoff is stored on the job and crosses the snapshot boundary
    // in the clear, because a job holds no credential.
    //
    // Empty when there are none, and decided rather than assumed either way:
    // naming nothing would teach an agent to go looking in an empty
    // environment, and saying nothing where there *are* variables leaves
    // them set and unmentioned, which is the same as not setting them.
    //
    // The leading blank lines live inside this value rather than in the
    // template below, so that a project with no variables gets a prompt that
    // is byte-for-byte what it was before they existed.
    let supplied = if variables.is_empty() {
        String::new()
    } else {
        let named = variables
            .iter()
            .map(VariableName::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "\n\nSome of what this project needs is already in your environment: {named}. \
Nothing here knows what any of them is for, so follow whatever the repository says about them. \
Treat each as a credential — do not print one, do not write one into a file, and never include \
one in a change you propose."
        )
    };

    format!(
        "\
You are working on {repository}.

It is checked out in the current directory, on its default branch. {tools}

The work:

{work}

{showing}{supplied}

When you have a change to propose, open a pull request and stop there. Do not \
merge it, do not deploy anything, and do not push to the default branch. \
Somebody reads what you propose before it counts for anything, which is what \
lets you work unattended.

{speaking}

Before you stop, call the `stopping` tool, every time and last of all. Say \
`ready_for_review` if you have done what was asked and there is something for a \
person to look at, or `waiting_for_an_answer` if you need something from a \
person before you can go on. Nothing else tells anybody which of the two this \
is, so a job that stops without calling it is recorded as having stopped for \
reasons nobody knows."
    )
}

#[cfg(test)]
mod tests {
    use super::{ProjectId, VariableName, Waiting, resumption_notice};

    /// Where a job of this project would be reachable.
    ///
    /// A literal rather than a value built from a domain, because this crate
    /// composes the text and does not decide the address — the app does, and
    /// asserts its own half separately. Using a real-looking one anyway: a
    /// placeholder that does not read like a URL would let the surrounding
    /// sentence be wrong without a snapshot noticing.
    const A_TUNNEL: &str = "https://00000000-0000-0000-0000-000000000001.example.com";

    /// A project that gives its jobs no variables, which is most of them.
    ///
    /// Named rather than written as an empty slice at each call, because what
    /// these assertions are pinning is that such a project's prompt is
    /// byte-for-byte what it was before variables existed at all.
    const NONE: &[VariableName] = &[];

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
outside this workspace, such as a branch pushed or a comment posted, and including anything you \
left running, such as whatever you were showing. Check how things actually stand before you act. \
Do not assume your last step completed, and do not assume it did not.

Then carry on with the work you were given."
        );
    }

    /// A kickoff naming what a project put in the environment, as literal text.
    ///
    /// `docs/conventions.md` §4: prompt text is the only kind that changes
    /// behaviour without changing control flow, so it is asserted whole rather
    /// than probed for substrings.
    ///
    /// Note the three things the paragraph does, each load-bearing. Naming the
    /// variables is what makes an agent reach for one at all. Saying nothing
    /// here knows what they are for is honest — this project never reads one —
    /// and points at the repository, which is where
    /// `docs/decisions/0019-a-projects-tooling-is-the-projects-business.md`
    /// puts that knowledge. And the last sentence is the only thing standing
    /// between a credential and a pull request description.
    #[test]
    fn a_kickoff_with_variables_reads_exactly_as_written() {
        let variables = [
            VariableName::new("STRIPE_API_KEY").expect("a deliverable name"),
            VariableName::new("DATABASE_URL").expect("a deliverable name"),
        ];

        assert_eq!(
            super::kickoff(
                "https://example.invalid/repo",
                "Fix the flaky test in the parser.",
                A_TUNNEL,
                &variables,
            ),
            "You are working on https://example.invalid/repo.

It is checked out in the current directory, on its default branch. You have git and gh, both \
signed in as the account this work belongs to, and the `say` tool for talking to people.

The work:

Fix the flaky test in the parser.

You can show people what you are doing. Anything you serve inside this container on port 47201 \
is reachable at https://00000000-0000-0000-0000-000000000001.example.com — a dev server while \
you work, or a built result for somebody to look at before you propose it. Bind it to 0.0.0.0 \
and not to localhost: a server on localhost answers you from inside this container and is \
reachable from nowhere else. Say where to look, because nobody finds that address on their \
own.

Some of what this project needs is already in your environment: STRIPE_API_KEY, DATABASE_URL. \
Nothing here knows what any of them is for, so follow whatever the repository says about them. \
Treat each as a credential — do not print one, do not write one into a file, and never include \
one in a change you propose.

When you have a change to propose, open a pull request and stop there. Do not merge it, do not \
deploy anything, and do not push to the default branch. Somebody reads what you propose before \
it counts for anything, which is what lets you work unattended.

Everything you write is posted to the people in this job's room as you write it, in Markdown, \
which is rendered — so write for them: what you found, what you changed, or what you could not \
do, and the answer, if the work was a question.

The `say` tool posts at the root of your room, or under a message when you name it with `to`, \
as each message is shown to you. Use it whenever you need an answer from a person: say what you \
need, then stop. It reaches somebody who can answer, but not now — no reply arrives in this \
session, so do not wait for one and do not guess.

Before you stop, call the `stopping` tool, every time and last of all. Say `ready_for_review` if \
you have done what was asked and there is something for a person to look at, or \
`waiting_for_an_answer` if you need something from a person before you can go on. Nothing else \
tells anybody which of the two this is, so a job that stops without calling it is recorded as \
having stopped for reasons nobody knows."
        );
    }

    /// A project with no variables gets the prompt it always got.
    ///
    /// Worth pinning separately from the snapshots, because it is what makes
    /// this change safe to land: most projects have none, and a paragraph that
    /// appeared as an empty gap — or left a stray blank line — would change
    /// every one of their prompts for nothing.
    #[test]
    fn a_project_with_no_variables_is_told_nothing_about_them() {
        let prompt = super::kickoff("https://example.invalid/repo", "anything", A_TUNNEL, NONE);

        assert!(!prompt.contains("in your environment"), "{prompt}");
        assert!(
            !prompt.contains("\n\n\n"),
            "an absent paragraph must leave no gap behind: {prompt:?}",
        );
    }

    /// Names travel; values have nowhere to travel in.
    ///
    /// The type is the defence — a `VariableName` cannot hold a credential —
    /// and this says so out loud, because a kickoff is stored on the job and
    /// written to the snapshot in the clear.
    #[test]
    fn a_kickoff_names_a_variable_and_says_it_is_a_credential() {
        let variables = [VariableName::new("STRIPE_API_KEY").expect("a deliverable name")];
        let prompt = super::kickoff(
            "https://example.invalid/repo",
            "anything",
            A_TUNNEL,
            &variables,
        );

        assert!(prompt.contains("STRIPE_API_KEY"), "{prompt}");
        assert!(
            prompt.contains("Treat each as a credential"),
            "an agent told about a credential has to be told it is one: {prompt}",
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
                A_TUNNEL,
                NONE,
            ),
            "You are working on https://example.invalid/repo.

It is checked out in the current directory, on its default branch. You have git and gh, both \
signed in as the account this work belongs to, and the `say` tool for talking to people.

The work:

Fix the flaky test in the parser.

You can show people what you are doing. Anything you serve inside this container on port 47201 \
is reachable at https://00000000-0000-0000-0000-000000000001.example.com — a dev server while \
you work, or a built result for somebody to look at before you propose it. Bind it to 0.0.0.0 \
and not to localhost: a server on localhost answers you from inside this container and is \
reachable from nowhere else. Say where to look, because nobody finds that address on their \
own.

When you have a change to propose, open a pull request and stop there. Do not merge it, do not \
deploy anything, and do not push to the default branch. Somebody reads what you propose before \
it counts for anything, which is what lets you work unattended.

Everything you write is posted to the people in this job's room as you write it, in Markdown, \
which is rendered — so write for them: what you found, what you changed, or what you could not \
do, and the answer, if the work was a question.

The `say` tool posts at the root of your room, or under a message when you name it with `to`, \
as each message is shown to you. Use it whenever you need an answer from a person: say what you \
need, then stop. It reaches somebody who can answer, but not now — no reply arrives in this \
session, so do not wait for one and do not guess.

Before you stop, call the `stopping` tool, every time and last of all. Say `ready_for_review` if \
you have done what was asked and there is something for a person to look at, or \
`waiting_for_an_answer` if you need something from a person before you can go on. Nothing else \
tells anybody which of the two this is, so a job that stops without calling it is recorded as \
having stopped for reasons nobody knows."
        );
    }

    /// Every job is told about the tool that speaks, because every job has
    /// one: its narration reaches the room on its own since
    /// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`,
    /// and the tool is for the thread a person asked in.
    #[test]
    fn a_kickoff_names_the_tool_that_speaks() {
        let prompt = super::kickoff("https://example.invalid/repo", "anything", A_TUNNEL, NONE);

        assert!(prompt.contains("`say` tool"), "{prompt}");
    }

    /// A kickoff never tells a job to wait, and that is the constraint
    /// `docs/open-questions.md` puts on this text until a reply can reach a
    /// running job: a question moves to the channel, and the job still stops.
    #[test]
    fn no_kickoff_tells_a_job_to_wait_for_an_answer() {
        let prompt = super::kickoff("https://example.invalid/repo", "anything", A_TUNNEL, NONE);

        assert!(prompt.contains("do not wait"), "{prompt}");
        assert!(prompt.contains("stop"), "{prompt}");
    }

    /// Asserted whole, per `docs/conventions.md` §4. Read by a person rather
    /// than an agent, which is exactly why nothing else would notice it
    /// changing.
    #[test]
    fn a_rooms_opening_reads_exactly_as_written() {
        assert_eq!(
            super::room_opening(
                "https://example.invalid/repo",
                "an issue was opened",
                "<@U0BOT>"
            ),
            "**A job on https://example.invalid/repo**

an issue was opened

_Everything it has to say appears here. Mention <@U0BOT> to talk to it; anything else said \
here is between people._"
        );
        assert_eq!(
            super::started_notice("<#C0C1VNX9AA2>"),
            "Started a job for this: <#C0C1VNX9AA2>."
        );
        assert_eq!(
            super::turn_notice(&super::Because::Message(Some(
                "https://example.slack.com/archives/C0123/p1788000000000100"
            ))),
            "▶️ Handling https://example.slack.com/archives/C0123/p1788000000000100."
        );
        assert_eq!(
            super::turn_notice(&super::Because::Message(None)),
            "▶️ Handling a message."
        );
        assert_eq!(
            super::turn_notice(&super::Because::Signal {
                app: "GitHub",
                link: Some("https://example.slack.com/archives/C0123/p1788000000000100"),
            }),
            "▶️ Judging what GitHub posted: \
             https://example.slack.com/archives/C0123/p1788000000000100."
        );
        assert_eq!(
            super::turn_notice(&super::Because::Signal {
                app: "GitHub",
                link: None,
            }),
            "▶️ Judging what GitHub posted."
        );
        assert_eq!(
            super::turn_notice(&super::Because::Restart(Some(
                "https://example.slack.com/archives/C0123/p1788000000000100"
            ))),
            "▶️ Picking up https://example.slack.com/archives/C0123/p1788000000000100 again \
             after a restart."
        );
        assert_eq!(
            super::turn_notice(&super::Because::Restart(None)),
            "▶️ Picking up again after a restart."
        );
        assert_eq!(
            super::answered_elsewhere_notice("<#C0C1VNX9AA2>"),
            "↩️ Answered at the root of <#C0C1VNX9AA2>."
        );
        assert_eq!(
            super::handled_elsewhere_notice(Some("<#C0C1VNX9AA2>")),
            "↩️ Handled without answering here; its notes are in <#C0C1VNX9AA2>."
        );
        assert_eq!(
            super::handled_elsewhere_notice(None),
            "↩️ Handled without answering here."
        );
        assert_eq!(
            super::foreman_room_purpose("aviary"),
            "Where aviary's foreman thinks: what it is handling, what it decided, and why."
        );
        assert_eq!(
            super::foreman_room_opening("aviary", "<@U0BOT>"),
            "**aviary's foreman.**

_Everything it says and does as it works appears here. Mention <@U0BOT> anywhere to talk to \
it; anything else said here is between people._"
        );
    }

    /// The opening teaches the mention, because the room is where a
    /// newcomer meets the rule and nothing else in the room ever says it.
    #[test]
    fn a_rooms_opening_teaches_the_mention() {
        let said = super::room_opening("https://example.invalid/repo", "why", "<@U0BOT>");

        assert!(said.contains("Mention <@U0BOT>"), "{said}");
        assert!(said.contains("between people"), "{said}");
    }

    /// Being read is the rule, not the exception — and this is why.
    ///
    /// The first version of this paragraph opened with *if you need an answer
    /// from a person*, and put finishing in a subordinate clause at the end of
    /// it. An agent given read-only work then has no question, no change to
    /// propose, and no reason to speak: it answers into a session nothing
    /// keeps, and the channel stays empty. That is not hypothetical — it is
    /// what the first real job did. Since
    /// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
    /// what a job writes is what a person reads, and the same order holds:
    /// it is told that first, and about the tool for a person's thread after.
    ///
    /// Asserted alongside the snapshot rather than left to it, because a
    /// snapshot is updated wholesale by whoever changes the text and records
    /// no opinion about which sentence mattered.
    #[test]
    fn a_kickoff_makes_being_read_the_rule() {
        let prompt = super::kickoff("https://example.invalid/repo", "anything", A_TUNNEL, NONE);

        let read = prompt
            .find("Everything you write is posted")
            .expect("it must say that what it writes is read");
        let asking = prompt
            .find("whenever you need an answer")
            .expect("and still offer the tool during the work");

        assert!(
            read < asking,
            "being read must come first, or it reads as a special case of asking: {prompt}"
        );
    }

    /// The lines a burst is built from, asserted whole, per
    /// `docs/conventions.md` §4: a person reads them in a room, and nothing
    /// else would notice them changing.
    #[test]
    fn the_working_lines_read_exactly_as_written() {
        use stageman_agent::{ToolCallStatus, ToolKind};

        assert_eq!(
            super::working_line(ToolKind::Execute, "cargo test", None),
            "⏳ ran `cargo test`"
        );
        assert_eq!(
            super::working_line(
                ToolKind::Execute,
                "cargo test",
                Some(ToolCallStatus::InProgress)
            ),
            "⏳ ran `cargo test`"
        );
        assert_eq!(
            super::working_line(
                ToolKind::Execute,
                "cargo test",
                Some(ToolCallStatus::Completed)
            ),
            "✅ ran `cargo test`"
        );
        assert_eq!(
            super::working_line(ToolKind::Edit, "src/lib.rs", Some(ToolCallStatus::Failed)),
            "❌ edited `src/lib.rs`"
        );
        assert_eq!(
            super::working_line(ToolKind::Read, "README.md", Some(ToolCallStatus::Completed)),
            "✅ read `README.md`"
        );
        assert_eq!(
            super::working_line(ToolKind::Other, "something", None),
            "⏳ did `something`"
        );
        assert_eq!(
            super::thought_line("The tests first.\nThen the fix."),
            "> 💭 The tests first.\n> Then the fix."
        );
    }

    /// Asserted as literal text, per `docs/conventions.md` §4.
    ///
    /// Every one of these is read by a person on a channel and by nothing
    /// else, so a rewrite would change what an operator sees and no other
    /// test would go red.
    #[test]
    fn the_notices_read_exactly_as_written() {
        assert_eq!(
            super::stopped_notice(&Waiting::Asked, Some("<@U0HUMAN>"), "<@U0BOT>"),
            "❓ **Waiting for an answer**, <@U0HUMAN>. Mention <@U0BOT> here to reply."
        );
        assert_eq!(
            super::stopped_notice(&Waiting::Asked, None, "<@U0BOT>"),
            "❓ **Waiting for an answer.** Mention <@U0BOT> here to reply."
        );
        assert_eq!(
            super::stopped_notice(&Waiting::Proposed, Some("<@U0HUMAN>"), "<@U0BOT>"),
            "✅ **Ready for review.** Mention <@U0BOT> here to send it back for changes."
        );
        assert_eq!(
            super::stopped_notice(&Waiting::Paused, None, "<@U0BOT>"),
            "⏸️ **Stopped by an operator.** Mention <@U0BOT> here to carry on."
        );
        assert_eq!(
            super::stopped_notice(
                &Waiting::Failed("the credential had expired".to_owned()),
                None,
                "<@U0BOT>"
            ),
            "❌ **Failed:** `the credential had expired`. Mention <@U0BOT> here to try again once \
that is fixed."
        );
        assert_eq!(
            super::stopped_notice(&Waiting::Silent, None, "<@U0BOT>"),
            "⏹️ **Stopped without saying why.** Mention <@U0BOT> here to carry on."
        );
        assert_eq!(
            super::stuck_notice("the agent would not start"),
            "❌ That could not be handled: `the agent would not start`. It will not be retried — \
send it again once that is fixed."
        );
        assert_eq!(
            super::busy_notice(),
            "⏳ Still working, so that did not reach it. Say it again once it stops."
        );
        assert_eq!(
            super::resumed_notice(),
            "stageman restarted while working on this, and is picking it up again."
        );
        assert_eq!(
            super::over_notice(),
            "This job is over, so nothing more reaches it. Start a new job if there is still work \
here."
        );
    }

    /// An interrupted turn is told so, and told before it is told anything
    /// else.
    ///
    /// Asserted as literal text, per `docs/conventions.md` §4, and asserted
    /// *in order* because the order is the whole of it: an agent that read its
    /// instructions before hearing it was interrupted has already decided what
    /// to do about them.
    #[test]
    fn an_interrupted_turn_is_told_first_that_it_was_interrupted() {
        let picked = super::asked(
            super::Turn {
                said: "look at the parser",
                target: "C0123/1788000000.000100",
                thread: None,
                starting: super::Starting::Interrupted,
                app: None,
            },
            &[("claude", "General-purpose.")],
            "",
        );

        assert!(picked.starts_with(super::INTERRUPTION), "{picked}");
        assert!(picked.contains("look at the parser"), "{picked}");
    }

    /// The message underneath is untouched, whichever way the turn starts.
    ///
    /// The notice is prepended rather than woven in, so that what a person
    /// wrote reaches the agent as they wrote it — and so that adding the
    /// notice cannot change a prompt that was already snapshot-tested.
    #[test]
    fn an_interrupted_turn_is_the_fresh_one_with_a_paragraph_in_front() {
        let agents = [("claude", "General-purpose.")];
        let first = super::asked(fresh("look at the parser"), &agents, "");
        let again = super::asked(
            super::Turn {
                said: "look at the parser",
                target: "C0123/1788000000.000100",
                thread: None,
                starting: super::Starting::Interrupted,
                app: None,
            },
            &agents,
            "",
        );

        assert_eq!(again, format!("{}\n\n{first}", super::INTERRUPTION));
    }

    /// What an interrupted turn must not be told: that anything failed.
    ///
    /// The same trap `RESUMPTION` names for a job, and worse here — a foreman
    /// that assumes its last step did not happen starts a second job for a
    /// message that already has one, and says so on the channel twice.
    #[test]
    fn an_interrupted_turn_is_told_to_check_rather_than_to_assume() {
        assert!(super::INTERRUPTION.contains("Do not assume your last step completed"));
        assert!(super::INTERRUPTION.contains("do not assume it did not"));
        assert!(super::INTERRUPTION.contains("a job you started"));
    }

    /// The stuck notice must say the message is gone, not that it is queued.
    ///
    /// It is deliberately not retried — a message that cannot be handled must
    /// not become one handled for ever, blocking everything behind it — so a
    /// person has to be told to send it again. A notice that merely apologised
    /// would leave them waiting for a turn that is never coming.
    #[test]
    fn the_stuck_notice_says_the_message_was_not_kept_and_why() {
        let said = super::stuck_notice("the disk is full");

        assert!(said.contains("send it again"), "{said}");
        assert!(said.contains("not be retried"), "{said}");
        assert!(said.contains("the disk is full"), "{said}");
    }

    /// Every reading of idle has a line of its own, each says how to reach
    /// the job, and only the one waiting on a person names them.
    ///
    /// The instance passes on the agent's claim and never guesses, so five
    /// readings are five texts: one that collapsed two would tell an
    /// operator to do the wrong thing about one of them.
    #[test]
    fn every_reading_of_idle_has_a_line_of_its_own() {
        let readings = [
            Waiting::Asked,
            Waiting::Proposed,
            Waiting::Paused,
            Waiting::Failed("why".to_owned()),
            Waiting::Silent,
        ];
        let lines: Vec<String> = readings
            .iter()
            .map(|waiting| super::stopped_notice(waiting, Some("<@U0HUMAN>"), "<@U0BOT>"))
            .collect();

        for (i, line) in lines.iter().enumerate() {
            assert!(line.contains("Mention <@U0BOT> here"), "{line}");
            assert!(
                lines.iter().filter(|other| *other == line).count() == 1,
                "reading {i} shares its line with another: {line}"
            );
        }
        assert!(
            lines[0].contains("<@U0HUMAN>"),
            "asked names who is waited on"
        );
        for line in &lines[1..] {
            assert!(!line.contains("<@U0HUMAN>"), "nobody is waited on: {line}");
        }
    }

    /// Asserted whole, per `docs/conventions.md` §4.
    #[test]
    fn a_reply_reads_exactly_as_written() {
        assert_eq!(
            super::reply("use postgres", "C0123/1788000000.000100", None),
            "A person replied on the channel:

use postgres

To answer them where they asked, call `say` with `to` set to `C0123/1788000000.000100`; whatever you \
write without it is posted at the root of your room. Carry on from there. The same rules still \
hold: propose rather than merge, and say what you did when you finish."
        );
    }

    /// What a turn is shown of its thread, asserted whole: who said each
    /// message, its identifier, both leads, and the note that the thread
    /// was longer. A reply or a message with a thread reads the thread
    /// first, and the frame after it is the one without.
    #[test]
    fn a_thread_shown_reads_exactly_as_written() {
        let shown = [
            super::Shown {
                id: "C0123/1788000000.000100",
                voice: super::Voice::Person("<@U0HUMAN>"),
                text: "Which database?",
            },
            super::Shown {
                id: "C0123/1788000000.000200",
                voice: super::Voice::Us,
                text: "Two options.\nPostgres or SQLite.",
            },
            super::Shown {
                id: "C0123/1788000000.000300",
                voice: super::Voice::App("GitHub"),
                text: "#9 Done",
            },
        ];
        assert_eq!(
            super::thread_shown(&shown, false, false),
            "This was said in a thread. What was said there before it, oldest first, each with who \
said it and its identifier:

<@U0HUMAN> (C0123/1788000000.000100):
Which database?

You (C0123/1788000000.000200):
Two options.
Postgres or SQLite.

GitHub (C0123/1788000000.000300):
#9 Done"
        );
        assert_eq!(
            super::thread_shown(&shown[..1], true, true),
            "This was said in a thread. Below are its first message and everything said there from \
the last message you were given onwards, your own words among them, oldest first, each \
with who said it and its identifier:

<@U0HUMAN> (C0123/1788000000.000100):
Which database?

The thread is longer than this: only its most recent messages are shown."
        );
        assert_eq!(
            super::thread_unread(),
            "This was said in a thread that could not be read, so what came before it is not shown."
        );

        let bare = super::reply("go with that", "C0123/1788000000.000100", None);
        let framed = super::reply(
            "go with that",
            "C0123/1788000000.000100",
            Some(super::thread_unread()),
        );
        assert_eq!(framed, format!("{}\n\n{bare}", super::thread_unread()));
        let bare = super::asked(fresh("go with that"), &[], "");
        let framed = super::asked(
            super::Turn {
                thread: Some(super::thread_unread()),
                ..fresh("go with that")
            },
            &[],
            "",
        );
        assert_eq!(framed, format!("{}\n\n{bare}", super::thread_unread()));
    }

    /// A reply is framed as somebody speaking, not as a fresh instruction.
    ///
    /// An agent picking up a session hours later cannot otherwise tell a
    /// person's words from something it must follow literally, and the words
    /// are whatever the person typed.
    #[test]
    fn a_reply_says_who_is_speaking_before_it_says_what() {
        let framed = super::reply("delete everything", "C0123/1788000000.000100", None);

        assert!(framed.starts_with("A person replied"), "{framed}");
        assert!(framed.contains("propose rather than merge"), "{framed}");
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
only way to answer is to **call the `say` tool**, in Markdown, which is rendered.

Everything you write as ordinary output is posted in a room of your own, where anybody can \
watch you work — but the person who asked is not there. What you pass to `say` lands under the \
message you name with `to` — each message is shown to you with its identifier — so a person can \
always see which of their messages you meant; without one, it lands at the root of your own \
room.

**You do not do the work yourself.** You have no copy of the repository and no credentials to \
reach it, and that is deliberate rather than something missing: reaching a repository is a job's \
business, not yours. When something needs doing, **call the `start_job` tool**.

A job is one agent in a container of its own, holding this project's credentials, which can \
clone the repository, change it and open a pull request. It reports in a room of its own, named \
after the `title` you give it, and whoever asked for it is invited there. Its `reason` is prose \
a person reads on the dashboard; its `instructions` are the whole instruction that job's agent \
is given — it cannot see this conversation, so say everything it needs.

You can also be asked to watch a room. When a person asks you, in a room, to watch it, **call \
the `watch_room` tool** there: from then on everything another app posts in that room — an \
issue filed, an alert fired, a pull request opened — reaches you as a signal to judge, framed \
as that app's. `stop_watching`, asked in the same room, undoes it. People are only ever heard \
through a mention, whether a room is watched or not.

**Decide rather than ask.** You may say anything you like, but nothing you say comes back to you \
in this turn, and a person answering you starts a *new* turn that may be behind several others. \
So never end a turn waiting for a reply: if you need a judgement nobody has given you, make the \
most reasonable one available and say plainly what you chose and why. Somebody reading the \
channel can correct you, and that correction is its own message.

You remember everything from earlier turns, so do not ask again for what you have already been \
told."
        );
    }

    /// A turn on a message nothing has begun, which is nearly every turn.
    fn fresh(said: &str) -> super::Turn<'_> {
        super::Turn {
            said,
            target: "C0123/1788000000.000100",
            thread: None,
            starting: super::Starting::Fresh,
            app: None,
        }
    }

    /// A signal: what another app posted in a watched room.
    fn signalled<'a>(said: &'a str, app: &'a str) -> super::Turn<'a> {
        super::Turn {
            said,
            target: "C0123/1788000000.000100",
            thread: None,
            starting: super::Starting::Fresh,
            app: Some(app),
        }
    }

    /// Asserted whole, and framed as somebody speaking.
    #[test]
    fn a_message_to_a_foreman_reads_exactly_as_written() {
        assert_eq!(
            super::asked(
                fresh("look at the parser"),
                &[("claude", "General-purpose.")],
                ""
            ),
            "A person said this to you on the channel:

look at the parser

Answer it, or start a job for it with the `start_job` tool, or both. Then call `say` before you \
finish, with `to` set to `C0123/1788000000.000100`, so that your answer lands under their message: a \
turn that ends without calling it has told nobody anything, however much you wrote.

The kits this project's jobs may run on — each an agent, set a particular way — and what each \
is for:

  claude — General-purpose.

Choose one deliberately and name it first. It is your judgement to make — \
`docs/decisions/0048-a-job-runs-on-a-kit.md` — and the list is said here rather than at the \
start of this session because it can change while you are still running.

If a command fails, say what it printed rather than what you think it meant. An explanation you \
inferred is one a person will act on, and you have no way to check it."
        );
    }

    /// Asserted whole, per `docs/conventions.md` §4: a signal is framed as
    /// the app's, and the foreman is told to judge rather than to answer.
    #[test]
    fn a_signal_to_a_foreman_reads_exactly_as_written() {
        assert_eq!(
            super::asked(
                signalled(
                    "Issue created by somebody\n#9 The parser is flaky",
                    "GitHub"
                ),
                &[("claude", "General-purpose.")],
                ""
            ),
            "GitHub posted this in a room you watch:

Issue created by somebody
#9 The parser is flaky

Nobody asked you anything: this is a signal, and yours to judge. Decide what it deserves — \
nothing, a job started with the `start_job` tool, or a word to the people in that room — and do \
that. Call `say` only if you acted on it or a person needs to know something, with `to` set to \
`C0123/1788000000.000100` to speak under what GitHub posted; the reaction on the message already says \
you looked, so a turn that ends in silence is a decision, not a failure.

The kits this project's jobs may run on — each an agent, set a particular way — and what each \
is for:

  claude — General-purpose.

Choose one deliberately and name it first. It is your judgement to make — \
`docs/decisions/0048-a-job-runs-on-a-kit.md` — and the list is said here rather than at the \
start of this session because it can change while you are still running.

If a command fails, say what it printed rather than what you think it meant. An explanation you \
inferred is one a person will act on, and you have no way to check it."
        );
    }

    /// A signal is told it need not speak, and a person's message is told
    /// it must: the two framings differ in exactly the sentence that
    /// decides whether a room fills with acknowledgements.
    #[test]
    fn a_signal_is_told_to_speak_only_if_it_acted_and_a_message_is_told_to_answer() {
        let kits = [("claude", "General-purpose.")];
        let signal = super::asked(signalled("an alert", "Alerts"), &kits, "");
        let message = super::asked(fresh("an alert"), &kits, "");

        assert!(
            signal.starts_with("Alerts posted this in a room you watch"),
            "{signal}"
        );
        assert!(signal.contains("only if you acted"), "{signal}");
        assert!(signal.contains("reaction on the message"), "{signal}");
        assert!(!signal.contains("Answer it"), "{signal}");
        assert!(
            message.contains("Then call `say` before you finish"),
            "{message}"
        );
        assert!(!message.contains("only if you acted"), "{message}");
    }

    /// The brief is said every turn, after the message and before the kits,
    /// and an empty one leaves the prompt byte-for-byte what it was.
    #[test]
    fn the_brief_is_said_every_turn_and_an_empty_one_says_nothing() {
        let kits = [("claude", "General-purpose.")];
        let without = super::asked(fresh("look at the parser"), &kits, "");
        let blank = super::asked(fresh("look at the parser"), &kits, "  \n ");
        assert_eq!(without, blank, "whitespace is no brief");
        assert!(!without.contains("brief"), "{without}");

        let with = super::asked(
            fresh("look at the parser"),
            &kits,
            "Ignore alerts below error. Our jobs act as the machine user stageman-bot.\n",
        );
        assert_eq!(
            with,
            "A person said this to you on the channel:

look at the parser

Answer it, or start a job for it with the `start_job` tool, or both. Then call `say` before you \
finish, with `to` set to `C0123/1788000000.000100`, so that your answer lands under their message: a \
turn that ends without calling it has told nobody anything, however much you wrote.

The operator's brief for this project — standing instructions, in their own words, that apply \
to every message and every signal:

Ignore alerts below error. Our jobs act as the machine user stageman-bot.

The kits this project's jobs may run on — each an agent, set a particular way — and what each \
is for:

  claude — General-purpose.

Choose one deliberately and name it first. It is your judgement to make — \
`docs/decisions/0048-a-job-runs-on-a-kit.md` — and the list is said here rather than at the \
start of this session because it can change while you are still running.

If a command fails, say what it printed rather than what you think it meant. An explanation you \
inferred is one a person will act on, and you have no way to check it."
        );
        // And never in the opening, which is said once and cannot be revised.
        assert!(
            !super::opening("https://example.invalid/repo").contains("brief"),
            "a brief said once goes stale the first time it is edited"
        );
    }

    /// The opening teaches the tool that watches a room, and that people
    /// are still heard only through a mention.
    #[test]
    fn a_foreman_is_told_how_a_room_comes_to_be_watched() {
        let told = super::opening("https://example.invalid/repo");

        assert!(told.contains("call the `watch_room` tool"), "{told}");
        assert!(told.contains("`stop_watching`"), "{told}");
        assert!(told.contains("only ever heard through a mention"), "{told}");
    }

    /// The instruction has to name the tool, and say ordinary output is lost.
    ///
    /// **Found by running it**, when saying was a program. The opening read
    /// "you answer with stageman-say", which is perfectly clear English and
    /// behaved wrong: the agent took it for a tool name, searched for one,
    /// found nothing, and answered in ordinary output — which the daemon
    /// discards. The turn ended cleanly, nothing failed, and the person who
    /// asked got silence.
    ///
    /// `docs/decisions/0034-tools-are-served-not-shipped.md` resolved that by
    /// agreeing with the agent: it is a tool now, so the reflex that was wrong
    /// is right. What survives is the second half of the lesson, which was
    /// never about the mechanism — an agent has no way to know where its
    /// ordinary output goes, so it has to be told. Since
    /// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
    /// it goes to a room of the foreman's own, which the person who asked
    /// is not in, and the lesson holds in that form.
    #[test]
    fn a_foreman_is_told_which_tool_answers_and_where_its_output_goes() {
        let told = super::opening("https://example.invalid/repo");

        assert!(
            told.contains("call the `say` tool"),
            "the tool that answers has to be named: {told}"
        );
        // The half of the original lesson that outlived the mechanism: an
        // agent has no way to know its ordinary output does not reach the
        // person, and one that is not told believes it has answered when it
        // has not.
        assert!(
            told.contains(
                "Everything you write as ordinary output is posted in a room of your own"
            ),
            "{told}"
        );
        assert!(told.contains("the person who asked is not there"), "{told}");
    }

    /// A foreman is told to report what failed, not to explain it.
    ///
    /// **Found by watching one do the opposite.** A command answered with a
    /// bare status, and the foreman told a person the job could not start
    /// because only one may run at a time — a rule that does not exist, stated
    /// with confidence, and acted on. An invented explanation is worse than no
    /// explanation, because it stops the person looking.
    #[test]
    fn a_foreman_is_told_to_report_a_failure_rather_than_explain_it() {
        let every = super::asked(fresh("do the thing"), &[("claude", "General-purpose.")], "");

        assert!(every.contains("say what it printed"), "{every}");
        assert!(every.contains("no way to check it"), "{every}");
    }

    /// The kits are named every turn, never once at the start.
    ///
    /// A project's kits are edited from the dashboard, and a foreman's session
    /// outlives those edits — it lasts as long as the project. Said in the
    /// opening, the list would be right until somebody changed it and wrong
    /// from then on, with nothing to notice and a foreman naming a kit that is
    /// no longer offered.
    #[test]
    fn the_kits_a_job_may_run_on_are_said_every_turn() {
        let each = super::asked(
            fresh("do the thing"),
            &[("claude", "General-purpose."), ("other", "Narrow.")],
            "",
        );

        assert!(each.contains("claude — General-purpose."), "{each}");
        assert!(each.contains("other — Narrow."), "{each}");
        assert!(each.contains("name it first"), "{each}");

        // And never in the opening, which is said once and cannot be revised.
        let once = super::opening("https://example.invalid/repo");
        assert!(
            !once.contains("General-purpose."),
            "a list said once goes stale the first time a project is edited: {once}"
        );
    }

    /// The instruction has to say a foreman assigns work rather than doing it.
    ///
    /// **Found by running it.** The opening described only the talking half,
    /// so a foreman asked to change a repository tried to change it, found it
    /// had no credentials, and reported that as a blocker — correctly, since
    /// it has none and never will. Every word of that answer was true and the
    /// whole of it was the wrong thing to do.
    ///
    /// The missing credentials are asserted too, because a foreman that is not
    /// told the absence is deliberate will keep reporting it as broken
    /// configuration.
    #[test]
    fn a_foreman_is_told_to_start_jobs_rather_than_do_the_work() {
        let told = super::opening("https://example.invalid/repo");

        assert!(told.contains("start_job"), "{told}");
        assert!(told.contains("do not do the work yourself"), "{told}");
        assert!(
            told.contains("deliberate rather than something missing"),
            "an absence it is not told is deliberate reads as a fault: {told}"
        );
        // The job's instruction has to stand alone, because its agent never
        // sees the conversation that produced it.
        assert!(told.contains("cannot see this conversation"), "{told}");
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
            A_TUNNEL,
            NONE,
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
