//! Listing folders, for the Open page.
//!
//! The listing itself is `km-folders`, which `km-package-simple` draws from too. What this module
//! adds is the one thing only this tool can say about a folder: whether it holds a `.kmbuild`
//! database already. That badge is what a native dialog could not show, and it is half of why the
//! picker is drawn inside the page. See `How a corpus is opened` in `docs/decisions/curation.md`.

use std::path::{Path, PathBuf};

pub use km_folders::{Ask, PAGE};

/// What a folder holds, as far as this tool cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Indexed {
    /// A `.kmbuild` database is here.
    Yes,
    /// Nothing has looked at this folder.
    #[default]
    No,
}

impl Indexed {
    /// Whether this folder can be opened as it stands.
    ///
    /// A method rather than an `==` in the template: askama cannot name a Rust path, so comparing
    /// against the variants would mean a stringly-typed comparison on every row. A predicate reads
    /// better in the markup and keeps the enum the single source of truth.
    pub fn openable(self) -> bool {
        matches!(self, Self::Yes)
    }
}

/// One row in the listing.
#[derive(Debug, Clone)]
pub struct Row {
    /// What to show.
    pub name: String,
    /// Where it goes.
    pub path: String,
    /// Whether it has been curated before.
    pub indexed: Indexed,
}

/// A directory listing, plus the trail above it. The fields are [`km_folders::Listing`]'s, with a
/// badge on each row and on the folder itself.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Where we are. `None` means the list of drives, which only Windows has.
    pub here: Option<String>,
    /// The folder above, if there is one.
    pub parent: Option<String>,
    /// Sub-folders, sorted by name: one page of them. See [`PAGE`].
    pub rows: Vec<Row>,
    /// Whether this folder is itself openable.
    pub indexed: Indexed,
    /// Why the listing is short, when it is.
    pub error: Option<String>,
    /// What is being narrowed by, as it was typed.
    pub filter: String,
    /// How many folders match, of which [`Self::rows`] is a page.
    pub total: usize,
    /// Where this page starts, counting from zero.
    pub offset: usize,
    /// The query string for the page before, empty when this is the first.
    pub previous: String,
    /// The query string for the page after, empty when this is the last.
    pub next: String,
    /// Which page of folders this is, out of how many, as the pager says it. Set by [`Self::say_range`].
    pub range: String,
}

impl Listing {
    /// Which page of folders this is, out of how many, as the pager says it. See
    /// [`crate::views::say_page`].
    pub fn say_range(&mut self, locale: km_locale::Locale) {
        let n = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        self.range = crate::views::say_page(
            locale,
            "folders-range",
            n(self.offset),
            n(self.total),
            n(PAGE),
        );
    }
}

/// Whether a folder has been curated.
pub fn indexed(root: &Path) -> Indexed {
    if crate::db::database_in(root).is_some() {
        Indexed::Yes
    } else {
        Indexed::No
    }
}

/// Lists one page of what is under `ask.here`, or the drives when it is `None`.
///
/// **Take the page, and only then ask which folders are indexed.** [`indexed`] opens and reads the
/// folder it is asked about, so probing every row costs one directory enumeration per subdirectory
/// whether or not anybody sees it. Measured on the system temp folder, whose three thousand
/// subdirectories took about a minute of that. A page of a hundred costs a hundred, and the count
/// beside it costs none.
pub fn list(ask: &Ask) -> Listing {
    let listing = km_folders::list(ask);
    Listing {
        indexed: ask.here.as_deref().map(indexed).unwrap_or_default(),
        rows: listing
            .rows
            .into_iter()
            .map(|folder| Row {
                indexed: indexed(Path::new(&folder.path)),
                name: folder.name,
                path: folder.path,
            })
            .collect(),
        here: listing.here,
        parent: listing.parent,
        error: listing.error,
        filter: listing.filter,
        total: listing.total,
        offset: listing.offset,
        previous: listing.previous,
        next: listing.next,
        range: String::new(),
    }
}

/// Where the picker should start when nothing else says.
pub fn start() -> Option<PathBuf> {
    km_folders::start()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Scratch;

    /// Asks for one folder, all of it, unnarrowed.
    fn all_of(here: &Path) -> Listing {
        list(&Ask {
            here: Some(here.to_path_buf()),
            ..Ask::default()
        })
    }

    /// A folder holding `count` numbered subdirectories, named so that they sort as they are made.
    fn folders(name: &str, count: usize) -> Scratch {
        let scratch = Scratch::new(name);
        for number in 0..count {
            std::fs::create_dir_all(scratch.join(format!("folder-{number:04}")))
                .expect("making a test folder");
        }
        scratch
    }

    #[test]
    fn a_folder_with_no_database_says_so() {
        let scratch = Scratch::new("browse-unindexed");
        assert_eq!(indexed(scratch.path()), Indexed::No);
    }

    /// The pager says which page this is and how many folders there are.
    #[test]
    fn the_pager_says_the_page_and_the_count() {
        let scratch = folders("browse-paged", PAGE + 30);

        let mut first = all_of(scratch.path());
        first.say_range(km_locale::Locale::English);
        assert_eq!(first.range, "page 1 of 2 (130 folders)");

        let mut past_the_end = list(&Ask {
            here: Some(scratch.to_path_buf()),
            offset: PAGE * 40,
            ..Ask::default()
        });
        past_the_end.say_range(km_locale::Locale::English);
        assert_eq!(past_the_end.range, "page 2 of 2 (130 folders)");
    }

    /// **Only the folders on the page are asked whether they are indexed**, which is the whole
    /// reason the page exists: the question costs a directory read each.
    ///
    /// Observable without a clock. The corpus sorts onto the second page, so a first page that
    /// named it as indexed could only have got that by probing a row it was not going to draw.
    #[test]
    fn only_the_folders_on_the_page_are_asked_whether_they_are_indexed() {
        let scratch = folders("browse-probe", PAGE + 1);
        let corpus = scratch.join(format!("folder-{PAGE:04}"));
        std::fs::write(corpus.join("corpus.kmbuild"), b"not really a database")
            .expect("writing a database");

        let first = all_of(scratch.path());
        assert!(
            first.rows.iter().all(|row| row.indexed == Indexed::No),
            "a row off the page was probed"
        );

        // ...and it is found on the page it is actually on, so the probe still happens.
        let second = list(&Ask {
            here: Some(scratch.to_path_buf()),
            offset: PAGE,
            ..Ask::default()
        });
        assert_eq!(second.rows[0].indexed, Indexed::Yes);
    }
}
