//! What this tool remembers between runs: the language it speaks and the last folder it read.
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
    /// The folder read last, offered again on the first page.
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

/// Where the settings file lives: the platform's per-user configuration folder.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "km-package-simple")
        .map(|dirs| dirs.config_dir().join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_a_bad_file_is_a_fresh_start() {
        let dir = std::env::temp_dir().join("km-package-simple-settings");
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
