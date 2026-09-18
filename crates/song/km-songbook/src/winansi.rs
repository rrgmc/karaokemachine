//! Turning Rust text into the bytes a base-14 PDF font can draw.
//!
//! A PDF that embeds no font can still draw text, because a reader is required to supply the metrics
//! and glyphs for the fourteen standard fonts itself. What it cannot do is draw a character those
//! fonts have no glyph for — so the price of shipping no font file is that the book speaks
//! **WinAnsiEncoding**, which is cp1252: ASCII, the Latin-1 supplement, and twenty-seven typographic
//! extras in the `0x80..=0x9F` band that Latin-1 leaves as control codes.
//!
//! That covers Portuguese, Spanish, English, Italian, French, German and the rest of Western Europe
//! — which is the corpus, and which is also exactly the scope the standing
//! `No complex-script (CJK/Thai/Arabic) text shaping` non-goal already draws. What it does not cover
//! is Cyrillic, Greek, CJK, Vietnamese and the Central European letters, and this module does two
//! things about that rather than one:
//!
//! 1. **Transliterates what has an honest Latin answer** — `ā` is an `a` with a macron and prints as
//!    `a`; nobody is misled. The table is strictly what cp1252 *lacks*, which is why it is short:
//!    `á é í ó ú ç ñ ý` and their friends are all in cp1252 already and never reach it.
//! 2. **Counts what it replaces with `?`**, and hands the count and a sample back up. The cost of
//!    this bargain is said out loud in what the command line prints, rather than being discovered
//!    by somebody holding a page of question marks.
//!
//! **Nothing is drawn into the book about it**, and that is deliberate rather than missing: a
//! reader holding the PDF cannot re-encode a title, and the two commands that build one already
//! tell the person who can. `GET /songs/book.pdf` therefore reports nothing, having no terminal.

use std::collections::BTreeSet;

/// How much text a book could not draw.
///
/// Carried out of *pagination* rather than out of rendering, because pagination is where text is
/// encoded (it has to measure in order to truncate), and because the caller's report needs the
/// answer before anything is drawn.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Replacements {
    /// How many characters became `?`. Characters, not bytes: a transliteration that emits two
    /// bytes is not a loss and is not counted.
    pub count: usize,
    /// Up to [`SAMPLE_LIMIT`] of the distinct characters, so a report can show what was lost.
    pub sample: BTreeSet<char>,
}

/// How many distinct lost characters a [`Replacements`] keeps.
///
/// A cap rather than the whole set: a book of Japanese titles would otherwise accumulate thousands
/// of them to print three of.
pub const SAMPLE_LIMIT: usize = 10;

impl Replacements {
    /// Records one character that could not be drawn.
    fn lost(&mut self, ch: char) {
        self.count += 1;
        if self.sample.len() < SAMPLE_LIMIT {
            self.sample.insert(ch);
        }
    }
}

/// The WinAnsi byte for a character, if it has one.
///
/// Three cases and a table:
///
/// 1. `0x20..=0x7E` is ASCII and maps to itself.
/// 2. `0xA0..=0xFF` is the Latin-1 supplement, which cp1252 leaves alone, so it maps to itself too.
/// 3. `0x80..=0x9F` is where the two differ: Latin-1 has control codes there and cp1252 has the
///    typographic characters below.
///
/// Everything else is `None`, and [`encode`] decides what to do about it.
#[must_use]
pub fn byte_of(ch: char) -> Option<u8> {
    let code = ch as u32;
    if (0x20..=0x7E).contains(&code) || (0xA0..=0xFF).contains(&code) {
        // `u8` because the range is checked; the cast cannot truncate.
        return Some(code as u8);
    }
    Some(match ch {
        '\u{20AC}' => 0x80, // euro
        '\u{201A}' => 0x82, // single low-9 quotation
        '\u{0192}' => 0x83, // florin
        '\u{201E}' => 0x84, // double low-9 quotation
        '\u{2026}' => 0x85, // horizontal ellipsis — the truncation marker, see `metrics`
        '\u{2020}' => 0x86, // dagger
        '\u{2021}' => 0x87, // double dagger
        '\u{02C6}' => 0x88, // modifier circumflex
        '\u{2030}' => 0x89, // per mille
        '\u{0160}' => 0x8A, // S caron
        '\u{2039}' => 0x8B, // single left angle quotation
        '\u{0152}' => 0x8C, // OE ligature
        '\u{017D}' => 0x8E, // Z caron
        '\u{2018}' => 0x91, // left single quotation
        '\u{2019}' => 0x92, // right single quotation — the apostrophe a word processor produces
        '\u{201C}' => 0x93, // left double quotation
        '\u{201D}' => 0x94, // right double quotation
        '\u{2022}' => 0x95, // bullet
        '\u{2013}' => 0x96, // en dash
        '\u{2014}' => 0x97, // em dash
        '\u{02DC}' => 0x98, // small tilde
        '\u{2122}' => 0x99, // trade mark
        '\u{0161}' => 0x9A, // s caron
        '\u{203A}' => 0x9B, // single right angle quotation
        '\u{0153}' => 0x9C, // oe ligature
        '\u{017E}' => 0x9E, // z caron
        '\u{0178}' => 0x9F, // Y diaeresis
        _ => return None,
    })
}

/// The WinAnsi byte for the ellipsis, which is the marker [`crate::metrics::fit`] truncates with.
pub const ELLIPSIS: u8 = 0x85;

/// A plain-Latin stand-in for a character cp1252 has no room for.
///
/// **Strictly the set cp1252 lacks.** `km_song::text::fold_char` already folds `á é í ó ú ç ñ ý` and
/// every one of those *is* in cp1252, so none of them arrives here — which is the whole reason this
/// table is thirty rows rather than three hundred. What is here is the Central European letters, a
/// few Baltic and Romanian ones, and the typographic strays a real title picks up from a web page.
///
/// `ß` is deliberately absent: it is `0xDF` and needs no help.
#[must_use]
pub fn transliterate(ch: char) -> Option<&'static str> {
    Some(match ch {
        'ā' | 'ă' | 'ą' | 'ǎ' => "a",
        'Ā' | 'Ă' | 'Ą' | 'Ǎ' => "A",
        'ć' | 'č' | 'ċ' => "c",
        'Ć' | 'Č' | 'Ċ' => "C",
        'ď' | 'đ' | 'ḋ' => "d",
        'Ď' | 'Đ' | 'Ḋ' => "D",
        'ē' | 'ė' | 'ę' | 'ě' => "e",
        'Ē' | 'Ė' | 'Ę' | 'Ě' => "E",
        'ğ' | 'ģ' | 'ġ' => "g",
        'Ğ' | 'Ģ' | 'Ġ' => "G",
        'ħ' => "h",
        'Ħ' => "H",
        'ī' | 'į' | 'ı' => "i",
        'Ī' | 'Į' | 'İ' => "I",
        'ķ' => "k",
        'Ķ' => "K",
        'ĺ' | 'ľ' | 'ļ' | 'ł' => "l",
        'Ĺ' | 'Ľ' | 'Ļ' | 'Ł' => "L",
        'ń' | 'ň' | 'ņ' => "n",
        'Ń' | 'Ň' | 'Ņ' => "N",
        'ō' | 'ő' => "o",
        'Ō' | 'Ő' => "O",
        'ŕ' | 'ř' => "r",
        'Ŕ' | 'Ř' => "R",
        'ś' | 'ş' | 'ș' => "s",
        'Ś' | 'Ş' | 'Ș' => "S",
        'ţ' | 'ť' | 'ț' => "t",
        'Ţ' | 'Ť' | 'Ț' => "T",
        'ũ' | 'ū' | 'ů' | 'ű' | 'ų' => "u",
        'Ũ' | 'Ū' | 'Ů' | 'Ű' | 'Ų' => "U",
        'ŵ' => "w",
        'Ŵ' => "W",
        'ŷ' => "y",
        'Ŷ' => "Y",
        'ź' | 'ż' => "z",
        'Ź' | 'Ż' => "Z",
        // Typographic strays. A title copied off a web page carries these far more often than it
        // carries a Baltic letter.
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2212}' => "-",
        '\u{2032}' => "'",
        '\u{2033}' => "\"",
        '\u{00A0}' | '\u{2006}' | '\u{2007}' | '\u{2009}' | '\u{200A}' | '\u{202F}'
        | '\u{3000}' => " ",
        '\u{2116}' => "No",
        _ => return None,
    })
}

/// Encodes text into WinAnsi bytes, transliterating what it can and counting what it cannot.
///
/// Appends rather than returning, because a row is five cells and a page is fifty-two rows, and a
/// `Vec` per cell is a `Vec` per cell.
pub fn encode(text: &str, into: &mut Vec<u8>, replaced: &mut Replacements) {
    for ch in text.chars() {
        if let Some(byte) = byte_of(ch) {
            into.push(byte);
        } else if let Some(latin) = transliterate(ch) {
            into.extend_from_slice(latin.as_bytes());
        } else {
            into.push(b'?');
            replaced.lost(ch);
        }
    }
}

/// [`encode`], for the common case of one string on its own.
#[must_use]
pub fn encoded(text: &str, replaced: &mut Replacements) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    encode(text, &mut out, replaced);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(text: &str) -> (Vec<u8>, Replacements) {
        let mut replaced = Replacements::default();
        let bytes = encoded(text, &mut replaced);
        (bytes, replaced)
    }

    #[test]
    fn ascii_survives_unchanged() {
        let (bytes, replaced) = only("Wish You Were Here (Live)");
        assert_eq!(bytes, b"Wish You Were Here (Live)");
        assert_eq!(replaced.count, 0);
    }

    /// The corpus is more than half Portuguese. If this row is wrong, the book is wrong everywhere.
    #[test]
    fn western_european_accents_are_latin_1_and_map_to_themselves() {
        let (bytes, replaced) = only("Coração à Ré");
        assert_eq!(
            bytes,
            [
                b'C', b'o', b'r', b'a', 0xE7, 0xE3, b'o', b' ', 0xE0, b' ', b'R', 0xE9
            ]
        );
        assert_eq!(replaced.count, 0);
    }

    /// A lyric line whose words are divided rather than spaced reaches the book, so a narrow space
    /// has to print as a space. A question mark per word gap would be the alternative.
    #[test]
    fn the_narrow_spaces_print_as_spaces() {
        for ch in ['\u{2006}', '\u{2009}', '\u{200A}'] {
            let (bytes, replaced) = only(&format!("can{ch}ta{ch}re{ch}mos"));
            assert_eq!(bytes, b"can ta re mos", "for {ch:?}");
            assert_eq!(replaced.count, 0, "for {ch:?}");
        }
    }

    #[test]
    fn the_cp1252_band_is_where_latin_1_and_winansi_differ() {
        for (ch, byte) in [
            ('€', 0x80u8),
            ('…', 0x85),
            ('‘', 0x91),
            ('’', 0x92),
            ('“', 0x93),
            ('”', 0x94),
            ('–', 0x96),
            ('—', 0x97),
            ('™', 0x99),
        ] {
            assert_eq!(byte_of(ch), Some(byte), "{ch:?}");
        }
    }

    #[test]
    fn the_ellipsis_constant_is_the_character_it_claims() {
        assert_eq!(byte_of('…'), Some(ELLIPSIS));
    }

    /// `ó` is Latin-1 and stays `0xF3`; everything around it is a letter cp1252 has no room for.
    #[test]
    fn what_cp1252_lacks_is_transliterated_rather_than_lost() {
        let (bytes, replaced) = only("Ārstniecība řeka Łódź");
        let mut expected = b"Arstnieciba reka L".to_vec();
        expected.push(0xF3);
        expected.extend_from_slice(b"dz");
        assert_eq!(bytes, expected);
        assert_eq!(replaced.count, 0);
    }

    #[test]
    fn a_sharp_s_is_in_cp1252_and_is_never_transliterated() {
        assert_eq!(byte_of('ß'), Some(0xDF));
        assert_eq!(transliterate('ß'), None);
    }

    #[test]
    fn a_script_no_base_14_font_has_becomes_a_question_mark_and_is_counted() {
        let (bytes, replaced) = only("東京 ロマンス");
        assert!(bytes.iter().all(|byte| *byte == b'?' || *byte == b' '));
        // Six characters; the space between them is ASCII and costs nothing.
        assert_eq!(replaced.count, 6);
        assert!(replaced.sample.contains(&'東'));
    }

    /// The count is of *characters replaced*, not of bytes emitted — a transliteration that grows
    /// the string is not a loss.
    #[test]
    fn a_transliteration_that_grows_the_string_is_not_counted_as_a_loss() {
        let (bytes, replaced) = only("№5");
        assert_eq!(bytes, b"No5");
        assert_eq!(replaced.count, 0);
    }

    #[test]
    fn the_sample_is_capped_but_the_count_is_not() {
        let mut replaced = Replacements::default();
        let _ = encoded("あいうえおかきくけこさしすせそ", &mut replaced);
        assert_eq!(replaced.count, 15);
        assert_eq!(replaced.sample.len(), SAMPLE_LIMIT);
    }
}
