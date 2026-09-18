//! Decodes an MP3+G song offline and writes the audio to a WAV.
//!
//! The audio counterpart of `examples/preview`, and the same idea: prove the half that needs a sound
//! card without needing one. It drives a real `km_audio::TrackPlayer` over a real feed, so what it
//! writes has been through the exact path the machine plays through — the ring, the seek protocol
//! and the resampler included.
//!
//! With a `.cdg` beside the `.mp3` it also reports how the two line up, which is the check that
//! catches a graphics file paired with the wrong song.
//!
//! ```sh
//! cargo run --release -p km-cdg --example decode -- song.mp3 out.wav
//! cargo run --release -p km-cdg --example decode -- song.mp3 out.wav --rate 44100 --seek 60
//! ```

use std::path::PathBuf;
use std::time::Instant;

use km_audio::{Rendered, TrackPlayer, write_wav};

/// Frames rendered per call. Arbitrary; large enough that the loop is not the cost.
const BLOCK: usize = 4096;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut rate = 48_000u32;
    let mut seek_s: Option<u32> = None;

    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--rate" => {
                rate = args
                    .next()
                    .ok_or("--rate needs a number")?
                    .to_string_lossy()
                    .parse()?;
            }
            "--seek" => {
                seek_s = Some(
                    args.next()
                        .ok_or("--seek needs a number of seconds")?
                        .to_string_lossy()
                        .parse()?,
                );
            }
            "-h" | "--help" => {
                eprintln!("usage: decode <song.mp3> <out.wav> [--rate 48000] [--seek SECONDS]");
                return Ok(());
            }
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [source, output] = positional.as_slice() else {
        return Err("usage: decode <song.mp3> <out.wav> [--rate 48000] [--seek SECONDS]".into());
    };

    let info = km_cdg::probe_audio(source)?;
    println!("{}", source.display());
    println!(
        "  {} {} Hz, {} channel(s), {:.1}s",
        info.codec,
        info.sample_rate,
        info.channels,
        f64::from(info.duration_ms) / 1000.0
    );
    match (&info.title, &info.artist) {
        (None, None) => println!("  no usable tags — the file stem is what packaging will use"),
        (title, artist) => println!(
            "  tags say title {:?}, artist {:?} (offered, not trusted)",
            title.as_deref().unwrap_or("-"),
            artist.as_deref().unwrap_or("-")
        ),
    }

    // The graphics half, when the pair is on disk. Only ever a report here; the words are not in the
    // WAV and this is what tells a mispaired `.cdg` from a normal short one.
    let graphics = source.with_extension("cdg");
    if graphics.is_file() {
        let stats = km_cdg::read_graphics(&graphics)?.stats();
        let short_by = info.duration_ms.saturating_sub(stats.duration_ms);
        println!(
            "  graphics: {} tiles over {:.1}s, stopping {:.1}s before the audio{}",
            stats.tiles_written,
            f64::from(stats.duration_ms) / 1000.0,
            f64::from(short_by) / 1000.0,
            if short_by > 60_000 {
                "  <-- over a minute: suspect a mispairing"
            } else {
                ""
            }
        );
    }

    let started = Instant::now();
    let (_reader, feed, _frames) = km_cdg::open(source, &graphics)?;
    let mut player = TrackPlayer::new(feed, rate);

    // Let the ring fill before pulling. Without it the first blocks are rendered against an empty
    // feed, which the player correctly answers with silence — right for a live machine that must not
    // stutter, wrong for an offline render that can simply wait.
    std::thread::sleep(std::time::Duration::from_millis(300));

    if let Some(seconds) = seek_s {
        player.seek_ms(seconds * 1000);
        std::thread::sleep(std::time::Duration::from_millis(300));
        println!("  seeking to {seconds}s before rendering");
    }

    let mut samples: Vec<f32> = Vec::new();
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let limit = (u64::from(info.duration_ms) + 5_000) * u64::from(rate) / 1000;

    // **Paced, and it has to be.** On the machine the audio callback pulls at exactly real time and
    // the decoder is a hundred times faster, so the ring never runs dry. A loop that pulls flat out
    // wins that race instead, and the player answers an empty feed the only way it can — silence,
    // with the position frozen. That is right for a live machine and produces a WAV full of gaps
    // here, so this yields a fraction of each block's own duration and lets the decoder stay ahead.
    // Measured: unpaced, a six-minute song came out five seconds long with 466,697 starved frames.
    let block_ms = (BLOCK as u64 * 1000 / u64::from(rate)).max(1);
    let nap = std::time::Duration::from_millis((block_ms / 8).max(1));

    while !player.is_finished() && (samples.len() as u64) < limit * 2 {
        player.render(&mut left, &mut right, true);
        for (l, r) in left.iter().zip(&right) {
            samples.push(*l);
            samples.push(*r);
        }
        std::thread::sleep(nap);
    }

    let rendered = Rendered {
        sample_rate: rate,
        reached_end: player.is_finished(),
        samples,
    };
    let frames = rendered.samples.len() / 2;
    let peak = rendered
        .samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));

    write_wav(output, &rendered)?;
    println!(
        "\n  {} -> {:.1}s at {} Hz, peak {:.3}, {} starved frames, in {:.1}s",
        output.display(),
        frames as f64 / f64::from(rate),
        rate,
        peak,
        player.starved_frames(),
        started.elapsed().as_secs_f64()
    );
    if peak < 0.01 {
        println!("  WARNING: that is silence — something is wrong with the decode");
    }
    Ok(())
}
