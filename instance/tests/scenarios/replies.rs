//! A reply arriving on a job's thread, run against the simulated world.

use stageman_channel::Reaction;

use crate::simulation::{
    CHANNEL, SAID_IN_ROOM, SAID_IN_ROOMS_THREAD, Simulation, Spoken, in_room, job, link_to, room,
    seed, watching_a_channel,
};
use stageman_core::{JobId, Outcome, Place, Progress, State, Waiting};

fn progress_of(state: &State, id: JobId) -> Progress {
    state.job(id).expect("the job").progress.clone()
}

/// A reply to an idle job resumes it with the person's words framed as a
/// reply, once the record that it is working has landed, and its thread is
/// told when the turn ends.
#[test]
fn a_reply_to_an_idle_job_resumes_it_after_the_record_lands() {
    let mut world = Simulation::new();
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

    world.says_in_room(100, 1, "use postgres");
    world.run_until(&mut instance, 5_000);

    let shape = world.shape();
    let persisted = shape
        .iter()
        .position(|line| line.starts_with("<- Written"))
        .expect("the record was written");
    let resumed = world.first_turn().expect("the job was resumed");
    assert!(
        persisted < resumed,
        "the turn waits for the record to land: {shape:?}"
    );
    let run = world.talks().last().expect("the agent was spoken to");
    assert!(run.was_told("A person replied on the channel:"), "{run:?}");
    assert!(run.was_told("use postgres"), "{run:?}");
    assert_eq!(
        progress_of(instance.state(), idle),
        Progress::Idle(Waiting::Silent)
    );
    assert_eq!(
        world.posts(),
        [
            (
                in_room(1),
                format!("▶️ Handling {}.", link_to(&room(1).id, SAID_IN_ROOM, None))
            ),
            (in_room(1), "done".to_owned()),
            (
                in_room(1),
                stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>", &[])
            ),
        ],
        "why the turn started, what the agent said, and that it ended, at the root in order"
    );
}

/// A turn dropped because its record did not land forgets its own warrant
/// and nobody else's: the other speaker's goes on answering.
#[test]
fn a_dropped_turn_forgets_only_its_own_warrant() {
    let mut world = Simulation::new();
    let idle = job(1);
    let running = job(2);
    world.holding(&watching_a_channel(&[
        (idle, Progress::Idle(Waiting::Asked), 1),
        (running, Progress::Working, 2),
    ]));
    for which in [idle, running] {
        let (name, held) = Simulation::ours(&stageman_job::container(which));
        world.container(&name, held);
    }
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);
    let kept = world
        .warrants()
        .last()
        .cloned()
        .expect("the resumed job's warrant");
    assert!(instance.warranted(kept.as_str()).is_some());

    // The reply is taken, and the write that would let it resume fails.
    world.next_write_fails("the disk is full");
    world.says_in_room(100, 1, "use postgres");
    world.run_until(&mut instance, 200);

    assert!(
        matches!(
            progress_of(instance.state(), idle),
            Progress::Idle(Waiting::Failed(_))
        ),
        "the turn that was not started is recorded as such"
    );
    assert!(
        instance.warranted(kept.as_str()).is_some(),
        "the other speaker's warrant still answers"
    );
    assert_eq!(progress_of(instance.state(), running), Progress::Working);
}

// What a message to a working job leads to, and two arriving together, are
// the inbox's since
// `docs/decisions/0069-a-message-reaches-a-working-job.md`: see
// `scenarios/inbox.rs`.

/// A reply to a job that is over is refused with the notice that says so.
#[test]
fn a_reply_to_a_job_that_is_over_is_refused_with_its_own_notice() {
    let mut world = Simulation::new();
    let over = job(1);
    world.holding(&watching_a_channel(&[(
        over,
        Progress::Retired(Outcome::Done),
        1,
    )]));
    let mut instance = world.wake(seed(1));

    world.says_in_room(100, 1, "one more thing");
    world.run_until(&mut instance, 200);

    assert_eq!(
        world.posts(),
        [(in_room(1), stageman_foreman::over_notice().to_owned())]
    );
    assert_eq!(
        progress_of(instance.state(), over),
        Progress::Retired(Outcome::Done),
        "a verdict is never overwritten"
    );
}

/// A mention inside a thread of a job's room reaches the job, and the turn
/// answers in that thread — its warrant names it — while the notice that
/// it ended goes to the root of the room, because that is about the job
/// rather than part of the exchange.
#[test]
fn a_mention_in_a_thread_of_a_jobs_room_is_answered_there_and_noticed_at_the_root() {
    let mut world = Simulation::new();
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Silent),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));

    world.says_in_rooms_thread(100, 1, "1788000000.500000", "use postgres");
    world.run_until(&mut instance, 150);
    let warrant = world.warrants().last().expect("the job's warrant").clone();
    let warranted = instance
        .warranted(warrant.as_str())
        .expect("known while the turn runs");
    assert_eq!(
        warranted.place,
        Some(Place {
            room: room(1),
            thread: Some("1788000000.500000".to_owned()),
        }),
        "the job answers where it was asked"
    );

    world.run_until(&mut instance, 5_000);
    assert!(
        world
            .talks()
            .iter()
            .any(|talk| talk.was_told("use postgres")),
        "{:?}",
        world.talks()
    );
    assert_eq!(
        world.posts(),
        [
            (
                in_room(1),
                format!(
                    "▶️ Handling {}.",
                    link_to(&room(1).id, SAID_IN_ROOMS_THREAD, Some("1788000000.500000"))
                )
            ),
            (in_room(1), "done".to_owned()),
            (
                in_room(1),
                stageman_foreman::stopped_notice(&Waiting::Silent, None, "<@U0BOT>", &[])
            ),
            (
                Place {
                    room: room(1),
                    thread: Some("1788000000.500000".to_owned()),
                },
                stageman_foreman::answered_elsewhere_notice("<#C-job-001>")
            ),
        ],
        "why it started, what the agent said and that it ended go to the root, whatever thread \
         the exchange was in; the thread that got no answer is signposted there"
    );
}

/// A mention in a thread that belongs to no job reaches the foreman, and is
/// acknowledged in that thread — which is where a person lands by replying
/// to something the foreman said. The idle job beside it is left alone.
#[test]
fn a_mention_in_a_thread_belonging_to_no_job_reaches_the_foreman() {
    let mut world = Simulation::new();
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Silent),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));

    world.says_in(100, 7, "hello?");
    world.run_until(&mut instance, 5_000);

    let in_threads: Vec<_> = world
        .posts()
        .iter()
        .filter(|(place, _)| place.thread.is_some())
        .collect();
    assert!(
        in_threads.len() == 1 && in_threads[0].1.starts_with("↩️"),
        "the thread is only signposted, since the simulated foreman answers nowhere: {:?}",
        world.posts()
    );
    assert_eq!(
        world.reactions().first(),
        Some(&(
            CHANNEL.to_owned(),
            "1788000099.000001".to_owned(),
            Reaction::Seen
        )),
        "seen where it was said"
    );
    assert!(
        world.talks().iter().any(|talk| talk.was_told("hello?")),
        "the foreman was given it: {:?}",
        world.talks()
    );
    assert_eq!(
        progress_of(instance.state(), idle),
        Progress::Idle(Waiting::Silent),
        "not the job's thread, so not the job's"
    );
}

/// What is not a person's mention reaches nobody: people talking to each
/// other, something this instance said, and the copy of a mention that the
/// message subscription delivers beside the mention event.
#[test]
fn what_is_not_a_persons_mention_reaches_nobody() {
    let mut world = Simulation::new();
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Silent),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));

    let plain = world.said_in_room(1, "people talking to each other", Spoken::Plain);
    let ours = world.said_in_room(1, "<@U0BOT> something this instance posted", Spoken::Ours);
    let copy = world.said_in_room(1, "<@U0BOT> use postgres", Spoken::Plain);
    world.schedule(100, plain);
    world.schedule(101, ours);
    world.schedule(102, copy);
    world.run_until(&mut instance, 200);

    assert!(world.posts().is_empty());
    assert!(world.talks().is_empty(), "{:?}", world.talks());
    assert_eq!(
        progress_of(instance.state(), idle),
        Progress::Idle(Waiting::Silent)
    );
}

/// A crash between a reply being taken and its record landing loses the
/// reply: on waking the job is resumed with the resumption notice instead,
/// which is what today's daemon does too.
#[test]
fn a_crash_before_the_record_lands_loses_the_reply_but_not_the_job() {
    let mut world = Simulation::new();
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Silent),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));

    world.says_in_room(100, 1, "use postgres");
    // The reply is taken at 100 and its write lands at 101; the daemon dies
    // in between.
    world.run_until(&mut instance, 100);
    assert_eq!(progress_of(instance.state(), idle), Progress::Working);
    let mut instance = world.crash(seed(2));

    assert_eq!(
        progress_of(instance.state(), idle),
        Progress::Idle(Waiting::Silent),
        "the disk never learned of the reply"
    );
    world.run_until(&mut instance, 5_000);
    assert!(
        !world
            .talks()
            .iter()
            .any(|talk| talk.was_told("use postgres")),
        "{:?}",
        world.talks()
    );
}

/// A write that fails after a reply was taken fails the turn before it
/// starts, records why, and leaves the job able to take the next reply.
#[test]
fn a_failed_write_fails_the_turn_before_it_starts() {
    let mut world = Simulation::new();
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Silent),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));
    world.next_write_fails("the disk is full");

    world.says_in_room(100, 1, "use postgres");
    world.run_until(&mut instance, 5_000);

    assert!(world.talks().is_empty(), "{:?}", world.talks());
    let Progress::Idle(Waiting::Failed(why)) = progress_of(instance.state(), idle) else {
        panic!("the turn that never started is a failure");
    };
    assert!(why.contains("could not be written"), "{why}");
    assert_eq!(
        world.disk().map(|state| progress_of(&state, idle)),
        Some(Progress::Idle(Waiting::Failed(why))),
        "the next write carried the record"
    );
}
