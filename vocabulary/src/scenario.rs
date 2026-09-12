//! A scenario: one typed file of events in and effects out, compared exactly.
//!
//! Recorded by running the deciding half against a world that answers, and
//! replayed by feeding the file's events to a fresh one and comparing what it
//! emits and what it holds, turn by turn. Nothing behaves during a replay and
//! nothing interprets: any difference in effects or in state is a change of
//! behaviour, and the diff of the file says where. See
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! The state appears in full once, after construction, and as a standard
//! JSON patch after every turn: a review then reads what changed rather than
//! twenty copies of what did not, and a field renamed in the domain touches
//! one line rather than every file.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{App, Deciding, Effect, Environment, Event, Seed};

/// What a scenario is called, and what it is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// A sentence naming the behaviour it pins.
    pub title: String,
    /// Longer, if the title cannot say why it exists.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Construction: the two facts that exist before anything happens, what the
/// deciding half asked for first, and everything it held once constructed.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub struct Init<A: App> {
    /// The seed, as hex.
    #[serde(with = "hex_seed")]
    pub seed: Seed,
    /// The environment the process was given.
    pub environment: Environment,
    /// What it asked for on construction, in order.
    pub effects: Vec<Effect<A>>,
    /// Everything it held once constructed.
    pub state: serde_json::Value,
}

/// One event, and what it caused.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub struct Turn<A: App> {
    /// When the event arrived, on the recording world's clock.
    pub at: u64,
    /// The event.
    pub event: Event<A>,
    /// What it caused, in order.
    pub effects: Vec<Effect<A>>,
    /// What it changed, as a patch from the state before it.
    pub changed: json_patch::Patch,
}

/// One scenario, whole.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub struct Scenario<A: App> {
    /// What it is.
    pub meta: Meta,
    /// How it starts.
    pub init: Init<A>,
    /// What happens, one event at a time.
    pub turns: Vec<Turn<A>>,
}

/// Records a scenario as a deciding half is run against a world.
pub struct Recorder<A: App> {
    scenario: Scenario<A>,
    /// The state as of the last turn, for the next patch.
    state: serde_json::Value,
}

impl<A: App> Recorder<A> {
    /// Begins a recording at construction.
    #[must_use]
    pub fn started(
        meta: Meta,
        seed: Seed,
        environment: Environment,
        effects: Vec<Effect<A>>,
        state: serde_json::Value,
    ) -> Self {
        Self {
            scenario: Scenario {
                meta,
                init: Init {
                    seed,
                    environment,
                    effects,
                    state: state.clone(),
                },
                turns: Vec::new(),
            },
            state,
        }
    }

    /// Records one turn: the event, what it caused, and what changed.
    pub fn turned(
        &mut self,
        at: u64,
        event: Event<A>,
        effects: Vec<Effect<A>>,
        state: serde_json::Value,
    ) {
        let changed = json_patch::diff(&self.state, &state);
        self.state = state;
        self.scenario.turns.push(Turn {
            at,
            event,
            effects,
            changed,
        });
    }

    /// The scenario recorded so far.
    #[must_use]
    pub fn finished(self) -> Scenario<A> {
        self.scenario
    }
}

/// Where a replay stopped agreeing with its file.
///
/// Rendered through serialisation, deliberately: this is the one place a
/// value of the vocabulary is written out for a person, it is a test failure,
/// and every credential in a scenario file is fake.
pub enum Mismatch {
    /// What was asked for on construction differs.
    Init {
        /// From the file.
        expected: Box<serde_json::Value>,
        /// From the run.
        actual: Box<serde_json::Value>,
    },
    /// What a turn caused differs.
    Effects {
        /// Which turn, counting from zero.
        turn: usize,
        /// From the file.
        expected: Box<serde_json::Value>,
        /// From the run.
        actual: Box<serde_json::Value>,
    },
    /// What is held differs.
    State {
        /// After which turn, or none for construction.
        turn: Option<usize>,
        /// From the file, reconstructed patch by patch.
        expected: Box<serde_json::Value>,
        /// From the run.
        actual: Box<serde_json::Value>,
    },
    /// A patch in the file could not be applied to the state before it,
    /// which means the file is inconsistent with itself.
    Patch {
        /// Which turn.
        turn: usize,
        /// Why not.
        why: String,
    },
    /// Effects that could not be written out as a value, which no effect
    /// made of plain data can be — but the comparison is made on values,
    /// so the failure has to have somewhere to go.
    Unserialisable {
        /// Why not.
        why: String,
    },
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pretty = |value: &serde_json::Value| {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| "<unrenderable>".to_owned())
        };
        match self {
            Self::Init { expected, actual } => write!(
                f,
                "construction asked for different effects\nexpected: {}\nactual: {}",
                pretty(expected),
                pretty(actual)
            ),
            Self::Effects {
                turn,
                expected,
                actual,
            } => write!(
                f,
                "turn {turn} caused different effects\nexpected: {}\nactual: {}",
                pretty(expected),
                pretty(actual)
            ),
            Self::State {
                turn,
                expected,
                actual,
            } => write!(
                f,
                "the state differs after {}\nexpected: {}\nactual: {}",
                turn.map_or_else(|| "construction".to_owned(), |turn| format!("turn {turn}")),
                pretty(expected),
                pretty(actual)
            ),
            Self::Patch { turn, why } => {
                write!(
                    f,
                    "turn {turn}'s patch does not apply to the state before it: {why}"
                )
            }
            Self::Unserialisable { why } => write!(f, "effects could not be serialised: {why}"),
        }
    }
}

impl fmt::Debug for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Mismatch {}

/// Replays a scenario against a fresh deciding half, and says where it
/// first disagrees, if it does.
///
/// # Errors
///
/// Fails at the first turn whose effects or state differ from the file's.
pub fn replay<D: Deciding>(scenario: &Scenario<D::App>) -> Result<(), Mismatch> {
    let (mut deciding, effects) = D::boot(scenario.init.seed, scenario.init.environment.clone());
    let (expected, actual) = (value(&scenario.init.effects)?, value(&effects)?);
    if actual != expected {
        return Err(Mismatch::Init {
            expected: Box::new(expected),
            actual: Box::new(actual),
        });
    }
    let mut expected = scenario.init.state.clone();
    let actual = deciding.snapshot();
    if actual != expected {
        return Err(Mismatch::State {
            turn: None,
            expected: Box::new(expected),
            actual: Box::new(actual),
        });
    }
    for (turn, recorded) in scenario.turns.iter().enumerate() {
        let effects = deciding.step(recorded.event.clone());
        let (wanted, caused) = (value(&recorded.effects)?, value(&effects)?);
        if caused != wanted {
            return Err(Mismatch::Effects {
                turn,
                expected: Box::new(wanted),
                actual: Box::new(caused),
            });
        }
        json_patch::patch(&mut expected, &recorded.changed).map_err(|why| Mismatch::Patch {
            turn,
            why: why.to_string(),
        })?;
        let actual = deciding.snapshot();
        if actual != expected {
            return Err(Mismatch::State {
                turn: Some(turn),
                expected: Box::new(expected),
                actual: Box::new(actual),
            });
        }
    }
    Ok(())
}

/// Effects as the value they are compared and reported as.
fn value<A: App>(effects: &[Effect<A>]) -> Result<serde_json::Value, Mismatch> {
    serde_json::to_value(effects).map_err(|why| Mismatch::Unserialisable {
        why: why.to_string(),
    })
}

/// A seed as hex on the wire, because thirty-two numbers say nothing.
mod hex_seed {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::Seed;

    pub fn serialize<S: Serializer>(seed: &Seed, serializer: S) -> Result<S::Ok, S::Error> {
        use std::fmt::Write as _;
        let hex = seed.iter().fold(String::new(), |mut hex, byte| {
            // Writing to a string cannot fail, so the result says nothing.
            let _ = write!(hex, "{byte:02x}");
            hex
        });
        hex.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Seed, D::Error> {
        let hex = String::deserialize(deserializer)?;
        let digits = hex.as_bytes();
        if digits.len() != 64 {
            return Err(serde::de::Error::custom("a seed is sixty-four hex digits"));
        }
        let mut seed: Seed = [0; 32];
        for (slot, pair) in seed.iter_mut().zip(digits.chunks(2)) {
            let text = std::str::from_utf8(pair)
                .map_err(|_| serde::de::Error::custom("hex that is not ASCII"))?;
            *slot = u8::from_str_radix(text, 16)
                .map_err(|_| serde::de::Error::custom("hex with a digit that is not one"))?;
        }
        Ok(seed)
    }
}

/// An environment with nothing in it, for a scenario that needs none.
#[must_use]
pub const fn no_environment() -> Environment {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::{Meta, Recorder, Scenario, replay};
    use crate::doorbell::{Bell, Doorbell, Told};
    use crate::{Deciding, Effect, EffectId, Environment, Event};

    /// Records the bell being rung twice, as a world that answers would.
    fn recorded() -> Scenario<Doorbell> {
        let seed = [7; 32];
        let environment: Environment = [("BELL".to_owned(), "/tmp/bell".to_owned())].into();
        let (mut bell, effects) = Bell::boot(seed, environment.clone());
        let mut recorder = Recorder::started(
            Meta {
                title: "a bell counts its rings".to_owned(),
                description: String::new(),
            },
            seed,
            environment,
            effects,
            bell.snapshot(),
        );
        let arrivals = [
            Event::Read {
                id: EffectId(1),
                contents: Ok(Some("3".to_owned().into())),
            },
            Event::App(Told::Rang { times: 2 }),
            Event::Written {
                id: EffectId(2),
                outcome: Ok(()),
            },
        ];
        for (at, event) in (0_u64..).zip(arrivals) {
            let effects = bell.step(event.clone());
            recorder.turned(at, event, effects, bell.snapshot());
        }
        recorder.finished()
    }

    /// A recording replays against a fresh bell, through the file and back.
    #[test]
    fn a_recording_replays_exactly_through_its_file() {
        let scenario = recorded();
        let file = serde_json::to_string_pretty(&scenario).expect("it serialises");
        assert!(file.contains(r#""seed": "0707"#), "{file}");
        assert!(
            file.contains(r#""title": "a bell counts its rings""#),
            "{file}"
        );
        assert!(
            file.contains(r#""op": "replace""#),
            "the patch says what changed: {file}"
        );
        let read: Scenario<Doorbell> = serde_json::from_str(&file).expect("and back");
        assert_eq!(read.turns.len(), 3);
        replay::<Bell>(&read).expect("the bell behaves as recorded");
    }

    /// A file that says something else is caught at the first turn that
    /// differs, and says which.
    #[test]
    fn a_replay_says_where_it_first_disagrees() {
        let mut scenario = recorded();
        scenario.turns[1].effects.clear();
        let failure = replay::<Bell>(&scenario).expect_err("the effects differ");
        assert!(
            failure
                .to_string()
                .contains("turn 1 caused different effects"),
            "{failure}"
        );

        let mut scenario = recorded();
        scenario.init.effects.push(Effect::Print {
            text: "extra".to_owned(),
        });
        let failure = replay::<Bell>(&scenario).expect_err("construction differs");
        assert!(
            failure.to_string().starts_with("construction asked for"),
            "{failure}"
        );

        let mut scenario = recorded();
        scenario.turns[1].changed = json_patch::Patch(Vec::new());
        let failure = replay::<Bell>(&scenario).expect_err("the state differs");
        assert!(
            failure
                .to_string()
                .contains("the state differs after turn 1"),
            "{failure}"
        );
    }
}
