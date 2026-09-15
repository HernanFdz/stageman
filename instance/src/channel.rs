//! Speaking on a project's channel: what a request to a platform was sent
//! for, and what its answer means.
//!
//! The daemon speaks for itself — the message a job's thread hangs from, a
//! notice that an agent stopped, a refusal said back — and on an agent's
//! behalf, when a tool call asks it to. Every one is one request the world
//! makes, rendered by the channel crate, and answered as one event routed
//! back here by the identifier it carries. Nothing here knows a platform's
//! shape: what is sent and what an answer means are `stageman_channel`'s,
//! and this is only the asking and what follows from the answer.

use std::time::Duration;

use stageman_core::{Channel, JobId, ProjectId, Speaking, Thread};
use stageman_vocabulary::{Bytes, Effect as Generic, EffectId, RequestId, Responded};

use crate::{Effect, Running};

/// How long a channel is given to answer a request before the request is
/// taken to have failed.
///
/// A budget for the platform rather than for the network: the question is
/// how long an answer can be worth waiting for. A thread opening waits on
/// it before a job can start, and a tool call is held open on it, so an
/// answer that never comes would otherwise hold both for ever — which is
/// what no budget at all meant.
pub const ANSWERS_WITHIN: Duration = Duration::from_secs(30);

/// What a request to a channel was sent for, and therefore what its answer
/// means.
///
/// Held and never kept: an answer arriving after this process dies is
/// answered to nobody, and whatever was waiting on it is decided afresh by
/// the next start.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sent {
    /// Which channel it went to, which is what reads the answer.
    pub channel: Channel,
    /// What for.
    pub purpose: Purpose,
}

/// Why a request to a channel was made.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Purpose {
    /// A message posted.
    Post(Post),
    /// A question a listener asks before it can read anything.
    Question(Question),
}

/// Why a message was posted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Post {
    /// A notice on the instance's own behalf. Its failure is said and
    /// changes nothing: it is a notice about an outcome, and the outcome
    /// does not change because the notice of it did not arrive.
    Notice,
    /// The message a job's thread hangs from, posted before its container
    /// exists.
    Opening {
        /// Whose thread.
        job: JobId,
        /// The room it was posted in, which the thread is then in.
        room: String,
    },
    /// A message on an agent's behalf, with the tool call that asked for it
    /// held open until the platform answers.
    Saying {
        /// Which call, as the world holds it open.
        request: RequestId,
    },
}

/// What a listener asked, on its way to a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Question {
    /// Who this instance is on the channel.
    Introducing {
        /// Whose channel.
        project: ProjectId,
    },
    /// Where to connect for the event stream.
    Locating {
        /// Whose channel.
        project: ProjectId,
    },
}

/// One request to a channel, as the world makes it, with the budget every
/// one of them is given.
pub fn request(id: EffectId, rendered: stageman_channel::Request) -> Effect {
    Generic::Request {
        id,
        method: rendered.method,
        url: rendered.url,
        headers: rendered.headers,
        body: rendered.body.map(Bytes::new),
        within: ANSWERS_WITHIN,
    }
}

impl Running {
    /// Asks the world to post one message, remembering what for.
    fn spoken(
        &mut self,
        channel: Channel,
        speaking: &Speaking,
        room: &str,
        text: &str,
        thread: Option<&str>,
        post: Post,
    ) -> Effect {
        let rendered = stageman_channel::post(channel, speaking, room, text, thread);
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Post(post),
            },
        );
        request(id, rendered)
    }

    /// Says something in a thread on the instance's own behalf, once
    /// whatever this step changed is on the disk.
    pub fn say(&mut self, speaking: &Speaking, thread: &Thread, text: &str) {
        let request = self.spoken(
            thread.channel,
            speaking,
            &thread.room,
            text,
            Some(&thread.id),
            Post::Notice,
        );
        self.defer(request);
    }

    /// Opens the thread a job's conversation happens in, by posting its
    /// announcement at the root of the project's home room, once the job's
    /// record is on the disk.
    pub fn open_thread(
        &mut self,
        job: JobId,
        channel: Channel,
        speaking: &Speaking,
        room: &str,
        announcement: &str,
    ) {
        let request = self.spoken(
            channel,
            speaking,
            room,
            announcement,
            None,
            Post::Opening {
                job,
                room: room.to_owned(),
            },
        );
        self.defer(request);
    }

    /// Posts on an agent's behalf, with the tool call held open until the
    /// platform answers.
    pub fn post_for(
        &mut self,
        request: RequestId,
        speaking: &Speaking,
        thread: &Thread,
        text: &str,
        effects: &mut Vec<Effect>,
    ) {
        let posting = self.spoken(
            thread.channel,
            speaking,
            &thread.room,
            text,
            Some(&thread.id),
            Post::Saying { request },
        );
        effects.push(posting);
    }

    /// What the world said about a request to a channel.
    ///
    /// A refusal arrives as an answer and is read as one; a request that
    /// never became an answer is a channel that could not be reached. Both
    /// are one string by the time they reach a record or an agent, since
    /// what either can do with the reason is show it to a person.
    pub fn responded(
        &mut self,
        id: EffectId,
        responded: &Responded,
        at: u64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(sent) = self.sent.remove(&id) else {
            tracing::warn!("a request was answered that this instance did not make; ignored");
            return;
        };
        match sent.purpose {
            Purpose::Post(post) => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::posted(sent.channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => {
                        Err(format!("the channel could not be reached: {why}"))
                    }
                };
                self.answered(sent.channel, &post, outcome);
            }
            Purpose::Question(question) => {
                self.questioned(sent.channel, question, responded, at, effects);
            }
        }
    }

    /// What follows from a channel's answer to a post, given why it was
    /// posted.
    fn answered(&mut self, channel: Channel, post: &Post, outcome: Result<String, String>) {
        match post {
            Post::Notice => {
                if let Err(why) = outcome {
                    tracing::warn!(%why, "the thread could not be spoken to");
                }
            }
            Post::Opening { job, room } => {
                self.thread_opened(
                    *job,
                    outcome.map(|id| Thread {
                        channel,
                        room: room.clone(),
                        id,
                    }),
                );
            }
            Post::Saying { request } => self.posted(*request, outcome.map(|_| ())),
        }
    }

    /// A request that waited on a write that never landed, and so was never
    /// made.
    ///
    /// Whatever waited on its answer is answered as if the channel could not
    /// be reached, which from where it stands is the truth: a notice is
    /// simply not said, a job whose thread was never opened is recorded as
    /// failed rather than left working with nowhere to speak, a call held
    /// open is answered rather than held for ever, and a listener's question
    /// is asked again later.
    pub fn unsent(&mut self, id: EffectId, effects: &mut Vec<Effect>) {
        let Some(sent) = self.sent.remove(&id) else {
            return;
        };
        match sent.purpose {
            Purpose::Post(post) => self.answered(
                sent.channel,
                &post,
                Err("the record it waited on could not be written".to_owned()),
            ),
            Purpose::Question(
                Question::Introducing { project } | Question::Locating { project },
            ) => self.unasked(project, effects),
        }
    }
}
