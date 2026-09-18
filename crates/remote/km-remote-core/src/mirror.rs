//! This phone's own copy of a machine's catalog.
//!
//! The reason the offline app exists. Everything on the Songs tab — browsing, searching, artists,
//! folders — is answered from here, so it all works with the karaoke machine switched off, which is
//! the normal state of a machine under a television at four in the afternoon.
//!
//! **Shaped like `km-catalog`'s `songs` table, and searched with the same helper.**
//! [`km_catalog::fts_match_query`] builds the FTS5 MATCH string for both, so `AC/DC` is not a syntax
//! error here either and `coracao` finds `Coração` on both sides. Two columns are added that the
//! machine's catalog does not have — see `schema.sql`.
//!
//! Synchronous, because rusqlite is; the `Songs` implementation at the bottom of this file is what
//! gets it off the async runtime, once, in one place, rather than every handler remembering.

use std::path::Path;
use std::sync::{Arc, Mutex};

use km_api::dto::SongDto;
use km_catalog::fts_match_query;
use km_remote_pages::machine::{
    ArtistFilter, ArtistRow, BrowseQuery, LanguageRow, Miss, Order, PackageRow, RemoteError,
    Resolution, SongPage, SongRef, Songs, TagRow,
};
use km_song::text::{fold, initial};
use km_songcode::SongCode;
use rusqlite::{Connection, OptionalExtension, params};

/// The file the mirror lives in.
///
/// **Separate from the favorites**, which is not tidiness: a catalog refresh may legitimately
/// throw this whole file away and start again, and a collection somebody built up over a year must
/// not be able to go with it. Two files means the destructive operation cannot reach the precious
/// one by accident.
pub const MIRROR_FILE: &str = "catalog.sqlite";

/// The mirrored machine's instance id.
const KEY_MACHINE: &str = "machine_id";
/// The catalog version this copy is of.
const KEY_VERSION: &str = "catalog_version";

/// The local catalog.
pub struct Mirror {
    conn: Connection,
}

impl Mirror {
    /// Opens or creates the mirror.
    pub fn open(dir: &Path) -> Result<Self, rusqlite::Error> {
        Self::prepare(Connection::open(dir.join(MIRROR_FILE))?)
    }

    /// A mirror in memory, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self, rusqlite::Error> {
        // Ignored rather than fatal: an in-memory database has no journal to switch.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Before the schema batch, not after: `schema.sql` is `CREATE ... IF NOT EXISTS` throughout,
        // so it cannot tell a current table from an older one. See [`discard_unless_current`].
        discard_unless_current(&conn)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Self { conn })
    }

    /// Which machine this is a copy of, and how far along.
    pub fn mirrored(&self) -> Result<Option<(String, u64)>, rusqlite::Error> {
        let machine: Option<String> = self.meta(KEY_MACHINE)?;
        let version: Option<String> = self.meta(KEY_VERSION)?;
        Ok(match (machine, version) {
            (Some(machine), Some(version)) => Some((machine, version.parse().unwrap_or_default())),
            _ => None,
        })
    }

    fn meta(&self, key: &str) -> Result<Option<String>, rusqlite::Error> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    /// Whether a re-download would find anything new.
    ///
    /// **A different machine always means yes**, whatever the numbers say. A catalog version is
    /// monotonic within one machine and meaningless between two, so the same number against a
    /// different id is not "nothing has changed" — it is a different catalog, and importing on top
    /// of the old one would leave a list that is half of each.
    pub fn is_current(&self, machine_id: &str, version: u64) -> bool {
        match self.mirrored() {
            Ok(Some((mirrored_id, mirrored_version))) => {
                mirrored_id == machine_id && mirrored_version == version
            }
            _ => false,
        }
    }

    /// How many songs are held.
    pub fn count(&self) -> Result<usize, rusqlite::Error> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM songs", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Replaces the whole catalog with what an import produced.
    ///
    /// One transaction, and it **empties the table first**. An incremental merge was the obvious
    /// alternative and is wrong: the export says what a machine has, not what it has gained, so a
    /// song removed by uninstalling a package would stay in the mirror forever and be queueable from
    /// a phone and unqueueable from anywhere else.
    ///
    /// `packages` is each package's id and name, the one thing about a package the Setup page shows
    /// that a song row does not carry.
    pub fn replace(
        &mut self,
        machine_id: &str,
        version: u64,
        songs: &[SongDto],
        packages: &[(String, String)],
    ) -> Result<(), rusqlite::Error> {
        let transaction = self.conn.transaction()?;
        transaction.execute("DELETE FROM songs", [])?;
        transaction.execute("DELETE FROM packages", [])?;
        {
            let mut insert_package = transaction
                .prepare("INSERT OR REPLACE INTO packages (id, name) VALUES (?1, ?2)")?;
            for (id, name) in packages {
                insert_package.execute(params![id, name])?;
            }
        }
        {
            let mut insert = transaction.prepare(
                "INSERT INTO songs (number, title, artist, language, kind, duration_ms,
                                    suitability, melody_available, default_transpose, package_id,
                                    content_hash, sort_key, sort_artist, initial, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            let mut insert_tag = transaction
                .prepare("INSERT OR IGNORE INTO song_tags (song_id, tag) VALUES (?1, ?2)")?;
            for song in songs {
                // Read through `Tag` rather than trusted as it arrived: this is a wire type, and the
                // machine on the other end may be any version. It also makes the column's sorted,
                // de-duplicated spelling here rather than depending on the sender's.
                let tags = km_kmpkg::tag::parse_list(&song.tags.join(","));
                insert.execute(params![
                    song.number.number(),
                    &song.title,
                    &song.artist,
                    &song.language,
                    // Stored as TEXT: `SongKind` is the wire type and knows nothing about SQL, so the
                    // conversion lives at this boundary rather than as a `ToSql` impl in `km-kmpkg`.
                    song.kind.as_str(),
                    song.duration_ms,
                    song.suitability,
                    song.melody_available as i64,
                    song.default_transpose,
                    &song.package_id,
                    &song.content_hash,
                    fold(&song.title),
                    fold(song.artist.as_deref().unwrap_or_default()),
                    initial(&song.title).map(|first| first.to_string()),
                    km_kmpkg::tag::join(&tags),
                ])?;
                if !tags.is_empty() {
                    let song_id = transaction.last_insert_rowid();
                    for tag in &tags {
                        insert_tag.execute(params![song_id, tag.as_str()])?;
                    }
                }
            }
        }
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_MACHINE, machine_id],
        )?;
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_VERSION, version.to_string()],
        )?;
        transaction.commit()
    }

    /// One page of songs.
    fn search(&self, query: &BrowseQuery) -> Result<SongPage, rusqlite::Error> {
        let Conditions {
            where_sql,
            joins,
            bindings,
            ranked,
        } = self.conditions(query);

        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM songs s {joins} WHERE {where_sql}"),
            rusqlite::params_from_iter(bindings.iter()),
            |row| row.get(0),
        )?;

        let order = match query.order {
            // `ranked`, not `query.text.is_some()`. Those are not the same question and the
            // difference is a real failure: a query of pure punctuation sanitises down to nothing, so
            // there is text but **no MATCH and therefore no join** — and `f.rank` in the ORDER BY is
            // then `no such column`. Somebody typing `???` got a 500 where they should have got the
            // whole catalog.
            Order::Best if ranked => "f.rank, s.number",
            // The performer decides between two songs sharing a title, by the rule the machine's own
            // catalog sorts by: a song number is not an order anybody can read. `sort_artist` is
            // `NOT NULL DEFAULT ''` here too, so `= ''` is what puts a song nobody named a performer
            // for at the end of its title rather than the front.
            Order::Best | Order::Title => "s.sort_key, s.sort_artist = '', s.sort_artist, s.number",
            // The folded name, for the reason the title leg has used one all along. This was the
            // one leg still on `COLLATE NOCASE`, which is ASCII-only — so the A-Z strip folded, the
            // title order folded, and the artist order did not. One list, two alphabets.
            Order::Artist => "s.sort_artist, s.sort_key, s.number",
            Order::Number => "s.number",
        };

        let mut page = bindings.clone();
        page.push(Value::Integer(query.limit as i64));
        page.push(Value::Integer(query.offset as i64));
        let limit_index = page.len() - 1;

        let sql = format!(
            "SELECT s.number, s.title, s.artist, s.language, s.kind, s.duration_ms, s.suitability,
                    s.melody_available, s.default_transpose, s.package_id, s.content_hash,
                    s.tags
             FROM songs s {joins} WHERE {where_sql}
             ORDER BY {order} LIMIT ?{} OFFSET ?{}",
            limit_index,
            limit_index + 1
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(page.iter()), read_song)?;
        let songs = rows.collect::<Result<Vec<_>, _>>()?;

        let total = usize::try_from(total).unwrap_or(0);
        Ok(SongPage {
            more: query.offset + songs.len() < total,
            songs,
            total: Some(total),
        })
    }

    /// The WHERE clause, the joins it needs, the values to bind, and whether anything can be ranked.
    fn conditions(&self, query: &BrowseQuery) -> Conditions {
        let mut sql = String::from("1 = 1");
        let mut joins = String::new();
        let mut bindings: Vec<Value> = Vec::new();
        let mut ranked = false;

        if let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            let match_query = fts_match_query(text);
            // An entirely punctuation query sanitises down to nothing. Matching everything is right:
            // somebody who typed `???` has narrowed by nothing, and an error page for it would be an
            // error page for a slip of a thumb.
            if !match_query.is_empty() {
                joins.push_str(" JOIN songs_fts f ON f.rowid = s.id");
                bindings.push(Value::Text(match_query));
                sql.push_str(&format!(" AND songs_fts MATCH ?{}", bindings.len()));
                ranked = true;
            }
        }

        match &query.artist {
            Some(ArtistFilter::Exactly(name)) => {
                bindings.push(Value::Text(name.clone()));
                sql.push_str(&format!(" AND s.artist = ?{}", bindings.len()));
            }
            // Folded on both sides, matching `km_catalog`'s narrowing of the same name — so the
            // artist list and the songs it opens onto agree about who `Legião Urbana` is however
            // the singer spelled it. `Exactly` above stays on the raw column: it comes from a row
            // the list drew, so it is the name as stored, and an exact match is the point of it.
            Some(ArtistFilter::Contains(name)) => {
                bindings.push(Value::Text(format!(
                    "%{}%",
                    escape_like(&fold(name.trim()))
                )));
                sql.push_str(&format!(
                    " AND s.sort_artist LIKE ?{} ESCAPE '\\'",
                    bindings.len()
                ));
            }
            None => {}
        }

        if let Some(language) = query
            .language
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            // Exact and lowercased, exactly as `km_catalog::SearchQuery` matches it.
            bindings.push(Value::Text(language.to_lowercase()));
            sql.push_str(&format!(" AND s.language = ?{}", bindings.len()));
        }

        if let Some(initial) = query.initial {
            bindings.push(Value::Text(initial.to_string()));
            sql.push_str(&format!(" AND s.initial = ?{}", bindings.len()));
        }

        // One `EXISTS` holding an `IN`, exactly as `km_catalog::SearchQuery::to_sql` spells it, so
        // the online remote and the offline one narrow identically. An `EXISTS` rather than a join
        // because `joins` above is already carrying `songs_fts` and a second join would multiply the
        // rows; a subquery keeps the tags one seek on `(song_id, tag)`.
        //
        // An empty list is no filter at all, and `IN ()` is a syntax error, so it is checked.
        let tags: Vec<String> = query
            .tags
            .iter()
            .filter_map(|tag| km_kmpkg::Tag::parse(tag))
            .map(km_kmpkg::Tag::into_string)
            .collect();
        if !tags.is_empty() {
            let mut placeholders = Vec::with_capacity(tags.len());
            for tag in tags {
                bindings.push(Value::Text(tag));
                placeholders.push(format!("?{}", bindings.len()));
            }
            sql.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM song_tags t WHERE t.song_id = s.id AND t.tag IN ({}))",
                placeholders.join(", ")
            ));
        }

        push_package_exclusion(
            &mut sql,
            &mut bindings,
            "s.package_id",
            &query.hidden_packages,
        );

        Conditions {
            where_sql: sql,
            joins,
            bindings,
            ranked,
        }
    }

    fn song(&self, number: SongCode) -> Result<Option<SongDto>, rusqlite::Error> {
        self.conn
            .query_row(
                "SELECT number, title, artist, language, kind, duration_ms, suitability,
                        melody_available, default_transpose, package_id, content_hash, tags
                 FROM songs WHERE number = ?1",
                params![number.number()],
                read_song,
            )
            .optional()
    }

    fn songs_by_number(&self, numbers: &[SongCode]) -> Result<Vec<SongDto>, rusqlite::Error> {
        // In the order asked for, which for a favorites folder is "most recently added first" and
        // is the order the caller went to the trouble of producing. SQL would give it back in
        // whatever order the index felt like.
        let mut found = Vec::with_capacity(numbers.len());
        let mut statement = self.conn.prepare(
            "SELECT number, title, artist, language, kind, duration_ms, suitability,
                    melody_available, default_transpose, package_id, content_hash, tags
             FROM songs WHERE number = ?1",
        )?;
        for number in numbers {
            // A folder is allowed to outlive the package a song came from, so a number that is no
            // longer catalogd is skipped rather than failing the page.
            if let Some(song) = statement
                .query_row(params![number.number()], read_song)
                .optional()?
            {
                found.push(song);
            }
        }
        Ok(found)
    }

    /// The three-rung resolve. See `Songs::resolve` for what the rungs are and why.
    ///
    /// Every statement is prepared once for the whole folder rather than once per song — a folder
    /// of a thousand favorites is one page of the app, and this is what it costs to draw.
    fn resolve(&self, refs: &[SongRef]) -> Result<Vec<Resolution>, rusqlite::Error> {
        const COLUMNS: &str = "number, title, artist, language, kind, duration_ms, suitability,
                               melody_available, default_transpose, package_id, content_hash, tags";
        let mut by_pair = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM songs WHERE package_id = ?1 AND content_hash = ?2"
        ))?;
        // **`ORDER BY number` is not decoration.** Two packages holding one recording is reported
        // rather than refused, so this rung can legitimately see several rows, and picking the
        // lowest is what makes the same catalog answer the same way every time it is asked.
        let mut by_hash = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM songs WHERE content_hash = ?1 ORDER BY number LIMIT 1"
        ))?;
        let mut by_number = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM songs WHERE number = ?1"))?;
        // Only ever run on the way to a miss, to tell the two absences apart.
        let mut package_present = self
            .conn
            .prepare("SELECT 1 FROM songs WHERE package_id = ?1 LIMIT 1")?;

        let mut out = Vec::with_capacity(refs.len());
        for asked in refs {
            let mut found = None;
            if let (Some(package), Some(hash)) = (&asked.package_id, &asked.content_hash) {
                found = by_pair
                    .query_row(params![package, hash], read_song)
                    .optional()?;
            }
            if found.is_none()
                && let Some(hash) = &asked.content_hash
            {
                found = by_hash.query_row(params![hash], read_song).optional()?;
            }
            if found.is_none() {
                // The number rung, and the filter is what keeps it from answering wrongly: a song
                // whose own hash contradicts the favorite's is a different recording sharing a
                // number, which is the ordinary shape of a collection looked at on a second
                // machine. See `SongRef::contradicted_by` for why a null on either side passes.
                found = by_number
                    .query_row(params![asked.code.number()], read_song)
                    .optional()?
                    .filter(|song| !asked.contradicted_by(song.content_hash.as_deref()));
            }
            let outcome = match found {
                Some(song) => Ok(song),
                None => Err(match &asked.package_id {
                    // A hash was carried and its package is nowhere in this catalog: the package is
                    // not installed, which is the one of the three somebody can act on.
                    Some(package) if !package_present.exists(params![package])? => {
                        Miss::PackageAbsent
                    }
                    _ if asked.is_identified() => Miss::RecordingAbsent,
                    _ => Miss::NumberAbsent,
                }),
            };
            out.push(Resolution {
                asked: asked.clone(),
                outcome,
            });
        }
        Ok(out)
    }

    /// **Grouped on the name somebody typed, ordered by the fold of it** — the same shape, and for
    /// the same reason, as `km_catalog::Library::artists`. Grouping on `sort_artist` would merge
    /// two spellings of one name into a row whose count the exact drill-down below cannot honor.
    fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<ArtistRow>, rusqlite::Error> {
        let mut sql = String::from(
            "SELECT artist, COUNT(*) FROM songs WHERE artist IS NOT NULL AND artist != ''",
        );
        let mut bindings: Vec<Value> = Vec::new();
        if let Some(needle) = contains.map(str::trim).filter(|value| !value.is_empty()) {
            bindings.push(Value::Text(format!("%{}%", escape_like(&fold(needle)))));
            sql.push_str(" AND sort_artist LIKE ?1 ESCAPE '\\'");
        }
        push_package_exclusion(&mut sql, &mut bindings, "package_id", hidden);
        sql.push_str(" GROUP BY artist ORDER BY MIN(sort_artist), artist");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok(ArtistRow {
                name: row.get(0)?,
                songs: usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            })
        })?;
        rows.collect()
    }

    /// Joined to `songs` only when something is hidden, as `km_catalog::Library::tags` is.
    fn tags(&self, hidden: &[String]) -> Result<Vec<TagRow>, rusqlite::Error> {
        let mut sql = String::from("SELECT t.tag, COUNT(*) FROM song_tags t");
        let mut bindings: Vec<Value> = Vec::new();
        if !hidden.is_empty() {
            sql.push_str(" JOIN songs s ON s.id = t.song_id WHERE 1 = 1");
            push_package_exclusion(&mut sql, &mut bindings, "s.package_id", hidden);
        }
        sql.push_str(" GROUP BY t.tag ORDER BY COUNT(*) DESC, t.tag");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok(TagRow {
                tag: row.get(0)?,
                songs: usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            })
        })?;
        rows.collect()
    }

    fn languages(&self, hidden: &[String]) -> Result<Vec<LanguageRow>, rusqlite::Error> {
        let mut sql = String::from(
            "SELECT language, COUNT(*) FROM songs WHERE language IS NOT NULL AND language != ''",
        );
        let mut bindings: Vec<Value> = Vec::new();
        push_package_exclusion(&mut sql, &mut bindings, "package_id", hidden);
        sql.push_str(" GROUP BY language ORDER BY COUNT(*) DESC, language");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok(LanguageRow::new(
                row.get::<_, String>(0)?,
                usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?;
        rows.collect()
    }

    /// Every package a mirrored song comes from, by name.
    ///
    /// Read from the songs rather than from `packages`, so the list is exactly what can be hidden,
    /// and a package whose name did not arrive is listed under its id rather than left out.
    fn packages(&self) -> Result<Vec<PackageRow>, rusqlite::Error> {
        let mut statement = self.conn.prepare(
            "SELECT s.package_id, COALESCE(p.name, s.package_id), COUNT(*)
             FROM songs s LEFT JOIN packages p ON p.id = s.package_id
             GROUP BY s.package_id ORDER BY 2 COLLATE NOCASE, s.package_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PackageRow {
                id: row.get(0)?,
                name: row.get(1)?,
                songs: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
            })
        })?;
        rows.collect()
    }
}

/// Appends `AND <column> NOT IN (…)` for the packages a person hid, binding one value per id.
///
/// The twin of the helper in `km_catalog::search`, so both remotes leave out the same songs. An
/// empty list appends nothing.
fn push_package_exclusion(
    sql: &mut String,
    bindings: &mut Vec<Value>,
    column: &str,
    packages: &[String],
) {
    if packages.is_empty() {
        return;
    }
    let mut placeholders = Vec::with_capacity(packages.len());
    for package in packages {
        bindings.push(Value::Text(package.clone()));
        placeholders.push(format!("?{}", bindings.len()));
    }
    sql.push_str(&format!(
        " AND {column} NOT IN ({})",
        placeholders.join(", ")
    ));
}

/// What one browse request becomes, in SQL.
///
/// A struct rather than a four-element tuple because the fourth element is the one that is easy to
/// get wrong — see the `ranked` note in [`Mirror::search`].
struct Conditions {
    where_sql: String,
    joins: String,
    bindings: Vec<Value>,
    /// Whether the FTS table was actually joined, and so whether `f.rank` exists to order by.
    ranked: bool,
}

/// A bound value. Local, because the two rusqlite types this needs are not one type.
#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Integer(i64),
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            Value::Text(text) => text.to_sql(),
            Value::Integer(number) => number.to_sql(),
        }
    }
}

/// Reads a song's code from the column that holds it, which is always the first.
///
/// The mirror's own copy of `km-catalog`'s helper, for the same reason `escape_like` is duplicated
/// below: the machine's version is internal to that crate, and this is three lines.
fn read_code(row: &rusqlite::Row<'_>) -> rusqlite::Result<SongCode> {
    Ok(SongCode::new(row.get(0)?))
}

fn read_song(row: &rusqlite::Row<'_>) -> rusqlite::Result<SongDto> {
    Ok(SongDto {
        number: read_code(row)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        language: row.get(3)?,
        kind: km_kmpkg::SongKind::from_wire(&row.get::<_, String>(4)?),
        duration_ms: row.get(5)?,
        suitability: row.get(6)?,
        melody_available: row.get::<_, i64>(7)? != 0,
        default_transpose: row.get(8)?,
        package_id: row.get(9)?,
        // Mirrored, and the one field here that no page draws: it is carried so that a favorite can
        // be resolved by what its song *is* when the number it was filed under has moved. See
        // `Mirror::resolve`.
        content_hash: row.get(10)?,
        // **Not mirrored, deliberately.** This table stores what the offline pages actually draw,
        // and none of them shows a lyric preview; carrying it would mean a column, an INSERT, two
        // SELECT lists, a `read_song` and a place in `discard_unless_current` — five edits for a
        // field nothing reads. The field arrives on the wire and is dropped here rather than
        // refused, which is the whole reason `SongDto::lyric_preview` carries `serde(default)`.
        // Whenever a page wants it, the mirror is re-downloadable by design and a refresh fills it.
        lyric_preview: Vec::new(),
        // Mirrored where the preview is not, and the test above the two is what the pages draw: the
        // tag filter has to work with the machine switched off, which is what this database is for.
        tags: row
            .get::<_, String>(11)?
            .split(',')
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect(),
    })
}

/// Escapes the two characters `LIKE` treats as wildcards, plus the escape character.
///
/// The same job `km_catalog::escape_like` does, and for the same reason: without it, an artist search
/// for `50%` matches every artist there is. Not shared, because that one is `pub(crate)` to a crate
/// this does not belong to, and eight lines is a poorer reason to widen an API than it looks.
fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The mirror behind the remote's catalog trait.
///
/// One mutex around one connection. A pool would be machinery for contention that cannot occur: this
/// is one person's phone, and the writes are a catalog refresh that happens about once a month.
#[derive(Clone)]
pub struct MirrorSongs {
    inner: Arc<Mutex<Mirror>>,
}

impl MirrorSongs {
    /// Wraps an open mirror.
    pub fn new(mirror: Mirror) -> Self {
        Self {
            inner: Arc::new(Mutex::new(mirror)),
        }
    }

    /// The mirror itself, for the importer.
    pub fn handle(&self) -> Arc<Mutex<Mirror>> {
        Arc::clone(&self.inner)
    }

    /// Runs a query on a blocking thread.
    ///
    /// The lock is taken and released inside the closure, so it is never held across an `await` —
    /// the same discipline `km-package-builder`'s `State::blocking` keeps, and for the same reason.
    async fn blocking<T, F>(&self, work: F) -> Result<T, RemoteError>
    where
        F: FnOnce(&mut Mirror) -> Result<T, rusqlite::Error> + Send + 'static,
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
        .map_err(|error| RemoteError::Failed(error.to_string()))
    }
}

#[async_trait::async_trait]
impl Songs for MirrorSongs {
    async fn search(&self, query: &BrowseQuery) -> Result<SongPage, RemoteError> {
        let query = query.clone();
        self.blocking(move |mirror| mirror.search(&query)).await
    }

    async fn song(&self, number: SongCode) -> Result<Option<SongDto>, RemoteError> {
        self.blocking(move |mirror| mirror.song(number)).await
    }

    async fn songs_by_number(&self, numbers: &[SongCode]) -> Result<Vec<SongDto>, RemoteError> {
        let numbers = numbers.to_vec();
        self.blocking(move |mirror| mirror.songs_by_number(&numbers))
            .await
    }

    async fn resolve(&self, refs: &[SongRef]) -> Result<Vec<Resolution>, RemoteError> {
        let refs = refs.to_vec();
        self.blocking(move |mirror| mirror.resolve(&refs)).await
    }

    async fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<ArtistRow>, RemoteError> {
        let contains = contains.map(str::to_owned);
        let hidden = hidden.to_vec();
        self.blocking(move |mirror| mirror.artists(contains.as_deref(), &hidden))
            .await
    }

    async fn languages(&self, hidden: &[String]) -> Result<Vec<LanguageRow>, RemoteError> {
        let hidden = hidden.to_vec();
        self.blocking(move |mirror| mirror.languages(&hidden)).await
    }

    async fn tags(&self, hidden: &[String]) -> Result<Vec<TagRow>, RemoteError> {
        let hidden = hidden.to_vec();
        self.blocking(move |mirror| mirror.tags(&hidden)).await
    }

    async fn packages(&self) -> Result<Vec<PackageRow>, RemoteError> {
        self.blocking(|mirror| mirror.packages()).await
    }

    async fn count(&self) -> Result<usize, RemoteError> {
        self.blocking(|mirror| mirror.count()).await
    }
}

/// Throws the mirror away when it is not the shape `schema.sql` writes, so it is fetched again.
///
/// **Discarded rather than converted, because this file is a copy.** A machine still holds what it is
/// a copy of, so the cost of dropping it is one re-download, and `favorites.sqlite` sits in a second
/// database beside it for exactly this reason: a collection cannot go with it. Converting would be
/// worse than slower — tags, content hashes and package names come off the wire and cannot be derived
/// from anything here, so a column added empty would read as *nothing tagged* and *no hash recorded*
/// until the machine's catalog version happened to move.
///
/// **The shape is the number.** The mirror carries no version, so what makes it current is having
/// every column and table the queries in this file read. `CREATE ... IF NOT EXISTS` in `schema.sql`
/// cannot tell a current table from an older one, which is why this runs first.
///
/// **`meta` goes with the songs, and that is the part worth not getting wrong.** `sync::refresh`
/// asks [`Mirror::is_current`] whether the machine's instance and catalog version already match
/// what is stored here, and skips the download when they do. Dropping the songs while leaving that
/// pair behind would answer "already up to date" over an empty table, so the browse page would come
/// back working and permanently empty — a failure in which nothing looks broken.
fn discard_unless_current(conn: &Connection) -> Result<(), rusqlite::Error> {
    // A mirror that does not exist yet is created correctly by `schema.sql`.
    if !has_table(conn, "songs")? {
        return Ok(());
    }
    let mut current = has_table(conn, "packages")?;
    for column in ["initial", "sort_artist", "tags", "content_hash"] {
        current = current && has_column(conn, "songs", column)?;
    }
    if current {
        return Ok(());
    }
    tracing::info!(
        "this copy of the catalog is not the current shape; discarding it to fetch again"
    );
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS songs_fts_insert;
         DROP TRIGGER IF EXISTS songs_fts_delete;
         DROP TRIGGER IF EXISTS songs_fts_update;
         DROP TABLE IF EXISTS songs_fts;
         DROP TABLE IF EXISTS song_tags;
         DROP TABLE IF EXISTS packages;
         DROP TABLE IF EXISTS songs;
         DROP TABLE IF EXISTS meta;",
    )
}

/// Whether a table exists yet.
///
/// `pub(crate)` for [`crate::favdb`], which asks the same question of the collection. The pair is
/// this crate's one spelling of "probe the shape, there is no version number", and a second copy is
/// exactly the drift the header above warns about.
pub(crate) fn has_table(conn: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Whether a table already carries a column. See [`has_table`] for why both are `pub(crate)`.
pub(crate) fn has_column(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, rusqlite::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn song(number: u32, title: &str, artist: Option<&str>, language: Option<&str>) -> SongDto {
        SongDto {
            number: SongCode::new(number),
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            language: language.map(str::to_owned),
            kind: km_kmpkg::SongKind::Midi,
            duration_ms: 200_000,
            suitability: Some(8),
            melody_available: true,
            default_transpose: 0,
            package_id: "vol1".to_owned(),
            content_hash: Some(format!("{number:032x}")),
            lyric_preview: Vec::new(),
            tags: Vec::new(),
        }
    }

    fn mirror_with(songs: &[SongDto]) -> Mirror {
        let mut mirror = Mirror::open_in_memory().expect("open");
        mirror.replace("machine-1", 7, songs, &[]).expect("import");
        mirror
    }

    /// The same song, filed in a named package under a named hash.
    fn song_in(number: u32, package: &str, hash: &str) -> SongDto {
        SongDto {
            package_id: package.to_owned(),
            content_hash: Some(hash.to_owned()),
            ..song(number, "Tempo Perdido", Some("Legião"), Some("pt"))
        }
    }

    /// A hidden package leaves the search and every picker, and still answers a song number and a
    /// favorite, which are the two ways a person names one song on purpose.
    #[test]
    fn a_hidden_package_leaves_the_search_and_the_pickers_but_not_a_lookup() {
        let tagged = |number, title, artist, language, package: &str| SongDto {
            package_id: package.to_owned(),
            tags: vec!["rock".to_owned()],
            ..song(number, title, Some(artist), Some(language))
        };
        let mut mirror = Mirror::open_in_memory().expect("open");
        mirror
            .replace(
                "machine-1",
                7,
                &[
                    tagged(1001, "One", "Cazuza", "pt", "kept"),
                    tagged(2001, "Two", "Only Hidden", "en", "hidden"),
                    tagged(2002, "Three", "Cazuza", "pt", "hidden"),
                ],
                &[("kept".to_owned(), "Rock Brasil".to_owned())],
            )
            .expect("import");
        let hide = vec!["hidden".to_owned()];

        let page = mirror
            .search(&BrowseQuery {
                hidden_packages: hide.clone(),
                limit: 50,
                ..Default::default()
            })
            .expect("search");
        let titles: Vec<&str> = page.songs.iter().map(|song| song.title.as_str()).collect();
        assert_eq!(titles, ["One"]);
        assert_eq!(page.total, Some(1), "the count agrees with the rows");

        let artists = mirror.artists(None, &hide).expect("artists");
        assert_eq!(
            artists,
            [ArtistRow {
                name: "Cazuza".to_owned(),
                songs: 1
            }]
        );
        let languages = mirror.languages(&hide).expect("languages");
        assert_eq!(languages, [LanguageRow::new("pt", 1)]);
        assert_eq!(
            mirror.tags(&hide).expect("tags"),
            [TagRow {
                tag: "rock".to_owned(),
                songs: 1
            }]
        );

        assert!(mirror.song(SongCode::new(2001)).expect("song").is_some());
        let found = mirror
            .resolve(&[asked(2001, Some("hidden"), Some(&format!("{:032x}", 2001)))])
            .expect("resolve");
        assert!(found[0].song().is_some());

        assert_eq!(
            mirror.packages().expect("packages"),
            [
                PackageRow {
                    id: "hidden".to_owned(),
                    name: "hidden".to_owned(),
                    songs: 2
                },
                PackageRow {
                    id: "kept".to_owned(),
                    name: "Rock Brasil".to_owned(),
                    songs: 1
                },
            ],
            "by name, and a package with no name arrives under its id"
        );
    }

    fn asked(code: u32, package: Option<&str>, hash: Option<&str>) -> SongRef {
        SongRef {
            code: SongCode::new(code),
            package_id: package.map(str::to_owned),
            content_hash: hash.map(str::to_owned),
        }
    }

    /// The rung that makes the whole thing worth doing: the number the favorite was filed under now
    /// belongs to a *different package's* song, and resolving by content finds the right one anyway.
    ///
    /// This is the re-banking failure written out. Without the first rung the favorite would resolve
    /// — to the wrong song, silently, which is worse than resolving to nothing.
    #[test]
    fn a_song_whose_bank_moved_is_found_by_its_package_and_hash() {
        let mirror = mirror_with(&[
            // `vol1` was re-banked from 1 to 2, so its song is at 2001 now...
            song_in(2001, "vol1", "aaa"),
            // ...and `vol2` moved into the bank it left, so 1001 is a real number for another song.
            song_in(1001, "vol2", "bbb"),
        ]);
        let found = mirror
            .resolve(&[asked(1001, Some("vol1"), Some("aaa"))])
            .expect("resolve");
        let song = found[0].song().expect("the favorite resolves");
        assert_eq!(
            song.number,
            SongCode::new(2001),
            "found by content, not code"
        );
        assert_eq!(song.package_id, "vol1");
        assert!(found[0].moved(), "and the caller is told to refile it");
    }

    /// The second rung: the same recording, re-packaged under an id nothing here has seen.
    #[test]
    fn a_recording_repackaged_elsewhere_is_still_found_by_its_hash() {
        let mirror = mirror_with(&[song_in(3001, "vol3", "aaa")]);
        let found = mirror
            .resolve(&[asked(1001, Some("gone"), Some("aaa"))])
            .expect("resolve");
        assert_eq!(found[0].song().expect("found").number, SongCode::new(3001));
    }

    /// Two packages holding one recording is reported at install rather than refused, so the second
    /// rung has a real choice to make and must make the same one every time.
    #[test]
    fn the_hash_rung_picks_the_lowest_number_every_time() {
        let mirror = mirror_with(&[
            song_in(5005, "vol5", "shared"),
            song_in(4004, "vol4", "shared"),
            song_in(6006, "vol6", "shared"),
        ]);
        for _ in 0..3 {
            let found = mirror
                .resolve(&[asked(9999, Some("gone"), Some("shared"))])
                .expect("resolve");
            assert_eq!(
                found[0].song().expect("found").number,
                SongCode::new(4004),
                "the lowest number, and the same one each time"
            );
        }
    }

    /// The number rung is bounded by the hash: a song holding that number whose own hash is a
    /// different one is a different recording, and answering with it is the *wrong song, right
    /// folder* failure the rungs above exist to prevent. A collection looked at on a second machine
    /// meets this on every song that machine does not have, so it is the ordinary case there.
    #[test]
    fn a_number_belonging_to_another_recording_is_refused_rather_than_answered() {
        let mirror = mirror_with(&[
            // `vol1` is installed here and holds a song the favorite is not about...
            song_in(3001, "vol1", "ccc"),
            // ...while the number the favorite was filed under belongs to another package.
            song_in(1001, "vol2", "bbb"),
        ]);
        let found = mirror
            .resolve(&[asked(1001, Some("vol1"), Some("aaa"))])
            .expect("resolve");
        assert_eq!(
            found[0].outcome,
            Err(Miss::RecordingAbsent),
            "the package is here and the recording is not, so the favorite lists as nothing \
             rather than as somebody else's song"
        );
    }

    /// The other side of the same rule, and why it is the *candidate's* null that is tested rather
    /// than the favorite being identified at all: a package built before manifests carried a hash
    /// mirrors none, so refusing it here would strand every favorite of one.
    #[test]
    fn a_number_whose_song_records_no_hash_still_answers_an_identified_favorite() {
        let mirror = mirror_with(&[SongDto {
            content_hash: None,
            ..song_in(1001, "vol1", "unused")
        }]);
        let found = mirror
            .resolve(&[asked(1001, Some("vol1"), Some("aaa"))])
            .expect("resolve");
        assert_eq!(
            found[0].song().expect("found").number,
            SongCode::new(1001),
            "a null hash is unknown, never different"
        );
    }

    /// A null hash means *unknown*, never *different* — a package built before the manifest carried
    /// one is the ordinary reason for it, and treating it as a mismatch would strand every favorite
    /// of such a package.
    #[test]
    fn a_favorite_carrying_no_hash_falls_through_to_its_number() {
        let mirror = mirror_with(&[song_in(1001, "vol1", "aaa")]);
        let found = mirror.resolve(&[asked(1001, None, None)]).expect("resolve");
        assert_eq!(found[0].song().expect("found").number, SongCode::new(1001));
        assert!(!found[0].moved(), "nothing moved, so nothing to refile");
    }

    /// The two absences are told apart, because the remedies differ: one is a package somebody can
    /// go and install, the other is not.
    #[test]
    fn a_miss_says_whether_the_package_or_only_the_recording_is_absent() {
        let mirror = mirror_with(&[song_in(1001, "vol1", "aaa")]);
        let found = mirror
            .resolve(&[
                asked(7007, Some("never-installed"), Some("zzz")),
                asked(7008, Some("vol1"), Some("zzz")),
                asked(7009, None, None),
            ])
            .expect("resolve");
        assert_eq!(found[0].outcome, Err(Miss::PackageAbsent));
        assert_eq!(found[1].outcome, Err(Miss::RecordingAbsent));
        assert_eq!(found[2].outcome, Err(Miss::NumberAbsent));
    }

    /// The answers come back one per question and in the order asked, because a caller pairs them
    /// with the favorites it sent.
    #[test]
    fn resolve_answers_once_per_favorite_in_the_order_asked() {
        let mirror = mirror_with(&[song_in(1001, "vol1", "aaa"), song_in(1002, "vol1", "bbb")]);
        let found = mirror
            .resolve(&[
                asked(1002, Some("vol1"), Some("bbb")),
                asked(4242, None, None),
                asked(1001, Some("vol1"), Some("aaa")),
            ])
            .expect("resolve");
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].song().expect("found").number, SongCode::new(1002));
        assert!(found[1].outcome.is_err());
        assert_eq!(found[2].song().expect("found").number, SongCode::new(1001));
    }

    /// A mirror as it was written while a code was a prefix and a number: `UNIQUE (prefix, number)`
    /// with a `prefix` column beside it. Reproduced by hand rather than kept as a fixture file,
    /// because what matters is the one difference and a binary blob would hide it.
    ///
    /// **The shape before *that* is not tested here any more, and cannot be**: a pre-code mirror had
    /// `number INTEGER NOT NULL UNIQUE` and no prefix, which is exactly the current schema — there is
    /// no column to tell them apart, and nothing to repair if there were.
    fn write_prefix_era_mirror(path: &Path) {
        let conn = Connection::open(path).expect("create");
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE songs (
                 id                INTEGER PRIMARY KEY,
                 prefix            TEXT    NOT NULL DEFAULT '',
                 number            INTEGER NOT NULL,
                 title             TEXT    NOT NULL,
                 artist            TEXT,
                 language          TEXT,
                 kind              TEXT    NOT NULL DEFAULT 'midi',
                 duration_ms       INTEGER NOT NULL,
                 suitability       INTEGER,
                 melody_available  INTEGER NOT NULL DEFAULT 0,
                 default_transpose INTEGER NOT NULL DEFAULT 0,
                 package_id        TEXT    NOT NULL DEFAULT '',
                 sort_key          TEXT    NOT NULL DEFAULT '',
                 alpha             TEXT,
                 UNIQUE (prefix, number)
             );
             CREATE VIRTUAL TABLE songs_fts USING fts5(
                 title, artist, content='songs', content_rowid='id',
                 tokenize='unicode61 remove_diacritics 2');
             INSERT INTO songs (id, prefix, number, title, duration_ms, sort_key, alpha)
                 VALUES (1, 'BR', 500, 'Tempo Perdido', 200000, 'tempo perdido', 'T');
             INSERT INTO meta (key, value) VALUES ('machine_id', 'machine-1');
             INSERT INTO meta (key, value) VALUES ('catalog_version', '7');",
        )
        .expect("old schema");
    }

    /// Opening a mirror in a shape this build does not write must not fail every query afterwards.
    ///
    /// `schema.sql` cannot repair one, being `CREATE ... IF NOT EXISTS` throughout, so without
    /// [`discard_unless_current`] the browse page would answer `no such column` and name nothing
    /// anybody could act on. The fixture is a mirror with a `prefix` column and none of the current
    /// ones.
    #[test]
    fn a_mirror_from_the_prefix_era_is_discarded_rather_than_failing_every_query() {
        let scratch = crate::testing::Scratch::new("oldmirror");
        let dir = scratch.path();
        write_prefix_era_mirror(&dir.join(MIRROR_FILE));

        let mirror = Mirror::open(dir).expect("opening an old mirror must succeed");

        // The query that used to fail. It has to *run*; that it returns nothing is the point below.
        let page = mirror.search(&query(None)).expect("browsing must not fail");
        assert!(
            page.songs.is_empty(),
            "the old rows should have been discarded"
        );

        // **The half that would otherwise be a second bug.** `sync::refresh` skips the download when
        // the stored instance and version already match the machine's, so a mirror that dropped its
        // songs but kept its `meta` would report itself up to date over an empty table and never
        // fetch again -- a browse page that looks fine and is permanently empty.
        assert_eq!(mirror.mirrored().expect("meta"), None);
        assert!(!mirror.is_current("machine-1", 7));
    }

    fn query(text: Option<&str>) -> BrowseQuery {
        BrowseQuery {
            text: text.map(str::to_owned),
            limit: 50,
            ..BrowseQuery::default()
        }
    }

    /// The property the whole mirror exists for: a search here means what the same search means on
    /// the machine, because both build the MATCH string with the same function and both use
    /// `remove_diacritics 2`.
    #[test]
    fn a_search_folds_accents_the_way_the_machine_does() {
        let mirror = mirror_with(&[song(1, "Coração Com Buraquinhos", Some("Zé Ramalho"), None)]);
        let page = mirror.search(&query(Some("coracao"))).expect("search");
        assert_eq!(page.songs.len(), 1);
        assert_eq!(page.total, Some(1));
    }

    /// FTS5 treats these as operators. Neither may reach it unescaped.
    #[test]
    fn punctuation_a_person_types_is_not_a_syntax_error() {
        let mirror = mirror_with(&[song(1, "Highway to Hell", Some("AC/DC"), None)]);
        for text in ["AC/DC", "rock 'n' roll", "???", "-", "NEAR"] {
            let page = mirror.search(&query(Some(text))).expect(text);
            let _ = page.songs.len();
        }
    }

    /// A query that sanitises down to nothing has narrowed by nothing.
    #[test]
    fn a_query_of_pure_punctuation_matches_everything_rather_than_failing() {
        let mirror = mirror_with(&[song(1, "One", None, None), song(2, "Two", None, None)]);
        let page = mirror.search(&query(Some("!!!"))).expect("search");
        assert_eq!(page.songs.len(), 2);
    }

    /// SQLite's BINARY collation puts every accented character after `Z`, so ordering by the title
    /// would bury these at the end of the alphabet. The folded key is what stops that.
    #[test]
    fn songs_sort_by_a_folded_key_rather_than_by_the_raw_title() {
        let mirror = mirror_with(&[
            song(1, "Zebra", None, None),
            song(2, "Águas de Março", None, None),
            song(3, "Banana", None, None),
        ]);
        let page = mirror
            .search(&BrowseQuery {
                order: Order::Title,
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        let titles: Vec<&str> = page.songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Águas de Março", "Banana", "Zebra"]);
    }

    /// The twin of `km_catalog`'s `songs_sharing_a_title_sort_by_performer`, deliberately in the
    /// same words so that one grep finds the pair: a song number is not an order anybody can read,
    /// so the performer decides between two songs called *Goodbye*. The unnamed one comes last
    /// within the title, which the `= ''` term is what buys — an empty fold sorts first.
    #[test]
    fn songs_sharing_a_title_sort_by_performer() {
        let mirror = mirror_with(&[
            song(1, "Goodbye", Some("Spice Girls"), None),
            song(2, "Goodbye", None, None),
            song(3, "Goodbye", Some("Air Supply"), None),
            song(4, "Gotta Tell You", Some("Aaron"), None),
        ]);
        let page = mirror
            .search(&BrowseQuery {
                order: Order::Title,
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        let listed: Vec<(&str, Option<&str>)> = page
            .songs
            .iter()
            .map(|s| (s.title.as_str(), s.artist.as_deref()))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("Goodbye", Some("Air Supply")),
                ("Goodbye", Some("Spice Girls")),
                ("Goodbye", None),
                ("Gotta Tell You", Some("Aaron")),
            ]
        );
    }

    /// The artist leg was the one still on `COLLATE NOCASE`, which is ASCII-only — so the same list
    /// folded its titles and did not fold its artists, and `Ângela` came after `Zeca`.
    #[test]
    fn artists_sort_by_a_folded_key_rather_than_by_the_raw_name() {
        let mirror = mirror_with(&[
            song(1, "One", Some("Zeca"), None),
            song(2, "Two", Some("Ângela"), None),
            song(3, "Three", Some("Bebel"), None),
        ]);
        let page = mirror
            .search(&BrowseQuery {
                order: Order::Artist,
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        let artists: Vec<&str> = page
            .songs
            .iter()
            .filter_map(|s| s.artist.as_deref())
            .collect();
        assert_eq!(artists, vec!["Ângela", "Bebel", "Zeca"]);
    }

    /// `artist COLLATE NOCASE` put a song with no artist first, because SQLite sorts NULL first.
    /// `sort_artist` is `NOT NULL DEFAULT ''` and the fold of no artist is the empty string, which
    /// sorts first too — so nothing moved, which is the property worth pinning.
    #[test]
    fn songs_with_no_artist_come_first_in_artist_order() {
        let mirror = mirror_with(&[
            song(1, "One", Some("Ângela"), None),
            song(2, "Two", None, None),
            song(3, "Three", Some("Bebel"), None),
        ]);
        let page = mirror
            .search(&BrowseQuery {
                order: Order::Artist,
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        let titles: Vec<&str> = page.songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Two", "One", "Three"]);
    }

    #[test]
    fn the_artist_list_is_ordered_by_the_folded_name() {
        let mirror = mirror_with(&[
            song(1, "One", Some("Zeca"), None),
            song(2, "Two", Some("Ângela"), None),
            song(3, "Three", Some("Bebel"), None),
        ]);
        let names: Vec<String> = mirror
            .artists(None, &[])
            .expect("artists")
            .into_iter()
            .map(|row| row.name)
            .collect();
        assert_eq!(names, vec!["Ângela", "Bebel", "Zeca"]);
    }

    /// The narrowing folds too, so the list and the songs it opens onto agree about who the band is
    /// however the singer spelled the name.
    #[test]
    fn the_artist_list_is_narrowed_accent_insensitively() {
        let mirror = mirror_with(&[
            song(1, "One", Some("Legião Urbana"), None),
            song(2, "Two", Some("Cazuza"), None),
        ]);
        let names: Vec<String> = mirror
            .artists(Some("legiao"), &[])
            .expect("artists")
            .into_iter()
            .map(|row| row.name)
            .collect();
        assert_eq!(names, vec!["Legião Urbana"]);
    }

    #[test]
    fn the_a_to_z_strip_buckets_digits_together_and_folds_accents() {
        let mirror = mirror_with(&[
            song(1, "Águas de Março", None, None),
            song(2, "99 Luftballons", None, None),
            song(3, "Banana", None, None),
        ]);
        let by_initial = |initial: char| {
            mirror
                .search(&BrowseQuery {
                    initial: Some(initial),
                    limit: 50,
                    ..BrowseQuery::default()
                })
                .expect("search")
                .songs
                .len()
        };
        assert_eq!(by_initial('A'), 1, "Águas is filed under A");
        assert_eq!(by_initial('#'), 1);
        assert_eq!(by_initial('B'), 1);
        assert_eq!(by_initial('Z'), 0);
    }

    /// Opening an artist must show *that* artist, not everybody whose name contains theirs.
    #[test]
    fn an_artist_drill_down_is_exact_where_the_search_box_is_a_substring() {
        let mirror = mirror_with(&[
            song(1, "One", Some("Ana"), None),
            song(2, "Two", Some("Ana Carolina"), None),
        ]);
        let exact = mirror
            .search(&BrowseQuery {
                artist: Some(ArtistFilter::Exactly("Ana".to_owned())),
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        assert_eq!(exact.songs.len(), 1);

        let contains = mirror
            .search(&BrowseQuery {
                artist: Some(ArtistFilter::Contains("ana".to_owned())),
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        assert_eq!(contains.songs.len(), 2);
    }

    #[test]
    fn a_language_is_matched_whole_and_not_as_a_substring() {
        let mirror = mirror_with(&[
            song(1, "One", None, Some("pt")),
            song(2, "Two", None, Some("ja")),
        ]);
        let narrowed = mirror
            .search(&BrowseQuery {
                language: Some("PT".to_owned()),
                limit: 50,
                ..BrowseQuery::default()
            })
            .expect("search");
        assert_eq!(
            narrowed.songs.len(),
            1,
            "a code is matched case-insensitively"
        );
    }

    /// Tags survive the sync and widen to the union **with the machine switched off**.
    ///
    /// Which is the reason this database exists at all, and the reason tags are mirrored where
    /// `lyric_preview` deliberately is not: nothing offline draws a preview, and the tag filter is a
    /// control on the offline page.
    #[test]
    fn tags_are_mirrored_and_widen_to_the_songs_carrying_any_of_them() {
        let tagged = |number: u32, title: &str, tags: &[&str]| {
            let mut dto = song(number, title, None, Some("pt"));
            dto.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
            dto
        };
        // A song under each word alone, and the counts kept apart: a fixture where `brasil` adds no
        // row of its own cannot tell a union from a filter that reads the first tag and stops.
        let mirror = mirror_with(&[
            tagged(1, "Both", &["rock", "brasil"]),
            tagged(2, "Rock only", &["rock"]),
            tagged(3, "Neither", &[]),
            tagged(4, "Brasil only", &["brasil"]),
            tagged(5, "Rock too", &["rock"]),
        ]);

        let titles = |tags: &[&str]| {
            let mut found: Vec<String> = mirror
                .search(&BrowseQuery {
                    tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
                    limit: 50,
                    ..BrowseQuery::default()
                })
                .expect("search")
                .songs
                .into_iter()
                .map(|song| song.title)
                .collect();
            found.sort();
            found
        };

        assert_eq!(titles(&["rock"]), ["Both", "Rock only", "Rock too"]);
        assert_eq!(
            titles(&["rock", "brasil"]),
            ["Both", "Brasil only", "Rock only", "Rock too"],
            "OR, not AND"
        );
        // No tags is no tag filter, which is the case the `IN` has to be guarded against.
        assert_eq!(
            titles(&[]),
            ["Both", "Brasil only", "Neither", "Rock only", "Rock too"]
        );

        // The vocabulary is what the songs are filed under, commonest first — what the picker draws.
        assert_eq!(
            mirror.tags(&[]).expect("tags"),
            vec![
                TagRow {
                    tag: "rock".to_owned(),
                    songs: 3
                },
                TagRow {
                    tag: "brasil".to_owned(),
                    songs: 2
                },
            ]
        );

        // And they ride back on the row, so a page never has to ask a second question.
        let one = mirror
            .song(SongCode::new(1))
            .expect("query")
            .expect("song 1");
        assert_eq!(one.tags, ["brasil", "rock"]);
    }

    /// A mirror without the tags column is discarded rather than given an empty one.
    ///
    /// Adding the column would compile and would be wrong: nothing here can derive a tag,
    /// so every song would read as untagged, `song_tags` would index nothing, and the picker would
    /// draw nothing — and `sync::refresh` would not repair it, because the machine's catalog version
    /// has not moved. Discarding costs one re-download of what the machine still has.
    #[test]
    fn a_mirror_that_predates_tags_is_thrown_away_rather_than_left_untagged() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(include_str!("schema.sql"))
            .expect("schema");
        conn.execute_batch(
            "INSERT INTO songs (number, title, duration_ms) VALUES (1, 'One', 1000);
             INSERT INTO meta (key, value) VALUES ('machine_id', 'machine-1');
             INSERT INTO meta (key, value) VALUES ('catalog_version', '7');",
        )
        .expect("a mirror with songs in it");
        // Wind the schema back to before tags existed. The index names the column, so it goes first.
        conn.execute_batch(
            "DROP TABLE song_tags;
             ALTER TABLE songs DROP COLUMN tags;",
        )
        .expect("wind back");

        discard_unless_current(&conn).expect("discard");
        conn.execute_batch(include_str!("schema.sql"))
            .expect("schema");

        let mirror = Mirror { conn };
        assert_eq!(
            mirror.count().expect("count"),
            0,
            "the songs were discarded"
        );
        assert_eq!(
            mirror.mirrored().expect("mirrored"),
            None,
            "and `meta` went with them, or a refresh would answer `already up to date` over an \
             empty table"
        );
    }

    /// The export says what a machine *has*, not what it has gained. A song removed by uninstalling
    /// a package has to leave the mirror too, or it stays queueable from this phone and from nowhere
    /// else.
    #[test]
    fn importing_replaces_the_catalog_rather_than_merging_into_it() {
        let mut mirror = mirror_with(&[song(1, "One", None, None), song(2, "Two", None, None)]);
        mirror
            .replace("machine-1", 8, &[song(1, "One", None, None)], &[])
            .expect("re-import");
        assert_eq!(mirror.count().expect("count"), 1);
        // And the search index went with it, which is what the triggers are for.
        let page = mirror.search(&query(Some("Two"))).expect("search");
        assert!(page.songs.is_empty());
    }

    #[test]
    fn a_mirror_knows_which_machine_and_which_version_it_holds() {
        let mirror = mirror_with(&[song(1, "One", None, None)]);
        assert_eq!(
            mirror.mirrored().expect("meta"),
            Some(("machine-1".to_owned(), 7))
        );
        assert!(mirror.is_current("machine-1", 7));
        assert!(!mirror.is_current("machine-1", 8));
    }

    /// The trap this guards: the same version number against a different machine is not "nothing has
    /// changed", it is a different catalog.
    #[test]
    fn the_same_version_from_a_different_machine_is_never_current() {
        let mirror = mirror_with(&[song(1, "One", None, None)]);
        assert!(!mirror.is_current("machine-2", 7));
    }

    #[test]
    fn a_folder_that_names_a_song_the_catalog_lost_skips_it_rather_than_failing() {
        let mirror = mirror_with(&[song(1, "One", None, None)]);
        let found = mirror
            .songs_by_number(&[SongCode::new(3), SongCode::new(1), SongCode::new(9)])
            .expect("lookup");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].number, SongCode::new(1));
    }

    #[test]
    fn songs_come_back_in_the_order_they_were_asked_for() {
        let mirror = mirror_with(&[
            song(1, "One", None, None),
            song(2, "Two", None, None),
            song(3, "Three", None, None),
        ]);
        let found = mirror
            .songs_by_number(&[SongCode::new(3), SongCode::new(1), SongCode::new(2)])
            .expect("lookup");
        let numbers: Vec<SongCode> = found.iter().map(|song| song.number).collect();
        assert_eq!(
            numbers,
            vec![SongCode::new(3), SongCode::new(1), SongCode::new(2)]
        );
    }

    #[test]
    fn paging_reports_whether_there_is_more() {
        let songs: Vec<SongDto> = (1..=5)
            .map(|n| song(n, &format!("Song {n}"), None, None))
            .collect();
        let mirror = mirror_with(&songs);
        let first = mirror
            .search(&BrowseQuery {
                limit: 2,
                ..BrowseQuery::default()
            })
            .expect("search");
        assert_eq!(first.songs.len(), 2);
        assert_eq!(first.total, Some(5));
        assert!(first.more);

        let last = mirror
            .search(&BrowseQuery {
                limit: 2,
                offset: 4,
                ..BrowseQuery::default()
            })
            .expect("search");
        assert_eq!(last.songs.len(), 1);
        assert!(!last.more);
    }
}
