//! Working out where a file keeps its lyrics, and normalizing them.
//!
//! There is no single karaoke MIDI format. Three conventions cover essentially everything in
//! circulation, and they are tried in order of how much they tell us:
//!
//! 1. **Soft Karaoke** (`.kar`) — announced by a `@KMIDI KARAOKE FILE` text event, with `@`-prefixed
//!    header lines on one track and the lyrics as plain *Text* events on another.
//! 2. **Standard MIDI karaoke** — lyrics in dedicated *Lyric* meta events (`0x05`).
//! 3. **Named text track** — *Text* events on a track called `Words`, `Lyrics` or similar.
//!
//! Whatever the source, the output is the same normalized [`LyricTimeline`], so nothing downstream
//! has to care which convention a song came from.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::encoding::TextDecoder;
use crate::timeline::{LineBreak, LineInference, LyricTimeline, RawSyllable, build_timeline};

/// Which text-bearing meta event a payload came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaTextKind {
    /// A generic text event (`0x01`).
    Text,
    /// A lyric event (`0x05`).
    Lyric,
    /// A track name (`0x03`).
    TrackName,
    /// A copyright notice (`0x02`).
    Copyright,
    /// A marker (`0x06`).
    Marker,
}

/// One text-bearing meta event with its absolute position.
#[derive(Debug, Clone, Copy)]
pub struct MetaText<'a> {
    /// Index of the track it appeared on.
    pub track: usize,
    /// Absolute tick.
    pub tick: u32,
    /// Which kind of meta event carried it.
    pub kind: MetaTextKind,
    /// The undecoded payload.
    pub bytes: &'a [u8],
}

/// The convention a song's lyrics were found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KaraokeFlavor {
    /// Soft Karaoke, as produced by Tune 1000 and everything that followed it.
    SoftKaraoke,
    /// Lyrics in `Lyric` meta events.
    LyricEvents,
    /// Lyrics as `Text` events on a track named for them.
    NamedTextTrack,
    /// No lyrics found. The song is still playable.
    None,
}

/// Song identification recovered from the file's own metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KaraokeMeta {
    /// Song title, if the file states one.
    pub title: Option<String>,
    /// Performer, if the file states one.
    pub artist: Option<String>,
    /// Copyright notice.
    pub copyright: Option<String>,
    /// Language tag as written in the file, for example `ENGL`.
    pub language: Option<String>,
    /// Soft Karaoke format version, for example `0100`.
    pub version: Option<String>,
    /// Free-form information lines.
    pub info: Vec<String>,
}

/// Everything extracted from a file's text events.
#[derive(Debug, Clone)]
pub struct KaraokeSource {
    /// Which convention the lyrics were found in.
    pub flavor: KaraokeFlavor,
    /// The normalized lyric timeline.
    pub timeline: LyricTimeline,
    /// Identification recovered from the file.
    pub meta: KaraokeMeta,
    /// The encoding decision applied to every text payload.
    pub decoder: TextDecoder,
    /// What this file's own habits said about how it writes lyrics.
    pub dialect: Dialect,
}

const SOFT_KARAOKE_MAGIC: &str = "@KMIDI KARAOKE FILE";

/// Extracts and normalizes the lyrics of one file.
///
/// `declared_encoding` comes from the package manifest when the song is played from a package, and
/// overrides detection. `tick_to_ms` is only consulted for the gap heuristic on files that give no
/// line markers.
pub fn extract(
    events: &[MetaText<'_>],
    inference: LineInference,
    tick_to_ms: impl Fn(u32) -> u32,
    declared_encoding: Option<&str>,
) -> KaraokeSource {
    // One encoding decision for the whole file, made from every text payload it has.
    let samples: Vec<&[u8]> = events.iter().map(|e| e.bytes).collect();
    let decoder = TextDecoder::resolve(&samples, declared_encoding);

    let decoded: Vec<(usize, u32, MetaTextKind, String)> = events
        .iter()
        .map(|e| (e.track, e.tick, e.kind, decoder.decode(e.bytes)))
        .collect();

    let mut meta = KaraokeMeta::default();

    // --- Flavor 1: Soft Karaoke -------------------------------------------------------------
    let magic_track = decoded
        .iter()
        .find(|(_, _, kind, text)| *kind == MetaTextKind::Text && text.trim() == SOFT_KARAOKE_MAGIC)
        .map(|(track, _, _, _)| *track);

    if let Some(info_track) = magic_track {
        read_soft_karaoke_header(&decoded, &mut meta);
        let lyric_track = pick_soft_karaoke_lyric_track(&decoded, info_track);
        let (raws, dialect) = collect_raws(&decoded, |track, kind, text| {
            *kind == MetaTextKind::Text && *track == lyric_track && !text.starts_with('@')
        });
        if !raws.is_empty() {
            // The `@` header carries no copyright line, and a Soft Karaoke file can still have a
            // real Copyright meta event. Only unset fields are filled, so `@T` keeps precedence.
            fill_generic_meta(&decoded, &mut meta);
            return KaraokeSource {
                flavor: KaraokeFlavor::SoftKaraoke,
                timeline: build_timeline(raws, inference, tick_to_ms),
                meta,
                decoder,
                dialect,
            };
        }
        // A file can announce itself as Soft Karaoke and still carry no lyrics; fall through and
        // give the other conventions a chance rather than reporting an empty Soft Karaoke song.
    }

    // --- Flavor 2: Lyric meta events ---------------------------------------------------------
    let tab_tracks = harmonica_tab_tracks(&decoded);
    let (raws, mut dialect) = collect_raws(&decoded, |track, kind, _| {
        *kind == MetaTextKind::Lyric && !tab_tracks.contains(track)
    });
    dialect.harmonica_tabs |= !tab_tracks.is_empty();
    if !raws.is_empty() {
        fill_generic_meta(&decoded, &mut meta);
        return KaraokeSource {
            flavor: KaraokeFlavor::LyricEvents,
            timeline: build_timeline(raws, inference, tick_to_ms),
            meta,
            decoder,
            dialect,
        };
    }

    // --- Flavor 3: a track named for its words ------------------------------------------------
    if let Some(track) = find_lyric_named_track(&decoded) {
        let (raws, dialect) = collect_raws(&decoded, |t, kind, text| {
            *kind == MetaTextKind::Text && *t == track && !text.starts_with('@')
        });
        if !raws.is_empty() {
            fill_generic_meta(&decoded, &mut meta);
            return KaraokeSource {
                flavor: KaraokeFlavor::NamedTextTrack,
                timeline: build_timeline(raws, inference, tick_to_ms),
                meta,
                decoder,
                dialect,
            };
        }
    }

    fill_generic_meta(&decoded, &mut meta);
    KaraokeSource {
        flavor: KaraokeFlavor::None,
        timeline: LyricTimeline::default(),
        meta,
        decoder,
        // A file with no lyrics has no habits about how it writes them.
        dialect: Dialect::default(),
    }
}

/// Reads the `@`-prefixed header lines of a Soft Karaoke file.
///
/// The convention is positional: the first `@T` line is the title, the second the performer, and
/// any further ones are extra credits, which we keep as info rather than inventing fields for.
///
/// Every track is scanned, not just the one announcing the format. Real files routinely put
/// `@KMIDI KARAOKE FILE` alone on a track called `Soft karaoke` and the rest of the header —
/// `@L`, `@T`, `@I` — at the top of the *Words* track alongside the lyrics. A `@`-prefixed text
/// event is a control line wherever it appears, and lyrics starting with `@` are excluded from the
/// lyric stream anyway, so scanning everywhere is both safer and consistent.
fn read_soft_karaoke_header(
    decoded: &[(usize, u32, MetaTextKind, String)],
    meta: &mut KaraokeMeta,
) {
    let mut titles = Vec::new();
    for (_, _, kind, text) in decoded {
        if *kind != MetaTextKind::Text {
            continue;
        }
        let text = text.trim_end_matches(['\r', '\n']);
        if text.trim() == SOFT_KARAOKE_MAGIC {
            continue;
        }
        match text.as_bytes().first() {
            Some(b'@') => {}
            _ => continue,
        }
        let (tag, value) = text.split_at(2.min(text.len()));
        // Cleaned here rather than at each assignment, so `looks_like_a_credit` and the emptiness
        // test below both judge the string a person would actually see.
        let Some(value) = clean_meta_text(value) else {
            continue;
        };
        match tag {
            "@T" => titles.push(value),
            "@I" => meta.info.push(value),
            "@L" => meta.language = Some(value),
            "@V" => meta.version = Some(value),
            _ => {}
        }
    }
    // A separator row is dropped before the positions are counted, so the `@T` under an ornament is
    // the title rather than the artist. Unlike a credit it is kept nowhere: a credit is often the
    // only record of where a file came from, and a row of marks is a record of nothing.
    titles.retain(|value| names_something(value));
    // Credits are separated out *before* anything is assigned, because they are not reliably in
    // second position. Building a real package exposed files whose very first `@T` is the producer
    // ("Karaokê do Brasil - Família Ribeiro - 2000"), which put a studio's name in the title of
    // hundreds of songs. Title and artist are then the first two lines that are not credits.
    let (credits, names): (Vec<String>, Vec<String>) = titles
        .into_iter()
        .partition(|value| looks_like_a_credit(value));

    let mut names = names.into_iter();
    meta.title = names.next();
    meta.artist = names.next();
    // Anything further, plus every credit, is kept as information rather than discarded: it is often
    // the only record of where a file came from.
    meta.info.extend(names);
    meta.info.extend(credits);
}

/// A metadata string with the bytes nobody can read taken out, or `None` if nothing survives.
///
/// **The gate this replaces was `!text.trim().is_empty()`, and it does not hold.** `str::trim`
/// removes Unicode whitespace and nothing else -- not NUL, not `\x01`, not DEL -- so a title made
/// entirely of padding passed it and became the song's name. A great deal of software writes MIDI
/// text events as fixed-length fields, so a track name arrives NUL-padded: 1,467 files in the local
/// corpus carry control characters in their title, and 179 of those are nothing else.
///
/// That is not a cosmetic problem, because **NUL sorts before every printable character**. Those 179
/// rows took the whole first page of a title-ordered browse, each drawn as an empty and unclickable
/// link -- the same blank first page `songs.stem` was added to cure, arriving by a second route.
///
/// Line breaks become spaces rather than vanishing, so a two-line `@T` reads as one sentence rather
/// than one run-together word; the runs of whitespace that makes then collapse. For
/// [`KaraokeMeta`] only -- lyric text has [`clean_lyric_text`], which runs after a carriage return
/// has been read as the line marker the timeline is built out of.
pub fn clean_meta_text(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' | '\n' | '\r' => out.push(' '),
            _ if ch.is_control() => {}
            _ => out.push(ch),
        }
    }
    // `split_whitespace` collapses the runs the mapping above can leave behind, and trims both ends.
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// A name with the bytes nobody can read taken out, or `None` if what is left names nothing.
///
/// [`clean_meta_text`] and one more test, for the two fields that become the name of a song. A title
/// or an artist is a thing somebody can ask for by name, and a row of marks is not one.
///
/// **Every other metadata field keeps the looser gate**, which is why this is a second function
/// rather than a stricter [`clean_meta_text`]: a copyright line that is only `©` is a notice, `@V`
/// is a version, and neither has to name anything.
pub fn clean_meta_name(text: &str) -> Option<String> {
    clean_meta_text(text).filter(|value| names_something(value))
}

/// Whether a title or an artist names something, rather than being a row of marks.
///
/// **Two letters or digits, anywhere in it.** A corpus carries separator rows in its title meta
/// events -- `====================`, `<>-<>-<>-<>`, `****`, `???`, `-` and `.` -- written by whoever
/// typed the file, and taken as a title they sort to the front of a title-ordered browse and fill
/// the first pages somebody curating ever sees. The same shape as the padding [`clean_meta_text`]
/// answers, arriving by a different route: a string that survives cleaning and still says nothing.
///
/// **Two rather than one, which is what makes the marks around a name harmless.** `***** I Love
/// You *****`, `----- TAKE FIVE -----` and `- - -X-FILES- - -` are real titles somebody framed, and
/// a rule counting marks against letters refuses all three -- 203 of them in the local corpus. What
/// a frame has inside it is the whole question, and a count of letters cannot see a frame, so
/// decoration costs a name nothing. `====== X ======` and `** 2` still go, because one character is
/// what a frame with nothing in it has inside.
///
/// **So a name of a single character is refused**, and that is the same call
/// [`looks_like_a_banner`] makes one column over: `a`, `w` and `{M}` are an event that escaped
/// rather than a word. Here it takes `1` through `9`, `A`, `D#` and `*2` -- 629 rows of track index
/// and key signature, and no song's name among them.
///
/// **Except where the one character is a word**, which is what [`is_a_word_on_its_own`] answers:
/// `虹` is a title and `A` is not, because Han and Hangul write in one character what a Latin script
/// spells out in five. Seven rows of the local corpus, and each of them a real song.
///
/// [`char::is_alphanumeric`] rather than its ASCII twin, so `Águas de Março` and a Japanese title
/// are names.
fn names_something(value: &str) -> bool {
    value.chars().any(is_a_word_on_its_own)
        || value.chars().filter(|ch| ch.is_alphanumeric()).count() >= 2
}

/// Whether one character carries a word by itself.
///
/// Han, Kana and Hangul write in one or two characters what a Latin script spells out in several, so
/// the one-character rule above would refuse `虹`, `脈` and `비` -- rainbow, pulse and rain, and all
/// three real titles in the local corpus.
///
/// **Not `km_display::is_cjk`, and not only because of the layering.** That one decides whether a
/// *font* has to be opened, so it is deliberately a question about blocks: it takes in the
/// ideographic punctuation and the halfwidth and fullwidth forms, and an ideographic comma is no more
/// a word than a Latin one is. Two questions, two lists, and the comment each carries is what keeps
/// them from being merged by somebody who notices they overlap.
fn is_a_word_on_its_own(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x309F   // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3400..=0x4DBF // CJK unified ideographs, extension A
        | 0x4E00..=0x9FFF // CJK unified ideographs
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
    )
}

/// A syllable with the bytes nobody can sing taken out.
///
/// **Padding reaches the words as well as the titles**, and from the same fixed-length text fields:
/// a writer that keeps `%SOL` in a five-byte field puts the terminator in the lyric event with it,
/// so the syllable arrives as `%SOL\0`. A NUL is not a character anybody sings, and neither is any
/// other control character, so they go.
///
/// **Left in, one of them ends the process.** SDL_ttf is asked for a C string, and a Rust string
/// with a NUL inside it cannot become one; the failure lands on the display thread, which is the
/// one thread whose panic is fatal. [`crate::spacing`] and the break markers are both read before
/// this runs, so everything still here is either text or is nothing -- and a syllable that is
/// nothing is dropped by the caller, exactly as a bare space mark is.
///
/// Taken by value because almost every syllable has nothing to remove, and `String::retain` rewrites
/// in place rather than building a second string for them.
pub fn clean_lyric_text(mut text: String) -> String {
    text.retain(|ch| !ch.is_control());
    text
}

/// Whether a `@T` line reads as a credit rather than a song title or a performer.
fn looks_like_a_credit(value: &str) -> bool {
    const PHRASES: [&str; 16] = [
        "karaoke by",
        "kar by",
        "midi by",
        "sequenced by",
        "sequence by",
        "arranged by",
        "transcribed by",
        "created by",
        "produced by",
        "www.",
        // `by:` with a colon is a credit's punctuation and not a sentence's -- `exclusive by: tomson`
        // opens 88 files in the corpus and matched none of the phrases above, because the verb in
        // front of it is not one anybody could enumerate.
        "by:",
        // The same constructions in the two languages half this corpus is in. Spanish and Portuguese
        // put the credit after `por`, so an English-only list missed them entirely: `Trabajo
        // realizado por` and `Editado por "GLOMAR"` between them open 223 files.
        "realizado por",
        "editado por",
        "produzido por",
        "gravado por",
        "arranjo de",
    ];
    // `karaok` covers karaoke, karaokê and karaoké. A song is very unlikely to be *called* that,
    // while producers put it in their own name constantly -- so on this corpus the word alone is a
    // reliable signal, and it was worth adding: without it a studio name became the title.
    const WORDS: [&str; 2] = ["karaok", "sequenc"];

    let lower = value.to_lowercase();
    PHRASES.iter().any(|phrase| lower.contains(phrase))
        || WORDS.iter().any(|word| lower.contains(word))
}

/// The publisher's notice, as marks a line may contain.
///
/// **`rights reserved` and `rights secured` are deliberately not anchored to `all`**, and
/// `transmission of any kind` is here because it is a *continuation*: several hundred files open
/// with `ALL rights reserved. Not for broadcast or` and wrap the sentence onto the next line, so
/// catching only the first line let the second through as the preview. 97 files in 30,000.
const LEGAL: [&str; 10] = [
    "©",
    "(c)",
    "copyright",
    "rights reserved",
    "rights secured",
    "todos os direitos",
    "not for broadcast",
    "transmission of any kind",
    "do not duplicate",
    "not for rental",
];

/// Whether a line is a publisher's notice and nothing else.
///
/// **A third question, and the narrowest of the three.** [`looks_like_a_banner`] asks whether a
/// *leading* line is worth showing, where being wrong costs one line of a preview;
/// `km_suitability`'s `is_credit_line` asks whether a line counts towards how much lyric a file has.
/// This one asks whether a line may be thrown away entirely, anywhere in a song, which is the
/// widest consequence of the three and so has to be the tightest test.
///
/// So it is not [`LEGAL`] as a `contains`. A mark has to be there, and then **every word of the
/// line has to be a word a notice is made of**: the marks themselves, the words that join them in
/// the notices this corpus carries, and a year. `Copyright 1994 Some Publisher` keeps a publisher,
/// so it stays a banner and is not this; `DO NOT DUPLICATE. NOT FOR RENTAL.` keeps nothing, and
/// nobody sings it.
///
/// **Word by word and never by substring**, because taking `or` out of `world` is how a rule like
/// this quietly eats a verse. The joining words are only ever reached on a line that already holds
/// a mark, so `and` on its own is still a lyric.
///
/// **The mark is looked for in the lower-cased line rather than the folded one.** Folding maps
/// punctuation to spaces, so `©` folds to nothing at all and `(c)` to a bare `c` — and a `contains`
/// against an empty string is true of every line there has ever been.
#[must_use]
pub fn is_only_a_legal_notice(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    if lower.is_empty() {
        return false;
    }
    if !LEGAL.iter().any(|mark| lower.contains(mark)) {
        return false;
    }

    let folded = crate::text::fold(line);
    let mut allowed: Vec<&str> = Vec::new();
    let marks: Vec<String> = LEGAL.iter().map(|mark| crate::text::fold(mark)).collect();
    for mark in &marks {
        allowed.extend(mark.split(' ').filter(|word| !word.is_empty()));
    }
    // What joins the marks in the notices the corpus actually carries, and nothing beyond them.
    // `reservados` is the Portuguese notice's own adjective, which `todos os direitos` stops short
    // of; the rest are the connectives of the English one.
    allowed.extend([
        "all",
        "international",
        "not",
        "for",
        "of",
        "any",
        "kind",
        "or",
        "and",
        "no",
        "reservados",
        "reservadas",
    ]);

    folded
        .split(' ')
        .filter(|word| !word.is_empty())
        .all(|word| allowed.contains(&word) || is_a_year(word))
}

/// Whether a word is a four-digit year, which is part of a notice and not a word of a song.
///
/// Bounded to `19xx` and `20xx` rather than any four digits, so `TUNE 1000 CORP.` still keeps a
/// number the notice cannot account for.
fn is_a_year(word: &str) -> bool {
    word.len() == 4
        && word.chars().all(|c| c.is_ascii_digit())
        && (word.starts_with("19") || word.starts_with("20"))
}

/// Whether a line of *lyrics* is a banner rather than something anybody sings.
///
/// A great many karaoke files open with the sequencer's own advertisement — a studio name, a
/// telephone number, a web address, a row of asterisks — before the words start. The `@`-prefixed
/// Soft Karaoke header never reaches the timeline (see the `accept` closures above), but the same
/// text arrives unfiltered in the `Lyric` flavor, and on a named text track it is simply typed into
/// the words. Anything that shows the *beginning* of a song therefore has to skip past it.
///
/// **This is deliberately not `km_suitability`'s `is_credit_line`, and the two must not be merged.**
/// That one decides whether a line counts towards *how much lyric a file has*, which feeds the
/// 0–10 suitability score — a number stored in every package ever built — so widening it would
/// silently move suitabilities across a whole corpus. This one decides whether a line is worth
/// *showing*, where being wrong costs one line of a preview. Two functions, two blast radii,
/// and that is the whole reason there are two.
///
/// The rules, in the order they are cheapest to check. Each is narrow on purpose: a false positive
/// here skips a real lyric.
///
/// * A producer credit — [`looks_like_a_credit`]'s phrase list, shared rather than restated, plus
///   the two shapes it misses because they carry no "by": `SAROBA PRODUCOES`, `Sincronizado por`.
/// * Contact details: an email address, or a web address.
/// * A copyright or licensing line, including the second half of the wrapped one the commercial
///   discs in this corpus carry.
/// * A label that leaked out of a track name: `Vocals`, `Midi`, `Words`. Exact match only.
/// * An ornament: nothing but punctuation, `****` or `-----`.
/// * A telephone number: seven or more digits, and digits at least half of what is there.
///
/// Note what is **not** here. Nothing judges the language, the capitalisation or the length: a line
/// of a song can be one word, can be shouted in capitals, and can be in any language. Everything
/// below keys on a shape that words do not have.
#[must_use]
pub fn looks_like_a_banner(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_lowercase();

    if looks_like_a_credit(trimmed) {
        return true;
    }

    // A web address. `www.` is already in the phrase list above; the two schemes are not, because a
    // `@T` line carrying one is rarer than a lyric track carrying one.
    if lower.contains("http://") || lower.contains("https://") {
        return true;
    }

    // An email address: an `@` whose next whitespace-delimited token is a dotted host.
    //
    // **The token, not the rest of the line** — that distinction is worth 24 files in 30,000 on its
    // own. `someone@example.com (0**19) 5550123` is the *shape* of one line in the corpus, with the
    // address redacted because it is a stranger's; an earlier version of this rule required the
    // whole remainder to be space-free, so an address with a telephone number after it was not an
    // address.
    if let Some((_, rest)) = lower.split_once('@')
        && let Some(host) = rest.split_whitespace().next()
        && host.contains('.')
        && host.len() > 3
    {
        return true;
    }

    // A copyright or licensing line. `(c)` and `©` are the marks; the phrases are the boilerplate the
    // commercial discs in this corpus actually carry, in the two languages it is actually in.
    //
    // The marks are [`LEGAL`], shared with [`is_only_a_legal_notice`] so the two cannot come to
    // disagree about what a notice is made of.
    if LEGAL.iter().any(|mark| lower.contains(mark)) {
        return true;
    }

    // A label rather than a line: a track name or a section marker that reached the lyric stream.
    // **Exact match, folded**, because the risk of a substring rule here is obvious — a song can
    // certainly contain the word "words". Nobody sings a line that is only the word "Vocals".
    //
    // Folding does the unwrapping for free: it maps punctuation to spaces and trims, so `(Intro)`,
    // `{Words}` and `[ VOCALS ]` all arrive here as the bare word.
    const LABELS: [&str; 22] = [
        "midi",
        "vocals",
        "vocal",
        "vocal line",
        "voice",
        "melody",
        "words",
        "lyrics",
        "lyric",
        "text",
        "track",
        "untitled",
        "intro",
        "outro",
        "chorus",
        "refrao",
        "instrumental",
        // A template somebody never filled in, verbatim. It is **two lines**, which is why both are
        // here: catching `song title` alone simply promoted `artist` to being the preview of the
        // same 110 files. That is the shape of most of what this list has caught — a banner is
        // usually a block, and removing its first line reveals its second.
        "song title",
        "title",
        "artist",
        "song artist",
        "performer",
    ];
    let folded = crate::text::fold(trimmed);
    if LABELS.contains(&folded.as_str()) {
        return true;
    }

    // A stray single character. `a`, `w` and `{M}` open 89 files in 30,000 of the corpus and are an
    // event that escaped rather than a word. A line somebody sings is at least two characters, and
    // the budget above means a file that really is all single letters still gets a preview.
    if folded.chars().count() == 1 {
        return true;
    }

    // A line that is nothing but digits: a disc number, a track index, an identifier. Distinct from
    // the telephone rule below, which needs seven; `7444` and `4266` are four. Deliberately checked
    // before folding turns `1, 2, 3, 4` into digits and spaces — a count-in *is* a line of a song,
    // and it has separators in it where an identifier does not.
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }

    // Producer names in the shapes the phrase list above misses because they carry no "by": a
    // Brazilian studio credit is a noun, `SAROBA PRODUCOES`, and a synchronization credit is
    // `Sincronizado por KIRA`. Folded, so the accents in `produções` do not need a second entry.
    const CREDIT_WORDS: [&str; 2] = ["producoes", "sincroniz"];
    if CREDIT_WORDS.iter().any(|word| folded.contains(word)) {
        return true;
    }

    // An ornament: a separator row somebody typed to frame the credit above or below it. Judged on
    // there being no letter or digit in it at all, so a line of lyrics consisting of "..." or "!!!"
    // is the only false positive available and is not a line anybody sings.
    if !trimmed.chars().any(char::is_alphanumeric) {
        return true;
    }

    // A telephone number. Both halves are needed: the digit count alone catches a lyric about a
    // year, and the ratio alone catches "1999" on a line of its own.
    let digits = trimmed.chars().filter(char::is_ascii_digit).count();
    let solid = trimmed.chars().filter(|ch| !ch.is_whitespace()).count();
    digits >= 7 && digits * 2 >= solid
}

/// Chooses which track holds Soft Karaoke lyrics.
///
/// The name the file gives a track outranks how much text the track holds. A second text track is a
/// second timing of the same words — a working copy, an earlier sync, a translation — and its length
/// says nothing about which of them the file means to sing, so `Words` wins even when it is the
/// shorter. One corpus file carries a 243-syllable `Words` track landing on every melody note and an
/// unnamed 347-syllable track that runs steadily early and stops eleven seconds before the music
/// does; the longer track is the one nobody can sing to.
///
/// Length decides among tracks the name does not separate. Convention puts the words on the track
/// after the header, and files disagree with that often enough that the count is worth more than the
/// order.
fn pick_soft_karaoke_lyric_track(
    decoded: &[(usize, u32, MetaTextKind, String)],
    info_track: usize,
) -> usize {
    let mut counts: Vec<(usize, usize)> = Vec::new();
    for (track, _, kind, text) in decoded {
        if *kind != MetaTextKind::Text || text.starts_with('@') || text.trim().is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(t, _)| t == track) {
            Some((_, n)) => *n += 1,
            None => counts.push((*track, 1)),
        }
    }
    let rank = |(track, n): &(usize, usize)| (names_the_words(decoded, *track), *n);
    // Prefer a track other than the header track, but accept the header track if it is the only
    // one carrying lyrics.
    counts
        .iter()
        .filter(|(track, _)| *track != info_track)
        .max_by_key(|entry| rank(entry))
        .or_else(|| counts.iter().max_by_key(|entry| rank(entry)))
        .map_or(info_track, |(track, _)| *track)
}

/// Is this track named for the words, in the one name Soft Karaoke uses for them?
///
/// `Words` and nothing else. The wider list [`find_lyric_named_track`] accepts takes `melody`, and
/// files carrying both a `Words` and a `Melody` text track are common enough that the wider list
/// separates nothing. Matching whole rather than by prefix is what keeps a credit out: a track named
/// `Words & Music By ...` is somebody's name in the place a name goes, not a lyric stream.
fn names_the_words(decoded: &[(usize, u32, MetaTextKind, String)], track: usize) -> bool {
    decoded.iter().any(|(t, _, kind, name)| {
        *t == track
            && *kind == MetaTextKind::TrackName
            && name
                .trim_matches(|ch: char| ch.is_whitespace() || ch == '\0')
                .eq_ignore_ascii_case("words")
    })
}

/// Finds a track whose name says it holds the words.
fn find_lyric_named_track(decoded: &[(usize, u32, MetaTextKind, String)]) -> Option<usize> {
    const NAMES: [&str; 6] = ["words", "lyrics", "lyric", "text", "vocal", "melody"];
    decoded
        .iter()
        .filter(|(_, _, kind, _)| *kind == MetaTextKind::TrackName)
        .find(|(_, _, _, name)| {
            let lower = name.trim().to_lowercase();
            NAMES.iter().any(|n| lower.contains(n))
        })
        .map(|(track, _, _, _)| *track)
}

/// Fills in title, artist and copyright for files with no Soft Karaoke header.
fn fill_generic_meta(decoded: &[(usize, u32, MetaTextKind, String)], meta: &mut KaraokeMeta) {
    if meta.copyright.is_none() {
        meta.copyright = decoded
            .iter()
            .filter(|(_, _, kind, _)| *kind == MetaTextKind::Copyright)
            .find_map(|(_, _, _, text)| clean_meta_text(text));
    }
    if meta.title.is_none() {
        // Only the first two tracks are considered. Later track names are instrument names --
        // "Baixo eletrico", "Bateria" -- and taking one of those as the song title is worse than
        // having no title at all.
        meta.title = decoded
            .iter()
            .filter(|(track, _, kind, _)| *track <= 1 && *kind == MetaTextKind::TrackName)
            // Cleaned before `is_generic_track_name` looks at it, so a NUL-padded `Track` is
            // judged on the word rather than on the padding.
            .filter_map(|(_, _, _, text)| clean_meta_text(text))
            .find(|name| !is_generic_track_name(name) && names_something(name));
    }
}

fn is_generic_track_name(name: &str) -> bool {
    const GENERIC: [&str; 8] = [
        "words",
        "lyrics",
        "lyric",
        "text",
        "melody",
        "untitled",
        "track",
        "soft karaoke",
    ];
    let lower = name.trim().to_lowercase();
    lower.is_empty() || GENERIC.iter().any(|g| lower == *g)
}

/// What an annotation begins with, in a file that writes them this way.
const ANNOTATION_MARK: char = '%';

/// What a line begins with, in a file that marks its lines this way.
const LINE_MARK: char = '<';

/// Whether a payload opens with `mark`.
///
/// **Control characters are skipped rather than trimmed**, so this reaches the same verdict on the
/// raw payload and on the cleaned body -- which is what lets the whole-file measurement and the
/// per-event test share one rule instead of agreeing by luck. See [`clean_lyric_text`] for why a
/// payload carries them at all.
fn opens_with(text: &str, mark: char) -> bool {
    text.chars()
        .find(|ch| !ch.is_control())
        .is_some_and(|ch| ch == mark)
}

/// How many events a file must have before its habits are read as habits.
///
/// Below this a share says nothing: three events that all begin with `<` is as likely to be a song
/// quoting an arrow as a file marking its lines.
pub const MIN_JUDGED_EVENTS: usize = 16;

/// What share of a file's events must be marked before the mark is read as one.
///
/// A file writing this way spends most of its events on annotations -- 92 of 138 in one sample and
/// 145 of 187 in another, two thirds and three quarters. A file that merely contains a percent sign
/// is nowhere near: one `%` among two hundred words is half of one percent. Anywhere in that gap
/// does the same work, and a quarter is the round number in it.
pub const ANNOTATION_SHARE_PERCENT: usize = 25;

/// What share of the unmarked events must begin with `<` before `<` is read as a line mark.
///
/// High, because a file either marks its lines or it does not: one that marks nine tenths of them
/// and leaves the rest bare is likelier to be a file whose words start with a bracket.
pub const ANGLE_SHARE_PERCENT: usize = 90;

/// What share of a file's events must stack a harmonica tab and a word before every tab in the file
/// is read as one.
///
/// Low, because the evidence is specific and the files that carry it are uneven. A play-along often
/// tabs only the choruses, and spends its solos on bare tabs, which count for nothing here: a bare
/// number is also a count-in or a telephone number split into groups. Sampled harp files stack a
/// tab over anywhere from 9% to nearly all of their events, and no event in 21,064 files sampled
/// across the rest of the corpus stacks one at all.
pub const TAB_SHARE_PERCENT: usize = 5;

/// Whether one row of an event is a harmonica tab: a hole number, drawn with `-`, bent with `b` or
/// `'`, overblown with `o`.
///
/// `6`, `-6`, `+4`, `6b`, `-3''` and a doubled `--7` are tabs. `10a`, `Do` and `100%` are not.
fn is_harmonica_tab(row: &str) -> bool {
    let row = row.trim_matches(|ch: char| ch.is_control() || ch.is_whitespace());
    let rest = row.trim_start_matches(['-', '+']);
    if row.len() - rest.len() > 2 {
        return false;
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let suffix = &rest[digits..];
    (1..=2).contains(&digits)
        && suffix.len() <= 3
        && suffix.chars().all(|ch| matches!(ch, 'b' | '\'' | 'o'))
}

/// Whether an event stacks a harmonica tab on one row and a word on another.
///
/// The word is what makes it evidence. `26\r` is a group of a telephone number with a line break
/// after it, and `1\n` is a count-in, and both are a tab-shaped row beside an empty one.
fn stacks_a_tab(text: &str) -> bool {
    let rows = || text.split(['\n', '\r']);
    rows().any(is_harmonica_tab)
        && rows().any(|row| !is_harmonica_tab(row) && row.chars().any(char::is_alphabetic))
}

/// What share of a track's lyric events must be tabs before the track is read as a tab track.
///
/// High, because the rule drops the whole track. A sampled play-along keeps its words on one track
/// and a tab for each note on two others, and those two hold nothing else at all.
pub const TAB_TRACK_SHARE_PERCENT: usize = 90;

/// The tracks whose lyric events are harmonica tabs, in a file whose words are on another track.
///
/// **Only beside a track of words.** A file whose only lyrics are tabs is an instrumental written
/// for the harmonica, and taking its one track away leaves nothing to decide about; a track of bare
/// numbers next to the words is a second line of notation, and nobody sings it. A telephone number
/// split into groups sits on the same track as the credit it belongs to, so it never makes a track
/// of its own.
fn harmonica_tab_tracks(decoded: &[(usize, u32, MetaTextKind, String)]) -> Vec<usize> {
    // Per track: events with anything in them, and how many of those are tabs and nothing else.
    let mut tracks: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    for (track, _, kind, text) in decoded {
        if *kind != MetaTextKind::Lyric
            || text.chars().all(|ch| ch.is_control() || ch.is_whitespace())
        {
            continue;
        }
        let counts = tracks.entry(*track).or_default();
        counts.0 += 1;
        let blank = |row: &str| row.chars().all(|ch| ch.is_control() || ch.is_whitespace());
        if text
            .split(['\n', '\r'])
            .all(|row| blank(row) || is_harmonica_tab(row))
        {
            counts.1 += 1;
        }
    }
    let is_tab_track = |&(events, tabs): &(usize, usize)| {
        events >= MIN_JUDGED_EVENTS && tabs * 100 / events >= TAB_TRACK_SHARE_PERCENT
    };
    if !tracks.values().any(|counts| !is_tab_track(counts)) {
        return Vec::new();
    }
    tracks
        .iter()
        .filter(|(_, counts)| is_tab_track(counts))
        .map(|(track, _)| *track)
        .collect()
}

/// The rows of an event that are not harmonica tabs, run together.
///
/// **In a file written this way a newline divides rows, and never ends a line.** The rows are a tab,
/// the syllable under it, and sometimes a second tab under that; an event of two empty rows sits in
/// the middle of a word as readily as between two phrases. So the lines are left to be inferred, as
/// in any file that marks none.
fn without_tabs(text: &str) -> String {
    text.split(['\n', '\r'])
        .filter(|row| !is_harmonica_tab(row))
        .collect()
}

/// What a file's lyric events say about how that file writes lyrics.
///
/// **Measured once over the events one flavor accepts, rather than per event.** Both rules turn
/// punctuation into markup, and markup is a property of the writer that made the file -- so the
/// question is what this file habitually does, not what this event happens to look like. The same
/// argument [`marks_no_word_ends`](crate::timeline) is built on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dialect {
    /// `<` at the start of an event opens a line.
    pub angle_starts_lines: bool,
    /// `%` at the start of an event marks an annotation, and no part of the words.
    pub annotations_are_marked: bool,
    /// Harmonica tabs are written among the words, stacked over a syllable or on a track of their
    /// own, and no tab is any part of the words.
    pub harmonica_tabs: bool,
}

impl Dialect {
    /// Reads the habits of a file from the text of the events one flavor accepts.
    ///
    /// **The mark is what is recognised, not what is written after it.** Both sampled files spend
    /// their marked events on chord symbols, and they spell them differently -- `%SOL` `%LA-`
    /// `%FA7+` in Latin note names, `%Bm7` `%F#m7` `%A7+/D` in English ones, with slash chords and
    /// accidentals. A test for the notation would have read one file and not the other, where the
    /// structure is identical in both: an event either opens a line or is marked, and nothing else
    /// appears at all.
    #[must_use]
    pub fn of<'a>(texts: impl IntoIterator<Item = &'a str>) -> Self {
        let mut judged = 0usize;
        let mut marked = 0usize;
        let mut opened = 0usize;
        let mut tabbed = 0usize;
        for text in texts {
            // **Measured past the ordinary marks, because a writer may hang one on either end.** A
            // file that closes its lines on the annotation sends `/%LA-`, and a measurement reading
            // the raw payload would count that as neither and talk itself out of a habit the file
            // plainly has. The marks are read here with the dialect *off*, which is what leaves `<`
            // in place to be counted -- it is the thing being decided.
            let (_, body, _) = split_break_markers(text, Self::default());
            if body.chars().all(|ch| ch.is_control() || ch.is_whitespace()) {
                continue;
            }
            judged += 1;
            // The raw payload, because the rows are divided by the same newlines the break reading
            // takes off either end: `7\n` is a tab over an empty row, not a bare `7`.
            if stacks_a_tab(text) {
                tabbed += 1;
            }
            if opens_with(body, ANNOTATION_MARK) {
                marked += 1;
            } else if opens_with(body, LINE_MARK) {
                opened += 1;
            }
        }
        if judged < MIN_JUDGED_EVENTS {
            return Self::default();
        }
        // The unmarked events are the ones that could be opening a line, so they are what that share
        // is measured against -- otherwise a file three quarters full of annotations could never
        // reach a high enough proportion to be believed about its lines.
        let words = judged - marked;
        let angle_starts_lines = words > 0 && opened * 100 / words >= ANGLE_SHARE_PERCENT;
        Self {
            angle_starts_lines,
            // **A mark is only read as one in a file that also marks its lines**, which is the
            // second signal that makes dropping an event safe. Recognising the mark rather than the
            // notation is what needs it: `%` alone is a percent sign, and a file that opens every
            // line with `<` and prefixes a quarter of its events with `%` is not writing lyrics
            // that begin with a percent sign a quarter of the time. No file in the corpus sample
            // uses the one habit without the other.
            annotations_are_marked: angle_starts_lines
                && marked * 100 / judged >= ANNOTATION_SHARE_PERCENT,
            harmonica_tabs: tabbed * 100 / judged >= TAB_SHARE_PERCENT,
        }
    }
}

/// Turns matching text events into raw syllables, resolving break markers.
///
/// Answers the [`Dialect`] it read as well as the syllables, so that what was decided about a file
/// travels with the song rather than being worked out again by whoever wants to know.
fn collect_raws(
    decoded: &[(usize, u32, MetaTextKind, String)],
    mut accept: impl FnMut(&usize, &MetaTextKind, &str) -> bool,
) -> (Vec<RawSyllable>, Dialect) {
    // **Accepted once, into a list.** The closure is asked about each event exactly one time, and
    // the dialect below is measured over the events *this flavor* reads rather than over every text
    // event in the file -- a Soft Karaoke header is not evidence about how the words are written.
    let taken: Vec<(u32, &str)> = decoded
        .iter()
        .filter(|(track, _, kind, text)| accept(track, kind, text))
        .map(|(_, tick, _, text)| (*tick, text.as_str()))
        .collect();
    let dialect = Dialect::of(taken.iter().map(|(_, text)| *text));

    let mut raws = Vec::new();
    let mut pending = LineBreak::None;

    for (tick, text) in taken {
        // The tabs come off before anything reads a newline as a break, because here none is one.
        // An event that was nothing but a tab is left with nothing, and goes the way padding does.
        let untabbed;
        let text = if dialect.harmonica_tabs {
            untabbed = without_tabs(text);
            untabbed.as_str()
        } else {
            text
        };
        let (leading, body, trailing) = split_break_markers(text, dialect);
        let break_before = stronger(pending, leading);
        pending = trailing;

        // Break markers first, so a payload closing with `MEL_ \r` still ends in the space its
        // mark cancels. See [`crate::spacing`] for what an underscore in the words means. Cleaning
        // is last, once every character that was markup has been read as markup.
        let body = clean_lyric_text(crate::spacing::resolve(body));

        // **The annotation goes; the break it was carrying does not.** `break_before` has already
        // taken what was pending, so dropping the event here would swallow a line break that
        // belonged to whatever comes next -- which is the one way this rule could quietly change a
        // file it was only supposed to tidy.
        if dialect.annotations_are_marked && opens_with(&body, ANNOTATION_MARK) {
            pending = stronger(break_before, pending);
            continue;
        }

        if body.is_empty() && break_before == LineBreak::None {
            continue;
        }
        raws.push(RawSyllable {
            tick,
            text: body,
            break_before,
            end_tick: None,
        });
    }
    (raws, dialect)
}

/// Separates a payload into the break it requests before itself, its text, and any break its
/// trailing characters request for whatever comes next.
///
/// Both marker conventions are in the wild — Soft Karaoke puts `/` and `\` at the start of the
/// following syllable, while other writers append them to the end of the line they close — so both
/// ends are inspected. Bare carriage returns and newlines are treated as line breaks too, since
/// plenty of files use them instead.
fn split_break_markers(text: &str, dialect: Dialect) -> (LineBreak, &str, LineBreak) {
    let mut leading = LineBreak::None;
    let mut rest = text;

    while let Some(first) = rest.chars().next() {
        match first {
            '\\' => leading = stronger(leading, LineBreak::Page),
            '/' | '\r' | '\n' => leading = stronger(leading, LineBreak::Line),
            // **Leading only: the trailing scan below does not take this one.** The mark opens a
            // line rather than closing one, so a `<` that ends a payload is a character in the
            // words. Every other mark here is read at both ends; this is the one that is not.
            LINE_MARK if dialect.angle_starts_lines => {
                leading = stronger(leading, LineBreak::Line);
            }
            // Padding, from a field of fixed length. Stepped over rather than stopping the scan, so
            // a terminator sitting outside a marker cannot hide the marker -- `\r\0` is a line
            // break with a NUL after it, and reading it as neither loses the break as well as the
            // NUL. See [`clean_lyric_text`] for what becomes of the rest of them.
            other if other.is_control() => {}
            _ => break,
        }
        rest = &rest[first.len_utf8()..];
    }

    let mut trailing = LineBreak::None;
    while let Some(last) = rest.chars().next_back() {
        match last {
            '\\' => trailing = stronger(trailing, LineBreak::Page),
            '/' | '\r' | '\n' => trailing = stronger(trailing, LineBreak::Line),
            other if other.is_control() => {}
            _ => break,
        }
        rest = &rest[..rest.len() - last.len_utf8()];
    }

    (leading, rest, trailing)
}

/// A page break subsumes a line break; a line break subsumes nothing.
fn stronger(a: LineBreak, b: LineBreak) -> LineBreak {
    match (a, b) {
        (LineBreak::Page, _) | (_, LineBreak::Page) => LineBreak::Page,
        (LineBreak::Line, _) | (_, LineBreak::Line) => LineBreak::Line,
        _ => LineBreak::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(track: usize, tick: u32, kind: MetaTextKind, bytes: &[u8]) -> MetaText<'_> {
        MetaText {
            track,
            tick,
            kind,
            bytes,
        }
    }

    fn inference() -> LineInference {
        LineInference::for_ticks_per_quarter(480)
    }

    fn ticks_to_ms(tick: u32) -> u32 {
        (u64::from(tick) * 1_000 / 960) as u32
    }

    fn run<'a>(events: &'a [MetaText<'a>]) -> KaraokeSource {
        extract(events, inference(), ticks_to_ms, None)
    }

    #[test]
    fn marker_splitting_handles_both_conventions() {
        assert_eq!(
            split_break_markers("plain", Dialect::default()),
            (LineBreak::None, "plain", LineBreak::None)
        );
        assert_eq!(
            split_break_markers("/next", Dialect::default()),
            (LineBreak::Line, "next", LineBreak::None)
        );
        assert_eq!(
            split_break_markers("\\page", Dialect::default()),
            (LineBreak::Page, "page", LineBreak::None)
        );
        assert_eq!(
            split_break_markers("end/", Dialect::default()),
            (LineBreak::None, "end", LineBreak::Line)
        );
        assert_eq!(
            split_break_markers("end\r\n", Dialect::default()),
            (LineBreak::None, "end", LineBreak::Line)
        );
        // A terminator outside the marker does not hide it: the padding is stepped over and the
        // break is still found.
        assert_eq!(
            split_break_markers("end\r\0", Dialect::default()),
            (LineBreak::None, "end", LineBreak::Line)
        );
        assert_eq!(
            split_break_markers("\0/next", Dialect::default()),
            (LineBreak::Line, "next", LineBreak::None)
        );
        assert_eq!(
            split_break_markers("/", Dialect::default()),
            (LineBreak::Line, "", LineBreak::None)
        );
        // A page marker outranks a line marker in the same payload.
        assert_eq!(
            split_break_markers("\\/both", Dialect::default()),
            (LineBreak::Page, "both", LineBreak::None)
        );
    }

    #[test]
    fn marker_splitting_does_not_cut_multibyte_characters() {
        assert_eq!(
            split_break_markers("/ção", Dialect::default()),
            (LineBreak::Line, "ção", LineBreak::None)
        );
        assert_eq!(
            split_break_markers("ção/", Dialect::default()),
            (LineBreak::None, "ção", LineBreak::Line)
        );
    }

    #[test]
    fn soft_karaoke_is_detected_and_its_header_read() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@V0100"),
            ev(0, 0, MetaTextKind::Text, b"@LENGL"),
            ev(0, 0, MetaTextKind::Text, b"@IA test file"),
            ev(0, 0, MetaTextKind::Text, b"@TSong Title"),
            ev(0, 0, MetaTextKind::Text, b"@TThe Performers"),
            ev(1, 0, MetaTextKind::Text, b"\\Twin"),
            ev(1, 240, MetaTextKind::Text, b"kle "),
            ev(1, 480, MetaTextKind::Text, b"twin"),
            ev(1, 720, MetaTextKind::Text, b"kle"),
            ev(1, 960, MetaTextKind::Text, b"/lit"),
            ev(1, 1_200, MetaTextKind::Text, b"tle "),
            ev(1, 1_440, MetaTextKind::Text, b"star"),
        ];
        let source = run(&events);

        assert_eq!(source.flavor, KaraokeFlavor::SoftKaraoke);
        assert_eq!(source.meta.title.as_deref(), Some("Song Title"));
        assert_eq!(source.meta.artist.as_deref(), Some("The Performers"));
        assert_eq!(source.meta.language.as_deref(), Some("ENGL"));
        assert_eq!(source.meta.version.as_deref(), Some("0100"));
        assert_eq!(source.meta.info, vec!["A test file".to_owned()]);
        assert_eq!(source.timeline.line_count(), 2);
        assert_eq!(source.timeline.lines[0].text(), "Twinkle twinkle");
        assert_eq!(source.timeline.lines[1].text(), "little star");
    }

    #[test]
    fn soft_karaoke_header_lines_never_leak_into_the_lyrics() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@TTitle"),
            // A stray header line on the lyric track, which some writers emit.
            ev(1, 0, MetaTextKind::Text, b"@Iignore me"),
            ev(1, 0, MetaTextKind::Text, b"real"),
            ev(1, 240, MetaTextKind::Text, b" words"),
        ];
        let source = run(&events);
        assert_eq!(source.timeline.plain_text(), "real words");
    }

    #[test]
    fn soft_karaoke_header_lines_are_found_on_a_different_track_from_the_magic() {
        // The layout real files actually use: the magic sits alone on its own track and the rest of
        // the header is at the top of the Words track.
        let events = [
            ev(1, 0, MetaTextKind::TrackName, b"Soft karaoke"),
            ev(1, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(2, 0, MetaTextKind::TrackName, b"Words"),
            ev(2, 0, MetaTextKind::Text, b"@LENGL"),
            ev(2, 0, MetaTextKind::Text, b"@TThe Real Title"),
            ev(2, 0, MetaTextKind::Text, b"@TThe Real Band"),
            ev(2, 0, MetaTextKind::Text, b"\\Amor"),
            ev(2, 240, MetaTextKind::Text, b" da vida"),
            ev(3, 0, MetaTextKind::TrackName, b"Baixo eletrico"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::SoftKaraoke);
        assert_eq!(source.meta.title.as_deref(), Some("The Real Title"));
        assert_eq!(source.meta.artist.as_deref(), Some("The Real Band"));
        assert_eq!(source.meta.language.as_deref(), Some("ENGL"));
        assert_eq!(source.timeline.plain_text(), "Amor da vida");
    }

    #[test]
    fn the_track_called_words_wins_against_a_longer_one() {
        // A second timing of the same song, left in the file and finer than the one it replaced.
        // Counting events picks it; the name is what says which one the file means to sing.
        let mut events = vec![
            ev(1, 0, MetaTextKind::TrackName, b"Soft karaoke"),
            ev(1, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(2, 0, MetaTextKind::TrackName, b"Words"),
            ev(2, 0, MetaTextKind::Text, b"the"),
            ev(2, 480, MetaTextKind::Text, b" words"),
        ];
        for i in 0..20u32 {
            events.push(ev(3, i * 237, MetaTextKind::Text, b"stale "));
        }
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::SoftKaraoke);
        assert_eq!(source.timeline.plain_text(), "the words");
    }

    #[test]
    fn a_name_holding_a_credit_does_not_claim_the_words() {
        // `Words & Music By ...` is somebody's name in the place a name goes. Matching the name
        // whole rather than by its opening is what keeps the real lyric track.
        let events = [
            ev(1, 0, MetaTextKind::TrackName, b"Soft karaoke"),
            ev(1, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            // The longer of the two, so only the name can decide between them.
            ev(2, 0, MetaTextKind::TrackName, b"Words & Music By A Person"),
            ev(2, 0, MetaTextKind::Text, b"not"),
            ev(2, 240, MetaTextKind::Text, b" the"),
            ev(2, 480, MetaTextKind::Text, b" words"),
            ev(3, 0, MetaTextKind::TrackName, b"Words\0"),
            ev(3, 0, MetaTextKind::Text, b"right"),
            ev(3, 480, MetaTextKind::Text, b" words"),
        ];
        assert_eq!(run(&events).timeline.plain_text(), "right words");
    }

    #[test]
    fn an_instrument_track_name_is_never_taken_as_the_title() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"Soft karaoke"),
            ev(1, 0, MetaTextKind::TrackName, b"Words"),
            ev(2, 0, MetaTextKind::TrackName, b"Baixo eletrico"),
            ev(3, 0, MetaTextKind::TrackName, b"Bateria"),
            ev(1, 0, MetaTextKind::Lyric, b"la la"),
        ];
        let source = run(&events);
        assert_eq!(
            source.meta.title, None,
            "an instrument name is worse than no title"
        );
    }

    #[test]
    fn a_transcription_credit_is_not_recorded_as_the_artist() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@TExagerado"),
            ev(0, 0, MetaTextKind::Text, b"@T(Karaoke by Lucia Maria)"),
            ev(1, 0, MetaTextKind::Text, b"words here"),
        ];
        let source = run(&events);
        assert_eq!(source.meta.title.as_deref(), Some("Exagerado"));
        assert_eq!(source.meta.artist, None);
        assert_eq!(
            source.meta.info,
            vec!["(Karaoke by Lucia Maria)".to_owned()]
        );
    }

    #[test]
    fn a_real_performer_after_a_credit_line_is_still_found() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@TSong"),
            ev(0, 0, MetaTextKind::Text, b"@TSequenced by Someone"),
            ev(0, 0, MetaTextKind::Text, b"@TThe Band"),
            ev(1, 0, MetaTextKind::Text, b"words here"),
        ];
        let source = run(&events);
        assert_eq!(source.meta.artist.as_deref(), Some("The Band"));
    }

    #[test]
    fn soft_karaoke_lyrics_on_the_header_track_are_still_found() {
        // Some files put everything on one track.
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@TOne Track"),
            ev(0, 0, MetaTextKind::Text, b"all"),
            ev(0, 240, MetaTextKind::Text, b" together"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::SoftKaraoke);
        assert_eq!(source.timeline.plain_text(), "all together");
    }

    #[test]
    fn lyric_events_are_used_when_there_is_no_soft_karaoke_header() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"My Song"),
            ev(1, 0, MetaTextKind::Lyric, b"Hel"),
            ev(1, 240, MetaTextKind::Lyric, b"lo "),
            ev(1, 480, MetaTextKind::Lyric, b"world"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::LyricEvents);
        assert_eq!(source.meta.title.as_deref(), Some("My Song"));
        assert_eq!(source.timeline.plain_text(), "Hello world");
    }

    #[test]
    fn an_elided_space_reaches_the_line_as_a_space() {
        let events = [
            ev(1, 0, MetaTextKind::Lyric, b"Se_a"),
            ev(1, 240, MetaTextKind::Lyric, b"pron"),
            ev(1, 480, MetaTextKind::Lyric, b"ta "),
            ev(1, 720, MetaTextKind::Lyric, b"pra"),
        ];
        assert_eq!(run(&events).timeline.plain_text(), "Se apronta pra");
    }

    #[test]
    fn a_cancelled_space_takes_the_space_after_it() {
        let events = [
            ev(1, 0, MetaTextKind::Lyric, b"THE "),
            ev(1, 240, MetaTextKind::Lyric, b"MEL_ "),
            ev(1, 480, MetaTextKind::Lyric, b"O_ "),
            ev(1, 720, MetaTextKind::Lyric, b"DY "),
        ];
        assert_eq!(run(&events).timeline.plain_text(), "THE MELODY ");
    }

    #[test]
    fn a_break_marker_is_taken_off_before_the_space_mark_is_read() {
        // The mark still ends the payload once the `\r` closing the line has gone, so what follows
        // it is still the space it cancels. A word split across a line break is the file's doing.
        let events = [
            ev(1, 0, MetaTextKind::Lyric, b"TO_ \r"),
            ev(1, 240, MetaTextKind::Lyric, b"NIGHT."),
        ];
        let timeline = run(&events).timeline;
        assert_eq!(timeline.lines[0].text(), "TO");
        assert_eq!(timeline.lines[1].text(), "NIGHT.");
    }

    #[test]
    fn a_syllable_of_nothing_but_a_space_mark_is_dropped() {
        let events = [
            ev(1, 0, MetaTextKind::Lyric, b"one "),
            ev(1, 240, MetaTextKind::Lyric, b"_"),
            ev(1, 480, MetaTextKind::Lyric, b"two"),
        ];
        let source = run(&events);
        assert_eq!(source.timeline.plain_text(), "one two");
        assert_eq!(source.timeline.syllable_count(), 2);
    }

    /// A writer keeping its words in fixed-length fields sends the terminator along with them, and a
    /// NUL that survives as far as a font ends the process rather than the frame.
    #[test]
    fn a_nul_terminated_lyric_field_arrives_without_its_terminator() {
        let events = [
            ev(1, 0, MetaTextKind::Lyric, b"Hello\0"),
            ev(1, 240, MetaTextKind::Lyric, b" beauti\0"),
            ev(1, 480, MetaTextKind::Lyric, b"ful world\0"),
        ];
        let source = run(&events);
        assert_eq!(source.timeline.plain_text(), "Hello beautiful world");
    }

    /// Builds a lyric-event file from `(tick, payload)` pairs, all on one track.
    fn lyrics<'a>(items: &'a [(u32, &'a [u8])]) -> Vec<MetaText<'a>> {
        items
            .iter()
            .map(|(tick, bytes)| ev(1, *tick, MetaTextKind::Lyric, bytes))
            .collect()
    }

    /// A file in the chord dialect: eight chords and eight lines, in the order a writer emits them.
    ///
    /// Invented rather than taken from a corpus file, which is the rule for every fixture here. The
    /// shape is what was measured: one chord event per change, one whole line per lyric event, a
    /// terminator on every payload because the fields are fixed length.
    const CHORD_FILE: [(u32, &[u8]); 16] = [
        (0, b"%SOL\0"),
        (120, b"<WELL MY FRIENDS THE TIME HAS COME\0"),
        (480, b"%LA-\0"),
        (600, b"<RAISE THE ROOF AND HAVE SOME FUN\0"),
        (960, b"%FA7+\0"),
        (1080, b"<THROW AWAY THE WORK TO BE DONE\0"),
        (1440, b"%MI-7\0"),
        (1560, b"<LET THE MUSIC PLAY ON\0"),
        (1920, b"%DO\0"),
        (2040, b"<EV'RYBODY SING\0"),
        (2400, b"%SIb-\0"),
        (2520, b"<EV'RYBODY DANCE\0"),
        (2880, b"%SOL11\0"),
        (3000, b"<LOSE YOURSELF IN WILD ROMANCE\0"),
        (3360, b"%MIb7\0"),
        (3480, b"<COME ON AND SING ALONG\0"),
    ];

    /// The chords are annotations and the brackets are line marks: neither reaches the screen.
    #[test]
    fn a_chord_is_not_a_word_and_a_bracket_opens_a_line() {
        let events = lyrics(&CHORD_FILE);
        let source = run(&events);
        let text = source.timeline.plain_text();

        assert!(!text.contains('%'), "a chord reached the words: {text}");
        assert!(!text.contains('<'), "a line mark reached the words: {text}");
        assert_eq!(
            text.lines().next(),
            Some("WELL MY FRIENDS THE TIME HAS COME")
        );
        // One line per lyric event, and one syllable in each: the file times its words by the line,
        // and dropping the chords is what stops that being disguised as syllable-level timing.
        assert_eq!(text.lines().count(), 8, "{text}");
        assert_eq!(source.timeline.syllable_count(), 8);
    }

    /// The dropped event is still allowed to have been carrying a break.
    ///
    /// **The one way this rule could quietly change a file it was only meant to tidy.** `pending`
    /// has already been taken into `break_before` by the time the chord is recognized, so a chord
    /// that closes a line has to hand that break on rather than leave with it.
    #[test]
    fn a_chord_hands_on_the_line_break_it_was_carrying() {
        let mut file = CHORD_FILE;
        // A writer that closes its lines on the chord rather than opening them on the words.
        file[2].1 = b"/%LA-\0";
        let source = run(&lyrics(&file));
        let text = source.timeline.plain_text();

        assert!(!text.contains('%'), "{text}");
        assert_eq!(
            text.lines().count(),
            8,
            "the break the chord carried was swallowed with it: {text}"
        );
    }

    /// A file that merely mentions a chord is not a file that is written in chords.
    #[test]
    fn one_chord_among_many_words_leaves_the_file_alone() {
        let mut file = CHORD_FILE;
        // Fifteen ordinary syllables and one `%DO`, which is six percent and no habit at all.
        for (index, entry) in file.iter_mut().enumerate() {
            if index > 0 {
                entry.1 = b"la la \0";
            }
        }
        file[0].1 = b"%DO\0";
        let source = run(&lyrics(&file));

        assert!(
            source.timeline.plain_text().contains("%DO"),
            "a lone chord-shaped syllable was taken for markup"
        );
    }

    /// Nor is a file that opens some of its lines with a bracket a file that marks them that way.
    #[test]
    fn brackets_on_most_lines_is_not_enough_to_read_them_as_marks() {
        let mut file = CHORD_FILE;
        for entry in file.iter_mut() {
            entry.1 = b"<a line of words\0";
        }
        // Fourteen of sixteen is 87 percent, under the share, so every bracket stays a character.
        file[0].1 = b"a line of words\0";
        file[1].1 = b"a line of words\0";
        let source = run(&lyrics(&file));

        assert!(
            source.timeline.plain_text().contains('<'),
            "brackets were read as marks below the share"
        );
    }

    /// Too few events to have a habit, and one more than too few.
    #[test]
    fn a_file_is_not_judged_until_it_has_enough_events_to_have_a_habit() {
        let short: Vec<&str> = CHORD_FILE
            .iter()
            .take(MIN_JUDGED_EVENTS - 1)
            .map(|(_, bytes)| std::str::from_utf8(bytes).expect("ascii"))
            .collect();
        assert_eq!(Dialect::of(short), Dialect::default());

        let enough: Vec<&str> = CHORD_FILE
            .iter()
            .take(MIN_JUDGED_EVENTS)
            .map(|(_, bytes)| std::str::from_utf8(bytes).expect("ascii"))
            .collect();
        assert_eq!(
            Dialect::of(enough),
            Dialect {
                angle_starts_lines: true,
                annotations_are_marked: true,
                harmonica_tabs: false,
            }
        );
    }

    /// The mark is read wherever the notation after it goes, which is the point of reading the mark.
    ///
    /// One sampled file spells its chords in Latin note names and another in English ones, with
    /// slash chords and accidentals. A test for the notation would have read one file and not the
    /// other.
    #[test]
    fn the_mark_is_what_is_read_and_not_the_notation_after_it() {
        for marked in ["%SOL", "%LA-", "%FA7+", "%Bm7", "%F#m7", "%A7+/D", "%E4_7"] {
            assert!(opens_with(marked, ANNOTATION_MARK), "{marked}");
        }
        // The terminator makes no difference, which is what lets the whole-file measurement and the
        // per-event test share one rule.
        assert!(opens_with("%SOL\0", ANNOTATION_MARK));
        // A percent inside a line is a percent.
        assert!(!opens_with("100% PURE LOVE", ANNOTATION_MARK));
        assert!(!opens_with("", ANNOTATION_MARK));
    }

    /// A file full of percent signs that does not mark its lines keeps every one of them.
    ///
    /// **The second signal is what makes dropping an event safe.** `%` alone is a percent sign; a
    /// file that also opens every line with `<` is one writing in a notation, and no corpus file
    /// was found using either habit without the other.
    #[test]
    fn marks_are_only_read_in_a_file_that_also_marks_its_lines() {
        let mut file = CHORD_FILE;
        // The same marked events, but the words arrive plainly rather than opened with a bracket.
        for (index, entry) in file.iter_mut().enumerate() {
            if index % 2 == 1 {
                entry.1 = b"a line of words\0";
            }
        }
        let source = run(&lyrics(&file));

        assert!(
            source.timeline.plain_text().contains('%'),
            "an event was dropped on the strength of one habit"
        );
    }

    /// A bracket opens a line and never closes one, unlike every other mark here.
    #[test]
    fn a_bracket_is_a_leading_mark_only() {
        let reading = Dialect {
            angle_starts_lines: true,
            annotations_are_marked: true,
            harmonica_tabs: false,
        };
        assert_eq!(
            split_break_markers("<a line", reading),
            (LineBreak::Line, "a line", LineBreak::None)
        );
        // A bare one is a break with nothing in it, exactly as a bare `/` is.
        assert_eq!(
            split_break_markers("<", reading),
            (LineBreak::Line, "", LineBreak::None)
        );
        // At the end it is a character somebody wrote.
        assert_eq!(
            split_break_markers("a line<", reading),
            (LineBreak::None, "a line<", LineBreak::None)
        );
        // And in a file with no such habit it is a character wherever it sits.
        assert_eq!(
            split_break_markers("<a line", Dialect::default()),
            (LineBreak::None, "<a line", LineBreak::None)
        );
    }

    /// A file in the harmonica dialect: a tab over every syllable, a second tab under some, a run of
    /// bare tabs for the solo and an event of two empty rows in the middle of a word.
    ///
    /// Invented rather than taken from a corpus file. The shape is what was measured.
    const HARP_FILE: [(u32, &[u8]); 20] = [
        (0, b"6\nWalk"),
        (240, b"-6\nin"),
        (480, b"5b\nthe"),
        (720, b"--7\nsun"),
        (960, b"6\nshi"),
        (1200, b"\n"),
        (1440, b"-5\nning,"),
        (1680, b"4\nwalk\n7"),
        (1920, b"-4\nin\n-8"),
        (2160, b"5\nthe"),
        (2400, b"6'\nrain."),
        (4800, b"-8"),
        (5040, b"7\n"),
        (5280, b"-6"),
        (5520, b"6"),
        (9600, b"6\nSing"),
        (9840, b"-6\nit"),
        (10080, b"5\nout"),
        (10320, b"4\nloud"),
        (10560, b"-4\nnow."),
    ];

    #[test]
    fn a_harmonica_tab_is_a_hole_and_how_it_is_played() {
        for tab in ["6", "-6", "+4", "10", "6b", "-3''", "--7", "6o", " 5 "] {
            assert!(is_harmonica_tab(tab), "{tab:?} is a tab");
        }
        for word in ["", "-", "Do", "10a", "100", "100%", "6bbbb", "---7", "b6"] {
            assert!(!is_harmonica_tab(word), "{word:?} is not a tab");
        }
    }

    /// The tabs go, the words stay, and the solo leaves nothing behind.
    #[test]
    fn a_harmonica_tab_is_not_a_word() {
        let source = run(&lyrics(&HARP_FILE));
        let text = source.timeline.plain_text();

        assert!(source.dialect.harmonica_tabs);
        assert!(
            !text.chars().any(|ch| ch.is_ascii_digit()),
            "a tab reached the words: {text}"
        );
        assert!(
            text.contains("shining"),
            "the empty rows split a word: {text}"
        );
        // No newline in the file marked a line, so the gaps place them.
        assert!(!source.timeline.lines_are_marked);
        assert_eq!(text.lines().count(), 2, "{text}");
    }

    /// A song that sings a number is not a file written in tabs.
    #[test]
    fn numbers_in_the_words_leave_the_file_alone() {
        let mut file = HARP_FILE;
        for entry in &mut file {
            entry.1 = b"la ";
        }
        file[0].1 = b"10 ";
        file[1].1 = b"99\n";
        file[2].1 = b"6\n";
        let source = run(&lyrics(&file));

        assert!(!source.dialect.harmonica_tabs);
        assert!(source.timeline.plain_text().contains("10"));
    }

    /// Tabs on a track of their own go with their track, and the words on the other one stay.
    #[test]
    fn a_track_of_tabs_beside_the_words_is_dropped() {
        const WORDS: [&[u8]; 4] = [b"walk ", b"in ", b"the ", b"rain "];
        const TABS: [&[u8]; 4] = [b"6", b"-6", b"5b", b"-4"];
        let mut events = Vec::new();
        for i in 0..20u32 {
            let index = i as usize % 4;
            events.push(ev(1, i * 240, MetaTextKind::Lyric, WORDS[index]));
            events.push(ev(5, i * 240, MetaTextKind::Lyric, TABS[index]));
        }
        let source = run(&events);
        let text = source.timeline.plain_text();

        assert!(source.dialect.harmonica_tabs);
        assert!(!text.chars().any(|ch| ch.is_ascii_digit()), "{text}");
        assert!(text.contains("walk in the rain"), "{text}");

        // With nothing beside it, the track of tabs is all the file has, and it stays.
        let alone: Vec<_> = events.into_iter().filter(|e| e.track == 5).collect();
        let source = run(&alone);
        assert!(!source.dialect.harmonica_tabs);
        assert!(source.timeline.plain_text().contains("-6"));
    }

    /// A telephone number split into groups is on the track of the credit it belongs to.
    #[test]
    fn numbers_among_the_words_on_one_track_are_not_a_tab_track() {
        let mut events = Vec::new();
        for i in 0..20u32 {
            let text: &[u8] = if i % 2 == 0 { b"47 " } else { b"call " };
            events.push(ev(1, i * 240, MetaTextKind::Lyric, text));
        }
        let source = run(&events);

        assert!(!source.dialect.harmonica_tabs);
        assert!(source.timeline.plain_text().contains("47"));
    }

    /// The same rule as a bare space mark: what cleaning empties, the line does not carry.
    #[test]
    fn a_syllable_of_nothing_but_padding_is_dropped() {
        let events = [
            ev(1, 0, MetaTextKind::Lyric, b"one two "),
            ev(1, 240, MetaTextKind::Lyric, b"\0\0\0"),
            ev(1, 480, MetaTextKind::Lyric, b"three four"),
        ];
        let source = run(&events);
        assert_eq!(source.timeline.plain_text(), "one two three four");
        assert_eq!(source.timeline.syllable_count(), 2);
    }

    #[test]
    fn lyric_events_win_over_a_named_text_track() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"Words"),
            ev(0, 0, MetaTextKind::Text, b"stale copy"),
            ev(1, 0, MetaTextKind::Lyric, b"live copy"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::LyricEvents);
        assert_eq!(source.timeline.plain_text(), "live copy");
    }

    #[test]
    fn a_named_text_track_is_the_last_resort() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"Conductor"),
            ev(1, 0, MetaTextKind::TrackName, b"Words"),
            ev(1, 0, MetaTextKind::Text, b"from"),
            ev(1, 240, MetaTextKind::Text, b" a track"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::NamedTextTrack);
        assert_eq!(source.timeline.plain_text(), "from a track");
        // "Conductor" is not a generic name, so it becomes the title.
        assert_eq!(source.meta.title.as_deref(), Some("Conductor"));
    }

    // -- control characters in metadata ----------------------------------------------------
    //
    // Real files, not a hypothetical: the local corpus has 1,467 titles carrying a control
    // character and 179 that are nothing but padding. Synthetic shapes rather than corpus files,
    // per CLAUDE.local.md.

    #[test]
    fn a_track_name_of_pure_padding_is_not_a_title() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"\x00\x00\x00\x00"),
            ev(0, 0, MetaTextKind::Marker, b"Verse 1"),
        ];
        // Not merely blank-looking: a NUL sorts before every printable character, so accepting this
        // put a row with nothing in it at the top of a title-ordered browse.
        assert_eq!(run(&events).meta.title, None);
    }

    #[test]
    fn a_soft_karaoke_title_of_pure_padding_is_not_a_title() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@T\x00"),
            ev(0, 0, MetaTextKind::Text, b"@TThe Real Title"),
            ev(1, 0, MetaTextKind::Lyric, b"words"),
        ];
        let meta = run(&events).meta;
        // The padded line is dropped rather than consuming the title slot: positional parsing means
        // keeping it would have made the real title the *artist*.
        assert_eq!(meta.title.as_deref(), Some("The Real Title"));
        assert_eq!(meta.artist, None);
    }

    #[test]
    fn padding_after_a_real_title_is_trimmed_off_it() {
        let events = [
            ev(
                0,
                0,
                MetaTextKind::TrackName,
                b"Otonosuke\x00\x00\x00\x00\x00\x00",
            ),
            ev(0, 0, MetaTextKind::Marker, b"Verse 1"),
        ];
        // The commonest shape by far -- 1,343 of the corpus's 1,467 -- and the one that rendered as
        // a title that looked right and sorted wrong.
        assert_eq!(run(&events).meta.title.as_deref(), Some("Otonosuke"));
    }

    #[test]
    fn a_padded_generic_track_name_is_still_recognized_as_generic() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"Track\x00"),
            ev(1, 0, MetaTextKind::TrackName, b"Piano Roll"),
            ev(0, 0, MetaTextKind::Marker, b"Verse 1"),
        ];
        // The cleaning has to happen *before* the generic test, which compares whole names:
        // `Track\0` is equal to none of them, so the padding would win twice over.
        assert_eq!(run(&events).meta.title.as_deref(), Some("Piano Roll"));
    }

    #[test]
    fn a_two_line_title_becomes_one_line() {
        assert_eq!(
            clean_meta_text("Some Song\r\nAnd More"),
            Some("Some Song And More".to_owned())
        );
        // Whitespace-only, control-only and empty all mean the same thing: no title here.
        assert_eq!(clean_meta_text("\x00\x01\x7f"), None);
        assert_eq!(clean_meta_text("   "), None);
        assert_eq!(clean_meta_text(""), None);
        // An ordinary title is returned exactly as it was.
        assert_eq!(
            clean_meta_text("Águas de Março"),
            Some("Águas de Março".to_owned())
        );
    }

    // -- names made of marks ------------------------------------------------------------------

    #[test]
    fn a_name_holds_two_letters_or_digits() {
        // The separator rows a corpus puts in a title meta event, then the frames with a single
        // character inside them, then the bare ones -- a track index and a key signature.
        for value in [
            "====================",
            "-",
            ".",
            "???",
            "<>-<>-<>-<>",
            "****************",
            "-----------------------------------",
            "====== X ======",
            "-<>-a-<>-<>-",
            "** 2",
            "(+ 1)",
            "<(°L°)>",
            "--------- C# ---------",
            "1",
            "A",
            "D#",
            "*2",
        ] {
            assert_eq!(clean_meta_name(value), None, "{value} names something");
        }
        // Names, including the framed ones a rule counting marks against letters would refuse, and
        // the accented and Japanese ones an ASCII rule would.
        for value in [
            "Corcovado",
            "1999",
            "AC/DC",
            "R.E.M.",
            "S.O.S.",
            "P!nk",
            "Águas de Março",
            "夜空ノムコウ",
            // One character, and a word: rainbow, pulse, rain.
            "虹",
            "脈",
            "비",
            "***** I Love You *****",
            "----- TAKE FIVE -----",
            "- - -X-FILES- - -",
            "-<>-<>- Corcovado -<>-<>-",
        ] {
            assert_eq!(
                clean_meta_name(value).as_deref(),
                Some(value),
                "{value} names nothing"
            );
        }
    }

    #[test]
    fn a_soft_karaoke_title_of_marks_is_not_a_title() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@T===================="),
            ev(0, 0, MetaTextKind::Text, b"@TThe Real Title"),
            ev(1, 0, MetaTextKind::Lyric, b"words"),
        ];
        let meta = run(&events).meta;
        // Dropped rather than consuming the title slot, exactly as padding is: positional parsing
        // means keeping it would have made the real title the *artist*.
        assert_eq!(meta.title.as_deref(), Some("The Real Title"));
        assert_eq!(meta.artist, None);
        // And kept nowhere. A credit records where a file came from; a row of marks records nothing.
        assert!(meta.info.is_empty(), "got {:?}", meta.info);
    }

    #[test]
    fn a_title_survives_an_artist_made_of_marks() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@TCorcovado"),
            ev(0, 0, MetaTextKind::Text, b"@T<>-<>-<>"),
            ev(1, 0, MetaTextKind::Lyric, b"words"),
        ];
        let meta = run(&events).meta;
        assert_eq!(meta.title.as_deref(), Some("Corcovado"));
        assert_eq!(meta.artist, None);
    }

    #[test]
    fn a_track_name_of_marks_is_not_a_title() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"****************"),
            ev(1, 0, MetaTextKind::TrackName, b"Piano Roll"),
            ev(0, 0, MetaTextKind::Marker, b"Verse 1"),
        ];
        assert_eq!(run(&events).meta.title.as_deref(), Some("Piano Roll"));
    }

    #[test]
    fn a_file_whose_only_names_are_marks_has_none() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@T===================="),
            ev(0, 0, MetaTextKind::Text, b"@T<>-<>-<>-<>"),
            ev(1, 0, MetaTextKind::Lyric, b"words"),
        ];
        let meta = run(&events).meta;
        // Both columns empty is the point: the row falls through to its file name in curation, and
        // an artist is never invented.
        assert_eq!(meta.title, None);
        assert_eq!(meta.artist, None);
    }

    #[test]
    fn every_other_metadata_field_keeps_the_looser_gate() {
        // `clean_meta_name` is a second function rather than a stricter `clean_meta_text` because a
        // notice does not have to name anything.
        assert_eq!(clean_meta_text("©"), Some("©".to_owned()));
        assert_eq!(clean_meta_name("©"), None);
    }

    #[test]
    fn a_file_with_no_lyrics_reports_none_but_still_yields_metadata() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"Instrumental"),
            ev(0, 0, MetaTextKind::Copyright, b"(c) 1999 Somebody"),
            ev(0, 0, MetaTextKind::Marker, b"Verse 1"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::None);
        assert!(source.timeline.is_empty());
        assert_eq!(source.meta.title.as_deref(), Some("Instrumental"));
        assert_eq!(source.meta.copyright.as_deref(), Some("(c) 1999 Somebody"));
    }

    #[test]
    fn a_soft_karaoke_header_with_no_lyrics_falls_through_to_lyric_events() {
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(0, 0, MetaTextKind::Text, b"@TEmpty Header"),
            ev(1, 0, MetaTextKind::Lyric, b"actual words"),
        ];
        let source = run(&events);
        assert_eq!(source.flavor, KaraokeFlavor::LyricEvents);
        assert_eq!(source.timeline.plain_text(), "actual words");
        // The header was still read, so its title survives.
        assert_eq!(source.meta.title.as_deref(), Some("Empty Header"));
    }

    #[test]
    fn generic_track_names_are_not_mistaken_for_titles() {
        let events = [
            ev(0, 0, MetaTextKind::TrackName, b"Words"),
            ev(1, 0, MetaTextKind::Lyric, b"la"),
        ];
        let source = run(&events);
        assert_eq!(source.meta.title, None);
    }

    #[test]
    fn a_trailing_marker_breaks_the_following_syllable_not_the_current_one() {
        let events = [
            ev(0, 0, MetaTextKind::Lyric, b"first line/"),
            ev(0, 480, MetaTextKind::Lyric, b"second line"),
        ];
        let source = run(&events);
        assert_eq!(source.timeline.line_count(), 2);
        assert_eq!(source.timeline.lines[0].text(), "first line");
        assert_eq!(source.timeline.lines[1].text(), "second line");
    }

    #[test]
    fn a_declared_encoding_is_applied_to_lyrics() {
        // 0xE7 is "ç" in CP1252 and invalid UTF-8.
        let bytes = [b'c', 0xE7, b'a'];
        let events = [ev(0, 0, MetaTextKind::Lyric, &bytes)];
        let source = extract(&events, inference(), ticks_to_ms, Some("windows-1252"));
        assert_eq!(source.timeline.plain_text(), "cça");
        assert_eq!(source.decoder.name(), "windows-1252");
    }

    #[test]
    fn empty_input_is_handled() {
        let source = run(&[]);
        assert_eq!(source.flavor, KaraokeFlavor::None);
        assert!(source.timeline.is_empty());
        assert_eq!(source.meta, KaraokeMeta::default());
    }

    #[test]
    fn a_producer_credit_in_first_position_does_not_become_the_title() {
        // From real files, found while building a package from the corpus: the studio's name is the
        // *first* `@T`, so taking position 0 as the title put "Karaoke do Brasil" in the catalog
        // as the title of hundreds of songs.
        let events = [
            ev(0, 0, MetaTextKind::Text, SOFT_KARAOKE_MAGIC.as_bytes()),
            ev(
                0,
                0,
                MetaTextKind::Text,
                b"@TKaraoke do Brasil - Familia Ribeiro - 2000",
            ),
            ev(0, 0, MetaTextKind::Text, b"@TTrilhos Urbanos"),
            ev(0, 0, MetaTextKind::Text, b"@TCaetano Veloso"),
            ev(1, 0, MetaTextKind::Text, b"words here"),
        ];
        let source = run(&events);
        assert_eq!(source.meta.title.as_deref(), Some("Trilhos Urbanos"));
        assert_eq!(source.meta.artist.as_deref(), Some("Caetano Veloso"));
        assert!(
            source
                .meta
                .info
                .iter()
                .any(|line| line.contains("Familia Ribeiro")),
            "the credit is kept, just not as the title: {:?}",
            source.meta.info
        );
    }

    #[test]
    fn the_word_karaoke_alone_marks_a_credit() {
        for credit in [
            "Karaoke do Brasil",
            "KARAOKE VERSION 3",
            "Sequenciado por Alguem",
            "produced by Someone",
        ] {
            assert!(
                looks_like_a_credit(credit),
                "{credit:?} should read as a credit"
            );
        }
        // Ordinary titles and performer names must not be caught.
        for name in ["Trilhos Urbanos", "Caetano Veloso", "The Beatles", "Wave"] {
            assert!(!looks_like_a_credit(name), "{name:?} is not a credit");
        }
    }

    #[test]
    fn a_line_that_is_only_a_publishers_notice_is_one() {
        for line in [
            "ALL rights reserved. Not for broadcast or",
            "transmission of any kind.",
            "DO NOT DUPLICATE. NOT FOR RENTAL.",
            "International rights secured. All rights reserved.",
            "Copyright 1994",
            "(c) 1998",
            "Todos os direitos reservados",
        ] {
            assert!(
                is_only_a_legal_notice(line),
                "{line:?} is a notice and nothing else"
            );
        }
    }

    #[test]
    fn a_notice_that_names_somebody_is_a_banner_and_not_only_a_notice() {
        // The distinction the whole rule turns on: a publisher is something, so the line is not
        // *only* the notice and is not thrown away. `looks_like_a_banner` still keeps it out of a
        // preview, which is the lighter consequence.
        for line in [
            "Copyright 1994 Some Publisher",
            "All rights reserved, TUNE 1000 CORP.",
            "Copyright Clubhouse Productions, All Rights Reserved",
        ] {
            assert!(!is_only_a_legal_notice(line), "{line:?} names somebody");
            assert!(looks_like_a_banner(line), "{line:?} is still a banner");
        }
    }

    #[test]
    fn a_line_of_a_song_is_never_only_a_notice() {
        // The direction that costs a verse. Every one of these carries a word the notice list
        // touches, and none of them is a notice.
        for line in [
            "100% PURE LOVE",
            "and I have no reserved seat for you",
            "All the rights and wrongs of loving you",
            "Do not duplicate my heart tonight and leave",
            "copyright my soul, she said, and laughed",
            "or",
            "and",
            "no",
        ] {
            assert!(
                !is_only_a_legal_notice(line),
                "{line:?} is a line of a song"
            );
        }
    }

    #[test]
    fn nothing_at_all_is_not_a_notice() {
        // Unlike `looks_like_a_banner`, which answers true for an empty line because a preview may
        // skip one. Dropping every blank line from a timeline would close up the gaps a song has.
        for line in ["", "   ", "\t"] {
            assert!(!is_only_a_legal_notice(line));
        }
    }

    #[test]
    fn a_sequencers_advertisement_is_not_a_line_of_the_song() {
        for banner in [
            // Producer credits, through the shared phrase list.
            "Karaoke do Brasil - Familia Ribeiro - 2000",
            "Sequenced by Terry.J",
            "MIDI by Paul Malcolm",
            // Contact details. Both shapes are all over the corpus's own fixtures.
            "http://www.example.com/someone/karaoke.html",
            "someone@example.com.br",
            "Visit us at https://example.test/kar",
            // Copyright, in both the languages this corpus is actually in.
            "Copyright 1998 Paul Malcolm",
            "© 2000 Editora Zardo Ltda.",
            "(c) 1999 Someone",
            "Todos os direitos reservados",
            // The wrapped commercial-disc notice, both halves. Only the first matched until the
            // corpus showed 97 files whose *preview* was the second.
            "ALL rights reserved. Not for broadcast or",
            "transmission of any kind.",
            "DO NOT DUPLICATE. NOT FOR RENTAL.",
            "International Rights Secured.",
            // An address with a telephone number after it — the case that showed the host had to be
            // the next token rather than the rest of the line.
            "someone@example.com (0**19) 5550123",
            // Credits with no "by" in them.
            "SAROBA PRODUCOES 34 3212 9158",
            "Adriano Produções",
            "Sincronizado por KIRA",
            // A track name or a section marker that reached the lyric stream. The brackets need no
            // rule of their own: folding maps them to spaces and trims.
            "Vocals",
            "Midi",
            "{Words}",
            "(Intro)",
            "[ VOCALS ]",
            // A stray event, and an identifier. Both are common enough in the corpus to be worth a
            // rule: 89 and 41 files in 30,000.
            "a",
            "{M}",
            "7444",
            // The whole-corpus run's own findings: a credit in the other two languages, a credit
            // whose verb nobody could enumerate, an unfilled placeholder, and a track label with the
            // trailing padding a fixed-width text event leaves behind.
            "Trabajo realizado por",
            "Editado por \"GLOMAR\"",
            "exclusive by: tomson",
            // Both lines of the unfilled template, which is the point: catching only the first
            // promoted the second to being the preview of the same 110 files.
            "song title",
            "artist",
            "Vocal-Line                    ",
            // Ornaments framing the credit above or below them.
            "****************",
            "- - - - - - - -",
            "================",
            // A telephone number, which is what "0**17 3463-1150" in the corpus actually is.
            "0**17 3463-1150",
            "(011) 5555-1234",
            // Blank lines, so a caller skipping banners does not have to check twice.
            "",
            "   ",
        ] {
            assert!(
                looks_like_a_banner(banner),
                "{banner:?} should read as a banner"
            );
        }

        // Real lyric lines, including the ones each rule could plausibly over-reach on.
        for line in [
            "Tempo perdido",
            "E que tudo mais vá pro inferno",
            "Hello darkness my old friend",
            // The digit rules, at the edges. A year is four digits; a date is six but is more than
            // half letters; "1234567" is seven and nothing else, and would be caught -- which is
            // why no test asserts a lyric of bare digits survives, because there is not one.
            "1999 was the year",
            "In the summer of 69",
            "Vinte e um de abril de 1792",
            // Capitals are not a signal: plenty of files shout.
            "NÃO CHORE MAIS",
            // A single word, and a line ending in punctuation.
            "Wave",
            "Oh!",
            // The label rule is exact-match for exactly this reason.
            "Words of love",
            "Track of my tears",
            "Say the words and I'll be there",
            // A count-in is a line of a song, and is what the digit rule checks *before* folding:
            // an identifier has no separators in it and this does.
            "1, 2, 3, 4",
            // Two characters is a line; one is a stray event.
            "Oh",
            // `por` and `by` in the middle of a sentence are ordinary words. Only the credit
            // constructions above are caught, and `by:` needs its colon.
            "Passei por aqui ontem",
            "Stand by me",
            "The title of this song is love",
            "The artist formerly known as",
            // An `@` that is not an email, which is why the host is checked rather than the sign.
            "meet me @ the corner",
        ] {
            assert!(!looks_like_a_banner(line), "{line:?} is a line of a song");
        }
    }
}
