//! Mirroring the catalog: the keyset export and the version that says whether to bother.
//!
//! These exist for `km-remote`, which keeps its own copy of a machine's catalog so it can be
//! browsed with the machine switched off. Two properties matter to it and to nothing else, so they
//! are pinned here rather than left to the API's tests: **a page boundary never loses or repeats a
//! song**, and **the version moves if and only if the set of songs did**.

use std::path::PathBuf;

use km_catalog::{Library, SongCode};
use km_kmpkg::{Package, PackageBuilder, PackageMeta, SongEntry};

const NOW: &str = "2026-08-27T12:00:00Z";

/// Two package ids of the generated shape, which is the only shape a manifest may carry.
const VOL1: &str = "1f4a9c8e2b7d0356";
const VOL2: &str = "a1b2c3d4e5f60789";

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("km-catalog-export-{name}"));
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

fn song(number: u32, title: &str, artist: Option<&str>, language: Option<&str>) -> SongEntry {
    SongEntry {
        number,
        kind: km_kmpkg::SongKind::Midi,
        title: title.to_owned(),
        artist: artist.map(ToOwned::to_owned),
        language: language.map(ToOwned::to_owned),
        file: String::new(),
        duration_ms: 200_000,
        lyric_encoding: None,
        default_transpose: 0,
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

/// The property a mirror depends on: page after page, following the last number seen, covers the
/// catalog exactly once. A boundary that dropped or repeated a song would give somebody a song
/// list quietly missing one, which nothing downstream could detect.
#[test]
fn paging_by_the_last_number_covers_every_song_exactly_once() {
    let dir = temp_dir("keyset");
    // Slots 1..25, installed into bank 10, so the codes are 10001..10025 — the package numbers its
    // songs from 1 and the machine puts them in a thousand, which is the whole arrangement in one
    // line.
    let songs: Vec<SongEntry> = (1..=25)
        .map(|n| song(n, &format!("Song {n}"), Some("Somebody"), None))
        .collect();
    let package = build_package(&dir, VOL1, songs);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 10, NOW).expect("install");

    let mut seen: Vec<SongCode> = Vec::new();
    let mut after: Option<SongCode> = None;
    loop {
        let page = library.export_after(after, 7).expect("export");
        if page.is_empty() {
            break;
        }
        after = page.last().map(|song| song.number);
        seen.extend(page.iter().map(|song| song.number));
    }

    let expected: Vec<SongCode> = (1..=25).map(|n| SongCode::new(10_000 + n)).collect();
    assert_eq!(seen, expected, "every code, in order, once");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_of_an_empty_catalog_is_an_empty_page_and_not_an_error() {
    let library = Library::open_in_memory().expect("open");
    assert!(library.export_after(None, 100).expect("export").is_empty());
}

/// A mirror stores the version it copied and re-reads only when it moves. Both halves matter: a
/// version that failed to move after an install would leave somebody's new songs invisible on their
/// phone until something else happened to bump it.
#[test]
fn the_version_moves_when_the_catalog_does_and_not_otherwise() {
    let dir = temp_dir("version");
    let one = build_package(&dir, VOL1, vec![song(1, "One", None, None)]);
    let two = build_package(&dir, VOL2, vec![song(2, "Two", None, None)]);

    let mut library = Library::open_in_memory().expect("open");
    let start = library.catalog_version().expect("version");

    library.install(&one, 1, NOW).expect("install one");
    let after_one = library.catalog_version().expect("version");
    assert!(after_one > start, "installing moved it");

    // Reading it does not move it, which is the whole point of storing it.
    assert_eq!(library.catalog_version().expect("version"), after_one);

    // A bank of its own, because two packages cannot share one — which is exactly what stops their
    // songs sharing a number.
    library.install(&two, 2, NOW).expect("install two");
    let after_two = library.catalog_version().expect("version");
    assert!(after_two > after_one, "a second install moved it again");

    library.uninstall(VOL1).expect("uninstall");
    let after_removal = library.catalog_version().expect("version");
    assert!(after_removal > after_two, "uninstalling moved it");

    // Uninstalling something that is not installed changed nothing, so it says nothing changed.
    library.uninstall(VOL1).expect("uninstall again");
    assert_eq!(
        library.catalog_version().expect("version"),
        after_removal,
        "a no-op uninstall must not send a mirror to re-read the catalog"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The property the machine's own startup depends on, and the one the test above cannot see.
///
/// `Machine::install_startup_packages` reinstalls every configured package at **every start**,
/// deliberately, so that a package rebuilt with a new column fills it. So if installing the *same*
/// package twice moved the counter, it would move on every start of the machine and every mirror in
/// the house would re-download a catalog that had not changed — which is what it did.
///
/// The test above cannot catch that: it installs two *different* packages, so both of its installs
/// legitimately move the version. Only a repeat of one package can tell the two apart.
#[test]
fn reinstalling_an_unchanged_package_does_not_move_the_version() {
    let dir = temp_dir("reinstall");
    let songs = || {
        vec![
            song(1, "One", None, None),
            song(2, "Two", Some("Someone"), Some("en")),
        ]
    };
    let package = build_package(&dir, VOL1, songs());

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");
    let after_first = library.catalog_version().expect("version");

    // Twice more, as two restarts of the machine would.
    library.install(&package, 1, NOW).expect("reinstall");
    library.install(&package, 1, NOW).expect("reinstall again");
    assert_eq!(
        library.catalog_version().expect("version"),
        after_first,
        "reinstalling an unchanged package must not send every mirror to re-read the catalog"
    );

    // ...and it is not simply stuck. A package rebuilt with a song genuinely different moves it,
    // which is the half that matters more: a version that failed to move would leave somebody's
    // corrected title invisible on their phone until something else happened to bump it.
    let changed = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", None, None),
            song(2, "Two", Some("Someone Else"), Some("en")),
        ],
    );
    library
        .install(&changed, 1, NOW)
        .expect("install the rebuilt package");
    let after_change = library.catalog_version().expect("version");
    assert!(
        after_change > after_first,
        "a package whose songs changed must move it"
    );

    // The bank is not in the manifest but is inside every song's number, so re-installing the same
    // package into a different one is a change to exactly what a mirror stores. It needs no special
    // handling in the digest and never did: the digest hashes `number`, and the bank is in there.
    library
        .install(&changed, 7, NOW)
        .expect("install into another bank");
    assert!(
        library.catalog_version().expect("version") > after_change,
        "moving a package to another bank renumbers its songs and must move it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_version_survives_reopening_the_file() {
    let dir = temp_dir("persist");
    let path = dir.join("library.sqlite");
    let package = build_package(&dir, VOL1, vec![song(1, "One", None, None)]);

    let recorded = {
        let mut library = Library::open(&path).expect("open");
        library.install(&package, 1, NOW).expect("install");
        library.catalog_version().expect("version")
    };

    let library = Library::open(&path).expect("reopen");
    assert_eq!(library.catalog_version().expect("version"), recorded);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The remote's language picker. Ordered by how much of the catalog each accounts for, because a
/// picker whose first entry is whatever sorts first alphabetically is a picker somebody has to read.
#[test]
fn languages_are_counted_and_the_commonest_comes_first() {
    let dir = temp_dir("languages");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", None, Some("pt")),
            song(2, "Two", None, Some("pt")),
            song(3, "Three", None, Some("pt")),
            song(4, "Four", None, Some("en")),
            song(5, "Five", None, None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let languages = library.languages(&[]).expect("languages");
    assert_eq!(
        languages,
        vec![("pt".to_owned(), 3), ("en".to_owned(), 1)],
        "songs with no language recorded are not a language"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn artists_are_counted_and_can_be_narrowed_by_name() {
    let dir = temp_dir("artists");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("Legião Urbana"), None),
            song(2, "Two", Some("Legião Urbana"), None),
            song(3, "Three", Some("Cazuza"), None),
            song(4, "Four", None, None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let all = library.artists(None, &[]).expect("artists");
    assert_eq!(
        all,
        vec![("Cazuza".to_owned(), 1), ("Legião Urbana".to_owned(), 2)],
        "songs with no artist are not somebody to browse"
    );

    let narrowed = library.artists(Some("urbana"), &[]).expect("artists");
    assert_eq!(narrowed, vec![("Legião Urbana".to_owned(), 2)]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A person hides a package on their own remote. Its songs leave the search and every picker, and
/// the counts left are the songs that person can reach.
#[test]
fn a_hidden_package_leaves_the_search_and_every_picker() {
    let dir = temp_dir("hidden");
    let tagged = |number, title, artist, language, tag: &str| {
        let mut entry = song(number, title, Some(artist), Some(language));
        entry.tags = vec![tag.to_owned()];
        entry
    };
    let kept = build_package(
        &dir,
        VOL1,
        vec![
            tagged(1, "One", "Cazuza", "pt", "rock"),
            tagged(2, "Two", "Shared", "pt", "rock"),
        ],
    );
    let hidden = build_package(
        &dir,
        VOL2,
        vec![
            tagged(1, "Three", "Only Hidden", "en", "kids"),
            tagged(2, "Four", "Shared", "pt", "rock"),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&kept, 1, NOW).expect("install kept");
    library.install(&hidden, 2, NOW).expect("install hidden");
    let hide = [VOL2.to_owned()];

    let titles: Vec<String> = library
        .search(&km_catalog::SearchQuery {
            exclude_packages: hide.to_vec(),
            sort: km_catalog::SortOrder::Number,
            ..Default::default()
        })
        .expect("search")
        .into_iter()
        .map(|song| song.title)
        .collect();
    assert_eq!(titles, ["One", "Two"]);

    assert_eq!(
        library.artists(None, &hide).expect("artists"),
        vec![("Cazuza".to_owned(), 1), ("Shared".to_owned(), 1)]
    );
    assert_eq!(
        library.languages(&hide).expect("languages"),
        vec![("pt".to_owned(), 2)]
    );
    assert_eq!(
        library.tags(&hide).expect("tags"),
        vec![("rock".to_owned(), 2)]
    );
    assert_eq!(
        library.tags(&[]).expect("tags"),
        vec![("rock".to_owned(), 3), ("kids".to_owned(), 1)],
        "nothing hidden counts every package"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn artists_are_narrowed_accent_insensitively() {
    let dir = temp_dir("artists-folded");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("Legião Urbana"), None),
            song(2, "Two", Some("Cazuza"), None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    assert_eq!(
        library.artists(Some("legiao"), &[]).expect("artists"),
        vec![("Legião Urbana".to_owned(), 1)],
        "a singer typing the name without its tilde still finds the band"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_artist_list_is_ordered_by_the_folded_name() {
    let dir = temp_dir("artists-order");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("Zeca"), None),
            song(2, "Two", Some("Ângela"), None),
            song(3, "Three", Some("Bebel"), None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let names: Vec<String> = library
        .artists(None, &[])
        .expect("artists")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["Ângela", "Bebel", "Zeca"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Two spellings of one name stay two rows**, and this is a guard rather than a description. The
/// tempting simplification is `GROUP BY sort_artist`, which merges them into one row of two — but
/// the drill-down from a row is exact (`s.artist = ?`), so that row would advertise two songs and
/// open onto one. What the fold buys here is that the two land next to each other, which is what
/// lets somebody notice the duplicate at all.
#[test]
fn two_spellings_of_one_artist_stay_two_rows() {
    let dir = temp_dir("artists-spellings");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("Legião Urbana"), None),
            song(2, "Two", Some("Legiao Urbana"), None),
            song(3, "Three", Some("Zeca"), None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let rows = library.artists(None, &[]).expect("artists");
    assert_eq!(
        rows,
        vec![
            ("Legiao Urbana".to_owned(), 1),
            ("Legião Urbana".to_owned(), 1),
            ("Zeca".to_owned(), 1),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `%` is a wildcard in `LIKE`, and until the escaping was shared it was passed through — so an
/// artist search for `50%` matched every artist there was.
///
/// **It passes for a second reason now**, which is worth knowing before trusting it: the needle is
/// folded before it is escaped, and `fold` turns `%` into a space and trims it. `escape_like` stays
/// anyway — the safety of this line should not rest on another function's internals.
#[test]
fn a_wildcard_typed_into_an_artist_search_is_a_literal() {
    let dir = temp_dir("wildcard");
    let package = build_package(
        &dir,
        VOL1,
        vec![
            song(1, "One", Some("50% Off"), None),
            song(2, "Two", Some("Cazuza"), None),
        ],
    );

    let mut library = Library::open_in_memory().expect("open");
    library.install(&package, 1, NOW).expect("install");

    let narrowed = library.artists(Some("50%"), &[]).expect("artists");
    assert_eq!(narrowed, vec![("50% Off".to_owned(), 1)]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Reconciling against the set that is already there must not move the version.
///
/// **The most load-bearing test of `retain_packages`**, and for the same reason
/// `reinstalling_an_unchanged_package_does_not_move_the_version` exists above: the reconcile runs at
/// *every* pass, and the overwhelmingly common outcome is that nothing has gone. A version that
/// moved anyway would send every mirror in the house to re-download a catalog that had not
/// changed — the fault this file was written to pin down.
#[test]
fn reconciling_against_an_unchanged_set_does_not_move_the_version() {
    let dir = temp_dir("retain-unchanged");
    let one = build_package(&dir, VOL1, vec![song(1, "One", None, None)]);
    let two = build_package(&dir, VOL2, vec![song(2, "Two", None, None)]);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&one, 1, NOW).expect("install one");
    library.install(&two, 2, NOW).expect("install two");
    let settled = library.catalog_version().expect("version");

    let dropped = library.retain_packages(&[VOL1, VOL2]).expect("reconcile");
    assert!(dropped.is_empty(), "nothing was missing, so nothing went");
    assert_eq!(
        library.catalog_version().expect("version"),
        settled,
        "a pass that found the catalog already right must not move the version"
    );

    // And a `keep` naming something that is not installed is still not a change.
    let dropped = library
        .retain_packages(&[VOL1, VOL2, "vol3-never-installed"])
        .expect("reconcile");
    assert!(dropped.is_empty());
    assert_eq!(library.catalog_version().expect("version"), settled);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A package the reconcile was not told to keep loses its rows, its songs and its searchability.
///
/// The search half is not decoration: it is the only check that the two hops actually connect —
/// deleting the package row cascades to its songs, and the songs' own trigger takes them out of the
/// full-text index. A phantom that is gone from the count but still findable by title is exactly the
/// shape of bug this replaces.
#[test]
fn reconciling_drops_what_it_was_not_told_to_keep() {
    let dir = temp_dir("retain-drops");
    let one = build_package(&dir, VOL1, vec![song(1, "Only Mine", None, None)]);
    let two = build_package(&dir, VOL2, vec![song(2, "Gone Away", None, None)]);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&one, 1, NOW).expect("install one");
    library.install(&two, 2, NOW).expect("install two");
    let before = library.catalog_version().expect("version");
    assert_eq!(library.song_count().expect("count"), 2);

    let dropped = library.retain_packages(&[VOL1]).expect("reconcile");
    assert_eq!(dropped, vec![VOL2.to_owned()]);
    assert_eq!(library.song_count().expect("count"), 1);
    assert_eq!(library.package_count().expect("count"), 1);
    assert!(
        library.catalog_version().expect("version") > before,
        "songs left the catalog, so a mirror has to hear about it"
    );

    // No longer findable, which is the hop a cascade alone would not have taken.
    let hits = library
        .search(&km_catalog::search::SearchQuery {
            text: Some("Gone Away".to_owned()),
            ..Default::default()
        })
        .expect("search");
    assert!(
        hits.is_empty(),
        "a dropped package's songs must leave the search index too: {hits:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty `keep` empties the catalog rather than failing.
///
/// Included because it discriminates between two implementations: a `DELETE` with a `NOT IN` list
/// would make this the one case that is a SQL syntax error, and it is the case an owner reaches by
/// emptying the packages folder — the least acceptable place to fail.
#[test]
fn reconciling_with_an_empty_keep_list_empties_the_catalog() {
    let dir = temp_dir("retain-empty");
    let one = build_package(&dir, VOL1, vec![song(1, "One", None, None)]);

    let mut library = Library::open_in_memory().expect("open");
    library.install(&one, 1, NOW).expect("install");
    assert_eq!(library.song_count().expect("count"), 1);

    let dropped = library.retain_packages(&[]).expect("reconcile");
    assert_eq!(dropped, vec![VOL1.to_owned()]);
    assert_eq!(library.song_count().expect("count"), 0);
    assert_eq!(library.package_count().expect("count"), 0);

    // ...and doing it again on an already-empty catalog is not a change.
    let settled = library.catalog_version().expect("version");
    assert!(library.retain_packages(&[]).expect("reconcile").is_empty());
    assert_eq!(library.catalog_version().expect("version"), settled);

    let _ = std::fs::remove_dir_all(&dir);
}
