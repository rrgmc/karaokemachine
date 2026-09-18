//! UltraStar `.txt` files, read for their words and the times they are sung.
//!
//! An UltraStar file is a singing game's song: a header of `#TAG:value` lines naming the audio and
//! the beat rate, then one note per line. **Only the words and their timing are kept.** Every note
//! also carries a pitch and a type that the games score a singer against, and both are discarded,
//! because this project scores nobody. See `UltraStar as a song source` in
//! `docs/decisions/song-sources.md`, and `docs/research/ultrastar.md` for the format.
//!
//! The result is a [`LyricTimeline`] whose ticks are milliseconds from the start of the audio, so a
//! reader pairs it with [`TICKS_PER_SECOND`] and never needs the file again.

use crate::encoding::TextDecoder;
use crate::karaoke::clean_lyric_text;
use crate::timeline::{LineBreak, LineInference, LyricTimeline, RawSyllable, build_timeline};

/// The timeline's timebase: one tick is one millisecond of audio.
pub const TICKS_PER_SECOND: u32 = 1_000;

/// The beat a timeline at [`TICKS_PER_SECOND`] is measured against: half a second, as a timecode
/// MIDI file's is.
const NOMINAL_BEAT_TICKS: u16 = 500;

/// Why a file is not an UltraStar song this project plays.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UltraStarError {
    /// The file has no `#TITLE`, so it is not an UltraStar file at all. A song folder holds readme
    /// files too.
    #[error("not an UltraStar file: there is no #TITLE")]
    NotUltraStar,
    /// `#VERSION` names a major version this reader does not know.
    #[error("UltraStar version {0} is not supported")]
    UnsupportedVersion(String),
    /// A versioned file asks for relative beats, which version 1 removed.
    #[error("a versioned UltraStar file cannot use #RELATIVE")]
    RelativeInVersioned,
    /// The file has two voices.
    #[error("a duet: the words are drawn as one voice")]
    Duet,
    /// Neither `#AUDIO` nor `#MP3` names the audio.
    #[error("the file names no audio")]
    NoAudio,
    /// `#BPM` is missing, not a number, or not above zero.
    #[error("#BPM is missing or not a positive number")]
    NoBpm,
    /// The file has no note with words in it.
    #[error("the file has no sung words")]
    NoNotes,
}

/// An UltraStar file, reduced to what a karaoke machine uses.
#[derive(Debug, Clone)]
pub struct UltraStar {
    /// `#TITLE`, decoded and trimmed.
    pub title: String,
    /// `#ARTIST`, decoded and trimmed, where the file gives one.
    pub artist: Option<String>,
    /// `#LANGUAGE`, as the file writes it: a name such as `English`, not a code.
    pub language: Option<String>,
    /// The audio file's name, relative to the `.txt`: `#AUDIO`, else `#MP3`.
    pub audio: String,
    /// `#VIDEO`, where the file names one. The audio is still the song.
    pub video: Option<String>,
    /// `#VERSION` as written, where the file has one.
    pub version: Option<String>,
    /// Whether the file counted its beats from each line rather than from the start.
    pub relative: bool,
    /// The encoding the text was read in.
    pub decoder: TextDecoder,
    /// The words, in milliseconds from the start of the audio.
    pub timeline: LyricTimeline,
}

/// Reads an UltraStar file.
///
/// # Errors
///
/// Whatever makes the file something other than one voice of timed words over named audio: see
/// [`UltraStarError`].
pub fn parse(bytes: &[u8]) -> Result<UltraStar, UltraStarError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let lines = split_lines(bytes);

    let mut tags: Vec<(String, &[u8])> = Vec::new();
    let mut body_start = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let line = trim_ascii(line);
        if line.is_empty() {
            continue;
        }
        let Some(tag) = line.strip_prefix(b"#") else {
            body_start = index;
            break;
        };
        let Some(colon) = tag.iter().position(|&b| b == b':') else {
            continue;
        };
        let name = String::from_utf8_lossy(&tag[..colon])
            .trim()
            .to_ascii_uppercase();
        tags.push((name, trim_ascii(&tag[colon + 1..])));
    }
    let tag = |name: &str| {
        tags.iter()
            .find(|(tag, value)| tag == name && !value.is_empty())
            .map(|(_, value)| *value)
    };
    let ascii = |value: &[u8]| String::from_utf8_lossy(value).trim().to_owned();

    if tag("TITLE").is_none() {
        return Err(UltraStarError::NotUltraStar);
    }

    let version = tag("VERSION").map(ascii);
    let versioned = match version.as_deref() {
        None => false,
        Some(written) => match major_version(written) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(UltraStarError::UnsupportedVersion(written.to_owned())),
        },
    };
    let relative = tag("RELATIVE").is_some_and(|value| ascii(value).eq_ignore_ascii_case("yes"));
    if versioned && relative {
        return Err(UltraStarError::RelativeInVersioned);
    }

    let bpm = tag("BPM")
        .and_then(|value| number(&ascii(value)))
        .filter(|bpm| *bpm > 0.0)
        .ok_or(UltraStarError::NoBpm)?;
    let gap_ms = tag("GAP")
        .and_then(|value| number(&ascii(value)))
        .unwrap_or(0.0);

    let notes = read_notes(&lines[body_start..], relative)?;

    // **Only an unversioned file declares its encoding.** Version 1 says UTF-8, and a file that is
    // UTF-8 is read as UTF-8 without being told; one that is not was written by an editor that put
    // a version on a legacy code page, and detection reads it where a declared UTF-8 would not.
    // Every text byte is sampled, header values included: a file whose words are all ASCII can
    // still name its audio with an `Ä` in it, and that name has to match a file on disk.
    let declared = if versioned {
        None
    } else {
        tag("ENCODING").map(|value| encoding_label(&ascii(value)))
    };
    let mut samples: Vec<&[u8]> = notes.iter().map(|note| note.text).collect();
    samples.extend(tags.iter().map(|(_, value)| *value));
    let domain = tag("LANGUAGE").and_then(|value| language_domain(&ascii(value)));
    let decoder = TextDecoder::resolve_for_domain(&samples, declared.as_deref(), domain);
    let text = |value: &[u8]| {
        let decoded = clean_lyric_text(decoder.decode(value));
        let trimmed = decoded.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };

    let audio = tag("AUDIO")
        .or_else(|| tag("MP3"))
        .and_then(text)
        .ok_or(UltraStarError::NoAudio)?;

    let raws = syllables(&notes, &decoder, bpm, gap_ms);
    if raws.iter().all(|raw| raw.text.trim().is_empty()) {
        return Err(UltraStarError::NoNotes);
    }
    let timeline = build_timeline(
        raws,
        LineInference::for_ticks_per_quarter(NOMINAL_BEAT_TICKS),
        |tick| tick,
    );

    Ok(UltraStar {
        title: tag("TITLE").and_then(text).unwrap_or_default(),
        artist: tag("ARTIST").and_then(text),
        language: tag("LANGUAGE").and_then(text),
        audio,
        video: tag("VIDEO").and_then(text),
        version,
        relative,
        decoder,
        timeline,
    })
}

/// A [`crate::Song`] carrying a stored timeline and nothing to play, for the readers built on one.
///
/// **The lyric view, the lyric-line events and the lyrics endpoint all read a `Song`**, for its
/// timeline and its tempo map. An UltraStar song's music is a recording, so this one has no events,
/// no tracks and a timecode tempo map at [`TICKS_PER_SECOND`], under which a tick is a millisecond.
#[must_use]
pub fn song_from_timeline(timeline: LyricTimeline) -> crate::Song {
    let duration_ticks = timeline.lines.last().map_or(0, |line| line.end_tick);
    crate::Song {
        flavor: crate::KaraokeFlavor::LyricEvents,
        tempo_map: crate::TempoMap::new(
            crate::Timebase::Smpte {
                ticks_per_second: TICKS_PER_SECOND,
            },
            Vec::new(),
        ),
        ticks_per_quarter: 0,
        duration_ticks,
        events: Vec::new(),
        lyrics: timeline,
        meta: crate::KaraokeMeta::default(),
        decoder: TextDecoder::utf8(),
        dialect: crate::Dialect::default(),
        track_count: 0,
        track_names: Vec::new(),
        truncated_tracks: Vec::new(),
        missing_tracks: 0,
        repaired_notes: 0,
    }
}

/// One sung note, in absolute beats, with its text still undecoded.
struct Note<'a> {
    start: i64,
    end: i64,
    text: &'a [u8],
    break_before: bool,
}

/// Reads the body: notes, phrase ends, and the markers that refuse a file.
fn read_notes<'a>(lines: &[&'a [u8]], relative: bool) -> Result<Vec<Note<'a>>, UltraStarError> {
    let mut notes: Vec<Note<'a>> = Vec::new();
    let mut offset: i64 = 0;
    let mut pending_break = false;
    for &line in lines {
        let line = line.trim_ascii_start();
        let Some(&kind) = line.first() else {
            continue;
        };
        match kind {
            b':' | b'*' | b'F' | b'R' | b'G' => {
                let Some((start, length, text)) = note_fields(&line[1..]) else {
                    continue;
                };
                let start = start + offset;
                notes.push(Note {
                    start,
                    end: start + length.max(0),
                    text,
                    break_before: pending_break && !notes.is_empty(),
                });
                pending_break = false;
            }
            // **Only the first number marks the break.** Older editors wrote a second number in an
            // absolute file too, saying when the next line appears, and it moves nothing. In a
            // relative file the second number is how far the following beats are shifted.
            b'-' => {
                pending_break = true;
                if relative {
                    let numbers: Vec<i64> = words(&line[1..])
                        .filter_map(|word| std::str::from_utf8(word).ok()?.parse().ok())
                        .collect();
                    offset += numbers.get(1).or(numbers.first()).copied().unwrap_or(0);
                }
            }
            b'P' if line.len() > 1 && line[1..].iter().any(u8::is_ascii_digit) => {
                return Err(UltraStarError::Duet);
            }
            b'E' if trim_ascii(&line[1..]).is_empty() => break,
            _ => {}
        }
    }
    Ok(notes)
}

/// Turns notes into the syllables a timeline is built from.
fn syllables(notes: &[Note<'_>], decoder: &TextDecoder, bpm: f64, gap_ms: f64) -> Vec<RawSyllable> {
    // The header's BPM counts quarter beats, and a note's beats are a quarter of one: UltraStar
    // Deluxe multiplies the header value by four before using it.
    let to_ms = |beat: i64| {
        let ms = gap_ms + beat as f64 * 60_000.0 / (bpm * 4.0);
        ms.round().clamp(0.0, f64::from(u32::MAX)) as u32
    };

    let mut raws: Vec<RawSyllable> = Vec::new();
    for note in notes {
        let mut text = clean_lyric_text(decoder.decode(note.text));
        let (start, end) = (to_ms(note.start), to_ms(note.end));
        // **`~` is a sustained vowel, not a syllable.** It carries the note before it across
        // another pitch, so it lengthens that one and draws nothing of its own.
        if text.trim() == "~"
            && !note.break_before
            && let Some(previous) = raws.last_mut()
        {
            previous.end_tick = Some(previous.end_tick.map_or(end, |known| known.max(end)));
            if text.ends_with(' ') && !previous.text.ends_with(' ') {
                previous.text.push(' ');
            }
            continue;
        }
        if text.trim() == "~" {
            text.clear();
        }
        raws.push(RawSyllable {
            tick: start,
            text,
            break_before: if note.break_before {
                LineBreak::Line
            } else {
                LineBreak::None
            },
            end_tick: Some(end),
        });
    }

    // A space before a line's first word or after its last is a word boundary with nothing on its
    // other side, and a line is centred on its measured width. A syllable that is only a space
    // there draws nothing, and is emptied; one that carries the line's break keeps the break.
    let starts: Vec<usize> = (0..raws.len())
        .filter(|&index| index == 0 || raws[index].break_before != LineBreak::None)
        .collect();
    for (position, &first) in starts.iter().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(raws.len());
        let line = &mut raws[first..end];
        for raw in line.iter_mut() {
            raw.text = raw.text.trim_start().to_owned();
            if !raw.text.is_empty() {
                break;
            }
        }
        for raw in line.iter_mut().rev() {
            raw.text = raw.text.trim_end().to_owned();
            if !raw.text.is_empty() {
                break;
            }
        }
    }
    raws
}

/// The start, the length and the text of a note line, after its type character.
///
/// **The text is everything after the single space that follows the pitch**, so a leading space in
/// it survives: that space is a word boundary.
fn note_fields(rest: &[u8]) -> Option<(i64, i64, &[u8])> {
    let mut rest = rest;
    let mut numbers = [0i64; 3];
    for number in &mut numbers {
        let start = rest.iter().position(|b| !b.is_ascii_whitespace())?;
        rest = &rest[start..];
        let len = rest
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(rest.len());
        *number = std::str::from_utf8(&rest[..len]).ok()?.parse().ok()?;
        rest = &rest[len..];
    }
    let text = rest
        .strip_prefix(b" ")
        .or_else(|| rest.strip_prefix(b"\t"))
        .unwrap_or(rest);
    Some((numbers[0], numbers[1], text))
}

/// A number as UltraStar files write them, with `.` or `,` as the decimal separator.
fn number(written: &str) -> Option<f64> {
    written.trim().replace(',', ".").parse().ok()
}

/// The major version of a `#VERSION` value, which can be written `1.0.0`, `1.1` or `1,00`.
fn major_version(written: &str) -> Option<u32> {
    written
        .trim()
        .split(['.', ','])
        .next()
        .and_then(|major| major.trim().parse().ok())
}

/// The WHATWG label for an `#ENCODING` value. The three the format defines are spelled without the
/// hyphen the labels need; anything else is passed on and falls through to detection if unknown.
fn encoding_label(written: &str) -> String {
    let squashed: String = written
        .chars()
        .filter(|ch| !matches!(ch, '-' | '_' | ' '))
        .collect::<String>()
        .to_ascii_lowercase();
    match squashed.as_str() {
        "utf8" => "utf-8".to_owned(),
        "cp1252" => "windows-1252".to_owned(),
        "cp1250" => "windows-1250".to_owned(),
        _ => written.to_owned(),
    }
}

/// The top-level domain whose text `#LANGUAGE` names, as the hint encoding detection takes.
///
/// Only the languages whose legacy code pages detection confuses are here: a name it does not know
/// leaves detection unhinted, which is what every other file gets.
fn language_domain(written: &str) -> Option<&'static [u8]> {
    let name = written.trim().to_ascii_lowercase();
    let domain: &[u8] = match name.as_str() {
        "english" => b"uk",
        "portuguese" | "português" | "portugues" => b"pt",
        "spanish" | "español" | "espanol" => b"es",
        "french" | "français" | "francais" => b"fr",
        "german" | "deutsch" => b"de",
        "italian" | "italiano" => b"it",
        "dutch" | "nederlands" => b"nl",
        "swedish" | "svenska" => b"se",
        "norwegian" | "norsk" => b"no",
        "danish" | "dansk" => b"dk",
        "finnish" | "suomi" => b"fi",
        "polish" | "polski" => b"pl",
        "czech" | "čeština" | "cestina" => b"cz",
        "slovak" | "slovenčina" => b"sk",
        "hungarian" | "magyar" => b"hu",
        "croatian" | "hrvatski" => b"hr",
        "turkish" | "türkçe" | "turkce" => b"tr",
        _ => return None,
    };
    Some(domain)
}

/// The file's lines, with CR, LF or CRLF endings, each without its ending.
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let separator = if bytes.contains(&b'\n') { b'\n' } else { b'\r' };
    bytes
        .split(move |&b| b == separator)
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect()
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    bytes.trim_ascii()
}

fn words(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(u8::is_ascii_whitespace)
        .filter(|word| !word.is_empty())
}

#[cfg(test)]
mod tests;
