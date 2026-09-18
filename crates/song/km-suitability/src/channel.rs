//! Per-channel measurements taken from a parsed song.
//!
//! Both melody detection and the arrangement half of the suitability score work from these, so they
//! are computed once and shared.

use km_song::{EventKind, Song};
use serde::Serialize;

use crate::thresholds::Thresholds;

/// The drum channel, 0-based. Note numbers select instruments here rather than pitches, so it is
/// excluded from anything that reasons about melody or range.
pub const DRUM_CHANNEL: u8 = 9;

/// What one channel does over the course of a song.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChannelStats {
    /// The channel, 0-based.
    pub channel: u8,
    /// Number of note-ons.
    pub note_count: u32,
    /// Note-on ticks, ascending.
    pub onset_ticks: Vec<u32>,
    /// Lowest note played.
    pub min_key: u8,
    /// Highest note played.
    pub max_key: u8,
    /// Median note, which describes the register better than the extremes.
    pub median_key: u8,
    /// Fraction of notes inside the plausible singing range.
    ///
    /// A fraction rather than a min/max test, so a single outlier note cannot disqualify a real
    /// melody line while a bass part is still ruled out decisively.
    pub vocal_key_fraction: f32,
    /// Fraction of sounding time with at most one note sounding, 0.0 to 1.0.
    ///
    /// A sung line is monophonic, so this is the cheapest way to rule a channel out.
    pub monophony: f32,
    /// Ticks during which at least one note was sounding.
    pub sounding_ticks: u64,
    /// Names of tracks that put notes on this channel.
    pub track_names: Vec<String>,
    /// Programs selected on this channel, in the order first seen.
    pub programs: Vec<u8>,
}

impl ChannelStats {
    /// Whether this is the drum channel.
    pub fn is_drums(&self) -> bool {
        self.channel == DRUM_CHANNEL
    }
}

/// Measures every channel that plays a note.
///
/// Returns one entry per sounding channel, ordered by channel number.
pub fn measure(song: &Song, thresholds: &Thresholds) -> Vec<ChannelStats> {
    let mut per_channel: Vec<ChannelBuilder> = (0..16).map(ChannelBuilder::new).collect();

    for event in &song.events {
        let channel = usize::from(event.kind.channel());
        let Some(builder) = per_channel.get_mut(channel) else {
            continue;
        };
        match event.kind {
            EventKind::NoteOn { key, .. } => builder.note_on(event.tick, key, event.track),
            EventKind::NoteOff { .. } => builder.note_off(event.tick),
            EventKind::ProgramChange { program, .. } => builder.program(program),
            _ => {}
        }
    }

    per_channel
        .into_iter()
        .filter_map(|builder| builder.finish(song, thresholds))
        .collect()
}

/// Accumulates one channel's measurements in a single pass over the events.
struct ChannelBuilder {
    channel: u8,
    keys: Vec<u8>,
    onset_ticks: Vec<u32>,
    programs: Vec<u8>,
    tracks: Vec<u16>,
    /// Notes currently sounding.
    active: u32,
    /// Tick of the last change in `active`.
    last_tick: u32,
    sounding_ticks: u64,
    polyphonic_ticks: u64,
}

impl ChannelBuilder {
    fn new(channel: u8) -> Self {
        Self {
            channel,
            keys: Vec::new(),
            onset_ticks: Vec::new(),
            programs: Vec::new(),
            tracks: Vec::new(),
            active: 0,
            last_tick: 0,
            sounding_ticks: 0,
            polyphonic_ticks: 0,
        }
    }

    /// Credits the span since the last change to the sounding and polyphonic totals.
    fn advance(&mut self, tick: u32) {
        let span = u64::from(tick.saturating_sub(self.last_tick));
        if self.active >= 1 {
            self.sounding_ticks += span;
        }
        if self.active >= 2 {
            self.polyphonic_ticks += span;
        }
        self.last_tick = tick;
    }

    fn note_on(&mut self, tick: u32, key: u8, track: u16) {
        self.advance(tick);
        self.active += 1;
        self.keys.push(key);
        self.onset_ticks.push(tick);
        if !self.tracks.contains(&track) {
            self.tracks.push(track);
        }
    }

    fn note_off(&mut self, tick: u32) {
        self.advance(tick);
        // Files contain unmatched note-offs; saturating keeps the count honest rather than wrapping.
        self.active = self.active.saturating_sub(1);
    }

    fn program(&mut self, program: u8) {
        if !self.programs.contains(&program) {
            self.programs.push(program);
        }
    }

    fn finish(mut self, song: &Song, thresholds: &Thresholds) -> Option<ChannelStats> {
        if self.keys.is_empty() {
            return None;
        }
        // A note still sounding at the end of the file is held to the end, not dropped.
        self.advance(song.duration_ticks);

        let mut sorted = self.keys.clone();
        sorted.sort_unstable();
        let median_key = sorted[sorted.len() / 2];
        let in_range = sorted
            .iter()
            .filter(|&&key| (thresholds.vocal_key_min..=thresholds.vocal_key_max).contains(&key))
            .count();
        let vocal_key_fraction = in_range as f32 / sorted.len() as f32;

        let monophony = if self.sounding_ticks == 0 {
            // Every note is zero-length, so nothing ever overlaps. Treating that as monophonic
            // would hand a strong signal to a degenerate channel, so call it unknown instead.
            0.0
        } else {
            1.0 - (self.polyphonic_ticks as f32 / self.sounding_ticks as f32)
        };

        let track_names = self
            .tracks
            .iter()
            .filter_map(|&track| song.track_names.get(usize::from(track)).cloned().flatten())
            .collect();

        Some(ChannelStats {
            channel: self.channel,
            note_count: u32::try_from(self.keys.len()).unwrap_or(u32::MAX),
            onset_ticks: self.onset_ticks,
            min_key: *sorted.first().unwrap_or(&0),
            max_key: *sorted.last().unwrap_or(&0),
            median_key,
            vocal_key_fraction,
            monophony: monophony.clamp(0.0, 1.0),
            sounding_ticks: self.sounding_ticks,
            track_names,
            programs: self.programs,
        })
    }
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;

    fn song(bytes: &[u8]) -> Song {
        Song::parse(bytes, &ParseOptions::default()).expect("fixture parses")
    }

    fn channel(stats: &[ChannelStats], channel: u8) -> &ChannelStats {
        stats
            .iter()
            .find(|s| s.channel == channel)
            .unwrap_or_else(|| panic!("channel {channel} should have notes"))
    }

    #[test]
    fn only_sounding_channels_are_reported() {
        let stats = measure(
            &song(&testing::melody_and_accompaniment()),
            &Thresholds::default(),
        );
        let channels: Vec<u8> = stats.iter().map(|s| s.channel).collect();
        assert_eq!(channels, vec![0, 1, 9]);
    }

    #[test]
    fn a_single_line_melody_measures_as_monophonic() {
        let stats = measure(
            &song(&testing::melody_and_accompaniment()),
            &Thresholds::default(),
        );
        let melody = channel(&stats, 0);
        assert!(
            melody.monophony > 0.99,
            "a one-note-at-a-time channel should be monophonic, got {}",
            melody.monophony
        );
        assert_eq!(melody.note_count, 8);
        assert_eq!(melody.track_names, vec!["Melody".to_owned()]);
        assert_eq!(melody.programs, vec![73]);
    }

    #[test]
    fn chords_measure_as_polyphonic() {
        let stats = measure(
            &song(&testing::melody_and_accompaniment()),
            &Thresholds::default(),
        );
        let chords = channel(&stats, 1);
        assert!(
            chords.monophony < 0.1,
            "three notes at a time should not look monophonic, got {}",
            chords.monophony
        );
        assert_eq!(chords.track_names, vec!["Piano".to_owned()]);
    }

    #[test]
    fn the_drum_channel_is_identified() {
        let stats = measure(
            &song(&testing::melody_and_accompaniment()),
            &Thresholds::default(),
        );
        assert!(channel(&stats, 9).is_drums());
        assert!(!channel(&stats, 0).is_drums());
    }

    #[test]
    fn key_range_and_median_are_reported() {
        let stats = measure(
            &song(&testing::melody_and_accompaniment()),
            &Thresholds::default(),
        );
        let melody = channel(&stats, 0);
        // The fixture plays 62, 64, 65, 67, 65, 64, 62, 60.
        assert_eq!(melody.min_key, 60);
        assert_eq!(melody.max_key, 67);
        assert!((62..=65).contains(&melody.median_key));
    }

    #[test]
    fn onsets_are_recorded_in_order() {
        let stats = measure(
            &song(&testing::melody_and_accompaniment()),
            &Thresholds::default(),
        );
        let melody = channel(&stats, 0);
        assert!(melody.onset_ticks.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(melody.onset_ticks.len(), 8);
    }

    #[test]
    fn a_song_with_no_notes_measures_nothing() {
        let stats = measure(&song(&testing::lyric_events()), &Thresholds::default());
        assert!(stats.is_empty());
    }
}
