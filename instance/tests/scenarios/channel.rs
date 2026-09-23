//! A job's room, run against the simulated platform: made before the job
//! starts, described and opened, a platform that refuses, and a record that
//! never lands.

use stageman_core::{JobId, Progress, Uuid, Waiting};
use stageman_instance::{Instance, Request, Response};

use crate::simulation::{Simulation, in_room, project, request, room, seed, watching_a_channel};

/// Asks for a job by hand and performs what asking caused.
fn asking(sim: &mut Simulation, instance: &mut Instance, id: u64, work: &str) {
    for effect in instance.step(
        sim.now(),
        request(
            id,
            Request::Start {
                project: project().to_string(),
                kit: "Claude".to_owned(),
                work: work.to_owned(),
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
    JobId::from_uuid(Uuid::parse_str(&listed.id).expect("an identifier"))
}

fn progress_of(instance: &Instance, id: JobId) -> Progress {
    instance.state().job(id).expect("the job").progress.clone()
}

/// A job begins by having a room made for it, named after the project, its
/// title and its identifier, and starts only once the platform has named
/// the room — which is recorded on the job, so that a reply can find it.
/// The room is described, and its opening says what the room is for.
#[test]
fn a_jobs_room_is_made_before_it_starts() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));

    asking(
        &mut sim,
        &mut instance,
        1,
        "fix the build before the release",
    );
    sim.run_until(&mut instance, 5_000);
    let job = which(&sim, 1, "fix the build before the release");

    let made = room(1);
    let (id, name) = sim.rooms().first().expect("the room was made");
    assert_eq!(id, &made.id);
    assert!(
        name.starts_with("example--fix-the-build-before-the-release--"),
        "named after the project and the title: {name}"
    );
    let identifier: String = job.as_uuid().simple().to_string().chars().take(8).collect();
    assert!(
        name.ends_with(&identifier),
        "and the job's identifier: {name}"
    );
    assert_eq!(
        instance.state().job(job).expect("the job").room,
        Some(made),
        "the room the platform named is where the job speaks"
    );
    assert!(
        sim.described()
            .iter()
            .any(|(room, text)| room == &id.clone() && text == "started by hand from the dashboard"),
        "its purpose is the reason: {:?}",
        sim.described()
    );
    assert!(
        sim.described()
            .iter()
            .any(|(room, text)| room == &id.clone() && text.contains("Showing at")),
        "its topic is where to look: {:?}",
        sim.described()
    );
    assert!(sim.invited().is_empty(), "started by hand, so nobody asked");
    let (opened_at, opening) = sim.posts().first().expect("the opening");
    assert_eq!(opened_at, &in_room(1));
    assert!(opening.starts_with("**A job on"), "{opening}");

    let shape = sim.shape();
    let asked = shape
        .iter()
        .position(|line| line.starts_with("-> Request"))
        .expect("the room was asked for");
    let answered = shape
        .iter()
        .position(|line| line.starts_with("<- Responded"))
        .expect("and answered");
    let started = sim.first_turn().expect("the job was started");
    assert!(
        asked < answered && answered < started,
        "the turn waits for the room: {shape:?}"
    );
    assert_eq!(progress_of(&instance, job), Progress::Idle(Waiting::Silent));
}

/// A platform that refuses to make the room fails the job before any
/// container exists, saying what the platform said — and a refusal arrives
/// with a successful status, which is the trap the reader exists for.
#[test]
fn a_platform_that_refuses_the_room_fails_the_job_before_any_container() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.next_room_fails("name_taken");

    asking(&mut sim, &mut instance, 1, "fix the build");
    sim.run_until(&mut instance, 5_000);
    let job = which(&sim, 1, "fix the build");

    let Progress::Idle(Waiting::Failed(why)) = progress_of(&instance, job) else {
        panic!("a job whose room could not be made has failed");
    };
    assert!(why.contains("its room could not be made"), "{why}");
    assert!(why.contains("name_taken"), "{why}");
    assert!(sim.talks().is_empty(), "no agent was spoken to");
    assert!(sim.posts().is_empty(), "nothing was said anywhere");
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
/// speak: the room that waited on the write is never asked for, and the
/// job is failed for that reason, the same way a turn never started is.
#[test]
fn a_job_whose_record_never_landed_is_failed_rather_than_left_working() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.next_write_fails("the disk is full");

    asking(&mut sim, &mut instance, 1, "fix the build");
    sim.run_until(&mut instance, 5_000);

    assert!(
        sim.rooms().is_empty(),
        "no room was made for a record nobody has"
    );
    assert!(sim.posts().is_empty(), "and nothing was said");
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
