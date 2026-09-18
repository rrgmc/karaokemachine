//! Standard MIDI file parsing and karaoke lyric normalization.
//!
//! This crate turns the bytes of a `.mid` or `.kar` file into a [`Song`]: a tick-ordered event
//! list the sequencer can play, and a [`LyricTimeline`] the display can follow. It performs no
//! I/O and knows nothing about packages, audio devices or screens.
//!
//! The two things worth knowing before reading further:
//!
//! * **Everything is in ticks.** Milliseconds are derived from [`TempoMap`] on demand, because the
//!   user can change tempo mid-song and any precomputed timing would quietly become wrong.
//! * **There is no single karaoke format.** [`karaoke`] handles the three conventions that cover
//!   real-world files and normalizes them into one shape.
//!
//! See `docs/ARCHITECTURE.md` for the wider design.

pub mod encoding;
pub mod karaoke;
pub mod loudness;
pub mod redact;
pub mod spacing;
pub mod tempo;
#[cfg(feature = "testing")]
pub mod testing;
pub mod text;
pub mod timeline;
pub mod ultrastar;

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use rayon::prelude::*;
use serde::Serialize;

pub use crate::encoding::{EncodingSource, TextDecoder};
pub use crate::karaoke::{
    Dialect, KaraokeFlavor, KaraokeMeta, clean_meta_name, clean_meta_text, is_only_a_legal_notice,
    looks_like_a_banner,
};
pub use crate::redact::{MASK, contact_spans, redact};
pub use crate::tempo::{TempoMap, Timebase};
pub use crate::timeline::{
    COMFORTABLE_LINE_CHARS, LineInference, LyricGranularity, LyricLine, LyricTimeline,
    RUNAWAY_LINE_CHARS, SYLLABLE_DIVIDER, Syllable, WordEnds,
};

use crate::karaoke::{MetaText, MetaTextKind};

/// Why a file could not be turned into a [`Song`].
#[derive(Debug, thiserror::Error)]
pub enum SongError {
    /// The bytes are not a readable standard MIDI file.
    #[error("not a readable MIDI file: {0}")]
    Midi(#[from] midly::Error),
    /// The file parsed but contains no tracks, so there is nothing to play.
    #[error("the file contains no tracks")]
    NoTracks,
}

/// A channel voice event, at a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// Start a note. Never carries velocity 0 — those are normalized to [`EventKind::NoteOff`].
    NoteOn {
        /// MIDI channel, 0-based. Channel 9 is the drum channel.
        channel: u8,
        /// Note number.
        key: u8,
        /// Velocity, always non-zero.
        velocity: u8,
    },
    /// Stop a note.
    NoteOff {
        /// MIDI channel, 0-based.
        channel: u8,
        /// Note number.
        key: u8,
    },
    /// Continuous controller change.
    Controller {
        /// MIDI channel, 0-based.
        channel: u8,
        /// Controller number.
        controller: u8,
        /// Controller value.
        value: u8,
    },
    /// Instrument selection.
    ProgramChange {
        /// MIDI channel, 0-based.
        channel: u8,
        /// Program number.
        program: u8,
    },
    /// Pitch bend, as a 14-bit value centered on 8192.
    PitchBend {
        /// MIDI channel, 0-based.
        channel: u8,
        /// Bend amount.
        value: u16,
    },
    /// Per-note pressure.
    PolyAftertouch {
        /// MIDI channel, 0-based.
        channel: u8,
        /// Note number.
        key: u8,
        /// Pressure.
        value: u8,
    },
    /// Whole-channel pressure.
    ChannelAftertouch {
        /// MIDI channel, 0-based.
        channel: u8,
        /// Pressure.
        value: u8,
    },
}

impl EventKind {
    /// The channel this event addresses.
    pub fn channel(&self) -> u8 {
        match *self {
            Self::NoteOn { channel, .. }
            | Self::NoteOff { channel, .. }
            | Self::Controller { channel, .. }
            | Self::ProgramChange { channel, .. }
            | Self::PitchBend { channel, .. }
            | Self::PolyAftertouch { channel, .. }
            | Self::ChannelAftertouch { channel, .. } => channel,
        }
    }
}

/// An event with its absolute position in the song.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimedEvent {
    /// Absolute tick from the start of the song.
    pub tick: u32,
    /// Index of the track it came from, so a channel can be tied back to a track name.
    pub track: u16,
    /// What happens.
    pub kind: EventKind,
}

/// How to parse a file.
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// Encoding label from a package manifest, which overrides detection.
    pub declared_encoding: Option<String>,
    /// Line-splitting thresholds; defaults are derived from the file's timebase.
    pub inference: Option<LineInference>,
}

impl ParseOptions {
    /// Options that declare the lyric encoding rather than detecting it.
    pub fn with_encoding(encoding: impl Into<String>) -> Self {
        Self {
            declared_encoding: Some(encoding.into()),
            inference: None,
        }
    }
}

/// A parsed song: everything needed to play it and to show its words.
#[derive(Debug, Clone)]
pub struct Song {
    /// Which karaoke convention the lyrics came from.
    pub flavor: KaraokeFlavor,
    /// Tick-to-time mapping.
    pub tempo_map: TempoMap,
    /// Ticks per quarter note, or 0 for a SMPTE-timed file.
    pub ticks_per_quarter: u16,
    /// Last tick at which anything happens.
    pub duration_ticks: u32,
    /// Channel voice events, ordered by tick.
    pub events: Vec<TimedEvent>,
    /// The lyrics, normalized.
    pub lyrics: LyricTimeline,
    /// Identification recovered from the file.
    pub meta: KaraokeMeta,
    /// The encoding decision applied to the file's text.
    pub decoder: TextDecoder,
    /// What this file's own habits said about how it writes lyrics.
    pub dialect: Dialect,
    /// Number of tracks in the source file.
    pub track_count: usize,
    /// Each track's name, indexed by track, where the file gives one.
    pub track_names: Vec<Option<String>>,
    /// Tracks the parser stopped reading early, by index. Empty for a well-formed file.
    ///
    /// Not an error: the file still plays, and refusing it would be worse than playing the part
    /// that is readable. It is a warning worth surfacing, because everything after the bad byte --
    /// notes, lyrics, the real end of the track -- is gone and nothing else says so.
    pub truncated_tracks: Vec<usize>,
    /// Tracks the header declared that never parsed as a chunk.
    pub missing_tracks: usize,
    /// Note-offs synthesized for notes left sounding when their track's data ran out.
    pub repaired_notes: usize,
}

impl Song {
    /// Parses a standard MIDI or Soft Karaoke file.
    pub fn parse(bytes: &[u8], options: &ParseOptions) -> Result<Self, SongError> {
        let (header, track_iter) = midly::parse(bytes)?;
        // Read from the MThd chunk before iterating: `TrackIter` decrements its own hint as it goes
        // and has nothing left to say by the end.
        let declared_tracks = track_iter.size_hint().0;

        // Track by track rather than through `Smf::parse`, which is `collect_tracks` and throws away
        // the one fact this needs. midly is deliberately not in `strict` mode -- turning it on would
        // refuse files that today play imperfectly -- so a malformed event makes `EventIter` empty
        // its remaining bytes and report exhaustion, with no error and no signal. The tell is that
        // it still had bytes in hand when it said it was done, which means `unread()` has to be
        // sampled *before* each `next()`: afterwards it is always empty.
        //
        // Splitting the tracks across threads is `collect_tracks`' own behavior, kept rather than
        // introduced: it parallelises above 3 KiB and karaoke files are tens of those, so reading
        // them one after another here would have made every corpus scan slower for nothing.
        let chunks = track_iter.collect::<Result<Vec<_>, _>>()?;
        let parsed: Vec<(Vec<TrackEvent<'_>>, bool)> = chunks
            .into_par_iter()
            .map(|mut iter| {
                let mut track_events = Vec::new();
                // Bytes abandoned *after* the end-of-track marker are padding, not data loss: the
                // marker is what ends a track, and midly reads past it only because it is consuming
                // a chunk rather than obeying the track.
                //
                // This guard fires on nothing in the 5,787-file sample -- every truncation there
                // happens before any marker -- so it is a false positive prevented rather than one
                // removed. Kept because it costs a bool and the alternative is reporting lost data
                // where none was lost.
                let mut ended = false;
                let truncated = loop {
                    let remaining = iter.unread().len();
                    match iter.next() {
                        Some(Ok(event)) => {
                            if matches!(event.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)) {
                                ended = true;
                            }
                            track_events.push(event);
                        }
                        // Only reachable under `strict`. Handled anyway so that enabling it would
                        // record the truncation rather than fail the whole file here.
                        Some(Err(_)) => break !ended,
                        None => break remaining != 0 && !ended,
                    }
                };
                (track_events, truncated)
            })
            .collect();

        let mut tracks: Vec<Vec<TrackEvent<'_>>> = Vec::with_capacity(parsed.len());
        let mut truncated_tracks: Vec<usize> = Vec::new();
        for (index, (track_events, truncated)) in parsed.into_iter().enumerate() {
            if truncated {
                truncated_tracks.push(index);
            }
            tracks.push(track_events);
        }
        if tracks.is_empty() {
            return Err(SongError::NoTracks);
        }
        // A chunk midly skipped -- an invalid one, or a duplicate header -- is a track the file said
        // it had and then never produced.
        let missing_tracks = declared_tracks.saturating_sub(tracks.len());

        let (timebase, ticks_per_quarter) = match header.timing {
            Timing::Metrical(tpqn) => {
                let tpqn = tpqn.as_int();
                (
                    Timebase::Metrical {
                        ticks_per_quarter: tpqn,
                    },
                    tpqn,
                )
            }
            Timing::Timecode(fps, subframe) => {
                let ticks_per_second = u32::from(fps.as_int()) * u32::from(subframe.max(1));
                (
                    Timebase::Smpte {
                        ticks_per_second: ticks_per_second.max(1),
                    },
                    0,
                )
            }
        };

        let mut events: Vec<TimedEvent> = Vec::new();
        let mut tempo_changes: Vec<(u32, u32)> = Vec::new();
        let mut texts: Vec<MetaText<'_>> = Vec::new();
        let mut last_tick = 0u32;

        let mut repaired_notes = 0usize;

        for (track_index, track) in tracks.iter().enumerate() {
            let mut tick: u32 = 0;
            // Note-ons this track never turns off. A file that ends holding a note leaves the
            // synthesizer holding it for as long as the process lives, and a truncated track is one
            // whose note-offs were thrown away by the paragraph above.
            let mut sounding: Vec<(u8, u8)> = Vec::new();
            for event in track {
                tick = tick.saturating_add(event.delta.as_int());
                last_tick = last_tick.max(tick);

                match event.kind {
                    TrackEventKind::Midi { channel, message } => {
                        let channel = channel.as_int();
                        let kind = match message {
                            // Velocity 0 is a note-off in disguise; normalizing here means the
                            // sequencer and the analysis never have to remember that.
                            MidiMessage::NoteOn { key, vel } if vel.as_int() == 0 => {
                                EventKind::NoteOff {
                                    channel,
                                    key: key.as_int(),
                                }
                            }
                            MidiMessage::NoteOn { key, vel } => EventKind::NoteOn {
                                channel,
                                key: key.as_int(),
                                velocity: vel.as_int(),
                            },
                            MidiMessage::NoteOff { key, .. } => EventKind::NoteOff {
                                channel,
                                key: key.as_int(),
                            },
                            MidiMessage::Controller { controller, value } => {
                                EventKind::Controller {
                                    channel,
                                    controller: controller.as_int(),
                                    value: value.as_int(),
                                }
                            }
                            MidiMessage::ProgramChange { program } => EventKind::ProgramChange {
                                channel,
                                program: program.as_int(),
                            },
                            MidiMessage::PitchBend { bend } => EventKind::PitchBend {
                                channel,
                                value: bend.0.as_int(),
                            },
                            MidiMessage::Aftertouch { key, vel } => EventKind::PolyAftertouch {
                                channel,
                                key: key.as_int(),
                                value: vel.as_int(),
                            },
                            MidiMessage::ChannelAftertouch { vel } => {
                                EventKind::ChannelAftertouch {
                                    channel,
                                    value: vel.as_int(),
                                }
                            }
                        };
                        match kind {
                            EventKind::NoteOn { channel, key, .. } => {
                                // A second note-on for a key already down retriggers it rather than
                                // stacking, so one entry is one note-off owed.
                                if !sounding.contains(&(channel, key)) {
                                    sounding.push((channel, key));
                                }
                            }
                            EventKind::NoteOff { channel, key } => {
                                if let Some(at) =
                                    sounding.iter().position(|&note| note == (channel, key))
                                {
                                    sounding.swap_remove(at);
                                }
                            }
                            _ => {}
                        }
                        events.push(TimedEvent {
                            tick,
                            track: u16::try_from(track_index).unwrap_or(u16::MAX),
                            kind,
                        });
                    }
                    TrackEventKind::Meta(meta) => {
                        let kind = match meta {
                            MetaMessage::Tempo(us) => {
                                tempo_changes.push((tick, us.as_int()));
                                continue;
                            }
                            MetaMessage::Text(_) => MetaTextKind::Text,
                            MetaMessage::Lyric(_) => MetaTextKind::Lyric,
                            MetaMessage::TrackName(_) => MetaTextKind::TrackName,
                            MetaMessage::Copyright(_) => MetaTextKind::Copyright,
                            MetaMessage::Marker(_) => MetaTextKind::Marker,
                            _ => continue,
                        };
                        let bytes = match meta {
                            MetaMessage::Text(b)
                            | MetaMessage::Lyric(b)
                            | MetaMessage::TrackName(b)
                            | MetaMessage::Copyright(b)
                            | MetaMessage::Marker(b) => b,
                            _ => continue,
                        };
                        texts.push(MetaText {
                            track: track_index,
                            tick,
                            kind,
                            bytes,
                        });
                    }
                    _ => {}
                }
            }

            // Whatever is still down when the track's data runs out is stopped where the data
            // stopped, rather than left sounding for ever. Note-ons and their note-offs are written
            // on the same track in practice, so pairing per track is what a file means; it also puts
            // a truncated track's repair at the point of truncation instead of at the end of a song
            // it never reached. `km-suitability` deliberately keeps the opposite convention for
            // measurement -- see `ChannelStats::finish` -- because this is about playback.
            repaired_notes += sounding.len();
            for (channel, key) in sounding.drain(..) {
                events.push(TimedEvent {
                    tick,
                    track: u16::try_from(track_index).unwrap_or(u16::MAX),
                    kind: EventKind::NoteOff { channel, key },
                });
            }
        }

        // Stable sort: events at the same tick keep source order, so earlier tracks win ties the
        // way a sequencer replaying the file would.
        events.sort_by_key(|e| e.tick);
        texts.sort_by_key(|t| t.tick);

        let tempo_map = TempoMap::new(timebase, tempo_changes);
        let inference = options
            .inference
            .unwrap_or_else(|| LineInference::for_ticks_per_quarter(effective_tpqn(&tempo_map)));

        let source = karaoke::extract(
            &texts,
            inference,
            |tick| tempo_map.tick_to_ms(tick),
            options.declared_encoding.as_deref(),
        );

        // Decoded with the same decoder as everything else, so a Shift-JIS track name is not
        // mojibake while the lyrics beside it are fine.
        let mut track_names: Vec<Option<String>> = vec![None; tracks.len()];
        for text in &texts {
            if text.kind == MetaTextKind::TrackName {
                let name = source.decoder.decode(text.bytes).trim().to_owned();
                if !name.is_empty()
                    && let Some(slot) = track_names.get_mut(text.track)
                    && slot.is_none()
                {
                    *slot = Some(name);
                }
            }
        }

        let duration_ticks =
            last_tick.max(source.timeline.lines.last().map_or(0, |line| line.end_tick));

        Ok(Self {
            flavor: source.flavor,
            tempo_map,
            ticks_per_quarter,
            duration_ticks,
            events,
            lyrics: source.timeline,
            meta: source.meta,
            decoder: source.decoder,
            dialect: source.dialect,
            track_count: tracks.len(),
            track_names,
            truncated_tracks,
            missing_tracks,
            repaired_notes,
        })
    }

    /// The song's length in milliseconds at its written tempo.
    pub fn duration_ms(&self) -> u32 {
        self.tempo_map.tick_to_ms(self.duration_ticks)
    }

    /// The ticks in one beat, for anything measured in beats: a quarter note, or half a second in a
    /// timecode file, which has no quarter notes.
    ///
    /// **Not [`Self::ticks_per_quarter`]**, which is 0 for a timecode file. A lyric view that reads
    /// ahead eight beats would read ahead eight ticks.
    pub fn beat_ticks(&self) -> u16 {
        effective_tpqn(&self.tempo_map)
    }

    /// Channels that carry at least one note, ordered ascending.
    ///
    /// Used by the arrangement half of the suitability score and by melody detection.
    pub fn sounding_channels(&self) -> Vec<u8> {
        let mut seen = [false; 16];
        for event in &self.events {
            if let EventKind::NoteOn { channel, .. } = event.kind
                && let Some(slot) = seen.get_mut(usize::from(channel))
            {
                *slot = true;
            }
        }
        (0u8..16).filter(|&c| seen[usize::from(c)]).collect()
    }

    /// Total number of note-ons in the file.
    pub fn note_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .count()
    }
}

/// One text-bearing meta event, decoded, as it appears in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextEventInfo {
    /// Track it appeared on.
    pub track: usize,
    /// Absolute tick.
    pub tick: u32,
    /// Which meta event carried it, as a lowercase name.
    pub kind: &'static str,
    /// The decoded text.
    pub text: String,
}

/// Lists every text-bearing meta event in a file, decoded but not interpreted.
///
/// This is a diagnostic for the project's top risk: when a real file's lyrics come out wrong, the
/// first question is always what the file actually contains, and answering it should not require a
/// hex editor.
pub fn text_events(
    bytes: &[u8],
    declared_encoding: Option<&str>,
) -> Result<Vec<TextEventInfo>, SongError> {
    let smf = Smf::parse(bytes)?;
    let mut raw: Vec<(usize, u32, &'static str, &[u8])> = Vec::new();

    for (track_index, track) in smf.tracks.iter().enumerate() {
        let mut tick = 0u32;
        for event in track {
            tick = tick.saturating_add(event.delta.as_int());
            if let TrackEventKind::Meta(meta) = event.kind {
                let (kind, payload) = match meta {
                    MetaMessage::Text(b) => ("text", b),
                    MetaMessage::Lyric(b) => ("lyric", b),
                    MetaMessage::TrackName(b) => ("track_name", b),
                    MetaMessage::Copyright(b) => ("copyright", b),
                    MetaMessage::Marker(b) => ("marker", b),
                    MetaMessage::InstrumentName(b) => ("instrument_name", b),
                    MetaMessage::CuePoint(b) => ("cue_point", b),
                    MetaMessage::ProgramName(b) => ("program_name", b),
                    MetaMessage::DeviceName(b) => ("device_name", b),
                    _ => continue,
                };
                raw.push((track_index, tick, kind, payload));
            }
        }
    }

    let samples: Vec<&[u8]> = raw.iter().map(|(_, _, _, b)| *b).collect();
    let decoder = TextDecoder::resolve(&samples, declared_encoding);
    Ok(raw
        .into_iter()
        .map(|(track, tick, kind, payload)| TextEventInfo {
            track,
            tick,
            kind,
            text: decoder.decode(payload),
        })
        .collect())
}

/// Ticks per quarter note to use for heuristics, with a sane value for SMPTE files.
fn effective_tpqn(tempo_map: &TempoMap) -> u16 {
    match *tempo_map.timebase() {
        Timebase::Metrical { ticks_per_quarter } => ticks_per_quarter.max(1),
        // For a timecode file, treat half a second as the nominal beat.
        Timebase::Smpte { ticks_per_second } => u16::try_from(ticks_per_second / 2)
            .unwrap_or(u16::MAX)
            .max(1),
    }
}
