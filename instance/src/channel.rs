//! Speaking on a project's channel: what a request to a platform was sent
//! for, and what its answer means.
//!
//! The daemon speaks for itself — the opening of a job's room, a notice that
//! an agent stopped, a refusal said back — and on an agent's behalf, when a
//! tool call asks it to. It also keeps a job's room: makes it, describes it,
//! invites the person who asked into it, and archives it when the job is
//! over. Every one is one request the world makes, rendered by the channel
//! crate, and answered as one event routed back here by the identifier it
//! carries. Nothing here knows a platform's shape: what is sent and what an
//! answer means are `stageman_channel`'s, and this is only the asking and
//! what follows from the answer.

use std::collections::VecDeque;
use std::time::Duration;

use stageman_core::{Channel, JobId, Place, ProjectId, Room, Speaking, Thread};
use stageman_vocabulary::{Bytes, Effect as Generic, EffectId, RequestId, Responded};

use crate::vocabulary::Speaker;
use crate::{Effect, Running};

/// How long a channel is given to answer a request before the request is
/// taken to have failed.
///
/// A budget for the platform rather than for the network: the question is
/// how long an answer can be worth waiting for. A room being made waits on
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
    /// Which room, for a post: whose chain the answer moves along.
    pub room: Option<Room>,
}

/// What one room is being posted: whether a request is in flight, and what
/// waits behind it.
///
/// One request in flight per room, the next sent when the previous is
/// answered, so that posts land in the order they were made — two in flight
/// land in whichever order the platform receives them. The rule of
/// `docs/conventions.md` §3, from
/// `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.
#[derive(Default, Clone, PartialEq, serde::Serialize)]
pub struct Posting {
    /// Whether a request to this room is waiting to be answered.
    in_flight: bool,
    /// What is posted next, front first.
    waiting: VecDeque<Queued>,
}

/// One post waiting its turn in a room.
#[derive(Clone, PartialEq, serde::Serialize)]
struct Queued {
    /// The request, rendered.
    effect: Effect,
    /// Whether it waits for the writes in flight when its turn comes, as a
    /// notice about a record does; a post that changes nothing goes at once.
    after_writes: bool,
}

/// Why a request to a channel was made.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Purpose {
    /// A message posted.
    Post(Post),
    /// A job's room being made, before its container exists.
    Creating {
        /// Whose room.
        job: JobId,
        /// Where the job came from, if a person's message is what started
        /// it: told where the job is, and invited.
        origin: Option<Origin>,
    },
    /// Something done to a room whose outcome changes nothing here.
    Keeping(Keeping),
    /// A question a listener asks before it can read anything.
    Question(Question),
    /// A message of a turn's transcript, grown by editing.
    Growing {
        /// Whose turn.
        speaker: Speaker,
        /// Which of its messages.
        run: u64,
    },
}

/// Where a job came from, when a person's message is what started it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Origin {
    /// The thread the message was in, or opens: where to say where the job
    /// is.
    pub thread: Thread,
    /// Who said it, as the platform names them, when the platform said:
    /// who is invited into the room.
    pub user: Option<String>,
}

/// Why a message was posted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Post {
    /// A notice on the instance's own behalf. Its failure is said and
    /// changes nothing: it is a notice about an outcome, and the outcome
    /// does not change because the notice of it did not arrive.
    Notice,
    /// A message on an agent's behalf, with the tool call that asked for it
    /// held open until the platform answers.
    Saying {
        /// Which call, as the world holds it open.
        request: RequestId,
    },
    /// A message of a turn's transcript, opened: its answer names the
    /// message, which is what lets it grow. Its failure is said and changes
    /// nothing, like a notice's: what a person must see is what the tool's
    /// answer guarantees.
    Transcript {
        /// Whose turn.
        speaker: Speaker,
        /// Which of its messages.
        run: u64,
    },
}

/// Something done to a room that changes nothing here: its failure is said
/// in the log, and that is all.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Keeping {
    /// A room described: its purpose, or its topic.
    Describing {
        /// Whose room.
        job: JobId,
    },
    /// Somebody invited into a room.
    Inviting {
        /// Whose room.
        job: JobId,
    },
    /// A room archived, because its job is over or its project forgotten.
    Archiving {
        /// Which room.
        room: String,
    },
    /// A reaction put on somebody's message: received, or done.
    Reacting {
        /// Which room.
        room: String,
        /// Which message.
        message: String,
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
    /// Asks the world to post one message, remembering what for, in its turn
    /// behind whatever the room is already being posted.
    ///
    /// A post about a record waits for the write, as it always did; one that
    /// changes nothing goes at the step's end. Either way it goes only once
    /// the room's previous post has been answered.
    fn spoken(
        &mut self,
        speaking: &Speaking,
        place: &Place,
        text: &str,
        post: Post,
        after_writes: bool,
    ) {
        let channel = place.room.channel;
        let rendered = stageman_channel::post(
            channel,
            speaking,
            &place.room.id,
            text,
            place.thread.as_deref(),
        );
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Post(post),
                room: Some(place.room.clone()),
            },
        );
        self.enqueue(place.room.clone(), request(id, rendered), after_writes);
    }

    /// Puts a post in its room's chain: sent now if nothing is in flight
    /// there, and behind what is otherwise.
    fn enqueue(&mut self, room: Room, effect: Effect, after_writes: bool) {
        let posting = self.posting.entry(room).or_default();
        if posting.in_flight {
            posting.waiting.push_back(Queued {
                effect,
                after_writes,
            });
            return;
        }
        posting.in_flight = true;
        if after_writes {
            self.defer(effect);
        } else {
            self.immediate.push(effect);
        }
    }

    /// A room's post was answered: the next waiting one goes, or the room
    /// is idle.
    ///
    /// A post released here was justified by a step that is over, so one
    /// that waits on a write waits on every write still in flight rather
    /// than on this step's.
    fn next_post(&mut self, room: &Room) {
        let next = self
            .posting
            .get_mut(room)
            .and_then(|posting| posting.waiting.pop_front());
        match next {
            Some(Queued {
                effect,
                after_writes: true,
            }) => self.after_writes(effect),
            Some(Queued {
                effect,
                after_writes: false,
            }) => self.immediate.push(effect),
            None => {
                if let Some(posting) = self.posting.get_mut(room) {
                    posting.in_flight = false;
                }
            }
        }
    }

    /// Says something at a place on the instance's own behalf, once
    /// whatever this step changed is on the disk.
    pub fn say(&mut self, speaking: &Speaking, place: &Place, text: &str) {
        self.spoken(speaking, place, text, Post::Notice, true);
    }

    /// Opens one message of a turn's transcript at a place, at the step's
    /// end: it changes no record, so it waits for no write.
    pub fn transcribed(
        &mut self,
        speaking: &Speaking,
        place: &Place,
        text: &str,
        speaker: Speaker,
        run: u64,
    ) {
        self.spoken(
            speaking,
            place,
            text,
            Post::Transcript { speaker, run },
            false,
        );
    }

    /// Grows one message of a turn's transcript by editing it in place, at
    /// the step's end. Not in the room's chain: it names a message already
    /// there, so it cannot land out of order with a post.
    pub fn grow_message(
        &mut self,
        speaking: &Speaking,
        place: &Place,
        message: &str,
        text: &str,
        speaker: Speaker,
        run: u64,
    ) {
        let channel = place.room.channel;
        let rendered = stageman_channel::update(channel, speaking, &place.room.id, message, text);
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Growing { speaker, run },
                room: None,
            },
        );
        let effect = request(id, rendered);
        self.immediate.push(effect);
    }

    /// Makes the room a job's conversation happens in, once the job's
    /// record is on the disk.
    pub fn create_room(
        &mut self,
        job: JobId,
        channel: Channel,
        speaking: &Speaking,
        name: &str,
        origin: Option<Origin>,
    ) {
        let rendered = stageman_channel::create_room(channel, speaking, name);
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Creating { job, origin },
                room: None,
            },
        );
        let request = request(id, rendered);
        self.defer(request);
    }

    /// Asks the platform to do something to a room, once whatever this step
    /// changed is on the disk. What it answers changes nothing here, so a
    /// failure is logged and that is all.
    fn keep(&mut self, channel: Channel, rendered: stageman_channel::Request, keeping: Keeping) {
        let id = self.effect_id();
        self.sent.insert(
            id,
            Sent {
                channel,
                purpose: Purpose::Keeping(keeping),
                room: None,
            },
        );
        let request = request(id, rendered);
        self.defer(request);
    }

    /// Describes a job's room: what it is for, and where the job shows its
    /// work.
    pub fn describe_room(
        &mut self,
        job: JobId,
        channel: Channel,
        speaking: &Speaking,
        room: &str,
        purpose: &str,
        topic: &str,
    ) {
        self.keep(
            channel,
            stageman_channel::set_purpose(channel, speaking, room, purpose),
            Keeping::Describing { job },
        );
        self.keep(
            channel,
            stageman_channel::set_topic(channel, speaking, room, topic),
            Keeping::Describing { job },
        );
    }

    /// Invites the person who asked for a job into its room.
    pub fn invite_into(
        &mut self,
        job: JobId,
        channel: Channel,
        speaking: &Speaking,
        room: &str,
        user: &str,
    ) {
        self.keep(
            channel,
            stageman_channel::invite(channel, speaking, room, user),
            Keeping::Inviting { job },
        );
    }

    /// Archives a job's room, if it has one, once the record that made it
    /// over is on the disk.
    ///
    /// An archived room leaves the sidebar, stays readable, and takes no
    /// more posts — which is what makes a retired job's conversation
    /// finished on the platform as well as here.
    pub fn archive_room_of(&mut self, job: JobId) {
        let Some(project) = self.state.project_of(job) else {
            return;
        };
        let Some((channel, speaking, room)) =
            self.state.projects.get(&project).and_then(|watched| {
                let room = watched.jobs.get(&job)?.room.clone()?;
                let bound = watched.channels.get(&room.channel)?;
                Some((room.channel, bound.speaking(), room.id))
            })
        else {
            return;
        };
        self.keep(
            channel,
            stageman_channel::archive(channel, &speaking, &room),
            Keeping::Archiving { room },
        );
    }

    /// Puts a reaction on a message, once whatever this step changed is on
    /// the disk: what a foreman says instead of "got it", per
    /// `docs/decisions/0062-what-this-instance-says-is-markdown.md`.
    pub fn react(
        &mut self,
        channel: Channel,
        speaking: &Speaking,
        room: &str,
        message: &str,
        reaction: stageman_channel::Reaction,
    ) {
        self.keep(
            channel,
            stageman_channel::react(channel, speaking, room, message, reaction),
            Keeping::Reacting {
                room: room.to_owned(),
                message: message.to_owned(),
            },
        );
    }

    /// How this instance is mentioned on a project's channel: its own
    /// identity there, once its listener has been told who it is, and the
    /// name the setup instructions give the app until then.
    #[must_use]
    pub fn own_mention(&self, project: ProjectId, channel: Channel) -> String {
        self.listeners
            .get(&project)
            .and_then(|listener| listener.us.as_ref())
            .map_or_else(
                || "@stageman".to_owned(),
                |us| stageman_channel::mention(channel, &us.user),
            )
    }

    /// Posts on an agent's behalf, with the tool call held open until the
    /// platform answers.
    pub fn post_for(&mut self, request: RequestId, speaking: &Speaking, place: &Place, text: &str) {
        self.spoken(speaking, place, text, Post::Saying { request }, false);
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
        let unreachable = |why: &str| format!("the channel could not be reached: {why}");
        match sent.purpose {
            Purpose::Post(post) => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::posted(sent.channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.answered(&post, outcome);
                if let Some(room) = &sent.room {
                    self.next_post(room);
                }
            }
            Purpose::Creating { job, origin } => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::room_created(sent.channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.room_created(job, origin, outcome);
            }
            Purpose::Keeping(keeping) => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::done(sent.channel, *status, body.as_slice())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                if let Err(why) = outcome {
                    tracing::warn!(?keeping, %why, "a room could not be kept");
                }
            }
            Purpose::Question(question) => {
                self.questioned(sent.channel, question, responded, at, effects);
            }
            Purpose::Growing { speaker, run } => {
                let outcome = match responded {
                    Responded::Answered { status, body, .. } => {
                        stageman_channel::posted(sent.channel, *status, body.as_slice())
                            .map(|_| ())
                            .map_err(|why| why.to_string())
                    }
                    Responded::Failed(why) => Err(unreachable(why)),
                };
                self.run_grown(speaker, run, outcome);
            }
        }
    }

    /// What follows from a channel's answer to a post, given why it was
    /// posted.
    fn answered(&mut self, post: &Post, outcome: Result<String, String>) {
        match post {
            Post::Notice => {
                if let Err(why) = outcome {
                    tracing::warn!(%why, "the room could not be spoken to");
                }
            }
            Post::Transcript { speaker, run } => self.run_posted(*speaker, *run, outcome),
            Post::Saying { request } => self.posted(*request, outcome.map(|_| ())),
        }
    }

    /// A request that waited on a write that never landed, and so was never
    /// made.
    ///
    /// Whatever waited on its answer is answered as if the channel could not
    /// be reached, which from where it stands is the truth: a notice is
    /// simply not said, a job whose room was never made is recorded as
    /// failed rather than left working with nowhere to speak, a call held
    /// open is answered rather than held for ever, a room is not kept, and a
    /// listener's question is asked again later.
    pub fn unsent(&mut self, id: EffectId, effects: &mut Vec<Effect>) {
        let Some(sent) = self.sent.remove(&id) else {
            return;
        };
        let never = || "the record it waited on could not be written".to_owned();
        match sent.purpose {
            Purpose::Post(post) => {
                self.answered(&post, Err(never()));
                if let Some(room) = &sent.room {
                    self.next_post(room);
                }
            }
            Purpose::Creating { job, origin } => self.room_created(job, origin, Err(never())),
            Purpose::Keeping(keeping) => {
                tracing::debug!(?keeping, "not done: {}", never());
            }
            Purpose::Question(
                Question::Introducing { project } | Question::Locating { project },
            ) => self.unasked(project, effects),
            Purpose::Growing { speaker, run } => self.run_grown(speaker, run, Err(never())),
        }
    }
}
