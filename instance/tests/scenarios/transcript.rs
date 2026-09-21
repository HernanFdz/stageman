//! A job's transcript posted at the root of its room as it happens —
//! narration one message per run, working one burst per run, each grown in
//! place — run against the simulated world. See
//! `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.

use crate::simulation::{Simulation, Utterance, in_room, job, seed, watching_a_channel};
use stageman_core::{JobId, Progress, Waiting};

/// An idle job with a room, ready to be put back to work by a reply at the
/// room's root.
fn a_job_replied_to(world: &mut Simulation) -> (stageman_instance::Instance, JobId) {
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

/// What the room's root reads, in order.
fn root_reads(world: &Simulation) -> Vec<String> {
    world
        .posts()
        .iter()
        .map(|(place, text)| {
            assert_eq!(*place, in_room(1), "everything goes to the room's root");
            text.clone()
        })
        .collect()
}

fn the_notice() -> String {
    stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>")
}

/// Each run of narration is one message and each run of working one
/// burst, at the root in the order they happened, with the notice that the
/// turn ended after all of it. A run of narration that grew after its post
/// was answered reads whole in the end.
#[test]
fn narration_and_working_are_posted_per_run_at_the_root_in_order() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    world.next_turn_narrates(vec![
        Utterance::Says("Looking at "),
        Utterance::Says("the parser."),
        Utterance::Calls("cargo test"),
        Utterance::Calls("cargo test -- --nocapture"),
        Utterance::Says("Fixed the flaky assertion."),
    ]);

    world.says_in_room(100, 1, "use postgres");
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        root_reads(&world),
        [
            "Looking at the parser.".to_owned(),
            "⏳ ran `cargo test`\n⏳ ran `cargo test -- --nocapture`".to_owned(),
            "Fixed the flaky assertion.".to_owned(),
            the_notice(),
        ]
    );
}

/// A room is posted one request at a time: a message is not opened until
/// the platform has answered the previous post, so posts land in order.
#[test]
fn a_message_is_opened_only_after_the_previous_post_is_answered() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    world.next_turn_narrates(vec![
        Utterance::Says("First."),
        Utterance::Calls("cargo build"),
        Utterance::Says("Second."),
        Utterance::Calls("cargo test"),
    ]);

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 5_000);

    let posts = world.post_calls();
    let answered = world.responded_at();
    assert_eq!(posts.len(), 5, "{posts:?}");
    for pair in posts.windows(2) {
        let (before, after) = (pair[0].0, pair[1].0);
        assert!(
            answered.iter().any(|&at| before < at && at < after),
            "the post at {after} went before the one at {before} was answered: {:?}",
            world.shape()
        );
    }
}

/// A run longer than a post may be continues in a second one, in order,
/// and nothing is lost between them.
#[test]
fn a_long_run_continues_in_a_second_post() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    let line = "x".repeat(99);
    let long: &'static str = Box::leak(
        std::iter::repeat_n(line.as_str(), 130)
            .collect::<Vec<_>>()
            .join("\n")
            .into_boxed_str(),
    );
    world.next_turn_narrates(vec![Utterance::Says(long)]);

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 5_000);

    let posts = root_reads(&world);
    assert_eq!(posts.len(), 3, "two pieces and the notice: {posts:?}");
    assert!(posts[0].chars().count() <= 12_000);
    assert_eq!(format!("{}\n{}", posts[0], posts[1]), long);
}

/// A burst grows in place: a call ending marks its line, a call beginning
/// adds one, and the message is edited rather than posted again.
#[test]
fn a_burst_grows_in_place_as_calls_begin_and_end() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    world.next_turn_narrates_over(
        vec![
            Utterance::Says("Running the tests."),
            Utterance::Calls("cargo test"),
            Utterance::Done("cargo test"),
            Utterance::Calls("cargo build"),
            Utterance::Fails("cargo build"),
            Utterance::Says("Red."),
        ],
        1_500,
    );

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 20_000);

    assert_eq!(
        root_reads(&world),
        [
            "Running the tests.".to_owned(),
            "✅ ran `cargo test`\n❌ ran `cargo build`".to_owned(),
            "Red.".to_owned(),
            the_notice(),
        ],
        "four messages, the burst grown rather than posted again"
    );
    assert!(
        !world.edits().is_empty(),
        "the burst grew by editing: {:?}",
        world.shape()
    );
}

/// Growth is paced: a message that grows a piece at a time is edited at
/// most every pace, not once per piece, and reads whole in the end.
#[test]
fn growth_is_paced() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    let pieces: Vec<Utterance> = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"]
        .into_iter()
        .map(Utterance::Says)
        .collect();
    world.next_turn_narrates_over(pieces, 200);

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 20_000);

    assert_eq!(
        root_reads(&world),
        ["abcdefghijkl".to_owned(), the_notice()],
        "one message, read whole"
    );
    assert!(
        world.edits().len() <= 3,
        "twelve pieces over two seconds are a couple of edits and the last, not twelve: {:?}",
        world.edits()
    );
}

/// A call that ends after its burst closed still marks its line, since the
/// burst is kept until every call in it has ended.
#[test]
fn a_call_ending_after_its_burst_closed_still_marks_the_line() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    world.next_turn_narrates(vec![
        Utterance::Calls("cargo test"),
        Utterance::Says("Meanwhile, the docs."),
        Utterance::Done("cargo test"),
    ]);

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        root_reads(&world),
        [
            "✅ ran `cargo test`".to_owned(),
            "Meanwhile, the docs.".to_owned(),
            the_notice(),
        ]
    );
}

/// A thought, where an adapter carries any, is a quoted line in the burst
/// beside what the agent did.
#[test]
fn a_thought_is_a_quoted_line_in_the_burst() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_replied_to(&mut world);
    world.next_turn_narrates(vec![
        Utterance::Thinks("The tests "),
        Utterance::Thinks("first."),
        Utterance::Calls("ls"),
    ]);

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        root_reads(&world),
        [
            "> 💭 The tests first.\n⏳ ran `ls`".to_owned(),
            the_notice()
        ]
    );
}
