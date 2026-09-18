//! The printed song book: a PDF listing every song a machine can play.
//!
//! A commercial home karaoke machine ships with a book. Every song it holds, on paper, so a room
//! full of people can find something to sing without queueing at the one screen — a catalog, a
//! search box and a phone all need the machine switched on and the network working, and a book
//! needs neither. This crate is that book.
//!
//! # Four columns, and why the code is one of them
//!
//! ```text
//! KaraokeMachine                  SONG LIST                        1,204 songs
//! ┌─────────────────┬──────┬──────────────────────┬───────────────────────────┐
//! │ ARTIST          │ CODE │ TITLE                │ FIRST LINE                │
//! ├─────────────────┴──────┴──────────────────────┴───────────────────────────┤
//! │ Portuguese                                                                │
//! ├─────────────────┬──────┬──────────────────────┬───────────────────────────┤
//! │ Legião Urbana   │ 3500 │ Tempo Perdido        │ Todos os dias quando a…   │
//! ├─────────────────┼──────┼──────────────────────┼───────────────────────────┤
//! │                 │ 3512 │ Faroeste Caboclo     │ Não tinha medo o tal Jo…  │
//! ├─────────────────┼──────┼──────────────────────┼───────────────────────────┤
//! │ Roberto Carlos  │   88 │ Detalhes             │ Não adianta nem tentar…   │
//! └─────────────────┴──────┴──────────────────────┴───────────────────────────┘
//! ```
//!
//! **There were five, and the fifth held the package prefix.** It was separate because `BR500` run
//! together reads as a six-figure number, so the eye could not find the number it was given. The
//! code *is* a six-figure number now — `3500` is bank 3, slot 500 — and printing it whole is the
//! only correct answer rather than a compromise.
//!
//! A `VOL` column holding the bank was considered, since it is what the commercial book does with
//! the volume a song lives on, and **rejected**: a reader would have to concatenate `3` and `005` to
//! dial 3500, which works only if the slot is zero-padded to three digits, and a book whose code
//! column cannot be read off in one piece is worse than one column narrower. The 24.2 pt went to the
//! title and the first line, half each, being the two columns that truncate.
//!
//! The code is **centered** and the three text columns are ranged left, which is how the book this
//! copies sets them: a short number adrift at the left of its cell reads as a mistake rather than as
//! a choice.
//!
//! # The table is ruled, and a section heading is one cell
//!
//! Every cell is boxed, the column headings repeat on each page, and the grid **stops at the last
//! row** rather than ruling empty cells down to the bottom margin — all three measured from the
//! reference rather than invented. A section heading is a band across all four columns with no
//! vertical through it, so it reads as one merged cell rather than as four with a word in the
//! first. See `layout::grid_segments`, which returns that geometry rather than drawing it.
//!
//! # No font ships with this
//!
//! A PDF reader is required to supply the glyphs and metrics for the fourteen standard fonts, so a
//! book set in `/Helvetica` embeds nothing: no font file, no font parser, no glyph subsetting, no
//! CID machinery. That is the whole reason this crate takes `km-songcode` and no external dependency at
//! all, and it is the same judgment `km-cdg` made about rendering CD+G rather than converting it.
//!
//! The price is that text is **WinAnsi** — cp1252, so Western European and no further. Latin letters
//! cp1252 lacks are transliterated (`ā` prints as `a`); anything else becomes `?` and is **counted**,
//! and the count is reported rather than swallowed. See [`winansi`]. That boundary is the standing
//! `No complex-script (CJK/Thai/Arabic) text shaping` non-goal, met again in a second place.
//!
//! # Using it
//!
//! ```
//! use km_songbook::{BookSong, BookStyle, Entry, SortKey};
//! use km_songcode::SongCode;
//!
//! let entries = vec![Entry {
//!     song: BookSong {
//!         artist: Some("Legião Urbana".to_owned()),
//!         number: SongCode::new(500),
//!         title: "Tempo Perdido".to_owned(),
//!         first_line: Some("Todos os dias quando acordo".to_owned()),
//!     },
//!     section: "Portuguese".to_owned(),
//!     // Both folded by the caller — see `arrange` for why this crate does not fold.
//!     section_sort: "portuguese".to_owned(),
//!     sort: SortKey {
//!         artist_missing: false,
//!         artist: "legiao urbana".to_owned(),
//!         title: "tempo perdido".to_owned(),
//!     },
//! }];
//!
//! let book = km_songbook::build(entries, "No language recorded", BookStyle::default());
//! assert_eq!(book.row_count(), 1);
//! let pdf: Vec<u8> = book.render();
//! assert!(pdf.starts_with(b"%PDF"));
//! ```
//!
//! # What this crate does not decide
//!
//! **Which songs, in what sections, under what names.** Rows arrive already carrying their heading
//! and their sort key, because naming a language needs `km_kmpkg::Language` and folding a name
//! needs `km_song::text::fold`, and a crate that draws rectangles should not drag `midly` and an
//! encoding detector behind it. Two adapters do that work — `km_api::book` for a machine's
//! catalog and `km_pack::book` for a stack of `.kmpkg` files — and both hand their rows here, so
//! the *order* is still decided once. See [`arrange()`].

pub mod arrange;
pub mod layout;
pub mod metrics;
pub mod pdf;
pub mod winansi;

pub use arrange::{Entry, SortKey, arrange};
pub use layout::{Book, BookStyle, build};
pub use metrics::Font;
pub use winansi::Replacements;

use km_songcode::SongCode;

/// One row of the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSong {
    /// The performer. `None` prints an empty cell and sorts after every named artist.
    pub artist: Option<String>,
    /// The song's code, printed whole in the CODE column — see the crate header for why the bank is
    /// not set apart in a column of its own.
    pub number: SongCode,
    /// The title, as the catalog holds it. Case is kept: these are hand-curated in
    /// km-package-builder, and the commercial book's shouting capitals are not an improvement.
    pub title: String,
    /// The first line of the words, from `lyric_preview`.
    ///
    /// `None` for a video song and an MP3+G song, whose words are pixels — the same reasoning as
    /// the `Searching a video's words` decision — and for any song whose package predates previews.
    /// The cell is simply empty; a reader loses nothing but a hint.
    pub first_line: Option<String>,
}

/// A heading and the rows under it, in the order they will be printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSection {
    /// What the divider says. `Portuguese`, not `pt` — the adapter has already named it.
    pub heading: String,
    /// The rows, already ordered. [`arrange()`] is what puts them in order.
    pub songs: Vec<BookSong>,
}
