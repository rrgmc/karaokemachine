//! Deciding what character set a file's lyrics are in.
//!
//! Karaoke MIDI files carry no encoding declaration, and real-world ones are routinely Shift-JIS,
//! CP949, CP874, GB18030 or CP1252. Guessing per event gives inconsistent results within one song,
//! so the decision is made **once per file** from every lyric byte it contains, then applied
//! uniformly.
//!
//! Rendering non-Latin scripts is deferred (see `docs/ARCHITECTURE.md`), but decoding them correctly
//! is nearly free and keeps the stored data right for when rendering catches up.

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};
use serde::Serialize;

/// How the encoding for a file was arrived at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingSource {
    /// Taken from the package manifest's `lyric_encoding` field, which always wins.
    Declared,
    /// The bytes are valid UTF-8.
    Utf8,
    /// Statistically detected from the file's own lyric bytes.
    Detected,
    /// Nothing else applied; CP1252 as the historical lowest common denominator.
    Fallback,
}

/// Whether an encoding label names something that can actually be decoded with.
///
/// **This exists so packaging can refuse what playback has to tolerate**, and the asymmetry is
/// deliberate. [`TextDecoder::resolve`] falls through to ordinary detection when a declared label is
/// unrecognised — it must, because refusing to play a song over a manifest typo would be the worse
/// failure, and there is a test pinning it. But that leniency also meant a packager who wrote
/// `cp-1252` or `Shift-JIS-2004` got detection's guess, silently: no problem reported by
/// `km-pack check`, no warning in the builder, nothing in the log. The symptom was mojibake on
/// exactly the songs the field had been set to fix.
///
/// The whole label vocabulary is the WHATWG Encoding Standard's, which is what `encoding_rs`
/// implements — so `windows-1252`, `cp1252` and `Windows-1252` are all the same known label, and
/// `cp-1252` is not a label at all.
#[must_use]
pub fn is_known_label(label: &str) -> bool {
    Encoding::for_label(label.as_bytes()).is_some()
}

/// A decided encoding, applied to every text event in one file.
#[derive(Debug, Clone, Copy)]
pub struct TextDecoder {
    encoding: &'static Encoding,
    source: EncodingSource,
}

impl TextDecoder {
    /// Resolves the encoding for a file.
    ///
    /// `samples` should be every lyric or text payload in the file — the more bytes, the better
    /// detection behaves. `declared` is an encoding label (as in `Shift_JIS`, `windows-1252`,
    /// `windows-949`) from the package manifest, and takes precedence when it names a known encoding.
    ///
    /// **The label vocabulary is the WHATWG Encoding Standard's, not the platform's codepage names**,
    /// and the difference bites: `cp1252` is a label and `cp-1252` is not, `windows-949` and
    /// `euc-kr` are labels and `cp949` -- the spelling this very line used to give as an example --
    /// is not one at all. An unrecognised label falls through to detection here, deliberately; it is
    /// `Manifest::problems` that refuses to let one ship, and `encoding::is_known_label` is the test
    /// both of them use.
    pub fn resolve(samples: &[&[u8]], declared: Option<&str>) -> Self {
        Self::resolve_for_domain(samples, declared, None)
    }

    /// [`Self::resolve`], told which country's text the bytes most likely are.
    ///
    /// `domain` is a top-level domain such as `pt`, which is the hint `chardetng` takes. **A short
    /// file is where it matters**: a few accented letters in otherwise ASCII words read as well in
    /// windows-1250 as in windows-1252, and Portuguese `ê` becomes Polish `ę`. A file that names its
    /// language says which of the two it is.
    pub fn resolve_for_domain(
        samples: &[&[u8]],
        declared: Option<&str>,
        domain: Option<&[u8]>,
    ) -> Self {
        if let Some(label) = declared
            && let Some(encoding) = Encoding::for_label(label.as_bytes())
        {
            return Self {
                encoding,
                source: EncodingSource::Declared,
            };
        }

        let total: usize = samples.iter().map(|s| s.len()).sum();
        if total == 0 {
            return Self {
                encoding: UTF_8,
                source: EncodingSource::Utf8,
            };
        }

        let mut joined = Vec::with_capacity(total);
        for sample in samples {
            joined.extend_from_slice(sample);
        }

        // Pure ASCII is valid in every encoding we would consider, so calling it UTF-8 is both
        // correct and stable -- detection on ASCII-only input is a coin flip worth avoiding.
        if std::str::from_utf8(&joined).is_ok() {
            return Self {
                encoding: UTF_8,
                source: EncodingSource::Utf8,
            };
        }

        // ISO-2022-JP is allowed: these are local files, not web content that could run scripts,
        // and it is a real encoding for Japanese karaoke files. UTF-8 is denied because the check
        // above already ruled it out, so letting the detector guess it would only produce mojibake.
        let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
        detector.feed(&joined, true);
        let encoding = detector.guess(domain, chardetng::Utf8Detection::Deny);
        // chardetng falls back to windows-1252 itself; distinguish that from a positive guess so
        // the tooling can report how much to trust the text.
        let source = if encoding == WINDOWS_1252 {
            EncodingSource::Fallback
        } else {
            EncodingSource::Detected
        };
        Self { encoding, source }
    }

    /// A decoder that trusts the bytes to be UTF-8, for tests and synthetic input.
    pub fn utf8() -> Self {
        Self {
            encoding: UTF_8,
            source: EncodingSource::Utf8,
        }
    }

    /// The encoding's canonical name, suitable for storing in a package manifest.
    pub fn name(&self) -> &'static str {
        self.encoding.name()
    }

    /// How this encoding was chosen.
    pub fn source(&self) -> EncodingSource {
        self.source
    }

    /// Decodes one payload, replacing malformed sequences rather than failing.
    ///
    /// A lyric that renders imperfectly is far better than a song that refuses to load.
    pub fn decode(&self, bytes: &[u8]) -> String {
        let (text, _, _) = self.encoding.decode(bytes);
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_treated_as_utf8_without_detection() {
        let decoder = TextDecoder::resolve(&[b"Hello ", b"world"], None);
        assert_eq!(decoder.source(), EncodingSource::Utf8);
        assert_eq!(decoder.decode(b"Hello"), "Hello");
    }

    #[test]
    fn valid_utf8_is_preferred_over_detection() {
        let text = "Sei que vou te amar por toda a minha vida".as_bytes();
        let decoder = TextDecoder::resolve(&[text], None);
        assert_eq!(decoder.source(), EncodingSource::Utf8);
        assert_eq!(
            decoder.decode(text),
            "Sei que vou te amar por toda a minha vida"
        );
    }

    #[test]
    fn a_declared_encoding_wins_even_when_the_bytes_look_like_something_else() {
        // Valid UTF-8 bytes, but the manifest says CP1252, so they decode as CP1252 would.
        let bytes = "cção".as_bytes();
        let decoder = TextDecoder::resolve(&[bytes], Some("windows-1252"));
        assert_eq!(decoder.source(), EncodingSource::Declared);
        assert_eq!(decoder.name(), "windows-1252");
        assert_ne!(decoder.decode(bytes), "cção");
    }

    #[test]
    fn an_unknown_declared_label_falls_through_to_the_normal_order() {
        let decoder = TextDecoder::resolve(&[b"plain"], Some("not-an-encoding"));
        assert_eq!(decoder.source(), EncodingSource::Utf8);
    }

    /// The label vocabulary is the standard's, and the near misses are the point.
    ///
    /// `is_known_label` is what lets `Manifest::problems` refuse a label the test above deliberately
    /// tolerates. The pairs below are what a person actually types: a platform codepage name, or a
    /// hyphen where the standard has none. **`cp949` is in here because this crate's own doc used to
    /// offer it as an example of a valid label**, and it is not one.
    #[test]
    fn a_platform_codepage_name_is_not_always_a_standard_label() {
        for known in [
            "windows-1252",
            "cp1252",
            "Windows-1252",
            "Shift_JIS",
            "euc-kr",
            "windows-949",
        ] {
            assert!(is_known_label(known), "{known} should be a known label");
        }
        for unknown in ["cp-1252", "cp949", "Shift-JIS-2004", "not-an-encoding", ""] {
            assert!(!is_known_label(unknown), "{unknown} should not be a label");
        }
    }

    #[test]
    fn legacy_single_byte_text_decodes_via_the_fallback() {
        // 0xE7 0xE3 is "çã" in CP1252 and invalid UTF-8.
        let bytes = &[b'c', 0xE7, 0xE3, b'o'];
        let decoder = TextDecoder::resolve(&[bytes], Some("windows-1252"));
        assert_eq!(decoder.decode(bytes), "cção");
    }

    #[test]
    fn malformed_bytes_are_replaced_rather_than_failing() {
        let decoder = TextDecoder::utf8();
        let decoded = decoder.decode(&[b'a', 0xFF, b'b']);
        assert!(decoded.starts_with('a') && decoded.ends_with('b'));
    }

    #[test]
    fn empty_input_does_not_panic() {
        let decoder = TextDecoder::resolve(&[], None);
        assert_eq!(decoder.decode(b""), "");
    }

    // --- the detection branch --------------------------------------------------------------------
    //
    // Everything above this line stops before `chardetng` is reached: a declared label, empty input
    // and valid UTF-8 all return earlier, and CP1252 is what the detector answers when it has
    // nothing. So these are the only tests here that exercise a *positive* guess, which is the
    // decision the manifest's `lyric_encoding` override exists to correct.
    //
    // The bytes are written out rather than produced by encoding them, so what is being tested is
    // the decision and not a round trip through `encoding_rs` and back. Each was taken from the
    // encoder once and the decoded string is asserted, so a wrong byte fails rather than passing
    // quietly.

    /// `これは にほんご の もじ を よむ ため の みじかい ぶんしょう です`, in Shift-JIS.
    ///
    /// Split the way a file's lyric events are, because that is how [`TextDecoder::resolve`] is fed.
    const JAPANESE_SHIFT_JIS: [&[u8]; 3] = [
        &[0x82, 0xb1, 0x82, 0xea, 0x82, 0xcd, 0x20],
        &[
            0x82, 0xc9, 0x82, 0xd9, 0x82, 0xf1, 0x82, 0xb2, 0x20, 0x82, 0xcc, 0x20, 0x82, 0xe0,
            0x82, 0xb6, 0x20, 0x82, 0xf0, 0x20, 0x82, 0xe6, 0x82, 0xde, 0x20,
        ],
        &[
            0x82, 0xbd, 0x82, 0xdf, 0x20, 0x82, 0xcc, 0x20, 0x82, 0xdd, 0x82, 0xb6, 0x82, 0xa9,
            0x82, 0xa2, 0x20, 0x82, 0xd4, 0x82, 0xf1, 0x82, 0xb5, 0x82, 0xe5, 0x82, 0xa4, 0x20,
            0x82, 0xc5, 0x82, 0xb7,
        ],
    ];

    /// `příliš žluťoučký kůň úpěl ďábelské ódy`, in CP1250.
    ///
    /// The Czech pangram, which is here for the property a pangram has: it carries every diacritic
    /// the code page adds, so the sample is Central European by more than one byte.
    const CZECH_CP1250: [&[u8]; 3] = [
        &[0x70, 0xf8, 0xed, 0x6c, 0x69, 0x9a, 0x20],
        &[
            0x9e, 0x6c, 0x75, 0x9d, 0x6f, 0x75, 0xe8, 0x6b, 0xfd, 0x20, 0x6b, 0xf9, 0xf2, 0x20,
        ],
        &[
            0xfa, 0x70, 0xec, 0x6c, 0x20, 0xef, 0xe1, 0x62, 0x65, 0x6c, 0x73, 0x6b, 0xe9, 0x20,
            0xf3, 0x64, 0x79,
        ],
    ];

    fn decode_all(decoder: &TextDecoder, samples: &[&[u8]]) -> String {
        samples.iter().map(|s| decoder.decode(s)).collect()
    }

    #[test]
    fn japanese_lyrics_are_detected_as_shift_jis() {
        let decoder = TextDecoder::resolve(&JAPANESE_SHIFT_JIS, None);
        assert_eq!(decoder.name(), "Shift_JIS");
        assert_eq!(
            decoder.source(),
            EncodingSource::Detected,
            "a positive guess, not the CP1252 fallback"
        );
        assert_eq!(
            decode_all(&decoder, &JAPANESE_SHIFT_JIS),
            "これは にほんご の もじ を よむ ため の みじかい ぶんしょう です"
        );
    }

    #[test]
    fn central_european_lyrics_are_detected_as_windows_1250() {
        // The bytes are legal CP1252 as well -- they would read as `pøíli` and `žluouèký` -- so
        // nothing structural forces this answer and the statistics have to carry it. That is why
        // the sample is a pangram rather than one accented word.
        let decoder = TextDecoder::resolve(&CZECH_CP1250, None);
        assert_eq!(decoder.name(), "windows-1250");
        assert_eq!(decoder.source(), EncodingSource::Detected);
        assert_eq!(
            decode_all(&decoder, &CZECH_CP1250),
            "příliš žluťoučký kůň úpěl ďábelské ódy"
        );
    }

    #[test]
    fn ascii_track_names_do_not_dilute_the_decision() {
        // A real file hands detection every text-ish meta payload it holds, and most of them are
        // ASCII track names in English. They must not drown the lyrics that carry the evidence.
        let mut samples: Vec<&[u8]> = vec![b"Soft Karaoke", b"Words", b"Melody", b"Bass", b"Drums"];
        samples.extend_from_slice(&JAPANESE_SHIFT_JIS);
        let decoder = TextDecoder::resolve(&samples, None);
        assert_eq!(decoder.name(), "Shift_JIS");
        assert_eq!(decoder.source(), EncodingSource::Detected);
    }

    #[test]
    fn a_declared_label_beats_a_positive_detection() {
        // Not only the fallback: a *confident* guess is overridden too, because the packager saying
        // so is the last word. This is the path that answers a file detection reads wrongly.
        let decoder = TextDecoder::resolve(&JAPANESE_SHIFT_JIS, Some("windows-1252"));
        assert_eq!(decoder.source(), EncodingSource::Declared);
        assert_eq!(decoder.name(), "windows-1252");
        assert_ne!(
            decode_all(&decoder, &JAPANESE_SHIFT_JIS),
            "これは にほんご の もじ を よむ ため の みじかい ぶんしょう です"
        );
    }

    #[test]
    fn detection_gives_the_same_answer_every_time() {
        // Heuristics that varied between runs would be untestable, and a song whose lyrics changed
        // encoding between two openings would be worse than one that never opened.
        for samples in [&JAPANESE_SHIFT_JIS, &CZECH_CP1250] {
            let first = TextDecoder::resolve(samples, None);
            let second = TextDecoder::resolve(samples, None);
            assert_eq!(first.name(), second.name());
            assert_eq!(first.source(), second.source());
        }
    }
}
