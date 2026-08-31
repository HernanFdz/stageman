//! Operating the instance: the file it is kept in, and the work it supervises.
//!
//! Everything here is the daemon's half and is compiled only for it, because
//! the browser has no business holding a decryption key — see
//! `docs/decisions/0022-the-browser-never-sees-the-domain.md`. What the
//! dashboard is allowed to know is derived from this and lives in
//! [`crate::dashboard`].

use std::fs;
use std::io::{self, Write as _};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use parking_lot::{Mutex, MutexGuard};
use rand::rngs::{StdRng, SysRng};
use rand::{Rng as _, SeedableRng as _};
use stageman_agent::{Answer, ContainerRuntime, StopReason};
use stageman_core::{
    Agent, Errand, Handout, Inconsistent, Job, JobId, Key, NONCE_LEN, Nonce, OpenError, Progress,
    ProjectId, SealError, Snapshot, Speaking, State, Taken, Thread, Timestamp,
};

/// An instance could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The snapshot exists but could not be read.
    #[error("the snapshot could not be read")]
    Read(#[source] io::Error),
    /// The snapshot is not valid JSON.
    #[error("the snapshot is not valid JSON")]
    Parse(#[source] serde_json::Error),
    /// The snapshot could not be decrypted or did not pass its checks.
    #[error("the snapshot could not be opened")]
    Open(#[source] OpenError),
    /// The instance opened but could not write, so it would fail later.
    #[error("the snapshot could not be written at startup")]
    Write(#[source] SaveError),
    /// No source of randomness was available to seed nonce generation.
    #[error("no source of randomness is available")]
    Randomness,
}

/// A snapshot could not be written.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The state describes an instance that cannot exist.
    ///
    /// Refused before it reaches disk rather than after. The same check runs
    /// when a file is read, so writing an inconsistent state would produce one
    /// that cannot be opened — turning a mistake somebody could still undo into
    /// an instance nobody can start.
    ///
    /// It lives here rather than in the domain's sealing, which is about
    /// cryptography and has no business judging whether a state makes sense.
    #[error("the state describes an instance that is not internally consistent")]
    Inconsistent(#[source] Inconsistent),
    /// Credentials could not be sealed.
    #[error("credentials could not be sealed")]
    Seal(#[source] SealError),
    /// The snapshot could not be encoded.
    #[error("the snapshot could not be encoded")]
    Encode(#[source] serde_json::Error),
    /// The file could not be written or replaced.
    #[error("the snapshot file could not be written")]
    Io(#[source] io::Error),
}

/// Everything this instance knows, and the file it is kept in.
///
/// The only way to obtain a mutable borrow of the state is [`Store::update`],
/// which persists when that borrow ends. That is the whole design: writing on
/// every change is a property of the type rather than a rule somebody has to
/// remember, and there is deliberately no other path that hands out a
/// `&mut State`.
pub struct Store {
    state: Mutex<State>,
    /// Seeded once, fallibly, so that producing a nonce afterwards cannot fail.
    ///
    /// Sealing needs a fresh nonce per credential per write and has nowhere to
    /// report a failure mid-write, so the fallible step is moved to startup
    /// where it can be reported properly.
    rng: Mutex<StdRng>,
    path: PathBuf,
    key: Key,
}

impl Store {
    /// Opens the instance kept at `path`, or reports that there is none.
    ///
    /// `Ok(None)` means a first run rather than a failure — see
    /// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`.
    ///
    /// # Errors
    ///
    /// Fails if the file exists but cannot be read, parsed, decrypted, or
    /// believed; or if the instance cannot write.
    pub fn load(path: PathBuf, key: Key) -> Result<Option<Self>, LoadError> {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(LoadError::Read(error)),
        };
        let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(LoadError::Parse)?;
        let state = snapshot.open(&key).map_err(LoadError::Open)?;
        Self::start(path, key, state).map(Some)
    }

    /// Creates an instance from freshly configured state.
    ///
    /// # Errors
    ///
    /// Fails if the instance cannot write, or has no randomness.
    pub fn create(path: PathBuf, key: Key, state: State) -> Result<Self, LoadError> {
        Self::start(path, key, state)
    }

    fn start(path: PathBuf, key: Key, state: State) -> Result<Self, LoadError> {
        let rng = StdRng::try_from_rng(&mut SysRng).map_err(|_| LoadError::Randomness)?;
        let store = Self {
            state: Mutex::new(state),
            rng: Mutex::new(rng),
            path,
            key,
        };
        // Write immediately, before anything can depend on this instance. A
        // bad path, a missing directory, a read-only filesystem or wrong
        // permissions all fail here rather than at the first state change —
        // which is what `docs/conventions.md` §3 asks of anything that can
        // fail at startup. What that leaves is a full disk, which is transient
        // and fixable without stopping.
        store.write(&store.state.lock()).map_err(LoadError::Write)?;
        Ok(store)
    }

    /// Borrows the state for reading.
    ///
    /// Immutable by construction: a mutable borrow would let a caller change
    /// the state without the write that must follow, which is exactly what
    /// [`Store::update`] exists to prevent.
    #[must_use]
    pub fn read(&self) -> StateRef<'_> {
        StateRef(self.state.lock())
    }

    /// Borrows the state for modification, writing a snapshot when the borrow
    /// ends.
    ///
    /// Failure to write is logged rather than returned: no caller can repair a
    /// full disk, the in-memory state is still correct, and stopping would be
    /// a worse answer than continuing.
    #[must_use]
    pub fn update(&self) -> StateGuard<'_> {
        StateGuard {
            state: self.state.lock(),
            store: self,
        }
    }

    fn write(&self, state: &State) -> Result<(), SaveError> {
        state.check().map_err(SaveError::Inconsistent)?;
        let mut rng = self.rng.lock();
        let mut nonces = || {
            let mut nonce: Nonce = [0; NONCE_LEN];
            rng.fill_bytes(&mut nonce);
            nonce
        };
        let snapshot = state
            .seal(&self.key, &mut nonces)
            .map_err(SaveError::Seal)?;
        let encoded = serde_json::to_vec_pretty(&snapshot).map_err(SaveError::Encode)?;
        write_atomically(&self.path, &encoded).map_err(SaveError::Io)
    }
}

/// Replaces a file in one step, so a crash mid-write cannot truncate it.
///
/// Written beside the target rather than in a temporary directory, because
/// renaming across filesystems is not atomic and would silently become a copy.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let outcome = (|| {
        let mut file = fs::File::create(&temporary)?;
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

/// A read-only borrow of the state.
pub struct StateRef<'a>(MutexGuard<'a, State>);

impl Deref for StateRef<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.0
    }
}

/// A mutable borrow of the state that writes a snapshot when it ends.
///
/// Dropping this is what persists the change, so holding one open across
/// unrelated work delays the write and holds the lock. Take it, change what
/// you came to change, and let it go.
pub struct StateGuard<'a> {
    state: MutexGuard<'a, State>,
    store: &'a Store,
}

impl Deref for StateGuard<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.state
    }
}

impl DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.store.write(&self.state) {
            // Deliberately not a panic and deliberately not propagated: a drop
            // cannot report, no caller could repair a full disk, and the state
            // in memory is still correct. It says what happened and not where
            // that should go, which is the whole of
            // `docs/decisions/0018-diagnostics-are-emitted-through-tracing.md`
            // — the destination is still an open question and this line does
            // not have to know the answer.
            tracing::error!(%error, "the instance could not be written");
        }
    }
}

/// A job could not be created.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// A foreman's turn could not be taken.
    #[error("the foreman could not take a turn: {0}")]
    Foreman(String),
    /// The project is not one this instance watches.
    #[error("no project {0} in this instance")]
    UnknownProject(ProjectId),
    /// What the job's agent may see could not be decided.
    #[error("what the job's agent may see could not be decided")]
    Handout(#[source] stageman_core::HandoutError),
}

/// A failure and everything underneath it, as one line of prose.
///
/// `to_string` on an error renders only its outermost line, and every error
/// that reaches a job's record wraps a more specific one — so recording the
/// outer line alone throws away the only part that says what actually went
/// wrong. A job that failed because a credential was rejected read as "the
/// job's agent could not be run", and the reason was only ever visible by
/// asking the container runtime for the container's output.
///
/// One line rather than several, because this is going into a record that a
/// dashboard shows as prose — `docs/conventions.md` §2 keeps a failure as
/// something to read rather than a code to branch on, and the whole chain is
/// what there is to read.
fn because(failure: &dyn std::error::Error) -> String {
    let mut told = failure.to_string();
    let mut cause = std::error::Error::source(failure);
    while let Some(reason) = cause {
        told.push_str(": ");
        told.push_str(&reason.to_string());
        cause = reason.source();
    }
    told
}

/// A job that exists and has not been run yet.
///
/// The gap between those two is the whole reason this type is separate, and it
/// is deliberately narrow: a job is recorded before its container exists, so
/// something has to carry what the container will need from the moment of
/// recording to the moment of starting. Holding it means a caller can answer a
/// request with a job that is already in the instance, and start it afterwards
/// on a task of its own.
pub struct Started {
    job: JobId,
    handout: Handout,
    kickoff: String,
    /// What the channel is told when this starts, composed with the kickoff so
    /// that every text this system emits is authored in the one crate.
    announcement: String,
}

impl Started {
    /// Which job this is.
    #[must_use]
    pub const fn job(&self) -> JobId {
        self.job
    }
}

/// Records a job on a project, ready to be run.
///
/// **The record is written before the container exists, and the order is not
/// arbitrary.** Killed in between, this leaves a job believed to be running
/// with nothing to run in — which the sweep recognises and records as failed.
/// The other order leaves a container naming a job the instance has no record
/// of, which is the case the sweep can only warn about, because a container it
/// cannot place is a container it must not remove. One ordering produces a
/// problem that resolves itself; the other produces one that needs a person.
///
/// Separate from [`supervise`] so that the two halves can happen at different
/// times. A request that starts a job must answer while the job runs — see
/// `docs/conventions.md` §3 on keeping supervision off the request path — and
/// it can only answer with the job if the job already exists.
///
/// The instruction the agent begins from is composed here, from the work, by
/// the foreman. Nothing else composes one: `docs/architecture.md` §1 puts
/// every place an instruction is authored in that one crate, which is what
/// makes the snapshot-testing rule in `docs/conventions.md` §4 mean anything.
///
/// # Errors
///
/// Fails if the project is unknown, or if a handout cannot be decided for it.
pub fn begin(
    store: &Store,
    project: ProjectId,
    agent: Agent,
    reason: &str,
    work: &str,
) -> Result<Started, RunError> {
    let (repository, handout) = {
        let state = store.read();
        let repository = state
            .projects
            .get(&project)
            .ok_or(RunError::UnknownProject(project))?
            .repository
            .clone();
        let handout = Handout::for_job(&state, agent, project).map_err(RunError::Handout)?;
        // Explicit, because holding a lock on the instance across the work
        // below would mean a job's whole run blocking every reader of it.
        drop(state);
        (repository, handout)
    };

    // What the job can be told depends on what it was handed. Asked of the
    // handout rather than of the project, so the prompt and the variables the
    // container is started with are decided from one value — two reads could
    // disagree, and the way that shows up is a job told to run a tool whose
    // credential it was not given.
    let voice = if handout.channels().next().is_some() {
        stageman_foreman::Voice::Channel
    } else {
        stageman_foreman::Voice::Silent
    };
    let kickoff = stageman_foreman::kickoff(&repository, work, voice);
    let job = JobId::from_uuid(uuid::Uuid::new_v4());
    let announcement = stageman_foreman::announcement(&repository, reason, job);

    {
        let mut state = store.update();
        if let Some(project) = state.projects.get_mut(&project) {
            project.jobs.insert(
                job,
                Job {
                    agent,
                    reason: reason.to_owned(),
                    kickoff: kickoff.clone(),
                    created_at: Timestamp::now(),
                    progress: Progress::Working,
                    thread: None,
                },
            );
        }
    }

    Ok(Started {
        job,
        handout,
        kickoff,
        announcement,
    })
}

/// Opens the thread a job speaks in, if its project has a channel bound.
///
/// Answers with the handout narrowed to that thread, or with the progress to
/// record if it could not be opened.
///
/// **A channel that will not take a message fails the job**, rather than
/// letting it run speaking at the root of the channel instead. The kickoff has
/// already told this agent it can reach a person; running it anyway would make
/// that quietly false, and `docs/conventions.md` §3 would rather a credential
/// that has stopped working produce a visible job failure than a mystery. It
/// is also the cheapest moment to fail — no container exists yet, so nothing
/// outward-facing has happened.
///
/// Revisit if a transient outage turns out to fail jobs often enough to
/// matter; the fallback is to run without a thread and say so loudly, which
/// trades one kind of quiet for another and is worth taking only against
/// evidence.
async fn opening(
    store: &Store,
    job: JobId,
    handout: Handout,
    announcement: &str,
) -> Result<Handout, Progress> {
    let Some((channel, bound)) = handout
        .channels()
        .next()
        .map(|(channel, bound)| (channel, bound.clone()))
    else {
        return Ok(handout);
    };

    match crate::channel::open_thread(&bound, channel, announcement).await {
        Ok(thread) => {
            {
                let mut state = store.update();
                if let Some(recorded) = state.job_mut(job) {
                    recorded.thread = Some(thread.clone());
                }
            }
            Ok(handout.speaking_in(thread))
        }
        Err(why) => {
            tracing::warn!(%job, %why, "the job's thread could not be opened");
            Err(Progress::Failed(format!(
                "its channel could not be reached: {why}"
            )))
        }
    }
}

/// Runs a job that has been recorded, to completion.
///
/// Returns rather than fails when the job goes badly: a job that runs and
/// fails is a recorded outcome and not an error, because the job happened.
pub async fn supervise(
    store: &Store,
    runtime: &ContainerRuntime,
    started: Started,
) -> (JobId, Progress) {
    let Started {
        job,
        handout,
        kickoff,
        announcement,
    } = started;

    // Before the container, because a container is given the thread it speaks
    // in at creation and never afterwards — and after the record, because
    // `begin` has already written the job. Killed between posting and
    // recording, this leaves a thread nothing points at and the job opens
    // another on the next attempt. That window is one snapshot write wide and
    // the cost of losing it is a duplicate message rather than duplicated
    // work, which is the trade `docs/decisions/0015-a-job-survives-the-daemon-dying.md`
    // takes seriously for containers and can afford to take lightly here.
    let handout = match opening(store, job, handout, &announcement).await {
        Ok(handout) => handout,
        Err(progress) => {
            record(store, job, progress.clone());
            return (job, progress);
        }
    };

    let progress = match stageman_job::start(runtime, &handout, job, &kickoff).await {
        Ok(answer) => outcome(&answer),
        Err(error) => Progress::Failed(because(&error)),
    };
    // Derived from the outcome rather than from a second look at the stop
    // reason. Two conditions saying the same thing can disagree, and mutation
    // testing found this one untested when it was separate.
    if let Progress::Failed(ref why) = progress {
        tracing::warn!(%job, %why, "the job did not finish");
    }

    record(store, job, progress.clone());
    // Said whichever way it went. The agent has already reported for itself if
    // it could; this says the one thing the agent cannot, which is that it has
    // stopped and a reply now reaches it — and it is the only thing said at all
    // when the agent was what failed.
    notice(store, job, stageman_foreman::attention_notice()).await;
    (job, progress)
}

/// Says something in a job's thread, on the instance's own behalf.
///
/// Silent when the job has no thread, which is every job on a project with no
/// channel bound. A failure to speak is logged and nothing more: this is a
/// notice *about* an outcome, and failing the job over it would mean the
/// outcome changed because the announcement of it did not arrive.
#[mutants::skip]
async fn notice(store: &Store, job: JobId, text: &str) {
    let speaking = {
        let state = store.read();
        let found = speaking_for(&state, job);
        drop(state);
        found
    };

    let Some((bound, thread)) = speaking else {
        return;
    };
    if let Err(why) = crate::channel::say_in(&bound, &thread, text).await {
        tracing::warn!(%job, %why, "the job's thread could not be spoken to");
    }
}

/// Where to speak on a job's behalf, if it has anywhere.
///
/// Pure, so that the several ways of having nowhere — no project, no thread,
/// a thread on a channel the project no longer binds — are testable without a
/// network. The function above only does the speaking.
fn speaking_for(state: &State, job: JobId) -> Option<(Speaking, Thread)> {
    let project = state.project_of(job)?;
    let thread = state.job(job)?.thread.clone()?;
    let bound = state
        .projects
        .get(&project)?
        .channels
        .get(&thread.channel)?
        .speaking();
    Some((bound, thread))
}

/// Whether a job can take a reply now, taking it if so.
///
/// **The check and the transition are one operation on purpose.** Split, two
/// replies arriving together would both find the job idle and resume one
/// container twice; together, the second finds it running and is refused. That
/// makes the refusal a person sees for a genuinely busy job and the refusal
/// that prevents a collision the same code, which is right — from the outside
/// they are the same situation.
fn accepting(state: &mut State, job: JobId) -> Accepted {
    let Some(recorded) = state.job_mut(job) else {
        return Accepted::Unknown;
    };
    if recorded.progress == Progress::Working {
        return Accepted::Busy;
    }
    recorded.progress = Progress::Working;
    Accepted::Taken
}

/// What [`accepting`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Accepted {
    /// The job was idle and is now running.
    Taken,
    /// It was already working, so nothing was taken.
    Busy,
    /// This instance has no such job.
    Unknown,
}

/// Hands a reply to the job whose thread it arrived in.
///
/// **The busy check and the transition to running happen under one lock**, and
/// that is what serialises replies rather than anything else: two arriving at
/// once would otherwise both find the job idle and resume the same container
/// twice. The second one is refused and told so, which is the same answer a
/// person gets for replying to a job that is genuinely still working — because
/// from the outside it is the same situation.
///
/// Answering a job this instance does not have, or one with nothing to resume,
/// is deliberately not this function's problem to hide: both come back as a
/// refusal the caller says on the thread.
pub async fn deliver(
    store: &Store,
    runtime: &ContainerRuntime,
    job: JobId,
    said: &str,
) -> Progress {
    let taken = {
        let mut state = store.update();
        let taken = accepting(&mut state, job);
        drop(state);
        taken
    };
    match taken {
        Accepted::Unknown => return Progress::Failed("no such job".to_owned()),
        Accepted::Busy => {
            notice(store, job, stageman_foreman::busy_notice()).await;
            return Progress::Working;
        }
        Accepted::Taken => {}
    }

    // A job's thread never changes, but it still has to be written in before
    // each start: an environment is fixed at creation, so nothing the container
    // was given at birth can be counted on to still be there in the shape a
    // long-lived one needs. Read from the record rather than remembered.
    let speaking = {
        let state = store.read();
        let thread = state.job(job).and_then(|recorded| recorded.thread.clone());
        drop(state);
        thread
    };

    let progress = match stageman_job::resume(runtime, job, speaking.as_ref(), said).await {
        Ok(answer) => outcome(&answer),
        Err(error) => Progress::Failed(because(&error)),
    };
    if let Progress::Failed(ref why) = progress {
        tracing::warn!(%job, %why, "the reply did not reach the job");
    }
    record(store, job, progress.clone());
    notice(store, job, stageman_foreman::attention_notice()).await;
    progress
}

/// Creates a job on a project and runs it to completion.
///
/// The whole of the doing, in the one crate allowed to name both the store and
/// the job — `docs/architecture.md` §1. What the foreman will eventually
/// decide (which project, which agent, why, and what work) arrives here as
/// arguments, because nothing decides it yet.
///
/// Both halves, for a caller that has nothing else to do until the job is
/// over. A caller that does — a request, most obviously — uses [`begin`] and
/// [`supervise`] separately.
///
/// # Errors
///
/// Fails if the project is unknown, or if a handout cannot be decided for it.
/// A job that *runs* and fails is not an error here: it is a recorded outcome,
/// returned as [`Progress::Failed`], because the job happened.
pub async fn run(
    store: &Store,
    runtime: &ContainerRuntime,
    project: ProjectId,
    agent: Agent,
    reason: &str,
    work: &str,
) -> Result<(JobId, Progress), RunError> {
    let started = begin(store, project, agent, reason, work)?;

    Ok(supervise(store, runtime, started).await)
}

/// What a sweep found, and what it did about it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Swept {
    /// Jobs put back to work.
    pub resumed: usize,
    /// Jobs whose resumption failed, and which are recorded as failed.
    pub failed: usize,
    /// Containers whose names this version does not understand.
    ///
    /// Almost always a container from an older version of this project: odd,
    /// and benign.
    pub unidentified: usize,
    /// Containers naming a job this instance has no record of.
    ///
    /// Counted apart from the above because it means something worse. The name
    /// parsed, so the container was made by a version that names them as this
    /// one does — and the job it names is *gone from the instance*. That is a
    /// snapshot restored from an older backup, or hand-edited, or a write that
    /// never landed: work exists that this instance no longer knows it asked
    /// for. Blurring the two would hide the serious one behind the harmless
    /// one.
    pub forgotten: usize,
    /// Jobs believed to be running with no container to run in.
    pub stranded: usize,
}

/// Reconciles what the runtime actually has against what the instance believes.
///
/// The last piece of `docs/decisions/0015-a-job-survives-the-daemon-dying.md`,
/// and it runs in **app** because reconciling needs both the store and the
/// doing, and this is the only crate allowed to name both —
/// `docs/architecture.md` §1.
///
/// It walks the two directions separately, because they answer different
/// questions and only one of them is about work. From *jobs* to containers:
/// every job the instance believes is running goes back to work. From
/// *containers* to jobs: anything the instance cannot place is reported. The
/// second exists because the first can only ever see what the instance already
/// knows, and a container it has lost is exactly the one worth finding — which
/// is the other half of `docs/conventions.md` §4's bar.
///
/// **Nothing unplaceable is removed**, and there are two ways to be
/// unplaceable — a name this version cannot read, and a name that reads
/// perfectly and points at a job the instance has lost. The second is the
/// worse one and is reported apart from the first. A container is where a job's work
/// lives, so "I did not recognise this, so I deleted it" is the wrong answer
/// when the instance is the thing that is wrong — a snapshot restored from an
/// older backup, most obviously. Reporting costs an operator a decision;
/// removing could cost them the work. Retention proper is still open in
/// `docs/open-questions.md`.
///
/// It resumes each job in turn and waits for it, which is right while nothing
/// else is running and wrong the moment there is a dashboard to serve.
/// `docs/conventions.md` §3 says foreman work never happens on the
/// request path; this is not on one yet, but a startup that waits for several
/// agents to finish is a dashboard that does not appear for minutes. Whoever
/// adds the server moves this onto its own task, and the shape of it will want
/// a span per job — see `docs/open-questions.md` on where log lines go.
///
/// # Errors
///
/// Fails only if the runtime will not say what containers it has. A single
/// job that cannot be resumed is recorded as failed and does not stop the
/// others: one broken job is not a reason to abandon the rest.
pub async fn reconcile(
    store: &Store,
    runtime: &ContainerRuntime,
) -> Result<Swept, stageman_job::JobError> {
    let left = stageman_job::left_behind(runtime).await?;

    let unplaceable = unplaceable(&left, &store.read());
    for container in &unplaceable {
        match container {
            Unplaceable::Unidentified(name) => tracing::warn!(
                container = %name,
                "a container this instance started, under a name it does not understand; \
                 left alone rather than removed"
            ),
            Unplaceable::Forgotten(name, job) => tracing::warn!(
                container = %name,
                %job,
                "a container naming a job this instance has no record of — the instance may \
                 have lost work it started; left alone rather than removed"
            ),
        }
    }

    // Counted by collecting rather than by incrementing. The gate denies
    // arithmetic that can overflow, and the escape hatches that quiet it are
    // exactly the ones `.quality/gate-reference.md` warns produce silent wrong
    // values — a counter is not worth either.
    let believed_working: Vec<JobId> = store.read().working().collect();
    let mut attended: Vec<Attended> = Vec::new();

    for job in believed_working {
        if !has_container(&left, job) {
            // Believed running with nothing to run in. Not resumable and not
            // removable, because there is nothing there — so it is recorded as
            // failed, which is the one outcome that stops it being swept for
            // ever afterwards.
            tracing::warn!(%job, "believed to be running, but it has no container");
            record(
                store,
                job,
                Progress::Failed("its container is gone".to_owned()),
            );
            attended.push(Attended::Stranded);
            continue;
        }

        let speaking = {
            let state = store.read();
            let thread = state.job(job).and_then(|recorded| recorded.thread.clone());
            drop(state);
            thread
        };
        let progress = match stageman_job::resume(
            runtime,
            job,
            speaking.as_ref(),
            stageman_foreman::resumption_notice(),
        )
        .await
        {
            Ok(answer) => outcome(&answer),
            Err(error) => Progress::Failed(because(&error)),
        };
        if let Progress::Failed(ref why) = progress {
            tracing::warn!(%job, %why, "could not be put back to work");
        }
        attended.push(Attended::from(&progress));
        record(store, job, progress);
    }

    Ok(tallied(&attended, &unplaceable))
}

/// What a sweep did about one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attended {
    /// Put back to work, and it finished.
    Resumed,
    /// Tried and did not finish, or could not be tried at all.
    Failed,
    /// Believed to be running with no container to run in.
    Stranded,
}

impl From<&Progress> for Attended {
    /// Only a job that finished counts as resumed.
    ///
    /// Written as a conversion rather than as a branch inside the sweep
    /// because mutation testing found the branch untested: inverting it swaps
    /// the resumed and failed tallies, which is a sweep that reports the
    /// opposite of what happened, and nothing noticed.
    fn from(progress: &Progress) -> Self {
        match progress {
            Progress::Idle => Self::Resumed,
            Progress::Working | Progress::Failed(_) => Self::Failed,
        }
    }
}

/// Counts what a sweep did.
///
/// Separated from doing it so the arithmetic can be checked without a
/// container runtime. Counting is exactly the kind of code that is obviously
/// right and occasionally is not.
fn tallied(attended: &[Attended], unplaceable: &[Unplaceable<'_>]) -> Swept {
    let counted = |wanted: Attended| attended.iter().filter(|had| **had == wanted).count();
    Swept {
        resumed: counted(Attended::Resumed),
        failed: counted(Attended::Failed),
        stranded: counted(Attended::Stranded),
        unidentified: unplaceable
            .iter()
            .filter(|container| matches!(container, Unplaceable::Unidentified(_)))
            .count(),
        forgotten: unplaceable
            .iter()
            .filter(|container| matches!(container, Unplaceable::Forgotten(..)))
            .count(),
    }
}

/// Why a container could not be matched to work this instance knows about.
///
/// Two cases and not one, because an operator does something different about
/// each. Pulled out of [`reconcile`] so the distinction can be tested without
/// a container runtime: it is a judgement about names and records, and neither
/// needs anything running to check.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unplaceable<'a> {
    /// Its name says nothing this version understands.
    Unidentified(&'a str),
    /// Its name says which job, and this instance has no such job.
    Forgotten(&'a str, JobId),
}

/// Everything the runtime has that the instance cannot account for.
/// Whether anything left behind is this job's container.
///
/// A named function rather than the closure it was, for the reason
/// `unplaceable` below is one: the decision is testable without a container
/// runtime and the loop it came from is not, so inline it was reachable only
/// by tests that `just check` skips — which is the same as being untested from
/// the gate's point of view. Mutation testing is what said so.
fn has_container(left: &[stageman_job::Abandoned], job: JobId) -> bool {
    left.iter().any(|abandoned| abandoned.job == Some(job))
}

fn unplaceable<'a>(left: &'a [stageman_job::Abandoned], state: &State) -> Vec<Unplaceable<'a>> {
    left.iter()
        .filter_map(|abandoned| match abandoned.job {
            // A name no *job* claims may still be a foreman's, and a foreman's
            // is placed rather than reported: it belongs to a project this
            // instance watches and is exactly where that foreman's session
            // lives. Without this it would be counted as a name this version
            // cannot read — which `Swept::unidentified` describes as odd and
            // benign, and a foreman's container is neither.
            None => match stageman_foreman::project_of(&abandoned.container) {
                Some(project) if state.projects.contains_key(&project) => None,
                // A foreman's container for a project that is gone is the
                // same loss as a forgotten job's: the name parsed, and what it
                // names is not here.
                Some(_) | None => Some(Unplaceable::Unidentified(&abandoned.container)),
            },
            Some(job) if state.job(job).is_none() => {
                Some(Unplaceable::Forgotten(&abandoned.container, job))
            }
            Some(_) => None,
        })
        .collect()
}

/// What an agent's answer means for the job that produced it.
///
/// Pulled out of the two callers that had it inline, and tested, because
/// mutation testing found it untested in both: the guard could be replaced
/// with `true`, with `false`, or inverted, and every test still passed. It is
/// the line that decides whether a job succeeded, which makes it close to the
/// worst line in this crate to have had no test — a system that records
/// failures as successes is worse than one that records nothing.
///
/// Anything short of finishing the turn is a failure, and the stop reason is
/// carried into the message rather than collapsed. A turn cut off by a token
/// limit and one the agent refused are both "not finished", and an operator
/// does something different about each.
fn outcome(answer: &Answer) -> Progress {
    if answer.stop_reason == StopReason::EndTurn {
        Progress::Idle
    } else {
        Progress::Failed(format!("the agent stopped: {:?}", answer.stop_reason))
    }
}

/// Writes what became of a job, and persists it.
///
/// Silent when the job is not there: the only caller has just read it out of
/// this same store, and a job removed in between is not something this could
/// report to anybody who could act on it.
fn record(store: &Store, job: JobId, progress: Progress) {
    let mut state = store.update();
    if let Some(recorded) = state.job_mut(job) {
        recorded.progress = progress;
    }
}

/// Hands a message to a project's foreman, and keeps it working until its
/// inbox is empty.
///
/// Skipped by mutation testing because it decides nothing: whether this call
/// drives the loop, what to work on next, and what happens when a turn ends
/// are three functions beside it, all tested without a runtime. What is left
/// here is a loop and two awaits.
///
/// **One turn is not the unit; draining is.** A foreman that took a message,
/// answered it and stopped would leave whatever arrived meanwhile waiting for
/// the next arrival to wake it — so the loop here is what
/// `Attending::finish` exists for, and running until it answers `None` is the
/// only thing that returns a foreman to idle.
///
/// Returns without doing anything when the foreman is already working: the
/// message has been put in its inbox by then, and whichever call is running
/// the loop will reach it. That is the same one-operation rule the inbox is
/// built on — two messages arriving together cannot both start a loop, because
/// only one of them finds it idle.
#[mutants::skip]
pub async fn attend(store: &Store, runtime: &ContainerRuntime, project: ProjectId, said: Errand) {
    // Kept before the message is moved into the inbox, because the answer to
    // it belongs under the message that asked.
    let thread = said.thread.clone();
    let (started, ahead) = {
        let mut state = store.update();
        let attending = state
            .projects
            .get_mut(&project)
            .map(|watched| &mut watched.attending);
        let outcome = attending.map(|attending| {
            let taken = attending.take(said);
            // Counting the one in hand: from outside, everything not yet
            // answered is ahead of this.
            let ahead = match taken {
                Taken::Started => 0,
                Taken::Waiting => attending.waiting(),
            };
            (taken, ahead)
        });
        drop(state);
        match outcome {
            Some((taken, ahead)) => (Some(taken), ahead),
            None => (None, 0),
        }
    };

    // Said at once, before any work. A foreman three messages behind is silent
    // for a while, and somebody who hears nothing cannot tell queued from
    // ignored.
    if started.is_some() {
        notice_in(
            store,
            project,
            &thread,
            &stageman_foreman::received_notice(ahead),
        )
        .await;
    }

    if !drives(started) {
        return;
    }

    loop {
        // Read and released before the turn, never held across it. Kept in the
        // `while let` scrutinee this lock lived until the end of the body —
        // which is an await and a write — so the first turn would have waited
        // on a lock it was itself holding. The compiler's lint about a
        // temporary with a significant drop is what caught it.
        let waiting = {
            let state = store.read();
            let waiting = waiting_on(&state, project);
            drop(state);
            waiting
        };
        let Some(errand) = waiting else {
            break;
        };
        let outcome = turn(store, runtime, project, &errand).await;
        if let Err(why) = outcome {
            // Logged and moved past rather than retried. A message that cannot
            // be handled must not become a message that is handled for ever,
            // and the person who sent it is told on their own thread.
            tracing::warn!(%project, %why, "the foreman's turn did not finish");
            notice_in(
                store,
                project,
                &errand.thread,
                stageman_foreman::stuck_notice(),
            )
            .await;
        }
        let done = {
            let mut state = store.update();
            let done = state
                .projects
                .get_mut(&project)
                .is_none_or(|watched| watched.attending.finish().is_none());
            drop(state);
            done
        };
        if done {
            break;
        }
    }
}

/// Whether this call is the one that drives the foreman's loop.
///
/// Only the message that found it idle does. Extracted because it is a
/// comparison, and mutation testing inverted it without a test noticing —
/// which would have every message either run a second loop over the same
/// container or return having done nothing at all.
const fn drives(taken: Option<Taken>) -> bool {
    matches!(taken, Some(Taken::Started))
}

/// What the foreman should be working on now, if anything.
///
/// Over a state rather than a store, so the answer can be asked for without
/// one — the same split the rest of this crate uses wherever a decision would
/// otherwise be reachable only through I/O.
fn waiting_on(state: &State, project: ProjectId) -> Option<Errand> {
    state
        .projects
        .get(&project)
        .and_then(|watched| watched.attending.on().cloned())
}

/// Runs one of a foreman's turns.
///
/// The handout is narrowed to the thread the message arrived in, so everything
/// the foreman says while answering lands under what it is answering. That is
/// the whole of why an `Errand` carries a thread.
#[mutants::skip]
async fn turn(
    store: &Store,
    runtime: &ContainerRuntime,
    project: ProjectId,
    errand: &Errand,
) -> Result<(), RunError> {
    // Minted here rather than in the domain, which takes no randomness by
    // design, and lazily rather than at project creation: a project whose
    // foreman never runs never needs one, and every project that already
    // exists has none.
    {
        let mut state = store.update();
        if let Some(watched) = state.projects.get_mut(&project)
            && watched.warrant.is_none()
        {
            watched.warrant = Some(minted_warrant());
        }
        drop(state);
    }

    let (repository, handout) = {
        let state = store.read();
        let repository = state
            .projects
            .get(&project)
            .ok_or(RunError::UnknownProject(project))?
            .repository
            .clone();
        let handout = Handout::for_foreman(&state, project)
            .map_err(RunError::Handout)?
            .speaking_in(errand.thread.clone());
        drop(state);
        (repository, handout)
    };

    // Read now rather than at the start of the session, because a project's
    // set of job agents is edited from the dashboard and a session outlives
    // those edits.
    let agents: Vec<(&'static str, &'static str)> = {
        let state = store.read();
        let named = state
            .projects
            .get(&project)
            .map_or_else(Vec::new, |watched| {
                watched
                    .job_agents
                    .iter()
                    .map(|agent| (crate::dashboard::wire_name(*agent).0, agent.description()))
                    .collect()
            });
        drop(state);
        named
    };

    stageman_foreman::attend(
        runtime,
        &handout,
        project,
        &repository,
        &crate::tooling::endpoint(*crate::asking::PORT),
        &agents,
        &errand.said,
    )
    .await
    .map(drop)
    .map_err(|why| RunError::Foreman(why.to_string()))
}

/// A fresh warrant.
///
/// Two version-four identifiers, which is what this crate already mints
/// anything unguessable from — a job's identifier and a project's are both
/// this. Using the same source keeps one answer to "where does an unguessable
/// value here come from" rather than introducing an encoding and a second
/// generator for one field.
///
/// Two rather than one for margin: one is already past brute force at 122
/// bits, and a warrant is the single thing standing between any container on
/// the machine and a foreman's authority. Measured, not assumed: an unrelated
/// container reaches this daemon as easily as a foreman's does.
fn minted_warrant() -> stageman_core::Secret {
    stageman_core::Secret::new(format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()))
}

/// Says something in a thread on the instance's own behalf.
///
/// Skipped by mutation testing for the reason the other speaking functions
/// are: it decides nothing. Where to speak is `speaking_for` above, and what
/// to say is authored in the foreman crate and asserted there.
#[mutants::skip]
async fn notice_in(store: &Store, project: ProjectId, thread: &Thread, text: &str) {
    let speaking = {
        let state = store.read();
        let bound = state
            .projects
            .get(&project)
            .and_then(|watched| watched.channels.get(&thread.channel))
            .map(stageman_core::ChannelConfig::speaking);
        drop(state);
        bound
    };
    let Some(bound) = speaking else {
        return;
    };
    if let Err(why) = crate::channel::say_in(&bound, thread, text).await {
        tracing::warn!(%project, %why, "the thread could not be answered");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Attended, LoadError, Store, Swept, Unplaceable, has_container, outcome, reconcile, run,
        tallied, unplaceable,
    };
    use stageman_agent::{Answer, ContainerRuntime, StopReason};
    use stageman_core::{
        Agent, AgentConfig, Job, JobId, Key, Progress, Project, ProjectId, Secret, State, Taken,
        Thread, Timestamp, Uuid,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn key() -> Key {
        Key::new([3; 32])
    }

    /// An instance with one agent configured and nothing else.
    fn configured() -> State {
        State {
            agents: BTreeMap::from([(
                Agent::Claude,
                AgentConfig {
                    auth_token: Secret::new("agent-token".to_owned()),
                },
            )]),
            ..State::default()
        }
    }

    /// An instance watching one project, with one job on it, running.
    fn an_instance_with_a_job() -> (State, JobId) {
        let mut state = configured();
        let project = ProjectId::from_uuid(Uuid::from_u128(11));
        let job = JobId::from_uuid(Uuid::from_u128(12));
        state.projects.insert(
            project,
            Project {
                name: "example".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                foreman_agent: Agent::Claude,
                job_agents: only_claude(),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                warrant: None,
                attending: stageman_core::Attending::default(),
                jobs: BTreeMap::from([(
                    job,
                    Job {
                        agent: Agent::Claude,
                        reason: "started by hand".to_owned(),
                        kickoff: "do the thing".to_owned(),
                        created_at: Timestamp::UNIX_EPOCH,
                        progress: Progress::Working,
                        thread: None,
                    },
                )]),
            },
        );
        (state, job)
    }

    fn only_claude() -> BTreeSet<Agent> {
        BTreeSet::from([Agent::Claude])
    }

    /// A recorded failure carries the reason, not just the category.
    ///
    /// The test that would have caught the thing this fixed: a job whose
    /// agent was rejected recorded "the job's agent could not be run" and
    /// nothing else, so the only way to learn that a credential had been
    /// refused was to ask the container runtime for the container's output.
    #[test]
    fn a_failure_records_what_was_underneath_it() {
        let underneath = stageman_agent::AgentError::Unusable {
            path: PathBuf::from("/usr/local/bin/docker"),
            message: "401 API key is invalid".to_owned(),
        };
        let reported = stageman_job::JobError::Agent(underneath);

        let told = super::because(&reported);

        assert!(
            told.starts_with("the job's agent could not be run"),
            "it should still say what kind of failure this was: {told}"
        );
        assert!(
            told.contains("401 API key is invalid"),
            "it should say what actually went wrong: {told}"
        );
    }

    /// An error with nothing underneath reads exactly as it always did.
    #[test]
    fn a_failure_with_no_cause_is_unchanged() {
        let alone = stageman_agent::AgentError::NoChannel;

        assert_eq!(
            super::because(&alone),
            "the container runtime offered no channel to speak the protocol over"
        );
    }

    /// A missing file is a first run; anything else is a failure.
    ///
    /// The distinction is one match guard, and inverting it turns every
    /// unreadable instance into a silent fresh one — which would lose an
    /// operator's whole configuration rather than refusing to start.
    #[test]
    fn an_instance_that_cannot_be_read_is_not_mistaken_for_a_first_run() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        // A directory, which exists and cannot be read as a file. Chosen over
        // permissions, which behave differently when the tests run as root.
        let outcome = Store::load(directory.path().to_owned(), key());

        assert!(
            matches!(outcome, Err(LoadError::Read(_))),
            "a path that is not a readable file must not read as a first run"
        );
    }

    /// Borrowing for modification shows the state that is actually there.
    #[test]
    fn the_guard_that_writes_on_release_reads_the_real_state() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store =
            Store::create(snapshot_path(&directory), key(), configured()).expect("it can write");

        let borrowed = store.update();
        let configured_agents = borrowed.agents.len();
        let names_claude = borrowed.agents.contains_key(&Agent::Claude);
        // Released before asserting: the guard writes a snapshot when it ends,
        // and a failing assertion inside its lifetime would drop it while
        // unwinding.
        drop(borrowed);

        assert_eq!(configured_agents, 1);
        assert!(names_claude);
    }

    #[test]
    fn a_job_is_only_matched_to_a_container_that_names_it() {
        let job = JobId::from_uuid(Uuid::from_u128(7));
        let other = JobId::from_uuid(Uuid::from_u128(8));
        let left = [
            stageman_job::Abandoned {
                container: "stageman-job-unidentified".to_owned(),
                job: None,
            },
            stageman_job::Abandoned {
                container: format!("stageman-job-{other}"),
                job: Some(other),
            },
        ];

        assert!(!has_container(&left, job));
        assert!(has_container(&left, other));
        assert!(!has_container(&[], job));
    }

    fn snapshot_path(directory: &TempDir) -> PathBuf {
        directory.path().join("state.json")
    }

    fn temporaries_in(directory: &Path) -> usize {
        std::fs::read_dir(directory)
            .expect("the directory exists")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
            .count()
    }

    #[test]
    fn a_missing_snapshot_is_a_first_run_rather_than_a_failure() {
        let directory = TempDir::new().expect("a temporary directory");
        let opened =
            Store::load(snapshot_path(&directory), key()).expect("absence is not an error");
        assert!(opened.is_none());
    }

    #[test]
    fn creating_an_instance_writes_it_before_anything_can_depend_on_it() {
        // The point of writing at startup: a bad path or a read-only directory
        // fails here rather than at the first state change, hours later.
        let directory = TempDir::new().expect("a temporary directory");
        let path = snapshot_path(&directory);
        let _store = Store::create(path.clone(), key(), configured()).expect("it can write");
        assert!(path.exists());
        assert_eq!(temporaries_in(directory.path()), 0);
    }

    #[test]
    fn creating_an_instance_somewhere_unwritable_fails_immediately() {
        let directory = TempDir::new().expect("a temporary directory");
        let path = directory.path().join("no").join("such").join("place.json");
        assert!(matches!(
            Store::create(path, key(), configured()),
            Err(LoadError::Write(_))
        ));
    }

    #[test]
    fn a_change_survives_being_reloaded() {
        let directory = TempDir::new().expect("a temporary directory");
        let path = snapshot_path(&directory);
        let id = ProjectId::from_uuid(Uuid::from_u128(11));
        {
            let store = Store::create(path.clone(), key(), configured()).expect("it can write");
            let mut state = store.update();
            state.projects.insert(
                id,
                Project {
                    name: "example".to_owned(),
                    repository: "https://example.invalid/repo".to_owned(),
                    foreman_agent: Agent::Claude,
                    job_agents: only_claude(),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    warrant: None,
                    attending: stageman_core::Attending::default(),
                },
            );
        }
        let reopened = Store::load(path, key())
            .expect("it reads back")
            .expect("and there is something there");
        assert!(reopened.read().projects.contains_key(&id));
    }

    #[test]
    fn reading_does_not_rewrite_the_snapshot() {
        // Every write re-seals with fresh nonces, so an unnecessary write is
        // visible as changed bytes. That is what makes this assertion sharp.
        let directory = TempDir::new().expect("a temporary directory");
        let path = snapshot_path(&directory);
        let store = Store::create(path.clone(), key(), configured()).expect("it can write");
        let before = std::fs::read(&path).expect("the file is there");
        drop(store.read());
        let after = std::fs::read(&path).expect("the file is still there");
        assert_eq!(before, after);
    }

    #[test]
    fn a_change_does_rewrite_the_snapshot() {
        // The counterpart to the test above: if this ever stops holding, the
        // guarantee the guard exists for is gone.
        let directory = TempDir::new().expect("a temporary directory");
        let path = snapshot_path(&directory);
        let store = Store::create(path.clone(), key(), configured()).expect("it can write");
        let before = std::fs::read(&path).expect("the file is there");
        drop(store.update());
        let after = std::fs::read(&path).expect("the file is still there");
        assert_ne!(before, after);
    }

    #[test]
    fn the_wrong_key_does_not_open_an_instance() {
        let directory = TempDir::new().expect("a temporary directory");
        let path = snapshot_path(&directory);
        drop(Store::create(path.clone(), key(), configured()).expect("it can write"));
        assert!(matches!(
            Store::load(path, Key::new([4; 32])),
            Err(LoadError::Open(_))
        ));
    }

    /// A runtime that answers every query with nothing, so a sweep can be
    /// tested without a container. `docker ps` returning no lines is exactly
    /// what an instance with nothing left behind looks like.
    fn empty_runtime() -> ContainerRuntime {
        let accepting = ["/usr/bin/true", "/bin/true"]
            .into_iter()
            .map(PathBuf::from)
            .find(|candidate| candidate.exists())
            .expect("a standard utility that succeeds");
        ContainerRuntime::new(accepting)
    }

    fn with_a_running_job() -> (State, JobId) {
        let mut state = configured();
        let project = ProjectId::from_uuid(Uuid::from_u128(1));
        let job = JobId::from_uuid(Uuid::from_u128(2));
        let mut jobs = BTreeMap::new();
        jobs.insert(
            job,
            Job {
                agent: Agent::Claude,
                reason: "an issue was opened".to_owned(),
                kickoff: "work on it".to_owned(),
                created_at: Timestamp::UNIX_EPOCH,
                progress: Progress::Working,
                thread: None,
            },
        );
        state.projects.insert(
            project,
            Project {
                name: "example".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                foreman_agent: Agent::Claude,
                job_agents: only_claude(),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                jobs,
                warrant: None,
                attending: stageman_core::Attending::default(),
            },
        );
        (state, job)
    }

    /// The rule that serialises replies, and the one the user meets.
    ///
    /// Mutation testing found this untested by inverting the comparison, which
    /// would deliver every reply to a job that was working and refuse every
    /// one to a job that was idle — the exact opposite of the behaviour, with
    /// no test going red.
    #[test]
    fn a_reply_is_taken_only_by_a_job_that_is_not_working() {
        let (mut state, job) = an_instance_with_a_job();

        // Running, so nothing is taken and nothing is changed.
        assert_eq!(super::accepting(&mut state, job), super::Accepted::Busy);
        assert_eq!(
            state.job(job).expect("the job").progress,
            Progress::Working,
            "a refused reply must not move the job"
        );

        // Finished, so the reply is taken and the job goes back to work.
        state.job_mut(job).expect("the job").progress = Progress::Idle;
        assert_eq!(super::accepting(&mut state, job), super::Accepted::Taken);
        assert_eq!(state.job(job).expect("the job").progress, Progress::Working);

        // And taking it once is what stops a second taking it as well, which
        // is the collision this exists to prevent.
        assert_eq!(super::accepting(&mut state, job), super::Accepted::Busy);
    }

    /// A job this instance never had is neither taken nor busy.
    #[test]
    fn a_reply_for_a_job_this_instance_does_not_have_is_unknown() {
        let (mut state, _) = an_instance_with_a_job();

        assert_eq!(
            super::accepting(&mut state, JobId::from_uuid(Uuid::from_u128(99))),
            super::Accepted::Unknown
        );
    }

    /// Where to speak, and the several ways of having nowhere.
    #[test]
    fn a_job_with_no_thread_has_nowhere_to_be_spoken_to() {
        let (mut state, job) = an_instance_with_a_job();

        // No thread: the ordinary case for a project with no channel bound.
        assert!(super::speaking_for(&state, job).is_none());

        // A thread, and a channel bound: somewhere to speak.
        state.job_mut(job).expect("the job").thread = Some(Thread {
            channel: stageman_core::Channel::Slack,
            id: "1728312345.678901".to_owned(),
        });
        let project = state.project_of(job).expect("the project");
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .channels
            .insert(
                stageman_core::Channel::Slack,
                stageman_core::ChannelConfig {
                    address: "C0123456789".to_owned(),
                    credential: Secret::new("xoxb-not-a-real-token".to_owned()),
                    listen_credential: None,
                },
            );
        let (bound, thread) = super::speaking_for(&state, job).expect("somewhere to speak");
        assert_eq!(bound.address, "C0123456789");
        assert_eq!(thread.id, "1728312345.678901");

        // A thread naming a channel the project no longer binds: nowhere
        // again, rather than a panic or the wrong channel.
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .channels
            .clear();
        assert!(super::speaking_for(&state, job).is_none());
    }

    /// A job the instance believes is running, with nothing to run in. It
    /// cannot be resumed and there is nothing to remove, so it is recorded as
    /// failed — which is also what stops it being swept for on every start
    /// from now on.
    #[tokio::test]
    async fn a_job_whose_container_is_gone_is_recorded_as_failed() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (state, job) = with_a_running_job();
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        let swept = reconcile(&store, &empty_runtime())
            .await
            .expect("the runtime answers");

        assert_eq!(swept.stranded, 1, "{swept:?}");
        assert_eq!(swept.resumed, 0);
        assert!(matches!(
            store.read().job(job).map(|j| j.progress.clone()),
            Some(Progress::Failed(_))
        ));
    }

    /// And having said so once, it must not say so again: a swept instance
    /// converges rather than reporting the same casualty on every start.
    #[tokio::test]
    async fn a_second_sweep_finds_nothing_left_to_do() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (state, _) = with_a_running_job();
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        let first = reconcile(&store, &empty_runtime()).await.expect("answers");
        let again = reconcile(&store, &empty_runtime()).await.expect("answers");

        assert_eq!(first.stranded, 1);
        assert_eq!(again, Swept::default(), "it should have settled");
    }

    /// The outcome of a sweep is written, not merely held: the point of
    /// recording a casualty is that the next start does not repeat it, and the
    /// next start reads a file.
    #[tokio::test]
    async fn what_a_sweep_decided_survives_reopening_the_instance() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = snapshot_path(&directory);
        let (state, job) = with_a_running_job();
        let store = Store::create(path.clone(), key(), state).expect("it can write");

        reconcile(&store, &empty_runtime()).await.expect("answers");
        drop(store);

        let reopened = Store::load(path, key())
            .expect("it opens")
            .expect("it is there");
        assert!(matches!(
            reopened.read().job(job).map(|j| j.progress.clone()),
            Some(Progress::Failed(_))
        ));
    }

    fn left(container: &str, job: Option<JobId>) -> stageman_job::Abandoned {
        stageman_job::Abandoned {
            container: container.to_owned(),
            job,
        }
    }

    /// The distinction that decides what an operator does next, checked without
    /// a container runtime because it is a judgement about names and records.
    #[test]
    fn a_name_that_cannot_be_read_and_one_naming_a_lost_job_are_told_apart() {
        let (state, known) = with_a_running_job();
        let lost = JobId::from_uuid(Uuid::from_u128(404));
        let containers = [
            left("stageman-job-from-an-older-scheme", None),
            left(&stageman_job::container(lost), Some(lost)),
            left(&stageman_job::container(known), Some(known)),
        ];

        let found = unplaceable(&containers, &state);

        assert_eq!(
            found,
            vec![
                Unplaceable::Unidentified("stageman-job-from-an-older-scheme"),
                Unplaceable::Forgotten(&stageman_job::container(lost), lost),
            ],
            "a container for a job the instance still has is placeable"
        );
    }

    /// A warrant is long, unguessable, and different every time.
    ///
    /// Anything that can reach the daemon may try to guess one — measured:
    /// an unrelated container reaches the host as easily as a foreman's does
    /// — so the only thing standing between a job's agent and a foreman's
    /// authority is that this is not worth attacking.
    #[test]
    fn a_minted_warrant_is_long_and_never_the_same_twice() {
        let first = super::minted_warrant();
        let second = super::minted_warrant();

        assert_eq!(first.expose().len(), 72, "two identifiers, hyphens and all");
        assert!(
            first
                .expose()
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-')
        );
        assert_ne!(
            first.expose(),
            second.expose(),
            "a warrant reused across projects would be one warrant"
        );
    }

    /// Only the message that found the foreman idle drives its loop.
    ///
    /// Inverting this comparison would have every other message start a second
    /// loop over the same container, and the one that should have started do
    /// nothing at all. Mutation testing found it untested.
    #[test]
    fn only_the_message_that_started_a_turn_drives_the_loop() {
        assert!(super::drives(Some(Taken::Started)));
        assert!(!super::drives(Some(Taken::Waiting)));
        // No project, so nothing was taken and there is nothing to drive.
        assert!(!super::drives(None));
    }

    /// What a foreman is working on is what its inbox says.
    #[test]
    fn what_a_foreman_is_working_on_comes_from_its_inbox() {
        let (mut state, _) = with_a_running_job();
        let project = *state.projects.keys().next().expect("a project");

        assert_eq!(
            super::waiting_on(&state, project),
            None,
            "idle holds nothing"
        );

        let errand = stageman_core::Errand {
            said: "look at the parser".to_owned(),
            thread: Thread {
                channel: stageman_core::Channel::Slack,
                id: "1788000000.000000".to_owned(),
            },
        };
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .attending
            .take(errand.clone());

        assert_eq!(super::waiting_on(&state, project), Some(errand));
        assert_eq!(
            super::waiting_on(&state, ProjectId::from_uuid(Uuid::from_u128(404))),
            None,
            "a project this instance does not watch has no inbox"
        );
    }

    /// A foreman's container is placed, not reported.
    ///
    /// It carries this project's label and names no job, which is exactly the
    /// shape `Swept::unidentified` describes as *a container from an older
    /// version: odd, and benign*. A foreman's is neither, and it is where that
    /// foreman's whole session lives — so reporting it would teach an operator
    /// to distrust a count that was right.
    #[test]
    fn a_foremans_container_is_placed_rather_than_reported() {
        let (mut state, _) = with_a_running_job();
        let watched = *state.projects.keys().next().expect("a project");
        let containers = [left(&stageman_foreman::container(watched), None)];

        assert_eq!(
            unplaceable(&containers, &state),
            vec![],
            "a foreman's container belongs to a project this instance watches"
        );

        // And one whose project is gone is a loss, like a forgotten job's:
        // the name parsed and what it names is not here.
        let forgotten = stageman_foreman::container(ProjectId::from_uuid(Uuid::from_u128(404)));
        let containers = [left(&forgotten, None)];
        assert_eq!(
            unplaceable(&containers, &state),
            vec![Unplaceable::Unidentified(&forgotten)]
        );

        // Removing the project it belongs to turns the first case into the
        // second, which is what makes this about the record rather than the
        // name.
        state.projects.clear();
        let named = stageman_foreman::container(watched);
        assert_eq!(
            unplaceable(&[left(&named, None)], &state),
            vec![Unplaceable::Unidentified(&named)]
        );
    }

    /// The serious case has to carry the identifier, because it is the only
    /// thing an operator can search their backups for.
    #[test]
    fn a_container_naming_a_forgotten_job_reports_which_job() {
        let (state, _) = with_a_running_job();
        let lost = JobId::from_uuid(Uuid::from_u128(7));

        let containers = [left(&stageman_job::container(lost), Some(lost))];
        let found = unplaceable(&containers, &state);

        assert!(
            matches!(found.first(), Some(Unplaceable::Forgotten(_, job)) if *job == lost),
            "{found:?}"
        );
    }

    /// Tests that spend real money and reach the network, grouped so a filter
    /// can name them. Run with `just image-session`.
    mod costs_a_credential {
        use super::*;

        /// A repository small enough to clone in a test, public enough to need
        /// no credential, and owned by the platform itself so it will not
        /// vanish.
        const PUBLIC_REPOSITORY: &str = "https://github.com/octocat/Hello-World";

        fn credential() -> Secret {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../.local/anthropic-token");
            let raw = std::fs::read_to_string(path)
                .expect("write an agent credential to .local/anthropic-token (it is gitignored)");
            Secret::new(raw.trim().to_owned())
        }

        fn located_runtime() -> ContainerRuntime {
            let located = std::process::Command::new("sh")
                .args(["-c", "command -v docker"])
                .output()
                .expect("looking for a container runtime");
            let path = String::from_utf8(located.stdout).expect("a runtime path is text");
            ContainerRuntime::new(PathBuf::from(path.trim()))
        }

        fn an_instance_watching(repository: &str) -> (State, ProjectId) {
            let mut state = State::default();
            state.agents.insert(
                Agent::Claude,
                AgentConfig {
                    auth_token: credential(),
                },
            );
            let project = ProjectId::from_uuid(Uuid::from_u128(11));
            state.projects.insert(
                project,
                Project {
                    name: "hello".to_owned(),
                    repository: repository.to_owned(),
                    foreman_agent: Agent::Claude,
                    job_agents: only_claude(),
                    // No platform credential: the repository is public, so this
                    // also checks that a job with nothing to authenticate with
                    // is a perfectly ordinary job rather than a broken one.
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    warrant: None,
                    attending: stageman_core::Attending::default(),
                },
            );
            (state, project)
        }

        /// One job, end to end: a project configured, an agent started in a
        /// container named for its job, a kickoff it did not write, and a
        /// repository it fetched itself.
        ///
        /// The clone is the part worth proving, because
        /// `docs/decisions/0016-the-agent-clones-the-repository.md` removed
        /// every mechanism that would have put a repository there — so the
        /// files being present is evidence the agent did it, and nothing else
        /// could have.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image, a credential and the network; run `just image-session`"]
        async fn a_job_runs_from_kickoff_to_a_cloned_repository() {
            let runtime = located_runtime();
            let directory = tempfile::tempdir().expect("a temporary directory");
            let (state, project) = an_instance_watching(PUBLIC_REPOSITORY);
            let store =
                Store::create(snapshot_path(&directory), key(), state).expect("it can write");

            let (job, progress) = run(
                &store,
                &runtime,
                project,
                Agent::Claude,
                "checking that a job can run at all",
                "Clone the repository. Then reply with one short line saying how many files are \
                 in it. Make no changes, commit nothing, and open nothing.",
            )
            .await
            .expect("the job is created");

            assert_eq!(progress, Progress::Idle, "the job did not finish");
            assert_eq!(
                store
                    .read()
                    .job(job)
                    .map(|recorded| recorded.progress.clone()),
                Some(Progress::Idle),
                "and the instance should have recorded that"
            );

            // The kickoff it was given is kept on the job, so what a job was
            // told survives the job — which is most of what makes a bad run
            // answerable afterwards.
            let told = store
                .read()
                .job(job)
                .map(|recorded| recorded.kickoff.clone())
                .expect("the job is recorded");
            assert!(told.contains(PUBLIC_REPOSITORY), "{told}");
            assert!(told.contains("open a pull request"), "{told}");

            // The repository really is in the container, and nothing but the
            // agent could have put it there.
            let workspace = directory.path().join("workspace");
            let copied = std::process::Command::new(runtime.path())
                .arg("cp")
                .arg(format!("{}:/workspace", stageman_job::container(job)))
                .arg(&workspace)
                .output()
                .expect("the runtime runs");
            assert!(
                copied.status.success(),
                "{}",
                String::from_utf8_lossy(&copied.stderr)
            );
            let cloned = std::fs::read_dir(&workspace)
                .expect("the workspace was copied out")
                .filter_map(Result::ok)
                .any(|entry| entry.path().join(".git").exists());
            assert!(cloned, "no clone in the workspace: {workspace:?}");

            stageman_job::discard(&runtime, job)
                .await
                .expect("it is removable");
        }
    }

    fn answered(stop_reason: StopReason) -> Answer {
        Answer {
            text: "whatever it said".to_owned(),
            stop_reason,
        }
    }

    /// The line mutation testing found untested. Finishing the turn is the
    /// only thing that counts as having finished.
    #[test]
    fn only_a_finished_turn_counts_as_a_completed_job() {
        assert_eq!(outcome(&answered(StopReason::EndTurn)), Progress::Idle);
    }

    /// Every other way a turn can end is a failure, and each is checked rather
    /// than one standing in for the rest: a guard replaced by `true` passes a
    /// test that only ever looks at the successful case.
    #[test]
    fn every_other_ending_is_a_failure() {
        for ending in [
            StopReason::MaxTokens,
            StopReason::MaxTurnRequests,
            StopReason::Refusal,
            StopReason::Cancelled,
        ] {
            assert!(
                matches!(outcome(&answered(ending)), Progress::Failed(_)),
                "{ending:?} should not read as success"
            );
        }
    }

    /// The stop reason survives into the message, because "it did not finish"
    /// and "it refused" send an operator to different places.
    #[test]
    fn a_failure_says_how_the_turn_ended() {
        let Progress::Failed(why) = outcome(&answered(StopReason::Refusal)) else {
            panic!("a refusal is not a completed job");
        };

        assert!(why.contains("Refusal"), "{why}");
    }

    /// The conversion mutation testing found untested. Inverting it makes a
    /// sweep report the opposite of what happened.
    #[test]
    fn only_a_completed_job_counts_as_resumed() {
        assert_eq!(Attended::from(&Progress::Idle), Attended::Resumed);
        assert_eq!(
            Attended::from(&Progress::Failed("anything".to_owned())),
            Attended::Failed
        );
        // Still running when the sweep looked is not success either: it means
        // the resume did not reach an ending.
        assert_eq!(Attended::from(&Progress::Working), Attended::Failed);
    }

    #[test]
    fn a_tally_counts_each_kind_separately() {
        let attended = [
            Attended::Resumed,
            Attended::Resumed,
            Attended::Failed,
            Attended::Stranded,
        ];
        let unplaceable = [
            Unplaceable::Unidentified("older-scheme"),
            Unplaceable::Forgotten("a-name", JobId::from_uuid(Uuid::from_u128(9))),
            Unplaceable::Forgotten("another", JobId::from_uuid(Uuid::from_u128(10))),
        ];

        assert_eq!(
            tallied(&attended, &unplaceable),
            Swept {
                resumed: 2,
                failed: 1,
                stranded: 1,
                unidentified: 1,
                forgotten: 2,
            }
        );
    }

    #[test]
    fn a_sweep_that_found_nothing_counts_nothing() {
        assert_eq!(tallied(&[], &[]), Swept::default());
    }

    /// A state that cannot exist must not reach disk, because the same check
    /// runs when a file is read: writing it would turn something still
    /// repairable into an instance that will not start.
    #[test]
    fn an_inconsistent_state_is_refused_before_it_is_written() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (state, _) = with_a_running_job();
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        store.update().agents.clear();

        // The write refused, so what is on disk is still the instance that was
        // valid — and it still opens.
        let reopened = Store::load(snapshot_path(&directory), key()).expect("it still opens");
        assert!(reopened.is_some());
    }
}
