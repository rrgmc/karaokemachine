//! The SQL these queries are assembled from, and the rows they come back as.
//!
//! Two kinds of thing, and they are here for the same reason. The fragment builders — `eff_title`,
//! `title_is_filename`, `browse_columns` — are expressions written once because the browse list, the
//! package contents, the duplicate review and the search index disagreeing about what a song is
//! called is exactly the confusion this tool exists to remove. The row readers are the other end of
//! the same pipe: `song_row` reads the columns `browse_columns` names, in that order, and a column
//! added to one without the other is the fault both halves living here is meant to make obvious.
//!
//! None of it touches `Db`. Everything takes a `&Connection` or a `&Row`, which is what lets the
//! `impl Db` blocks be split without any of them owning a fragment the others also need.

use super::*;

/// The SQL for the title to put in front of somebody: what a person typed, else what the file said,
/// else the file's own name.
///
/// One function rather than the expression written out at each call site, because the browse list,
/// the package contents, the duplicate review and the search index disagreeing about what a song is
/// called is exactly the confusion this tool exists to remove. `alias` is the table prefix, `"s."` or
/// `"a."` or empty.
///
/// `nullif` matters: a `@T` line with nothing after it parses to `Some("")`, which coalesce would
/// happily accept and render as the blank row this replaced.
pub(super) fn eff_title(alias: &str) -> String {
    format!("coalesce(nullif({alias}title, ''), nullif({alias}det_title, ''), {alias}stem, '')")
}

/// The SQL for the artist to put in front of somebody: what a person typed, else what the file said.
///
/// One function rather than the expression written out at each call site, for the reason
/// [`eff_title`] is one. It is a display expression and nothing more: the artist sort reads the
/// stored `sort_artist` column rather than an expression index over this, so nothing here has to
/// match a query's expression tree for tree the way SQLite requires of such an index.
///
/// There is deliberately no `nullif` here, unlike [`eff_title`]. An artist recorded as the empty
/// string is not the same as no artist at all, and the browse list has always sorted the two
/// differently — empty first, absent last. Adding `nullif` would merge them, which is a change to
/// what a curator sees and not one this function is entitled to make on its own.
pub(super) fn eff_artist(alias: &str) -> String {
    format!("coalesce({alias}artist, {alias}det_artist)")
}

/// How many songs one pass of [`refold_chunk`] folds.
///
/// Big enough that a corpus-sized backfill is sixty transactions rather than hundreds of thousands,
/// small enough that each one's write-ahead log drains instead of accumulating.
pub(super) const FOLD_CHUNK: usize = 10_000;

/// How many songs one pass of [`Db::backfill_language_guess`] reads.
///
/// A tenth of [`FOLD_CHUNK`], because a chunk of that one holds two folded names per row where a
/// chunk of this holds a whole lyric track — ten thousand of those is a corpus's worth of words in
/// memory at once, which is the thing that backfill is chunked to avoid.
pub(super) const GUESS_CHUNK: usize = 1_000;

/// Folds up to `limit` of the rows whose sort keys are missing, and says how many it did.
///
/// **The fold is `km_song::text::fold` and cannot be anything else.** It is the alphabet the A-Z
/// strip files by, the one the song book prints and the one both catalogs sort by, and a second
/// spelling of it here — in SQL, over its own accent table — would put this tool's browse list into
/// an alphabet of its own. That is the fault `km_song::text`'s header describes: it would look like
/// bad data rather than like a bug.
///
/// **`sort_artist` is an `Option` all the way through, and that is load-bearing.** [`eff_artist`] has
/// no `nullif`, so an artist recorded as the empty string is not the same as no artist at all, and
/// [`Filter::order_by`] sorts the two differently — empty first, absent last. Folding
/// `eff_artist.unwrap_or_default()` would merge them.
///
/// The rows are collected before any is written, because SQLite leaves the result of a `SELECT`
/// undefined if the table changes while it is still being stepped.
pub(super) fn refold_chunk(conn: &Connection, limit: usize) -> Result<usize, DbError> {
    let rows: Vec<(String, String, Option<String>)> = {
        let mut statement = conn.prepare(&format!(
            "SELECT id, {}, {} FROM songs WHERE sort_title IS NULL LIMIT ?1",
            eff_title(""),
            eff_artist("")
        ))?;
        let mapped = statement.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };
    if rows.is_empty() {
        return Ok(0);
    }
    let mut update =
        conn.prepare("UPDATE songs SET sort_title = ?2, sort_artist = ?3 WHERE id = ?1")?;
    for (id, title, artist) in &rows {
        update.execute(params![
            id,
            km_song::text::fold(title),
            artist.as_deref().map(km_song::text::fold),
        ])?;
    }
    Ok(rows.len())
}

/// The SQL for the language to act on: what a person chose, else what the file said, else what its
/// words read as.
///
/// One function rather than the expression at each call site, for the same reason [`eff_title`] is
/// one: the browse column, the language filter and the language sort all read it, and a song listed
/// under one language that opened under another would be exactly the confusion this tool removes.
///
/// **The three legs are in order of what stands behind them.** A person looked at the song; a file
/// made a statement about itself; a detector read the words and was sure enough to say so. Each is
/// worth less than the one before, and the last is written only above
/// [`km_langguess::MIN_CONFIDENCE`] — so a song still reaching the end of the coalesce with nothing
/// is one no witness could place, which is what the browse list's `unset` filter is for.
pub(super) fn eff_language(alias: &str) -> String {
    format!(
        "coalesce(nullif({alias}language, ''), {alias}det_language_tag, {alias}det_language_guess)"
    )
}

/// The terms every browse query carries and every `songs_browse_*` index is partial on.
///
/// **One function rather than the predicate at each call site, because the two sides have to match
/// exactly or the indexes stop being used.** SQLite serves a partial index only where the query
/// implies its `WHERE`, and a term added to [`Filter::to_sql`] but not to
/// [`Db::create_browse_indexes`] does not fail — it makes every sort read the whole corpus into a
/// temp B-tree with nothing anywhere saying why. Eleven copies of the predicate were eleven chances
/// for that.
///
/// `alias` is what the query prefixes its columns with and an index cannot have any, which is why
/// this takes one at all — the shape [`eff_title`] and [`eff_language`] above already have.
///
/// A song set aside as a near-duplicate is not here: `VersionsFilter` decides that one, and a
/// filter naming a single favorite drops it.
pub(super) fn browsable(alias: &str) -> String {
    format!(
        "{alias}merged_into IS NULL AND {}",
        DeletedFilter::Live.clause(alias)
    )
}

/// How many ids go into one `IN (…)`.
///
/// SQLite binds at most 999 parameters in a statement and nothing stops a caller handing more: the
/// header tick-box ticks a whole page, and a page is a constant that has moved before. Comfortably
/// under, leaving room for what is being written beside them.
pub(super) const ID_CHUNK: usize = 500;

/// `?n, ?n+1, …` for a batch of ids, and the bindings to go with them.
///
/// `first` is the number to start at, so a statement that binds something else at `?1` can say so.
pub(super) fn id_holes(ids: &[String], first: usize) -> (String, Vec<Binding>) {
    let holes = (first..first + ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let values = ids.iter().map(|id| Binding::Text(id.clone())).collect();
    (holes, values)
}

/// ` AND <nobody has said>`, or nothing.
///
/// **[`LanguageFilter::Unset`]'s own clause, asked for rather than restated.** *Unset* means neither
/// a language somebody typed nor one the file implied — which is what the browse bar's *language
/// unset* means and what its chip says — and two spellings of that would be two answers to which
/// songs a bulk write leaves alone.
pub(super) fn unset_narrowing(only_unset: bool) -> String {
    if !only_unset {
        return String::new();
    }
    LanguageFilter::Unset
        .clause(&eff_language(""))
        .map(|clause| format!(" AND {clause}"))
        .unwrap_or_default()
}

/// What `deleted_at` is set to by a delete or an undelete.
///
/// **A SQL expression rather than a bound value, because the time has to be the statement's.**
/// `'now'` is fixed for one step of a statement, so a write over a whole filter lands one timestamp
/// on every row it changes; a value bound from Rust would do the same, and this way the shape is
/// `crate::scan::timestamp()`'s character for character without a second place spelling it — UTC,
/// four-digit year, two digits everywhere else, `Z`. The same expression `songs_stamp_update` uses.
pub(super) fn deleted_value(deleted: bool) -> &'static str {
    if deleted {
        "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')"
    } else {
        "NULL"
    }
}

/// Whether the title being shown is only the file's name — nobody typed one and the file said none.
///
/// Rendered as a hint rather than hidden: on this corpus it is the difference between a song that has
/// been looked at and one that has not, which is the whole question a curator is answering.
pub(super) fn title_is_filename(alias: &str) -> String {
    format!("(nullif({alias}title, '') IS NULL AND nullif({alias}det_title, '') IS NULL)")
}

/// The letter a song files under: `A`–`Z`, `0`–`9`, or `#` for everything else.
///
/// Folding matters here rather than being a nicety. Much of this corpus is Portuguese, so `Águas de
/// Março` under `#` instead of `A` would not be a rounding error — it would be a letter of the
/// alphabet that quietly holds several thousand songs nobody browsing by name would find.
///
/// **Read off `sort_title`, which `km_song::text::fold` has already lower-cased and unaccented**,
/// rather than off an accent table of this crate's own. A second accent table here — two parallel
/// strings indexed with `instr` — covers a different set of characters than `fold_char` does, and
/// two of those silently disagreeing is the fault `km_song::text`'s header names. Reading the folded
/// column is what makes this tool agree with `km_song::text::initial` about where a song files
/// rather than agree by coincidence, which also means the A-Z strip here and the one on the offline
/// remote are the same strip.
///
/// One consequence of that agreement, worth knowing rather than discovering: `fold` strips leading
/// punctuation, so `¿Y ahora qué?` files under `Y` rather than under `#`. That is what the remote and
/// the printed book do, and a Spanish song is not a symbol. It also kills a
/// class of bug at the root — a stem with a leading space can no longer sort ahead of the letter its
/// song belongs under.
///
/// **Still one bucket per digit**, and `km_song::text::initial`'s single `#` for all ten is
/// deliberately not adopted: [`Initial::clause`] seeks this expression with an `IN` list of the
/// digits and `Initial::Symbol` with `= '#'`, so collapsing here would make the first match nothing
/// and the second match everything. The collapse belongs in the predicate, which is what the note on
/// [`Initial`] says.
pub(super) fn title_initial(alias: &str) -> String {
    let first = format!("substr({alias}sort_title, 1, 1)");
    format!(
        "CASE
            WHEN {first} GLOB '[0-9]' THEN {first}
            WHEN {first} GLOB '[a-z]' THEN upper({first})
            ELSE '#'
         END"
    )
}

/// The folder a file sits in, as a path ending in `/`. The empty string for a file at the root.
pub(super) fn parent_folder(path: &str) -> &str {
    match path.rfind('/') {
        Some(cut) => &path[..=cut],
        None => "",
    }
}

/// Every folder at or above `folder`, including the root and `folder` itself.
///
/// `a/b/` yields `a/b/`, `a/` and `""`. The root is always in the list, which is what gives the
/// `folders` table its one row with `parent IS NULL`.
pub(super) fn ancestors(folder: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut so_far = String::new();
    for segment in folder.split('/').filter(|segment| !segment.is_empty()) {
        so_far.push_str(segment);
        so_far.push('/');
        out.push(so_far.clone());
    }
    out
}

/// The folder containing `folder`, or `None` for the root.
pub(super) fn parent_of(folder: &str) -> Option<String> {
    if folder.is_empty() {
        return None;
    }
    let trimmed = folder.trim_end_matches('/');
    Some(match trimmed.rfind('/') {
        Some(cut) => trimmed[..=cut].to_owned(),
        None => String::new(),
    })
}

/// A folder's last segment: `b` for `a/b/`, empty for the root.
pub(super) fn folder_name(folder: &str) -> &str {
    let trimmed = folder.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(cut) => &trimmed[cut + 1..],
        None => trimmed,
    }
}

/// The half-open range of paths that lie under a folder prefix.
///
/// A range rather than `LIKE 'prefix%'` for two reasons: `files.path` is `UNIQUE`, so SQLite has an
/// index it can seek into with a range and cannot with a `LIKE` (whose default case-insensitivity
/// rules it out anyway); and a real corpus has folders with `%` and `_` in their names, which `LIKE`
/// would read as wildcards.
///
/// The upper bound is the prefix with its last character bumped by one, which is the next string
/// that sorts after everything beginning with it.
pub fn prefix_range(prefix: &str) -> (String, String) {
    let low = prefix.to_owned();
    let mut high = low.clone();
    match high.pop().and_then(|last| char::from_u32(last as u32 + 1)) {
        Some(next) => high.push(next),
        // Only reachable for an empty prefix or one ending at the last code point; a bound past
        // every path is then the honest answer, and the caller filters nothing.
        None => high = format!("{low}\u{10FFFF}"),
    }
    (low, high)
}

/// The `SELECT` list every browse row is built from.
///
/// One definition, used by the list and by the single-row re-render an inline edit answers with. Two
/// copies of this would be two rows that disagree about a song the moment one of them is changed —
/// and the disagreement would show up as a row that looks different after being edited, which reads
/// as the edit having done something it did not.
pub(super) fn browse_columns() -> String {
    format!(
        "s.id,
         {} AS eff_title,
         {} AS eff_artist,
         (s.title IS NOT NULL OR s.artist IS NOT NULL) AS edited,
         s.duration_ms, s.suitability, s.user_score,
         -- How many favorites it is in, so the star in the row can be filled without a second
         -- query per row. A count rather than a flag: the row can say *in two favorites*, which is
         -- the thing a boolean could never say.
         (SELECT COUNT(*) FROM song_favorites sf WHERE sf.song_id = s.id) AS favorite_count,
         s.melody_channel,
         -- The column, not a correlated `COUNT(*)`. See the note on `Sort::Copies` for why; the two
         -- path subqueries below stay, because a path is not a count and nothing would be gained
         -- by denormalizing one.
         s.file_count,
         (SELECT f.path FROM files f WHERE f.song_id = s.id ORDER BY f.path LIMIT 1) AS path,
         -- Every copy's path, newline-separated, so the row can show the best-looking one without a
         -- second query per row. `group_concat` promises no ordering, which does not matter here:
         -- the choice among them is made by shape, not by order.
         (SELECT group_concat(f.path, char(10)) FROM files f WHERE f.song_id = s.id) AS paths,
         {} AS from_filename,
         s.kind,
         -- Appended, so `song_row` gains an index and none of the existing ones move.
         {} AS eff_language,
         -- The cluster this row belongs to, itself included, and 1 when it is in none. A column
         -- rather than a correlated `COUNT(*)` for `file_count`'s reason above.
         --
         -- **Read off the representative for a song set aside**, because `Db::cluster` writes the
         -- count there alone. A hidden version reached through *every version*, a favorite or a
         -- search would otherwise read 1, which is what a song in no cluster says. One primary-key
         -- lookup, and only for the rows that are hidden.
         CASE WHEN s.duplicate_of IS NULL THEN s.version_count
              ELSE coalesce((SELECT r.version_count FROM songs r WHERE r.id = s.duplicate_of), 1)
         END AS version_count,
         -- Of the favorites above, how many are not a working list. Appended, so `song_row` gains an
         -- index and none of the existing ones move.
         --
         -- A second subquery rather than a narrowing of the first, because the row needs both
         -- numbers and they answer different questions: the count says how many lists this song is
         -- in and fills the star, this one says whether any of them is a filing and colors it.
         (SELECT COUNT(*) FROM song_favorites sf JOIN favorites f ON f.id = sf.favorite_id
           WHERE sf.song_id = s.id AND f.temporary = 0) AS permanent_count,
         -- The version the song list shows instead of this one, when this one is set aside.
         -- Appended, so `song_row` gains an index and none of the existing ones move.
         s.duplicate_of,
         -- Whether the song has words, so a row can decide whether to offer the search for the
         -- songs that sing them. Appended, for the reason above.
         --
         -- Word for word the predicate `lyrics_fts_insert` indexes by, so a row that offers the
         -- search is a row that can be a candidate. The empty string is never stored, which makes
         -- the second test redundant today and keeps the two from parting if that ever changes.
         (s.lyrics IS NOT NULL AND s.lyrics <> '') AS has_words,
         -- What the analysis had to say against this song, for the browse row's warning chips.
         -- Appended, for the reason above.
         --
         -- **Selected on every page whether or not the box asking for them is ticked**, which the
         -- chips being a class on the block rather than a flag on the row already requires: a row
         -- redrawn on its own by the single-row route never sees the browse query, and a row that
         -- fetched its warnings only when asked would lose them the first time somebody rated it.
         -- The column is a short JSON array and empty on most of a corpus.
         s.warnings",
        eff_title("s."),
        eff_artist("s."),
        title_is_filename("s."),
        eff_language("s."),
    )
}

pub(super) fn song_row(row: &Row<'_>) -> rusqlite::Result<SongRow> {
    let kind = SongKind::from_str(&row.get::<_, String>(13)?);
    Ok(SongRow {
        // Filled by `fill_tags` over a whole page, or by `song_row` for one; a join here would
        // multiply rows and break the `limit + 1` the paging leans on.
        tags: Vec::new(),
        // Filled by `State::mark_hints` from what this run was asked to hint, which is not a fact
        // about the song and so has no column to select.
        hint: None,
        // Filled by `State::say_rows`, for the same reason one step further: what a row's tooltips
        // say depends on the language the page is being drawn in, and a database module has none.
        artist_title: String::new(),
        melody_title: String::new(),
        versions_title: String::new(),
        favorite_title: String::new(),
        path_said: String::new(),
        likeness: None,
        searched_from: false,
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        edited: row.get::<_, i64>(3)? != 0,
        duration_ms: row.get::<_, i64>(4)? as u32,
        // Read from the column for every kind, because the scan writes one for every kind. The
        // filter and the sort read that same column, so a number worked out here instead would show
        // one thing on the page and mean another in the `WHERE` clause.
        suitability: row.get::<_, Option<i64>>(5)?.map(|v| v as u8),
        user_score: row.get::<_, Option<i64>>(6)?.map(|v| v as u8),
        favorite_count: row.get::<_, i64>(7)? as u32,
        melody_channel: row.get::<_, Option<i64>>(8)?.map(|v| v as u8),
        file_count: row.get::<_, i64>(9)? as u32,
        path: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        paths: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
        from_filename: row.get::<_, i64>(12)? != 0,
        kind,
        language: row.get(14)?,
        version_count: row.get::<_, i64>(15)? as u32,
        permanent_count: row.get::<_, i64>(16)? as u32,
        duplicate_of: row.get(17)?,
        has_words: row.get::<_, i64>(18)? != 0,
        warnings: row.get(19)?,
    })
}

/// The columns [`package_row`] reads, in its order, from `packages p` joined to `package_volumes v`.
///
/// **One copy, because four queries read a package and every one reads it the same way.** A caller
/// adds the `WHERE` that picks the volume.
pub(super) const PACKAGE_COLUMNS: &str =
    "p.id, p.name, v.package_version, p.publisher, v.start_number,
        p.default_language, v.out_path, v.built_at,
        (SELECT COUNT(*) FROM package_songs ps
          WHERE ps.package_id = p.id AND ps.volume = v.volume),
        v.volume, v.id,
        (SELECT COUNT(*) FROM package_volumes x WHERE x.package_id = p.id),
        (SELECT COUNT(*) FROM package_songs ps WHERE ps.package_id = p.id),
        p.volume_format, p.number_one_volume
   FROM packages p JOIN package_volumes v ON v.package_id = p.id";

pub(super) fn package_row(row: &Row<'_>) -> rusqlite::Result<PackageRow> {
    Ok(PackageRow {
        id: row.get(0)?,
        name: row.get(1)?,
        version: row.get(2)?,
        publisher: row.get(3)?,
        start_number: row.get::<_, i64>(4)? as u32,
        default_language: row.get(5)?,
        out_path: row.get(6)?,
        built_at: row.get(7)?,
        song_count: row.get::<_, i64>(8)? as u32,
        volume: row.get::<_, i64>(9)? as u32,
        volume_id: row.get(10)?,
        volumes: row.get::<_, i64>(11)? as u32,
        total_songs: row.get::<_, i64>(12)? as u32,
        volume_format: row.get(13)?,
        number_one_volume: row.get(14)?,
    })
}
