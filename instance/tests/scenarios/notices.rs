//! The notices around a turn: the line at the root saying why it started,
//! and the signpost in a thread that got no answer — see
//! `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.

use crate::simulation::{
    CHANNEL, SAID_IN_ROOM, SAID_IN_ROOMS_THREAD, Simulation, in_room, in_thread, job, link_to,
    room, seed, thread, watching_a_channel, watching_a_room,
};
use stageman_core::{JobId, Place, Progress, Waiting};

/// An idle job with a room, ready to be put back to work.
fn a_job_with_a_room(world: &mut Simulation) -> (stageman_instance::Instance, JobId) {
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Asked),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);
    (instance, idle)
}

fn say(message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "say", "arguments": {"message": message}},
    })
}

/// A reply at the root of a job's room: the root is told why the turn
/// started, with a link to the reply, before the agent says anything; a
/// reply at the root needs no signpost, since the answer lands beside it.
#[test]
fn a_reply_at_the_root_is_noticed_before_the_agent_speaks() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);

    world.says_in_room(100, 1, "use postgres");
    world.run_until(&mut instance, 5_000);

    let link = link_to(&room(1).id, SAID_IN_ROOM, None);
    assert_eq!(
        world.posts(),
        [
            (in_room(1), format!("▶️ Handling {link}.")),
            (in_room(1), "done".to_owned()),
            (
                in_room(1),
                stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>")
            ),
        ]
    );
}

/// A reply in a thread that the agent never answers in is signposted to
/// the root, where its transcript went; the notice at the root links the
/// reply in its thread.
#[test]
fn a_thread_the_job_never_answered_in_is_signposted() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);

    world.says_in_rooms_thread(100, 1, "1788000000.500000", "use postgres");
    world.run_until(&mut instance, 5_000);

    let asked_in = Place {
        room: room(1),
        thread: Some("1788000000.500000".to_owned()),
    };
    let link = link_to(&room(1).id, SAID_IN_ROOMS_THREAD, Some("1788000000.500000"));
    assert_eq!(
        world.posts(),
        [
            (in_room(1), format!("▶️ Handling {link}.")),
            (in_room(1), "done".to_owned()),
            (
                in_room(1),
                stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>")
            ),
            (
                asked_in,
                stageman_foreman::answered_elsewhere_notice("<#C-job-001>")
            ),
        ]
    );
}

/// A thread the agent answered in through the tool is not signposted.
#[test]
fn a_thread_the_job_answered_in_is_not_signposted() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);

    world.says_in_rooms_thread(100, 1, "1788000000.500000", "use postgres");
    world.run_until(&mut instance, 150);
    let warrant = world.warrants().last().expect("the turn's warrant").clone();
    world.calls(160, &warrant, &say("On it."));
    world.run_until(&mut instance, 5_000);

    let asked_in = Place {
        room: room(1),
        thread: Some("1788000000.500000".to_owned()),
    };
    assert!(
        world
            .posts()
            .contains(&(asked_in.clone(), "On it.".to_owned())),
        "{:?}",
        world.posts()
    );
    assert!(
        !world
            .posts()
            .iter()
            .any(|(place, text)| *place == asked_in && text.starts_with("↩️")),
        "no signpost where the answer already is: {:?}",
        world.posts()
    );
}

/// A foreman's turn is noticed at the root of its room with a link to the
/// mention, and a person it answered nowhere is signposted to that room.
#[test]
fn a_foremans_turn_is_noticed_in_its_room_and_a_silent_answer_is_signposted() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    let link = link_to(CHANNEL, &thread(1).id, None);
    assert_eq!(
        world.posts(),
        [
            (
                in_room(1),
                stageman_foreman::foreman_room_opening("example", "<@U0BOT>")
            ),
            (in_room(1), format!("▶️ Handling {link}.")),
            (in_room(1), "done".to_owned()),
            (
                in_thread(1),
                stageman_foreman::handled_elsewhere_notice(Some("<#C-job-001>"))
            ),
        ]
    );
}

/// A signal is noticed as the app's, and never signposted: silence under a
/// signal is the decision 0063 keeps.
#[test]
fn a_signal_is_noticed_as_the_apps_and_never_signposted() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    watching_a_room(&mut state, CHANNEL);
    world.holding(&state);
    let mut instance = world.wake(seed(1));

    world.app_posts(100, CHANNEL, "1788000000.000001", "#9 An issue", "opened");
    world.run_until(&mut instance, 5_000);

    let link = link_to(CHANNEL, "1788000000.000001", None);
    assert!(
        world.posts().contains(&(
            in_room(1),
            format!("▶️ Judging what GitHub posted: {link}.")
        )),
        "{:?}",
        world.posts()
    );
    assert!(
        world
            .posts()
            .iter()
            .all(|(place, _)| place.thread.is_none()),
        "nothing under the signal: {:?}",
        world.posts()
    );
}
