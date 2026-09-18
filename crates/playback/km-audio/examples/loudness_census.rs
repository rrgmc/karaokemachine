//! Measures how loud a folder of karaoke files renders, and how well a synthesizer-free estimate
//! predicts it.
//!
//! ```text
//! KM_CORPUS=<your karaoke folder> cargo run --release -p km-audio --example loudness_census -- \
//!     "$KM_CORPUS" --bank <a.sf2> [--bank <b.sf2>] [--limit 1000] [--tsv rows.tsv]
//! ```
//!
//! **It exists because the song-to-song spread a bank is judged on was measured over seven songs,
//! and a levelling rule has to be designed against the corpus.** `soundfont-banks.conf` records
//! 7.4 LU for the recommended bank and 7.8 for the bundled one; what that says about a hundred
//! thousand real files is unknown, and so is whether a song's distance from its bank's mean is the
//! same distance on another bank.
//!
//! Rendering is the expensive half, so the two questions that do not need it are answered without
//! it. `--estimate-only` skips every bank and sweeps the event-based estimate over as many files as
//! there is patience for; the estimate is printed beside the rendered figure on a smaller sample, so
//! one run says how far apart the two are.
//!
//! **The rendering matches `tools/dev/soundfont-measure.sh`** — 44.1 kHz, music volume 1.0, guide
//! melody off, 90 seconds by default — so a mean printed here is comparable with the `lufs` column
//! that script produced. Run it over that script's seven songs first: a harness that does not
//! reproduce the figures already written down is measuring something else.
//!
//! Every summary this prints is arithmetic over the rows, done here because the development box has
//! neither `python` nor `bc`.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use km_audio::{Bank, PlaybackSettings, RenderOptions, SoundFontSource, render};
use km_loudness::Meter;
use km_song::{EventKind, ParseOptions, Song};
use km_suitability::Analysis;

const USAGE: &str = "usage: loudness_census <folder> [options]

options:
  --bank <file.sf2>      render through this bank; repeat for a comparison
  --limit <n>            sample n files, spread evenly through the sorted list
  --max-ms <ms>          how much of each song to render (default 90000, 0 for all of it)
  --estimate-only        skip every bank and only run the event-based estimate
  --tsv <file>           write the per-song rows here instead of to stdout
  --melody <on|off>      guide melody audible (default off, as the bank table was measured)
  --refit <rows.tsv>     re-fit the estimate against a run's rows, rendering nothing
";

/// The rate every bank was measured at, and the one `render_wav` uses.
const SAMPLE_RATE: u32 = 44_100;
/// What the census was asked to do.
struct Args {
    root: PathBuf,
    banks: Vec<PathBuf>,
    limit: Option<usize>,
    max_ms: u32,
    estimate_only: bool,
    tsv: Option<PathBuf>,
    melody_enabled: bool,
    refit: Option<PathBuf>,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut positional: Vec<String> = Vec::new();
    let mut args = Args {
        root: PathBuf::new(),
        banks: Vec::new(),
        limit: None,
        max_ms: 90_000,
        estimate_only: false,
        tsv: None,
        melody_enabled: false,
        refit: None,
    };
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        let mut value = || raw.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--bank" => args.banks.push(PathBuf::from(value()?)),
            "--limit" => args.limit = Some(value()?.parse()?),
            "--max-ms" => args.max_ms = value()?.parse()?,
            "--estimate-only" => args.estimate_only = true,
            "--tsv" => args.tsv = Some(PathBuf::from(value()?)),
            "--melody" => {
                args.melody_enabled = matches!(value()?.as_str(), "on" | "true" | "1");
            }
            "--refit" => args.refit = Some(PathBuf::from(value()?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}").into());
            }
            other => positional.push(other.to_owned()),
        }
    }
    // `--refit` reads its file list out of the rows, so it is the one mode that needs no folder.
    if args.refit.is_none() {
        args.root = PathBuf::from(positional.first().ok_or(USAGE)?);
    }
    if args.estimate_only {
        args.banks.clear();
    }
    Ok(args)
}

// ---------------------------------------------------------------------------------------------
// Finding the files
// ---------------------------------------------------------------------------------------------

/// Every karaoke file under `root`, sorted, so a re-run reads the same list in the same order.
fn collect(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => {
                    let extension = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if matches!(extension.as_str(), "mid" | "midi" | "kar") {
                        found.push(path);
                    }
                }
                _ => {}
            }
        }
    }
    found.sort();
    found
}

/// `limit` files spread evenly through the list, rather than the first `limit` of it.
///
/// The corpus is arranged in folders by origin, so the head of a sorted list is one collection
/// rather than a sample of the whole.
fn sample(files: Vec<PathBuf>, limit: Option<usize>) -> Vec<PathBuf> {
    let Some(limit) = limit else { return files };
    if limit == 0 || files.len() <= limit {
        return files;
    }
    (0..limit)
        .map(|i| files[i * files.len() / limit].clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Master volume SysEx, which the parser drops
// ---------------------------------------------------------------------------------------------

/// Whether the file's bytes carry a General MIDI, Roland GS or Yamaha XG master volume message.
///
/// **A byte scan rather than a parse**, because `km_song::Song` discards SysEx before a caller can
/// see it. A message the file never plays because it sits inside a lyric would be counted here, so
/// this answers "is this worth looking into" and not "how many files do it".
fn master_volume_sysex(bytes: &[u8]) -> (bool, bool, bool) {
    let gm = bytes
        .windows(5)
        .any(|w| w[0] == 0xF0 && w[1] == 0x7F && w[3] == 0x04 && w[4] == 0x01);
    let gs = bytes.windows(8).any(|w| {
        w[0] == 0xF0
            && w[1] == 0x41
            && w[3] == 0x42
            && w[4] == 0x12
            && w[5] == 0x40
            && w[6] == 0x00
            && w[7] == 0x04
    });
    let xg = bytes.windows(7).any(|w| {
        w[0] == 0xF0 && w[1] == 0x43 && w[3] == 0x4C && w[4] == 0x00 && w[5] == 0x00 && w[6] == 0x04
    });
    (gm, gs, xg)
}

// ---------------------------------------------------------------------------------------------
// Arithmetic over the rows
// ---------------------------------------------------------------------------------------------

/// The shape of one column of numbers.
struct Stats {
    n: usize,
    mean: f64,
    sd: f64,
    min: f64,
    p10: f64,
    p50: f64,
    p90: f64,
    max: f64,
}

impl Stats {
    fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let variance = sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
        Some(Self {
            n,
            mean,
            sd: variance.sqrt(),
            min: sorted[0],
            p10: percentile(&sorted, 0.10),
            p50: percentile(&sorted, 0.50),
            p90: percentile(&sorted, 0.90),
            max: sorted[n - 1],
        })
    }

    fn row(&self, label: &str) -> String {
        format!(
            "| {label} | {} | {:.2} | {:.2} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | **{:.1}** | {:.1} |",
            self.n,
            self.mean,
            self.sd,
            self.min,
            self.p10,
            self.p50,
            self.p90,
            self.max,
            self.p90 - self.p10,
            self.max - self.min,
        )
    }
}

const STATS_HEADER: &str = "| what | n | mean | sd | min | p10 | p50 | p90 | max | p90-p10 | range |\n\
     |---|---|---|---|---|---|---|---|---|---|---|";

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

/// Pearson correlation, for the two questions that are about agreement rather than about level.
fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 2 {
        return f64::NAN;
    }
    let mean_a = a[..n].iter().sum::<f64>() / n as f64;
    let mean_b = b[..n].iter().sum::<f64>() / n as f64;
    let mut covariance = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for i in 0..n {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        covariance += da * db;
        var_a += da * da;
        var_b += db * db;
    }
    covariance / (var_a.sqrt() * var_b.sqrt())
}

/// Least squares `b` against `a`, returning slope, intercept and the residual's standard deviation.
fn regress(a: &[f64], b: &[f64]) -> (f64, f64, f64) {
    let n = a.len().min(b.len());
    if n < 2 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let mean_a = a[..n].iter().sum::<f64>() / n as f64;
    let mean_b = b[..n].iter().sum::<f64>() / n as f64;
    let mut covariance = 0.0;
    let mut var_a = 0.0;
    for i in 0..n {
        covariance += (a[i] - mean_a) * (b[i] - mean_b);
        var_a += (a[i] - mean_a).powi(2);
    }
    let slope = covariance / var_a;
    let intercept = mean_b - slope * mean_a;
    let residuals: Vec<f64> = (0..n).map(|i| b[i] - (slope * a[i] + intercept)).collect();
    let mean_r = residuals.iter().sum::<f64>() / n as f64;
    let sd = (residuals.iter().map(|r| (r - mean_r).powi(2)).sum::<f64>() / n as f64).sqrt();
    (slope, intercept, sd)
}

// ---------------------------------------------------------------------------------------------
// What one bank produced
// ---------------------------------------------------------------------------------------------

/// Everything one bank said about the sample.
struct BankRun {
    label: String,
    bank: Bank,
    lufs: Vec<f64>,
    peak: Vec<f64>,
    seconds: f64,
}

/// What levelling to a target would leave, for a bank whose songs measured `lufs`.
///
/// Two columns for two rules. **Attenuate** is the rule media already plays by: a song above the
/// target comes down to it and a song below it is left alone, so nothing can clip and the catalog
/// gets quieter. **Both ways** lets a quiet song come up as well, by no more than the headroom its
/// own measured true peak leaves under −1 dBTP.
fn levelling_table(lufs: &[f64], peak: &[f64], mean: f64) {
    println!(
        "| target | attenuated | p90-p10 after | mean level change | p90-p10 both ways | mean change both ways |"
    );
    println!("|---|---|---|---|---|---|");
    let mut offset = 0.0;
    while offset <= 12.0 {
        let target = mean - offset;
        let mut down = Vec::with_capacity(lufs.len());
        let mut both = Vec::with_capacity(lufs.len());
        let mut touched = 0usize;
        for (index, level) in lufs.iter().enumerate() {
            let wanted = target - level;
            if wanted < 0.0 {
                touched += 1;
            }
            down.push(level + wanted.min(0.0));
            let headroom = (-1.0 - peak.get(index).copied().unwrap_or(0.0)).max(0.0);
            both.push(level + wanted.min(headroom));
        }
        let (Some(down), Some(both)) = (Stats::of(&down), Stats::of(&both)) else {
            return;
        };
        println!(
            "| {:.1} | {}% | {:.1} | {:+.1} | {:.1} | {:+.1} |",
            target,
            touched * 100 / lufs.len().max(1),
            down.p90 - down.p10,
            down.mean - mean,
            both.p90 - both.p10,
            both.mean - mean,
        );
        offset += 1.0;
    }
}

// ---------------------------------------------------------------------------------------------

/// The General MIDI families, which are its program numbers in groups of eight.
const FAMILIES: [&str; 16] = [
    "piano",
    "chromatic",
    "organ",
    "guitar",
    "bass",
    "strings",
    "ensemble",
    "brass",
    "reed",
    "pipe",
    "synth lead",
    "synth pad",
    "synth fx",
    "ethnic",
    "percussive",
    "sound fx",
];

/// What a song is made of, as shares of the power the estimate counts.
///
/// **The estimate's named weakness is that it cannot see which instrument a program number
/// selects**, and this is what tests whether that is where its error lives. Eighteen numbers per
/// song: a share for each General MIDI family, a share for the drum channel, and the power-weighted
/// mean pitch, which stands in for the frequency weighting R128 applies and this does not.
///
/// The power is the same product the estimate uses, so a share here is a share of the quantity that
/// actually drives the figure rather than a count of notes.
fn instrument_features(song: &Song, muted: Option<u8>) -> ([f64; 16], f64, f64, f64, f64) {
    let mut family = [0.0f64; 16];
    let mut drums = 0.0f64;
    let mut pitch_weighted = 0.0f64;
    let mut reverb_weighted = 0.0f64;
    let mut total = 0.0f64;
    let mut notes = 0.0f64;

    let mut program = [0u8; 16];
    let mut cc7 = [100u8; 16];
    let mut cc11 = [127u8; 16];
    // Reverb send. The synthesizer applies it and the estimate ignores it, so a song drenched in it
    // renders louder than its notes alone say.
    let mut cc91 = [40u8; 16];

    for event in &song.events {
        match event.kind {
            EventKind::ProgramChange {
                channel,
                program: p,
            } => {
                program[usize::from(channel.min(15))] = p;
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
                    91 => cc91[channel] = value,
                    _ => {}
                }
            }
            EventKind::NoteOn {
                channel,
                key,
                velocity,
            } => {
                if muted == Some(channel) {
                    continue;
                }
                let index = usize::from(channel.min(15));
                let level = f64::from(velocity) / 127.0 * f64::from(cc7[index]) / 127.0
                    * f64::from(cc11[index])
                    / 127.0;
                let power = level.powi(4);
                total += power;
                notes += 1.0;
                pitch_weighted += power * f64::from(key);
                reverb_weighted += power * f64::from(cc91[index]);
                if index == 9 {
                    drums += power;
                } else {
                    family[usize::from(program[index] / 8).min(15)] += power;
                }
            }
            _ => {}
        }
    }
    if total <= 0.0 {
        return ([0.0; 16], 0.0, 0.0, 0.0, 0.0);
    }
    for share in &mut family {
        *share /= total;
    }
    // Notes a second, which stands in for how densely the arrangement stacks. Powers are summed as
    // though notes were uncorrelated, and a dense arrangement is where that assumption is worst.
    let density = notes / f64::from(song.duration_ms().max(1)) * 1_000.0;
    (
        family,
        drums / total,
        pitch_weighted / total,
        reverb_weighted / total,
        density,
    )
}

/// Re-fits the estimate against rows a rendering run already wrote.
///
/// **Changing the estimate must not cost another render.** Every constant in
/// `km_song::loudness` has to be answered with a residual, and re-rendering a thousand songs to try
/// one would mean nobody tries the second. This re-parses the files a run measured, recomputes the
/// estimate as the crate now computes it, and prints only the comparison. It also times the estimate
/// on its own, which is what says whether it belongs on a song start.
fn refit(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .ok_or("the rows file is empty")?
        .split('\t')
        .collect();
    let banks: Vec<(usize, String)> = header
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            name.strip_suffix("_lufs")
                .map(|label| (index, label.to_owned()))
        })
        .collect();
    if banks.is_empty() {
        return Err("the rows file carries no rendered loudness to fit against".into());
    }

    let mut rendered: Vec<Vec<f64>> = vec![Vec::new(); banks.len()];
    let mut estimated: Vec<f64> = Vec::new();
    let mut features: Vec<([f64; 16], f64, f64, f64, f64)> = Vec::new();
    let mut skipped = 0usize;
    let mut estimate_ms = 0.0f64;
    let mut slowest_ms = 0.0f64;
    let mut slowest = String::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        let Some(file) = cells.first() else { continue };
        let measured: Option<Vec<f64>> = banks
            .iter()
            .map(|(index, _)| cells.get(*index).and_then(|c| c.parse::<f64>().ok()))
            .collect();
        let Some(measured) = measured else {
            skipped += 1;
            continue;
        };
        let Ok(bytes) = std::fs::read(file) else {
            skipped += 1;
            continue;
        };
        let Ok(song) = Song::parse(&bytes, &ParseOptions::default()) else {
            skipped += 1;
            continue;
        };
        let melody = Analysis::of(&song).melody_channel();
        // Timed on its own, not with the read and the parse: the question this answers is what the
        // estimate would add to a song start, and a caller there has already parsed the song.
        let started = Instant::now();
        let Some(level) = song.estimated_loudness_db(melody) else {
            skipped += 1;
            continue;
        };
        let level = f64::from(level);
        let took = started.elapsed().as_secs_f64() * 1_000.0;
        estimate_ms += took;
        if took > slowest_ms {
            slowest_ms = took;
            slowest = file.to_string();
        }
        for (column, measured_level) in rendered.iter_mut().zip(&measured) {
            column.push(*measured_level);
        }
        estimated.push(level);
        let (family, drums, pitch, reverb, density) = instrument_features(&song, melody);
        features.push((family, drums, pitch, reverb, density));
    }

    println!("{} songs, {skipped} skipped", estimated.len());
    // The mean is what `km_loudness::MIDI_REFERENCE_ESTIMATE` is set from, so a change to the
    // estimate that moves it is a change to that constant as well.
    if let Some(stats) = Stats::of(&estimated) {
        println!(
            "the estimate over this sample: mean {:.2} dB, sd {:.2} (km_loudness::MIDI_REFERENCE_ESTIMATE is {:.1})",
            stats.mean,
            stats.sd,
            km_loudness::MIDI_REFERENCE_ESTIMATE
        );
    }
    println!(
        "the estimate alone: {:.2} ms a song on average, {:.1} ms at worst ({})",
        estimate_ms / estimated.len().max(1) as f64,
        slowest_ms,
        std::path::Path::new(&slowest)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
    );
    println!();
    println!("| bank | r | slope | residual sd (LU) |");
    println!("|---|---|---|---|");
    for ((_, label), column) in banks.iter().zip(&rendered) {
        let (slope, _, sd) = regress(&estimated, column);
        println!(
            "| {label} | {:.3} | {:.3} | {:.2} |",
            pearson(&estimated, column),
            slope,
            sd
        );
    }

    // What the estimate got wrong, against what each song is made of. The residual is taken against
    // the first bank's own fit, so it is what a correction would have to explain.
    if let Some(column) = rendered.first() {
        let (slope, intercept, sd) = regress(&estimated, column);
        let residual: Vec<f64> = estimated
            .iter()
            .zip(column)
            .map(|(estimate, rendered)| rendered - (slope * estimate + intercept))
            .collect();

        println!();
        println!(
            "What the estimate misses, against {} songs:",
            residual.len()
        );
        println!();
        println!(
            "| what a song is made of | mean share | r with the residual | LU across its range |"
        );
        println!("|---|---|---|---|");

        let mut rows: Vec<(String, f64, f64, f64)> = Vec::new();
        for (index, name) in FAMILIES.iter().enumerate() {
            let share: Vec<f64> = features.iter().map(|f| f.0[index]).collect();
            rows.push(describe(name, &share, &residual));
        }
        let drums: Vec<f64> = features.iter().map(|f| f.1).collect();
        rows.push(describe("drum channel", &drums, &residual));
        let pitch: Vec<f64> = features.iter().map(|f| f.2).collect();
        rows.push(describe("mean pitch", &pitch, &residual));
        let reverb: Vec<f64> = features.iter().map(|f| f.3).collect();
        rows.push(describe("reverb send", &reverb, &residual));
        let density: Vec<f64> = features.iter().map(|f| f.4).collect();
        rows.push(describe("notes a second", &density, &residual));

        // Strongest first: a feature worth acting on is one whose own range moves the residual by
        // something against the sd above.
        rows.sort_by(|a, b| {
            b.3.abs()
                .partial_cmp(&a.3.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (name, mean, r, swing) in &rows {
            println!("| {name} | {mean:.3} | {r:+.3} | {swing:+.2} |");
        }
        println!();
        println!("The residual to beat is {sd:.2} LU.");

        // **Fitted on half the songs and scored on the other half**, because twenty features against
        // 989 songs will always look better on the songs they were fitted to. Only the second number
        // says whether a correction is real.
        let mut columns: Vec<(String, Vec<f64>)> = Vec::new();
        for (index, name) in FAMILIES.iter().enumerate() {
            columns.push((
                (*name).to_owned(),
                features.iter().map(|f| f.0[index]).collect(),
            ));
        }
        columns.push(("drum channel".to_owned(), drums));
        columns.push(("reverb send".to_owned(), reverb));

        let train: Vec<usize> = (0..residual.len()).filter(|i| i % 2 == 0).collect();
        let test: Vec<usize> = (0..residual.len()).filter(|i| i % 2 == 1).collect();
        let pick = |rows: &[usize], values: &[f64]| -> Vec<f64> {
            rows.iter().map(|i| values[*i]).collect()
        };

        // Coordinate descent over a chosen subset: fit one feature against what is left, ten times
        // round. The features overlap, since a song's family shares sum to one, so one pass would
        // credit whichever came first.
        let fit = |chosen: &[usize]| -> Vec<f64> {
            let mut weights = vec![0.0f64; columns.len()];
            let mut left = pick(&train, &residual);
            for _ in 0..10 {
                for index in chosen {
                    let column = pick(&train, &columns[*index].1);
                    let (slope, _, _) = regress(&column, &left);
                    if !slope.is_finite() {
                        continue;
                    }
                    weights[*index] += slope;
                    for (row, value) in left.iter_mut().zip(&column) {
                        *row -= slope * value;
                    }
                }
            }
            weights
        };
        let score = |weights: &[f64], rows: &[usize]| -> f64 {
            let corrected: Vec<f64> = rows
                .iter()
                .map(|i| {
                    let correction: f64 = columns
                        .iter()
                        .zip(weights)
                        .map(|((_, values), weight)| values[*i] * weight)
                        .sum();
                    residual[*i] - correction
                })
                .collect();
            let mean = corrected.iter().sum::<f64>() / corrected.len().max(1) as f64;
            (corrected.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / corrected.len().max(1) as f64)
                .sqrt()
        };

        let piano = 0;
        let bass = 4;
        let reverb = columns.len() - 1;
        let all: Vec<usize> = (0..columns.len()).collect();

        println!();
        println!("| correcting by | constants | fitted to | held back |");
        println!("|---|---|---|---|");
        for (label, chosen) in [
            ("piano and reverb", vec![piano, reverb]),
            ("piano, reverb, bass", vec![piano, reverb, bass]),
            ("every feature above", all),
        ] {
            let weights = fit(&chosen);
            println!(
                "| {label} | {} | {:.2} LU | **{:.2} LU** |",
                chosen.len(),
                score(&weights, &train),
                score(&weights, &test)
            );
            if chosen.len() <= 3 {
                for index in &chosen {
                    println!(
                        "|   {} | | | {:+.2} per unit |",
                        columns[*index].0, weights[*index]
                    );
                }
            }
        }
    }

    Ok(())
}

/// One feature's mean, its correlation with the residual, and how much of the residual its own
/// spread accounts for.
///
/// The last is what says whether a feature is worth acting on: a strong correlation over a range no
/// real song varies across buys nothing.
fn describe(name: &str, feature: &[f64], residual: &[f64]) -> (String, f64, f64, f64) {
    let mean = feature.iter().sum::<f64>() / feature.len().max(1) as f64;
    let r = pearson(feature, residual);
    let (slope, _, _) = regress(feature, residual);
    let spread = Stats::of(feature).map_or(0.0, |s| s.p90 - s.p10);
    (name.to_owned(), mean, r, slope * spread)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;

    if let Some(rows) = &args.refit {
        return refit(rows);
    }

    let files = sample(collect(&args.root), args.limit);
    if files.is_empty() {
        return Err(format!("no .mid, .midi or .kar files under {}", args.root.display()).into());
    }
    eprintln!("loudness_census: {} files", files.len());

    let mut runs: Vec<BankRun> = Vec::new();
    for path in &args.banks {
        let bank = Bank::load(path)?;
        let label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("bank")
            .to_owned();
        if !bank.defects().is_empty() {
            eprintln!("loudness_census: {label}: {}", bank.defects());
        }
        eprintln!("loudness_census: loaded {label}");
        runs.push(BankRun {
            label,
            bank,
            lufs: Vec::new(),
            peak: Vec::new(),
            seconds: 0.0,
        });
    }

    let options = RenderOptions {
        volume: 1.0,
        max_ms: if args.max_ms == 0 {
            RenderOptions::default().max_ms
        } else {
            args.max_ms
        },
        settings: PlaybackSettings {
            melody_enabled: args.melody_enabled,
            ..PlaybackSettings::default()
        },
        ..RenderOptions::default()
    };

    let mut rows = String::new();
    rows.push_str("file");
    for run in &runs {
        rows.push_str(&format!("\t{}_lufs\t{}_peak", run.label, run.label));
    }
    rows.push_str("\testimate\tcrest\tduration_ms\tnotes\tsysex\n");

    let mut estimates: Vec<f64> = Vec::new();
    let mut estimate_for_row: Vec<Option<f64>> = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    let mut duplicates = 0usize;
    let mut unreadable = 0usize;
    let mut unmeasurable = 0usize;
    let mut diverged = 0usize;
    let mut sysex = (0usize, 0usize, 0usize);
    let mut kept = 0usize;

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            unreadable += 1;
            continue;
        };
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        if !seen.insert(hasher.finish()) {
            duplicates += 1;
            continue;
        }
        let Ok(parsed) = Song::parse(&bytes, &ParseOptions::default()) else {
            unreadable += 1;
            continue;
        };
        let song = Arc::new(parsed);
        // The bank table was measured through `render_wav`, which detects the melody channel and
        // leaves it muted, so a census that skipped the detection would measure a different song.
        let melody = Analysis::of(&song).melody_channel();

        let (gm, gs, xg) = master_volume_sysex(&bytes);
        sysex.0 += usize::from(gm);
        sysex.1 += usize::from(gs);
        sysex.2 += usize::from(xg);

        let estimated = song.estimated_loudness_db(melody).map(f64::from);

        let mut measurements: Vec<Option<(f64, f64)>> = Vec::with_capacity(runs.len());
        for run in &mut runs {
            let started = Instant::now();
            let Ok(source) = SoundFontSource::from_bank(&run.bank, SAMPLE_RATE) else {
                measurements.push(None);
                continue;
            };
            let rendered = render(source, Arc::clone(&song), melody, &options);
            let measured = Meter::stereo(rendered.sample_rate).and_then(|mut meter| {
                meter.add(&rendered.samples);
                meter.finish()
            });
            run.seconds += started.elapsed().as_secs_f64();
            match measured {
                // A figure at or above full scale is a synthesizer that has run away rather than a
                // loud song, and averaging one in would move every number after it.
                Some(loudness) if loudness.lufs >= 0.0 => {
                    diverged += 1;
                    measurements.push(None);
                }
                Some(loudness) => measurements.push(Some((
                    f64::from(loudness.lufs),
                    f64::from(loudness.peak_dbtp),
                ))),
                None => {
                    unmeasurable += 1;
                    measurements.push(None);
                }
            }
        }

        // A song only joins the columns if every bank measured it, so the per-bank statistics and
        // the transfer comparison are over one sample rather than three overlapping ones.
        let complete = measurements.iter().all(Option::is_some);
        if complete {
            for (run, measured) in runs.iter_mut().zip(&measurements) {
                if let Some((lufs, peak)) = measured {
                    run.lufs.push(*lufs);
                    run.peak.push(*peak);
                }
            }
            estimate_for_row.push(estimated);
        }
        if let Some(level) = estimated {
            estimates.push(level);
        }
        kept += 1;

        rows.push_str(&path.display().to_string());
        for measured in &measurements {
            match measured {
                Some((lufs, peak)) => rows.push_str(&format!("\t{lufs:.2}\t{peak:.2}")),
                None => rows.push_str("\t\t"),
            }
        }
        match estimated {
            Some(level) => rows.push_str(&format!("\t{level:.2}")),
            None => rows.push('\t'),
        }
        rows.push_str(&format!(
            "\t{}\t{}\t{}{}{}\n",
            song.duration_ms(),
            song.note_count(),
            if gm { "gm" } else { "" },
            if gs { "gs" } else { "" },
            if xg { "xg" } else { "" },
        ));

        if kept.is_multiple_of(50) {
            eprintln!("loudness_census: {kept} measured");
        }
    }

    match &args.tsv {
        Some(path) => std::fs::write(path, &rows)?,
        None => print!("{rows}"),
    }

    // -----------------------------------------------------------------------------------------
    println!();
    println!("## The sample");
    println!();
    println!("- {kept} files measured, of {} sampled", files.len());
    println!("- {duplicates} dropped as byte-identical duplicates");
    println!("- {unreadable} would not read or would not parse");
    println!("- {unmeasurable} rendered too quiet or too short for the meter");
    println!("- {diverged} rendered at or above full scale and are excluded");
    println!(
        "- master volume SysEx by byte scan: {} General MIDI, {} Roland GS, {} Yamaha XG",
        sysex.0, sysex.1, sysex.2
    );
    if !runs.is_empty() {
        println!(
            "- rendered at {SAMPLE_RATE} Hz, music volume 1.0, guide melody {}, {} ms cap",
            if args.melody_enabled { "on" } else { "muted" },
            options.max_ms
        );
    }

    println!();
    println!("## What each bank measured");
    println!();
    println!("{STATS_HEADER}");
    for run in &runs {
        if let Some(stats) = Stats::of(&run.lufs) {
            println!("{}", stats.row(&format!("{} LUFS", run.label)));
        }
        if let Some(stats) = Stats::of(&run.peak) {
            println!("{}", stats.row(&format!("{} dBTP", run.label)));
        }
    }
    if let Some(stats) = Stats::of(&estimates) {
        println!("{}", stats.row("estimate dB"));
    }

    for run in &runs {
        println!();
        println!("### Rendering through {}", run.label);
        println!();
        println!(
            "{:.2} s per song, {:.0} s for {} songs",
            run.seconds / run.lufs.len().max(1) as f64,
            run.seconds,
            run.lufs.len()
        );
        if let Some(stats) = Stats::of(&run.lufs) {
            println!();
            println!(
                "Levelling, against this bank's own mean of {:.1}:",
                stats.mean
            );
            println!();
            levelling_table(&run.lufs, &run.peak, stats.mean);
        }
    }

    if runs.len() > 1 {
        println!();
        println!("## Does a song's distance from the mean carry between banks");
        println!();
        println!("| banks | r | slope | residual sd (LU) |");
        println!("|---|---|---|---|");
        for (i, first) in runs.iter().enumerate() {
            for second in runs.iter().skip(i + 1) {
                let (slope, _, sd) = regress(&first.lufs, &second.lufs);
                println!(
                    "| {} against {} | {:.3} | {:.3} | {:.2} |",
                    first.label,
                    second.label,
                    pearson(&first.lufs, &second.lufs),
                    slope,
                    sd
                );
            }
        }
    }

    let paired: Vec<f64> = estimate_for_row.iter().filter_map(|e| *e).collect();
    if !runs.is_empty() && paired.len() > 1 {
        println!();
        println!("## Does the estimate predict the render");
        println!();
        println!("| bank | n | r | slope | residual sd (LU) |");
        println!("|---|---|---|---|---|");
        for run in &runs {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (index, estimated) in estimate_for_row.iter().enumerate() {
                if let (Some(value), Some(lufs)) = (estimated, run.lufs.get(index)) {
                    xs.push(*value);
                    ys.push(*lufs);
                }
            }
            let (slope, _, sd) = regress(&xs, &ys);
            println!(
                "| {} | {} | {:.3} | {:.3} | {:.2} |",
                run.label,
                xs.len(),
                pearson(&xs, &ys),
                slope,
                sd
            );
        }
    }

    Ok(())
}
