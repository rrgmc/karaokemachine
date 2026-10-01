//! The song the promotional video sings: *Sing It Out Loud*, a power ballad that turns into rock.
//!
//! **The words and the music are written here, for this video.** No karaoke hit may be published in
//! it, and neither may a copy of one's tune. So this song takes the shape of a karaoke anthem, a
//! quiet verse and a loud chorus, and borrows nothing else.
//!
//! It is an ordinary Soft Karaoke file, the kind a karaoke catalog is full of. The machine parses
//! it, draws it and plays it exactly as it would any other.
//!
//! The shape, bar by bar, with `BAR` ticks to a bar:
//!
//! | bars   | tempo   | what plays                                                           |
//! |--------|---------|----------------------------------------------------------------------|
//! | 0      | 72 BPM  | the piano alone, in A minor                                          |
//! | 1 to 3 | 72 BPM  | the verse, sung softly over piano, strings and bass; a snare roll    |
//! | 4 to 11| 132 BPM | the chorus in C major, with drums and two distorted guitars          |
//! | 12     | 132 BPM | the last chord, left to ring                                         |

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

/// Ticks per quarter note.
const TPQN: u32 = 480;

/// An eighth note, which every rhythm here is counted in.
const E: u32 = TPQN / 2;

/// A bar of four beats.
pub const BAR: u32 = 4 * TPQN;

/// The bar the chorus starts on, where the tempo jumps.
const CHORUS: u32 = 4;

/// The bar holding the last chord.
const LAST: u32 = 12;

/// The song's title and performer, as its header gives them.
pub const TITLE: &str = "Sing It Out Loud";
pub const ARTIST: &str = "KaraokeMachine";

/// The bar after the last chord has rung out, which is where every track ends.
pub const END_BAR: u32 = LAST + 2;

// The channels, one part to each. Drums are on channel 10, as General MIDI requires.
const MELODY: u8 = 0;
const PIANO: u8 = 1;
const STRINGS: u8 = 2;
const BASS: u8 = 3;
const GUITAR: u8 = 4;
const GUITAR_WIDE: u8 = 5;
const DRUMS: u8 = 9;

// General MIDI programs, counted from zero.
const VOICE_OOHS: u8 = 53;
const SAW_LEAD: u8 = 81;
const GRAND_PIANO: u8 = 0;
const STRING_ENSEMBLE: u8 = 48;
const FINGERED_BASS: u8 = 33;
const PICKED_BASS: u8 = 34;
const DISTORTION_GUITAR: u8 = 30;
const OVERDRIVEN_GUITAR: u8 = 29;

// General MIDI drum keys.
const KICK: u8 = 36;
const SNARE: u8 = 38;
const CLOSED_HAT: u8 = 42;
const CRASH: u8 = 49;
const HIGH_TOM: u8 = 50;
const MID_TOM: u8 = 47;
const LOW_TOM: u8 = 45;
const FLOOR_TOM: u8 = 43;

/// A chord as its root and the notes of a close triad above middle C.
#[derive(Clone, Copy)]
struct Chord {
    root: u8,
    triad: [u8; 3],
}

const A_MINOR: Chord = Chord {
    root: 45,
    triad: [60, 64, 69],
};
const F_MAJOR: Chord = Chord {
    root: 41,
    triad: [60, 65, 69],
};
const C_MAJOR: Chord = Chord {
    root: 48,
    triad: [60, 64, 67],
};
const G_MAJOR: Chord = Chord {
    root: 43,
    triad: [62, 67, 71],
};

/// One chord to a bar, from bar 0 to the last chord.
const CHORDS: [Chord; 13] = [
    A_MINOR, F_MAJOR, C_MAJOR, G_MAJOR, // the verse
    C_MAJOR, G_MAJOR, A_MINOR, F_MAJOR, C_MAJOR, G_MAJOR, F_MAJOR, G_MAJOR, // the chorus
    C_MAJOR, // the last chord
];

/// One sung line: the bar it starts on, and its syllables.
///
/// A syllable is its text, its key, and its length in eighths. An empty text is a rest. A syllable
/// that ends a word carries the space after it, as a Soft Karaoke file writes it.
struct Line {
    bar: u32,
    new_page: bool,
    syllables: &'static [(&'static str, u8, u32)],
}

const LINES: [Line; 7] = [
    Line {
        bar: 1,
        new_page: true,
        syllables: &[
            ("", 0, 1),
            ("Turn ", 57, 1),
            ("the ", 60, 1),
            ("lights ", 64, 2),
            ("down ", 62, 1),
            ("low,", 60, 2),
        ],
    },
    Line {
        bar: 2,
        new_page: false,
        syllables: &[
            ("", 0, 1),
            ("type ", 64, 1),
            ("a ", 64, 1),
            ("num", 67, 2),
            ("ber ", 64, 1),
            ("in,", 62, 2),
        ],
    },
    Line {
        bar: 3,
        new_page: false,
        syllables: &[
            ("and ", 62, 1),
            ("when ", 62, 1),
            ("the ", 64, 1),
            ("mu", 67, 2),
            ("sic ", 69, 1),
            ("starts", 71, 2),
        ],
    },
    Line {
        bar: 4,
        new_page: true,
        syllables: &[
            ("Sing ", 67, 2),
            ("it ", 67, 1),
            ("out ", 69, 1),
            ("loud ", 72, 2),
            ("to", 72, 1),
            ("night,", 74, 7),
        ],
    },
    Line {
        bar: 6,
        new_page: false,
        syllables: &[
            ("ev", 76, 1),
            ("ery ", 74, 1),
            ("word ", 72, 2),
            ("lights ", 72, 1),
            ("up ", 69, 1),
            ("in ", 67, 2),
            ("time,", 69, 6),
        ],
    },
    Line {
        bar: 8,
        new_page: false,
        syllables: &[
            ("pass ", 67, 1),
            ("the ", 67, 1),
            ("mic, ", 72, 2),
            ("you're ", 72, 1),
            ("next ", 74, 1),
            ("in ", 76, 2),
            ("line,", 74, 6),
        ],
    },
    Line {
        bar: 10,
        new_page: false,
        syllables: &[
            ("ev", 69, 2),
            ("ery", 72, 2),
            ("bo", 74, 2),
            ("dy ", 76, 2),
            ("sing!", 79, 14),
        ],
    },
];

/// One event on one track, at an absolute tick.
enum Event {
    Name(&'static str),
    Text(String),
    Tempo(u32),
    Program(u8, u8),
    Control(u8, u8, u8),
    NoteOff(u8, u8),
    NoteOn(u8, u8, u8),
}

impl Event {
    /// The order of events that share a tick: settings first, then notes ending, then notes starting.
    fn rank(&self) -> u8 {
        match self {
            Event::NoteOff(..) => 1,
            Event::NoteOn(..) => 2,
            _ => 0,
        }
    }
}

#[derive(Default)]
struct Track {
    events: Vec<(u32, Event)>,
}

impl Track {
    fn at(&mut self, tick: u32, event: Event) {
        self.events.push((tick, event));
    }

    fn note(&mut self, tick: u32, channel: u8, key: u8, velocity: u8, length: u32) {
        self.at(tick, Event::NoteOn(channel, key, velocity));
        self.at(tick + length, Event::NoteOff(channel, key));
    }

    fn chord(&mut self, tick: u32, channel: u8, keys: &[u8], velocity: u8, length: u32) {
        for &key in keys {
            self.note(tick, channel, key, velocity, length);
        }
    }

    /// The events in the order a file stores them, with the text they borrow kept alive by `self`.
    fn into_midly(mut self, end: u32) -> Vec<(u32, Event)> {
        self.events
            .sort_by_key(|(tick, event)| (*tick, event.rank()));
        self.events.push((end, Event::Text(String::new())));
        self.events
    }
}

fn to_midly(events: &[(u32, Event)]) -> Vec<TrackEvent<'_>> {
    let mut previous = 0;
    let mut out = Vec::with_capacity(events.len());
    let last = events.len() - 1;
    for (index, (tick, event)) in events.iter().enumerate() {
        let delta = u28::new(tick - previous);
        previous = *tick;
        let kind = if index == last {
            TrackEventKind::Meta(MetaMessage::EndOfTrack)
        } else {
            match event {
                Event::Name(name) => TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
                Event::Text(text) => TrackEventKind::Meta(MetaMessage::Text(text.as_bytes())),
                Event::Tempo(us) => TrackEventKind::Meta(MetaMessage::Tempo(u24::new(*us))),
                Event::Program(channel, program) => midi(
                    *channel,
                    MidiMessage::ProgramChange {
                        program: u7::new(*program),
                    },
                ),
                Event::Control(channel, controller, value) => midi(
                    *channel,
                    MidiMessage::Controller {
                        controller: u7::new(*controller),
                        value: u7::new(*value),
                    },
                ),
                Event::NoteOff(channel, key) => midi(
                    *channel,
                    MidiMessage::NoteOff {
                        key: u7::new(*key),
                        vel: u7::new(64),
                    },
                ),
                Event::NoteOn(channel, key, velocity) => midi(
                    *channel,
                    MidiMessage::NoteOn {
                        key: u7::new(*key),
                        vel: u7::new(*velocity),
                    },
                ),
            }
        };
        out.push(TrackEvent { delta, kind });
    }
    out
}

fn midi(channel: u8, message: MidiMessage) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::new(channel),
        message,
    }
}

/// Microseconds per quarter note at a tempo in beats per minute.
fn tempo(bpm: u32) -> u32 {
    60_000_000 / bpm
}

/// The song as the bytes of a Standard MIDI File.
pub fn compose() -> Vec<u8> {
    let end = END_BAR * BAR;

    // The header track: the Soft Karaoke marks, the song's own header, and the two tempos.
    let mut header = Track::default();
    header.at(0, Event::Name("Soft Karaoke"));
    for line in [
        "@KMIDI KARAOKE FILE",
        "@V0100",
        "@LENGL",
        &format!("@T{TITLE}"),
        &format!("@T{ARTIST}"),
    ] {
        header.at(0, Event::Text(line.to_owned()));
    }
    header.at(0, Event::Tempo(tempo(72)));
    header.at(CHORUS * BAR, Event::Tempo(tempo(132)));

    // The words, and the guide melody that sings them.
    let mut words = Track::default();
    words.at(0, Event::Name("Words"));
    let mut melody = Track::default();
    melody.at(0, Event::Name("Melody"));
    melody.at(0, Event::Program(MELODY, VOICE_OOHS));
    melody.at(0, Event::Control(MELODY, 7, 100));
    melody.at(0, Event::Control(MELODY, 91, 70));
    melody.at(CHORUS * BAR, Event::Program(MELODY, SAW_LEAD));
    melody.at(CHORUS * BAR, Event::Control(MELODY, 7, 88));
    for line in &LINES {
        let mut tick = line.bar * BAR;
        let mut first = true;
        for &(text, key, eighths) in line.syllables {
            let length = eighths * E;
            if !text.is_empty() {
                let mark = match (first, line.new_page) {
                    (true, true) => "\\",
                    (true, false) => "/",
                    _ => "",
                };
                words.at(tick, Event::Text(format!("{mark}{text}")));
                // A small gap between notes, so a repeated key sounds twice.
                melody.note(tick, MELODY, key, 100, length - E / 8);
                first = false;
            }
            tick += length;
        }
    }

    let mut band = Track::default();
    band.at(0, Event::Name("Band"));
    for (channel, program, volume, pan, reverb) in [
        (PIANO, GRAND_PIANO, 100, 64, 60),
        (STRINGS, STRING_ENSEMBLE, 90, 64, 80),
        (BASS, FINGERED_BASS, 105, 64, 20),
        (GUITAR, DISTORTION_GUITAR, 92, 34, 30),
        (GUITAR_WIDE, OVERDRIVEN_GUITAR, 80, 94, 40),
        (DRUMS, 0, 110, 64, 40),
    ] {
        band.at(0, Event::Program(channel, program));
        band.at(0, Event::Control(channel, 7, volume));
        band.at(0, Event::Control(channel, 10, pan));
        band.at(0, Event::Control(channel, 91, reverb));
    }
    band.at(CHORUS * BAR, Event::Program(BASS, PICKED_BASS));

    for (bar, chord) in CHORDS.iter().enumerate() {
        let bar = bar as u32;
        let start = bar * BAR;
        let Chord { root, triad } = *chord;
        let fifth = root + 7;

        if bar < CHORUS {
            // The verse: the piano rolls the chord in eighths, and the strings hold it.
            let pattern = [
                root,
                fifth,
                root + 12,
                triad[0],
                triad[1],
                triad[2],
                triad[1],
                triad[0],
            ];
            for (step, key) in pattern.into_iter().enumerate() {
                let velocity = 72 + 6 * (step % 2 == 0) as u8 + 6 * bar as u8;
                band.note(start + step as u32 * E, PIANO, key, velocity, 2 * E);
            }
            if bar >= 1 {
                band.chord(start, STRINGS, &triad, 80, BAR);
                band.note(start, BASS, root - 12, 84, BAR - E / 4);
            }
            if bar == CHORUS - 1 {
                // The strings swell into the chorus.
                for step in 0..16 {
                    band.at(
                        start + step * BAR / 16,
                        Event::Control(STRINGS, 11, (70 + 57 * step / 15) as u8),
                    );
                }
                band.at(CHORUS * BAR, Event::Control(STRINGS, 11, 127));
                drum_roll(&mut band, start);
            }
        } else if bar < LAST {
            // The chorus: stabs on the piano, eighths on the bass and the guitars, and a rock beat.
            let power = [root, fifth, root + 12];
            for beat in 0..4 {
                band.chord(
                    start + beat * TPQN,
                    PIANO,
                    &triad.map(|k| k + 12),
                    96,
                    TPQN - E / 4,
                );
                band.note(start + beat * TPQN, PIANO, root, 96, TPQN - E / 4);
            }
            band.chord(start, STRINGS, &triad.map(|k| k + 12), 92, BAR);
            band.chord(start, GUITAR_WIDE, &power.map(|k| k + 12), 88, BAR - E / 4);
            for step in 0..8 {
                let accent = if step % 2 == 0 { 110 } else { 92 };
                band.chord(start + step * E, GUITAR, &power, accent, E - E / 6);
                band.note(start + step * E, BASS, root - 12, accent, E - E / 6);
            }
            rock_beat(&mut band, bar, start);
        } else {
            // The last chord, struck once by everybody and left to ring.
            band.chord(
                start,
                PIANO,
                &[root - 12, root, triad[0], triad[1], triad[2], triad[2] + 12],
                110,
                2 * BAR,
            );
            band.chord(start, STRINGS, &triad.map(|k| k + 12), 100, 2 * BAR - E);
            band.chord(start, GUITAR, &[root, fifth, root + 12], 115, 2 * BAR - E);
            band.chord(
                start,
                GUITAR_WIDE,
                &[root + 12, fifth + 12, root + 24],
                100,
                2 * BAR - E,
            );
            band.note(start, BASS, root - 12, 115, 2 * BAR - E);
            band.note(start, DRUMS, CRASH, 127, TPQN);
            band.note(start, DRUMS, KICK, 127, E);
        }
    }

    let tracks = [header, words, melody, band].map(|track| track.into_midly(end));
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPQN as u16)),
    ));
    smf.tracks = tracks.iter().map(|events| to_midly(events)).collect();
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)
        .expect("writing to memory cannot fail");
    bytes
}

/// The last bar of the verse: a kick, a snare roll that grows, and a run down the toms into the chorus.
fn drum_roll(band: &mut Track, start: u32) {
    band.note(start, DRUMS, KICK, 90, E);
    let sixteenth = E / 2;
    for step in 0..8 {
        let velocity = 40 + 10 * step as u8;
        band.note(
            start + 2 * TPQN + step * sixteenth,
            DRUMS,
            SNARE,
            velocity,
            sixteenth,
        );
    }
    for (step, tom) in [
        HIGH_TOM, HIGH_TOM, MID_TOM, MID_TOM, LOW_TOM, LOW_TOM, FLOOR_TOM, FLOOR_TOM,
    ]
    .into_iter()
    .enumerate()
    {
        let step = step as u32;
        band.note(
            start + 3 * TPQN + step * sixteenth / 2,
            DRUMS,
            tom,
            100 + 3 * step as u8,
            sixteenth / 2,
        );
    }
}

/// One bar of the chorus beat, with a crash on the first bar of every line and a fill before the end.
fn rock_beat(band: &mut Track, bar: u32, start: u32) {
    if (bar - CHORUS).is_multiple_of(2) {
        band.note(start, DRUMS, CRASH, 118, TPQN);
    }
    for step in 0..8 {
        band.note(
            start + step * E,
            DRUMS,
            CLOSED_HAT,
            if step % 2 == 0 { 96 } else { 70 },
            E / 2,
        );
    }
    for tick in [0, 2 * TPQN, 2 * TPQN + E] {
        band.note(start + tick, DRUMS, KICK, 118, E);
    }
    let fill = bar == LAST - 1;
    for beat in [1, 3] {
        if fill && beat == 3 {
            continue;
        }
        band.note(start + beat * TPQN, DRUMS, SNARE, 120, E);
    }
    if fill {
        let sixteenth = E / 2;
        for (step, tom) in [SNARE, HIGH_TOM, MID_TOM, FLOOR_TOM]
            .into_iter()
            .enumerate()
        {
            band.note(
                start + 3 * TPQN + step as u32 * sixteenth,
                DRUMS,
                tom,
                118,
                sixteenth,
            );
        }
    }
}
