//! Writes the synthetic karaoke fixtures to disk for manual inspection.
//!
//! ```text
//! cargo run -p km-song --features testing --example write_fixtures -- fixtures/generated
//! ```
//!
//! Useful for eyeballing the display and the sequencer before the real corpus is packaged, and for
//! feeding `km-lyrics dump` something known-good.

use std::path::PathBuf;

use km_song::testing::{FIXTURES, UNREADABLE_FIXTURES};

fn main() -> std::io::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("fixtures/generated"), PathBuf::from);
    std::fs::create_dir_all(&dir)?;

    // Both lists, because the ones that must be *refused* are as much use in front of a person as
    // the ones that play -- pointing a tool at one is how you see what it says about a bad file.
    for (name, build) in FIXTURES.iter().chain(UNREADABLE_FIXTURES) {
        let path = dir.join(name);
        std::fs::write(&path, build())?;
        println!("wrote {}", path.display());
    }
    println!(
        "{} playable and {} unreadable fixtures written to {}",
        FIXTURES.len(),
        UNREADABLE_FIXTURES.len(),
        dir.display()
    );
    Ok(())
}
