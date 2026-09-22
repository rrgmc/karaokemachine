-- The catalog of installed packages and their songs.
--
-- **A song is identified by its number, and the number carries the package inside it.** A number is
-- `bank * 1000 + slot`: the slot is the song's number within its package, 1 to 999, and the bank is
-- the block of a thousand this machine put that package in. So `UNIQUE (number)` is the whole
-- identity again, and two packages that both number their songs from 1 cannot collide, because
-- `UNIQUE (bank)` on `packages` below means they are never in the same thousand.
--
-- **That is a reversal, and the reversed argument is worth keeping.** This file used to hold
-- `UNIQUE (prefix, number)` with a denormalized `prefix` column, and it said that packing the pair
-- into one integer column had been tried and rejected: two places read `number` *as a number* --
-- `duplicate_content` parses it out of a `GROUP_CONCAT` with `.ok()`, and `read_song` reads it into
-- a `u32` -- and neither stops compiling, so a packed value would have failed silently. Every word
-- of that was true **of packing a string into an integer column**. It says nothing about a bank,
-- which *is* a number: `read_song` reads the whole code correctly by doing exactly what it always
-- did, and there is no spelling of it that a `u32` gets wrong.
--
-- **`id` is a surrogate and means nothing outside this file.** It exists because SQLite's FTS5
-- external-content tables need an integer rowid to point at, and nothing else. It is never shown,
-- never sent and never stored anywhere else -- the offline remote's favorites key on the *code*,
-- so a reinstall renumbering these cannot cost anybody their collection. They key on
-- `(package_id, content_hash)` beside it now, which covers the case the code alone does not: a
-- re-banked package moves the code as well, and only those two survive that.

PRAGMA foreign_keys = ON;

-- Facts about the catalog itself rather than about anything in it.
--
-- One row so far: `catalog_version`, a counter bumped inside the same transaction as every install
-- and every uninstall. It exists for the offline remote, which mirrors this catalog over the API
-- and needs to know whether re-downloading it would change anything -- on a corpus of any size that
-- is the difference between a refresh that costs nothing and one that costs a minute of somebody's
-- evening.
--
-- **Not `PRAGMA data_version`**, which is the obvious candidate and is the wrong one twice over: it
-- only moves when *another connection* writes, so a machine that installs a package through its own
-- handle sees no change at all, and it resets on restart, so a client that stored yesterday's value
-- would be told the catalog had gone backwards.
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO meta (key, value) VALUES ('catalog_version', '0');

CREATE TABLE IF NOT EXISTS packages (
    id           TEXT    PRIMARY KEY,
    name         TEXT    NOT NULL,
    version      TEXT    NOT NULL,
    -- Packages are read in place, so the path is how songs are found later. It is stored absolute.
    path         TEXT    NOT NULL,
    song_count   INTEGER NOT NULL,
    installed_at TEXT    NOT NULL,
    -- The block of a thousand this package's songs are dialled in, 1 to 9999. The machine's
    -- assignment; a package can only *suggest* one. See the `A song number is a bank and a slot`
    -- and `Bank 0 is the machine's own` decisions.
    --
    -- The default is never taken -- every insert names a bank, and `Library::install` refuses 0 --
    -- so it stands as the column's zero value rather than as a bank anything lands in.
    bank         INTEGER NOT NULL DEFAULT 0,
    -- The package header's flags word, whole and with unknown bits kept, so a new flag needs no
    -- column. See `A package's header carries flags, and an unknown one is kept`.
    flags        INTEGER NOT NULL DEFAULT 0
);

-- Two packages may not share a bank, or their songs would not have distinct numbers after all.
--
-- **Not partial, unlike the prefix index it replaces**, and that difference is the point rather than
-- a detail. Every package has exactly one bank -- there is no "no bank" the way there was "no
-- prefix" -- so this is a total constraint, and it is what makes a number collision between two
-- packages *impossible* rather than something the install has to look for and refuse.
CREATE UNIQUE INDEX IF NOT EXISTS packages_bank ON packages(bank);

CREATE TABLE IF NOT EXISTS songs (
    id                INTEGER PRIMARY KEY,
    -- The whole dialled number: the package's bank times a thousand, plus the song's slot within it.
    -- Not the slot the package carries -- that one is only unique inside its own package.
    number            INTEGER NOT NULL,
    package_id        TEXT    NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    title             TEXT    NOT NULL,
    artist            TEXT,
    language          TEXT,
    -- 'midi' or 'video'. Defaulted rather than nullable so every row answers the question.
    kind              TEXT    NOT NULL DEFAULT 'midi',
    -- For a MIDI song, a path inside the package archive. For a video song, a file name inside the
    -- package's media folder -- the sibling directory named after the package.
    file              TEXT    NOT NULL,
    duration_ms       INTEGER NOT NULL,
    lyric_encoding    TEXT,
    default_transpose INTEGER NOT NULL DEFAULT 0,
    -- Whether the machine plays this song and draws none of its words, because they are mistimed,
    -- are the arranger's business card, or are a chord chart. The package decides it -- by
    -- measurement, or because a curator said so -- and this is what the machine reads at song
    -- start. 0 is what every song is and was, so the default is the whole migration for a row.
    lyrics_hidden     INTEGER NOT NULL DEFAULT 0,
    -- The corrections in force on the song's own MIDI events, as the manifest's JSON array. Stored
    -- whole rather than split into columns so that a fix this build cannot read still reaches the
    -- machine that can: the list is carried, not interpreted, until playback resolves it.
    fixes             TEXT    NOT NULL DEFAULT '[]',
    -- NULL when melody detection abstained. The machine must not guess at playback.
    melody_channel    INTEGER,
    suitability       INTEGER,
    content_hash      TEXT,
    -- The song's first line or two, as the package recorded them, newline-separated. A line cannot
    -- contain a newline by construction -- the timeline is what splits them -- so one column is
    -- enough and no JSON is needed. NULL for a song whose package carries none, which is every song
    -- of a package built before previews existed, and every video and MP3+G song for ever.
    -- NOT in the FTS index: `songs_fts` declares title and artist, and adding a column to an FTS5
    -- table means dropping and rebuilding it. Searching the words is a different feature.
    lyric_preview     TEXT,
    -- The accent-folded title and artist, for `ORDER BY` and nothing else.
    --
    -- Not a nicety. SQLite's `COLLATE NOCASE` is ASCII-only, so it sorts every accented character
    -- *after* `Z` -- on a Portuguese corpus that buries a hundred songs past the end of the
    -- alphabet, on the machine's own screens and on the remote the machine serves. `É o amor` came
    -- last in a list of eleven thousand songs.
    --
    -- Filled in Rust by `km_song::text::fold`, which is deliberately the same folding
    -- `unicode61 remove_diacritics 2` implies for `songs_fts` below, so the alphabet somebody
    -- browses and the alphabet they search are one alphabet. A song with no artist folds to the
    -- empty string, which sorts before every name -- the same place `artist COLLATE NOCASE` put it,
    -- because SQLite sorts NULL first.
    --
    -- **There is no `alpha` here, and its absence is a decision rather than an omission.** The
    -- folded *initial* is what the A-Z strip needs, and that strip is deliberately offline-only.
    -- See `km-remote-core`'s schema and `The two remotes` in `docs/decisions/remotes.md`.
    sort_key          TEXT    NOT NULL DEFAULT '',
    sort_artist       TEXT    NOT NULL DEFAULT '',
    -- The song's tags, sorted and comma-joined. A tag cannot contain a comma by construction -- see
    -- `km_kmpkg::Tag` -- so one column is enough, exactly the argument `lyric_preview` makes about
    -- newlines above.
    --
    -- **This is the canonical value, and `song_tags` below is an index over it rather than a second
    -- truth.** The reason is `SONG_COLUMNS` in lib.rs: `package_digest` selects through that list,
    -- so a column here is what makes a package rebuilt with only its tags changed move
    -- `catalog_version`. With the join table alone the digest would be identical, every mirrored
    -- phone would keep the tags it already had, and nothing would ever say so.
    tags              TEXT    NOT NULL DEFAULT '',
    -- Integrated loudness in LUFS, as the package measured it, and NULL where nothing did: every
    -- MIDI song for ever, and every song of a package built before levelling existed. The machine
    -- attenuates a video or MP3+G song from this to the level its SoundFont bank plays at; a NULL
    -- plays at gain 1.0, which is exactly what every song did before.
    --
    -- REAL rather than an integer of tenths. It is a measurement rather than a rating, the two
    -- decoders hand back an `f32`, and rounding on the way into a column that only ever feeds a
    -- logarithm buys nothing.
    --
    -- The true peak the manifest also carries is deliberately **not** here: the machine never reads
    -- it, since levelling only attenuates and no attenuation can clip. It stays in the package,
    -- which is where the question it exists to answer would be asked from.
    loudness_lufs     REAL,
    UNIQUE (number)
);

CREATE INDEX IF NOT EXISTS songs_package      ON songs(package_id);
CREATE INDEX IF NOT EXISTS songs_artist       ON songs(artist);
CREATE INDEX IF NOT EXISTS songs_suitability  ON songs(suitability);
-- Narrowing a search to one language. An index rather than a column, so `execute_batch` alone brings
-- it to a machine already in service -- `prepare_existing` in lib.rs is only for columns.
CREATE INDEX IF NOT EXISTS songs_language     ON songs(language);
-- Duplicate detection across packages: the same recording filed under two numbers.
CREATE INDEX IF NOT EXISTS songs_content_hash ON songs(content_hash);
-- Browsing by name. Indexes rather than columns as far as an installed machine is concerned:
-- `execute_batch` alone brings these two, where the columns they key on needed `prepare_existing`.
-- `songs_artist` above stays -- it serves `GROUP BY artist` in the artist list, which still groups
-- on the name somebody typed and not on the fold.
CREATE INDEX IF NOT EXISTS songs_sort_key    ON songs(sort_key);
CREATE INDEX IF NOT EXISTS songs_sort_artist ON songs(sort_artist);

-- One row per tag a song carries: the index the tag filter and the vocabulary list seek through,
-- rather than scanning a six-figure catalog and splitting a string per row.
--
-- **A derived index, in the sense `songs_fts` below is one.** `songs.tags` is the value; this is how
-- it is looked up. The difference from `songs_fts` is who fills it: that one has triggers, and this
-- one is written by `Library::install`, because a trigger cannot split a string without a recursive
-- CTE and `install` is the only path that writes a song here at all.
--
-- A new table, so `execute_batch` alone brings it to a machine already in service -- `prepare_existing` in
-- lib.rs is only for columns, which is where `songs.tags` needed an entry.
CREATE TABLE IF NOT EXISTS song_tags (
    song_id INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    PRIMARY KEY (song_id, tag)
);
CREATE INDEX IF NOT EXISTS song_tags_tag ON song_tags(tag);

-- Full-text search over title and artist.
--
-- `remove_diacritics 2` is not cosmetic for this catalog: much of the corpus is Portuguese, and a
-- singer typing "cancao" or "coracao" must find "canção" and "coração". Option 2 rather than 1
-- because 1 leaves some Latin-1 supplement characters alone.
CREATE VIRTUAL TABLE IF NOT EXISTS songs_fts USING fts5(
    title,
    artist,
    content='songs',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Kept in step by triggers rather than by application code, so a write path that forgets to update
-- the index cannot exist.
CREATE TRIGGER IF NOT EXISTS songs_fts_insert AFTER INSERT ON songs BEGIN
    INSERT INTO songs_fts(rowid, title, artist) VALUES (new.id, new.title, new.artist);
END;

CREATE TRIGGER IF NOT EXISTS songs_fts_delete AFTER DELETE ON songs BEGIN
    INSERT INTO songs_fts(songs_fts, rowid, title, artist)
        VALUES ('delete', old.id, old.title, old.artist);
END;

CREATE TRIGGER IF NOT EXISTS songs_fts_update AFTER UPDATE ON songs BEGIN
    INSERT INTO songs_fts(songs_fts, rowid, title, artist)
        VALUES ('delete', old.id, old.title, old.artist);
    INSERT INTO songs_fts(rowid, title, artist) VALUES (new.id, new.title, new.artist);
END;
