//! Writes an HLS stream of a moving test pattern, for looking at in a player.
//!
//! ```text
//! cargo run -p km-stream --example synthetic --features ffmpeg -- out_dir [seconds] [encoder]
//! ```
//!
//! **What it is for is the question a unit test cannot answer**: whether what comes out of this
//! crate is something a television, a browser or VLC will actually play. Every part of the chain is
//! exercised — the colour conversion, the encoder, the fMP4 segments and the rolling playlist —
//! with a picture generated here rather than drawn, so nothing but this crate is involved.
//!
//! The pattern moves, because a still picture cannot show a stream that has stopped advancing: an
//! encoder fed one repeated frame produces a playlist that grows and a screen that looks identical
//! whether time is passing or not.

use std::path::PathBuf;

use km_stream::encode::{Config, Stream};

/// Small enough to encode quickly, and still an ordinary 16:9 shape.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const FPS: u32 = 30;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .ok_or("usage: synthetic <out_dir> [seconds] [encoder]")?,
    );
    let seconds: u32 = args.next().map_or(Ok(6), |value| value.parse())?;
    let encoder = args.next().unwrap_or_else(|| "libopenh264".to_owned());

    let config = Config {
        width: WIDTH,
        height: HEIGHT,
        fps: FPS,
        // Well above what this pattern needs, so nothing in the picture is the encoder's opinion.
        bitrate: 4_000_000,
        encoder,
        ..Config::default()
    };

    println!(
        "encoding {seconds}s of {WIDTH}x{HEIGHT} at {FPS} fps with {} into {}",
        config.encoder,
        dir.display()
    );

    let mut stream = Stream::open(&dir, &config)?;
    let frames = seconds * FPS;
    let mut screen = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    // **One frame's worth of sound per frame, which is what keeps the two in step.** Nothing here
    // consults a clock: the picture's time is the count of frames and the sound's is the count of
    // samples, so handing over this many samples per frame *is* the synchronisation.
    let samples_per_frame = (config.sample_rate / FPS) as usize;
    let mut sound = vec![0f32; samples_per_frame * km_stream::encode::CHANNELS];
    let mut phase = 0f32;
    for number in 0..frames {
        paint(&mut screen, number);
        tone(&mut sound, number, config.sample_rate, &mut phase);
        stream.push(&screen)?;
        stream.push_audio(&sound)?;
    }
    stream.finish()?;

    println!(
        "wrote {frames} frames; play {}",
        dir.join("live.m3u8").display()
    );
    Ok(())
}

/// Fills one frame's worth of interleaved stereo with a tone that changes every second.
///
/// **It changes on the same second the corner block does**, which is what makes the two streams
/// checkable against each other by watching and listening: if the sound and the picture agree on
/// when the second turned, they are in step.
///
/// The phase carries across calls rather than being computed from the frame number, so the wave is
/// continuous at every boundary. Restarting it each frame would put a click thirty times a second
/// into a stream whose whole purpose here is to sound right.
fn tone(out: &mut [f32], number: u32, sample_rate: u32, phase: &mut f32) {
    let second = number / FPS;
    // A fifth apart, alternating, at a level that is audible and nowhere near clipping.
    let hz = if second.is_multiple_of(2) {
        440.0
    } else {
        660.0
    };
    let step = std::f32::consts::TAU * hz / sample_rate as f32;
    for pair in out.as_chunks_mut::<{ km_stream::encode::CHANNELS }>().0 {
        let value = phase.sin() * 0.2;
        pair[0] = value;
        pair[1] = value;
        *phase += step;
        if *phase > std::f32::consts::TAU {
            *phase -= std::f32::consts::TAU;
        }
    }
}

/// Paints one frame of the pattern into a BGRA screen.
///
/// Three things a person can check at a glance: the bars say the colour conversion kept red and
/// blue apart, the sweeping band says frames are arriving in order, and the corner block changing
/// every second says the timestamps advance at the rate that was asked for.
fn paint(screen: &mut [u8], number: u32) {
    let (width, height) = (WIDTH as usize, HEIGHT as usize);
    let bars: [[u8; 3]; 8] = [
        [255, 255, 255], // white
        [0, 255, 255],   // yellow
        [255, 255, 0],   // cyan
        [0, 255, 0],     // green
        [255, 0, 255],   // magenta
        [0, 0, 255],     // red
        [255, 0, 0],     // blue
        [0, 0, 0],       // black
    ];
    let sweep = (number as usize * 4) % width;
    let second = number / FPS;

    for y in 0..height {
        for x in 0..width {
            let mut pixel = bars[x * bars.len() / width];
            // A bright band travelling left to right, one frame at a time.
            if x.abs_diff(sweep) < 6 {
                pixel = [255, 255, 255];
            }
            // A block in the corner that alternates once a second.
            if x < 40 && y < 40 && second.is_multiple_of(2) {
                pixel = [0, 128, 255];
            }
            let at = (y * width + x) * 4;
            screen[at] = pixel[0];
            screen[at + 1] = pixel[1];
            screen[at + 2] = pixel[2];
            screen[at + 3] = 255;
        }
    }
}
