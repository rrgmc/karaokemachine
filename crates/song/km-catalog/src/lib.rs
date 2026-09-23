//! The catalog of installed song packages.
//!
//! SQLite with FTS5, bundled so it behaves identically on every platform including Android. Packages
//! are registered by path and read in place; this holds only the index.
//!
//! Two decisions worth stating. **A song is identified by its number**, `UNIQUE (number)`, because
//! that is how a singer asks for it — so uniqueness is the database's job rather than something
//! application code remembers to check. And **a number carries its package inside it**: it is
//! `bank * 1000 + slot`, where the slot is what the package numbered the song and the bank is the
//! block of a thousand this machine put that package in.
//!
//! The second is what makes the first cheap. `UNIQUE (bank)` on `packages` means two packages are
//! never in the same thousand, so **two songs cannot share a number** — a collision is impossible
//! rather than something the install looks for and refuses. Both hazards a refusal would exist to
//! avoid are real — silently renumbering changes the identity of somebody's songs and silently
//! overwriting loses them — and neither is reachable through a number.

pub mod search;

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::Path;

use km_kmpkg::{Package, SongEntry};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, params};

pub use crate::search::{SearchQuery, SortOrder, fts_match_query};

// Re-exported so callers can name a song's kind without depending on `km-kmpkg` directly: the
// catalog is where most of them meet it.
pub use km_kmpkg::SongKind;
pub use km_songcode::SongCode;

/// Why a catalog operation failed.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    /// The database could not be opened or queried.
    #[error("catalog database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// Another package is already in the bank this one was given.
    ///
    /// **One fault and one remedy, rather than a list of clashing song numbers.** The bank is what
    /// collides, so what a caller is told is *put this package in a different bank* — not two
    /// thousand lines saying the same thing one number at a time.
    #[error("bank {bank} already belongs to the package {owner}")]
    BankTaken {
        /// The bank asked for.
        bank: u16,
        /// The package that is in it.
        owner: String,
    },
    /// The bank asked for is not one.
    #[error("bank {bank} is above the highest bank, {}", km_songcode::MAX_BANK)]
    BadBank {
        /// The bank asked for.
        bank: u16,
    },
    /// Bank 0 was asked for, and it is not a package's to take.
    ///
    /// **Separate from [`LibraryError::BadBank`] rather than folded into it**, and it carries no
    /// bank because there is only one: a number above the last bank is a caller with arithmetic
    /// wrong, where this is a caller asking for the one block that is spoken for. One sentence
    /// cannot say both without saying neither.
    #[error("bank 0 is the machine's own and cannot hold a package")]
    BankReserved,
    /// The package's path could not be resolved to something storable.
    #[error("could not resolve the path of {0}")]
    BadPath(String),
}

/// A package as installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    /// Stable package identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Where the archive lives.
    pub path: String,
    /// How many songs it contributed.
    pub song_count: usize,
    /// When it was installed, ISO-8601.
    pub installed_at: String,
    /// The block of a thousand its songs are dialled in.
    pub bank: u16,
}

/// A song as the catalog knows it.
///
/// `PartialEq` without `Eq` since [`Self::loudness_lufs`] arrived: a measurement is a float, and a
/// float is not `Eq`. Nothing compared these for total equality — the derive was there because every
/// field in the struct happened to allow it.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogSong {
    /// The code a singer dials: this package's bank and the song's slot in it, as one number.
    pub number: SongCode,
    /// Which package it came from.
    pub package_id: String,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// Language tag.
    pub language: Option<String>,
    /// Whether this is a MIDI song or a video song.
    pub kind: SongKind,
    /// For a MIDI song, a path inside the archive; for a video song, a file name inside the
    /// package's media folder.
    pub file: String,
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// Encoding to decode lyrics with.
    pub lyric_encoding: Option<String>,
    /// Transposition to apply by default.
    pub default_transpose: i8,
    /// Whether the machine plays this song and draws none of its words.
    ///
    /// Read at song start, which is why it is here rather than reachable only through the package —
    /// the same reason [`Self::fixes`] is.
    pub lyrics_hidden: bool,
    /// The corrections in force on the song's own MIDI events.
    ///
    /// Read at song start, which is why it is here rather than reachable only through the package —
    /// the same reason [`Self::loudness_lufs`] is, and it takes the same consequence of moving
    /// `catalog_version`.
    pub fixes: Vec<km_fixes::Fix>,
    /// The melody channel, or `None` when detection abstained.
    pub melody_channel: Option<u8>,
    /// The 0-10 suitability score.
    pub suitability: Option<u8>,
    /// Hash of the MIDI bytes.
    pub content_hash: Option<String>,
    /// The song's first line or two, as its package recorded them.
    ///
    /// Empty for a video or MP3+G song, whose words are pixels, and for every song of a package
    /// built before packages carried this — the catalog can only hold what the manifest says.
    pub lyric_preview: Vec<String>,
    /// What somebody filed this song under, sorted — see `km_kmpkg::Tag`.
    ///
    /// Empty is the ordinary case rather than a gap: nothing detects a tag, so a song has one only
    /// because a person said so in the builder.
    pub tags: Vec<String>,
    /// How loud the song's audio was measured to be, in LUFS, when a package measured it.
    ///
    /// `None` for every MIDI song — it is the reference the other kinds are levelled to — and for
    /// every song of a package built before levelling existed. The machine turns this into a gain at
    /// song start; `None` means gain 1.0, which is how every song played before.
    ///
    /// Only the loudness, not the true peak the manifest also carries: the machine never reads a
    /// peak, because levelling only attenuates and no attenuation can clip.
    pub loudness_lufs: Option<f32>,
}

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// The package's identifier.
    pub package_id: String,
    /// The package's human-readable name, as its manifest gives it.
    ///
    /// Carried beside the id because an id is sixteen hexadecimal characters and every surface a
    /// person reads shows the name — see `A package's id is generated, not typed` in
    /// `docs/decisions/packaging.md`. Empty where a manifest names nothing, which is the caller's
    /// cue to fall back to the id.
    pub package_name: String,
    /// Songs added.
    pub songs_added: usize,
    /// Whether this replaced an earlier install of the same package.
    pub replaced_existing: bool,
    /// Songs whose content matches a song already in the catalog under a different number.
    ///
    /// Reported rather than refused: two packages legitimately containing the same recording is a
    /// catalog smell the owner should see, not an error that blocks an install.
    pub duplicate_content: Vec<DuplicateContent>,
}

/// The same recording under two codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateContent {
    /// The code just installed.
    pub number: SongCode,
    /// The code that already had this content.
    pub existing_number: SongCode,
    /// The package that number belongs to.
    pub existing_package: String,
}

/// The song catalog.
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Opens or creates a catalog at a path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LibraryError> {
        let conn = Connection::open(path.as_ref())?;
        Self::prepare(conn)
    }

    /// Opens a catalog in memory, for tests and for a throwaway index.
    pub fn open_in_memory() -> Result<Self, LibraryError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self, LibraryError> {
        // Write-ahead logging so a search from the display thread is not blocked by an install.
        // Ignored rather than fatal: an in-memory database has no journal to switch.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Before the schema batch, not after: the batch below cannot tell a current table from an
        // older one. See [`prepare_existing`].
        prepare_existing(&conn)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        // **After the batch, because a brand-new catalog has no `meta` table until it runs.**
        // `prepare_existing` keeps this current for any catalog that had songs in it, so the only
        // case that reaches here unwritten is a file being created or rebuilt now — whose rows will
        // be folded by the current table as they are installed. `OR IGNORE` is what keeps this from
        // overwriting the answer `prepare_existing` already gave.
        conn.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('fold_version', ?1)",
            [fold_version()],
        )?;
        Ok(Self { conn })
    }

    /// Number of songs in the catalog.
    pub fn song_count(&self) -> Result<usize, LibraryError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM songs", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Number of installed packages.
    ///
    /// Beside [`Self::packages`] rather than derived from it, because the two answer different
    /// questions: that one hands back seven owned columns per package for a caller that is going to
    /// list them, and this one is a count for a caller that is going to draw a sentence. The display
    /// thread reads it once a catalog change, so allocating a `Vec` to call `.len()` on would be
    /// paid for nothing.
    pub fn package_count(&self) -> Result<usize, LibraryError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// How many times the catalog has changed.
    ///
    /// Bumped inside the transaction of every install and uninstall that **changed something**, so it
    /// moves if and only if the set of songs did. A client that mirrors this catalog stores the
    /// number it mirrored and re-downloads only when it differs — see `songs/export` in `km-api`.
    ///
    /// The "changed something" is load-bearing rather than a nicety, and it is why an install
    /// compares a digest of the package's rows across itself. `install_startup_packages` reinstalls
    /// every configured package at **every start** — deliberately, so that a package rebuilt with a
    /// new column fills it — so a counter that moved on each install would move on each start of the
    /// machine, and every mirror in the house would re-download a catalog that had not changed.
    ///
    /// Monotonic within one catalog file and meaningless between two, which is why the remote
    /// stores the machine's instance id beside it: the same number against a different machine says
    /// nothing at all.
    pub fn catalog_version(&self) -> Result<u64, LibraryError> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'catalog_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        // A catalog written before this column existed has no row, and zero is the honest answer:
        // it has changed zero times *that anything recorded*, so a mirror re-reads once and then
        // tracks it from there.
        Ok(raw.and_then(|value| value.parse().ok()).unwrap_or(0))
    }

    /// Bumps the counter, inside whatever transaction is already open.
    fn bump_version(transaction: &rusqlite::Transaction<'_>) -> Result<(), LibraryError> {
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES ('catalog_version', '1')
             ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
            [],
        )?;
        Ok(())
    }

    /// A digest of every song row this package contributes, in code order.
    ///
    /// Taken before and after an install so [`Library::bump_version`] moves only when the catalog
    /// a mirror would download actually differs. It selects through [`SONG_COLUMNS`] — the very list
    /// [`Library::export_after`] exports — so the two cannot drift into disagreeing about what
    /// "changed" means, and *both* digests come from this one function, so there is no
    /// manifest-versus-database normalization for the comparison to get wrong.
    ///
    /// Columns are read as raw `ValueRef` rather than typed, which keeps this indifferent to the
    /// column list: adding a column to `SONG_COLUMNS` extends the digest with no edit here. Each row
    /// is terminated, so two adjacent columns cannot hash the same as one longer one.
    ///
    /// The hasher is `DefaultHasher`, whose output Rust does not promise to keep stable between
    /// releases. That is deliberate and safe: the two values compared are produced by one process
    /// inside one transaction, and neither is stored, sent, nor compared with one from anywhere
    /// else.
    fn package_digest(
        transaction: &rusqlite::Transaction<'_>,
        package_id: &str,
    ) -> Result<u64, LibraryError> {
        let mut statement = transaction.prepare(&format!(
            "SELECT {SONG_COLUMNS} FROM songs WHERE package_id = ?1 ORDER BY number"
        ))?;
        let mut hasher = DefaultHasher::new();
        let mut rows = statement.query(params![package_id])?;
        while let Some(row) = rows.next()? {
            for index in 0..row.as_ref().column_count() {
                match row.get_ref(index)? {
                    ValueRef::Null => hasher.write_u8(0),
                    ValueRef::Integer(value) => {
                        hasher.write_u8(1);
                        hasher.write_i64(value);
                    }
                    ValueRef::Real(value) => {
                        hasher.write_u8(2);
                        hasher.write_u64(value.to_bits());
                    }
                    ValueRef::Text(value) => {
                        hasher.write_u8(3);
                        hasher.write(value);
                    }
                    ValueRef::Blob(value) => {
                        hasher.write_u8(4);
                        hasher.write(value);
                    }
                }
            }
            hasher.write_u8(0xff);
        }
        Ok(hasher.finish())
    }

    /// One page of the whole catalog, in number order, for a client mirroring it.
    ///
    /// **Keyset paging, never `OFFSET`.** SQLite walks an offset row by row, so paging a six-figure
    /// catalog with `LIMIT/OFFSET` costs time proportional to the square of its size, and does it
    /// while somebody is waiting for their song list. `after` is the last number of the previous
    /// page, so each query is an index seek.
    ///
    /// Separate from [`search`](Self::search) rather than a big `limit` on it, because
    /// [`MAX_LIMIT`](search::MAX_LIMIT) is a deliberate cap on a *search* — a page a person reads —
    /// and lifting it there to serve a mirror would lift it for every caller.
    pub fn export_after(
        &self,
        after: Option<SongCode>,
        limit: usize,
    ) -> Result<Vec<CatalogSong>, LibraryError> {
        // One comparison against `UNIQUE (number)`, so the cursor is one index seek. A code split
        // across `(prefix, number)` needs a **row-value** comparison here, because the obvious
        // `prefix > ?1 OR (prefix = ?1 AND number > ?2)` says the same thing and loses the index.
        // Half the identity living inside the number is what keeps that question from arising.
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SONG_COLUMNS} FROM songs WHERE number > ?1 ORDER BY number LIMIT ?2"
        ))?;
        // No cursor means start before everything: a number below the lowest, since zero is not a
        // song number.
        let number = after.map_or(0, |code| code.number());
        let rows = statement.query_map(params![number, limit as i64], read_song)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every language present in the catalog, with how many songs are in it.
    ///
    /// Ordered by how much of the catalog each accounts for, because that is the order a picker
    /// wants: the one language nearly everything is in belongs at the top, not wherever its ISO code
    /// happens to sort. Songs with no language recorded are not a language and are left out.
    ///
    /// `hidden` leaves out the songs of packages a person hid, so a language only those packages
    /// hold drops out of the picker and every count is what that person can reach.
    pub fn languages(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, LibraryError> {
        let mut sql = String::from(
            "SELECT language, COUNT(*) FROM songs WHERE language IS NOT NULL AND language != ''",
        );
        let mut bindings = Vec::new();
        search::push_package_exclusion(&mut sql, &mut bindings, "package_id", hidden);
        sql.push_str(" GROUP BY language ORDER BY COUNT(*) DESC, language");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every tag present in the catalog, with how many songs carry it.
    ///
    /// The vocabulary — there is no table of legal tags anywhere, so *what tags exist* is only ever
    /// this question asked of the songs. Ordered the way [`Self::languages`] is and for the same
    /// reason: a picker wants the commonest first, not whichever slug sorts earliest.
    ///
    /// Read from `song_tags` rather than by splitting `songs.tags` on every row, which is the entire
    /// reason that table exists.
    ///
    /// `hidden` is as in [`Self::languages`]. The join to `songs` is made only when something is
    /// hidden, so the common case stays one scan of `song_tags`.
    pub fn tags(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, LibraryError> {
        let mut sql = String::from("SELECT t.tag, COUNT(*) FROM song_tags t");
        let mut bindings = Vec::new();
        if !hidden.is_empty() {
            sql.push_str(" JOIN songs s ON s.id = t.song_id WHERE 1 = 1");
            search::push_package_exclusion(&mut sql, &mut bindings, "s.package_id", hidden);
        }
        sql.push_str(" GROUP BY t.tag ORDER BY COUNT(*) DESC, t.tag");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every artist, with how many songs they have.
    ///
    /// For a remote's artist list. `contains` narrows by substring, case- and accent-insensitively:
    /// both sides go through `km_song::text::fold`, so a singer typing `legiao` finds
    /// `Legião Urbana`. Songs with no artist are left out: "no artist" is not somebody to browse.
    ///
    /// **Grouped on the name somebody typed, ordered by the fold of it.** Grouping on `sort_artist`
    /// instead would merge `Legião Urbana` and `Legiao Urbana` into one row of two — and the
    /// drill-down from that row is exact (`ArtistFilter::Exactly`, `s.artist = ?`), so the row would
    /// advertise two songs and open onto one. Two spellings stay two rows with honest counts, and
    /// the fold puts them next to each other, which is what lets somebody notice the duplicate.
    ///
    /// `MIN(sort_artist)` rather than a bare `sort_artist`: the key is functionally determined by
    /// the name, so every row of a group carries the same one and SQLite would accept the bare
    /// column — saying it out loud costs nothing and does not lean on an extension.
    ///
    /// `hidden` is as in [`Self::languages`]: an artist whose every song is hidden is not listed.
    pub fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<(String, usize)>, LibraryError> {
        let mut sql = String::from(
            "SELECT artist, COUNT(*) FROM songs WHERE artist IS NOT NULL AND artist != ''",
        );
        let mut bindings: Vec<search::Binding> = Vec::new();
        if let Some(needle) = contains.map(str::trim).filter(|value| !value.is_empty()) {
            // Folded, then escaped. After the fold there is nothing left for `escape_like` to
            // escape — `%` and `_` are not alphanumeric, so the fold has already turned them into
            // spaces — but it stays, because dropping it would make the wildcard safety depend on
            // another function's internals rather than on this line.
            bindings.push(search::Binding::Text(format!(
                "%{}%",
                escape_like(&km_song::text::fold(needle))
            )));
            sql.push_str(" AND sort_artist LIKE ?1 ESCAPE '\\'");
        }
        search::push_package_exclusion(&mut sql, &mut bindings, "package_id", hidden);
        sql.push_str(" GROUP BY artist ORDER BY MIN(sort_artist), artist");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Installed packages, most recent first.
    pub fn packages(&self) -> Result<Vec<InstalledPackage>, LibraryError> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, version, path, song_count, installed_at, bank
             FROM packages ORDER BY installed_at DESC, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(InstalledPackage {
                id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                path: row.get(3)?,
                song_count: usize::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                installed_at: row.get(5)?,
                bank: row.get::<_, i64>(6)?.try_into().unwrap_or(0),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Installs a package into a bank, indexing its songs.
    ///
    /// Reinstalling the same package identifier replaces it, which is how an upgrade works. A bank
    /// belonging to a *different* package is a hard error, and it is the **only** way an install can
    /// clash now: a song number is the user's handle on a song, so quietly reassigning or
    /// overwriting one is never the right answer, and putting each package in a thousand of its own
    /// is what makes that never arise.
    ///
    /// `bank` is the one the package is about to be installed under, which is why it is an argument
    /// rather than read from anywhere — the caller decides where a package goes, and for a package
    /// that has never installed there is nowhere else the answer could come from.
    ///
    /// `now` is supplied rather than read from the clock so the caller controls the timestamp format
    /// and tests are deterministic.
    pub fn install(
        &mut self,
        package: &Package,
        bank: u16,
        now: &str,
    ) -> Result<InstallReport, LibraryError> {
        let manifest = package.manifest();
        let package_id = manifest.package.id.clone();

        if bank == 0 {
            return Err(LibraryError::BankReserved);
        }
        if bank > km_songcode::MAX_BANK {
            return Err(LibraryError::BadBank { bank });
        }
        if let Some(owner) = self.package_holding(bank)?
            && owner != package_id
        {
            return Err(LibraryError::BankTaken { bank, owner });
        }

        let path = package
            .path()
            .canonicalize()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| package.path().display().to_string());

        let transaction = self.conn.transaction()?;

        let replaced_existing: bool = transaction
            .query_row(
                "SELECT 1 FROM packages WHERE id = ?1",
                params![&package_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        // What this package held before we replace it, so that the counter at the end can move only
        // if the replacement actually differs. Over an install of something new this hashes no rows,
        // which is exactly right: nothing became something.
        let digest_before = Self::package_digest(&transaction, &package_id)?;

        // Cascades to the package's songs, and the FTS triggers keep the index in step.
        transaction.execute("DELETE FROM packages WHERE id = ?1", params![&package_id])?;
        transaction.execute(
            "INSERT INTO packages (id, name, version, path, song_count, installed_at, bank)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &package_id,
                &manifest.package.name,
                &manifest.package.version,
                &path,
                i64::try_from(manifest.songs.len()).unwrap_or(0),
                now,
                bank
            ],
        )?;

        let mut duplicate_content = Vec::new();
        {
            let mut insert = transaction.prepare(
                "INSERT INTO songs (
                    number, package_id, title, artist, language, kind, file, duration_ms,
                    lyric_encoding, default_transpose, melody_channel, suitability, content_hash,
                    lyric_preview, sort_key, sort_artist, tags, loudness_lufs, fixes,
                    lyrics_hidden
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                           ?17, ?18, ?19, ?20)",
            )?;
            // The join table `song_tags` is filled here rather than by a trigger, unlike
            // `songs_fts`: a trigger would have to split `songs.tags` on a comma, which SQL cannot
            // do without a recursive CTE, and this is the only path that writes a song at all.
            let mut insert_tag = transaction
                .prepare("INSERT OR IGNORE INTO song_tags (song_id, tag) VALUES (?1, ?2)")?;
            let mut find_duplicate = transaction.prepare(
                "SELECT number, package_id FROM songs
                 WHERE content_hash = ?1 AND content_hash IS NOT NULL LIMIT 1",
            )?;

            for song in &manifest.songs {
                // A slot outside the bank is refused by `Manifest::problems`, so a package that
                // reaches an install cannot carry one — but the arithmetic is fallible in the type
                // and skipping the song is the only answer that cannot corrupt a neighbor.
                let Some(code) = u16::try_from(song.number)
                    .ok()
                    .and_then(|slot| SongCode::in_bank(bank, slot))
                else {
                    continue;
                };
                if let Some(hash) = &song.content_hash {
                    let existing: Option<(SongCode, String)> = find_duplicate
                        .query_row(params![hash], |row| Ok((read_code(row)?, row.get(1)?)))
                        .optional()?;
                    if let Some((existing_number, existing_package)) = existing {
                        duplicate_content.push(DuplicateContent {
                            number: code,
                            existing_number,
                            existing_package,
                        });
                    }
                }
                // Read through `Tag` rather than trusted as written: a manifest is a file somebody
                // may have edited, so this is where a typed word becomes a slug and where the
                // sorted, de-duplicated, comma-joined spelling the column relies on is made.
                let tags = km_kmpkg::tag::parse_list(&song.tags.join(","));
                insert.execute(params![
                    code.number(),
                    &package_id,
                    &song.title,
                    &song.artist,
                    &song.language,
                    song.kind.as_str(),
                    &song.file,
                    song.duration_ms,
                    &song.lyric_encoding,
                    song.default_transpose,
                    song.melody.as_ref().map(|m| m.channel),
                    song.suitability.as_ref().map(|s| s.value),
                    &song.content_hash,
                    store_preview(&song.lyric_preview),
                    // The sort keys, folded here rather than in SQL: `COLLATE NOCASE` is ASCII-only
                    // and there is no SQL spelling of this fold that would not be a second copy of
                    // the accent table. A song with no artist folds to the empty string, which is
                    // where SQLite already sorted its NULL.
                    km_song::text::fold(&song.title),
                    km_song::text::fold(song.artist.as_deref().unwrap_or_default()),
                    km_kmpkg::tag::join(&tags),
                    // Only the loudness; the manifest also carries a true peak and the machine
                    // never reads one, since levelling only attenuates and no attenuation clips.
                    song.loudness.as_ref().map(|l| l.lufs),
                    store_fixes(&song.fixes),
                    song.lyrics_hidden,
                ])?;
                if !tags.is_empty() {
                    let song_id = transaction.last_insert_rowid();
                    for tag in &tags {
                        insert_tag.execute(params![song_id, tag.as_str()])?;
                    }
                }
            }
        }
        // Inside the transaction, so the counter and the songs it describes are committed together
        // or not at all. A version that moved without the catalog moving would tell a mirror to
        // re-download for nothing; one that did not move when the catalog did would tell it not to
        // bother, which is the failure that matters.
        //
        // **And only when the rows actually changed**, which is not a refinement but the difference
        // between the counter working and not: `install_startup_packages` reinstalls every
        // configured package at every start of the machine, so an unconditional bump here made the
        // number move on each start and sent every mirror in the house to re-download a catalog
        // that was identical. Re-reading the rows costs one indexed scan of the package just
        // written; the alternative costs somebody's phone the whole catalog.
        if Self::package_digest(&transaction, &package_id)? != digest_before {
            Self::bump_version(&transaction)?;
        }
        transaction.commit()?;

        Ok(InstallReport {
            package_id,
            package_name: manifest.package.name.clone(),
            songs_added: manifest.songs.len(),
            replaced_existing,
            duplicate_content,
        })
    }

    /// Removes a package and its songs. Returns how many songs went with it.
    pub fn uninstall(&mut self, package_id: &str) -> Result<usize, LibraryError> {
        let transaction = self.conn.transaction()?;
        let songs: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM songs WHERE package_id = ?1",
            params![package_id],
            |row| row.get(0),
        )?;
        // Cascades to the package's songs, and the FTS triggers keep the index in step.
        let removed =
            transaction.execute("DELETE FROM packages WHERE id = ?1", params![package_id])?;
        // Only when something actually went. Asking to uninstall a package that is not installed is
        // not a change, and a mirror should not be sent to re-read a catalog nobody touched.
        if removed > 0 {
            Self::bump_version(&transaction)?;
        }
        transaction.commit()?;
        Ok(usize::try_from(songs).unwrap_or(0))
    }

    /// Keeps exactly the packages named and removes every other, returning the ids that went.
    ///
    /// **This is what makes the packages folders the truth rather than merely the usual source.**
    /// Nothing else in this file ever removed a package that had not been asked for by name, so a
    /// `.kmpkg` taken out of a folder left its rows behind for ever: its songs stayed in the count,
    /// stayed dialable, and failed when somebody picked one — a fault nothing reported and nothing
    /// could clear. The caller reconciles against what a scan found, so what the folders hold and
    /// what this file holds cannot drift apart across a restart.
    ///
    /// **`keep` is the scan plus the debug extras**, never the scan alone. Said that way here
    /// because getting it wrong would prune, at every pass, exactly the packages `debug.packages`
    /// had just installed.
    ///
    /// **A second write path into this file, and deliberately a narrower one than
    /// [`Library::uninstall`].** That one is an owner removing a package: it deletes the archive and
    /// releases the bank. This one is the machine noticing a package is no longer there; it touches
    /// **no file** and releases **no bank**, so a package that comes back — a folder that was
    /// unreadable for one pass, a file being mended — comes back with the song numbers it had rather
    /// than whatever is free next. Do not unify the two.
    ///
    /// Reads the ids and then deletes them one by one rather than issuing `DELETE … WHERE id NOT IN
    /// (…)`: the caller needs the list for the log line, a `NOT IN` needs a placeholder list built
    /// by hand, and `NOT IN ()` is a syntax error — so the empty-`keep` case, an owner who emptied
    /// the folder, would be the one that failed. Deleting a package cascades to its songs and the
    /// FTS triggers keep the index in step, exactly as `uninstall` relies on.
    pub fn retain_packages(&mut self, keep: &[&str]) -> Result<Vec<String>, LibraryError> {
        let transaction = self.conn.transaction()?;
        let doomed: Vec<String> = {
            let mut statement = transaction.prepare("SELECT id FROM packages")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<String>, _>>()?
                .into_iter()
                .filter(|id| !keep.contains(&id.as_str()))
                .collect()
        };
        if !doomed.is_empty() {
            let mut delete = transaction.prepare("DELETE FROM packages WHERE id = ?1")?;
            for id in &doomed {
                delete.execute(params![id])?;
            }
            // Only when something actually went. A pass that finds the catalog already correct is
            // the common one, and bumping the version there would send every mirror in the house to
            // re-download a catalog that is identical — the fault `bump_version` records.
            Self::bump_version(&transaction)?;
        }
        transaction.commit()?;
        Ok(doomed)
    }

    /// Looks a song up by its code, which is what the keypad does.
    pub fn song(&self, code: SongCode) -> Result<Option<CatalogSong>, LibraryError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SONG_COLUMNS} FROM songs WHERE number = ?1"
        ))?;
        Ok(statement
            .query_row(params![code.number()], read_song)
            .optional()?)
    }

    /// Looks a song up by the package it came from and what its content hashes to.
    ///
    /// **A candidate key, and not by luck.** Two songs in one package with the same hash are a
    /// `ManifestProblem::DuplicateContent`, which `km_kmpkg` refuses both when opening a package and
    /// when writing one — so this can return at most one row, where [`Self::song_by_content`] below
    /// cannot. Neither half of the key moves when a bank does, which is what it is for: it is how
    /// the offline remote finds a favorite again after [`Self::set_package_bank`] has renumbered
    /// everything the favorite was filed under.
    pub fn song_in_package(
        &self,
        package_id: &str,
        content_hash: &str,
    ) -> Result<Option<CatalogSong>, LibraryError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SONG_COLUMNS} FROM songs WHERE package_id = ?1 AND content_hash = ?2"
        ))?;
        Ok(statement
            .query_row(params![package_id, content_hash], read_song)
            .optional()?)
    }

    /// Looks a *recording* up by what it hashes to, wherever it is filed.
    ///
    /// **Legitimately several rows, so this picks rather than assumes.** Two packages holding one
    /// recording is a catalog smell reported at install and never refused — see
    /// [`InstallReport::duplicate_content`] — so the lowest number is taken, which is what makes the
    /// same catalog answer the same way every time it is asked.
    pub fn song_by_content(&self, content_hash: &str) -> Result<Option<CatalogSong>, LibraryError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SONG_COLUMNS} FROM songs
             WHERE content_hash = ?1 ORDER BY number LIMIT 1"
        ))?;
        Ok(statement
            .query_row(params![content_hash], read_song)
            .optional()?)
    }

    /// Whether any song in the catalog came from this package.
    ///
    /// Only ever asked on the way to reporting a miss, to tell *"that package is not installed"*
    /// apart from *"that package does not hold that recording"*.
    pub fn has_package(&self, package_id: &str) -> Result<bool, LibraryError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM songs WHERE package_id = ?1)",
            params![package_id],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    /// The archive path for a song, so its MIDI can be read.
    pub fn package_path_for(&self, code: SongCode) -> Result<Option<String>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT p.path FROM songs s JOIN packages p ON p.id = s.package_id
                 WHERE s.number = ?1",
                params![code.number()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Every bank in use, ascending.
    ///
    /// For whoever is choosing the next one. Ascending rather than in install order because the
    /// caller is looking for the lowest gap, and a sorted list is what makes that a walk rather than
    /// a search.
    pub fn banks(&self) -> Result<Vec<u16>, LibraryError> {
        let mut statement = self
            .conn
            .prepare("SELECT bank FROM packages ORDER BY bank")?;
        let rows = statement.query_map([], |row| {
            Ok(row.get::<_, i64>(0)?.try_into().unwrap_or(0u16))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Which bank a package is installed in, if it is installed.
    ///
    /// **The catalog is a second place the answer lives**, and that is what makes it worth having:
    /// the machine records a package's bank in its settings, and a settings file that is lost or
    /// rescued from a bad one would otherwise let an installed package be moved to a different bank
    /// under a live queue. Asking here first is what makes "a package that has a bank keeps it" true
    /// rather than merely usual.
    pub fn bank_of(&self, package_id: &str) -> Result<Option<u16>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT bank FROM packages WHERE id = ?1",
                params![package_id],
                |row| Ok(row.get::<_, i64>(0)?.try_into().unwrap_or(0u16)),
            )
            .optional()?)
    }

    /// Moves a package to another bank.
    ///
    /// **Re-keys every song in the package**, because the number *is* the identity — so it is
    /// refused when another package is already there, and the caller is responsible for refusing it
    /// while anything is playing or queued (the machine does; see the API's 409).
    ///
    /// The songs are renumbered by arithmetic rather than re-read from the package: a song's slot is
    /// what it was, and only the thousand it sits in has changed.
    ///
    /// **The sort keys are deliberately not touched.** This is the only `UPDATE songs` in the crate
    /// and the reflex on reading that is to add them to it; a renumber changes no name, so
    /// `sort_key` and `sort_artist` are already right and rewriting them would be work that also
    /// retokenises the whole package's FTS rows.
    pub fn set_package_bank(&mut self, package_id: &str, bank: u16) -> Result<usize, LibraryError> {
        if bank == 0 {
            return Err(LibraryError::BankReserved);
        }
        if bank > km_songcode::MAX_BANK {
            return Err(LibraryError::BadBank { bank });
        }
        if let Some(owner) = self.package_holding(bank)?
            && owner != package_id
        {
            return Err(LibraryError::BankTaken { bank, owner });
        }
        let transaction = self.conn.transaction()?;
        transaction.execute(
            "UPDATE packages SET bank = ?1 WHERE id = ?2",
            params![bank, package_id],
        )?;
        let changed = transaction.execute(
            "UPDATE songs SET number = ?1 * ?2 + (number % ?2) WHERE package_id = ?3",
            params![bank, km_songcode::BANK_SPAN, package_id],
        )?;
        Self::bump_version(&transaction)?;
        transaction.commit()?;
        Ok(changed)
    }

    /// Which package is in a bank, if any.
    pub fn package_holding(&self, bank: u16) -> Result<Option<String>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM packages WHERE bank = ?1",
                params![bank],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Searches the catalog.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<CatalogSong>, LibraryError> {
        let (sql, bindings) = query.to_sql();
        let mut statement = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bindings.iter().map(|b| b as &dyn rusqlite::ToSql).collect();
        let rows = statement.query_map(params.as_slice(), read_song)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Songs sharing content with another song, across the whole catalog.
    ///
    /// The local corpus is full of the same recording under different names, so a catalog built
    /// from it will contain duplicates unless somebody looks.
    /// **Reads the number as a column rather than out of a concatenated string.** It used to
    /// `GROUP_CONCAT` the numbers and parse them back with `.ok()`, which silently dropped anything
    /// that did not read as a `u32`. That was harmless while every code was a number, became a way
    /// to lose every prefixed song when a code stopped being one, and is harmless again now — the
    /// query stays as it is, because the failure it was written for is a class rather than an
    /// instance. Grouping in Rust costs one pass over rows the query already returns.
    pub fn duplicate_content(&self) -> Result<Vec<Vec<SongCode>>, LibraryError> {
        let mut statement = self.conn.prepare(
            // `number` first, because that is what `read_code` reads.
            "SELECT number, content_hash FROM songs
             WHERE content_hash IS NOT NULL
               AND content_hash IN (
                   SELECT content_hash FROM songs
                   WHERE content_hash IS NOT NULL
                   GROUP BY content_hash HAVING COUNT(*) > 1)
             ORDER BY content_hash, number",
        )?;
        let rows = statement.query_map([], |row| {
            let hash: String = row.get(1)?;
            Ok((hash, read_code(row)?))
        })?;

        let mut groups: Vec<Vec<SongCode>> = Vec::new();
        let mut current: Option<String> = None;
        for row in rows {
            let (hash, code) = row?;
            if current.as_deref() != Some(hash.as_str()) {
                current = Some(hash);
                groups.push(Vec::new());
            }
            // `groups` gains an entry whenever the hash changes, so this cannot be empty.
            if let Some(group) = groups.last_mut() {
                group.push(code);
            }
        }
        Ok(groups)
    }
}

/// Rebuilds a catalog that is not the shape `schema.sql` writes, and refolds one whose sort keys an
/// older fold table wrote.
///
/// **Rebuilt rather than converted, because this file is a derived index.** Every installed package
/// is sitting in one of the folders the machine scans, and the machine reinstalls from them at every
/// start, so dropping the song tables costs one rebuild on the next boot and nothing else. Anything
/// ever stored here that is *not* derivable from a package would break that, so do not store any.
///
/// **The shape is the number.** The catalog carries no schema version, so what makes it current is
/// having every column its queries read. `schema.sql` is `CREATE ... IF NOT EXISTS` throughout and
/// cannot tell a current table from an older one, which is why this runs first.
///
/// **`meta` survives the rebuild**, and that is load-bearing. The reinstall after a drop sees an empty
/// catalog, so every package's digest differs and the catalog version moves — which is what tells
/// every mirror in the house to fetch again. A `meta` dropped with the songs would restart the
/// counter, and a mirror already holding the restarted number would answer *already up to date*.
///
/// **`km-remote-core`'s mirror answers the same question and does not get it from here**, being a
/// different crate with a schema of its own; one grep for the column names finds the pair.
fn prepare_existing(conn: &Connection) -> Result<(), LibraryError> {
    // The catalog may not exist at all yet, in which case `schema.sql` creates it correctly.
    if !has_table(conn, "songs")? {
        return Ok(());
    }

    let mut current = has_table(conn, "meta")? && has_column(conn, "packages", "bank")?;
    for column in [
        "kind",
        "lyric_preview",
        "tags",
        "loudness_lufs",
        "fixes",
        "sort_key",
        "sort_artist",
        "lyrics_hidden",
    ] {
        current = current && has_column(conn, "songs", column)?;
    }
    if !current {
        tracing::info!(
            "the catalog is not the current shape; rebuilding it from the installed packages"
        );
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS songs_fts_insert;
             DROP TRIGGER IF EXISTS songs_fts_delete;
             DROP TRIGGER IF EXISTS songs_fts_update;
             DROP TABLE IF EXISTS songs_fts;
             DROP TABLE IF EXISTS song_tags;
             DROP TABLE IF EXISTS songs;
             DROP TABLE IF EXISTS packages;",
        )?;
        return Ok(());
    }

    // **The sort keys were written by a fold table, and the table can move.** A machine that learned
    // to file `Příliš` under `P` would otherwise go on showing it past the end of the alphabet until
    // every package happened to be reinstalled. The refold moves the catalog version, because
    // `km-remote-core` copies `sort_key` and orders by it, so an offline remote that was not told
    // would go on sorting by the old alphabet with nothing to notice.
    if stored_fold_version(conn)?.as_deref() != Some(fold_version().as_str()) {
        backfill_sort_keys(conn)?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('fold_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [fold_version()],
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('catalog_version', '1')
             ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
            [],
        )?;
    }
    Ok(())
}

/// Which spelling of `km_song::text::fold` this catalog's sort keys were written by.
///
/// **Not a number of this crate's own**, deliberately: it is `km_song::text::FOLD_REVISION`, which
/// lives beside the table it describes, so the one edit that changes the folding is also the one
/// that invalidates every stored key. A constant here would be a second thing to remember.
fn fold_version() -> String {
    km_song::text::FOLD_REVISION.to_string()
}

/// The recorded fold version, or `None` where none is stored.
fn stored_fold_version(conn: &Connection) -> Result<Option<String>, LibraryError> {
    Ok(conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'fold_version'",
            [],
            |row| row.get(0),
        )
        .optional()?)
}

/// Refolds every sort key in the catalog with the current fold table.
///
/// In Rust and not in SQL, with the same `km_song::text::fold` an install uses — which is the whole
/// point of doing it here at all. A SQL spelling would be a second copy of the accent table, and two
/// of those disagreeing looks like bad data rather than like a bug.
///
/// **The FTS triggers are dropped first, and that is not an optimization.** All three fire on every
/// `UPDATE songs`, and each costs a delete and a reinsert into the FTS5 index — for a six-figure
/// catalog that is minutes spent rewriting an index over two columns this loop does not touch.
/// `prepare` runs `prepare_existing` and *then* `schema.sql`, whose `CREATE TRIGGER IF NOT EXISTS` puts all
/// three back before the caller can reach anything that writes.
///
/// **The rows are read into memory before any of them is written.** SQLite leaves the result of a
/// `SELECT` undefined if the table is modified while it is still being stepped, so updating inside
/// the `query_map` would be reading rows the writes had moved.
fn backfill_sort_keys(conn: &Connection) -> Result<(), LibraryError> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS songs_fts_insert;
         DROP TRIGGER IF EXISTS songs_fts_delete;
         DROP TRIGGER IF EXISTS songs_fts_update;",
    )?;
    let transaction = conn.unchecked_transaction()?;
    let rows: Vec<(i64, String, Option<String>)> = {
        let mut statement = transaction.prepare("SELECT id, title, artist FROM songs")?;
        let mapped = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };
    if !rows.is_empty() {
        tracing::info!(songs = rows.len(), "folding the catalog's sort keys");
    }
    {
        let mut update = transaction
            .prepare("UPDATE songs SET sort_key = ?2, sort_artist = ?3 WHERE id = ?1")?;
        for (id, title, artist) in rows {
            update.execute(params![
                id,
                km_song::text::fold(&title),
                km_song::text::fold(artist.as_deref().unwrap_or_default()),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn has_table(conn: &Connection, table: &str) -> Result<bool, LibraryError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, LibraryError> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Columns selected for a [`CatalogSong`], in the order [`read_song`] expects.
/// Escapes the two characters `LIKE` treats as wildcards, plus the escape character itself.
///
/// Every `LIKE` in this crate is written `ESCAPE '\'` and, until this existed, none of them escaped
/// anything — so a search for `50%` matched every artist, and one for `AC_DC` matched `ACaDC`. It is
/// a small wrongness and an easy one to reintroduce, which is why it is a function rather than a
/// note: a pattern built any other way is now visibly built any other way.
pub(crate) fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Every column a song is read back through — and, because [`Library::package_digest`] selects
/// through this list, every column a change to which moves `catalog_version`.
///
/// `tags` is here for that second reason as much as the first: a package rebuilt with nothing
/// changed but its tags has to look different to a mirror, or every phone in the house keeps the
/// tags it downloaded once and nothing ever says otherwise.
///
/// `loudness_lufs` is here for the **first** reason and takes the second as a consequence: the
/// machine reads it at song start out of a [`CatalogSong`], so it has to be in the list this reads
/// through — and one list serving both purposes is the whole point of the paragraph above. What that
/// costs is that a package re-analysed for nothing but its levels moves `catalog_version` and the
/// phones re-download. That is the honest answer rather than a price to dodge: the package really
/// did change, and a second list to keep it out of the digest would be a second thing to keep in
/// step in order to tell a mirror less than the truth.
///
/// `lyrics_hidden` is here on `loudness_lufs`' terms and takes the same consequence: the machine
/// reads it at song start, so it has to be in the list this reads through, and a package rebuilt
/// for nothing but that flag moves `catalog_version` and the phones re-download. What a song puts
/// on a television is as much a part of it as how loud it is.
const SONG_COLUMNS: &str = "number, package_id, title, artist, language, kind, file, duration_ms, \
     lyric_encoding, default_transpose, melody_channel, suitability, content_hash, lyric_preview, \
     tags, loudness_lufs, fixes, lyrics_hidden";

/// Reads a song's code from the column that holds it.
///
/// One column rather than two — the bank is inside the number rather than beside it, so there is
/// nothing to reassemble and nothing that can be reassembled wrongly.
fn read_code(row: &rusqlite::Row<'_>) -> rusqlite::Result<SongCode> {
    Ok(SongCode::new(row.get(0)?))
}

fn read_song(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogSong> {
    let kind: String = row.get(5)?;
    Ok(CatalogSong {
        number: read_code(row)?,
        package_id: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        language: row.get(4)?,
        // Anything unrecognized reads as MIDI rather than failing the row. A catalog is not a
        // wire format somebody else writes; the only way this is not a kind named here is a
        // database from a future build, and refusing to list a song is worse than mislabeling it.
        //
        // With three kinds that mislabeling is no longer harmless in principle — a `cdg` song read
        // as MIDI would be looked for inside the archive. It stays anyway, because the outcome is
        // one song that fails to load and says so in the log, where the alternative is a catalog
        // that has silently lost rows. The package format's own version gate is what actually keeps
        // a newer package away from an older build; see `Manifest::required_format`.
        kind: match kind.as_str() {
            k if k == SongKind::Video.as_str() => SongKind::Video,
            k if k == SongKind::Cdg.as_str() => SongKind::Cdg,
            k if k == SongKind::UltraStar.as_str() => SongKind::UltraStar,
            k if k == SongKind::Lrc.as_str() => SongKind::Lrc,
            _ => SongKind::Midi,
        },
        file: row.get(6)?,
        duration_ms: row.get(7)?,
        lyric_encoding: row.get(8)?,
        default_transpose: row.get(9)?,
        melody_channel: row.get(10)?,
        suitability: row.get(11)?,
        content_hash: row.get(12)?,
        lyric_preview: row
            .get::<_, Option<String>>(13)?
            .map(|stored| stored.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
        // Split rather than parsed: what is in this column was written through `Tag` by `install`,
        // and re-folding every row on every read would cost a fold per song per search for nothing.
        tags: row
            .get::<_, String>(14)?
            .split(',')
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect(),
        loudness_lufs: row.get(15)?,
        fixes: read_fixes(&row.get::<_, String>(16)?),
        lyrics_hidden: row.get(17)?,
    })
}

/// Joins preview lines for storage, or `None` when there are none.
///
/// One TEXT column rather than JSON: a preview line cannot contain a newline, because the timeline is
/// what splits lines in the first place. `None` and an empty list are the same thing here and the
/// column is nullable, so a song with no words costs a NULL rather than an empty string.
fn store_preview(lines: &[String]) -> Option<String> {
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// A song's fix list as the column holds it.
///
/// JSON rather than a set of columns, because a fix carries arguments and this build must hand on
/// one it cannot read. An unserializable list stores as empty rather than failing the install: a
/// song that plays its own defect is a worse answer than a package that will not open.
fn store_fixes(fixes: &[km_fixes::Fix]) -> String {
    serde_json::to_string(fixes).unwrap_or_else(|_| "[]".to_owned())
}

/// Reads a fix list back, treating an unreadable column as no fixes.
///
/// A corrupt value here means a song plays as its file was written, which is what every song did
/// before anything corrected one — never a refusal to play it.
fn read_fixes(stored: &str) -> Vec<km_fixes::Fix> {
    serde_json::from_str(stored).unwrap_or_default()
}

/// Turns a package manifest entry into what the catalog stores, for tools that need both.
pub fn song_columns() -> &'static str {
    SONG_COLUMNS
}

/// The melody channel a song entry declares, if any.
pub fn melody_channel_of(entry: &SongEntry) -> Option<u8> {
    entry.melody.as_ref().map(|melody| melody.channel)
}
