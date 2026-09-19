-- The curation database: one per folder of source karaoke files.
--
-- This is not the machine's catalog. `km-catalog` indexes what has been *installed* and is keyed by
-- song number, because a number is how a singer asks for a song. Here nothing has a number yet: the
-- subject is files on a disk, most of them duplicates of each other, and the question being answered
-- is which of them deserve to become songs at all.
--
-- Every statement is `IF NOT EXISTS`, so running this file over a database already at this schema
-- changes nothing. Adding a column means adding it here *and* a numbered step in `db/migrate.rs` that
-- adds it to a database one version behind, because SQLite has no `ADD COLUMN IF NOT EXISTS`.

PRAGMA foreign_keys = ON;

-- One row per file found under the root.
--
-- Paths are stored relative to the root with forward slashes, so the whole corpus folder can be moved
-- or reached from another machine without invalidating the database.
CREATE TABLE IF NOT EXISTS files (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    size         INTEGER NOT NULL,
    -- Seconds since the epoch. Together with `size` this is what makes a re-scan cheap: an unchanged
    -- file is not read, let alone parsed.
    mtime        INTEGER NOT NULL,
    content_hash TEXT,
    -- NULL when the file could not be turned into a song at all.
    song_id      TEXT    REFERENCES songs(id) ON DELETE SET NULL,
    -- ok | unreadable | not_midi | panicked
    scan_status  TEXT    NOT NULL,
    scan_error   TEXT,
    scanned_at   TEXT    NOT NULL,
    -- Which revision of the analysis read this file. A re-scan asks it only of a file with no song:
    -- a failure or a claimed half has no `songs.analysis_revision` to ask, and without this one it
    -- would be read again on every scan.
    analysis_revision INTEGER
);

-- A failure somebody has looked at and accepted, so the scan page stops counting it.
--
-- Per file rather than per reason, because the two questions a reason answers are different: eight
-- orphan .cdg files somebody has been through are settled, and a ninth appearing is news. Dismissing
-- the reason itself would make the ninth silent, which is the one thing the list exists to prevent.
--
-- **The status is stored, and it is what makes a dismissal specific.** A file that starts failing a
-- different way is a different thing to know, so it counts again rather than staying quiet under a
-- verdict passed on something else. A rescan updates a file row in place -- `ON CONFLICT(path)` --
-- so an id survives it, and a file deleted from disk takes its dismissal with it through the cascade.
CREATE TABLE IF NOT EXISTS dismissed_failures (
    file_id      INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    scan_status  TEXT    NOT NULL,
    dismissed_at TEXT    NOT NULL
);

-- Restoring works by reason, over a table that can hold a whole corpus's worth of one failure.
CREATE INDEX IF NOT EXISTS dismissed_failures_status ON dismissed_failures(scan_status);

CREATE INDEX IF NOT EXISTS files_song   ON files(song_id);
CREATE INDEX IF NOT EXISTS files_hash   ON files(content_hash);
CREATE INDEX IF NOT EXISTS files_status ON files(scan_status);

-- The red badge in the status bar, which is drawn on every page of the tool: how many files failed
-- to read and nobody has accepted yet. `files_status` cannot serve it, because the question is
-- `scan_status <> 'ok'` and an inequality has no range in an index to seek -- so the count walked
-- every row of the corpus, twice per page render, for a number that is almost always zero.
--
-- Partial, so it holds only the rows the question is about: on a corpus that reads cleanly it is
-- empty, and the count becomes reading an empty index instead of the whole of `files`. The same
-- shape `songs_unfolded` below takes, and for the same reason.
CREATE INDEX IF NOT EXISTS files_failed ON files(id) WHERE scan_status <> 'ok';

-- `(song_id, path)` rather than the `files_song` above, for the three subqueries every browse row
-- runs: how many copies a song has, the nicest-looking path, and the newline-joined list of all of
-- them (`browse_columns`). Two of those want the paths *in order* -- `ORDER BY f.path LIMIT 1` --
-- and `files_song` carries no path, so each of the rows on a page sorted its own file list
-- and then went to the table for the path. With the path in the key all three are answered from the
-- index alone, in order, touching no table row at all.
--
-- `files_song` stays: it is a shorter key, and the several places that ask only *which song is this
-- file* have no use for the path. Two indexes over the same leading column is a real cost on a
-- corpus this size, and it is bought here rather than assumed -- the browse page runs these three
-- subqueries five hundred times a page.
CREATE INDEX IF NOT EXISTS files_song_path ON files(song_id, path);

-- One row per distinct recording.
--
-- `id` is the content hash of the file's bytes, which settles two requirements at once: the same file
-- always yields the same id, and byte-identical copies land on the same row without any grouping pass
-- to run or get wrong. `files` carries the one-to-many.
--
-- Detection and correction are kept in separate columns on purpose. `det_*` is what the file said and
-- is rewritten by every scan; the bare columns are what a person typed and are never touched by the
-- scanner. NULL in a bare column means "nobody has said", not "empty".
CREATE TABLE IF NOT EXISTS songs (
    id                  TEXT PRIMARY KEY,

    -- `midi` or `video`. A song is one or the other and nothing else, per the `Video as a song
    -- source` decision in docs/decisions/.
    --
    -- **This column is why so many others below are nullable.** A video has no flavor, no channels,
    -- no lyric encoding and no automatic suitability. Filling them with zeros would be cheaper and
    -- wrong: a video rated 0 sorts below a genuinely broken MIDI file, which is the same reasoning
    -- that makes an unset `user_score` NULL rather than 0.
    kind                TEXT    NOT NULL DEFAULT 'midi',

    det_title           TEXT,
    det_artist          TEXT,
    det_language        TEXT,

    -- The ISO 639-1 code `det_language` and `det_encoding` between them imply, or NULL when neither
    -- said anything this build recognizes. See `km_kmpkg::Language::detect`: the encoding is the
    -- stronger witness and wins, because a Shift-JIS lyric track is Japanese whatever its `@L`
    -- header claims.
    --
    -- A second detected column rather than a rewrite of `det_language`, because that one has to stay
    -- answerable to "what did the file say?" -- it says `ENGL`, and the edit form shows it. Stored
    -- rather than derived because the mapping lives in a Rust table, and a filter and a sort over the
    -- effective language have to be expressible in SQL.
    det_language_tag    TEXT,

    -- The ISO 639-1 code the song's own words read as, or NULL where nothing could place them
    -- confidently. See `km_langguess::guess`: the lyrics are read first and the title is the
    -- fallback, and an answer below `km_langguess::MIN_CONFIDENCE` is not written at all.
    --
    -- A third detected column rather than a rewrite of either above it, for the reason
    -- `det_language_tag` gives about `det_language`: each has to stay answerable to what produced
    -- it, because the edit form shows all three and a curator correcting one is choosing between
    -- witnesses. It is the weakest of them and `eff_language` reads it last.
    det_language_guess  TEXT,

    -- How sure the reading above was, between zero and one, and NULL exactly when it is. Stored to
    -- be shown rather than to be filtered on: a curator scanning a page of guessed languages wants
    -- to know which were close calls, and the gate that decided whether to store one at all lives in
    -- Rust where the detector is.
    det_language_guess_confidence REAL,

    -- The file's own name with its extension removed. Most of a real corpus carries no title meta
    -- event at all, and a row with nothing in it is unreadable and unclickable, so this is the
    -- last-resort title -- the same fallback `km-pack` has always applied when building a package.
    --
    -- Deliberately not `det_title`: nothing inside the file said this, and `det_title` has to stay
    -- answerable to "what did the file say?" for the edit form to be honest about it. Written by every
    -- scan.
    stem                TEXT,

    title               TEXT,
    artist              TEXT,
    language            TEXT,
    lyric_encoding      TEXT,
    default_transpose   INTEGER,
    -- Whether to play the song and draw none of its words, for a file whose lyric track is
    -- mistimed, is the arranger's business card, or is a chord chart.
    --
    -- Three states, the shape `fixes` and `melody_chosen` carry: NULL is nobody has said and a
    -- build takes whatever the analysis concludes, 1 is somebody saying to silence the words, and
    -- **0 is somebody saying to draw them** on a file the analysis would have silenced. The last is
    -- why this is not a `NOT NULL DEFAULT 0` boolean: `Db::hand_set_predicate` reads *set at all*
    -- as `IS NOT NULL`, so a stored 0 has to mean an answer rather than a column nobody touched.
    lyrics_hidden       INTEGER,
    -- The corrections a person decided on, as km-fixes' JSON array. NULL means nobody has said, and
    -- a build then takes whatever detection proposes -- the same three states `default_transpose`
    -- above has. An empty array is a decision rather than an absence: it is how a detected fix that
    -- would otherwise apply itself is turned off.
    fixes               TEXT,

    -- Everything from here to `warnings` is a MIDI fact, and is NULL for a video song. See `kind`
    -- above for why NULL rather than a zero.
    flavor             TEXT,
    granularity         TEXT,
    -- The exception: both kinds of song have a length, so this stays NOT NULL. A video's comes from
    -- the probe rather than a tempo map.
    duration_ms         INTEGER NOT NULL,
    note_count          INTEGER,
    channel_count       INTEGER,
    line_count          INTEGER,
    syllable_count      INTEGER,
    det_encoding        TEXT,
    -- declared | utf8 | detected | fallback. `fallback` is the CP1252 guess -- around a third of the
    -- corpus -- and is its own filter in the UI, because that is the text worth eyeballing.
    det_encoding_source TEXT,

    -- NULL when detection abstained; `melody_abstained` then says why. Also NULL for a video, which
    -- has no channels to abstain about.
    melody_channel      INTEGER,
    melody_confidence   REAL,
    melody_abstained    TEXT,

    -- What a person said the melody channel is, where they disagreed with the three columns above.
    -- Three states, the same shape `fixes` and `default_transpose` carry: NULL is nobody has said
    -- and the build takes detection's answer, `none` is somebody saying this song has no melody
    -- channel, and `0`..`15` names one. Text rather than an integer because `none` is a decision
    -- and needs a spelling that is not a channel.
    --
    -- `melody_channel` is left alone by a save, so a rescan goes on recording what detection found
    -- and an answer survives it.
    melody_chosen       TEXT,

    suitability             INTEGER,
    suitability_lyrics      INTEGER,
    suitability_sync        INTEGER,
    suitability_channels    INTEGER,
    suitability_arrangement INTEGER,
    warnings            TEXT    NOT NULL DEFAULT '[]',

    -- Which revision of the analysis decided every detected column on this row, from
    -- `km_suitability::ANALYSIS_REVISION`. A scan re-reads the songs that disagree with the build
    -- doing the scanning and skips the rest, which is what makes a changed heuristic cost minutes
    -- rather than a reading of the whole corpus.
    --
    -- NULL means nobody knows: written before this column existed, or by a build that predates it.
    -- Not a number standing for "old", because a row cannot claim what produced it.
    analysis_revision   INTEGER,

    -- What a video song has instead, all from `km_video::probe` and all NULL for a MIDI file.
    -- Reported rather than judged: nothing here is scored, because a picture's dimensions say
    -- nothing about whether the song is worth singing.
    width               INTEGER,
    height              INTEGER,
    -- Frames per second times 1000, the same units `km_video::VideoInfo` uses, so the ubiquitous
    -- 29.97 survives being written down.
    frame_rate_milli    INTEGER,
    video_codec         TEXT,
    audio_codec         TEXT,

    -- What an MP3+G pair has instead, all from `km_cdg::probe` and all NULL for the other two kinds.
    -- `cdg_graphics_path` is the file that actually got paired, which is worth storing even though
    -- it is derivable: pairing tolerates mixed-case extensions and a stem with a trailing space, so
    -- "which .cdg is this song using" is a real question with a surprising answer.
    cdg_graphics_path   TEXT,
    cdg_sample_rate     INTEGER,
    cdg_channels        INTEGER,
    cdg_packets         INTEGER,
    cdg_graphics_ms     INTEGER,
    -- How far the graphics stop short of the audio. Seconds is ordinary; a minute is what a `.cdg`
    -- paired with the wrong song looks like.
    cdg_short_by_ms     INTEGER,
    -- Zero means the pair has no words in it, which is the one thing that disqualifies a file.
    cdg_tiles           INTEGER,
    -- Diagnostic only. 223 files of 2,849 carry some and every one of them renders.
    cdg_unknown         INTEGER,

    -- NULL is UNSET, which is different from 0. A song nobody has rated and a song rated unusable
    -- must not sort together.
    --
    -- One hand-set rating and not two. A second one asking *how good is this to sing* sat beside it
    -- for a judgment a curator makes with this one, and two 0-10 boxes a column apart, both a
    -- person's taste, both unset on almost every row, cost a column of the browse table to tell
    -- apart. See `User score` in `docs/decisions/songs.md`.
    user_score          INTEGER,
    -- There is no `favorite` flag here. Favoriting is membership of a named favorite (see
    -- `favorites` below), never a bare boolean: a song that was "a favorite" of nothing said less
    -- than the tree already says, and the star in the song list had no answer to *which one*.
    -- `migrate` moves any such song into a top-level `Favorites` and drops the column.
    notes               TEXT,

    -- Every lyric line the file carries, joined with newlines -- `LyricTimeline::plain_text`, which
    -- has carried the words "for search indexing" in its doc comment since it was written.
    --
    -- This is the *only* lyric text in the database. Everything else about lyrics here is a count or
    -- a label, and the song page gets its words by re-parsing the file on every view, which is right
    -- for one song and hopeless as an answer to "which song goes like this?".
    --
    -- NULL means "no scan has written it", which on a corpus indexed by an earlier version is every
    -- row: an incremental scan skips unchanged files, so a `--force` scan is what fills it. The empty
    -- string is never stored -- an instrumental is NULL too, and `lyrics_fts` below indexes neither.
    lyrics              TEXT,

    -- Set when a person confirms this song is the same recording as another. One level only: the
    -- target must itself have `merged_into IS NULL`.
    merged_into         TEXT    REFERENCES songs(id) ON DELETE SET NULL,
    -- A cheap structural signature used to *suggest* near-duplicates. Never acted on by itself.
    fingerprint         TEXT,

    -- The representative of the cluster of files this one is a version of, or NULL when this song
    -- *is* the representative or belongs to no cluster.
    --
    -- **Deliberately not `merged_into`, and the two must never be conflated.** That column means
    -- *a person decided these are one recording*; this one is the machine's guess from a structural
    -- fingerprint and a matching name. Both hide a song from browsing, and only one of them is
    -- somebody's word -- so a guess filed under the other would take a song away with nothing to
    -- say who did it. `Db::cluster` writes this and no person ever does.
    duplicate_of        TEXT    REFERENCES songs(id) ON DELETE SET NULL,

    -- Songs in this one's cluster, itself included. 1 for a song in no cluster, and meaningful only
    -- on a representative -- a song with `duplicate_of` set keeps its own 1, because it is not the
    -- row anybody counts from. The browse query reads a hidden row's count off its representative.
    --
    -- Stored rather than counted, for the reason `file_count` below gives at length: a browse row
    -- that asks `COUNT(*)` of another table is the shape that column exists to avoid, and this one
    -- is drawn on every row of every page.
    --
    -- Maintained by `Db::cluster` alone. Unlike `file_count` it gets no triggers: nothing but a
    -- clustering pass can change what it counts, where `file_count` moves whenever a file appears.
    version_count       INTEGER NOT NULL DEFAULT 1,

    -- How many rows in `files` point at this song. Derived, and the only derived value in this table.
    --
    -- It is here because the browse page both *sorts* and *filters* on it, and neither could be
    -- indexed while it was the correlated `COUNT(*)` over `files` it used to be: an index cannot carry
    -- a value computed from another table. `Sort::Copies` said so in writing and accepted being slow;
    -- adding `CopiesFilter` beside it made that a full pass on every page and on every count, which is
    -- where the trade turned.
    --
    -- **Maintained by the three triggers below, never by application code.** That is the same rule
    -- `songs_fts` follows and for the same reason: a count kept by whoever remembers to keep it is a
    -- count that goes wrong the first time somebody adds a write path.
    file_count          INTEGER NOT NULL DEFAULT 0,

    -- The browse list's sort keys: `km_song::text::fold` of the effective title and the effective
    -- artist. Stored rather than computed, because SQLite's default BINARY collation sorts every
    -- accented character after `Z` *and* every capital before every lower-case letter -- so
    -- `É o amor` sat past the end of a corpus-sized list while filing correctly under `A`, which is
    -- a page nobody browsing by name would ever reach the bottom of. The same shape, for the same
    -- reason, as `songs.sort_key` in `km-catalog` and in `km-remote-core`.
    --
    -- **`sort_artist` is nullable and its NULL means something.** `eff_artist` in `db.rs` has no
    -- `nullif`: an artist recorded as the empty string is not the same as no artist at all, and the
    -- browse list sorts the two differently -- empty first, absent last. So the fold is applied
    -- *through* the Option and never to a `''` standing in for absent.
    --
    -- **`sort_title IS NULL` is the "not folded yet" sentinel and `sort_artist IS NULL` is not.**
    -- `eff_title` ends in `coalesce(..., '')` and so is never NULL, which is what makes the sentinel
    -- unambiguous. `''` could not be the sentinel either way: `fold('---')` is `''`, and a real
    -- corpus contains files named that.
    --
    -- Maintained by `Db::refold`, which every path that writes a name calls; cleared by
    -- `songs_refold_update` below when a path does not; backfilled by `Db::backfill_sort_keys`.
    sort_title          TEXT,
    sort_artist         TEXT,

    first_seen          TEXT    NOT NULL,
    last_scanned        TEXT    NOT NULL,

    -- When somebody last changed this song.
    --
    -- **NULL is UNSET and not a date**, the rule `user_score` above follows and for its reason: a
    -- song nobody has edited and a song edited at the epoch are different facts, and the browse
    -- sort keeps them apart by putting the NULLs last.
    --
    -- **Maintained by `songs_stamp_update` below, never by application code.** That is the rule
    -- `file_count` states, and this column needs it harder: nine separate statements write a
    -- hand-set column of this table, and a stamp kept by whoever remembers to keep it is a stamp
    -- that goes wrong the first time somebody adds a write path.
    --
    -- The `YYYY-MM-DDTHH:MM:SSZ` shape `first_seen`, `last_scanned`, `packages.created_at` and
    -- `files.scanned_at` all carry: fixed width to the second, so lexicographic order is
    -- chronological order and one plain index serves the sort.
    updated_at          TEXT,

    -- When somebody threw this song away, and NULL when nobody has. A deleted song is in no list
    -- but the one that asks for deleted songs, and the scan does not read its files again.
    --
    -- **A third way of hiding a song, and it must not be confused with the two above it.**
    -- `merged_into` is somebody saying two files are one recording and `duplicate_of` is the
    -- machine's guess at the same thing; both are statements about which *copy* to show. This one
    -- says the song is not worth keeping, which is a judgment about the song rather than about the
    -- group it is in, so a deleted song stays deleted through a clustering pass that rewrites every
    -- group from nothing.
    --
    -- **The file rows stay.** The scan skips a deleted song's files rather than forgetting them, so
    -- undeleting needs no rescan to find the file again; `known_files` reads this column through
    -- the join it already makes.
    --
    -- **Hand curation, so `HAND_SET_COLUMNS` carries it and a backup restores it**, and
    -- `songs_stamp_update` watches it for the same reason it watches `merged_into`: both are a
    -- person deciding a song does not belong in a list, and a scan can work out neither. A whole
    -- corpus of judgment about what to keep is the thing a backup exists to hold.
    --
    -- The *recently edited* order does not fill with discarded songs, because the browse list
    -- excludes them outright -- so the stamp costs nothing on screen and is what puts a song back
    -- near the top of that order when somebody brings it back.
    --
    -- Last in the table because `ALTER TABLE ... ADD COLUMN` appends, and a database altered into
    -- this shape should hold its columns in the order a freshly created one does.
    deleted_at          TEXT
);

CREATE INDEX IF NOT EXISTS songs_kind       ON songs(kind);
CREATE INDEX IF NOT EXISTS songs_suitability ON songs(suitability);
CREATE INDEX IF NOT EXISTS songs_user_score ON songs(user_score);
-- (no `songs_favorite`: the flag it indexed is gone -- see the note in the table above.)
CREATE INDEX IF NOT EXISTS songs_duration   ON songs(duration_ms);
CREATE INDEX IF NOT EXISTS songs_melody     ON songs(melody_channel);
CREATE INDEX IF NOT EXISTS songs_encsource  ON songs(det_encoding_source);
CREATE INDEX IF NOT EXISTS songs_merged     ON songs(merged_into);
CREATE INDEX IF NOT EXISTS songs_fingerprint ON songs(fingerprint);
CREATE INDEX IF NOT EXISTS songs_duplicate_of ON songs(duplicate_of);
-- The plain index for `CopiesFilter`. The *sort* wants a wider key and gets one in
-- `create_browse_indexes` (`songs_browse_copies`), where the partial `merged_into IS NULL` predicate
-- every browse query implies can be put on it; this one serves a filter combined with any other sort.
CREATE INDEX IF NOT EXISTS songs_file_count ON songs(file_count);
-- For the count the Scan page shows: how many songs this build would answer differently. It is one
-- `COUNT` per page load over a table of hundreds of thousands, and the column has as many distinct
-- values as there have been revisions -- so the index is what keeps it off the table itself.
CREATE INDEX IF NOT EXISTS songs_analysis_revision ON songs(analysis_revision);

-- `songs.file_count`, kept in step with `files`.
--
-- Dropped and recreated on every open, exactly like the FTS triggers below and for the identical
-- reason: a trigger is code, and a database created by an earlier version must not keep an older one
-- forever.
--
-- Three of them rather than one, because SQLite has no combined trigger and because the `UPDATE OF
-- song_id` case is the one that is easy to forget -- a re-scan that repoints a file at a different
-- song has to move the count as well as the row. `song_id` is nullable (a file that could not be read
-- is a row with no song), and an `UPDATE ... WHERE id IS NULL` matches nothing, so the NULL cases need
-- no guard of their own.
DROP TRIGGER IF EXISTS files_count_insert;
DROP TRIGGER IF EXISTS files_count_delete;
DROP TRIGGER IF EXISTS files_count_update;

CREATE TRIGGER files_count_insert AFTER INSERT ON files BEGIN
    UPDATE songs SET file_count = file_count + 1 WHERE id = new.song_id;
END;

CREATE TRIGGER files_count_delete AFTER DELETE ON files BEGIN
    UPDATE songs SET file_count = file_count - 1 WHERE id = old.song_id;
END;

CREATE TRIGGER files_count_update AFTER UPDATE OF song_id ON files
WHEN old.song_id IS NOT new.song_id BEGIN
    UPDATE songs SET file_count = file_count - 1 WHERE id = old.song_id;
    UPDATE songs SET file_count = file_count + 1 WHERE id = new.song_id;
END;
-- The browse page's granularity filter had no index at all, while every other control on that bar
-- had one. Nothing about the column made it the exception -- it was simply missed.
CREATE INDEX IF NOT EXISTS songs_granularity ON songs(granularity);

-- The fold in `Db::backfill_sort_keys` asks "is any song still unfolded?" on every open, and this is
-- what stops that question being a full scan of `songs` -- ten seconds on a large corpus, to find
-- nothing, every single time. Partial, so it holds only the rows the fold is looking for: on a
-- database it has already finished the index is empty and the probe touches one page. Covering on
-- `id`, which is what the probe selects.
--
-- **Guarded by the data rather than by a flag in `settings`**, because a flag is a claim about the
-- data kept outside it, and nothing says when it stops being true. Here it would stop being true the
-- first time somebody added a write path that did not refold, and the songs it was meant to fix would
-- sort to the top of the list for ever with nothing explaining why. This index cannot be wrong; it is
-- the predicate.
--
-- **Keyed on `sort_title` and never on `sort_artist`.** `sort_artist IS NULL` is a real value, held
-- permanently by every song nobody has named an artist for; `sort_title IS NULL` cannot be, because
-- `eff_title` always produces a string.
CREATE INDEX IF NOT EXISTS songs_unfolded ON songs(id) WHERE sort_title IS NULL;

-- How many songs a filter matched, which is the label on the browse page and the target of its
-- five-pages-on button.
--
-- **Two terms on two separately indexed columns defeat both of them.** Every filter carries
-- `merged_into IS NULL AND duplicate_of IS NULL` -- `Filter::to_sql` puts them there so a count
-- narrows with the page it counts -- and `songs_merged` and `songs_duplicate_of` can each serve one
-- half. SQLite cannot use two indexes for one scan, so it took neither and read every row of the
-- widest table here: measured on a whole corpus with the cache cold, thirteen seconds, against one
-- millisecond for the rows beside it, which have a `LIMIT` and a browse index.
--
-- **Composite on the two columns, and emphatically not partial on `id`.** The obvious shape is the
-- one `songs_unfolded` takes -- `ON songs(id) WHERE <the two terms>` -- and it
-- was built and measured and the planner refuses it. Forced with `INDEXED BY` it reports
-- `SCAN s USING INDEX`, without `COVERING`: the predicate guarantees the terms, but the generated
-- code still reads them from the row, so the index is a scan *plus* a lookup per row and honestly
-- worse than the table scan it was meant to replace. Holding the columns themselves is what makes
-- the count answerable without touching a row, which is the same reason `songs_merged` serves the
-- single-term count as a covering search.
--
-- Selectivity is irrelevant here and would be a reason to reject this index anywhere else: both
-- columns are NULL on nearly every song, so a seek returns nearly everything. What makes it cheap is
-- that two columns are a fraction of the width of the widest table in this schema.
--
-- Named in `HEAVY_INDEXES`, because an index with no `sqlite_stat1` row is one the planner will not
-- choose, and the figures above are what it is for.
-- **`deleted_at` is the third column for the reason the two above it are here at all.** Every
-- filter carries all three terms, a separate index can serve only one of them, and SQLite uses one
-- index per scan -- so leaving this one out returns the count to the table scan the other two were
-- measured against. The migration drops this index by name so a database that already holds the
-- two-column shape gets the three-column one; `IF NOT EXISTS` alone would keep the narrow one for
-- ever.
CREATE INDEX IF NOT EXISTS songs_countable ON songs(merged_into, duplicate_of, deleted_at);

-- The one list that asks for deleted songs, and nothing else reads this column as a search term.
--
-- Partial, so it holds only the rows the question is about: on a corpus nobody has deleted from it
-- is empty, and the page becomes reading an empty index rather than a pass over every song. The
-- shape `files_failed` and `songs_unfolded` take, and for their reason. Covering on `id`, which is
-- what the browse query seeks by.
CREATE INDEX IF NOT EXISTS songs_deleted ON songs(id) WHERE deleted_at IS NOT NULL;

-- The three indexes the browse page's order and its A-Z filter need are **not** here: their key is an
-- expression, SQLite only uses an expression index when the query's expression matches it tree for
-- tree, and the query side is generated by `eff_title` and `title_initial` in `db.rs`. Writing them
-- out again here would be a second copy free to drift -- and drift would not fail, it would silently
-- stop the planner using them. See `Db::create_browse_indexes`.

-- Full-text search over the *effective* title and artist -- the hand-typed value when there is one,
-- the detected value next, the file's own name last -- so searching finds a song by whichever name
-- it is known under. The file name has to be in here: on a corpus where most files have no metadata
-- it is the only name the song has, and a search box that cannot find it is a search box that cannot
-- find most of the corpus.
--
-- `remove_diacritics 2` is copied from `km-catalog` for the same reason it is there: much of this
-- corpus is Portuguese, and typing "coracao" must find "coração".
CREATE VIRTUAL TABLE IF NOT EXISTS songs_fts USING fts5(
    title,
    artist,
    tokenize='unicode61 remove_diacritics 2'
);

-- An external-content table cannot be used here: its content column would have to be an expression
-- (`coalesce(title, det_title)`), which FTS5 does not support. So the index is a plain one kept in
-- step by triggers -- still never by application code, which is the property that matters.
--
-- Because it is a plain table and not external-content, a row is removed with an ordinary `DELETE`.
-- The `INSERT INTO songs_fts(songs_fts, ...) VALUES('delete', ...)` command that `km-catalog` uses is
-- only valid on external-content and contentless tables; against this one it fails at run time with
-- nothing but "SQL logic error", which is how this was found.
--
-- Dropped and recreated on every open rather than `IF NOT EXISTS`. A trigger is code, and a database
-- created by an earlier version would otherwise keep the old, broken one forever.
DROP TRIGGER IF EXISTS songs_fts_insert;
DROP TRIGGER IF EXISTS songs_fts_delete;
DROP TRIGGER IF EXISTS songs_fts_update;

CREATE TRIGGER songs_fts_insert AFTER INSERT ON songs BEGIN
    INSERT INTO songs_fts(rowid, title, artist)
        VALUES (new.rowid, coalesce(nullif(new.title, ''), nullif(new.det_title, ''), new.stem, ''),
                           coalesce(new.artist, new.det_artist, ''));
END;

CREATE TRIGGER songs_fts_delete AFTER DELETE ON songs BEGIN
    DELETE FROM songs_fts WHERE rowid = old.rowid;
END;

-- **`UPDATE OF`, not a bare `AFTER UPDATE`.** These five columns are every input the statement below
-- reads, so no other write can change what it would index -- and a great many writes touch this
-- table without touching a name. Setting one language over a filter rewrites every matching row; the
-- rating setter fires on a click; a merge writes `merged_into` and nothing else. A bare
-- `AFTER UPDATE` would retokenise the title and artist of every row each of those touched, for a
-- result identical to the one already stored.
--
-- **`WHEN` as well, because `UPDATE OF` fires on assignment and not on change.** A scan assigns the
-- detected columns on every row it reads, and re-analysis reads the whole corpus to move suitability,
-- so without this every song is deleted from the index and reinserted to arrive at the title already
-- in it. The overwhelming majority of a scan's writes name a song exactly what it was already
-- called, so that is a whole corpus retokenised to reach the entry already stored. The five columns
-- are every input the statement below reads, so inputs that did not change cannot produce a
-- different entry.
CREATE TRIGGER songs_fts_update
AFTER UPDATE OF title, det_title, stem, artist, det_artist ON songs
WHEN new.title IS NOT old.title
  OR new.det_title IS NOT old.det_title
  OR new.stem IS NOT old.stem
  OR new.artist IS NOT old.artist
  OR new.det_artist IS NOT old.det_artist
BEGIN
    DELETE FROM songs_fts WHERE rowid = old.rowid;
    INSERT INTO songs_fts(rowid, title, artist)
        VALUES (new.rowid, coalesce(nullif(new.title, ''), nullif(new.det_title, ''), new.stem, ''),
                           coalesce(new.artist, new.det_artist, ''));
END;

-- Full-text search over the words themselves, for "which song goes like this?".
--
-- A table of its own rather than a third column on `songs_fts`, and that is the load-bearing choice
-- here. An FTS5 `MATCH` with no column filter searches *every* column, so putting lyrics beside
-- title and artist would silently turn the browse page's "title or artist" box into a lyric search:
-- type `love` and half the corpus comes back because half the corpus sings the word. The two indexes
-- answer two different questions and a person asks them on two different pages, so they are two
-- tables. It also sidesteps a migration -- `songs_fts` is `IF NOT EXISTS`, so an existing database
-- would keep its two-column shape and would have had to be dropped and rebuilt.
--
-- `remove_diacritics 2` for the same reason as `songs_fts`: much of this corpus is Portuguese, and
-- somebody half-remembering a line will type it without the accents.
CREATE VIRTUAL TABLE IF NOT EXISTS lyrics_fts USING fts5(
    text,
    tokenize='unicode61 remove_diacritics 2'
);

-- How many songs sing a given word, read straight off `lyrics_fts`'s own term list.
--
-- No storage and no trigger: `fts5vocab` is a view over the index that is already there, and asking
-- it about one term is a seek into a b-tree the index maintains anyway.
--
-- It has one caller, the same-words page, which chooses the phrases it asks the index for by how
-- rare their rarest word is. `ORDER BY bm25` ranks every row a query matches before any `LIMIT` can
-- cut one, so a page that asked for a dozen phrases of whatever words a song happened to open with
-- would have SQLite score a large part of the corpus to return a hundred rows.
--
-- `IF NOT EXISTS` like the two tables above and unlike the triggers below: a view over an index is
-- not code that can go stale, so a database made by an earlier version keeps a correct one.
CREATE VIRTUAL TABLE IF NOT EXISTS lyrics_vocab USING fts5vocab(lyrics_fts, 'row');

-- Like `songs_fts`, a plain index kept in step by triggers rather than an external-content one, so a
-- row is removed with an ordinary `DELETE`. Dropped and recreated on every open for the same reason:
-- a trigger is code, and a database created by an earlier version must not keep an older one.
--
-- The `INSERT ... SELECT ... WHERE` rather than `VALUES` is what keeps the empties out. Most of this
-- corpus is instrumental, and indexing several hundred thousand empty rows would cost real space to
-- record that they have nothing to say. The `DELETE` is unconditional and the `INSERT` is not, which
-- is also what makes a song whose lyrics are cleared leave the index rather than linger in it.
DROP TRIGGER IF EXISTS lyrics_fts_insert;
DROP TRIGGER IF EXISTS lyrics_fts_delete;
DROP TRIGGER IF EXISTS lyrics_fts_update;

CREATE TRIGGER lyrics_fts_insert AFTER INSERT ON songs BEGIN
    INSERT INTO lyrics_fts(rowid, text)
        SELECT new.rowid, new.lyrics WHERE new.lyrics IS NOT NULL AND new.lyrics <> '';
END;

CREATE TRIGGER lyrics_fts_delete AFTER DELETE ON songs BEGIN
    DELETE FROM lyrics_fts WHERE rowid = old.rowid;
END;

-- `UPDATE OF lyrics`, for the reason `songs_fts_update` above gives at more length: `lyrics` is the
-- only column this statement reads, and a bare `AFTER UPDATE` had every language change and every
-- score click deleting and reinserting a lyric row to arrive at the text already in it.
--
-- `WHEN` for the reason `songs_fts_update` gives: a scan assigns `lyrics` on every row it reads, and
-- a lyric is the longest text in the table, so retokenising one to arrive at the words already
-- indexed is the most expensive way this schema has of doing nothing.
CREATE TRIGGER lyrics_fts_update AFTER UPDATE OF lyrics ON songs
WHEN new.lyrics IS NOT old.lyrics
BEGIN
    DELETE FROM lyrics_fts WHERE rowid = old.rowid;
    INSERT INTO lyrics_fts(rowid, text)
        SELECT new.rowid, new.lyrics WHERE new.lyrics IS NOT NULL AND new.lyrics <> '';
END;

-- `songs.sort_title` / `songs.sort_artist`, invalidated whenever a name is written.
--
-- The fold itself is `km_song::text::fold` and has no SQL spelling -- a second accent table here is
-- exactly what that module's own header refuses, because two of them disagreeing looks like bad data
-- rather than like a bug. So this trigger does the half SQL *can* do: it clears the key, and
-- `Db::refold` -- which every write path calls, in the same transaction -- puts the right value back
-- a statement later.
--
-- **That makes forgetting loud instead of silent, which is the whole reason it exists.** A future
-- write path that does not refold leaves NULL, and NULL sorts to the top of the browse list where
-- somebody will see it; `Db::backfill_sort_keys` then repairs it on the next open. Maintaining the
-- keys in Rust alone would have made the same mistake invisible: a stale value that is merely wrong.
--
-- `UPDATE OF` and not a bare `AFTER UPDATE`, for the reason `songs_fts_update` gives: the language
-- setter, the rating setter and every merge write this table without touching a name, and
-- clearing a key for those is work for nothing across hundreds of thousands of rows.
--
-- `WHEN` for the rest of that reason. A scan assigns a name to every row it reads whether or not the
-- file says anything new, and a cleared key costs a seek on `songs_unfolded` and a write of both
-- keys to put back what was there.
--
-- No INSERT twin. A new row's columns start NULL by themselves.
--
-- Dropped and recreated on every open, like the FTS triggers and for the same reason: a trigger is
-- code, and a database created by an earlier version must not keep an older one.
DROP TRIGGER IF EXISTS songs_refold_update;

CREATE TRIGGER songs_refold_update
AFTER UPDATE OF title, det_title, stem, artist, det_artist ON songs
WHEN new.title IS NOT old.title
  OR new.det_title IS NOT old.det_title
  OR new.stem IS NOT old.stem
  OR new.artist IS NOT old.artist
  OR new.det_artist IS NOT old.det_artist
BEGIN
    UPDATE songs SET sort_title = NULL, sort_artist = NULL WHERE id = new.id;
END;

-- `songs.updated_at`, stamped whenever a curation decision about a song changes.
--
-- **`UPDATE OF`, and the list is exactly the hand-set columns.** The detected half of this table is
-- rewritten by every scan -- `write_scanned`'s `ON CONFLICT` sets `det_title`, `det_artist`, `stem`,
-- `lyrics` and a dozen counts -- and three repairs in `db.rs` rewrite them again on an open. A bare
-- `AFTER UPDATE`, or any list holding a `det_` column or `stem`, stamps a whole corpus as edited the
-- first time it is rescanned, which is the one thing this column must never say. `songs_refold_update`
-- above watches those columns for the opposite reason: a fold has to follow a rescan, and a stamp
-- must not.
--
-- `crate::backup::HAND_SET_COLUMNS` is this same list in Rust, and
-- `the_stamp_trigger_watches_every_hand_set_column` asserts the two against each other, because
-- neither can be derived from the other -- a trigger's column list is SQL text and a `const` is not.
-- `Db::hand_set_predicate` reads that constant too, so *edited* means one thing here and one thing in
-- the backup.
--
-- **A tag and a favorite are filed in tables of their own and leave this alone**, which is the one
-- place this list is narrower than the backup's. See `When a song was last edited` in
-- `docs/decisions/curation.md`.
--
-- **The `WHEN` is what makes the stamp mean *changed* rather than *written***, following
-- `files_count_update` above. `edit_song` treats every box on its form as authoritative, the
-- filter-wide language set writes every matching row whether or not it already holds that language,
-- and `unmerge` clears `merged_into` without asking whether it was set -- so without this, saving a
-- form unchanged is an edit, and setting every row to the language most of them already have
-- moves those entries of `songs_browse_updated_artist` for nothing.
--
-- `strftime` rather than a value bound from Rust, because a trigger body takes no parameter. Its
-- output is `crate::scan::timestamp()`'s character for character -- UTC, four-digit year, two digits
-- everywhere else, `Z` -- and `the_sql_stamp_is_the_one_this_crate_writes` pins that. `'now'` is
-- fixed for one step of a statement, so a bulk edit lands one value on every row it changes rather
-- than smearing them across the seconds the statement took.
--
-- **It cannot recurse, by shape rather than by the `recursive_triggers` default**: the column it
-- writes is in no trigger's `UPDATE OF` list, its own included. `songs_refold_update` and the three
-- `files_count_*` triggers are safe by the same construction.
--
-- No INSERT twin. A scanned song is not an edited song, so a new row's NULL is the answer -- which is
-- also why `write_scanned` does not name this column.
--
-- Dropped and recreated on every open, like every trigger here: a trigger is code, and a database
-- created by an earlier version must not keep an older one.
DROP TRIGGER IF EXISTS songs_stamp_update;

CREATE TRIGGER songs_stamp_update
AFTER UPDATE OF title, artist, language, lyric_encoding, default_transpose, lyrics_hidden, fixes,
                melody_chosen, user_score, notes, merged_into, deleted_at ON songs
WHEN old.title             IS NOT new.title
  OR old.artist            IS NOT new.artist
  OR old.language          IS NOT new.language
  OR old.lyric_encoding    IS NOT new.lyric_encoding
  OR old.default_transpose IS NOT new.default_transpose
  OR old.lyrics_hidden     IS NOT new.lyrics_hidden
  OR old.fixes             IS NOT new.fixes
  OR old.melody_chosen     IS NOT new.melody_chosen
  OR old.user_score        IS NOT new.user_score
  OR old.notes             IS NOT new.notes
  OR old.merged_into       IS NOT new.merged_into
  OR old.deleted_at        IS NOT new.deleted_at
BEGIN
    UPDATE songs SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = new.id;
END;

-- The folder tree, derived from `files` and kept as a table because computing it costs a full scan.
--
-- The Folders page asks "what is directly inside this folder, and how many songs are under each?".
-- Answered from `files` that is a `substr`/`instr` group-by over every row -- the grouping key is an
-- expression, so no index can help, and at the root of a large corpus it is the slowest page in
-- the tool by an order of magnitude. Here it is an index seek on `parent`.
--
-- Derived, never authoritative: `Db::folders` rebuilds it whenever `folders_index` below says it no
-- longer matches `files`, so a stale tree is a slow page once rather than a wrong page forever.
CREATE TABLE IF NOT EXISTS folders (
    -- Path from the root ending in `/`; the empty string is the root itself.
    path    TEXT PRIMARY KEY,
    -- The containing folder's path. NULL for the root row, which is what makes `parent = ?` a
    -- complete answer for every level below it.
    parent  TEXT,
    -- The last segment, empty for the root.
    name    TEXT    NOT NULL,
    -- Distinct songs whose files sit *in* this folder rather than under a subfolder. This is the
    -- "files here" bucket the page shows.
    direct  INTEGER NOT NULL,
    -- Distinct songs anywhere at or below it. A song with six copies in one subtree counts once,
    -- which is why this cannot be a sum of its children.
    beneath INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS folders_parent ON folders(parent);

-- Favorites: a flat set of named lists.
--
-- This *is* favoriting -- there is no flag on `songs` beside it. A favorite is a named list a song
-- is in, a song can be in as many as somebody likes, and a collection divides by naming more lists
-- rather than by putting one inside another -- see `A favorite does not nest` in
-- `docs/decisions/curation.md`. These tables were called `categories` and `song_categories` when
-- they sat next to a boolean; `migrate` renames them in place.
CREATE TABLE IF NOT EXISTS favorites (
    id        INTEGER PRIMARY KEY,
    name      TEXT    NOT NULL,
    -- A list that is scaffolding for a later pass rather than a filing: *look at these again*,
    -- *decide about these*. The songs in one have not been filed, so the browse row's gold star does
    -- not claim they have -- see `A favorite can be a working list` in `docs/decisions/curation.md`.
    -- On the list rather than on the membership below: it is the list that is a working one, and a
    -- song's being in it says the same thing about every song in it.
    temporary INTEGER NOT NULL DEFAULT 0
);

-- One name, one list. The name is what every control labels a favorite by and what a backup matches
-- on, so two lists sharing one would be two rows nothing downstream could tell apart.
CREATE UNIQUE INDEX IF NOT EXISTS favorites_name ON favorites(name);

CREATE TABLE IF NOT EXISTS song_favorites (
    song_id     TEXT    NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    favorite_id INTEGER NOT NULL REFERENCES favorites(id) ON DELETE CASCADE,
    PRIMARY KEY (song_id, favorite_id)
);

CREATE INDEX IF NOT EXISTS song_favorites_favorite ON song_favorites(favorite_id);

-- What a song is filed under: `rock`, `anime`, `brasil`. See `km_kmpkg::Tag`.
--
-- The same two tables as `favorites` / `song_favorites` above, and what separates them is what they
-- mean rather than what they hold. A favorite is somebody's own filing of this corpus and stays
-- here; a tag is a word about the song itself, and the machine's own catalog carries it. Both are
-- hand curation and neither is detected -- which is why both have to survive a backup.
--
-- **This is the truth here, where in `km-catalog` the packed column is.** The two databases want
-- opposite things: that one is read by searches and hashed by `package_digest`, this one is *edited*
-- one song at a time and asked "what tags exist" on every page render.
--
-- The `tags` table is the vocabulary, and it exists so that question is a read rather than the
-- recursive-CTE skip-scan `languages_present` needs over `songs`. A row appears the first time a tag
-- is used and goes when its last song loses it, so the list is never a museum of words nobody uses
-- any more.
--
-- Two new tables, so there is no `migrate` entry and no `ALTER TABLE`: this file runs in full on
-- every open.
CREATE TABLE IF NOT EXISTS tags (
    -- The slug, and `km_kmpkg::Tag::parse` is what put it here -- so `Forró` and `forro` cannot
    -- both be in this column.
    name     TEXT PRIMARY KEY,
    -- The folded name, for ordering a picker. Filled in Rust by `km_song::text::fold`, exactly as
    -- `favorites.sort_key` is and for that column's reason: `COLLATE NOCASE` is ASCII-only.
    sort_key TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS song_tags (
    song_id TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    tag     TEXT NOT NULL REFERENCES tags(name) ON DELETE CASCADE,
    PRIMARY KEY (song_id, tag)
);

CREATE INDEX IF NOT EXISTS song_tags_tag ON song_tags(tag);

-- Packages curated in this folder, and what went into them.
--
-- **A package is what a curator names, and a volume is what a build writes.** One package holds one
-- or more volumes, each a `.kmpkg` of its own with its own id, version and 999 numbers. Everything a
-- curator says about the whole set lives here; everything that differs between two files of it lives
-- in `package_volumes`. See `A package holds volumes` in `docs/decisions/curation.md`.
CREATE TABLE IF NOT EXISTS packages (
    id             TEXT PRIMARY KEY,
    name           TEXT    NOT NULL,
    publisher      TEXT,
    -- Files any song in this package that names no language of its own under this code, in the
    -- package and nowhere else -- the curation database is never written to by a build.
    --
    -- Defaults to `en` rather than to NULL, because most packages are one language and being asked
    -- to classify every song before the first build is where people give up. NULL is still a
    -- meaningful value and means "no default": the build then refuses a song with no language.
    default_language TEXT DEFAULT 'en',
    -- Raises a volume's patch number on every build that writes a file, so two .kmpkg files holding
    -- different songs do not both claim to be 1.0.0. The first build of a volume is exempt: it ships
    -- at the version it was given, and every build after that raises first, so the version column
    -- and the file on disk always agree.
    --
    -- Defaults to on, because a version nobody remembers to raise is a label that says nothing, and
    -- the cost of it being wrong is one number in a field anybody can edit.
    raise_version  INTEGER NOT NULL DEFAULT 1,
    -- How a volume's number is written after the package's name once there are two volumes: `{n}` is
    -- the number, so `vol{n}` names them `Brasil vol1`, `Brasil vol2`. Always holds `{n}`, because
    -- without it every volume would take one name and one file.
    volume_format  TEXT    NOT NULL DEFAULT 'vol{n}',
    -- Numbers the volume while the package has only one, so a set its curator knows will outgrow 999
    -- songs is `Brasil vol1` from its first build and keeps that file name when a second volume
    -- starts. Off by default, because most packages never grow a second volume.
    number_one_volume INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT    NOT NULL
);

-- One `.kmpkg` of a package. Volume 1's id is the package's own id, so a package that never grows
-- past one volume is banked, installed and replaced exactly as its id always said.
CREATE TABLE IF NOT EXISTS package_volumes (
    package_id      TEXT    NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    volume          INTEGER NOT NULL,
    -- What the machine keys an install on and hashes into a bank. Generated, and never reused.
    id              TEXT    NOT NULL UNIQUE,
    package_version TEXT    NOT NULL DEFAULT '1.0.0',
    start_number    INTEGER NOT NULL DEFAULT 1,
    -- Where this volume's .kmpkg was last written, relative to the root when it is inside it.
    out_path        TEXT,
    built_at        TEXT,
    created_at      TEXT    NOT NULL,
    PRIMARY KEY (package_id, volume)
);

-- **`UNIQUE (package_id, song_id)` is what keeps a song in one volume.** A song belongs to a package
-- once, whichever volume numbered it, so a sync that moved it to a second volume would be refused by
-- the table rather than by a check somebody has to remember.
CREATE TABLE IF NOT EXISTS package_songs (
    package_id TEXT    NOT NULL,
    volume     INTEGER NOT NULL DEFAULT 1,
    number     INTEGER NOT NULL,
    song_id    TEXT    NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    -- Which file on disk this entry was sourced from. A song can have several identical files and it
    -- must be recorded which one a package actually read, so a rebuild is reproducible.
    file_id    INTEGER REFERENCES files(id) ON DELETE SET NULL,
    added_at   TEXT    NOT NULL,
    PRIMARY KEY (package_id, volume, number),
    UNIQUE (package_id, song_id),
    FOREIGN KEY (package_id, volume)
        REFERENCES package_volumes(package_id, volume) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS package_songs_song ON package_songs(song_id);

-- `file_id` is a foreign key with `ON DELETE SET NULL`, and `foreign_keys` is ON. Without an index
-- on it, **every row deleted from `files` scans this whole table** to find the rows it has to blank.
-- Nothing else reads this column, so the index exists purely to keep that cascade a seek. The scan's
-- `forget_missing` is the delete that would pay for it: a drive reorganized under the corpus removes
-- files in the tens of thousands, and unindexed that is quadratic.
CREATE INDEX IF NOT EXISTS package_songs_file ON package_songs(file_id);

-- The favorites a package draws its songs from, and a row here is the whole of what makes a package
-- *sourced*: its songs are the union of these lists, a sync is what reconciles the two, and it is
-- offered nowhere a song is added to a package one at a time. Clearing the last row makes it an
-- ordinary package again. See `A package can be the songs in some favorites` in
-- `docs/decisions/curation.md`.
--
-- A new table, so there is no `migrate` entry and no `ALTER TABLE`: this file runs in full on every
-- open and every statement is `IF NOT EXISTS`, which is the `tags` / `song_tags` arrangement above.
CREATE TABLE IF NOT EXISTS package_favorites (
    package_id  TEXT    NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    favorite_id INTEGER NOT NULL REFERENCES favorites(id) ON DELETE CASCADE,
    PRIMARY KEY (package_id, favorite_id)
);

-- `favorite_id` is a foreign key with `ON DELETE CASCADE` and `foreign_keys` is ON, so without this
-- every favorite deleted scans the table for the rows to take with it. The argument
-- `package_songs_file` makes above, over a smaller table.
CREATE INDEX IF NOT EXISTS package_favorites_favorite ON package_favorites(favorite_id);

-- Suggested near-duplicates, awaiting a person's judgment. Nothing here ever merges on its own.
-- `a_id` is always the lexicographically smaller id, so a pair is stored once.
CREATE TABLE IF NOT EXISTS duplicate_candidates (
    a_id       TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    b_id       TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    similarity REAL NOT NULL,
    reason     TEXT NOT NULL,
    -- NULL is unreviewed; 'same' merged; 'different' dismissed and never suggested again.
    verdict    TEXT,
    PRIMARY KEY (a_id, b_id)
);

CREATE INDEX IF NOT EXISTS duplicate_candidates_verdict ON duplicate_candidates(verdict);

-- The same cascade hazard as `package_songs_file` above, and easier to miss because the table looks
-- indexed: the primary key `(a_id, b_id)` covers `a_id` and cannot serve `b_id`. Both columns are
-- `ON DELETE CASCADE` onto `songs`, so a song deleted by `forget_missing` scanned this whole table
-- once for the half the key does not reach. `unmerge` reads `b_id` too, so this is not only for the
-- cascade.
CREATE INDEX IF NOT EXISTS duplicate_candidates_b ON duplicate_candidates(b_id);

-- A filter somebody named, so a corpus can be worked from several positions rather than one.
--
-- **Here and not beside the recent-folder list, which holds the cursor.** The two are different
-- facts. Where somebody happens to be is a fact about a run; `Portuguese, unclassified` is a
-- judgment about how this corpus divides, made once and worth months — the kind of thing
-- `favorites` and `tags` above already are, kept where they are kept and travelling to a second
-- machine pointed at the same drive for the same reason.
--
-- `query` is the query string with no leading `?`, exactly as `FilterQuery::rebuild` writes it, so
-- restoring is an ordinary link and nothing has to re-derive the spelling of a filter. **Empty is a
-- legal value and means the whole corpus**, which is why there is no CHECK on it.
--
-- A new table, so this file is the whole migration and `SCHEMA_VERSION` does not move: an older
-- build opening this corpus draws no strip, where a version bump would make it refuse the corpus
-- outright. Same reasoning as `tags` above.
CREATE TABLE IF NOT EXISTS saved_filters (
    id       INTEGER PRIMARY KEY,
    name     TEXT    NOT NULL,
    query    TEXT    NOT NULL,
    -- The folded name, for ordering the strip. Filled in Rust by `km_song::text::fold`, as
    -- `tags.sort_key` is and for its reason: `COLLATE NOCASE` is ASCII-only and these names are
    -- Portuguese.
    sort_key TEXT    NOT NULL DEFAULT '',
    -- Stamped on every write, create or replace. One column rather than created and updated:
    -- nothing reads the difference, and a saved filter that has been replaced is a new decision.
    saved_at TEXT    NOT NULL
);

-- The replace rule is a lookup by name, and two rows under one name is the state it exists to
-- prevent. On the name as typed rather than on the fold, which is what `favorites_root_name`
-- already decides: two names differing in case are two names, and refusing one for a reason nobody
-- can see in the box they typed it into is worse than allowing it.
CREATE UNIQUE INDEX IF NOT EXISTS saved_filters_name ON saved_filters(name);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
