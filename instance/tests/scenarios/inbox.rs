//! A message reaching a job whatever it is doing, run against the simulated
//! world — see `docs/decisions/0069-a-message-reaches-a-working-job.md`.
//!
//! Every scenario here is one row of what that record decides: what a
//! message finds, and what happens to it. The simulated adapter answers a
//! message handed to a turn as the pinned one was measured to, and never
//! echoes it, so what the agent was told mid-turn is read off the
//! conversation's own record.

use crate::simulation::{
    Simulation, Utterance, job, link_to, project, request, room, seed, watching_a_channel,
};
use stageman_agent::{Command, ToolKind};
use stageman_channel::Reaction;
use stageman_core::{JobId, Place, Progress, State, Waiting};
use stageman_foreman::{Because, Finding, turn_notice};
use stageman_instance::Request;

/// The first and second messages a person says in the job's room.
const FIRST: &str = "1788000099.000010";
const SECOND: &str = "1788000099.000020";

/// Two threads in the job's room, by their parents.
const PARENT: &str = "1788000000.500000";
const OTHER: &str = "1788000000.600000";

fn progress_of(state: &State, id: JobId) -> Progress {
    state.job(id).expect("the job").progress.clone()
}

fn in_room(n: u32) -> Place {
    Place::root(room(n))
}

/// A place under a message at the root of a job's room: where a notice
/// about that message alone is said.
fn under(n: u32, message: &str) -> Place {
    Place {
        room: room(n),
        thread: Some(message.to_owned()),
    }
}

/// How a message at the root of the job's room is named to the agent, and
/// the frame it is handed over with, as an interruption.
fn handed(message: &str, text: &str) -> String {
    stageman_foreman::reply(
        &format!("<@U0BOT> {text}"),
        &format!("{}/{message}", room(1).id),
        None,
        Finding::PartWay,
    )
}

/// A job put back to work by waking: its turn is talking by the time a
/// message arrives.
fn a_working_job(world: &mut Simulation) -> (stageman_instance::Instance, JobId) {
    let working = job(1);
    world.holding(&watching_a_channel(&[(working, Progress::Working, 1)]));
    let (name, held) = Simulation::ours(&stageman_job::container(working));
    world.container(&name, held);
    let instance = world.wake(seed(1));
    (instance, working)
}

/// An idle job with a room, waiting to be given something.
fn an_idle_job(world: &mut Simulation) -> (stageman_instance::Instance, JobId) {
    let idle = job(1);
    world.holding(&watching_a_channel(&[(
        idle,
        Progress::Idle(Waiting::Asked),
        1,
    )]));
    let (name, held) = Simulation::ours(&stageman_job::container(idle));
    world.container(&name, held);
    let instance = world.wake(seed(1));
    (instance, idle)
}

/// A message to a working job is received, not refused: the eyes reaction
/// once the record holding it has landed, handed to the turn in flight
/// framed as an interruption, the root told it landed with a link, the
/// check mark when the turn ends, and no second turn.
#[test]
fn a_message_to_a_working_job_is_handed_to_its_turn() {
    let mut world = Simulation::new();
    let (mut instance, working) = a_working_job(&mut world);

    // Before its resumed turn ends, and after its conversation has opened.
    world.says_in_room_as(100, 1, FIRST, "also check the tests");
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        world.talks().len(),
        1,
        "only the resume waking asked for: {:?}",
        world.talks()
    );
    assert_eq!(
        world.talks()[0].steered,
        vec![handed(FIRST, "also check the tests")],
        "handed to the turn in flight, as an interruption"
    );
    let link = link_to(&room(1).id, FIRST, None);
    assert_eq!(
        world.posts(),
        [
            (in_room(1), turn_notice(&Because::Restart(None))),
            (in_room(1), stageman_foreman::landed_notice(Some(&link))),
            (in_room(1), "done".to_owned()),
        ],
        "why the turn started, that the message landed, and what the agent said; \
         a turn waking started says nothing about its end"
    );
    assert_eq!(world.reacted(Reaction::Seen), [FIRST], "received");
    assert_eq!(
        world.reacted(Reaction::Done),
        [FIRST],
        "finished with when the turn that absorbed it ended"
    );
    assert!(
        world
            .first_call(|call| matches!(call, stageman_channel::Call::Replies { .. }))
            .is_none(),
        "a message at the root has no thread to read"
    );
    let recorded = instance.state().job(working).expect("the job");
    assert_eq!(recorded.progress, Progress::Idle(Waiting::Silent));
    assert!(recorded.inbox.is_empty(), "nothing waits for an idle job");
    let shape = world.shape();
    let written = shape
        .iter()
        .position(|line| line.starts_with("<- Written"))
        .expect("the record was written");
    let reacted = shape
        .iter()
        .position(|line| line.contains("reactions.add"))
        .expect("the reaction was asked for");
    assert!(written < reacted, "the eyes wait for the record: {shape:?}");
}

/// A message that lands clears the claim the agent had made, and marks the
/// call it interrupted: the agent said it was waiting for an answer, then
/// received one mid-turn, so the turn that ends without claiming again is
/// silent rather than asked; and the call running when it landed is shown
/// interrupted in its burst, since no ending arrives for it.
#[test]
fn a_message_that_lands_clears_the_claim_and_marks_the_interrupted_call() {
    let mut world = Simulation::new();
    // The agent runs a command that is still running when the message
    // lands, and speaks once after: the turn lasts until it does.
    world.next_turn_narrates_over(
        vec![
            Utterance::Calls("cargo test"),
            Utterance::Says("Switching to Postgres."),
        ],
        2_000,
    );
    let (mut instance, working) = a_working_job(&mut world);
    // The claim, as the agent's own tool call makes it, before the message.
    world.run_until(&mut instance, 300);
    let warrant = world.warrants().last().expect("the job's warrant").clone();
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), "stopping".into());
    params.insert(
        "arguments".to_owned(),
        serde_json::json!({"because": "waiting_for_an_answer"}),
    );
    let mut envelope = serde_json::Map::new();
    envelope.insert("jsonrpc".to_owned(), "2.0".into());
    envelope.insert("id".to_owned(), 1.into());
    envelope.insert("method".to_owned(), "tools/call".into());
    envelope.insert("params".to_owned(), serde_json::Value::Object(params));
    let asked = world.calls(301, &warrant, &serde_json::Value::Object(envelope));
    world.run_until(&mut instance, 350);
    assert_eq!(
        world
            .tool_answer(asked)
            .and_then(|answer| answer.1.as_ref())
            .and_then(|body| body.pointer("/result/content/0/text"))
            .and_then(serde_json::Value::as_str),
        Some("noted"),
        "the claim was registered"
    );

    // While the command runs, and before the agent speaks again.
    world.says_in_room_as(2_000, 1, FIRST, "use postgres");
    world.run_until(&mut instance, 10_000);

    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent),
        "the claim made before the message landed no longer counts"
    );
    let interrupted = stageman_foreman::interrupted_line(ToolKind::Execute, "cargo test");
    assert!(
        world
            .edits()
            .iter()
            .any(|(_, text)| text.contains(&interrupted)),
        "the call running when the message landed is marked interrupted: {:?}",
        world.edits()
    );
}

/// Two messages arriving together to an idle job: the first puts it to
/// work, the second waits — since receiving is one step — and is handed to
/// the turn the moment its conversation can take one, as an interruption.
/// Both are received, and both are finished with when the turn ends.
#[test]
fn two_messages_arriving_together_start_one_turn_and_the_second_is_handed_to_it() {
    let mut world = Simulation::new();
    let (mut instance, idle) = an_idle_job(&mut world);

    world.says_in_room_as(100, 1, FIRST, "first");
    world.says_in_room_as(100, 1, SECOND, "second");
    world.run_until(&mut instance, 5_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].was_told("first"), "{runs:?}");
    assert!(
        !runs[0].was_told("second"),
        "the second is not folded into the first's frame: {runs:?}"
    );
    assert_eq!(runs[0].steered, vec![handed(SECOND, "second")]);
    assert_eq!(world.reacted(Reaction::Seen), [FIRST, SECOND]);
    assert_eq!(world.reacted(Reaction::Done), [FIRST, SECOND]);
    assert!(
        instance
            .state()
            .job(idle)
            .expect("the job")
            .inbox
            .is_empty()
    );
}

/// A message the adapter declines waits for the turn's end, which starts
/// the next turn on it at once — with no probe of the container between the
/// two, since nothing stops between queued messages — and nothing more is
/// handed to that turn.
#[test]
fn a_message_the_adapter_declines_is_delivered_by_the_turns_end() {
    let mut world = Simulation::new();
    let (mut instance, working) = a_working_job(&mut world);
    world.next_steer_declined();

    world.says_in_room_as(100, 1, FIRST, "first");
    world.says_in_room_as(150, 1, SECOND, "second");
    world.run_until(&mut instance, 10_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(
        runs[0].steered.is_empty(),
        "declined, and nothing more handed to that turn: {runs:?}"
    );
    assert_eq!(
        runs[1].prompt.as_deref(),
        Some(
            stageman_foreman::reply(
                "<@U0BOT> first",
                &format!("{}/{FIRST}", room(1).id),
                None,
                Finding::AtRest
            )
            .as_str()
        ),
        "the next turn starts on the message that was declined"
    );
    assert_eq!(
        runs[1].steered,
        vec![handed(SECOND, "second")],
        "and the one behind it is handed to that turn"
    );
    // Nothing stops between queued messages: the first probe of the
    // container comes after the last turn has ended.
    let last_ended = runs[1].ended_at.expect("the second ended");
    let first_probe = world
        .first_asking(|command| matches!(command, Command::Port { .. } | Command::Halt { .. }));
    assert!(
        first_probe.is_none_or(|at| at > last_ended),
        "no probe until the last turn has ended: {:?}",
        world.commands_after(runs[0].ended_at.expect("the first ended"))
    );
    assert_eq!(world.reacted(Reaction::Done), [FIRST, SECOND]);
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent)
    );
}

/// An adapter that does not say it can take a message is handed none: the
/// message waits for the turn's end, which delivers it as a turn of its
/// own.
#[test]
fn an_adapter_that_cannot_be_steered_is_handed_nothing() {
    let mut world = Simulation::new();
    world.adapters_unsteerable();
    let (mut instance, _) = a_working_job(&mut world);
    world.person_says(20, &room(1).id, PARENT, None, "Which database?");

    // In a thread, so that a read is what handing it over would begin with.
    world.says_in_rooms_thread(100, 1, PARENT, "first");
    world.run_until(&mut instance, 10_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(runs.iter().all(|run| run.steered.is_empty()), "{runs:?}");
    assert!(runs[1].was_told("first"));
    let reads = world
        .shape()
        .iter()
        .filter(|line| line.contains("conversations.replies"))
        .count();
    assert_eq!(
        reads,
        1,
        "the thread is read once, for the turn that delivers it: {:?}",
        world.shape()
    );
}

/// A cancel the agent does not answer within the bound closes its process
/// after all, which is what a stop was before it became a cancel: the job
/// is paused either way.
#[test]
fn a_cancel_nobody_answers_closes_the_process() {
    let mut world = Simulation::new();
    world.next_cancel_ignored();
    // A turn long enough to outlast the bound: it ends with its last
    // utterance, a long way after its first.
    world.next_turn_narrates_over(
        vec![Utterance::Calls("sleep"), Utterance::Says("Awake.")],
        60_000,
    );
    let (mut instance, working) = a_working_job(&mut world);

    world.run_until(&mut instance, 200);
    for effect in instance.step(request(
        7,
        Request::Stop {
            project: project().to_string(),
            job: working.to_string(),
        },
    )) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 20_000);
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Working,
        "still waiting on the cancel being answered"
    );
    world.run_until(&mut instance, 40_000);

    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Paused)
    );
    let runs = world.talks();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(!runs[0].cancelled, "the adapter ignored the cancel");
    assert!(runs[0].ended_at.is_some(), "and its process was closed");
}

/// A person's stop is a cancel: the prompt in flight ends as cancelled, the
/// job is paused, and a message still waiting is told, under it, that it
/// will not be delivered — nothing waits for a job that is not working.
/// The next message resumes the job, framed as an interruption.
#[test]
fn a_stop_is_a_cancel_and_tells_what_was_waiting() {
    let mut world = Simulation::new();
    // An adapter that takes no message mid-turn, so that the second waits
    // for the turn's end — and the stop comes first.
    world.adapters_unsteerable();
    let (mut instance, working) = an_idle_job(&mut world);

    // The first starts the turn and is in hand; the second waits.
    world.says_in_room_as(100, 1, FIRST, "first");
    world.says_in_room_as(150, 1, SECOND, "second");
    world.run_until(&mut instance, 300);
    for effect in instance.step(request(
        7,
        Request::Stop {
            project: project().to_string(),
            job: working.to_string(),
        },
    )) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Paused)
    );
    let runs = world.talks();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].was_told("first"));
    assert!(runs[0].cancelled, "cancelled rather than closed: {runs:?}");
    assert!(
        world.posts().contains(&(
            under(1, SECOND),
            stageman_foreman::stopped_before_notice("<@U0BOT>")
        )),
        "the message still waiting is told: {:?}",
        world.posts()
    );
    assert!(
        !world
            .posts()
            .iter()
            .any(|(place, _)| *place == under(1, FIRST)),
        "the one in hand had reached the agent, and is told nothing: {:?}",
        world.posts()
    );
    assert!(
        instance
            .state()
            .job(working)
            .expect("the job")
            .inbox
            .is_empty(),
        "nothing waits for a paused job"
    );
    assert_eq!(
        world.reacted(Reaction::Seen),
        [FIRST, SECOND],
        "both were received"
    );
    assert_eq!(
        world.reacted(Reaction::Done),
        Vec::<String>::new(),
        "a stopped turn handled nothing, the message in hand included"
    );

    // The next message resumes it, told it was interrupted.
    world.says_in_room_as(6_000, 1, "1788000099.000030", "carry on");
    world.run_until(&mut instance, 12_000);
    let runs = world.talks();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(
        runs[1].prompt.as_deref(),
        Some(
            stageman_foreman::reply(
                "<@U0BOT> carry on",
                &format!("{}/1788000099.000030", room(1).id),
                None,
                Finding::PartWay
            )
            .as_str()
        )
    );
}

/// A turn given messages from two threads signposts each thread it did not
/// answer in, and not the one it did: the agent answered the second
/// through the tool, so only the first is told where its answer went.
#[test]
fn each_thread_a_turn_was_asked_in_is_signposted_on_its_own() {
    let mut world = Simulation::new();
    let (mut instance, _) = an_idle_job(&mut world);
    world.person_says(20, &room(1).id, PARENT, None, "Which database?");
    world.person_says(21, &room(1).id, OTHER, None, "And which cache?");

    // The first starts the turn; the second is handed to it.
    world.says_in_rooms_thread(100, 1, PARENT, "decide");
    world.run_until(&mut instance, 300);
    let warrant = world.warrants().last().expect("the job's warrant").clone();
    world.says_in_rooms_thread_as(400, 1, OTHER, SECOND, "and this");
    world.run_until(&mut instance, 600);
    // The agent answers the second where it was asked.
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), "say".into());
    params.insert(
        "arguments".to_owned(),
        serde_json::json!({"message": "Redis.", "to": format!("{}/{OTHER}", room(1).id)}),
    );
    let mut envelope = serde_json::Map::new();
    envelope.insert("jsonrpc".to_owned(), "2.0".into());
    envelope.insert("id".to_owned(), 1.into());
    envelope.insert("method".to_owned(), "tools/call".into());
    envelope.insert("params".to_owned(), serde_json::Value::Object(params));
    world.calls(700, &warrant, &serde_json::Value::Object(envelope));
    world.run_until(&mut instance, 10_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs[0].steered.len(),
        1,
        "the second was handed over: {runs:?}"
    );
    let signpost = stageman_foreman::answered_elsewhere_notice(&stageman_channel::room_link(
        stageman_core::Channel::Slack,
        &room(1).id,
    ));
    assert!(
        world.posts().contains(&(under(1, PARENT), signpost)),
        "the first thread, answered nowhere, is signposted: {:?}",
        world.posts()
    );
    assert_eq!(
        world
            .posts()
            .iter()
            .filter(|(place, _)| *place == under(1, OTHER))
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        vec!["Redis."],
        "the second, answered where asked, is not: {:?}",
        world.posts()
    );
}

/// A stop asked for while the job is working with no turn registered — its
/// thread being read — is held, and takes effect when the turn is: it ends
/// at its first step, paused, and the agent is never spoken to.
#[test]
fn a_stop_before_the_turn_is_registered_is_held_and_takes_effect() {
    let mut world = Simulation::new();
    let (mut instance, idle) = an_idle_job(&mut world);
    world.person_says(
        20,
        &room(1).id,
        "1788000000.500000",
        None,
        "Which database?",
    );

    // A threaded reply: its thread is read before the turn is registered,
    // and the stop lands in between.
    world.says_in_rooms_thread(100, 1, "1788000000.500000", "go with that");
    world.run_until(&mut instance, 100);
    assert_eq!(
        progress_of(instance.state(), idle),
        Progress::Working,
        "received, and its thread not yet read: {:?}",
        world.shape()
    );
    for effect in instance.step(request(
        7,
        Request::Stop {
            project: project().to_string(),
            job: idle.to_string(),
        },
    )) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        progress_of(instance.state(), idle),
        Progress::Idle(Waiting::Paused)
    );
    assert!(
        world.talks().is_empty(),
        "the agent was never spoken to: {:?}",
        world.talks()
    );
    assert!(
        instance
            .state()
            .job(idle)
            .expect("the job")
            .inbox
            .is_empty()
    );
}

/// A turn that ends with a message waiting starts the next turn on it at
/// once, in arrival order, and the container is not probed between the
/// two.
#[test]
fn a_turn_that_ends_with_a_message_waiting_starts_the_next_at_once() {
    let mut world = Simulation::new();
    world.adapters_unsteerable();
    let (mut instance, working) = a_working_job(&mut world);

    world.says_in_room_as(100, 1, FIRST, "first");
    world.says_in_room_as(110, 1, SECOND, "second");
    world.run_until(&mut instance, 15_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 3, "the resume, then one per message: {runs:?}");
    assert!(runs[1].was_told("first"));
    assert!(runs[2].was_told("second"));
    for pair in runs.windows(2) {
        let [earlier, later] = pair else {
            panic!("a pair");
        };
        let ended = earlier.ended_at.expect("ended");
        assert!(ended < later.opened_at, "one turn at a time: {runs:?}");
    }
    // Nothing stops between queued messages: the first probe of the
    // container comes after the last turn has ended.
    let last_ended = runs[2].ended_at.expect("the last ended");
    let first_probe = world
        .first_asking(|command| matches!(command, Command::Port { .. } | Command::Halt { .. }));
    assert!(
        first_probe.is_none_or(|at| at > last_ended),
        "no probe until the last turn has ended: {:?}",
        world.commands_after(runs[0].ended_at.expect("the first ended"))
    );
    assert_eq!(world.reacted(Reaction::Done), [FIRST, SECOND]);
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent)
    );
}

/// A message arriving just after a turn ended, while the probe that follows
/// a turn's end is still in flight, starts the next turn — and the probe's
/// answer that nothing is behind the tunnel does not halt the container
/// under it.
#[test]
fn a_probe_in_flight_does_not_halt_a_container_a_new_turn_needs() {
    let mut world = Simulation::new();
    let (mut instance, working) = a_working_job(&mut world);

    // The resumed turn's prompt is answered a fixed time after it is sent;
    // a message one tick after finds the turn just ended and the probe
    // just asked.
    world.run_until(&mut instance, 100);
    let prompted = world
        .trace()
        .iter()
        .find(|line| line.contains("session/prompt"))
        .expect("the prompt was sent")
        .clone();
    let (sent_at, _) = prompted.split_once(':').expect("an instant");
    // One tick after the answer: the turn has ended and the probe been
    // asked, and its answer has not yet arrived.
    let ends_at = sent_at.trim().parse::<u64>().expect("a number") + 1_001;
    world.says_in_room_as(ends_at, 1, FIRST, "one more thing");
    world.run_until(&mut instance, 15_000);

    let runs = world.talks();
    assert_eq!(
        runs.len(),
        2,
        "the resume, and the message's turn: {runs:?}"
    );
    assert!(runs[1].was_told("one more thing"));
    let second_opened = runs[1].opened_at;
    let halted_between = world
        .first_asking(|command| matches!(command, Command::Halt { .. }))
        .is_some_and(|at| at < second_opened);
    assert!(
        !halted_between,
        "the container the new turn needs is not halted under it: {:?}",
        world.commands_after(runs[0].ended_at.expect("the first ended"))
    );
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent)
    );
}

/// A turn that fails with a message waiting is tried again by that message
/// at once, as a reply tries a failed job again.
#[test]
fn a_turn_that_fails_with_a_message_waiting_is_tried_again_by_it() {
    let mut world = Simulation::new();
    world.adapters_unsteerable();
    world.next_turn_ends(Err("the credential had expired".to_owned()));
    let (mut instance, working) = a_working_job(&mut world);

    world.says_in_room_as(100, 1, FIRST, "try again");
    world.run_until(&mut instance, 15_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(runs[1].was_told("try again"));
    assert_eq!(
        world.reacted(Reaction::Done),
        [FIRST],
        "the message the second turn handled, and nothing for the failed one"
    );
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent)
    );
}

/// A job that dies with messages in hand is resumed with them said again,
/// after the notice that it was interrupted, as something it may or may
/// not have seen; nothing is reacted to again, and no thread is read
/// again.
#[test]
fn a_resumed_job_is_told_again_what_it_had_in_hand() {
    let mut world = Simulation::new();
    let working = job(1);
    let mut state = watching_a_channel(&[(working, Progress::Working, 1)]);
    let recorded = state.job_mut(working).expect("the job");
    recorded.inbox.receive(stageman_core::Errand {
        said: "<@U0BOT> use postgres".to_owned(),
        thread: stageman_core::Thread {
            channel: stageman_core::Channel::Slack,
            room: room(1).id,
            id: "1788000000.500000".to_owned(),
        },
        from: Some("U0HUMAN".to_owned()),
        message: Some(FIRST.to_owned()),
        app: None,
    });
    recorded.inbox.give();
    world.holding(&state);
    let (name, held) = Simulation::ours(&stageman_job::container(working));
    world.container(&name, held);
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 5_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let expected = stageman_foreman::resumption_with(&[stageman_foreman::reply(
        "<@U0BOT> use postgres",
        &format!("{}/1788000000.500000", room(1).id),
        None,
        Finding::AtRest,
    )]);
    assert_eq!(runs[0].prompt.as_deref(), Some(expected.as_str()));
    assert!(
        world.reacted(Reaction::Seen).is_empty(),
        "nothing is re-acknowledged"
    );
    assert!(
        world
            .first_call(|call| matches!(call, stageman_channel::Call::Replies { .. }))
            .is_none(),
        "no thread is read again"
    );
    assert_eq!(
        world.reacted(Reaction::Done),
        [FIRST],
        "finished with when the turn ends"
    );
    assert!(
        instance
            .state()
            .job(working)
            .expect("the job")
            .inbox
            .is_empty()
    );
}

/// A file in which a job that is not working holds messages — a shape this
/// version never writes — opens with them dropped, and a line saying so.
#[test]
fn a_job_that_is_not_working_opens_with_nothing_waiting() {
    let mut world = Simulation::new();
    let idle = job(1);
    let mut state = watching_a_channel(&[(idle, Progress::Idle(Waiting::Asked), 1)]);
    state
        .job_mut(idle)
        .expect("the job")
        .inbox
        .receive(stageman_core::Errand {
            said: "<@U0BOT> anyone?".to_owned(),
            thread: stageman_core::Thread {
                channel: stageman_core::Channel::Slack,
                room: room(1).id,
                id: FIRST.to_owned(),
            },
            from: None,
            message: Some(FIRST.to_owned()),
            app: None,
        });
    world.holding(&state);
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 100);

    assert!(
        instance
            .state()
            .job(idle)
            .expect("the job")
            .inbox
            .is_empty()
    );
    assert!(
        world.talks().is_empty(),
        "nothing delivers what was dropped"
    );
}

/// The foreman is not steered: a second mention while it works waits in its
/// inbox and is its own turn, and nothing is handed to the turn in flight.
#[test]
fn a_foreman_is_not_steered() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "first");
    world.says_at_root(200, 2, "second");
    world.run_until(&mut instance, 10_000);

    let runs = world.talks();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(runs.iter().all(|run| run.steered.is_empty()), "{runs:?}");
    assert!(runs[0].was_told("first") && runs[1].was_told("second"));
}

/// A message to a job that is over is still refused, with the notice that
/// says so, and nothing waits for it.
#[test]
fn a_message_to_a_job_that_is_over_is_refused() {
    let mut world = Simulation::new();
    let over = job(1);
    world.holding(&watching_a_channel(&[(
        over,
        Progress::Retired(stageman_core::Outcome::Done),
        1,
    )]));
    let mut instance = world.wake(seed(1));

    world.says_in_room_as(100, 1, FIRST, "one more thing");
    world.run_until(&mut instance, 200);

    assert_eq!(
        world.posts(),
        [(in_room(1), stageman_foreman::over_notice().to_owned())]
    );
    assert!(
        instance
            .state()
            .job(over)
            .expect("the job")
            .inbox
            .is_empty()
    );
    assert!(world.reacted(Reaction::Seen).is_empty());
}
