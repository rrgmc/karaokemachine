//! A pack made from images somebody chose, rather than from a search.
//!
//! The three-phase pipeline — `fetch`, `analyze`, `build` — exists to get a hundred usable
//! photographs out of thousands nobody will ever look at. That is the right shape for a pack, and
//! the wrong shape for the six or eight images that **ship with the machine**: those get seen every
//! night, they are chosen by eye, and their provenance is typed by the person who chose them rather
//! than parsed out of an API response.
//!
//! What they must not skip is the measurement. "Looks calm enough" is precisely the judgment the
//! contrast gate exists to replace, and a shipped wallpaper that fails it fails on every machine.
//! So this runs the same [`crate::metrics::measure`] against the same [`crate::config::Legibility`]
//! and writes the same [`crate::manifest::Manifest`] — which is also what lets `verify` re-check a
//! shipped set exactly as it re-checks a pack.
//!
//! # The sidecar
//!
//! Provenance cannot be inferred from a JPEG — `process::render` strips metadata, and a CC0 image's
//! photographer is not in the file to begin with. It is written by hand, in a TOML file beside the
//! pictures:
//!
//! ```toml
//! [[image]]
//! file        = "lake-at-dusk.jpg"
//! author      = "Ada Lovelace"
//! author_url  = "https://example.test/@ada"          # optional
//! source_url  = "https://example.test/photos/12345"
//! license     = "CC0 1.0"
//! license_url = "https://creativecommons.org/publicdomain/zero/1.0/"
//! title       = "A lake at dusk"                     # optional
//! ```
//!
//! **Every image in the folder must appear in it**, and an entry naming a file that is not there is
//! an error rather than a warning. A shipped image whose license nobody wrote down is the failure
//! this whole exercise is about, and a folder that quietly indexes as fewer images than it holds is
//! the shape of it.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::license::License;

/// The sidecar file's name, when one is not given.
pub const CREDITS_FILE: &str = "credits.toml";

/// The whole sidecar.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credits {
    /// One per image in the folder.
    #[serde(default, rename = "image")]
    pub images: Vec<Credit>,
}

/// What is known about one curated image, all of it typed by a person.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credit {
    /// The file's name inside the source folder.
    pub file: String,
    /// Who took it.
    pub author: String,
    /// Their page, when there is one.
    #[serde(default)]
    pub author_url: Option<String>,
    /// The page a person can visit to find this image and check its license.
    pub source_url: String,
    /// The license, named as a person would say it: `CC0 1.0`, `CC BY 4.0`.
    pub license: String,
    /// Where that license's text is.
    #[serde(default)]
    pub license_url: Option<String>,
    /// The source's own title, when it has one. Recorded, not rendered.
    #[serde(default)]
    pub title: Option<String>,
    /// The machine-readable license code, when the name is not one [`License::code_for_name`] knows.
    ///
    /// Optional because the common names are in that table and typing a code twice is a way to
    /// disagree with yourself. State it when the table would answer `unstated` — which is what a
    /// pack refusing to call itself redistributable will be complaining about.
    #[serde(default)]
    pub license_code: Option<String>,
}

impl Credit {
    /// The license code for this image: the one stated, or the one its name implies.
    pub fn code(&self) -> String {
        self.license_code
            .clone()
            .unwrap_or_else(|| License::code_for_name(&self.license))
    }
}

impl Credits {
    /// Reads and checks a sidecar against the folder it describes.
    ///
    /// The check is the point of the function. Three ways to be wrong, and all three are errors
    /// rather than warnings, because each produces a shipped image with no license attached to it —
    /// which is the exact defect this command exists to make impossible:
    ///
    /// * an image in the folder that the sidecar does not mention;
    /// * a sidecar entry naming a file that is not in the folder;
    /// * two entries for one file, which would credit it twice and to two different people.
    pub fn read(path: &Path, files: &BTreeSet<String>) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        let credits: Self = toml::from_str(&text)
            .map_err(|error| Error::config(format!("{}: {error}", path.display())))?;

        let mut named: BTreeSet<String> = BTreeSet::new();
        for credit in &credits.images {
            if !named.insert(credit.file.clone()) {
                return Err(Error::config(format!(
                    "{}: `{}` is described twice",
                    path.display(),
                    credit.file
                )));
            }
            if credit.license.trim().is_empty() {
                return Err(Error::config(format!(
                    "{}: `{}` has no license, and an image with no license cannot ship",
                    path.display(),
                    credit.file
                )));
            }
        }

        let missing: Vec<&String> = files.difference(&named).collect();
        if !missing.is_empty() {
            return Err(Error::config(format!(
                "{}: {} image(s) in the folder are not described here, so nothing records who took \
                 them or under what license: {}",
                path.display(),
                missing.len(),
                display_list(&missing)
            )));
        }

        let absent: Vec<&String> = named.difference(files).collect();
        if !absent.is_empty() {
            return Err(Error::config(format!(
                "{}: {} described image(s) are not in the folder: {}",
                path.display(),
                absent.len(),
                display_list(&absent)
            )));
        }
        Ok(credits)
    }

    /// The entry for one file, which [`Credits::read`] has already proved is there.
    pub fn for_file(&self, file: &str) -> Option<&Credit> {
        self.images.iter().find(|credit| credit.file == file)
    }
}

/// A few names for an error message, capped so a folder of a hundred does not print a hundred.
fn display_list(names: &[&String]) -> String {
    const SHOWN: usize = 5;
    let head: Vec<&str> = names.iter().take(SHOWN).map(|name| name.as_str()).collect();
    if names.len() > SHOWN {
        format!("{}, and {} more", head.join(", "), names.len() - SHOWN)
    } else {
        head.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn write(dir: &Path, text: &str) -> std::path::PathBuf {
        let path = dir.join(CREDITS_FILE);
        std::fs::write(&path, text).expect("write the sidecar");
        path
    }

    /// A directory of this test's own, removed when the value goes.
    ///
    /// **Held by the caller rather than returned as a bare path**, which is the whole of the fix it
    /// carries: a path cannot clean up after itself, so every run left its directories in the temp
    /// folder and five hundred of them accumulated there. Nothing here holds a file open, so the
    /// removal cannot lose the race a database makes.
    struct Scratch(std::path::PathBuf);

    impl std::ops::Deref for Scratch {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "wp-curated-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Scratch(dir)
    }

    const ONE: &str = r#"
[[image]]
file = "lake.jpg"
author = "Ada"
source_url = "https://example.test/1"
license = "CC0 1.0"
license_url = "https://creativecommons.org/publicdomain/zero/1.0/"
"#;

    #[test]
    fn a_described_folder_reads() {
        let dir = scratch("ok");
        let path = write(&dir, ONE);
        let credits = Credits::read(&path, &folder(&["lake.jpg"])).expect("read");
        assert_eq!(credits.images.len(), 1);
        assert_eq!(credits.for_file("lake.jpg").expect("found").author, "Ada");
    }

    /// The failure this command exists to prevent: an image ships and nothing says whose it is.
    #[test]
    fn an_image_nobody_described_is_refused_and_named() {
        let dir = scratch("undescribed");
        let path = write(&dir, ONE);
        let error =
            Credits::read(&path, &folder(&["lake.jpg", "forest.jpg"])).expect_err("refused");
        assert!(error.to_string().contains("forest.jpg"), "{error}");
    }

    #[test]
    fn describing_a_file_that_is_not_there_is_refused() {
        let dir = scratch("absent");
        let path = write(&dir, ONE);
        let error = Credits::read(&path, &folder(&[])).expect_err("refused");
        assert!(error.to_string().contains("lake.jpg"), "{error}");
    }

    /// Two entries would credit one photograph to two people, and the second would silently win.
    #[test]
    fn describing_one_file_twice_is_refused() {
        let dir = scratch("twice");
        let path = write(&dir, &format!("{ONE}{ONE}"));
        let error = Credits::read(&path, &folder(&["lake.jpg"])).expect_err("refused");
        assert!(error.to_string().contains("twice"), "{error}");
    }

    #[test]
    fn an_empty_license_is_refused() {
        let dir = scratch("nolicense");
        let path = write(
            &dir,
            "[[image]]\nfile = \"lake.jpg\"\nauthor = \"Ada\"\n\
             source_url = \"https://example.test/1\"\nlicense = \"\"\n",
        );
        let error = Credits::read(&path, &folder(&["lake.jpg"])).expect_err("refused");
        assert!(error.to_string().contains("no license"), "{error}");
    }

    /// `deny_unknown_fields`, so a mistyped key is caught rather than silently dropping the value it
    /// was meant to carry — a `licence_url` that never becomes a `license_url` is a missing link in a
    /// credits file nobody re-reads.
    ///
    /// **The `licence_url` below is British on purpose and has to stay that way.** It is the typo
    /// this test exists to catch, and since the repository settled on US English it is also the
    /// likeliest one anybody will now make. A spelling sweep that "corrects" it quietly inverts the
    /// assertion into a claim that a *valid* key is refused, which is a test that passes and proves
    /// nothing.
    #[test]
    fn a_mistyped_key_is_refused() {
        let dir = scratch("mistyped");
        let path = write(
            &dir,
            "[[image]]\nfile = \"lake.jpg\"\nauthor = \"Ada\"\n\
             source_url = \"https://example.test/1\"\nlicense = \"CC0 1.0\"\n\
             licence_url = \"https://example.test/cc0\"\n",
        );
        let error = Credits::read(&path, &folder(&["lake.jpg"])).expect_err("refused");
        assert!(error.to_string().contains("licence_url"), "{error}");
    }
}
