//! The dashboard entered with one password: set on a first start from the
//! environment, checked at the door through a derivation the world
//! performs, and bought as a session a restart forgets — see
//! `docs/decisions/0084-the-instance-authenticates-itself.md`.

use stageman_core::{Outcome, Progress};
use stageman_instance::{ENTRY_PATH, Instance, LOGIN_PATH, PASSWORD_VARIABLE, UP_PATH};
use stageman_vocabulary::{Environment, Now, RequestId};

use crate::simulation::{Sent, Simulation, job, seed, tunnel_host, watching};

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
    let script = sim.browses(now, "GET", "/wasm/stageman.js", &[], "");
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
    assert_eq!(sim.route(script), Some(Sent::To(9000)), "and what wakes it");
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

/// Visits a job's host with the headers given, and runs until answered.
fn visits_job(
    sim: &mut Simulation,
    instance: &mut Instance,
    which: u128,
    path: &str,
    headers: &[(&str, &str)],
) -> RequestId {
    let now = sim.now();
    let host = tunnel_host(&job(which));
    let mut with_host = vec![("host", host.as_str())];
    with_host.extend_from_slice(headers);
    let id = sim.browses_on(now, "GET", path, &with_host);
    sim.run_until(instance, now + 5);
    id
}

/// A job's host is entered through the apex: a visit without a session is
/// sent to the apex to be granted one, the apex grants it to somebody
/// signed in and sends the browser to the host's own entry with a token,
/// the entry spends the token for a session of the host's own, and every
/// forwarded request is stripped of the cookies of this instance's.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one walk from the cold visit to the stripped forward and every refusal after, \
              and splitting it would set the host up three times to say the same thing"
)]
fn a_jobs_host_is_entered_through_the_apex() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[
        (job(1), Progress::Working),
        (job(2), Progress::Working),
        (job(3), Progress::Retired(Outcome::Done)),
    ]));
    for which in [1, 2] {
        let (name, held) = Simulation::ours(&stageman_job::container(&job(which)));
        sim.container(&name, held);
    }
    let mut instance = started(&mut sim, with_a_password("correct horse"));
    let port = sim
        .port_of(&stageman_job::container(&job(1)))
        .expect("resumed, so running on a port");

    // Without a session for the host, the browser is sent to the apex,
    // carrying where it was going.
    let cold = visits_job(&mut sim, &mut instance, 1, "/page?x=1", &[]);
    assert_eq!(sim.status(cold), Some(303));
    let entry = format!(
        "http://localhost:8080/enter/{}?back=%2Fpage%3Fx%3D1",
        job(1)
    );
    assert_eq!(sim.header(cold, "location"), Some(entry.as_str()));

    // The apex asks for its own session first, then grants one for the
    // host and sends the browser to the host's entry with a token.
    let now = sim.now();
    let path = format!("/enter/{}?back=%2Fpage%3Fx%3D1", job(1));
    let unsigned = sim.browses(now, "GET", &path, &[], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.status(unsigned), Some(303));
    assert!(
        sim.header(unsigned, "location")
            .is_some_and(|to| to.starts_with("/login?back=")),
        "the apex's own login first"
    );
    let now = sim.now();
    let right = logs_in(&mut sim, &mut instance, now, "correct+horse", "");
    let cookie = cookie_of(&sim, right);
    let now = sim.now();
    let granted = sim.browses(now, "GET", &path, &[("cookie", &cookie)], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.status(granted), Some(303));
    let to = sim
        .header(granted, "location")
        .expect("sent to the host")
        .to_owned();
    let prefix = format!("http://{}:8080{ENTRY_PATH}?t=", tunnel_host(&job(1)));
    assert!(to.starts_with(&prefix), "{to}");
    let token = to.strip_prefix(&prefix).expect("a token").to_owned();

    // The host's entry spends the token for a session of its own, and
    // sends the browser where it was going.
    let entered = visits_job(
        &mut sim,
        &mut instance,
        1,
        &format!("{ENTRY_PATH}?t={token}"),
        &[],
    );
    assert_eq!(sim.status(entered), Some(303));
    assert_eq!(sim.header(entered, "location"), Some("/page?x=1"));
    let set = sim
        .header(entered, "set-cookie")
        .expect("a session for the host");
    assert!(
        set.starts_with("stageman_tunnel=") && set.ends_with("; Path=/; HttpOnly; SameSite=Lax"),
        "{set}"
    );
    let tunnel_cookie = set
        .split_once(';')
        .map(|(pair, _)| pair.to_owned())
        .expect("a pair");

    // With it, the request is forwarded to the container, stripped of every
    // cookie of this instance's and of nothing else.
    let warm = visits_job(
        &mut sim,
        &mut instance,
        1,
        "/page?x=1",
        &[("cookie", &tunnel_cookie)],
    );
    assert_eq!(sim.route(warm), Some(Sent::To(port)));
    let stripped = sim.stripped(warm).expect("forwarded with a strip list");
    assert!(
        stripped.iter().any(|name| name == "stageman_tunnel"),
        "{stripped:?}"
    );
    assert!(
        stripped
            .iter()
            .any(|name| name == "__Host-stageman_session"),
        "{stripped:?}"
    );

    // A token is spent once, and the host sends the browser back to the
    // apex for another rather than refusing outright.
    let spent = visits_job(
        &mut sim,
        &mut instance,
        1,
        &format!("{ENTRY_PATH}?t={token}"),
        &[],
    );
    assert_eq!(sim.status(spent), Some(303));
    assert!(
        sim.header(spent, "location")
            .is_some_and(|to| to.starts_with("http://localhost:8080/enter/")),
        "back to the apex"
    );

    // A session for one host opens no other, and a grant for one host is
    // refused at another's entry.
    let elsewhere = visits_job(
        &mut sim,
        &mut instance,
        2,
        "/",
        &[("cookie", &tunnel_cookie)],
    );
    assert_eq!(sim.status(elsewhere), Some(303), "another host's session");
    let now = sim.now();
    let for_one = sim.browses(
        now,
        "GET",
        &format!("/enter/{}", job(1)),
        &[("cookie", &cookie)],
        "",
    );
    sim.run_until(&mut instance, now);
    let to = sim.header(for_one, "location").expect("granted").to_owned();
    let token = to
        .rsplit_once("?t=")
        .map(|(_, token)| token.to_owned())
        .expect("a token");
    let misused = visits_job(
        &mut sim,
        &mut instance,
        2,
        &format!("{ENTRY_PATH}?t={token}"),
        &[],
    );
    assert_eq!(sim.status(misused), Some(303));
    assert!(
        sim.header(misused, "location")
            .is_some_and(|to| to.starts_with("http://localhost:8080/enter/")),
        "another host's grant buys nothing here"
    );

    // A job that is over has no host to enter.
    let now = sim.now();
    let over = sim.browses(
        now,
        "GET",
        &format!("/enter/{}", job(3)),
        &[("cookie", &cookie)],
        "",
    );
    let nobody = sim.browses(now, "GET", "/enter/not-a-name", &[("cookie", &cookie)], "");
    sim.run_until(&mut instance, now);
    assert_eq!(sim.status(over), Some(404));
    assert_eq!(sim.status(nobody), Some(404));
}

/// A grant lapses a minute after it was minted.
#[test]
fn a_grant_lapses_after_a_minute() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[(job(1), Progress::Working)]));
    let (name, held) = Simulation::ours(&stageman_job::container(&job(1)));
    sim.container(&name, held);
    let mut instance = started(&mut sim, with_a_password("correct horse"));
    let now = sim.now();
    let right = logs_in(&mut sim, &mut instance, now, "correct+horse", "");
    let cookie = cookie_of(&sim, right);
    let now = sim.now();
    let granted = sim.browses(
        now,
        "GET",
        &format!("/enter/{}", job(1)),
        &[("cookie", &cookie)],
        "",
    );
    sim.run_until(&mut instance, now);
    let to = sim.header(granted, "location").expect("granted").to_owned();
    let token = to
        .rsplit_once("?t=")
        .map(|(_, token)| token.to_owned())
        .expect("a token");

    let late = sim.now() + 61_000;
    let host = tunnel_host(&job(1));
    let lapsed = sim.browses_on(
        late,
        "GET",
        &format!("{ENTRY_PATH}?t={token}"),
        &[("host", &host)],
    );
    sim.run_until(&mut instance, late + 5);
    assert_eq!(sim.status(lapsed), Some(303));
    assert!(
        sim.header(lapsed, "location")
            .is_some_and(|to| to.starts_with("http://localhost:8080/enter/")),
        "lapsed, so back to the apex"
    );
}
