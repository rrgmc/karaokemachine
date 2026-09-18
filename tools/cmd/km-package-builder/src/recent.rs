//! The folders this machine has curated before.
//!
//! The one piece of state in this tool that is **not** in the curated folder. Everything else about a
//! corpus lives beside the corpus, deliberately, so that a second machine pointed at the same drive
//! picks up where the first left off (see the `Curation database` decision in `docs/decisions/`). This
//! is the opposite kind of fact — *which* corpora this person opens — and it cannot live in any of
//! them, because the question is asked before one is chosen.
//!
//! So it goes where `km-app` and `km-remote` already put per-user state: the platform's config
//! directory, via `directories::ProjectDirs`.
//!
//! Failure to read or write it is never an error. A missing, unreadable or corrupt list means the
//! Open page shows no recents, which is exactly what it shows the first time anyway; refusing to
//! start a curation tool because a convenience file could not be parsed would be absurd.
//!
//! **A `cargo test` run has nowhere to save it**, and that is structural rather than a rule to
//! remember: [`path`] answers `None` under `cfg(test)`, so every list a test holds is in memory. The
//! two other per-user files in this workspace are already kept out of a test run by being handed a
//! directory (`--data-dir`); this one is never named on a command line, so the guard goes here
//! instead. See `No test writes into the user's own config directory` in `CONTRIBUTING.md`.
//!
//! **…and a `cargo test` was never the only run that is not curation.** A screenshot pipeline, a
//! smoke test and a scripted build all open real folders through the real binary, where a `cfg` says
//! nothing at all. [`ENV_VAR`] is what those runs set: it names the file this run's list goes in, and
//! set-but-empty says there is no file. It is an environment variable rather than a flag because the
//! runs that must not write here are the ones a script starts, and a script always has an
//! environment. See `Whose recent list a run writes` in `docs/decisions/curation.md`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many folders to remember.
///
/// Enough that a person moving between the handful of corpora they actually work on never has to
/// browse for one, and few enough that the list stays a list rather than a history.
const KEEP: usize = 12;

/// One remembered folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Where it is.
    pub path: PathBuf,
    /// Songs indexed when it was last open, for the line under the path.
    #[serde(default)]
    pub songs: u32,
    /// Files indexed when it was last open.
    #[serde(default)]
    pub files: u32,
}

/// The remembered list, newest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Recent {
    /// Most recently opened first.
    #[serde(default)]
    pub folders: Vec<Entry>,
    /// Where this list came from and goes back to, or `None` for one that is only ever in memory.
    ///
    /// Not in the file — it *is* the file — and the reason it is a field rather than a call to
    /// [`path`] at each end is that "nowhere to save" then has a value. A default `Recent` is one,
    /// which is what makes a list a test built by hand unable to reach the config directory even if
    /// something later hands it to `remember`.
    #[serde(skip)]
    file: Option<PathBuf>,
}

/// The environment variable that says where this run's list goes.
///
/// Named after the file rather than the directory holding it, because it moves that one file and
/// nothing else: the log files and the webview profile are in this tool's *data* directory, and a
/// name reading `…_CONFIG_DIR` would claim to cover them.
pub const ENV_VAR: &str = "KM_PACKAGE_BUILDER_RECENT";

/// Where the list goes, given whatever [`ENV_VAR`] was set to.
///
/// Three answers, and the middle one is the point:
///
/// - **unset** — the file this machine has always kept, in the platform's config directory.
/// - **set and empty** — nowhere. The run remembers nothing and, because `folder_to_reopen` reads
///   the same list, reopens nothing either. That is the whole opt-out: no file to clean up
///   afterwards, and a run that names its own folder had no use for last time's.
/// - **set** — that file, whose parent [`Recent::save`] makes if it is not there.
///
/// Takes the setting rather than reading it, so the tests can ask all three questions without
/// `set_var` — which is process-wide, `unsafe` in this edition, and would race every other test in
/// the binary.
fn file_for(setting: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    match setting {
        Some(named) if named.is_empty() => None,
        Some(named) => Some(PathBuf::from(named)),
        None => {
            let dirs = directories::ProjectDirs::from("", "", "km-package-builder")?;
            Some(dirs.config_dir().join("recent.json"))
        }
    }
}

/// Where the list is kept, or `None` if there is nowhere to keep it.
///
/// Three reasons for `None`, and the middle one is the interesting one:
///
/// - the platform will not say where per-user config goes,
/// - **this is a test run.** The list is a real file in the config directory of whoever is building,
///   and a suite that wrote it evicts the corpora they actually curate — [`KEEP`] entries is the
///   whole list, and a scratch folder goes in at the *top*. It cost exactly that: eight
///   `km-package-builder-server-<pid>-…` temp folders above the one real corpus, and with them the
///   startup reopen, which takes `folders.first()` and found a scratch folder rather than the corpus
///   the person had been working on.
/// - or [`ENV_VAR`] is set and empty, which is a run saying it is not curation.
///
/// The test guard is checked **first**, deliberately: somebody who keeps the variable set in their
/// shell should still get a suite that writes nowhere rather than one that writes their scratch file.
///
/// `cfg!` rather than `#[cfg]` so the production branch is still compiled and linted under test.
fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    file_for(std::env::var_os(ENV_VAR).as_deref())
}

impl Recent {
    /// Reads the list this machine keeps, or an empty one.
    pub fn load() -> Self {
        Self::at(path())
    }

    /// Reads the list kept in `file`, which is where it will be written back.
    ///
    /// `None` is a list with nowhere to go, which is what a test run gets from [`path`] and what the
    /// tests below hand in deliberately.
    fn at(file: Option<PathBuf>) -> Self {
        let mut recent = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .map(|text| {
                serde_json::from_str(&text).unwrap_or_else(|error| {
                    tracing::debug!("ignoring an unreadable recent-folder list: {error}");
                    Self::default()
                })
            })
            .unwrap_or_default();
        recent.file = file;
        recent
    }

    /// Writes the list back, best-effort.
    fn save(&self) {
        let Some(file) = self.file.as_ref() else {
            return;
        };
        if let Some(parent) = file.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            tracing::debug!("could not make {}: {error}", parent.display());
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(error) = std::fs::write(file, text) {
                    tracing::debug!("could not write {}: {error}", file.display());
                }
            }
            Err(error) => tracing::debug!("could not serialize the recent-folder list: {error}"),
        }
    }

    /// Puts a folder at the top, with the counts it had when it was open.
    ///
    /// Moves rather than duplicates: opening the same folder twice leaves one entry, at the top.
    /// Comparison is on the tidied absolute path, which is what every caller has.
    pub fn remember(&mut self, root: &Path, songs: u32, files: u32) {
        self.folders.retain(|entry| entry.path != root);
        self.folders.insert(
            0,
            Entry {
                path: root.to_path_buf(),
                songs,
                files,
            },
        );
        self.folders.truncate(KEEP);
        self.save();
    }

    /// Takes a folder out of the list.
    pub fn forget(&mut self, root: &Path) {
        self.folders.retain(|entry| entry.path != root);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::OsStr;

    use crate::testing::Scratch;

    #[test]
    fn remembering_moves_rather_than_duplicates() {
        // `remember` in full, `save` included: a list built here has nowhere to write, so the call
        // is safe to make.
        let mut recent = Recent::default();
        recent.remember(Path::new("/a"), 1, 1);
        recent.remember(Path::new("/b"), 2, 2);
        recent.remember(Path::new("/b"), 9, 9);

        assert_eq!(recent.folders.len(), 2);
        assert_eq!(recent.folders[0].path, PathBuf::from("/b"));
        assert_eq!(recent.folders[0].songs, 9);
    }

    /// A test run cannot reach the list somebody actually curates from.
    ///
    /// The fault this closes: one test called `State::begin_open`, whose worker thread records the
    /// folder it opened, and every suite run put a scratch folder at the top of the real
    /// `recent.json` — evicting real corpora from a twelve-entry list and breaking the startup
    /// reopen, which takes the first entry and found a deleted temp directory.
    #[test]
    fn a_test_run_has_nowhere_to_save() {
        // Whatever the environment says. The guard is checked before `ENV_VAR`, so a shell that
        // happens to have one set does not hand the suite a file to write after all.
        assert!(
            path().is_none(),
            "a test run just named the config file it is about to overwrite"
        );
        assert!(
            Recent::load().file.is_none(),
            "loading under test picked up a file to write back to"
        );
    }

    /// A run told where to keep its list keeps it there.
    ///
    /// The case this exists for is the one the guard above cannot see: `tools/dev/screenshots.sh`
    /// and any smoke test open a real folder through the real binary, not under `cfg(test)`.
    #[test]
    fn a_named_file_is_where_the_list_goes() {
        assert_eq!(
            file_for(Some(OsStr::new("/tunes/scratch/recent.json"))),
            Some(PathBuf::from("/tunes/scratch/recent.json"))
        );
    }

    /// ...and a run told to keep it nowhere ends up exactly where a test run is.
    #[test]
    fn an_empty_setting_means_no_list_at_all() {
        assert!(file_for(Some(OsStr::new(""))).is_none());
    }

    /// Unset is still the file this machine has always kept — named only, never opened.
    #[test]
    fn an_unset_variable_still_means_the_config_directory() {
        // `None` on a platform that will not say where config goes, which is not a failure.
        if let Some(file) = file_for(None) {
            assert!(file.ends_with("recent.json"));
            assert!(file.to_string_lossy().contains("km-package-builder"));
        }
    }

    /// ...and the writing that a test run does not do still works, against a file of its own.
    #[test]
    fn a_list_survives_being_written_and_read_back() {
        let scratch = Scratch::new("round-trip");
        let file = scratch.0.join("recent.json");

        let mut written = Recent::at(Some(file.clone()));
        assert!(written.folders.is_empty(), "nothing has been written yet");
        written.remember(Path::new("/tunes/karaoke"), 120_500, 310_400);

        let read = Recent::at(Some(file));
        assert_eq!(read.folders.len(), 1);
        assert_eq!(read.folders[0].path, PathBuf::from("/tunes/karaoke"));
        assert_eq!(read.folders[0].songs, 120_500);
    }

    /// The list is capped, which is what makes an unwanted entry an eviction rather than clutter.
    #[test]
    fn the_list_stays_a_list() {
        let mut recent = Recent::default();
        for n in 0..KEEP + 5 {
            recent.remember(&PathBuf::from(format!("/corpus-{n}")), 0, 0);
        }
        assert_eq!(recent.folders.len(), KEEP);
        assert_eq!(
            recent.folders[0].path,
            PathBuf::from(format!("/corpus-{}", KEEP + 4)),
            "the newest is first"
        );
    }

    #[test]
    fn an_unreadable_list_is_an_empty_list_rather_than_an_error() {
        let parsed: Result<Recent, _> = serde_json::from_str("not json at all");
        assert!(parsed.is_err());
        // Which is what `load` turns into a default rather than propagating.
        let recovered = parsed.unwrap_or_default();
        assert!(recovered.folders.is_empty());
    }

    #[test]
    fn a_list_written_by_an_older_build_still_reads() {
        // `songs` and `files` were added after the first version; `serde(default)` is what keeps an
        // older file loadable rather than silently emptying the list.
        let entry: Entry = serde_json::from_str(r#"{"path":"/corpus"}"#).expect("older shape");
        assert_eq!(entry.path, PathBuf::from("/corpus"));
        assert_eq!(entry.songs, 0);
    }

    /// A key this build does not know is read past rather than refusing the list.
    ///
    /// The file is a convenience, and one written by a build that kept more in it than this one does
    /// must still open the folders it names.
    #[test]
    fn a_list_carrying_a_key_this_build_does_not_know_still_reads() {
        let entry: Entry =
            serde_json::from_str(r#"{"path":"/corpus","songs":9,"filter":"language=pt"}"#)
                .expect("unknown key");
        assert_eq!(entry.path, PathBuf::from("/corpus"));
        assert_eq!(entry.songs, 9);
    }

    /// Opening a folder again moves it to the top with the counts it now has.
    #[test]
    fn opening_a_folder_again_moves_it_up_rather_than_duplicating_it() {
        let mut recent = Recent::default();
        recent.remember(Path::new("/tunes/karaoke"), 1, 1);
        recent.remember(Path::new("/tunes/other"), 0, 0);
        recent.remember(Path::new("/tunes/karaoke"), 120_500, 310_400);

        assert_eq!(recent.folders.len(), 2);
        assert_eq!(recent.folders[0].path, PathBuf::from("/tunes/karaoke"));
        assert_eq!(recent.folders[0].songs, 120_500);
    }
}
