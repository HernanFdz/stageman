//! The tools endpoint, called by the agents this instance runs, against the
//! simulated world.

use crate::simulation::{
    FARAWAY, NEARBY, Simulation, in_room, in_thread, job, room, seed, watching_a_channel,
};
use stageman_channel::Call;
use stageman_core::{Progress, Waiting};

fn body(method: &str, params: serde_json::Value) -> serde_json::Value {
    let mut envelope = serde_json::Map::new();
    envelope.insert("jsonrpc".to_owned(), "2.0".into());
    envelope.insert("id".to_owned(), 1.into());
    envelope.insert("method".to_owned(), method.into());
    envelope.insert("params".to_owned(), params);
    serde_json::Value::Object(envelope)
}

fn call(name: &str, arguments: serde_json::Value) -> serde_json::Value {
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), name.into());
    params.insert("arguments".to_owned(), arguments);
    body("tools/call", serde_json::Value::Object(params))
}

fn text_of(answer: &(u16, Option<serde_json::Value>)) -> String {
    answer
        .1
        .as_ref()
        .expect("a body")
        .pointer("/result/content/0/text")
        .and_then(serde_json::Value::as_str)
        .expect("text")
        .to_owned()
}

fn is_error(answer: &(u16, Option<serde_json::Value>)) -> bool {
    answer
        .1
        .as_ref()
        .expect("a body")
        .pointer("/result/isError")
        .is_some_and(|flag| *flag == serde_json::json!(true))
}

/// A world in which the foreman is mid-turn, with the warrant it holds.
fn with_a_foreman_working() -> (Simulation, stageman_instance::Instance, String) {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 150);
    let warrant = world
        .warrants()
        .last()
        .expect("the foreman's warrant")
        .clone();
    (world, instance, warrant)
}

/// Nobody from beyond this machine, and nobody without a credential this
/// instance minted, is answered with anything but a refusal.
#[test]
fn only_a_nearby_caller_with_a_minted_credential_is_served() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    // From beyond this machine, which is nowhere a container is.
    let asked1 = world.arrives(
        160,
        "POST",
        "/mcp",
        &[("authorization", &format!("Bearer {warrant}"))],
        FARAWAY,
        &serde_json::to_string(&body("tools/list", serde_json::json!({}))).expect("JSON"),
    );
    let asked2 = world.calls(
        161,
        "not-minted",
        &body("tools/list", serde_json::json!({})),
    );
    let asked3 = world.calls(162, &warrant, &body("tools/list", serde_json::json!({})));
    world.run_until(&mut instance, 200);

    assert_eq!(world.tool_answer(asked1).map(|a| a.0), Some(403));
    assert_eq!(world.tool_answer(asked2).map(|a| a.0), Some(403));
    assert_eq!(world.tool_answer(asked3).map(|a| a.0), Some(200));
}

/// What a client offers, what it hangs up on, and what it asks for by
/// another name.
///
/// Every tool answers within its own call, so the stream a client may offer
/// to open is declined; nothing is held per connection, so hanging up has
/// nothing to release; and one path is served, so another is not found.
#[test]
fn a_stream_is_declined_a_hangup_accepted_and_another_path_is_not_found() {
    let (mut world, mut instance, _) = with_a_foreman_working();

    let offered = world.arrives(160, "GET", "/mcp", &[], NEARBY, "");
    let hung_up = world.arrives(161, "DELETE", "/mcp", &[], NEARBY, "");
    let elsewhere = world.arrives(162, "POST", "/elsewhere", &[], NEARBY, "");
    world.run_until(&mut instance, 200);

    assert_eq!(world.tool_answer(offered).map(|a| a.0), Some(405));
    assert_eq!(world.tool_answer(hung_up).map(|a| a.0), Some(204));
    assert_eq!(world.tool_answer(elsewhere).map(|a| a.0), Some(404));
}

/// A call is read up to a limit: a large one is answered, and one beyond
/// the limit is refused rather than held.
///
/// The limit is generous because the largest call carries a message a person
/// wrote, and bounded because whoever sends one is somebody else's code
/// running in a container.
#[test]
fn a_call_is_read_up_to_a_limit_and_refused_beyond_it() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    let large = world.calls(
        160,
        &warrant,
        &body(
            "tools/list",
            serde_json::json!({"padding": "x".repeat(8 * 1024)}),
        ),
    );
    let beyond = world.arrives(
        161,
        "POST",
        "/mcp",
        &[("authorization", &format!("Bearer {warrant}"))],
        NEARBY,
        &"x".repeat(2 * 1024 * 1024),
    );
    world.run_until(&mut instance, 200);

    assert_eq!(
        world.tool_answer(large).map(|a| a.0),
        Some(200),
        "eight kilobytes is an ordinary call"
    );
    assert_eq!(world.tool_answer(beyond).map(|a| a.0), Some(400));
}

/// The handshake, a notification, and the listing a foreman is offered.
#[test]
fn a_foreman_is_greeted_and_offered_the_tools_that_start_jobs() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    let asked1 = world.calls(
        160,
        &warrant,
        &body(
            "initialize",
            serde_json::json!({"protocolVersion": "2024-11-05"}),
        ),
    );
    let asked2 = world.calls(
        161,
        &warrant,
        &serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    let asked3 = world.calls(162, &warrant, &body("tools/list", serde_json::json!({})));
    let asked4 = world.calls(163, &warrant, &serde_json::json!({"not": "a request"}));
    let asked5 = world.calls(
        164,
        &warrant,
        &body("notifications/initialized", serde_json::json!({})),
    );
    world.run_until(&mut instance, 200);

    let greeting = world.tool_answer(asked1).expect("answered");
    assert_eq!(greeting.0, 200);
    let result = &greeting.1.as_ref().expect("a body")["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert_eq!(result["serverInfo"]["name"], "stageman");
    assert_eq!(
        result["serverInfo"]["version"],
        stageman_instance::release::described()
    );

    assert_eq!(
        world.tool_answer(asked2),
        Some(&(202, None)),
        "a notification wants no answer"
    );

    let listing = world.tool_answer(asked3).expect("answered");
    let names: Vec<&str> = listing.1.as_ref().expect("a body")["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["say", "start_job", "watch_room", "stop_watching"]);
    assert_eq!(
        listing.1.as_ref().expect("a body")["result"]["tools"][1]["inputSchema"]["properties"]["kit"]
            ["enum"],
        serde_json::json!(["Claude"]),
        "the project's kits are enumerated"
    );

    assert_eq!(world.tool_answer(asked4).map(|a| a.0), Some(400));

    // A notification that nonetheless carries an identifier is answering
    // something, and is answered with nothing rather than merely accepted.
    let noted = world.tool_answer(asked5).expect("answered");
    assert_eq!(noted.0, 200);
    assert_eq!(
        noted.1.as_ref().expect("a body")["result"],
        serde_json::json!({})
    );
}

/// Minting a warrant forgets only that speaker's previous one: another
/// speaker's goes on answering, and the speaker's own old one stops.
#[test]
fn minting_a_warrant_forgets_only_that_speakers_previous_one() {
    let (mut world, mut instance, first) = with_a_foreman_working();

    // The foreman starts a job, which mints the job its own warrant.
    let _ = world.calls(
        160,
        &first,
        &call(
            "start_job",
            serde_json::json!({
                "reason": "the parser is flaky",
                "instructions": "Fix the flaky test in the parser.",
                "kit": "Claude",
            }),
        ),
    );
    world.run_until(&mut instance, 300);
    assert!(
        world.warrants().len() >= 2,
        "the job's turn has begun with a warrant of its own: {:?}",
        world.shape()
    );

    // Another speaker's minting leaves the foreman's answering.
    let asked2 = world.calls(400, &first, &body("tools/list", serde_json::json!({})));
    world.run_until(&mut instance, 500);
    assert_eq!(world.tool_answer(asked2).map(|a| a.0), Some(200));

    // The foreman's next turn mints it a new one, and the old one stops.
    world.run_until(&mut instance, 2_000);
    world.says_at_root(2_100, 2, "and look at the tests");
    world.run_until(&mut instance, 2_200);
    let second = world.warrants().last().cloned().expect("the new warrant");
    assert_ne!(second, first);
    let asked3 = world.calls(2_300, &first, &body("tools/list", serde_json::json!({})));
    let asked4 = world.calls(2_301, &second, &body("tools/list", serde_json::json!({})));
    world.run_until(&mut instance, 2_400);
    assert_eq!(world.tool_answer(asked3).map(|a| a.0), Some(403));
    assert_eq!(world.tool_answer(asked4).map(|a| a.0), Some(200));
}

/// A foreman starts a job: the job is recorded, its room is made once the
/// record has landed, the agent is told once the room is on the record, and
/// the foreman is answered with the job's identifier. The person whose
/// message the foreman was answering is invited into the room and recorded
/// as having asked, and their thread is told where the job is.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one flow asserted end to end, where the order of its steps is the point"
)]
fn a_foreman_starts_a_job_whose_room_is_made_before_its_agent_speaks() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    let asked1 = world.calls(
        160,
        &warrant,
        &call(
            "start_job",
            serde_json::json!({
                "reason": "the parser is flaky",
                "instructions": "Fix the flaky test in the parser.",
                "kit": "Claude",
                "title": "Flaky parser test",
            }),
        ),
    );
    world.run_until(&mut instance, 10_000);

    let answer = world.tool_answer(asked1).expect("answered");
    assert!(!is_error(answer), "{answer:?}");
    let said = text_of(answer);
    assert!(said.starts_with("started job "), "{said}");
    let started = said
        .trim_start_matches("started job ")
        .parse()
        .expect("an identifier");
    let started = stageman_core::JobId::from_uuid(started);

    let shape = world.shape();
    let persisted = shape
        .iter()
        .position(|line| line.starts_with("<- Written"))
        .expect("written");
    // Read back through the channel crate's inverse: the room is a request
    // like the listener's questions, and is told apart the same way.
    let opened = world
        .first_call(|call| matches!(call, Call::CreateRoom { .. }))
        .expect("made");
    let talks = world.talks_in(&stageman_job::container(started));
    let [run] = talks.as_slice() else {
        panic!("the job's agent was spoken to once: {talks:?}");
    };
    assert!(
        persisted < opened && opened < run.opened_at,
        "record, then room, then agent: {shape:?}"
    );
    assert!(run.began(), "{run:?}");
    assert!(run.was_told("Fix the flaky test in the parser."), "{run:?}");
    assert!(
        run.was_told("the `say` tool"),
        "a job with a channel is told how to speak: {run:?}"
    );

    let recorded = instance
        .state()
        .job(started)
        .expect("the job is on the record");
    assert_eq!(recorded.reason, "the parser is flaky");
    assert_eq!(
        recorded.room,
        Some(room(2)),
        "in the room that was made for it, after the foreman's own"
    );
    assert_eq!(
        recorded.asked_by.as_deref(),
        Some("U0HUMAN"),
        "the person the foreman was answering asked for it"
    );
    assert!(
        world
            .rooms()
            .get(1)
            .is_some_and(|(_, name)| name.starts_with("example--flaky-parser-test--")),
        "named after the project and the title the foreman gave: {:?}",
        world.rooms()
    );
    assert_eq!(
        world.invited(),
        [(room(2).id, "U0HUMAN".to_owned())],
        "and they are invited into it"
    );
    assert!(
        world.posts().contains(&(
            in_thread(1),
            stageman_foreman::started_notice("<#C-job-002>")
        )),
        "their thread is told where the job is: {:?}",
        world.posts()
    );
    assert!(
        world
            .posts()
            .iter()
            .any(|(at, text)| *at == in_room(2) && text.contains("Mention <@U0BOT>")),
        "the room opens by teaching the mention: {:?}",
        world.posts()
    );
    assert_eq!(
        recorded.progress,
        Progress::Idle(Waiting::Silent),
        "and its first turn ended"
    );
    assert!(
        world.posts().iter().any(|(at, text)| {
            *at == in_room(2)
                && *text
                    == stageman_foreman::stopped_notice(
                        &Waiting::Silent,
                        Some("<@U0HUMAN>"),
                        "<@U0BOT>",
                    )
        }),
        "its room was told when the turn ended: {:?}",
        world.posts()
    );
}

/// A job may not start jobs, and a kit the project does not offer is refused
/// with what it does offer.
#[test]
fn starting_is_refused_to_a_job_and_for_a_kit_the_project_does_not_offer() {
    let (mut world, mut instance, foreman) = with_a_foreman_working();
    let asked1 = world.calls(
        160,
        &foreman,
        &call(
            "start_job",
            serde_json::json!({
                "reason": "why", "instructions": "what", "kit": "Nope",
            }),
        ),
    );
    world.run_until(&mut instance, 200);
    let refused = world.tool_answer(asked1).expect("answered");
    assert!(is_error(refused));
    assert!(
        text_of(refused).contains("offers no kit called \"Nope\""),
        "{}",
        text_of(refused)
    );
    assert!(
        text_of(refused).contains("Claude"),
        "what it does offer: {}",
        text_of(refused)
    );

    // A job's warrant, from a job resumed by a reply.
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
    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 150);
    let job_warrant = world.warrants().last().expect("the job's warrant").clone();
    let asked2 = world.calls(
        160,
        &job_warrant,
        &call(
            "start_job",
            serde_json::json!({
                "reason": "why", "instructions": "what", "kit": "Claude",
            }),
        ),
    );
    world.run_until(&mut instance, 200);
    let refused = world.tool_answer(asked2).expect("answered");
    assert!(is_error(refused));
    assert!(text_of(refused).contains("serves no tool called \"start_job\""));
}

/// Saying posts under the message named, or at the root of the speaker's
/// own room when none is; the agent is told what was posted, by the
/// identifier it may name later, only once the platform has answered.
#[test]
fn saying_posts_where_it_is_told_to_and_reports_a_failure_to_the_agent() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    let asked0 = world.calls(
        155,
        &warrant,
        &call(
            "say",
            serde_json::json!({"message": "On it.", "to": "C0123456789/1788000000.000001"}),
        ),
    );
    let asked1 = world.calls(
        160,
        &warrant,
        &call("say", serde_json::json!({"message": "Thinking aloud."})),
    );
    world.run_until(&mut instance, 200);
    let answer = world.tool_answer(asked0).expect("answered");
    assert!(!is_error(answer));
    assert!(
        text_of(answer).starts_with("C0123456789/"),
        "the identifier of what was posted: {}",
        text_of(answer)
    );
    assert!(world.posts().contains(&(in_thread(1), "On it.".to_owned())));
    let answer = world.tool_answer(asked1).expect("answered");
    assert!(!is_error(answer));
    assert!(
        text_of(answer).starts_with("C-job-001/"),
        "posted in the foreman's own room: {}",
        text_of(answer)
    );
    assert!(
        world
            .posts()
            .contains(&(in_room(1), "Thinking aloud.".to_owned())),
        "{:?}",
        world.posts()
    );
    let asked = world.calls(
        205,
        &warrant,
        &call(
            "say",
            serde_json::json!({"message": "Elsewhere.", "to": "not-a-message"}),
        ),
    );
    world.run_until(&mut instance, 208);
    let refused = world.tool_answer(asked).expect("answered");
    assert!(is_error(refused));
    assert!(
        text_of(refused).contains("not a message as it was shown to you"),
        "{}",
        text_of(refused)
    );

    world.next_post_fails("channel_not_found");
    let asked2 = world.calls(
        210,
        &warrant,
        &call("say", serde_json::json!({"message": "Again."})),
    );
    let asked3 = world.calls(
        211,
        &warrant,
        &call("say", serde_json::json!({"message": "   "})),
    );
    world.run_until(&mut instance, 300);
    let failed = world.tool_answer(asked2).expect("answered");
    assert!(is_error(failed), "{failed:?}");
    assert_eq!(
        text_of(failed),
        "it could not be said: the channel refused it: channel_not_found",
        "the platform's own reason reaches the agent, and so does what kind of failure it was"
    );
    let empty = world.tool_answer(asked3).expect("answered");
    assert!(is_error(empty));
    assert_eq!(text_of(empty), "nothing was said, so nothing was posted");

    // Answered where it was asked, so the thread is not signposted to the
    // foreman's room when the turn ends: what a post through the tool
    // counts as, for the speaker asked there.
    world.run_until(&mut instance, 5_000);
    assert_eq!(
        world
            .posts()
            .iter()
            .filter(|(place, _)| *place == in_thread(1))
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        vec!["On it."],
        "nothing but the answer in the thread: {:?}",
        world.posts()
    );
}

/// A job's claim about why it is stopping is read when its turn ends, and
/// only a job with a turn running may make one.
#[test]
fn a_jobs_claim_is_recorded_when_its_turn_ends() {
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
    world.says_in_room(100, 1, "go on");
    world.run_until(&mut instance, 150);
    let warrant = world.warrants().last().expect("the job's warrant").clone();

    let asked1 = world.calls(
        200,
        &warrant,
        &call(
            "stopping",
            serde_json::json!({"because": "ready_for_review"}),
        ),
    );
    let asked2 = world.calls(
        201,
        &warrant,
        &call("stopping", serde_json::json!({"because": "gave_up"})),
    );
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        text_of(world.tool_answer(asked1).expect("answered")),
        "noted"
    );
    let refused = world.tool_answer(asked2).expect("answered");
    assert!(is_error(refused));
    assert!(
        text_of(refused).contains("not one of the reasons"),
        "{}",
        text_of(refused)
    );
    assert_eq!(
        instance.state().job(idle).expect("the job").progress,
        Progress::Idle(Waiting::Proposed),
        "the claim was read when the turn ended"
    );

    // A foreman may not, and a warrant from a turn that has ended names
    // nobody any more.
    let (mut world, mut instance, foreman) = with_a_foreman_working();
    let asked3 = world.calls(
        160,
        &foreman,
        &call(
            "stopping",
            serde_json::json!({"because": "ready_for_review"}),
        ),
    );
    world.run_until(&mut instance, 5_000);
    let refused = world.tool_answer(asked3).expect("answered");
    assert!(is_error(refused));
    let asked4 = world.calls(
        6_000,
        &foreman,
        &call("say", serde_json::json!({"message": "late"})),
    );
    world.run_until(&mut instance, 6_100);
    assert_eq!(
        world.tool_answer(asked4).map(|a| a.0),
        Some(403),
        "the turn ended, the warrant with it"
    );
}

/// The same seed gives the same trace through a job's whole life.
#[test]
fn a_jobs_whole_life_runs_the_same_twice() {
    fn scenario() -> Vec<String> {
        let (mut world, mut instance, warrant) = with_a_foreman_working();
        let _ = world.calls(
            160,
            &warrant,
            &call(
                "start_job",
                serde_json::json!({
                    "reason": "why", "instructions": "what", "kit": "Claude",
                }),
            ),
        );
        world.run_until(&mut instance, 3_000);
        world.shape()
    }
    assert_eq!(scenario(), scenario());
}
