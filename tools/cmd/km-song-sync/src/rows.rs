//! What the page says about each song in a folder, and where a song's words come from.
//!
//! **Everything here reads the disk, and reads it for one page of files.** The listing has already
//! been narrowed and paged, so a folder of thousands costs a hundred small reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use km_song::{ParseOptions, Song};

/// Whether a path names a file the editor can open: a MIDI file under either of its names, or a
/// karaoke MIDI file.
#[must_use]
pub fn is_song(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(extension.to_lowercase().as_str(), "mid" | "midi" | "kar")
        })
}

/// What a song file holds, as far as starting the editor on it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holds {
    /// Words of its own, which the editor can open with nothing else given.
    Words,
    /// Music and no words.
    NoWords,
    /// Nothing this program can read as a MIDI file.
    NotMidi,
}

/// One song on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The file's name.
    pub name: String,
    /// Its full path, as the page sends it back.
    pub path: String,
    /// What the file holds.
    pub holds: Holds,
    /// The title the file states.
    pub title: Option<String>,
    /// The artist the file states.
    pub artist: Option<String>,
    /// The language the file states, as the file spells it.
    pub language: Option<String>,
    /// The name of the text file beside it that supplies its words, where there is one.
    pub sidecar: Option<String>,
    /// The name of the synced copy the editor writes.
    pub out: String,
    /// Whether that copy is there already.
    pub out_exists: bool,
}

/// The text files of one folder, by the song name each one sits beside.
///
/// **One pass over the folder, whatever the number of songs.** A name is matched without its
/// extension and without regard to case, so `Song.TXT` sits beside `song.mid`.
#[derive(Debug, Default)]
pub struct Sidecars(HashMap<String, PathBuf>);

impl Sidecars {
    /// Reads the folder. One that cannot be read holds no text files.
    #[must_use]
    pub fn read(folder: &Path) -> Self {
        let mut found = HashMap::new();
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Self(found);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_text = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));
            if is_text && let Some(stem) = stem_of(&path) {
                found.entry(stem).or_insert(path);
            }
        }
        Self(found)
    }

    /// The text file beside `song`, where there is one.
    #[must_use]
    pub fn beside(&self, song: &Path) -> Option<&Path> {
        self.0.get(&stem_of(song)?).map(PathBuf::as_path)
    }
}

/// A file's name without its extension, as two files are matched by it.
fn stem_of(path: &Path) -> Option<String> {
    Some(path.file_stem()?.to_string_lossy().trim().to_lowercase())
}

/// The words a text file holds, in UTF-8, where it holds any.
///
/// **An UltraStar song is not a page of words.** It is a `.txt` with timing in it, and it is a
/// song in its own right. A file with nothing but white space holds no words either.
#[must_use]
pub fn words_in(text_file: &Path) -> Option<String> {
    let bytes = std::fs::read(text_file).ok()?;
    if km_song::ultrastar::parse(&bytes).is_ok() {
        return None;
    }
    let words = km_song::encoding::decode_text_file(&bytes);
    (!words.trim().is_empty()).then_some(words)
}

/// The words beside `song`, with the name of the file they came from.
#[must_use]
pub fn words_beside(song: &Path) -> Option<(String, String)> {
    let sidecars = Sidecars::read(song.parent()?);
    let text_file = sidecars.beside(song)?;
    let name = text_file.file_name()?.to_string_lossy().into_owned();
    Some((name, words_in(text_file)?))
}

/// What one read of a song file found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What the file holds.
    pub holds: Holds,
    /// The title the file states.
    pub title: Option<String>,
    /// The artist the file states.
    pub artist: Option<String>,
    /// The language the file states, as the file spells it.
    pub language: Option<String>,
}

/// Reads `song` once, for what it holds and what it calls itself.
///
/// The parse that says whether a file has words has read its title, artist and language already,
/// so the page shows them at no further cost.
#[must_use]
pub fn found(song: &Path) -> Found {
    let parsed = std::fs::read(song)
        .ok()
        .and_then(|bytes| Song::parse(&bytes, &ParseOptions::default()).ok());
    let Some(parsed) = parsed else {
        return Found {
            holds: Holds::NotMidi,
            title: None,
            artist: None,
            language: None,
        };
    };
    let has_words = parsed
        .lyrics
        .lines
        .iter()
        .any(|line| !line.syllables.is_empty());
    let stated = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
    Found {
        holds: if has_words {
            Holds::Words
        } else {
            Holds::NoWords
        },
        title: stated(parsed.meta.title),
        artist: stated(parsed.meta.artist),
        language: stated(parsed.meta.language),
    }
}

/// What `song` holds.
#[must_use]
pub fn holds(song: &Path) -> Holds {
    found(song).holds
}

/// Describes one page of song files, all from one folder.
#[must_use]
pub fn describe(files: &[km_folders::Folder]) -> Vec<Row> {
    let sidecars = files
        .first()
        .and_then(|file| Path::new(&file.path).parent().map(Sidecars::read))
        .unwrap_or_default();
    files
        .iter()
        .map(|file| {
            let song = Path::new(&file.path);
            let out = km_song::kar_write::synced_path(song);
            let sidecar = sidecars
                .beside(song)
                .filter(|text_file| words_in(text_file).is_some())
                .and_then(|text_file| text_file.file_name())
                .map(|name| name.to_string_lossy().into_owned());
            let found = found(song);
            Row {
                name: file.name.clone(),
                path: file.path.clone(),
                holds: found.holds,
                title: found.title,
                artist: found.artist,
                language: found.language,
                sidecar,
                out_exists: out.exists(),
                out: out
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use km_testkit::Scratch;

    fn file(scratch: &Scratch, name: &str) -> km_folders::Folder {
        km_folders::Folder {
            name: name.to_owned(),
            path: scratch.join(name).display().to_string(),
        }
    }

    #[test]
    fn a_song_is_a_midi_file_under_any_of_its_three_names() {
        for name in ["a.mid", "a.MIDI", "a.Kar"] {
            assert!(is_song(Path::new(name)), "{name}");
        }
        for name in ["a.txt", "a.mp3", "a.rmi", "no-extension"] {
            assert!(!is_song(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn a_row_says_what_the_file_holds_and_what_is_beside_it() {
        let scratch = Scratch::new("sync-rows");
        std::fs::write(scratch.join("sung.kar"), km_song::testing::soft_karaoke()).expect("write");
        std::fs::write(scratch.join("tune.mid"), km_song::testing::instrumental()).expect("write");
        std::fs::write(scratch.join("Tune.TXT"), "la la\n").expect("write");
        std::fs::write(scratch.join("tune.kar"), b"").expect("write");
        std::fs::write(scratch.join("broken.mid"), b"not a midi file").expect("write");
        std::fs::write(scratch.join("broken.txt"), "  \n").expect("write");

        let rows = describe(&[
            file(&scratch, "sung.kar"),
            file(&scratch, "tune.mid"),
            file(&scratch, "broken.mid"),
        ]);

        assert_eq!(rows[0].holds, Holds::Words);
        assert!(rows[0].title.is_some(), "a karaoke file states its title");
        assert_eq!(rows[0].sidecar, None);
        assert_eq!(rows[0].out, "sung-synced.kar");
        assert!(!rows[0].out_exists);

        assert_eq!(rows[1].holds, Holds::NoWords);
        assert_eq!(rows[1].artist, None, "a bare MIDI file states no artist");
        assert_eq!(rows[1].language, None);
        assert_eq!(rows[1].sidecar.as_deref(), Some("Tune.TXT"));
        assert_eq!(rows[1].out, "tune.kar");
        assert!(rows[1].out_exists);

        assert_eq!(rows[2].holds, Holds::NotMidi);
        assert_eq!(rows[2].sidecar, None, "a blank text file holds no words");
    }

    #[test]
    fn the_words_beside_a_song_arrive_as_utf8_whatever_they_were_saved_in() {
        let scratch = Scratch::new("sync-sidecar");
        let song = scratch.join("song.mid");
        std::fs::write(&song, km_song::testing::instrumental()).expect("write");
        let mut utf16: Vec<u8> = vec![0xFF, 0xFE];
        utf16.extend("coração\r\n".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(scratch.join("song.txt"), utf16).expect("write");

        let (name, words) = words_beside(&song).expect("the words beside the song");
        assert_eq!(name, "song.txt");
        assert_eq!(words, "coração\n");
    }
}
