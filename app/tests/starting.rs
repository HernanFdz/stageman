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
//! It also covers the routes that binary serves, which is a second concern in
//! one file and is noted as such: `docs/open-questions.md` intends to move
//! whole-flow tests into their own crate, and that move is where these two
//! should part company. Until then they share a harness rather than
//! duplicating one.
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

use std::collections::BTreeMap;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use stageman_core::{
    Access, Agent, AgentConfig, Channel, ChannelConfig, InstanceId, Job, JobId, Key, Kit,
    KitConfig, KitName, NONCE_LEN, Outcome, Platform, Progress, Project, ProjectId, Role, Secret,
    State, Timestamp, Waiting,
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
    // The dashboard is bound before the instance boots, so a refusal these
    // tests look for is reached only if the port was free: whichever is.
    command
        .env_clear()
        .env("IP", "127.0.0.1")
        .env("PORT", "0")
        .env("STAGEMAN_STATE", snapshot);
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

    /// What it said about where its key came from.
    fn key_source(&self) -> String {
        self.said
            .lines()
            .find_map(|line| line.trim().strip_prefix("key        "))
            .expect("it says where the key came from")
            .trim()
            .to_owned()
    }

    /// The whole of the response to one `POST` of JSON, headers included.
    fn post(&self, path: &str, body: &str) -> String {
        self.request(&format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.address,
            body.len()
        ))
    }

    /// The whole of the response to one `GET`, headers included.
    ///
    /// Written by hand rather than with an HTTP client, because `Connection:
    /// close` makes the whole exchange "write a request, read until the peer
    /// stops talking" and a dependency would buy nothing. Headers are kept in
    /// the returned text on purpose: a test asserting on a status line should
    /// not have to take this function's word for it.
    ///
    /// **A reset after the payload counts as the end**, which is the one piece
    /// of this that is not obvious. Under `Connection: close` the response is
    /// over when the peer closes, and whether that arrives as an orderly
    /// shutdown or a reset is a detail of the kernel and the timing rather than
    /// anything about the response — `read_to_end` calls the second one an
    /// error, and continuous integration duly produced one where this machine
    /// never has. Tolerating it hides nothing: a reset that arrived *early*
    /// leaves a truncated response, and the assertions in the tests below then
    /// fail against the partial text, which says far more than
    /// `ConnectionReset` did.
    fn get(&self, path: &str) -> String {
        self.request(&format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address
        ))
    }

    /// Opens a `GET` and hands back the socket with only the request sent,
    /// for a response that does not end: the caller reads what it waits for.
    fn opened(&self, path: &str) -> TcpStream {
        let mut connection = TcpStream::connect(&self.address).expect("the dashboard accepts");
        connection
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: {}\r\n\r\n", self.address).as_bytes())
            .expect("the request is sent");
        connection
    }

    /// Writes a request and reads everything the server says back.
    fn request(&self, request: &str) -> String {
        let mut connection = TcpStream::connect(&self.address).expect("the dashboard accepts");
        connection
            .write_all(request.as_bytes())
            .expect("the request is sent");

        let mut response = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match connection.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => response.extend_from_slice(chunk.get(..read).unwrap_or_default()),
                Err(reset) if reset.kind() == io::ErrorKind::ConnectionReset => break,
                Err(interrupted) if interrupted.kind() == io::ErrorKind::Interrupted => {}
                Err(failure) => panic!("the response did not arrive: {failure}"),
            }
        }
        assert!(!response.is_empty(), "the connection closed saying nothing");
        String::from_utf8_lossy(&response).into_owned()
    }
}

/// Reads from an open response until `needle` has arrived, or gives up
/// after the same patience a start is given.
fn until(connection: &mut TcpStream, needle: &str) -> String {
    connection
        .set_read_timeout(Some(PATIENCE))
        .expect("a timeout is set");
    let mut seen = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !String::from_utf8_lossy(&seen).contains(needle) {
        match connection.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => seen.extend_from_slice(chunk.get(..read).unwrap_or_default()),
            Err(failure) => panic!(
                "waiting for {needle:?}: {failure}; seen so far: {}",
                String::from_utf8_lossy(&seen)
            ),
        }
    }
    String::from_utf8_lossy(&seen).into_owned()
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
        // Whichever port is free, for the job endpoint too. Every test here
        // runs a real binary, and a fixed port would have them contending with
        // each other and with whatever instance the operator is running — and
        // any one of them that leaks would hold it. That is not hypothetical:
        // a leaked mutation-testing process held this port and a real daemon
        // quietly could not bind it, so a foreman talked to a zombie.
        .env("STAGEMAN_JOB_PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in variables {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("the binary runs");
    let stdout = child.stdout.take().expect("its output was piped");
    // Killed here rather than left to `Serving`'s drop, because there is no
    // `Serving` yet: dropping a `Child` does not stop the process it names, so
    // a start that never announces would otherwise leak a serving binary per
    // failed test — and a suite that leaks one per failure takes longer to fail
    // than mutation testing is willing to wait, which is how this was found.
    let (said, address) = match watch(stdout) {
        Ok(seen) => seen,
        Err(why) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{why}");
        }
    };
    Serving {
        child,
        said,
        address,
    }
}

/// How long a start is given to say where it is listening.
///
/// A backstop against a process that starts and never speaks, not a
/// measurement of anything: the whole of this suite runs in about a second on
/// continuous integration, so a single start is a fraction of that and this is
/// two orders of magnitude more than it needs.
///
/// Kept small because it is spent more than once. Eleven tests here wait on a
/// start, and a change that stops any start printing makes every one of them
/// wait the full time — serially, on a machine with fewer cores than tests.
/// That is what `MUTANT_TIMEOUT` in `xtask` is sized against, and keeping this
/// modest is the other half of the same arrangement.
const PATIENCE: Duration = Duration::from_secs(5);

/// Reads a starting binary's output until it names the address it took.
///
/// Answers rather than panicking, so the caller still owns the child and can
/// stop it. Panicking here would abandon a process that is serving happily and
/// simply never said so.
///
/// Read on another thread with a deadline, which is not ceremony: a blocking
/// read of a serving process's output has no end of file to reach, so a start
/// that prints nothing hangs rather than fails. Mutation testing found this by
/// deleting the line that prints the address and watching the suite time out
/// instead of go red.
///
/// **The thread keeps reading after it has what it came for**, and that is
/// load-bearing rather than tidy. Returning early drops the pipe, and the
/// child's next `println!` then writes to a pipe with no reader — which fails,
/// and which Rust's printing macros turn into a panic. The child dies, and a
/// client that has already connected gets a closed socket with nothing on it.
/// That is not hypothetical: it is what continuous integration failed with,
/// twice, on a machine fast enough to win the race that this one loses.
///
/// End of file before the address means it exited instead of serving, and the
/// output it did produce is the only evidence of why — so it is reported
/// rather than swallowed.
fn watch(stdout: ChildStdout) -> Result<(String, String), String> {
    const MARKER: &str = "dashboard  http://";

    let (found, arrived) = mpsc::channel();
    std::thread::spawn(move || {
        let mut said = String::new();
        let mut reported = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            said.push_str(&line);
            said.push('\n');
            if !reported && let Some((_, address)) = line.split_once(MARKER) {
                let _ = found.send(Some((said.clone(), address.trim().to_owned())));
                reported = true;
            }
        }
        if !reported {
            let _ = found.send(None);
        }
    });

    match arrived.recv_timeout(PATIENCE) {
        Ok(Some(listening)) => Ok(listening),
        Ok(None) => Err("it stopped without ever saying where it was listening".to_owned()),
        Err(waited) => Err(format!(
            "it started and never said where it was listening: {waited}"
        )),
    }
}

/// One job, in whatever state the caller needs it.
fn job(progress: Progress) -> Job {
    let mut job = Job::new(
        Kit::defaults(Agent::Claude),
        "because a test said so".to_owned(),
        "do the thing".to_owned(),
        Timestamp::UNIX_EPOCH,
        Secret::new(JOB_WARRANT.to_owned()),
    );
    job.progress = progress;
    job
}

/// A job's warrant: the fourth kind of credential a project's record holds,
/// and the newest, so the browser test below has to know it.
const JOB_WARRANT: &str = "not-a-real-warrant";

/// A channel credential, distinct from the agent's so that a test finding one
/// where it should not be can say which it was.
const CHANNEL_CREDENTIAL: &str = "not-a-real-channel-credential";

/// The credential that opens an event stream rather than posting.
///
/// Distinct from the one above so that the browser test can say which escaped.
/// This one never enters a container either, so it is the credential with the
/// fewest legitimate places to appear — see
/// `docs/decisions/0029-a-reply-is-routed-by-its-thread.md`.
const LISTEN_CREDENTIAL: &str = "not-a-real-listening-credential";

/// The value of one of the project's own variables.
///
/// The third kind of credential a project holds, and the one with the least
/// structure — this project never reads it, so nothing but the browser test
/// below would notice it escaping. See
/// `docs/decisions/0046-a-projects-variables-are-carried-never-read.md`.
const VARIABLE_VALUE: &str = "not-a-real-third-party-key";

/// An instance with one project in it, which is the smallest state that puts
/// anything on the dashboard.
///
/// It binds a channel and holds a variable, neither of which the dashboard
/// reads. That is the point: the project below is the fixture the credential
/// test runs against, and one holding only an agent credential would have
/// stopped covering the second kind the moment channels arrived and the third
/// the moment variables did.
fn watching(name: &str, repository: &str) -> State {
    State {
        apps: std::collections::BTreeMap::new(),
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
                repository: stageman_core::RepositoryAddress::parse(repository)
                    .expect("an address on the platform"),
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: BTreeMap::from([(
                    KitName::new("Claude").expect("a name"),
                    KitConfig::defaults(Agent::Claude),
                )]),
                access: BTreeMap::new(),
                channels: BTreeMap::from([(
                    Channel::Slack,
                    ChannelConfig {
                        credential: Secret::new(CHANNEL_CREDENTIAL.to_owned()),
                        listen_credential: Secret::new(LISTEN_CREDENTIAL.to_owned()),
                    },
                )]),
                variables: BTreeMap::from([(
                    stageman_core::VariableName::new("STRIPE_API_KEY").expect("a deliverable name"),
                    stageman_core::Variable {
                        value: Secret::new(VARIABLE_VALUE.to_owned()),
                        note: "the payment provider, in test mode".to_owned(),
                    },
                )]),
                jobs: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
                brief: String::new(),
                watched: std::collections::BTreeSet::new(),
                foreman_room: None,
            },
        )]),
    }
}

/// Puts a state on the disk, sealed as the instance would seal it, for the
/// binary to open.
///
/// Written here rather than through the binary, because what these tests are
/// about is what a start does with a file that already exists.
fn written(snapshot: &Path, state: &State) {
    written_as(snapshot, state, None);
}

/// The same, naming the instance the file belongs to, for a test that has to
/// label containers as that instance's before the binary wakes.
fn written_as(snapshot: &Path, state: &State, instance: Option<InstanceId>) {
    let mut fresh = (1_u8..).map(|n| [n; NONCE_LEN]);
    let mut nonces = || fresh.next().expect("fewer credentials than a byte counts");
    let mut sealed = state
        .seal(&key(), &mut nonces)
        .expect("a well-formed state seals");
    sealed.instance = instance;
    let encoded = serde_json::to_vec_pretty(&sealed).expect("a snapshot encodes");
    std::fs::write(snapshot, encoded).expect("the snapshot is written");
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

    // Reaching the address line is most of the assertion — `serving` fails
    // loudly if the process stops before printing one — so what is left to
    // check is that it got there having asked nothing and written an instance
    // it invented the whole of.
    assert!(said.contains("stageman is running"), "{said}");
    assert!(snapshot.exists(), "it should have written an instance");
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

/// Asking what it is needs nothing at all.
///
/// No key, no instance, no container runtime — and that is the property rather
/// than an incidental one. A binary is asked what it is precisely when
/// something is wrong with the machine it is on, so an answer that required
/// the machine to be working would be useless exactly when it was wanted.
/// `env_clear` here is the whole test.
#[test]
fn it_says_what_it_is_without_needing_anything() {
    let finished = Command::new(env!("CARGO_BIN_EXE_stageman"))
        .env_clear()
        .arg("--version")
        .output()
        .expect("the binary runs");

    assert!(
        finished.status.success(),
        "it should answer and exit cleanly: {}",
        String::from_utf8_lossy(&finished.stderr),
    );
    let said = String::from_utf8_lossy(&finished.stdout);
    // One labelled fact per line, in the shape the startup block uses, so the
    // two read as one thing rather than as two formats for the same facts.
    for line in said.lines() {
        assert!(line.starts_with("  "), "not a labelled line: {line:?}");
    }
    // The gate builds no release, so what it must say is that it is not one —
    // and it must still name what it was built for, which every build knows.
    assert!(said.contains("not a release build"), "{said}");
    assert!(said.contains("target"), "{said}");
    assert!(
        said.contains(std::env::consts::ARCH),
        "it should name the target it was built for: {said}",
    );
}

/// A start with no key generates one, and the next start reuses it.
///
/// Both halves in one test on purpose: generating is only correct if it
/// happens exactly once, and a test that only checked the first start would
/// pass just as happily on an instance that minted a fresh key every time and
/// therefore lost everything on every restart. See
/// `docs/decisions/0037-the-instance-key-is-generated-on-first-run.md`.
///
/// `HOME` is pointed at a scratch directory for the reason
/// `an_instance_goes_somewhere_sensible_when_nobody_says_where` does it: this
/// is a path derived from the machine, and a test that derived the real one
/// would write a key into the home of whoever ran it.
#[test]
fn a_start_with_no_key_generates_one_and_the_next_start_keeps_it() {
    let (_kept, snapshot) = scratch();
    let home = tempfile::tempdir().expect("a temporary directory");
    let elsewhere = &[
        ("HOME", home.path().to_string_lossy().into_owned()),
        // Honoured ahead of `HOME` where it applies, so a machine that has one
        // set would otherwise send this test to the real directory.
        (
            "XDG_CONFIG_HOME",
            home.path().join("config").to_string_lossy().into_owned(),
        ),
        ("STAGEMAN_STATE", snapshot.to_string_lossy().into_owned()),
    ];

    let first = started(elsewhere);
    let generated = first.key_source();
    assert!(
        generated.contains("generated"),
        "the first start should have minted one: {generated}"
    );
    let path = PathBuf::from(
        generated
            .split(" (generated")
            .next()
            .expect("the line names a path"),
    );
    assert!(
        path.starts_with(home.path()),
        "the key should be under the home it was given: {}",
        path.display()
    );
    assert!(
        path.exists(),
        "it should have written one: {}",
        path.display()
    );
    let written = std::fs::read_to_string(&path).expect("the key is readable");
    drop(first);

    let second = started(elsewhere);
    let kept = second.key_source();
    assert!(
        !kept.contains("generated"),
        "the second start should have reused it: {kept}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("the key is still readable"),
        written,
        "the second start rewrote the key, which would strand the instance"
    );
}

/// A key file readable by anybody but its owner is worse than the variable it
/// replaced.
///
/// Nothing can keep a key from another process running as this user — 0037 is
/// explicit about that — but a mode is the difference between that and every
/// account on a shared machine.
#[cfg(unix)]
#[test]
fn a_generated_key_is_not_readable_by_anybody_else() {
    use std::os::unix::fs::PermissionsExt as _;

    let (_kept, snapshot) = scratch();
    let home = tempfile::tempdir().expect("a temporary directory");

    let running = started(&[
        ("HOME", home.path().to_string_lossy().into_owned()),
        (
            "XDG_CONFIG_HOME",
            home.path().join("config").to_string_lossy().into_owned(),
        ),
        ("STAGEMAN_STATE", snapshot.to_string_lossy().into_owned()),
    ]);

    let path = PathBuf::from(
        running
            .key_source()
            .split(" (generated")
            .next()
            .expect("the line names a path"),
    );
    let mode = std::fs::metadata(&path)
        .expect("the key is there")
        .permissions()
        .mode();

    assert_eq!(
        mode & 0o077,
        0,
        "the key is readable beyond its owner: {mode:o}"
    );
}

/// The variable still wins, because a service manager passing a secret in is
/// the case it exists for.
#[test]
fn saying_what_the_key_is_still_wins() {
    let (_kept, snapshot) = scratch();

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    assert_eq!(running.key_source(), "STAGEMAN_KEY");
}

/// A wrong key must be refused rather than half-read, and the message must not
/// repeat what it was given.
#[test]
fn a_key_that_is_not_key_material_is_refused_without_echoing_it() {
    let (_kept, snapshot) = scratch();

    let finished = run(&snapshot, &[("STAGEMAN_KEY", "far-too-short")]);

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("key"), "{said}");
    assert!(!said.contains("far-too-short"), "it echoed the key: {said}");
}

/// A file that exists and cannot be read is refused as such, rather than
/// treated as a first run and written over.
///
/// A directory where the file should be is the cheapest unreadable file
/// there is, and the failure it must not produce is the quiet one: an
/// instance that could not be read, replaced by an empty one that could.
#[test]
fn an_instance_that_cannot_be_read_is_not_mistaken_for_a_first_run() {
    let (kept, _) = scratch();

    let finished = run(&kept.path().to_path_buf(), &[("STAGEMAN_KEY", KEY)]);

    assert!(!finished.status.success());
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(said.contains("could not be read"), "{said}");
}

/// Two instances started from nothing are two instances.
///
/// The identity is minted from the seed the daemon draws for the instance,
/// and a seed that was not drawn would give every fresh instance the same
/// one — which is how one development instance's sweep would come to remove
/// the real one's containers. Read off the files, because the identity is
/// the one thing on them the state does not carry.
#[test]
fn two_instances_started_from_nothing_are_told_apart() {
    let (_one_kept, one) = scratch();
    let (_two_kept, two) = scratch();
    drop(serving(&one, &[("STAGEMAN_KEY", KEY)]));
    drop(serving(&two, &[("STAGEMAN_KEY", KEY)]));

    let identity = |path: &Path| {
        let text = std::fs::read_to_string(path).expect("it wrote an instance");
        let snapshot: serde_json::Value = serde_json::from_str(&text).expect("it is JSON");
        snapshot
            .get("instance")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .expect("an instance names itself")
    };
    assert_ne!(identity(&one), identity(&two));
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
    let watched = watching("aviary", "https://github.com/example/aviary");
    written(&snapshot, &watched);

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let page = running.get("/");

    assert!(page.contains("200 OK"), "{page}");
    assert!(
        page.contains("aviary"),
        "the page should name the project: {page}"
    );
    assert!(
        page.contains("example/aviary"),
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
    let watched = watching("aviary", "https://github.com/example/aviary");
    written(&snapshot, &watched);

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let answer = running.get("/api/home");

    assert!(answer.contains("200 OK"), "{answer}");
    assert!(answer.contains("aviary"), "{answer}");
}

/// `docs/conventions.md` §4 asks that secrets never render, and this is where
/// that stops being about `Debug` and starts being about the network.
///
/// The instance behind both of these holds an agent credential and a channel's.
/// Neither the page nor the route has any field to put either in — see
/// `docs/decisions/0022-the-browser-never-sees-the-domain.md` — so this test
/// passes by construction today, which is exactly why it is worth writing: the
/// construction is what a later field would change, and nothing else would
/// notice.
#[test]
fn nothing_served_carries_a_credential() {
    let (_kept, snapshot) = scratch();
    let mut watched = watching("aviary", "https://github.com/example/aviary");
    // A job too, so that its page below renders one: its warrant is the
    // newest credential a record holds, and the one a page about a job
    // would be the first to show.
    watched
        .projects
        .get_mut(&ProjectId::from_uuid(uuid::Uuid::nil()))
        .expect("the project")
        .jobs
        .insert(
            JobId::from_uuid(uuid::Uuid::from_u128(7)),
            job(Progress::Idle(Waiting::Asked)),
        );
    written(&snapshot, &watched);

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    for served in [
        running.get("/"),
        running.get("/api/home"),
        running.get("/api/instance"),
        running.get("/projects/00000000-0000-0000-0000-000000000000/settings"),
        running.get("/projects/00000000-0000-0000-0000-000000000000/jobs/00000000-0000-0000-0000-000000000007"),
    ] {
        for secret in [
            VARIABLE_VALUE,
            "not-a-real-credential",
            CHANNEL_CREDENTIAL,
            LISTEN_CREDENTIAL,
            JOB_WARRANT,
        ] {
            assert!(
                !served.contains(secret),
                "a credential reached the browser: {served}"
            );
        }
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

// The startup summary used to distinguish a build with a browser bundle from
// one without, and a test here asserted both halves. Both are gone: what ships
// always carries its own, so the line said the expected thing every time it
// was read, and `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`
// is what made that true.
//
// The states still exist — an ordinary `cargo build` carries nothing — and are
// now a warning instead, which says the same thing only when it is worth
// saying. That is deliberately *not* covered here: this harness reads a
// running process's standard output and never its standard error, so covering
// it would mean teaching it to read both. Recorded rather than left as a gap
// nobody chose.

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
fn the_dashboard_counts_working_jobs_rather_than_all_of_them() {
    let (_kept, snapshot) = scratch();
    let mut state = watching("aviary", "https://github.com/example/aviary");
    let project = state.projects.values_mut().next().expect("the project");
    project.jobs.insert(
        JobId::from_uuid(uuid::Uuid::from_u128(1)),
        job(Progress::Idle(Waiting::Silent)),
    );
    project.jobs.insert(
        JobId::from_uuid(uuid::Uuid::from_u128(2)),
        job(Progress::Idle(Waiting::Failed(
            "it did not work".to_owned(),
        ))),
    );
    written(&snapshot, &state);

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let answer = running.get("/api/home");

    assert!(answer.contains(r#""working":0"#), "{answer}");
    assert!(answer.contains(r#""jobs":2"#), "{answer}");
}

/// The first page arrives with its three regions and the projects on it.
///
/// The fixture's two jobs are idle and have no container, so the waking
/// sweep retires them as lost before anything serves a page — which is why
/// nothing needs a person here, and why the count of jobs is still two. What
/// goes under *needs you* is pinned where a job can be idle without a
/// runtime, in the instance's own tests; this checks the page and the route
/// agree about the instance a binary actually started from.
#[test]
fn the_first_page_arrives_with_its_regions_and_the_projects() {
    let (_kept, snapshot) = scratch();
    let mut state = watching("aviary", "https://github.com/example/aviary");
    let project = state.projects.values_mut().next().expect("the project");
    project.jobs.insert(
        JobId::from_uuid(uuid::Uuid::from_u128(1)),
        job(Progress::Idle(Waiting::Silent)),
    );
    project.jobs.insert(
        JobId::from_uuid(uuid::Uuid::from_u128(2)),
        job(Progress::Idle(Waiting::Failed(
            "it did not work".to_owned(),
        ))),
    );
    written(&snapshot, &state);

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    let home = running.get("/api/home");
    assert!(home.contains(r#""needs_you":[]"#), "{home}");
    assert!(home.contains(r#""working":[]"#), "{home}");
    assert!(home.contains(r#""jobs":2"#), "{home}");

    let page = running.get("/");
    for region in ["Needs you", "Working now", "Projects", "aviary"] {
        assert!(page.contains(region), "no {region} on the page: {page}");
    }
}

/// An agent a project still names cannot be forgotten.
///
/// The guard that matters most on the agents screen, and the one no unit test
/// can reach: it lives in a route, and what is being checked is that the
/// refusal survives all the way to a status code rather than merely existing
/// in a function. `docs/decisions/0021-an-instance-starts-empty.md` requires
/// it, and mutation testing found it unprotected.
#[test]
fn an_agent_a_project_still_names_cannot_be_forgotten() {
    let (_kept, snapshot) = scratch();
    let watched = watching("aviary", "https://github.com/example/aviary");
    written(&snapshot, &watched);

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let refused = running.post("/api/agents/forget", r#"{"agent":"claude"}"#);

    assert!(refused.contains("409"), "it should refuse: {refused}");
    assert!(
        refused.contains("aviary"),
        "it should name what would break: {refused}"
    );

    // Still there, which is the half a status code does not prove.
    let listing = running.get("/api/agents");
    assert!(listing.contains(r#""configured":true"#), "{listing}");
}

/// A credential is accepted, kept, and never handed back.
#[test]
fn a_credential_is_taken_once_and_never_returned() {
    let (_kept, snapshot) = scratch();

    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);
    let saved = running.post(
        "/api/agents/configure",
        r#"{"agent":"claude","credential":"sk-not-a-real-token"}"#,
    );

    assert!(saved.contains(r#""configured":true"#), "{saved}");
    for served in [saved, running.get("/api/agents"), running.get("/agents")] {
        assert!(
            !served.contains("sk-not-a-real-token"),
            "a credential reached the browser: {served}"
        );
    }
}

/// A project is made and changed on pages of their own, at addresses of
/// their own: the settings page shows what the project is, and a new project
/// is that page with nothing filled in — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
#[test]
fn a_project_has_a_settings_page_and_a_new_one_is_that_page_empty() {
    let (_kept, snapshot) = scratch();
    let mut watched = watching("aviary", "https://github.com/example/aviary");
    // Reached with a token, so that the page has a repository to show: a
    // project reaching nothing has nothing chosen, per
    // `docs/decisions/0078-a-repository-is-chosen-from-what-its-access-reaches.md`.
    watched
        .projects
        .get_mut(&ProjectId::from_uuid(uuid::Uuid::nil()))
        .expect("the project")
        .access
        .insert(
            stageman_core::Platform::GitHub,
            stageman_core::Access::Token {
                secret: Secret::new("github_pat_not_a_real_token".to_owned()),
                owner: Some("example".to_owned()),
                expires: None,
            },
        );
    written(&snapshot, &watched);
    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    let fresh = running.get("/projects/new");
    assert!(fresh.contains("200 OK"), "{fresh}");
    assert!(fresh.contains("New project"), "{fresh}");
    assert!(
        !fresh.contains("no project has the identifier"),
        "the static address was taken for an identifier: {fresh}"
    );

    let settings = running.get("/projects/00000000-0000-0000-0000-000000000000/settings");
    assert!(settings.contains("200 OK"), "{settings}");
    assert!(
        settings.contains(r#"value="aviary""#),
        "the name should be in its box: {settings}"
    );
    assert!(settings.contains("example/aviary"), "{settings}");
    assert!(
        // The apostrophe as the server escapes it in text.
        settings.contains("With example&#39;s token."),
        "the access sentence says the shape, and whose the token is, on the server too: {settings}"
    );
    // A text area's value is its text and not an attribute, so a box the
    // server rendered from an attribute alone arrives empty. The kit's
    // description is the one text this helper fills.
    assert!(
        settings.contains("explains what it did.</textarea>"),
        "a text area rendered on the server carries its value as its text: {settings}"
    );
}

/// A job has a page of its own, at an address that keeps its identifier:
/// its reason, what it was told, and its standing — see
/// `docs/decisions/0070-the-dashboard-opens-on-what-needs-a-person.md`.
#[test]
fn a_job_has_a_page_of_its_own() {
    let (_kept, snapshot) = scratch();
    let mut watched = watching("aviary", "https://github.com/example/aviary");
    watched
        .projects
        .get_mut(&ProjectId::from_uuid(uuid::Uuid::nil()))
        .expect("the project")
        .jobs
        // Over already, so that the waking sweep — which finds no container
        // for it and would otherwise record it lost — leaves it as written.
        .insert(JobId::from_uuid(uuid::Uuid::from_u128(7)), {
            let mut done = job(Progress::Retired(Outcome::Done));
            done.pull_requests.insert(7);
            done
        });
    written(&snapshot, &watched);
    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    let page = running.get(
        "/projects/00000000-0000-0000-0000-000000000000/jobs/00000000-0000-0000-0000-000000000007",
    );
    assert!(page.contains("200 OK"), "{page}");
    assert!(page.contains("because a test said so"), "{page}");
    assert!(page.contains("do the thing"), "{page}");
    assert!(
        page.contains("This job is over"),
        "a job that is over offers the mark and no control: {page}"
    );
    assert!(
        page.contains(r#"href="https://github.com/example/aviary""#),
        "the repository is a link where it is an address: {page}"
    );
    assert!(
        page.contains(r#"href="https://github.com/example/aviary/pull/7""#),
        "a pull request it opened is linked by number: {page}"
    );

    let missing = running.get(
        "/projects/00000000-0000-0000-0000-000000000000/jobs/00000000-0000-0000-0000-00000000dead",
    );
    assert!(missing.contains("no job here is"), "{missing}");
}

/// A page learns of change from a tick: a write that lands is told to every
/// open stream, and nothing is told while nothing lands — see
/// `docs/decisions/0071-a-page-learns-of-change-from-a-tick.md`.
///
/// Through the binary, because the stream crosses the forwarder the instance
/// puts in front of the framework, and a forwarder that buffered it would
/// deliver no tick, ever.
#[test]
fn a_write_that_lands_is_told_to_an_open_page_as_a_tick() {
    let (_kept, snapshot) = scratch();
    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    let mut ticks = running.opened("/api/ticks");
    // The head and the opening frame arrive at once, and then nothing: no
    // write has landed since the stream was opened.
    let opening = until(&mut ticks, "open");
    assert!(opening.contains("200 OK"), "{opening}");
    assert!(
        !opening.contains("tick"),
        "a tick before any write: {opening}"
    );

    let saved = running.post(
        "/api/agents/configure",
        r#"{"agent":"claude","credential":"sk-not-a-real-token"}"#,
    );
    assert!(saved.contains("200 OK"), "{saved}");

    let heard = until(&mut ticks, "tick");
    assert!(
        heard.contains("tick"),
        "the write that landed was not told: {heard}"
    );
}

/// The navigation says which screen you are on.
///
/// Checked through the served markup because that is the only place the answer
/// exists: the decision is one comparison inside a component, and a wrong
/// highlight is the kind of thing nobody notices in review and everybody
/// notices in use.
///
/// The first attempt at this test split the page on the link's `href` and read
/// what followed, which quietly matched the document's `<base href="/">` and
/// handed back the rest of the page — so both links were in scope and the test
/// passed whichever way round the comparison went. Mutation testing caught it.
/// Hence taking the anchor tag itself.
#[test]
fn the_navigation_marks_the_screen_being_looked_at() {
    let (_kept, snapshot) = scratch();
    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    let agents = running.get("/agents");
    let here = anchor(&agents, "/agents");
    let elsewhere = anchor(&agents, "/");

    assert!(
        here.contains("font-medium"),
        "the current screen should be marked: {here}"
    );
    assert!(
        !elsewhere.contains("font-medium"),
        "a screen you are not on should not be: {elsewhere}"
    );
}

/// The look is chosen before the page paints, by a script the page carries.
///
/// The server renders no theme of its own: it does not know what the browser
/// holds, and a class it guessed would flash — see
/// `docs/decisions/0072-the-dashboard-has-a-dark-theme.md`. So the page must
/// carry the script that decides, and must not carry a decision.
#[test]
fn the_page_carries_the_theme_script_and_no_theme_of_its_own() {
    let (_kept, snapshot) = scratch();
    let running = serving(&snapshot, &[("STAGEMAN_KEY", KEY)]);

    let page = running.get("/");

    assert!(page.contains("stagemanTheme"), "no theme script: {page}");
    assert!(
        page.contains("prefers-color-scheme"),
        "the script should follow the system: {page}"
    );
    assert!(
        !page.contains(r#"class="dark""#),
        "the server decided a theme: {page}"
    );
}

/// The opening tag of the link to `href`, and nothing after it.
///
/// Bounded at the tag's own `>` on purpose — see the test above for what
/// happens when it is not.
/// The container runtime, found rather than configured.
///
/// Not a breach of the rule `docs/conventions.md` §3 states: that rule is
/// about a daemon which must work under a service manager, and this is a
/// test which must work on a developer's machine — the exemption the agent
/// crate's own container tests take.
fn located_runtime() -> stageman_agent::ContainerRuntime {
    let located = Command::new("sh")
        .args(["-c", "command -v docker"])
        .output()
        .expect("looking for a container runtime");
    let path = String::from_utf8(located.stdout).expect("a runtime path is text");
    stageman_agent::ContainerRuntime::new(PathBuf::from(path.trim()))
}

/// Runs one runtime command to its end, inheriting this process's
/// environment — the runtime needs its own configuration — with what a
/// caller adds on top, and with text on its standard input where one is
/// given.
fn runtime_says(
    runtime: &Path,
    arguments: &[String],
    added: &[(&str, &str)],
    stdin: Option<&str>,
) -> Output {
    let mut command = Command::new(runtime);
    command
        .args(arguments)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in added {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("the runtime runs");
    if let Some(text) = stdin {
        let mut writing = child.stdin.take().expect("its input was piped");
        writing
            .write_all(text.as_bytes())
            .expect("the input is written");
        drop(writing);
    }
    child.wait_with_output().expect("the runtime finishes")
}

fn words(arguments: &[&str]) -> Vec<String> {
    arguments.iter().map(|word| (*word).to_owned()).collect()
}

/// The port the tools are served on for the container test below: fixed,
/// because the wrapper written into a container names the port, and the
/// binary does not print the one it took. Unusual, so that it collides with
/// nothing an operator's own daemon or the other recipes take.
const WRAPPER_TOOLS_PORT: &str = "47116";

/// Two projects, each holding a token of its own and one idle job holding a
/// warrant of its own: the state the container test below runs the binary
/// on, and what it hands back to name each job's container and check each
/// job's answer.
fn two_projects_each_with_a_job() -> (State, Vec<(JobId, &'static str, &'static str)>) {
    let mut state = State {
        apps: BTreeMap::new(),
        agents: BTreeMap::from([(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new("not-a-real-credential".to_owned()),
            },
        )]),
        projects: BTreeMap::new(),
    };
    let jobs = vec![
        (
            JobId::from_uuid(uuid::Uuid::from_u128(11)),
            "warrant-of-the-job-on-one",
            "ghp-not-a-real-token-of-one",
        ),
        (
            JobId::from_uuid(uuid::Uuid::from_u128(12)),
            "warrant-of-the-job-on-the-other",
            "ghp-not-a-real-token-of-the-other",
        ),
    ];
    for ((job, warrant, token), (n, name)) in jobs.iter().zip([(1_u128, "one"), (2, "other")]) {
        let mut recorded = Job::new(
            Kit::defaults(Agent::Claude),
            "a test said so".to_owned(),
            "fetch your credential".to_owned(),
            Timestamp::UNIX_EPOCH,
            Secret::new((*warrant).to_owned()),
        );
        recorded.progress = Progress::Idle(Waiting::Asked);
        state.projects.insert(
            ProjectId::from_uuid(uuid::Uuid::from_u128(n)),
            Project {
                name: name.to_owned(),
                repository: stageman_core::RepositoryAddress::new("example", name)
                    .expect("an address"),
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: BTreeMap::from([(
                    KitName::new("Claude").expect("a name"),
                    KitConfig::defaults(Agent::Claude),
                )]),
                access: BTreeMap::from([(
                    Platform::GitHub,
                    Access::Token {
                        secret: Secret::new((*token).to_owned()),
                        owner: None,
                        expires: None,
                    },
                )]),
                channels: BTreeMap::new(),
                variables: BTreeMap::new(),
                jobs: BTreeMap::from([(job.clone(), recorded)]),
                attending: stageman_core::Attending::default(),
                brief: String::new(),
                watched: std::collections::BTreeSet::new(),
                foreman_room: None,
            },
        );
    }
    (state, jobs)
}

/// One job's container as the instance would have made it: from the job
/// image, labelled as this instance's, the warrant forwarded from the
/// environment, and left stopped.
fn a_jobs_container(runtime: &Path, image: &str, instance: InstanceId, name: &str, warrant: &str) {
    runtime_says(runtime, &words(&["rm", "--force", name]), &[], None);
    let created = runtime_says(
        runtime,
        &stageman_agent::Command::Create {
            name: name.to_owned(),
            image: image.to_owned(),
            agent: Agent::Claude,
            instance,
            variables: vec!["STAGEMAN_WARRANT".to_owned()],
        }
        .arguments(),
        &[("STAGEMAN_WARRANT", warrant)],
        None,
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
}

/// Starts a container and writes the wrapper into it as the instance does,
/// naming where to fetch from.
fn wrapped(runtime: &Path, name: &str, endpoint: &str) {
    let started = runtime_says(runtime, &words(&["start", name]), &[], None);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let written = runtime_says(
        runtime,
        &stageman_agent::Command::Wrap {
            name: name.to_owned(),
        }
        .arguments(),
        &[],
        Some(&stageman_agent::wrapper(endpoint)),
    );
    assert!(
        written.status.success(),
        "{}",
        String::from_utf8_lossy(&written.stderr)
    );
}

/// Asks the platform's tool, inside a container, for the token it holds —
/// which reaches the tool through the wrapper — presenting the warrant the
/// container was created with, or another in its place.
fn token_through_the_wrapper(runtime: &Path, name: &str, presenting: Option<&str>) -> Output {
    let mut arguments = vec!["exec".to_owned()];
    if let Some(warrant) = presenting {
        arguments.push("--env".to_owned());
        arguments.push(format!("STAGEMAN_WARRANT={warrant}"));
    }
    arguments.extend(words(&[name, "gh", "auth", "token"]));
    runtime_says(runtime, &arguments, &[], None)
}

/// A job's wrapper fetches its own project's credential from the running
/// binary and nothing else's: the isolation test `docs/conventions.md` §4
/// asks for since
/// `docs/decisions/0077-a-repository-is-reached-through-an-app-the-instance-owns.md`,
/// against a real container running the wrapper and a real listener
/// serving the route.
///
/// Two projects, each with a token and a job; each job's container is
/// created as the instance creates one — labelled as this instance's, the
/// warrant forwarded from the environment — and the wrapper is written in
/// as the instance writes it, naming the port the binary is told to take.
/// The platform's tool, asked for its token through the wrapper, prints the
/// project's own; a warrant the instance never minted fails the command
/// loudly rather than running it as nobody, and prints neither; and what
/// stands in the tool's place is the wrapper, with the tool beside it.
///
/// Here rather than beside the agent crate's container tests because the
/// proof needs both halves at once: a container that runs the wrapper, and
/// a daemon that serves the route — which only this crate's harness has.
#[test]
#[ignore = "needs a container runtime and the network; run `just image-handshake`"]
fn a_jobs_wrapper_fetches_its_own_projects_credential_and_is_refused_anothers() {
    let runtime = located_runtime();
    let image = tokio::runtime::Runtime::new()
        .expect("an async runtime")
        .block_on(stageman_agent::build(&runtime, Agent::Claude, Role::Job))
        .expect("the job image builds");
    let instance = InstanceId::from_uuid(uuid::Uuid::from_u128(0x5747_2b2b));
    let (state, jobs) = two_projects_each_with_a_job();
    let (_kept, snapshot) = scratch();
    written_as(&snapshot, &state, Some(instance));
    // Stopped, so that waking finds each job with somewhere to resume and
    // leaves it.
    let containers: Vec<String> = jobs
        .iter()
        .map(|(job, _, _)| stageman_job::container(job))
        .collect();
    for ((_, warrant, _), name) in jobs.iter().zip(&containers) {
        a_jobs_container(runtime.path(), image.as_argument(), instance, name, warrant);
    }

    let running = serving(
        &snapshot,
        &[
            ("STAGEMAN_KEY", KEY),
            ("STAGEMAN_JOB_PORT", WRAPPER_TOOLS_PORT),
        ],
    );
    let endpoint = format!("http://host.docker.internal:{WRAPPER_TOOLS_PORT}/credential");
    for name in &containers {
        wrapped(runtime.path(), name, &endpoint);
    }

    for ((_, _, token), name) in jobs.iter().zip(&containers) {
        let said = token_through_the_wrapper(runtime.path(), name, None);
        assert!(
            said.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&said.stderr),
            running.said
        );
        assert_eq!(
            String::from_utf8_lossy(&said.stdout).trim(),
            *token,
            "its own project's credential, and no other's"
        );
    }

    // A warrant this instance never minted buys nothing, and the command
    // fails loudly rather than running as nobody; none at all is refused
    // before anything is asked.
    let stranger = token_through_the_wrapper(runtime.path(), &containers[0], Some("not-a-warrant"));
    assert!(
        !stranger.status.success(),
        "a stranger was handed something"
    );
    let complained = String::from_utf8_lossy(&stranger.stderr);
    assert!(
        complained.contains("403") && complained.contains("did not hand over a credential"),
        "{complained}"
    );
    let printed = String::from_utf8_lossy(&stranger.stdout);
    for (_, _, token) in &jobs {
        assert!(!printed.contains(token), "{printed}");
    }
    let unwarranted = token_through_the_wrapper(runtime.path(), &containers[0], Some(""));
    assert!(!unwarranted.status.success());
    assert!(
        String::from_utf8_lossy(&unwarranted.stderr).contains("holds no warrant"),
        "{}",
        String::from_utf8_lossy(&unwarranted.stderr)
    );

    // What stands in the tool's place is the wrapper, and the tool is beside it.
    let first_line = runtime_says(
        runtime.path(),
        &words(&[
            "exec",
            &containers[0],
            "head",
            "-c",
            "9",
            "/usr/local/bin/gh",
        ]),
        &[],
        None,
    );
    assert_eq!(String::from_utf8_lossy(&first_line.stdout), "#!/bin/sh");
    let aside = runtime_says(
        runtime.path(),
        &words(&[
            "exec",
            &containers[0],
            "/usr/local/libexec/stageman/gh",
            "--version",
        ]),
        &[],
        None,
    );
    assert!(
        aside.status.success(),
        "the tool itself still runs from where it was moved"
    );

    for name in &containers {
        runtime_says(runtime.path(), &words(&["rm", "--force", name]), &[], None);
    }
    drop(running);
}

fn anchor(page: &str, href: &str) -> String {
    let opening = format!("<a href=\"{href}\"");
    let from = page
        .find(&opening)
        .unwrap_or_else(|| panic!("no link to {href} on the page"));
    let rest = page.get(from..).unwrap_or_default();
    let until = rest.find('>').unwrap_or(rest.len());

    rest.get(..until).unwrap_or_default().to_owned()
}
