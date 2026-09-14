//! Somebody visiting a job's name under the domain: the instance says where
//! to forward, asks the runtime once per look, and forgets a port that has
//! moved.

use stageman_agent::Command;
use stageman_core::{Outcome, Progress, Waiting};
use stageman_instance::Instance;
use stageman_vocabulary::RequestId;

use crate::simulation::{
    Sent, Simulation, job, said_in, seed, tunnel_host, watching, watching_a_channel,
};

/// Visits a job's name and runs until it has been answered.
fn visit(sim: &mut Simulation, instance: &mut Instance, which: u128) -> RequestId {
    let now = sim.now();
    let id = sim.visits(now, &tunnel_host(job(which)));
    sim.run_until(instance, now);
    id
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

    // Two visits before the runtime has answered either.
    let now = sim.now();
    let first = sim.visits(now, &tunnel_host(job(1)));
    let second = sim.visits(now, &tunnel_host(job(1)));
    sim.run_until(&mut instance, now);
    assert_eq!(looked(&sim), 1, "asked once for both");
    assert_eq!(sim.route(first), Some(Sent::To(port)));
    assert_eq!(sim.route(second), Some(Sent::To(port)));

    let third = visit(&mut sim, &mut instance, 1);
    assert_eq!(sim.route(third), Some(Sent::To(port)));
    assert_eq!(looked(&sim), 1, "the third look asks nobody");
}

/// Everything that is not a job's name is forwarded to the pages.
///
/// The dashboard is a presentation server this instance proxies to, on a
/// loopback port the entry point took and told it about — so the address a
/// person types carries both, and neither can collide with the other.
#[test]
fn what_is_not_a_jobs_name_is_forwarded_to_the_pages() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[(job(1), Progress::Working)]));
    let mut instance = sim.wake(seed(1));

    let now = sim.now();
    let apex = sim.visits(now, "localhost");
    let page = sim.visits(now, "localhost:8080");
    sim.run_until(&mut instance, now);

    assert_eq!(sim.route(apex), Some(Sent::To(9000)));
    assert_eq!(
        sim.route(page),
        Some(Sent::To(9000)),
        "a port is not a name"
    );
    assert_eq!(looked(&sim), 0, "no runtime was asked about a page");
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

    let stranger = visit(&mut sim, &mut instance, 99);
    assert_eq!(sim.route(stranger), Some(Sent::Nowhere), "no such job");
    let over = visit(&mut sim, &mut instance, 1);
    assert_eq!(
        sim.route(over),
        Some(Sent::Nowhere),
        "over, so nothing to show"
    );
    assert_eq!(looked(&sim), 0);

    // An idle job whose container is stopped is asked about, and found to
    // be reachable nowhere.
    let stopped = visit(&mut sim, &mut instance, 2);
    assert_eq!(sim.route(stopped), Some(Sent::Nowhere));
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

    let once = visit(&mut sim, &mut instance, 1);
    assert_eq!(sim.route(once), Some(Sent::To(first)));
    assert_eq!(looked(&sim), 1);

    // The turn ends and nothing answers on the tunnel, so the container is
    // halted; the port it was on reaches nothing now.
    sim.run_until(&mut instance, 2_000);
    assert!(!sim.is_running(&stageman_job::container(job(1))));
    let halted = visit(&mut sim, &mut instance, 1);
    assert_eq!(
        sim.route(halted),
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
    let resumed = visit(&mut sim, &mut instance, 1);
    assert_eq!(sim.route(resumed), Some(Sent::To(second)));
    assert_eq!(looked(&sim), 3, "resuming forgot the port");
}
