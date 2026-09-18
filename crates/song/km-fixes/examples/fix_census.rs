//! Counts how many karaoke files in a folder carry a defect this crate can correct.
//!
//! ```text
//! cargo run --release -p km-fixes --example fix_census -- <folder> [limit] [stride]
//! ```
//!
//! **`stride` is what makes a partial run a sample rather than a corner of the corpus.** A limit on
//! its own takes the first files the walk reaches, and a walk reaches them a folder at a time — so
//! a corpus whose folders came from different sources gives a figure about whichever few happened
//! to be first. Reading every twenty-fourth file spreads the same number of reads across the whole
//! of it.
//!
//! **It exists because a rate quoted in a decision has to be one somebody measured**, and because
//! the obvious way to measure this one is wrong in both directions. Searching the bytes for
//! `Bn 00 7F` finds that pattern wherever it sits — inside a track name, a delta time, the middle
//! of a multi-byte value — and none of those is a bank select; it also misses every such message
//! written under running status, which carries no status byte to match. Over the same corpus the
//! two errors did not cancel, and the byte search came out lower. This parses instead, so what it
//! counts is events.
//!
//! It reports files rather than events, which is the useful number here: a fix is a decision about
//! a *song*, and a file carrying one stray bank select and a file carrying forty are the same
//! decision. `event_census` beside this one explains when the other number matters.
//!
//! Unreadable files are counted, not fatal: the corpus is full of them and a census that stopped at
//! the first would never finish. `limit` caps how many files are read, for a sample rather than a
//! sweep — the repository quotes rates over 25,000 files.

use km_fixes::Fix;
use km_song::{ParseOptions, Song};

/// How many files carry each kind of fix, and how many carry any.
#[derive(Default)]
struct Census {
    ignore_bank_select: usize,
    recentre_bend: usize,
    any: usize,
    /// Files with a channel over each stranded-bend threshold, indexed like [`CENTS`] and [`ONSETS`].
    stranded: [[usize; ONSETS.len()]; CENTS.len()],
    /// A few files the detector proposes a recentre for, so they can be listened to.
    examples: Vec<String>,
}

/// The stranded-bend offsets swept, in cents, so the detector's own threshold is chosen from a table.
const CENTS: [f32; 5] = [5.0, 10.0, 20.0, 35.0, 50.0];

/// The note starts a channel must play on a stranded bend, swept beside [`CENTS`].
const ONSETS: [usize; 4] = [1, 2, 4, 8];

/// How many flagged files are named at the end.
const EXAMPLES: usize = 25;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .ok_or("usage: fix_census <folder> [limit] [stride]")?;
    let limit: usize = args
        .next()
        .map_or(usize::MAX, |n| n.parse().unwrap_or(usize::MAX));
    let stride: usize = args.next().map_or(1, |n| n.parse().unwrap_or(1)).max(1);

    let mut census = Census::default();
    let mut scanned = 0usize;
    let mut unreadable = 0usize;
    // Eligible files met, so only every `stride`-th of them is read.
    let mut seen = 0usize;

    // Explicit stack rather than recursion, and a folder that cannot be read is skipped rather than
    // ending the walk. The same shape `event_census` uses, and for the same corpus.
    let mut stack = vec![std::path::PathBuf::from(&root)];
    while let Some(dir) = stack.pop() {
        if scanned >= limit {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scanned >= limit {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(extension.as_str(), "mid" | "midi" | "kar") {
                continue;
            }
            seen += 1;
            if !seen.is_multiple_of(stride) {
                continue;
            }
            scanned += 1;
            if scanned.is_multiple_of(2_000) {
                eprintln!("  {scanned} read, {} flagged", census.any);
            }
            let Ok(bytes) = std::fs::read(&path) else {
                unreadable += 1;
                continue;
            };
            let Ok(song) = Song::parse(&bytes, &ParseOptions::default()) else {
                unreadable += 1;
                continue;
            };

            for (row, cents) in CENTS.iter().enumerate() {
                let most = km_fixes::recentre_bend::stranded_onsets(&song, *cents)
                    .into_iter()
                    .max()
                    .unwrap_or(0);
                for (column, onsets) in ONSETS.iter().enumerate() {
                    if most >= *onsets {
                        census.stranded[row][column] += 1;
                    }
                }
            }

            let fixes = km_fixes::detect(&song);
            if fixes.is_empty() {
                continue;
            }
            census.any += 1;
            if fixes
                .iter()
                .any(|fix| matches!(fix, Fix::IgnoreBankSelect { .. }))
            {
                census.ignore_bank_select += 1;
            }
            let recentred: Vec<u8> = fixes
                .iter()
                .filter(|fix| matches!(fix, Fix::RecentreBend { .. }))
                .filter_map(Fix::channel)
                .collect();
            if !recentred.is_empty() {
                census.recentre_bend += 1;
                if census.examples.len() < EXAMPLES {
                    census
                        .examples
                        .push(format!("{} {recentred:?}", path.display()));
                }
            }
        }
    }

    let readable = scanned - unreadable;
    let share = |count: usize| {
        if readable == 0 {
            0.0
        } else {
            100.0 * count as f64 / readable as f64
        }
    };
    if stride > 1 {
        println!("every {stride}th of {seen} files met");
    }
    println!("{scanned} files read, {unreadable} unreadable, {readable} parsed");
    println!();
    println!("| fix | files | of parsed |");
    println!("|---|---|---|");
    println!(
        "| ignore bank select | {} | {:.2}% |",
        census.ignore_bank_select,
        share(census.ignore_bank_select)
    );
    println!(
        "| recentre bend | {} | {:.2}% |",
        census.recentre_bend,
        share(census.recentre_bend)
    );
    println!("| any | {} | {:.2}% |", census.any, share(census.any));
    println!();
    println!(
        "Files with a channel playing at least N note starts on a bend left off by at least C cents:"
    );
    println!();
    print!("| cents |");
    for onsets in ONSETS {
        print!(" N={onsets} |");
    }
    println!();
    println!("|---|{}", "---|".repeat(ONSETS.len()));
    for (row, cents) in CENTS.iter().enumerate() {
        print!("| {cents} |");
        for count in census.stranded[row] {
            print!(" {count} ({:.2}%) |", share(count));
        }
        println!();
    }
    if !census.examples.is_empty() {
        println!();
        for example in &census.examples {
            println!("{example}");
        }
    }
    Ok(())
}
