//! Where a program keeps its files: the per-user directories, or a folder beside a portable copy.
//!
//! **Every program asks here, and nothing else names a per-user directory.** The rule this serves is
//! `A portable copy keeps its state beside its programs` in `docs/decisions/distribution.md`. A
//! portable copy must never read or write the per-user directories, and a rule of *never* holds
//! only where one function gives the answer. `clippy.toml` refuses `ProjectDirs::from` anywhere
//! else.
//!
//! # What makes a copy portable
//!
//! **A file named [`MARKER`] beside the executable.** The portable archive carries it, and a setup
//! program does not. So one build serves both, and the folder says which one it is.
//!
//! A portable copy keeps everything under [`DATA_FOLDER`] beside the executable, one folder per
//! program. Settings and data share that folder, and the cache takes a folder inside it.
//!
//! **This crate names directories and creates none.** A program creates its own, and reports its
//! own failure. A portable folder that cannot be written is therefore an error at the program that
//! tried, and never a quiet return to the per-user directories.
//!
//! # Where the marker counts
//!
//! **Windows and Linux.** A macOS bundle is sealed by its signature and cannot hold files that
//! change, so macOS reads no marker.

use std::path::{Path, PathBuf};

/// The file that makes the folder holding it a portable copy.
pub const MARKER: &str = "karaokemachine-portable.txt";

/// The folder beside the executable where a portable copy keeps every program's files.
pub const DATA_FOLDER: &str = "data";

/// The three directories one program keeps its files in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// Settings, and what a person chose.
    pub config: PathBuf,
    /// What the program built up: a catalog, packages, logs.
    pub data: PathBuf,
    /// What can be deleted and made again.
    pub cache: PathBuf,
    /// Whether these are beside the executable.
    pub portable: bool,
}

/// The directories of the program named `app`, for the executable that is running.
///
/// `None` when the copy is not portable and the platform names no home directory.
#[must_use]
pub fn for_app(app: &str) -> Option<Dirs> {
    from(exe_dir().as_deref(), app)
}

/// Whether the executable that is running belongs to a portable copy.
#[must_use]
pub fn is_portable() -> bool {
    portable_root(exe_dir().as_deref()).is_some()
}

/// The directories of `app`, decided from the executable's folder.
///
/// Kept apart from reading `current_exe`, so a test can ask about a folder it made.
#[must_use]
pub fn from(exe_dir: Option<&Path>, app: &str) -> Option<Dirs> {
    if let Some(root) = portable_root(exe_dir) {
        let own = root.join(app);
        // One folder for settings and data, as `--data-dir` gives. A person who opens the folder
        // finds `packages` one level down, and no file of one kind shares a name with the other.
        return Some(Dirs {
            config: own.clone(),
            cache: own.join("cache"),
            data: own,
            portable: true,
        });
    }
    per_user(app)
}

/// The folder a portable copy keeps its files in, or `None` when `exe_dir` is not a portable copy.
#[must_use]
pub fn portable_root(exe_dir: Option<&Path>) -> Option<PathBuf> {
    if !cfg!(any(windows, target_os = "linux")) {
        return None;
    }
    let dir = exe_dir?;
    dir.join(MARKER).is_file().then(|| dir.join(DATA_FOLDER))
}

/// The folder holding the executable that is running.
fn exe_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(Path::to_path_buf)
}

/// The platform's per-user directories for `app`.
#[allow(clippy::disallowed_methods)]
fn per_user(app: &str) -> Option<Dirs> {
    let dirs = directories::ProjectDirs::from("", "", app)?;
    Some(Dirs {
        config: dirs.config_dir().to_path_buf(),
        data: dirs.data_dir().to_path_buf(),
        cache: dirs.cache_dir().to_path_buf(),
        portable: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of this test's own, under the build's scratch directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("km-dirs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        dir
    }

    #[test]
    fn a_folder_without_the_marker_takes_the_per_user_directories() {
        let dir = scratch("plain");
        assert_eq!(portable_root(Some(&dir)), None);
        assert_eq!(from(Some(&dir), "km-example"), per_user("km-example"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_executable_folder_is_not_a_portable_copy() {
        assert_eq!(portable_root(None), None);
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_folder_with_the_marker_keeps_everything_beside_the_executable() {
        let dir = scratch("portable");
        std::fs::write(dir.join(MARKER), "").expect("the marker");

        let dirs = from(Some(&dir), "km-example").expect("a portable copy always has an answer");
        let own = dir.join(DATA_FOLDER).join("km-example");
        assert!(dirs.portable);
        assert_eq!(dirs.config, own);
        assert_eq!(dirs.data, own);
        assert_eq!(dirs.cache, own.join("cache"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn two_programs_in_one_portable_copy_keep_apart() {
        let dir = scratch("two");
        std::fs::write(dir.join(MARKER), "").expect("the marker");

        let one = from(Some(&dir), "km-one").expect("an answer");
        let two = from(Some(&dir), "km-two").expect("an answer");
        assert_ne!(one.config, two.config);
        assert_ne!(one.cache, two.cache);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_folder_named_like_the_marker_is_not_the_marker() {
        let dir = scratch("folder");
        std::fs::create_dir_all(dir.join(MARKER)).expect("a folder");
        assert_eq!(portable_root(Some(&dir)), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    #[test]
    fn this_platform_reads_no_marker() {
        let dir = scratch("ignored");
        std::fs::write(dir.join(MARKER), "").expect("the marker");
        assert_eq!(portable_root(Some(&dir)), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
