//! The purse travels with every turn: a credential replaced between two
//! turns, a container made before the purse travelled this way, and a job
//! whose purse is not held — see
//! `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`.
//!
//! What a container is made with and what its agent is run with on an
//! ordinary turn are asserted where each is decided, in the foreman's and
//! the credentials' scenarios.

use std::collections::BTreeMap;

use crate::dashboard::ask;
use crate::simulation::{Simulation, charging, job, project, seed, watching_a_channel};
use stageman_agent::Command;
use stageman_core::{Agent, Kit, Progress, Purse, PurseName, Secret, Waiting};
use stageman_instance::{Request, Response};

/// Claude as it comes, charging the subscription.
fn subscribed() -> Kit {
    Kit::defaults(Agent::Claude, PurseName::AnthropicSubscription)
        .expect("Claude charges a subscription")
}

/// An environment holding one variable.
fn one(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

/// Holds a purse from the dashboard, as an operator replacing one does.
fn hold(
    world: &mut Simulation,
    instance: &mut stageman_instance::Instance,
    purse: &str,
    credential: &str,
) {
    let Response::Agents(_) = ask(
        world,
        instance,
        1,
        Request::HoldPurse {
            purse: purse.to_owned(),
            credential: credential.to_owned(),
        },
    ) else {
        panic!("the agents screen");
    };
}

/// A credential replaced between two messages reaches the foreman at the
/// second, in the container it already has and the session it remembers:
/// the purse is selected when the agent is run, and the container was made
/// with none.
#[test]
fn a_replaced_purse_reaches_a_foreman_at_its_next_message() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);
    hold(
        &mut world,
        &mut instance,
        "anthropic-key",
        "sk-ant-api03-the-new-one",
    );
    world.says_at_root(6_000, 2, "and the lexer");
    world.run_until(&mut instance, 12_000);

    let container = stageman_foreman::container(project());
    let talks = world.talks_in(&container);
    assert_eq!(talks.len(), 2, "{talks:?}");
    assert_eq!(talks[0].given, one("ANTHROPIC_API_KEY", "agent-token"));
    assert_eq!(
        talks[1].given,
        one("ANTHROPIC_API_KEY", "sk-ant-api03-the-new-one"),
        "the credential held when the agent was run"
    );
    assert!(talks[1].resumed(), "in the session it had: {:?}", talks[1]);
    assert_eq!(
        world
            .commands()
            .iter()
            .filter(|command| matches!(command, Command::Create { .. }))
            .count(),
        1,
        "and in the container it had"
    );
    assert!(
        world
            .environment_of(&container)
            .expect("the container was made here")
            .is_empty(),
        "which was made with no purse and holds none"
    );
}

/// A container made before the purse travelled with the turn holds the one
/// it was created with, and everything run in it would see that. A foreman
/// since moved to the other purse is run with the one its kit charges and
/// cleared of the one its container was made with, so that its agent finds
/// one purse and the right one pays.
#[test]
fn a_container_made_with_a_purse_does_not_leak_it_into_a_turn_charged_to_another() {
    let mut world = Simulation::new();
    let mut state = watching_a_channel(&[]);
    state.purses.hold(Purse::AnthropicSubscription(Secret::new(
        "sk-ant-oat01-the-subscription".to_owned(),
    )));
    state
        .projects
        .get_mut(&project())
        .expect("the project")
        .foreman_kit = subscribed();
    world.holding(&state);
    let (name, mut held) = Simulation::ours(&stageman_foreman::container(project()));
    held.environment = one("ANTHROPIC_API_KEY", "the-key-it-was-made-with");
    world.container(&name, held);
    let mut instance = world.wake(seed(1));

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 5_000);

    let talks = world.talks_in(&name);
    assert_eq!(talks.len(), 1, "{talks:?}");
    assert_eq!(
        talks[0].given,
        one("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-the-subscription"),
        "one purse, and the one its kit charges"
    );
    assert_eq!(
        world.environment_of(&name),
        Some(&one("ANTHROPIC_API_KEY", "the-key-it-was-made-with")),
        "the container still holds what it was made with, which no agent run in it sees"
    );
}

/// A job whose purse is not held fails at its turn, saying which, with no
/// agent run and its container kept; holding the purse and saying something
/// to the job is the repair. Nothing the dashboard does leads here, since a
/// purse is not forgotten while an unfinished job charges it: this is what a
/// file edited by hand meets.
#[test]
fn a_job_whose_purse_is_not_held_fails_at_its_turn_and_is_repaired_by_holding_it() {
    let mut world = Simulation::new();
    let working = job(1);
    let mut state = watching_a_channel(&[(working.clone(), Progress::Working, 1)]);
    let recorded = state.job(&working).expect("the job").clone();
    *state.job_mut(&working).expect("the job") = charging(&recorded, &subscribed());
    world.holding(&state);
    let container = stageman_job::container(&working);
    let (name, held) = Simulation::ours(&container);
    world.container(&name, held);
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 5_000);

    assert!(
        world.talks_in(&container).is_empty(),
        "no agent is run with nothing to pay with"
    );
    assert_eq!(
        instance
            .state()
            .job(&working)
            .map(|job| job.progress.clone()),
        Some(Progress::Idle(Waiting::Failed(
            "its kit charges the Anthropic subscription, which is not held".to_owned()
        )))
    );
    assert!(
        world.environment_of(&container).is_some(),
        "and its container is kept"
    );

    hold(
        &mut world,
        &mut instance,
        "anthropic-subscription",
        "sk-ant-oat01-held-now",
    );
    world.says_in_room(6_000, 1, "carry on");
    world.run_until(&mut instance, 12_000);

    let talks = world.talks_in(&container);
    assert_eq!(talks.len(), 1, "{talks:?}");
    assert_eq!(
        talks[0].given,
        one("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-held-now"),
        "run with the purse now held"
    );
}
