//! LRC files, read for their words and the times they are sung.
//!
//! An LRC file is plain text beside an audio file. Each line opens with one or more `[mm:ss.xx]`
//! timestamps saying when it is sung, and a `[name:value]` line carries a tag such as the title.
//! **Most files time whole lines.** The enhanced form adds a `<mm:ss.xx>` tag before each word, and
//! those words reach the timeline as syllables. See `LRC as a song source` in
//! `docs/decisions/song-sources.md`.
//!
//! The result is a [`LyricTimeline`] whose ticks are milliseconds from the start of the audio, as an
//! UltraStar file's is, so everything after this reader treats the two alike.

use crate::encoding::TextDecoder;
use crate::karaoke::clean_lyric_text;
use crate::recording::{NOMINAL_BEAT_TICKS, split_lines};
use crate::timeline::{LineBreak, LineInference, LyricTimeline, RawSyllable, build_timeline};

/// How long the last line of a line-timed file is held, in milliseconds, when no blank line ends it.
///
/// A line-timed file says when each line starts and never when one stops, and nothing follows the
/// last line to bound it. A few seconds is a short line sung at an ordinary pace.
pub const LAST_LINE_HOLD_MS: u32 = 3_000;

/// Why a file is not an LRC song.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LrcError {
    /// No line carries both a timestamp and words.
    #[error("not an LRC file: no line has a timestamp and words")]
    NotLrc,
}

/// An LRC file, reduced to what a karaoke machine uses.
#[derive(Debug, Clone)]
pub struct Lrc {
    /// `[ti:]`, decoded and trimmed, where the file gives one.
    pub title: Option<String>,
    /// `[ar:]`, decoded and trimmed, where the file gives one.
    pub artist: Option<String>,
    /// `[offset:]` in milliseconds, already applied to every tick in the timeline.
    pub offset_ms: i32,
    /// Whether any line carries `<mm:ss.xx>` word tags.
    pub word_timed: bool,
    /// The encoding the text was read in.
    pub decoder: TextDecoder,
    /// The words, in milliseconds from the start of the audio.
    pub timeline: LyricTimeline,
}

/// Reads an LRC file.
///
/// # Errors
///
/// [`LrcError::NotLrc`] when no line has a timestamp and words.
pub fn parse(bytes: &[u8]) -> Result<Lrc, LrcError> {
    let transcoded = from_utf16(bytes);
    let bytes = transcoded.as_deref().unwrap_or(bytes);
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);

    let mut tags: Vec<(String, &[u8])> = Vec::new();
    let mut entries: Vec<Entry<'_>> = Vec::new();
    for line in split_lines(bytes) {
        read_line(line.trim_ascii(), &mut tags, &mut entries);
    }

    let tag = |name: &str| {
        tags.iter()
            .find(|(tag, value)| tag == name && !value.is_empty())
            .map(|(_, value)| *value)
    };

    // **The whole file decides the encoding once**, as a MIDI file's lyrics do. The tags are
    // sampled too: a file whose words are all ASCII can still spell its title with an `é`.
    let mut samples: Vec<&[u8]> = entries
        .iter()
        .flat_map(|entry| entry.segments.iter().map(|segment| segment.text))
        .collect();
    samples.extend(tags.iter().map(|(_, value)| *value));
    let decoder = TextDecoder::resolve(&samples, None);
    let text = |value: &[u8]| {
        let decoded = clean_lyric_text(decoder.decode(value));
        let trimmed = decoded.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };

    let offset_ms = tag("offset")
        .and_then(|value| String::from_utf8_lossy(value).trim().parse::<i32>().ok())
        .unwrap_or(0);
    let word_timed = entries
        .iter()
        .any(|entry| entry.segments.iter().any(|segment| segment.at.is_some()));

    let raws = syllables(&entries, &decoder, offset_ms);
    if raws.iter().all(|raw| raw.text.trim().is_empty()) {
        return Err(LrcError::NotLrc);
    }
    let hold = if word_timed {
        u32::from(NOMINAL_BEAT_TICKS)
    } else {
        LAST_LINE_HOLD_MS
    };
    let timeline = build_timeline(
        raws,
        LineInference {
            default_hold_ticks: hold,
            ..LineInference::for_ticks_per_quarter(NOMINAL_BEAT_TICKS)
        },
        |tick| tick,
    );

    Ok(Lrc {
        title: tag("ti").and_then(text),
        artist: tag("ar").and_then(text),
        offset_ms,
        word_timed,
        decoder,
        timeline,
    })
}

/// One timestamped line, with its text still undecoded.
struct Entry<'a> {
    /// Every timestamp the line opens with, in milliseconds as written.
    stamps: Vec<u32>,
    /// The text, cut at each word tag.
    segments: Vec<Segment<'a>>,
}

/// A run of text and the word tag in front of it, where there is one.
struct Segment<'a> {
    at: Option<u32>,
    text: &'a [u8],
}

/// Reads one line: a tag, a timestamped line, or something to skip.
///
/// **The brackets are found in the raw bytes, before the text is decoded.** `[`, `]`, `<` and `>`
/// never occur inside a multibyte character in UTF-8, Shift-JIS, GBK or Big5, so the markup is
/// found without knowing the encoding.
fn read_line<'a>(line: &'a [u8], tags: &mut Vec<(String, &'a [u8])>, entries: &mut Vec<Entry<'a>>) {
    let mut rest = line;
    let mut stamps = Vec::new();
    while let Some(inner) = rest.strip_prefix(b"[") {
        let Some(close) = inner.iter().position(|&b| b == b']') else {
            break;
        };
        let group = &inner[..close];
        if let Some(ms) = timestamp(group) {
            stamps.push(ms);
            rest = inner[close + 1..].trim_ascii_start();
            continue;
        }
        // A tag stands alone on its line, so a group that is not a timestamp after one is text.
        if stamps.is_empty()
            && let Some(colon) = group.iter().position(|&b| b == b':')
        {
            let name = String::from_utf8_lossy(&group[..colon])
                .trim()
                .to_ascii_lowercase();
            tags.push((name, group[colon + 1..].trim_ascii()));
            return;
        }
        break;
    }
    if !stamps.is_empty() {
        entries.push(Entry {
            stamps,
            segments: segments(rest),
        });
    }
}

/// Cuts a line's text at each `<mm:ss.xx>` word tag. A `<` that opens no timestamp is text.
fn segments(text: &[u8]) -> Vec<Segment<'_>> {
    let mut out = vec![Segment { at: None, text }];
    let mut search = 0;
    loop {
        let current = out.last_mut().expect("never empty");
        let Some(open) = current.text[search..].iter().position(|&b| b == b'<') else {
            break;
        };
        let open = search + open;
        let tail = &current.text[open + 1..];
        let Some(close) = tail.iter().position(|&b| b == b'>') else {
            break;
        };
        let Some(ms) = timestamp(&tail[..close]) else {
            search = open + 1;
            continue;
        };
        let whole = current.text;
        current.text = &whole[..open];
        out.push(Segment {
            at: Some(ms),
            text: &whole[open + close + 2..],
        });
        search = 0;
    }
    out
}

/// Turns timestamped lines into the syllables a timeline is built from.
fn syllables(entries: &[Entry<'_>], decoder: &TextDecoder, offset_ms: i32) -> Vec<RawSyllable> {
    // **A positive offset makes the words come sooner**, as the tag is defined. Nothing is sung
    // before the audio starts, so a time the offset pushes below zero is zero.
    let shift = |ms: i64| (ms - i64::from(offset_ms)).clamp(0, i64::from(u32::MAX)) as u32;

    // One line per timestamp. **A chorus written once with every time it is sung** is that many
    // lines, and a word tag inside it is moved with each repeat.
    let mut lines: Vec<(u32, Option<Vec<RawSyllable>>)> = Vec::new();
    for entry in entries {
        let first = i64::from(entry.stamps[0]);
        for &stamp in &entry.stamps {
            let moved = |at: u32| shift(i64::from(at) - first + i64::from(stamp));
            let start = shift(i64::from(stamp));
            lines.push((start, words(&entry.segments, decoder, start, moved)));
        }
    }
    lines.sort_by_key(|(start, _)| *start);

    let mut raws: Vec<RawSyllable> = Vec::new();
    let mut last_start: Option<u32> = None;
    for (start, words) in lines {
        match words {
            // **A blank line ends the one before it.** It is how a file marks an instrumental break,
            // and without it the line before the break runs until the next one starts.
            None => {
                if let Some(previous) = raws.last_mut()
                    && start > previous.tick
                {
                    previous.end_tick = Some(previous.end_tick.map_or(start, |end| end.min(start)));
                }
            }
            // **A second line at the same time is a translation**, the way players that show two
            // languages write them. The first is the one sung.
            Some(_) if last_start == Some(start) => {}
            Some(words) => {
                last_start = Some(start);
                raws.extend(words);
            }
        }
    }
    raws
}

/// A line's words as syllables, or `None` for a line with no words.
///
/// The first syllable carries the line break. **A word boundary is a leading space**, never a
/// trailing one: a timeline reads a trailing space on every fragment as a file that marks no word
/// ends at all, and a leading one as a file that marks them.
fn words(
    segments: &[Segment<'_>],
    decoder: &TextDecoder,
    start: u32,
    moved: impl Fn(u32) -> u32,
) -> Option<Vec<RawSyllable>> {
    let mut out: Vec<RawSyllable> = Vec::new();
    let mut space_pending = false;
    for (index, segment) in segments.iter().enumerate() {
        let tick = segment.at.map_or(start, &moved);
        let mut decoded = clean_lyric_text(decoder.decode(segment.text));
        if index == 0 || out.is_empty() {
            decoded = strip_part_prefix(decoded.trim_start()).to_owned();
        }
        let body = decoded.trim();
        if body.is_empty() {
            // A tag with no words after it says when the word before it ends.
            if let Some(previous) = out.last_mut()
                && segment.at.is_some()
                && tick > previous.tick
            {
                previous.end_tick = Some(tick);
            }
            space_pending |= !decoded.is_empty();
            continue;
        }
        let boundary = space_pending || decoded.starts_with(char::is_whitespace);
        let text = if out.is_empty() || !boundary {
            body.to_owned()
        } else {
            format!(" {body}")
        };
        out.push(RawSyllable {
            tick,
            text,
            break_before: if out.is_empty() {
                LineBreak::Line
            } else {
                LineBreak::None
            },
            end_tick: None,
        });
        space_pending = decoded.ends_with(char::is_whitespace);
    }
    (!out.is_empty()).then_some(out)
}

/// A line without the `M:`, `F:` or `D:` that marks a duet part.
///
/// The display has one voice, so the part is dropped and the words are kept.
fn strip_part_prefix(line: &str) -> &str {
    for prefix in ["M:", "F:", "D:"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.trim_start();
        }
    }
    line
}

/// A timestamp as milliseconds: `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx`, or `mm:ss:xx`.
fn timestamp(written: &[u8]) -> Option<u32> {
    let written = written.trim_ascii();
    let colon = written.iter().position(|&b| b == b':')?;
    let minutes = digits(&written[..colon])?;
    let rest = &written[colon + 1..];
    let split = rest.iter().position(|&b| b == b'.' || b == b':');
    let (seconds, fraction) = match split {
        Some(at) => (digits(&rest[..at])?, Some(&rest[at + 1..])),
        None => (digits(rest)?, None),
    };
    if seconds >= 60 {
        return None;
    }
    let millis = match fraction {
        None => 0,
        Some(fraction) => {
            digits(fraction)?;
            // Two digits are hundredths and three are thousandths: the fraction's first three
            // digits, padded, are milliseconds.
            fraction
                .iter()
                .chain(b"000")
                .take(3)
                .fold(0u32, |ms, digit| ms * 10 + u32::from(digit - b'0'))
        }
    };
    minutes
        .checked_mul(60_000)?
        .checked_add(seconds * 1_000 + millis)
}

/// A run of ASCII digits as a number; `None` for anything else, or an empty run.
fn digits(written: &[u8]) -> Option<u32> {
    if written.is_empty() || !written.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(written).ok()?.parse().ok()
}

/// A UTF-16 file, re-encoded as UTF-8, where its byte-order mark says it is one.
///
/// Windows editors save LRC files this way, and none of the byte scanning above can read UTF-16.
fn from_utf16(bytes: &[u8]) -> Option<Vec<u8>> {
    let encoding = match bytes {
        [0xFF, 0xFE, ..] => encoding_rs::UTF_16LE,
        [0xFE, 0xFF, ..] => encoding_rs::UTF_16BE,
        _ => return None,
    };
    let (text, _) = encoding.decode_with_bom_removal(bytes);
    Some(text.into_owned().into_bytes())
}

#[cfg(test)]
mod tests;
