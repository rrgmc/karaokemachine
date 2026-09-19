//! What language a song's own words are in, when nothing the file says answers it.
//!
//! [`km_kmpkg::Language::detect`] reads two witnesses, the Soft Karaoke `@L` header and the lyric
//! encoding, and both are silent over most of a real corpus: `windows-1252` and UTF-8 each carry a
//! dozen languages, and a file with no header says nothing at all. What is left is the text itself,
//! which this reads.
//!
//! # A guess counts, so it has to be worth counting
//!
//! What comes back here is stored as the song's language and is read by every filter, by the sort
//! and by the gate that refuses to package a song nobody has classified. So the bar is not *which
//! language is likeliest* but *is this certain enough to be treated as a fact*: below
//! [`MIN_CONFIDENCE`] the answer is `None` and the song stays honestly unclassified.
//!
//! **How much text there was is not asked separately, because the confidence already holds it.**
//! The detector scores how far the winner stood clear of the rest, so thin evidence scores low by
//! itself: a two-word English title comes back around 0.49 and a Spanish verse of 127 letters around
//! 0.485, both refused, where an English verse of 262 letters is certain. A second gate counting
//! letters would refuse the one population that needs no length — a title carrying characters only
//! one language writes, which is certain at ten letters — and would still not catch the long verse
//! in a language with three close neighbours.
//!
//! # The codes are transcribed, not chosen
//!
//! The detector answers in ISO 639-3 and a song's language is ISO 639-1, so [`ISO_639_1`] maps one
//! onto the other and does nothing else. A detector language with no two-letter code yields `None`
//! rather than an invention.

/// How many times the guess has changed its mind about the same text.
///
/// Stored by `km-package-builder` beside its guessed-language column so that a change here re-runs
/// its backfill exactly once. Bump it whenever [`ISO_639_1`], [`MIN_CONFIDENCE`] or the detector's
/// pinned version moves — a consumer cannot see a source edit, only this number.
pub const GUESS_REVISION: u32 = 1;

/// How sure the detector has to be before its answer is stored.
///
/// The detector's own line between reliable and not, which is where two languages sharing an
/// alphabet stop being told apart.
pub const MIN_CONFIDENCE: f64 = 0.9;

/// A language read out of a song's own words, and how sure the reading is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guess {
    language: km_kmpkg::Language,
    confidence: f64,
}

impl Guess {
    /// The language, as the code everything else stores.
    #[must_use]
    pub fn language(self) -> km_kmpkg::Language {
        self.language
    }

    /// How sure the detector was, between zero and one.
    ///
    /// Shown beside a guessed language so that a curator correcting one can see which were close
    /// calls. Never stored as anything but a number to read.
    #[must_use]
    pub fn confidence(self) -> f64 {
        self.confidence
    }
}

/// What language a song is sung in, read from its words.
///
/// **The lyrics are asked first and the title is the fallback**, because a lyric track is hundreds
/// of words where a title is four. A song with neither, and a song whose text nothing can place
/// confidently enough, is `None`.
///
/// The two are never concatenated. A title in one language over lyrics in another is a real shape —
/// a translated title, a transliterated one — and joining them would produce a reading of neither.
#[must_use]
pub fn guess(lyrics: Option<&str>, title: Option<&str>) -> Option<Guess> {
    lyrics
        .and_then(from_text)
        .or_else(|| title.and_then(from_text))
}

/// The guess for one piece of text, refused unless it is certain enough to be a fact.
fn from_text(text: &str) -> Option<Guess> {
    let info = whatlang::detect(text.trim())?;
    if info.confidence() < MIN_CONFIDENCE {
        return None;
    }
    Some(Guess {
        language: iso_639_1(info.lang())?,
        confidence: info.confidence(),
    })
}

/// The two-letter code for a detector language, where the standard has one.
fn iso_639_1(lang: whatlang::Lang) -> Option<km_kmpkg::Language> {
    ISO_639_1
        .iter()
        .find(|(three, _)| *three == lang.code())
        .and_then(|(_, two)| km_kmpkg::Language::parse(two))
}

/// Every language the detector knows, against its ISO 639-1 code.
///
/// `pes` is Western Persian, which 639-1 spells `fa` with no distinction, and `cmn` is Mandarin,
/// which lands under `zh` with the rest of Chinese — the same two-letter flattening
/// [`km_kmpkg::Language`] documents for the column as a whole.
pub const ISO_639_1: &[(&str, &str)] = &[
    ("afr", "af"),
    ("aka", "ak"),
    ("amh", "am"),
    ("ara", "ar"),
    ("aze", "az"),
    ("bel", "be"),
    ("ben", "bn"),
    ("bul", "bg"),
    ("cat", "ca"),
    ("ces", "cs"),
    ("cmn", "zh"),
    ("cym", "cy"),
    ("dan", "da"),
    ("deu", "de"),
    ("ell", "el"),
    ("eng", "en"),
    ("epo", "eo"),
    ("est", "et"),
    ("fin", "fi"),
    ("fra", "fr"),
    ("guj", "gu"),
    ("heb", "he"),
    ("hin", "hi"),
    ("hrv", "hr"),
    ("hun", "hu"),
    ("hye", "hy"),
    ("ind", "id"),
    ("ita", "it"),
    ("jav", "jv"),
    ("jpn", "ja"),
    ("kan", "kn"),
    ("kat", "ka"),
    ("khm", "km"),
    ("kor", "ko"),
    ("lat", "la"),
    ("lav", "lv"),
    ("lit", "lt"),
    ("mal", "ml"),
    ("mar", "mr"),
    ("mkd", "mk"),
    ("mya", "my"),
    ("nep", "ne"),
    ("nld", "nl"),
    ("nob", "nb"),
    ("ori", "or"),
    ("pan", "pa"),
    ("pes", "fa"),
    ("pol", "pl"),
    ("por", "pt"),
    ("ron", "ro"),
    ("rus", "ru"),
    ("sin", "si"),
    ("slk", "sk"),
    ("slv", "sl"),
    ("sna", "sn"),
    ("spa", "es"),
    ("srp", "sr"),
    ("swe", "sv"),
    ("tam", "ta"),
    ("tel", "te"),
    ("tgl", "tl"),
    ("tha", "th"),
    ("tuk", "tk"),
    ("tur", "tr"),
    ("ukr", "uk"),
    ("urd", "ur"),
    ("uzb", "uz"),
    ("vie", "vi"),
    ("yid", "yi"),
    ("zul", "zu"),
];

#[cfg(test)]
mod tests {
    use super::*;

    // Invented lines, not anybody's lyrics, so that a fixture carries no licence with it.
    const VIETNAMESE: &str = "Buổi sáng đi ngang con đường vắng một lần nữa, mỗi khung cửa sổ \
                              giữ một gương mặt không quay lại nhìn tôi";
    const PORTUGUESE: &str = "A manhã chegou devagar pela rua vazia outra vez, e cada janela \
                              guardava um rosto que não quis me olhar";
    const ENGLISH: &str = "The morning light came walking down the empty street again, and every \
                           window held a face that would not turn to me. I counted all the \
                           reasons that I gave you for the leaving, and not one of them was true";

    #[test]
    fn every_mapped_code_is_a_language() {
        for (three, two) in ISO_639_1 {
            assert!(
                km_kmpkg::Language::parse(two).is_some(),
                "{three} maps to {two}, which is not in the language table"
            );
        }
    }

    #[test]
    fn every_detector_language_is_mapped() {
        for lang in whatlang::Lang::all() {
            assert!(
                iso_639_1(*lang).is_some(),
                "the detector can answer {} and nothing maps it",
                lang.code()
            );
        }
    }

    #[test]
    fn no_code_is_mapped_twice() {
        let mut seen: Vec<&str> = ISO_639_1.iter().map(|(three, _)| *three).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "a detector code appears more than once");
    }

    #[test]
    fn a_lyric_track_is_placed() {
        for (text, code) in [(VIETNAMESE, "vi"), (PORTUGUESE, "pt"), (ENGLISH, "en")] {
            let got = guess(Some(text), None).expect("a verse is enough to place");
            assert_eq!(got.language().code(), code, "{text}");
        }
    }

    /// Characters only one language writes place a title with no verse behind it.
    #[test]
    fn a_title_in_an_alphabet_of_its_own_is_placed() {
        for (title, code) in [("ÁO ẢNH SA MẠC", "vi"), ("静かな朝の歌", "ja")] {
            let got = guess(None, Some(title)).expect("distinctive characters are certain");
            assert_eq!(got.language().code(), code, "{title}");
        }
    }

    /// The gate that stops a guess becoming a fact: two ordinary words in a shared alphabet.
    #[test]
    fn a_title_its_alphabet_cannot_place_is_refused() {
        assert!(guess(None, Some("The Empty Street")).is_none());
        assert!(guess(Some("Áo Xanh"), None).is_none());
    }

    #[test]
    fn nothing_to_read_is_no_guess() {
        assert!(guess(None, None).is_none());
        assert!(guess(Some(""), Some("   ")).is_none());
        assert!(guess(Some("1234 (5) - [6]"), None).is_none());
    }

    #[test]
    fn the_title_answers_when_there_are_no_lyrics() {
        let got = guess(None, Some(VIETNAMESE)).expect("a title is text like any other");
        assert_eq!(got.language().code(), "vi");
    }

    #[test]
    fn the_lyrics_outrank_the_title() {
        let got = guess(Some(PORTUGUESE), Some(VIETNAMESE)).expect("lyrics are asked first");
        assert_eq!(got.language().code(), "pt");
    }

    /// A guess the lyrics refuse falls through to the title rather than ending there.
    #[test]
    fn a_refused_lyric_still_lets_the_title_answer() {
        let got = guess(Some("Áo Xanh"), Some(VIETNAMESE)).expect("the title is still asked");
        assert_eq!(got.language().code(), "vi");
    }

    #[test]
    fn the_confidence_comes_back_with_the_language() {
        let got = guess(Some(VIETNAMESE), None).expect("placed");
        assert!(
            got.confidence() >= MIN_CONFIDENCE,
            "a stored guess is never below the gate"
        );
    }
}
