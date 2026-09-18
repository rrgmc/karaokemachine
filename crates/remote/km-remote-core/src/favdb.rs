//! Favorites: folders, and the songs filed in them.
//!
//! **A favorite is a named list a song is in, and there is no other kind.** The same conclusion
//! `km-package-builder` reached in the `What a favorite is` decision, arrived at here for the same
//! reason: a star that sets a boolean has no answer to *which favorite?*, and a song could be a
//! favorite of nothing.
//!
//! **A file of its own, separate from the mirror**, and that separation is load-bearing rather than
//! tidy. A catalog refresh may legitimately throw the mirror away and rebuild it; a collection
//! somebody built up over a year must not be able to go with it. Two files means the destructive
//! operation cannot reach the precious one by accident. It is the same split, for the same reason,
//! that the Go remote keeps between `song.db` and `favorites.db`.
//!
//! **Song numbers, and what the song is — never what it is called.** Titles are joined from the
//! mirror when a folder is listed, so a folder does not quietly hold a title that has since been
//! corrected on the machine. `package_id` and `content_hash` sit beside the number because they are
//! not description but identity: a number carries a bank, a bank belongs to the machine rather than
//! to the package, and re-banking one renumbers every song in it. See `Songs::resolve`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use km_remote_pages::machine::{Favorites, FolderRow, Reconciliation, RemoteError, SongRef, codes};
use km_songcode::SongCode;
use rusqlite::{Connection, OptionalExtension, params};

use crate::mirror::{has_column, has_table};

/// The file the collection lives in.
pub const FAVORITES_FILE: &str = "favorites.sqlite";

/// What a folder is called when the first one is made for somebody.
const FIRST_FOLDER: &str = "Favorites";

/// The collection.
pub struct FavDb {
    conn: Connection,
}

impl FavDb {
    /// Opens or creates the collection.
    pub fn open(dir: &Path) -> Result<Self, rusqlite::Error> {
        let file = dir.join(FAVORITES_FILE);
        let label = file.display().to_string();
        Self::prepare(Connection::open(file)?, &label)
    }

    /// One in memory, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        Self::prepare(Connection::open_in_memory()?, ":memory:")
    }

    fn prepare(conn: Connection, file: &str) -> Result<Self, rusqlite::Error> {
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // **Before the batch, and that order is the whole point.** Everything in it is
        // `CREATE ... IF NOT EXISTS`, which is a no-op against a table that already exists and so
        // cannot tell a current one from an older one -- and `CREATE INDEX IF NOT EXISTS
        // favorite_song ON favorite(song_code)` would itself fail on a collection without the column.
        check_shape(&conn, file)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS folder (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 -- `COLLATE NOCASE UNIQUE`: two folders called `Party` and `party` are one folder
                 -- somebody has typed twice, and finding out later means merging them by hand.
                 name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
                 created_at TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE IF NOT EXISTS favorite (
                 folder_id INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
                 -- The song's **code**, as text, not the mirror's surrogate id. The mirror is thrown
                 -- away and rebuilt by every refresh, so keying on anything it assigns would empty
                 -- somebody's folders the first time they refreshed. The code is what the machine
                 -- and the person both call the song, and it survives.
                 song_code TEXT NOT NULL,
                 -- **What the song is, beside the number it is filed under.** The code above is
                 -- still the key and still what a page queues; these two are what find the song
                 -- again when the code has moved under it. A number carries a bank, the bank is
                 -- the machine's to assign rather than the package's, and re-banking a package
                 -- renumbers every song in it -- so a code that was right last week can be right
                 -- for somebody else's song this week, which is worse than being wrong.
                 --
                 -- Nullable, and both of them: a favorite `reconcile` has not reached yet has
                 -- neither, and a package whose manifest carries no hash never will. A
                 -- null means *unknown* and never *different*, so a resolver falls back to the
                 -- code rather than treating a missing hash as a mismatch. `Songs::resolve` is
                 -- where that is spelled out.
                 --
                 -- Filled in by `reconcile` from what the catalog says, not by whatever filed the
                 -- favorite: the mirror is the only thing here that knows, and it is refreshed
                 -- long after a folder is made.
                 package_id   TEXT,
                 content_hash TEXT,
                 added_at  TEXT NOT NULL DEFAULT (datetime('now')),
                 PRIMARY KEY (folder_id, song_code)
             );
             CREATE INDEX IF NOT EXISTS favorite_song ON favorite(song_code);",
        )?;
        Ok(Self { conn })
    }

    /// Makes the first folder, if no folder has ever existed.
    ///
    /// **Checked by "has there ever been one", not by "is there one called `Favorites`"** — the
    /// second would resurrect it for somebody who renamed it, once per start, forever.
    pub fn seed(&self) -> Result<(), rusqlite::Error> {
        let any: Option<i64> = self
            .conn
            .query_row("SELECT id FROM folder LIMIT 1", [], |row| row.get(0))
            .optional()?;
        if any.is_none() {
            self.conn.execute(
                "INSERT INTO folder (name) VALUES (?1)",
                params![FIRST_FOLDER],
            )?;
        }
        Ok(())
    }

    fn folders(&self) -> Result<Vec<FolderRow>, rusqlite::Error> {
        let mut statement = self.conn.prepare(
            "SELECT f.id, f.name, COUNT(v.song_code)
             FROM folder f LEFT JOIN favorite v ON v.folder_id = f.id
             GROUP BY f.id, f.name ORDER BY f.name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], read_folder)?;
        rows.collect()
    }

    fn folder(&self, id: i64) -> Result<Option<FolderRow>, rusqlite::Error> {
        self.conn
            .query_row(
                "SELECT f.id, f.name, COUNT(v.song_code)
                 FROM folder f LEFT JOIN favorite v ON v.folder_id = f.id
                 WHERE f.id = ?1 GROUP BY f.id, f.name",
                params![id],
                read_folder,
            )
            .optional()
    }

    /// Creates a folder, or finds the one already called this.
    ///
    /// Create-or-find rather than create-or-fail, so that filing a song into a new folder is one
    /// action even when the same folder was made a moment ago in another tab.
    fn ensure_folder(&self, name: &str) -> Result<(FolderRow, bool), rusqlite::Error> {
        if let Some(existing) = self
            .conn
            .query_row(
                "SELECT f.id, f.name, COUNT(v.song_code)
                 FROM folder f LEFT JOIN favorite v ON v.folder_id = f.id
                 WHERE f.name = ?1 COLLATE NOCASE GROUP BY f.id, f.name",
                params![name],
                read_folder,
            )
            .optional()?
        {
            return Ok((existing, false));
        }
        self.conn
            .execute("INSERT INTO folder (name) VALUES (?1)", params![name])?;
        let id = self.conn.last_insert_rowid();
        Ok((
            FolderRow {
                id,
                name: name.to_owned(),
                songs: 0,
            },
            true,
        ))
    }

    fn song_ids(&self, folder: i64) -> Result<Vec<SongCode>, rusqlite::Error> {
        // Most recently added first: a folder is a working list, and the thing somebody just put in
        // it is the thing they are most likely to want next.
        let mut statement = self.conn.prepare(
            "SELECT song_code FROM favorite WHERE folder_id = ?1 ORDER BY added_at DESC, song_code",
        )?;
        let rows = statement.query_map(params![folder], read_code)?;
        rows.collect()
    }

    /// The same rows as [`Self::song_ids`], carrying what each favorite knows about its song.
    ///
    /// Same order, and for the same reason.
    fn song_refs(&self, folder: i64) -> Result<Vec<SongRef>, rusqlite::Error> {
        let mut statement = self.conn.prepare(
            "SELECT song_code, package_id, content_hash FROM favorite
             WHERE folder_id = ?1 ORDER BY added_at DESC, song_code",
        )?;
        let rows = statement.query_map(params![folder], |row| {
            Ok(SongRef {
                code: read_code(row)?,
                package_id: row.get(1)?,
                content_hash: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Writes back what the catalog said these favorites turned out to be.
    ///
    /// **`OR REPLACE` on the move, and it has to be.** Moving a row to a number the same folder
    /// already holds is a real case rather than a hypothetical -- two favorites of two songs whose
    /// packages were merged into one, or a folder that already held the song under its new number --
    /// and a plain `UPDATE` would fail the primary key and abort the transaction over what is
    /// properly a merge. `OR REPLACE` collapses the pair, which is the same answer
    /// [`Self::add_songs`] gives when a song is filed twice.
    ///
    /// **`added_at` is carried across the move**, or a re-banking would reorder somebody's folder --
    /// the "most recently added first" that [`Self::song_ids`] promises is the one visible thing a
    /// repair must not disturb.
    ///
    /// **Every folder at once, not the one being drawn.** A song is commonly filed in several, and a
    /// re-banked package moved it in all of them — repairing only the folder somebody happened to
    /// open would leave the others to be found broken later, one at a time.
    ///
    /// One transaction, matching [`Self::add_songs`], and `unchecked_transaction` for its reason.
    fn reconcile(&self, rows: &[Reconciliation]) -> Result<(), rusqlite::Error> {
        let transaction = self.conn.unchecked_transaction()?;
        {
            let mut statement = transaction.prepare(
                "UPDATE OR REPLACE favorite
                    SET song_code = ?1, package_id = ?2, content_hash = ?3
                  WHERE song_code = ?4",
            )?;
            for row in rows {
                statement.execute(params![
                    row.now.to_string(),
                    &row.package_id,
                    &row.content_hash,
                    row.was.to_string(),
                ])?;
            }
        }
        transaction.commit()
    }

    /// Files a song, or takes it back out. Returns whether it is now in.
    fn toggle(&self, folder: i64, song: SongCode) -> Result<bool, rusqlite::Error> {
        let removed = self.conn.execute(
            "DELETE FROM favorite WHERE folder_id = ?1 AND song_code = ?2",
            params![folder, song.to_string()],
        )?;
        if removed > 0 {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO favorite (folder_id, song_code) VALUES (?1, ?2)",
            params![folder, song.to_string()],
        )?;
        Ok(true)
    }

    /// Files songs that are not already filed, and says how many that was.
    ///
    /// **`INSERT OR IGNORE` and nothing else**, which is what makes both the features above
    /// idempotent rather than carefully made so: a song already in this folder collides with the
    /// `(folder_id, song_code)` primary key and is skipped, so reading the same shared code twice or
    /// restoring the same backup twice is the same insert again rather than a case somebody had to
    /// handle.
    ///
    /// One transaction, so a folder cannot be left half-filled by a failure partway down a
    /// thousand-song restore. `unchecked_transaction` rather than `Connection::transaction`,
    /// because every other method on this type takes `&self`, and requiring
    /// `&mut self` for this one would put a mutable borrow through
    /// [`FavDbFavorites::blocking`] for no gain.
    ///
    /// The count comes from `execute`'s own answer per row — 1 where the row was written, 0 where it
    /// collided — rather than from a second query, which could disagree with the write if another
    /// tab acted in between.
    fn add_songs(&self, folder: i64, songs: &[SongCode]) -> Result<usize, rusqlite::Error> {
        let transaction = self.conn.unchecked_transaction()?;
        let mut added = 0usize;
        {
            let mut statement = transaction
                .prepare("INSERT OR IGNORE INTO favorite (folder_id, song_code) VALUES (?1, ?2)")?;
            for song in songs {
                added += statement.execute(params![folder, song.to_string()])?;
            }
        }
        transaction.commit()?;
        Ok(added)
    }

    fn folders_for_song(&self, song: SongCode) -> Result<Vec<i64>, rusqlite::Error> {
        let mut statement = self
            .conn
            .prepare("SELECT folder_id FROM favorite WHERE song_code = ?1")?;
        let rows = statement.query_map(params![song.to_string()], |row| row.get(0))?;
        rows.collect()
    }

    /// Which of these songs are filed anywhere.
    ///
    /// One query for a whole page rather than one per star — fifty round trips against one, and the
    /// shape `favdb.FavoritedIDs` landed on in the Go remote for the same reason.
    fn favorited(&self, songs: &[SongCode]) -> Result<HashSet<SongCode>, rusqlite::Error> {
        if songs.is_empty() {
            return Ok(HashSet::new());
        }
        let placeholders = (1..=songs.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql =
            format!("SELECT DISTINCT song_code FROM favorite WHERE song_code IN ({placeholders})");
        let mut statement = self.conn.prepare(&sql)?;
        let codes: Vec<String> = songs.iter().map(SongCode::to_string).collect();
        let rows = statement.query_map(rusqlite::params_from_iter(codes.iter()), read_code)?;
        rows.collect()
    }
}

/// Refuses a collection whose `favorite` table is not the shape this build reads.
///
/// **Refused and never converted or dropped, because this is the one file nothing can rebuild.** The
/// mirror beside it answers a changed shape by throwing itself away and fetching again, which is free
/// because a machine still holds what it is a copy of; nothing holds a copy of this one. A collection
/// in an older shape is left exactly as it is, and the open fails naming the file and the column it
/// lacks.
///
/// It has no version number to compare, so the shape is the number: the three columns every
/// statement here reads. A file with no `favorite` table yet is new, and `prepare` creates it.
fn check_shape(conn: &Connection, file: &str) -> Result<(), rusqlite::Error> {
    if !has_table(conn, "favorite")? {
        return Ok(());
    }
    for column in ["song_code", "package_id", "content_hash"] {
        if !has_column(conn, "favorite", column)? {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some(format!(
                    "{file} holds favorites in an older shape, with no `{column}` column, and this \
                     build reads only the current one; it was left exactly as it is"
                )),
            ));
        }
    }
    Ok(())
}

/// Reads a song code out of the text column favorites are keyed by.
///
/// A row this build cannot parse is **skipped by the callers' `collect`** only if it errors, so it
/// does not: an unreadable code reads as 0 and simply resolves to no song, which is the behavior a
/// favorite already has when the package it came from is uninstalled. The one thing that must not
/// happen is a whole folder failing to list because of one bad row.
fn read_code(row: &rusqlite::Row<'_>) -> rusqlite::Result<SongCode> {
    let text: String = row.get(0)?;
    Ok(text.parse().unwrap_or(SongCode::new(0)))
}

fn read_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<FolderRow> {
    Ok(FolderRow {
        id: row.get(0)?,
        name: row.get(1)?,
        songs: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
    })
}

/// The collection behind the remote's trait.
#[derive(Clone)]
pub struct FavDbFavorites {
    inner: Arc<Mutex<FavDb>>,
}

impl FavDbFavorites {
    /// Wraps an open collection.
    pub fn new(db: FavDb) -> Self {
        Self {
            inner: Arc::new(Mutex::new(db)),
        }
    }

    async fn blocking<T, F>(&self, work: F) -> Result<T, RemoteError>
    where
        F: FnOnce(&mut FavDb) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let mut guard = inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            work(&mut guard)
        })
        .await
        .map_err(|error| RemoteError::Failed(format!("the worker thread died: {error}")))?
        .map_err(translate)
    }
}

/// Turns a SQLite failure into something a page can say.
///
/// The one that matters is the unique-name constraint, which is not a fault but somebody typing a
/// name they have already used — and the picker has a place to show that, right under the box.
///
/// **Everything else is logged here and summarized on screen**, which is a change made after a
/// singer at a party was shown `no such column: v.song_code in SELECT f.id, f.name, COUNT(...)
/// ... at offset 27`. `RemoteError::Failed` is rendered into the failure page's hint verbatim, so
/// whatever this puts in it is what a phone reads out. A statement and a byte offset are the right
/// thing to have and the wrong thing to show: they belong in the log, where the person who can act
/// on them will look.
fn translate(error: rusqlite::Error) -> RemoteError {
    let message = error.to_string();
    if message.contains("UNIQUE constraint failed: folder.name") {
        return RemoteError::Refused(codes::FOLDER_NAME_TAKEN);
    }
    tracing::error!(%error, "the favorites database refused a statement");
    RemoteError::Failed("The favorites could not be read. The log says why.".to_owned())
}

#[async_trait::async_trait]
impl Favorites for FavDbFavorites {
    async fn folders(&self) -> Result<Vec<FolderRow>, RemoteError> {
        self.blocking(|db| db.folders()).await
    }

    async fn folder(&self, id: i64) -> Result<Option<FolderRow>, RemoteError> {
        self.blocking(move |db| db.folder(id)).await
    }

    async fn ensure_folder(&self, name: &str) -> Result<(FolderRow, bool), RemoteError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(RemoteError::Refused(codes::FOLDER_NEEDS_NAME));
        }
        self.blocking(move |db| db.ensure_folder(&name)).await
    }

    async fn rename_folder(&self, id: i64, name: &str) -> Result<(), RemoteError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(RemoteError::Refused(codes::FOLDER_NEEDS_NAME));
        }
        self.blocking(move |db| {
            db.conn.execute(
                "UPDATE folder SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
            Ok(())
        })
        .await
    }

    async fn delete_folder(&self, id: i64) -> Result<(), RemoteError> {
        // Refusing the last one is not paternalism: the star's whole flow is "pick a folder", and
        // with none there is nowhere for it to lead and no obvious way back to having one.
        let remaining: i64 = self
            .blocking(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM folder", [], |row| row.get(0))
            })
            .await?;
        if remaining <= 1 {
            return Err(RemoteError::Refused(codes::ONLY_FOLDER));
        }
        self.blocking(move |db| {
            db.conn
                .execute("DELETE FROM folder WHERE id = ?1", params![id])?;
            Ok(())
        })
        .await
    }

    async fn song_ids(&self, folder: i64) -> Result<Vec<SongCode>, RemoteError> {
        self.blocking(move |db| db.song_ids(folder)).await
    }

    async fn song_refs(&self, folder: i64) -> Result<Vec<SongRef>, RemoteError> {
        self.blocking(move |db| db.song_refs(folder)).await
    }

    async fn reconcile(&self, rows: &[Reconciliation]) -> Result<(), RemoteError> {
        // The ordinary case by a wide margin: a folder drawn against a mirror that has not moved
        // produces nothing to write, and opening a transaction to say so would take a write lock on
        // the collection every time somebody looked at a folder.
        if rows.is_empty() {
            return Ok(());
        }
        let rows = rows.to_vec();
        self.blocking(move |db| db.reconcile(&rows)).await
    }

    async fn toggle(&self, folder: i64, song: SongCode) -> Result<bool, RemoteError> {
        self.blocking(move |db| db.toggle(folder, song)).await
    }

    async fn add_songs(&self, folder: i64, songs: &[SongCode]) -> Result<usize, RemoteError> {
        // Nothing to open a transaction for, and nothing to check the folder for either: a merge
        // that found every scanned song already filed, or a restore of a folder whose songs this
        // catalog cannot show, both arrive here empty and both mean "no work", not "no folder".
        if songs.is_empty() {
            return Ok(0);
        }
        // **The folder first, in its own visit**, for the reason the trait method documents:
        // `OR IGNORE` covers the primary key and explicitly not the foreign key, so a folder deleted
        // in another tab would come back as a constraint failure — logged and read out as the
        // generic database fault by `translate` — where `NotFound` is a sentence a page already has.
        // `delete_folder` above reads before it writes for the same reason.
        if self.folder(folder).await?.is_none() {
            return Err(RemoteError::NotFound);
        }
        let songs = songs.to_vec();
        self.blocking(move |db| db.add_songs(folder, &songs)).await
    }

    async fn remove(&self, folder: i64, song: SongCode) -> Result<(), RemoteError> {
        self.blocking(move |db| {
            db.conn.execute(
                "DELETE FROM favorite WHERE folder_id = ?1 AND song_code = ?2",
                params![folder, song.to_string()],
            )?;
            Ok(())
        })
        .await
    }

    async fn folders_for_song(&self, song: SongCode) -> Result<Vec<i64>, RemoteError> {
        self.blocking(move |db| db.folders_for_song(song)).await
    }

    async fn favorited(&self, songs: &[SongCode]) -> Result<HashSet<SongCode>, RemoteError> {
        let songs = songs.to_vec();
        self.blocking(move |db| db.favorited(&songs)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> FavDb {
        let db = FavDb::open_in_memory().expect("open");
        db.seed().expect("seed");
        db
    }

    /// A collection in an older shape is refused, naming the file and the column it lacks.
    ///
    /// **This is the one file nothing can rebuild**, so the check runs before anything is written:
    /// the refusal is the whole of what the open does to it.
    #[test]
    fn a_collection_in_an_older_shape_is_refused_and_left_alone() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE folder (id INTEGER PRIMARY KEY AUTOINCREMENT,
                                  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                                  created_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE favorite (folder_id INTEGER NOT NULL,
                                    song_id INTEGER NOT NULL,
                                    added_at TEXT NOT NULL DEFAULT (datetime('now')),
                                    PRIMARY KEY (folder_id, song_id));
             INSERT INTO folder (name) VALUES ('Party');
             INSERT INTO favorite (folder_id, song_id) VALUES (1, 1001);",
        )
        .expect("an older collection");

        let said = match FavDb::prepare(conn, "favorites.sqlite") {
            Ok(_) => panic!("an older collection must not open"),
            Err(error) => error.to_string(),
        };
        assert!(said.contains("favorites.sqlite"), "{said}");
        assert!(
            said.contains("song_code"),
            "it names what is missing: {said}"
        );
    }

    /// The backfill half of `reconcile`: a favorite that resolved where it stood picks up what it
    /// is, so the repair is armed long before anything needs repairing.
    #[test]
    fn reconciling_fills_in_what_a_favorite_turned_out_to_be() {
        let db = db();
        db.add_songs(1, &[SongCode::new(1001)]).expect("file");
        db.reconcile(&[Reconciliation {
            was: SongCode::new(1001),
            now: SongCode::new(1001),
            package_id: Some("vol1".to_owned()),
            content_hash: Some("aaa".to_owned()),
        }])
        .expect("reconcile");
        let refs = db.song_refs(1).expect("refs");
        assert_eq!(refs[0].package_id.as_deref(), Some("vol1"));
        assert_eq!(refs[0].content_hash.as_deref(), Some("aaa"));
    }

    /// The repair half: a re-banked package moved the song, and the row moves with it.
    ///
    /// **Every folder holding it, not just the one being looked at** — a song is commonly filed in
    /// several, and repairing one at a time would leave the others to be found broken later.
    #[test]
    fn reconciling_refiles_a_moved_song_in_every_folder_holding_it() {
        let db = db();
        let (other, _) = db.ensure_folder("Quiet").expect("folder");
        db.add_songs(1, &[SongCode::new(1001)]).expect("file");
        db.add_songs(other.id, &[SongCode::new(1001)])
            .expect("file");

        db.reconcile(&[Reconciliation {
            was: SongCode::new(1001),
            now: SongCode::new(2001),
            package_id: Some("vol1".to_owned()),
            content_hash: Some("aaa".to_owned()),
        }])
        .expect("reconcile");

        assert_eq!(
            db.folders_for_song(SongCode::new(2001)).expect("folders"),
            vec![1, other.id],
            "both folders now hold the new number"
        );
        assert!(
            db.folders_for_song(SongCode::new(1001))
                .expect("folders")
                .is_empty(),
            "and none of them holds the old one"
        );
    }

    /// Moving a row onto a number the same folder already holds is a merge, not a failure —
    /// `UPDATE OR REPLACE` rather than a plain `UPDATE`, which would abort the whole transaction on
    /// the primary key.
    #[test]
    fn refiling_onto_a_number_a_folder_already_holds_merges_rather_than_failing() {
        let db = db();
        db.add_songs(1, &[SongCode::new(1001), SongCode::new(2001)])
            .expect("file");
        db.reconcile(&[Reconciliation {
            was: SongCode::new(1001),
            now: SongCode::new(2001),
            package_id: Some("vol1".to_owned()),
            content_hash: Some("aaa".to_owned()),
        }])
        .expect("a merge, not a constraint failure");
        assert_eq!(
            db.song_ids(1).expect("songs"),
            vec![SongCode::new(2001)],
            "the two collapse into one"
        );
    }

    /// A database fault is not something to read out to a singer. See [`translate`].
    #[test]
    fn a_database_fault_is_logged_rather_than_read_out_to_a_singer() {
        let db = db();
        let error = db
            .conn
            .prepare("SELECT nonesuch FROM favorite")
            .expect_err("no such column");
        let raw = error.to_string();
        match translate(error) {
            RemoteError::Failed(message) => {
                assert!(!message.contains("SELECT"), "{message}");
                assert!(!message.contains("nonesuch"), "{message}");
                assert_ne!(message, raw);
            }
            other => panic!("{other:?}"),
        }

        let taken = db
            .conn
            .execute("INSERT INTO folder (name) VALUES ('Favorites')", [])
            .expect_err("the name is taken");
        assert!(
            matches!(
                translate(taken),
                RemoteError::Refused(codes::FOLDER_NAME_TAKEN)
            ),
            "a name somebody has already used is a code the page has a sentence for"
        );
    }

    #[test]
    fn the_first_folder_is_made_once_and_a_rename_does_not_bring_it_back() {
        let db = db();
        let folders = db.folders().expect("folders");
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, FIRST_FOLDER);

        db.conn
            .execute("UPDATE folder SET name = 'Party'", [])
            .expect("rename");
        db.seed().expect("seed again");

        let folders = db.folders().expect("folders");
        assert_eq!(folders.len(), 1, "seeding again must not resurrect it");
        assert_eq!(folders[0].name, "Party");
    }

    #[test]
    fn a_song_goes_in_and_comes_back_out_with_the_same_tap() {
        let db = db();
        let folder = db.folders().expect("folders")[0].id;
        assert!(db.toggle(folder, SongCode::new(1001)).expect("in"));
        assert_eq!(db.song_ids(folder).expect("ids"), vec![SongCode::new(1001)]);
        assert!(!db.toggle(folder, SongCode::new(1001)).expect("out"));
        assert!(db.song_ids(folder).expect("ids").is_empty());
    }

    #[test]
    fn a_song_can_be_in_several_folders() {
        let db = db();
        let (a, _) = db.ensure_folder("Party").expect("folder");
        let (b, _) = db.ensure_folder("Quiet").expect("folder");
        db.toggle(a.id, SongCode::new(1001)).expect("file");
        db.toggle(b.id, SongCode::new(1001)).expect("file");
        let mut folders = db.folders_for_song(SongCode::new(1001)).expect("folders");
        folders.sort_unstable();
        assert_eq!(folders, vec![a.id, b.id]);
    }

    /// The number a page reports is the number that was *new*, not the number offered.
    #[test]
    fn adding_songs_reports_only_the_ones_that_were_new() {
        let db = db();
        let folder = db.folders().expect("folders")[0].id;
        db.toggle(folder, SongCode::new(1001)).expect("file");

        let added = db
            .add_songs(
                folder,
                &[
                    SongCode::new(1001),
                    SongCode::new(1002),
                    SongCode::new(1003),
                ],
            )
            .expect("add");
        assert_eq!(added, 2, "1001 was already filed and must not be counted");
        assert_eq!(db.folder(folder).expect("folder").expect("there").songs, 3);
    }

    /// The property both features rest on: a second run is the same insert again.
    #[test]
    fn adding_the_same_songs_twice_changes_nothing_the_second_time() {
        let db = db();
        let folder = db.folders().expect("folders")[0].id;
        let songs = [SongCode::new(1001), SongCode::new(2005)];

        assert_eq!(db.add_songs(folder, &songs).expect("add"), 2);
        assert_eq!(
            db.add_songs(folder, &songs).expect("add again"),
            0,
            "reading the same code twice, or restoring the same file twice, must add nothing"
        );
        assert_eq!(db.folder(folder).expect("folder").expect("there").songs, 2);
    }

    /// An add-only write still cannot take anything away, which is the guarantee stated as a test.
    #[test]
    fn adding_songs_never_removes_one_that_was_already_there() {
        let db = db();
        let folder = db.folders().expect("folders")[0].id;
        db.toggle(folder, SongCode::new(1001)).expect("file");

        db.add_songs(folder, &[SongCode::new(2005)]).expect("add");
        assert!(
            db.song_ids(folder)
                .expect("ids")
                .contains(&SongCode::new(1001)),
            "a song the incoming set did not mention has to survive"
        );
    }

    /// `OR IGNORE` does not cover a foreign key, so the folder is checked before the insert — see
    /// the trait method. Asserted at the storage layer too, because this is the statement that
    /// would otherwise fail on its first row and be read out as a database fault.
    #[tokio::test]
    async fn adding_songs_to_a_folder_that_is_gone_is_refused_rather_than_partly_done() {
        let favorites = FavDbFavorites::new(db());
        let error = favorites
            .add_songs(9_999, &[SongCode::new(1001)])
            .await
            .expect_err("no such folder");
        assert!(matches!(error, RemoteError::NotFound), "{error:?}");
    }

    /// A merge that found nothing new, and a restore this catalog could show none of, both arrive
    /// empty — and neither is a missing folder.
    #[tokio::test]
    async fn adding_nothing_is_not_an_error_even_for_a_folder_that_is_gone() {
        let favorites = FavDbFavorites::new(db());
        assert_eq!(favorites.add_songs(9_999, &[]).await.expect("no work"), 0);
    }

    /// `added_at` defaults per row, so a bulk add must not flatten what was already filed into one
    /// instant — that would destroy `song_ids`' "most recently added first".
    #[test]
    fn adding_songs_leaves_the_order_of_what_was_already_filed_alone() {
        let db = db();
        let folder = db.folders().expect("folders")[0].id;
        db.conn
            .execute(
                "INSERT INTO favorite (folder_id, song_code, added_at) VALUES (?1, '1001', '2026-01-01 00:00:00')",
                params![folder],
            )
            .expect("an older row");

        db.add_songs(folder, &[SongCode::new(2005)]).expect("add");
        let ids = db.song_ids(folder).expect("ids");
        assert_eq!(
            ids,
            vec![SongCode::new(2005), SongCode::new(1001)],
            "the new song is the most recent; the old one keeps its date"
        );
    }

    /// Two folders called `Party` and `party` are one folder somebody typed twice.
    #[test]
    fn a_folder_name_is_matched_without_regard_to_case() {
        let db = db();
        let (first, created) = db.ensure_folder("Party").expect("folder");
        assert!(created);
        let (again, created) = db.ensure_folder("party").expect("folder");
        assert!(!created, "create-or-find, not create-or-fail");
        assert_eq!(first.id, again.id);
    }

    #[test]
    fn a_folder_counts_what_is_in_it() {
        let db = db();
        let (folder, _) = db.ensure_folder("Party").expect("folder");
        db.toggle(folder.id, SongCode::new(1)).expect("file");
        db.toggle(folder.id, SongCode::new(2)).expect("file");
        let listed = db.folder(folder.id).expect("folder").expect("there");
        assert_eq!(listed.songs, 2);
    }

    /// One query for a page of rows rather than one per star.
    #[test]
    fn a_whole_page_of_stars_is_answered_at_once() {
        let db = db();
        let folder = db.folders().expect("folders")[0].id;
        db.toggle(folder, SongCode::new(2)).expect("file");
        db.toggle(folder, SongCode::new(4)).expect("file");
        let starred = db
            .favorited(&[
                SongCode::new(1),
                SongCode::new(2),
                SongCode::new(3),
                SongCode::new(4),
                SongCode::new(5),
            ])
            .expect("starred");
        assert_eq!(starred, HashSet::from([SongCode::new(2), SongCode::new(4)]));
        assert!(db.favorited(&[]).expect("none").is_empty());
    }

    #[test]
    fn deleting_a_folder_takes_what_was_filed_in_it() {
        let db = db();
        let (folder, _) = db.ensure_folder("Party").expect("folder");
        db.toggle(folder.id, SongCode::new(1001)).expect("file");
        db.conn
            .execute("DELETE FROM folder WHERE id = ?1", params![folder.id])
            .expect("delete");
        assert!(
            db.folders_for_song(SongCode::new(1001))
                .expect("folders")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_last_folder_cannot_be_deleted() {
        let favorites = FavDbFavorites::new(db());
        let folders = favorites.folders().await.expect("folders");
        let error = favorites
            .delete_folder(folders[0].id)
            .await
            .expect_err("refused");
        assert!(
            matches!(error, RemoteError::Refused(codes::ONLY_FOLDER)),
            "{error:?}"
        );
    }

    /// Not a fault — somebody typed a name they have already used, and the picker has a place to say
    /// so right under the box.
    #[tokio::test]
    async fn a_blank_folder_name_is_refused_with_something_worth_reading() {
        let favorites = FavDbFavorites::new(db());
        let error = favorites.ensure_folder("   ").await.expect_err("refused");
        assert!(
            matches!(error, RemoteError::Refused(codes::FOLDER_NEEDS_NAME)),
            "{error:?}"
        );
    }
}
