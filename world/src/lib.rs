//! The world: everything the instance is not, performed.
//!
//! One loop owns the deciding half and steps it, one event at a time, on a
//! task of its own. Everything that happens is an event sent to that loop,
//! and everything it asks for comes back as an effect this crate performs —
//! the generic ones as the mechanism each names, the application's own by
//! handing them to what the entry point supplied. Nothing here decides; see
//! `docs/decisions/0057-the-world-is-generic-and-the-instance-boots-itself.md`.
//!
//! Small enough to be read rather than tested, which is the point of it
//! being a crate of its own: what is here is a channel, a loop, and one
//! performer per mechanism, and none of them holds a domain type.
//!
//! **A write is performed inline and everything else on a task.** A write is
//! the one effect the deciding half waits on before anything outward-facing,
//! so it is performed where the loop can await it and answered in the order
//! it was asked for; everything that takes time runs on a task of its own,
//! and the loop is free to answer the next event.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::Arc;

use stageman_vocabulary::{App, Bytes, Deciding, Effect, Environment, Event, Finished, Named};

/// The way in: events go to the loop, and nothing comes back this way.
///
/// Whoever needs an answer to what they sent waits on the application's own
/// effect carrying the identifier back, which the application's performer
/// delivers.
pub struct World<A: App> {
    events: tokio::sync::mpsc::UnboundedSender<Event<A>>,
}

impl<A: App> World<A> {
    /// A world nothing is stepping yet, and the events it will send.
    #[must_use]
    pub fn new() -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<Event<A>>) {
        let (events, receiving) = tokio::sync::mpsc::unbounded_channel();
        (Arc::new(Self { events }), receiving)
    }

    /// Tells the instance something happened.
    pub fn send(&self, event: impl Into<Event<A>>) {
        let event = event.into();
        if self.events.send(event).is_err() {
            tracing::error!("the instance is no longer stepping, so an event was lost");
        }
    }
}

/// What performs the application's own effects.
///
/// The one thing the entry point supplies. The world calls it for every
/// effect in the application hole and for nothing else.
pub trait Perform<A: App>: Send + Sync + 'static {
    /// Performs one of the application's effects.
    fn perform(&self, effect: A::Effect) -> impl std::future::Future<Output = ()> + Send;
}

/// Steps the deciding half for as long as this process runs.
///
/// `pending` is what it asked for on construction, performed before the
/// first event is read. Each effect is performed before the next is looked
/// at, which is what keeps an answered effect answered in the order it was
/// asked for; a performer that has nothing to wait on returns at once.
#[mutants::skip]
pub fn run<D, P>(
    deciding: D,
    pending: Vec<Effect<D::App>>,
    world: Arc<World<D::App>>,
    performer: Arc<P>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<Event<D::App>>,
) where
    D: Deciding + Send + 'static,
    P: Perform<D::App>,
{
    drop(tokio::spawn(async move {
        let mut deciding = deciding;
        for effect in pending {
            perform(&world, &performer, effect).await;
        }
        while let Some(event) = events.recv().await {
            tracing::trace!(kind = event.kind(), "stepping");
            for effect in deciding.step(event) {
                perform(&world, &performer, effect).await;
            }
        }
        tracing::error!("the world stopped sending events, so the instance stopped stepping");
    }));
}

/// Performs one effect: a generic one as the mechanism it names, and one of
/// the application's by handing it over.
#[mutants::skip]
async fn perform<A: App, P: Perform<A>>(
    world: &Arc<World<A>>,
    performer: &Arc<P>,
    effect: Effect<A>,
) {
    tracing::trace!(kind = effect.kind(), "performing");
    match effect {
        Effect::Read { id, path } => {
            let world = Arc::clone(world);
            drop(tokio::spawn(async move {
                let contents = tokio::task::spawn_blocking(move || read(&path))
                    .await
                    .unwrap_or_else(|why| Err(why.to_string()));
                world.send(Event::Read { id, contents });
            }));
        }
        Effect::Write {
            id,
            path,
            bytes,
            private,
        } => {
            let outcome =
                tokio::task::spawn_blocking(move || write(&path, bytes.as_slice(), private))
                    .await
                    .unwrap_or_else(|why| Err(why.to_string()));
            world.send(Event::Written { id, outcome });
        }
        Effect::Run {
            id,
            program,
            arguments,
            environment,
            stdin,
        } => {
            let world = Arc::clone(world);
            drop(tokio::spawn(async move {
                let finished = run_once(&program, &arguments, &environment, stdin).await;
                world.send(Event::Ran { id, finished });
            }));
        }
        Effect::Wake { id, after } => {
            let world = Arc::clone(world);
            drop(tokio::spawn(async move {
                tokio::time::sleep(after).await;
                world.send(Event::Woke { id });
            }));
        }
        Effect::Print { text } => {
            // Whoever started this process is reading here, and a reader
            // that has gone away is not a reason to stop: the write is
            // attempted, and its failure is the one thing not worth a line.
            let mut out = io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
        #[expect(
            clippy::exit,
            reason = "an exit effect is how the instance ends the process, and this is its performer"
        )]
        Effect::Exit { message } => {
            // The program's last word, to whoever ran it, and not through a
            // log level anybody could filter.
            eprintln!("stageman: {message}");
            std::process::exit(1);
        }
        Effect::App(effect) => performer.perform(effect).await,
    }
}

/// Reads a file whole, telling absent from unreadable.
fn read(path: &Path) -> Result<Option<Bytes>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(Bytes::new(bytes))),
        Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(why) => Err(why.to_string()),
    }
}

/// Writes a file whole, atomically, making its directory if need be.
fn write(path: &Path, bytes: &[u8], private: bool) -> Result<(), String> {
    if let Some(directory) = path.parent()
        && !directory.as_os_str().is_empty()
    {
        fs::create_dir_all(directory).map_err(|why| why.to_string())?;
    }
    write_atomically(path, bytes, private).map_err(|why| why.to_string())
}

/// Replaces a file in one step, so a crash mid-write cannot truncate it.
///
/// Written beside the target rather than in a temporary directory, because
/// renaming across filesystems is not atomic and would silently become a
/// copy. A private file is created readable and writable by its owner alone,
/// where the platform has an opinion; a mode is set at creation rather than
/// afterwards, so there is no moment at which it is readable by anybody
/// else.
///
/// # Errors
///
/// Fails if the file cannot be created, written, synced or renamed.
pub fn write_atomically(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let outcome = (|| {
        let mut opening = fs::OpenOptions::new();
        opening.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            std::os::unix::fs::OpenOptionsExt::mode(&mut opening, 0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = opening.open(&temporary)?;
        file.write_all(bytes)?;
        // Rename is atomic, but only orders against data that has reached the
        // disk. Without this a crash can leave an intact name over empty
        // contents, which is the failure this function exists to prevent.
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if outcome.is_err() {
        // Best effort: the write already failed, and failing to tidy up after
        // it is not worth reporting over the failure itself.
        drop(fs::remove_file(&temporary));
    }
    outcome
}

/// Runs a program once, with exactly the environment given, to its end.
#[mutants::skip]
async fn run_once(
    program: &Path,
    arguments: &[String],
    environment: &Environment,
    stdin: Option<Bytes>,
) -> Finished {
    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .env_clear()
        .envs(environment)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(why) if why.kind() == io::ErrorKind::NotFound => return Finished::NotFound,
        Err(why) => return Finished::Failed(why.to_string()),
    };
    if let Some(bytes) = stdin
        && let Some(mut writing) = child.stdin.take()
    {
        use tokio::io::AsyncWriteExt as _;
        if let Err(why) = writing.write_all(bytes.as_slice()).await {
            return Finished::Failed(format!("its standard input could not be written: {why}"));
        }
        // End of file is what tells a program it has the whole of its input,
        // and dropping the handle is what sends one.
        drop(writing);
    }
    match child.wait_with_output().await {
        Ok(output) => Finished::Exited {
            status: output.status.code(),
            stdout: Bytes::new(output.stdout),
            stderr: Bytes::new(output.stderr),
        },
        Err(why) => Finished::Failed(why.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde::{Deserialize, Serialize};
    use stageman_vocabulary::{App, EffectId, Named};

    use super::{Bytes, Event, World, read, write, write_atomically};

    /// An application that adds nothing, so what is tested here is the
    /// mechanisms and nothing of anybody's domain.
    #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Nothing;

    impl Named for Nothing {
        fn kind(&self) -> &'static str {
            "Nothing"
        }
    }

    impl App for Nothing {
        type Event = Self;
        type Effect = Self;
    }

    /// Absent is an answer, and unreadable is a different one.
    ///
    /// The whole of what stands between "this is a first run" and "your
    /// instance cannot be read": one of those writes a new file over
    /// nothing and the other must refuse. A directory is the cheapest
    /// unreadable file there is, and it fails with something that is not
    /// absence.
    #[test]
    fn a_read_tells_absent_from_unreadable() {
        let scratch = tempfile::tempdir().expect("a temporary directory");

        let missing = scratch.path().join("not-here");
        assert!(read(&missing) == Ok(None), "absent is not a failure");

        let present = scratch.path().join("here");
        fs::write(&present, b"contents").expect("it writes");
        assert_eq!(
            read(&present).expect("it reads").map(Bytes::into_inner),
            Some(b"contents".to_vec())
        );

        let directory = scratch.path().join("a-directory");
        fs::create_dir(&directory).expect("it is made");
        assert!(
            read(&directory).is_err(),
            "a file that cannot be read is not an absent one"
        );
    }

    /// A write makes the directory it was told to write into, and replaces
    /// what was there.
    ///
    /// An instance is kept under a directory nobody made, so a first run
    /// that refused for want of one would refuse on every fresh machine.
    #[test]
    fn a_write_makes_the_directory_it_needs_and_replaces_what_is_there() {
        let scratch = tempfile::tempdir().expect("a temporary directory");
        let path = scratch.path().join("nested").join("deeper").join("file");

        write(&path, b"first", false).expect("it writes");
        assert_eq!(fs::read(&path).expect("it is there"), b"first");

        write(&path, b"second", false).expect("it writes again");
        assert_eq!(
            fs::read(&path).expect("still there"),
            b"second",
            "a write replaces rather than appends"
        );
        assert!(
            !path.with_extension("tmp").exists(),
            "and leaves nothing beside it"
        );
    }

    /// A private write is readable by its owner and by nobody else.
    #[cfg(unix)]
    #[test]
    fn a_private_write_is_readable_by_nobody_else() {
        use std::os::unix::fs::PermissionsExt as _;

        let scratch = tempfile::tempdir().expect("a temporary directory");
        let path = scratch.path().join("key");
        write(&path, b"material", true).expect("it writes");

        let mode = fs::metadata(&path)
            .expect("it is there")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }

    /// A write that cannot happen says so rather than reporting success.
    #[test]
    fn a_write_that_cannot_happen_fails_and_tidies_up() {
        let scratch = tempfile::tempdir().expect("a temporary directory");
        // No directory, and this is the function that does not make one, so
        // what fails is creating the temporary beside the target.
        let path = scratch.path().join("absent").join("file");

        assert!(write_atomically(&path, b"x", false).is_err());
        assert!(!path.exists());
        assert!(
            !path.with_extension("tmp").exists(),
            "nothing is left behind"
        );
    }

    /// An event sent reaches whoever is stepping, whole.
    #[test]
    fn an_event_sent_is_an_event_received() {
        let (world, mut events) = World::<Nothing>::new();
        world.send(Event::Woke { id: EffectId(7) });

        let heard = events.try_recv().expect("it arrived");
        assert!(heard == Event::Woke { id: EffectId(7) });
    }
}
