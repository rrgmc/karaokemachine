//! Tests for the MP3+G packaging path.
//!
//! Pairing is what these mostly exercise, because pairing is the part a real corpus attacks: the
//! names in one commercial starter kit include extensions in both cases inside a single folder, and
//! one pair whose stems differ by a trailing space. The file contents are irrelevant here and are
//! not real — `add_cdg_song` itself is covered end to end by building a package from the corpus.

use super::*;

/// A folder with `names` in it, each file given a byte so it exists.
fn folder(tag: &str, names: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("km-pack-cdg-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    for name in names {
        std::fs::write(dir.join(name), b"x").expect("write");
    }
    dir
}

fn stems(pairs: &[CdgPair]) -> Vec<String> {
    pairs.iter().map(|pair| file_stem(&pair.audio)).collect()
}

#[test]
fn a_pair_is_found_once_and_from_the_audio_side() {
    let dir = folder("simple", &["a.mp3", "a.cdg", "b.mp3", "b.cdg"]);
    let (mut pairs, mut orphans) = (Vec::new(), Vec::new());
    collect_cdg(&dir, &mut pairs, &mut orphans);

    pairs.sort_by(|a, b| a.audio.cmp(&b.audio));
    assert_eq!(stems(&pairs), vec!["a", "b"], "each pair exactly once");
    assert!(orphans.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pairing_survives_the_extension_being_in_either_case() {
    // Measured in one real album folder: `.MP3` and `.mp3` side by side.
    let dir = folder("case", &["loud.MP3", "loud.cdg", "quiet.mp3", "quiet.CDG"]);
    let (mut pairs, mut orphans) = (Vec::new(), Vec::new());
    collect_cdg(&dir, &mut pairs, &mut orphans);

    pairs.sort_by(|a, b| a.audio.cmp(&b.audio));
    assert_eq!(stems(&pairs), vec!["loud", "quiet"]);
    assert!(orphans.is_empty(), "{orphans:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pairing_survives_a_stem_that_differs_only_by_a_trailing_space() {
    // One real pair in the measured corpus differs by exactly this and nothing else. Windows strips
    // trailing spaces from path components, so the file may only be openable by the name `read_dir`
    // reports — which is why the slow path returns that name rather than a rebuilt one.
    let dir = folder("space", &["hurricane.mp3", "hurricane .cdg"]);
    let (mut pairs, mut orphans) = (Vec::new(), Vec::new());
    collect_cdg(&dir, &mut pairs, &mut orphans);

    assert_eq!(pairs.len(), 1, "orphans: {orphans:?}");
    assert!(orphans.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_half_of_a_broken_pair_is_reported_rather_than_dropped() {
    // Eight files in the measured corpus are one half of a pair. A build that lost them without a
    // word would be a build nobody could reconcile against the folder they pointed it at.
    let dir = folder(
        "orphans",
        &["whole.mp3", "whole.cdg", "lonely.mp3", "silent.cdg"],
    );
    let (mut pairs, mut orphans) = (Vec::new(), Vec::new());
    collect_cdg(&dir, &mut pairs, &mut orphans);

    assert_eq!(stems(&pairs), vec!["whole"]);
    orphans.sort_by(|a, b| a.0.cmp(&b.0));
    let reported: Vec<(String, CdgOrphan)> = orphans
        .iter()
        .map(|(path, why)| (file_stem(path), *why))
        .collect();
    assert_eq!(
        reported,
        vec![
            ("lonely".to_owned(), CdgOrphan::NoGraphics),
            ("silent".to_owned(), CdgOrphan::NoAudio),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_mp3_plus_g_entry_scores_ten_and_carries_no_midi_facts() {
    let entry = entry_from_cdg(CdgFields {
        number: 4,
        title: "T".to_owned(),
        artist: None,
        language: None,
        tags: Vec::new(),
        file: "4.mp3".to_owned(),
        duration_ms: 231_967,
        loudness: None,
        content_hash: None,
    });
    assert_eq!(entry.kind, km_kmpkg::SongKind::Cdg);
    // `None` in, nothing out: the package's `default_language` is what answers, and
    // `settle_languages` is where that happens rather than here.
    assert_eq!(entry.language, None);
    // Full marks by what it is: a commercial karaoke disc was made to be sung to.
    assert_eq!(entry.suitability.map(|record| record.value), Some(10));
    assert_eq!(entry.melody, None);
    assert_eq!(entry.lyric_encoding, None);
    // CD+G carries no text at all, so nothing here can know what it is sung in.
    assert_eq!(entry.language, None);
}

/// **A commercial disc is made to be sung to, and a one-second track is not a track off one.** CD+G
/// words are one-bit tiles with no timing to read, so the pair is answered by its own length, which
/// is the only thing it says about how much of it there is.
#[test]
fn an_mp3_plus_g_pair_that_lasts_a_second_is_not_a_karaoke_disc() {
    let entry = entry_from_cdg(CdgFields {
        number: 4,
        title: "T".to_owned(),
        artist: None,
        language: None,
        tags: Vec::new(),
        file: "4.mp3".to_owned(),
        duration_ms: 1000,
        loudness: None,
        content_hash: None,
    });
    let record = entry.suitability.expect("a suitability");
    assert_eq!(record.value, 4);
    assert_eq!(record.breakdown.lyrics, 0);
    assert_eq!(record.breakdown.sync, 0);
    assert_eq!(
        record.warnings.first().map(|w| w.code.as_str()),
        Some(crate::warning_code(km_suitability::WarningCode::BriefSinging).as_str())
    );
}

#[test]
fn a_tag_is_dropped_when_it_is_a_placeholder_or_blank() {
    assert_eq!(usable_tag(Some("Queen")).as_deref(), Some("Queen"));
    assert_eq!(usable_tag(Some("  ")), None);
    assert_eq!(usable_tag(None), None);
    // Measured verbatim in the corpus, on a file whose title is really something else.
    assert_eq!(usable_tag(Some("Track  6")), None);
    assert_eq!(usable_tag(Some("track 12")), None);
    // Not a placeholder: a real song whose title happens to start with the word.
    assert_eq!(
        usable_tag(Some("Tracks Of My Tears")).as_deref(),
        Some("Tracks Of My Tears")
    );
}

#[test]
fn text_is_compared_ignoring_case_and_spacing() {
    // What stops a file name copied into the artist tag being packaged as the performer — which
    // several tracks in one real album do.
    assert!(same_text("QUEEN - BICYCLE RACE", "Queen - Bicycle Race"));
    assert!(same_text("a  b", "a b"));
    assert!(!same_text("Queen", "Queen - Bicycle Race"));
}
