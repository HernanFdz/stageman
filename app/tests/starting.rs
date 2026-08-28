//! The binary, run as a binary.
//!
//! There is no first run to test any more. An instance starts with nothing —
//! no agents, no projects, no container runtime — and asks nothing, per
//! `docs/decisions/0021-an-instance-starts-empty.md`. What is left to check is
//! that starting from nothing works, that starting again changes nothing, that
//! the two things which *do* come from the environment fail clearly when they
//! are wrong, and that what a start produces is a dashboard with the instance
//! already on it.
//!
//! **A start that works no longer ends.** The binary serves until it is
//! stopped, so the tests below split in two: the ones about refusing to start
//! wait for an exit, and the ones about starting wait for the line that names
//! the address and then kill the process. Killing is the supported way to stop
//! this — `docs/conventions.md` §4 — so the tests stop it the way an operator
//! will.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers in an integration-test crate are not seen as test code by \
              clippy's allow-expect-in-tests, which only covers #[test] functions and \
              #[cfg(test)] modules; a helper that failed here has nothing to report to. \
              The bare panic is the one place a message has to be built rather than \
              named: a process that stopped instead of serving is only diagnosable \
              from what it managed to say first"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use stageman::Store;
use stageman_core::{
    Agent, AgentConfig, Job, JobId, Key, Progress, Project, ProjectId, Secret, State, Timestamp,
};

/// A key, as an operator would supply it: thirty-two bytes of base64.
const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// Runs the binary with exactly the environment given and nothing else.
///
/// `env_clear` is the point rather than tidiness: `docs/conventions.md` §3 says
/// what a process is handed is constructed and never inherited, and a test that
/// let the surrounding shell through could pass because of a variable nobody
/// meant to set.
fn run(snapshot: &PathBuf, variables: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stageman"));
    command.env_clear().env("STAGEMAN_STATE", snapshot);
    for (name, value) in variables {
        command.env(name, value);
    }
    command.output().expect("the binary runs")
}

/// A binary that started, is serving, and is killed when this is dropped.
struct Serving {
    child: Child,
    /// Everything it said before it began serving.
    said: String,
    /// The address it is actually listening on, which is not the one asked for
    /// — the tests ask for port zero so that two running at once cannot
    /// collide.
    address: String,
}

impl Serving {
    /// The path it said it was keeping the instance in.
    fn instance(&self) -> PathBuf {
        let line = self
            .said
            .lines()
            .find_map(|line| line.trim().strip_prefix("instance   "))
            .expect("it says where the instance is");
        PathBuf::from(line.trim())
    }

    /// The whole of the response to one `GET`, headers included.
    ///
    /// Written by hand rather than with an HTTP client, because `Connection:
    /// close` makes the whole exchange "write a request, read to end of file"
    /// and a dependency would buy nothing. Headers are kept in the returned
    /// text on purpose: a test asserting on a status line should not have to
    /// take this function's word for it.
    fn get(&self, path: &str) -> String {
        let mut connection = TcpStream::connect(&self.address).expect("the dashboard accepts");
        write!(
            connection,
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address
        )
        .expect("the request is sent");
        let mut response = Vec::new();
        connection
            .read_to_end(&mut response)
            .expect("the response arrives");
        String::from_utf8_lossy(&response).into_owned()
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        // Killed, not signalled and waited for. `docs/conventions.md` §4 makes
        // hard-killing a supported operation rather than an accident, so the
        // tests exercise the same thing an operator does.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the binary and waits until it says where it is listening.
///
/// `PORT=0` asks the operating system for whichever port is free, and the
/// address is read back out of the process rather than assumed — which is the
/// only way several of these can run at once, and nextest runs every test in
/// its own process.
fn serving(snapshot: &Path, variables: &[(&str, &str)]) -> Serving {
    let mut all = vec![("STAGEMAN_STATE", snapshot.to_string_lossy().into_owned())];
    all.extend(
        variables
            .iter()
            .map(|(name, value)| (*name, (*value).to_owned())),
    );
    started(&all)
}

/// Starts the binary with exactly these variables, saying nothing about where
/// the instance goes.
///
/// Split from `serving` for the one test that is *about* the default: telling
/// it where to put the file is precisely what must not happen there.
fn started(variables: &[(&str, String)]) -> Serving {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stageman"));
    command
        .env_clear()
        .env("IP", "127.0.0.1")
        .env("PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in variables {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("the binary runs");
    let stdout = child.stdout.take().expect("its output was piped");
    let (said, address) = listening(stdout);
    Serving {
        child,
        said,
        address,
    }
}

/// How long a start is given to say where it is listening.
///
/// Bounded on both sides, which is why it is a small number rather than a
/// generous one. Below: a start prints its address in well under a second, so
/// this is twenty times what it needs even under a loaded parallel run. Above:
/// `cargo mutants` sets its own timeout from the baseline run — twenty seconds
/// here — and a wait longer than that turns a mutant this suite *does* catch
/// into a reported timeout, which the gate treats as a survivor. Both bounds
/// were measured rather than guessed.
const PATIENCE: Duration = Duration::from_secs(10);

/// Reads a starting binary's output until it names the address it took.
///
/// Read on another thread with a deadline, which is not ceremony: a blocking
/// read of a serving process's output has no end of file to reach, so a start
/// that prints nothing hangs rather than fails. Mutation testing found this by
/// deleting the line that prints the address and watching the suite time out
/// instead of go red.
///
/// End of file before that line means it exited instead of serving, and the
/// output it did produce is the only evidence of why — so it is reported
/// rather than swallowed.
fn listening(stdout: ChildStdout) -> (String, String) {
    const MARKER: &str = "dashboard  http://";

    let (found, arrived) = mpsc::channel();
    std::thread::spawn(move || {
        let mut said = String::new();
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            said.push_str(&line);
            said.push('\n');
            if let Some((_, address)) = line.split_once(MARKER) {
                let _ = found.send(Some((said, address.trim().to_owned())));
                return;
            }
        }
        let _ = found.send(None);
    });

    match arrived.recv_timeout(PATIENCE) {
        Ok(Some(listening)) => listening,
        Ok(None) => panic!("it stopped without ever saying where it was listening"),
        Err(waited) => panic!("it started and never said where it was listening: {waited}"),
    }
}

/// One job, in whatever state the caller needs it.
fn job(progress: Progress) -> Job {
    Job {
        agent: Agent::Claude,
        reason: "because a test said so".to_owned(),
        kickoff: "do the thing".to_owned(),
        created_at: Timestamp::UNIX_EPOCH,
        progress,
    }
}

/// An instance with one project in it, which is the smallest state that puts
/// anything on the dashboard.
fn watching(name: &str, repository: &str) -> State {
    State {
        agents: BTreeMap::from([(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new("not-a-real-credential".to_owned()),
            },
        )]),
        projects: BTreeMap::from([(
            ProjectId::from_uuid(uuid::Uuid::nil()),
            Project {
                name: name.to_owned(),
                repository: repository.to_owned(),
                orchestrator_agent: Agent::Claude,
                job_agents: BTreeSet::from([Agent::Claude]),
                credentials: BTreeMap::new(),
                jobs: BTreeMap::new(),
            },
        )]),
    }
}

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let snapshot = directory.path().join("state.json");
    (directory, snapshot)
}

fn key() -> Key {
    Key::from_base64(KEY).expect("the test key is well formed")
}

#[test]
fn an_instance_starts_with_nothing_configured() {
    let (_kept, snapshot) = scratch();

    // Reaching the address line is the assertion: `serving` fails loudly if
    // the process stops before printing one.
    let _running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    assert!(snapshot.exists(), "it should have written an instance");
}

/// What removing the first-run flow actually bought: nothing is asked, so
/// nothing has to be answered, so a machine with no terminal stops being a
/// special case rather than being handled as one.
#[test]
fn starting_needs_no_answers_and_no_terminal() {
    let (_kept, snapshot) = scratch();

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let said = &running.said;

    assert!(said.contains("agents     0"), "{said}");
    assert!(said.contains("projects   0"), "{said}");
}

#[test]
fn starting_again_changes_nothing() {
    let (_kept, snapshot) = scratch();
    drop(serving(&snapshot, &[("STAGEMAN_KEY", KEY)]));
    let first = std::fs::read_to_string(&snapshot).expect("it wrote an instance");

    drop(serving(&snapshot, &[("STAGEMAN_KEY", KEY)]));
    let again = std::fs::read_to_string(&snapshot).expect("it is still there");

    // Byte equality is deliberately not a claim about snapshots in general:
    // sealing consumes a fresh nonce per credential per write, so two snapshots
    // of the same configured state differ. With nothing configured there is
    // nothing to seal, which is what makes the comparison meaningful here and
    // misleading anywhere else.
    assert_eq!(first, again);
}

// Deliberately absent: a test that a broken container runtime stops the start.
// It lived here and pointed the binary at `/usr/bin/false`, which discovery
// took away — there is no longer any way to tell this process where to look,
// which is the whole of what
// `docs/decisions/0023-the-container-runtime-is-discovered-once.md` chose. The
// check it made is not lost: it is `a_runtime_that_runs_and_refuses_is_not_usable`
// in the agent crate, against the mechanism rather than through the binary,
// which is where it could always have been.

#[test]
fn a_start_without_a_key_says_which_variable_is_missing() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[]);

    assert!(!finished.status.success());
    assert!(
        String::from_utf8_lossy(&finished.stderr).contains("STAGEMAN_KEY"),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
}

/// A wrong key must be refused rather than half-read, and the message must not
/// repeat what it was given.
#[test]
fn a_key_that_is_not_key_material_is_refused_without_echoing_it() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[("STAGEMAN_KEY", "far-too-short")]);

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("STAGEMAN_KEY"), "{said}");
    assert!(!said.contains("far-too-short"), "it echoed the key: {said}");
}

/// The whole of what this first piece of the dashboard claims: a page, served,
/// with real state already rendered into it.
///
/// Asserted against the HTML rather than the route below, because a page that
/// arrives empty and fills itself in afterwards would pass a test of the route
/// and fail the claim. There is no client bundle here — `just check` builds no
/// wasm — so what this proves is the server-rendered half, which is the half
/// that has to be right for the other one to have anything to hydrate.
#[test]
fn the_dashboard_arrives_with_the_instance_already_on_it() {
    let (_kept, snapshot) = scratch();
    let watched = watching("aviary", "https://example.invalid/aviary");
    drop(Store::create(snapshot.clone(), key(), watched).expect("it can write"));

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let page = running.get("/");

    assert!(page.contains("200 OK"), "{page}");
    assert!(
        page.contains("aviary"),
        "the page should name the project: {page}"
    );
    assert!(
        page.contains("example.invalid/aviary"),
        "the page should name the repository: {page}"
    );
}

/// The route exists on its own, at the path it says it does.
///
/// Worth a test separate from the page: the server function is the mechanism
/// every later screen reads through, and a page that renders correctly says
/// nothing about whether the client can call the same thing again.
#[test]
fn the_route_the_page_reads_through_answers_on_its_own() {
    let (_kept, snapshot) = scratch();
    let watched = watching("aviary", "https://example.invalid/aviary");
    drop(Store::create(snapshot.clone(), key(), watched).expect("it can write"));

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let answer = running.get("/api/instance");

    assert!(answer.contains("200 OK"), "{answer}");
    assert!(answer.contains("aviary"), "{answer}");
}

/// `docs/conventions.md` §4 asks that secrets never render, and this is where
/// that stops being about `Debug` and starts being about the network.
///
/// The instance behind both of these holds an agent credential. Neither the
/// page nor the route has any field to put it in — see
/// `docs/decisions/0022-the-browser-never-sees-the-domain.md` — so this test
/// passes by construction today, which is exactly why it is worth writing: the
/// construction is what a later field would change, and nothing else would
/// notice.
#[test]
fn nothing_served_carries_a_credential() {
    let (_kept, snapshot) = scratch();
    let watched = watching("aviary", "https://example.invalid/aviary");
    drop(Store::create(snapshot.clone(), key(), watched).expect("it can write"));

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    for served in [running.get("/"), running.get("/api/instance")] {
        assert!(
            !served.contains("not-a-real-credential"),
            "a credential reached the browser: {served}"
        );
    }
}

/// Where an instance goes when nobody says, which is the ordinary case.
///
/// `HOME` is pointed at a scratch directory rather than trusted, because the
/// whole point of this test is a path derived from the machine — and a test
/// that derived the real one would write to whoever ran it. That also makes
/// the assertion portable: exactly which directory under a home is the
/// platform's business, and restating its answer here would be reimplementing
/// it rather than checking it.
#[test]
fn an_instance_goes_somewhere_sensible_when_nobody_says_where() {
    let home = tempfile::tempdir().expect("a temporary directory");

    let running = started(&[
        ("STAGEMAN_KEY", KEY.to_owned()),
        ("HOME", home.path().to_string_lossy().into_owned()),
        // Honoured ahead of `HOME` where it applies, so a machine that has one
        // set would otherwise send this test to the real directory.
        (
            "XDG_DATA_HOME",
            home.path().join("data").to_string_lossy().into_owned(),
        ),
    ]);

    let instance = running.instance();
    assert!(
        instance.starts_with(home.path()),
        "it should have kept the instance under the home it was given: {}",
        instance.display()
    );
    assert!(
        instance.exists(),
        "it should have created the file and the directory holding it: {}",
        instance.display()
    );
}

/// The override still overrides, because a second instance on one machine and
/// a test that must not touch the real one both need it.
#[test]
fn saying_where_the_instance_goes_still_wins() {
    let (_kept, snapshot) = scratch();

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    assert_eq!(running.instance(), snapshot);
}

/// The startup summary distinguishes a build with a browser bundle from one
/// without, and both are ordinary.
///
/// `DIOXUS_PUBLIC_PATH` is what the Dioxus tooling sets, so pointing it at a
/// directory is the same thing a real bundle does rather than a test-only
/// door. Without it, a `cargo` build has no bundle at all — which is the case
/// every other test here runs in, and is asserted first so that this test
/// fails if the two ever stop differing.
#[test]
fn a_build_says_whether_it_has_a_browser_half() {
    let (_kept, snapshot) = scratch();
    let bundle = tempfile::tempdir().expect("a temporary directory");

    let without = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    assert!(
        without.said.contains("client     not built"),
        "{}",
        without.said
    );
    drop(without);

    let with = serving(
        &snapshot,
        &[
            ("STAGEMAN_KEY", KEY),
            ("DIOXUS_PUBLIC_PATH", &bundle.path().to_string_lossy()),
        ],
    );

    assert!(
        with.said.contains(&*bundle.path().to_string_lossy()),
        "it should name the bundle it found: {}",
        with.said
    );
}

/// A project's running count is not its job count.
///
/// The two were the same in every other fixture here, which is the shape of
/// thing that lets a comparison be inverted without a test noticing — and
/// mutation testing duly noticed that it could be.
///
/// **Neither job is running, and that is not a shortcut.** A fixture cannot
/// contain a running one: startup reconciles what the instance believes
/// against what the runtime actually has, and a job believed to be running
/// with no container is recorded as failed before anything serves a page. So
/// the discriminating case is two finished jobs — nought of two — and
/// inverting the comparison says two of two.
#[test]
fn the_dashboard_counts_running_jobs_rather_than_all_of_them() {
    let (_kept, snapshot) = scratch();
    let mut state = watching("aviary", "https://example.invalid/aviary");
    let project = state.projects.values_mut().next().expect("the project");
    project.jobs.insert(
        JobId::from_uuid(uuid::Uuid::from_u128(1)),
        job(Progress::Completed),
    );
    project.jobs.insert(
        JobId::from_uuid(uuid::Uuid::from_u128(2)),
        job(Progress::Failed("it did not work".to_owned())),
    );
    drop(Store::create(snapshot.clone(), key(), state).expect("it can write"));

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let answer = running.get("/api/instance");

    assert!(answer.contains(r#""running":0"#), "{answer}");
    assert!(answer.contains(r#""jobs":2"#), "{answer}");
}
