//! Renders a real `.cdg` to PNG, at several points through the song.
//!
//! The same trick `km-display` uses to judge a screen on a machine with no monitor, and the fastest
//! way to tell a working renderer from a plausible one — a palette read wrong, a tile bitmap
//! reversed or a scroll offset misapplied all produce something that *looks* like output until
//! somebody looks at it.
//!
//! Pictures come out at the shape a television would show, **not** at 288x192: CD+G pixels are not
//! square (see `km_cdg::DISPLAY_ASPECT`), so a preview at the pixel size would be 12% too wide and
//! would hide exactly the mistake this is meant to catch.
//!
//! ```sh
//! cargo run -p km-cdg --example stills -- "song.cdg"
//! cargo run -p km-cdg --example stills -- "song.cdg" --at 12,45,90 --out ./shots --scale 4
//! ```

use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut source: Option<PathBuf> = None;
    let mut out = PathBuf::from("target/cdg-preview");
    let mut at: Option<Vec<f64>> = None;
    let mut scale = 3u32;

    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--out" => out = args.next().ok_or("--out needs a directory")?.into(),
            "--scale" => {
                scale = args
                    .next()
                    .ok_or("--scale needs a number")?
                    .to_string_lossy()
                    .parse()?
            }
            "--at" => {
                let list = args.next().ok_or("--at needs a list of seconds")?;
                at = Some(
                    list.to_string_lossy()
                        .split(',')
                        .map(str::trim)
                        .filter(|part| !part.is_empty())
                        .map(str::parse)
                        .collect::<Result<_, _>>()?,
                );
            }
            "-h" | "--help" => {
                eprintln!("usage: stills <file.cdg> [--at 12,45,90] [--out DIR] [--scale N]");
                return Ok(());
            }
            _ => source = Some(PathBuf::from(arg)),
        }
    }
    let source =
        source.ok_or("usage: stills <file.cdg> [--at 12,45,90] [--out DIR] [--scale N]")?;

    let stream = km_cdg::read_graphics(&source)?;
    let stats = stream.stats();
    println!("{}", source.display());
    println!(
        "  {} packets, {} unknown instructions, {} tiles drawn, graphics run {:.1}s{}",
        stats.packets,
        stats.unknown_instructions,
        stats.tiles_written,
        f64::from(stats.duration_ms) / 1000.0,
        if stats.trailing_bytes == 0 {
            String::new()
        } else {
            format!(", {} trailing bytes ignored", stats.trailing_bytes)
        }
    );
    if stats.tiles_written == 0 {
        println!("  NOTE: nothing is ever drawn — this file has no words in it");
    }

    // A spread through the song by default. Fractions rather than fixed seconds, so the same command
    // is useful on a two-minute song and a six-minute one.
    let seconds = at.unwrap_or_else(|| {
        let total = f64::from(stats.duration_ms) / 1000.0;
        [0.10, 0.25, 0.40, 0.55, 0.70, 0.85]
            .iter()
            .map(|fraction| total * fraction)
            .collect()
    });

    std::fs::create_dir_all(&out)?;
    let reader = km_cdg::FrameReader::new(stream);
    let stem = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    for second in seconds {
        let position_ms = (second * 1000.0).max(0.0) as u32;
        // The reader answers `None` when nothing has changed, which is the ordinary case mid-song.
        // A preview wants a picture regardless, so ask twice: the second call after a rewind always
        // redraws, and at these spacings the first will have advanced past something.
        let frame = reader
            .take_frame_for(position_ms)
            .or_else(|| {
                reader.take_frame_for(0);
                reader.take_frame_for(position_ms)
            })
            .ok_or("the renderer produced no picture at all")?;

        let path = out.join(format!("{stem}-{position_ms:07}ms.png"));
        write_png(&frame, scale, &path)?;
        println!("  {}", path.display());
        reader.recycle(frame);
    }
    Ok(())
}

/// Writes one picture, scaled nearest-neighbor and corrected to the shape a television shows.
fn write_png(
    frame: &km_cdg::Frame,
    scale: u32,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (pixels, pitch) = frame.pixels();
    let (source_width, source_height) = (frame.width(), frame.height());

    // Height by the scale, width by the display aspect. Nearest-neighbor throughout, deliberately:
    // a karaoke machine's picture is meant to look blocky, and a smoothed preview would hide a
    // one-pixel mistake.
    let height = source_height * scale;
    let (aspect_w, aspect_h) = km_cdg::DISPLAY_ASPECT;
    let width = height * aspect_w / aspect_h;

    let mut out = image::RgbaImage::new(width, height);
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let sx = (x * source_width / width).min(source_width - 1) as usize;
        let sy = (y * source_height / height).min(source_height - 1) as usize;
        let at = sy * pitch + sx * 4;
        // Read the packed value back rather than picking bytes out: the frame carries native-endian
        // `0xAARRGGBB`, so a fixed byte order would be right on one endianness and wrong on the other.
        let argb = u32::from_ne_bytes([pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]);
        *pixel = image::Rgba([
            (argb >> 16) as u8,
            (argb >> 8) as u8,
            argb as u8,
            (argb >> 24) as u8,
        ]);
    }
    out.save(path)?;
    Ok(())
}
