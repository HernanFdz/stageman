//! Listening on a project's channel, run against the simulated platform:
//! the connection's lifecycle as the instance drives it, from the questions
//! that come before a socket to the gap an unscheduled close leaves.

use stageman_core::ProjectId;
use stageman_instance::{Instance, Request};
use stageman_wire::ChannelDraft;

use crate::dashboard::a_draft;
use crate::simulation::{Simulation, project, request, seed, watching, watching_a_channel};

/// The kinds in the trace, with the fields taken out.
///
/// Booting asks for nothing a listener does — no request, no socket, no
/// timer — so the whole trace can be read for those.
fn kinds(world: &Simulation) -> Vec<String> {
    world
        .shape()
        .into_iter()
        .map(|line| {
            line.split_once(" {")
                .map_or_else(|| line.clone(), |(head, _)| head.to_owned())
        })
        .collect()
}

/// What the instance holds about one project's listener, or null once it
/// holds nothing.
fn listener(instance: &Instance, project: ProjectId) -> serde_json::Value {
    instance
        .snapshot()
        .get("held")
        .and_then(|held| held.get("listeners"))
        .and_then(|listeners| listeners.get(project.to_string()))
        .map_or(serde_json::Value::Null, Clone::clone)
}

/// Waking on a bound project asks who this instance is, then where to
/// connect, then connects; the platform's greeting is what says it is up.
/// A message is acknowledged in the step it arrives in, before anything is
/// done about it.
#[test]
fn listening_begins_on_waking_and_a_message_is_acknowledged_before_it_is_acted_on() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    let opening: Vec<String> = kinds(&world)
        .into_iter()
        .filter(|line| {
            ["-> Request", "<- Responded", "-> Connect", "<- Frame"]
                .iter()
                .any(|kind| line.starts_with(kind))
        })
        .collect();
    assert_eq!(
        opening,
        [
            "-> Request",
            "<- Responded",
            "-> Request",
            "<- Responded",
            "-> Connect",
            "<- Frame",
        ],
        "who am I, where to connect, connect, and the greeting"
    );
    assert_eq!(
        listener(&instance, project())["phase"],
        serde_json::json!({ "Listening": { "socket": world.sockets_open()[0] } })
    );
    assert_eq!(listener(&instance, project())["us"]["user"], "U0BOT");

    world.says_at_root(100, 1, "look at the parser");
    world.run_until(&mut instance, 100);
    let shape = world.shape();
    let heard = shape
        .iter()
        .rposition(|line| line.starts_with("<- Frame"))
        .expect("the message arrived");
    assert!(
        shape[heard + 1].starts_with("-> Transmit"),
        "acknowledged first, whatever follows: {:?}",
        &shape[heard..]
    );
    assert_eq!(world.acked(), ["e-1"]);
}

/// The platform's warning opens a replacement before the old connection
/// closes: what is said on the old one until then is still heard, its
/// closing costs nothing, and nothing waits.
#[test]
fn the_platforms_warning_opens_a_replacement_before_the_old_connection_closes() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    let old = world.sockets_open()[0];

    world.platform_warns(100);
    world.run_until(&mut instance, 100);
    let open = world.sockets_open();
    assert_eq!(open.len(), 2, "two connections, briefly: {open:?}");
    let new = open[1];
    assert_eq!(
        listener(&instance, project())["phase"],
        serde_json::json!({ "Listening": { "socket": new } }),
        "the replacement is the one listened on"
    );

    // Said on the old connection while it is being drained.
    world.says_at_root_on(old, 150, 1, "still here");
    world.run_until(&mut instance, 150);
    assert_eq!(
        world.acked(),
        ["e-1"],
        "heard and acknowledged on the old one"
    );

    world.platform_closes_socket(old, 200);
    world.run_until(&mut instance, 5_000);
    assert_eq!(world.sockets_open(), [new]);
    assert_eq!(
        listener(&instance, project())["phase"],
        serde_json::json!({ "Listening": { "socket": new } }),
        "the old one closing changed nothing"
    );
    assert_eq!(
        listener(&instance, project())["deaf_since"],
        serde_json::Value::Null,
        "there was no window with no connection"
    );
    let waits = kinds(&world)
        .iter()
        .filter(|line| line.starts_with("-> Wake"))
        .count();
    assert_eq!(
        waits, 1,
        "only the settling timer was set; nothing waited to reconnect"
    );
}

/// A connection the platform closes without warning is tried again after
/// a wait, and the window in which nothing was listening is measured from
/// the close to the next greeting.
#[test]
fn an_unscheduled_close_waits_and_reconnects_and_the_gap_is_measured() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));

    world.platform_closes(300);
    world.run_until(&mut instance, 300);
    assert!(world.sockets_open().is_empty());
    assert_eq!(listener(&instance, project())["deaf_since"], 300);
    assert!(
        listener(&instance, project())["phase"]["Waiting"].is_object(),
        "{}",
        listener(&instance, project())
    );

    // Nothing before the wait is over.
    world.run_until(&mut instance, 5_299);
    assert!(world.sockets_open().is_empty(), "still waiting");
    world.run_until(&mut instance, 5_400);
    assert_eq!(world.sockets_open().len(), 1, "reconnected after the wait");
    assert_eq!(
        listener(&instance, project())["deaf_since"],
        serde_json::Value::Null,
        "the window closed with the greeting"
    );
}

/// A platform that refuses a listener's credential is tried again at a
/// fixed rate, and listened to once it accepts.
#[test]
fn a_refused_credential_is_tried_again_at_a_fixed_rate() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    world.next_listen_fails("invalid_auth");
    world.next_listen_fails("invalid_auth");
    let mut instance = world.wake(seed(1));

    assert!(
        world.sockets_open().is_empty(),
        "refused, so nothing is open"
    );
    world.run_until(&mut instance, 5_100);
    assert!(world.sockets_open().is_empty(), "refused again");
    world.run_until(&mut instance, 10_200);
    assert_eq!(world.sockets_open().len(), 1, "accepted the third time");
    let asked = kinds(&world)
        .iter()
        .filter(|line| line.starts_with("-> Request"))
        .count();
    assert_eq!(asked, 4, "three times who am I, and once where to connect");
}

/// A socket that cannot be opened at all is tried again after the wait, and
/// the window is measured from the failure.
#[test]
fn a_socket_that_cannot_open_is_tried_again_after_the_wait() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    world.next_socket_fails("the handshake was refused");
    let mut instance = world.wake(seed(1));

    assert!(world.sockets_open().is_empty());
    assert_eq!(listener(&instance, project())["deaf_since"], 0);
    world.run_until(&mut instance, 6_000);
    assert_eq!(world.sockets_open().len(), 1);
    assert_eq!(
        listener(&instance, project())["deaf_since"],
        serde_json::Value::Null
    );
}

/// Forgetting a project disconnects its channel, and the socket's end is
/// placed rather than wondered about.
#[test]
fn forgetting_a_project_disconnects_its_channel() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    assert_eq!(world.listening(), 1);

    for effect in instance.step(crate::simulation::request(
        1,
        stageman_instance::Request::Forget {
            project: project().to_string(),
        },
    )) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 5_000);

    assert_eq!(world.listening(), 0, "disconnected");
    assert!(listener(&instance, project()).is_null(), "and forgotten");
    assert_eq!(
        instance.snapshot()["held"]["sockets"],
        serde_json::json!([]),
        "its end was placed"
    );
    let waits = kinds(&world)
        .iter()
        .filter(|line| line.starts_with("-> Wake"))
        .count();
    assert_eq!(waits, 1, "nothing tried to reconnect");
}

/// A crash takes the socket with it, and waking listens again from what
/// the project says.
#[test]
fn a_crash_takes_the_socket_and_waking_listens_again() {
    let mut world = Simulation::new();
    world.holding(&watching_a_channel(&[]));
    let mut instance = world.wake(seed(1));
    world.run_until(&mut instance, 100);
    assert_eq!(world.listening(), 1);

    let mut instance = world.crash(seed(1));
    world.run_until(&mut instance, 200);
    assert_eq!(world.listening(), 1, "listened to again from the record");
    assert!(
        listener(&instance, project())["phase"]["Listening"].is_object(),
        "{}",
        listener(&instance, project())
    );
}

/// A channel bound while its record could not be written is listened to
/// once the wait is over: the question that waited on the failed write is
/// asked again rather than never, the way a connection that failed is.
#[test]
fn a_channel_bound_under_a_failed_write_is_listened_to_after_the_wait() {
    let mut world = Simulation::new();
    world.holding(&watching(&[]));
    let mut instance = world.wake(seed(1));
    world.next_write_fails("the disk is full");

    let mut draft = a_draft("burrow");
    draft.channel = ChannelDraft {
        credential: "xoxb-not-a-real-token".to_owned(),
        listen_credential: "xapp-not-a-real-token".to_owned(),
    };
    for effect in instance.step(request(1, Request::Create { draft })) {
        world.perform(effect);
    }
    world.run_until(&mut instance, 100);
    assert_eq!(
        world.listening(),
        0,
        "the question waited on a write that never landed, so it was never asked"
    );

    world.run_until(&mut instance, 6_000);
    assert_eq!(
        world.listening(),
        1,
        "asked again after the wait, and listened to"
    );
}
