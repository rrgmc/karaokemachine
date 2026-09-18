//! A song book from a stack of packages, before any machine has seen them.
//!
//! The counterpart of `km_api::book`, which builds one from a machine's catalog. Same renderer,
//! same [`arrange`](km_songbook::arrange()) ordering, same section naming — only the source differs, and the one
//! place that matters is where the bank comes from.
//!
//! # The bank the id implies, used directly
//!
//! A song's number is `bank * 1000 + slot`. A machine decides the bank when it installs a package;
//! nothing has installed these, so [`PackageMeta::wanted_bank`](km_kmpkg::PackageMeta::wanted_bank)
//! is the answer here — the bank the package's id implies, which is the one a machine reaches for
//! too.
//!
//! That makes this the same decision seen from both ends rather than a contradiction of it. The id
//! is the only thing about a package every machine reads the same way, so a book printed before
//! anybody has installed the files is right on the machines that install them.
//!
//! `--bank` is the exception, and it is about a machine rather than about a package: an owner may
//! move an installed package with `PUT /api/v1/packages/{id}/bank`, and this is how the book they
//! print afterwards follows it.
//!
//! # It warns rather than refusing
//!
//! Both problems below are reported and neither is fatal, because both describe a book somebody may
//! still want. A collision is the interesting one: two songs under one code is a book that **lies**,
//! and it is what `Library::install` makes impossible — but packages bound for two different
//! machines may legitimately be given the same bank, and refusing would make this command useless
//! for them. So it is said, loudly, with both songs named.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use km_kmpkg::{Language, Manifest, Package};
use km_songbook::{BookSong, BookStyle, Entry, SortKey};
use km_songcode::SongCode;

/// The heading songs with no language recorded appear under.
///
/// The same words `km_api::book` uses, deliberately: a book of a package and a book of the machine
/// that installed it should not name the same section two different things.
pub const UNCLASSIFIED: &str = "No language recorded";

/// Something about a package a person should know before they print it.
///
/// **There were four of these and there are two.** `NoPrefix` and `BadPrefix` cannot happen any
/// more: a bank is a `u16` rather than free text, so it cannot be malformed, and a package that
/// suggests nothing gets one from its own id rather than printing a blank column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// Two packages want the same bank, which a machine would refuse outright.
    SharedBank {
        /// The bank both want.
        bank: u16,
        /// The package that claimed it first.
        first: String,
        /// The package that wanted it too.
        second: String,
    },
    /// Two songs would be printed under one code.
    Collision {
        /// The code they share.
        code: SongCode,
        /// The song already printed under it.
        first: String,
        /// The song that wants it too.
        second: String,
    },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::SharedBank {
                bank,
                first,
                second,
            } => write!(
                f,
                "{first} and {second} both want bank {bank} — a machine would put the second \
                 somewhere else, so this book's numbers for it would be wrong; \
                 set one with --bank {second}=<N>"
            ),
            Warning::Collision {
                code,
                first,
                second,
            } => write!(
                f,
                "two songs share the code {code}: '{first}' and '{second}' — one of them is not \
                 the song somebody reading this book will get"
            ),
        }
    }
}

/// One package, opened and named.
///
/// **The manifest and not the `Package`.** A book is made entirely of metadata — number, title,
/// artist, language, the first line of the words — and never touches a song's bytes, so holding the
/// archive open for the length of a build would be holding a file handle for nothing. It also means
/// a test can build one of these without a `.kmpkg` on disk.
pub struct Loaded {
    /// What to call it in a warning: the file's stem, which is what somebody typed.
    pub name: String,
    /// Everything a book needs.
    pub manifest: Manifest,
}

impl Loaded {
    /// Opens a package, validates it, and keeps what a book needs.
    ///
    /// Through [`Package::open`] rather than [`km_kmpkg::read_manifest_unchecked`], because a
    /// manifest a machine would refuse should be refused here too: a book of a package that cannot
    /// be installed is a book of songs nobody will be able to dial.
    pub fn open(path: &Path) -> Result<Self, km_kmpkg::PackageError> {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let package = Package::open(path)?;
        Ok(Self {
            name,
            manifest: package.manifest().clone(),
        })
    }
}

/// Reads `--bank` values: `NAME=N`, or a bare `N` when only one package was named.
///
/// The bare form is the common case — one volume, one bank — and the paired form is what makes
/// the flag usable at all once there are several.
pub fn parse_overrides(
    values: &[String],
    packages: &[Loaded],
) -> Result<BTreeMap<String, u16>, String> {
    let mut overrides = BTreeMap::new();
    for value in values {
        let (name, code) = match value.split_once('=') {
            Some((name, code)) => (name.trim().to_owned(), code.trim()),
            None if packages.len() == 1 => (packages[0].name.clone(), value.trim()),
            None => {
                return Err(format!(
                    "--bank {value} needs to say which package it is for, because more than one \
                     was named: --bank <PACKAGE>=<N>"
                ));
            }
        };
        // From 1: bank 0 is the machine's own, so no machine will put a package there and a book
        // printed with those numbers would be right nowhere.
        let bank: u16 = code
            .parse()
            .ok()
            .filter(|n| (1..=km_songcode::MAX_BANK).contains(n))
            .ok_or_else(|| {
                format!(
                    "--bank {value}: '{code}' is not a bank (1 to {})",
                    km_songcode::MAX_BANK
                )
            })?;
        if !packages.iter().any(|loaded| loaded.name == name) {
            let known: Vec<&str> = packages.iter().map(|l| l.name.as_str()).collect();
            return Err(format!(
                "--bank {value} names '{name}', which is not one of the packages given ({})",
                known.join(", ")
            ));
        }
        overrides.insert(name, bank);
    }
    Ok(overrides)
}

/// Turns packages into book rows, and says what is wrong with them.
#[must_use]
pub fn entries(
    packages: &[Loaded],
    overrides: &BTreeMap<String, u16>,
    default_language: Option<&str>,
    tags: &[String],
) -> (Vec<Entry>, Vec<Warning>) {
    let mut warnings = Vec::new();
    let mut claimed: HashMap<u16, String> = HashMap::new();
    let mut seen: HashMap<SongCode, String> = HashMap::new();
    let mut entries = Vec::new();

    for loaded in packages {
        let meta = &loaded.manifest.package;
        // **Through `wanted_bank`, which the machine also calls.** Two copies of the question is how
        // they come to have two answers — a package printing one number in a book and installing
        // under another — so this asks rather than deriving the same thing again here.
        let bank = match overrides.get(&loaded.name) {
            Some(chosen) => *chosen,
            None => meta.wanted_bank(),
        };
        if let Some(first) = claimed.get(&bank) {
            warnings.push(Warning::SharedBank {
                bank,
                first: first.clone(),
                second: loaded.name.clone(),
            });
        } else {
            claimed.insert(bank, loaded.name.clone());
        }

        for song in &loaded.manifest.songs {
            // **A filter, never a heading.** The book is sectioned by language, and it stays that
            // way: a language is a closed table with an English name per row, and an open vocabulary
            // has neither an order nor a name to head a section with — and a song carrying three
            // tags would have to appear in three sections or arbitrarily in one. So tags decide
            // which songs go in, and every one that does is filed under its language as before.
            //
            // Any tag, not every: the same OR `km_api::book::BookFilter` makes, so a book printed
            // from `--tags rock,brasil` holds exactly the songs a remote showing that filter lists.
            //
            // **The emptiness is checked first**, because no tags at all is no tag filter, where an
            // `any` over nothing is nothing.
            if !tags.is_empty() && !tags.iter().any(|wanted| song.tags.contains(wanted)) {
                continue;
            }
            // Neither half can reach the fallback, so it is unreachable rather than a policy: a
            // slot outside the bank is refused by `Package::open`, which `Loaded::open` goes
            // through, and `bank` is 1 to `MAX_BANK` from either source above.
            let code = u16::try_from(song.number)
                .ok()
                .and_then(|slot| SongCode::in_bank(bank, slot))
                .unwrap_or_else(|| SongCode::new(song.number));
            if let Some(first) = seen.get(&code) {
                warnings.push(Warning::Collision {
                    code,
                    first: first.clone(),
                    second: song.title.clone(),
                });
            } else {
                seen.insert(code, song.title.clone());
            }

            // A song's own language, then the package's default. The second is what
            // `A package's default language` fills in at build time, so most packages will not
            // need it — but one built before that decision can still be printed.
            let language = song
                .language
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or(default_language);

            let section = section_of(language);
            entries.push(Entry {
                sort: SortKey {
                    artist_missing: song.artist.is_none(),
                    artist: km_song::text::fold(song.artist.as_deref().unwrap_or_default()),
                    title: km_song::text::fold(&song.title),
                },
                section_sort: km_song::text::fold(&section),
                section,
                song: BookSong {
                    artist: song.artist.clone(),
                    number: code,
                    title: song.title.clone(),
                    first_line: song.lyric_preview.first().cloned(),
                },
            });
        }
    }
    (entries, warnings)
}

/// What section a language code puts a song in.
///
/// Kept in step with `km_api::book::section_of` by having the same rule and the same reason: the
/// **raw** code identifies a section and the name is only what is printed, so a value the ISO table
/// does not know prints as itself rather than being swept in with the unclassified.
fn section_of(language: Option<&str>) -> String {
    match language.map(str::trim).filter(|code| !code.is_empty()) {
        None => UNCLASSIFIED.to_owned(),
        Some(code) => {
            Language::parse(code).map_or_else(|| code.to_owned(), |known| known.name().to_owned())
        }
    }
}

/// What a book calls itself: the machine's name top left, the document's title centered.
///
/// A pair rather than two arguments because they are two `Option<&str>` in a row, which is the
/// shape a caller silently gets the wrong way round.
#[derive(Debug, Clone, Copy, Default)]
pub struct Naming<'a> {
    /// Top left of every page. `KaraokeMachine` when nobody says.
    pub name: Option<&'a str>,
    /// Centered above the columns. `SONG LIST` when nobody says.
    pub title: Option<&'a str>,
}

/// The words on every page.
#[must_use]
pub fn style(naming: Naming<'_>, packages: &[Loaded], rows: usize) -> BookStyle {
    let names: Vec<&str> = packages.iter().map(|loaded| loaded.name.as_str()).collect();
    let mut subtitle = format!("{rows} song{}", if rows == 1 { "" } else { "s" });
    // Named while there are few enough to read; counted after that, because a subtitle listing
    // thirty volumes is a subtitle nobody reads.
    if names.len() <= 4 {
        subtitle.push_str(&format!(" · {}", names.join(", ")));
    } else {
        subtitle.push_str(&format!(" · {} packages", names.len()));
    }

    let default = BookStyle::default();
    BookStyle {
        name: naming.name.map_or(default.name, str::to_owned),
        title: naming.title.unwrap_or("SONG LIST").to_owned(),
        subtitle: Some(subtitle),
        empty_message: "No songs.".to_owned(),
        ..BookStyle::default()
    }
}

/// Where the book goes when nobody says.
///
/// One package gets its own name with a `.pdf` extension, the way `km-pack export` names its CSV;
/// several get a generic name, since no one of them is the book.
#[must_use]
pub fn default_out(packages: &[PathBuf]) -> PathBuf {
    match packages {
        [only] => {
            let mut path = only.clone();
            path.set_extension("pdf");
            path
        }
        _ => PathBuf::from("songbook.pdf"),
    }
}

/// The whole job: packages in, book and warnings out.
#[must_use]
pub fn build(
    packages: &[Loaded],
    overrides: &BTreeMap<String, u16>,
    default_language: Option<&str>,
    tags: &[String],
    naming: Naming<'_>,
) -> (km_songbook::Book, Vec<Warning>) {
    let (entries, warnings) = entries(packages, overrides, default_language, tags);
    let style = style(naming, packages, entries.len());
    (km_songbook::build(entries, UNCLASSIFIED, style), warnings)
}

#[cfg(test)]
mod tests {
    use km_kmpkg::{Manifest, PackageMeta, SongEntry, SongKind};

    use super::*;

    fn song(number: u32, title: &str, artist: Option<&str>, language: Option<&str>) -> SongEntry {
        SongEntry {
            number,
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            language: language.map(str::to_owned),
            kind: SongKind::Midi,
            file: format!("songs/{number}.kar"),
            duration_ms: 1000,
            lyric_encoding: None,
            default_transpose: 0,
            fixes: Vec::new(),
            melody: None,
            melody_abstained: None,
            suitability: None,
            lyric_preview: vec![format!("first line of {number}")],
            tags: Vec::new(),
            loudness: None,
            content_hash: None,
            edited: Vec::new(),
        }
    }

    /// A `Loaded` without a file on disk, which is what holding the manifest rather than the
    /// `Package` buys: none of this needs a `.kmpkg` to exist.
    /// The id is the name, so `bank_of(name)` is the bank these packages print under.
    fn loaded(name: &str, songs: Vec<SongEntry>) -> Loaded {
        Loaded {
            name: name.to_owned(),
            manifest: Manifest {
                format: km_kmpkg::FORMAT_VERSION_MIDI_ONLY,
                package: PackageMeta {
                    id: name.to_owned(),
                    name: name.to_owned(),
                    version: "1.0.0".to_owned(),
                    publisher: None,
                    created: None,
                    volume: None,
                },
                songs,
            },
        }
    }

    fn bank_of(name: &str) -> u16 {
        km_kmpkg::PackageMeta::suggested_bank(name)
    }

    fn codes(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|e| e.song.number.to_string()).collect()
    }

    /// **The whole point of the subcommand.** The machine decides a bank at install; nothing has
    /// installed these, so the bank the package's id implies is the answer — and it is the same one
    /// the machine will reach, which is what makes a book printed in advance right.
    #[test]
    fn a_packages_numbers_come_from_the_bank_its_id_implies() {
        let packages = vec![loaded("vol1", vec![song(500, "One", None, Some("pt"))])];
        let (entries, warnings) = entries(&packages, &BTreeMap::new(), None, &[]);
        assert_eq!(entries[0].song.number.bank(), bank_of("vol1"));
        assert_eq!(entries[0].song.number.slot(), 500);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// `--tags` takes the songs carrying any of them, and naming none is no tag filter.
    ///
    /// The second half is the one worth a test: `any` over an empty list is false, so a book asked
    /// for without `--tags` would print nothing. The same pair `km_api::book::BookFilter` asserts,
    /// because these are the two adapters that have to agree.
    #[test]
    fn the_tag_filter_takes_the_songs_carrying_any_tag() {
        let tagged = |number: u32, title: &str, tags: &[&str]| {
            let mut song = song(number, title, None, Some("pt"));
            song.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
            song
        };
        let packages = || {
            vec![loaded(
                "vol1",
                vec![
                    tagged(1, "Both", &["brasil", "rock"]),
                    tagged(2, "Rock only", &["rock"]),
                    tagged(3, "Brasil only", &["brasil"]),
                    tagged(4, "Neither", &[]),
                ],
            )]
        };
        let titles = |tags: &[&str]| {
            let wanted: Vec<String> = tags.iter().map(|tag| (*tag).to_owned()).collect();
            let (entries, _) = entries(&packages(), &BTreeMap::new(), None, &wanted);
            let mut found: Vec<String> = entries.into_iter().map(|e| e.song.title).collect();
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
    fn an_override_beats_the_bank_the_id_implies() {
        let packages = vec![loaded("vol1", vec![song(1, "One", None, None)])];
        let overrides = parse_overrides(&["7".to_owned()], &packages).expect("a bare bank");
        let (entries, warnings) = entries(&packages, &overrides, None, &[]);
        assert_eq!(codes(&entries), ["7001"]);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_bare_override_is_refused_when_it_cannot_say_which_package_it_means() {
        let packages = vec![loaded("vol1", Vec::new()), loaded("vol2", Vec::new())];
        let error = parse_overrides(&["9".to_owned()], &packages).expect_err("ambiguous");
        assert!(error.contains("which package"), "{error}");

        let named = parse_overrides(&["vol2=9".to_owned()], &packages).expect("named");
        assert_eq!(named.get("vol2").copied(), Some(9));
    }

    #[test]
    fn an_override_for_a_package_that_was_not_given_is_a_mistake_worth_naming() {
        let packages = vec![loaded("vol1", Vec::new())];
        let error = parse_overrides(&["vol9=9".to_owned()], &packages).expect_err("unknown");
        assert!(error.contains("vol9"), "{error}");
        assert!(
            error.contains("vol1"),
            "it should say what there was: {error}"
        );
    }

    /// Two packages in one bank is a machine's problem too, so the book says so.
    ///
    /// It takes overrides to arrange, and that is the shape of the thing rather than an awkward
    /// test: left alone, two ids land in two thousands. A machine that meets the collision puts the
    /// second package in the next free bank, so a book that printed both under one number would be
    /// wrong about one of them.
    #[test]
    fn two_packages_wanting_one_bank_is_what_a_machine_would_put_elsewhere() {
        let packages = vec![
            loaded("vol1", vec![song(1, "One", None, None)]),
            loaded("vol2", vec![song(2, "Two", None, None)]),
        ];
        let overrides =
            parse_overrides(&["vol1=3".to_owned(), "vol2=3".to_owned()], &packages).expect("banks");
        let (_, warnings) = entries(&packages, &overrides, None, &[]);
        assert!(
            warnings
                .iter()
                .any(|w| matches!(w, Warning::SharedBank { bank, .. } if *bank == 3)),
            "{warnings:?}"
        );
    }

    /// A book listing two songs under one code is a book that lies, and this is the case where it
    /// would: two volumes forced into the same bank, so their slot 500s are one number.
    ///
    /// **It takes an override to reach now**, which is the point rather than an inconvenience. Two
    /// packages left alone are banked from their own ids and land in different thousands; this is
    /// somebody insisting, and being told what it costs.
    #[test]
    fn two_songs_under_one_code_are_reported_by_name() {
        let packages = vec![
            loaded("vol1", vec![song(500, "Tempo Perdido", None, None)]),
            loaded("vol2", vec![song(500, "Like a Prayer", None, None)]),
        ];
        let overrides =
            parse_overrides(&["vol1=3".to_owned(), "vol2=3".to_owned()], &packages).expect("banks");
        let (entries, warnings) = entries(&packages, &overrides, None, &[]);
        assert_eq!(entries.len(), 2, "both are still printed");
        let collision = warnings
            .iter()
            .find_map(|w| match w {
                Warning::Collision {
                    code,
                    first,
                    second,
                } => Some((code, first, second)),
                _ => None,
            })
            .expect("a collision");
        assert_eq!(collision.0.to_string(), "3500");
        assert_eq!(collision.1, "Tempo Perdido");
        assert_eq!(collision.2, "Like a Prayer");
    }

    /// Bank 0 is the machine's own, so the flag that types a bank refuses it.
    ///
    /// The book is where the refusal is cheapest to read: a number printed on paper cannot be
    /// corrected by the machine that finds it wrong.
    #[test]
    fn bank_zero_cannot_be_printed_because_no_machine_will_hand_it_out() {
        let packages = vec![loaded("vol1", vec![song(1, "One", None, None)])];
        let error = parse_overrides(&["vol1=0".to_owned()], &packages).expect_err("refused");
        assert!(error.contains("1 to"), "it should name the range: {error}");
    }

    /// The banks are what stop it: the same slot in two volumes is two different numbers.
    ///
    /// And nobody arranges that — two ids hash apart, so two volumes handed out together are in two
    /// thousands with no curator having chosen anything.
    #[test]
    fn banks_are_what_let_two_volumes_number_a_song_the_same() {
        let packages = vec![
            loaded("vol1", vec![song(500, "Tempo Perdido", None, None)]),
            loaded("vol2", vec![song(500, "Like a Prayer", None, None)]),
        ];
        let (entries, warnings) = entries(&packages, &BTreeMap::new(), None, &[]);
        assert_eq!(entries[0].song.number.slot(), 500);
        assert_eq!(entries[1].song.number.slot(), 500);
        assert_eq!(entries[0].song.number.bank(), bank_of("vol1"));
        assert_eq!(entries[1].song.number.bank(), bank_of("vol2"));
        assert_ne!(entries[0].song.number, entries[1].song.number);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_song_with_no_language_falls_back_to_the_default_then_to_the_unclassified_section() {
        let packages = vec![loaded("vol1", vec![song(1, "One", None, None)])];
        let (with_default, _) = entries(&packages, &BTreeMap::new(), Some("pt"), &[]);
        assert_eq!(with_default[0].section, "Portuguese");
        let (without, _) = entries(&packages, &BTreeMap::new(), None, &[]);
        assert_eq!(without[0].section, UNCLASSIFIED);
    }

    #[test]
    fn one_package_names_the_book_after_itself() {
        assert_eq!(
            default_out(&[PathBuf::from("./out/vol1.kmpkg")]),
            PathBuf::from("./out/vol1.pdf")
        );
        assert_eq!(
            default_out(&[PathBuf::from("a.kmpkg"), PathBuf::from("b.kmpkg")]),
            PathBuf::from("songbook.pdf")
        );
    }

    #[test]
    fn a_subtitle_names_a_few_packages_and_counts_many() {
        let few = vec![loaded("vol1", Vec::new()), loaded("vol2", Vec::new())];
        assert!(
            style(Naming::default(), &few, 9)
                .subtitle
                .expect("one")
                .contains("vol1, vol2"),
            "a couple should be named"
        );
        let many: Vec<Loaded> = (0..7)
            .map(|n| loaded(&format!("vol{n}"), Vec::new()))
            .collect();
        assert!(
            style(Naming::default(), &many, 9)
                .subtitle
                .expect("one")
                .contains("7 packages"),
            "a shelf should be counted"
        );
    }

    #[test]
    fn a_whole_book_renders() {
        let packages = vec![loaded(
            "vol1",
            vec![
                song(1, "Tempo Perdido", Some("Legiao Urbana"), Some("pt")),
                song(2, "Like a Prayer", Some("Madonna"), Some("en")),
            ],
        )];
        let (book, warnings) = build(
            &packages,
            &BTreeMap::new(),
            None,
            &[],
            Naming {
                title: Some("KARAOKE"),
                ..Naming::default()
            },
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(book.row_count(), 2);
        assert!(book.render().starts_with(b"%PDF"));
    }
}
