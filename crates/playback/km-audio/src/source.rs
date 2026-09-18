//! Things that turn MIDI messages into audio.
//!
//! Playback uses [`SoundFontSource`], a thin adapter over `rustysynth`. Tests use
//! [`TestToneSource`], which synthesises plain sine tones and needs no SoundFont file — so the whole
//! render pipeline (block stepping, buffer handling, interleaving, volume) is verifiable in CI,
//! where no `.sf2` exists and no audio device is available.

use std::path::Path;
use std::sync::Arc;

use crate::sequencer::MidiSink;

/// A MIDI sink that also produces audio.
pub trait AudioSource: MidiSink {
    /// Renders exactly [`AudioSource::block_size`] frames into two mono buffers.
    fn render(&mut self, left: &mut [f32], right: &mut [f32]);
    /// Frames produced per [`AudioSource::render`] call.
    fn block_size(&self) -> usize;
    /// Sample rate the source was built for.
    fn sample_rate(&self) -> u32;
}

/// Why a SoundFont-backed source could not be created.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// The SoundFont file could not be read.
    #[error("could not read SoundFont {path}: {source}")]
    Io {
        /// The file that failed.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The file is not a usable SoundFont.
    #[error("not a usable SoundFont: {0}")]
    SoundFont(String),
    /// The synthesizer rejected the requested settings.
    #[error("could not create the synthesizer: {0}")]
    Synthesizer(String),
}

/// Voices the synthesizer may sound at once — `rustysynth`'s validated maximum.
///
/// Not the number of *notes*: see [`SoundFontSource::new`], where it is set and where the reason it
/// is not the crate's own default of 64 is written down.
///
/// `pub(crate)` so [`crate::sequencer::Sequencer`] can size its sounding-note list against the real
/// ceiling rather than against a number somebody guessed.
pub(crate) const MAX_POLYPHONY: usize = 256;

/// How many dropped records [`BankDefects`] quotes, out of however many there were.
///
/// The synthesizer itself keeps the first 64 and counts the rest. Three is what fits in the one-line
/// message every consumer of this wants — a journal entry, a line under `--set-soundfont` — and a
/// bank with a hundred bad records is not better explained by the ninth of them.
const DEFECT_EXAMPLES: usize = 3;

/// What the synthesizer dropped while parsing a bank, and did not refuse it for.
///
/// A bank with one defective record loads without that record rather than not loading at all, which
/// is why the machine plays several well-regarded banks it used to refuse. The cost of that is a
/// bank that is quietly *incomplete* — an instrument that never sounds, and no error anywhere — so
/// whatever was dropped has to be sayable. Empty is the ordinary case.
///
/// The synthesizer's own warning type is deliberately not exposed: this crate is the only one that
/// names `rustysynth`, and the warnings are formatted here at load time so they can cross that line
/// as ordinary strings. They are also `#[non_exhaustive]` upstream, so formatting is the only thing
/// that stays correct as the fork gains variants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BankDefects {
    dropped: usize,
    examples: Vec<String>,
}

impl BankDefects {
    /// How many records the synthesizer dropped, in total.
    ///
    /// Not `examples().len()`: the synthesizer retains only the first 64 warnings but counts every
    /// one, and this is the count.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// The first few dropped records, in the synthesizer's own words.
    pub fn examples(&self) -> &[String] {
        &self.examples
    }

    /// Whether the bank loaded whole, which is the ordinary case.
    pub fn is_empty(&self) -> bool {
        self.dropped == 0
    }

    #[cfg(test)]
    fn new(dropped: usize, examples: &[&str]) -> Self {
        Self {
            dropped,
            examples: examples.iter().map(|e| (*e).to_string()).collect(),
        }
    }
}

impl std::fmt::Display for BankDefects {
    /// One line, because every caller puts this in one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return f.write_str("no defective records");
        }
        write!(f, "{} defective record", self.dropped)?;
        if self.dropped != 1 {
            f.write_str("s")?;
        }
        f.write_str(" dropped")?;
        if !self.examples.is_empty() {
            write!(f, ": {}", self.examples.join("; "))?;
            // The remainder is counted rather than listed, and is counted against `dropped` so that
            // it stays right whether the examples were capped here or by the synthesizer's own 64.
            let rest = self.dropped.saturating_sub(self.examples.len());
            if rest > 0 {
                write!(f, "; and {rest} more")?;
            }
        }
        Ok(())
    }
}

/// A parsed General MIDI bank, shareable across synthesizers.
///
/// The split matters because the output device is opened and released repeatedly over one run, and
/// each open needs a synthesizer built at *that* device's sample rate. Reading the file is tens of
/// megabytes of parsing; building a synthesizer from an already-parsed bank is not. So the file is
/// read once at startup and every reopen clones an `Arc`.
///
/// It wraps the `rustysynth` type rather than exposing it so that `km-app` — which holds the bank
/// for the life of the process — does not have to name a dependency of this crate.
#[derive(Clone)]
pub struct Bank {
    soundfont: Arc<rustysynth::SoundFont>,
    defects: BankDefects,
}

impl Bank {
    /// Reads and parses a `.sf2` file.
    ///
    /// Says nothing itself, on purpose: a bank that loaded with records missing is reported by
    /// whoever asked for it, in the words that surface suits — a `warn!` in the journal at startup,
    /// a line on stdout from `--set-soundfont`. See [`Bank::defects`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        let path = path.as_ref();
        let mut file = std::fs::File::open(path).map_err(|source| SourceError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let soundfont = rustysynth::SoundFont::new(&mut file)
            .map_err(|e| SourceError::SoundFont(e.to_string()))?;
        let defects = BankDefects {
            dropped: soundfont.get_warning_count(),
            examples: soundfont
                .get_warnings()
                .iter()
                .take(DEFECT_EXAMPLES)
                .map(|warning| warning.to_string())
                .collect(),
        };
        Ok(Self {
            soundfont: Arc::new(soundfont),
            defects,
        })
    }

    /// What the synthesizer dropped to get this bank to load.
    pub fn defects(&self) -> &BankDefects {
        &self.defects
    }
}

impl std::fmt::Debug for Bank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bank")
            .field("dropped", &self.defects.dropped)
            .finish_non_exhaustive()
    }
}

/// A General MIDI synthesizer driven by a SoundFont.
pub struct SoundFontSource {
    synth: rustysynth::Synthesizer,
    sample_rate: u32,
}

impl SoundFontSource {
    /// Loads a SoundFont from disk and builds a synthesizer for it.
    ///
    /// One-shot: the file is read every call. Anything that builds more than one synthesizer from
    /// the same bank — the audio thread, which does so on every reopen — loads a [`Bank`] once and
    /// uses [`SoundFontSource::from_bank`].
    pub fn from_path(path: impl AsRef<Path>, sample_rate: u32) -> Result<Self, SourceError> {
        Self::from_bank(&Bank::load(path)?, sample_rate)
    }

    /// Builds a synthesizer for an already-parsed bank, at the device's rate.
    pub fn from_bank(bank: &Bank, sample_rate: u32) -> Result<Self, SourceError> {
        Self::new(Arc::clone(&bank.soundfont), sample_rate)
    }

    /// Builds a synthesizer for an already-loaded SoundFont.
    ///
    /// Takes an [`Arc`] so one bank can back several synthesizers without being loaded twice; a
    /// General MIDI bank is tens of megabytes.
    pub fn new(
        soundfont: Arc<rustysynth::SoundFont>,
        sample_rate: u32,
    ) -> Result<Self, SourceError> {
        let mut settings =
            rustysynth::SynthesizerSettings::new(i32::try_from(sample_rate).unwrap_or(44_100));
        // rustysynth counts *voices*, not notes, and its default of 64 is far too few for a dense
        // General MIDI arrangement: `note_on` starts one voice per matching instrument region, and
        // every voice lingers through its release envelope long after its note-off. Measured over a
        // real file riding the sustain pedal, true demand peaks at 139 voices on the hungriest of
        // three banks while only ~33 notes sound at once, and 64 saturates on all three. How many
        // voices a note costs is the bank's property, so the ceiling has to answer the bank a
        // machine is pointed at rather than the one it ships. This is close to free rather than a
        // trade: `VoiceCollection::process` iterates only the active prefix, so per-block cost
        // tracks real demand and the cap costs one allocation at construction.
        settings.maximum_polyphony = MAX_POLYPHONY;
        let synth = rustysynth::Synthesizer::new(&soundfont, &settings)
            .map_err(|e| SourceError::Synthesizer(e.to_string()))?;
        Ok(Self { synth, sample_rate })
    }

    /// A channel's pitch bend range in semitones, as the file has left it.
    ///
    /// **The one piece of channel state a rendered buffer cannot be read for.** The default is 2 and
    /// a file asking for more says so with RPN 0, whose three control changes have no observable
    /// effect of their own: a data entry arriving with no parameter selected is discarded in
    /// silence, so the same messages in a different order either establish a range or establish
    /// nothing. This is how a caller tells those apart.
    pub fn channel_pitch_bend_range(&self, channel: u8) -> f32 {
        self.synth.get_channel_pitch_bend_range(i32::from(channel))
    }

    /// A channel's tune in semitones, from RPN 1 and RPN 2 together.
    pub fn channel_tune(&self, channel: u8) -> f32 {
        self.synth.get_channel_tune(i32::from(channel))
    }

    /// The semitones one key of a channel is retuned by, from GS NRPN 18H.
    ///
    /// Zero away from the drum channel, where the font tunes the keys itself.
    pub fn channel_key_tune(&self, channel: u8, key: u8) -> f32 {
        self.synth
            .get_channel_key_tune(i32::from(channel), i32::from(key))
    }
}

/// MIDI status bytes, as `process_midi_message` expects them.
const CONTROL_CHANGE: i32 = 0xB0;
const PROGRAM_CHANGE: i32 = 0xC0;
/// Channel pressure takes its value in `data1` and ignores `data2`, unlike poly pressure (`0xA0`),
/// which puts the key there.
const CHANNEL_PRESSURE: i32 = 0xD0;
const PITCH_BEND: i32 = 0xE0;

impl MidiSink for SoundFontSource {
    fn note_on(&mut self, channel: u8, key: u8, velocity: u8) {
        self.synth
            .note_on(i32::from(channel), i32::from(key), i32::from(velocity));
    }

    fn note_off(&mut self, channel: u8, key: u8) {
        self.synth.note_off(i32::from(channel), i32::from(key));
    }

    fn control_change(&mut self, channel: u8, controller: u8, value: u8) {
        self.synth.process_midi_message(
            i32::from(channel),
            CONTROL_CHANGE,
            i32::from(controller),
            i32::from(value),
        );
    }

    fn program_change(&mut self, channel: u8, program: u8) {
        self.synth
            .process_midi_message(i32::from(channel), PROGRAM_CHANGE, i32::from(program), 0);
    }

    fn pitch_bend(&mut self, channel: u8, value: u16) {
        // Split back into the two 7-bit halves the wire format uses.
        let lsb = i32::from(value & 0x7F);
        let msb = i32::from((value >> 7) & 0x7F);
        self.synth
            .process_midi_message(i32::from(channel), PITCH_BEND, lsb, msb);
    }

    fn channel_aftertouch(&mut self, channel: u8, value: u8) {
        self.synth
            .process_midi_message(i32::from(channel), CHANNEL_PRESSURE, i32::from(value), 0);
    }

    fn all_notes_off(&mut self) {
        // Immediate, not released: a seek or a skip must not leave the previous song audible.
        self.synth.note_off_all(true);
    }

    fn reset(&mut self) {
        // Clears the voices, returns every channel to General MIDI defaults, and mutes the reverb
        // and chorus tails -- which is why this is not what a pause calls.
        self.synth.reset();
    }
}

impl AudioSource for SoundFontSource {
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.synth.render(left, right);
    }

    fn block_size(&self) -> usize {
        self.synth.get_block_size()
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Frames rendered per block by [`TestToneSource`], matching rustysynth's default.
const TEST_BLOCK_SIZE: usize = 64;

/// Simultaneous notes [`TestToneSource`] can sound.
const TEST_VOICES: usize = 32;

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    channel: u8,
    key: u8,
    phase: f32,
    increment: f32,
    amplitude: f32,
    active: bool,
}

/// A dependency-free sine synthesizer for tests.
///
/// Not a musical instrument: it exists so the render path can be exercised where no SoundFont is
/// available. It is deliberately simple and deterministic, so a test can assert on exact output.
pub struct TestToneSource {
    sample_rate: u32,
    voices: [Voice; TEST_VOICES],
    master_volume: f32,
    /// Counted so a test can tell a reset from a plain silence, which is otherwise unobservable
    /// here: this source holds no channel state for a reset to clear.
    resets: usize,
}

impl TestToneSource {
    /// Creates a source at the given sample rate.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            voices: [Voice::default(); TEST_VOICES],
            master_volume: 0.5,
            resets: 0,
        }
    }

    /// Number of notes currently sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// How many times [`MidiSink::reset`] has been called.
    pub fn resets(&self) -> usize {
        self.resets
    }

    fn frequency(key: u8) -> f32 {
        // A4 = MIDI 69 = 440 Hz.
        440.0 * 2.0f32.powf((f32::from(key) - 69.0) / 12.0)
    }
}

impl MidiSink for TestToneSource {
    fn note_on(&mut self, channel: u8, key: u8, velocity: u8) {
        let increment = Self::frequency(key) / self.sample_rate as f32;
        let amplitude = f32::from(velocity) / 127.0;
        if let Some(voice) = self.voices.iter_mut().find(|v| !v.active) {
            *voice = Voice {
                channel,
                key,
                phase: 0.0,
                increment,
                amplitude,
                active: true,
            };
        }
    }

    fn note_off(&mut self, channel: u8, key: u8) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.active && v.channel == channel && v.key == key)
        {
            voice.active = false;
        }
    }

    fn control_change(&mut self, _channel: u8, _controller: u8, _value: u8) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: u16) {}
    fn channel_aftertouch(&mut self, _channel: u8, _value: u8) {}

    fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
        }
    }

    fn reset(&mut self) {
        // It holds no channel state to restore, so this is a silence plus a counter.
        self.resets += 1;
        self.all_notes_off();
    }
}

impl AudioSource for TestToneSource {
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
        let scale = self.master_volume / TEST_VOICES as f32;
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let sample = (voice.phase * std::f32::consts::TAU).sin() * voice.amplitude * scale;
                *l += sample;
                *r += sample;
                voice.phase = (voice.phase + voice.increment).fract();
            }
        }
    }

    fn block_size(&self) -> usize {
        TEST_BLOCK_SIZE
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

impl std::fmt::Debug for SoundFontSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundFontSource")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_source_renders_silence() {
        let mut source = TestToneSource::new(44_100);
        let mut left = [1.0f32; TEST_BLOCK_SIZE];
        let mut right = [1.0f32; TEST_BLOCK_SIZE];
        source.render(&mut left, &mut right);
        assert!(left.iter().all(|&s| s == 0.0), "no notes means no sound");
        assert!(right.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn a_sounding_note_produces_audio() {
        let mut source = TestToneSource::new(44_100);
        source.note_on(0, 69, 100);
        let mut left = [0.0f32; TEST_BLOCK_SIZE];
        let mut right = [0.0f32; TEST_BLOCK_SIZE];
        source.render(&mut left, &mut right);
        assert!(
            left.iter().any(|&s| s != 0.0),
            "a held note should be audible"
        );
        assert_eq!(left, right, "the test source is centered");
    }

    #[test]
    fn a_note_off_silences_only_that_note() {
        let mut source = TestToneSource::new(44_100);
        source.note_on(0, 60, 100);
        source.note_on(0, 64, 100);
        assert_eq!(source.active_voices(), 2);
        source.note_off(0, 60);
        assert_eq!(source.active_voices(), 1);
    }

    #[test]
    fn all_notes_off_silences_everything() {
        let mut source = TestToneSource::new(44_100);
        for key in 60..70 {
            source.note_on(0, key, 100);
        }
        assert!(source.active_voices() > 0);
        source.all_notes_off();
        assert_eq!(source.active_voices(), 0);
    }

    #[test]
    fn output_stays_within_range_even_when_saturated() {
        let mut source = TestToneSource::new(44_100);
        // More notes than voices, all at full velocity.
        for key in 40..100 {
            source.note_on(0, key, 127);
        }
        let mut left = [0.0f32; TEST_BLOCK_SIZE];
        let mut right = [0.0f32; TEST_BLOCK_SIZE];
        source.render(&mut left, &mut right);
        assert!(
            left.iter().all(|s| s.abs() <= 1.0),
            "the mix must not exceed full scale"
        );
    }

    #[test]
    fn voices_beyond_the_limit_are_dropped_rather_than_overwriting() {
        let mut source = TestToneSource::new(44_100);
        for key in 0..(TEST_VOICES as u8 + 10) {
            source.note_on(0, key, 100);
        }
        assert_eq!(source.active_voices(), TEST_VOICES);
    }

    #[test]
    fn a_missing_soundfont_is_an_error_not_a_panic() {
        let result = SoundFontSource::from_path("definitely/not/here.sf2", 44_100);
        assert!(matches!(result, Err(SourceError::Io { .. })));
    }

    #[test]
    fn a_file_that_is_not_a_soundfont_is_rejected() {
        let dir = std::env::temp_dir().join("km-audio-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("not-a-soundfont.sf2");
        std::fs::write(&path, b"this is not a SoundFont").expect("write");
        let result = SoundFontSource::from_path(&path, 44_100);
        assert!(
            matches!(result, Err(SourceError::SoundFont(_))),
            "got {result:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The ordinary case, and the one that must not produce a message anybody reads.
    #[test]
    fn a_bank_with_nothing_dropped_is_empty() {
        let defects = BankDefects::default();
        assert!(defects.is_empty());
        assert_eq!(defects.dropped(), 0);
        assert_eq!(defects.to_string(), "no defective records");
    }

    #[test]
    fn one_dropped_record_is_singular() {
        let defects = BankDefects::new(1, &["instrument 12 region 3: loop end past wave data"]);
        assert_eq!(
            defects.to_string(),
            "1 defective record dropped: instrument 12 region 3: loop end past wave data"
        );
    }

    /// Every consumer puts this in one line, so the examples are capped and the rest counted.
    ///
    /// The remainder is computed against `dropped` rather than against the examples' own cap, so it
    /// stays right when the synthesizer has already truncated at its own 64 — which is the case this
    /// got wrong first.
    #[test]
    fn more_dropped_records_than_are_quoted_are_counted() {
        let defects = BankDefects::new(37, &["first", "second", "third"]);
        assert_eq!(
            defects.to_string(),
            "37 defective records dropped: first; second; third; and 34 more"
        );
        assert!(!defects.is_empty());
        assert_eq!(defects.dropped(), 37);
        assert_eq!(defects.examples().len(), 3);
    }

    /// A bank so defective the synthesizer stopped keeping examples: 500 dropped, 64 retained, of
    /// which three are quoted, so the tail is 497 and not 61.
    #[test]
    fn the_counted_remainder_ignores_the_synthesizers_own_cap() {
        let defects = BankDefects::new(500, &["a", "b", "c"]);
        assert!(
            defects.to_string().ends_with("and 497 more"),
            "got {defects}"
        );
    }
}
