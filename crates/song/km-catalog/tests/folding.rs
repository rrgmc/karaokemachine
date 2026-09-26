//! `km_song::text::fold` and the search index must fold identically.
//!
//! The catalog stores a `sort_key` folded in Rust and indexes the same text in an FTS5 table
//! tokenized by `unicode61 remove_diacritics 2`. If those two ever disagree, a song found by the
//! search box silently fails to match the same song filtered in memory, or files under a letter
//! nobody would look under — a fault that reads like bad data rather than like a bug.
//!
//! Neither claim is safe as a comment. This walks every Latin character the corpus can produce and
//! holds the two to each other.

use km_song::text::fold;
use rusqlite::Connection;

/// Latin-1 Supplement, Latin Extended-A and Latin Extended-B.
///
/// Everything windows-1250, ISO-8859-2, ISO-8859-4, ISO-8859-13 and windows-1254 can decode to
/// lives in this range — those five being what the corpus scan actually found beyond CP1252.
const RANGE: std::ops::RangeInclusive<u32> = 0x00C0..=0x024F;

/// What the search index makes of each piece of text, one row per piece.
fn tokenize_each(texts: &[String]) -> Vec<Option<String>> {
    let db = Connection::open_in_memory().expect("open");
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(x, tokenize='unicode61 remove_diacritics 2');
         CREATE VIRTUAL TABLE v USING fts5vocab(t, 'instance');",
    )
    .expect("schema");
    for (index, text) in texts.iter().enumerate() {
        db.execute(
            "INSERT INTO t(rowid, x) VALUES (?1, ?2)",
            rusqlite::params![index as i64, text],
        )
        .expect("insert");
    }

    let mut out = vec![None; texts.len()];
    let mut stmt = db.prepare("SELECT doc, term FROM v").expect("prepare");
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query");
    for row in rows {
        let (doc, term) = row.expect("row");
        out[doc as usize] = Some(term);
    }
    out
}

#[test]
fn fold_and_the_search_index_agree() {
    let chars: Vec<char> = RANGE.filter_map(char::from_u32).collect();
    let disagreements = disagreements(chars.iter().map(char::to_string).collect());
    assert!(
        disagreements.is_empty(),
        "{} character(s) sort under one letter and search under another:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The same range decomposed, as a file name written on macOS spells it.
///
/// `A` and U+0301 is one letter to the search index, which strips the combining accent. `fold` has
/// to compose the pair first, or it reads the accent as punctuation and splits the word in two.
#[test]
fn fold_and_the_search_index_agree_on_decomposed_text() {
    use unicode_normalization::UnicodeNormalization;
    let decomposed: Vec<String> = RANGE
        .filter_map(char::from_u32)
        .map(|ch| ch.to_string().nfd().collect::<String>())
        .filter(|text| text.chars().count() > 1)
        .collect();
    assert!(
        !decomposed.is_empty(),
        "the range holds letters that decompose"
    );
    let disagreements = disagreements(decomposed);

    assert!(
        disagreements.is_empty(),
        "{} decomposed letter(s) sort under one letter and search under another:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// Every piece of text on which `fold` and the search index give different answers.
fn disagreements(texts: Vec<String>) -> Vec<String> {
    let tokens = tokenize_each(&texts);
    let mut out = Vec::new();
    for (text, token) in texts.iter().zip(tokens) {
        // A character the tokenizer treats as a separator produces no token; `fold` drops it to the
        // empty string for the same reason, since it is not alphanumeric.
        let indexed = token.unwrap_or_default();
        let sorted = fold(text);
        if sorted != indexed {
            let points: Vec<String> = text
                .chars()
                .map(|ch| format!("U+{:04X}", ch as u32))
                .collect();
            out.push(format!(
                "{} {text}: fold gave {sorted:?}, the index gave {indexed:?}",
                points.join(" ")
            ));
        }
    }
    out
}

/// The letters `remove_diacritics` deliberately leaves alone, and so does `fold`.
///
/// A stroke, a ligature and a dotless `i` are letters rather than decorated ones. Listing them is
/// what stops somebody "finishing" the table above by adding `ł => l`, which would look like a
/// tidy-up and would silently split Polish titles between the browse strip and the search box.
#[test]
fn a_stroke_is_not_an_accent() {
    for ch in ['ł', 'ø', 'æ', 'đ', 'ß', 'ı', 'ħ', 'ŋ', 'œ', 'ŧ'] {
        assert_eq!(
            fold(&ch.to_string()),
            ch.to_string(),
            "{ch} must survive folding, because the search index keeps it"
        );
    }
}

/// The reason any of this exists, in the language the corpus is actually in.
#[test]
fn the_accents_the_corpus_is_full_of_still_fold() {
    assert_eq!(fold("Coração"), fold("CORACAO"));
    assert_eq!(fold("Águas de Março"), "aguas de marco");
    // Czech, Polish and Hungarian: 1,968 files in the corpus scan, every one of which sorts past Z
    // unfolded.
    assert_eq!(fold("Příliš žluťoučký kůň"), "prilis zlutoucky kun");
    assert_eq!(fold("Gdańsk"), "gdansk");
    assert_eq!(fold("Tükörfúrógép"), "tukorfurogep");
    // Turkish, whose dotted capital lower-cases to two characters.
    assert_eq!(fold("İstanbul"), "istanbul");
}

/// A catalog folded by an older spelling of `fold` refolds itself, and tells the mirrors.
///
/// Not the backfill in `catalog.rs`, which fires when the sort columns are *absent*. Here they are
/// present and filled, and what changed is the table inside [`fold`]: `ř` and `ě` used to survive it
/// and now become `r` and `e`, so a Czech title that sorted past `Z` sorts under `P`. A machine that
/// did not notice would keep the old order until every package happened to be reinstalled.
#[test]
fn a_catalog_folded_by_an_older_table_refolds_itself() {
    let dir = std::env::temp_dir().join("km-catalog-tests-refold");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("library.sqlite");

    {
        let conn = Connection::open(&path).expect("create catalog");
        conn.execute_batch(include_str!("../src/schema.sql"))
            .expect("schema");
        // Sort keys exactly as the Latin-1-only table would have written them: the accents it knew
        // are folded, and the Czech ones are left alone, which is what put them after `Z`.
        conn.execute_batch(
            r#"INSERT INTO packages (id, name, version, path, song_count, installed_at, bank)
                 VALUES ('old', 'Old', '1.0.0', '/tmp/old.kmpkg', 3, '2026-01-01', 1);
               INSERT INTO songs (number, package_id, title, artist, file, duration_ms,
                                  sort_key, sort_artist)
                 VALUES (1001, 'old', 'Zebra',  'Zeca',  'a.mid', 1000, 'zebra',  'zeca'),
                        (1002, 'old', 'Příliš', 'Bara',  'b.mid', 1000, 'příliš', 'bara'),
                        (1003, 'old', 'Banana', 'Bebel', 'c.mid', 1000, 'banana', 'bebel');
               UPDATE meta SET value = '7' WHERE key = 'catalog_version';
               INSERT INTO meta (key, value) VALUES ('fold_version', '1')
                 ON CONFLICT(key) DO UPDATE SET value = '1';"#,
        )
        .expect("songs folded by the older table");
    }

    let after = {
        let library = km_catalog::Library::open(&path).expect("an older catalog still opens");
        let titles: Vec<String> = library
            .search(&km_catalog::SearchQuery {
                sort: km_catalog::SortOrder::Title,
                ..km_catalog::SearchQuery::default()
            })
            .expect("search")
            .into_iter()
            .map(|song| song.title)
            .collect();
        assert_eq!(
            titles,
            ["Banana", "Příliš", "Zebra"],
            "refolded, so `Příliš` files under P rather than after Z"
        );
        let version = library.catalog_version().expect("version");
        assert!(
            version > 7,
            "the mirrors have to be told the alphabet moved; got {version}"
        );
        version
    };

    let library = km_catalog::Library::open(&path).expect("reopen");
    assert_eq!(
        library.catalog_version().expect("version"),
        after,
        "the refold happens once, not at every open"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
