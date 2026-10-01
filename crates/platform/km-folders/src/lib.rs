//! Listing folders, for a folder picker drawn inside a web page.
//!
//! A browser cannot hand a web page a folder path. A file input gives file contents, not locations,
//! and that is a security property, not a gap. So the picking is done on the server side: the
//! server lists a directory, the page draws it, and a click asks for the next one. The tools that use
//! this are on loopback and already read and write files as whoever ran them, so listing directories
//! grants nothing they did not already have.
//!
//! Deliberately not a native dialog. That would mean a GUI toolkit on every platform for one
//! interaction, and on Linux these tools run in a browser where a dialog has no window to belong to.
//! See `How a corpus is opened` in `docs/decisions/curation.md`.

use std::path::PathBuf;

/// One folder in the listing.
///
/// **Paths are carried as `String`s**, because that is what a template can render and what a query
/// string can hold. `PathBuf` is neither `Display` nor escapable by askama, deliberately, since a
/// path is not necessarily valid Unicode. Converting once here beats converting at every point in
/// the markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// What to show.
    pub name: String,
    /// Where it goes.
    pub path: String,
}

/// A directory listing, plus the trail above it.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Where we are. `None` means the list of drives, which only Windows has.
    pub here: Option<String>,
    /// The folder above, if there is one.
    pub parent: Option<String>,
    /// Sub-folders, sorted by name: one page of them. See [`PAGE`].
    pub rows: Vec<Folder>,
    /// Why the listing is short, when it is.
    pub error: Option<String>,
    /// What is being narrowed by, as it was typed.
    pub filter: String,
    /// How many folders match, of which [`Self::rows`] is a page.
    ///
    /// **Exact, because the whole directory has been read to produce it.** The page is a slice of
    /// names already in hand, not a query that stopped early. So the count beside the pager is a
    /// fact rather than an estimate, which is what lets it be shown at all.
    pub total: usize,
    /// Where this page starts, counting from zero.
    pub offset: usize,
    /// The query string for the page before, empty when this is the first.
    pub previous: String,
    /// The query string for the page after, empty when this is the last.
    pub next: String,
}

/// What to list, and which part of it.
///
/// **A struct rather than three arguments**, because two of the three are optional and adjacent.
/// `list(Some(&path), "", 0)` is exactly the call somebody eventually writes the wrong way round.
#[derive(Debug, Clone, Default)]
pub struct Ask {
    /// Which folder. `None` means the drives.
    pub here: Option<PathBuf>,
    /// Show only folders whose name holds this, ignoring case. Empty shows all of them.
    pub filter: String,
    /// Which page, as a row index from the start of what matched.
    pub offset: usize,
}

/// How many folders a page of the picker holds.
///
/// A row is a name and at most one badge, read at a glance rather than judged. What decides the
/// number is that the listing is swapped into a panel on a page with other things on it, so a page
/// somebody has to scroll past to reach the pager is a page that hides its own way on.
pub const PAGE: usize = 100;

/// Lists one page of what is under `ask.here`, or the drives when it is `None`.
///
/// **A folder that cannot be read is a listing with a reason on it, never a failure.** Half the
/// interesting places on a real machine refuse to be listed: a system folder, someone else's
/// profile, a disconnected network share. A picker that dies on one of them cannot be used to walk
/// to the folder beside it.
///
/// **Gather, narrow, sort, and take the page.** A caller that asks something costly of each row asks
/// it of the page it gets back, so a directory of thousands costs a hundred.
pub fn list(ask: &Ask) -> Listing {
    let Some(here) = ask.here.as_deref() else {
        return drives();
    };

    let mut listing = Listing {
        here: Some(here.display().to_string()),
        parent: here.parent().map(|parent| parent.display().to_string()),
        filter: ask.filter.clone(),
        ..Default::default()
    };

    let entries = match std::fs::read_dir(here) {
        Ok(entries) => entries,
        Err(error) => {
            listing.error = Some(format!("{} could not be read: {error}", here.display()));
            return listing;
        }
    };

    let wanted = ask.filter.to_lowercase();
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        // `file_type` rather than `metadata`, so a symlink is reported as a symlink instead of being
        // followed. Following one is how a picker ends up in a cycle, or spends a minute waiting on a
        // dead network mount that something linked to years ago.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !worth_showing(&entry, &name) {
            continue;
        }
        if !wanted.is_empty() && !name.to_lowercase().contains(&wanted) {
            continue;
        }
        found.push((name, entry.path()));
    }

    // Case-insensitive, because a corpus folder is as likely to be `Brasil` as `brasil`. A picker
    // that sorts every capital ahead of every lower-case name is one nobody can scan.
    found.sort_by_key(|(name, _)| name.to_lowercase());

    listing.total = found.len();
    // A page above the end draws the last one rather than nothing. An offset outlives the listing it
    // was written against: a folder is walked away from and come back to, and things are created
    // and deleted in it meanwhile.
    let last_page = listing.total.saturating_sub(1) / PAGE * PAGE;
    listing.offset = ask.offset.min(last_page);
    // Empty means *do not draw this control*. It has to be a separate test from what `page_at`
    // returns, since the first page's own query string is legitimately empty too.
    if listing.offset > 0 {
        listing.previous = page_at(ask, listing.offset.saturating_sub(PAGE));
    }
    if listing.offset + PAGE < listing.total {
        listing.next = page_at(ask, listing.offset + PAGE);
    }

    listing.rows = found
        .into_iter()
        .skip(listing.offset)
        .take(PAGE)
        .map(|(name, path)| Folder {
            name,
            path: path.display().to_string(),
        })
        .collect();
    listing
}

/// The same folder and filter at another offset, as a query string with no leading `?`.
///
/// Every part of the ask has to be here. One left out is a control that quietly drops the filter
/// somebody typed, which is discovered on the second page of a folder they have narrowed.
fn page_at(ask: &Ask, offset: usize) -> String {
    let encode =
        |value: &str| form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    let mut parts = Vec::new();
    if let Some(here) = ask.here.as_deref() {
        parts.push(format!("at={}", encode(&here.display().to_string())));
    }
    if !ask.filter.is_empty() {
        parts.push(format!("filter={}", encode(&ask.filter)));
    }
    if offset > 0 {
        parts.push(format!("offset={offset}"));
    }
    parts.join("&")
}

/// Whether a folder belongs in a karaoke-folder picker.
///
/// **Hidden and system folders are noise here.** Nobody keeps karaoke files in `.git`, `AppData` or
/// `System Volume Information`, and those are most of what stands between somebody and the folder
/// they are walking to. A dot-prefixed name is the portable half of the test. Windows keeps the rest
/// in an attribute, which is the only place `AppData` says what it is.
///
/// A folder somebody deliberately hid is still reachable: a path box opens one by name. What stops
/// is walking to it.
fn worth_showing(entry: &std::fs::DirEntry, name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    #[cfg(windows)]
    {
        // `DirEntry::metadata` on Windows is answered from the directory entry already read, so it
        // costs no system call and does not follow the symlink `file_type` was careful about.
        use std::os::windows::fs::MetadataExt;
        if let Ok(meta) = entry.metadata()
            && concealed(meta.file_attributes())
        {
            return false;
        }
    }
    #[cfg(not(windows))]
    let _ = entry;
    true
}

/// Whether Windows' own attributes say a folder is not for browsing.
///
/// Split out from [`worth_showing`] so the rule can be tested over the bits, rather than through a
/// test that has to make a hidden directory to have anything to assert.
#[cfg(windows)]
fn concealed(attributes: u32) -> bool {
    /// `FILE_ATTRIBUTE_HIDDEN`, named here rather than taken from a platform crate for one constant.
    const HIDDEN: u32 = 0x2;
    /// `FILE_ATTRIBUTE_SYSTEM`.
    const SYSTEM: u32 = 0x4;
    attributes & (HIDDEN | SYSTEM) != 0
}

/// The drive list, which exists only on Windows.
///
/// Probed rather than enumerated through an API, so this needs no platform crate. A drive letter that
/// answers a metadata call is mounted, and one that does not is not. Twenty-six stats, once, on a
/// page nobody opens in a loop.
#[cfg(windows)]
fn drives() -> Listing {
    let mut listing = Listing::default();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if std::fs::metadata(&root).is_err() {
            continue;
        }
        listing.rows.push(Folder {
            name: root.clone(),
            path: root,
        });
    }
    listing.total = listing.rows.len();
    listing
}

/// Everywhere else there are no drives, so the top is the filesystem root.
#[cfg(not(windows))]
fn drives() -> Listing {
    list(&Ask {
        here: Some(PathBuf::from("/")),
        ..Ask::default()
    })
}

/// Where the picker should start when nothing else says.
///
/// The home directory, which is where a person's own files are. It is a shorter walk to a corpus
/// than the filesystem root on any of the three platforms.
pub fn start() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use km_testkit::Scratch;

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
    fn a_folder_that_cannot_be_read_reports_why_instead_of_failing() {
        let listing = all_of(Path::new("/definitely/not/a/real/folder/anywhere"));
        assert!(listing.error.is_some(), "the reason should be carried");
        assert!(listing.rows.is_empty());
        // Still navigable: the trail above it is what lets somebody back out of a wrong turn.
        assert!(listing.here.is_some());
    }

    #[test]
    fn listing_sorts_case_insensitively_and_skips_dotted_names() {
        let scratch = Scratch::new("folders-sorted");
        for name in ["beta", "Alpha", ".hidden"] {
            std::fs::create_dir_all(scratch.join(name)).expect("making a test folder");
        }
        std::fs::write(scratch.join("a-file.mid"), b"").expect("writing a test file");

        let listing = all_of(scratch.path());
        let names: Vec<&str> = listing.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["Alpha", "beta"],
            "dotted names and files are skipped, order folds case"
        );
    }

    /// A folder of more than a page draws a page, and says how many there are.
    #[test]
    fn a_big_folder_draws_a_page_and_counts_the_rest() {
        let scratch = folders("folders-paged", PAGE + 30);

        let first = all_of(scratch.path());
        assert_eq!(first.rows.len(), PAGE);
        assert_eq!(
            first.total,
            PAGE + 30,
            "the count is of what matched, not of what is drawn"
        );
        assert_eq!(first.rows[0].name, "folder-0000");
        assert!(
            first.previous.is_empty(),
            "the first page has nowhere back to"
        );
        assert!(!first.next.is_empty());

        let second = list(&Ask {
            here: Some(scratch.to_path_buf()),
            offset: PAGE,
            ..Ask::default()
        });
        assert_eq!(second.rows.len(), 30);
        assert_eq!(second.rows[0].name, format!("folder-{PAGE:04}"));
        assert!(!second.previous.is_empty());
        assert!(second.next.is_empty(), "the last page has nowhere on to");
    }

    /// A page above the end draws the last one, because an offset outlives the listing it was
    /// written against.
    #[test]
    fn a_page_above_the_end_draws_the_last_one() {
        let scratch = folders("folders-past-the-end", PAGE + 5);

        let listing = list(&Ask {
            here: Some(scratch.to_path_buf()),
            offset: PAGE * 40,
            ..Ask::default()
        });
        assert_eq!(listing.offset, PAGE);
        assert_eq!(listing.rows.len(), 5);
    }

    #[test]
    fn the_filter_narrows_by_name_ignoring_case() {
        let scratch = Scratch::new("folders-filtered");
        for name in ["Brasil", "brasileiras", "Ingles", "Rock"] {
            std::fs::create_dir_all(scratch.join(name)).expect("making a test folder");
        }

        let listing = list(&Ask {
            here: Some(scratch.to_path_buf()),
            filter: "BRAS".to_owned(),
            ..Ask::default()
        });
        let names: Vec<&str> = listing.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Brasil", "brasileiras"]);
        assert_eq!(listing.total, 2, "the count is of what matched");
        // What was typed comes back, so the box redraws holding it.
        assert_eq!(listing.filter, "BRAS");
    }

    /// The page turn carries the folder and the filter, or narrowing would end at the first page.
    #[test]
    fn a_page_turn_keeps_the_folder_and_the_filter() {
        let scratch = folders("folders-turn-keeps", PAGE + 1);

        let listing = list(&Ask {
            here: Some(scratch.to_path_buf()),
            filter: "folder".to_owned(),
            ..Ask::default()
        });
        assert!(listing.next.contains("at="), "{}", listing.next);
        assert!(listing.next.contains("filter=folder"), "{}", listing.next);
        assert!(
            listing.next.contains(&format!("offset={PAGE}")),
            "{}",
            listing.next
        );
    }

    /// Windows says `AppData` is hidden and says nothing else about it, so the attribute is the
    /// only place the rule can read.
    #[cfg(windows)]
    #[test]
    fn a_hidden_or_system_folder_is_not_walked_to() {
        const NORMAL: u32 = 0x80;
        assert!(!concealed(NORMAL));
        assert!(concealed(NORMAL | 0x2), "hidden");
        assert!(concealed(NORMAL | 0x4), "system");
    }
}
