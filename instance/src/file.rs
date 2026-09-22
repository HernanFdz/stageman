//! The instance's file: opening what was written, and sealing what will be.
//!
//! Both halves live here rather than in the world, because both are
//! deterministic given the key and the nonces, and the nonces are the
//! instance's own. What the world does is read one file before the instance
//! exists and write one file when asked.

use rand::Rng as _;
use rand::rngs::StdRng;
use stageman_core::{
    Inconsistent, InstanceId, Key, NONCE_LEN, Nonce, OpenError, Progress, SealError, Snapshot,
    State,
};

/// The file could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// It is not valid JSON.
    #[error("the file is not valid JSON")]
    Parse(#[source] serde_json::Error),
    /// It could not be decrypted, or does not pass its checks.
    #[error("the file could not be opened")]
    Open(#[source] OpenError),
}

/// The state a file holds, and the identity it names if it names one.
///
/// No file is a first run rather than a failure, per
/// `docs/decisions/0013-an-instance-is-configured-before-it-exists.md`.
///
/// # Errors
///
/// Fails if the bytes are not JSON, cannot be decrypted with `key`, or
/// describe an instance that cannot exist.
pub fn opened(bytes: Option<&[u8]>, key: &Key) -> Result<(State, Option<InstanceId>), LoadError> {
    let Some(bytes) = bytes else {
        return Ok((State::default(), None));
    };
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(LoadError::Parse)?;
    // Read before opening, because opening consumes the snapshot and this is
    // the one thing on it the state does not carry.
    let named = snapshot.instance;
    let mut state = snapshot.open(key).map_err(LoadError::Open)?;
    // Nothing running means nothing waiting: a job not working with messages
    // in its inbox is a shape this version never writes, and carrying it
    // would mean inventing a turn to deliver them. Dropped with a line, as
    // `docs/conventions.md` §4 permits for a message in hand.
    for (job, recorded) in state
        .projects
        .values_mut()
        .flat_map(|project| project.jobs.iter_mut())
    {
        if recorded.progress != Progress::Working && !recorded.inbox.is_empty() {
            let dropped = recorded.inbox.drain().len();
            tracing::warn!(
                %job,
                dropped,
                "a job that is not working held messages, which are dropped"
            );
        }
    }
    Ok((state, named))
}

/// The state could not be sealed.
///
/// Logged and never returned to a caller: a step has nowhere to report it,
/// and the state in memory is still right. What is refused is the write.
#[derive(Debug, thiserror::Error)]
pub enum SealFailure {
    /// The state describes an instance that cannot exist, and the same check
    /// runs when a file is read — so writing it would turn a mistake still in
    /// memory into a file that will not open.
    #[error("the state describes an instance that is not internally consistent")]
    Inconsistent(#[source] Inconsistent),
    /// A credential could not be sealed.
    #[error("credentials could not be sealed")]
    Seal(#[source] SealError),
    /// The snapshot could not be encoded.
    #[error("the snapshot could not be encoded")]
    Encode(#[source] serde_json::Error),
}

/// The bytes that go on disk for this state.
///
/// A fresh nonce per credential per write, from the instance's own
/// generator: reusing one under the same key leaks the authentication key,
/// so there is no such thing as a cheap reuse.
pub fn sealed(
    state: &State,
    id: InstanceId,
    key: &Key,
    rng: &mut StdRng,
) -> Result<Vec<u8>, SealFailure> {
    state.check().map_err(SealFailure::Inconsistent)?;
    let mut nonces = || {
        let mut nonce: Nonce = [0; NONCE_LEN];
        rng.fill_bytes(&mut nonce);
        nonce
    };
    let mut snapshot = state.seal(key, &mut nonces).map_err(SealFailure::Seal)?;
    snapshot.instance = Some(id);
    serde_json::to_vec_pretty(&snapshot).map_err(SealFailure::Encode)
}
