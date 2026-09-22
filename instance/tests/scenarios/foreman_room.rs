//! A project's foreman has a room of its own, made before its first turn,
//! where its transcript is posted — see
//! `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.

use crate::simulation::{
    CHANNEL, SAID_IN_ROOM, Simulation, Utterance, in_room, in_thread, link_to, project, request,
    room, seed, thread, watching_a_channel,
};
use stageman_core::{Place, Room};
use stageman_instance::Request;

/// The room a foreman's transcript goes to, once made: the first room the
/// simulation makes.
fn the_foremans_room() -> Room {
    room(1)
}

/// The first message makes the room before anything turns: named for the
/// project, described, opened, and recorded, with the turn's transcript
/// posted at its root and the answer still where the person asked.
#[test]
fn the_first_message_makes_the_foremans_room_before_it_turns() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    let shape = world.shape();
    let made = shape
        .iter()
        .position(|line| line.starts_with("<- Responded"))
        .expect("the platform answered about the room");
    let inspected = world
        .first_asking(|command| matches!(command, stageman_agent::Command::Label { .. }))
        .expect("the runtime was asked about the container");
    assert!(made < inspected, "the room before the turn: {shape:?}");

    let rooms = world.rooms();
    assert_eq!(rooms.len(), 1, "{rooms:?}");
    assert!(
        rooms[0].1.starts_with("example--foreman--"),
        "named for the project: {rooms:?}"
    );
    assert_eq!(
        instance
            .state()
            .projects
            .get(&project())
            .and_then(|watched| watched.foreman_room.clone()),
        Some(the_foremans_room()),
        "recorded on the project"
    );
    assert_eq!(
        world.described(),
        [(
            the_foremans_room().id,
            stageman_foreman::foreman_room_purpose("example"),
        )]
    );
    assert_eq!(
        world.posts(),
        [
            (
                in_room(1),
                stageman_foreman::foreman_room_opening("example", "<@U0BOT>")
            ),
            (
                in_room(1),
                format!("▶️ Handling {}.", link_to(CHANNEL, &thread(1).id, None))
            ),
            (in_room(1), "done".to_owned()),
            (
                in_thread(1),
                stageman_foreman::handled_elsewhere_notice(Some("<#C-job-001>"))
            ),
        ],
        "the opening, why the turn started, and what the foreman said, at the root of its \
         room; the person's thread signposted, since the simulated foreman never calls the tool"
    );
}

/// A second message finds the room made and does not make another.
#[test]
fn the_room_is_made_once() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);
    world.says_at_root(6_000, 2, "and the lexer");
    world.run_until(&mut instance, 12_000);

    assert_eq!(world.rooms().len(), 1, "{:?}", world.rooms());
    assert_eq!(
        world
            .posts()
            .iter()
            .filter(|(place, text)| *place == in_room(1) && text == "done")
            .count(),
        2,
        "both turns' transcripts in the one room: {:?}",
        world.posts()
    );
}

/// A platform that will not make the room does not stop the turn: the
/// message is handled, and the transcript is let go.
#[test]
fn a_room_that_cannot_be_made_does_not_stop_the_turn() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.next_room_fails("name_taken");

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    assert_eq!(world.talks().len(), 1, "the turn ran: {:?}", world.shape());
    assert!(
        instance
            .state()
            .projects
            .get(&project())
            .is_some_and(|watched| watched.foreman_room.is_none()),
        "nothing recorded"
    );
    assert_eq!(
        world.posts(),
        [(
            in_thread(1),
            stageman_foreman::handled_elsewhere_notice(None)
        )],
        "signposted with nowhere to point"
    );
}

/// A person's mention in the foreman's own room reaches the foreman, as a
/// mention anywhere that is no job's does, and is answered where said.
#[test]
fn a_mention_in_the_foremans_room_reaches_the_foreman() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    world.next_turn_narrates(vec![Utterance::Says("Reading it back.")]);
    world.says_in_room(6_000, 1, "why did you decide that?");
    world.run_until(&mut instance, 12_000);

    assert_eq!(world.talks().len(), 2, "{:?}", world.shape());
    assert!(
        world
            .posts()
            .contains(&(in_room(1), "Reading it back.".to_owned())),
        "the second turn's transcript, in the same room: {:?}",
        world.posts()
    );
    assert_eq!(
        world.posts().last(),
        Some(&(
            Place {
                room: room(1),
                thread: Some(SAID_IN_ROOM.to_owned()),
            },
            stageman_foreman::handled_elsewhere_notice(Some("<#C-job-001>"))
        )),
        "and the question's thread signposted, since the simulated foreman answers nowhere"
    );
    assert_eq!(
        world.rooms().len(),
        1,
        "and no second room: {:?}",
        world.rooms()
    );
}

/// Forgetting the project archives its foreman's room, before the record
/// that names it goes.
#[test]
fn forgetting_the_project_archives_the_foremans_room() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);
    assert_eq!(world.rooms().len(), 1, "{:?}", world.rooms());

    for effect in instance.step(request(
        7,
        Request::Forget {
            project: project().to_string(),
        },
    )) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 6_000);

    assert_eq!(
        world.archived(),
        [the_foremans_room().id],
        "the foreman's room, and nothing else to archive"
    );
    assert!(instance.state().projects.is_empty());
}
