//! A job's narration posted at the root of its room as it happens, run
//! against the simulated world — see
//! `docs/decisions/0067-a-transcript-is-posted-where-its-speaker-owns-the-room.md`.

use crate::simulation::{Simulation, Utterance, in_room, job, seed, watching_a_channel};
use stageman_core::{JobId, Progress, Waiting};

/// An idle job with a room, put back to work by a reply at the room's root.
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

/// Each run of what the agent says is posted at the root of its room when
/// the run closes — a tool call closes one, the turn's end closes the last
/// — and the notice that the turn ended comes after all of it.
#[test]
fn a_jobs_narration_is_posted_per_run_at_the_root_in_order() {
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
        world.posts(),
        [
            (in_room(1), "Looking at the parser.".to_owned()),
            (in_room(1), "Fixed the flaky assertion.".to_owned()),
            (
                in_room(1),
                stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>")
            ),
        ],
        "two runs, nothing for the run with nothing in it, and the notice last"
    );
}

/// A room is posted one request at a time: the second run is not asked for
/// until the platform has answered the first, so posts land in order.
#[test]
fn a_run_is_posted_only_after_the_previous_post_is_answered() {
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
    let texts: Vec<&str> = posts.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "First.",
            "Second.",
            stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>").as_str(),
        ]
    );
    for pair in posts.windows(2) {
        let (before, after) = (pair[0].0, pair[1].0);
        assert!(
            answered.iter().any(|&at| before < at && at < after),
            "the post at {after} went before the one at {before} was answered: {:?}",
            world.shape()
        );
    }
}

/// A run longer than a post may be continues in a second one, in order.
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

    let posts = world.posts();
    assert_eq!(posts.len(), 3, "two pieces and the notice: {posts:?}");
    assert!(posts[0].1.chars().count() <= 12_000);
    assert_eq!(
        format!("{}\n{}", posts[0].1, posts[1].1),
        long,
        "nothing lost between the pieces"
    );
}
