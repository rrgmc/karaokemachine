//! What every song whose music is a recording shares: a timeline in milliseconds, and the [`Song`]
//! built around one.
//!
//! An UltraStar file and an LRC file both reduce to a [`LyricTimeline`] timed against an audio file.
//! The readers differ and nothing after them does, so the timebase and the wrapper live here rather
//! than in either reader.
//!
//! [`Song`]: crate::Song

use crate::encoding::TextDecoder;
use crate::timeline::LyricTimeline;

/// The timeline's timebase: one tick is one millisecond of audio.
pub const TICKS_PER_SECOND: u32 = 1_000;

/// The beat a timeline at [`TICKS_PER_SECOND`] is measured against: half a second, as a timecode
/// MIDI file's is.
pub const NOMINAL_BEAT_TICKS: u16 = 500;

/// A [`crate::Song`] carrying a stored timeline and nothing to play, for the readers built on one.
///
/// **The lyric view, the lyric-line events and the lyrics endpoint all read a `Song`**, for its
/// timeline and its tempo map. A recording's song has no events, no tracks and a timecode tempo map
/// at [`TICKS_PER_SECOND`], under which a tick is a millisecond.
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

/// The file's lines, with CR, LF or CRLF endings, each without its ending.
pub(crate) fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let separator = if bytes.contains(&b'\n') { b'\n' } else { b'\r' };
    bytes
        .split(move |&b| b == separator)
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect()
}
