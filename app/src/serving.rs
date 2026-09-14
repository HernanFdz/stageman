//! Starting an instance, and serving the dashboard from it.
//!
//! This is the daemon's half of the binary and never reaches the browser. It
//! lives here rather than in `main.rs` because that file now holds two entry
//! points — `dx` builds the same binary for both halves — and an entry point
//! carrying a hundred lines of startup would make it impossible to see at a
//! glance which half you are reading.
//!
//! Serving is plain `axum` around a router Dioxus assembles, rather than
//! `dioxus::serve`, which builds a runtime of its own and never returns. The
//! difference is what `docs/conventions.md` §3 asks for: whatever makes an
//! instance unusable has to fail at startup, with an exit code and a reason,
//! and that is not available inside a function that cannot return.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use dioxus::prelude::{DioxusRouterExt as _, ServeConfig};
use dioxus::server::axum;
use rand::rngs::{StdRng, SysRng};
use rand::{Rng as _, SeedableRng as _};
use stageman_instance::{AppEvent, Instance, Seed, Target};
use stageman_vocabulary::Environment;
use stageman_world::World;

use crate::Dashboard;
use crate::world::{Asking, Performer};

/// How much is reported, when the environment does not say.
const DEFAULT_VERBOSITY: &str = "warn";

/// The variable that overrides how much is reported.
const VERBOSITY_VARIABLE: &str = "STAGEMAN_LOG";

/// An instance could not be started, for a reason the instance itself
/// cannot say: everything it can say, it says as an exit effect.
#[derive(Debug, thiserror::Error)]
enum StartupError {
    /// The address the dashboard would be served on could not be taken.
    #[error("the dashboard cannot listen on {address}")]
    Listen {
        /// What it tried to bind.
        address: SocketAddr,
        /// Why it could not.
        #[source]
        source: io::Error,
    },
    /// Serving stopped on something other than being asked to.
    #[error("serving the dashboard stopped")]
    Serving(#[source] io::Error),
    /// The index could not be put where the framework will read it.
    ///
    /// Fatal rather than a warning, and that is the judgement worth recording:
    /// a binary carrying a browser half and unable to place its index serves a
    /// page that renders and never comes alive, which is almost indistinguishable
    /// from one that works. `docs/conventions.md` §3 puts what an operator can
    /// act on in the dashboard — and this cannot be, because the dashboard is
    /// the thing that would not be working.
    #[error(
        "the browser half could not be placed at {path}.\n  This binary carries \
         one, and the directory it must be written to is not writable.\n  Either \
         run from a directory you can write to, or set DIOXUS_PUBLIC_PATH to one."
    )]
    Bundle {
        /// Where it tried to write.
        path: PathBuf,
        /// Why it could not.
        #[source]
        source: io::Error,
    },
    /// There is no randomness to seed the instance from.
    ///
    /// Refused rather than substituted. A predictable seed is worse than no
    /// seed, because everything unguessable the instance mints comes from it.
    #[error("no source of randomness, so the instance cannot be seeded")]
    NoRandomness,
}

/// Starts an instance and serves its dashboard until the process is stopped.
///
/// The whole of what running stageman means, and the reason this returns an
/// [`ExitCode`] rather than a `Result`: a failure here is the program's last
/// word to whoever ran it, so it is printed in full and turned into a status
/// rather than handed to a caller who has nothing better to do with it.
#[must_use]
pub fn serve() -> ExitCode {
    // Every field on its own line: this is the whole output, so it has the room
    // that the startup block does not.
    //
    // Before the subscriber, the runtime, the instance and the runtime check,
    // because it is a question about this file rather than about this machine
    // — and it has to be answerable on a machine where none of the rest would
    // work. Asking a binary what it is must never require it to be able to run.
    if asked_what_it_is(std::env::args().skip(1)) {
        print!("{}", stageman_instance::release::detailed());
        return ExitCode::SUCCESS;
    }

    // Standard error, which is a real answer for a process somebody started
    // and is watching, and a placeholder for the daemon this becomes — see
    // `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md`.
    //
    // Installed before anything else, and that ordering is load-bearing now
    // that Dioxus is here: `dioxus_logger` installs a subscriber of its own
    // unless one is already set, so being second would mean losing
    // `STAGEMAN_LOG` to a default nobody chose.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env(VERBOSITY_VARIABLE)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_VERBOSITY)),
        )
        .with_writer(io::stderr)
        .init();

    // Everything after this needs one, including every way a start refuses:
    // since
    // `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`
    // the instance says why it will not run by asking the world to exit, and
    // the world performs that here. So not having a runtime is the one failure
    // that has to be reported without one.
    match tokio::runtime::Runtime::new() {
        Ok(runtime) => match runtime.block_on(start()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(failure) => {
                report(&failure);
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            eprintln!("stageman: no async runtime: {failure}");
            ExitCode::FAILURE
        }
    }
}

/// Whether the only thing wanted is what this binary is.
///
/// Hand-rolled rather than parsed, because this is the whole command line
/// there is: stageman takes its configuration from the environment and the
/// dashboard, so a parser would be a dependency in front of one question. Both
/// spellings, because both are what people type.
///
/// Anything else is ignored rather than refused. A binary a service manager
/// starts with an argument nobody meant is better serving than exiting, and
/// there is no argument it could be given that means something else.
///
/// It takes the arguments rather than reading them, because a test cannot
/// choose what a process was started with — and the two spellings and the
/// comparison between them are exactly what is worth asserting.
fn asked_what_it_is(mut arguments: impl Iterator<Item = String>) -> bool {
    arguments.any(|argument| argument == "--version" || argument == "-V")
}

/// Prints a failure and everything underneath it.
///
/// Deliberately not through `tracing`, and the distinction is worth keeping:
/// this is the program's last word to whoever ran it, not a record of
/// something that happened. Routing it through a level would let
/// `STAGEMAN_LOG` silence the reason the process exited, which is the one
/// message that must never be filterable.
///
/// The chain rather than the top line alone: every error here wraps a more
/// specific one, and "the instance could not be opened" without the reason
/// underneath is the shape of message that sends somebody to read the source.
fn report(failure: &StartupError) {
    eprintln!("stageman: {failure}");
    let mut cause: Option<&dyn std::error::Error> = std::error::Error::source(failure);
    while let Some(reason) = cause {
        eprintln!("  caused by: {reason}");
        cause = reason.source();
    }
}

async fn start() -> Result<(), StartupError> {
    // Bound before the instance is constructed, because where the dashboard
    // is served is the one thing the instance is told rather than asks for:
    // a port of zero is an ordinary request for whichever port is free, and
    // the answer is only known here. It is also what makes the address the
    // instance announces a readiness signal: the socket is already accepting
    // by the time the line naming it appears.
    let address = dioxus::cli_config::fullstack_address_or_localhost();
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|source| StartupError::Listen { address, source })?;
    let serving = listener
        .local_addr()
        .map_err(|source| StartupError::Listen { address, source })?;

    // Everything else the instance learns by asking: whether a runtime
    // answers, its key, its file, what was left behind. Every way a start
    // can refuse is an exit effect with its reason, performed by the world.
    let environment: Environment = std::env::vars().collect();
    // The one place a compile-time condition says anything about a platform.
    // Everything downstream decides on the value, so a start can be recorded
    // here and replayed on a machine this is not.
    let (instance, effects) = Instance::boot(seed()?, environment, Target::compiled());
    let (world, events) = World::new();
    let asking = Asking::new(Arc::clone(&world));
    crate::world::adopt(Arc::clone(&asking));
    asking.send(AppEvent::Serving {
        address: serving.to_string(),
        port: serving.port(),
    });

    stageman_world::run(
        instance,
        effects,
        world,
        Arc::new(Performer::new(asking)),
        events,
    );

    // Three states, and which one holds is a question about how this binary
    // was built rather than about how it is configured.
    //
    // *Carrying its own* — what `just build` produces. The index is written
    // where the framework looks, read once, and removed; every other file is
    // served from memory by a route of ours, so the framework is asked for a
    // rendering that serves no static files at all.
    //
    // *A bundle beside it* — what `dx serve` arranges during development, and
    // what a hand-assembled directory looks like. The framework serves it.
    //
    // *Neither* — what `cargo build` produces, and a working thing to run: the
    // page is rendered here and arrives complete, it just does not come alive
    // afterwards.
    let carried = crate::bundle::CARRIED.index();
    let configured = configuration(carried, public_directory())?;

    let beside = public_directory().filter(|path| path.is_dir());
    let router = axum::Router::new();
    let router = if carried.is_some() {
        serving_embedded(router.serve_api_application(configured, Dashboard))
    } else if beside.is_some() {
        router.serve_dioxus_application(configured, Dashboard)
    } else {
        router.serve_api_application(configured, Dashboard)
    };
    // Warned rather than printed, because it is an anomaly rather than a fact:
    // every build that ships carries one. What it produces is a dashboard that
    // renders and never responds, which is the state most easily mistaken for
    // one that works — so it is worth saying, and worth saying only when true.
    if clientless(carried, beside.as_deref()) {
        tracing::warn!(
            "this build carries no browser half and none is beside it — the dashboard \
             will render and never respond. `just build` produces one that does"
        );
    }

    // Outermost, and that is the whole of it: a job's tunnel serves an
    // application somebody else's agent wrote, so it must not pass through the
    // server-function and static-file machinery on its way — a path collision
    // would otherwise decide which of the two answers. Applied last because a
    // layer added last is the one that runs first. See
    // `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`.
    let router = router.layer(axum::middleware::from_fn(crate::tunnel::route));

    axum::serve(listener, router)
        .await
        .map_err(StartupError::Serving)
}

/// The seed the instance draws everything random from.
///
/// Drawn once, here, from the system: the instance takes no entropy of its
/// own, so this is the one place in the process a random number is made for
/// it — see `docs/decisions/0056-the-instance-decides-and-the-world-performs.md`.
/// From the system and never from a clock, because a warrant a container
/// presents is minted from this, and a seed anybody could reconstruct from a
/// start time would let one job forge another's — see
/// `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
fn seed() -> Result<Seed, StartupError> {
    let mut rng = StdRng::try_from_rng(&mut SysRng).map_err(|_| StartupError::NoRandomness)?;
    let mut seed: Seed = [0_u8; 32];
    rng.fill_bytes(&mut seed);
    Ok(seed)
}

/// Whether this build has no browser half anywhere.
///
/// Both halves of the question, and a named function rather than a condition
/// written where it is used: the two sources are not interchangeable, and
/// getting the combination wrong is silent. Warning when only one is absent would fire on every ordinary
/// development build, which is how a warning stops being read.
const fn clientless(carried: Option<&[u8]>, beside: Option<&Path>) -> bool {
    carried.is_none() && beside.is_none()
}

/// The directory the framework resolves a browser half from.
///
/// The same rule Dioxus applies, restated because the function that applies it
/// is private and because it panics on a directory that is not there. Asking
/// first turns "built without a client" from a crash into the ordinary state
/// of anything `cargo build` produces.
///
/// Restating somebody else's rule is drift waiting to happen, and what keeps
/// it honest is that the integration tests run this binary: a Dioxus upgrade
/// that moved the directory would fail them here rather than in production.
///
/// **It is now read for two reasons rather than one**, and that is why it is
/// named for the directory rather than for the bundle that may be in it: it
/// says where a bundle would be found, and it says where this binary must put
/// its own index so the framework will find *that* — see
/// [`materialise`] and
/// `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`. One rule,
/// read twice, so the two can never point at different directories.
fn public_directory() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DIOXUS_PUBLIC_PATH") {
        return Some(PathBuf::from(path));
    }
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|beside| beside.join("public"))
}

/// Builds the serving configuration, putting a carried index in front of it.
///
/// The whole of the arrangement in
/// `docs/decisions/0038-the-browsers-half-lives-in-the-binary.md`, in the order
/// that makes it safe: write the index where the framework looks, build the
/// configuration — which is the one call that reads it — then take it away.
///
/// With nothing carried this is the configuration on its own, which is what
/// every build that is not `just build` produces.
///
/// It takes the directory rather than resolving one, for the reason
/// [`materialise`] does: the gate compiles this binary carrying nothing, so a
/// test can only reach the interesting half by being handed somewhere to put
/// it.
///
/// # Errors
///
/// Fails if there is nowhere to derive a directory from, or the index cannot be
/// written there. Both are refusals rather than a configuration without a
/// client: a binary carrying a browser half that could not place it would serve
/// a page which renders and never responds, and the dashboard is the thing an
/// operator would otherwise be sent to fix it in.
fn configuration(
    carried: Option<&[u8]>,
    directory: Option<PathBuf>,
) -> Result<ServeConfig, StartupError> {
    let Some(index) = carried else {
        return Ok(ServeConfig::new());
    };
    let directory = directory.ok_or_else(|| StartupError::Bundle {
        path: PathBuf::new(),
        source: io::Error::other("this program has no location to derive one from"),
    })?;
    let written = materialise(&directory, index)?;
    let configured = ServeConfig::new();
    withdraw(&written);
    Ok(configured)
}

/// Puts the carried index where the framework will read it, and says where.
///
/// The framework accepts an index only as a path. The one call that takes a
/// parsed index in memory needs a type its own crate does not export, checked
/// in three releases — so this writes the file, and the caller removes it as
/// soon as the configuration has been built. It is on disk for the length of
/// one read.
///
/// Written unconditionally rather than deferring to a file already there.
/// Assets are named by a hash of their contents and this binary's index names
/// the hashes this binary carries, so an index left by some other build would
/// send a browser looking for files nothing here has. Overwriting is the safe
/// direction.
///
/// It takes the directory rather than resolving one, which is what lets a test
/// hand it somewhere harmless. Resolving is [`public_directory`]'s job and the
/// caller's to ask for, so the two questions — *where does the framework look*
/// and *can this be written there* — are answered in different places.
///
/// # Errors
///
/// Fails if the directory cannot be made or the file cannot be written.
fn materialise(directory: &Path, index: &[u8]) -> Result<PathBuf, StartupError> {
    let written = directory.join(crate::bundle::INDEX);
    let failed = |source| StartupError::Bundle {
        path: written.clone(),
        source,
    };
    fs::create_dir_all(directory).map_err(failed)?;
    fs::write(&written, modulepreloaded(index)).map_err(failed)?;
    Ok(written)
}

/// How the bundler asks a browser to preload the browser's half.
///
/// It emits this and then loads the same file with `<script type="module">`,
/// and to a browser those are two different requests: a module is fetched in
/// its own mode, so the preloaded copy never matches the one the page goes on
/// to ask for. Firefox says so — *preloaded with link preload was not used* —
/// having fetched sixty kilobytes twice to find out.
const PRELOADED: &str = r#"rel="preload" as="script""#;

/// What it should have said.
///
/// `modulepreload` is the tag whose fetch matches a module's, so the preload
/// is used rather than raced. `as` goes with it because the relationship
/// already implies the destination, and the `href` and `crossorigin` either
/// side of this are left exactly as they were.
const MODULEPRELOADED: &str = r#"rel="modulepreload""#;

/// The carried index, with the browser's half preloaded as the module it is.
///
/// A substitution over somebody else's generated file, which is worth being
/// plain about rather than burying. [`PRELOADED`] is a literal the bundler
/// writes, so a version of it spelling that differently is a version this
/// quietly stops correcting — and what comes back then is the console line it
/// was written for, on a page that still works. That asymmetry is the whole
/// argument for patching generated output here: the failure of the patch is
/// the state before it.
///
/// Everything else is left alone, deliberately. This is not an HTML rewriter
/// and must not become one — the index is the renderer's input, and a
/// transformation that reflowed it would change what the framework parses.
///
/// Bytes that are not text come back untouched rather than lossily converted.
/// An index this could not read is not one it should be editing, and a build
/// carrying no browser half has no such tag in the first place, so leaving it
/// alone is the ordinary case rather than a failure.
fn modulepreloaded(index: &[u8]) -> Vec<u8> {
    std::str::from_utf8(index).map_or_else(
        |_| index.to_vec(),
        |text| text.replace(PRELOADED, MODULEPRELOADED).into_bytes(),
    )
}

/// Removes what [`materialise`] wrote, so nothing is left on disk.
///
/// Best effort and deliberately silent. The configuration has already been
/// built by the time this runs, so a file that cannot be removed changes
/// nothing about whether the dashboard works — and the next start overwrites
/// it. Refusing to start over an undeletable temporary file would be trading a
/// working instance for tidiness.
///
/// **The directory goes too, but only if it is empty**, and `remove_dir` is
/// what makes that safe to attempt rather than something to reason about
/// first. It is `rmdir` underneath: emptiness and removal are one operation,
/// so there is no window in which a directory tested as empty gains a file
/// before it is taken away. Trying and being refused *is* the check.
///
/// That the refusal is silent is the point rather than laziness. A directory
/// with anything else in it belongs to somebody else — an operator who pointed
/// `DIOXUS_PUBLIC_PATH` at a real bundle, most obviously — and leaving it is
/// the correct outcome, not a failure to report. Only a directory this created
/// and emptied is one nothing else wants, and that one is recreated on the
/// next start anyway.
fn withdraw(written: &Path) {
    drop(fs::remove_file(written));
    if let Some(directory) = written.parent() {
        drop(fs::remove_dir(directory));
    }
}

/// Adds a route per carried file, so the bundle is served from memory.
///
/// One exact route each rather than a wildcard under a prefix. There is no
/// path to normalise and no way to ask for something outside the table, so the
/// class of bug where a crafted path escapes the directory cannot occur — it
/// is not defended against, it is absent.
///
/// The index is skipped: it is the renderer's input rather than a response,
/// and serving the unrendered template would hand back a page that boots
/// without the state the server had already put in it.
/// What each carried file is served as, and under what path.
///
/// Split from registering it so that the two decisions here — which files get
/// a route, and what each is called — can be asserted without building a
/// router or driving one. It takes the table rather than reading the one
/// compiled in, and that is the difference between a test and a tautology: the
/// gate embeds nothing, so a function reading the real table would return
/// nothing and agree with every mutation of itself.
fn routed(carried: crate::bundle::Bundle) -> Vec<(String, &'static str, &'static [u8])> {
    carried
        .entries()
        .iter()
        .filter(|(served, _)| *served != crate::bundle::INDEX)
        .map(|(served, bytes)| {
            (
                format!("/{served}"),
                crate::bundle::content_type(served),
                *bytes,
            )
        })
        .collect()
}

/// Registers one route per entry [`routed`] names.
///
/// Skipped by mutation testing, and untestable rather than untested: what it
/// does beyond `routed` is one framework call per entry, and reaching it needs
/// a binary with a bundle compiled into it — which the gate never builds,
/// because the directory it would come from is empty in a fresh clone. What
/// can be checked is checked, one function up.
#[mutants::skip]
fn serving_embedded(mut router: axum::Router) -> axum::Router {
    for (path, kind, body) in routed(crate::bundle::CARRIED) {
        router = router.route(
            &path,
            axum::routing::get(move || async move {
                ([(axum::http::header::CONTENT_TYPE, kind)], body)
            }),
        );
    }
    router
}

#[cfg(test)]
mod tests {

    /// A browser half from either source is a browser half.
    ///
    /// All four combinations, because the failure is one-sided in both
    /// directions: a condition that warned when either was absent would fire
    /// on every development build until nobody read it, and one that warned
    /// when neither was would never fire at all.
    #[test]
    fn only_a_build_with_no_browser_half_anywhere_is_clientless() {
        use super::clientless;
        use std::path::Path;

        let beside = Path::new("/somewhere/public");

        assert!(clientless(None, None), "nothing carried and nothing beside");
        assert!(
            !clientless(Some(b"<html></html>"), None),
            "carried is enough"
        );
        assert!(!clientless(None, Some(beside)), "beside is enough");
        assert!(!clientless(Some(b"<html></html>"), Some(beside)));
    }

    /// Both spellings ask, and nothing else does.
    ///
    /// Every part of this is worth asserting: that either spelling is enough,
    /// that both are required to *match* rather than to coincide, and that an
    /// ordinary argument is not mistaken for one. A binary that read `--help`
    /// as a version request would exit instead of serving.
    #[test]
    fn only_the_two_spellings_ask_what_it_is() {
        use super::asked_what_it_is;

        let given =
            |arguments: &[&str]| asked_what_it_is(arguments.iter().map(|a| (*a).to_owned()));

        assert!(given(&["--version"]));
        assert!(given(&["-V"]));
        assert!(given(&["--serve", "--version"]), "position does not matter");

        assert!(!given(&[]), "no arguments is not a question");
        assert!(!given(&["--help"]));
        assert!(!given(&["-v"]), "lowercase is not the flag");
        assert!(!given(&["--versions"]));
        assert!(!given(&["version"]));
    }

    /// Nothing of the index survives, the directory it needed included.
    ///
    /// The directory is the half that was missed first time round: the file
    /// went and an empty `public/` stayed beside the binary, which is exactly
    /// the residue this whole arrangement exists to avoid.
    #[test]
    fn withdrawing_takes_the_directory_it_needed_with_it() {
        use super::{materialise, withdraw};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");

        let written = materialise(&at, b"<html></html>").expect("it is writable");
        assert!(at.is_dir(), "it should have made the directory");

        withdraw(&written);

        assert!(
            !written.exists(),
            "the index outlived the read it existed for"
        );
        assert!(
            !at.exists(),
            "an empty directory was left beside the binary"
        );
    }

    /// A directory holding anything else is left exactly as it was.
    ///
    /// The case an operator creates by pointing `DIOXUS_PUBLIC_PATH` at a real
    /// bundle. Removing it would take their files with it, so being refused is
    /// the outcome asked for rather than an error — which is why nothing here
    /// reports one.
    #[test]
    fn a_directory_holding_anything_else_survives() {
        use super::{materialise, withdraw};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");
        std::fs::create_dir_all(&at).expect("the directory is made");
        let theirs = at.join("something-else.js");
        std::fs::write(&theirs, "not ours").expect("their file is written");

        let written = materialise(&at, b"<html></html>").expect("it is writable");
        withdraw(&written);

        assert!(!written.exists(), "ours should still be removed");
        assert!(at.is_dir(), "their directory should have survived");
        assert_eq!(
            std::fs::read_to_string(&theirs).expect("their file is still there"),
            "not ours",
        );
    }

    /// A carried index is written, read, and taken away again.
    ///
    /// Asserted through a sentinel rather than by looking at what comes back,
    /// because the configuration is opaque: a stale index is put there first,
    /// and what proves the whole sequence ran is that it is gone afterwards.
    /// Nothing else could have removed it.
    #[test]
    fn a_carried_index_is_written_read_and_taken_away() {
        use super::configuration;

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");
        std::fs::create_dir_all(&at).expect("the directory is made");
        let stale = at.join("index.html");
        std::fs::write(&stale, "STALE").expect("the sentinel is written");

        configuration(Some(b"<html>ours</html>"), Some(at)).expect("it is configurable");

        assert!(
            !stale.exists(),
            "the index should have been overwritten and then removed",
        );
    }

    /// A binary carrying nothing touches nothing.
    ///
    /// The other half, and the one that keeps `just dev` working: a build with
    /// no bundle of its own must leave a directory that has one alone.
    #[test]
    fn carrying_nothing_leaves_the_directory_alone() {
        use super::configuration;

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let at = elsewhere.path().join("public");
        std::fs::create_dir_all(&at).expect("the directory is made");
        let theirs = at.join("index.html");
        std::fs::write(&theirs, "THEIRS").expect("the sentinel is written");

        configuration(None, Some(at)).expect("it is configurable");

        assert_eq!(
            std::fs::read_to_string(&theirs).expect("it is still there"),
            "THEIRS",
            "a build carrying nothing must not disturb a bundle beside it",
        );
    }

    /// The index is written where it was asked for, with what it was given.
    ///
    /// The whole of what the framework needs from us, and the one step that
    /// touches a disk — so it is worth asserting rather than assuming, and
    /// asserting somewhere harmless rather than beside a real binary.
    #[test]
    fn the_index_is_written_where_the_framework_will_look() {
        use super::materialise;

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let nested = elsewhere.path().join("public");

        let written = materialise(&nested, b"<html>carried</html>").expect("it is writable");

        assert_eq!(written, nested.join("index.html"));
        assert_eq!(
            std::fs::read_to_string(&written).expect("it is there"),
            "<html>carried</html>",
            "the framework would parse whatever this wrote",
        );
    }

    /// The browser's half is preloaded as the module the page then loads.
    ///
    /// Written against the tag the bundler actually emits rather than a
    /// paraphrase of it, because a paraphrase would keep passing after the
    /// real one changed shape — and the substitution missing is silent by
    /// design, so this test is the only thing that would say so.
    #[test]
    fn a_module_is_preloaded_as_a_module() {
        use super::modulepreloaded;

        let emitted = concat!(
            r#"<link rel="preload" as="script" href="/./assets/x-dxh0.js" crossorigin>"#,
            r#"<script type="module" async src="/./assets/x-dxh0.js"></script>"#,
        );

        let corrected =
            String::from_utf8(modulepreloaded(emitted.as_bytes())).expect("text in, text out");

        assert_eq!(
            corrected,
            concat!(
                r#"<link rel="modulepreload" href="/./assets/x-dxh0.js" crossorigin>"#,
                r#"<script type="module" async src="/./assets/x-dxh0.js"></script>"#,
            ),
            "the link should preload a module, and nothing else should move",
        );
    }

    /// An index with nothing to correct is passed through untouched.
    ///
    /// Two of them, and they are different failures rather than the same one
    /// twice: a build carrying no browser half has no such tag, and bytes that
    /// are not text are not an index this has any business editing. Both must
    /// arrive at the framework exactly as they left.
    #[test]
    fn an_index_with_no_such_tag_is_left_alone() {
        use super::modulepreloaded;

        assert_eq!(
            modulepreloaded(b"<html>no client here</html>"),
            b"<html>no client here</html>",
        );
        assert_eq!(
            modulepreloaded(&[0xff, 0xfe, 0x00]),
            &[0xff, 0xfe, 0x00],
            "bytes that are not text are not rewritten lossily",
        );
    }

    /// A directory that cannot be written is a refusal, not an empty success.
    ///
    /// The failure this guards is the expensive one: a binary carrying a
    /// browser half that quietly serves a page which renders and never
    /// responds. Provoked with a path under a file, which cannot be a
    /// directory on any platform.
    #[test]
    fn an_index_that_cannot_be_written_is_a_failure() {
        use super::{StartupError, materialise};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let blocked = elsewhere.path().join("a-file");
        std::fs::write(&blocked, "not a directory").expect("the file is written");

        let refused = materialise(&blocked.join("public"), b"<html></html>");

        assert!(
            matches!(refused, Err(StartupError::Bundle { .. })),
            "expected a refusal, got {refused:?}",
        );
    }

    /// What was written is taken away again.
    ///
    /// The point of writing it at all is that it does not stay, so this is the
    /// assertion that the whole arrangement rests on.
    #[test]
    fn what_was_written_does_not_stay() {
        use super::{materialise, withdraw};

        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let written = materialise(elsewhere.path(), b"<html></html>").expect("it is writable");
        assert!(written.exists());

        withdraw(&written);

        assert!(
            !written.exists(),
            "the index outlived the read it existed for"
        );
        // Removing what is already gone is the outcome asked for rather than a
        // failure, and a second start must not trip over the first.
        withdraw(&written);
    }

    /// The index gets no route, and everything else gets one under its path.
    ///
    /// Vacuous when nothing is embedded, which is the gate's case — the table
    /// is a compile-time input, so no test can put anything in it. What this
    /// catches is the filter inverting, which would serve the unrendered
    /// template and nothing else.
    #[test]
    fn the_index_is_not_routed_and_the_rest_are() {
        use super::routed;

        // A table of this shape rather than the one compiled in. The gate
        // embeds nothing, so asserting against the real table would assert
        // that nothing maps to nothing.
        let carrying = crate::bundle::Bundle::of(&[
            ("index.html", b"<html></html>"),
            ("assets/app-abc.js", b"console.log(1)"),
            ("assets/app-abc.wasm", b"\0asm"),
        ]);

        let routes = routed(carrying);

        assert_eq!(
            routes.len(),
            2,
            "the index must not get a route: {routes:?}"
        );
        let paths: Vec<&str> = routes.iter().map(|(path, ..)| path.as_str()).collect();
        assert!(paths.contains(&"/assets/app-abc.js"), "{paths:?}");
        assert!(paths.contains(&"/assets/app-abc.wasm"), "{paths:?}");
        assert!(!paths.contains(&"/index.html"), "{paths:?}");

        let wasm = routes
            .iter()
            .find(|(path, ..)| path == "/assets/app-abc.wasm")
            .expect("the wasm is routed");
        assert_eq!(
            wasm.1, "application/wasm",
            "a browser refuses a module served as anything else",
        );
        assert_eq!(wasm.2, b"\0asm", "the route serves the bytes it was given");
    }
}
