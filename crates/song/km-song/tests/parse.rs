//! End-to-end parsing tests over synthetic MIDI bytes.
//!
//! The unit tests in the crate exercise each stage in isolation; these drive real file bytes
//! through [`Song::parse`], which is what the rest of the system actually calls.

use km_song::testing;
use km_song::{EventKind, KaraokeFlavor, LyricGranularity, ParseOptions, Song, Timebase};

fn parse(bytes: &[u8]) -> Song {
    Song::parse(bytes, &ParseOptions::default()).expect("fixture should parse")
}

#[test]
fn soft_karaoke_file_parses_completely() {
    let song = parse(&testing::soft_karaoke());

    assert_eq!(song.flavor, KaraokeFlavor::SoftKaraoke);
    assert_eq!(song.track_count, 3);
    assert_eq!(song.ticks_per_quarter, testing::TPQN);
    assert_eq!(song.meta.title.as_deref(), Some("Twinkle Twinkle"));
    assert_eq!(song.meta.artist.as_deref(), Some("The Test Fixtures"));
    assert_eq!(song.meta.language.as_deref(), Some("ENGL"));
    assert_eq!(song.meta.version.as_deref(), Some("0100"));
    assert_eq!(song.meta.copyright.as_deref(), Some("(c) 2026 nobody"));
    assert_eq!(song.meta.info, vec!["Generated fixture".to_owned()]);

    assert_eq!(song.lyrics.line_count(), 2);
    assert_eq!(song.lyrics.lines[0].text(), "Twinkle twinkle little star");
    assert_eq!(song.lyrics.lines[1].text(), "How I wonder what you are");
    assert_eq!(song.lyrics.granularity(), LyricGranularity::SyllableLevel);

    // Fourteen notes, one per syllable.
    assert_eq!(song.note_count(), 14);
    assert_eq!(song.sounding_channels(), vec![0]);
}

#[test]
fn soft_karaoke_page_and_line_markers_survive_parsing() {
    let song = parse(&testing::soft_karaoke());
    // The file opens with a page marker and uses a line marker before "How".
    assert_eq!(song.lyrics.page_count(), 1);
    assert_eq!(song.lyrics.lines[0].page, 0);
    assert_eq!(song.lyrics.lines[1].page, 0);
    // No stray marker characters leak into the text.
    assert!(!song.lyrics.plain_text().contains('/'));
    assert!(!song.lyrics.plain_text().contains('\\'));
}

#[test]
fn both_underscore_conventions_reach_the_words_as_spacing() {
    let song = parse(&testing::underscore_spacing());

    assert_eq!(song.lyrics.line_count(), 2);
    assert_eq!(song.lyrics.lines[0].text(), "Se apronta pra");
    assert_eq!(song.lyrics.lines[1].text(), "THE MELODY ");
    assert!(!song.lyrics.plain_text().contains('_'));
}

#[test]
fn a_file_written_in_chords_parses_to_its_words_alone() {
    let song = parse(&testing::chords_and_bracketed_lines());

    assert_eq!(song.flavor, KaraokeFlavor::LyricEvents);
    assert!(song.dialect.angle_starts_lines);
    assert!(song.dialect.annotations_are_marked);

    let text = song.lyrics.plain_text();
    assert!(!text.contains('%'), "a chord reached the words: {text:?}");
    assert!(
        !text.contains('<'),
        "a line mark reached the words: {text:?}"
    );
    assert_eq!(text.lines().next(), Some("cantaremos juntos"));

    // Forty lines and one syllable in each: the chords were the only thing making this look like
    // syllable-level timing, and it never was.
    assert_eq!(song.lyrics.syllable_count(), 40);
    assert_eq!(song.lyrics.granularity(), LyricGranularity::LineLevel);
}

#[test]
fn a_file_written_with_harmonica_tabs_parses_to_its_words_alone() {
    let song = parse(&testing::harmonica_tablature());

    assert_eq!(song.flavor, KaraokeFlavor::LyricEvents);
    assert!(song.dialect.harmonica_tabs);

    let text = song.lyrics.plain_text();
    assert!(
        !text.chars().any(|ch| ch.is_ascii_digit()),
        "a tab reached the words: {text:?}"
    );
    // Three verses of eight syllables, and the solos between them leave nothing behind.
    assert_eq!(song.lyrics.syllable_count(), 24);
    assert!(song.lyrics.line_count() > 1, "{text:?}");
}

#[test]
fn a_file_that_marks_no_word_ends_parses_with_narrow_dividers() {
    let song = parse(&testing::word_ends_unmarked());

    assert_eq!(song.flavor, KaraokeFlavor::LyricEvents);
    assert!(song.lyrics.word_ends.divided());
    assert_eq!(song.lyrics.syllable_count(), 240);
    assert_eq!(song.lyrics.granularity(), LyricGranularity::SyllableLevel);

    let text = song.lyrics.plain_text();
    assert!(
        text.contains(km_song::SYLLABLE_DIVIDER),
        "the space after every syllable is drawn narrow"
    );
    assert!(
        !text.contains(' '),
        "and none is left to read as a word end: {text:?}"
    );
}

#[test]
fn a_file_that_marks_no_word_boundary_at_all_parses_with_narrow_dividers() {
    let song = parse(&testing::word_boundaries_unmarked());

    assert_eq!(song.flavor, KaraokeFlavor::LyricEvents);
    assert!(song.lyrics.word_ends.divided());
    assert_eq!(song.lyrics.syllable_count(), 240);
    assert_eq!(song.lyrics.granularity(), LyricGranularity::SyllableLevel);

    let text = song.lyrics.plain_text();
    assert!(
        text.contains(km_song::SYLLABLE_DIVIDER),
        "the fragments are drawn apart rather than run together"
    );
    assert!(
        !text.contains(' '),
        "and on nothing that reads as a word end: {text:?}"
    );
}

#[test]
fn a_real_world_soft_karaoke_layout_parses() {
    // The magic on one track, the `@` header on the Words track, instrument names after that.
    let song = parse(&testing::soft_karaoke_real_layout());
    assert_eq!(song.flavor, KaraokeFlavor::SoftKaraoke);
    assert_eq!(song.meta.title.as_deref(), Some("The Real Title"));
    assert_eq!(song.meta.language.as_deref(), Some("ENGL"));
    // The second `@T` is a transcription credit, not a performer.
    assert_eq!(song.meta.artist, None);
    assert_eq!(
        song.meta.info,
        vec!["(Karaoke by Somebody Else)".to_owned()]
    );
    assert_eq!(song.lyrics.plain_text(), "Amor da vida");
    assert_eq!(song.sounding_channels(), vec![1, 9]);
}

#[test]
fn a_whole_song_in_the_real_world_layout_parses() {
    // The same layout as the test above, carrying an actual song rather than four syllables: this is
    // the shape that cost 67 points of artist recovery when the header was read off the wrong track,
    // and the length is part of what it covers — a browse list, a page count and the lyric preview
    // all behave differently on one line than on thirty.
    let song = parse(&testing::soft_karaoke_header_on_words_track());

    assert_eq!(song.flavor, KaraokeFlavor::SoftKaraoke);
    assert_eq!(
        song.meta.title.as_deref(),
        Some("The Long One"),
        "the title comes from an `@T` line on the Words track, not from a track name"
    );
    assert_eq!(song.meta.language.as_deref(), Some("ENGL"));
    assert_eq!(
        song.meta.artist, None,
        "the only other `@T` is a credit, and a credit is not a performer"
    );
    assert_eq!(song.meta.info, vec!["(Karaoke by Somebody)".to_owned()]);
    assert_eq!(song.lyrics.granularity(), LyricGranularity::SyllableLevel);
    assert_eq!(song.lyrics.line_count(), 32);
    assert_eq!(
        song.lyrics.lines[0].text(),
        "This song was made up for a test"
    );
    // Instrument tracks after the words, which is what the generic title fallback must not reach for.
    assert_eq!(song.sounding_channels(), vec![1, 5, 9]);
}

#[test]
fn a_producer_credit_in_the_first_position_does_not_become_the_title() {
    // The `@T` convention is positional — first the title, second the performer — so a studio in
    // front of them puts a studio in the title and shifts the real names along by one. Credits are
    // partitioned out *before* the two names are assigned, which is why both come out right here.
    let song = parse(&testing::soft_karaoke_producer_credit_first());

    assert_eq!(song.meta.title.as_deref(), Some("A Made Up Song"));
    assert_eq!(song.meta.artist.as_deref(), Some("A Made Up Singer"));
    assert_eq!(
        song.meta.info,
        vec!["Karaoke Fixture Studios - 2001".to_owned()],
        "the studio is kept, just not as the name of anything"
    );
}

#[test]
fn titles_made_of_marks_leave_a_song_with_no_name_at_all() {
    // A separator row is not a name, and neither column is filled in with one. The words are still
    // read, so the song plays and is searchable; what it has not got is a title, which curation
    // answers with the file's own name.
    let song = parse(&testing::soft_karaoke_titles_of_marks());

    assert_eq!(song.flavor, KaraokeFlavor::SoftKaraoke);
    assert_eq!(song.meta.title, None);
    assert_eq!(song.meta.artist, None);
    assert!(song.meta.info.is_empty(), "got {:?}", song.meta.info);
    assert_eq!(song.lyrics.line_count(), 2);
}

#[test]
fn lyric_event_file_parses() {
    let song = parse(&testing::lyric_events());
    assert_eq!(song.flavor, KaraokeFlavor::LyricEvents);
    assert_eq!(song.meta.title.as_deref(), Some("Mary Had A Little Lamb"));
    assert_eq!(song.lyrics.line_count(), 2);
    assert_eq!(song.lyrics.lines[0].text(), "Mary had a little lamb");
    assert_eq!(song.lyrics.lines[1].text(), "Its fleece was white as snow");
}

#[test]
fn named_text_track_file_parses() {
    let song = parse(&testing::named_text_track());
    assert_eq!(song.flavor, KaraokeFlavor::NamedTextTrack);
    assert_eq!(song.meta.title.as_deref(), Some("Row Your Boat"));
    assert_eq!(song.lyrics.lines[0].text(), "Row row row your boat");
    assert_eq!(song.lyrics.lines[1].text(), "Gently down the stream");
}

#[test]
fn a_credit_inside_a_track_name_stays_in_the_title() {
    // The credit filter partitions `@T` lines and deliberately does not reach a track name. A file
    // with no Soft Karaoke header has only its track names to offer, and an ugly-but-complete title
    // beats no title at all — 10.8% of a real corpus is this shape. Curation is where it gets tidied.
    let song = parse(&testing::named_text_track_credit_in_the_name());
    assert_eq!(song.flavor, KaraokeFlavor::NamedTextTrack);
    assert_eq!(
        song.meta.title.as_deref(),
        Some("A Made Up Song - Kar by Somebody")
    );
}

#[test]
fn unmarked_lyrics_are_split_by_the_gap_heuristic() {
    let song = parse(&testing::unmarked_lyrics());
    assert_eq!(song.flavor, KaraokeFlavor::LyricEvents);
    assert_eq!(
        song.lyrics.line_count(),
        2,
        "a two-second gap should break the line"
    );
    assert_eq!(song.lyrics.lines[0].text(), "first half here");
    assert_eq!(song.lyrics.lines[1].text(), "second half here");
}

#[test]
fn an_instrumental_file_is_playable_without_lyrics() {
    let song = parse(&testing::instrumental());
    assert_eq!(song.flavor, KaraokeFlavor::None);
    assert!(song.lyrics.is_empty());
    assert_eq!(song.lyrics.granularity(), LyricGranularity::None);
    assert_eq!(song.note_count(), 5);
    assert!(song.duration_ticks > 0, "the song still has a length");
}

#[test]
fn tempo_changes_are_reflected_in_wall_clock_time() {
    let song = parse(&testing::tempo_change());
    assert_eq!(song.tempo_map.change_count(), 2);

    let tpqn = u32::from(testing::TPQN);
    // Two beats at 120 BPM.
    assert_eq!(song.tempo_map.tick_to_ms(tpqn * 2), 1_000);
    // Two more at 60 BPM take twice as long.
    assert_eq!(song.tempo_map.tick_to_ms(tpqn * 4), 3_000);

    // The lyric timeline stays in ticks, so it is unaffected by where the tempo changes.
    let ticks = song.lyrics.syllable_ticks();
    assert_eq!(ticks, vec![0, 960, 1_920]);
}

#[test]
fn a_tempo_superseded_at_the_same_tick_does_not_govern() {
    // Two tempo events at tick 0, as Soft Karaoke files routinely write. The second is the real
    // one; keeping the first played every such file at the sequencer's placeholder 120 BPM.
    let song = parse(&testing::superseded_initial_tempo());
    assert_eq!(song.tempo_map.change_count(), 1);

    let tpqn = u32::from(testing::TPQN);
    // 60 BPM throughout, so a beat is a second -- not the 500 ms the placeholder would give.
    assert_eq!(song.tempo_map.tick_to_ms(tpqn), 1_000);
    assert_eq!(song.tempo_map.tick_to_ms(tpqn * 4), 4_000);
}

#[test]
fn velocity_zero_note_ons_become_note_offs() {
    let song = parse(&testing::velocity_zero_note_offs());
    let note_ons = song
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }))
        .count();
    let note_offs = song
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::NoteOff { .. }))
        .count();
    assert_eq!(note_ons, 2, "only the real note-ons should count as such");
    assert_eq!(
        note_offs, 2,
        "velocity-0 note-ons must be normalized to note-offs"
    );
    // And no NoteOn carries velocity 0 anywhere.
    assert!(
        !song
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::NoteOn { velocity: 0, .. }))
    );
}

#[test]
fn events_from_all_tracks_are_merged_in_tick_order() {
    let song = parse(&testing::melody_and_accompaniment());
    assert_eq!(song.track_count, 5);
    assert!(
        song.events.windows(2).all(|w| w[0].tick <= w[1].tick),
        "merged events must be sorted by tick"
    );
    // Melody on 0, chords on 1, drums on 9.
    assert_eq!(song.sounding_channels(), vec![0, 1, 9]);
}

#[test]
fn legacy_encoded_lyrics_are_decoded() {
    // Detection has very little to work with in two short words, so pin it via the manifest path,
    // which is what a real package does.
    let song = Song::parse(
        &testing::legacy_encoded_lyrics(),
        &ParseOptions::with_encoding("windows-1252"),
    )
    .expect("fixture should parse");
    assert_eq!(song.decoder.name(), "windows-1252");
    assert_eq!(song.lyrics.plain_text(), "cção não");
}

#[test]
fn legacy_encoded_lyrics_do_not_panic_without_a_declared_encoding() {
    // Whatever detection picks, the file must load and produce some text.
    let song = parse(&testing::legacy_encoded_lyrics());
    assert!(!song.lyrics.plain_text().is_empty());
}

#[test]
fn smpte_timed_files_convert_ticks_linearly() {
    let song = parse(&testing::smpte_timed());
    assert!(matches!(
        song.tempo_map.timebase(),
        Timebase::Smpte {
            ticks_per_second: 1_000
        }
    ));
    assert_eq!(
        song.ticks_per_quarter, 0,
        "a SMPTE file has no ticks per quarter note"
    );
    assert_eq!(song.tempo_map.tick_to_ms(1_000), 1_000);
    assert_eq!(song.tempo_map.tick_to_ms(2_000), 2_000);
}

// The sweep over every fixture is in `synthetic_corpus.rs`, with the rest of the corpus-wide
// invariants: two files walking the same list and asserting overlapping halves of the same thing is
// one file too many.

#[test]
fn garbage_input_is_an_error_not_a_panic() {
    assert!(Song::parse(b"", &ParseOptions::default()).is_err());
    assert!(Song::parse(b"not a midi file at all", &ParseOptions::default()).is_err());
    // A valid header claiming tracks that are not there.
    let truncated = b"MThd\x00\x00\x00\x06\x00\x01\x00\x02\x01\xE0";
    assert!(Song::parse(truncated, &ParseOptions::default()).is_err());
}

/// Both pressure kinds are parsed and kept distinct, whatever anybody downstream does with them.
///
/// `km-song` reads a file; it is not the place to decide that a message is not worth carrying. The
/// dropping happens one layer up, in `km-audio`'s sequencer, and there is a test there saying which
/// one it drops and why.
#[test]
fn both_kinds_of_aftertouch_are_parsed() {
    let song = parse(&testing::channel_pressure());
    let channel: Vec<_> = song
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::ChannelAftertouch { channel, value } => Some((channel, value)),
            _ => None,
        })
        .collect();
    let poly: Vec<_> = song
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::PolyAftertouch {
                channel,
                key,
                value,
            } => Some((channel, key, value)),
            _ => None,
        })
        .collect();
    assert_eq!(channel, vec![(0, 64), (0, 127)]);
    assert_eq!(poly, vec![(0, 60, 90)]);
}

/// A registered parameter reaches a caller as its five control changes, in the order it was written.
///
/// The order is the whole of the meaning. A data entry is a bare number until the two selectors in
/// front of it say what parameter it belongs to, so a reader that kept the five messages but not
/// their sequence would have thrown the value away while appearing to carry it.
#[test]
fn a_registered_parameter_keeps_its_order() {
    let song = parse(&testing::pitch_bend_range());
    let run: Vec<_> = song
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Controller {
                channel: 8,
                controller,
                value,
            } => Some((controller, value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        run,
        vec![(101, 0), (100, 0), (6, 12), (101, 127), (100, 127)],
        "RPN 0 set to twelve semitones, then the null parameter"
    );
}

/// Pitch bends survive as the raw 14-bit value, centered on 8192 rather than signed.
#[test]
fn pitch_bends_are_parsed_as_fourteen_bit_values() {
    let song = parse(&testing::pitch_bend_range());
    let bends: Vec<_> = song
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::PitchBend { channel, value } => Some((channel, value)),
            _ => None,
        })
        .collect();
    assert_eq!(bends, vec![(8, 5461), (2, 9557)]);
}

/// A track that simply stops, with no `0x2F` terminator, is read rather than run into.
///
/// The shape came from the synthesizer fork's own changelog — its MIDI reader "parsed past the end
/// of its own chunk" on exactly this — and this project uses `midly` rather than that reader, so the
/// point of the fixture is to hold the answer rather than to change it. It reads: the note and the
/// lyric both survive, and nothing from beyond the chunk appears.
#[test]
fn a_track_with_no_end_of_track_is_read_to_its_chunk_boundary() {
    let song = parse(&testing::track_without_end_of_track());
    assert_eq!(song.track_count, 1);
    assert!(
        song.note_count() > 0,
        "the note before the missing terminator"
    );
    assert_eq!(
        song.lyrics
            .lines
            .iter()
            .flat_map(|line| line.syllables.iter().map(|s| s.text.as_str()))
            .collect::<Vec<_>>(),
        vec!["unterminated"]
    );
}

#[test]
fn the_lyric_timeline_serializes_to_stable_json() {
    let song = parse(&testing::lyric_events());
    let json = serde_json::to_value(&song.lyrics).expect("timeline should serialize");
    let lines = json["lines"].as_array().expect("lines array");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["page"], 0);
    assert_eq!(lines[0]["start_tick"], 0);
    assert_eq!(lines[0]["syllables"][0]["text"], "Ma");
    assert_eq!(lines[0]["syllables"][0]["start_tick"], 0);
    assert_eq!(lines[0]["syllables"][0]["end_tick"], 240);
}

/// A track midly abandons mid-stream is recorded, not silently half-read.
///
/// This is the case `every_unreadable_fixture_is_refused_rather_than_half_read` never reached: all
/// four of its fixtures fail at the header, where an error is the right answer. Here the header is
/// fine and one *track* stops early, which is not an error — the file still plays — and so used to
/// be indistinguishable from a clean parse.
#[test]
fn a_track_that_stops_early_is_recorded_rather_than_passed_off_as_clean() {
    let song = parse(&testing::truncated_by_realtime_byte());
    assert_eq!(
        song.truncated_tracks,
        vec![0],
        "the illegal 0xFE abandons the rest of track 0"
    );
    assert_eq!(song.missing_tracks, 0, "the chunk itself parsed");
    assert!(
        !song
            .lyrics
            .lines
            .iter()
            .flat_map(|line| line.syllables.iter())
            .any(|s| s.text.contains("never read")),
        "everything after the bad byte is genuinely gone -- that is the defect being recorded"
    );
}

/// A note left sounding when its track's data runs out is stopped where the data stopped.
///
/// Without this the note has no note-off at all: the sequencer holds it, and because
/// `Sequencer::advance` early-returns for ever once the song is finished, nothing ever lifts it.
#[test]
fn a_truncated_track_gets_its_dangling_notes_turned_off() {
    let song = parse(&testing::truncated_by_realtime_byte());
    assert_eq!(song.repaired_notes, 1, "key 67 is down when the data ends");

    let last_tick = song
        .events
        .iter()
        .map(|e| e.tick)
        .max()
        .expect("the fixture has events");
    assert!(
        song.events.iter().any(|e| e.tick == last_tick
            && matches!(
                e.kind,
                EventKind::NoteOff {
                    channel: 0,
                    key: 67
                }
            )),
        "a note-off for the dangling key at the last tick the track reached"
    );
    assert!(
        every_note_is_turned_off(&song),
        "nothing may still be sounding once the events run out"
    );
}

/// The same repair applies to a well-formed file that simply ends holding a note.
///
/// Nothing is malformed in this one — 0.5% of the real corpus is like this — so a parser cannot
/// reject it. The repair is unconditional for exactly that reason: truncation is one way to arrive
/// at a hanging note and not the only one.
#[test]
fn an_unbalanced_note_on_is_turned_off_even_in_a_well_formed_file() {
    let song = parse(&testing::unbalanced_note_on());
    assert!(
        song.truncated_tracks.is_empty(),
        "this file is not malformed"
    );
    assert_eq!(song.repaired_notes, 1);
    assert!(
        every_note_is_turned_off(&song),
        "the held note is closed at the end of its track"
    );
}

/// A clean fixture is left exactly as it was: no repair, no truncation, nothing invented.
#[test]
fn a_well_formed_file_reports_nothing_and_gains_nothing() {
    for (name, build) in testing::FIXTURES {
        if name.starts_with("truncated_") || name.starts_with("unbalanced_") {
            continue;
        }
        let song = parse(&build());
        assert!(
            song.truncated_tracks.is_empty(),
            "{name}: reported a truncated track"
        );
        assert_eq!(song.missing_tracks, 0, "{name}: reported a missing track");
        assert_eq!(
            song.repaired_notes, 0,
            "{name}: synthesized a note-off it did not need"
        );
    }
}

/// Whether every note-on in the song is matched by a later note-off on the same channel and key.
fn every_note_is_turned_off(song: &Song) -> bool {
    let mut down: Vec<(u8, u8)> = Vec::new();
    for event in &song.events {
        match event.kind {
            EventKind::NoteOn { channel, key, .. } => {
                if !down.contains(&(channel, key)) {
                    down.push((channel, key));
                }
            }
            EventKind::NoteOff { channel, key } => {
                if let Some(at) = down.iter().position(|&n| n == (channel, key)) {
                    down.swap_remove(at);
                }
            }
            _ => {}
        }
    }
    down.is_empty()
}
