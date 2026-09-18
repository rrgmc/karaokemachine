//! Packaging-time melody detection and karaoke suitability scoring.
//!
//! Everything here runs **once, when a package is built** -- never during playback. The results go
//! into the package manifest, so the machine reads a recorded fact instead of re-deriving it on
//! every play, and a packager can inspect or override what was decided.
//!
//! Two questions are answered:
//!
//! * *Which channel carries the tune?* [`melody::detect`] answers only when several independent
//!   signals agree, and abstains otherwise. Muting the wrong channel ruins a song, so a false
//!   positive is worse than no detection at all.
//! * *How well does this file work as a karaoke song?* [`suitability::assess`] scores it 0 to 10
//!   with a per-component breakdown and warnings, because a bare number would not be trustworthy.
//!
//! Note that suitability rates **files, not performances**. There is no scoring of
//! singers anywhere in this project.
//!
//! Every tuning constant lives in [`thresholds`].
//!
//! See `docs/ARCHITECTURE.md`.

pub mod channel;
pub mod melody;
pub mod suitability;
pub mod thresholds;

use km_song::Song;
use serde::Serialize;

pub use crate::channel::{ChannelStats, DRUM_CHANNEL};
pub use crate::melody::{Abstention, MelodyChannel, MelodyEvidence, MelodyOutcome, MelodySignal};
pub use crate::suitability::{Breakdown, Suitability, Warning, WarningCode};
pub use crate::thresholds::Thresholds;

/// Which revision of the analysis produced a stored row.
///
/// **A curated corpus keeps conclusions, not the evidence they came from**, so a build that decides
/// something new about a file cannot correct what is already stored — it can only say that what is
/// stored was decided by somebody else. This number is that statement. `km-package-builder` writes it
/// beside every song and re-reads the ones that disagree, which is what turns "read all the songs
/// again" into "read the ones this build would answer differently".
///
/// **Bump it whenever a build would write a different value for the same bytes**, in either crate
/// that decides one: `km-song` for the words, the flavor, the granularity, the counts and the
/// detected title, artist and encoding; `km-suitability` for the melody, the suitability and its
/// warnings. A threshold moved, a heuristic changed, a parser corrected — all the same answer.
///
/// **`the_analysis_revision_covers_what_the_fixtures_say` is what stops it being forgotten**, and it
/// is honest about its own reach: it hashes the analysis of every fixture, so a change no fixture
/// exercises passes it. A new behavior worth storing is worth a fixture.
///
/// **A new fixture moves the digest without a bump.** The fixture's name and analysis join the hash,
/// but no build answers any existing file differently, so the pinned number changes and the revision
/// does not.
///
/// **A bump adds an entry to [`REVISIONS`] saying which rows it can change.**
pub const ANALYSIS_REVISION: u32 = 4;

/// Which stored rows one revision of the analysis can answer differently.
///
/// **A reach may name only a stored fact the revision itself leaves alone.** The row being judged
/// was written by the revision before, so a fact the new revision computes differently says nothing
/// about what it would write. When unsure, the answer is [`Reach::Everything`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Any row may change.
    Everything,
    /// Only a song with at least this many lyric lines, as `km_song`'s `line_count` counts them.
    /// A video, an MP3+G pair and a file with fewer lines are out of reach.
    LyricLinesAtLeast(u32),
    /// Only a song with at least this many syllables, as `km_song`'s `syllable_count` counts them.
    /// A video, an MP3+G pair and a file with fewer syllables are out of reach.
    SyllablesAtLeast(u32),
}

/// One revision of the analysis and the rows it can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revision {
    /// The value of [`ANALYSIS_REVISION`] that introduced it.
    pub number: u32,
    /// The rows written by the revision before that this one can answer differently.
    pub reach: Reach,
}

/// Every revision after the first, oldest first, ending at [`ANALYSIS_REVISION`].
///
/// **This is what turns a bump from "read the corpus again" into "read what the change can reach".**
/// `km-package-builder` moves a row forward through each revision whose reach excludes it, without
/// reading its file, and reads only the rows a revision can reach.
pub const REVISIONS: &[Revision] = &[
    // A melody candidate must sound under the words. With no syllables there are no words to sound
    // under, the gate passes every channel, and detection answers as it did.
    Revision {
        number: 2,
        reach: Reach::SyllablesAtLeast(1),
    },
    // A chord chart needs `Thresholds::min_chord_lines` chord lines, each of them a lyric line.
    Revision {
        number: 3,
        reach: Reach::LyricLinesAtLeast(8),
    },
    // A file that marks its lines and spaces every syllable draws the divider, and `km_song` judges
    // no file below `MIN_JUDGED_SYLLABLES` syllables for it.
    Revision {
        number: 4,
        reach: Reach::SyllablesAtLeast(32),
    },
];

/// Everything the analysis found about one file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Analysis {
    /// The melody channel, or the reason none was claimed.
    pub melody: MelodyOutcome,
    /// How well the file works as a karaoke song.
    pub suitability: Suitability,
    /// Per-channel measurements, kept so a packager can see what the decisions were based on.
    pub channels: Vec<ChannelStats>,
}

impl Analysis {
    /// Analyzes a parsed song with the default thresholds.
    pub fn of(song: &Song) -> Self {
        Self::with_thresholds(song, &Thresholds::default())
    }

    /// Analyzes a parsed song with explicit thresholds.
    pub fn with_thresholds(song: &Song, thresholds: &Thresholds) -> Self {
        let channels = channel::measure(song, thresholds);
        let melody = melody::detect(song, &channels, thresholds);
        let suitability = suitability::assess(song, &channels, &melody, thresholds);
        Self {
            melody,
            suitability,
            channels,
        }
    }

    /// The detected melody channel, if one was claimed.
    pub fn melody_channel(&self) -> Option<u8> {
        self.melody.channel().map(|m| m.channel)
    }

    /// The suitability, 0 to 10.
    ///
    /// Named for the whole quantity rather than shortened to `score`: this project does not score
    /// singers, and a bare `score()` beside `user_score` in the curation tool is exactly the
    /// ambiguity that costs a reader a second look.
    pub fn suitability_value(&self) -> u8 {
        self.suitability.value
    }
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;

    #[test]
    fn analysis_is_consistent_across_every_fixture() {
        for (name, build) in testing::FIXTURES {
            let song = Song::parse(&build(), &ParseOptions::default())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let analysis = Analysis::of(&song);

            assert!(
                analysis.suitability_value() <= 10,
                "{name}: suitability out of range"
            );
            // The channel points and the warning that explains them say one thing between them: a
            // component scoring zero with nothing to account for it is a rating nobody can read.
            assert_eq!(
                analysis.suitability.breakdown.channels == 2,
                !analysis
                    .suitability
                    .warnings
                    .iter()
                    .any(|w| w.code == suitability::WarningCode::SingleChannel),
                "{name}: the channel component and its warning disagree"
            );
            // Drums are never the melody.
            assert_ne!(
                analysis.melody_channel(),
                Some(DRUM_CHANNEL),
                "{name}: the drum channel was claimed as the melody"
            );
        }
    }

    /// What [`ANALYSIS_REVISION`] is for, and what stops it being left behind.
    ///
    /// **A constant somebody has to remember to bump is a constant that does not get bumped**, and
    /// forgetting is silent: a corpus goes on reporting what an older build decided, every scan
    /// skips it because nothing about the files moved, and nothing anywhere says so. This hashes
    /// what the analysis says about every fixture, so a change to the words, the counts, the melody
    /// or the rubric moves the number and fails here with the two lines to edit.
    ///
    /// **It reaches as far as the fixtures do and no further.** A behavior no fixture exercises
    /// passes this and reaches a corpus unannounced; the answer to that is a fixture, which is what
    /// the table is for.
    ///
    /// Hashed rather than compared field by field because the point is *everything* the analysis
    /// says, and a list of fields is one more thing to forget. FNV rather than
    /// [`std::collections::hash_map::DefaultHasher`], whose output is explicitly not stable between
    /// Rust releases — a pinned number has to mean the same thing after a toolchain bump.
    #[test]
    fn the_analysis_revision_covers_what_the_fixtures_say() {
        /// FNV-1a, 64-bit: eight lines, and the same answer on every platform and every compiler.
        fn fold(bytes: &[u8]) -> u64 {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x1000_0000_01b3);
            }
            hash
        }

        // **The tuning constants, as well as what they decided.** A threshold moved by a millisecond
        // changes no fixture — measured: 120 ms to 119 ms leaves every one of them scoring exactly
        // what it scored — and changes thousands of rows in a corpus of hundreds of thousands, which
        // is the case this whole mechanism exists for. Debug rather than a field list, because a
        // field list is the thing somebody forgets to add a field to and the derive is not.
        let mut digest: u64 = fold(format!("{:?}", Thresholds::default()).as_bytes());
        for (name, build) in testing::FIXTURES {
            let song = Song::parse(&build(), &ParseOptions::default())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            // The analysis, and beside it the facts `km-song` decides that a curated corpus stores
            // next to it — so a change to the parser moves this as surely as a change to the rubric.
            let said = format!(
                "{name}|{}|{:?}|{:?}|{}|{}|{}|{}|{}|{:?}|{:?}",
                serde_json::to_string(&Analysis::of(&song)).expect("serializes"),
                song.lyrics.granularity(),
                song.flavor,
                song.lyrics.line_count(),
                song.lyrics.syllable_count(),
                song.lyrics.plain_text(),
                song.note_count(),
                song.duration_ms(),
                song.meta.title,
                song.meta.artist,
            );
            digest ^= fold(said.as_bytes());
        }

        assert_eq!(
            digest, 16_700_313_269_617_445_406,
            "the analysis of the fixtures has changed, so a corpus scanned by an older build no \
             longer agrees with this one. Bump km_suitability::ANALYSIS_REVISION and put the new \
             digest here; a scan then re-reads what that build decided and nothing else."
        );
    }

    /// A revision list that stops short of the constant leaves the newest bump with no reach, and a
    /// gap lets a row skip a revision that could have changed it.
    #[test]
    fn every_revision_after_the_first_states_its_reach() {
        assert_eq!(
            REVISIONS.last().map(|r| r.number),
            Some(ANALYSIS_REVISION),
            "a bump of ANALYSIS_REVISION needs an entry in REVISIONS"
        );
        assert_eq!(REVISIONS.first().map(|r| r.number), Some(2));
        for pair in REVISIONS.windows(2) {
            assert_eq!(pair[1].number, pair[0].number + 1, "{pair:?}");
        }
    }

    /// The melody revision's gate passes every channel of a song with no syllables.
    #[test]
    fn the_melody_revision_reaches_only_songs_with_syllables() {
        let reach = REVISIONS.iter().find(|r| r.number == 2).map(|r| r.reach);
        assert_eq!(reach, Some(Reach::SyllablesAtLeast(1)));
    }

    /// The chord-chart revision reaches exactly as far as the chord-chart threshold does.
    #[test]
    fn the_chord_chart_revision_reaches_the_files_with_enough_lines() {
        let reach = REVISIONS.iter().find(|r| r.number == 3).map(|r| r.reach);
        assert_eq!(
            reach,
            Some(Reach::LyricLinesAtLeast(
                Thresholds::default().min_chord_lines as u32
            ))
        );
    }

    #[test]
    fn analysis_serializes_for_the_package_manifest() {
        let song = Song::parse(&testing::high_quality_song(), &ParseOptions::default())
            .expect("fixture parses");
        let json = serde_json::to_value(Analysis::of(&song)).expect("serializes");

        assert_eq!(json["suitability"]["value"], 10);
        assert_eq!(json["suitability"]["breakdown"]["lyrics"], 3);
        assert_eq!(json["melody"]["channel"], 0);
        assert!(
            json["melody"]["signals"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v == "track_name"))
        );
    }

    #[test]
    fn an_abstention_serializes_as_a_reason_rather_than_a_channel() {
        let song = Song::parse(&testing::ambiguous_melody(), &ParseOptions::default())
            .expect("fixture parses");
        let json = serde_json::to_value(Analysis::of(&song)).expect("serializes");
        assert_eq!(json["melody"]["abstained"], "ambiguous");
        assert!(json["melody"]["channel"].is_null());
    }
}
