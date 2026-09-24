//! A credential checked against its platform before it is kept: the
//! request held while the platforms answer, refused with the platform's
//! reason on the box the credential was typed in, kept once every check has
//! passed, and answered to nobody when the daemon dies mid-check. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.

use stageman_channel::Call;
use stageman_core::{Platform, Secret};
use stageman_instance::{Request, Response};
use stageman_platform::Call as PlatformCall;
use stageman_wire::Refusal;

use crate::dashboard::{a_draft, ask, count, first, nth};
use crate::simulation::{Simulation, project, request, seed, watching};

/// How many reads of a repository were asked for.
fn reads(sim: &Simulation) -> usize {
    sim.platform_calls()
        .iter()
        .filter(|(_, call)| matches!(call, PlatformCall::Repository { .. }))
        .count()
}

/// A token the platform does not accept is refused on the token's box,
/// with the platform's clause, and nothing is written or listened to.
#[test]
fn a_token_the_platform_refuses_is_not_kept() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let written = count(&sim, "-> Write");

    sim.next_platform_answers(401, r#"{"message":"Bad credentials"}"#);
    let answered = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: a_draft("burrow"),
        },
    );
    assert_eq!(
        answered,
        Response::Refused(Refusal::TokenRefused {
            why: "GitHub does not accept it".to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing was kept");
    assert_eq!(count(&sim, "-> Write"), written, "nothing was written");
    assert_eq!(sim.listening(), 0, "nothing listens for a project not made");
    // The read was of the repository the form named, with the token.
    assert!(
        sim.platform_calls().iter().any(|(_, call)| matches!(
            call,
            PlatformCall::Repository { platform: Platform::GitHub, owner, name }
                if owner == "example" && name == "burrow"
        )),
        "{:?}",
        sim.platform_calls()
    );
    let read = sim
        .trace()
        .iter()
        .find(|line| line.contains("api.github.com/repos/example/burrow"))
        .expect("the read is in the trace");
    assert!(read.contains("Bearer ghp-not-a-real-token"), "{read}");
}

/// A private repository the token was not granted is named in the refusal,
/// so the operator knows which grant is missing.
#[test]
fn a_repository_the_token_cannot_see_is_named_in_the_refusal() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    sim.next_platform_answers(404, r#"{"message":"Not Found"}"#);
    let answered = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: a_draft("burrow"),
        },
    );
    assert_eq!(
        answered,
        Response::Refused(Refusal::TokenRefused {
            why: "GitHub cannot see example/burrow with it — a fine-grained token has to be \
                  granted that repository"
                .to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing was kept");
}

/// A platform that cannot be reached keeps nothing either, and says the
/// token could not be checked rather than that it was wrong.
#[test]
fn a_platform_that_cannot_be_reached_keeps_nothing_and_says_so() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let written = count(&sim, "-> Write");

    sim.next_platform_fails("dns error");
    let answered = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: a_draft("burrow"),
        },
    );
    assert_eq!(
        answered,
        Response::Refused(Refusal::TokenUnchecked {
            why: "GitHub could not be reached: dns error".to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing was kept");
    assert_eq!(count(&sim, "-> Write"), written, "nothing was written");
}

/// Each of the binding's credentials is refused on its own box: the bot
/// token where the channel does not know it, the app-level token where
/// the channel will not open a stream for it.
#[test]
fn a_bindings_credentials_are_refused_on_their_own_boxes() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    sim.next_listen_fails("invalid_auth");
    let answered = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: a_draft("burrow"),
        },
    );
    assert_eq!(
        answered,
        Response::Refused(Refusal::ChannelRefused {
            listening: false,
            why: "Slack refused it (invalid_auth)".to_owned()
        })
    );

    sim.next_locate_fails("not_allowed_token_type");
    let answered = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Create {
            draft: a_draft("burrow"),
        },
    );
    assert_eq!(
        answered,
        Response::Refused(Refusal::ChannelRefused {
            listening: true,
            why: "Slack refused it (not_allowed_token_type)".to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing was kept");
    assert_eq!(sim.listening(), 0);
    // Every credential was asked about each time, and the first refusal
    // answered the request whatever the rest said.
    assert_eq!(reads(&sim), 2);
    assert_eq!(
        sim.channel_calls()
            .iter()
            .filter(|(_, call)| matches!(call, Call::WhoAmI { .. } | Call::OpenSocket { .. }))
            .count(),
        4
    );
}

/// The checks go out before anything is written, together, and the answer
/// follows the write that a passed check allows.
#[test]
fn the_checks_come_before_the_write_and_the_answer_after() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    let requests_before = count(&sim, "-> Request");
    let writes_before = count(&sim, "-> Write");

    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: a_draft("burrow"),
        },
    ) else {
        panic!("the projects screen");
    };
    assert_eq!(shown.projects.len(), 2, "kept once every check passed");
    let written = nth(&sim, "-> Write", writes_before);
    let asked: Vec<usize> = sim
        .trace()
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("-> Request"))
        .map(|(at, _)| at)
        .skip(requests_before)
        .take(3)
        .collect();
    assert_eq!(
        asked.len(),
        3,
        "the token and both of the binding's credentials"
    );
    assert!(
        asked.iter().all(|at| *at < written),
        "checked before written: {asked:?} < {written}"
    );
    assert!(written < first(&sim, "-> Respond"));
    // Nothing of the check is kept: the listener asks who it is again.
    assert_eq!(
        sim.channel_calls()
            .iter()
            .filter(|(_, call)| matches!(call, Call::WhoAmI { .. }))
            .count(),
        2
    );
}

/// Amending with a blank token asks nobody; a typed one is checked, kept
/// when the platform accepts it, and the old one kept when it does not.
#[test]
fn an_amended_token_is_checked_and_a_blank_one_is_not() {
    let mut sim = Simulation::new();
    let mut state = watching(&[]);
    state
        .projects
        .get_mut(&project())
        .expect("the project")
        .credentials
        .insert(Platform::GitHub, Secret::new("ghp-the-old-one".to_owned()));
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();
    let held = |sim: &Simulation| {
        sim.disk()
            .expect("landed")
            .projects
            .get(&project())
            .expect("watched")
            .credentials
            .get(&Platform::GitHub)
            .map(|token| token.expose().to_owned())
    };

    let mut blank = a_draft("renamed");
    blank.credential.clear();
    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Amend {
            project: id.clone(),
            draft: blank,
        },
    ) else {
        panic!("the projects screen");
    };
    assert_eq!(reads(&sim), 0, "a blank box is the token already held");
    assert_eq!(held(&sim).as_deref(), Some("ghp-the-old-one"));

    let mut typed = a_draft("renamed");
    typed.credential = "ghp-the-new-one".to_owned();
    sim.next_platform_answers(401, r#"{"message":"Bad credentials"}"#);
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Amend {
                project: id.clone(),
                draft: typed.clone(),
            },
        ),
        Response::Refused(Refusal::TokenRefused {
            why: "GitHub does not accept it".to_owned()
        })
    );
    assert_eq!(reads(&sim), 1);
    assert_eq!(
        held(&sim).as_deref(),
        Some("ghp-the-old-one"),
        "refused keeps the old"
    );

    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        3,
        Request::Amend {
            project: id,
            draft: typed,
        },
    ) else {
        panic!("the projects screen");
    };
    assert_eq!(reads(&sim), 2);
    assert_eq!(
        held(&sim).as_deref(),
        Some("ghp-the-new-one"),
        "accepted replaces"
    );
}

/// A draft refused on its own is refused before any platform is asked.
#[test]
fn a_draft_refused_on_its_own_asks_no_platform() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    let mut blank = a_draft("blank");
    blank.repository = "git@github.com:example/blank.git".to_owned();
    assert!(matches!(
        ask(&mut sim, &mut instance, 1, Request::Create { draft: blank }),
        Response::Refused(Refusal::RepositoryRefused { .. })
    ));
    assert_eq!(reads(&sim), 0);
    assert!(
        sim.channel_calls()
            .iter()
            .all(|(_, call)| !matches!(call, Call::WhoAmI { .. } | Call::OpenSocket { .. }))
    );
}

/// A daemon dying mid-check answers nobody and keeps nothing: the request
/// was held and never kept, and the next start knows nothing of it.
#[test]
fn a_daemon_dying_mid_check_answers_nobody_and_keeps_nothing() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    for effect in instance.step(
        sim.now(),
        request(
            1,
            Request::Create {
                draft: a_draft("burrow"),
            },
        ),
    ) {
        sim.perform(effect);
    }
    assert_eq!(reads(&sim), 1, "the checks went out");
    assert!(sim.response(1).is_none(), "held, not answered");

    let mut instance = sim.crash(seed(1));
    sim.run_until(&mut instance, 5_000);
    assert!(sim.response(1).is_none(), "answered to nobody");
    assert_eq!(instance.state().projects.len(), 1, "nothing was kept");
    assert_eq!(reads(&sim), 1, "the next start asks nothing of it");
}
