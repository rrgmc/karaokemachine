//! Tests for the UltraStar reader. Every file here is written by hand: see `Every fixture in the
//! tree is synthetic` in `docs/decisions/repository.md`.

use super::*;

/// A header of `#BPM:300` and `#GAP:1000`: a beat is 50 ms, and beat 0 is one second in.
const HEADER: &str = "#TITLE:Song\n#ARTIST:Someone\n#MP3:Someone - Song.mp3\n#BPM:300\n#GAP:1000\n";

fn read(text: &str) -> UltraStar {
    parse(text.as_bytes()).expect("parses")
}

fn lines(song: &UltraStar) -> Vec<String> {
    song.timeline.lines.iter().map(LyricLine::text).collect()
}

use crate::timeline::LyricLine;

#[test]
fn a_note_starts_and_ends_on_its_beats() {
    let song = read(&format!(
        "{HEADER}: 0 4 0 Hel\n: 4 2 0 lo\n- 8\n: 10 6 0 world\nE\n"
    ));
    assert_eq!(lines(&song), ["Hello", "world"]);
    let first = &song.timeline.lines[0].syllables;
    assert_eq!((first[0].start_tick, first[0].end_tick), (1_000, 1_200));
    assert_eq!((first[1].start_tick, first[1].end_tick), (1_200, 1_300));
    let second = &song.timeline.lines[1].syllables[0];
    assert_eq!((second.start_tick, second.end_tick), (1_500, 1_800));
    assert!(song.timeline.lines_are_marked);
}

#[test]
fn a_phrase_ends_where_its_last_note_does_and_not_at_the_next_phrase() {
    // Without the note's own length, `lo` would be wiped across the whole second of silence.
    let song = read(&format!("{HEADER}: 0 2 0 lo\n- 4\n: 20 2 0 next\nE\n"));
    assert_eq!(song.timeline.lines[0].end_tick, 1_100);
}

#[test]
fn a_space_between_syllables_is_a_word_boundary_and_the_ends_of_a_line_lose_theirs() {
    let song = read(&format!(
        "{HEADER}: 0 1 0  Hel\n: 1 1 0 lo \n: 2 1 0 big\n: 3 1 0  world \nE\n"
    ));
    assert_eq!(lines(&song), ["Hello big world"]);
}

#[test]
fn a_decimal_comma_is_read_in_every_number() {
    let song = read(
        "#TITLE:Song\n#MP3:a.mp3\n#VERSION:1,00\n#BPM:150,5\n#GAP:1000,4\n: 0 1 0 a\n: 301 1 0 b\nE\n",
    );
    let starts: Vec<u32> = song
        .timeline
        .lines
        .iter()
        .map(|line| line.start_tick)
        .collect();
    // 301 beats of a quarter of 150.5 BPM is 30,000 ms.
    assert_eq!(starts, [1_000, 31_000]);
}

#[test]
fn relative_beats_are_shifted_by_each_break() {
    let song = read(
        "#TITLE:Song\n#MP3:a.mp3\n#RELATIVE:yes\n#BPM:300\n#GAP:0\n: 0 2 0 one\n- 4 20\n: 0 2 0 two\n- 4 10\n: 2 2 0 three\nE\n",
    );
    let starts: Vec<u32> = song
        .timeline
        .lines
        .iter()
        .map(|line| line.start_tick)
        .collect();
    assert_eq!(starts, [0, 1_000, 1_600]);
    assert!(song.relative);
}

#[test]
fn only_the_first_number_of_a_break_counts_in_an_absolute_file() {
    let song = read(&format!("{HEADER}: 0 2 0 one\n- 4 100\n: 10 2 0 two\nE\n"));
    assert_eq!(song.timeline.lines[1].start_tick, 1_500);
}

#[test]
fn a_held_vowel_lengthens_the_syllable_before_it() {
    let song = read(&format!("{HEADER}: 0 2 0 oh\n: 2 6 0 ~\nE\n"));
    assert_eq!(lines(&song), ["oh"]);
    assert_eq!(song.timeline.lines[0].syllables[0].end_tick, 1_400);
}

#[test]
fn an_unversioned_file_in_a_legacy_code_page_is_detected() {
    let mut bytes = HEADER.as_bytes().to_vec();
    // `Coração` in windows-1252, with `ç` and `ã` outside ASCII.
    bytes.extend_from_slice(b": 0 2 0 Cora\xE7\xE3o\n: 2 2 0  sem\n: 4 2 0  voc\xEA\nE\n");
    let song = parse(&bytes).expect("parses");
    assert_eq!(lines(&song), ["Coração sem você"]);
}

#[test]
fn a_named_language_steers_detection_between_two_code_pages() {
    // Five accented letters in ASCII words read as well in windows-1250 as in windows-1252; the
    // Portuguese file is the second.
    let mut bytes = b"#TITLE:Ai Se Eu Te Pego\n#ARTIST:Michel Tel\xF3\n#LANGUAGE:Portuguese\n#MP3:a.mp3\n#BPM:300\n".to_vec();
    bytes.extend_from_slice(
        b": 0 2 0 N\xF3s\n: 2 2 0  va\n: 4 2 0 mos\n- 6\n: 8 2 0 Voc\xEA\n: 10 2 0  me\n- 12\n: 14 2 0 voc\xEA\n: 16 2 0  me\n: 18 2 0  mata\nE\n",
    );
    let song = parse(&bytes).expect("parses");
    assert_eq!(song.decoder.name(), "windows-1252");
    assert!(song.timeline.plain_text().contains("Você"));
}

#[test]
fn a_declared_encoding_is_read_in_the_spelling_the_format_uses() {
    let text = format!("#ENCODING:UTF8\n{HEADER}: 0 2 0 Coração\nE\n");
    let song = read(&text);
    assert_eq!(song.decoder.name(), "UTF-8");
    assert_eq!(lines(&song), ["Coração"]);
}

#[test]
fn every_line_ending_reads_the_same() {
    let lf = format!("{HEADER}: 0 2 0 one\n- 4\n: 10 2 0 two\nE\n");
    let crlf = lf.replace('\n', "\r\n");
    let cr = lf.replace('\n', "\r");
    let expected = lines(&read(&lf));
    assert_eq!(lines(&read(&crlf)), expected);
    assert_eq!(lines(&read(&cr)), expected);
}

#[test]
fn audio_is_named_by_audio_before_mp3() {
    let song = read("#TITLE:Song\n#MP3:old.mp3\n#AUDIO:new.mp3\n#BPM:300\n: 0 1 0 a\nE\n");
    assert_eq!(song.audio, "new.mp3");
}

#[test]
fn nothing_after_the_end_marker_is_read() {
    let song = read(&format!("{HEADER}: 0 2 0 sung\nE\n: 10 2 0 never\n"));
    assert_eq!(lines(&song), ["sung"]);
}

#[test]
fn a_text_file_with_no_title_is_not_an_ultrastar_file() {
    let error = parse(b"Read me first.\r\nThanks for downloading.\r\n").unwrap_err();
    assert_eq!(error, UltraStarError::NotUltraStar);
}

#[test]
fn what_cannot_be_drawn_as_one_voice_over_named_audio_is_refused() {
    let cases = [
        (
            format!("{HEADER}P1\n: 0 2 0 me\nP2\n: 4 2 0 you\nE\n"),
            UltraStarError::Duet,
        ),
        (
            "#TITLE:Song\n#VERSION:2.0.0\n#MP3:a.mp3\n#BPM:300\n: 0 1 0 a\nE\n".to_owned(),
            UltraStarError::UnsupportedVersion("2.0.0".to_owned()),
        ),
        (
            "#TITLE:Song\n#VERSION:1.0.0\n#RELATIVE:yes\n#MP3:a.mp3\n#BPM:300\n: 0 1 0 a\nE\n"
                .to_owned(),
            UltraStarError::RelativeInVersioned,
        ),
        (
            "#TITLE:Song\n#VIDEO:a.mp4\n#BPM:300\n: 0 1 0 a\nE\n".to_owned(),
            UltraStarError::NoAudio,
        ),
        (
            "#TITLE:Song\n#MP3:a.mp3\n: 0 1 0 a\nE\n".to_owned(),
            UltraStarError::NoBpm,
        ),
        (format!("{HEADER}E\n"), UltraStarError::NoNotes),
    ];
    for (text, expected) in cases {
        assert_eq!(parse(text.as_bytes()).unwrap_err(), expected, "{text}");
    }
}

#[test]
fn the_header_says_who_and_what() {
    let song = read(&format!(
        "#LANGUAGE:Portuguese\n#VIDEO:clip.mpg\n{HEADER}: 0 1 0 a\nE\n"
    ));
    assert_eq!(song.title, "Song");
    assert_eq!(song.artist.as_deref(), Some("Someone"));
    assert_eq!(song.language.as_deref(), Some("Portuguese"));
    assert_eq!(song.video.as_deref(), Some("clip.mpg"));
    assert_eq!(song.audio, "Someone - Song.mp3");
}

#[test]
fn a_version_on_a_legacy_code_page_is_read_in_that_code_page() {
    let mut bytes =
        b"#TITLE:Schrei\n#ARTIST:Die \xC4rzte\n#MP3:DIE \xC4RZTE.mp3\n#VERSION:1.1\n#BPM:300\n"
            .to_vec();
    bytes.extend_from_slice(b": 0 2 0 wirk\n: 2 2 0 lich\nE\n");
    let song = parse(&bytes).expect("parses");
    assert_eq!(song.audio, "DIE ÄRZTE.mp3");
    assert_eq!(song.artist.as_deref(), Some("Die Ärzte"));
}

#[test]
fn a_space_alone_at_the_edge_of_a_line_draws_nothing() {
    let song = read(&format!(
        "{HEADER}: 0 2 0 one\n- 4\n: 10 2 0  \n: 12 2 0 two\n: 14 2 0  \nE\n"
    ));
    assert_eq!(lines(&song), ["one", "two"]);
}

#[test]
fn a_song_built_from_a_timeline_counts_milliseconds_and_reads_ahead_in_half_seconds() {
    let parsed = read(&format!(
        "{HEADER}: 0 4 0 Hel\n: 4 2 0 lo\n- 8\n: 10 6 0 world\nE\n"
    ));
    let song = song_from_timeline(parsed.timeline.clone());
    assert_eq!(song.tempo_map.ms_to_tick(1_500), 1_500);
    assert_eq!(song.tempo_map.tick_to_ms(1_800), 1_800);
    assert_eq!(song.duration_ms(), 1_800);
    // Half a second, not the 0 a timecode file's `ticks_per_quarter` is: a lyric view reading
    // ahead eight beats would otherwise read ahead eight milliseconds.
    assert_eq!(song.beat_ticks(), 500);
    assert!(song.events.is_empty());
    assert_eq!(song.lyrics, parsed.timeline);
}

#[test]
fn a_stored_timeline_reads_back_as_it_was_written() {
    let song = read(&format!(
        "{HEADER}: 0 4 0 Hel\n: 4 2 0 lo\n- 8\n: 10 6 0 world\nE\n"
    ));
    let json = serde_json::to_string(&song.timeline).expect("serializes");
    let back: LyricTimeline = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, song.timeline);
}
