//! The tools endpoint, called by the agents this instance runs, against the
//! simulated world.

use crate::simulation::{
    Simulation, job, said_at_root, seed, thread, tool_call, watching_a_channel,
};
use stageman_core::{Progress, Waiting};
use stageman_instance::RequestId;

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
fn with_a_foreman_working() -> (
    Simulation,
    stageman_instance::Instance,
    stageman_core::Secret,
) {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.schedule(100, said_at_root(1, "look at the parser"));
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

    let mut faraway = tool_call(1, &warrant, body("tools/list", serde_json::json!({})));
    if let stageman_instance::Event::ToolCalled { nearby, .. } = &mut faraway {
        *nearby = false;
    }
    world.schedule(160, faraway);
    world.schedule(
        161,
        tool_call(
            2,
            &stageman_core::Secret::new("not-minted".to_owned()),
            body("tools/list", serde_json::json!({})),
        ),
    );
    world.schedule(
        162,
        tool_call(3, &warrant, body("tools/list", serde_json::json!({}))),
    );
    world.run_until(&mut instance, 200);

    assert_eq!(world.tool_answer(RequestId(1)).map(|a| a.0), Some(403));
    assert_eq!(world.tool_answer(RequestId(2)).map(|a| a.0), Some(403));
    assert_eq!(world.tool_answer(RequestId(3)).map(|a| a.0), Some(200));
}

/// The handshake, a notification, and the listing a foreman is offered.
#[test]
fn a_foreman_is_greeted_and_offered_the_tools_that_start_jobs() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    world.schedule(
        160,
        tool_call(
            1,
            &warrant,
            body(
                "initialize",
                serde_json::json!({"protocolVersion": "2024-11-05"}),
            ),
        ),
    );
    world.schedule(
        161,
        tool_call(
            2,
            &warrant,
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        ),
    );
    world.schedule(
        162,
        tool_call(3, &warrant, body("tools/list", serde_json::json!({}))),
    );
    world.schedule(
        163,
        tool_call(4, &warrant, serde_json::json!({"not": "a request"})),
    );
    world.run_until(&mut instance, 200);

    let greeting = world.tool_answer(RequestId(1)).expect("answered");
    assert_eq!(greeting.0, 200);
    let result = &greeting.1.as_ref().expect("a body")["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert_eq!(result["serverInfo"]["name"], "stageman");
    assert_eq!(result["serverInfo"]["version"], "a test build");

    assert_eq!(
        world.tool_answer(RequestId(2)),
        Some(&(202, None)),
        "a notification wants no answer"
    );

    let listing = world.tool_answer(RequestId(3)).expect("answered");
    let names: Vec<&str> = listing.1.as_ref().expect("a body")["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["say", "start_job"]);
    assert_eq!(
        listing.1.as_ref().expect("a body")["result"]["tools"][1]["inputSchema"]["properties"]["kit"]
            ["enum"],
        serde_json::json!(["Claude"]),
        "the project's kits are enumerated"
    );

    assert_eq!(world.tool_answer(RequestId(4)).map(|a| a.0), Some(400));
}

/// A foreman starts a job: the job is recorded, its thread is opened once
/// the record has landed, the agent is told once the thread is on the record,
/// and the foreman is answered with the job's identifier.
#[test]
fn a_foreman_starts_a_job_whose_thread_is_opened_before_its_agent_speaks() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    world.schedule(
        160,
        tool_call(
            1,
            &warrant,
            call(
                "start_job",
                serde_json::json!({
                    "reason": "the parser is flaky",
                    "instructions": "Fix the flaky test in the parser.",
                    "kit": "Claude",
                }),
            ),
        ),
    );
    world.run_until(&mut instance, 10_000);

    let answer = world.tool_answer(RequestId(1)).expect("answered");
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
        .position(|line| line.starts_with("<- Persisted"))
        .expect("written");
    let opened = shape
        .iter()
        .position(|line| line.starts_with("-> OpenThread"))
        .expect("opened");
    let begun = shape
        .iter()
        .position(|line| line.starts_with("-> RunTurn { speaker: Job("))
        .expect("begun");
    assert!(
        persisted < opened && opened < begun,
        "record, then thread, then agent: {shape:?}"
    );
    let run = &shape[begun];
    assert!(run.contains("Begin"), "{run}");
    assert!(run.contains("Fix the flaky test in the parser."), "{run}");
    assert!(
        run.contains("the `say` tool"),
        "a job with a channel is told how to speak: {run}"
    );

    let recorded = instance
        .state()
        .job(started)
        .expect("the job is on the record");
    assert_eq!(recorded.reason, "the parser is flaky");
    assert_eq!(
        recorded.thread,
        Some(thread(101)),
        "in the thread that was opened"
    );
    assert_eq!(
        recorded.progress,
        Progress::Idle(Waiting::Silent),
        "and its first turn ended"
    );
    assert!(
        world
            .posts()
            .iter()
            .any(|(t, text)| *t == thread(101) && text == stageman_foreman::attention_notice()),
        "its thread was told when the turn ended: {:?}",
        world.posts()
    );
}

/// A job may not start jobs, and a kit the project does not offer is refused
/// with what it does offer.
#[test]
fn starting_is_refused_to_a_job_and_for_a_kit_the_project_does_not_offer() {
    let (mut world, mut instance, foreman) = with_a_foreman_working();
    world.schedule(
        160,
        tool_call(
            1,
            &foreman,
            call(
                "start_job",
                serde_json::json!({
                    "reason": "why", "instructions": "what", "kit": "Nope",
                }),
            ),
        ),
    );
    world.run_until(&mut instance, 200);
    let refused = world.tool_answer(RequestId(1)).expect("answered");
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
    world.schedule(100, crate::simulation::said_in(1, "go on"));
    world.run_until(&mut instance, 150);
    let job_warrant = world.warrants().last().expect("the job's warrant").clone();
    world.schedule(
        160,
        tool_call(
            2,
            &job_warrant,
            call(
                "start_job",
                serde_json::json!({
                    "reason": "why", "instructions": "what", "kit": "Claude",
                }),
            ),
        ),
    );
    world.run_until(&mut instance, 200);
    let refused = world.tool_answer(RequestId(2)).expect("answered");
    assert!(is_error(refused));
    assert!(text_of(refused).contains("serves no tool called \"start_job\""));
}

/// Saying posts in the thread the warrant names, and the agent is told
/// whether it was heard only once the platform has answered.
#[test]
fn saying_posts_in_the_warrants_thread_and_reports_a_failure_to_the_agent() {
    let (mut world, mut instance, warrant) = with_a_foreman_working();

    world.schedule(
        160,
        tool_call(
            1,
            &warrant,
            call("say", serde_json::json!({"message": "On it."})),
        ),
    );
    world.run_until(&mut instance, 200);
    let answer = world.tool_answer(RequestId(1)).expect("answered");
    assert!(!is_error(answer));
    assert_eq!(text_of(answer), "said");
    assert!(world.posts().contains(&(thread(1), "On it.".to_owned())));

    world.next_post_fails("channel_not_found");
    world.schedule(
        210,
        tool_call(
            2,
            &warrant,
            call("say", serde_json::json!({"message": "Again."})),
        ),
    );
    world.schedule(
        211,
        tool_call(
            3,
            &warrant,
            call("say", serde_json::json!({"message": "   "})),
        ),
    );
    world.run_until(&mut instance, 300);
    let failed = world.tool_answer(RequestId(2)).expect("answered");
    assert!(is_error(failed), "{failed:?}");
    assert_eq!(text_of(failed), "it could not be said: channel_not_found");
    let empty = world.tool_answer(RequestId(3)).expect("answered");
    assert!(is_error(empty));
    assert_eq!(text_of(empty), "nothing was said, so nothing was posted");
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
    world.schedule(100, crate::simulation::said_in(1, "go on"));
    world.run_until(&mut instance, 150);
    let warrant = world.warrants().last().expect("the job's warrant").clone();

    world.schedule(
        200,
        tool_call(
            1,
            &warrant,
            call(
                "stopping",
                serde_json::json!({"because": "ready_for_review"}),
            ),
        ),
    );
    world.schedule(
        201,
        tool_call(
            2,
            &warrant,
            call("stopping", serde_json::json!({"because": "gave_up"})),
        ),
    );
    world.run_until(&mut instance, 5_000);

    assert_eq!(
        text_of(world.tool_answer(RequestId(1)).expect("answered")),
        "noted"
    );
    let refused = world.tool_answer(RequestId(2)).expect("answered");
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
    world.schedule(
        160,
        tool_call(
            3,
            &foreman,
            call(
                "stopping",
                serde_json::json!({"because": "ready_for_review"}),
            ),
        ),
    );
    world.run_until(&mut instance, 5_000);
    let refused = world.tool_answer(RequestId(3)).expect("answered");
    assert!(is_error(refused));
    world.schedule(
        6_000,
        tool_call(
            4,
            &foreman,
            call("say", serde_json::json!({"message": "late"})),
        ),
    );
    world.run_until(&mut instance, 6_100);
    assert_eq!(
        world.tool_answer(RequestId(4)).map(|a| a.0),
        Some(403),
        "the turn ended, the warrant with it"
    );
}

/// The same seed gives the same trace through a job's whole life.
#[test]
fn a_jobs_whole_life_runs_the_same_twice() {
    fn scenario() -> Vec<String> {
        let (mut world, mut instance, warrant) = with_a_foreman_working();
        world.schedule(
            160,
            tool_call(
                1,
                &warrant,
                call(
                    "start_job",
                    serde_json::json!({
                        "reason": "why", "instructions": "what", "kit": "Claude",
                    }),
                ),
            ),
        );
        world.run_until(&mut instance, 3_000);
        world.shape()
    }
    assert_eq!(scenario(), scenario());
}
