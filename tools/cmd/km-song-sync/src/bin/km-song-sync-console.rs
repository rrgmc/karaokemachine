//! The console executable: `km-song-sync-console`.
//!
//! The same library with no window, so `--help`, the version and a refusal to start print where
//! somebody can read them. Built only with the `desktop` feature, because without it the other
//! executable already is this one.

fn main() -> anyhow::Result<()> {
    km_song_sync::run(km_song_sync::Shell::Console)
}
