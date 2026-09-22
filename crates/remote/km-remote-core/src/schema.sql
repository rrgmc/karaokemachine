-- This phone's copy of a machine's catalog.
--
-- Deliberately shaped like `km-catalog`'s `songs` table, because it holds the same rows and is
-- searched the same way -- `km_catalog::fts_match_query` builds the MATCH string for both, so a
-- search here means exactly what the same search means on the machine. What it is *not* is a
-- `km-catalog`: nothing here is installed, a package is only a name, there are no files, and a song
-- row is whatever the export sent.

PRAGMA foreign_keys = ON;

-- Which machine this is a copy of, and how far along.
--
-- The instance id is beside the version on purpose. A catalog version is monotonic within one
-- machine and meaningless between two, so a phone that was pointed at a different machine has to
-- notice: the same number against a different id is not "nothing has changed", it is a different
-- catalog entirely, and mirroring on top of the old one would leave a list that is half of each.
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS songs (
    -- A surrogate, because FTS5 external content needs an integer rowid. The identity is the code:
    -- see `UNIQUE (number)` below and the same note in `km-catalog`'s schema. Nothing stores
    -- this id -- favorites key on the code text, and on `package_id` and `content_hash` beside it --
    -- so the mirror being rebuilt and renumbering them costs nobody their collection.
    id                INTEGER PRIMARY KEY,
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
    -- What the package said this song's content hashes to, or NULL for a package that predates the
    -- field. **Mirrored for one reason: a favorite rejoins on it.** The code a favorite was filed
    -- under carries a bank, the bank is the machine's to assign rather than the package's, and it
    -- moves under the collection whenever a package is re-banked or lands somewhere else on another
    -- machine. `(package_id, content_hash)` moves for none of that.
    --
    -- Nothing here browses or searches by it, so it is carried rather than used: the two indexes
    -- below serve `Mirror::resolve` and nothing else.
    content_hash      TEXT,

    -- `sort_key` and `sort_artist` are the accent-folded title and artist. Ordering by `title` or by
    -- `artist COLLATE NOCASE` instead would be wrong rather than merely different: SQLite's BINARY
    -- collation sorts every accented character after `Z` and `NOCASE` is ASCII-only, which on a
    -- Portuguese corpus buries a hundred songs past the end of the alphabet.
    --
    -- **These two are no longer the columns the machine's catalog does not have.** `library.sqlite`
    -- carries the same pair now, filled by the same `km_song::text::fold`, so the two sides mean the
    -- same thing by construction rather than by coincidence — which is what makes an online remote
    -- and an offline one show one alphabet.
    --
    -- **`initial` is the one column the machine does not have, and it alone is why the A-Z strip is an
    -- offline-only capability.** It is the folded initial, or `#` for a digit. Indexed, which is the
    -- whole point -- the alternative is `substr(upper(title),1,1)` in the WHERE clause, which no
    -- index can serve and which would scan the table for every tap on a letter.
    sort_key          TEXT    NOT NULL DEFAULT '',
    sort_artist       TEXT    NOT NULL DEFAULT '',
    initial             TEXT,
    -- The song's tags, sorted and comma-joined -- the same column `km-catalog` carries, for the
    -- same reason a tag cannot contain a comma. Mirrored where `lyric_preview` deliberately is not,
    -- and the difference is what the pages draw: nothing here shows a preview, and the tag filter
    -- has to work with the machine switched off, which is the whole point of this database.
    tags              TEXT    NOT NULL DEFAULT '',
    UNIQUE (number)
);

CREATE INDEX IF NOT EXISTS songs_initial       ON songs(initial);
CREATE INDEX IF NOT EXISTS songs_sort_key    ON songs(sort_key);
CREATE INDEX IF NOT EXISTS songs_sort_artist ON songs(sort_artist);
CREATE INDEX IF NOT EXISTS songs_artist   ON songs(artist);
CREATE INDEX IF NOT EXISTS songs_language ON songs(language);
-- The two rungs of `Mirror::resolve` that are not the number. `km-catalog` indexes the same pair
-- for its own duplicate scan; here they exist so that resolving a folder of favorites is a seek per
-- song rather than a scan per song.
CREATE INDEX IF NOT EXISTS songs_content_hash ON songs(content_hash);
CREATE INDEX IF NOT EXISTS songs_package      ON songs(package_id);

-- What each package is called and which flags it carries, for the list a person hides packages
-- from on Setup. A song row already carries its package id, and nothing here installs or removes a
-- package.
-- Not a foreign key from `songs`, because the two arrive in separate requests and a song whose
-- package has no row here still has to be browsable.
CREATE TABLE IF NOT EXISTS packages (
    id    TEXT PRIMARY KEY,
    name  TEXT NOT NULL,
    -- The package header's flags word, whole, as `km-catalog` keeps it: a new flag needs no column.
    flags INTEGER NOT NULL DEFAULT 0
);

-- One row per tag a song carries: the index the tag filter and the tag picker seek through. The
-- twin of `km-catalog`'s, filled by `Mirror::replace` rather than by a trigger for that one's
-- reason -- SQL cannot split a string without a recursive CTE, and `replace` is the only write path.
CREATE TABLE IF NOT EXISTS song_tags (
    song_id INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    PRIMARY KEY (song_id, tag)
);
CREATE INDEX IF NOT EXISTS song_tags_tag ON song_tags(tag);

-- The same tokenizer as the machine's, and `remove_diacritics 2` is load-bearing for the same
-- reason: much of the corpus is Portuguese, and somebody typing "coracao" has to find "coração".
CREATE VIRTUAL TABLE IF NOT EXISTS songs_fts USING fts5(
    title,
    artist,
    content='songs',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Kept in step by triggers rather than by application code, so an import path that forgets to update
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
