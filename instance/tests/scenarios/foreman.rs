//! A project's foreman: messages taken and queued, turns run in order, and a
//! foreman interrupted mid-turn put back to work, against the simulated
//! world.

use crate::simulation::{
    Simulation, Talk, holding_a_message, in_thread, project, seed, watching_a_channel,
};
use stageman_agent::Command;
use stageman_foreman::Starting;

/// The foreman's conversations so far, in order.
fn runs(world: &Simulation) -> Vec<Talk> {
    world
        .talks_in(&stageman_foreman::container(project()))
        .into_iter()
        .cloned()
        .collect()
}

/// The first message begins a session: the person is told at once that it
/// is being worked on, the runtime is asked whether the foreman has a
/// container, and the turn opens with the foreman's opening and the message.
#[test]
fn a_first_message_opens_a_session_and_is_acknowledged_first() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    let shape = world.shape();
    let persisted = shape
        .iter()
        .position(|line| line.starts_with("<- Written"))
        .expect("the inbox was written");
    // Read back through the agent crate's inverse: looking at a foreman's
    // container is a label asked of the runtime like any other command.
    let inspected = world
        .first_asking(|command| matches!(command, Command::Label { .. }))
        .expect("the runtime was asked about the container");
    assert!(
        persisted < inspected,
        "nothing turns before the inbox lands: {shape:?}"
    );
    let runs = runs(&world);
    assert_eq!(runs.len(), 1, "{shape:?}");
    let run = &runs[0];
    assert!(run.began(), "no container, so a session is begun: {run:?}");
    assert!(run.was_told("You are the foreman for"), "{run:?}");
    assert!(run.was_told("look at the parser"), "{run:?}");
    assert!(world.exists(&stageman_foreman::container(project())));
    assert_eq!(
        world.posts(),
        [(in_thread(1), stageman_foreman::received_notice(0))],
        "acknowledged once, and told nothing when the turn ended"
    );
}

/// What a container's agent may see is decided here and nowhere else.
///
/// `docs/conventions.md` §3 asks that what a child process is handed be
/// constructed rather than inherited, and this is where the construction
/// happens: the variable a credential arrives in is the adapter's knowledge,
/// the decision about which credential is this instance's, and a turn that
/// began with an empty environment would run an agent authenticating as
/// whoever started the daemon — no error, no log line, and a bill.
#[test]
fn what_a_containers_agent_may_see_is_decided_here() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    let container = stageman_foreman::container(project());
    assert_eq!(runs(&world).len(), 1);
    let given = world
        .environment_of(&container)
        .expect("the container was made here");
    assert_eq!(
        given.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("agent-token"),
        "the agent's own credential, in the variable its adapter reads: {given:?}"
    );
    assert_eq!(given.len(), 1, "and nothing else: {given:?}");
}

/// A foreman with a container continues its session rather than opening one.
#[test]
fn a_foreman_with_a_container_continues_its_session() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let (name, held) = Simulation::ours(&stageman_foreman::container(project()));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    let runs = runs(&world);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].resumed(), "{:?}", runs[0]);
    assert!(
        !runs[0].was_told("You are the foreman for"),
        "{:?}",
        runs[0]
    );
    assert!(
        runs[0].was_told("A person said this to you on the channel"),
        "{:?}",
        runs[0]
    );
}

/// Messages arriving while the foreman works are queued in arrival order,
/// each told how many are ahead of it, and worked one after another.
#[test]
fn messages_arriving_while_it_works_are_queued_and_worked_in_order() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "first");
    world.says_at_root(200, 2, "second");
    world.says_at_root(300, 3, "third");
    world.run_until(&mut instance, 10_000);

    let runs = runs(&world);
    assert_eq!(runs.len(), 3, "{:?}", world.shape());
    for (run, said) in runs.iter().zip(["first", "second", "third"]) {
        assert!(run.was_told(said), "in arrival order: {run:?}");
    }
    assert_eq!(
        world.posts(),
        [
            (in_thread(1), stageman_foreman::received_notice(0)),
            (in_thread(2), stageman_foreman::received_notice(1)),
            (in_thread(3), stageman_foreman::received_notice(2)),
        ]
    );
    // Never two turns at once: each begins after the last ended.
    for pair in runs.windows(2) {
        let [earlier, later] = pair else {
            panic!("a pair");
        };
        assert!(
            earlier
                .ended_at
                .is_some_and(|ended| ended < later.opened_at),
            "a second turn started before the first ended: {runs:?}"
        );
    }
    assert!(runs.iter().all(|run| run.ended_at.is_some()));
}

/// A foreman found holding a message on waking is told its wait had a
/// reason, and its first turn is told it was interrupted.
#[test]
fn a_foreman_interrupted_mid_turn_is_picked_up_on_waking() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    holding_a_message(&mut state, 1, "look at the parser");
    holding_a_message(&mut state, 2, "and the tests");
    world.holding(&state);
    let (name, held) = Simulation::ours(&stageman_foreman::container(project()));
    world.container(&name, held);

    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10_000);

    let runs = runs(&world);
    assert_eq!(runs.len(), 2, "{:?}", world.shape());
    let interruption = stageman_foreman::asked(
        stageman_foreman::Turn {
            said: "look at the parser",
            starting: Starting::Interrupted,
        },
        &[],
    );
    let told_first = interruption
        .lines()
        .next()
        .expect("the interruption's first line");
    assert!(runs[0].was_told(told_first), "{:?}", runs[0]);
    assert!(
        !runs[1].was_told(told_first),
        "the next was never begun: {:?}",
        runs[1]
    );
    assert_eq!(
        world.posts(),
        [(in_thread(1), stageman_foreman::resumed_notice().to_owned())],
        "no message is re-acknowledged"
    );
}

/// A crash mid-turn is the same case, and the same seed gives the same trace.
#[test]
fn a_crash_mid_turn_picks_the_message_up_again() {
    fn scenario(seed_byte: u8) -> (Vec<String>, Vec<Talk>) {
        let mut world = Simulation::new();
        world.holding(&watching_a_channel(&[]));
        let mut instance = world.wake(seed(seed_byte));
        world.says_at_root(100, 1, "look at the parser");
        world.run_until(&mut instance, 200);
        let mut instance = world.crash(seed(seed_byte));
        world.run_until(&mut instance, 10_000);
        (world.shape(), runs(&world))
    }

    let (first, runs) = scenario(4);
    assert_eq!(first, scenario(4).0);
    assert_eq!(runs.len(), 2, "{first:?}");
    assert!(runs[0].began(), "{:?}", runs[0]);
    assert!(
        runs[1].resumed() && runs[1].was_told("You were interrupted"),
        "the container survived the crash, the session with it: {:?}",
        runs[1]
    );
}

/// A turn that could not be taken is said to be stuck, and the next message
/// is worked all the same.
#[test]
fn a_turn_that_fails_is_said_to_be_stuck_and_the_next_is_worked() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.next_turn_ends(Err("the agent would not start".to_owned()));

    world.says_at_root(100, 1, "first");
    world.says_at_root(200, 2, "second");
    world.run_until(&mut instance, 10_000);

    assert_eq!(runs(&world).len(), 2);
    assert_eq!(
        world.posts(),
        [
            (in_thread(1), stageman_foreman::received_notice(0)),
            (in_thread(2), stageman_foreman::received_notice(1)),
            (in_thread(1), stageman_foreman::stuck_notice().to_owned()),
        ]
    );
}

/// A foreman's warrant speaks in the thread of the message it is answering,
/// and lives as long as the turn.
#[test]
fn a_foremans_warrant_names_the_thread_of_the_message_it_answers() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 150);
    let warrant = world.warrants().last().expect("a warrant").clone();
    let warranted = instance
        .warranted(warrant.as_str())
        .expect("known while the turn runs");
    assert_eq!(
        warranted.speaker,
        stageman_instance::Speaker::Foreman(project())
    );
    assert_eq!(warranted.place, Some(in_thread(1)));

    world.run_until(&mut instance, 5_000);
    assert!(instance.warranted(warrant.as_str()).is_none());
}
