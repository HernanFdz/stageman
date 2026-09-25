//! A project's binding in its two shapes, on the project form — see
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`:
//! a project created on the workspace its tab brought back, the state
//! spent by the save; one moved onto the workspace from an app of its own
//! and back, with what listens following; a binding kept not checked
//! again, and an own pair pasted again its own; and the panel's check,
//! answering where the app speaks and refusing a pair already heard with.

use stageman_core::{Binding, Channel};
use stageman_instance::{Request, Response};
use stageman_wire::{BindingDraft, BindingView, Bound, ChannelDraft, Refusal, WorkspaceArrival};

use crate::dashboard::{a_draft, a_draft_on_its_own_app, ask, count};
use crate::shared_app::{TEAM, on_the_shared_app, the_app};
use crate::simulation::{Simulation, project, seed, watching_a_channel};

/// A draft bound to the workspace that came back under a state.
fn on_the_workspace(name: &str, state: &str) -> stageman_wire::Draft {
    let mut draft = a_draft(name);
    draft.binding = BindingDraft::Workspace {
        arrival: Some(state.to_owned()),
    };
    draft
}

/// What the form is told when it asks whether its tab has come back under
/// a state.
fn arrival(
    sim: &mut Simulation,
    instance: &mut stageman_instance::Instance,
    id: u64,
    state: &str,
) -> Response {
    ask(
        sim,
        instance,
        id,
        Request::WorkspaceArrived {
            channel: "slack".to_owned(),
            state: state.to_owned(),
        },
    )
}

/// A press on Install on a workspace and the tab back with a code: the
/// state the link was minted under, which the workspace came back under.
fn installed_under(sim: &mut Simulation, instance: &mut stageman_instance::Instance) -> String {
    let Response::InstallLink(minted) = ask(
        sim,
        instance,
        1,
        Request::WorkspaceLink {
            channel: "slack".to_owned(),
        },
    ) else {
        panic!("an install link");
    };
    let path = format!(
        "/instance/apps/slack/installed?code=c0de&state={}",
        minted.state
    );
    sim.visits_path(sim.now(), &path);
    let until = sim.now() + 5_000;
    sim.run_until(instance, until);
    minted.state
}

/// A form's tab comes back with the workspace, the form learns so on its
/// tick, and the project is created on that workspace: its binding says
/// the workspace by name, the projects screen says an app is registered,
/// nothing new listens, and the state is spent by the save so a second
/// form cannot name the same arrival.
#[test]
fn a_project_is_created_on_the_workspace_its_tab_brought_back() {
    let mut sim = Simulation::new();
    let mut state = on_the_shared_app(false, false);
    state
        .channel_apps
        .get_mut(&Channel::Slack)
        .expect("the app")
        .workspaces
        .clear();
    state.projects.clear();
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);

    let Response::InstallLink(minted) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::WorkspaceLink {
            channel: "slack".to_owned(),
        },
    ) else {
        panic!("an install link");
    };
    assert_eq!(
        arrival(&mut sim, &mut instance, 2, &minted.state),
        Response::WorkspaceArrival(WorkspaceArrival::NotYet),
        "the tab is still out"
    );
    assert_eq!(
        arrival(&mut sim, &mut instance, 3, "nobody-minted-this"),
        Response::Refused(Refusal::WorkspaceArrivalUnknown)
    );
    let path = format!(
        "/instance/apps/slack/installed?code=c0de&state={}",
        minted.state
    );
    sim.visits_path(sim.now(), &path);
    let until = sim.now() + 5_000;
    sim.run_until(&mut instance, until);
    assert_eq!(
        arrival(&mut sim, &mut instance, 4, &minted.state),
        Response::WorkspaceArrival(WorkspaceArrival::Installed {
            id: TEAM.to_owned(),
            name: "Acme".to_owned(),
        })
    );
    let listening_before = sim.listening();

    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        5,
        Request::Create {
            draft: on_the_workspace("burrow", &minted.state),
        },
    ) else {
        panic!("the projects screen");
    };
    assert!(shown.slack_app_registered);
    let burrow = shown
        .projects
        .iter()
        .find(|project| project.name == "burrow")
        .expect("the new project");
    assert_eq!(
        burrow.binding,
        Some(BindingView::Workspace {
            id: TEAM.to_owned(),
            name: "Acme".to_owned(),
        })
    );
    let kept = instance
        .state()
        .projects
        .values()
        .find(|project| project.name == "burrow")
        .expect("kept");
    assert_eq!(
        kept.channels.get(&Channel::Slack),
        Some(&Binding::Workspace(TEAM.to_owned()))
    );
    sim.run_until(&mut instance, until + 2_000);
    assert_eq!(
        sim.listening(),
        listening_before,
        "the app was listened to already; a workspace binding opens nothing"
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            6,
            Request::Create {
                draft: on_the_workspace("second", &minted.state),
            },
        ),
        Response::Refused(Refusal::WorkspaceArrivalUnknown),
        "the state was spent by the save"
    );
}

/// A project with an app of its own is moved onto the workspace its tab
/// brought back, and what listened for its own app is closed; moved back
/// onto an app of its own, it is listened to again.
#[test]
fn a_project_moves_onto_the_workspace_from_an_app_of_its_own_and_back() {
    let mut sim = Simulation::new();
    let mut state = watching_a_channel(&[]);
    let mut app = the_app(false);
    app.workspaces.clear();
    state.channel_apps.insert(Channel::Slack, app);
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);
    assert_eq!(sim.listening(), 1, "the project's own app");

    let state = installed_under(&mut sim, &mut instance);
    let disconnects_before = count(&sim, "-> Disconnect");
    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        7,
        Request::Amend {
            project: project().to_string(),
            draft: on_the_workspace("example", &state),
        },
    ) else {
        panic!("the projects screen");
    };
    assert_eq!(
        shown.projects[0].binding,
        Some(BindingView::Workspace {
            id: TEAM.to_owned(),
            name: "Acme".to_owned(),
        })
    );
    let until = sim.now() + 2_000;
    sim.run_until(&mut instance, until);
    assert_eq!(
        count(&sim, "-> Disconnect"),
        disconnects_before + 1,
        "the connection of the app it left is closed"
    );
    assert_eq!(
        sim.listening(),
        1,
        "the instance's app, and nothing of its own"
    );

    // And back onto an app of its own, with tokens of another app.
    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        8,
        Request::Amend {
            project: project().to_string(),
            draft: a_draft_on_its_own_app("example", "xoxb-another"),
        },
    ) else {
        panic!("the projects screen");
    };
    assert!(
        matches!(shown.projects[0].binding, Some(BindingView::Own { .. })),
        "{:?}",
        shown.projects[0].binding
    );
    let until = sim.now() + 5_000;
    sim.run_until(&mut instance, until);
    assert_eq!(sim.listening(), 2, "its own app is listened to again");
}

/// A binding kept as it is asks the channel nothing on save. The same app
/// with its app-level token regenerated is checked again, since the pair
/// differs, and is the project's own rather than another's, though its
/// bot is the one the project's listener already hears with; and it is
/// listened to again with the new token.
#[test]
fn a_binding_kept_is_not_checked_again_and_its_own_app_checked_again_is_its_own() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);
    let asks_the_channel =
        |line: &String| line.contains("auth.test") || line.contains("apps.connections.open");
    assert!(
        sim.trace().iter().any(asks_the_channel),
        "the listener asked, so the trace does say it"
    );
    let trace_before = sim.trace().len();

    let mut kept = a_draft("example");
    kept.binding = BindingDraft::Kept;
    let Response::Projects(_) = ask(
        &mut sim,
        &mut instance,
        1,
        Request::Amend {
            project: project().to_string(),
            draft: kept,
        },
    ) else {
        panic!("the projects screen");
    };
    // The token was set, so the platform was asked about it; nothing was
    // asked of the channel.
    assert!(
        !sim.trace().iter().skip(trace_before).any(asks_the_channel),
        "nothing asked of the channel for a binding kept"
    );

    let trace_before = sim.trace().len();
    let disconnects_before = count(&sim, "-> Disconnect");
    let mut rotated = a_draft("example");
    rotated.binding = BindingDraft::Own(ChannelDraft {
        credential: "xoxb-not-a-real-token".to_owned(),
        listen_credential: "xapp-regenerated".to_owned(),
    });
    let Response::Projects(shown) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::Amend {
            project: project().to_string(),
            draft: rotated,
        },
    ) else {
        panic!("its own app, checked again, is its own");
    };
    assert!(
        sim.trace().iter().skip(trace_before).any(asks_the_channel),
        "a pair that differs is checked"
    );
    assert!(matches!(
        shown.projects[0].binding,
        Some(BindingView::Own { .. })
    ));
    let until = sim.now() + 5_000;
    sim.run_until(&mut instance, until);
    assert_eq!(
        count(&sim, "-> Disconnect"),
        disconnects_before + 1,
        "the connection on the old token is closed"
    );
    assert_eq!(sim.listening(), 1, "and one is open on the new");
}

/// The form's panel checks a pair before the form moves onto it: accepted,
/// the answer says where the app speaks; a pair another project's
/// listener already hears with is refused naming it, unless the pair is
/// that project's own.
#[test]
fn the_panels_check_says_where_the_app_speaks_and_refuses_a_pair_already_heard_with() {
    let mut sim = Simulation::new();
    sim.holding(&watching_a_channel(&[]));
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);

    let pair = |credential: &str| ChannelDraft {
        credential: credential.to_owned(),
        listen_credential: "xapp-not-a-real-token".to_owned(),
    };
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            1,
            Request::Binds {
                project: None,
                binding: pair("xoxb-burrow"),
            },
        ),
        Response::Bound(Bound {
            url: "https://example.slack.com/".to_owned(),
        })
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            2,
            Request::Binds {
                project: None,
                binding: pair("xoxb-not-a-real-token"),
            },
        ),
        Response::Refused(Refusal::ChannelRefused {
            listening: false,
            why: "it is already example's app".to_owned(),
        })
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            3,
            Request::Binds {
                project: Some(project().to_string()),
                binding: pair("xoxb-not-a-real-token"),
            },
        ),
        Response::Bound(Bound {
            url: "https://example.slack.com/".to_owned(),
        })
    );
    assert_eq!(
        ask(
            &mut sim,
            &mut instance,
            4,
            Request::Binds {
                project: None,
                binding: pair(""),
            },
        ),
        Response::Refused(Refusal::ChannelIncomplete)
    );
    assert_eq!(instance.state().projects.len(), 1, "nothing was kept");
}
