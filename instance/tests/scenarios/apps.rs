//! An App the instance owns, registered from the dashboard by the
//! platform's flow: the form minted, the browser back with a code, the
//! code exchanged and the App kept, a state nobody minted refused, a
//! platform that refuses said on the page, a daemon dying mid-flow keeping
//! nothing, and the App forgotten. See
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`.

use stageman_core::Platform;
use stageman_instance::{Instance, Request, Response};
use stageman_platform::Call as PlatformCall;
use stageman_wire::Refusal;

use crate::dashboard::{ask, count, first, nth};
use crate::simulation::{Simulation, seed, watching};

/// Asks the instance for a registration form, and hands back its state.
fn begun(sim: &mut Simulation, instance: &mut Instance, id: u64) -> stageman_wire::Registration {
    let Response::Registration(form) = ask(
        sim,
        instance,
        id,
        Request::Registration {
            platform: "github".to_owned(),
            anywhere: false,
        },
    ) else {
        panic!("a registration form");
    };
    form
}

/// The path the platform sends the browser back to, with a code and a state.
fn back(code: &str, state: &str) -> String {
    format!("/instance/apps/github/registered?code={code}&state={state}")
}

/// The form posts to the platform with the manifest and the state, the
/// browser comes back with a code, the code is exchanged, and the App is
/// kept with its key — once the write has landed, which is when the
/// browser is sent on to the page.
#[test]
fn an_app_is_registered_from_the_dashboard_and_kept() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");

    let form = begun(&mut sim, &mut instance, 1);
    assert_eq!(
        form.action,
        format!("https://github.com/settings/apps/new?state={}", form.state)
    );
    assert!(
        form.manifest.contains(r#""name":"stageman""#),
        "{}",
        form.manifest
    );
    assert!(
        form.manifest
            .contains(r#""redirect_url":"http://localhost:8080/instance/apps/github/registered""#),
        "the redirect is this instance's own address: {}",
        form.manifest
    );
    assert!(
        form.manifest.contains(r#""public":false"#),
        "{}",
        form.manifest
    );

    let arrived = sim.visits_path(sim.now(), &back("c0de", &form.state));
    sim.run_until(&mut instance, 5_000);

    let exchanged = sim
        .platform_calls()
        .iter()
        .find(|(_, call)| matches!(call, PlatformCall::Exchange { code, .. } if code == "c0de"))
        .map(|(at, _)| *at)
        .expect("the code was exchanged");
    let written = nth(&sim, "-> Write", writes_before);
    assert!(exchanged < written, "exchanged, then written");
    assert_eq!(
        sim.tool_answer(arrived).map(|(status, _)| *status),
        Some(303)
    );
    assert!(
        written < first(&sim, "-> Answer"),
        "sent on once the record landed"
    );

    let kept = sim.disk().expect("landed");
    let app = kept.apps.get(&Platform::GitHub).expect("the App is kept");
    assert_eq!(app.slug, "stageman-sim");
    assert_eq!(app.id, 4242);
    assert!(
        app.private_key
            .expose()
            .starts_with("-----BEGIN RSA PRIVATE KEY-----")
    );

    let Response::Apps(shown) = ask(&mut sim, &mut instance, 2, Request::Apps) else {
        panic!("the Instance page");
    };
    assert_eq!(
        shown
            .github
            .as_ref()
            .map(|app| (app.slug.as_str(), app.link.as_str())),
        Some(("stageman-sim", "https://github.com/apps/stageman-sim"))
    );
    assert_eq!(shown.failed, None);
    let served = serde_json::to_string(&shown).expect("it serialises");
    assert!(!served.contains("PRIVATE KEY"), "{served}");
}

/// A state this instance did not mint is a registration begun elsewhere
/// or before a restart: refused at once, with nothing asked of the
/// platform and nothing written.
#[test]
fn a_state_nobody_minted_is_refused_without_asking_the_platform() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");

    let arrived = sim.visits_path(sim.now(), &back("c0de", "deadbeef"));
    sim.run_until(&mut instance, 5_000);
    assert_eq!(
        sim.tool_answer(arrived).map(|(status, _)| *status),
        Some(400)
    );
    assert!(sim.platform_calls().is_empty(), "nothing asked");
    assert_eq!(count(&sim, "-> Write"), writes_before);
    assert!(instance.state().apps.is_empty());
}

/// A state buys one exchange: the browser coming back twice with the same
/// state is refused the second time.
#[test]
fn a_state_is_spent_on_its_first_return() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let form = begun(&mut sim, &mut instance, 1);

    sim.visits_path(sim.now(), &back("c0de", &form.state));
    sim.run_until(&mut instance, 5_000);
    let again = sim.visits_path(sim.now(), &back("c0de", &form.state));
    sim.run_until(&mut instance, 10_000);
    assert_eq!(sim.tool_answer(again).map(|(status, _)| *status), Some(400));
    assert_eq!(
        sim.platform_calls()
            .iter()
            .filter(|(_, call)| matches!(call, PlatformCall::Exchange { .. }))
            .count(),
        1
    );
}

/// A platform that refuses the code leaves nothing kept, sends the browser
/// on all the same, and the page says why.
#[test]
fn a_registration_the_platform_refuses_is_said_on_the_page() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");
    let form = begun(&mut sim, &mut instance, 1);

    sim.next_platform_answers(404, r#"{"message":"Not Found"}"#);
    let arrived = sim.visits_path(sim.now(), &back("c0de", &form.state));
    sim.run_until(&mut instance, 5_000);
    assert_eq!(
        sim.tool_answer(arrived).map(|(status, _)| *status),
        Some(303)
    );
    assert_eq!(count(&sim, "-> Write"), writes_before, "nothing kept");

    let Response::Apps(shown) = ask(&mut sim, &mut instance, 2, Request::Apps) else {
        panic!("the Instance page");
    };
    assert_eq!(shown.github, None);
    assert_eq!(
        shown.failed.as_deref(),
        Some("GitHub does not know that code — one is good for an hour, and once")
    );

    // The next registration that is kept clears what the last one said.
    let form = begun(&mut sim, &mut instance, 3);
    sim.visits_path(sim.now(), &back("c0de2", &form.state));
    sim.run_until(&mut instance, 10_000);
    let Response::Apps(shown) = ask(&mut sim, &mut instance, 4, Request::Apps) else {
        panic!("the Instance page");
    };
    assert!(shown.github.is_some());
    assert_eq!(shown.failed, None);
}

/// A daemon dying between the browser's return and the platform's answer
/// keeps nothing and answers nobody: the registration was held, never
/// kept, and the next start knows nothing of it.
#[test]
fn a_daemon_dying_mid_registration_keeps_nothing() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let form = begun(&mut sim, &mut instance, 1);

    let arrived = sim.visits_path(sim.now(), &back("c0de", &form.state));
    let Some(event) = sim.next() else {
        panic!("the arrival is queued");
    };
    for effect in instance.step(sim.now(), event) {
        sim.perform(effect);
    }
    assert_eq!(
        sim.platform_calls()
            .iter()
            .filter(|(_, call)| matches!(call, PlatformCall::Exchange { .. }))
            .count(),
        1,
        "the code went out for exchange"
    );

    let mut instance = sim.crash(seed(1));
    sim.run_until(&mut instance, 5_000);
    assert!(sim.tool_answer(arrived).is_none(), "answered to nobody");
    assert!(instance.state().apps.is_empty(), "nothing kept");
    // And a browser coming back to the new instance with the old state is
    // one it did not mint.
    let again = sim.visits_path(sim.now(), &back("c0de", &form.state));
    sim.run_until(&mut instance, 10_000);
    assert_eq!(sim.tool_answer(again).map(|(status, _)| *status), Some(400));
}

/// Forgetting takes the App and its key out of the file, and forgetting
/// what is not there is refused by name.
#[test]
fn the_app_is_forgotten_from_the_page() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let form = begun(&mut sim, &mut instance, 1);
    sim.visits_path(sim.now(), &back("c0de", &form.state));
    sim.run_until(&mut instance, 5_000);
    assert!(
        sim.disk()
            .expect("landed")
            .apps
            .contains_key(&Platform::GitHub)
    );

    let Response::Apps(shown) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::ForgetApp {
            platform: "github".to_owned(),
        },
    ) else {
        panic!("the Instance page");
    };
    assert_eq!(shown.github, None);
    assert!(sim.disk().expect("landed").apps.is_empty());
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::ForgetApp {
                platform: "github".to_owned(),
            },
        ),
        Response::Refused(Refusal::AppMissing {
            platform: "GitHub".to_owned()
        })
    );
}
