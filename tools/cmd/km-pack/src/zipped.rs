//! A zip holding one song.
//!
//! **A zip is a song when it holds the files of exactly one**: an MP3+G pair, an UltraStar `.txt`
//! with the MP3 it names, or an `.lrc` with the MP3 of its stem. These are the songs made of two
//! files, and one zip a song is how they are traded. See `A zip holding one song is a song source`
//! in `docs/decisions/song-sources.md`.
//!
//! Nothing is unpacked. A zip is a stranger's file, so its entry names are compared and shown and
//! never joined to a path, and every read is bounded.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use km_kmpkg::{AUDIO_EXTENSIONS, GRAPHICS_EXTENSION};
use km_song::lrc::Lrc;
use km_song::ultrastar::UltraStar;

use crate::media::{MAX_ZIP_ENTRIES, MAX_ZIPPED_TEXT, ZipMember, entry_name, is_os_junk};
use crate::ultrastar::{UltraStarRefusal, interpret_ultrastar};

/// The extension a zip has.
pub const ZIP_EXTENSION: &str = "zip";

/// The most `.txt` and `.lrc` bytes read out of one zip while deciding what it is.
///
/// Each file is bounded by [`MAX_ZIPPED_TEXT`], and a zip can hold two thousand of them.
const MAX_TEXT_READ: u64 = 16 * 1024 * 1024;

/// Whether a path has the extension a zip has.
///
/// **A candidate, not a verdict.** Most zips are not songs, and only [`read_zip`] says which are.
#[must_use]
pub fn is_zip_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(ZIP_EXTENSION))
}

/// Which kind of song a zip holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipKind {
    /// An MP3+G pair.
    Cdg,
    /// An UltraStar `.txt` and its MP3.
    UltraStar,
    /// An `.lrc` and its MP3.
    Lrc,
}

/// The one song a zip holds.
#[derive(Debug, Clone)]
pub enum ZipSong {
    /// An MP3+G pair.
    Cdg {
        /// The MP3.
        audio: ZipMember,
        /// The `.cdg` in the same folder of the zip.
        graphics: ZipMember,
    },
    /// An UltraStar song.
    UltraStar {
        /// The `.txt`.
        text: ZipMember,
        /// The MP3 its header names.
        audio: ZipMember,
        /// What the file says.
        song: UltraStar,
    },
    /// An LRC song.
    Lrc {
        /// The `.lrc`.
        lyrics: ZipMember,
        /// The MP3 with the same stem.
        audio: ZipMember,
        /// What the file says.
        song: Lrc,
    },
}

impl ZipSong {
    /// Which kind of song it is.
    #[must_use]
    pub fn kind(&self) -> ZipKind {
        match self {
            Self::Cdg { .. } => ZipKind::Cdg,
            Self::UltraStar { .. } => ZipKind::UltraStar,
            Self::Lrc { .. } => ZipKind::Lrc,
        }
    }

    /// The entry the song is found from: the MP3 of a pair, the `.txt`, or the `.lrc`.
    ///
    /// The file a loose song of the same kind is filed under, so both take their title from the
    /// same name.
    #[must_use]
    pub fn found_from(&self) -> &ZipMember {
        match self {
            Self::Cdg { audio, .. } => audio,
            Self::UltraStar { text, .. } => text,
            Self::Lrc { lyrics, .. } => lyrics,
        }
    }

    /// The MP3.
    #[must_use]
    pub fn audio(&self) -> &ZipMember {
        match self {
            Self::Cdg { audio, .. } | Self::UltraStar { audio, .. } | Self::Lrc { audio, .. } => {
                audio
            }
        }
    }
}

/// What a zip is, as a song source.
#[derive(Debug, Clone)]
pub enum ZipVerdict {
    /// It holds one song.
    Song(Box<ZipSong>),
    /// It does not hold both files of any song. Not a failure: most zips are something else.
    NotASong,
    /// It holds more than one song, named here by the entry each is found from.
    SeveralSongs(Vec<String>),
    /// It holds a song that cannot be packaged, or it cannot be read, with the reason.
    Refused(String),
}

impl ZipVerdict {
    /// Why the zip is refused, when it is.
    #[must_use]
    pub fn refusal(&self) -> Option<String> {
        match self {
            Self::Song(_) | Self::NotASong => None,
            Self::SeveralSongs(names) => Some(format!(
                "it holds {} songs, and a zip is one song: {}",
                names.len(),
                names.join(", ")
            )),
            Self::Refused(reason) => Some(reason.clone()),
        }
    }
}

/// One entry of a zip that is not a folder and not something an operating system added.
struct Entry {
    member: ZipMember,
    encrypted: bool,
}

impl Entry {
    fn file(&self) -> &str {
        self.member.file_name()
    }

    fn folder(&self) -> &str {
        self.member
            .name
            .rsplit_once('/')
            .map_or("", |(folder, _)| folder)
    }

    fn has_extension(&self, wanted: &[&str]) -> bool {
        Path::new(self.file())
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| wanted.iter().any(|want| ext.eq_ignore_ascii_case(want)))
    }
}

/// Lists a zip's entries without reading any of them.
fn list(path: &Path) -> Result<Vec<Entry>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("could not be read: {error}"))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|error| format!("it is not a readable zip archive: {error}"))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "it holds {} files, which is a collection and not one song",
            zip.len()
        ));
    }

    let mut entries = Vec::new();
    for index in 0..zip.len() {
        // The raw form, which opens an entry behind a password or in a compression this build
        // lacks. Nothing is read through it: only what the directory says.
        let entry = zip
            .by_index_raw(index)
            .map_err(|error| format!("entry {} could not be read: {error}", index + 1))?;
        let name = entry_name(entry.name());
        if entry.is_dir() || is_os_junk(&name) {
            continue;
        }
        entries.push(Entry {
            member: ZipMember {
                archive: path.to_path_buf(),
                index,
                name,
                size: entry.size(),
            },
            encrypted: entry.encrypted(),
        });
    }

    // Sorted so that what a zip is does not depend on the order its entries were written in.
    entries.sort_by(|a, b| a.member.name.cmp(&b.member.name));
    if let Some(twice) = entries
        .windows(2)
        .find(|pair| pair[0].member.name == pair[1].member.name)
    {
        return Err(format!("it holds two files named {}", twice[0].member.name));
    }
    Ok(entries)
}

/// What is found in a zip before it is decided what the zip is.
#[derive(Default)]
struct Found {
    /// Complete songs that can be packaged.
    songs: Vec<ZipSong>,
    /// Complete songs that cannot, each with the entry it is found from and the reason.
    refused: Vec<(String, String)>,
    /// How many `.txt` and `.lrc` bytes have been read.
    text_read: u64,
}

impl Found {
    /// Reads a `.txt` or an `.lrc`.
    fn text(&mut self, entry: &Entry) -> Result<Vec<u8>, String> {
        if entry.encrypted {
            return Err(PASSWORD.to_owned());
        }
        if self.text_read.saturating_add(entry.member.size) > MAX_TEXT_READ {
            return Err(format!(
                "the zip holds more than {} MiB of text",
                MAX_TEXT_READ / (1024 * 1024)
            ));
        }
        let bytes = entry
            .member
            .read(MAX_ZIPPED_TEXT)
            .map_err(|error| error.to_string())?;
        self.text_read += bytes.len() as u64;
        Ok(bytes)
    }

    fn refuse(&mut self, entry: &Entry, reason: impl Into<String>) {
        self.refused
            .push((entry.member.name.clone(), reason.into()));
    }
}

/// Why a song behind a password is refused.
const PASSWORD: &str = "it is password-protected";

/// Finds the songs in one folder of a zip.
///
/// **One MP3 is one song, and an MP3+G pair comes first**, then an UltraStar song, then an LRC
/// song. An `.lrc` or a `.txt` beside a pair belongs to no second song.
fn songs_in_folder(entries: &[&Entry], found: &mut Found) {
    let names: Vec<&str> = entries.iter().map(|entry| entry.file()).collect();
    let named = |file: &str| entries.iter().copied().find(|entry| entry.file() == file);
    let mut taken: BTreeSet<&str> = BTreeSet::new();

    for audio in entries {
        if !audio.has_extension(&AUDIO_EXTENSIONS) {
            continue;
        }
        let graphics = km_kmpkg::sibling_among(audio.file(), &[GRAPHICS_EXTENSION], &names);
        let Some(graphics) = graphics.and_then(named) else {
            continue;
        };
        taken.insert(audio.file());
        if audio.encrypted || graphics.encrypted {
            found.refuse(audio, PASSWORD);
            continue;
        }
        found.songs.push(ZipSong::Cdg {
            audio: audio.member.clone(),
            graphics: graphics.member.clone(),
        });
    }

    // A `.txt` in a folder with no MP3 names no audio that is here, so it is not read.
    if !entries
        .iter()
        .any(|entry| entry.has_extension(&AUDIO_EXTENSIONS))
    {
        return;
    }

    let paired = taken.clone();
    for text in entries {
        if !text.has_extension(&[crate::ultrastar::ULTRASTAR_EXTENSION]) {
            continue;
        }
        let bytes = match found.text(text) {
            Ok(bytes) => bytes,
            Err(reason) => {
                found.refuse(text, reason);
                continue;
            }
        };
        let song = match interpret_ultrastar(&bytes) {
            Ok(song) => song,
            Err(UltraStarRefusal::Refused(error)) => {
                found.refuse(text, error.to_string());
                continue;
            }
            // A readme, or a song whose audio is a video or another format: the MP3 it needs is
            // not here.
            Err(_) => continue,
        };
        let Some(audio) = km_kmpkg::named_among(&song.audio, &names).and_then(named) else {
            continue;
        };
        if paired.contains(audio.file()) {
            continue;
        }
        taken.insert(audio.file());
        if audio.encrypted {
            found.refuse(text, PASSWORD);
            continue;
        }
        found.songs.push(ZipSong::UltraStar {
            text: text.member.clone(),
            audio: audio.member.clone(),
            song,
        });
    }

    for lyrics in entries {
        if !lyrics.has_extension(&[km_kmpkg::LRC_EXTENSION]) {
            continue;
        }
        let audio = km_kmpkg::sibling_among(lyrics.file(), &AUDIO_EXTENSIONS, &names);
        let Some(audio) = audio.and_then(named) else {
            continue;
        };
        if taken.contains(audio.file()) {
            continue;
        }
        let song = found
            .text(lyrics)
            .and_then(|bytes| km_song::lrc::parse(&bytes).map_err(|error| error.to_string()));
        match song {
            Err(reason) => found.refuse(lyrics, reason),
            Ok(_) if audio.encrypted => found.refuse(lyrics, PASSWORD),
            Ok(song) => found.songs.push(ZipSong::Lrc {
                lyrics: lyrics.member.clone(),
                audio: audio.member.clone(),
                song,
            }),
        }
    }
}

/// Reads a zip and says what it is.
///
/// Files are paired within one folder of the zip, by the rules loose files are paired by. A zip
/// inside the zip is not opened.
#[must_use]
pub fn read_zip(path: &Path) -> ZipVerdict {
    let entries = match list(path) {
        Ok(entries) => entries,
        Err(reason) => return ZipVerdict::Refused(reason),
    };

    let mut folders: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for entry in &entries {
        folders.entry(entry.folder()).or_default().push(entry);
    }
    let mut found = Found::default();
    for folder in folders.values() {
        songs_in_folder(folder, &mut found);
    }

    // A song that cannot be packaged matters only when it is the zip's one song. Beside a song that
    // can be, it is the duet version or the second timing that song folders carry.
    match found.songs.len() {
        0 => match found.refused.into_iter().next() {
            Some((name, reason)) => ZipVerdict::Refused(format!("{name}: {reason}")),
            None => ZipVerdict::NotASong,
        },
        1 => ZipVerdict::Song(Box::new(found.songs.remove(0))),
        _ => ZipVerdict::SeveralSongs(
            found
                .songs
                .iter()
                .map(|song| song.found_from().name.clone())
                .collect(),
        ),
    }
}
