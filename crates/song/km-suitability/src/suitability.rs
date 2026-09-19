//! Rating how well a file works as a karaoke song, from 0 to 10.
//!
//! The corpus is of mixed quality: files with no lyrics, lyrics dumped at tick 0, lyrics with no
//! music, two-track sketches. Suitability exists so a catalog can be sorted and filtered rather
//! than leaving singers to find the bad ones by picking them.
//!
//! It rates **files, not performances** — there is no scoring of singers anywhere in this project.
//!
//! The number alone would be untrustworthy, so every rating carries its per-component breakdown and
//! a list of warnings saying what is wrong. That makes a low one explainable, and it means the
//! weights can be revised later with `km-pack reanalyze` instead of being frozen by the first guess.

use km_song::{LyricGranularity, Song};
use serde::Serialize;

use crate::channel::ChannelStats;
use crate::melody::{MelodyOutcome, MelodySignal};
use crate::thresholds::Thresholds;

/// Which component of the suitability a value came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Breakdown {
    /// Are there lyrics, and are they timed per syllable? 0 to 3.
    pub lyrics: u8,
    /// Do the lyric timings line up with the music? 0 to 3.
    pub sync: u8,
    /// Is the backing spread across channels, or piled onto one? 0 or 2.
    ///
    /// **Not whether a melody channel was found.** Detection failing says the tool could not pick
    /// the tune out with confidence, which is a statement about the tool and about how the file was
    /// named and voiced — a well-made arrangement whose guide track is called `Track 3` sings
    /// exactly as well as one that says `MELODY`. What a file cannot recover from is everything
    /// being on one channel: no instrument can be balanced against another, and the key change and
    /// the guide-melody toggle both act on channels that are not there.
    pub channels: u8,
    /// Is this a real arrangement rather than a sketch? 0 to 2.
    pub arrangement: u8,
}

impl Breakdown {
    /// The components summed, which is the suitability.
    pub fn total(&self) -> u8 {
        self.lyrics + self.sync + self.channels + self.arrangement
    }
}

/// A specific problem found in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// No lyrics of any kind, so the file cannot be sung from.
    NoLyrics,
    /// A handful of syllables and no more — usually the arranger's credits in the lyric track.
    NegligibleLyrics,
    /// Real words that stop early: a verse timed, then nothing for the rest of the song.
    ///
    /// A different fault from [`Self::NegligibleLyrics`] and worth its own name. The text reads as a
    /// song and is one; the file simply is not timed past the first few seconds of it, which is only
    /// visible by comparing where the words end against how long the music runs.
    PartialLyrics,
    /// The lyric track holds chord names, `A# 7th` and `D# 6`, where the words should be.
    ///
    /// A style demo or a player's chart rather than a karaoke file. The text is plentiful and timed
    /// across the whole song, so the quantity test passes it; what fails is that none of it is a
    /// word, and saying "only 0 syllables" about fifty lines of text would not be believed.
    ChordNamesOnly,
    /// Real lyrics, but thin — fewer words than a song usually has, or silent for most of its length.
    SparseLyrics,
    /// Real words, well timed, and over before there is anything to sing.
    ///
    /// A different fault from [`Self::PartialLyrics`], which is a file whose words stop early
    /// against the music it has. This is a file with no more music to stop against: the words cover
    /// it and there is barely any of it. Coverage is a fraction and answers such a file perfectly,
    /// which is why the span is measured beside it.
    BriefSinging,
    /// A space after every syllable or after none, so nothing in the file says where a word ends.
    ///
    /// The words are there and they are timed; they simply cannot be drawn as words, because the
    /// file claims each fragment is one or the whole verse is one. Nothing recovers the boundaries,
    /// so a copy of the same song that marks them is the better file and this is how it comes to
    /// rank above this one.
    NoWordBoundaries,
    /// Lyrics arrive a line at a time, so the highlight cannot follow the words.
    LineLevelLyrics,
    /// Every lyric timing point is at tick 0 — the words are there but the timing is not.
    LyricsAllAtZero,
    /// Lyrics but no notes: nothing to sing along to.
    LyricsWithoutNotes,
    /// The lyric timings mostly do not coincide with any note.
    PoorLyricSync,
    /// No melody channel could be identified, so the guide-melody toggle is unavailable.
    ///
    /// **Reported and not scored.** It says what the machine will not be able to offer, which is
    /// worth knowing; it does not say the file is worse, because how well a song sings does not
    /// depend on whether this tool could name its tune.
    NoMelodyChannel,
    /// Every instrument is on one channel, so nothing in the backing can be separated.
    SingleChannel,
    /// Fewer separate instrument channels than a full arrangement.
    FewChannels,
    /// No drum channel.
    NoDrumChannel,
    /// The length is implausible for a song.
    ImplausibleDuration,
    /// Very few notes for the length, suggesting a stub or a broken file.
    SparseNotes,
    /// The lyric encoding was a fallback guess, so the text may be wrong.
    EncodingGuessed,
}

/// A problem with a human-readable explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    /// Machine-readable code, stable across revisions of the wording.
    pub code: WarningCode,
    /// What is wrong, in words a packager can act on.
    pub message: String,
}

impl Warning {
    fn new(code: WarningCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// How suitable a file is as a karaoke song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suitability {
    /// 0 to 10. Rates the file, never a singer.
    pub value: u8,
    /// Where it came from.
    pub breakdown: Breakdown,
    /// What is wrong with the file.
    pub warnings: Vec<Warning>,
}

impl Suitability {
    /// Whether any hard defect makes the file unusable rather than merely poor.
    ///
    /// `NegligibleLyrics` belongs here for the same reason `NoLyrics` does: a file whose lyric track
    /// holds the arranger's telephone number cannot be sung from, and the fact that the text exists
    /// makes it *harder* to spot than an instrumental, not easier.
    ///
    /// `BriefSinging` belongs here on the same reasoning read from the other side: such a file can
    /// be sung from and is over before it is worth having been chosen, and it is harder to spot than
    /// either, every measurement but the one passing.
    pub fn has_hard_defect(&self) -> bool {
        self.warnings.iter().any(|w| {
            matches!(
                w.code,
                WarningCode::NoLyrics
                    | WarningCode::NegligibleLyrics
                    | WarningCode::PartialLyrics
                    | WarningCode::BriefSinging
                    | WarningCode::ChordNamesOnly
                    | WarningCode::LyricsAllAtZero
                    | WarningCode::LyricsWithoutNotes
            )
        })
    }

    /// Whether there is nothing on the screen for a singer to follow.
    ///
    /// A narrower question than [`Self::has_hard_defect`], and about the *display* rather than about
    /// the file: it decides whether the machine draws the words at all. Three codes answer yes.
    /// [`WarningCode::LyricsAllAtZero`] leaves the text standing unchanged from the first bar to the
    /// last, so there is no highlight to follow and never was.
    /// [`WarningCode::NegligibleLyrics`] is the arranger's name, an email and a web address where
    /// the words should be. [`WarningCode::ChordNamesOnly`] is a player's chart, timed across the
    /// whole song and perfectly synced, with not one word in it.
    ///
    /// [`WarningCode::PartialLyrics`] is deliberately not among them although it is a hard defect:
    /// those are the song's own words, timed for a verse and then stopped, and half a verse somebody
    /// can sing is worth more than an empty screen. [`WarningCode::NoLyrics`] is not among them
    /// either, because a file with no words draws none already.
    pub fn words_cannot_be_followed(&self) -> bool {
        self.warnings.iter().any(|w| {
            matches!(
                w.code,
                WarningCode::NegligibleLyrics
                    | WarningCode::ChordNamesOnly
                    | WarningCode::LyricsAllAtZero
            )
        })
    }
}

/// Rates one file.
pub fn assess(
    song: &Song,
    channels: &[ChannelStats],
    melody: &MelodyOutcome,
    thresholds: &Thresholds,
) -> Suitability {
    let mut warnings = Vec::new();
    let mut breakdown = Breakdown::default();

    let granularity = song.lyrics.granularity();
    let non_drum: Vec<&ChannelStats> = channels.iter().filter(|c| !c.is_drums()).collect();
    let has_drums = channels.iter().any(ChannelStats::is_drums);
    let note_count = song.note_count();
    let duration_ms = song.duration_ms();

    // --- Lyrics: are there any, is there enough of them, and how finely timed? -----------------
    //
    // Quantity is asked *before* granularity, because the two answer different questions and the
    // second is meaningless without the first. A file whose lyric track holds a phone number and an
    // email address is syllable-timed, perfectly synced and completely unsingable — it scored 10/10
    // until this existed.
    let content = LyricContent::measure(song, duration_ms);
    let syllable_ticks = song.lyrics.syllable_ticks();
    let all_at_zero = !syllable_ticks.is_empty() && syllable_ticks.iter().all(|&t| t == 0);
    let no_notes = note_count == 0;
    // The coarse complaint yields to the specific ones. A file whose timing points are all at tick
    // zero, or which has no notes at all, has *no span* and so measures as negligible — but "every
    // lyric timing point is at the start of the song" is the sentence that tells somebody what is
    // actually wrong with it, and reporting the vaguer one instead would be a loss.
    let negligible = content.is_negligible(thresholds) && !all_at_zero && !no_notes;
    let chord_chart = content.is_chord_chart(thresholds) && !all_at_zero && !no_notes;
    let brief = content.is_too_brief(thresholds) && !all_at_zero && !no_notes;

    breakdown.lyrics = match granularity {
        LyricGranularity::None => {
            warnings.push(Warning::new(
                WarningCode::NoLyrics,
                "no lyrics found in any supported format; the file cannot be sung from",
            ));
            0
        }
        // Ahead of the quantity test, because it is the more specific sentence: a chart of a few
        // lines may also be negligible, and "these are chord names" is what tells somebody why.
        _ if chord_chart => {
            warnings.push(Warning::new(
                WarningCode::ChordNamesOnly,
                format!(
                    "the lyric track holds chord names rather than words ({} of {} lines); there \
                     is nothing to sing along to",
                    content.chord_lines, content.judged_lines
                ),
            ));
            0
        }
        _ if negligible => {
            let (code, message) = content.unsingable(thresholds, duration_ms);
            warnings.push(Warning::new(code, message));
            0
        }
        // After the quantity test, because a business card is brief as well as negligible and the
        // syllable count is the fault worth naming. What is left here is a file whose words are a
        // song and whose song is over before it is worth having chosen.
        _ if brief => {
            warnings.push(Warning::new(
                WarningCode::BriefSinging,
                format!(
                    "only {} of singing across {}; there is too little of it to be worth choosing",
                    format_duration(content.sung_ms),
                    format_duration(duration_ms)
                ),
            ));
            0
        }
        LyricGranularity::LineLevel => {
            warnings.push(Warning::new(
                WarningCode::LineLevelLyrics,
                "lyrics are timed a line at a time, so the highlight cannot follow the words",
            ));
            1
        }
        LyricGranularity::SyllableLevel if content.is_sparse(thresholds) => {
            warnings.push(Warning::new(
                WarningCode::SparseLyrics,
                format!(
                    "only {} syllable(s), covering {:.0}% of the song; thinner than a song usually is",
                    content.syllables,
                    content.coverage * 100.0
                ),
            ));
            2
        }
        // A point off, not two: the highlight really does follow the singing, so this is nowhere
        // near line-level timing. But the words as drawn are not the words, which is the same order
        // of loss as lyrics thinner than a song's — and that is what the arm above scores 2.
        LyricGranularity::SyllableLevel if song.lyrics.word_ends.divided() => {
            warnings.push(Warning::new(
                WarningCode::NoWordBoundaries,
                "the file spaces every syllable or none of them, so nothing in it says where a \
                 word ends; the words are drawn divided rather than joined",
            ));
            2
        }
        LyricGranularity::SyllableLevel => 3,
    };

    // --- Sync: do the lyric timings coincide with the music? -----------------------------------
    //
    // Negligible lyrics score nothing here either. "Every syllable lands near a note" is not a
    // measurement when there are eleven syllables and four thousand notes to land near. A chord
    // chart scores nothing for the opposite reason: its names land on the chord changes, so they
    // sync perfectly, and there is still nothing to sing. A file over in forty seconds scores
    // nothing for a third reason: how well it was timed is a question about a song worth singing.
    breakdown.sync = if granularity == LyricGranularity::None || negligible || chord_chart || brief
    {
        0
    } else if all_at_zero {
        warnings.push(Warning::new(
            WarningCode::LyricsAllAtZero,
            "every lyric timing point is at the start of the song; the words cannot be followed",
        ));
        0
    } else if no_notes {
        warnings.push(Warning::new(
            WarningCode::LyricsWithoutNotes,
            "the file has lyrics but no notes, so there is nothing to sing along to",
        ));
        0
    } else {
        // Which notes the question is asked about decides whether it is a question at all. In a
        // full arrangement a syllable is near *something* whatever it does: one file whose words
        // run eleven seconds early by the end still put 94% of them within the window of some note
        // and took full marks here. Asked of the sung line alone, the same words score 36%.
        //
        // The melody may only judge the lyrics when it was identified without them. Where
        // alignment chose the channel, alignment agrees with the channel it chose, and the file
        // marks its own work.
        let sung = sung_line(melody, &non_drum);
        let alignment = match sung {
            Some(stats) => sync_alignment(song, &[stats], &syllable_ticks, thresholds),
            None => sync_alignment(song, &non_drum, &syllable_ticks, thresholds),
        };
        if alignment < thresholds.poor_sync_alignment {
            warnings.push(Warning::new(
                WarningCode::PoorLyricSync,
                format!(
                    "only {:.0}% of syllables land near a note{}; the lyrics may not be timed to \
                     this arrangement",
                    alignment * 100.0,
                    if sung.is_some() { " of the melody" } else { "" }
                ),
            ));
        }
        // Scaled to the 3 points this component is worth, then rounded once.
        (alignment * 3.0).round().clamp(0.0, 3.0) as u8
    };

    // --- Melody: said, and not scored ----------------------------------------------------------
    if !melody.is_found() {
        warnings.push(Warning::new(
            WarningCode::NoMelodyChannel,
            "no melody channel could be identified with confidence, so the guide-melody toggle \
             is unavailable for this song",
        ));
    }

    // --- Channels: an arrangement, or everything piled onto one? -------------------------------
    //
    // The one channel fact a file cannot be good in spite of. Two instruments on one channel share
    // a patch, a volume and a transposition, so nothing downstream can act on either of them
    // separately — which is what the guide-melody toggle and the key change both need.
    breakdown.channels = if non_drum.len() > 1 {
        2
    } else {
        warnings.push(Warning::new(
            WarningCode::SingleChannel,
            "every instrument is on one channel, so nothing in the backing can be separated",
        ));
        0
    };

    // --- Arrangement: a real backing track, or a sketch? ---------------------------------------
    let mut arrangement = 0u8;
    if non_drum.len() >= thresholds.good_channel_count {
        arrangement += 1;
    } else {
        warnings.push(Warning::new(
            WarningCode::FewChannels,
            format!(
                "only {} instrument channel(s); a full arrangement usually has at least {}",
                non_drum.len(),
                thresholds.good_channel_count
            ),
        ));
    }

    let duration_ok = thresholds.is_plausible_duration(duration_ms);
    if !duration_ok {
        warnings.push(Warning::new(
            WarningCode::ImplausibleDuration,
            format!(
                "length of {} is implausible for a song",
                format_duration(duration_ms)
            ),
        ));
    }
    if !has_drums {
        warnings.push(Warning::new(
            WarningCode::NoDrumChannel,
            "no drum channel; the backing track will feel thin",
        ));
    }
    let density_ok = is_dense_enough(note_count, duration_ms, thresholds);
    if !density_ok {
        warnings.push(Warning::new(
            WarningCode::SparseNotes,
            format!(
                "only {note_count} note(s) across {}",
                format_duration(duration_ms)
            ),
        ));
    }
    if has_drums && duration_ok && density_ok {
        arrangement += 1;
    }
    breakdown.arrangement = arrangement;

    // Not a suitability component: the text may simply be wrong, which a packager can fix once by
    // declaring the encoding, rather than every listener noticing it at play time.
    if song.decoder.source() == km_song::EncodingSource::Fallback
        && granularity != LyricGranularity::None
    {
        warnings.push(Warning::new(
            WarningCode::EncodingGuessed,
            format!(
                "lyric encoding fell back to {}; set it explicitly if the text looks wrong",
                song.decoder.name()
            ),
        ));
    }

    Suitability {
        value: breakdown.total(),
        breakdown,
        warnings,
    }
}

/// How much of a song is actually sung, once obvious credits are set aside.
///
/// Three numbers, because they fail in different ways and a file needs to pass all of them.
/// **Coverage** catches a credit block occupying a few seconds of a three-minute file, which no real
/// song's words do. **Syllables** catches what coverage cannot — a single word at the start and
/// another at the end spans the whole song and is still not a lyric. **The span** catches what
/// neither can: a file whose words cover all of it and all of it is forty seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LyricContent {
    /// Syllables that are not part of a credit line.
    pub syllables: usize,
    /// Milliseconds between the first and last counted syllable.
    pub sung_ms: u32,
    /// Fraction of the song's length between the first and last counted syllable.
    pub coverage: f32,
    /// Lines set aside as credits rather than lyrics.
    pub credit_lines: usize,
    /// Lines with text that are not credits: what the chord share is measured against.
    pub judged_lines: usize,
    /// Of those, lines made of nothing but chord names.
    ///
    /// **Still counted as syllables.** One line reading `A` or `E` may be a word, so a line is never
    /// set aside for looking like a chord; only a file made mostly of them is judged, by
    /// [`Self::is_chord_chart`].
    pub chord_lines: usize,
}

impl LyricContent {
    /// Measures a song's lyric content.
    pub fn measure(song: &Song, duration_ms: u32) -> Self {
        let mut syllables = 0usize;
        let mut credit_lines = 0usize;
        let mut judged_lines = 0usize;
        let mut chord_lines = 0usize;
        let mut first: Option<u32> = None;
        let mut last: u32 = 0;

        for line in &song.lyrics.lines {
            // **`contact_redacted` first, and it is not a new rule — it is how the old one keeps
            // working.** `km_song` now replaces an address in the words with a dash before anything
            // reads them, so a business card that used to arrive here as
            // `someone@example.com (0**19) 5550123` arrives as `— (0**19) —` and
            // `is_credit_line` no longer recognizes it. Left alone, every such line would start
            // counting as lyric and **the stored 0–10 suitability of files already packaged would
            // move** — which `docs/architecture/song.md` names as the trap that keeps this function
            // and `looks_like_a_banner` separate. The flag says "there was an address here", which
            // is the fact `is_credit_line` was reading the text to find out.
            let text = line.text();
            if line.contact_redacted || is_credit_line(&text) {
                credit_lines += 1;
                continue;
            }
            if line.syllables.is_empty() {
                continue;
            }
            if !text.trim().is_empty() {
                judged_lines += 1;
                if is_chord_symbol(&text) {
                    chord_lines += 1;
                }
            }
            syllables += line.syllables.len();
            let start = song.tempo_map.tick_to_ms(line.start_tick);
            let end = song.tempo_map.tick_to_ms(line.end_tick);
            first = Some(first.map_or(start, |current: u32| current.min(start)));
            last = last.max(end);
        }

        let sung_ms = match first {
            Some(first) if last > first => last - first,
            _ => 0,
        };
        let coverage = if duration_ms > 0 {
            sung_ms as f32 / duration_ms as f32
        } else {
            0.0
        };
        Self {
            syllables,
            sung_ms,
            coverage: coverage.clamp(0.0, 1.0),
            credit_lines,
            judged_lines,
            chord_lines,
        }
    }

    /// Whether the lyric track is a chord chart: enough lines, nearly all of them chord names.
    pub fn is_chord_chart(&self, thresholds: &Thresholds) -> bool {
        self.chord_lines >= thresholds.min_chord_lines
            && self.chord_lines as f32 >= self.judged_lines as f32 * thresholds.chord_chart_share
    }

    /// Whether there is so little here that the file cannot be sung from.
    pub fn is_negligible(&self, thresholds: &Thresholds) -> bool {
        self.syllables < thresholds.min_lyric_syllables
            || self.coverage < thresholds.min_lyric_coverage
    }

    /// Whether the lyrics are real but thinner than a song's usually are.
    pub fn is_sparse(&self, thresholds: &Thresholds) -> bool {
        self.syllables < thresholds.sparse_lyric_syllables
            || self.coverage < thresholds.sparse_lyric_coverage
    }

    /// Whether there is too little singing here for the file to be worth choosing.
    pub fn is_too_brief(&self, thresholds: &Thresholds) -> bool {
        !thresholds.is_enough_singing(self.sung_ms)
    }

    /// Which fault this is, and how to say it.
    ///
    /// The two failures look the same in the suitability and read quite differently to a person. *Too few
    /// syllables* is a file whose lyric track holds a phone number. *Words that stop early* is a real
    /// song — the text scans, it rhymes — that was only timed for its first verse, which is invisible
    /// until you compare where the words end against how long the music runs. Telling a packager
    /// "there is nothing to sing along to" about a file that plainly has a verse in it would be the
    /// sort of wrong that makes somebody stop trusting the warnings.
    fn unsingable(&self, thresholds: &Thresholds, duration_ms: u32) -> (WarningCode, String) {
        let credits = match self.credit_lines {
            0 => String::new(),
            1 => " (1 line looked like credits and was not counted)".to_owned(),
            n => format!(" ({n} lines looked like credits and were not counted)"),
        };

        if self.syllables < thresholds.min_lyric_syllables {
            return (
                WarningCode::NegligibleLyrics,
                format!(
                    "only {} syllable(s) of lyrics, covering {:.0}% of the song; there is nothing to \
                     sing along to{credits}",
                    self.syllables,
                    self.coverage * 100.0
                ),
            );
        }
        (
            WarningCode::PartialLyrics,
            format!(
                "the words stop after {:.0}% of the song — {} syllable(s), ending around {}; the \
                 rest is not timed for singing{credits}",
                self.coverage * 100.0,
                self.syllables,
                format_duration((self.coverage * duration_ms as f32) as u32)
            ),
        )
    }
}

/// How long a song is sung for: first counted syllable to last, in milliseconds.
///
/// **For a caller that wants the span and not the rest of the measurement**, which is every caller
/// outside this crate: packaging asks it of an UltraStar song, and a corpus sweep asks it of a MIDI
/// file. A length of zero is passed in because coverage is the one thing here that needs one, and
/// nobody asking this question is asking that one.
///
/// Zero where the file has no counted syllables, which is the case a caller answers with the file's
/// own length.
pub fn sung_span_ms(song: &Song) -> u32 {
    LyricContent::measure(song, 0).sung_ms
}

/// Whether a lyric line is somebody's contact details rather than words to sing.
///
/// Deliberately narrow, and deliberately not a judgment about language: an email address, a web
/// address, or a line that is mostly telephone digits. A name — `Cleiton Ferraz` — is *not* caught,
/// because a name can be a lyric and the quantity test settles that file anyway. The point of this is
/// the case quantity alone would miss: forty lines of credits scrolling past, which counts as plenty
/// of syllables covering plenty of the song.
fn is_credit_line(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_lowercase();

    // An email address: an `@` with something that looks like a domain after it.
    // Any whitespace ends the address, not the ASCII space alone: a line whose word gaps are
    // `SYLLABLE_DIVIDER` must reach the same verdict as the same line spaced with spaces.
    if let Some((_, after)) = lower.split_once('@')
        && after.contains('.')
        && !after.chars().any(char::is_whitespace)
    {
        return true;
    }
    // A web address.
    if lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return true;
    }
    // Mostly digits, with enough of them to be a telephone number rather than a year or a count-in.
    let digits = trimmed.chars().filter(char::is_ascii_digit).count();
    let solid = trimmed.chars().filter(|c| !c.is_whitespace()).count();
    if digits >= 7 && solid > 0 && digits * 2 >= solid {
        return true;
    }
    false
}

/// Whether a lyric line is made of chord names and nothing else.
///
/// A chord is a root `A` to `G` with an optional `#` or `b`, then a quality and a slash bass, written
/// joined (`F#m7`, `A7/G`) or spread over separate words (`A  min 7th    /G`). A line passes only if
/// every word is part of a chord and at least one word is a root, so `A little love`, `Be my baby`
/// and `dim the lights` all fail on a word that is not. Roots are uppercase for the same reason:
/// `a` and `be` are English.
///
/// **A verdict about one line, and not enough on its own.** `A` or `E` alone can be a lyric;
/// [`LyricContent::is_chord_chart`] is what decides a file.
fn is_chord_symbol(text: &str) -> bool {
    let mut roots = 0usize;
    for word in text.split_whitespace() {
        if let Some(rest) = strip_root(word) {
            if !is_chord_suffix(rest) {
                return false;
            }
            roots += 1;
        } else if !is_chord_suffix(word) {
            return false;
        }
    }
    roots > 0
}

/// A note name at the start of `word`, returning what follows it.
fn strip_root(word: &str) -> Option<&str> {
    let rest = word.strip_prefix(['A', 'B', 'C', 'D', 'E', 'F', 'G'])?;
    Some(rest.strip_prefix(['#', 'b']).unwrap_or(rest))
}

/// Whether `text` is a chord's quality and bass: `min7`, `7th`, `Maj`, `sus4`, `/G`, or nothing.
fn is_chord_suffix(text: &str) -> bool {
    // Longest first, so `min` is not read as `m` followed by `in`.
    const PIECES: [&str; 17] = [
        "maj", "Maj", "MAJ", "min", "Min", "MIN", "dim", "aug", "sus", "add", "th", "M", "m", "+",
        "-", "#", "b",
    ];
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(bass) = rest.strip_prefix('/') {
            return strip_root(bass).is_some_and(str::is_empty);
        }
        if let Some(after) = rest.strip_prefix(|c: char| c.is_ascii_digit()) {
            rest = after;
        } else if let Some(after) = PIECES.iter().find_map(|piece| rest.strip_prefix(piece)) {
            rest = after;
        } else {
            return false;
        }
    }
    true
}

/// The channel carrying the sung line, when the lyrics played no part in finding it.
///
/// Drums cannot be it, so a melody reported on channel 9 yields nothing and the wider measurement
/// stands.
fn sung_line<'a>(
    melody: &MelodyOutcome,
    non_drum: &[&'a ChannelStats],
) -> Option<&'a ChannelStats> {
    let found = melody.channel()?;
    if found.signals.contains(&MelodySignal::LyricAlignment) {
        return None;
    }
    non_drum
        .iter()
        .copied()
        .find(|stats| stats.channel == found.channel)
}

/// Fraction of syllables landing near a note onset on one of the given channels.
///
/// Drums are excluded by every caller: lyrics coincide with the beat in almost every file, so
/// counting drum hits would make even unsynced lyrics look well timed.
fn sync_alignment(
    song: &Song,
    channels: &[&ChannelStats],
    syllable_ticks: &[u32],
    thresholds: &Thresholds,
) -> f32 {
    if syllable_ticks.is_empty() {
        return 0.0;
    }
    let mut onsets_ms: Vec<u32> = channels
        .iter()
        .flat_map(|c| c.onset_ticks.iter())
        .map(|&tick| song.tempo_map.tick_to_ms(tick))
        .collect();
    if onsets_ms.is_empty() {
        return 0.0;
    }
    onsets_ms.sort_unstable();
    onsets_ms.dedup();

    let window = thresholds.sync_window_ms;
    let matched = syllable_ticks
        .iter()
        .map(|&tick| song.tempo_map.tick_to_ms(tick))
        .filter(|&target| nearest_distance(&onsets_ms, target) <= window)
        .count();
    matched as f32 / syllable_ticks.len() as f32
}

/// Distance from `target` to the closest value in a sorted slice.
fn nearest_distance(sorted: &[u32], target: u32) -> u32 {
    match sorted.binary_search(&target) {
        Ok(_) => 0,
        Err(index) => {
            let after = sorted.get(index).map(|&v| v.abs_diff(target));
            let before = index
                .checked_sub(1)
                .and_then(|i| sorted.get(i))
                .map(|&v| v.abs_diff(target));
            match (before, after) {
                (Some(a), Some(b)) => a.min(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => u32::MAX,
            }
        }
    }
}

fn is_dense_enough(note_count: usize, duration_ms: u32, thresholds: &Thresholds) -> bool {
    if duration_ms == 0 {
        return false;
    }
    let minutes = f64::from(duration_ms) / 60_000.0;
    let per_minute = note_count as f64 / minutes.max(0.05);
    per_minute >= f64::from(thresholds.min_notes_per_minute)
}

fn format_duration(ms: u32) -> String {
    format!("{}:{:02}", ms / 60_000, (ms / 1_000) % 60)
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;
    use crate::{channel, melody};

    fn assess_bytes(bytes: &[u8]) -> (Suitability, Song) {
        let song = Song::parse(bytes, &ParseOptions::default()).expect("fixture parses");
        let thresholds = Thresholds::default();
        let channels = channel::measure(&song, &thresholds);
        let melody = melody::detect(&song, &channels, &thresholds);
        let suitability = assess(&song, &channels, &melody, &thresholds);
        (suitability, song)
    }

    fn codes(suitability: &Suitability) -> Vec<WarningCode> {
        suitability.warnings.iter().map(|w| w.code).collect()
    }

    /// Which notes the sync question is asked about is the question. These words are a song, they
    /// are timed a syllable at a time and they cover the whole file — and every one of them falls
    /// half a beat from the note it belongs to. An accompaniment striking every sixteenth is near
    /// anything at all, so measured against the arrangement the file is perfectly synced.
    #[test]
    fn words_that_miss_the_melody_lose_what_a_busy_arrangement_would_hide() {
        let bytes = testing::lyrics_against_another_arrangement();
        let (suitability, song) = assess_bytes(&bytes);

        let thresholds = Thresholds::default();
        let channels = channel::measure(&song, &thresholds);
        let non_drum: Vec<&ChannelStats> = channels.iter().filter(|c| !c.is_drums()).collect();
        let ticks = song.lyrics.syllable_ticks();
        assert_eq!(
            sync_alignment(&song, &non_drum, &ticks, &thresholds),
            1.0,
            "against the whole arrangement nothing is out of place"
        );

        assert_eq!(
            suitability.breakdown.lyrics, 3,
            "the words themselves are a song"
        );
        assert_eq!(suitability.breakdown.sync, 0);
        assert_eq!(suitability.value, 7);
        assert_eq!(codes(&suitability), vec![WarningCode::PoorLyricSync]);
        assert!(
            !suitability.has_hard_defect(),
            "badly timed is not unsingable"
        );
    }

    /// A melody identified from the lyrics cannot then be asked whether the lyrics are right.
    #[test]
    fn the_melody_judges_the_words_only_when_it_was_found_without_them() {
        let sung_line_of = |bytes: &[u8]| {
            let song = Song::parse(bytes, &ParseOptions::default()).expect("fixture parses");
            let thresholds = Thresholds::default();
            let channels = channel::measure(&song, &thresholds);
            let melody = melody::detect(&song, &channels, &thresholds);
            let non_drum: Vec<&ChannelStats> = channels.iter().filter(|c| !c.is_drums()).collect();
            sung_line(&melody, &non_drum).map(|stats| stats.channel)
        };

        assert_eq!(
            sung_line_of(&testing::lyrics_against_another_arrangement()),
            Some(0),
            "name, monophony and range found it; the words played no part"
        );
        assert_eq!(
            sung_line_of(&testing::high_quality_song()),
            None,
            "alignment is among the signals that chose the channel"
        );
    }

    /// A wrong name does not decide which notes the words are judged against. The track called
    /// `Melody` rests under every sung word, so it is not the sung line, and the words are measured
    /// against the arrangement, which strikes every beat they land on.
    #[test]
    fn a_riff_named_melody_does_not_mark_well_timed_words_down() {
        let (suitability, _) = assess_bytes(&testing::named_melody_silent_under_the_words());
        assert_eq!(suitability.breakdown.sync, 3);
        assert_eq!(suitability.value, 10);
        assert_eq!(codes(&suitability), vec![WarningCode::NoMelodyChannel]);
    }

    /// The shape the track-picking rule exists for: one file, two timings, and the longer of them
    /// is the one nobody can sing to.
    #[test]
    fn a_file_holding_two_timings_is_rated_on_the_one_it_names() {
        let (suitability, song) = assess_bytes(&testing::two_lyric_tracks_disagreeing());
        assert_eq!(
            song.lyrics.syllable_count(),
            240,
            "the 240-syllable Words track, not the 300-syllable one beside it"
        );
        assert_eq!(suitability.value, 10);
        assert!(suitability.warnings.is_empty());
    }

    #[test]
    fn a_well_formed_song_scores_at_the_top() {
        let (suitability, _) = assess_bytes(&testing::high_quality_song());
        assert_eq!(suitability.breakdown.lyrics, 3);
        assert_eq!(suitability.breakdown.sync, 3);
        assert_eq!(suitability.breakdown.channels, 2);
        assert_eq!(suitability.breakdown.arrangement, 2);
        assert_eq!(suitability.value, 10);
        assert!(
            suitability.warnings.is_empty(),
            "unexpected: {:?}",
            suitability.warnings
        );
        assert!(!suitability.has_hard_defect());
    }

    /// The same arrangement as [`testing::high_quality_song`], differing only in that its spaces
    /// say nothing about words — so the one point between them is the words and nothing else.
    #[test]
    fn a_file_that_marks_no_word_ends_loses_one_point_of_the_ten() {
        let (suitability, _) = assess_bytes(&testing::word_ends_unmarked());
        assert_eq!(suitability.breakdown.lyrics, 2);
        assert_eq!(suitability.breakdown.sync, 3);
        assert_eq!(suitability.value, 9);
        assert_eq!(codes(&suitability), vec![WarningCode::NoWordBoundaries]);
        assert!(
            !suitability.has_hard_defect(),
            "the file is singable, only worse"
        );
    }

    /// The file that marks nothing costs what the file that marks everything costs. The loss is the
    /// same one seen from the other side: the words as drawn are not the words.
    #[test]
    fn a_file_that_marks_no_word_boundary_at_all_loses_the_same_point() {
        let (suitability, _) = assess_bytes(&testing::word_boundaries_unmarked());
        assert_eq!(suitability.breakdown.lyrics, 2);
        assert_eq!(suitability.value, 9);
        assert_eq!(codes(&suitability), vec![WarningCode::NoWordBoundaries]);
        assert!(!suitability.has_hard_defect());
    }

    #[test]
    fn the_suitability_always_equals_its_breakdown() {
        for (name, build) in testing::FIXTURES {
            let (suitability, _) = assess_bytes(&build());
            assert_eq!(
                suitability.value,
                suitability.breakdown.total(),
                "{name}: suitability and breakdown disagree"
            );
            assert!(suitability.value <= 10, "{name}: suitability above 10");
        }
    }

    /// **The guard on the one interaction that could move every stored suitability.**
    ///
    /// `km_song` masks contact details in the words before anything reads them, so the address the
    /// test below relies on never reaches `is_credit_line` as an address — it arrives as a dash.
    /// `LyricContent::measure` therefore reads `contact_redacted` first, and if that check is ever
    /// removed as a tidy-up, these lines start counting as lyric and the suitability of every
    /// already-packaged file with a business card in it changes. Asserted on the measurement rather
    /// than on the suitability, because it is the count that carries the meaning.
    #[test]
    fn a_masked_business_card_is_still_a_credit_line_and_not_lyric() {
        let (_, song) = assess_bytes(&testing::credits_in_the_lyric_track());
        let redacted = song
            .lyrics
            .lines
            .iter()
            .filter(|line| line.contact_redacted)
            .count();
        assert!(redacted > 0, "the fixture carries an address to mask");

        let content = LyricContent::measure(&song, 60_000);
        assert!(
            content.credit_lines >= redacted,
            "every masked line still counts as a credit: {} redacted, {} credits",
            redacted,
            content.credit_lines
        );
    }

    /// The file this whole measurement exists for.
    ///
    /// Same arrangement as `high_quality_song`, same melody, same timing quality — the *only*
    /// difference is that the lyric track holds a name, a telephone number and an email address. It
    /// scored 10/10 before lyric quantity was measured, which is what a corpus of hundreds of
    /// thousands of files makes expensive: the bad ones sort to the top.
    #[test]
    fn a_lyric_track_holding_the_arrangers_business_card_is_not_a_karaoke_file() {
        let (good, _) = assess_bytes(&testing::high_quality_song());
        let (credits, _) = assess_bytes(&testing::credits_in_the_lyric_track());

        assert_eq!(good.value, 10, "the same file with real words is excellent");
        assert_eq!(credits.breakdown.lyrics, 0);
        assert_eq!(
            credits.breakdown.sync, 0,
            "eleven syllables landing near a note is not synchronization"
        );
        assert_eq!(
            credits.breakdown.channels, good.breakdown.channels,
            "the channels and arrangement are untouched, which is why the suitability has to come from here"
        );
        assert_eq!(credits.breakdown.arrangement, good.breakdown.arrangement);
        assert!(credits.value < good.value - 4, "{}", credits.value);

        assert!(codes(&credits).contains(&WarningCode::NegligibleLyrics));
        assert!(credits.has_hard_defect(), "it cannot be sung from at all");

        // And the email line is discounted rather than counted as three syllables of song.
        let message = &credits
            .warnings
            .iter()
            .find(|w| w.code == WarningCode::NegligibleLyrics)
            .expect("the warning")
            .message;
        assert!(message.contains("looked like credits"), "{message}");
    }

    #[test]
    fn a_real_song_timed_only_for_its_first_verse_says_the_words_stop() {
        let (suitability, _) = assess_bytes(&testing::lyrics_that_stop_early());
        assert_eq!(suitability.breakdown.lyrics, 0);
        assert_eq!(suitability.breakdown.sync, 0);
        assert!(suitability.has_hard_defect());

        // Not `NegligibleLyrics`: there are real words here, and telling somebody there is nothing to
        // sing when a verse is plainly there is how a warning stops being believed.
        let codes = codes(&suitability);
        assert!(codes.contains(&WarningCode::PartialLyrics), "{codes:?}");
        assert!(!codes.contains(&WarningCode::NegligibleLyrics), "{codes:?}");
        let message = &suitability
            .warnings
            .iter()
            .find(|w| w.code == WarningCode::PartialLyrics)
            .expect("the warning")
            .message;
        assert!(message.contains("the words stop after"), "{message}");
    }

    #[test]
    fn contact_details_are_not_counted_as_lyrics_but_names_are() {
        assert!(is_credit_line("midis@example.com.br"));
        assert!(is_credit_line("  WWW.Example.COM.BR  "));
        assert!(is_credit_line("https://example.test/midis"));
        assert!(is_credit_line("0**17 3463-1150"));
        assert!(is_credit_line("(017) 9705 4266"));

        // A name is not obviously junk, and a lyric can be a name. The quantity test settles those
        // files; guessing at language here would not.
        assert!(!is_credit_line("Cleiton Ferraz"));
        assert!(!is_credit_line("Descobridor dos sete mares"));
        // Numbers appear in real lyrics, and a year is not a telephone number.
        assert!(!is_credit_line("1999 was the year"));
        assert!(!is_credit_line("one two three four"));
        assert!(!is_credit_line(""));
    }

    #[test]
    fn a_thin_but_real_lyric_keeps_most_of_its_marks_and_is_still_flagged() {
        // Enough words to sing, spread across the song, but fewer than a song usually has.
        let thresholds = Thresholds::default();
        let content = LyricContent {
            syllables: 40,
            sung_ms: 140_000,
            coverage: 0.7,
            credit_lines: 0,
            judged_lines: 0,
            chord_lines: 0,
        };
        assert!(!content.is_negligible(&thresholds), "40 words is singable");
        assert!(content.is_sparse(&thresholds), "but thin enough to say so");

        // A full song is neither.
        let full = LyricContent {
            syllables: 240,
            sung_ms: 160_000,
            coverage: 0.8,
            credit_lines: 0,
            judged_lines: 0,
            chord_lines: 0,
        };
        assert!(!full.is_negligible(&thresholds));
        assert!(!full.is_sparse(&thresholds));
    }

    #[test]
    fn the_thresholds_sit_in_the_gap_the_corpus_actually_shows() {
        // Measured over 198 files of the local corpus: credit blocks carried 1 to 49 syllables and
        // covered 0% to 8% of their file; the songs among them carried 144 or more and covered 56% or
        // better. These numbers are what those two clusters are separated by, and a change to them
        // should be made against fresh measurements rather than by taste.
        let thresholds = Thresholds::default();
        assert!(
            thresholds.min_lyric_syllables >= 12,
            "above the credit blocks"
        );
        assert!(thresholds.min_lyric_syllables <= 120, "below the songs");
        assert!(thresholds.min_lyric_coverage > 0.08);
        assert!(thresholds.min_lyric_coverage < 0.56);
    }

    /// Plenty of text, timed across the whole song and landing on every bar, and not one word of it.
    /// Every other measurement passes it, which is why the names have to be read.
    #[test]
    fn a_lyric_track_of_chord_names_is_not_a_karaoke_file() {
        let (suitability, _) = assess_bytes(&testing::chord_names_as_lyrics());
        assert_eq!(suitability.breakdown.lyrics, 0);
        assert_eq!(suitability.breakdown.sync, 0);
        assert_eq!(suitability.value, 4);
        assert!(
            codes(&suitability).contains(&WarningCode::ChordNamesOnly),
            "{:?}",
            suitability.warnings
        );
        assert!(suitability.has_hard_defect());
    }

    #[test]
    fn a_chord_is_read_joined_or_spread_over_words() {
        for line in [
            "A# 7th",
            "F  Maj 7th",
            "A  min 7th    /G",
            "D  min        F  7th",
            "Bb",
            "F#m7",
            "Csus4",
            "E7/G#",
        ] {
            assert!(is_chord_symbol(line), "{line:?} is a chord");
        }
    }

    #[test]
    fn a_line_with_one_word_that_is_not_a_chord_is_not_a_chord() {
        for line in [
            "",
            "   ",
            "A little love",
            "Be my baby",
            "dim the lights",
            "Am I blue",
            "a",
            "7th",
            "Dime",
        ] {
            assert!(!is_chord_symbol(line), "{line:?} is not a chord");
        }
    }

    /// A song that names a few chords among its words stays a song.
    #[test]
    fn a_file_is_a_chord_chart_only_when_nearly_every_line_is_one() {
        let thresholds = Thresholds::default();
        let content = |chord_lines, judged_lines| LyricContent {
            syllables: 300,
            sung_ms: 160_000,
            coverage: 0.8,
            credit_lines: 0,
            judged_lines,
            chord_lines,
        };
        assert!(content(53, 53).is_chord_chart(&thresholds));
        assert!(content(45, 50).is_chord_chart(&thresholds));
        assert!(!content(10, 40).is_chord_chart(&thresholds));
        assert!(
            !content(5, 5).is_chord_chart(&thresholds),
            "too few lines to judge"
        );
    }

    #[test]
    fn a_file_with_no_lyrics_scores_zero_on_both_lyric_components() {
        let (suitability, _) = assess_bytes(&testing::instrumental());
        assert_eq!(suitability.breakdown.lyrics, 0);
        assert_eq!(suitability.breakdown.sync, 0);
        assert!(codes(&suitability).contains(&WarningCode::NoLyrics));
        assert!(suitability.has_hard_defect());
        // And it lands near the bottom overall, which is suitability doing its job rather than merely
        // recording a defect: a playable file nobody can sing to is still a bad karaoke song.
        assert!(
            suitability.value <= 3,
            "no lyrics should score near the bottom, got {}",
            suitability.value
        );
    }

    /// The three faults that leave nothing on screen to follow, and the two hard defects that do
    /// not. A song whose words are real is never silenced by measurement, however badly the file
    /// treats them.
    #[test]
    fn three_faults_mean_there_is_nothing_on_screen_to_follow() {
        for (name, bytes) in [
            ("credits", testing::credits_in_the_lyric_track()),
            ("chord chart", testing::chord_names_as_lyrics()),
            ("all at zero", testing::lyrics_all_at_zero()),
        ] {
            let (suitability, _) = assess_bytes(&bytes);
            assert!(
                suitability.words_cannot_be_followed(),
                "{name} leaves nothing to follow: {:?}",
                codes(&suitability)
            );
        }

        // A verse timed and then abandoned is a hard defect and is still words somebody can sing
        // for as long as they last, so measurement does not take them away.
        let (early, _) = assess_bytes(&testing::lyrics_that_stop_early());
        assert!(early.has_hard_defect());
        assert!(!early.words_cannot_be_followed());

        // And a file with no words draws none already; silencing it would say a decision had been
        // taken over a file nobody had to decide anything about.
        let (none, _) = assess_bytes(&testing::instrumental());
        assert!(none.has_hard_defect());
        assert!(!none.words_cannot_be_followed());

        // The same arrangement as the credits fixture, differing only in carrying real words.
        let (good, _) = assess_bytes(&testing::high_quality_song());
        assert!(!good.words_cannot_be_followed());
    }

    #[test]
    fn lyrics_dumped_at_tick_zero_are_a_hard_defect() {
        let (suitability, _) = assess_bytes(&testing::lyrics_all_at_zero());
        assert_eq!(
            suitability.breakdown.sync, 0,
            "untimed lyrics cannot score for synchronization"
        );
        assert!(codes(&suitability).contains(&WarningCode::LyricsAllAtZero));
        assert!(suitability.has_hard_defect());
    }

    #[test]
    fn lyrics_with_no_notes_are_a_hard_defect() {
        let (suitability, _) = assess_bytes(&testing::lyric_events());
        assert_eq!(suitability.breakdown.sync, 0);
        assert!(codes(&suitability).contains(&WarningCode::LyricsWithoutNotes));
        assert!(suitability.has_hard_defect());
    }

    /// An undetectable melody is reported and costs nothing.
    ///
    /// How well a song sings does not depend on whether this tool could pick its tune out — a
    /// well-made arrangement whose guide track is called `Track 3` is not a worse file than the
    /// same arrangement with the track named `MELODY`. The warning stays because the machine really
    /// will not be able to offer the toggle, which is worth knowing before a package is built.
    #[test]
    fn an_undetectable_melody_is_explained_and_costs_nothing() {
        let (suitability, _) = assess_bytes(&testing::ambiguous_melody());
        assert!(codes(&suitability).contains(&WarningCode::NoMelodyChannel));
        assert_eq!(
            suitability.breakdown.channels, 2,
            "the file has its channels apart, whatever the melody detector made of them"
        );
        assert!(
            !codes(&suitability).contains(&WarningCode::SingleChannel),
            "{:?}",
            codes(&suitability)
        );
    }

    /// Everything on one channel is the one channel fault a file cannot be good in spite of.
    #[test]
    fn a_song_with_every_instrument_on_one_channel_is_marked_down() {
        let (suitability, _) = assess_bytes(&testing::soft_karaoke());
        assert_eq!(suitability.breakdown.channels, 0);
        assert!(codes(&suitability).contains(&WarningCode::SingleChannel));

        // And a file with its parts apart keeps the two points, so the warning is about this file
        // rather than about every small arrangement.
        let (spread, _) = assess_bytes(&testing::melody_and_accompaniment());
        assert_eq!(spread.breakdown.channels, 2);
        assert!(!codes(&spread).contains(&WarningCode::SingleChannel));
    }

    #[test]
    fn a_two_channel_sketch_is_marked_down_on_arrangement() {
        let (suitability, _) = assess_bytes(&testing::soft_karaoke());
        assert_eq!(suitability.breakdown.arrangement, 0);
        assert!(codes(&suitability).contains(&WarningCode::FewChannels));
        assert!(codes(&suitability).contains(&WarningCode::NoDrumChannel));
    }

    #[test]
    fn a_short_fixture_is_flagged_as_an_implausible_song_length() {
        let (suitability, song) = assess_bytes(&testing::soft_karaoke());
        assert!(song.duration_ms() < 60_000);
        assert!(codes(&suitability).contains(&WarningCode::ImplausibleDuration));
    }

    /// The file the span exists to catch.
    ///
    /// Same words, same timing, same arrangement and the same melody as `high_quality_song`, and it
    /// is over in forty seconds. Every measurement the rubric made before the span passes it, its
    /// coverage most emphatically: the words run the whole length of the file.
    #[test]
    fn a_whole_song_that_is_over_in_forty_seconds_is_not_worth_choosing() {
        let (good, _) = assess_bytes(&testing::high_quality_song());
        let (short, song) = assess_bytes(&testing::a_complete_short_song());

        assert_eq!(good.value, 10, "the same song at length is excellent");
        let content = LyricContent::measure(&song, song.duration_ms());
        assert!(
            content.coverage > 0.95,
            "the words cover the file: {}",
            content.coverage
        );
        assert!(!content.is_negligible(&Thresholds::default()));
        assert!(!content.is_sparse(&Thresholds::default()));

        assert_eq!(short.breakdown.lyrics, 0);
        assert_eq!(short.breakdown.sync, 0);
        assert_eq!(
            short.breakdown.channels, good.breakdown.channels,
            "the arrangement is the same one, which is why the loss has to come from the words"
        );
        assert!(codes(&short).contains(&WarningCode::BriefSinging));
        assert!(short.has_hard_defect());
        assert!(
            short.value < 5,
            "below the floor a demo draws from: {}",
            short.value
        );
    }

    /// The span and the file's own length are different questions, and this file answers them
    /// differently: two minutes long, forty seconds of it sung.
    #[test]
    fn a_verse_in_a_long_file_is_caught_by_the_span_and_not_by_the_length() {
        let (suitability, song) = assess_bytes(&testing::a_verse_in_a_long_song());
        let thresholds = Thresholds::default();

        assert!(
            thresholds.is_plausible_duration(song.duration_ms()),
            "the file is a plausible length, so the length rule has nothing to say"
        );
        assert!(!codes(&suitability).contains(&WarningCode::ImplausibleDuration));

        let content = LyricContent::measure(&song, song.duration_ms());
        assert!(
            !content.is_negligible(&thresholds),
            "a third of the song is not a business card"
        );
        assert_eq!(suitability.breakdown.lyrics, 0);
        assert_eq!(suitability.breakdown.sync, 0);
        assert!(codes(&suitability).contains(&WarningCode::BriefSinging));
        assert!(
            !codes(&suitability).contains(&WarningCode::PartialLyrics),
            "the words do not stop early; there is only ever forty seconds of them"
        );
    }

    /// The quantity test keeps its say, because the syllable count is the fault worth naming.
    #[test]
    fn a_business_card_is_named_for_its_syllables_rather_than_for_its_length() {
        let (credits, _) = assess_bytes(&testing::credits_in_the_lyric_track());
        let codes = codes(&credits);
        assert!(codes.contains(&WarningCode::NegligibleLyrics), "{codes:?}");
        assert!(!codes.contains(&WarningCode::BriefSinging), "{codes:?}");
    }

    #[test]
    fn a_guessed_encoding_is_reported_but_does_not_change_the_suitability() {
        let bytes = testing::legacy_encoded_lyrics();
        let (guessed, _) = assess_bytes(&bytes);

        let song = Song::parse(&bytes, &ParseOptions::with_encoding("windows-1252"))
            .expect("fixture parses");
        let thresholds = Thresholds::default();
        let channels = channel::measure(&song, &thresholds);
        let melody = melody::detect(&song, &channels, &thresholds);
        let declared = assess(&song, &channels, &melody, &thresholds);

        assert_eq!(
            guessed.value, declared.value,
            "the encoding decision is a data-quality note, not a suitability component"
        );
        assert!(!codes(&declared).contains(&WarningCode::EncodingGuessed));
    }

    #[test]
    fn every_warning_carries_a_usable_message() {
        for (name, build) in testing::FIXTURES {
            let (suitability, _) = assess_bytes(&build());
            for warning in &suitability.warnings {
                assert!(
                    warning.message.len() > 15,
                    "{name}: {:?} has an unhelpful message {:?}",
                    warning.code,
                    warning.message
                );
            }
        }
    }
}
