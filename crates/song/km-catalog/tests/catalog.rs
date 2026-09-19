//! The catalog against a real SQLite database.
//!
//! The unit tests in the crate check the SQL that gets built; these run it. That distinction matters
//! most for FTS5, which is a compile-time option in SQLite — a query that looks right is no evidence
//! the index exists.

use std::path::PathBuf;

use km_catalog::{Library, LibraryError, SearchQuery, SongCode, SortOrder};
use km_kmpkg::{
    BreakdownRecord, MelodyRecord, Package, PackageBuilder, PackageMeta, SongEntry,
    SuitabilityRecord,
};

const NOW: &str = "2026-08-23T10:00:00Z";

/// Three package ids of the generated shape, which is the only shape a manifest may carry.
///
/// Spelled out rather than generated per run so that a failure names the same package twice.
const VOL1: &str = "1f4a9c8e2b7d0356";
const VOL2: &str = "a1b2c3d4e5f60789";
const VOL3: &str = "0987f6e5d4c3b2a1";

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("km-catalog-tests-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn meta(id: &str) -> PackageMeta {
    PackageMeta {
        id: id.to_owned(),
        name: format!("Package {id}"),
        version: "1.0.0".to_owned(),
        publisher: None,
        created: None,
        volume: None,
    }
}

fn song(number: u32, title: &str, artist: Option<&str>) -> SongEntry {
    SongEntry {
        number,
        kind: km_kmpkg::SongKind::Midi,
        title: title.to_owned(),
        artist: artist.map(ToOwned::to_owned),
        language: Some("por".to_owned()),
        file: String::new(),
        duration_ms: 200_000,
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

fn rated(mut entry: SongEntry, value: u8, melody: Option<u8>) -> SongEntry {
    entry.suitability = Some(SuitabilityRecord {
        value,
        breakdown: BreakdownRecord {
            lyrics: 3,
            sync: 3,
            channels: 2,
            arrangement: 2,
        },
        warnings: Vec::new(),
    });
    entry.melody = melody.map(|channel| MelodyRecord {
        channel,
        confidence: 0.9,
        signals: vec!["track_name".to_owned()],
    });
    entry
}

/// Builds a package on disk with the given songs, each with distinct content.
fn build_package(dir: &std::path::Path, id: &str, songs: Vec<SongEntry>) -> Package {
    let path = dir.join(format!("{id}.kmpkg"));
    let mut builder = PackageBuilder::new(meta(id));
    for entry in songs {
        let bytes = format!("midi bytes for {}", entry.number).into_bytes();
        builder.add(entry, bytes).expect("add");
    }
    builder.write(&path).expect("write");
    Package::open(&path).expect("open")
}

#[test]
fn a_package_installs_and_its_songs_are_found_by_number() {
    let dir = temp_dir("install");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "Exagerado", Some("Cazuza")),
            song(2, "Wave", Some("Tom Jobim")),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    let report = library.install(&package, 10, NOW).expect("install");
    assert_eq!(report.songs_added, 2);
    assert!(!report.replaced_existing);
    // The name travels with the report because the sentence a person reads quotes it, and an id is
    // sixteen hexadecimal characters that say nothing about which package arrived.
    assert_eq!(report.package_name, format!("Package {VOL1}"));
    assert_eq!(library.song_count().expect("count"), 2);

    let found = library
        .song(SongCode::new(10_001))
        .expect("query")
        .expect("song 10001");
    assert_eq!(found.title, "Exagerado");
    assert_eq!(found.artist.as_deref(), Some("Cazuza"));
    assert_eq!(found.package_id, VOL1);

    assert!(
        library
            .song(SongCode::new(99_999))
            .expect("query")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn full_text_search_works_which_proves_fts5_is_compiled_in() {
    let dir = temp_dir("fts");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "Exagerado", Some("Cazuza")),
            song(2, "Garota de Ipanema", Some("Tom Jobim")),
            song(3, "Yesterday", Some("The Beatles")),
        ],
    );
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let results = library
        .search(&SearchQuery::text("ipanema"))
        .expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].number, SongCode::new(1_002));

    // Matching on the artist column too.
    let results = library
        .search(&SearchQuery::text("beatles"))
        .expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].number, SongCode::new(1_003));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_word_finds_the_song() {
    let dir = temp_dir("prefix");
    let package = build_package(&dir, VOL1, vec![song(1, "Yesterday", Some("The Beatles"))]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    // Somebody typing part of a word expects to find it.
    let results = library
        .search(&SearchQuery::text("yester"))
        .expect("search");
    assert_eq!(results.len(), 1, "a prefix should match");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_ignores_accents_which_matters_for_this_corpus() {
    let dir = temp_dir("accents");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "Coração", Some("Alguém")),
            song(2, "Canção do Amor", Some("Outro")),
        ],
    );
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    // Nobody types the cedilla and the tilde on a remote.
    let results = library
        .search(&SearchQuery::text("coracao"))
        .expect("search");
    assert_eq!(
        results.len(),
        1,
        "an unaccented search must find the accented title"
    );
    assert_eq!(results[0].number, SongCode::new(1_001));

    let results = library
        .search(&SearchQuery::text("cancao"))
        .expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].number, SongCode::new(1_002));

    // And the other direction works too.
    let results = library
        .search(&SearchQuery::text("coração"))
        .expect("search");
    assert_eq!(results.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_text_that_is_all_operators_does_not_error() {
    let dir = temp_dir("operators");
    let package = build_package(&dir, VOL1, vec![song(1, "AC/DC Live", Some("AC/DC"))]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    // Each of these is FTS5 syntax that would be a query error if passed through raw.
    for query in [
        "AND", "OR", "NOT", "NEAR", "*", "^", "\"", "-", "a:b", "!!!",
    ] {
        let results = library.search(&SearchQuery::text(query));
        assert!(results.is_ok(), "{query:?} should not error: {results:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_packages_that_number_from_the_same_slot_do_not_collide() {
    // The requirement, stated as a test, and it is the one this whole arrangement exists for: two
    // packages both number a song 500, and both live in one catalog with nobody renumbering
    // anything, because the machine put them in different thousands.
    let dir = temp_dir("banks");
    let first = build_package(&dir, VOL1, vec![song(500, "First Claim", Some("A"))]);
    let second = build_package(&dir, VOL2, vec![song(500, "Second Claim", Some("B"))]);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&first, 1, NOW).expect("install");
    library
        .install(&second, 2, NOW)
        .expect("a second package fits beside the first");

    assert_eq!(library.song_count().expect("count"), 2);
    assert_eq!(
        library
            .song(SongCode::new(1_500))
            .expect("query")
            .expect("1500")
            .title,
        "First Claim"
    );
    assert_eq!(
        library
            .song(SongCode::new(2_500))
            .expect("query")
            .expect("2500")
            .title,
        "Second Claim"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_packages_cannot_take_the_same_bank() {
    let dir = temp_dir("bank-taken");
    let first = build_package(&dir, VOL1, vec![song(1, "One", Some("A"))]);
    let second = build_package(&dir, VOL2, vec![song(2, "Two", Some("B"))]);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&first, 3, NOW).expect("install");

    // Note these two do *not* share a slot. The bank is the only thing that can clash now, which is
    // why there is one fault to report and one remedy rather than a list of numbers.
    match library.install(&second, 3, NOW) {
        Err(LibraryError::BankTaken { bank, owner }) => {
            assert_eq!(bank, 3);
            assert_eq!(owner, VOL1);
        }
        other => panic!("expected the bank to be refused, got {other:?}"),
    }
    assert_eq!(library.song_count().expect("count"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bank_above_the_last_one_is_refused() {
    let dir = temp_dir("bad-bank");
    let package = build_package(&dir, VOL1, vec![song(1, "One", Some("A"))]);
    let mut library = Library::open_in_memory().expect("open");

    match library.install(&package, km_songcode::MAX_BANK + 1, NOW) {
        Err(LibraryError::BadBank { bank }) => assert_eq!(bank, km_songcode::MAX_BANK + 1),
        other => panic!("expected the bank to be refused, got {other:?}"),
    }
    assert_eq!(library.song_count().expect("count"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bank 0 is the machine's own, and the catalog is where that is an invariant rather than a check.
///
/// Both roads in, because they are two functions: nothing may be installed there, and nothing
/// already installed may be moved there.
#[test]
fn bank_zero_holds_no_package() {
    let dir = temp_dir("reserved-bank");
    let package = build_package(&dir, VOL1, vec![song(1, "One", Some("A"))]);
    let mut library = Library::open_in_memory().expect("open");

    assert!(matches!(
        library.install(&package, 0, NOW),
        Err(LibraryError::BankReserved)
    ));
    assert_eq!(library.song_count().expect("count"), 0);

    library.install(&package, 5, NOW).expect("install");
    assert!(matches!(
        library.set_package_bank(VOL1, 0),
        Err(LibraryError::BankReserved)
    ));
    assert_eq!(library.bank_of(VOL1).expect("bank"), Some(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn moving_a_package_to_another_bank_re_keys_every_song_in_it() {
    let dir = temp_dir("re-bank");
    let package = build_package(
        &dir,
        VOL1,
        vec![song(1, "One", Some("A")), song(2, "Two", Some("A"))],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");
    let before = library.catalog_version().expect("version");

    assert_eq!(library.set_package_bank(VOL1, 4).expect("re-bank"), 2);
    assert!(library.song(SongCode::new(1_001)).expect("query").is_none());
    assert_eq!(
        library
            .song(SongCode::new(4_001))
            .expect("query")
            .expect("4001")
            .title,
        "One"
    );
    // The slot survives the move: only the thousand it sits in changed.
    assert_eq!(
        SongCode::new(4_001).slot(),
        1,
        "a song keeps the number its package gave it"
    );
    assert_eq!(library.banks().expect("banks"), vec![4]);
    assert_eq!(library.bank_of(VOL1).expect("bank"), Some(4));
    // A mirror has to be told: every code in the package just changed.
    assert!(library.catalog_version().expect("version") > before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reinstalling_the_same_package_replaces_it_rather_than_colliding() {
    let dir = temp_dir("upgrade");
    let v1 = build_package(&dir, VOL1, vec![song(1, "Old Title", Some("A"))]);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&v1, 1, NOW).expect("install");

    // Same package id, changed contents: an upgrade, not a conflict.
    let path = dir.join("vol1-v2.kmpkg");
    let mut builder = PackageBuilder::new(PackageMeta {
        version: "2.0.0".to_owned(),
        ..meta(VOL1)
    });
    builder
        .add(song(1, "New Title", Some("A")), b"different bytes".to_vec())
        .expect("add");
    builder
        .add(song(2, "Added Song", Some("A")), b"more bytes".to_vec())
        .expect("add");
    builder.write(&path).expect("write");
    let v2 = Package::open(&path).expect("open");

    let report = library
        .install(&v2, 1, NOW)
        .expect("upgrade should succeed");
    assert!(report.replaced_existing);
    assert_eq!(library.song_count().expect("count"), 2);
    assert_eq!(
        library
            .song(SongCode::new(1_001))
            .expect("query")
            .expect("song")
            .title,
        "New Title"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uninstalling_removes_the_songs_and_the_search_index_with_them() {
    let dir = temp_dir("uninstall");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "Findable", Some("A")),
            song(2, "Also Findable", Some("B")),
        ],
    );
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");
    assert_eq!(
        library
            .search(&SearchQuery::text("findable"))
            .expect("search")
            .len(),
        2
    );

    let removed = library.uninstall(VOL1).expect("uninstall");
    assert_eq!(removed, 2);
    assert_eq!(library.song_count().expect("count"), 0);
    // A stale index would still return hits here.
    assert!(
        library
            .search(&SearchQuery::text("findable"))
            .expect("search")
            .is_empty()
    );
    assert!(library.packages().expect("packages").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_suitability_filter_and_ordering_work() {
    let dir = temp_dir("suitability");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            rated(song(1, "Rough One", Some("A")), 3, None),
            rated(song(2, "Decent One", Some("B")), 7, Some(4)),
            rated(song(3, "Great One", Some("C")), 10, Some(2)),
            song(4, "Unrated", Some("D")),
        ],
    );
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let good = library
        .search(&SearchQuery {
            min_suitability: Some(7),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(good.len(), 2, "only the 7 and the 10");

    let by_suitability = library
        .search(&SearchQuery {
            sort: SortOrder::Suitability,
            ..Default::default()
        })
        .expect("search");
    assert_eq!(
        by_suitability[0].number,
        SongCode::new(1_003),
        "the best first"
    );
    assert_eq!(
        by_suitability.last().expect("last").number,
        SongCode::new(1_004),
        "the unrated song must not outrank rated ones"
    );

    let with_melody = library
        .search(&SearchQuery {
            melody_only: true,
            ..Default::default()
        })
        .expect("search");
    assert_eq!(with_melody.len(), 2);
    assert!(with_melody.iter().all(|s| s.melody_channel.is_some()));

    let _ = std::fs::remove_dir_all(&dir);
}

/// What the machine's demo mode draws on: one arbitrary song, and a different one next time.
#[test]
fn the_random_order_shuffles_and_still_honors_the_filter() {
    let dir = temp_dir("random");
    // Twenty songs, so "every draw returned the same one" is a one-in-a-huge-number coincidence
    // rather than something four songs could produce by luck.
    let songs: Vec<_> = (1..=20)
        .map(|number| {
            let entry = song(number, &format!("Song {number}"), Some("A"));
            // Half of them rate 8 and half rate 2, so the filter has something to cut.
            rated(entry, if number % 2 == 0 { 8 } else { 2 }, None)
        })
        .collect();
    let package = build_package(&dir, VOL1, songs);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let draw = || {
        library
            .search(&SearchQuery {
                sort: SortOrder::Random,
                min_suitability: Some(5),
                limit: 1,
                ..Default::default()
            })
            .expect("search")
            .first()
            .map(|song| song.number)
            .expect("a song")
    };

    // The filter still applies: every even-numbered song rates 8, every odd one 2.
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..40 {
        let number = draw();
        assert_eq!(
            number.number() % 2,
            0,
            "the suitability floor must still hold under a random order, got {number}"
        );
        seen.insert(number);
    }
    // Forty draws from ten candidates landing on one song would mean `RANDOM()` is not being
    // applied at all -- which is exactly what a stale `ORDER BY s.number` would look like.
    assert!(
        seen.len() > 1,
        "forty draws returned only {seen:?}; the order is not random"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_artist_filter_matches_a_substring_case_insensitively() {
    let dir = temp_dir("artist");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "A", Some("The Rolling Stones")),
            song(2, "B", Some("Rolling Blackouts")),
            song(3, "C", Some("Someone Else")),
        ],
    );
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let results = library
        .search(&SearchQuery {
            artist: Some("rolling".to_owned()),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(results.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The language filter matches exactly, where the artist filter above matches a substring.
///
/// The asymmetry is deliberate and worth a test rather than a comment: an artist is half-remembered
/// text somebody typed, and a language is a code out of a closed table. `zh` matching `zh-Hant` would
/// leave no way to ask for exactly `zh`, and would be the only route to a wrong answer here.
#[test]
fn the_language_filter_matches_a_whole_code_and_nothing_else() {
    let dir = temp_dir("language");
    let mut brazilian = song(1, "Corcovado", Some("Tom Jobim"));
    brazilian.language = Some("pt".to_owned());
    let mut japanese = song(2, "Sakura", None);
    japanese.language = Some("ja".to_owned());
    let mut unknown = song(3, "Mystery", None);
    unknown.language = None;

    let package = build_package(&dir, VOL1, vec![brazilian, japanese, unknown]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let matching = |language: &str| {
        library
            .search(&SearchQuery {
                language: Some(language.to_owned()),
                ..Default::default()
            })
            .expect("search")
            .into_iter()
            .map(|song| song.number)
            .collect::<Vec<_>>()
    };

    assert_eq!(matching("pt"), vec![SongCode::new(1_001)]);
    assert_eq!(matching("ja"), vec![SongCode::new(1_002)]);
    // A remote that shouts still finds them: every code a packager writes is lowercase.
    assert_eq!(matching("JA"), vec![SongCode::new(1_002)]);
    // Not a prefix match, and no partial code matches anything.
    assert!(matching("p").is_empty());
    assert!(matching("por").is_empty());

    // An empty parameter is no filter at all, rather than a search for songs with no language.
    let unfiltered = library
        .search(&SearchQuery {
            language: Some("   ".to_owned()),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(unfiltered.len(), 3, "including the one with no language");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn paging_returns_distinct_pages_that_cover_everything() {
    let dir = temp_dir("paging");
    let songs: Vec<SongEntry> = (1..=25)
        .map(|n| song(n, &format!("Song {n:02}"), Some("A")))
        .collect();
    let package = build_package(&dir, VOL1, songs);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let page = |offset: usize| SearchQuery {
        sort: SortOrder::Number,
        limit: 10,
        offset,
        ..Default::default()
    };
    let first = library.search(&page(0)).expect("search");
    let second = library.search(&page(10)).expect("search");
    let third = library.search(&page(20)).expect("search");

    assert_eq!(first.len(), 10);
    assert_eq!(second.len(), 10);
    assert_eq!(third.len(), 5);

    let mut all: Vec<SongCode> = first
        .iter()
        .chain(second.iter())
        .chain(third.iter())
        .map(|s| s.number)
        .collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 25, "pages must not overlap or skip");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_content_across_packages_is_reported_but_not_refused() {
    let dir = temp_dir("duplicates");

    // Two packages containing the same recording under different numbers -- exactly what building a
    // catalog from a corpus full of duplicates produces.
    let first_path = dir.join("vol1.kmpkg");
    let mut builder = PackageBuilder::new(meta(VOL1));
    builder
        .add(song(1, "Same Song", Some("A")), b"identical midi".to_vec())
        .expect("add");
    builder.write(&first_path).expect("write");
    let first = Package::open(&first_path).expect("open");

    let second_path = dir.join("vol2.kmpkg");
    let mut builder = PackageBuilder::new(meta(VOL2));
    builder
        .add(
            song(2, "Same Song Again", Some("A")),
            b"identical midi".to_vec(),
        )
        .expect("add");
    builder.write(&second_path).expect("write");
    let second = Package::open(&second_path).expect("open");

    let mut library = Library::open_in_memory().expect("open");
    library.install(&first, 1, NOW).expect("install");
    let report = library
        .install(&second, 2, NOW)
        .expect("install should succeed");

    // The same recording in two packages, which banking does nothing about and should not: the two
    // songs have different numbers by construction now, and being the same recording is still worth
    // saying out loud.
    assert_eq!(report.duplicate_content.len(), 1);
    assert_eq!(report.duplicate_content[0].number, SongCode::new(2_002));
    assert_eq!(
        report.duplicate_content[0].existing_number,
        SongCode::new(1_001)
    );
    assert_eq!(report.duplicate_content[0].existing_package, VOL1);

    // Both are installed: this is a smell to surface, not an error to block on.
    assert_eq!(library.song_count().expect("count"), 2);

    let groups = library.duplicate_content().expect("scan");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0], vec![SongCode::new(1_001), SongCode::new(2_002)]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_catalog_persists_across_reopening() {
    let dir = temp_dir("persist");
    let db = dir.join("library.sqlite");
    let package = build_package(&dir, VOL1, vec![song(42, "Persistent", Some("A"))]);

    {
        let mut library = Library::open(&db).expect("open");
        library.install(&package, 1, NOW).expect("install");
    }
    let library = Library::open(&db).expect("reopen");
    assert_eq!(library.song_count().expect("count"), 1);
    assert_eq!(
        library
            .song(SongCode::new(1_042))
            .expect("query")
            .expect("song")
            .title,
        "Persistent"
    );
    // The path is recorded so the MIDI can be read back later.
    let path = library
        .package_path_for(SongCode::new(1_042))
        .expect("query")
        .expect("path");
    assert!(path.ends_with(&format!("{VOL1}.kmpkg")), "got {path}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_recorded_path_leads_back_to_playable_midi() {
    let dir = temp_dir("readback");
    let package = build_package(&dir, VOL1, vec![song(7, "Readable", Some("A"))]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    // The whole point of storing the path: getting the bytes back out.
    let path = library
        .package_path_for(SongCode::new(1_007))
        .expect("query")
        .expect("path");
    let reopened = Package::open(&path).expect("reopen from the catalog path");
    let midi = reopened.read_song(7).expect("read");
    assert_eq!(midi, b"midi bytes for 7");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_songs_first_lines_travel_from_the_package_into_the_catalog() {
    let dir = temp_dir("preview");
    let mut with_words = song(1, "Tempo Perdido", Some("Legião Urbana"));
    with_words.lyric_preview = vec![
        "Tempo perdido".to_owned(),
        // An accent and a comma, so the storage format is exercised rather than assumed.
        "E que tudo mais vá, pro inferno".to_owned(),
    ];
    let package = build_package(
        &dir,
        VOL1,
        vec![with_words, song(2, "Wave", Some("Tom Jobim"))],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 10, NOW).expect("install");

    let found = library
        .song(SongCode::new(10_001))
        .expect("query")
        .expect("song 10001");
    assert_eq!(
        found.lyric_preview,
        vec![
            "Tempo perdido".to_owned(),
            "E que tudo mais vá, pro inferno".to_owned()
        ]
    );

    // A song whose package carries none reads back as none rather than as one empty line, which is
    // what a naive `split('\n')` over a NULL-turned-empty-string would give.
    let plain = library
        .song(SongCode::new(10_002))
        .expect("query")
        .expect("song 10002");
    assert!(plain.lyric_preview.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn installed_packages_are_listed_with_their_details() {
    let dir = temp_dir("listing");
    let package = build_package(&dir, VOL1, vec![song(1, "A", None), song(2, "B", None)]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let packages = library.packages().expect("packages");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].id, VOL1);
    assert_eq!(packages[0].song_count, 2);
    assert_eq!(packages[0].installed_at, NOW);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A catalog keyed by number alone is thrown away rather than converted.
///
/// Safe only because this file is a **derived index**: every installed package is named in
/// one of the folders the machine scans, or named in `debug.packages`, and it reinstalls from them at
/// every start. The test states the invariant so that storing anything here that is not derivable
/// from a package fails loudly rather than quietly losing it.
#[test]
fn a_catalog_from_before_song_codes_is_dropped_and_rebuilt() {
    let dir = temp_dir("migrate-codes");
    let path = dir.join("library.sqlite");

    {
        // The old shape written out, because it cannot be reached by winding the current one back:
        // `prefix` is half of `UNIQUE (prefix, number)`, and SQLite will not drop a column a
        // constraint names.
        let conn = rusqlite::Connection::open(&path).expect("create catalog");
        conn.execute_batch(
            r#"CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
               INSERT INTO meta (key, value) VALUES ('catalog_version', '4');
               CREATE TABLE packages (
                   id TEXT PRIMARY KEY, name TEXT NOT NULL, version TEXT NOT NULL,
                   path TEXT NOT NULL, song_count INTEGER NOT NULL, installed_at TEXT NOT NULL);
               CREATE TABLE songs (
                   number INTEGER PRIMARY KEY,
                   package_id TEXT NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
                   title TEXT NOT NULL, artist TEXT, language TEXT,
                   kind TEXT NOT NULL DEFAULT 'midi', file TEXT NOT NULL,
                   duration_ms INTEGER NOT NULL, lyric_encoding TEXT,
                   default_transpose INTEGER NOT NULL DEFAULT 0, melody_channel INTEGER,
                   suitability INTEGER, content_hash TEXT);
               CREATE VIRTUAL TABLE songs_fts USING fts5(
                   title, artist, content='songs', content_rowid='number',
                   tokenize='unicode61 remove_diacritics 2');
               CREATE TRIGGER songs_fts_insert AFTER INSERT ON songs BEGIN
                   INSERT INTO songs_fts(rowid, title, artist)
                       VALUES (new.number, new.title, new.artist);
               END;
               INSERT INTO packages (id, name, version, path, song_count, installed_at)
                 VALUES ('old', 'Old', '1.0.0', '/tmp/old.kmpkg', 1, '2026-01-01');
               INSERT INTO songs (number, package_id, title, file, duration_ms)
                 VALUES (42, 'old', 'Something Already Installed', 'midi/42.mid', 1000);"#,
        )
        .expect("a catalog from before song codes");
    }

    let library = Library::open(&path).expect("an older catalog still opens");
    assert_eq!(
        library.song_count().expect("count"),
        0,
        "the old rows went, and the next start reinstalls them from the packages themselves"
    );
    assert!(library.packages().expect("packages").is_empty());
    // And the rebuilt catalog is usable rather than merely empty.
    assert!(library.song(SongCode::new(42)).expect("query").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The twin of the test above, for a catalog with `songs.prefix`.
///
/// **`km-remote-core`'s mirror has the same test and a rule of its own**, because the two schemas
/// are in two crates and no code is shared between them; changing one without the other leaves the
/// offline remote answering every browse with `no such column`.
#[test]
fn a_catalog_from_the_prefix_era_is_dropped_and_rebuilt() {
    let dir = temp_dir("migrate-banks");
    let path = dir.join("library.sqlite");

    {
        // The prefix-era schema, written out for the same reason as above: `prefix` is half of
        // `UNIQUE (prefix, number)`, and SQLite will not drop a column a constraint names, so the
        // current schema cannot be wound back into this one.
        let conn = rusqlite::Connection::open(&path).expect("create catalog");
        conn.execute_batch(
            r#"CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
               INSERT INTO meta (key, value) VALUES ('catalog_version', '9');
               CREATE TABLE packages (
                   id TEXT PRIMARY KEY, name TEXT NOT NULL, version TEXT NOT NULL,
                   path TEXT NOT NULL, song_count INTEGER NOT NULL, installed_at TEXT NOT NULL,
                   prefix TEXT NOT NULL DEFAULT '');
               CREATE TABLE songs (
                   id INTEGER PRIMARY KEY,
                   prefix TEXT NOT NULL DEFAULT '',
                   number INTEGER NOT NULL,
                   package_id TEXT NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
                   title TEXT NOT NULL, artist TEXT, language TEXT,
                   kind TEXT NOT NULL DEFAULT 'midi', file TEXT NOT NULL,
                   duration_ms INTEGER NOT NULL, lyric_encoding TEXT,
                   default_transpose INTEGER NOT NULL DEFAULT 0, melody_channel INTEGER,
                   suitability INTEGER, content_hash TEXT, lyric_preview TEXT,
                   UNIQUE (prefix, number));
               CREATE VIRTUAL TABLE songs_fts USING fts5(
                   title, artist, content='songs', content_rowid='id',
                   tokenize='unicode61 remove_diacritics 2');
               CREATE TRIGGER songs_fts_insert AFTER INSERT ON songs BEGIN
                   INSERT INTO songs_fts(rowid, title, artist)
                       VALUES (new.id, new.title, new.artist);
               END;
               INSERT INTO packages (id, name, version, path, song_count, installed_at, prefix)
                 VALUES ('old', 'Old', '1.0.0', '/tmp/old.kmpkg', 1, '2026-01-01', 'BR');
               INSERT INTO songs (prefix, number, package_id, title, file, duration_ms)
                 VALUES ('BR', 500, 'old', 'Dialled As BR500', 'midi/500.mid', 1000);"#,
        )
        .expect("a catalog from the prefix era");
    }

    let library = Library::open(&path).expect("a prefixed catalog still opens");
    assert_eq!(
        library.song_count().expect("count"),
        0,
        "the old rows went, and the next start reinstalls them from the packages themselves"
    );
    assert!(library.packages().expect("packages").is_empty());
    assert!(library.banks().expect("banks").is_empty());
    // Usable rather than merely empty: the rebuilt schema takes an install and answers by number.
    let package = build_package(&dir, VOL1, vec![song(500, "Dialled As 3500", Some("A"))]);
    let mut library = library;
    library.install(&package, 3, NOW).expect("install");
    assert_eq!(
        library
            .song(SongCode::new(3_500))
            .expect("query")
            .expect("3500")
            .title,
        "Dialled As 3500"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A measurement in a manifest reaches the catalog, and the peak beside it deliberately does not.
///
/// A real video song rather than a MIDI one carrying a level it could not have: a MIDI song is the
/// reference and never has a measurement, so testing this with one would be testing a shape no build
/// produces. The media is a stub because the catalog never opens it — installing reads the manifest.
#[test]
fn a_measured_song_carries_its_loudness_into_the_catalog() {
    let dir = temp_dir("install-loudness");
    let source = dir.join("stub.mp4");
    std::fs::write(&source, b"not really a video, and nothing here opens it").expect("stub");

    let path = dir.join("levelled.kmpkg");
    {
        let mut builder = PackageBuilder::new(meta(VOL3));
        let mut entry = song(7, "A Loud Karaoke Video", Some("Somebody"));
        entry.kind = km_kmpkg::SongKind::Video;
        entry.melody = None;
        entry.loudness = Some(km_kmpkg::LoudnessRecord {
            lufs: -5.5,
            peak_dbtp: 2.5,
        });
        builder
            .add_video_source(entry, "media/7.mp4", &source, None)
            .expect("add a video");
        builder.write(&path).expect("write");
    }
    let package = Package::open(&path).expect("open");

    let mut library = Library::open(dir.join("library.sqlite")).expect("catalog");
    library.install(&package, 1, "2026-01-01").expect("install");

    let installed = library
        .song(SongCode::in_bank(1, 7).expect("code"))
        .expect("query")
        .expect("the song is there");
    let lufs = installed
        .loudness_lufs
        .expect("the measurement came across");
    assert!((lufs - (-5.5)).abs() < 1e-4, "got {lufs}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The twin of `km_remote_core`'s `songs_sort_by_a_folded_key_rather_than_by_the_raw_title`,
/// deliberately in the same words so that one grep finds the pair. Until the sort key existed the
/// machine and the online remote both sorted with `COLLATE NOCASE`, which is ASCII-only — so every
/// accented title came after `Z`, and `É o amor` was last in a catalog of eleven thousand songs.
#[test]
fn songs_sort_by_a_folded_key_rather_than_by_the_raw_title() {
    let dir = temp_dir("sort-title");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "Zebra", None),
            song(2, "É o amor", None),
            song(3, "Banana", None),
            song(4, "Águas de Março", None),
            song(5, "apple", None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let titles: Vec<String> = library
        .search(&SearchQuery {
            sort: SortOrder::Title,
            ..SearchQuery::default()
        })
        .expect("search")
        .into_iter()
        .map(|s| s.title)
        .collect();
    assert_eq!(
        titles,
        ["Águas de Março", "apple", "Banana", "É o amor", "Zebra"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two songs sharing a title are two recordings, so the performer decides between them.
///
/// A song number is not an order anybody can read, and a catalog carries several *Goodbye*s. The
/// unnamed one goes to the end of the title rather than the front of it, which is the half a plain
/// `sort_artist` term gets wrong — an empty fold sorts first.
#[test]
fn songs_sharing_a_title_sort_by_performer() {
    let dir = temp_dir("sort-title-artist");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "Goodbye", Some("Spice Girls")),
            song(2, "Goodbye", None),
            song(3, "Goodbye", Some("Air Supply")),
            song(4, "Gotta Tell You", Some("Aaron")),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let listed: Vec<(String, Option<String>)> = library
        .search(&SearchQuery {
            sort: SortOrder::Title,
            ..SearchQuery::default()
        })
        .expect("search")
        .into_iter()
        .map(|s| (s.title, s.artist))
        .collect();
    assert_eq!(
        listed,
        [
            ("Goodbye".to_owned(), Some("Air Supply".to_owned())),
            ("Goodbye".to_owned(), Some("Spice Girls".to_owned())),
            ("Goodbye".to_owned(), None),
            ("Gotta Tell You".to_owned(), Some("Aaron".to_owned())),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn artists_sort_by_a_folded_key_rather_than_by_the_raw_name() {
    let dir = temp_dir("sort-artist");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("Zeca")),
            song(2, "Two", Some("Ângela")),
            song(3, "Three", Some("Bebel")),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let artists: Vec<String> = library
        .search(&SearchQuery {
            sort: SortOrder::Artist,
            ..SearchQuery::default()
        })
        .expect("search")
        .into_iter()
        .filter_map(|s| s.artist)
        .collect();
    assert_eq!(artists, ["Ângela", "Bebel", "Zeca"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `artist COLLATE NOCASE` sorted a song with no artist first, because SQLite sorts NULL first.
/// `sort_artist` is `NOT NULL DEFAULT ''` and the fold of no artist is the empty string, which sorts
/// first too — so the change is invisible here, which is exactly the property worth pinning.
#[test]
fn a_song_with_no_artist_still_sorts_before_the_named_ones() {
    let dir = temp_dir("sort-artist-none");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("Ângela")),
            song(2, "Two", None),
            song(3, "Three", Some("Bebel")),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let titles: Vec<String> = library
        .search(&SearchQuery {
            sort: SortOrder::Artist,
            ..SearchQuery::default()
        })
        .expect("search")
        .into_iter()
        .map(|s| s.title)
        .collect();
    assert_eq!(titles, ["Two", "One", "Three"]);
    let _ = std::fs::remove_dir_all(&dir);
}

fn tagged(number: u32, title: &str, tags: &[&str]) -> SongEntry {
    let mut entry = song(number, title, Some("A"));
    entry.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
    entry
}

/// Two tags widen to the union, not the intersection.
///
/// The property that makes the filter worth having over an open vocabulary: one kind of song is
/// filed under several words, so a second pick brings the rows filed under the other word in. A
/// clause per tag would instead answer the second pick with the handful of songs somebody happened
/// to file under both, and usually with nothing at all.
///
/// An empty list is still no filter, which is the case a bare `any` gets wrong.
#[test]
fn two_tags_widen_to_the_songs_that_carry_either() {
    let dir = temp_dir("tag-or");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            tagged(1, "Both", &["rock", "brasil"]),
            tagged(2, "Rock only", &["rock"]),
            tagged(3, "Brasil only", &["brasil"]),
            tagged(4, "Neither", &[]),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let titles = |tags: &[&str]| {
        let mut found: Vec<String> = library
            .search(&SearchQuery {
                tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
                sort: SortOrder::Number,
                ..SearchQuery::default()
            })
            .expect("search")
            .into_iter()
            .map(|found| found.title)
            .collect();
        found.sort();
        found
    };

    assert_eq!(titles(&["rock"]), ["Both", "Rock only"]);
    assert_eq!(titles(&["brasil"]), ["Both", "Brasil only"]);
    assert_eq!(
        titles(&["rock", "brasil"]),
        ["Both", "Brasil only", "Rock only"]
    );

    // A word nobody has used carries no rows of its own and takes none away from the tag beside it.
    assert_eq!(
        titles(&["rock", "nobody-typed-this"]),
        ["Both", "Rock only"]
    );

    // No tags is no tag filter, which the `IN` has to be guarded against reading as *no rows*.
    assert_eq!(titles(&[]), ["Both", "Brasil only", "Neither", "Rock only"]);

    // And the filter speaks the alphabet the column was written in, because what arrives in a query
    // string is a word somebody typed rather than a slug they spelled.
    assert_eq!(titles(&["ROCK"]), ["Both", "Rock only"]);

    // The tags come back on the song, sorted, so nothing has to ask a second question.
    let found = library
        .song(SongCode::new(1_001))
        .expect("query")
        .expect("1");
    assert_eq!(found.tags, ["brasil", "rock"]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A package rebuilt with **only its tags changed** moves `catalog_version`.
///
/// This is the test that earns the packed `songs.tags` column its place. `package_digest` reads
/// through `SONG_COLUMNS`, so a tag living only in the `song_tags` join table would leave the digest
/// identical — and every mirrored phone in the house would keep the tags it downloaded once, for
/// ever, with nothing anywhere reporting a fault. The twin of the unchanged-reinstall assertion
/// above it, and it fails without the column.
#[test]
fn a_package_whose_only_change_is_its_tags_moves_the_catalog_version() {
    let dir = temp_dir("tag-digest");

    let mut library = Library::open_in_memory().expect("open");
    let plain = build_package(&dir, VOL1, vec![tagged(1, "One", &[])]);
    library.install(&plain, 1, NOW).expect("install");
    let before = library.catalog_version().expect("version");

    // The same package again: nothing changed, so no mirror is sent anywhere. Reinstalling every
    // configured package is what the machine does at every start, so this half is not hypothetical.
    library.install(&plain, 1, NOW).expect("reinstall");
    assert_eq!(
        library.catalog_version().expect("version"),
        before,
        "an unchanged reinstall must not send every mirror to re-download"
    );

    // Same id, same songs, same bytes — one tag added. `build_package` writes over the same file.
    let retagged = build_package(&dir, VOL1, vec![tagged(1, "One", &["rock"])]);
    library.install(&retagged, 1, NOW).expect("install tagged");
    assert!(
        library.catalog_version().expect("version") > before,
        "a tag change has to reach the phones"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A song whose words are turned off reaches the machine saying so, and tells the phones.
///
/// Two assertions for one column, both of which the machine depends on: the flag has to survive
/// the install, because playback reads it at song start, and it has to move `catalog_version`,
/// because a package rebuilt for nothing but this really is a different package to look at.
#[test]
fn a_song_whose_words_are_turned_off_survives_the_install_and_moves_the_version() {
    let dir = temp_dir("lyrics-hidden");

    let mut library = Library::open_in_memory().expect("open");
    let plain = build_package(&dir, VOL1, vec![song(1, "One", Some("A"))]);
    library.install(&plain, 1, NOW).expect("install");
    let before = library.catalog_version().expect("version");

    let code = SongCode::in_bank(1, 1).expect("code");
    assert!(
        !library
            .song(code)
            .expect("song")
            .expect("installed")
            .lyrics_hidden
    );

    let mut silenced = song(1, "One", Some("A"));
    silenced.lyrics_hidden = true;
    let rebuilt = build_package(&dir, VOL1, vec![silenced]);
    library.install(&rebuilt, 1, NOW).expect("reinstall");

    assert!(
        library
            .song(code)
            .expect("song")
            .expect("installed")
            .lyrics_hidden,
        "playback reads this at song start, so it has to be on the row"
    );
    assert!(
        library.catalog_version().expect("version") > before,
        "what a song puts on a television is part of the package, so the mirrors re-read"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A song's corrections reach the catalog and come back as they went in.
#[test]
fn a_fix_list_survives_the_round_trip_through_the_catalog() {
    let dir = temp_dir("fixes-round-trip");
    let mut entry = song(1, "Corrected", None);
    entry.fixes = vec![
        km_fixes::Fix::IgnoreBankSelect { channel: 4 },
        km_fixes::Fix::MuteChannel { channel: 2 },
    ];
    let package = build_package(&dir, VOL1, vec![entry.clone()]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let stored = library
        .song(SongCode::new(1001))
        .expect("query")
        .expect("the song is there");
    assert_eq!(stored.fixes, entry.fixes);

    // And it resolves to the table playback reads, which is the only reason the column exists.
    let resolved = km_fixes::resolve(&stored.fixes);
    assert!(resolved.ignore_bank[4]);
    assert!(resolved.mute[2]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A song with nothing wrong stores an empty list rather than a NULL nobody can read.
#[test]
fn a_song_with_no_corrections_reads_back_as_an_empty_list() {
    let dir = temp_dir("fixes-empty");
    let package = build_package(&dir, VOL1, vec![song(1, "Plain", None)]);
    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let stored = library
        .song(SongCode::new(1001))
        .expect("query")
        .expect("the song is there");
    assert!(stored.fixes.is_empty());
    assert!(km_fixes::resolve(&stored.fixes).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
