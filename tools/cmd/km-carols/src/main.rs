//! Builds the Christmas carol pack's songs and its description.
//!
//! `tools/dist/carols.sh` is what you normally run: it fetches and verifies the pinned hymnal,
//! calls this, and then calls `km-pack build` on the description this writes.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use km_carols::{Options, selection};

/// Turn the pinned Open Hymnal ABC into karaoke MIDI for the Christmas carol pack.
#[derive(Parser)]
#[command(name = "km-carols", version, about, long_about = None)]
struct Cli {
    /// The pinned hymnal, already fetched and verified by tools/dist/carols.sh.
    #[arg(long)]
    source: PathBuf,

    /// Where to write songs/, the .kmspec.yaml and CREDITS.md.
    #[arg(long, default_value = "dist/carols")]
    out: PathBuf,

    /// The abc2midi to run. It is a build-time tool and is neither linked nor shipped.
    #[arg(long, default_value = "abc2midi")]
    abc2midi: PathBuf,

    /// The pack's own version, which is not the workspace's.
    #[arg(long, default_value = "1.0.0")]
    pack_version: String,

    /// Keep the generated ABC beside each song, for when one sounds wrong.
    #[arg(long)]
    keep_abc: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = Options {
        source: cli.source,
        out: cli.out,
        abc2midi: cli.abc2midi,
        version: cli.pack_version,
        keep_abc: cli.keep_abc,
    };

    match km_carols::build(&options) {
        Ok(built) => {
            for song in &built {
                let b = song.breakdown;
                println!(
                    "  {:>3}  {:<36} {:<20} {} verses  {:>3} syll  {}:{:02}  {}/10 \
                     (lyrics {}, sync {}, channels {}, arrangement {})",
                    song.number,
                    song.title,
                    song.artist,
                    song.verses,
                    song.syllables,
                    song.duration_ms / 60_000,
                    (song.duration_ms / 1_000) % 60,
                    song.suitability,
                    b.lyrics,
                    b.sync,
                    b.channels,
                    b.arrangement,
                );
            }
            let total: u64 = built.iter().map(|s| s.duration_ms).sum();
            println!(
                "\n{} carols, {} minutes of singing, described in {}.kmspec.yaml",
                built.len(),
                total / 60_000,
                selection::PACKAGE_ID,
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("km-carols: {error:#}");
            ExitCode::FAILURE
        }
    }
}
