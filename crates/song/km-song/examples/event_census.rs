//! Counts what kinds of MIDI event a folder of karaoke files actually contains.
//!
//! ```text
//! cargo run --release -p km-song --example event_census -- <folder> [limit]
//! ```
//!
//! **It exists because "that message is rare in karaoke files" is a claim, and claims about the
//! corpus should be measurable rather than remembered.** One of them was wrong for years: aftertouch
//! was dropped on the floor partly because it was assumed rare, and channel pressure turns out to be
//! in 7.7% of files. The two numbers that matter for a question like that are different — how many
//! *files* carry a message at all, and how many events there are once one does — so both are
//! reported. A message in 8% of files at 800 events each is not the same proposition as one in 8% of
//! files at three.
//!
//! Report `files` when deciding whether to handle something, and `events` when deciding whether
//! handling it could cost anything.
//!
//! Unreadable files are counted, not fatal: the corpus is full of them and a census that stopped at
//! the first would never finish. `limit` caps how many files are read, for a sample rather than a
//! sweep — 25,000 files of the corpus took a couple of minutes and agreed with a 1,049-file
//! sample to within about a percentage point.

use km_song::{EventKind, ParseOptions, Song};

/// One row of the census.
#[derive(Default)]
struct Count {
    files: usize,
    events: u64,
}

impl Count {
    fn add(&mut self, in_this_file: u64) {
        if in_this_file > 0 {
            self.files += 1;
            self.events += in_this_file;
        }
    }
}

#[derive(Default)]
struct Census {
    note_on: Count,
    note_off: Count,
    controller: Count,
    program_change: Count,
    pitch_bend: Count,
    poly_aftertouch: Count,
    channel_aftertouch: Count,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("usage: event_census <folder> [limit]")?;
    let limit: usize = args
        .next()
        .map_or(usize::MAX, |n| n.parse().unwrap_or(usize::MAX));

    let mut census = Census::default();
    let mut scanned = 0usize;
    let mut unreadable = 0usize;

    // Explicit stack rather than recursion: the corpus nests deeply and unevenly, and a folder that
    // cannot be read is skipped rather than ending the walk.
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

            let mut per_file = [0u64; 7];
            for event in &song.events {
                let slot = match event.kind {
                    EventKind::NoteOn { .. } => 0,
                    EventKind::NoteOff { .. } => 1,
                    EventKind::Controller { .. } => 2,
                    EventKind::ProgramChange { .. } => 3,
                    EventKind::PitchBend { .. } => 4,
                    EventKind::PolyAftertouch { .. } => 5,
                    EventKind::ChannelAftertouch { .. } => 6,
                };
                per_file[slot] += 1;
            }
            census.note_on.add(per_file[0]);
            census.note_off.add(per_file[1]);
            census.controller.add(per_file[2]);
            census.program_change.add(per_file[3]);
            census.pitch_bend.add(per_file[4]);
            census.poly_aftertouch.add(per_file[5]);
            census.channel_aftertouch.add(per_file[6]);
        }
    }

    let readable = scanned - unreadable;
    println!("{scanned} files read, {unreadable} unreadable, {readable} parsed");
    println!();
    println!("| event | files | of parsed | events |");
    println!("|---|---|---|---|");
    for (name, count) in [
        ("note on", &census.note_on),
        ("note off", &census.note_off),
        ("controller", &census.controller),
        ("program change", &census.program_change),
        ("pitch bend", &census.pitch_bend),
        ("channel aftertouch", &census.channel_aftertouch),
        ("poly aftertouch", &census.poly_aftertouch),
    ] {
        let share = if readable == 0 {
            0.0
        } else {
            100.0 * count.files as f64 / readable as f64
        };
        println!(
            "| {name} | {} | {share:.2}% | {} |",
            count.files, count.events
        );
    }
    Ok(())
}
