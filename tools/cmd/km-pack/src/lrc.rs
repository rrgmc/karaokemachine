//! LRC songs: an MP3, and the `.lrc` beside it with the same stem that times its words.
//!
//! **The `.lrc` is the file a song is found from**, as the `.txt` is for an UltraStar song: an MP3
//! alone has no words. The package receives the audio and a lyric timeline; the `.lrc` itself stays
//! behind. An MP3 that is already an MP3+G pair or an UltraStar song's audio is that song, and the
//! `.lrc` beside it is refused. See `LRC as a song source` in `docs/decisions/song-sources.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use km_kmpkg::{PackageBuilder, SongEntry, SongKind};
use km_song::lrc::{self, Lrc, LrcError};

use crate::ultrastar::{UltraStarFields, UltraStarOutcome, UltraStarRequest};

/// What a caller wants done with one LRC song: the fields an UltraStar song takes.
///
/// `language` is the only source of one, because an LRC file names no language.
pub type LrcRequest = UltraStarRequest;

/// What packaging one LRC song did: what packaging an UltraStar song does.
pub type LrcOutcome = UltraStarOutcome;

/// How far the words may run past the end of the audio before the pairing looks wrong.
///
/// The rule an UltraStar song has, for its reason: words sung long after the recording ends are the
/// words of a different recording.
const PAST_THE_AUDIO_MS: u32 = 10_000;

/// Whether a path has the extension an LRC file has.
#[must_use]
pub fn is_lrc_candidate(path: &Path) -> bool {
    km_kmpkg::is_lrc_file(path)
}

/// An LRC file that is a song, and the audio beside it.
#[derive(Debug, Clone)]
pub struct LrcSource {
    /// The `.lrc`.
    pub lyrics: PathBuf,
    /// The MP3 with the same stem, as found on disk.
    pub audio: PathBuf,
    /// What the file says.
    pub song: Lrc,
}

/// Why an `.lrc` is not an LRC song this project packages.
#[derive(Debug)]
pub enum LrcRefusal {
    /// The file could not be read.
    Unreadable(std::io::Error),
    /// No line has a timestamp and words.
    Refused(LrcError),
    /// There is no MP3 with the same stem beside it.
    AudioMissing,
    /// The MP3 beside it is already another song.
    AudioTaken(&'static str),
}

impl std::fmt::Display for LrcRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(error) => write!(f, "could not be read: {error}"),
            Self::Refused(error) => write!(f, "{error}"),
            Self::AudioMissing => f.write_str("there is no MP3 with the same name beside it"),
            Self::AudioTaken(song) => write!(f, "its MP3 is {song}, which is the song"),
        }
    }
}

impl std::error::Error for LrcRefusal {}

/// Reads an `.lrc` and finds the MP3 with its stem.
///
/// # Errors
///
/// When the file is not an LRC song this project packages: see [`LrcRefusal`].
pub fn read_lrc(lyrics: &Path) -> Result<LrcSource, LrcRefusal> {
    let bytes = std::fs::read(lyrics).map_err(LrcRefusal::Unreadable)?;
    let song = lrc::parse(&bytes).map_err(LrcRefusal::Refused)?;
    let audio = km_kmpkg::sibling_with_extension(lyrics, &km_kmpkg::AUDIO_EXTENSIONS)
        .ok_or(LrcRefusal::AudioMissing)?;
    if km_kmpkg::pair_for(&audio).is_some() {
        return Err(LrcRefusal::AudioTaken("an MP3+G song"));
    }
    if crate::ultrastar_naming(&audio).is_some() {
        return Err(LrcRefusal::AudioTaken("an UltraStar song's audio"));
    }
    Ok(LrcSource {
        lyrics: lyrics.to_path_buf(),
        audio,
        song,
    })
}

/// The LRC file beside an MP3 that makes it an LRC song, if there is one.
///
/// For a scan that walks every file: an MP3 with no `.cdg` beside it is part of an LRC song when an
/// `.lrc` with its stem reads as one.
#[must_use]
pub fn lrc_naming(audio: &Path) -> Option<PathBuf> {
    let lyrics = km_kmpkg::sibling_with_extension(audio, &[km_kmpkg::LRC_EXTENSION])?;
    read_lrc(&lyrics).is_ok().then_some(lyrics)
}

/// Collects every LRC song under a directory, recursively, with the files that were refused.
pub fn collect_lrc(
    dir: &Path,
    songs: &mut Vec<LrcSource>,
    refused: &mut Vec<(PathBuf, LrcRefusal)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lrc(&path, songs, refused);
        } else if is_lrc_candidate(&path) {
            match read_lrc(&path) {
                Ok(source) => songs.push(source),
                Err(refusal) => refused.push((path, refusal)),
            }
        }
    }
}

/// Turns an LRC song into a manifest entry.
///
/// The entry an UltraStar song gets, under its own kind. **The suitability differs**: a line-timed
/// file loses the words' share of it, by [`crate::purpose_made_suitability_for`].
#[must_use]
pub fn entry_from_lrc(
    fields: UltraStarFields,
    granularity: km_song::LyricGranularity,
) -> SongEntry {
    let sung_ms = fields.sung_ms;
    let mut entry = crate::ultrastar::entry_from_ultrastar(fields);
    entry.kind = SongKind::Lrc;
    entry.suitability = Some(crate::purpose_made_suitability_for(sung_ms, granularity));
    entry
}

/// Adds one LRC song to a package: its audio, and the timeline read from its `.lrc`.
///
/// **This is the only place an LRC song enters a package.** A song is refused when its audio will
/// not decode; everything else about it was settled when the file was read.
pub fn add_lrc_song(
    builder: &mut PackageBuilder,
    source: &LrcSource,
    request: &LrcRequest,
) -> Result<LrcOutcome> {
    let info = km_cdg::probe_audio(&source.audio)
        .with_context(|| format!("probing {}", source.audio.display()))?;

    let timeline = &source.song.timeline;
    let mut findings = Vec::new();
    let words_end = timeline.lines.last().map_or(0, |line| line.start_tick);
    if words_end > info.duration_ms.saturating_add(PAST_THE_AUDIO_MS) {
        findings.push(format!(
            "the words run {:.0}s past the end of the audio, which usually means the MP3 is a \
             different recording from the one the words were timed to",
            f64::from(words_end - info.duration_ms) / 1000.0
        ));
    }

    let measured = if request.measure_loudness {
        crate::Measured::from(km_cdg::measure_loudness(&source.audio))
    } else {
        crate::Measured::default()
    };
    if let Some(note) = measured.note {
        findings.push(note);
    }

    let file = format!("media/{}.mp3", request.number);
    let source_hash = km_kmpkg::pair_content_hash_of(&source.audio, &source.lyrics)
        .with_context(|| format!("hashing {}", source.lyrics.display()))?;

    // **A person first, then the tags, then the file's name**, as for an UltraStar file: `[ti:]`
    // and `[ar:]` are what the file's author wrote about the song.
    let stem = crate::file_stem(&source.lyrics);
    let title = request
        .title
        .clone()
        .or_else(|| {
            source
                .song
                .title
                .as_deref()
                .and_then(km_song::clean_meta_name)
        })
        .unwrap_or_else(|| stem.clone());
    let artist = request.artist.clone().or_else(|| {
        source
            .song
            .artist
            .as_deref()
            .and_then(km_song::clean_meta_name)
    });

    let entry = entry_from_lrc(
        UltraStarFields {
            number: request.number,
            title,
            artist,
            language: request.language.clone(),
            tags: request.tags.clone(),
            file: file.clone(),
            duration_ms: info.duration_ms,
            sung_ms: km_suitability::sung_span_ms(&km_song::recording::song_from_timeline(
                timeline.clone(),
            )),
            loudness: measured.record,
            lyric_preview: timeline.preview(crate::LYRIC_PREVIEW_LINES),
            lyrics_hidden: request.lyrics_hidden.unwrap_or(false),
            content_hash: Some(source_hash.clone()),
        },
        timeline.granularity(),
    );
    builder.add_timeline_source(
        SongKind::Lrc,
        entry,
        &file,
        &source.audio,
        timeline,
        Some(source_hash.clone()),
    )?;

    Ok(LrcOutcome {
        findings,
        file,
        source_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &[u8] = b"[ti:Song]\n[ar:Someone]\n[00:01.00]First line\n[00:30.00]Second line\n\
        [01:00.00]Third line\n[01:30.00]Fourth line\n";

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("km-pack-lrc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("folder");
        dir
    }

    #[test]
    fn an_lrc_file_is_a_song_with_the_mp3_of_its_stem() {
        let dir = folder("pair");
        std::fs::write(dir.join("Someone - Song.lrc"), LINES).expect("lyrics");
        assert!(matches!(
            read_lrc(&dir.join("Someone - Song.lrc")),
            Err(LrcRefusal::AudioMissing)
        ));
        std::fs::write(dir.join("Someone - Song.mp3"), b"audio").expect("audio");
        let source = read_lrc(&dir.join("Someone - Song.lrc")).expect("a song");
        assert_eq!(source.audio, dir.join("Someone - Song.mp3"));
        assert_eq!(source.song.title.as_deref(), Some("Song"));
        assert_eq!(
            lrc_naming(&dir.join("Someone - Song.mp3")),
            Some(dir.join("Someone - Song.lrc"))
        );
    }

    #[test]
    fn an_mp3_plus_g_pair_is_that_song_and_its_lrc_is_refused() {
        let dir = folder("taken");
        std::fs::write(dir.join("Song.lrc"), LINES).expect("lyrics");
        std::fs::write(dir.join("Song.mp3"), b"audio").expect("audio");
        std::fs::write(dir.join("Song.cdg"), b"graphics").expect("graphics");
        assert!(matches!(
            read_lrc(&dir.join("Song.lrc")),
            Err(LrcRefusal::AudioTaken(_))
        ));
        assert_eq!(lrc_naming(&dir.join("Song.mp3")), None);
    }

    #[test]
    fn a_line_timed_song_scores_eight_and_says_why() {
        let song = lrc::parse(LINES).expect("parses");
        let fields = UltraStarFields {
            number: 1,
            title: "Song".to_owned(),
            artist: None,
            language: None,
            tags: Vec::new(),
            file: "media/1.mp3".to_owned(),
            duration_ms: 120_000,
            sung_ms: 90_000,
            loudness: None,
            lyric_preview: Vec::new(),
            lyrics_hidden: false,
            content_hash: None,
        };
        let entry = entry_from_lrc(fields.clone(), song.timeline.granularity());
        assert_eq!(entry.kind, SongKind::Lrc);
        let suitability = entry.suitability.expect("scored");
        assert_eq!(suitability.value, 8);
        assert_eq!(suitability.warnings[0].code, "linelevellyrics");

        let entry = entry_from_lrc(fields, km_song::LyricGranularity::SyllableLevel);
        assert_eq!(entry.suitability.expect("scored").value, 10);
    }
}
