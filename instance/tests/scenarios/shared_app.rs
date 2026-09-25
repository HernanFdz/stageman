//! Projects speaking through the instance's own Slack app — see
//! `docs/decisions/0081-the-instance-owns-a-slack-app-installed-per-workspace.md`:
//! the app listened to once with a voice per workspace it is installed on;
//! a message on it routed by the workspace it names and then by its room,
//! to a job, to a project's foreman, or to nobody; a person's mention in a
//! room none of several projects owns answered with the notice that says
//! where to ask, and costing no turn; an app's message a watching project's
//! or nobody's; and the connection following the workspaces: opened by the
//! first install, kept through a second, closed by the last forget and by
//! forgetting the app.

use std::collections::BTreeMap;

use stageman_channel::Call;
use stageman_core::{
    Binding, Channel, ChannelApp, Progress, ProjectId, Room, Secret, State, Uuid, Waiting,
    Workspace,
};
use stageman_instance::{Instance, Request, Response};

use crate::dashboard::{ask, count};
use crate::simulation::{Simulation, job, project, seed, watching, watching_a_channel};

/// The identifier the simulated platform answers an install's exchange
/// with, and the workspace every fixture here is on.
const TEAM: &str = "T0TEAM";
/// The second project's identifier.
const SECOND: u128 = 2;
/// The third project's identifier, on the other workspace.
const THIRD: u128 = 3;

/// The instance's app, installed on the fixture's workspace and, where
/// asked, on a second.
fn the_app(second_workspace: bool) -> ChannelApp {
    let mut workspaces = BTreeMap::from([(
        TEAM.to_owned(),
        Workspace {
            name: "Acme".to_owned(),
            bot_user: "U0BOT".to_owned(),
            bot_token: Secret::new("xoxb-acme".to_owned()),
        },
    )]);
    if second_workspace {
        workspaces.insert(
            "T0BETA".to_owned(),
            Workspace {
                name: "Beta".to_owned(),
                bot_user: "U0BOT".to_owned(),
                bot_token: Secret::new("xoxb-beta".to_owned()),
            },
        );
    }
    ChannelApp {
        client_id: "1234.5678".to_owned(),
        client_secret: Secret::new("s3cret".to_owned()),
        app_token: Secret::new("xapp-instance".to_owned()),
        workspaces,
    }
}

/// The fixture's project, bound through the workspace rather than an app
/// of its own, with a foreman room and its one job in a room, on the
/// instance's app; and, where asked, a second project on the same
/// workspace and a third on another.
fn on_the_shared_app(second: bool, third: bool) -> State {
    let mut state = watching(&[(job(1), Progress::Idle(Waiting::Silent))]);
    state.channel_apps.insert(Channel::Slack, the_app(third));
    let first = state.projects.get_mut(&project()).expect("the project");
    first
        .channels
        .insert(Channel::Slack, Binding::Workspace(TEAM.to_owned()));
    first.foreman_room = Some(Room {
        channel: Channel::Slack,
        id: "C0FOREMAN1".to_owned(),
    });
    if let Some(recorded) = first.jobs.get_mut(&job(1)) {
        recorded.room = Some(Room {
            channel: Channel::Slack,
            id: "C0JOBROOM1".to_owned(),
        });
    }
    let template = first.clone();
    if second {
        let mut other = template.clone();
        "second".clone_into(&mut other.name);
        other.jobs.clear();
        other.foreman_room = Some(Room {
            channel: Channel::Slack,
            id: "C0FOREMAN2".to_owned(),
        });
        other.watched.insert(Room {
            channel: Channel::Slack,
            id: "C0ALERTS".to_owned(),
        });
        state
            .projects
            .insert(ProjectId::from_uuid(Uuid::from_u128(SECOND)), other);
    }
    if third {
        let mut other = template;
        "third".clone_into(&mut other.name);
        other.jobs.clear();
        other
            .channels
            .insert(Channel::Slack, Binding::Workspace("T0BETA".to_owned()));
        other.foreman_room = Some(Room {
            channel: Channel::Slack,
            id: "C0FOREMAN3".to_owned(),
        });
        state
            .projects
            .insert(ProjectId::from_uuid(Uuid::from_u128(THIRD)), other);
    }
    state
}

/// Which project a foreman's message went to: the projects whose foreman
/// is working.
fn foremen_working(instance: &Instance) -> Vec<String> {
    instance
        .state()
        .projects
        .values()
        .filter(|watched| matches!(watched.attending, stageman_core::Attending::Working { .. }))
        .map(|watched| watched.name.clone())
        .collect()
}

/// A press on Install on a workspace and the tab back with a code: the
/// workspace kept, and whatever listening follows.
fn installs(sim: &mut Simulation, instance: &mut Instance, id: u64, code: &str) {
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
    let path = format!(
        "/instance/apps/slack/installed?code={code}&state={}",
        minted.state
    );
    sim.visits_path(sim.now(), &path);
    let until = sim.now() + 5_000;
    sim.run_until(instance, until);
}

/// The questions and the connection so far, in order, as their kinds.
fn asked(sim: &Simulation) -> Vec<String> {
    sim.channel_calls()
        .iter()
        .filter_map(|(_, call)| match call {
            Call::WhoAmI { .. } => Some("who am I".to_owned()),
            Call::OpenSocket { .. } => Some("where to connect".to_owned()),
            _ => None,
        })
        .collect()
}

/// The instance's app is listened to once, whatever the number of
/// workspaces and projects: who this instance is asked once per workspace,
/// then where to connect once, then one connection, held with a voice per
/// workspace; a project's own app beside it is its own connection still.
#[test]
fn the_instances_app_is_listened_to_once_with_a_voice_per_workspace() {
    let mut sim = Simulation::new();
    sim.holding(&on_the_shared_app(true, true));
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);

    assert_eq!(
        asked(&sim),
        vec!["who am I", "who am I", "where to connect"],
        "once per workspace, then once for the app"
    );
    assert_eq!(sim.listening(), 1, "one connection for three projects");
    let held = instance.snapshot();
    let listener = held
        .get("held")
        .and_then(|held| held.get("listeners"))
        .and_then(|listeners| listeners.get("app:Slack"))
        .expect("the app's listener");
    assert_eq!(listener["voices"][TEAM]["us"]["user"], "U0BOT");
    assert_eq!(listener["voices"]["T0BETA"]["us"]["user"], "U0BOT");
    assert!(
        held["held"]["listeners"]
            .get(project().to_string())
            .is_none(),
        "no listener of the project's own"
    );
}

/// A message on the instance's app is routed by the workspace it names and
/// then by its room: a mention in the first project's foreman room is that
/// foreman's, one in its job's room is that job's, and one in the third
/// project's foreman room on the other workspace is the third's.
#[test]
fn a_message_on_the_instances_app_finds_its_project_by_workspace_and_room() {
    let mut sim = Simulation::new();
    sim.holding(&on_the_shared_app(false, true));
    // The job's container is there to resume, so that waking keeps the job.
    let (name, held) = Simulation::ours(&stageman_job::container(&job(1)));
    sim.container(&name, held);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);

    sim.on_workspace(TEAM);
    sim.says_in_room_id(
        6_000,
        "C0FOREMAN1",
        "1788000099.000001",
        "look at the parser",
    );
    sim.run_until(&mut instance, 7_000);
    assert_eq!(foremen_working(&instance), vec!["example".to_owned()]);

    sim.says_in_room_id(8_000, "C0JOBROOM1", "1788000099.000002", "and the lexer");
    sim.run_until(&mut instance, 9_000);
    assert_eq!(
        instance
            .state()
            .job(&job(1))
            .map(|recorded| recorded.progress.clone()),
        Some(Progress::Working),
        "the job whose room it is in"
    );

    sim.on_workspace("T0BETA");
    sim.says_in_room_id(10_000, "C0FOREMAN3", "1788000099.000003", "and yours");
    sim.run_until(&mut instance, 11_000);
    // The first foreman's turn has ended by now; the third's is the one
    // running.
    assert_eq!(foremen_working(&instance), vec!["third".to_owned()]);
}

/// A person's mention in a room none of several projects on one workspace
/// owns is answered where it was said with where to ask instead, naming
/// each project's foreman room, and starts no turn; with one project on
/// the workspace the same mention is that project's foreman's.
#[test]
fn a_mention_in_a_room_none_of_several_projects_owns_is_pointed_elsewhere() {
    let mut sim = Simulation::new();
    sim.holding(&on_the_shared_app(true, false));
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);
    let turns_before = sim.first_turn();

    sim.on_workspace(TEAM);
    sim.says_in_room_id(6_000, "C0NOBODYS", "1788000099.000007", "anybody?");
    sim.run_until(&mut instance, 8_000);
    assert!(foremen_working(&instance).is_empty(), "no foreman wakes");
    assert_eq!(sim.first_turn(), turns_before, "no turn is spent");
    let posted = sim.posts();
    let (place, text) = posted.last().expect("the notice is posted");
    assert_eq!(place.room.id, "C0NOBODYS");
    assert_eq!(
        place.thread.as_deref(),
        Some("1788000099.000007"),
        "under the message, where it was said"
    );
    assert_eq!(
        text,
        "🔀 This room is nobody's here, since several projects talk on this workspace. Ask in \
         <#C0FOREMAN1> for example, or in <#C0FOREMAN2> for second."
    );

    let mut sim = Simulation::new();
    sim.holding(&on_the_shared_app(false, false));
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);
    sim.on_workspace(TEAM);
    sim.says_in_room_id(6_000, "C0NOBODYS", "1788000099.000007", "anybody?");
    sim.run_until(&mut instance, 8_000);
    assert!(
        sim.first_turn().is_some(),
        "the workspace's only project's foreman took the turn"
    );
    assert!(
        !sim.posts().iter().any(|(_, text)| text.starts_with("🔀")),
        "and no notice was posted"
    );
}

/// Another app's message on the instance's app is the foreman's of the
/// project watching the room, and nobody's anywhere else, however many
/// projects share the workspace.
#[test]
fn an_apps_message_on_the_instances_app_is_a_watching_projects_or_nobodys() {
    let mut sim = Simulation::new();
    sim.holding(&on_the_shared_app(true, false));
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);

    sim.on_workspace(TEAM);
    sim.app_posts(
        6_000,
        "C0NOBODYS",
        "1788000099.000011",
        "Issue opened",
        "a bug",
    );
    sim.run_until(&mut instance, 7_000);
    assert!(foremen_working(&instance).is_empty(), "nobody's");
    assert!(sim.posts().is_empty(), "and no notice either");

    sim.app_posts(
        8_000,
        "C0ALERTS",
        "1788000099.000012",
        "Alert fired",
        "disk full",
    );
    sim.run_until(&mut instance, 9_000);
    assert_eq!(foremen_working(&instance), vec!["second".to_owned()]);
}

/// The connection follows the workspaces: nothing listens while the app is
/// installed nowhere, the first install opens it, a workspace forgotten
/// drops its voice and the last one forgotten closes it, and forgetting
/// the app closes it too.
#[test]
fn the_connection_follows_the_workspaces() {
    let mut sim = Simulation::new();
    let mut state = watching_a_channel(&[]);
    let mut app = the_app(false);
    app.workspaces.clear();
    state.channel_apps.insert(Channel::Slack, app);
    // The project keeps an app of its own here, so that its connection can
    // be told from the instance's.
    sim.holding(&state);
    let mut instance = sim.wake(seed(1));
    sim.run_until(&mut instance, 5_000);
    assert_eq!(
        sim.listening(),
        1,
        "the project's own app, and nothing for the instance's"
    );
    assert_eq!(asked(&sim), vec!["who am I", "where to connect"]);

    installs(&mut sim, &mut instance, 1, "c0de");
    assert_eq!(
        sim.listening(),
        2,
        "the first install opens the app's connection"
    );
    assert_eq!(
        asked(&sim),
        vec![
            "who am I",
            "where to connect",
            "who am I",
            "where to connect"
        ]
    );

    let disconnects_before = count(&sim, "-> Disconnect");
    let Response::Apps(_) = ask(
        &mut sim,
        &mut instance,
        2,
        Request::ForgetWorkspace {
            channel: "slack".to_owned(),
            id: TEAM.to_owned(),
        },
    ) else {
        panic!("the Instance page");
    };
    let until = sim.now() + 2_000;
    sim.run_until(&mut instance, until);
    assert_eq!(
        count(&sim, "-> Disconnect"),
        disconnects_before + 1,
        "the last workspace forgotten closes the app's connection"
    );

    installs(&mut sim, &mut instance, 3, "c0de2");
    let disconnects_before = count(&sim, "-> Disconnect");
    let Response::Apps(_) = ask(
        &mut sim,
        &mut instance,
        4,
        Request::ForgetChannelApp {
            channel: "slack".to_owned(),
        },
    ) else {
        panic!("the Instance page");
    };
    let until = sim.now() + 2_000;
    sim.run_until(&mut instance, until);
    assert_eq!(
        count(&sim, "-> Disconnect"),
        disconnects_before + 1,
        "forgetting the app closes its connection"
    );
}
