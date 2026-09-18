//! The provider keys, and where one that was typed into a browser lives.
//!
//! **`km-wallpaper-pack`'s rule is that a key comes from the environment and never from a file**, and
//! the reason is exact: its `config.toml` is a document somebody edits and shares, so a key-shaped
//! entry in one is a hard error and a config file is always safe to commit. That rule cannot be kept
//! here unchanged — a person typing a key into a form has no environment to put it in, and
//! `std::env::set_var` is `unsafe` in edition 2024 against a workspace that denies `unsafe`.
//!
//! So it becomes: **in memory by default, and in this program's own data folder only if asked**.
//! What survives from the original is the part that mattered — a key never goes near a document
//! anybody would send somebody else. `settings.json` beside it holds queries and thresholds and
//! deliberately never a key, so the two cannot merge by accident.
//!
//! The environment still works and is still first: `Keys::from_env()` is consulted at startup, so
//! somebody who already has `PIXABAY_API_KEY` set does not have to type it in.
//!
//! ## What 0600 does and does not do
//!
//! On unix the file is written with mode 0600, so other users on the machine cannot read it. **On
//! Windows there is no one-line equivalent** and this does not attempt one, so the file is protected
//! by the ordinary permissions of a user's profile directory and nothing more. The page says so
//! beside the checkbox rather than implying a protection that is not there.

use std::path::{Path, PathBuf};

use km_wallpaper_pack::config::ProviderKind;
use km_wallpaper_pack::providers::Keys;
use serde::{Deserialize, Serialize};

/// What the remembered keys are written to.
///
/// **Its own file, never `settings.json`.** Two files rather than one so that "remember my key" and
/// "remember my search terms" can never become one decision by accident, and so that deleting the
/// first is a whole answer to "forget my key".
const FILE: &str = "provider-keys.json";

/// The keys as they are written down.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(skip_serializing_if = "Option::is_none")]
    pixabay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pexels: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openverse: Option<String>,
}

/// Where the remembered keys live.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// The keys this run starts with: the environment's, then whatever was remembered.
///
/// **Remembered beats the environment**, because it is the more recent statement of intent: somebody
/// who typed a key into this program and ticked the box did so after whatever their shell profile
/// says, and the box is the thing they can see.
pub fn load(data_dir: &Path) -> Keys {
    let mut keys = Keys::from_env();
    let Ok(text) = std::fs::read_to_string(path(data_dir)) else {
        return keys;
    };
    let Ok(stored) = serde_json::from_str::<Stored>(&text) else {
        tracing::warn!(
            path = %path(data_dir).display(),
            "the remembered keys could not be read; using the environment only"
        );
        return keys;
    };
    if stored.pixabay.is_some() {
        keys.pixabay = stored.pixabay;
    }
    if stored.pexels.is_some() {
        keys.pexels = stored.pexels;
    }
    if stored.openverse.is_some() {
        keys.openverse = stored.openverse;
    }
    keys
}

/// Writes the keys down.
pub fn remember(data_dir: &Path, keys: &Keys) -> Result<(), String> {
    let stored = Stored {
        pixabay: keys.pixabay.clone(),
        pexels: keys.pexels.clone(),
        openverse: keys.openverse.clone(),
    };
    if stored.pixabay.is_none() && stored.pexels.is_none() && stored.openverse.is_none() {
        // Nothing to remember is the same as being asked to forget.
        return forget(data_dir);
    }

    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("{} cannot be written to: {error}", data_dir.display()))?;
    let text = serde_json::to_string_pretty(&stored)
        .map_err(|error| format!("could not write the keys down: {error}"))?;
    let file = path(data_dir);
    std::fs::write(&file, text)
        .map_err(|error| format!("{} cannot be written: {error}", file.display()))?;
    restrict(&file);
    Ok(())
}

/// Deletes the remembered keys.
///
/// A file that was not there is not an error: "forget my key" is satisfied either way.
pub fn forget(data_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(path(data_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not forget the keys: {error}")),
    }
}

/// Makes the file readable only by its owner, where the platform has a way to say so.
///
/// **A warning rather than a failure if it cannot**, and the page says plainly that Windows has no
/// equivalent — a program that refused to remember a key because it could not promise 0600 would be
/// trading a real convenience for a protection it was not going to provide anyway.
#[cfg(unix)]
fn restrict(file: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Err(error) = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(%error, path = %file.display(), "could not restrict the key file");
    }
}

/// Windows has no one-line equivalent, so this does nothing and says nothing.
#[cfg(not(unix))]
fn restrict(_file: &Path) {}

/// Whether this platform can restrict the file to its owner.
///
/// Read by the page, so that the sentence beside the checkbox is true on the machine it is being
/// read on rather than true in general.
pub const fn can_restrict() -> bool {
    cfg!(unix)
}

/// Every provider, for a page that lists them.
///
/// **Openverse first, and that is the product decision rather than an ordering.** It needs no
/// account, and it is the source whose packs may be passed on; the other two are reachable only
/// under somebody's own agreement with those sites.
pub const ALL: [ProviderKind; 3] = [
    ProviderKind::Openverse,
    ProviderKind::Pixabay,
    ProviderKind::Pexels,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn nothing_remembered_leaves_the_environment_alone() {
        // A data directory with no key file must not clear a key the environment supplied.
        let dir = scratch();
        let keys = load(dir.path());
        // Whatever the environment says is what comes back; the point is that reading an absent
        // file is not an error and does not blank anything.
        assert_eq!(keys.pixabay, Keys::from_env().pixabay);
    }

    #[test]
    fn a_remembered_key_comes_back_and_can_be_forgotten() {
        let dir = scratch();
        let keys = Keys {
            pexels: Some("typed-into-the-page".to_owned()),
            ..Default::default()
        };
        remember(dir.path(), &keys).expect("remember");

        assert!(path(dir.path()).is_file());
        let read = load(dir.path());
        assert_eq!(read.pexels.as_deref(), Some("typed-into-the-page"));

        forget(dir.path()).expect("forget");
        assert!(!path(dir.path()).is_file());
        assert!(load(dir.path()).pexels.is_none() || Keys::from_env().pexels.is_some());
    }

    #[test]
    fn remembering_nothing_is_the_same_as_forgetting() {
        // Clearing every field in the page and saving must not leave a file behind holding `{}`,
        // which would read as "a key is remembered" to anybody who found it.
        let dir = scratch();
        let keys = Keys {
            pixabay: Some("x".to_owned()),
            ..Default::default()
        };
        remember(dir.path(), &keys).expect("remember");
        assert!(path(dir.path()).is_file());

        remember(dir.path(), &Keys::default()).expect("remember nothing");
        assert!(
            !path(dir.path()).is_file(),
            "the file goes rather than emptying"
        );
    }

    #[test]
    fn a_corrupt_key_file_is_a_warning_and_not_a_dead_program() {
        let dir = scratch();
        std::fs::write(path(dir.path()), "this is not json").expect("write");
        // Falls back to the environment rather than refusing to start.
        let keys = load(dir.path());
        assert_eq!(keys.pexels, Keys::from_env().pexels);
    }

    #[test]
    fn forgetting_a_file_that_was_never_there_is_not_an_error() {
        let dir = scratch();
        forget(dir.path()).expect("nothing to forget is still success");
    }

    #[test]
    fn the_key_file_is_never_the_settings_file() {
        // Two files, so that "forget my key" cannot take somebody's search terms with it and
        // "remember my terms" cannot quietly write a credential.
        let dir = scratch();
        assert_ne!(path(dir.path()), dir.path().join("settings.json"));
        assert!(path(dir.path()).ends_with(FILE));
    }

    #[test]
    fn openverse_is_offered_first_because_it_needs_no_account() {
        assert_eq!(ALL[0], ProviderKind::Openverse);
        assert!(!ALL[0].needs_key());
    }
}
