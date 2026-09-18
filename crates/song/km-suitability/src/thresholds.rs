//! Every tuning number in one place.
//!
//! These are heuristics, not facts, and the corpus will keep suggesting revisions. Keeping them
//! together means a change is a change to one struct with one set of tests, rather than a hunt
//! through the detection code — and it means `km-pack reanalyze` can be pointed at a package to
//! recompute it after a revision.

/// Tunable limits for melody detection and suitability scoring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// Minimum fraction of sounding time with one note for a channel to be a melody candidate.
    ///
    /// A sung line is monophonic. Allowing a little slack absorbs overlapping note-offs from
    /// sloppy sequencing without admitting actual chords.
    pub melody_min_monophony: f32,
    /// Minimum syllable-to-note alignment for alignment alone to qualify a channel.
    pub melody_min_lyric_alignment: f32,
    /// How far ahead of the runner-up the winner must score. Below this, detection abstains.
    pub melody_margin: f32,
    /// How close a note onset must be to a syllable to count as the note being sung, in
    /// milliseconds. Wide enough for human sequencing, tight enough not to match everything.
    pub note_align_window_ms: u32,
    /// How close a note onset must be to a syllable for the channel to count as playing while it is
    /// sung, in milliseconds.
    ///
    /// Far wider than either alignment window, because it asks a coarser question: whether the
    /// channel is there under the words at all, not whether its notes are the ones being sung.
    /// Words timed a beat off the melody still pass it; a riff that plays only between the verses
    /// does not.
    pub melody_presence_window_ms: u32,
    /// Least share of syllables that must have a note on a channel within
    /// [`Self::melody_presence_window_ms`] for the channel to be a melody candidate.
    ///
    /// A gate, and the one a track name cannot talk its way past. A sung line plays while the words
    /// are sung, whatever it is called.
    pub melody_min_lyric_presence: f32,
    /// Channel 4 in 1-based terms, the common karaoke convention, used only as a tiebreaker.
    pub conventional_melody_channel: u8,

    /// Lowest note still plausibly sung.
    pub vocal_key_min: u8,
    /// Highest note still plausibly sung.
    pub vocal_key_max: u8,
    /// Lowest plausible median note. The median describes the register; the extremes do not.
    pub vocal_median_min: u8,
    /// Highest plausible median note.
    pub vocal_median_max: u8,
    /// Fraction of a channel's notes that must be inside the plausible singing range for it to be
    /// a melody candidate at all. This is a gate, not a bonus: a line nobody could sing is not the
    /// melody, whatever else it has going for it.
    pub melody_min_vocal_fraction: f32,

    /// How close a syllable must be to *some* note onset to count as synced, in milliseconds.
    /// Looser than the melody window: here the question is only whether the lyrics were timed
    /// against the music at all.
    pub sync_window_ms: u32,
    /// Alignment below which the lyrics are called badly synced.
    pub poor_sync_alignment: f32,
    /// Non-drum channels expected of a full arrangement rather than a sketch.
    pub good_channel_count: usize,
    /// Shortest plausible song length.
    pub min_duration_ms: u32,
    /// Longest plausible song length.
    pub max_duration_ms: u32,
    /// Fewest notes per minute before a file is called sparse.
    pub min_notes_per_minute: u32,

    /// Fewest syllables a file can carry and still be a song somebody sings.
    ///
    /// Below this there is nothing to follow, whatever the timing looks like. Credit blocks in this
    /// corpus run to a dozen syllables; the songs measured against them start at seventy-eight.
    pub min_lyric_syllables: usize,
    /// Least of the song's length the lyrics must span, as a fraction.
    ///
    /// The stronger of the two signals by far. Measured over 40 files of the local corpus, real songs
    /// covered 0.56 to 0.9 of their length and credit blocks covered 0.00 and 0.02 — the threshold
    /// sits in a gap with nothing in it.
    pub min_lyric_coverage: f32,
    /// Below this many syllables the lyrics are thin enough to say so, without calling them absent.
    pub sparse_lyric_syllables: usize,
    /// Below this coverage the lyrics are thin enough to say so. Real songs sit well above it: the
    /// tenth percentile of the same sample was 0.64.
    pub sparse_lyric_coverage: f32,
    /// Fewest lines of chord names a file needs before its lyric track is read as a chord chart.
    ///
    /// A floor, so that a short file whose few lines happen to be `A` and `E` is left to the quantity
    /// test rather than named for a fault it may not have.
    pub min_chord_lines: usize,
    /// Least share of a file's lyric lines that must be chord names for the file to be a chord chart.
    ///
    /// Measured over the whole local corpus: every file at 0.81 or above was a chart, and every file
    /// at 0.49 or below was a song whose words include `A` or a chord above a line. Nothing fell
    /// between 0.49 and 0.75.
    pub chord_chart_share: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            melody_min_monophony: 0.90,
            melody_min_lyric_alignment: 0.70,
            melody_margin: 1.5,
            note_align_window_ms: 60,
            melody_presence_window_ms: 1_000,
            melody_min_lyric_presence: 0.5,
            // Channel 4, 1-based.
            conventional_melody_channel: 3,

            // G2 to C6: below a bass voice and above a soprano respectively.
            vocal_key_min: 43,
            vocal_key_max: 84,
            // G3 to G5, where a sung line normally sits.
            vocal_median_min: 55,
            vocal_median_max: 79,
            melody_min_vocal_fraction: 0.90,

            sync_window_ms: 120,
            poor_sync_alignment: 0.5,
            good_channel_count: 4,
            min_duration_ms: 60_000,
            max_duration_ms: 600_000,
            min_notes_per_minute: 60,

            min_lyric_syllables: 20,
            min_lyric_coverage: 0.15,
            sparse_lyric_syllables: 60,
            sparse_lyric_coverage: 0.40,
            min_chord_lines: 8,
            chord_chart_share: 0.80,
        }
    }
}

impl Thresholds {
    /// Whether a channel's notes sit where a person could sing them.
    ///
    /// The median must be in the comfortable register, and nearly all the notes must be inside the
    /// plausible range. Testing the fraction rather than the extremes tolerates the odd outlier
    /// note in a real melody while still ruling out a bass line decisively.
    pub fn is_vocal_range(&self, median_key: u8, vocal_key_fraction: f32) -> bool {
        (self.vocal_median_min..=self.vocal_median_max).contains(&median_key)
            && vocal_key_fraction >= self.melody_min_vocal_fraction
    }

    /// Whether a duration is plausible for a song.
    pub fn is_plausible_duration(&self, duration_ms: u32) -> bool {
        (self.min_duration_ms..=self.max_duration_ms).contains(&duration_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_middle_register_line_is_in_vocal_range() {
        let t = Thresholds::default();
        // Median around E4 with every note singable.
        assert!(t.is_vocal_range(64, 1.0));
    }

    #[test]
    fn a_bass_line_is_not_in_vocal_range() {
        let t = Thresholds::default();
        // Median at F2, and most of the line below the plausible floor.
        assert!(!t.is_vocal_range(38, 0.4));
    }

    #[test]
    fn one_outlier_note_does_not_disqualify_a_real_melody() {
        let t = Thresholds::default();
        assert!(t.is_vocal_range(64, 0.95));
    }

    #[test]
    fn a_sane_median_is_not_enough_if_most_notes_are_unsingable() {
        let t = Thresholds::default();
        assert!(!t.is_vocal_range(64, 0.5));
    }

    #[test]
    fn duration_plausibility_has_both_ends() {
        let t = Thresholds::default();
        assert!(!t.is_plausible_duration(5_000));
        assert!(t.is_plausible_duration(210_000));
        assert!(!t.is_plausible_duration(3_600_000));
    }
}
