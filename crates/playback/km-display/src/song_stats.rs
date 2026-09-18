//! What was done to the loaded song, in the shape the screen wants it.
//!
//! **The deciding lives in `km-app` and only the reporting is here**, the seam
//! [`crate::performance`] sits on: this crate is handed a gain somebody else worked out and a count
//! of corrections somebody else resolved, and knows nothing about SoundFonts, catalogs or the
//! loudness of a package. What it owns is which of them are worth a person's attention, which is
//! [`SongStats::damaged`], [`SongStats::fixed`] and [`SongStats::gain_is_steep`].
//!
//! It exists because these answers were in the wrong room. *Why is this one so quiet* and *why does
//! that channel sound wrong* are asked in front of the television, and the answer was a `debug!`
//! line in a log on the box under it -- reachable over ssh, from somewhere else, after the song had
//! ended.

use km_song::{Dialect, KaraokeFlavor};

/// Which of the three kinds of song is loaded.
///
/// **Worded in this crate rather than resolved by the caller**, the call [`crate::DeveloperMode`]
/// makes and for its reason: a fixed set of states carries no value the caller holds, so it is
/// written in the locale the rest of the screen is already drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SongMedia {
    /// A MIDI or karaoke file.
    Midi,
    /// A video file.
    Video,
    /// An MP3 and a CD+G graphics file.
    Cdg,
    /// An MP3 and the lyric timeline read from an UltraStar file.
    UltraStar,
}

/// Where the gain in force came from.
///
/// **Four cases, and telling them apart is the whole point of drawing this**: a gain of `1.00` means
/// something different in each, and nothing else on the machine separates them. A song already at
/// the reference, a song nobody asked to level, and a song nothing could measure all sound the same
/// and want different answers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GainSource {
    /// A media song, brought down to the level the bank renders MIDI at, from the loudness its
    /// package measured.
    Package {
        /// What the package measured, in LUFS.
        lufs: f32,
    },
    /// A MIDI song, moved either way to that same level, from a reading of its own events.
    Events {
        /// The event-based estimate, in that estimate's own decibels.
        db: f32,
    },
    /// Levelling is switched off for this kind of song.
    Disabled,
    /// Nothing to level with: a song too short or too quiet to estimate, a media song in a package
    /// built before levelling, or a machine with no bank to level against.
    Unmeasured,
}

/// What the machine did to the song it is playing.
///
/// `Copy`, so [`crate::draw::Frame`] holds it by value rather than by reference, for the reason
/// [`crate::FrameStats`] is: it is rebuilt once a frame and read once a frame, and a borrow would
/// force the caller to keep the machine's state lock alive across the frame it is drawing.
///
/// **Every field a video or MP3+G song cannot fill in is absent or zero**, rather than defaulted to
/// something that reads as a measurement. There are no channels to correct in a video, no tracks and
/// no karaoke convention, so the panel draws three rows for one and five for a MIDI song.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SongStats {
    /// Which of the three kinds this is.
    pub kind: SongMedia,
    /// The factor the audio thread is applying, as the machine worked it out when the song started.
    pub gain: f32,
    /// Which of the four derivations produced it.
    pub gain_source: GainSource,
    /// Channels whose Bank Select is dropped by a stored correction.
    pub bank_ignored: u8,
    /// Channels silenced by a stored correction.
    ///
    /// The guide-melody mute is **not** counted here. That one is a button somebody is pressing, it
    /// is already drawn as a badge, and folding it in would make a control look like a defect in the
    /// file.
    pub muted: u8,
    /// Which convention the words were found in, for a MIDI song.
    pub flavor: Option<KaraokeFlavor>,
    /// What the file's own habits said about how it writes lyrics, for a MIDI song.
    pub dialect: Option<Dialect>,
    /// Tracks in the source file.
    pub tracks: usize,
    /// Tracks the parser stopped reading early. Everything past the bad byte is gone.
    pub truncated_tracks: usize,
    /// Tracks the header declared that never parsed as a chunk.
    pub missing_tracks: usize,
    /// Note-offs synthesized for notes left sounding when their track's data ran out.
    pub repaired_notes: usize,
}

impl SongStats {
    /// How far a song may be moved before the panel says so, in decibels either way.
    ///
    /// Six is a halving or a doubling of amplitude, which is the point at which the levelling stops
    /// being a correction and becomes the thing somebody is hearing. The bound is here rather than
    /// read from the machine's own clamps for the reason `FrameStats::TIGHT` is here: what counts as
    /// worth colouring is a question about a screen, and a crate whose whole job is to put pixels on
    /// one should not link a loudness meter to answer it.
    const STEEP_DB: f32 = 6.0;

    /// Whether anything about this song is worth colouring its heading.
    ///
    /// Two conditions that fail in opposite directions, the shape
    /// [`strained`](crate::FrameStats::strained) has: a file the parser could not finish reading,
    /// and a file read perfectly whose level had to be hauled a long way. Neither sees the other,
    /// and a song can arrive in either state alone.
    pub fn worth_attention(&self) -> bool {
        self.damaged() || self.gain_is_steep()
    }

    /// Whether the file did not read to the end.
    ///
    /// **A repaired note on its own is not damage**, which is the same cut `warn_if_damaged` makes
    /// in the log: a note-off synthesized in an otherwise well-formed file is the parser doing its
    /// job on one file in thirty-four, and a panel that called that damage would be red on songs
    /// with nothing wrong with them. It is drawn as detail beside a truncation and never alone.
    ///
    /// So the panel and the log cannot disagree about whether a file is damaged, which is the song
    /// half of the property the frame block and `--frame-stats` already have.
    pub fn damaged(&self) -> bool {
        self.truncated_tracks > 0 || self.missing_tracks > 0
    }

    /// Whether any stored correction is in force on this song's events.
    ///
    /// Only the two kinds a stored fix can set. The table the machine resolves carries two more
    /// columns that nothing writes, so counting those would be counting fields that cannot move.
    pub fn fixed(&self) -> bool {
        self.bank_ignored > 0 || self.muted > 0
    }

    /// Whether the levelling moved this song far enough to be what somebody is hearing.
    ///
    /// Either direction: a song hauled up is as much the answer to *why does this sound wrong* as
    /// one pushed down.
    pub fn gain_is_steep(&self) -> bool {
        self.gain_db().abs() >= Self::STEEP_DB
    }

    /// How many rows this block takes, so the panel's height can be bounded by a test.
    ///
    /// **Here rather than in the drawing**, for the reason every threshold on this struct is: a
    /// count worked out inside `draw_performance` is a count no test can reach without a font and a
    /// screen, and the one assertion this layout rests on is how far down the panel reaches at its
    /// tallest.
    ///
    /// A MIDI song takes the heading, the position, the gain, its derivation, the corrections and
    /// the lyric convention, and one more when the file is damaged. A video or MP3+G song has no
    /// channels to correct, no tracks and no karaoke convention, so it takes four.
    pub fn rows(&self) -> u32 {
        match self.kind {
            SongMedia::Midi => 6 + u32::from(self.damaged()),
            SongMedia::Video | SongMedia::Cdg | SongMedia::UltraStar => 4,
        }
    }

    /// The gain as decibels, which is what a person reasons in.
    ///
    /// A gain of zero cannot arrive, since the machine clamps well above it, but it is
    /// guarded anyway, because a logarithm of zero on a screen reads as a fault in the machine.
    pub fn gain_db(&self) -> f32 {
        if self.gain > 0.0 {
            20.0 * self.gain.log10()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi() -> SongStats {
        SongStats {
            kind: SongMedia::Midi,
            gain: 1.0,
            gain_source: GainSource::Unmeasured,
            bank_ignored: 0,
            muted: 0,
            flavor: Some(KaraokeFlavor::SoftKaraoke),
            dialect: Some(Dialect::default()),
            tracks: 9,
            truncated_tracks: 0,
            missing_tracks: 0,
            repaired_notes: 0,
        }
    }

    #[test]
    fn a_well_formed_song_reports_neither_damage_nor_corrections() {
        let stats = midi();
        assert!(!stats.damaged());
        assert!(!stats.fixed());
        assert!(!stats.gain_is_steep());
    }

    #[test]
    fn either_kind_of_lost_track_is_damage() {
        for stats in [
            SongStats {
                truncated_tracks: 1,
                ..midi()
            },
            SongStats {
                missing_tracks: 1,
                ..midi()
            },
        ] {
            assert!(stats.damaged());
        }
    }

    #[test]
    fn a_repaired_note_on_its_own_is_not_damage() {
        // The cut `warn_if_damaged` makes in the log, held to here so the two cannot disagree. One
        // file in thirty-four carries one of these and plays perfectly.
        let stats = SongStats {
            repaired_notes: 47,
            ..midi()
        };
        assert!(!stats.damaged());
    }

    #[test]
    fn either_kind_of_correction_counts() {
        assert!(SongStats { muted: 1, ..midi() }.fixed());
        assert!(
            SongStats {
                bank_ignored: 1,
                ..midi()
            }
            .fixed()
        );
    }

    #[test]
    fn a_steep_move_in_either_direction_is_worth_colouring() {
        // Half and double, which is where the threshold sits.
        assert!(
            SongStats {
                gain: 0.5,
                ..midi()
            }
            .gain_is_steep()
        );
        assert!(
            SongStats {
                gain: 2.0,
                ..midi()
            }
            .gain_is_steep()
        );
    }

    #[test]
    fn an_ordinary_correction_is_not_a_complaint() {
        // About 2.5 dB down -- the machine doing its job, not the thing to look at.
        let stats = SongStats {
            gain: 0.75,
            ..midi()
        };
        assert!(!stats.gain_is_steep());
    }

    #[test]
    fn unity_gain_is_zero_decibels() {
        assert!(midi().gain_db().abs() < 0.001);
    }

    #[test]
    fn half_the_amplitude_is_six_decibels_down() {
        let stats = SongStats {
            gain: 0.5,
            ..midi()
        };
        assert!((stats.gain_db() + 6.02).abs() < 0.01);
    }

    #[test]
    fn a_gain_of_zero_draws_a_number_rather_than_an_infinity() {
        // Unreachable through the machine, which clamps, and guarded because a screen is the worst
        // place to discover a logarithm of zero.
        let stats = SongStats {
            gain: 0.0,
            ..midi()
        };
        assert!(stats.gain_db().is_finite());
    }

    #[test]
    fn a_media_song_fills_in_nothing_it_cannot_know() {
        let stats = SongStats {
            kind: SongMedia::Video,
            gain: 0.71,
            gain_source: GainSource::Package { lufs: -18.2 },
            flavor: None,
            dialect: None,
            tracks: 0,
            ..midi()
        };
        assert!(!stats.damaged());
        assert!(!stats.fixed());
        assert_eq!(stats.rows(), 4);
    }

    #[test]
    fn a_midi_song_costs_one_row_more_when_the_file_is_damaged() {
        assert_eq!(midi().rows(), 6);
        assert_eq!(
            SongStats {
                truncated_tracks: 1,
                ..midi()
            }
            .rows(),
            7
        );
    }
}
