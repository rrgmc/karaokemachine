//! Everything a person typed into a corpus, in a file they can keep somewhere else.
//!
//! # What is in it, and why that is the whole design
//!
//! `schema.sql` states the split this module is built on: `det_*` is what the file said and is
//! rewritten by every scan; the bare columns are what a person typed and are never touched by the
//! scanner. The first half is free — point the tool at the folder again and it comes back. **The
//! second half exists nowhere else.** Hundreds of thousands of songs' analysis is rebuildable; the few
//! thousand titles somebody sat and corrected are not, and a backup of the rebuildable half would be
//! a hundred megabytes describing what a scan already knows.
//!
//! So a backup carries the nine hand-set columns of `songs`, the favorites and their membership,
//! and nothing else. Packages and duplicate verdicts are hand-set too and are deliberately out of
//! this first cut — the file says so in its own `note`, so that a person reading one is not left to
//! infer it from an absence.
//!
//! # How it finds its way back
//!
//! By the content hash of a song's bytes, which is `songs.id` — see the `Song identity in curation`
//! decision. That is the one key here that survives a rebuilt index, a different machine, or a
//! replaced drive, and every other identity in the document is expressed through it: a favorite is
//! its name rather than its rowid, because a rowid means nothing in another database.
//!
//! A song in the file whose bytes are nowhere under this root is **listed, never created**. That is
//! `crate::build::import`'s rule and the argument is stronger here: `kind`, `duration_ms` and
//! `warnings` are `NOT NULL` with no honest value for a song nobody has read, and an invented row
//! would enter `songs_fts` through the insert trigger and be findable in the browse list as a song
//! with no file, no length and a fabricated suitability. So a restore rejoins; it does not conjure, and
//! the answer to an unmatched song is to scan.
//!
//! # Two rules a restore keeps
//!
//! **Nothing is ever blanked.** NULL in a hand-set column means "nobody has said", not "empty", so
//! there is no such thing as a recorded decision to be blank and a field the file does not mention
//! is left exactly as it is. That is what makes restoring the wrong file noisy rather than
//! destructive, and it holds under both policies.
//!
//! **A refusal is a line in a report, never a rollback.** Everything this build will not accept — a
//! language it cannot read, a rating above ten, a transposition that will not fit, a merge that
//! would chain — is found *before* the transaction opens. `Db::edit_song` builds one `UPDATE` for
//! all its fields, so a single bad language would take the good title beside it down with the
//! statement; `crate::build::import` hit exactly this and its comment says so.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::{Db, DbError};

/// The document shape this build writes.
///
/// **A newer format is reported and read as far as this build understands it.**
/// [`km_pack::Spec`](crate::build) denies unknown fields and argues for it in writing, because a
/// description is hand-written and one silent typo dropped four thousand songs. The trade inverts
/// here: nobody types into a backup, so there is no typo to catch, and the expensive failure is an
/// older build refusing a newer file at the moment somebody is trying to get their work back. A
/// backup is what you have when the database is not, so recovery degrades rather than refuses.
///
/// **An older format is refused, with both numbers.** Nothing in this build reads its shape, and a
/// document read as a shape it does not have would restore the wrong names under the right songs.
pub const FORMAT: u32 = 2;

/// The `songs` columns a person fills in, and the only ones a backup carries.
///
/// **Written down, and that is forced rather than chosen.** A hand-written column list is how
/// somebody's corrections get lost: one forgotten column and every hand-typed title is gone, with
/// nothing to say so. This module has nothing to derive it from — a JSON key is not a column — so the
/// list is written, and `the_backed_up_fields_are_exactly_the_hand_set_columns_the_schema_has`
/// makes `schema.sql` the authority over it. Add a hand-set column and forget this constant, and
/// that test fails by name.
pub(crate) const HAND_SET_COLUMNS: &[&str] = &[
    "title",
    "artist",
    "language",
    "lyric_encoding",
    "default_transpose",
    "lyrics_hidden",
    "fixes",
    "melody_chosen",
    "user_score",
    "notes",
    "merged_into",
];

/// Everything else in `songs`: what a scan writes, what a trigger keeps, what this build derives.
///
/// The complement of [`HAND_SET_COLUMNS`], written out so that a column added to `schema.sql` lands
/// in *neither* list and fails the guard test with its own name in the message. **Adding a name here
/// is a claim that no person ever types it**, and is the one line in this file to read twice.
#[cfg(test)]
const DERIVED_COLUMNS: &[&str] = &[
    "id",
    "kind",
    "det_title",
    "det_artist",
    "det_language",
    "det_language_tag",
    "det_language_guess",
    "det_language_guess_confidence",
    "stem",
    "flavor",
    "granularity",
    "duration_ms",
    "note_count",
    "channel_count",
    "line_count",
    "syllable_count",
    "det_encoding",
    "det_encoding_source",
    "melody_channel",
    "melody_confidence",
    "melody_abstained",
    // Not a measurement of the file but a note about the measurements: which build made them. It
    // belongs here rather than beside the hand-set columns for the reason the rest of this list
    // does — a scan writes it, nobody types it, and restoring one from a backup would be a claim
    // about a build that never saw these bytes.
    "analysis_revision",
    "suitability",
    "suitability_lyrics",
    "suitability_sync",
    "suitability_channels",
    "suitability_arrangement",
    "warnings",
    "width",
    "height",
    "frame_rate_milli",
    "video_codec",
    "audio_codec",
    "cdg_graphics_path",
    "cdg_sample_rate",
    "cdg_channels",
    "cdg_packets",
    "cdg_graphics_ms",
    "cdg_short_by_ms",
    "cdg_tiles",
    "cdg_unknown",
    "lyrics",
    "fingerprint",
    // The cluster a song was put in and how big it is. Derived, and pointedly so: a person can
    // release a song from a cluster but never state one, and the next pass would put it back. What
    // survives a backup instead is the *dismissal* -- a verdict on a pair, which is the thing
    // somebody actually typed.
    "duplicate_of",
    "version_count",
    "file_count",
    // Folded from the columns above them by `Db::refold`. Nobody types a fold, so a backup that
    // carried them would be carrying a value it could recompute — and a stale one, if the title it
    // was folded from were edited before the restore.
    "sort_title",
    "sort_artist",
    "first_seen",
    "last_scanned",
    // Stamped by `songs_stamp_update`, never typed. A backup carrying it would be restoring a claim
    // about a different database, and the honest answer to when *this* corpus last changed a song is
    // the restore -- which is what the trigger writes, `apply_restore` setting the hand-set columns
    // being an edit like any other.
    "updated_at",
];

/// Keys on [`SongBackup`] that are not columns of `songs`, and why each is allowed to be one.
///
/// `id` is the key itself; `favorites` is a join table folded onto the row so that one song reads as
/// one object; `seen_as` is the effective title, carried so that an unmatched song can be *named* in
/// a report rather than reported as a bare hash, and never written back.
#[cfg(test)]
const NON_COLUMN_KEYS: &[&str] = &["id", "favorites", "seen_as"];

/// The sentence every backup opens with.
///
/// JSON has no comments, so the job `km_pack::Spec::HEADER` does in a description has to be done by a
/// field. Written on the way out and ignored on the way in.
fn note() -> String {
    "Hand-curated data from KaraokeMachine Package Builder. Songs are keyed by the content hash of \
     their bytes, so this restores onto a rebuilt index or another machine. It holds only what a \
     person typed -- titles, artists, languages, encodings, transpositions, ratings, notes, merges \
     and favorites. Nothing a scan can work out again is here, and neither are packages or \
     duplicate verdicts."
        .to_owned()
}

/// The format number a file with no `format` key is assumed to be.
fn first_format() -> u32 {
    1
}

/// Everything hand-set in one corpus.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    /// What this file is, for whoever opens it. See [`note`].
    #[serde(default = "note")]
    pub note: String,
    /// The document shape. See [`FORMAT`].
    #[serde(default = "first_format")]
    pub format: u32,
    /// When it was written.
    #[serde(default)]
    pub written_at: String,
    /// The corpus it was taken from.
    ///
    /// **Descriptive only — a restore never reads it.** A backup carried to another machine names a
    /// path that is not there, and a restore that checked would refuse the case this whole file
    /// exists for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The favorites, so that an empty one survives a round trip.
    ///
    /// Its own array rather than something implied by the memberships below: a favorite somebody
    /// made and has not filed anything under yet is still a favorite they made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub favorites: Vec<FavoriteBackup>,
    /// Every song carrying something a person typed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub songs: Vec<SongBackup>,
}

/// One favorite, by name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FavoriteBackup {
    /// What the list is called, which is what a restore matches it on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Whether this list is scaffolding for a later pass rather than a filing.
    ///
    /// Left out of the file when it is false, which is what nearly every favorite is, and read as
    /// false when a file does not carry it — so a backup written before the flag existed restores
    /// its favorites as the filings they were made as.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub temporary: bool,
}

impl FavoriteBackup {
    /// The list's name.
    pub fn name(&self) -> String {
        self.name.clone()
    }
}

/// How a song names a favorite it is filed under: by the favorite's name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FavoriteRef {
    /// The name, as a flat favorite has one.
    Name(String),
}

impl FavoriteRef {
    /// The name this names.
    pub fn name(&self) -> String {
        match self {
            Self::Name(name) => name.clone(),
        }
    }
}

/// One song's corrections, keyed by the hash of its bytes.
///
/// The field names are the SQL column names, exactly. That is load-bearing rather than tidy: the
/// guard test compares this struct's serialized key set against `pragma_table_info('songs')`, and it
/// can only do that if the two spell things the same.
///
/// **The two numeric fields are `i64` and not the narrow types they end up in.** A rating is a
/// `u8` in the database and a transposition an `i8`, and a file carrying `999` or `-1` in one of
/// them would fail to *parse* — taking every title in the document down with it. Held wide, one
/// impossible value is one line in a report. This is the module's stated rule about degrading rather
/// than refusing, spelled out in a type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongBackup {
    /// The content hash of the file's bytes: `songs.id`, and the rejoin key.
    pub id: String,
    /// The title somebody typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The performer somebody typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// The language somebody chose. Canonicalised on the way back in, so `PT` arrives as `pt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The lyric encoding somebody pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyric_encoding: Option<String>,
    /// The transposition to apply by default. Narrowed to `i8` on the way in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_transpose: Option<i64>,
    /// Whether somebody said to play the song and draw none of its words.
    ///
    /// **`Some(false)` is an answer and not an absence**, which is why the field is skipped only
    /// when it is `None`: somebody saying the words *are* to be drawn is overruling the analysis
    /// just as much as somebody silencing them, and a backup that dropped it would restore a corpus
    /// with that decision missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics_hidden: Option<bool>,
    /// The corrections somebody decided on, as stored JSON. Checked on the way in rather than
    /// parsed into types: a list this build cannot read belongs to a newer one and is carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixes: Option<String>,
    /// The melody channel somebody named: `none`, or a channel. Carried as stored rather than
    /// parsed, for the reason `fixes` above is -- a spelling this build cannot read belongs to a
    /// newer one and is somebody's answer either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub melody_chosen: Option<String>,
    /// How good a karaoke file somebody said this is, 0-10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_score: Option<i64>,
    /// Whatever somebody wrote about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The song this one was folded into, when somebody said they are the same recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_into: Option<String>,
    /// The favorites this song is filed under, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub favorites: Vec<FavoriteRef>,
    /// What this song was called when the backup was written.
    ///
    /// **Never restored.** It is here so that a song this corpus no longer holds can be *named* in
    /// the report rather than reported as a sixty-four-character hash — the same reason
    /// `ImportReport::unmatched` carries a title beside its number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_as: Option<String>,
}

impl Backup {
    /// Reads a backup.
    ///
    /// A file this build cannot parse at all is an error naming the file. Anything it *can* parse is
    /// loaded, whatever else is in it — there is no `deny_unknown_fields` here, on purpose. See
    /// [`FORMAT`].
    pub fn read(path: &Path) -> Result<Self, DbError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| DbError::Rejected(format!("reading {}: {error}", path.display())))?;
        // **The format first, on its own.** An older shape can fail to parse as this one, and the
        // error that produced would name a field rather than the reason.
        let format = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value.get("format").and_then(serde_json::Value::as_u64))
            .map_or(first_format(), |format| {
                u32::try_from(format).unwrap_or(u32::MAX)
            });
        if format < FORMAT {
            return Err(DbError::Rejected(format!(
                "{} is a backup in format {format}, and this build reads format {FORMAT}",
                path.display()
            )));
        }
        serde_json::from_str(&text)
            .map_err(|error| DbError::Rejected(format!("reading {}: {error}", path.display())))
    }

    /// Writes a backup, **through a temporary file and a rename**.
    ///
    /// The one place this departs from `km_pack::Spec::write`, and the reason is the difference
    /// between the two documents. A description regenerates from the database in one click; a backup
    /// is what somebody has *when the database is gone*, and a half-written one that overwrote last
    /// week's good one is the single failure that would make this feature worth less than not having
    /// it. So the bytes land beside the target and `std::fs::rename` puts them in place, which
    /// replaces an existing file on every platform this ships to.
    ///
    /// Pretty-printed, like `recent.rs` and for the same reason: a person opens it.
    pub fn write(&self, path: &Path) -> Result<(), DbError> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| DbError::Rejected(format!("writing the backup: {error}")))?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                DbError::Rejected(format!("creating {}: {error}", parent.display()))
            })?;
        }

        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".writing");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, text).map_err(|error| {
            DbError::Rejected(format!("writing {}: {error}", temporary.display()))
        })?;
        std::fs::rename(&temporary, path).map_err(|error| {
            // The rename is what makes this safe, so a failed one must not leave the scratch file
            // behind pretending to be a backup.
            let _ = std::fs::remove_file(&temporary);
            DbError::Rejected(format!("writing {}: {error}", path.display()))
        })?;
        Ok(())
    }
}

/// Where a backup goes if nobody says otherwise: the corpus's own data folder, named after the tool
/// and after the moment — `km-package-builder-20260909T140233Z.kmbackup.json`.
///
/// **The moment is what makes it a backup rather than a copy.** A backup is a thing somebody takes
/// more than once, and one fixed name means the second one destroys the first: the state anybody
/// wants to go back to is the one before whatever they have just noticed, which is exactly the file
/// the fixed name has already overwritten. That is the failure [`Backup::write`]'s rename exists to
/// prevent, arriving by a door the rename cannot cover.
///
/// `.json` last so that a file manager and an editor both know what it is, with the middle segment
/// saying what kind of JSON. **Deliberately not a `.kmbuild`-shaped name**: `db::database_in` refuses
/// a folder holding two of those, so a backup that looked like a second database would stop the
/// folder opening at all. That remains true and is belt and braces, since
/// [`crate::db::DATA_SUBDIR`] is not where `database_in` looks in the first place.
///
/// **Still somewhere to copy off this drive**, which the settings page says in as many words. A
/// default has to be somewhere, and beside the thing it describes is the only place that needs no
/// explaining; it is not a claim that a backup inside the corpus is a backup.
pub fn default_path(root: &Path) -> PathBuf {
    crate::db::data_dir(root).join(format!("{STEM}-{}{SUFFIX}", file_stamp()))
}

/// What every default backup name begins with.
const STEM: &str = "km-package-builder";

/// What every backup name ends with, and the part that is not a matter of taste.
///
/// Two things read it. The scan collects by extension and this is none of the ones it collects, so
/// a data folder inside the tree being scanned cannot enter the catalog; and [`crate::db`]'s
/// `database_in` refuses a folder holding two `.kmbuild`-shaped files, which a backup that looked
/// like a second database would make of every corpus. Only the stem carries the moment.
const SUFFIX: &str = ".kmbackup.json";

/// The moment inside the document, in a form a file name may take: `20260909T140233Z`.
///
/// **Sliced from [`crate::scan::timestamp`] rather than read from a second clock**, which is what
/// [`Backup::written_at`] is set from — so the name and the contents cannot disagree about when a
/// backup was taken. It is also the reason this is not a sixth hand-rolled clock in a workspace
/// that carries no date library.
///
/// The separators come out because `:` is not a legal character in a Windows file name and this
/// tool ships a Windows file association. What is left is `km-logfile`'s own stamp, so a dated file
/// looks the same wherever this workspace writes one, and it sorts: newest is last in name order,
/// which is what [`newest`] leans on.
fn file_stamp() -> String {
    crate::scan::timestamp().replace(['-', ':'], "")
}

/// The most recent backup in the data folder, by name.
///
/// **The other half of a dated default.** One fixed name was a name anybody could type into the
/// restore box from memory; the moment that makes each backup its own file is also what makes the
/// name unmemorable, so the page has to say which one is newest rather than leaving somebody to
/// open a file manager at the moment they have already lost something.
///
/// Name order is time order, which is [`file_stamp`]'s doing. `None` covers a folder with no
/// backups, a folder that cannot be read, and one holding only hand-named backups — three
/// situations with one honest answer for a page: there is nothing to suggest.
pub fn newest(root: &Path) -> Option<String> {
    std::fs::read_dir(crate::db::data_dir(root))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(STEM) && name.ends_with(SUFFIX))
        .max()
}

/// How much a backup turned out to hold, for the sentence a page prints afterwards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Songs carrying something a person typed.
    pub songs: usize,
    /// Favorites in the tree.
    pub favorites: usize,
}

/// Reads everything hand-set out of a database.
///
/// A pure `Db -> value` reader with no file and no policy in it, the shape `build::spec_for` has —
/// which is what makes the whole of it testable without a disk.
pub fn backup_of(db: &Db) -> Result<Backup, DbError> {
    let songs = db.hand_set_songs()?;
    let mut memberships = db.favorite_memberships()?;

    Ok(Backup {
        note: note(),
        format: FORMAT,
        written_at: crate::scan::timestamp(),
        root: Some(km_pack::spec::slashed(
            &crate::model::tidy(db.root()).display().to_string(),
        )),
        favorites: db
            .favorite_names()?
            .into_iter()
            .map(|(name, temporary)| FavoriteBackup { name, temporary })
            .collect(),
        songs: songs
            .into_iter()
            .map(|song| SongBackup {
                favorites: memberships
                    .remove(&song.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(FavoriteRef::Name)
                    .collect(),
                seen_as: Some(song.seen_as),
                id: song.id,
                title: song.title,
                artist: song.artist,
                language: song.language,
                lyric_encoding: song.lyric_encoding,
                default_transpose: song.default_transpose,
                lyrics_hidden: song.lyrics_hidden,
                fixes: song.fixes,
                melody_chosen: song.melody_chosen,
                user_score: song.user_score,
                notes: song.notes,
                merged_into: song.merged_into,
            })
            .collect(),
    })
}

/// Reads a database and writes the file, which is the whole of what a caller wants.
pub fn write(db: &Db, out: &Path) -> Result<Counts, DbError> {
    let backup = backup_of(db)?;
    backup.write(out)?;
    Ok(Counts {
        songs: backup.songs.len(),
        favorites: backup.favorites.len(),
    })
}

/// Whether a restore may write over a value this database already holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Policy {
    /// Write only where this database has nothing.
    ///
    /// The default, and the safe direction: a restore can put work back and can never take work
    /// away.
    #[default]
    FillBlanks,
    /// The file wins wherever the two disagree.
    Overwrite,
}

impl Policy {
    /// The flag that travels into the SQL.
    fn overwrites(self) -> bool {
        matches!(self, Self::Overwrite)
    }
}

/// One song's restore, with every value already checked.
///
/// Nothing here can be refused any more — that happened in [`plan`] — so the transaction below has
/// only writes in it and no branch that could abandon one halfway.
#[derive(Debug, Clone)]
pub struct PlannedSong {
    /// The song this writes to. Known to exist.
    pub id: String,
    /// Title, artist, language, encoding and notes, in that order, as they will be bound.
    pub title: Option<String>,
    /// See [`Self::title`].
    pub artist: Option<String>,
    /// Canonicalised to an ISO 639-1 tag.
    pub language: Option<String>,
    /// See [`Self::title`].
    pub lyric_encoding: Option<String>,
    /// Narrowed to what the column takes.
    pub default_transpose: Option<i64>,
    /// Somebody's answer about whether the words are drawn, where they gave one.
    pub lyrics_hidden: Option<bool>,
    /// See [`Self::title`].
    pub fixes: Option<String>,
    /// `none` or a channel, checked on the way in. See [`Self::title`].
    pub melody_chosen: Option<String>,
    /// Range-checked to 0-10.
    pub user_score: Option<i64>,
    /// See [`Self::title`].
    pub notes: Option<String>,
}

impl PlannedSong {
    /// Whether this song has anything to write.
    ///
    /// A song that reaches the plan with nothing set is one that is only *favorited*, and running a
    /// no-op `UPDATE songs` for it would still fire both FTS triggers — a delete and a reinsert per
    /// song, for nothing.
    fn writes_anything(&self) -> bool {
        self.title.is_some()
            || self.artist.is_some()
            || self.language.is_some()
            || self.lyric_encoding.is_some()
            || self.default_transpose.is_some()
            || self.lyrics_hidden.is_some()
            || self.fixes.is_some()
            || self.melody_chosen.is_some()
            || self.user_score.is_some()
            || self.notes.is_some()
    }
}

/// A checked restore, ready to be written in one transaction.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Whether the file wins where the two disagree.
    pub overwrite: bool,
    /// The favorites to make sure exist, by name.
    ///
    /// A name a membership asks for is here too, even when the file's own favorites array left it
    /// out. One reached that way carries the defaults, which is the honest reading: nothing in the
    /// file said it was a working list.
    ///
    /// The file's own favorite type, because a plan holds exactly the facts a file carries.
    pub favorites: Vec<FavoriteBackup>,
    /// The song fields to write.
    pub songs: Vec<PlannedSong>,
    /// `(song, target)` merges, each already known not to chain within the file.
    pub merges: Vec<(String, String)>,
    /// `(song, favorite name)` memberships to file.
    pub memberships: Vec<(String, String)>,
}

/// What a restore did, and what it could not do.
///
/// `ImportReport`'s shape: every refusal is counted and named, and none of them stops the restore.
#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    /// Songs whose fields were written.
    pub songs_applied: usize,
    /// Songs in the file whose bytes are nowhere in this corpus, with what they were called there.
    pub songs_unmatched: Vec<(String, String)>,
    /// Values this build refused, one sentence each.
    ///
    /// One list of prose rather than a vector per kind, which is where this parts company with
    /// `ImportReport::unreadable_language`. That one had a single refusal to report and could afford
    /// a typed pair; a restore has four, and four vectors would be four rendering branches for a
    /// list that is usually empty and is always read as prose.
    pub rejected: Vec<String>,
    /// Favorites this restore had to create.
    pub favorites_created: usize,
    /// Songs filed into a favorite.
    pub memberships_applied: usize,
    /// Merges recorded.
    pub merges_applied: usize,
    /// Set when the file says it was written by a later build than this one.
    pub from_a_later_format: Option<u32>,
}

/// Puts a backup back into this folder's database.
///
/// **All of it or none of it.** Every write runs inside a single transaction owned by
/// [`Db::apply_restore`], so a statement that will not go through leaves the titles alone rather than
/// leaving somebody with half a restore they cannot tell from a whole one. Anything this build
/// refuses is refused *before* that transaction opens, so a refusal is a line in the report and never
/// a rollback.
pub fn restore(db: &mut Db, backup: &Backup, policy: Policy) -> Result<RestoreReport, DbError> {
    let mut report = RestoreReport::default();
    if backup.format > FORMAT {
        report.from_a_later_format = Some(backup.format);
    }

    let plan = plan(db, backup, policy, &mut report)?;
    let outcome = db.apply_restore(&plan)?;

    report.songs_applied = outcome.songs_applied;
    report.favorites_created = outcome.favorites_created;
    report.memberships_applied = outcome.memberships_applied;
    report.merges_applied = outcome.merges_applied;
    for id in outcome.merges_refused {
        report.rejected.push(format!(
            "{}: the song it was merged into is itself merged here, and a merge is one level only",
            name_of(backup, &id)
        ));
    }
    Ok(report)
}

/// What to call a song in a report.
///
/// The name the backup recorded, else the hash. A hash is a poor thing to read in a list of ten and
/// is still better than nothing, which is what an older file with no `seen_as` leaves.
fn name_of(backup: &Backup, id: &str) -> String {
    backup
        .songs
        .iter()
        .find(|song| song.id == id)
        .and_then(|song| song.seen_as.clone())
        .unwrap_or_else(|| id.to_owned())
}

/// Reads the database and the file, and decides everything that can be decided without writing.
///
/// Nothing here changes anything, which is the property that makes a refusal free: a song this build
/// will not accept costs a line in the report and no recovery at all.
fn plan(
    db: &Db,
    backup: &Backup,
    policy: Policy,
    report: &mut RestoreReport,
) -> Result<Plan, DbError> {
    let ids: Vec<String> = backup.songs.iter().map(|song| song.id.clone()).collect();
    let present = db.existing_song_ids(&ids)?;

    // Which songs the *file* merges away. A merge is one level only, so a song that is itself merged
    // in here cannot also be somebody else's target -- checked against the file rather than the
    // half-restored database, because the database's answer changes as this very restore runs.
    let merged_in_file: BTreeSet<&str> = backup
        .songs
        .iter()
        .filter(|song| song.merged_into.is_some())
        .map(|song| song.id.as_str())
        .collect();

    let mut plan = Plan {
        overwrite: policy.overwrites(),
        ..Plan::default()
    };
    let mut wanted_favorites: BTreeMap<String, bool> = BTreeMap::new();
    for favorite in &backup.favorites {
        if let Some(name) = tidy_name(&favorite.name()) {
            wanted_favorites.insert(name, favorite.temporary);
        }
    }

    for song in &backup.songs {
        let name = song.seen_as.clone().unwrap_or_else(|| song.id.clone());
        if !present.contains(&song.id) {
            report.songs_unmatched.push((song.id.clone(), name));
            continue;
        }

        let language = match &song.language {
            // The same mapping `build::import` applies, for the same reason: the values in a file
            // are not this build's to trust. An older backup may carry a raw `ENGL`, and a newer one
            // a code this build has never heard of.
            Some(raw) => match km_kmpkg::Language::parse(raw) {
                Some(parsed) => Some(parsed.code().to_owned()),
                None => {
                    report
                        .rejected
                        .push(format!("{name}: {raw:?} is not a language code"));
                    None
                }
            },
            None => None,
        };

        let user_score = checked_score(song.user_score, &name, "rating", report);
        let default_transpose = match song.default_transpose {
            // Reported rather than clamped. A clamp silently changes a number somebody typed, and a
            // silent change is the one thing a restore must not do.
            Some(value) if i8::try_from(value).is_err() => {
                report.rejected.push(format!(
                    "{name}: a transposition of {value} is not a number of semitones"
                ));
                None
            }
            other => other,
        };
        let fixes = match &song.fixes {
            // Reported rather than dropped, on the same terms as the transposition above: a list
            // that is not a list is a file somebody edited by hand into something the column cannot
            // hold, and writing it would put a value in the database that no page can read back.
            Some(value) if crate::fixes::stored(Some(value)).is_none() => {
                report.rejected.push(format!(
                    "{name}: the corrections are not a list of corrections"
                ));
                None
            }
            other => other.clone(),
        };
        let melody_chosen = match &song.melody_chosen {
            // Checked the way the corrections above are: a spelling this build cannot read is a file
            // edited by hand into something the column cannot hold, and writing it would leave a
            // page unable to say which channel the song is supposed to sing on.
            Some(value) if crate::fixes::MelodyChoice::parse(Some(value)).is_none() => {
                report
                    .rejected
                    .push(format!("{name}: the melody channel is not a channel"));
                None
            }
            other => other.clone(),
        };

        if let Some(target) = &song.merged_into {
            if target == &song.id {
                report
                    .rejected
                    .push(format!("{name}: a song cannot be merged into itself"));
            } else if !present.contains(target) {
                report.rejected.push(format!(
                    "{name}: the song it was merged into is not in this folder"
                ));
            } else if merged_in_file.contains(target.as_str()) {
                report.rejected.push(format!(
                    "{name}: the song it was merged into is itself merged in this file, and a merge \
                     is one level only"
                ));
            } else {
                plan.merges.push((song.id.clone(), target.clone()));
            }
        }

        let planned = PlannedSong {
            id: song.id.clone(),
            title: song.title.clone(),
            artist: song.artist.clone(),
            language,
            lyric_encoding: song.lyric_encoding.clone(),
            default_transpose,
            // Nothing to check: a JSON boolean is already the whole of the column's range, so
            // there is no shape a hand-edited file could put here that the column cannot hold.
            lyrics_hidden: song.lyrics_hidden,
            fixes,
            melody_chosen,
            user_score,
            notes: song.notes.clone(),
        };
        if planned.writes_anything() {
            plan.songs.push(planned);
        }

        for reference in &song.favorites {
            if let Some(name) = tidy_name(&reference.name()) {
                wanted_favorites.entry(name.clone()).or_default();
                plan.memberships.push((song.id.clone(), name));
            }
        }
    }

    plan.favorites = wanted_favorites
        .into_iter()
        .map(|(name, temporary)| FavoriteBackup { name, temporary })
        .collect();

    Ok(plan)
}

/// A rating, if it is one this build can store.
fn checked_score(
    value: Option<i64>,
    name: &str,
    what: &str,
    report: &mut RestoreReport,
) -> Option<i64> {
    match value {
        Some(score) if !(0..=10).contains(&score) => {
            report
                .rejected
                .push(format!("{name}: {score} is not a {what} out of ten"));
            None
        }
        other => other,
    }
}

/// A favorite's name, trimmed, or `None` if there is nothing left of it.
///
/// `Db::create_favorite` refuses an empty name, so a file carrying one has to be dropped here rather
/// than aborting the transaction below.
fn tidy_name(name: &str) -> Option<String> {
    let tidied = name.trim();
    (!tidied.is_empty()).then(|| tidied.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Scratch;

    fn db() -> Db {
        Db::open_in_memory(Path::new("/corpus")).expect("open")
    }

    /// A scanned song with nothing hand-set on it, so a test can then set exactly what it means to.
    ///
    /// Through `write_scanned` rather than an `INSERT`, so the row is one the scanner would actually
    /// have produced — every `det_*` filled, the file beside it, and the triggers fired. A restore
    /// writing onto a row no scan could have made would prove nothing about the corpus this runs on.
    fn add_song(db: &mut Db, id: &str, stem: &str) {
        let song = crate::model::ScannedSong {
            id: id.to_owned(),
            det_title: None,
            det_artist: None,
            det_language: None,
            stem: stem.to_owned(),
            duration_ms: 200_000,
            lyrics: None,
            fingerprint: String::new(),
            suitability: crate::model::SuitabilityFacts {
                value: 7,
                breakdown: (3, 2, 2, 0),
                warnings: "[]".to_owned(),
            },
            midi: Some(crate::model::MidiFacts {
                flavor: "soft".to_owned(),
                granularity: "syllablelevel".to_owned(),
                note_count: 500,
                channel_count: 6,
                line_count: 20,
                syllable_count: 100,
                det_encoding: "windows-1252".to_owned(),
                det_encoding_source: "fallback".to_owned(),
                melody_channel: Some(3),
                melody_confidence: Some(0.9),
                melody_abstained: None,
            }),
            video: None,
            cdg: None,
            ultrastar: None,
        };
        let file = crate::model::ScannedFile {
            path: format!("folder/{stem}.kar"),
            size: 1234,
            mtime: 0,
            content_hash: Some(id.to_owned()),
            status: crate::model::ScanStatus::Ok,
            error: None,
            song: Some(song),
        };
        db.write_scanned(&[file], "2026-09-02T00:00:00Z")
            .expect("write a scanned song");
    }

    impl SongBackup {
        /// One song with every field set, for the guard test.
        ///
        /// A full struct literal on purpose, with no `..Default::default()`: that is what makes
        /// adding a field to this type a compile error *here* rather than a `None` which serializes
        /// to nothing and passes the guard by disappearing from it.
        fn filled_for_test() -> Self {
            Self {
                id: "abc".to_owned(),
                title: Some("Corcovado".to_owned()),
                artist: Some("Tom Jobim".to_owned()),
                language: Some("pt".to_owned()),
                lyric_encoding: Some("cp1252".to_owned()),
                default_transpose: Some(-2),
                lyrics_hidden: Some(true),
                fixes: Some("[{\"fix\":\"mute_channel\",\"channel\":2}]".to_owned()),
                melody_chosen: Some("3".to_owned()),
                user_score: Some(9),
                notes: Some("the good one".to_owned()),
                merged_into: Some("def".to_owned()),
                favorites: vec![FavoriteRef::Name("Brasil / Bossa".to_owned())],
                seen_as: Some("Corcovado".to_owned()),
            }
        }
    }

    /// A column added to `songs` and forgotten here is every value of it silently lost on the next
    /// restore. `schema.sql` is the authority, and this asserts against it directly.
    #[test]
    fn the_backed_up_fields_are_exactly_the_hand_set_columns_the_schema_has() {
        let db = db();
        let columns = db.columns_for_test("songs").expect("read the schema");

        // The half that catches a *new* column: it is in neither list, and the message names it and
        // says what to do with it.
        let unclassified: Vec<&String> = columns
            .iter()
            .filter(|column| !HAND_SET_COLUMNS.contains(&column.as_str()))
            .filter(|column| !DERIVED_COLUMNS.contains(&column.as_str()))
            .collect();
        assert!(
            unclassified.is_empty(),
            "`songs` has {unclassified:?}, in neither list. If a person types it, add it to \
             HAND_SET_COLUMNS *and* to SongBackup. If a scan writes it, add it to DERIVED_COLUMNS."
        );

        // And the hand-set half of the schema is exactly the list, in schema order -- so removing a
        // name from HAND_SET_COLUMNS without removing the column fails too.
        let from_schema: Vec<&str> = columns
            .iter()
            .map(String::as_str)
            .filter(|column| !DERIVED_COLUMNS.contains(column))
            .collect();
        assert_eq!(
            from_schema, HAND_SET_COLUMNS,
            "the schema and the backup's field list disagree"
        );

        // The constant and the struct cannot drift either.
        let json = serde_json::to_value(SongBackup::filled_for_test()).expect("serialize");
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut expected: Vec<&str> = HAND_SET_COLUMNS
            .iter()
            .chain(NON_COLUMN_KEYS)
            .copied()
            .collect();
        keys.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            keys, expected,
            "SongBackup and HAND_SET_COLUMNS have come apart"
        );
    }

    /// The favorites tables have no detected half at all, so the assertion is simply that a backup
    /// carries every column of them.
    #[test]
    fn the_backup_carries_every_column_of_the_tables_that_are_all_hand_set() {
        let db = db();

        let favorites = db.columns_for_test("favorites").expect("read the schema");
        let carried = ["id", "name", "temporary"];
        assert_eq!(
            favorites, carried,
            "`favorites` has a column a backup says nothing about -- `id` is this database's own \
             and `name` is what stands in for it, and `temporary` is carried outright"
        );

        let membership = db.columns_for_test("song_favorites").expect("read");
        assert_eq!(
            membership,
            ["song_id", "favorite_id"],
            "`song_favorites` is the membership, and both halves are in SongBackup::favorites"
        );
    }

    #[test]
    fn a_backup_reads_back_exactly_what_was_written() {
        let scratch = Scratch::new("round-trip");
        let path = scratch.0.join("backup.json");

        let written = Backup {
            note: note(),
            format: FORMAT,
            written_at: "2026-09-02T00:00:00Z".to_owned(),
            root: Some("/tunes/karaoke".to_owned()),
            favorites: vec![FavoriteBackup {
                name: "Brasil / Bossa".to_owned(),
                temporary: false,
            }],
            songs: vec![SongBackup::filled_for_test()],
        };
        written.write(&path).expect("write");

        assert_eq!(Backup::read(&path).expect("read"), written);
    }

    /// A backup in an older format is refused with both numbers, and one with no format is format 1.
    ///
    /// The format is read before the rest, because an older shape can fail to parse as this one and
    /// the error that produced would name a field rather than the reason.
    #[test]
    fn a_backup_in_an_older_format_is_refused_by_its_number() {
        let scratch = Scratch::new("older-format");
        for (name, text) in [
            (
                "unstamped.json",
                r#"{"songs":[{"id":"abc","title":"Corcovado","favorites":[["Brasil","Bossa"]]}]}"#,
            ),
            ("one.json", r#"{"format":1,"songs":[]}"#),
        ] {
            let path = scratch.0.join(name);
            std::fs::write(&path, text).expect("write");
            let said = Backup::read(&path)
                .expect_err("an older format must not be read as this one")
                .to_string();
            assert!(said.contains("format 1"), "{name}: {said}");
            assert!(said.contains(&format!("format {FORMAT}")), "{name}: {said}");
        }
    }

    /// The other direction, which is the one this format exists to survive: a newer file is read for
    /// everything this build understands, and the fact that it is newer is *reported*.
    #[test]
    fn a_backup_written_by_a_later_build_loads_and_says_so() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        let backup: Backup = serde_json::from_str(
            r#"{"format":99,"songs":[{"id":"abc","title":"Corcovado","gramophone":true}]}"#,
        )
        .expect("an unknown key must not stop a recovery");

        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");
        assert_eq!(report.from_a_later_format, Some(99));
        assert_eq!(report.songs_applied, 1);
        assert_eq!(
            db.song("abc").expect("song").title.as_deref(),
            Some("Corcovado")
        );
    }

    /// **The load-bearing test.** Every hand-set field in a backup comes back into a database rebuilt
    /// from nothing, which is the whole reason the file exists.
    #[test]
    fn a_backup_restored_into_a_rebuilt_database_returns_every_hand_set_field() {
        let scratch = Scratch::new("rebuilt");
        let path = scratch.0.join("backup.json");

        {
            let mut db = db();
            add_song(&mut db, "aaa", "corcovado");
            add_song(&mut db, "bbb", "corcovado-again");
            db.edit_song(
                "aaa",
                &crate::db::SongEdit {
                    title: Some(Some("Corcovado".to_owned())),
                    artist: Some(Some("Tom Jobim".to_owned())),
                    language: Some(Some("pt".to_owned())),
                    lyric_encoding: Some(Some("cp1252".to_owned())),
                    default_transpose: Some(Some(-2)),
                    lyrics_hidden: Some(Some(true)),
                    notes: Some(Some("the good one".to_owned())),
                    fixes: None,
                    melody_chosen: None,
                },
            )
            .expect("edit");
            db.set_user_score("aaa", Some(9)).expect("user score");
            db.set_merged_into("bbb", Some("aaa")).expect("merge");
            let bossa = db.create_favorite("Bossa").expect("favorite");
            db.set_favorite("aaa", bossa, true).expect("file it");
            db.create_favorite("Empty").expect("an empty one");

            write(&db, &path).expect("write the backup");
        }

        // A brand-new database over the same two songs -- which is what a re-scan after losing the
        // index leaves behind.
        let mut db = db();
        add_song(&mut db, "aaa", "corcovado");
        add_song(&mut db, "bbb", "corcovado-again");

        let backup = Backup::read(&path).expect("read");
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        assert!(report.songs_unmatched.is_empty());

        let song = db.song("aaa").expect("song");
        assert_eq!(song.title.as_deref(), Some("Corcovado"));
        assert_eq!(song.artist.as_deref(), Some("Tom Jobim"));
        assert_eq!(song.language.as_deref(), Some("pt"));
        assert_eq!(song.lyric_encoding.as_deref(), Some("cp1252"));
        assert_eq!(song.default_transpose, Some(-2));
        assert_eq!(song.user_score, Some(9));
        assert_eq!(song.notes.as_deref(), Some("the good one"));

        let filed: Vec<String> = db
            .favorites_for("aaa")
            .expect("favorites")
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(filed, ["Bossa"], "the list it was filed under came back");
        let lists: Vec<String> = db
            .favorites()
            .expect("favorites")
            .into_iter()
            .map(|node| node.name)
            .collect();
        assert!(
            lists.contains(&"Empty".to_owned()),
            "a favorite with nothing in it is still a favorite somebody made: {lists:?}"
        );
        assert_eq!(report.merges_applied, 1);
        assert_eq!(
            db.song("bbb").expect("song").merged_into.as_deref(),
            Some("aaa")
        );
    }

    #[test]
    fn a_song_the_corpus_no_longer_has_is_listed_rather_than_invented() {
        let mut db = db();
        let before = db.counts().expect("counts").songs;

        let backup = Backup {
            songs: vec![SongBackup {
                id: "gone".to_owned(),
                title: Some("Corcovado".to_owned()),
                seen_as: Some("Corcovado".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        assert_eq!(
            report.songs_unmatched,
            [("gone".to_owned(), "Corcovado".to_owned())]
        );
        assert_eq!(report.songs_applied, 0);
        assert!(db.song("gone").is_err(), "nothing was invented");
        assert_eq!(db.counts().expect("counts").songs, before);
    }

    /// The property `build::import` argues for, on this path: one bad value must not take the good
    /// one beside it down with the statement.
    #[test]
    fn a_language_this_build_cannot_read_is_reported_and_the_title_beside_it_still_lands() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                title: Some("Corcovado".to_owned()),
                language: Some("ENGL".to_owned()),
                seen_as: Some("Corcovado".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        assert_eq!(report.rejected.len(), 1, "{:?}", report.rejected);
        assert!(report.rejected[0].contains("ENGL"), "{:?}", report.rejected);
        let song = db.song("abc").expect("song");
        assert_eq!(song.title.as_deref(), Some("Corcovado"));
        assert_eq!(song.language, None);
    }

    #[test]
    fn a_rating_above_ten_is_refused_without_stopping_the_restore() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                title: Some("Corcovado".to_owned()),
                user_score: Some(99),
                seen_as: Some("Corcovado".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        assert_eq!(report.rejected.len(), 1, "{:?}", report.rejected);
        let song = db.song("abc").expect("song");
        assert_eq!(song.title.as_deref(), Some("Corcovado"));
        assert_eq!(song.user_score, None, "the rating beside it is refused");
    }

    #[test]
    fn a_transposition_that_will_not_fit_is_reported_rather_than_clamped() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                default_transpose: Some(300),
                seen_as: Some("Corcovado".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        assert_eq!(report.rejected.len(), 1, "{:?}", report.rejected);
        assert!(report.rejected[0].contains("300"), "{:?}", report.rejected);
        assert_eq!(
            db.song("abc").expect("song").default_transpose,
            None,
            "127 would have been a number nobody typed"
        );
    }

    #[test]
    fn filling_blanks_never_writes_over_what_is_already_there() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");
        db.edit_song(
            "abc",
            &crate::db::SongEdit {
                title: Some(Some("What I typed today".to_owned())),
                ..crate::db::SongEdit::default()
            },
        )
        .expect("edit");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                title: Some("What the backup says".to_owned()),
                artist: Some("Tom Jobim".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        let song = db.song("abc").expect("song");
        assert_eq!(song.title.as_deref(), Some("What I typed today"));
        assert_eq!(
            song.artist.as_deref(),
            Some("Tom Jobim"),
            "the empty field beside it was still filled -- a song is nine decisions, not one"
        );
    }

    /// Writes an artist a person is taken to have typed, blank included.
    fn set_artist(db: &mut crate::db::Db, id: &str, artist: &str) {
        db.edit_song(
            id,
            &crate::db::SongEdit {
                artist: Some(Some(artist.to_owned())),
                ..crate::db::SongEdit::default()
            },
        )
        .expect("edit");
    }

    /// An artist recorded as blank is a decision, and both directions treat it as one.
    ///
    /// `artist` is the one hand-set column where `''` and NULL differ, and *Title from file name*
    /// writes the empty one over every row it touches. So it has to survive the file — `serde` keeps
    /// it, since only `None` is skipped — and it has to be read back as a value by the `coalesce` on
    /// either side of the switch, rather than as the blank neither direction will write.
    #[test]
    fn an_artist_recorded_as_blank_travels_as_a_value_and_not_as_a_gap() {
        let blank = SongBackup {
            id: "abc".to_owned(),
            artist: Some(String::new()),
            ..SongBackup::default()
        };
        let json = serde_json::to_value(&blank).expect("serialize");
        assert_eq!(
            json.get("artist").and_then(serde_json::Value::as_str),
            Some(""),
            "a blank artist is written down, where an absent one is left out"
        );

        // Filling blanks leaves one that is already there alone.
        {
            let mut filling = db();
            add_song(&mut filling, "abc", "corcovado");
            set_artist(&mut filling, "abc", "");
            let backup = Backup {
                songs: vec![SongBackup {
                    id: "abc".to_owned(),
                    artist: Some("Tom Jobim".to_owned()),
                    ..SongBackup::default()
                }],
                ..Backup::default()
            };
            restore(&mut filling, &backup, Policy::FillBlanks).expect("restore");
            assert_eq!(
                filling.song("abc").expect("song").artist.as_deref(),
                Some(""),
                "a decision that nobody performed it is work, and filling blanks takes none away"
            );
        }

        // And overwriting writes the blank the file records.
        {
            let mut overwriting = db();
            add_song(&mut overwriting, "abc", "corcovado");
            set_artist(&mut overwriting, "abc", "Tom Jobim");
            restore(
                &mut overwriting,
                &Backup {
                    songs: vec![blank],
                    ..Backup::default()
                },
                Policy::Overwrite,
            )
            .expect("restore");
            assert_eq!(
                overwriting.song("abc").expect("song").artist.as_deref(),
                Some("")
            );
        }
    }

    #[test]
    fn overwriting_writes_over_it_and_still_never_blanks_a_field() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");
        db.edit_song(
            "abc",
            &crate::db::SongEdit {
                title: Some(Some("What I typed today".to_owned())),
                notes: Some(Some("keep me".to_owned())),
                ..crate::db::SongEdit::default()
            },
        )
        .expect("edit");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                title: Some("What the backup says".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        restore(&mut db, &backup, Policy::Overwrite).expect("restore");

        let song = db.song("abc").expect("song");
        assert_eq!(song.title.as_deref(), Some("What the backup says"));
        assert_eq!(
            song.notes.as_deref(),
            Some("keep me"),
            "an absent key says nothing about a field, under either policy"
        );
    }

    /// The whole argument for a path being an array of segments, as a test.
    #[test]
    fn a_favorite_whose_name_contains_a_slash_survives_the_round_trip() {
        let scratch = Scratch::new("slash");
        let path = scratch.0.join("backup.json");

        {
            let mut db = db();
            add_song(&mut db, "abc", "corcovado");
            let rock = db.create_favorite("Rock/Pop").expect("favorite");
            db.set_favorite("abc", rock, true).expect("file it");
            write(&db, &path).expect("write");
        }

        let mut db = db();
        add_song(&mut db, "abc", "corcovado");
        let backup = Backup::read(&path).expect("read");
        restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        let filed: Vec<String> = db
            .favorites_for("abc")
            .expect("favorites")
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(filed, ["Rock/Pop"], "one favorite, not two");
    }

    /// What kind of list a favorite is survives a round trip, and a new one is made as what it was.
    #[test]
    fn a_working_list_comes_back_a_working_list() {
        let mut curated = db();
        add_song(&mut curated, "abc", "corcovado");
        let to_check = curated.create_favorite("to-check").expect("favorite");
        curated.create_favorite("Bossa").expect("favorite");
        curated.set_favorite_temporary(to_check, true).expect("set");

        let written = backup_of(&curated).expect("read");
        let kinds: Vec<(String, bool)> = written
            .favorites
            .iter()
            .map(|favorite| (favorite.name(), favorite.temporary))
            .collect();
        assert_eq!(
            kinds,
            [("Bossa".to_owned(), false), ("to-check".to_owned(), true)]
        );

        let mut fresh = db();
        add_song(&mut fresh, "abc", "corcovado");
        restore(&mut fresh, &written, Policy::FillBlanks).expect("restore");
        let made: Vec<(String, bool)> = fresh
            .favorites()
            .expect("favorites")
            .into_iter()
            .map(|node| (node.name, node.temporary))
            .collect();
        assert_eq!(
            made,
            [("Bossa".to_owned(), false), ("to-check".to_owned(), true)]
        );
    }

    /// **A restore that is not overwriting says nothing about a list somebody already has.**
    ///
    /// An empty sort key is a fact nobody has supplied, so filling one in takes nothing away.
    /// `temporary` has no such value — false is a decision as much as true is — so only the policy
    /// that lets the file win may change one.
    #[test]
    fn a_restore_only_changes_what_kind_of_list_an_existing_favorite_is_when_the_file_wins() {
        let backup = Backup {
            favorites: vec![FavoriteBackup {
                name: "to-check".to_owned(),
                temporary: true,
            }],
            ..Backup::default()
        };

        let mut keeps = db();
        keeps.create_favorite("to-check").expect("favorite");
        restore(&mut keeps, &backup, Policy::FillBlanks).expect("restore");
        assert!(
            !keeps.favorites().expect("favorites")[0].temporary,
            "a filing here stays a filing"
        );

        let mut overwrites = db();
        overwrites.create_favorite("to-check").expect("favorite");
        restore(&mut overwrites, &backup, Policy::Overwrite).expect("restore");
        assert!(overwrites.favorites().expect("favorites")[0].temporary);
    }

    #[test]
    fn restoring_the_same_file_twice_changes_nothing_the_second_time() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        let backup = Backup {
            favorites: vec![FavoriteBackup {
                name: "Brasil / Bossa".to_owned(),
                temporary: false,
            }],
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                title: Some("Corcovado".to_owned()),
                favorites: vec![FavoriteRef::Name("Brasil / Bossa".to_owned())],
                ..SongBackup::default()
            }],
            ..Backup::default()
        };

        let first = restore(&mut db, &backup, Policy::FillBlanks).expect("first");
        assert_eq!(first.favorites_created, 1);
        let second = restore(&mut db, &backup, Policy::FillBlanks).expect("second");
        assert_eq!(
            second.favorites_created, 0,
            "the list was already there and must not be doubled"
        );

        assert_eq!(db.favorites().expect("favorites").len(), 1);
        assert_eq!(db.favorites_for("abc").expect("filed").len(), 1);
    }

    /// The transactionality test. A membership that will not insert must leave the title alone.
    #[test]
    fn a_restore_that_fails_partway_leaves_the_database_exactly_as_it_was() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        // Hand-built rather than planned: `plan` filters an unknown song out, which is exactly its
        // job, so the only way to reach the transaction with a statement that cannot go through is
        // to build the plan directly. `song_favorites.song_id` has a foreign key to `songs`.
        let plan = Plan {
            overwrite: false,
            favorites: vec![FavoriteBackup {
                name: "Brasil".to_owned(),
                ..FavoriteBackup::default()
            }],
            songs: vec![PlannedSong {
                id: "abc".to_owned(),
                title: Some("Corcovado".to_owned()),
                artist: None,
                language: None,
                lyric_encoding: None,
                default_transpose: None,
                lyrics_hidden: None,
                fixes: None,
                melody_chosen: None,
                user_score: None,
                notes: None,
            }],
            merges: Vec::new(),
            memberships: vec![("nobody".to_owned(), "Brasil".to_owned())],
        };

        assert!(
            db.apply_restore(&plan).is_err(),
            "a membership for a song that is not here must not go through"
        );
        assert_eq!(
            db.song("abc").expect("song").title,
            None,
            "the title earlier in the same restore went back with it"
        );
        assert!(
            db.favorites().expect("favorites").is_empty(),
            "and so did the favorite"
        );
    }

    #[test]
    fn a_merge_that_would_chain_is_refused_rather_than_recorded() {
        let mut db = db();
        add_song(&mut db, "aaa", "one");
        add_song(&mut db, "bbb", "two");
        add_song(&mut db, "ccc", "three");
        db.set_merged_into("bbb", Some("aaa")).expect("merge");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "ccc".to_owned(),
                merged_into: Some("bbb".to_owned()),
                seen_as: Some("Three".to_owned()),
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        assert_eq!(report.merges_applied, 0);
        assert_eq!(report.rejected.len(), 1, "{:?}", report.rejected);
        assert_eq!(db.song("ccc").expect("song").merged_into, None);
    }

    /// **The default name carries the moment, and keeps the suffix two other things read.**
    ///
    /// The suffix half is the load-bearing one: the scan skips these files by extension, and
    /// `db::database_in` refuses a folder holding two `.kmbuild`-shaped files, so a stem-side
    /// stamp is the only place a name may grow.
    #[test]
    fn the_default_backup_name_carries_the_moment_and_keeps_its_suffix() {
        let name = default_path(Path::new("/corpus"))
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();

        let stamp = name
            .strip_prefix("km-package-builder-")
            .and_then(|rest| rest.strip_suffix(".kmbackup.json"))
            .unwrap_or_else(|| panic!("{name} is not a dated backup name"));

        assert_eq!(stamp.len(), 16, "YYYYMMDDThhmmssZ, got {stamp}");
        assert!(
            !stamp.contains(':') && !stamp.contains('-'),
            "{stamp} carries a character a Windows file name may not"
        );
        assert!(
            stamp.ends_with('Z') && stamp[..8].chars().all(|c| c.is_ascii_digit()),
            "{stamp} is not the UTC stamp the rest of this workspace writes"
        );
    }

    /// **Name order is time order, which is what lets the page suggest one.**
    #[test]
    fn the_newest_backup_is_the_one_the_restore_box_suggests() {
        let scratch = Scratch::new("newest-backup");
        let data = crate::db::data_dir(&scratch.0);
        std::fs::create_dir_all(&data).expect("the data folder");

        assert_eq!(newest(&scratch.0), None, "nothing taken yet");

        for stamp in ["20260910T090000Z", "20260909T140233Z", "20260909T235959Z"] {
            std::fs::write(
                data.join(format!("km-package-builder-{stamp}.kmbackup.json")),
                b"{}",
            )
            .expect("a backup");
        }
        // Neither a backup nor dated, so neither may be suggested as the newest one.
        std::fs::write(data.join("km-package-builder-notes.txt"), b"x").expect("a stray file");
        std::fs::write(data.join("kept.kmbackup.json"), b"{}").expect("a hand-named backup");

        assert_eq!(
            newest(&scratch.0).as_deref(),
            Some("km-package-builder-20260910T090000Z.kmbackup.json")
        );
    }

    /// A corpus whose data folder was never made is not an error, it is nothing to suggest.
    #[test]
    fn a_corpus_with_no_data_folder_suggests_no_backup() {
        let scratch = Scratch::new("newest-backup-absent");
        assert_eq!(newest(&scratch.0), None);
    }

    #[test]
    fn a_backup_leaves_no_half_written_file_behind() {
        let scratch = Scratch::new("atomic");
        let path = scratch.0.join("backup.json");

        Backup {
            format: FORMAT,
            ..Backup::default()
        }
        .write(&path)
        .expect("first");
        let bigger = Backup {
            format: FORMAT,
            songs: vec![SongBackup::filled_for_test()],
            ..Backup::default()
        };
        bigger.write(&path).expect("second");

        assert_eq!(Backup::read(&path).expect("read"), bigger);
        assert!(
            !scratch.0.join("backup.json.writing").exists(),
            "the scratch file is renamed away, never left beside the real one"
        );
    }

    #[test]
    fn a_favorite_with_no_name_left_in_it_is_dropped_rather_than_refused() {
        let mut db = db();
        add_song(&mut db, "abc", "corcovado");

        let backup = Backup {
            songs: vec![SongBackup {
                id: "abc".to_owned(),
                favorites: vec![
                    FavoriteRef::Name("  ".to_owned()),
                    FavoriteRef::Name("Brasil".to_owned()),
                ],
                ..SongBackup::default()
            }],
            ..Backup::default()
        };
        let report = restore(&mut db, &backup, Policy::FillBlanks).expect("restore");

        assert_eq!(report.favorites_created, 1);
        assert_eq!(report.memberships_applied, 1);
    }
}
