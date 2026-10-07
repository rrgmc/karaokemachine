//! Writing timed words into a MIDI file, as Soft Karaoke.
//!
//! The words go in as [`RawSyllable`]s, which is the shape [`crate::karaoke`] reads a file into, so
//! a file written here reads back as the syllables it was given. Soft Karaoke is the convention
//! written because it is the first one the reader tries and the one most other players know.
//!
//! **The music is not touched.** Every event that is not karaoke text leaves with the bytes it
//! arrived with, through [`crate::smf`].

use std::path::{Path, PathBuf};

use crate::karaoke::KaraokeFlavor;
use crate::smf::{self, Event, SmfError};
use crate::timeline::{LineBreak, RawSyllable};
use crate::{ParseOptions, Song, SongError};

/// The text event that announces a Soft Karaoke file.
const MAGIC: &str = "@KMIDI KARAOKE FILE";

/// The Soft Karaoke version line every file in circulation carries.
const VERSION: &str = "@V0100";

/// The track name [`crate::karaoke`] ranks above any other when it looks for the words.
const WORDS_TRACK: &str = "Words";

/// The name real files give the track that holds only the announcement.
const HEADER_TRACK: &str = "Soft Karaoke";

/// What to write into a file: what it says about itself, and its words.
#[derive(Debug, Clone, Default)]
pub struct KarWords {
    /// The first `@T` line. Soft Karaoke reads titles by position, so this one is never skipped.
    pub title: String,
    /// The second `@T` line, left out when empty.
    pub artist: String,
    /// The `@L` line in the four-letter form real files use (`ENGL`), left out when empty.
    pub language: String,
    /// The syllables in singing order, each at its tick.
    ///
    /// A syllable that opens a word carries a leading space, and one that continues a word carries
    /// none. [`RawSyllable::break_before`] says where a line or a page opens.
    pub syllables: Vec<RawSyllable>,
}

/// Why a file could not be given its words.
#[derive(Debug, thiserror::Error)]
pub enum KarWriteError {
    /// There is nothing to write.
    #[error("there are no words to write")]
    NoWords,
    /// The source could not be walked or extended.
    #[error(transparent)]
    Smf(#[from] SmfError),
    /// The written file does not parse.
    #[error("the written file cannot be read back: {0}")]
    Unreadable(#[from] SongError),
    /// The written file parses and its words are not found in it as Soft Karaoke.
    #[error("the written file does not read back as Soft Karaoke")]
    NotReadBack,
}

/// Returns `source` with its karaoke text replaced by `words`.
///
/// Three kinds of event are removed, because any of them would be read as words beside the new
/// ones. They are every lyric event, every text event after tick zero, and every text event that
/// opens with `@`. A text event at tick zero that is not a control line is a comment, and stays.
///
/// A track too damaged to walk is copied whole, with whatever text it holds.
pub fn write_soft_karaoke(source: &[u8], words: &KarWords) -> Result<Vec<u8>, KarWriteError> {
    if words.syllables.is_empty() {
        return Err(KarWriteError::NoWords);
    }
    let (header, chunks) = smf::split_chunks(source)?;

    let mut tracks: Vec<Vec<u8>> = Vec::with_capacity(chunks.len() + 2);
    for chunk in &chunks {
        match smf::parse_track(chunk) {
            Ok(mut events) => {
                smf::drop_where(&mut events, is_karaoke_text);
                tracks.push(smf::emit_track(&events));
            }
            Err(_) => tracks.push(smf::emit_chunk(chunk)),
        }
    }

    tracks.push(smf::emit_track(&smf::track_at(
        HEADER_TRACK,
        vec![(0, text(MAGIC)), (0, text(VERSION))],
    )));
    tracks.push(smf::emit_track(&smf::track_at(
        WORDS_TRACK,
        words_events(words),
    )));

    let mut out = smf::header_for(&header, tracks.len())?;
    for track in &tracks {
        out.extend_from_slice(track);
    }

    let read_back = Song::parse(&out, &ParseOptions::default())?;
    if read_back.flavor != KaraokeFlavor::SoftKaraoke {
        return Err(KarWriteError::NotReadBack);
    }
    Ok(out)
}

/// Where a song's synced copy goes when nobody names a place: beside it, with the karaoke extension.
///
/// A song that already has that extension gets `-synced` on its name, so the copy is never the
/// song. A program listing songs asks here, and marks the copies that exist as the editor would
/// find them.
#[must_use]
pub fn synced_path(song: &Path) -> PathBuf {
    let beside = song.with_extension("kar");
    if beside != song {
        return beside;
    }
    let stem = song.file_stem().unwrap_or_default().to_string_lossy();
    song.with_file_name(format!("{stem}-synced.kar"))
}

/// Splits typed words into syllables, every one at tick zero.
///
/// One typed line is one sung line, and an empty line opens a page. A hyphen splits a word into
/// syllables and is not drawn, so `ka-ra-o-ke` is four syllables of one word. `\-` is a hyphen that
/// is drawn. A word made only of hyphens is drawn as typed.
pub fn split_words(typed: &str) -> Vec<RawSyllable> {
    let mut syllables = Vec::new();
    let mut pending = LineBreak::Page;

    for line in typed.lines() {
        if line.trim().is_empty() {
            pending = LineBreak::Page;
            continue;
        }
        for word in line.split_whitespace() {
            let mut opens_the_word = true;
            for part in split_syllables(word) {
                let break_before = std::mem::replace(&mut pending, LineBreak::None);
                let space = opens_the_word && break_before == LineBreak::None;
                opens_the_word = false;
                syllables.push(RawSyllable {
                    tick: 0,
                    text: if space { format!(" {part}") } else { part },
                    break_before,
                    end_tick: None,
                });
            }
        }
        // Whatever the next line is, it opens a line at least. A page already asked for stays.
        if pending == LineBreak::None {
            pending = LineBreak::Line;
        }
    }
    syllables
}

/// Writes syllables back as the text [`split_words`] reads.
///
/// One sung line is one line of text, and a page opens after an empty line. A syllable that goes on
/// a word gets a hyphen before it, and a hyphen that is drawn is written `\-`. Reading the result
/// gives the same words, lines and pages.
#[must_use]
pub fn join_words(syllables: &[RawSyllable]) -> String {
    let mut typed = String::new();
    let mut after_space = true;
    for (index, syllable) in syllables.iter().enumerate() {
        let text = syllable.text.trim();
        let opens_a_word = after_space || syllable.text.starts_with(char::is_whitespace);
        match syllable.break_before {
            _ if index == 0 => {}
            LineBreak::Page => typed.push_str("\n\n"),
            LineBreak::Line => typed.push('\n'),
            LineBreak::None if opens_a_word => typed.push(' '),
            LineBreak::None => typed.push('-'),
        }
        typed.push_str(&text.replace('-', "\\-"));
        after_space = syllable.text.ends_with(char::is_whitespace);
    }
    if !typed.is_empty() {
        typed.push('\n');
    }
    typed
}

/// One typed word as its syllables.
fn split_syllables(word: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = word.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars.peek() == Some(&'-') => {
                chars.next();
                parts.last_mut().expect("never empty").push('-');
            }
            '-' => parts.push(String::new()),
            other => parts.last_mut().expect("never empty").push(other),
        }
    }
    parts.retain(|part| !part.is_empty());
    if parts.is_empty() {
        parts.push(word.to_owned());
    }
    parts
}

/// Whether an event would be read as karaoke text beside the words being written.
fn is_karaoke_text(tick: u32, event: &Event) -> bool {
    if event.is_meta(0x05) {
        return true;
    }
    event.is_meta(0x01) && (tick > 0 || event.payload().is_some_and(|text| text.starts_with(b"@")))
}

/// The words track's events: its header lines at tick zero, then each syllable at its tick.
fn words_events(words: &KarWords) -> Vec<(u32, Event)> {
    let mut placed = Vec::with_capacity(words.syllables.len() + 3);
    if !words.language.trim().is_empty() {
        placed.push((0, text(&format!("@L{}", one_line(&words.language)))));
    }
    placed.push((0, text(&format!("@T{}", one_line(&words.title)))));
    if !words.artist.trim().is_empty() {
        placed.push((0, text(&format!("@T{}", one_line(&words.artist)))));
    }

    for (index, syllable) in words.syllables.iter().enumerate() {
        // The first syllable always opens a page, so no word can open the stream with an `@` and
        // be taken for a header line.
        let break_before = if index == 0 {
            LineBreak::Page
        } else {
            syllable.break_before
        };
        // A marker stands in for the space it replaces, or the line is drawn a space off center.
        let written = match break_before {
            LineBreak::Page => format!("\\{}", syllable.text.trim_start()),
            LineBreak::Line => format!("/{}", syllable.text.trim_start()),
            LineBreak::None => syllable.text.clone(),
        };
        placed.push((syllable.tick, text(&written)));
        // An event with no bytes is how a file says a word stops. See `karaoke::collect_raws`.
        if let Some(end) = syllable.end_tick.filter(|&end| end > syllable.tick) {
            placed.push((end, text("")));
        }
    }
    placed
}

/// A text meta event.
fn text(value: &str) -> Event {
    Event::meta(0x01, value.as_bytes())
}

/// A header value with no line end in it, since a line end closes the event's text for a reader.
fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_owned()
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use crate::timeline::WordEnds;
    use crate::{EventKind, TimedEvent, testing};

    #[test]
    fn the_synced_copy_goes_beside_the_song_and_is_never_the_song() {
        assert_eq!(
            synced_path(Path::new("a/song.mid")),
            Path::new("a/song.kar")
        );
        assert_eq!(
            synced_path(Path::new("a/song.kar")),
            Path::new("a/song-synced.kar")
        );
    }

    #[test]
    fn words_written_back_read_as_the_same_words() {
        let typed = "ka-ra-o-ke night\nwell\\-known song\n\nsecond page\n";
        let syllables = split_words(typed);
        assert_eq!(join_words(&syllables), typed);
        assert_eq!(split_words(&join_words(&syllables)), syllables);
        let dashes = split_words("a ---");
        assert_eq!(split_words(&join_words(&dashes)), dashes);
        assert_eq!(join_words(&[]), "");
    }

    fn timed(typed: &str, step: u32) -> KarWords {
        let mut syllables = split_words(typed);
        for (index, syllable) in syllables.iter_mut().enumerate() {
            syllable.tick = 480 + step * u32::try_from(index).unwrap();
        }
        KarWords {
            title: "A Song".to_owned(),
            artist: "A Singer".to_owned(),
            language: "ENGL".to_owned(),
            syllables,
        }
    }

    fn music(song: &Song) -> Vec<(u32, EventKind)> {
        song.events
            .iter()
            .map(|TimedEvent { tick, kind, .. }| (*tick, *kind))
            .collect()
    }

    const TWINKLE: &str = "Twin-kle twin-kle lit-tle star\nhow I won-der what you are\n\nUp a-bove";

    #[test]
    fn typed_words_split_into_lines_pages_words_and_syllables() {
        let got: Vec<(String, LineBreak)> =
            split_words("Twin-kle lit-tle\nstar\n\n\nUp a\\-bove -")
                .into_iter()
                .map(|s| (s.text, s.break_before))
                .collect();
        let want = [
            ("Twin", LineBreak::Page),
            ("kle", LineBreak::None),
            (" lit", LineBreak::None),
            ("tle", LineBreak::None),
            ("star", LineBreak::Line),
            ("Up", LineBreak::Page),
            (" a-bove", LineBreak::None),
            (" -", LineBreak::None),
        ];
        let want: Vec<(String, LineBreak)> =
            want.iter().map(|(t, b)| ((*t).to_owned(), *b)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn words_written_into_a_file_with_none_read_back_where_they_were_put() {
        let source = testing::instrumental();
        let words = timed(TWINKLE, 120);
        let out = write_soft_karaoke(&source, &words).unwrap();
        let song = testing::parse(&out);

        assert_eq!(song.flavor, KaraokeFlavor::SoftKaraoke);
        assert_eq!(song.lyrics.word_ends, WordEnds::AsWritten);
        assert_eq!(song.meta.title.as_deref(), Some("A Song"));
        assert_eq!(song.meta.artist.as_deref(), Some("A Singer"));
        assert_eq!(song.meta.language.as_deref(), Some("ENGL"));

        let lines: Vec<String> = song.lyrics.lines.iter().map(|l| l.text()).collect();
        assert_eq!(
            lines,
            [
                "Twinkle twinkle little star",
                "how I wonder what you are",
                "Up above"
            ]
        );
        assert_eq!(song.lyrics.lines[0].page, song.lyrics.lines[1].page);
        assert_ne!(song.lyrics.lines[1].page, song.lyrics.lines[2].page);

        let ticks: Vec<u32> = song.lyrics.syllable_ticks();
        let want: Vec<u32> = words.syllables.iter().map(|s| s.tick).collect();
        assert_eq!(ticks, want);
    }

    #[test]
    fn a_syllable_given_an_end_reads_back_ending_there() {
        let mut words = timed("one two\nthree", 480);
        // "two" closes its line at tick 960, and "three" does not start until 1440.
        words.syllables[1].end_tick = Some(1_100);
        let out = write_soft_karaoke(&testing::instrumental(), &words).unwrap();
        let song = testing::parse(&out);

        let first = &song.lyrics.lines[0];
        assert_eq!(first.text(), "one two");
        assert_eq!(first.syllables[1].start_tick, 960);
        assert_eq!(first.syllables[1].end_tick, 1_100);
        assert_eq!(first.end_tick, 1_100);
        // A syllable with no end still runs to the next one.
        assert_eq!(first.syllables[0].end_tick, 960);
        assert_eq!(song.lyrics.syllable_count(), 3);
    }

    #[test]
    fn the_music_is_the_same_events_at_the_same_ticks() {
        for source in [
            testing::instrumental(),
            testing::melody_and_accompaniment(),
            testing::tempo_change(),
        ] {
            let before = testing::parse(&source);
            let out = write_soft_karaoke(&source, &timed(TWINKLE, 120)).unwrap();
            let after = testing::parse(&out);
            assert_eq!(music(&before), music(&after));
            assert_eq!(
                before.tempo_map.tick_to_us(9_600),
                after.tempo_map.tick_to_us(9_600)
            );
        }
    }

    #[test]
    fn a_files_own_words_are_replaced_whichever_convention_held_them() {
        for source in [
            testing::lyric_events(),
            testing::soft_karaoke(),
            testing::named_text_track(),
            testing::two_lyric_tracks_disagreeing(),
        ] {
            let out = write_soft_karaoke(&source, &timed("New words on-ly", 240)).unwrap();
            let song = testing::parse(&out);
            assert_eq!(song.flavor, KaraokeFlavor::SoftKaraoke);
            assert_eq!(song.lyrics.plain_text().trim(), "New words only");
            assert_eq!(song.meta.title.as_deref(), Some("A Song"));
        }
    }

    #[test]
    fn a_format_0_file_takes_the_words_as_format_1() {
        let mut track = testing::channel_events(&[(0, [0x90, 60, 100]), (480, [0x80, 60, 0])]);
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut source = b"MThd\x00\x00\x00\x06\x00\x00\x00\x01\x01\xE0".to_vec();
        source.extend_from_slice(&smf::emit_chunk(&track));

        let before = testing::parse(&source);
        let out = write_soft_karaoke(&source, &timed("la la", 240)).unwrap();
        assert_eq!(&out[8..12], &[0, 1, 0, 3]);
        let after = testing::parse(&out);
        assert_eq!(music(&before), music(&after));
        assert_eq!(after.lyrics.plain_text().trim(), "la la");
    }

    #[test]
    fn a_damaged_track_is_copied_whole_and_the_words_still_land() {
        let source = testing::truncated_by_realtime_byte();
        let before = testing::parse(&source);
        let out = write_soft_karaoke(&source, &timed("la la", 240)).unwrap();
        let after = testing::parse(&out);
        assert_eq!(music(&before), music(&after));
        assert_eq!(after.flavor, KaraokeFlavor::SoftKaraoke);
    }

    #[test]
    fn no_words_and_no_midi_are_both_refused() {
        let empty = KarWords::default();
        assert!(matches!(
            write_soft_karaoke(&testing::instrumental(), &empty),
            Err(KarWriteError::NoWords)
        ));
        assert!(matches!(
            write_soft_karaoke(&testing::not_midi_text_file(), &timed("la", 1)),
            Err(KarWriteError::Smf(SmfError::NotMidi))
        ));
    }
}
