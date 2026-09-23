//! Checking that a `.kmbuild` is at the schema this build writes, and bringing it there when it is
//! one numbered step behind.
//!
//! **A database opens at a version this build knows or is refused.** The version is
//! `PRAGMA user_version`; [`OLDEST_SCHEMA_VERSION`] and [`SCHEMA_VERSION`] are the two ends of what
//! opens, and [`step_to`] is the ladder between them. A database below the floor is refused rather
//! than guessed at, because a `.kmbuild` holds months of hand curation and an open that
//! misread one would go on to write over it.

use super::*;

/// The schema this build writes and understands, stamped into `PRAGMA user_version`.
///
/// Bump this and add an arm to [`step_to`] in the same change. The number keeps counting.
pub(super) const SCHEMA_VERSION: u32 = 20;

/// The oldest schema this build opens. Everything from here to [`SCHEMA_VERSION`] is an arm of
/// [`step_to`].
///
/// **Raised when a step is retired, never lowered.** A database below it is refused with both numbers
/// in the message, which is a better answer than a step nobody has run in years getting it wrong.
pub(super) const OLDEST_SCHEMA_VERSION: u32 = 14;

/// Brings a database up to [`SCHEMA_VERSION`], or refuses one this build cannot vouch for.
///
/// **The refusals are the point of the version existing.** A `.kmbuild` from a newer build would
/// otherwise open silently: SQLite is happy to hand back a row from a table with columns this build
/// has never heard of, and the tool would then curate a corpus while quietly ignoring whatever the
/// newer schema had added. One from below the floor would be read by queries written for a shape it
/// does not have.
///
/// **A version of 0 means two different things**, told apart by whether the `songs` table exists. A
/// brand-new file has none yet — `schema.sql` runs after this, so the triggers it creates find every
/// column they name — and is stamped current. A file with `songs` and no stamp carries no version
/// this build can place, and is refused like any other below the floor.
pub(super) fn migrate(conn: &Connection, phase: Phase<'_>) -> Result<(), DbError> {
    let found: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if found > SCHEMA_VERSION {
        return Err(DbError::Rejected(format!(
            "this database was written by a newer build of km-package-builder (schema {found}; \
             this build understands {SCHEMA_VERSION}). Opening it would quietly ignore whatever \
             that build added, so it is refused instead. Use the newer build, or start a fresh \
             database over the same folder"
        )));
    }

    if found == 0 && !has_table(conn, "songs")? {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        return Ok(());
    }

    if found < OLDEST_SCHEMA_VERSION {
        return Err(DbError::Rejected(format!(
            "this database is at schema {found}, and this build of km-package-builder opens schema \
             {OLDEST_SCHEMA_VERSION} to {SCHEMA_VERSION}. It is refused rather than read as a \
             shape it does not have"
        )));
    }

    // Nothing at all to do, which is what every open of a current database costs.
    if found == SCHEMA_VERSION {
        return Ok(());
    }

    // **Said before the first step, not inside one.** A step is a handful of statements with no loop
    // in it to report from, so one sentence covering the lot is what stops the page holding the
    // sentence before this one for the length of the work.
    phase(OpeningPhase::UpToDate);

    for version in (found + 1)..=SCHEMA_VERSION {
        step_to(conn, version)?;
    }

    // Stamped last, so an interrupted step is retried rather than skipped.
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// One numbered migration step.
///
/// **Each arm knows exactly what the database looked like before it**, because the version says so.
fn step_to(conn: &Connection, version: u32) -> Result<(), DbError> {
    match version {
        // The box that numbers a package's only volume. Off, so every package keeps its name.
        15 => {
            conn.execute_batch(
                "ALTER TABLE packages
                   ADD COLUMN number_one_volume INTEGER NOT NULL DEFAULT 0",
            )?;
            Ok(())
        }
        // What the song's own words read as, and how sure the reading was. Both start NULL on every
        // row: `Db::backfill_language_guess` fills them from text the database already holds, so
        // nothing here has to open a file.
        16 => {
            conn.execute_batch(
                "ALTER TABLE songs ADD COLUMN det_language_guess TEXT;
                 ALTER TABLE songs ADD COLUMN det_language_guess_confidence REAL",
            )?;
            Ok(())
        }
        // Whether a song plays with none of its words drawn. NULL on every row, which is the state
        // that takes whatever the analysis concludes -- so a corpus already curated keeps every
        // answer it has and gains the new one where nobody has spoken.
        17 => {
            conn.execute_batch("ALTER TABLE songs ADD COLUMN lyrics_hidden INTEGER")?;
            Ok(())
        }
        // When somebody threw a song away. Every row starts NULL, which is what a corpus nobody has
        // deleted from holds anyway, so there is nothing to backfill.
        //
        // **The `DROP INDEX` is the half that is easy to miss.** `songs_countable` changes shape in
        // `schema.sql`, and every statement in that file is `IF NOT EXISTS` -- so a database
        // already holding the old index would keep it, and the count that index exists to answer
        // would go back to reading every row. Dropping it here is what makes the next
        // `execute_batch` of `schema.sql` build the one this version wants.
        18 => {
            conn.execute_batch(
                "ALTER TABLE songs ADD COLUMN deleted_at TEXT;
                 DROP INDEX IF EXISTS songs_countable",
            )?;
            Ok(())
        }
        // `songs_countable` again, and this time it is the shape rather than the width.
        //
        // **A three-column key holding `deleted_at` costs four seconds a page**, because three
        // equality terms look like the best match available and the planner takes this index for
        // the browse query — which then sorts the whole corpus by hand, having been handed no
        // order. The predicate belongs in the `WHERE`, where the count still implies it and the
        // sort indexes keep their bids. `schema.sql` carries the argument at length.
        //
        // Dropped by name for step 18's reason: every statement in that file is `IF NOT EXISTS`,
        // so a database holding either earlier shape would keep it.
        19 => {
            conn.execute_batch("DROP INDEX IF EXISTS songs_countable")?;
            Ok(())
        }
        // The header flags an imported package carried. `0` on every row, which is what a package
        // this tool made from its own songs carries anyway.
        20 => {
            conn.execute_batch("ALTER TABLE packages ADD COLUMN flags INTEGER NOT NULL DEFAULT 0")?;
            Ok(())
        }
        _ => Err(DbError::Rejected(format!(
            "no migration leads to schema {version}; this build writes {SCHEMA_VERSION}"
        ))),
    }
}

/// The three names on a song row that [`Db::clean_detected_text`] rewrites.
///
/// A struct rather than a tuple because four `Option<String>`s in a row say nothing about which is
/// which, and this one is read twice: once out of the database and once back into it.
pub(super) struct CleanedNames {
    pub(super) id: String,
    pub(super) title: Option<String>,
    pub(super) artist: Option<String>,
    pub(super) stem: Option<String>,
}

impl CleanedNames {
    /// This row with the names nobody can read taken out of it, or `None` if there were none.
    ///
    /// Returning `None` for an unchanged row is what keeps the sweep to the rows that need it rather
    /// than every row: an `UPDATE` here fires both full-text triggers and maintains three expression
    /// indexes, so rewriting a row to the value it already held is not free.
    pub(super) fn cleaned(self) -> Option<Self> {
        let title = self.title.as_deref().and_then(km_song::clean_meta_name);
        let artist = self.artist.as_deref().and_then(km_song::clean_meta_name);
        // The stem comes from a file name rather than from inside the file, so it never carries a
        // control character -- but 399 of them have a space on one end, which sorts ahead of the
        // letter the song belongs under. Free to straighten while the row is in hand.
        let stem = self.stem.as_deref().map(|value| value.trim().to_owned());
        let unchanged = title == self.title && artist == self.artist && stem == self.stem;
        (!unchanged).then_some(Self {
            id: self.id,
            title,
            artist,
            stem,
        })
    }
}

/// The `settings` key holding the sweep revision [`Db::clean_detected_text`] last ran at.
///
/// A settings key rather than a schema version: the sweep is a one-off repair of data, not a shape
/// change, and every other migration here is driven by what the tables look like.
pub(super) const CLEANED_META: &str = "cleaned_meta_text";

/// Which spelling of [`km_song::clean_meta_name`] wrote the detected names in this database.
///
/// A revision rather than the boolean this began as, and for the reason [`LANGUAGE_TAGS`] is one:
/// the gate has now tightened twice, so a database swept by the first spelling has to be swept once
/// more and a database swept by this one has to stay free. **Bump it whenever the gate would answer
/// differently**, which costs every curation database in the field one pass over its songs.
///
/// `1` took the control characters out. `2` also refuses a name that is mostly marks.
pub(super) const CLEANED_META_REVISION: u32 = 2;

/// The settings key holding the language-table revision the detected codes were computed from.
///
/// A revision rather than a boolean, so that changing `km_kmpkg`'s mapping tables re-runs
/// [`Db::backfill_language_tags`] once and only once. See that function for why this is a flag at
/// all, where `stem` uses a partial index.
pub(super) const LANGUAGE_TAGS: &str = "language_tags_revision";

/// The settings key holding the guess revision the read-from-the-words codes were computed from.
///
/// [`LANGUAGE_TAGS`]'s twin one column over, and a revision for the same reason: the detector's
/// model, the code table and the confidence gate all decide what a guess says, and
/// `km_langguess::GUESS_REVISION` moves whenever one of them does, which re-runs
/// [`Db::backfill_language_guess`] over every song exactly once.
pub(super) const LANGUAGE_GUESS: &str = "language_guess_revision";

/// The settings key holding how far [`Db::backfill_language_guess`] has read.
///
/// **A cursor rather than a pending set**, because NULL is one of the answers: a song whose words
/// nothing could place stores no code, so "the rows with no guess" would name the refused ones for
/// ever and re-read them at every open. The id last written says the same thing in one row, and it
/// is deleted when [`LANGUAGE_GUESS`] is written.
pub(super) const LANGUAGE_GUESS_CURSOR: &str = "language_guess_cursor";

/// The settings key holding the fold revision the browse keys were computed from.
///
/// The twin of [`LANGUAGE_TAGS`], one column over: `km_song::text::FOLD_REVISION` says which
/// spelling of the fold wrote `sort_title` and `sort_artist`, so a change to that table blanks them
/// once and `Db::backfill_sort_keys` fills them again with the machinery it already has.
///
/// **A revision rather than a boolean, and rather than a schema version.** This is a repair of data
/// and not a change of shape, which is [`CLEANED_META`]'s argument; and the work list grows again
/// every time the fold moves, which a boolean would freeze.
pub(super) const FOLD_REVISION: &str = "fold_revision";

/// The indexes on a table that this schema declared, excluding the ones SQLite made for itself.
///
/// **Derived, not written down.** A hand-kept list goes stale the day an index is added to
/// `schema.sql`, and an index it missed is left in place by whatever meant to replace it — a database
/// that is correct and merely slow, which is the hardest kind of damage to notice.
///
/// `sql IS NOT NULL` is what excludes `sqlite_autoindex_songs_1`, the index behind `id TEXT PRIMARY
/// KEY`: it cannot be dropped and asking to is an error rather than a no-op.
///
/// Names alone, which is what a test asking whether an index is there wants. The open path compares
/// the statement as well, through [`own_indexes_with_sql`], because an index can be wrong without
/// being absent.
#[cfg(test)]
pub(super) fn own_indexes(conn: &Connection, table: &str) -> Result<Vec<String>, DbError> {
    Ok(own_indexes_with_sql(conn, table)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// The same indexes, each with the `CREATE` statement that made it.
///
/// **What a name alone cannot answer is whether the key is still the one this build wants.** An
/// index whose *body* changed keeps its name, so `CREATE INDEX IF NOT EXISTS` is a no-op and the old
/// key survives — the browse page then matches no index and sorts a whole corpus, with nothing
/// anywhere saying so. Comparing the stored statement is what catches that.
pub(super) fn own_indexes_with_sql(
    conn: &Connection,
    table: &str,
) -> Result<Vec<(String, String)>, DbError> {
    let mut statement = conn.prepare(
        "SELECT name, sql FROM sqlite_master
         WHERE type = 'index' AND tbl_name = ?1 AND sql IS NOT NULL",
    )?;
    let rows = statement
        .query_map([table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// An index statement with the spacing taken out, so two spellings of one key compare equal.
///
/// SQLite stores the `CREATE` verbatim, newlines and runs of spaces included, and this build writes
/// its bodies through `format!` across several lines. Comparing the text as stored would call every
/// index stale on every open and rebuild eight corpus-sized B-trees each time.
pub(super) fn index_shape(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Settings that only start to matter at corpus scale.
///
/// Every one of them is tolerated rather than required, the same as the WAL line in [`Db::prepare`]
/// and for the same reason: an in-memory database and one on a network share are both real here, and
/// neither is a reason to refuse to start.
///
/// `execute_batch` rather than `pragma_update`, which is not a style choice — `pragma_update` goes
/// through `Statement::execute` and errors on a pragma that returns a row, which `mmap_size` does.
pub(super) fn tune(conn: &Connection) {
    tune_shared(conn, WRITER_CACHE_KIB);
    // NORMAL is crash-safe under WAL — a power loss can lose the last commits, it cannot corrupt —
    // and is *not* crash-safe under a rollback journal. So it is set only where the WAL switch above
    // actually took, which is a readback rather than an ignored error because getting this wrong is
    // silent. FULL costs an fsync per commit, and this tool commits once per scan batch across
    // hundreds of thousands of files and once per star somebody clicks.
    match journal_mode(conn).as_deref() {
        Some("wal") => {
            let _ = conn.execute_batch("PRAGMA synchronous = NORMAL;");
        }
        // **Said, because the two tolerable reasons and the one fault look identical from here.** An
        // in-memory database has no journal to switch and a network share refuses one; a switch that
        // lost its exclusive lock leaves the connection in rollback-journal mode, where a writer
        // excludes readers and an ordinary page render answers "database is locked". Only the log
        // can tell them apart.
        other => tracing::warn!(
            mode = other.unwrap_or("unreadable"),
            "not in WAL: readers and writers block each other, and synchronous stays FULL"
        ),
    }
}

/// The page cache the writing connection takes, in KiB.
///
/// It holds the whole of `songs`'s primary-key index, every secondary and the folder tree, which is
/// what keeps a rebuild and a scan batch off the disk.
const WRITER_CACHE_KIB: u32 = 262_144;

/// The page cache a reading connection takes, in KiB.
///
/// Far smaller than the writer's, because `cache_size` is per connection and a reader answering one
/// page at a time touches a fraction of what a corpus-wide rewrite does. Both at the writer's figure
/// would reserve half a gigabyte to serve a page.
const READER_CACHE_KIB: u32 = 65_536;

/// The pragmas every connection wants, with the page cache sized for what this one is for.
///
/// `cache_size` is in KiB when negative, pages when positive.
///
/// `mmap_size` covers the file with headroom, so reads come from the OS mapping with no copy through
/// the pager. Address space is free in a 64-bit process. Worth knowing: under mmap an I/O error
/// arrives as an access violation rather than a return code, which is one more reason this whole
/// function is best-effort.
///
/// `temp_store` keeps a spilled sort in memory. Any `ORDER BY` an index cannot serve builds one, and
/// the default sends it to the system disk's temp folder — a second disk to seek, on a machine whose
/// corpus is already on a slow one.
///
/// `busy_timeout` is the one here that is not about speed. SQLite's default is **zero** — a lock held
/// by anyone else is an immediate `SQLITE_BUSY` rather than a wait — and that was survivable only
/// while a corpus could have exactly one curator, which stopped being true when the database became a
/// document somebody can double-click twice. Five seconds is far longer than any contention this tool
/// can generate and far shorter than a person's patience.
fn tune_shared(conn: &Connection, cache_kib: u32) {
    let mapped = mapped_bytes();
    let _ = conn.execute_batch(&format!(
        "PRAGMA cache_size = -{cache_kib};
         PRAGMA mmap_size  = {mapped};
         PRAGMA temp_store = MEMORY;
         PRAGMA busy_timeout = 5000;"
    ));
}

/// How much of the file the mapping covers, which `tune_shared` sets on every connection.
///
/// A named constant rather than a literal in the batch above, so that the one test allowed to change
/// it has something to change and the shipped figure has something to be pinned against.
const MAPPED_BYTES: i64 = 1_073_741_824;

/// What a measurement asked for instead, or a negative for *the figure above*.
///
/// **`#[cfg(test)]`, so no shipped build carries the branch and no shipped code reads an environment
/// variable.** What the mapping should be is a decision this program owns; a pragma a stray variable
/// could change is a worse thing to carry than a branch that cannot exist outside a test binary.
///
/// **It is here, rather than a second opener beside `Db::open_reading`, because of where `tune` sits
/// in an open.** `Db::prepare` runs the version ladder and the backfills, all of which query, and it
/// calls `tune` in the middle of them — so choosing the mapping per connection would mean threading
/// a figure through `prepare`, `tune`, `tune_reading` and `tune_shared` to arrive at this one line.
/// Set here it reaches every connection the process opens, reader and writer, before either runs its
/// first statement, which is the only moment the choice can be made: taking the mapping off does not
/// un-fault pages already touched through it.
#[cfg(test)]
static MAPPED_BYTES_FOR_TEST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// Asks every connection opened after this to map `bytes`, or restores the shipped figure with a
/// negative. See [`MAPPED_BYTES_FOR_TEST`].
#[cfg(test)]
pub(super) fn map_this_much_for_test(bytes: i64) {
    MAPPED_BYTES_FOR_TEST.store(bytes, std::sync::atomic::Ordering::Relaxed);
}

/// The mapping in force for the next connection.
fn mapped_bytes() -> i64 {
    #[cfg(test)]
    {
        let asked = MAPPED_BYTES_FOR_TEST.load(std::sync::atomic::Ordering::Relaxed);
        if asked >= 0 {
            return asked;
        }
    }
    MAPPED_BYTES
}

/// The same, for a connection that will only ever read.
///
/// No `journal_mode` and no `synchronous`: the first needs an exclusive lock a read-only connection
/// cannot take, and the second describes writes this one cannot make. The writer sets both, and this
/// connection is opened after it.
pub(super) fn tune_reading(conn: &Connection) {
    tune_shared(conn, READER_CACHE_KIB);
}

/// The journal mode this connection is actually in, folded to lower case.
///
/// `None` when it cannot be read at all, which is its own answer and not the same as a mode that is
/// merely not WAL.
pub(super) fn journal_mode(conn: &Connection) -> Option<String> {
    conn.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
        .ok()
        .map(|mode| mode.to_ascii_lowercase())
}

/// The column names of a table, in declaration order.
#[cfg(test)]
pub(super) fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, DbError> {
    // `pragma_table_info` takes the table name as a value, so this is still not string-interpolated
    // SQL despite the table being dynamic.
    let mut statement = conn.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
    let names = statement
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

/// Whether a table exists.
pub(super) fn has_table(conn: &Connection, name: &str) -> Result<bool, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
