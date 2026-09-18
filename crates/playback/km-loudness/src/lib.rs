//! Measuring how loud a song is, and deriving the gain that levels it against its bank's own level.
//!
//! **The metering here runs when a package is built, never during playback.** The two gain rules are
//! arithmetic over a couple of numbers and are called once per song as it starts: [`gain_for`] brings
//! a measured media song down, and [`midi_gain`] moves a MIDI song either way from the estimate
//! `km_song` reads out of its events.
//!
//! **The two kinds get two rules because their headroom differs.** A media song is a finished master
//! with none to give, so it can only come down. A MIDI song is this machine's own synthesizer output,
//! and a quiet one is quiet because it is sparse rather than compressed, so its peak falls with its
//! loudness and the room to raise it comes free. See `docs/research/midi-loudness.md`.
//!
//! ## Why this exists
//!
//! A MIDI song plays through this machine's own synthesizer at whatever the bank renders it at; the
//! bundled bank measures around −22 LUFS (`docs/architecture/audio.md`). A video or MP3+G song plays
//! whatever its publisher mastered, and karaoke media is mastered loud — between about −14 and −9
//! LUFS. So the same machine at one setting of one amplifier plays a MIDI song and a video song 8 to
//! 13 dB apart, and moves within that range from one video to the next because every publisher
//! masters differently.
//!
//! That is the defect the bundled bank was chosen to avoid, one layer further out: a karaoke machine
//! has its music-to-microphone balance set once, in hardware, and then plays a hundred songs at it.
//! See the `Video and MP3+G play at the MIDI reference level` decision in `docs/decisions/audio.md`.
//!
//! ## The meter is a port of the one the measurements were taken with
//!
//! `ebur128` is a pure-Rust port of libebur128, which is what ffmpeg's own `ebur128` filter is built
//! on — and that filter is what produced every LUFS figure in `docs/architecture/audio.md` and what
//! `tools/dev/soundfont-measure.sh` still runs. So the numbers this crate reports and the numbers
//! this project already wrote down are the same measurement rather than two that ought to agree.
//!
//! **libavfilter is not an option here even though ffmpeg is linked.** `tools/setup/ffmpeg-pin.sh`
//! configures with `--disable-avfilter`, so `ebur128` the *filter* is absent from the libraries this
//! workspace links and from every carrier it ships. The system `ffmpeg` binary a developer has is
//! what the shell script uses, and a packaging step cannot depend on one being installed.

/// Loudness as EBU R128 measures it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loudness {
    /// Integrated loudness over the whole file, in LUFS. Negative for anything but a divergence.
    pub lufs: f32,
    /// The loudest true peak in any channel, in dBTP. `0.0` is full scale.
    ///
    /// **Recorded although nothing reads it yet**, and deliberately. It is four bytes a song, and it
    /// is the only number that could ever license a gain *above*
    /// 1.0 — which is the first thing somebody will ask for once media stops being too loud. Having
    /// measured it means that question gets answered with a measurement instead of reopened with a
    /// guess. See `Deliberately out of scope` in the decision.
    pub peak_dbtp: f32,
}

/// The reference level for a bank the table has no measurement for, in LUFS.
///
/// **The bundled bank's own figure is not this constant**, and must not be: it lives in
/// `soundfont-banks.conf` beside the bank's `volume` and `spread`, measured by
/// `tools/dev/soundfont-measure.sh` through the synthesizer this machine actually renders with. This
/// is the fallback for a bank nobody has measured — somebody's own `.sf2` dropped into the
/// SoundFont folder — where the honest answer is a level in the right region rather than no
/// levelling at all.
///
/// It sits at the quiet end of the surveyed banks rather than at their mean. The surveyed range runs
/// from about −18 to −22 LUFS, and erring quiet errs towards attenuating a media song slightly too
/// much, which sounds like a machine that wants its amplifier up; erring loud leaves the media song
/// louder than the MIDI one, which is the complaint this exists to answer.
pub const DEFAULT_REFERENCE_LUFS: f32 = -22.0;

/// The most a song may be attenuated: −20 dB.
///
/// **A guard against a wrong measurement, not a taste.** R128's gating already stops a mostly-silent
/// recording from measuring quiet — blocks under the absolute threshold are not counted — so nothing
/// in the real corpus derives a gain this small. A file that does has been measured wrongly, and
/// muting it would turn a bad number into a silent song in front of a room.
pub const MIN_GAIN: f32 = 0.1;

/// How much audio R128 needs before it can integrate anything, in milliseconds.
///
/// One gating block. Shorter than this and [`Meter::finish`] answers `None` rather than a number
/// derived from less than the standard measures over.
pub const MIN_MEASURABLE_MS: u32 = 400;

/// The gain that brings a song measured at `song_lufs` to `reference_lufs`.
///
/// **Attenuation only, and that is forced rather than chosen.** `Player`'s music volume clamps at
/// `1.0` and `rustysynth`'s master volume is deliberately left at its hardcoded 0.5, because raising
/// it would invalidate every loudness figure the bundled bank was chosen against. MIDI therefore
/// cannot be turned up, so media comes down: a song already at or below the reference is left
/// exactly alone, and no gain this returns can clip.
///
/// Clamped to [`MIN_GAIN`] at the bottom for the reason given there. A non-finite input — which is
/// what a meter reports for silence — returns `1.0`, because the only safe reading of "there is no
/// measurement" is "do not touch it".
#[must_use]
pub fn gain_for(reference_lufs: f32, song_lufs: f32) -> f32 {
    if !reference_lufs.is_finite() || !song_lufs.is_finite() {
        return 1.0;
    }
    let gain = 10.0_f32.powf((reference_lufs - song_lufs) / 20.0);
    if gain.is_finite() {
        gain.clamp(MIN_GAIN, 1.0)
    } else {
        1.0
    }
}

/// The corpus mean of `km_song`'s event-based estimate, in that estimate's own decibels.
///
/// **The one number that turns a figure on an arbitrary scale into a level.** Measured over 989
/// corpus songs at −1.75 dB with a standard deviation of 4.54; the same sample renders at −22.80
/// LUFS through the recommended bank and −23.18 through the bundled one.
///
/// **Any change to `km_song::loudness` moves this**, because it is that code's own output over a
/// fixed sample. `cargo run --release -p km-audio --example loudness_census -- --refit <rows>` prints
/// the mean beside this constant, over rows a previous rendering run wrote, so re-deriving it costs
/// no render. See `docs/research/midi-loudness.md`.
pub const MIDI_REFERENCE_ESTIMATE: f32 = -1.8;

/// The most a MIDI song may be raised: +12 dB.
///
/// **The complaint this answers is a song that plays too low**, and the songs that need answering
/// most are 12.1 dB below their bank's mean on average, so a smaller cap leaves them half fixed.
/// Measured over the corpus, levelling at this cap puts 2% of songs marginally past −1 dBTP against
/// the 10% already past it, because the rule that raises a quiet song lowers a loud one.
pub const MAX_MIDI_GAIN: f32 = 3.98;

/// The gain that brings a MIDI song whose estimate is `estimated_db` to its bank's own level.
///
/// **The bank cancels out, which is why this takes no reference.** A song's estimate predicts its
/// rendered level as `bank_mean + (estimated_db − MIDI_REFERENCE_ESTIMATE)`, and the target is
/// `bank_mean`, so the bank appears on both sides and the gain is the song's distance from the corpus
/// mean and nothing else. A slope of 1 is measured rather than assumed: the fits across six banks
/// bracket it from 0.76 to 1.13.
///
/// **This one boosts, where [`gain_for`] cannot.** A media song is a finished master with no headroom
/// to give, and a MIDI song is a synthesizer's output whose peak falls with its loudness: measured
/// over the corpus, 98% or more of the quiet songs have two to three times the headroom they need. So
/// the two kinds get two rules.
///
/// Clamped to [`MIN_GAIN`] and [`MAX_MIDI_GAIN`], and a non-finite input returns `1.0`, because the
/// only safe reading of "there is no estimate" is "do not touch it".
#[must_use]
pub fn midi_gain(estimated_db: f32) -> f32 {
    if !estimated_db.is_finite() {
        return 1.0;
    }
    let gain = 10.0_f32.powf((MIDI_REFERENCE_ESTIMATE - estimated_db) / 20.0);
    if gain.is_finite() {
        gain.clamp(MIN_GAIN, MAX_MIDI_GAIN)
    } else {
        1.0
    }
}

/// Feeds interleaved stereo samples to an R128 meter.
///
/// Built for the shape both decoders already produce: `km-cdg`'s `append` and `km-video`'s
/// `append_samples` each hand over interleaved stereo `f32` at the file's own rate, which is exactly
/// what the machine plays. Nothing here resamples, and nothing here goes through a `TrackPlayer` —
/// see `Judging it by ear, and one trap in doing so` in `docs/architecture/cdg.md` for why driving a
/// player faster than real time measures silence.
pub struct Meter {
    inner: ebur128::EbuR128,
    frames: u64,
    rate: u32,
}

impl Meter {
    /// A meter for interleaved stereo at `rate`.
    ///
    /// `None` for a rate the meter will not accept, which is the honest answer for a file whose
    /// header says something impossible — a caller reports it unmeasured rather than guessing a
    /// rate and writing down a number for the wrong one.
    #[must_use]
    pub fn stereo(rate: u32) -> Option<Self> {
        // `I` for the integrated loudness and `TRUE_PEAK` for the peak. Deliberately not `LRA` or
        // `S`: the crate's own advice is to ask for the lowest modes that suit, and loudness range
        // is a number nothing here has a use for.
        let inner =
            ebur128::EbuR128::new(2, rate, ebur128::Mode::I | ebur128::Mode::TRUE_PEAK).ok()?;
        Some(Self {
            inner,
            frames: 0,
            rate,
        })
    }

    /// Adds interleaved stereo frames.
    ///
    /// A partial frame at the end of `samples` is dropped rather than padded — it can only arrive
    /// from a decoder that has produced half a frame, and inventing the other half would put a
    /// sample the file does not contain into the measurement.
    pub fn add(&mut self, samples: &[f32]) {
        let whole = samples.len() - samples.len() % 2;
        if whole == 0 {
            return;
        }
        if self.inner.add_frames_f32(&samples[..whole]).is_ok() {
            self.frames += whole as u64 / 2;
        }
    }

    /// How much audio has been measured, in milliseconds.
    #[must_use]
    pub fn measured_ms(&self) -> u64 {
        if self.rate == 0 {
            return 0;
        }
        self.frames * 1000 / u64::from(self.rate)
    }

    /// The measurement, or `None` when there is not enough audio to make one.
    ///
    /// `None` covers three cases that are one answer: too little audio to fill a gating block,
    /// silence, and a recording every block of which fell under R128's absolute threshold. The meter
    /// reports all three as negative infinity, and a caller writes no record — which leaves the song
    /// at gain `1.0`, playing exactly as it does today.
    #[must_use]
    pub fn finish(&self) -> Option<Loudness> {
        if self.measured_ms() < u64::from(MIN_MEASURABLE_MS) {
            return None;
        }
        let lufs = self.inner.loudness_global().ok()?;
        if !lufs.is_finite() {
            return None;
        }

        // libebur128 reports a peak as a linear amplitude, per channel; dBTP is 20*log10 of it. A
        // silent channel is 0.0 and would take the log to negative infinity, so it is floored at the
        // quietest peak worth writing down rather than allowed to poison the record.
        let peak = (0..self.inner.channels())
            .filter_map(|channel| self.inner.true_peak(channel).ok())
            .fold(0.0_f64, f64::max);
        let peak_dbtp = if peak > 0.0 {
            20.0 * peak.log10()
        } else {
            -f64::from(120)
        };

        Some(Loudness {
            lufs: lufs as f32,
            peak_dbtp: peak_dbtp as f32,
        })
    }
}

impl std::fmt::Debug for Meter {
    /// Hand-written because `ebur128::EbuR128` is not `Debug`, and a meter inside a struct that is
    /// derives nothing without this.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meter")
            .field("rate", &self.rate)
            .field("frames", &self.frames)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `amplitude`, `ms` long, interleaved stereo at `rate`.
    fn sine(rate: u32, ms: u32, hz: f32, amplitude: f32) -> Vec<f32> {
        let frames = (rate as u64 * u64::from(ms) / 1000) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let t = frame as f32 / rate as f32;
            let value = amplitude * (std::f32::consts::TAU * hz * t).sin();
            out.push(value);
            out.push(value);
        }
        out
    }

    fn measure(samples: &[f32], rate: u32) -> Option<Loudness> {
        let mut meter = Meter::stereo(rate).expect("48 kHz stereo is a shape the meter takes");
        meter.add(samples);
        meter.finish()
    }

    /// The one absolute figure, and where it comes from.
    ///
    /// **Measured, not derived.** The first version of this test asserted a number worked out on
    /// paper and was wrong by 8 dB, which is the argument for the test existing in this shape: the
    /// bytes this function builds were written out as raw `f32le` and put through the same command
    /// `tools/dev/soundfont-measure.sh` uses, and what that command said is what this asserts.
    ///
    /// ```text
    /// ffmpeg -hide_banner -nostats -f f32le -ar 48000 -ac 2 -i tone.raw \
    ///        -filter_complex ebur128=peak=true -f null -
    /// #   I: -6.0 LUFS      True peak: -6.0 dBFS
    /// ```
    ///
    /// Asserted to the tenth ffmpeg prints, because agreement with that command is the whole point
    /// and a tighter tolerance would be asserting more than the printed figure can support.
    ///
    /// Two facts about the number, both measured the same way rather than assumed. A half-scale sine
    /// is −9.03 dBFS RMS, and in **one** channel this signal measures **−9.0 LUFS** — so at 1 kHz
    /// the K-weighting and R128's −0.691 dB offset cancel, and a tone there reads its own RMS level.
    /// Across **two** correlated channels it measures −6.0, the expected 3.01 dB above one. Getting
    /// that factor of two backwards is what this test catches.
    #[test]
    fn a_half_scale_1khz_sine_measures_what_ffmpeg_says_it_does() {
        let measured = measure(&sine(48_000, 3_000, 1_000.0, 0.5), 48_000)
            .expect("three seconds is plenty to integrate");
        assert!(
            (measured.lufs - (-6.0)).abs() < 0.1,
            "measured {} LUFS, ffmpeg says -6.0",
            measured.lufs
        );
    }

    /// Doubling the amplitude has to read 6 dB louder, whatever the absolute figures are.
    ///
    /// The relative check is the stronger one for catching a broken meter: an implementation with
    /// the wrong reference level still passes it, and one that has lost its logarithm does not.
    #[test]
    fn six_db_louder_measures_six_lu_louder() {
        let quiet = measure(&sine(48_000, 3_000, 1_000.0, 0.25), 48_000).expect("quiet");
        let loud = measure(&sine(48_000, 3_000, 1_000.0, 0.5), 48_000).expect("loud");
        let difference = loud.lufs - quiet.lufs;
        assert!(
            (difference - 6.02).abs() < 0.1,
            "doubling the amplitude moved it {difference} LU, not 6.02"
        );
    }

    /// A full-scale sine peaks at about 0 dBTP, and the peak is reported in dBTP rather than linear.
    ///
    /// The unit is the thing being asserted. libebur128 hands back a linear amplitude, so a record
    /// that forgot to convert would carry `1.0` here and read as +1 dBTP everywhere it was shown.
    #[test]
    fn the_peak_is_in_dbtp() {
        let full = measure(&sine(48_000, 1_000, 1_000.0, 1.0), 48_000).expect("full scale");
        assert!(
            full.peak_dbtp.abs() < 0.5,
            "a full-scale sine peaked at {} dBTP",
            full.peak_dbtp
        );

        let half = measure(&sine(48_000, 1_000, 1_000.0, 0.5), 48_000).expect("half scale");
        assert!(
            (half.peak_dbtp - (-6.02)).abs() < 0.5,
            "a half-scale sine peaked at {} dBTP, not about -6",
            half.peak_dbtp
        );
    }

    /// Under one gating block there is no measurement, and saying so is the answer.
    #[test]
    fn too_little_audio_is_no_measurement() {
        assert_eq!(measure(&sine(48_000, 100, 1_000.0, 0.5), 48_000), None);
        assert_eq!(measure(&[], 48_000), None);
    }

    /// Silence is not a quiet song. R128 gates it away and this reports nothing.
    #[test]
    fn silence_is_no_measurement() {
        let silent = vec![0.0_f32; 48_000 * 2 * 3];
        assert_eq!(measure(&silent, 48_000), None);
    }

    /// A partial frame is dropped rather than padded, and does not shift the measurement.
    #[test]
    fn a_trailing_half_frame_is_dropped() {
        let mut odd = sine(48_000, 3_000, 1_000.0, 0.5);
        let even = measure(&odd, 48_000).expect("even");
        odd.push(0.9);
        let with_half = measure(&odd, 48_000).expect("odd");
        assert!((even.lufs - with_half.lufs).abs() < 0.001);
        // ...and the dropped sample is not in the peak either, though it is the loudest thing here.
        assert!((even.peak_dbtp - with_half.peak_dbtp).abs() < 0.001);
    }

    /// The gain rule, at both clamps and in between.
    #[test]
    fn the_gain_attenuates_and_never_boosts() {
        // A song 6 dB louder than the reference comes down by half.
        assert!((gain_for(-22.0, -16.0) - 0.501).abs() < 0.01);
        // Level with the reference: left alone.
        assert!((gain_for(-22.0, -22.0) - 1.0).abs() < 0.001);
        // **Quieter than the reference is left alone, not boosted.** This is the clamp that keeps a
        // MIDI-referenced machine from ever raising a media song into clipping.
        assert_eq!(gain_for(-22.0, -30.0), 1.0);
        // A hot master, but a real one: about -5 LUFS is the loudest thing the world ships.
        let hot = gain_for(-22.0, -5.0);
        assert!(
            hot > MIN_GAIN,
            "a real hot master should not reach the floor"
        );
        assert!(hot < 0.2);
    }

    /// The floor holds, and a measurement that is not a number changes nothing.
    #[test]
    fn a_nonsense_measurement_cannot_mute_a_song() {
        assert_eq!(gain_for(-22.0, 100.0), MIN_GAIN);
        assert_eq!(gain_for(-22.0, f32::NAN), 1.0);
        assert_eq!(gain_for(-22.0, f32::NEG_INFINITY), 1.0);
        assert_eq!(gain_for(f32::NAN, -10.0), 1.0);
    }

    /// A MIDI song at the corpus mean is left exactly alone, and the scale is 20·log₁₀.
    #[test]
    fn the_midi_gain_moves_a_song_by_its_distance_from_the_corpus_mean() {
        assert!((midi_gain(MIDI_REFERENCE_ESTIMATE) - 1.0).abs() < 0.001);
        // Six decibels below the mean comes up by two, six above comes down by half.
        assert!((midi_gain(MIDI_REFERENCE_ESTIMATE - 6.02) - 2.0).abs() < 0.01);
        assert!((midi_gain(MIDI_REFERENCE_ESTIMATE + 6.02) - 0.5).abs() < 0.01);
    }

    /// **The rule that answers the complaint**: a quiet song is raised, where a media song never is.
    ///
    /// The two functions are handed the same shape of argument and must disagree, so this pins the
    /// disagreement rather than trusting the two clamps to stay apart.
    #[test]
    fn a_quiet_midi_song_is_raised_where_a_quiet_media_song_is_not() {
        assert!(midi_gain(MIDI_REFERENCE_ESTIMATE - 9.0) > 1.0);
        assert_eq!(gain_for(-22.0, -31.0), 1.0);
    }

    /// Both clamps hold, and a nonsense estimate cannot mute or explode a song.
    #[test]
    fn the_midi_gain_is_clamped_at_both_ends() {
        // Far below the mean: capped at +12 dB rather than raised without limit.
        assert!((midi_gain(MIDI_REFERENCE_ESTIMATE - 40.0) - MAX_MIDI_GAIN).abs() < 0.001);
        // Far above it: the same −20 dB floor a media song has.
        assert!((midi_gain(MIDI_REFERENCE_ESTIMATE + 40.0) - MIN_GAIN).abs() < 0.001);
        assert_eq!(midi_gain(f32::NAN), 1.0);
        assert_eq!(midi_gain(f32::NEG_INFINITY), 1.0);
        assert_eq!(midi_gain(f32::INFINITY), 1.0);
    }

    /// The cap is +12 dB, which is what reaches the songs the feature exists for.
    #[test]
    fn the_cap_is_twelve_decibels() {
        let decibels = 20.0 * MAX_MIDI_GAIN.log10();
        assert!(
            (decibels - 12.0).abs() < 0.05,
            "the cap is {decibels} dB, not 12"
        );
    }

    /// `measured_ms` counts frames rather than samples, which is the factor of two worth pinning.
    #[test]
    fn measured_ms_counts_frames() {
        let mut meter = Meter::stereo(48_000).expect("shape");
        meter.add(&sine(48_000, 1_000, 1_000.0, 0.5));
        assert_eq!(meter.measured_ms(), 1_000);
    }
}
