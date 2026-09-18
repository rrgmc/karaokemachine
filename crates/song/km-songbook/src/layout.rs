//! The page: four columns, fifty-two rows, and where each of them goes.
//!
//! The geometry is taken from a real commercial machine's book — A4 portrait, 233 pages for 11,999
//! songs, which is 52 rows a page at about 7pt. Copying a shape that a room full of people has
//! already proved they can read beats inventing one.
//!
//! # Pagination encodes to WinAnsi; rendering does not
//!
//! A cell has to be **measured** to know whether it needs truncating, and
//! [`crate::metrics::text_width`] measures WinAnsi bytes rather than a `&str` — so encoding happens
//! here, once, and a laid-out page holds bytes. Two things fall out of that and both are wanted:
//! nothing is encoded twice, and [`Book::replaced`] can be answered *before* anything is drawn,
//! which is what lets the command line report the cost.

use crate::arrange::Entry;
use crate::metrics::{self, Font};
use crate::pdf::{self, Content, Writer};
use crate::winansi::{self, Replacements};
use crate::{BookSection, BookSong};

/// The margin at the left and right edges, in points.
pub const MARGIN: f32 = 28.0;
/// The gap between two columns.
pub const GUTTER: f32 = 4.0;
/// The gap between a cell's rule and the text in it — half a gutter, so that two columns either
/// side of one rule are padded off it equally.
///
/// **Every cell has this on both sides, the first and the last included.** They did not, at first:
/// the outer rules were drawn at [`MARGIN`] and [`RIGHT_EDGE`], which are the first column's own
/// left edge and the last one's own right edge, so the artist name was set *on* the rule and the
/// last column's text sat a point off its cell's center. Both are the same mistake, and it is
/// invisible until the table is ruled — which is why it survived the first cut.
const CELL_PAD: f32 = GUTTER / 2.0;

/// Baseline of the masthead and the document title, which share it.
const TITLE_BASELINE: f32 = 806.0;
/// Baseline of the small right-hand note under the title.
const SUBTITLE_BASELINE: f32 = 795.0;
/// Baseline of the column headings.
const HEADINGS_BASELINE: f32 = 782.0;
/// Baseline of the first body row.
const FIRST_ROW_BASELINE: f32 = 768.0;
/// The distance between one row's baseline and the next.
const ROW_HEIGHT: f32 = 14.0;
/// How far above a baseline the rule over that row sits.
///
/// With [`ROW_DROP`] this splits [`ROW_HEIGHT`] the way the reference book splits its own — its
/// baselines sit 3.12 pt above the rule under them in a 13.56 pt row. The consequence worth
/// knowing: the bottom of the column-heading row lands at 778.5, which is where the reference book
/// hand-places its one header rule — so here that rule is simply a line of the grid.
const ROW_RISE: f32 = 10.5;
/// …and how far below it the rule under that row sits. `ROW_RISE + ROW_DROP == ROW_HEIGHT`.
const ROW_DROP: f32 = 3.5;
/// Baseline of the page number.
const FOOTER_BASELINE: f32 = 30.0;

/// How many rows fit on a page.
///
/// Every page gets all of them. A section heading used to cost two of them on the page a section
/// started on — it was set larger and wanted the blank under it — and it is now a running header in
/// the masthead instead, which costs no body row on any page. See [`Book::chrome`].
pub const SLOTS_PER_PAGE: usize = 52;

/// The body face's size.
const BODY_SIZE: f32 = 7.0;
/// The column headings' size.
const HEADING_SIZE: f32 = 7.0;
/// The document title's size.
const TITLE_SIZE: f32 = 10.0;
/// The subtitle's and the footer's size.
const SMALL_SIZE: f32 = 6.5;

/// Gray for the small print. 0.0 is black.
const GRAY: f32 = 0.45;

/// The grid's line weight.
///
/// The reference draws 0.72 pt in solid black, but its type is 11.16 pt against this book's 7 pt,
/// so the same weight lands proportionally half again as heavy here. A hairline in the same family
/// as the rule the book already drew keeps the words dominant over the table they sit in.
const GRID_WEIGHT: f32 = 0.4;
/// Gray for the grid.
const GRID_GRAY: f32 = 0.35;

/// How a column's text sits in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    /// The code, as the reference book sets it.
    Center,
}

/// One column of the table.
struct Column {
    /// Its left edge — **always**, whatever the alignment. The grid's verticals are drawn from it.
    x: f32,
    /// How much room its text has.
    width: f32,
    /// How the text sits between those two.
    align: Align,
}

impl Column {
    /// Where text is drawn from, given how it is aligned.
    const fn anchor(&self) -> f32 {
        match self.align {
            Align::Left => self.x,
            Align::Center => self.x + self.width / 2.0,
        }
    }
}

/// The four columns, in the order the reference book has them.
///
/// Widths are the owner's proportions, near enough: artist 29%, code 7%, title 30%, first line 32%.
/// They total 523.3 of the 539.3 pt between the margins, the other 16 being [`CELL_PAD`] on each
/// side of four cells. The right-hand edge works out at [`RIGHT_EDGE`].
///
/// **There were five**, the last a narrow `PREF` holding the package prefix. Its 24.2 pt and the
/// gutter beside it were given to the title and the first line, half each, because those are the two
/// that truncate; the first three columns' geometry is measured off the reference and did not move.
///
/// The invariant that ties this to the grid, asserted in full by
/// `every_column_is_padded_off_its_rules`: a column's rules are at `x - CELL_PAD` and
/// `x + width + CELL_PAD`, and neighbors share one, so `x + width + GUTTER == next.x`.
const COLUMNS: [Column; 4] = [
    // ARTIST — a pad in from the margin rather than on it, so the name clears the left rule by the
    // same 2 pt every other column clears its own by. Spelled as the sum rather than as 30.0,
    // because that is what it is: the 2 pt comes out of this column's width, not out of the margin.
    Column {
        x: MARGIN + CELL_PAD,
        width: 155.0,
        align: Align::Left,
    },
    // CODE — centered, as the reference's `CÓD` is. Every digit is 556/1000 em in both faces
    // (asserted in `metrics`), so a column of numbers of the same length still lines up.
    Column {
        x: 189.0,
        width: 36.6,
        align: Align::Center,
    },
    // TITLE
    Column {
        x: 229.6,
        width: 160.6,
        align: Align::Left,
    },
    // FIRST LINE — the last column now, so its right rule is [`RIGHT_EDGE`].
    Column {
        x: 394.2,
        width: 171.1,
        align: Align::Left,
    },
];

/// The right-hand edge of the table, which the rules are drawn to.
const RIGHT_EDGE: f32 = pdf::PAGE_WIDTH - MARGIN - 0.02;

/// How much wider than its column a heading is, in points — `0.0` when it fits.
///
/// **Public for the one caller that does not write its headings in English.** `km-api` fills
/// [`BookStyle::column_headings`] from a Fluent catalog, so the four words change with the locale
/// and a translator cannot see a column width; [`Book::paginate`] ellipsizes what does not fit,
/// which is the right thing to do at print time and the wrong thing to discover in a printed book.
/// The check has to live where the words are, so the geometry comes out to meet it.
///
/// `column` indexes the four in the order of [`BookStyle::column_headings`], and is asserted rather
/// than defaulted: a fifth column would be a caller's mistake, not a heading that happens to fit.
///
/// Measured on the *encoded* heading, because that is what gets drawn — a character cp1252 lacks
/// becomes `?` first, and `?` is not the width of what it replaced.
#[must_use]
pub fn heading_overflow(column: usize, heading: &str) -> f32 {
    let width = COLUMNS[column].width;
    let encoded = winansi::encoded(heading, &mut Replacements::default());
    (metrics::text_width(&encoded, Font::Bold, HEADING_SIZE) - width).max(0.0)
}

/// What a book says about itself, on every page.
#[derive(Debug, Clone)]
pub struct BookStyle {
    /// Top left of every page: whose book this is. `KaraokeMachine`.
    ///
    /// A separate string from [`title`](Self::title) because they answer different questions — this
    /// one names the machine the book belongs to, that one names the document. A commercial book
    /// carries both, the owner's mark in one corner and `LISTA DE MÚSICAS` across the top.
    pub name: String,
    /// Centered at the top of every page. `SONG LIST`.
    pub title: String,
    /// A small note under it — the song count, the date, whatever the caller wants said.
    pub subtitle: Option<String>,
    /// The four column headings, in the order of [`COLUMNS`].
    pub column_headings: [String; 4],
    /// What a book with no rows in it says, centered on its one page.
    pub empty_message: String,
}

impl Default for BookStyle {
    fn default() -> Self {
        Self {
            name: "KaraokeMachine".to_owned(),
            title: "SONG LIST".to_owned(),
            subtitle: None,
            column_headings: [
                "ARTIST".to_owned(),
                "CODE".to_owned(),
                "TITLE".to_owned(),
                "FIRST LINE".to_owned(),
            ],
            empty_message: "No songs.".to_owned(),
        }
    }
}

/// One laid-out row: five cells of WinAnsi bytes, four of which may be empty.
#[derive(Debug, Default)]
struct Row {
    cells: [Vec<u8>; 4],
}

/// What goes on one page, once pagination has decided.
#[derive(Debug)]
struct Page {
    /// The section this page belongs to, carried by **every** page of it rather than by the first.
    ///
    /// That is what makes it a running header. A section is routinely twenty pages long, and a
    /// heading you have turned past is not on the page you are reading — which was the whole
    /// complaint against the band this replaced.
    section: Option<Vec<u8>>,
    rows: Vec<Row>,
}

/// A book, laid out and ready to render.
#[derive(Debug)]
pub struct Book {
    pages: Vec<Page>,
    style: EncodedStyle,
    rows: usize,
    replaced: Replacements,
}

/// [`BookStyle`], encoded once rather than once a page.
#[derive(Debug)]
struct EncodedStyle {
    name: Vec<u8>,
    title: Vec<u8>,
    raw_title: String,
    subtitle: Option<Vec<u8>>,
    headings: [Vec<u8>; 4],
    empty: Vec<u8>,
}

impl Book {
    /// Lays out sections into pages.
    ///
    /// Sections arrive already ordered — see [`arrange`](crate::arrange()), which is the one place that is
    /// decided. This function only decides where the page breaks fall.
    #[must_use]
    pub fn paginate(sections: Vec<BookSection>, style: BookStyle) -> Self {
        let mut replaced = Replacements::default();
        let encoded_style = EncodedStyle {
            name: winansi::encoded(&style.name, &mut replaced),
            title: winansi::encoded(&style.title, &mut replaced),
            raw_title: style.title.clone(),
            subtitle: style
                .subtitle
                .as_deref()
                .map(|text| winansi::encoded(text, &mut replaced)),
            // Fitted, exactly as a cell is. A heading is supplied by the caller and `km-api` fills
            // all four from a Fluent catalog, so a translation longer than its column is a thing a
            // build can grow — and an unfitted heading does not stop at the rule, it runs into the
            // next column's words. Ellipsized is the same bargain every cell already makes;
            // `km-api`'s `every_translated_heading_fits_its_column` is what stops it being struck.
            headings: {
                let mut column = 0;
                style.column_headings.each_ref().map(|text| {
                    let width = COLUMNS[column].width;
                    column += 1;
                    metrics::fit(
                        winansi::encoded(text, &mut replaced),
                        Font::Bold,
                        HEADING_SIZE,
                        width,
                    )
                })
            },
            empty: winansi::encoded(&style.empty_message, &mut replaced),
        };

        let mut pages: Vec<Page> = Vec::new();
        let mut rows = 0;
        for section in sections {
            // **A section starts a new page.** It restarts the alphabet, and two alphabets on one
            // sheet is a page somebody would read as one run and get wrong.
            let heading = winansi::encoded(&section.heading, &mut replaced);
            let mut page = Page {
                section: Some(heading.clone()),
                rows: Vec::new(),
            };
            let mut room = SLOTS_PER_PAGE;

            // The artist last printed, so a repeat can be blanked. `None` at the top of every page
            // as well as at the start of every section: a page that opens mid-artist must repeat
            // the name, or its first rows have no singer against them at all.
            let mut previous: Option<String> = None;

            for song in section.songs {
                if room == 0 {
                    pages.push(page);
                    page = Page {
                        // The same section, on every page of it: this is a running header.
                        section: Some(heading.clone()),
                        rows: Vec::new(),
                    };
                    room = SLOTS_PER_PAGE;
                    previous = None;
                }
                let artist = song.artist.as_deref().map(str::trim).unwrap_or_default();
                // Blanked on an **exact** match, not a folded one. Two spellings of a name are two
                // artists as far as the catalog is concerned, and printing one under the other
                // would be a claim the reader has no way to check.
                let repeat = !artist.is_empty() && previous.as_deref() == Some(artist);
                // A song with no artist **clears** the run rather than continuing it: it prints
                // [`NO_ARTIST`] rather than a blank, so the next named row must print its name
                // again or it would appear to belong to the dash.
                previous = (!artist.is_empty()).then(|| artist.to_owned());
                page.rows.push(lay_out(&song, repeat, &mut replaced));
                rows += 1;
                room -= 1;
            }
            // A section with no songs still gets its page, so the book says the section is empty
            // rather than skipping it silently.
            if !page.rows.is_empty() || page.section.is_some() {
                pages.push(page);
            }
        }

        Self {
            pages,
            style: encoded_style,
            rows,
            replaced,
        }
    }

    /// How many pages the book has. Never zero — an empty book is one page saying so.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len().max(1)
    }

    /// How many songs are in it.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows
    }

    /// What could not be drawn. See [`crate::winansi`].
    #[must_use]
    pub fn replaced(&self) -> &Replacements {
        &self.replaced
    }

    /// Draws the book and hands back the PDF.
    #[must_use]
    pub fn render(&self) -> Vec<u8> {
        let mut writer = Writer::new(&self.style.raw_title);
        let total = self.page_count();
        if self.pages.is_empty() {
            // **Never zero pages.** A PDF with an empty page tree is invalid, and a download of one
            // is baffling in a way a page saying "no songs" is not.
            let mut content = self.chrome(None, 0, 1, total);
            content.gray(0.0);
            content.text_centered(
                &self.style.empty,
                Font::Regular,
                BODY_SIZE + 2.0,
                pdf::PAGE_WIDTH / 2.0,
                FIRST_ROW_BASELINE - 40.0,
            );
            writer.page(content);
            return writer.finish();
        }
        for (index, page) in self.pages.iter().enumerate() {
            let mut content =
                self.chrome(page.section.as_deref(), page.rows.len(), index + 1, total);
            content.gray(0.0);
            for (slot, row) in page.rows.iter().enumerate() {
                let y = FIRST_ROW_BASELINE - slot as f32 * ROW_HEIGHT;
                for (cell, column) in row.cells.iter().zip(COLUMNS.iter()) {
                    match column.align {
                        Align::Left => content.text(cell, Font::Regular, BODY_SIZE, column.x, y),
                        Align::Center => {
                            content.text_centered(
                                cell,
                                Font::Regular,
                                BODY_SIZE,
                                column.anchor(),
                                y,
                            );
                        }
                    }
                }
            }
            writer.page(content);
        }
        writer.finish()
    }

    /// The parts of a page that are the same on every page: the grid, the masthead, the title, the
    /// section this page is in, the subtitle, the column headings and the page number.
    ///
    /// **The masthead is three fields on one baseline** — whose book this is at the left, what the
    /// document is in the middle, and which section this page belongs to at the right — with the
    /// subtitle a line under the third of them. A band across all four columns on the page a section
    /// *starts* on is the reference book's answer, and it leaves pages two through twenty of a
    /// section with nothing on the sheet saying which language they are. A running header is the
    /// ordinary answer to that and it costs no body row: `SLOTS_PER_PAGE` is every page's whole
    /// allowance. See `The song book` in docs/decisions/interface.md.
    ///
    /// `rows` is how many body rows this page carries, because the grid stops at the last one — a
    /// short final page is not ruled down to the bottom margin, which is what the reference does
    /// too.
    fn chrome(&self, section: Option<&[u8]>, rows: usize, page: usize, total: usize) -> Content {
        let mut content = Content::new();

        // **The grid first.** A content stream has no z-order, so whatever is drawn last is drawn
        // on top; text over its own rules is the way round that wants to be true.
        content.lines(GRID_WEIGHT, GRID_GRAY, &grid_segments(rows));

        content.gray(0.0);
        content.text(
            &self.style.name,
            Font::Bold,
            TITLE_SIZE,
            MARGIN,
            TITLE_BASELINE,
        );
        content.text_centered(
            &self.style.title,
            Font::Bold,
            TITLE_SIZE,
            pdf::PAGE_WIDTH / 2.0,
            TITLE_BASELINE,
        );
        if let Some(section) = section {
            content.text_right(section, Font::Bold, TITLE_SIZE, RIGHT_EDGE, TITLE_BASELINE);
        }
        if let Some(subtitle) = &self.style.subtitle {
            content.gray(GRAY);
            content.text_right(
                subtitle,
                Font::Regular,
                SMALL_SIZE,
                RIGHT_EDGE,
                SUBTITLE_BASELINE,
            );
            content.gray(0.0);
        }
        for (text, column) in self.style.headings.iter().zip(COLUMNS.iter()) {
            match column.align {
                Align::Left => {
                    content.text(text, Font::Bold, HEADING_SIZE, column.x, HEADINGS_BASELINE);
                }
                Align::Center => content.text_centered(
                    text,
                    Font::Bold,
                    HEADING_SIZE,
                    column.anchor(),
                    HEADINGS_BASELINE,
                ),
            }
        }
        content.gray(GRAY);
        let footer = format!("{page} / {total}");
        content.text_centered(
            footer.as_bytes(),
            Font::Regular,
            SMALL_SIZE,
            pdf::PAGE_WIDTH / 2.0,
            FOOTER_BASELINE,
        );
        content
    }
}

/// Every line of the table's grid, for a page carrying `rows` body rows.
///
/// Two things here are copied from the reference book rather than invented. The grid **stops at the
/// last row on the page**, so a section's short final page is not ruled out to the bottom margin.
/// And the column-heading row is **boxed like any other row**, repeating on every page.
///
/// **The interior verticals run the whole height.** A section heading merged across all four
/// columns is what cuts them in two, because nothing may cross such a band; the heading is in the
/// masthead here, so there is no band and no cut. Every page of the book is ruled identically,
/// which is what makes this a function of one number.
///
/// A function of one number and the constants, returning the geometry rather than drawing it, so
/// that a test can read the lines it asked for instead of parsing a content stream back out.
pub(crate) fn grid_segments(rows: usize) -> Vec<[f32; 4]> {
    let mut segments: Vec<[f32; 4]> = Vec::new();

    let top = HEADINGS_BASELINE + ROW_RISE;
    let headings_bottom = HEADINGS_BASELINE - ROW_DROP;
    let bottom = if rows == 0 {
        // An empty page — the one a book with no songs gets, and the one an empty section gets.
        // There is nothing to box below the headings.
        headings_bottom
    } else {
        FIRST_ROW_BASELINE - (rows - 1) as f32 * ROW_HEIGHT - ROW_DROP
    };

    // Horizontals: the top of the table, the line under the column headings, then one under every
    // row.
    segments.push([MARGIN, top, RIGHT_EDGE, top]);
    segments.push([MARGIN, headings_bottom, RIGHT_EDGE, headings_bottom]);
    for row in 0..rows {
        let y = FIRST_ROW_BASELINE - row as f32 * ROW_HEIGHT - ROW_DROP;
        segments.push([MARGIN, y, RIGHT_EDGE, y]);
    }

    // Verticals, all of them the full height of what was drawn.
    segments.push([MARGIN, top, MARGIN, bottom]);
    segments.push([RIGHT_EDGE, top, RIGHT_EDGE, bottom]);
    for column in &COLUMNS[1..] {
        let x = column.x - GUTTER / 2.0;
        segments.push([x, top, x, bottom]);
    }

    segments
}

/// What the artist column says when the catalog does not know who sang it.
///
/// **Not a blank**, and the distinction is the whole reason there is a constant here. A blank in
/// that column already means "the same as the row above", so a song with no artist printed blank
/// would be filed under whoever happened to precede it — which in a book somebody reads a number
/// out of is a wrong answer rather than a missing one. An em dash says nobody knows.
const NO_ARTIST: &str = "—";

/// Encodes one song's five cells and cuts each to its column.
fn lay_out(song: &BookSong, blank_artist: bool, replaced: &mut Replacements) -> Row {
    let named = song.artist.as_deref().unwrap_or_default().trim();
    let artist = if blank_artist {
        String::new()
    } else if named.is_empty() {
        NO_ARTIST.to_owned()
    } else {
        named.to_owned()
    };
    // The whole number, in one column. A fifth column holding the package prefix is what a
    // letter-prefixed code needs, because `BR500` run together reads as a six-figure number — and
    // the code *is* a six-figure number, so printing it whole is the only correct answer.
    //
    // **A `VOL` column holding the bank was considered and rejected**, though it is what the
    // reference book does: a reader would have to concatenate `3` and `005` to dial 3005, which
    // only works if the slot is zero-padded to three digits, and a book whose code column cannot be
    // read off in one piece is worse than one column narrower.
    let code = song.number.to_string();
    let first_line = song
        .first_line
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_owned();

    let texts = [artist, code, song.title.trim().to_owned(), first_line];
    let cells = texts.map(|text| winansi::encoded(&text, replaced));
    let mut index = 0;
    let cells = cells.map(|bytes| {
        let column = &COLUMNS[index];
        index += 1;
        metrics::fit(bytes, Font::Regular, BODY_SIZE, column.width)
    });
    Row { cells }
}

/// Lays a whole book out in one call: arrange, then paginate.
///
/// The two are separate types so each can be tested on its own, and joined here so a caller does
/// not have to know they are two.
#[must_use]
pub fn build(entries: Vec<Entry>, unclassified: &str, style: BookStyle) -> Book {
    Book::paginate(crate::arrange::arrange(entries, unclassified), style)
}

#[cfg(test)]
mod tests {
    use km_songcode::SongCode;

    use super::*;

    fn song(artist: Option<&str>, title: &str, number: u32) -> BookSong {
        BookSong {
            artist: artist.map(str::to_owned),
            number: SongCode::new(number),
            title: title.to_owned(),
            first_line: None,
        }
    }

    fn section(heading: &str, songs: Vec<BookSong>) -> BookSection {
        BookSection {
            heading: heading.to_owned(),
            songs,
        }
    }

    fn filler(count: usize) -> Vec<BookSong> {
        (0..count)
            .map(|n| {
                song(
                    Some(&format!("Artist {n:04}")),
                    &format!("Title {n:04}"),
                    n as u32 + 1,
                )
            })
            .collect()
    }

    fn page_sizes(book: &Book) -> Vec<usize> {
        book.pages.iter().map(|page| page.rows.len()).collect()
    }

    /// The invariant the whole vertical layout rests on: the last row must clear the page number.
    #[test]
    fn the_last_row_does_not_land_on_the_footer() {
        let last = FIRST_ROW_BASELINE - (SLOTS_PER_PAGE - 1) as f32 * ROW_HEIGHT;
        assert!(
            last >= FOOTER_BASELINE + ROW_HEIGHT,
            "last row at {last} clashes with the footer"
        );
    }

    /// The columns must add up to the page, or the rightmost one runs off the edge.
    #[test]
    fn the_columns_fit_between_the_margins() {
        let last = &COLUMNS[COLUMNS.len() - 1];
        assert!(last.x + last.width <= pdf::PAGE_WIDTH - MARGIN + 0.01);
        assert!(COLUMNS[0].x >= MARGIN);
        // **Every column's `x` is its left edge now**, whatever its alignment, so one statement
        // covers all four gaps — it could not before, when the code column's `x` was its right
        // edge and the gap either side of it had to be asserted by hand.
        for pair in COLUMNS.windows(2) {
            let [left, right] = pair else { unreachable!() };
            assert!(
                (left.x + left.width + GUTTER - right.x).abs() < 0.01,
                "a gutter between every pair, so the grid's verticals land midway"
            );
        }
    }

    /// No column's text is set on one of the grid's rules, and every one clears them equally.
    ///
    /// **The first and the last are what this is really for.** Draw their outer rules at `MARGIN`
    /// and `RIGHT_EDGE` — the artist column's own left edge and the last column's own right edge —
    /// and the artist name is set *on* the line while every other column clears its rule by 2 pt,
    /// with the last column's centered text a point left of its cell's center. Neither is visible
    /// until the table is ruled, which is why a grid can carry both and look finished.
    #[test]
    fn every_column_is_padded_off_its_rules() {
        for (index, column) in COLUMNS.iter().enumerate() {
            let left = if index == 0 {
                MARGIN
            } else {
                column.x - GUTTER / 2.0
            };
            let right = match COLUMNS.get(index + 1) {
                Some(next) => next.x - GUTTER / 2.0,
                None => RIGHT_EDGE,
            };
            assert!(
                (column.x - left - CELL_PAD).abs() < 0.01,
                "column {index} starts {} from its left rule, not {CELL_PAD}",
                column.x - left
            );
            assert!(
                (right - (column.x + column.width) - CELL_PAD).abs() < 0.01,
                "column {index} ends {} from its right rule, not {CELL_PAD}",
                right - (column.x + column.width)
            );
            // …and therefore a centered cell centers on its cell, not merely near it.
            if column.align == Align::Center {
                assert!((column.anchor() - (left + right) / 2.0).abs() < 0.01);
            }
        }
    }

    /// Every page of a section gets the whole allowance, the first one included.
    ///
    /// The heading used to take two of the 52 on the page a section started on, so a long section
    /// ran 50, 52, 52, … It is a running header in the masthead now and costs no body row anywhere,
    /// which is worth two pages in a book of two hundred. That arithmetic is the thing most likely
    /// to drift, so it is asserted exactly rather than approximately.
    #[test]
    fn every_page_of_a_section_holds_a_full_page_of_rows() {
        let book = Book::paginate(
            vec![section("Portuguese", filler(105))],
            BookStyle::default(),
        );
        assert_eq!(page_sizes(&book), [52, 52, 1]);
        assert_eq!(book.row_count(), 105);
    }

    /// The section is on **every** page of it, which is what makes it a running header.
    ///
    /// The complaint this answers: a section is routinely twenty pages long, and a band printed on
    /// the page the section started on left pages two through twenty saying nothing at all about
    /// which language they were.
    #[test]
    fn every_page_names_the_section_it_is_in() {
        let book = Book::paginate(
            vec![section("Portuguese", filler(105))],
            BookStyle::default(),
        );
        assert_eq!(book.pages.len(), 3);
        for (index, page) in book.pages.iter().enumerate() {
            assert_eq!(
                page.section.as_deref(),
                Some(b"Portuguese".as_slice()),
                "page {} does not say which section it is in",
                index + 1
            );
        }
    }

    #[test]
    fn a_section_starts_a_new_page_however_much_room_is_left() {
        let book = Book::paginate(
            vec![
                section("English", filler(1)),
                section("Portuguese", filler(1)),
            ],
            BookStyle::default(),
        );
        assert_eq!(page_sizes(&book), [1, 1]);
        assert_eq!(
            book.pages[1].section.as_deref(),
            Some(b"Portuguese".as_slice())
        );
    }

    #[test]
    fn a_repeated_artist_is_blanked_under_the_first() {
        let book = Book::paginate(
            vec![section(
                "English",
                vec![
                    song(Some("Queen"), "One", 1),
                    song(Some("Queen"), "Two", 2),
                    song(Some("Queen"), "Three", 3),
                    song(Some("Abba"), "Four", 4),
                ],
            )],
            BookStyle::default(),
        );
        let artists: Vec<&[u8]> = book.pages[0]
            .rows
            .iter()
            .map(|r| r.cells[0].as_slice())
            .collect();
        assert_eq!(artists, [&b"Queen"[..], b"", b"", b"Abba"]);
    }

    /// Without this, the top of a page is a run of songs with no singer against them — and a page
    /// is exactly what somebody reads on its own.
    #[test]
    fn an_artist_spanning_a_page_break_is_printed_again_at_the_top() {
        let songs: Vec<BookSong> = (0..60)
            .map(|n| song(Some("Roberto Carlos"), &format!("Song {n}"), n + 1))
            .collect();
        let book = Book::paginate(vec![section("Portuguese", songs)], BookStyle::default());
        assert_eq!(book.pages[0].rows[0].cells[0], b"Roberto Carlos");
        assert_eq!(book.pages[0].rows[1].cells[0], b"");
        assert_eq!(book.pages[1].rows[0].cells[0], b"Roberto Carlos");
    }

    /// The code column carries the whole number, bank included, and there is no column after it.
    ///
    /// A fifth column holding the package prefix is what keeps `BR500` from reading as one
    /// six-figure number. A code *is* one six-figure number, and a reader has to be able to take it
    /// off the page in one piece and dial it — which is exactly what a `VOL` column prevents.
    #[test]
    fn the_code_column_carries_the_whole_number() {
        let mut first = song(None, "In Bank One", 1);
        first.number = SongCode::in_bank(1, 500).expect("1500");
        let mut banked = song(None, "In Bank Three", 1);
        banked.number = SongCode::in_bank(3, 500).expect("3500");
        let book = Book::paginate(
            vec![section("English", vec![first, banked])],
            BookStyle::default(),
        );
        assert_eq!(book.pages[0].rows[0].cells[1], b"1500");
        assert_eq!(book.pages[0].rows[1].cells[1], b"3500");
        assert_eq!(
            book.pages[0].rows[0].cells.len(),
            4,
            "four columns, not five"
        );
    }

    /// A blank artist cell already means "the same as above". A song nobody attributed must
    /// therefore say something else, or it is filed under whoever precedes it.
    #[test]
    fn a_song_with_no_artist_says_so_rather_than_going_blank() {
        let book = Book::paginate(
            vec![section(
                "English",
                vec![
                    song(Some("Queen"), "One", 1),
                    song(None, "Two", 2),
                    song(Some("Queen"), "Three", 3),
                ],
            )],
            BookStyle::default(),
        );
        let artists: Vec<Vec<u8>> = book.pages[0]
            .rows
            .iter()
            .map(|r| r.cells[0].clone())
            .collect();
        let dash = winansi::encoded(NO_ARTIST, &mut Replacements::default());
        // And the run is broken by it: the third row names Queen again rather than blanking under
        // a dash it has nothing to do with.
        assert_eq!(artists, [b"Queen".to_vec(), dash, b"Queen".to_vec()]);
    }

    #[test]
    fn a_title_wider_than_its_column_is_cut_and_marked() {
        let long = "Faca Alguma Coisa Pelo Nosso Amor Porque Eu Ja Nao Aguento Mais Esperar";
        let book = Book::paginate(
            vec![section("English", vec![song(Some("Os Vips"), long, 1)])],
            BookStyle::default(),
        );
        let cell = &book.pages[0].rows[0].cells[2];
        assert_eq!(cell.last(), Some(&winansi::ELLIPSIS));
        assert!(metrics::text_width(cell, Font::Regular, BODY_SIZE) <= COLUMNS[2].width);
    }

    /// The widest number the catalog can hold fits the CODE column without being cut.
    ///
    /// **The one column a song number must never be ellipsized in**, because a code that reads
    /// `123456…` is not a code — a reader would dial it and get nothing, or worse, get something.
    /// Every other column degrades gracefully; this one does not degrade at all.
    ///
    /// Asserted rather than measured by hand, because the arithmetic has already changed once:
    /// `MAX_NUMBER` went from six digits to seven when `MAX_BANK` was widened to 9,999, and at
    /// 556/1000 em on a 7 pt body that took the widest code from 23.4 pt to 27.2 pt against a
    /// 36.6 pt column. It fitted, and nothing in the layout had to move — but the next widening is
    /// the one that would not, and this is what will say so.
    #[test]
    fn the_widest_song_number_fits_the_code_column() {
        let widest = km_songcode::MAX_NUMBER.to_string();
        let encoded = winansi::encoded(&widest, &mut winansi::Replacements::default());
        let width = metrics::text_width(&encoded, Font::Regular, BODY_SIZE);
        assert!(
            width <= COLUMNS[1].width,
            "the widest code {widest} needs {width} pt of a {} pt column",
            COLUMNS[1].width
        );
        // And it really is drawn whole, rather than merely fitting on paper.
        let book = Book::paginate(
            vec![section(
                "English",
                vec![song(Some("A"), "T", km_songcode::MAX_NUMBER)],
            )],
            BookStyle::default(),
        );
        let cell = &book.pages[0].rows[0].cells[1];
        assert_eq!(cell, &encoded, "the widest code was not drawn whole");
    }

    /// The measurement `km-api` asserts its catalogs against, checked in both directions.
    ///
    /// A test that can only pass is not a test, and this one guards a number no reviewer can see:
    /// `heading_overflow` returning `0.0` for everything would make
    /// `every_translated_heading_fits_its_column` green forever.
    #[test]
    fn a_heading_wider_than_its_column_is_reported_and_then_ellipsized() {
        // The English four, which are what `BookStyle::default()` draws.
        for (column, heading) in BookStyle::default().column_headings.iter().enumerate() {
            assert_eq!(
                heading_overflow(column, heading),
                0.0,
                "`{heading}` does not fit column {column}"
            );
        }
        // And the code column, which is the narrow one at 36.6 pt.
        let over = heading_overflow(1, "CODE NUMBER OF THE SONG");
        assert!(over > 0.0, "a heading twenty characters too long fits?");

        // Reported *and* handled: the book draws the ellipsis rather than the overrun.
        let style = BookStyle {
            column_headings: [
                "ARTIST".to_owned(),
                "CODE NUMBER OF THE SONG".to_owned(),
                "TITLE".to_owned(),
                "FIRST LINE".to_owned(),
            ],
            ..BookStyle::default()
        };
        let book = Book::paginate(
            vec![section("English", vec![song(Some("A"), "T", 1)])],
            style,
        );
        let drawn = &book.style.headings[1];
        assert!(
            metrics::text_width(drawn, Font::Bold, HEADING_SIZE) <= COLUMNS[1].width,
            "the heading was drawn wider than its column"
        );
    }

    #[test]
    fn an_empty_book_is_one_page_that_says_so() {
        let book = Book::paginate(Vec::new(), BookStyle::default());
        assert_eq!(book.page_count(), 1);
        assert_eq!(book.row_count(), 0);
        let pdf = book.render();
        assert!(pdf.windows(9).any(|w| w == b"/Count 1 "));
    }

    #[test]
    fn what_could_not_be_drawn_is_known_before_anything_is_drawn() {
        let book = Book::paginate(
            vec![section("Japanese", vec![song(Some("東京"), "ロマンス", 1)])],
            BookStyle::default(),
        );
        assert!(book.replaced().count > 0);
        assert!(book.replaced().sample.contains(&'東'));
    }

    #[test]
    fn a_real_book_renders_to_a_pdf_with_the_pages_it_counted() {
        let book = Book::paginate(
            vec![
                section("Portuguese", filler(120)),
                section("English", filler(10)),
            ],
            BookStyle::default(),
        );
        let pdf = book.render();
        assert!(pdf.starts_with(b"%PDF-1."));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let needle = format!("/Count {}", book.page_count());
        assert!(pdf.windows(needle.len()).any(|w| w == needle.as_bytes()));
    }

    /// Horizontal segments only, in the order they are drawn.
    fn horizontals(segments: &[[f32; 4]]) -> Vec<f32> {
        segments
            .iter()
            .filter(|[_, y0, _, y1]| (y0 - y1).abs() < f32::EPSILON)
            .map(|[_, y, _, _]| *y)
            .collect()
    }

    /// Vertical segments only, as `(x, top, bottom)`.
    fn verticals(segments: &[[f32; 4]]) -> Vec<(f32, f32, f32)> {
        segments
            .iter()
            .filter(|[x0, _, x1, _]| (x0 - x1).abs() < f32::EPSILON)
            .map(|[x, y0, _, y1]| (*x, y0.max(*y1), y0.min(*y1)))
            .collect()
    }

    /// The two halves of a row's height, which every horizontal rule is placed with.
    #[test]
    fn a_rows_rise_and_drop_are_its_height() {
        assert!((ROW_RISE + ROW_DROP - ROW_HEIGHT).abs() < f32::EPSILON);
        // The table's top edge must not be drawn through the subtitle's descenders.
        let top = HEADINGS_BASELINE + ROW_RISE;
        assert!(
            top < SUBTITLE_BASELINE,
            "the table's top edge at {top} runs through the subtitle"
        );
        // …and the rule under the last row must clear the page number, the same clearance
        // `the_last_row_does_not_land_on_the_footer` asserts for the row's text.
        let last = FIRST_ROW_BASELINE - (SLOTS_PER_PAGE - 1) as f32 * ROW_HEIGHT;
        assert!(last - ROW_DROP > FOOTER_BASELINE + SMALL_SIZE);
    }

    /// The code is centered in its cell, as the reference book sets it; the three text columns are
    /// ranged left. `anchor` is what `render` and `chrome` draw from, so asserting it is asserting
    /// where the ink goes.
    #[test]
    fn the_code_is_centered_in_its_column() {
        assert_eq!(COLUMNS[1].align, Align::Center);
        for index in [0, 2, 3] {
            assert_eq!(COLUMNS[index].align, Align::Left);
            assert!((COLUMNS[index].anchor() - COLUMNS[index].x).abs() < f32::EPSILON);
        }
        let code = &COLUMNS[1];
        assert!((code.anchor() - (code.x + code.width / 2.0)).abs() < f32::EPSILON);
    }

    /// Every row is boxed, and the table **stops at the last one**.
    ///
    /// That second half is measured from the reference rather than chosen: its final page draws 44
    /// horizontals and ends at the last song, exactly as a table in a word processor would, instead
    /// of ruling empty cells down to the bottom margin.
    #[test]
    fn the_grid_stops_at_the_last_row_on_the_page() {
        let rules = horizontals(&grid_segments(3));
        // The table's top, the line under the column headings, and then one under each of the three
        // rows.
        assert_eq!(rules.len(), 2 + 3);
        assert!((rules[4] - (FIRST_ROW_BASELINE - 2.0 * ROW_HEIGHT - ROW_DROP)).abs() < 0.01);
        assert!(
            rules[4] > FOOTER_BASELINE + ROW_HEIGHT,
            "nothing ruled near the footer"
        );

        // A full page rules all the way down, and no further.
        let full = horizontals(&grid_segments(SLOTS_PER_PAGE));
        assert_eq!(full.len(), 2 + SLOTS_PER_PAGE);
        assert!(full[full.len() - 1] > FOOTER_BASELINE + SMALL_SIZE);
    }

    /// Five verticals, at the margins and midway through each gutter.
    #[test]
    fn there_is_a_vertical_at_every_column_boundary() {
        let lines = verticals(&grid_segments(10));
        let mut xs: Vec<f32> = lines.iter().map(|(x, _, _)| *x).collect();
        xs.sort_by(f32::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert_eq!(xs.len(), COLUMNS.len() + 1);
        assert!((xs[0] - MARGIN).abs() < 0.01);
        assert!((xs[COLUMNS.len()] - RIGHT_EDGE).abs() < 0.01);
        for (index, column) in COLUMNS[1..].iter().enumerate() {
            let midway = COLUMNS[index].x + COLUMNS[index].width + GUTTER / 2.0;
            assert!((xs[index + 1] - midway).abs() < 0.01);
            assert!((xs[index + 1] - (column.x - GUTTER / 2.0)).abs() < 0.01);
        }
    }

    /// Every page is ruled identically, because no page carries a band.
    ///
    /// A section heading on the page it starts cuts the interior verticals in two, so that the
    /// heading reads as one merged cell rather than four empty ones with a word in the first. With
    /// the heading in the masthead there is nothing to cut round, and a grid that is a function of
    /// the row count alone is the shape that says so.
    #[test]
    fn every_page_is_ruled_the_same_way() {
        for (x, top, bottom) in verticals(&grid_segments(6)) {
            let full = (top - (HEADINGS_BASELINE + ROW_RISE)).abs() < 0.01;
            assert!(full, "the vertical at {x} starts at {top}, not at the top");
            assert!(
                bottom < HEADINGS_BASELINE,
                "and runs down past the headings"
            );
        }
        // One vertical per boundary and no more: nothing is in two pieces.
        assert_eq!(verticals(&grid_segments(6)).len(), COLUMNS.len() + 1);
    }

    /// A book with no songs still gets a boxed header row, and nothing ruled below it.
    #[test]
    fn an_empty_book_is_ruled_no_further_than_its_headings() {
        let segments = grid_segments(0);
        assert_eq!(horizontals(&segments).len(), 2);
        let floor = HEADINGS_BASELINE - ROW_DROP;
        for (_, _, bottom) in verticals(&segments) {
            assert!((bottom - floor).abs() < 0.01);
        }
    }

    /// The masthead is a second string, **beside** the centered title rather than instead of it: one
    /// names the machine the book belongs to and the other names the document.
    #[test]
    fn the_masthead_says_whose_book_it_is() {
        let style = BookStyle::default();
        assert_eq!(style.name, "KaraokeMachine");
        assert_eq!(style.title, "SONG LIST");
        let pdf = Book::paginate(vec![section("English", filler(2))], style).render();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("(KaraokeMachine)"), "the masthead is drawn");
        assert!(text.contains("(SONG LIST)"), "and so is the title");
    }

    /// The section is drawn once on every page, and not once in the whole book.
    ///
    /// A `contains` would pass on the band this replaced, so this counts: three pages of Portuguese
    /// means the word appears three times in the content streams.
    #[test]
    fn the_section_is_drawn_on_every_page() {
        let pdf = Book::paginate(
            vec![section("Portuguese", filler(105))],
            BookStyle::default(),
        )
        .render();
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(
            text.matches("(Portuguese)").count(),
            3,
            "the section should be drawn on each of the three pages"
        );
    }

    /// A masthead outside cp1252 is counted like any other text, so a caller can report it before
    /// the file is written — the same promise `Replacements` makes about titles and lyrics.
    #[test]
    fn a_masthead_that_cannot_be_drawn_is_counted() {
        let book = Book::paginate(
            vec![section("English", filler(1))],
            BookStyle {
                name: "カラオケ".to_owned(),
                ..BookStyle::default()
            },
        );
        assert_eq!(book.replaced().count, 4);
    }
}
