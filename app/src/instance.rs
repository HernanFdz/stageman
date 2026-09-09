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
    Errand, Handout, Inconsistent, InstanceId, Job, JobId, Key, Kit, NONCE_LEN, Nonce, OpenError,
    Outcome, Progress, ProjectId, SealError, Snapshot, Speaking, State, Taken, Thread, Timestamp,
    Uuid, Waiting,
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
    /// Which instance this is, for as long as this file is the instance.
    ///
    /// Read from the file, or minted when the file has none — which is a first
    /// run, and an upgrade from before instances were told apart. Held here
    /// rather than in the state because nothing in the domain reads it: what
    /// it answers is whether a container belongs to this instance or to
    /// another sharing the same runtime.
    instance: InstanceId,
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
        // Read before opening, because opening consumes the snapshot and this
        // is the one thing on it the state does not carry.
        let named = snapshot.instance;
        let state = snapshot.open(&key).map_err(LoadError::Open)?;
        Self::start(path, key, state, named).map(Some)
    }

    /// Creates an instance from freshly configured state.
    ///
    /// # Errors
    ///
    /// Fails if the instance cannot write, or has no randomness.
    pub fn create(path: PathBuf, key: Key, state: State) -> Result<Self, LoadError> {
        Self::start(path, key, state, None)
    }

    /// The identity this instance answers to, and labels its containers with.
    #[must_use]
    pub const fn instance(&self) -> InstanceId {
        self.instance
    }

    fn start(
        path: PathBuf,
        key: Key,
        state: State,
        named: Option<InstanceId>,
    ) -> Result<Self, LoadError> {
        let mut rng = StdRng::try_from_rng(&mut SysRng).map_err(|_| LoadError::Randomness)?;
        // Minted here rather than in the domain, which takes no effects, and
        // written by the first snapshot below — so a file that had none has
        // one from the moment it is opened rather than from the first change.
        let instance = named.unwrap_or_else(|| {
            let mut bytes = [0_u8; 16];
            rng.fill_bytes(&mut bytes);
            let minted = InstanceId::from_uuid(Uuid::from_bytes(bytes));
            tracing::info!(instance = %minted, "this instance had no identity, so it was given one");
            minted
        });
        let store = Self {
            state: Mutex::new(state),
            instance,
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
        let mut snapshot = state
            .seal(&self.key, &mut nonces)
            .map_err(SaveError::Seal)?;
        // The one field the domain cannot fill in, for the reason the field
        // says: minting needs randomness and the domain takes no effects.
        snapshot.instance = Some(self.instance);
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

/// What a job is told about the tools it may use.
///
/// Minted fresh on every start and every resume, carrying the thread that job
/// speaks in. A job is offered only the tool that speaks — starting jobs is a
/// foreman's, and `crate::tooling::tools` is where that is decided.
/// `None` for a job this instance cannot place, which is honest rather than
/// tidy: a credential naming the wrong project would let one project's job
/// speak on another's channel, and a substituted default is exactly what
/// `.quality/gate-reference.md` forbids. A job with no tools reports having
/// none, which is a visible failure rather than a silent misattribution.
fn tools_for(store: &Store, job: JobId, thread: Option<Thread>) -> Option<stageman_agent::Tools> {
    let project = store.read().project_of(job)?;
    Some(stageman_agent::Tools::new(
        crate::tooling::endpoint(*crate::endpoint::PORT),
        crate::SESSIONS.mint(crate::tooling::Warranted {
            project,
            speaker: crate::tooling::Speaker::Job(job),
            thread,
        }),
    ))
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
    kit: Kit,
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
        // The kit arrives decided — by a foreman naming one of the project's,
        // or by a person picking one on the dashboard — and is never composed
        // here: `docs/decisions/0048-a-job-runs-on-a-kit.md` has a project's
        // kits be the only kits, and both doors check the name against them
        // before reaching this.
        let handout = Handout::for_job(&state, kit, project).map_err(RunError::Handout)?;
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
    // Minted before the instruction rather than after it, because the
    // instruction names where this job can be reached and that address is
    // built from the identifier — see
    // `docs/decisions/0042-a-job-shows-its-work-on-a-subdomain.md`, where a
    // job's own identifier being the hostname is what lets nothing about a
    // tunnel be stored.
    let job = JobId::from_uuid(uuid::Uuid::new_v4());
    // Asked of the handout for the reason the comment above gives: the prompt
    // and the environment the container is started with are decided from one
    // value, so a job cannot be told about a variable it was not given, nor
    // given one it was never told about.
    let variables: Vec<_> = handout.variable_names().cloned().collect();
    let kickoff = stageman_foreman::kickoff(
        &repository,
        work,
        voice,
        &crate::tunnel::showing(job),
        &variables,
    );
    let announcement = stageman_foreman::announcement(&repository, reason, job);

    {
        let mut state = store.update();
        if let Some(project) = state.projects.get_mut(&project) {
            // The kit is read off the handout rather than built a second time,
            // for the reason the variables above are: the record and the
            // container are decided from one value, so a job cannot be
            // recorded as running on one kit and started on another.
            project.jobs.insert(
                job,
                Job::new(
                    handout.kit().clone(),
                    reason.to_owned(),
                    kickoff.clone(),
                    Timestamp::now(),
                ),
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
            Err(Progress::Idle(Waiting::Failed(format!(
                "its channel could not be reached: {why}"
            ))))
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

    let thread = store
        .read()
        .job(job)
        .and_then(|recorded| recorded.thread.clone());
    let tools = tools_for(store, job, thread);
    let progress = worked(
        store,
        runtime,
        job,
        "the job did not finish",
        stageman_job::start(
            runtime,
            &handout,
            job,
            store.instance(),
            tools.as_ref(),
            &kickoff,
        ),
    )
    .await;
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
    match recorded.progress {
        Progress::Working => Accepted::Busy,
        // Over, so there is nothing to resume: the container went with the
        // retirement and the session with it. Refused here rather than left to
        // fail at the runtime, which is the same outcome with nobody told.
        Progress::Retired(_) => Accepted::Over,
        Progress::Idle(_) => {
            recorded.progress = Progress::Working;
            Accepted::Taken
        }
    }
}

/// What [`accepting_reply`] decided when a reply arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    /// The job was idle and is now running.
    Taken,
    /// It was already working, so nothing was taken.
    Busy,
    /// It is over, so nothing can be given to it ever again.
    ///
    /// Apart from [`Accepted::Unknown`] because a person is told something
    /// different: this thread did belong to a job, so saying something here
    /// was not a mistake about where to say it.
    Over,
    /// This instance has no such job.
    Unknown,
}

/// Takes a reply for a job if it can take one, and says which.
///
/// **Synchronous, and called on the task that reads the channel**, which is
/// what fixes the order two replies arriving together are considered in. The
/// work each authorises happens elsewhere and may overlap; deciding which of
/// them the job accepts may not, or the loser would be whichever task the
/// runtime happened to poll first rather than whichever message arrived
/// second. See `docs/decisions/0044-a-listener-only-listens.md`.
pub fn accepting_reply(store: &Store, job: JobId) -> Accepted {
    let mut state = store.update();
    let taken = accepting(&mut state, job);
    drop(state);
    taken
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
/// That decision is [`accepting_reply`]'s and arrives here as an argument,
/// because it belongs to the moment the message was read and this function
/// runs long after it — see
/// `docs/decisions/0044-a-listener-only-listens.md`. What is *refused* is
/// still said from here, so a refusal and an acceptance leave by the same
/// door.
///
/// Answering a job this instance does not have, or one with nothing to resume,
/// is deliberately not this function's problem to hide: both come back as a
/// refusal the caller says on the thread.
pub async fn deliver(
    store: &Store,
    runtime: &ContainerRuntime,
    job: JobId,
    said: &str,
    taken: Accepted,
) -> Progress {
    match taken {
        Accepted::Unknown => return Progress::Idle(Waiting::Failed("no such job".to_owned())),
        Accepted::Busy => {
            notice(store, job, stageman_foreman::busy_notice()).await;
            return Progress::Working;
        }
        Accepted::Over => {
            notice(store, job, stageman_foreman::over_notice()).await;
            // Unchanged, which is the whole of what happened: a retirement is
            // a verdict somebody recorded, and a reply arriving afterwards
            // must not overwrite it with anything. Absent only if the record
            // went between one lock and the next, which is the same situation
            // the branch above answers and gets the same answer.
            return store.read().job(job).map_or_else(
                || Progress::Idle(Waiting::Failed("no such job".to_owned())),
                |recorded| recorded.progress.clone(),
            );
        }
        Accepted::Taken => {}
    }

    // A job's thread never changes, but it still has to be written in before
    // each start: an environment is fixed at creation, so nothing the container
    // was given at birth can be counted on to still be there in the shape a
    // long-lived one needs. Read from the record rather than remembered.
    let Some((speaking, kit)) = recorded(store, job) else {
        return Progress::Idle(Waiting::Failed("no such job".to_owned()));
    };

    let tools = tools_for(store, job, speaking.clone());
    let progress = worked(
        store,
        runtime,
        job,
        "the reply did not reach the job",
        stageman_job::resume(runtime, job, &kit, tools.as_ref(), said),
    )
    .await;
    notice(store, job, stageman_foreman::attention_notice()).await;
    progress
}

/// Creates a job on a project and runs it to completion.
///
/// The whole of the doing, in the one crate allowed to name both the store and
/// the job — `docs/architecture.md` §1. What the foreman decides (which
/// project, which kit, why, and what work) arrives here as arguments.
///
/// Both halves, for a caller that has nothing else to do until the job is
/// over. A caller that does — a request, most obviously — uses [`begin`] and
/// [`supervise`] separately.
///
/// # Errors
///
/// Fails if the project is unknown, or if a handout cannot be decided for it.
/// A job that *runs* and fails is not an error here: it is a recorded outcome,
/// returned as a failed reading of [`Progress::Idle`], because the job happened.
pub async fn run(
    store: &Store,
    runtime: &ContainerRuntime,
    project: ProjectId,
    kit: Kit,
    reason: &str,
    work: &str,
) -> Result<(JobId, Progress), RunError> {
    let started = begin(store, project, kit, reason, work)?;

    Ok(supervise(store, runtime, started).await)
}

/// One turn in progress: what it can be told, and what it has said.
///
/// Both halves are per turn rather than per job, and that is the whole reason
/// this is a fresh value each time. A stop signal left over from a turn that
/// has ended would stop the next one, and a claim left over would be recorded
/// against work the agent never described.
struct Running {
    /// Woken when a person asks this turn to stop.
    stopping: tokio::sync::Notify,
    /// What the agent said about why it is stopping, if it has said.
    claimed: parking_lot::Mutex<Option<Waiting>>,
}

/// The turns running right now, by job.
///
/// **One entry per turn in progress, and none otherwise.** Registered before a
/// turn starts and removed however it ends, so the presence of an entry is
/// exactly the question "is there a turn to talk to".
///
/// In memory and never written down. A turn does not survive this process, so
/// neither should anything about one: a stop recorded now and acted on after a
/// restart would stop a turn nobody asked about, and a claim outliving its
/// turn is a job describing itself from the past.
static RUNNING: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<JobId, std::sync::Arc<Running>>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

/// Registers a turn, and hands back what it waits on and speaks through.
fn began(job: JobId) -> std::sync::Arc<Running> {
    let running = std::sync::Arc::new(Running {
        stopping: tokio::sync::Notify::new(),
        claimed: parking_lot::Mutex::new(None),
    });
    RUNNING.lock().insert(job, std::sync::Arc::clone(&running));
    running
}

/// Forgets a turn however it ended, and answers with what it claimed.
///
/// The claim is *consumed* rather than read: taking the entry out is what
/// makes it impossible for one turn's account of itself to be recorded against
/// the next.
fn ended(job: JobId) -> Option<Waiting> {
    let running = RUNNING.lock().remove(&job)?;
    // Cloned out from behind the lock rather than returned through it, so the
    // guard is released before the caller does anything with the answer.
    let claimed = running.claimed.lock();
    claimed.clone()
}

/// Asks the turn running in a job to stop, and says whether one was there.
///
/// **Asking rather than killing**, and the difference is what the turn does
/// with it: the future running the agent is dropped, which closes the pipe the
/// agent is speaking on and ends it, and the container carries on because the
/// agent is no longer what it runs. So a stopped job is one that can be given
/// something again — see
/// `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`.
///
/// `false` for a job with no turn in it, which covers both an idle job and one
/// whose turn ended between a person deciding and this being called. The
/// caller reports the first and can ignore the second, since the job is
/// already where stopping would have left it.
#[must_use]
pub fn stop(job: JobId) -> bool {
    // `notify_one` rather than `notify_waiters`, because it stores a permit: a
    // stop arriving in the instant between registration and the turn actually
    // waiting would otherwise be dropped on the floor.
    RUNNING.lock().get(&job).is_some_and(|running| {
        running.stopping.notify_one();
        true
    })
}

/// Records what a job says about why it is about to stop.
///
/// **Recording a claim is not a state change**, and that is the whole shape of
/// it. A job stays working until its agent actually stops, because the reply
/// gate leans on that to keep two replies from resuming one container; what
/// this writes is consulted when the turn ends and never before. See
/// `docs/decisions/0055-a-job-says-why-it-stopped.md`.
///
/// `false` when no turn is running, which is what a claim arriving after its
/// own turn was stopped looks like. Refused rather than kept, because there is
/// nothing left for it to describe.
///
/// Takes the whole vocabulary and only two of it are reachable: the tool that
/// calls this can spell *asked* and *proposed*, and nothing else. A failure is
/// observed rather than claimed, and a pause is a person's doing.
#[must_use]
pub fn claimed(job: JobId, waiting: Waiting) -> bool {
    RUNNING.lock().get(&job).is_some_and(|running| {
        *running.claimed.lock() = Some(waiting);
        true
    })
}

/// Runs one turn of a job, and writes down what became of it.
///
/// Named for the job rather than for the turn, because a project's foreman
/// takes turns too and `turn` beside this is that one.
///
/// **The one place a turn happens**, which is what gives stopping a single
/// owner: three callers used to run one and record it in nearly the same
/// words, and a fourth that forgot to register the turn would have produced a
/// job nobody could stop, silently.
///
/// What is deliberately not here is the notice on a job's thread. A sweep
/// resuming a job sends none, and folding it in would have this decide
/// something that belongs to the caller.
///
/// **A turn stopped during its own setup can leave a half-made workspace.** A
/// job's first turn creates a container and checks a repository out into it
/// before the agent speaks, and dropping the future partway leaves whatever
/// had been done. Accepted rather than prevented: the window is seconds out of
/// minutes, the container is named after the job so nothing is untracked, and
/// an agent given the workspace afterwards reports what it actually finds.
///
/// Skipped by mutation testing, like everything here that drives a container:
/// every path through it runs an agent, and what it decides that can be
/// checked cheaply is [`outcome`], which has its own tests.
#[mutants::skip]
async fn worked(
    store: &Store,
    runtime: &ContainerRuntime,
    job: JobId,
    complaint: &str,
    running: impl std::future::Future<Output = Result<Answer, stageman_job::JobError>>,
) -> Progress {
    let turn = began(job);
    // Split from the decision below so that the turn is forgotten on every
    // path, including the one where a person stopped it.
    let answered = tokio::select! {
        answered = running => Some(answered),
        () = turn.stopping.notified() => None,
    };
    let claimed = ended(job);

    let progress = match answered {
        None => Progress::Idle(Waiting::Paused),
        Some(Ok(answer)) => {
            noted(store, job, answer.reported.clone());
            outcome(&answer, claimed)
        }
        Some(Err(error)) => Progress::Idle(Waiting::Failed(because(&error))),
    };

    match progress {
        Progress::Idle(Waiting::Failed(ref why)) => tracing::warn!(%job, %why, "{complaint}"),
        Progress::Idle(Waiting::Paused) => tracing::info!(%job, "a person stopped it"),
        _ => {}
    }
    record(store, job, progress.clone());
    settled(runtime, job).await;
    progress
}

/// Releases everything a project is holding, before it is forgotten.
///
/// Its jobs' containers and its foreman's, and then the images nothing needs.
/// A project whose record is removed takes every name that could reach those
/// containers with it, so this has to happen first — otherwise they become
/// exactly the untracked leak `docs/conventions.md` §4 forbids, warned about
/// on every start and removable only by hand.
///
/// **Total, and nothing is reported.** A container that will not go is logged
/// and left, because there is no useful answer for a caller: refusing to
/// forget the project over it would leave an operator with a project they
/// cannot remove and a container they cannot name.
///
/// Says nothing about jobs still working — that is the caller's to refuse
/// before calling this, since it is the one condition an operator can act on.
///
/// Skipped by mutation testing: it removes containers through the runtime and
/// decides nothing a test could check without one.
#[mutants::skip]
pub async fn release(store: &Store, runtime: &ContainerRuntime, project: ProjectId) {
    let jobs: Vec<JobId> = {
        let state = store.read();
        let jobs = state
            .projects
            .get(&project)
            .into_iter()
            .flat_map(|watched| watched.jobs.keys().copied())
            .collect();
        drop(state);
        jobs
    };

    for job in jobs {
        if let Err(why) = stageman_job::discard(runtime, job).await {
            tracing::warn!(%job, %why, "a forgotten project's job kept its container");
        }
        crate::tunnel::TUNNELS.forget(job);
    }

    // And the one container that is the project's own rather than any job's.
    let foreman = stageman_foreman::container(project);
    if let Err(why) = stageman_agent::discard(runtime, &foreman).await {
        tracing::warn!(%project, %why, "a forgotten project kept its foreman's container");
    }

    // The count is the sweep's to report; here the logging inside is the whole
    // of what anybody reads.
    reclaimed(runtime).await;
}

/// Why a job could not be retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// This instance has no such job.
    Unknown,
    /// There is a turn in it, so it has to be stopped first.
    ///
    /// Refused rather than stopped on the operator's behalf, because the two
    /// are different decisions: stopping keeps the work and retiring destroys
    /// it, and doing both from one press would make the destructive one a
    /// side effect of the reversible one.
    Working,
}

/// Ends a job, and reclaims everything it was holding.
///
/// The other half of `docs/open-questions.md`'s retirement question, and the
/// only thing besides the sweep that writes a retirement — see
/// `docs/decisions/0053-a-job-is-stopped-or-retired-by-a-person.md`.
///
/// **The record is written before the container is removed**, and the order is
/// the one `begin` argues for from the other end. Killed in between, this
/// leaves a retired job with a container, which the next sweep finishes.
/// The other order leaves a job that looks answerable with nothing to answer
/// in, which is the case that needs a person.
///
/// **A verdict already recorded is never overwritten.** Retiring a job that is
/// over does nothing to the record and still makes sure its container is gone,
/// which is what makes this safe to press twice and safe to retry after a
/// failure.
///
/// # Errors
///
/// Fails if this instance has no such job, or if a turn is running in it.
pub async fn retire(
    store: &Store,
    runtime: &ContainerRuntime,
    job: JobId,
    ending: Outcome,
) -> Result<(), Refused> {
    {
        let mut state = store.update();
        let Some(recorded) = state.job_mut(job) else {
            drop(state);
            return Err(Refused::Unknown);
        };
        match recorded.progress {
            Progress::Working => {
                drop(state);
                return Err(Refused::Working);
            }
            Progress::Retired(_) => {}
            Progress::Idle(_) => recorded.progress = Progress::Retired(ending),
        }
        drop(state);
    }

    // Not fatal, and deliberately not retried here. What is left is a
    // container belonging to a job that is over, which is exactly what the
    // startup sweep looks for — so the failure resolves itself, and saying so
    // is more useful than failing a retirement that has already happened.
    if let Err(why) = stageman_job::discard(runtime, job).await {
        tracing::warn!(%job, %why, "its container could not be removed; a sweep will try again");
    }
    // The port it was published on can be handed to another container, so a
    // remembered one would send a browser to somebody else's page.
    crate::tunnel::TUNNELS.forget(job);
    // The count is the sweep's to report; here the logging inside is the whole
    // of what anybody reads.
    reclaimed(runtime).await;
    Ok(())
}

/// What a sweep found, and what it did about it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Swept {
    /// Jobs put back to work.
    pub resumed: usize,
    /// Jobs whose resumption failed, and which are recorded as failed.
    pub failed: usize,
    /// Containers of this instance's whose names it does not understand, removed.
    ///
    /// Almost always a container from an older version of this project: odd,
    /// and benign. Removed rather than reported, because the instance label
    /// says it is this instance's and nothing here accounts for it.
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
    ///
    /// Removed, and the warning naming it is what an operator has to go on
    /// afterwards. That is a change of policy rather than of tally — see
    /// `docs/decisions/0054-a-container-says-which-instance-started-it.md` for
    /// why the label is what makes removing this safe, and what it costs.
    pub forgotten: usize,
    /// Containers carrying no instance label, left exactly where they were.
    ///
    /// Made before instances were told apart, so nothing can say whether they
    /// are this instance's or another's. The only category that still
    /// accumulates, and the only one that needs a person: one generation of
    /// containers, removed by hand once.
    pub unclaimed: usize,
    /// Containers belonging to another instance, passed over.
    ///
    /// Not a problem and not this instance's business. Counted so that an
    /// operator debugging a shared runtime can see they were seen, which is
    /// the difference between *ignored* and *not noticed*.
    pub elsewhere: usize,
    /// Jobs whose container was gone, and which are now over.
    ///
    /// Named for the state they were put in rather than for what was noticed
    /// about them, because that is what an operator sees on the dashboard.
    /// Covers an idle job as well as a working one: both owned a container,
    /// and neither can be given anything without it.
    pub lost: usize,
    /// Containers of jobs that are over, removed.
    ///
    /// Ordinarily zero, because retiring a job removes its container there and
    /// then. Anything here is a retirement that was interrupted between
    /// writing the record and removing what it named, which is the ordering
    /// chosen so that this sweep can finish it.
    pub cleared: usize,
    /// Images of this project's that nothing needed any more.
    ///
    /// Counted apart from everything above because it is the only tally here
    /// that is not about work: no job is better or worse off for it, and what
    /// it reclaims is disk. Ordinarily zero — one image serves every container
    /// built from the same recipe, so this moves only after a recipe is
    /// edited, which is a release rather than a Tuesday. See
    /// `docs/decisions/0051-an-image-is-named-by-the-recipe-it-is-built-from.md`.
    pub reclaimed: usize,
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

    let (unplaceable, disowned) = disowned(store, runtime, &left).await;

    // First, because everything below reasons about containers a job still
    // needs, and these belong to jobs that need nothing. A retirement writes
    // the record before removing the container, so one left here is a
    // retirement interrupted rather than a state anything else should see.
    let mut cleared: Vec<JobId> = Vec::new();
    // Decided under one read that ends before any of it is acted on, like the
    // settling below: holding the instance open across a runtime call is what
    // this shape exists to avoid.
    let finished = {
        let state = store.read();
        let finished = over(&left, &state);
        drop(state);
        finished
    };
    for job in finished {
        match stageman_job::discard(runtime, job).await {
            Ok(()) => {
                tracing::info!(%job, "removed the container of a job that is over");
                crate::tunnel::TUNNELS.forget(job);
                cleared.push(job);
            }
            Err(why) => tracing::warn!(%job, %why, "the container of a job that is over would \
                                                    not go; it will be tried again"),
        }
    }

    let unfinished: Vec<JobId> = store.read().unfinished().collect();
    let mut attended: Vec<Attended> = Vec::new();

    // **Two passes rather than one loop with two guards.** Anything with
    // nothing to run in is over, and only then is what remains put back to
    // work — which also means the second pass reads a state the first has
    // finished changing.
    //
    // Asked of every job that is not over rather than only of the ones
    // believed to be working, and that is wider than it used to be on purpose.
    // An idle job whose container has gone is in exactly the same position as
    // a working one — a reply to it would fail, and nothing said so — and the
    // old sweep left it looking answerable indefinitely.
    for job in unfinished.iter().copied() {
        if has_container(&left, job) {
            continue;
        }
        // Nothing left to run in, and nothing to resume: the session lived in
        // that container. Recorded as lost, which is terminal and so is the
        // one outcome that stops it being swept for ever afterwards.
        tracing::warn!(%job, "its container is gone, so the job is lost");
        record(store, job, Progress::Retired(Outcome::Lost));
        attended.push(Attended::Lost);
    }

    // An idle job with a container is in the state it should be in, so it is
    // not here: its container is stopped or left up by `settle` below, and a
    // reply is what puts it back to work rather than a sweep.
    let resuming: Vec<JobId> = {
        let state = store.read();
        let resuming = unfinished
            .iter()
            .copied()
            .filter(|job| working_now(&state, *job))
            .collect();
        drop(state);
        resuming
    };

    for job in resuming {
        // Cannot be absent — the job came from this same state a moment ago —
        // and is refused rather than substituted anyway, for the reason the
        // handout's constructors give: a job resumed on a kit it did not
        // record would be exactly the silent wrong answer the kit exists to
        // rule out.
        let Some((speaking, kit)) = recorded(store, job) else {
            tracing::warn!(%job, "believed to be running, but it has no record");
            attended.push(Attended::Lost);
            continue;
        };
        let tools = tools_for(store, job, speaking.clone());
        let progress = worked(
            store,
            runtime,
            job,
            "could not be put back to work",
            stageman_job::resume(
                runtime,
                job,
                &kit,
                tools.as_ref(),
                stageman_foreman::resumption_notice(),
            ),
        )
        .await;
        attended.push(Attended::from(&progress));
    }

    // Last, so that it sees the containers this sweep has just finished with
    // as well as the ones a previous process left up.
    settle(store, runtime).await;

    // After every container this sweep will touch, and for one reason: an
    // image is kept by the containers using it, so asking before the sweep has
    // finished with them would ask about a state it was about to change.
    Ok(tallied(
        &attended,
        &unplaceable,
        &disowned,
        cleared.len(),
        reclaimed(runtime).await,
    ))
}

/// Stops a job's container unless it is still showing something.
///
/// Called wherever a turn ends and wherever a container is found up, which is
/// the three moments
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`
/// names. The decision is the job crate's; what belongs here is saying so,
/// because a container left running is the one outcome an operator would
/// otherwise have to discover from the runtime.
/// Skipped by mutation testing: it performs an effect through the runtime and
/// chooses a diagnostic, and there is nothing else in it.
#[mutants::skip]
async fn settled(runtime: &ContainerRuntime, job: JobId) {
    if stageman_job::rest(runtime, job).await == stageman_job::Showing::Still {
        tracing::info!(
            %job,
            "its container is left running, because something is still answering on its tunnel"
        );
    }
}

/// Reclaims the images nothing needs, and says how many went.
///
/// Housekeeping rather than work, so a runtime that will not answer is warned
/// about and the sweep carries on: an instance that refused to start over a
/// reclaim it could not make would be refusing over disk.
///
/// Skipped by mutation testing, like everything here that drives the runtime:
/// what it does is call the agent crate and choose a diagnostic.
#[mutants::skip]
async fn reclaimed(runtime: &ContainerRuntime) -> usize {
    match stageman_agent::reclaim(runtime).await {
        Ok(gone) => {
            if gone > 0 {
                tracing::info!(images = gone, "reclaimed images no container needed");
            }
            gone
        }
        Err(why) => {
            tracing::warn!(%why, "could not reclaim the images nothing is using");
            0
        }
    }
}

/// Stops every container left up by a job that is no longer working.
///
/// The sweep half of the rule. A container held open because its tunnel
/// answered does not stop when that server does, so something has to ask
/// again — and this is also what a start does with whatever the last process
/// left behind, since a hard kill leaves containers running and cannot do
/// otherwise.
///
/// Jobs believed to be working are passed over: their container is up because
/// a turn is in it, which is the other half of the same rule.
#[mutants::skip]
pub async fn settle(store: &Store, runtime: &ContainerRuntime) {
    let up = match stageman_job::still_running(runtime).await {
        Ok(up) => up,
        Err(why) => {
            tracing::warn!(%why, "could not ask which containers are running");
            return;
        }
    };

    // Decided under one read rather than one per job, and before any of it is
    // acted on: asking again between two containers would be reading a state
    // that a turn finishing had moved underneath the loop.
    let (placed, unplaced) = {
        let state = store.read();
        let split = resting(&up, &state);
        drop(state);
        split
    };

    for job in placed {
        settled(runtime, job).await;
    }

    // And the ones no work here accounts for, each asked whose it is first.
    // Anything but this instance's is left running: another instance's
    // container is very likely mid-turn, and one that cannot say is not worth
    // guessing about when the cost of guessing wrong is somebody's work.
    for job in unplaced {
        if whose(runtime, store.instance(), &stageman_job::container(job)).await == Whose::Ours {
            settled(runtime, job).await;
        } else {
            tracing::debug!(%job, "a container up for work this instance has no record of; left running");
        }
    }
}

/// Which of the containers that are up should be asked to stop, split by
/// whether this instance can place them.
///
/// Pure, so the rule can be tested without a runtime — and it is the rule
/// rather than the plumbing, which is the half worth pinning.
///
/// **A job believed to be working is passed over**, because its container is
/// up for the other reason in
/// `docs/decisions/0043-a-container-lives-as-long-as-its-tunnel-answers.md`:
/// there is a turn in it. Stopping that container would end an agent
/// mid-turn, which is the one outcome this must never produce.
///
/// **A job the instance has no record of is answered separately**, and that is
/// the correction. It cannot be working *here*, so this used to stop it — and
/// on a shared runtime it is most likely another instance's job, mid-turn,
/// with a record and an agent of its own. Measured: a daemon running this rule
/// stopped a container belonging to a different instance within thirty
/// seconds. The caller asks the container whose it is before touching one of
/// these; see
/// `docs/decisions/0054-a-container-says-which-instance-started-it.md`.
fn resting(up: &[JobId], state: &stageman_core::State) -> (Vec<JobId>, Vec<JobId>) {
    let mut placed = Vec::new();
    let mut unplaced = Vec::new();
    for job in up.iter().copied() {
        match state.job(job) {
            // A turn is running in it, here, now.
            Some(recorded) if matches!(recorded.progress, Progress::Working) => {}
            Some(_) => placed.push(job),
            None => unplaced.push(job),
        }
    }
    (placed, unplaced)
}

/// What a sweep did about one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attended {
    /// Put back to work, and it finished.
    Resumed,
    /// Tried and did not finish, or could not be tried at all.
    Failed,
    /// Its container was gone, so the job is over.
    Lost,
}

impl From<&Progress> for Attended {
    /// Only a turn that ended counts as resumed.
    ///
    /// Written as a conversion rather than as a branch inside the sweep
    /// because mutation testing found the branch untested: inverting it swaps
    /// the resumed and failed tallies, which is a sweep that reports the
    /// opposite of what happened, and nothing noticed.
    ///
    /// Every reading of idle but one counts as resumed, and the exception is
    /// the turn that went wrong. What the agent claimed about *why* it stopped
    /// is not this tally's business: a job that asked a question was resumed
    /// exactly as much as one that proposed an answer.
    ///
    /// A retired job cannot arrive here, because a turn does not end in a
    /// retirement and the sweep only resumes jobs that are not over. It is
    /// counted as failed rather than made unrepresentable, which is the
    /// cheaper wrong answer: a tally is off by one, where a panic would take
    /// the startup sweep down with it.
    fn from(progress: &Progress) -> Self {
        match progress {
            Progress::Idle(Waiting::Failed(_)) | Progress::Working | Progress::Retired(_) => {
                Self::Failed
            }
            Progress::Idle(_) => Self::Resumed,
        }
    }
}

/// Counts what a sweep did.
///
/// Separated from doing it so the arithmetic can be checked without a
/// container runtime. Counting is exactly the kind of code that is obviously
/// right and occasionally is not.
fn tallied(
    attended: &[Attended],
    unplaceable: &[Unplaceable<'_>],
    disowned: &[Whose],
    cleared: usize,
    reclaimed: usize,
) -> Swept {
    let counted = |wanted: Attended| attended.iter().filter(|had| **had == wanted).count();
    // Zipped rather than indexed, because the two are built in one pass and
    // the pairing is what makes a count mean anything: an unplaceable
    // container is only removed if its label said it was ours.
    let paired = || unplaceable.iter().zip(disowned.iter());
    let ours = |wanted: fn(&Unplaceable<'_>) -> bool| {
        paired()
            .filter(|(container, whose)| **whose == Whose::Ours && wanted(container))
            .count()
    };

    Swept {
        cleared,
        reclaimed,
        resumed: counted(Attended::Resumed),
        failed: counted(Attended::Failed),
        lost: counted(Attended::Lost),
        unidentified: ours(|container| matches!(container, Unplaceable::Unidentified(_))),
        forgotten: ours(|container| matches!(container, Unplaceable::Forgotten(..))),
        unclaimed: disowned
            .iter()
            .filter(|whose| **whose == Whose::Unlabelled)
            .count(),
        elsewhere: disowned
            .iter()
            .filter(|whose| **whose == Whose::Elsewhere)
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

impl Unplaceable<'_> {
    /// The container's name, whichever way it could not be placed.
    const fn named(&self) -> &str {
        match self {
            Self::Unidentified(name) | Self::Forgotten(name, _) => name,
        }
    }
}

/// Everything the runtime has that the instance cannot account for.
/// Whether this instance believes a turn is running in this job.
///
/// A named function rather than the pattern it was, for the reason
/// `has_container` is one: the loop it came from drives a runtime and the
/// decision does not, so inline it was reachable only by tests the gate skips.
/// Inverted, a sweep resumes every idle job and leaves every working one
/// where it is.
fn working_now(state: &State, job: JobId) -> bool {
    state
        .job(job)
        .is_some_and(|recorded| matches!(recorded.progress, Progress::Working))
}

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

/// Deals with every container the instance cannot account for.
///
/// A phase of its own rather than lines inside the sweep, because it is the
/// one that *destroys* something and it answers a question none of the others
/// ask: not "what should this job do next" but "is this container mine at
/// all". See
/// `docs/decisions/0054-a-container-says-which-instance-started-it.md`.
///
/// Answers with what it found and what it decided about each, in the same
/// order, so the tally can pair them: a container is only counted as removed
/// if its label said it was this instance's.
///
/// Skipped by mutation testing: what it decides is [`unplaceable`] and
/// [`belonging`], both pure and both tested, and what it does is remove
/// containers through the runtime.
#[mutants::skip]
async fn disowned<'a>(
    store: &Store,
    runtime: &ContainerRuntime,
    left: &'a [stageman_job::Abandoned],
) -> (Vec<Unplaceable<'a>>, Vec<Whose>) {
    let unplaceable = {
        let state = store.read();
        let found = unplaceable(left, &state);
        drop(state);
        found
    };

    let mut decided: Vec<Whose> = Vec::new();
    for container in &unplaceable {
        let whose = whose(runtime, store.instance(), container.named()).await;
        match (whose, container) {
            (Whose::Ours, Unplaceable::Unidentified(name)) => tracing::warn!(
                container = %name,
                "a container of this instance's, under a name it does not understand; removed"
            ),
            (Whose::Ours, Unplaceable::Forgotten(name, job)) => tracing::warn!(
                container = %name,
                %job,
                "a container of this instance's naming a job it has no record of — work may \
                 have been lost; removed"
            ),
            // Somebody else's, and not this instance's business. Reported at
            // all only because an operator debugging a shared runtime wants to
            // know it was seen and passed over rather than not noticed.
            (Whose::Elsewhere, _) => tracing::debug!(
                container = %container.named(),
                "a container belonging to another instance; left alone"
            ),
            (Whose::Unlabelled, _) => tracing::warn!(
                container = %container.named(),
                "a container this project started before instances were told apart, so it \
                 cannot be attributed; left alone rather than removed"
            ),
        }
        if whose == Whose::Ours {
            drop(stageman_agent::discard(runtime, container.named()).await);
        }
        decided.push(whose);
    }

    (unplaceable, decided)
}

/// Who a container belongs to, as far as this instance can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Whose {
    /// This instance started it, so it is this instance's to remove.
    Ours,
    /// Another instance started it. Not ours to touch, and not ours to report.
    Elsewhere,
    /// It says nothing, so nobody can tell.
    ///
    /// A container made before instances were told apart. Left alone for ever,
    /// which is the honest answer: removing it would be guessing, and on a
    /// shared runtime the guess destroys somebody else's work. One generation
    /// of containers, cleaned up by hand once.
    Unlabelled,
}

/// Asks a container which instance started it.
///
/// **Every uncertainty answers `Unlabelled`**, which is the direction that
/// cannot destroy anything: a runtime that will not say, a label that is not
/// an identifier, and a container that has gone since the listing all mean
/// *do not remove this*. The only thing that earns a removal is a label
/// naming this instance and nothing else.
///
/// Skipped by mutation testing, like everything here that drives the runtime:
/// what it decides is a comparison, and that is what the test below pins.
#[mutants::skip]
async fn whose(runtime: &ContainerRuntime, instance: InstanceId, container: &str) -> Whose {
    match stageman_agent::started_by(runtime, container).await {
        Ok(started) => belonging(started, instance),
        Err(why) => {
            tracing::warn!(%container, %why, "could not ask which instance started a container");
            Whose::Unlabelled
        }
    }
}

/// What a container's label means, given whose instance is asking.
///
/// Pure, so the comparison can be tested without a runtime — and it is the
/// whole of the decision that lets a sweep remove anything, so it is the part
/// worth pinning. Inverted, this instance would remove every container except
/// its own.
const fn belonging(started: Option<InstanceId>, instance: InstanceId) -> Whose {
    match started {
        Some(named) if named.as_uuid().as_u128() == instance.as_uuid().as_u128() => Whose::Ours,
        Some(_) => Whose::Elsewhere,
        None => Whose::Unlabelled,
    }
}

/// Every container belonging to a job that is over.
///
/// What a retirement leaves behind when it is interrupted, and what the next
/// sweep finishes. Pure, so the judgement is testable without a runtime: it is
/// a question about records, and only the removal needs anything running.
///
/// A job the instance has no record of is not here — that is a container to
/// report rather than one to remove, and `unplaceable` is where it goes.
fn over(left: &[stageman_job::Abandoned], state: &State) -> Vec<JobId> {
    left.iter()
        .filter_map(|abandoned| abandoned.job)
        .filter(|job| {
            state
                .job(*job)
                .is_some_and(|recorded| recorded.progress.is_retired())
        })
        .collect()
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
fn outcome(answer: &Answer, claimed: Option<Waiting>) -> Progress {
    if answer.stop_reason == StopReason::EndTurn {
        // What the agent said about itself, and the honest residual when it
        // said nothing. Not a substituted default: a turn that ended without a
        // claim genuinely is one nobody described, and inventing *asked* or
        // *proposed* for it would record something no agent ever said.
        Progress::Idle(claimed.unwrap_or(Waiting::Silent))
    } else {
        // A claim is ignored when the turn did not end cleanly, and
        // deliberately: an agent that said it was ready for review and then
        // ran out of tokens did not finish, whatever it had believed a moment
        // earlier.
        Progress::Idle(Waiting::Failed(format!(
            "the agent stopped: {:?}",
            answer.stop_reason
        )))
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

/// Writes down what a job's session reported it was set to, this turn.
///
/// Beside the kit rather than checked against it, because the two are
/// different facts and the adapter has already done the checking — a setting
/// that did not take fails the turn before anything is reported. See
/// `docs/decisions/0048-a-job-runs-on-a-kit.md`.
fn noted(store: &Store, job: JobId, reported: std::collections::BTreeMap<String, String>) {
    let mut state = store.update();
    if let Some(recorded) = state.job_mut(job) {
        recorded.reported = reported;
    }
}

/// What a job resuming needs from its record: where it speaks, and what it
/// runs on.
///
/// One read rather than two, so that the thread and the kit come from the
/// same moment of the instance. `None` for a job this instance has no record
/// of, which every caller treats as a refusal rather than a default.
fn recorded(store: &Store, job: JobId) -> Option<(Option<Thread>, Kit)> {
    let state = store.read();
    let found = state
        .job(job)
        .map(|recorded| (recorded.thread.clone(), recorded.kit().clone()));
    drop(state);
    found
}

/// A message that is already in a foreman's inbox, and what that arrival was.
///
/// Carried from the reading task to the working one because all three answers
/// belong to the moment the message arrived rather than to the moment it is
/// acted on: whether this arrival is the one that has to drive the loop, how
/// many were ahead of it when it landed, and which thread the answer belongs
/// under.
pub struct Arrived {
    /// What became of it, and `None` when there is no such project to take it.
    taken: Option<Taken>,
    /// How many were waiting ahead of it as it landed.
    ahead: usize,
    /// Where its answer belongs.
    thread: Thread,
}

/// Puts a message in a project's foreman's inbox.
///
/// **Synchronous, and called on the task that reads the channel.** The inbox
/// is ordered and that order is the whole of what it promises, so the taking
/// has to happen where messages are still in the order they arrived — see
/// `docs/decisions/0044-a-listener-only-listens.md`. Everything the message
/// then causes is [`attend`]'s, and runs elsewhere.
#[must_use]
pub fn arriving(store: &Store, project: ProjectId, said: Errand) -> Arrived {
    // Kept before the message is moved into the inbox, because the answer to
    // it belongs under the message that asked.
    let thread = said.thread.clone();
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
    let (taken, ahead) = match outcome {
        Some((taken, ahead)) => (Some(taken), ahead),
        None => (None, 0),
    };
    Arrived {
        taken,
        ahead,
        thread,
    }
}

/// Works a project's foreman until its inbox is empty.
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
/// only one of them finds it idle. Which of them did is [`arriving`]'s answer
/// and arrives here as an argument, so that it is settled by arrival order
/// rather than by polling order.
#[mutants::skip]
pub async fn attend(
    store: &Store,
    runtime: &ContainerRuntime,
    project: ProjectId,
    arrived: Arrived,
) {
    let Arrived {
        taken,
        ahead,
        thread,
    } = arrived;

    // Said at once, before any work. A foreman three messages behind is silent
    // for a while, and somebody who hears nothing cannot tell queued from
    // ignored.
    if taken.is_some() {
        notice_in(
            store,
            project,
            &thread,
            &stageman_foreman::received_notice(ahead),
        )
        .await;
    }

    if !drives(taken) {
        return;
    }

    working(store, runtime, project, stageman_foreman::Starting::Fresh).await;
}

/// Puts a foreman that was interrupted mid-turn back to work.
///
/// The foreman's half of
/// `docs/decisions/0015-a-job-survives-the-daemon-dying.md`, and it arrived
/// late: a project's inbox is in the snapshot, so a message in hand when this
/// process stopped is still in hand — and nothing was driving it, because only
/// an arrival that finds a foreman idle does that and this one is not idle.
/// See `docs/decisions/0045-a-foremans-turn-survives-the-daemon-dying.md`.
///
/// **No message is taken and none is acknowledged**, which is the whole
/// difference from [`attend`]: the arrival already happened, and was already
/// answered on the thread when it did. What the thread is told instead is that
/// the wait had a reason.
#[mutants::skip]
pub async fn resumed(store: &Store, runtime: &ContainerRuntime, project: ProjectId) {
    let thread = {
        let state = store.read();
        let found = waiting_on(&state, project).map(|errand| errand.thread);
        drop(state);
        found
    };
    let Some(thread) = thread else {
        return;
    };
    notice_in(store, project, &thread, stageman_foreman::resumed_notice()).await;

    working(
        store,
        runtime,
        project,
        stageman_foreman::Starting::Interrupted,
    )
    .await;
}

/// Every project whose foreman was working when this process last stopped.
///
/// Over a state rather than a store, like every other decision in this crate
/// that could otherwise only be reached through I/O. A foreman that is idle is
/// not here, and a foreman that is idle *while something waits* is a state the
/// domain has no way to write down — see `Attending`.
#[must_use]
pub fn interrupted(state: &State) -> Vec<ProjectId> {
    state
        .projects
        .iter()
        .filter(|(_, watched)| watched.attending.on().is_some())
        .map(|(project, _)| *project)
        .collect()
}

/// Runs a foreman's turns until its inbox is empty.
///
/// The half [`attend`] and [`resumed`] share. `starting` describes the *first*
/// turn only: whatever this picks up afterwards was never begun, so it is
/// fresh however this loop was entered.
#[mutants::skip]
async fn working(
    store: &Store,
    runtime: &ContainerRuntime,
    project: ProjectId,
    starting: stageman_foreman::Starting,
) {
    let mut starting = starting;
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
        let outcome = turn(store, runtime, project, &errand, starting).await;
        starting = stageman_foreman::Starting::Fresh;
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
    starting: stageman_foreman::Starting,
) -> Result<(), RunError> {
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
    // kits are edited from the dashboard and a session outlives those edits.
    // Each with the description its operator wrote, which is what the choice
    // is made on — `docs/decisions/0048-a-job-runs-on-a-kit.md`.
    let kits: Vec<(String, String)> = {
        let state = store.read();
        let offered = crate::tooling::allowed_kits(&state, project);
        drop(state);
        offered
    };
    let kits: Vec<(&str, &str)> = kits
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect();

    // Minted per turn, not per project, and this is the turn's thread going
    // with it: a foreman answers in whichever thread it was spoken to in, so
    // the credential is what carries that rather than a file written into the
    // container — see `docs/decisions/0034-tools-are-served-not-shipped.md`.
    let credential = crate::SESSIONS.mint(crate::tooling::Warranted {
        project,
        speaker: crate::tooling::Speaker::Foreman,
        thread: Some(errand.thread.clone()),
    });

    stageman_foreman::attend(
        runtime,
        &handout,
        stageman_foreman::Watching {
            project,
            repository: &repository,
            kits: &kits,
        },
        store.instance(),
        &stageman_agent::Tools::new(crate::tooling::endpoint(*crate::endpoint::PORT), credential),
        stageman_foreman::Turn {
            said: &errand.said,
            starting,
        },
    )
    .await
    .map(drop)
    .map_err(|why| RunError::Foreman(why.to_string()))
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
        Attended, LoadError, Store, Swept, Unplaceable, Whose, has_container, interrupted, outcome,
        reconcile, resting, run, tallied, unplaceable,
    };
    use stageman_agent::{Answer, ContainerRuntime, StopReason};
    use stageman_core::{
        Agent, AgentConfig, InstanceId, Job, JobId, Key, Kit, Outcome, Progress, Project,
        ProjectId, Secret, State, Taken, Thread, Timestamp, Uuid, Waiting,
    };
    use std::collections::BTreeMap;
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
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: only_claude(),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
                jobs: BTreeMap::from([(
                    job,
                    Job::new(
                        Kit::defaults(Agent::Claude),
                        "started by hand".to_owned(),
                        "do the thing".to_owned(),
                        Timestamp::UNIX_EPOCH,
                    ),
                )]),
            },
        );
        (state, job)
    }

    /// The kits a project offers when nobody has written one: its agent's
    /// defaults under the agent's name, which is what every project written
    /// before kits opens with.
    fn only_claude() -> BTreeMap<stageman_core::KitName, stageman_core::KitConfig> {
        BTreeMap::from([(
            stageman_core::KitName::new("Claude").expect("a name"),
            stageman_core::KitConfig::defaults(Agent::Claude),
        )])
    }

    fn a_message() -> stageman_core::Errand {
        stageman_core::Errand {
            said: "look at the parser".to_owned(),
            thread: Thread {
                channel: stageman_core::Channel::Slack,
                id: "1700000000.000100".to_owned(),
            },
        }
    }

    /// A foreman holding a message is found; an idle one is not.
    ///
    /// Both halves, because the two failures are opposite and both are silent.
    /// Missing a foreman that was working leaves it wedged for ever — no
    /// arrival drives a loop that is already running, and after a restart none
    /// is. Naming an idle one starts a turn on a message that does not exist.
    #[test]
    fn a_foreman_holding_a_message_is_the_one_put_back_to_work() {
        let (mut state, _) = an_instance_with_a_job();
        let project = *state
            .projects
            .keys()
            .next()
            .expect("the helper watches one project");

        assert_eq!(
            interrupted(&state),
            Vec::new(),
            "an idle foreman has nothing to pick up"
        );

        let watched = state
            .projects
            .get_mut(&project)
            .expect("the project just read");
        assert_eq!(watched.attending.take(a_message()), Taken::Started);

        assert_eq!(interrupted(&state), vec![project]);
    }

    /// What waits behind the message in hand does not make a second project.
    ///
    /// One entry per project, because what is resumed is a foreman rather than
    /// a message: the loop it is put back into drains everything behind the
    /// one it is holding.
    #[test]
    fn a_foreman_with_several_waiting_is_named_once() {
        let (mut state, _) = an_instance_with_a_job();
        let project = *state
            .projects
            .keys()
            .next()
            .expect("the helper watches one project");
        let watched = state
            .projects
            .get_mut(&project)
            .expect("the project just read");
        assert_eq!(watched.attending.take(a_message()), Taken::Started);
        assert_eq!(watched.attending.take(a_message()), Taken::Waiting);
        assert_eq!(watched.attending.take(a_message()), Taken::Waiting);

        assert_eq!(interrupted(&state), vec![project]);
    }

    /// A container holding a turn is never asked to stop; every other one is.
    ///
    /// Both halves, because the two failures are opposite and neither is
    /// visible from the other side. A sweep asking about a working job ends its
    /// agent mid-turn; one asking about nothing lets every container that was
    /// ever held open run until the process dies.
    #[test]
    fn a_sweep_asks_about_everything_up_except_a_job_still_working() {
        let (mut state, working) = an_instance_with_a_job();
        let idle = JobId::from_uuid(Uuid::from_u128(13));
        let unknown = JobId::from_uuid(Uuid::from_u128(99));
        let project = ProjectId::from_uuid(Uuid::from_u128(11));

        let mut resting_job = state.job(working).expect("the running job").clone();
        resting_job.progress = Progress::Idle(Waiting::Silent);
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .jobs
            .insert(idle, resting_job);

        let (placed, unplaced) = resting(&[working, idle, unknown], &state);
        assert_eq!(
            placed,
            vec![idle],
            "a job mid-turn is passed over and an idle one is not",
        );
        assert_eq!(
            unplaced,
            vec![unknown],
            "a job this instance never heard of is answered separately, because it \
             is most likely another instance's and mid-turn",
        );

        let (placed, unplaced) = resting(&[working], &state);
        assert!(
            placed.is_empty() && unplaced.is_empty(),
            "nothing is asked about when the only container up holds a turn",
        );
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
                    foreman_kit: Kit::defaults(Agent::Claude),
                    kits: only_claude(),
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    variables: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
                },
            );
        }
        let reopened = Store::load(path, key())
            .expect("it reads back")
            .expect("and there is something there");
        assert!(reopened.read().projects.contains_key(&id));
    }

    /// A job this instance cannot place is given no tools at all.
    ///
    /// The refusal that stands where `unwrap_or_default` nearly went. A
    /// credential naming the wrong project would let one project's job speak
    /// on another's channel — a silent misattribution rather than a visible
    /// failure — so the absence has to travel rather than be filled in.
    /// Mutation testing found nothing checking it, which is exactly the shape
    /// `.quality/gate-reference.md` warns a substituted default takes.
    #[test]
    fn a_job_this_instance_cannot_place_is_given_no_tools() {
        use stageman_core::{Progress, Timestamp};

        let directory = TempDir::new().expect("a temporary directory");
        let mut state = configured();
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        let job = JobId::from_uuid(Uuid::from_u128(7));
        state.projects.insert(
            project,
            stageman_core::Project {
                name: "aviary".to_owned(),
                repository: "https://example.invalid/aviary".to_owned(),
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: only_claude(),
                credentials: std::collections::BTreeMap::new(),
                channels: std::collections::BTreeMap::new(),
                jobs: std::collections::BTreeMap::new(),
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
            },
        );
        state
            .projects
            .get_mut(&project)
            .expect("the project")
            .jobs
            .insert(job, {
                let mut idle = stageman_core::Job::new(
                    Kit::defaults(Agent::Claude),
                    "because".to_owned(),
                    "do the thing".to_owned(),
                    Timestamp::now(),
                );
                idle.progress = Progress::Idle(Waiting::Silent);
                idle
            });
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        assert!(
            super::tools_for(&store, job, None).is_some(),
            "a job this instance holds must be given the tools it speaks with",
        );
        assert!(
            super::tools_for(&store, JobId::from_uuid(Uuid::from_u128(99)), None).is_none(),
            "a job this instance cannot place must be given none, not somebody else's",
        );
    }

    /// What a session reported is written onto the job and read back beside
    /// the kit it runs on, and a job this instance does not hold reads as
    /// nothing.
    ///
    /// Mutation testing is what asked for this: the write could be deleted
    /// and the read could answer nothing, and no test noticed either.
    #[test]
    fn what_a_session_reported_is_noted_and_read_back_with_the_kit() {
        let directory = TempDir::new().expect("a temporary directory");
        let mut state = configured();
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        let job = JobId::from_uuid(Uuid::from_u128(7));
        state.projects.insert(
            project,
            stageman_core::Project {
                name: "aviary".to_owned(),
                repository: "https://example.invalid/aviary".to_owned(),
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: only_claude(),
                credentials: std::collections::BTreeMap::new(),
                channels: std::collections::BTreeMap::new(),
                jobs: std::collections::BTreeMap::from([(
                    job,
                    stageman_core::Job::new(
                        stageman_core::Kit::Claude {
                            model: stageman_core::ClaudeModel::Haiku,
                        },
                        "because".to_owned(),
                        "do the thing".to_owned(),
                        Timestamp::now(),
                    ),
                )]),
                variables: BTreeMap::new(),
                attending: stageman_core::Attending::default(),
            },
        );
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        let reported = BTreeMap::from([("model".to_owned(), "haiku".to_owned())]);
        super::noted(&store, job, reported.clone());
        assert_eq!(
            store.read().job(job).expect("the job").reported,
            reported,
            "what the session said is on the record"
        );

        let (thread, kit) = super::recorded(&store, job).expect("a job this instance holds");
        assert!(thread.is_none(), "no channel is bound, so no thread");
        assert_eq!(
            kit,
            stageman_core::Kit::Claude {
                model: stageman_core::ClaudeModel::Haiku
            },
            "the kit read back is the one the job was created on"
        );
        assert!(
            super::recorded(&store, JobId::from_uuid(Uuid::from_u128(404))).is_none(),
            "a job this instance has no record of reads as nothing, not as a default"
        );
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
            Job::new(
                Kit::defaults(Agent::Claude),
                "an issue was opened".to_owned(),
                "work on it".to_owned(),
                Timestamp::UNIX_EPOCH,
            ),
        );
        state.projects.insert(
            project,
            Project {
                name: "example".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                foreman_kit: Kit::defaults(Agent::Claude),
                kits: only_claude(),
                credentials: BTreeMap::new(),
                channels: BTreeMap::new(),
                jobs,
                variables: BTreeMap::new(),
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
        state.job_mut(job).expect("the job").progress = Progress::Idle(Waiting::Silent);
        assert_eq!(super::accepting(&mut state, job), super::Accepted::Taken);
        assert_eq!(state.job(job).expect("the job").progress, Progress::Working);

        // And taking it once is what stops a second taking it as well, which
        // is the collision this exists to prevent.
        assert_eq!(super::accepting(&mut state, job), super::Accepted::Busy);
    }

    /// A reply is taken by any reading of idle, and by no other state.
    ///
    /// Every reading means the same thing to this decision: the agent stopped
    /// and can be given something. Treating one of them differently — refusing
    /// a job that failed, most plausibly — would make the state a person is
    /// most likely to reply to the one state a reply cannot reach.
    #[test]
    fn a_reply_is_taken_by_every_reading_of_idle() {
        for waiting in [
            Waiting::Asked,
            Waiting::Proposed,
            Waiting::Paused,
            Waiting::Silent,
            Waiting::Failed("the credential had expired".to_owned()),
        ] {
            let (mut state, job) = an_instance_with_a_job();
            state.job_mut(job).expect("the job").progress = Progress::Idle(waiting.clone());

            assert_eq!(
                super::accepting(&mut state, job),
                super::Accepted::Taken,
                "{waiting:?} must be able to take a reply",
            );
            assert_eq!(state.job(job).expect("the job").progress, Progress::Working);
        }
    }

    /// A job that is over takes nothing, and keeps the verdict it was given.
    ///
    /// The state that must not move, and the one where moving it is worst: a
    /// retired job has no container, so taking the reply would put it back to
    /// *working* and the next sweep would find it lost all over again — with a
    /// person's verdict overwritten on the way.
    #[test]
    fn a_reply_to_a_job_that_is_over_is_refused_and_changes_nothing() {
        for outcome in [Outcome::Done, Outcome::Discarded, Outcome::Lost] {
            let (mut state, job) = an_instance_with_a_job();
            state.job_mut(job).expect("the job").progress = Progress::Retired(outcome);

            assert_eq!(
                super::accepting(&mut state, job),
                super::Accepted::Over,
                "{outcome:?} must refuse a reply",
            );
            assert_eq!(
                state.job(job).expect("the job").progress,
                Progress::Retired(outcome),
                "a refused reply must not move a job that is over",
            );
        }
    }

    /// A working job is refused, and keeps its container until somebody stops
    /// it.
    ///
    /// The refusal that makes stopping and retiring two decisions rather than
    /// one press. Retiring destroys the container and the session in it, so
    /// doing it to a job mid-turn would end work nobody said to end.
    #[tokio::test]
    async fn a_working_job_cannot_be_retired() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (state, job) = with_a_running_job();
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        assert_eq!(
            super::retire(&store, &empty_runtime(), job, Outcome::Done).await,
            Err(super::Refused::Working),
        );
        assert_eq!(
            store
                .read()
                .job(job)
                .map(|recorded| recorded.progress.clone()),
            Some(Progress::Working),
            "a refused retirement must not move the job",
        );
    }

    /// An idle job takes the verdict it was given, whichever it is.
    #[tokio::test]
    async fn an_idle_job_is_retired_with_the_ending_it_was_given() {
        for ending in [Outcome::Done, Outcome::Discarded] {
            let directory = tempfile::tempdir().expect("a temporary directory");
            let (mut state, job) = with_a_running_job();
            state.job_mut(job).expect("the job").progress = Progress::Idle(Waiting::Proposed);
            let store =
                Store::create(snapshot_path(&directory), key(), state).expect("it can write");

            super::retire(&store, &empty_runtime(), job, ending)
                .await
                .expect("an idle job can be retired");

            assert_eq!(
                store
                    .read()
                    .job(job)
                    .map(|recorded| recorded.progress.clone()),
                Some(Progress::Retired(ending)),
            );
        }
    }

    /// Retiring a job that is over changes nothing, and is not an error.
    ///
    /// What makes the control safe to press twice and safe to retry after the
    /// container removal failed. Overwriting would let a second press turn a
    /// job somebody marked done into a discarded one, or bury the fact that a
    /// job was lost rather than ended by anybody.
    #[tokio::test]
    async fn retiring_a_job_that_is_over_keeps_the_verdict_it_already_has() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (mut state, job) = with_a_running_job();
        state.job_mut(job).expect("the job").progress = Progress::Retired(Outcome::Lost);
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        super::retire(&store, &empty_runtime(), job, Outcome::Done)
            .await
            .expect("it is already over, which is what was asked for");

        assert_eq!(
            store
                .read()
                .job(job)
                .map(|recorded| recorded.progress.clone()),
            Some(Progress::Retired(Outcome::Lost)),
        );
    }

    /// A job this instance does not have cannot be retired.
    #[tokio::test]
    async fn retiring_a_job_this_instance_does_not_have_is_refused() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (state, _) = with_a_running_job();
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        assert_eq!(
            super::retire(
                &store,
                &empty_runtime(),
                JobId::from_uuid(Uuid::from_u128(99)),
                Outcome::Done,
            )
            .await,
            Err(super::Refused::Unknown),
        );
    }

    /// Asking a job with no turn in it to stop says so rather than pretending.
    ///
    /// The answer a route needs in order to tell an operator apart from a
    /// race: nothing was running, so nothing was stopped.
    #[test]
    fn stopping_a_job_with_no_turn_in_it_finds_nothing() {
        assert!(!super::stop(JobId::from_uuid(Uuid::from_u128(1234))));
    }

    /// Only containers of jobs that are over are cleared, and every one of
    /// them is.
    ///
    /// The classifier the sweep destroys things with, so both halves are worth
    /// pinning. Missing one leaves a container nothing will ever reclaim;
    /// including a job that is merely idle destroys work somebody can still
    /// reply to.
    #[test]
    fn only_the_containers_of_jobs_that_are_over_are_cleared() {
        // The fixture's own job is the one still working; the other two are
        // added beside it on the same project.
        let (mut state, busy) = with_a_running_job();
        let project = state.project_of(busy).expect("the fixture's project");
        let ended = JobId::from_uuid(Uuid::from_u128(3));
        let idle = JobId::from_uuid(Uuid::from_u128(4));

        for (id, progress) in [
            (ended, Progress::Retired(Outcome::Done)),
            (idle, Progress::Idle(Waiting::Asked)),
        ] {
            let mut job = Job::new(
                Kit::defaults(Agent::Claude),
                "a reason".to_owned(),
                "some work".to_owned(),
                Timestamp::UNIX_EPOCH,
            );
            job.progress = progress;
            state
                .projects
                .get_mut(&project)
                .expect("the project")
                .jobs
                .insert(id, job);
        }

        let containers = [
            left(&stageman_job::container(ended), Some(ended)),
            left(&stageman_job::container(idle), Some(idle)),
            left(&stageman_job::container(busy), Some(busy)),
            // A container naming a job this instance has lost is reported
            // rather than removed, so it must not be here.
            left(
                "stageman-job-00000000-0000-0000-0000-000000000077",
                Some(JobId::from_uuid(Uuid::from_u128(0x77))),
            ),
            left("stageman-foreman-something", None),
        ];

        assert_eq!(super::over(&containers, &state), vec![ended]);
    }

    /// Asking a job with a turn in it to stop finds one.
    ///
    /// The other half of the answer below, and the one that matters: without
    /// it, a `stop` that always reported nothing would look exactly like a
    /// dashboard button that does nothing.
    #[test]
    fn stopping_a_job_with_a_turn_in_it_finds_it() {
        let job = JobId::from_uuid(Uuid::from_u128(5678));
        let turn = super::began(job);

        assert!(super::stop(job), "a registered turn is there to be stopped");
        drop(super::ended(job));
        drop(turn);

        assert!(!super::stop(job), "and once it has ended, it is not");
    }

    /// A container that cannot be placed is named the same way whichever way
    /// it could not be.
    ///
    /// The name is what gets removed, so a reader that answered with anything
    /// else would remove something other than what was reported — or, empty,
    /// address whatever the runtime makes of a blank name.
    #[test]
    fn an_unplaceable_container_answers_with_its_own_name() {
        let job = JobId::from_uuid(Uuid::from_u128(7));

        assert_eq!(
            Unplaceable::Unidentified("older-scheme").named(),
            "older-scheme"
        );
        assert_eq!(Unplaceable::Forgotten("a-name", job).named(), "a-name");
    }

    /// Only a job this instance believes is working counts as working.
    ///
    /// The guard that decides whether the sweep resumes a job or passes over
    /// it. Inverted, every idle job is put back to work and every working one
    /// is left alone — which is a sweep that resumes conversations nobody
    /// added to and abandons the ones that were interrupted.
    #[test]
    fn a_turn_is_running_only_in_a_job_recorded_as_working() {
        let (mut state, job) = an_instance_with_a_job();
        assert!(super::working_now(&state, job));

        state.job_mut(job).expect("the job").progress = Progress::Idle(Waiting::Asked);
        assert!(!super::working_now(&state, job));

        state.job_mut(job).expect("the job").progress = Progress::Retired(Outcome::Lost);
        assert!(!super::working_now(&state, job));

        assert!(
            !super::working_now(&state, JobId::from_uuid(Uuid::from_u128(99))),
            "a job this instance does not have is not working",
        );
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

    /// A job with nothing to run in is over. It cannot be resumed, because the
    /// session lived in the container that is gone — which is also what stops
    /// it being swept for on every start from now on.
    #[tokio::test]
    async fn a_job_whose_container_is_gone_is_recorded_as_lost() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (state, job) = with_a_running_job();
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        let swept = reconcile(&store, &empty_runtime())
            .await
            .expect("the runtime answers");

        assert_eq!(swept.lost, 1, "{swept:?}");
        assert_eq!(swept.resumed, 0);
        assert_eq!(
            store.read().job(job).map(|j| j.progress.clone()),
            Some(Progress::Retired(Outcome::Lost)),
        );
    }

    /// And an *idle* job whose container is gone is lost in the same way.
    ///
    /// The half the sweep used not to look at. Such a job looks answerable on
    /// every screen, and a reply to it fails at the runtime with nothing
    /// anywhere having said the work was already unreachable.
    #[tokio::test]
    async fn an_idle_job_whose_container_is_gone_is_lost_too() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (mut state, job) = with_a_running_job();
        state.job_mut(job).expect("the job").progress = Progress::Idle(Waiting::Proposed);
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        let swept = reconcile(&store, &empty_runtime())
            .await
            .expect("the runtime answers");

        assert_eq!(swept.lost, 1, "{swept:?}");
        assert_eq!(
            store.read().job(job).map(|j| j.progress.clone()),
            Some(Progress::Retired(Outcome::Lost)),
        );
    }

    /// A job that is already over is left entirely alone.
    ///
    /// It owns no container, so there is nothing to reconcile and nothing to
    /// report — and a sweep that counted it would report the same casualty on
    /// every start for as long as the record exists.
    #[tokio::test]
    async fn a_retired_job_is_not_swept_for() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (mut state, job) = with_a_running_job();
        state.job_mut(job).expect("the job").progress = Progress::Retired(Outcome::Done);
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        let swept = reconcile(&store, &empty_runtime())
            .await
            .expect("the runtime answers");

        assert_eq!(swept, Swept::default(), "{swept:?}");
        assert_eq!(
            store.read().job(job).map(|j| j.progress.clone()),
            Some(Progress::Retired(Outcome::Done)),
            "a verdict a person recorded must not be overwritten by a sweep",
        );
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

        assert_eq!(first.lost, 1);
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
        assert_eq!(
            reopened.read().job(job).map(|j| j.progress.clone()),
            Some(Progress::Retired(Outcome::Lost)),
        );
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
    /// Retiring a job really does take its container and its image with it.
    ///
    /// Everything above this proves what gets *recorded*. This is the half a
    /// unit test cannot reach, and it is the half that reclaims anything: the
    /// container is removed and the image behind it goes with it, because
    /// nothing else is using it. Needs no credential — the container is never
    /// asked to speak.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn retiring_a_job_takes_its_container_and_its_image() {
        let runtime = stageman_agent::first_present(stageman_agent::candidates())
            .expect("a container runtime is installed");
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (mut state, job) = with_a_running_job();
        state.job_mut(job).expect("the job").progress = Progress::Idle(Waiting::Proposed);
        let store = Store::create(snapshot_path(&directory), key(), state).expect("it can write");

        // A container of this job's, made the cheap way: the image a job would
        // run, holding itself open, with the label a sweep finds it by.
        let name = stageman_job::container(job);
        drop(stageman_job::discard(&runtime, job).await);
        let image = stageman_agent::build(&runtime, Agent::Claude, stageman_core::Role::Job)
            .await
            .expect("the image builds");
        let created = std::process::Command::new(runtime.path())
            .args(["create", "--name", &name, image.as_argument()])
            .output()
            .expect("the runtime runs");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );

        super::retire(&store, &runtime, job, Outcome::Done)
            .await
            .expect("an idle job can be retired");

        assert_eq!(
            store
                .read()
                .job(job)
                .map(|recorded| recorded.progress.clone()),
            Some(Progress::Retired(Outcome::Done)),
        );
        let left = stageman_job::left_behind(&runtime)
            .await
            .expect("the runtime answers");
        assert!(
            !left.iter().any(|abandoned| abandoned.job == Some(job)),
            "the container of a retired job is still here: {left:?}",
        );
    }

    /// A sweep removes its own abandoned container and leaves another
    /// instance's exactly where it is.
    ///
    /// **The test this whole label exists for.** Both containers are named for
    /// jobs this instance has no record of, so nothing but the label tells
    /// them apart — and getting it wrong means a development instance served
    /// out of a checkout destroys the real instance's work, which is the
    /// ordinary arrangement on a developer's machine rather than an exotic
    /// one. Needs a runtime and no credential: neither container is asked to
    /// speak.
    #[tokio::test]
    #[ignore = "needs a container runtime and the network; run `just image-handshake`"]
    async fn a_sweep_removes_its_own_abandoned_container_and_not_another_instances() {
        let runtime = stageman_agent::first_present(stageman_agent::candidates())
            .expect("a container runtime is installed");
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store =
            Store::create(snapshot_path(&directory), key(), configured()).expect("it can write");

        let image = stageman_agent::build(&runtime, Agent::Claude, stageman_core::Role::Foreman)
            .await
            .expect("the image builds");

        // Two containers, both naming jobs this instance has never heard of,
        // differing only in the instance that started them. The labels are
        // spelled out rather than imported because this is a test about what
        // reaches the runtime, and a constant shared with the code under test
        // would agree with itself.
        let ours = "stageman-job-00000000-0000-0000-0000-0000000000a1";
        let theirs = "stageman-job-00000000-0000-0000-0000-0000000000a2";
        let elsewhere = InstanceId::from_uuid(Uuid::from_u128(0xbeef));
        for (name, instance) in [(ours, store.instance()), (theirs, elsewhere)] {
            drop(stageman_agent::discard(&runtime, name).await);
            let created = std::process::Command::new(runtime.path())
                .args([
                    "create",
                    "--name",
                    name,
                    "--label",
                    &format!("stageman.job={name}"),
                    "--label",
                    &format!("stageman.instance={instance}"),
                    image.as_argument(),
                ])
                .output()
                .expect("the runtime runs");
            assert!(
                created.status.success(),
                "{}",
                String::from_utf8_lossy(&created.stderr)
            );
        }

        let swept = reconcile(&store, &runtime)
            .await
            .expect("the runtime answers");

        let left = stageman_job::left_behind(&runtime)
            .await
            .expect("the runtime answers");
        let names: Vec<&str> = left
            .iter()
            .map(|abandoned| abandoned.container.as_str())
            .collect();

        // Tidied before anything is asserted, so a failure leaves nothing on
        // the daemon for the next run to trip over.
        drop(stageman_agent::discard(&runtime, theirs).await);

        assert!(
            !names.contains(&ours),
            "this instance's own abandoned container survived: {names:?}",
        );
        assert!(
            names.contains(&theirs),
            "another instance's container was destroyed: {names:?}",
        );
        // Counted rather than counted exactly. Every container test shares one
        // daemon, so anything else running at the same moment is unplaceable
        // from here too — and what this test is about is which of *these two*
        // went, which the assertions above settle.
        assert!(swept.forgotten >= 1, "{swept:?}");
        assert!(swept.elsewhere >= 1, "{swept:?}");
    }

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
                    foreman_kit: Kit::defaults(Agent::Claude),
                    kits: only_claude(),
                    // No platform credential: the repository is public, so this
                    // also checks that a job with nothing to authenticate with
                    // is a perfectly ordinary job rather than a broken one.
                    credentials: BTreeMap::new(),
                    channels: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    variables: BTreeMap::new(),
                    attending: stageman_core::Attending::default(),
                },
            );
            (state, project)
        }

        /// One job, end to end: a project configured, an agent started in a
        /// container named for its job, a kickoff it did not write, and a
        /// repository checked out for it before it spoke.
        ///
        /// The checkout is the part worth proving, and what it proves changed
        /// with `docs/decisions/0050-the-repository-is-checked-out-before-the-first-turn.md`:
        /// it used to be evidence that the agent cloned, since nothing else
        /// could have; it is now evidence that the checkout was there for the
        /// agent, at the workspace root where the adapter looks, and that an
        /// agent started inside it could answer about it.
        #[tokio::test]
        #[ignore = "needs a container runtime, a built image, a credential and the network; run `just image-session`"]
        async fn a_job_runs_from_kickoff_to_a_cloned_repository() {
            let runtime = located_runtime();
            let directory = tempfile::tempdir().expect("a temporary directory");
            let (state, project) = an_instance_watching(PUBLIC_REPOSITORY);
            let store = std::sync::Arc::new(
                Store::create(snapshot_path(&directory), key(), state).expect("it can write"),
            );

            // **The tools this instance serves, actually served.** A job is
            // told where to find them at session creation, so without this the
            // agent is handed an address nothing answers on — which is not the
            // harmless absence it sounds like: an agent told to use a tool it
            // cannot reach was measured dying outright about half the time.
            // Every interface and the port a container is told about, which is
            // what the daemon itself binds. `just image-session` names a port
            // of its own, because the operator's daemon is very likely holding
            // the default one while this runs.
            let listening = tokio::net::TcpListener::bind(("0.0.0.0", *crate::endpoint::PORT))
                .await
                .expect("the tools port is free — `just image-session` sets one");
            let serving = tokio::spawn(crate::endpoint::serve(
                listening,
                std::sync::Arc::clone(&store),
                std::sync::Arc::clone(&crate::SESSIONS),
            ));

            let (job, progress) = run(
                &store,
                &runtime,
                project,
                Kit::defaults(Agent::Claude),
                "checking that a job can run at all",
                "Reply with one short line saying how many files are in the repository checked \
                 out in the current directory. Make no changes, commit nothing, and open nothing.",
            )
            .await
            .expect("the job is created");

            // **Two properties from one turn, and the second is new.** That
            // the job finished, and that its agent said why it was stopping —
            // which is the only evidence anywhere that the kickoff actually
            // gets the tool called, since nothing forces it. Asserted as *not
            // silent* rather than as a particular reading, because which of
            // the two the agent picks is its judgement about this instruction
            // and pinning it would make the test an opinion about the model.
            assert!(
                matches!(progress, Progress::Idle(Waiting::Asked | Waiting::Proposed)),
                "the job did not finish having said why it stopped: {progress:?}",
            );
            assert_eq!(
                store
                    .read()
                    .job(job)
                    .map(|recorded| recorded.progress.clone()),
                Some(progress.clone()),
                "the instance should have recorded what it answered with"
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

            // The repository really is in the container, at the root of the
            // workspace — where the adapter looks for a project's settings.
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
            assert!(
                workspace.join(".git").exists(),
                "no checkout at the workspace root: {workspace:?}"
            );

            serving.abort();
            stageman_job::discard(&runtime, job)
                .await
                .expect("it is removable");
        }
    }

    fn answered(stop_reason: StopReason) -> Answer {
        Answer {
            text: "whatever it said".to_owned(),
            stop_reason,
            reported: BTreeMap::new(),
        }
    }

    /// The line mutation testing found untested. Finishing the turn is the
    /// only thing that counts as having finished.
    ///
    /// And a turn that finished without the agent describing itself is
    /// *silent* rather than anything more flattering: the claim is absent,
    /// and absent is a reading of its own.
    #[test]
    fn only_a_finished_turn_counts_as_a_completed_job() {
        assert_eq!(
            outcome(&answered(StopReason::EndTurn), None),
            Progress::Idle(Waiting::Silent)
        );
    }

    /// A finished turn is recorded as whatever its agent said about it.
    ///
    /// The whole point of the tool: without this the two readings a person
    /// acts on differently collapse into the one that says nothing.
    #[test]
    fn a_finished_turn_is_recorded_as_what_its_agent_claimed() {
        for claimed in [Waiting::Asked, Waiting::Proposed] {
            assert_eq!(
                outcome(&answered(StopReason::EndTurn), Some(claimed.clone())),
                Progress::Idle(claimed),
            );
        }
    }

    /// A claim is ignored when the turn did not end cleanly.
    ///
    /// An agent that said it was ready for review and then ran out of tokens
    /// did not finish, whatever it believed a moment earlier — and recording
    /// its claim would put a job somebody should look at into the list of jobs
    /// somebody should read.
    #[test]
    fn a_claim_does_not_survive_a_turn_that_went_wrong() {
        assert!(
            matches!(
                outcome(&answered(StopReason::MaxTokens), Some(Waiting::Proposed)),
                Progress::Idle(Waiting::Failed(_)),
            ),
            "a claim must not turn a failed turn into a finished one",
        );
    }

    /// A claim reaches the turn it was made during, and nothing reaches a job
    /// with no turn in it.
    ///
    /// The property the per-turn registry exists for. A claim arriving after
    /// its own turn was stopped has nothing left to describe, and keeping it
    /// would let it be recorded against the next turn instead.
    #[test]
    fn a_claim_belongs_to_the_turn_it_was_made_during() {
        let job = JobId::from_uuid(Uuid::from_u128(4321));

        assert!(
            !super::claimed(job, Waiting::Proposed),
            "a job with no turn in it must take no claim",
        );

        let turn = super::began(job);
        assert!(super::claimed(job, Waiting::Proposed));
        assert_eq!(super::ended(job), Some(Waiting::Proposed));
        drop(turn);

        assert!(
            !super::claimed(job, Waiting::Asked),
            "the turn is over, so there is nothing left to claim against",
        );
        assert_eq!(super::ended(job), None, "and nothing left to consume");
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
                matches!(
                    outcome(&answered(ending), None),
                    Progress::Idle(Waiting::Failed(_))
                ),
                "{ending:?} should not read as success"
            );
        }
    }

    /// The stop reason survives into the message, because "it did not finish"
    /// and "it refused" send an operator to different places.
    #[test]
    fn a_failure_says_how_the_turn_ended() {
        let Progress::Idle(Waiting::Failed(why)) = outcome(&answered(StopReason::Refusal), None)
        else {
            panic!("a refusal is not a completed job");
        };

        assert!(why.contains("Refusal"), "{why}");
    }

    /// The conversion mutation testing found untested. Inverting it makes a
    /// sweep report the opposite of what happened.
    #[test]
    fn only_a_completed_job_counts_as_resumed() {
        assert_eq!(
            Attended::from(&Progress::Idle(Waiting::Silent)),
            Attended::Resumed
        );
        assert_eq!(
            Attended::from(&Progress::Idle(Waiting::Failed("anything".to_owned()))),
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
            Attended::Lost,
        ];
        let unplaceable = [
            Unplaceable::Unidentified("older-scheme"),
            Unplaceable::Forgotten("a-name", JobId::from_uuid(Uuid::from_u128(9))),
            Unplaceable::Forgotten("another", JobId::from_uuid(Uuid::from_u128(10))),
        ];

        // One of each kind is this instance's and so removed; the forgotten
        // pair is split, so that a container of somebody else's and one that
        // cannot say are both proved not to be counted as removals.
        let disowned = [Whose::Ours, Whose::Ours, Whose::Elsewhere];

        assert_eq!(
            tallied(&attended, &unplaceable, &disowned, 3, 4),
            Swept {
                resumed: 2,
                failed: 1,
                lost: 1,
                unidentified: 1,
                forgotten: 1,
                unclaimed: 0,
                elsewhere: 1,
                cleared: 3,
                reclaimed: 4,
            }
        );
    }

    /// A container is removed only when its label names this instance.
    ///
    /// The comparison the whole sweep rests on. Inverted, an instance would
    /// remove every container except its own, which on a shared runtime is the
    /// worst outcome available — and the two instances would take turns doing
    /// it to each other.
    #[test]
    fn only_a_container_this_instance_labelled_is_ours_to_remove() {
        let ours = InstanceId::from_uuid(Uuid::from_u128(1));
        let theirs = InstanceId::from_uuid(Uuid::from_u128(2));

        assert_eq!(super::belonging(Some(ours), ours), Whose::Ours);
        assert_eq!(super::belonging(Some(theirs), ours), Whose::Elsewhere);
        assert_eq!(
            super::belonging(None, ours),
            Whose::Unlabelled,
            "a container that says nothing must never be taken for ours",
        );
    }

    #[test]
    fn a_sweep_that_found_nothing_counts_nothing() {
        assert_eq!(tallied(&[], &[], &[], 0, 0), Swept::default());
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
