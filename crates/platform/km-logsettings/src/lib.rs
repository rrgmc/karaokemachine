//! The `logging` section of a program's settings file.
//!
//! Three programs here keep a settings file and can be started without a command line: the machine,
//! the package builder and the picture-and-bank tool. A flag reaches the run somebody types and an
//! environment variable reaches the unit somebody wrote; neither reaches the box a person walks up
//! to and starts by double-clicking its icon, which is the machine that most needs a log and the one
//! nobody can pass an argument to. So each of them reads this section, and it is the same section in
//! all three.
//!
//! ```json
//! {
//!   "logging": {
//!     "level": "info,km_api=debug",
//!     "file": true,
//!     "keep": "all",
//!     "ecapplog": true
//!   }
//! }
//! ```
//!
//! # A crate rather than three copies
//!
//! The `-v` ladder is duplicated across the programs that offer one, and `docs/ARCHITECTURE.md` says
//! why: ten lines of `match` in three crates is cheaper than a workspace member existing to hold
//! them. The axis it names is not size but **how badly a divergence would hurt**, and this falls the
//! other side of it, for [`km_logfile`]'s reason read one key further. Four keys, two of which take
//! either of two shapes, each answering a bad value with no opinion rather than with a refusal, is a
//! grammar — and three copies of a grammar is three answers to what `logging.level` accepts.
//!
//! What stays with each program is the ladder itself, because the rungs differ: every one of them
//! names its own crate and its own chatty dependencies.
//!
//! # Nothing here refuses to start
//!
//! Every accessor answers `None` for a value it cannot read, and [`LoggingSettings::warn_about_unread`]
//! is what says so out loud once there is a subscriber to say it to. **A value nobody can read is
//! worse than a wrong one**: the program goes on doing what it did while the person who set it
//! believes otherwise, and they find that out on the evening they go looking for the run that broke.
//!
//! [`peek`] is the read that happens before any of that, and it is silent even about a file that
//! will not parse — there is nowhere to say it yet, and the program's own settings loader is a
//! moment behind it to report the fault properly.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// What a settings file says about this program's log.
///
/// **Persisted, unlike the rest of the diagnostics, because a machine being worked on is a standing
/// state rather than an evening.** See the `A log file for the runs nobody is watching` decision.
///
/// **Not under `debug`**, deliberately: the machine's `debug.enabled` publishes a passwordless copy
/// of the API, and asking a program to write down what it did must not be a way to arrive at that.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingSettings {
    /// How much detail, in the grammar `RUST_LOG` takes.
    ///
    /// `"debug"` is the whole program; `"info,km_api=debug"` is one part of it. It replaces the
    /// `-v` ladder rather than moving along it, which is what lets it say the thing the ladder has
    /// no rung for — and it is the only shape that could, since the ladder's rungs are a different
    /// set of names in every program.
    ///
    /// Absent means the ladder decides, which is what a program with no such key does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Write every run's log to a file, with nothing on the command line saying so.
    pub file: bool,
    /// How many files to keep: a number, or `all`.
    ///
    /// Absent means the built-in ten. Counted separately for runs and for crash reports, so an
    /// evening of restarts cannot delete the report of the panic that ended one of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep: Option<KeepSetting>,
    /// Send every run's log to the ECAppLog viewer, with nothing on the command line saying so.
    ///
    /// `true` for the viewer on this machine, an address for one anywhere else. Absent means no.
    /// The console is what this replaces, which is why it sits beside [`file`] rather than under a
    /// debugging switch: it changes where a run talks, not what it will answer.
    ///
    /// [`file`]: LoggingSettings::file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ecapplog: Option<ViewerSetting>,
}

/// What `logging.keep` may say.
///
/// **Two shapes rather than one, because both are the obvious thing to type.** `"all"` is a word and
/// `200` is a number, and a configuration file that accepted only one of them would be answering a
/// question about serialization with an error message about JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeepSetting {
    /// `"keep": 200`
    Count(usize),
    /// `"keep": "all"`
    Named(String),
}

/// What `logging.ecapplog` may say.
///
/// **Two shapes rather than one**, for [`KeepSetting`]'s reason: `true` is what somebody types to
/// mean *the viewer on this machine* and an address is what they type to name another, and a file
/// accepting only the second would make the common case the wordy one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViewerSetting {
    /// `"ecapplog": true`
    On(bool),
    /// `"ecapplog": "192.168.1.5:13991"`
    At(String),
}

impl LoggingSettings {
    /// The filter this file asks for, or `None` for nobody has said.
    ///
    /// A directive that will not parse answers `None` rather than guessing, exactly as a bad `keep`
    /// does. Guessing is the worse failure here: a typo that quietly became `info` is a machine
    /// somebody believes they turned up.
    #[must_use]
    pub fn level(&self) -> Option<&str> {
        let level = self.level.as_deref()?;
        parse_level(level).is_ok().then_some(level)
    }

    /// How many files this program keeps, or `None` for the built-in default.
    ///
    /// A word that is not `all` answers `None` rather than guessing; [`Self::warn_about_unread`] is
    /// what says so out loud, once there is a subscriber to say it to.
    #[must_use]
    pub fn keep(&self) -> Option<usize> {
        match self.keep.as_ref()? {
            KeepSetting::Count(count) => Some(*count),
            KeepSetting::Named(word) => km_logfile::parse_keep(word).ok(),
        }
    }

    /// Where this program sends its log, or `None` for nowhere.
    ///
    /// An address that will not parse answers `None` rather than guessing, exactly as a bad `keep`
    /// does. `false` is a written-down no, which is what lets somebody turn this off without
    /// deleting the line.
    #[must_use]
    pub fn ecapplog(&self) -> Option<String> {
        match self.ecapplog.as_ref()? {
            ViewerSetting::On(true) => Some(km_ecapplog::DEFAULT_ADDRESS.to_owned()),
            ViewerSetting::On(false) => None,
            ViewerSetting::At(address) => km_ecapplog::parse_address(address).ok(),
        }
    }

    /// Says out loud what this section asked for and could not be read.
    ///
    /// Called once the program has a subscriber, which is necessarily after [`peek`] has already
    /// decided what that subscriber is: a level nobody can read is reported through the log it
    /// failed to configure, which is the only place there is to report it.
    pub fn warn_about_unread(&self) {
        if let Some(level) = &self.level
            && let Err(why) = parse_level(level)
        {
            tracing::warn!(%why, "logging.level was not understood; this run keeps its usual level");
        }
        if let Some(KeepSetting::Named(word)) = &self.keep
            && let Err(why) = km_logfile::parse_keep(word)
        {
            tracing::warn!(%why, "logging.keep was not understood; keeping the usual number");
        }
        if let Some(ViewerSetting::At(address)) = &self.ecapplog
            && let Err(why) = km_ecapplog::parse_address(address)
        {
            tracing::warn!(%why, "logging.ecapplog was not understood; this run's log goes to the console");
        }
    }
}

/// Reads a filter directive.
///
/// Shared with every caller so that a settings key and the `RUST_LOG` beside it cannot disagree
/// about what they accept, and so the refusal is written once.
///
/// # Errors
///
/// If the directive is not one `RUST_LOG` would take.
pub fn parse_level(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("expected a level such as `debug`, or a filter such as `info,km_api=debug`, not an empty value".to_owned());
    }
    tracing_subscriber::EnvFilter::builder()
        .parse(value)
        .map(drop)
        .map_err(|error| format!("`{value}` is not a filter RUST_LOG would take: {error}"))
}

/// What a settings file says about logging, without writing one or reporting on one.
///
/// **A narrow read rather than the program's own settings loader, and both halves of that are
/// load-bearing.** The subscriber has to exist before anything can be said, so this runs before the
/// settings are loaded — and a loader that seeds a missing file would create one for a program that
/// has never run.
///
/// **Silent about a file it cannot read**, because there is nowhere to say it yet and the program's
/// loader is a moment away from saying it properly. A program whose settings will not parse starts
/// with no log file rather than no log file *and* a duplicate complaint.
#[must_use]
pub fn peek(file: impl AsRef<Path>) -> LoggingSettings {
    /// Everything else in the file, ignored. The program's own root type would do, and would drag
    /// every other field's default and migration into a question about one section.
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct JustLogging {
        logging: LoggingSettings,
    }

    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<JustLogging>(&text).ok())
        .unwrap_or_default()
        .logging
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_a_rust_log_directive() {
        let plain = LoggingSettings {
            level: Some("debug".to_owned()),
            ..LoggingSettings::default()
        };
        assert_eq!(plain.level(), Some("debug"));

        let targeted = LoggingSettings {
            level: Some("info,km_api=trace".to_owned()),
            ..LoggingSettings::default()
        };
        assert_eq!(targeted.level(), Some("info,km_api=trace"));

        assert_eq!(LoggingSettings::default().level(), None);
    }

    #[test]
    fn a_level_nobody_can_read_is_no_opinion_at_all() {
        let nonsense = LoggingSettings {
            level: Some("=".to_owned()),
            ..LoggingSettings::default()
        };
        assert_eq!(nonsense.level(), None);

        let empty = LoggingSettings {
            level: Some("   ".to_owned()),
            ..LoggingSettings::default()
        };
        assert_eq!(empty.level(), None);
    }

    #[test]
    fn how_many_to_keep_is_a_number_or_the_word_all() {
        let counted = LoggingSettings {
            keep: Some(KeepSetting::Count(200)),
            ..LoggingSettings::default()
        };
        assert_eq!(counted.keep(), Some(200));

        let named = LoggingSettings {
            keep: Some(KeepSetting::Named("all".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(named.keep(), Some(km_logfile::KEEP_ALL));

        let nonsense = LoggingSettings {
            keep: Some(KeepSetting::Named("lots".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(nonsense.keep(), None);

        assert_eq!(LoggingSettings::default().keep(), None);
    }

    #[test]
    fn the_viewer_is_a_switch_or_an_address() {
        let on = LoggingSettings {
            ecapplog: Some(ViewerSetting::On(true)),
            ..LoggingSettings::default()
        };
        assert_eq!(on.ecapplog().as_deref(), Some(km_ecapplog::DEFAULT_ADDRESS));

        let elsewhere = LoggingSettings {
            ecapplog: Some(ViewerSetting::At("192.168.1.5:13991".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(elsewhere.ecapplog().as_deref(), Some("192.168.1.5:13991"));

        let off = LoggingSettings {
            ecapplog: Some(ViewerSetting::On(false)),
            ..LoggingSettings::default()
        };
        assert_eq!(off.ecapplog(), None);

        let nonsense = LoggingSettings {
            ecapplog: Some(ViewerSetting::At("nope".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(nonsense.ecapplog(), None);

        assert_eq!(LoggingSettings::default().ecapplog(), None);
    }

    #[test]
    fn each_key_takes_either_shape_it_is_written_in() {
        let level: LoggingSettings =
            serde_json::from_str(r#"{"level":"warn"}"#).expect("a level parses");
        assert_eq!(level.level.as_deref(), Some("warn"));

        let counted: LoggingSettings =
            serde_json::from_str(r#"{"keep":200}"#).expect("a count parses");
        assert_eq!(counted.keep, Some(KeepSetting::Count(200)));

        let named: LoggingSettings =
            serde_json::from_str(r#"{"keep":"all"}"#).expect("a word parses");
        assert_eq!(named.keep, Some(KeepSetting::Named("all".to_owned())));

        let switch: LoggingSettings =
            serde_json::from_str(r#"{"ecapplog":true}"#).expect("a switch parses");
        assert_eq!(switch.ecapplog, Some(ViewerSetting::On(true)));

        let address: LoggingSettings =
            serde_json::from_str(r#"{"ecapplog":"192.168.1.5:13991"}"#).expect("an address parses");
        assert_eq!(
            address.ecapplog,
            Some(ViewerSetting::At("192.168.1.5:13991".to_owned()))
        );
    }

    #[test]
    fn a_section_with_nothing_set_is_written_as_one_key() {
        let written =
            serde_json::to_string(&LoggingSettings::default()).expect("the defaults serialize");
        assert_eq!(written, r#"{"file":false}"#);
    }

    #[test]
    fn an_absent_section_reads_as_no_opinion_at_all() {
        let none: LoggingSettings = serde_json::from_str("{}").expect("an empty object parses");
        assert_eq!(none, LoggingSettings::default());
    }

    #[test]
    fn peeking_reads_the_section_out_of_a_whole_settings_file() {
        let dir = std::env::temp_dir().join(format!(
            "km-logsettings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let file = dir.join("settings.json");

        std::fs::write(
            &file,
            r#"{"machine":{"name":"upstairs"},"logging":{"level":"debug","file":true}}"#,
        )
        .expect("a settings file");
        let found = peek(&file);
        assert_eq!(found.level(), Some("debug"));
        assert!(found.file);

        std::fs::write(&file, "{ not json at all").expect("a broken settings file");
        assert_eq!(
            peek(&file),
            LoggingSettings::default(),
            "a file that will not parse peeks as no opinion at all"
        );

        std::fs::remove_dir_all(&dir).expect("the scratch directory goes");
    }

    #[test]
    fn peeking_a_file_that_is_not_there_writes_nothing() {
        let file = std::env::temp_dir().join("km-logsettings-no-such-file.json");
        assert_eq!(peek(&file), LoggingSettings::default());
        assert!(!file.exists(), "peeking made a settings.json");
    }
}
