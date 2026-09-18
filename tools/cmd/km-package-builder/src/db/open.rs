//! Opening a curation database, and everything that happens before the first query.
//!
//! Four public entry points and one private `prepare` behind all of them, because a database can be
//! reached four ways — opened, created, opened while saying so to a progress bar, or made in memory
//! for a test — and only one of those differences survives past the first line.
//!
//! **Most of this file is repair, and it is why an open is not instant.** `prepare` checks the version,
//! then runs the backfills: sort keys, language tags, the detected-name sweep. Each is keyed on a
//! revision, so a change to the table it depends on runs it again once; each is guarded so it costs
//! a single query on a database that has already had it done, and
//! each is separate rather than folded into the ladder because they rewrite *data* where a migration
//! rewrites *shape* — a distinction that matters when one of them is interrupted, since the guard is
//! what makes re-running it free.
//!
//! The `_for_test` escapes live here too. They are `#[cfg(test)]` to a method, so nothing outside a
//! test binary can reach them, and they are here rather than in `tests.rs` because each is a thin
//! wrapper over a private function this module owns.

use super::*;

impl Db {
    /// Opens an existing database in `root`, refusing if there is not one.
    ///
    /// Not creating one implicitly is a requirement, not caution: pointing the tool at the wrong
    /// folder should be an error, not an empty index that looks like the corpus is missing.
    pub fn open(root: &Path) -> Result<Self, DbError> {
        Self::open_saying(root, &|_| {})
    }

    /// Opens it, saying what it is doing at the one point where that takes long enough to matter.
    ///
    /// The twin exists rather than a parameter on [`Db::open`] because only one caller has anywhere
    /// to put the answer — the job behind the Open page — and every other call site, the tests
    /// included, would have gained a `&|_| {}` that says nothing about what it is doing.
    pub fn open_saying(root: &Path, phase: Phase<'_>) -> Result<Self, DbError> {
        let database = require_database(root)?;
        Self::prepare(Connection::open(database)?, root, phase)
    }

    /// Creates the database in `root` if it is absent, then opens it.
    ///
    /// An existing `.kmbuild` under any name is opened rather than a second one being made beside
    /// it — `--init` on a folder that already has one has always been harmless, and it stays that
    /// way now that the name is not fixed.
    pub fn create(root: &Path) -> Result<Self, DbError> {
        Self::create_saying(root, &|_| {})
    }

    /// Creates it if absent and opens it, saying what it is doing. See [`Db::open_saying`].
    pub fn create_saying(root: &Path, phase: Phase<'_>) -> Result<Self, DbError> {
        let database = database_in(root).unwrap_or_else(|| root.join(DATABASE_NAME));
        Self::prepare(Connection::open(database)?, root, phase)
    }

    /// Opens a database in memory, for tests.
    #[cfg(test)]
    pub fn open_in_memory(root: &Path) -> Result<Self, DbError> {
        Self::prepare(Connection::open_in_memory()?, root, &|_| {})
    }

    /// Whether this connection got write-ahead logging, which decides whether it may have a sibling.
    ///
    /// **A second connection is only safe in WAL.** The other modes are the ones where a writer
    /// excludes readers outright, so a page drawn through a second connection there would wait out
    /// `busy_timeout` and then answer "database is locked" — answered wrongly rather than answered
    /// late, which is worse than sharing one connection. An in-memory database is in this group and
    /// is the case every test takes.
    pub fn in_wal(&self) -> bool {
        journal_mode(&self.conn).as_deref() == Some("wal")
    }

    /// Opens a second connection on the same file, which may only read.
    ///
    /// **A page reads through one of these so that a scan cannot stop the tool.** The writing
    /// connection is held for a whole scan batch, and a request needing that same connection had to
    /// win a gap between batches — on a corpus-sized scan, a wait with no bound on it. WAL is what
    /// makes the pair safe: a reader sees the last committed state while a write is in progress, and
    /// neither waits for the other.
    ///
    /// It runs no migration, no backfill and no index build. [`Db::prepare`] does all of that, and
    /// [`Workspace::new`](crate::workspace::Workspace::new) opens this one afterwards, so the schema
    /// it sees is already the final one. A read-only connection could not change it in any case,
    /// which is the property that keeps the two from racing over the shape of the database.
    ///
    /// **A folder with no database is an error here, as it is for [`Db::open`]**, and so is one whose
    /// `-shm` file cannot be created — SQLite needs to write that to read a WAL database at all. Both
    /// are reported to the caller rather than swallowed, and `Workspace::new` is what decides that
    /// falling back to the writing connection beats refusing to open the folder.
    pub fn open_reading(root: &Path) -> Result<Self, DbError> {
        let database = require_database(root)?;
        let conn = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // First, before anything that can meet a lock, for the reason `prepare` gives at length: the
        // writer holds this file for a whole batch, and the default timeout of zero would turn that
        // into an immediate `SQLITE_BUSY` rather than the short wait it should be.
        let _ = conn.execute_batch("PRAGMA busy_timeout = 5000;");
        tune_reading(&conn);
        Ok(Self {
            conn,
            root: root.to_path_buf(),
            counts: Cell::new(None),
        })
    }

    fn prepare(conn: Connection, root: &Path, phase: Phase<'_>) -> Result<Self, DbError> {
        // **First, before anything that can meet a lock.** SQLite's default busy timeout is zero, so
        // every statement below this line would answer contention with an immediate `SQLITE_BUSY`
        // rather than a wait — and the first of them is the one that most needs to wait. Its own
        // statement rather than a line in `tune`'s batch, because a batch stops at its first failure
        // and this must not be reachable by one.
        let _ = conn.execute_batch("PRAGMA busy_timeout = 5000;");
        // Write-ahead logging, ignored rather than fatal: an in-memory database has no journal to
        // switch, and a corpus on a network share is one of the places SQLite refuses WAL.
        //
        // **A switch that does not take is said out loud.** It needs an exclusive lock, so a second
        // connection on the file can cost it — and the connection then runs in rollback-journal
        // mode, where a writer excludes readers outright and an ordinary page render comes back
        // "database is locked". `tune` reads the mode back for `synchronous`; this is the same
        // reading, reported.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Before `migrate`, so a step it runs gets the large page cache rather than the 2 MB default.
        // On a large corpus that is the difference between a rewrite being memory-bound and being
        // seek-bound.
        tune(&conn);
        // Before the schema batch, not after: the FTS triggers it recreates name `new.stem` and
        // `new.lyrics`, and SQLite rejects a trigger over a column that is not there yet.
        migrate(&conn, phase)?;
        // **Before the schema batch, because the batch is half the work being announced.** The
        // composite on `files` is an ordinary index and so lives in `schema.sql`, which runs first
        // and silently; the browse indexes are built afterwards in Rust. Announcing from inside
        // `create_browse_indexes` therefore let a corpus-sized index build finish before the banner
        // saying it was happening appeared. One check up here covers both.
        let missing = missing_indexes(&conn, HEAVY_INDEXES)?;
        let announce = !missing.is_empty() && counts_as_a_corpus(&conn)?;
        let started = Instant::now();
        if announce {
            // `say` and not `println!`. This pair was the last thing in the crate still printing
            // directly, and it is exactly the pair `km-console`'s own header names as the reason it
            // exists — `Db::prepare` is reached from the windowed build, where stdout may be a null
            // handle and `println!` therefore a panic rather than a lost line. It said so while
            // these two went on being the counter-example.
            km_console::say(format!(
                "  indexing    {} — once,\n\
                 \x20             and on a corpus this size on a spinning disk that is several\n\
                 \x20             minutes. Interrupting is safe; reopening carries on.",
                OpeningPhase::Indexing {
                    missing: missing.len(),
                }
                .in_english(),
            ));
        }
        // The same sentence to whoever has no console, which is the windowed build — and that is the
        // build most likely to be sitting through this, since it is what a double-click starts. Out
        // of one function with the `say` above so the two cannot come to disagree about *what* is
        // being built, only about how much room they have to say it.
        //
        // **Without `announce`'s second half.** That threshold holds a banner back from a terminal
        // somebody is watching over a folder of forty songs, where a block of three lines about
        // spinning disks is noise. The page has no such trade to make: the sentence it would print
        // instead is the one it is already showing, and the whole fault here is a sentence that does
        // not change.
        if !missing.is_empty() {
            phase(OpeningPhase::Indexing {
                missing: missing.len(),
            });
        }
        conn.execute_batch(include_str!("../schema.sql"))?;
        let db = Self {
            conn,
            root: root.to_path_buf(),
            counts: Cell::new(None),
        };
        // **A sentence before each of the repairs below, and it is the shape rather than any one of
        // them that matters.** Each is guarded and so costs a query on a database that has had it
        // done, but the one with work to do is minutes of it — and a step that says nothing leaves
        // the page on the sentence before it for exactly that long. A step that turns out to have
        // nothing to do corrects itself within a query, which is the cheaper of the two mistakes.
        //
        // **Before the browse indexes, not with the other backfills below them.** Eight of those
        // indexes are keyed on `sort_title`, so building them first would mean corpus-sized random
        // B-tree updates as this moves every entry, where building them after is eight sorted scans
        // of a table that is already right. It is safe this early because the index *this* one needs,
        // `songs_unfolded`, comes from `schema.sql`, which has already run.
        // Before the backfill, because it is what gives the backfill something to do: a fold table
        // that has moved makes every stored key stale, and this is what says so.
        db.invalidate_folded_keys()?;
        db.backfill_sort_keys(phase)?;
        // No sentence of its own: the one `missing_indexes` produced above names exactly this, and is
        // set before `schema.sql` runs because that file builds half of it.
        db.create_browse_indexes()?;
        phase(OpeningPhase::WorkingOutLanguage);
        db.backfill_language_tags()?;
        phase(OpeningPhase::TidyingText);
        db.clean_detected_text()?;
        // Before anything counts stale songs, so the Scan page's number is the rows a scan will
        // actually read. No sentence of its own: it is one statement per revision, seconds at most.
        db.promote_unreached_revisions()?;
        // Last, so it sees the final schema — a rebuilt `songs` and the browse indexes included. Only
        // when there are none: a second or later open of an unchanged corpus has nothing to learn, and
        // this is a startup path whose whole point is to stop doing work it does not need to do.
        //
        // The `missing` half is not an optimization but a correctness condition: an index with no
        // `sqlite_stat1` row is one the planner will not choose, so a database that already had
        // statistics and has just gained indexes needs them gathered again or the indexes are dead
        // weight — and the symptom of getting this wrong is the fix appearing not to work at all.
        if !missing.is_empty() || db.has_no_statistics() {
            // The largest single thing an open does on a real corpus, and it is one statement with
            // nothing inside it to count, so this is the only sentence it can offer.
            phase(OpeningPhase::GatheringStatistics);
            db.refresh_statistics();
        }
        phase(OpeningPhase::FoldingJournal);
        // **The journal goes back into the file here, not whenever the folder is closed.** Everything
        // above this line is a write, and a backfill with work to do leaves a `-wal` of about the size
        // of what it rewrote. That is paid eventually either way; the choice is when. Here it is
        // inside the window the Open page is already reporting, on the connection that wrote it, with
        // nobody reading. Left to `Workspace`'s `Drop` it lands on whichever thread releases the last
        // reference, which is a `wal_checkpoint(TRUNCATE)` of several hundred megabytes running
        // against the first page somebody asked for.
        //
        // Best-effort, like every other checkpoint here: a journal that could not be folded back is a
        // larger file, never a broken one.
        let _ = db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        // **After the analyze, not after the indexes.** The `ANALYZE` is the larger half of this on a
        // real corpus — it re-reads every index on both big tables, and adding six of them made it
        // bigger still — so a `done` printed before it left somebody watching a silent terminal for
        // several more minutes having just been told the work had finished. Worse than saying
        // nothing, which is why it is the whole window that is timed and not the part that was easy
        // to time.
        if announce {
            km_console::say(format!(
                "              done — {} seconds.",
                started.elapsed().as_secs()
            ));
        }
        // Not "done", which is the console's word for a line nothing will overwrite. This one
        // replaces a claim that is about to stop being true: what follows an open is publishing the
        // workspace and writing the recent list, and a page still promising several minutes of index
        // building through that is worse than one saying less.
        //
        // Outside `announce` for the reason the indexing sentence above is: a page that took the
        // cheap path through all of this still has one sentence on it, and *finishing* is the true
        // one to end on.
        phase(OpeningPhase::Finishing);
        if !missing.is_empty() {
            tracing::info!(
                built = missing.len(),
                seconds = started.elapsed().as_secs(),
                "indexes built and statistics gathered"
            );
        }
        Ok(db)
    }

    /// Keeps `sqlite_stat1` current enough for the planner to choose the browse indexes.
    ///
    /// **Not housekeeping — the browse indexes do not work without it.** Their key is an expression,
    /// so with no statistics SQLite falls back on built-in guesses, decides the `merged_into IS NULL`
    /// term is the selective one, and seeks `songs_merged` instead, sorting the result. Measured on
    /// a large table: 0.486 s guessing, 0.003 s once analyzed. Every database written before this
    /// existed has no `sqlite_stat1` at all.
    ///
    /// It is an explicit, **unbounded** `ANALYZE`, and both halves of that were got wrong first.
    ///
    /// `PRAGMA optimize` on its own is not enough: on SQLite 3.37 it writes nothing whatever for a
    /// table that has never been analyzed, so a build linking an older library would silently get the
    /// slow plan everywhere.
    ///
    /// **`PRAGMA analysis_limit` is not merely insufficient, it causes the very plan it was added to
    /// avoid.** Bounding the sampling does not make the statistics approximate; for a low-cardinality
    /// index it makes them wrong, in a direction. With `analysis_limit = 400`, `songs_merged` was
    /// recorded as *401 rows per distinct value of `merged_into`* when every row in a corpus has it
    /// NULL and the truth is the whole table. So the planner believed seeking `songs_merged`
    /// returned 401 rows and sorting them was free, chose it over `songs_browse_title_artist`, and
    /// sorted the whole corpus on every page. A bare `PRAGMA optimize` shares the flaw: it caps the
    /// limit at 2000 of its own accord, which understates this index just as badly.
    ///
    /// **What it costs depends entirely on whether the indexes are resident, and the two answers are
    /// minutes apart.** Warm, on a database with fewer indexes than this one now declares, a full
    /// `ANALYZE` measured about a second. At the end of a scan over a whole corpus — cold, against a
    /// disk the scan has just been reading — it measured **11 m 17 s**. The second figure is the one
    /// a scan pays, and [`Db::refresh_statistics`]'s caller holds the writing connection for all of
    /// it. Either way it earns its place: it takes the browse page far down the list from 4.21 s to
    /// 9.71 ms.
    ///
    /// So it runs in full, and only when something has actually changed: once on a database that has
    /// no statistics at all, and at the end of a scan — the only thing that changes the *shape* of
    /// this database rather than its contents.
    ///
    /// Best-effort: statistics that could not be gathered are a slower tool, never a broken one.
    pub fn refresh_statistics(&self) {
        let _ = self
            .conn
            .execute_batch("PRAGMA analysis_limit = 0; ANALYZE;");
    }

    /// Whether this database has never been measured for the query planner.
    pub(super) fn has_no_statistics(&self) -> bool {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'sqlite_stat1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            == 0
    }

    /// The indexes that make the browse page an index seek rather than a sort of the corpus.
    ///
    /// **Here rather than in `schema.sql` because two of their keys are still expressions**, and
    /// SQLite will only use an expression index when the query's expression matches the index's tree
    /// for tree. The query side of those two is generated by [`title_initial`] and [`eff_language`];
    /// writing the same expressions out again in `schema.sql` would be a second copy free to drift,
    /// and the drift would not fail — it would silently stop the planner using the index and put the
    /// tool back to sorting the whole corpus on every page. One definition, used both places, is the only
    /// version of this that stays true. (`browse_columns` carries the same argument for the same
    /// reason.)
    ///
    /// **Only a few of these are expression indexes, and that is deliberate.** Keying on
    /// `eff_title` and `eff_artist` is what makes one fragile in that way; the browse order keys on
    /// the stored `sort_title` and `sort_artist` columns — ordinary columns, which cannot drift from
    /// anything — because it has to fold accents and a fold has no SQL spelling. Losing the
    /// fragility with it is a second benefit rather than the reason.
    ///
    /// All of them are partial on `merged_into IS NULL`, which [`Filter::to_sql`] emits first and
    /// always, so every browse query implies the predicate and can use them.
    ///
    /// **There is one index per arm of [`Filter::order_by`], and the key mirrors that arm term for
    /// term** — including the leading `x IS NULL` the five "unrated last" sorts open with, and
    /// including each term's ASC/DESC. That leading term looks like something an index could not
    /// carry, which is what makes it worth stating plainly: SQLite indexes *expressions*, and
    /// `x IS NULL` is one, so the sort that pushes unrated songs to the bottom is as index-servable
    /// as the plain one. Writing the key this way is what let those sorts be fixed without changing
    /// what any of them puts on screen. Before this, six of the eight sorts read the whole filtered
    /// corpus into a temp B-tree on every page turn — on the measured corpus, tens of seconds a page
    /// against single-digit milliseconds, and worse the further in you were.
    ///
    /// The cost is real and lands on the scan rather than on browsing: eleven indexes to maintain
    /// per written row, on top of the nine in `schema.sql` and the two FTS tables. That is the right
    /// side to spend it on — a corpus is scanned rarely and browsed constantly.
    /// Every name here is also in [`HEAVY_INDEXES`], which is what `prepare` checks *before* any of
    /// this runs to decide whether to announce a pause and whether to gather statistics again. **A
    /// new index with no `sqlite_stat1` row is an index the planner will not use**, so the two lists
    /// agreeing is a correctness condition rather than tidiness; a `debug_assert!` below says so.
    pub(super) fn create_browse_indexes(&self) -> Result<(), DbError> {
        let letter = title_initial("");
        let language = eff_language("");
        let indexes = [
            (
                "songs_browse_title_artist",
                format!("ON songs({WITHIN_TITLE_KEY}) WHERE merged_into IS NULL"),
            ),
            (
                "songs_browse_letter_artist",
                format!("ON songs({letter}, {WITHIN_TITLE_KEY}) WHERE merged_into IS NULL"),
            ),
            (
                "songs_browse_suitability_artist",
                format!("ON songs(suitability DESC, {WITHIN_TITLE_KEY}) WHERE merged_into IS NULL"),
            ),
            (
                "songs_browse_sort_artist",
                "ON songs(sort_artist IS NULL, sort_artist, sort_title, id) \
                 WHERE merged_into IS NULL"
                    .to_owned(),
            ),
            (
                "songs_browse_language_artist",
                format!(
                    "ON songs({language} IS NULL, {language}, {WITHIN_TITLE_KEY}) \
                     WHERE merged_into IS NULL"
                ),
            ),
            (
                "songs_browse_user_score_artist",
                format!(
                    "ON songs(user_score IS NULL, user_score DESC, {WITHIN_TITLE_KEY}) \
                     WHERE merged_into IS NULL"
                ),
            ),
            // `schema.sql` already has a `songs(duration_ms)`, and it is not partial, so it cannot
            // combine with the `merged_into IS NULL` every browse query carries. This one can, and
            // it carries `id` so the tiebreak needs no sort either.
            (
                "songs_browse_duration",
                "ON songs(duration_ms DESC, id) WHERE merged_into IS NULL".to_owned(),
            ),
            // The eighth, and the one that could not exist until `file_count` was a column. The
            // same relationship to `schema.sql`'s `songs_file_count` that the entry above has to
            // `songs(duration_ms)`: that one is not partial and serves the filter under any sort,
            // this one carries the `merged_into IS NULL` every browse query implies, plus the
            // tiebreak terms, so the sort needs no pass of its own.
            (
                "songs_browse_copies",
                "ON songs(file_count DESC, suitability DESC, id) WHERE merged_into IS NULL"
                    .to_owned(),
            ),
            // The one whose key is nearly all sentinel, and it is worth pricing. On a corpus
            // somebody has just opened every entry reads `(1, NULL, <the title terms>)` -- a copy of
            // `songs_browse_title_artist` with a constant pair in front of it -- so this is another
            // corpus-sized B-tree, maintained on every row any write touches. What it buys is the
            // one order the browse bar could not offer at all: a curator coming back to a corpus
            // asking what they were working on.
            (
                "songs_browse_updated_artist",
                format!(
                    "ON songs(updated_at IS NULL, updated_at DESC, {WITHIN_TITLE_KEY}) \
                     WHERE merged_into IS NULL"
                ),
            ),
            // Leading on `first_seen` also serves the added-date filter's range under this sort.
            (
                "songs_browse_added_artist",
                format!("ON songs(first_seen DESC, {WITHIN_TITLE_KEY}) WHERE merged_into IS NULL"),
            ),
        ];

        // Asserted rather than assumed: `prepare` decides whether to announce a pause and whether to
        // gather statistics again by looking up `HEAVY_INDEXES` before any of this runs, and an index
        // added here but not named there would be built silently and then never used, because
        // nothing would re-analyze. That is a failure with no symptom except the tool being slow.
        debug_assert!(
            indexes.iter().all(|(name, _)| HEAVY_INDEXES.contains(name)),
            "every browse index must be named in HEAVY_INDEXES"
        );

        // Any `songs_browse_*` this build does not intend, dropped rather than left behind.
        //
        // **This is what makes changing an index key safe**, and it is worth stating plainly because
        // the alternative failed silently. Every `CREATE` below is `IF NOT EXISTS`, so an index whose
        // key changed would sit in an existing database under its old name holding its old key, the
        // query would match neither it nor anything else, and the browse page would go back to
        // sorting the whole corpus with nothing anywhere saying so. A written-down list of retired names
        // would work until somebody renamed one and did not add it; this one cannot be incomplete,
        // because the intended set *is* the list.
        //
        // Before the `CREATE`s rather than after, so a corpus does not have to hold two full sets of
        // browse indexes at once — on a large corpus that is hundreds of megabytes of peak disk. The
        // cost is that an open interrupted between the drop and the build leaves no browse indexes at
        // all, which the banner already covers: reopening carries on.
        for name in own_indexes(&self.conn, "songs")? {
            if name.starts_with("songs_browse_") && !indexes.iter().any(|(known, _)| *known == name)
            {
                self.conn
                    .execute_batch(&format!("DROP INDEX IF EXISTS {name}"))?;
            }
        }

        for (name, body) in &indexes {
            self.conn
                .execute_batch(&format!("CREATE INDEX IF NOT EXISTS {name} {body};"))?;
        }
        Ok(())
    }

    /// Folds the browse sort keys of every row still waiting for one.
    ///
    /// **Chunked, and the chunking is what makes it correct as well as bounded.** Each pass takes the
    /// next [`FOLD_CHUNK`] rows the `songs_unfolded` index still holds and writes them in a
    /// transaction of its own — which removes them from that index, so the next pass's `LIMIT` finds
    /// the next batch with no `OFFSET` and no cursor walking rows it is deleting out from under
    /// itself. A transaction per chunk is also what lets the write-ahead log drain rather than
    /// growing a second copy of the corpus beside the first.
    ///
    /// Chunks rather than one whole-corpus `Vec` because this one has a number to report: at corpus
    /// size on a spinning disk the pause is minutes, and `prepare` already has somewhere to say so.
    pub(super) fn backfill_sort_keys(&self, phase: Phase<'_>) -> Result<(), DbError> {
        let total: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM songs WHERE sort_title IS NULL",
            [],
            |row| row.get(0),
        )?;
        let total = usize::try_from(total).unwrap_or(0);
        if total == 0 {
            return Ok(());
        }
        let started = Instant::now();
        km_console::say(format!(
            "  folding     {total} titles for the browse order — once, and only for a\n\
             \x20             database written before it had one. Interrupting is safe."
        ));
        let mut done = 0;
        loop {
            phase(OpeningPhase::Folding { done, total });
            let transaction = self.conn.unchecked_transaction()?;
            let folded = refold_chunk(&transaction, FOLD_CHUNK)?;
            transaction.commit()?;
            if folded == 0 {
                break;
            }
            done += folded;
        }
        tracing::info!(
            songs = done,
            seconds = started.elapsed().as_secs(),
            "folded the browse sort keys; `Db::refold` keeps them right from here"
        );
        Ok(())
    }

    /// Brings the sort keys back into step after a write that changed a name.
    ///
    /// **Every path that writes `title`, `det_title`, `stem`, `artist` or `det_artist` calls this, in
    /// its own transaction, before that transaction commits.** It does not have to be told which rows
    /// it wrote: `songs_refold_update` has already set their keys to NULL, so `sort_title IS NULL` —
    /// served by the partial `songs_unfolded` index — *is* the list of rows that need one. That is
    /// the property worth having, because it is the one a hand-kept id list cannot give: a caller
    /// cannot pass the wrong set, and a `WHERE` clause that turned out to match more rows than its
    /// author expected is still handled.
    ///
    /// On a database with nothing to do it reads one page of an empty index, so calling it after a
    /// write that changed no name costs nothing worth measuring.
    ///
    /// No transaction of its own: it runs inside the caller's, so the keys and the names they were
    /// folded from commit together or not at all.
    pub(super) fn refold(conn: &Connection) -> Result<(), DbError> {
        while refold_chunk(conn, FOLD_CHUNK)? > 0 {}
        Ok(())
    }

    /// Blanks the browse keys when `km_song::text::fold` itself has changed.
    ///
    /// **This writes no keys**, and that is the whole design: it puts the rows back on the
    /// `songs_unfolded` index and lets [`Db::backfill_sort_keys`] — chunked, resumable, and already
    /// reporting its progress — do the folding. Blanking is one indexed-free `UPDATE`; folding
    /// A whole corpus is minutes, and there is no reason to have two things that know how to do it.
    ///
    /// Guarded by a revision rather than by anything derivable from the table, because a stale key is
    /// indistinguishable from a fresh one by inspection: `příliš` is exactly what the previous fold
    /// correctly produced. Only the number says otherwise.
    ///
    /// `songs_refold_update` fires on `title`, `det_title`, `stem`, `artist` and `det_artist`, so
    /// writing `sort_title` here does not set off a trigger that would write it again.
    pub(super) fn invalidate_folded_keys(&self) -> Result<(), DbError> {
        let want = km_song::text::FOLD_REVISION.to_string();
        if self.setting(FOLD_REVISION)?.as_deref() == Some(want.as_str()) {
            return Ok(());
        }
        // A database that has never recorded one is either brand new — nothing to blank — or was
        // written before the revision existed, in which case its keys really are from an older fold.
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute("UPDATE songs SET sort_title = NULL, sort_artist = NULL", [])?;
        transaction.commit()?;
        self.set_setting(FOLD_REVISION, &want)?;
        Ok(())
    }

    /// Fills `det_language_tag` from the stored detection, again whenever the mapping tables move.
    ///
    /// Reads only what is already stored — `det_language` and `det_encoding` — so a corpus of
    /// hundreds of thousands of songs costs one pass and one transaction, and **no file is opened**.
    /// That is the whole reason the mapped code is a column: recovering it from the sources means
    /// touching nothing on disk.
    ///
    /// Guarded by a settings flag holding [`km_kmpkg::LANGUAGE_TABLE_REVISION`], rather than by a
    /// partial index over the rows still waiting. Two reasons, and they pull the same way.
    /// Nearly every row has a tag to compute — the `@L` header is taken at face value — so an index
    /// over the pending set would cover most of the table and cost more than it saves. And the work
    /// list legitimately *grows* when the mapping tables change, which a boolean flag would freeze
    /// and a revision number handles for nothing: bump the constant and every song is reconsidered,
    /// exactly once.
    pub(super) fn backfill_language_tags(&self) -> Result<(), DbError> {
        let want = km_kmpkg::LANGUAGE_TABLE_REVISION.to_string();
        if self.setting(LANGUAGE_TAGS)?.as_deref() == Some(want.as_str()) {
            return Ok(());
        }

        let pending: Vec<(String, Option<String>, Option<String>)> = {
            let mut statement = self.conn.prepare(
                "SELECT id, det_language, det_encoding FROM songs
                 WHERE det_language IS NOT NULL OR det_encoding IS NOT NULL",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let resolved: Vec<(String, Option<&'static str>)> = pending
            .iter()
            .map(|(id, declared, encoding)| {
                let language = Language::detect(declared.as_deref(), encoding.as_deref());
                (id.clone(), language.map(Language::code))
            })
            .collect();

        tracing::info!(
            songs = resolved.len(),
            "working out the language of songs indexed by an earlier version"
        );
        let transaction = self.conn.unchecked_transaction()?;
        {
            let mut update =
                transaction.prepare("UPDATE songs SET det_language_tag = ?2 WHERE id = ?1")?;
            for (id, tag) in &resolved {
                update.execute(params![id, tag])?;
            }
        }
        transaction.commit()?;
        self.set_setting(LANGUAGE_TAGS, &want)?;
        Ok(())
    }

    /// Takes the names nobody can read out of `det_title` and `det_artist` on rows already indexed.
    ///
    /// **Two defects of one shape, and a re-scan reaches neither**, because it revisits only files
    /// that have *changed* and over a corpus scanned once that is none of them. Without this the
    /// rows already written keep what an earlier gate let through.
    ///
    /// The first was padding. A great deal of software writes MIDI text events as fixed-length
    /// fields, so track names arrive NUL-padded, and the gate was `!text.trim().is_empty()` --
    /// `str::trim` removes Unicode whitespace and not NUL. **A NUL sorts before every printable
    /// character**, so the 179 rows in the local corpus whose title was nothing else took the whole
    /// first page of a title-ordered browse, drawn as empty and unclickable links.
    ///
    /// The second is ornament: `====================`, `<>-<>-<>-<>`, `???` and `-` are what a
    /// corpus puts in a title meta event when the person typing it had no title to put there, and
    /// they sort to the front of that same first page. `km_song::clean_meta_name` is the gate for
    /// both now, and [`CLEANED_META_REVISION`] is what brings a database swept by the first spelling
    /// back for the second.
    ///
    /// **The check is in Rust, over every row, once.** No partial index can express *contains a
    /// control character*, and SQLite's own string functions are worse than unhelpful here: they
    /// take text as C strings, so `length`, `substr` and `trim` all stop at the first NUL and a SQL
    /// predicate would silently pass exactly the rows that are wrong. Reading every row takes
    /// about a second, so a `settings` key makes it a first-open cost rather than a per-open one.
    ///
    /// Only rows that actually change are written -- 1,467 of them for the padding pass on the
    /// real corpus -- so both full-text indexes and the three browse indexes do a trivial amount of
    /// work. The revision is set inside the same transaction, so a pass that was interrupted runs
    /// again rather than being skipped with the job half done.
    pub(super) fn clean_detected_text(&self) -> Result<(), DbError> {
        let want = CLEANED_META_REVISION.to_string();
        if self.setting(CLEANED_META)?.as_deref() == Some(want.as_str()) {
            return Ok(());
        }

        let pending: Vec<CleanedNames> = {
            let mut statement = self
                .conn
                .prepare("SELECT id, det_title, det_artist, stem FROM songs")?;
            let rows = statement.query_map([], |row| {
                Ok(CleanedNames {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    stem: row.get(3)?,
                })
            })?;
            rows.filter_map(|row| match row {
                Ok(row) => row.cleaned().map(Ok),
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?
        };

        let transaction = self.conn.unchecked_transaction()?;
        if !pending.is_empty() {
            tracing::info!(
                songs = pending.len(),
                "taking the names nobody can read out of titles written by an earlier version"
            );
            let mut update = transaction.prepare(
                "UPDATE songs SET det_title = ?2, det_artist = ?3, stem = ?4 WHERE id = ?1",
            )?;
            for row in &pending {
                update.execute(params![row.id, row.title, row.artist, row.stem])?;
            }
        }
        Self::refold(&transaction)?;
        self.set_setting(CLEANED_META, &want)?;
        transaction.commit()?;
        Ok(())
    }

    /// Folds the write-ahead log back into the database file and gives up the memory mapping.
    ///
    /// **This is the pause on the way out, and it is why it is called rather than left to happen.**
    /// SQLite checkpoints on the last connection's close whether or not anybody asks, and on a
    /// corpus this size that is seconds of writing on a slow disk. Left implicit it happened after
    /// `main` returned, with nothing on screen and no way to tell a busy tool from a wedged one.
    /// Called here, the wait has a sentence in front of it.
    ///
    /// Returns how many pages the log held, which is the number the wait is proportional to.
    ///
    /// Best-effort in both statements: a database that could not be checkpointed is one SQLite will
    /// checkpoint itself a moment later, and refusing to exit over it would be absurd.
    pub fn close(&self) -> u32 {
        let pages = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                row.get::<_, i64>(1)
            })
            .unwrap_or(0);
        // Dropping the mapping before the process tears down is the other half of the wait on a
        // large file, and unlike the checkpoint it is pure bookkeeping.
        let _ = self.conn.execute_batch("PRAGMA mmap_size = 0;");
        pages.max(0) as u32
    }

    /// The folder being curated.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs a statement, for tests that need to put the database into a state the code cannot.
    #[cfg(test)]
    pub fn execute_for_test(&self, sql: &str) -> Result<(), DbError> {
        self.conn.execute_batch(sql)?;
        Ok(())
    }

    /// Stamps a database as written by a build that does not exist yet.
    ///
    /// **It cannot be fabricated by editing tables**: what makes a database too new is the number
    /// alone, since the columns a later build added are exactly what this build has no way to write.
    /// One pragma is the whole fixture.
    #[cfg(test)]
    pub fn as_if_from_a_newer_build(&self) {
        self.conn
            .pragma_update(None, "user_version", super::migrate::SCHEMA_VERSION + 1)
            .expect("stamp a version this build does not understand");
    }

    /// Re-runs the language backfill, which `prepare` has already flagged as done.
    #[cfg(test)]
    pub fn backfill_language_tags_for_test(&self) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [LANGUAGE_TAGS])?;
        self.backfill_language_tags()
    }

    /// Re-runs the detected-name sweep, which `prepare` has already recorded as done.
    #[cfg(test)]
    pub fn clean_detected_text_for_test(&self) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [CLEANED_META])?;
        self.clean_detected_text()
    }

    /// Counts rows, for tests that need to look at a table nothing exposes.
    ///
    /// `lyrics_fts` is the case it exists for: what is *not* in that index matters as much as what
    /// is, and no query the tool runs can see an absence.
    #[cfg(test)]
    pub fn count_for_test(&self, sql: &str) -> Result<i64, DbError> {
        Ok(self.conn.query_row(sql, [], |row| row.get(0))?)
    }

    /// The query plan for a statement, for the tests that pin a *performance* property.
    ///
    /// A count-based test cannot tell an index seek from a scan of the whole corpus: both give the
    /// right answer, and only one of them does it in a tenth of a second. Every other test in this
    /// file would go on passing if the browse indexes stopped being used, which is exactly why this
    /// exists.
    ///
    /// **Indented by nesting, which is the part that matters.** `browse_columns` carries five
    /// correlated subqueries, and one of them sorts a single song's files — so a flat plan contains
    /// the words `USE TEMP B-TREE` whatever the outer query does, and a test reading it flat would
    /// either never pass or never fail. The indentation is what lets an assertion say *the corpus is
    /// not sorted* rather than *nothing anywhere is sorted*.
    #[cfg(test)]
    pub fn plan_for_test(&self, sql: &str, bindings: &[Binding]) -> Result<String, DbError> {
        let mut statement = self.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let rows: Vec<(i64, i64, String)> = statement
            .query_map(params_from_iter(bindings.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut depths: BTreeMap<i64, usize> = BTreeMap::new();
        depths.insert(0, 0);
        let mut lines = Vec::new();
        for (id, parent, detail) in rows {
            let depth = depths.get(&parent).copied().unwrap_or(0) + 1;
            depths.insert(id, depth);
            lines.push(format!("{}{detail}", "    ".repeat(depth - 1)));
        }
        Ok(lines.join("\n"))
    }

    /// What `ANALYZE` recorded about one table's indexes, for a measurement asking why the planner
    /// declined one.
    ///
    /// An index with no row here is one the planner will not choose, whatever it would cost, so this
    /// is the first thing to look at when an index that should obviously serve a query does not.
    #[cfg(test)]
    pub fn stats_for_test(&self, table: &str) -> Result<String, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT idx, stat FROM sqlite_stat1 WHERE tbl = ?1 ORDER BY idx")?;
        let rows = statement
            .query_map([table], |row| {
                Ok(format!(
                    "{} {}",
                    row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    row.get::<_, String>(1)?
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.join("\n"))
    }

    /// Reads a pragma back, for the test that pins the tuning in [`tune`].
    #[cfg(test)]
    pub fn pragma_for_test(&self, name: &str) -> Result<String, DbError> {
        Ok(self
            .conn
            .pragma_query_value(None, name, |row| row.get::<_, rusqlite::types::Value>(0))
            .map(|value| match value {
                rusqlite::types::Value::Integer(number) => number.to_string(),
                rusqlite::types::Value::Text(text) => text,
                other => format!("{other:?}"),
            })?)
    }

    /// The columns a table declares, in schema order.
    ///
    /// For the test that keeps `crate::backup`'s written-down field list honest against
    /// `schema.sql`. Wraps the existing [`table_columns`] rather than asking again: a second spelling
    /// of this question is a second thing to be wrong, and the whole point of the test is that
    /// nothing about the schema is written down twice.
    #[cfg(test)]
    pub fn columns_for_test(&self, table: &str) -> Result<Vec<String>, DbError> {
        table_columns(&self.conn, table)
    }
}
