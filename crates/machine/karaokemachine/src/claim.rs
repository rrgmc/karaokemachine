//! One machine per data directory.
//!
//! **The conflict is the directory and not the port**, which is the whole reason this exists rather
//! than a check on the listening socket. `The machine holds its port, rather than claiming it once`
//! makes a failed bind a state the machine is built to sit in and recover from — a television that
//! was switched off, a network that arrived late — so a bind failure cannot mean *something else is
//! running* without contradicting a decision that exists precisely because a machine that gave up is
//! one nobody notices.
//!
//! What genuinely cannot be shared is the data directory: one settings file written whole, one
//! packages folder, one catalog. Two machines over one of those is two machines each convinced it
//! is the only one.
//!
//! # One *per directory*, which is better than one at a time
//!
//! Two machines with two `--data-dir`s on two ports are two machines, and they work — which is what
//! the development port table already assumes. What is refused is two programs sharing one
//! machine's state.
//!
//! # Why a lock rather than a file somebody has to clean up
//!
//! The lock is held by the open file for the life of the process, and the operating system releases
//! it on exit **or on a crash**. So there is no stale claim to reason about and nothing to sweep:
//! a machine that was killed leaves a file behind and no claim on it.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::settings::Paths;

/// What the claim is written on, inside the data directory.
///
/// **Named rather than hidden**, because somebody who opens the folder should be able to tell what
/// it is. It holds nothing: what matters is the lock the operating system puts on it, and its
/// contents would only be something else to keep in step.
const CLAIM_FILE: &str = "machine.lock";

/// A held claim on one data directory.
///
/// **Kept for as long as the machine runs.** Dropping it closes the file, which releases the lock —
/// so this is bound for the whole of a run rather than tested and thrown away, and a `let _ =` at
/// the call site would take the claim and give it back in the same statement.
#[derive(Debug)]
pub(crate) struct Claim {
    /// Held open for its lock alone. The file's contents are never read or written.
    _file: File,
}

/// Why a directory could not be claimed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ClaimError {
    /// Another machine is using it.
    #[error(
        "KaraokeMachine is already running on {}. \
         Stop it, or start this one with a --data-dir of its own",
        dir.display()
    )]
    Taken {
        /// Which directory.
        dir: PathBuf,
    },
    /// The claim file could not be opened at all.
    ///
    /// **Distinct from being taken**, because the answers differ: a directory somebody else holds
    /// wants a different `--data-dir`, and one that cannot be written wants a look at its
    /// permissions.
    #[error("claiming {}: {source}", dir.display())]
    Unusable {
        /// Which directory.
        dir: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
}

impl Claim {
    /// Claims `paths`' data directory for this process, or says who has it.
    pub(crate) fn take(paths: &Paths) -> Result<Self, ClaimError> {
        // The directory has to exist before anything in it can be opened, and this is the earliest
        // thing that touches it. Idempotent, and the same call every other writer here makes.
        paths.create().map_err(|source| ClaimError::Unusable {
            dir: paths.data_dir.clone(),
            source,
        })?;
        Self::take_in(&paths.data_dir)
    }

    /// The same, on a plain directory, so a test needs no `Paths`.
    fn take_in(dir: &Path) -> Result<Self, ClaimError> {
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(CLAIM_FILE))
            .map_err(|source| ClaimError::Unusable {
                dir: dir.to_path_buf(),
                source,
            })?;
        // **`try_lock` and never `lock`.** Waiting would make a second machine hang with nothing
        // said rather than report what is wrong, and the answer to a directory in use is a
        // different directory rather than a queue for this one.
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(ClaimError::Taken {
                dir: dir.to_path_buf(),
            }),
            Err(std::fs::TryLockError::Error(source)) => Err(ClaimError::Unusable {
                dir: dir.to_path_buf(),
                source,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory that lasts as long as the test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("km-claim-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// The second claim on one directory is refused, and says which directory.
    #[test]
    fn a_second_claim_on_one_directory_is_refused() {
        let dir = scratch("second");
        let first = Claim::take_in(&dir).expect("the first claim");
        match Claim::take_in(&dir) {
            Err(ClaimError::Taken { dir: named }) => assert_eq!(named, dir),
            other => panic!("a second claim should be refused, got {other:?}"),
        }
        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ...and letting the first go hands the directory to the next machine.
    ///
    /// **This is what makes a restart work**, and it is the property a lock file somebody has to
    /// delete would not have: the operating system releases the claim with the file, so a machine
    /// that stopped — however it stopped — leaves nothing behind for the next one to clear.
    #[test]
    fn releasing_a_claim_frees_the_directory() {
        let dir = scratch("release");
        let first = Claim::take_in(&dir).expect("the first claim");
        drop(first);
        let second = Claim::take_in(&dir).expect("the directory should be free again");
        drop(second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two directories are two machines, which is the arrangement the port table assumes.
    #[test]
    fn two_directories_are_two_machines() {
        let one = scratch("one");
        let two = scratch("two");
        let first = Claim::take_in(&one).expect("the first");
        let second = Claim::take_in(&two).expect("the second, elsewhere");
        drop((first, second));
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }
}
