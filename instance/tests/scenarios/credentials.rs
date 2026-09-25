//! A job fetches its project's credential from this instance and never
//! carries it — see
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`:
//! the warrant minted with a job and delivered into its container in place
//! of the token, the wrapper written in before the checkout and again on
//! every resume, the route that serves the credential to the job's own
//! warrant and refuses every other, and a warrant that survives a crash.

use stageman_agent::Command;
use stageman_core::{JobId, Outcome, Progress, Waiting};
use stageman_instance::{Instance, Request, Response};

use crate::simulation::{
    FARAWAY, NEARBY, Simulation, holding_a_token, job, project, request, seed, warrant_of,
    watching, watching_a_channel, without_a_warrant,
};

/// A token the fixture's project holds, and never a container.
const TOKEN: &str = "ghp-not-a-real-token";

/// Starts a job by hand, lets its record land, and says which it is.
fn started(sim: &mut Simulation, instance: &mut Instance, id: u64, work: &str) -> JobId {
    for effect in instance.step(
        sim.now(),
        request(
            id,
            Request::Start {
                project: project().to_string(),
                kit: "Claude".to_owned(),
                work: work.to_owned(),
                title: String::new(),
            },
        ),
    ) {
        sim.perform(effect);
    }
    let until = sim.now() + 5;
    sim.run_until(instance, until);
    let Some(Response::Jobs(shown)) = sim.response(id).cloned() else {
        panic!("the project's screen, once the record landed");
    };
    let listed = shown
        .jobs
        .iter()
        .find(|listed| listed.kickoff.contains(work))
        .expect("the job just started");
    JobId::parse(&listed.id).expect("a name")
}

/// A request on the credential route, presenting something or nothing.
fn fetching(sim: &mut Simulation, at: u64, bearer: Option<&str>, peer: &str) -> Asked {
    let header = bearer.map(|bearer| format!("Bearer {bearer}"));
    let headers: Vec<(&str, &str)> = header
        .as_deref()
        .map(|value| ("authorization", value))
        .into_iter()
        .collect();
    sim.arrives(at, "GET", "/credential", &headers, peer, "")
}

type Asked = stageman_vocabulary::RequestId;

fn listing() -> serde_json::Value {
    let mut envelope = serde_json::Map::new();
    envelope.insert("jsonrpc".to_owned(), "2.0".into());
    envelope.insert("id".to_owned(), 1.into());
    envelope.insert("method".to_owned(), "tools/list".into());
    envelope.insert("params".to_owned(), serde_json::json!({}));
    serde_json::Value::Object(envelope)
}

/// Where in the trace the runtime was first asked something of a container.
fn first(sim: &Simulation, wanted: impl Fn(&Command) -> bool) -> usize {
    sim.first_asking(wanted)
        .unwrap_or_else(|| panic!("never asked: {:?}", sim.commands()))
}

/// A job begun is given a warrant, kept sealed on its record; its container
/// is created with the warrant and without the token; the wrapper is
/// written in after the container starts and before the checkout, naming
/// the route on this instance's listener; and the checkout is the
/// platform's, since the project holds access to it.
#[test]
fn a_job_begun_holds_a_warrant_and_its_container_carries_no_token() {
    let mut sim = Simulation::new();
    let mut state = watching_a_channel(&[]);
    holding_a_token(&mut state, TOKEN);
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));

    let job = started(&mut sim, &mut instance, 1, "fetch something");
    sim.run_until(&mut instance, 5_000);

    let warrant = instance
        .state()
        .job(&job)
        .and_then(|recorded| recorded.warrant())
        .expect("a job created now holds a warrant")
        .expose()
        .to_owned();
    assert!(warrant.len() >= 64, "unguessable: {}", warrant.len());
    let container = stageman_job::container(&job);
    let environment = sim
        .environment_of(&container)
        .expect("the container was made");
    assert_eq!(environment.get("STAGEMAN_WARRANT"), Some(&warrant));
    assert!(
        !environment.contains_key("GH_TOKEN"),
        "no token rests in the environment: {environment:?}"
    );
    assert!(
        !environment.values().any(|value| value == TOKEN),
        "under any name: {environment:?}"
    );

    let starting = first(
        &sim,
        |command| matches!(command, Command::Start { name } if *name == container),
    );
    let wrapping = first(
        &sim,
        |command| matches!(command, Command::Wrap { name } if *name == container),
    );
    let checking_out = first(&sim, |command| {
        matches!(
            command,
            Command::Checkout { name, platform: Some(stageman_core::Platform::GitHub), actor: None, .. }
                if *name == container
        )
    });
    assert!(
        starting < wrapping && wrapping < checking_out,
        "started, then wrapped, then checked out: {:?}",
        sim.commands()
    );
    let wrapper = sim
        .wrapper_of(&container)
        .expect("the wrapper was written in");
    assert!(
        wrapper.contains("'http://host.docker.internal:") && wrapper.contains("/credential'"),
        "it names the route on this instance's listener: {wrapper}"
    );
    assert!(wrapper.contains("Bearer $STAGEMAN_WARRANT"), "{wrapper}");
    assert!(!wrapper.contains(TOKEN), "{wrapper}");

    // Kept sealed: on the record after a reopen, and nowhere in the clear.
    let kept = sim.disk().expect("landed");
    assert_eq!(
        kept.job(&job)
            .and_then(|recorded| recorded.warrant())
            .map(stageman_core::Secret::expose),
        Some(warrant.as_str())
    );
    let bytes = sim.disk_bytes().expect("landed");
    assert!(!String::from_utf8_lossy(bytes).contains(&warrant));
}

/// The route serves the credential to the job's own warrant, at once and
/// as text, and to nothing else: not a turn's warrant, which buys the tools
/// and never this; not a warrant this instance never minted; not nothing;
/// not from beyond this machine. And a job's warrant buys no tools.
#[test]
fn the_credential_route_serves_a_jobs_own_warrant_and_refuses_every_other() {
    let mut sim = Simulation::new();
    let working = job(1);
    let mut state = watching(&[(working.clone(), Progress::Working)]);
    holding_a_token(&mut state, TOKEN);
    sim.holding(&state);
    let (name, held) = Simulation::ours(&stageman_job::container(&working));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);
    let turns = sim
        .warrants()
        .first()
        .expect("the resumed turn's warrant")
        .clone();
    let jobs = warrant_of(&working);

    let own = fetching(&mut sim, 20, Some(&jobs), NEARBY);
    let with_the_turns = fetching(&mut sim, 21, Some(&turns), NEARBY);
    let from_afar = fetching(&mut sim, 22, Some(&jobs), FARAWAY);
    let never_minted = fetching(&mut sim, 23, Some("not-a-warrant"), NEARBY);
    let nothing = fetching(&mut sim, 24, None, NEARBY);
    let tools_with_the_jobs = sim.calls(25, &jobs, &listing());
    sim.run_until(&mut instance, 40);

    assert_eq!(sim.tool_answer(own).map(|answer| answer.0), Some(200));
    assert_eq!(sim.answer_text(own), Some(TOKEN));
    for (refused, why) in [
        (
            with_the_turns,
            "a turn's warrant buys the tools and never this",
        ),
        (from_afar, "nowhere a container is"),
        (never_minted, "a warrant this instance never minted"),
        (nothing, "nothing presented"),
        (tools_with_the_jobs, "a job's warrant buys no tools"),
    ] {
        assert_eq!(
            sim.tool_answer(refused).map(|answer| answer.0),
            Some(403),
            "{why}"
        );
        assert_ne!(sim.answer_text(refused), Some(TOKEN), "{why}");
    }
}

/// A job whose project holds no credential is told so, and a retired job's
/// warrant buys nothing: its container is gone with everything in it.
#[test]
fn a_project_without_a_token_answers_not_found_and_a_retired_job_is_refused() {
    let mut sim = Simulation::new();
    let idle = job(2);
    let over = job(3);
    sim.holding(&watching(&[
        (idle.clone(), Progress::Idle(Waiting::Asked)),
        (over.clone(), Progress::Retired(Outcome::Done)),
    ]));
    let (name, held) = Simulation::ours(&stageman_job::container(&idle));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);

    let no_token = fetching(&mut sim, 20, Some(&warrant_of(&idle)), NEARBY);
    let retired = fetching(&mut sim, 21, Some(&warrant_of(&over)), NEARBY);
    sim.run_until(&mut instance, 40);

    assert_eq!(sim.tool_answer(no_token).map(|answer| answer.0), Some(404));
    assert_eq!(
        sim.answer_text(no_token),
        Some("this job's project holds no credential for its repository")
    );
    assert_eq!(sim.tool_answer(retired).map(|answer| answer.0), Some(403));
}

/// A warrant is kept, so a daemon dying and starting again finds it: the
/// job's container is resumed with its wrapper rewritten, and the route
/// answers the same warrant with the same credential.
#[test]
fn a_jobs_warrant_survives_the_daemon_dying() {
    let mut sim = Simulation::new();
    let working = job(1);
    let mut state = watching(&[(working.clone(), Progress::Working)]);
    holding_a_token(&mut state, TOKEN);
    sim.holding(&state);
    let container = stageman_job::container(&working);
    let (name, held) = Simulation::ours(&container);
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);
    let wrapped_before = sim
        .commands()
        .iter()
        .filter(|command| matches!(command, Command::Wrap { .. }))
        .count();
    assert_eq!(wrapped_before, 1, "wrapped on the first resume");

    let mut instance = sim.crash(seed(2));
    sim.run_until(&mut instance, 30);

    let wrapped_after = sim
        .commands()
        .iter()
        .filter(|command| matches!(command, Command::Wrap { name } if *name == container))
        .count();
    assert_eq!(wrapped_after, 2, "and again on the next");
    let own = fetching(&mut sim, 40, Some(&warrant_of(&working)), NEARBY);
    sim.run_until(&mut instance, 60);
    assert_eq!(sim.tool_answer(own).map(|answer| answer.0), Some(200));
    assert_eq!(sim.answer_text(own), Some(TOKEN));
}

/// A job the last release wrote holds no warrant, and its container was
/// created with the credential itself in its environment: it is resumed
/// as it was, with no wrapper written in, while a job that holds a
/// warrant has its wrapper rewritten before its agent runs.
#[test]
fn a_job_from_before_warrants_is_resumed_as_it_was() {
    let mut sim = Simulation::new();
    let warranted = job(1);
    let older = job(2);
    let mut state = watching(&[
        (warranted.clone(), Progress::Working),
        (older.clone(), Progress::Working),
    ]);
    let recorded = state.job(&older).expect("the older job").clone();
    *state.job_mut(&older).expect("the older job") = without_a_warrant(&recorded);
    assert!(state.job(&older).and_then(|job| job.warrant()).is_none());
    sim.holding(&state);
    for job in [&warranted, &older] {
        let (name, held) = Simulation::ours(&stageman_job::container(job));
        sim.container(&name, held);
    }
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);

    let with = stageman_job::container(&warranted);
    let without = stageman_job::container(&older);
    assert!(
        sim.wrapper_of(&with).is_some(),
        "rewrapped: {:?}",
        sim.commands()
    );
    assert!(
        sim.wrapper_of(&without).is_none(),
        "left as it was: {:?}",
        sim.commands()
    );
    assert!(!sim.talks_in(&with).is_empty() && !sim.talks_in(&without).is_empty());
    // And the older job's warrant, being none, buys nothing.
    let asked = fetching(&mut sim, 5_010, Some(&warrant_of(&older)), NEARBY);
    sim.run_until(&mut instance, 5_020);
    assert_eq!(sim.tool_answer(asked).map(|answer| answer.0), Some(403));
}

/// A job on a project that reaches the platform through an installation
/// of the App checks out as the App's bot: the commit identity the
/// checkout sets is the App's name marked as a bot, and a token's project
/// sets none, since its token says who it is — see
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.
#[test]
fn a_jobs_checkout_commits_as_the_apps_bot_on_an_installation() {
    let mut sim = Simulation::new();
    let mut state = watching_a_channel(&[]);
    crate::simulation::with_an_app(&mut state);
    crate::simulation::installed(&mut state, 77);
    crate::simulation::on_github(&mut state, "example/repo");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));

    let job = started(&mut sim, &mut instance, 1, "fetch something");
    sim.run_until(&mut instance, 5_000);
    let container = stageman_job::container(&job);
    let actor = sim.commands().iter().find_map(|command| match command {
        Command::Checkout { name, actor, .. } if *name == container => Some(actor.clone()),
        _ => None,
    });
    assert_eq!(
        actor,
        Some(Some("stageman-sim[bot]".to_owned())),
        "checked out as the App's bot: {:?}",
        sim.commands()
    );
}
