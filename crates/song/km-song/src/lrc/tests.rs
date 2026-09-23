//! Tests for the LRC reader. Every file here is written by hand: see `Every fixture in the tree is
//! synthetic` in `docs/decisions/repository.md`.

use super::*;
use crate::timeline::{LyricGranularity, LyricLine, WordEnds};

fn read(text: &str) -> Lrc {
    parse(text.as_bytes()).expect("parses")
}

fn lines(song: &Lrc) -> Vec<String> {
    song.timeline.lines.iter().map(LyricLine::text).collect()
}

fn starts(song: &Lrc) -> Vec<u32> {
    song.timeline
        .lines
        .iter()
        .map(|line| line.start_tick)
        .collect()
}

#[test]
fn every_timestamp_form_is_read_as_milliseconds() {
    let song = read(
        "[00:01]one\n[00:02.5]two\n[00:03.25]three\n[00:04.125]four\n[00:05:50]five\n[61:00.00]six\n",
    );
    assert_eq!(lines(&song), ["one", "two", "three", "four", "five", "six"]);
    assert_eq!(
        starts(&song),
        [1_000, 2_500, 3_250, 4_125, 5_500, 3_660_000]
    );
}

#[test]
fn a_line_timed_file_is_line_level_and_each_line_runs_to_the_next() {
    let song = read("[ti:Song]\n[ar:Someone]\n[00:10.00]First line here\n[00:14.00]Second line\n");
    assert_eq!(song.title.as_deref(), Some("Song"));
    assert_eq!(song.artist.as_deref(), Some("Someone"));
    assert!(!song.word_timed);
    assert_eq!(song.timeline.granularity(), LyricGranularity::LineLevel);
    assert!(song.timeline.lines_are_marked);
    let first = &song.timeline.lines[0];
    assert_eq!((first.start_tick, first.end_tick), (10_000, 14_000));
    let last = &song.timeline.lines[1];
    assert_eq!(last.end_tick, 14_000 + LAST_LINE_HOLD_MS);
}

#[test]
fn a_blank_timestamped_line_ends_the_line_before_it() {
    let song = read("[00:10.00]Before the solo\n[00:13.00]\n[00:40.00]After the solo\n");
    assert_eq!(lines(&song), ["Before the solo", "After the solo"]);
    assert_eq!(song.timeline.lines[0].end_tick, 13_000);
}

#[test]
fn a_line_with_several_timestamps_is_sung_at_each() {
    let song = read("[00:05.00]Verse\n[00:10.00][00:30.00]Chorus\n[00:20.00]Bridge\n");
    assert_eq!(lines(&song), ["Verse", "Chorus", "Bridge", "Chorus"]);
    assert_eq!(starts(&song), [5_000, 10_000, 20_000, 30_000]);
}

#[test]
fn a_positive_offset_makes_the_words_come_sooner_and_never_before_zero() {
    let song = read("[offset:+500]\n[00:00.20]early\n[00:10.00]later\n");
    assert_eq!(song.offset_ms, 500);
    assert_eq!(starts(&song), [0, 9_500]);
    let song = read("[offset:-250]\n[00:10.00]later\n");
    assert_eq!(starts(&song), [10_250]);
}

#[test]
fn word_tags_time_each_word_and_a_trailing_tag_ends_the_last() {
    let song = read(
        "[00:12.00]<00:12.00>I <00:12.50>see <00:13.00>trees<00:14.00>\n[00:20.00]<00:20.00>Next\n",
    );
    assert!(song.word_timed);
    assert_eq!(lines(&song), ["I see trees", "Next"]);
    let words = &song.timeline.lines[0].syllables;
    let spans: Vec<(u32, u32)> = words.iter().map(|w| (w.start_tick, w.end_tick)).collect();
    assert_eq!(
        spans,
        [(12_000, 12_500), (12_500, 13_000), (13_000, 14_000)]
    );
    assert_eq!(song.timeline.granularity(), LyricGranularity::SyllableLevel);
}

#[test]
fn whole_words_are_not_mistaken_for_syllables_spaced_apart() {
    // Every word ends in a space in the file, which reads as the spacing convention for syllables
    // unless the boundary moves to the start of the next word.
    let mut text = String::new();
    for line in 0..6 {
        let at = line * 5;
        text.push_str(&format!(
            "[00:{at:02}.00]<00:{at:02}.00>we <00:{at:02}.30>go <00:{at:02}.60>up <00:{at:02}.90>and <00:{:02}.20>on\n",
            at + 1
        ));
    }
    let song = read(&text);
    assert_eq!(song.timeline.word_ends, WordEnds::AsWritten);
    assert_eq!(lines(&song)[0], "we go up and on");
}

#[test]
fn a_word_tag_splitting_a_word_joins_its_syllables() {
    let song = read("[00:01.00]<00:01.00>Hel<00:01.40>lo <00:02.00>world\n");
    assert_eq!(lines(&song), ["Hello world"]);
    assert_eq!(song.timeline.lines[0].syllables.len(), 3);
}

#[test]
fn a_repeated_line_moves_its_word_tags_with_it() {
    let song = read("[00:10.00][00:30.00]<00:10.00>La <00:10.50>la\n");
    let second = &song.timeline.lines[1].syllables;
    assert_eq!(
        (second[0].start_tick, second[1].start_tick),
        (30_000, 30_500)
    );
}

#[test]
fn a_duet_part_marker_is_dropped_and_the_words_kept() {
    let song = read("[00:01.00]M: His line\n[00:03.00]F:Her line\n[00:05.00]D: Both\n");
    assert_eq!(lines(&song), ["His line", "Her line", "Both"]);
}

#[test]
fn a_second_line_at_the_same_time_is_a_translation_and_dropped() {
    let song = read("[00:01.00]Hola\n[00:01.00]Hello\n[00:03.00]Adiós\n[00:03.00]Goodbye\n");
    assert_eq!(lines(&song), ["Hola", "Adiós"]);
}

#[test]
fn a_bracket_that_is_not_a_timestamp_after_one_is_part_of_the_words() {
    let song = read("[00:01.00][Chorus] Sing <loud>\n");
    assert_eq!(lines(&song), ["[Chorus] Sing <loud>"]);
}

#[test]
fn a_file_with_no_timed_words_is_not_lrc() {
    assert_eq!(
        parse(b"[ti:Song]\n[ar:Someone]\nplain text\n").unwrap_err(),
        LrcError::NotLrc
    );
    assert_eq!(
        parse(b"[00:01.00]\n[00:02.00]\n").unwrap_err(),
        LrcError::NotLrc
    );
    assert_eq!(parse(b"").unwrap_err(), LrcError::NotLrc);
}

#[test]
fn line_endings_and_a_utf8_mark_are_read() {
    let song = read("\u{FEFF}[00:01.00]one\r\n[00:02.00]two\r\n");
    assert_eq!(lines(&song), ["one", "two"]);
    let song = parse(b"[00:01.00]one\r[00:02.00]two\r").expect("parses");
    assert_eq!(lines(&song), ["one", "two"]);
}

#[test]
fn a_utf16_file_is_read() {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "[ti:Canção]\n[00:01.00]Coração\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let song = parse(&bytes).expect("parses");
    assert_eq!(song.title.as_deref(), Some("Canção"));
    assert_eq!(lines(&song), ["Coração"]);
}

#[test]
fn a_legacy_encoding_is_detected_from_the_whole_file() {
    let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(
        "[ti:さくら]\n[00:01.00]さくら さくら\n[00:05.00]やよいの そらは\n[00:09.00]みわたす かぎり\n",
    );
    let song = parse(&bytes).expect("parses");
    assert_eq!(song.decoder.name(), "Shift_JIS");
    assert_eq!(song.title.as_deref(), Some("さくら"));
    assert_eq!(lines(&song)[0], "さくら さくら");
}
