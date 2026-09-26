//! Another app heard in a watched room, run against the simulated
//! platform: a signal for the foreman, framed as the app's; a room that is
//! not watched; a follow-up in its thread; a job started from a signal; the
//! tools that watch and stop watching; and the brief said every turn.

use stageman_channel::Reaction;
use stageman_core::{Channel, JobId, Place, Progress, Room};
use stageman_instance::{Request, Response};

use crate::simulation::{
    CHANNEL, Simulation, Talk, briefed, job, project, request, seed, thread, watching_a_channel,
    watching_a_room,
};

/// The foreman's conversations so far, in order.
fn runs(world: &Simulation) -> Vec<Talk> {
    world
        .talks_in(&stageman_foreman::container(project()))
        .into_iter()
        .cloned()
        .collect()
}

/// A tool call's body, as an agent's client would send one.
pub fn call(name: &str, arguments: serde_json::Value) -> serde_json::Value {
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), name.into());
    params.insert("arguments".to_owned(), arguments);
    let mut envelope = serde_json::Map::new();
    envelope.insert("jsonrpc".to_owned(), "2.0".into());
    envelope.insert("id".to_owned(), 1.into());
    envelope.insert("method".to_owned(), "tools/call".into());
    envelope.insert("params".to_owned(), serde_json::Value::Object(params));
    serde_json::Value::Object(envelope)
}

/// The text a tool answered with.
pub fn text_of(answer: &(u16, Option<serde_json::Value>)) -> String {
    answer
        .1
        .as_ref()
        .expect("a body")
        .pointer("/result/content/0/text")
        .and_then(serde_json::Value::as_str)
        .expect("text")
        .to_owned()
}

/// The room people talk in, as the domain names it.
fn the_room() -> Room {
    Room {
        channel: Channel::Slack,
        id: CHANNEL.to_owned(),
    }
}

/// The rooms the project watches, as the instance holds them.
fn watched(instance: &stageman_instance::Instance) -> Vec<Room> {
    instance
        .state()
        .projects
        .get(&project())
        .expect("the project")
        .watched
        .iter()
        .cloned()
        .collect()
}

/// An app's message in a watched room is a signal for the foreman: framed as
/// the app's, received with the same reactions a person's message gets, and
/// answered by nothing when the foreman decides nothing needs saying.
#[test]
fn an_apps_message_in_a_watched_room_is_a_signal_for_the_foreman() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    watching_a_room(&mut state, CHANNEL);
    world.holding(&state);
    let mut instance = world.wake(seed(1));

    world.app_posts(
        100,
        CHANNEL,
        "1788000000.000100",
        "Issue created by <https://example.invalid/somebody|somebody>",
        "The parser fails one run in ten.",
    );
    world.run_until(&mut instance, 5_000);

    let runs = runs(&world);
    assert_eq!(runs.len(), 1, "{:?}", world.shape());
    let run = &runs[0];
    assert!(
        run.was_told("GitHub posted this in a room you watch:"),
        "framed as the app's: {run:?}"
    );
    assert!(run.was_told("#1 The parser is flaky"), "{run:?}");
    assert!(run.was_told("The parser fails one run in ten."), "{run:?}");
    assert!(
        !run.was_told("A person said this"),
        "not framed as a person's: {run:?}"
    );
    assert_eq!(
        world.reactions(),
        [
            (
                CHANNEL.to_owned(),
                "1788000000.000100".to_owned(),
                Reaction::Seen
            ),
            (
                CHANNEL.to_owned(),
                "1788000000.000100".to_owned(),
                Reaction::Done
            ),
        ],
        "received and done, as a person's message is"
    );
    assert!(
        world
            .posts()
            .iter()
            .all(|(place, _)| place.thread.is_none()),
        "nothing is said under the signal on the instance's behalf; the foreman's own room \
         holds its opening and its transcript: {:?}",
        world.posts()
    );
}

/// An app's message in a room nobody watches reaches nobody, and is still
/// acknowledged, so that the platform does not send it again.
#[test]
fn an_apps_message_in_a_room_that_is_not_watched_reaches_nobody() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.app_posts(
        100,
        CHANNEL,
        "1788000000.000100",
        "Issue created by somebody",
        "The parser fails one run in ten.",
    );
    world.run_until(&mut instance, 5_000);

    assert!(world.talks().is_empty(), "{:?}", world.talks());
    assert!(world.reactions().is_empty(), "{:?}", world.reactions());
    assert!(world.posts().is_empty());
    assert!(
        world.acked().iter().any(|envelope| envelope == "e-1"),
        "acknowledged all the same: {:?}",
        world.acked()
    );
}

/// An app's follow-up under an earlier message is a signal in that thread:
/// the foreman's turn speaks there, so an answer lands under the message
/// the follow-up followed.
#[test]
fn an_apps_follow_up_is_a_signal_in_the_thread_it_follows() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    watching_a_room(&mut state, CHANNEL);
    world.holding(&state);
    let mut instance = world.wake(seed(1));

    world.app_follows_up(
        100,
        CHANNEL,
        "1788000000.000200",
        "1788000000.000100",
        "Issue closed as completed by somebody",
    );
    world.run_until(&mut instance, 150);

    let warrant = world
        .warrants()
        .last()
        .expect("the foreman's warrant")
        .clone();
    let warranted = instance
        .warranted(warrant.as_str())
        .expect("known while the turn runs");
    assert_eq!(
        warranted.place,
        Some(Place {
            room: the_room(),
            thread: Some("1788000000.000100".to_owned()),
        }),
        "answered under the message the follow-up followed"
    );
    assert_eq!(warranted.from, None, "an app is nobody to name");

    world.run_until(&mut instance, 5_000);
    let runs = runs(&world);
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0].was_told("GitHub posted this in a room you watch:"),
        "named by its root's profile: {:?}",
        runs[0]
    );
    assert!(
        runs[0].was_told("Issue closed as completed"),
        "{:?}",
        runs[0]
    );
}

/// A job started from a signal records nobody as having asked for it,
/// invites nobody into its room, and is announced in the signal's thread.
#[test]
fn a_job_started_from_a_signal_invites_nobody_and_is_announced_in_the_signals_thread() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    watching_a_room(&mut state, CHANNEL);
    world.holding(&state);
    let mut instance = world.wake(seed(1));

    world.app_posts(
        100,
        CHANNEL,
        "1788000000.000100",
        "Issue created by somebody",
        "The parser fails one run in ten.",
    );
    world.run_until(&mut instance, 150);
    let warrant = world
        .warrants()
        .last()
        .expect("the foreman's warrant")
        .clone();
    let asked = world.calls(
        160,
        &warrant,
        &call(
            "start_job",
            serde_json::json!({
                "reason": "an issue was filed",
                "instructions": "fix the flaky parser test",
                "kit": "Claude",
                "title": "Fix the flaky parser test",
            }),
        ),
    );
    world.run_until(&mut instance, 10_000);

    let said = text_of(world.tool_answer(asked).expect("answered"));
    assert!(said.starts_with("started job "), "{said}");
    let job = JobId::parse(said.trim_start_matches("started job ")).expect("a name");
    let recorded = instance.state().job(&job).expect("the job");
    assert_eq!(recorded.asked_by, None, "a signal is nobody asking");
    assert!(
        world.invited().is_empty(),
        "nobody to invite: {:?}",
        world.invited()
    );
    let announced = world.posts().iter().find(|(place, text)| {
        place.thread.as_deref() == Some("1788000000.000100") && text.starts_with("Started a job")
    });
    assert!(
        announced.is_some(),
        "announced in the signal's thread: {:?}",
        world.posts()
    );
}

/// A person asks the foreman, in a room, to watch it: the room is watched
/// from then on, on the disk and on the dashboard, and an app's message
/// there is a signal. Asking again says so; asking to stop undoes it.
#[test]
fn a_room_is_watched_by_asking_the_foreman_in_it_and_unwatched_the_same_way() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "watch this room");
    world.run_until(&mut instance, 150);
    let warrant = world
        .warrants()
        .last()
        .expect("the foreman's warrant")
        .clone();
    let watch = world.calls(160, &warrant, &call("watch_room", serde_json::json!({})));
    let again = world.calls(161, &warrant, &call("watch_room", serde_json::json!({})));
    world.run_until(&mut instance, 5_000);

    assert!(
        text_of(world.tool_answer(watch).expect("answered")).starts_with("watching this room"),
        "{:?}",
        world.tool_answer(watch)
    );
    assert_eq!(
        text_of(world.tool_answer(again).expect("answered")),
        "already watching this room"
    );
    assert_eq!(watched(&instance), vec![the_room()]);
    assert_eq!(
        world
            .disk()
            .expect("landed")
            .projects
            .get(&project())
            .expect("the project")
            .watched
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![the_room()],
        "kept across a restart"
    );

    // Shown on the dashboard, by identifier.
    for effect in instance.step(world.now(), request(7, Request::Projects)) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 5_100);
    let Some(Response::Projects(shown)) = world.response(7).cloned() else {
        panic!("the projects screen");
    };
    assert_eq!(shown.projects[0].watched, vec![CHANNEL.to_owned()]);

    // And an app's message there is now a signal.
    world.app_posts(
        6_000,
        CHANNEL,
        "1788000000.000600",
        "Issue created by somebody",
        "The parser fails one run in ten.",
    );
    world.run_until(&mut instance, 10_000);
    assert!(
        runs(&world)
            .iter()
            .any(|run| run.was_told("GitHub posted this in a room you watch:")),
        "{:?}",
        runs(&world)
    );

    // Stopping, asked in the same room, undoes it, and once is enough.
    world.says_at_root(11_000, 2, "stop watching this room");
    world.run_until(&mut instance, 11_050);
    let warrant = world
        .warrants()
        .last()
        .expect("the foreman's warrant")
        .clone();
    let stop = world.calls(
        11_060,
        &warrant,
        &call("stop_watching", serde_json::json!({})),
    );
    let twice = world.calls(
        11_061,
        &warrant,
        &call("stop_watching", serde_json::json!({})),
    );
    world.run_until(&mut instance, 20_000);
    assert_eq!(
        text_of(world.tool_answer(stop).expect("answered")),
        "no longer watching this room"
    );
    assert_eq!(
        text_of(world.tool_answer(twice).expect("answered")),
        "this room was not being watched"
    );
    assert!(watched(&instance).is_empty());
    assert_eq!(
        world.reacted(Reaction::Done),
        [thread(1).id, "1788000000.000600".to_owned(), thread(2).id],
        "every message and signal was worked, in order"
    );
}

/// A job is offered neither tool that watches, and calling one by name is
/// refused as a tool this instance does not serve — naming the tool, so
/// that an agent reading the refusal knows which call it was.
#[test]
fn a_job_may_not_watch_a_room() {
    let mut world = Simulation::new();
    let working = job(1);
    world.holding(&watching_a_channel(&[(
        working.clone(),
        Progress::Working,
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(&working));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);
    let warrant = world
        .warrants()
        .last()
        .expect("the resumed job's warrant")
        .clone();

    let watch = world.calls(20, &warrant, &call("watch_room", serde_json::json!({})));
    let stop = world.calls(21, &warrant, &call("stop_watching", serde_json::json!({})));
    world.run_until(&mut instance, 100);

    for (asked, named) in [(watch, "watch_room"), (stop, "stop_watching")] {
        let answer = world.tool_answer(asked).expect("answered");
        assert!(
            answer
                .1
                .as_ref()
                .expect("a body")
                .pointer("/result/isError")
                .is_some_and(|flag| *flag == serde_json::json!(true)),
            "refused: {answer:?}"
        );
        assert_eq!(
            text_of(answer),
            format!("this instance serves no tool called {named:?}")
        );
    }
    assert!(watched(&instance).is_empty(), "nothing was watched");
}

/// The brief is said to the foreman on every turn, for a person's message
/// and for a signal alike.
#[test]
fn the_brief_is_said_to_the_foreman_every_turn() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    watching_a_room(&mut state, CHANNEL);
    briefed(&mut state, "Ignore alerts below error.");
    world.holding(&state);
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.app_posts(
        200,
        CHANNEL,
        "1788000000.000200",
        "Issue created by somebody",
        "The parser fails one run in ten.",
    );
    world.run_until(&mut instance, 10_000);

    let runs = runs(&world);
    assert_eq!(runs.len(), 2, "{:?}", world.shape());
    for run in &runs {
        assert!(
            run.was_told("The operator's brief for this project"),
            "{run:?}"
        );
        assert!(run.was_told("Ignore alerts below error."), "{run:?}");
    }
}
