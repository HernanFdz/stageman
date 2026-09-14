//! Booting, against the simulated world: what an instance asks before it is
//! awake, in what order, and how it refuses.

use stageman_core::Progress;

use stageman_instance::{AppEvent, Effect, Event, Instance};
use stageman_vocabulary::{Bytes, EffectId, Environment, Finished};

use crate::simulation::{Simulation, TARGET, job, seed, watching};

/// A program that ran and said nothing, which is what a runtime answering
/// its version check looks like to everything downstream of the parsing.
const fn exited() -> Finished {
    Finished::Exited {
        status: Some(0),
        stdout: Bytes::new(Vec::new()),
        stderr: Bytes::new(Vec::new()),
    }
}

/// The startup block is printed once, after the first write has landed, and
/// its address is the last line: what anything supervising a start waits
/// for, so everything worth reading is above it.
#[test]
fn the_address_is_announced_last_and_only_once_the_first_write_has_landed() {
    let mut world = Simulation::new();
    world.holding(&watching(&[]));
    let mut instance = world.wake(seed(1));
    assert!(
        world.printed().is_empty(),
        "nothing is announced before the write lands: {:?}",
        world.printed()
    );

    world.run_until(&mut instance, 1);
    let [block] = world.printed() else {
        panic!("one block: {:?}", world.printed());
    };
    assert!(block.contains("stageman is running."), "{block}");
    assert!(block.contains("key        STAGEMAN_KEY"), "{block}");
    assert!(block.contains("instance   /sim/instance.json"), "{block}");
    assert!(block.contains("domain     localhost"), "{block}");
    assert!(block.contains("runtime    "), "{block}");
    assert!(
        block
            .trim_end()
            .ends_with("dashboard  http://127.0.0.1:8080"),
        "the address is the last line: {block}"
    );
    assert_eq!(world.exited(), None);

    // Never again, however many writes land.
    world.run_until(&mut instance, 60_000);
    assert_eq!(world.printed().len(), 1);
}

/// A machine with no runtime cannot run this at all, and that is the failure
/// said before any other: nothing else is asked for.
#[test]
fn no_runtime_is_refused_before_anything_else_is_asked() {
    let mut world = Simulation::new();
    world.without_a_runtime();
    world.holding(&watching(&[(job(1), Progress::Working)]));
    let _instance = world.wake(seed(1));

    let refused = world.exited().expect("refused");
    assert!(
        refused.starts_with("no container runtime found."),
        "{refused}"
    );
    assert!(refused.contains("Looked in:"), "{refused}");
    assert!(
        !world.shape().iter().any(|line| line.starts_with("-> Read")),
        "the file is not asked for once there is nothing to run with: {:?}",
        world.shape()
    );
    assert!(world.printed().is_empty());
}

/// A first run finds no file, writes one, and is announced on the strength
/// of that write.
#[test]
fn a_first_run_writes_its_file_before_it_is_announced() {
    let mut world = Simulation::new();
    world.recording(
        "a-first-run",
        "A first run finds no file, writes one, and is announced on the strength of that write",
    );
    let mut instance = world.wake(seed(1));
    assert!(world.disk().is_none(), "not yet: the write has not landed");

    world.run_until(&mut instance, 1);
    assert!(world.disk().is_some(), "an instance with nothing in it");
    assert_eq!(world.printed().len(), 1);
    assert!(instance.id().is_some(), "and it has an identity");
    world.recorded();
}

/// A key that is not key material is refused, and the refusal does not
/// repeat what it was given.
#[test]
fn a_key_that_is_not_key_material_is_refused_without_echoing_it() {
    let mut environment = Simulation::environment();
    environment.insert("STAGEMAN_KEY".to_owned(), "far-too-short".to_owned());
    let mut world = Simulation::new();
    let mut instance = world.wake_given(seed(1), environment);
    world.run_until(&mut instance, 0);

    let refused = world.exited().expect("refused");
    assert!(refused.contains("key"), "{refused}");
    assert!(
        !refused.contains("far-too-short"),
        "it echoed the key: {refused}"
    );
}

/// A key variable a wrapper script cleared says *do not use this*, and the
/// key file is what answers instead.
///
/// The failure this exists to prevent is a start refused over a value nobody
/// wrote: an exported-but-empty variable is not key material, and reading it
/// as one would stop a daemon that has a perfectly good key on disk.
#[test]
fn a_cleared_key_variable_is_ignored_rather_than_read_as_key_material() {
    let mut environment = Simulation::environment();
    environment.insert("STAGEMAN_KEY".to_owned(), "   ".to_owned());
    let mut world = Simulation::new();
    let mut instance = world.wake_given(seed(1), environment);
    // Two, because the minted key is a write of its own and the instance's
    // own file cannot be asked for until that one has landed.
    world.run_until(&mut instance, 2);

    assert_eq!(world.exited(), None, "a cleared variable refused the start");
    let [block] = world.printed() else {
        panic!("one block: {:?}", world.printed());
    };
    assert!(
        block.contains("(generated just now)"),
        "the key file answered instead of the variable: {block}"
    );
}

/// A file that cannot be written fails the start, with the path, rather
/// than serving an instance nothing can save.
#[test]
fn a_file_that_cannot_be_written_refuses_the_start() {
    let mut world = Simulation::new();
    world.next_write_fails("the disk is read-only");
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 1);

    let refused = world.exited().expect("refused");
    assert!(refused.contains("could not be written"), "{refused}");
    assert!(refused.contains("/sim/instance.json"), "{refused}");
    assert!(world.printed().is_empty(), "nothing is announced");
}

/// An answer to a question booting did not ask moves nothing.
///
/// Every phase carries the identifier it is waiting on, and the world
/// answers with the identifier it was given. A phase that took whatever
/// arrived would walk on the strength of a timer going off, or of a read it
/// had already been handed, and a start would be decided by ordering rather
/// than by what it asked. Driven event by event rather than through the
/// simulation, because what is under test is exactly the answers a world
/// would never send.
#[test]
fn an_answer_to_another_question_moves_nothing() {
    let stray = EffectId(9999);
    // No key in the environment, so the key file is asked for and written:
    // the two phases that would otherwise be skipped.
    let mut environment = Simulation::environment();
    environment.remove("STAGEMAN_KEY");
    let (mut instance, effects) = Instance::boot(seed(1), environment, TARGET);

    // A runtime candidate, and the address the tools are served on.
    let [Effect::Run { id: version, .. }, Effect::Bind { .. }] = effects.as_slice() else {
        panic!("a runtime question and a bind, and this is not them");
    };

    let mut walked = |at: &str, stray_event: Event, real: Event| {
        assert_eq!(instance.snapshot(), serde_json::json!({ "booting": at }));
        assert!(
            instance.step(stray_event).is_empty(),
            "a stray answer asked for something at {at}"
        );
        assert_eq!(
            instance.snapshot(),
            serde_json::json!({ "booting": at }),
            "a stray answer moved booting on from {at}"
        );
        instance.step(real)
    };

    let asked = walked(
        "runtime",
        Event::Ran {
            id: stray,
            finished: exited(),
        },
        Event::Ran {
            id: *version,
            finished: exited(),
        },
    );
    let [Effect::Read { id: key, .. }] = asked.as_slice() else {
        panic!("the key file is asked for");
    };

    let asked = walked(
        "key",
        Event::Read {
            id: stray,
            contents: Ok(None),
        },
        Event::Read {
            id: *key,
            contents: Ok(None),
        },
    );
    let [Effect::Write { id: minted, .. }] = asked.as_slice() else {
        panic!("a key is minted and written");
    };

    let asked = walked(
        "key written",
        Event::Written {
            id: stray,
            outcome: Ok(()),
        },
        Event::Written {
            id: *minted,
            outcome: Ok(()),
        },
    );
    let [Effect::Read { id: file, .. }] = asked.as_slice() else {
        panic!("then the instance's own file");
    };

    let asked = walked(
        "file",
        Event::Read {
            id: stray,
            contents: Ok(None),
        },
        Event::Read {
            id: *file,
            contents: Ok(None),
        },
    );
    assert_eq!(asked.len(), 2, "both listings at once");

    assert_eq!(
        instance.snapshot(),
        serde_json::json!({ "booting": "listing" })
    );
    assert!(
        instance
            .step(Event::Ran {
                id: stray,
                finished: exited(),
            })
            .is_empty(),
        "a stray answer asked for something while listing"
    );
    assert_eq!(
        instance.snapshot(),
        serde_json::json!({ "booting": "listing" })
    );
}

/// A file that is there and cannot be opened says why, underneath.
///
/// The reason is the whole message: "the instance could not be opened" on
/// its own sends somebody to read the source, where the line underneath
/// says whether it was the key or the file.
#[test]
fn an_instance_that_cannot_be_opened_says_what_went_wrong_underneath() {
    let mut world = Simulation::new();
    world.holding_bytes(b"{\"this\": \"is not a snapshot\"}");
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 1);

    let refused = world.exited().expect("refused");
    assert!(refused.contains("could not be opened"), "{refused}");
    // Every level of it, to the bottom: the outermost says which part of
    // opening failed and only the innermost says what was actually wrong
    // with the file, which is the line somebody can act on.
    assert!(
        refused.contains("caused by: the file is not valid JSON"),
        "{refused}"
    );
    assert!(refused.contains("missing field"), "{refused}");
    assert!(world.printed().is_empty(), "nothing is announced");
}

/// A runtime command is given this process's environment, less this
/// project's own.
///
/// Constructed rather than inherited, which `docs/conventions.md` §3 asks
/// of every child process — and narrower than what this process was given,
/// because a container runtime reads a host or a context out of its
/// environment and has no business with the key that opens an instance.
#[test]
fn a_runtime_command_is_given_this_environment_less_this_projects_own() {
    let (_, effects) = Instance::boot(seed(1), Simulation::environment(), TARGET);

    let [
        Effect::Run {
            environment: given, ..
        },
        Effect::Bind { .. },
    ] = effects.as_slice()
    else {
        panic!("a runtime question and a bind, and this is not them");
    };
    assert_eq!(given.get("HOME").map(String::as_str), Some("/sim/home"));
    assert!(
        !given.keys().any(|name| name.starts_with("STAGEMAN_")),
        "a runtime was handed this project's own: {given:?}"
    );
}

/// The deciding half a replay drives is this instance.
///
/// A replay steps the instance through the vocabulary's own trait rather
/// than through its type, so a trait that answered differently would make
/// every recorded file agree with a run nobody performed.
#[test]
fn the_deciding_half_a_replay_drives_is_this_instance() {
    use stageman_vocabulary::Deciding;

    let (mut instance, effects) = <Instance as stageman_vocabulary::Deciding>::boot(
        seed(1),
        Simulation::environment(),
        TARGET,
    );
    let [Effect::Run { id, .. }, Effect::Bind { .. }] = effects.as_slice() else {
        panic!("a runtime question and a bind, and this is not them");
    };

    let caused = Deciding::step(
        &mut instance,
        Event::Ran {
            id: *id,
            finished: exited(),
        },
    );
    assert!(
        matches!(caused.as_slice(), [Effect::Read { .. }]),
        "the instance's own file is asked for through the trait"
    );
    assert_eq!(Deciding::snapshot(&instance), instance.snapshot());
    assert_eq!(
        Deciding::snapshot(&instance),
        serde_json::json!({ "booting": "file" })
    );
}

/// Everything an awake instance holds reads in full, credentials included.
///
/// This is what a scenario compares and a reviewer reads, so a snapshot
/// that quietly dropped a half would make every replay agree with itself
/// and with nothing else. The credentials are in the clear on purpose: the
/// kept state's own types refuse to serialise one, and every file a test
/// writes holds fakes.
#[test]
fn what_an_awake_instance_holds_reads_in_full() {
    let mut world = Simulation::new();
    world.holding(&watching(&[(job(1), Progress::Working)]));
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 1);

    let held = instance.snapshot();
    assert_eq!(held["held"]["key_source"], "STAGEMAN_KEY");
    assert_eq!(held["held"]["path"], "/sim/instance.json");
    assert_eq!(held["held"]["domain"], "localhost");
    assert_eq!(held["held"]["serving"], 8080);
    assert_eq!(held["held"]["announced"], true);

    let agents = held["kept"]["agents"].as_object().expect("the agents");
    assert!(
        agents
            .values()
            .all(|configured| configured["auth_token"].is_string()),
        "a credential reads as itself: {agents:?}"
    );
    let projects = held["kept"]["projects"].as_object().expect("the projects");
    let project = projects.values().next().expect("one project");
    assert_eq!(project["name"], "example");
    assert_eq!(project["repository"], "https://example.invalid/repo");
    assert!(
        !project["jobs"].as_object().expect("its jobs").is_empty(),
        "and the job it is watching: {project:?}"
    );
}

/// Waking waits for both addresses it was promised.
///
/// One is the dashboard's, which the entry point takes and tells it about;
/// the other is the tools', which it takes itself. An instance that woke on
/// either alone would tell a container where to reach tools on a port
/// nothing had taken — and would do it on the very first turn, before
/// anything could notice.
#[test]
fn waking_waits_for_every_address_it_was_promised() {
    let (mut instance, effects) = Instance::boot(seed(1), Simulation::environment(), TARGET);
    let [
        Effect::Run { id: version, .. },
        Effect::Bind { id: binding, .. },
    ] = effects.as_slice()
    else {
        panic!("a runtime question and a bind, and this is not them");
    };

    let asked = instance.step(Event::Ran {
        id: *version,
        finished: exited(),
    });
    let [Effect::Read { id: file, .. }] = asked.as_slice() else {
        panic!("the key is in the environment, so the file is next");
    };
    let asked = instance.step(Event::Read {
        id: *file,
        contents: Ok(None),
    });
    let [Effect::Run { id: all, .. }, Effect::Run { id: up, .. }] = asked.as_slice() else {
        panic!("both listings, asked at once");
    };
    instance.step(Event::Ran {
        id: *all,
        finished: exited(),
    });
    instance.step(Event::Ran {
        id: *up,
        finished: exited(),
    });
    instance.step(
        AppEvent::Serving {
            address: "127.0.0.1:8080".to_owned(),
            port: 8080,
        }
        .into(),
    );

    // Everything is known but the tools' address, and an answer to somebody
    // else's bind is not an answer to this one's.
    instance.step(Event::Bound {
        id: EffectId(9999),
        outcome: Ok(1234),
    });
    assert_eq!(
        instance.snapshot(),
        serde_json::json!({ "booting": "ready" }),
        "it has everything but the address it took itself"
    );

    instance.step(Event::Bound {
        id: *binding,
        outcome: Ok(47_999),
    });
    assert!(
        instance.snapshot().get("held").is_some(),
        "and that address wakes it: {}",
        instance.snapshot()
    );
}

/// One listing is not both.
///
/// Booting asks two questions of the runtime at once — everything left
/// behind, and what is up — and cannot place a container until both have
/// answered. Each carries its own identifier, and a phase that took
/// whichever arrived would walk on with one listing counted twice: every
/// container would look stopped, and the sweep would act on it.
#[test]
fn one_listing_is_not_both() {
    let stray = EffectId(9999);
    let (mut instance, effects) = Instance::boot(seed(1), Simulation::environment(), TARGET);

    // A runtime candidate, and the address the tools are served on.
    let [Effect::Run { id: version, .. }, Effect::Bind { .. }] = effects.as_slice() else {
        panic!("a runtime question and a bind, and this is not them");
    };
    let asked = instance.step(Event::Ran {
        id: *version,
        finished: exited(),
    });
    let [Effect::Read { id: file, .. }] = asked.as_slice() else {
        panic!("the key is in the environment, so the file is next");
    };
    let asked = instance.step(Event::Read {
        id: *file,
        contents: Ok(None),
    });
    let [Effect::Run { id: all, .. }, Effect::Run { id: up, .. }] = asked.as_slice() else {
        panic!("both listings, asked at once");
    };

    let listing = serde_json::json!({ "booting": "listing" });
    assert_eq!(instance.snapshot(), listing);
    assert!(
        instance
            .step(Event::Ran {
                id: stray,
                finished: exited(),
            })
            .is_empty()
    );
    assert_eq!(instance.snapshot(), listing, "a stray answer was counted");

    instance.step(Event::Ran {
        id: *all,
        finished: exited(),
    });
    assert_eq!(
        instance.snapshot(),
        listing,
        "one listing answered is not both"
    );

    instance.step(Event::Ran {
        id: *up,
        finished: exited(),
    });
    assert_eq!(
        instance.snapshot(),
        serde_json::json!({ "booting": "ready" }),
        "and both is everything booting was waiting on"
    );
}

/// A machine with nowhere to keep an instance says so and stops.
///
/// Before a runtime is asked anything, because a start that cannot keep what
/// it learns has no reason to learn it — and the message names the variable
/// that would fix it, since a machine with no home is usually a service
/// manager's idea of one rather than a mistake.
///
/// Recorded as a replay: the whole shape of a refusal, in four lines.
#[test]
fn nowhere_to_keep_an_instance_is_refused_before_anything_is_asked() {
    let mut world = Simulation::new();
    world.recording(
        "nowhere-to-keep-an-instance",
        "A machine with no home and nothing naming a file refuses at once, asking nothing",
    );
    let mut instance = world.wake_given(seed(1), Environment::new());
    world.run_until(&mut instance, 1);

    let refused = world.exited().expect("refused");
    assert!(refused.contains("no home directory"), "{refused}");
    assert!(refused.contains("STAGEMAN_STATE"), "{refused}");
    assert!(
        !world.shape().iter().any(|line| line.starts_with("-> Run")),
        "nothing is asked of a runtime: {:?}",
        world.shape()
    );
    assert!(world.printed().is_empty());
    world.recorded();
}
