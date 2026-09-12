//! Booting, against the simulated world: what an instance asks before it is
//! awake, in what order, and how it refuses.

use stageman_core::Progress;

use crate::simulation::{Simulation, job, seed, watching};

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
    let mut instance = world.wake(seed(1));
    assert!(world.disk().is_none(), "not yet: the write has not landed");

    world.run_until(&mut instance, 1);
    assert!(world.disk().is_some(), "an instance with nothing in it");
    assert_eq!(world.printed().len(), 1);
    assert!(instance.id().is_some(), "and it has an identity");
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
