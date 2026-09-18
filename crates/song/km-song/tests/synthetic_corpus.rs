//! Every fixture, swept for the properties that must hold whatever the input is.
//!
//! Two layers, and this file is the second. [`parse.rs`](./parse.rs) pins what each *named* fixture
//! produces — the title this one recovers, the flavor that one is, the channel the melody is on.
//! Here nothing is named: both fixture lists are walked and asked the questions that have the same
//! answer for every file. No panic, ever. Events in tick order. Lines and syllables that do not run
//! backwards. No break marker leaking into the words. And, for
//! [`km_song::testing::UNREADABLE_FIXTURES`], an error rather than a half-read song.
//!
//! **A sweep is what catches the fixture nobody thought to assert about.** It used to walk a folder
//! of real karaoke files, which is where the open-endedness came from and why it counted refusals
//! (`rejected >= 3`) rather than naming them. Walking a `const` instead makes each refusal its own
//! assertion, and makes a fixture impossible to lose by deleting a file.

use km_song::testing::{FIXTURES, UNREADABLE_FIXTURES};
use km_song::{ParseOptions, Song};

#[test]
fn every_fixture_satisfies_the_parser_invariants() {
    for (name, build) in FIXTURES {
        let bytes = build();
        let song = Song::parse(&bytes, &ParseOptions::default())
            .unwrap_or_else(|e| panic!("{name} failed to parse: {e}"));

        assert!(song.track_count > 0, "{name}: parsed with no tracks");
        assert!(song.duration_ms() > 0, "{name}: should have a duration");
        assert!(
            song.events.windows(2).all(|w| w[0].tick <= w[1].tick),
            "{name}: events are not tick-ordered"
        );

        let mut previous_end = 0u32;
        for (i, line) in song.lyrics.lines.iter().enumerate() {
            assert!(!line.syllables.is_empty(), "{name}: line {i} is empty");
            assert!(
                line.end_tick >= line.start_tick,
                "{name}: line {i} ends before it starts"
            );
            assert!(
                line.start_tick >= previous_end.saturating_sub(1),
                "{name}: line {i} starts before the previous line ended"
            );
            previous_end = line.end_tick;

            for syllable in &line.syllables {
                assert!(
                    syllable.end_tick > syllable.start_tick,
                    "{name}: zero-length syllable in line {i}"
                );
                // Break markers are control characters, not content. One reaching the display shows
                // up as a stray glyph mid-lyric.
                assert!(
                    !syllable.text.starts_with(['/', '\\', '\r', '\n']),
                    "{name}: unstripped break marker in {:?}",
                    syllable.text
                );
                // An underscore in the words is markup about a space rather than a character
                // anybody sings, so none survives to be drawn. `km-lyrics scan` makes the same
                // assertion over a real corpus.
                assert!(
                    !syllable.text.contains('_'),
                    "{name}: unresolved space mark in {:?}",
                    syllable.text
                );
            }
        }

        assert!(
            song.lyrics.lines.windows(2).all(|w| w[0].page <= w[1].page),
            "{name}: page numbers decrease"
        );
    }
}

#[test]
fn every_unreadable_fixture_is_refused_rather_than_half_read() {
    // Each of these is a shape a real corpus is full of — bytes that are not MIDI, a text file under
    // a `.mid` name, a file an FTP transfer damaged, a header declaring nothing. An error is the
    // right answer to all four; a panic never is, and that is what this loop is really testing.
    for (name, build) in UNREADABLE_FIXTURES {
        let bytes = build();
        assert!(
            Song::parse(&bytes, &ParseOptions::default()).is_err(),
            "{name} must be refused"
        );
    }
}

#[test]
fn a_fixture_that_parses_can_always_be_sequenced_without_panicking() {
    // The engine is where a bad file would actually cause damage, so every fixture is played through
    // the sequencer with no audio device attached.
    for (name, build) in FIXTURES {
        let bytes = build();
        let song = Song::parse(&bytes, &ParseOptions::default()).expect("fixture should parse");

        let melody = km_suitability::Analysis::of(&song).melody_channel();
        let rendered = km_audio::render(
            km_audio::TestToneSource::new(8_000),
            std::sync::Arc::new(song),
            melody,
            &km_audio::RenderOptions {
                // Just enough to exercise the event dispatch, not the whole song.
                max_ms: 3_000,
                tail_ms: 0,
                ..Default::default()
            },
        );
        assert!(rendered.frames() > 0, "{name}: produced no audio at all");
        assert!(rendered.peak() <= 1.0, "{name}: output clipped");
    }
}

#[test]
fn parsing_is_deterministic() {
    // The same bytes must always give the same result: encoding detection and line inference both
    // involve heuristics, and a heuristic that varies between runs would be untestable.
    for (name, build) in FIXTURES {
        let bytes = build();
        let first = Song::parse(&bytes, &ParseOptions::default()).expect("parses");
        let second = Song::parse(&bytes, &ParseOptions::default()).expect("parses");
        assert_eq!(first.flavor, second.flavor, "{name}");
        assert_eq!(first.decoder.name(), second.decoder.name(), "{name}");
        assert_eq!(first.lyrics, second.lyrics, "{name}");
    }
    for (name, build) in UNREADABLE_FIXTURES {
        let bytes = build();
        assert!(
            Song::parse(&bytes, &ParseOptions::default()).is_err(),
            "{name}"
        );
        assert!(
            Song::parse(&bytes, &ParseOptions::default()).is_err(),
            "{name}: refused once and accepted the second time"
        );
    }
}
