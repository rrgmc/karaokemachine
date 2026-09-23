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

use km_song::{LyricGranularity, LyricLine, LyricTimeline, TempoMap};

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
    /// How strongly to draw the line, 0.0 to 1.0.
    ///
    /// Below 1 only for a line-timed song's line after its singing ends, before a long gap. See
    /// [`LyricView::hold_ticks`].
    pub opacity: f32,
    /// How far the lead-in cue has filled, 0.0 to 1.0, for a line about to start after a long gap.
    ///
    /// Only a line-timed song has one. See [`LyricView::cue_ticks`].
    pub cue: Option<f32>,
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
///
/// **A line-timed song is drawn a line at a time.** Its file says when each line starts and nothing
/// about the words inside it, so the whole line lights at its start rather than being wiped across
/// at a speed the file never gave. Three thresholds serve that mode only; a song timed by the
/// syllable keeps its wipe and ignores them. See `A line-timed song lights a line at a time` in
/// `docs/decisions/interface.md`.
#[derive(Debug, Clone, Copy)]
pub struct LyricView {
    /// How long before a line starts it appears, in ticks.
    ///
    /// A line that appeared exactly as it began would be unreadable; a singer needs it in advance.
    pub lead_in_ticks: u32,
    /// How long a line-timed line counts as sung, in ticks, when its file does not say.
    ///
    /// A line runs until the next one starts, and that includes any solo after it. A line still lit
    /// over a guitar break reads as words the singer has missed.
    pub hold_ticks: u32,
    /// How long a line-timed line takes to fade once its singing is over, in ticks.
    pub fade_ticks: u32,
    /// How long the lead-in cue takes to fill before a line-timed line starts, in ticks.
    pub cue_ticks: u32,
    /// How long a gap has to be before a line-timed line for it to fade out and cue in, in ticks.
    ///
    /// Between two lines sung back to back a cue would flicker, and a line fading for a breath would
    /// read as a fault.
    pub cue_min_gap_ticks: u32,
}

/// Beats of lead-in: two bars of common time.
///
/// Generous on purpose. The whole point of the second row is reading ahead, so under normal singing
/// the next line should already be there. The threshold exists only to stop a line appearing during
/// a long instrumental break, minutes before anybody sings it.
const LEAD_IN_BEATS: u32 = 8;

/// Beats a line-timed line counts as sung when its file does not say: two bars.
const LINE_HOLD_BEATS: u32 = 8;

/// Beats a line-timed line takes to fade out.
const FADE_BEATS: u32 = 1;

/// Beats the lead-in cue takes to fill: one bar, which is how a count-in is felt.
const CUE_BEATS: u32 = 4;

/// Beats of gap before a line-timed line that earn a fade and a cue.
const CUE_MIN_GAP_BEATS: u32 = 6;

// A cued line has to be on screen already, or the cue fills under nothing.
const _: () = assert!(CUE_BEATS <= LEAD_IN_BEATS);

impl Default for LyricView {
    fn default() -> Self {
        // At the common 480 ticks per quarter note.
        Self::for_ticks_per_quarter(480)
    }
}

impl LyricView {
    /// Thresholds scaled to a song's resolution.
    pub fn for_ticks_per_quarter(ticks_per_quarter: u16) -> Self {
        let beat = u32::from(ticks_per_quarter.max(1));
        Self {
            lead_in_ticks: beat * LEAD_IN_BEATS,
            hold_ticks: beat * LINE_HOLD_BEATS,
            fade_ticks: beat * FADE_BEATS,
            cue_ticks: beat * CUE_BEATS,
            cue_min_gap_ticks: beat * CUE_MIN_GAP_BEATS,
        }
    }

    /// Works out what to show at `tick`.
    pub fn frame(&self, timeline: &LyricTimeline, tick: u32) -> LyricFrame {
        if timeline.is_empty() {
            return LyricFrame::default();
        }

        let current_index = self.current_index(timeline, tick);
        let line_timed = timeline.granularity() == LyricGranularity::LineLevel;
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
            let (syllable, syllable_progress) = match (is_current, line_timed) {
                (false, _) => (None, 0.0),
                (true, false) => Self::progress_within(line, tick),
                // **Lit whole at its start.** The last syllable at full progress is the whole line
                // in the sung color, which the renderer already draws.
                (true, true) if tick >= line.start_tick && !line.syllables.is_empty() => {
                    (Some(line.syllables.len() - 1), 1.0)
                }
                (true, true) => (None, 0.0),
            };
            let (opacity, cue) = if line_timed {
                (
                    self.opacity(timeline, index, tick),
                    self.cue(timeline, index, tick),
                )
            } else {
                (1.0, None)
            };
            lines.push(VisibleLine {
                index,
                row: index % ROWS,
                is_current,
                syllable,
                syllable_progress,
                opacity,
                cue,
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

    /// The tick a line-timed line's singing ends by.
    ///
    /// **The file's own end where it gave one**: a blank line in an LRC file, or the last line's hold.
    /// Otherwise the line ran to the next one's start, which says nothing about when the singing
    /// stopped, and [`Self::hold_ticks`] stands in for it.
    fn sung_until(&self, timeline: &LyricTimeline, index: usize) -> u32 {
        let line = &timeline.lines[index];
        match timeline.lines.get(index + 1) {
            Some(next) if line.end_tick >= next.start_tick => line
                .end_tick
                .min(line.start_tick.saturating_add(self.hold_ticks)),
            _ => line.end_tick,
        }
    }

    /// The silence before a line-timed line, in ticks: from the song's start for the first line.
    fn gap_before(&self, timeline: &LyricTimeline, index: usize) -> u32 {
        let start = timeline.lines[index].start_tick;
        match index.checked_sub(1) {
            Some(previous) => start.saturating_sub(self.sung_until(timeline, previous)),
            None => start,
        }
    }

    /// How strongly a line-timed line is drawn at `tick`.
    ///
    /// Full until its singing ends. **It fades only before a long gap**: a line followed at once by
    /// the next is replaced before a fade could be seen, and one fading over a breath looks broken.
    fn opacity(&self, timeline: &LyricTimeline, index: usize, tick: u32) -> f32 {
        let sung_until = self.sung_until(timeline, index);
        let long_gap_follows = timeline
            .lines
            .get(index + 1)
            .is_none_or(|_| self.gap_before(timeline, index + 1) >= self.cue_min_gap_ticks);
        if tick <= sung_until || !long_gap_follows {
            return 1.0;
        }
        let faded = (tick - sung_until) as f32 / self.fade_ticks.max(1) as f32;
        (1.0 - faded).clamp(0.0, 1.0)
    }

    /// How far a line-timed line's lead-in cue has filled at `tick`, if it has one.
    ///
    /// **The cue ends exactly on the line's start**, the one moment the file knows. Only a line after
    /// a long gap has one, the song's first line included: coming back in after a break is what a
    /// singer misses.
    fn cue(&self, timeline: &LyricTimeline, index: usize, tick: u32) -> Option<f32> {
        let start = timeline.lines[index].start_tick;
        let from = start.saturating_sub(self.cue_ticks);
        if tick >= start || tick < from || self.gap_before(timeline, index) < self.cue_min_gap_ticks
        {
            return None;
        }
        Some(((tick - from) as f32 / self.cue_ticks.max(1) as f32).clamp(0.0, 1.0))
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
        let frame = LyricView {
            lead_in_ticks: 10,
            ..view()
        }
        .frame(&lyrics, 0);
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
            ..view()
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

    /// A line-timed timeline in milliseconds, as an LRC file gives one: a line per timestamp.
    fn line_timed(text: &str) -> LyricTimeline {
        km_song::lrc::parse(text.as_bytes())
            .expect("parses")
            .timeline
    }

    /// The thresholds a millisecond timeline is drawn with: a nominal beat of half a second.
    fn ms_view() -> LyricView {
        LyricView::for_ticks_per_quarter(500)
    }

    /// Three lines sung back to back, a solo, and a line after it.
    const WITH_A_SOLO: &str = "[00:10.00]First line\n[00:13.00]Second line\n[00:16.00]Last before the \
        solo\n[00:40.00]After the solo\n";

    #[test]
    fn a_line_timed_line_lights_whole_at_its_start_and_not_before() {
        let lyrics = line_timed(WITH_A_SOLO);
        let before = ms_view().frame(&lyrics, 9_990);
        assert_eq!(before.current().expect("current").syllable, None);

        let at_start = ms_view().frame(&lyrics, 10_000);
        let current = at_start.current().expect("current");
        assert_eq!(
            (current.syllable, current.syllable_progress),
            (Some(0), 1.0)
        );
        // Nothing crawls across it: a moment later it is exactly as lit.
        let later = ms_view().frame(&lyrics, 12_000);
        assert_eq!(later.current().expect("current").syllable_progress, 1.0);
    }

    #[test]
    fn a_syllable_timed_song_still_wipes() {
        let lyrics = timeline(&testing::soft_karaoke());
        let line = &lyrics.lines[0];
        let middle = line.syllables[1].start_tick
            + (line.syllables[1].end_tick - line.syllables[1].start_tick) / 2;
        let current = view().frame(&lyrics, middle).lines[0].clone();
        assert_eq!(current.syllable, Some(1));
        assert!(current.syllable_progress < 1.0);
        assert_eq!((current.opacity, current.cue), (1.0, None));
    }

    #[test]
    fn a_line_followed_at_once_by_the_next_never_fades_and_the_next_has_no_cue() {
        let lyrics = line_timed(WITH_A_SOLO);
        let frame = ms_view().frame(&lyrics, 12_900);
        assert_eq!(frame.lines[0].opacity, 1.0);
        assert_eq!(frame.lines[1].cue, None);
    }

    #[test]
    fn a_line_before_a_solo_fades_once_its_hold_is_over() {
        let lyrics = line_timed(WITH_A_SOLO);
        let hold = ms_view().hold_ticks;
        let sung_until = 16_000 + hold;
        let frame = ms_view().frame(&lyrics, sung_until);
        assert_eq!(frame.current().expect("current").opacity, 1.0);

        let half = ms_view().frame(&lyrics, sung_until + ms_view().fade_ticks / 2);
        let opacity = half.current().expect("current").opacity;
        assert!(opacity > 0.0 && opacity < 1.0, "{opacity}");

        let gone = ms_view().frame(&lyrics, sung_until + ms_view().fade_ticks);
        let current = gone.current().expect("still the current line");
        assert_eq!((current.index, current.opacity), (2, 0.0));
    }

    #[test]
    fn a_blank_line_in_the_file_ends_the_singing_there() {
        let lyrics = line_timed("[00:10.00]Before\n[00:12.00]\n[00:40.00]After\n");
        let frame = ms_view().frame(&lyrics, 12_000 + ms_view().fade_ticks);
        assert_eq!(frame.current().expect("current").opacity, 0.0);
    }

    #[test]
    fn the_line_after_a_solo_is_cued_in_and_the_cue_ends_on_its_start() {
        let lyrics = line_timed(WITH_A_SOLO);
        let view = ms_view();
        let early = view.frame(&lyrics, 40_000 - view.cue_ticks - 1);
        assert!(early.lines.iter().all(|line| line.cue.is_none()));

        let halfway = view.frame(&lyrics, 40_000 - view.cue_ticks / 2);
        let next = halfway
            .lines
            .iter()
            .find(|line| line.index == 3)
            .expect("the line after the solo is on screen");
        let cue = next.cue.expect("cued");
        assert!((cue - 0.5).abs() < 0.01, "{cue}");

        let started = view.frame(&lyrics, 40_000);
        let current = started.current().expect("current");
        assert_eq!((current.index, current.cue), (3, None));
    }

    #[test]
    fn the_first_line_is_cued_in_after_the_intro() {
        let lyrics = line_timed(WITH_A_SOLO);
        let view = ms_view();
        let frame = view.frame(&lyrics, 10_000 - view.cue_ticks / 4);
        let first = frame
            .current()
            .expect("the first line waits in the top row");
        assert_eq!(first.index, 0);
        assert!(first.cue.is_some_and(|cue| cue > 0.7));
    }
}
