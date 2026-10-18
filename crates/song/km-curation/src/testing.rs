//! What the tests need and the tool does not.
//!
//! **A test that touches a disk takes a [`Scratch`], never a fixed name in the temp folder.**
//! Several checkouts run this suite at once, and cargo runs its tests in parallel. A shared name
//! lets one run delete the database another run is reading. [`Scratch`] names its folder per
//! process and per call, and removes it when it drops.
//!
//! **The songs below are the fixture `db`'s tests write, and `km-package-builder`'s server tests
//! write the same ones.** That crate reaches them through the `testing` feature, which is off in
//! every build that ships.

use std::path::Path;

use crate::db::Db;

pub use km_testkit::Scratch;

/// Verses of a song, one line each, well clear of the floor a lyric has to reach.
///
/// Every line distinct, so a test can take one away and say which one.
pub fn verses(from: usize, to: usize) -> String {
    (from..to)
        .map(|nth| {
            format!(
                "she walked in from the rain of number {nth}\n\
                 and nobody ever knew her name at all {nth}\n\
                 while the band played on until the morning {nth}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Writes one scanned song with the given title at the given path.
pub fn add(db: &mut Db, id: &str, title: Option<&str>, path: &str) {
    add_built(db, id, title, path, |_| {});
}

/// [`add`], with the artist the file declared, for a test that needs a row already credited.
pub fn add_with_artist(db: &mut Db, id: &str, title: &str, artist: &str, path: &str) {
    add_built(db, id, Some(title), path, |song| {
        song.det_artist = Some(artist.to_owned());
    });
}

/// [`add`], with a chance to adjust the scanned song before it is written.
pub fn add_scanned(db: &mut Db, id: &str, adjust: impl FnOnce(&mut crate::model::ScannedSong)) {
    add_built(db, id, Some(id), &format!("folder/{id}.kar"), adjust);
}

/// [`add`], with the title and the path given and a chance to adjust the song before it is written.
pub fn add_built(
    db: &mut Db,
    id: &str,
    title: Option<&str>,
    path: &str,
    adjust: impl FnOnce(&mut crate::model::ScannedSong),
) {
    add_all(db, vec![scanned_file(id, title, path, adjust)]);
}

/// Writes many songs in one transaction.
///
/// A fixture of hundreds of songs is what a query plan is measured against. Written one at a time,
/// each song is a commit, and the commits cost seconds.
pub fn add_all(db: &mut Db, files: Vec<crate::model::ScannedFile>) {
    db.write_scanned(&files, "2026-08-24T00:00:00Z")
        .expect("write");
}

/// The file [`add`] writes, built and not yet written, for [`add_all`].
pub fn scanned_file(
    id: &str,
    title: Option<&str>,
    path: &str,
    adjust: impl FnOnce(&mut crate::model::ScannedSong),
) -> crate::model::ScannedFile {
    let mut song = crate::model::ScannedSong {
        id: id.to_owned(),
        det_title: title.map(ToOwned::to_owned),
        det_artist: None,
        det_language: None,
        stem: Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_owned(),
        duration_ms: 200_000,
        // Enough words for the lyric search to have something to find, and different for every
        // song so a test can tell which one came back.
        lyrics: Some(format!("the words of {id}\nsecond line")),
        // A shape shared by every song this makes, so a test wanting two of them paired has
        // only to give them matching names, which is the other half `compare` insists on.
        fingerprint: "12:100:0,1".to_owned(),
        suitability: crate::model::SuitabilityFacts {
            value: 7,
            breakdown: (3, 2, 2, 0),
            warnings: "[]".to_owned(),
        },
        midi: Some(crate::model::MidiFacts {
            flavor: "soft".to_owned(),
            granularity: "syllablelevel".to_owned(),
            note_count: 500,
            channel_count: 6,
            line_count: 20,
            syllable_count: 100,
            det_encoding: "windows-1252".to_owned(),
            det_encoding_source: "fallback".to_owned(),
            melody_channel: Some(3),
            melody_confidence: Some(0.9),
            melody_abstained: None,
        }),
        video: None,
        cdg: None,
        ultrastar: None,
        lrc: None,
    };
    adjust(&mut song);
    crate::model::ScannedFile {
        path: path.to_owned(),
        size: 1234,
        mtime: 0,
        content_hash: Some(id.to_owned()),
        status: crate::model::ScanStatus::Ok,
        error: None,
        song: Some(song),
    }
}
