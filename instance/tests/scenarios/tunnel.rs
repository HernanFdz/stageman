//! Somebody visiting a job's name under the domain: the instance says where
//! to forward, asks the runtime once per look, and forgets a port that has
//! moved.

use stageman_agent::Command;
use stageman_core::{Outcome, Progress, Waiting};
use stageman_instance::{AppEvent, Effect, Instance};

use crate::simulation::{
    Sent, Simulation, job, said_in, seed, tunnel_asked, watching, watching_a_channel,
};

/// Sends a tunnel request and performs whatever answers it in the step.
fn visit(sim: &mut Simulation, instance: &mut Instance, id: u64, which: u128) {
    for effect in instance.step(tunnel_asked(id, job(which))) {
        sim.perform(effect);
    }
    let until = sim.now();
    sim.run_until(instance, until);
}

/// How many times the runtime was asked where a tunnel is.
///
/// Read back through the agent crate's own inverse rather than matched as a
/// string: every runtime command is one generic effect now, so what tells
/// them apart is the argument list, and that is rendered in one place.
fn looked(sim: &Simulation) -> usize {
    sim.commands()
        .iter()
        .filter(|command| matches!(command, Command::Port { .. }))
        .count()
}

/// One look at the runtime answers every request waiting on it, and the
/// next request is answered from what was found.
#[test]
fn a_jobs_tunnel_is_looked_up_once_and_remembered() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[(job(1), Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(job(1)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    // Waking resumes the job, which starts its container on a port.
    let port = sim
        .port_of(&stageman_job::container(job(1)))
        .expect("resumed, so running on a port");

    // Two requests in one step, before the runtime has answered.
    let mut effects = instance.step(tunnel_asked(1, job(1)));
    effects.extend(instance.step(tunnel_asked(2, job(1))));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| {
                matches!(effect, Effect::Run { arguments, .. }
                    if matches!(Command::parse(arguments), Some(Command::Port { .. })))
            })
            .count(),
        1,
        "asked once for both"
    );
    for effect in effects {
        sim.perform(effect);
    }
    let until = sim.now();
    sim.run_until(&mut instance, until);
    assert_eq!(sim.route(1), Some(Sent::To(port)));
    assert_eq!(sim.route(2), Some(Sent::To(port)));

    visit(&mut sim, &mut instance, 3, 1);
    assert_eq!(sim.route(3), Some(Sent::To(port)));
    assert_eq!(looked(&sim), 1, "the third look asks nobody");
}

/// A name that identifies no job of this instance's, or a job that is over,
/// is answered without asking the runtime.
#[test]
fn a_stranger_and_a_retired_job_are_answered_without_asking() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[
        (job(1), Progress::Retired(Outcome::Done)),
        (job(2), Progress::Idle(Waiting::Silent)),
    ]));
    let (name, held) = Simulation::ours(&stageman_job::container(job(2)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));

    visit(&mut sim, &mut instance, 1, 99);
    assert_eq!(sim.route(1), Some(Sent::Nowhere), "no such job");
    visit(&mut sim, &mut instance, 2, 1);
    assert_eq!(
        sim.route(2),
        Some(Sent::Nowhere),
        "over, so nothing to show"
    );
    assert_eq!(looked(&sim), 0);

    // An idle job whose container is stopped is asked about, and found to
    // be reachable nowhere.
    visit(&mut sim, &mut instance, 3, 2);
    assert_eq!(sim.route(3), Some(Sent::Nowhere));
    assert_eq!(looked(&sim), 1);
}

/// The port is forgotten at every moment it can have moved — a halt, a
/// restart, a failed connection — and the next look asks the runtime again.
#[test]
fn a_port_that_can_have_moved_is_looked_up_again() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[(job(1), Progress::Working, 1)]));
    let (name, held) = Simulation::ours(&stageman_job::container(job(1)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    let first = sim
        .port_of(&stageman_job::container(job(1)))
        .expect("resumed, so running on a port");

    visit(&mut sim, &mut instance, 1, 1);
    assert_eq!(sim.route(1), Some(Sent::To(first)));
    assert_eq!(looked(&sim), 1);

    // The turn ends and nothing answers on the tunnel, so the container is
    // halted; the port it was on reaches nothing now.
    sim.run_until(&mut instance, 2_000);
    assert!(!sim.is_running(&stageman_job::container(job(1))));
    visit(&mut sim, &mut instance, 2, 1);
    assert_eq!(
        sim.route(2),
        Some(Sent::Nowhere),
        "stopped, so nothing to reach"
    );
    assert_eq!(looked(&sim), 2, "halting forgot the port");

    // A reply resumes the job, which restarts the container on a fresh port.
    for effect in instance.step(said_in(1, "go on")) {
        sim.perform(effect);
    }
    let until = sim.now() + 5;
    sim.run_until(&mut instance, until);
    let second = sim
        .port_of(&stageman_job::container(job(1)))
        .expect("resumed again");
    assert_ne!(first, second, "the runtime publishes afresh on every start");
    visit(&mut sim, &mut instance, 3, 1);
    assert_eq!(sim.route(3), Some(Sent::To(second)));
    assert_eq!(looked(&sim), 3, "resuming forgot the port");

    // A connection that did not go through forgets it too.
    for effect in instance.step(
        AppEvent::TunnelFailed {
            job: job(1),
            why: "connection refused".to_owned(),
        }
        .into(),
    ) {
        sim.perform(effect);
    }
    visit(&mut sim, &mut instance, 4, 1);
    assert_eq!(sim.route(4), Some(Sent::To(second)));
    assert_eq!(looked(&sim), 4, "a failure forgot the port");
}
