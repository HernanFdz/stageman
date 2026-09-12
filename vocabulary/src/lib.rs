//! What the instance and the world say to each other.
//!
//! An [`Event`] is what the world tells the instance and an [`Effect`] is
//! what the instance asks of the world. Both are plain data, generic over an
//! [`App`] that fills the one hole an application needs: its own events and
//! effects, which the world never interprets and hands to whatever the
//! application supplied to perform them. See
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
//! The mechanisms the vocabulary carries — a file, a process, a request, a
//! socket, a port, a timer — arrive one family at a time as the instance
//! starts speaking them, so that each family's shape is decided by its use.
//! Until then the application's own variants carry everything.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// What the world seeds the instance's randomness with, once.
///
/// From the operating system in production and from the scenario in a test,
/// which is the whole difference between the two. Never from anything
/// guessable: every unguessable value the instance mints comes from it.
pub type Seed = [u8; 32];

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
pub trait App {
    /// What the application's own world tells the instance.
    type Event: Serialize + DeserializeOwned + Clone + PartialEq + Named + Send + 'static;
    /// What the instance asks of the application's own world.
    type Effect: Serialize + DeserializeOwned + Clone + PartialEq + Named + Send + 'static;
}

/// One thing the world tells the instance.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Event<A: App> {
    /// Something of the application's own.
    App(A::Event),
}

// By hand rather than derived, because a derive would demand the bounds of
// the application marker itself, which is a type with nothing in it.
impl<A: App> Clone for Event<A> {
    fn clone(&self) -> Self {
        match self {
            Self::App(event) => Self::App(event.clone()),
        }
    }
}

impl<A: App> PartialEq for Event<A> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::App(mine), Self::App(theirs)) => mine == theirs,
        }
    }
}

impl<A: App> Named for Event<A> {
    fn kind(&self) -> &'static str {
        match self {
            Self::App(event) => event.kind(),
        }
    }
}

/// One thing the instance asks of the world.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Effect<A: App> {
    /// Something of the application's own.
    App(A::Effect),
}

impl<A: App> Clone for Effect<A> {
    fn clone(&self) -> Self {
        match self {
            Self::App(effect) => Self::App(effect.clone()),
        }
    }
}

impl<A: App> PartialEq for Effect<A> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::App(mine), Self::App(theirs)) => mine == theirs,
        }
    }
}

impl<A: App> Named for Effect<A> {
    fn kind(&self) -> &'static str {
        match self {
            Self::App(effect) => effect.kind(),
        }
    }
}

/// Something the world steps: one event in, effects out.
///
/// The only way anything reaches the deciding half, and the only way it
/// answers. The world calls this and nothing else.
pub trait Deciding {
    /// The application whose events and effects this speaks.
    type App: App;

    /// Handles one event and answers with what to do about it.
    fn step(&mut self, event: Event<Self::App>) -> Vec<Effect<Self::App>>;
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
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
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
mod tests {
    use super::{App, Bytes, Effect, Event, Named};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    enum Told {
        Rang { times: u32 },
    }

    impl Named for Told {
        fn kind(&self) -> &'static str {
            match self {
                Self::Rang { .. } => "Rang",
            }
        }
    }

    struct Doorbell;

    impl App for Doorbell {
        type Event = Told;
        type Effect = Told;
    }

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

        let binary = Bytes::new(vec![0xff, 0x00, 0x7f]);
        let served = serde_json::to_string(&binary).expect("it serialises");
        assert_eq!(served, r#"{"hex":"ff007f"}"#);
        let back: Bytes = serde_json::from_str(&served).expect("back");
        assert!(back == binary);
        assert_eq!(back.len(), 3);
        assert!(!back.is_empty());

        assert!(serde_json::from_str::<Bytes>(r#"{"hex":"abc"}"#).is_err());
        assert!(serde_json::from_str::<Bytes>(r#"{"hex":"zz"}"#).is_err());
    }
}
