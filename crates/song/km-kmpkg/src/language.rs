//! What language a song is sung in, as a code rather than a string.
//!
//! Every song has carried a `language` field since packages existed, and until now it was whatever
//! the file happened to say: the local corpus holds `ENGL`, `PORT`, `ITAL` and `ITALIANO`, the
//! fixtures hold `eng` and `por`, and a video song holds nothing at all. Four spellings of one
//! language is a column that can be *stored* and never *asked about*.
//!
//! So a language is one of the [`Language`] values below — an **ISO 639-1 two-letter code**, from a
//! table compiled into the binary, carrying an English name for the pickers that have to show it.
//!
//! # The list is the whole standard, not a selection
//!
//! All 184 ISO 639-1 codes are here, so this table is a *transcription* rather than a judgment, and
//! there is never a question about whether some language has earned a row. Two consequences worth
//! knowing: it does not need editing when somebody turns up with a Welsh song, and [`TABLE_REVISION`]
//! should essentially never change, because the standard does not.
//!
//! # What two letters cannot say
//!
//! ISO 639-1 has no script or region, so `pt` cannot distinguish Brazilian from European Portuguese
//! and `zh` cannot distinguish Simplified from Traditional — and it has no code for Cantonese at all,
//! which lands under `zh` with everything else. BCP 47 (`pt-BR`, `zh-Hant`, `yue-HK`) can express all
//! three and was considered; two letters were chosen for how they read and type. Every code here is
//! also a valid BCP 47 primary subtag, so nothing stored now would have to change if that is ever
//! revisited — the codes would only gain suffixes.
//!
//! # The two codes that are not ISO 639-1, and why
//!
//! [`Language::undetermined`] (`und`) and `zxx` come from ISO 639-2/-3, which is a deliberate
//! exception to the paragraph above: 639-1 offers no way to say *somebody looked and could not tell*,
//! and `km-pack build --default-language` needs something to write; and `zxx` — "no linguistic
//! content" — is what an instrumental backing track honestly is, of which the corpus holds thousands.
//! Both are three letters, which is exactly why they cannot be mistaken for a language.
//!
//! `mul` ("multiple languages") is deliberately *not* offered: it is neither a language anybody
//! filters to nor a statement that nobody has looked, so it would only be a third kind of empty.

/// How many times this table has changed.
///
/// Stored by `km-package-builder` beside its detected-language column, so that a change here re-runs its
/// backfill exactly once. Bump it whenever [`DECLARED`] or [`ENCODINGS`] gains or loses an entry — a
/// consumer cannot see a source edit, only this number. [`LANGUAGES`] itself is the ISO standard and
/// is not expected to move.
pub const TABLE_REVISION: u32 = 1;

/// One row of the table.
struct Entry {
    /// The two-letter code, and the only thing ever stored.
    code: &'static str,
    /// English name, for a `<select>` and for the display.
    name: &'static str,
}

/// A language a song can be in.
///
/// A borrowed row of the compiled-in table, so it is `Copy`, cannot be constructed with a code the
/// standard does not have, and compares by identity.
#[derive(Clone, Copy)]
pub struct Language(&'static Entry);

impl Language {
    /// The wire name — `"pt"` — as it appears in a manifest, a database and the API.
    ///
    /// Deliberately the same contract as [`crate::SongKind::as_str`]: this is the spelling that is
    /// stored, and [`Language::parse`] reads back exactly what it wrote.
    ///
    /// **`code`, not `tag`.** `km_locale::Locale::tag` is a BCP 47 tag, which is what that standard
    /// calls it, and a song tag is a word somebody typed onto a song. Three meanings of one word
    /// is what the `One spelling per concept` decision exists to stop, and this is the one of the
    /// three that was never a tag in the first place.
    #[must_use]
    pub fn code(self) -> &'static str {
        self.0.code
    }

    /// The English name — `"Portuguese"` — for a picker and for the display.
    ///
    /// Never stored. A name is what a person reads; a code is what everything else takes.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.0.name
    }

    /// Every language, in code order.
    pub fn all() -> impl Iterator<Item = Language> {
        LANGUAGES.iter().map(Language)
    }

    /// Every language, ordered by [`Language::name`] — the order a picker shows them in.
    ///
    /// Sorted here rather than in the table so that [`LANGUAGES`] can stay in code order, where it
    /// can be checked against the standard a line at a time. 184 rows sorted once per page render is
    /// not worth a lazily-initialized static.
    #[must_use]
    pub fn by_name() -> Vec<Language> {
        let mut all: Vec<Language> = Self::all().collect();
        all.sort_by_key(|language| language.name());
        all
    }

    /// Reads a code back, forgiving case and surrounding space.
    ///
    /// An unfamiliar value is `None` rather than a guess. There is no safe default here — unlike
    /// [`crate::SongKind`], where "assume MIDI" is right, assuming a language would be inventing a
    /// fact.
    #[must_use]
    pub fn parse(value: &str) -> Option<Language> {
        let folded = fold(value);
        LANGUAGES
            .iter()
            .find(|entry| entry.code == folded)
            .map(Language)
    }

    /// `und` — "undetermined".
    ///
    /// What `km-pack build --default-language und` writes: a song that *says* nobody could tell,
    /// which is a fact, and which a search can find again. Not what an unreadable value maps to —
    /// see [`Language::detect`].
    #[must_use]
    pub fn undetermined() -> Language {
        Language::parse("und").expect("und is in the table")
    }

    /// What the file itself says its language is, best evidence first.
    ///
    /// `encoding` is the canonical label of the decoder the lyrics were read with (what
    /// `km_song::TextDecoder::name` returns), and `declared` is the Soft Karaoke `@L` header
    /// verbatim. **The encoding wins where it speaks at all**, because it is the far stronger
    /// signal: a Shift-JIS lyric track is Japanese whatever the header claims, and — see
    /// [`DECLARED`] — the header very often claims English regardless.
    ///
    /// When neither speaks the answer is `None`, and `None` means *nothing is written*: not `und`,
    /// not a guess. `und` is a statement that somebody looked and could not tell, so writing it
    /// automatically would satisfy the packaging rule on every song in a corpus without anybody
    /// having seen one.
    #[must_use]
    pub fn detect(declared: Option<&str>, encoding: Option<&str>) -> Option<Language> {
        encoding
            .and_then(Self::from_encoding)
            .or_else(|| declared.and_then(Self::from_declared))
    }

    /// Maps a Soft Karaoke `@L` header onto a code.
    ///
    /// Three layers, each tried in turn: a value that is already a code (a file that says `ja` needs
    /// no table entry), then the four-letter abbreviations the format actually uses, then the
    /// ISO 639-2/-3 three-letter codes that turn up in the wild — both the bibliographic and the
    /// terminological spelling, because files carry both.
    ///
    /// **This is transcription, not inference.** The file made a statement and this reads it; it is
    /// not the same thing as guessing a language from a title, which nothing here does.
    ///
    /// See [`DECLARED`] for how much the statement is worth, which is less than it looks.
    #[must_use]
    pub fn from_declared(raw: &str) -> Option<Language> {
        if let Some(language) = Self::parse(raw) {
            return Some(language);
        }
        // Real headers carry decoration: `HRV (CROSCII)` names its character set, and `English.`
        // and `Italian.` end in a full stop. Both are the same word with something after it, so the
        // decoration is cut rather than each variant being given a row of its own.
        let head = raw.split('(').next().unwrap_or(raw);
        let folded = fold(head.trim_end_matches(['.', ',', ';', ':', ' ', '\t']));
        if let Some(language) = Self::parse(&folded) {
            return Some(language);
        }
        DECLARED
            .iter()
            .find(|(declared, _)| *declared == folded)
            .and_then(|(_, code)| Self::parse(code))
    }

    /// What the lyrics' text encoding implies, where it implies anything.
    ///
    /// The stronger of the two signals, and the only reason a Japanese file does not end up filed as
    /// English — a Japanese karaoke file almost certainly also says `@LENGL`.
    ///
    /// Only unambiguous encodings are mapped; see [`ENCODINGS`]. The decoder's *source* — whether
    /// the encoding was declared, sniffed or fallen back to — is deliberately not consulted, because
    /// the fallback is always `windows-1252` and `windows-1252` maps to nothing anyway.
    #[must_use]
    pub fn from_encoding(label: &str) -> Option<Language> {
        let folded = fold(label);
        ENCODINGS
            .iter()
            .find(|(encoding, _)| *encoding == folded)
            .and_then(|(_, code)| Self::parse(code))
    }
}

impl PartialEq for Language {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Language {}

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// Trims and lowercases, so one comparison serves every lookup table.
///
/// Every code in every table below is already lowercase, which is a property ISO 639-1 hands us for
/// free and which a test pins — so there is no canonical-casing algorithm to get wrong.
fn fold(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|c| unaccent(c).to_ascii_lowercase())
        .collect()
}

/// Strips the accent off a Latin letter, leaving everything else alone.
///
/// So a header that says `Français` — 194 files in the local corpus do — matches the plain
/// `francais` in [`DECLARED`], and every lookup table below can stay ASCII. That matters more than
/// it sounds: a non-ASCII key is a key that a careless `sed` or `perl -i` silently replaces with
/// `U+FFFD`, which is exactly how this function came to be written.
///
/// Latin-1 and Latin Extended-A only. There is nothing to fold in the scripts this does not cover,
/// and nothing here is trying to be a general normalizer.
fn unaccent(c: char) -> char {
    match c {
        'á'..='å' | 'à' | 'ā' | 'ă' | 'ą' | 'Á'..='Å' | 'À' | 'Ā' | 'Ă' | 'Ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => 'c',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È'..='Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => {
            'e'
        }
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'Ì'..='Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' => 'i',
        'ñ' | 'ń' | 'ņ' | 'ň' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'Ò'..='Ö' | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => 'o',
        'ù'..='ü'
        | 'ũ'
        | 'ū'
        | 'ŭ'
        | 'ů'
        | 'ű'
        | 'ų'
        | 'Ù'..='Ü'
        | 'Ũ'
        | 'Ū'
        | 'Ŭ'
        | 'Ů'
        | 'Ű'
        | 'Ų' => 'u',
        'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ÿ' | 'Ŷ' => 'y',
        'š' | 'ś' | 'ŝ' | 'ş' | 'Š' | 'Ś' | 'Ŝ' | 'Ş' => 's',
        'ž' | 'ź' | 'ż' | 'Ž' | 'Ź' | 'Ż' => 'z',
        'ł' | 'Ł' => 'l',
        other => other,
    }
}

/// Every ISO 639-1 language, in code order, plus the two non-language answers.
///
/// **In code order on purpose**, so the list can be checked against the standard a line at a time;
/// [`Language::by_name`] is what a picker shows. A test holds the order, so an insertion in the wrong
/// place is caught rather than quietly making the file harder to audit.
///
/// `und` and `zxx` are the two entries that are not ISO 639-1 — see the module doc for why they are
/// here. They sit in code order with everything else, and their three letters are what makes them
/// visibly not languages.
static LANGUAGES: &[Entry] = &[
    Entry {
        code: "aa",
        name: "Afar",
    },
    Entry {
        code: "ab",
        name: "Abkhazian",
    },
    Entry {
        code: "ae",
        name: "Avestan",
    },
    Entry {
        code: "af",
        name: "Afrikaans",
    },
    Entry {
        code: "ak",
        name: "Akan",
    },
    Entry {
        code: "am",
        name: "Amharic",
    },
    Entry {
        code: "an",
        name: "Aragonese",
    },
    Entry {
        code: "ar",
        name: "Arabic",
    },
    Entry {
        code: "as",
        name: "Assamese",
    },
    Entry {
        code: "av",
        name: "Avaric",
    },
    Entry {
        code: "ay",
        name: "Aymara",
    },
    Entry {
        code: "az",
        name: "Azerbaijani",
    },
    Entry {
        code: "ba",
        name: "Bashkir",
    },
    Entry {
        code: "be",
        name: "Belarusian",
    },
    Entry {
        code: "bg",
        name: "Bulgarian",
    },
    Entry {
        code: "bh",
        name: "Bihari languages",
    },
    Entry {
        code: "bi",
        name: "Bislama",
    },
    Entry {
        code: "bm",
        name: "Bambara",
    },
    Entry {
        code: "bn",
        name: "Bengali",
    },
    Entry {
        code: "bo",
        name: "Tibetan",
    },
    Entry {
        code: "br",
        name: "Breton",
    },
    Entry {
        code: "bs",
        name: "Bosnian",
    },
    Entry {
        code: "ca",
        name: "Catalan",
    },
    Entry {
        code: "ce",
        name: "Chechen",
    },
    Entry {
        code: "ch",
        name: "Chamorro",
    },
    Entry {
        code: "co",
        name: "Corsican",
    },
    Entry {
        code: "cr",
        name: "Cree",
    },
    Entry {
        code: "cs",
        name: "Czech",
    },
    Entry {
        code: "cu",
        name: "Church Slavic",
    },
    Entry {
        code: "cv",
        name: "Chuvash",
    },
    Entry {
        code: "cy",
        name: "Welsh",
    },
    Entry {
        code: "da",
        name: "Danish",
    },
    Entry {
        code: "de",
        name: "German",
    },
    Entry {
        code: "dv",
        name: "Divehi",
    },
    Entry {
        code: "dz",
        name: "Dzongkha",
    },
    Entry {
        code: "ee",
        name: "Ewe",
    },
    Entry {
        code: "el",
        name: "Greek",
    },
    Entry {
        code: "en",
        name: "English",
    },
    Entry {
        code: "eo",
        name: "Esperanto",
    },
    Entry {
        code: "es",
        name: "Spanish",
    },
    Entry {
        code: "et",
        name: "Estonian",
    },
    Entry {
        code: "eu",
        name: "Basque",
    },
    Entry {
        code: "fa",
        name: "Persian",
    },
    Entry {
        code: "ff",
        name: "Fulah",
    },
    Entry {
        code: "fi",
        name: "Finnish",
    },
    Entry {
        code: "fj",
        name: "Fijian",
    },
    Entry {
        code: "fo",
        name: "Faroese",
    },
    Entry {
        code: "fr",
        name: "French",
    },
    Entry {
        code: "fy",
        name: "Western Frisian",
    },
    Entry {
        code: "ga",
        name: "Irish",
    },
    Entry {
        code: "gd",
        name: "Scottish Gaelic",
    },
    Entry {
        code: "gl",
        name: "Galician",
    },
    Entry {
        code: "gn",
        name: "Guarani",
    },
    Entry {
        code: "gu",
        name: "Gujarati",
    },
    Entry {
        code: "gv",
        name: "Manx",
    },
    Entry {
        code: "ha",
        name: "Hausa",
    },
    Entry {
        code: "he",
        name: "Hebrew",
    },
    Entry {
        code: "hi",
        name: "Hindi",
    },
    Entry {
        code: "ho",
        name: "Hiri Motu",
    },
    Entry {
        code: "hr",
        name: "Croatian",
    },
    Entry {
        code: "ht",
        name: "Haitian Creole",
    },
    Entry {
        code: "hu",
        name: "Hungarian",
    },
    Entry {
        code: "hy",
        name: "Armenian",
    },
    Entry {
        code: "hz",
        name: "Herero",
    },
    Entry {
        code: "ia",
        name: "Interlingua",
    },
    Entry {
        code: "id",
        name: "Indonesian",
    },
    Entry {
        code: "ie",
        name: "Interlingue",
    },
    Entry {
        code: "ig",
        name: "Igbo",
    },
    Entry {
        code: "ii",
        name: "Sichuan Yi",
    },
    Entry {
        code: "ik",
        name: "Inupiaq",
    },
    Entry {
        code: "io",
        name: "Ido",
    },
    Entry {
        code: "is",
        name: "Icelandic",
    },
    Entry {
        code: "it",
        name: "Italian",
    },
    Entry {
        code: "iu",
        name: "Inuktitut",
    },
    Entry {
        code: "ja",
        name: "Japanese",
    },
    Entry {
        code: "jv",
        name: "Javanese",
    },
    Entry {
        code: "ka",
        name: "Georgian",
    },
    Entry {
        code: "kg",
        name: "Kongo",
    },
    Entry {
        code: "ki",
        name: "Kikuyu",
    },
    Entry {
        code: "kj",
        name: "Kuanyama",
    },
    Entry {
        code: "kk",
        name: "Kazakh",
    },
    Entry {
        code: "kl",
        name: "Kalaallisut",
    },
    Entry {
        code: "km",
        name: "Khmer",
    },
    Entry {
        code: "kn",
        name: "Kannada",
    },
    Entry {
        code: "ko",
        name: "Korean",
    },
    Entry {
        code: "kr",
        name: "Kanuri",
    },
    Entry {
        code: "ks",
        name: "Kashmiri",
    },
    Entry {
        code: "ku",
        name: "Kurdish",
    },
    Entry {
        code: "kv",
        name: "Komi",
    },
    Entry {
        code: "kw",
        name: "Cornish",
    },
    Entry {
        code: "ky",
        name: "Kyrgyz",
    },
    Entry {
        code: "la",
        name: "Latin",
    },
    Entry {
        code: "lb",
        name: "Luxembourgish",
    },
    Entry {
        code: "lg",
        name: "Ganda",
    },
    Entry {
        code: "li",
        name: "Limburgish",
    },
    Entry {
        code: "ln",
        name: "Lingala",
    },
    Entry {
        code: "lo",
        name: "Lao",
    },
    Entry {
        code: "lt",
        name: "Lithuanian",
    },
    Entry {
        code: "lu",
        name: "Luba-Katanga",
    },
    Entry {
        code: "lv",
        name: "Latvian",
    },
    Entry {
        code: "mg",
        name: "Malagasy",
    },
    Entry {
        code: "mh",
        name: "Marshallese",
    },
    Entry {
        code: "mi",
        name: "Maori",
    },
    Entry {
        code: "mk",
        name: "Macedonian",
    },
    Entry {
        code: "ml",
        name: "Malayalam",
    },
    Entry {
        code: "mn",
        name: "Mongolian",
    },
    Entry {
        code: "mr",
        name: "Marathi",
    },
    Entry {
        code: "ms",
        name: "Malay",
    },
    Entry {
        code: "mt",
        name: "Maltese",
    },
    Entry {
        code: "my",
        name: "Burmese",
    },
    Entry {
        code: "na",
        name: "Nauru",
    },
    Entry {
        code: "nb",
        name: "Norwegian Bokmal",
    },
    Entry {
        code: "nd",
        name: "North Ndebele",
    },
    Entry {
        code: "ne",
        name: "Nepali",
    },
    Entry {
        code: "ng",
        name: "Ndonga",
    },
    Entry {
        code: "nl",
        name: "Dutch",
    },
    Entry {
        code: "nn",
        name: "Norwegian Nynorsk",
    },
    Entry {
        code: "no",
        name: "Norwegian",
    },
    Entry {
        code: "nr",
        name: "South Ndebele",
    },
    Entry {
        code: "nv",
        name: "Navajo",
    },
    Entry {
        code: "ny",
        name: "Nyanja",
    },
    Entry {
        code: "oc",
        name: "Occitan",
    },
    Entry {
        code: "oj",
        name: "Ojibwa",
    },
    Entry {
        code: "om",
        name: "Oromo",
    },
    Entry {
        code: "or",
        name: "Oriya",
    },
    Entry {
        code: "os",
        name: "Ossetian",
    },
    Entry {
        code: "pa",
        name: "Punjabi",
    },
    Entry {
        code: "pi",
        name: "Pali",
    },
    Entry {
        code: "pl",
        name: "Polish",
    },
    Entry {
        code: "ps",
        name: "Pashto",
    },
    Entry {
        code: "pt",
        name: "Portuguese",
    },
    Entry {
        code: "qu",
        name: "Quechua",
    },
    Entry {
        code: "rm",
        name: "Romansh",
    },
    Entry {
        code: "rn",
        name: "Rundi",
    },
    Entry {
        code: "ro",
        name: "Romanian",
    },
    Entry {
        code: "ru",
        name: "Russian",
    },
    Entry {
        code: "rw",
        name: "Kinyarwanda",
    },
    Entry {
        code: "sa",
        name: "Sanskrit",
    },
    Entry {
        code: "sc",
        name: "Sardinian",
    },
    Entry {
        code: "sd",
        name: "Sindhi",
    },
    Entry {
        code: "se",
        name: "Northern Sami",
    },
    Entry {
        code: "sg",
        name: "Sango",
    },
    Entry {
        code: "si",
        name: "Sinhala",
    },
    Entry {
        code: "sk",
        name: "Slovak",
    },
    Entry {
        code: "sl",
        name: "Slovenian",
    },
    Entry {
        code: "sm",
        name: "Samoan",
    },
    Entry {
        code: "sn",
        name: "Shona",
    },
    Entry {
        code: "so",
        name: "Somali",
    },
    Entry {
        code: "sq",
        name: "Albanian",
    },
    Entry {
        code: "sr",
        name: "Serbian",
    },
    Entry {
        code: "ss",
        name: "Swati",
    },
    Entry {
        code: "st",
        name: "Southern Sotho",
    },
    Entry {
        code: "su",
        name: "Sundanese",
    },
    Entry {
        code: "sv",
        name: "Swedish",
    },
    Entry {
        code: "sw",
        name: "Swahili",
    },
    Entry {
        code: "ta",
        name: "Tamil",
    },
    Entry {
        code: "te",
        name: "Telugu",
    },
    Entry {
        code: "tg",
        name: "Tajik",
    },
    Entry {
        code: "th",
        name: "Thai",
    },
    Entry {
        code: "ti",
        name: "Tigrinya",
    },
    Entry {
        code: "tk",
        name: "Turkmen",
    },
    Entry {
        code: "tl",
        name: "Tagalog",
    },
    Entry {
        code: "tn",
        name: "Tswana",
    },
    Entry {
        code: "to",
        name: "Tongan",
    },
    Entry {
        code: "tr",
        name: "Turkish",
    },
    Entry {
        code: "ts",
        name: "Tsonga",
    },
    Entry {
        code: "tt",
        name: "Tatar",
    },
    Entry {
        code: "tw",
        name: "Twi",
    },
    Entry {
        code: "ty",
        name: "Tahitian",
    },
    Entry {
        code: "ug",
        name: "Uighur",
    },
    Entry {
        code: "uk",
        name: "Ukrainian",
    },
    // Not ISO 639-1: the standard has no way to say "somebody looked and could not tell".
    Entry {
        code: "und",
        name: "Undetermined",
    },
    Entry {
        code: "ur",
        name: "Urdu",
    },
    Entry {
        code: "uz",
        name: "Uzbek",
    },
    Entry {
        code: "ve",
        name: "Venda",
    },
    Entry {
        code: "vi",
        name: "Vietnamese",
    },
    Entry {
        code: "vo",
        name: "Volapuk",
    },
    Entry {
        code: "wa",
        name: "Walloon",
    },
    Entry {
        code: "wo",
        name: "Wolof",
    },
    Entry {
        code: "xh",
        name: "Xhosa",
    },
    Entry {
        code: "yi",
        name: "Yiddish",
    },
    Entry {
        code: "yo",
        name: "Yoruba",
    },
    Entry {
        code: "za",
        name: "Zhuang",
    },
    Entry {
        code: "zh",
        name: "Chinese",
    },
    Entry {
        code: "zu",
        name: "Zulu",
    },
    // Not ISO 639-1 either: an instrumental backing track has no words, which is a different fact
    // from nobody having looked.
    Entry {
        code: "zxx",
        name: "No words (instrumental)",
    },
];

/// What a Soft Karaoke `@L` header means, for the spellings that actually occur.
///
/// # How much this is worth
///
/// Less than it looks, and the numbers are worth keeping because the obvious reading of them is
/// wrong. Sampling 60 `.kar` files from each of the local corpus's language-named folders:
///
/// | folder | `@L` says |
/// |---|---|
/// | `Ingles/` | 59 `ENGL`, 1 `ENG` — right |
/// | `Brasil/` | **35 `ENGL`**, 1 `PORT`, 24 nothing |
/// | `Musicas Nacionais/` | **13 `ENGL`**, 9 `PORT`, 37 nothing |
/// | `Musicas Italianas/` | **30 `ENGL`**, 28 Italian, 1 nothing |
///
/// `ENGL` is the editor's *default*, not a statement: across 2,500 files it appears 1,724 times
/// against 163 `PORT` and 53 Italian. Every other value is typed on purpose and is reliable — nobody
/// writes `PORT` by accident.
///
/// **`ENGL` is nevertheless mapped, and that is deliberate.** Discarding it was tried first and
/// rejected: it leaves over 90% of a real corpus with no language at all, and a column that is empty
/// on nine songs in ten answers no question. A column that is populated and sometimes wrong can be
/// corrected in one action — which is what `km-package-builder`'s bulk set over the current filter
/// is for. Do not "fix" this by reading the table above and removing the English rows.
///
/// One further rule: **an abbreviation that names more than one thing is absent.** `LATI` is Latin or
/// Latin-American depending on who wrote the file, so it is not here.
static DECLARED: &[(&str, &str)] = &[
    ("engl", "en"),
    ("engle", "en"),
    ("eng", "en"),
    ("english", "en"),
    ("ingles", "en"),
    ("port", "pt"),
    ("por", "pt"),
    ("portugues", "pt"),
    ("portuguese", "pt"),
    ("span", "es"),
    ("spa", "es"),
    ("espa", "es"),
    ("esp", "es"),
    ("espanol", "es"),
    ("spanish", "es"),
    ("ital", "it"),
    ("ita", "it"),
    ("italiano", "it"),
    ("italian", "it"),
    ("fren", "fr"),
    ("fra", "fr"),
    ("fre", "fr"),
    ("fran", "fr"),
    ("francais", "fr"),
    ("frn", "fr"),
    ("french", "fr"),
    ("germ", "de"),
    ("ger", "de"),
    ("deu", "de"),
    ("deut", "de"),
    ("deutsch", "de"),
    ("german", "de"),
    ("dutc", "nl"),
    ("dut", "nl"),
    ("nld", "nl"),
    ("nede", "nl"),
    ("dutch", "nl"),
    ("swed", "sv"),
    ("swe", "sv"),
    ("swedish", "sv"),
    ("norw", "no"),
    ("nor", "no"),
    ("dani", "da"),
    ("dans", "da"),
    ("dan", "da"),
    ("finn", "fi"),
    ("fin", "fi"),
    ("poli", "pl"),
    ("pols", "pl"),
    ("pol", "pl"),
    ("polish", "pl"),
    ("russ", "ru"),
    ("rus", "ru"),
    ("russian", "ru"),
    ("ukra", "uk"),
    ("ukr", "uk"),
    ("turk", "tr"),
    ("tur", "tr"),
    ("gree", "el"),
    ("gre", "el"),
    ("ell", "el"),
    ("czec", "cs"),
    ("cze", "cs"),
    ("ces", "cs"),
    ("hung", "hu"),
    ("hun", "hu"),
    ("roma", "ro"),
    ("ron", "ro"),
    ("rum", "ro"),
    ("serb", "sr"),
    ("srp", "sr"),
    ("croa", "hr"),
    ("hrv", "hr"),
    ("bulg", "bg"),
    ("bul", "bg"),
    ("esto", "et"),
    ("est", "et"),
    ("latv", "lv"),
    ("lav", "lv"),
    ("lith", "lt"),
    ("lit", "lt"),
    ("icel", "is"),
    ("isl", "is"),
    ("ice", "is"),
    ("cata", "ca"),
    ("cat", "ca"),
    ("basq", "eu"),
    ("baq", "eu"),
    ("gali", "gl"),
    ("glg", "gl"),
    ("japa", "ja"),
    ("jpan", "ja"),
    ("jpn", "ja"),
    ("japanese", "ja"),
    ("kore", "ko"),
    ("kor", "ko"),
    ("korean", "ko"),
    ("chin", "zh"),
    ("zho", "zh"),
    ("chi", "zh"),
    ("chinese", "zh"),
    ("thai", "th"),
    ("tha", "th"),
    ("viet", "vi"),
    ("vie", "vi"),
    ("indo", "id"),
    ("ind", "id"),
    ("taga", "tl"),
    ("tgl", "tl"),
    ("fili", "tl"),
    ("arab", "ar"),
    ("ara", "ar"),
    ("hebr", "he"),
    ("heb", "he"),
    ("hind", "hi"),
    ("hin", "hi"),
    ("afri", "af"),
    ("afr", "af"),
];

/// What a lyric encoding implies about the language, where it implies anything.
///
/// Keyed by the canonical `encoding_rs` label, lowercased — which is what `km_song::TextDecoder`
/// reports and what `km-package-builder` stores.
///
/// **Only what is unambiguous.** `windows-1252` is Latin-1 and covers a dozen languages;
/// `windows-1250` (Central European), `windows-1251` (Cyrillic — Russian or Ukrainian or Bulgarian)
/// and `windows-1257` (Baltic) each name a *region*, not a language. UTF-8 says nothing at all. All
/// of those are absent for the same reason `LATI` is absent from [`DECLARED`].
///
/// Big5 and GBK both land on `zh`: the distinction they carry is Traditional against Simplified, and
/// two letters cannot say it. See the module doc.
static ENCODINGS: &[(&str, &str)] = &[
    ("shift_jis", "ja"),
    ("euc-jp", "ja"),
    ("iso-2022-jp", "ja"),
    ("euc-kr", "ko"),
    ("gbk", "zh"),
    ("gb18030", "zh"),
    ("big5", "zh"),
    ("windows-874", "th"),
    ("windows-1253", "el"),
    ("windows-1254", "tr"),
    ("windows-1255", "he"),
    ("windows-1256", "ar"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(language: Option<Language>) -> Option<&'static str> {
        language.map(Language::code)
    }

    #[test]
    fn a_code_reads_back_as_it_was_written() {
        for language in Language::all() {
            assert_eq!(
                Language::parse(language.code()),
                Some(language),
                "{} did not round-trip",
                language.code()
            );
        }
    }

    #[test]
    fn casing_and_surrounding_space_are_forgiven() {
        for spelling in ["pt", "PT", "  pt  ", "Pt"] {
            assert_eq!(code_of(Language::parse(spelling)), Some("pt"), "{spelling}");
        }
        assert_eq!(code_of(Language::parse("JA")), Some("ja"));
        assert_eq!(code_of(Language::parse("UND")), Some("und"));
    }

    #[test]
    fn every_code_is_two_letters_except_the_two_that_are_not_languages() {
        for language in Language::all() {
            let expected = match language.code() {
                "und" | "zxx" => 3,
                _ => 2,
            };
            assert_eq!(
                language.code().len(),
                expected,
                "{} is the wrong length for what it is",
                language.code()
            );
            assert!(
                language.code().chars().all(|c| c.is_ascii_lowercase()),
                "{} is not lowercase ascii, which `fold` assumes",
                language.code()
            );
        }
    }

    #[test]
    fn every_code_and_every_name_is_unique() {
        let mut codes: Vec<&str> = LANGUAGES.iter().map(|e| e.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count, "a code appears twice in the table");

        let mut names: Vec<&str> = LANGUAGES.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "a name appears twice in the table");
    }

    #[test]
    fn the_table_is_in_code_order() {
        // So it can be checked against the standard a line at a time. `by_name` is what a picker
        // shows, and it sorts.
        let codes: Vec<&str> = LANGUAGES.iter().map(|e| e.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        assert_eq!(codes, sorted, "the table is not in code order");
    }

    #[test]
    fn the_whole_standard_is_here() {
        // 184 ISO 639-1 codes, plus `und` and `zxx`. A wrong count means a row was dropped or
        // duplicated while editing, which nothing else here would notice.
        assert_eq!(LANGUAGES.len(), 186);
        for expected in [
            "en", "pt", "es", "ja", "ko", "zh", "it", "fr", "de", "cy", "sw",
        ] {
            assert!(Language::parse(expected).is_some(), "{expected} is missing");
        }
    }

    #[test]
    fn a_picker_gets_them_in_name_order() {
        let names: Vec<&str> = Language::by_name().iter().map(|l| l.name()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn a_soft_karaoke_tag_maps_to_a_code() {
        assert_eq!(code_of(Language::from_declared("PORT")), Some("pt"));
        assert_eq!(code_of(Language::from_declared("ITALIANO")), Some("it"));
        assert_eq!(code_of(Language::from_declared("ESPA")), Some("es"));
        assert_eq!(code_of(Language::from_declared("FREN")), Some("fr"));
        assert_eq!(code_of(Language::from_declared("jpn")), Some("ja"));
        assert_eq!(code_of(Language::from_declared("por")), Some("pt"));
        // Already a code: no map entry needed.
        assert_eq!(code_of(Language::from_declared("ja")), Some("ja"));
    }

    #[test]
    fn english_is_mapped_even_though_it_is_weak_evidence() {
        // `ENGL` is the Soft Karaoke editor's default and is wrong for most non-English songs -- in
        // the corpus's own `Brasil/` folder it is wrong 35 times out of 36. Discarding it was tried
        // and rejected: it leaves over 90% of a real corpus blank, and a mostly-empty column answers
        // nothing. See the `DECLARED` doc comment before changing this.
        assert_eq!(code_of(Language::from_declared("ENGL")), Some("en"));
        assert_eq!(code_of(Language::from_declared("ENG")), Some("en"));
        assert_eq!(code_of(Language::from_declared("English")), Some("en"));
    }

    #[test]
    fn a_header_with_decoration_on_it_still_reads() {
        // Every one of these was found in the local corpus, by counting what the mapping could not
        // read across the whole corpus. `HRV (CROSCII)` names its character set after the language;
        // `English.` and `Italian.` end in a full stop.
        assert_eq!(
            code_of(Language::from_declared("HRV (CROSCII)")),
            Some("hr")
        );
        assert_eq!(code_of(Language::from_declared("English.")), Some("en"));
        assert_eq!(code_of(Language::from_declared("Italian.")), Some("it"));
        assert_eq!(code_of(Language::from_declared("Français")), Some("fr"));
        assert_eq!(code_of(Language::from_declared("ESP")), Some("es"));
        assert_eq!(code_of(Language::from_declared("Engle")), Some("en"));
    }

    #[test]
    fn an_unrecognized_declaration_maps_to_nothing() {
        // Not to `und`: that is a statement somebody made, and nobody made it here.
        for raw in ["LATI", "", "   ", "Karaoke", "H7", "-D"] {
            assert_eq!(Language::from_declared(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn every_declared_and_encoded_value_names_a_language_in_the_table() {
        // Holds the three tables together: a row here that names a code the main table lost would
        // otherwise silently start mapping to nothing.
        for (raw, tag) in DECLARED {
            assert!(
                Language::parse(tag).is_some(),
                "{raw} -> {tag} is not a language"
            );
        }
        for (encoding, tag) in ENCODINGS {
            assert!(
                Language::parse(tag).is_some(),
                "{encoding} -> {tag} is not a language"
            );
        }
    }

    #[test]
    fn an_unambiguous_encoding_names_a_language() {
        assert_eq!(code_of(Language::from_encoding("Shift_JIS")), Some("ja"));
        assert_eq!(code_of(Language::from_encoding("EUC-KR")), Some("ko"));
        assert_eq!(code_of(Language::from_encoding("Big5")), Some("zh"));
        assert_eq!(code_of(Language::from_encoding("windows-874")), Some("th"));
    }

    #[test]
    fn an_encoding_that_names_a_region_rather_than_a_language_says_nothing() {
        for label in [
            "windows-1252",
            "windows-1251",
            "windows-1250",
            "windows-1257",
            "UTF-8",
        ] {
            assert_eq!(Language::from_encoding(label), None, "{label}");
        }
    }

    #[test]
    fn the_encoding_outranks_what_the_file_declared() {
        // The whole reason a Japanese file is not filed as English: it almost certainly also says
        // `@LENGL`, and the bytes are the better witness.
        assert_eq!(
            code_of(Language::detect(Some("ENGL"), Some("Shift_JIS"))),
            Some("ja")
        );
        // ...and where the encoding says nothing, the header stands.
        assert_eq!(
            code_of(Language::detect(Some("ENGL"), Some("windows-1252"))),
            Some("en")
        );
        assert_eq!(code_of(Language::detect(Some("PORT"), None)), Some("pt"));
        assert_eq!(Language::detect(None, Some("windows-1252")), None);
        assert_eq!(Language::detect(None, None), None);
    }

    #[test]
    fn und_and_zxx_are_offered_and_mul_is_not() {
        assert_eq!(Language::undetermined().code(), "und");
        assert!(Language::parse("zxx").is_some());
        assert!(
            Language::parse("mul").is_none(),
            "\"several\" is neither a language to filter to nor a statement that nobody has said"
        );
    }

    #[test]
    fn a_region_qualified_tag_is_not_a_code_here() {
        // ISO 639-1 has no region. Every code here is a valid BCP 47 primary subtag, so widening to
        // `pt-BR` later would only add suffixes -- but until then this is a typo, not a language.
        for spelling in ["pt-BR", "zh-Hant", "en-GB", "yue"] {
            assert_eq!(Language::parse(spelling), None, "{spelling}");
        }
    }
}
