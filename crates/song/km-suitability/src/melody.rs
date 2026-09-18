//! Finding the melody channel, or declining to.
//!
//! A guide-melody toggle is only useful if it mutes the right channel. Muting the wrong one ruins
//! the song, which makes a false positive strictly worse than no detection at all. So this module
//! is built to **abstain**: several independent signals must agree, and the winner must beat the
//! runner-up clearly, or nothing is claimed and the feature stays hidden for that song.
//!
//! It runs at packaging time only. The result is written into the package, so the machine reads a
//! recorded fact instead of guessing on every playback, and a packager can inspect or override it.

use km_song::Song;
use serde::Serialize;

use crate::channel::{ChannelStats, DRUM_CHANNEL};
use crate::thresholds::Thresholds;

/// A signal that supported a channel being the melody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MelodySignal {
    /// A track putting notes on this channel is named for the melody or the voice.
    TrackName,
    /// Note onsets line up with the lyric syllables.
    LyricAlignment,
    /// The channel plays one note at a time.
    Monophonic,
    /// The notes sit in a range a person could sing.
    VocalRange,
    /// Channel 4 (1-based), the common karaoke convention.
    ConventionalChannel,
}

/// A confidently detected melody channel.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MelodyChannel {
    /// The channel, 0-based.
    pub channel: u8,
    /// How strongly the evidence favored it, 0.0 to 1.0.
    pub confidence: f32,
    /// Which signals fired, so a borderline call can be audited.
    pub signals: Vec<MelodySignal>,
    /// Fraction of syllables with a note onset on this channel.
    pub lyric_alignment: f32,
    /// Fraction of sounding time with at most one note.
    pub monophony: f32,
}

/// How strongly one channel's own evidence favors it being the melody.
///
/// **Evidence is not [`MelodyChannel::confidence`], and the two must not be read as the same
/// number.** Confidence folds in how far the winner stands clear of the field, which is a property
/// of the song and not of the channel; evidence is the channel's own signals alone, so every channel
/// has one and they can be put in a column beside each other.
///
/// A channel the gates rule out still carries its evidence, with [`Self::eligible`] saying so: a
/// person looking at a song that abstained is asking which channel came closest, and a blank row
/// does not answer that.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MelodyEvidence {
    /// The channel, 0-based.
    pub channel: u8,
    /// The strength of this channel's own signals, 0.0 to 1.0.
    pub evidence: f32,
    /// Which signals fired.
    pub signals: Vec<MelodySignal>,
    /// Fraction of syllables with a note onset on this channel.
    pub lyric_alignment: f32,
    /// Fraction of sounding time with at most one note.
    pub monophony: f32,
    /// Whether the channel passed the gates [`detect`] applies before it weighs anything: one note
    /// at a time, inside singing range, sounding while the words are sung, and something beyond
    /// monophony tying it to the singing.
    pub eligible: bool,
}

/// Why no melody channel was claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Abstention {
    /// The song has no notes on any non-drum channel.
    NoCandidates,
    /// No channel plays one note at a time.
    NothingMonophonic,
    /// The monophonic channels are all outside singing range -- a bass or a bell part, not a tune.
    OutsideVocalRange,
    /// The channels left are silent while the words are sung -- an intro riff or a fill between
    /// verses, whatever its track is called.
    SilentUnderTheWords,
    /// A monophonic channel exists, but nothing ties it to the singing.
    NoSupportingEvidence,
    /// Two or more channels are equally plausible, so picking one would be a guess.
    Ambiguous,
}

/// The outcome of melody detection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MelodyOutcome {
    /// A channel was identified.
    Found(MelodyChannel),
    /// Nothing was claimed, with the reason.
    Abstained {
        /// Why detection declined.
        abstained: Abstention,
    },
}

impl MelodyOutcome {
    /// The detected channel, if any.
    pub fn channel(&self) -> Option<&MelodyChannel> {
        match self {
            Self::Found(melody) => Some(melody),
            Self::Abstained { .. } => None,
        }
    }

    /// Whether a channel was claimed.
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Found(_))
    }
}

/// Track names that indicate the melody or the sung line, across the languages the corpus uses.
///
/// `tema` earns its place from real files: Brazilian sequencers routinely name the guide-melody
/// track "Tema", and without it the detector abstained on songs whose melody was clearly labeled.
const MELODY_NAMES: [&str; 15] = [
    "melody",
    "melodia",
    "melodie",
    "vocal",
    "voice",
    "voz",
    "lead",
    "sing",
    "guide",
    "tune",
    "canto",
    "cantor",
    "gesang",
    "tema",
    "principal",
];

/// A candidate channel with its score and evidence.
struct Candidate {
    channel: u8,
    score: f32,
    signals: Vec<MelodySignal>,
    lyric_alignment: f32,
    monophony: f32,
}

/// Attempts to identify the melody channel.
pub fn detect(song: &Song, channels: &[ChannelStats], thresholds: &Thresholds) -> MelodyOutcome {
    let eligible: Vec<&ChannelStats> = channels
        .iter()
        .filter(|stats| stats.channel != DRUM_CHANNEL && stats.note_count > 0)
        .collect();

    if eligible.is_empty() {
        return MelodyOutcome::Abstained {
            abstained: Abstention::NoCandidates,
        };
    }

    // Syllable positions in wall-clock time. The alignment window is a perceptual quantity, so it
    // has to be compared in milliseconds rather than ticks, which vary with tempo.
    let syllable_ms: Vec<u32> = song
        .lyrics
        .syllable_ticks()
        .into_iter()
        .map(|tick| song.tempo_map.tick_to_ms(tick))
        .collect();

    let monophonic: Vec<&ChannelStats> = eligible
        .iter()
        .copied()
        .filter(|stats| stats.monophony >= thresholds.melody_min_monophony)
        .collect();

    if monophonic.is_empty() {
        return MelodyOutcome::Abstained {
            abstained: Abstention::NothingMonophonic,
        };
    }

    // Singable range is a gate, not a bonus. A bass line is monophonic, and its notes land on most
    // syllables because it follows the rhythm, so alignment alone cannot separate it from the tune.
    // Register can: nobody sings a line whose notes sit two octaves below the voice.
    let singable: Vec<&ChannelStats> = monophonic
        .iter()
        .copied()
        .filter(|stats| thresholds.is_vocal_range(stats.median_key, stats.vocal_key_fraction))
        .collect();

    if singable.is_empty() {
        return MelodyOutcome::Abstained {
            abstained: Abstention::OutsideVocalRange,
        };
    }

    // A sung line plays while the words are sung. This is the gate a track name cannot pass on its
    // own: a part called `Melody` that sounds only in the intro and between the verses is not the
    // line being sung, and a toggle muting it would leave the singer the same song with a riff
    // missing.
    let present: Vec<&ChannelStats> = singable
        .iter()
        .copied()
        .filter(|stats| plays_under_the_words(song, stats, &syllable_ms, thresholds))
        .collect();

    if present.is_empty() {
        return MelodyOutcome::Abstained {
            abstained: Abstention::SilentUnderTheWords,
        };
    }

    let mut candidates: Vec<Candidate> = present
        .iter()
        .map(|stats| score(song, stats, &syllable_ms, thresholds))
        .collect();

    // Anything with no supporting evidence beyond being monophonic is not a candidate. A bass line
    // is monophonic too.
    candidates.retain(|candidate| {
        candidate.lyric_alignment >= thresholds.melody_min_lyric_alignment
            || candidate.signals.contains(&MelodySignal::TrackName)
    });

    if candidates.is_empty() {
        return MelodyOutcome::Abstained {
            abstained: Abstention::NoSupportingEvidence,
        };
    }

    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = &candidates[0];

    if let Some(runner_up) = candidates.get(1) {
        // A clear margin is required. Two plausible channels mean the file does not tell us which
        // one carries the tune, and guessing is what this module exists to avoid.
        let beaten =
            runner_up.score <= 0.0 || best.score >= runner_up.score * thresholds.melody_margin;
        if !beaten {
            return MelodyOutcome::Abstained {
                abstained: Abstention::Ambiguous,
            };
        }
    }

    MelodyOutcome::Found(MelodyChannel {
        channel: best.channel,
        confidence: confidence(best, &candidates),
        signals: best.signals.clone(),
        lyric_alignment: best.lyric_alignment,
        monophony: best.monophony,
    })
}

/// Every non-drum channel that sounds, weighed as a melody candidate, strongest first.
///
/// **What [`detect`] weighed, without the choosing.** Detection answers one channel or none, which
/// is what a machine needs and what a person auditing a song does not: a song that abstained as
/// ambiguous has two channels a curator may want to tell apart, and one that found nothing has a
/// runner-up worth seeing. This reports the field.
///
/// The gates are reported rather than applied, so the ordering is by evidence alone and a channel
/// that failed one still appears.
pub fn rank(
    song: &Song,
    channels: &[ChannelStats],
    thresholds: &Thresholds,
) -> Vec<MelodyEvidence> {
    let syllable_ms: Vec<u32> = song
        .lyrics
        .syllable_ticks()
        .into_iter()
        .map(|tick| song.tempo_map.tick_to_ms(tick))
        .collect();

    let mut ranked: Vec<MelodyEvidence> = channels
        .iter()
        .filter(|stats| stats.channel != DRUM_CHANNEL && stats.note_count > 0)
        .map(|stats| {
            let candidate = score(song, stats, &syllable_ms, thresholds);
            let gated = stats.monophony >= thresholds.melody_min_monophony
                && thresholds.is_vocal_range(stats.median_key, stats.vocal_key_fraction)
                && plays_under_the_words(song, stats, &syllable_ms, thresholds)
                && (candidate.lyric_alignment >= thresholds.melody_min_lyric_alignment
                    || candidate.signals.contains(&MelodySignal::TrackName));
            MelodyEvidence {
                channel: candidate.channel,
                evidence: evidence_of(candidate.score),
                signals: candidate.signals,
                lyric_alignment: candidate.lyric_alignment,
                monophony: candidate.monophony,
                eligible: gated,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.evidence
            .total_cmp(&a.evidence)
            .then(a.channel.cmp(&b.channel))
    });
    ranked
}

/// Weighs one channel's evidence.
///
/// The weights are relative, not absolute: only their ordering and the margin between candidates
/// matter. Lyric alignment dominates because it is the only signal that actually ties a channel to
/// the singing; the channel-number convention is a tiebreaker and never enough on its own.
fn score(
    song: &Song,
    stats: &ChannelStats,
    syllable_ms: &[u32],
    thresholds: &Thresholds,
) -> Candidate {
    let mut signals = Vec::new();
    let mut score = 0.0f32;

    let alignment = lyric_alignment(song, stats, syllable_ms, thresholds);
    if alignment > 0.0 {
        score += 4.0 * alignment;
        if alignment >= thresholds.melody_min_lyric_alignment {
            signals.push(MelodySignal::LyricAlignment);
        }
    }

    if stats.track_names.iter().any(|name| is_melody_name(name)) {
        score += 3.0;
        signals.push(MelodySignal::TrackName);
    }

    score += stats.monophony;
    signals.push(MelodySignal::Monophonic);

    // Required to be a candidate at all, so it carries no weight here; reported so the record
    // shows it was checked.
    signals.push(MelodySignal::VocalRange);

    if stats.channel == thresholds.conventional_melody_channel {
        score += 0.5;
        signals.push(MelodySignal::ConventionalChannel);
    }

    Candidate {
        channel: stats.channel,
        score,
        signals,
        lyric_alignment: alignment,
        monophony: stats.monophony,
    }
}

/// Fraction of syllables that have a note onset on this channel close enough to be the sung note.
fn lyric_alignment(
    song: &Song,
    stats: &ChannelStats,
    syllable_ms: &[u32],
    thresholds: &Thresholds,
) -> f32 {
    syllables_within(song, stats, syllable_ms, thresholds.note_align_window_ms)
}

/// Whether a channel sounds while the words are sung, which a melody must.
///
/// A song with no syllables has no words to play under, so the question does not arise and the
/// channel passes.
fn plays_under_the_words(
    song: &Song,
    stats: &ChannelStats,
    syllable_ms: &[u32],
    thresholds: &Thresholds,
) -> bool {
    syllable_ms.is_empty()
        || syllables_within(
            song,
            stats,
            syllable_ms,
            thresholds.melody_presence_window_ms,
        ) >= thresholds.melody_min_lyric_presence
}

/// Fraction of syllables with a note onset on this channel within `window` milliseconds.
fn syllables_within(song: &Song, stats: &ChannelStats, syllable_ms: &[u32], window: u32) -> f32 {
    if syllable_ms.is_empty() || stats.onset_ticks.is_empty() {
        return 0.0;
    }
    let onsets_ms: Vec<u32> = stats
        .onset_ticks
        .iter()
        .map(|&tick| song.tempo_map.tick_to_ms(tick))
        .collect();

    let matched = syllable_ms
        .iter()
        .filter(|&&target| nearest_distance(&onsets_ms, target) <= window)
        .count();
    matched as f32 / syllable_ms.len() as f32
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

fn is_melody_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    MELODY_NAMES
        .iter()
        .any(|candidate| lower.contains(candidate))
}

/// How much to trust the winner: how far clear of the field it is, tempered by its own evidence.
fn confidence(best: &Candidate, all: &[Candidate]) -> f32 {
    let runner_up = all.get(1).map_or(0.0, |c| c.score);
    let separation = if best.score > 0.0 {
        (best.score - runner_up) / best.score
    } else {
        0.0
    };
    // Half from standing clear of the alternatives, half from the strength of its own evidence.
    ((separation * 0.5) + (evidence_of(best.score) * 0.5)).clamp(0.0, 1.0)
}

/// A raw score as a fraction of the strongest case a channel can make for itself.
///
/// 8.0 is the practical maximum: full lyric alignment (4.0), a name match (3.0), monophony (1.0).
/// The conventional-channel bonus can carry it past that, which is what the clamp is for.
fn evidence_of(score: f32) -> f32 {
    (score / 8.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;
    use crate::channel;

    fn analyze(bytes: &[u8]) -> MelodyOutcome {
        let thresholds = Thresholds::default();
        let song = Song::parse(bytes, &ParseOptions::default()).expect("fixture parses");
        let channels = channel::measure(&song, &thresholds);
        detect(&song, &channels, &thresholds)
    }

    fn ranked(bytes: &[u8]) -> Vec<MelodyEvidence> {
        let thresholds = Thresholds::default();
        let song = Song::parse(bytes, &ParseOptions::default()).expect("fixture parses");
        let channels = channel::measure(&song, &thresholds);
        rank(&song, &channels, &thresholds)
    }

    #[test]
    fn the_ranking_leads_with_the_channel_detection_chose() {
        let found = analyze(&testing::melody_and_accompaniment())
            .channel()
            .expect("the melody channel should be found")
            .channel;
        let ranked = ranked(&testing::melody_and_accompaniment());
        assert_eq!(ranked.first().expect("a candidate").channel, found);
        assert!(ranked.first().expect("a candidate").eligible);
        assert!(
            ranked
                .windows(2)
                .all(|pair| pair[0].evidence >= pair[1].evidence),
            "strongest first: {ranked:?}"
        );
    }

    #[test]
    fn a_channel_the_gates_rule_out_is_ranked_and_marked() {
        // The bass line above. Detection has nothing to say about it, and somebody asking why this
        // song abstained is asking exactly what it scored.
        let ranked = ranked(&testing::soft_karaoke_real_layout());
        assert!(!ranked.is_empty());
        assert!(ranked.iter().any(|row| !row.eligible));
        assert!(ranked.iter().all(|row| row.channel != DRUM_CHANNEL));
    }

    #[test]
    fn a_named_monophonic_channel_aligned_with_the_lyrics_is_found() {
        let outcome = analyze(&testing::melody_and_accompaniment());
        let melody = outcome
            .channel()
            .expect("the melody channel should be found");
        assert_eq!(melody.channel, 0);
        assert!(melody.signals.contains(&MelodySignal::TrackName));
        assert!(melody.signals.contains(&MelodySignal::LyricAlignment));
        assert!(melody.signals.contains(&MelodySignal::Monophonic));
        assert!(melody.lyric_alignment > 0.9);
        assert!(
            melody.confidence > 0.6,
            "confidence was {}",
            melody.confidence
        );
    }

    #[test]
    fn a_bass_line_is_not_mistaken_for_a_melody() {
        // A monophonic bass part. This is the case that matters most: it was picked up as a
        // candidate by alignment alone until singable range became a gate.
        let outcome = analyze(&testing::soft_karaoke_real_layout());
        assert_eq!(
            outcome,
            MelodyOutcome::Abstained {
                abstained: Abstention::OutsideVocalRange
            },
            "got {outcome:?}"
        );
    }

    #[test]
    fn a_melody_is_chosen_over_an_equally_aligned_bass_line() {
        // Both channels are monophonic and hit every syllable. Register is the only thing that
        // separates them, which is why it is a gate. Regression for a real corpus file.
        let outcome = analyze(&testing::melody_with_monophonic_bass());
        let melody = outcome
            .channel()
            .expect("the melody should be found, not the bass");
        assert_eq!(melody.channel, 4, "channel 1 is the bass line");
        assert!(
            melody.signals.contains(&MelodySignal::TrackName),
            "\"Tema\" should register as a melody track name"
        );
    }

    #[test]
    fn a_whole_song_finds_its_guide_track_and_not_its_bass() {
        // The same contest as above over thirty-two lines rather than one, on a mid channel: the
        // guide is `Tema` on channel 5, and channel 1 is a bass playing the same rhythm two octaves
        // down. Length matters here — alignment and monophony are fractions, and a fraction over
        // eight notes says much less than one over two hundred and fifty-six.
        let outcome = analyze(&testing::soft_karaoke_header_on_words_track());
        let melody = outcome
            .channel()
            .expect("the melody should be found, not the bass");
        assert_eq!(melody.channel, 5, "channel 1 is the bass line");
        assert!(melody.lyric_alignment > 0.9);
    }

    #[test]
    fn a_melody_on_the_top_channel_is_found() {
        // A channel is a four-bit field. Code that indexes an array of ten, or reads a channel as a
        // decimal digit, is wrong only at the top of the range — so nothing may assume a melody
        // lives on a low one.
        let outcome = analyze(&testing::melody_on_channel_fifteen());
        let melody = outcome.channel().expect("channel 15 is still a channel");
        assert_eq!(melody.channel, 15);
    }

    #[test]
    fn a_channel_named_melody_that_rests_under_the_words_is_not_the_melody() {
        let bytes = testing::named_melody_silent_under_the_words();
        assert_eq!(
            analyze(&bytes),
            MelodyOutcome::Abstained {
                abstained: Abstention::SilentUnderTheWords
            }
        );

        // The table shows the riff with its name, and says why it was not chosen.
        let riff = ranked(&bytes)
            .into_iter()
            .find(|row| row.channel == 0)
            .expect("the riff is ranked");
        assert!(riff.signals.contains(&MelodySignal::TrackName));
        assert!(!riff.eligible);
    }

    /// The case the name-only path exists for, and what the presence gate must not take from it.
    #[test]
    fn a_named_melody_the_words_miss_by_half_a_beat_is_still_found() {
        let outcome = analyze(&testing::lyrics_against_another_arrangement());
        let melody = outcome
            .channel()
            .expect("the melody plays under every word");
        assert_eq!(melody.channel, 0);
        assert!(!melody.signals.contains(&MelodySignal::LyricAlignment));
    }

    #[test]
    fn an_instrumental_with_no_lyrics_abstains() {
        let outcome = analyze(&testing::instrumental());
        assert!(!outcome.is_found());
    }

    #[test]
    fn a_song_with_no_notes_abstains_with_no_candidates() {
        let outcome = analyze(&testing::lyric_events());
        assert_eq!(
            outcome,
            MelodyOutcome::Abstained {
                abstained: Abstention::NoCandidates
            }
        );
    }

    #[test]
    fn two_equally_plausible_channels_abstain_as_ambiguous() {
        let outcome = analyze(&testing::ambiguous_melody());
        assert_eq!(
            outcome,
            MelodyOutcome::Abstained {
                abstained: Abstention::Ambiguous
            },
            "with two identical candidates the file does not say which carries the tune"
        );
    }

    #[test]
    fn a_chords_only_song_abstains_because_nothing_is_monophonic() {
        let outcome = analyze(&testing::chords_only());
        assert_eq!(
            outcome,
            MelodyOutcome::Abstained {
                abstained: Abstention::NothingMonophonic
            }
        );
    }

    #[test]
    fn the_drum_channel_is_never_a_candidate() {
        let outcome = analyze(&testing::drums_and_lyrics());
        assert!(
            !outcome.is_found(),
            "drums are monophonic and on the beat, but note numbers there are instruments"
        );
    }

    #[test]
    fn nearest_distance_handles_the_edges() {
        let sorted = [10u32, 20, 30];
        assert_eq!(nearest_distance(&sorted, 20), 0);
        assert_eq!(nearest_distance(&sorted, 0), 10);
        assert_eq!(nearest_distance(&sorted, 40), 10);
        assert_eq!(nearest_distance(&sorted, 16), 4);
        assert_eq!(nearest_distance(&[], 5), u32::MAX);
    }

    #[test]
    fn melody_name_matching_is_case_and_language_tolerant() {
        assert!(is_melody_name("Melody"));
        assert!(is_melody_name("  MELODIA  "));
        assert!(is_melody_name("Lead Vocal"));
        assert!(is_melody_name("voz principal"));
        assert!(!is_melody_name("Baixo eletrico"));
        assert!(!is_melody_name("Bateria"));
        assert!(!is_melody_name(""));
    }
}
