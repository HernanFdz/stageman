//! What an instance does on waking, on settling, and when a turn ends, run
//! against the simulated world.

use std::collections::BTreeMap;

use crate::simulation::{
    Held, Simulation, another_instance, job, project, seed, this_instance, watching,
};
use stageman_agent::{Answer, StopReason};
use stageman_core::{Agent, JobId, Outcome, Progress, ProjectId, State, Uuid, Waiting};

fn progress_of(state: &State, id: JobId) -> Progress {
    state.job(id).expect("the job").progress.clone()
}

/// A container of this instance's, up, that is or is not showing something.
const fn up(serving: bool) -> Held {
    Held {
        instance: Some(this_instance()),
        agent: Some(Agent::Claude),
        running: true,
        serving,
        port: None,
    }
}

const fn ended(stop_reason: StopReason) -> Answer {
    Answer {
        text: String::new(),
        stop_reason,
        reported: BTreeMap::new(),
    }
}

/// The trace's shape with the one varying number taken out.
fn shape_of(world: &Simulation) -> Vec<String> {
    world
        .shape()
        .into_iter()
        .map(|line| {
            line.split_once(" { bytes")
                .map_or_else(|| line.clone(), |(head, _)| head.to_owned())
        })
        .collect()
}

/// A first run writes a file at once, and asks for nothing but its timers.
#[test]
fn a_first_run_writes_its_file_and_settles_later() {
    let mut world = Simulation::new();
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 60_000);

    assert_eq!(
        shape_of(&world),
        [
            "woke, swept Swept { resumed: 0, lost: 0, cleared: 0, unidentified: 0, forgotten: 0, unclaimed: 0, elsewhere: 0 }",
            "-> Reclaim",
            "-> Wake { after: 60s, timer: Settle }",
            "-> Persist",
            "<- Persisted { outcome: Ok(()) }",
            "<- Woke { timer: Settle }",
            "-> ListRunning",
            "-> Wake { after: 60s, timer: Settle }",
            "<- Listed { running: [] }",
        ]
    );
    assert!(world.disk().is_some(), "a first run has a file");
}

/// An instance writes itself once on waking, whether or not the file
/// changed — so a path that cannot be written fails at startup rather than
/// at the first change, `docs/conventions.md` §3 — and after that only when
/// something changed.
#[test]
fn an_instance_writes_once_on_waking_and_then_only_when_something_changed() {
    let mut world = Simulation::new();
    world.holding(&watching(&[]));
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);
    let written = |world: &Simulation| {
        world
            .shape()
            .iter()
            .filter(|line| line.starts_with("-> Persist"))
            .count()
    };
    assert_eq!(written(&world), 1, "{:?}", world.shape());

    // A settle that finds nothing to do changes nothing, and writes nothing.
    world.run_until(&mut instance, 60_000);
    assert_eq!(written(&world), 1, "{:?}", world.shape());
}

/// What a job's session reports it was set to is written down beside the
/// kit, this turn, and reaches the disk.
#[test]
fn what_a_session_reported_is_recorded_beside_the_kit() {
    let mut world = Simulation::new();
    world.holding(&watching(&[(job(1), Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(job(1)));
    world.container(&name, held);
    world.next_turn_ends(Ok(Answer {
        text: "done".to_owned(),
        stop_reason: StopReason::EndTurn,
        reported: std::collections::BTreeMap::from([(
            "model".to_owned(),
            "claude-opus-4".to_owned(),
        )]),
    }));
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 5_000);

    let reported = |state: &stageman_core::State| {
        state
            .job(job(1))
            .expect("the job")
            .reported
            .get("model")
            .cloned()
    };
    assert_eq!(reported(instance.state()), Some("claude-opus-4".to_owned()));
    assert_eq!(
        reported(&world.disk().expect("landed")),
        Some("claude-opus-4".to_owned())
    );
}

/// A working job with a container is put back to work, and what its turn
/// comes to is recorded and reaches the disk.
#[test]
fn a_working_job_with_a_container_is_resumed_and_its_ending_recorded() {
    let mut world = Simulation::new();
    let working = job(1);
    world.holding(&watching(&[(working, Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(working));
    world.container(&name, held);

    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 2_000);

    let shape = world.shape();
    assert!(
        shape
            .iter()
            .any(|line| line.starts_with("-> RunTurn { speaker: Job(") && line.contains("Resume")),
        "{shape:?}"
    );
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent)
    );
    assert_eq!(
        world.disk().map(|state| progress_of(&state, working)),
        Some(Progress::Idle(Waiting::Silent)),
        "the ending reached the disk"
    );
    assert!(
        !world.is_running(&name),
        "nothing answered on its tunnel, so the container was stopped"
    );
    assert!(
        world.posts().is_empty(),
        "a turn that waking put back to work tells nobody when it ends"
    );
}

/// A resumed turn that could not be run is recorded as failed, with the
/// reason.
#[test]
fn a_resumed_turn_that_fails_is_recorded_as_failed() {
    let mut world = Simulation::new();
    let working = job(1);
    world.holding(&watching(&[(working, Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(working));
    world.container(&name, held);
    world.next_turn_ends(Err("the credential was refused".to_owned()));

    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 2_000);

    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Failed("the credential was refused".to_owned()))
    );
}

/// A turn that ended badly is recorded as failed, saying how.
#[test]
fn a_turn_cut_short_is_a_failure_that_names_the_reason() {
    let mut world = Simulation::new();
    let working = job(1);
    world.holding(&watching(&[(working, Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(working));
    world.container(&name, held);
    world.next_turn_ends(Ok(ended(StopReason::MaxTokens)));

    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 2_000);

    let Progress::Idle(Waiting::Failed(why)) = progress_of(instance.state(), working) else {
        panic!("a turn cut short is not a finished job");
    };
    assert!(why.contains("MaxTokens"), "{why}");
}

/// A job with nothing to run in is over, working or idle, and the record of
/// it reaches the disk so that the next waking does not say so again.
#[test]
fn a_job_whose_container_is_gone_is_lost_once() {
    let mut world = Simulation::new();
    let working = job(1);
    let idle = job(2);
    world.holding(&watching(&[
        (working, Progress::Working),
        (idle, Progress::Idle(Waiting::Proposed)),
    ]));

    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);

    for lost in [working, idle] {
        assert_eq!(
            progress_of(instance.state(), lost),
            Progress::Retired(Outcome::Lost)
        );
    }
    let disk = world.disk().expect("a file");
    assert_eq!(
        progress_of(&disk, working),
        Progress::Retired(Outcome::Lost)
    );

    let again = world.crash(seed(2));
    let woke = world
        .trace()
        .iter()
        .rev()
        .find(|line| line.contains("woke, swept"))
        .expect("a second waking");
    assert!(
        woke.contains("lost: 0"),
        "a second waking finds nothing to lose: {woke}"
    );
    assert_eq!(
        progress_of(again.state(), idle),
        Progress::Retired(Outcome::Lost)
    );
}

/// What waking removes, and what it leaves alone.
#[test]
fn waking_removes_what_is_ours_and_over_and_leaves_the_rest() {
    let mut world = Simulation::new();
    let retired = job(1);
    let forgotten = job(2);
    let theirs = job(3);
    let unlabelled = job(4);
    let gone_project = ProjectId::from_uuid(Uuid::from_u128(404));
    world.holding(&watching(&[(retired, Progress::Retired(Outcome::Done))]));

    for name in [
        stageman_job::container(retired),
        stageman_job::container(forgotten),
        "stageman-job-from-an-older-scheme".to_owned(),
        stageman_foreman::container(project()),
        stageman_foreman::container(gone_project),
    ] {
        let (name, held) = Simulation::ours(&name);
        world.container(&name, held);
    }
    world.container(
        &stageman_job::container(theirs),
        Held {
            instance: Some(another_instance()),
            agent: Some(Agent::Claude),
            running: true,
            serving: false,
            port: None,
        },
    );
    world.container(
        &stageman_job::container(unlabelled),
        Held {
            instance: None,
            agent: None,
            running: false,
            serving: false,
            port: None,
        },
    );

    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 10);

    assert!(
        !world.exists(&stageman_job::container(retired)),
        "over, so removed"
    );
    assert!(
        !world.exists(&stageman_job::container(forgotten)),
        "ours and unknown, so removed"
    );
    assert!(
        world.exists(&stageman_job::container(theirs)),
        "another instance's, left alone"
    );
    assert!(
        world.exists(&stageman_job::container(unlabelled)),
        "cannot be attributed, left alone"
    );
    assert!(
        !world.exists("stageman-job-from-an-older-scheme"),
        "ours under an old name, removed"
    );
    assert!(
        world.exists(&stageman_foreman::container(project())),
        "a watched project's foreman stays"
    );
    assert!(
        !world.exists(&stageman_foreman::container(gone_project)),
        "a gone project's foreman goes"
    );
    assert_eq!(
        world.reclaims(),
        1,
        "images are reclaimed once, after the removals"
    );
    let woke = world.trace().first().expect("the waking line");
    assert!(
        woke.contains("Swept { resumed: 0, lost: 0, cleared: 1, unidentified: 2, forgotten: 1, unclaimed: 1, elsewhere: 1 }"),
        "{woke}"
    );
}

/// Settling stops what shows nothing and keeps what shows something, and
/// never touches a container with a turn in it.
#[test]
fn settling_stops_what_shows_nothing_and_keeps_what_shows_something() {
    let mut world = Simulation::new();
    let showing = job(1);
    let silent = job(2);
    let working = job(3);
    world.holding(&watching(&[
        (showing, Progress::Idle(Waiting::Proposed)),
        (silent, Progress::Idle(Waiting::Asked)),
        (working, Progress::Working),
    ]));
    for (id, serving) in [(showing, true), (silent, false), (working, false)] {
        world.container(&stageman_job::container(id), up(serving));
    }

    let mut instance = world.wake(seed(1));
    // Waking probes what is up and idle. The working job's container is
    // resumed, not probed.
    world.run_until(&mut instance, 10);
    assert!(world.is_running(&stageman_job::container(showing)));
    assert!(!world.is_running(&stageman_job::container(silent)));
    assert!(world.is_running(&stageman_job::container(working)));

    // Later the turn ends and its container is probed, and settling comes
    // round to find the showing one still showing.
    world.run_until(&mut instance, 61_000);
    assert!(
        world.is_running(&stageman_job::container(showing)),
        "still showing, still up"
    );
    assert!(
        !world.is_running(&stageman_job::container(working)),
        "its turn ended showing nothing"
    );
    assert_eq!(
        progress_of(instance.state(), working),
        Progress::Idle(Waiting::Silent)
    );
    let listed = world
        .shape()
        .iter()
        .filter(|line| line.starts_with("<- Listed"))
        .count();
    assert_eq!(listed, 1, "settling came round once: {:?}", world.shape());
}

/// Settling asks only about what is ours: a stranger's running container is
/// never probed, and one of ours that the instance has no record of is.
#[test]
fn settling_asks_only_about_what_is_ours() {
    let mut world = Simulation::new();
    let ours = job(1);
    let theirs = job(2);
    world.holding(&watching(&[]));
    world.container(
        &stageman_job::container(theirs),
        Held {
            instance: Some(another_instance()),
            agent: Some(Agent::Claude),
            running: true,
            serving: false,
            port: None,
        },
    );
    let mut instance = world.wake(seed(1));
    // Appears after waking, as a container another process of this instance
    // might have started: no record, our label.
    world.container(&stageman_job::container(ours), up(false));
    world.run_until(&mut instance, 61_000);

    assert!(
        !world.is_running(&stageman_job::container(ours)),
        "ours, so asked and stopped"
    );
    assert!(
        world.is_running(&stageman_job::container(theirs)),
        "theirs, so left running"
    );
}

/// The same seed gives the same trace, across a crash that kills two turns
/// mid-flight and a restart that puts both back to work.
#[test]
fn the_same_seed_gives_the_same_trace_across_a_crash() {
    fn scenario(seed_byte: u8) -> (Vec<String>, Progress, Progress) {
        let mut world = Simulation::new();
        let one = job(1);
        let two = job(2);
        world.holding(&watching(&[
            (one, Progress::Working),
            (two, Progress::Working),
        ]));
        for id in [one, two] {
            let (name, held) = Simulation::ours(&stageman_job::container(id));
            world.container(&name, held);
        }

        let mut instance = world.wake(seed(seed_byte));
        world.run_until(&mut instance, 500);
        let mut instance = world.crash(seed(seed_byte));
        world.run_until(&mut instance, 5_000);
        (
            world.shape(),
            progress_of(instance.state(), one),
            progress_of(instance.state(), two),
        )
    }

    let (first, one, two) = scenario(3);
    let (again, _, _) = scenario(3);
    assert_eq!(first, again, "the same seed must give the same trace");
    assert_eq!(
        one,
        Progress::Idle(Waiting::Silent),
        "resumed after the crash and finished"
    );
    assert_eq!(two, Progress::Idle(Waiting::Silent));
    assert_eq!(
        first.iter().filter(|line| line.contains("CRASH")).count(),
        1
    );
    assert_eq!(
        first
            .iter()
            .filter(|line| line.starts_with("-> RunTurn"))
            .count(),
        4,
        "each job was put to work twice, once each side of the crash: {first:?}"
    );
    assert_eq!(
        first
            .iter()
            .filter(|line| line.starts_with("<- TurnEnded"))
            .count(),
        2,
        "the turns cut off by the crash never ended: {first:?}"
    );
}

/// Nothing a turn is given prints a credential, and the warrant it presents
/// is known while the turn runs and forgotten when it ends.
#[test]
fn a_turn_prints_no_credential_and_its_warrant_lives_as_long_as_it_does() {
    let mut world = Simulation::new();
    let working = job(1);
    world.holding(&watching(&[(working, Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(working));
    world.container(&name, held);

    let mut instance = world.wake(seed(1));
    let run = world
        .shape()
        .into_iter()
        .find(|line| line.starts_with("-> RunTurn"))
        .expect("a turn was run");
    let warrant = world
        .warrants()
        .first()
        .expect("a warrant was handed over")
        .clone();
    assert!(!run.contains("agent-token"), "{run}");
    assert!(!run.contains(warrant.expose()), "{run}");
    assert!(
        warrant.expose().len() >= 64,
        "unguessable: {}",
        warrant.expose().len()
    );
    assert!(
        instance.warranted(warrant.expose()).is_some(),
        "known while the turn runs"
    );
    assert!(instance.warranted("not-a-warrant").is_none());

    world.run_until(&mut instance, 2_000);
    assert!(
        instance.warranted(warrant.expose()).is_none(),
        "forgotten when the turn ends"
    );
}
