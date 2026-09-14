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
//!
//! **A process kept open is three tasks and a handle.** One task writes it
//! every line it is sent, one turns every line it writes into an event, and
//! one waits for it to end — after the last line, so that its end is never
//! announced before what it said. What the loop holds is where the lines go
//! and what kills it, which is all closing one needs.
//!
//! **A probe is a connection and one read, and says what the port did.** Not
//! what that means: a port that accepted and closed at once is a fact about
//! a socket, and whether anything was behind it is a question about what
//! accepted, which the deciding half knows and this crate does not.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use stageman_vocabulary::{
    Answer, App, Arrival, Bytes, Deciding, Effect, EffectId, Ended, Environment, Event, Finished,
    Named, Probed, RequestId,
};

/// The way in: events go to the loop, and nothing comes back this way.
///
/// Whoever needs an answer to what they sent waits on the application's own
/// effect carrying the identifier back, which the application's performer
/// delivers.
pub struct World<A: App> {
    events: tokio::sync::mpsc::UnboundedSender<Event<A>>,
    /// Requests being held open, by the identifier whoever decides answers
    /// with. A request is in here for exactly as long as nothing has said
    /// what to do about it.
    held: parking_lot::Mutex<BTreeMap<RequestId, tokio::sync::oneshot::Sender<Answer>>>,
    /// The next request identifier. Minted here because this is what holds
    /// the connection open; every other identifier is minted by the
    /// deciding half, for the same reason inverted.
    next: AtomicU64,
    /// Processes kept open, by the identifier each was opened under. One is
    /// in here from being started until it ends or is closed, whichever
    /// comes first.
    open: parking_lot::Mutex<BTreeMap<EffectId, Opened>>,
}

/// A process kept open, as far as the loop needs to reach it.
///
/// The process itself is owned by the task waiting on it; what is held here
/// is the two ways of reaching it: lines on their way to its standard input,
/// and the signal that kills it.
struct Opened {
    /// Where a line sent to it goes. Dropping this is what closes its input.
    lines: tokio::sync::mpsc::UnboundedSender<String>,
    /// What kills it, once.
    closing: tokio::sync::oneshot::Sender<()>,
}

impl<A: App> World<A> {
    /// A world nothing is stepping yet, and the events it will send.
    #[must_use]
    pub fn new() -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<Event<A>>) {
        let (events, receiving) = tokio::sync::mpsc::unbounded_channel();
        (
            Arc::new(Self {
                events,
                held: parking_lot::Mutex::new(BTreeMap::new()),
                next: AtomicU64::new(1),
                open: parking_lot::Mutex::new(BTreeMap::new()),
            }),
            receiving,
        )
    }

    /// Sends a line to a process kept open, if it still is.
    ///
    /// One that has ended is not written to, and that is not an error: its
    /// end is on its way as an event, and whoever sent the line will hear
    /// it.
    fn tell(&self, id: EffectId, line: String) {
        let delivered = self
            .open
            .lock()
            .get(&id)
            .is_some_and(|opened| opened.lines.send(line).is_ok());
        undelivered(delivered);
    }

    /// Ends a process kept open: its input is closed and it is killed.
    ///
    /// Closing one that has already ended is nothing, because its end was
    /// already said.
    fn close(&self, id: EffectId) {
        let Some(Opened { lines, closing }) = self.open.lock().remove(&id) else {
            tracing::debug!("asked to close a process that is not open; nothing to do");
            return;
        };
        // In this order: end of file first, so that a process reading its
        // input learns it is over before it is made to be.
        drop(lines);
        // Nobody at the other end means it already ended, which is fine.
        let _ = closing.send(());
    }

    /// Holds a request open, and says what identifies it.
    fn holding(&self, answering: tokio::sync::oneshot::Sender<Answer>) -> RequestId {
        let id = RequestId(self.next.fetch_add(1, Ordering::Relaxed));
        drop(self.held.lock().insert(id, answering));
        id
    }

    /// Holds an already-named request open again, for the answer that
    /// follows its body.
    fn holding_again(&self, id: RequestId, answering: tokio::sync::oneshot::Sender<Answer>) {
        drop(self.held.lock().insert(id, answering));
    }

    /// Says what to do about a request being held, if it is still held.
    fn answer(&self, id: RequestId, answer: Answer) {
        let Some(answering) = self.held.lock().remove(&id) else {
            tracing::warn!("an answer arrived for a request nobody is holding; ignored");
            return;
        };
        if answering.send(answer).is_err() {
            tracing::debug!("a request was answered after whoever sent it had gone");
        }
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
        Effect::Open {
            id,
            program,
            arguments,
            environment,
        } => open(world, id, &program, &arguments, &environment),
        Effect::Send { id, line } => world.tell(id, line),
        Effect::Close { id } => world.close(id),
        Effect::Wake { id, after } => {
            let world = Arc::clone(world);
            drop(tokio::spawn(async move {
                tokio::time::sleep(after).await;
                world.send(Event::Woke { id });
            }));
        }
        Effect::Bind { id, address } => {
            let world = Arc::clone(world);
            drop(tokio::spawn(async move {
                match tokio::net::TcpListener::bind(&address).await {
                    Ok(listening) => {
                        let port = listening.local_addr().map(|taken| taken.port());
                        match port {
                            Ok(port) => {
                                world.send(Event::Bound {
                                    id,
                                    outcome: Ok(port),
                                });
                                accepting(listening, id, world).await;
                            }
                            Err(why) => world.send(Event::Bound {
                                id,
                                outcome: Err(why.to_string()),
                            }),
                        }
                    }
                    Err(why) => world.send(Event::Bound {
                        id,
                        outcome: Err(why.to_string()),
                    }),
                }
            }));
        }
        Effect::Answer { id, answer } => world.answer(id, answer),
        Effect::Probe { id, port, within } => probing(world, id, port, within),
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

/// Probes a port on a task of its own, and answers with what it did.
fn probing<A: App>(world: &Arc<World<A>>, id: EffectId, port: u16, within: Duration) {
    let world = Arc::clone(world);
    drop(tokio::spawn(async move {
        let probed = probe(port, within).await;
        world.send(Event::Probed { id, probed });
    }));
}

/// Connects to a port on loopback and reads from it once, writing nothing,
/// and says what the port did.
///
/// One read of one byte, with the same budget for connecting and for
/// reading. Whether a port that accepted and then closed had anything behind
/// it is not decided here: that is a question about what accepted, and this
/// crate does not know what did.
///
/// Public because the one test that can meet a real proxy — a port a
/// container runtime published — has to live where that runtime can be
/// named, and this crate names no runtime.
pub async fn probe(port: u16, within: Duration) -> Probed {
    use tokio::io::AsyncReadExt as _;

    let Ok(Ok(mut stream)) = tokio::time::timeout(
        within,
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
    )
    .await
    else {
        return Probed::Refused;
    };

    let mut first = [0_u8; 1];
    match tokio::time::timeout(within, stream.read(&mut first)).await {
        // A clean end of file and a reset are the same thing seen through
        // different far ends, and neither is a fault here.
        Ok(Ok(0) | Err(_)) => Probed::Closed,
        Ok(Ok(_)) => Probed::Spoke,
        Err(_) => Probed::Silent,
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

/// Says so when a line went to a process that is not open.
///
/// Skipped by mutation testing because it is equivalent under one: what the
/// test decides is whether a line is logged, and a log line is not something
/// a test can see. That a line to a process that has ended goes nowhere is
/// tested, by nothing arriving.
#[mutants::skip]
fn undelivered(delivered: bool) {
    if !delivered {
        tracing::debug!("a line was sent to a process that is not open; dropped");
    }
}

/// How much of what a process kept open writes to its standard error is
/// kept for its end.
///
/// From the beginning, because a process that fails says so at once, and
/// bounded because one that runs for an hour says a great deal that is not
/// that. Everything past the bound is still read and discarded: a pipe
/// nobody drains fills, and a process blocked writing to it never ends.
const COMPLAINT_LIMIT: usize = 64 * 1024;

/// Starts a program and keeps it open, with exactly the environment given.
///
/// A program that cannot be started ends at once, so that whoever asked
/// hears one thing either way. Skipped by mutation testing: it starts a
/// process and wires three tasks to it, and everything decided is in what
/// those tasks do, which the tests below drive over real processes.
#[mutants::skip]
fn open<A: App>(
    world: &Arc<World<A>>,
    id: EffectId,
    program: &Path,
    arguments: &[String],
    environment: &Environment,
) {
    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .env_clear()
        .envs(environment)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(why) => {
            let ended = if why.kind() == io::ErrorKind::NotFound {
                Ended::NotFound
            } else {
                Ended::Failed(why.to_string())
            };
            world.send(Event::Ended { id, ended });
            return;
        }
    };
    let (Some(input), Some(output), Some(complaints)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        world.send(Event::Ended {
            id,
            ended: Ended::Failed("it was started without its streams".to_owned()),
        });
        return;
    };

    let (lines, mut queued) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (closing, mut closed) = tokio::sync::oneshot::channel::<()>();
    drop(world.open.lock().insert(id, Opened { lines, closing }));

    // Its input: every line sent, until the sender is dropped — which is
    // what closing it means, and which the process sees as end of file once
    // the handle goes with this task.
    let writing = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        let mut input = input;
        while let Some(mut line) = queued.recv().await {
            line.push('\n');
            if input.write_all(line.as_bytes()).await.is_err() {
                // The far end is gone, and its end will say so.
                break;
            }
        }
    });
    // Its output: one event per line, in order, on a task of its own so
    // that a process saying a great deal never holds up what it is sent.
    let reading = {
        let world = Arc::clone(world);
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let mut lines = tokio::io::BufReader::new(output).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                world.send(Event::Line { id, line });
            }
        })
    };
    let complaining = tokio::spawn(drained(complaints));

    // Its life: waited on, or killed when it is closed, and then its end —
    // after its last line, which is what makes the order a promise.
    let world = Arc::clone(world);
    drop(tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            _ = &mut closed => {
                // Killing one that has just exited on its own is not a
                // failure, and not worth a line.
                drop(child.start_kill());
                child.wait().await
            }
        };
        drop(reading.await);
        let stderr = complaining.await.unwrap_or_default();
        // Gone from the map, so a line sent now is dropped rather than
        // queued for nobody; the sender going with it ends the writer.
        drop(world.open.lock().remove(&id));
        drop(writing);
        let ended = match status {
            Ok(status) => Ended::Exited {
                status: status.code(),
                stderr: Bytes::new(stderr),
            },
            Err(why) => Ended::Failed(why.to_string()),
        };
        world.send(Event::Ended { id, ended });
    }));
}

/// Reads a process's standard error to its end, keeping the beginning.
///
/// Reads past the bound and discards, rather than stopping at it: stopping
/// would leave the pipe to fill, and a process blocked on a pipe nobody
/// drains is a process that never ends — the hang the bound would otherwise
/// reinstate for anything that printed more than it.
async fn drained(mut complaints: tokio::process::ChildStderr) -> Vec<u8> {
    use tokio::io::AsyncReadExt as _;
    let mut kept = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = match complaints.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let Some(room) = COMPLAINT_LIMIT.checked_sub(kept.len()) else {
            continue;
        };
        if let Some(head) = chunk.get(..room.min(read)) {
            kept.extend_from_slice(head);
        }
    }
    kept
}

/// Accepts on a bound address for as long as anything is stepping.
///
/// A task per connection, because a request held open for an answer must not
/// hold up the next one, and the answer comes from a loop this is not on.
async fn accepting<A: App>(
    listening: tokio::net::TcpListener,
    on: stageman_vocabulary::EffectId,
    world: Arc<World<A>>,
) {
    loop {
        let (stream, peer) = match listening.accept().await {
            Ok(accepted) => accepted,
            Err(why) => {
                tracing::warn!(%why, "a connection could not be accepted");
                continue;
            }
        };
        let world = Arc::clone(&world);
        drop(tokio::spawn(async move {
            let serving = hyper::service::service_fn(move |request| {
                let world = Arc::clone(&world);
                async move { Ok::<_, std::convert::Infallible>(asked(&world, on, peer, request).await) }
            });
            // With upgrades, so that a websocket through this stays one
            // rather than a connection closed after its handshake.
            if let Err(why) = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), serving)
                .with_upgrades()
                .await
            {
                tracing::debug!(%why, "a connection ended");
            }
        }));
    }
}

/// One request, from arriving to being answered.
///
/// Nothing here decides: the head goes out as an event, and what comes back
/// says to respond, to forward, or to read the body and ask again.
async fn asked<A: App>(
    world: &Arc<World<A>>,
    on: stageman_vocabulary::EffectId,
    peer: std::net::SocketAddr,
    request: hyper::Request<hyper::body::Incoming>,
) -> hyper::Response<Sent> {
    let (parts, incoming) = request.into_parts();
    let arrival = Arrival {
        method: parts.method.to_string(),
        path: parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path().to_owned(), ToString::to_string),
        headers: named(&parts.headers),
        peer: peer.to_string(),
        at: now(),
    };

    let (answering, mut answered) = tokio::sync::oneshot::channel();
    let id = world.holding(answering);
    world.send(Event::Arrived {
        listener: on,
        id,
        request: arrival,
    });

    let mut body = Waiting::Arriving(incoming);
    loop {
        let Ok(answer) = answered.await else {
            tracing::warn!("nothing said what to do about a request, so nobody was answered");
            return nobody(&Bytes::new(Vec::new()));
        };
        match answer {
            Answer::Respond {
                status,
                headers,
                body,
            } => return responded(status, &headers, &body),
            Answer::Proxy { port, refused } => {
                return relayed(port, &refused, parts, body).await;
            }
            Answer::Read { limit } => {
                let (outcome, read) = taken(body, limit).await;
                body = read;
                let (again, waiting) = tokio::sync::oneshot::channel();
                world.holding_again(id, again);
                answered = waiting;
                world.send(Event::Body { id, outcome });
            }
        }
    }
}

/// A request's headers, by lowercased name, one line per name.
fn named(headers: &hyper::HeaderMap) -> std::collections::BTreeMap<String, String> {
    let mut named: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (name, value) in headers {
        let Ok(value) = value.to_str() else {
            // A header this cannot spell is one nothing downstream could
            // read either, and dropping it is better than refusing the
            // request over it.
            continue;
        };
        named
            .entry(name.as_str().to_ascii_lowercase())
            .and_modify(|already| {
                already.push_str(", ");
                already.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
    }
    named
}

/// Now, in milliseconds since the epoch.
fn now() -> u64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let Ok(millis) = u64::try_from(since.as_millis()) else {
        // A clock some hundreds of millions of years from now.
        return u64::MAX;
    };
    millis
}

/// Reads a body, up to a limit, and keeps it for whatever comes next.
async fn taken(body: Waiting, limit: usize) -> (Result<Bytes, String>, Waiting) {
    match body {
        Waiting::Read(read) => (Ok(Bytes::new(read.to_vec())), Waiting::Read(read)),
        Waiting::Arriving(incoming) => {
            use http_body_util::BodyExt as _;
            match http_body_util::Limited::new(incoming, limit)
                .collect()
                .await
            {
                Ok(collected) => {
                    let read = collected.to_bytes();
                    (Ok(Bytes::new(read.to_vec())), Waiting::Read(read))
                }
                // Whatever was read is gone with the error, so what follows
                // has an empty body rather than half of one.
                Err(why) => (Err(why.to_string()), Waiting::Read(bytes::Bytes::new())),
            }
        }
    }
}

/// A response built from what an answer said to send.
fn responded(
    status: u16,
    headers: &std::collections::BTreeMap<String, String>,
    body: &Bytes,
) -> hyper::Response<Sent> {
    use http_body_util::BodyExt as _;
    let mut built = hyper::Response::builder().status(status);
    for (name, value) in headers {
        built = built.header(name, value);
    }
    let body = http_body_util::Full::new(bytes::Bytes::from(body.as_slice().to_vec()))
        .map_err(|never| match never {})
        .boxed();
    built.body(body).unwrap_or_else(|why| {
        tracing::warn!(%why, "an answer could not be built into a response");
        nobody(&Bytes::new(Vec::new()))
    })
}

/// What is sent when nothing usable came back, saying what it was given.
fn nobody(said: &Bytes) -> hyper::Response<Sent> {
    use http_body_util::BodyExt as _;
    let mut response = hyper::Response::new(
        http_body_util::Full::new(bytes::Bytes::from(said.as_slice().to_vec()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = hyper::StatusCode::BAD_GATEWAY;
    response
}

/// Speaks HTTP to a port on loopback and hands back what it said.
///
/// Upgrades are carried through in both directions. Both handles are taken
/// before anything is awaited on them, because an upgrade is available only
/// until the message it belongs to is consumed.
async fn relayed(
    port: u16,
    refused: &Bytes,
    mut parts: hyper::http::request::Parts,
    body: Waiting,
) -> hyper::Response<Sent> {
    use http_body_util::BodyExt as _;
    let upward = parts.extensions.remove::<hyper::upgrade::OnUpgrade>();
    // Whatever framing arrived describes a body this process has already
    // decoded, and the client re-derives it from what it is given. Left in
    // place, the two disagree and the request is refused.
    parts.headers.remove(hyper::header::TRANSFER_ENCODING);

    let stream = match tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await {
        Ok(stream) => stream,
        Err(why) => {
            tracing::debug!(%port, %why, "nothing answered where a request was forwarded");
            return nobody(refused);
        }
    };
    let handshake =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await;
    let (mut sender, connection) = match handshake {
        Ok(spoken) => spoken,
        Err(why) => {
            tracing::debug!(%port, %why, "a forwarded connection could not be opened");
            return nobody(refused);
        }
    };
    // With upgrades, so that a 101 leaves the connection to be taken over
    // rather than closed underneath it.
    drop(tokio::spawn(connection.with_upgrades()));

    let sending: Sent = match body {
        Waiting::Arriving(incoming) => incoming.map_err(BoxedError::from).boxed(),
        Waiting::Read(read) => http_body_util::Full::new(read)
            .map_err(|never| match never {})
            .boxed(),
    };
    let mut response = match sender
        .send_request(hyper::Request::from_parts(parts, sending))
        .await
    {
        Ok(response) => response,
        Err(why) => {
            tracing::debug!(%port, %why, "a forwarded request was not answered");
            return nobody(refused);
        }
    };

    if response.status() == hyper::StatusCode::SWITCHING_PROTOCOLS
        && let Some(upward) = upward
    {
        let downward = hyper::upgrade::on(&mut response);
        // On a task of its own, because it lives as long as the connection
        // does and the response has to be returned now for the handshake to
        // complete at all.
        drop(tokio::spawn(async move {
            let (Ok(upward), Ok(downward)) = tokio::join!(upward, downward) else {
                tracing::debug!(%port, "an upgraded connection was not established");
                return;
            };
            let mut upward = hyper_util::rt::TokioIo::new(upward);
            let mut downward = hyper_util::rt::TokioIo::new(downward);
            if let Err(why) = tokio::io::copy_bidirectional(&mut upward, &mut downward).await {
                tracing::debug!(%port, %why, "an upgraded connection ended");
            }
        }));
    }

    response.map(|body| body.map_err(BoxedError::from).boxed())
}

/// Whatever a response carries, however it was made.
type Sent = http_body_util::combinators::BoxBody<bytes::Bytes, BoxedError>;

/// What a body can fail with, once two kinds of body are one type.
type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// A request's body, before and after it has been read.
///
/// Read once and kept, because an answer may read a body and then forward
/// the request anyway, and a body arriving off a socket can only be taken
/// once.
enum Waiting {
    /// Still on the wire.
    Arriving(hyper::body::Incoming),
    /// Read, and held for whatever comes next.
    Read(bytes::Bytes),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};
    use stageman_vocabulary::{App, EffectId, Ended, Environment, Named, Probed};

    use super::{Answer, Bytes, Effect, Event, Perform, World, read, write, write_atomically};

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

    /// Performs nothing, for the tests that are about mechanisms only.
    struct NoOne;

    impl Perform<Nothing> for NoOne {
        async fn perform(&self, _: Nothing) {}
    }

    /// Takes an address and says which port it got.
    async fn bound(
        world: &Arc<World<Nothing>>,
        events: &mut tokio::sync::mpsc::UnboundedReceiver<Event<Nothing>>,
        id: EffectId,
    ) -> u16 {
        answering(
            world,
            Effect::Bind {
                id,
                address: "127.0.0.1:0".to_owned(),
            },
        )
        .await;
        match next(events).await {
            Event::Bound { outcome, .. } => outcome.expect("it was taken"),
            other => panic!("expected a port: {}", other.kind()),
        }
    }

    /// How long a test waits before calling something hung.
    ///
    /// Bounded so that a mutation which stops answering fails in a second
    /// rather than hanging: a hang is caught either way, and one that costs
    /// a timeout makes every mutation run longer than it needs to be.
    const PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

    /// The next event, or a failure rather than a wait without end.
    async fn next(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<Event<Nothing>>,
    ) -> Event<Nothing> {
        tokio::time::timeout(PATIENCE, events.recv())
            .await
            .expect("something was sent before this gave up")
            .expect("the world is still sending")
    }

    /// Performs one effect, with nothing behind the application hole.
    async fn answering(world: &Arc<World<Nothing>>, effect: Effect<Nothing>) {
        super::perform(world, &Arc::new(NoOne), effect).await;
    }

    /// Says one request and reads everything said back.
    ///
    /// Written out by hand rather than through a client, because what is
    /// under test is what this crate does with the bytes.
    async fn saying(port: u16, said: String) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("it connects");
        stream
            .write_all(said.as_bytes())
            .await
            .expect("it is written");
        let mut heard = Vec::new();
        tokio::time::timeout(PATIENCE, stream.read_to_end(&mut heard))
            .await
            .expect("it was answered before this gave up")
            .expect("it is answered");
        String::from_utf8_lossy(&heard).into_owned()
    }

    /// An address taken says which port it got, and one taken twice says it
    /// could not be.
    #[tokio::test]
    async fn an_address_taken_says_which_port_it_got() {
        let (world, mut events) = World::<Nothing>::new();
        let port = bound(&world, &mut events, EffectId(1)).await;
        assert_ne!(port, 0, "a port of zero is answered with a real one");

        answering(
            &world,
            Effect::Bind {
                id: EffectId(2),
                address: format!("127.0.0.1:{port}"),
            },
        )
        .await;
        match next(&mut events).await {
            Event::Bound { id, outcome } => {
                assert_eq!(id, EffectId(2));
                assert!(outcome.is_err(), "an address in use cannot be taken twice");
            }
            other => panic!("expected an answer: {}", other.kind()),
        }
    }

    /// A request arrives as its head, and what is answered is what is sent.
    #[tokio::test]
    async fn a_request_arrives_as_its_head_and_is_answered() {
        let (world, mut events) = World::<Nothing>::new();
        let port = bound(&world, &mut events, EffectId(1)).await;

        let asking = tokio::spawn(saying(
            port,
            "GET /jobs?open=1 HTTP/1.1\r\nHost: stageman.test\r\nConnection: close\r\n\r\n"
                .to_owned(),
        ));

        let (id, request) = match next(&mut events).await {
            Event::Arrived {
                listener,
                id,
                request,
            } => {
                assert_eq!(listener, EffectId(1), "the listener it arrived on");
                (id, request)
            }
            other => panic!("expected an arrival: {}", other.kind()),
        };
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/jobs?open=1", "the query is part of it");
        assert_eq!(
            request.headers.get("host").map(String::as_str),
            Some("stageman.test")
        );
        assert!(request.peer.starts_with("127.0.0.1:"), "{}", request.peer);
        assert!(
            request.at > 1_700_000_000_000,
            "stamped with the time rather than with a number: {}",
            request.at
        );

        answering(
            &world,
            Effect::Answer {
                id,
                answer: Answer::Respond {
                    status: 418,
                    headers: [("content-type".to_owned(), "text/plain".to_owned())].into(),
                    body: Bytes::new(b"no".to_vec()),
                },
            },
        )
        .await;

        let said = asking.await.expect("it finished");
        assert!(said.starts_with("HTTP/1.1 418"), "{said}");
        assert!(said.contains("content-type: text/plain"), "{said}");
        assert!(said.ends_with("no"), "{said}");
    }

    /// A body is read only when it is asked for, and then handed back whole.
    #[tokio::test]
    async fn a_body_is_read_only_when_it_is_asked_for() {
        let (world, mut events) = World::<Nothing>::new();
        let port = bound(&world, &mut events, EffectId(1)).await;

        let asking = tokio::spawn(saying(
            port,
            "POST /tools HTTP/1.1\r\nHost: x\r\nContent-Length: 7\r\nConnection: close\r\n\r\n{\"a\":1}"
                .to_owned(),
        ));

        let id = match next(&mut events).await {
            Event::Arrived { id, .. } => id,
            other => panic!("expected an arrival: {}", other.kind()),
        };
        answering(
            &world,
            Effect::Answer {
                id,
                answer: Answer::Read { limit: 64 },
            },
        )
        .await;

        match next(&mut events).await {
            Event::Body { id: whose, outcome } => {
                assert_eq!(whose, id);
                let read = outcome.expect("it read");
                assert_eq!(read.as_text(), Some("{\"a\":1}"), "the body, whole");
            }
            other => panic!("expected a body: {}", other.kind()),
        }

        answering(
            &world,
            Effect::Answer {
                id,
                answer: Answer::Respond {
                    status: 200,
                    headers: std::collections::BTreeMap::new(),
                    body: Bytes::new(b"ok".to_vec()),
                },
            },
        )
        .await;
        let said = asking.await.expect("it finished");
        assert!(said.starts_with("HTTP/1.1 200"), "{said}");
        assert!(said.ends_with("ok"), "{said}");
    }

    /// A request forwarded to a port is answered by whatever is behind it.
    ///
    /// Behind it is another listener of this crate's, which is the cheapest
    /// honest thing to forward to: what comes back has been through the
    /// whole path rather than a fixture.
    #[tokio::test]
    async fn a_request_forwarded_is_answered_by_what_is_behind_the_port() {
        let (world, mut events) = World::<Nothing>::new();
        let front = bound(&world, &mut events, EffectId(1)).await;
        let behind = bound(&world, &mut events, EffectId(2)).await;

        let asking = tokio::spawn(saying(
            front,
            "GET /page HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".to_owned(),
        ));

        let id = match next(&mut events).await {
            Event::Arrived { id, listener, .. } => {
                assert_eq!(listener, EffectId(1));
                id
            }
            other => panic!("expected an arrival: {}", other.kind()),
        };
        answering(
            &world,
            Effect::Answer {
                id,
                answer: Answer::Proxy {
                    port: behind,
                    refused: Bytes::new(Vec::new()),
                },
            },
        )
        .await;

        let (behind_id, request) = match next(&mut events).await {
            Event::Arrived {
                id,
                listener,
                request,
            } => {
                assert_eq!(listener, EffectId(2), "it arrived on the other listener");
                (id, request)
            }
            other => panic!("expected an arrival: {}", other.kind()),
        };
        assert_eq!(request.path, "/page", "forwarded whole");

        answering(
            &world,
            Effect::Answer {
                id: behind_id,
                answer: Answer::Respond {
                    status: 201,
                    headers: std::collections::BTreeMap::new(),
                    body: Bytes::new(b"behind".to_vec()),
                },
            },
        )
        .await;

        let said = asking.await.expect("it finished");
        assert!(said.starts_with("HTTP/1.1 201"), "{said}");
        assert!(said.ends_with("behind"), "{said}");
    }

    /// A websocket through a forward is still a websocket.
    ///
    /// The handshake is only the beginning of one: what makes it work is
    /// that the connection is handed over afterwards and bytes go both
    /// ways. Something that forwarded the handshake and closed would look
    /// right in a trace and be broken in a browser — which is why what is
    /// behind this one is a socket that speaks an upgrade by hand rather
    /// than another listener of ours.
    #[tokio::test]
    async fn an_upgrade_through_a_forward_is_carried_both_ways() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let (world, mut events) = World::<Nothing>::new();
        let front = bound(&world, &mut events, EffectId(1)).await;

        let behind = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("it binds");
        let port = behind.local_addr().expect("it has an address").port();
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = behind.accept().await.expect("it is reached");
            let mut head = [0_u8; 1024];
            let read = stream.read(&mut head).await.expect("a head arrives");
            assert!(read > 0, "the request came through");
            stream
                .write_all(
                    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                      Connection: Upgrade\r\n\r\n",
                )
                .await
                .expect("it answers");
            let mut said = [0_u8; 5];
            stream.read_exact(&mut said).await.expect("it hears");
            stream.write_all(&said).await.expect("it says it back");
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", front))
            .await
            .expect("it connects");
        client
            .write_all(
                b"GET /socket HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\
                  Connection: Upgrade\r\n\r\n",
            )
            .await
            .expect("it is written");

        let id = match next(&mut events).await {
            Event::Arrived { id, .. } => id,
            other => panic!("expected an arrival: {}", other.kind()),
        };
        answering(
            &world,
            Effect::Answer {
                id,
                answer: Answer::Proxy {
                    port,
                    refused: Bytes::new(Vec::new()),
                },
            },
        )
        .await;

        let mut head = [0_u8; 1024];
        let read = tokio::time::timeout(PATIENCE, client.read(&mut head))
            .await
            .expect("it answered before this gave up")
            .expect("it answers");
        let answered = String::from_utf8_lossy(&head[..read]).into_owned();
        assert!(answered.starts_with("HTTP/1.1 101"), "{answered}");

        // And the part a handshake alone would not prove.
        client.write_all(b"hello").await.expect("it is written");
        let mut echoed = [0_u8; 5];
        tokio::time::timeout(PATIENCE, client.read_exact(&mut echoed))
            .await
            .expect("it came back before this gave up")
            .expect("it comes back through the upgrade");
        assert_eq!(&echoed, b"hello");
        upstream.await.expect("the far end finished");
    }

    /// A forward to a port with nothing behind it says what it was told to.
    ///
    /// What it means for nothing to answer is the deciding half's to know,
    /// so the words are carried rather than composed here.
    #[tokio::test]
    async fn a_forward_to_nothing_says_what_it_was_given_to_say() {
        let (world, mut events) = World::<Nothing>::new();
        let front = bound(&world, &mut events, EffectId(1)).await;
        // Taken and let go of, so it is a port nothing is behind.
        let empty = {
            let taken = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("it binds");
            taken.local_addr().expect("it has an address").port()
        };

        let asking = tokio::spawn(saying(
            front,
            "GET /page HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".to_owned(),
        ));
        let id = match next(&mut events).await {
            Event::Arrived { id, .. } => id,
            other => panic!("expected an arrival: {}", other.kind()),
        };
        answering(
            &world,
            Effect::Answer {
                id,
                answer: Answer::Proxy {
                    port: empty,
                    refused: Bytes::new(b"nothing is showing".to_vec()),
                },
            },
        )
        .await;

        let said = asking.await.expect("it finished");
        assert!(said.starts_with("HTTP/1.1 502"), "{said}");
        assert!(said.ends_with("nothing is showing"), "{said}");
    }

    /// How long a probe here gives a port. Short, because one of the four
    /// answers costs all of it: a port held open in silence is only known to
    /// be by having waited.
    const BRIEFLY: std::time::Duration = std::time::Duration::from_millis(200);

    /// Probes a port through the world, and says what it did.
    async fn probed(
        world: &Arc<World<Nothing>>,
        events: &mut tokio::sync::mpsc::UnboundedReceiver<Event<Nothing>>,
        port: u16,
    ) -> Probed {
        answering(
            world,
            Effect::Probe {
                id: EffectId(9),
                port,
                within: BRIEFLY,
            },
        )
        .await;
        match next(events).await {
            Event::Probed { id, probed } => {
                assert_eq!(id, EffectId(9));
                probed
            }
            other => panic!("expected what the port did: {}", other.kind()),
        }
    }

    /// A probe says what the port did, and each of the four things a port
    /// can do is told from the other three.
    ///
    /// Bound and never accepted on is the held-open-and-silent case for
    /// free: the handshake completes in the kernel's backlog and the read
    /// then waits, which is the shape an HTTP server has before it is asked
    /// anything. Accepted and dropped is the shape a container runtime's
    /// proxy has with nothing behind it — the case a probe that only
    /// connected could not see, and the one that once kept every container
    /// running for ever. What that proxy actually does is not this crate's
    /// to know, and is met by the test that publishes a real port.
    #[tokio::test]
    async fn a_port_probed_says_what_it_did() {
        use tokio::io::AsyncWriteExt as _;

        let (world, mut events) = World::<Nothing>::new();

        // Taken and let go of, so nothing accepts on it.
        let empty = {
            let taken = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("it binds");
            taken.local_addr().expect("it has an address").port()
        };
        assert_eq!(probed(&world, &mut events, empty).await, Probed::Refused);

        let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("it binds");
        let port = silent.local_addr().expect("it has an address").port();
        assert_eq!(
            probed(&world, &mut events, port).await,
            Probed::Silent,
            "accepted in the backlog and never spoken to"
        );
        drop(silent);

        let hanging_up = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("it binds");
        let port = hanging_up.local_addr().expect("it has an address").port();
        let far = tokio::spawn(async move {
            let (stream, _) = hanging_up.accept().await.expect("it is reached");
            drop(stream);
        });
        assert_eq!(probed(&world, &mut events, port).await, Probed::Closed);
        far.await.expect("the far end finished");

        let talking = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("it binds");
        let port = talking.local_addr().expect("it has an address").port();
        let far = tokio::spawn(async move {
            let (mut stream, _) = talking.accept().await.expect("it is reached");
            stream.write_all(b"220 hello\r\n").await.expect("it speaks");
        });
        assert_eq!(probed(&world, &mut events, port).await, Probed::Spoke);
        far.await.expect("the far end finished");
    }

    /// An event sent reaches whoever is stepping, whole.
    #[test]
    fn an_event_sent_is_an_event_received() {
        let (world, mut events) = World::<Nothing>::new();
        world.send(Event::Woke { id: EffectId(7) });

        let heard = events.try_recv().expect("it arrived");
        assert!(heard == Event::Woke { id: EffectId(7) });
    }

    /// Keeps a shell open on a script, with exactly the environment given.
    async fn shell(
        world: &Arc<World<Nothing>>,
        id: EffectId,
        script: &str,
        environment: Environment,
    ) {
        answering(
            world,
            Effect::Open {
                id,
                program: "/bin/sh".into(),
                arguments: vec!["-c".to_owned(), script.to_owned()],
                environment,
            },
        )
        .await;
    }

    /// Nothing arrives for a while, which is the only way to say nothing
    /// arrives at all.
    async fn nothing(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event<Nothing>>) {
        let waited =
            tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await;
        assert!(waited.is_err(), "something arrived that should not have");
    }

    /// A process kept open hears every line it is sent, answers line by
    /// line, and ends when it is closed — killed, since this one would not
    /// go on its own. What is sent after that goes nowhere, and closing it
    /// again is nothing.
    #[tokio::test]
    async fn a_process_kept_open_is_spoken_to_line_by_line_and_ended_when_closed() {
        let (world, mut events) = World::<Nothing>::new();
        let id = EffectId(3);
        // It reads until its input closes and then refuses to go, so that
        // what ends it is the kill and not the end of file — which is the
        // half of closing a test could not otherwise tell was there.
        shell(
            &world,
            id,
            "while read line; do echo \"heard $line\"; done; exec sleep 30",
            Environment::new(),
        )
        .await;

        for said in ["one", "two"] {
            answering(
                &world,
                Effect::Send {
                    id,
                    line: said.to_owned(),
                },
            )
            .await;
            let heard = next(&mut events).await;
            assert!(
                heard
                    == Event::Line {
                        id,
                        line: format!("heard {said}"),
                    },
                "{}",
                heard.kind()
            );
        }

        answering(&world, Effect::Close { id }).await;
        let ended = next(&mut events).await;
        assert!(
            ended
                == Event::Ended {
                    id,
                    ended: Ended::Exited {
                        status: None,
                        stderr: Bytes::new(Vec::new()),
                    },
                },
            "killed, so no status: {}",
            ended.kind()
        );

        answering(
            &world,
            Effect::Send {
                id,
                line: "three".to_owned(),
            },
        )
        .await;
        answering(&world, Effect::Close { id }).await;
        nothing(&mut events).await;
    }

    /// A process that ends on its own says so, and says so after its last
    /// line: the order is a promise, and this is what keeps it one.
    #[tokio::test]
    async fn a_process_that_ends_on_its_own_says_so_after_its_last_line() {
        let (world, mut events) = World::<Nothing>::new();
        let id = EffectId(4);
        shell(
            &world,
            id,
            "echo first; echo second; echo complaint >&2; exit 3",
            Environment::new(),
        )
        .await;

        for said in ["first", "second"] {
            let heard = next(&mut events).await;
            assert!(
                heard
                    == Event::Line {
                        id,
                        line: said.to_owned(),
                    },
                "{}",
                heard.kind()
            );
        }
        let ended = next(&mut events).await;
        assert!(
            ended
                == Event::Ended {
                    id,
                    ended: Ended::Exited {
                        status: Some(3),
                        stderr: Bytes::new(b"complaint\n".to_vec()),
                    },
                },
            "{}",
            ended.kind()
        );
    }

    /// A program that is not there ends at once as not found, and one that
    /// cannot be started for any other reason ends at once saying why.
    #[tokio::test]
    async fn a_program_that_cannot_be_started_ends_at_once_and_says_why() {
        let (world, mut events) = World::<Nothing>::new();
        answering(
            &world,
            Effect::Open {
                id: EffectId(5),
                program: "/nowhere/such/program".into(),
                arguments: Vec::new(),
                environment: Environment::new(),
            },
        )
        .await;
        let ended = next(&mut events).await;
        assert!(
            ended
                == Event::Ended {
                    id: EffectId(5),
                    ended: Ended::NotFound,
                },
            "{}",
            ended.kind()
        );

        // A directory is there and is not a program.
        answering(
            &world,
            Effect::Open {
                id: EffectId(6),
                program: "/".into(),
                arguments: Vec::new(),
                environment: Environment::new(),
            },
        )
        .await;
        match next(&mut events).await {
            Event::Ended {
                id: EffectId(6),
                ended: Ended::Failed(why),
            } => assert!(!why.is_empty()),
            other => panic!("expected a failure to start: {}", other.kind()),
        }
    }

    /// A process is given exactly the environment it was opened with, and
    /// nothing of this process's own.
    ///
    /// The mechanism half of `docs/conventions.md` §3's rule that what a
    /// child is handed is constructed and never inherited: an agent that
    /// found a credential in an inherited variable would bill somebody
    /// else, and this is where inheriting would happen.
    #[tokio::test]
    async fn a_process_is_given_exactly_the_environment_it_was_opened_with() {
        let (world, mut events) = World::<Nothing>::new();
        let id = EffectId(7);
        shell(
            &world,
            id,
            // A home rather than a path, because a shell that finds no path
            // supplies one of its own, and what this asks is whether the
            // world supplied anything.
            "echo \"[$ONLY][$HOME]\"",
            [("ONLY".to_owned(), "this".to_owned())].into(),
        )
        .await;
        let heard = next(&mut events).await;
        assert!(
            heard
                == Event::Line {
                    id,
                    line: "[this][]".to_owned(),
                },
            "{}",
            heard.kind()
        );
    }

    /// A process that floods its error pipe neither hangs nor is kept whole.
    ///
    /// The pipe has a small buffer, and a process blocked writing to one
    /// nobody drains never reaches its own exit. So the flood is read to its
    /// end and only its beginning is kept — and the process still says its
    /// last word afterwards, which is what proves it was never blocked.
    #[tokio::test]
    async fn a_process_that_floods_its_error_pipe_neither_hangs_nor_is_kept_whole() {
        let (world, mut events) = World::<Nothing>::new();
        let id = EffectId(8);
        shell(
            &world,
            id,
            "yes | head -c 300000 >&2; echo done",
            Environment::new(),
        )
        .await;

        let heard = next(&mut events).await;
        assert!(
            heard
                == Event::Line {
                    id,
                    line: "done".to_owned(),
                },
            "{}",
            heard.kind()
        );
        match next(&mut events).await {
            Event::Ended {
                ended: Ended::Exited { status, stderr },
                ..
            } => {
                assert_eq!(status, Some(0));
                assert_eq!(
                    stderr.len(),
                    64 * 1024,
                    "the beginning, sixty-four kibibytes of it, and no more"
                );
                assert!(
                    stderr
                        .as_text()
                        .is_some_and(|text| text.starts_with("y\ny\n"))
                );
            }
            other => panic!("expected its end: {}", other.kind()),
        }
    }
}
