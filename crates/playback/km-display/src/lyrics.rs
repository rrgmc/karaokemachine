//! Deciding what lyrics to show, and how far the highlight has crossed them.
//!
//! Pure logic: it takes a tick and a [`LyricTimeline`] and says which lines belong on screen and
//! where the wipe has reached. No SDL, no fonts, no pixels — so the behavior that makes a karaoke
//! machine feel right is testable rather than something to squint at.
//!
//! Pixels come later: this reports *which syllable* is current and *how far through it* the
//! highlight is, and the renderer converts that to an x position using measured glyph widths. Doing
//! it the other way round -- interpolating position linearly across a line -- makes the highlight
//! drift away from the words, because syllables are not equal widths.

use km_song::{LyricLine, LyricTimeline, TempoMap};

/// Which of the two rows a line occupies.
///
/// Commercial machines do not scroll. They hold two fixed rows and alternate: while the singer is on
/// the line in row 0, the next line is already sitting in row 1 to be read ahead, and when row 0 is
/// finished it is replaced by the line after that. Alternating by line index reproduces that.
pub type Row = usize;

/// Rows the display holds.
pub const ROWS: usize = 2;

/// How far the lyric highlight may be shifted from the audio, either way, in milliseconds.
///
/// Wide enough to cover any real display latency -- realistic values are 0-60 -- and narrow enough
/// that a mistyped setting cannot put the highlight in a different part of the song. The clamp is
/// applied wherever the number is read, the way [`km_queue::MicChannel`]'s is, because a
/// hand-edited settings file can hold anything and must still boot.
pub const MAX_LYRIC_OFFSET_MS: i16 = 500;

/// A line placed on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleLine {
    /// Index into the timeline's lines.
    pub index: usize,
    /// Which row it occupies.
    pub row: Row,
    /// Whether this is the line being sung now.
    pub is_current: bool,
    /// The syllable being sung, if this is the current line and singing has reached it.
    pub syllable: Option<usize>,
    /// How far through that syllable the highlight has traveled, 0.0 to 1.0.
    pub syllable_progress: f32,
}

/// What to draw for one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricFrame {
    /// Lines to draw, at most one per row.
    pub lines: Vec<VisibleLine>,
    /// Page the current line belongs to, for a page-change effect.
    pub page: u16,
    /// Whether the song has lyrics at all.
    pub has_lyrics: bool,
}

impl LyricFrame {
    /// The line occupying a row, if any.
    pub fn line_in_row(&self, row: Row) -> Option<&VisibleLine> {
        self.lines.iter().find(|line| line.row == row)
    }

    /// The line being sung.
    pub fn current(&self) -> Option<&VisibleLine> {
        self.lines.iter().find(|line| line.is_current)
    }
}

/// Turns a playback position into a frame to draw.
#[derive(Debug, Clone, Copy)]
pub struct LyricView {
    /// How long before a line starts it appears, in ticks.
    ///
    /// A line that appeared exactly as it began would be unreadable; a singer needs it in advance.
    pub lead_in_ticks: u32,
}

/// Beats of lead-in: two bars of common time.
///
/// Generous on purpose. The whole point of the second row is reading ahead, so under normal singing
/// the next line should already be there. The threshold exists only to stop a line appearing during
/// a long instrumental break, minutes before anybody sings it.
const LEAD_IN_BEATS: u32 = 8;

impl Default for LyricView {
    fn default() -> Self {
        // At the common 480 ticks per quarter note.
        Self {
            lead_in_ticks: 480 * LEAD_IN_BEATS,
        }
    }
}

impl LyricView {
    /// Thresholds scaled to a song's resolution.
    pub fn for_ticks_per_quarter(ticks_per_quarter: u16) -> Self {
        Self {
            lead_in_ticks: u32::from(ticks_per_quarter.max(1)) * LEAD_IN_BEATS,
        }
    }

    /// Works out what to show at `tick`.
    pub fn frame(&self, timeline: &LyricTimeline, tick: u32) -> LyricFrame {
        if timeline.is_empty() {
            return LyricFrame::default();
        }

        let current_index = self.current_index(timeline, tick);
        let mut lines = Vec::with_capacity(ROWS);

        for offset in 0..ROWS {
            let index = current_index + offset;
            let Some(line) = timeline.lines.get(index) else {
                break;
            };
            // A line further ahead than the lead-in is not shown yet, so the second row stays empty
            // through a long instrumental rather than showing a line minutes early.
            if offset > 0 && line.start_tick > tick.saturating_add(self.lead_in_ticks) {
                break;
            }
            let is_current = offset == 0;
            let (syllable, syllable_progress) = if is_current {
                Self::progress_within(line, tick)
            } else {
                (None, 0.0)
            };
            lines.push(VisibleLine {
                index,
                row: index % ROWS,
                is_current,
                syllable,
                syllable_progress,
            });
        }

        let page = timeline
            .lines
            .get(current_index)
            .map_or(0, |line| line.page);

        LyricFrame {
            lines,
            page,
            has_lyrics: true,
        }
    }

    /// The line the singer is on.
    ///
    /// During a gap between lines the previous line stays current, so it remains on screen fully
    /// highlighted instead of the display going blank between phrases.
    fn current_index(&self, timeline: &LyricTimeline, tick: u32) -> usize {
        if let Some(index) = timeline.line_at_tick(tick) {
            return index;
        }
        // Before the first line: hold at the start so the opening line is visible during the intro.
        if timeline.lines.first().is_some_and(|l| tick < l.start_tick) {
            return 0;
        }
        // Otherwise the last line that has started.
        timeline
            .lines
            .iter()
            .rposition(|line| line.start_tick <= tick)
            .unwrap_or(0)
    }

    /// Which syllable is being sung and how far through it, within one line.
    fn progress_within(line: &LyricLine, tick: u32) -> (Option<usize>, f32) {
        if line.syllables.is_empty() || tick < line.start_tick {
            return (None, 0.0);
        }
        if tick >= line.end_tick {
            // Finished: the highlight sits at the end of the last syllable.
            return (Some(line.syllables.len() - 1), 1.0);
        }
        for (index, syllable) in line.syllables.iter().enumerate() {
            if tick < syllable.start_tick {
                // In a gap before this syllable; the previous one is complete and stays lit.
                return match index.checked_sub(1) {
                    Some(previous) => (Some(previous), 1.0),
                    None => (None, 0.0),
                };
            }
            if tick < syllable.end_tick {
                let span = syllable.end_tick.saturating_sub(syllable.start_tick);
                let progress = if span == 0 {
                    1.0
                } else {
                    (tick - syllable.start_tick) as f32 / span as f32
                };
                return (Some(index), progress.clamp(0.0, 1.0));
            }
        }
        (Some(line.syllables.len() - 1), 1.0)
    }
}

/// Shifts a playback tick by a display offset in wall-clock milliseconds.
///
/// Positive `offset_ms` moves the highlight ahead of the audio, which is the direction that
/// compensates a late picture -- a television's panel processing, or an audio path taken out of the
/// HDMI chain early so microphones can be mixed into it. That is the case nearly every installation
/// has; negative covers an external audio path with buffering of its own.
///
/// `tempo_ratio` converts wall time to song time: the sequencer advances song time at that multiple
/// of real time, so at 1.25x speed 40 ms of real latency is 50 ms of song time. Without the scaling
/// the correction drifts as soon as the speed control is used.
///
/// This is a *display* correction and nothing else. The audio is already right, and the tick the
/// API publishes is left alone -- see "The lyric timing offset" in `docs/ARCHITECTURE.md`.
pub fn shift_ticks(tempo_map: &TempoMap, tick: u32, offset_ms: i16, tempo_ratio: f32) -> u32 {
    // Load-bearing rather than an optimization: at the shipped default of 0 the drawn tick is the
    // published tick bit for bit, so a machine that never sets this cannot be moved by a rounding
    // difference in the tick -> us -> tick round trip.
    if offset_ms == 0 {
        return tick;
    }
    // Both inputs are bounded before they are multiplied, and that is load-bearing rather than
    // defensive dressing. A float-to-int cast in Rust saturates, so an unbounded ratio makes
    // `delta_us` `i64::MAX`; the addition below then overflows, and even if it did not,
    // `TempoMap::us_to_tick` multiplies what it is given by the ticks-per-quarter and overflows
    // there instead. A debug build panics, and that would take the display thread out over a number
    // in a settings file. Both bounds are the real ones: the offset's own range, and the ceiling the
    // engine already holds the tempo control to.
    //
    // The floor is 0 rather than `km_queue::MIN_TEMPO_RATIO`, deliberately. Clamping *up* to the
    // engine's minimum would invent movement where a caller said there was none; a ratio of zero
    // means song time is not advancing, and no amount of real latency is any amount of song time.
    // A NaN ratio falls through the same way -- `clamp` keeps it and the cast turns it into 0.
    let offset_ms = offset_ms.clamp(-MAX_LYRIC_OFFSET_MS, MAX_LYRIC_OFFSET_MS);
    let tempo_ratio = tempo_ratio.clamp(0.0, km_queue::MAX_TEMPO_RATIO);
    let delta_us = (f64::from(offset_ms) * 1_000.0 * f64::from(tempo_ratio)) as i64;
    let us = (tempo_map.tick_to_us(tick) as i64).saturating_add(delta_us);
    // Saturating at 0 rather than wrapping: a negative offset early in a song asks for a tick before
    // the beginning, and the beginning is the honest answer.
    tempo_map.us_to_tick(us.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;

    fn timeline(bytes: &[u8]) -> LyricTimeline {
        Song::parse(bytes, &ParseOptions::default())
            .expect("fixture parses")
            .lyrics
    }

    fn view() -> LyricView {
        LyricView::for_ticks_per_quarter(testing::TPQN)
    }

    fn tempo_map(bytes: &[u8]) -> TempoMap {
        Song::parse(bytes, &ParseOptions::default())
            .expect("fixture parses")
            .tempo_map
    }

    /// A plain 120 BPM map: one quarter note is 500 ms, so at `TPQN` 480 a tick is 1041.67 us.
    fn steady() -> TempoMap {
        tempo_map(&testing::soft_karaoke())
    }

    #[test]
    fn a_song_with_no_lyrics_produces_an_empty_frame() {
        let frame = view().frame(&timeline(&testing::instrumental()), 0);
        assert!(!frame.has_lyrics);
        assert!(frame.lines.is_empty());
        assert!(frame.current().is_none());
    }

    #[test]
    fn the_first_two_lines_are_shown_at_the_start() {
        let lyrics = timeline(&testing::soft_karaoke());
        let frame = view().frame(&lyrics, 0);
        assert_eq!(
            frame.lines.len(),
            2,
            "a singer needs the next line to read ahead"
        );
        assert_eq!(frame.lines[0].index, 0);
        assert!(frame.lines[0].is_current);
        assert_eq!(frame.lines[1].index, 1);
        assert!(!frame.lines[1].is_current);
    }

    #[test]
    fn lines_alternate_between_the_two_rows() {
        let lyrics = timeline(&testing::soft_karaoke());
        let frame = view().frame(&lyrics, 0);
        assert_eq!(frame.lines[0].row, 0);
        assert_eq!(frame.lines[1].row, 1);

        // On the second line, it holds row 1 and the line after it takes row 0 -- the alternation
        // that lets a real machine avoid scrolling.
        let second_start = lyrics.lines[1].start_tick;
        let frame = view().frame(&lyrics, second_start);
        assert_eq!(frame.current().expect("a current line").row, 1);
    }

    #[test]
    fn the_current_line_follows_the_playback_position() {
        let lyrics = timeline(&testing::soft_karaoke());
        let second_start = lyrics.lines[1].start_tick;

        let frame = view().frame(&lyrics, second_start);
        assert_eq!(frame.current().expect("current").index, 1);
    }

    #[test]
    fn the_highlight_advances_through_the_syllables_of_a_line() {
        let lyrics = timeline(&testing::soft_karaoke());
        let line = &lyrics.lines[0];
        assert!(line.syllables.len() > 3, "the fixture has syllable timing");

        let first = view().frame(&lyrics, line.syllables[0].start_tick);
        assert_eq!(first.current().and_then(|l| l.syllable), Some(0));

        let third = view().frame(&lyrics, line.syllables[2].start_tick);
        assert_eq!(third.current().and_then(|l| l.syllable), Some(2));
    }

    #[test]
    fn progress_within_a_syllable_is_proportional() {
        let lyrics = timeline(&testing::soft_karaoke());
        let syllable = &lyrics.lines[0].syllables[1];
        let span = syllable.end_tick - syllable.start_tick;

        let quarter = view().frame(&lyrics, syllable.start_tick + span / 4);
        let progress = quarter.current().expect("current").syllable_progress;
        assert!(
            (0.15..=0.35).contains(&progress),
            "a quarter of the way in should read about 0.25, got {progress}"
        );

        let start = view().frame(&lyrics, syllable.start_tick);
        assert!(start.current().expect("current").syllable_progress < 0.05);
    }

    #[test]
    fn a_finished_line_stays_fully_highlighted() {
        let lyrics = timeline(&testing::soft_karaoke());
        let line = &lyrics.lines[0];
        // One tick before the next line starts, the first line is done but still on screen.
        let frame = view().frame(&lyrics, line.end_tick.saturating_sub(1));
        let current = frame.current().expect("current");
        assert!(current.syllable.is_some());
    }

    #[test]
    fn the_display_does_not_go_blank_in_the_gap_between_lines() {
        let lyrics = timeline(&testing::soft_karaoke());
        // Between the end of line 0 and the start of line 1.
        let gap = lyrics.lines[0].end_tick + 1;
        if gap < lyrics.lines[1].start_tick {
            let frame = view().frame(&lyrics, gap);
            assert!(
                frame.current().is_some(),
                "something must stay on screen during a gap"
            );
        }
    }

    #[test]
    fn a_line_far_in_the_future_is_not_shown_early() {
        let lyrics = timeline(&testing::unmarked_lyrics());
        assert_eq!(lyrics.line_count(), 2);
        // The fixture separates its two lines by two seconds, well beyond the lead-in.
        let frame = LyricView { lead_in_ticks: 10 }.frame(&lyrics, 0);
        assert_eq!(
            frame.lines.len(),
            1,
            "the second row should stay empty until the line is nearly due"
        );
    }

    #[test]
    fn a_line_just_ahead_is_shown_so_it_can_be_read() {
        let lyrics = timeline(&testing::unmarked_lyrics());
        let frame = LyricView {
            lead_in_ticks: 100_000,
        }
        .frame(&lyrics, 0);
        assert_eq!(
            frame.lines.len(),
            2,
            "with a long lead-in both lines appear"
        );
    }

    #[test]
    fn the_last_line_leaves_the_second_row_empty() {
        let lyrics = timeline(&testing::soft_karaoke());
        let last = lyrics.lines.last().expect("a line");
        let frame = view().frame(&lyrics, last.start_tick);
        assert_eq!(frame.lines.len(), 1);
        assert_eq!(
            frame.current().expect("current").index,
            lyrics.line_count() - 1
        );
    }

    #[test]
    fn a_position_past_the_end_holds_the_final_line() {
        let lyrics = timeline(&testing::soft_karaoke());
        let frame = view().frame(&lyrics, u32::MAX / 2);
        assert_eq!(
            frame.current().expect("current").index,
            lyrics.line_count() - 1
        );
        assert_eq!(frame.current().expect("current").syllable_progress, 1.0);
    }

    #[test]
    fn the_page_number_of_the_current_line_is_reported() {
        let lyrics = timeline(&testing::soft_karaoke());
        let frame = view().frame(&lyrics, 0);
        assert_eq!(frame.page, lyrics.lines[0].page);
    }

    #[test]
    fn frames_are_monotonic_through_a_whole_song() {
        // Sweeping the whole song must never panic, never regress the current line, and never
        // report a syllable index outside its line.
        let lyrics = timeline(&testing::high_quality_song());
        let view = view();
        let mut last_index = 0usize;
        let end = lyrics.lines.last().expect("lines").end_tick;

        for tick in (0..=end).step_by(97) {
            let frame = view.frame(&lyrics, tick);
            let current = frame.current().expect("always a current line");
            assert!(
                current.index >= last_index,
                "the current line went backwards at tick {tick}"
            );
            last_index = current.index;

            let line = &lyrics.lines[current.index];
            if let Some(syllable) = current.syllable {
                assert!(
                    syllable < line.syllables.len(),
                    "syllable {syllable} out of range at tick {tick}"
                );
            }
            assert!((0.0..=1.0).contains(&current.syllable_progress));
            assert!(frame.lines.len() <= ROWS);
            // No two lines may share a row.
            if frame.lines.len() == 2 {
                assert_ne!(frame.lines[0].row, frame.lines[1].row);
            }
        }
    }

    // -- the display offset ----------------------------------------------------------------------

    #[test]
    fn a_zero_offset_returns_the_tick_unchanged() {
        let map = steady();
        // Bit for bit, at every tick, and at a tempo where the round trip would not be exact. This
        // is the property the early return exists for.
        for tick in [0, 1, 479, 480, 961, 100_000] {
            assert_eq!(shift_ticks(&map, tick, 0, 1.0), tick);
            assert_eq!(shift_ticks(&map, tick, 0, 1.25), tick);
        }
    }

    #[test]
    fn a_positive_offset_advances_and_a_negative_one_retreats() {
        let map = steady();
        let tick = 4 * u32::from(testing::TPQN);
        assert!(shift_ticks(&map, tick, 100, 1.0) > tick);
        assert!(shift_ticks(&map, tick, -100, 1.0) < tick);
    }

    #[test]
    fn a_hundred_milliseconds_is_ninety_six_ticks_at_120_bpm() {
        // 100 ms of a 500 ms quarter note is 96 of its 480 ticks. Stated as a number rather than a
        // comparison so a change to the conversion has to be deliberate.
        let map = steady();
        let tick = 4 * u32::from(testing::TPQN);
        assert_eq!(shift_ticks(&map, tick, 100, 1.0), tick + 96);
        assert_eq!(shift_ticks(&map, tick, -100, 1.0), tick - 96);
    }

    #[test]
    fn the_offset_is_scaled_by_the_tempo_ratio() {
        // Faster playback covers more song time per second, so a fixed real latency is more song
        // time. Without this the correction drifts the moment the speed control is touched. The two
        // ratios are the ends of the range the engine allows: 100 ms is 125 ms of song time at
        // 1.25x and 75 ms at 0.75x, which at 1041.67 us a tick is 120 and 72 ticks.
        let map = steady();
        let tick = 4 * u32::from(testing::TPQN);
        assert_eq!(shift_ticks(&map, tick, 100, 1.25), tick + 120);
        assert_eq!(shift_ticks(&map, tick, 100, 0.75), tick + 72);
    }

    #[test]
    fn an_out_of_range_tempo_ratio_is_held_to_the_engines_ceiling() {
        // `Machine::new` seeds the live tempo from settings.json without validating it, so a
        // hand-edited file really can put a nonsense ratio here.
        let map = steady();
        let tick = 4 * u32::from(testing::TPQN);
        let ceiling = shift_ticks(&map, tick, 100, km_queue::MAX_TEMPO_RATIO);
        assert_eq!(shift_ticks(&map, tick, 100, 99.0), ceiling);
        assert_eq!(shift_ticks(&map, tick, 100, f32::MAX), ceiling);
        // Negative is not "backwards"; it is nonsense, and it stops at no shift at all.
        assert_eq!(shift_ticks(&map, tick, 100, -5.0), tick);
    }

    #[test]
    fn a_negative_offset_at_the_start_stops_at_tick_zero() {
        // Rather than wrapping through a u32 and landing at the end of the song.
        let map = steady();
        assert_eq!(shift_ticks(&map, 0, -500, 1.0), 0);
        assert_eq!(shift_ticks(&map, 10, -500, 1.0), 0);
    }

    #[test]
    fn a_tempo_change_is_shifted_through_the_map_not_by_a_fixed_tick_count() {
        // The same 100 ms is worth 96 ticks at 120 BPM and 48 at 60 BPM. A correction added as a
        // constant number of ticks would be wrong on one side of the change; this is the test that
        // catches it.
        let map = tempo_map(&testing::tempo_change());
        let tpqn = u32::from(testing::TPQN);
        let before = 480;
        let after = 3 * tpqn;
        assert_eq!(shift_ticks(&map, before, 100, 1.0) - before, 96);
        assert_eq!(shift_ticks(&map, after, 100, 1.0) - after, 48);
    }

    #[test]
    fn extreme_offsets_and_ratios_do_not_panic() {
        // A hand-edited settings file can hold anything, and the clamp is what stands between it and
        // an arithmetic overflow here.
        let map = steady();
        let tick = 4 * u32::from(testing::TPQN);
        for offset in [
            i16::MIN,
            i16::MAX,
            -MAX_LYRIC_OFFSET_MS,
            MAX_LYRIC_OFFSET_MS,
        ] {
            for ratio in [0.0, 1.0, f32::MAX] {
                let _ = shift_ticks(&map, tick, offset, ratio);
                let _ = shift_ticks(&map, 0, offset, ratio);
                let _ = shift_ticks(&map, u32::MAX, offset, ratio);
            }
        }
    }

    #[test]
    fn a_zero_tempo_ratio_leaves_the_tick_where_it_is() {
        // Song time is not advancing, so no amount of real latency is any amount of song time.
        let map = steady();
        let tick = 4 * u32::from(testing::TPQN);
        assert_eq!(shift_ticks(&map, tick, 100, 0.0), tick);
    }

    /// **The invariant the wipe stands on.** `draw` measures the line from its syllables and renders
    /// it from `text()` — two traversals, and the highlight lands on the wrong glyph if they differ
    /// by one byte. A file whose word gaps are narrowed is where a separator would creep in.
    #[test]
    fn a_line_reads_the_same_whether_walked_by_syllable_or_joined() {
        for build in [
            testing::word_ends_unmarked as fn() -> Vec<u8>,
            testing::lyric_events,
            testing::soft_karaoke,
        ] {
            for line in &timeline(&build()).lines {
                let joined: String = line.syllables.iter().map(|s| s.text.as_str()).collect();
                assert_eq!(line.text(), joined);
            }
        }
    }
}
