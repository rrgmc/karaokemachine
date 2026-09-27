//! What the tests of more than one crate need, written once.
//!
//! **A dev-dependency, and never anything else.** Nothing here ships: no release binary links it, and
//! `ALL_APPS` in `tools/dist/bin.sh` does not name it.
//!
//! Two things live here. [`Scratch`] is a folder that removes itself, for every crate whose tests
//! touch a disk. The `http` feature adds [`http`], which drives an axum router as a service, with no
//! socket, for every crate that serves pages or an API.
//!
//! A helper belongs here when a second crate wants it. One crate's own doubles stay in that crate's
//! `testing` module, where they can reach what is private to it.

#[cfg(feature = "http")]
pub mod http;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How long [`Scratch`] waits for whoever still holds a file in it.
///
/// Windows refuses to delete a file another handle has open. A connection closed on another thread
/// can still hold its SQLite file when the test's last value drops. Two seconds is long enough for
/// that thread, and short enough that a file held forever costs a test a second rather than a hang.
const PATIENCE: Duration = Duration::from_secs(2);

/// The prefix every scratch folder is named with, and what [`sweep_once`] recognizes its own by.
const PREFIX: &str = "km-test-";

/// How old a folder has to be before the sweep takes it.
///
/// Longer than any run of any suite here, so a checkout building beside this one keeps its own.
const STALE: Duration = Duration::from_secs(60 * 60);

/// A scratch folder that removes itself when it drops.
///
/// The name carries the test's own name, the process id and a count within the process. Two suites
/// in two checkouts therefore cannot collide, and neither can two scratches with one name in one
/// test. A folder that does survive says which test left it.
///
/// It reads as the path it is: `scratch.join("mirror.sqlite")`, and `&scratch` wherever a `&Path`
/// is wanted.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    /// Makes an empty folder for a test called `name`.
    ///
    /// # Panics
    ///
    /// When the temp folder refuses a new folder, which leaves the test nothing to run on.
    #[must_use]
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        sweep_once();
        let dir = std::env::temp_dir().join(format!(
            "{PREFIX}{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        remove_patiently(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch folder");
        Self(dir)
    }

    /// The folder itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes a file into the folder, making any folders on the way, and answers its path.
    ///
    /// # Panics
    ///
    /// When the write fails, which leaves the test without its fixture.
    pub fn write(&self, name: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("make the parent folder");
        }
        std::fs::write(&path, bytes).expect("write the fixture");
        path
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        remove_patiently(&self.0);
    }
}

/// Removes a folder, waiting out a handle that is on its way to being closed.
///
/// It returns either way. A folder that cannot be removed is a leak, and a leak is better than a
/// test that hangs. Its name says which test to look at.
fn remove_patiently(dir: &Path) {
    let until = std::time::Instant::now() + PATIENCE;
    loop {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return,
            // Nothing to remove is the ordinary case on the way in.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if std::time::Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return,
        }
    }
}

/// Takes the folders an earlier run could not remove, once per process.
///
/// **Some folders outlive their own `Drop`.** A test can hand its data folder to a server that the
/// runtime owns. The connection then closes when the runtime shuts down, after every local in the
/// test body has gone, so the removal fails while the file is open. This bounds what accumulates at
/// one run's worth.
fn sweep_once() {
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(|| {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .map(|at| at.elapsed().unwrap_or_default() > STALE)
                .unwrap_or(false);
            if stale && entry.file_name().to_string_lossy().starts_with(PREFIX) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::Scratch;

    /// A scratch folder is gone once its value drops, with whatever a test wrote into it.
    #[test]
    fn a_scratch_folder_leaves_nothing_behind() {
        let scratch = Scratch::new("leaves-nothing");
        let written = scratch.write("deep/down/file.txt", b"words");
        assert_eq!(std::fs::read(&written).expect("read back"), b"words");
        let path = scratch.path().to_path_buf();
        drop(scratch);
        assert!(!path.exists(), "{} survived its drop", path.display());
    }

    /// Two scratch folders with one name in one test are two folders.
    #[test]
    fn one_name_asked_for_twice_is_two_folders() {
        let first = Scratch::new("twice");
        first.write("kept.txt", b"first");
        let second = Scratch::new("twice");
        assert_ne!(first.path(), second.path());
        assert!(
            first.join("kept.txt").exists(),
            "the second emptied the first"
        );
    }
}
