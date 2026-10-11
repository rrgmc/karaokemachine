//! The candidate stretchers, each behind one function.
//!
//! A key change and a tempo change are one operation here. A stretcher changes the length of the
//! audio and keeps its pitch. A resampler changes both together. So a song at tempo `T` and pitch
//! ratio `P` is the audio stretched to rate `T / P` and then resampled by `P`, and one stretcher
//! serves both controls.

use timestretch::StretchParams;
use timestretch::engine::{Engine as Realtime, EngineConfig, EngineProfile};

/// Channels in every buffer here. Interleaved stereo, as a feed carries it.
const CHANNELS: usize = 2;

/// Frames a real-time engine renders per call, which is the block the machine's player renders.
const BLOCK_FRAMES: usize = 64;

/// Frames given to a streaming stretcher per push.
const PUSH_FRAMES: usize = 4096;

/// One candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// The `wsola` crate, which works in the time domain.
    Wsola,
    /// `timestretch`'s real-time engine with its time-domain corrector.
    Keylock,
    /// `timestretch`'s real-time engine with its phase vocoder.
    Wide,
    /// `timestretch`'s whole-file functions, which no audio callback can call.
    Batch,
}

/// The frequency ratio of a key change in semitones.
pub fn pitch_ratio(semitones: i8) -> f64 {
    2f64.powf(f64::from(semitones) / 12.0)
}

/// Renders interleaved stereo `input` at another key and tempo.
///
/// Returns the samples and the delay the engine reports, in frames. The delay is already removed
/// from the samples.
pub fn render(
    engine: Engine,
    input: &[f32],
    sample_rate: u32,
    semitones: i8,
    tempo: f32,
) -> Result<(Vec<f32>, usize), Box<dyn std::error::Error>> {
    let pitch = pitch_ratio(semitones);
    let rate = f64::from(tempo) / pitch;
    match engine {
        Engine::Wsola => {
            let stretched = wsola_stretch(input, sample_rate, rate)?;
            Ok((resample(&stretched, pitch), 0))
        }
        Engine::Keylock => {
            let (stretched, delay) =
                realtime_stretch(EngineProfile::Keylock, input, sample_rate, rate)?;
            Ok((resample(&stretched, pitch), delay))
        }
        Engine::Wide => {
            let (stretched, delay) =
                realtime_stretch(EngineProfile::WideKeylock, input, sample_rate, rate)?;
            Ok((resample(&stretched, pitch), delay))
        }
        Engine::Batch => {
            let params = StretchParams::new(1.0 / f64::from(tempo))
                .with_sample_rate(sample_rate)
                .with_channels(CHANNELS as u32);
            let stretched = timestretch::stretch(input, &params)?;
            let shifted = if semitones == 0 {
                stretched
            } else {
                timestretch::pitch_shift(&stretched, &params, pitch)?
            };
            Ok((shifted, 0))
        }
    }
}

/// Stretches to `rate` times the speed through the `wsola` crate's streaming calls.
fn wsola_stretch(
    input: &[f32],
    sample_rate: u32,
    rate: f64,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let mut stretcher = wsola::TimeStretch::new(sample_rate, CHANNELS as u16)?;
    stretcher.set_tempo(rate as f32);
    let mut out = Vec::with_capacity((input.len() as f64 / rate) as usize + PUSH_FRAMES);
    for chunk in input.chunks(PUSH_FRAMES * CHANNELS) {
        stretcher.push(chunk);
        out.extend(stretcher.pull(usize::MAX));
    }
    out.extend(stretcher.flush());
    Ok(out)
}

/// Stretches to `rate` times the speed through `timestretch`'s real-time engine.
///
/// The loop is the shape an audio callback has: a feed side that keeps a ring full, and a render
/// side that asks for one small block at a time.
fn realtime_stretch(
    profile: EngineProfile,
    input: &[f32],
    sample_rate: u32,
    rate: f64,
) -> Result<(Vec<f32>, usize), Box<dyn std::error::Error>> {
    let handles = Realtime::build(EngineConfig {
        sample_rate,
        channels: CHANNELS,
        profile,
        initial_tempo_rate: rate,
        max_block_frames: BLOCK_FRAMES,
        ..EngineConfig::default()
    })?;
    let (mut processor, mut source) = (handles.processor, handles.source);

    let delay = processor.pipeline_latency_frames();
    let wanted = (input.len() / CHANNELS) as f64 / rate;
    let wanted = (wanted.round() as usize + delay) * CHANNELS;

    let mut out = Vec::with_capacity(wanted + BLOCK_FRAMES * CHANNELS);
    let mut block = [0.0f32; BLOCK_FRAMES * CHANNELS];
    let mut cursor = 0;
    let mut finished = false;
    while out.len() < wanted {
        if cursor < input.len() {
            cursor += source.push(&input[cursor..]) * CHANNELS;
        } else if !finished {
            finished = source.finish();
        }
        processor.process(&mut block);
        out.extend_from_slice(&block);
    }
    out.truncate(wanted);
    out.drain(..delay * CHANNELS);
    Ok((out, delay))
}

/// Resamples by linear interpolation, reading `step` input frames per output frame.
///
/// Linear because the machine's player resamples that way, so a render says what the machine would
/// play.
fn resample(input: &[f32], step: f64) -> Vec<f32> {
    let frames = input.len() / CHANNELS;
    if frames == 0 || (step - 1.0).abs() < 1e-9 {
        return input.to_vec();
    }
    let out_frames = (frames as f64 / step) as usize;
    let mut out = Vec::with_capacity(out_frames * CHANNELS);
    for i in 0..out_frames {
        let position = i as f64 * step;
        let a = (position as usize).min(frames - 1);
        let b = (a + 1).min(frames - 1);
        let t = (position - a as f64) as f32;
        for channel in 0..CHANNELS {
            let from = input[a * CHANNELS + channel];
            let to = input[b * CHANNELS + channel];
            out.push(from + (to - from) * t);
        }
    }
    out
}
