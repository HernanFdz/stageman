//! What a turn is shown of the thread its message was said in — see
//! `docs/decisions/0068-a-mention-is-shown-its-thread.md`.
//!
//! A mention in a thread is shown that thread before its turn starts: one
//! request, made once the record that the message is in hand has landed and
//! answered before the turn begins, the way a room is made before a job
//! starts. What is shown is the parent and everything said since this
//! instance last posted there, derived from the fetched thread by the bot
//! identifier; the whole thread when the session is fresh; and nothing for
//! a root mention, which fetches nothing. A thread that cannot be read does
//! not stop the turn: the frame says so instead.
//!
//! Held and never kept: a fetch in flight when this process dies is asked
//! again by the next start, which finds the message still in hand.

use stageman_channel::Message;
use stageman_core::{Channel, JobId, ProjectId};
use stageman_foreman::{Shown, Voice};

use crate::Running;
use crate::vocabulary::Speaker;

/// How many of a thread's most recent replies are asked for: a bound
/// rather than a design, as the record says.
pub const THREAD_AT_MOST: usize = 50;

/// What waits on a thread being read: the turn to start once it is.
#[derive(Clone, PartialEq, Eq)]
pub enum Pending {
    /// A job's reply, to be delivered once its thread is read.
    Job {
        /// Whose turn.
        job: JobId,
        /// The reply, as heard.
        message: Message,
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
    /// Whether older replies were left out.
    pub longer: bool,
    /// Whether it could not be read at all.
    pub failed: bool,
}

/// What a turn is shown of its thread, composed from what was read: that
/// it could not be read, when it could not; else the parent and what
/// followed this instance's last post there — or everything, when the
/// session is fresh and remembers none of it — with the message being
/// handled left out, since it follows. Nothing when nothing is left.
pub fn thread_context(
    read: &Read,
    channel: Channel,
    whole: bool,
    handling: &str,
) -> Option<String> {
    if read.failed {
        return Some(stageman_foreman::thread_unread().to_owned());
    }
    let before: Vec<&Message> = read
        .messages
        .iter()
        .filter(|message| message.id != handling)
        .collect();
    // Where this instance last spoke, when that is what decides: everything
    // after it is what the agent has not seen, and the parent is shown
    // again so that the rest reads as a thread rather than as fragments.
    let last_ours = if whole {
        None
    } else {
        before.iter().rposition(|message| message.from_us)
    };
    let selected: Vec<&Message> = before
        .iter()
        .enumerate()
        .filter(|(at, _)| last_ours.is_none_or(|last| *at == 0 || *at > last))
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
        last_ours.is_some(),
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
        let Some(project) = self.project_of_speaker(speaker) else {
            return false;
        };
        let Some((channel, speaking)) = self
            .state
            .projects
            .get(&project)
            .and_then(|watched| watched.channels.iter().next())
            .map(|(channel, bound)| (*channel, bound.speaking()))
        else {
            return false;
        };
        self.pending_threads.insert(speaker, pending);
        self.ask_for_thread(speaker, channel, &speaking, room, thread);
        true
    }

    /// The platform answered about a thread, or could not: what was read is
    /// kept for the turn's frame, and the turn that waited starts.
    pub fn thread_read_back(
        &mut self,
        speaker: Speaker,
        outcome: Result<(Vec<Message>, bool), String>,
    ) {
        let read = match outcome {
            Ok((messages, longer)) => Read {
                messages,
                longer,
                failed: false,
            },
            Err(why) => {
                tracing::warn!(%why, "the thread a message was said in could not be read");
                Read {
                    messages: Vec::new(),
                    longer: false,
                    failed: true,
                }
            }
        };
        self.threads_read.insert(speaker, read);
        match self.pending_threads.remove(&speaker) {
            Some(Pending::Job { job, message }) => {
                let bound = self.state.project_of(job).and_then(|project| {
                    let channel = self.state.projects.get(&project)?.channels.keys().next()?;
                    Some((project, *channel))
                });
                if let Some((project, channel)) = bound {
                    self.resume_job(project, job, channel, &message);
                }
            }
            Some(Pending::Foreman { project }) => self.inspect_before_turning(project),
            None => tracing::debug!("a thread was read for nobody; ignored"),
        }
    }

    /// The record that a message is in hand was never written, so the
    /// thread it was in is not asked for and the turn does not start: a
    /// job's is failed as a turn never started is, and a foreman's message
    /// stays in hand for the next start to find.
    pub fn thread_unasked(&mut self, speaker: Speaker) {
        match self.pending_threads.remove(&speaker) {
            Some(Pending::Job { .. }) => self.abandoned(speaker),
            Some(Pending::Foreman { .. }) => {
                tracing::warn!(
                    "the foreman's turn is not started, since its message is not on the disk"
                );
            }
            None => {}
        }
    }

    /// What was read for a speaker, taken for the turn's frame.
    pub fn thread_taken(&mut self, speaker: Speaker) -> Option<Read> {
        self.threads_read.remove(&speaker)
    }

    /// Which project a speaker belongs to.
    fn project_of_speaker(&self, speaker: Speaker) -> Option<ProjectId> {
        match speaker {
            Speaker::Foreman(project) => Some(project),
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

    fn read(messages: Vec<Message>) -> Read {
        Read {
            messages,
            longer: false,
            failed: false,
        }
    }

    fn shown<'a>(id: &'a str, voice: Voice<'a>, text: &'a str) -> Shown<'a> {
        Shown { id, voice, text }
    }

    /// A parent, this instance's answer, an untagged reply, this instance
    /// again, another untagged reply, and the mention in hand.
    fn a_thread() -> Vec<Message> {
        vec![
            person("100", "Which database?"),
            ours("200", "Two options."),
            person("300", "Postgres?"),
            ours("400", "Noted."),
            person("500", "Or SQLite."),
            person("600", "<@U0BOT> decide"),
        ]
    }

    /// A thread that could not be read says so, and shows nothing else.
    #[test]
    fn a_thread_that_failed_says_it_could_not_be_read() {
        let failed = Read {
            messages: a_thread(),
            longer: true,
            failed: true,
        };
        assert_eq!(
            thread_context(&failed, Channel::Slack, false, "600").as_deref(),
            Some(thread_unread())
        );
    }

    /// A session that remembers is shown the parent and what followed this
    /// instance's last post: not its own words, not what came before them,
    /// and not the message in hand.
    #[test]
    fn a_session_that_remembers_is_shown_the_parent_and_what_followed_its_last_post() {
        assert_eq!(
            thread_context(&read(a_thread()), Channel::Slack, false, "600"),
            Some(thread_shown(
                &[
                    shown("C0123/100", Voice::Person("<@U0HUMAN>"), "Which database?"),
                    shown("C0123/500", Voice::Person("<@U0HUMAN>"), "Or SQLite."),
                ],
                true,
                false,
            ))
        );
    }

    /// A parent this instance wrote, with nothing of its own after it, is
    /// shown once, as its own, and everything after it follows.
    #[test]
    fn a_parent_of_this_instances_own_is_shown_once_with_what_followed() {
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
                    shown("C0123/200", Voice::Person("<@U0HUMAN>"), "Why the parser?"),
                ],
                true,
                false,
            ))
        );
    }

    /// A fresh session is shown everything but the message in hand, its
    /// own earlier words as its own, and is not told "since".
    #[test]
    fn a_fresh_session_is_shown_everything_but_the_message_in_hand() {
        assert_eq!(
            thread_context(&read(a_thread()), Channel::Slack, true, "600"),
            Some(thread_shown(
                &[
                    shown("C0123/100", Voice::Person("<@U0HUMAN>"), "Which database?"),
                    shown("C0123/200", Voice::Us, "Two options."),
                    shown("C0123/300", Voice::Person("<@U0HUMAN>"), "Postgres?"),
                    shown("C0123/400", Voice::Us, "Noted."),
                    shown("C0123/500", Voice::Person("<@U0HUMAN>"), "Or SQLite."),
                ],
                false,
                false,
            ))
        );
    }

    /// A thread this instance never spoke in is shown whole even to a
    /// session that remembers, and not as "since": an app by its name,
    /// somebody the platform did not name as somebody, and that the thread
    /// was longer when it was.
    #[test]
    fn a_thread_this_instance_never_spoke_in_is_shown_whole() {
        let thread = Read {
            messages: vec![
                app("100", "Issue opened"),
                Message {
                    user: None,
                    ..person("200", "Looking.")
                },
                person("300", "<@U0BOT> look at this"),
            ],
            longer: true,
            failed: false,
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
