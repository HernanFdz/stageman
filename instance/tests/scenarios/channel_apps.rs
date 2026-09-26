//! The Slack app the instance owns, registered from the Instance page by
//! pasting three values: the app-level token checked against the platform
//! and the app kept with its secrets sealed, a token the platform refuses
//! keeping nothing, a value missing refused before the platform is asked,
//! the app forgotten, and re-registering keeping the workspaces of the same
//! app and dropping another's. See
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.

use std::collections::BTreeMap;

use stageman_channel::Call;
use stageman_core::{Channel, ChannelApp, Secret, State, Workspace};
use stageman_instance::{Instance, Request, Response};
use stageman_wire::Refusal;

use crate::dashboard::{ask, count};
use crate::simulation::{Simulation, seed, watching};

/// Asks the instance to register the app, with a client identifier given
/// and the two secrets as said.
fn registering(client_id: &str, client_secret: &str, app_token: &str) -> Request {
    Request::RegisterChannelApp {
        channel: "slack".to_owned(),
        client_id: client_id.to_owned(),
        client_secret: client_secret.to_owned(),
        app_token: app_token.to_owned(),
    }
}

/// Registers the app and hands back the page.
fn registered(sim: &mut Simulation, instance: &mut Instance, id: u64) -> stageman_wire::Apps {
    let Response::Apps(shown) = ask(
        sim,
        instance,
        id,
        registering("1234.5678", "s3cret", "xapp-1"),
    ) else {
        panic!("the Instance page");
    };
    shown
}

/// An instance already holding the app, installed on one workspace.
fn holding_the_app(client_id: &str) -> State {
    let mut state = watching(&[]);
    state.channel_apps.insert(
        Channel::Slack,
        ChannelApp {
            client_id: client_id.to_owned(),
            client_secret: Secret::new("old-secret".to_owned()),
            app_token: Secret::new("xapp-old".to_owned()),
            workspaces: BTreeMap::from([(
                "T0TEAM".to_owned(),
                Workspace {
                    name: "Acme".to_owned(),
                    bot_user: "U0BOT".to_owned(),
                    bot_token: Secret::new("xoxb-acme".to_owned()),
                },
            )]),
        },
    );
    state
}

/// The three values pasted, the app-level token checked by asking where
/// to connect, and the app kept — once the write has landed — with the
/// client identifier in the clear on the page and nothing else of it.
#[test]
fn a_slack_app_is_registered_from_the_page_and_kept() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");
    let asked_before = sim.channel_calls().len();

    let shown = registered(&mut sim, &mut instance, 1);
    let asked: Vec<&Call> = sim.channel_calls()[asked_before..]
        .iter()
        .map(|(_, call)| call)
        .collect();
    assert!(
        matches!(asked.as_slice(), [Call::OpenSocket { .. }]),
        "the app-level token is checked by asking where to connect, and nothing else is asked"
    );
    assert_eq!(count(&sim, "-> Write"), writes_before + 1);

    let slack = shown.slack.clone().expect("the app is shown");
    assert_eq!(slack.client_id, "1234.5678");
    assert!(slack.workspaces.is_empty());
    assert!(
        shown
            .slack_form
            .starts_with("https://api.slack.com/apps?new_app=1&manifest_yaml="),
        "{}",
        shown.slack_form
    );
    let served = serde_json::to_string(&shown).expect("it serialises");
    assert!(
        !served.contains("s3cret") && !served.contains("xapp-1"),
        "{served}"
    );

    let kept = sim.disk().expect("landed");
    let app = kept
        .channel_apps
        .get(&Channel::Slack)
        .expect("the app is kept");
    assert_eq!(app.client_id, "1234.5678");
    assert_eq!(app.client_secret.expose(), "s3cret");
    assert_eq!(app.app_token.expose(), "xapp-1");
    assert!(app.workspaces.is_empty());
}

/// An app-level token the platform refuses keeps nothing: the refusal
/// names the token's box with the platform's word, and nothing is written.
#[test]
fn a_slack_app_whose_token_the_platform_refuses_is_not_kept() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");
    sim.next_locate_fails("invalid_auth");

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            registering("1234.5678", "s3cret", "xapp-wrong")
        ),
        Response::Refused(Refusal::ChannelRefused {
            listening: true,
            why: "Slack refused it (invalid_auth)".to_owned(),
        })
    );
    assert_eq!(count(&sim, "-> Write"), writes_before);
    assert!(sim.disk().expect("the file").channel_apps.is_empty());
}

/// A value left blank is refused before the platform is asked anything,
/// and a channel this build does not know is refused by name.
#[test]
fn a_slack_app_needs_all_three_values_before_the_platform_is_asked() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");
    let asked_before = sim.channel_calls().len();

    for (client_id, client_secret, app_token) in [
        ("", "s3cret", "xapp-1"),
        ("1234.5678", " ", "xapp-1"),
        ("1234.5678", "s3cret", ""),
    ] {
        assert_eq!(
            ask(
                &mut sim,
                &mut instance,
                1,
                registering(client_id, client_secret, app_token)
            ),
            Response::Refused(Refusal::ChannelAppIncomplete)
        );
    }
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::RegisterChannelApp {
                channel: "discord".to_owned(),
                client_id: "1234.5678".to_owned(),
                client_secret: "s3cret".to_owned(),
                app_token: "xapp-1".to_owned(),
            }
        ),
        Response::Refused(Refusal::ChannelAppMissing {
            channel: "discord".to_owned()
        })
    );
    assert_eq!(sim.channel_calls().len(), asked_before);
    assert_eq!(count(&sim, "-> Write"), writes_before);
}

/// The app is forgotten from the page, and forgetting again is refused,
/// as is forgetting on a channel this build does not know.
#[test]
fn the_slack_app_is_forgotten_from_the_page() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    registered(&mut sim, &mut instance, 1);

    let Response::Apps(shown) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::ForgetChannelApp {
            channel: "slack".to_owned(),
        },
    ) else {
        panic!("the Instance page");
    };
    assert_eq!(shown.slack, None);
    assert!(sim.disk().expect("landed").channel_apps.is_empty());
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::ForgetChannelApp {
                channel: "slack".to_owned(),
            },
        ),
        Response::Refused(Refusal::ChannelAppMissing {
            channel: "Slack".to_owned()
        })
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            4,
            Request::ForgetChannelApp {
                channel: "discord".to_owned(),
            },
        ),
        Response::Refused(Refusal::ChannelAppMissing {
            channel: "discord".to_owned()
        })
    );
}

/// Registering over an app already kept replaces its values, keeping the
/// workspaces when it is the same app by its client identifier, and
/// dropping them when it is another's.
#[test]
fn re_registering_keeps_the_same_apps_workspaces_and_drops_anothers() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app("1234.5678"));
    let mut instance = sim.wake(seed(1));

    let shown = registered(&mut sim, &mut instance, 1);
    let slack = shown.slack.expect("the app is shown");
    assert_eq!(
        slack
            .workspaces
            .iter()
            .map(|workspace| (workspace.id.as_str(), workspace.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("T0TEAM", "Acme")]
    );
    let kept = sim.disk().expect("landed");
    let app = kept.channel_apps.get(&Channel::Slack).expect("the app");
    assert_eq!(app.app_token.expose(), "xapp-1");
    assert_eq!(app.client_secret.expose(), "s3cret");
    assert_eq!(
        app.workspaces
            .get("T0TEAM")
            .map(|workspace| workspace.bot_token.expose()),
        Some("xoxb-acme")
    );

    let mut sim = Simulation::new();
    sim.holding(&holding_the_app("another.app"));
    let mut instance = sim.wake(seed(1));
    let shown = registered(&mut sim, &mut instance, 1);
    assert!(shown.slack.expect("the app is shown").workspaces.is_empty());
    assert!(
        sim.disk()
            .expect("landed")
            .channel_apps
            .get(&Channel::Slack)
            .expect("the app")
            .workspaces
            .is_empty()
    );
}
