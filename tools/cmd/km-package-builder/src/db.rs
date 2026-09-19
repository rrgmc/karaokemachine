//! The curation database.
//!
//! A `Db` is one `rusqlite::Connection`, owned outright, the way `km-catalog` does it. There is no
//! pool, and an open folder holds exactly two of these: one that writes and one that only reads.
//!
//! **The reason for the second is not how many people use the tool.** It is that a scan holds the
//! connection it writes through for the length of a batch, so a page drawn through that same
//! connection had to wait for a gap between batches — and over a whole corpus that is a wait with
//! nothing bounding it. [`Db::open_reading`] is the second connection and `server.rs`'s header is
//! where the arrangement is set out.
//!
//! Every query that takes text from a person binds it. The FTS `MATCH` clause is the one place raw
//! text could reach SQL as syntax rather than data, and [`fts_match_query`] is what stops it.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use km_kmpkg::{Language, Tag};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, named_params, params, params_from_iter,
};

use crate::model::{
    CdgFacts, FavoriteNode, HandSetSong, LyricHit, PackageMember, PackageRow, SavedFilter,
    ScanStatus, ScannedFile, SongKind, SongRow, StoredWarning, VideoFacts,
};

// Every one of these is a private child of `db`, and every type they speak in is re-exported below,
// so nothing outside this file learned that any of it moved. **They are not layers**: `songs.rs`,
// `packages.rs`, `favorites.rs`, `backup.rs`, `scan.rs` and `open.rs` are six `impl Db` blocks on the
// one `Db`, split by the question being asked rather than by anything the type system enforces. The
// seams are the ones this file already drew as `// -- packages ---` banners, which is why the split
// is a move with no logic in it.

/// What the browse bar narrows by, and how each control becomes SQL — see the module's own header.
mod filter;
/// The version check and the ladder above it — see the module's own header.
mod migrate;
/// The shapes a query hands back — see the module's own header.
mod model;
/// The SQL fragments and the row readers — see the module's own header.
mod sql;

/// The hand-made half, out and back in — see the module's own header.
mod backup;
/// Grouping suggested pairs, and which file of a group is shown — see the module's own header.
mod cluster;
/// Suggested near-duplicates and their verdicts — see the module's own header.
mod dupes;
/// Somebody's own filing of the corpus — see the module's own header.
mod favorites;
/// Opening, migrating and backfilling — see the module's own header.
mod open;
/// Packages and the merges that change them — see the module's own header.
mod packages;
/// Filters somebody named — see the module's own header.
mod saved;
/// What a corpus scan writes and forgets — see the module's own header.
mod scan;
/// Browsing, searching the words, and editing — see the module's own header.
mod songs;

/// Every test over any of this — see the module's own header for why they are not split too.
#[cfg(test)]
pub(crate) mod tests;

/// Timing a page and a scan batch against a real corpus — see the module's own header for why it is
/// two ignored tests and not one of the census examples.
#[cfg(test)]
mod measure;

// **Re-exported flat, so nothing outside this file learned that it moved.** `crate::db::SongDetail`
// and `crate::db::Filter` are what `handlers.rs`, `views.rs` and `backup.rs` have always written,
// and a split that made them write `crate::db::model::SongDetail` instead would have been a rename
// of forty call sites wearing a refactor's clothes. The submodules are private for the same reason:
// there is one public path to each of these types and it is the one that already existed.
pub use self::cluster::ClusterCounts;
pub use self::filter::*;
pub use self::model::*;

// `prefix_range` is the one thing in `sql` a caller outside `db` has: `handlers.rs` needs it to ask
// for a folder's children. The rest is not exported, because a fragment builder loose in the program
// is how a second spelling of "the title to show" gets written.
pub use self::sql::prefix_range;

// These two are private and stay private: nothing outside `db` may run a migration, ask what version
// a database is, or assemble a `WHERE` clause. The globs are how the `impl Db` blocks reach them.
use self::migrate::*;
use self::sql::*;

/// The extension every curation database carries.
///
/// The database *is* the document: a `.kmbuild` file is an ordinary SQLite database, and the reason
/// it does not say `.sqlite` is that the operating system needs an extension it can hand to one
/// program. Associating `.sqlite` would have claimed every SQLite file on the machine, which belongs
/// to nobody. See the `A corpus is a document` entry in `docs/decisions/`.
pub const DATABASE_EXTENSION: &str = "kmbuild";

/// What `--init` and the Open page call a database they are creating.
///
/// Only the default. [`database_in`] finds whatever single `.kmbuild` file a folder holds, so a
/// person may rename theirs to `Brasil.kmbuild` and have a file manager show the corpus by name —
/// which is most of the point of it being a document at all.
pub const DATABASE_NAME: &str = "km-package-builder.kmbuild";

/// The folder inside a corpus that the tool's own output goes into.
///
/// Packages, descriptions and backups loose in the corpus root, next to the songs, would be tidy on a
/// folder holding a handful of files; on one holding hundreds of thousands they are three files that
/// are impossible to find again. A folder of their own means a package built last week is somewhere
/// with four things in it.
///
/// **Leading underscore so it sorts away from the songs** in every file manager and in `ls`, and
/// because a corpus is somebody else's directory that this tool is a guest in.
///
/// The scanner does not have to be taught to skip it: `scan` walks through `km_pack::collect_*`,
/// which filter by extension, and `.kmpkg`, `.kmspec.yaml` and `.kmbackup.json` are none of the
/// ones it collects.
pub const DATA_SUBDIR: &str = "_kmbuild-data";

/// Where the tool writes what it produces, for a corpus rooted at `root`.
///
/// **The database is deliberately not in here**, and that is the one thing about this folder worth
/// stating twice. A `.kmbuild` is the document you double-click: [`database_in`] looks in `root`
/// itself, the file association opens the corpus by opening that file, and `A corpus is a document`
/// in `docs/decisions/` is the row that settled it. Moving it one level down would make the corpus
/// a folder you cannot open by opening anything.
pub fn data_dir(root: &Path) -> PathBuf {
    root.join(DATA_SUBDIR)
}

/// Joins a path stored in the database onto the corpus root, refusing one that would leave it.
///
/// **A `.kmbuild` is a document, so `files.path` is somebody's claim and not this tool's own
/// writing.** The schema asks for a relative path and nothing makes it one: an absolute path
/// discards the root when joined, and a `..` component walks out of it. What the routes do with the
/// answer is why it matters — a song's file is read, downloaded, handed to the system opener, and
/// uploaded to the machine named by the same database — so an unchecked path here reads and sends
/// any file its opener can.
///
/// Every route reaching a file goes through [`Db::best_file`], and this is the guard on it, placed
/// here rather than at each caller so a route added later inherits it.
///
/// **A symlink below the root is deliberately still followed.** Refusing one would need
/// canonicalizing every lookup, and a corpus that spans two drives through a linked folder is an
/// ordinary way to keep one. The rule this enforces is about what the *document* may say, and a
/// link is something the corpus's owner made.
pub(crate) fn contained(root: &Path, relative: &str) -> Result<PathBuf, DbError> {
    if !km_kmpkg::is_safe_path(relative) {
        return Err(DbError::Rejected(format!(
            "{relative:?} is not a path below the corpus folder, so it will not be opened"
        )));
    }
    Ok(root.join(relative))
}

/// The database in `root`, if it has exactly one.
///
/// Returns `None` both when there is no `.kmbuild` file and when there are several — a folder holding
/// two of them has no single answer to "which corpus is this?", and picking one arbitrarily would
/// silently curate the wrong index. [`require_database`] is what turns each case into its own
/// sentence.
pub fn database_in(root: &Path) -> Option<PathBuf> {
    let mut found = candidates(root);
    match found.len() {
        1 => Some(found.remove(0)),
        _ => None,
    }
}

/// Every `.kmbuild` file in `root`, sorted, so a message can name them all.
fn candidates(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    // Case-insensitively: Windows and macOS both hand back whatever case the file
                    // was created with, and a `.KMBUILD` that did not open would be baffling.
                    .is_some_and(|ext| ext.eq_ignore_ascii_case(DATABASE_EXTENSION))
        })
        .collect();
    found.sort();
    found
}

/// Errors this layer can produce.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Anything SQLite reported.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A row was asked for that does not exist.
    #[error("no such {0}")]
    NotFound(String),
    /// The caller asked for something the data will not allow.
    #[error("{0}")]
    Rejected(String),
    /// Nothing is open, so there is no data to ask anything of.
    ///
    /// **Its own variant rather than a [`Rejected`](DbError::Rejected) with a sentence in it**,
    /// because it is the one error whose right answer is not a message: every folder-dependent route
    /// can now produce it, and the response mapper turns it into a redirect to the Open page. Told
    /// apart only by its text, that would have been a rendered 500 reading "no folder is open" on a
    /// page offering no way to open one.
    #[error("no folder is open")]
    NoWorkspace,
    /// The connection that writes was busy for longer than a request may wait for it.
    ///
    /// **Its own variant rather than a [`Rejected`](DbError::Rejected) with a sentence in it**, for
    /// the reason [`NoWorkspace`](DbError::NoWorkspace) is one: it is neither the person's mistake
    /// nor the program's, so it is neither a 400 nor a 500 — it means *ask again*, and only a
    /// variant can carry that as far as the response.
    ///
    /// **A backstop rather than the ordinary answer during a scan.** A scan commits hundreds of
    /// scattered rows at a time and nothing else may write inside one of those, but it stands aside
    /// between batches — see [`Shared::wanted`] — so a star clicked while a corpus is being read
    /// waits out one batch and goes through. This is what is left: a holder that has died, or a
    /// stretch longer than `server.rs`'s `WRITE_WAIT` with no gap anywhere in it.
    ///
    /// Reads do not come here at all: they go through a connection of their own — see
    /// [`Db::open_reading`].
    #[error("the database is busy writing")]
    Busy,
}

impl DbError {
    /// What a page says about this, in the language it is being drawn in.
    ///
    /// **`Display` stays as it is and stays English**: it is what the log carries and what a console
    /// run prints, and both are read beside a stack trace. What a page says is this, which is the
    /// shape `A refusal travels as a code, and the page writes the sentence` sets.
    ///
    /// [`Self::Rejected`] carries a sentence its caller already worded through the catalog, and
    /// [`Self::NoWorkspace`] never reaches a page at all — `handlers::failure` turns it into a
    /// redirect to the folder picker.
    pub fn say(&self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Sqlite(error) => words
                .msg_with(
                    "db-error-sqlite",
                    &[("why", error.to_string().as_str().into())],
                )
                .into_owned(),
            Self::NotFound(what) => words
                .msg_with("db-error-not-found", &[("what", what.as_str().into())])
                .into_owned(),
            Self::Rejected(said) => said.clone(),
            Self::NoWorkspace => words.msg("db-error-no-folder").into_owned(),
            Self::Busy => words.msg("db-error-busy").into_owned(),
        }
    }
}

/// The indexes whose construction is slow enough that a person should be told it is happening.
///
/// Two things are decided from this list before the schema batch runs: whether to print a banner,
/// and — the load-bearing one — whether to gather statistics again afterwards, since an index with
/// no `sqlite_stat1` row is one the planner will not choose.
///
/// It spans both places indexes are declared, deliberately. `files_song_path` is an ordinary index
/// and lives in `schema.sql`; the ten `songs_browse_*` have expression keys and are built in
/// [`Db::create_browse_indexes`]. The pause a person actually experiences covers both, so the check
/// that reports it has to as well. `Db::create_browse_indexes` carries a `debug_assert!` that it has
/// not gained an index this list does not know about.
const HEAVY_INDEXES: &[&str] = &[
    "files_song_path",
    // Cheap to read and not cheap to build — one pass over `files` — and it is here for the second
    // of the two things this list decides rather than the banner: an index with no `sqlite_stat1`
    // row is one the planner will not choose, so a database that already had statistics and has
    // just gained this one needs them gathered again or the index is dead weight.
    "files_failed",
    // The same, one table over, and the statistics matter more here than for any other name on this
    // list: without them the planner goes on scanning `songs` for a filtered count, which is the
    // thirteen seconds this index exists to remove.
    "songs_countable",
    // The discard pile, and it is here for the same second reason: it is empty on a corpus nobody
    // has deleted from, so the first deletion is the moment the planner needs statistics for it.
    // Without them the *only deleted* list takes `songs_duplicate_of`'s equality instead, which
    // matches nearly every song and then filters.
    "songs_deleted",
    // **An index whose key changes takes a new name, and the name says what the key is.** Every
    // `CREATE` in `create_browse_indexes` is `IF NOT EXISTS`, which cannot see a key — so under the
    // same name an existing database would keep the old one for ever, and nothing would say so.
    // `create_browse_indexes` drops every `songs_browse_*` this list does not name, so the pair is
    // what retires a generation. The name is load-bearing a second time here: `missing_indexes`
    // reports a renamed index as absent, which is what asks for the banner and for the `ANALYZE`
    // without which the planner will not choose it.
    "songs_browse_title_artist",
    "songs_browse_letter_artist",
    "songs_browse_suitability_artist",
    "songs_browse_sort_artist",
    "songs_browse_language_artist",
    "songs_browse_user_score_artist",
    "songs_browse_duration",
    "songs_browse_copies",
    "songs_browse_updated_artist",
    "songs_browse_added_artist",
];

/// Which of `names` the database does not have.
fn missing_indexes(
    conn: &Connection,
    names: &[&'static str],
) -> Result<Vec<&'static str>, DbError> {
    let mut absent = Vec::new();
    for name in names {
        let found: i64 = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
            [name],
            |row| row.get(0),
        )?;
        if found == 0 {
            absent.push(*name);
        }
    }
    Ok(absent)
}

/// Whether there are enough songs here for building an index to be worth announcing.
///
/// Under the threshold the pause is imperceptible and a banner would be noise on every fresh
/// database, every `--init`, and every test. Tolerant of `songs` not existing yet, because this is
/// asked *before* the schema batch that creates it.
fn counts_as_a_corpus(conn: &Connection) -> Result<bool, DbError> {
    if !has_table(conn, "songs")? {
        return Ok(false);
    }
    let songs: i64 = conn.query_row("SELECT COUNT(*) FROM songs", [], |row| row.get(0))?;
    Ok(songs > 10_000)
}

/// A connection, and a count of who is waiting for it.
///
/// **The count is the whole reason this is a type rather than a bare mutex.** `std::sync::Mutex`
/// makes no fairness promise, so a thread that unlocks and immediately locks again does not hand it
/// over — and that is exactly what a scan's writer does, once per batch, for as long as reading a
/// whole corpus takes. A waiter can therefore be passed over indefinitely by a holder that is
/// technically releasing the lock between every unit of work.
///
/// So the mutex and the queue depth live together, because they are useless apart: a request counts
/// itself in while it waits, and a long job asks between units of work whether anybody is there and
/// [stands aside](Self::wanted) until they have been through. Kept as two values passed separately,
/// the day somebody adds a third lock site is the day one of them stops being counted.
///
/// **Both connections a folder holds are one of these.** A dedicated reading connection is never
/// contended, so the count stays at zero and costs a relaxed load; where there is no second
/// connection the reads come through this one, and then they are exactly the waiters a scan should
/// be standing aside for.
pub struct Shared {
    db: std::sync::Mutex<Db>,
    waiting: std::sync::atomic::AtomicUsize,
}

impl Shared {
    /// Wraps a connection.
    pub fn new(db: Db) -> Self {
        Self {
            db: std::sync::Mutex::new(db),
            waiting: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Takes the connection, waiting however long it takes. What a long job uses.
    ///
    /// **Poisoning is recovered rather than propagated, and that is one policy in one place** where
    /// this crate had two of them on the same mutex. `server.rs` recovered with `into_inner` and
    /// argued for it; `scan.rs` asserted "the database lock is never poisoned" at nine sites. So a
    /// handler that panicked while holding it left the web UI working and the next scan dying, which
    /// is the worst of both answers and was nobody's decision.
    ///
    /// Recovery is the right one. A [`Connection`] is not made untrustworthy by a panic elsewhere
    /// that happened to be holding the lock; what such a panic costs is the one request it happened
    /// in. Turning that into "everything after it panics too" trades a bad minute for an application
    /// that, in a window with no console, is silently dead.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Db> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Takes it, or gives up at the deadline. What a request uses.
    ///
    /// **A poll, because `std::sync::Mutex` has no timed acquire.** Only reached when the connection
    /// is already held, and on a thread that had nothing else to do, so the cost of asking again is
    /// the wake-up. The wait is counted for the whole of it, which is what [`Self::wanted`] reports
    /// and what a scan stands aside for — so the deadline is a backstop against a holder that has
    /// died rather than the way a write normally gets in.
    pub fn lock_within(&self, wait: Duration) -> Option<std::sync::MutexGuard<'_, Db>> {
        let _queued = Queued::on(self);
        let deadline = Instant::now() + wait;
        loop {
            match self.db.try_lock() {
                Ok(guard) => return Some(guard),
                Err(std::sync::TryLockError::Poisoned(error)) => return Some(error.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return None;
                    }
                    std::thread::sleep(RETRY_EVERY.min(left));
                }
            }
        }
    }

    /// Whether anything is waiting for this connection.
    ///
    /// What a scan asks between batches. It answers for waiters only, never for the holder, so a job
    /// asking about the connection it has just released gets a useful answer.
    pub fn wanted(&self) -> bool {
        self.waiting.load(std::sync::atomic::Ordering::Relaxed) > 0
    }
}

/// How often [`Shared::lock_within`] asks again while it waits.
const RETRY_EVERY: Duration = Duration::from_millis(20);

/// Counts one waiter in for as long as it waits, whatever becomes of the wait.
///
/// A guard rather than a pair of `fetch_add`/`fetch_sub` calls, so a panic or an early return cannot
/// leave a phantom waiter behind — which a scan would stand aside for for ever after.
struct Queued<'a>(&'a Shared);

impl<'a> Queued<'a> {
    fn on(shared: &'a Shared) -> Self {
        shared
            .waiting
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(shared)
    }
}

impl Drop for Queued<'_> {
    fn drop(&mut self) {
        self.0
            .waiting
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// What [`Db::counts`]'s cache is keyed on: this connection's own writes, then every other
/// connection's commits.
///
/// Two counters and not one, because neither sees what the other does. See [`Db::counts`].
type CacheVersion = (u64, i64);

/// A curation database, open on one folder.
pub struct Db {
    conn: Connection,
    root: PathBuf,
    /// The last answer [`Db::counts`] gave, and the database version it was true at.
    ///
    /// A `Cell` rather than a field set by whoever writes, and that is the whole point of it — see
    /// `counts` for why the key is SQLite's own two counters and not something a handler has to
    /// remember to bump.
    counts: Cell<Option<(CacheVersion, Counts)>>,
}

/// Refuses, before anything has been bound or printed, if `root` holds no single database.
///
/// The same check [`Db::open`] makes, in the same words, hoisted so that `main` can make it *before*
/// it opens a port and prints a URL for a server that is about to exit. `Db::open` calls this rather
/// than repeating it, so the two cannot come to disagree about the message.
///
/// **Three refusals, not one**, because the three situations need three different actions and a
/// single "no database here" would send somebody looking for a folder they are already standing in:
///
/// * nothing here — create one, which is the `--init` case;
/// * a database under a name this build does not open — rename it, which the Open page offers as a
///   button and this message spells out as a command. **Not opened silently:** a fallback keeps a
///   dead name alive in every corpus for as long as the tool exists;
/// * several — say which, because no arbitrary choice is safe.
pub fn require_database(root: &Path) -> Result<PathBuf, DbError> {
    if let Some(database) = database_in(root) {
        return Ok(database);
    }

    let found = candidates(root);
    if found.len() > 1 {
        let names: Vec<String> = found
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        return Err(DbError::Rejected(format!(
            "{} holds {} databases ({}). Move all but one aside.",
            root.display(),
            found.len(),
            names.join(", ")
        )));
    }

    Err(DbError::Rejected(format!(
        "{} has no {DATABASE_NAME}. Pass --init to create one here.",
        root.display()
    )))
}

/// Somewhere for an open to say what it is doing now.
///
/// A bare `&dyn Fn(&str)` rather than a type of this module's own, because all that crosses the seam
/// is a sentence: `db` has no business knowing that the thing on the other end is a job behind a web
/// page, and the tests hand it a `Vec` to push onto.
pub type Phase<'a> = &'a dyn Fn(OpeningPhase);

/// What the slow part of an open is doing.
///
/// **A value rather than a sentence**, because an open runs before any page exists and two of
/// these carry a count. The fact travels and whoever shows it writes the words: the Open page in the
/// language it is being drawn in, the console banner in English, which is what a log is written in.
/// `scan::phase` and `build::Phase` are the same arrangement one layer up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpeningPhase {
    /// Opening the file itself, which is where an open with nothing to close begins.
    #[default]
    Database,
    /// Closing whatever folder was open before this one.
    ClosingPrevious,
    /// Running the schema migrations.
    UpToDate,
    /// Building the indexes a newer build wants.
    Indexing {
        /// How many are missing.
        missing: usize,
    },
    /// Working out what language each song is in.
    WorkingOutLanguage,
    /// Reading the words of each song to see what language they are in.
    ReadingWords {
        /// How many are done.
        done: usize,
        /// Out of how many.
        total: usize,
    },
    /// Tidying the text read out of the files.
    TidyingText,
    /// Folding the browse sort keys, which is the other long one.
    Folding {
        /// How many are done.
        done: usize,
        /// Out of how many.
        total: usize,
    },
    /// Gathering statistics over the whole corpus.
    GatheringStatistics,
    /// Folding the journal back into the database.
    FoldingJournal,
    /// The last step before the folder opens.
    Finishing,
}

/// Every rung of an open, in the order one climbs them.
///
/// **Listed whole, and a rung is skipped by being passed.** Which of them an open actually runs is
/// decided by `if`s inside [`Db::prepare`] — indexes this build wants and the file has not got, a
/// fold revision that moved, a guess revision that moved, a database with no statistics — and none
/// of those has a page in reach to tell. So the page lists all eleven and marks each one it is told
/// about, which is what lets it say *not needed this time* rather than leaving a rung waiting for
/// ever.
///
/// **Bare of the counts [`OpeningPhase::say`] folds into its sentence.** The headline says how many
/// titles of how many have been folded; the rung beside it is the name of the step, and the counts
/// arrive next to it as the running step's own detail.
pub const OPENING_LADDER: &[&str] = &[
    "opening-step-closing-previous",
    "opening-step-database",
    "opening-step-up-to-date",
    "opening-step-indexing",
    "opening-step-folding",
    "opening-step-working-out-language",
    "opening-step-reading-words",
    "opening-step-tidying-text",
    "opening-step-gathering-statistics",
    "opening-step-folding-journal",
    "opening-step-finishing",
];

impl OpeningPhase {
    /// The rung of [`OPENING_LADDER`] this stands on.
    pub fn step(self) -> &'static str {
        match self {
            Self::Database => "opening-step-database",
            Self::ClosingPrevious => "opening-step-closing-previous",
            Self::UpToDate => "opening-step-up-to-date",
            Self::Indexing { .. } => "opening-step-indexing",
            Self::WorkingOutLanguage => "opening-step-working-out-language",
            Self::ReadingWords { .. } => "opening-step-reading-words",
            Self::TidyingText => "opening-step-tidying-text",
            Self::Folding { .. } => "opening-step-folding",
            Self::GatheringStatistics => "opening-step-gathering-statistics",
            Self::FoldingJournal => "opening-step-folding-journal",
            Self::Finishing => "opening-step-finishing",
        }
    }

    /// How far through, where the step has a denominator to be through.
    ///
    /// **Two of eleven, and they are the two that take the minutes.** The rest are one statement
    /// each — an `ALTER TABLE`, a `CREATE INDEX`, an `ANALYZE` — with nothing inside them to count,
    /// which is why the bar beside them says only *working*.
    pub fn counted(self) -> Option<(usize, usize)> {
        match self {
            Self::Folding { done, total } | Self::ReadingWords { done, total } => {
                Some((done, total))
            }
            _ => None,
        }
    }

    /// What this says, in one language.
    pub fn say(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        let n = |value: usize| i64::try_from(value).unwrap_or(i64::MAX);
        match self {
            Self::Database => words.msg("opening-database").into_owned(),
            Self::ClosingPrevious => words.msg("opening-closing-previous").into_owned(),
            Self::UpToDate => words.msg("opening-up-to-date").into_owned(),
            Self::Indexing { missing } => words
                .msg_with("opening-indexing", &[("missing", n(missing).into())])
                .into_owned(),
            Self::WorkingOutLanguage => words.msg("opening-working-out-language").into_owned(),
            Self::TidyingText => words.msg("opening-tidying-text").into_owned(),
            Self::Folding { done, total } => words
                .msg_with(
                    "opening-folding",
                    &[("done", n(done).into()), ("total", n(total).into())],
                )
                .into_owned(),
            Self::ReadingWords { done, total } => words
                .msg_with(
                    "opening-reading-words",
                    &[("done", n(done).into()), ("total", n(total).into())],
                )
                .into_owned(),
            Self::GatheringStatistics => words.msg("opening-gathering-statistics").into_owned(),
            Self::FoldingJournal => words.msg("opening-folding-journal").into_owned(),
            Self::Finishing => words.msg("opening-finishing").into_owned(),
        }
    }

    /// What this says to a console, which is a log and so is English.
    pub fn in_english(self) -> String {
        self.say(km_locale::Locale::English)
    }
}

// There is deliberately no `Drop` gathering statistics on the way out. It was written, and it was
// wrong twice over: the only bounded form of it distorts the numbers (see `refresh_statistics`), and
// the unbounded form would put a second of `ANALYZE` between somebody pressing Ctrl-C and the process
// leaving, every time, for a database that in a browsing session has not changed shape at all. The two
// moments that *have* something to learn — a database with no statistics, and the end of a scan — ask
// for it themselves.

impl Db {
    // -- settings ---------------------------------------------------------------------------

    /// Reads a setting.
    pub fn setting(&self, key: &str) -> Result<Option<String>, DbError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Writes a setting.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // -- counts -----------------------------------------------------------------------------

    /// Headline counts for the status bar, from cache when nothing has been written since.
    ///
    /// Six aggregates, two of which walk `files` end to end — every row of the corpus this was
    /// measured against — for a status bar that is the same on every page of the tool. They ran on
    /// every full page load, so merely moving between tabs paid for them.
    ///
    /// **The cache key is a pair, and it has to be: one half notices this connection's own writes
    /// and the other notices everybody else's.**
    ///
    /// `sqlite3_total_changes` is SQLite's own count of every row inserted, updated or deleted on
    /// this connection since it was opened. The alternative — a generation number bumped by each
    /// handler that writes — works right up until somebody adds the twenty-sixth write path and does
    /// not bump it, and the symptom then is a status bar quietly showing yesterday's numbers, which
    /// nobody reports as a bug because it looks like a number. There is nothing to remember: a read
    /// cannot move the counter and a write cannot fail to, triggers included.
    ///
    /// It says nothing at all about a *different* connection, though, and a page now reads through
    /// one of those — [`Db::open_reading`]. On a connection that never writes, `total_changes` is
    /// permanently zero, so this half alone would compute the counts once and show them for the life
    /// of the folder however much a scan wrote.
    ///
    /// `PRAGMA data_version` is the exact complement: it moves when *another* connection commits and
    /// stays put for this one's own writes. So the writer keeps noticing itself through
    /// `total_changes`, a reader starts noticing the writer through `data_version`, and neither needs
    /// to know which kind it is.
    ///
    /// **`data_version` is turned down for a different job**, in `A catalog version is a counter in
    /// the database` in `docs/decisions/remotes.md`, on two grounds that point the other way here: it
    /// only moves when another connection writes, which is precisely the signal wanted, and it resets
    /// when the connection is reopened, which cannot matter to a cache that dies with the connection.
    pub fn counts(&self) -> Result<Counts, DbError> {
        let version = self.cache_version();
        if let Some((at, counts)) = self.counts.get()
            && at == version
        {
            return Ok(counts);
        }
        let counts = self.count_everything()?;
        self.counts.set(Some((version, counts)));
        Ok(counts)
    }

    /// What [`Db::counts`] keys its cache on: this connection's writes, and everyone else's commits.
    ///
    /// `data_version` is best-effort. A pragma that cannot be read leaves the writes half doing the
    /// work alone, which is exactly right for the writing connection and merely stale for a reading
    /// one — the same trade every other pragma in `tune` makes.
    fn cache_version(&self) -> CacheVersion {
        let writes = self.conn.total_changes();
        let data = self
            .conn
            .pragma_query_value(None, "data_version", |row| row.get::<_, i64>(0))
            .unwrap_or(0);
        (writes, data)
    }

    /// The six aggregates themselves. Call [`Db::counts`] instead; this is what it fills its cache
    /// from.
    ///
    /// **One function per aggregate below, so that each can be timed on its own.** They are not
    /// separate for any reason the program needs — this is the only caller — but for the one thing
    /// the program cannot otherwise offer: which of the six a page is actually waiting for. Timing
    /// them by copying the SQL into a measurement would produce a figure about a query nobody runs
    /// the day either spelling moved.
    fn count_everything(&self) -> Result<Counts, DbError> {
        Ok(Counts {
            songs: self.count_songs()? as u32,
            files: self.count_files()? as u32,
            failed: self.count_failed()? as u32,
            favorites: self.count_favorites()? as u32,
            packages: self.count_packages()? as u32,
            deleted: self.count_deleted()? as u32,
        })
    }

    /// Songs a curator has not merged or thrown away.
    ///
    /// [`browsable`], so the number in the header counts the songs the browse list would show.
    ///
    /// Proportional to the corpus and not helped by an index: `songs_merged` exists, but a corpus
    /// has `merged_into` NULL on essentially every row, so seeking it returns the whole table.
    fn count_songs(&self) -> Result<i64, DbError> {
        Ok(self.conn.query_row(
            &format!("SELECT COUNT(*) FROM songs WHERE {}", browsable("")),
            [],
            |row| row.get(0),
        )?)
    }

    /// Every file the corpus holds.
    ///
    /// **The one aggregate here that no index can make cheap.** SQLite keeps no row count, so this
    /// walks the smallest index on `files` whatever else is declared — and a partial index cannot
    /// serve it, because it does not hold every row.
    fn count_files(&self) -> Result<i64, DbError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?)
    }

    /// Failures nobody has accepted, which is what the red badge in the bar is for: it says there is
    /// something to go and look at.
    ///
    /// Counting an accepted one would leave that badge lit on every page for a corpus whose scan
    /// page has nothing left to show, and the only way to put it out would be to make the files
    /// parse.
    ///
    /// Served by `files_failed`, a partial index holding only the rows the question is about, since
    /// `<>` has no range in `files_status` to seek and this otherwise walked every row.
    fn count_failed(&self) -> Result<i64, DbError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM files f
             WHERE f.scan_status <> 'ok'
               AND NOT EXISTS (SELECT 1 FROM dismissed_failures d
                               WHERE d.file_id = f.id AND d.scan_status = f.scan_status)",
            [],
            |row| row.get(0),
        )?)
    }

    /// Songs, not memberships: a song in four favorites is one favorite song, and the status bar is
    /// answering *how much of this corpus have I picked out?*.
    ///
    /// Driven from `song_favorites` with an `EXISTS`, not a join. Written as a join, SQLite reads
    /// `songs` as the outer table and probes the small side — a whole corpus and a tenth of a second
    /// on every page in the tool, for a number that is usually in the hundreds. This way it is one
    /// primary-key lookup per favorited song.
    fn count_favorites(&self) -> Result<i64, DbError> {
        Ok(self.conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT sf.song_id) FROM song_favorites sf
                 WHERE EXISTS (SELECT 1 FROM songs s
                               WHERE s.id = sf.song_id AND {})",
                browsable("s.")
            ),
            [],
            |row| row.get(0),
        )?)
    }

    /// Packages, of which a corpus holds a handful.
    fn count_packages(&self) -> Result<i64, DbError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))?)
    }

    /// Songs in the discard pile.
    ///
    /// **Served by `songs_deleted`, which is partial — `WHERE deleted_at IS NOT NULL`.** It holds
    /// only the rows this asks about, so the cost is the size of the discard pile rather than the
    /// size of the corpus, and it is nothing at all on a corpus nobody has thrown anything away
    /// from.
    ///
    /// **`merged_into` is tested for the reason the list tests it.** *Only deleted* inverts the
    /// deleted half of [`browsable`](crate::db::sql) and keeps the merged half, because a song both
    /// merged and deleted is still a merge and has no row of its own. A count that dropped the
    /// second half would report more than the list it is a count of.
    fn count_deleted(&self) -> Result<i64, DbError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM songs WHERE deleted_at IS NOT NULL AND merged_into IS NULL",
            [],
            |row| row.get(0),
        )?)
    }
}
