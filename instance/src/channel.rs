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

use stageman_core::{Channel, JobId, Speaking, Thread};
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
    /// A notice on the instance's own behalf. Its failure is said and
    /// changes nothing: it is a notice about an outcome, and the outcome
    /// does not change because the notice of it did not arrive.
    Notice,
    /// The message a job's thread hangs from, posted before its container
    /// exists.
    Opening {
        /// Whose thread.
        job: JobId,
    },
    /// A message on an agent's behalf, with the tool call that asked for it
    /// held open until the platform answers.
    Saying {
        /// Which call, as the world holds it open.
        request: RequestId,
    },
}

impl Running {
    /// Asks the world to post one message, remembering what for.
    fn spoken(
        &mut self,
        channel: Channel,
        speaking: &Speaking,
        text: &str,
        thread: Option<&str>,
        purpose: Purpose,
    ) -> Effect {
        let rendered = stageman_channel::post(channel, speaking, text, thread);
        let id = self.effect_id();
        self.sent.insert(id, Sent { channel, purpose });
        Generic::Request {
            id,
            method: rendered.method,
            url: rendered.url,
            headers: rendered.headers,
            body: rendered.body.map(Bytes::new),
            within: ANSWERS_WITHIN,
        }
    }

    /// Says something in a thread on the instance's own behalf, once
    /// whatever this step changed is on the disk.
    pub fn say(&mut self, speaking: &Speaking, thread: &Thread, text: &str) {
        let request = self.spoken(
            thread.channel,
            speaking,
            text,
            Some(&thread.id),
            Purpose::Notice,
        );
        self.defer(request);
    }

    /// Says something in a thread on the instance's own behalf, without
    /// waiting for anything: a notice that changes no state.
    pub fn say_now(
        &mut self,
        speaking: &Speaking,
        thread: &Thread,
        text: &str,
        effects: &mut Vec<Effect>,
    ) {
        let request = self.spoken(
            thread.channel,
            speaking,
            text,
            Some(&thread.id),
            Purpose::Notice,
        );
        effects.push(request);
    }

    /// Opens the thread a job's conversation happens in, by posting its
    /// announcement at the root of the channel, once the job's record is on
    /// the disk.
    pub fn open_thread(
        &mut self,
        job: JobId,
        channel: Channel,
        speaking: &Speaking,
        announcement: &str,
    ) {
        let request = self.spoken(
            channel,
            speaking,
            announcement,
            None,
            Purpose::Opening { job },
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
            text,
            Some(&thread.id),
            Purpose::Saying { request },
        );
        effects.push(posting);
    }

    /// What the world said about a request to a channel.
    ///
    /// A refusal arrives as an answer and is read as one; a request that
    /// never became an answer is a channel that could not be reached. Both
    /// are one string by the time they reach a record or an agent, since
    /// what either can do with the reason is show it to a person.
    pub fn responded(&mut self, id: EffectId, responded: &Responded) {
        let Some(sent) = self.sent.remove(&id) else {
            tracing::warn!("a request was answered that this instance did not make; ignored");
            return;
        };
        let outcome = match responded {
            Responded::Answered { status, body, .. } => {
                stageman_channel::posted(sent.channel, *status, body.as_slice())
                    .map_err(|why| why.to_string())
            }
            Responded::Failed(why) => Err(format!("the channel could not be reached: {why}")),
        };
        self.answered(&sent, outcome);
    }

    /// What follows from a channel's answer, given what was asked.
    fn answered(&mut self, sent: &Sent, outcome: Result<String, String>) {
        match sent.purpose {
            Purpose::Notice => {
                if let Err(why) = outcome {
                    tracing::warn!(%why, "the thread could not be spoken to");
                }
            }
            Purpose::Opening { job } => self.thread_opened(
                job,
                outcome.map(|id| Thread {
                    channel: sent.channel,
                    id,
                }),
            ),
            Purpose::Saying { request } => self.posted(request, outcome.map(|_| ())),
        }
    }

    /// A request that waited on a write that never landed, and so was never
    /// made.
    ///
    /// Whatever waited on its answer is answered as if the channel could not
    /// be reached, which from where it stands is the truth: a notice is
    /// simply not said, a job whose thread was never opened is recorded as
    /// failed rather than left working with nowhere to speak, and a call
    /// held open is answered rather than held for ever.
    pub fn unsent(&mut self, id: EffectId) {
        let Some(sent) = self.sent.remove(&id) else {
            return;
        };
        self.answered(
            &sent,
            Err("the record it waited on could not be written".to_owned()),
        );
    }
}
