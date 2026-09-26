//! What a turn is shown of the thread its message was said in — see
//! `docs/decisions/0068-a-mention-is-shown-its-thread.md`.
//!
//! A mention in a thread is shown that thread before its turn starts: one
//! request, made once the record that the message is in hand has landed and
//! answered before the turn begins, the way a room is made before a job
//! starts. What is shown is the parent and everything said from the last
//! message there that was given to this instance, derived from the fetched
//! thread by the markup a mention carries and the bot identifier; the whole
//! thread when the session is fresh; never what followed the message in
//! hand; and nothing for a root mention, which fetches nothing. A thread
//! that cannot be read does not stop the turn: the frame says so instead.
//!
//! Held and never kept: a fetch in flight when this process dies is asked
//! again by the next start for a foreman, which finds the message still in
//! hand; a job's messages in hand are said again without one, per
//! `docs/decisions/0069-a-message-reaches-a-working-job.md`.

use std::collections::BTreeSet;

use stageman_channel::{Message, ThreadRead};
use stageman_core::{Channel, JobId, ProjectId};
use stageman_foreman::{Finding, Shown, Voice};

use crate::Running;
use crate::vocabulary::Speaker;

/// How many of a thread's most recent replies are asked for: a bound
/// rather than a design, as the record says.
pub const THREAD_AT_MOST: usize = 50;

/// What waits on a thread being read: the turn to start once it is.
#[derive(Clone, PartialEq, Eq)]
pub enum Pending {
    /// A job's message in hand, to start its turn once the thread is read.
    Job {
        /// Whose turn.
        job: JobId,
        /// How the message found the job.
        finding: Finding,
    },
    /// A job's message in hand, to be handed to its running turn once the
    /// thread is read — see
    /// `docs/decisions/0069-a-message-reaches-a-working-job.md`.
    Steer {
        /// Whose turn.
        job: JobId,
    },
    /// A foreman's message, in hand in its inbox.
    Foreman {
        /// Whose foreman.
        project: ProjectId,
    },
}

/// A thread as read, kept until the turn it is for composes its frame.
#[derive(Clone, PartialEq, Eq)]
pub struct Read {
    /// Its messages, oldest first, the parent included.
    pub messages: Vec<Message>,
    /// Which of them mention this instance, by identifier.
    pub mentioning: BTreeSet<String>,
    /// Whether older replies were left out.
    pub longer: bool,
    /// Whether it could not be read at all.
    pub failed: bool,
}

/// What a turn is shown of its thread, composed from what was read: that
/// it could not be read, when it could not; else the parent and everything
/// from the last message there that was given to this instance — or
/// everything, when the session is fresh and remembers none of it, or when
/// no such message was read — and never the message being handled or what
/// followed it. Nothing when nothing is left.
pub fn thread_context(
    read: &Read,
    channel: Channel,
    whole: bool,
    handling: &str,
) -> Option<String> {
    if read.failed {
        return Some(stageman_foreman::thread_unread().to_owned());
    }
    // Only what came before the message in hand. One that waited before its
    // turn may have been followed by others, and those are shown with the
    // next message given: a later mention shown here would be acted on
    // twice.
    let before: Vec<&Message> = read
        .messages
        .iter()
        .take_while(|message| message.id != handling)
        .collect();
    // What is given to this instance: a person's mention of it, and another
    // app's post when the message in hand is one, which is a signal's
    // thread.
    let signal = read
        .messages
        .iter()
        .any(|message| message.id == handling && message.app.is_some());
    let given = |message: &Message| {
        read.mentioning.contains(&message.id) || (signal && message.app.is_some())
    };
    // Where the agent was last brought in: the last message given to it
    // that a post of this instance's follows, which is how a thread says a
    // turn was taken on it. What was said while that turn ran lies after
    // it, so everything from it on is shown, and the parent before it so
    // that the rest reads as a thread rather than as fragments.
    let brought_in = if whole {
        None
    } else {
        before
            .iter()
            .rposition(|message| message.from_us)
            .and_then(|ours| before.iter().take(ours).rposition(|message| given(message)))
    };
    let selected: Vec<&Message> = before
        .iter()
        .enumerate()
        .filter(|(at, _)| brought_in.is_none_or(|from| *at == 0 || *at >= from))
        .map(|(_, message)| *message)
        .collect();
    if selected.is_empty() {
        return None;
    }
    let voices: Vec<String> = selected
        .iter()
        .map(|message| {
            message.user.as_deref().map_or_else(
                || "somebody".to_owned(),
                |user| stageman_channel::mention(channel, user),
            )
        })
        .collect();
    let ids: Vec<String> = selected
        .iter()
        .map(|message| stageman_channel::reference(channel, &message.room, &message.id))
        .collect();
    let shown: Vec<Shown<'_>> = selected
        .iter()
        .zip(voices.iter())
        .zip(ids.iter())
        .map(|((message, voice), id)| Shown {
            id,
            voice: if message.from_us {
                Voice::Us
            } else if let Some(app) = message.app.as_deref() {
                Voice::App(app)
            } else {
                Voice::Person(voice)
            },
            text: &message.text,
        })
        .collect();
    Some(stageman_foreman::thread_shown(
        &shown,
        brought_in.is_some(),
        read.longer,
    ))
}

impl Running {
    /// Asks the platform for the thread a speaker's message was said in,
    /// remembering what waits on the answer, and says whether it asked: a
    /// project with no channel has nothing to ask, and the turn must not be
    /// left waiting on an answer that will never come.
    pub fn read_thread(
        &mut self,
        speaker: Speaker,
        pending: Pending,
        room: &str,
        thread: &str,
    ) -> bool {
        let Some(project) = self.project_of_speaker(&speaker) else {
            return false;
        };
        let Some((channel, speaking)) = self
            .state
            .projects
            .get(&project)
            .and_then(|watched| watched.channels.keys().next().copied())
            .and_then(|channel| Some((channel, self.state.speaking(project, channel)?)))
        else {
            return false;
        };
        self.pending_threads.insert(speaker.clone(), pending);
        self.ask_for_thread(speaker, channel, &speaking, room, thread);
        true
    }

    /// The platform answered about a thread, or could not: what was read is
    /// kept for the turn's frame, and the turn that waited starts.
    pub fn thread_read_back(&mut self, speaker: &Speaker, outcome: Result<ThreadRead, String>) {
        let read = match outcome {
            Ok(ThreadRead {
                messages,
                mentioning,
                longer,
            }) => Read {
                messages,
                mentioning,
                longer,
                failed: false,
            },
            Err(why) => {
                tracing::warn!(%why, "the thread a message was said in could not be read");
                Read {
                    messages: Vec::new(),
                    mentioning: BTreeSet::new(),
                    longer: false,
                    failed: true,
                }
            }
        };
        self.threads_read.insert(speaker.clone(), read);
        match self.pending_threads.remove(speaker) {
            Some(Pending::Job { job, finding }) => {
                if let Some((project, channel)) = self.bound(&job) {
                    self.resume_job(project, &job, channel, finding);
                }
            }
            Some(Pending::Steer { job }) => {
                if let Some((project, channel)) = self.bound(&job) {
                    self.steer_given(project, &job, channel);
                }
            }
            Some(Pending::Foreman { project }) => self.inspect_before_turning(project),
            None => tracing::debug!("a thread was read for nobody; ignored"),
        }
    }

    /// The record that a message is in hand was never written, so the
    /// thread it was in is not asked for and the turn does not start: a
    /// job's is failed as a turn never started is, a message that was to
    /// be handed to a running turn waits again for that turn's end, and a
    /// foreman's message stays in hand for the next start to find.
    pub fn thread_unasked(&mut self, speaker: Speaker) {
        match self.pending_threads.remove(&speaker) {
            Some(Pending::Job { .. }) => self.abandoned(speaker),
            Some(Pending::Steer { job }) => {
                if let Some(turn) = self.turns.get_mut(&speaker) {
                    turn.delivery = crate::turns::Delivery::Open;
                }
                if let Some(recorded) = self.state.job_mut(&job) {
                    recorded.inbox.hand_back();
                    self.dirty = true;
                }
            }
            Some(Pending::Foreman { .. }) => {
                tracing::warn!(
                    "the foreman's turn is not started, since its message is not on the disk"
                );
            }
            None => {}
        }
    }

    /// What was read for a speaker, taken for the turn's frame.
    pub fn thread_taken(&mut self, speaker: &Speaker) -> Option<Read> {
        self.threads_read.remove(speaker)
    }

    /// Which project a speaker belongs to.
    fn project_of_speaker(&self, speaker: &Speaker) -> Option<ProjectId> {
        match speaker {
            Speaker::Foreman(project) => Some(*project),
            Speaker::Job(job) => self.state.project_of(job),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Read, thread_context};
    use stageman_channel::Message;
    use stageman_core::Channel;
    use stageman_foreman::{Shown, Voice, thread_shown, thread_unread};

    const ROOM: &str = "C0123";

    /// How this instance is mentioned in these threads.
    const US: &str = "<@U0BOT>";

    fn person(id: &str, text: &str) -> Message {
        Message {
            room: ROOM.to_owned(),
            id: id.to_owned(),
            thread: Some("100".to_owned()),
            text: text.to_owned(),
            user: Some("U0HUMAN".to_owned()),
            from_us: false,
            app: None,
        }
    }

    fn ours(id: &str, text: &str) -> Message {
        Message {
            user: Some("U0BOT".to_owned()),
            from_us: true,
            ..person(id, text)
        }
    }

    fn app(id: &str, text: &str) -> Message {
        Message {
            user: None,
            app: Some("GitHub".to_owned()),
            ..person(id, text)
        }
    }

    /// A thread as read. Which messages mention this instance is read off
    /// their text here, as the channel crate reads it off the platform's.
    fn read(messages: Vec<Message>) -> Read {
        let mentioning = messages
            .iter()
            .filter(|message| {
                !message.from_us && message.app.is_none() && message.text.contains(US)
            })
            .map(|message| message.id.clone())
            .collect();
        Read {
            messages,
            mentioning,
            longer: false,
            failed: false,
        }
    }

    fn shown<'a>(id: &'a str, voice: Voice<'a>, text: &'a str) -> Shown<'a> {
        Shown { id, voice, text }
    }

    fn human(id: &'static str, text: &'static str) -> Shown<'static> {
        shown(id, Voice::Person("<@U0HUMAN>"), text)
    }

    /// Two mentions, each answered, with something said without a mention
    /// while each turn ran and after it; then the mention in hand, and one
    /// that followed it while it waited.
    fn a_thread() -> Vec<Message> {
        vec![
            person("100", "<@U0BOT> which database?"),
            person("150", "Staging is down, by the way."),
            ours("200", "Two options."),
            person("300", "Postgres?"),
            person("350", "<@U0BOT> is Postgres fine?"),
            person("375", "It was fine last year."),
            ours("400", "Noted."),
            person("500", "Or SQLite."),
            person("600", "<@U0BOT> decide"),
            person("700", "<@U0BOT> and hurry"),
        ]
    }

    /// A thread that could not be read says so, and shows nothing else.
    #[test]
    fn a_thread_that_failed_says_it_could_not_be_read() {
        let failed = Read {
            longer: true,
            failed: true,
            ..read(a_thread())
        };
        assert_eq!(
            thread_context(&failed, Channel::Slack, false, "600").as_deref(),
            Some(thread_unread())
        );
    }

    /// A session that remembers is shown the parent, and everything from
    /// the last message it was given and answered: that message, what was
    /// said while its turn ran, its own answer, and what followed. Not what
    /// came before, which an earlier turn was shown; not the message in
    /// hand; and not what followed that.
    #[test]
    fn a_session_that_remembers_is_shown_from_the_last_message_it_was_given() {
        assert_eq!(
            thread_context(&read(a_thread()), Channel::Slack, false, "600"),
            Some(thread_shown(
                &[
                    human("C0123/100", "<@U0BOT> which database?"),
                    human("C0123/350", "<@U0BOT> is Postgres fine?"),
                    human("C0123/375", "It was fine last year."),
                    shown("C0123/400", Voice::Us, "Noted."),
                    human("C0123/500", "Or SQLite."),
                ],
                true,
                false,
            ))
        );
    }

    /// What was said without a mention while a turn ran is shown at the
    /// next mention, though this instance posted after it: the case a line
    /// drawn at this instance's last post never showed anybody.
    #[test]
    fn what_was_said_while_a_turn_ran_is_shown_at_the_next_mention() {
        let thread = vec![
            person("100", "<@U0BOT> which database?"),
            person("150", "Staging is down, by the way."),
            ours("200", "Two options."),
            person("300", "<@U0BOT> given that, which?"),
        ];
        assert_eq!(
            thread_context(&read(thread), Channel::Slack, false, "300"),
            Some(thread_shown(
                &[
                    human("C0123/100", "<@U0BOT> which database?"),
                    human("C0123/150", "Staging is down, by the way."),
                    shown("C0123/200", Voice::Us, "Two options."),
                ],
                true,
                false,
            ))
        );
    }

    /// A mention nobody answered does not move the line: it is shown again
    /// with what surrounds it, since no turn may ever have been taken on it.
    #[test]
    fn a_mention_nobody_answered_is_shown_again() {
        let thread = vec![
            person("100", "<@U0BOT> which database?"),
            ours("200", "Two options."),
            person("300", "<@U0BOT> still there?"),
            person("400", "It does not seem to be."),
            person("500", "<@U0BOT> hello?"),
        ];
        assert_eq!(
            thread_context(&read(thread), Channel::Slack, false, "500"),
            Some(thread_shown(
                &[
                    human("C0123/100", "<@U0BOT> which database?"),
                    shown("C0123/200", Voice::Us, "Two options."),
                    human("C0123/300", "<@U0BOT> still there?"),
                    human("C0123/400", "It does not seem to be."),
                ],
                true,
                false,
            ))
        );
    }

    /// A thread under this instance's own words, where nothing was given to
    /// it before, is shown whole and not as "from the last message given".
    #[test]
    fn a_thread_under_this_instances_own_words_is_shown_whole() {
        let thread = vec![
            ours("100", "Reading the parser."),
            person("200", "Why the parser?"),
            person("300", "<@U0BOT> why?"),
        ];
        assert_eq!(
            thread_context(&read(thread), Channel::Slack, false, "300"),
            Some(thread_shown(
                &[
                    shown("C0123/100", Voice::Us, "Reading the parser."),
                    human("C0123/200", "Why the parser?"),
                ],
                false,
                false,
            ))
        );
    }

    /// What followed the message in hand is not shown with it, a later
    /// mention least of all: it is given in its own turn, and shown as
    /// context it would be acted on twice.
    #[test]
    fn what_followed_the_message_in_hand_is_not_shown_with_it() {
        let thread = vec![
            person("100", "Which database?"),
            person("200", "<@U0BOT> decide"),
            person("300", "Actually, wait."),
            person("400", "<@U0BOT> and hurry"),
        ];
        assert_eq!(
            thread_context(&read(thread), Channel::Slack, false, "200"),
            Some(thread_shown(
                &[human("C0123/100", "Which database?")],
                false,
                false
            ))
        );
    }

    /// A fresh session is shown everything before the message in hand, its
    /// own earlier words as its own, and is not told "from the last".
    #[test]
    fn a_fresh_session_is_shown_everything_before_the_message_in_hand() {
        assert_eq!(
            thread_context(&read(a_thread()), Channel::Slack, true, "600"),
            Some(thread_shown(
                &[
                    human("C0123/100", "<@U0BOT> which database?"),
                    human("C0123/150", "Staging is down, by the way."),
                    shown("C0123/200", Voice::Us, "Two options."),
                    human("C0123/300", "Postgres?"),
                    human("C0123/350", "<@U0BOT> is Postgres fine?"),
                    human("C0123/375", "It was fine last year."),
                    shown("C0123/400", Voice::Us, "Noted."),
                    human("C0123/500", "Or SQLite."),
                ],
                false,
                false,
            ))
        );
    }

    /// A signal's thread draws the line at the app's last post that this
    /// instance answered, since another app's post is what is given there;
    /// a person's mention in the same thread is given mentions only, finds
    /// none answered, and is shown everything.
    #[test]
    fn a_signals_thread_counts_the_apps_posts_as_given() {
        let thread = vec![
            app("100", "Issue opened"),
            app("200", "Issue closed"),
            ours("250", "Filed as a job."),
            person("260", "Thanks."),
            app("300", "Issue reopened"),
        ];
        assert_eq!(
            thread_context(&read(thread.clone()), Channel::Slack, false, "300"),
            Some(thread_shown(
                &[
                    shown("C0123/100", Voice::App("GitHub"), "Issue opened"),
                    shown("C0123/200", Voice::App("GitHub"), "Issue closed"),
                    shown("C0123/250", Voice::Us, "Filed as a job."),
                    human("C0123/260", "Thanks."),
                ],
                true,
                false,
            ))
        );

        let mut mentioned = thread;
        mentioned.truncate(4);
        mentioned.push(person("300", "<@U0BOT> why was this filed?"));
        assert_eq!(
            thread_context(&read(mentioned), Channel::Slack, false, "300"),
            Some(thread_shown(
                &[
                    shown("C0123/100", Voice::App("GitHub"), "Issue opened"),
                    shown("C0123/200", Voice::App("GitHub"), "Issue closed"),
                    shown("C0123/250", Voice::Us, "Filed as a job."),
                    human("C0123/260", "Thanks."),
                ],
                false,
                false,
            ))
        );
    }

    /// A thread this instance never spoke in is shown whole even to a
    /// session that remembers: an app by its name, somebody the platform
    /// did not name as somebody, and that the thread was longer when it
    /// was.
    #[test]
    fn a_thread_this_instance_never_spoke_in_is_shown_whole() {
        let thread = Read {
            longer: true,
            ..read(vec![
                app("100", "Issue opened"),
                Message {
                    user: None,
                    ..person("200", "Looking.")
                },
                person("300", "<@U0BOT> look at this"),
            ])
        };
        assert_eq!(
            thread_context(&thread, Channel::Slack, false, "300"),
            Some(thread_shown(
                &[
                    shown("C0123/100", Voice::App("GitHub"), "Issue opened"),
                    shown("C0123/200", Voice::Person("somebody"), "Looking."),
                ],
                false,
                true,
            ))
        );
    }

    /// A thread holding nothing but the message in hand shows nothing.
    #[test]
    fn a_thread_holding_only_the_message_in_hand_shows_nothing() {
        let thread = vec![person("300", "<@U0BOT> look at this")];
        assert_eq!(
            thread_context(&read(thread), Channel::Slack, false, "300"),
            None
        );
    }
}
