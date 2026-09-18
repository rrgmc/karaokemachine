//! Per-song corrections for defects in a song file's own MIDI events.
//!
//! A fix is a filter on the event stream, decided once when a song is analyzed and applied every
//! time it plays. **It may depend only on what the file says, never on how a bank renders it**: a
//! package is built once and played on machines with different SoundFonts, so bank knowledge is not
//! available at the moment a fix is decided. A defect in how one bank renders a correct file belongs
//! to `km-banks` or to a machine setting, and is not a fix.
//!
//! Two rules divide the list, and [`Fix::applies_itself`] is where they are stated:
//!
//! * A fix that makes every bank agree may apply itself. Suppressing a bank select that names a bank
//!   most fonts do not carry gives the same result everywhere, so nobody has to be asked.
//! * A fix that picks a winner must be offered. Muting a channel deletes music some banks render
//!   correctly, so it stays off until a person turns it on.
//!
//! The crate is read from two places that share no other ancestor: packaging detects fixes into a
//! manifest, and playback resolves them into [`ChannelFixes`]. Putting it in either would make the
//! other depend on it.
//!
//! Only what [`EventKind`] can express is reachable. `km_song::Song` carries no SysEx, so a GM, GS
//! or XG reset can be neither detected nor corrected, and nothing here touches lyrics, sync or
//! missing notes — those stay file problems.
//!
//! See `docs/ARCHITECTURE.md`.

pub mod force_program;
pub mod ignore_bank_select;
pub mod mute_channel;
pub mod recentre_bend;

use km_song::{EventKind, Song};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The drum channel, where note numbers select instruments rather than pitches.
///
/// Named here rather than borrowed because this crate's whole dependency list is `km-song` and
/// `serde`, and reaching a copy in `km-suitability` or `km-audio` would make a detector depend on a
/// scorer or a synthesizer to learn a fact about General MIDI.
pub const DRUM_CHANNEL: u8 = 9;

/// The number of MIDI channels, and so the width of every array in [`ChannelFixes`].
pub const CHANNELS: usize = 16;

/// One correction to a song's event stream.
///
/// **The catch-all keeps the JSON it could not read.** `SongKind::Unknown` and
/// `EditedField::Unknown` are unit variants because the values they stand in for are scalars, and a
/// name is all there is to lose. A fix carries arguments, and a curator's hand-set list survives a
/// rebuild by being copied whole — so a variant that remembered only *that* there had been a fix
/// would turn somebody's channel mute into `{"fix":"unknown"}` the first time an older build
/// rewrote the manifest it was sitting in. Holding the original value keeps every argument.
///
/// What a rewrite does not preserve is key order, which comes back sorted. Nothing reads a manifest
/// by byte, and buying the order back would mean turning on `serde_json`'s `preserve_order` for the
/// whole workspace — which cargo unifies, so it would reorder every manifest this project writes to
/// settle the spelling of the one kind it cannot read.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// Drop Bank Select on one channel, leaving the program change to select from the default bank.
    IgnoreBankSelect {
        /// The channel to suppress bank select on, 0-based.
        channel: u8,
    },
    /// Silence one channel entirely.
    MuteChannel {
        /// The channel to silence, 0-based.
        channel: u8,
    },
    /// Play one channel on a chosen instrument, whatever program the file selects.
    ForceProgram {
        /// The channel to re-voice, 0-based.
        channel: u8,
        /// The General MIDI program to play it on.
        program: u8,
    },
    /// Return one channel's pitch bend to centre when a note starts on a bend the file left behind.
    RecentreBend {
        /// The channel to recentre, 0-based.
        channel: u8,
    },
    /// A fix named by a build newer than this one, held exactly as it was read. Never constructed
    /// here, never resolved, and never described.
    Unknown(serde_json::Value),
}

/// The known shapes, which is what the wire format is derived from.
///
/// Separate from [`Fix`] so that a value failing to match any of them can be kept rather than
/// refused, which a derived enum has no way to express once its variants carry fields.
#[derive(Serialize, Deserialize)]
#[serde(tag = "fix", rename_all = "snake_case", deny_unknown_fields)]
enum Known {
    IgnoreBankSelect { channel: u8 },
    MuteChannel { channel: u8 },
    ForceProgram { channel: u8, program: u8 },
    RecentreBend { channel: u8 },
}

impl Serialize for Fix {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::IgnoreBankSelect { channel } => {
                Known::IgnoreBankSelect { channel: *channel }.serialize(serializer)
            }
            Self::MuteChannel { channel } => {
                Known::MuteChannel { channel: *channel }.serialize(serializer)
            }
            Self::ForceProgram { channel, program } => Known::ForceProgram {
                channel: *channel,
                program: *program,
            }
            .serialize(serializer),
            Self::RecentreBend { channel } => {
                Known::RecentreBend { channel: *channel }.serialize(serializer)
            }
            Self::Unknown(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Fix {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        // A channel outside the sixteen is refused rather than kept as unknown: the shape is one
        // this build understands and the argument is wrong, which is a corrupt manifest and not a
        // newer one.
        match Known::deserialize(&value) {
            Ok(Known::IgnoreBankSelect { channel }) => {
                check_channel::<D>(channel)?;
                Ok(Self::IgnoreBankSelect { channel })
            }
            Ok(Known::MuteChannel { channel }) => {
                check_channel::<D>(channel)?;
                Ok(Self::MuteChannel { channel })
            }
            Ok(Known::ForceProgram { channel, program }) => {
                check_channel::<D>(channel)?;
                check_program::<D>(program)?;
                Ok(Self::ForceProgram { channel, program })
            }
            Ok(Known::RecentreBend { channel }) => {
                check_channel::<D>(channel)?;
                Ok(Self::RecentreBend { channel })
            }
            Err(_) => Ok(Self::Unknown(value)),
        }
    }
}

/// Refuses a channel this build recognises the shape of but cannot place.
fn check_channel<'de, D: Deserializer<'de>>(channel: u8) -> Result<(), D::Error> {
    if usize::from(channel) < CHANNELS {
        Ok(())
    } else {
        Err(D::Error::custom(format!(
            "channel {channel} is outside the sixteen a MIDI file has"
        )))
    }
}

/// Refuses a program no program change could carry, on the same terms as [`check_channel`].
fn check_program<'de, D: Deserializer<'de>>(program: u8) -> Result<(), D::Error> {
    if u16::from(program) < force_program::PROGRAMS {
        Ok(())
    } else {
        Err(D::Error::custom(format!(
            "program {program} is outside the hundred and twenty-eight General MIDI names"
        )))
    }
}

impl Fix {
    /// Whether this fix may turn itself on without anybody agreeing to it.
    ///
    /// True only where every bank would otherwise disagree about the same file. An unknown fix is
    /// never applied by this build, so it answers false.
    pub fn applies_itself(&self) -> bool {
        match self {
            Self::IgnoreBankSelect { .. } => true,
            Self::MuteChannel { .. }
            | Self::ForceProgram { .. }
            | Self::RecentreBend { .. }
            | Self::Unknown(_) => false,
        }
    }

    /// The channel this fix acts on, where it names one.
    pub fn channel(&self) -> Option<u8> {
        match self {
            Self::IgnoreBankSelect { channel }
            | Self::MuteChannel { channel }
            | Self::ForceProgram { channel, .. }
            | Self::RecentreBend { channel } => Some(*channel),
            Self::Unknown(_) => None,
        }
    }

    /// The name this fix is stored under, for a form control and a log line.
    ///
    /// Matches the `fix` key in the manifest, so a checkbox and a stored value are named once.
    pub fn key(&self) -> &'static str {
        match self {
            Self::IgnoreBankSelect { .. } => "ignore_bank_select",
            Self::MuteChannel { .. } => "mute_channel",
            Self::ForceProgram { .. } => "force_program",
            Self::RecentreBend { .. } => "recentre_bend",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// Every fix in force on a song, flattened to what the sequencer acts on.
///
/// `Copy` and heap-free, because it rides to the audio thread inside a load command and is read in
/// the callback. A new kind of fix adds one array here and one arm in [`resolve`]; the sequencer
/// never learns the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFixes {
    /// Channels whose Bank Select is dropped, coarse and fine alike.
    pub ignore_bank: [bool; CHANNELS],
    /// Channels whose notes are not sounded.
    pub mute: [bool; CHANNELS],
    /// Channels whose program changes are replaced by a chosen program.
    pub force_program: [Option<u8>; CHANNELS],
    /// Channels shifted by a number of semitones of their own.
    pub channel_transpose: [i8; CHANNELS],
    /// Channels whose bend returns to centre when a note starts on one the file left behind.
    pub recentre_bend: [bool; CHANNELS],
}

impl Default for ChannelFixes {
    fn default() -> Self {
        Self {
            ignore_bank: [false; CHANNELS],
            mute: [false; CHANNELS],
            force_program: [None; CHANNELS],
            channel_transpose: [0; CHANNELS],
            recentre_bend: [false; CHANNELS],
        }
    }
}

impl ChannelFixes {
    /// Whether anything at all is in force, so a caller can skip work a song does not need.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Every fix the detectors are confident about, in channel order.
///
/// A detector that is unsure emits nothing. Muting or re-voicing the wrong channel ruins a song, so
/// a false positive costs more than no detection at all — the same bar `km_suitability::melody`
/// sets for the melody channel.
pub fn detect(song: &Song) -> Vec<Fix> {
    let mut fixes = automatic(song);
    fixes.extend(suggested(song));
    fixes.sort_by_key(|fix| (fix.key(), fix.channel()));
    fixes
}

/// The fixes a song may be given without anybody being asked.
///
/// **This is what a package records, and [`detect`] is not.** A stored list means *the corrections
/// in force*, which is what playback resolves and applies whole — so a fix that must be offered
/// cannot be in it until somebody has agreed, and a curator's answer would be overruled by a
/// re-detection if playback filtered instead.
///
/// **It runs only the detectors whose fixes apply themselves.** A package build and a song load
/// call it once per song, and a detector whose every answer would be filtered out is work thrown
/// away. The filter stays as the guard that keeps a detector in the wrong list harmless.
pub fn automatic(song: &Song) -> Vec<Fix> {
    let mut fixes = ignore_bank_select::detect(song);
    fixes.retain(Fix::applies_itself);
    fixes.sort_by_key(|fix| (fix.key(), fix.channel()));
    fixes
}

/// The fixes detection proposes that wait for a person to agree, for the curation tool to offer.
pub fn suggested(song: &Song) -> Vec<Fix> {
    let mut fixes = recentre_bend::detect(song);
    fixes.retain(|fix| !fix.applies_itself());
    fixes.sort_by_key(|fix| (fix.key(), fix.channel()));
    fixes
}

/// Flattens a list of fixes into the table playback reads.
///
/// An unknown fix is skipped, which is what keeps a package built by a newer build playable here
/// instead of refused.
pub fn resolve(fixes: &[Fix]) -> ChannelFixes {
    let mut resolved = ChannelFixes::default();
    for fix in fixes {
        match fix {
            Fix::IgnoreBankSelect { channel } => {
                resolved.ignore_bank[usize::from(*channel)] = true;
            }
            Fix::MuteChannel { channel } => {
                resolved.mute[usize::from(*channel)] = true;
            }
            Fix::ForceProgram { channel, program } => {
                resolved.force_program[usize::from(*channel)] = Some(*program);
            }
            Fix::RecentreBend { channel } => {
                resolved.recentre_bend[usize::from(*channel)] = true;
            }
            Fix::Unknown(_) => {}
        }
    }
    resolved
}

/// One line per fix, for the log the machine writes when a song starts.
///
/// Allocates, so it belongs on the control thread and never in the audio callback.
pub fn describe(fixes: &[Fix]) -> Vec<String> {
    fixes
        .iter()
        .filter_map(|fix| match fix {
            Fix::IgnoreBankSelect { channel } => Some(ignore_bank_select::describe(*channel)),
            Fix::MuteChannel { channel } => Some(mute_channel::describe(*channel)),
            Fix::ForceProgram { channel, program } => {
                Some(force_program::describe(*channel, *program))
            }
            Fix::RecentreBend { channel } => Some(recentre_bend::describe(*channel)),
            Fix::Unknown(_) => None,
        })
        .collect()
}

/// Every channel the song addresses, so a control can offer the ones a mute would mean something on.
pub fn channels_used(song: &Song) -> Vec<u8> {
    let mut seen = [false; CHANNELS];
    for event in &song.events {
        if let EventKind::NoteOn { channel, .. } = event.kind {
            seen[usize::from(channel)] = true;
        }
    }
    (0..CHANNELS)
        .filter(|channel| seen[*channel])
        .map(|channel| channel as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_fix_keeps_every_argument_through_a_round_trip() {
        let json = r#"{"fix":"swap_channels","from":3,"to":11}"#;
        let fix: Fix = serde_json::from_str(json).expect("reads as unknown");
        assert!(matches!(fix, Fix::Unknown(_)));

        // Keys come back sorted, because a `serde_json::Value` is a `BTreeMap`. Comparing values
        // rather than strings is what the claim is: every argument is kept, and a fix this build
        // cannot read is handed on with nothing dropped.
        let written = serde_json::to_string(&fix).expect("writes");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&written).expect("re-reads"),
            serde_json::from_str::<serde_json::Value>(json).expect("re-reads"),
        );
    }

    #[test]
    fn an_unknown_fix_changes_nothing() {
        let fix: Fix = serde_json::from_str(r#"{"fix":"whatever"}"#).expect("reads");
        assert!(resolve(std::slice::from_ref(&fix)).is_empty());
        assert!(describe(&[fix]).is_empty());
    }

    #[test]
    fn a_known_fix_round_trips_by_name() {
        let fix = Fix::MuteChannel { channel: 4 };
        let json = serde_json::to_string(&fix).expect("writes");
        assert_eq!(json, r#"{"fix":"mute_channel","channel":4}"#);
        assert_eq!(serde_json::from_str::<Fix>(&json).expect("reads"), fix);
    }

    #[test]
    fn a_fix_carrying_an_argument_round_trips_with_it() {
        let fix = Fix::ForceProgram {
            channel: 4,
            program: 52,
        };
        let json = serde_json::to_string(&fix).expect("writes");
        assert_eq!(json, r#"{"fix":"force_program","channel":4,"program":52}"#);
        assert_eq!(serde_json::from_str::<Fix>(&json).expect("reads"), fix);
    }

    #[test]
    fn a_channel_outside_the_sixteen_is_refused() {
        let json = r#"{"fix":"mute_channel","channel":16}"#;
        assert!(serde_json::from_str::<Fix>(json).is_err());
    }

    #[test]
    fn a_program_no_program_change_could_carry_is_refused() {
        let json = r#"{"fix":"force_program","channel":4,"program":128}"#;
        assert!(serde_json::from_str::<Fix>(json).is_err());
    }

    #[test]
    fn only_the_bank_select_fix_applies_itself() {
        assert!(Fix::IgnoreBankSelect { channel: 0 }.applies_itself());
        assert!(!Fix::MuteChannel { channel: 0 }.applies_itself());
        assert!(
            !Fix::ForceProgram {
                channel: 0,
                program: 0,
            }
            .applies_itself()
        );
    }

    #[test]
    fn automatic_and_suggested_split_detection_by_whether_a_fix_applies_itself() {
        use km_song::{ParseOptions, testing};

        for bytes in [
            testing::kit_bank_on_a_melodic_channel(),
            testing::bend_left_off_centre(),
        ] {
            let song = Song::parse(&bytes, &ParseOptions::default()).expect("fixture parses");
            let detected = detect(&song);
            let (applies, waits): (Vec<Fix>, Vec<Fix>) =
                detected.iter().cloned().partition(Fix::applies_itself);
            assert!(!detected.is_empty());
            assert_eq!(automatic(&song), applies);
            assert_eq!(suggested(&song), waits);
        }
    }

    #[test]
    fn resolving_sets_the_array_the_sequencer_reads() {
        let resolved = resolve(&[
            Fix::IgnoreBankSelect { channel: 4 },
            Fix::MuteChannel { channel: 7 },
        ]);
        assert!(resolved.ignore_bank[4]);
        assert!(!resolved.ignore_bank[7]);
        assert!(resolved.mute[7]);
        assert!(!resolved.is_empty());
    }
}
