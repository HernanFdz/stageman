//! A turn as the commands and the conversation it is, against the simulated
//! world: an image built once and shared, a build or a checkout that fails,
//! and a stop that lands between steps.

use stageman_agent::Command;
use stageman_core::{JobId, Progress, Waiting};
use stageman_instance::{Instance, Request, Response};

use crate::simulation::{Simulation, job, project, request, seed, watching_a_channel};

/// Asks for a job by hand, and performs what asking caused.
fn asking(sim: &mut Simulation, instance: &mut Instance, id: u64, work: &str) {
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
}

/// Which job a request for one started, once it has been answered.
fn which(sim: &Simulation, id: u64, work: &str) -> JobId {
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

/// Starts a job by hand, waits for the record to land, and says which it is.
fn started(sim: &mut Simulation, instance: &mut Instance, id: u64, work: &str) -> JobId {
    asking(sim, instance, id, work);
    // Answered once the record has landed, which is a moment later.
    let until = sim.now() + 5;
    sim.run_until(instance, until);
    which(sim, id, work)
}

fn progress_of(instance: &Instance, id: &JobId) -> Progress {
    instance.state().job(id).expect("the job").progress.clone()
}

/// How many times the runtime was asked something of a kind.
fn asked(sim: &Simulation, wanted: impl Fn(&Command) -> bool) -> usize {
    sim.commands()
        .iter()
        .filter(|command| wanted(command))
        .count()
}

/// Two jobs starting together want the same image: it is asked about twice,
/// built once, and both containers are made from it.
///
/// The second build would otherwise move the name onto its own copy and
/// leave the first's unreferenced, which the runtime deletes from under a
/// container just made from it — so one turn waits on the other's build
/// and finds what it left.
#[test]
fn an_image_is_built_once_for_every_turn_that_wants_it() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));

    asking(&mut sim, &mut instance, 1, "one thing");
    asking(&mut sim, &mut instance, 2, "another");
    sim.run_until(&mut instance, 5_000);
    let one = which(&sim, 1, "one thing");
    let two = which(&sim, 2, "another");

    assert_eq!(
        asked(&sim, |command| matches!(command, Command::Present { .. })),
        2,
        "each asked whether the image was there: {:?}",
        sim.commands()
    );
    assert_eq!(
        asked(&sim, |command| matches!(command, Command::Build { .. })),
        1,
        "and it was built once: {:?}",
        sim.commands()
    );
    for job in [one, two] {
        assert!(sim.exists(&stageman_job::container(&job)));
        assert_eq!(
            progress_of(&instance, &job),
            Progress::Idle(Waiting::Silent)
        );
    }
    assert_eq!(sim.talks().len(), 2);
}

/// A build that fails fails every turn waiting on it, with the build's last
/// words, and no container is made.
#[test]
fn a_build_that_fails_fails_every_turn_waiting_on_it() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.next_build_fails("step 3/7: RUN npm install\nfailed to fetch the adapter\n");

    asking(&mut sim, &mut instance, 1, "one thing");
    asking(&mut sim, &mut instance, 2, "another");
    sim.run_until(&mut instance, 5_000);
    let one = which(&sim, 1, "one thing");
    let two = which(&sim, 2, "another");

    for job in [one.clone(), two] {
        let Progress::Idle(Waiting::Failed(why)) = progress_of(&instance, &job) else {
            panic!("a job whose image could not be built has failed");
        };
        assert!(why.contains("failed to fetch the adapter"), "{why}");
        assert!(why.contains("could not be built"), "{why}");
        assert!(!sim.exists(&stageman_job::container(&job)));
    }
    assert_eq!(
        asked(&sim, |command| matches!(command, Command::Create { .. })),
        0
    );
    assert!(sim.talks().is_empty(), "no agent was spoken to");
    assert_eq!(
        progress_of(&instance, &one),
        sim.disk()
            .expect("landed")
            .job(&one)
            .expect("the job")
            .progress
            .clone(),
        "the failure reached the disk"
    );
}

/// A repository that cannot be checked out fails the job before any model
/// turn is spent, naming the repository and what the tool said.
#[test]
fn a_checkout_that_fails_fails_the_job_before_its_agent_speaks() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.next_checkout_fails("fatal: repository not found\n");

    let job = started(&mut sim, &mut instance, 1, "one thing");
    sim.run_until(&mut instance, 5_000);

    let Progress::Idle(Waiting::Failed(why)) = progress_of(&instance, &job) else {
        panic!("a job whose repository could not be checked out has failed");
    };
    assert!(why.contains("repository not found"), "{why}");
    assert!(why.contains("example.invalid"), "{why}");
    assert!(sim.talks().is_empty(), "no agent was spoken to");
    assert!(
        sim.exists(&stageman_job::container(&job)),
        "the container was made, and is kept for whoever looks"
    );
}

/// A stop that lands before the agent speaks ends the turn at its next
/// step, paused, and the agent is never run.
#[test]
fn a_stop_before_the_agent_speaks_ends_the_turn_at_its_next_step() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));

    // Started, and stopped while the runtime is still being given the
    // commands that come before the agent: the turn is held from the moment
    // it is asked for, so there is something to mark.
    let job = started(&mut sim, &mut instance, 1, "one thing");
    assert!(sim.talks().is_empty(), "not yet spoken to");
    for effect in instance.step(
        sim.now(),
        request(
            2,
            Request::Stop {
                project: project().to_string(),
                job: job.to_string(),
            },
        ),
    ) {
        sim.perform(effect);
    }
    sim.run_until(&mut instance, 5_000);

    assert_eq!(
        progress_of(&instance, &job),
        Progress::Idle(Waiting::Paused)
    );
    assert!(sim.talks().is_empty(), "the agent was never run");
    assert!(
        sim.exists(&stageman_job::container(&job)),
        "what was made before the stop landed is kept, like everything a stop leaves"
    );
}

/// The commands a turn runs, in order, for a job that begins: whether the
/// image is there, a build, the container made, started, the repository
/// checked out, and the agent run inside it; for one that resumes, the
/// container started and the agent run.
#[test]
fn a_turn_is_the_commands_it_runs_in_order() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[(job(1), Progress::Working, 1)]));
    let (name, held) = Simulation::ours(&stageman_job::container(&job(1)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    let woke = sim.commands().len();
    let begun = started(&mut sim, &mut instance, 1, "one thing");
    sim.run_until(&mut instance, 5_000);

    let kinds: Vec<&str> = sim
        .commands()
        .iter()
        .skip(woke)
        .filter_map(|command| match command {
            Command::Present { .. } => Some("present"),
            Command::Build { .. } => Some("build"),
            Command::Create { .. } => Some("create"),
            Command::Start { .. } => Some("start"),
            Command::Checkout { .. } => Some("checkout"),
            Command::Exec { .. } => Some("exec"),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        ["present", "build", "create", "start", "checkout"],
        "{:?}",
        sim.commands()
    );
    let resumed = sim
        .commands()
        .iter()
        .take(woke)
        .any(|command| matches!(command, Command::Start { name: started } if *started == name));
    assert!(resumed, "waking started the working job's container");
    assert_eq!(sim.talks().len(), 2);
    assert!(sim.talks_in(&name)[0].resumed());
    assert!(sim.talks_in(&stageman_job::container(&begun))[0].began());

    // Both turns have ended, and nothing of either is held: not the turn,
    // not the process it talked over, not the credential it presented.
    let held = &instance.snapshot()["held"];
    assert_eq!(held["turns"], serde_json::json!([]));
    assert_eq!(held["talking"], serde_json::json!([]));
    assert_eq!(held["warrants"], serde_json::json!({}));
}
