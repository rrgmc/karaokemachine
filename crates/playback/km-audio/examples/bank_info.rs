//! Prints what a SoundFont bank contains, and what loading it cost.
//!
//! ```text
//! cargo run --release -p km-audio --example bank_info -- <bank.sf2> [more.sf2 ...]
//! ```
//!
//! Sample size, preset and instrument counts, drum kits, melodic banks and modulator counts. See
//! `crates/machine/km-banks/data/soundfont-banks.conf`, where the per-bank numbers live, for what
//! they mean.
//!
//! **Nothing here walks RIFF.** Every figure comes from the synthesizer's own accessors, which is
//! both less code and a better measurement: it reports what *this machine* will actually play, not
//! what the file claims. A bank whose records are dropped on load reports the smaller counts, and
//! says how many went.
//!
//! It lives in `km-audio` because this is the only crate that may name `rustysynth`
//! (`docs/ARCHITECTURE.md`), and every one of these numbers needs it. **It calls `rustysynth`
//! directly rather than going through [`km_audio::Bank`]**, deliberately: `Bank` wraps the
//! synthesizer's type precisely so that nothing outside this crate has to name it, and giving it a
//! `soundfont()` accessor to satisfy one diagnostic would undo that for every caller. An example
//! is already inside the boundary, so it can reach for the dependency without widening the API.
//!
//! ## Reading the counts
//!
//! **The research note excludes the terminal record** that SF2 requires at the end of the preset,
//! instrument and sample-header tables, and says so: that convention is why its 287/920 for
//! GeneralUser GS differs from an earlier 288/921 for the same file. `rustysynth` drops those
//! records while parsing, so its slices are already the note's convention and nothing is subtracted
//! here.

use std::time::Instant;

/// SF2 puts every percussion preset in bank 128; everything else is melodic.
const DRUM_BANK: i32 = 128;

/// How many dropped records to quote before counting the rest.
const DEFECTS_SHOWN: usize = 5;

/// How much of an INFO string to show before counting the rest of it.
///
/// GeneralUser GS's `ICMT` is its entire license, several kilobytes of it, and a survey run prints
/// fifty banks at once. Enough to see whose bank it is and under what terms; not enough to bury the
/// counts below it.
const INFO_CHARS: usize = 300;

/// Collapses an INFO string onto one line, truncating a long one.
///
/// These carry embedded newlines — a copyright notice is often three lines — and a table that has
/// one bank per block cannot have one field spanning four of them.
fn one_line(value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = flat.chars().count();
    if chars <= INFO_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(INFO_CHARS).collect();
    format!("{head}... ({chars} chars in all)")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: bank_info <bank.sf2> [more.sf2 ...]");
        std::process::exit(2);
    }

    let mut failures = 0;
    for path in &paths {
        if paths.len() > 1 {
            println!();
        }
        println!("{path}");

        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) => {
                println!("  load        UNREADABLE");
                println!("  error       {error}");
                failures += 1;
                continue;
            }
        };
        let started = Instant::now();
        let font = match rustysynth::SoundFont::new(&mut file) {
            Ok(font) => font,
            Err(error) => {
                // The verbatim message, because which one it is decides what to do about it: an
                // unsupported sample format is a different file, a sanity-check failure a
                // different synthesizer, and a missing chunk a broken download.
                println!("  load        FAILED after {:?}", started.elapsed());
                println!("  error       {error}");
                failures += 1;
                continue;
            }
        };
        let elapsed = started.elapsed();

        let info = font.get_info();
        let version = info.get_version();
        let presets = font.get_presets();
        let instruments = font.get_instruments();
        let samples = font.get_sample_headers();

        // A `smpl` chunk is 16-bit samples, so the byte count is twice the slice. `sm24`, the
        // 24-bit extension, is discarded by this synthesizer -- which `bits_per_sample` reports,
        // and which is why a 24-bit bank costs the same RAM here as a 16-bit one.
        let smpl_bytes = font.get_wave_data().len() * 2;
        let kits = presets
            .iter()
            .filter(|preset| preset.get_bank_number() == DRUM_BANK)
            .count();
        let melodic: std::collections::BTreeSet<i32> = presets
            .iter()
            .map(|preset| preset.get_bank_number())
            .filter(|bank| *bank != DRUM_BANK)
            .collect();
        let pmod: usize = presets
            .iter()
            .flat_map(|preset| preset.get_regions())
            .map(|region| region.get_modulators().len())
            .sum();
        let imod: usize = instruments
            .iter()
            .flat_map(|instrument| instrument.get_regions())
            .map(|region| region.get_modulators().len())
            .sum();

        println!("  load        OK in {elapsed:?}");
        println!(
            "  version     SF v{}.{}",
            version.get_major(),
            version.get_minor()
        );
        println!("  bank name   {}", info.get_bank_name());

        // `soundfont-banks.conf` rates every bank's terms, and the survey marked a finding
        // **[bytes]** exactly when it read them out of the file instead of off a mirror — `ICOP`,
        // `ICMT` and `IENG`. That was done by hand for fifteen banks and does not scale to fifty,
        // which is the whole reason these lines are here.
        //
        // **Only the fields a bank actually carries are printed.** Most of these are empty in most
        // files -- several banks in the survey have no `ICOP` at all, which is itself a finding --
        // and seven blank lines per bank would bury the counts below.
        for (label, value) in [
            ("author", info.get_author()),
            ("product", info.get_target_product()),
            ("engine", info.get_target_sound_engine()),
            ("created", info.get_creation_date()),
            ("tools", info.get_tools()),
            ("copyright", info.get_copyright()),
            ("comments", info.get_comments()),
        ] {
            if !value.trim().is_empty() {
                println!("  {label:<12}{}", one_line(value));
            }
        }

        println!(
            "  smpl        {smpl_bytes} bytes ({:.1} MiB)",
            smpl_bytes as f64 / (1024.0 * 1024.0)
        );
        println!("  bits        {}", font.get_bits_per_sample());
        println!("  presets     {}", presets.len());
        println!("  instruments {}", instruments.len());
        println!("  samples     {}", samples.len());
        println!("  kits        {kits}");
        println!("  melodic     {} bank(s)", melodic.len());
        println!("  modulators  {pmod} pmod / {imod} imod");

        // More than the one line `BankDefects` produces for a log entry: this is the tool somebody
        // uses when they want to know *what* a bank lost, so it lists rather than summarizing.
        let dropped = font.get_warning_count();
        if dropped == 0 {
            println!("  defects     none");
        } else {
            println!("  defects     {dropped}");
            for warning in font.get_warnings().iter().take(DEFECTS_SHOWN) {
                println!("              {warning}");
            }
            let shown = font.get_warnings().len().min(DEFECTS_SHOWN);
            if dropped > shown {
                println!("              ... and {} more", dropped - shown);
            }
        }
    }

    if failures > 0 {
        eprintln!();
        eprintln!("{failures} of {} did not load", paths.len());
    }
    Ok(())
}
