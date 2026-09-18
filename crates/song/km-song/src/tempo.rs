//! Tick-to-wall-clock conversion.
//!
//! The playback timeline is measured in ticks, never milliseconds: the user can change tempo
//! mid-song, and any precomputed millisecond value would silently become wrong the moment they do.
//! Milliseconds are derived on demand from here, for display and for the analysis heuristics that
//! genuinely need real time (syllable gaps, note-onset alignment).

use serde::Serialize;

/// Default tempo assumed by the MIDI spec when a file declares none: 120 BPM.
pub const DEFAULT_US_PER_QUARTER: u32 = 500_000;

/// One tempo change, with the elapsed time up to it precomputed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct TempoEntry {
    /// Tick at which this tempo takes effect.
    tick: u32,
    /// Microseconds per quarter note from this tick onward.
    us_per_quarter: u32,
    /// Microseconds elapsed from tick 0 up to (not including) `tick`.
    us_at_tick: u64,
}

/// How a file expresses time, and the tempo changes within it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Timebase {
    /// Musical time: ticks per quarter note, modulated by tempo events.
    Metrical {
        /// Ticks per quarter note, from the file header.
        ticks_per_quarter: u16,
    },
    /// SMPTE timecode: ticks are a fixed subdivision of real time, so tempo events do not apply.
    Smpte {
        /// Ticks elapsed per second of wall-clock time.
        ticks_per_second: u32,
    },
}

/// Maps ticks to wall-clock time for one song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TempoMap {
    timebase: Timebase,
    /// Sorted by tick, always non-empty, always starting at tick 0.
    entries: Vec<TempoEntry>,
}

impl TempoMap {
    /// Builds a map from a file's timebase and its tempo changes.
    ///
    /// `changes` is `(tick, microseconds per quarter note)` in any order; duplicates at the same
    /// tick keep the last one, matching how a sequencer applying events in order would behave.
    /// They arrive track by track and the sort below is stable, so a tie between two tracks
    /// resolves to the higher track index.
    /// Under [`Timebase::Smpte`] the changes are ignored, because tempo has no effect on a
    /// timecode-based file.
    pub fn new(timebase: Timebase, mut changes: Vec<(u32, u32)>) -> Self {
        if matches!(timebase, Timebase::Smpte { .. }) {
            return Self {
                timebase,
                entries: vec![TempoEntry {
                    tick: 0,
                    us_per_quarter: DEFAULT_US_PER_QUARTER,
                    us_at_tick: 0,
                }],
            };
        }

        changes.sort_by_key(|&(tick, _)| tick);

        let mut entries: Vec<TempoEntry> = Vec::with_capacity(changes.len() + 1);
        for (tick, us_per_quarter) in changes {
            // A tempo of zero is nonsense and would make time stand still; ignore it rather than
            // dividing by it later.
            if us_per_quarter == 0 {
                continue;
            }
            if tick == 0 {
                entries.clear();
                entries.push(TempoEntry {
                    tick: 0,
                    us_per_quarter,
                    us_at_tick: 0,
                });
                continue;
            }
            if entries.is_empty() {
                entries.push(TempoEntry {
                    tick: 0,
                    us_per_quarter: DEFAULT_US_PER_QUARTER,
                    us_at_tick: 0,
                });
            }
            // A later change at the same tick supersedes the one already recorded: a sequencer
            // applying events in order ends on the last of them.
            if entries.last().is_some_and(|e| e.tick == tick) {
                entries.pop();
            }
            let prev = *entries.last().expect("just ensured non-empty");
            let us_at_tick = prev.us_at_tick
                + span_us(
                    u64::from(tick - prev.tick),
                    prev.us_per_quarter,
                    ticks_per_quarter_of(&timebase),
                );
            entries.push(TempoEntry {
                tick,
                us_per_quarter,
                us_at_tick,
            });
        }
        if entries.is_empty() {
            entries.push(TempoEntry {
                tick: 0,
                us_per_quarter: DEFAULT_US_PER_QUARTER,
                us_at_tick: 0,
            });
        }

        Self { timebase, entries }
    }

    /// The file's timebase.
    pub fn timebase(&self) -> &Timebase {
        &self.timebase
    }

    /// Number of tempo changes in the file, counting the implicit starting tempo.
    pub fn change_count(&self) -> usize {
        self.entries.len()
    }

    /// Converts a tick to microseconds from the start of the song.
    pub fn tick_to_us(&self, tick: u32) -> u64 {
        if let Timebase::Smpte { ticks_per_second } = self.timebase {
            let tps = u64::from(ticks_per_second.max(1));
            return u64::from(tick) * 1_000_000 / tps;
        }
        let entry = self.entry_at(tick);
        entry.us_at_tick
            + span_us(
                u64::from(tick - entry.tick),
                entry.us_per_quarter,
                ticks_per_quarter_of(&self.timebase),
            )
    }

    /// Converts a tick to milliseconds from the start of the song.
    pub fn tick_to_ms(&self, tick: u32) -> u32 {
        u32::try_from(self.tick_to_us(tick) / 1_000).unwrap_or(u32::MAX)
    }

    /// Converts milliseconds from the start of the song back to a tick.
    ///
    /// Used for seeking, where the request arrives in real time but the sequencer works in ticks.
    pub fn ms_to_tick(&self, ms: u32) -> u32 {
        self.us_to_tick(u64::from(ms) * 1_000)
    }

    /// Converts microseconds from the start of the song to a tick.
    ///
    /// The sequencer advances in microseconds of song time -- audio blocks are a fixed number of
    /// samples, not a whole number of ticks -- and converts here on every block, so millisecond
    /// resolution would accumulate error over a song.
    pub fn us_to_tick(&self, target_us: u64) -> u32 {
        if let Timebase::Smpte { ticks_per_second } = self.timebase {
            let ticks = target_us * u64::from(ticks_per_second.max(1)) / 1_000_000;
            return u32::try_from(ticks).unwrap_or(u32::MAX);
        }
        let tpqn = ticks_per_quarter_of(&self.timebase);
        // Last entry that starts no later than the target time.
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|e| e.us_at_tick <= target_us)
            .copied()
            .unwrap_or(self.entries[0]);
        let remainder_us = target_us.saturating_sub(entry.us_at_tick);
        let ticks = remainder_us * u64::from(tpqn) / u64::from(entry.us_per_quarter);
        entry
            .tick
            .saturating_add(u32::try_from(ticks).unwrap_or(u32::MAX))
    }

    /// Microseconds per quarter note in effect at `tick`.
    pub fn us_per_quarter_at(&self, tick: u32) -> u32 {
        self.entry_at(tick).us_per_quarter
    }

    fn entry_at(&self, tick: u32) -> TempoEntry {
        match self.entries.binary_search_by_key(&tick, |e| e.tick) {
            Ok(i) => self.entries[i],
            // `Err(0)` cannot happen: the first entry is always at tick 0.
            Err(i) => self.entries[i.saturating_sub(1)],
        }
    }
}

fn ticks_per_quarter_of(timebase: &Timebase) -> u16 {
    match timebase {
        Timebase::Metrical { ticks_per_quarter } => (*ticks_per_quarter).max(1),
        // Unused for SMPTE, but must not be zero if it is ever reached.
        Timebase::Smpte { .. } => 1,
    }
}

fn span_us(ticks: u64, us_per_quarter: u32, ticks_per_quarter: u16) -> u64 {
    ticks * u64::from(us_per_quarter) / u64::from(ticks_per_quarter.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPQN: u16 = 480;

    fn metrical(changes: Vec<(u32, u32)>) -> TempoMap {
        TempoMap::new(
            Timebase::Metrical {
                ticks_per_quarter: TPQN,
            },
            changes,
        )
    }

    #[test]
    fn default_tempo_is_120_bpm() {
        let map = metrical(vec![]);
        // At 120 BPM a quarter note is 500 ms.
        assert_eq!(map.tick_to_ms(0), 0);
        assert_eq!(map.tick_to_ms(u32::from(TPQN)), 500);
        assert_eq!(map.tick_to_ms(u32::from(TPQN) * 4), 2_000);
    }

    #[test]
    fn initial_tempo_event_replaces_the_default() {
        // 1_000_000 us/quarter = 60 BPM, so a quarter note is 1 s.
        let map = metrical(vec![(0, 1_000_000)]);
        assert_eq!(map.change_count(), 1);
        assert_eq!(map.tick_to_ms(u32::from(TPQN)), 1_000);
    }

    #[test]
    fn mid_song_tempo_change_applies_from_its_tick() {
        // 120 BPM for two beats, then 60 BPM.
        let map = metrical(vec![(0, 500_000), (u32::from(TPQN) * 2, 1_000_000)]);
        assert_eq!(map.tick_to_ms(u32::from(TPQN) * 2), 1_000);
        // One further beat now takes 1 s instead of 500 ms.
        assert_eq!(map.tick_to_ms(u32::from(TPQN) * 3), 2_000);
    }

    #[test]
    fn later_tempo_change_at_the_same_tick_wins() {
        // Soft Karaoke files often write a placeholder 120 BPM immediately superseded by the real
        // tempo, both at tick 0. Keeping the first one plays the whole intro at the wrong speed.
        let map = metrical(vec![(0, 500_000), (0, 1_000_000)]);
        assert_eq!(map.change_count(), 1);
        assert_eq!(map.tick_to_ms(u32::from(TPQN)), 1_000);
    }

    #[test]
    fn later_tempo_change_at_the_same_non_zero_tick_wins() {
        // The same rule away from tick 0, which the tick-0 special case above does not cover.
        let map = metrical(vec![(0, 500_000), (960, 250_000), (960, 1_000_000)]);
        assert_eq!(map.tick_to_ms(960), 1_000);
        assert_eq!(map.tick_to_ms(1_440), 2_000);
    }

    #[test]
    fn tempo_changes_are_sorted() {
        let out_of_order = metrical(vec![(u32::from(TPQN) * 2, 1_000_000), (0, 500_000)]);
        let in_order = metrical(vec![(0, 500_000), (u32::from(TPQN) * 2, 1_000_000)]);
        assert_eq!(out_of_order, in_order);
    }

    #[test]
    fn zero_tempo_is_ignored_rather_than_dividing_by_it() {
        let map = metrical(vec![(0, 500_000), (480, 0)]);
        assert_eq!(map.change_count(), 1);
        assert_eq!(map.tick_to_ms(960), 1_000);
    }

    #[test]
    fn ms_to_tick_round_trips_across_a_tempo_change() {
        let map = metrical(vec![(0, 500_000), (u32::from(TPQN) * 2, 1_000_000)]);
        for tick in [0u32, 240, 480, 960, 1_440, 2_400] {
            let ms = map.tick_to_ms(tick);
            let back = map.ms_to_tick(ms);
            assert!(
                back.abs_diff(tick) <= 1,
                "tick {tick} -> {ms} ms -> {back}, expected to round-trip"
            );
        }
    }

    #[test]
    fn smpte_timebase_is_linear_and_ignores_tempo() {
        // 25 fps with 40 subframes = 1000 ticks per second.
        let map = TempoMap::new(
            Timebase::Smpte {
                ticks_per_second: 1_000,
            },
            vec![(0, 1_000_000)],
        );
        assert_eq!(map.tick_to_ms(1_000), 1_000);
        assert_eq!(map.tick_to_ms(2_500), 2_500);
        assert_eq!(map.ms_to_tick(1_500), 1_500);
    }
}
