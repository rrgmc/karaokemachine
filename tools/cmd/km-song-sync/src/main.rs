//! The windowed executable: `km-song-sync`.
//!
//! The attribute is why this file is separate from `src/bin/km-song-sync-console.rs`:
//! `#![windows_subsystem]` applies to a binary crate root, and everything real is in `src/lib.rs`.
//! `km-package-builder`'s `main.rs` states the whole argument, and the `Two executables on Windows`
//! decision in `docs/decisions/distribution.md` is the rule.
#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    km_song_sync::run(km_song_sync::Shell::Windowed)
}
