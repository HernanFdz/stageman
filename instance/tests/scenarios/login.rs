//! The dashboard entered with one password: set on a first start from the
//! environment, checked at the door through a derivation the world
//! performs, and bought as a session a restart forgets — see
//! `docs/decisions/0084-the-instance-authenticates-itself.md`.

use stageman_instance::{Instance, LOGIN_PATH, PASSWORD_VARIABLE, UP_PATH};
use stageman_vocabulary::{Environment, Now, RequestId};

use crate::simulation::{Sent, Simulation, seed};

/// The environment of a start that names a password.
fn with_a_password(password: &str) -> Environment {
    let mut environment = Simulation::environment();
    environment.insert(PASSWORD_VARIABLE.to_owned(), password.to_owned());
    environment
}

/// Starts an instance with the environment given and runs until it is
/// awake: the derivation a first start asks for is answered a tick later,
/// as the slow thing it stands in for would be.
fn started(sim: &mut Simulation, environment: Environment) -> Instance {
    let mut instance = sim.wake_given(seed(1), environment);
    let now = sim.now();
    sim.run_until(&mut instance, now + 5);
    instance
}

/// Posts the login form at a moment and runs until it has been answered.
fn logs_in(
    sim: &mut Simulation,
    instance: &mut Instance,
    at: Now,
    typed: &str,
    back: &str,
) -> RequestId {
    let body = format!("password={typed}&back={back}");
    let id = sim.browses(
        at,
        "POST",
        LOGIN_PATH,
        &[("content-type", "application/x-www-form-urlencoded")],
        &body,
    );
    sim.run_until(instance, at + 5);
    id
}

/// The cookie a successful login set, as a browser would send it back.
fn cookie_of(sim: &Simulation, id: RequestId) -> String {
    let set = sim.header(id, "set-cookie").expect("a session was set");
    set.split_once(';')
        .map(|(pair, _)| pair.to_owned())
        .expect("a cookie is a pair")
}

/// A first start that names a password hashes it once, keeps the hash, and
/// from then on the dashboard is behind the login: a page is sent to it,
/// a route is refused, and the login's own paths and the bundle are not.
#[test]
fn a_password_named_at_a_first_start_gates_the_dashboard() {
    let mut sim = Simulation::new();
    sim.recording(
        "a-login",
        "A first start names a password; a page is sent to the login, a wrong password earns a wait, the right one buys a session, and a restart forgets it",
    );
    let mut instance = started(&mut sim, with_a_password("correct horse"));
    assert!(
        instance.state().password.is_some(),
        "the hash is kept from the first start"
    );

    let now = sim.now();
    let page = sim.browses(now, "GET", "/projects?open=1", &[], "");
    let route = sim.browses(now, "GET", "/api/home", &[], "");
    let login = sim.browses(now, "GET", LOGIN_PATH, &[], "");
    let bundle = sim.browses(now, "GET", "/assets/styles.css", &[], "");
    let up = sim.browses(now, "GET", UP_PATH, &[], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.status(page), Some(303));
    assert_eq!(
        sim.header(page, "location"),
        Some("/login?back=%2Fprojects%3Fopen%3D1")
    );
    assert_eq!(sim.status(route), Some(401));
    assert_eq!(
        sim.route(login),
        Some(Sent::To(9000)),
        "the login page is served"
    );
    assert_eq!(sim.route(bundle), Some(Sent::To(9000)), "and what draws it");
    assert_eq!(sim.status(up), Some(200));

    // A wrong password is sent back to the login page saying so, and the
    // next attempt from the same address, before the wait is over, is
    // refused without being checked. The wait after one wrong password is
    // a second from the moment it was found wrong, which is a tick after
    // it was posted, since the derivation is answered a tick later.
    let wrong_at = sim.now();
    let wrong = logs_in(
        &mut sim,
        &mut instance,
        wrong_at,
        "battery+staple",
        "%2Fprojects",
    );
    assert_eq!(sim.status(wrong), Some(303));
    assert_eq!(
        sim.header(wrong, "location"),
        Some("/login?said=wrong&back=%2Fprojects")
    );
    let waited = logs_in(
        &mut sim,
        &mut instance,
        wrong_at + 1_000,
        "correct+horse",
        "%2Fprojects",
    );
    assert_eq!(
        sim.header(waited, "location"),
        Some("/login?said=wait&back=%2Fprojects")
    );

    // The moment the wait is over the right password buys a session, and
    // the browser is sent where it was going with the cookie.
    let right = logs_in(
        &mut sim,
        &mut instance,
        wrong_at + 1_001,
        "correct+horse",
        "%2Fprojects",
    );
    assert_eq!(sim.status(right), Some(303));
    assert_eq!(sim.header(right, "location"), Some("/projects"));
    let cookie = cookie_of(&sim, right);
    assert!(cookie.starts_with("stageman_session="), "{cookie}");
    assert!(
        sim.header(right, "set-cookie")
            .is_some_and(|set| set.ends_with("; Path=/; HttpOnly; SameSite=Lax")),
        "a local domain has no certificate to be Secure under"
    );

    let now = sim.now();
    let opened = sim.browses(now, "GET", "/projects", &[("cookie", &cookie)], "");
    let read = sim.browses(now, "GET", "/api/home", &[("cookie", &cookie)], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.route(opened), Some(Sent::To(9000)));
    assert_eq!(sim.route(read), Some(Sent::To(9000)));

    // One life per replay file: what follows is a second.
    sim.recorded();

    // A restart forgets every session, and the variable still in the
    // environment does not replace the password already set.
    let hashed = instance.state().password.clone();
    let mut instance = sim.crash_given(seed(2), with_a_password("something else"));
    let now = sim.now();
    sim.run_until(&mut instance, now + 5);
    assert_eq!(instance.state().password, hashed);
    let now = sim.now();
    let again = sim.browses(now, "GET", "/projects", &[("cookie", &cookie)], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.status(again), Some(303));
}

/// A session lapses a fortnight after its last use, and is renewed by
/// every use before then.
#[test]
fn a_session_lapses_a_fortnight_after_its_last_use() {
    let mut sim = Simulation::new();
    let mut instance = started(&mut sim, with_a_password("correct horse"));
    let now = sim.now();
    let right = logs_in(&mut sim, &mut instance, now, "correct+horse", "");
    let cookie = cookie_of(&sim, right);

    let day = 24 * 60 * 60 * 1000;
    let mut at = sim.now();
    for _ in 0..3 {
        at += 10 * day;
        let opened = sim.browses(at, "GET", "/", &[("cookie", &cookie)], "");
        sim.run_until(&mut instance, at);
        assert_eq!(sim.route(opened), Some(Sent::To(9000)), "renewed by use");
    }
    at += 15 * day;
    let lapsed = sim.browses(at, "GET", "/", &[("cookie", &cookie)], "");
    sim.run_until(&mut instance, at);
    assert_eq!(sim.status(lapsed), Some(303), "a fortnight unused");
}

/// Without a password set the dashboard is what it always was, and the
/// form posted anyway sends the browser where it was going.
#[test]
fn without_a_password_the_dashboard_is_open() {
    let mut sim = Simulation::new();
    let mut instance = sim.wake(seed(1));
    assert!(instance.state().password.is_none());

    let now = sim.now();
    let page = sim.browses(now, "GET", "/projects", &[], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.route(page), Some(Sent::To(9000)));

    let now = sim.now();
    let posted = logs_in(&mut sim, &mut instance, now, "anything", "%2Fprojects");
    assert_eq!(sim.status(posted), Some(303));
    assert_eq!(sim.header(posted, "location"), Some("/projects"));
}

/// Where the browser is sent back is a path on this host or the front
/// page, never an address somebody typed into the form.
#[test]
fn the_login_never_sends_a_browser_to_another_host() {
    let mut sim = Simulation::new();
    let mut instance = started(&mut sim, with_a_password("correct horse"));
    let now = sim.now();
    let elsewhere = logs_in(
        &mut sim,
        &mut instance,
        now,
        "correct+horse",
        "%2F%2Fevil.example%2F",
    );
    assert_eq!(sim.header(elsewhere, "location"), Some("/"));
}
