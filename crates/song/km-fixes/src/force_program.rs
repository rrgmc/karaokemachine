//! Playing one channel on an instrument of somebody's choosing rather than the file's.
//!
//! **There is no detector here, and there cannot be one.** What makes a channel's instrument wrong
//! is how the bank in the machine renders the program the file asks for — a lead that whistles, a
//! pad that swallows the singer, a patch the arranger picked for a module nobody owns. That is a
//! fact about a bank, and a fix may depend only on what the file says, so no rule may propose this
//! one. A person listening to the song may still say it.
//!
//! **It is therefore offered rather than applied**, on the same half of the rule as a channel mute
//! and for the same reason: a program somebody chose deletes the one the arranger chose. It is the
//! gentler of the two answers to a part that spoils a song, because the notes survive.
//!
//! **The drum channel is not offered one.** A program change there selects a kit rather than an
//! instrument, so a melodic program on it means nothing a singer would want.

use crate::DRUM_CHANNEL;

/// The number of programs General MIDI names, and so the bound on a forced one.
pub const PROGRAMS: u16 = 128;

/// The log line this fix writes when a song starts.
pub fn describe(channel: u8, program: u8) -> String {
    format!("channel {channel}: played as {}", program_name(program))
}

/// Whether a channel may be given an instrument at all.
pub fn allows(channel: u8) -> bool {
    channel != DRUM_CHANNEL && usize::from(channel) < crate::CHANNELS
}

/// The General MIDI name of a program, for a log line and for a control somebody reads.
///
/// Named here for the reason [`crate::DRUM_CHANNEL`] is: it is a fact about General MIDI, and this
/// crate's whole dependency list is `km-song` and `serde`, so reaching a copy elsewhere would make
/// every reader of a fix depend on a synthesizer or a curation tool to learn one.
///
/// **The numbers are the wire's and not the sheet's.** General MIDI counts instruments from one and
/// a MIDI program change carries zero, so `program` 0 is "Acoustic grand piano". A program outside
/// the hundred and twenty-eight is named as itself, which only a corrupt list can reach.
pub fn program_name(program: u8) -> String {
    NAMES
        .get(usize::from(program))
        .map(|name| (*name).to_owned())
        .unwrap_or_else(|| format!("program {program}"))
}

/// The General MIDI 1 sound set, in program order.
///
/// Sentence case rather than the specification's title case, because these are read on a page and in
/// a log line beside ordinary prose.
const NAMES: [&str; PROGRAMS as usize] = [
    "Acoustic grand piano",
    "Bright acoustic piano",
    "Electric grand piano",
    "Honky-tonk piano",
    "Electric piano 1",
    "Electric piano 2",
    "Harpsichord",
    "Clavi",
    "Celesta",
    "Glockenspiel",
    "Music box",
    "Vibraphone",
    "Marimba",
    "Xylophone",
    "Tubular bells",
    "Dulcimer",
    "Drawbar organ",
    "Percussive organ",
    "Rock organ",
    "Church organ",
    "Reed organ",
    "Accordion",
    "Harmonica",
    "Tango accordion",
    "Acoustic guitar (nylon)",
    "Acoustic guitar (steel)",
    "Electric guitar (jazz)",
    "Electric guitar (clean)",
    "Electric guitar (muted)",
    "Overdriven guitar",
    "Distortion guitar",
    "Guitar harmonics",
    "Acoustic bass",
    "Electric bass (finger)",
    "Electric bass (pick)",
    "Fretless bass",
    "Slap bass 1",
    "Slap bass 2",
    "Synth bass 1",
    "Synth bass 2",
    "Violin",
    "Viola",
    "Cello",
    "Contrabass",
    "Tremolo strings",
    "Pizzicato strings",
    "Orchestral harp",
    "Timpani",
    "String ensemble 1",
    "String ensemble 2",
    "Synth strings 1",
    "Synth strings 2",
    "Choir aahs",
    "Voice oohs",
    "Synth voice",
    "Orchestra hit",
    "Trumpet",
    "Trombone",
    "Tuba",
    "Muted trumpet",
    "French horn",
    "Brass section",
    "Synth brass 1",
    "Synth brass 2",
    "Soprano sax",
    "Alto sax",
    "Tenor sax",
    "Baritone sax",
    "Oboe",
    "English horn",
    "Bassoon",
    "Clarinet",
    "Piccolo",
    "Flute",
    "Recorder",
    "Pan flute",
    "Blown bottle",
    "Shakuhachi",
    "Whistle",
    "Ocarina",
    "Lead 1 (square)",
    "Lead 2 (sawtooth)",
    "Lead 3 (calliope)",
    "Lead 4 (chiff)",
    "Lead 5 (charang)",
    "Lead 6 (voice)",
    "Lead 7 (fifths)",
    "Lead 8 (bass + lead)",
    "Pad 1 (new age)",
    "Pad 2 (warm)",
    "Pad 3 (polysynth)",
    "Pad 4 (choir)",
    "Pad 5 (bowed)",
    "Pad 6 (metallic)",
    "Pad 7 (halo)",
    "Pad 8 (sweep)",
    "FX 1 (rain)",
    "FX 2 (soundtrack)",
    "FX 3 (crystal)",
    "FX 4 (atmosphere)",
    "FX 5 (brightness)",
    "FX 6 (goblins)",
    "FX 7 (echoes)",
    "FX 8 (sci-fi)",
    "Sitar",
    "Banjo",
    "Shamisen",
    "Koto",
    "Kalimba",
    "Bag pipe",
    "Fiddle",
    "Shanai",
    "Tinkle bell",
    "Agogo",
    "Steel drums",
    "Woodblock",
    "Taiko drum",
    "Melodic tom",
    "Synth drum",
    "Reverse cymbal",
    "Guitar fret noise",
    "Breath noise",
    "Seashore",
    "Bird tweet",
    "Telephone ring",
    "Helicopter",
    "Applause",
    "Gunshot",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fix, resolve};

    #[test]
    fn an_instrument_never_arrives_by_itself() {
        assert!(
            !Fix::ForceProgram {
                channel: 3,
                program: 52,
            }
            .applies_itself()
        );
    }

    #[test]
    fn forcing_marks_only_the_channel_named() {
        let resolved = resolve(&[Fix::ForceProgram {
            channel: 3,
            program: 52,
        }]);
        assert_eq!(resolved.force_program[3], Some(52));
        assert_eq!(
            resolved
                .force_program
                .iter()
                .filter(|slot| slot.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn the_drum_channel_takes_no_instrument() {
        assert!(!allows(DRUM_CHANNEL));
        assert!(allows(0) && allows(15));
    }

    #[test]
    fn a_program_is_named_by_its_wire_number() {
        // The off-by-one that catches everybody: General MIDI's sheet counts from one, and the
        // message carries zero.
        assert_eq!(program_name(0), "Acoustic grand piano");
        assert_eq!(program_name(52), "Choir aahs");
        assert_eq!(program_name(127), "Gunshot");
    }
}
