//! A mention in a thread shown that thread before its turn starts, run
//! against the simulated world — see
//! `docs/decisions/0068-a-mention-is-shown-its-thread.md`.

use crate::simulation::{
    CHANNEL, Simulation, job, project, room, seed, thread, watching_a_channel, watching_a_room,
};
use stageman_channel::Call;
use stageman_core::{JobId, Progress, Waiting};
use stageman_foreman::{Shown, Voice};

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

/// The parent of a thread in a job's room.
const PARENT: &str = "1788000000.500000";

/// A thread in a job's room, as the platform remembers it: a person's
/// question at the root, which mentioned the job; what somebody said without
/// a mention while the job was answering; the job's answer; and an untagged
/// reply after it.
///
/// Said as plain messages, the question included, so that nothing here
/// starts a turn: a person is read from the mention event and from nothing
/// else, and the platform remembers the markup either way.
fn a_thread_in_the_jobs_room(world: &mut Simulation) {
    let room = room(1).id;
    world.person_says(
        20,
        &room,
        PARENT,
        None,
        "<@U0BOT> which database should this use?",
    );
    world.person_says(
        25,
        &room,
        "1788000000.500001",
        Some(PARENT),
        "Staging is down, by the way.",
    );
    world.we_said(
        30,
        &room,
        "1788000000.500002",
        Some(PARENT),
        "Two options: Postgres or SQLite.",
    );
    world.person_says(
        40,
        &room,
        "1788000000.500003",
        Some(PARENT),
        "Postgres, I think.",
    );
}

/// A mention in a thread is shown the thread first, once the record that
/// the job is working has landed: the parent, and everything from the last
/// message the job was given there, each with who said it and its
/// identifier. What was said without a mention while that turn ran is among
/// it, though the job posted after it, and so are the job's own words, so
/// that the rest reads in order.
#[test]
fn a_mention_in_a_thread_is_shown_everything_from_the_last_message_it_was_given() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);
    a_thread_in_the_jobs_room(&mut world);

    world.says_in_rooms_thread(100, 1, PARENT, "go with that");
    world.run_until(&mut instance, 5_000);

    let shape = world.shape();
    let persisted = shape
        .iter()
        .position(|line| line.starts_with("<- Written"))
        .expect("the record was written");
    let read = world
        .first_call(|call| matches!(call, Call::Replies { .. }))
        .expect("the thread was asked for");
    let turned = world.first_turn().expect("the job was resumed");
    assert!(
        persisted < read && read < turned,
        "the record, then the thread, then the turn: {shape:?}"
    );
    // The whole of what the job was told: the question it was given, what
    // was said while it answered, its own answer and the untagged reply, by
    // who said each and its identifier, in order; and the mention after
    // them, framed as the reply it is rather than shown among what came
    // before.
    let shown = [
        Shown {
            id: "C-job-001/1788000000.500000",
            voice: Voice::Person("<@U0HUMAN>"),
            text: "<@U0BOT> which database should this use?",
        },
        Shown {
            id: "C-job-001/1788000000.500001",
            voice: Voice::Person("<@U0HUMAN>"),
            text: "Staging is down, by the way.",
        },
        Shown {
            id: "C-job-001/1788000000.500002",
            voice: Voice::Us,
            text: "Two options: Postgres or SQLite.",
        },
        Shown {
            id: "C-job-001/1788000000.500003",
            voice: Voice::Person("<@U0HUMAN>"),
            text: "Postgres, I think.",
        },
    ];
    let run = world.talks().last().expect("the agent was spoken to");
    assert_eq!(
        run.prompt.as_deref(),
        Some(
            stageman_foreman::reply(
                "<@U0BOT> go with that",
                "C-job-001/1788000000.500000",
                Some(&stageman_foreman::thread_shown(&shown, true, false)),
                stageman_foreman::Finding::AtRest,
            )
            .as_str()
        )
    );
}

/// A mention at the root asks for nothing.
#[test]
fn a_mention_at_the_root_is_shown_nothing() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);

    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 5_000);

    assert!(
        world
            .first_call(|call| matches!(call, Call::Replies { .. }))
            .is_none(),
        "{:?}",
        world.shape()
    );
    let run = world.talks().last().expect("the agent was spoken to");
    assert!(!run.was_told("This was said in a thread"), "{run:?}");
}

/// A thread that cannot be read does not stop the turn: the frame says so.
#[test]
fn a_thread_that_cannot_be_read_does_not_stop_the_turn() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);
    a_thread_in_the_jobs_room(&mut world);
    world.next_thread_read_fails("thread_not_found");

    world.says_in_rooms_thread(100, 1, PARENT, "go with that");
    world.run_until(&mut instance, 5_000);

    let run = world.talks().last().expect("the agent was spoken to");
    assert!(run.was_told(stageman_foreman::thread_unread()), "{run:?}");
    assert!(run.was_told("go with that"), "{run:?}");
    assert!(!run.was_told("Which database"), "nothing was read: {run:?}");
}

/// A foreman beginning a session is shown the whole thread, its own earlier
/// words included, since a fresh session remembers none of it.
#[test]
fn a_fresh_foreman_is_shown_the_whole_thread() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);
    let parent = thread(7).id;
    world.person_says(20, CHANNEL, &parent, None, "Should we bump the toolchain?");
    world.we_said(30, CHANNEL, "1788000000.000070", Some(&parent), "Not yet.");
    world.person_says(
        40,
        CHANNEL,
        "1788000000.000071",
        Some(&parent),
        "Rust 1.95 is out now.",
    );

    world.says_in(100, 7, "what do you think now?");
    world.run_until(&mut instance, 5_000);

    let run = world
        .talks_in(&stageman_foreman::container(project()))
        .into_iter()
        .last()
        .expect("the foreman was spoken to");
    assert!(run.began(), "a fresh session: {run:?}");
    assert!(
        run.was_told("What was said there before it, oldest first"),
        "{run:?}"
    );
    assert!(
        run.was_told("<@U0HUMAN> (C0123456789/1788000000.000007):\nShould we bump the toolchain?"),
        "{run:?}"
    );
    assert!(
        run.was_told("You (C0123456789/1788000000.000070):\nNot yet."),
        "its own earlier words, shown as its own: {run:?}"
    );
    assert!(run.was_told("Rust 1.95 is out now."), "{run:?}");
    assert!(
        run.was_told(
            "A person said this to you on the channel:\n\n<@U0BOT> what do you think now?"
        ),
        "{run:?}"
    );
}

/// A thread longer than what is asked for is shown its most recent
/// messages, and told that it was longer.
#[test]
fn a_longer_thread_says_so() {
    let mut world = Simulation::new();
    let (mut instance, _) = a_job_with_a_room(&mut world);
    let room = room(1).id;
    world.person_says(20, &room, PARENT, None, "The first.");
    for n in 0..60_u32 {
        let id = format!("1788000000.6{n:05}");
        world.person_says(21, &room, &id, Some(PARENT), &format!("Reply {n}."));
    }

    world.says_in_rooms_thread(100, 1, PARENT, "and now?");
    world.run_until(&mut instance, 5_000);

    let run = world.talks().last().expect("the agent was spoken to");
    assert!(run.was_told("The thread is longer than this"), "{run:?}");
    assert!(
        run.was_told("Reply 59."),
        "the most recent are shown: {run:?}"
    );
    assert!(!run.was_told("Reply 5.\n"), "the oldest are not: {run:?}");
    assert!(
        run.was_told("The first."),
        "the parent is shown regardless: {run:?}"
    );
}

/// A reply in a thread whose record does not land asks for no thread and
/// starts no turn: the job is recorded as failed for that reason, as a turn
/// never started is.
#[test]
fn a_thread_is_not_asked_for_when_the_record_did_not_land() {
    let mut world = Simulation::new();
    let (mut instance, idle) = a_job_with_a_room(&mut world);
    a_thread_in_the_jobs_room(&mut world);

    world.next_write_fails("the disk is full");
    world.says_in_rooms_thread(100, 1, PARENT, "go with that");
    world.run_until(&mut instance, 5_000);

    assert!(
        world
            .first_call(|call| matches!(call, Call::Replies { .. }))
            .is_none(),
        "{:?}",
        world.shape()
    );
    assert!(world.first_turn().is_none(), "{:?}", world.shape());
    assert!(
        matches!(
            instance.state().job(idle).expect("the job").progress,
            Progress::Idle(Waiting::Failed(_))
        ),
        "the turn that was not started is recorded as such"
    );
}

/// A signal in a thread is shown the thread the same way: a follow-up is
/// told the card it follows, by the app's name, to a foreman whose session
/// is resumed and never spoke there.
#[test]
fn a_signal_in_a_thread_is_shown_the_card_it_follows() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    watching_a_room(&mut state, CHANNEL);
    world.holding(&state);
    let mut instance = world.wake(seed(1));

    world.app_posts(
        100,
        CHANNEL,
        "1788000000.000100",
        "Issue opened by somebody",
        "The parser is flaky on Windows.",
    );
    world.run_until(&mut instance, 2_000);
    world.app_follows_up(
        3_000,
        CHANNEL,
        "1788000000.000200",
        "1788000000.000100",
        "Issue closed as completed by somebody",
    );
    world.run_until(&mut instance, 8_000);

    let runs = world.talks_in(&stageman_foreman::container(project()));
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(
        !runs[0].was_told("This was said in a thread"),
        "a card at the root is its own context: {:?}",
        runs[0]
    );
    let follow_up = runs[1];
    assert!(follow_up.resumed(), "{follow_up:?}");
    assert!(
        follow_up.was_told(
            "This was said in a thread. What was said there before it, oldest first, each with \
             who said it and its identifier:\n\nGitHub (C0123456789/1788000000.000100):\nIssue \
             opened by somebody"
        ),
        "the card it follows, by the app's name: {follow_up:?}"
    );
    assert!(
        follow_up.was_told("The parser is flaky on Windows."),
        "{follow_up:?}"
    );
    assert!(
        follow_up.was_told("GitHub posted this in a room you watch:"),
        "{follow_up:?}"
    );
}
