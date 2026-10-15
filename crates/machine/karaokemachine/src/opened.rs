//! Packages a file browser handed over, waiting for the display loop to install them.
//!
//! **A static queue, because the two people who fill it cannot reach the loop.** iOS's shell calls
//! [`open_package`] through `km_machine_open_package` from Objective-C. SDL's own file dialog calls
//! it from a callback that SDL runs on a thread of its own on Windows. Neither has the installer.
//! The display loop drains [`take_opened`] once a frame and hands each path to the installer a
//! dropped file reaches, so a chosen file and a dropped one go the same way.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Packages chosen since the display loop last looked, oldest first.
static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Queues a package the person chose in a file browser.
///
/// The display loop takes it within a frame. Call it on any thread.
pub fn open_package(path: impl AsRef<Path>) {
    let path = path.as_ref().to_path_buf();
    tracing::info!(path = %path.display(), "a package was chosen in the file browser");
    OPENED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(path);
}

/// The packages chosen since the last call, oldest first.
pub(crate) fn take_opened() -> Vec<PathBuf> {
    let mut opened = OPENED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::mem::take(&mut *opened)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_package_is_taken_once_in_the_order_it_came() {
        // One test owns the static, so no other test can drain it underneath this one.
        let _ = take_opened();
        open_package("first.kmpkg");
        open_package("second.kmpkg");
        assert_eq!(
            take_opened(),
            [PathBuf::from("first.kmpkg"), PathBuf::from("second.kmpkg")]
        );
        assert!(take_opened().is_empty());
    }
}
