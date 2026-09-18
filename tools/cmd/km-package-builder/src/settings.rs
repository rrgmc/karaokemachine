//! What this person wants the tool to do, as opposed to what this corpus holds.
//!
//! The second file in this tool that is **not** in the curated folder, and it is there for
//! [`crate::recent`]'s reason read one step further. Everything about a corpus lives beside the
//! corpus, so that a second machine pointed at the same drive picks up where the first left off —
//! see the `Curation database` decision. A suggested vocabulary is not a fact about any corpus: it
//! is what *this curator* thinks a good tag looks like, and it should follow them from one folder to
//! the next.
//!
//! So it goes where `recent.json` already goes: the platform's config directory, via
//! `directories::ProjectDirs`.
//!
//! **Failure to read is never an error.** A missing, unreadable or corrupt file means the built-in
//! defaults, which is what a first run gets anyway. Refusing to start a curation tool because a
//! preferences file has a stray comma in it would be absurd.
//!
//! **Written on the first run that finds none, and afterwards only when somebody asks.** The
//! Settings page edits it; nothing else does. A file that the tool rewrote on its own would
//! eventually undo an edit made in a text editor, and both routes have to stay usable — the page is
//! the one somebody finds, and the file is the one they can put in a dotfiles repository.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The tags a fresh install suggests.
///
/// **Three, and deliberately not thirty.** These are a hint about the *shape* of a good tag — short,
/// lower case, a genre rather than a sentence — and a list long enough to browse would be answering
/// a question nobody asked, in a vocabulary that is meant to be the curator's own. `classic-rock` is
/// in it because it is the one that shows the hyphen, which is the part of the convention somebody
/// would otherwise have to discover by typing a space and watching what happened.
pub const DEFAULT_TAGS: [&str; 3] = ["pop", "rock", "classic-rock"];

/// The environment variable that says where this run's settings live.
///
/// [`crate::recent::ENV_VAR`]'s twin, and it exists for the same reason: a scripted run — a
/// screenshot pipeline, a smoke test — must be able to say *not mine*, and a script always has an
/// environment. Set and empty means read the built-in defaults and write nothing.
pub const ENV_VAR: &str = "KM_PACKAGE_BUILDER_SETTINGS";

/// What this curator has told the tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Tags offered in every picker whether or not a song here carries one.
    ///
    /// A hint about what to file songs under, which is what makes an untouched corpus tagable at
    /// all: the vocabulary is otherwise read off the songs, and a corpus nobody has tagged has none,
    /// so the datalist would be empty and the bar's picker would not be drawn.
    ///
    /// **A suggestion never reaches a package.** `build::spec_for` reads `song_tags` per member, so
    /// a word nobody has put on a song has no row to be read from — which is the property that lets
    /// this be a hint rather than a commitment, and it is asserted rather than assumed.
    ///
    /// **And it never reaches a remote.** The machine's catalog is built from packages, so the same
    /// property covers that: a suggestion that is on no song is in no package, so it is in no
    /// catalog and in no phone's mirror. This is a curation-tool preference and nothing else.
    ///
    /// Folded through `km_kmpkg::Tag` when read, so a hand-edited file holding `Classic Rock` offers
    /// `classic-rock` rather than a word no song could ever match.
    pub default_tags: Vec<String>,

    /// Which language these pages are drawn in, as a BCP 47 tag.
    ///
    /// **`locale`, not `language`.** `language` is what a song is sung in everywhere in this tool —
    /// the browse bar's filter, a package's default, `/songs/language-bulk`. See
    /// `The interface has a locale; a song has a language`.
    ///
    /// **`None` means follow the browser**, and it is the state a fresh install is in. A curator
    /// whose browser asks for Portuguese gets Portuguese without finding the picker first, which is
    /// the half worth stating: a language picker is the one control somebody who needs it cannot
    /// read the page around. The picker is what writes a tag here, and nothing else does.
    ///
    /// A `String` rather than a `Locale`, because this is a file somebody can edit and a tag this
    /// build has no catalog for has to be readable without refusing to start. [`Settings::locale`]
    /// is where it becomes one.
    pub locale: Option<String>,

    /// Where this tool's log goes, how much detail is in it, and how many runs are kept.
    ///
    /// **The one section here that is about a run rather than about a corpus**, and it is here
    /// because a tool started by double-clicking its icon has no command line to be told on. The
    /// rest of this file follows the curator from one folder to the next; this follows the box.
    ///
    /// Read by [`peek_logging`] before the subscriber exists, and carried on the struct as well so
    /// that the Settings page writing a tag back cannot drop it.
    #[serde(default)]
    pub logging: km_logsettings::LoggingSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_tags: DEFAULT_TAGS.iter().map(|tag| (*tag).to_owned()).collect(),
            locale: None,
            logging: km_logsettings::LoggingSettings::default(),
        }
    }
}

/// What the settings file says about logging, without writing one or reporting on one.
///
/// **Before [`Settings::load`] rather than through it**, because the subscriber has to exist before
/// anything can be said and `load` seeds a missing file — so asking it where the log goes would
/// create a `settings.json` for a run that has said nothing yet. `None` from [`path`] is a run with
/// nowhere to read from, which answers as no opinion at all.
#[must_use]
pub fn peek_logging() -> km_logsettings::LoggingSettings {
    path().map(km_logsettings::peek).unwrap_or_default()
}

/// Where the settings live, given whatever [`ENV_VAR`] was set to.
///
/// [`crate::recent`]'s `file_for`, with the same three answers and the same reason for taking the
/// setting rather than reading it: `set_var` is process-wide, `unsafe` in this edition, and would
/// race every other test in the binary.
fn file_for(setting: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    match setting {
        Some(named) if named.is_empty() => None,
        Some(named) => Some(PathBuf::from(named)),
        None => {
            let dirs = directories::ProjectDirs::from("", "", "km-package-builder")?;
            Some(dirs.config_dir().join("settings.json"))
        }
    }
}

/// Where the settings are kept, or `None` if there is nowhere.
///
/// The test guard is first for `recent::path`'s reason, and it matters more here than there: this
/// file is *written* on a first run, so a suite without the guard would create one in the config
/// directory of whoever is building — and then every later run would read a file the tests made.
///
/// `cfg!` rather than `#[cfg]`, so the production branch is still compiled and linted under test.
fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    file_for(std::env::var_os(ENV_VAR).as_deref())
}

impl Settings {
    /// Reads this machine's settings, writing the defaults out if there are none yet.
    pub fn load() -> Self {
        Self::at(path())
    }

    /// Reads the settings kept in `file`, seeding it when it is not there.
    ///
    /// `None` is settings with nowhere to live: the defaults, and nothing written. That is what a
    /// test run gets from [`path`] and what the tests below hand in deliberately.
    fn at(file: Option<PathBuf>) -> Self {
        let Some(file) = file else {
            return Self::default();
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                // Kept, not replaced. A file somebody was halfway through editing is worth more
                // than the defaults, and silently overwriting it is how an afternoon's list is lost
                // to a stray comma.
                tracing::warn!(
                    "{} could not be read ({error}); using the built-in defaults and leaving the \
                     file alone",
                    file.display()
                );
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // **Written out rather than left implicit**, which is the one thing this file does
                // that a constant could not: somebody who wants to change the list has to be able
                // to find it, and an empty config directory tells them nothing.
                let settings = Self::default();
                settings.write(&file);
                settings
            }
            Err(error) => {
                tracing::debug!("could not read {}: {error}", file.display());
                Self::default()
            }
        }
    }

    /// Replaces the suggested tags and writes them back. Returns what was stored.
    ///
    /// Folded on the way in, so what the Settings page saves is what a picker will offer and what a
    /// song could actually be filed under — a box somebody typed `Classic Rock` into stores
    /// `classic-rock`, and the page shows that back.
    ///
    /// Best-effort like every write here: a settings file that could not be written leaves the run
    /// working with the list it was given rather than refusing to save anything at all.
    pub fn set_default_tags(&mut self, raw: &str) -> Vec<String> {
        self.default_tags = km_kmpkg::tag::parse_list(raw)
            .into_iter()
            .map(km_kmpkg::Tag::into_string)
            .collect();
        if let Some(file) = path() {
            self.write(&file);
        }
        self.default_tags.clone()
    }

    /// The language chosen here, or `None` where the browser is still being followed.
    ///
    /// A tag no catalog answers to reads as `None` rather than as an error: this file is
    /// hand-editable, and a curation tool that will not start because a preferences file says `de`
    /// is the worse failure by a distance.
    pub fn locale(&self) -> Option<km_locale::Locale> {
        self.locale
            .as_deref()
            .and_then(km_locale::Locale::best_match)
    }

    /// Records which language the pages are drawn in and writes it back.
    ///
    /// Best-effort like every write here: a settings file that could not be written leaves this run
    /// speaking the language just chosen rather than refusing the choice.
    pub fn set_locale(&mut self, locale: km_locale::Locale) {
        self.locale = Some(locale.tag().to_owned());
        if let Some(file) = path() {
            self.write(&file);
        }
    }

    /// Where the settings file is, for a page that has to tell somebody.
    ///
    /// `None` when there is nowhere — a test run, or `KM_PACKAGE_BUILDER_SETTINGS` set and empty.
    pub fn file() -> Option<PathBuf> {
        path()
    }

    /// Writes the settings out, best-effort — the same discipline `recent::save` keeps.
    fn write(&self, file: &Path) {
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
            Err(error) => tracing::debug!("could not serialize the settings: {error}"),
        }
    }

    /// The suggested tags, folded to slugs and de-duplicated.
    ///
    /// Folded here rather than trusted, so a hand-edited `Classic Rock` offers `classic-rock` — the
    /// word a song could actually be filed under — instead of one nothing will ever match.
    pub fn suggested_tags(&self) -> Vec<String> {
        km_kmpkg::tag::parse_list(&self.default_tags.join(","))
            .into_iter()
            .map(km_kmpkg::Tag::into_string)
            .collect()
    }

    /// Every tag offerable: what the corpus holds first, then the suggestions it does not.
    ///
    /// **The corpus first, and that ordering is the message.** A word a song is actually filed under
    /// is a fact; a suggestion is advice. Sorted within each half, so a picker is still findable.
    pub fn offerable_tags(&self, present: &[String]) -> Vec<String> {
        let mut offerable = present.to_vec();
        for tag in self.suggested_tags() {
            if !offerable.contains(&tag) {
                offerable.push(tag);
            }
        }
        offerable
    }

    /// Of the offerable tags, the ones no song here carries — the hints.
    pub fn hints(&self, present: &[String]) -> Vec<String> {
        self.suggested_tags()
            .into_iter()
            .filter(|tag| !present.contains(tag))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-package-builder-settings-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A test run writes nowhere. The guard that keeps a suite out of the builder's config
    /// directory, and the reason it matters more here than in `recent`: this file is *created*.
    #[test]
    fn a_test_run_has_nowhere_to_keep_settings() {
        assert_eq!(path(), None);
    }

    #[test]
    fn set_and_empty_means_the_defaults_and_no_file() {
        assert_eq!(file_for(Some(std::ffi::OsStr::new(""))), None);
        assert_eq!(Settings::at(None).default_tags, DEFAULT_TAGS);
    }

    /// A first run writes the list out, so somebody can find it and change it.
    #[test]
    fn a_missing_file_is_seeded_with_the_defaults() {
        let scratch = Scratch::new("seed");
        let file = scratch.0.join("settings.json");
        assert!(!file.exists());

        let settings = Settings::at(Some(file.clone()));
        assert_eq!(settings.default_tags, DEFAULT_TAGS);
        assert!(file.exists(), "a first run leaves something to edit");

        // And what it wrote reads back as what it is.
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.contains("classic-rock"), "{text}");
        assert_eq!(Settings::at(Some(file)).default_tags, DEFAULT_TAGS);
    }

    /// An edited list is honoured, and folded — a hand-typed `Classic Rock` offers `classic-rock`.
    #[test]
    fn a_hand_edited_list_is_read_and_folded() {
        let scratch = Scratch::new("edited");
        let file = scratch.0.join("settings.json");
        std::fs::write(
            &file,
            r#"{"default_tags": ["Sertanejo", "Classic Rock", "Forró"]}"#,
        )
        .expect("write");

        let settings = Settings::at(Some(file));
        assert_eq!(
            settings.suggested_tags(),
            ["classic-rock", "forro", "sertanejo"],
            "folded to what a song could actually be filed under, and sorted"
        );
    }

    /// A corrupt file leaves the defaults **and is not overwritten**.
    #[test]
    fn an_unreadable_file_is_left_alone() {
        let scratch = Scratch::new("corrupt");
        let file = scratch.0.join("settings.json");
        std::fs::write(&file, "{ this is not json").expect("write");

        assert_eq!(Settings::at(Some(file.clone())).default_tags, DEFAULT_TAGS);
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "{ this is not json",
            "somebody's half-finished edit is worth more than the defaults"
        );
    }

    /// The corpus's own tags come first and a suggestion already in use stops being a hint.
    #[test]
    fn what_the_corpus_holds_comes_before_what_is_merely_suggested() {
        let settings = Settings::default();
        let present = vec!["anime".to_owned(), "rock".to_owned()];

        assert_eq!(
            settings.offerable_tags(&present),
            ["anime", "rock", "classic-rock", "pop"],
            "facts first, then advice"
        );
        assert_eq!(
            settings.hints(&present),
            ["classic-rock", "pop"],
            "`rock` is on a song here, so it is no longer a hint"
        );
    }

    /// A fresh install names no language, which is what lets the browser answer for it.
    #[test]
    fn a_first_run_leaves_the_language_unanswered() {
        assert_eq!(Settings::default().locale(), None);
    }

    /// The Settings page saving a tag does not take the logging section with it.
    ///
    /// **The one that would have been silent.** This file is written whole, so a section the struct
    /// did not carry would be dropped on the first edit somebody made through the page — and what
    /// they would notice is a machine that stopped writing a log, weeks later.
    #[test]
    fn a_logging_section_survives_a_write_of_the_whole_file() {
        let scratch = Scratch::new("logging");
        let file = scratch.0.join("settings.json");
        std::fs::write(
            &file,
            r#"{"default_tags":["pop"],"logging":{"level":"debug","file":true,"keep":"all"}}"#,
        )
        .expect("write");

        let mut settings = Settings::at(Some(file.clone()));
        assert_eq!(settings.logging.level(), Some("debug"));
        assert_eq!(settings.logging.keep(), Some(km_logfile::KEEP_ALL));

        settings.locale = Some("pt-BR".to_owned());
        settings.write(&file);

        let read = Settings::at(Some(file));
        assert_eq!(
            read.logging, settings.logging,
            "the section came back whole"
        );
        assert_eq!(read.locale.as_deref(), Some("pt-BR"));
    }

    /// A settings file written before the section existed reads as no opinion at all.
    #[test]
    fn a_file_with_no_logging_section_says_nothing_about_logging() {
        let settings: Settings =
            serde_json::from_str(r#"{"default_tags":["pop"]}"#).expect("parse");
        assert_eq!(settings.logging, km_logsettings::LoggingSettings::default());
        assert_eq!(settings.logging.level(), None);
    }

    /// A hand-edited tag is read for what it means, and an unknown one is not an error.
    #[test]
    fn a_hand_edited_tag_reads_as_the_nearest_language_this_build_has() {
        let read = |tag: &str| {
            Settings {
                locale: Some(tag.to_owned()),
                ..Settings::default()
            }
            .locale()
        };
        assert_eq!(read("pt-BR"), Some(km_locale::Locale::BrazilianPortuguese));
        // Case is what somebody types, and European Portuguese is served far better by Brazilian
        // Portuguese than by English.
        assert_eq!(read("PT-br"), Some(km_locale::Locale::BrazilianPortuguese));
        assert_eq!(read("pt-PT"), Some(km_locale::Locale::BrazilianPortuguese));
        // And a language with no catalog reads as nothing at all, so the browser still answers —
        // rather than refusing to start a curation tool over a preferences file.
        assert_eq!(read("de"), None);
        assert_eq!(read("nonsense"), None);
    }

    /// An empty list is a legitimate setting: no suggestions at all.
    #[test]
    fn an_empty_list_offers_nothing_extra() {
        let settings = Settings {
            default_tags: Vec::new(),
            ..Settings::default()
        };
        assert!(settings.suggested_tags().is_empty());
        assert_eq!(settings.offerable_tags(&["rock".to_owned()]), ["rock"]);
    }
}
