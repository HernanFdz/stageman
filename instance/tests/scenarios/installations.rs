//! Where the App is installed, what an access reaches, and the tokens a
//! project's jobs run on — see
//! `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`
//! and
//! `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`:
//! the form told whether an App is registered, an install link minted per
//! press with a state the tab comes back under, an installation confirmed
//! with the App's key and kept beside it with the tab closing itself, what
//! came back under a state listed for the form that holds it and for no
//! other, a foreign installation refused with the tab staying to say so,
//! an arrival naming none refused without asking, an update refreshing
//! the record and dropping the tokens minted on it, an arrival under a
//! state nobody here minted kept with the tab staying, what a project
//! holds listed for its own form, a token's reach listed and a bad token
//! answered with why, a project created on the installation its tab
//! brought back with its coverage checked and the state spent, a kept
//! access checked again when the repository moves, the access and the
//! repository required, an installation refused forgetting while a
//! project names it, a token minted once for a project's jobs and served
//! through its hour, and a daemon dying mid-installation keeping nothing
//! and forgetting the states minted before it.

use stageman_core::{Access, Platform, Progress};
use stageman_instance::{Instance, Request, Response};
use stageman_platform::Call as PlatformCall;
use stageman_wire::{
    AccessDraft, AccessView, InstallLink, InstallationView, Reachable, Reached, Refusal, Through,
};

use crate::dashboard::{a_draft, ask, count, first, nth, repo};
use crate::simulation::{
    NEARBY, Simulation, app_installed, installed, job, on_github, project, seed, warrant_of,
    watching, with_an_app,
};

/// The projects screen, as the settings page reads it.
fn screen(sim: &mut Simulation, instance: &mut Instance, id: u64) -> stageman_wire::Watching {
    let Response::Projects(shown) = ask(sim, instance, id, Request::Projects) else {
        panic!("the projects screen");
    };
    shown
}

/// The Instance page.
fn instance_page(sim: &mut Simulation, instance: &mut Instance, id: u64) -> stageman_wire::Apps {
    let Response::Apps(shown) = ask(sim, instance, id, Request::Apps) else {
        panic!("the Instance page");
    };
    shown
}

/// The fixture's project, as the screen shows it.
fn the_project(shown: &stageman_wire::Watching) -> &stageman_wire::Project {
    shown
        .projects
        .iter()
        .find(|listed| listed.id == project().to_string())
        .expect("the fixture's project")
}

/// An install link, minted for one press.
fn pressed(sim: &mut Simulation, instance: &mut Instance, id: u64) -> InstallLink {
    let Response::InstallLink(minted) = ask(
        sim,
        instance,
        id,
        Request::InstallLink {
            platform: "github".to_owned(),
        },
    ) else {
        panic!("an install link");
    };
    minted
}

/// The path the platform sends the browser back to after an installation,
/// or after an update to one, with the state the link carried where the
/// tab was opened from a page here.
fn back(installation: u64, update: bool, state: Option<&str>) -> String {
    let action = if update { "update" } else { "install" };
    let state = state
        .map(|state| format!("&state={state}"))
        .unwrap_or_default();
    format!(
        "/instance/apps/github/installed?installation_id={installation}&setup_action={action}{state}"
    )
}

/// The platform calls made so far, in order, as the platform crate reads
/// them back.
fn calls(sim: &Simulation) -> Vec<PlatformCall> {
    sim.platform_calls()
        .iter()
        .map(|(_, call)| call.clone())
        .collect()
}

/// A state with the simulated App registered and the fixture's project on
/// a repository of the platform's.
fn app_and_project(full_name: &str) -> stageman_core::State {
    let mut state = watching(&[]);
    with_an_app(&mut state);
    on_github(&mut state, full_name);
    state
}

/// The browser comes back from installing, under a state or none, and the
/// platform's answers land: the page the tab was answered with.
fn arrives_installed(
    sim: &mut Simulation,
    instance: &mut Instance,
    installation: u64,
    state: Option<&str>,
) -> String {
    let arrived = sim.visits_path(sim.now(), &back(installation, false, state));
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

/// A press and its return: the state minted, and the installation kept
/// under it.
fn installed_from_a_page(
    sim: &mut Simulation,
    instance: &mut Instance,
    installation: u64,
) -> String {
    let minted = pressed(sim, instance, 900 + installation);
    let page = arrives_installed(sim, instance, installation, Some(&minted.state));
    assert!(page.contains("window.close()"), "{page}");
    minted.state
}

/// A page closing the tab says the App is installed on the account, and
/// that the page the tab was opened from has it.
fn closes_saying_installed_on(page: &str, account: &str) -> bool {
    page.contains(&format!(
        "The App is installed on <b>{account}</b>. Back in stageman, the page you left has it."
    )) && page.contains("window.close()")
}

/// A page staying open says why the App was not installed.
fn stays_saying(page: &str, why: &str) -> bool {
    page.contains(&format!("The App was not installed: {why}.")) && !page.contains("window.close()")
}

/// What an access reaches, as a form asks it.
fn reaches(sim: &mut Simulation, instance: &mut Instance, id: u64, through: Through) -> Response {
    ask(sim, instance, id, Request::Reaches { through })
}

/// What came back under a state, as the form holding it asks.
fn arrived(state: &str) -> Through {
    Through::Arrived {
        state: state.to_owned(),
    }
}

/// One row of a listing, from `owner/name`.
fn row(full_name: &str, private: bool) -> Reachable {
    Reachable {
        repository: named(full_name),
        private,
    }
}

/// A repository from `owner/name`, as a form names it.
fn named(full_name: &str) -> stageman_wire::Repository {
    let (owner, name) = full_name.split_once('/').expect("owner/name");
    repo(owner, name)
}

/// A draft on the App — on the installation that came back under a
/// state, or on the one the project holds — on a repository of the
/// fixture's account.
fn on_the_app(name: &str, arrival: Option<&str>, repository: &str) -> stageman_wire::Draft {
    let mut draft = a_draft(name);
    draft.access = AccessDraft::App {
        arrival: arrival.map(str::to_owned),
        repository: Some(named(repository)),
    };
    draft
}

/// A draft with a token — one set, or the one held — on a repository of
/// the fixture's account.
fn with_a_token(name: &str, token: Option<&str>, repository: &str) -> stageman_wire::Draft {
    let mut draft = a_draft(name);
    draft.access = AccessDraft::Token {
        token: token.map(str::to_owned),
        repository: Some(named(repository)),
    };
    draft
}

/// The form is told whether an App is registered, and nothing more of it:
/// where it is installed is the instance's business.
#[test]
fn the_projects_screen_says_whether_an_app_is_registered() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let shown = screen(&mut sim, &mut instance, 1);
    assert!(!shown.app_registered);
    assert_eq!(
        the_project(&shown).access,
        None,
        "the fixture holds nothing"
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::InstallLink {
                platform: "github".to_owned()
            }
        ),
        Response::Refused(Refusal::AppMissing {
            platform: "GitHub".to_owned()
        }),
        "nothing to install"
    );

    let mut sim = Simulation::new();
    let mut state = app_and_project("example/repo");
    app_installed(&mut state, 77, "acme");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    let shown = screen(&mut sim, &mut instance, 1);
    assert!(shown.app_registered);
}

/// A press mints an install link carrying a state of its own, held: the
/// browser comes back under it with an installation, which is fetched
/// with the App's key, kept beside the App with its account before the
/// tab is answered, and named under the state; the tab is answered with
/// the page that closes it. The installation is shown on the Instance
/// page; the project it may have been begun from is not touched, since an
/// installation is the App's until a save names it.
#[test]
fn an_install_link_is_minted_per_press_and_the_tab_comes_back_under_its_state() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");
    sim.installed_on(77, "acme");

    let first_press = pressed(&mut sim, &mut instance, 1);
    let second_press = pressed(&mut sim, &mut instance, 2);
    assert_eq!(
        first_press.link,
        format!(
            "https://github.com/apps/stageman-sim/installations/new?state={}",
            first_press.state
        )
    );
    assert_ne!(first_press.state, second_press.state, "one state per press");
    assert_eq!(
        count(&sim, "-> Write"),
        writes_before,
        "a state is held, never kept"
    );

    let arrived = sim.visits_path(sim.now(), &back(77, false, Some(&second_press.state)));
    sim.run_until(&mut instance, 5_000);

    assert_eq!(
        calls(&sim),
        [PlatformCall::Installation {
            platform: Platform::GitHub,
            id: 77
        }],
        "fetched, and nothing more: what it covers is listed when the form asks"
    );
    let written = nth(&sim, "-> Write", writes_before);
    assert!(
        written < first(&sim, "-> Answer"),
        "kept, then the tab answered"
    );
    assert_eq!(
        sim.tool_answer(arrived).map(|(status, _)| *status),
        Some(200)
    );
    assert!(
        closes_saying_installed_on(sim.answer_text(arrived).expect("a page"), "acme"),
        "{:?}",
        sim.answer_text(arrived)
    );
    let kept = sim.disk().expect("landed");
    assert_eq!(
        kept.apps
            .get(&Platform::GitHub)
            .and_then(|app| app.installations.get(&77)),
        Some(&stageman_core::Installation {
            account: "acme".to_owned(),
            every_repository: false,
        })
    );
    assert_eq!(
        kept.projects
            .get(&project())
            .and_then(|watched| watched.access.get(&Platform::GitHub)),
        None,
        "the project holds what it held"
    );

    let page = instance_page(&mut sim, &mut instance, 3);
    let app = page.github.expect("the App");
    assert_eq!(
        app.installations,
        [InstallationView {
            id: 77,
            account: "acme".to_owned(),
            every_repository: false,
            used_by: Vec::new(),
        }]
    );
    assert_eq!(app.install_failure, None);
}

/// Only the last few installs begun are remembered, one press each: the
/// seventeenth press forgets the first and only the first, so a tab coming
/// back under the first's state finds no page waiting and stays open,
/// while one under the second's is announced and closes.
#[test]
fn only_the_last_few_installs_begun_are_remembered() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let remembered = 16;
    let states: Vec<String> = (1..=remembered + 1)
        .map(|id| pressed(&mut sim, &mut instance, id).state)
        .collect();

    let page = arrives_installed(&mut sim, &mut instance, 77, Some(&states[0]));
    assert!(
        page.contains("opened before stageman last started") && !page.contains("window.close()"),
        "the oldest state has been forgotten: {page}"
    );
    let page = arrives_installed(&mut sim, &mut instance, 78, Some(&states[1]));
    assert!(
        closes_saying_installed_on(&page, "example"),
        "the next-oldest is still remembered: {page}"
    );
}

/// What came back under a state is listed for the form that holds the
/// state — a token minted for that installation for listing, then its
/// repositories, with the account — and for no other form: another
/// installation of the App is not in it, a state nothing has come back
/// under yet is answered so, and a state this instance never minted is
/// answered with why. Asked again within the hour, nothing is minted; and
/// nothing of a listing is kept.
#[test]
fn what_came_back_under_a_state_is_listed_for_the_form_that_holds_it() {
    let mut sim = Simulation::new();
    let mut state = app_and_project("example/repo");
    app_installed(&mut state, 78, "example");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.installed_on(77, "acme");
    sim.installation_covers(77, &[("acme/b", false), ("acme/a", true)]);
    sim.installation_covers(78, &[("example/a", true)]);

    let minted = pressed(&mut sim, &mut instance, 1);
    assert_eq!(
        reaches(&mut sim, &mut instance, 2, arrived(&minted.state)),
        Response::Reached(Reached::NotYet),
        "the tab is still on the platform"
    );
    assert!(calls(&sim).is_empty(), "nothing asked for nothing");

    let page = arrives_installed(&mut sim, &mut instance, 77, Some(&minted.state));
    assert!(closes_saying_installed_on(&page, "acme"), "{page}");
    let writes_before = count(&sim, "-> Write");

    let answered = reaches(&mut sim, &mut instance, 3, arrived(&minted.state));
    assert_eq!(
        answered,
        Response::Reached(Reached::Listed {
            account: Some("acme".to_owned()),
            expires: None,
            repositories: vec![row("acme/a", true), row("acme/b", false)],
            more: false,
        }),
        "that installation, and not the App's other one"
    );
    let made = calls(&sim);
    assert_eq!(
        made.iter()
            .filter(|call| matches!(call, PlatformCall::Mint { id: 77, repositories, .. } if repositories.is_empty()))
            .count(),
        1,
        "a listing token for the one installation, restricted to nothing: {made:?}"
    );
    assert!(
        !made
            .iter()
            .any(|call| matches!(call, PlatformCall::Mint { id: 78, .. })),
        "the App's other installation is nobody's business here: {made:?}"
    );
    assert_eq!(count(&sim, "-> Write"), writes_before, "nothing kept");

    let again = reaches(&mut sim, &mut instance, 4, arrived(&minted.state));
    assert_eq!(again, answered);
    assert_eq!(
        sim.tokens_minted(),
        1,
        "the listing token is held through its hour"
    );

    assert_eq!(
        reaches(&mut sim, &mut instance, 5, arrived("not-minted-here")),
        Response::Reached(Reached::Unlisted {
            why: "no installation has come back for this form, or it came back before the \
                  instance last started: press Install the App again"
                .to_owned()
        }),
        "a state nobody here minted: the form's cue to stop waiting"
    );
}

/// An installation the platform does not know under the App's key, or one
/// naming another App, is refused: nothing is kept, the tab stays open
/// saying why, and the Instance page says it too until the next one is
/// kept.
#[test]
fn a_foreign_installation_is_refused_and_said_where_it_came_back() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let writes_before = count(&sim, "-> Write");

    sim.next_platform_answers(404, r#"{"message":"Not Found"}"#);
    let page = arrives_installed(&mut sim, &mut instance, 99, None);
    assert!(
        stays_saying(
            &page,
            "GitHub knows no installation with that identifier for this App"
        ),
        "{page}"
    );
    assert_eq!(count(&sim, "-> Write"), writes_before, "nothing kept");
    let shown = instance_page(&mut sim, &mut instance, 1);
    let app = shown.github.expect("the App");
    assert!(app.installations.is_empty());
    assert_eq!(
        app.install_failure.as_deref(),
        Some("GitHub knows no installation with that identifier for this App")
    );

    sim.next_platform_answers(
        200,
        r#"{"id":99,"app_id":9,"account":{"login":"example"},"repository_selection":"all"}"#,
    );
    let page = arrives_installed(&mut sim, &mut instance, 99, None);
    assert!(
        stays_saying(&page, "that installation belongs to another App on GitHub"),
        "{page}"
    );
    let shown = instance_page(&mut sim, &mut instance, 2);
    assert_eq!(
        shown.github.and_then(|app| app.install_failure).as_deref(),
        Some("that installation belongs to another App on GitHub")
    );

    // The next one kept clears what the last one said.
    let page = arrives_installed(&mut sim, &mut instance, 77, None);
    assert!(closes_saying_installed_on(&page, "example"), "{page}");
    let shown = instance_page(&mut sim, &mut instance, 3);
    let app = shown.github.expect("the App");
    assert_eq!(app.install_failure, None);
    assert_eq!(app.installations.len(), 1);
}

/// A browser coming back to the installation's path naming no installation
/// is refused without asking the platform; one naming an installation
/// while no App is registered is answered with the failure said.
#[test]
fn an_arrival_naming_no_installation_is_refused_without_asking() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));

    let nameless = sim.visits_path(
        sim.now(),
        "/instance/apps/github/installed?setup_action=install",
    );
    sim.run_until(&mut instance, 5_000);
    assert_eq!(
        sim.tool_answer(nameless).map(|(status, _)| *status),
        Some(400)
    );
    assert!(calls(&sim).is_empty(), "nothing asked");

    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let page = arrives_installed(&mut sim, &mut instance, 77, None);
    assert!(
        stays_saying(&page, "no App is registered on this instance"),
        "{page}"
    );
    assert!(calls(&sim).is_empty(), "no key to ask with");
    assert!(instance.state().apps.is_empty());
}

/// The platform sends the browser back on an installation's update too,
/// from its own settings page and so under no state: the record is
/// refreshed, the tab is answered as kept, and the tokens minted on the
/// installation for projects' jobs are dropped, so the next command mints
/// anew and fails loudly if what it needs was removed.
#[test]
fn an_update_refreshes_the_installation_and_drops_the_tokens_minted_on_it() {
    let mut sim = Simulation::new();
    let working = job(1);
    let mut state = watching(&[(working.clone(), Progress::Working)]);
    with_an_app(&mut state);
    installed(&mut state, 77);
    on_github(&mut state, "example/repo");
    sim.holding(&state);
    let (name, held) = Simulation::ours(&stageman_job::container(&working));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);
    let warrant = warrant_of(&working);
    let fetching = |sim: &mut Simulation, at: u64| {
        sim.arrives(
            at,
            "GET",
            "/credential",
            &[("authorization", &format!("Bearer {warrant}"))],
            NEARBY,
            "",
        )
    };
    let before = fetching(&mut sim, 20);
    sim.run_until(&mut instance, 40);
    assert_eq!(sim.answer_text(before), Some("ghs_sim_1"));

    sim.next_platform_answers(
        200,
        r#"{"id":77,"app_id":4242,"account":{"login":"example"},"repository_selection":"all"}"#,
    );
    let updated = sim.visits_path(sim.now(), &back(77, true, None));
    sim.run_until(&mut instance, 100);
    assert_eq!(
        sim.tool_answer(updated).map(|(status, _)| *status),
        Some(200)
    );
    assert!(
        closes_saying_installed_on(sim.answer_text(updated).expect("a page"), "example"),
        "{:?}",
        sim.answer_text(updated)
    );
    assert_eq!(
        sim.disk()
            .expect("landed")
            .apps
            .get(&Platform::GitHub)
            .and_then(|app| app.installations.get(&77))
            .map(|installation| installation.every_repository),
        Some(true),
        "refreshed"
    );

    let after = fetching(&mut sim, 200);
    sim.run_until(&mut instance, 300);
    assert_eq!(sim.answer_text(after), Some("ghs_sim_2"), "minted anew");
    assert_eq!(sim.tokens_minted(), 2);
}

/// An installation coming back under a state this instance did not mint
/// is kept all the same — the key is the check, and the state is no part
/// of it — but no page here is waiting on it, so the tab stays open saying
/// what to do; and the state buys nothing afterwards.
#[test]
fn an_arrival_under_a_state_nobody_here_minted_is_kept_and_the_tab_stays() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));

    let page = arrives_installed(&mut sim, &mut instance, 77, Some("f00d"));
    assert!(
        page.contains(
            "The App is installed on <b>example</b>, but this tab was opened before stageman \
             last started, so the page you left will not learn of it."
        ),
        "{page}"
    );
    assert!(page.contains("press Install the App again there"), "{page}");
    assert!(!page.contains("window.close()"), "stays: {page}");
    assert!(
        sim.disk()
            .expect("landed")
            .apps
            .get(&Platform::GitHub)
            .expect("the App")
            .installations
            .contains_key(&77),
        "kept all the same"
    );
    assert!(matches!(
        reaches(&mut sim, &mut instance, 1, arrived("f00d")),
        Response::Reached(Reached::Unlisted { .. })
    ));
}

/// What a project holds is listed for its own form: the one installation
/// it is on, with its account, and not the App's others; the token it
/// holds, read with it; and nothing where it holds nothing.
#[test]
fn what_a_project_holds_is_listed_for_its_own_form() {
    let mut sim = Simulation::new();
    let mut state = app_and_project("example/repo");
    installed(&mut state, 77);
    app_installed(&mut state, 78, "acme");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.installation_covers(77, &[("example/a", true)]);
    sim.installation_covers(78, &[("acme/b", false)]);
    let held = || Through::Held {
        project: project().to_string(),
    };

    assert_eq!(
        reaches(&mut sim, &mut instance, 1, held()),
        Response::Reached(Reached::Listed {
            account: Some("example".to_owned()),
            expires: None,
            repositories: vec![row("example/a", true)],
            more: false,
        })
    );
    assert!(
        !calls(&sim)
            .iter()
            .any(|call| matches!(call, PlatformCall::Mint { id: 78, .. })),
        "the other installation is not the project's"
    );

    let mut sim = Simulation::new();
    let mut state = watching(&[]);
    crate::simulation::holding_a_token(&mut state, "ghp-the-held-one");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    let listed = reaches(&mut sim, &mut instance, 1, held());
    assert!(
        matches!(
            &listed,
            Response::Reached(Reached::Listed { account: Some(login), expires: None, .. })
                if login == "example"
        ),
        "the account a token was made under, asked with it: {listed:?}"
    );
    let read = sim
        .trace()
        .iter()
        .find(|line| line.contains("api.github.com/user/repos"))
        .expect("the listing is in the trace");
    assert!(read.contains("Bearer ghp-the-held-one"), "{read}");

    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    assert_eq!(
        reaches(&mut sim, &mut instance, 1, held()),
        Response::Reached(Reached::default())
    );
    assert!(calls(&sim).is_empty());
}

/// A form asking what a token can read is answered with what the platform
/// lists for it, each row saying whether it is private, with whose the
/// token is and when it expires, asked with it in the same breath; a
/// token the platform does not accept, or a platform that cannot be
/// asked, is answered with why rather than refused — the question was
/// asked and that is its answer — and an empty token is not asked about.
#[test]
fn what_a_token_can_read_is_listed_and_a_bad_token_answered_with_why() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    sim.token_reads(&[("example/private", true), ("example/public", false)]);
    sim.token_owned_by("somebody");
    sim.token_expires(Some("2026-09-27 08:42:20 UTC"));

    let asked = |sim: &mut Simulation, instance: &mut Instance, id: u64, token: &str| {
        reaches(
            sim,
            instance,
            id,
            Through::Token {
                token: token.to_owned(),
            },
        )
    };
    assert_eq!(
        asked(&mut sim, &mut instance, 1, "ghp-not-a-real-token"),
        Response::Reached(Reached::Listed {
            account: Some("somebody".to_owned()),
            expires: Some("2026-09-27T08:42:20Z".to_owned()),
            repositories: vec![row("example/private", true), row("example/public", false)],
            more: false,
        })
    );
    assert_eq!(
        calls(&sim),
        [
            PlatformCall::Readable {
                platform: Platform::GitHub
            },
            PlatformCall::Owner {
                platform: Platform::GitHub
            }
        ]
    );
    let read = sim
        .trace()
        .iter()
        .find(|line| line.contains("api.github.com/user/repos"))
        .expect("the listing is in the trace");
    assert!(read.contains("Bearer ghp-not-a-real-token"), "{read}");

    sim.next_platform_answers(401, r#"{"message":"Bad credentials"}"#);
    assert_eq!(
        asked(&mut sim, &mut instance, 2, "ghp-not-a-real-token"),
        Response::Reached(Reached::Unlisted {
            why: "GitHub does not accept it".to_owned()
        })
    );
    sim.next_platform_fails("dns error");
    assert_eq!(
        asked(&mut sim, &mut instance, 3, "ghp-not-a-real-token"),
        Response::Reached(Reached::Unlisted {
            why: "GitHub could not be reached: dns error".to_owned()
        })
    );
    assert_eq!(
        asked(&mut sim, &mut instance, 4, "  "),
        Response::Refused(Refusal::Incomplete {
            field: "access".to_owned()
        })
    );
    assert_eq!(
        calls(&sim).len(),
        6,
        "two reads per token, and an empty token is not asked about"
    );
}

/// A project created on the installation its tab brought back has its
/// coverage checked before anything is kept, by a token minted restricted
/// to the repository: kept where the platform mints, with the state spent
/// so that it buys nothing more; refused on the repository where the
/// platform will not, with the state kept for another choice; and refused
/// before asking where the state is nobody's.
#[test]
fn a_project_is_created_on_the_installation_its_tab_brought_back_and_checked() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    sim.installation_covers(77, &[("example/aviary", true)]);
    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    let writes_before = count(&sim, "-> Write");
    let calls_before = calls(&sim).len();

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Create {
                draft: on_the_app("other", Some(&state), "example/other")
            }
        ),
        Response::Refused(Refusal::NotReached {
            repository: named("example/other"),
            why: "GitHub refused: There is at least one repository that does not exist or is \
                  not accessible to the parent installation."
                .to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing kept");

    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Create {
            draft: on_the_app("aviary", Some(&state), "example/aviary"),
        },
    ) else {
        panic!("created");
    };
    let minted = sim
        .platform_calls()
        .iter()
        .skip(calls_before)
        .find(|(_, call)| {
            matches!(call, PlatformCall::Mint { id: 77, repositories, .. } if repositories == &["aviary".to_owned()])
        })
        .map(|(at, _)| *at)
        .expect("minted for the one repository");
    assert!(
        minted < nth(&sim, "-> Write", writes_before),
        "checked before written"
    );
    let created = shown
        .projects
        .iter()
        .find(|listed| listed.name == "aviary")
        .expect("the new project");
    assert_eq!(
        created.access,
        Some(AccessView::Installation {
            account: "example".to_owned()
        })
    );
    assert_eq!(created.repository, named("example/aviary"));
    assert_eq!(created.repository_link, "https://github.com/example/aviary");
    assert_eq!(
        instance_page(&mut sim, &mut instance, 3)
            .github
            .and_then(|app| app.installations.first().map(|i| i.used_by.clone())),
        Some(vec!["aviary".to_owned()])
    );

    let calls_before = calls(&sim).len();
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            4,
            Request::Create {
                draft: on_the_app("again", Some(&state), "example/aviary")
            }
        ),
        Response::Refused(Refusal::ArrivalUnknown),
        "spent by the save"
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            5,
            Request::Create {
                draft: on_the_app("unknown", Some("not-minted-here"), "example/aviary")
            }
        ),
        Response::Refused(Refusal::ArrivalUnknown)
    );
    assert_eq!(calls(&sim).len(), calls_before, "refused before asking");
}

/// An installation left as it is is checked again by a restricted mint
/// when the repository moves, and not when it does not; and a token set
/// replaces it.
#[test]
fn a_kept_installation_is_checked_again_when_the_repository_moves() {
    let mut sim = Simulation::new();
    let mut state = app_and_project("example/repo");
    installed(&mut state, 77);
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.installation_covers(77, &[("example/repo", true), ("example/other", true)]);
    let id = project().to_string();

    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Amend {
            project: id.clone(),
            draft: on_the_app("example", None, "example/repo"),
        },
    ) else {
        panic!("amended");
    };
    assert!(calls(&sim).is_empty(), "nothing moved, so nothing is asked");

    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Amend {
            project: id.clone(),
            draft: on_the_app("example", None, "example/other"),
        },
    ) else {
        panic!("amended");
    };
    assert!(
        matches!(
            calls(&sim).as_slice(),
            [PlatformCall::Mint { id: 77, repositories, .. }] if repositories == &["other".to_owned()]
        ),
        "{:?}",
        calls(&sim)
    );
    assert_eq!(the_project(&shown).repository, named("example/other"));

    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        3,
        Request::Amend {
            project: id,
            draft: with_a_token("example", Some("ghp-not-a-real-token"), "example/other"),
        },
    ) else {
        panic!("amended");
    };
    assert_eq!(
        the_project(&shown).access,
        Some(AccessView::Token {
            owner: Some("example".to_owned()),
            expires: None,
            expired: false,
        })
    );
    assert!(matches!(
        sim.disk()
            .expect("landed")
            .projects
            .get(&project())
            .and_then(|watched| watched.access.get(&Platform::GitHub)),
        Some(Access::Token { secret, .. }) if secret.expose() == "ghp-not-a-real-token"
    ));
}

/// A token left unsaid is read against the repository when that moves,
/// and an installation come back under the form's own state replaces it.
#[test]
fn a_kept_token_is_read_against_a_moved_repository_and_an_installation_replaces_it() {
    let mut sim = Simulation::new();
    let mut state = app_and_project("example/other");
    crate::simulation::holding_a_token(&mut state, "ghp-not-a-real-token");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.installation_covers(77, &[("example/repo", true)]);
    let id = project().to_string();

    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        4,
        Request::Amend {
            project: id.clone(),
            draft: with_a_token("example", None, "example/third"),
        },
    ) else {
        panic!("amended");
    };
    assert_eq!(
        calls(&sim)
            .iter()
            .filter(|call| matches!(
                call,
                PlatformCall::Repository { name, .. } if name == "third"
            ))
            .count(),
        1,
        "read once, with the token held"
    );
    let read = sim
        .trace()
        .iter()
        .find(|line| line.contains("api.github.com/repos/example/third"))
        .expect("the read is in the trace");
    assert!(read.contains("Bearer ghp-not-a-real-token"), "{read}");

    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        5,
        Request::Amend {
            project: id,
            draft: on_the_app("example", Some(&state), "example/repo"),
        },
    ) else {
        panic!("amended");
    };
    assert_eq!(
        the_project(&shown).access,
        Some(AccessView::Installation {
            account: "example".to_owned()
        })
    );
}

/// A draft is refused before any platform is asked when it says nothing
/// about how the repository is reached — nothing chosen, or an access left
/// unsaid where there is none to hold — or names no repository in the
/// shape it chose.
#[test]
fn the_access_and_the_repository_are_required() {
    let mut sim = Simulation::new();
    let mut state = watching(&[]);
    with_an_app(&mut state);
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    let calls_before = calls(&sim).len();

    for access in [
        AccessDraft::None,
        AccessDraft::Token {
            token: None,
            repository: Some(named("example/aviary")),
        },
        AccessDraft::App {
            arrival: None,
            repository: Some(named("example/aviary")),
        },
    ] {
        let mut draft = a_draft("aviary");
        draft.access = access;
        assert_eq!(
            ask(&mut sim, &mut instance, 1, Request::Create { draft }),
            Response::Refused(Refusal::Incomplete {
                field: "access".to_owned()
            })
        );
    }
    for access in [
        AccessDraft::App {
            arrival: Some(state),
            repository: None,
        },
        AccessDraft::Token {
            token: Some("ghp-not-a-real-token".to_owned()),
            repository: None,
        },
    ] {
        let mut blank = a_draft("aviary");
        blank.access = access;
        assert_eq!(
            ask(&mut sim, &mut instance, 2, Request::Create { draft: blank }),
            Response::Refused(Refusal::Incomplete {
                field: "repository".to_owned()
            })
        );
    }
    assert_eq!(calls(&sim).len(), calls_before, "nothing asked");
    assert_eq!(instance.state().projects.len(), 1, "nothing kept");
}

/// An installation cannot be forgotten while a project reaches its
/// repository through it, and the refusal names the project; one nothing
/// names is forgotten, and the Instance page no longer lists it; and one
/// the App is not installed as is refused as such.
#[test]
fn an_installation_in_use_is_refused_forgetting_and_a_free_one_is_forgotten() {
    let mut sim = Simulation::new();
    let mut state = app_and_project("example/repo");
    installed(&mut state, 77);
    app_installed(&mut state, 78, "acme");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    let forget = |id: u64| Request::ForgetInstallation {
        platform: "github".to_owned(),
        id,
    };

    assert_eq!(
        ask(&mut sim, &mut instance, 1, forget(77)),
        Response::Refused(Refusal::InstallationInUse {
            projects: vec!["example".to_owned()]
        })
    );
    let Response::Apps(page) = ask(&mut sim, &mut instance, 2, forget(78)) else {
        panic!("forgotten");
    };
    assert_eq!(
        page.github
            .map(|app| app.installations.iter().map(|i| i.id).collect::<Vec<_>>()),
        Some(vec![77])
    );
    assert!(
        !sim.disk()
            .expect("landed")
            .apps
            .get(&Platform::GitHub)
            .expect("the App")
            .installations
            .contains_key(&78)
    );
    assert_eq!(
        ask(&mut sim, &mut instance, 3, forget(99)),
        Response::Refused(Refusal::NoSuchInstallation { id: 99 })
    );
}

/// A job whose project reaches the platform through an installation is
/// handed a token minted for the project's one repository, the same token
/// for as long as it is good, and a new one once its hour is nearly up;
/// two wrappers asking while one is minted share it.
#[test]
fn a_token_is_minted_once_for_a_projects_jobs_and_served_through_its_hour() {
    let mut sim = Simulation::new();
    let working = job(1);
    let mut state = watching(&[(working.clone(), Progress::Working)]);
    with_an_app(&mut state);
    installed(&mut state, 77);
    on_github(&mut state, "example/repo");
    sim.holding(&state);
    let (name, held) = Simulation::ours(&stageman_job::container(&working));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);
    let warrant = warrant_of(&working);
    let fetching = |sim: &mut Simulation, at: u64| {
        sim.arrives(
            at,
            "GET",
            "/credential",
            &[("authorization", &format!("Bearer {warrant}"))],
            NEARBY,
            "",
        )
    };

    let first_ask = fetching(&mut sim, 20);
    let at_once = fetching(&mut sim, 20);
    sim.run_until(&mut instance, 40);
    assert_eq!(sim.tool_answer(first_ask).map(|a| a.0), Some(200));
    assert_eq!(sim.answer_text(first_ask), Some("ghs_sim_1"));
    assert_eq!(sim.answer_text(at_once), Some("ghs_sim_1"), "shared");
    assert_eq!(sim.tokens_minted(), 1);
    assert!(
        calls(&sim).iter().any(|call| matches!(
            call,
            PlatformCall::Mint { id: 77, repositories, .. } if repositories == &["repo".to_owned()]
        )),
        "minted for the one repository: {:?}",
        calls(&sim)
    );

    let later = fetching(&mut sim, 1_000);
    sim.run_until(&mut instance, 1_100);
    assert_eq!(sim.answer_text(later), Some("ghs_sim_1"), "still good");
    assert_eq!(sim.tokens_minted(), 1);

    // Fifty-six minutes on, the hour is within its margin: minted anew.
    let nearly_up = sim.now() + 56 * 60 * 1_000;
    let renewed = fetching(&mut sim, nearly_up);
    sim.run_until(&mut instance, nearly_up + 100);
    assert_eq!(sim.answer_text(renewed), Some("ghs_sim_2"));
    assert_eq!(sim.tokens_minted(), 2);
}

/// A platform that will not mint fails the command loudly rather than
/// handing it nothing: the wrapper is told why, and nothing is kept.
#[test]
fn a_token_the_platform_will_not_mint_fails_the_command_loudly() {
    let mut sim = Simulation::new();
    let working = job(1);
    let mut state = watching(&[(working.clone(), Progress::Working)]);
    with_an_app(&mut state);
    installed(&mut state, 77);
    on_github(&mut state, "example/repo");
    sim.holding(&state);
    let (name, held) = Simulation::ours(&stageman_job::container(&working));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);

    sim.next_platform_answers(
        422,
        r#"{"message":"There is at least one repository that does not exist or is not accessible to the parent installation."}"#,
    );
    let asked = sim.arrives(
        20,
        "GET",
        "/credential",
        &[("authorization", &format!("Bearer {}", warrant_of(&working)))],
        NEARBY,
        "",
    );
    sim.run_until(&mut instance, 40);
    assert_eq!(sim.tool_answer(asked).map(|a| a.0), Some(502));
    assert_eq!(
        sim.answer_text(asked),
        Some(
            "a token could not be minted for this job: GitHub refused: There is at least one \
             repository that does not exist or is not accessible to the parent installation."
        )
    );
}

/// The App cannot be forgotten while a project reaches its repository
/// through it, and the refusal names the project.
#[test]
fn the_app_is_refused_forgetting_while_a_project_is_installed_on_through_it() {
    let mut sim = Simulation::new();
    let mut state = app_and_project("example/repo");
    installed(&mut state, 77);
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));

    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::ForgetApp {
                platform: "github".to_owned()
            },
        ),
        Response::Refused(Refusal::AppInUse {
            platform: "GitHub".to_owned(),
            projects: vec!["example".to_owned()],
        })
    );
    assert!(
        sim.disk()
            .expect("landed")
            .apps
            .contains_key(&Platform::GitHub)
    );
}

/// A daemon dying between the browser's return and the platform's answer
/// keeps nothing and answers nobody. The key being the check, the same
/// arrival after the restart is kept; but the state was minted before the
/// restart and is held, not kept, so the tab stays open saying so, and the
/// form holding the state is told to press again.
#[test]
fn a_daemon_dying_mid_installation_keeps_nothing_and_forgets_the_states_minted_before() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let minted = pressed(&mut sim, &mut instance, 1);

    let arrived_at = sim.visits_path(sim.now(), &back(77, false, Some(&minted.state)));
    let Some(event) = sim.next() else {
        panic!("the arrival is queued");
    };
    for effect in instance.step(sim.now(), event) {
        sim.perform(effect);
    }
    assert_eq!(
        calls(&sim).len(),
        1,
        "the installation went out to be fetched"
    );

    let mut instance = sim.crash(seed(1));
    sim.run_until(&mut instance, 5_000);
    assert!(sim.tool_answer(arrived_at).is_none(), "answered to nobody");
    assert!(
        sim.disk()
            .expect("landed")
            .apps
            .get(&Platform::GitHub)
            .expect("the App")
            .installations
            .is_empty(),
        "nothing kept"
    );

    let page = arrives_installed(&mut sim, &mut instance, 77, Some(&minted.state));
    assert!(
        page.contains("opened before stageman last started") && !page.contains("window.close()"),
        "{page}"
    );
    assert!(
        sim.disk()
            .expect("landed")
            .apps
            .get(&Platform::GitHub)
            .expect("the App")
            .installations
            .contains_key(&77),
        "accepted after the restart, the key being the check"
    );
    assert!(matches!(
        reaches(&mut sim, &mut instance, 2, arrived(&minted.state)),
        Response::Reached(Reached::Unlisted { .. })
    ));
}

/// The platform's own trouble on the restricted mint that checks an
/// installation's coverage refuses the installation as unchecked — not
/// wrong, and not known to be right — rather than as refused.
#[test]
fn a_platform_in_trouble_on_the_coverage_check_refuses_the_installation_as_unchecked() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    sim.next_platform_answers(503, r#"{"message":"down"}"#);
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Create {
                draft: on_the_app("aviary", Some(&state), "example/aviary")
            }
        ),
        Response::Refused(Refusal::InstallationUnchecked {
            why: "GitHub could not be reached: it answered 503".to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing kept");
}

/// Forgetting the App forgets everything held for its installations: the
/// listing token minted for one is not served to the App registered next,
/// which mints its own.
#[test]
fn forgetting_the_app_forgets_the_tokens_held_for_its_installations() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    assert!(matches!(
        reaches(&mut sim, &mut instance, 1, arrived(&state)),
        Response::Reached(Reached::Listed { .. })
    ));
    assert_eq!(
        sim.tokens_minted(),
        1,
        "a listing token for the installation"
    );

    let Response::Apps(page) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::ForgetApp {
            platform: "github".to_owned(),
        },
    ) else {
        panic!("forgotten");
    };
    assert!(page.github.is_none());

    // Registered again, from the page, and installed again: what the
    // forgotten App held for the installation is not this App's.
    let Response::Registration(form) = ask(
        &mut sim,
        &mut instance,
        3,
        Request::Registration {
            platform: "github".to_owned(),
            anywhere: false,
        },
    ) else {
        panic!("a form");
    };
    sim.visits_path(
        sim.now(),
        &format!(
            "/instance/apps/github/registered?code=c0de&state={}",
            form.state
        ),
    );
    sim.run_until(&mut instance, sim.now() + 5_000);
    assert!(instance.state().apps.contains_key(&Platform::GitHub));
    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    assert!(matches!(
        reaches(&mut sim, &mut instance, 4, arrived(&state)),
        Response::Reached(Reached::Listed { .. })
    ));
    assert_eq!(
        sim.tokens_minted(),
        2,
        "minted anew, nothing having been kept"
    );
}

/// A project's access amended, or its repository moved, drops the token
/// minted for its jobs: the next command mints one for what the project
/// holds now.
#[test]
fn an_amended_access_drops_the_token_minted_for_the_projects_jobs() {
    let mut sim = Simulation::new();
    let working = job(1);
    let mut state = watching(&[(working.clone(), Progress::Working)]);
    with_an_app(&mut state);
    installed(&mut state, 77);
    on_github(&mut state, "example/repo");
    sim.holding(&state);
    let (name, held) = Simulation::ours(&stageman_job::container(&working));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 10);
    sim.installation_covers(77, &[("example/repo", true), ("example/other", true)]);
    let warrant = warrant_of(&working);
    let fetching = |sim: &mut Simulation, at: u64| {
        sim.arrives(
            at,
            "GET",
            "/credential",
            &[("authorization", &format!("Bearer {warrant}"))],
            NEARBY,
            "",
        )
    };
    let before = fetching(&mut sim, 20);
    sim.run_until(&mut instance, 40);
    assert_eq!(sim.answer_text(before), Some("ghs_sim_1"));

    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Amend {
            project: project().to_string(),
            draft: on_the_app("example", None, "example/other"),
        },
    ) else {
        panic!("amended");
    };
    let minted_by_the_check = sim.tokens_minted();
    let later = sim.now() + 100;
    let after = fetching(&mut sim, later);
    sim.run_until(&mut instance, later + 100);
    assert_eq!(
        sim.tokens_minted(),
        minted_by_the_check + 1,
        "minted anew for the repository the project holds now"
    );
    assert_ne!(sim.answer_text(after), Some("ghs_sim_1"));
}

/// A listing the platform had more of than one page holds says so: a
/// token's page that is full, and an installation whose count exceeds
/// the page it listed.
#[test]
fn a_listing_says_when_the_platform_had_more_than_a_page() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let names: Vec<String> = (1..=101).map(|n| format!("example/r{n}")).collect();
    let page: Vec<(&str, bool)> = names.iter().map(|name| (name.as_str(), false)).collect();
    sim.token_reads(&page);
    let listed = reaches(
        &mut sim,
        &mut instance,
        1,
        Through::Token {
            token: "ghp-not-a-real-token".to_owned(),
        },
    );
    assert!(
        matches!(
            &listed,
            Response::Reached(Reached::Listed { more: true, repositories, .. }) if repositories.len() == 100
        ),
        "{listed:?}"
    );

    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    let state = installed_from_a_page(&mut sim, &mut instance, 77);
    sim.installation_covers(77, &page);
    let listed = reaches(&mut sim, &mut instance, 1, arrived(&state));
    assert!(
        matches!(
            &listed,
            Response::Reached(Reached::Listed { more: true, repositories, .. }) if repositories.len() == 100
        ),
        "{listed:?}"
    );
}

/// Forgetting the App forgets what was held about its installations: the
/// last installation's failure is not said of the App registered next.
#[test]
fn forgetting_the_app_forgets_the_last_installations_failure() {
    let mut sim = Simulation::new();
    sim.holding(&app_and_project("example/repo"));
    let mut instance = sim.wake(seed(1));
    sim.next_platform_answers(404, r#"{"message":"Not Found"}"#);
    let page = arrives_installed(&mut sim, &mut instance, 99, None);
    assert!(stays_saying(
        &page,
        "GitHub knows no installation with that identifier for this App"
    ));
    assert!(
        instance_page(&mut sim, &mut instance, 1)
            .github
            .and_then(|app| app.install_failure)
            .is_some(),
        "said on the page while the App stands"
    );

    let Response::Apps(page) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::ForgetApp {
            platform: "github".to_owned(),
        },
    ) else {
        panic!("forgotten");
    };
    assert!(page.github.is_none());
    let Response::Registration(form) = ask(
        &mut sim,
        &mut instance,
        3,
        Request::Registration {
            platform: "github".to_owned(),
            anywhere: false,
        },
    ) else {
        panic!("a form");
    };
    sim.visits_path(
        sim.now(),
        &format!(
            "/instance/apps/github/registered?code=c0de&state={}",
            form.state
        ),
    );
    sim.run_until(&mut instance, sim.now() + 5_000);
    let registered = instance_page(&mut sim, &mut instance, 4)
        .github
        .expect("the App registered again");
    assert_eq!(
        registered.install_failure, None,
        "nothing of the forgotten App's is said of this one"
    );
}
