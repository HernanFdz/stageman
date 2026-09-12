//! What the instance and the world say to each other.
//!
//! An [`Event`] is what the world tells the instance and an [`Effect`] is
//! what the instance asks of the world. Both are plain data: the mechanisms a
//! process reaches the outside through — a file, a process, a timer, its own
//! standard output, its own exit — and one hole an [`App`] fills with its own
//! events and effects, which the world never interprets and hands to whatever
//! the application supplied to perform them. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! **Everything here serialises in full and formats not at all.** A scenario
//! is a file of events and a trace is a file of effects, and replaying one
//! needs every byte, credentials included — which are fake in every file a
//! test writes. Neither enumeration implements `Debug` or `Display`, so that
//! nothing can format one into a log by accident; what the world logs is the
//! kind, from [`Named`]. That absence is a rule rather than an omission, and
//! `docs/conventions.md` §4 says why.
//!
//! The remaining mechanisms — a request, a socket, a port — arrive one family
//! at a time as the instance starts speaking them, so that each family's
//! shape is decided by its use.

pub mod scenario;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// What the world seeds the instance's randomness with, once.
///
/// From the operating system in production and from the scenario in a test,
/// which is the whole difference between the two. Never from anything
/// guessable: every unguessable value the instance mints comes from it.
pub type Seed = [u8; 32];

/// The environment the process was given, as the instance is constructed
/// with it.
///
/// What it was actually given and nothing synthesised, so that a scenario's
/// author knows exactly what to write: the application's own facts arrive as
/// the application's own events.
pub type Environment = BTreeMap<String, String>;

/// Something whose kind can be named, for a log line.
///
/// The one thing the world may say about an event or an effect without
/// formatting it, and so the one thing it does say.
pub trait Named {
    /// What kind of thing this is, as one word.
    fn kind(&self) -> &'static str;
}

/// What an application adds to the vocabulary.
///
/// The world never looks inside either type. It carries an event of the
/// application's to the instance like any other, and hands an effect of the
/// application's to whatever the entry point supplied for them.
pub trait App: Send + Sync + 'static {
    /// What the application's own world tells the instance.
    type Event: Serialize + DeserializeOwned + Clone + PartialEq + Named + Send + 'static;
    /// What the instance asks of the application's own world.
    type Effect: Serialize + DeserializeOwned + Clone + PartialEq + Named + Send + 'static;
}

/// What identifies an effect the instance is waiting to have answered.
///
/// Minted by the instance, from a counter, so that it is as deterministic as
/// everything else it does; carried on the effect and echoed on the event
/// that answers it. Opaque to the world.
///
/// It has to be unique among the effects still *waiting*, and nothing more
/// than that: an identifier is spent the moment its answer arrives, and a
/// handful are outstanding at a time. So the counter is cyclic rather than
/// bounded, and there is no exhausting it — coming round would take more
/// effects than a run can perform, and even then it could only meet an
/// identifier answered long before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EffectId(pub u64);

/// How a process that was run once came to an end.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finished {
    /// It ran and exited.
    Exited {
        /// Its exit status, or none if a signal ended it.
        status: Option<i32>,
        /// Everything it wrote to its standard output.
        stdout: Bytes,
        /// Everything it wrote to its standard error.
        stderr: Bytes,
    },
    /// There is no such program, which is an answer of its own: it is how a
    /// candidate for something is found to be absent.
    NotFound,
    /// It could not be started for some other reason.
    Failed(String),
}

/// One thing the world tells the instance.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Event<A: App> {
    /// Answers [`Effect::Read`]: the file's contents, or that there is no
    /// such file, or why it could not be read. Absent is its own answer
    /// rather than a failure, because a first run is not a failure.
    Read {
        /// Which read.
        id: EffectId,
        /// The contents, none for a file that is not there, or the reason.
        contents: Result<Option<Bytes>, String>,
    },
    /// Answers [`Effect::Write`]: the bytes reached the disk, or did not.
    Written {
        /// Which write.
        id: EffectId,
        /// Why not, if not.
        outcome: Result<(), String>,
    },
    /// Answers [`Effect::Run`]: how the process came to an end.
    Ran {
        /// Which run.
        id: EffectId,
        /// How.
        finished: Finished,
    },
    /// Answers [`Effect::Wake`].
    Woke {
        /// Which wake.
        id: EffectId,
    },
    /// Something of the application's own.
    App(A::Event),
}

// By hand rather than derived, because a derive would demand the bounds of
// the application marker itself, which is a type with nothing in it.
impl<A: App> Clone for Event<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Read { id, contents } => Self::Read {
                id: *id,
                contents: contents.clone(),
            },
            Self::Written { id, outcome } => Self::Written {
                id: *id,
                outcome: outcome.clone(),
            },
            Self::Ran { id, finished } => Self::Ran {
                id: *id,
                finished: finished.clone(),
            },
            Self::Woke { id } => Self::Woke { id: *id },
            Self::App(event) => Self::App(event.clone()),
        }
    }
}

impl<A: App> PartialEq for Event<A> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Read { id, contents },
                Self::Read {
                    id: their_id,
                    contents: theirs,
                },
            ) => id == their_id && contents == theirs,
            (
                Self::Written { id, outcome },
                Self::Written {
                    id: their_id,
                    outcome: theirs,
                },
            ) => id == their_id && outcome == theirs,
            (
                Self::Ran { id, finished },
                Self::Ran {
                    id: their_id,
                    finished: theirs,
                },
            ) => id == their_id && finished == theirs,
            (Self::Woke { id }, Self::Woke { id: their_id }) => id == their_id,
            (Self::App(mine), Self::App(theirs)) => mine == theirs,
            _ => false,
        }
    }
}

impl<A: App> Named for Event<A> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Read { .. } => "Read",
            Self::Written { .. } => "Written",
            Self::Ran { .. } => "Ran",
            Self::Woke { .. } => "Woke",
            Self::App(event) => event.kind(),
        }
    }
}

/// One thing the instance asks of the world.
///
/// The doc comment on each says whether it is answered, and by what. An
/// unanswered effect's failure is the world's to log.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Effect<A: App> {
    /// Read a file whole. Answered by [`Event::Read`].
    Read {
        /// Which read, on the answer.
        id: EffectId,
        /// The file.
        path: PathBuf,
    },
    /// Write a file whole and atomically — a temporary beside it, flushed,
    /// then renamed over it — making its directory if need be. Answered by
    /// [`Event::Written`], in the order writes were asked for.
    Write {
        /// Which write, on the answer.
        id: EffectId,
        /// The file.
        path: PathBuf,
        /// Its whole new contents.
        bytes: Bytes,
        /// Whether nobody but this user may read it, where the platform can
        /// say so. A property rather than a mode, because a mode is a
        /// mechanism.
        private: bool,
    },
    /// Run a program once, to its end. Answered by [`Event::Ran`].
    Run {
        /// Which run, on the answer.
        id: EffectId,
        /// The program, by path.
        program: PathBuf,
        /// Its arguments, none being a legitimate number of them.
        arguments: Vec<String>,
        /// Exactly the environment it is given, and nothing inherited.
        environment: Environment,
        /// What it is given on its standard input, then end of file.
        stdin: Option<Bytes>,
    },
    /// Wake the instance later. Answered by [`Event::Woke`].
    Wake {
        /// Which wake, on the answer.
        id: EffectId,
        /// How long from now.
        after: Duration,
    },
    /// Write to the process's standard output, which is where whoever
    /// started it is reading. Unanswered.
    Print {
        /// What to write, whole; a trailing newline is the instance's to
        /// include.
        text: String,
    },
    /// Stop the process, with the reason as its last word. Unanswered, since
    /// there is nobody left to answer.
    Exit {
        /// The reason, for whoever started it.
        message: String,
    },
    /// Something of the application's own.
    App(A::Effect),
}

impl<A: App> Clone for Effect<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Read { id, path } => Self::Read {
                id: *id,
                path: path.clone(),
            },
            Self::Write {
                id,
                path,
                bytes,
                private,
            } => Self::Write {
                id: *id,
                path: path.clone(),
                bytes: bytes.clone(),
                private: *private,
            },
            Self::Run {
                id,
                program,
                arguments,
                environment,
                stdin,
            } => Self::Run {
                id: *id,
                program: program.clone(),
                arguments: arguments.clone(),
                environment: environment.clone(),
                stdin: stdin.clone(),
            },
            Self::Wake { id, after } => Self::Wake {
                id: *id,
                after: *after,
            },
            Self::Print { text } => Self::Print { text: text.clone() },
            Self::Exit { message } => Self::Exit {
                message: message.clone(),
            },
            Self::App(effect) => Self::App(effect.clone()),
        }
    }
}

impl<A: App> PartialEq for Effect<A> {
    fn eq(&self, other: &Self) -> bool {
        // Through the one representation both sides share, which is what a
        // scenario compares anyway.
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl<A: App> Named for Effect<A> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Read { .. } => "Read",
            Self::Write { .. } => "Write",
            Self::Run { .. } => "Run",
            Self::Wake { .. } => "Wake",
            Self::Print { .. } => "Print",
            Self::Exit { .. } => "Exit",
            Self::App(effect) => effect.kind(),
        }
    }
}

/// Something the world steps: constructed from a seed and an environment,
/// then one event in and effects out, for as long as the process runs.
///
/// The only way anything reaches the deciding half, and the only way it
/// answers. The world calls these and nothing else.
pub trait Deciding: Sized {
    /// The application whose events and effects this speaks.
    type App: App;

    /// Constructs the deciding half from the two facts that exist before
    /// anything happens, and answers with what it asks for first.
    ///
    /// Everything else — a key, a file, what is installed — it asks for
    /// through effects and learns from their answers.
    fn boot(seed: Seed, environment: Environment) -> (Self, Vec<Effect<Self::App>>);

    /// Handles one event and answers with what to do about it.
    fn step(&mut self, event: Event<Self::App>) -> Vec<Effect<Self::App>>;

    /// Everything it holds, as a value a scenario compares and a reviewer
    /// reads, credentials in the clear.
    fn snapshot(&self) -> serde_json::Value;
}

/// Bytes that cross the vocabulary, readable where they are text.
///
/// Serialised as the text they spell when they are valid UTF-8, and as hex
/// otherwise, so that a file's contents in a scenario read as the file and
/// a trace never carries an array of numbers. Compared and stored as the
/// bytes themselves.
#[derive(Clone, PartialEq, Eq)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    /// Wraps bytes.
    #[must_use]
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// The bytes, owned.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }

    /// How many.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The bytes as text, where they are text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<String> for Bytes {
    fn from(text: String) -> Self {
        Self(text.into_bytes())
    }
}

/// The two spellings bytes take on the wire.
#[derive(Serialize, Deserialize)]
enum Spelled {
    #[serde(rename = "text")]
    Text(String),
    #[serde(rename = "hex")]
    Hex(String),
}

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let spelled = std::str::from_utf8(&self.0).map_or_else(
            |_| {
                use std::fmt::Write as _;
                Spelled::Hex(self.0.iter().fold(String::new(), |mut hex, byte| {
                    // Writing to a string cannot fail, so the result says nothing.
                    let _ = write!(hex, "{byte:02x}");
                    hex
                }))
            },
            |text| Spelled::Text(text.to_owned()),
        );
        spelled.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Spelled::deserialize(deserializer)? {
            Spelled::Text(text) => Ok(Self(text.into_bytes())),
            Spelled::Hex(hex) => {
                let digits: Vec<u8> = hex.as_bytes().to_vec();
                if !digits.len().is_multiple_of(2) {
                    return Err(serde::de::Error::custom("hex with an odd number of digits"));
                }
                digits
                    .chunks(2)
                    .map(|pair| {
                        let text = std::str::from_utf8(pair)
                            .map_err(|_| serde::de::Error::custom("hex that is not ASCII"))?;
                        u8::from_str_radix(text, 16).map_err(|_| {
                            serde::de::Error::custom("hex with a digit that is not one")
                        })
                    })
                    .collect::<Result<Vec<u8>, D::Error>>()
                    .map(Self)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod doorbell {
    //! The smallest application there is, for the tests here: a bell that
    //! counts its rings and asks the world to print each one.

    use super::{App, Deciding, Effect, EffectId, Environment, Event, Named, Seed};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Told {
        Rang { times: u32 },
    }

    impl Named for Told {
        fn kind(&self) -> &'static str {
            match self {
                Self::Rang { .. } => "Rang",
            }
        }
    }

    pub struct Doorbell;

    impl App for Doorbell {
        type Event = Told;
        type Effect = Told;
    }

    /// Counts rings, and reads a file of its own name at boot to know how
    /// many it had heard before.
    #[derive(Serialize)]
    pub struct Bell {
        pub heard: u32,
        pub next: u64,
        pub booted: bool,
    }

    impl Deciding for Bell {
        type App = Doorbell;

        fn boot(_seed: Seed, environment: Environment) -> (Self, Vec<Effect<Doorbell>>) {
            let path = environment
                .get("BELL")
                .map_or_else(|| "bell".to_owned(), Clone::clone);
            (
                Self {
                    heard: 0,
                    next: 2,
                    booted: false,
                },
                vec![Effect::Read {
                    id: EffectId(1),
                    path: path.into(),
                }],
            )
        }

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "a bell that counted past its type should fail this crate's tests rather than clamp"
        )]
        fn step(&mut self, event: Event<Doorbell>) -> Vec<Effect<Doorbell>> {
            match event {
                Event::Read { contents, .. } => {
                    self.heard = contents
                        .ok()
                        .flatten()
                        .and_then(|bytes| bytes.as_text().and_then(|text| text.parse().ok()))
                        .unwrap_or(0);
                    self.booted = true;
                    Vec::new()
                }
                Event::App(Told::Rang { times }) => {
                    self.heard += times;
                    let id = EffectId(self.next);
                    self.next += 1;
                    vec![
                        Effect::Print {
                            text: format!("rang {times}, heard {} in all\n", self.heard),
                        },
                        Effect::Write {
                            id,
                            path: "bell".into(),
                            bytes: self.heard.to_string().into(),
                            private: false,
                        },
                    ]
                }
                Event::Written { .. } | Event::Ran { .. } | Event::Woke { .. } => Vec::new(),
            }
        }

        fn snapshot(&self) -> serde_json::Value {
            serde_json::to_value(self).expect("a bell serialises")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::doorbell::{Doorbell, Told};
    use super::{Bytes, Effect, EffectId, Event, Finished, Named};

    /// The hole carries the application's own, and the kind is what a log
    /// may say.
    #[test]
    fn an_applications_own_event_crosses_whole_and_names_its_kind() {
        let event: Event<Doorbell> = Event::App(Told::Rang { times: 2 });
        let served = serde_json::to_string(&event).expect("it serialises");
        assert_eq!(served, r#"{"App":{"Rang":{"times":2}}}"#);
        let back: Event<Doorbell> = serde_json::from_str(&served).expect("and back");
        assert!(back == event);
        assert_eq!(event.kind(), "Rang");

        let effect: Effect<Doorbell> = Effect::App(Told::Rang { times: 1 });
        assert_eq!(effect.kind(), "Rang");
        let served = serde_json::to_string(&effect).expect("it serialises");
        let back: Effect<Doorbell> = serde_json::from_str(&served).expect("and back");
        assert!(back == effect);
    }

    /// Every mechanism crosses whole and names its kind.
    #[test]
    fn every_mechanism_crosses_whole_and_names_its_kind() {
        let effects: Vec<Effect<Doorbell>> = vec![
            Effect::Read {
                id: EffectId(1),
                path: "/etc/hostname".into(),
            },
            Effect::Write {
                id: EffectId(2),
                path: "/tmp/x".into(),
                bytes: Bytes::new(b"x".to_vec()),
                private: true,
            },
            Effect::Run {
                id: EffectId(3),
                program: "/usr/bin/true".into(),
                arguments: vec!["--version".to_owned()],
                environment: [("HOME".to_owned(), "/home/x".to_owned())].into(),
                stdin: None,
            },
            Effect::Wake {
                id: EffectId(4),
                after: std::time::Duration::from_secs(1),
            },
            Effect::Print {
                text: "hello\n".to_owned(),
            },
            Effect::Exit {
                message: "bye".to_owned(),
            },
        ];
        let kinds: Vec<&str> = effects.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Read", "Write", "Run", "Wake", "Print", "Exit"]);
        for effect in &effects {
            let served = serde_json::to_string(effect).expect("it serialises");
            let back: Effect<Doorbell> = serde_json::from_str(&served).expect("and back");
            assert!(back == *effect, "{served}");
            assert!(back.clone() == *effect);
        }

        let events: Vec<Event<Doorbell>> = vec![
            Event::Read {
                id: EffectId(1),
                contents: Ok(None),
            },
            Event::Written {
                id: EffectId(2),
                outcome: Err("full".to_owned()),
            },
            Event::Ran {
                id: EffectId(3),
                finished: Finished::Exited {
                    status: Some(0),
                    stdout: Bytes::new(Vec::new()),
                    stderr: Bytes::new(Vec::new()),
                },
            },
            Event::Ran {
                id: EffectId(3),
                finished: Finished::NotFound,
            },
            Event::Woke { id: EffectId(4) },
        ];
        let kinds: Vec<&str> = events.iter().map(Named::kind).collect();
        assert_eq!(kinds, ["Read", "Written", "Ran", "Ran", "Woke"]);
        for event in &events {
            let served = serde_json::to_string(event).expect("it serialises");
            let back: Event<Doorbell> = serde_json::from_str(&served).expect("and back");
            assert!(back == *event, "{served}");
            assert!(back.clone() == *event);
        }
        assert!(events[0] != events[4]);
    }

    /// Neither enumeration formats, and this is what keeps it so: an
    /// inherent method exists only where `Debug` does, and the trait
    /// method underneath answers otherwise, so deriving `Debug` on either
    /// flips the answer and fails here.
    #[test]
    fn the_vocabulary_formats_not_at_all() {
        struct Probe<T>(std::marker::PhantomData<T>);
        impl<T: std::fmt::Debug> Probe<T> {
            #[expect(
                clippy::unused_self,
                reason = "a method, so that resolution prefers it to the trait's where it exists"
            )]
            const fn formats(&self) -> bool {
                true
            }
        }
        trait Otherwise {
            fn formats(&self) -> bool {
                false
            }
        }
        impl<T> Otherwise for Probe<T> {}

        assert!(!Probe::<Event<Doorbell>>(std::marker::PhantomData).formats());
        assert!(!Probe::<Effect<Doorbell>>(std::marker::PhantomData).formats());
        assert!(!Probe::<Bytes>(std::marker::PhantomData).formats());
        assert!(
            Probe::<String>(std::marker::PhantomData).formats(),
            "the probe tells"
        );
    }

    /// Text reads as text, and anything else is spelled so it survives.
    #[test]
    fn bytes_read_as_the_text_they_spell_and_survive_otherwise() {
        let text = Bytes::new(b"{\"kept\": true}".to_vec());
        assert_eq!(
            serde_json::to_string(&text).expect("it serialises"),
            r#"{"text":"{\"kept\": true}"}"#
        );
        let back: Bytes = serde_json::from_str(r#"{"text":"{\"kept\": true}"}"#).expect("back");
        assert!(back == text);
        assert_eq!(back.as_text(), Some("{\"kept\": true}"));

        let binary = Bytes::new(vec![0xff, 0x00, 0x7f]);
        let served = serde_json::to_string(&binary).expect("it serialises");
        assert_eq!(served, r#"{"hex":"ff007f"}"#);
        let back: Bytes = serde_json::from_str(&served).expect("back");
        assert!(back == binary);
        assert_eq!(back.len(), 3);
        assert!(!back.is_empty());
        assert_eq!(back.as_text(), None);

        assert!(serde_json::from_str::<Bytes>(r#"{"hex":"abc"}"#).is_err());
        assert!(serde_json::from_str::<Bytes>(r#"{"hex":"zz"}"#).is_err());

        // What comes out is what went in, both ways round: these are what a
        // file is written from and what a program's output is read through,
        // so bytes invented here would be bytes on somebody's disk.
        assert_eq!(binary.as_slice(), [0xff, 0x00, 0x7f]);
        assert_eq!(binary.clone().into_inner(), vec![0xff, 0x00, 0x7f]);
        let none = Bytes::new(Vec::new());
        assert!(none.is_empty(), "nothing is empty");
        assert!(!binary.is_empty(), "and something is not");
        assert_eq!(none.len(), 0);
        assert!(none.as_slice().is_empty());
        assert_eq!(none.into_inner(), Vec::<u8>::new());
    }

    /// One field apart is a different value, on both enumerations.
    ///
    /// The whole of what a replay rests on: a comparison satisfied by an
    /// identifier alone would call an answer to one effect the answer to
    /// another, and a file would agree with a run that had done something
    /// else. Written as pairs differing in exactly one place, because that
    /// is the comparison a weakened one gets wrong.
    #[test]
    fn one_field_apart_is_not_the_same_value() {
        let read = |id: u64, contents: Result<Option<Bytes>, String>| Event::<Doorbell>::Read {
            id: EffectId(id),
            contents,
        };
        let some = || Ok(Some(Bytes::new(b"x".to_vec())));
        assert!(read(1, Ok(None)) == read(1, Ok(None)));
        assert!(
            read(1, Ok(None)) != read(1, some()),
            "the same read, answered differently"
        );
        assert!(
            read(1, Ok(None)) != read(2, Ok(None)),
            "a different read, answered the same"
        );

        let written = |id: u64, outcome: Result<(), String>| Event::<Doorbell>::Written {
            id: EffectId(id),
            outcome,
        };
        assert!(written(1, Ok(())) == written(1, Ok(())));
        assert!(written(1, Ok(())) != written(1, Err("full".to_owned())));
        assert!(written(1, Ok(())) != written(2, Ok(())));

        let ran = |id: u64, finished: Finished| Event::<Doorbell>::Ran {
            id: EffectId(id),
            finished,
        };
        assert!(ran(1, Finished::NotFound) == ran(1, Finished::NotFound));
        assert!(ran(1, Finished::NotFound) != ran(1, Finished::Failed("no".to_owned())));
        assert!(ran(1, Finished::NotFound) != ran(2, Finished::NotFound));

        // Effects compare through the one representation both sides share,
        // so the field that differs here is the one a reader would miss.
        let write = |private: bool| Effect::<Doorbell>::Write {
            id: EffectId(1),
            path: "/tmp/x".into(),
            bytes: Bytes::new(b"x".to_vec()),
            private,
        };
        assert!(write(true) == write(true));
        assert!(
            write(true) != write(false),
            "a private write is not a public one"
        );
    }
}
