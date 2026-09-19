//! The curation database's tests.
//!
//! **One module, in its own file, rather than one per `impl Db` block.** The subjects are split
//! across `songs.rs`, `packages.rs`, `favorites.rs`, `scan.rs` and `migrate.rs`; the tests are not,
//! because they share a corpus fixture and a `db()` helper and most of them cross two of those
//! blocks in a single assertion — a rebuild that must carry every hand-set field across is a songs
//! test, a packages test and a migration test at once. Splitting them would have meant a fixture
//! module for the fixtures to live in, which is one more file than the sharing is worth.
//!
//! `super::*` reaches every one of those blocks: a `pub(super)` helper in a sibling is visible in
//! `db`, and this module is `db`'s child.

use super::*;

use crate::testing::Scratch;

fn db() -> Db {
    Db::open_in_memory(Path::new("/corpus")).expect("open")
}

#[test]
fn a_fresh_database_is_empty_but_valid() {
    let db = db();
    let counts = db.counts().expect("counts");
    assert_eq!(counts.songs, 0);
    assert_eq!(counts.files, 0);
    assert!(db.songs(&Filter::default()).expect("songs").is_empty());
}

#[test]
fn opening_a_folder_with_no_database_is_refused_rather_than_created() {
    let Err(error) = Db::open(Path::new("/definitely/not/here")) else {
        panic!("opening a folder with no database must be refused");
    };
    assert!(error.to_string().contains("--init"), "{error}");

    // `main` makes the same refusal itself, before it binds a port and prints a URL — so it has
    // to be the same refusal. Two copies of this message would let the pre-bind one drift into
    // saying something that is no longer true of the real check.
    let Err(hoisted) = require_database(Path::new("/definitely/not/here")) else {
        panic!("the check main makes before binding must refuse too");
    };
    assert_eq!(
        hoisted.to_string(),
        error.to_string(),
        "the early refusal and the real one are one message"
    );
}

#[test]
fn typed_text_cannot_become_fts_syntax() {
    assert_eq!(fts_match_query("tom jobim"), "\"tom\" \"jobim\"*");
    // `OR` and `*` are operators in FTS5 and must not survive as such.
    assert_eq!(fts_match_query("a OR b"), "\"a\" \"OR\" \"b\"*");
    assert_eq!(fts_match_query("!!!"), "\"\"");
    // Every `"` in the answer is one written by the function. A mark somebody typed opens or closes
    // a phrase and never reaches the expression, so no input can close a token and start syntax.
    assert_eq!(fts_match_query(r#"a" OR "b"#), "\"a\" \"OR\" \"b\"*");
}

/// A quoted run is one phrase: those words, in that order, next to each other.
///
/// Two words in the box match a song holding both anywhere, which is right for a half-remembered
/// fragment and wrong for a line somebody can quote — over a corpus this size *quiet nights* alone
/// is thousands of songs with the two words nowhere near each other.
#[test]
fn quoted_text_asks_for_the_words_in_that_order() {
    assert_eq!(
        fts_match_query(r#""quiet nights""#),
        "\"quiet nights\"",
        "a closed phrase is exact, so no prefix star widens its last word"
    );
    // A phrase beside loose words, either way round. The star lands on the last group only when
    // that group is not a closed phrase.
    assert_eq!(
        fts_match_query(r#""quiet nights" stars"#),
        "\"quiet nights\" \"stars\"*"
    );
    assert_eq!(
        fts_match_query(r#"tom "quiet nights""#),
        "\"tom\" \"quiet nights\""
    );
    // Quoting one word is asking for that word and not for things starting with it, which is the
    // only way to say so: the last token is otherwise always a prefix.
    assert_eq!(fts_match_query("\"quoted\""), "\"quoted\"");

    // Still being typed. The mark has not been closed yet, so the phrase is a phrase prefix and the
    // results narrow as the line is typed rather than meaning something else until it is finished.
    assert_eq!(
        fts_match_query(r#""quiet nig"#),
        "\"quiet nig\"*",
        "an unclosed phrase is half-typed, which is what the star is for"
    );
    assert_eq!(
        fts_match_query(r#"tom "quiet nig"#),
        "\"tom\" \"quiet nig\"*"
    );

    // Punctuation inside a phrase is a word boundary like anywhere else, so a phrase cannot carry a
    // token this tokenizer would not have produced.
    assert_eq!(
        fts_match_query(r#""don't stop""#),
        "\"don t stop\"",
        "the same split as outside the marks"
    );
    // Empty marks add nothing and must not become a phrase matching everything or nothing.
    assert_eq!(fts_match_query(r#"tom "" jobim"#), "\"tom\" \"jobim\"*");
    assert_eq!(fts_match_query(r#""""#), "\"\"");
}

/// A discarded song is in no list a curator can act from, words search included.
///
/// **The Lyrics page is the one that mattered.** A hit carries the browse row's own buttons — the
/// star and *add to a package* among them — so a deleted song reaching this list is a deleted song
/// one press away from a build. The count above the list asks the same predicate, or the number
/// and the rows disagree.
///
/// The ticked writers are here for the same reason at one remove: the ids come off a page, and a
/// page can hold a discarded song through *only deleted*, a saved filter or a tab left open.
#[test]
fn a_song_thrown_away_is_in_no_list_and_takes_no_bulk_write() {
    let mut db = db();
    for id in ["kept", "gone"] {
        add_scanned(&mut db, id, |song| {
            song.lyrics = Some("quiet nights of quiet stars".to_owned());
        });
    }
    let search = LyricSearch {
        query: "quiet".to_owned(),
        limit: 50,
        offset: 0,
    };
    assert_eq!(db.lyric_search(&search).expect("search").len(), 2);
    assert_eq!(db.lyric_search_count(&search).expect("count"), 2);

    db.set_deleted_of(&["gone".to_owned()], true)
        .expect("delete");

    let hits = db.lyric_search(&search).expect("search");
    assert_eq!(
        ids(&hits.iter().map(|hit| hit.song.clone()).collect::<Vec<_>>()),
        vec!["kept"]
    );
    assert_eq!(
        db.lyric_search_count(&search).expect("count"),
        1,
        "the number over the list is the list's own predicate"
    );

    // Every ticked write counts what it changed, so a row nobody can see would be a number saying
    // work was done where none was.
    let both = ["kept".to_owned(), "gone".to_owned()];
    assert_eq!(
        db.set_language_of(&both, Some(Language::parse("en").expect("code")), false)
            .expect("language"),
        1
    );
    assert_eq!(
        db.add_tag_of(&both, &Tag::parse("bossa").expect("tag"))
            .expect("tag"),
        1
    );
    assert_eq!(db.set_names_from_stem(&both).expect("titles"), 1);
    assert_eq!(db.fix_name_case(&both).expect("capitals"), 1);
    assert_eq!(db.paths_of(&both).expect("paths").len(), 1);
}

/// And FTS5 reads what that function writes the way it is meant.
///
/// The assertions above are about a string, and a string that looks like a phrase query is not a
/// phrase query: `"a b"` and `"a b"*` are both constructions SQLite either honours or rejects, and
/// either failure is invisible to a test that never runs one. So this puts all three shapes through
/// the real index over lyrics chosen to tell them apart.
#[test]
fn a_phrase_reaches_sqlite_as_a_phrase() {
    let mut db = db();
    // Both songs hold both words. Only one holds them in that order and next to each other, which
    // is the whole difference a pair of marks buys.
    add_scanned(&mut db, "adjacent", |song| {
        song.lyrics = Some("quiet nights of quiet stars".to_owned());
    });
    add_scanned(&mut db, "apart", |song| {
        song.lyrics = Some("nights are long and the lake is quiet".to_owned());
    });

    let found = |query: &str| {
        let mut ids: Vec<String> = db
            .lyric_search(&LyricSearch {
                query: query.to_owned(),
                limit: 50,
                offset: 0,
            })
            .expect("lyric search")
            .into_iter()
            .map(|hit| hit.song.id)
            .collect();
        ids.sort();
        ids
    };

    assert_eq!(
        found("quiet nights"),
        ["adjacent", "apart"],
        "two loose words match a song holding both anywhere"
    );
    assert_eq!(
        found(r#""quiet nights""#),
        ["adjacent"],
        "and the marks are what narrow it to the line somebody quoted"
    );
    // Half-typed, and still narrowing rather than meaning something else until the mark is closed.
    assert_eq!(found(r#""quiet nig"#), ["adjacent"]);
}

// -- finding a song by the words it sings ------------------------------------------------------

/// Verses of a song, one line each, well clear of the floor a lyric has to reach.
///
/// Every line distinct, so a test can take one away and say which one.
pub(crate) fn verses(from: usize, to: usize) -> String {
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

/// Whether `lyrics_vocab` answers, which is the guard against a SQLite built without `fts5vocab`.
///
/// A missing virtual table fails at `Db::open`, so this would never be the only thing to break —
/// but the count it returns is what the phrase choosing leans on, and a table that existed and
/// answered nothing would leave that silently taking the first phrase of every stretch.
#[test]
fn the_lyric_vocabulary_counts_the_songs_holding_a_word() {
    let mut db = db();
    add_scanned(&mut db, "one", |song| {
        song.lyrics = Some(verses(0, 3));
    });
    add_scanned(&mut db, "two", |song| {
        song.lyrics = Some(verses(0, 3));
    });
    add_scanned(&mut db, "alone", |song| {
        song.lyrics = Some(format!("{}\nzarzuela", verses(9, 12)));
    });

    let held_by = |word: &str| -> i64 {
        db.conn
            .query_row(
                "SELECT doc FROM lyrics_vocab WHERE term = ?1",
                [word],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
    };
    assert_eq!(held_by("rain"), 3, "every song sings it");
    assert_eq!(held_by("zarzuela"), 1, "one song sings it");
    assert_eq!(
        held_by("nothinghere"),
        0,
        "and a word nothing sings is absent"
    );
}

/// Two files of one song under names that share nothing are found by their words.
#[test]
fn a_song_filed_twice_under_two_names_is_found_by_its_words() {
    let mut db = db();
    add_scanned(&mut db, "known", |song| {
        song.det_title = Some("Dancing In The Dark".to_owned());
        song.lyrics = Some(verses(0, 5));
    });
    add_scanned(&mut db, "earthw2", |song| {
        song.det_title = Some("EARTHW~2".to_owned());
        song.lyrics = Some(verses(0, 5));
    });
    add_scanned(&mut db, "other", |song| {
        song.det_title = Some("Something Else".to_owned());
        song.lyrics = Some(verses(20, 25));
    });

    let (hits, comparable) = db
        .similar_words("known", &Filter::default())
        .expect("same words");
    assert!(comparable);
    let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(
        ids,
        ["known", "earthw2"],
        "the song searched from heads the list, and no name was needed to reach the other"
    );
    assert!(hits[0].searched_from);
    assert_eq!(hits[1].likeness, Some(1.0));
}

/// A file with a verse missing is what the exact key cannot reach, and the whole point.
#[test]
fn a_file_missing_a_verse_is_still_the_same_song() {
    let mut db = db();
    add_scanned(&mut db, "whole", |song| {
        song.lyrics = Some(verses(0, 5));
    });
    add_scanned(&mut db, "short", |song| {
        song.lyrics = Some(verses(0, 4));
    });
    // What the duplicate pass sees, which is the reason this page exists.
    assert_ne!(
        crate::dupes::lyric_key(&verses(0, 5)),
        crate::dupes::lyric_key(&verses(0, 4)),
        "the exact key parts them"
    );

    let (hits, _) = db
        .similar_words("whole", &Filter::default())
        .expect("same words");
    let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids, ["whole", "short"], "and this does not");
}

/// A song with nothing to compare says so, which is not the same as nothing matching.
#[test]
fn a_song_with_too_few_words_is_not_compared_at_all() {
    let mut db = db();
    add_scanned(&mut db, "instrumental", |song| {
        song.lyrics = None;
    });
    add_scanned(&mut db, "carded", |song| {
        // A lyric track holding nothing but the sequencer's card, which a great many real files are.
        song.lyrics = Some("Sequenced by Somebody, 123 Any Street, Anytown".to_owned());
    });

    for id in ["instrumental", "carded"] {
        let (hits, comparable) = db
            .similar_words(id, &Filter::default())
            .expect("same words");
        assert!(!comparable, "{id} has no words to compare");
        assert!(hits.is_empty(), "{id}");
    }
}

/// The narrowing runs in SQL, so a match a filter drops is dropped before anything is scored.
#[test]
fn the_filter_narrows_the_same_words_candidates() {
    let mut db = db();
    add_scanned(&mut db, "origin", |song| {
        song.lyrics = Some(verses(0, 5));
    });
    add_scanned(&mut db, "twin", |song| {
        song.lyrics = Some(verses(0, 5));
    });

    let (hits, _) = db
        .similar_words(
            "origin",
            &Filter {
                suitability: SuitabilityFilter::High,
                ..Filter::default()
            },
        )
        .expect("same words");
    let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(
        ids,
        ["origin"],
        "the fixtures score 7, so the high band keeps neither -- and the song searched from is \
         listed anyway, because every other row is read against it"
    );
}

#[test]
fn a_filter_binds_its_values_rather_than_interpolating_them() {
    let filter = Filter {
        query: Some("o'brien".to_owned()),
        suitability: SuitabilityFilter::Middle,
        favorited: FavoritedFilter::In,
        ..Filter::default()
    };
    let (sql, values) = filter.to_sql();
    assert!(sql.contains("MATCH ?1"), "{sql}");
    // The band's bounds are this code's own constants, so they are written into the fragment and
    // are deliberately *not* a second binding — which is what the count below pins.
    assert!(sql.contains("s.suitability BETWEEN 5 AND 7"), "{sql}");
    assert!(
        sql.contains("s.id IN (SELECT song_id FROM song_favorites)"),
        "{sql}"
    );
    assert!(!sql.contains("o'brien"), "the text must not reach the SQL");
    assert_eq!(values.len(), 1);

    // A range's ends go the same way, and they can: `parse` reads them as integers and refuses
    // anything outside 0–10, so what reaches the fragment is a number rather than what was typed.
    let ranged = Filter {
        suitability: SuitabilityFilter::parse("2-5"),
        ..Filter::default()
    };
    let (sql, values) = ranged.to_sql();
    assert!(sql.contains("s.suitability BETWEEN 2 AND 5"), "{sql}");
    assert!(values.is_empty());
}

/// The three bands partition 0–10, which the `≥ N` ladder they replaced did not.
///
/// Worth a test rather than a reading of the `match`: an off-by-one at either seam is a suitability
/// that no band shows, and the symptom is a song missing from every filtered view while the
/// unfiltered page still holds it — which reads as a broken index rather than as a broken
/// boundary.
#[test]
fn every_score_falls_in_exactly_one_band() {
    for score in 0..=10u8 {
        let matched: Vec<_> = [
            SuitabilityFilter::High,
            SuitabilityFilter::Middle,
            SuitabilityFilter::Low,
        ]
        .into_iter()
        .filter(|band| match band {
            SuitabilityFilter::High => (8..=10).contains(&score),
            SuitabilityFilter::Middle => (5..=7).contains(&score),
            SuitabilityFilter::Low => score < 5,
            SuitabilityFilter::Any | SuitabilityFilter::Range { .. } => unreachable!(),
        })
        .collect();
        assert_eq!(matched.len(), 1, "score {score} matched {matched:?}");
    }

    // *any* adds no clause at all, rather than a clause that happens to be true of everything.
    assert!(SuitabilityFilter::Any.clause("s.suitability").is_none());
}

/// A hand-typed value cannot produce an empty page with no reason, and a round trip keeps it.
#[test]
fn a_band_survives_a_round_trip_and_nonsense_reads_as_any() {
    for band in [
        SuitabilityFilter::Any,
        SuitabilityFilter::High,
        SuitabilityFilter::Middle,
        SuitabilityFilter::Low,
    ] {
        assert_eq!(SuitabilityFilter::parse(&band.as_str()), band);
    }
    // A `<` is escaped by everything that touches a URL, so the control spells the low band `0-4`
    // and this is not a second spelling of it.
    assert_eq!(SuitabilityFilter::parse("<5"), SuitabilityFilter::Any);
}

/// The address can name a range the dropdown does not offer, and it survives a page turn.
///
/// Worth a test of its own rather than a reading of the `match`: every one of these values reaches
/// the same select and the same chip, and a range that fell through to *any* would answer *the 2s to
/// the 5s* with the whole corpus — the fault the three bands are written to prevent one filter over.
#[test]
fn a_range_the_address_names_parses_and_round_trips() {
    let range = |low, high| SuitabilityFilter::Range { low, high };
    assert_eq!(SuitabilityFilter::parse("2-5"), range(2, 5));
    assert_eq!(SuitabilityFilter::parse("9"), range(9, 9));
    assert_eq!(SuitabilityFilter::parse("9-10"), range(9, 10));
    // A range over the whole column is not *any*: a song with no stored suitability is outside it,
    // and outside every band, while *any* holds the whole corpus.
    assert_eq!(SuitabilityFilter::parse("0-10"), range(0, 10));

    // An open end takes the end of the column, and the round trip writes both ends down. One filter,
    // one spelling, by the rule `initial` already follows.
    assert_eq!(SuitabilityFilter::parse("7-"), range(7, 10));
    assert_eq!(SuitabilityFilter::parse("-9"), range(0, 9));
    assert_eq!(SuitabilityFilter::parse("7-").as_str(), "7-10");
    assert_eq!(SuitabilityFilter::parse("9").as_str(), "9");
    assert_eq!(SuitabilityFilter::parse("2-5").as_str(), "2-5");
    for value in ["2-5", "9", "0-10", "7-10"] {
        let parsed = SuitabilityFilter::parse(value);
        assert_eq!(SuitabilityFilter::parse(&parsed.as_str()), parsed);
    }

    // A range whose ends are a band's ends is that band, so the two spellings draw one chip.
    assert_eq!(SuitabilityFilter::parse("-4"), SuitabilityFilter::Low);
    assert_eq!(SuitabilityFilter::parse("5-7"), SuitabilityFilter::Middle);
    assert_eq!(SuitabilityFilter::parse("8-"), SuitabilityFilter::High);

    // And what no range can mean reads as *any*, so no address produces an empty page in silence.
    for nonsense in [
        "7-3", "11", "0-11", "a-b", "-", "", "2-5-7", " 2-5", "2 - 5",
    ] {
        assert_eq!(
            SuitabilityFilter::parse(nonsense),
            SuitabilityFilter::Any,
            "{nonsense:?}"
        );
    }
}

#[test]
fn unrated_songs_sort_after_rated_ones() {
    assert!(
        Filter {
            sort: Sort::UserScore,
            ..Filter::default()
        }
        .order_by()
        .starts_with("s.user_score IS NULL")
    );
}

/// A corpus opens alphabetically, not by suitability.
///
/// Pinned through `parse` rather than through `Sort::default()`, because an unset query parameter
/// arrives as the empty string and it is `parse` that decides what that means — changing the
/// `#[default]` alone would have left the browse page exactly as it was.
#[test]
fn an_unset_sort_orders_by_title() {
    assert_eq!(Sort::parse(""), Sort::Title);
    assert_eq!(Sort::parse("nonsense"), Sort::Title);
    assert_eq!(
        Sort::parse("suitability"),
        Sort::Suitability,
        "the file rating has one spelling"
    );
    assert_eq!(
        Sort::parse("score"),
        Sort::Title,
        "and `score` is not a second one"
    );
    assert!(
        Filter::default().order_by().starts_with("s.sort_title"),
        "{}",
        Filter::default().order_by()
    );
}

/// Every sort survives a round trip through its URL spelling.
///
/// A sort with a `parse` arm and no `as_str` arm, or the other way about, is a `<select>` that
/// cannot hold what a page turn puts back into it — the control silently reverting to *title* while
/// the option a person picked is still in the URL.
#[test]
fn every_sort_survives_a_round_trip() {
    for sort in [
        Sort::Suitability,
        Sort::UserScore,
        Sort::Title,
        Sort::Artist,
        Sort::Duration,
        Sort::Copies,
        Sort::Language,
        Sort::Updated,
        Sort::Added,
    ] {
        assert_eq!(Sort::parse(sort.as_str()), sort);
    }
    assert_eq!(Sort::parse("updated"), Sort::Updated);
}

#[test]
fn merged_songs_are_hidden_from_every_query() {
    let (sql, _) = Filter::default().to_sql();
    assert!(sql.contains("s.merged_into IS NULL"), "{sql}");
}

#[test]
fn settings_round_trip() {
    let db = db();
    assert!(db.setting("app_url").expect("read").is_none());
    db.set_setting("app_url", "http://127.0.0.1:8177")
        .expect("write");
    db.set_setting("app_url", "http://10.0.0.4:8177")
        .expect("overwrite");
    assert_eq!(
        db.setting("app_url").expect("read").as_deref(),
        Some("http://10.0.0.4:8177")
    );
}

#[test]
fn the_favorites_come_back_by_name() {
    let db = db();
    db.create_favorite("Rock").expect("create");
    db.create_favorite("Brasil").expect("create");
    db.create_favorite("brasil pop").expect("create");

    let names: Vec<_> = db
        .favorites()
        .expect("favorites")
        .into_iter()
        .map(|node| node.name)
        .collect();
    assert_eq!(
        names,
        vec!["Brasil", "brasil pop", "Rock"],
        "ordered by name, and a lower-case one files with its own letter rather than after Z"
    );
}

#[test]
fn two_favorites_cannot_share_a_name() {
    let db = db();
    db.create_favorite("Rock").expect("create");
    assert!(db.create_favorite("Rock").is_err());
}

#[test]
fn a_rating_above_ten_is_refused() {
    let db = db();
    assert!(db.set_user_score("whatever", Some(11)).is_err());
}

#[test]
fn the_star_files_a_song_in_one_favorite_and_a_second_click_takes_it_out() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    let bossa = db.create_favorite("Bossa").expect("create");

    assert!(
        db.toggle_favorite("song-a", bossa).expect("toggle"),
        "the first click files it"
    );
    assert_eq!(
        db.favorites_for("song-a").expect("read"),
        vec![(bossa, "Bossa".to_owned())]
    );
    assert!(
        !db.toggle_favorite("song-a", bossa).expect("toggle"),
        "the same favorite again takes it back out"
    );
    assert!(db.favorites_for("song-a").expect("read").is_empty());
}

#[test]
fn a_song_can_be_in_several_favorites_at_once() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    let bossa = db.create_favorite("Bossa").expect("create");
    let parties = db.create_favorite("Parties").expect("create");
    db.toggle_favorite("song-a", bossa).expect("toggle");
    db.toggle_favorite("song-a", parties).expect("toggle");

    assert_eq!(db.favorites_for("song-a").expect("read").len(), 2);
    // One song, however many favorites it is in.
    assert_eq!(db.counts().expect("counts").favorites, 1);
    let row = db.song_row("song-a").expect("row");
    assert_eq!(row.favorite_count, 2, "the star says how many");
}

/// A working list fills the star and does not color it, and a filing does both.
///
/// The two counts are what keeps those apart: one says how many lists a song is in, the other how
/// many of those are a filing. A song set aside is a song still to do.
#[test]
fn only_a_favorite_that_is_not_a_working_list_counts_as_filed() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    let to_check = db.create_favorite("to-check").expect("create");
    let bossa = db.create_favorite("Bossa").expect("create");
    db.set_favorite_temporary(to_check, true).expect("set");

    db.toggle_favorite("song-a", to_check).expect("toggle");
    let row = db.song_row("song-a").expect("row");
    assert_eq!(row.favorite_count, 1, "it is in a list");
    assert_eq!(row.permanent_count, 0, "and filed in none");

    db.toggle_favorite("song-a", bossa).expect("toggle");
    let row = db.song_row("song-a").expect("row");
    assert_eq!(row.favorite_count, 2);
    assert_eq!(row.permanent_count, 1, "one of the two is a filing");

    // Turning the flag back is the whole of the undo: no song moves.
    db.set_favorite_temporary(to_check, false).expect("unset");
    let row = db.song_row("song-a").expect("row");
    assert_eq!(row.favorite_count, 2);
    assert_eq!(row.permanent_count, 2);
}

/// A favorite is a filing until somebody says otherwise, which is what every list made before the
/// flag existed was made as.
#[test]
fn a_new_favorite_is_not_a_working_list() {
    let db = db();
    let bossa = db.create_favorite("Bossa").expect("create");
    let node = db
        .favorites()
        .expect("tree")
        .into_iter()
        .find(|favorite| favorite.id == bossa)
        .expect("the favorite");
    assert!(!node.temporary);
}

#[test]
fn filing_an_unknown_song_is_refused_rather_than_written() {
    let db = db();
    let bossa = db.create_favorite("Bossa").expect("create");
    assert!(db.toggle_favorite("no-such-song", bossa).is_err());
}

#[test]
fn browsing_can_ask_for_one_favorite_for_any_or_for_none() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    add(&mut db, "song-b", Some("Wave"), "b/WAVE.kar");
    add(&mut db, "song-c", Some("Garota"), "c/GAROTA.kar");
    let bossa = db.create_favorite("Bossa").expect("create");
    let parties = db.create_favorite("Parties").expect("create");
    db.toggle_favorite("song-a", bossa).expect("toggle");
    db.toggle_favorite("song-b", parties).expect("toggle");

    let in_bossa = db
        .songs(&Filter {
            favorite: Some(bossa),
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&in_bossa), vec!["song-a"]);

    let with = |favorited| {
        let mut rows = db
            .songs(&Filter {
                favorited,
                sort: Sort::Title,
                ..Filter::default()
            })
            .expect("browse");
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };

    assert_eq!(with(FavoritedFilter::In), ["song-a", "song-b"]);
    // The arm a checkbox could not offer, and the one a curation pass is made of: what is left to
    // look at. It is not the absence of the filter above — that shows all three.
    assert_eq!(with(FavoritedFilter::NotIn), ["song-c"]);
    assert_eq!(with(FavoritedFilter::Any), ["song-a", "song-b", "song-c"]);

    // A working list holds songs set aside to be decided about, so making `Parties` one takes
    // `song-b` out of what has been settled without taking it out of what has been filed anywhere.
    // The two arms disagreeing here is the whole of what the fourth one is for.
    db.set_favorite_temporary(parties, true).expect("working");
    assert_eq!(with(FavoritedFilter::Filed), ["song-a"]);
    assert_eq!(with(FavoritedFilter::In), ["song-a", "song-b"]);
    assert_eq!(with(FavoritedFilter::NotIn), ["song-c"]);
    // What is left to settle counts the song set aside with the song nobody has looked at.
    assert_eq!(with(FavoritedFilter::NotFiled), ["song-b", "song-c"]);

    // A song in a working list *and* a filing is settled: the flag is on the list, and one list
    // saying "not yet" cannot unsay another list's decision about the same song.
    db.toggle_favorite("song-b", bossa).expect("toggle");
    assert_eq!(with(FavoritedFilter::Filed), ["song-a", "song-b"]);
    assert_eq!(with(FavoritedFilter::NotFiled), ["song-c"]);
}

#[test]
fn a_favorited_filter_round_trips_through_the_query_string() {
    for value in ["", "in", "out", "filed", "unfiled"] {
        assert_eq!(FavoritedFilter::parse(value).as_str(), value, "{value:?}");
    }
    // The spelling a checkbox sent is read and never written, so a link carrying it narrows the list
    // it was written for and says `in` from its first page turn.
    assert_eq!(FavoritedFilter::parse("1"), FavoritedFilter::In);
    assert_eq!(FavoritedFilter::In.as_str(), "in");

    // Anything else shows the whole corpus rather than nothing, by the rule the rest of the bar
    // follows: a hand-edited query string must not produce an empty page with no explanation.
    for nonsense in ["0", "yes", "no", "none", "true"] {
        assert_eq!(
            FavoritedFilter::parse(nonsense),
            FavoritedFilter::Any,
            "{nonsense}"
        );
    }
}

/// The artist filter is exact, folds case and accents, and never matches a song with no artist.
///
/// **The fold is the whole reason this is usable on a real corpus.** The same performer arrives
/// under whatever spelling each sequencer chose — `DIRE STRAITS`, `Dire Straits`, an accented name
/// with and without its accents — and an exact match on the raw column would answer *what else did
/// they do?* with a fraction of the answer while looking as though it had answered. `sort_artist` is
/// `km_song::text::fold` of the effective artist, which is the alphabet the A–Z strip, the song book
/// and both catalogs already file by, so this filter agrees with all of them for free.
///
/// The last case is the one that would go unnoticed: a song with no artist has NULL there, so it
/// falls out of every artist filter rather than joining the one for the empty string.
#[test]
fn the_artist_filter_is_exact_and_folded_and_skips_songs_with_no_artist() {
    let mut db = db();
    add_built(&mut db, "shout", Some("Sultans"), "a/SULT.kar", |song| {
        song.det_artist = Some("DIRE STRAITS".to_owned());
    });
    add_built(&mut db, "romeo", Some("Romeo"), "b/ROMEO.kar", |song| {
        song.det_artist = Some("Dire Straits".to_owned());
    });
    add_built(&mut db, "acai", Some("Açaí"), "c/ACAI.kar", |song| {
        song.det_artist = Some("Açaí Trio".to_owned());
    });
    add(&mut db, "nobody", Some("Nobody"), "d/NOBODY.kar");

    let by = |artist: &str| {
        let rows = db
            .songs(&Filter {
                artist: Some(artist.to_owned()),
                sort: Sort::Title,
                ..Filter::default()
            })
            .expect("browse");
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };

    // Two spellings of one performer are one artist.
    assert_eq!(by("Dire Straits"), vec!["romeo", "shout"]);
    assert_eq!(by("dire straits"), by("Dire Straits"), "case is folded");

    // Accents fold the way the A-Z strip folds them, so what is typed need not carry them.
    assert_eq!(by("Acai Trio"), vec!["acai"]);

    // Exact, not a substring: this is what separates it from the `title or artist` box.
    assert!(by("Dire").is_empty(), "the filter matched a prefix");
    assert!(by("Straits").is_empty(), "the filter matched a suffix");

    // A song with no artist is not by anybody, and joins no artist's list.
    assert!(by("").is_empty());
    assert!(!by("Dire Straits").contains(&"nobody".to_owned()));
}

/// A hand-typed artist wins over the detected one, because the filter reads the *effective* artist.
///
/// `sort_artist` is folded from `eff_artist`, so a correction changes which artist's list a song
/// appears in — which is the point of correcting it. Filtering on `det_artist` would leave a curator
/// fixing a name and then not finding the song under it.
#[test]
fn correcting_an_artist_moves_the_song_to_that_artists_list() {
    let mut db = db();
    add_built(&mut db, "song-a", Some("Wave"), "a/WAVE.kar", |song| {
        song.det_artist = Some("Unknown Artist".to_owned());
    });
    db.edit_song(
        "song-a",
        &SongEdit {
            artist: Some(Some("Tom Jobim".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    let by = |artist: &str| {
        db.songs(&Filter {
            artist: Some(artist.to_owned()),
            ..Filter::default()
        })
        .expect("browse")
        .len()
    };
    assert_eq!(by("Tom Jobim"), 1);
    assert_eq!(by("Unknown Artist"), 0, "the corrected name did not take");
}

/// The number box is where somebody types a number by hand, so it is where the limit is met.
#[test]
fn a_song_number_stops_at_the_last_slot_in_a_bank() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    db.create_package(
        &PackageRow {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-08-25T00:00:00Z",
    )
    .expect("create a package");
    db.add_to_package("vol1", &["song-a".to_owned()], "2026-08-25T00:00:00Z")
        .expect("select it");

    let error = db
        .set_package_number("vol1", "song-a", u32::from(km_songcode::MAX_SLOT) + 1)
        .expect_err("refused");
    assert!(
        matches!(&error, DbError::Rejected(why) if why.contains("999")),
        "{error}"
    );

    // The boundary in the other direction, so the rule cannot quietly become 999998.
    db.set_package_number("vol1", "song-a", u32::from(km_songcode::MAX_SLOT))
        .expect("the last dialable number is a number");
}

/// A member's language is the effective one, which is what the browse column reads too.
///
/// Asserted through the member rather than through a Rust helper, for the reason
/// [`the_effective_language_is_the_chosen_one_over_the_detected_one`] gives: a song listed under one
/// language in the corpus and another in the package that holds it is the confusion the column
/// removes, and the two agree only by reading one expression.
#[test]
fn a_package_member_carries_the_language_the_song_acts_on() {
    let mut db = db();
    // A header claiming English on a song somebody has since corrected, which is the corpus's
    // commonest wrong answer and the case the chosen column has to win.
    add_with_language(&mut db, "song-a", Some("ENGL"), "windows-1252");
    db.edit_song(
        "song-a",
        &SongEdit {
            language: Some(Some("pt".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("correct it");
    // Nobody has said, and the encoding is unambiguous, so detection stands.
    add_with_language(&mut db, "song-b", None, "shift_jis");
    // Neither source speaks: the package's own default language is what such a song is built under.
    add(&mut db, "song-c", Some("Wave"), "c/WAVE.kar");

    let package = PackageRow {
        id: "vol1".to_owned(),
        name: "Volume 1".to_owned(),
        version: "1.0.0".to_owned(),
        publisher: None,
        start_number: 1,
        default_language: None,
        out_path: None,
        built_at: None,
        song_count: 0,
        ..crate::model::PackageRow::new("", "")
    };
    db.create_package(&package, "2026-08-25T00:00:00Z")
        .expect("create a package");
    db.add_to_package(
        "vol1",
        &[
            "song-a".to_owned(),
            "song-b".to_owned(),
            "song-c".to_owned(),
        ],
        "2026-08-25T00:00:00Z",
    )
    .expect("select them");

    let languages: Vec<Option<String>> = db
        .package_members("vol1", 1)
        .expect("members")
        .into_iter()
        .map(|member| member.language)
        .collect();
    assert_eq!(
        languages,
        vec![Some("pt".to_owned()), Some("ja".to_owned()), None]
    );
}

#[test]
fn a_reflow_that_would_run_past_the_highest_number_is_refused_whole() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    add(&mut db, "song-b", Some("Wave"), "b/WAVE.kar");
    let package = PackageRow {
        id: "vol1".to_owned(),
        name: "Volume 1".to_owned(),
        version: "1.0.0".to_owned(),
        publisher: None,
        start_number: 1,
        default_language: None,
        out_path: None,
        built_at: None,
        song_count: 0,
        ..crate::model::PackageRow::new("", "")
    };
    db.create_package(&package, "2026-08-25T00:00:00Z")
        .expect("create a package");
    db.add_to_package(
        "vol1",
        &["song-a".to_owned(), "song-b".to_owned()],
        "2026-08-25T00:00:00Z",
    )
    .expect("select them");
    assert_eq!(db.package_members("vol1", 1).expect("members").len(), 2);

    // Two songs will not fit from the last number, and a half-done re-flow is worse than none.
    db.update_package(&PackageRow {
        start_number: u32::from(km_songcode::MAX_SLOT),
        ..package
    })
    .expect("move the start number");
    let error = db.renumber_package("vol1", 1).expect_err("refused");
    assert!(
        matches!(&error, DbError::Rejected(why) if why.contains("999")),
        "{error}"
    );
    // Nothing moved: the numbers are still the ones the selection gave them.
    let numbers: Vec<u32> = db
        .package_members("vol1", 1)
        .expect("members")
        .iter()
        .map(|member| member.number)
        .collect();
    assert_eq!(numbers, vec![1, 2]);
}

/// Selecting more songs than there are numbers adds what fits rather than numbering past the end.
#[test]
fn selecting_songs_stops_at_the_highest_number() {
    let mut db = db();
    add(&mut db, "song-a", Some("Corcovado"), "a/CORCOVAD.kar");
    add(&mut db, "song-b", Some("Wave"), "b/WAVE.kar");
    db.create_package(
        &PackageRow {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: u32::from(km_songcode::MAX_SLOT),
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-08-25T00:00:00Z",
    )
    .expect("create a package");

    let added = db
        .add_to_package(
            "vol1",
            &["song-a".to_owned(), "song-b".to_owned()],
            "2026-08-25T00:00:00Z",
        )
        .expect("select them");
    assert_eq!(added.added, 1, "only one number was left");
    // The song that did not fit is counted as one nobody could number, and never as one the package
    // already held: the two send somebody to different places, and only this one is Re-flow's.
    assert_eq!(
        (added.already, added.no_room, added.full),
        (0, 1, false),
        "a package of one song is not full; its numbers have run out"
    );
    assert_eq!(db.package_members("vol1", 1).expect("members").len(), 1);
}

/// The room a package reports is the room the write then finds.
///
/// **The two numbers have to agree or the confirmation lies.** A filter-wide add asks `package_room`
/// how many of its matches to fetch and says so on the screen; `add_to_package` recomputes the same
/// number inside its own transaction. They share `next_number`, and this is what says the sharing
/// holds across an empty package, one with members, and one whose numbers have run out.
#[test]
fn the_room_a_package_reports_is_the_room_the_write_finds() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    let package = PackageRow {
        id: "vol1".to_owned(),
        name: "Volume 1".to_owned(),
        version: "1.0.0".to_owned(),
        publisher: None,
        start_number: u32::from(km_songcode::MAX_SLOT) - 1,
        default_language: None,
        out_path: None,
        built_at: None,
        song_count: 0,
        ..crate::model::PackageRow::new("", "")
    };
    db.create_package(&package, "2026-09-12T00:00:00Z")
        .expect("create a package");

    // Empty and starting at 998: two numbers, 998 and 999.
    assert_eq!(db.package_room("vol1").expect("room"), 2);

    db.add_to_package("vol1", &["aaa".to_owned()], "2026-09-12T00:00:00Z")
        .expect("add one");
    assert_eq!(db.package_room("vol1").expect("room"), 1, "998 is spent");

    // And the write finds exactly that one number, which is the half a confirmation cannot check.
    let added = db
        .add_to_package(
            "vol1",
            &["bbb".to_owned(), "ccc".to_owned()],
            "2026-09-12T00:00:00Z",
        )
        .expect("add two more");
    assert_eq!((added.added, added.no_room), (1, 1));
    assert_eq!(
        db.package_room("vol1").expect("room"),
        0,
        "a package with nothing left says nothing left, and never one more"
    );
}

/// A song the package already holds is counted as that, rather than left for a caller to infer.
#[test]
fn adding_a_song_the_package_already_holds_counts_it_apart() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    db.create_package(
        &PackageRow {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-09-11T00:00:00Z",
    )
    .expect("create a package");
    db.add_to_package("vol1", &["aaa".to_owned()], "2026-09-11T00:00:00Z")
        .expect("the first");

    let again = db
        .add_to_package(
            "vol1",
            &["aaa".to_owned(), "bbb".to_owned()],
            "2026-09-11T00:00:00Z",
        )
        .expect("the overlap");
    assert_eq!((again.added, again.already, again.no_room), (1, 1, 0));
    assert_eq!(again.added + again.already + again.no_room, 2, "every song");
    assert_eq!(db.package_members("vol1", 1).expect("members").len(), 2);
}

// -- a package sourced from favorites -----------------------------------------------------------

/// A package to sync, taking its `start_number` and defaulting the rest.
fn sourced_package(db: &mut Db, id: &str, start_number: u32) {
    db.create_package(
        &PackageRow {
            id: id.to_owned(),
            name: id.to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-09-15T00:00:00Z",
    )
    .expect("create a package");
}

/// Points a package at several lists, which the tool itself does one gesture at a time.
///
/// Answers on the first refusal rather than at the end: a test naming a working list among four is a
/// test that meant something else.
fn source_from(db: &Db, package_id: &str, favorites: &[i64]) -> Result<(), DbError> {
    for favorite in favorites {
        assert!(
            db.set_package_source(package_id, *favorite, true)?,
            "favorite {favorite} was refused as a source"
        );
    }
    Ok(())
}

/// A song in a favorite, made and filed in one line.
fn filed(db: &mut Db, id: &str, favorite: i64) {
    add(db, id, Some(id), &format!("f/{id}.kar"));
    db.set_favorite(id, favorite, true).expect("file it");
}

/// What a package holds, as number-and-song pairs in number order.
fn numbering(db: &Db, package_id: &str) -> Vec<(u32, String)> {
    db.package_members(package_id, 1)
        .expect("members")
        .into_iter()
        .map(|member| (member.number, member.song_id))
        .collect()
}

/// A sourced package holds the union of its lists, and a song in two of them arrives once.
#[test]
fn a_sourced_package_holds_the_union_of_its_lists() {
    let mut db = db();
    let axe = db.create_favorite("Brasil Axé").expect("a list");
    let samba = db.create_favorite("Brasil Samba").expect("a second list");
    filed(&mut db, "aaa", axe);
    filed(&mut db, "bbb", samba);
    // In both, which is the case `DISTINCT` is there for: a curator has said two things about this
    // song and asked for one entry.
    filed(&mut db, "ccc", axe);
    db.set_favorite("ccc", samba, true).expect("file it twice");

    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[axe, samba]).expect("point it at both");
    assert!(db.is_sourced("vol1").expect("sourced"));

    let synced = db
        .sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("sync");
    assert_eq!(
        (synced.placed.added, synced.removed, synced.kept),
        (3, 0, 0)
    );
    let held: Vec<String> = numbering(&db, "vol1")
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    assert_eq!(held, ["aaa", "bbb", "ccc"], "each song once, by title");
}

/// A song somebody threw away does not reach a machine, by either route into a package.
///
/// **The two routes are the hand-added member and the starred source, and they fail apart.** A
/// member sits in `package_songs` and is read by `package_members`, which is what `build::spec_for`
/// writes the `.kmpkg` from. A star sits in `song_favorites`, which nothing clears on a delete, so
/// `WANTED_SQL` would count a discarded song as kept and a sync would put it back after somebody
/// took it out by hand. `Throwing a song away` in `docs/decisions/curation.md` promises neither
/// happens.
#[test]
fn a_song_thrown_away_leaves_the_package_it_was_in() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    for id in ["aaa", "bbb"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("sync");
    assert_eq!(numbering(&db, "vol1").len(), 2);

    assert_eq!(
        db.set_deleted_of(&["aaa".to_owned()], true)
            .expect("delete"),
        1
    );

    // The build reads this, so a member still listed here is a member that ships.
    let held: Vec<String> = numbering(&db, "vol1")
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    assert_eq!(held, ["bbb"], "the discarded song is not in the volume");

    // And the star it kept does not count as a song the package still wants.
    let plan = db.package_sync_plan("vol1").expect("plan");
    assert_eq!(
        (plan.kept, plan.would_add, plan.would_remove),
        (1, 0, 1),
        "the row it left behind is counted out rather than kept"
    );

    // Bringing it back puts it in both again, which is what makes a delete undoable.
    assert_eq!(
        db.set_deleted_of(&["aaa".to_owned()], false)
            .expect("undelete"),
        1
    );
    let plan = db.package_sync_plan("vol1").expect("plan");
    assert_eq!((plan.kept, plan.would_add, plan.would_remove), (2, 0, 0));
}

/// A song no source names any more leaves, and the songs that stay keep their numbers.
///
/// **The load-bearing one.** Keeping a number across a sync is the promise the whole arrangement
/// rests on: a songbook printed from a package stays true for every song that is still in it.
#[test]
fn a_synced_song_keeps_the_number_it_had() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    for id in ["aaa", "bbb", "ccc", "ddd"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    let before = numbering(&db, "vol1");
    assert_eq!(before.len(), 4);

    // Two go out of the list, one comes in.
    db.set_favorite("bbb", list, false).expect("unfile");
    db.set_favorite("ddd", list, false).expect("unfile");
    filed(&mut db, "eee", list);

    let synced = db
        .sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("sync");
    assert_eq!(
        (synced.placed.added, synced.removed, synced.kept),
        (1, 2, 2)
    );
    let after = numbering(&db, "vol1");
    for (number, song_id) in &before {
        if song_id == "bbb" || song_id == "ddd" {
            continue;
        }
        assert!(
            after.contains(&(*number, song_id.clone())),
            "{song_id} moved from {number}: {after:?}"
        );
    }
}

/// The numbers a sync holds in one volume, as number-and-song pairs in number order.
fn held(db: &Db, package_id: &str, volume: u32) -> Vec<(u32, Option<String>)> {
    db.package_held(package_id, volume)
        .expect("held")
        .into_iter()
        .map(|hold| (hold.number, hold.song_id))
        .collect()
}

/// A song a sync takes out leaves its number held, and a newcomer takes the number after it.
///
/// **A printed songbook still lists the song at that number**, so handing it to another song would
/// send a singer who dials it to the wrong music.
#[test]
fn a_removed_song_holds_its_number() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb", "ccc", "ddd"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");

    db.set_favorite("bbb", list, false).expect("unfile");
    filed(&mut db, "eee", list);
    let synced = db
        .sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");
    assert_eq!((synced.placed.added, synced.removed), (1, 1));
    assert_eq!(
        numbering(&db, "vol1"),
        [
            (1, "aaa".to_owned()),
            (3, "ccc".to_owned()),
            (4, "ddd".to_owned()),
            (5, "eee".to_owned())
        ],
        "the newcomer took the number after the last, not the held one"
    );
    assert_eq!(held(&db, "vol1", 1), [(2, Some("bbb".to_owned()))]);
    let hold = &db.package_held("vol1", 1).expect("held")[0];
    assert_eq!(hold.title, "bbb", "the hold keeps the title it left with");
}

/// A song that comes back to the lists takes back the number held for it.
#[test]
fn a_returning_song_takes_back_its_held_number() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("bbb", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");

    db.set_favorite("bbb", list, true).expect("file it again");
    filed(&mut db, "ddd", list);
    let plan = db.package_sync_plan("vol1").expect("plan");
    assert_eq!((plan.would_add, plan.would_return), (2, 1));
    let synced = db
        .sync_package("vol1", "2026-09-15T02:00:00Z")
        .expect("third sync");
    assert_eq!((synced.placed.added, synced.placed.returned), (2, 1));
    assert_eq!(
        numbering(&db, "vol1"),
        [
            (1, "aaa".to_owned()),
            (2, "bbb".to_owned()),
            (3, "ccc".to_owned()),
            (4, "ddd".to_owned())
        ]
    );
    assert!(held(&db, "vol1", 1).is_empty(), "the hold is spent");
}

/// A released number is an ordinary free number, and the next song takes it.
#[test]
fn a_released_number_goes_to_the_next_song() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("bbb", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");

    assert!(
        db.release_held("vol1", 1, 3).is_err(),
        "a number nothing holds is refused"
    );
    db.release_held("vol1", 1, 2).expect("release");
    filed(&mut db, "ddd", list);
    db.sync_package("vol1", "2026-09-15T02:00:00Z")
        .expect("third sync");
    assert_eq!(
        numbering(&db, "vol1"),
        [
            (1, "aaa".to_owned()),
            (2, "ddd".to_owned()),
            (3, "ccc".to_owned())
        ]
    );
}

/// A person fills a held number by moving a song of the package into it, from any volume.
#[test]
fn a_held_number_is_filled_from_another_volume() {
    let mut db = db();
    let list = db.create_favorite("Everything").expect("a list");
    sourced_package(&mut db, "vol1", u32::from(km_songcode::MAX_SLOT) - 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("aaa", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");
    assert_eq!(held(&db, "vol1", 1), [(998, Some("aaa".to_owned()))]);

    assert!(
        db.fill_held("vol1", 1, 999, "ccc").is_err(),
        "a number nothing holds is refused"
    );
    add(&mut db, "zzz", Some("zzz"), "f/zzz.kar");
    assert!(
        db.fill_held("vol1", 1, 998, "zzz").is_err(),
        "a song outside the package is refused"
    );

    assert_eq!(
        db.member_at("vol1", 2, 1).expect("read"),
        Some("ccc".to_owned())
    );
    db.fill_held("vol1", 1, 998, "ccc").expect("fill");
    assert_eq!(
        volume_numbering(&db, "vol1", 1),
        [(998, "ccc".to_owned()), (999, "bbb".to_owned())]
    );
    assert!(volume_numbering(&db, "vol1", 2).is_empty());
    assert!(held(&db, "vol1", 1).is_empty(), "the hold is spent");
    assert!(
        held(&db, "vol1", 2).is_empty(),
        "the number the song left is free, not held: only a sync holds"
    );
}

/// Typing a held number into a member's box fills the hold.
#[test]
fn a_member_numbered_onto_a_hold_fills_it() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("bbb", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");

    db.set_package_number("vol1", "ccc", 2).expect("renumber");
    assert_eq!(
        numbering(&db, "vol1"),
        [(1, "aaa".to_owned()), (2, "ccc".to_owned())]
    );
    assert!(held(&db, "vol1", 1).is_empty());
}

/// A re-flow keeps every hold and flows the songs around them.
#[test]
fn a_reflow_flows_around_held_numbers() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb", "ccc", "ddd"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("bbb", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");
    db.set_package_number("vol1", "ddd", 9).expect("a gap");

    db.renumber_package("vol1", 1).expect("re-flow");
    assert_eq!(
        numbering(&db, "vol1"),
        [
            (1, "aaa".to_owned()),
            (3, "ccc".to_owned()),
            (4, "ddd".to_owned())
        ]
    );
    assert_eq!(held(&db, "vol1", 1), [(2, Some("bbb".to_owned()))]);
}

/// A replacement at a held number is refused, and names the song held for.
#[test]
fn a_replacement_at_a_held_number_is_refused() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb"] {
        filed(&mut db, id, list);
    }
    add(&mut db, "ccc", Some("ccc"), "f/ccc.kar");
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("bbb", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");

    assert_eq!(
        db.replacement_at("vol1", 1, 2, "ccc")
            .expect("read")
            .expect_err("refused"),
        ReplaceRefusal::Held("vol1".to_owned(), "bbb".to_owned())
    );
}

/// A held number offers the package's other files of the same recording.
#[test]
fn a_held_number_offers_other_files_of_its_recording() {
    let mut db = db();
    let list = db.create_favorite("TODO Rock").expect("a list");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    db.execute_for_test("UPDATE songs SET duplicate_of = 'bbb' WHERE id = 'ccc'")
        .expect("group");
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");
    db.set_favorite("bbb", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");

    let holds = db.package_held("vol1", 1).expect("held");
    assert_eq!(holds.len(), 1);
    assert_eq!(holds[0].candidates, [(1, 3, "ccc".to_owned())]);
}

/// What one volume of a package holds, as number-and-song pairs in number order.
fn volume_numbering(db: &Db, package_id: &str, volume: u32) -> Vec<(u32, String)> {
    db.package_members(package_id, volume)
        .expect("members")
        .into_iter()
        .map(|member| (member.number, member.song_id))
        .collect()
}

/// A union bigger than every volume starts another volume, under an id of its own, and leaves
/// nothing out.
#[test]
fn a_source_larger_than_a_volume_starts_another() {
    let mut db = db();
    let list = db.create_favorite("Everything").expect("a list");
    sourced_package(&mut db, "vol1", u32::from(km_songcode::MAX_SLOT) - 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    // Three songs into a volume with two numbers left.
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }

    let plan = db.package_sync_plan("vol1").expect("plan");
    assert_eq!((plan.would_add, plan.new_volumes), (3, 1));

    let synced = db
        .sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("sync");
    assert_eq!(
        (
            synced.placed.added,
            synced.placed.no_room,
            synced.new_volumes
        ),
        (3, 0, 1),
        "the sync the plan described"
    );
    assert_eq!(
        volume_numbering(&db, "vol1", 1),
        [(998, "aaa".to_owned()), (999, "bbb".to_owned())]
    );
    assert_eq!(volume_numbering(&db, "vol1", 2), [(1, "ccc".to_owned())]);

    let volumes = db.package_volumes("vol1").expect("volumes");
    assert_eq!(volumes.len(), 2);
    assert_eq!(
        volumes[0].volume_id, "vol1",
        "the first volume keeps the package's id"
    );
    assert!(
        km_kmpkg::PackageMeta::is_generated_id(&volumes[1].volume_id),
        "a later volume is banked by an id of its own: {}",
        volumes[1].volume_id
    );
    assert_eq!(volumes[1].volume_name(), "vol1 vol2");
    assert_eq!(volumes[0].volume_name(), "vol1 vol1");
    assert_eq!(volumes[0].total_songs, 3);
}

/// A song stays in the volume that numbered it, and a newcomer takes a hole in the first volume
/// before the last one.
#[test]
fn a_synced_song_keeps_its_volume_and_holes_fill_from_the_first() {
    let mut db = db();
    let list = db.create_favorite("Everything").expect("a list");
    sourced_package(&mut db, "vol1", u32::from(km_songcode::MAX_SLOT) - 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("first sync");

    // A hole in the first volume, and a newcomer to fill it. The sync holds the number, so the
    // hole exists once somebody releases it.
    db.set_favorite("aaa", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T00:30:00Z")
        .expect("the sync that holds the number");
    db.release_held("vol1", 1, 998).expect("release");
    filed(&mut db, "ddd", list);
    let synced = db
        .sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("second sync");
    assert_eq!(synced.new_volumes, 0);
    assert_eq!(
        volume_numbering(&db, "vol1", 1),
        [(998, "ddd".to_owned()), (999, "bbb".to_owned())]
    );
    assert_eq!(
        volume_numbering(&db, "vol1", 2),
        [(1, "ccc".to_owned())],
        "a song in the second volume did not move to the hole"
    );

    // Emptying the second volume keeps it: its id is what a machine banked.
    db.set_favorite("ccc", list, false).expect("unfile");
    db.sync_package("vol1", "2026-09-15T02:00:00Z")
        .expect("third sync");
    let volumes = db.package_volumes("vol1").expect("volumes");
    assert_eq!(volumes.len(), 2, "an emptied volume stays");
    assert_eq!(volumes[1].song_count, 0);
}

/// A package's volume name writes the number where `{n}` is, and the default is the number alone.
#[test]
fn a_volume_name_follows_the_package_format() {
    let mut db = db();
    sourced_package(&mut db, "vol1", 1);
    db.ensure_volume(
        "vol1",
        2,
        &km_kmpkg::PackageMeta::new_id(),
        "2026-09-16T00:00:00Z",
    )
    .expect("a second volume");
    let named = |db: &Db| {
        db.package_volumes("vol1")
            .expect("volumes")
            .iter()
            .map(PackageRow::volume_name)
            .collect::<Vec<_>>()
    };
    assert_eq!(named(&db), ["vol1 vol1", "vol1 vol2"]);

    db.update_package_details("vol1", "Brasil", None, None, "vol{n}", false)
        .expect("a format");
    assert_eq!(named(&db), ["Brasil vol1", "Brasil vol2"]);
}

/// A package of one volume carries its bare name, and is numbered from the first when asked.
///
/// **The box is for a set that will outgrow 999 songs**, whose first file would otherwise be renamed
/// when the second volume starts.
#[test]
fn a_package_of_one_volume_is_numbered_when_asked() {
    let mut db = db();
    sourced_package(&mut db, "vol1", 1);
    let named = |db: &Db| {
        db.package_volume("vol1", 1)
            .expect("the volume")
            .volume_name()
    };
    db.update_package_details("vol1", "Brasil", None, None, "vol{n}", false)
        .expect("unticked");
    assert_eq!(named(&db), "Brasil");

    db.update_package_details("vol1", "Brasil", None, None, "vol{n}", true)
        .expect("ticked");
    assert_eq!(named(&db), "Brasil vol1");
    assert!(
        db.package_volume("vol1", 1)
            .expect("the volume")
            .number_one_volume
    );
}

/// The first number and the version are two forms on two tabs, and each leaves the other alone.
#[test]
fn a_volume_update_leaves_what_its_form_did_not_send() {
    let mut db = db();
    sourced_package(&mut db, "vol1", 5);
    db.update_volume("vol1", 1, Some("2.0.0"), None)
        .expect("the version alone");
    let volume = db.package_volume("vol1", 1).expect("the volume");
    assert_eq!((volume.version.as_str(), volume.start_number), ("2.0.0", 5));

    db.update_volume("vol1", 1, None, Some(40))
        .expect("the first number alone");
    let volume = db.package_volume("vol1", 1).expect("the volume");
    assert_eq!(
        (volume.version.as_str(), volume.start_number),
        ("2.0.0", 40)
    );
}

/// A hand add appends to the last volume and never starts one.
#[test]
fn a_hand_add_never_starts_a_volume() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    sourced_package(&mut db, "vol1", u32::from(km_songcode::MAX_SLOT) - 1);
    let added = db
        .add_to_package(
            "vol1",
            &["aaa".to_owned(), "bbb".to_owned(), "ccc".to_owned()],
            "2026-09-15T00:00:00Z",
        )
        .expect("add by hand");
    assert_eq!((added.added, added.no_room), (2, 1));
    assert_eq!(db.package_volumes("vol1").expect("volumes").len(), 1);
}

/// Pressing Sync on a package nobody has given a list to is refused, not obeyed.
#[test]
fn a_sync_with_no_sources_is_refused_rather_than_emptying_the_package() {
    let mut db = db();
    add(&mut db, "aaa", Some("aaa"), "f/aaa.kar");
    sourced_package(&mut db, "vol1", 1);
    db.add_to_package("vol1", &["aaa".to_owned()], "2026-09-15T00:00:00Z")
        .expect("add by hand");

    let error = db
        .sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect_err("refused");
    assert!(matches!(&error, DbError::Rejected(_)), "{error}");
    assert_eq!(
        numbering(&db, "vol1").len(),
        1,
        "the refusal wrote nothing at all"
    );
}

/// Deleting a source takes it out of the package's sources and leaves the songs until the next sync.
///
/// The cascade's answer, and the reason the Favorites page's Delete names the packages first: what
/// goes here is silent, and what it costs arrives the next time somebody presses Sync.
#[test]
fn deleting_a_source_favorite_leaves_the_package_alone_until_it_is_synced() {
    let mut db = db();
    let going = db.create_favorite("to check").expect("a list");
    let staying = db.create_favorite("Bossa nova").expect("a second list");
    filed(&mut db, "aaa", going);
    filed(&mut db, "bbb", staying);
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[going, staying]).expect("source it");
    db.sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("sync");
    assert_eq!(numbering(&db, "vol1").len(), 2);

    db.delete_favorite(going).expect("delete the list");
    assert_eq!(
        db.package_sources("vol1").expect("sources").len(),
        1,
        "the source went with the list"
    );
    assert_eq!(
        numbering(&db, "vol1").len(),
        2,
        "and the songs stayed where they were"
    );

    let synced = db
        .sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("sync");
    assert_eq!((synced.removed, synced.kept), (1, 1));
}

/// A song merged into another syncs as the song it turned out to be.
#[test]
fn a_merged_song_syncs_as_the_song_it_turned_out_to_be() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    filed(&mut db, "aaa", list);
    filed(&mut db, "bbb", list);
    // Both were starred before anybody noticed they are one recording. `DISTINCT` over the survivor
    // is what keeps the union from naming it twice.
    db.set_merged_into("bbb", Some("aaa")).expect("merge");

    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    let synced = db
        .sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("sync");
    assert_eq!(synced.placed.added, 1);
    assert_eq!(
        numbering(&db, "vol1"),
        [(1, "aaa".to_owned())],
        "the survivor, once"
    );
}

/// The songs one favorite holds, by id.
fn in_favorite(db: &Db, favorite: i64) -> Vec<String> {
    let mut ids: Vec<String> = db
        .songs(&Filter {
            favorite: Some(favorite),
            ..Filter::default()
        })
        .expect("browse")
        .into_iter()
        .map(|row| row.id)
        .collect();
    ids.sort();
    ids
}

/// A replacement keeps the number, and the file the package reads becomes the new song's.
#[test]
fn a_replaced_song_keeps_the_number() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    sourced_package(&mut db, "vol1", 1);
    db.add_to_package("vol1", &["aaa".to_owned(), "bbb".to_owned()], "t")
        .expect("add");

    let asked = db
        .replacement_at("vol1", 1, 1, "ccc")
        .expect("read")
        .expect("allowed");
    assert_eq!(asked.old.0, "aaa");
    assert!(asked.favorites.is_empty(), "a package that follows no list");
    assert_eq!(numbering(&db, "vol1")[0].1, "aaa", "asking writes nothing");

    let done = db
        .replace_in_package("vol1", 1, 1, "ccc", "t")
        .expect("write")
        .expect("allowed");
    assert_eq!(done, asked, "the question and the write read one plan");
    assert_eq!(
        numbering(&db, "vol1"),
        [(1, "ccc".to_owned()), (2, "bbb".to_owned())]
    );
    let members = db.package_members("vol1", 1).expect("members");
    assert_eq!(members[0].path.as_deref(), Some("f/ccc.kar"));
}

/// Each refusal is its own value, and none of them writes.
#[test]
fn a_replacement_is_refused_by_reason() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc", "ddd"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    db.set_merged_into("ddd", Some("ccc")).expect("merge");
    sourced_package(&mut db, "vol1", 1);
    db.add_to_package("vol1", &["aaa".to_owned(), "bbb".to_owned()], "t")
        .expect("add");

    let refused = |db: &mut Db, number, song| {
        db.replace_in_package("vol1", 1, number, song, "t")
            .expect("write")
            .expect_err("refused")
    };
    assert_eq!(
        refused(&mut db, 9, "ccc"),
        ReplaceRefusal::Empty("vol1".to_owned())
    );
    assert_eq!(refused(&mut db, 1, "aaa"), ReplaceRefusal::SameSong);
    assert_eq!(
        refused(&mut db, 1, "bbb"),
        ReplaceRefusal::AlreadyIn("vol1".to_owned(), 2)
    );
    assert_eq!(refused(&mut db, 1, "ddd"), ReplaceRefusal::Merged);
    // Beside the merge, and for a harder reason: a merged song has a survivor standing in its
    // place, where a discarded one has nobody. `replace_in_package` stars the substitute into the
    // sourcing favorites, so this is the route by which a thrown-away song would reach a build.
    db.set_deleted_of(&["ccc".to_owned()], true)
        .expect("delete");
    assert_eq!(refused(&mut db, 1, "ccc"), ReplaceRefusal::Deleted);
    assert_eq!(
        numbering(&db, "vol1"),
        [(1, "aaa".to_owned()), (2, "bbb".to_owned())]
    );
}

/// In a package that follows lists, the lists change with it, so the next sync moves nothing.
///
/// **The one the arrangement rests on.** Changing only the list would have the sync take the old song
/// out and give the new one the lowest free number, which is the old number only by chance.
#[test]
fn a_replacement_in_a_sourced_package_changes_its_lists_and_the_sync_keeps_it() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    let other = db
        .create_favorite("Parties")
        .expect("a list no package follows");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    // A second file of `bbb` that was starred before the merge: the list names it, and the sync
    // reads it as `bbb`, so the replacement has to take it out too.
    filed(&mut db, "bb2", list);
    db.set_merged_into("bb2", Some("bbb")).expect("merge");
    db.set_favorite("bbb", other, true)
        .expect("file it elsewhere");
    add(&mut db, "new", Some("new"), "f/new.kar");

    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    db.sync_package("vol1", "t").expect("sync");
    assert_eq!(numbering(&db, "vol1")[1], (2, "bbb".to_owned()));

    let done = db
        .replace_in_package("vol1", 1, 2, "new", "t")
        .expect("write")
        .expect("allowed");
    assert_eq!(done.favorites, ["Bossa nova"]);
    assert_eq!(in_favorite(&db, list), ["aaa", "ccc", "new"]);
    assert_eq!(
        in_favorite(&db, other),
        ["bbb"],
        "a list no package follows"
    );

    let plan = db.package_sync_plan("vol1").expect("plan");
    assert!(plan.is_quiet(), "nothing left for a sync to do: {plan:?}");
    db.sync_package("vol1", "t").expect("sync");
    assert_eq!(numbering(&db, "vol1")[1], (2, "new".to_owned()));
}

/// What a sync says it would do is what it then does.
///
/// **The one that keeps the confirmation honest.** The plan and the write read the same statement,
/// and this is what says the sharing holds — over an empty package, one that agrees with its lists,
/// and one that has both something to add and something to take out.
#[test]
fn the_plan_agrees_with_what_the_sync_then_does() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    for id in ["aaa", "bbb", "ccc"] {
        filed(&mut db, id, list);
    }
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");

    let plan = db.package_sync_plan("vol1").expect("plan");
    assert_eq!((plan.would_add, plan.would_remove, plan.kept), (3, 0, 0));
    assert_eq!(plan.sources.len(), 1);
    let synced = db
        .sync_package("vol1", "2026-09-15T00:00:00Z")
        .expect("sync");
    assert_eq!(
        (synced.placed.added, synced.removed, synced.kept),
        (plan.would_add, plan.would_remove, plan.kept)
    );

    // Nothing to do, which the page says rather than asks about.
    let quiet = db.package_sync_plan("vol1").expect("plan");
    assert!(quiet.is_quiet(), "{quiet:?}");
    assert_eq!(quiet.kept, 3);

    db.set_favorite("bbb", list, false).expect("unfile");
    filed(&mut db, "ddd", list);
    let plan = db.package_sync_plan("vol1").expect("plan");
    assert_eq!((plan.would_add, plan.would_remove, plan.kept), (1, 1, 2));
    let synced = db
        .sync_package("vol1", "2026-09-15T01:00:00Z")
        .expect("sync");
    assert_eq!(
        (synced.placed.added, synced.removed, synced.kept),
        (plan.would_add, plan.would_remove, plan.kept)
    );
}

/// A sourced package is not offered where songs are added to a package one at a time.
#[test]
fn a_sourced_package_is_not_offered_to_add_to() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    sourced_package(&mut db, "vol1", 1);
    sourced_package(&mut db, "vol2", 1);
    source_from(&db, "vol1", &[list]).expect("source it");

    let offered: Vec<String> = db
        .packages_taking_songs()
        .expect("packages")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(offered, ["vol2"], "the sourced one is not an answer");
    assert_eq!(
        db.packages().expect("packages").len(),
        2,
        "and the Packages page still lists both"
    );

    // Clearing the last source makes it an ordinary package again.
    db.set_package_source("vol1", list, false)
        .expect("take the last one out");
    assert!(!db.is_sourced("vol1").expect("sourced"));
    assert_eq!(db.packages_taking_songs().expect("packages").len(), 2);
}

/// Saving the sources writes the sources, and nothing else at all.
#[test]
fn saving_the_sources_syncs_nothing() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    filed(&mut db, "aaa", list);
    sourced_package(&mut db, "vol1", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    assert!(
        numbering(&db, "vol1").is_empty(),
        "pressing Save is not pressing Sync"
    );
}

/// Both pages that name a source read one query, and it answers in both directions.
#[test]
fn one_query_says_which_lists_feed_which_packages() {
    let mut db = db();
    let list = db.create_favorite("Bossa nova").expect("a list");
    sourced_package(&mut db, "vol1", 1);
    sourced_package(&mut db, "vol2", 1);
    source_from(&db, "vol1", &[list]).expect("source it");
    source_from(&db, "vol2", &[list]).expect("source it");

    let links = db.package_sources_all().expect("links");
    assert_eq!(links.len(), 2);
    assert!(links.iter().all(|link| link.favorite_id == list));
    assert!(links.iter().all(|link| link.favorite_name == "Bossa nova"));
    let packages: Vec<&str> = links.iter().map(|link| link.package_id.as_str()).collect();
    assert_eq!(packages, ["vol1", "vol2"]);
}

/// A working list is no package's source, and the statement is what refuses one.
///
/// **In the write rather than in a read beforehand**, which is [`MERGE_SQL`]'s discipline: the flag
/// is set on another page, so a check that passed a moment earlier is a check the answer can change
/// under. What a caller gets back is whether the package draws on the list, so *already a source*
/// and *refused* are not the same number.
#[test]
fn a_working_list_is_refused_as_a_source() {
    let mut db = db();
    let setting_aside = db.create_favorite("to check").expect("a list");
    let filing = db.create_favorite("Bossa nova").expect("a second list");
    db.set_favorite_temporary(setting_aside, true)
        .expect("set it aside");
    sourced_package(&mut db, "vol1", 1);

    assert!(
        !db.set_package_source("vol1", setting_aside, true)
            .expect("answered"),
        "a list meaning decide-about-these-later is not a volume to build"
    );
    assert!(db.package_sources("vol1").expect("sources").is_empty());

    // The filing beside it is taken, and answering again says the package already draws on it
    // rather than repeating the refusal's answer.
    assert!(db.set_package_source("vol1", filing, true).expect("added"));
    assert!(
        db.set_package_source("vol1", filing, true).expect("again"),
        "already a source is not refused"
    );
    assert_eq!(db.package_sources("vol1").expect("sources").len(), 1);

    // A source that becomes a working list stays one, because only its own row can take it away.
    db.set_favorite_temporary(filing, true)
        .expect("set it aside");
    assert_eq!(db.package_sources("vol1").expect("sources").len(), 1);
    assert!(
        !db.set_package_source("vol1", filing, false)
            .expect("removed"),
        "and taking it out answers that the package no longer draws on it"
    );
}

/// A filter is *only this favorite* when the list is the whole of what it narrows by.
///
/// **The offer to keep a new package sourced rests entirely on this.** A filter of one list plus a
/// language makes a package that list alone would not, so the box is absent and the package is an
/// ordinary one; getting that backwards sources a package from a list whose next sync would pour
/// every other song in it into the package.
#[test]
fn a_filter_is_only_a_favorite_when_nothing_else_narrows_it() {
    let list = Filter {
        favorite: Some(7),
        ..Filter::default()
    };
    assert_eq!(list.only_this_favorite(), Some(7));

    // The page, the order and how many rows are on it are not narrowing, so they travel across.
    assert_eq!(
        Filter {
            sort: Sort::Artist,
            limit: 25,
            offset: 300,
            ..list.clone()
        }
        .only_this_favorite(),
        Some(7)
    );

    // Anything that does narrow takes the offer away.
    for narrowed in [
        Filter {
            language: LanguageFilter::Unset,
            ..list.clone()
        },
        Filter {
            query: Some("bossa".to_owned()),
            ..list.clone()
        },
        Filter {
            unpackaged: true,
            ..list.clone()
        },
        Filter {
            tags: vec!["rock".to_owned()],
            ..list.clone()
        },
        Filter {
            artist: Some("Tom Jobim".to_owned()),
            ..list.clone()
        },
        Filter {
            favorited: FavoritedFilter::Filed,
            ..list.clone()
        },
    ] {
        assert_eq!(
            narrowed.only_this_favorite(),
            None,
            "a package of this filter is not that list: {narrowed:?}"
        );
    }

    // And a filter naming no list has none to offer.
    assert_eq!(Filter::default().only_this_favorite(), None);
}

/// A package holding every number it can says so, because Re-flow is no answer to it.
#[test]
fn a_package_with_every_number_used_says_it_is_full() {
    let mut db = db();
    db.create_package(
        &PackageRow {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-09-11T00:00:00Z",
    )
    .expect("create a package");
    // Filled through the membership rather than through `add`, because what is asserted is the
    // count of numbers in use and a thousand scans would buy nothing towards it.
    for number in 1..=u32::from(km_songcode::MAX_SLOT) {
        let id = format!("song-{number}");
        add(&mut db, &id, Some(&id), &format!("f/{id}.kar"));
        db.add_to_package("vol1", &[id], "2026-09-11T00:00:00Z")
            .expect("fill it");
    }
    add(&mut db, "one-more", Some("One More"), "f/one-more.kar");

    let over = db
        .add_to_package("vol1", &["one-more".to_owned()], "2026-09-11T00:00:00Z")
        .expect("the one over");
    assert_eq!((over.added, over.already, over.no_room), (0, 0, 1));
    assert!(
        over.full,
        "every number is in use, so there is nothing to re-flow"
    );
}

/// A title of nothing but padding is swept away, and the song browses under its own file name.
///
/// The corpus shape this reproduces: MIDI text events are fixed-length fields in a lot of
/// software, so track names arrive NUL-padded, and `str::trim` -- which is what km-song used to
/// gate on -- does not remove a NUL. **A NUL sorts before every printable character**, so the 179
/// such rows in the real corpus took the whole first page of a title-ordered browse and drew
/// themselves as empty links. km-song rejects them now; this is the repair for rows already
/// written, which no re-scan would reach because no file has changed.
#[test]
fn a_title_of_control_characters_is_swept_and_the_song_browses_under_its_file_name() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(
        &mut db,
        "padded",
        Some("\u{0}\u{0}\u{0}"),
        "folder/Real Name.kar",
    );
    add(
        &mut db,
        "trailing",
        Some("Otonosuke\u{0}\u{0}"),
        "folder/other.kar",
    );
    add(
        &mut db,
        "ordinary",
        Some("Águas de Março"),
        "folder/aguas.kar",
    );

    // Before the sweep the padded row sorts to the top with nothing in it.
    let before = db.songs(&Filter::default()).expect("browse");
    assert_eq!(before[0].title, "\u{0}\u{0}\u{0}");

    db.clean_detected_text_for_test().expect("sweep");

    let after = db.songs(&Filter::default()).expect("browse");
    let titles: Vec<&str> = after.iter().map(|row| row.title.as_str()).collect();
    // The padded one falls through to its file name, the trailing padding is trimmed off a real
    // title, and a title that was always fine is untouched.
    assert!(titles.contains(&"Real Name"), "got {titles:?}");
    assert!(titles.contains(&"Otonosuke"), "got {titles:?}");
    assert!(titles.contains(&"Águas de Março"), "got {titles:?}");
    assert!(
        !titles.iter().any(|title| title.contains('\u{0}')),
        "no title keeps a control character; got {titles:?}"
    );
    // And it is a *file name* title now, so the row wears the tag that says so.
    let padded = after
        .iter()
        .find(|row| row.title == "Real Name")
        .expect("the padded song");
    assert!(padded.from_filename);
}

/// A title of nothing but marks is swept away, and the song browses under its own file name.
///
/// The second defect of the shape the sweep above answers, and the one a curator sees: a corpus
/// writes `====================` and `<>-<>-<>-<>` into a title meta event where the person typing
/// it had no title, and those rows fill the first page of a title-ordered browse. The gate refuses
/// them now; this is the repair for the rows already written, which no re-scan reaches.
#[test]
fn a_title_of_marks_is_swept_and_the_song_browses_under_its_file_name() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(
        &mut db,
        "rule",
        Some("===================="),
        "f/Real Name.kar",
    );
    add(&mut db, "arrows", Some("<>-<>-<>-<>"), "f/Another Name.kar");
    add(
        &mut db,
        "stray",
        Some("====== X ======"),
        "f/Third Name.kar",
    );
    add(&mut db, "ordinary", Some("Águas de Março"), "f/aguas.kar");
    add(&mut db, "initials", Some("R.E.M."), "f/rem.kar");
    add(
        &mut db,
        "framed",
        Some("----- TAKE FIVE -----"),
        "f/five.kar",
    );

    // A database at revision 1: swept of control characters, and so due this sweep.
    db.set_setting(CLEANED_META, "1").expect("the old sweep");
    // Before the sweep the marks are what the rows are called, and the two that are nothing else
    // fold to an empty browse key, so they take the front of the page ahead of every real name.
    // The one with a letter in it sorts under that letter, which is the quieter half of the same
    // fault: it is not at the front, and it is still not a name.
    let before = db.songs(&Filter::default()).expect("browse");
    let front: Vec<&str> = before
        .iter()
        .take(2)
        .map(|row| row.title.as_str())
        .collect();
    assert!(front.contains(&"===================="), "got {front:?}");
    assert!(front.contains(&"<>-<>-<>-<>"), "got {front:?}");

    db.clean_detected_text().expect("sweep");

    let after = db.songs(&Filter::default()).expect("browse");
    let titles: Vec<&str> = after.iter().map(|row| row.title.as_str()).collect();
    // The three that named nothing fall through to their file names. A name that was always fine is
    // untouched, so is one written in initials, and so is the one inside a frame -- what the marks
    // are around is what decides it.
    assert!(titles.contains(&"Real Name"), "got {titles:?}");
    assert!(titles.contains(&"Another Name"), "got {titles:?}");
    assert!(titles.contains(&"Third Name"), "got {titles:?}");
    assert!(titles.contains(&"Águas de Março"), "got {titles:?}");
    assert!(titles.contains(&"R.E.M."), "got {titles:?}");
    assert!(titles.contains(&"----- TAKE FIVE -----"), "got {titles:?}");
    // And each is a *file name* title now, so the row wears the tag that says so.
    let swept = after
        .iter()
        .find(|row| row.title == "Real Name")
        .expect("the ornament song");
    assert!(swept.from_filename);
}

/// A database swept by the older spelling is swept once more, and then left alone.
///
/// The flag this began as could not express that: a boolean says *swept*, and what has to be
/// answerable is *swept by which gate*.
#[test]
fn a_database_swept_by_an_older_gate_is_swept_again() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "arrows", Some("<>-<>-<>-<>"), "f/Real Name.kar");
    db.set_setting(CLEANED_META, "1").expect("the old sweep");

    db.clean_detected_text().expect("sweep");
    assert_eq!(
        db.setting(CLEANED_META).expect("setting").as_deref(),
        Some(CLEANED_META_REVISION.to_string().as_str())
    );
    let titles: Vec<String> = db
        .songs(&Filter::default())
        .expect("browse")
        .into_iter()
        .map(|row| row.title)
        .collect();
    assert_eq!(titles, vec!["Real Name".to_owned()]);
}

/// Closing reports the journal it folded back in, which is the number the exit pause tracks.
///
/// An in-memory database has no write-ahead log to checkpoint, so this pins the shape of the
/// answer rather than a size: the call succeeds, says zero, and leaves the connection usable.
/// What it is really guarding is that `PRAGMA wal_checkpoint` is read from column 1 -- the pragma
/// returns `(busy, log, checkpointed)`, and reading column 0 would report *busy* as a page count
/// and print a closing line that is always zero.
#[test]
fn closing_reports_the_journal_it_wrote_back() {
    let db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    assert_eq!(db.close(), 0);
    // Still answers afterwards: closing is a checkpoint, not a teardown.
    assert_eq!(db.counts().expect("counts").songs, 0);
}

/// The sweep costs nothing on every open after the first.
#[test]
fn the_sweep_runs_once_and_then_knows_it_has_run() {
    let db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    // `prepare` has already run it, so the revision is stored and a second call is a single lookup.
    assert_eq!(
        db.setting(CLEANED_META).expect("setting").as_deref(),
        Some(CLEANED_META_REVISION.to_string().as_str())
    );
    db.clean_detected_text().expect("second");
}

/// The browse page's order is an index seek rather than a sort of the whole corpus.
///
/// **This is the regression the expression indexes exist to prevent, and it is invisible to every
/// other test here** — an unused index gives exactly the same rows, only after sorting a quarter
/// of a million of them. Measured on the real corpus: page one 0.24 s, page one thousand 14 s.
///
/// It also pins the thing that makes an expression index fragile: SQLite uses one only when the
/// query's expression matches the index's tree for tree, so an edit that reached only one of the
/// two would silently un-index the tool. **That surface is now two indexes rather than seven** —
/// the letter bucket, via [`title_initial`], and the language sort, via [`eff_language`]. The
/// other five key on stored columns, which cannot drift from anything; it is worth knowing this
/// test guards less than it used to, because the code needing guarding shrank.
///
/// **Songs are inserted and the statistics refreshed before the plan is read**, and that is not
/// setup that could be skipped. On an empty table SQLite has nothing to reason from, guesses that
/// `merged_into IS NULL` is the selective term, and seeks `songs_merged` instead — so a test on an
/// empty database would fail against code that is perfectly correct. It is also the second half of
/// what this test protects: `refresh_statistics` is what makes these indexes get used at all.
#[test]
fn the_browse_order_is_an_index_seek_and_not_a_sort_of_the_corpus() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    for number in 0..300 {
        add(
            &mut db,
            &format!("song-{number:04}"),
            Some(&format!("Song {number:04}")),
            &format!("folder/SONG{number:04}.kar"),
        );
    }
    db.refresh_statistics();

    // The statistics are gathered unbounded, and this is the assertion that keeps them that way.
    // A row count alone cannot catch the regression: `analysis_limit` only distorts an index once
    // the table is bigger than the limit, so with a few hundred songs here the bounded form would
    // produce identical numbers and this test would pass while the real corpus sorted itself on
    // every page — which is exactly how it got shipped the first time. So the property is pinned
    // where it is visible at any size. See `refresh_statistics`.
    assert_eq!(
        db.pragma_for_test("analysis_limit")
            .expect("analysis_limit"),
        "0",
        "statistics are gathered in full; a limit understates a low-cardinality index"
    );

    let (where_sql, bindings) = Filter::default().to_sql();
    let plan = db
        .plan_for_test(
            &format!(
                "SELECT {} FROM songs s WHERE {where_sql} ORDER BY {} LIMIT 100 OFFSET 0",
                browse_columns(),
                Filter::default().order_by(),
            ),
            &bindings,
        )
        .expect("plan");
    assert!(
        plan.contains("songs_browse_title_artist"),
        "the default browse order seeks its index; plan was:\n{plan}"
    );
    assert!(
        !sorts_the_corpus(&plan),
        "and sorts nothing to do it; plan was:\n{plan}"
    );

    // The A-Z bar, whose key is `title_initial` over the same expression.
    let letters = Filter {
        initial: Initial::Letter('A'),
        ..Filter::default()
    };
    let (letter_sql, letter_bindings) = letters.to_sql();
    let plan = db
        .plan_for_test(
            &format!(
                "SELECT {} FROM songs s WHERE {letter_sql} ORDER BY {} LIMIT 100 OFFSET 0",
                browse_columns(),
                letters.order_by(),
            ),
            &letter_bindings,
        )
        .expect("plan");
    assert!(
        plan.contains("songs_browse_letter_artist"),
        "one letter of the alphabet is a range of one index; plan was:\n{plan}"
    );
    assert!(
        !sorts_the_corpus(&plan),
        "and sorts nothing either; plan was:\n{plan}"
    );

    // The digits bucket, which is ten of those ranges. **This is the test that stops the obvious
    // simplification**: `title_initial GLOB '[0-9]'` says the same thing, reads better, and is not
    // sargable — the planner would drop the index and scan the corpus, and nothing else here would
    // notice. `Initial::clause` emits an `IN` list for exactly this reason.
    let digits = Filter {
        initial: Initial::Digits,
        ..Filter::default()
    };
    let (digit_sql, digit_bindings) = digits.to_sql();
    let plan = db
        .plan_for_test(
            &format!(
                "SELECT {} FROM songs s WHERE {digit_sql} ORDER BY {} LIMIT 100 OFFSET 0",
                browse_columns(),
                digits.order_by(),
            ),
            &digit_bindings,
        )
        .expect("plan");
    assert!(
        plan.contains("songs_browse_letter_artist"),
        "ten digits are ten ranges of one index, not a scan; plan was:\n{plan}"
    );

    // Sorting by copies, which had no index at all until `file_count` became a column.
    let copies = Filter {
        sort: Sort::Copies,
        ..Filter::default()
    };
    let (copies_sql, copies_bindings) = copies.to_sql();
    let plan = db
        .plan_for_test(
            &format!(
                "SELECT {} FROM songs s WHERE {copies_sql} ORDER BY {} LIMIT 100 OFFSET 0",
                browse_columns(),
                copies.order_by(),
            ),
            &copies_bindings,
        )
        .expect("plan");
    assert!(
        plan.contains("songs_browse_copies"),
        "the copies sort has an index now; plan was:\n{plan}"
    );
    assert!(
        !sorts_the_corpus(&plan),
        "and no longer reads the corpus into a temp B-tree; plan was:\n{plan}"
    );
}

/// Whether a plan sorts the *outer* query — the corpus — rather than something inside a subquery.
///
/// Only an unindented line counts. `browse_columns` picks the best of one song's files with an
/// `ORDER BY … LIMIT 1`, which is a sort of a handful of rows and is meant to be there.
fn sorts_the_corpus(plan: &str) -> bool {
    plan.lines()
        .any(|line| !line.starts_with(' ') && line.contains("TEMP B-TREE"))
}

/// **Every** sort the browse bar offers is an index seek, not just the two that always were.
///
/// The sibling test above pins the default order. This one pins the other eight, and it exists
/// because six of them were sorting the whole filtered corpus on every page turn — a fault with
/// no symptom a test could see, since the rows come back correct either way and only the clock
/// says anything. On the measured corpus that was tens of seconds a page against milliseconds.
///
/// The five "unrated last" sorts are the interesting ones. Each opens with `x IS NULL` to push
/// unclassified songs to the bottom, which looks like something no index could carry — and is why
/// they were left alone. SQLite indexes expressions, `IS NULL` is one, and
/// `create_browse_indexes` names it as the leading column, so the sort is served without changing
/// what any of them puts on screen. If somebody later "simplifies" either side of that pairing,
/// this is what fails.
///
/// `Sort::Copies` is deliberately absent: it orders by a correlated `COUNT(*)` over another
/// table, which an index cannot carry. See [`Filter::order_by`].
#[test]
fn every_browse_sort_is_an_index_seek() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    // Varied, and varied on purpose: with one distinct value per column the planner has no
    // reason to prefer an index and would pick a scan against perfectly correct code — the same
    // trap the sibling test documents for an empty table.
    let languages = ["pt", "en", "es"];
    for number in 0..300 {
        let id = format!("song-{number:04}");
        add_built(
            &mut db,
            &id,
            // Shared by three songs, so the performer terms of every key are terms the planner has
            // a reason to read. One title per song would let an index that had never gained them
            // pass this test — the tie-break would have no tie to break.
            Some(&format!("Song {:04}", number / 3)),
            &format!("folder/SONG{number:04}.kar"),
            |song| {
                song.det_artist = Some(format!("Artist {:02}", number % 40));
                song.det_language = (number % 5 != 0).then(|| languages[number % 3].to_owned());
                song.duration_ms = 100_000 + (number as u32 % 90) * 1_000;
            },
        );
        // Left unset on every fifth song, so the `IS NULL` leading column has both values in it.
        if number % 5 != 0 {
            db.set_user_score(&id, Some((number % 11) as u8))
                .expect("user score");
            // Written straight into the column, not left to the two setters above. They stamp every
            // one of these songs inside the same second, which leaves the second key column of
            // `songs_browse_updated_artist` constant and the planner with no reason to prefer it. A
            // direct write fires no trigger, `updated_at` being in no `UPDATE OF` list.
            db.conn
                .execute(
                    "UPDATE songs SET updated_at = ?2 WHERE id = ?1",
                    params![
                        &id,
                        format!("2026-0{}-{:02}T12:00:00Z", 1 + number % 9, 1 + number % 28)
                    ],
                )
                .expect("a stamp to sort by");
        }
        // Varied for the same reason: one scan stamps every song with the same `first_seen`.
        db.conn
            .execute(
                "UPDATE songs SET first_seen = ?2 WHERE id = ?1",
                params![
                    &id,
                    format!(
                        "2025-{:02}-{:02}T08:00:00Z",
                        1 + number % 12,
                        1 + number % 28
                    )
                ],
            )
            .expect("an added date to sort by");
    }
    db.refresh_statistics();

    for (sort, index) in [
        (Sort::Title, "songs_browse_title_artist"),
        (Sort::Suitability, "songs_browse_suitability_artist"),
        (Sort::Artist, "songs_browse_sort_artist"),
        (Sort::Language, "songs_browse_language_artist"),
        (Sort::UserScore, "songs_browse_user_score_artist"),
        (Sort::Duration, "songs_browse_duration"),
        (Sort::Updated, "songs_browse_updated_artist"),
        (Sort::Added, "songs_browse_added_artist"),
    ] {
        let filter = Filter {
            sort,
            ..Filter::default()
        };
        let (where_sql, bindings) = filter.to_sql();
        let plan = db
            .plan_for_test(
                &format!(
                    "SELECT {} FROM songs s WHERE {where_sql} ORDER BY {} LIMIT 100 OFFSET 0",
                    browse_columns(),
                    filter.order_by(),
                ),
                &bindings,
            )
            .expect("plan");
        assert!(
            plan.contains(index),
            "{sort:?} seeks {index}; plan was:\n{plan}"
        );
        assert!(
            !sorts_the_corpus(&plan),
            "{sort:?} sorts nothing to do it; plan was:\n{plan}"
        );
    }
}

/// A database that gains an index gains the statistics for it in the same open.
///
/// The condition this guards is not obvious and its failure is silent. `refresh_statistics` runs
/// only when there are none, because on a corpus it costs a second and a reopen usually has
/// nothing to learn. But an index with no `sqlite_stat1` row is one the planner will not choose —
/// so a database that already had statistics, opened by a build that adds indexes, would take the
/// indexes and go on sorting the corpus. Every row would still be correct and every other test
/// here would still pass; only the clock would say anything, which is the same trap the browse
/// indexes were shipped into once already.
#[test]
fn new_indexes_bring_their_statistics_with_them() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    for number in 0..50 {
        add(
            &mut db,
            &format!("song-{number:04}"),
            Some(&format!("Song {number:04}")),
            &format!("folder/SONG{number:04}.kar"),
        );
    }
    db.refresh_statistics();

    // Nothing missing: `prepare` therefore skips the re-analyze on every later open.
    assert!(
        missing_indexes(&db.conn, HEAVY_INDEXES)
            .expect("missing")
            .is_empty(),
        "an up-to-date database has nothing to build"
    );

    // Now the state a version bump lands in: statistics present, one index gone.
    db.conn
        .execute_batch("DROP INDEX songs_browse_language_artist;")
        .expect("drop");
    assert!(
        !db.has_no_statistics(),
        "the database still has statistics, which is what makes this the interesting case"
    );
    assert_eq!(
        missing_indexes(&db.conn, HEAVY_INDEXES).expect("missing"),
        vec!["songs_browse_language_artist"],
        "the gap is seen before the schema batch, so `prepare` knows to analyze again"
    );

    // And what `prepare` then does: build, and analyze because something was missing.
    db.create_browse_indexes().expect("rebuild");
    db.refresh_statistics();
    let analyzed: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_stat1 WHERE idx = 'songs_browse_language_artist'",
            [],
            |row| row.get(0),
        )
        .expect("stat1");
    assert_eq!(analyzed, 1, "the planner can now cost the rebuilt index");
}

/// The status bar's counts are cached, and no write path can leave them stale.
///
/// This is the half of the cache that matters. A cache that is merely slow to warm costs a page
/// load; a cache that misses an invalidation shows the wrong number forever, and nobody reports
/// a wrong number as a bug because it looks like a number.
///
/// The three writes here go in by three different routes on purpose — a scan batch, a favorite,
/// and a duplicate verdict — because the failure this guards against is per-route: a generation
/// counter bumped by hand catches the paths somebody remembered. Keying on
/// `sqlite3_total_changes` is what makes the route irrelevant, and that is the property being
/// pinned.
#[test]
fn nothing_can_be_written_without_the_counts_noticing() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    assert_eq!(db.counts().expect("counts").songs, 0);

    add(&mut db, "one", Some("One"), "folder/ONE.kar");
    assert_eq!(db.counts().expect("counts").songs, 1, "a scanned song");
    // Twice in a row with nothing in between is the cached path, and must give the same answer.
    assert_eq!(
        db.counts().expect("counts").songs,
        1,
        "and again, from cache"
    );

    let shelf = db.create_favorite("Party").expect("favorite");
    assert_eq!(
        db.counts().expect("counts").packages,
        0,
        "creating a shelf is not a package"
    );
    db.set_favorite("one", shelf, true).expect("file it");
    assert_eq!(
        db.counts().expect("counts").favorites,
        1,
        "a song filed under a favorite"
    );

    db.create_package(
        &PackageRow {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-08-27T00:00:00Z",
    )
    .expect("package");
    assert_eq!(db.counts().expect("counts").packages, 1, "a package");
}

/// The discard pile is counted the way the list of it is filtered.
///
/// **Settings is the only place this number is written**, because the browse list shows the pile
/// only to somebody who has already asked for it. A corpus with a hundred songs thrown away looks
/// exactly like one with none until then.
///
/// The merged song is the case worth pinning. *Only deleted* keeps `merged_into IS NULL`, since a
/// song both merged and thrown away is still a merge and has no row of its own — so a count that
/// dropped that half would report more than the list it counts.
#[test]
fn the_discard_pile_is_counted_as_the_list_of_it_is_filtered() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "keep", Some("Keep"), "folder/KEEP.kar");
    add(&mut db, "toss", Some("Toss"), "folder/TOSS.kar");
    add(&mut db, "both", Some("Both"), "folder/BOTH.kar");

    assert_eq!(
        db.counts().expect("counts").deleted,
        0,
        "a corpus nobody has thrown anything away from"
    );

    db.set_deleted_of(&["toss".to_owned()], true)
        .expect("throw one away");
    assert_eq!(db.counts().expect("counts").deleted, 1);

    db.set_deleted_of(&["both".to_owned()], true)
        .expect("throw another away");
    db.set_merged_into("both", Some("keep")).expect("merge it");
    assert_eq!(
        db.counts().expect("counts").deleted,
        1,
        "a song both merged and thrown away has no row of its own in either list"
    );

    db.set_deleted_of(&["toss".to_owned()], false)
        .expect("bring it back");
    assert_eq!(
        db.counts().expect("counts").deleted,
        0,
        "and bringing one back empties the pile again"
    );
}

/// A page asks for one row more than it shows, and that spare row is what says *next*.
///
/// The count it replaces was `SELECT COUNT(*)` over the filtered corpus on **every page turn**,
/// to compare against the offset — the most expensive thing the browse page did, for one button.
#[test]
fn a_page_knows_there_is_another_without_counting_the_corpus() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    for number in 0..5 {
        add(
            &mut db,
            &format!("song-{number}"),
            Some(&format!("Song {number}")),
            &format!("folder/SONG{number}.kar"),
        );
    }

    let page = |offset: u32| Filter {
        limit: 2,
        offset,
        ..Filter::default()
    };

    let (rows, more) = db.songs_page(&page(0)).expect("page");
    assert_eq!(rows.len(), 2, "the page is the size it asked for");
    assert!(more, "and there are three more behind it");

    // The last full page: two rows, one left over, so still more.
    let (rows, more) = db.songs_page(&page(2)).expect("page");
    assert_eq!(rows.len(), 2);
    assert!(more);

    // The remainder. One row, nothing past it.
    let (rows, more) = db.songs_page(&page(4)).expect("page");
    assert_eq!(rows.len(), 1);
    assert!(!more, "the fifth song is the last one");

    // Exactly divisible is the case that catches an off-by-one: five songs, a page of five.
    let (rows, more) = db
        .songs_page(&Filter {
            limit: 5,
            ..Filter::default()
        })
        .expect("page");
    assert_eq!(rows.len(), 5);
    assert!(!more, "a full last page is still a last page");

    // Past the end entirely.
    let (rows, more) = db.songs_page(&page(99)).expect("page");
    assert!(rows.is_empty());
    assert!(!more);

    // And the plain `songs` never hands the spare row to a caller.
    assert_eq!(db.songs(&page(0)).expect("songs").len(), 2);
}

/// The three subqueries every browse row runs read the index and never the table.
///
/// Two of them want a song's paths *in order* (`ORDER BY f.path LIMIT 1`), which `files_song`
/// cannot give — so each of a page's rows sorted its own file list and then went to the
/// table for the path. Five hundred subquery executions a page is the scale that makes this
/// worth an index of its own.
#[test]
fn a_rows_file_subqueries_are_answered_from_the_index() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    for number in 0..300 {
        let id = format!("song-{number:04}");
        // Two copies of most songs, so the path list is worth ordering.
        add(
            &mut db,
            &id,
            Some(&format!("Song {number:04}")),
            &format!("a/SONG{number:04}.kar"),
        );
        if number % 3 != 0 {
            add(
                &mut db,
                &id,
                Some(&format!("Song {number:04}")),
                &format!("b/SONG{number:04}.kar"),
            );
        }
    }
    db.refresh_statistics();

    let plan = db
        .plan_for_test(
            "SELECT (SELECT f.path FROM files f WHERE f.song_id = s.id ORDER BY f.path LIMIT 1)
             FROM songs s LIMIT 100",
            &[],
        )
        .expect("plan");
    assert!(
        plan.contains("files_song_path"),
        "the nicest-path subquery seeks the composite index; plan was:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "and needs no sort, because the index is already in path order; plan was:\n{plan}"
    );
}

/// The connection is tuned for a database the size of a real corpus.
///
/// On disk rather than in memory, because the tolerance is the point: WAL and `mmap` behave
/// differently in memory, and `synchronous` is asserted **only** where WAL actually took. Pinning
/// it unconditionally would fail on exactly the network-share case the tolerant WAL switch in
/// `prepare` exists for.
#[test]
fn the_connection_is_tuned_for_a_database_this_size() {
    let scratch = Scratch::new("tuning");
    let dir = scratch.0.clone();
    let db = Db::create(&dir).expect("create");

    assert_eq!(
        db.pragma_for_test("cache_size").expect("cache_size"),
        "-262144",
        "256 MB of page cache, not the 2 MB default"
    );
    assert_eq!(
        db.pragma_for_test("temp_store").expect("temp_store"),
        "2",
        "a spilled sort stays in memory"
    );
    // Unconditional where the three around it are not, and set before anything that can meet a lock
    // rather than among them: a connection running with SQLite's default of zero answers contention
    // with an immediate failure, which is the "database is locked" a page render came back with.
    assert_eq!(
        db.pragma_for_test("busy_timeout").expect("busy_timeout"),
        "5000",
        "five seconds of waiting, not the default of none"
    );
    if db
        .pragma_for_test("journal_mode")
        .expect("journal_mode")
        .eq_ignore_ascii_case("wal")
    {
        assert_eq!(
            db.pragma_for_test("synchronous").expect("synchronous"),
            "1",
            "NORMAL, which is crash-safe under WAL and only under WAL"
        );
    }
}

/// Writes one song with one file, enough for the browse queries to have something to find.
/// A scan records the code the file's own evidence implies, and the encoding outranks the header.
#[test]
fn a_scan_works_out_the_language_and_the_encoding_wins() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_with_language(&mut db, "brazilian", Some("PORT"), "windows-1252");
    // The case the precedence rule exists for: a Japanese file whose header says English,
    // which is what almost every Japanese file in a real corpus says.
    add_with_language(&mut db, "japanese", Some("ENGL"), "Shift_JIS");
    add_with_language(&mut db, "english", Some("ENGL"), "windows-1252");
    add_with_language(&mut db, "silent", None, "windows-1252");

    assert_eq!(tag(&db, "brazilian"), Some("pt".to_owned()));
    assert_eq!(tag(&db, "japanese"), Some("ja".to_owned()));
    assert_eq!(tag(&db, "english"), Some("en".to_owned()));
    assert_eq!(
        tag(&db, "silent"),
        None,
        "nothing said anything, so nothing is written -- not `und`, which is a claim"
    );

    // `det_language` is untouched by any of it: "what did the file say?" stays answerable, and
    // the edit form shows the answer.
    assert_eq!(
        db.song("japanese").expect("detail").det_language.as_deref(),
        Some("ENGL")
    );
}

/// The backfill repairs rows written before the column existed, from what is already stored.
#[test]
fn the_language_backfill_fills_rows_indexed_by_an_earlier_version() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_with_language(&mut db, "brazilian", Some("PORT"), "windows-1252");
    add_with_language(&mut db, "japanese", Some("ENGL"), "Shift_JIS");
    add_with_language(&mut db, "mystery", Some("LATI"), "windows-1252");

    // The sources are there and the derived column is not, which is what a revision bump leaves.
    db.execute_for_test("UPDATE songs SET det_language_tag = NULL")
        .expect("degrade");
    assert_eq!(tag(&db, "brazilian"), None);

    db.backfill_language_tags_for_test().expect("backfill");

    assert_eq!(tag(&db, "brazilian"), Some("pt".to_owned()));
    assert_eq!(
        tag(&db, "japanese"),
        Some("ja".to_owned()),
        "the backfill reads the encoding too, not only the header"
    );
    assert_eq!(
        tag(&db, "mystery"),
        None,
        "a declaration this build cannot read leaves the column alone rather than guessing"
    );
    assert_eq!(
        db.song("mystery").expect("detail").det_language.as_deref(),
        Some("LATI"),
        "and the declaration itself is still there to be looked at"
    );
}

/// A `.kmbuild` carrying a `packages.bank` column still opens, and nothing reads it.
///
/// **A curation database at the current schema can carry a column `schema.sql` does not declare**,
/// and one that holds months of hand curation does. So every statement here names its columns: a
/// package's bank comes from its id, and a number sitting in a row cannot move one.
#[test]
fn a_curation_database_carrying_a_bank_column_still_opens() {
    let scratch = Scratch::new("prefix");
    let dir = scratch.0.clone();

    {
        let db = Db::create(&dir).expect("create");
        // The column is there, with a number in it.
        db.execute_for_test("ALTER TABLE packages ADD COLUMN bank INTEGER")
            .expect("add the column");
        db.create_package(
            &crate::model::PackageRow {
                id: km_kmpkg::EXAMPLE_ID.to_owned(),
                name: "Volume 1".to_owned(),
                version: "1.0.0".to_owned(),
                publisher: None,
                start_number: 1,
                default_language: None,
                out_path: None,
                built_at: None,
                song_count: 0,
                ..crate::model::PackageRow::new("", "")
            },
            "2026-08-29T00:00:00Z",
        )
        .expect("create a package");
        db.execute_for_test("UPDATE packages SET bank = 3")
            .expect("a bank somebody once chose");
    }

    let db = Db::open(&dir).expect("a database carrying the column still opens");
    let mut package = db.package(km_kmpkg::EXAMPLE_ID).expect("package");
    assert_eq!(package.name, "Volume 1");

    // And it is still writable, which is the half a column list gets wrong: an `UPDATE` naming
    // every column but this one has to be fine on a table that still has it.
    package.name = "Volume One".to_owned();
    db.update_package(&package).expect("update");
    assert_eq!(
        db.package(km_kmpkg::EXAMPLE_ID).expect("package").name,
        "Volume One"
    );
}

#[test]
fn a_language_that_is_not_a_code_is_refused_and_a_real_one_is_canonicalised() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "song", Some("Corcovado"), "a/corcovado.kar");

    let refused = db.edit_song(
        "song",
        &SongEdit {
            language: Some(Some("Portuguese".to_owned())),
            ..SongEdit::default()
        },
    );
    assert!(
        matches!(refused, Err(DbError::Rejected(_))),
        "a column a filter and a sort read cannot hold something nothing matches; got {refused:?}"
    );

    db.edit_song(
        "song",
        &SongEdit {
            language: Some(Some("  PT  ".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("a real code, however it was typed");
    assert_eq!(
        db.song("song").expect("detail").language.as_deref(),
        Some("pt"),
        "stored canonically, so comparisons stay exact"
    );
}

#[test]
fn the_effective_language_is_the_chosen_one_over_the_detected_one() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    // A Brazilian song whose header claims English -- the corpus's commonest wrong answer.
    add_with_language(&mut db, "song", Some("ENGL"), "windows-1252");
    // Asserted through the browse row rather than a Rust helper, because `eff_language` is what
    // the column, the filter and the sort all read: if the SQL and the page disagreed, a song
    // would be listed under one language and open under another.
    assert_eq!(
        db.song_row("song").expect("row").language.as_deref(),
        Some("en")
    );
    assert!(db.song("song").expect("detail").language_is_detected());

    db.edit_song(
        "song",
        &SongEdit {
            language: Some(Some("pt".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("correct it");

    let detail = db.song("song").expect("detail");
    assert_eq!(
        db.song_row("song").expect("row").language.as_deref(),
        Some("pt")
    );
    assert!(
        !detail.language_is_detected(),
        "somebody has said now, so the page must stop calling it detected"
    );
    assert_eq!(
        detail.det_language_tag.as_deref(),
        Some("en"),
        "the correction does not erase what was detected"
    );
}

#[test]
fn filtering_by_language_narrows_and_both_sentinels_work() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_with_language(&mut db, "brazilian", Some("PORT"), "windows-1252");
    add_with_language(&mut db, "japanese", Some("ENGL"), "Shift_JIS");
    add_with_language(&mut db, "silent", None, "UTF-8");

    let matching = |language: LanguageFilter| {
        let rows = db
            .songs(&Filter {
                language,
                ..Filter::default()
            })
            .expect("browse");
        let mut ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        ids.sort();
        ids
    };

    assert_eq!(
        matching(LanguageFilter::parse("pt")),
        vec!["brazilian".to_owned()]
    );
    assert_eq!(
        matching(LanguageFilter::parse("ja")),
        vec!["japanese".to_owned()]
    );
    assert_eq!(
        matching(LanguageFilter::Unset),
        vec!["silent".to_owned()],
        "the commonest case on a real corpus, and the one the bulk set pairs with"
    );
    assert_eq!(
        matching(LanguageFilter::Set),
        vec!["brazilian".to_owned(), "japanese".to_owned()]
    );
    assert_eq!(matching(LanguageFilter::Any).len(), 3);
    assert_eq!(
        matching(LanguageFilter::parse("Klingon")).len(),
        3,
        "a hand-typed nonsense value narrows nothing rather than emptying the page"
    );

    // A chosen language filters as itself, not as what was detected.
    db.edit_song(
        "japanese",
        &SongEdit {
            language: Some(Some("ko".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("correct it");
    assert_eq!(
        matching(LanguageFilter::parse("ko")),
        vec!["japanese".to_owned()]
    );
    assert!(matching(LanguageFilter::parse("ja")).is_empty());
}

/// A song thrown away leaves every list but the one that asks for it, and comes back whole.
///
/// **The count is asserted beside the rows**, because the two are answered by different SQL over
/// the same `Filter::to_sql` — a page that hides a song while the label goes on counting it is the
/// failure a term added to one and not the other produces.
#[test]
fn a_song_thrown_away_is_in_no_list_but_the_one_that_asks_for_it() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "keep", Some("Corcovado"), "folder/keep.kar");
    add(&mut db, "toss", Some("Chega de Saudade"), "folder/toss.kar");

    let listed = |deleted: DeletedFilter| {
        let filter = Filter {
            deleted,
            ..Filter::default()
        };
        let mut ids: Vec<String> = db
            .songs(&filter)
            .expect("browse")
            .iter()
            .map(|row| row.id.clone())
            .collect();
        ids.sort();
        (ids, db.count_matching(&filter).expect("count"))
    };

    assert_eq!(
        db.set_deleted_of(&["toss".to_owned()], true)
            .expect("throw"),
        1
    );

    let (live, live_count) = listed(DeletedFilter::Live);
    assert_eq!(live, vec!["keep".to_owned()]);
    assert_eq!(
        live_count, 1,
        "the label has to agree with the rows under it"
    );

    let (gone, gone_count) = listed(DeletedFilter::Only);
    assert_eq!(gone, vec!["toss".to_owned()]);
    assert_eq!(gone_count, 1);

    // Every other surface asks the same question through the same clause, so a search that found
    // it before must not find it now.
    assert!(
        db.songs(&Filter {
            query: Some("Saudade".to_owned()),
            ..Filter::default()
        })
        .expect("search")
        .is_empty(),
        "a search is a narrowing of the browse list and inherits the term"
    );

    // And back. The row was never removed, so nothing has to be scanned to return it.
    assert_eq!(
        db.set_deleted_of(&["toss".to_owned()], false)
            .expect("back"),
        1
    );
    assert_eq!(listed(DeletedFilter::Live).0.len(), 2);
    assert!(listed(DeletedFilter::Only).0.is_empty());
}

/// Deleting over a filter writes the rows that filter lists, and counts a package before it does.
#[test]
fn deleting_a_whole_filter_takes_what_it_lists_and_counts_what_a_package_holds() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "one", Some("Corcovado"), "brasil/one.kar");
    add(&mut db, "two", Some("Insensatez"), "brasil/two.kar");
    add(&mut db, "far", Some("Yesterday"), "ingles/far.kar");

    db.create_package(
        &PackageRow::new("1f4a9c8e2b7d0356", "Brasil"),
        "2026-09-18T00:00:00Z",
    )
    .expect("a package");
    db.add_to_package(
        "1f4a9c8e2b7d0356",
        &["one".to_owned()],
        "2026-09-18T00:00:00Z",
    )
    .expect("file one of them");

    let brasil = Filter {
        folder: Some("brasil/".to_owned()),
        ..Filter::default()
    };
    assert_eq!(db.count_matching(&brasil).expect("count"), 2);
    assert_eq!(
        db.packaged_count_matching(&brasil).expect("packaged"),
        1,
        "the confirmation names this number before the write"
    );

    assert_eq!(db.set_deleted_for(&brasil, true).expect("throw"), 2);
    // A packaged song goes with the rest: the count is said, not enforced.
    assert_eq!(db.count_matching(&brasil).expect("count"), 0);
    assert_eq!(
        db.count_matching(&Filter::default()).expect("count"),
        1,
        "the folder that was not named keeps its song"
    );
}

/// A ticked row already in the state being asked for is not a row the write touches.
#[test]
fn the_ticked_count_is_what_the_write_will_do_rather_than_what_was_ticked() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "gone", Some("Corcovado"), "folder/gone.kar");
    add(&mut db, "here", Some("Insensatez"), "folder/here.kar");
    db.set_deleted_of(&["gone".to_owned()], true)
        .expect("throw");

    let both = ["gone".to_owned(), "here".to_owned()];
    assert_eq!(
        db.deletable_count_of(&both, true).expect("count"),
        1,
        "one of the two is already thrown away"
    );
    assert_eq!(
        db.deletable_count_of(&both, false).expect("count"),
        1,
        "and the other way round"
    );
}

/// Songs enough to reason from, with the statistics gathered.
///
/// **A plan read off an empty table says nothing.** SQLite has no `sqlite_stat1` to consult, guesses
/// which term is selective, and picks an index a real corpus would never make it pick — so a planner
/// test without this passes and fails for reasons unconnected to the code. The same setup
/// `the_browse_order_is_an_index_seek_and_not_a_sort_of_the_corpus` makes, and its note says why at
/// length.
fn corpus_with_statistics() -> Db {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    for number in 0..300 {
        add(
            &mut db,
            &format!("song-{number:04}"),
            Some(&format!("Song {number:04}")),
            &format!("folder/SONG{number:04}.kar"),
        );
    }
    db.refresh_statistics();
    db
}

/// The *only deleted* list seeks its own partial index rather than reading the corpus.
///
/// **A planner test rather than a behavior one**, like the browse sorts beside it: the predicate is
/// true of nearly no row, so a page that scans to find the handful somebody discarded does not
/// fail — it is merely slow for ever.
///
/// **It is also what says a partial index beside `songs_countable` would be waste.** All three
/// terms are in that key, so this is equality on two columns and a range on the third; a second
/// corpus-sized B-tree to answer the same question would be maintained on every write for nothing.
#[test]
fn the_deleted_list_is_answered_from_its_own_index() {
    let db = corpus_with_statistics();
    // A handful thrown away out of three hundred, because an empty partial index has no statistics
    // row at all and the planner walks past one it cannot price. The proportion is the point as
    // much as the rows: a discard pile is a sliver of a corpus, which is what makes the partial
    // index worth having.
    db.set_deleted_of(
        &[
            "song-0007".to_owned(),
            "song-0042".to_owned(),
            "song-0100".to_owned(),
        ],
        true,
    )
    .expect("throw three away");
    db.refresh_statistics();

    let filter = Filter {
        deleted: DeletedFilter::Only,
        ..Filter::default()
    };
    let (where_clause, _values) = filter.to_sql();
    let plan = db
        .plan_for_test(
            &format!(
                "SELECT s.id FROM songs s WHERE {where_clause} ORDER BY {} LIMIT 51",
                filter.order_by()
            ),
            &[],
        )
        .expect("a plan");
    assert!(
        plan.contains("songs_deleted"),
        "the deleted list has to seek its own partial index rather than scan the corpus: {plan}"
    );
}

/// Deleting must not cost the browse list its orderings, which is the expensive way to be wrong.
///
/// **The `ORDER BY` is the whole test, and leaving it out is what let this through once.** Checking
/// the `WHERE` alone says an index was chosen and nothing about which; the browse page's cost is
/// decided by whether the chosen one also supplies the order. `songs_countable` holding `deleted_at`
/// as a third *key* column beat `songs_browse_*` on the equality terms, carried no order, and every
/// sorted page read `USE TEMP B-TREE FOR ORDER BY` over the whole corpus — measured at four seconds
/// a page against milliseconds, on a real corpus, with nothing on screen saying why. The predicate
/// lives in that index's `WHERE` for exactly this reason.
///
/// One assertion per sort, because an index is chosen per query and a single sort passing says
/// nothing about the other nine.
#[test]
fn deleting_costs_the_browse_sorts_none_of_their_indexes() {
    let db = corpus_with_statistics();
    for sort in [
        Sort::Title,
        Sort::Artist,
        Sort::Suitability,
        Sort::UserScore,
        Sort::Duration,
        Sort::Copies,
        Sort::Language,
        Sort::Updated,
        Sort::Added,
    ] {
        let filter = Filter {
            sort,
            ..Filter::default()
        };
        let (where_clause, _values) = filter.to_sql();
        let plan = db
            .plan_for_test(
                &format!(
                    "SELECT s.id FROM songs s WHERE {where_clause} ORDER BY {} LIMIT 51",
                    filter.order_by()
                ),
                &[],
            )
            .expect("a plan");
        assert!(
            !plan.contains("TEMP B-TREE"),
            "sorting by {} reads the corpus into a temp B-tree: {plan}",
            sort.as_str()
        );
        assert!(
            plan.contains("songs_browse_"),
            "sorting by {} has to walk a browse index: {plan}",
            sort.as_str()
        );
    }
}

/// The bulk set writes exactly the rows the same filter lists, and nothing else.
///
/// The property worth pinning is not that it works but that it agrees: it and the browse list
/// are built from one `Filter::to_sql`, so a filter that narrows the page narrows the write.
#[test]
fn the_bulk_set_writes_exactly_the_rows_the_same_filter_lists() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("Corcovado"), "brasil/a.kar");
    add(&mut db, "b", Some("Wave"), "brasil/b.kar");
    add(&mut db, "c", Some("Hey Jude"), "ingles/c.kar");

    let brasil = Filter {
        folder: Some("brasil/".to_owned()),
        ..Filter::default()
    };
    assert_eq!(db.count_matching(&brasil).expect("count"), 2);

    let changed = db
        .set_language_for(&brasil, Language::parse("pt"))
        .expect("bulk set");
    assert_eq!(changed, 2);

    let language = |id: &str| db.song(id).expect("detail").language;
    assert_eq!(language("a").as_deref(), Some("pt"));
    assert_eq!(language("b").as_deref(), Some("pt"));
    assert_eq!(
        language("c"),
        None,
        "the song outside the filter is untouched"
    );

    // And the rows it wrote are the rows the list shows for that filter -- one clause, so the
    // two cannot disagree.
    let rows = db.songs(&brasil).expect("browse");
    assert_eq!(ids(&rows), vec!["a", "b"]);

    // Clearing is possible, for the same reason clearing a score is: a mis-click must not be
    // permanent.
    assert_eq!(db.set_language_for(&brasil, None).expect("clear"), 2);
    assert_eq!(language("a"), None);
}

/// Everything the filter matches, in the order the list shows it, and not one page of it.
///
/// The order is the half that is easy to leave untested and expensive to get wrong: the one
/// caller hands this straight to `add_to_package`, which numbers songs in the order it is given
/// them, so a package made from a list sorted by title would otherwise come out numbered by
/// whatever SQLite felt like.
#[test]
fn the_matching_ids_are_every_match_in_the_order_the_list_shows_them() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "c", Some("Corcovado"), "brasil/c.kar");
    add(&mut db, "a", Some("Amanhã"), "brasil/a.kar");
    add(&mut db, "w", Some("Wave"), "brasil/w.kar");
    add(&mut db, "h", Some("Hey Jude"), "ingles/h.kar");

    let brasil = Filter {
        folder: Some("brasil/".to_owned()),
        // A page of one, which this must ignore -- it answers *everything matching*.
        limit: 1,
        ..Filter::default()
    };
    assert_eq!(
        db.matching_ids(&brasil, None).expect("ids"),
        vec!["a", "c", "w"],
        "three songs, by title, with the one outside the folder left out"
    );

    // And the sort is the list's own, so a different sort gives a different numbering.
    let by_length = Filter {
        sort: Sort::parse("duration"),
        ..brasil.clone()
    };
    assert_eq!(db.matching_ids(&by_length, None).expect("ids").len(), 3);
}

/// Taking the title from the file name beats what the file said, and leaves what it said alone.
///
/// Both halves matter. Winning over `det_title` is the point of the action — the corpus is full
/// of files whose own title is `UNTITLED` — and it is why this writes the column rather than
/// clearing it, which would fall back to exactly the value being escaped. Leaving `det_title`
/// where it is keeps the song page able to answer *what did the file say?*.
#[test]
fn taking_the_title_from_the_file_name_beats_what_the_file_said() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("UNTITLED"), "brasil/Corcovado.kar");
    add(&mut db, "b", Some("Wave"), "brasil/WAVE001.kar");

    assert_eq!(
        db.set_names_from_stem(&["a".to_owned()]).expect("rename"),
        1
    );

    let a = db.song("a").expect("detail");
    assert_eq!(a.effective_title(), "Corcovado");
    assert_eq!(
        a.det_title.as_deref(),
        Some("UNTITLED"),
        "what the file said is still answerable"
    );
    assert!(
        !a.title_is_filename(),
        "a title somebody chose is not the same as no title at all, even when they are the \
         same words"
    );

    // The song that was not ticked keeps the title it had.
    assert_eq!(db.song("b").expect("detail").effective_title(), "Wave");
}

/// The artist goes with the title, and it goes as `''` rather than as NULL.
///
/// The distinction is the whole of why this is not a one-word change. `eff_artist` has no `nullif`,
/// so a NULL artist falls back to `det_artist` — the value the action exists to escape — and only
/// the empty string stands in front of it. `det_artist` stays where it is for `det_title`'s reason.
#[test]
fn taking_the_title_from_the_file_name_empties_the_artist() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(
        &mut db,
        "a",
        Some("UNTITLED"),
        "brasil/Corcovado.kar",
        |song| {
            song.det_artist = Some("MIDI KARAOKE".to_owned());
        },
    );

    assert_eq!(
        db.set_names_from_stem(&["a".to_owned()]).expect("rename"),
        1
    );

    let a = db.song("a").expect("detail");
    assert_eq!(
        a.effective_artist().as_deref(),
        Some(""),
        "an artist nobody named, and not the one the sequencer left behind"
    );
    assert_eq!(
        a.det_artist.as_deref(),
        Some("MIDI KARAOKE"),
        "what the file said is still answerable"
    );
}

/// And it takes an artist somebody typed with it.
///
/// The judgment being made is about the whole of what the file claims to be, so a correction made
/// before the file name was chosen is not carried over it. A curator who wants one keeps it by not
/// ticking the row.
#[test]
fn taking_the_title_from_the_file_name_clears_an_artist_somebody_typed() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("UNTITLED"), "brasil/Corcovado.kar");
    db.edit_song(
        "a",
        &SongEdit {
            artist: Some(Some("Tom Jobim".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    db.set_names_from_stem(&["a".to_owned()]).expect("rename");

    assert_eq!(
        db.song("a").expect("detail").effective_artist().as_deref(),
        Some("")
    );
}

/// A row that got no title keeps its artist.
///
/// One statement, so the `WHERE` that spares a song with no `stem` spares both columns. A count
/// that said nothing changed while an artist had gone would be the worst of both.
#[test]
fn a_song_with_no_file_name_to_take_keeps_the_artist_it_had() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(
        &mut db,
        "a",
        Some("UNTITLED"),
        "brasil/Corcovado.kar",
        |song| {
            song.det_artist = Some("Cazuza".to_owned());
            song.stem = String::new();
        },
    );

    assert_eq!(
        db.set_names_from_stem(&["a".to_owned()]).expect("rename"),
        0
    );

    assert_eq!(
        db.song("a").expect("detail").effective_artist().as_deref(),
        Some("Cazuza")
    );
}

/// Fixing the capitals writes what the file said into the column a person owns.
///
/// The rule itself is `casing::recase`'s and is tested there. What is asserted here is where the
/// answer lands: `det_title` and `det_artist` stay answerable to *what did the file say?*, so a row
/// whose only name came from the file gains one of its own rather than having the file's rewritten.
#[test]
fn fixing_the_capitals_writes_into_the_column_a_person_owns() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(
        &mut db,
        "a",
        Some("CORCOVADO"),
        "brasil/Corcovado.kar",
        |song| {
            song.det_artist = Some("TOM JOBIM".to_owned());
        },
    );

    assert_eq!(db.fix_name_case(&["a".to_owned()]).expect("recase"), 1);

    let a = db.song("a").expect("detail");
    assert_eq!(a.effective_title(), "Corcovado");
    assert_eq!(a.effective_artist().as_deref(), Some("Tom Jobim"));
    assert_eq!(
        (a.det_title.as_deref(), a.det_artist.as_deref()),
        (Some("CORCOVADO"), Some("TOM JOBIM")),
        "what the file said is still answerable"
    );
}

/// A name the file gave in mixed but wrong case is fixed too, and lands in the column a person owns.
///
/// A sequencer's case is not a decision anybody made about this corpus, so the guard that protects a
/// typed name does not reach it.
#[test]
fn fixing_the_capitals_recases_a_mixed_name_the_file_gave() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(&mut db, "a", Some("Garota De Ipanema"), "b/g.kar", |song| {
        song.det_artist = Some("Tom jobim".to_owned());
    });

    assert_eq!(db.fix_name_case(&["a".to_owned()]).expect("recase"), 1);

    let a = db.song("a").expect("detail");
    assert_eq!(a.title.as_deref(), Some("Garota de Ipanema"));
    assert_eq!(a.artist.as_deref(), Some("Tom Jobim"));
    assert_eq!(
        (a.det_title.as_deref(), a.det_artist.as_deref()),
        (Some("Garota De Ipanema"), Some("Tom jobim")),
        "what the file said is still answerable"
    );
}

/// A mixed-case name somebody typed is their decision, and stays over a shouting file name.
#[test]
fn fixing_the_capitals_leaves_a_mixed_name_somebody_typed() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("MCCARTNEY MEDLEY"), "b/m.kar");
    db.edit_song(
        "a",
        &SongEdit {
            title: Some(Some("McCartney Medley De".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    assert_eq!(db.fix_name_case(&["a".to_owned()]).expect("recase"), 0);

    assert_eq!(
        db.song("a").expect("detail").effective_title(),
        "McCartney Medley De"
    );
}

/// A title that moved does not take an artist with it, in either direction.
///
/// Each column keeps what it held wherever the rule had nothing to say. The failure this rules out
/// is a single `UPDATE` of both columns writing NULL over the half that did not change — which on
/// the artist means falling back to `det_artist`, the value a curator typed over.
#[test]
fn fixing_the_capitals_leaves_the_half_it_had_nothing_to_say_about() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(&mut db, "a", Some("GAROTA DE IPANEMA"), "b/g.kar", |song| {
        song.det_artist = Some("MIDI KARAOKE".to_owned());
    });
    db.edit_song(
        "a",
        &SongEdit {
            artist: Some(Some("Tom Jobim".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    assert_eq!(db.fix_name_case(&["a".to_owned()]).expect("recase"), 1);

    let a = db.song("a").expect("detail");
    assert_eq!(a.effective_title(), "Garota de Ipanema");
    assert_eq!(
        a.effective_artist().as_deref(),
        Some("Tom Jobim"),
        "an artist already cased is not handed back to the sequencer's"
    );
}

/// A row showing its file name has no title to fix, and is not given one.
///
/// `eff_title` falls back to the stem for display. Following that fallback here would quietly make
/// this the file-name button as well — two actions in one press, only one of them asked for.
#[test]
fn fixing_the_capitals_does_not_take_a_title_from_the_file_name() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", None, "brasil/CORCOVAD.kar");

    assert_eq!(db.fix_name_case(&["a".to_owned()]).expect("recase"), 0);

    let a = db.song("a").expect("detail");
    assert!(a.title_is_filename(), "still nameless: {:?}", a.title);
}

/// A name nobody has to decide about is not a write, and the count says so.
#[test]
fn fixing_the_capitals_counts_only_the_songs_it_changed() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "shouting", Some("CORCOVADO"), "b/a.kar");
    add(&mut db, "decided", Some("Tom Jobim"), "b/b.kar");
    add(&mut db, "nameless", None, "b/c.kar");

    let ids = ["shouting", "decided", "nameless"].map(ToOwned::to_owned);
    assert_eq!(db.fix_name_case(&ids).expect("recase"), 1);

    assert_eq!(
        db.song("decided").expect("detail").effective_title(),
        "Tom Jobim"
    );
}

/// The artist comes out of the title and lands in the column a person owns.
///
/// The rule is `names::artist_and_title`'s and is tested there. What is asserted here is where the
/// answer lands: `det_title` stays answerable to *what did the file say?*, so the pair a curator now
/// sees is theirs and the file's is untouched.
#[test]
fn splitting_the_artist_writes_into_the_columns_a_person_owns() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(
        &mut db,
        "a",
        Some("Bob Dylan-A Hard Rain's A-Gonna Fall"),
        "us/dylan.kar",
    );

    assert_eq!(
        db.split_artist_from_title(&["a".to_owned()])
            .expect("split"),
        1
    );

    let a = db.song("a").expect("detail");
    assert_eq!(a.title.as_deref(), Some("A Hard Rain's A-Gonna Fall"));
    assert_eq!(a.artist.as_deref(), Some("Bob Dylan"));
    assert_eq!(
        a.det_title.as_deref(),
        Some("Bob Dylan-A Hard Rain's A-Gonna Fall"),
        "what the file said is still answerable"
    );
}

/// A song that names an artist has had this judgment made about it.
///
/// Both readings of an artist count as named: one the file declared, and one a curator typed.
#[test]
fn splitting_the_artist_leaves_a_song_that_names_one() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(
        &mut db,
        "told",
        Some("Bob Seger-Mainstreet"),
        "us/s.kar",
        |song| {
            song.det_artist = Some("Bob Seger".to_owned());
        },
    );
    add(&mut db, "typed", Some("Bob Seger-Blind Love"), "us/b.kar");
    db.edit_song(
        "typed",
        &SongEdit {
            artist: Some(Some("Bob Seger".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    let ids = ["told", "typed"].map(ToOwned::to_owned);
    assert_eq!(db.split_artist_from_title(&ids).expect("split"), 0);

    assert_eq!(
        db.song("told").expect("detail").effective_title(),
        "Bob Seger-Mainstreet",
        "the title keeps the name the artist column already holds"
    );
}

/// An artist recorded as the empty string is blank, and is what this button is reached for.
///
/// `''` is *Title from file name*'s output and `eff_artist` reads it as *explicitly nobody*. Those
/// rows are exactly the ones still carrying their artist inside the title, so the `nullif` here
/// reads them as blank where the browse list reads them as a value.
#[test]
fn splitting_the_artist_reaches_a_row_the_file_name_button_emptied() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_built(
        &mut db,
        "a",
        Some("UNTITLED"),
        "us/Bob Seger-Night moves.kar",
        |song| {
            song.det_artist = Some("MIDI KARAOKE".to_owned());
        },
    );
    assert_eq!(db.set_names_from_stem(&["a".to_owned()]).expect("stem"), 1);

    assert_eq!(
        db.split_artist_from_title(&["a".to_owned()])
            .expect("split"),
        1
    );

    let a = db.song("a").expect("detail");
    assert_eq!(a.title.as_deref(), Some("Night moves"));
    assert_eq!(a.artist.as_deref(), Some("Bob Seger"));
}

/// A row whose only name is its file name splits too, and that is the answer `fix_name_case` does
/// not give.
///
/// Recasing a stem row would be the file-name button's work done again. Splitting one fills the
/// artist column that button never fills, so the press is not two actions in one.
#[test]
fn splitting_the_artist_reads_the_file_name_when_that_is_the_only_name() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", None, "jam/bob_marley-zimbabwe.kar");

    assert_eq!(
        db.split_artist_from_title(&["a".to_owned()])
            .expect("split"),
        1
    );

    let a = db.song("a").expect("detail");
    assert_eq!(a.title.as_deref(), Some("zimbabwe"));
    assert_eq!(a.artist.as_deref(), Some("bob_marley"));
    assert!(
        !a.title_is_filename(),
        "the row now has a name of its own: {:?}",
        a.title
    );
}

/// A title with no seam in it is not a write, and the count says so.
#[test]
fn splitting_the_artist_counts_only_the_songs_it_changed() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(
        &mut db,
        "seamed",
        Some("Bob Seger-Still the same"),
        "b/a.kar",
    );
    add(&mut db, "whole", Some("Corcovado"), "b/b.kar");
    add(&mut db, "half", Some("-Zimbabwe"), "b/c.kar");

    let ids = ["seamed", "whole", "half"].map(ToOwned::to_owned);
    assert_eq!(db.split_artist_from_title(&ids).expect("split"), 1);

    assert_eq!(
        db.song("whole").expect("detail").effective_title(),
        "Corcovado"
    );
    assert_eq!(
        db.song("half").expect("detail").effective_title(),
        "-Zimbabwe",
        "half a name is left exactly as it is"
    );
}

/// The browse list files the song under its new name, which means the sort keys were refolded.
#[test]
fn splitting_the_artist_refolds_the_sort_keys() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("Bob Seger-Night moves"), "us/n.kar");

    db.split_artist_from_title(&["a".to_owned()])
        .expect("split");

    let filter = Filter {
        initial: Initial::Letter('N'),
        ..Filter::default()
    };
    let songs = db.songs(&filter).expect("browse");
    assert_eq!(
        songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        ["a"],
        "the song files under the title it now has"
    );
}

/// Paging must not leak into the bulk set: it acts on everything matching, not on one page.
#[test]
fn the_bulk_set_ignores_the_pages_the_list_is_shown_in() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    for n in 0..5 {
        add(
            &mut db,
            &format!("s{n}"),
            Some("Song"),
            &format!("f/{n}.kar"),
        );
    }
    let one_page = Filter {
        limit: 2,
        ..Filter::default()
    };
    assert_eq!(db.songs(&one_page).expect("browse").len(), 2);
    assert_eq!(
        db.set_language_for(&one_page, Language::parse("pt"))
            .expect("bulk set"),
        5,
        "the page is how the list is read, not what the action is over"
    );
}

#[test]
fn sorting_by_language_puts_the_unclassified_last() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_with_language(&mut db, "silent", None, "UTF-8");
    add_with_language(&mut db, "japanese", Some("ENGL"), "Shift_JIS");
    add_with_language(&mut db, "brazilian", Some("PORT"), "windows-1252");

    let rows = db
        .songs(&Filter {
            sort: Sort::Language,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(
        ids(&rows),
        vec!["japanese", "brazilian", "silent"],
        "ja, pt, then the one nobody has classified -- unclassified is not a language that \
         sorts before `ja`"
    );
}

/// One scanned song with only the fact block its kind implies, for the mapping test below.
fn scanned(
    id: &str,
    suitability: crate::model::SuitabilityFacts,
    midi: Option<crate::model::MidiFacts>,
    video: Option<crate::model::VideoFacts>,
    cdg: Option<crate::model::CdgFacts>,
) -> crate::model::ScannedFile {
    crate::model::ScannedFile {
        path: format!("folder/{id}.kar"),
        size: 4_242,
        mtime: 99,
        content_hash: Some(id.to_owned()),
        status: crate::model::ScanStatus::Ok,
        error: None,
        song: Some(crate::model::ScannedSong {
            id: id.to_owned(),
            det_title: Some("Detected Title".to_owned()),
            det_artist: Some("Detected Artist".to_owned()),
            det_language: Some("ENGL".to_owned()),
            stem: "the-stem".to_owned(),
            duration_ms: 234_567,
            lyrics: Some("the words".to_owned()),
            fingerprint: "12:100:0,1".to_owned(),
            suitability,
            midi,
            video,
            cdg,
            ultrastar: None,
        }),
    }
}

/// Every column `write_scanned` writes comes back holding what was put in it.
///
/// **The guard for the one statement in this crate that could be wrong without failing.** The
/// upsert names forty-four columns; it used to bind them by ordinal, and because `first_seen`
/// and `last_scanned` share one value the run read `?22, ?23, ?23, ?24` — so every ordinal
/// after that sat one place to the left of its column. Adding a column meant renumbering the
/// run, the column list, the `DO UPDATE SET` and the `params!` array together, and getting it
/// wrong wrote `det_encoding` into `melody_confidence`: both nullable, so SQLite accepts it,
/// nothing then looked, and the damage was a whole corpus of quietly wrong facts.
///
/// Every value here is **distinct**, the numbers included, so a swap between any two columns
/// changes an answer below. Three rows rather than one because `Db::song` builds each fact
/// block from `kind` rather than from which columns are NULL — deliberately, and for a reason
/// its own comment gives — so one row can only ever prove one block.
#[test]
fn every_scanned_column_comes_back_holding_what_was_put_in_it() {
    let scratch = Scratch::new("scanned-columns");
    let mut db = Db::create(&scratch.0).expect("create");

    let midi_facts = crate::model::MidiFacts {
        flavor: "the-flavor".to_owned(),
        granularity: "the-granularity".to_owned(),
        note_count: 1_001,
        channel_count: 1_002,
        line_count: 1_003,
        syllable_count: 1_004,
        det_encoding: "windows-1252".to_owned(),
        det_encoding_source: "the-source".to_owned(),
        melody_channel: Some(11),
        melody_confidence: Some(0.625),
        melody_abstained: Some("the-reason".to_owned()),
    };
    let suitability_facts = crate::model::SuitabilityFacts {
        value: 9,
        breakdown: (1, 2, 3, 4),
        warnings: r#"["the-warning"]"#.to_owned(),
    };
    let video_facts = crate::model::VideoFacts {
        width: 2_001,
        height: 2_002,
        frame_rate_milli: 2_003,
        video_codec: "the-video-codec".to_owned(),
        audio_codec: "the-audio-codec".to_owned(),
    };
    let cdg_facts = crate::model::CdgFacts {
        graphics_path: "the/graphics.cdg".to_owned(),
        sample_rate: 3_001,
        channels: 3_002,
        packets: 3_003,
        graphics_ms: 3_004,
        graphics_short_by_ms: 3_005,
        tiles_written: 3_006,
        unknown_instructions: 3_007,
    };

    db.write_scanned(
        &[
            scanned("midi-hash", suitability_facts, Some(midi_facts), None, None),
            scanned(
                "video-hash",
                crate::model::SuitabilityFacts {
                    value: 10,
                    breakdown: (3, 3, 2, 2),
                    warnings: "[]".to_owned(),
                },
                None,
                Some(video_facts),
                None,
            ),
            scanned(
                "cdg-hash",
                crate::model::SuitabilityFacts {
                    value: 4,
                    breakdown: (0, 0, 2, 2),
                    warnings: r#"[{"code":"briefsinging","message":"m"}]"#.to_owned(),
                },
                None,
                None,
                Some(cdg_facts),
            ),
        ],
        "2026-09-07T00:00:00Z",
    )
    .expect("write");

    // The columns every song has, on the row that also carries the MIDI ones.
    let got = db.song("midi-hash").expect("read back");
    assert_eq!(got.det_title.as_deref(), Some("Detected Title"));
    assert_eq!(got.det_artist.as_deref(), Some("Detected Artist"));
    assert_eq!(got.det_language.as_deref(), Some("ENGL"));
    // Derived inside the statement's own parameter list rather than carried on the song, so it
    // is as much part of this mapping as anything read straight off a field.
    assert_eq!(got.det_language_tag.as_deref(), Some("en"));
    assert_eq!(got.stem.as_deref(), Some("the-stem"));
    assert_eq!(got.duration_ms, 234_567);
    assert_eq!(got.kind, SongKind::Midi);

    let midi = got.midi.expect("the MIDI facts");
    assert_eq!(midi.flavor, "the-flavor");
    assert_eq!(midi.granularity, "the-granularity");
    assert_eq!(midi.note_count, 1_001);
    assert_eq!(midi.channel_count, 1_002);
    assert_eq!(midi.line_count, 1_003);
    assert_eq!(midi.syllable_count, 1_004);
    assert_eq!(midi.det_encoding, "windows-1252");
    assert_eq!(midi.det_encoding_source, "the-source");
    assert_eq!(midi.melody_channel, Some(11));
    assert_eq!(midi.melody_confidence, Some(0.625));
    assert_eq!(midi.melody_abstained.as_deref(), Some("the-reason"));
    assert_eq!(got.suitability.suitability, 9);
    assert_eq!(got.suitability.suitability_lyrics, 1);
    assert_eq!(got.suitability.suitability_sync, 2);
    assert_eq!(got.suitability.suitability_channels, 3);
    assert_eq!(got.suitability.suitability_arrangement, 4);

    let got = db.song("video-hash").expect("read back");
    assert_eq!(got.kind, SongKind::Video);
    // Written for a media song too, and read back off the same columns: a browse list, the band
    // filter and the sort all read this one number.
    assert_eq!(got.suitability.suitability, 10);
    assert_eq!(got.suitability.suitability_arrangement, 2);
    let video = got.video.expect("the video facts");
    assert_eq!(video.width, 2_001);
    assert_eq!(video.height, 2_002);
    assert_eq!(video.frame_rate_milli, 2_003);
    assert_eq!(video.video_codec, "the-video-codec");
    assert_eq!(video.audio_codec, "the-audio-codec");

    let got = db.song("cdg-hash").expect("read back");
    assert_eq!(got.kind, SongKind::Cdg);
    let cdg = got.cdg.expect("the CD+G facts");
    assert_eq!(cdg.graphics_path, "the/graphics.cdg");
    assert_eq!(cdg.sample_rate, 3_001);
    assert_eq!(cdg.channels, 3_002);
    assert_eq!(cdg.packets, 3_003);
    assert_eq!(cdg.graphics_ms, 3_004);
    assert_eq!(cdg.graphics_short_by_ms, 3_005);
    assert_eq!(cdg.tiles_written, 3_006);
    assert_eq!(cdg.unknown_instructions, 3_007);
}

/// A database this build made is stamped, so the next open does no migration work at all.
#[test]
fn a_database_this_build_made_carries_its_schema_version() {
    let scratch = Scratch::new("stamped");
    let db = Db::create(&scratch.0).expect("create");
    assert_eq!(
        db.pragma_for_test("user_version").expect("pragma"),
        SCHEMA_VERSION.to_string(),
        "a fresh database must be stamped, or the next open refuses it as unstamped"
    );
}

/// A database at schema 14 climbs every step to the current schema and keeps what it held.
///
/// **One test over the whole ladder rather than one per rung**, because what it guards is that a
/// database in the field opens at all: a step that adds a column the schema already creates fails
/// with `duplicate column name` and takes the whole open with it, and only a database that
/// genuinely predates the step can catch that.
#[test]
fn a_database_at_schema_14_steps_to_the_current_schema() {
    let scratch = Scratch::new("schema-14");
    {
        let db = Db::create(&scratch.0).expect("create");
        db.create_package(
            &PackageRow::new("1f4a9c8e2b7d0356", "Brasil"),
            "2026-09-18T00:00:00Z",
        )
        .expect("a package");
        // Everything every step above 14 adds, taken back off in one go — and the browse index
        // whose key names one of those columns put back the way a schema-14 build wrote it, since
        // an expression index cannot outlive a column it reads.
        db.execute_for_test(
            // The stamp trigger names the hand-set columns, so a column it watches cannot be
            // dropped underneath it. Every trigger is dropped and recreated on open, so taking it
            // off here costs nothing and is what a schema-14 database would have had anyway.
            //
            // Every browse index is partial on `deleted_at` as well, and SQLite refuses to drop a
            // column any index names — so all ten go, and the one this test is about is put back in
            // the shape a schema-14 build wrote it in. `create_browse_indexes` rebuilds the rest on
            // the open below, which is exactly what a database in the field gets.
            "DROP TRIGGER IF EXISTS songs_stamp_update;
             DROP INDEX IF EXISTS songs_browse_title_artist;
             DROP INDEX IF EXISTS songs_browse_letter_artist;
             DROP INDEX IF EXISTS songs_browse_suitability_artist;
             DROP INDEX IF EXISTS songs_browse_sort_artist;
             DROP INDEX IF EXISTS songs_browse_language_artist;
             DROP INDEX IF EXISTS songs_browse_user_score_artist;
             DROP INDEX IF EXISTS songs_browse_duration;
             DROP INDEX IF EXISTS songs_browse_copies;
             DROP INDEX IF EXISTS songs_browse_updated_artist;
             DROP INDEX IF EXISTS songs_browse_added_artist;
             DROP INDEX IF EXISTS songs_countable;
             DROP INDEX IF EXISTS songs_deleted;
             ALTER TABLE packages DROP COLUMN number_one_volume;
             ALTER TABLE songs DROP COLUMN det_language_guess;
             ALTER TABLE songs DROP COLUMN det_language_guess_confidence;
             ALTER TABLE songs DROP COLUMN lyrics_hidden;
             ALTER TABLE songs DROP COLUMN deleted_at;
             -- The narrow shape a schema-14 build wrote, so the step's `DROP INDEX` has the index
             -- it exists to replace rather than a wider one already in place.
             CREATE INDEX songs_countable ON songs(merged_into, duplicate_of);
             CREATE INDEX songs_browse_language_artist ON songs(
                 coalesce(nullif(language, ''), det_language_tag) IS NULL,
                 coalesce(nullif(language, ''), det_language_tag),
                 sort_artist IS NULL, sort_artist, sort_title, id)
               WHERE merged_into IS NULL;
             PRAGMA user_version = 14;",
        )
        .expect("put it back at schema 14");
    }

    let db = Db::open(&scratch.0).expect("a schema-14 database opens");
    assert_eq!(
        db.pragma_for_test("user_version").expect("pragma"),
        SCHEMA_VERSION.to_string()
    );
    let volume = db
        .package_volume("1f4a9c8e2b7d0356", 1)
        .expect("the package survives the step");
    assert!(!volume.number_one_volume);
    assert_eq!(volume.volume_name(), "Brasil");
    // The guessed-language columns are back, and empty, which is what a row nothing has read says.
    db.execute_for_test(
        "SELECT det_language_guess, det_language_guess_confidence FROM songs LIMIT 1",
    )
    .expect("the step added both columns");
    // And the words column, which starts NULL on every row — nobody has said, so a corpus already
    // curated gains the question without gaining an answer to it.
    db.execute_for_test("SELECT lyrics_hidden FROM songs LIMIT 1")
        .expect("the step added the words column");
    // And the browse index built on the old two-leg key was rebuilt on the current one. Left alone
    // it would keep its name and its old key, so the browse page would match no index and sort the
    // whole corpus with nothing saying so.
    let key = db
        .index_sql_for_test("songs_browse_language_artist")
        .expect("the index is there");
    assert!(
        key.contains("det_language_guess"),
        "the language index still holds a key that predates the guess: {key}"
    );
    // And its partial predicate was widened with it. A browse index still partial on `merged_into`
    // alone cannot serve a query that also asks `deleted_at IS NULL`, so the planner would walk
    // away from it and sort the corpus instead — silently, which is what this whole rebuild guards.
    assert!(
        key.contains("deleted_at"),
        "the language index is partial on a predicate that predates deleting: {key}"
    );
    // The count's index too, which is dropped by the step rather than by the index rebuild: it
    // lives in `schema.sql` under `IF NOT EXISTS`, so without the drop a database in the field
    // would keep the two-column shape for ever and the count would read every row.
    let countable = db
        .index_sql_for_test("songs_countable")
        .expect("the index is there");
    assert!(
        countable.contains("deleted_at"),
        "the count's index kept the shape that predates deleting: {countable}"
    );
}

/// A database written by a newer build is refused rather than opened.
///
/// **This is the whole reason the version exists.** SQLite will happily hand back rows from a
/// table with columns this build has never heard of, so without the stamp a `.kmbuild` from a
/// newer build would open silently and the tool would curate a corpus while ignoring whatever that
/// build had added.
#[test]
fn a_database_from_a_newer_build_is_refused_rather_than_half_understood() {
    let scratch = Scratch::new("from-the-future");
    {
        let db = Db::create(&scratch.0).expect("create");
        db.execute_for_test(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
            .expect("stamp it from the future");
    }

    // `expect_err` would want `Db: Debug`, which it deliberately is not — it holds a
    // `Connection`. The match says the same thing without asking for one.
    let said = match Db::open(&scratch.0) {
        Ok(_) => panic!("a database from a newer build must not open"),
        Err(error) => error.to_string(),
    };
    assert!(
        said.contains("newer build"),
        "the refusal has to say what is wrong: {said}"
    );
    assert!(
        said.contains(&(SCHEMA_VERSION + 1).to_string()),
        "and name both versions: {said}"
    );
}

/// A database below the oldest schema this build opens is refused, and so is one with no stamp.
///
/// **A `.kmbuild` holds months of hand curation**, so an open that read one as a shape it does not
/// have would go on to write over it. Both numbers are in the refusal. An unstamped file with a
/// `songs` table is the other case: nothing says what shape it has, so it is refused the same way —
/// and a brand-new file, which is also unstamped, is not, because it has no `songs` table yet.
#[test]
fn a_database_below_the_oldest_schema_is_refused_by_its_number() {
    for (name, stamp) in [("older", OLDEST_SCHEMA_VERSION - 1), ("unstamped", 0)] {
        let scratch = Scratch::new(name);
        {
            let db = Db::create(&scratch.0).expect("create");
            db.execute_for_test(&format!("PRAGMA user_version = {stamp};"))
                .expect("stamp it older");
        }

        let said = match Db::open(&scratch.0) {
            Ok(_) => panic!("{name}: a database below the oldest schema must not open"),
            Err(error) => error.to_string(),
        };
        assert!(
            said.contains(&format!("schema {stamp}")),
            "{name}: the refusal has to name what it found: {said}"
        );
        assert!(
            said.contains(&OLDEST_SCHEMA_VERSION.to_string()),
            "{name}: and what this build opens: {said}"
        );
    }
}

/// The limit takes the first rows of the order, not an arbitrary slice of the match set.
///
/// **The property that makes the limit safe.** `package_from_filter` asks for `MAX_SLOT` ids
/// because that is all `add_to_package` can number — and the whole point of the `ORDER BY` is
/// that somebody who sorted the browse list before pressing the button meant that order. A
/// `LIMIT` that ignored it would build a different package from the same press.
#[test]
fn a_limited_match_takes_the_front_of_the_order() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "c", Some("Cherry"), "c.kar");
    add(&mut db, "a", Some("Apple"), "a.kar");
    add(&mut db, "b", Some("Banana"), "b.kar");

    let by_title = Filter {
        sort: Sort::Title,
        ..Filter::default()
    };
    assert_eq!(
        db.matching_ids(&by_title, None).expect("all"),
        ["a", "b", "c"]
    );
    assert_eq!(
        db.matching_ids(&by_title, Some(2)).expect("limited"),
        ["a", "b"],
        "a limit has to take the front of the order, not any two rows"
    );
    assert_eq!(
        db.matching_ids(&by_title, Some(99)).expect("over-asking"),
        ["a", "b", "c"],
        "asking for more than there are is every match, not an error"
    );
}

/// Filing a list of songs into a favorite is one transaction, and files all of them.
#[test]
fn a_batch_of_songs_is_filed_into_a_favorite_at_once() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("A"), "a.kar");
    add(&mut db, "b", Some("B"), "b.kar");
    add(&mut db, "c", Some("C"), "c.kar");
    let bossa = db.create_favorite("Bossa").expect("favorite");

    let songs = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    assert_eq!(db.set_favorites(&songs, bossa, true).expect("file them"), 3);
    assert_eq!(
        db.set_favorites(&songs, bossa, true).expect("file again"),
        0,
        "filing songs already in it writes nothing, which is success and not a fault"
    );
    for id in &songs {
        assert!(
            db.favorites_for(id)
                .expect("read back")
                .iter()
                .any(|(favorite, _)| *favorite == bossa),
            "{id} should be in the favorite"
        );
    }

    assert_eq!(
        db.set_favorites(&songs, bossa, false)
            .expect("take them out"),
        3
    );
    for id in &songs {
        assert!(
            db.favorites_for(id).expect("read back").is_empty(),
            "{id} should have left the favorite"
        );
    }
}

/// A filter files its whole match into a favorite, and takes the same match back out.
///
/// The `WHERE` is `Filter::to_sql` verbatim, so what this proves is that the write lands on exactly
/// the songs the list would have shown — the invariant every filter-wide write in this file rests on.
#[test]
fn a_filter_files_its_whole_match_into_a_favorite_and_takes_it_back() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add(&mut db, "a", Some("Corcovado"), "brasil/a.kar");
    add(&mut db, "b", Some("Desafinado"), "brasil/b.kar");
    add(&mut db, "c", Some("Yesterday"), "ingles/c.kar");
    let bossa = db.create_favorite("Bossa").expect("favorite");

    let brasil = Filter {
        folder: Some("brasil/".to_owned()),
        ..Default::default()
    };
    assert_eq!(db.set_favorites_for(&brasil, bossa, true).expect("file"), 2);
    assert_eq!(
        db.set_favorites_for(&brasil, bossa, true).expect("again"),
        0,
        "a song already in it is not written twice"
    );
    assert!(
        db.favorites_for("c").expect("read back").is_empty(),
        "the song the filter does not match is left alone"
    );

    assert_eq!(
        db.set_favorites_for(&brasil, bossa, false).expect("unfile"),
        2
    );
    for id in ["a", "b"] {
        assert!(
            db.favorites_for(id).expect("read back").is_empty(),
            "{id} should have left the favorite"
        );
    }
}

/// A verse in a language whose alphabet places it, for the tests about reading a song's words.
///
/// Invented rather than taken from a song, so a fixture carries no licence with it, and long enough
/// to be what a lyric track is rather than what a title is.
const VIETNAMESE_VERSE: &str = "Buổi sáng đi ngang con đường vắng một lần nữa, mỗi khung cửa sổ \
                                giữ một gương mặt không quay lại nhìn tôi";

/// A song scanned with a declared language and an encoding, for the language tests.
///
/// Goes through `write_scanned` like every other scan, so what is being tested is the real path
/// rather than the backfill.
fn add_with_language(db: &mut Db, id: &str, declared: Option<&str>, encoding: &str) {
    add_scanned(db, id, |song| {
        song.det_language = declared.map(ToOwned::to_owned);
        if let Some(midi) = song.midi.as_mut() {
            midi.det_encoding = encoding.to_owned();
        }
    });
}

/// The detected code stored for a song.
fn tag(db: &Db, id: &str) -> Option<String> {
    db.song(id).expect("detail").det_language_tag
}

// -- clustering ---------------------------------------------------------------------------

/// Suggests `a` and `b` as a pair, the way a suggestion pass would.
/// Leaves one suggested pair in the table, as a pass that proposed it would.
///
/// A direct insert rather than [`Db::store_candidates`], because that call is a whole pass: what it
/// is handed replaces every unjudged row, so two of these in a row would leave only the second pair.
/// These tests are about what grouping does with pairs; `store_candidates` has its own below.
fn suggest_pair(db: &mut Db, a: &str, b: &str) {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    db.conn
        .execute(
            "INSERT INTO duplicate_candidates(a_id, b_id, similarity, reason)
             VALUES (?1, ?2, 0.9, 'same shape')
             ON CONFLICT(a_id, b_id) DO NOTHING",
            params![a, b],
        )
        .expect("store");
}

/// One whole suggestion pass, proposing exactly these pairs and nothing else.
fn suggestion_pass(db: &mut Db, pairs: &[(&str, &str)]) -> usize {
    let proposed: Vec<(String, String, f32, String)> = pairs
        .iter()
        .map(|(a, b)| {
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            (
                (*a).to_owned(),
                (*b).to_owned(),
                0.9,
                "same shape".to_owned(),
            )
        })
        .collect();
    db.store_candidates(&proposed).expect("store")
}

#[test]
fn a_pass_that_stops_proposing_a_pair_takes_it_out_of_the_table() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    assert_eq!(
        suggestion_pass(&mut db, &[("aaa", "bbb"), ("bbb", "ccc")]),
        2
    );
    assert_eq!(db.cluster().expect("cluster").clusters, 1);

    // The files changed under it, and the second pair is no longer one. Left in the table it would
    // go on holding `ccc` in the group, because grouping reads every undismissed pair rather than
    // the ones a pass has just written.
    assert_eq!(suggestion_pass(&mut db, &[("aaa", "bbb")]), 1);
    let counts = db.cluster().expect("recluster");
    assert_eq!(counts.clusters, 1);
    assert_eq!(
        counts.set_aside, 1,
        "only the pair still proposed is a group"
    );
    assert_eq!(
        db.songs(&Filter::default()).expect("songs").len(),
        2,
        "the song nothing pairs any more is browsable on its own"
    );
}

#[test]
fn a_pass_that_stops_proposing_a_dismissed_pair_leaves_the_verdict() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    suggestion_pass(&mut db, &[("aaa", "bbb")]);
    db.dismiss_pair("aaa", "bbb").expect("dismiss");

    // What a person said is not this pass's to withdraw, and a verdict thrown away here would be a
    // dismissal that lasted until the next pass stopped proposing the pair.
    assert_eq!(
        suggestion_pass(&mut db, &[]),
        0,
        "a pair already judged is not stored again"
    );
    let verdict: Option<String> = db
        .conn
        .query_row(
            "SELECT verdict FROM duplicate_candidates WHERE a_id = 'aaa' AND b_id = 'bbb'",
            [],
            |row| row.get(0),
        )
        .expect("read");
    assert_eq!(verdict.as_deref(), Some("different"));
}

#[test]
fn a_chain_of_suggested_pairs_becomes_one_cluster() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");

    let counts = db.cluster().expect("cluster");
    assert_eq!(counts.clusters, 1, "three songs, two pairs, one cluster");
    assert_eq!(counts.set_aside, 2, "one representative survives");

    let rows = db.songs(&Filter::default()).expect("songs");
    assert_eq!(rows.len(), 1, "the browse list collapses to the survivor");
    assert_eq!(rows[0].version_count, 3);
}

#[test]
fn a_dismissed_pair_splits_the_cluster_it_joined() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.cluster().expect("cluster");
    assert_eq!(db.songs(&Filter::default()).expect("songs").len(), 1);

    // The link in the middle is the one somebody says is wrong, so the chain has to fall in two.
    db.dismiss_pair("bbb", "ccc").expect("dismiss");
    let counts = db.cluster().expect("recluster");
    assert_eq!(counts.clusters, 1, "aaa and bbb are still a pair");
    assert_eq!(counts.set_aside, 1);
    assert_eq!(
        db.songs(&Filter::default()).expect("songs").len(),
        2,
        "the song a dismissal separated is browsable again"
    );
}

#[test]
fn the_representative_is_the_best_file_and_stays_the_same_one() {
    let mut db = db();
    // Same shape, same title: what `compare` needs to pair them. The suitability is what differs.
    for (id, suitability) in [("aaa", 4u8), ("bbb", 9), ("ccc", 4)] {
        add_built(
            &mut db,
            id,
            Some("One Song"),
            &format!("f/{id}.kar"),
            |song| {
                song.suitability.value = suitability;
            },
        );
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.cluster().expect("cluster");

    let rows = db.songs(&Filter::default()).expect("songs");
    assert_eq!(ids(&rows), vec!["bbb"], "the 9 is what a curator is shown");

    // Twice, because a representative that moves between runs is a browse list that reshuffles for
    // a reason nobody can see.
    db.cluster().expect("recluster");
    let rows = db.songs(&Filter::default()).expect("songs");
    assert_eq!(ids(&rows), vec!["bbb"]);
}

/// A `.kmbuild` is a document somebody may have been sent, so a path in it is a claim.
///
/// Every route that reaches a file goes through `best_file` — playing, downloading, revealing in the
/// system opener, and uploading to the machine the same database names — so a path escaping the
/// corpus folder here reads and sends any file the tool can open.
#[test]
fn a_stored_path_that_leaves_the_corpus_folder_is_refused() {
    // Both separators, both kinds of absolute, a climb out of a real subfolder, and the NTFS
    // alternate-data-stream spelling that reaches a second file under a first one's name.
    let escapes = [
        "../../../../etc/passwd",
        "..\\..\\..\\Windows\\win.ini",
        "/etc/shadow",
        "C:/Windows/System32/config/SAM",
        "D:\\tunes\\evil.kar",
        "songs/../../../outside.kar",
        "song.kar:stream",
    ];

    for escape in escapes {
        let mut db = db();
        add(&mut db, "aaa", Some("One Song"), escape);

        let Err(error) = db.best_file("aaa") else {
            panic!("{escape:?} must not be joined onto the corpus root");
        };
        assert!(
            error
                .to_string()
                .contains("not a path below the corpus folder"),
            "{escape:?} was refused for the wrong reason: {error}"
        );
    }
}

/// The other half of the rule above: an ordinary path still resolves, and below the root.
#[test]
fn an_ordinary_stored_path_still_resolves_below_the_root() {
    let mut db = db();
    add(&mut db, "aaa", Some("One Song"), "folder/deeper/song.kar");

    let (_, path) = db.best_file("aaa").expect("an ordinary path resolves");
    assert_eq!(path, Path::new("/corpus").join("folder/deeper/song.kar"));
    assert!(
        path.starts_with("/corpus"),
        "the resolved path must stay under the corpus root: {}",
        path.display()
    );
}

#[test]
fn every_version_shows_what_collapsing_hid() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");

    let all = Filter {
        versions: VersionsFilter::All,
        ..Filter::default()
    };
    assert_eq!(db.songs(&all).expect("songs").len(), 2);
    assert_eq!(db.songs(&Filter::default()).expect("songs").len(), 1);
}

#[test]
fn a_list_shows_the_versions_filed_in_it() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");

    let shown = db.songs(&Filter::default()).expect("songs");
    assert_eq!(shown.len(), 1, "the corpus still collapses");
    let hidden = if shown[0].id == "aaa" { "bbb" } else { "aaa" };

    let pop = db.create_favorite("Pop").expect("favorite");
    db.set_favorite(hidden, pop, true).expect("file it");

    let list = Filter {
        favorite: Some(pop),
        ..Filter::default()
    };
    let rows = db.songs(&list).expect("songs");
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        [hidden],
        "a list shows the version filed in it"
    );
    assert_eq!(db.song_count(&list).expect("count") as usize, rows.len());
    let counted = db.favorites().expect("favorites")[0].song_count;
    assert_eq!(counted as usize, rows.len(), "the Favorites page agrees");
}

#[test]
fn a_hidden_version_carries_its_groups_count_and_names_the_shown_one() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");

    let all = Filter {
        versions: VersionsFilter::All,
        ..Filter::default()
    };
    let rows = db.songs(&all).expect("songs");
    let shown = db.songs(&Filter::default()).expect("songs");
    let representative = shown
        .iter()
        .find(|row| row.id != "ccc")
        .expect("the group's shown version");

    for row in &rows {
        match row.id.as_str() {
            "ccc" => {
                assert_eq!(row.version_count, 1, "a song in no group");
                assert_eq!(row.duplicate_of, None);
            }
            id if id == representative.id => {
                assert_eq!(row.version_count, 2);
                assert_eq!(
                    row.duplicate_of, None,
                    "the shown version hides behind nothing"
                );
            }
            _ => {
                assert_eq!(
                    row.version_count, 2,
                    "a hidden version counts its group too"
                );
                assert_eq!(
                    row.duplicate_of.as_deref(),
                    Some(representative.id.as_str())
                );
            }
        }
    }
}

#[test]
fn the_count_agrees_with_the_rows_a_collapsed_page_can_reach() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc", "ddd"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");

    // The pager reads `song_count` and the page reads `songs_page`. A count taken through a
    // different `WHERE` than the rows offers a page that comes back empty.
    let filter = Filter::default();
    let counted = db.song_count(&filter).expect("count");
    let rows = db.songs(&filter).expect("songs");
    assert_eq!(counted as usize, rows.len());
    assert_eq!(counted, 3, "four songs, one of them set aside");
}

#[test]
fn a_merged_song_never_joins_a_cluster() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    // `bbb` is somebody's own decision, and a guess must not be layered on top of it.
    db.set_merged_into("bbb", Some("aaa")).expect("merge");

    let counts = db.cluster().expect("cluster");
    assert_eq!(counts.clusters, 0, "both pairs ran through the merged song");
    assert_eq!(counts.set_aside, 0);
}

#[test]
fn releasing_one_song_leaves_the_rest_of_its_cluster_alone() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.cluster().expect("cluster");

    let hidden: Vec<String> = db
        .songs(&Filter {
            versions: VersionsFilter::All,
            ..Filter::default()
        })
        .expect("songs")
        .into_iter()
        .map(|row| row.id)
        .filter(|id| id != &db.songs(&Filter::default()).expect("songs")[0].id)
        .collect();
    db.release_from_cluster(&hidden[0]).expect("release");

    assert_eq!(
        db.songs(&Filter::default()).expect("songs").len(),
        2,
        "the released song browses; the other stays set aside"
    );
}

#[test]
fn a_cluster_that_loses_its_pairs_releases_its_songs() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");
    assert_eq!(db.songs(&Filter::default()).expect("songs").len(), 1);

    // Every pass rewrites the clusters from nothing, so a song whose group is gone comes back
    // rather than staying hidden by a grouping nobody can find.
    db.conn
        .execute("DELETE FROM duplicate_candidates", [])
        .expect("clear");
    let counts = db.cluster().expect("recluster");
    assert_eq!(counts.clusters, 0);
    assert_eq!(db.songs(&Filter::default()).expect("songs").len(), 2);
}

#[test]
fn versions_of_names_the_other_files_and_not_the_song_asked_about() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.cluster().expect("cluster");

    // Asked of the representative and of a song set aside, the answer names the same cluster.
    for id in ["aaa", "bbb", "ccc"] {
        let others = db.versions_of(id).expect("versions");
        assert_eq!(others.len(), 2, "{id} is one of three");
        assert!(!ids(&others).contains(&id), "{id} must not list itself");
    }
}

#[test]
fn a_dismissal_between_two_songs_no_pass_proposed_is_still_recorded() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    // A star: `aaa` is paired with each of the others and the two leaves with nothing. This is what
    // the lyric pass emits, and the song page offers *Not the same* between the two leaves anyway.
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "aaa", "ccc");
    db.cluster().expect("cluster");
    assert_eq!(db.songs(&Filter::default()).expect("songs").len(), 1);

    db.dismiss_pair("bbb", "ccc").expect("dismiss");
    let recorded: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM duplicate_candidates
             WHERE a_id = 'bbb' AND b_id = 'ccc' AND verdict = 'different'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        recorded, 1,
        "no pair existed to update, so one has to be made"
    );

    // And it holds: `aaa` cannot carry the two back together.
    db.cluster().expect("recluster");
    let browsable = db.songs(&Filter::default()).expect("songs").len();
    assert!(
        browsable >= 2,
        "the two told apart must not share a group, {browsable} rows"
    );
}

#[test]
fn a_dismissal_inside_a_clique_is_not_undone_by_the_third_song() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    // A clique, which is what the fingerprint pass emits: every pair of the three exists, so
    // dismissing one of them leaves a path through the other two.
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    suggest_pair(&mut db, "aaa", "ccc");
    db.cluster().expect("cluster");
    assert_eq!(db.songs(&Filter::default()).expect("songs").len(), 1);

    db.dismiss_pair("aaa", "bbb").expect("dismiss");
    db.cluster().expect("recluster");

    let together: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM songs a, songs b
             WHERE a.id = 'aaa' AND b.id = 'bbb'
               AND coalesce(a.duplicate_of, a.id) = coalesce(b.duplicate_of, b.id)",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        together, 0,
        "the third song must not carry them back together"
    );
}

#[test]
fn a_dismissal_survives_the_pass_that_suggests_the_pair_again() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    suggestion_pass(&mut db, &[("aaa", "bbb")]);
    db.dismiss_pair("aaa", "bbb").expect("dismiss");

    // The suggester proposes it again, as it will on every run. A row carrying a verdict is left
    // exactly as it is, so what a person said outlives what the machine keeps finding.
    suggestion_pass(&mut db, &[("aaa", "bbb")]);
    let verdict: Option<String> = db
        .conn
        .query_row(
            "SELECT verdict FROM duplicate_candidates WHERE a_id = 'aaa' AND b_id = 'bbb'",
            [],
            |row| row.get(0),
        )
        .expect("read");
    assert_eq!(verdict.as_deref(), Some("different"));

    let counts = db.cluster().expect("cluster");
    assert_eq!(counts.clusters, 0, "a dismissed pair is not a group");
}

#[test]
fn a_dismissal_takes_only_the_pair_it_names() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.dismiss_pair("aaa", "bbb").expect("dismiss");
    db.cluster().expect("cluster");

    // `bbb` and `ccc` were never told apart, so they are still one recording as far as anybody said.
    let together: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM songs a, songs b
             WHERE a.id = 'bbb' AND b.id = 'ccc'
               AND coalesce(a.duplicate_of, a.id) = coalesce(b.duplicate_of, b.id)",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(together, 1, "only the named pair is separated");
}

/// A song somebody threw away is in no cluster, as head or as member.
///
/// **Both halves matter, because they fail at different moments.** A deleted song the pass can
/// still see wins the head on suitability and hides every live copy behind a row no list draws. A
/// song deleted *after* the pass leaves a cluster whose head has gone, which takes the rest of the
/// cluster off the page with it.
#[test]
fn a_deleted_song_neither_heads_a_cluster_nor_hides_one() {
    let mut db = db();
    // The best file of the three, so it wins the head wherever it is still in the running.
    for (id, suitability) in [("aaa", 9u8), ("bbb", 4), ("ccc", 4)] {
        add_built(
            &mut db,
            id,
            Some("One Song"),
            &format!("f/{id}.kar"),
            |song| {
                song.suitability.value = suitability;
            },
        );
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");

    // Deleted before the pass: it is not in the fingerprints and not in the pairs, so the head is
    // the best of what is left.
    assert_eq!(
        db.set_deleted_of(&["aaa".to_owned()], true)
            .expect("delete"),
        1
    );
    assert!(
        db.fingerprints()
            .expect("fingerprints")
            .iter()
            .all(|print| print.id != "aaa"),
        "a song thrown away is not offered to the duplicate pass"
    );
    db.cluster().expect("cluster");
    let rows = db.songs(&Filter::default()).expect("songs");
    assert_eq!(ids(&rows), vec!["bbb"], "the best of the two that are left");

    // Deleted after the pass: the head goes, and the song it was hiding comes back rather than
    // going with it.
    assert_eq!(
        db.set_deleted_of(&["bbb".to_owned()], true)
            .expect("delete"),
        1
    );
    let rows = db.songs(&Filter::default()).expect("songs");
    assert_eq!(
        ids(&rows),
        vec!["ccc"],
        "the last live copy is on the page rather than behind a head that has gone"
    );
}

#[test]
fn songs_with_the_same_words_group_although_no_name_matches() {
    let mut db = db();
    // The shape pass cannot reach these: `compare` refuses a match no title or artist confirms,
    // and a real corpus is full of files called exactly this.
    for id in ["EARTHW~2", "FANTAZY"] {
        add_built(&mut db, id, Some(id), &format!("f/{id}.kar"), |song| {
            song.lyrics = Some("one two three four five six seven eight nine ten eleven twelve                                 thirteen fourteen fifteen sixteen seventeen eighteen nineteen                                 twenty twentyone twentytwo twentythree twentyfour twentyfive"
                .to_owned());
        });
    }
    let prints = db.fingerprints().expect("fingerprints");
    let pairs = crate::dupes::suggest(&prints);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].3, "same words");

    db.store_candidates(&pairs).expect("store");
    let counts = db.cluster().expect("cluster");
    assert_eq!(counts.clusters, 1);
    assert_eq!(counts.set_aside, 1);
    assert_eq!(
        db.songs(&Filter::default()).expect("songs").len(),
        1,
        "the song list shows one of the two"
    );
}

#[test]
fn a_favorite_counts_the_entries_that_are_second_copies() {
    let mut db = db();
    for id in ["aaa", "bbb", "ccc"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    add(&mut db, "zzz", Some("Another"), "f/zzz.kar");
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.cluster().expect("cluster");

    let list = db.create_favorite("Party").expect("create");
    for id in ["aaa", "bbb", "ccc", "zzz"] {
        db.set_favorite(id, list, true).expect("star");
    }

    let node = db
        .favorites()
        .expect("favorites")
        .into_iter()
        .find(|f| f.id == list)
        .expect("the list");
    assert_eq!(node.song_count, 4);
    // Four entries, two recordings: three files of one song and one of another.
    assert_eq!(node.second_copies, 2);
}

#[test]
fn tidying_a_favorite_keeps_the_best_copy_that_list_actually_holds() {
    let mut db = db();
    for (id, suitability) in [("aaa", 3u8), ("bbb", 9), ("ccc", 6)] {
        add_built(
            &mut db,
            id,
            Some("One Song"),
            &format!("f/{id}.kar"),
            |song| {
                song.suitability.value = suitability;
            },
        );
    }
    suggest_pair(&mut db, "aaa", "bbb");
    suggest_pair(&mut db, "bbb", "ccc");
    db.cluster().expect("cluster");

    // The best file of the three is deliberately left out of the list, so what survives has to be
    // the best one the list *holds* rather than the cluster's own representative.
    let list = db.create_favorite("Party").expect("create");
    for id in ["aaa", "ccc"] {
        db.set_favorite(id, list, true).expect("star");
    }

    assert_eq!(db.tidy_favorite(list).expect("tidy"), 1);
    let kept = db.favorites_for("ccc").expect("for ccc");
    assert_eq!(kept.len(), 1, "the 6 stayed and the 3 went");
    assert!(db.favorites_for("aaa").expect("for aaa").is_empty());
}

/// Tidying keeps a copy the list can show, never the discarded one.
///
/// A star stays on a song somebody throws away. Without the term the discarded file can win on
/// suitability and take every live copy of that recording out of the list, which leaves the
/// recording represented by an entry no page draws.
///
/// **The state is set by hand because the writers already keep it from arising.** A clustering pass
/// never gives the head to a deleted song, and `release_behind_hidden` dissolves a group whose head
/// is deleted afterwards. What is left is a member whose suitability was raised after the pass that
/// grouped it, so this asserts the term rather than a route to it.
#[test]
fn tidying_a_favorite_never_keeps_the_copy_that_was_thrown_away() {
    let mut db = db();
    for (id, suitability) in [("head", 6u8), ("raised", 9), ("poor", 3)] {
        add_built(
            &mut db,
            id,
            Some("One Song"),
            &format!("f/{id}.kar"),
            |song| {
                song.suitability.value = suitability;
            },
        );
    }
    // One group of three headed by the live `head`, with the best file set aside under it.
    db.execute_for_test("UPDATE songs SET duplicate_of = 'head' WHERE id IN ('raised', 'poor')")
        .expect("group them");
    db.execute_for_test("UPDATE songs SET deleted_at = '2026-09-19T00:00:00Z' WHERE id = 'raised'")
        .expect("throw the best one away");

    let list = db.create_favorite("Party").expect("create");
    for id in ["head", "raised", "poor"] {
        db.set_favorite(id, list, true).expect("star");
    }

    assert_eq!(db.tidy_favorite(list).expect("tidy"), 2);
    assert_eq!(
        db.favorites_for("head").expect("for head").len(),
        1,
        "the survivor is the best copy the list can show"
    );
    assert!(
        db.favorites_for("raised").expect("for raised").is_empty(),
        "the discarded copy is a second copy like any other and goes with them"
    );
    assert!(
        db.favorites_for("poor").expect("for poor").is_empty(),
        "and so does the poorer live one"
    );
}

#[test]
fn tidying_a_favorite_of_different_songs_removes_nothing() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some(id), &format!("f/{id}.kar"));
    }
    let list = db.create_favorite("Party").expect("create");
    for id in ["aaa", "bbb"] {
        db.set_favorite(id, list, true).expect("star");
    }
    assert_eq!(db.tidy_favorite(list).expect("tidy"), 0);
}

#[test]
fn adding_a_second_version_to_a_package_warns_and_still_adds() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");

    let package = "vol1".to_owned();
    db.create_package(
        &PackageRow {
            id: package.clone(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-09-10T00:00:00Z",
    )
    .expect("create");
    let first = db
        .add_to_package(&package, &["aaa".to_owned()], "2026-09-10T00:00:00Z")
        .expect("add");
    assert_eq!((first.added, first.clashed), (1, 0));

    // A warning and never a refusal: an acoustic take and a full arrangement are one recording to
    // a fingerprint and two songs to a singer.
    let second = db
        .add_to_package(&package, &["bbb".to_owned()], "2026-09-10T00:00:00Z")
        .expect("add");
    assert_eq!((second.added, second.clashed), (1, 1));
    assert_eq!(db.package_members(&package, 1).expect("members").len(), 2);
}

#[test]
fn two_versions_arriving_together_catch_each_other() {
    let mut db = db();
    for id in ["aaa", "bbb"] {
        add(&mut db, id, Some("One Song"), &format!("f/{id}.kar"));
    }
    suggest_pair(&mut db, "aaa", "bbb");
    db.cluster().expect("cluster");

    let package = "vol1".to_owned();
    db.create_package(
        &PackageRow {
            id: package.clone(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-09-10T00:00:00Z",
    )
    .expect("create");
    // The second one sees the first already inserted, which is why the check is read per song
    // rather than once before the loop.
    let result = db
        .add_to_package(
            &package,
            &["aaa".to_owned(), "bbb".to_owned()],
            "2026-09-10T00:00:00Z",
        )
        .expect("add");
    assert_eq!((result.added, result.clashed), (2, 1));
}

/// A hidden version is left out of a collapsed search, returns with every version, and heads the
/// list when the search started from it.
#[test]
fn a_similar_name_search_shows_one_row_per_recording_unless_asked() {
    let mut db = db();
    for id in ["primary", "hidden"] {
        add_scanned(&mut db, id, |song| {
            song.det_title = Some("A Casa".to_owned());
            song.det_artist = Some("Vinicius de Morais".to_owned());
        });
    }
    db.conn
        .execute(
            "UPDATE songs SET duplicate_of = 'primary' WHERE id = 'hidden'",
            [],
        )
        .expect("hide");
    let every = Filter {
        versions: VersionsFilter::All,
        ..Filter::default()
    };
    let ids = |db: &Db, from: &str, filter: &Filter| -> Vec<String> {
        db.similar_names("A Casa", "Vinicius de Morais", from, filter)
            .expect("search")
            .into_iter()
            .map(|song| song.id)
            .collect()
    };

    assert_eq!(ids(&db, "primary", &Filter::default()), ["primary"]);
    assert_eq!(ids(&db, "primary", &every), ["primary", "hidden"]);
    assert_eq!(
        ids(&db, "hidden", &Filter::default()),
        ["hidden", "primary"]
    );
}

/// A song found under other spellings of its name comes back likeliest first, headed by itself.
///
/// The file name only case is the one the index reaches through `stem`: nobody typed a title and
/// the file gave none, so the name on disk holds the artist and the title together.
#[test]
fn a_song_is_found_under_other_spellings_of_its_name() {
    let mut db = db();
    let named = |db: &mut Db, id: &str, title: &str, artist: &str| {
        let (title, artist) = (title.to_owned(), artist.to_owned());
        add_scanned(db, id, move |song| {
            song.det_title = Some(title);
            song.det_artist = (!artist.is_empty()).then_some(artist);
        });
    };
    named(&mut db, "self", "Dancing in the Dark", "Bruce Springsteen");
    named(&mut db, "caps", "DANCING IN THE DARK", "Springsteen, Bruce");
    named(&mut db, "typo", "Dancin' in the Dark", "Bruce Springstein");
    named(
        &mut db,
        "street",
        "Dancing in the Street",
        "Martha and the Vandellas",
    );
    named(&mut db, "other", "Corcovado", "Tom Jobim");
    add_built(
        &mut db,
        "stem",
        None,
        "folder/Bruce Springsteen - Dancing in the Dark.kar",
        |_| {},
    );
    let every = Filter {
        versions: VersionsFilter::All,
        ..Filter::default()
    };

    let hits = db
        .similar_names("Dancing in the Dark", "Bruce Springsteen", "self", &every)
        .expect("search");
    let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();

    assert_eq!(ids.first(), Some(&"self"), "{ids:?}");
    assert_eq!(ids.iter().filter(|id| **id == "self").count(), 1, "{ids:?}");
    assert!(hits[0].searched_from);
    assert!(hits[1..].iter().all(|song| !song.searched_from));
    assert!(!ids.contains(&"other"), "{ids:?}");
    assert_eq!(ids.get(1), Some(&"caps"), "{ids:?}");
    for id in ["typo", "stem"] {
        assert!(ids.contains(&id), "{id} is missing from {ids:?}");
    }
    if let Some(street) = ids.iter().position(|id| *id == "street") {
        assert_eq!(street, ids.len() - 1, "{ids:?}");
    }
    assert!(
        hits[1..]
            .windows(2)
            .all(|pair| pair[0].likeness >= pair[1].likeness),
        "not likeliest first"
    );

    // A filter drops the matches it does not keep, and the song searched from heads the list anyway.
    db.conn
        .execute(
            "UPDATE songs SET granularity = 'linelevel' WHERE id IN ('self', 'caps')",
            [],
        )
        .expect("granularity");
    let per_syllable = Filter {
        granularity: Some("syllablelevel".to_owned()),
        ..every.clone()
    };
    let hits = db
        .similar_names(
            "Dancing in the Dark",
            "Bruce Springsteen",
            "self",
            &per_syllable,
        )
        .expect("search");
    let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids.first(), Some(&"self"), "{ids:?}");
    assert!(!ids.contains(&"caps"), "{ids:?}");
    assert!(ids.contains(&"typo"), "{ids:?}");

    // The song searched from heads the list even when the typed name no longer finds it.
    let hits = db
        .similar_names("Corcovado", "Tom Jobim", "self", &every)
        .expect("search");
    let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids, ["self", "other"]);

    // An id that names no song, or a merged one, is no row and no error.
    let hits = db
        .similar_names("Dancing in the Dark", "", "missing", &every)
        .expect("search");
    assert!(hits.iter().all(|song| !song.searched_from));
    db.conn
        .execute(
            "UPDATE songs SET merged_into = 'caps' WHERE id = 'self'",
            [],
        )
        .expect("merge");
    let hits = db
        .similar_names("Dancing in the Dark", "Bruce Springsteen", "self", &every)
        .expect("search");
    assert!(hits.iter().all(|song| song.id != "self"));

    // A name with no words is no search, not a search of everything.
    assert!(
        db.similar_names("!!", "", "", &every)
            .expect("search")
            .is_empty()
    );
}

pub(crate) fn add(db: &mut Db, id: &str, title: Option<&str>, path: &str) {
    add_built(db, id, title, path, |_| {});
}

/// [`add`], with the artist the file declared, for a test that needs a row already credited.
pub(crate) fn add_with_artist(db: &mut Db, id: &str, title: &str, artist: &str, path: &str) {
    add_built(db, id, Some(title), path, |song| {
        song.det_artist = Some(artist.to_owned());
    });
}

/// [`add`], with a chance to adjust the scanned song before it is written.
pub(crate) fn add_scanned(
    db: &mut Db,
    id: &str,
    adjust: impl FnOnce(&mut crate::model::ScannedSong),
) {
    add_built(db, id, Some(id), &format!("folder/{id}.kar"), adjust);
}

fn add_built(
    db: &mut Db,
    id: &str,
    title: Option<&str>,
    path: &str,
    adjust: impl FnOnce(&mut crate::model::ScannedSong),
) {
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
    };
    adjust(&mut song);
    let file = crate::model::ScannedFile {
        path: path.to_owned(),
        size: 1234,
        mtime: 0,
        content_hash: Some(id.to_owned()),
        status: crate::model::ScanStatus::Ok,
        error: None,
        song: Some(song),
    };
    db.write_scanned(&[file], "2026-08-24T00:00:00Z")
        .expect("write");
}

fn ids(rows: &[SongRow]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

/// Adds `copies` byte-identical files for one song. The content hash is the id, so they land on
/// one `songs` row without any grouping pass — which is the whole reason `songs.id` is the hash.
fn add_copies(db: &mut Db, id: &str, copies: usize) {
    for n in 0..copies {
        add(db, id, Some(id), &format!("folder{n}/{id}.kar"));
    }
}

/// What `songs.file_count` holds, straight from the column rather than through a browse row.
fn stored_count(db: &Db, id: &str) -> i64 {
    db.conn
        .query_row("SELECT file_count FROM songs WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("the song")
}

#[test]
fn the_copies_count_is_maintained_by_the_triggers_and_not_by_anything_else() {
    let mut db = db();
    add_copies(&mut db, "a", 1);
    assert_eq!(stored_count(&db, "a"), 1, "the insert trigger counted it");

    add(&mut db, "a", Some("a"), "elsewhere/a.kar");
    assert_eq!(stored_count(&db, "a"), 2);

    // A second song, so repointing a file has somewhere to go.
    add_copies(&mut db, "b", 1);
    db.conn
        .execute(
            "UPDATE files SET song_id = 'b' WHERE path = 'elsewhere/a.kar'",
            [],
        )
        .expect("repoint");
    assert_eq!(
        (stored_count(&db, "a"), stored_count(&db, "b")),
        (1, 2),
        "an UPDATE OF song_id has to move the count, not just the row"
    );

    db.conn
        .execute("DELETE FROM files WHERE path = 'elsewhere/a.kar'", [])
        .expect("delete");
    assert_eq!((stored_count(&db, "a"), stored_count(&db, "b")), (1, 1));

    // And the browse row reads the same number the column holds.
    assert_eq!(db.song_row("a").expect("row").file_count, 1);
}

#[test]
fn the_copies_filter_buckets_songs_by_how_many_files_they_have() {
    let mut db = db();
    add_copies(&mut db, "one", 1);
    add_copies(&mut db, "three", 3);
    add_copies(&mut db, "eleven", 11);

    let with = |copies: CopiesFilter| {
        let mut rows = db
            .songs(&Filter {
                copies,
                sort: Sort::Title,
                ..Filter::default()
            })
            .expect("browse");
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };

    assert_eq!(with(CopiesFilter::Any), ["eleven", "one", "three"]);
    assert_eq!(with(CopiesFilter::One), ["one"]);
    // Ten is inside the middle bucket and eleven is not, which is the boundary worth pinning:
    // `2-10` and `10+` must not both claim a song with ten copies.
    assert_eq!(with(CopiesFilter::TwoToTen), ["three"]);
    assert_eq!(with(CopiesFilter::OverTen), ["eleven"]);

    // The bar's three buckets partition the column between them, which is what retired the
    // fourth: *2 or more* was `TwoToTen ∪ OverTen`, and offering a union beside its parts is a
    // dropdown whose options overlap. It lived on as an arm only the Duplicates page could
    // reach, and went with it.
    assert_eq!(
        [
            CopiesFilter::One,
            CopiesFilter::TwoToTen,
            CopiesFilter::OverTen
        ]
        .into_iter()
        .flat_map(with)
        .collect::<std::collections::BTreeSet<_>>()
        .len(),
        3
    );
}

/// The three bands over a real corpus, seams included.
///
/// The seams are the whole risk: 7 belongs to the middle band and 8 to the high one, and an
/// off-by-one at either is a suitability that no band shows — a song missing from every filtered view
/// while the unfiltered page still holds it, which reads as a broken index rather than as a
/// broken boundary.
#[test]
fn the_suitability_bands_partition_the_corpus_by_score() {
    let mut db = db();
    for score in 0..=10u8 {
        add_scanned(&mut db, &format!("s{score:02}"), |song| {
            song.suitability.value = score;
        });
    }

    let with = |suitability: SuitabilityFilter| {
        let mut rows = db
            .songs(&Filter {
                suitability,
                sort: Sort::Title,
                ..Filter::default()
            })
            .expect("browse");
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };

    assert_eq!(with(SuitabilityFilter::High), ["s08", "s09", "s10"]);
    assert_eq!(with(SuitabilityFilter::Middle), ["s05", "s06", "s07"]);
    assert_eq!(
        with(SuitabilityFilter::Low),
        ["s00", "s01", "s02", "s03", "s04"]
    );
    assert_eq!(with(SuitabilityFilter::Any).len(), 11);

    // The three bands between them account for the whole corpus and overlap nowhere, which is
    // what the ladder they replaced could not say: `≥ 5` and `≥ 8` are nested.
    let banded = [
        SuitabilityFilter::High,
        SuitabilityFilter::Middle,
        SuitabilityFilter::Low,
    ]
    .into_iter()
    .flat_map(with)
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(banded.len(), 11);

    // And a range the address names cuts the column wherever it says, both ends included.
    assert_eq!(
        with(SuitabilityFilter::parse("2-5")),
        ["s02", "s03", "s04", "s05"]
    );
    assert_eq!(with(SuitabilityFilter::parse("9")), ["s09"]);
    assert_eq!(with(SuitabilityFilter::parse("9-")), ["s09", "s10"]);
    assert_eq!(with(SuitabilityFilter::parse("0-10")).len(), 11);
}

/// The added-date filter counts back from now, and a later scan does not move the date.
#[test]
fn the_added_filter_counts_back_from_now_and_a_rescan_keeps_the_date() {
    let mut db = db();
    for (id, age) in [
        ("hour", "-1 hour"),
        ("days", "-3 days"),
        ("weeks", "-20 days"),
        ("months", "-90 days"),
    ] {
        add_scanned(&mut db, id, |_| {});
        db.conn
            .execute(
                "UPDATE songs SET first_seen = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                 WHERE id = ?1",
                params![id, age],
            )
            .expect("backdate");
    }

    let with = |added: AddedFilter| {
        let mut rows = db
            .songs(&Filter {
                added,
                ..Filter::default()
            })
            .expect("browse");
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(with(AddedFilter::Day), ["hour"]);
    assert_eq!(with(AddedFilter::Week), ["days", "hour"]);
    assert_eq!(with(AddedFilter::Month), ["days", "hour", "weeks"]);
    assert_eq!(with(AddedFilter::OverMonth), ["months"]);
    assert_eq!(with(AddedFilter::Any).len(), 4);

    let newest = db
        .songs(&Filter {
            sort: Sort::Added,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&newest), ["hour", "days", "weeks", "months"]);

    let before = db.song("months").expect("song").first_seen;
    add_scanned(&mut db, "months", |_| {});
    assert_eq!(db.song("months").expect("song").first_seen, before);
    assert_eq!(db.song("months").expect("song").added_on(), &before[..10]);

    for filter in [
        AddedFilter::Day,
        AddedFilter::Week,
        AddedFilter::Month,
        AddedFilter::OverMonth,
    ] {
        assert_eq!(AddedFilter::parse(filter.as_str()), filter);
    }
    assert_eq!(AddedFilter::parse("lots"), AddedFilter::Any);
}

/// A song with no automatic suitability is in no band, *including* the low one.
///
/// This is the decision the low band forced and it is worth pinning rather than discovering: a
/// video song's `score` is NULL, not 0 — see the note on the column in `schema.sql` — and SQL's
/// three-valued logic means `NULL < 5` is not true, so *under 5* passes it over. That is the
/// right answer (an unscored song is not a bad one) and it is also what the `≥ N` ladder always
/// did, so no filtered view changes behavior here. `set`/`unset` is [`ScoreFilter`]'s job, and
/// the `kind` filter is what actually finds videos.
///
/// The NULL is written straight into the column rather than by scanning a video, because it is
/// the storage the clause has to cope with — building a `VideoFacts` would test the scanner.
#[test]
fn an_unscored_song_falls_outside_every_band() {
    let mut db = db();
    add_scanned(&mut db, "scored", |song| {
        song.suitability.value = 2;
    });
    add_scanned(&mut db, "unscored", |_| {});
    db.conn
        .execute(
            "UPDATE songs SET suitability = NULL WHERE id = 'unscored'",
            [],
        )
        .expect("clear the suitability");

    let with = |suitability: SuitabilityFilter| {
        let rows = db
            .songs(&Filter {
                suitability,
                ..Filter::default()
            })
            .expect("browse");
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };

    assert_eq!(with(SuitabilityFilter::Low), ["scored"]);
    assert!(with(SuitabilityFilter::Middle).is_empty());
    assert!(with(SuitabilityFilter::High).is_empty());

    // A range keeps the same rule, `0-10` included: it is a clause, so the NULL falls outside it.
    assert_eq!(with(SuitabilityFilter::parse("0-4")), ["scored"]);
    assert_eq!(with(SuitabilityFilter::parse("0-10")), ["scored"]);

    // Only *any* shows it, because only *any* adds no clause.
    assert_eq!(with(SuitabilityFilter::Any).len(), 2);
}

#[test]
fn a_copies_filter_round_trips_through_the_query_string() {
    // `2+` among them: the bar cannot show that bucket, but the Duplicates page links to it, so it
    // has to survive a page turn like any other.
    for value in ["", "1", "2-10", "10+", "2+"] {
        assert_eq!(CopiesFilter::parse(value).as_str(), value, "{value:?}");
    }

    // Anything else shows the whole corpus rather than nothing, by the rule the rest of the bar
    // follows: a hand-edited query string must not produce an empty page with no explanation.
    for nonsense in ["2", "0", "many", "-1"] {
        assert_eq!(
            CopiesFilter::parse(nonsense),
            CopiesFilter::Any,
            "{nonsense}"
        );
    }
}

#[test]
fn the_languages_present_are_the_ones_the_corpus_holds_and_no_others() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "a.kar");
    add(&mut db, "b", Some("B"), "b.kar");
    add(&mut db, "c", Some("C"), "c.kar");
    // Hand-set on one...
    db.edit_song(
        "a",
        &SongEdit {
            language: Some(Some("pt".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("set pt");
    // ...and detected on another, which `eff_language` reads and the picker therefore has to
    // offer: a song filed under a language the picker does not list is a song nobody can move.
    db.conn
        .execute(
            "UPDATE songs SET det_language_tag = 'ja' WHERE id = 'b'",
            [],
        )
        .expect("set ja");

    let present: Vec<&str> = db
        .languages_present()
        .expect("present")
        .iter()
        .map(|language| language.code())
        .collect();
    assert_eq!(present, ["ja", "pt"], "code order, and nothing 'c' implies");

    // A merged song is not in the list either — it is not in any browse query, so a language only
    // it holds is a filter option that matches nothing.
    db.conn
        .execute("UPDATE songs SET merged_into = 'c' WHERE id = 'a'", [])
        .expect("merge");
    let present: Vec<&str> = db
        .languages_present()
        .expect("present")
        .iter()
        .map(|language| language.code())
        .collect();
    assert_eq!(present, ["ja"]);
}

#[test]
fn a_hand_set_score_survives_a_round_trip_and_can_be_cleared() {
    let mut db = db();
    add(&mut db, "a", Some("Corcovado"), "a.kar");

    db.set_user_score("a", Some(9)).expect("set");
    assert_eq!(db.song("a").expect("song").user_score, Some(9));
    assert_eq!(db.song_row("a").expect("row").user_score, Some(9));

    // Clearing has to be possible, and has to land on NULL rather than 0: a song rated 0 and a
    // song nobody has rated are different answers.
    db.set_user_score("a", None).expect("clear");
    assert_eq!(db.song("a").expect("song").user_score, None);
}

#[test]
fn scoring_a_song_that_is_not_there_says_so() {
    let db = db();
    assert!(matches!(
        db.set_user_score("nope", Some(5)),
        Err(DbError::NotFound(_))
    ));
}

#[test]
fn a_score_filter_reads_every_shape_it_offers() {
    assert_eq!(ScoreFilter::parse(""), ScoreFilter::Any);
    assert_eq!(ScoreFilter::parse("unset"), ScoreFilter::Unset);
    assert_eq!(ScoreFilter::parse("set"), ScoreFilter::Set);
    assert_eq!(ScoreFilter::parse("0"), ScoreFilter::AtLeast(0));
    assert_eq!(ScoreFilter::parse("10"), ScoreFilter::AtLeast(10));
    // Anything else is *any*, so a hand-typed URL loosens the filter rather than showing nothing.
    assert_eq!(ScoreFilter::parse("11"), ScoreFilter::Any);
    assert_eq!(ScoreFilter::parse("nonsense"), ScoreFilter::Any);
    // And a round trip through the query string keeps the control set.
    for value in ["", "unset", "set", "7"] {
        assert_eq!(ScoreFilter::parse(value).as_str(), value);
    }
}

#[test]
fn the_hand_set_score_filters_every_shape_the_bar_offers() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "a.kar");
    add(&mut db, "b", Some("B"), "b.kar");
    add(&mut db, "c", Some("C"), "c.kar");
    db.set_user_score("a", Some(9)).expect("set");
    db.set_user_score("b", Some(4)).expect("set");

    let by = |user: ScoreFilter| {
        db.songs(&Filter {
            user_score: user,
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse")
    };

    assert_eq!(ids(&by(ScoreFilter::Set)), ["a", "b"]);
    assert_eq!(ids(&by(ScoreFilter::Unset)), ["c"]);
    assert_eq!(ids(&by(ScoreFilter::AtLeast(5))), ["a"]);
    assert_eq!(ids(&by(ScoreFilter::AtLeast(4))), ["a", "b"]);
    assert_eq!(ids(&by(ScoreFilter::AtLeast(10))), [] as [&str; 0]);
}

#[test]
fn an_unrated_song_sorts_after_a_badly_rated_one() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "a.kar");
    add(&mut db, "b", Some("B"), "b.kar");
    db.set_user_score("a", Some(0)).expect("set");

    let rows = db
        .songs(&Filter {
            sort: Sort::UserScore,
            ..Filter::default()
        })
        .expect("browse");
    // Zero is a judgment and NULL is not; sorting them together would bury the songs somebody
    // has actually rejected among the hundreds of thousands nobody has looked at.
    assert_eq!(ids(&rows), ["a", "b"]);
}

/// The bug as reported: SQLite's default collation puts every accented character after `Z`, so a
/// Portuguese corpus browsed by name ended with the songs a Portuguese speaker was looking for.
/// The twin of `km_catalog`'s and `km_remote_core`'s tests of the same name.
#[test]
fn songs_sort_by_a_folded_key_rather_than_by_the_raw_title() {
    let mut db = db();
    add(&mut db, "a", Some("Zebra"), "zebra.kar");
    add(
        &mut db,
        "b",
        Some("É o amor (Zezé di Camargo e Luciano)"),
        "eoamor.kar",
    );
    add(&mut db, "c", Some("Banana"), "banana.kar");
    add(&mut db, "d", Some("Águas de Março"), "aguas.kar");

    let rows = db
        .songs(&Filter {
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&rows), ["d", "c", "b", "a"]);
    // **And the list still shows the accents.** The folded key is what the order reads; a
    // `browse_columns` "simplified" to select it would render the whole corpus unaccented and
    // lower-case, which nothing else here would catch.
    assert_eq!(rows[0].title, "Águas de Março");
}

/// The default collation is case-*sensitive* as well as accent-blind, so `apple` came after
/// `Zebra`. Nothing asserted this before, because `COLLATE NOCASE` was never reached for.
#[test]
fn the_browse_order_ignores_case() {
    let mut db = db();
    add(&mut db, "a", Some("apple"), "apple.kar");
    add(&mut db, "b", Some("Banana"), "banana.kar");
    add(&mut db, "c", Some("cherry"), "cherry.kar");
    add(&mut db, "d", Some("Date"), "date.kar");

    let rows = db
        .songs(&Filter {
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&rows), ["a", "b", "c", "d"]);
}

/// **Three groups, not two**, and the middle one is the easy casualty of this change.
/// `eff_artist` has no `nullif`, so an artist recorded as the empty string is not the same as no
/// artist at all, and the browse list has always sorted them apart. `sort_artist` is NULL only
/// where `eff_artist` is, and `Sort::Artist`'s leading `IS NULL` term is what keeps it so.
#[test]
fn an_artist_recorded_as_blank_still_sorts_before_one_nobody_recorded() {
    let mut db = db();
    add_built(&mut db, "named", Some("One"), "one.kar", |song| {
        song.det_artist = Some("Ástor".to_owned());
    });
    add_built(&mut db, "blank", Some("Two"), "two.kar", |song| {
        song.det_artist = Some(String::new());
    });
    add(&mut db, "absent", Some("Three"), "three.kar");

    let rows = db
        .songs(&Filter {
            sort: Sort::Artist,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&rows), ["blank", "named", "absent"]);
}

/// One title's rows are contiguous by performer, and a song nobody named one for comes last.
///
/// A content hash is not an order anybody can read, so a page holding five songs called *Goodbye*
/// scattered its performers through them. The three groups of the test above are the three here:
/// named, blank and absent — and the absent one goes to the end of the title rather than the front
/// of it, which is the half a plain `sort_artist` term would get wrong, SQLite sorting NULL first.
#[test]
fn within_one_title_the_rows_are_ordered_by_performer() {
    let mut db = db();
    add_built(&mut db, "spice", Some("Goodbye"), "spice.kar", |song| {
        song.det_artist = Some("Spice Girls".to_owned());
    });
    add(&mut db, "nobody", Some("Goodbye"), "nobody.kar");
    add_built(&mut db, "blank", Some("Goodbye"), "blank.kar", |song| {
        song.det_artist = Some(String::new());
    });
    add_built(&mut db, "air", Some("Goodbye"), "air.kar", |song| {
        song.det_artist = Some("Air Supply".to_owned());
    });
    add_built(
        &mut db,
        "later",
        Some("Gotta Tell You"),
        "gotta.kar",
        |song| {
            song.det_artist = Some("Aaron".to_owned());
        },
    );

    let rows = db
        .songs(&Filter {
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&rows), ["blank", "air", "spice", "nobody", "later"]);
}

/// The tie-break reaches every order that gets as far as a title, and not only the title order.
///
/// Each of these leads with a column every song here shares, so what is being read is the tail. The
/// two that never reach a title — length and copies — are absent for that reason.
#[test]
fn every_order_that_reaches_a_title_breaks_the_tie_on_the_performer() {
    let mut db = db();
    for (id, artist) in [("spice", "Spice Girls"), ("air", "Air Supply")] {
        add_built(&mut db, id, Some("Goodbye"), &format!("{id}.kar"), |song| {
            song.det_artist = Some(artist.to_owned());
            song.det_language = Some("en".to_owned());
        });
        db.set_user_score(id, Some(7)).expect("user score");
    }

    for sort in [
        Sort::Title,
        Sort::Suitability,
        Sort::UserScore,
        Sort::Language,
        Sort::Updated,
    ] {
        let rows = db
            .songs(&Filter {
                sort,
                ..Filter::default()
            })
            .expect("browse");
        assert_eq!(ids(&rows), ["air", "spice"], "{sort:?}");
    }
}

/// The order and the index key that serves it are one rule in two spellings.
///
/// The difference between them is the table alias a query has and an index key does not, so the
/// derivation is the whole assertion. A term added to one and forgotten in the other does not fail:
/// the planner drops the index and sorts the corpus, which no test of what a page shows can see.
#[test]
fn the_browse_order_and_its_index_key_say_the_same_thing() {
    assert_eq!(
        WITHIN_TITLE,
        format!("s.{}", WITHIN_TITLE_KEY.replace(", ", ", s."))
    );
}

/// The reported bug in full: a song files under `A` *and* sorts to the top of the A page, where
/// before it filed correctly and then sank to the bottom. The bucket folded and the order did
/// not, which is one list running two alphabets.
#[test]
fn the_letter_bucket_and_the_order_within_it_both_fold() {
    let mut db = db();
    add(&mut db, "a", Some("Azul"), "azul.kar");
    add(&mut db, "b", Some("Águas de Março"), "aguas.kar");
    add(&mut db, "c", Some("Abacaxi"), "abacaxi.kar");

    let rows = db
        .songs(&Filter {
            initial: Initial::Letter('A'),
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&rows), ["c", "b", "a"]);
}

/// The leak the browse tests cannot see. `add_to_package` numbers songs in the order it is handed
/// them, and it is handed [`Db::matching_ids`] — so before this the accent bug did not merely
/// look wrong on a page, it decided the song numbers a package shipped with.
#[test]
fn a_package_is_numbered_in_the_order_the_list_showed() {
    let mut db = db();
    add(&mut db, "a", Some("Zebra"), "zebra.kar");
    add(&mut db, "b", Some("Águas de Março"), "aguas.kar");
    add(&mut db, "c", Some("Banana"), "banana.kar");

    let ordered = db
        .matching_ids(
            &Filter {
                sort: Sort::Title,
                ..Filter::default()
            },
            None,
        )
        .expect("matching");
    assert_eq!(ordered, ["b", "c", "a"]);
}

/// Every path that writes a name leaves the key it was folded from in step with it.
///
/// One test naming all of them rather than one test each, because what is being asserted is a
/// property of the set: **no write path may leave a stale key**. A new path added without a
/// `refold` fails here.
#[test]
fn every_write_path_leaves_the_sort_keys_in_step_with_the_titles() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");
    add(&mut db, "b", Some("UNTITLED"), "Corcovado.kar");
    assert_folded(&db);

    // The scan's upsert, over a song that already exists.
    add(&mut db, "a", Some("Água de Beber"), "aguas.kar");
    assert_folded(&db);

    db.edit_song(
        "a",
        &SongEdit {
            title: Some(Some("Éramos Nós".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");
    assert_folded(&db);

    // Clearing it, which falls the effective title back to the detected one.
    db.edit_song(
        "a",
        &SongEdit {
            title: Some(None),
            ..SongEdit::default()
        },
    )
    .expect("clear");
    assert_folded(&db);

    db.set_names_from_stem(&["b".to_owned()]).expect("rename");
    assert_folded(&db);

    add(&mut db, "c", Some("ÁGUAS DE MARÇO"), "aguas-again.kar");
    db.fix_name_case(&["c".to_owned()]).expect("recase");
    assert_folded(&db);
}

/// The invalidator, on its own.
///
/// A write that goes round `Db::refold` leaves NULL rather than a stale value — so it sorts to
/// the top of the list where somebody will see it, and the next open repairs it. That is the
/// whole reason the trigger exists beside the Rust, and it is what would stop being true if
/// somebody dropped `songs_refold_update` or widened it back to a bare `AFTER UPDATE`.
#[test]
fn a_write_path_that_forgets_to_refold_is_left_visible_rather_than_wrong() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");

    db.execute_for_test("UPDATE songs SET det_title = 'Zebra' WHERE id = 'a'")
        .expect("a write nobody taught to refold");
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs WHERE sort_title IS NULL")
            .expect("count"),
        1
    );

    // And the next open puts it right.
    db.backfill_sort_keys(&|_| {}).expect("backfill");
    assert_folded(&db);
}

/// A browse index whose key has changed is replaced rather than kept under its old name.
///
/// **The failure this catches has no symptom but the clock.** Every `CREATE` in
/// `create_browse_indexes` is `IF NOT EXISTS`, which cannot see a key — so an index whose terms
/// changed while its name did not would sit in an existing database holding the old terms, the query
/// would match neither it nor anything else, and the page would go back to sorting the corpus. The
/// name changing is what makes the drop loop find it, and what makes `missing_indexes` ask for the
/// statistics the planner needs before it will choose the replacement.
#[test]
fn an_index_under_a_retired_name_is_dropped_on_open() {
    let scratch = Scratch::new("retired-index");
    let dir = scratch.0.clone();

    {
        let db = Db::create(&dir).expect("create");
        db.execute_for_test(
            "DROP INDEX songs_browse_title_artist;
             CREATE INDEX songs_browse_sort_title
                 ON songs(sort_title, id) WHERE merged_into IS NULL;",
        )
        .expect("an index under the name a previous key was built with");
    }

    let db = Db::open(&dir).expect("open");
    let indexes = own_indexes(&db.conn, "songs").expect("indexes");
    assert!(
        !indexes.iter().any(|name| name == "songs_browse_sort_title"),
        "the retired name is gone: {indexes:?}"
    );
    assert!(
        indexes
            .iter()
            .any(|name| name == "songs_browse_title_artist"),
        "and the key this build intends is there: {indexes:?}"
    );
}

/// A database folded by an older `km_song::text::fold` refolds itself on the next open.
///
/// Not the migration above, which fires on the *absence* of the columns. Here they are present and
/// filled, and every value in them is what the previous fold correctly produced — `přiliš` is
/// indistinguishable from a fresh key by inspection, which is why the guard is a revision number and
/// not something read off the table.
#[test]
fn a_database_folded_by_an_older_table_refolds_itself_on_open() {
    let scratch = Scratch::new("fold-revision");
    let dir = scratch.0.clone();

    {
        let mut db = Db::create(&dir).expect("create");
        add(&mut db, "a", Some("Zebra"), "zebra.kar");
        add(&mut db, "b", Some("Příliš"), "prilis.kar");
        add(&mut db, "c", Some("apple"), "apple.kar");
        // The keys the Latin-1-only table wrote: `í` folded because it knew that one, `ř` and `š`
        // left alone, which is what put the row after `Z`.
        db.execute_for_test("UPDATE songs SET sort_title = 'přiliš' WHERE id = 'b';")
            .expect("fabricate an older fold");
        db.set_setting(FOLD_REVISION, "1").expect("older revision");
    }

    let db = Db::open(&dir).expect("an older curation database still opens");
    assert_folded(&db);
    let rows = db
        .songs(&Filter {
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(
        ids(&rows),
        ["c", "b", "a"],
        "refolded: `Příliš` browses under P rather than after Z"
    );
    assert_eq!(
        db.setting(FOLD_REVISION).expect("setting").as_deref(),
        Some(km_song::text::FOLD_REVISION.to_string().as_str()),
        "the revision is recorded, so the next open does no work"
    );
}

/// The every-open probe reads its partial index rather than the corpus — the same property
/// `a_repaired_database_finds_nothing_to_repair_without_reading_it_all` pins for the stems, and
/// for the same reason: this question is asked on every open and the answer is almost always no.
#[test]
fn a_folded_database_finds_nothing_to_fold_without_reading_it_all() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");

    let plan = db
        .plan_for_test("SELECT COUNT(*) FROM songs WHERE sort_title IS NULL", &[])
        .expect("plan");
    assert!(
        plan.contains("songs_unfolded"),
        "the fold probe reads its partial index; plan was:\n{plan}"
    );
    // Scanning the *index* is the right plan and the whole point: on a folded database it holds
    // nothing, so the scan is one page. What must never appear is a scan of the table, which is
    // what this cost before the index existed and what it would cost again if the predicate and
    // the index stopped matching.
    assert!(
        !plan.lines().any(|line| line.trim() == "SCAN songs"),
        "and never the corpus itself; plan was:\n{plan}"
    );
}

/// A `songs_browse_*` this build does not intend is dropped rather than left to be chosen.
///
/// **This is what makes changing an index key safe.** Every `CREATE` in `create_browse_indexes`
/// is `IF NOT EXISTS`, so without the sweep an existing database would keep the seven indexes
/// whose keys this change replaced, holding their old keys under their old names, and the browse
/// page would silently go back to sorting the corpus.
#[test]
fn an_index_this_build_no_longer_intends_is_dropped_rather_than_left_to_be_chosen() {
    let db = db();
    db.execute_for_test(
        "CREATE INDEX songs_browse_stale ON songs(det_title) WHERE merged_into IS NULL",
    )
    .expect("an index from a build that is gone");

    db.create_browse_indexes().expect("rebuild");
    assert!(
        !own_indexes(&db.conn, "songs")
            .expect("indexes")
            .iter()
            .any(|name| name == "songs_browse_stale")
    );
}

/// Rebuilding an index says so, because the statistics describing the old one outlive it.
///
/// **The hole this closes is the one `missing_indexes` cannot see.** That list is read before
/// `schema.sql` runs and it holds *names*; an index whose key or predicate changed keeps its name
/// all the way through, so it is never reported missing and nothing asks for an `ANALYZE`. Its
/// `sqlite_stat1` row then survives describing a shape the database no longer has, and the planner
/// prices an index that is gone.
///
/// **Measured on a real corpus rather than reasoned about**: every browse sort abandoned its index
/// and read the whole corpus into a temp B-tree, four seconds a page, and one `ANALYZE` by hand put
/// all nine back. The plan was correct the moment the statistics were, which is why this is about
/// the signal and not about the index.
///
/// The false case matters as much as the true one: an open that rebuilt nothing must not pay for a
/// corpus-sized `ANALYZE`, which is minutes on the database this tool is for.
#[test]
fn a_rebuilt_index_asks_for_the_statistics_it_invalidated() {
    let db = db();
    assert!(
        !db.create_browse_indexes().expect("a settled database"),
        "an open that changed no index must not ask for an ANALYZE"
    );

    // The shape a build one predicate behind wrote: the right name, the wrong `WHERE`.
    db.execute_for_test(
        "DROP INDEX songs_browse_title_artist;
         CREATE INDEX songs_browse_title_artist
             ON songs(sort_title, sort_artist IS NULL, sort_artist, id)
           WHERE merged_into IS NULL",
    )
    .expect("an index from a build that is behind");

    assert!(
        db.create_browse_indexes().expect("rebuild"),
        "an index rebuilt under its own name has to ask for the statistics it invalidated"
    );
    assert!(
        !db.create_browse_indexes().expect("settled again"),
        "and the open after it has nothing left to gather"
    );
}

/// A bulk write that touches no name retokenises nothing.
///
/// `songs_fts_update` and `lyrics_fts_update` were bare `AFTER UPDATE`, so setting one language
/// over a filter deleted and reinserted the title, artist and lyric rows of every song it
/// touched, to arrive at the text already in them. `UPDATE OF` is what stopped it, and narrowing
/// them is also what makes `songs_refold_update`'s own inner `UPDATE` free.
#[test]
fn a_bulk_language_change_does_not_retokenise_every_title() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");
    add(&mut db, "b", Some("Corcovado"), "corc.kar");

    let lyrics_before = db
        .count_for_test("SELECT COUNT(*) FROM lyrics_fts")
        .expect("count");
    db.set_language_for(&Filter::default(), Language::parse("pt"))
        .expect("language");

    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM lyrics_fts")
            .expect("count"),
        lyrics_before,
        "a language change is not a lyric change"
    );
    // The titles are still findable, which is what would break if `UPDATE OF` had been given the
    // wrong column list and the index had been left holding a deleted row.
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs_fts WHERE songs_fts MATCH 'aguas'")
            .expect("count"),
        1
    );
    assert_folded(&db);
}

/// A bulk tag write touches neither `songs` nor the FTS index.
///
/// The twin of the language test above, and cheaper than it by construction: that one writes a
/// column on `songs` and leans on `songs_fts_update` naming the right columns; this one writes
/// a different table entirely, so the triggers cannot fire at all. Worth asserting anyway,
/// because the obvious alternative design — a `songs.tags` column here, as the machine's catalog
/// has — would have put a bulk tag write straight through those triggers over a quarter of a
/// million rows.
#[test]
fn a_bulk_tag_write_does_not_retokenise_every_title() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");
    add(&mut db, "b", Some("Corcovado"), "corc.kar");

    let lyrics_before = db
        .count_for_test("SELECT COUNT(*) FROM lyrics_fts")
        .expect("count");
    let tag = Tag::parse("rock").expect("a tag");
    assert_eq!(
        db.add_tag_for(&Filter::default(), &tag).expect("tag"),
        2,
        "both songs match an empty filter"
    );

    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM lyrics_fts")
            .expect("count"),
        lyrics_before,
        "a tag is not a lyric change"
    );
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs_fts WHERE songs_fts MATCH 'aguas'")
            .expect("count"),
        1
    );
    assert_folded(&db);
}

/// A scan that names a song what it is already called retokenises nothing.
///
/// **`UPDATE OF` fires on assignment and not on change**, and a scan assigns the detected columns on
/// every row it reads whether or not the file says anything new. Re-analysis reads the whole corpus
/// to move suitability, so without the `WHEN` guards on `songs_fts_update`, `lyrics_fts_update` and
/// `songs_refold_update` every song is deleted from two full-text indexes and reinserted, and both
/// its sort keys are cleared and written back, to arrive at what is already stored.
///
/// **The cleared key is the half of that a test can see.** `Db::refold` runs inside every write
/// path's own transaction, so a scan's writes are folded again by the time they commit and the
/// trigger firing is observable only from a bare `UPDATE`. The three guards are one predicate over
/// one column list, so a key left folded is all three of them holding.
#[test]
fn a_rescan_that_renames_nothing_clears_no_sort_key() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");
    assert_folded(&db);

    db.execute_for_test(
        "UPDATE songs SET det_title = 'Águas de Março', stem = 'aguas' WHERE id = 'a'",
    )
    .expect("a scan naming the song what it is already called");

    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs WHERE sort_title IS NULL")
            .expect("count"),
        0,
        "a name that did not change is not a name to fold again"
    );
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs_fts WHERE songs_fts MATCH 'aguas'")
            .expect("count"),
        1,
        "and the title is still findable"
    );
}

/// A name the file really did change reaches the index.
///
/// **The risk a `WHEN` carries is a predicate narrower than the statement it guards**, which leaves
/// the index answering with a name nobody can search for and no error anywhere to say so. Every
/// column compared is a column the body reads, and this is what holds that true.
#[test]
fn a_rescan_that_does_rename_a_song_reindexes_it() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");

    db.execute_for_test("UPDATE songs SET det_title = 'Corcovado' WHERE id = 'a'")
        .expect("a scan reading a new name out of the file");

    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs_fts WHERE songs_fts MATCH 'corcovado'")
            .expect("count"),
        1,
        "the new name is findable"
    );
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs_fts WHERE songs_fts MATCH 'aguas'")
            .expect("count"),
        0,
        "and the old one has left the index"
    );
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM songs WHERE sort_title IS NULL")
            .expect("count"),
        1,
        "and the sort key is waiting to be folded again"
    );
}

/// A lyric the file really did change reaches the index.
///
/// The twin of the rename above, over the column that costs the most to tokenise.
#[test]
fn a_rescan_that_does_change_a_lyric_reindexes_it() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM lyrics_fts WHERE lyrics_fts MATCH 'second'")
            .expect("count"),
        1,
        "the words this fixture writes are in the index"
    );

    db.execute_for_test("UPDATE songs SET lyrics = 'chuva canteiro' WHERE id = 'a'")
        .expect("a scan reading new words out of the file");

    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM lyrics_fts WHERE lyrics_fts MATCH 'canteiro'")
            .expect("count"),
        1,
        "the new words are findable"
    );
    assert_eq!(
        db.count_for_test("SELECT COUNT(*) FROM lyrics_fts WHERE lyrics_fts MATCH 'second'")
            .expect("count"),
        0,
        "and the old ones have left the index"
    );
}

/// The vocabulary follows the songs: a word appears when used and goes when its last song drops.
///
/// Which is what keeps the picker a list of words in use rather than a museum of every word ever
/// typed — on a corpus this size, the difference between a useful datalist and one nobody reads.
#[test]
fn the_vocabulary_gains_a_tag_when_it_is_used_and_loses_it_when_it_is_not() {
    let mut db = db();
    add(&mut db, "a", Some("One"), "one.kar");
    add(&mut db, "b", Some("Two"), "two.kar");
    let rock = Tag::parse("rock").expect("a tag");

    assert!(db.tags_present().expect("vocabulary").is_empty());

    db.add_tag_of(&["a".to_owned()], &rock).expect("add");
    assert_eq!(db.tags_present().expect("vocabulary"), ["rock"]);
    assert_eq!(db.tags_of("a").expect("tags"), ["rock"]);
    assert!(db.tags_of("b").expect("tags").is_empty());

    // A second song keeps the word alive when the first loses it.
    db.add_tag_of(&["b".to_owned()], &rock).expect("add");
    db.remove_tag_of(&["a".to_owned()], &rock).expect("remove");
    assert_eq!(db.tags_present().expect("vocabulary"), ["rock"]);

    db.remove_tag_of(&["b".to_owned()], &rock).expect("remove");
    assert!(
        db.tags_present().expect("vocabulary").is_empty(),
        "the last song lost it, so the word goes"
    );
}

/// Two tags widen to the union, exactly as the machine's own filter does.
#[test]
fn the_tag_filter_takes_the_songs_carrying_any_tag() {
    let mut db = db();
    add(&mut db, "a", Some("Both"), "a.kar");
    add(&mut db, "b", Some("Rock only"), "b.kar");
    add(&mut db, "c", Some("Neither"), "c.kar");
    // A song under `brasil` alone: without one, a union and a filter that reads the first tag and
    // stops give the same answer, and the assertion below pins nothing.
    add(&mut db, "d", Some("Brasil only"), "d.kar");
    let rock = Tag::parse("rock").expect("a tag");
    let brasil = Tag::parse("brasil").expect("a tag");
    db.add_tag_of(&["a".to_owned(), "b".to_owned()], &rock)
        .expect("add");
    db.add_tag_of(&["a".to_owned(), "d".to_owned()], &brasil)
        .expect("add");

    let matching = |tags: &[&str]| {
        let filter = Filter {
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            ..Filter::default()
        };
        db.count_matching(&filter).expect("count")
    };

    // No tags is no tag filter, which is the case the `IN` has to be guarded against.
    assert_eq!(matching(&[]), 4);
    assert_eq!(matching(&["rock"]), 2);
    assert_eq!(matching(&["brasil"]), 2);
    assert_eq!(matching(&["rock", "brasil"]), 3, "OR, not AND");
    assert_eq!(matching(&["nobody-typed-this"]), 0);
    // A word nobody has used takes no songs off the tag beside it.
    assert_eq!(matching(&["rock", "nobody-typed-this"]), 2);
}

/// Every song's stored keys are what folding its effective names produces.
///
/// In Rust rather than as a SQL predicate, because the fold has no SQL spelling — which is the
/// same reason `Db::refold` exists at all.
fn assert_folded(db: &Db) {
    let sql = format!(
        "SELECT id, {}, {}, sort_title, sort_artist FROM songs",
        eff_title(""),
        eff_artist("")
    );
    let mut statement = db.conn.prepare(&sql).expect("prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");
    for (id, title, artist, sort_title, sort_artist) in rows {
        assert_eq!(
            sort_title.as_deref(),
            Some(km_song::text::fold(&title).as_str()),
            "song {id}'s title key is stale"
        );
        assert_eq!(
            sort_artist,
            artist.as_deref().map(km_song::text::fold),
            "song {id}'s artist key is stale"
        );
    }
}

#[test]
fn songs_file_under_their_first_letter_accents_and_all() {
    let mut db = db();
    add(&mut db, "a", Some("Águas de Março"), "aguas.kar");
    add(&mut db, "b", Some("Corcovado"), "corc.kar");
    add(&mut db, "c", Some("9 Crimes"), "nine.kar");
    add(&mut db, "d", Some("¿Y ahora qué?"), "yahora.kar");
    // No title at all: it files under the letter of the file name it is shown as.
    add(&mut db, "e", None, "Rosinha.kar");
    // Nothing to file under at all, which a real corpus does contain.
    add(&mut db, "g", Some("---"), "dashes.kar");

    // A second song under a different digit, so the one bucket has to hold both.
    add(&mut db, "f", Some("3 Marias"), "tres.kar");

    let under = |initial: &str| {
        db.songs(&Filter {
            initial: Initial::parse(initial),
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse")
    };

    assert_eq!(ids(&under("A")), ["a"], "Águas has to file under A");
    assert_eq!(ids(&under("C")), ["b"]);
    assert_eq!(ids(&under("R")), ["e"]);
    // **Not `#`.** The bucket is read off the folded sort key and `fold` strips leading
    // punctuation, so this files where the offline remote's A-Z strip and the printed book file
    // it, both of which go through `km_song::text::initial`. A Spanish song is not a symbol.
    assert_eq!(ids(&under("Y")), ["d"], "an opening ¿ does not hide a Y");
    // What genuinely has no letter still has none.
    assert_eq!(ids(&under("#")), ["g"]);
    // One bucket for every digit, sorted by title within it. `title_initial` still files them
    // one per digit — the collapse is in the predicate, so the index goes on working.
    assert_eq!(ids(&under("0-9")), ["f", "c"], "3 Marias, then 9 Crimes");
}

#[test]
fn a_nonsense_initial_shows_the_whole_corpus_rather_than_an_empty_page() {
    let mut db = db();
    add(&mut db, "a", Some("Corcovado"), "corc.kar");
    add(&mut db, "b", Some("9 Crimes"), "nine.kar");

    // The rule the `kind` parameter and `LanguageFilter::parse` already follow: `?initial=ZZ` must
    // not be a blank page with nothing on it saying why.
    // A single digit is not a bucket: the bar has one button for every digit, and it is `0-9`.
    for nonsense in ["ZZ", "0-99", "%", " ", "7"] {
        assert_eq!(Initial::parse(nonsense), Initial::Any, "{nonsense:?}");
    }
    let rows = db
        .songs(&Filter {
            initial: Initial::parse("ZZ"),
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse");
    assert_eq!(ids(&rows), ["b", "a"]);
}

#[test]
fn an_initial_round_trips_through_the_query_string() {
    for value in ["", "A", "Z", "0-9", "#"] {
        assert_eq!(Initial::parse(value).as_str(), value, "{value:?}");
    }
    // Case folded, because a hand-typed URL is as likely to be lower case.
    assert_eq!(Initial::parse("a"), Initial::Letter('A'));
    assert_eq!(Initial::Symbol.label(km_locale::Locale::English), "symbol");
    assert_eq!(
        Initial::Digits.describe(km_locale::Locale::English),
        "starts with a number"
    );
}

#[test]
fn a_folder_filter_matches_by_path_prefix_and_not_by_wildcard() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    add(&mut db, "b", Some("B"), "rock/deep/b.kar");
    add(&mut db, "c", Some("C"), "rockabilly/c.kar");
    // A folder named with a LIKE wildcard in it. `100% Hits` is not an unusual name.
    add(&mut db, "d", Some("D"), "100% Hits/d.kar");
    add(&mut db, "e", Some("E"), "100X Hits/e.kar");

    let under = |folder: &str| {
        db.songs(&Filter {
            folder: Some(folder.to_owned()),
            sort: Sort::Title,
            ..Filter::default()
        })
        .expect("browse")
    };

    // Subfolders included, and `rockabilly` is a different folder rather than a longer match.
    assert_eq!(ids(&under("rock/")), ["a", "b"]);
    assert_eq!(ids(&under("rockabilly/")), ["c"]);
    assert_eq!(
        ids(&under("100% Hits/")),
        ["d"],
        "% is a character, not a wildcard"
    );
}

/// Lists a folder the way the page does: rebuild where the index has fallen behind, then read it.
///
/// [`Db::folders`] reads the index and does not refresh it, because a rebuild writes and a page is
/// drawn through a connection that cannot — so deciding whether this is a moment for one belongs to
/// the caller. See [`Db::folder_index_is_current`].
fn listing(db: &Db, prefix: &str) -> Vec<FolderNode> {
    if !db.folder_index_is_current().expect("marker") {
        db.rebuild_folders().expect("rebuild");
    }
    db.folders(prefix).expect("folders")
}

#[test]
fn the_folder_listing_shows_one_level_at_a_time() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    add(&mut db, "b", Some("B"), "rock/deep/b.kar");
    add(&mut db, "c", Some("C"), "rock/deep/deeper/c.kar");
    add(&mut db, "d", Some("D"), "mpb/d.kar");

    let top = listing(&db, "");
    assert_eq!(
        top.iter()
            .map(|node| (node.name.as_str(), node.song_count))
            .collect::<Vec<_>>(),
        vec![("mpb", 1), ("rock", 3)],
        "the count is everything beneath a folder, not just what sits in it"
    );
    assert_eq!(top[1].path, "rock/");

    let rock = listing(&db, "rock/");
    assert_eq!(
        rock.iter()
            .map(|node| (node.name.as_str(), node.song_count))
            .collect::<Vec<_>>(),
        // The empty name is the bucket for `rock/a.kar`, which is in this folder rather than
        // under one of its children.
        vec![("", 1), ("deep", 2)]
    );
    assert!(rock[0].is_files_here());
    assert_eq!(rock[1].path, "rock/deep/");
}

// -- forgetting what is gone ---------------------------------------------------------------

/// The fast path, and the one nearly every scan takes.
///
/// Worth pinning rather than assuming, because it is the whole reason this function has the
/// shape it has: handed every path *present*, it would full-scan `files` and `songs` to discover
/// that none of them had gone. Handed nothing, it must do nothing — and "nothing" has to include
/// leaving the corpus exactly as it was.
#[test]
fn forgetting_nothing_deletes_nothing() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    add(&mut db, "b", Some("B"), "rock/b.kar");

    assert_eq!(db.forget_missing(&[]).expect("forget"), (0, 0));
    let counts = db.counts().expect("counts");
    assert_eq!((counts.songs, counts.files), (2, 2));
}

#[test]
fn a_file_that_is_gone_takes_its_last_song_with_it() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    add(&mut db, "b", Some("B"), "rock/b.kar");

    assert_eq!(
        db.forget_missing(&["rock/a.kar".to_owned()])
            .expect("forget"),
        (1, 1),
        "one file, and the song it was the only copy of"
    );
    let counts = db.counts().expect("counts");
    assert_eq!((counts.songs, counts.files), (1, 1));
    assert!(db.song("a").is_err() || db.song("b").is_ok());
}

/// A song with copies elsewhere survives losing one of them.
///
/// The scoped sweep asks `NOT EXISTS (SELECT 1 FROM files WHERE song_id = ?)` rather than
/// assuming a deleted file was the song's only one — this is the row that tells the difference.
#[test]
fn a_song_with_another_copy_survives_losing_one() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/one/a.kar");
    db.execute_for_test(
        "INSERT INTO files(path, size, mtime, content_hash, song_id, scan_status, scanned_at)
         VALUES ('rock/two/a.kar', 1234, 0, 'a', 'a', 'ok', '2026-08-24T00:00:00Z')",
    )
    .expect("second copy");

    assert_eq!(
        db.forget_missing(&["rock/one/a.kar".to_owned()])
            .expect("forget"),
        (1, 0),
        "the file goes, the song stays"
    );
    assert!(db.song("a").is_ok(), "the other copy still holds it");
}

/// **The one failure this must not have.**
///
/// Losing a curated selection because a drive was unmounted is worse than any amount of stale
/// data, so a song a package still names is kept even with no files at all, and shows as
/// *source missing*. Both the scoped delete and the `file_count = 0` backstop have to honor
/// it, which is why the sweep runs here too.
#[test]
fn a_song_a_package_still_names_survives_losing_every_file() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    db.create_package(
        &PackageRow {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        },
        "2026-08-25T00:00:00Z",
    )
    .expect("create a package");
    db.add_to_package("vol1", &["a".to_owned()], "2026-08-25T00:00:00Z")
        .expect("select it");

    assert_eq!(
        db.forget_missing(&["rock/a.kar".to_owned()])
            .expect("forget"),
        (1, 0),
        "the file goes and the song does not"
    );
    assert_eq!(db.forget_orphaned_songs().expect("sweep"), 0);
    assert!(db.song("a").is_ok(), "the package still names it");
    assert_eq!(db.counts().expect("counts").files, 0);
}

/// The backstop catches an orphan this scan did not create.
///
/// The scoped delete only looks at songs whose files it just removed, which is exact for every
/// orphan it makes and blind to one already there — a row left by an older version, or by a
/// write path since fixed. The whole-table statement it replaced swept those up as a side
/// effect, and dropping that quietly would be a slow leak rather than a visible bug.
#[test]
fn a_song_orphaned_before_this_scan_is_swept_up_too() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    // Around the trigger, deliberately: this is the state a *previous* version left behind, so
    // reaching it through the current write path would not be the same test.
    db.execute_for_test("DELETE FROM files")
        .expect("strand the song");
    db.execute_for_test("UPDATE songs SET file_count = 0")
        .expect("as an older version left it");

    assert_eq!(db.forget_orphaned_songs().expect("sweep"), 1);
    assert_eq!(db.counts().expect("counts").songs, 0);
}

#[test]
fn a_song_with_copies_in_two_subfolders_counts_once_in_their_parent() {
    let mut db = db();
    // The same recording, filed twice — the shape a scavenged corpus is full of.
    add(&mut db, "a", Some("A"), "rock/one/a.kar");
    db.execute_for_test(
        "INSERT INTO files(path, size, mtime, content_hash, song_id, scan_status, scanned_at)
         VALUES ('rock/two/a.kar', 1234, 0, 'a', 'a', 'ok', '2026-08-24T00:00:00Z')",
    )
    .expect("second copy");
    add(&mut db, "b", Some("B"), "rock/two/b.kar");

    let top = listing(&db, "");
    assert_eq!(
        top.iter()
            .map(|node| (node.name.as_str(), node.song_count))
            .collect::<Vec<_>>(),
        vec![("rock", 2)],
        "two songs under rock/, not three files and not one per copy"
    );
    let rock = listing(&db, "rock/");
    assert_eq!(
        rock.iter()
            .map(|node| (node.name.as_str(), node.song_count))
            .collect::<Vec<_>>(),
        vec![("one", 1), ("two", 2)]
    );
}

#[test]
fn a_file_at_the_root_shows_as_files_here_at_the_top_level() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "loose.kar");
    add(&mut db, "b", Some("B"), "rock/b.kar");

    let top = listing(&db, "");
    assert!(top[0].is_files_here(), "the bucket comes first: {top:?}");
    assert_eq!(top[0].song_count, 1);
    assert_eq!(top[0].path, "", "and filters to the root");
    assert_eq!(top[1].name, "rock");
}

/// The index says when it has fallen behind the corpus, and a rebuild is what catches it up.
///
/// **The saying and the catching up are two calls, and that is the point of the test.** A rebuild is
/// a whole pass over `files` and it writes, so a page — drawn through a connection that cannot
/// write, while a scan may be moving the marker on every batch — has to be able to ask the question
/// without paying for the answer.
#[test]
fn the_folder_index_says_when_the_corpus_has_moved_on() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    assert_eq!(listing(&db, "").len(), 1);
    assert!(
        db.folder_index_is_current().expect("marker"),
        "nothing has moved since the rebuild"
    );

    // A later scan writes more rows. The index has to say so, or the page would go on answering for
    // a corpus that no longer exists with nothing to notice it.
    add(&mut db, "b", Some("B"), "mpb/b.kar");
    assert!(
        !db.folder_index_is_current().expect("marker"),
        "rows arrived, so the index is behind"
    );
    assert_eq!(
        db.folders("").expect("folders").len(),
        1,
        "and reading it alone does not catch it up"
    );

    let top = listing(&db, "");
    assert_eq!(
        top.iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["mpb", "rock"]
    );
}

#[test]
fn an_unchanged_corpus_is_read_from_the_index_rather_than_recomputed() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    listing(&db, "");

    // Reaching past the API to prove the fast path is taken: with `files` unchanged, a listing
    // must not recompute, so a row removed from the index stays removed. This asserts a *stale*
    // answer on purpose — it is the only way to show the group-by is not being run again, which
    // is the entire point of the table.
    db.execute_for_test("DELETE FROM folders WHERE path = 'rock/'")
        .expect("delete");
    assert!(
        db.folders("").expect("folders").is_empty(),
        "the listing came from the table, not from files"
    );

    // And an explicit rebuild puts it back.
    assert_eq!(db.rebuild_folders().expect("rebuild"), 2, "root and rock/");
    assert_eq!(db.folders("").expect("folders").len(), 1);
}

/// A rebuild asked to stop keeps the last tree and leaves it marked stale for the page to rebuild.
#[test]
fn a_stopped_folder_rebuild_writes_nothing_and_leaves_the_index_stale() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    listing(&db, "");
    add(&mut db, "b", Some("B"), "mpb/b.kar");
    assert!(!db.folder_index_is_current().expect("marker"));

    assert_eq!(db.rebuild_folders_unless(|| true).expect("rebuild"), None);
    assert_eq!(
        db.folders("")
            .expect("folders")
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["rock"],
        "the tree built before the stop is the one still there"
    );
    assert!(!db.folder_index_is_current().expect("marker"));

    assert_eq!(
        db.rebuild_folders().expect("rebuild"),
        3,
        "root, mpb/ and rock/"
    );
    assert!(db.folder_index_is_current().expect("marker"));
}

/// The folder pass streams in song order, so a stop checked between rows reaches all of it.
///
/// A sort would run in full before the first row arrived, and on a whole corpus that sort is where a
/// stop would go unheard.
#[test]
fn the_folder_pass_reads_in_index_order_without_a_sort() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    db.refresh_statistics();
    let plan = db
        .plan_for_test(&songs::folder_pass_sql(), &[])
        .expect("plan");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
}

/// A folder's count says how many songs clicking it shows, so a deleted song is in neither.
///
/// **The marker is the half that fails silently.** A delete leaves `files` alone and moves neither
/// the top rowid nor the last scan, so the index would go on answering the number it had — with the
/// folder's own list already one song shorter.
#[test]
fn a_deleted_song_leaves_the_folder_counts_as_well_as_the_list() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    add(&mut db, "b", Some("B"), "rock/b.kar");

    let counts = |db: &Db| {
        listing(db, "")
            .iter()
            .map(|node| (node.name.clone(), node.song_count))
            .collect::<Vec<_>>()
    };
    assert_eq!(counts(&db), vec![("rock".to_owned(), 2)]);

    assert_eq!(
        db.set_deleted_of(&["a".to_owned()], true).expect("delete"),
        1
    );
    assert!(
        !db.folder_index_is_current().expect("marker"),
        "a delete changes what the pass would count, so the index has to be rebuilt"
    );
    assert_eq!(counts(&db), vec![("rock".to_owned(), 1)]);
    assert_eq!(
        db.songs(&Filter {
            folder: Some("rock/".to_owned()),
            ..Filter::default()
        })
        .expect("browse")
        .len(),
        1,
        "the count and the list it promises are one number"
    );

    // And bringing it back puts the song into both again.
    assert_eq!(
        db.set_deleted_of(&["a".to_owned()], false)
            .expect("undelete"),
        1
    );
    assert_eq!(counts(&db), vec![("rock".to_owned(), 2)]);
}

#[test]
fn folder_paths_split_into_parents_and_names() {
    assert_eq!(parent_folder("rock/deep/a.kar"), "rock/deep/");
    assert_eq!(parent_folder("a.kar"), "", "a file at the root");
    assert_eq!(
        ancestors("rock/deep/"),
        vec!["".to_owned(), "rock/".to_owned(), "rock/deep/".to_owned()]
    );
    assert_eq!(ancestors(""), vec![String::new()], "the root is its own");
    assert_eq!(parent_of("rock/deep/").as_deref(), Some("rock/"));
    assert_eq!(parent_of("rock/").as_deref(), Some(""));
    assert_eq!(parent_of(""), None, "the root has no parent");
    assert_eq!(folder_name("rock/deep/"), "deep");
    assert_eq!(folder_name("rock/"), "rock");
    assert_eq!(folder_name(""), "");
}

/// The link beside a copy on the song page is the Folders page's *only this folder* link.
///
/// A file at the root has no link at all: `folder=` is not a filter, so a link built from it
/// would say "this folder" and show the whole corpus.
#[test]
fn a_copy_links_to_the_song_list_for_its_own_folder() {
    let file = |path: &str| SongFile {
        path: path.to_owned(),
        size: 1,
    };
    assert_eq!(file("rock/deep/a.kar").folder(), "rock/deep/");
    assert_eq!(
        file("rock/deep/a.kar").folder_url(),
        "/songs?folder=rock%2Fdeep%2F"
    );
    // A corpus made of other people's folders has spaces and ampersands in them.
    assert_eq!(
        file("Rock & Roll/a.kar").folder_url(),
        "/songs?folder=Rock+%26+Roll%2F"
    );
    assert_eq!(file("a.kar").folder(), "");
    assert_eq!(file("a.kar").folder_url(), "", "no link at the root");
}

#[test]
fn a_prefix_range_covers_exactly_what_is_under_it() {
    let (low, high) = prefix_range("rock/");
    assert_eq!(low, "rock/");
    assert!("rock/a.kar" >= low.as_str() && "rock/a.kar" < high.as_str());
    assert!("rock/z/z.kar" < high.as_str());
    assert!("rockabilly/a.kar" >= high.as_str());
    // An empty prefix must not exclude anything.
    let (low, high) = prefix_range("");
    assert!("anything" >= low.as_str() && "anything" < high.as_str());
}

/// The slow part of an open says so to whoever asked, not only to a console.
///
/// This is the test the frozen phase needed. `Opening::phase` was written once at construction
/// and never again, so the Open page reported "opening the database" for however long an open
/// ran — while `prepare`, which knows exactly when the expensive window opens and closes,
/// was telling a console the windowed build does not have.
///
/// Reaching that window needs both halves of `announce`: more than ten thousand songs, and an
/// index this build would have to construct. The rows go in through one recursive statement
/// rather than the `add` helper — ten thousand songs through the scan model is a slow test, and
/// nothing here cares what is in them, only how many.
#[test]
fn the_slow_part_of_an_open_says_so_to_whoever_asked() {
    let scratch = Scratch::new("indexing-phase");
    let dir = scratch.0.clone();

    {
        let db = Db::create(&dir).expect("create");
        db.execute_for_test(
            "WITH RECURSIVE n(i) AS (
                 SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10001
             )
             INSERT INTO songs (id, duration_ms, first_seen, last_scanned)
             SELECT 'song-' || i, 200000, '2026-08-31T00:00:00Z', '2026-08-31T00:00:00Z'
             FROM n;",
        )
        .expect("a corpus large enough to be worth announcing");
        db.execute_for_test("DROP INDEX songs_browse_language_artist;")
            .expect("an index the next open has to build");
    }

    let said = std::sync::Mutex::new(Vec::new());
    let db = Db::open_saying(&dir, &|phase| {
        said.lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(phase)
    })
    .expect("reopen");
    drop(db);

    let said = said.into_inner().unwrap_or_else(|error| error.into_inner());
    assert!(
        said.contains(&OpeningPhase::Indexing { missing: 1 }),
        "the open never said what it was doing, so the page shows its opening sentence for the \
         whole run: {said:?}"
    );
    assert!(
        said.contains(&OpeningPhase::Finishing),
        "the page is left promising several minutes of index building after they are over: \
         {said:?}"
    );
}

/// ...and an open with nothing slow in it claims nothing slow.
///
/// The other half, and the one that keeps the first from being satisfied by a sink called
/// unconditionally. What it holds is the *claim*, not the silence: every step an open takes says
/// which one it is, because a sentence that does not change is the fault all of this exists to
/// remove, and a folder that opens in a moment passes through them too quickly to read. What it may
/// not do is promise several minutes of index building on a folder that has none to do.
#[test]
fn a_quick_open_claims_nothing_slow() {
    let scratch = Scratch::new("quick-open");
    let dir = scratch.0.clone();
    drop(Db::create(&dir).expect("create"));

    let said = std::sync::Mutex::new(Vec::new());
    drop(
        Db::open_saying(&dir, &|phase| {
            said.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(phase)
        })
        .expect("reopen"),
    );

    let said = said.into_inner().unwrap_or_else(|error| error.into_inner());
    assert!(
        !said.iter().any(|phase| matches!(
            phase,
            OpeningPhase::Indexing { .. } | OpeningPhase::Folding { .. }
        )),
        "a folder that opens in a moment warned about work it was not doing: {said:?}"
    );
    assert_eq!(
        said.last(),
        Some(&OpeningPhase::Finishing),
        "the page is left on whichever step happened to be last rather than on the one true \
         ending: {said:?}"
    );
}

/// Every stretch of an open says which one it is.
///
/// **The repairs after the version check are where the minutes go**, so a stretch of them that says
/// nothing is a page sitting on the seed sentence for the whole of it. The assertion is
/// deliberately about the *shape*: more than one sentence, and none of them the seed. A backfill
/// added later with no sentence of its own leaves the page holding the one before it, which is this
/// fault coming back one step along.
#[test]
fn every_stretch_of_an_open_says_which_one_it_is() {
    let scratch = Scratch::new("open-phases");
    let dir = scratch.0.clone();

    {
        let mut db = Db::create(&dir).expect("create");
        add(&mut db, "corcovado", Some("Corcovado"), "a/CORCOVAD.kar");
    }

    let said = std::sync::Mutex::new(Vec::new());
    drop(
        Db::open_saying(&dir, &|phase| {
            said.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(phase)
        })
        .expect("reopen"),
    );

    let said = said.into_inner().unwrap_or_else(|error| error.into_inner());
    assert!(
        said.len() > 2,
        "one sentence covers the whole open, which is indistinguishable from a hang: {said:?}"
    );
    assert!(
        !said.contains(&OpeningPhase::Database),
        "a step said the seed sentence back, which tells a page nothing has moved: {said:?}"
    );
}

/// A backup of every row would be a hundred megabytes of nulls on a real corpus, so the
/// predicate is the feature. The favorited-but-otherwise-untouched song is the case it is easy
/// to leave out, and leaving it out loses a decision somebody made.
#[test]
fn the_hand_set_songs_are_the_ones_somebody_touched_and_no_others() {
    let mut db = db();
    add(&mut db, "untouched", Some("Untouched"), "a/untouched.kar");
    add(&mut db, "renamed", Some("Renamed"), "a/renamed.kar");
    add(&mut db, "rated", Some("Rated"), "a/rated.kar");
    add(&mut db, "filed", Some("Filed"), "a/filed.kar");

    db.edit_song(
        "renamed",
        &SongEdit {
            title: Some(Some("Corcovado".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");
    db.set_user_score("rated", Some(9)).expect("rate");
    let favorite = db.create_favorite("Brasil").expect("favorite");
    db.set_favorite("filed", favorite, true).expect("file it");

    let found: Vec<String> = db
        .hand_set_songs()
        .expect("hand-set")
        .into_iter()
        .map(|song| song.id)
        .collect();
    assert_eq!(
        found,
        ["filed", "rated", "renamed"],
        "a song nobody has said anything about is not in a backup, and one that is only \
         favorited is"
    );
    assert_eq!(db.hand_set_count().expect("count"), 3);

    let named = db.hand_set_songs().expect("hand-set");
    let renamed = named.iter().find(|song| song.id == "renamed").expect("row");
    assert_eq!(
        renamed.seen_as, "Corcovado",
        "the effective title, so a report can name a song rather than print its hash"
    );
}

/// One level only. A chain would put "the song this really is" two hops away, which every query
/// that hides a merged song assumes cannot happen.
#[test]
fn a_merge_target_that_is_itself_merged_is_refused() {
    let mut db = db();
    add(&mut db, "aaa", Some("One"), "a/one.kar");
    add(&mut db, "bbb", Some("Two"), "a/two.kar");
    add(&mut db, "ccc", Some("Three"), "a/three.kar");

    assert!(db.set_merged_into("bbb", Some("aaa")).expect("merge"));
    assert!(
        !db.set_merged_into("ccc", Some("bbb")).expect("refused"),
        "merging into a song that is itself merged is refused rather than chained"
    );
    assert_eq!(db.song("ccc").expect("song").merged_into, None);

    let Err(error) = db.set_merged_into("aaa", Some("aaa")) else {
        panic!("a song merged into itself must be refused");
    };
    assert!(error.to_string().contains("itself"), "{error}");

    assert!(db.set_merged_into("bbb", None).expect("unmerge"));
    assert_eq!(db.song("bbb").expect("song").merged_into, None);
}

/// The rowid a favorite has here means nothing in another database, so a backup carries the name —
/// and a name is only useful if it comes back whole, whatever somebody put in it.
#[test]
fn a_favorite_reads_back_as_the_name_it_is() {
    let db = db();
    db.create_favorite("Brasil").expect("one");
    let mixed = db.create_favorite("Rock/Pop & Soul / 80s").expect("two");
    db.execute_for_test(
        "INSERT INTO songs(id, kind, duration_ms, warnings, first_seen, last_scanned)
         VALUES ('abc', 'midi', 1, '[]', 'now', 'now')",
    )
    .expect("a song to file");
    db.set_favorite("abc", mixed, true).expect("file it");

    let names: Vec<String> = db
        .favorite_names()
        .expect("names")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        ["Brasil".to_owned(), "Rock/Pop & Soul / 80s".to_owned()],
        "a name is carried as somebody typed it, slashes and all"
    );

    let memberships = db.favorite_memberships().expect("memberships");
    assert_eq!(memberships["abc"], ["Rock/Pop & Soul / 80s".to_owned()]);
}

/// What `songs.updated_at` holds, straight from the column: no browse row and no page carries it.
fn stamp(db: &Db, id: &str) -> Option<String> {
    db.conn
        .query_row("SELECT updated_at FROM songs WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("the song")
}

/// Puts a song back to never-edited without going through a write path.
///
/// `updated_at` is in no trigger's `UPDATE OF` list, so writing it directly stamps nothing — which
/// is what lets each case below start from NULL and assert only its own path.
fn clear_stamp(db: &Db, id: &str) {
    db.conn
        .execute("UPDATE songs SET updated_at = NULL WHERE id = ?1", [id])
        .expect("clear");
}

#[test]
fn a_fresh_scan_leaves_the_stamp_unset() {
    let mut db = db();
    add(&mut db, "abc", Some("Song"), "folder/song.kar");
    assert_eq!(
        stamp(&db, "abc"),
        None,
        "a scanned song is not an edited one"
    );

    // The same file again, which is what a scan of an unchanged corpus does to every row it
    // revisits.
    add(&mut db, "abc", Some("Song"), "folder/song.kar");
    assert_eq!(
        stamp(&db, "abc"),
        None,
        "and a rescan is not an edit either"
    );
}

#[test]
fn an_edit_stamps_the_song_and_a_rescan_does_not() {
    let mut db = db();
    add(&mut db, "abc", Some("Detected"), "folder/song.kar");
    db.edit_song(
        "abc",
        &SongEdit {
            title: Some(Some("Corrected".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    let stamped = stamp(&db, "abc").expect("an edited song carries a stamp");
    assert_eq!(stamped.len(), 20, "{stamped}");
    assert!(stamped.ends_with('Z'), "{stamped}");
    assert_eq!(stamped.as_bytes()[10], b'T', "{stamped}");
    assert!(
        stamped
            .chars()
            .all(|character| character.is_ascii_digit() || "-:TZ".contains(character)),
        "{stamped}"
    );

    add(&mut db, "abc", Some("Detected again"), "folder/song.kar");
    assert_eq!(
        stamp(&db, "abc").as_deref(),
        Some(stamped.as_str()),
        "a rescan rewrites what the file says, not when anybody last said anything"
    );
}

/// One way of somebody editing a song, as the table below calls it.
type WritePath = fn(&mut Db, &str);

/// Every path that writes a hand-set column stamps the song, whichever statement it is.
///
/// A table rather than eight tests, because what it guards is the *set*: the trigger names columns
/// and the paths that write them are nine separate statements, so a tenth added outside those
/// columns is invisible until somebody sorts by the stamp and finds their work missing from the
/// top. Each case starts from a cleared stamp, so none can pass on another's work.
#[test]
fn every_hand_set_write_path_stamps() {
    let mut db = db();
    add(
        &mut db,
        "target",
        Some("The same recording"),
        "folder/a.kar",
    );

    let paths: [(&str, WritePath); 7] = [
        ("edit_song", |db, id| {
            db.edit_song(
                id,
                &SongEdit {
                    notes: Some(Some("worth a look".to_owned())),
                    ..SongEdit::default()
                },
            )
            .expect("edit");
        }),
        ("set_user_score", |db, id| {
            db.set_user_score(id, Some(9)).expect("score");
        }),
        ("set_names_from_stem", |db, id| {
            db.set_names_from_stem(&[id.to_owned()]).expect("titles");
        }),
        ("set_language_of", |db, id| {
            db.set_language_of(&[id.to_owned()], Language::parse("pt"), false)
                .expect("language");
        }),
        // Over the whole match set, which is what the bar's language control does. It stamps the
        // other songs in the fixture too; each case asserts only its own.
        ("set_language_for", |db, _id| {
            db.set_language_for(&Filter::default(), Language::parse("en"))
                .expect("language");
        }),
        ("set_merged_into", |db, id| {
            db.set_merged_into(id, Some("target")).expect("merge");
        }),
        // Merged first, because the `WHEN` is what stops clearing an already-clear column being an
        // edit — and the stamp the merge itself left is cleared so this case tests only the unmerge.
        ("unmerge", |db, id| {
            db.set_merged_into(id, Some("target")).expect("merge");
            clear_stamp(db, id);
            db.unmerge(id).expect("unmerge");
        }),
    ];

    for (number, (name, write)) in paths.iter().enumerate() {
        let id = format!("song-{number}");
        add(
            &mut db,
            &id,
            Some("Detected"),
            &format!("folder/song-{number}.kar"),
        );
        clear_stamp(&db, &id);
        write(&mut db, &id);
        assert!(
            stamp(&db, &id).is_some(),
            "{name} is somebody editing a song, so it has to stamp one"
        );
    }
}

/// The trigger's column list and `backup::HAND_SET_COLUMNS` say the same thing.
///
/// Neither can be derived from the other — one is SQL text in `schema.sql` and the other a Rust
/// `const` — so they are read back and compared. The exclusion half is the one that costs a corpus
/// if it goes wrong: every column named below is rewritten by `write_scanned` or by an open-time
/// repair, so a trigger watching one of them stamps every song in a corpus as edited the first time
/// it is rescanned.
#[test]
fn the_stamp_trigger_watches_every_hand_set_column() {
    let db = db();
    let sql: String = db
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'songs_stamp_update'",
            [],
            |row| row.get(0),
        )
        .expect("the trigger");

    let (head, _body) = sql.split_once("BEGIN").expect("a trigger body");
    let (watched, guard) = head.split_once("WHEN").expect("a WHEN guard");
    let watched = watched
        .split_once("UPDATE OF")
        .expect("an UPDATE OF list")
        .1;
    // Split into names rather than searched as text. `lyrics_hidden` contains `lyrics`, so a
    // substring test reads the watched list as naming a column written by every scan and fails a
    // trigger that is correct.
    let watched_names: Vec<&str> = watched
        .trim()
        .trim_end_matches("ON songs")
        .split(',')
        .map(str::trim)
        .collect();

    for column in crate::backup::HAND_SET_COLUMNS {
        assert!(
            watched_names.contains(column),
            "{column} is hand-set, so writing it is an edit: name it in UPDATE OF"
        );
        assert!(
            guard.contains(&format!("new.{column}")),
            "{column} is in UPDATE OF, so the WHEN has to compare it or a write that changed \
             nothing still stamps"
        );
    }

    for written_by_a_machine in [
        "det_title",
        "det_artist",
        "det_language",
        "det_language_tag",
        "stem",
        "lyrics",
        "suitability",
        "file_count",
        "sort_title",
        "sort_artist",
        "first_seen",
        "last_scanned",
    ] {
        assert!(
            !watched_names.contains(&written_by_a_machine),
            "{written_by_a_machine} is written by a scan or a repair, so watching it stamps a \
             whole corpus as edited the first time it is rescanned"
        );
    }
}

#[test]
fn writing_the_same_value_is_not_an_edit() {
    let mut db = db();
    add(&mut db, "abc", Some("Song"), "folder/song.kar");
    db.set_language_of(&["abc".to_owned()], Language::parse("pt"), false)
        .expect("language");
    let stamped = stamp(&db, "abc").expect("the first one is an edit");

    // The bar's language control writes every matching row whether or not it already holds that
    // language, so without the `WHEN` this would be an edit — and so would saving a song's form
    // without touching a box on it.
    db.set_language_of(&["abc".to_owned()], Language::parse("pt"), false)
        .expect("language again");
    assert_eq!(
        stamp(&db, "abc").as_deref(),
        Some(stamped.as_str()),
        "writing the value a song already holds changes nothing about the song"
    );
}

/// A tag and a favorite are filed in tables of their own and leave the stamp alone.
///
/// The one place this column is narrower than what the backup carries, so it is asserted rather
/// than left to be discovered. See `When a song was last edited` in `docs/decisions/curation.md`.
#[test]
fn filing_a_song_does_not_stamp_it() {
    let mut db = db();
    add(&mut db, "abc", Some("Song"), "folder/song.kar");
    let bossa = db.create_favorite("Bossa").expect("create");
    db.toggle_favorite("abc", bossa).expect("file it");
    db.add_tag_of(&["abc".to_owned()], &Tag::parse("live").expect("a tag"))
        .expect("tag it");

    assert_eq!(
        stamp(&db, "abc"),
        None,
        "filing a song is not a change to the song's own record"
    );
}

/// The stamp SQLite writes is the one `crate::scan::timestamp()` writes.
///
/// Bracketed rather than compared, so there is no clock to race: the SQL value has to fall between
/// two Rust ones taken either side of it. Compared **as strings**, which is the assertion that
/// earns its keep twice — it proves the two formats agree character for character, and it proves
/// that ordering the column as text orders it in time, which is what lets one plain index serve the
/// sort.
#[test]
fn the_sql_stamp_is_the_one_this_crate_writes() {
    let db = db();
    let before = crate::scan::timestamp();
    let sql: String = db
        .conn
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
            row.get(0)
        })
        .expect("the clock");
    let after = crate::scan::timestamp();

    assert!(
        before <= sql && sql <= after,
        "{before} <= {sql} <= {after}"
    );
}

/// One bulk edit lands one stamp, however many rows it touches.
///
/// `'now'` is fixed for one step of a statement, so the filter-wide language set gives every row it
/// changes the same value rather than smearing them across the seconds the statement took — which
/// is what makes one action one block at the top of the sort.
#[test]
fn a_bulk_language_set_gives_every_row_one_stamp() {
    let mut db = db();
    for number in 0..20 {
        add(
            &mut db,
            &format!("song-{number:02}"),
            Some(&format!("Song {number:02}")),
            &format!("folder/song-{number:02}.kar"),
        );
    }
    db.set_language_for(&Filter::default(), Language::parse("pt"))
        .expect("language");

    let stamped: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM songs WHERE updated_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("count");
    let distinct: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(DISTINCT updated_at) FROM songs WHERE updated_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(stamped, 20, "every song matched the filter");
    assert_eq!(distinct, 1, "and one action is one time");
}

// -- filters somebody named ----------------------------------------------------------------------

#[test]
fn a_fresh_database_has_no_saved_filters() {
    // The table is `schema.sql`'s, which runs in full on every open, so it is there without a
    // migration step having been reached for.
    assert!(db().saved_filters().expect("read").is_empty());
}

#[test]
fn a_saved_filter_comes_back_under_its_name() {
    let db = db();
    db.save_filter(
        "Portuguese, unclassified",
        "language=unset&folder=Brasil/",
        "2026-09-11T10:00:00Z",
    )
    .expect("save");

    let saved = db.saved_filters().expect("read");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Portuguese, unclassified");
    assert_eq!(saved[0].query, "language=unset&folder=Brasil/");
}

/// The whole-corpus case, which has to survive as itself rather than as *nothing was saved*.
#[test]
fn an_empty_query_is_a_legal_saved_filter() {
    let db = db();
    db.save_filter("Everything", "", "2026-09-11T10:00:00Z")
        .expect("save");
    assert_eq!(db.saved_filters().expect("read")[0].query, "");
}

/// A name is a key, so a second write replaces the first rather than sitting beside it.
#[test]
fn saving_under_a_name_that_is_taken_replaces_its_query() {
    let db = db();
    let first = db
        .save_filter("Videos", "kind=video", "2026-09-11T10:00:00Z")
        .expect("save");
    let second = db
        .save_filter(
            "Videos",
            "kind=video&language=unset",
            "2026-09-11T11:00:00Z",
        )
        .expect("save again");

    assert_eq!(first, second, "the same row, not a second one");
    let saved = db.saved_filters().expect("read");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].query, "kind=video&language=unset");
}

/// …and the confirmation has to be able to name what it is about to replace.
#[test]
fn a_name_that_is_taken_is_found_before_it_is_replaced() {
    let db = db();
    db.save_filter("Videos", "kind=video", "2026-09-11T10:00:00Z")
        .expect("save");

    let found = db
        .saved_filter_named("Videos")
        .expect("look")
        .expect("there");
    assert_eq!(found.query, "kind=video");
    assert!(db.saved_filter_named("Nothing").expect("look").is_none());
}

/// A blank name is the one thing the write refuses: a chip with nothing written on it cannot be told
/// from the next one.
#[test]
fn a_saved_filter_needs_a_name() {
    let db = db();
    assert!(
        db.save_filter("   ", "kind=video", "2026-09-11T10:00:00Z")
            .is_err()
    );
    assert!(db.saved_filters().expect("read").is_empty());
}

/// Ordered by the fold, which is what an accent and a capital make the difference between. Ordering
/// on the name itself gives `Bossa`, `agora`, `Ágil` — SQLite comparing bytes, where `B` is `0x42`
/// and `Á` starts `0xC3`.
#[test]
fn saved_filters_come_back_in_the_order_a_reader_expects() {
    let db = db();
    for name in ["Bossa", "Ágil", "agora"] {
        db.save_filter(name, "", "2026-09-11T10:00:00Z")
            .expect("save");
    }
    let names: Vec<String> = db
        .saved_filters()
        .expect("read")
        .into_iter()
        .map(|filter| filter.name)
        .collect();
    assert_eq!(names, vec!["Ágil", "agora", "Bossa"]);
}

#[test]
fn forgetting_a_saved_filter_leaves_the_others() {
    let db = db();
    let id = db
        .save_filter("Videos", "kind=video", "2026-09-11T10:00:00Z")
        .expect("save");
    db.save_filter("Everything", "", "2026-09-11T10:00:00Z")
        .expect("save");

    assert!(db.delete_saved_filter(id).expect("delete"));
    // Already gone is not a failure: two tabs on one strip is the ordinary way it happens.
    assert!(!db.delete_saved_filter(id).expect("delete again"));

    let left = db.saved_filters().expect("read");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].name, "Everything");
}

#[test]
fn a_saved_filter_can_be_given_a_new_query_under_the_same_name() {
    let db = db();
    let id = db
        .save_filter("Curating", "language=pt", "2026-09-11T10:00:00Z")
        .expect("save");

    assert!(
        db.update_saved_filter(id, "language=pt&favorited=out", "2026-09-11T11:00:00Z")
            .expect("update")
    );

    let held = db.saved_filters().expect("read");
    assert_eq!(held.len(), 1, "the same row, not a second one");
    assert_eq!(held[0].id, id);
    assert_eq!(held[0].name, "Curating");
    assert_eq!(held[0].query, "language=pt&favorited=out");
}

/// Already gone is not a failure, which is what every other write to the strip answers too.
#[test]
fn rewriting_a_saved_filter_that_has_gone_says_so_rather_than_failing() {
    let db = db();
    let id = db
        .save_filter("Curating", "language=pt", "2026-09-11T10:00:00Z")
        .expect("save");
    assert!(db.delete_saved_filter(id).expect("delete"));

    assert!(
        !db.update_saved_filter(id, "kind=video", "2026-09-11T11:00:00Z")
            .expect("update")
    );
}

#[test]
fn renaming_a_saved_filter_keeps_its_query_and_reorders_the_strip() {
    let db = db();
    let id = db
        .save_filter("Zebra", "kind=video", "2026-09-11T10:00:00Z")
        .expect("save");
    db.save_filter("Bossa", "language=pt", "2026-09-11T10:00:00Z")
        .expect("save");

    db.rename_saved_filter(id, "Agora").expect("rename");

    let held = db.saved_filters().expect("read");
    let names: Vec<String> = held.iter().map(|filter| filter.name.clone()).collect();
    assert_eq!(names, vec!["Agora", "Bossa"], "the fold is rewritten too");
    assert_eq!(held[0].query, "kind=video", "and the query is untouched");
}

/// **Refused, where saving under a taken name replaces.** A save writes a query somebody is looking
/// at into a name, so the two queries can be shown and a choice put. A rename writes a name over a
/// query that is not on screen, and the row it landed on would simply go.
#[test]
fn renaming_a_saved_filter_onto_a_name_that_is_taken_is_refused() {
    let db = db();
    let id = db
        .save_filter("Zebra", "kind=video", "2026-09-11T10:00:00Z")
        .expect("save");
    db.save_filter("Bossa", "language=pt", "2026-09-11T10:00:00Z")
        .expect("save");

    let refused = db.rename_saved_filter(id, "Bossa").expect_err("refused");
    assert!(refused.to_string().contains("Bossa"), "{refused}");

    let held = db.saved_filters().expect("read");
    assert_eq!(held.len(), 2, "and neither row went");

    // Its own name is not a collision with itself.
    db.rename_saved_filter(id, "Zebra").expect("rename");
}

#[test]
fn a_renamed_saved_filter_still_needs_a_name() {
    let db = db();
    let id = db
        .save_filter("Zebra", "kind=video", "2026-09-11T10:00:00Z")
        .expect("save");
    assert!(db.rename_saved_filter(id, "  ").is_err());
    assert_eq!(db.saved_filters().expect("read")[0].name, "Zebra");
}

/// A curator's corrections are stored, and "nobody has said" is a distinct third state.
///
/// The three states are what the column is for: NULL leaves the song to detection, an empty array
/// refuses what detection proposes, and a list is a decision. A column that could only be empty or
/// full would make the first two the same thing, and a song nobody had touched would stop learning.
#[test]
fn a_songs_corrections_are_stored_and_can_be_handed_back_to_detection() {
    let mut db = db();
    add_built(&mut db, "song-a", Some("Wave"), "a/WAVE.kar", |_| {});
    assert_eq!(db.song("song-a").expect("song").fixes, None);

    let stored = r#"[{"fix":"mute_channel","channel":2}]"#;
    db.edit_song(
        "song-a",
        &SongEdit {
            fixes: Some(Some(stored.to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");
    assert_eq!(
        db.song("song-a").expect("song").fixes.as_deref(),
        Some(stored)
    );

    // An empty list is a decision and stays one.
    db.edit_song(
        "song-a",
        &SongEdit {
            fixes: Some(Some("[]".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");
    assert_eq!(
        db.song("song-a").expect("song").fixes.as_deref(),
        Some("[]")
    );

    // And back to nobody having said.
    db.edit_song(
        "song-a",
        &SongEdit {
            fixes: Some(None),
            ..SongEdit::default()
        },
    )
    .expect("edit");
    assert_eq!(db.song("song-a").expect("song").fixes, None);
}

/// Editing the corrections stamps `updated_at`, which needs both halves of the trigger.
///
/// The trigger names its columns twice — once in `AFTER UPDATE OF` and once in the `WHEN` — and a
/// column added to one and not the other stamps nothing, silently.
#[test]
fn changing_the_corrections_stamps_the_song() {
    let mut db = db();
    add_built(&mut db, "song-a", Some("Wave"), "a/WAVE.kar", |_| {});
    clear_stamp(&db, "song-a");
    assert_eq!(stamp(&db, "song-a"), None);

    db.edit_song(
        "song-a",
        &SongEdit {
            fixes: Some(Some(r#"[{"fix":"mute_channel","channel":2}]"#.to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("edit");

    assert!(
        stamp(&db, "song-a").is_some(),
        "the trigger did not stamp the song, so one half of it is missing the column"
    );
}

/// One scanned MIDI song's parse facts, for a test that cares about its suitability.
fn midi_facts() -> crate::model::MidiFacts {
    crate::model::MidiFacts {
        flavor: "lyric_events".to_owned(),
        granularity: "syllablelevel".to_owned(),
        note_count: 900,
        channel_count: 8,
        line_count: 40,
        syllable_count: 300,
        det_encoding: "windows-1252".to_owned(),
        det_encoding_source: "detected".to_owned(),
        melody_channel: Some(3),
        melody_confidence: Some(0.9),
        melody_abstained: None,
    }
}

/// A suitability saying what a test needs it to say.
fn suitability_of(value: u8, breakdown: (u8, u8, u8, u8)) -> crate::model::SuitabilityFacts {
    crate::model::SuitabilityFacts {
        value,
        breakdown,
        warnings: "[]".to_owned(),
    }
}

/// A quality hint carries only MIDI songs, however many of the ticks are something else.
///
/// A video song and an MP3+G song are a flat 10 by what they are, and every component behind that
/// 10 is a fill rather than a reading — so ordering them would be the id tie-break wearing a badge.
/// The handler counts what comes back against what was asked for and says how many it left out.
#[test]
fn a_quality_hint_leaves_out_what_is_not_a_midi_file() {
    let scratch = Scratch::new("hint-midi-only");
    let mut db = Db::create(&scratch.0).expect("create");

    let video = crate::model::VideoFacts {
        width: 1_920,
        height: 1_080,
        frame_rate_milli: 29_970,
        video_codec: "h264".to_owned(),
        audio_codec: "aac".to_owned(),
    };
    let cdg = crate::model::CdgFacts {
        graphics_path: "a/song.cdg".to_owned(),
        sample_rate: 44_100,
        channels: 2,
        packets: 10_000,
        graphics_ms: 200_000,
        graphics_short_by_ms: 0,
        tiles_written: 4_000,
        unknown_instructions: 0,
    };

    db.write_scanned(
        &[
            scanned(
                "a-midi",
                suitability_of(9, (3, 2, 2, 2)),
                Some(midi_facts()),
                None,
                None,
            ),
            scanned(
                "b-video",
                suitability_of(10, (3, 3, 2, 2)),
                None,
                Some(video),
                None,
            ),
            scanned(
                "c-cdg",
                suitability_of(10, (3, 3, 2, 2)),
                None,
                None,
                Some(cdg),
            ),
        ],
        "2026-09-07T00:00:00Z",
    )
    .expect("write");

    let ids = [
        "a-midi".to_owned(),
        "b-video".to_owned(),
        "c-cdg".to_owned(),
    ];
    assert_eq!(
        db.quality_hint(&ids).expect("hint"),
        vec!["a-midi".to_owned()]
    );
}

/// The hint orders by the components behind the suitability, not by the suitability alone.
///
/// This is what it exists for: 89% of the duplicate groups on a real corpus have their top
/// suitability tied, so a hint reading only that column would be the id tie-break with a badge on
/// it. Two songs at 8 whose lyrics components differ must not come back in id order.
#[test]
fn two_songs_tied_on_suitability_are_separated_by_their_components() {
    let scratch = Scratch::new("hint-components");
    let mut db = Db::create(&scratch.0).expect("create");

    db.write_scanned(
        &[
            // First by id, and the worse of the two where the words are concerned.
            scanned(
                "a-line-level",
                suitability_of(8, (1, 3, 2, 2)),
                Some(midi_facts()),
                None,
                None,
            ),
            scanned(
                "b-word-ends",
                suitability_of(8, (3, 1, 2, 2)),
                Some(midi_facts()),
                None,
                None,
            ),
        ],
        "2026-09-07T00:00:00Z",
    )
    .expect("write");

    let ids = ["a-line-level".to_owned(), "b-word-ends".to_owned()];
    assert_eq!(
        db.quality_hint(&ids).expect("hint"),
        vec!["b-word-ends".to_owned(), "a-line-level".to_owned()],
        "the lyrics component decides where the suitability ties"
    );
}

/// A rating somebody typed moves no badge.
///
/// It answers how much this song is wanted in a package rather than which copy of it is the better
/// file, so a 10 on the worse-measured of two copies must not put it first. See
/// `A quality hint is a position on the row, and it is rubbed out rather than kept` in
/// `docs/decisions/curation.md`.
#[test]
fn a_rating_somebody_typed_decides_nothing_about_which_to_play_first() {
    let scratch = Scratch::new("hint-user-score");
    let mut db = Db::create(&scratch.0).expect("create");

    db.write_scanned(
        &[
            scanned(
                "a-rated",
                suitability_of(5, (1, 1, 2, 1)),
                Some(midi_facts()),
                None,
                None,
            ),
            scanned(
                "b-measured",
                suitability_of(9, (3, 3, 2, 1)),
                Some(midi_facts()),
                None,
                None,
            ),
        ],
        "2026-09-07T00:00:00Z",
    )
    .expect("write");
    db.set_user_score("a-rated", Some(10)).expect("rate");

    let ids = ["a-rated".to_owned(), "b-measured".to_owned()];
    assert_eq!(
        db.quality_hint(&ids).expect("hint"),
        vec!["b-measured".to_owned(), "a-rated".to_owned()]
    );
}

// -- the connection a page is drawn through --------------------------------------------------

/// A second connection is opened read-only, and SQLite is what enforces that.
///
/// **The guard the type system cannot give.** `State::reading` hands out `&Db`, which stops a
/// closure reaching the seven methods that need the connection exclusively — but most writes here go
/// through `&self`, because `Connection::execute` does. So the open flag is the real fence, and this
/// is what says it is up.
#[test]
fn a_reading_connection_refuses_a_write() {
    let scratch = Scratch::new("reader-refuses-write");
    let _writer = Db::create(&scratch.0).expect("create");
    let reader = Db::open_reading(&scratch.0).expect("open for reading");

    let refused =
        reader.execute_for_test("INSERT INTO settings(key, value) VALUES ('reader', 'not here')");
    assert!(
        matches!(refused, Err(DbError::Sqlite(_))),
        "a write through the reading connection has to fail: {refused:?}"
    );
}

/// The reading connection sees what the writing one has committed.
///
/// Proves the snapshot moves. A reader pinned to the state it opened on would serve a corpus frozen
/// at whenever the folder was opened, which is the one way this arrangement could be worse than the
/// single connection it replaces.
#[test]
fn a_reading_connection_sees_what_the_writer_committed() {
    let scratch = Scratch::new("reader-sees-commits");
    let writer = Db::create(&scratch.0).expect("create");
    let reader = Db::open_reading(&scratch.0).expect("open for reading");

    assert!(reader.favorites().expect("favorites").is_empty());
    writer.create_favorite("Sunday").expect("favorite");
    assert_eq!(
        reader
            .favorites()
            .expect("favorites")
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Sunday"],
        "the reader is a connection, not a snapshot"
    );
}

/// The status bar's cache notices a commit made on the *other* connection.
///
/// **This is the test that fails without `PRAGMA data_version`.** `sqlite3_total_changes` counts
/// only what the connection asking has written, and the reading connection writes nothing ever — so
/// keyed on that alone the bar would show whatever was true when the folder was opened, for as long
/// as it stayed open, with nothing on the page to say so.
#[test]
fn the_counts_cache_notices_the_other_connections_commit() {
    let scratch = Scratch::new("counts-across-connections");
    let mut writer = Db::create(&scratch.0).expect("create");
    let reader = Db::open_reading(&scratch.0).expect("open for reading");

    assert_eq!(reader.counts().expect("counts").songs, 0);
    add(&mut writer, "a", Some("A"), "rock/a.kar");
    assert_eq!(
        reader.counts().expect("counts").songs,
        1,
        "the bar has to follow the writer"
    );
}

/// The five aggregates are the five fields, which is what makes them safe to time one at a time.
///
/// **The safety net under splitting `count_everything` into named functions.** The split exists so a
/// measurement can say which of the five a page is waiting for without carrying its own copy of the
/// SQL — see `db::measure` — and this is what stops the split drifting from the answer the status bar
/// actually shows.
#[test]
fn each_aggregate_answers_the_field_it_fills() {
    let mut db = db();
    add(&mut db, "a", Some("A"), "rock/a.kar");
    add(&mut db, "b", Some("B"), "mpb/b.kar");
    db.create_favorite("Sunday").expect("favorite");
    db.set_favorite("a", 1, true).expect("file it");

    let counts = db.counts().expect("counts");
    assert_eq!(counts.songs as i64, db.count_songs().expect("songs"));
    assert_eq!(counts.files as i64, db.count_files().expect("files"));
    assert_eq!(counts.failed as i64, db.count_failed().expect("failed"));
    assert_eq!(
        counts.favorites as i64,
        db.count_favorites().expect("favorites")
    );
    assert_eq!(
        counts.packages as i64,
        db.count_packages().expect("packages")
    );
}

/// The mapping a shipped build asks for is the figure the note's measurements were taken against.
///
/// `db::measure` can move it for the length of a run, which is the only way to compare the two
/// settings; this is what says the default did not move with it.
#[test]
fn a_shipped_connection_asks_for_the_mapping_the_note_measured() {
    let scratch = Scratch::new("shipped-mapping");
    let db = Db::create(&scratch.0).expect("create");
    assert_eq!(
        db.pragma_for_test("mmap_size").expect("mmap_size"),
        "1073741824",
        "the mapping a build ships with is what the research note's figures are about"
    );
}

/// And it still notices the connection's own writes, which is the half that was always there.
#[test]
fn the_counts_cache_still_notices_its_own_writes() {
    let mut db = db();
    assert_eq!(db.counts().expect("counts").songs, 0);
    add(&mut db, "a", Some("A"), "rock/a.kar");
    assert_eq!(db.counts().expect("counts").songs, 1);
}

/// A database outside write-ahead logging gets no second connection.
///
/// **The fallback is not a nicety.** The other journal modes are the ones where a writer excludes
/// readers outright, so a page drawn through a second connection there would wait out
/// `busy_timeout` and then answer "database is locked" — answered wrongly rather than answered late.
/// An in-memory database is in that group, which is why every test reaches this path.
#[test]
fn a_database_outside_wal_is_read_through_the_writing_connection() {
    assert!(
        !db().in_wal(),
        "an in-memory database has no journal to switch"
    );

    let scratch = Scratch::new("wal-took");
    let on_disk = Db::create(&scratch.0).expect("create");
    assert!(on_disk.in_wal(), "a file database takes WAL");
}

/// Leaving a language out takes its songs and keeps everything else, unclassified songs included.
///
/// **The unclassified half is the point.** `NOT IN` over a NULL is NULL rather than true, so a
/// clause without its own `IS NULL` leg would take every song nothing has placed along with the
/// language asked about — which on a corpus mid-classification is most of it, gone for a reason the
/// bar does not show.
#[test]
fn leaving_a_language_out_keeps_the_songs_nothing_has_placed() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_with_language(&mut db, "brazilian", Some("PORT"), "windows-1252");
    add_with_language(&mut db, "japanese", Some("ENGL"), "Shift_JIS");
    add_with_language(&mut db, "silent", None, "UTF-8");

    let without = |codes: &[&str]| {
        let rows = db
            .songs(&Filter {
                language_not: codes
                    .iter()
                    .map(|code| km_kmpkg::Language::parse(code).expect("a code"))
                    .collect(),
                ..Filter::default()
            })
            .expect("browse");
        let mut ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        ids.sort();
        ids
    };

    assert_eq!(
        without(&["pt"]),
        vec!["japanese".to_owned(), "silent".to_owned()],
        "the song nothing has placed is not in the language being left out"
    );
    assert_eq!(
        without(&["pt", "ja"]),
        vec!["silent".to_owned()],
        "several at once, which is what a corpus of unread folders needs"
    );
    assert_eq!(without(&[]).len(), 3, "nothing left out narrows nothing");
}

/// Leaving a language out reads the same three witnesses the column shows.
#[test]
fn leaving_a_language_out_follows_a_correction_and_a_guess() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_with_language(&mut db, "japanese", Some("ENGL"), "Shift_JIS");
    add_scanned(&mut db, "guessed", |song| {
        song.lyrics = Some(VIETNAMESE_VERSE.to_owned());
    });

    let without_japanese = |db: &Db| {
        let rows = db
            .songs(&Filter {
                language_not: vec![km_kmpkg::Language::parse("ja").expect("a code")],
                ..Filter::default()
            })
            .expect("browse");
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()
    };

    assert!(
        !without_japanese(&db).contains(&"japanese".to_owned()),
        "the file's own evidence is what it is left out by"
    );

    // A guessed language is left out by the same control, having become the song's language.
    let rows = db
        .songs(&Filter {
            language_not: vec![km_kmpkg::Language::parse("vi").expect("a code")],
            ..Filter::default()
        })
        .expect("browse");
    assert!(
        !rows.iter().any(|row| row.id == "guessed"),
        "a song placed by its words leaves the list with the language it was placed under"
    );

    // And a correction moves it, the chosen language outranking the detected one.
    db.edit_song(
        "japanese",
        &SongEdit {
            language: Some(Some("ko".to_owned())),
            ..SongEdit::default()
        },
    )
    .expect("correct it");
    assert!(
        without_japanese(&db).contains(&"japanese".to_owned()),
        "what somebody typed is what the exclusion reads"
    );
}

/// A song's own words place it, and the confidence comes back beside the code.
#[test]
fn a_scan_reads_the_words_of_a_song_the_file_says_nothing_about() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_scanned(&mut db, "vietnamese", |song| {
        song.det_language = None;
        song.lyrics = Some(VIETNAMESE_VERSE.to_owned());
    });
    add_scanned(&mut db, "wordless", |song| {
        song.det_language = None;
        song.lyrics = None;
    });

    let placed = db.song("vietnamese").expect("detail");
    assert_eq!(placed.det_language_guess.as_deref(), Some("vi"));
    assert!(
        placed.det_language_guess_confidence.unwrap_or_default() >= km_langguess::MIN_CONFIDENCE,
        "a stored guess is never below the gate"
    );
    assert!(
        placed.language_is_guessed(),
        "nothing else spoke for this song, so the words are what the column shows"
    );

    let unplaced = db.song("wordless").expect("detail");
    assert_eq!(
        unplaced.det_language_guess, None,
        "a song with nothing to read is honestly unclassified rather than guessed at"
    );
}

/// The stronger witnesses outrank the guess, in the order the column coalesces them.
#[test]
fn the_file_and_the_curator_both_outrank_what_the_words_read_as() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    // Shift-JIS bytes say Japanese; the words are Vietnamese. The encoding is the stronger witness.
    add_scanned(&mut db, "both", |song| {
        song.lyrics = Some(VIETNAMESE_VERSE.to_owned());
        if let Some(midi) = song.midi.as_mut() {
            midi.det_encoding = "Shift_JIS".to_owned();
        }
    });
    let detail = db.song("both").expect("detail");
    assert_eq!(detail.det_language_guess.as_deref(), Some("vi"));
    assert_eq!(detail.det_language_tag.as_deref(), Some("ja"));
    assert!(
        !detail.language_is_guessed(),
        "a file that spoke is what the column shows, whatever the words read as"
    );
    assert_eq!(
        db.songs(&Filter {
            language: LanguageFilter::parse("ja"),
            ..Filter::default()
        })
        .expect("browse")
        .len(),
        1,
        "and it is found under what the file said"
    );
}

/// The backfill reads rows a scan wrote before the guess, and runs once.
#[test]
fn the_guess_backfill_fills_rows_indexed_by_an_earlier_version() {
    let mut db = Db::open_in_memory(Path::new("/corpus")).expect("open");
    add_scanned(&mut db, "vietnamese", |song| {
        song.det_language = None;
        song.lyrics = Some(VIETNAMESE_VERSE.to_owned());
    });
    // What a database written before the guess existed holds.
    db.execute_for_test(
        "UPDATE songs SET det_language_guess = NULL, det_language_guess_confidence = NULL;
         DELETE FROM settings WHERE key = 'language_guess_revision';",
    )
    .expect("put it back");

    db.backfill_language_guess_for_test().expect("backfill");
    assert_eq!(
        db.song("vietnamese")
            .expect("detail")
            .det_language_guess
            .as_deref(),
        Some("vi")
    );

    // And a second run is a no-op, the revision having been written.
    db.execute_for_test("UPDATE songs SET det_language_guess = 'zz'")
        .expect("scribble");
    db.backfill_language_guess_for_test().expect("again");
    assert_eq!(
        db.song("vietnamese")
            .expect("detail")
            .det_language_guess
            .as_deref(),
        Some("zz"),
        "the revision is what stops a corpus being re-read at every open"
    );
}
