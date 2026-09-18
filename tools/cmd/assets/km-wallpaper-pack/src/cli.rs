//! The command line.
//!
//! The three phases are separate commands on purpose. `fetch` is the only one that touches the
//! network and the only one that costs quota; `analyze` and `build` are pure functions of the cache,
//! so a threshold can be tuned twenty times in an afternoon without asking a provider for anything.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Builds a legibility-verified wallpaper pack from royalty-free stock photography.
#[derive(Debug, Parser)]
#[command(name = "km-wallpaper-pack", version, about, long_about = None)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,

    /// Where downloaded originals and search responses are kept.
    #[arg(long, default_value = "./.wpcache", global = true)]
    pub cache_dir: PathBuf,

    /// Where the built pack goes.
    #[arg(long, default_value = "./out", global = true)]
    pub out: PathBuf,

    // Why the default is the local overlay rather than `assets/wallpapers`: `km-display` reads a zip
    // in the wallpaper folder as a folder of images (`ARCHIVE_EXTENSIONS` in
    // `crates/playback/km-display/src/wallpaper.rs`), and the machine prefers `local/assets/` to
    // `assets/` when run from a checkout — so a pack dropped there is picked up by `cargo run` and
    // by nothing else, `/local/` being gitignored wholesale.
    //
    // Every staging script copies `assets/` into the build it stages, so a pack put there travels
    // into the Windows folder, the macOS bundle, the tarball, `dist/bin`, the installer and the APK.
    // Two carriers still will not take one, deliberately: the `.deb` lists
    // `assets/wallpapers/*.png`, so a `*.zip` entry would fail every build on a machine that has
    // never run this tool; the APK copies everything under `assets/` and counts it against the size
    // warning in `tools/port/machine/android/assets.sh`.
    /// Where to also copy the finished zip.
    ///
    /// The pack itself is built in `--out`, and this is a copy. Use `--zip-dest ./out` for no copy.
    ///
    /// The default puts the pack where a `cargo run` from this checkout will show it, and where no
    /// release can pick it up. To include a pack in a release, use
    /// `--zip-dest ./assets/wallpapers`.
    ///
    /// Note that a wallpaper folder in `local/assets/` replaces the bundled pictures rather than
    /// adding to them, so the four built-in gradients are not shown while a pack is there. Copy them
    /// across if you want both.
    #[arg(long, default_value = "./local/assets/wallpapers", global = true)]
    pub zip_dest: PathBuf,

    /// How many images to process at once. Defaults to the number of cores.
    #[arg(long, global = true)]
    pub jobs: Option<usize>,

    /// Say what would happen without writing anything.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Machine-readable output on stdout, for scripts and agents.
    #[arg(long, global = true)]
    pub json: bool,

    /// More logging. Repeat for more still.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Search the providers and download originals into the cache.
    Fetch {
        /// The config file.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
        /// Re-ask the providers even when a cached response exists.
        #[arg(long)]
        refresh: bool,
    },
    /// Measure, deduplicate and select, writing `analysis.json`. No network.
    Analyze {
        /// The config file.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
        /// Re-decode every image instead of reusing cached measurements.
        ///
        /// Needed only when this tool's own measuring code changes: the cache key covers the settings
        /// a measurement depends on, but it cannot see an edit to `metrics.rs` or `process.rs`.
        #[arg(long)]
        remeasure: bool,
    },
    /// Process the selected images and write the pack. No network.
    Build {
        /// The config file.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
        /// Replace the pack an output directory already holds.
        #[arg(long)]
        force: bool,
    },
    /// Fetch, then analyze, then build.
    All {
        /// The config file.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
        /// Replace the pack an output directory already holds.
        #[arg(long)]
        force: bool,
        /// Re-decode every image instead of reusing cached measurements. See `analyze --remeasure`.
        #[arg(long)]
        remeasure: bool,
    },
    // For the set that ships with the machine: six or eight pictures chosen by eye rather than a
    // hundred filtered out of thousands, so there is nothing to search, no cache and no config file.
    /// Build a pack from images you chose yourself, with a sidecar saying where each came from.
    ///
    /// Runs the same contrast check as a searched pack. Every image in the folder must be listed in
    /// the sidecar, and an image that fails the check stops the run rather than being dropped.
    Local {
        /// The folder of images to build from.
        #[arg(long, default_value = "./picks")]
        dir: PathBuf,
        /// Where each image came from and under what license. Defaults to `credits.toml` in `--dir`.
        #[arg(long)]
        credits: Option<PathBuf>,
        /// Replace the pack an output directory already holds.
        #[arg(long)]
        force: bool,
    },
    /// Re-check a built pack against its own contrast gate.
    Verify {
        /// The pack directory.
        #[arg(long, default_value = "./out")]
        pack: PathBuf,
    },
}

impl Cli {
    /// The tracing filter this verbosity asks for.
    ///
    /// `RUST_LOG` wins when it is set, which is what makes a one-off "show me everything from the
    /// cache layer" possible without adding a flag for it.
    pub fn log_filter(&self) -> String {
        if let Ok(filter) = std::env::var("RUST_LOG") {
            return filter;
        }
        match self.verbose {
            0 => "km_wallpaper_pack=info".to_owned(),
            1 => "km_wallpaper_pack=debug".to_owned(),
            _ => "km_wallpaper_pack=trace,reqwest=debug".to_owned(),
        }
    }

    /// How many images to work on at once.
    pub fn parallelism(&self) -> usize {
        self.jobs
            .filter(|jobs| *jobs > 0)
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_valid() {
        // clap's own audit: duplicate flags, a global on a subcommand, a bad default.
        Cli::command().debug_assert();
    }

    #[test]
    fn the_defaults_are_the_ones_the_readme_promises() {
        let cli = Cli::try_parse_from(["km-wallpaper-pack", "analyze"]).expect("parse");
        assert_eq!(cli.cache_dir, PathBuf::from("./.wpcache"));
        assert_eq!(cli.out, PathBuf::from("./out"));
        // The pack still lands in ./out; the zip is additionally copied into the checkout's local
        // asset overlay, which `cargo run` reads and which no staging script can see. Asserted by
        // literal rather than by a constant on purpose: this default decides whether tens of
        // megabytes of somebody else's photographs end up in every release, so a change to it should
        // have to come here and be read.
        assert_eq!(cli.zip_dest, PathBuf::from("./local/assets/wallpapers"));
        assert!(!cli.dry_run);
        assert!(!cli.json);
    }

    #[test]
    fn verbosity_raises_the_filter_and_rust_log_beats_it() {
        let quiet = Cli::try_parse_from(["km-wallpaper-pack", "analyze"]).expect("parse");
        let loud = Cli::try_parse_from(["km-wallpaper-pack", "-vv", "analyze"]).expect("parse");
        // Not asserted against a live RUST_LOG, which a developer may well have set.
        if std::env::var("RUST_LOG").is_err() {
            assert_eq!(quiet.log_filter(), "km_wallpaper_pack=info");
            assert!(loud.log_filter().contains("trace"));
        }
    }

    #[test]
    fn jobs_defaults_to_the_machine_and_rejects_zero() {
        let cli =
            Cli::try_parse_from(["km-wallpaper-pack", "--jobs", "0", "analyze"]).expect("parse");
        assert!(cli.parallelism() >= 1, "zero jobs would do nothing");
        let three =
            Cli::try_parse_from(["km-wallpaper-pack", "--jobs", "3", "analyze"]).expect("parse");
        assert_eq!(three.parallelism(), 3);
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        // Which is where anybody actually types them.
        let cli = Cli::try_parse_from(["km-wallpaper-pack", "build", "--force", "--json"])
            .expect("parse");
        assert!(cli.json);
        assert!(matches!(cli.command, Command::Build { force: true, .. }));
    }
}
