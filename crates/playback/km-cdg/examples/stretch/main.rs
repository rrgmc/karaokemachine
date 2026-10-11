//! Renders a recording at another key or tempo and writes it to a WAV.
//!
//! A recorded song has no notes to move, so changing its key or tempo means processing its audio.
//! This renders one file through one candidate stretcher, so that somebody can hear the result and
//! read what it cost.
//!
//! ```sh
//! cargo run --release -p km-cdg --example stretch -- song.mp3 out.wav --engine wide --key -3
//! cargo run --release -p km-cdg --example stretch -- song.mp3 out.wav --engine wsola --tempo 1.1
//! ```
//!
//! The decoder is the one an MP3+G, UltraStar or LRC song plays through. The key and tempo limits
//! are the machine's own.

mod engines;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use km_audio::{Rendered, TrackPlayer, write_wav};
use km_queue::{MAX_TEMPO_RATIO, MAX_TRANSPOSE, MIN_TEMPO_RATIO};

use crate::engines::Engine;

const USAGE: &str = "usage: stretch <song.mp3> <out.wav> --engine <name> [options]

engines:
  wsola     the wsola crate, in the time domain
  keylock   timestretch's real-time engine, time-domain corrector
  wide      timestretch's real-time engine, phase vocoder
  batch     timestretch's whole-file functions, for comparison only

options:
  --key <-6..6>          shift the key, in semitones
  --tempo <0.75..1.25>   playback speed, as a multiple of the recorded tempo
  --max-ms <ms>          render only this much of the song
";

/// Frames decoded per call.
const BLOCK: usize = 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut engine: Option<Engine> = None;
    let mut key = 0i8;
    let mut tempo = 1.0f32;
    let mut max_ms: Option<u64> = None;

    while let Some(arg) = args.next() {
        let name = arg.to_string_lossy().into_owned();
        let mut value = || {
            args.next()
                .map(|value| value.to_string_lossy().into_owned())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "--engine" => {
                engine = Some(match value()?.as_str() {
                    "wsola" => Engine::Wsola,
                    "keylock" => Engine::Keylock,
                    "wide" => Engine::Wide,
                    "batch" => Engine::Batch,
                    other => return Err(format!("unknown engine {other}").into()),
                });
            }
            "--key" => key = value()?.parse()?,
            "--tempo" => tempo = value()?.parse()?,
            "--max-ms" => max_ms = Some(value()?.parse()?),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}").into());
            }
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let ([source, output], Some(engine)) = (positional.as_slice(), engine) else {
        return Err(USAGE.into());
    };
    // Clamped here so the printed summary reports what is used.
    let key = key.clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE);
    let tempo = tempo.clamp(MIN_TEMPO_RATIO, MAX_TEMPO_RATIO);

    let (input, sample_rate) = decode(source, max_ms)?;
    let in_frames = input.len() / 2;
    let in_seconds = in_frames as f64 / f64::from(sample_rate);

    let started = Instant::now();
    let (samples, delay) = engines::render(engine, &input, sample_rate, key, tempo)?;
    let elapsed = started.elapsed().as_secs_f64();

    let out_seconds = (samples.len() / 2) as f64 / f64::from(sample_rate);
    let expected = in_seconds / f64::from(tempo);
    println!(
        "{engine:?} key {key:+} tempo {tempo:.2}: {in_seconds:.2}s in, {out_seconds:.2}s out, \
         {expected:.2}s expected"
    );
    println!(
        "  rendered in {elapsed:.2}s, {:.2}% of real time, delay {:.1} ms",
        elapsed / out_seconds.max(f64::EPSILON) * 100.0,
        delay as f64 * 1000.0 / f64::from(sample_rate)
    );

    write_wav(
        output,
        &Rendered {
            samples,
            sample_rate,
            reached_end: max_ms.is_none(),
        },
    )?;
    println!("  {}", output.display());
    Ok(())
}

/// Decodes a whole file to interleaved stereo at the file's own rate.
///
/// It goes through a real feed and a real `TrackPlayer` at the file's rate, where the player's
/// resampler passes samples through unchanged.
fn decode(
    source: &PathBuf,
    max_ms: Option<u64>,
) -> Result<(Vec<f32>, u32), Box<dyn std::error::Error>> {
    let name = source.display().to_string();
    let file = std::fs::File::open(source)?;
    let (_reader, feed) = km_cdg::open_audio_from(file, &name)?;
    let sample_rate = feed.sample_rate();
    let mut player = TrackPlayer::new(feed, sample_rate);
    let limit = max_ms.map(|ms| ms * u64::from(sample_rate) / 1000 * 2);

    let mut samples: Vec<f32> = Vec::new();
    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    while !player.is_finished() && limit.is_none_or(|limit| (samples.len() as u64) < limit) {
        // A block the decoder left short ends in silence the song does not hold. The player counts
        // those frames, so the count says how much of the block is the song.
        let starved_before = player.starved_frames();
        player.render(&mut left, &mut right, true);
        let starved = (player.starved_frames() - starved_before) as usize;
        let filled = BLOCK.saturating_sub(starved);
        for (l, r) in left[..filled].iter().zip(&right[..filled]) {
            samples.push(*l);
            samples.push(*r);
        }
        if starved > 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok((samples, sample_rate))
}
