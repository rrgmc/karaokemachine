//! Builds a legibility-verified wallpaper pack from royalty-free stock photography.
//!
//! The karaoke display draws outlined near-white lyrics over whatever wallpaper is showing, so a
//! photograph is a gamble twice over: it needs a license, and it has to stay calm behind two lines
//! of text for three minutes.
//!
//! **This tool settles the second, by measurement**: a WCAG contrast ratio computed in the band
//! where the lyrics actually are, so a hundred photographs are accepted or rejected without anybody
//! eyeballing them.
//!
//! **It does not settle the first.** "Stock APIs for the license" answers
//! whether the app may *show* a photograph and is silent on whether a pack may be *passed on* —
//! which is a different question, and one Pixabay and Pexels both answer no. A pack built from
//! either is for the machine that built it. What may be shipped comes from sources that grant
//! redistribution, recorded per image rather than per provider; `commands::local` is how the set
//! the machine ships was built.
//!
//! **The pack is a folder of images, not a database.** The app scans its wallpaper folder and reads
//! zip files in it as folders of wallpapers, so the deliverable is one zip to drop in. `manifest.json`
//! and `ATTRIBUTION.md` travel inside it for provenance and for a credits screen; nothing at run time
//! depends on them.
//!
//! **The pack does not darken anything.** The display already lays `wallpaper.dim` (45% by default)
//! over every wallpaper. So the contrast solver is used as a *filter*: an image ships when the
//! darkening it needs is no more than the darkening it will get. Baking a second scrim on top would
//! darken twice and fight the user's own setting.
//!
//! Three phases, because two of them are pure functions of the cache and can be re-run all afternoon
//! while thresholds are tuned:
//!
//! * [`commands::fetch`] — search results and original bytes into the cache. The only phase that
//!   touches the network.
//! * [`commands::analyze`] — metrics, dedupe and selection over the cache alone.
//! * [`commands::build`] — crop, resize, blur, encode, manifest, zip.

pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod curated;
pub mod dedupe;
pub mod error;
pub mod license;
pub mod manifest;
pub mod metrics;
pub mod process;
pub mod progress;
pub mod providers;
pub mod select;

pub use crate::config::Config;
pub use crate::error::{Error, Result};

/// The tool's version, for the manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
