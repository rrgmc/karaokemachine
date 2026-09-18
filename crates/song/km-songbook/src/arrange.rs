//! Grouping rows into sections and putting them in order.
//!
//! This is here, and not in either of the two adapters that feed it, because *the order a book is
//! in* is one decision and there are two callers — the machine's catalog and a stack of `.kmpkg`
//! files. Two copies of it would drift, and the drift would be invisible: both books would look
//! right, and only somebody holding them side by side would notice they disagreed.
//!
//! # What is deliberately not here
//!
//! The **fold**. Sorting `Ângela` next to `Angela` rather than after `Zebra` needs accent folding,
//! and the fold it must use is `km_song::text::fold` — the same one `km-catalog`'s FTS5
//! `unicode61 remove_diacritics 2` implies, so that the book's alphabet and the search box's are
//! the same alphabet. That function lives in a crate which drags `midly`, `encoding_rs` and
//! `chardetng`, and this crate draws rectangles.
//!
//! So the caller folds and this module sorts: an [`Entry`] arrives carrying its own [`SortKey`].
//! Folding is a fact about a catalog; ordering is a fact about a book.

use crate::{BookSection, BookSong};

/// What to sort a row by, computed by the caller.
///
/// `artist` and `title` are expected to be **already folded** — lower case, accents removed. The
/// type does not enforce that, because it cannot; see the module header for why it does not try.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey {
    /// `true` for a song with no artist, so those sort *after* every named one rather than under a
    /// blank at the top of the section. "No artist" is not somebody to look up.
    pub artist_missing: bool,
    /// The folded artist name.
    pub artist: String,
    /// The folded title, which decides the order within one artist.
    pub title: String,
}

/// A row on its way into a book: what to print, where it belongs, and what to sort it by.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The row itself.
    pub song: BookSong,
    /// The heading this row belongs under, already named for a reader — `Portuguese`, not `pt`.
    pub section: String,
    /// The folded form of [`Self::section`], which is what the sections are ordered by.
    ///
    /// **A separate field rather than folding [`Self::section`] here**, for the reason the module
    /// header gives about [`SortKey`]: the fold has to be `km_song::text::fold`, so that the book's
    /// alphabet is the search box's alphabet, and this crate cannot reach it.
    ///
    /// It earns its place the moment a heading stops being an English language name. `Português`
    /// and `Índico` are headings a reader expects between `Polonês` and `Italiano`, and a byte
    /// compare files both after `Z` — which is exactly the fault `One alphabet, everywhere` exists
    /// to prevent, arriving in the one place nothing was folding.
    pub section_sort: String,
    /// How to place it among its neighbors.
    pub sort: SortKey,
}

/// Groups entries into sections, orders each section's rows, and orders the sections themselves.
///
/// Sections come out **alphabetically by [`Entry::section_sort`]**, with one exception: the section
/// named `last` is pinned to the end however its name sorts. That is where the songs nobody
/// classified go, and a book wants them after the languages rather than filed under whatever letter
/// the phrase begins with.
///
/// `last` is matched against the printed heading rather than the folded one, because it is the same
/// string the caller put in [`Entry::section`] — one value, compared with itself.
///
/// Alphabetical rather than by size, which is where `km_catalog::Library::languages` puts the
/// largest first. That is right for a picker — the one language nearly everything is in belongs at
/// the top of a list somebody is choosing from — and wrong for a book, which is a thing somebody
/// flips through looking for a divider.
#[must_use]
pub fn arrange(mut entries: Vec<Entry>, last: &str) -> Vec<BookSection> {
    // One sort of everything rather than a sort per section: the section name leads the key, so a
    // single pass leaves the rows of each section contiguous and already in order.
    entries.sort_by(|a, b| {
        let a_last = a.section == last;
        let b_last = b.section == last;
        a_last
            .cmp(&b_last)
            .then_with(|| a.section_sort.cmp(&b.section_sort))
            .then_with(|| a.sort.cmp(&b.sort))
    });

    let mut sections: Vec<BookSection> = Vec::new();
    for entry in entries {
        match sections.last_mut() {
            Some(section) if section.heading == entry.section => section.songs.push(entry.song),
            _ => sections.push(BookSection {
                heading: entry.section,
                songs: vec![entry.song],
            }),
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use km_songcode::SongCode;

    use super::*;

    fn entry(section: &str, artist: Option<&str>, title: &str, number: u32) -> Entry {
        Entry {
            song: BookSong {
                artist: artist.map(str::to_owned),
                number: SongCode::new(number),
                title: title.to_owned(),
                first_line: None,
            },
            section: section.to_owned(),
            section_sort: section.to_lowercase(),
            sort: SortKey {
                artist_missing: artist.is_none(),
                // The tests fold by hand; the real callers use `km_song::text::fold`.
                artist: artist.unwrap_or_default().to_lowercase(),
                title: title.to_lowercase(),
            },
        }
    }

    fn artists(section: &BookSection) -> Vec<String> {
        section
            .songs
            .iter()
            .map(|song| song.artist.clone().unwrap_or_else(|| "—".to_owned()))
            .collect()
    }

    #[test]
    fn rows_are_grouped_under_their_own_heading() {
        let sections = arrange(
            vec![
                entry("Portuguese", Some("Legiao Urbana"), "Tempo Perdido", 1),
                entry("English", Some("Madonna"), "Like a Prayer", 2),
                entry("Portuguese", Some("Cazuza"), "Exagerado", 3),
            ],
            "No language recorded",
        );
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "English");
        assert_eq!(sections[1].heading, "Portuguese");
        assert_eq!(sections[1].songs.len(), 2);
    }

    /// The whole point of the caller doing the folding: an accented name has to file under its
    /// letter, not after `Z` where a byte comparison would put it.
    #[test]
    fn a_folded_key_puts_an_accented_name_where_a_reader_looks_for_it() {
        let mut first = entry("Portuguese", Some("Ângela Ro Ro"), "Amor Meu", 1);
        first.sort.artist = "angela ro ro".to_owned();
        let sections = arrange(
            vec![
                entry("Portuguese", Some("Zeca"), "Coisinha", 2),
                first,
                entry("Portuguese", Some("Bruno"), "Desce", 3),
            ],
            "x",
        );
        assert_eq!(artists(&sections[0]), ["Ângela Ro Ro", "Bruno", "Zeca"]);
    }

    /// The same point one level up, and the reason [`Entry::section_sort`] exists at all: a heading
    /// is a word in the reader's language too, so the moment it stops being an English language name
    /// a byte comparison files every accented one after `Z`.
    #[test]
    fn a_folded_key_puts_an_accented_heading_where_a_reader_looks_for_it() {
        let mut accented = entry("Índico", Some("Ravi"), "Raga", 1);
        accented.section_sort = "indico".to_owned();
        let sections = arrange(
            vec![
                entry("Polonês", Some("Anna"), "Piosenka", 2),
                accented,
                entry("Alemão", Some("Nena"), "Luftballons", 3),
            ],
            "x",
        );
        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(headings, ["Alemão", "Índico", "Polonês"]);
    }

    #[test]
    fn one_artists_songs_are_ordered_by_title() {
        let sections = arrange(
            vec![
                entry("English", Some("Queen"), "Somebody to Love", 2),
                entry("English", Some("Queen"), "Bohemian Rhapsody", 1),
            ],
            "x",
        );
        let titles: Vec<&str> = sections[0].songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Bohemian Rhapsody", "Somebody to Love"]);
    }

    /// "No artist" is not somebody to look up, so those rows go after the named ones rather than
    /// heading the section under a blank.
    #[test]
    fn songs_with_no_artist_come_last_within_their_section() {
        let sections = arrange(
            vec![
                entry("English", None, "Aaa Unknown", 1),
                entry("English", Some("Zeca"), "Zzz Known", 2),
            ],
            "x",
        );
        assert_eq!(artists(&sections[0]), ["Zeca", "—"]);
    }

    /// However its name sorts. `No language recorded` begins with `N` and would otherwise land in
    /// the middle of the book.
    #[test]
    fn the_unclassified_section_is_pinned_last() {
        let sections = arrange(
            vec![
                entry("Portuguese", Some("Cazuza"), "Exagerado", 1),
                entry("No language recorded", Some("Anon"), "Mystery", 2),
                entry("English", Some("Madonna"), "Frozen", 3),
            ],
            "No language recorded",
        );
        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(headings, ["English", "Portuguese", "No language recorded"]);
    }

    #[test]
    fn no_entries_is_no_sections() {
        assert!(arrange(Vec::new(), "x").is_empty());
    }
}
