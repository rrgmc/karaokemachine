//! How wide a string will be, without a font file to ask.
//!
//! The base-14 fonts have fixed metrics: every conforming PDF reader draws `/Helvetica` at exactly
//! the widths below, and that agreement is what lets a book embed no font at all. The catch is that
//! the *writer* still has to know them, because a column has to be truncated before it is drawn —
//! measuring is our problem even though drawing is not.
//!
//! # Where these numbers come from
//!
//! Adobe's Core 14 AFM files, `Helvetica.afm` and `Helvetica-Bold.afm`, which ship with Ghostscript
//! under `Resource/Font/` and with poppler, and which every PDF library reproduces identically.
//! Widths are in 1/1000 em, so a glyph's width in points is `w / 1000 * size`.
//!
//! **An AFM is keyed by glyph *name* against StandardEncoding, and this table is keyed by WinAnsi
//! code**, so building it was a transposition rather than a copy. That step is where a table like
//! this goes subtly wrong, so the places the two encodings disagree are written down:
//!
//! | WinAnsi | glyph | | WinAnsi | glyph |
//! |---|---|---|---|---|
//! | `0x80` | `Euro` | | `0x93` | `quotedblleft` |
//! | `0x85` | `ellipsis` | | `0x94` | `quotedblright` |
//! | `0x91` | `quoteleft` | | `0x95` | `bullet` |
//! | `0x92` | `quoteright` | | `0x97` | `emdash` |
//!
//! StandardEncoding has `quoteright` at `0x27` and `quoteleft` at `0x60`, where WinAnsi has the
//! ASCII `quotesingle` and `grave`; the widths there follow **WinAnsi**, which is what the font
//! object declares.
//!
//! Codes with no glyph in WinAnsi — the control ranges, and `0x81`, `0x8D`, `0x8F`, `0x90`, `0x9D` —
//! are zero. [`crate::winansi::byte_of`] never produces one, so a zero here means a bug there
//! rather than a character somebody typed.

/// Which of the two faces a run of text is set in.
///
/// Two, and there will not be a third: the book is a table, and a table wants one text face and one
/// face for the things that are not text. Oblique would be a fourth font object for no gain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    /// The body face — every cell of every row.
    Regular,
    /// Headings, column headings and the document title.
    Bold,
}

impl Font {
    /// The PDF resource name this face is mounted under. See [`crate::pdf`].
    #[must_use]
    pub fn resource(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
        }
    }

    /// The base-14 name that goes in the font object's `/BaseFont`.
    #[must_use]
    pub fn base_font(self) -> &'static str {
        match self {
            Font::Regular => "Helvetica",
            Font::Bold => "Helvetica-Bold",
        }
    }

    /// This face's widths, indexed by WinAnsi byte.
    #[must_use]
    pub fn widths(self) -> &'static [u16; 256] {
        match self {
            Font::Regular => &HELVETICA,
            Font::Bold => &HELVETICA_BOLD,
        }
    }
}

/// `Helvetica`, in 1/1000 em, indexed by WinAnsi code.
#[rustfmt::skip]
pub const HELVETICA: [u16; 256] = [
    // 0x00
       0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
       0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    // 0x20  space ! " # $ % & ' ( ) * + , - . /
     278, 278, 355, 556, 556, 889, 667, 222, 333, 333, 389, 584, 278, 333, 278, 278,
    // 0x30  0-9 : ; < = > ?
     556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    // 0x40  @ A-O
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    // 0x50  P-Z [ \ ] ^ _
     667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    // 0x60  grave a-o
     222, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    // 0x70  p-z { | } ~
     556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,   0,
    // 0x80  the cp1252 band
     556,   0, 222, 556, 333,1000, 556, 556, 333,1000, 667, 333,1000,   0, 611,   0,
    // 0x90
       0, 222, 222, 333, 333, 350, 556,1000, 333,1000, 500, 333, 944,   0, 500, 667,
    // 0xA0  the Latin-1 supplement, which cp1252 leaves alone
     278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    // 0xB0
     400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    // 0xC0
     667, 667, 667, 667, 667, 667,1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    // 0xD0
     722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    // 0xE0
     556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
    // 0xF0
     556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

/// `Helvetica-Bold`, in 1/1000 em, indexed by WinAnsi code.
#[rustfmt::skip]
pub const HELVETICA_BOLD: [u16; 256] = [
    // 0x00
       0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
       0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    // 0x20
     278, 333, 474, 556, 556, 889, 722, 278, 333, 333, 389, 584, 278, 333, 278, 278,
    // 0x30
     556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    // 0x40
     975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    // 0x50
     667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    // 0x60
     278, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    // 0x70
     611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,   0,
    // 0x80
     556,   0, 278, 556, 500,1000, 556, 556, 333,1000, 667, 333,1000,   0, 611,   0,
    // 0x90
       0, 278, 278, 500, 500, 350, 556,1000, 333,1000, 556, 333, 889,   0, 500, 667,
    // 0xA0
     278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    // 0xB0
     400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    // 0xC0
     722, 722, 722, 722, 722, 722,1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    // 0xD0
     722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    // 0xE0
     556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278,
    // 0xF0
     611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];

/// How wide already-encoded text will be, in points.
///
/// **Takes WinAnsi bytes, not a `&str`**, and that is the whole reason measuring and drawing cannot
/// disagree: the table is indexed by the same byte the content stream will carry, so there is no
/// second decoding step between deciding a string fits and writing it out.
#[must_use]
pub fn text_width(bytes: &[u8], font: Font, size: f32) -> f32 {
    let widths = font.widths();
    let sum: u32 = bytes
        .iter()
        .map(|byte| u32::from(widths[usize::from(*byte)]))
        .sum();
    // `as f32` on a sum of 1000-max values over a line of at most a few hundred bytes: far inside
    // f32's exact-integer range.
    sum as f32 / 1000.0 * size
}

/// Cuts text down until it fits, marking the cut with an ellipsis.
///
/// Returns the bytes to draw. A string that already fits comes back untouched — including its
/// allocation, since the common case is a title far shorter than its column.
///
/// **No word-boundary cleverness.** At 7pt in a five-column table, breaking at a space leaves a
/// ragged column that reads worse than a mid-word cut, and the reference book cuts mid-word too.
#[must_use]
pub fn fit(bytes: Vec<u8>, font: Font, size: f32, max: f32) -> Vec<u8> {
    if text_width(&bytes, font, size) <= max {
        return bytes;
    }
    let ellipsis = text_width(&[crate::winansi::ELLIPSIS], font, size);
    // A column too narrow for the marker itself has nothing to say; an empty cell beats a lone `…`.
    if ellipsis > max {
        return Vec::new();
    }
    let widths = font.widths();
    let budget = max - ellipsis;
    let mut used = 0.0;
    let mut keep = 0;
    for (index, byte) in bytes.iter().enumerate() {
        let step = f32::from(widths[usize::from(*byte)]) / 1000.0 * size;
        if used + step > budget {
            break;
        }
        used += step;
        keep = index + 1;
    }
    let mut out = bytes;
    out.truncate(keep);
    out.push(crate::winansi::ELLIPSIS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anchors from the AFM. If the transposition in the module header went wrong, these are what
    /// says so — a width table with no source is a table nobody can check.
    #[test]
    fn the_anchor_widths_are_the_ones_adobe_publishes() {
        for (ch, width) in [
            (' ', 278u16),
            ('!', 278),
            ('A', 667),
            ('M', 833),
            ('W', 944),
            ('i', 222),
            ('l', 222),
            ('m', 833),
            ('w', 722),
        ] {
            assert_eq!(HELVETICA[ch as usize], width, "Helvetica {ch:?}");
        }
        for (ch, width) in [
            (' ', 278u16),
            ('A', 722),
            ('M', 833),
            ('i', 278),
            ('m', 889),
        ] {
            assert_eq!(HELVETICA_BOLD[ch as usize], width, "Helvetica-Bold {ch:?}");
        }
    }

    /// **The invariant the CODE column rests on.** Every digit is 556 in both faces, so a
    /// right-aligned number lines up with the one above it without zero-padding and without a
    /// monospaced font. If this ever stopped being true the column would need a different design,
    /// not a different number.
    #[test]
    fn every_digit_is_the_same_width_in_both_faces() {
        for face in [&HELVETICA, &HELVETICA_BOLD] {
            for digit in b'0'..=b'9' {
                assert_eq!(face[usize::from(digit)], 556, "digit {}", digit as char);
            }
        }
    }

    /// [`fit`] budgets for the marker before it starts cutting, so this number is load-bearing.
    #[test]
    fn the_ellipsis_is_a_full_em_in_both_faces() {
        assert_eq!(HELVETICA[usize::from(crate::winansi::ELLIPSIS)], 1000);
        assert_eq!(HELVETICA_BOLD[usize::from(crate::winansi::ELLIPSIS)], 1000);
    }

    /// A zero width means [`crate::winansi::byte_of`] produced a code with no glyph, which would
    /// draw nothing and measure as nothing — a fault that hides itself.
    #[test]
    fn every_code_winansi_can_produce_has_a_width() {
        let mut checked = 0;
        for code in 0..=0x10FFFFu32 {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            let Some(byte) = crate::winansi::byte_of(ch) else {
                continue;
            };
            assert_ne!(
                HELVETICA[usize::from(byte)],
                0,
                "Helvetica {ch:?} -> {byte:#04x}"
            );
            assert_ne!(
                HELVETICA_BOLD[usize::from(byte)],
                0,
                "Bold {ch:?} -> {byte:#04x}"
            );
            checked += 1;
        }
        // Sanity that the sweep actually swept: 95 ASCII + 96 Latin-1 + 27 cp1252 specials.
        assert_eq!(checked, 95 + 96 + 27);
    }

    #[test]
    fn a_width_is_a_thousandth_of_an_em_each() {
        // MMM at 7pt: three glyphs of 833/1000 em.
        assert!((text_width(b"MMM", Font::Regular, 7.0) - 3.0 * 0.833 * 7.0).abs() < 0.001);
        assert_eq!(text_width(b"", Font::Regular, 7.0), 0.0);
    }

    #[test]
    fn text_that_already_fits_is_returned_untouched() {
        let text = b"Tempo Perdido".to_vec();
        assert_eq!(fit(text.clone(), Font::Regular, 7.0, 500.0), text);
    }

    #[test]
    fn text_that_does_not_fit_is_cut_and_marked_and_still_fits() {
        let long = b"Faca Alguma Coisa Pelo Nosso Amor Que Eu Nao Aguento Mais".to_vec();
        let cut = fit(long.clone(), Font::Regular, 7.0, 60.0);
        assert!(cut.len() < long.len());
        assert_eq!(cut.last(), Some(&crate::winansi::ELLIPSIS));
        assert!(text_width(&cut, Font::Regular, 7.0) <= 60.0);
    }

    #[test]
    fn a_column_too_narrow_for_the_marker_draws_nothing() {
        assert!(fit(b"anything".to_vec(), Font::Regular, 7.0, 1.0).is_empty());
    }
}
