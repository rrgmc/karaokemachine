//! Finding the machine, which holds the lyric sync editor.
//!
//! This program ships beside the machine and never holds a path to it. Each install puts the two
//! in a known arrangement, so the search is a short list of places and the first hit wins.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The environment variable that names the machine's executable, where the flag cannot be passed.
pub const MACHINE_EXE_ENV: &str = "KM_MACHINE_EXE";

/// What the machine's executable is called, without the platform's extension.
const NAME: &str = "karaokemachine";

/// What the machine's macOS bundle is called.
const BUNDLE: &str = "Karaoke Machine.app";

/// The machine, as this program can start it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Machine {
    /// An executable, started as a child process.
    Binary(PathBuf),
    /// A macOS bundle, which the system starts on this program's behalf.
    Bundle(PathBuf),
}

/// Finds the machine.
///
/// `given` is what `--machine-exe` or [`MACHINE_EXE_ENV`] named, and it wins when it exists. The
/// rest follow the installs:
///
/// * beside this program: the Windows install, a portable copy, and a build folder;
/// * one folder up: the Debian package, whose tools sit in a folder under the machine;
/// * the bundle beside this program's bundle, then the one in `/Applications`;
/// * the search path: the Linux tarball, whose installer links the machine there.
#[must_use]
pub fn find(
    given: Option<&Path>,
    exe_dir: Option<&Path>,
    search: Option<&OsStr>,
) -> Option<Machine> {
    if let Some(given) = given {
        return given.exists().then(|| of(given.to_path_buf()));
    }
    let file = format!("{NAME}{}", std::env::consts::EXE_SUFFIX);
    let mut places: Vec<PathBuf> = Vec::new();
    if let Some(dir) = exe_dir {
        places.push(dir.join(&file));
        if let Some(above) = dir.parent() {
            places.push(above.join(&file));
        }
        // `<folder>/<this>.app/Contents/MacOS` is where a bundled executable runs from.
        if let Some(folder) = dir.ancestors().nth(3) {
            places.push(folder.join(BUNDLE));
        }
    }
    if cfg!(target_os = "macos") {
        places.push(Path::new("/Applications").join(BUNDLE));
    }
    if let Some(search) = search {
        places.extend(std::env::split_paths(search).map(|dir| dir.join(&file)));
    }
    places.into_iter().find(|place| place.exists()).map(of)
}

/// A bundle is a folder with the bundle extension, and anything else is an executable.
fn of(path: PathBuf) -> Machine {
    if path.extension().is_some_and(|extension| extension == "app") && path.is_dir() {
        Machine::Bundle(path)
    } else {
        Machine::Binary(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use km_testkit::Scratch;

    fn machine_file() -> String {
        format!("{NAME}{}", std::env::consts::EXE_SUFFIX)
    }

    #[test]
    fn the_machine_beside_this_program_is_found_before_the_search_path() {
        let scratch = Scratch::new("sync-machine-beside");
        let beside = scratch.join("bin");
        let elsewhere = scratch.join("elsewhere");
        for dir in [&beside, &elsewhere] {
            std::fs::create_dir_all(dir).expect("making a test folder");
            std::fs::write(dir.join(machine_file()), b"").expect("writing a test file");
        }

        let found = find(None, Some(&beside), Some(elsewhere.as_os_str()));
        assert_eq!(found, Some(Machine::Binary(beside.join(machine_file()))));
    }

    #[test]
    fn the_machine_one_folder_up_is_found() {
        let scratch = Scratch::new("sync-machine-above");
        let tools = scratch.join("tools");
        std::fs::create_dir_all(&tools).expect("making a test folder");
        std::fs::write(scratch.join(machine_file()), b"").expect("writing a test file");

        let found = find(None, Some(&tools), None);
        assert_eq!(found, Some(Machine::Binary(scratch.join(machine_file()))));
    }

    #[test]
    fn the_bundle_beside_this_programs_bundle_is_found() {
        let scratch = Scratch::new("sync-machine-bundle");
        let inside = scratch.join("KM Song Sync.app/Contents/MacOS");
        std::fs::create_dir_all(&inside).expect("making a test folder");
        std::fs::create_dir_all(scratch.join(BUNDLE)).expect("making a test folder");

        let found = find(None, Some(&inside), None);
        assert_eq!(found, Some(Machine::Bundle(scratch.join(BUNDLE))));
    }

    #[test]
    fn a_named_machine_wins_and_a_missing_one_is_not_replaced() {
        let scratch = Scratch::new("sync-machine-given");
        std::fs::write(scratch.join(machine_file()), b"").expect("writing a test file");
        let named = scratch.join("another");
        std::fs::write(&named, b"").expect("writing a test file");

        let found = find(Some(&named), Some(scratch.path()), None);
        assert_eq!(found, Some(Machine::Binary(named)));
        // Somebody who names a machine means that one. Starting another would hide the mistake.
        let missing = scratch.join("not-there");
        assert_eq!(find(Some(&missing), Some(scratch.path()), None), None);
    }

    #[test]
    fn the_search_path_is_the_last_place_looked() {
        let scratch = Scratch::new("sync-machine-path");
        let empty = scratch.join("empty");
        let linked = scratch.join("linked");
        for dir in [&empty, &linked] {
            std::fs::create_dir_all(dir).expect("making a test folder");
        }
        std::fs::write(linked.join(machine_file()), b"").expect("writing a test file");
        let search = std::env::join_paths([&empty, &linked]).expect("joining the search path");

        let found = find(None, Some(&empty), Some(&search));
        assert_eq!(found, Some(Machine::Binary(linked.join(machine_file()))));
        assert_eq!(find(None, Some(&empty), None), None);
    }
}
