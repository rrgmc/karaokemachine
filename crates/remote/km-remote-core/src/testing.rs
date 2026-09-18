//! What the tests in this crate need and the remote does not.
//!
//! One type, and it earns a module by having had **five** copies: `struct Scratch(PathBuf)` in
//! `tests` and in `link`, two directories built inline in `mirror` that were never removed at all,
//! and a `scratch()` in `find` that returns a bare path. `km-package-builder`'s own `testing` module
//! is the same consolidation for the same reason.
//!
//! **The removal has to be patient, and that is the part worth writing down.** Every directory here
//! holds a SQLite file, and Windows refuses to delete a file another handle still has open. A remote
//! that has been asked to stop releases its connection on a thread of its own, so a removal issued
//! the instant the test's last value drops can lose that race and fail — silently, since a scratch
//! directory is not worth failing a green test over. It lost it on every run of three tests, which
//! is how thirteen thousand directories came to be sitting in the temp folder, the oldest from
//! 2024-12-13.

use std::path::{Path, PathBuf};

/// How long [`Scratch`] waits for whoever still holds a file in it.
///
/// Long enough for a connection being closed on another thread, short enough that a directory
/// something holds forever costs a test a second rather than hanging it.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

/// A scratch directory that removes itself.
///
/// The name carries the process id, the test's own name and the thread the test is running on —
/// enough that two suites in two checkouts cannot collide, and that a directory which does survive
/// says which test left it.
pub struct Scratch(pub PathBuf);

/// The prefix every directory here is named with, and what the sweep recognizes its own by.
const PREFIX: &str = "km-remote-core-";

/// How old a directory has to be before the sweep will take it.
///
/// Longer than any run of this suite, so a checkout building beside this one keeps its own — the
/// name carries a process id, but asking the operating system whether that process is alive is more
/// machinery than an age comparison for the same answer.
const STALE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

impl Scratch {
    /// Makes an empty directory for a test called `name`.
    pub fn new(name: &str) -> Self {
        sweep_once();
        let dir = std::env::temp_dir().join(format!(
            "{PREFIX}{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        remove_patiently(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    /// The directory itself, for the many callers that want a `&Path`.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// So that a scratch directory reads as the path it is: `scratch.join("mirror.sqlite")`, and
/// `&scratch` wherever a `&Path` is wanted.
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

/// Removes a directory, waiting out a handle that is on its way to being closed.
///
/// Returns either way: a directory that cannot be removed is a leak worth having over a test that
/// hangs, and the name says which test to look at.
fn remove_patiently(dir: &Path) {
    let until = std::time::Instant::now() + PATIENCE;
    loop {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return,
            // Nothing to remove is the ordinary case on the way in.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if std::time::Instant::now() < until => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => return,
        }
    }
}

/// Takes the directories an earlier run could not remove, once per process.
///
/// **Two tests here cannot be cleaned up by their own `Drop`, and this is what answers for them.**
/// `a_server_stops_when_asked` and its early-stop twin hand the data directory to a `Server` that
/// the runtime owns; the connection closes when the runtime shuts down, which is after every local
/// in the test body has already gone. So the removal is issued while the file is open, fails, and
/// the directory stays until some later process can take it. That is this: the growth is bounded at
/// one run's worth rather than left to accumulate, which is what thirteen thousand of them were.
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
