#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! This crate serves the dashboard and runs the orchestrator in the same
//! process. It operates the instance and never talks to a job: conversation
//! belongs to a channel, so no conversational state lives here — see
//! `docs/decisions/0005-conversation-happens-on-channels.md`.
//!
//! There is no binary target yet. It arrives with the server, and adding one
//! now would mean a `main` that starts nothing.

use std::fs;
use std::io::{self, Write as _};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use parking_lot::{Mutex, MutexGuard};
use rand::rngs::{StdRng, SysRng};
use rand::{Rng as _, SeedableRng as _};
use stageman_agent::{ContainerRuntime, StopReason};
use stageman_core::{
    JobId, Key, NONCE_LEN, Nonce, OpenError, Progress, SealError, Snapshot, State,
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
/// `docs/conventions.md` §3 says orchestrator work never happens on the
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
    let believed_running: Vec<JobId> = store.read().running().collect();
    let mut resumed: Vec<JobId> = Vec::new();
    let mut failed: Vec<JobId> = Vec::new();
    let mut stranded: Vec<JobId> = Vec::new();

    for job in believed_running {
        if !left.iter().any(|abandoned| abandoned.job == Some(job)) {
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
            stranded.push(job);
            continue;
        }

        match stageman_job::resume(runtime, job, stageman_orchestrator::resumption_notice()).await {
            Ok(answer) if answer.stop_reason == StopReason::EndTurn => {
                record(store, job, Progress::Completed);
                resumed.push(job);
            }
            Ok(answer) => {
                let why = format!("the agent stopped: {:?}", answer.stop_reason);
                tracing::warn!(%job, stop_reason = ?answer.stop_reason, "resumed and did not finish");
                record(store, job, Progress::Failed(why));
                failed.push(job);
            }
            Err(error) => {
                tracing::warn!(%job, %error, "could not be put back to work");
                record(store, job, Progress::Failed(error.to_string()));
                failed.push(job);
            }
        }
    }

    Ok(Swept {
        resumed: resumed.len(),
        failed: failed.len(),
        unidentified: unplaceable
            .iter()
            .filter(|container| matches!(container, Unplaceable::Unidentified(_)))
            .count(),
        forgotten: unplaceable
            .iter()
            .filter(|container| matches!(container, Unplaceable::Forgotten(..)))
            .count(),
        stranded: stranded.len(),
    })
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
fn unplaceable<'a>(left: &'a [stageman_job::Abandoned], state: &State) -> Vec<Unplaceable<'a>> {
    left.iter()
        .filter_map(|abandoned| match abandoned.job {
            None => Some(Unplaceable::Unidentified(&abandoned.container)),
            Some(job) if state.job(job).is_none() => {
                Some(Unplaceable::Forgotten(&abandoned.container, job))
            }
            Some(_) => None,
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{LoadError, Store, Swept, Unplaceable, reconcile, unplaceable};
    use stageman_agent::ContainerRuntime;
    use stageman_core::{
        Agent, AgentConfig, Job, JobId, Key, Progress, Project, ProjectId, Secret, State,
        Timestamp, Uuid,
    };
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn key() -> Key {
        Key::new([3; 32])
    }

    fn configured() -> State {
        State::new(
            Agent::Claude,
            AgentConfig {
                auth_token: Secret::new("agent-token".to_owned()),
            },
            PathBuf::from("/usr/local/bin/container-runtime"),
        )
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
                    credentials: BTreeMap::new(),
                    jobs: BTreeMap::new(),
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
                progress: Progress::Running,
            },
        );
        state.projects.insert(
            project,
            Project {
                name: "example".to_owned(),
                repository: "https://example.invalid/repo".to_owned(),
                credentials: BTreeMap::new(),
                jobs,
            },
        );
        (state, job)
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
}
