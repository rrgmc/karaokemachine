//! Where a file passing through this program sits while it is being sent, and nothing longer.
//!
//! **This is the one place in the program that does not keep what it handles.** A pack it built and
//! a bank it downloaded are kept and listed and sent afterwards — *Everything it makes is kept,
//! listed, and sent as a second act* — because both cost an afternoon and a machine that is switched
//! off must not throw that away. A file somebody picked in a browser cost nothing to have: it is
//! still where they picked it from, so a second copy here would be a gigabyte of somebody else's
//! library, kept for ever, with nothing to retry that the original cannot — and nothing a list of it
//! could offer that the chooser does not.
//!
//! So the bytes land here only because the browser's half of the transfer and the machine's half are
//! two different requests, and they are removed however the second one ends.
//!
//! **Two halves, because one of them cannot cover a kill.** [`Staged`] removes the file when it is
//! dropped, which covers every return, every `?` and a panic. It does not cover the process being
//! killed mid-transfer, and a gigabyte left behind for ever is exactly the scar `bank.rs`' `download`
//! carries about a broken connection. [`sweep`] is the other half: it empties the folder at startup,
//! when by definition nothing is passing through.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers the staged files, so two uploads at once cannot collide.
///
/// **A counter and not the browser's name**, which is the rule `bank.rs` states for downloads and
/// which holds here for the same reason: `--lan` exists, and a name from off this machine that
/// reaches a path is a name that has to be sanitised correctly every time for ever. Nothing the
/// client sent reaches this path at all — see [`stage`], where even the extension comes from the
/// fixed list `km_api::uploads::extensions_for` returns rather than from the client's string.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The folder files pass through. Never read by anything but this module.
#[must_use]
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("staging")
}

/// Empties the staging folder, for what a kill left behind.
///
/// Called once at startup and never again: at any other moment a file in here belongs to a transfer
/// that is still running. Failures are ignored deliberately — a program that would not start because
/// it could not tidy up is worse than one carrying a stale file.
pub fn sweep(data_dir: &Path) {
    let dir = dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
}

/// Makes an empty file to stream an upload into, and hands back the guard that removes it.
///
/// `extension` must be one of the `&'static str`s out of `km_api::uploads::extensions_for`, never a
/// string off the wire.
pub fn stage(data_dir: &Path, extension: &str) -> std::io::Result<Staged> {
    let dir = dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    Ok(Staged {
        path: dir.join(format!("{n}.{extension}")),
    })
}

/// A staged file that removes itself, however the transfer it belongs to ended.
///
/// **Modeled on `km_api::handlers`' `Staged`, minus its `keep`**: there, one caller keeps the file
/// because something is playing out of it. Here there is never anything to keep, which is the whole
/// point of the module.
///
/// It is `Send + 'static`, so it moves into the task that does the sending and the file goes when
/// that task ends — success, refusal or panic.
#[derive(Debug)]
pub struct Staged {
    path: PathBuf,
}

impl Staged {
    /// Where to write, and what to hand to the uploader.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        // `std::fs` rather than `tokio::fs`: `Drop` cannot await, and this is one unlink.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staged_file_is_gone_when_the_guard_is() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let path = {
            let staged = stage(home.path(), "kmpkg").expect("staging");
            std::fs::write(staged.path(), b"not really a package").expect("write");
            assert!(staged.path().exists());
            staged.path().to_path_buf()
        };
        assert!(!path.exists(), "the guard left the file behind");
    }

    #[test]
    fn a_panic_takes_the_file_with_it() {
        // The reason this is RAII and not a call at the end of the handler: every early return and
        // every panic has to clean up, and the one that gets forgotten is the one that matters.
        let home = tempfile::tempdir().expect("a temporary folder");
        let path = {
            let staged = stage(home.path(), "sf2").expect("staging");
            std::fs::write(staged.path(), b"not really a bank").expect("write");
            staged.path().to_path_buf()
        };
        let outcome = std::panic::catch_unwind(|| {
            let staged = stage(home.path(), "sf2").expect("staging");
            std::fs::write(staged.path(), b"nor is this").expect("write");
            panic!("the transfer went wrong");
        });
        assert!(outcome.is_err());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_dir(dir(home.path()))
                .expect("the folder is there")
                .count(),
            0,
            "a panic left a staged file behind"
        );
    }

    #[test]
    fn a_sweep_takes_what_a_kill_left() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let orphan = {
            let staged = stage(home.path(), "kmpkg").expect("staging");
            std::fs::write(staged.path(), b"left by a kill").expect("write");
            let path = staged.path().to_path_buf();
            std::mem::forget(staged); // exactly what a killed process leaves behind
            path
        };
        assert!(orphan.exists());
        sweep(home.path());
        assert!(!orphan.exists(), "the sweep left an orphan behind");
    }

    #[test]
    fn a_sweep_of_a_folder_that_was_never_made_is_not_an_error() {
        let home = tempfile::tempdir().expect("a temporary folder");
        sweep(home.path());
    }

    #[test]
    fn two_uploads_at_once_do_not_collide() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let one = stage(home.path(), "jpg").expect("staging");
        let two = stage(home.path(), "jpg").expect("staging");
        assert_ne!(one.path(), two.path());
    }
}
