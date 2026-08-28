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
use stageman_core::{Key, NONCE_LEN, Nonce, OpenError, SealError, Snapshot, State};

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
            // in memory is still correct. Which logging this should go through
            // is not yet decided.
            eprintln!("stageman: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoadError, Store};
    use stageman_core::{Agent, AgentConfig, Key, Project, ProjectId, Secret, State, Uuid};
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
}
