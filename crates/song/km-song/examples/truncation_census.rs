//! Counts how much of a folder of karaoke files the parser cannot read to the end.
//!
//! ```text
//! cargo run --release -p km-song --example truncation_census -- <folder> [limit]
//! ```
//!
//! **It exists because the two defects it counts are invisible by construction.** A track midly
//! abandons mid-stream produces no error and no short read anybody can see; a file that ends holding
//! a note is not malformed at all. Both used to end the same way — a note sounding until the process
//! died — and neither could be counted before `Song` carried `truncated_tracks` and `repaired_notes`.
//!
//! The first four buckets are **not** mutually exclusive, and the report says so: a truncated file
//! usually has dangling notes too, because losing the rest of a track is exactly how its note-offs go
//! missing. `clean` is the count of files in none of them, so the columns do not sum to the total
//! and are not meant to.
//!
//! **The last four are the hold pedal**, which is a third way to arrive at a note that never stops
//! and the only one that survives the synthesizer's envelope — `rustysynth` holds every released
//! voice while CC64 is down. The first three of them measure how far the seek defect can reach
//! rather than anything about the file's condition; the fourth measures a defect that needs no seek,
//! where a lift and the press after it share one render block. `hold_pedal` explains why the four
//! are separate questions.
//!
//! `--verbose` names each file in the first three buckets, which is what to reach for when a number
//! moves and the question is which files moved it.
//!
//! Unreadable files are counted, not fatal: a corpus is full of them and a census that stopped at
//! the first would never finish. `limit` caps how many files are read, for a sample rather than a
//! sweep.

use km_song::{EventKind, ParseOptions, Song};

/// What a single file turned out to be.
#[derive(Default)]
struct Tally {
    /// Files with at least one track the parser could not finish.
    truncated: usize,
    /// Files the header promised more tracks than it delivered.
    missing: usize,
    /// Files that ended holding at least one note, however they got there.
    dangling: usize,
    /// Files in none of the three buckets above.
    clean: usize,
    /// Note-offs synthesized across every file.
    repaired_notes: usize,
    /// Tracks abandoned across every file.
    truncated_tracks: usize,
    /// Files that touch the hold pedal at all. The seek defect cannot reach the others.
    uses_hold_pedal: usize,
    /// Files that end with the pedal down on at least one channel.
    pedal_down_at_end: usize,
    /// Files where the pedal is down across a stretch long enough for a seek to land inside it.
    pedal_held_over_a_gap: usize,
    /// Files where at least one pedal lift falls inside a render block, so the synthesizer never
    /// observes it.
    pedal_lift_cancelled: usize,
    /// Pedal lifts cancelled that way, across every file.
    cancelled_lifts: usize,
    /// Pedal lifts with a block boundary to fall in, across every file.
    honoured_lifts: usize,
    /// The most note-offs any one file leaves deferred at one moment by cancelled lifts.
    worst_stranded: usize,
}

/// Ticks the pedal must stay down before a seek landing inside it is worth counting.
///
/// A quarter note at the usual 480 ticks is far too short to matter; this is four whole notes, or
/// roughly eight seconds at 120 BPM. The point is to separate a pedal used as a pedal from one left
/// down over a section, which is the shape a backwards seek can strand.
const LONG_HOLD_TICKS: u32 = 480 * 16;

/// One render block of song time, at the block size and rate the machine plays at.
///
/// The sequencer dispatches every event due at or before the target tick and then renders, so two
/// events closer together than this land in one block and the level between them is never observed.
/// Measured in microseconds rather than ticks because a block is wall-clock and a tick is not: one
/// tick at 480 per quarter and 120 BPM is 1.04 ms, which is most of a block on its own.
const BLOCK_US: u64 = 64 * 1_000_000 / 44_100;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut root = None;
    let mut limit = usize::MAX;
    let mut verbose = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--verbose" | "-v" => verbose = true,
            other if root.is_none() => root = Some(other.to_owned()),
            other => limit = other.parse().unwrap_or(usize::MAX),
        }
    }
    let root = root.ok_or("usage: truncation_census <folder> [limit] [--verbose]")?;

    let mut tally = Tally::default();
    let mut scanned = 0usize;
    let mut unreadable = 0usize;

    // Explicit stack rather than recursion, and a folder that cannot be read is skipped rather than
    // ending the walk -- the same shape as `event_census`, for the same reasons.
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
            scanned += 1;
            let Ok(bytes) = std::fs::read(&path) else {
                unreadable += 1;
                continue;
            };
            let Ok(song) = Song::parse(&bytes, &ParseOptions::default()) else {
                unreadable += 1;
                continue;
            };

            let truncated = !song.truncated_tracks.is_empty();
            let missing = song.missing_tracks > 0;
            let dangling = song.repaired_notes > 0;

            tally.truncated += usize::from(truncated);
            tally.missing += usize::from(missing);
            tally.dangling += usize::from(dangling);
            tally.clean += usize::from(!truncated && !missing && !dangling);
            tally.repaired_notes += song.repaired_notes;
            tally.truncated_tracks += song.truncated_tracks.len();

            let pedal = hold_pedal(&song);
            tally.uses_hold_pedal += usize::from(pedal.used);
            tally.pedal_down_at_end += usize::from(pedal.down_at_end);
            tally.pedal_held_over_a_gap += usize::from(pedal.longest_hold >= LONG_HOLD_TICKS);
            tally.pedal_lift_cancelled += usize::from(pedal.cancelled_lifts > 0);
            tally.cancelled_lifts += pedal.cancelled_lifts;
            tally.honoured_lifts += pedal.honoured_lifts;
            tally.worst_stranded = tally.worst_stranded.max(pedal.stranded_peak);

            if verbose && (truncated || missing || dangling) {
                println!(
                    "{}\ttruncated={:?}\tmissing={}\trepaired={}",
                    path.display(),
                    song.truncated_tracks,
                    song.missing_tracks,
                    song.repaired_notes
                );
            }

            // The invariant the repair exists to establish, checked on real files rather than only
            // on fixtures: nothing may still be sounding once the events run out.
            if let Some(note) = still_sounding(&song) {
                println!(
                    "STILL SOUNDING {} channel {} key {}",
                    path.display(),
                    note.0,
                    note.1
                );
            }
        }
    }

    let readable = scanned - unreadable;
    println!();
    println!("{scanned} files read, {unreadable} unreadable, {readable} parsed");
    println!();
    println!("| finding | files | of parsed |");
    println!("|---|---:|---:|");
    for (name, files) in [
        ("truncated mid-track", tally.truncated),
        ("tracks declared but missing", tally.missing),
        ("ended holding a note", tally.dangling),
        ("clean", tally.clean),
        ("uses the hold pedal", tally.uses_hold_pedal),
        ("pedal down at the end", tally.pedal_down_at_end),
        (
            "pedal held over a seekable stretch",
            tally.pedal_held_over_a_gap,
        ),
        (
            "a pedal lift cancelled inside a block",
            tally.pedal_lift_cancelled,
        ),
    ] {
        let share = if readable == 0 {
            0.0
        } else {
            100.0 * files as f64 / readable as f64
        };
        println!("| {name} | {files} | {share:.2}% |");
    }
    println!();
    println!(
        "{} tracks abandoned, {} note-offs synthesized",
        tally.truncated_tracks, tally.repaired_notes
    );
    println!(
        "{} of {} pedal lifts cancelled inside a block ({:.2}%); the worst file leaves {} notes \
         deferred at once",
        tally.cancelled_lifts,
        tally.cancelled_lifts + tally.honoured_lifts,
        if tally.cancelled_lifts + tally.honoured_lifts == 0 {
            0.0
        } else {
            100.0 * tally.cancelled_lifts as f64
                / (tally.cancelled_lifts + tally.honoured_lifts) as f64
        },
        tally.worst_stranded
    );
    println!("The first three overlap; `clean` is the count in none of them, so they do not sum.");
    Ok(())
}

/// What a file does with the hold pedal.
struct HoldPedal {
    /// CC64 appears at all.
    used: bool,
    /// It is down on at least one channel at the last event.
    down_at_end: bool,
    /// The longest run of ticks it stayed down on any one channel.
    longest_hold: u32,
    /// Lifts whose next press lands inside the same render block.
    cancelled_lifts: usize,
    /// Lifts with a block boundary to fall in.
    honoured_lifts: usize,
    /// The most note-offs deferred at one moment, counting a cancelled lift as releasing nothing.
    stranded_peak: usize,
}

/// Measures a file's use of CC64, which is what decides whether either pedal defect can reach it.
///
/// **The four answers are not the same question, and two of them are defects.** A seek replays
/// controllers stated *before* the target, so a pedal is stranded by a seek only when it goes down
/// before the point being seeked away from and is not re-stated before the point being seeked to —
/// which needs it held across a stretch a seek can land inside. `down_at_end` is the proxy the
/// working note used and is kept for comparison; it over-counts, because a pedal pressed in the
/// final bar strands nothing.
///
/// `cancelled_lifts` is the other defect and needs no seek at all. A lift whose press follows it
/// inside one render block is never observed as a position, so every note-off the pedal deferred
/// goes on sounding — see [`BLOCK_US`].
///
/// `stranded_peak` counts *notes*, and one note is one voice per matching instrument region, so the
/// voice figure on a layered bank is higher. It also assumes a deferred note is still worth
/// counting until a lift is observed, which is true of a sustaining patch and generous to a
/// percussive one.
///
/// 64 is the spec's threshold: 0–63 is off, 64–127 is on.
fn hold_pedal(song: &Song) -> HoldPedal {
    let mut down_since: [Option<u32>; 16] = [None; 16];
    let mut used = false;
    let mut longest_hold = 0u32;

    // The block-boundary pass. `lifted_at` is the lift waiting to find out whether the next press
    // cancels it, and `deferred` the note-offs it would have released.
    let mut lifted_at: [Option<u64>; 16] = [None; 16];
    let mut deferred = [0usize; 16];
    let mut cancelled_lifts = 0usize;
    let mut honoured_lifts = 0usize;
    let mut stranded_peak = 0usize;

    for event in &song.events {
        match event.kind {
            // A note-off under a pedal that is down is a release the synthesizer defers.
            EventKind::NoteOff { channel, .. } => {
                let Some(slot) = down_since.get(usize::from(channel)) else {
                    continue;
                };
                if slot.is_some() {
                    deferred[usize::from(channel)] += 1;
                    stranded_peak = stranded_peak.max(deferred.iter().sum());
                }
            }
            EventKind::Controller {
                channel,
                controller: 64,
                value,
            } => {
                used = true;
                let Some(slot) = down_since.get_mut(usize::from(channel)) else {
                    continue;
                };
                let index = usize::from(channel);
                match (value >= 64, *slot) {
                    // Pressed, and not already down. A lift waiting on this press is cancelled if
                    // the press is close enough to share its block.
                    (true, None) => {
                        *slot = Some(event.tick);
                        if let Some(at) = lifted_at[index].take() {
                            let us = song.tempo_map.tick_to_us(event.tick);
                            if us.saturating_sub(at) < BLOCK_US {
                                cancelled_lifts += 1;
                            } else {
                                honoured_lifts += 1;
                                deferred[index] = 0;
                            }
                        }
                    }
                    // Released: close the run, and start waiting on the next press.
                    (false, Some(since)) => {
                        longest_hold = longest_hold.max(event.tick.saturating_sub(since));
                        *slot = None;
                        lifted_at[index] = Some(song.tempo_map.tick_to_us(event.tick));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // A lift nothing presses after is honoured: the block that follows it observes the pedal up.
    honoured_lifts += lifted_at.iter().flatten().count();

    // A pedal still down at the end is held to the end of the song, not dropped.
    let mut down_at_end = false;
    for since in down_since.into_iter().flatten() {
        down_at_end = true;
        longest_hold = longest_hold.max(song.duration_ticks.saturating_sub(since));
    }

    HoldPedal {
        used,
        down_at_end,
        longest_hold,
        cancelled_lifts,
        honoured_lifts,
        stranded_peak,
    }
}

/// A note-on with no note-off after it, or `None` if every note is turned off.
fn still_sounding(song: &Song) -> Option<(u8, u8)> {
    let mut down: Vec<(u8, u8)> = Vec::new();
    for event in &song.events {
        match event.kind {
            EventKind::NoteOn { channel, key, .. } => {
                if !down.contains(&(channel, key)) {
                    down.push((channel, key));
                }
            }
            EventKind::NoteOff { channel, key } => {
                if let Some(at) = down.iter().position(|&n| n == (channel, key)) {
                    down.swap_remove(at);
                }
            }
            _ => {}
        }
    }
    down.first().copied()
}
