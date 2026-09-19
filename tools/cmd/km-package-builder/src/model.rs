//! The row types the database stores and the view types the templates render.
//!
//! These are deliberately separate from `km-kmpkg`'s manifest types. A manifest describes a song
//! that has been *chosen*; these describe a file somebody is still deciding about, and carry things a
//! package has no place for — how the metadata was detected, what a person typed over it, and how
//! many copies of the file exist.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// What kind of song a row describes.
///
/// A song is a MIDI file, a video file, or an MP3+G pair, and nothing else — the `Video as a song
/// source` and `MP3+G as a song source` decisions in `docs/decisions/`. This is the curation
/// tool's own copy rather than `km_kmpkg::SongKind` because the two answer different questions:
/// that one describes a song already chosen for a package, this one describes a file somebody is
/// still deciding about, and the curation tool must go on compiling without the `video` feature and
/// therefore without anything that names ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SongKind {
    /// A MIDI or `.kar` file, parsed and analyzed.
    #[default]
    Midi,
    /// A video file, probed rather than analyzed.
    Video,
    /// An MP3 and a `.cdg` of the same stem, probed as a pair.
    Cdg,
    /// An UltraStar `.txt` and the MP3 its header names.
    UltraStar,
}

impl SongKind {
    /// The spelling stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Midi => "midi",
            Self::Video => "video",
            Self::Cdg => "cdg",
            Self::UltraStar => "ultrastar",
        }
    }

    /// Reads back what [`Self::as_str`] wrote.
    ///
    /// An unfamiliar value reads as MIDI, which is the safe direction and what the column's own
    /// default says.
    pub fn from_str(value: &str) -> Self {
        match value {
            "video" => Self::Video,
            "cdg" => Self::Cdg,
            "ultrastar" => Self::UltraStar,
            _ => Self::Midi,
        }
    }

    /// Reads a filter's `kind` parameter: one of the three spellings, or `None` for every kind.
    ///
    /// Unlike [`Self::from_str`], anything unfamiliar is `None`, so a hand-edited query string shows
    /// every kind rather than an empty page with no reason.
    pub fn filter(value: &str) -> Option<Self> {
        match value {
            "midi" => Some(Self::Midi),
            "video" => Some(Self::Video),
            "cdg" => Some(Self::Cdg),
            "ultrastar" => Some(Self::UltraStar),
            _ => None,
        }
    }

    /// Whether this is a video song.
    pub fn is_video(self) -> bool {
        matches!(self, Self::Video)
    }

    /// Whether this is an MP3+G song.
    pub fn is_cdg(self) -> bool {
        matches!(self, Self::Cdg)
    }

    /// Whether this is a MIDI song — the only kind with an analysis behind it.
    ///
    /// The question most callers actually want, because everything a MIDI song has and the others do
    /// not (a suitability, a melody channel, lyrics, an encoding) hangs off this one answer.
    pub fn is_midi(self) -> bool {
        matches!(self, Self::Midi)
    }

    /// Whether the machine draws this song's words, rather than the song bringing its own picture.
    ///
    /// The same question `km_kmpkg::SongKind::draws_words` answers, asked of this tool's own copy of
    /// the enum — which exists because the curation tool compiles without the `video` feature. A
    /// control about the words is offered exactly where this is true.
    pub fn draws_words(self) -> bool {
        matches!(self, Self::Midi | Self::UltraStar)
    }

    /// Which sentence a page writes for this.
    ///
    /// **A key, because one of the three is an ordinary word.** `MIDI` and `MP3+G` are format names
    /// and read the same in every language; *video* does not, and a row that said one of the three in
    /// English on a Portuguese page would be saying the same thing two ways.
    pub fn key(self) -> &'static str {
        match self {
            Self::Midi => "kind-midi",
            Self::Video => "kind-video",
            Self::Cdg => "kind-cdg",
            Self::UltraStar => "kind-ultrastar",
        }
    }

    /// Every key [`Self::key`] can return, for the parity tests in [`crate::words`].
    #[cfg(test)]
    pub const KEYS: &'static [&'static str] =
        &["kind-midi", "kind-video", "kind-cdg", "kind-ultrastar"];
}

/// What happened when the scanner last looked at a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    /// Parsed and analyzed.
    Ok,
    /// The bytes could not be read from disk.
    Unreadable,
    /// The bytes are not a MIDI file.
    NotMidi,
    /// The bytes are not a video file ffmpeg can read, or carry no video or audio stream.
    ///
    /// Separate from [`Self::NotMidi`] because the two are reached by different routes and a person
    /// reads them differently: telling somebody their `.mp4` is "not a readable MIDI file" invites
    /// them to look for a problem that is not there.
    NotVideo,
    /// A video file, in a build with no `video` feature.
    ///
    /// Its own status rather than a [`Self::NotVideo`] carrying an explanation, because the scan page
    /// groups failures **by reason** and shows a count and one example — so an explanation held in
    /// the row's error text would never be read. The reason itself has to be the sentence worth
    /// reading, and "this build cannot read video files" is a different thing to know from "this
    /// video is broken": one is fixed by rebuilding, the other by replacing the file.
    VideoUnsupported,
    /// An MP3 with no `.cdg` beside it. Playable audio, and no words: not a song.
    ///
    /// **Deliberately not a silent skip.** Eight files in one measured corpus are half a pair, and a
    /// folder that indexes as fewer songs than it holds, with no reason given, is a folder nobody
    /// can reconcile against what they put in it.
    MissingGraphics,
    /// A `.cdg` with no audio beside it: words, and nothing to sing them over.
    OrphanGraphics,
    /// The audio half of a pair could not be decoded.
    ///
    /// Its own status for the same reason [`Self::NotVideo`] is: telling somebody their MP3 is "not
    /// a readable MIDI file" sends them looking for a problem that is not there.
    NotAudio,
    /// The CD+G stream is unreadable, or never draws a tile — so the pair has no words in it.
    BadGraphics,
    /// An UltraStar file this project does not play: a duet, a version it does not know, or no
    /// BPM or words. The row's error says which.
    BadUltraStar,
    /// An UltraStar file whose named audio is not beside it, is not MP3, or is a video.
    UltraStarAudio,
    /// Parsing panicked. Kept as a status rather than allowed to end the scan.
    Panicked,
}

impl ScanStatus {
    /// The spelling stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unreadable => "unreadable",
            Self::NotMidi => "not_midi",
            Self::NotVideo => "not_video",
            Self::VideoUnsupported => "video_unsupported",
            Self::MissingGraphics => "missing_graphics",
            Self::OrphanGraphics => "orphan_graphics",
            Self::NotAudio => "not_audio",
            Self::BadGraphics => "bad_graphics",
            Self::BadUltraStar => "bad_ultrastar",
            Self::UltraStarAudio => "ultrastar_audio",
            Self::Panicked => "panicked",
        }
    }

    /// Reads back what [`Self::as_str`] wrote. An unfamiliar value is treated as a failure rather
    /// than a success, which is the safe direction.
    pub fn from_str(value: &str) -> Self {
        match value {
            "ok" => Self::Ok,
            "unreadable" => Self::Unreadable,
            "not_midi" => Self::NotMidi,
            "not_video" => Self::NotVideo,
            "video_unsupported" => Self::VideoUnsupported,
            "missing_graphics" => Self::MissingGraphics,
            "orphan_graphics" => Self::OrphanGraphics,
            "not_audio" => Self::NotAudio,
            "bad_graphics" => Self::BadGraphics,
            "bad_ultrastar" => Self::BadUltraStar,
            "ultrastar_audio" => Self::UltraStarAudio,
            _ => Self::Panicked,
        }
    }

    /// Which sentence a page writes for this.
    ///
    /// **A key rather than the sentence**, because a tally is built inside a `query_map` closure
    /// where no request and so no language is in reach. The code travels and the page writes the
    /// words, which is the shape `A refusal travels as a code, and the page writes the sentence`
    /// sets.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ok => "failure-parsed",
            Self::Unreadable => "failure-unreadable",
            Self::NotMidi => "failure-not-midi",
            Self::NotVideo => "failure-not-video",
            Self::VideoUnsupported => "failure-video-unsupported",
            // Note there is deliberately no `CdgUnsupported` to sit beside `VideoUnsupported`:
            // `km-cdg` is pure Rust with no cargo feature, so there is no build of this tool that
            // can find an MP3+G pair and be unable to read it.
            Self::MissingGraphics => "failure-missing-graphics",
            Self::OrphanGraphics => "failure-orphan-graphics",
            Self::NotAudio => "failure-not-audio",
            Self::BadGraphics => "failure-bad-graphics",
            Self::BadUltraStar => "failure-bad-ultrastar",
            Self::UltraStarAudio => "failure-ultrastar-audio",
            Self::Panicked => "failure-panicked",
        }
    }

    /// Every key [`Self::key`] can return.
    ///
    /// The list `words::COMPOSED` chains, so a variant added without a message is a failing test
    /// rather than a bracketed key in the Reason column.
    #[cfg(test)]
    pub const KEYS: &'static [&'static str] = &[
        "failure-parsed",
        "failure-unreadable",
        "failure-not-midi",
        "failure-not-video",
        "failure-video-unsupported",
        "failure-missing-graphics",
        "failure-orphan-graphics",
        "failure-not-audio",
        "failure-bad-graphics",
        "failure-bad-ultrastar",
        "failure-ultrastar-audio",
        "failure-panicked",
    ];
}

/// Everything a scan learned about one file, ready to be written.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Path relative to the root, forward slashes.
    pub path: String,
    /// Size in bytes, as `std::fs` reported it.
    pub size: u64,
    /// Modification time in seconds since the epoch.
    pub mtime: i64,
    /// The content hash, absent only when the bytes could not be read.
    pub content_hash: Option<String>,
    /// How the scan went.
    pub status: ScanStatus,
    /// The error, when there was one.
    pub error: Option<String>,
    /// The song this file is a copy of, when it parsed.
    pub song: Option<ScannedSong>,
}

/// The analysis of one recording, ready to be written.
///
/// Only the detected columns are here. A scan never writes the hand-set ones — that separation is the
/// whole reason corrections survive a re-scan.
#[derive(Debug, Clone)]
pub struct ScannedSong {
    /// The content hash, which is the song's identity.
    pub id: String,
    /// Title as the file gave it.
    pub det_title: Option<String>,
    /// Performer as the file gave it.
    pub det_artist: Option<String>,
    /// Language tag as the file gave it.
    pub det_language: Option<String>,
    /// The file's own name without its extension — the last-resort title, for the very many corpus
    /// files that carry no metadata at all. Not a detection, which is why it sits outside `det_*`.
    pub stem: String,
    /// Length in milliseconds — from the tempo map for a MIDI file, from the probe for a video.
    ///
    /// The one measurement both kinds of song have, which is why it sits here rather than in either
    /// block below.
    pub duration_ms: u32,
    /// Every lyric line, joined with newlines — the text the lyric search reads.
    ///
    /// `None` for an instrumental, so nothing empty reaches the index. Decoded with whatever encoding
    /// the scan settled on, which means pinning a better one and re-scanning re-indexes the words as
    /// well as re-drawing them.
    ///
    /// Always `None` for a video: its words are pixels in somebody else's picture, which is the whole
    /// reason video songs are played rather than transcribed.
    pub lyrics: Option<String>,
    /// A structural signature, for suggesting near-duplicates.
    pub fingerprint: String,
    /// How suitable the file is as a karaoke song, whatever kind of file it is.
    ///
    /// Not inside either block below, and not an `Option`: every song has one, because a song made
    /// to be sung to is answered by what it is where a MIDI file is answered by measurement.
    pub suitability: SuitabilityFacts,
    /// What parsing and analysis found, for a MIDI file.
    pub midi: Option<MidiFacts>,
    /// What a probe found, for a video file.
    pub video: Option<VideoFacts>,
    /// What a probe found, for an MP3+G pair.
    pub cdg: Option<CdgFacts>,
    /// What reading the file found, for an UltraStar song.
    pub ultrastar: Option<UltraStarFacts>,
}

impl ScannedSong {
    /// Which kind of song this is, from which block is filled.
    ///
    /// Derived rather than stored, so the two cannot contradict each other.
    pub fn kind(&self) -> SongKind {
        if self.video.is_some() {
            SongKind::Video
        } else if self.cdg.is_some() {
            SongKind::Cdg
        } else if self.ultrastar.is_some() {
            SongKind::UltraStar
        } else {
            SongKind::Midi
        }
    }
}

/// Everything about a song that only a MIDI file has.
///
/// Grouped rather than left as a dozen `Option` fields because they arrive together and are absent
/// together: they are the result of parsing and analyzing a MIDI file, and a video has had neither
/// done to it. One `Option` around the lot says that; twelve say it twelve times and let a caller
/// invent a combination that cannot happen.
#[derive(Debug, Clone)]
pub struct MidiFacts {
    /// Which karaoke convention the file uses.
    pub flavor: String,
    /// Whether lyrics arrive per syllable, per line, or not at all.
    pub granularity: String,
    /// How many notes the file contains.
    pub note_count: u32,
    /// How many distinct channels sound.
    pub channel_count: u32,
    /// Lyric lines.
    pub line_count: u32,
    /// Lyric syllables.
    pub syllable_count: u32,
    /// The encoding the lyrics were decoded with.
    pub det_encoding: String,
    /// How that encoding was arrived at.
    pub det_encoding_source: String,
    /// The melody channel, when detection was confident.
    pub melody_channel: Option<u8>,
    /// How strongly the evidence favored it.
    pub melody_confidence: Option<f32>,
    /// Why no melody channel was claimed.
    pub melody_abstained: Option<String>,
}

/// The suitability of a song of any kind, and what it is made of.
///
/// **Outside [`MidiFacts`] because every kind of song has one.** A MIDI file's is measured; a video,
/// an MP3+G pair and an UltraStar song are answered by what they are and by how much of them is
/// sung. Keeping the number here and filling it at scan time is what lets the browse list, the
/// suitability filter and the sort read one column and agree — a number invented on the way out
/// agrees with the page and not with the `WHERE` clause beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuitabilityFacts {
    /// Suitability, 0 to 10.
    pub value: u8,
    /// Its four components: lyrics, sync, channels, arrangement.
    pub breakdown: (u8, u8, u8, u8),
    /// Everything wrong with the file, as JSON.
    pub warnings: String,
}

impl SuitabilityFacts {
    /// What a scan writes for a song that was made to be sung to, given how much of it is sung.
    ///
    /// Through `km-pack` rather than derived here, so the number a browse list shows is the number
    /// packaging will write into the manifest.
    #[must_use]
    pub fn purpose_made(sung_ms: u32) -> Self {
        Self::from(&km_pack::purpose_made_suitability(sung_ms))
    }
}

impl From<&km_kmpkg::SuitabilityRecord> for SuitabilityFacts {
    fn from(record: &km_kmpkg::SuitabilityRecord) -> Self {
        Self {
            value: record.value,
            breakdown: (
                record.breakdown.lyrics,
                record.breakdown.sync,
                record.breakdown.channels,
                record.breakdown.arrangement,
            ),
            warnings: serde_json::to_string(&record.warnings).unwrap_or_else(|_| "[]".to_owned()),
        }
    }
}

/// Everything about a song that only an UltraStar file has.
///
/// The lyric counts and the encoding land in the columns a MIDI file's do, so a lyric filter reads
/// both kinds alike. What an UltraStar file says about its title, artist and language is in
/// `det_*`, where a person's correction overrules it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UltraStarFacts {
    /// Lyric lines.
    pub line_count: u32,
    /// Lyric syllables.
    pub syllable_count: u32,
    /// The encoding the file was read in.
    pub det_encoding: String,
    /// How that encoding was arrived at.
    pub det_encoding_source: String,
}

/// Everything about a song that only an MP3+G pair has.
///
/// Grouped for the reason [`MidiFacts`] and [`VideoFacts`] are: they arrive together and are absent
/// together, so one `Option` cannot contradict another.
///
/// Note what is **not** here. No suitability — that is a package-time fact and is a flat 10
/// for a song made to be sung to, not something a scan measures. And no title or artist: an ID3 tag
/// is what the *file* said, so it belongs in `det_title`/`det_artist` beside a MIDI file's parsed
/// title, where a person's correction can overrule it the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdgFacts {
    /// The `.cdg`'s path under the scanned root.
    ///
    /// Derived by rule at play time, and stored here anyway: which file actually got paired is the
    /// one thing somebody looking at a suspicious song wants to see, and the corpus pairs across
    /// mixed-case extensions and a trailing space.
    pub graphics_path: String,
    /// Audio sample rate. Uniformly 44,100 across the measured corpus.
    pub sample_rate: u32,
    /// Audio channels, before the downmix to stereo the feed carries.
    pub channels: u16,
    /// Whole 24-byte packets in the graphics stream.
    pub packets: u32,
    /// How long the graphics run. **Not the song's length** — see `km_cdg::GraphicsStream`.
    pub graphics_ms: u32,
    /// How far the graphics stop short of the audio.
    ///
    /// Seconds is ordinary, because the words end before the outro does. A minute or more is what a
    /// `.cdg` paired with the wrong song looks like, and in one corpus of 2,849 exactly one file
    /// trips it — the same file whose stream is not a whole number of packets.
    pub graphics_short_by_ms: u32,
    /// Tiles the stream draws. **Zero means the pair has no words in it**, whatever else it holds.
    pub tiles_written: u32,
    /// CD+G instructions this build does not implement. Diagnostic, never a quality signal: 223
    /// files of 2,849 carry some, the worst is 29% of its packets, and all of them render.
    pub unknown_instructions: u32,
}

impl CdgFacts {
    /// How long the words run, and how far short of the audio they stop.
    ///
    /// One phrase rather than two numbers, because the interesting thing is the *relationship*: a
    /// few seconds short is ordinary — the words end before the outro does — and a minute or more is
    /// what a `.cdg` paired with the wrong song looks like.
    pub fn graphics_summary(&self) -> String {
        let seconds = f64::from(self.graphics_ms) / 1000.0;
        match self.graphics_short_by_ms {
            0 => format!("{seconds:.0}s, to the end"),
            short if short < 60_000 => format!(
                "{seconds:.0}s, stopping {:.0}s early",
                f64::from(short) / 1000.0
            ),
            short => format!(
                "{seconds:.0}s, stopping {:.0}s early — suspect a mispairing",
                f64::from(short) / 1000.0
            ),
        }
    }
}

/// Everything about a song that only a video has, straight from a probe.
///
/// Note what is not here: a suitability. That is a package-time fact — a flat 10, for a file that was made
/// to be sung to — and not something a scan measures. Nothing below is judged either; a picture's
/// dimensions say nothing about whether the song is worth singing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFacts {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Frames per second times 1000, so the common 29.97 survives being written down.
    pub frame_rate_milli: u32,
    /// The video codec, spelled as ffmpeg spells it.
    pub video_codec: String,
    /// The audio codec, spelled the same way.
    pub audio_codec: String,
}

impl VideoFacts {
    /// The picture size as `1920x1080`.
    pub fn resolution(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    /// Frames per second as text, without a trailing `.000` on the whole ones.
    ///
    /// 29.97 and 30 both have to read correctly, and `{:.2}` would turn the second into `30.00`,
    /// which looks like a measurement where it is a fact.
    pub fn frame_rate(&self) -> String {
        if self.frame_rate_milli.is_multiple_of(1000) {
            (self.frame_rate_milli / 1000).to_string()
        } else {
            format!("{:.2}", f64::from(self.frame_rate_milli) / 1000.0)
        }
    }
}

/// One row of the browse table.
#[derive(Debug, Clone)]
pub struct SongRow {
    /// Content hash.
    pub id: String,
    /// The title to show: hand-typed if there is one, detected otherwise.
    pub title: String,
    /// The performer to show, if any is known.
    pub artist: Option<String>,
    /// Whether the shown title or artist was typed by a person.
    pub edited: bool,
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// What kind of song this is.
    pub kind: SongKind,
    /// Automatic suitability, 0 to 10.
    ///
    /// Measured for a MIDI file. **A flat 10 for a video or MP3+G song**, which is not a measurement
    /// but a fact about what the file is: a purpose-made karaoke file is the best possible answer to
    /// "how good is this as a karaoke source". `None` only for a row scanned before it
    /// existed. See the `Suitability, for a song that was made to be sung to` decision in
    /// `docs/decisions/songs.md`.
    ///
    /// Only two fields on this row are MIDI-only — this and `melody_channel`, which was already
    /// optional — so they stay flat here rather than being grouped the way [`MidiFacts`] groups the
    /// song page's dozen.
    pub suitability: Option<u8>,
    /// The person's own rating, when they have given one.
    pub user_score: Option<u8>,
    /// How many favorites it is in. Zero is "not a favorite"; the star in the row is filled when
    /// it is more, and the number is what the tooltip says.
    pub favorite_count: u32,
    /// Of those, how many are not a working list.
    ///
    /// **This is what colors the star, where the count above only fills it.** Gold says *somebody
    /// filed this*, and a list somebody made to hold the songs they have not decided about yet is
    /// not a filing — so a song in nothing but those keeps a plain star. See
    /// `A favorite can be a working list` in `docs/decisions/curation.md`.
    pub permanent_count: u32,
    /// The melody channel, when one was found.
    pub melody_channel: Option<u8>,
    /// How many files on disk are byte-identical copies of this.
    pub file_count: u32,
    /// How many files look like this same recording, itself included, and 1 when none do.
    ///
    /// A different fact from [`Self::file_count`], which is why the row carries both: that one
    /// counts copies of these exact bytes, this one counts files that are the same song saved
    /// differently.
    ///
    /// The same number on every row of one group, the hidden versions included.
    pub version_count: u32,
    /// The version the song list shows in this one's place, when this one is hidden behind it.
    pub duplicate_of: Option<String>,
    /// Whether the song has any words at all, which decides whether the row offers to look for the
    /// songs that sing them.
    ///
    /// Most of a real corpus is instrumental, and a button that can only lead to a page saying *this
    /// song has no words* is a button worth not drawing.
    pub has_words: bool,
    /// The first file's path, for the "open" and "play" actions.
    pub path: String,
    /// Every copy's path, newline-separated, straight from the query.
    ///
    /// Held as one string rather than a `Vec` because that is what `group_concat` returns and
    /// splitting it is only worth doing for the one row somebody hovers over.
    pub paths: String,
    /// Whether [`Self::title`] is only the file's name, because nobody typed a title and the file
    /// gave none. Shown as a hint rather than hidden: telling a curated song from an untouched one is
    /// most of what browsing this list is for.
    pub from_filename: bool,
    /// The language to act on: what a person chose, else what the file's evidence implied.
    ///
    /// The code, not the name. A column of "Portuguese (Brazil)" is 22 characters wide in a table
    /// already fighting for room, and the code is what the filter and the API take — so a curator
    /// reads the vocabulary they will type.
    pub language: Option<String>,
    /// What this song is filed under, sorted.
    ///
    /// Filled per page by `Db::tags_of_many` rather than per row, which is the difference between
    /// one query and fifty. Empty is the ordinary case: nothing detects a tag.
    pub tags: Vec<String>,
    /// Where this song sits in the quality hint, when one has been asked for.
    ///
    /// **A position and not a rating**: 1 is the file to play first of those somebody ticked. It is
    /// held for the run in [`crate::server::State`] and filled per page from there, so it is neither
    /// a column of the browse query nor a column of the database. See
    /// `A quality hint is a position on the row, and it is rubbed out rather than kept` in
    /// `docs/decisions/curation.md`.
    pub hint: Option<u32>,
    /// What the artist link says it would show, which names the artist.
    ///
    /// **Composed rather than assembled in markup**, which is the rule every sentence carrying a
    /// value follows here. The four that follow are the same shape, and all five are filled by
    /// [`Self::say`] — a row comes out of the database, where no language is in reach.
    pub artist_title: String,
    /// What the melody tick says, which names the channel.
    pub melody_title: String,
    /// What the versions badge says, which counts the files this row stands for.
    pub versions_title: String,
    /// What the favorites button says, which counts the lists this song is in.
    pub favorite_title: String,
    /// The title's tooltip: the name a person would recognize, and how many copies there are.
    pub path_said: String,
    /// How alike this song's name is to the one searched for, from 0 to 1, on the similar-names list
    /// and `None` on every other.
    ///
    /// A field of the row rather than of a hit wrapped around it, because it is a cell of the row.
    /// See [`crate::similar::likeness`].
    pub likeness: Option<f32>,
    /// Whether this is the song a similar-names search started from, which heads the list marked.
    pub searched_from: bool,
}

impl SongRow {
    /// The likeness as a whole percentage, or empty when the row has none.
    pub fn likeness_text(&self) -> String {
        self.likeness_percent()
            .map(|percent| format!("{percent}%"))
            .unwrap_or_default()
    }

    /// Whether the likeness is high enough to draw in the color that means *good*.
    pub fn likeness_is_high(&self) -> bool {
        self.likeness_percent().is_some_and(|percent| percent > 90)
    }

    fn likeness_percent(&self) -> Option<u32> {
        self.likeness
            .map(|likeness| (likeness * 100.0).round() as u32)
    }
    /// The English name of this row's language, for the cell's tooltip.
    pub fn language_name(&self) -> &str {
        language_name(self.language.as_deref())
    }

    /// This row's language as the select's `<option>` values spell it, or `""` when it has none.
    ///
    /// The counterpart of [`SongRow::user_score_text`], and it exists for the same reason: one list of
    /// options serves every row, so which one is selected is decided in the template by comparing
    /// against this — and askama has no way to compare an `Option<String>` with a `&str` that reads
    /// like anything at all.
    ///
    /// **Note it is the *effective* language**, so a row whose tag came from the file's own evidence
    /// shows that tag preselected. Choosing it again is not a no-op: it writes what was inferred into
    /// the column a person owns, which is the same thing *Title from file name* does with a stem, and
    /// is a reasonable way to say *yes, that one is right*.
    pub fn language_text(&self) -> &str {
        self.language.as_deref().unwrap_or_default()
    }
}

/// One result of searching the lyrics.
///
/// A whole [`SongRow`] rather than a title and an id, so a hit is the same row the browse table draws
/// and everything on it — the star, the play button, the rating select — works where it stands.
/// Finding a song by a line you half-remember and then having to go and look it up somewhere else to
/// do anything with it would be a search that stops one step short of useful.
#[derive(Debug, Clone)]
pub struct LyricHit {
    /// The song, exactly as the browse table would draw it.
    pub song: SongRow,
    /// The matching passage, still carrying the STX/ETX markers FTS5 was asked to put around each
    /// matched term. [`crate::views::highlight`] is what turns it into something a template can
    /// safely render.
    pub passage: String,
}

impl SongRow {
    /// Length as `m:ss`, or `-` when the file claims none.
    pub fn duration(&self) -> String {
        format_duration(self.duration_ms)
    }

    /// The person's rating as text, blank when they have not given one.
    ///
    /// Blank rather than a dash: a dash is a mark on the page that has to be read and dismissed, and
    /// over a page of rows of a corpus almost none of which anybody has rated, that is a column of
    /// noise. Nothing there says "nothing here" more quietly.
    pub fn user_score_text(&self) -> String {
        score_text(self.user_score)
    }

    /// This row's place in the quality hint as text, empty when it has none.
    ///
    /// Empty and not a dash, for [`Self::user_score_text`]'s reason turned up: the badge is a filled
    /// mark, so anything at all in it is a shape on the row. `.place:empty` is what takes it away,
    /// and it can only fire on an element that is genuinely empty.
    pub fn hint_text(&self) -> String {
        self.hint.map(|at| at.to_string()).unwrap_or_default()
    }

    /// The path to show when somebody hovers the title.
    ///
    /// A song can have several byte-identical copies filed under different names; this is the one
    /// worth showing. See [`nicest_path`].
    pub fn display_path(&self) -> &str {
        if self.paths.is_empty() {
            return &self.path;
        }
        nicest_path(&self.paths)
    }

    /// The name of that copy, without its folders.
    ///
    /// Shown beside the title when the browse list is asked for file names, because a corpus is full
    /// of songs whose detected title says less than the name somebody typed on disk. The extension
    /// is kept: `.kar` and `.mid` are the two kinds of file here and which one a copy is is worth
    /// seeing at a glance.
    ///
    /// Split on `/` rather than through [`std::path::Path`]: `files.path` is stored relative to the
    /// root with forward slashes precisely so the folder can move, and `Path` on Linux treats `\` as
    /// an ordinary character — so `Path::file_name` would give two different answers for the same
    /// database read on two platforms.
    pub fn file_name(&self) -> &str {
        let path = self.display_path();
        path.rsplit('/').next().unwrap_or(path)
    }

    /// Where clicking this row's artist goes, or empty when there is nothing to click.
    ///
    /// The shape [`crate::db::model::FolderNode::folder_url`] set: the link is built here so the
    /// encoding happens in Rust, and an empty string is how the template is told to draw plain text
    /// instead — a song with no artist has no *set* of songs to lead to, and a link to
    /// `?artist=` would be a filter matching nothing.
    ///
    /// **The link carries nothing else, and that is forced rather than chosen.** `song_row.html` is
    /// included by the browse list, by the single-row fragment that a rename, a score or a star
    /// swaps back in, and by the lyric-search hits — and the fragment routes never see the browse
    /// query at all. (That is the same fact `show_filename` answers by being a class on `#rows`
    /// rather than a field on a row.) A row therefore cannot know what else is narrowing the list.
    /// It is also what every other filter link in this tool does: the Folders page, the Favorites
    /// page and a package's *not packaged* link all replace the filter rather than adding to it, and
    /// the chips strip is what shows the result either way.
    pub fn artist_url(&self) -> String {
        match self.artist.as_deref().map(str::trim) {
            Some(artist) if !artist.is_empty() => format!("/songs?artist={}", encode(artist)),
            _ => String::new(),
        }
    }

    /// What hovering the title says: the copy to read, and how many there are.
    ///
    /// Listing **every** path, headed by the count, is the thorough answer, and it keeps a folder
    /// filter from making a row look misplaced. On a corpus that files one recording under six
    /// names it turns the hover into a wall of near-identical paths, none of which is read —
    /// reading six of them to learn
    /// that a song exists six times is worse than being told it exists six times. So: the name a
    /// person would recognize ([`nicest_path`]), then the count.
    ///
    /// The count is [`Self::file_count`] — the same number the Copies column shows, rather than the
    /// number of lines here, so the two can never look like they disagree. It stays in the tooltip
    /// rather than becoming a badge beside the title: that was tried, and a second visible marker for
    /// something already in its own column is noise.
    pub fn path_tooltip(&self) -> &str {
        &self.path_said
    }

    /// Whether a YouTube search would be worth offering.
    /// Words the four sentences on this row that carry a value.
    ///
    /// Called by [`crate::server::State::say_rows`], beside the hint pass, for the same reason: what
    /// a row says is a fact about the page being drawn rather than about the corpus, and a database
    /// module has no language in reach.
    pub fn say(&mut self, locale: km_locale::Locale, picking: bool) {
        let words = crate::words::messages(locale);
        self.artist_title = match self.artist.as_deref() {
            Some(artist) if !artist.is_empty() => words
                .msg_with("row-artist-title", &[("artist", artist.into())])
                .into_owned(),
            _ => String::new(),
        };
        self.melody_title = match self.melody_channel {
            Some(channel) => words
                .msg_with(
                    "row-melody-title",
                    &[("channel", i64::from(channel).into())],
                )
                .into_owned(),
            None => String::new(),
        };
        // The singular never appears (a count of one takes the path alone), which is why the plural
        // arm can be worded without a selector.
        self.path_said = match self.file_count {
            0 | 1 => self.display_path().to_owned(),
            count => words
                .msg_with(
                    "row-path-copies",
                    &[
                        ("path", self.display_path().into()),
                        ("count", i64::from(count).into()),
                    ],
                )
                .into_owned(),
        };
        // A hidden version does not stand for the group, so it says which of the group it is.
        let count = [("count", i64::from(self.version_count).into())];
        self.versions_title = match self.duplicate_of {
            Some(_) => words.msg_with("row-hidden-version-count-title", &count),
            None => words.msg_with("row-versions-title", &count),
        }
        .into_owned();
        // Four states, and which one a reader is in decides what the button offers. `permanent_count`
        // is filings; `favorite_count` counts working lists as well.
        let key = if picking {
            "row-favorites-close"
        } else if self.permanent_count > 0 {
            "row-favorites-filed"
        } else if self.favorite_count > 0 {
            "row-favorites-working"
        } else {
            "row-favorites-none"
        };
        self.favorite_title = words
            .msg_with(key, &[("count", i64::from(self.favorite_count).into())])
            .into_owned();
    }

    /// Where the ≈ button goes: songs whose name is like this row's, or empty when the row has no
    /// name to compare.
    ///
    /// A title that is only the file's name is searched as it is: a stem often holds the artist and
    /// the title together, and the likeness pools the two for exactly that case.
    pub fn similar_url(&self) -> String {
        similar_url(&self.title, self.artist.as_deref().unwrap_or(""), &self.id)
    }

    /// Where the ≋ button goes: songs that sing what this row sings, or empty when it has no words.
    ///
    /// Empty for most of a real corpus, which is instrumental — and a button leading only to a page
    /// that says *this song has no words* is a button worth not drawing on every row.
    pub fn words_url(&self) -> String {
        if self.has_words {
            words_url(&self.id)
        } else {
            String::new()
        }
    }

    pub fn searchable(&self) -> bool {
        youtube_query(&self.title, self.artist.as_deref(), &self.path).is_some()
    }

    /// The YouTube search URL, or an empty string when there is nothing worth searching for.
    ///
    /// Templates guard on [`Self::searchable`] first; this returning empty is a belt-and-braces
    /// answer rather than a case that should be rendered.
    pub fn youtube_url(&self) -> String {
        youtube_query(&self.title, self.artist.as_deref(), &self.path)
            .map(|query| {
                format!(
                    "https://www.youtube.com/results?search_query={}",
                    encode(&query)
                )
            })
            .unwrap_or_default()
    }
}

/// The similar-names search for one song's name, headed by that song, or empty when the name has no
/// words to search for.
///
/// Built here so the encoding happens in Rust, as [`SongRow::artist_url`] is. The song page asks for
/// it from the song's effective names, and a browse row from the names it shows.
pub fn similar_url(title: &str, artist: &str, id: &str) -> String {
    if crate::similar::match_query(title, artist).is_none() {
        return String::new();
    }
    let mut url = format!("/similar?title={}", encode(title.trim()));
    if !artist.trim().is_empty() {
        url.push_str(&format!("&artist={}", encode(artist.trim())));
    }
    url.push_str(&format!("&from={}", encode(id)));
    url
}

/// The same-words search for one song, headed by that song.
///
/// **An id and nothing else.** The similar-names search carries the name it is looking for, because
/// that name is editable where it lands; the subject here is a whole lyric body, so the address names
/// the song and the page reads its words out of the row.
///
/// Built here so the encoding happens in Rust, as [`similar_url`] is.
pub fn words_url(id: &str) -> String {
    format!("/similar-words?from={}", encode(id))
}

/// A hand-set score as text: the number, or nothing at all when nobody has said.
fn score_text(score: Option<u8>) -> String {
    score.map(|score| score.to_string()).unwrap_or_default()
}

/// The best-looking path out of a newline-separated list of copies.
///
/// The corpus files the same recording under several names — `CORCOVAD.KAR` in one folder and
/// `Corcovado - Tom Jobim.kar` in another — and only one of them can go in a tooltip. The one worth
/// showing is the one a person would recognize, so a name is scored by how much like a name it looks:
/// spaces and lower-case letters are what a truncated DOS filename does not have, and a longer stem
/// carries more of the title. Ties break on the shortest whole path, so the answer does not depend on
/// the order `group_concat` happened to return.
///
/// This decides what is *shown* and nothing else. Playing, opening and packaging all keep going
/// through the first file by path, because the prettiest name is not necessarily the copy a package
/// was built from, and quietly changing which bytes those act on would be a different feature.
pub fn nicest_path(paths: &str) -> &str {
    paths
        .split('\n')
        .filter(|path| !path.is_empty())
        .max_by_key(|path| (name_quality(path), std::cmp::Reverse(path.len())))
        .unwrap_or("")
}

/// How much the file's own name looks like a song title rather than an 8.3 stub.
fn name_quality(path: &str) -> u32 {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let mut score = 0;
    if stem.contains(' ') || stem.contains('_') || stem.contains('-') {
        score += 4;
    }
    if stem.chars().any(char::is_lowercase) {
        score += 2;
    }
    if !stem.chars().all(|c| c.is_ascii_digit()) {
        score += 1;
    }
    // Length last and bounded, so a long name breaks a tie between equally shaped ones without ever
    // outweighing the shape itself.
    score * 64 + (stem.chars().count() as u32).min(63)
}

/// The English name of a language tag, for a cell's tooltip, and `""` for no tag at all.
///
/// Falls back to whatever is stored when the table does not know the tag, so a value written by a
/// later build is shown rather than swallowed.
///
/// One function because two tables draw the column — the browse row and a package's members — and a
/// tag named one thing in one of them and another in the other is the confusion the column removes.
pub fn language_name(tag: Option<&str>) -> &str {
    let Some(tag) = tag else {
        return "";
    };
    match km_kmpkg::Language::parse(tag) {
        Some(language) => language.name(),
        None => tag,
    }
}

/// Milliseconds as `m:ss`.
pub fn format_duration(ms: u32) -> String {
    if ms == 0 {
        return "—".to_owned();
    }
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The text to search YouTube for, or `None` when there is nothing worth searching for.
///
/// A title that is only the file's name is not a song title — the corpus is full of `CORCOVAD` and
/// `AMD0123` — so a link built from one sends somebody to a page of nothing. With an artist the title
/// no longer has to stand alone, which is why the file-stem test only applies when there is not one.
pub fn youtube_query(title: &str, artist: Option<&str>, path: &str) -> Option<String> {
    let title = title.trim();
    let artist = artist.map(str::trim).filter(|value| !value.is_empty());

    if let Some(artist) = artist {
        return Some(if title.is_empty() {
            artist.to_owned()
        } else {
            format!("{artist} {title}")
        });
    }

    if title.is_empty() || looks_like_a_filename(title, path) {
        return None;
    }
    Some(title.to_owned())
}

/// Whether a title is just the file's own name dressed up.
fn looks_like_a_filename(title: &str, path: &str) -> bool {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !stem.is_empty() && stem.eq_ignore_ascii_case(title) {
        return true;
    }
    // No spaces and no lower-case letters is the shape of a truncated 8.3 name rather than a title.
    // "CORCOVAD" and "AMD0123" both fail this; "Corcovado" and "Águas de Março" both pass.
    !title.contains(' ') && !title.chars().any(char::is_lowercase)
}

/// Percent-encodes a value for a URL. Re-exported from [`crate::form`], which owns both halves.
pub use crate::form::encode;

/// An absolute path in the form a person recognizes.
///
/// `canonicalize` on Windows returns `\\?\C:\…`, which is valid, which nobody types, and which nobody
/// reading it in a settings file recognizes — inviting somebody to "fix" it into something that no
/// longer matches. `km-app` strips it before storing a path for exactly that reason, and this tool
/// puts paths in front of people twice over: in the guidance for `debug.play_file_roots`, and in the
/// heading of every page.
///
/// A path that cannot be canonicalised is returned as it came, because a path that does not exist yet
/// is still worth showing.
pub fn tidy(path: &Path) -> std::path::PathBuf {
    let Ok(canonical) = path.canonicalize() else {
        return path.to_path_buf();
    };
    match canonical.to_string_lossy().strip_prefix(r"\\?\") {
        Some(stripped) => std::path::PathBuf::from(stripped),
        None => canonical,
    }
}

/// One favorite, as a page draws it.
#[derive(Debug, Clone)]
pub struct FavoriteNode {
    /// Row id.
    pub id: i64,
    /// What the list is called, and the only thing any control labels it by.
    pub name: String,
    /// How many songs are in it.
    pub song_count: u32,
    /// Of those, how many are a second version of a song already in this same list.
    ///
    /// Memberships minus distinct recordings. A list holding three files of one song contributes
    /// two, whichever of the three the browse list would have shown.
    pub second_copies: u32,
    /// Whether this list is scaffolding for a later pass rather than a filing.
    ///
    /// A song in nothing but working lists has not been filed, and the browse row's gold star says
    /// so. Nothing else treats one differently: it holds songs, it filters, it backs up and it
    /// packages exactly as any other favorite does.
    pub temporary: bool,
}

/// A filter somebody named.
///
/// The whole of what the strip draws. `sort_key` and `saved_at` stay in the table: one is a fold of
/// `name` and the other is a stamp nothing on the page reads.
#[derive(Debug, Clone, Default)]
pub struct SavedFilter {
    /// Row id, which the forget route takes.
    pub id: i64,
    /// What somebody called it: `Portuguese, unclassified`.
    pub name: String,
    /// The query string with no leading `?`, as `FilterQuery::rebuild` writes it.
    ///
    /// **Empty is a real value and means the whole corpus**, so the link that restores it writes
    /// `/songs?` with the `?` always there: a bare `/songs` is answered with the *remembered*
    /// filter — see [`crate::handlers::songs`] — which is the opposite of what was saved.
    pub query: String,
}

/// What a volume format writes the volume's number as.
pub const VOLUME_NUMBER: &str = "{n}";

/// The volume format a package starts with. A word before the number, because a bare number lands
/// beside the version in a file name and `brasil-2-1.0.0` reads as one run of digits.
pub const DEFAULT_VOLUME_FORMAT: &str = "vol{n}";

/// A package being curated, seen through one of its volumes.
///
/// **One row type for both, because every page shows a package through a volume.** A package of one
/// volume is the case nearly everything is, and there the two are the same thing; a page about a
/// larger package draws the volume somebody picked. So the version, the first number, the output
/// path, the build time and the song count are that volume's, and the rest are the package's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRow {
    /// Stable package identifier. The first volume's id is the same value.
    pub id: String,
    /// Display name.
    pub name: String,
    /// This volume's version string.
    pub version: String,
    /// Who made it.
    pub publisher: Option<String>,
    /// The first song number this volume assigns when numbering automatically.
    pub start_number: u32,
    /// Files any song that names no language of its own under this code, **in the package only**.
    ///
    /// `None` means no default, and the build then refuses such a song — which is what this tool did
    /// for everybody before the column existed. Nothing here is ever written back to a song: a
    /// package saying "call the rest English" is a statement about one package, not a claim that
    /// somebody looked at each song.
    pub default_language: Option<String>,
    /// Where this volume's `.kmpkg` was last written.
    pub out_path: Option<String>,
    /// When this volume was last built.
    pub built_at: Option<String>,
    /// How many songs this volume holds.
    pub song_count: u32,
    /// Which volume this row shows, from 1.
    pub volume: u32,
    /// This volume's own id, which a build writes into its manifest and a machine banks.
    pub volume_id: String,
    /// How many volumes the package has.
    pub volumes: u32,
    /// How many songs the whole package holds.
    pub total_songs: u32,
    /// How a volume's number is written after the name, with `{n}` standing for the number.
    pub volume_format: String,
    /// Numbers the volume while the package has only one, for a set that will outgrow 999 songs.
    pub number_one_volume: bool,
}

impl PackageRow {
    /// A row for a package that does not exist yet, which the create form and an import fill in.
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: Some("en".to_owned()),
            out_path: None,
            built_at: None,
            song_count: 0,
            volume: 1,
            volume_id: id.to_owned(),
            volumes: 1,
            total_songs: 0,
            volume_format: DEFAULT_VOLUME_FORMAT.to_owned(),
            number_one_volume: false,
        }
    }

    /// What this volume's file and manifest are called.
    ///
    /// **The package's name while it has one volume, and numbered once it has two**, every volume
    /// alike: `Brasil vol1`, `Brasil vol2`. A curator who never outgrows 999 songs never sees a
    /// number. [`number_one_volume`](Self::number_one_volume) numbers the only volume too, so a set
    /// that will outgrow 999 keeps its first file name when the second volume starts.
    pub fn volume_name(&self) -> String {
        if self.volumes > 1 || self.number_one_volume {
            format!(
                "{} {}",
                self.name,
                self.volume_format
                    .replace(VOLUME_NUMBER, &self.volume.to_string())
            )
        } else {
            self.name.clone()
        }
    }
}

/// A song carrying something a person typed, flat, for a backup to write down.
///
/// **Deliberately not [`SongDetail`](crate::db::SongDetail)**, which is what a page needs: that one
/// runs three more queries per song — the files, the favorites and the packages — and a backup of a
/// curated corpus asks for a few thousand of these at once. Two flat queries against thousands of
/// round trips is the whole reason this type exists.
///
/// Every field but `seen_as` is a hand-set column of `songs`. `seen_as` is the effective title, and
/// is here to *name* a song in a report rather than to be restored — see `crate::backup::SongBackup`.
#[derive(Debug, Clone)]
pub struct HandSetSong {
    /// The content hash of the file's bytes, which is the song's id and a backup's rejoin key.
    pub id: String,
    /// The title somebody typed.
    pub title: Option<String>,
    /// The performer somebody typed.
    pub artist: Option<String>,
    /// The language somebody chose, as an ISO 639-1 tag.
    pub language: Option<String>,
    /// The lyric encoding somebody pinned.
    pub lyric_encoding: Option<String>,
    /// The transposition to apply by default.
    pub default_transpose: Option<i64>,
    /// Whether to draw the song's words. `None` is *nobody has said*, and the analysis stands.
    pub lyrics_hidden: Option<bool>,
    /// The corrections somebody decided on, as stored JSON. `None` is *nobody has said*.
    pub fixes: Option<String>,
    /// The melody channel somebody named. `None` is *nobody has said*, and detection stands.
    pub melody_chosen: Option<String>,
    /// How good a karaoke file somebody said this is, 0-10. `None` is unset, which is not 0.
    pub user_score: Option<i64>,
    /// Whatever somebody wrote about it.
    pub notes: Option<String>,
    /// The song this one was folded into, when somebody said they are the same recording.
    pub merged_into: Option<String>,
    /// What this song is called here, for a report to name it by. Never restored.
    pub seen_as: String,
}

/// One member of a package.
#[derive(Debug, Clone)]
pub struct PackageMember {
    /// The queueing number.
    pub number: u32,
    /// The song's content hash.
    pub song_id: String,
    /// Title as it would be written into the package.
    pub title: String,
    /// Performer as it would be written into the package.
    pub artist: Option<String>,
    /// The language to act on: what a person chose, else what the file's evidence implied.
    ///
    /// **The code, as the browse column draws it**, because a column of "Portuguese (Brazil)" is 22
    /// characters wide in a table already fighting for room and the code is what the filter and the
    /// API take. `None` is nobody has said, and the package's own default language is what such a
    /// song is written under — the form that sets it is on the pane directly above this table.
    pub language: Option<String>,
    /// Automatic suitability, or `None` for a video song, which has none.
    pub suitability: Option<u8>,
    /// What a person rated the song, which is the judgment a suitability cannot make.
    ///
    /// **Beside the suitability rather than instead of it**, for the reason the browse list draws
    /// both: one is what the file's notes and timings came to, the other is somebody's ear, and a
    /// package is built out of the second where the first only narrows the field.
    pub user_score: Option<u8>,
    /// Which channel carries the tune, when one was found.
    ///
    /// Drawn as a mark rather than a number, exactly as the browse row draws it: what a curator
    /// reads off a package is whether the words will be highlighted, and the channel itself is a
    /// thing they change on the song's own page.
    pub melody_channel: Option<u8>,
    // There is no `kind` here, and the absence is the rule: what a song *is* is decided from its
    // file, by `km_pack::build`, and never from anything a description or a row claims. A second
    // source of truth that can disagree with the bytes is a defect waiting to be filed.
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// The file this entry reads from, when it still exists.
    ///
    /// **Read for whether it is there and not for what it says.** A path is the widest column a
    /// package could carry and the least like anything somebody is looking for; what the page needs
    /// from it is the one case that stops a build, which is a source that has gone.
    pub path: Option<String>,
}

impl PackageMember {
    /// Length as `m:ss`.
    pub fn duration(&self) -> String {
        format_duration(self.duration_ms)
    }

    /// The English name of this member's language, for the cell's tooltip.
    pub fn language_name(&self) -> &str {
        language_name(self.language.as_deref())
    }
}

/// One warning, as stored in the `warnings` JSON column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWarning {
    /// Machine-readable code.
    pub code: String,
    /// What is wrong, in words a packager can act on.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_with_an_artist_is_always_searchable() {
        assert_eq!(
            youtube_query("CORCOVAD", Some("Tom Jobim"), "x/CORCOVAD.kar").as_deref(),
            Some("Tom Jobim CORCOVAD")
        );
    }

    #[test]
    fn a_title_that_is_only_the_filename_is_not_worth_searching_for() {
        assert!(youtube_query("CORCOVAD", None, "x/CORCOVAD.kar").is_none());
        assert!(youtube_query("AMD0123", None, "x/other.kar").is_none());
        assert!(youtube_query("", None, "x/other.kar").is_none());
    }

    #[test]
    fn a_real_title_is_searchable_without_an_artist() {
        assert_eq!(
            youtube_query("Águas de Março", None, "x/AGUAS.kar").as_deref(),
            Some("Águas de Março")
        );
        // One word, but capitalised like a word rather than a DOS filename.
        assert_eq!(
            youtube_query("Corcovado", None, "x/CORCOVAD.kar").as_deref(),
            Some("Corcovado")
        );
    }

    #[test]
    fn an_empty_artist_does_not_rescue_a_filename_title() {
        assert!(youtube_query("CORCOVAD", Some("   "), "x/CORCOVAD.kar").is_none());
    }

    #[test]
    fn queries_are_percent_encoded() {
        assert_eq!(encode("Tom Jobim"), "Tom+Jobim");
        assert_eq!(encode("Águas"), "%C3%81guas");
        assert_eq!(encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn a_path_that_does_not_exist_is_shown_as_it_came() {
        // `canonicalize` fails, and a path nobody can resolve is still worth putting on a page.
        let path = Path::new("/definitely/not/here");
        assert_eq!(tidy(path), path);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_windows_path_loses_the_prefix_nobody_recognizes() {
        // The temp directory certainly exists, so this really does go through `canonicalize` and
        // really does have a `\\?\` prefix to strip. Leaving it on put an unusable path into the
        // `debug.play_file_roots` guidance, which is where this was noticed.
        let tidied = tidy(&std::env::temp_dir());
        assert!(
            !tidied.to_string_lossy().starts_with(r"\?\"),
            "{} still carries the UNC prefix",
            tidied.display()
        );
        assert!(tidied.is_absolute());
    }

    #[test]
    fn the_prettiest_of_several_copies_is_the_one_worth_showing() {
        // The same recording, filed twice. One name is a truncated DOS one and the other is what
        // somebody would recognize; the tooltip gets the second.
        assert_eq!(
            nicest_path("new3/karaoke1/CORCOVAD.KAR\nBrasil/Corcovado - Tom Jobim.kar"),
            "Brasil/Corcovado - Tom Jobim.kar"
        );
        // Order must not decide it: `group_concat` promises none.
        assert_eq!(
            nicest_path("Brasil/Corcovado - Tom Jobim.kar\nnew3/karaoke1/CORCOVAD.KAR"),
            "Brasil/Corcovado - Tom Jobim.kar"
        );
        // A bare number is the least useful name there is.
        assert_eq!(nicest_path("x/00123.mid\ny/Rosinha.mid"), "y/Rosinha.mid");
    }

    #[test]
    fn one_copy_is_its_own_best_copy_and_none_is_empty() {
        assert_eq!(nicest_path("only/ONE.KAR"), "only/ONE.KAR");
        assert_eq!(nicest_path(""), "");
    }

    #[test]
    fn equally_shaped_names_break_the_tie_the_same_way_every_time() {
        // Same shape, same stem length: the shorter whole path wins, so the answer does not depend
        // on which row the database happened to return first.
        let paths = "deep/nested/folder/Some Song.kar\na/Some Song.kar";
        assert_eq!(nicest_path(paths), "a/Some Song.kar");
        let reversed = "a/Some Song.kar\ndeep/nested/folder/Some Song.kar";
        assert_eq!(nicest_path(reversed), "a/Some Song.kar");
    }

    /// A row carrying nothing but the two path columns, which is all the name helpers read.
    /// Green means more than nine in ten, and a row off the similar-names list says nothing.
    #[test]
    fn the_likeness_is_a_percentage_and_green_above_ninety() {
        let mut row = row_with_paths("a/X.kar", "a/X.kar");
        assert_eq!(row.likeness_text(), "");
        assert!(!row.likeness_is_high());

        row.likeness = Some(0.904);
        assert_eq!(row.likeness_text(), "90%");
        assert!(!row.likeness_is_high());

        row.likeness = Some(0.912);
        assert_eq!(row.likeness_text(), "91%");
        assert!(row.likeness_is_high());
    }

    fn row_with_paths(path: &str, paths: &str) -> SongRow {
        SongRow {
            id: "abc123".to_owned(),
            title: "Corcovado".to_owned(),
            artist: None,
            language: Some("pt".to_owned()),
            tags: Vec::new(),
            hint: None,
            artist_title: String::new(),
            melody_title: String::new(),
            versions_title: String::new(),
            favorite_title: String::new(),
            path_said: String::new(),
            likeness: None,
            searched_from: false,
            edited: false,
            duration_ms: 200_000,
            suitability: Some(8),
            kind: SongKind::Midi,
            user_score: None,
            favorite_count: 0,
            permanent_count: 0,
            melody_channel: None,
            file_count: 1,
            version_count: 1,
            duplicate_of: None,
            has_words: false,
            path: path.to_owned(),
            paths: paths.to_owned(),
            from_filename: false,
        }
    }

    #[test]
    fn the_file_name_is_the_prettiest_copys_name_without_its_folders() {
        let song = row_with_paths(
            "new3/karaoke1/CORCOVAD.KAR",
            "new3/karaoke1/CORCOVAD.KAR\nBrasil/Corcovado - Tom Jobim.kar",
        );
        // The same copy the hover names, so the two cannot disagree about which file this is.
        assert_eq!(song.file_name(), "Corcovado - Tom Jobim.kar");
        // The extension stays: which of the two kinds of file a copy is is worth seeing.
        assert!(song.file_name().ends_with(".kar"));
    }

    #[test]
    fn a_file_at_the_root_is_its_own_name() {
        let song = row_with_paths("CORCOVAD.KAR", "CORCOVAD.KAR");
        assert_eq!(song.file_name(), "CORCOVAD.KAR");
    }

    #[test]
    fn a_row_with_no_copies_listed_falls_back_to_its_one_path() {
        // `paths` is empty only when `group_concat` returned nothing; `display_path` then uses
        // `path`, and the name has to come from there rather than being blank.
        let song = row_with_paths("a/b/Rosinha.mid", "");
        assert_eq!(song.file_name(), "Rosinha.mid");
    }

    #[test]
    fn a_backslash_is_an_ordinary_character_in_a_stored_path() {
        // Paths are stored relative with forward slashes on every platform. A name that really does
        // contain a backslash must not be cut at it, which `Path::file_name` would do on Windows.
        let song = row_with_paths("odd/AC\\DC - Jailbreak.kar", "");
        assert_eq!(song.file_name(), "AC\\DC - Jailbreak.kar");
    }

    #[test]
    fn an_unset_score_shows_as_nothing_at_all() {
        assert_eq!(score_text(None), "");
        assert_eq!(score_text(Some(0)), "0");
        assert_eq!(score_text(Some(10)), "10");
    }

    #[test]
    fn durations_read_as_minutes_and_seconds() {
        assert_eq!(format_duration(0), "—");
        assert_eq!(format_duration(65_000), "1:05");
        assert_eq!(format_duration(203_800), "3:23");
    }

    #[test]
    fn an_unfamiliar_scan_status_is_a_failure_not_a_success() {
        assert_eq!(ScanStatus::from_str("ok"), ScanStatus::Ok);
        assert_eq!(ScanStatus::from_str("not_midi"), ScanStatus::NotMidi);
        assert_eq!(ScanStatus::from_str("something new"), ScanStatus::Panicked);
    }
}
