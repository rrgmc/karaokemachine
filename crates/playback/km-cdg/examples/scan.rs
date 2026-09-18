//! Replays every `.cdg` under a folder and reports what is in them.
//!
//! The counterpart of `km-lyrics scan` for the MIDI corpus, and here for the same reason: the
//! parser's real specification is what thousands of files off a real disc actually contain, not what
//! the format document says they may. Every file must replay without panicking, and the numbers it
//! prints are what the packaging checks in `km-pack` are set from.
//!
//! ```sh
//! cargo run --release -p km-cdg --example scan -- "/path/to/corpus"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: scan <folder>")?;

    let mut files = Vec::new();
    collect(&root, &mut files)?;
    files.sort();
    println!("{} .cdg files under {}", files.len(), root.display());

    let started = Instant::now();
    let mut unreadable = Vec::new();
    let mut wordless = Vec::new();
    let mut with_unknown = Vec::new();
    let mut with_offscreen = Vec::new();
    let mut misaligned = Vec::new();
    let mut total_packets = 0u64;
    let mut total_unknown = 0u64;
    let mut total_offscreen = 0u64;
    let mut total_tiles = 0u64;
    let mut total_ms = 0u64;
    let mut no_audio = Vec::new();
    let mut unreadable_audio = Vec::new();
    let mut suspect_pairing = Vec::new();
    let mut longer_than_audio = Vec::new();
    let mut rates: BTreeMap<u32, usize> = BTreeMap::new();
    let mut channels: BTreeMap<u16, usize> = BTreeMap::new();
    let mut tagged = 0usize;
    let mut total_short_by = 0u64;

    for path in &files {
        let stream = match km_cdg::read_graphics(path) {
            Ok(stream) => stream,
            Err(error) => {
                unreadable.push(format!("{error}"));
                continue;
            }
        };
        let stats = stream.stats();
        total_packets += u64::from(stats.packets);
        total_unknown += u64::from(stats.unknown_instructions);
        total_offscreen += u64::from(stats.offscreen_tiles);
        total_tiles += u64::from(stats.tiles_written);
        total_ms += u64::from(stats.duration_ms);

        if stats.tiles_written == 0 {
            wordless.push(display(&root, path));
        }
        if stats.offscreen_tiles > 0 {
            with_offscreen.push(format!(
                "{} off-screen tiles  {}",
                stats.offscreen_tiles,
                display(&root, path)
            ));
        }
        if stats.unknown_instructions > 0 {
            with_unknown.push((
                stats.unknown_instructions,
                stats.packets,
                display(&root, path),
            ));
        }
        if stats.trailing_bytes > 0 {
            misaligned.push((stats.trailing_bytes, display(&root, path)));
        }

        // The audio beside it, when there is one. Probing the pair is what validates the half of
        // this that a `.cdg` cannot: whether the MP3 opens, and whether the two describe the same
        // song. A `.cdg` that runs a minute or more short of its audio is the signal that the two
        // were paired by a name and not by a recording.
        let Some(audio_path) = audio_beside(path) else {
            no_audio.push(display(&root, path));
            continue;
        };
        match km_cdg::probe_audio(&audio_path) {
            Ok(audio) => {
                rates
                    .entry(audio.sample_rate)
                    .and_modify(|n| *n += 1)
                    .or_insert(1usize);
                channels
                    .entry(audio.channels)
                    .and_modify(|n| *n += 1)
                    .or_insert(1usize);
                if audio.title.is_some() || audio.artist.is_some() {
                    tagged += 1;
                }
                let short_by = audio.duration_ms.saturating_sub(stats.duration_ms);
                let over = stats.duration_ms.saturating_sub(audio.duration_ms);
                total_short_by += u64::from(short_by);
                if over > 1_000 {
                    longer_than_audio.push(format!(
                        "graphics run {:.1}s past the audio  {}",
                        f64::from(over) / 1000.0,
                        display(&root, path)
                    ));
                }
                if short_by > 60_000 {
                    suspect_pairing.push(format!(
                        "graphics stop {:.0}s early  {}",
                        f64::from(short_by) / 1000.0,
                        display(&root, path)
                    ));
                }
            }
            Err(error) => unreadable_audio.push(format!("{error}")),
        }
    }

    let elapsed = started.elapsed();
    println!(
        "\nreplayed {total_packets} packets in {:.1}s ({:.0} files/s)",
        elapsed.as_secs_f64(),
        files.len() as f64 / elapsed.as_secs_f64().max(0.001)
    );
    println!(
        "  {total_tiles} tiles drawn, {total_unknown} unknown instructions, {total_offscreen} off-screen tiles"
    );
    println!(
        "  {:.1} hours of graphics, {:.1} minutes a song on average",
        total_ms as f64 / 3_600_000.0,
        total_ms as f64 / 60_000.0 / files.len().max(1) as f64
    );

    let paired = files.len() - no_audio.len();
    println!(
        "\naudio beside the graphics: {paired} of {} paired, {tagged} carrying usable tags",
        files.len()
    );
    println!("  sample rates: {rates:?}");
    println!("  channels: {channels:?}");
    if paired > 0 {
        println!(
            "  graphics stop {:.1}s before the audio on average",
            total_short_by as f64 / 1000.0 / paired as f64
        );
    }

    report("unreadable", &unreadable);
    report("no tiles drawn — no words in the file", &wordless);
    report("no audio file beside the graphics", &no_audio);
    report("audio that would not probe", &unreadable_audio);
    report(
        "graphics running PAST the end of the audio (never seen; would break the clock)",
        &longer_than_audio,
    );
    report(
        "graphics stopping over a minute early — suspect a mispairing",
        &suspect_pairing,
    );
    report(
        "not a whole number of packets",
        &misaligned
            .iter()
            .map(|(bytes, name)| format!("{bytes} trailing bytes  {name}"))
            .collect::<Vec<_>>(),
    );

    report("tiles addressed off the screen", &with_offscreen);

    with_unknown.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    report(
        "unknown CD+G instructions (diagnostic only -- every such file renders)",
        &with_unknown
            .iter()
            .map(|(bad, all, name)| {
                format!(
                    "{bad} of {all}  ({:.2}%)  {name}",
                    *bad as f64 * 100.0 / f64::from(*all)
                )
            })
            .collect::<Vec<_>>(),
    );
    Ok(())
}

fn report(heading: &str, lines: &[String]) {
    println!("\n{}: {}", heading, lines.len());
    for line in lines.iter().take(15) {
        println!("  {line}");
    }
    if lines.len() > 15 {
        println!("  ...and {} more", lines.len() - 15);
    }
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The audio file paired with a `.cdg`, if one is on disk.
///
/// **Deliberately tolerant, because a real corpus is not tidy.** Extensions come in both cases in
/// the measured corpus, and one pair differs only by a trailing space in the stem — so the obvious
/// spellings are tried first and then the directory is listed and matched on a trimmed, lowercased
/// stem. The real pairing rule lands in `km-kmpkg` with the packaging work; this is the same idea,
/// kept here so the scan can report what it could not pair.
fn audio_beside(graphics: &Path) -> Option<PathBuf> {
    for extension in ["mp3", "MP3", "Mp3"] {
        let candidate = graphics.with_extension(extension);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let stem = graphics.file_stem()?.to_str()?.trim().to_lowercase();
    let entries = std::fs::read_dir(graphics.parent()?).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mp3"))
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.trim().to_lowercase() == stem)
        })
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cdg"))
        {
            out.push(path);
        }
    }
    Ok(())
}
