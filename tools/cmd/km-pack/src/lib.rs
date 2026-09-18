//! The packaging pipeline, as a library.
//!
//! `km-pack` the command is one caller of this; the curation tool in `tools/cmd/km-package-builder` is
//! the other. What lives here is everything both need to agree on — how a folder of MIDI is walked,
//! why a file is refused, how a [`km_suitability::Analysis`] becomes the records a manifest stores, and
//! what a hand-edited field means. Keeping one definition of those matters more than the line count
//! suggests: two tools writing subtly different manifests from the same file is a defect nobody
//! would notice until a package behaved oddly on the machine.
//!
//! Nothing here prints. Reporting is the caller's business, because a CLI and a web page want very
//! different things from the same result.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use km_kmpkg::{
    BreakdownRecord, EditedField, Language, MelodyRecord, PackageBuilder, SongEntry,
    SuitabilityRecord, WarningRecord,
};
use km_song::Song;
use km_suitability::{Analysis, MelodyOutcome};

#[cfg(feature = "video")]
pub mod profile;

pub mod book;
pub mod build;
pub mod describe;
pub mod listing;
pub mod spec;
pub mod ultrastar;

#[cfg(test)]
mod cdg_tests;

pub use build::{BuildEvent, BuildOptions, BuildOutcome, Skipped};
pub use describe::{DescribeOptions, Description, describe};
pub use listing::listing;
pub use spec::{Spec, SpecPackage, SpecSong};
pub use ultrastar::{
    UltraStarRefusal, UltraStarSource, is_ultrastar_candidate, read_ultrastar, ultrastar_naming,
};

/// The moment now, as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Hand-rolled rather than pulling `chrono` or `time` in for one line, which is the trade
/// `km-package-builder` already made — this is that function, moved here so the two things that
/// stamp a package's `created` share one. Civil-from-days, the standard algorithm, with the epoch
/// shifted to 0000-03-01.
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (hour, minute, second) = (time / 3600, (time % 3600) / 60, time % 60);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Why a file was not included in a package.
///
/// Shared so the curation tool can show the same taxonomy while browsing that `km-pack build`
/// prints after a run — a file refused at build time should be explainable before it is selected.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rejection {
    /// The file could not be read at all.
    Unreadable,
    /// The bytes are not a MIDI file.
    NotMidi,
    /// No lyrics were found, and lyrics were required.
    NoLyrics,
    /// The suitability was below the configured minimum.
    LowSuitability(u8),
    /// Byte-identical to a file already accepted, under the given number.
    DuplicateOf(u32),
    /// No song number could be assigned.
    NoNumber,
    /// An UltraStar file this project does not package, with the reason.
    UltraStar(String),
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => write!(f, "could not be read"),
            Self::NotMidi => write!(f, "not a readable MIDI file"),
            Self::NoLyrics => write!(f, "no lyrics"),
            Self::LowSuitability(value) => write!(f, "rated {value}/10"),
            Self::DuplicateOf(number) => write!(f, "identical to song {number}"),
            Self::NoNumber => write!(
                f,
                "no song number available (they run 1 to {})",
                km_songcode::MAX_SLOT
            ),
            Self::UltraStar(reason) => write!(f, "{reason}"),
        }
    }
}

/// The parts of a [`SongEntry`] a caller chooses, as opposed to the parts analysis dictates.
///
/// Split out because the two callers decide them very differently: `km-pack build` derives a title
/// from the file's metadata or its name, while the curation tool takes whatever a person typed.
#[derive(Debug, Clone)]
pub struct ChosenFields {
    /// The queueing number.
    pub number: u32,
    /// The title to store.
    pub title: String,
    /// The performer, if one is known.
    pub artist: Option<String>,
    /// Path of the MIDI file inside the archive.
    pub file: String,
    /// The language to file the song under, when the caller already knows one.
    ///
    /// `None` means "use what the file says", which is what [`entry_from_analysis`] then detects.
    /// Only `km-pack build --index` sets it, from a CSV cell; the curation tool leaves it `None` and
    /// applies a person's choice through [`Edits`] instead, so that the choice is recorded as a
    /// correction of detection rather than silently replacing it.
    pub language: Option<Language>,
    /// Encoding to record, so playback decodes the way analysis did.
    pub lyric_encoding: Option<String>,
}

/// Builds the manifest entry for a song from its analysis.
///
/// This is the single place an [`Analysis`] is converted into the manifest's record types. The
/// conversion is lossy on purpose — the manifest stores signal and warning names as strings so an
/// unfamiliar one from a later build is readable rather than a parse failure — and doing it in two
/// places would let the two spellings drift.
///
/// `content_hash` is deliberately left `None`: [`PackageBuilder::add`] computes and sets it from the
/// bytes it is given, which is the only version that cannot disagree with the file.
pub fn entry_from_analysis(song: &Song, analysis: &Analysis, chosen: ChosenFields) -> SongEntry {
    SongEntry {
        number: chosen.number,
        title: chosen.title,
        artist: chosen.artist,
        // What the caller already knew, else what the file says as a code -- see
        // `km_kmpkg::Language::detect`. The encoding is the stronger of the two witnesses and is
        // tried first: a Shift-JIS lyric track is Japanese whatever its `@L` header claims, and that
        // header claims English on most of a real corpus whatever the song is. Detected rather than
        // authoritative, exactly like the title beside it: a person's correction arrives through
        // `apply_edits` and wins. Same shape as `lyric_encoding` below.
        language: chosen
            .language
            .or_else(|| Language::detect(song.meta.language.as_deref(), Some(song.decoder.name())))
            .map(|language| language.code().to_owned()),
        // This function's whole input is a parsed MIDI song and its analysis, so there is nothing
        // else it could be. A video song is built by `entry_from_video`, which shares none of this
        // because a video has no analysis to draw on.
        kind: km_kmpkg::SongKind::Midi,
        file: chosen.file,
        duration_ms: song.duration_ms(),
        lyric_encoding: chosen
            .lyric_encoding
            .or_else(|| Some(song.decoder.name().to_owned())),
        default_transpose: 0,
        // Detected here rather than at playback, on the same terms as the melody channel beside it:
        // the machine reads what a package recorded instead of deriving it again per play. A person
        // who disagrees corrects it through `apply_edits`, and their list wins.
        fixes: km_fixes::automatic(song),
        melody: melody_record(analysis),
        melody_abstained: melody_abstained(analysis),
        suitability: Some(suitability_record(analysis)),
        // Never detected: only a person can judge it, and only the curation tool asks.
        // Detected, like everything above it and unlike the line before. `km_song` owns the rules for
        // what counts as a line worth showing — see `LyricTimeline::preview` and
        // `km_song::looks_like_a_banner`, which exist because a great many files open with the
        // sequencer's advertisement rather than with the song.
        lyric_preview: song.lyrics.preview(LYRIC_PREVIEW_LINES),
        // Hand curation only: nothing about a file says it is rock. `km-pack` fills these from
        // the spec, in `apply_spec`, and detection has no opinion to overwrite them with.
        tags: Vec::new(),
        // A MIDI song is the reference the other two kinds are levelled *to*, so it carries no
        // measurement of its own -- and could not: it has no level until a bank renders it, and
        // which bank that is belongs to the machine playing it. See `SongEntry::loudness`.
        loudness: None,
        content_hash: None,
        edited: Vec::new(),
    }
}

/// How many lines of a song a package carries.
///
/// Two: enough to recognize a song by, and short enough that a four-thousand-song manifest does not
/// turn into a lyric database. `km-lyrics preview` measures against this same number.
pub const LYRIC_PREVIEW_LINES: usize = 2;

/// What packaging knows about a video song.
///
/// Everything here comes from a probe or from a person; there is no analysis, because a video has
/// nothing to analyze. That is why this is a plain struct of chosen values rather than the
/// `(song, analysis, chosen)` shape [`entry_from_analysis`] takes.
///
/// `PartialEq` without `Eq` since [`Self::loudness`] arrived: a measurement is a float, and a float
/// is not `Eq`. Nothing compared these for total equality — the derive was there because everything
/// in the struct happened to allow it.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoFields {
    /// The queueing number.
    pub number: u32,
    /// Title, from the file's metadata tags, its name, or a person.
    pub title: String,
    /// Performer, if anything knows one.
    pub artist: Option<String>,
    /// The language to file the song under, when a person has said one.
    ///
    /// **Only a person can fill this**, which is what makes it different from the title and the
    /// artist beside it: a container has tags for those and none for the language of the singing.
    /// `None` leaves it for `settle_languages` and the package's `default_language`.
    pub language: Option<String>,
    /// What to file the song under, from the description. Nothing in a container says it.
    pub tags: Vec<String>,
    /// The media file's name inside the package's media folder.
    pub file: String,
    /// Length in milliseconds, from the probe.
    pub duration_ms: u32,
    /// How loud the audio is, when it was measured.
    ///
    /// `None` where measuring was skipped or came back with nothing to report — a build run with
    /// `--no-loudness`, or a file with less audio in it than R128 integrates over. The song then
    /// plays unlevelled, exactly as every package built before this field does.
    pub loudness: Option<km_kmpkg::LoudnessRecord>,
    /// Hash of the media file's bytes, for spotting the same video under two numbers.
    pub content_hash: Option<String>,
}

/// Turns a probed video into a manifest entry.
///
/// **The suitability score is a flat 10**, by what the file is rather than by measurement — see
/// [`SuitabilityRecord::purpose_made`]. A karaoke video was manufactured to be sung to; there is
/// nothing in doubt for a number to resolve. Carrying none at all is the tempting alternative,
/// and it is right that the four measured things are MIDI facts and wrong that the answer is
/// therefore unknown: it sorts professionally produced karaoke below mediocre MIDI.
///
/// `kind` is set here and not taken from the caller, so a video entry cannot be built that claims to
/// be a MIDI one.
pub fn entry_from_video(fields: VideoFields) -> SongEntry {
    SongEntry {
        number: fields.number,
        title: fields.title,
        artist: fields.artist,
        // Nothing in a container tells us the language of the singing, and guessing from a title
        // would be worse than saying nothing — so this is whatever a *person* said and nothing
        // else. `None` is left for `settle_languages` and the package default.
        language: fields.language,
        kind: km_kmpkg::SongKind::Video,
        file: fields.file,
        duration_ms: fields.duration_ms,
        // All five of these are MIDI facts. A video has no lyric bytes to decode, no key to shift
        // and no channel to mute.
        lyric_encoding: None,
        default_transpose: 0,
        // Nothing here has MIDI events, so there is nothing a fix could correct.
        fixes: Vec::new(),
        melody: None,
        melody_abstained: None,
        suitability: Some(km_kmpkg::SuitabilityRecord::purpose_made()),
        // Six now, and this is the one that is a statement rather than an absence: a video's words
        // are **pixels in somebody else's picture**, so there is no text to take two lines of. The
        // same reasoning as the `Searching a video's words` decision in docs/decisions/.
        lyric_preview: Vec::new(),
        // From the description alone -- nothing in a container or a subcode stream says what a
        // song is filed under. Folded and sorted so the manifest holds slugs.
        tags: fields.tags,
        // Measured, and the reason this kind carries a level where a MIDI song does not: the audio
        // is a finished rendering, so how loud it is is a fact about the file rather than about
        // whatever will play it.
        loudness: fields.loudness,
        content_hash: fields.content_hash,
        edited: Vec::new(),
    }
}

/// Extensions treated as video songs, and whether a path is one.
///
/// Re-exported from `km-kmpkg`, which is where all three crates that ask can reach it: packaging
/// walks a folder with it, and `km-app` tells a loose `.mp4` from a `.kar` with it. See the note
/// there for why it is not defined here.
pub use km_kmpkg::{VIDEO_EXTENSIONS, is_video_file as is_video};

/// The MP3+G equivalents, re-exported for the same reason.
pub use km_kmpkg::{
    AUDIO_EXTENSIONS, GRAPHICS_EXTENSION, is_audio_file as is_audio,
    is_graphics_file as is_graphics, pair_for,
};

/// The two files that make up one MP3+G song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdgPair {
    /// The MP3.
    pub audio: PathBuf,
    /// The `.cdg` beside it.
    pub graphics: PathBuf,
}

/// A file that is half of an MP3+G song and has no other half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdgOrphan {
    /// An MP3 with no `.cdg`. Playable audio, but no words: not a song.
    NoGraphics,
    /// A `.cdg` with no MP3. Words, and nothing to sing them over.
    NoAudio,
}

impl CdgOrphan {
    /// What to tell somebody about it.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::NoGraphics => "no .cdg beside it, so it has no words",
            Self::NoAudio => "no audio beside it, so there is nothing to sing over",
        }
    }
}

/// Collects every MP3+G pair under a directory, recursively, and every half-pair.
///
/// **Orphans are returned rather than dropped**, because a build that silently skipped them would be
/// a build nobody could reconcile against the folder they pointed it at. Measured over one real
/// starter kit: 2,849 `.cdg` files, of which 2,847 pair and two do not, plus five MP3s with no
/// graphics — eight files that would otherwise vanish without a word.
///
/// Pairing is by [`pair_for`], which is deliberately tolerant: the same corpus has extensions in
/// both cases inside one folder and one pair whose stems differ only by a trailing space.
pub fn collect_cdg(dir: &Path, pairs: &mut Vec<CdgPair>, orphans: &mut Vec<(PathBuf, CdgOrphan)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cdg(&path, pairs, orphans);
        } else if is_audio(&path) {
            // Walked from the audio side, so each pair is seen once: the `.cdg` is found from the
            // MP3 and never the other way round. A `.cdg` is only looked at on its own account when
            // nothing claimed it — see below.
            match pair_for(&path) {
                Some(graphics) => pairs.push(CdgPair {
                    audio: path,
                    graphics,
                }),
                None => orphans.push((path, CdgOrphan::NoGraphics)),
            }
        } else if is_graphics(&path) && pair_for(&path).is_none() {
            orphans.push((path, CdgOrphan::NoAudio));
        }
    }
}

/// Collects every file that is half of an MP3+G song, recursively.
///
/// Both halves, unlike [`collect_cdg`] which pairs them: a caller that indexes files rather than
/// songs — the curation tool's scan — needs a row for each, so that what is in the folder and what
/// is in the tool can be reconciled. Pairing them into songs is then that caller's business.
pub fn collect_cdg_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cdg_paths(&path, out);
        } else if is_audio(&path) || is_graphics(&path) {
            out.push(path);
        }
    }
}

/// The fields a manifest entry for an MP3+G song is built from.
///
/// `PartialEq` without `Eq`, for the reason [`VideoFields`] carries.
#[derive(Debug, Clone, PartialEq)]
pub struct CdgFields {
    /// The queueing number.
    pub number: u32,
    /// Title, from the file's name, its tags, or a person.
    pub title: String,
    /// Performer, if anything knows one.
    pub artist: Option<String>,
    /// The language to file the song under, when a person has said one.
    ///
    /// See [`VideoFields::language`]: CD+G carries even less to go on than a video container, so a
    /// person is the only source there has ever been.
    pub language: Option<String>,
    /// What to file the song under, from the description. As for a video, nothing else knows.
    pub tags: Vec<String>,
    /// The **audio** file's name inside the package's media folder. The graphics follow by rule.
    pub file: String,
    /// Length in milliseconds, counted from the audio.
    pub duration_ms: u32,
    /// How loud the audio is, when it was measured. See [`VideoFields::loudness`].
    pub loudness: Option<km_kmpkg::LoudnessRecord>,
    /// Hash of both files together — see [`km_kmpkg::pair_content_hash`].
    pub content_hash: Option<String>,
}

/// Turns a probed MP3+G pair into a manifest entry.
///
/// Scored a flat 10 for the same reason [`entry_from_video`] is: a commercial karaoke disc was made
/// to be sung to. None of the five MIDI facts is set, because a CD+G pair has none of them. Not
/// feature-gated, because nothing about MP3+G is optional.
pub fn entry_from_cdg(fields: CdgFields) -> SongEntry {
    SongEntry {
        number: fields.number,
        title: fields.title,
        artist: fields.artist,
        // CD+G carries no text at all — the words are one-bit tiles, so there is not even a
        // character to have an encoding — and ID3's `TLAN` frame appeared in none of the measured
        // corpus. Somebody has to say, and this is where what they said arrives; `None` falls
        // through to `settle_languages`.
        language: fields.language,
        kind: km_kmpkg::SongKind::Cdg,
        file: fields.file,
        duration_ms: fields.duration_ms,
        lyric_encoding: None,
        default_transpose: 0,
        // Nothing here has MIDI events, so there is nothing a fix could correct.
        fixes: Vec::new(),
        melody: None,
        melody_abstained: None,
        suitability: Some(km_kmpkg::SuitabilityRecord::purpose_made()),
        // Empty for the reason the comment above already gives about the encoding, and it is the
        // stronger version of it: CD+G words are **one-bit tiles**, so there is not a character
        // anywhere in the file to take two lines of.
        lyric_preview: Vec::new(),
        // From the description alone -- nothing in a container or a subcode stream says what a
        // song is filed under. Folded and sorted so the manifest holds slugs.
        tags: fields.tags,
        // Measured, and the reason this kind carries a level where a MIDI song does not: the audio
        // is a finished rendering, so how loud it is is a fact about the file rather than about
        // whatever will play it.
        loudness: fields.loudness,
        content_hash: fields.content_hash,
        edited: Vec::new(),
    }
}

/// Collects every video file under a directory, recursively.
///
/// The counterpart of [`collect_midi`], kept separate because the two are packaged completely
/// differently: one goes inside the archive, the other beside it.
pub fn collect_videos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_videos(&path, out);
        } else if is_video(&path) {
            out.push(path);
        }
    }
}

/// What a caller wants done with one video file.
#[cfg(feature = "video")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoRequest {
    /// The queueing number, which is also the stored file's name.
    pub number: u32,
    /// Title, when a person has given one.
    ///
    /// `None` means *work it out*: the container's own title tag, and the file's stem when there is
    /// no tag either. That chain is resolved in [`add_video_song`] rather than by each caller,
    /// because this is the only place a video enters a package and two callers resolving it
    /// separately is exactly the drift M9 turned this crate into a library to prevent.
    pub title: Option<String>,
    /// Performer, when a person has given one. `None` falls back to the container's artist tag.
    pub artist: Option<String>,
    /// The language of the singing, which only a person can supply.
    ///
    /// **There is no fallback chain behind this one**, unlike the title and the artist: no video
    /// container states it, so `None` means the package's `default_language` will have to answer.
    pub language: Option<String>,
    /// What to file the song under. Only a description can know: no container tag says it.
    pub tags: Vec<String>,
    /// Re-encode a file that is outside the profile.
    ///
    /// When this is false a merely irregular file — VP9, 60 fps, a `.webm` — is stored as it is and
    /// plays perfectly well. An *unplayable* one is refused either way: turning off transcoding is a
    /// statement about spending CPU, not a license to put a file in a package that the machine will
    /// skip at singing time.
    pub transcode: bool,
    /// Measure how loud the audio is, so the machine can level it against a MIDI song.
    ///
    /// **On for every real build**; `km-pack build --no-loudness` is what turns it off, for a
    /// curator iterating on a description who does not want to pay a full audio decode per song
    /// each time. A song built without it plays unlevelled, exactly as one built before the field
    /// existed does.
    pub measure_loudness: bool,
    /// Build the manifest entry but write no media.
    pub dry_run: bool,
}

/// What packaging did with one video file.
#[cfg(feature = "video")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoOutcome {
    /// The source file's probe.
    pub info: km_video::VideoInfo,
    /// How the source fell outside the profile, empty when it did not.
    pub mismatches: Vec<profile::Mismatch>,
    /// Whether it was re-encoded rather than copied.
    pub transcoded: bool,
    /// The name it was stored under inside the media folder.
    pub file: String,
    /// Hash of the source's bytes, which is how the same video is spotted arriving twice.
    ///
    /// The *source*, not the stored file: two copies of one download must be recognized as the same
    /// song, and a re-encode of each would produce two different results the moment the encoder,
    /// its version or its options changed.
    pub source_hash: String,
    /// Why the audio's level could not be measured, when it could not.
    ///
    /// `None` covers both "it was measured" and "there was nothing to measure" — a level is worth
    /// having and is not worth refusing a package over, so a decode that fails leaves the song
    /// unlevelled and puts the reason here for the caller to report. This crate has no logger and
    /// prints nothing itself, which is why the reason travels rather than being logged.
    pub loudness_note: Option<String>,
}

/// What measuring a song's level produced, and any finding about it.
///
/// **A level is worth having and is not worth refusing a package over.** Everything else packaging
/// reads from a file decides whether the song can be played at all; a level only decides how loud it
/// is, and the answer to not having one is that the song plays as every song did before levelling
/// existed. So a decode that fails leaves `record` empty and puts the reason in `note`, which the
/// caller reports — this crate prints nothing itself and has no logger, so a failure that only
/// wrote to one would be a failure nobody sees.
#[derive(Debug, Clone, Default, PartialEq)]
struct Measured {
    /// The measurement, when there is one.
    record: Option<km_kmpkg::LoudnessRecord>,
    /// Why there is not, when a decode failed. `None` for a file that simply had nothing to measure.
    note: Option<String>,
}

impl Measured {
    /// Turns either decoder's answer into one shape.
    fn from<E: std::fmt::Display>(
        measured: std::result::Result<Option<km_loudness::Loudness>, E>,
    ) -> Self {
        match measured {
            Ok(Some(loudness)) => Self {
                record: Some(km_kmpkg::LoudnessRecord {
                    lufs: loudness.lufs,
                    peak_dbtp: loudness.peak_dbtp,
                }),
                note: None,
            },
            // Nothing to report rather than a fault: less audio than R128 integrates over, or
            // silence. Not worth a note — a two-second sting has no level worth writing down and
            // saying so about every one of them would be noise.
            Ok(None) => Self::default(),
            Err(error) => Self {
                record: None,
                note: Some(format!(
                    "how loud it is could not be measured ({error}); it will play unlevelled"
                )),
            },
        }
    }
}

/// Adds one video song to a package, copying or re-encoding its media **into** it.
///
/// The video becomes a stored entry of the archive, seeked into at play time rather than extracted.
/// It used to go into a sibling folder named after the package, because reading an archive entry
/// meant reading the whole thing into memory — which was a fact about one function rather than about
/// zip, and is no longer true of either. See the `Where a song's media lives` decision in
/// `docs/decisions/packaging.md`.
///
/// Nothing is copied here. `scratch` is where a re-encode lands, and either that file or the
/// original is handed to the builder as a **path**; the bytes move once, when the package is
/// written. That is what keeps a build's memory flat whatever it is packaging.
///
/// **This is the only place a video enters a package.** `km-pack build` and the curation tool both
/// come through here, which is the property M9 established for MIDI and the reason that milestone
/// turned the packaging pipeline into a library: two tools writing subtly different manifests from
/// the same file is a defect nobody notices until a package behaves oddly on the machine.
#[cfg(feature = "video")]
pub fn add_video_song(
    builder: &mut PackageBuilder,
    source: &Path,
    scratch: &Path,
    request: &VideoRequest,
    encoders: Option<&profile::Encoders>,
    on_progress: impl FnMut(profile::Progress),
) -> Result<VideoOutcome> {
    let info = km_video::probe(source).with_context(|| format!("probing {}", source.display()))?;
    let mismatches = profile::DEFAULT.check(&info, source);

    // Refused whatever `transcode` says. The alternative is a package holding a song the machine
    // skips when somebody has queued it and is standing at the microphone.
    if profile::is_blocking(&mismatches) && !request.transcode {
        let reasons: Vec<_> = mismatches.iter().map(ToString::to_string).collect();
        anyhow::bail!(
            "{} cannot be played as it is ({}), and re-encoding is switched off",
            source.display(),
            reasons.join("; ")
        );
    }

    let re_encode = !mismatches.is_empty() && request.transcode;
    // A re-encode always produces the profile's container; a copy keeps whatever the source was, so
    // that the stored bytes really are the source's bytes.
    let stored_extension = if re_encode {
        format!(".{}", profile::DEFAULT.container)
    } else {
        extension(source)
    };
    let file = format!("media/{}{stored_extension}", request.number);

    // Streamed rather than read. A video is hundreds of megabytes, and landing the whole thing in
    // a `Vec` purely so it can be hashed undoes the streaming write one function further down.
    let source_hash = km_kmpkg::content_hash_of(source)
        .with_context(|| format!("hashing {}", source.display()))?;

    // What the archive will hold: the re-encoded file, or the source itself. A dry run names the
    // source and never writes, since the builder records a path and reads nothing until `write`.
    let stored = if re_encode && !request.dry_run {
        let encoders = encoders
            .context("re-encoding was asked for but the local ffmpeg was never looked up")?;
        std::fs::create_dir_all(scratch)
            .with_context(|| format!("creating {}", scratch.display()))?;
        // No `.part` and no rename any more: nothing reads this file until `write` streams it into
        // the archive, and the archive's own temp-and-rename is what makes an interrupted build
        // safe.
        let encoded = scratch.join(format!("{}.{}", request.number, profile::DEFAULT.container));
        profile::transcode(
            source,
            &encoded,
            &profile::DEFAULT,
            &info,
            encoders,
            on_progress,
        )?;
        encoded
    } else {
        source.to_path_buf()
    };

    // **Measured on `stored` rather than on `source`, and that is the whole point of doing it
    // here.** What the machine plays is what goes into the archive, and a re-encode is not level
    // with its input: the profile's `-ac 2` turns a mono source into stereo, which moves the
    // measurement by about 3 dB all by itself. Measuring the source would write down a number for a
    // file the package does not contain.
    let measured = if request.measure_loudness {
        Measured::from(km_video::measure_loudness(&stored))
    } else {
        Measured::default()
    };

    // A person's answer first, then what the file says about itself, then its name. The middle step
    // is what makes a downloaded video worth tagging: a container carrying the real title and
    // artist packages with both filled in rather than with a stem and a blank. The stem comes from
    // the **source**, never from a re-encode named after a song number.
    //
    // Through `usable_tag`, the same filter the audio path puts its tags through: a container tag is
    // believed more readily here than an ID3 frame is, and a placeholder is still a placeholder.
    let title = request
        .title
        .clone()
        .or_else(|| usable_tag(info.title.as_deref()))
        .unwrap_or_else(|| file_stem(source));
    let artist = request
        .artist
        .clone()
        .or_else(|| usable_tag(info.artist.as_deref()));

    let entry = entry_from_video(VideoFields {
        number: request.number,
        title,
        artist,
        language: request.language.clone(),
        tags: request.tags.clone(),
        file: file.clone(),
        duration_ms: info.duration_ms,
        loudness: measured.record,
        content_hash: Some(source_hash.clone()),
    });
    // Recorded, not read. `PackageBuilder::write` is what streams it.
    builder.add_video_source(entry, &file, &stored, Some(source_hash.clone()))?;

    Ok(VideoOutcome {
        info,
        mismatches,
        transcoded: re_encode,
        file,
        source_hash,
        loudness_note: measured.note,
    })
}

/// What a caller wants done with one MP3+G pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CdgRequest {
    /// The queueing number to give it.
    pub number: u32,
    /// A person's title, which beats anything read from the files.
    pub title: Option<String>,
    /// A person's artist, likewise.
    pub artist: Option<String>,
    /// A person's language. Nothing in an MP3+G pair says it, so there is nobody else to ask.
    pub language: Option<String>,
    /// What to file the song under. As for a video: only a description can know.
    pub tags: Vec<String>,
    /// Measure how loud the audio is. See [`VideoRequest::measure_loudness`].
    pub measure_loudness: bool,
    /// Work out what would happen without writing anything.
    pub dry_run: bool,
}

/// A finding about a pair that is not a reason to refuse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdgFinding(pub String);

impl std::fmt::Display for CdgFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What packaging one MP3+G pair did.
#[derive(Debug, Clone)]
pub struct CdgOutcome {
    /// What the probe found in both halves.
    pub info: km_cdg::CdgInfo,
    /// Reported, never blocking. See [`add_cdg_song`].
    pub findings: Vec<CdgFinding>,
    /// The audio file's name in the media folder; the graphics are the same stem.
    pub file: String,
    /// The pair's hash.
    pub source_hash: String,
}

/// Adds one MP3+G song to a package, copying both its files beside it.
///
/// **This is the only place an MP3+G song enters a package**, the property M9 established for MIDI
/// and M11 for video: two tools writing subtly different manifests from the same files is a defect
/// nobody notices until a package behaves oddly on the machine.
///
/// # Nothing is transcoded, and the check is two lines long
///
/// There is no packaging profile here, which is a deliberate departure from the video path rather
/// than an omission. That profile exists because `km-video` copies three planes and carries no
/// swscale, so a pixel format it cannot read genuinely blocks; nothing here has an equivalent limit,
/// since `symphonia` decodes any Layer III at any rate and `TrackPlayer` already resamples. So there
/// is nothing a re-encode could fix, and lossy-to-lossy would be a pure loss.
///
/// A song is refused for exactly two reasons: **the audio will not decode**, or **the graphics draw
/// no tile at all** — a CD+G with no words in it is an MP3. Everything else is reported and allowed,
/// and that list is short because the corpus made it short. Every intuitive damage signal was tried
/// against 2,849 real files and every one of them was wrong: a command byte that is not 9 is another
/// subcode application, an unimplemented instruction is a manufacturer's extension, and an
/// off-screen tile is real damage that nobody can see. A check built on any of them would have
/// refused songs that play perfectly.
pub fn add_cdg_song(
    builder: &mut PackageBuilder,
    pair: &CdgPair,
    request: &CdgRequest,
) -> Result<CdgOutcome> {
    let info = km_cdg::probe(&pair.audio, &pair.graphics)
        .with_context(|| format!("probing {}", pair.audio.display()))?;

    if info.graphics.tiles_written == 0 {
        anyhow::bail!(
            "{} never draws a tile, so the song has no words in it",
            pair.graphics.display()
        );
    }

    let mut findings = Vec::new();
    if info.graphics.trailing_bytes > 0 {
        findings.push(CdgFinding(format!(
            "{} trailing bytes are not a whole packet and were ignored",
            info.graphics.trailing_bytes
        )));
    }
    if info.graphics.unknown_instructions > 0 {
        findings.push(CdgFinding(format!(
            "{} of {} packets carry a CD+G instruction this build does not implement (harmless: \
             every measured file like this renders)",
            info.graphics.unknown_instructions, info.graphics.packets
        )));
    }
    if info.graphics.offscreen_tiles > 0 {
        findings.push(CdgFinding(format!(
            "{} tiles are addressed off the screen and were dropped",
            info.graphics.offscreen_tiles
        )));
    }
    // The one finding that suggests the *pair* is wrong rather than a file being scruffy. Seconds
    // short is ordinary — the words end before the outro — and a minute is not.
    let short_by = info.graphics_short_by_ms();
    if short_by > MISPAIR_GAP_MS {
        findings.push(CdgFinding(format!(
            "the graphics stop {:.0}s before the audio ends, which usually means the .cdg belongs \
             to a different song",
            f64::from(short_by) / 1000.0
        )));
    }

    // The audio goes into the package as it is — nothing re-encodes an MP3+G pair — so unlike a
    // video there is no distinction here between the source and what is stored.
    let measured = if request.measure_loudness {
        Measured::from(km_cdg::measure_loudness(&pair.audio))
    } else {
        Measured::default()
    };
    if let Some(note) = measured.note {
        // A finding rather than a field of its own: this list is already *what there is to say
        // about a pair that is not a reason to refuse it*, and a level that could not be taken is
        // exactly that.
        findings.push(CdgFinding(note));
    }

    let file = format!("media/{}.{}", request.number, extension_of(&pair.audio));

    // Streamed rather than read. Both halves used to land in memory purely to be hashed, and while
    // five megabytes is survivable where a video is not, doing it here would leave one path in this
    // file reading whole files and one not.
    let source_hash = km_kmpkg::pair_content_hash_of(&pair.audio, &pair.graphics)
        .with_context(|| format!("hashing {}", pair.audio.display()))?;

    // **A person first, then the file's name, then its tags** — and that middle step is the
    // difference from the video rule. A downloaded video's container tags are the best thing about
    // it; a karaoke MP3's are measurably not. Over 2,851 real tracks ID3 is present on about half
    // and, when present, is often wrong: artist and title swapped, titles that are literally
    // `Track  6`. The stem is reliably `Artist - Title`. Tags are still read, filtered, and used
    // where the stem gives nothing.
    let title = request
        .title
        .clone()
        .unwrap_or_else(|| file_stem(&pair.audio))
        .trim()
        .to_owned();
    let title = if title.is_empty() {
        usable_tag(info.audio.title.as_deref()).unwrap_or_else(|| file_stem(&pair.audio))
    } else {
        title
    };
    let artist = request.artist.clone().or_else(|| {
        usable_tag(info.audio.artist.as_deref())
            // An artist tag that is the whole file name is the file name, not an artist. Real and
            // common: in one album of the measured corpus several tracks carry
            // `QUEEN - BICYCLE RACE` in the artist field, which would otherwise be packaged as the
            // performer and shown on the television under the title that already says it.
            .filter(|value| !same_text(value, &title) && !same_text(value, &file_stem(&pair.audio)))
    });

    let entry = entry_from_cdg(CdgFields {
        number: request.number,
        title,
        artist,
        language: request.language.clone(),
        tags: request.tags.clone(),
        file: file.clone(),
        duration_ms: info.audio.duration_ms,
        loudness: measured.record,
        content_hash: Some(source_hash.clone()),
    });
    // Two entries, registered rather than read: the audio under this name and the graphics under the
    // name the rule derives from it. `PackageBuilder::write` streams both. A dry run records them
    // and never writes, since nothing is read until then.
    builder.add_cdg_source(
        entry,
        &file,
        &pair.audio,
        &pair.graphics,
        Some(source_hash.clone()),
    )?;

    Ok(CdgOutcome {
        info,
        findings,
        file,
        source_hash,
    })
}

/// How far the graphics may stop short of the audio before it looks like a mispairing.
///
/// Nine to thirty-seven seconds is ordinary across the measured corpus — the words end before the
/// outro does — and exactly one file of 2,849 trips this. That file is also the only one whose
/// `.cdg` is not a whole number of packets, which is what a truncated graphics file looks like from
/// two directions at once.
const MISPAIR_GAP_MS: u32 = 60_000;

/// A tag worth believing, or nothing.
///
/// Blank is dropped because a blank tag looks like an answer and is not one. `Track 6` is dropped
/// because it is a placeholder a ripper left behind, and it appears verbatim in the measured corpus.
/// A row of marks is dropped by `km_song::clean_meta_name`, which is the same judgement this crate's
/// own two rules make and is shared rather than restated so a tag and a MIDI title cannot come to
/// disagree about what a name is.
fn usable_tag(tag: Option<&str>) -> Option<String> {
    // Trimming and the blank test come with it: `clean_meta_name` collapses whitespace and answers
    // `None` for a tag that is nothing once it has.
    let value = km_song::clean_meta_name(tag?)?;
    let lower = value.to_ascii_lowercase();
    let placeholder = lower.strip_prefix("track").is_some_and(|rest| {
        !rest.trim().is_empty() && rest.trim().chars().all(|c| c.is_ascii_digit())
    });
    (!placeholder).then_some(value)
}

/// Whether two pieces of text say the same thing, ignoring case and runs of whitespace.
///
/// Case matters here because the corpus is inconsistent about it — the same album has `Queen` in one
/// tag and `QUEEN - BICYCLE RACE` in another — and comparing exactly would let half of them through.
fn same_text(left: &str, right: &str) -> bool {
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

/// A path's extension in lower case, without the dot. `mp3` when it somehow has none.
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "mp3".to_owned())
}

/// The melody record for a confidently detected channel, or `None` when detection abstained.
pub fn melody_record(analysis: &Analysis) -> Option<MelodyRecord> {
    analysis.melody.channel().map(|melody| MelodyRecord {
        channel: melody.channel,
        confidence: melody.confidence,
        signals: melody
            .signals
            .iter()
            .map(|signal| format!("{signal:?}").to_lowercase())
            .collect(),
    })
}

/// Why no melody channel was claimed, or `None` when one was.
pub fn melody_abstained(analysis: &Analysis) -> Option<String> {
    match &analysis.melody {
        MelodyOutcome::Abstained { abstained } => Some(format!("{abstained:?}").to_lowercase()),
        MelodyOutcome::Found(_) => None,
    }
}

/// The suitability record: the value, its breakdown, and every warning.
pub fn suitability_record(analysis: &Analysis) -> SuitabilityRecord {
    SuitabilityRecord {
        value: analysis.suitability.value,
        breakdown: BreakdownRecord {
            lyrics: analysis.suitability.breakdown.lyrics,
            sync: analysis.suitability.breakdown.sync,
            channels: analysis.suitability.breakdown.channels,
            arrangement: analysis.suitability.breakdown.arrangement,
        },
        warnings: analysis
            .suitability
            .warnings
            .iter()
            .map(|warning| WarningRecord {
                code: warning_code(warning.code),
                message: warning.message.clone(),
            })
            .collect(),
    }
}

/// The manifest spelling of a warning code.
pub fn warning_code(code: km_suitability::WarningCode) -> String {
    format!("{code:?}").to_lowercase()
}

/// One song's new values, as read from an edited CSV or a form.
///
/// The nested `Option` is not an accident. The outer one is "was this field mentioned at all"; the
/// inner one is "should it hold a value or be cleared". Conflating them would make it impossible to
/// remove a wrong artist.
#[derive(Debug, Default, Clone)]
pub struct Edits {
    /// A new title. Ignored when blank — a song with no title is worse than a wrong one.
    pub title: Option<String>,
    /// A new performer, or `Some(None)` to clear it.
    pub artist: Option<Option<String>>,
    /// A new language code, or `Some(None)` to clear it.
    pub language: Option<Option<String>>,
    /// The tags to file the song under. `Some(vec![])` clears them.
    ///
    /// **A `Vec` where its neighbours are `Option<Option<_>>`**, and the difference is that a tag
    /// list has no third state: empty *is* none, so there is nothing for an inner `Option` to mean.
    /// It is also the one field here with no detected counterpart, which makes any non-empty value
    /// an edit by construction — nothing can have proposed it.
    pub tags: Option<Vec<String>>,
    /// A new lyric encoding, or `Some(None)` to clear it.
    pub encoding: Option<Option<String>>,
    /// A new default transposition.
    pub transpose: Option<i8>,
    /// The corrections in force, replacing whatever detection proposed.
    ///
    /// `Some(vec![])` is a decision and not an absence: it says the detector's proposal is unwanted,
    /// which is the only way to turn off a fix that applies itself.
    pub fixes: Option<Vec<km_fixes::Fix>>,
    /// The melody channel somebody named, replacing whatever detection found.
    ///
    /// Three states, and the middle one is what earns the outer `Option`: `None` leaves detection's
    /// answer alone, `Some(None)` says the song has no melody channel, and `Some(Some(channel))`
    /// names one. The machine offers its guide-melody toggle only where a song has a melody
    /// channel, so both of the inner states change what a singer is offered.
    pub melody: Option<Option<u8>>,
}

/// One cell's worth of a song's fix list, for the export and for `inspect`.
///
/// Names and channels, comma-joined — enough to see which songs of a package are corrected and
/// where, and deliberately not enough to reconstruct the list. A fix carries arguments a cell cannot
/// carry back, so this is read by a person and never by `read_edits`.
pub fn describe_fixes(fixes: &[km_fixes::Fix]) -> String {
    fixes
        .iter()
        .map(|fix| match fix.channel() {
            Some(channel) => format!("{}:{channel}", fix.key()),
            None => fix.key().to_owned(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The signal a melody record carries when a person named the channel rather than a detector.
///
/// A name rather than a number, because the confidence beside it is the detector's unit and a
/// judgement has none: it stands at 1.0 and this is what says why. `signals` is typed as strings for
/// exactly this, so a build that has never heard of it reads it rather than failing.
pub const MELODY_CHOSEN_SIGNAL: &str = "chosen";

/// Applies one set of edits, returning how many fields actually changed.
///
/// Only fields whose value differs are marked as edited. Re-applying an unchanged export must not
/// mark the whole catalog as hand-edited, or the flag stops meaning anything.
pub fn apply_edits(entry: &mut SongEntry, edit: &Edits, original: &SongEntry) -> usize {
    let mut changed = 0;

    if let Some(title) = &edit.title
        && *title != original.title
        && !title.trim().is_empty()
    {
        entry.title = title.clone();
        entry.mark_edited(EditedField::Title);
        changed += 1;
    }
    if let Some(artist) = &edit.artist
        && *artist != original.artist
    {
        entry.artist = artist.clone();
        entry.mark_edited(EditedField::Artist);
        changed += 1;
    }
    if let Some(language) = &edit.language
        && *language != original.language
    {
        entry.language = language.clone();
        entry.mark_edited(EditedField::Language);
        changed += 1;
    }
    // Folded and sorted here rather than trusted, so the manifest holds slugs however the
    // description spelled them — and so an unchanged rebuild compares equal and marks nothing.
    if let Some(tags) = &edit.tags {
        let tags: Vec<String> = km_kmpkg::tag::parse_list(&tags.join(","))
            .into_iter()
            .map(km_kmpkg::Tag::into_string)
            .collect();
        if tags != original.tags {
            entry.tags = tags;
            entry.mark_edited(EditedField::Tags);
            changed += 1;
        }
    }
    if let Some(encoding) = &edit.encoding
        && *encoding != original.lyric_encoding
    {
        entry.lyric_encoding = encoding.clone();
        entry.mark_edited(EditedField::LyricEncoding);
        changed += 1;
    }
    if let Some(transpose) = edit.transpose
        && transpose != original.default_transpose
    {
        entry.default_transpose = transpose;
        entry.mark_edited(EditedField::DefaultTranspose);
        changed += 1;
    }
    if let Some(fixes) = &edit.fixes
        && *fixes != original.fixes
    {
        entry.fixes = fixes.clone();
        entry.mark_edited(EditedField::Fixes);
        changed += 1;
    }
    // Compared as a channel rather than as a record, because the record carries the detector's
    // confidence and signals: a person names a channel and has none of the rest to offer, so
    // comparing whole records would mark every song whose melody somebody merely confirmed.
    if let Some(melody) = edit.melody {
        let was = original.melody.as_ref().map(|record| record.channel);
        if melody != was {
            // A named channel keeps the evidence the detector gathered where it is the same
            // channel, and otherwise carries the one signal that says where it came from. Nothing
            // here can measure a part the detector did not pick, and `signals` is the field built
            // to carry a name a reader may not know rather than a number nobody measured.
            entry.melody = melody.map(|channel| match &original.melody {
                Some(record) if record.channel == channel => record.clone(),
                _ => km_kmpkg::MelodyRecord {
                    channel,
                    confidence: 1.0,
                    signals: vec![MELODY_CHOSEN_SIGNAL.to_owned()],
                },
            });
            // *No melody* said by a person is not detection giving up, and the field that says why
            // it gave up would otherwise go on explaining a silence that is now somebody's answer.
            if melody.is_none() {
                entry.melody_abstained = None;
            }
            entry.mark_edited(EditedField::Melody);
            changed += 1;
        }
    }
    changed
}

/// An override read from a CSV index: what a person wants a given source file to become.
#[derive(Debug, Default, Clone)]
pub struct Override {
    /// The song number to use instead of the next free one.
    pub number: Option<u32>,
    /// The title to use instead of the detected one.
    pub title: Option<String>,
    /// The performer to use instead of the detected one.
    pub artist: Option<String>,
    /// The language to file the song under, overriding what the file says.
    ///
    /// Here because `km-pack build` refuses a song with no language, so an index that could not name
    /// one left `build --index` unable to package a folder the detection could not classify — the
    /// only route would have been build, export, edit, apply, which is three passes over a package
    /// the index exists to avoid. Validated when the index is read, so a typo is reported against
    /// its row rather than stored.
    pub language: Option<Language>,
    /// What to file the song under, from the index's `tags` cell — comma-joined inside that cell.
    ///
    /// Here for the language's reason read the other way round. That one is here because a build
    /// *refuses* a song without it; this one is here because nothing anywhere can detect it, so an
    /// index is the only way a folder walk can produce a tagged package at all.
    pub tags: Vec<String>,
    /// The encoding to decode the lyrics with.
    pub encoding: Option<String>,
}

/// Writes a rebuilt package, replacing the original in place unless told otherwise.
///
/// In-place goes via a temporary file and a rename, so an interrupted write cannot leave a truncated
/// package where a working one was. Returns the path actually written.
pub fn write_package(
    builder: PackageBuilder,
    original: &Path,
    out: Option<&Path>,
) -> Result<PathBuf> {
    match out {
        Some(path) => {
            builder.write(path)?;
            Ok(path.to_path_buf())
        }
        None => {
            let temporary = original.with_extension("kmpkg.tmp");
            builder.write(&temporary)?;
            std::fs::rename(&temporary, original).with_context(|| {
                format!(
                    "replacing {} with {}",
                    original.display(),
                    temporary.display()
                )
            })?;
            Ok(original.to_path_buf())
        }
    }
}

/// A read of the editable CSV: the corrections, and the language cells that were not codes.
///
/// Two fields rather than a failure, because a spreadsheet is edited by hand at scale. A 4,000-row
/// export with three typos in it should apply 3,997 corrections and name the three; refusing the
/// file over one cell means somebody edits 4,000 rows again. This is the same argument `km-pack
/// apply` already makes about numbers that match no song.
#[derive(Debug, Default)]
pub struct EditsFile {
    /// The corrections, keyed by song number.
    pub edits: BTreeMap<u32, Edits>,
    /// Rows whose `language` cell was not an ISO 639-1 code: the song number and what was written.
    ///
    /// Those rows keep every other correction they carried; only the language is dropped.
    pub bad_language: Vec<(u32, String)>,
}

/// Reads the editable CSV, keyed by song number.
///
/// A column absent from the file means "do not touch"; a column present but empty means "clear it".
pub fn read_edits(path: &Path) -> Result<EditsFile> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|header| header.trim().eq_ignore_ascii_case(name))
    };

    let number_column = column("number").context("the CSV needs a `number` column")?;
    let title_column = column("title");
    let artist_column = column("artist");
    let language_column = column("language");
    let tags_column = column("tags");
    let encoding_column = column("encoding");
    let transpose_column = column("transpose");

    let mut edits = BTreeMap::new();
    let mut bad_language = Vec::new();
    for record in reader.records() {
        let record = record?;
        let Some(number) = record
            .get(number_column)
            .and_then(|value| value.trim().parse::<u32>().ok())
        else {
            continue;
        };
        // Present-and-empty is a deliberate clear; absent is "leave alone".
        let optional = |index: Option<usize>| {
            index.and_then(|i| record.get(i)).map(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            })
        };
        // Canonicalised on the way in, so `PT` in a spreadsheet becomes `pt` in the package and the
        // column stays exactly matchable. A cell that is not a code is dropped and reported --
        // `Some(None)` would *clear* the language, which is not what somebody who typed
        // "Portuguese" meant, and is worse than leaving it alone.
        let language = match optional(language_column) {
            Some(Some(raw)) => match Language::parse(&raw) {
                Some(language) => Some(Some(language.code().to_owned())),
                None => {
                    bad_language.push((number, raw));
                    None
                }
            },
            other => other,
        };
        edits.insert(
            number,
            Edits {
                title: title_column
                    .and_then(|i| record.get(i))
                    .map(|value| value.trim().to_owned()),
                artist: optional(artist_column),
                language,
                // **Absent when the column is not there, present-and-empty when the cell is.** An
                // export edited in a spreadsheet with the column deleted must not strip every tag
                // in the package; a cell somebody emptied on purpose must. That distinction is why
                // this is an `Option<Vec>` and not a bare `Vec` — see `Edits::tags`. Unreadable
                // words are dropped here rather than refused, unlike `--index`: this file is a
                // round trip of a package and its rows were not typed from nothing.
                tags: tags_column.and_then(|i| record.get(i)).map(|value| {
                    km_kmpkg::tag::parse_list(value)
                        .into_iter()
                        .map(km_kmpkg::Tag::into_string)
                        .collect()
                }),
                encoding: optional(encoding_column),
                transpose: transpose_column
                    .and_then(|i| record.get(i))
                    .and_then(|value| value.trim().parse().ok()),
                // Never read back. The export writes a fix list as names and channels, which is a
                // summary rather than a value — see `describe_fixes`. Always absent here means an
                // export edited in a spreadsheet leaves the corrections exactly as they were.
                fixes: None,
                // Not a column either, and for the reason above: the export summarizes a melody
                // record and cannot carry one back.
                melody: None,
            },
        );
    }
    Ok(EditsFile {
        edits,
        bad_language,
    })
}

/// Reads the CSV index into overrides keyed by the path relative to the scanned folder.
pub fn read_index(path: &Path) -> Result<BTreeMap<String, Override>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|header| header.eq_ignore_ascii_case(name))
    };

    let file_column = column("file").context("the index needs a `file` column")?;
    let number_column = column("number");
    let title_column = column("title");
    let artist_column = column("artist");
    let language_column = column("language");
    let tags_column = column("tags");
    let encoding_column = column("encoding");

    let mut overrides = BTreeMap::new();
    for (row, record) in reader.records().enumerate() {
        let record = record?;
        let Some(file) = record.get(file_column) else {
            continue;
        };
        let value = |index: Option<usize>| {
            index
                .and_then(|i| record.get(i))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        };
        // Refused rather than stored, and refused here rather than at the end: an index is written
        // by hand, and a language that is not a code would otherwise reach the manifest and make the
        // column unfilterable for one song out of a thousand -- the failure that is hardest to
        // notice. `row + 2` counts the header, so the number matches what a spreadsheet shows.
        let language = match value(language_column) {
            Some(raw) => Some(Language::parse(&raw).with_context(|| {
                format!("row {}: {raw:?} is not an ISO 639-1 language code", row + 2)
            })?),
            None => None,
        };
        // Comma-joined inside the cell, which is the same spelling every other surface takes — and
        // is what makes one CSV column enough for a field a song has several of. Refused rather
        // than dropped, by the language's argument above: an index is hand-written, and a tag that
        // silently vanished from one song in a thousand is the failure hardest to notice.
        let tags = match value(tags_column) {
            Some(raw) => raw
                .split(',')
                .map(str::trim)
                .filter(|word| !word.is_empty())
                .map(|word| {
                    km_kmpkg::Tag::parse(word)
                        .map(km_kmpkg::Tag::into_string)
                        .with_context(|| format!("row {}: {word:?} is not a tag", row + 2))
                })
                .collect::<Result<Vec<String>>>()?,
            None => Vec::new(),
        };
        // Refused for the same reason as the language beside it. A `.parse().ok()` here turns
        // `1000000` -- or `12o4`, or a number with a thousands separator in it -- into "no number
        // claimed", so the song silently gets the next free one instead, and a hand-written index
        // that is quietly half-ignored is worse than one that stops.
        let number = match value(number_column) {
            Some(raw) => Some(
                raw.parse::<u32>()
                    .ok()
                    .filter(|number| *number != 0 && *number <= u32::from(km_songcode::MAX_SLOT))
                    .with_context(|| {
                        format!(
                            "row {}: {raw:?} is not a song number (they run 1 to {})",
                            row + 2,
                            km_songcode::MAX_SLOT
                        )
                    })?,
            ),
            None => None,
        };
        overrides.insert(
            normalize_key(file),
            Override {
                number,
                title: value(title_column),
                artist: value(artist_column),
                language,
                tags,
                encoding: value(encoding_column),
            },
        );
    }
    Ok(overrides)
}

/// The key a file is looked up by in the index: its path relative to the scanned folder.
pub fn index_key(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalize_key(&relative.display().to_string())
}

/// Separators and case are normalized so an index written on one platform works on another.
pub fn normalize_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Collects every MIDI file under a folder, recursively.
///
/// An unreadable directory is skipped rather than fatal: a corpus of hundreds of thousands of files
/// on a shared drive will contain some, and abandoning the walk over one is the wrong trade.
pub fn collect_midi(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_midi(&path, out);
        } else if is_midi(&path) {
            out.push(path);
        }
    }
}

/// Collects every file that could be a song source — MIDI, video, or either half of MP3+G.
///
/// **One walk, where the obvious spelling is three.** A caller that indexes every kind of song, as
/// the curation tool's scan does, would otherwise call [`collect_midi`], [`collect_videos`] and
/// [`collect_cdg_paths`] in turn; each opens every directory under the root and stats every entry
/// in it, so a large corpus on a spinning disk is walked three times over to answer one
/// question. The three stay as they are, because `km-pack spec` wants one kind at a time and this
/// one's whole point is a small blast radius.
///
/// **The kind test is the union of theirs, term for term**, so a file this collects is a file one
/// of them collects — that is what makes it a substitution rather than a second opinion. Both
/// halves of an MP3+G pair are collected, matching [`collect_cdg_paths`] and not [`collect_cdg`]:
/// the caller indexes files, and a `.cdg` nothing claimed has to be able to say so.
///
/// An unreadable directory is skipped rather than fatal, for the reason [`collect_midi`] gives.
pub fn collect_songs(dir: &Path, out: &mut Vec<PathBuf>) {
    let _ = collect_songs_observed(dir, out, &mut |_| ControlFlow::Continue(()));
}

/// [`collect_songs`], telling `observe` how many files it has found after each folder it finishes.
///
/// **A walk of a large corpus on a spinning disk is minutes**, and a caller showing progress has
/// nothing to show for them without this: the list is not in its hands until the walk returns.
/// `observe` answering [`ControlFlow::Break`] ends the walk where it stands, which is how a caller
/// stops one — and a walk that broke has found only part of the folder, which the returned `Break`
/// says so the caller cannot mistake the list for the whole of it.
pub fn collect_songs_observed(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    observe: &mut dyn FnMut(usize) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return observe(out.len());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `entry.file_type()` comes back with the directory listing on every platform this runs on,
        // where `path.is_dir()` is a fresh stat per entry — the walk's dominant cost on a corpus
        // this size. A symlink is the one case it cannot answer, because it describes the link
        // rather than the target, so that one falls through to the stat and keeps the old
        // behavior of following it.
        let is_dir = match entry.file_type() {
            Ok(kind) if !kind.is_symlink() => kind.is_dir(),
            _ => path.is_dir(),
        };
        if is_dir {
            collect_songs_observed(&path, out, observe)?;
        } else if is_midi(&path)
            || is_video(&path)
            || is_audio(&path)
            || is_graphics(&path)
            || is_ultrastar_candidate(&path)
        {
            out.push(path);
        }
    }
    observe(out.len())
}

/// Whether a path names a file this project treats as a song source.
pub fn is_midi(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| matches!(ext.to_lowercase().as_str(), "mid" | "midi" | "kar"))
}

/// The archive extension to give a song, following its source file.
pub fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| format!(".{}", ext.to_lowercase()))
        .unwrap_or_else(|| ".mid".to_owned())
}

/// The last-resort title for a file with no usable metadata.
pub fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A named language reaches the entry for the two kinds that cannot detect one.
    ///
    /// **This is a regression test for a silent wrong answer, which is why it asserts the boring
    /// thing.** `VideoFields` and `CdgFields` had no `language` at all, so `entry_from_video` and
    /// `entry_from_cdg` wrote `None` however specific the description had been — and
    /// `settle_languages` then filled the package's `default_language` over the top. The build
    /// reported success and the catalog was wrong: a package whose description said `ko` for one
    /// song and `en` for six came out with all seven filed under `und`.
    ///
    /// It bit these two kinds only, and the reason it survived is that the third one does not go
    /// through here: a MIDI song is laid over a fresh parse by `apply_edits`, which always carried
    /// the field. So the one kind anybody tested was the one kind that worked.
    ///
    /// The `None` half is asserted next door in `cdg_tests.rs` — nothing here may *invent* a
    /// language, and a fallback that quietly guessed would be the opposite mistake.
    #[test]
    fn a_named_language_survives_into_a_video_or_mp3_plus_g_entry() {
        let video = entry_from_video(VideoFields {
            number: 3,
            title: "T".to_owned(),
            artist: None,
            language: Some("ko".to_owned()),
            tags: Vec::new(),
            file: "media/3.mp4".to_owned(),
            duration_ms: 273_020,
            loudness: None,
            content_hash: None,
        });
        assert_eq!(video.language.as_deref(), Some("ko"));

        let cdg = entry_from_cdg(CdgFields {
            number: 4,
            title: "T".to_owned(),
            artist: None,
            language: Some("pt".to_owned()),
            tags: Vec::new(),
            file: "media/4.mp3".to_owned(),
            duration_ms: 231_967,
            loudness: None,
            content_hash: None,
        });
        assert_eq!(cdg.language.as_deref(), Some("pt"));
    }

    /// One walk must find exactly what three walks found — no more, and **no less**.
    ///
    /// This is the load-bearing test of the whole substitution, and the asymmetry in what it guards
    /// is worth stating. A file `collect_songs` finds spuriously is a wasted parse. A file it
    /// *misses* is far worse: the curation scan subtracts what it walked from what the database
    /// holds and deletes the difference, so a kind dropped here is a kind silently deleted from
    /// somebody's catalog on the next scan, along with any song left with no copies. The three
    /// collectors stay in the tree for `km-pack spec`, which is what makes this comparison possible
    /// at all rather than merely desirable.
    #[test]
    fn one_walk_finds_exactly_what_three_walks_found() {
        let dir = std::env::temp_dir().join("km-pack-collect-songs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("deep/deeper")).expect("scratch");

        // Every kind, at three depths, with the case and extension variants the corpus actually
        // contains — and files of no kind at all, which must be found by neither.
        for name in [
            "a.kar",
            "b.MID",
            "c.midi",
            "deep/d.mp3",
            "deep/d.cdg",
            "deep/orphan.CDG",
            "deep/deeper/e.mp4",
            "deep/deeper/f.MKV",
            "deep/deeper/notes.txt",
            "cover.jpg",
        ] {
            std::fs::write(dir.join(name), b"x").expect("write");
        }

        let mut three = Vec::new();
        collect_midi(&dir, &mut three);
        collect_videos(&dir, &mut three);
        collect_cdg_paths(&dir, &mut three);
        // A `.txt` is collected as a candidate: only reading it says whether it is an UltraStar song.
        three.push(dir.join("deep/deeper/notes.txt"));
        three.sort();

        let mut one = Vec::new();
        collect_songs(&dir, &mut one);
        one.sort();

        assert_eq!(one, three);
        assert_eq!(one.len(), 9, "the non-song is in neither: {one:?}");

        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    /// A walk says how far it has got while it runs, and stops where it is told to.
    #[test]
    fn a_walk_reports_its_count_and_stops_when_told() {
        let dir = std::env::temp_dir().join("km-pack-collect-observed");
        let _ = std::fs::remove_dir_all(&dir);
        for folder in ["one", "two", "three"] {
            std::fs::create_dir_all(dir.join(folder)).expect("scratch");
            std::fs::write(dir.join(folder).join("a.kar"), b"x").expect("write");
        }

        let mut seen = Vec::new();
        let mut all = Vec::new();
        let whole = collect_songs_observed(&dir, &mut all, &mut |found| {
            seen.push(found);
            ControlFlow::Continue(())
        });
        assert_eq!(whole, ControlFlow::Continue(()));
        assert_eq!(all.len(), 3);
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&3));

        let mut part = Vec::new();
        let stopped = collect_songs_observed(&dir, &mut part, &mut |found| {
            if found >= 1 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert_eq!(stopped, ControlFlow::Break(()));
        assert_eq!(part.len(), 1, "the walk went on past the stop: {part:?}");

        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn index_keys_normalize_separators_and_case() {
        assert_eq!(normalize_key("Midi\\Song.KAR"), "midi/song.kar");
        assert_eq!(normalize_key("midi/song.kar"), "midi/song.kar");
    }

    #[test]
    fn an_index_key_is_relative_to_the_scanned_folder() {
        let root = Path::new("/corpus");
        let path = Path::new("/corpus/Brasil/Song.kar");
        assert_eq!(index_key(root, path), "brasil/song.kar");
    }

    /// An index is written by hand, so a number it cannot honor has to stop rather than be dropped.
    #[test]
    fn an_index_number_out_of_range_names_its_row_instead_of_being_ignored() {
        let dir = std::env::temp_dir().join("km-pack-index-range");
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("index.csv");

        std::fs::write(&path, "file,number\na.kar,1000000\n").expect("write");
        let error = read_index(&path).expect_err("refused");
        let text = format!("{error:#}");
        assert!(text.contains("row 2"), "{text}");
        assert!(text.contains("1000000"), "{text}");

        // Zero and a value that is not a number at all take the same path, where the second used to
        // be silently ignored and the song given the next free number instead.
        std::fs::write(&path, "file,number\na.kar,0\n").expect("write");
        assert!(read_index(&path).is_err());
        std::fs::write(&path, "file,number\na.kar,12o4\n").expect("write");
        assert!(read_index(&path).is_err());

        // And the highest slot a package may hold is still an ordinary row.
        std::fs::write(&path, "file,number\na.kar,999\n").expect("write");
        let overrides = read_index(&path).expect("accepted");
        assert_eq!(
            overrides["a.kar"].number,
            Some(u32::from(km_songcode::MAX_SLOT))
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn midi_extensions_are_recognized_case_insensitively() {
        assert!(is_midi(Path::new("a.mid")));
        assert!(is_midi(Path::new("a.MIDI")));
        assert!(is_midi(Path::new("a.Kar")));
        assert!(!is_midi(Path::new("a.st3")));
        assert!(!is_midi(Path::new("a.txt")));
        assert!(!is_midi(Path::new("no-extension")));
    }

    #[test]
    fn the_archive_extension_follows_the_source_file() {
        assert_eq!(extension(Path::new("song.KAR")), ".kar");
        assert_eq!(extension(Path::new("song.mid")), ".mid");
        assert_eq!(extension(Path::new("song")), ".mid");
    }

    #[test]
    fn rejection_reasons_read_as_sentences() {
        assert_eq!(Rejection::NotMidi.to_string(), "not a readable MIDI file");
        assert_eq!(Rejection::LowSuitability(3).to_string(), "rated 3/10");
        assert_eq!(
            Rejection::DuplicateOf(42).to_string(),
            "identical to song 42"
        );
    }

    #[test]
    fn a_file_stem_is_the_last_resort_title() {
        assert_eq!(file_stem(Path::new("/x/Some Song.kar")), "Some Song");
    }

    #[test]
    fn a_tag_worth_believing_says_something() {
        // The two this filter began with.
        assert_eq!(usable_tag(None), None);
        assert_eq!(usable_tag(Some("   ")), None);
        assert_eq!(usable_tag(Some("Track 6")), None);
        // And a row of marks, which reaches a tag as readily as it reaches a MIDI title. A stem is
        // what a song with one of these is packaged under.
        assert_eq!(usable_tag(Some("====================")), None);
        assert_eq!(usable_tag(Some("<>-<>-<>-<>")), None);
        assert_eq!(usable_tag(Some("???")), None);
        assert_eq!(usable_tag(Some("** 2")), None);
        // A name inside a frame is a name: the marks around it are not what is being judged.
        assert_eq!(
            usable_tag(Some("***** I Love You *****")).as_deref(),
            Some("***** I Love You *****")
        );
        // Cleaning comes with it, so a padded tag is believed for the name inside it.
        assert_eq!(
            usable_tag(Some("Corcovado\u{0}\u{0}")).as_deref(),
            Some("Corcovado")
        );
        assert_eq!(
            usable_tag(Some("Águas de Março")).as_deref(),
            Some("Águas de Março")
        );
    }

    /// The converter is the reason this module exists, so it is pinned against a real fixture
    /// rather than only compiled.
    #[test]
    fn an_entry_carries_the_analysis_it_was_built_from() {
        let bytes = km_song::testing::soft_karaoke();
        let song = Song::parse(&bytes, &km_song::ParseOptions::default()).expect("parse");
        let analysis = Analysis::of(&song);

        let entry = entry_from_analysis(
            &song,
            &analysis,
            ChosenFields {
                number: 4242,
                title: "Chosen Title".to_owned(),
                artist: Some("Chosen Artist".to_owned()),
                language: None,
                file: "midi/4242.kar".to_owned(),
                lyric_encoding: None,
            },
        );

        assert_eq!(entry.number, 4242);
        assert_eq!(entry.title, "Chosen Title");
        assert_eq!(entry.artist.as_deref(), Some("Chosen Artist"));
        // The fixture's header says `@LENGL`, and what lands in the manifest is the *code*. This is
        // the whole point of the change: `ENGL` is not something a filter or a sort can use.
        assert_eq!(entry.language.as_deref(), Some("en"));
        assert_eq!(entry.duration_ms, song.duration_ms());
        // Left for `PackageBuilder::add` to compute from the bytes it is handed.
        assert!(entry.content_hash.is_none());
        assert!(entry.edited.is_empty());
        // The encoding is recorded even when the caller names none, so playback decodes the way
        // analysis did.
        assert_eq!(entry.lyric_encoding.as_deref(), Some(song.decoder.name()));

        let suitability = entry.suitability.expect("a suitability is always recorded");
        assert_eq!(suitability.value, analysis.suitability_value());
        assert_eq!(
            entry.melody.map(|melody| melody.channel),
            analysis.melody_channel()
        );
        // Exactly one of the two melody fields is ever set.
        assert_ne!(
            analysis.melody.is_found(),
            melody_abstained(&analysis).is_some()
        );
    }

    #[test]
    fn warning_codes_are_lowercase_names() {
        assert_eq!(
            warning_code(km_suitability::WarningCode::NoLyrics),
            "nolyrics"
        );
    }

    #[test]
    fn a_caller_that_knows_the_language_beats_what_the_file_says() {
        // `km-pack build --index` naming a language in its CSV. The file still says `@LENGL`; the
        // person who wrote the index knew better.
        let bytes = km_song::testing::soft_karaoke();
        let song = Song::parse(&bytes, &km_song::ParseOptions::default()).expect("parse");
        let analysis = Analysis::of(&song);

        let entry = entry_from_analysis(
            &song,
            &analysis,
            ChosenFields {
                number: 1,
                title: "T".to_owned(),
                artist: None,
                language: Language::parse("pt"),
                file: "midi/1.kar".to_owned(),
                lyric_encoding: None,
            },
        );

        assert_eq!(entry.language.as_deref(), Some("pt"));
        // Not marked edited: the *index* is an input to the build, not a correction of one. A
        // curator's correction goes through `apply_edits`, which is what records provenance.
        assert!(entry.edited.is_empty());
    }

    #[test]
    fn a_typed_language_is_canonicalised_and_recorded_as_a_correction() {
        let bytes = km_song::testing::soft_karaoke();
        let song = Song::parse(&bytes, &km_song::ParseOptions::default()).expect("parse");
        let analysis = Analysis::of(&song);
        let mut entry = entry_from_analysis(
            &song,
            &analysis,
            ChosenFields {
                number: 1,
                title: "T".to_owned(),
                artist: None,
                language: None,
                file: "midi/1.kar".to_owned(),
                lyric_encoding: None,
            },
        );
        let detected = entry.clone();
        assert_eq!(detected.language.as_deref(), Some("en"));

        let edits = Edits {
            language: Some(Some("pt".to_owned())),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 1);
        assert_eq!(entry.language.as_deref(), Some("pt"));
        assert!(entry.is_edited(EditedField::Language));
    }

    /// The kit-bank fixture, as `entry_from_analysis` would record it.
    fn entry_with_a_detected_fix() -> SongEntry {
        let song = km_song::Song::parse(
            &km_song::testing::kit_bank_on_a_melodic_channel(),
            &km_song::ParseOptions::default(),
        )
        .expect("fixture parses");
        let analysis = Analysis::of(&song);
        entry_from_analysis(
            &song,
            &analysis,
            ChosenFields {
                number: 1,
                title: "T".to_owned(),
                artist: None,
                language: None,
                file: "midi/1.kar".to_owned(),
                lyric_encoding: None,
            },
        )
    }

    #[test]
    fn a_build_records_the_fixes_that_apply_themselves() {
        let entry = entry_with_a_detected_fix();
        assert_eq!(
            entry.fixes,
            vec![km_fixes::Fix::IgnoreBankSelect { channel: 4 }]
        );
        // Recorded, not marked: nobody has decided anything, so a later rebuild is free to find
        // something else.
        assert!(!entry.is_edited(EditedField::Fixes));
    }

    #[test]
    fn an_empty_list_is_how_a_detected_fix_is_turned_off() {
        // The case the `Option<Vec<_>>` exists for. Silence means "detect"; an empty list is a
        // decision, and it is the only way to refuse a fix that would otherwise apply itself.
        let mut entry = entry_with_a_detected_fix();
        let detected = entry.clone();

        let edits = Edits {
            fixes: Some(Vec::new()),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 1);
        assert!(entry.fixes.is_empty());
        assert!(entry.is_edited(EditedField::Fixes));
    }

    #[test]
    fn a_hand_added_mute_is_recorded_as_an_edit() {
        let mut entry = entry_with_a_detected_fix();
        let detected = entry.clone();

        let mut wanted = detected.fixes.clone();
        wanted.push(km_fixes::Fix::MuteChannel { channel: 2 });
        let edits = Edits {
            fixes: Some(wanted.clone()),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 1);
        assert_eq!(entry.fixes, wanted);
        assert!(entry.is_edited(EditedField::Fixes));
    }

    #[test]
    fn restating_what_was_detected_is_not_an_edit() {
        // Re-applying an unchanged export must not mark the catalog as hand-edited, or the flag
        // stops meaning anything.
        let mut entry = entry_with_a_detected_fix();
        let detected = entry.clone();

        let edits = Edits {
            fixes: Some(detected.fixes.clone()),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 0);
        assert!(!entry.is_edited(EditedField::Fixes));
    }

    /// A named melody channel reaches the package, and marks the field so a rebuild keeps it.
    ///
    /// The machine offers its guide-melody toggle only on a song that has a melody channel, so both
    /// directions of this change what a singer is offered: naming one where detection abstained
    /// turns the toggle on, and saying there is none turns it off.
    #[test]
    fn a_named_melody_channel_is_recorded_as_an_edit() {
        let mut entry = entry_with_a_detected_fix();
        let detected = entry.clone();

        let edits = Edits {
            melody: Some(Some(4)),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 1);
        assert_eq!(entry.melody.as_ref().map(|record| record.channel), Some(4));
        assert!(entry.is_edited(EditedField::Melody));
        // The confidence beside it is the detector's unit and a judgement has none, so the signal is
        // what says where the answer came from.
        assert_eq!(
            entry
                .melody
                .as_ref()
                .map(|record| record.signals.as_slice()),
            Some([MELODY_CHOSEN_SIGNAL.to_owned()].as_slice())
        );
    }

    /// *No melody* is a decision, and the reason detection gave up stops being the explanation.
    #[test]
    fn saying_a_song_has_no_melody_clears_the_record_and_the_reason() {
        let mut entry = entry_with_a_detected_fix();
        entry.melody = Some(km_kmpkg::MelodyRecord {
            channel: 2,
            confidence: 0.7,
            signals: vec!["range".to_owned()],
        });
        entry.melody_abstained = Some("ambiguous".to_owned());
        let detected = entry.clone();

        let edits = Edits {
            melody: Some(None),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 1);
        assert!(entry.melody.is_none());
        assert!(
            entry.melody_abstained.is_none(),
            "a person's answer is not the detector giving up"
        );
        assert!(entry.is_edited(EditedField::Melody));
    }

    /// Confirming what detection found is not an edit, and keeps the evidence it gathered.
    #[test]
    fn confirming_the_detected_melody_channel_is_not_an_edit() {
        let mut entry = entry_with_a_detected_fix();
        entry.melody = Some(km_kmpkg::MelodyRecord {
            channel: 2,
            confidence: 0.7,
            signals: vec!["range".to_owned()],
        });
        let detected = entry.clone();

        let edits = Edits {
            melody: Some(Some(2)),
            ..Edits::default()
        };
        assert_eq!(apply_edits(&mut entry, &edits, &detected), 0);
        assert!(!entry.is_edited(EditedField::Melody));
        // The detector's own numbers, kept rather than replaced by a judgement that agreed with it.
        assert_eq!(
            entry.melody.as_ref().map(|record| record.confidence),
            Some(0.7)
        );
    }

    /// A rebuild from source keeps the answer, including the one that is an absence.
    ///
    /// `melody` absent means detection abstained *or* somebody said there is none, and only the
    /// marker tells the two apart — so this is the case that would silently come back as a
    /// guide-melody toggle nobody asked for.
    #[test]
    fn a_rebuild_keeps_a_melody_somebody_chose() {
        let mut chosen = entry_with_a_detected_fix();
        chosen.melody = None;
        chosen.melody_abstained = None;
        chosen.mark_edited(EditedField::Melody);

        // What a fresh parse would say about the same bytes.
        let mut rebuilt = entry_with_a_detected_fix();
        rebuilt.melody = Some(km_kmpkg::MelodyRecord {
            channel: 2,
            confidence: 0.7,
            signals: vec!["range".to_owned()],
        });
        rebuilt.inherit_edits_from(&chosen);

        assert!(rebuilt.melody.is_none(), "the answer survives the rebuild");
        assert!(rebuilt.is_edited(EditedField::Melody));
    }

    #[test]
    fn the_export_summarises_a_fix_list_without_pretending_to_carry_it() {
        assert_eq!(
            describe_fixes(&[
                km_fixes::Fix::IgnoreBankSelect { channel: 4 },
                km_fixes::Fix::MuteChannel { channel: 2 },
            ]),
            "ignore_bank_select:4,mute_channel:2"
        );
        assert_eq!(describe_fixes(&[]), "");
    }
}
