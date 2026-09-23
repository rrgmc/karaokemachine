//! UltraStar songs: an MP3, and the `.txt` beside it that names it and times its words.
//!
//! **The `.txt` is the file a song is found from**, because it is the half that names the other: its
//! `#AUDIO` or `#MP3` header says which MP3 is the song, and the stem never does. The package
//! receives the audio and a lyric timeline; the text file itself stays behind. See `Which UltraStar
//! files are songs` and `The machine never reads an UltraStar file` in
//! `docs/decisions/song-sources.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use km_kmpkg::{Language, PackageBuilder, SongEntry};
use km_song::ultrastar::{self, UltraStar, UltraStarError};

/// The extension an UltraStar file has.
pub const ULTRASTAR_EXTENSION: &str = "txt";

/// Extensions a file named as UltraStar audio can have that make the song a video, which is refused.
///
/// Wider than [`km_kmpkg::VIDEO_EXTENSIONS`], which lists what this project packages as a video
/// song: song folders name `.mpg` and `.avi` files too, and those are still not audio.
const VIDEO_AUDIO_NAMES: [&str; 9] = [
    "mp4", "mkv", "webm", "mov", "avi", "mpg", "mpeg", "divx", "flv",
];

/// How far the words may run past the end of the audio before the pairing looks wrong.
///
/// A syllable held over a fade is ordinary. Words still being sung ten seconds after the recording
/// has ended are the words of a different recording.
const PAST_THE_AUDIO_MS: u32 = 10_000;

/// Whether a path has the extension an UltraStar file has.
///
/// **A candidate, not a verdict.** A song folder holds readme files too, and only reading one says
/// which it is: see [`read_ultrastar`].
#[must_use]
pub fn is_ultrastar_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(ULTRASTAR_EXTENSION))
}

/// An UltraStar file that is a song, and the audio it names.
#[derive(Debug, Clone)]
pub struct UltraStarSource {
    /// The `.txt`.
    pub text: PathBuf,
    /// The MP3 its header names, as found on disk.
    pub audio: PathBuf,
    /// What the file says.
    pub song: UltraStar,
}

/// Why a `.txt` is not an UltraStar song this project packages.
#[derive(Debug)]
pub enum UltraStarRefusal {
    /// The file could not be read.
    Unreadable(std::io::Error),
    /// It is a text file, but not an UltraStar one. Not a failure: song folders hold readmes.
    NotUltraStar,
    /// It is an UltraStar file this project does not play.
    Refused(UltraStarError),
    /// The audio it names is a video.
    VideoOnly(String),
    /// The audio it names is not MP3.
    NotMp3(String),
    /// The audio it names is not beside it.
    AudioMissing(String),
}

impl std::fmt::Display for UltraStarRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(error) => write!(f, "could not be read: {error}"),
            Self::NotUltraStar => f.write_str("not an UltraStar file: there is no #TITLE"),
            Self::Refused(error) => write!(f, "{error}"),
            Self::VideoOnly(name) => write!(
                f,
                "names the video {name} as its audio, and a song whose only media is a video is \
                 not packaged as an UltraStar song"
            ),
            Self::NotMp3(name) => write!(f, "names {name} as its audio, which is not MP3"),
            Self::AudioMissing(name) => {
                write!(f, "names {name} as its audio, which is not beside it")
            }
        }
    }
}

impl std::error::Error for UltraStarRefusal {}

/// Reads a `.txt` and finds the audio it names.
///
/// # Errors
///
/// When the file is not an UltraStar song this project packages: see [`UltraStarRefusal`].
pub fn read_ultrastar(text: &Path) -> Result<UltraStarSource, UltraStarRefusal> {
    let bytes = std::fs::read(text).map_err(UltraStarRefusal::Unreadable)?;
    let song = ultrastar::parse(&bytes).map_err(|error| match error {
        UltraStarError::NotUltraStar => UltraStarRefusal::NotUltraStar,
        other => UltraStarRefusal::Refused(other),
    })?;

    let extension = Path::new(&song.audio)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if VIDEO_AUDIO_NAMES.contains(&extension.as_str()) {
        return Err(UltraStarRefusal::VideoOnly(song.audio));
    }
    if !km_kmpkg::AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        return Err(UltraStarRefusal::NotMp3(song.audio));
    }
    let audio = named_file(text, &song.audio)
        .ok_or_else(|| UltraStarRefusal::AudioMissing(song.audio.clone()))?;

    Ok(UltraStarSource {
        text: text.to_path_buf(),
        audio,
        song,
    })
}

/// The file a header names, beside the `.txt`.
///
/// **Matched without regard to case when the exact spelling is not there**, because a song folder
/// copied from Windows onto a Linux disk keeps names whose case the header never had to agree with.
fn named_file(text: &Path, name: &str) -> Option<PathBuf> {
    let folder = text.parent().unwrap_or(Path::new("."));
    let exact = folder.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    let wanted = name.to_lowercase();
    std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| {
            candidate.is_file()
                && candidate
                    .file_name()
                    .and_then(|file| file.to_str())
                    .is_some_and(|file| file.to_lowercase() == wanted)
        })
}

impl UltraStarSource {
    /// The media files on disk this song names: its audio, and the video `#VIDEO` names when that
    /// is beside it.
    ///
    /// **Both belong to the song and neither is a song of its own.** An MP3 with no `.cdg` is not an
    /// MP3+G pair missing its graphics, and a singing game's music video has no words in its picture,
    /// so packaging it as a video song would put a song on the machine that nobody can sing.
    #[must_use]
    pub fn claimed_media(&self) -> Vec<PathBuf> {
        let mut claimed = vec![self.audio.clone()];
        claimed.extend(
            self.song
                .video
                .as_deref()
                .and_then(|video| named_file(&self.text, video)),
        );
        claimed
    }
}

/// The UltraStar file beside a media file that names it as its audio or its video, if there is one.
///
/// For a scan that walks every file: an MP3 with no `.cdg` beside it, or a video, is part of an
/// UltraStar song when one names it. See [`UltraStarSource::claimed_media`].
#[must_use]
pub fn ultrastar_naming(media: &Path) -> Option<PathBuf> {
    let folder = media.parent()?;
    let wanted = media.file_name()?.to_str()?.to_lowercase();
    std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| is_ultrastar_candidate(candidate) && candidate.is_file())
        .find(|candidate| {
            read_ultrastar(candidate).is_ok_and(|source| {
                source.claimed_media().iter().any(|path| {
                    path.file_name()
                        .and_then(|file| file.to_str())
                        .is_some_and(|file| file.to_lowercase() == wanted)
                })
            })
        })
}

/// Collects every UltraStar song under a directory, recursively, with the files that were refused.
///
/// A `.txt` that is not an UltraStar file is neither: it is not a song and not a failure.
pub fn collect_ultrastar(
    dir: &Path,
    songs: &mut Vec<UltraStarSource>,
    refused: &mut Vec<(PathBuf, UltraStarRefusal)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ultrastar(&path, songs, refused);
        } else if is_ultrastar_candidate(&path) {
            match read_ultrastar(&path) {
                Ok(source) => songs.push(source),
                Err(UltraStarRefusal::NotUltraStar) => {}
                Err(refusal) => refused.push((path, refusal)),
            }
        }
    }
}

/// What a caller wants done with one UltraStar song.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UltraStarRequest {
    /// The queueing number to give it.
    pub number: u32,
    /// A person's title, which beats the header's.
    pub title: Option<String>,
    /// A person's artist, likewise.
    pub artist: Option<String>,
    /// A person's language, which beats `#LANGUAGE`.
    pub language: Option<String>,
    /// What to file the song under.
    pub tags: Vec<String>,
    /// Play the song and draw none of its words, where somebody asked for that.
    pub lyrics_hidden: Option<bool>,
    /// Measure how loud the audio is.
    pub measure_loudness: bool,
    /// Work out what would happen without writing anything.
    pub dry_run: bool,
}

/// What packaging one UltraStar song did.
#[derive(Debug, Clone)]
pub struct UltraStarOutcome {
    /// Reported, never blocking.
    pub findings: Vec<String>,
    /// The audio's name inside the package; the timeline follows by rule.
    pub file: String,
    /// The hash of the audio and the `.txt` together.
    pub source_hash: String,
}

/// The fields a manifest entry for an UltraStar song is built from.
#[derive(Debug, Clone)]
pub struct UltraStarFields {
    /// The queueing number.
    pub number: u32,
    /// The title.
    pub title: String,
    /// The performer, where known.
    pub artist: Option<String>,
    /// An ISO 639-1 code, where known.
    pub language: Option<String>,
    /// What to file the song under.
    pub tags: Vec<String>,
    /// The audio's name inside the package.
    pub file: String,
    /// Length in milliseconds, counted from the audio.
    pub duration_ms: u32,
    /// The span from the first sung syllable to the last, in milliseconds.
    ///
    /// **The one thing about an UltraStar song that is measured rather than taken on trust.** A
    /// person timed these words to this recording, so the file says exactly how much of it is sung,
    /// where a video and an MP3+G pair can offer only their own length.
    pub sung_ms: u32,
    /// How loud the audio is, when it was measured.
    pub loudness: Option<km_kmpkg::LoudnessRecord>,
    /// The first lines of the words.
    pub lyric_preview: Vec<String>,
    /// Whether the machine plays the song and draws none of its words.
    ///
    /// **Hand-set only for this kind.** The three faults that answer it without a person are
    /// measured from MIDI events, and an UltraStar song has none — a person timed its words to a
    /// recording, so the file says nothing about whether they are the right words.
    pub lyrics_hidden: bool,
    /// Hash of the audio and the `.txt` together.
    pub content_hash: Option<String>,
}

/// Turns an UltraStar song into a manifest entry.
///
/// Scored under `Suitability, for a song that was made to be sung to`: full marks, because a person
/// timed its words to this recording, and less where there is too little of it sung to be worth
/// choosing. It is the one media kind whose span is read rather than stood in for by its length. No
/// MIDI fact is set, because the song has no MIDI in it.
#[must_use]
pub fn entry_from_ultrastar(fields: UltraStarFields) -> SongEntry {
    let mut entry = SongEntry {
        number: fields.number,
        title: fields.title,
        artist: fields.artist,
        language: fields.language,
        kind: km_kmpkg::SongKind::UltraStar,
        file: fields.file,
        duration_ms: fields.duration_ms,
        // The timeline is stored as UTF-8 text already decoded, so there is nothing left to decode.
        lyric_encoding: None,
        default_transpose: 0,
        lyrics_hidden: fields.lyrics_hidden,
        fixes: Vec::new(),
        melody: None,
        melody_abstained: None,
        suitability: Some(crate::purpose_made_suitability(fields.sung_ms)),
        lyric_preview: crate::preview_for(fields.lyrics_hidden, || fields.lyric_preview),
        tags: fields.tags,
        loudness: fields.loudness,
        content_hash: fields.content_hash,
        edited: Vec::new(),
    };
    // Marked here where a MIDI song is marked in `apply_edits`, because this path does not go
    // through it. Nothing measures the field for this kind, so a package that carried the value
    // without the marker would hand a re-import a silence it could not tell from detection's.
    if entry.lyrics_hidden {
        entry.mark_edited(km_kmpkg::EditedField::LyricsHidden);
    }
    entry
}

/// Adds one UltraStar song to a package: its audio, and the timeline read from its `.txt`.
///
/// **This is the only place an UltraStar song enters a package**, as `add_cdg_song` is for MP3+G.
/// A song is refused when its audio will not decode; everything else about it was settled when the
/// file was read.
pub fn add_ultrastar_song(
    builder: &mut PackageBuilder,
    source: &UltraStarSource,
    request: &UltraStarRequest,
) -> Result<UltraStarOutcome> {
    let info = km_cdg::probe_audio(&source.audio)
        .with_context(|| format!("probing {}", source.audio.display()))?;

    let mut findings = Vec::new();
    let words_end = source
        .song
        .timeline
        .lines
        .last()
        .map_or(0, |line| line.end_tick);
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
    let source_hash = km_kmpkg::pair_content_hash_of(&source.audio, &source.text)
        .with_context(|| format!("hashing {}", source.text.display()))?;

    // **A person first, then the header, then the file's name.** The header is what the song's author
    // wrote about it, which is the reverse of an MP3's tags: an UltraStar file exists to be read by
    // a program, and its title and artist are how a singing game lists it.
    let stem = crate::file_stem(&source.text);
    let title = request
        .title
        .clone()
        .or_else(|| km_song::clean_meta_name(&source.song.title))
        .unwrap_or_else(|| stem.clone());
    let artist = request.artist.clone().or_else(|| {
        source
            .song
            .artist
            .as_deref()
            .and_then(km_song::clean_meta_name)
    });
    let language = request.language.clone().or_else(|| {
        source
            .song
            .language
            .as_deref()
            .and_then(Language::from_declared)
            .map(|language| language.code().to_owned())
    });

    let entry = entry_from_ultrastar(UltraStarFields {
        number: request.number,
        title,
        artist,
        language,
        tags: request.tags.clone(),
        file: file.clone(),
        duration_ms: info.duration_ms,
        // Measured off the timeline rather than taken from `words_end` above, which asks a different
        // question, whether this MP3 is the recording these words were timed to, and answers it with
        // an end rather than a span.
        sung_ms: km_suitability::sung_span_ms(&km_song::recording::song_from_timeline(
            source.song.timeline.clone(),
        )),
        loudness: measured.record,
        lyric_preview: source.song.timeline.preview(crate::LYRIC_PREVIEW_LINES),
        // Nothing detects this for an UltraStar song, so the description is the only voice, and its
        // silence means the words are drawn.
        lyrics_hidden: request.lyrics_hidden.unwrap_or(false),
        content_hash: Some(source_hash.clone()),
    });
    builder.add_timeline_source(
        km_kmpkg::SongKind::UltraStar,
        entry,
        &file,
        &source.audio,
        &source.song.timeline,
        Some(source_hash.clone()),
    )?;

    Ok(UltraStarOutcome {
        findings,
        file,
        source_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder holding `files`, each with the given contents.
    fn folder(tag: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("km-pack-ultrastar-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).expect("write");
        }
        dir
    }

    const SONG: &[u8] =
        b"#TITLE:Song\n#ARTIST:Someone\n#LANGUAGE:Portuguese\n#MP3:Someone - Song.mp3\n#BPM:300\n: 0 4 0 Hel\n: 4 2 0 lo\nE\n";

    #[test]
    fn the_audio_is_found_by_the_header_and_not_by_the_stem() {
        let dir = folder(
            "header",
            &[
                ("notes.txt", SONG),
                ("Someone - Song.mp3", b"x"),
                ("notes.mp3", b"x"),
            ],
        );
        let source = read_ultrastar(&dir.join("notes.txt")).expect("a song");
        assert_eq!(source.audio, dir.join("Someone - Song.mp3"));
        assert_eq!(
            ultrastar_naming(&dir.join("Someone - Song.mp3")),
            Some(dir.join("notes.txt"))
        );
        assert_eq!(ultrastar_naming(&dir.join("notes.mp3")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_named_audio_is_found_whatever_its_case_on_disk() {
        let dir = folder("case", &[("song.txt", SONG), ("SOMEONE - SONG.MP3", b"x")]);
        let source = read_ultrastar(&dir.join("song.txt")).expect("a song");
        assert!(
            source
                .audio
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("someone - song.mp3"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_readme_is_skipped_and_a_song_with_no_mp3_is_refused() {
        let video = b"#TITLE:Song\n#MP3:clip.mpg\n#BPM:300\n: 0 1 0 a\nE\n";
        let ogg = b"#TITLE:Song\n#MP3:song.ogg\n#BPM:300\n: 0 1 0 a\nE\n";
        let missing = b"#TITLE:Song\n#MP3:gone.mp3\n#BPM:300\n: 0 1 0 a\nE\n";
        let dir = folder(
            "refused",
            &[
                ("ReadMe!.txt", b"Thanks for downloading."),
                ("video.txt", video),
                ("ogg.txt", ogg),
                ("missing.txt", missing),
            ],
        );
        let (mut songs, mut refused) = (Vec::new(), Vec::new());
        collect_ultrastar(&dir, &mut songs, &mut refused);
        assert!(songs.is_empty());
        refused.sort_by(|a, b| a.0.cmp(&b.0));
        let reasons: Vec<&str> = refused
            .iter()
            .map(|(_, refusal)| match refusal {
                UltraStarRefusal::VideoOnly(_) => "video",
                UltraStarRefusal::NotMp3(_) => "not mp3",
                UltraStarRefusal::AudioMissing(_) => "missing",
                _ => "other",
            })
            .collect();
        assert_eq!(reasons, ["missing", "not mp3", "video"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An UltraStar song with enough of it sung, which is the ordinary case.
    #[test]
    fn an_ultrastar_entry_scores_ten_and_carries_its_first_lines() {
        let song = ultrastar::parse(SONG).expect("parses");
        let entry = entry_from_ultrastar(UltraStarFields {
            number: 4,
            title: "Song".to_owned(),
            artist: None,
            language: Some("pt".to_owned()),
            tags: Vec::new(),
            lyrics_hidden: false,
            file: "media/4.mp3".to_owned(),
            duration_ms: 210_000,
            sung_ms: 180_000,
            loudness: None,
            lyric_preview: song.timeline.preview(crate::LYRIC_PREVIEW_LINES),
            content_hash: None,
        });
        assert_eq!(entry.kind, km_kmpkg::SongKind::UltraStar);
        assert_eq!(entry.suitability.map(|record| record.value), Some(10));
        assert_eq!(entry.lyric_preview, ["Hello"]);
        assert!(entry.melody.is_none() && entry.fixes.is_empty());
    }

    /// **Timed by a person and still not worth choosing.** The span is what this kind is measured on,
    /// so an UltraStar file holding one line is answered by the words rather than by the recording: a
    /// twenty-minute MP3 with `Hello` timed over its first second is not a karaoke song.
    #[test]
    fn an_ultrastar_song_sung_for_a_moment_loses_the_words_and_their_timing() {
        let song = ultrastar::parse(SONG).expect("parses");
        let entry = entry_from_ultrastar(UltraStarFields {
            number: 4,
            title: "Song".to_owned(),
            artist: None,
            language: Some("pt".to_owned()),
            tags: Vec::new(),
            lyrics_hidden: false,
            file: "media/4.mp3".to_owned(),
            duration_ms: 1_200_000,
            sung_ms: 1_000,
            loudness: None,
            lyric_preview: song.timeline.preview(crate::LYRIC_PREVIEW_LINES),
            content_hash: None,
        });
        let record = entry.suitability.expect("a suitability");
        assert_eq!(record.value, 4);
        assert_eq!(record.breakdown.lyrics, 0);
        assert_eq!(record.breakdown.sync, 0);
        assert_eq!(
            record.warnings.first().map(|w| w.code.as_str()),
            Some(crate::warning_code(km_suitability::WarningCode::BriefSinging).as_str())
        );
    }
}
