//! A purse is checked against its provider before it is kept, and travels
//! with every turn once it is — see
//! `docs/decisions/0086-a-kit-charges-a-purse-at-a-provider.md`.
//!
//! The check: the request held while the provider is asked, kept once it
//! has accepted, refused in the provider's own sentence, unchecked where
//! the provider could not be asked, and a purse already held left as it was
//! by a replacement that is not kept. A paste refused on its own asks no
//! provider, which the dashboard's scenarios assert beside the refusals.
//!
//! The travelling: a credential replaced between two turns, a container
//! made before the purse travelled this way, and a job whose purse is not
//! held. What a container is made with and what its agent is run with on an
//! ordinary turn are asserted where each is decided, in the foreman's and
//! the credentials' scenarios.

use std::collections::BTreeMap;

use crate::dashboard::{ask, count, first, nth};
use crate::simulation::{Simulation, charging, job, project, seed, watching_a_channel};
use stageman_agent::Command;
use stageman_core::{Agent, Kit, Progress, Purse, PurseName, Secret, Waiting};
use stageman_instance::{Instance, Request, Response};
use stageman_provider::Call as ProviderCall;
use stageman_wire::Refusal;

/// What the provider answered a made-up key in the key's header, as
/// measured.
const KEY_REFUSED: &str = r#"{"type":"error","error":{"type":"authentication_error","message":"API key is invalid."},"request_id":null}"#;

/// What it answered a made-up subscription token sent as a bearer.
const TOKEN_REFUSED: &str = r#"{"type":"error","error":{"type":"authentication_error","message":"OAuth access token is invalid."},"request_id":null}"#;

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
fn hold(world: &mut Simulation, instance: &mut Instance, purse: &str, credential: &str) {
    let Response::Agents(_) = pasting(world, instance, 1, purse, credential) else {
        panic!("the agents screen");
    };
}

/// What pasting a credential into a purse's box and pressing Save is
/// answered.
fn pasting(
    world: &mut Simulation,
    instance: &mut Instance,
    id: u64,
    purse: &str,
    credential: &str,
) -> Response {
    ask(
        world,
        instance,
        id,
        Request::HoldPurse {
            purse: purse.to_owned(),
            credential: credential.to_owned(),
        },
    )
}

/// What the purse of that name holds on the disk, if one is held there.
fn kept(world: &Simulation, purse: PurseName) -> Option<String> {
    world.disk().and_then(|landed| {
        landed
            .purses
            .get(purse)
            .map(|held| held.credential().expose().to_owned())
    })
}

/// A purse is asked about before it is kept, and answered after: the
/// provider's listing goes out with the credential in the header its kind
/// travels in, trimmed as it will be kept; the write follows the provider's
/// yes; and the answer follows the write.
#[test]
fn a_purse_is_asked_of_its_provider_before_it_is_kept_and_answered_after() {
    let mut world = Simulation::new();
    let mut instance = world.wake(seed(1));
    let requests_before = count(&world, "-> Request");
    let writes_before = count(&world, "-> Write");

    let Response::Agents(shown) = pasting(
        &mut world,
        &mut instance,
        1,
        "anthropic-key",
        "  sk-ant-api03-a-new-key \n",
    ) else {
        panic!("the agents screen");
    };
    assert!(
        shown
            .providers
            .iter()
            .flat_map(|provider| provider.purses.iter())
            .any(|purse| purse.id == "anthropic-key" && purse.held),
        "{shown:?}"
    );

    let asked = nth(&world, "-> Request", requests_before);
    let line = &world.trace()[asked];
    assert!(
        line.contains("https://api.anthropic.com/v1/models?limit=1"),
        "{line}"
    );
    assert!(
        line.contains(r#""x-api-key":"sk-ant-api03-a-new-key""#),
        "the key, in the key's header, trimmed: {line}"
    );
    let written = nth(&world, "-> Write", writes_before);
    assert!(asked < written, "asked before kept: {asked} < {written}");
    assert!(
        written < first(&world, "-> Respond"),
        "kept before answered"
    );
    assert_eq!(
        kept(&world, PurseName::AnthropicKey).as_deref(),
        Some("sk-ant-api03-a-new-key")
    );

    // The other kind is asked about as a bearer.
    hold(
        &mut world,
        &mut instance,
        "anthropic-subscription",
        "sk-ant-oat01-a-new-token",
    );
    let asked = nth(&world, "-> Request", requests_before + 1);
    let line = &world.trace()[asked];
    assert!(
        line.contains(r#""authorization":"Bearer sk-ant-oat01-a-new-token""#),
        "the token, as a bearer: {line}"
    );
    assert_eq!(
        world
            .provider_calls()
            .iter()
            .map(|(_, call)| *call)
            .collect::<Vec<_>>(),
        vec![
            ProviderCall::Check {
                purse: PurseName::AnthropicKey
            },
            ProviderCall::Check {
                purse: PurseName::AnthropicSubscription
            },
        ],
        "one question a purse, and nothing else asked of the provider"
    );
}

/// A purse its provider does not accept is refused in the provider's own
/// sentence, whichever kind it is, and nothing is kept or written.
#[test]
fn a_purse_its_provider_refuses_is_not_kept_and_is_refused_in_the_providers_words() {
    let mut world = Simulation::new();
    let mut instance = world.wake(seed(1));
    let written = count(&world, "-> Write");

    world.next_provider_answers(401, KEY_REFUSED);
    assert_eq!(
        pasting(
            &mut world,
            &mut instance,
            1,
            "anthropic-key",
            "sk-ant-api03-not-a-real-key",
        ),
        Response::Refused(Refusal::PurseRefused {
            purse: "Anthropic key".to_owned(),
            why: "Anthropic does not accept it: API key is invalid".to_owned(),
        })
    );
    world.next_provider_answers(401, TOKEN_REFUSED);
    assert_eq!(
        pasting(
            &mut world,
            &mut instance,
            2,
            "anthropic-subscription",
            "sk-ant-oat01-not-a-real-token",
        ),
        Response::Refused(Refusal::PurseRefused {
            purse: "Anthropic subscription".to_owned(),
            why: "Anthropic does not accept it: OAuth access token is invalid".to_owned(),
        })
    );

    assert_eq!(world.provider_calls().len(), 2, "each was asked about");
    assert!(instance.state().purses.is_empty(), "nothing was kept");
    assert_eq!(count(&world, "-> Write"), written, "nothing was written");
}

/// A provider that cannot be asked keeps nothing either, and says the purse
/// could not be checked rather than that it was wrong: no answer at all, and
/// an answer that is a verdict on nothing. Pasting it again once the
/// provider answers is the repair.
#[test]
fn a_provider_that_cannot_be_asked_keeps_nothing_and_says_the_purse_was_not_checked() {
    let mut world = Simulation::new();
    let mut instance = world.wake(seed(1));
    let written = count(&world, "-> Write");

    world.next_provider_fails("dns error");
    assert_eq!(
        pasting(
            &mut world,
            &mut instance,
            1,
            "anthropic-key",
            "sk-ant-api03-a-good-key",
        ),
        Response::Refused(Refusal::PurseUnchecked {
            purse: "Anthropic key".to_owned(),
            why: "Anthropic could not be reached: dns error".to_owned(),
        })
    );
    world.next_provider_answers(
        529,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    );
    assert_eq!(
        pasting(
            &mut world,
            &mut instance,
            2,
            "anthropic-key",
            "sk-ant-api03-a-good-key",
        ),
        Response::Refused(Refusal::PurseUnchecked {
            purse: "Anthropic key".to_owned(),
            why: "Anthropic could not be reached: it answered 529".to_owned(),
        })
    );
    assert!(instance.state().purses.is_empty(), "nothing was kept");
    assert_eq!(count(&world, "-> Write"), written, "nothing was written");

    let Response::Agents(_) = pasting(
        &mut world,
        &mut instance,
        3,
        "anthropic-key",
        "sk-ant-api03-a-good-key",
    ) else {
        panic!("the agents screen");
    };
    assert_eq!(
        kept(&world, PurseName::AnthropicKey).as_deref(),
        Some("sk-ant-api03-a-good-key"),
        "kept once the provider answered for it"
    );
}

/// A replacement that is not kept — refused by the provider, or never
/// checked — leaves the purse that was held exactly as it was, in memory and
/// on the disk: nothing is written until the provider has accepted.
#[test]
fn a_replacement_that_is_not_kept_leaves_the_purse_that_was_held() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    let written = count(&world, "-> Write");
    let held = |instance: &Instance| {
        instance
            .state()
            .purses
            .get(PurseName::AnthropicKey)
            .map(|purse| purse.credential().expose().to_owned())
    };
    assert_eq!(held(&instance).as_deref(), Some("agent-token"));

    world.next_provider_answers(401, KEY_REFUSED);
    assert_eq!(
        pasting(
            &mut world,
            &mut instance,
            1,
            "anthropic-key",
            "sk-ant-api03-a-wrong-one",
        ),
        Response::Refused(Refusal::PurseRefused {
            purse: "Anthropic key".to_owned(),
            why: "Anthropic does not accept it: API key is invalid".to_owned(),
        })
    );
    world.next_provider_fails("operation timed out");
    assert_eq!(
        pasting(
            &mut world,
            &mut instance,
            2,
            "anthropic-key",
            "sk-ant-api03-an-unchecked-one",
        ),
        Response::Refused(Refusal::PurseUnchecked {
            purse: "Anthropic key".to_owned(),
            why: "Anthropic could not be reached: operation timed out".to_owned(),
        })
    );

    assert_eq!(held(&instance).as_deref(), Some("agent-token"));
    assert_eq!(
        kept(&world, PurseName::AnthropicKey).as_deref(),
        Some("agent-token")
    );
    assert_eq!(count(&world, "-> Write"), written, "nothing was written");
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
