//! The shapes a query hands back, and the one it takes bindings in.
//!
//! Rows and reports rather than behaviour: `SongDetail`, `MidiDetail`, `SongFile`, `Counts`,
//! `FolderNode`, `RestoreOutcome`, `LyricSearch`, plus `Binding`, which is how a value reaches
//! SQLite when the statement is built at run time.
//!
//! Split out of `db.rs` because they change for a different reason than the queries do — a column
//! added to the browse table changes a struct here and a `SELECT` there, and the two were sharing an
//! 8,835-line file with the migrations and the filter vocabulary. Everything is re-exported from
//! `crate::db`, so no path outside this module changed.

use super::*;

/// What [`Db::apply_restore`] wrote.
///
/// Counts only, and one list of ids — everything a person reads is assembled in `crate::backup`,
/// which is the half that has the file in hand and can put a name to a hash.
#[derive(Debug, Clone, Default)]
pub struct RestoreOutcome {
    /// Songs whose row was written.
    pub songs_applied: usize,
    /// Favorites that had to be created.
    pub favorites_created: usize,
    /// Songs filed into a favorite.
    pub memberships_applied: usize,
    /// Merges recorded.
    pub merges_applied: usize,
    /// Songs whose merge was refused because it would have chained, or because the policy left an
    /// existing merge alone.
    pub merges_refused: Vec<String>,
}

/// Headline counts for the status bar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    /// Distinct recordings, not counting merged ones.
    pub songs: u32,
    /// Files on disk.
    pub files: u32,
    /// Files that did not parse.
    pub failed: u32,
    /// Songs marked as favorites.
    pub favorites: u32,
    /// Packages being curated.
    pub packages: u32,
}

/// One reason files did not parse, counted, as the scan page lists it.
#[derive(Debug, Clone)]
pub struct FailureTally {
    /// The stored `scan_status`, which is what a Remove or a Restore names.
    pub status: String,
    /// Which sentence the Reason column writes, as a catalog key.
    ///
    /// **A key rather than the sentence**, because a tally is read inside a `query_map` closure and
    /// a database module has no language in reach. `handlers::failures_panel` is where it becomes
    /// words, in the language the page is being drawn in.
    pub reason: String,
    /// How many files failed this way.
    pub count: u32,
    /// One of them, so the reason has something concrete beside it.
    pub example: String,
}

/// One folder under the corpus root, as the Folders page shows it.
#[derive(Debug, Clone)]
pub struct FolderNode {
    /// The folder's own name, empty for the "files here" bucket.
    pub name: String,
    /// Path from the root, ending in `/`. This is what the browse filter takes.
    pub path: String,
    /// Distinct songs anywhere beneath it.
    pub song_count: u32,
}

impl FolderNode {
    /// Whether this is the bucket for files sitting directly in the folder being listed.
    pub fn is_files_here(&self) -> bool {
        self.name.is_empty()
    }
}

/// The MIDI half of a song's page.
///
/// The scanner's [`MidiFacts`](crate::model::MidiFacts) plus the suitability broken into its four components, which the song page
/// shows and the browse row does not.
#[derive(Debug, Clone)]
pub struct MidiDetail {
    /// Karaoke convention.
    pub flavor: String,
    /// Lyric granularity.
    pub granularity: String,
    /// Notes in the file.
    pub note_count: u32,
    /// Sounding channels.
    pub channel_count: u32,
    /// Lyric lines.
    pub line_count: u32,
    /// Lyric syllables.
    pub syllable_count: u32,
    /// The encoding detection chose.
    pub det_encoding: String,
    /// How it chose it.
    pub det_encoding_source: String,
    /// Melody channel, when found.
    pub melody_channel: Option<u8>,
    /// Confidence in it.
    pub melody_confidence: Option<f32>,
    /// Why none was claimed.
    pub melody_abstained: Option<String>,
    /// Suitability out of 10.
    pub suitability: u8,
    /// Lyrics component.
    pub suitability_lyrics: u8,
    /// Sync component.
    pub suitability_sync: u8,
    /// Channel-separation component.
    pub suitability_channels: u8,
    /// Arrangement component.
    pub suitability_arrangement: u8,
    /// Warnings, as JSON.
    pub warnings: String,
}

impl MidiDetail {
    /// The suitability as the optional the shared color helper takes.
    ///
    /// A MIDI song always has one, so this is `Some` every time — it exists only so the song page
    /// and the browse row color it through the same function rather than two that could come
    /// to disagree about where `mid` ends and `high` begins.
    pub fn suitability_opt(&self) -> Option<u8> {
        Some(self.suitability)
    }
}

/// One song, in full.
#[derive(Debug, Clone)]
pub struct SongDetail {
    /// Content hash.
    pub id: String,
    /// Title the file gave.
    pub det_title: Option<String>,
    /// Performer the file gave.
    pub det_artist: Option<String>,
    /// Language the file gave, verbatim — `ENGL`, not `en`. What the edit form's hint shows.
    pub det_language: Option<String>,
    /// The code that and the lyric encoding between them imply, if either implied anything.
    pub det_language_tag: Option<String>,
    /// The code the song's own words read as, where they read confidently as anything.
    pub det_language_guess: Option<String>,
    /// How sure that reading was, between zero and one, and `None` exactly when there is none.
    pub det_language_guess_confidence: Option<f64>,
    /// Title a person typed.
    pub title: Option<String>,
    /// Performer a person typed.
    pub artist: Option<String>,
    /// Language a person typed.
    pub language: Option<String>,
    /// Encoding a person pinned.
    pub lyric_encoding: Option<String>,
    /// Transposition a person chose.
    pub default_transpose: Option<i64>,
    /// Whether somebody said to play the song and draw none of its words.
    ///
    /// Three states: `None` is nobody has said and the analysis stands, `Some(true)` silences the
    /// words, and `Some(false)` draws them on a file the analysis would have silenced.
    pub lyrics_hidden: Option<bool>,
    /// The corrections a person decided on, as stored JSON. `None` means nobody has said.
    ///
    /// Held as text rather than parsed, because the page shows it beside what detection proposes
    /// and the two are compared as lists; parsing happens once, in the view that draws them.
    pub fixes: Option<String>,
    /// The melody channel a person named, as stored. `None` means nobody has said, in which case
    /// the detected `melody_channel` stands; see `crate::fixes::MelodyChoice`.
    pub melody_chosen: Option<String>,
    /// What kind of song this is.
    pub kind: SongKind,
    /// Length in milliseconds. Both kinds of song have one.
    pub duration_ms: u32,
    /// Everything only a MIDI file has: the flavor, the counts, the encoding and the suitability.
    ///
    /// One `Option` around thirteen fields that arrive and are absent together, rather than thirteen
    /// separate ones — see [`MidiFacts`](crate::model::MidiFacts).
    pub midi: Option<MidiDetail>,
    /// Everything only a video has, straight from its probe.
    pub video: Option<VideoFacts>,
    /// Everything only an MP3+G pair has, from probing both of its files.
    pub cdg: Option<CdgFacts>,
    /// The person's own rating.
    pub user_score: Option<u8>,
    /// Free-text notes.
    pub notes: Option<String>,
    /// The song this was merged into, when somebody said they are the same.
    pub merged_into: Option<String>,
    /// The better-reading file this one was set aside behind, or NULL when it was not.
    ///
    /// Never [`Self::merged_into`]: that is what a person decided, this is what the tool worked
    /// out, and the page says which is which because only one of them is anybody's word.
    pub duplicate_of: Option<String>,
    /// The file's own name without its extension, the last-resort title. NULL only for a song whose
    /// every copy has gone from disk.
    pub stem: Option<String>,
    /// When a scan first found the song, as `YYYY-MM-DDTHH:MM:SSZ`. A later scan does not move it.
    pub first_seen: String,
    /// Every copy on disk.
    pub files: Vec<SongFile>,
    /// The favorites it is in, id and name.
    pub favorites: Vec<(i64, String)>,
    /// Packages it belongs to, with the volume and the number it has in each.
    pub packages: Vec<(String, u32, u32)>,
}

impl SongDetail {
    /// The day the song was added, `YYYY-MM-DD`, in UTC like the stamp it is cut from.
    pub fn added_on(&self) -> &str {
        self.first_seen.get(..10).unwrap_or(&self.first_seen)
    }

    /// The title to show: what a person typed, else what the file said, else the file's own name.
    ///
    /// The Rust twin of [`eff_title`]; the two must agree, or the heading of a song's page would not
    /// match the row that was clicked to reach it.
    pub fn effective_title(&self) -> String {
        self.title
            .clone()
            .filter(|value| !value.is_empty())
            .or_else(|| self.det_title.clone().filter(|value| !value.is_empty()))
            .or_else(|| self.stem.clone())
            .unwrap_or_default()
    }

    /// Whether the title being shown is only the file's name.
    pub fn title_is_filename(&self) -> bool {
        self.title.as_deref().unwrap_or_default().is_empty()
            && self.det_title.as_deref().unwrap_or_default().is_empty()
    }

    /// The performer to show.
    pub fn effective_artist(&self) -> Option<String> {
        self.artist.clone().or_else(|| self.det_artist.clone())
    }

    /// Where the ≈ button goes: songs whose name is like this one's, or empty when it has none.
    pub fn similar_url(&self) -> String {
        crate::model::similar_url(
            &self.effective_title(),
            &self.effective_artist().unwrap_or_default(),
            &self.id,
        )
    }

    /// Where the ≋ button goes: songs that sing what this one sings, or empty when it has no words.
    ///
    /// The line count and not the words themselves: a page carries them only for a MIDI song, and a
    /// song with a lyric line is a song with something in the `lyrics` column, which is what decides
    /// the same thing on a browse row.
    pub fn words_url(&self) -> String {
        if self.midi.as_ref().is_some_and(|midi| midi.line_count > 0) {
            crate::model::words_url(&self.id)
        } else {
            String::new()
        }
    }

    /// Whether the language being shown was worked out rather than chosen.
    ///
    /// What lets the song page mark a detected value as *detected*: after this change most rows have
    /// one, and a good many of those came from a header that says English whatever the song is.
    pub fn language_is_detected(&self) -> bool {
        self.language.as_deref().unwrap_or_default().is_empty() && self.det_language_tag.is_some()
    }

    /// The English name of the detected language, for the hint under the picker.
    pub fn detected_language_name(&self) -> &'static str {
        self.det_language_tag
            .as_deref()
            .and_then(Language::parse)
            .map_or("", Language::name)
    }

    /// Whether the language being shown was read out of the song's words.
    ///
    /// Distinct from [`Self::language_is_detected`] because the two are different claims: a file
    /// said `ENGL`, or a detector read the words and was sure. Only the last leg of the coalesce
    /// counts here, so a song whose file spoke is *detected* however its words read.
    pub fn language_is_guessed(&self) -> bool {
        self.language.as_deref().unwrap_or_default().is_empty()
            && self.det_language_tag.is_none()
            && self.det_language_guess.is_some()
    }

    /// The English name of the language the words read as, for the hint under the picker.
    pub fn guessed_language_name(&self) -> &'static str {
        self.det_language_guess
            .as_deref()
            .and_then(Language::parse)
            .map_or("", Language::name)
    }

    /// How sure the reading was, as whole percent, for the hint beside the name.
    ///
    /// Rounded for reading rather than for arithmetic: the number is there to tell a close call
    /// from a certainty, and two decimal places of a detector's confidence say no more than one.
    pub fn guessed_language_percent(&self) -> u8 {
        let confidence = self.det_language_guess_confidence.unwrap_or_default();
        let percent = (confidence * 100.0).round();
        if percent <= 0.0 {
            0
        } else if percent >= 100.0 {
            100
        } else {
            percent as u8
        }
    }

    /// Whether the file's own header is the Soft Karaoke default rather than a statement.
    ///
    /// `ENGL` is what the editor writes unless somebody changes it, so it appears on Portuguese and
    /// Italian songs alike — in this corpus's own `Brasil/` folder, 35 times out of 36. It is taken
    /// anyway, because discarding it leaves most of a corpus with no language at all; saying so on
    /// the page is what stops a curator reading it as a fact.
    pub fn declaration_is_default(&self) -> bool {
        self.det_language
            .as_deref()
            .and_then(Language::from_declared)
            .is_some_and(|language| language.code() == "en")
    }

    /// Length as `m:ss`.
    pub fn duration(&self) -> String {
        crate::model::format_duration(self.duration_ms)
    }

    /// Whether this song is in a favorite.
    ///
    /// A method rather than a closure in the template: askama has no closures, and the check has to
    /// happen once per favorite per song.
    pub fn has_favorite(&self, id: &i64) -> bool {
        self.favorites.iter().any(|(existing, _)| existing == id)
    }

    /// The warnings, parsed. An unreadable column yields none rather than an error — a warning list
    /// is not worth failing a page render over.
    ///
    /// A video has none: nothing analyzes it, so there is nothing to have gone wrong.
    pub fn parsed_warnings(&self) -> Vec<StoredWarning> {
        self.midi
            .as_ref()
            .and_then(|midi| serde_json::from_str(&midi.warnings).ok())
            .unwrap_or_default()
    }

    /// What the analysis concludes about drawing this song's words, where nobody has said.
    ///
    /// Read off the warnings this row already carries rather than measured again, which is what
    /// makes the automatic half cost no rescan: the three faults that answer it were named when the
    /// file was scanned. An UltraStar song has no analysis, so the answer is always to draw them.
    pub fn words_cannot_be_followed(&self) -> bool {
        let warnings = self.parsed_warnings();
        km_pack::warnings_hide_words(warnings.iter().map(|warning| warning.code.as_str()))
    }

    /// Whether the encoding was the CP1252 fallback, so the text may be wrong.
    ///
    /// False for a video, which has no text of ours to have decoded wrongly.
    pub fn encoding_guessed(&self) -> bool {
        self.midi
            .as_ref()
            .is_some_and(|midi| midi.det_encoding_source == "fallback")
    }

    /// The encoding a re-decode dropdown should start on, if the question applies at all.
    pub fn detected_encoding(&self) -> Option<&str> {
        self.midi.as_ref().map(|midi| midi.det_encoding.as_str())
    }
}

/// One copy of a song on disk.
///
/// Every file here parsed: only a file that became a song carries a `song_id`, so a status column
/// would read `ok` on every row.
#[derive(Debug, Clone)]
pub struct SongFile {
    /// Path relative to the root.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
}

impl SongFile {
    /// The folder this copy sits in, ending in `/`. Empty for a file at the root.
    pub fn folder(&self) -> &str {
        parent_folder(&self.path)
    }

    /// The song list filtered to this copy's folder, subfolders included.
    ///
    /// The same link the Folders page's *only this folder* button follows, so the two agree about
    /// what "this folder" means. Empty for a file at the root, where the filter would be no filter
    /// at all and the link would quietly mean *every song*; the template shows no link then.
    pub fn folder_url(&self) -> String {
        match self.folder() {
            "" => String::new(),
            folder => format!("/songs?folder={}", crate::form::encode(folder)),
        }
    }
}

/// A correction to apply. `None` leaves a field alone; `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct SongEdit {
    /// New title.
    pub title: Option<Option<String>>,
    /// New performer.
    pub artist: Option<Option<String>>,
    /// New language tag.
    pub language: Option<Option<String>>,
    /// New lyric encoding.
    pub lyric_encoding: Option<Option<String>>,
    /// New default transposition.
    pub default_transpose: Option<Option<i8>>,
    /// Whether to draw the song's words. `Some(None)` hands the song back to the analysis.
    ///
    /// **`Some(Some(false))` is not the same as `Some(None)`**, which is the whole reason this
    /// field nests two options over a type whose own range is two. Clearing it lets the analysis
    /// speak again; writing a false overrules it. A song whose words the analysis would silence
    /// reads those two answers as opposite ones.
    pub lyrics_hidden: Option<Option<bool>>,
    /// The corrections to record, as JSON. `Some(None)` hands the song back to detection.
    pub fixes: Option<Option<String>>,
    /// The melody channel to record. `Some(None)` hands that question back to detection too.
    pub melody_chosen: Option<Option<String>>,
    /// New notes.
    pub notes: Option<Option<String>>,
}

/// A value bound into a query.
///
/// Filters are composed as SQL fragments plus a list of these, so nothing a person typed is ever
/// interpolated into the statement text.
#[derive(Debug, Clone)]
pub enum Binding {
    /// A number.
    Integer(i64),
    /// Text.
    Text(String),
    /// SQL NULL.
    Null,
}

impl rusqlite::ToSql for Binding {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(match self {
            Self::Integer(value) => (*value).into(),
            Self::Text(value) => value.as_str().into(),
            Self::Null => rusqlite::types::Null.into(),
        })
    }
}

/// How many tokens of lyric FTS5 puts either side of a match in a passage.
///
/// Wide enough to carry the whole line a phrase sits in, which is the unit somebody half-remembers,
/// and short enough that a page of hits is still scannable. FTS5 refuses anything above 64.
pub(super) const SNIPPET_TOKENS: u32 = 16;

/// A search of the words themselves.
///
/// Deliberately not a variant of [`Filter`]. The browse page's filters describe a song — its suitability,
/// its folder, whether anyone has starred it — and are combined freely; this asks one question of one
/// index and orders by how well it was answered. Folding it in would have meant either a second sort
/// mode that is meaningless whenever the box is empty, or lyrics joining `songs_fts` and quietly
/// changing what the title box finds.
#[derive(Debug, Clone)]
pub struct LyricSearch {
    /// The words as they were typed.
    pub query: String,
    /// Rows per page.
    pub limit: u32,
    /// Rows to skip.
    pub offset: u32,
}

impl LyricSearch {
    /// The sanitised FTS5 expression, or `None` when nothing was typed.
    ///
    /// `None` rather than an expression matching nothing: an empty box is not a search that found
    /// nothing, it is no search at all, and the page says so instead of showing an empty table.
    pub(super) fn match_query(&self) -> Option<String> {
        (!self.query.trim().is_empty()).then(|| fts_match_query(&self.query))
    }
}

/// What adding songs to a package did.
///
/// **Every song asked for lands in exactly one of the first three counts**, so a caller says what
/// happened instead of working it out. A shortfall taken from the number asked for cannot tell a song
/// the package already held from one it had no number for, and those send somebody to two different
/// places: the first to a package that is doing its job, the second to Re-flow.
///
/// [`Self::clashed`] is a **warning and never a refusal**: `km_kmpkg` refuses two entries with the
/// same bytes, which is a catalog defect, but two files of one recording is a judgment — an acoustic
/// take and a full arrangement are one recording to a fingerprint and two songs to a singer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Added {
    /// Songs numbered into the package.
    pub added: u32,
    /// Songs the package already held. Skipped rather than refused, because adding a selection that
    /// overlaps what is already there is a normal thing to do.
    pub already: u32,
    /// Songs left out because the numbering reached the last slot a singer can dial.
    pub no_room: u32,
    /// Of those added, how many joined a package that already held another file of the same recording.
    pub clashed: u32,
    /// Whether the package holds every song it can, which is what picks the remedy for
    /// [`Self::no_room`]: numbers that ran to the end from a high first number are re-flowed, and a
    /// package holding every slot is full.
    pub full: bool,
}

/// What a sync of a sourced package did.
///
/// **It holds an [`Added`] rather than flattening one**, so the outcomes a placement has are counted
/// by the type that counts them and one sentence is built from both. `already` is 0 by construction
/// — a sync places only the songs the package does not hold — and `clashed` means here exactly what
/// it means there.
///
/// What a sync adds beside them is the half a hand add has no word for: entries that left because no
/// source names them any more, and entries the union already held, which keep the numbers they had.
/// Nothing here is a subtraction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    /// What went in, and what did not and why.
    pub placed: Added,
    /// Entries taken out because no source names them any more.
    pub removed: u32,
    /// Entries the union already held, which kept the numbers they had.
    pub kept: u32,
    /// Volumes the sync started, because every volume it had was full.
    pub new_volumes: u32,
}

/// One song put in another's place in a package: the slot, the two songs, and the lists that follow.
///
/// **The same value answers the question and reports the write**, because both read it through one
/// function inside the transaction that acts on it. A confirmation naming one song over a write that
/// replaced another is a number nobody can check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// The volume's name as a build writes it: the package's name, numbered once it has two.
    pub volume_name: String,
    /// The number both songs are dialled by, one after the other.
    pub number: u32,
    /// The song leaving the slot, as `(id, title, artist)`.
    pub old: (String, String, Option<String>),
    /// The song taking it, as `(id, title, artist)`.
    pub new: (String, String, Option<String>),
    /// The names of the lists this package follows that hold the song leaving. Empty for a package
    /// that follows no list.
    pub favorites: Vec<String>,
}

/// Why a song cannot take a slot in a package.
///
/// **Values rather than sentences**, because the database has no language to word them in and the
/// handler does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceRefusal {
    /// No song holds that number in the volume of this name.
    Empty(String),
    /// The song asked for already holds the slot.
    SameSong,
    /// The song asked for is already in the package, at this volume name and number.
    AlreadyIn(String, u32),
    /// The song asked for was merged into another, which every query shows in its place.
    Merged,
}

/// What a sync *would* do, for the question put before it is done.
///
/// **Counted through the SQL the write runs**, which is the rule every set-wide action in this tool
/// already follows: the number somebody reads and the set that is written are one clause, so they
/// cannot drift apart between the question and the answer.
#[derive(Debug, Clone)]
pub struct SyncPlan {
    /// The lists the union is read from — id, name, and whether the list is a working one.
    pub sources: Vec<(i64, String, bool)>,
    /// Songs that would be numbered in.
    pub would_add: u32,
    /// Songs that would be taken out.
    pub would_remove: u32,
    /// Songs in both, which would keep the numbers they have.
    pub kept: u32,
    /// Volumes the sync would start, because every volume together has fewer free numbers after the
    /// removals than [`Self::would_add`].
    pub new_volumes: u32,
}

impl SyncPlan {
    /// Whether pressing Sync would write nothing.
    ///
    /// A sync that changes nothing is an ordinary thing to press, and what it earns is a sentence
    /// saying the package already holds its lists — not a question with nothing to answer.
    pub fn is_quiet(&self) -> bool {
        self.would_add == 0 && self.would_remove == 0
    }
}

/// One package drawing on one favorite, with both names, as a page reads it.
///
/// **Both directions are drawn from one query.** The Packages page groups these by package to say
/// which lists decide a volume, and the Favorites page groups them by favorite to say which volumes
/// a Delete would change — two readings of one fact, and two queries would be two ideas of what a
/// source is.
#[derive(Debug, Clone)]
pub struct SourceLink {
    /// The package.
    pub package_id: String,
    /// What the package is called.
    pub package_name: String,
    /// The favorite it draws from.
    pub favorite_id: i64,
    /// What that list is called.
    pub favorite_name: String,
}

/// What the near-duplicate pass compares.
#[derive(Debug, Clone)]
pub struct Fingerprint {
    /// Content hash.
    pub id: String,
    /// The structural signature.
    pub fingerprint: String,
    /// Effective title.
    pub title: String,
    /// Effective performer.
    pub artist: String,
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// The song's words as a comparison key, or NULL when it has too few to identify it by.
    ///
    /// The key rather than the text: a corpus is hundreds of thousands of rows, and the words of the
    /// ones that have any would be tens of megabytes held to compute one hash apiece.
    pub lyric_key: Option<String>,
}
