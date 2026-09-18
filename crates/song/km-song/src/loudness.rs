//! How loud a song will sound, read from its events with no synthesizer.
//!
//! **A MIDI song's loudness is a fact about the file that a bank only shifts.** Rendering one and
//! metering it is the accurate way to find it, and it needs a SoundFont, seconds of CPU, and an
//! answer to which bank. This reads the same quantity out of the events in about a fifth of a
//! millisecond, and lands within about 2.3 LU of what a render says.
//!
//! **Where the form comes from.** General MIDI defines channel volume and expression as
//! `40·log₁₀(v/127)` dB, so each scales amplitude by the square of its controller value, and SF2's
//! default velocity modulator is close enough to the same curve to be treated the same way. One note
//! therefore contributes `(velocity/127)⁴ · (cc7/127)⁴ · (cc11/127)⁴` of power. Powers of
//! simultaneous notes add, because nothing here knows their phase. The result is integrated the way
//! EBU R128 integrates a real signal, over 400 ms blocks at a 100 ms hop with the same relative gate,
//! so the figure moves with a song's loud parts rather than with its length.
//!
//! **What it cannot see is which instrument a program number selects**, and that is where the
//! residual lives. A flute and a distorted guitar at the same velocity through the same channel
//! volume are the same number here and are not the same sound.
//!
//! The reasoning, the corpus figures behind every constant, and what each was measured against are in
//! `docs/research/midi-loudness.md`.

use crate::{EventKind, Song};

/// The estimate's sub-block, in milliseconds. Four of these make one R128 gating block.
const SUB_BLOCK_MS: u32 = 100;

/// How far below the loudest block a block stops counting.
///
/// R128's absolute gate is −70 LUFS, which is a level on an absolute scale. This figure has no
/// absolute scale, so the gate is taken from the song's own loudest block instead. What it has to
/// exclude is a song's own silence, and 70 dB down is silence by any reading.
const ABSOLUTE_GATE_DB: f64 = 70.0;

/// R128's relative gate: a block more than this far below the ungated mean is dropped.
const RELATIVE_GATE_LU: f64 = 10.0;

/// General MIDI's default channel volume, which a file that never sends CC7 is played at.
const DEFAULT_CC7: u8 = 100;

/// Expression defaults to full scale.
const DEFAULT_CC11: u8 = 127;

/// How fast a held note's contribution decays, in milliseconds.
///
/// **A stand-in for an envelope this cannot know.** A piano note and a string note held for the same
/// four seconds do not sound for the same four seconds, and which one a program number selects is
/// exactly what is missing here. Measured over the corpus, moving it between 200 ms and no decay at
/// all changes the residual by 0.2 LU, so the value matters far less than having one.
const DECAY_MS: f64 = 1_500.0;

/// How long a drum hit counts for, in milliseconds, whatever its note-off says.
///
/// A drum part is written with note lengths that mean nothing: a kick is a transient whether its
/// note-off arrives after a sixteenth or after a bar.
const DRUM_MS: u32 = 150;

/// Number of milliseconds below which there is not enough song to integrate.
const MIN_MEASURABLE_MS: u32 = 400;

/// General MIDI's percussion channel, where a note number selects an instrument rather than a pitch.
const DRUM_CHANNEL: usize = 9;

/// A note that is currently sounding.
struct Sounding {
    key: u8,
    /// The power this note contributes at its onset.
    power: f64,
    started_ms: u32,
}

impl Song {
    /// An estimate of how loud this song will sound, in decibels on a scale of its own.
    ///
    /// `muted` is a channel whose notes are left out, for the guide melody. **Pass the melody channel
    /// whether or not the melody is currently audible**: the figure calibrates a song against the
    /// corpus, the corpus figure was taken with the melody muted, and a number that moved when
    /// somebody pressed the melody button would change a song's level in the middle of it.
    ///
    /// `None` where there is not enough sound to integrate, which is the honest answer for a file
    /// that is empty, silent, or shorter than one gating block. A caller reads that as *do not touch
    /// this song*.
    ///
    /// Only differences between songs mean anything. [`km_loudness::midi_gain`] is what turns one
    /// into a level, and it carries the corpus figure the difference is taken against.
    #[must_use]
    pub fn estimated_loudness_db(&self, muted: Option<u8>) -> Option<f32> {
        let duration_ms = self.duration_ms();
        if duration_ms < MIN_MEASURABLE_MS {
            return None;
        }
        let sub_blocks = (duration_ms / SUB_BLOCK_MS) as usize + 1;
        let mut power = vec![0.0f64; sub_blocks];

        let mut cc7 = [DEFAULT_CC7; 16];
        let mut cc11 = [DEFAULT_CC11; 16];
        let mut sounding: Vec<Vec<Sounding>> = (0..16).map(|_| Vec::new()).collect();

        // Controller state is sampled at each onset rather than read at the end, so the walk is in
        // tick order and a note is laid into the array when its note-off arrives.
        for event in &self.events {
            let at_ms = self.tempo_map.tick_to_ms(event.tick);
            match event.kind {
                EventKind::NoteOn {
                    channel,
                    key,
                    velocity,
                } => {
                    if muted == Some(channel) {
                        continue;
                    }
                    let channel = usize::from(channel.min(15));
                    let level = f64::from(velocity) / 127.0 * f64::from(cc7[channel]) / 127.0
                        * f64::from(cc11[channel])
                        / 127.0;
                    sounding[channel].push(Sounding {
                        key,
                        power: level.powi(4),
                        started_ms: at_ms,
                    });
                }
                EventKind::NoteOff { channel, key } => {
                    let channel = usize::from(channel.min(15));
                    if let Some(index) = sounding[channel].iter().position(|n| n.key == key) {
                        let note = sounding[channel].remove(index);
                        lay(&mut power, &note, channel, at_ms, sub_blocks);
                    }
                }
                EventKind::Controller {
                    channel,
                    controller,
                    value,
                } => {
                    let channel = usize::from(channel.min(15));
                    match controller {
                        7 => cc7[channel] = value,
                        11 => cc11[channel] = value,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // A note still sounding when the file runs out plays to the end of it. The parser repairs
        // most of these, and one that reaches here is a file that ended mid-note.
        for (channel, left) in sounding.iter().enumerate() {
            for note in left {
                lay(&mut power, note, channel, duration_ms, sub_blocks);
            }
        }

        gated_mean_db(&power).map(|db| db as f32)
    }
}

/// Adds one note's power to every sub-block it sounds through.
fn lay(power: &mut [f64], note: &Sounding, channel: usize, ends_ms: u32, sub_blocks: usize) {
    // A drum note's own length is ignored in both directions: a sample plays out whether the file
    // releases the note after a sixteenth or after a bar, and a kick written 30 ms long is not a
    // quieter kick. Taking the shorter of the two would make it one.
    let ends_ms = if channel == DRUM_CHANNEL {
        note.started_ms.saturating_add(DRUM_MS)
    } else {
        ends_ms
    };
    let first = (note.started_ms / SUB_BLOCK_MS) as usize;
    let last = ((ends_ms / SUB_BLOCK_MS) as usize).min(sub_blocks.saturating_sub(1));
    for (index, slot) in power.iter_mut().enumerate().take(last + 1).skip(first) {
        let elapsed = f64::from(index as u32 * SUB_BLOCK_MS).max(f64::from(note.started_ms))
            - f64::from(note.started_ms);
        // The decay is an amplitude, and this array holds power.
        let decay = (-elapsed / DECAY_MS).exp();
        *slot += note.power * decay * decay;
    }
}

/// R128's gating, over blocks of power on the caller's own scale.
///
/// Four sub-blocks make one 400 ms gating block, stepped by one sub-block so blocks overlap by 75%,
/// which is what the standard integrates over.
fn gated_mean_db(sub_blocks: &[f64]) -> Option<f64> {
    if sub_blocks.len() < 4 {
        return None;
    }
    let blocks: Vec<f64> = sub_blocks
        .windows(4)
        .map(|w| w.iter().sum::<f64>() / 4.0)
        .filter(|p| *p > 0.0)
        .collect();
    let loudest = blocks.iter().copied().fold(0.0f64, f64::max);
    if loudest <= 0.0 {
        return None;
    }
    let floor = loudest / 10.0f64.powf(ABSOLUTE_GATE_DB / 10.0);
    let above: Vec<f64> = blocks.into_iter().filter(|p| *p >= floor).collect();
    if above.is_empty() {
        return None;
    }

    let ungated = above.iter().sum::<f64>() / above.len() as f64;
    let relative = ungated / 10.0f64.powf(RELATIVE_GATE_LU / 10.0);
    let gated: Vec<f64> = above.into_iter().filter(|p| *p >= relative).collect();
    if gated.is_empty() {
        return None;
    }
    let mean = gated.iter().sum::<f64>() / gated.len() as f64;
    Some(10.0 * mean.log10())
}

#[cfg(test)]
mod tests {
    use crate::{ParseOptions, Song, testing};

    /// One channel-voice event and the tick it happens at.
    type Timed = (u32, [u8; 3]);

    /// A one-track MIDI file carrying exactly these events.
    ///
    /// Hand-written bytes rather than a `Song` literal, because the parser is the only public way
    /// into a `Song` and these tests are then on the same footing as every other fixture in this
    /// crate. [`testing`] has no note-level builder to borrow.
    fn midi(events: &[Timed]) -> Vec<u8> {
        fn varlen(out: &mut Vec<u8>, mut value: u32) {
            let mut buffer = [0u8; 4];
            let mut len = 0;
            loop {
                buffer[len] = (value & 0x7F) as u8;
                len += 1;
                value >>= 7;
                if value == 0 {
                    break;
                }
            }
            for i in (0..len).rev() {
                out.push(buffer[i] | if i == 0 { 0x00 } else { 0x80 });
            }
        }

        let mut track: Vec<u8> = Vec::new();
        let mut previous = 0u32;
        for (tick, bytes) in events {
            varlen(&mut track, tick - previous);
            previous = *tick;
            track.extend_from_slice(bytes);
        }
        varlen(&mut track, 0);
        track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&testing::TPQN.to_be_bytes());
        out.extend_from_slice(b"MTrk");
        out.extend_from_slice(&u32::try_from(track.len()).expect("small").to_be_bytes());
        out.extend_from_slice(&track);
        out
    }

    /// At the fixtures' 480 ticks per quarter and 120 BPM, a quarter note is 500 ms.
    fn ticks(ms: u32) -> u32 {
        ms * u32::from(testing::TPQN) / 500
    }

    fn note_on(channel: u8, key: u8, velocity: u8) -> [u8; 3] {
        [0x90 | channel, key, velocity]
    }

    fn note_off(channel: u8, key: u8) -> [u8; 3] {
        [0x80 | channel, key, 0]
    }

    fn controller(channel: u8, number: u8, value: u8) -> [u8; 3] {
        [0xB0 | channel, number, value]
    }

    fn level_of(events: &[Timed]) -> Option<f32> {
        Song::parse(&midi(events), &ParseOptions::default())
            .expect("the bytes this builds are a valid MIDI file")
            .estimated_loudness_db(None)
    }

    /// A held note, two seconds long, with whatever controllers are wanted in front of it.
    fn held(setup: &[(u8, u8)], channel: u8, key: u8, velocity: u8) -> Vec<Timed> {
        let mut events: Vec<Timed> = setup
            .iter()
            .map(|(number, value)| (0, controller(channel, *number, *value)))
            .collect();
        events.push((0, note_on(channel, key, velocity)));
        events.push((ticks(2_000), note_off(channel, key)));
        events
    }

    /// A real fixture produces a number, which is the floor everything else rests on.
    #[test]
    fn a_song_estimates_to_something() {
        let song = Song::parse(&testing::high_quality_song(), &ParseOptions::default())
            .expect("fixture parses");
        let estimated = song
            .estimated_loudness_db(None)
            .expect("a song with notes has a level");
        assert!(estimated.is_finite(), "got {estimated}");
    }

    /// **The origin of the scale.** One note at full velocity through a channel at full volume and
    /// full expression is 0 dB by construction, and every corpus figure is a distance from that.
    #[test]
    fn a_full_scale_note_is_the_zero_of_the_scale() {
        let estimated = level_of(&held(&[(7, 127), (11, 127)], 0, 60, 127)).expect("a note");
        // The decay pulls the tail of a two-second note down, so this cannot be exactly 0. What is
        // pinned here is where the scale starts, not the envelope.
        assert!(
            (-8.0..=0.1).contains(&estimated),
            "a full-scale note measured {estimated} dB"
        );
    }

    /// Halving the channel volume takes 12 dB off, because volume scales amplitude by its square.
    #[test]
    fn halving_the_channel_volume_costs_twelve_decibels() {
        let loud = level_of(&held(&[(7, 127)], 0, 60, 100)).expect("loud");
        let quiet = level_of(&held(&[(7, 64)], 0, 60, 100)).expect("quiet");
        let difference = loud - quiet;
        assert!(
            (difference - 12.0).abs() < 0.5,
            "halving CC7 moved it {difference} dB, not about 12"
        );
    }

    /// Expression carries the same curve as volume, through the same code path.
    #[test]
    fn expression_scales_the_same_way_as_volume() {
        let by_volume = level_of(&held(&[(7, 64), (11, 127)], 0, 60, 100)).expect("volume");
        let by_expression = level_of(&held(&[(7, 127), (11, 64)], 0, 60, 100)).expect("expression");
        assert!(
            (by_volume - by_expression).abs() < 0.01,
            "{by_volume} against {by_expression}: the two controllers should behave alike"
        );
    }

    /// Halving the velocity costs the same 12 dB, which is the SF2 default modulator's curve.
    #[test]
    fn halving_the_velocity_costs_twelve_decibels() {
        let loud = level_of(&held(&[], 0, 60, 127)).expect("loud");
        let quiet = level_of(&held(&[], 0, 60, 64)).expect("quiet");
        let difference = loud - quiet;
        assert!(
            (difference - 11.9).abs() < 0.5,
            "halving the velocity moved it {difference} dB, not about 12"
        );
    }

    /// Two notes at once are 3 dB louder than one, because powers add.
    #[test]
    fn two_notes_are_three_decibels_louder_than_one() {
        let mut two = held(&[], 0, 60, 100);
        two.insert(1, (0, note_on(0, 67, 100)));
        two.push((ticks(2_000), note_off(0, 67)));

        let one = level_of(&held(&[], 0, 60, 100)).expect("one");
        let two = level_of(&two).expect("two");
        let difference = two - one;
        assert!(
            (difference - 3.01).abs() < 0.2,
            "a second note moved it {difference} dB, not about 3"
        );
    }

    /// A muted channel is left out, which is what keeps the figure level with the corpus.
    #[test]
    fn a_muted_channel_does_not_count() {
        let mut events = held(&[], 0, 60, 100);
        events.insert(1, (0, note_on(1, 67, 100)));
        events.push((ticks(2_000), note_off(1, 67)));
        let song = Song::parse(&midi(&events), &ParseOptions::default()).expect("parses");

        let both = song.estimated_loudness_db(None).expect("both");
        let one = song.estimated_loudness_db(Some(1)).expect("one");
        assert!(
            (both - one - 3.01).abs() < 0.2,
            "muting a channel moved it {} dB, not about 3",
            both - one
        );
    }

    /// A drum hit counts for a fixed time whatever its note-off says.
    ///
    /// A drum part is written with note lengths that mean nothing, so two files differing only in
    /// how long the kick is held have to measure alike.
    #[test]
    fn a_drum_note_is_a_transient_whatever_its_length() {
        let mut short = Vec::new();
        let mut long = Vec::new();
        for bar in 0..8u32 {
            let at = ticks(bar * 500);
            short.push((at, note_on(9, 36, 100)));
            short.push((at + ticks(50), note_off(9, 36)));
            long.push((at, note_on(9, 36, 100)));
            long.push((at + ticks(400), note_off(9, 36)));
        }
        let short = level_of(&short).expect("short");
        let long = level_of(&long).expect("long");
        // Not exact: the two files end at different ticks, so the gating sees one more part-block in
        // the longer of them. What is pinned is that the note length does not carry the level.
        assert!(
            (short - long).abs() < 0.2,
            "{short} against {long}: a drum's note length should not change its level"
        );
    }

    /// A song's length does not set its level: the same music twice over measures the same.
    ///
    /// This is what the gating buys. Without it a long quiet outro would drag a song's figure down
    /// and the machine would raise the whole song to compensate.
    #[test]
    fn a_longer_song_of_the_same_music_measures_the_same() {
        let phrase = |bars: u32| {
            let mut events = Vec::new();
            for bar in 0..bars {
                let at = ticks(bar * 1_000);
                events.push((at, note_on(0, 60, 100)));
                events.push((at + ticks(900), note_off(0, 60)));
            }
            events
        };
        let short = level_of(&phrase(4)).expect("short");
        let long = level_of(&phrase(16)).expect("long");
        assert!(
            (short - long).abs() < 0.5,
            "{short} against {long}: length should not set the level"
        );
    }

    /// Silence after the music does not count, which is the absolute gate doing its job.
    #[test]
    fn trailing_silence_does_not_count() {
        let mut with_silence = held(&[], 0, 60, 100);
        // A controller far in the future, so the file is long and mostly empty.
        with_silence.push((ticks(60_000), controller(0, 7, 100)));

        let plain = level_of(&held(&[], 0, 60, 100)).expect("plain");
        let padded = level_of(&with_silence).expect("padded");
        assert!(
            (plain - padded).abs() < 0.5,
            "{plain} against {padded}: a minute of silence should not lower the level"
        );
    }

    /// Too little sound to integrate is no answer rather than a quiet one.
    #[test]
    fn nothing_to_measure_is_no_measurement() {
        // Shorter than one gating block.
        let brief: Vec<Timed> = vec![(0, note_on(0, 60, 100)), (ticks(100), note_off(0, 60))];
        assert_eq!(level_of(&brief), None);

        // Long enough, and carrying no note at all.
        let quiet: Vec<Timed> = vec![
            (0, controller(0, 7, 100)),
            (ticks(4_000), controller(0, 7, 90)),
        ];
        assert_eq!(level_of(&quiet), None);
    }

    /// A note the file never releases plays to the end of it rather than being dropped.
    #[test]
    fn a_note_left_sounding_still_counts() {
        let events: Vec<Timed> = vec![
            (0, note_on(0, 60, 100)),
            // A later event so the song has a duration, and no note-off for the note above.
            (ticks(4_000), controller(0, 7, 100)),
        ];
        assert!(
            level_of(&events).is_some(),
            "a file that ends mid-note still has a level"
        );
    }
}
