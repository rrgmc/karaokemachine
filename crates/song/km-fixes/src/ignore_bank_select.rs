//! Dropping a Bank Select that names a bank of drum kits on a channel meant to carry an instrument.
//!
//! **The defect is a file written for a synthesizer module that is not the one playing it.** A setup
//! track sends Bank Select MSB 126 or 127 — the XG effect and drum banks — on a melodic channel, and
//! then a program change. The file's own track name and program say the channel is an organ or a
//! pad, and the notes on it are sustained chords.
//!
//! **What that sounds like depends entirely on the bank**, which is why nothing measured at
//! packaging time can hear it. A SoundFont with no bank 126 or 127 falls back to bank 0 and plays
//! some instrument, wrong but harmless. A SoundFont that has one succeeds: the channel stops being
//! an instrument and becomes a drum kit, and every chord note lands on whatever percussion sits at
//! that key — agogos, maracas, bells, at the velocity the chord was written with.
//!
//! **This fix applies itself**, because dropping the bank select makes every bank agree. The
//! fallback a font without those banks already performs becomes what all of them do, and the program
//! change the file sends anyway is left to select from the default bank.
//!
//! Both halves of the message go. A coarse select left behind with its fine partner suppressed still
//! names a bank, and the pair is one message written as two.

use km_song::{EventKind, Song};

use crate::{CHANNELS, DRUM_CHANNEL, Fix};

/// Bank Select, coarse.
pub const CC_BANK_SELECT_MSB: u8 = 0;

/// Bank Select, fine.
pub const CC_BANK_SELECT_LSB: u8 = 32;

/// The lowest bank number that holds kits rather than instruments.
///
/// 127 is XG's drum bank and 126 its effect bank, and neither holds anything a melodic channel can
/// have meant to select. A bank below this is an ordinary variation and is left alone: a file asking
/// for bank 8 is asking for a different piano, and on a font without one the fallback is already
/// correct.
pub const FIRST_KIT_BANK: u8 = 126;

/// Every channel carrying a kit bank select that a note then plays through.
///
/// A channel with no notes is passed over. The bank select on it changes nothing anybody can hear,
/// and a fix recorded against it would be a line in the manifest and a line in the log for a channel
/// that never sounds.
pub fn detect(song: &Song) -> Vec<Fix> {
    let mut flagged = [false; CHANNELS];
    let mut sounds = [false; CHANNELS];
    for event in &song.events {
        match event.kind {
            EventKind::Controller {
                channel,
                controller: CC_BANK_SELECT_MSB,
                value,
            } if channel != DRUM_CHANNEL && value >= FIRST_KIT_BANK => {
                flagged[usize::from(channel)] = true;
            }
            EventKind::NoteOn {
                channel, velocity, ..
            } if velocity > 0 => {
                sounds[usize::from(channel)] = true;
            }
            _ => {}
        }
    }
    (0..CHANNELS)
        .filter(|channel| flagged[*channel] && sounds[*channel])
        .map(|channel| Fix::IgnoreBankSelect {
            channel: channel as u8,
        })
        .collect()
}

/// The log line this fix writes when a song starts.
pub fn describe(channel: u8) -> String {
    format!("channel {channel}: bank select ignored, so the program selects from the default bank")
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;

    fn song(bytes: &[u8]) -> Song {
        Song::parse(bytes, &ParseOptions::default()).expect("fixture parses")
    }

    #[test]
    fn a_kit_bank_on_a_melodic_channel_is_flagged() {
        let song = song(&testing::kit_bank_on_a_melodic_channel());
        assert_eq!(detect(&song), vec![Fix::IgnoreBankSelect { channel: 4 }]);
    }

    #[test]
    fn the_drum_channel_is_left_alone() {
        // The one channel where a kit bank is what the file means.
        let song = song(&testing::drum_key_tune());
        assert!(detect(&song).is_empty());
    }

    #[test]
    fn an_ordinary_variation_bank_is_left_alone() {
        // Channel 2 of the fixture selects bank 8, which is a different piano and not a defect.
        let song = song(&testing::kit_bank_on_a_melodic_channel());
        assert!(!detect(&song).contains(&Fix::IgnoreBankSelect { channel: 2 }));
    }

    #[test]
    fn a_channel_that_never_sounds_is_left_alone() {
        // Channel 5 of the fixture selects a kit bank and plays nothing through it.
        let song = song(&testing::kit_bank_on_a_melodic_channel());
        assert!(!detect(&song).contains(&Fix::IgnoreBankSelect { channel: 5 }));
    }

    #[test]
    fn a_song_with_nothing_wrong_needs_no_fix() {
        let song = song(&testing::melody_and_accompaniment());
        assert!(detect(&song).is_empty());
    }

    #[test]
    fn the_fix_is_reported_against_the_channel_that_carries_it() {
        let song = song(&testing::kit_bank_on_a_melodic_channel());
        let resolved = crate::resolve(&detect(&song));
        assert!(resolved.ignore_bank[4]);
        assert_eq!(resolved.ignore_bank.iter().filter(|set| **set).count(), 1);
    }
}
