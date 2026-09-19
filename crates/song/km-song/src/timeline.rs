//! The lyric timeline: what to show, and exactly when.
//!
//! Everything here is measured in **ticks**. The display converts to a highlight position by
//! interpolating against the tick the sequencer is currently at, which stays correct under tempo
//! changes — embedded ones and the user's own tempo adjustment alike.

use serde::{Deserialize, Serialize};

/// A break introduced before a syllable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineBreak {
    /// Continue the current line.
    None,
    /// Start a new line on the same page.
    Line,
    /// Clear the screen and start a new page.
    Page,
}

/// A syllable as parsed, before lines are formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSyllable {
    /// Tick at which this syllable begins.
    pub tick: u32,
    /// Text, with any break markers already stripped.
    pub text: String,
    /// Break that the source file requested before this syllable.
    pub break_before: LineBreak,
    /// Tick at which the source says the syllable stops being sung, where it says so.
    ///
    /// **A MIDI lyric event has no length, and an UltraStar note has one.** Without it a syllable
    /// runs to the next timing point, so the last one of a phrase is wiped slowly across the pause
    /// after it. A syllable ends at the earlier of this and the next timing point.
    pub end_tick: Option<u32>,
}

/// One sung fragment: often a syllable, sometimes a whole word or line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Syllable {
    /// The text to draw.
    pub text: String,
    /// Tick at which this syllable becomes the current one.
    pub start_tick: u32,
    /// Tick at which the highlight finishes crossing it.
    pub end_tick: u32,
}

/// A line of lyrics as it appears on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    /// Tick of the first syllable.
    pub start_tick: u32,
    /// Tick at which the last syllable finishes.
    pub end_tick: u32,
    /// Which screenful this line belongs to; increments on a page break.
    pub page: u16,
    /// The line's syllables, in order.
    pub syllables: Vec<Syllable>,
    /// Whether contact details were taken out of this line's words.
    ///
    /// **Carried rather than inferred, and the reason is `km_suitability`.** Its `is_credit_line`
    /// reads [`Self::text`] to decide whether a line counts towards how much lyric a file has,
    /// which is an input to the stored 0–10 suitability — so a line whose address has been replaced
    /// by [`crate::redact::MASK`] stops reading as a credit, and the file's suitability moves. That
    /// is every suitability in every package that exists. This flag is what lets the measurement
    /// reach the same verdict without a rule being added to it, which `docs/architecture/song.md`
    /// says must not happen.
    #[serde(default)]
    pub contact_redacted: bool,
}

impl LyricLine {
    /// The line's text, with syllables joined as they were written.
    pub fn text(&self) -> String {
        self.syllables.iter().map(|s| s.text.as_str()).collect()
    }
}

/// How finely the source file times its lyrics.
///
/// This feeds the suitability score: syllable-level timing is what makes a karaoke file feel
/// right, and line-level timing is a materially worse experience.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricGranularity {
    /// The file has no lyrics at all.
    None,
    /// Timing points land roughly one per line — the highlight cannot follow the words.
    LineLevel,
    /// Timing points land within lines, so the highlight can track the singing.
    SyllableLevel,
}

/// What a file says about where its words end.
///
/// Two shapes claim a boundary that is not there, from opposite sides, and both reach the screen as
/// syllables drawn apart on a [`SYLLABLE_DIVIDER`]. They are one verdict with two spellings rather
/// than a flag, so a sweep can say how many files each rule claims without hunting for the divider
/// in text that rule has already rewritten.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WordEnds {
    /// The file's spacing is its own, and is drawn exactly as it was written.
    #[default]
    AsWritten,
    /// A space after every syllable, so the file claims each fragment is a whole word.
    EverySyllableSpaced,
    /// No space anywhere, so the file claims a whole verse is one word.
    NoneSpaced,
}

impl WordEnds {
    /// Whether the syllables are drawn apart on a [`SYLLABLE_DIVIDER`] rather than as written.
    pub fn divided(self) -> bool {
        !matches!(self, Self::AsWritten)
    }
}

/// Every lyric line in a song, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricTimeline {
    /// The lines, ordered by start tick.
    pub lines: Vec<LyricLine>,
    /// What the file said about where its words end, where that was not the truth.
    ///
    /// **Carried rather than inferred**, for the reason [`LyricLine::contact_redacted`] is: once the
    /// text is rewritten, the question can only be answered by hunting for the divider in it, which
    /// is a guess about a rewrite that has already run. `km_suitability` reads this to take a point
    /// off a file whose words are not where it says they are.
    pub word_ends: WordEnds,
    /// Whether the file said where any of its lines go.
    ///
    /// Carried rather than inferred, for the reason the flag above is: once the lines are grouped,
    /// the only way back to the question is to guess from their shape. A file that marks nothing has
    /// every line placed by [`LineInference`], and one that marks anything is trusted line by line,
    /// so this says which of the two a reader is holding.
    pub lines_are_marked: bool,
}

impl LyricTimeline {
    /// Whether the song has any lyrics.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Number of lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Total number of syllables across all lines.
    pub fn syllable_count(&self) -> usize {
        self.lines.iter().map(|l| l.syllables.len()).sum()
    }

    /// Number of pages, that is, how many times the screen is cleared.
    pub fn page_count(&self) -> usize {
        self.lines.last().map_or(0, |l| usize::from(l.page) + 1)
    }

    /// How finely the lyrics are timed.
    ///
    /// A file counts as syllable-level once it averages more than 1.5 timing points per line;
    /// below that the timing points are effectively per line, whatever the file intended.
    pub fn granularity(&self) -> LyricGranularity {
        if self.lines.is_empty() {
            return LyricGranularity::None;
        }
        let syllables = self.syllable_count() as f64;
        let lines = self.lines.len() as f64;
        if syllables / lines > 1.5 {
            LyricGranularity::SyllableLevel
        } else {
            LyricGranularity::LineLevel
        }
    }

    /// Index of the line being sung at `tick`, if any.
    ///
    /// Returns the line whose span contains the tick; between lines, returns `None` so the caller
    /// can decide whether to hold the previous line or show the next one.
    pub fn line_at_tick(&self, tick: u32) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| tick >= line.start_tick && tick < line.end_tick)
    }

    /// Index of the first line starting at or after `tick`.
    pub fn next_line_from_tick(&self, tick: u32) -> Option<usize> {
        self.lines.iter().position(|line| line.start_tick >= tick)
    }

    /// Every syllable start tick, in order — the raw timing points of the file.
    pub fn syllable_ticks(&self) -> Vec<u32> {
        self.lines
            .iter()
            .flat_map(|l| l.syllables.iter().map(|s| s.start_tick))
            .collect()
    }

    /// All lyric text, one line per line, for search indexing and debugging.
    pub fn plain_text(&self) -> String {
        self.lines
            .iter()
            .map(LyricLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The first few lines somebody would actually sing, for showing what a song is.
    ///
    /// Not `plain_text().lines().take(n)`, and the difference is the whole point: a great many
    /// karaoke files open with the sequencer's advertisement — a studio name, a telephone number, a
    /// web address, a row of asterisks — and a "preview" of those tells a reader nothing except that
    /// the file came from somebody. [`crate::looks_like_a_banner`] holds the rules and says why they
    /// are separate from `km-suitability`'s.
    ///
    /// **A line that merely repeats the song's title is kept, and that is a measurement rather than
    /// an oversight.** Skipping it was tried first, on the reasoning that files often print the name
    /// before the words start — and a run over 20,000 corpus files said the opposite: nearly every
    /// line it caught was the real opening line of a song whose hook *is* its title. `Good golly Miss
    /// Molly`, `Ain't no sunshine when she's gone`, `Give a little bit`. The rule turned the best
    /// preview a song could have into its second line. Popular music names itself after its first
    /// line far more often than a sequencer prints a header, so the rules below key on shapes words
    /// do not have and never on what the song is called.
    ///
    /// Three bounds, each of which exists because of something in the real corpus:
    ///
    /// * **At most [`PREVIEW_MAX_SKIP`] leading lines are skipped.** Past that the classifier is
    ///   more likely to be wrong than the file is to be all banner, so what is there is taken. An
    ///   unreliable answer beats a blank one.
    /// * **Only leading lines are skipped.** A banner in the middle of a song cannot reach the
    ///   preview anyway, and stopping at the front bounds the cost of a false positive to one line.
    /// * **Each line is cut to [`PREVIEW_MAX_CHARS`].** Break markers are stripped during parsing,
    ///   so a file carrying none at all yields *one* `LyricLine` holding the entire song — and the
    ///   corpus has those. Without a cut, a manifest would gain kilobytes for one such file.
    pub fn preview(&self, limit: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(limit);
        let mut skipped = 0usize;
        for line in &self.lines {
            if out.len() == limit {
                break;
            }
            let text = line.text();
            let trimmed = text.trim();
            // Blank lines never count against the skip budget: they are not a classifier's guess.
            if trimmed.is_empty() {
                continue;
            }
            // `contact_redacted` first, for the same reason `km_suitability` reads it first: the
            // masking has already run by the time anyone asks, so a telephone line that arrives here
            // as `0**17 —` no longer trips the digit rule that used to skip it, and the business
            // card this bound exists to hide would become the preview. Measured on the fixture, not
            // reasoned about — `km-lyrics preview` reported exactly that before the flag was added.
            if out.is_empty()
                && skipped < PREVIEW_MAX_SKIP
                && (line.contact_redacted || crate::looks_like_a_banner(trimmed))
            {
                skipped += 1;
                continue;
            }
            out.push(truncate(trimmed, PREVIEW_MAX_CHARS));
        }
        out
    }
}

/// How many leading lines [`LyricTimeline::preview`] will skip before giving up on the idea.
pub const PREVIEW_MAX_SKIP: usize = 8;

/// How long a preview line may be before it is cut.
///
/// Generous for a line of a song and small against a whole one: the longest lines in the corpus that
/// are genuinely one lyric line sit well under this, and the files that exceed it are the ones with
/// no break markers, where the "line" is the entire song.
pub const PREVIEW_MAX_CHARS: usize = 120;

/// Cuts to `max` characters on a character boundary, marking that something was dropped.
fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let mut out: String = value.chars().take(max).collect();
    out.push('…');
    out
}

/// How many characters of one line read comfortably on a karaoke screen.
///
/// Measured rather than chosen: 152,228 lines from 3,795 corpus files average 26.7 characters, with
/// the 90th percentile at 39 and only 0.7% past 55. This is the shoulder of that distribution, so a
/// line at or under it is an ordinary line and one well past it is not something the world produces.
///
/// It bounds two different things, which is why it is named here rather than written twice. Line
/// *inference* breaks an unmarked stream here ([`LineInference::for_ticks_per_quarter`]). And
/// `km-carols` subdivides at it when converting a hymnal, whose `w:` lines are whole musical
/// phrases and so run more than twice this long.
///
/// It is **not** a bound on what the machine can display. A line the file placed itself stands at
/// whatever width the file gave it, up to [`RUNAWAY_LINE_CHARS`] (see [`build_timeline`]), so the
/// screen meets longer lines than this and copes by shrinking the face rather than by re-breaking
/// the words.
pub const COMFORTABLE_LINE_CHARS: usize = 40;

/// How wide a line may be before the file that placed it has stopped saying where its lines go.
///
/// A file is trusted line by line, so this is the width at which trusting it stops describing
/// anything a sequencer did.
///
/// Measured rather than chosen: every line from the corpus files that place their own
/// have a 99th percentile of 50 characters and a 99.9th of 75, so a line past a hundred is already
/// past the tail. **865 of those files hold one**, which is 0.65% and is what re-breaking here
/// reaches.
pub const RUNAWAY_LINE_CHARS: usize = 100;

/// What separates two syllables of one word in a file that marks no word ends.
///
/// Narrower than a space, because such a file claims every syllable is a word and the screen would
/// otherwise read as more words than the line has. Not removed, because the words would then run
/// together, and not narrower still, because a hair space closes the gap where this one keeps it.
///
/// Six-per-em is a sixth of an em by definition, so it is the one narrow space whose width does not
/// move between faces. Measured at a 72px face it draws a 12px gap where a space draws 20px.
pub const SYLLABLE_DIVIDER: char = '\u{2006}';

/// How many syllables a file needs before [`marks_no_word_ends`] will judge it.
///
/// Roughly a verse, and above the twenty syllables `km_suitability` calls negligible, so a lyric
/// track holding somebody's telephone number cannot reach the question.
pub const MIN_JUDGED_SYLLABLES: usize = 32;

/// What share of syllables must end in a space, in percent, for the spaces to be saying nothing.
///
/// Files that mark word ends sit at 65 to 76%, because a word is two or three syllables. This is far
/// above anything that carries information, and loose enough that a file whose last syllable lacks
/// its space still counts.
pub const WORD_END_SHARE_PERCENT: usize = 95;

/// Mean fragment length, in hundredths of a character, below which the fragments are syllables.
///
/// **This is what separates a file that lost its word ends from one that never had syllables.** A
/// file with one event per whole word, each carrying a space, meets every other condition here
/// while its words are exactly where it says they are — narrowing those would run correct words
/// together.
///
/// Below this mean the corpus holds syllable files only. Above it, up to
/// [`MAX_JUDGED_MEAN_WITH_FEW_LONG_HUNDREDTHS`], the two kinds overlap: English is mostly words of
/// one syllable, so a file that spaces its syllables averages what a file of short whole words does.
pub const MAX_JUDGED_MEAN_HUNDREDTHS: usize = 300;

/// Mean fragment length, in hundredths of a character, below which a file with few long fragments
/// is still judged.
///
/// Between [`MAX_JUDGED_MEAN_HUNDREDTHS`] and this, the mean alone cannot tell the two kinds apart,
/// and the share of long fragments decides (see [`MAX_LONG_FRAGMENT_SHARE_PERCENT`]). Above it, the
/// files the corpus holds are whole words almost without exception.
pub const MAX_JUDGED_MEAN_WITH_FEW_LONG_HUNDREDTHS: usize = 365;

/// How many characters make a fragment long.
///
/// A syllable seldom reaches this in a language written with spaces; a whole word often does.
pub const LONG_FRAGMENT_CHARS: usize = 7;

/// What share of fragments may be long, in percent, for a file between the two means to be judged.
///
/// Measured over the corpus files whose mean falls between the two: those that space their
/// syllables hold 0 to 3.4% long fragments, and those of whole words mostly 4.3% and above. A file
/// of whole words that falls under this draws its words a divider apart, which still reads as words.
pub const MAX_LONG_FRAGMENT_SHARE_PERCENT: usize = 4;

/// Whether a file's spaces say nothing about where its words end.
///
/// Such a file puts a space after **every** syllable, so it claims each one is a whole word. Nothing
/// recovers the real boundaries: the fragments themselves straddle words, and the gaps between
/// timing points are the same size within a word as between two.
///
/// A single leading space disqualifies the file. In the other convention a leading space is *how* a
/// word start is marked, so one of them is the file telling us something, and this must not overrule
/// it.
///
/// **A break marker does not disqualify it.** A marker says where a line ends, and a file can mark
/// every line and still space every syllable. Only fragments with text are counted, so a marker
/// that stands alone does not dilute the share.
fn marks_no_word_ends(raws: &[RawSyllable]) -> bool {
    let mut judged = 0usize;
    let mut word_ends = 0usize;
    let mut body_chars = 0usize;
    let mut long = 0usize;
    for raw in raws.iter().filter(|r| !r.text.is_empty()) {
        if raw.text.starts_with(' ') {
            return false;
        }
        judged += 1;
        if raw.text.ends_with(' ') {
            word_ends += 1;
        }
        let chars = raw.text.trim_end().chars().count();
        body_chars += chars;
        if chars >= LONG_FRAGMENT_CHARS {
            long += 1;
        }
    }
    if judged < MIN_JUDGED_SYLLABLES || word_ends * 100 < judged * WORD_END_SHARE_PERCENT {
        return false;
    }
    body_chars * 100 < judged * MAX_JUDGED_MEAN_HUNDREDTHS
        || (body_chars * 100 < judged * MAX_JUDGED_MEAN_WITH_FEW_LONG_HUNDREDTHS
            && long * 100 < judged * MAX_LONG_FRAGMENT_SHARE_PERCENT)
}

/// What share of syllables may carry a space, in percent, for the file still to be marking nothing.
///
/// Not zero, and the reason is that a space can arrive from somewhere other than a convention: an
/// elided-space underscore resolves to one, and a file of one fragment per note holds a handful of
/// them among hundreds. Files that mark word boundaries at all mark most of them, so anything this
/// low is noise rather than a file speaking.
pub const MAX_SPACED_SHARE_PERCENT: usize = 5;

/// Whether a file says nothing about where its words begin and end.
///
/// Such a file carries no space to speak of, so every fragment joins the next and a verse reaches the
/// television as one run of letters. It makes the claim [`marks_no_word_ends`] answers, turned
/// around: one file says each of its fragments is a whole word, the other that the verse is one
/// word, and neither boundary is recoverable from what is written.
///
/// **Any whitespace counts, leading, trailing or inside a fragment**, and a file that carries more
/// than [`MAX_SPACED_SHARE_PERCENT`] of it is saying where its words are and is trusted completely.
///
/// **No mean fragment length is measured, where [`marks_no_word_ends`] measures one.**
/// [`MAX_JUDGED_MEAN_HUNDREDTHS`] is there to spare a file whose one event per whole word carries a
/// real space. Here there is no space to spare, and joining is wrong whether the fragments are words
/// or syllables, so their length settles nothing.
fn marks_no_word_boundaries(raws: &[RawSyllable]) -> bool {
    if raws.len() < MIN_JUDGED_SYLLABLES {
        return false;
    }
    let mut spaced = 0usize;
    for raw in raws {
        if raw.text.chars().any(writes_without_spaces) {
            return false;
        }
        if raw.text.contains(char::is_whitespace) {
            spaced += 1;
        }
    }
    spaced * 100 <= raws.len() * MAX_SPACED_SHARE_PERCENT
}

/// Whether `c` belongs to a script that writes its words without spaces between them.
///
/// A divider between the fragments of such a script is the same error the other way round: its words
/// run together because that is how the script is written, so joining them is right and one
/// character of it says which kind of file this is. Han, kana and Hangul the television draws; Thai,
/// Lao, Khmer and Myanmar it does not, and they are here because the question is what the writing
/// system does rather than what a font covers.
fn writes_without_spaces(c: char) -> bool {
    matches!(c,
        '\u{0E00}'..='\u{0EFF}'   // Thai, Lao
        | '\u{1000}'..='\u{109F}' // Myanmar
        | '\u{1100}'..='\u{11FF}' // Hangul Jamo
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{3040}'..='\u{30FF}' // Hiragana, Katakana
        | '\u{3130}'..='\u{318F}' // Hangul Compatibility Jamo
        | '\u{3400}'..='\u{9FFF}' // Han, with Extension A
        | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{FF61}'..='\u{FF9F}' // halfwidth kana
    )
}

/// Thresholds for deciding where a lyric stream's lines are when the file does not say.
///
/// Plenty of files carry `Lyric` events with no line markers at all, and plenty more mark some of
/// their lines and then go quiet for a verse. Something has to decide where the lines are, and the
/// alternative to these heuristics is one endless line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineInference {
    /// A silence longer than this between syllables starts a new line.
    pub max_gap_ms: u32,
    /// A line is broken once it reaches roughly this many characters.
    pub max_line_chars: usize,
    /// A line the file placed itself is re-broken once it passes this many characters.
    pub runaway_line_chars: usize,
    /// Ticks a final syllable is held for when nothing follows it to bound the highlight.
    pub default_hold_ticks: u32,
}

impl LineInference {
    /// Thresholds derived from a file's ticks-per-quarter-note.
    ///
    /// The defaults — a 1.2 s gap, [`COMFORTABLE_LINE_CHARS`] characters, and a marked line trusted
    /// up to [`RUNAWAY_LINE_CHARS`] — come from what reads comfortably on a karaoke screen, not from
    /// anything in the MIDI spec.
    pub fn for_ticks_per_quarter(ticks_per_quarter: u16) -> Self {
        Self {
            max_gap_ms: 1_200,
            max_line_chars: COMFORTABLE_LINE_CHARS,
            runaway_line_chars: RUNAWAY_LINE_CHARS,
            default_hold_ticks: u32::from(ticks_per_quarter.max(1)),
        }
    }
}

/// Whether a syllable closes the word it belongs to.
///
/// Two conventions say so and a file uses one of them: a trailing space, which is also what
/// [`SYLLABLE_DIVIDER`] replaces, or punctuation that can only end a word.
fn ends_word(text: &str) -> bool {
    text.chars().next_back().is_some_and(|c| {
        c.is_whitespace() || c == SYLLABLE_DIVIDER || matches!(c, '.' | ',' | '!' | '?' | ';' | ':')
    })
}

/// Where to cut a run the file left unmarked for too long.
///
/// `widths` is each syllable's character count, `gaps_ms[i]` the silence before syllable `i`, and
/// `breakable[i]` whether syllable `i` starts a word. The returned indices are where a new line
/// begins, in order.
///
/// **The widest gap in a segment is the cut, not the point the budget runs out.** A singer reads
/// phrases, and the silence between two of them is the only evidence in a file that lost its markers
/// about where one ended. Cutting at the budget instead lands mid-phrase every time, and a file's
/// phrase gaps are frequently just under [`LineInference::max_gap_ms`], so the gap threshold that
/// serves an unmarked stream finds nothing here.
///
/// Ties go to the cut nearest the middle, which keeps two halves of a run from coming out as one
/// full line and one word.
///
/// **`min_gap_ms` is what stops the cutting, and `budget` only asks whether to look.** A gap no
/// wider than the ordinary step between two syllables is not a pause, and cutting at one lands
/// inside a word in every file whose syllables carry no word ends. So a segment still over budget
/// with no pause left inside it stays exactly as wide as it came in, and shrinking the face is what
/// answers that — the same answer a file that wrote one long line on purpose already gets.
///
/// A segment nothing may be cut inside likewise stays whole, which is the honest answer for a run
/// of one unbroken word.
///
/// Pure, and takes counts rather than syllables, so it tests without a parser or a tick map.
fn split_points(
    widths: &[usize],
    gaps_ms: &[u32],
    breakable: &[bool],
    budget: usize,
    min_gap_ms: u32,
) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut pending = vec![(0usize, widths.len())];
    while let Some((lo, hi)) = pending.pop() {
        let width: usize = widths[lo..hi].iter().sum();
        if width <= budget {
            continue;
        }
        // Distance from the middle is doubled so a segment of odd width needs no halving.
        let mut before = 0usize;
        let mut best: Option<(usize, u32, usize)> = None;
        for at in lo..hi {
            if at > lo && breakable[at] && gaps_ms[at] >= min_gap_ms {
                let off_middle = (2 * before).abs_diff(width);
                let better = best.is_none_or(|(_, gap, other)| {
                    gaps_ms[at] > gap || (gaps_ms[at] == gap && off_middle < other)
                });
                if better {
                    best = Some((at, gaps_ms[at], off_middle));
                }
            }
            before += widths[at];
        }
        let Some((at, _, _)) = best else { continue };
        cuts.push(at);
        pending.push((lo, at));
        pending.push((at, hi));
    }
    cuts.sort_unstable();
    cuts
}

/// How much wider than the ordinary step between two syllables a gap must be to read as a pause.
///
/// A run's own median gap is what one syllable following another costs in it, whatever its tempo,
/// its timebase or the hand that sequenced it, so the threshold is taken from the run rather than
/// named in milliseconds.
///
/// **Three sits between the first quartile of what a real line break is worth and the median**,
/// measured by `km-lyrics scan` over the 5.9 million lines the corpus's marked files place: a break
/// a file writes is 1.0× its own step at the 10th percentile, 2.0× at the 25th, 4.0× at the 50th
/// and 13.0× at the 90th.
///
/// Above the quartile on purpose. A quarter of real breaks are written at twice the step or less,
/// which nothing separates from one syllable following another, so the two errors are not the same
/// size: missing a break leaves a line wider than it might have been, and the face ladder in
/// `km-display` was built for that, while taking one that is not there cuts a word in half in front
/// of somebody singing it.
const PHRASE_GAP_MULTIPLE: u32 = 3;

/// The gap above which a run's own timing reads as a pause rather than as the next syllable.
///
/// The median rather than the mean, because one four-second rest in a verse would drag a mean up
/// past every real phrase boundary around it.
fn phrase_gap_ms(gaps_ms: &[u32]) -> u32 {
    let mut sorted: Vec<u32> = gaps_ms.iter().skip(1).copied().collect();
    if sorted.is_empty() {
        return 0;
    }
    sorted.sort_unstable();
    sorted[sorted.len() / 2].saturating_mul(PHRASE_GAP_MULTIPLE)
}

/// Cuts one group of syllables the file left unmarked for too long, or hands it back whole.
fn split_runaway(
    page: u16,
    group: Vec<RawSyllable>,
    inference: LineInference,
    tick_to_ms: &impl Fn(u32) -> u32,
) -> Vec<(u16, Vec<RawSyllable>)> {
    let widths: Vec<usize> = group.iter().map(|r| r.text.chars().count()).collect();
    if widths.iter().sum::<usize>() <= inference.runaway_line_chars {
        return vec![(page, group)];
    }

    let gaps_ms: Vec<u32> = group
        .iter()
        .enumerate()
        .map(|(i, raw)| match i.checked_sub(1) {
            Some(prev) => tick_to_ms(raw.tick).saturating_sub(tick_to_ms(group[prev].tick)),
            None => 0,
        })
        .collect();

    let mut breakable: Vec<bool> = group
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            raw.text.starts_with(char::is_whitespace)
                || i.checked_sub(1).is_some_and(|p| ends_word(&group[p].text))
        })
        .collect();
    // A file that marks no word end at all offers nothing to respect, and refusing every cut would
    // leave the run exactly as wide as it came in. Every point becomes a word start, and the gaps
    // decide alone.
    if !breakable[1..].contains(&true) {
        breakable.fill(true);
    }

    let cuts = split_points(
        &widths,
        &gaps_ms,
        &breakable,
        inference.max_line_chars,
        phrase_gap_ms(&gaps_ms),
    );

    let mut out = Vec::with_capacity(cuts.len() + 1);
    let mut rest = group;
    // Taken from the back, so each split leaves the earlier syllables in place.
    for at in cuts.into_iter().rev() {
        let tail = rest.split_off(at);
        out.push((page, tail));
    }
    out.push((page, rest));
    out.reverse();
    out
}

/// Builds a timeline from parsed syllables.
///
/// `tick_to_ms` converts ticks to wall-clock time, needed only for the gap heuristics.
///
/// **Every break marker the source supplied is honored, and no inference competes with one** —
/// second-guessing a file that told us where its lines are would only make things worse. What a
/// marker cannot speak for is the stretch after it: a file that marks a chorus and then leaves a
/// whole verse unmarked has said nothing about that verse, and a run past
/// [`LineInference::runaway_line_chars`] is cut by [`split_points`] rather than drawn as one line
/// nobody can read.
pub fn build_timeline(
    mut raws: Vec<RawSyllable>,
    inference: LineInference,
    tick_to_ms: impl Fn(u32) -> u32,
) -> LyricTimeline {
    raws.retain(|r| !r.text.is_empty() || r.break_before != LineBreak::None);
    if raws.is_empty() {
        return LyricTimeline::default();
    }
    raws.sort_by_key(|r| r.tick);

    let has_markers = raws.iter().any(|r| r.break_before != LineBreak::None);

    // The rewrite runs here, before the grouping and the redaction below, so every reader downstream
    // sees the text the screen will draw.
    //
    // Two shapes reach the same divider from opposite sides: a file that spaces every fragment, and
    // one that spaces none. A file of the first kind is left as it stands wherever a fragment lacks
    // its space, because the file wrote one there and that is what a run of them looks like; a file
    // of the second kind has nothing to leave.
    //
    // A break marker says where a line ends and nothing about words, so a marked file is judged for
    // the first shape too. The second is judged only in an unmarked stream.
    let word_ends = if marks_no_word_ends(&raws) {
        for raw in &mut raws {
            if raw.text.ends_with(' ') {
                raw.text.pop();
                raw.text.push(SYLLABLE_DIVIDER);
            }
        }
        WordEnds::EverySyllableSpaced
    } else if !has_markers && marks_no_word_boundaries(&raws) {
        // The handful of fragments that do carry a space keep it, and take no divider on top: the
        // gap is already there, and a second one beside it is wider than either.
        for raw in &mut raws {
            if !raw.text.ends_with(char::is_whitespace) {
                raw.text.push(SYLLABLE_DIVIDER);
            }
        }
        WordEnds::NoneSpaced
    } else {
        WordEnds::AsWritten
    };

    // Group indices into lines.
    let mut groups: Vec<(u16, Vec<RawSyllable>)> = Vec::new();
    let mut page: u16 = 0;
    let mut current: Vec<RawSyllable> = Vec::new();
    let mut current_chars = 0usize;
    let mut prev_tick: Option<u32> = None;

    for raw in raws {
        let mut start_new_line = false;
        let mut start_new_page = false;

        if has_markers {
            match raw.break_before {
                LineBreak::Page => {
                    start_new_line = true;
                    start_new_page = true;
                }
                LineBreak::Line => start_new_line = true,
                LineBreak::None => {}
            }
        } else if let Some(prev) = prev_tick {
            // Two independent reasons to break, either of which is enough: a silence long enough
            // to read as the end of a phrase, or a line that has simply grown too wide to fit.
            let gap_ms = tick_to_ms(raw.tick).saturating_sub(tick_to_ms(prev));
            let too_long = current_chars + raw.text.chars().count() > inference.max_line_chars;
            start_new_line = gap_ms >= inference.max_gap_ms || too_long;
        }

        if start_new_line && !current.is_empty() {
            groups.push((page, std::mem::take(&mut current)));
            current_chars = 0;
        }
        if start_new_page {
            // Only advance the page if the previous one had content, so a leading page marker
            // does not produce an empty first page.
            if !groups.is_empty() {
                page = page.saturating_add(1);
            }
        }

        if raw.text.is_empty() {
            prev_tick = Some(raw.tick);
            continue;
        }
        current_chars += raw.text.chars().count();
        prev_tick = Some(raw.tick);
        current.push(raw);
    }
    if !current.is_empty() {
        groups.push((page, current));
    }

    // A marked file is trusted line by line, and a run it marked nothing across is not a line it
    // placed. Cutting happens here rather than in the loop above so the markers keep their meaning:
    // every break the file asked for is already in `groups`, and this only adds to them.
    //
    // **Gated on the file having spoken at all, rather than left to the bound.** An unmarked stream
    // is broken at `max_line_chars` above and would almost never reach here, but a single syllable
    // holding a whole verse would — and the answer for that file is the one
    // [`LineInference`] already gave, not a second opinion on top of it.
    if has_markers {
        groups = groups
            .into_iter()
            .flat_map(|(page, group)| split_runaway(page, group, inference, &tick_to_ms))
            .collect();
    }

    // Resolve end ticks. A syllable runs until the next timing point anywhere in the song; the
    // last syllable of the song is held for `default_hold_ticks`.
    let all_ticks: Vec<u32> = groups
        .iter()
        .flat_map(|(_, g)| g.iter().map(|r| r.tick))
        .collect();

    let mut lines = Vec::with_capacity(groups.len());
    let mut flat_index = 0usize;
    for (page, group) in groups {
        let mut syllables = Vec::with_capacity(group.len());
        for raw in group {
            let next_tick = all_ticks
                .get(flat_index + 1)
                .copied()
                .filter(|&t| t > raw.tick);
            let end_tick = match raw.end_tick.filter(|&end| end > raw.tick) {
                Some(end) => next_tick.map_or(end, |next| next.min(end)),
                None => next_tick
                    .unwrap_or_else(|| raw.tick.saturating_add(inference.default_hold_ticks)),
            };
            syllables.push(Syllable {
                text: raw.text,
                start_tick: raw.tick,
                end_tick,
            });
            flat_index += 1;
        }
        // A divider at the end of a line is a gap nothing follows, and the line is centered on its
        // measured width, so leaving it there sets the whole line off center by half a space.
        if word_ends.divided()
            && let Some(last) = syllables.last_mut()
            && last.text.ends_with(SYLLABLE_DIVIDER)
        {
            last.text.pop();
        }
        let start_tick = syllables.first().map_or(0, |s| s.start_tick);
        let end_tick = syllables.last().map_or(start_tick, |s| s.end_tick);
        let mut line = LyricLine {
            start_tick,
            end_tick,
            page,
            syllables,
            contact_redacted: false,
        };
        redact_contact_details(&mut line);
        // **A publisher's notice is not sung, so it does not reach anybody.** Dropped here rather
        // than masked, which is where this parts company with the contact details above: an address
        // can sit inside something singable and a line that is *only* the notice cannot, so losing
        // the whole line costs nothing. See `A line that is only a legal notice is not sung`.
        //
        // Dropped after the grouping rather than before it, because the test is on the whole line
        // and a line is not a line until its syllables are together. The next line keeps its own
        // ticks either way -- nothing here moves a syllable, it only stops carrying one.
        if crate::is_only_a_legal_notice(&line.text()) {
            continue;
        }
        lines.push(line);
    }

    LyricTimeline {
        lines,
        word_ends,
        lines_are_marked: has_markers,
    }
}

/// Replaces any contact details in `line`'s syllables with [`crate::redact::MASK`].
///
/// **Here, at the one place a lyric string is written, rather than at each place one is read.**
/// [`LyricLine::text`] looks like the tidier seam and is not: the display draws the *current* line
/// from `Syllable::text` directly, so that it can wipe one syllable at a time, and `SyllableDto`
/// puts the same field on the wire. Masking in `text()` would leave the address on the television
/// and in the API while hiding it everywhere nobody was looking. Writing it once here reaches the
/// screen, the wipe, the `lyric_line` event, the lyrics endpoint, the package's preview, the
/// catalog column, the song book's `INÍCIO DA LETRA` cell and the curation index, because every
/// one of those is downstream of this field.
///
/// The spans are found on the joined line and mapped back onto the syllables that produced it, so a
/// syllable split across an address — `some` + `one@example.com` — is handled: the first syllable
/// the span touches carries the mask and the rest lose only the part inside it. **Timing is never
/// touched.** A syllable whose text goes away keeps its ticks, so the wipe still crosses the line at
/// the speed the file asked for and the highlight does not jump.
fn redact_contact_details(line: &mut LyricLine) {
    let text = line.text();
    let spans = crate::redact::contact_spans(&text);
    if spans.is_empty() {
        return;
    }

    let mut offset = 0usize;
    for syllable in &mut line.syllables {
        let start = offset;
        let end = start + syllable.text.len();
        offset = end;

        let mut rebuilt = String::new();
        let mut cursor = start;
        for span in &spans {
            if span.end <= start || span.start >= end {
                continue;
            }
            rebuilt.push_str(&text[cursor..span.start.max(start)]);
            // The mask goes on the syllable the span *begins* in and nowhere else, so one address
            // leaves one dash however many syllables the file happened to split it into.
            if span.start >= start {
                rebuilt.push_str(crate::redact::MASK);
            }
            cursor = span.end.min(end);
        }
        rebuilt.push_str(&text[cursor..end]);
        syllable.text = rebuilt;
    }

    line.contact_redacted = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(tick: u32, text: &str, break_before: LineBreak) -> RawSyllable {
        RawSyllable {
            tick,
            text: text.to_owned(),
            break_before,
            end_tick: None,
        }
    }

    /// 480 ticks per quarter at 120 BPM: one tick is 1/960 s, so ticks map to ms as tick * 1000/960.
    fn ticks_to_ms(tick: u32) -> u32 {
        (u64::from(tick) * 1_000 / 960) as u32
    }

    fn inference() -> LineInference {
        LineInference::for_ticks_per_quarter(480)
    }

    #[test]
    fn empty_input_yields_an_empty_timeline() {
        let timeline = build_timeline(vec![], inference(), ticks_to_ms);
        assert!(timeline.is_empty());
        assert_eq!(timeline.granularity(), LyricGranularity::None);
    }

    #[test]
    fn explicit_line_markers_are_trusted() {
        let timeline = build_timeline(
            vec![
                raw(0, "Twin", LineBreak::None),
                raw(240, "kle ", LineBreak::None),
                raw(480, "twin", LineBreak::None),
                raw(720, "kle", LineBreak::None),
                raw(960, "lit", LineBreak::Line),
                raw(1_200, "tle ", LineBreak::None),
                raw(1_440, "star", LineBreak::None),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.line_count(), 2);
        assert_eq!(timeline.lines[0].text(), "Twinkle twinkle");
        assert_eq!(timeline.lines[1].text(), "little star");
        assert_eq!(timeline.granularity(), LyricGranularity::SyllableLevel);
    }

    #[test]
    fn page_markers_advance_the_page_number() {
        let timeline = build_timeline(
            vec![
                raw(0, "one", LineBreak::None),
                raw(480, "two", LineBreak::Line),
                raw(960, "three", LineBreak::Page),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.line_count(), 3);
        assert_eq!(timeline.lines[0].page, 0);
        assert_eq!(timeline.lines[1].page, 0);
        assert_eq!(timeline.lines[2].page, 1);
        assert_eq!(timeline.page_count(), 2);
    }

    #[test]
    fn a_leading_page_marker_does_not_create_an_empty_page() {
        let timeline = build_timeline(
            vec![
                raw(0, "first", LineBreak::Page),
                raw(480, "second", LineBreak::None),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.lines[0].page, 0);
        assert_eq!(timeline.page_count(), 1);
    }

    #[test]
    fn unmarked_lyrics_are_split_on_long_gaps() {
        // 1_920 ticks is 2 s at this timebase, well over the 1.2 s threshold.
        let timeline = build_timeline(
            vec![
                raw(0, "first", LineBreak::None),
                raw(240, " line", LineBreak::None),
                raw(2_400, "second", LineBreak::None),
                raw(2_640, " line", LineBreak::None),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.line_count(), 2);
        assert_eq!(timeline.lines[0].text(), "first line");
        assert_eq!(timeline.lines[1].text(), "second line");
    }

    #[test]
    fn unmarked_lyrics_are_split_when_a_line_gets_too_long() {
        let mut raws = Vec::new();
        for i in 0..20u32 {
            raws.push(raw(i * 120, "abcde", LineBreak::None));
        }
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(
            timeline.line_count() > 1,
            "100 characters should not stay on one line"
        );
        for line in &timeline.lines {
            assert!(
                line.text().chars().count() <= inference().max_line_chars + 5,
                "line too long: {:?}",
                line.text()
            );
        }
    }

    #[test]
    fn a_marked_file_keeps_a_line_a_gap_would_have_broken() {
        // A four-second gap that would break an unmarked stream, but the file uses markers, so the
        // two syllables stay on one line.
        let timeline = build_timeline(
            vec![
                raw(0, "held", LineBreak::None),
                raw(4_800, " note", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.line_count(), 2);
        assert_eq!(timeline.lines[0].text(), "held");
    }

    /// A run of `count` five-character words, one every 480 ticks, pausing before each of `pauses`.
    ///
    /// The first carries a marker, so the run is one a file placed rather than one inferred: without
    /// it the ordinary width rule would break the run and the answer would say nothing about what a
    /// marked file does.
    fn run_of_words(count: usize, pauses: &[usize]) -> Vec<RawSyllable> {
        let mut tick = 0u32;
        (0..count)
            .map(|i| {
                if pauses.contains(&i) {
                    tick += 4_800;
                } else if i > 0 {
                    tick += 480;
                }
                let marker = if i == 0 {
                    LineBreak::Line
                } else {
                    LineBreak::None
                };
                raw(tick, "word ", marker)
            })
            .collect()
    }

    #[test]
    fn a_run_is_cut_at_its_widest_gap_rather_than_where_the_budget_runs_out() {
        let widths = [5; 12];
        let mut gaps = [100u32; 12];
        gaps[4] = 3_000;
        assert_eq!(split_points(&widths, &gaps, &[true; 12], 40, 100), vec![4]);
    }

    #[test]
    fn an_equal_gap_is_broken_toward_the_middle() {
        let widths = [5; 12];
        let cuts = split_points(&widths, &[100; 12], &[true; 12], 40, 100);
        assert_eq!(cuts, vec![6]);
    }

    #[test]
    fn a_run_is_cut_until_every_piece_fits() {
        let widths = [5; 20];
        let cuts = split_points(&widths, &[100; 20], &[true; 20], 40, 100);
        assert_eq!(cuts, vec![5, 10, 15]);
    }

    #[test]
    fn a_run_with_nothing_breakable_inside_it_stays_whole() {
        let widths = [5; 12];
        let mut breakable = [false; 12];
        breakable[0] = true;
        assert!(split_points(&widths, &[100; 12], &breakable, 40, 100).is_empty());
    }

    #[test]
    fn a_run_the_file_never_paused_in_stays_whole() {
        // Wide enough to want three cuts, with nothing in it a singer would hear as a phrase end.
        let widths = [5; 20];
        assert!(split_points(&widths, &[100; 20], &[true; 20], 40, 500).is_empty());
    }

    #[test]
    fn cutting_stops_where_the_pauses_run_out() {
        // One real pause a third of the way in, and ordinary steps either side of it: the run is cut
        // once and the two pieces are left at whatever width that leaves them.
        let widths = [5; 20];
        let mut gaps = [100u32; 20];
        gaps[6] = 900;
        assert_eq!(split_points(&widths, &gaps, &[true; 20], 40, 500), vec![6]);
    }

    #[test]
    fn the_pause_threshold_is_read_off_the_run_itself() {
        assert_eq!(phrase_gap_ms(&[0, 400, 400, 400, 3_000]), 1_200);
        // A single rest cannot drag the threshold past the phrase boundaries around it.
        assert_eq!(phrase_gap_ms(&[0, 400, 400, 20_000]), 1_200);
        assert_eq!(phrase_gap_ms(&[0]), 0);
    }

    #[test]
    fn a_line_the_file_placed_is_kept_at_the_width_it_was_given() {
        // Twenty five-character words is 100 characters, which is the bound rather than past it.
        let count = RUNAWAY_LINE_CHARS / 5;
        let timeline = build_timeline(
            run_of_words(count, &[])
                .into_iter()
                .chain([raw(100_000, "after", LineBreak::Line)])
                .collect(),
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.line_count(), 2);
        assert_eq!(timeline.lines[0].text().chars().count(), RUNAWAY_LINE_CHARS);
    }

    #[test]
    fn a_run_the_file_left_unmarked_is_cut_at_the_phrases_it_paused_on() {
        // Twenty-four words is 120 characters, past the bound, with the pauses at thirds rather than
        // at the middle, so a cut placed where the budget ran out could not land on them.
        let timeline = build_timeline(run_of_words(24, &[8, 16]), inference(), ticks_to_ms);
        assert_eq!(timeline.line_count(), 3);
        for line in &timeline.lines {
            assert_eq!(line.text(), "word ".repeat(8));
        }
    }

    #[test]
    fn a_cut_does_not_land_inside_a_word() {
        // `ba` does not end a word and `by ` does. The wider of the two pauses sits inside a word, so
        // taking the widest gap alone would cut `ba` from `by`.
        let mut tick = 0u32;
        let mut raws = Vec::new();
        for i in 0..50 {
            match i {
                0 => {}
                7 => tick += 9_600,
                10 => tick += 4_800,
                _ => tick += 480,
            }
            let text = if i % 2 == 0 { "ba" } else { "by " };
            let marker = if i == 0 {
                LineBreak::Line
            } else {
                LineBreak::None
            };
            raws.push(raw(tick, text, marker));
        }
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert_eq!(timeline.line_count(), 2);
        for line in &timeline.lines {
            let text = line.text();
            assert!(
                text.split_inclusive(' ').all(|word| word == "baby "),
                "a word was cut: {text:?}"
            );
        }
    }

    #[test]
    fn syllables_end_where_the_next_one_begins() {
        let timeline = build_timeline(
            vec![
                raw(0, "a", LineBreak::None),
                raw(240, "b", LineBreak::None),
                raw(480, "c", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.lines[0].syllables[0].end_tick, 240);
        assert_eq!(timeline.lines[0].syllables[1].end_tick, 480);
        // The final syllable of the song has nothing after it, so it is held.
        assert_eq!(timeline.lines[1].syllables[0].end_tick, 480 + 480);
    }

    #[test]
    fn a_syllable_end_crosses_a_line_boundary_rather_than_snapping_to_it() {
        let timeline = build_timeline(
            vec![
                raw(0, "end", LineBreak::None),
                raw(960, "next", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        // The last syllable of line 0 runs until the first syllable of line 1 starts.
        assert_eq!(timeline.lines[0].end_tick, 960);
        assert_eq!(timeline.lines[1].start_tick, 960);
    }

    #[test]
    fn line_lookup_by_tick() {
        let timeline = build_timeline(
            vec![
                raw(0, "one", LineBreak::None),
                raw(480, "two", LineBreak::Line),
                raw(960, "three", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.line_at_tick(0), Some(0));
        assert_eq!(timeline.line_at_tick(479), Some(0));
        assert_eq!(timeline.line_at_tick(480), Some(1));
        assert_eq!(timeline.next_line_from_tick(500), Some(2));
        assert_eq!(timeline.line_at_tick(u32::MAX), None);
    }

    #[test]
    fn line_level_timing_is_reported_as_such() {
        let timeline = build_timeline(
            vec![
                raw(0, "a whole line at once", LineBreak::Line),
                raw(960, "another whole line", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.granularity(), LyricGranularity::LineLevel);
    }

    #[test]
    fn out_of_order_input_is_sorted() {
        let timeline = build_timeline(
            vec![
                raw(480, "second", LineBreak::None),
                raw(0, "first ", LineBreak::None),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.lines[0].text(), "first second");
    }

    #[test]
    fn plain_text_joins_lines_with_newlines() {
        let timeline = build_timeline(
            vec![
                raw(0, "one", LineBreak::None),
                raw(480, "two", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.plain_text(), "one\ntwo");
        assert_eq!(timeline.syllable_ticks(), vec![0, 480]);
    }

    /// Builds a timeline whose lines are exactly the strings given, one line each.
    fn lines_of(texts: &[&str]) -> LyricTimeline {
        let mut raws = Vec::new();
        for (index, text) in texts.iter().enumerate() {
            raws.push(raw(
                index as u32 * 480,
                text,
                if index == 0 {
                    LineBreak::None
                } else {
                    LineBreak::Line
                },
            ));
        }
        build_timeline(raws, inference(), ticks_to_ms)
    }

    /// **The case that decides where the redaction goes.**
    ///
    /// The address is split across syllables the way a karaoke file splits one, and the display
    /// draws the current line from `Syllable::text` rather than from `text()`. Masking in `text()`
    /// would pass this assertion on the joined line and still put the address on the television.
    #[test]
    fn an_address_split_across_syllables_is_gone_from_the_syllables_too() {
        let timeline = build_timeline(
            vec![
                raw(0, "Sung ", LineBreak::None),
                raw(480, "by ", LineBreak::None),
                raw(960, "some", LineBreak::None),
                raw(1440, "one@example.com", LineBreak::None),
                raw(1920, " tonight", LineBreak::None),
            ],
            inference(),
            ticks_to_ms,
        );
        let line = &timeline.lines[0];
        assert_eq!(line.text(), "Sung by — tonight");
        assert_eq!(
            line.syllables
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Sung ", "by ", "—", "", " tonight"],
            "the mask lands on the syllable the address starts in, and the rest lose their part"
        );
        assert!(line.contact_redacted);
    }

    /// Timing is what the wipe runs on, so an emptied syllable must keep its ticks.
    #[test]
    fn redaction_moves_no_ticks() {
        let timeline = build_timeline(
            vec![
                raw(0, "a@example.com", LineBreak::None),
                raw(480, " b", LineBreak::None),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.syllable_ticks(), vec![0, 480]);
        assert_eq!(timeline.lines[0].start_tick, 0);
    }

    /// **A masked line must still be skipped, and the digit rules cannot see that it was one.**
    ///
    /// `0**17 3463-1150` is a telephone banner by the ratio rule; masked to `0**17 —` it is three
    /// digits and looks like words. Without the flag this file's preview becomes the business card
    /// the skip exists to hide — which is what `km-lyrics preview` actually reported.
    #[test]
    fn a_masked_business_card_is_still_skipped_by_the_preview() {
        let timeline = build_timeline(
            vec![
                raw(0, "A. Sequencer", LineBreak::None),
                raw(480, "0**17 3463-1150", LineBreak::Line),
                raw(960, "someone@example.com", LineBreak::Line),
                raw(1440, "Tempo perdido", LineBreak::Line),
            ],
            inference(),
            ticks_to_ms,
        );
        assert_eq!(timeline.preview(2), vec!["Tempo perdido"]);
    }

    #[test]
    fn a_line_with_no_contact_details_is_not_marked() {
        let timeline = lines_of(&["Tempo perdido"]);
        assert!(!timeline.lines[0].contact_redacted);
        assert_eq!(timeline.lines[0].text(), "Tempo perdido");
    }

    #[test]
    fn a_preview_is_the_first_lines_of_the_song() {
        let timeline = lines_of(&["Tempo perdido", "E que tudo mais", "va pro inferno"]);
        assert_eq!(
            timeline.preview(2),
            vec!["Tempo perdido", "E que tudo mais"]
        );
    }

    #[test]
    fn a_preview_skips_the_sequencers_advertisement() {
        let timeline = lines_of(&[
            "Karaoke do Brasil",
            "http://www.example.test/kar",
            "****************",
            "Tempo perdido",
            "E que tudo mais",
        ]);
        assert_eq!(
            timeline.preview(2),
            vec!["Tempo perdido", "E que tudo mais"]
        );
    }

    /// The rule that was tried and measured away. Skipping a line because it matched the title cost
    /// far more than it saved: 20,000 corpus files said nearly every line it caught was the real
    /// opening of a song whose hook is its own name. Pinned as a test so it does not come back on
    /// the same plausible reasoning.
    #[test]
    fn a_first_line_that_is_also_the_title_is_kept() {
        let timeline = lines_of(&[
            "Good golly Miss Molly",
            "You sure like to ball",
            "Good golly Miss Molly",
        ]);
        assert_eq!(
            timeline.preview(2),
            vec!["Good golly Miss Molly", "You sure like to ball"]
        );
    }

    #[test]
    fn a_line_that_is_only_a_legal_notice_never_reaches_the_words() {
        // Carried as ordinary lyric events with no `@` on them, which is how the files that sing it
        // carry it. The whole line goes; the song around it does not move.
        let timeline = lines_of(&[
            "Tempo perdido",
            "ALL rights reserved. Not for broadcast or",
            "transmission of any kind.",
            "E que tudo mais",
        ]);
        assert_eq!(
            timeline
                .lines
                .iter()
                .map(LyricLine::text)
                .collect::<Vec<_>>(),
            vec!["Tempo perdido", "E que tudo mais"]
        );
        assert_eq!(timeline.line_count(), 2);
        assert!(!timeline.plain_text().contains("RENTAL"));
        assert!(!timeline.plain_text().contains("rights reserved"));
    }

    #[test]
    fn the_verse_after_a_notice_keeps_its_own_ticks() {
        // Nothing here moves a syllable, it only stops carrying one — so the line that followed the
        // notice starts where the file said it starts.
        let with_notice = lines_of(&[
            "Tempo perdido",
            "DO NOT DUPLICATE. NOT FOR RENTAL.",
            "E que tudo mais",
        ]);
        let last = with_notice.lines.last().expect("a line");
        assert_eq!(last.text(), "E que tudo mais");
        assert_eq!(last.start_tick, 2 * 480, "the third event's own tick");
    }

    #[test]
    fn a_notice_naming_a_publisher_stays_in_the_words() {
        // The narrow rule, seen from the timeline: this line is a banner and is kept out of a
        // preview, but it names somebody and so is not thrown away.
        let timeline = lines_of(&["All rights reserved, TUNE 1000 CORP.", "Tempo perdido"]);
        assert_eq!(timeline.line_count(), 2);
        assert_eq!(timeline.preview(2), vec!["Tempo perdido"]);
    }

    #[test]
    fn a_song_that_is_only_a_notice_has_no_words_at_all() {
        // The honest answer for a file whose whole lyric track is the publisher's notice: it has no
        // words, rather than words nobody would sing.
        let timeline = lines_of(&[
            "ALL rights reserved. Not for broadcast or",
            "transmission of any kind.",
            "DO NOT DUPLICATE. NOT FOR RENTAL.",
        ]);
        assert!(timeline.is_empty());
        assert_eq!(timeline.preview(2), Vec::<String>::new());
    }

    #[test]
    fn a_banner_after_the_words_have_started_is_left_alone() {
        // Only leading lines are skipped, so a false positive costs one line rather than the song.
        let timeline = lines_of(&["Tempo perdido", "Karaoke do Brasil", "E que tudo mais"]);
        assert_eq!(
            timeline.preview(2),
            vec!["Tempo perdido", "Karaoke do Brasil"]
        );
    }

    #[test]
    fn a_song_that_is_nothing_but_banner_gives_up_rather_than_skipping_for_ever() {
        // Past the budget the classifier is likelier to be wrong than the file is to be all credit,
        // so what is there is taken. An unreliable preview beats a blank one.
        let mut texts = vec!["Karaoke by Someone"; PREVIEW_MAX_SKIP];
        texts.push("Karaoke by Someone Else");
        texts.push("and another");
        let timeline = lines_of(&texts);
        assert_eq!(
            timeline.preview(2),
            vec!["Karaoke by Someone Else", "and another"]
        );
    }

    #[test]
    fn a_file_with_no_line_markers_is_cut_rather_than_stored_whole() {
        // One `LyricLine` holding the entire song is a real corpus shape, not a hypothetical.
        let whole = "la ".repeat(200);
        let timeline = lines_of(&[whole.as_str()]);
        let preview = timeline.preview(2);
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].chars().count(), PREVIEW_MAX_CHARS + 1);
        assert!(preview[0].ends_with('…'));
    }

    #[test]
    fn a_song_with_no_words_previews_as_nothing() {
        assert!(LyricTimeline::default().preview(2).is_empty());
    }

    #[test]
    fn a_preview_never_returns_more_lines_than_asked_for() {
        let timeline = lines_of(&["one", "two", "three", "four"]);
        assert_eq!(timeline.preview(1), vec!["one"]);
        assert!(timeline.preview(0).is_empty());
    }

    /// `pattern` repeated until there are `count` syllables, a beat apart, with no break markers.
    fn unmarked(pattern: &[&str], count: usize) -> Vec<RawSyllable> {
        (0..count)
            .map(|i| raw(i as u32 * 480, pattern[i % pattern.len()], LineBreak::None))
            .collect()
    }

    fn narrowed(pattern: &[&str], count: usize) -> LyricTimeline {
        build_timeline(unmarked(pattern, count), inference(), ticks_to_ms)
    }

    #[test]
    fn a_file_that_marks_every_syllable_as_a_word_is_drawn_with_narrow_dividers() {
        let timeline = narrowed(&["can ", "ta ", "re ", "mos "], 40);
        assert!(timeline.word_ends.divided());
        let text = timeline.lines[0].text();
        assert!(
            text.contains(SYLLABLE_DIVIDER),
            "the divider replaces the space: {text:?}"
        );
        assert!(
            !text.contains(' '),
            "no full space is left to read as a word end: {text:?}"
        );
        assert_eq!(timeline.granularity(), LyricGranularity::SyllableLevel);
    }

    #[test]
    fn a_narrowed_line_does_not_end_with_a_divider() {
        let timeline = narrowed(&["can ", "ta ", "re ", "mos "], 40);
        assert!(
            timeline.line_count() > 1,
            "the width heuristic should split"
        );
        for line in &timeline.lines {
            let text = line.text();
            assert!(
                !text.ends_with(SYLLABLE_DIVIDER),
                "a gap nothing follows sets the centered line off center: {text:?}"
            );
        }
    }

    /// **The case that stops correct words being run together.**
    ///
    /// One event per whole word, each with its space and no break marker, meets every other
    /// condition — and its words are exactly where it says they are.
    #[test]
    fn a_file_of_whole_words_keeps_its_spaces() {
        let timeline = narrowed(&["guitarra ", "cantando ", "sozinho ", "amanhece "], 40);
        assert!(!timeline.word_ends.divided());
        assert!(timeline.lines[0].text().contains(' '));
    }

    /// The word-per-event file that comes closest to the syllable ones, at 3.4 characters a
    /// fragment against their 2.75 — so this pins the constant where the corpus put it and not
    /// somewhere an easier example would allow.
    #[test]
    fn short_whole_words_keep_their_spaces_too() {
        let timeline = narrowed(&["MY ", "LOVE'S ", "HERE, ", "IT'S ", "NO ", "DREAM "], 42);
        assert!(!timeline.word_ends.divided());
        assert!(timeline.lines[0].text().contains(' '));
    }

    /// English syllables average what short whole words do, so the mean alone would spare them.
    /// None of them is long, and that is what says they are syllables.
    #[test]
    fn short_english_syllables_are_divided_when_none_is_long() {
        let timeline = narrowed(
            &[
                "wal ", "king ", "down ", "the ", "ci ", "ty ", "road ", "at ", "night ", "a ",
                "lone ", "wait ", "ing ",
            ],
            52,
        );
        assert_eq!(timeline.word_ends, WordEnds::EverySyllableSpaced);
    }

    /// Whole words at the same mean carry long ones among them, and keep their spaces.
    #[test]
    fn short_whole_words_with_long_ones_among_them_keep_their_spaces() {
        let timeline = narrowed(
            &[
                "I ",
                "remember ",
                "you ",
                "and ",
                "me ",
                "in ",
                "the ",
                "rain ",
                "so ",
                "long ",
                "ago ",
            ],
            44,
        );
        assert_eq!(timeline.word_ends, WordEnds::AsWritten);
        assert!(timeline.lines[0].text().contains(' '));
    }

    /// A break marker says where a line ends and nothing about words, so a file that marks every
    /// line and spaces every syllable is still divided.
    #[test]
    fn line_markers_do_not_leave_every_syllable_spaced() {
        let mut raws = unmarked(&["can ", "ta ", "re ", "mos "], 40);
        for raw in raws.iter_mut().step_by(4).skip(1) {
            raw.break_before = LineBreak::Line;
        }
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert_eq!(timeline.word_ends, WordEnds::EverySyllableSpaced);
        let text = timeline.lines[0].text();
        assert!(!text.contains(' '), "no full space is left: {text:?}");
    }

    /// The mean fragment length still spares whole words when the file marks its lines.
    #[test]
    fn a_marked_file_of_whole_words_keeps_its_spaces() {
        let mut raws = unmarked(&["guitarra ", "cantando ", "sozinho ", "amanhece "], 40);
        for raw in raws.iter_mut().step_by(4).skip(1) {
            raw.break_before = LineBreak::Line;
        }
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(!timeline.word_ends.divided());
        assert!(timeline.lines[0].text().contains(' '));
    }

    /// The no-space rule is judged only in an unmarked stream.
    #[test]
    fn line_markers_leave_an_unspaced_file_alone() {
        let mut raws = unmarked(&["can", "ta", "re", "mos"], 40);
        raws[8].break_before = LineBreak::Line;
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert_eq!(timeline.word_ends, WordEnds::AsWritten);
    }

    /// A leading space is how the other convention marks a word start, so one of them is the file
    /// saying where a word begins and must not be overruled.
    #[test]
    fn a_leading_space_anywhere_leaves_the_spacing_alone() {
        let mut raws = unmarked(&["can ", "ta ", "re ", "mos "], 40);
        raws[17].text = " ta".to_owned();
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(!timeline.word_ends.divided());
    }

    #[test]
    fn a_file_that_marks_word_ends_keeps_its_spaces() {
        // Half the syllables end a word, which is what the convention looks like.
        let timeline = narrowed(&["can", "ta ", "re", "mos "], 40);
        assert!(!timeline.word_ends.divided());
    }

    #[test]
    fn too_few_syllables_are_not_judged() {
        let timeline = narrowed(&["can ", "ta ", "re ", "mos "], MIN_JUDGED_SYLLABLES - 1);
        assert!(!timeline.word_ends.divided());
        let timeline = narrowed(&["can ", "ta ", "re ", "mos "], MIN_JUDGED_SYLLABLES);
        assert!(timeline.word_ends.divided());
    }

    /// The complaint this answers: a file of one fragment per note with no separator at all draws
    /// `andthisiscrazybutheresmynumber` across the television.
    #[test]
    fn a_file_that_marks_no_boundary_at_all_is_drawn_with_narrow_dividers() {
        let timeline = narrowed(&["can", "ta", "re", "mos"], 40);
        assert!(timeline.word_ends.divided());
        let text = timeline.lines[0].text();
        assert!(
            text.contains(SYLLABLE_DIVIDER),
            "the fragments are drawn apart: {text:?}"
        );
        assert!(
            !text.contains(' '),
            "and not on anything that reads as a word end: {text:?}"
        );
        assert!(
            !text.ends_with(SYLLABLE_DIVIDER),
            "a gap nothing follows sets the centered line off center: {text:?}"
        );
        assert_eq!(timeline.granularity(), LyricGranularity::SyllableLevel);
    }

    /// Spacing this file uses is the file saying where its words are, and it is then trusted.
    #[test]
    fn a_file_that_spaces_some_of_its_fragments_is_left_joined() {
        let timeline = narrowed(&["can", "ta", "re ", "mos"], 40);
        assert!(!timeline.word_ends.divided());
        assert!(!timeline.lines[0].text().contains(SYLLABLE_DIVIDER));
    }

    /// **One space in hundreds of fragments is noise, not a convention**, and the corpus file this
    /// answers holds exactly one: an elided-space underscore, which resolves to a space before the
    /// spacing is judged. Requiring none left that file drawn as one run of letters.
    #[test]
    fn a_stray_space_does_not_stop_an_unspaced_file_being_divided() {
        let mut raws = unmarked(&["can", "ta", "re", "mos"], 40);
        raws[17].text = "re ".to_owned();
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert_eq!(timeline.word_ends, WordEnds::NoneSpaced);
        let text = timeline.plain_text();
        assert!(text.contains(SYLLABLE_DIVIDER));
        assert!(
            text.contains("re re"),
            "the one space the file wrote is left as it wrote it: {text:?}"
        );
        assert!(
            !text.contains(&format!("re {SYLLABLE_DIVIDER}")),
            "and takes no second gap beside it: {text:?}"
        );
    }

    #[test]
    fn one_line_marker_leaves_an_unspaced_file_joined() {
        let mut raws = unmarked(&["can", "ta", "re", "mos"], 40);
        raws[8].break_before = LineBreak::Line;
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(!timeline.word_ends.divided());
        assert!(!timeline.lines[0].text().contains(SYLLABLE_DIVIDER));
    }

    /// **A script that writes its words without spaces is written correctly already**, so dividing
    /// its fragments is the same error the other way round.
    #[test]
    fn a_file_in_a_script_that_spaces_nothing_is_left_joined() {
        let timeline = narrowed(&["こ", "ん", "に", "ち", "は"], 40);
        assert!(!timeline.word_ends.divided());
        assert!(!timeline.lines[0].text().contains(SYLLABLE_DIVIDER));
    }

    /// One Han character in a Latin file is enough, because a file mixing the two is a file whose
    /// spacing this rule cannot speak for.
    #[test]
    fn one_character_of_such_a_script_is_enough() {
        let mut raws = unmarked(&["can", "ta", "re", "mos"], 40);
        raws[3].text = "愛".to_owned();
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(!timeline.word_ends.divided());
    }

    #[test]
    fn too_few_unspaced_syllables_are_not_judged() {
        let timeline = narrowed(&["can", "ta", "re", "mos"], MIN_JUDGED_SYLLABLES - 1);
        assert!(!timeline.word_ends.divided());
        let timeline = narrowed(&["can", "ta", "re", "mos"], MIN_JUDGED_SYLLABLES);
        assert!(timeline.word_ends.divided());
    }

    /// Where the two rules part company. A file that spaces every fragment keeps the one it left
    /// out joined to the next, because a run of spaced fragments is what it wrote.
    #[test]
    fn a_missing_space_in_a_spaced_file_is_not_filled_in() {
        let mut raws = unmarked(&["can ", "ta ", "re ", "mos "], 40);
        raws[1].text = "ta".to_owned();
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(timeline.word_ends.divided());
        assert!(
            timeline.lines[0]
                .text()
                .starts_with(&format!("can{SYLLABLE_DIVIDER}tare{SYLLABLE_DIVIDER}")),
            "{:?}",
            timeline.lines[0].text()
        );
    }

    /// The divider is whitespace, so the two functions that read a line as text still see ends.
    #[test]
    fn a_narrowed_preview_line_is_trimmed_like_any_other() {
        let timeline = narrowed(&["can ", "ta ", "re ", "mos "], 40);
        let preview = timeline.preview(1);
        assert_eq!(preview.len(), 1);
        assert!(!preview[0].starts_with(SYLLABLE_DIVIDER));
        assert!(!preview[0].ends_with(SYLLABLE_DIVIDER));
    }

    /// The redaction walks byte offsets over `text()`, and the divider is three bytes where a space
    /// is one — so this is the case that proves the walk survives the rewrite.
    #[test]
    fn an_address_in_a_narrowed_line_is_still_masked() {
        let mut raws = unmarked(&["can ", "ta ", "re ", "mos "], 40);
        raws[0].text = "midis@example.com ".to_owned();
        let timeline = build_timeline(raws, inference(), ticks_to_ms);
        assert!(timeline.word_ends.divided());
        let first = &timeline.lines[0];
        assert!(first.contact_redacted);
        assert!(
            !first.text().contains("midis@example.com"),
            "the address is gone from the joined line: {:?}",
            first.text()
        );
        assert!(
            !first.syllables[0].text.contains("midis@example.com"),
            "and from the syllable the screen draws: {:?}",
            first.syllables[0].text
        );
    }
}
