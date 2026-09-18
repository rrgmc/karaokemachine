//! The machine passwords a computer has been told to remember, keyed by machine id.
//!
//! **A file of its own, in the per-user config directory, and only if somebody ticked the box.** All
//! three halves of that are `Where a key somebody typed into a page lives` in
//! `docs/decisions/repository.md`, which `km-admin` already follows for provider keys. The product
//! rule this serves is `The password is remembered per machine, or not at all` in
//! `docs/decisions/curation.md`.
//!
//! # Two tools, one file format, and why this is a crate
//!
//! `km-package-builder` and `km-admin` both send things to a machine, both meet the same closed
//! door, and both hold a password now. They are also on **opposite sides of a workspace boundary**:
//! `tools/cmd/assets/` is excluded from the root workspace because its members need TLS from
//! `reqwest` where the builder needs it to have none, so neither can take the other by path. A
//! second copy of this is the drift the repository works to avoid, so what they share lives here,
//! where a crate with no HTTP client is safe to reach from either side.
//!
//! # What each host keeps for itself
//!
//! **Where the file goes**, which is [`file_for`]'s answer and a host's question: the environment
//! variable is that program's, and so is the name it registers with the platform. A host also keeps
//! its own `cfg!(test)` guard over that choice, and it has to: `cfg(test)` is per crate, so a guard
//! written here would be inert in the suite of whoever depends on this, and a test run would write a
//! test password into the config directory of whoever is building.
//!
//! # What is not here
//!
//! **Spending a password is a host's too.** A remembered password is a standing instruction to sign
//! in rather than a sign-in that has happened: it is exchanged for a token at the moment one is
//! wanted rather than at startup, where it would put a tool on the network before anybody had asked
//! it for anything. Only a host knows when that moment is.
//!
//! **Keyed by the machine's id, not its address**, so a machine that moves to another address is
//! still recognized. The consequence is that a machine which has never answered cannot be remembered
//! at all: there is nothing yet to key it under, and one row shared by every anonymous machine is a
//! password handed to whichever answers next. [`Remembered::remember`] refuses a blank id rather
//! than storing under `""`.
//!
//! **Forgetting deletes.** An emptied `{}` left on disk reads, to anybody who finds it, as *a
//! password is remembered here*.
//!
//! Failure to read or write is never an error: the worst case is being asked for a password that was
//! going to be asked for anyway.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The name the file takes, wherever a host puts it.
///
/// One spelling, because two tools writing two differently named files in two config directories is
/// two things for somebody clearing credentials off a computer to find.
pub const FILE_NAME: &str = "machine-passwords.json";

/// Whether this platform can make the file owner-only.
///
/// **A page says what is true rather than claiming a protection it did not apply**: on unix the file
/// is `0600`, and on Windows it is protected by the profile directory and nothing more. Refusing to
/// remember at all there would trade a real convenience for a protection that was not going to be
/// provided either way, and claiming it would be worse than both.
#[must_use]
pub fn owner_only() -> bool {
    cfg!(unix)
}

/// Where the file goes, given whatever a host's environment variable was set to.
///
/// Three answers, which are the three a run can want: **set and empty** is nowhere, so a smoke test
/// or a screenshot run remembers nothing and can read nothing that was remembered; **set** is that
/// path; **unset** is the per-user config directory of `app`.
///
/// Taken as arguments rather than read here, so the tests ask all three questions without `set_var`,
/// which is process-wide and `unsafe` in this edition.
#[must_use]
pub fn file_for(setting: Option<&std::ffi::OsStr>, app: &str) -> Option<PathBuf> {
    match setting {
        Some(named) if named.is_empty() => None,
        Some(named) => Some(PathBuf::from(named)),
        None => {
            let dirs = directories::ProjectDirs::from("", "", app)?;
            Some(dirs.config_dir().join(FILE_NAME))
        }
    }
}

/// The passwords this computer remembers, by machine id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Remembered {
    /// Machine id to password. A `BTreeMap` so the file is stable between writes.
    #[serde(default)]
    by_machine: BTreeMap<String, String>,
    /// Where this came from and goes back to, or `None` for a set that is only ever in memory.
    ///
    /// **"Nowhere to save" has to have a value**, or a set built by hand in a test could be handed
    /// to [`Self::remember`] and reach a real file.
    #[serde(skip)]
    file: Option<PathBuf>,
}

impl Remembered {
    /// Reads the set kept in `file`, which is where it will be written back.
    #[must_use]
    pub fn at(file: Option<PathBuf>) -> Self {
        let mut remembered = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .map(|text| {
                serde_json::from_str(&text).unwrap_or_else(|error| {
                    tracing::debug!("ignoring an unreadable password file: {error}");
                    Self::default()
                })
            })
            .unwrap_or_default();
        remembered.file = file;
        remembered
    }

    /// The password remembered for a machine, if there is one.
    #[must_use]
    pub fn get(&self, machine_id: &str) -> Option<&str> {
        self.by_machine.get(machine_id).map(String::as_str)
    }

    /// Whether anything is remembered for a machine.
    #[must_use]
    pub fn holds(&self, machine_id: &str) -> bool {
        self.by_machine.contains_key(machine_id)
    }

    /// Remembers a password for a machine, and writes the file.
    ///
    /// A blank id is refused rather than stored under `""`: a machine that has never answered has no
    /// identity, and one row shared by every such machine is a password handed to whichever answers
    /// next.
    pub fn remember(&mut self, machine_id: &str, password: &str) {
        if machine_id.trim().is_empty() {
            return;
        }
        self.by_machine
            .insert(machine_id.to_owned(), password.to_owned());
        self.save();
    }

    /// Forgets a machine's password, deleting the file once nothing is left in it.
    pub fn forget(&mut self, machine_id: &str) {
        if self.by_machine.remove(machine_id).is_none() {
            return;
        }
        self.save();
    }

    /// Writes the set back, best-effort, or deletes the file when there is nothing to write.
    ///
    /// **Deleting rather than writing `{}`** is the promise the checkbox makes: a file left behind
    /// says a password is remembered to anybody who finds it, and "forget it" that leaves a file is
    /// not forgetting.
    fn save(&self) {
        let Some(file) = self.file.as_ref() else {
            return;
        };
        if self.by_machine.is_empty() {
            match std::fs::remove_file(file) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::debug!("could not remove {}: {error}", file.display()),
            }
            return;
        }
        if let Some(parent) = file.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            tracing::debug!("could not make {}: {error}", parent.display());
            return;
        }
        let text = match serde_json::to_string_pretty(self) {
            Ok(text) => text,
            Err(error) => {
                tracing::debug!("could not serialize the password file: {error}");
                return;
            }
        };
        if let Err(error) = std::fs::write(file, text) {
            tracing::debug!("could not write {}: {error}", file.display());
            return;
        }
        restrict(file);
    }
}

/// Makes the file owner-only where the platform has a one-line way to say so.
///
/// Applied after the write rather than through the open, because `std::fs::write` is what every
/// other file in these tools uses and a bespoke `OpenOptions` here would be a second spelling of
/// writing a file. The window between the two is the writing process's own.
#[cfg(unix)]
fn restrict(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(error) = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)) {
        tracing::debug!("could not restrict {}: {error}", file.display());
    }
}

/// Windows has no one-line equivalent a program here can apply, and [`owner_only`] is what says so
/// on the page.
#[cfg(not(unix))]
fn restrict(_file: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty directory of this test's own, removed when it drops.
    ///
    /// Hand-rolled rather than `tempfile`, which the root workspace does not carry: one directory
    /// per test is not worth a dependency, and the thread id is what keeps two of these apart when
    /// the suite runs in parallel.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-passwords-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("make the scratch directory");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (Scratch, PathBuf) {
        let scratch = Scratch::new();
        let file = scratch.0.join(FILE_NAME);
        (scratch, file)
    }

    /// The three answers [`file_for`] gives, which are the three a run can want.
    #[test]
    fn the_environment_names_the_file_or_says_there_is_none() {
        use std::ffi::OsStr;
        assert_eq!(
            file_for(Some(OsStr::new("")), "km-admin"),
            None,
            "set and empty is nowhere"
        );
        assert_eq!(
            file_for(Some(OsStr::new("/tmp/pw.json")), "km-admin"),
            Some(PathBuf::from("/tmp/pw.json"))
        );
        // Unset is the platform's config directory, which a build machine may not have.
        if let Some(default) = file_for(None, "km-admin") {
            assert!(default.ends_with(FILE_NAME), "{default:?}");
        }
    }

    /// Two hosts name themselves, and neither writes into the other's directory.
    #[test]
    fn each_host_gets_its_own_directory() {
        let (Some(builder), Some(admin)) = (
            file_for(None, "km-package-builder"),
            file_for(None, "km-admin"),
        ) else {
            return;
        };
        assert_ne!(
            builder, admin,
            "two tools sharing one file would hand each other's passwords around"
        );
    }

    #[test]
    fn a_password_survives_a_reload_and_is_keyed_by_the_machine() {
        let (_scratch, file) = store();
        let mut remembered = Remembered::at(Some(file.clone()));
        remembered.remember("abc123", "hunter2");
        assert!(file.is_file(), "the file was not written");

        let reloaded = Remembered::at(Some(file));
        assert_eq!(reloaded.get("abc123"), Some("hunter2"));
        assert_eq!(reloaded.get("somebody-else"), None);
    }

    /// Forgetting deletes the file rather than leaving `{}` behind, which would read as *a password
    /// is remembered here*.
    #[test]
    fn forgetting_the_last_password_removes_the_file() {
        let (_scratch, file) = store();
        let mut remembered = Remembered::at(Some(file.clone()));
        remembered.remember("abc123", "hunter2");
        remembered.remember("def456", "swordfish");

        remembered.forget("abc123");
        assert!(file.is_file(), "one machine is still remembered");
        assert_eq!(
            Remembered::at(Some(file.clone())).get("def456"),
            Some("swordfish")
        );

        remembered.forget("def456");
        assert!(
            !file.exists(),
            "an emptied file says a password is remembered"
        );
    }

    /// A machine that has never answered has no id, and nothing is written under a blank one.
    #[test]
    fn a_machine_with_no_identity_is_not_remembered() {
        let (_scratch, file) = store();
        let mut remembered = Remembered::at(Some(file.clone()));
        remembered.remember("", "hunter2");
        remembered.remember("   ", "hunter2");
        assert!(!file.exists(), "a blank id is not a machine");
        assert_eq!(remembered.get(""), None);
    }

    /// A set with nowhere to go never reaches a disk, which is what a test run holds.
    #[test]
    fn a_set_with_no_file_is_only_ever_in_memory() {
        let mut remembered = Remembered::at(None);
        remembered.remember("abc123", "hunter2");
        assert_eq!(remembered.get("abc123"), Some("hunter2"));
    }
}
