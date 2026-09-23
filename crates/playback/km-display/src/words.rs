//! What the television says, and the message ids for saying it.
//!
//! **Separate from [`crate::text`], which is about fonts.** That module opens a file and measures
//! glyphs; this one holds words and never touches SDL — which is what lets every test here run on a
//! machine with no display, under the rule that no test in this crate may open a font.
//!
//! The two do meet at one point, and it is the interesting one: a catalog may only contain
//! characters the bundled font can draw. See [`is_drawable`].
//!
//! # What is not here
//!
//! Sentences the caller resolves. `Frame::demo`, `Frame::soundfont_label` and `SongInfo::language`
//! arrive already worded, from a crate that knows what a demo and a SoundFont are — this one does
//! not, and gains nothing by learning. Their catalogs live with their callers.
//!
//! `Frame::faults` **was** one of them and is not any more, which is the useful thing to know about
//! where that line falls: while the screen quoted a package's reason it needed a crate that knew
//! what a package was, so the sentence was resolved outside and arrived in English whatever the
//! machine's locale said. A count and an area are a number and an enum, so the words came here and
//! the television stopped having one line that ignored its own locale setting.

use std::sync::OnceLock;

use km_locale::{Catalog, Locale};

/// The screen's own words, one catalog per locale.
///
/// `include_str!` rather than a file beside the binary, per `Bundling assets`: this crate ends up
/// inside a `.deb`, a macOS bundle and an APK, and a catalog that failed to travel would leave a
/// television reading `⟦idle-prompt⟧` with no way to say why.
const CATALOGS: &[(Locale, &str)] = &[
    (Locale::English, include_str!("../i18n/en.ftl")),
    (
        Locale::BrazilianPortuguese,
        include_str!("../i18n/pt-BR.ftl"),
    ),
];

/// The screen's messages for one locale, parsed once.
#[must_use]
pub fn messages(locale: Locale) -> &'static Catalog {
    static PARSED: OnceLock<Vec<(Locale, Catalog)>> = OnceLock::new();
    let parsed = PARSED.get_or_init(|| {
        CATALOGS
            .iter()
            .map(|(locale, source)| {
                let catalog = Catalog::new(*locale, source)
                    // Compiled in, so this cannot be caused by anything at run time.
                    // `every_catalog_parses` is what turns it into a build failure.
                    .unwrap_or_else(|errors| {
                        panic!("{locale} display catalog: {}", errors.join("; "))
                    });
                (*locale, catalog)
            })
            .collect()
    });
    parsed
        .iter()
        .find(|(candidate, _)| *candidate == locale)
        .map(|(_, catalog)| catalog)
        .expect("every locale has a display catalog")
}

/// Characters outside Latin-1 that this screen draws and the bundled font has pictures for.
///
/// **The rule is font coverage, and `is_ascii()` is not it.** The screen already draws `·` and
/// `…` — the first inside Latin-1, the second not — so the honest rule is Latin-1 plus a named
/// handful, and naming them is what makes adding a twelfth one a deliberate act.
///
/// A glyph the font lacks does not fail; it draws as a blank box, which on a television reads as a
/// fault in the machine.
pub const DRAWABLE_EXTRAS: &[char] = &[
    '\u{2026}', // … the ellipsis a cut line ends with
    '\u{2014}', // — the em dash that joins two clauses on the idle prompt
    '\u{2013}', // – an en dash in a range
    '\u{2018}', // ' '
    '\u{2019}', // ' the apostrophe a word processor produces
    '\u{201c}', // " "
    '\u{201d}', // "
];

/// Whether the bundled font can be relied on to draw a character.
///
/// Latin-1 covers every letter both shipped locales need — Portuguese adds only `ãáâàçéêíóôõúü`,
/// all of which are in it — plus [`DRAWABLE_EXTRAS`].
#[must_use]
pub fn is_drawable(character: char) -> bool {
    character as u32 <= 0xff || DRAWABLE_EXTRAS.contains(&character)
}

/// Whether a character needs a CJK face to draw.
///
/// **This is the trigger for opening one, so it is a question about blocks and not about coverage.**
/// The wrong test is "is this character outside Latin-1": Czech and Polish titles are full of Latin
/// Extended-A, every font this product reaches already draws them, and treating one as a reason to
/// open twelve more faces would put the memory cost on the machines least likely to need it.
///
/// Deliberately not exhaustive. Ideographic Extension B and beyond, and the Kana Supplement, are
/// rare enough in a corpus of song titles that the first one seen can go on being a box until
/// somebody reports it — where a list long enough to be sure would be a list nobody checks.
#[must_use]
pub fn is_cjk(character: char) -> bool {
    matches!(character as u32,
        0x1100..=0x11FF     // Hangul Jamo
        | 0x3000..=0x303F   // CJK symbols and punctuation, the ideographic space included
        | 0x3040..=0x309F   // Hiragana
        | 0x30A0..=0x30FF   // Katakana
        | 0x3400..=0x4DBF   // CJK unified ideographs, extension A
        | 0x4E00..=0x9FFF   // CJK unified ideographs
        | 0xAC00..=0xD7AF   // Hangul syllables
        | 0xF900..=0xFAFF   // CJK compatibility ideographs
        | 0xFF00..=0xFFEF   // Halfwidth and fullwidth forms
    )
}

/// Whether any character in this text needs a CJK face.
#[must_use]
pub fn needs_cjk(text: &str) -> bool {
    text.chars().any(is_cjk)
}

// -- Message ids ---------------------------------------------------------------------------------
//
// **Constants rather than string literals at the call sites.** A key is looked up at run time and a
// typo in one renders `⟦typo⟧` rather than failing to compile, so naming them here is what turns a
// rename into a compiler error at every site but the catalog itself. `every_id_is_in_the_catalog`
// closes the last gap.

/// `CLR` on the number pad.
pub const KEYPAD_CLEAR: &str = "keypad-clear";
/// `OK` on the number pad.
pub const KEYPAD_SUBMIT: &str = "keypad-submit";

/// The prompt under the title on the idle screen.
pub const IDLE_PROMPT: &str = "idle-prompt";
/// What a machine with nothing installed says.
pub const CATALOG_EMPTY: &str = "catalog-empty";
/// How much is installed.
pub const CATALOG_SUMMARY: &str = "catalog-summary";

/// How many standing faults there are, and which areas they fall in.
pub const NOTICE_FAULTS: &str = "notice-faults";
/// The area name for a package that would not install.
pub const FAULT_PACKAGES: &str = "fault-packages";
/// The area name for a fault with the sound.
pub const FAULT_SOUND: &str = "fault-sound";

/// The marker for a machine in debugging mode.
pub const DEVELOPER_DEBUGGING: &str = "developer-debugging";
/// The marker for a machine serving the development console, which also has debugging on.
pub const DEVELOPER_CONSOLE: &str = "developer-console";

/// The queue overlay's heading.
pub const QUEUE_HEADING: &str = "queue-heading";
/// How many songs are waiting.
pub const QUEUE_WAITING: &str = "queue-waiting";
/// What the overlay says with an empty queue.
pub const QUEUE_EMPTY: &str = "queue-empty";
/// The reserved last row when the queue does not fit.
pub const QUEUE_OVERFLOW: &str = "queue-overflow";

/// What is queued after the song playing.
pub const NEXT_UP: &str = "next-up";
/// A MIDI file that carries no words.
pub const NO_LYRICS: &str = "no-lyrics";

/// The transposition badge.
pub const BADGE_KEY: &str = "badge-key";
/// The tempo badge.
pub const BADGE_TEMPO: &str = "badge-tempo";
/// The guide-melody badge.
pub const BADGE_MELODY: &str = "badge-melody";
/// The badge over a song whose words are turned off.
///
/// **A different message from [`NO_LYRICS`] although both read as *no lyrics*, and the ids are what
/// keep them apart.** That one is said about a file with no words in it; this one is said about a
/// decision taken over a file that has some. A translator seeing one id would have to write one
/// sentence for two facts.
pub const BADGE_LYRICS_HIDDEN: &str = "badge-lyrics-hidden";

/// A number that is not a song number at all.
pub const NUMBER_INVALID: &str = "number-invalid";

/// The frame meter's heading.
pub const FRAMES_HEADING: &str = "frames-heading";
/// What the frame meter says before it has enough samples.
pub const FRAMES_MEASURING: &str = "frames-measuring";

/// The frame meter's row labels.
///
/// **A slice rather than nine constants**, because these are only ever used as a table: each is
/// written beside the measurement it names, in the loop that draws the rows, and none is looked up
/// on its own. Listing them here is still what `no_message_is_left_unused` reads.
pub const FRAMES_ROWS: &[&str] = &[
    "frames-draw",
    "frames-present",
    "frames-interval",
    "frames-starved",
    "frames-dropped",
    "frames-late",
    "frames-xruns",
];

/// The song block's heading, and the kind of file it names.
pub const SONG_HEADING: &str = "song-heading";
/// A MIDI song, with how many tracks the file holds.
pub const SONG_KIND_MIDI: &str = "song-kind-midi";
/// A video song.
pub const SONG_KIND_VIDEO: &str = "song-kind-video";
/// An MP3+G song.
pub const SONG_KIND_CDG: &str = "song-kind-cdg";
/// The diagnostic panel's name for an UltraStar song.
pub const SONG_KIND_ULTRASTAR: &str = "song-kind-ultrastar";
/// The diagnostic panel's name for an LRC song.
pub const SONG_KIND_LRC: &str = "song-kind-lrc";

/// A media song levelled against the loudness its package measured.
pub const SONG_LEVELLED_PACKAGE: &str = "song-levelled-package";
/// A MIDI song levelled from a reading of its own events.
pub const SONG_LEVELLED_EVENTS: &str = "song-levelled-events";
/// Levelling switched off for this kind of song.
pub const SONG_LEVELLED_OFF: &str = "song-levelled-off";
/// Nothing to level this song with.
pub const SONG_LEVELLED_NONE: &str = "song-levelled-none";

/// A song carrying no stored correction at all.
pub const SONG_FIXES_NONE: &str = "song-fixes-none";

/// Words found as Soft Karaoke.
pub const SONG_FLAVOR_SOFT_KARAOKE: &str = "song-flavor-soft-karaoke";
/// Words found in `Lyric` meta events.
pub const SONG_FLAVOR_LYRIC_EVENTS: &str = "song-flavor-lyric-events";
/// Words found as text on a track named for them.
pub const SONG_FLAVOR_NAMED_TEXT_TRACK: &str = "song-flavor-named-text-track";
/// A playable file with no words in it.
pub const SONG_FLAVOR_NONE: &str = "song-flavor-none";

/// How far into the song playback has reached, against its length.
pub const SONG_POSITION: &str = "song-position";
/// The gain in force on the song.
pub const SONG_GAIN: &str = "song-gain";
/// Where that gain came from.
pub const SONG_LEVELLED: &str = "song-levelled";
/// The corrections applied to the song's own events.
pub const SONG_FIXES: &str = "song-fixes";
/// The convention the words were found in.
pub const SONG_LYRICS: &str = "song-lyrics";
/// What the parser could not read.
pub const SONG_DAMAGE: &str = "song-damage";

/// The tags counting each kind of stored correction, and each kind of damage.
///
/// Slices rather than constants, for [`FRAMES_ROWS`]' reason: each is only ever written beside the
/// count it names, in the loop that builds its row. Both loops drop the zeroes, so a row says only
/// what is actually true of the file.
pub const SONG_FIX_TAGS: &[&str] = &["song-fix-bank", "song-fix-mute"];
/// The damage tags, in the order the row draws them.
pub const SONG_DAMAGE_TAGS: &[&str] = &["song-damage-cut", "song-damage-gone", "song-damage-notes"];

/// The connect panel's heading.
pub const CONNECT_HEADING: &str = "connect-heading";
/// A machine whose remote is reachable only from itself.
pub const CONNECT_LOCAL_ONLY: &str = "connect-local-only";
/// A machine with no network at all.
pub const CONNECT_NO_NETWORK: &str = "connect-no-network";
/// What to do about having no network.
pub const CONNECT_NO_NETWORK_DETAIL: &str = "connect-no-network-detail";
/// A remote that cannot be offered, for a reason with no better sentence.
pub const CONNECT_UNAVAILABLE: &str = "connect-unavailable";
/// What to do about a machine listening only on loopback.
pub const CONNECT_LOCAL_ONLY_DETAIL: &str = "connect-local-only-detail";
/// No address worth printing was found.
pub const CONNECT_NO_ADDRESS: &str = "connect-no-address";
/// The generated PIN, drawn while it is still the password in force.
pub const CONNECT_FACTORY_PIN: &str = "connect-factory-pin";
/// How many other addresses the machine answers on.
pub const CONNECT_OTHER_ADDRESSES: &str = "connect-other-addresses";
/// The key that opens the remote in this computer's own browser.
pub const CONNECT_BROWSER_KEY: &str = "connect-browser-key";
/// The same sentence for a platform that only reaches the key with Control held.
pub const CONNECT_BROWSER_KEY_CTRL: &str = "connect-browser-key-ctrl";

/// Every id above, for the test that checks each is in the catalog.
///
/// Written out rather than derived, because a constant nothing lists is a constant nothing checks.
pub const ALL_IDS: &[&str] = &[
    KEYPAD_CLEAR,
    KEYPAD_SUBMIT,
    IDLE_PROMPT,
    CATALOG_EMPTY,
    CATALOG_SUMMARY,
    NOTICE_FAULTS,
    FAULT_PACKAGES,
    FAULT_SOUND,
    DEVELOPER_DEBUGGING,
    DEVELOPER_CONSOLE,
    QUEUE_HEADING,
    QUEUE_WAITING,
    QUEUE_EMPTY,
    QUEUE_OVERFLOW,
    NEXT_UP,
    NO_LYRICS,
    BADGE_KEY,
    BADGE_TEMPO,
    BADGE_MELODY,
    BADGE_LYRICS_HIDDEN,
    NUMBER_INVALID,
    FRAMES_HEADING,
    FRAMES_MEASURING,
    SONG_HEADING,
    SONG_KIND_MIDI,
    SONG_KIND_VIDEO,
    SONG_KIND_CDG,
    SONG_KIND_ULTRASTAR,
    SONG_KIND_LRC,
    SONG_LEVELLED_PACKAGE,
    SONG_LEVELLED_EVENTS,
    SONG_LEVELLED_OFF,
    SONG_LEVELLED_NONE,
    SONG_POSITION,
    SONG_GAIN,
    SONG_LEVELLED,
    SONG_FIXES,
    SONG_LYRICS,
    SONG_DAMAGE,
    SONG_FIXES_NONE,
    SONG_FLAVOR_SOFT_KARAOKE,
    SONG_FLAVOR_LYRIC_EVENTS,
    SONG_FLAVOR_NAMED_TEXT_TRACK,
    SONG_FLAVOR_NONE,
    CONNECT_HEADING,
    CONNECT_LOCAL_ONLY,
    CONNECT_LOCAL_ONLY_DETAIL,
    CONNECT_NO_NETWORK,
    CONNECT_NO_NETWORK_DETAIL,
    CONNECT_UNAVAILABLE,
    CONNECT_NO_ADDRESS,
    CONNECT_FACTORY_PIN,
    CONNECT_OTHER_ADDRESSES,
    CONNECT_BROWSER_KEY,
    CONNECT_BROWSER_KEY_CTRL,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_parses() {
        for locale in Locale::ALL {
            assert!(!messages(*locale).keys().is_empty(), "{locale}");
        }
    }

    #[test]
    fn every_message_is_translated() {
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
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let extra = english.missing_from(messages(*locale));
            assert!(
                extra.is_empty(),
                "{locale} has keys nothing asks for: {extra:?}"
            );
        }
    }

    #[test]
    fn every_id_is_in_the_catalog() {
        // The half a constant cannot check: the name compiles, and the catalog may still not have
        // it. Without this a renamed key reaches a television as `⟦badge-tempo⟧`.
        let english = messages(Locale::English);
        for id in ALL_IDS {
            assert!(english.keys().contains(*id), "no message called `{id}`");
        }
    }

    #[test]
    fn no_message_is_left_unused() {
        // Read backwards: a key nothing looks up is a leftover from a rename, sitting there looking
        // like work for whoever translates next. The transport strip's ids live in
        // `input::TRANSPORT_COMMANDS` rather than in `ALL_IDS`, so they are gathered from there.
        let listed: Vec<&str> = ALL_IDS
            .iter()
            .copied()
            .chain(
                crate::input::TRANSPORT_COMMANDS
                    .iter()
                    .map(|command| command.label_id),
            )
            .chain(FRAMES_ROWS.iter().copied())
            .chain(SONG_FIX_TAGS.iter().copied())
            .chain(SONG_DAMAGE_TAGS.iter().copied())
            .collect();
        for key in messages(Locale::English).keys() {
            assert!(
                listed.contains(&key.as_str()),
                "`{key}` is in the catalog and nothing asks for it"
            );
        }
    }

    #[test]
    fn every_message_is_drawable() {
        // The bundled font is Latin coverage only, so a character outside it draws as a blank box —
        // which on a television reads as a fault in the machine rather than as a missing glyph.
        // Stronger than the `is_ascii()` assertion it replaces: that one saw the labels, this sees
        // every word on the screen in every locale.
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in catalog.keys() {
                let rendered = catalog.msg(key);
                for character in rendered.chars() {
                    assert!(
                        is_drawable(character),
                        "{locale} `{key}`: {character:?} has no picture in the bundled font"
                    );
                }
            }
        }
    }

    #[test]
    fn portuguese_needs_no_glyph_latin_1_does_not_have() {
        // The claim `Non-Latin text` rests on: shipping a second locale changed nothing about the
        // font, because Portuguese adds only accented Latin letters.
        for character in "ãáâàçéêíóôõúüÃÁÂÀÇÉÊÍÓÔÕÚÜ".chars() {
            assert!(
                (character as u32) <= 0xff,
                "{character:?} is outside Latin-1"
            );
        }
    }

    /// The three scripts, in the words a corpus file would actually carry.
    #[test]
    fn han_kana_and_hangul_all_ask_for_a_cjk_face() {
        for text in ["こんにちは", "世界", "안녕하세요", "你好", "カタカナ"] {
            assert!(needs_cjk(text), "{text} needs a CJK face");
        }
    }

    /// **The test that pays for `is_cjk` being about blocks rather than about Latin-1.**
    ///
    /// A Czech or Polish title is full of Latin Extended-A, every font this product reaches draws
    /// it, and treating one as a reason to open twelve more faces would put the cost on the machines
    /// least likely to need it — and there are nine times as many of those files in the corpus.
    #[test]
    fn latin_however_accented_never_asks_for_one() {
        for text in [
            "Příliš žluťoučký kůň",
            "Łódź",
            "Tükörfúrógép",
            "Águas de Março",
            "Coração",
            "İstanbul",
            "plain ascii",
            "",
        ] {
            assert!(!needs_cjk(text), "{text} must not open a CJK font");
        }
    }

    /// Every message the screen ships stays clear of the trigger.
    ///
    /// The twin of `every_message_is_drawable`: that one says the catalogs need no glyph the font
    /// lacks, and this one says drawing them can never be what opens a CJK face. A locale that broke
    /// this would have every machine loading Japanese to render its own furniture.
    #[test]
    fn no_message_in_any_catalog_asks_for_a_cjk_face() {
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in catalog.keys() {
                let rendered = catalog.msg(key);
                assert!(
                    !needs_cjk(&rendered),
                    "{locale} `{key}` would open a CJK font: {rendered:?}"
                );
            }
        }
    }
}
