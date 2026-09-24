//! A credential checked against its platform before it is kept: the
//! request held while the platforms answer, refused with the platform's
//! reason on the box the credential was typed in, kept once every check has
//! passed, and answered to nobody when the daemon dies mid-check. See
//! `docs/decisions/0076-a-credential-is-guided-in-and-checked-before-it-is-kept.md`.

use stageman_channel::Call;
use stageman_core::{Access, Platform, Secret};
use stageman_instance::{Request, Response};
use stageman_platform::Call as PlatformCall;
use stageman_wire::{AccessDraft, Refusal};

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

/// A private repository the token was not granted is the repository's
/// refusal, naming it, so the operator knows which grant is missing — see
/// `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
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
        Response::Refused(Refusal::NotReached {
            repository: crate::dashboard::repo("example", "burrow"),
            why: "GitHub cannot see it with the token — a fine-grained token has to be granted \
                  that repository"
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

/// Amending with the access left as it is asks nobody while the repository
/// stays; a token set is checked, kept when the platform accepts it, and
/// the old one kept when it does not.
#[test]
fn an_amended_token_is_checked_and_a_kept_one_is_not() {
    let mut sim = Simulation::new();
    let mut state = watching(&[]);
    state
        .projects
        .get_mut(&project())
        .expect("the project")
        .access
        .insert(
            Platform::GitHub,
            Access::Token {
                secret: Secret::new("ghp-the-old-one".to_owned()),
                owner: None,
                expires: None,
            },
        );
    crate::simulation::on_github(&mut state, "example/renamed");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    let id = project().to_string();
    let held = |sim: &Simulation| {
        sim.disk()
            .expect("landed")
            .projects
            .get(&project())
            .expect("watched")
            .access
            .get(&Platform::GitHub)
            .and_then(|access| match access {
                Access::Token { secret, .. } => Some(secret.expose().to_owned()),
                Access::Installation { .. } => None,
            })
    };

    let mut blank = a_draft("renamed");
    blank.access = AccessDraft::Token {
        token: None,
        repository: Some(crate::dashboard::repo("example", "renamed")),
    };
    // The repository as the project holds it: a moved one would be read
    // against the token kept, per
    // `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
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
    assert_eq!(
        reads(&sim),
        0,
        "as it is, on the repository it holds: nothing to ask"
    );
    assert_eq!(held(&sim).as_deref(), Some("ghp-the-old-one"));

    let mut typed = a_draft("renamed");
    typed.access = AccessDraft::Token {
        token: Some("ghp-the-new-one".to_owned()),
        repository: Some(crate::dashboard::repo("example", "renamed")),
    };
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
    blank.access = AccessDraft::Token {
        token: Some("ghp-not-a-real-token".to_owned()),
        repository: Some(crate::dashboard::repo(
            "git@github.com:example",
            "blank.git",
        )),
    };
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

/// A token checked at save is asked whose it is beside the read of the
/// repository, and what the platform said — the account, and the expiry
/// off the header — is kept beside the token and shown on the project;
/// an amendment that leaves the token and the repository alone asks
/// nothing and keeps what was said; one that moves the repository under
/// the token asks both again — see
/// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
#[test]
fn a_tokens_owner_and_expiry_are_read_at_the_check_and_kept_beside_it() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));
    sim.token_owned_by("somebody");
    sim.token_expires(Some("2026-09-27 08:42:20 UTC"));
    let owner_reads = |sim: &Simulation| {
        sim.platform_calls()
            .iter()
            .filter(|(_, call)| matches!(call, PlatformCall::Owner { .. }))
            .count()
    };

    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Create {
            draft: a_draft("aviary"),
        },
    ) else {
        panic!("created");
    };
    assert_eq!(reads(&sim), 1, "the repository, read with the token");
    assert_eq!(
        owner_reads(&sim),
        1,
        "and whose the token is, in the same breath"
    );
    let created = shown
        .projects
        .iter()
        .find(|listed| listed.name == "aviary")
        .expect("the new project");
    assert_eq!(
        created.access,
        Some(stageman_wire::AccessView::Token {
            owner: Some("somebody".to_owned()),
            expires: Some("2026-09-27T08:42:20Z".to_owned()),
            expired: false,
        })
    );
    let id = created.id.clone();
    let kept = |sim: &Simulation| {
        sim.disk()
            .expect("landed")
            .projects
            .values()
            .find(|watched| watched.name == "aviary" || watched.name == "renamed")
            .and_then(|watched| watched.access.get(&Platform::GitHub).cloned())
    };
    assert!(
        matches!(
            kept(&sim),
            Some(Access::Token { owner: Some(ref owner), expires: Some(at), .. })
                if owner == "somebody" && at.to_string() == "2026-09-27T08:42:20Z"
        ),
        "{:?}",
        kept(&sim)
    );

    // Nothing moved under the token: nothing asked, and the facts kept.
    let mut renamed = a_draft("renamed");
    renamed.access = AccessDraft::Token {
        token: None,
        repository: Some(crate::dashboard::repo("example", "aviary")),
    };
    sim.token_owned_by("nobody");
    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Amend {
            project: id,
            draft: renamed,
        },
    ) else {
        panic!("amended");
    };
    assert_eq!(reads(&sim), 1);
    assert_eq!(owner_reads(&sim), 1);
    assert!(matches!(
        kept(&sim),
        Some(Access::Token { owner: Some(ref owner), .. }) if owner == "somebody"
    ));
}

/// The repository moved under the token held is both reads again, and
/// what the platform says now is what is kept — an expiry it no longer
/// sends included.
#[test]
fn a_kept_tokens_facts_are_read_again_when_the_repository_moves() {
    let mut sim = Simulation::new();
    let mut state = watching(&[]);
    crate::simulation::holding_a_token_of(
        &mut state,
        "ghp-the-held-one",
        Some("somebody"),
        Some("2026-09-27T08:42:20Z".parse().expect("a time")),
    );
    crate::simulation::on_github(&mut state, "example/repo");
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.token_owned_by("nobody");
    sim.token_expires(None);

    let mut moved = a_draft("example");
    moved.access = AccessDraft::Token {
        token: None,
        repository: Some(crate::dashboard::repo("example", "other")),
    };
    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Amend {
            project: project().to_string(),
            draft: moved,
        },
    ) else {
        panic!("amended");
    };
    assert_eq!(reads(&sim), 1);
    assert_eq!(
        sim.platform_calls()
            .iter()
            .filter(|(_, call)| matches!(call, PlatformCall::Owner { .. }))
            .count(),
        1
    );
    let kept = sim
        .disk()
        .expect("landed")
        .projects
        .get(&project())
        .and_then(|watched| watched.access.get(&Platform::GitHub).cloned());
    assert!(
        matches!(
            kept,
            Some(Access::Token { owner: Some(ref owner), expires: None, .. }) if owner == "nobody"
        ),
        "{kept:?}"
    );
}

/// The platform refusing the read of the account refuses the token on
/// its box, as the read of the repository would; and an expiry the
/// platform sends that this instance cannot read refuses the token too,
/// rather than being dropped — see
/// `docs/decisions/0080-a-tokens-owner-and-expiry-are-kept-beside-it.md`.
#[test]
fn a_token_whose_account_or_expiry_cannot_be_read_is_refused() {
    let mut sim = Simulation::new();
    sim.holding(&watching(&[]));
    let mut instance = sim.wake(seed(1));

    // The repository's read answers as measured; the account's is refused.
    sim.next_platform_answers(200, r#"{"full_name":"example/aviary","private":true}"#);
    sim.next_platform_answers(401, r#"{"message":"Bad credentials"}"#);
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Create {
                draft: a_draft("aviary")
            }
        ),
        Response::Refused(Refusal::TokenRefused {
            why: "GitHub does not accept it".to_owned()
        })
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing kept");

    sim.token_expires(Some("tomorrow"));
    let refused = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Create {
            draft: a_draft("aviary"),
        },
    );
    assert!(
        matches!(
            &refused,
            Response::Refused(Refusal::TokenRefused { why }) if why.contains("expiry could not be read")
        ),
        "{refused:?}"
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing kept");
}
