//! Speaking on a project's channel, run against the simulated platform: a
//! job's thread opened by a post at the root, a platform that refuses, and
//! a record that never lands.

use stageman_core::{JobId, Progress, Timestamp, Uuid, Waiting};
use stageman_instance::{Instance, Request, Response};

use crate::simulation::{Simulation, project, request, seed, thread, watching_a_channel};

/// Asks for a job by hand and performs what asking caused.
fn asking(sim: &mut Simulation, instance: &mut Instance, id: u64, work: &str) {
    for effect in instance.step(request(
        id,
        Request::Start {
            project: project().to_string(),
            kit: "Claude".to_owned(),
            work: work.to_owned(),
            at: Timestamp::UNIX_EPOCH,
        },
    )) {
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
    JobId::from_uuid(Uuid::parse_str(&listed.id).expect("an identifier"))
}

fn progress_of(instance: &Instance, id: JobId) -> Progress {
    instance.state().job(id).expect("the job").progress.clone()
}

/// A job on a project with a channel begins by posting its announcement at
/// the root, and starts only once the platform has named the thread — which
/// is recorded on the job, so that a reply can find it.
#[test]
fn a_jobs_thread_is_opened_by_a_post_at_the_root_before_it_starts() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));

    asking(&mut sim, &mut instance, 1, "fix the build");
    sim.run_until(&mut instance, 5_000);
    let job = which(&sim, 1, "fix the build");

    let opened = thread(101);
    let (posted_in, announcement) = sim.posts().first().expect("the announcement");
    assert_eq!(
        posted_in, &opened,
        "posted at the root, so it is a thread of its own"
    );
    assert!(
        announcement.contains(&format!("Job {job}")),
        "the announcement names the job it is for: {announcement}"
    );
    assert_eq!(
        instance.state().job(job).expect("the job").thread,
        Some(opened),
        "the thread the platform named is where the job speaks"
    );

    let shape = sim.shape();
    let asked = shape
        .iter()
        .position(|line| line.starts_with("-> Request"))
        .expect("the post was asked for");
    let answered = shape
        .iter()
        .position(|line| line.starts_with("<- Responded"))
        .expect("and answered");
    let started = sim.first_turn().expect("the job was started");
    assert!(
        asked < answered && answered < started,
        "the turn waits for the thread: {shape:?}"
    );
    assert_eq!(progress_of(&instance, job), Progress::Idle(Waiting::Silent));
}

/// A platform that refuses the announcement fails the job before any
/// container exists, saying what the platform said — and a refusal arrives
/// with a successful status, which is the trap the reader exists for.
#[test]
fn a_channel_that_refuses_the_announcement_fails_the_job_before_any_container() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.next_post_fails("not_in_channel");

    asking(&mut sim, &mut instance, 1, "fix the build");
    sim.run_until(&mut instance, 5_000);
    let job = which(&sim, 1, "fix the build");

    let Progress::Idle(Waiting::Failed(why)) = progress_of(&instance, job) else {
        panic!("a job whose thread could not be opened has failed");
    };
    assert!(why.contains("its channel could not be reached"), "{why}");
    assert!(why.contains("not_in_channel"), "{why}");
    assert!(sim.talks().is_empty(), "no agent was spoken to");
    assert!(
        !sim.exists(&stageman_job::container(job)),
        "no container was made for a job that cannot speak"
    );
    assert_eq!(
        progress_of(&instance, job),
        sim.disk()
            .expect("landed")
            .job(job)
            .expect("the job")
            .progress
            .clone(),
        "the failure reached the disk"
    );
}

/// A job whose record never lands is not left working with nowhere to
/// speak: the post that waited on the write is never made, and the job is
/// failed for that reason, the same way a turn never started is.
#[test]
fn a_job_whose_record_never_landed_is_failed_rather_than_left_working() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.next_write_fails("the disk is full");

    asking(&mut sim, &mut instance, 1, "fix the build");
    sim.run_until(&mut instance, 5_000);

    assert!(
        sim.posts().is_empty(),
        "nothing was posted for a record nobody has"
    );
    let failed: Vec<Progress> = instance
        .state()
        .projects
        .values()
        .flat_map(|watched| watched.jobs.values())
        .map(|recorded| recorded.progress.clone())
        .collect();
    let [Progress::Idle(Waiting::Failed(why))] = failed.as_slice() else {
        panic!("one job, failed: {failed:?}");
    };
    assert!(why.contains("could not be written"), "{why}");
    assert!(sim.talks().is_empty(), "no agent was spoken to");
    assert_eq!(
        instance.snapshot()["held"]["sent"],
        serde_json::json!([]),
        "nothing is left waiting on an answer"
    );
}
