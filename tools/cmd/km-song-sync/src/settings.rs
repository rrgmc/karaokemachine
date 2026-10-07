//! What this tool remembers between runs: the language it speaks and the folder its browser was
//! on last.
//!
//! **A folder, never a file.** The rule the machine's `settings.json` follows reaches here too: a
//! folder is where somebody keeps songs, and a file path is a fact about one song.
//!
//! A file that will not read is a fresh start rather than a refusal. Nothing in it is worth more
//! than the run that could not open it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The settings file's contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The language the pages speak, as a BCP 47 tag. Absent means the browser's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// The folder the browser was on last, where the next run opens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_folder: Option<PathBuf>,
}

impl Settings {
    /// Reads the file, or the defaults where there is none or it will not read.
    #[must_use]
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                tracing::warn!(%error, file = %path.display(), "settings unreadable; starting fresh");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Writes the file. A failure is logged, because a setting that did not stick is a nuisance and
    /// not a reason to fail the request that changed it.
    pub fn save(&self, path: Option<&Path>) {
        let Some(path) = path else {
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
                std::fs::write(path, text)
            });
        if let Err(error) = written {
            tracing::warn!(%error, file = %path.display(), "could not save the settings");
        }
    }
}

/// The environment variable that says where this run's settings live.
///
/// A scripted run must be able to say that the settings are not its owner's. A picture taken of
/// the page is one, and the folder it browses would otherwise become the folder the next run
/// opens. Set and empty means remember nothing.
pub const ENV_VAR: &str = "KM_SONG_SYNC_SETTINGS";

/// Where the settings file lives: where [`ENV_VAR`] says, or the platform's per-user
/// configuration folder.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    path_for(std::env::var_os(ENV_VAR).as_deref())
}

/// The settings file for what [`ENV_VAR`] holds. `None` is a run that remembers nothing.
fn path_for(named: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    match named {
        Some(named) if named.is_empty() => None,
        Some(named) => Some(PathBuf::from(named)),
        None => km_dirs::for_app("km-song-sync").map(|dirs| dirs.config.join("settings.json")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_file_is_the_settings_file_and_an_empty_name_remembers_nothing() {
        let named = std::ffi::OsStr::new("/tunes/karaoke/settings.json");
        assert_eq!(path_for(Some(named)), Some(PathBuf::from(named)));
        assert_eq!(path_for(Some(std::ffi::OsStr::new(""))), None);
    }

    #[test]
    fn settings_round_trip_and_a_bad_file_is_a_fresh_start() {
        let dir = std::env::temp_dir().join("km-song-sync-settings");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");

        assert_eq!(Settings::load(Some(&path)), Settings::default());
        let settings = Settings {
            locale: Some("pt-BR".to_owned()),
            last_folder: Some(PathBuf::from("/tunes/karaoke")),
        };
        settings.save(Some(&path));
        assert_eq!(Settings::load(Some(&path)), settings);

        std::fs::write(&path, "not json").expect("write");
        assert_eq!(Settings::load(Some(&path)), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
