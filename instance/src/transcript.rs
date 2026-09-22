//! What a turn posts as it happens — the transcript of
//! `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`
//! — and how each of its messages grows.
//!
//! A job's agent is read as it works. Its text is narration, posted one
//! message per contiguous run; what it does between two runs is working,
//! posted as one burst listing each tool call and, where an adapter carries
//! any, each thought. Either kind is posted when it opens, grown in place by
//! editing as it continues — paced, so that a busy agent stays inside the
//! platform's budget for edits — and closed when the other kind begins or
//! the turn ends. A message that would pass the platform's limit is closed
//! at it and continued in the next.
//!
//! Held per turn and never kept: a restart begins with nothing open, and
//! whatever was growing stays as it was last sent.

use std::time::Duration;

use stageman_agent::{Noticed, ToolCallStatus, ToolKind};

use stageman_core::{Place, Speaking};

use crate::turns::speaking_for;
use crate::vocabulary::Speaker;
use crate::{Effect, Running, Timer};

/// How often a growing message is edited, at most: often enough to read as
/// it happens, and well inside the platform's budget for edits.
pub const PACE: Duration = Duration::from_secs(2);

/// One entry of a burst of working.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub enum Entry {
    /// A tool call, with where it has got to.
    Call {
        /// The adapter's identifier for it.
        id: String,
        /// What kind of thing it does.
        kind: ToolKind,
        /// What the adapter calls it.
        title: String,
        /// Where it has got to, once the adapter said.
        status: Option<ToolCallStatus>,
        /// Whether a message landed while it ran, in which case the adapter
        /// was measured to send no ending for it — see
        /// `docs/decisions/0069-a-message-reaches-a-working-job.md`.
        interrupted: bool,
    },
    /// A thought, where an adapter carries any.
    Thought(String),
}

impl Entry {
    /// Whether this is a call that has not ended, as far as the adapter has
    /// said, and that no message cut short.
    const fn is_running(&self) -> bool {
        matches!(
            self,
            Self::Call {
                status: None | Some(ToolCallStatus::Pending | ToolCallStatus::InProgress),
                interrupted: false,
                ..
            }
        )
    }
}

/// What a message of the transcript holds.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub enum Body {
    /// The agent's own text.
    Narration(String),
    /// What it did between two runs of narration.
    Working(Vec<Entry>),
}

impl Body {
    /// The message as it should read now.
    fn text(&self) -> String {
        match self {
            Self::Narration(text) => text.clone(),
            Self::Working(entries) => entries
                .iter()
                .map(|entry| match entry {
                    Entry::Call {
                        kind,
                        title,
                        interrupted: true,
                        ..
                    } => stageman_foreman::interrupted_line(*kind, title),
                    Entry::Call {
                        kind,
                        title,
                        status,
                        ..
                    } => stageman_foreman::working_line(*kind, title, *status),
                    Entry::Thought(text) => stageman_foreman::thought_line(text),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Whether a call in it has not ended, as far as the adapter has said.
    fn has_a_call_running(&self) -> bool {
        match self {
            Self::Narration(_) => false,
            Self::Working(entries) => entries.iter().any(Entry::is_running),
        }
    }
}

/// Where a message of the transcript has got to.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum Standing {
    /// More may be added to it.
    Growing,
    /// Nothing more will be added to it.
    Closed,
    /// The post that opened it was refused, so it cannot grow and is let
    /// go.
    Lost,
}

/// One message of the transcript, from its opening to its last edit.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct Open {
    /// Which, in the order its turn opened them.
    run: u64,
    /// What it holds.
    body: Body,
    /// The text as last sent to the platform, posted or edited.
    sent: String,
    /// The platform's identifier for it, once the post that opened it is
    /// answered.
    message: Option<String>,
    /// Whether a post or an edit of it is waiting to be answered.
    awaiting: bool,
    /// Whether a wake to grow it is set.
    pacing: bool,
    /// Where it has got to.
    standing: Standing,
}

impl Open {
    /// Whether it was let go of.
    const fn lost(&self) -> bool {
        matches!(self.standing, Standing::Lost)
    }

    /// Whether nothing more will be added to it.
    const fn closed(&self) -> bool {
        matches!(self.standing, Standing::Closed | Standing::Lost)
    }

    /// Whether it is as sent and nothing about it is in flight.
    fn settled(&self) -> bool {
        !self.awaiting && !self.pacing && (self.lost() || self.body.text() == self.sent)
    }

    /// Whether it needs nothing more while its turn runs: settled, closed,
    /// and with no call still to end, whose ending would change it.
    fn done(&self) -> bool {
        self.closed() && self.settled() && (self.lost() || !self.body.has_a_call_running())
    }
}

/// What one piece of the transcript did to a turn's messages.
#[derive(Default)]
struct Change {
    /// A message closed, to be sent as it finally reads.
    closed: Option<u64>,
    /// A message opened, to be posted.
    opened: Option<u64>,
    /// A message that grew or changed, to be grown when its pace allows.
    grew: Option<u64>,
}

/// What one turn is posting: the message being added to, and those closed
/// but not yet done with.
#[derive(Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Transcript {
    /// The next message's number.
    next: u64,
    /// The message being added to, if any.
    open: Option<Open>,
    /// Messages closed but not done: an answer still to come, a call still
    /// to end.
    closing: Vec<Open>,
}

impl Transcript {
    /// Nothing open and nothing closing: what a turn starts with.
    pub const fn new() -> Self {
        Self {
            next: 0,
            open: None,
            closing: Vec::new(),
        }
    }

    /// Whether this holds the message numbered.
    fn has(&self, run: u64) -> bool {
        self.open.as_ref().is_some_and(|open| open.run == run)
            || self.closing.iter().any(|open| open.run == run)
    }

    /// The message numbered, open or closing.
    fn find(&mut self, run: u64) -> Option<&mut Open> {
        if self.open.as_ref().is_some_and(|open| open.run == run) {
            return self.open.as_mut();
        }
        self.closing.iter_mut().find(|open| open.run == run)
    }

    /// Closes the open message, if there is one, and says which.
    fn close(&mut self) -> Option<u64> {
        let mut open = self.open.take()?;
        if !open.lost() {
            open.standing = Standing::Closed;
        }
        let run = open.run;
        self.closing.push(open);
        Some(run)
    }

    /// Opens a message, and says which.
    fn begin(&mut self, body: Body) -> u64 {
        let run = self.next;
        // A turn opens a message per run of narration or working; coming
        // round is not a case.
        self.next = self.next.wrapping_add(1); // CLAMP-OK: messages per turn never come round.
        self.open = Some(Open {
            run,
            body,
            sent: String::new(),
            message: None,
            awaiting: false,
            pacing: false,
            standing: Standing::Growing,
        });
        run
    }

    /// The agent said something: onto the narration it is in, or a new one.
    fn said(&mut self, text: String) -> Change {
        if let Some(open) = &mut self.open
            && let Body::Narration(narration) = &mut open.body
        {
            narration.push_str(&text);
            return Change {
                grew: Some(open.run),
                ..Change::default()
            };
        }
        let closed = self.close();
        let opened = self.begin(Body::Narration(text));
        Change {
            closed,
            opened: Some(opened),
            ..Change::default()
        }
    }

    /// The agent did something: onto the burst it is in, or a new one. A
    /// thought following a thought continues it.
    fn worked(&mut self, entry: Entry) -> Change {
        if let Some(open) = &mut self.open
            && let Body::Working(entries) = &mut open.body
        {
            match (entries.last_mut(), entry) {
                (Some(Entry::Thought(thought)), Entry::Thought(more)) => thought.push_str(&more),
                (_, entry) => entries.push(entry),
            }
            return Change {
                grew: Some(open.run),
                ..Change::default()
            };
        }
        let closed = self.close();
        let opened = self.begin(Body::Working(vec![entry]));
        Change {
            closed,
            opened: Some(opened),
            ..Change::default()
        }
    }

    /// A message landed in the turn: every call still running is marked
    /// interrupted, since no ending will arrive for it. Which messages grew.
    fn interrupt_running(&mut self) -> Vec<u64> {
        let mut grew = Vec::new();
        for open in self.open.iter_mut().chain(self.closing.iter_mut()) {
            let Body::Working(entries) = &mut open.body else {
                continue;
            };
            for entry in entries.iter_mut() {
                if entry.is_running()
                    && let Entry::Call { interrupted, .. } = entry
                {
                    *interrupted = true;
                    grew.push(open.run);
                }
            }
        }
        grew.dedup();
        grew
    }

    /// A call was refined or ended, wherever its line is.
    fn call_changed(
        &mut self,
        id: &str,
        title: Option<String>,
        status: Option<ToolCallStatus>,
    ) -> Change {
        for open in self.open.iter_mut().chain(self.closing.iter_mut()) {
            let Body::Working(entries) = &mut open.body else {
                continue;
            };
            for entry in entries.iter_mut() {
                if let Entry::Call {
                    id: called,
                    title: named,
                    status: state,
                    ..
                } = entry
                    && called == id
                {
                    if let Some(title) = title {
                        *named = title;
                    }
                    if status.is_some() {
                        *state = status;
                    }
                    return Change {
                        grew: Some(open.run),
                        ..Change::default()
                    };
                }
            }
        }
        Change::default()
    }

    /// Closes the open message at the platform's limit when it would pass
    /// it, continuing in a new one: narration cut where the channel cuts
    /// it, and a burst at its last entry.
    fn fit(&mut self, pieces: impl Fn(&str) -> Vec<String>) -> Change {
        let Some(open) = &mut self.open else {
            return Change::default();
        };
        let mut cut = pieces(&open.body.text()).into_iter();
        let (Some(first), Some(second)) = (cut.next(), cut.next()) else {
            return Change::default();
        };
        let rest = match &mut open.body {
            Body::Narration(text) => {
                *text = first;
                Body::Narration(
                    std::iter::once(second)
                        .chain(cut)
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            Body::Working(entries) => {
                if entries.len() < 2 {
                    return Change::default();
                }
                let Some(last) = entries.pop() else {
                    return Change::default();
                };
                Body::Working(vec![last])
            }
        };
        let closed = self.close();
        let opened = self.begin(rest);
        Change {
            closed,
            opened: Some(opened),
            ..Change::default()
        }
    }

    /// Lets go of what is done with.
    ///
    /// Skipped by mutation testing: it frees what nothing will ask about
    /// again, so keeping it changes what is held and nothing that is said.
    #[mutants::skip]
    fn prune(&mut self) {
        self.closing.retain(|open| !open.done());
    }
}

/// What tending a message decided to do about it.
enum Tending {
    /// Nothing, for now.
    Nothing,
    /// Post it, opening it on the platform.
    Post(String),
    /// Edit it to read as it now does.
    Edit {
        /// The platform's identifier for it.
        message: String,
        /// What it now says.
        text: String,
    },
    /// Wake later and look again.
    Pace,
}

impl Running {
    /// Where a speaker's transcript goes, if it has anywhere: the root of
    /// the room it owns, a job's own or its project's foreman's.
    fn transcript_place(&self, speaker: Speaker) -> Option<(Speaking, Place)> {
        match speaker {
            Speaker::Job(job) => speaking_for(&self.state, job),
            Speaker::Foreman(project) => {
                let watched = self.state.projects.get(&project)?;
                let room = watched.foreman_room.clone()?;
                let bound = watched.channels.get(&room.channel)?;
                Some((bound.speaking(), Place::root(room)))
            }
        }
    }

    /// Something a turn's agent said or did, onto the message it is growing
    /// or a new one; and whatever that closed, opened or changed, sent.
    ///
    /// Let go when the speaker has no room: a job with none, or a foreman
    /// whose room could not be made.
    pub fn noticed(&mut self, speaker: Speaker, noticed: Noticed) {
        let Some((_, root)) = self.transcript_place(speaker) else {
            return;
        };
        let channel = root.room.channel;
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        let change = match noticed {
            Noticed::Said(text) => turn.transcript.said(text),
            Noticed::Thought(text) => turn.transcript.worked(Entry::Thought(text)),
            Noticed::Called { id, title, kind } => turn.transcript.worked(Entry::Call {
                id,
                kind,
                title,
                status: None,
                interrupted: false,
            }),
            Noticed::CallChanged { id, title, status } => {
                turn.transcript.call_changed(&id, title, status)
            }
        };
        let overflow = turn
            .transcript
            .fit(|text| stageman_channel::pieces(channel, text));
        // What closed is sent as it finally reads; what opened is posted,
        // in the order opened, so that the chain lands them in order; what
        // grew is grown at its pace.
        for run in [change.closed, overflow.closed].into_iter().flatten() {
            self.tend(speaker, run);
        }
        for run in [change.opened, overflow.opened].into_iter().flatten() {
            self.tend(speaker, run);
        }
        if let Some(run) = change.grew {
            self.tend(speaker, run);
        }
        self.prune(speaker);
    }

    /// A message landed in a speaker's turn: every tool call still running
    /// in its transcript is marked interrupted, and the messages that
    /// changed are grown.
    pub fn calls_interrupted(&mut self, speaker: Speaker) {
        let Some(turn) = self.turns.get_mut(&speaker) else {
            return;
        };
        for run in turn.transcript.interrupt_running() {
            self.tend(speaker, run);
        }
    }

    /// A turn ended: its open message closes, and what is not yet sent as
    /// it reads is kept until it is, since the answers to come will find
    /// no turn.
    pub fn finish(&mut self, speaker: Speaker, mut transcript: Transcript) {
        transcript.close();
        let runs: Vec<u64> = transcript.closing.iter().map(|open| open.run).collect();
        for open in transcript.closing.drain(..) {
            self.finishing.insert((speaker, open.run), open);
        }
        for run in runs {
            self.tend(speaker, run);
        }
        self.prune(speaker);
    }

    /// The platform answered the post that opened a message: it can grow
    /// now, or it never will.
    pub fn run_posted(&mut self, speaker: Speaker, run: u64, outcome: Result<String, String>) {
        if let Some(open) = self.find_run(speaker, run) {
            open.awaiting = false;
            match outcome {
                Ok(message) => open.message = Some(message),
                Err(why) => {
                    tracing::warn!(%why, "a message of the transcript could not be posted");
                    open.standing = Standing::Lost;
                }
            }
        }
        self.tend(speaker, run);
        self.prune(speaker);
    }

    /// The platform answered an edit of a message.
    pub fn run_grown(&mut self, speaker: Speaker, run: u64, outcome: Result<(), String>) {
        if let Some(open) = self.find_run(speaker, run) {
            open.awaiting = false;
            if let Err(why) = outcome {
                tracing::warn!(%why, "a message of the transcript could not be grown");
                open.standing = Standing::Lost;
            }
        }
        self.tend(speaker, run);
        self.prune(speaker);
    }

    /// The pace came round for a message: it is edited to read as it now
    /// does, if that differs from what was sent, whether or not it is still
    /// growing. That is what pacing paces.
    pub fn grow(&mut self, speaker: Speaker, run: u64) {
        if let Some(open) = self.find_run(speaker, run) {
            open.pacing = false;
        }
        self.tend_after(speaker, run, true);
        self.prune(speaker);
    }

    /// The message numbered, in its turn or among those finishing.
    fn find_run(&mut self, speaker: Speaker, run: u64) -> Option<&mut Open> {
        let in_turn = self
            .turns
            .get(&speaker)
            .is_some_and(|turn| turn.transcript.has(run));
        if in_turn {
            self.turns.get_mut(&speaker)?.transcript.find(run)
        } else {
            self.finishing.get_mut(&(speaker, run))
        }
    }

    /// Does what a message needs: posts it if it is not yet on the platform
    /// and has something to say; edits it if it reads differently from what
    /// was sent and is closed, or its pace has come round; waits a pace
    /// otherwise, so that a growing message is edited at most that often.
    /// Nothing while an answer is awaited, since the answer looks again.
    fn tend(&mut self, speaker: Speaker, run: u64) {
        self.tend_after(speaker, run, false);
    }

    /// [`Running::tend`], saying whether the message's pace has come round,
    /// which is when a message still growing is edited.
    fn tend_after(&mut self, speaker: Speaker, run: u64, paced: bool) {
        let Some((speaking, place)) = self.transcript_place(speaker) else {
            return;
        };
        let tending = {
            let Some(open) = self.find_run(speaker, run) else {
                return;
            };
            if open.lost() || open.awaiting {
                Tending::Nothing
            } else {
                let text = open.body.text();
                match &open.message {
                    None if text.trim().is_empty() => {
                        // Nothing to post yet; and nothing ever, if closed.
                        if open.closed() {
                            open.standing = Standing::Lost;
                        }
                        Tending::Nothing
                    }
                    None => {
                        open.sent.clone_from(&text);
                        open.awaiting = true;
                        Tending::Post(text)
                    }
                    Some(_) if text == open.sent => Tending::Nothing,
                    Some(message) if open.closed() || paced => {
                        let message = message.clone();
                        open.sent.clone_from(&text);
                        open.awaiting = true;
                        Tending::Edit { message, text }
                    }
                    Some(_) if open.pacing => Tending::Nothing,
                    Some(_) => {
                        open.pacing = true;
                        Tending::Pace
                    }
                }
            }
        };
        match tending {
            Tending::Nothing => {}
            Tending::Post(text) => self.transcribed(&speaking, &place, &text, speaker, run),
            Tending::Edit { message, text } => {
                self.grow_message(&speaking, &place, &message, &text, speaker, run);
            }
            Tending::Pace => {
                let id = self.effect_id();
                self.timers.insert(id, Timer::Growing { speaker, run });
                self.immediate.push(Effect::Wake { id, after: PACE });
            }
        }
    }

    /// Lets go of what a turn is done with, and of finished messages sent
    /// as they last read.
    ///
    /// Skipped by mutation testing for the reason the transcript's own is:
    /// what it frees, nothing asks about again.
    #[mutants::skip]
    fn prune(&mut self, speaker: Speaker) {
        if let Some(turn) = self.turns.get_mut(&speaker) {
            turn.transcript.prune();
        }
        self.finishing
            .retain(|(whose, _), open| *whose != speaker || !open.settled());
    }
}

#[cfg(test)]
mod tests {
    use super::{Body, Entry, Open, Standing, Transcript};
    use stageman_agent::{ToolCallStatus, ToolKind};

    fn call(status: Option<ToolCallStatus>, interrupted: bool) -> Entry {
        Entry::Call {
            id: "call-1".to_owned(),
            kind: ToolKind::Execute,
            title: "cargo test".to_owned(),
            status,
            interrupted,
        }
    }

    /// A call is running until the adapter says it ended, or a message
    /// interrupted it; a thought and narration never are.
    #[test]
    fn a_call_runs_until_it_ended_or_was_interrupted() {
        assert!(call(None, false).is_running());
        assert!(call(Some(ToolCallStatus::Pending), false).is_running());
        assert!(call(Some(ToolCallStatus::InProgress), false).is_running());
        assert!(!call(Some(ToolCallStatus::Completed), false).is_running());
        assert!(!call(Some(ToolCallStatus::Failed), false).is_running());
        assert!(
            !call(None, true).is_running(),
            "interrupted: no ending will come"
        );
        assert!(!Entry::Thought("hm".to_owned()).is_running());

        assert!(!Body::Narration("text".to_owned()).has_a_call_running());
        assert!(!Body::Working(vec![Entry::Thought("hm".to_owned())]).has_a_call_running());
        assert!(
            Body::Working(vec![
                call(Some(ToolCallStatus::Completed), false),
                call(None, false)
            ])
            .has_a_call_running()
        );
        assert!(
            !Body::Working(vec![call(Some(ToolCallStatus::Completed), false)]).has_a_call_running()
        );
    }

    /// An interrupted call reads as such, and a completed one beside it as
    /// it did.
    #[test]
    fn an_interrupted_call_reads_as_interrupted() {
        let body = Body::Working(vec![
            call(Some(ToolCallStatus::Completed), false),
            call(None, true),
        ]);
        assert_eq!(
            body.text(),
            "✅ ran `cargo test`\n⏹️ ran `cargo test` — interrupted"
        );
    }

    /// A message of the transcript, at some point in its life.
    fn message(
        body: Body,
        sent: &str,
        message: Option<&str>,
        awaiting: bool,
        pacing: bool,
        standing: Standing,
    ) -> Open {
        Open {
            run: 1,
            body,
            sent: sent.to_owned(),
            message: message.map(str::to_owned),
            awaiting,
            pacing,
            standing,
        }
    }

    fn narration(text: &str) -> Body {
        Body::Narration(text.to_owned())
    }

    /// A message is settled when it reads as sent with nothing in flight,
    /// or was let go of; and done when it is also closed and no call in it
    /// is still to end.
    #[test]
    fn a_message_is_settled_and_done_exactly_when() {
        let as_sent = message(
            narration("hi"),
            "hi",
            Some("m"),
            false,
            false,
            Standing::Growing,
        );
        assert!(as_sent.settled());
        assert!(!as_sent.lost());
        assert!(!as_sent.closed());
        assert!(!as_sent.done(), "still growing");

        let differs = message(
            narration("hi"),
            "h",
            Some("m"),
            false,
            false,
            Standing::Closed,
        );
        assert!(!differs.settled(), "reads differently from what was sent");
        let awaited = message(
            narration("hi"),
            "hi",
            Some("m"),
            true,
            false,
            Standing::Closed,
        );
        assert!(!awaited.settled(), "an answer is awaited");
        let paced = message(
            narration("hi"),
            "hi",
            Some("m"),
            false,
            true,
            Standing::Closed,
        );
        assert!(!paced.settled(), "a pace is set");

        let lost = message(narration("hi"), "", None, false, false, Standing::Lost);
        assert!(lost.lost() && lost.closed() && lost.settled() && lost.done());
        let lost_awaited = message(narration("hi"), "", None, true, false, Standing::Lost);
        assert!(
            !lost_awaited.settled(),
            "even let go of, an answer is awaited"
        );

        let closed = message(
            narration("hi"),
            "hi",
            Some("m"),
            false,
            false,
            Standing::Closed,
        );
        assert!(closed.closed() && !closed.lost() && closed.done());

        let running = message(
            Body::Working(vec![call(None, false)]),
            "⏳ ran `cargo test`",
            Some("m"),
            false,
            false,
            Standing::Closed,
        );
        assert!(running.settled() && !running.done(), "a call still to end");
        let ended = message(
            Body::Working(vec![call(Some(ToolCallStatus::Completed), false)]),
            "✅ ran `cargo test`",
            Some("m"),
            false,
            false,
            Standing::Closed,
        );
        assert!(ended.done());
    }

    /// What each piece of a turn does to its messages: narration onto the
    /// narration it is in or a new message, working onto the burst it is in
    /// or a new one, a call's change onto the burst holding it wherever it
    /// is; and the transcript knows which messages it holds.
    #[test]
    fn each_piece_says_what_it_closed_opened_or_grew() {
        let mut transcript = Transcript::new();
        assert!(!transcript.has(0));

        let first = transcript.said("Running ".to_owned());
        assert_eq!(
            (first.closed, first.opened, first.grew),
            (None, Some(0), None)
        );
        assert!(transcript.has(0) && !transcript.has(1));

        let more = transcript.said("the tests.".to_owned());
        assert_eq!((more.closed, more.opened, more.grew), (None, None, Some(0)));

        let called = transcript.worked(call(None, false));
        assert_eq!(
            (called.closed, called.opened, called.grew),
            (Some(0), Some(1), None),
            "a call closes the narration and opens a burst"
        );
        let thought = transcript.worked(Entry::Thought("hm".to_owned()));
        assert_eq!(
            (thought.closed, thought.opened, thought.grew),
            (None, None, Some(1))
        );

        let said = transcript.said("Red.".to_owned());
        assert_eq!(
            (said.closed, said.opened, said.grew),
            (Some(1), Some(2), None)
        );
        assert!(transcript.has(1), "closing, not gone");

        let ended = transcript.call_changed("call-1", None, Some(ToolCallStatus::Completed));
        assert_eq!(
            (ended.closed, ended.opened, ended.grew),
            (None, None, Some(1)),
            "the burst holding the call grew, though closed"
        );
        let unknown = transcript.call_changed("call-9", None, Some(ToolCallStatus::Completed));
        assert_eq!(
            (unknown.closed, unknown.opened, unknown.grew),
            (None, None, None)
        );
    }

    /// A message past the platform's limit is closed and continued: a
    /// narration cut where the channel cuts it, a burst at its last entry
    /// when it has more than one, and a burst of one entry left as it is.
    #[test]
    fn a_message_past_the_limit_is_closed_and_continued() {
        let at_five = |text: &str| {
            if text.chars().count() > 5 {
                let (head, tail) = text.split_at(5);
                vec![head.to_owned(), tail.to_owned()]
            } else {
                vec![text.to_owned()]
            }
        };
        let mut transcript = Transcript::new();
        let _opened = transcript.said("hello world".to_owned());
        let cut = transcript.fit(at_five);
        assert_eq!((cut.closed, cut.opened, cut.grew), (Some(0), Some(1), None));
        assert_eq!(transcript.closing[0].body.text(), "hello");
        assert_eq!(
            transcript.open.as_ref().map(|open| open.body.text()),
            Some(" world".to_owned())
        );

        let mut burst = Transcript::new();
        let _opened = burst.worked(call(None, false));
        let alone = burst.fit(|_| vec![String::new(), String::new()]);
        assert_eq!(
            (alone.closed, alone.opened, alone.grew),
            (None, None, None),
            "one entry cannot be split"
        );
        let _grew = burst.worked(Entry::Thought("hm".to_owned()));
        let split = burst.fit(|_| vec![String::new(), String::new()]);
        assert_eq!(
            (split.closed, split.opened, split.grew),
            (Some(0), Some(1), None)
        );
        assert_eq!(
            burst.open.as_ref().map(|open| open.body.text()),
            Some("> 💭 hm".to_owned()),
            "the last entry continues in the next message"
        );
        assert_eq!(burst.closing[0].body.text(), "⏳ ran `cargo test`");
    }
}
