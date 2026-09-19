//! Turning a machine's catalog into a song book.
//!
//! [`km_songbook`] draws the book and decides what order it is in; this decides *which* songs go in it,
//! what the sections are called, and what each row sorts by. Two things are needed for that which
//! `km-songbook` deliberately cannot see: [`km_kmpkg::Language`], to say `Portuguese` where the
//! catalog says `pt`, and [`km_song::text::fold`], so that `Ângela` files under `A` rather than
//! after `Z`.
//!
//! **That fold is not an arbitrary choice.** It is the same one `km-catalog`'s FTS5 index implies
//! through `unicode61 remove_diacritics 2`, so the book's alphabet and the search box's are the same
//! alphabet — a singer who finds a song by typing `coracao` finds it in the book under `C` too.
//!
//! This lives in `km-api` rather than in `km-app` because both callers can see it from here: the
//! HTTP handler, which has a [`Catalog`](crate::machine::Catalog) and not a `Library`, and the
//! machine's `--song-book`
//! flag, which has a `Library` and no HTTP. [`collect`] is shaped as a closure for exactly that
//! reason — one paging loop, two callers, no second place for the cursor arithmetic to be wrong.

use std::sync::OnceLock;

use km_catalog::CatalogSong;
use km_kmpkg::Language;
use km_locale::{Catalog as Messages, Locale};
use km_songbook::{BookSong, BookStyle, Entry, SortKey};
use km_songcode::SongCode;

/// The book's own words, one catalog per locale.
///
/// `include_str!` rather than a file beside the binary, per `Bundling assets`: this book is printed
/// by an appliance under a television with no filesystem anybody browses, and a missing catalog
/// would be a book of `⟦book-title⟧`.
const CATALOGS: &[(Locale, &str)] = &[
    (Locale::English, include_str!("../i18n/en.ftl")),
    (
        Locale::BrazilianPortuguese,
        include_str!("../i18n/pt-BR.ftl"),
    ),
];

/// The messages for one locale, parsed once.
///
/// A `OnceLock` per process rather than a parse per book: a catalog is immutable and a book is not
/// the only thing that will want one.
#[must_use]
pub fn messages(locale: Locale) -> &'static Messages {
    static PARSED: OnceLock<Vec<(Locale, Messages)>> = OnceLock::new();
    let parsed = PARSED.get_or_init(|| {
        CATALOGS
            .iter()
            .map(|(locale, source)| {
                let catalog = Messages::new(*locale, source)
                    // Compiled in, so this is a build fault rather than anything a caller can cause
                    // — and `every_catalog_parses` is what turns it into one before a release.
                    .unwrap_or_else(|errors| {
                        panic!("{locale} book catalog: {}", errors.join("; "))
                    });
                (*locale, catalog)
            })
            .collect()
    });
    parsed
        .iter()
        .find(|(candidate, _)| *candidate == locale)
        .map(|(_, catalog)| catalog)
        .expect("every locale has a book catalog")
}

/// The heading songs with no language recorded appear under.
///
/// **Not the same thing as `und`.** The ISO table has `und` — "undetermined" — and a package built
/// with `km-pack build --default-language und` says so on purpose: somebody looked and could not
/// tell. This heading is for a song whose language nobody ever wrote down at all. Two different
/// facts, and a book that merged them would be answering a question nobody asked.
#[must_use]
pub fn unclassified(locale: Locale) -> String {
    messages(locale).msg("book-unclassified").into_owned()
}

/// Everything `GET /songs/book.pdf` takes: which songs, and what the book calls itself.
///
/// **A separate type from [`BookFilter`], deliberately.** That one is a filter, and every field on
/// it narrows — `admits` and `is_everything` both say so, and `book_filename` reads it to name the
/// download. A name narrows nothing; putting it there would make two honest methods start lying.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BookQuery {
    /// Only this language code. An unrecognized one narrows to nothing rather than failing.
    pub language: Option<String>,
    /// Only this package id.
    pub package: Option<String>,
    /// Only songs carrying any one of these tags — `?tags=rock,brasil`, the one wire spelling.
    ///
    /// **A filter, never a heading.** The book is sectioned by language, and it stays that way: a
    /// language is a closed table with an English name per row, and an open vocabulary has neither
    /// an order nor a name to head a section with. A song carrying three tags would also have to
    /// appear in three sections or arbitrarily in one.
    pub tags: Option<String>,
    /// What the top left of every page says instead of `KaraokeMachine`.
    pub name: Option<String>,
    /// What language the book's own words are in. `None` means the machine's own.
    ///
    /// **`locale`, not `language`** — and the two travel together on this one route, which is the
    /// clearest place in the product to see why they had to be different words. `?language=pt`
    /// narrows to songs sung in Portuguese; `?locale=pt-BR` says the column headings read
    /// `ARTISTA`. Either without the other is a reasonable thing to ask for.
    pub locale: Option<String>,
}

impl BookQuery {
    /// The songs half of it.
    #[must_use]
    pub fn filter(&self) -> BookFilter {
        BookFilter {
            language: self.language.clone(),
            package: self.package.clone(),
            tags: self
                .tags
                .as_deref()
                .map(km_kmpkg::tag::parse_list)
                .unwrap_or_default()
                .into_iter()
                .map(km_kmpkg::Tag::into_string)
                .collect(),
        }
    }

    /// Which locale the book should be written in, given what the machine speaks.
    ///
    /// An unrecognized tag falls back to the machine's own rather than failing, the same judgement
    /// `?language=` makes about a code nobody has: a book in the wrong language is still a book,
    /// and a 400 in the middle of printing one is not an improvement.
    #[must_use]
    pub fn locale(&self, machine: Locale) -> Locale {
        self.locale
            .as_deref()
            .and_then(Locale::best_match)
            .unwrap_or(machine)
    }

    /// A short, safe stand-in for the name, for an `ETag`.
    ///
    /// **The name is not interpolated into the header**, and this is the reason the function
    /// exists. Every other parameter that reaches that `ETag` is a closed set — a language is only
    /// named when the compiled-in ISO table knows it — but a name is free text out of a query
    /// string, and a `"` in one would close the entity tag early. The body does vary with it, so
    /// leaving it out is not an option either: that is the cache bug the header's own comment was
    /// written to prevent.
    ///
    /// FNV-1a, which is five lines and no dependency. A hash collision here would serve one name's
    /// book to a request for another's, out of a cache, at the same catalog version — the cost of
    /// a 64-bit collision against a handful of names somebody types is not worth a crate.
    #[must_use]
    pub fn name_tag(&self) -> String {
        let Some(name) = &self.name else {
            return "default".to_owned();
        };
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

/// Which songs go in the book.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BookFilter {
    /// Only this language code. An unrecognized one narrows to nothing rather than failing.
    pub language: Option<String>,
    /// Only this package id.
    pub package: Option<String>,
    /// Only songs carrying **any** one of these, already folded to slugs.
    pub tags: Vec<String>,
}

impl BookFilter {
    /// Whether a song belongs in the book.
    fn admits(&self, song: &CatalogSong) -> bool {
        if let Some(language) = &self.language {
            // Compared case-insensitively, because the column is free text written by packaging and
            // the corpus holds `pt`, `PT` and worse.
            if !song
                .language
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(language))
            {
                return false;
            }
        }
        if let Some(package) = &self.package
            && song.package_id != *package
        {
            return false;
        }
        // Any one, not every: the same OR the catalog's own filter makes, so a book printed from
        // `?tags=rock,brasil` holds exactly the songs a remote showing that filter would list.
        // Compared exactly, because both sides are slugs `Tag::parse` produced.
        //
        // **The emptiness is checked first**, because no tags at all is no tag filter, where an
        // `any` over nothing is nothing.
        if !self.tags.is_empty()
            && !self
                .tags
                .iter()
                .any(|wanted| song.tags.iter().any(|held| held == wanted))
        {
            return false;
        }
        true
    }

    /// Whether this filter narrows anything at all.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.language.is_none() && self.package.is_none() && self.tags.is_empty()
    }
}

/// How many rows to ask for at a time when walking the whole catalog.
///
/// Twelve seeks for a twelve-thousand-song machine. Larger pages would not help — each one is an
/// index seek followed by a sequential read — and smaller ones would multiply the round trips.
const PAGE: usize = 1_000;

/// Walks a whole catalog by following its keyset cursor to exhaustion.
///
/// Takes a closure rather than a `Library` or a [`km_api_catalog`](crate::machine::Catalog)
/// because the two callers have one each and neither can be given the other's. The cursor arithmetic
/// is the part worth having in one place: it is `(prefix, number)` row-value paging against a unique
/// index, and getting it wrong silently drops or repeats a page.
///
/// A book holds the whole catalog in memory — about 200 bytes a row, so a few megabytes at twelve
/// thousand songs. That is inherent to a book rather than a shortcoming of this function. If a
/// machine ever meets a six-figure catalog the answer is a `Library::book_rows` selecting the five
/// columns a book uses instead of all fifteen; it is deliberately not built, because it would also
/// mean a [`Catalog`](crate::machine::Catalog) method and a test double for it.
pub fn collect<E>(
    mut page: impl FnMut(Option<SongCode>, usize) -> Result<Vec<CatalogSong>, E>,
) -> Result<Vec<CatalogSong>, E> {
    let mut rows: Vec<CatalogSong> = Vec::new();
    let mut cursor = None;
    loop {
        let batch = page(cursor, PAGE)?;
        let short = batch.len() < PAGE;
        // Taken before `batch` is moved, and from the last row rather than counted, because the
        // cursor is a code and not an offset.
        cursor = batch.last().map(|song| song.number);
        rows.extend(batch);
        if short {
            break;
        }
    }
    Ok(rows)
}

/// Turns catalog rows into book entries: filtered, named and keyed for sorting.
///
/// Sorting and grouping themselves happen in [`arrange`](km_songbook::arrange()), which both this adapter and
/// `km-pack`'s hand their entries to, so the two books cannot end up in different orders.
#[must_use]
pub fn entries(songs: Vec<CatalogSong>, filter: &BookFilter, locale: Locale) -> Vec<Entry> {
    songs
        .into_iter()
        .filter(|song| filter.admits(song))
        .map(|song| {
            let artist = song.artist.clone();
            let section = section_of(song.language.as_deref(), locale);
            Entry {
                sort: SortKey {
                    artist_missing: artist.is_none(),
                    artist: km_song::text::fold(artist.as_deref().unwrap_or_default()),
                    title: km_song::text::fold(&song.title),
                },
                section_sort: km_song::text::fold(&section),
                section,
                song: BookSong {
                    artist,
                    number: song.number,
                    title: song.title,
                    // Empty for a video song and an MP3+G song — their words are pixels — and for
                    // any song whose package was built before previews existed. The cell is blank
                    // rather than the row being wrong.
                    first_line: song.lyric_preview.into_iter().next(),
                },
            }
        })
        .collect()
}

/// What section a language code puts a song in.
///
/// **The raw code is what identifies a section, and the name is only what is printed.** A code the
/// ISO table does not know prints as itself rather than being swept in with the unclassified: the
/// `language` column is free text written by packaging, and this corpus has carried `ENGL`, `PORT`
/// and `ITALIANO`. Merging those into "no language recorded" would throw away the one thing they do
/// say, and merging them into each other would be worse.
/// **A translated name is offered and not required.** The catalog carries the handful of languages
/// a real machine actually has packages in; anything else falls back to the ISO table's English
/// name, which is the same answer a code the table does not know already gets — shown as itself
/// rather than dropped. See the `language-` note in `i18n/en.ftl`.
fn section_of(language: Option<&str>, locale: Locale) -> String {
    match language.map(str::trim).filter(|code| !code.is_empty()) {
        None => unclassified(locale),
        Some(code) => Language::parse(code).map_or_else(
            || code.to_owned(),
            |known| named_language(known, locale).into_owned(),
        ),
    }
}

/// A language's name in the reader's language, or its English one.
///
/// `pub(crate)` for the download filename, which names the language the same way the section
/// divider inside the book does — one function, so a book headed `Português` cannot arrive in a file
/// called `Portuguese`.
pub(crate) fn named_language(language: Language, locale: Locale) -> std::borrow::Cow<'static, str> {
    let key = format!("language-{}", language.code());
    let catalog = messages(locale);
    if catalog.keys().contains(&key) {
        return std::borrow::Cow::Owned(catalog.msg(&key).into_owned());
    }
    std::borrow::Cow::Borrowed(language.name())
}

/// The words on every page: the title, the note under it, and the four column headings.
///
/// In the reader's locale, which is the whole of `BookStyle` — `km-songbook` holds no words of its
/// own and never did, so the book needed no translation layer, only a caller that fills the struct
/// from a catalog instead of from `Default`.
///
/// The subtitle says what the book is of, so a printed copy found on a table a year later can still
/// be placed. It is built by concatenating three finished clauses rather than by one message with
/// three optional arguments: two of the three are usually absent, and a Fluent selector per clause
/// would put the sentence's shape in the catalog while leaving the ` · ` between them in Rust.
#[must_use]
pub fn style(
    rows: usize,
    version: u64,
    filter: &BookFilter,
    name: Option<&str>,
    locale: Locale,
) -> BookStyle {
    let messages = messages(locale);
    let mut subtitle = messages
        .msg_with("book-song-count", &[("count", (rows as i64).into())])
        .into_owned();
    // **`?language=` is deliberately not a clause here.** Every page's masthead now names the
    // section it is in, so a one-language book says its language in the top right of all two
    // hundred pages; repeating it in the small print under the title would be the same fact twice
    // on one line. What stays are the two the pages cannot say for themselves — how many songs, and
    // which catalog version.
    if let Some(package) = &filter.package {
        let clause = messages.msg_with("book-of-package", &[("package", package.as_str().into())]);
        subtitle.push_str(&format!(" · {clause}"));
    }
    // The catalog version is what changes when a package is installed or removed, so it is the
    // one number that tells two printed copies apart.
    let version = messages.msg_with(
        "book-catalog-version",
        &[("version", (version as i64).into())],
    );
    subtitle.push_str(&format!(" · {version}"));

    BookStyle {
        name: name.map_or_else(|| messages.msg("book-name").into_owned(), str::to_owned),
        // ^ `name` is what a caller asked for by hand. Where nobody did, `book_name_for` is what
        //   turns a machine's own name into one; see its own note for why that is not done here.
        title: messages.msg("book-title").into_owned(),
        subtitle: Some(subtitle),
        column_headings: [
            messages.msg("column-artist").into_owned(),
            messages.msg("column-code").into_owned(),
            messages.msg("column-title").into_owned(),
            messages.msg("column-first-line").into_owned(),
        ],
        empty_message: if filter.is_everything() {
            messages.msg("book-empty-catalog").into_owned()
        } else {
            messages.msg("book-empty-filter").into_owned()
        },
    }
}

/// What this product is called, wherever a book has to name it.
///
/// **Not the machine's name and never read from settings**: it is the fixed half of both the
/// masthead and the download filename, and the thing [`book_name_for`] compares against so a machine
/// still called `KaraokeMachine` does not get it twice.
pub(crate) const PRODUCT: &str = "KaraokeMachine";

/// What the top left of every page says for a machine called `machine`.
///
/// **`KaraokeMachine - Living Room`**, and `None` for a machine that has not been named — where
/// `None` means the plain product name, which is what the book said before there was a machine name
/// in it at all.
///
/// **A composition rather than a replacement.** The masthead answers *whose book is this*, and a
/// house with a machine in two rooms is the case the field exists for; a book that said only
/// `Living Room` would have dropped the half that says what kind of thing it is a list for.
///
/// **It is not done inside [`style`]**, and that is the seam that matters: `style` fills a struct
/// from what a caller asked for, and `?name=` is somebody saying *call it this*. A machine name is a
/// default for that parameter, so it is applied where the parameter is read — which is also what
/// makes the `ETag` follow, since the composed string is what reaches
/// [`BookQuery::name_tag`] and a rename therefore changes the validator.
///
/// The default name is compared case-sensitively and after trimming: a machine still called
/// `KaraokeMachine` gets `KaraokeMachine`, not `KaraokeMachine - KaraokeMachine`.
#[must_use]
pub fn book_name_for(machine: &str) -> Option<String> {
    let machine = machine.trim();
    if machine.is_empty() || machine == PRODUCT {
        return None;
    }
    Some(format!("{PRODUCT} - {machine}"))
}

/// The whole job: rows in, PDF out.
#[must_use]
pub fn render(
    songs: Vec<CatalogSong>,
    filter: &BookFilter,
    version: u64,
    name: Option<&str>,
    locale: Locale,
) -> km_songbook::Book {
    let entries = entries(songs, filter, locale);
    let style = style(entries.len(), version, filter, name, locale);
    km_songbook::build(entries, &unclassified(locale), style)
}

#[cfg(test)]
mod tests {
    use km_catalog::SongKind;

    use super::*;

    /// The locale most of these tests are about, since what they assert is the arrangement rather
    /// than the words. The ones about the words name their locale in the test's own name.
    const EN: Locale = Locale::English;
    const PT: Locale = Locale::BrazilianPortuguese;

    fn song(number: u32, artist: Option<&str>, title: &str, language: Option<&str>) -> CatalogSong {
        CatalogSong {
            number: SongCode::new(number),
            package_id: "pkg".to_owned(),
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            language: language.map(str::to_owned),
            kind: SongKind::Midi,
            file: "songs/1.kar".to_owned(),
            duration_ms: 1000,
            lyric_encoding: None,
            default_transpose: 0,
            lyrics_hidden: false,
            fixes: Vec::new(),
            melody_channel: None,
            suitability: None,
            content_hash: None,
            lyric_preview: Vec::new(),
            tags: Vec::new(),
            // MIDI, so nothing measured it -- a MIDI song is the reference. See `CatalogSong`.
            loudness_lufs: None,
        }
    }

    fn headings(sections: &[km_songbook::BookSection]) -> Vec<&str> {
        sections.iter().map(|s| s.heading.as_str()).collect()
    }

    fn arranged(songs: Vec<CatalogSong>) -> Vec<km_songbook::BookSection> {
        km_songbook::arrange(
            entries(songs, &BookFilter::default(), EN),
            &unclassified(EN),
        )
    }

    #[test]
    fn a_language_code_is_printed_as_the_name_a_reader_knows() {
        let sections = arranged(vec![
            song(1, Some("Cazuza"), "Exagerado", Some("pt")),
            song(2, Some("Madonna"), "Frozen", Some("en")),
        ]);
        assert_eq!(headings(&sections), ["English", "Portuguese"]);
    }

    /// The `language` column is free text written by packaging, and the corpus has held these. A
    /// code the table does not know must print as itself: it says *something*, and burying it with
    /// the songs nobody classified would throw that away.
    #[test]
    fn a_code_the_iso_table_does_not_know_prints_as_itself() {
        let sections = arranged(vec![
            song(1, Some("A"), "One", Some("ENGL")),
            song(2, Some("B"), "Two", Some("PORT")),
            song(3, Some("C"), "Three", None),
        ]);
        assert_eq!(
            headings(&sections),
            ["ENGL", "PORT", unclassified(EN).as_str()]
        );
    }

    /// `und` is a package saying "somebody looked and could not tell". That is a different fact from
    /// nobody ever having said, and it gets a different heading.
    #[test]
    fn undetermined_is_not_the_same_as_unrecorded() {
        let sections = arranged(vec![
            song(1, Some("A"), "One", Some("und")),
            song(2, Some("B"), "Two", None),
        ]);
        assert_eq!(
            headings(&sections),
            ["Undetermined", unclassified(EN).as_str()]
        );
    }

    /// The fold is `km_song::text::fold`, which is what `km-catalog`'s FTS5 index implies — so the
    /// book's alphabet is the search box's alphabet.
    #[test]
    fn an_accented_artist_files_under_its_letter() {
        let sections = arranged(vec![
            song(1, Some("Zeca Pagodinho"), "Deixa a Vida", Some("pt")),
            song(2, Some("Ângela Ro Ro"), "Amor Meu", Some("pt")),
            song(3, Some("Bruno"), "Desce", Some("pt")),
        ]);
        let artists: Vec<&str> = sections[0]
            .songs
            .iter()
            .map(|s| s.artist.as_deref().unwrap_or_default())
            .collect();
        assert_eq!(artists, ["Ângela Ro Ro", "Bruno", "Zeca Pagodinho"]);
    }

    #[test]
    fn the_language_filter_is_forgiving_about_case() {
        let songs = vec![
            song(1, Some("A"), "One", Some("PT")),
            song(2, Some("B"), "Two", Some("en")),
        ];
        let filter = BookFilter {
            language: Some("pt".to_owned()),
            package: None,
            tags: Vec::new(),
        };
        assert_eq!(entries(songs, &filter, EN).len(), 1);
    }

    #[test]
    fn the_package_filter_takes_one_packages_songs() {
        let mut other = song(2, Some("B"), "Two", Some("en"));
        other.package_id = "second".to_owned();
        let songs = vec![song(1, Some("A"), "One", Some("en")), other];
        let filter = BookFilter {
            language: None,
            package: Some("second".to_owned()),
            tags: Vec::new(),
        };
        let entries = entries(songs, &filter, EN);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].song.title, "Two");
    }

    /// The book takes the songs carrying any of the tags, and no tags is no tag filter.
    ///
    /// The second half is the one worth a test: `any` over an empty list is false, so a book asked
    /// for with no tags at all would print nothing.
    #[test]
    fn the_tag_filter_takes_the_songs_carrying_any_tag() {
        let tagged = |number: u32, title: &str, tags: &[&str]| {
            let mut song = song(number, Some("A"), title, Some("pt"));
            song.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
            song
        };
        let songs = || {
            vec![
                tagged(1, "Both", &["brasil", "rock"]),
                tagged(2, "Rock only", &["rock"]),
                tagged(3, "Brasil only", &["brasil"]),
                tagged(4, "Neither", &[]),
            ]
        };
        let titles = |tags: &[&str]| {
            let filter = BookFilter {
                language: None,
                package: None,
                tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            };
            let mut found: Vec<String> = entries(songs(), &filter, EN)
                .into_iter()
                .map(|entry| entry.song.title)
                .collect();
            found.sort();
            found
        };

        assert_eq!(titles(&["rock"]), ["Both", "Rock only"]);
        assert_eq!(
            titles(&["rock", "brasil"]),
            ["Both", "Brasil only", "Rock only"]
        );
        assert_eq!(titles(&[]), ["Both", "Brasil only", "Neither", "Rock only"]);
    }

    #[test]
    fn the_first_line_of_the_words_is_the_one_that_is_printed() {
        let mut with_words = song(1, Some("A"), "One", Some("en"));
        with_words.lyric_preview = vec!["first line".to_owned(), "second line".to_owned()];
        let entries = entries(vec![with_words], &BookFilter::default(), EN);
        assert_eq!(entries[0].song.first_line.as_deref(), Some("first line"));
    }

    /// The subtitle says what the pages cannot say for themselves, and no more than that.
    ///
    /// **The language is deliberately not in it.** Every page's masthead names the section it is in,
    /// so a book narrowed to one language says so in the top right of all two hundred pages; a
    /// clause under the title would be the same fact twice on one line. The package and the catalog
    /// version have nowhere else to appear and stay.
    #[test]
    fn a_filtered_book_says_what_its_pages_cannot() {
        let style = style(
            3,
            7,
            &BookFilter {
                language: Some("pt".to_owned()),
                package: Some("carnaval".to_owned()),
                tags: Vec::new(),
            },
            None,
            EN,
        );
        let subtitle = style.subtitle.expect("a subtitle");
        assert!(subtitle.contains("3 songs"), "{subtitle}");
        assert!(subtitle.contains("carnaval"), "{subtitle}");
        assert!(subtitle.contains("catalog 7"), "{subtitle}");
        assert!(
            !subtitle.contains("Portuguese"),
            "the pages say the language: {subtitle}"
        );
    }

    /// The masthead is the product's name and the machine's, and neither alone.
    ///
    /// A book saying only `Living Room` would have dropped the half that says what kind of list it
    /// is; a book saying only `KaraokeMachine` is what a house with two machines cannot tell apart.
    #[test]
    fn the_masthead_names_the_machine_when_it_has_one() {
        assert_eq!(
            book_name_for("Living Room").as_deref(),
            Some("KaraokeMachine - Living Room")
        );
        assert_eq!(
            book_name_for("  Living Room  ").as_deref(),
            Some("KaraokeMachine - Living Room")
        );
        // A machine still called what it was shipped as gets the plain product name rather than it
        // twice over.
        assert_eq!(book_name_for("KaraokeMachine"), None);
        assert_eq!(book_name_for(""), None);
        assert_eq!(book_name_for("   "), None);
    }

    /// The composed name reaches the `ETag`, so a rename does not serve the old book from a cache.
    #[test]
    fn renaming_the_machine_changes_the_books_validator() {
        let tag = |name: Option<&str>| {
            BookQuery {
                name: name.map(str::to_owned),
                ..BookQuery::default()
            }
            .name_tag()
        };
        assert_eq!(tag(None), "default");
        assert_ne!(tag(book_name_for("Living Room").as_deref()), tag(None));
        assert_ne!(
            tag(book_name_for("Living Room").as_deref()),
            tag(book_name_for("Kitchen").as_deref())
        );
    }

    #[test]
    fn one_song_is_not_one_songs() {
        let style = style(1, 1, &BookFilter::default(), None, EN);
        assert!(style.subtitle.expect("a subtitle").starts_with("1 song ·"));
    }

    /// The paging loop stops on a short page and never asks twice for the same row.
    #[test]
    fn walking_the_catalog_follows_the_cursor_to_the_end() {
        let all: Vec<CatalogSong> = (1..=2_500)
            .map(|n| song(n, Some("A"), "T", Some("en")))
            .collect();
        let mut calls = 0;
        let rows = collect(|after, limit| {
            calls += 1;
            let start = after.map_or(0, |code| code.number() as usize);
            Ok::<_, ()>(all[start.min(all.len())..(start + limit).min(all.len())].to_vec())
        })
        .expect("no error");
        assert_eq!(rows.len(), 2_500);
        // 1000, 1000, 500 — the third page is short and ends it.
        assert_eq!(calls, 3);
        assert_eq!(rows[0].number, SongCode::new(1));
        assert_eq!(rows[2_499].number, SongCode::new(2_500));
    }

    /// A catalog whose size is an exact multiple of the page needs one more request to learn it
    /// has ended — the case an off-by-one in the loop would get wrong.
    #[test]
    fn a_catalog_that_ends_on_a_page_boundary_still_terminates() {
        let all: Vec<CatalogSong> = (1..=PAGE as u32)
            .map(|n| song(n, Some("A"), "T", Some("en")))
            .collect();
        let mut calls = 0;
        let rows = collect(|after, limit| {
            calls += 1;
            let start = after.map_or(0, |code| code.number() as usize);
            Ok::<_, ()>(all[start.min(all.len())..(start + limit).min(all.len())].to_vec())
        })
        .expect("no error");
        assert_eq!(rows.len(), PAGE);
        assert_eq!(calls, 2);
    }

    #[test]
    fn a_whole_book_renders() {
        let songs = vec![
            song(1, Some("Cazuza"), "Exagerado", Some("pt")),
            song(2, Some("Madonna"), "Frozen", Some("en")),
        ];
        let book = render(songs, &BookFilter::default(), 3, None, EN);
        assert_eq!(book.row_count(), 2);
        assert_eq!(
            book.page_count(),
            2,
            "two languages are two sections and two pages"
        );
        assert!(book.render().starts_with(b"%PDF"));
    }

    /// The query splits cleanly into the songs half and the naming half.
    #[test]
    fn a_name_narrows_nothing() {
        let query = BookQuery {
            language: Some("pt".to_owned()),
            package: None,
            tags: None,
            name: Some("Sala de Estar".to_owned()),
            locale: None,
        };
        assert_eq!(
            query.filter(),
            BookFilter {
                language: Some("pt".to_owned()),
                package: None,
                tags: Vec::new(),
            }
        );
        // Which is the whole reason it is not a field on the filter: `is_everything` would start
        // answering "no" to a book of every song that happens to carry a name.
        let named_only = BookQuery {
            name: Some("Cozinha".to_owned()),
            ..BookQuery::default()
        };
        assert!(named_only.filter().is_everything());
    }

    /// The `ETag`'s stand-in for a name: stable, short, and never the caller's own bytes.
    #[test]
    fn a_name_reaches_the_etag_only_as_a_hash() {
        let named = |name: &str| {
            BookQuery {
                name: Some(name.to_owned()),
                ..BookQuery::default()
            }
            .name_tag()
        };
        assert_eq!(BookQuery::default().name_tag(), "default");
        assert_eq!(named("Cozinha"), named("Cozinha"));
        assert_ne!(named("Cozinha"), named("Sala de Estar"));
        // Nothing a caller types survives into the header, which is what keeps a `"` from closing
        // the entity tag early.
        for hostile in ["\"", "a\"b\r\nX: y", "Sala de Estar"] {
            let tag = named(hostile);
            assert_eq!(tag.len(), 16);
            assert!(tag.chars().all(|c| c.is_ascii_hexdigit()), "{tag}");
        }
    }

    // -- The catalogs --------------------------------------------------------------------------

    #[test]
    fn every_catalog_parses() {
        // `messages` panics on a bad catalog, which is right — it is compiled in, so nobody can
        // cause it at run time. This is what turns that into a build failure instead of a book.
        for locale in Locale::ALL {
            assert!(!messages(*locale).keys().is_empty(), "{locale}");
        }
    }

    #[test]
    fn every_message_is_translated() {
        // The guarantee Fluent cannot give at compile time, bought back one step later. Without it
        // an untranslated key reaches a printed page as `⟦book-title⟧`, which is a book somebody
        // has to throw away.
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let missing = messages(*locale).missing_from(english);
            assert!(
                missing.is_empty(),
                "{locale} has not caught up: {missing:?}"
            );
        }
    }

    #[test]
    fn no_locale_invents_a_message_english_does_not_have() {
        // The same test read backwards. A key only a translation has is one nothing looks up — a
        // leftover from a rename, which would otherwise sit there looking like work.
        //
        // The `language-` family is the one deliberate exception and is *not* exempted here,
        // because it is written in full in every catalog; it is exempt from being *complete*
        // against the ISO table, which is a different claim and is `a_language_without_a_translated
        // _name_falls_back_to_its_english_one`'s business.
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let extra = english.missing_from(messages(*locale));
            assert!(
                extra.is_empty(),
                "{locale} has keys nothing asks for: {extra:?}"
            );
        }
    }

    /// The catalog and the constant spell the product the same way.
    ///
    /// **Two copies exist for a reason and drift for none.** `book-name` is in the catalogs so
    /// `?name=` has one thing to override and the layout has one thing to measure; [`PRODUCT`] is a
    /// constant because the download filename and [`book_name_for`]'s does-this-name-already-say-it
    /// comparison are built where no catalog is in hand. Let them disagree and a machine still
    /// called `KaraokeMachine` gets its name twice across the masthead, which is the exact fault
    /// `book_name_for` exists to prevent — and the file would be named one thing while the page
    /// inside it says another.
    ///
    /// Every locale, because `book-name` is a proper noun that is deliberately not translated: a
    /// translator filling it in is the likeliest way this breaks.
    #[test]
    fn every_catalog_calls_the_product_what_the_constant_does() {
        for locale in Locale::ALL {
            assert_eq!(
                messages(*locale).msg("book-name"),
                PRODUCT,
                "{locale} names the product something the filename will not"
            );
        }
    }

    #[test]
    fn a_catalog_survives_the_books_encoding() {
        // The book embeds no font and speaks cp1252, so a character outside that repertoire is
        // replaced with `?` and counted. A *translation* tripping that counter would be reporting a
        // loss the corpus never had — and would report it only to whoever ran `km-pack book`, since
        // the HTTP route that draws the same book has nowhere to print a count.
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in catalog.keys() {
                let rendered = catalog.msg(key);
                let mut replaced = km_songbook::Replacements::default();
                // The bytes are not the point; what was lost on the way to them is.
                let _ = km_songbook::winansi::encoded(&rendered, &mut replaced);
                assert_eq!(
                    replaced.count, 0,
                    "{locale} `{key}` = {rendered:?} loses characters the book cannot print"
                );
            }
        }
    }

    /// The four column headings fit their columns, in every locale.
    ///
    /// **The other half of the encoding test, and the half that was missing.** That one says a
    /// translation can be *drawn*; this one says it can be drawn *where it goes*. A heading is the
    /// only string in the book whose column is fixed by a layout measured against a printed
    /// reference — a translator sees the English word and not the 36.6 pt it has to live in, and
    /// `CODE` becoming `CÓDIGO` is half as long again. `km-songbook` ellipsizes what does not fit,
    /// so the failure without this is a heading reading `CÓDIG…` on all two hundred pages.
    ///
    /// The margin today is comfortable — the widest is `CÓDIGO` at 28.4 pt of 36.6 — but the
    /// widest *song number* already needs 27.2 pt of that same column, so the two are closer than
    /// they look and a third language is what would find out.
    #[test]
    fn every_translated_heading_fits_its_column() {
        for locale in Locale::ALL {
            let style = style(0, 0, &BookFilter::default(), None, *locale);
            for (column, heading) in style.column_headings.iter().enumerate() {
                let over = km_songbook::layout::heading_overflow(column, heading);
                assert_eq!(
                    over, 0.0,
                    "{locale} heading `{heading}` overruns column {column} by {over} pt"
                );
            }
        }
    }

    #[test]
    fn a_portuguese_book_is_in_portuguese() {
        let style = style(3, 7, &BookFilter::default(), None, PT);
        assert_eq!(style.title, "LISTA DE MÚSICAS");
        assert_eq!(
            style.column_headings,
            ["ARTISTA", "CÓDIGO", "TÍTULO", "INÍCIO DA LETRA"]
        );
        let subtitle = style.subtitle.expect("a subtitle");
        assert!(subtitle.contains("3 músicas"), "{subtitle}");
        assert!(subtitle.contains("catálogo 7"), "{subtitle}");
    }

    #[test]
    fn portuguese_counts_one_song_in_its_own_singular() {
        // The hand-written `if rows == 1 { "" } else { "s" }` this replaced had no way to say this.
        let style = style(1, 1, &BookFilter::default(), None, PT);
        assert!(
            style
                .subtitle
                .expect("a subtitle")
                .starts_with("1 música ·")
        );
    }

    #[test]
    fn a_portuguese_book_names_its_sections_in_portuguese() {
        let sections = km_songbook::arrange(
            entries(
                vec![
                    song(1, Some("Cazuza"), "Exagerado", Some("pt")),
                    song(2, Some("Madonna"), "Frozen", Some("en")),
                    song(3, Some("C"), "Three", None),
                ],
                &BookFilter::default(),
                PT,
            ),
            &unclassified(PT),
        );
        assert_eq!(
            headings(&sections),
            ["Inglês", "Português", "Idioma não informado"]
        );
    }

    #[test]
    fn a_language_without_a_translated_name_falls_back_to_its_english_one() {
        // The `language-` family is deliberately only the handful a real machine has packages in.
        // A code outside it is named in English rather than dropped or shown as a code, which is
        // the answer `section_of` already gives a code the ISO table does not know at all.
        let welsh = Language::parse("cy").expect("cy is in the ISO table");
        assert!(!messages(PT).keys().contains("language-cy"));
        assert_eq!(named_language(welsh, PT), "Welsh");
    }

    #[test]
    fn an_unknown_locale_tag_falls_back_to_what_the_machine_speaks() {
        // A book in the wrong language is still a book; a 400 in the middle of printing one is not
        // an improvement.
        let query = |tag: &str| BookQuery {
            locale: Some(tag.to_owned()),
            ..BookQuery::default()
        };
        assert_eq!(query("pt-BR").locale(EN), PT);
        assert_eq!(query("pt-PT").locale(EN), PT);
        assert_eq!(query("klingon").locale(PT), PT);
        assert_eq!(BookQuery::default().locale(PT), PT);
    }
}
