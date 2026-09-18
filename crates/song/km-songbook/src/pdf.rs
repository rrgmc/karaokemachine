//! A PDF writer, as small as a document made of text and rules can be.
//!
//! This exists instead of a dependency for the reason `km-cdg` renders CD+G itself: what is actually
//! needed here is a fraction of what a PDF library does, and the fraction is small enough to read in
//! one sitting. A song book is pages of positioned text and a few horizontal rules; it embeds no
//! image, no font, no transparency, no annotation and no form.
//!
//! **The base-14 fonts are what make it small.** A PDF reader is required to supply the glyphs and
//! metrics for `/Helvetica` and its thirteen siblings, so a font object here is four keys and no
//! stream — no `/FontDescriptor`, no `/Widths`, no `/FontFile`, and above all no font parsing, no
//! glyph subsetting and no CID machinery, which is where the bulk of any PDF library lives. The
//! price is that text is WinAnsi; see [`crate::winansi`], which says so out loud rather than hiding
//! it.
//!
//! # The shape of the file
//!
//! ```text
//! %PDF-1.4
//! %<four bytes >= 0x80>          a binary marker, so nothing treats the file as text
//! 1 0 obj  << /Type /Catalog /Pages 2 0 R >>
//! 2 0 obj  << /Type /Pages /Kids [...] /Count N >>
//! 3 0 obj  << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>
//! 4 0 obj  ... the same, Helvetica-Bold
//! 5 0 obj  << /Title (...) /Producer (...) >>
//! 6 0 obj  << /Type /Page ... /Contents 7 0 R >>      one pair per page,
//! 7 0 obj  << /Length L >> stream ... endstream       the page and its operators
//! xref
//! trailer  << /Size N /Root 1 0 R /Info 5 0 R >>
//! startxref
//! %%EOF
//! ```
//!
//! # Streams are not compressed, and that is a decision
//!
//! `/Filter /FlateDecode` would cut the file to roughly a sixth. It is not used because `flate2`
//! reaches this workspace only through `zip`'s dependency tree — **no member names it directly** —
//! so compressing here would mean a new `[workspace.dependencies]` entry, and this crate's whole
//! claim is that it needs none. Measured, an uncompressed book runs about 235 bytes a row: roughly
//! 3 MB for twelve thousand songs, against the 6.7 MB of the commercial book this one is modeled
//! on. If that ever stops being affordable the change is one dict entry and one call.

use std::fmt::Write as _;

use crate::metrics::Font;

/// The page size every page uses: A4 portrait, in points.
///
/// `595.32 x 841.92` rather than the strict `595.276 x 841.890`. These are the numbers the
/// commercial book this one is modeled on carries, they are what Word rounds A4 to, and matching a
/// real machine's book to four hundredths of a point costs nothing.
pub const PAGE_WIDTH: f32 = 595.32;
/// The page height in points. See [`PAGE_WIDTH`].
pub const PAGE_HEIGHT: f32 = 841.92;

/// Object 1. Fixed, so the trailer can name it without looking it up.
const CATALOG: usize = 1;
/// Object 2, the page tree.
const PAGES: usize = 2;
/// Object 3, `/Helvetica`.
const FONT_REGULAR: usize = 3;
/// Object 4, `/Helvetica-Bold`.
const FONT_BOLD: usize = 4;
/// Object 5, the document information dictionary.
const INFO: usize = 5;
/// The first object a page may use. Everything below this is fixed.
const FIRST_PAGE_OBJECT: usize = 6;

/// One page's operators, built up before anything is written to the file.
///
/// Separate from [`Writer`] because a content stream needs its own length before it can be wrapped
/// in a dictionary, and because pagination wants to build a page without a document to put it in.
#[derive(Debug, Default)]
pub struct Content {
    ops: String,
    /// The face the last `Tf` selected, so a page of one face emits one `Tf`.
    face: Option<(Font, f32)>,
}

impl Content {
    /// A new, empty page.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws already-encoded WinAnsi text with its left edge at `x` and its baseline at `y`.
    pub fn text(&mut self, bytes: &[u8], font: Font, size: f32, x: f32, y: f32) {
        if bytes.is_empty() {
            return;
        }
        self.ops.push_str("BT\n");
        if self.face != Some((font, size)) {
            let _ = writeln!(self.ops, "/{} {} Tf", font.resource(), number(size));
            self.face = Some((font, size));
        }
        let _ = write!(self.ops, "1 0 0 1 {} {} Tm ", number(x), number(y));
        escape_into(bytes, &mut self.ops);
        self.ops.push_str(" Tj\nET\n");
    }

    /// The same, with the text's *right* edge at `x`. What the CODE column is set with.
    pub fn text_right(&mut self, bytes: &[u8], font: Font, size: f32, x: f32, y: f32) {
        let width = crate::metrics::text_width(bytes, font, size);
        self.text(bytes, font, size, x - width, y);
    }

    /// The same, centered on `x`. The document title and the page number.
    pub fn text_centered(&mut self, bytes: &[u8], font: Font, size: f32, x: f32, y: f32) {
        let width = crate::metrics::text_width(bytes, font, size);
        self.text(bytes, font, size, x - width / 2.0, y);
    }

    /// A batch of straight segments, stroked as one path. `gray` is 0.0 black to 1.0 white.
    ///
    /// **One path, not one per segment**, and that is about file size rather than tidiness: a page
    /// of the table's grid is around fifty-eight segments, and bracketing each one in its own
    /// `q`/`Q` costs about five times the bytes. These streams are deliberately uncompressed, so
    /// that difference is half a megabyte on a book the size of the reference's.
    pub fn lines(&mut self, weight: f32, gray: f32, segments: &[[f32; 4]]) {
        if segments.is_empty() {
            return;
        }
        // Color and line width are graphics state, not text state, so they sit outside any BT/ET.
        // `q`/`Q` brackets them so a path cannot leak its weight into the next one.
        let _ = write!(self.ops, "q {} w {} G", number(weight), number(gray));
        for [x0, y0, x1, y1] in segments {
            let _ = write!(
                self.ops,
                " {} {} m {} {} l",
                number(*x0),
                number(*y0),
                number(*x1),
                number(*y1)
            );
        }
        self.ops.push_str(" S Q\n");
        // A `q`/`Q` pair restores the text state too, so the next `Tf` must be written again.
        self.face = None;
    }

    /// A horizontal rule. `gray` is 0.0 black to 1.0 white.
    pub fn rule(&mut self, x0: f32, x1: f32, y: f32, weight: f32, gray: f32) {
        self.lines(weight, gray, &[[x0, y, x1, y]]);
    }

    /// Sets the fill color for subsequent text. `gray` is 0.0 black to 1.0 white.
    pub fn gray(&mut self, gray: f32) {
        let _ = writeln!(self.ops, "{} g", number(gray));
    }

    /// The operators, as the bytes that go in the stream.
    fn into_bytes(self) -> Vec<u8> {
        self.ops.into_bytes()
    }
}

/// Builds a document out of pages.
#[derive(Debug)]
pub struct Writer {
    out: Vec<u8>,
    /// Byte offset of each object, indexed by object number. Entry 0 is the free-list head and is
    /// never a real offset.
    offsets: Vec<usize>,
    pages: Vec<usize>,
    title: String,
}

impl Writer {
    /// Starts a document. `title` reaches the reader's title bar through `/Info`.
    #[must_use]
    pub fn new(title: &str) -> Self {
        let mut out = Vec::with_capacity(64 * 1024);
        out.extend_from_slice(b"%PDF-1.4\n");
        // The binary marker. Four bytes above 0x7F tell anything inspecting the first line that this
        // is not a text file — the convention every real producer follows.
        out.extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
        Self {
            out,
            // Index 0 is the free head; 1..=5 are the fixed objects, filled in by `finish`.
            offsets: vec![0; FIRST_PAGE_OBJECT],
            pages: Vec::new(),
            title: title.to_owned(),
        }
    }

    /// Appends one page.
    pub fn page(&mut self, content: Content) {
        let page = self.next_object();
        let stream = self.next_object();
        self.pages.push(page);

        self.begin(page);
        let _ = write!(
            self.body(),
            "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {} {}] \
             /Resources << /Font << /F1 {FONT_REGULAR} 0 R /F2 {FONT_BOLD} 0 R >> >> \
             /Contents {stream} 0 R >>",
            number(PAGE_WIDTH),
            number(PAGE_HEIGHT)
        );
        self.end();

        let body = content.into_bytes();
        self.begin(stream);
        // A **direct** `/Length`, which is why the body is built first: the indirect form the
        // specification also allows would cost a third object per page and buy nothing.
        let _ = write!(self.body(), "<< /Length {} >>\nstream\n", body.len());
        self.out.extend_from_slice(&body);
        self.out.extend_from_slice(b"\nendstream");
        self.end();
    }

    /// Writes the fixed objects, the cross-reference table and the trailer, and hands over the file.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        self.write_fixed_objects();

        // Captured *before* the keyword is appended, which is the whole contract of `startxref`.
        let xref_at = self.out.len();
        let count = self.offsets.len();
        let _ = write!(self.body(), "xref\n0 {count}\n");
        // Every record is exactly twenty bytes, including the trailing space before the newline.
        // Readers are entitled to seek by multiplying, so a record of nineteen or twenty-one breaks
        // files that open perfectly well in a forgiving viewer.
        self.out.extend_from_slice(b"0000000000 65535 f \n");
        let mut records = String::with_capacity(count * 20);
        for offset in self.offsets.iter().skip(1) {
            let _ = writeln!(records, "{offset:010} 00000 n ");
        }
        self.out.extend_from_slice(records.as_bytes());
        let _ = write!(
            self.body(),
            "trailer\n<< /Size {count} /Root {CATALOG} 0 R /Info {INFO} 0 R >>\n\
             startxref\n{xref_at}\n%%EOF\n"
        );
        self.out
    }

    /// The five objects whose numbers are fixed, written once the page list is known.
    fn write_fixed_objects(&mut self) {
        self.begin(CATALOG);
        let _ = write!(self.body(), "<< /Type /Catalog /Pages {PAGES} 0 R >>");
        self.end();

        self.begin(PAGES);
        self.out.extend_from_slice(b"<< /Type /Pages /Kids [");
        let kids: Vec<usize> = self.pages.clone();
        for (index, page) in kids.iter().enumerate() {
            if index > 0 {
                self.out.push(b' ');
            }
            let _ = write!(self.body(), "{page} 0 R");
        }
        let _ = write!(self.body(), "] /Count {} >>", kids.len());
        self.end();

        for (id, font) in [(FONT_REGULAR, Font::Regular), (FONT_BOLD, Font::Bold)] {
            self.begin(id);
            // Four keys and no stream. See the module header.
            let _ = write!(
                self.body(),
                "<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
                font.base_font()
            );
            self.end();
        }

        self.begin(INFO);
        self.out.extend_from_slice(b"<< /Title ");
        let title = self.title.clone();
        let mut escaped = String::new();
        // The title comes from a caller and may hold anything a song title may hold.
        escape_into(
            &crate::winansi::encoded(&title, &mut Default::default()),
            &mut escaped,
        );
        self.out.extend_from_slice(escaped.as_bytes());
        let _ = write!(
            self.body(),
            " /Producer (KaraokeMachine {}) /Creator (km-songbook) >>",
            env!("CARGO_PKG_VERSION")
        );
        self.end();
    }

    /// Reserves the next object number.
    ///
    /// The fixed five are reserved by `Writer::new`, so this only ever hands out page objects.
    fn next_object(&mut self) -> usize {
        let id = self.offsets.len();
        self.offsets.push(0);
        id
    }

    /// Records where an object starts and writes its header.
    ///
    /// **The offset is taken here and never revised.** Nothing patches the file afterwards, which
    /// removes the whole class of fault where an offset was correct at the moment it was recorded.
    fn begin(&mut self, id: usize) {
        self.offsets[id] = self.out.len();
        let _ = writeln!(self.body(), "{id} 0 obj");
    }

    /// Closes an object.
    fn end(&mut self) {
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// `write!` target. `Vec<u8>` implements `io::Write` and not `fmt::Write`, and every format
    /// string here is ASCII, so this adapter keeps the call sites readable.
    fn body(&mut self) -> Body<'_> {
        Body(&mut self.out)
    }
}

/// A `fmt::Write` view of the output buffer. See [`Writer::body`].
struct Body<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Body<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

/// Formats a coordinate with at most two decimals and no trailing zeros.
///
/// `28` rather than `28.00`. Worth a function because a page carries several hundred of these and
/// the saving over a whole book is around a seventh of the file.
fn number(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    let mut text = format!("{rounded:.2}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

/// Writes bytes as a PDF literal string, parentheses included.
///
/// Three characters must be escaped inside `( )` — `\`, `(` and `)` — and **everything outside
/// printable ASCII is escaped as three-digit octal as well**. That is one rule instead of five, it
/// is always correct, and it keeps every content stream 7-bit so a test can grep one. It also
/// settles `\r`, which some readers normalize inside a stream and which would otherwise corrupt a
/// title silently.
fn escape_into(bytes: &[u8], out: &mut String) {
    out.push('(');
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            0x20..=0x7E => out.push(char::from(*byte)),
            other => {
                let _ = write!(out, "\\{other:03o}");
            }
        }
    }
    out.push(')');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-page document with something on each page, for the structural tests to pick over.
    fn sample() -> Vec<u8> {
        let mut writer = Writer::new("Wish You Were Here (Live) \\ 50%");
        for page in 0..2 {
            let mut content = Content::new();
            content.text(b"Legiao Urbana", Font::Regular, 7.0, 28.0, 700.0);
            content.text_right(b"500", Font::Regular, 7.0, 225.6, 700.0);
            content.text_centered(format!("{page}").as_bytes(), Font::Bold, 7.0, 297.66, 30.0);
            content.rule(28.0, 567.3, 778.0, 0.5, 0.6);
            writer.page(content);
        }
        writer.finish()
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    fn count(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    #[test]
    fn a_document_has_every_part_a_reader_looks_for() {
        let pdf = sample();
        assert!(pdf.starts_with(b"%PDF-1."));
        // The binary marker: a `%` and four bytes above 0x7F on the second line.
        let second = &pdf[9..14];
        assert_eq!(second[0], b'%');
        assert!(second[1..].iter().all(|byte| *byte >= 0x80));
        assert!(pdf.ends_with(b"%%EOF\n"));
        for needle in [
            &b"/Type /Catalog"[..],
            b"/Type /Pages",
            b"/Type /Page ",
            b"/BaseFont /Helvetica ",
            b"/BaseFont /Helvetica-Bold",
            b"/Encoding /WinAnsiEncoding",
            b"/Subtype /Type1",
            b"trailer",
            b"startxref",
        ] {
            assert!(
                find(&pdf, needle).is_some(),
                "missing {:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }

    /// **The assertion that proves the feature.** If any of these ever appears, a font is being
    /// embedded and the crate's one claim — that it needs no font file and no font parser — has
    /// quietly stopped being true.
    #[test]
    fn no_font_is_embedded() {
        let pdf = sample();
        for needle in [
            &b"/FontFile"[..],
            b"/FontDescriptor",
            b"/Widths",
            b"/FirstChar",
        ] {
            assert!(
                find(&pdf, needle).is_none(),
                "{} appeared: something is embedding a font",
                String::from_utf8_lossy(needle)
            );
        }
    }

    /// The structural test, and the reason one is possible without a PDF parser: this is the first
    /// thing any parser does, so getting it right is most of getting the file right.
    #[test]
    fn every_xref_offset_points_at_the_object_it_claims() {
        let pdf = sample();
        let marker = find(&pdf, b"startxref\n").expect("startxref");
        let tail = String::from_utf8_lossy(&pdf[marker + b"startxref\n".len()..]).into_owned();
        let start: usize = tail
            .lines()
            .next()
            .expect("an offset")
            .trim()
            .parse()
            .expect("a number");
        assert!(
            pdf[start..].starts_with(b"xref\n"),
            "startxref does not point at the table"
        );

        let header_at = start + b"xref\n".len();
        let header = String::from_utf8_lossy(&pdf[header_at..header_at + 16]).into_owned();
        let mut words = header.split_whitespace();
        assert_eq!(
            words.next(),
            Some("0"),
            "one subsection, starting at object 0"
        );
        let count: usize = words.next().expect("a count").parse().expect("a number");

        let records_at = pdf[header_at..]
            .iter()
            .position(|b| *b == b'\n')
            .expect("eol")
            + header_at
            + 1;
        for id in 0..count {
            // Twenty bytes each, and readers are entitled to seek by multiplying.
            let record = &pdf[records_at + id * 20..records_at + id * 20 + 20];
            assert_eq!(record[19], b'\n', "record {id} is not twenty bytes");
            let offset: usize = String::from_utf8_lossy(&record[..10])
                .parse()
                .expect("an offset");
            if id == 0 {
                assert_eq!(&record[..18], b"0000000000 65535 f");
                continue;
            }
            let expected = format!("{id} 0 obj");
            assert!(
                pdf[offset..].starts_with(expected.as_bytes()),
                "object {id} claims offset {offset}, which holds {:?}",
                String::from_utf8_lossy(&pdf[offset..offset + 20])
            );
        }
    }

    #[test]
    fn the_trailer_and_the_page_tree_agree_with_what_was_written() {
        let pdf = sample();
        // Two pages: five fixed objects plus a page and a stream each.
        assert!(find(&pdf, b"/Size 10").is_some(), "trailer /Size");
        assert!(find(&pdf, b"/Count 2").is_some(), "page tree /Count");
        // `/Contents`, not `/Type /Page` — `/Type /Pages` has that as a prefix and would be counted.
        assert_eq!(count(&pdf, b"/Contents "), 2);
    }

    #[test]
    fn every_stream_is_as_long_as_it_says_it_is() {
        let pdf = sample();
        let mut at = 0;
        let mut streams = 0;
        while let Some(found) = find(&pdf[at..], b"<< /Length ") {
            let head = at + found + b"<< /Length ".len();
            let end = pdf[head..]
                .iter()
                .position(|b| *b == b' ')
                .expect("a space")
                + head;
            let declared: usize = String::from_utf8_lossy(&pdf[head..end])
                .parse()
                .expect("a number");
            let body = find(&pdf[end..], b"stream\n").expect("a stream") + end + b"stream\n".len();
            let close = find(&pdf[body..], b"\nendstream").expect("endstream") + body;
            assert_eq!(
                close - body,
                declared,
                "a stream is not the length it declares"
            );
            streams += 1;
            at = close;
        }
        assert_eq!(streams, 2);
    }

    #[test]
    fn a_parenthesis_in_a_title_cannot_close_the_string_early() {
        let pdf = sample();
        // The `/Info` title carries `(`, `)` and `\`, all of which must arrive escaped.
        assert!(find(&pdf, br"Wish You Were Here \(Live\) \\ 50%").is_some());
    }

    #[test]
    fn text_outside_printable_ascii_is_written_as_octal() {
        let mut out = String::new();
        // `Coração` in WinAnsi: ç is 0xE7, ã is 0xE3.
        escape_into(&[b'C', b'o', b'r', b'a', 0xE7, 0xE3, b'o'], &mut out);
        assert_eq!(out, r"(Cora\347\343o)");
        assert!(out.is_ascii(), "a content stream must stay 7-bit");
    }

    #[test]
    fn coordinates_are_written_without_trailing_zeros() {
        assert_eq!(number(28.0), "28");
        assert_eq!(number(225.6), "225.6");
        assert_eq!(number(146.55), "146.55");
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.001), "0");
    }

    /// A `q`/`Q` pair restores the text state, so a rule between two rows must force the next `Tf`
    /// to be written again. Without this the second row would be drawn in whatever the reader's
    /// default font is — which is a fault that only shows on pages that happen to carry a rule.
    #[test]
    fn a_rule_makes_the_next_row_name_its_font_again() {
        let mut content = Content::new();
        content.text(b"one", Font::Regular, 7.0, 0.0, 0.0);
        content.rule(0.0, 10.0, 5.0, 0.5, 0.6);
        content.text(b"two", Font::Regular, 7.0, 0.0, 0.0);
        assert_eq!(content.ops.matches("/F1 7 Tf").count(), 2);
    }

    /// The grid is one path per page, not one per line. A page of the song book's table is around
    /// fifty-eight segments, and this is what keeps that from costing fifty-eight `q`/`Q` pairs.
    #[test]
    fn many_segments_are_stroked_as_one_path() {
        let mut content = Content::new();
        content.lines(
            0.4,
            0.35,
            &[
                [0.0, 0.0, 10.0, 0.0],
                [0.0, 0.0, 0.0, 20.0],
                [5.0, 0.0, 5.0, 20.0],
            ],
        );
        assert_eq!(content.ops.matches(" S Q").count(), 1);
        assert_eq!(content.ops.matches(" m ").count(), 3);
        assert_eq!(content.ops.matches(" l").count(), 3);
        assert_eq!(content.ops.matches("q 0.4 w").count(), 1);
    }

    /// `rule` is a one-segment `lines`, and its bytes must not have changed when it became one:
    /// every rule the book already drew is pinned here rather than left to a rendering to notice.
    #[test]
    fn a_rule_is_the_one_segment_case_and_looks_the_same() {
        let mut content = Content::new();
        content.rule(28.0, 567.3, 778.0, 0.5, 0.45);
        assert_eq!(content.ops, "q 0.5 w 0.45 G 28 778 m 567.3 778 l S Q\n");
    }

    /// Same reasoning as `a_rule_makes_the_next_row_name_its_font_again`, asserted against the
    /// method that actually emits the `q`/`Q` now that `rule` only forwards to it.
    #[test]
    fn a_path_makes_the_next_row_name_its_font_again() {
        let mut content = Content::new();
        content.text(b"one", Font::Regular, 7.0, 0.0, 0.0);
        content.lines(0.4, 0.35, &[[0.0, 0.0, 0.0, 10.0]]);
        content.text(b"two", Font::Regular, 7.0, 0.0, 0.0);
        assert_eq!(content.ops.matches("/F1 7 Tf").count(), 2);
    }

    /// Nothing to draw draws nothing — not an empty path, which some readers warn about.
    #[test]
    fn no_segments_emit_no_operators() {
        let mut content = Content::new();
        content.lines(0.4, 0.35, &[]);
        assert!(content.ops.is_empty());
    }
}
