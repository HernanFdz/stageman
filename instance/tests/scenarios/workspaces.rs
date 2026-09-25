//! Where the instance's Slack app is installed: a workspace, by the
//! platform's redirect — see
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`.
//! An install link minted per press with a state the tab comes back under;
//! the code exchanged and the workspace kept beside the app with the tab
//! closing itself; a state nobody minted keeping the workspace with the tab
//! staying; an exchange the platform refuses keeping nothing and saying why
//! on the tab and the page; a person who did not allow; a workspace
//! forgotten, and the app forgotten taking its held state with it; a second
//! install of the same workspace refreshing its record; and a daemon dying
//! mid-exchange keeping nothing.

use stageman_channel::Call;
use stageman_core::{Channel, ChannelApp, Secret, State};
use stageman_instance::{Instance, Request, Response};
use stageman_wire::{InstallLink, Refusal};

use crate::dashboard::{ask, count, first, nth};
use crate::simulation::{Simulation, seed, watching};

/// An instance holding the app, installed nowhere yet.
fn holding_the_app() -> State {
    let mut state = watching(&[]);
    state.channel_apps.insert(
        Channel::Slack,
        ChannelApp {
            client_id: "1234.5678".to_owned(),
            client_secret: Secret::new("s3cret".to_owned()),
            app_token: Secret::new("xapp-1".to_owned()),
            workspaces: std::collections::BTreeMap::new(),
        },
    );
    state
}

/// An install link, minted for one press.
fn pressed(sim: &mut Simulation, instance: &mut Instance, id: u64) -> InstallLink {
    let Response::InstallLink(minted) = ask(
        sim,
        instance,
        id,
        Request::WorkspaceLink {
            channel: "slack".to_owned(),
        },
    ) else {
        panic!("an install link");
    };
    minted
}

/// The Instance page.
fn instance_page(sim: &mut Simulation, instance: &mut Instance, id: u64) -> stageman_wire::Apps {
    let Response::Apps(shown) = ask(sim, instance, id, Request::Apps) else {
        panic!("the Instance page");
    };
    shown
}

/// The path the platform sends the browser back to: with a code, or with
/// its word where the person did not allow, and the state the link
/// carried where the tab was opened from a page here.
fn back(code: Option<&str>, error: Option<&str>, state: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(code) = code {
        parts.push(format!("code={code}"));
    }
    if let Some(error) = error {
        parts.push(format!("error={error}"));
    }
    if let Some(state) = state {
        parts.push(format!("state={state}"));
    }
    format!("/instance/apps/slack/installed?{}", parts.join("&"))
}

/// The browser comes back, and the platform's answer lands: the page the
/// tab was answered with.
fn arrives(sim: &mut Simulation, instance: &mut Instance, path: &str) -> String {
    let arrived = sim.visits_path(sim.now(), path);
    let until = sim.now() + 5_000;
    sim.run_until(instance, until);
    assert_eq!(
        sim.tool_answer(arrived).map(|(status, _)| *status),
        Some(200),
        "the tab is answered with a page"
    );
    sim.answer_text(arrived)
        .expect("the page has text")
        .to_owned()
}

/// A page closing the tab says the app is installed on the workspace, and
/// that the page the tab was opened from has it.
fn closes_saying_installed_on(page: &str, name: &str) -> bool {
    page.contains(&format!(
        "The app is installed on <b>{name}</b>. Back in stageman, the page you left has it."
    )) && page.contains("window.close()")
}

/// A page staying open says why the app was not installed.
fn stays_saying(page: &str, why: &str) -> bool {
    page.contains(&format!("The app was not installed: {why}.")) && !page.contains("window.close()")
}

/// The exchanges made so far, by their codes.
fn exchanged(sim: &Simulation) -> Vec<String> {
    sim.channel_calls()
        .iter()
        .filter_map(|(_, call)| match call {
            Call::Exchange { code, .. } => Some(code.clone()),
            _ => None,
        })
        .collect()
}

/// A press mints a link onto the platform's authorisation page carrying
/// the app's client identifier, the bot scopes, a state of its own and
/// the redirect onto this instance; every press its own state; and no
/// link without an app.
#[test]
fn a_workspace_link_is_minted_per_press_with_a_state_of_its_own() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));

    let first = pressed(&mut sim, &mut instance, 1);
    assert!(
        first.link.starts_with(
            "https://slack.com/oauth/v2/authorize?client_id=1234.5678&scope=chat:write,"
        ),
        "{}",
        first.link
    );
    assert!(
        first.link.ends_with(&format!(
            "&state={}&redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Finstance%2Fapps%2Fslack%2Finstalled",
            first.state
        )),
        "{}",
        first.link
    );
    let second = pressed(&mut sim, &mut instance, 2);
    assert_ne!(first.state, second.state, "a state per press");

    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::WorkspaceLink {
                channel: "slack".to_owned()
            }
        ),
        Response::Refused(Refusal::ChannelAppMissing {
            channel: "Slack".to_owned()
        })
    );
}

/// The browser comes back with a code under the state its press minted:
/// the code is exchanged with the client pair, the workspace is kept
/// beside the app with the bot token minted for it — once the write has
/// landed, which is when the tab is answered with the page that closes
/// it — and the Instance page lists it with nothing of the token.
#[test]
fn a_workspace_installs_by_the_redirect_and_is_kept_under_the_state() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);
    let writes_before = count(&sim, "-> Write");

    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de"), None, Some(&minted.state)),
    );
    assert_eq!(exchanged(&sim), vec!["c0de".to_owned()]);
    assert!(closes_saying_installed_on(&page, "Acme"), "{page}");
    let written = nth(&sim, "-> Write", writes_before);
    assert!(
        written < first(&sim, "-> Answer"),
        "the tab is answered once the record landed"
    );

    let kept = sim.disk().expect("landed");
    let app = kept.channel_apps.get(&Channel::Slack).expect("the app");
    let workspace = app.workspaces.get("T0TEAM").expect("the workspace is kept");
    assert_eq!(workspace.name, "Acme");
    assert_eq!(workspace.bot_user, "U0BOT");
    assert_eq!(workspace.bot_token.expose(), "xoxb-sim-c0de");

    let shown = instance_page(&mut sim, &mut instance, 2);
    let slack = shown.slack.clone().expect("the app is shown");
    assert_eq!(
        slack
            .workspaces
            .iter()
            .map(|workspace| (
                workspace.id.as_str(),
                workspace.name.as_str(),
                workspace.used_by.len()
            ))
            .collect::<Vec<_>>(),
        vec![("T0TEAM", "Acme", 0)]
    );
    assert_eq!(slack.install_failure, None);
    let served = serde_json::to_string(&shown).expect("it serialises");
    assert!(!served.contains("xoxb-sim"), "{served}");
}

/// A workspace arriving under a state nobody here minted is kept all the
/// same, and the tab stays open saying to press again, since no page will
/// learn of it; one arriving under no state at all is nobody's to
/// announce, and the tab closes.
#[test]
fn a_workspace_arriving_under_a_state_nobody_minted_is_kept_and_the_tab_stays() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));

    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de"), None, Some("nobody")),
    );
    assert!(
        page.contains("opened before stageman last started") && !page.contains("window.close()"),
        "{page}"
    );
    assert!(
        sim.disk()
            .expect("landed")
            .channel_apps
            .get(&Channel::Slack)
            .expect("the app")
            .workspaces
            .contains_key("T0TEAM"),
        "kept all the same"
    );

    let page = arrives(&mut sim, &mut instance, &back(Some("c0de2"), None, None));
    assert!(closes_saying_installed_on(&page, "Acme"), "{page}");
}

/// An exchange the platform refuses keeps nothing: the tab stays saying
/// the platform's word, the Instance page says it until the next install
/// is kept, and nothing is written.
#[test]
fn an_exchange_the_platform_refuses_keeps_nothing_and_the_tab_says_why() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);
    let writes_before = count(&sim, "-> Write");
    sim.next_exchange_fails("invalid_code");

    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("st4le"), None, Some(&minted.state)),
    );
    assert!(
        stays_saying(&page, "Slack refused it (invalid_code)"),
        "{page}"
    );
    assert_eq!(count(&sim, "-> Write"), writes_before, "nothing written");
    let shown = instance_page(&mut sim, &mut instance, 2);
    let slack = shown.slack.expect("the app is shown");
    assert!(slack.workspaces.is_empty());
    assert_eq!(
        slack.install_failure.as_deref(),
        Some("Slack refused it (invalid_code)")
    );

    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de"), None, Some(&minted.state)),
    );
    assert!(closes_saying_installed_on(&page, "Acme"), "{page}");
    let shown = instance_page(&mut sim, &mut instance, 3);
    assert_eq!(shown.slack.expect("the app").install_failure, None);
}

/// A person who did not allow brings the platform's word back in the
/// code's place: nothing is asked, nothing kept, and the tab says so. A
/// tab arriving with neither a code nor a word is not the platform's
/// redirect, and is told so.
#[test]
fn a_person_who_did_not_allow_brings_back_the_platforms_word_and_nothing_is_asked() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);
    let writes_before = count(&sim, "-> Write");

    let page = arrives(
        &mut sim,
        &mut instance,
        &back(None, Some("access_denied"), Some(&minted.state)),
    );
    assert!(stays_saying(&page, "Slack said access_denied"), "{page}");
    assert!(exchanged(&sim).is_empty(), "nothing asked");
    assert_eq!(count(&sim, "-> Write"), writes_before);

    let bare = sim.visits_path(sim.now(), &back(None, None, Some(&minted.state)));
    let until = sim.now() + 5_000;
    sim.run_until(&mut instance, until);
    assert_eq!(sim.tool_answer(bare).map(|(status, _)| *status), Some(400));
}

/// A workspace is forgotten from the page, and forgetting again is refused
/// by name; forgetting the app forgets what was held for its installs, so
/// a tab coming back afterwards is told there is no app.
#[test]
fn a_workspace_is_forgotten_and_the_app_takes_its_installs_with_it() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);
    arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de"), None, Some(&minted.state)),
    );

    let Response::Apps(shown) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::ForgetWorkspace {
            channel: "slack".to_owned(),
            id: "T0TEAM".to_owned(),
        },
    ) else {
        panic!("the Instance page");
    };
    assert!(shown.slack.expect("the app").workspaces.is_empty());
    assert!(
        sim.disk()
            .expect("landed")
            .channel_apps
            .get(&Channel::Slack)
            .expect("the app")
            .workspaces
            .is_empty()
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::ForgetWorkspace {
                channel: "slack".to_owned(),
                id: "T0TEAM".to_owned(),
            },
        ),
        Response::Refused(Refusal::NoSuchWorkspace {
            id: "T0TEAM".to_owned()
        })
    );

    let again = pressed(&mut sim, &mut instance, 4);
    let Response::Apps(_) = ask(
        &mut sim,
        &mut instance,
        5,
        Request::ForgetChannelApp {
            channel: "slack".to_owned(),
        },
    ) else {
        panic!("the Instance page");
    };
    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de2"), None, Some(&again.state)),
    );
    assert!(
        stays_saying(&page, "no Slack app is registered on this instance"),
        "{page}"
    );
}

/// Installing on a workspace the app is already installed on refreshes its
/// record: the bot token minted last is the one kept, and it is still one
/// workspace.
#[test]
fn a_second_install_of_the_same_workspace_refreshes_its_record() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);
    arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de"), None, Some(&minted.state)),
    );
    let again = pressed(&mut sim, &mut instance, 2);
    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de2"), None, Some(&again.state)),
    );
    assert!(closes_saying_installed_on(&page, "Acme"), "{page}");

    let kept = sim.disk().expect("landed");
    let app = kept.channel_apps.get(&Channel::Slack).expect("the app");
    assert_eq!(app.workspaces.len(), 1);
    assert_eq!(
        app.workspaces
            .get("T0TEAM")
            .map(|workspace| workspace.bot_token.expose()),
        Some("xoxb-sim-c0de2")
    );
}

/// A daemon dying between the tab's arrival and the platform's answer
/// keeps nothing, and the next start knows nothing of the state: the same
/// tab coming back to it is kept and told no page will learn of it.
#[test]
fn a_daemon_dying_mid_exchange_keeps_nothing() {
    let mut sim = Simulation::new();
    sim.holding(&holding_the_app());
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);

    let arrived = sim.visits_path(sim.now(), &back(Some("c0de"), None, Some(&minted.state)));
    let Some(event) = sim.next() else {
        panic!("the arrival is queued");
    };
    for effect in instance.step(sim.now(), event) {
        sim.perform(effect);
    }
    assert_eq!(
        exchanged(&sim),
        vec!["c0de".to_owned()],
        "the code went out"
    );

    let mut instance = sim.crash(seed(1));
    sim.run_until(&mut instance, 5_000);
    assert!(sim.tool_answer(arrived).is_none(), "answered to nobody");
    assert!(
        instance
            .state()
            .channel_apps
            .get(&Channel::Slack)
            .expect("the app")
            .workspaces
            .is_empty(),
        "nothing kept"
    );

    let page = arrives(
        &mut sim,
        &mut instance,
        &back(Some("c0de2"), None, Some(&minted.state)),
    );
    assert!(
        page.contains("opened before stageman last started") && !page.contains("window.close()"),
        "{page}"
    );
}
