//! What a package says about itself, as a page somebody can read.
//!
//! A `.kmpkg` is a zip with a JSON manifest in it, so answering *what is in this?* means having
//! `km-pack` and knowing to run it. The listing is the same facts as plain text beside the package,
//! for whoever is handed one: a header saying which package this is, and a line per song.
//!
//! **Generated and never read back.** The manifest is the record and this is a rendering of it, so
//! nothing parses one, a build overwrites whatever is there, and editing one changes nothing. That
//! is what separates it from the YAML description, which is the *editable* middle step and is what
//! a build reads. See `A package is a document too` in `docs/decisions/packaging.md`.
//!
//! **It says what the manifest says and nothing more.** No source paths, no output folder, no host,
//! no build clock: this file travels with the package to a stranger, so
//! `A package says nothing about the machine that built it` reaches it exactly as it reaches the
//! archive. Every value here is read off [`km_kmpkg::Manifest`], which is the half that has already
//! been held to that rule.

use std::fmt::Write as _;

use km_kmpkg::Manifest;

/// The package and its songs as plain text.
///
/// Ordered by song number, which is the order a singer meets them in and the order the book prints.
/// The columns are padded to line up, because the point of the file is being read down.
#[must_use]
pub fn listing(manifest: &Manifest) -> String {
    let package = &manifest.package;
    let mut out = String::new();

    let _ = writeln!(out, "{}", package.name);
    let _ = writeln!(out, "{}", "=".repeat(package.name.chars().count()));
    let _ = writeln!(out);
    let _ = writeln!(out, "Version:   {}", package.version);
    if let Some(publisher) = &package.publisher {
        let _ = writeln!(out, "Publisher: {publisher}");
    }
    if let Some(created) = &package.created {
        let _ = writeln!(out, "Created:   {created}");
    }
    let _ = writeln!(out, "Id:        {}", package.id);
    if let Some(volume) = &package.volume {
        let _ = writeln!(out, "Volume:    {} of {}", volume.number, volume.name);
    }
    let _ = writeln!(out, "Songs:     {}", manifest.songs.len());
    let _ = writeln!(out);

    let mut songs: Vec<&km_kmpkg::SongEntry> = manifest.songs.iter().collect();
    songs.sort_by_key(|song| song.number);

    // The number column is as wide as the widest number rather than a fixed five, so a package of
    // three-digit numbers does not read with a gutter down its left.
    let width = songs
        .iter()
        .map(|song| song.number.to_string().len())
        .max()
        .unwrap_or(1);

    for song in songs {
        let _ = write!(out, "{:>width$}  {}", song.number, song.title);
        if let Some(artist) = &song.artist {
            let _ = write!(out, " — {artist}");
        }
        let _ = write!(out, "  [{}", duration(song.duration_ms));
        if let Some(language) = &song.language {
            let _ = write!(out, " {language}");
        }
        // Named only where it is not a MIDI file, which is what almost every song is: a column
        // saying `midi` four thousand times says nothing.
        if !song.kind.is_midi() {
            let _ = write!(out, " {}", song.kind.as_str());
        }
        let _ = writeln!(out, "]");
    }
    out
}

/// Milliseconds as `m:ss`, or `h:mm:ss` past an hour.
///
/// Rounded down to the second: a length is read to recognize a song rather than to time one.
fn duration(ms: u32) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    match hours {
        0 => format!("{minutes}:{seconds:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use km_kmpkg::{PackageMeta, SongEntry, SongKind};

    fn manifest() -> Manifest {
        Manifest {
            format: 3,
            package: PackageMeta {
                id: "a1b2c3d4e5f60718".to_owned(),
                name: "Bossa nova".to_owned(),
                version: "1.2.0".to_owned(),
                publisher: Some("Someone".to_owned()),
                created: Some("2026-09-13T10:00:00Z".to_owned()),
                volume: None,
            },
            songs: vec![
                song(2, "Wave", Some("Tom Jobim"), Some("pt"), 195_400),
                song(1, "Corcovado", None, None, 3_723_000),
            ],
        }
    }

    fn song(
        number: u32,
        title: &str,
        artist: Option<&str>,
        language: Option<&str>,
        duration_ms: u32,
    ) -> SongEntry {
        SongEntry {
            number,
            kind: SongKind::Midi,
            title: title.to_owned(),
            artist: artist.map(ToOwned::to_owned),
            language: language.map(ToOwned::to_owned),
            file: format!("songs/{number:04}.kar"),
            duration_ms,
            lyric_encoding: None,
            default_transpose: 0,
            lyrics_hidden: false,
            fixes: Vec::new(),
            melody: None,
            melody_abstained: None,
            suitability: None,
            lyric_preview: Vec::new(),
            tags: Vec::new(),
            loudness: None,
            content_hash: None,
            edited: Vec::new(),
        }
    }

    #[test]
    fn it_reads_as_a_page_about_the_package() {
        let text = listing(&manifest());
        assert!(text.starts_with("Bossa nova\n==========\n"), "{text}");
        assert!(text.contains("Version:   1.2.0"), "{text}");
        assert!(text.contains("Publisher: Someone"), "{text}");
        assert!(text.contains("Songs:     2"), "{text}");

        // Numbered order, which is the order a singer meets them in, not the order they were
        // written into the archive.
        let first = text.find("Corcovado").expect("a song");
        let second = text.find("Wave").expect("a song");
        assert!(first < second, "{text}");

        assert!(text.contains("2  Wave — Tom Jobim  [3:15 pt]"), "{text}");
        // No artist and no language leave no empty column behind them, and an hour turns the
        // length into three parts.
        assert!(text.contains("1  Corcovado  [1:02:03]"), "{text}");
    }

    /// A volume says which set it belongs to, and a package that is not one prints no line for it.
    #[test]
    fn a_volume_names_its_set() {
        assert!(!listing(&manifest()).contains("Volume:"));
        let mut volume = manifest();
        volume.package.volume = Some(km_kmpkg::VolumeOf {
            of: "a1b2c3d4e5f60718".to_owned(),
            name: "Bossa nova".to_owned(),
            number: 2,
        });
        let text = listing(&volume);
        assert!(text.contains("Volume:    2 of Bossa nova"), "{text}");
    }

    /// The rule the archive is held to, pointed at the file beside it.
    ///
    /// Everything here comes off the manifest, which carries no path, no host and no build clock.
    /// A field added to this file that did not is the way that rule would be undone, so the shapes
    /// are asserted absent rather than trusted.
    #[test]
    fn it_says_nothing_about_the_machine_that_built_it() {
        let text = listing(&manifest());
        for leak in ["/", "\\", "songs/0001.kar", ".kmpkg"] {
            assert!(
                !text.contains(leak),
                "a listing naming {leak} says where it was built:\n{text}"
            );
        }
    }

    /// A package nothing went into still says which package it is.
    #[test]
    fn an_empty_package_is_a_header_and_no_rows() {
        let mut empty = manifest();
        empty.songs.clear();
        let text = listing(&empty);
        assert!(text.contains("Songs:     0"), "{text}");
        assert!(text.trim_end().ends_with("Songs:     0"), "{text}");
    }
}
