//! Synthetic karaoke files for tests and manual inspection.
//!
//! Enabled by the `testing` feature.
//!
//! These are written as standard MIDI bytes by hand rather than through a MIDI writing library, so
//! the parser is tested against bytes we control end to end — a round trip through the same crate
//! that parses them would not prove the format was read correctly.
//!
//! **This is the whole of the committed test material**, and every fixture in it is named for the
//! case it covers rather than for anything it sounds like. When a real file exposes a bug, the shape
//! is distilled into a builder here and the file stays wherever it was found: see the `Every fixture
//! in the tree is synthetic` decision in `docs/decisions/`.
//!
//! Synthetic files cover the shapes we know about, which is not the same as covering the shapes that
//! exist. Lyric-format surprises come from real files, and what answers that standing risk is a
//! corpus swept by `km-lyrics scan` outside this repository — see the top risk in
//! `docs/ARCHITECTURE.md`.
//!
//! Two lists, because a parser is judged as much by what it declines as by what it reads:
//! [`FIXTURES`] must all parse and [`UNREADABLE_FIXTURES`] must all be refused.

/// Ticks per quarter note used by every fixture, so tick maths in tests stays readable.
pub const TPQN: u16 = 480;

/// Microseconds per quarter note at 120 BPM.
pub const TEMPO_120: u32 = 500_000;

/// Builds one MIDI track's byte payload.
#[derive(Default)]
struct TrackWriter {
    data: Vec<u8>,
}

impl TrackWriter {
    fn new() -> Self {
        Self::default()
    }

    /// Writes a variable-length quantity, as the SMF spec defines deltas.
    fn push_varlen(&mut self, mut value: u32) {
        let mut buffer = [0u8; 4];
        let mut len = 0;
        loop {
            buffer[len] = (value & 0x7F) as u8;
            len += 1;
            value >>= 7;
            if value == 0 {
                break;
            }
        }
        for i in (0..len).rev() {
            let last = i == 0;
            self.data.push(buffer[i] | if last { 0x00 } else { 0x80 });
        }
    }

    fn meta(&mut self, delta: u32, meta_type: u8, payload: &[u8]) -> &mut Self {
        self.push_varlen(delta);
        self.data.push(0xFF);
        self.data.push(meta_type);
        self.push_varlen(payload.len() as u32);
        self.data.extend_from_slice(payload);
        self
    }

    /// A text meta event (`0x01`) — where Soft Karaoke keeps both its header and its lyrics.
    fn text(&mut self, delta: u32, text: &[u8]) -> &mut Self {
        self.meta(delta, 0x01, text)
    }

    /// A copyright meta event (`0x02`).
    fn copyright(&mut self, delta: u32, text: &[u8]) -> &mut Self {
        self.meta(delta, 0x02, text)
    }

    /// A track name meta event (`0x03`).
    fn track_name(&mut self, delta: u32, name: &[u8]) -> &mut Self {
        self.meta(delta, 0x03, name)
    }

    /// A lyric meta event (`0x05`) — the standard MIDI karaoke convention.
    fn lyric(&mut self, delta: u32, text: &[u8]) -> &mut Self {
        self.meta(delta, 0x05, text)
    }

    /// A tempo meta event (`0x51`), in microseconds per quarter note.
    fn tempo(&mut self, delta: u32, us_per_quarter: u32) -> &mut Self {
        let b = us_per_quarter.to_be_bytes();
        self.meta(delta, 0x51, &[b[1], b[2], b[3]])
    }

    fn note_on(&mut self, delta: u32, channel: u8, key: u8, velocity: u8) -> &mut Self {
        self.push_varlen(delta);
        self.data
            .extend_from_slice(&[0x90 | (channel & 0x0F), key & 0x7F, velocity & 0x7F]);
        self
    }

    fn note_off(&mut self, delta: u32, channel: u8, key: u8) -> &mut Self {
        self.push_varlen(delta);
        self.data
            .extend_from_slice(&[0x80 | (channel & 0x0F), key & 0x7F, 0x40]);
        self
    }

    /// A note-on with velocity 0, which the spec makes equivalent to a note-off.
    fn note_on_velocity_zero(&mut self, delta: u32, channel: u8, key: u8) -> &mut Self {
        self.note_on(delta, channel, key, 0)
    }

    fn program_change(&mut self, delta: u32, channel: u8, program: u8) -> &mut Self {
        self.push_varlen(delta);
        self.data
            .extend_from_slice(&[0xC0 | (channel & 0x0F), program & 0x7F]);
        self
    }

    /// A control change (`0xB0`).
    fn controller(&mut self, delta: u32, channel: u8, controller: u8, value: u8) -> &mut Self {
        self.push_varlen(delta);
        self.data
            .extend_from_slice(&[0xB0 | (channel & 0x0F), controller & 0x7F, value & 0x7F]);
        self
    }

    /// A pitch bend (`0xE0`), taking the 14-bit value centered on 8192 and splitting it.
    fn pitch_bend(&mut self, delta: u32, channel: u8, value: u16) -> &mut Self {
        self.push_varlen(delta);
        self.data.extend_from_slice(&[
            0xE0 | (channel & 0x0F),
            (value & 0x7F) as u8,
            ((value >> 7) & 0x7F) as u8,
        ]);
        self
    }

    /// Establishes a registered parameter: select it, state its value, then leave the null parameter.
    ///
    /// The five control changes are what a file writes to set a pitch bend range, and the order is
    /// the whole of their meaning — a data entry is a bare number until a selector says what it is
    /// for.
    fn registered_parameter(
        &mut self,
        delta: u32,
        channel: u8,
        parameter: (u8, u8),
        value: u8,
    ) -> &mut Self {
        self.controller(delta, channel, 101, parameter.0);
        self.controller(0, channel, 100, parameter.1);
        self.controller(0, channel, 6, value);
        self.controller(0, channel, 101, 127);
        self.controller(0, channel, 100, 127);
        self
    }

    /// Whole-channel pressure (`0xD0`), which takes one data byte.
    fn channel_aftertouch(&mut self, delta: u32, channel: u8, value: u8) -> &mut Self {
        self.push_varlen(delta);
        self.data
            .extend_from_slice(&[0xD0 | (channel & 0x0F), value & 0x7F]);
        self
    }

    /// Per-note pressure (`0xA0`), which takes the key and then the pressure.
    fn poly_aftertouch(&mut self, delta: u32, channel: u8, key: u8, value: u8) -> &mut Self {
        self.push_varlen(delta);
        self.data
            .extend_from_slice(&[0xA0 | (channel & 0x0F), key & 0x7F, value & 0x7F]);
        self
    }

    /// Bytes written into the track exactly as given, with no delta and no framing.
    ///
    /// Every other method here encodes something well-formed, which is precisely why the fixtures
    /// could not reach the shapes that break a parser mid-track. `track_without_end_of_track` had to
    /// reach into `data` by hand for the same reason; this is that escape hatch, named.
    fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.data.extend_from_slice(bytes);
        self
    }

    /// A note of `duration` ticks starting `delta` ticks after the previous event.
    fn note(&mut self, delta: u32, channel: u8, key: u8, velocity: u8, duration: u32) -> &mut Self {
        self.note_on(delta, channel, key, velocity);
        self.note_off(duration, channel, key);
        self
    }

    fn finish(mut self) -> Vec<u8> {
        // End of track is mandatory.
        self.meta(0, 0x2F, &[]);
        let mut chunk = Vec::with_capacity(self.data.len() + 8);
        chunk.extend_from_slice(b"MTrk");
        chunk.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(&self.data);
        chunk
    }
}

/// Assembles a complete file from finished track chunks.
fn smf(tracks: Vec<Vec<u8>>) -> Vec<u8> {
    let format: u16 = if tracks.len() > 1 { 1 } else { 0 };
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
    out.extend_from_slice(&TPQN.to_be_bytes());
    for track in tracks {
        out.extend_from_slice(&track);
    }
    out
}

/// Assembles a SMPTE-timed file, to exercise the non-metrical timebase.
fn smf_smpte(tracks: Vec<Vec<u8>>, fps: u8, subframes: u8) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
    // Negative frames-per-second in the high byte marks SMPTE timing.
    out.push((!fps).wrapping_add(1));
    out.push(subframes);
    for track in tracks {
        out.extend_from_slice(&track);
    }
    out
}

/// A Soft Karaoke (`.kar`) file: `@`-prefixed header on track 0, lyrics on track 1, music on 2.
pub fn soft_karaoke() -> Vec<u8> {
    let mut header = TrackWriter::new();
    header
        .track_name(0, b"Soft Karaoke")
        .text(0, b"@KMIDI KARAOKE FILE")
        .text(0, b"@V0100")
        .text(0, b"@LENGL")
        .text(0, b"@IGenerated fixture")
        .text(0, b"@TTwinkle Twinkle")
        .text(0, b"@TThe Test Fixtures")
        .copyright(0, b"(c) 2026 nobody")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words
        .track_name(0, b"Words")
        .text(0, b"\\Twin")
        .text(240, b"kle ")
        .text(240, b"twin")
        .text(240, b"kle ")
        .text(240, b"lit")
        .text(240, b"tle ")
        .text(240, b"star")
        .text(480, b"/How ")
        .text(240, b"I ")
        .text(240, b"won")
        .text(240, b"der ")
        .text(240, b"what ")
        .text(240, b"you ")
        .text(240, b"are");

    let mut music = TrackWriter::new();
    music.track_name(0, b"Melody").program_change(0, 0, 73);
    // The tune, one note per syllable, monophonic.
    for key in [60u8, 60, 67, 67, 69, 69, 67, 65, 65, 64, 64, 62, 62, 60] {
        music.note(0, 0, key, 100, 240);
    }

    smf(vec![header.finish(), words.finish(), music.finish()])
}

/// A Soft Karaoke file whose words are Japanese, in Shift-JIS.
///
/// **Synthetic, and the bytes are written out rather than encoded**, exactly as the Shift-JIS sample
/// in `encoding.rs` is and for the same reason: what a fixture is worth is that it does not depend
/// on the machinery under test. The corpus holds 189 Shift-JIS files and none of them may be copied
/// here.
///
/// The syllables are the kana of `こんにちは せかい` — "hello, world" — split one kana per lyric
/// event, so a wipe crosses them one character at a time the way a Japanese karaoke file does. That
/// is what makes this worth rendering: the syllable boundaries are measured from the glyphs, and a
/// font without them puts every boundary in the same place.
pub fn soft_karaoke_japanese() -> Vec<u8> {
    let mut header = TrackWriter::new();
    header
        .track_name(0, b"Soft Karaoke")
        .text(0, b"@KMIDI KARAOKE FILE")
        .text(0, b"@V0100")
        // `@LJAPN` is what the format's own vocabulary calls it. The encoding is the stronger signal
        // and says the same thing — see `Song language` in `docs/decisions/songs.md`.
        .text(0, b"@LJAPN")
        // `テスト` — "test" — in Shift-JIS, so the *title* exercises the same path as the words.
        .text(0, &[0x40, 0x54, 0x83, 0x65, 0x83, 0x58, 0x83, 0x67])
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // こ ん に ち は  /  せ か い
    words
        .text(0, &[0x5c, 0x82, 0xb1])
        .text(240, &[0x82, 0xf1])
        .text(240, &[0x82, 0xc9])
        .text(240, &[0x82, 0xbf])
        .text(240, &[0x82, 0xcd])
        .text(480, &[0x2f, 0x82, 0xb9])
        .text(240, &[0x82, 0xa9])
        .text(240, &[0x82, 0xa2]);

    let mut music = TrackWriter::new();
    music.track_name(0, b"Melody").program_change(0, 0, 73);
    for key in [60u8, 62, 64, 65, 67, 67, 65, 64] {
        music.note(0, 0, key, 100, 240);
    }

    smf(vec![header.finish(), words.finish(), music.finish()])
}

/// A Soft Karaoke file laid out the way real ones are, which is not the way the format is described.
///
/// Taken from an actual corpus file: track 1 is named `Soft karaoke` and carries *only* the magic
/// string, while the `@L`/`@T` header lines sit at the top of the `Words` track next to the lyrics,
/// and later tracks are named after instruments. The second `@T` is a transcription credit rather
/// than a performer, which is also typical.
pub fn soft_karaoke_real_layout() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.tempo(0, TEMPO_120);

    let mut announce = TrackWriter::new();
    announce
        .track_name(0, b"Soft karaoke")
        .text(0, b"@KMIDI KARAOKE FILE");

    let mut words = TrackWriter::new();
    words
        .track_name(0, b"Words")
        .text(0, b"@LENGL")
        .text(0, b"@TThe Real Title")
        .text(0, b"@T(Karaoke by Somebody Else)")
        .text(0, b"\\A")
        .text(240, b"mor")
        .text(240, b" da")
        .text(240, b" vi")
        .text(240, b"da");

    let mut bass = TrackWriter::new();
    bass.track_name(0, b"Baixo eletrico      ")
        .program_change(0, 1, 33);
    bass.note(0, 1, 40, 90, 480);

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Bateria             ");
    drums.note(0, 9, 36, 100, 120);

    smf(vec![
        conductor.finish(),
        announce.finish(),
        words.finish(),
        bass.finish(),
        drums.finish(),
    ])
}

/// A standard MIDI karaoke file: lyrics in `Lyric` meta events, no Soft Karaoke header.
pub fn lyric_events() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"Mary Had A Little Lamb")
        .tempo(0, TEMPO_120);

    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Vocal")
        .lyric(0, b"Ma")
        .lyric(240, b"ry ")
        .lyric(240, b"had ")
        .lyric(240, b"a ")
        .lyric(240, b"lit")
        .lyric(240, b"tle ")
        .lyric(240, b"lamb/")
        .lyric(480, b"Its ")
        .lyric(240, b"fleece ")
        .lyric(240, b"was ")
        .lyric(240, b"white ")
        .lyric(240, b"as ")
        .lyric(240, b"snow");

    smf(vec![conductor.finish(), track.finish()])
}

/// Lyrics on a track named `Words`, carried as plain text events with no Soft Karaoke header.
pub fn named_text_track() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"Row Your Boat")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words
        .track_name(0, b"Words")
        .text(0, b"Row ")
        .text(240, b"row ")
        .text(240, b"row ")
        .text(240, b"your ")
        .text(240, b"boat/")
        .text(480, b"Gent")
        .text(240, b"ly ")
        .text(240, b"down ")
        .text(240, b"the ")
        .text(240, b"stream");

    smf(vec![conductor.finish(), words.finish()])
}

/// Lyrics with no line markers at all, so line breaks have to be inferred.
///
/// The two halves are separated by a two-second silence, which is over the gap threshold.
pub fn unmarked_lyrics() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"No Markers")
        .tempo(0, TEMPO_120)
        .lyric(0, b"first ")
        .lyric(240, b"half ")
        .lyric(240, b"here")
        // 1920 ticks at 120 BPM is 2 s.
        .lyric(1_920, b"second ")
        .lyric(240, b"half ")
        .lyric(240, b"here");

    smf(vec![track.finish()])
}

/// Both underscore conventions in one file, on either side of a line break.
///
/// The first line elides a space so two words sit on one timing point; the second cancels one, in a
/// writer's habit of ending every syllable with a space. Reading either mark the other way round
/// spells the line wrong, which is why they are here together.
pub fn underscore_spacing() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Spacing")
        .tempo(0, TEMPO_120)
        .lyric(0, b"Se_a")
        .lyric(240, b"pron")
        .lyric(240, b"ta ")
        .lyric(240, b"pra/")
        .lyric(480, b"THE ")
        .lyric(240, b"MEL_ ")
        .lyric(240, b"O_ ")
        .lyric(240, b"DY ");

    smf(vec![track.finish()])
}

/// A playable file with no lyrics whatsoever, which must still parse.
pub fn instrumental() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track.track_name(0, b"Instrumental").tempo(0, TEMPO_120);
    for key in [60u8, 62, 64, 65, 67] {
        track.note(0, 0, key, 90, 480);
    }
    smf(vec![track.finish()])
}

/// A playable file carrying no text whatsoever: no title, no track name, no lyrics.
///
/// The commonest shape in a real corpus and the one every other fixture here misses — each of the
/// others names its track, and a track name is promoted to a title by [`crate::karaoke`]'s generic
/// metadata fallback, so nothing else in this module ever produces a song with no name at all. Tools
/// that put a title in front of somebody have to decide what to show for this, and could not be
/// tested against it until it existed.
pub fn untitled_instrumental() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track.tempo(0, TEMPO_120);
    for key in [60u8, 62, 64, 65, 67] {
        track.note(0, 0, key, 90, 480);
    }
    smf(vec![track.finish()])
}

/// A file whose tempo doubles halfway through, for tick-to-time conversion.
pub fn tempo_change() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Tempo Change")
        .tempo(0, TEMPO_120)
        .lyric(0, b"slow ")
        .lyric(960, b"then ")
        // 60 BPM from two beats in.
        .tempo(0, 1_000_000)
        .lyric(960, b"fast");
    smf(vec![track.finish()])
}

/// A file that writes a placeholder tempo and supersedes it at the same tick.
///
/// Soft Karaoke files commonly open with the sequencer's default 120 BPM and then state the real
/// tempo, both at tick 0. Keeping the first of the two plays the whole song at the wrong speed.
pub fn superseded_initial_tempo() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Superseded Tempo")
        .tempo(0, TEMPO_120)
        // 60 BPM, at the same tick: this is the one that governs.
        .tempo(0, 1_000_000)
        .lyric(0, b"slow ")
        .lyric(960, b"still ")
        .lyric(960, b"slow");
    smf(vec![track.finish()])
}

/// A file using note-on with velocity 0 in place of note-off, as many sequencers do.
pub fn velocity_zero_note_offs() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track.track_name(0, b"Running Status").tempo(0, TEMPO_120);
    track.note_on(0, 0, 60, 100);
    track.note_on_velocity_zero(480, 0, 60);
    track.note_on(0, 0, 62, 100);
    track.note_on_velocity_zero(480, 0, 62);
    smf(vec![track.finish()])
}

/// A karaoke file with a clearly separated monophonic melody channel plus polyphonic accompaniment.
///
/// Built for melody detection (M2): channel 0 is named, monophonic, in vocal range, and aligns with
/// the lyrics; channel 1 plays chords; channel 9 is drums.
pub fn melody_and_accompaniment() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"Detectable Melody")
        .tempo(0, TEMPO_120);

    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Melody").program_change(0, 0, 73);
    for key in [62u8, 64, 65, 67, 65, 64, 62, 60] {
        melody.note(0, 0, key, 100, 480);
    }

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in [
        "This ", "is ", "the ", "tune/", "you ", "should ", "be ", "singing",
    ]
    .into_iter()
    .enumerate()
    {
        let delta = if i == 0 { 0 } else { 480 };
        words.lyric(delta, syllable.as_bytes());
    }

    let mut chords = TrackWriter::new();
    chords.track_name(0, b"Piano").program_change(0, 1, 0);
    for _ in 0..4 {
        // Three notes at once, so the channel is plainly not a melody line.
        chords.note_on(0, 1, 48, 80);
        chords.note_on(0, 1, 52, 80);
        chords.note_on(0, 1, 55, 80);
        chords.note_off(960, 1, 48);
        chords.note_off(0, 1, 52);
        chords.note_off(0, 1, 55);
    }

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Drums");
    for _ in 0..8 {
        drums.note(0, 9, 36, 100, 120);
        drums.note(240, 9, 38, 90, 120);
    }

    smf(vec![
        conductor.finish(),
        melody.finish(),
        words.finish(),
        chords.finish(),
        drums.finish(),
    ])
}

/// Lyrics encoded in CP1252, which is not valid UTF-8 — for the encoding path.
pub fn legacy_encoded_lyrics() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track.track_name(0, b"Legacy Encoding").tempo(0, TEMPO_120);
    // "cção" and "não" in CP1252.
    track.lyric(0, &[b'c', 0xE7, 0xE3, b'o', b' ']);
    track.lyric(480, &[b'n', 0xE3, b'o']);
    smf(vec![track.finish()])
}

/// A SMPTE-timed file: 25 fps with 40 subframes, so 1000 ticks per second.
pub fn smpte_timed() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"SMPTE Timed")
        .lyric(0, b"one ")
        .lyric(1_000, b"per ")
        .lyric(1_000, b"second");
    smf_smpte(vec![track.finish()], 25, 40)
}

/// Two channels that are equally plausible melodies, so detection must abstain.
///
/// Both are monophonic, both in vocal range, both aligned with every syllable, and neither track is
/// named for the melody. Nothing in the file says which one carries the tune, and the honest answer
/// is to claim neither.
pub fn ambiguous_melody() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.track_name(0, b"Ambiguous").tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in ["which ", "one ", "is ", "it"].into_iter().enumerate() {
        words.lyric(if i == 0 { 0 } else { 480 }, syllable.as_bytes());
    }

    let mut part_a = TrackWriter::new();
    part_a.track_name(0, b"Part A").program_change(0, 0, 73);
    for key in [62u8, 64, 65, 67] {
        part_a.note(0, 0, key, 100, 480);
    }

    let mut part_b = TrackWriter::new();
    part_b.track_name(0, b"Part B").program_change(0, 1, 73);
    for key in [62u8, 64, 65, 67] {
        part_b.note(0, 1, key, 100, 480);
    }

    smf(vec![
        conductor.finish(),
        words.finish(),
        part_a.finish(),
        part_b.finish(),
    ])
}

/// Lyrics over nothing but chords: no channel plays one note at a time.
pub fn chords_only() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.track_name(0, b"Chords Only").tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in ["no ", "line ", "to ", "follow"].into_iter().enumerate() {
        words.lyric(if i == 0 { 0 } else { 480 }, syllable.as_bytes());
    }

    let mut chords = TrackWriter::new();
    chords.track_name(0, b"Piano").program_change(0, 0, 0);
    for _ in 0..4 {
        chords.note_on(0, 0, 60, 80);
        chords.note_on(0, 0, 64, 80);
        chords.note_on(0, 0, 67, 80);
        chords.note_off(480, 0, 60);
        chords.note_off(0, 0, 64);
        chords.note_off(0, 0, 67);
    }

    smf(vec![conductor.finish(), words.finish(), chords.finish()])
}

/// Lyrics over drums only. Drum hits are monophonic and land on the beat, but note numbers on
/// channel 9 select instruments rather than pitches, so it must never be offered as a melody.
pub fn drums_and_lyrics() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.track_name(0, b"Drums Only").tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in ["beat ", "beat ", "beat ", "beat"].into_iter().enumerate() {
        words.lyric(if i == 0 { 0 } else { 480 }, syllable.as_bytes());
    }

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Melody");
    for _ in 0..4 {
        drums.note(0, 9, 36, 100, 480);
    }

    smf(vec![conductor.finish(), words.finish(), drums.finish()])
}

/// A well-formed karaoke song: syllable-timed lyrics, a detectable melody, a full arrangement, a
/// drum channel and a plausible length. The shape a high suitability score should describe.
pub fn high_quality_song() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // 240 syllables at one per beat is two minutes at 120 BPM. A word ends after every second
    // syllable, so the file says where its words end and draws them as written.
    for i in 0..240u32 {
        let text: &[u8] = match (i % 8, i % 2) {
            (0, _) => b"/la",
            (_, 0) => b"la",
            _ => b"la ",
        };
        words.lyric(if i == 0 { 0 } else { 480 }, text);
    }
    song_around(words)
}

/// A file identical to [`high_quality_song`] except that its lyric track holds the arranger's
/// contact details instead of a song.
///
/// This is a real shape from the corpus, and it is the reason lyric *quantity* is scored at all: the
/// file is syllable-timed, perfectly synced, has a detectable melody and a full arrangement, so every
/// other measurement says excellent. It scored 10/10 until the words were counted.
pub fn credits_in_the_lyric_track() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // A minute in, four seconds of somebody's business card.
    words
        .lyric(23_040, b"/A. ")
        .lyric(240, b"Se")
        .lyric(240, b"quen")
        .lyric(240, b"cer ")
        .lyric(480, b"/0**17 ")
        .lyric(240, b"3463-1150 ")
        .lyric(480, b"/midis@example.com.br");
    song_around(words)
}

/// A real song whose lyrics are timed for the first verse only, then stop.
///
/// The words scan and rhyme — this is not a credit block — but they cover the first few seconds of a
/// two-minute file, so there is nothing to follow for the rest of it. Distinct from
/// [`credits_in_the_lyric_track`] because the fault and the right thing to say about it differ.
pub fn lyrics_that_stop_early() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // 40 syllables in the first eight seconds, then silence for the remaining hundred-odd.
    for i in 0..40u32 {
        let text: &[u8] = match (i % 8, i % 2) {
            (0, _) => b"/la",
            (_, 0) => b"la",
            _ => b"la ",
        };
        words.lyric(if i == 0 { 0 } else { 96 }, text);
    }
    song_around(words)
}

/// A song whose every syllable ends with a space, so nothing in it says where a word ends.
///
/// A real shape from the corpus, and the reason [`crate::timeline::SYLLABLE_DIVIDER`] exists: the
/// file claims each of its fragments is a whole word, and the screen would otherwise draw four times
/// as many words as the line has. The boundaries cannot be recovered — the fragments here straddle
/// words exactly as the corpus files' do, and the timing points are evenly spaced, so no gap says
/// where a word ends either.
///
/// The backing is [`high_quality_song`]'s, so a suitability compared against that one is comparing
/// the words and nothing else.
pub fn word_ends_unmarked() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // "cantaremos juntos a guitarra", cut into fragments that cross the words, and no break marker
    // anywhere — the line inference is what has to find the lines.
    const SYLLABLES: [&[u8]; 10] = [
        b"can ", b"ta ", b"re ", b"mos ", b"jun ", b"tos ", b"a ", b"gui ", b"tar ", b"ra ",
    ];
    // 240 syllables at one per beat is two minutes at 120 BPM.
    for i in 0..240usize {
        words.lyric(if i == 0 { 0 } else { 480 }, SYLLABLES[i % SYLLABLES.len()]);
    }
    song_around(words)
}

/// [`word_ends_unmarked`] with every line ended by a `\r` event of its own.
///
/// A real shape from the corpus: the file places each of its lines and still spaces every syllable.
/// A break marker says where a line ends and nothing about words, so this is drawn with the same
/// divider.
pub fn word_ends_unmarked_lines_marked() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    const SYLLABLES: [&[u8]; 10] = [
        b"can ", b"ta ", b"re ", b"mos ", b"jun ", b"tos ", b"a ", b"gui ", b"tar ", b"ra ",
    ];
    for i in 0..240usize {
        words.lyric(if i == 0 { 0 } else { 480 }, SYLLABLES[i % SYLLABLES.len()]);
        if i % SYLLABLES.len() == SYLLABLES.len() - 1 {
            words.lyric(0, b"\r");
        }
    }
    song_around(words)
}

/// [`word_ends_unmarked`] in English, whose fragments average what a file of short whole words does.
///
/// A real shape from the corpus: English is mostly words of one syllable, so a file that spaces
/// every syllable averages over three characters a fragment. None of its fragments is long, and that
/// is what says they are syllables.
pub fn word_ends_unmarked_short_words() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // "walking down the city road at night alone waiting", 3.15 characters a fragment.
    const SYLLABLES: [&[u8]; 13] = [
        b"wal ", b"king ", b"down ", b"the ", b"ci ", b"ty ", b"road ", b"at ", b"night ", b"a ",
        b"lone ", b"wait ", b"ing ",
    ];
    for i in 0..260usize {
        words.lyric(if i == 0 { 0 } else { 480 }, SYLLABLES[i % SYLLABLES.len()]);
    }
    song_around(words)
}

/// A song with no space anywhere in its lyrics, so nothing in it says where a word ends.
///
/// A real shape from the corpus, and the other half of what [`crate::timeline::SYLLABLE_DIVIDER`]
/// exists for: sheet music read by an optical scanner writes one event per note and no separator at
/// all, so a whole verse joins into one run of letters. The fragments straddle words the same way
/// [`word_ends_unmarked`]'s do, and the timing is evenly spaced, so nothing here recovers a
/// boundary either.
///
/// The backing is [`high_quality_song`]'s, so a suitability compared against that one is comparing
/// the words and nothing else.
pub fn word_boundaries_unmarked() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    // "cantaremos juntos a guitarra" again, and this time not one space in it.
    const SYLLABLES: [&[u8]; 10] = [
        b"can", b"ta", b"re", b"mos", b"jun", b"tos", b"a", b"gui", b"tar", b"ra",
    ];
    for i in 0..240usize {
        words.lyric(if i == 0 { 0 } else { 480 }, SYLLABLES[i % SYLLABLES.len()]);
    }
    song_around(words)
}

/// A file whose lyric track holds chord names, one to a line, and no words at all.
///
/// A real shape from the corpus, invented here rather than copied: a keyboard style demo that puts
/// the chord of each bar where the lyrics go, spaced as the fixed-width display it was written for
/// spaces them. Timed to the bar across the whole song, so it has plenty of text covering all of it.
///
/// The backing is [`high_quality_song`]'s, so a suitability compared against that one is comparing
/// the words and nothing else.
pub fn chord_names_as_lyrics() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    const CHORDS: [&[u8]; 8] = [
        b"/A# 7th       ",
        b"/D# 6         ",
        b"/C  min 7th    /G ",
        b"/F  Maj 7th   ",
        b"/G# min6      ",
        b"/D  min        F  7th",
        b"/Bb           ",
        b"/F#m7         ",
    ];
    // One chord to the bar: 60 bars is two minutes at 120 BPM.
    for i in 0..60usize {
        words.lyric(if i == 0 { 0 } else { 1_920 }, CHORDS[i % CHORDS.len()]);
    }
    song_around(words)
}

/// A song written in chords, whose lines are opened with a bracket.
///
/// A real shape from the corpus, invented here rather than copied: a writer that keeps its lyrics in
/// fixed-length fields sends one whole line per event opened with `<`, and spends most of its events
/// on chord symbols in Latin note names — two thirds of them, in the file this was measured from.
/// Left alone, an instrumental draws nothing but `%FA7+%MI-7`.
///
/// The terminators are part of the shape and not decoration: the same fields that carry the notation
/// carry a NUL, which is what took a machine down before it was cleaned.
///
/// The backing is [`high_quality_song`]'s, so a suitability compared against that one is comparing
/// the words and nothing else.
pub fn chords_and_bracketed_lines() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    const CHORDS: [&[u8]; 4] = [b"%SOL\0", b"%LA-\0", b"%FA7+\0", b"%MI-7\0"];
    const LINES: [&[u8]; 4] = [
        b"<cantaremos juntos\0",
        b"<a noite inteira\0",
        b"<com a guitarra\0",
        b"<e ninguem vai dormir\0",
    ];
    // A chord, then the line it carries, forty times over: eighty events, which is well past the
    // count at which a file is judged to have a habit.
    for i in 0..40usize {
        words.lyric(if i == 0 { 0 } else { 240 }, CHORDS[i % CHORDS.len()]);
        words.lyric(240, LINES[i % LINES.len()]);
    }
    song_around(words)
}

/// A harmonica play-along: a tab stacked over each syllable, and a solo of bare tabs between verses.
///
/// Each verse event is two rows, the hole to play and the syllable under it, and the solo is the
/// same holes with nothing under them. Left alone the machine draws `6Sing-6it5out` and then a
/// line of numbers.
///
/// The backing is [`high_quality_song`]'s, so a suitability compared against that one is comparing
/// the words and nothing else.
pub fn harmonica_tablature() -> Vec<u8> {
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    const VERSE: [&[u8]; 8] = [
        b"6\nSing",
        b"-6\nit",
        b"5\nout",
        b"4\nloud,",
        b"-4\nsing",
        b"4\nit",
        b"5b\nlow",
        b"--7\nnow",
    ];
    const SOLO: [&[u8]; 8] = [b"6", b"-6", b"5", b"4", b"-4", b"4", b"5", b"6"];
    // Verse, solo, verse, solo, verse: forty events, the solos opening on a phrase gap.
    for part in 0..5usize {
        let events = if part % 2 == 0 { &VERSE } else { &SOLO };
        for (i, event) in events.iter().enumerate() {
            words.lyric(if i == 0 { 1_920 } else { 240 }, event);
        }
    }
    song_around(words)
}

/// The backing of [`high_quality_song`] wrapped around whatever lyric track is given.
///
/// One arrangement shared by four fixtures, so a test comparing them is comparing *the words* and
/// nothing else — which is exactly the claim the lyric-quantity scoring makes.
fn song_around(words: TrackWriter) -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.track_name(0, b"A Good One").tempo(0, TEMPO_120);

    let mut tracks = vec![conductor.finish(), words.finish()];
    tracks.extend(backing());
    smf(tracks)
}

/// The instruments of [`high_quality_song`], without a conductor and without words.
///
/// Two minutes of melody on channel 0, one note to the beat, with a full arrangement behind it and
/// drums on channel 9.
fn backing() -> Vec<Vec<u8>> {
    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Melody").program_change(0, 0, 73);
    for i in 0..240u32 {
        let key = 60 + u8::try_from(i % 8).unwrap_or(0);
        melody.note(0, 0, key, 100, 480);
    }

    let mut piano = TrackWriter::new();
    piano.track_name(0, b"Piano").program_change(0, 1, 0);
    for _ in 0..60 {
        piano.note_on(0, 1, 48, 70);
        piano.note_on(0, 1, 52, 70);
        piano.note_off(1_920, 1, 48);
        piano.note_off(0, 1, 52);
    }

    let mut bass = TrackWriter::new();
    bass.track_name(0, b"Bass").program_change(0, 2, 33);
    for i in 0..120u32 {
        let key = 36 + u8::try_from(i % 5).unwrap_or(0);
        bass.note(0, 2, key, 90, 960);
    }

    let mut strings = TrackWriter::new();
    strings.track_name(0, b"Strings").program_change(0, 3, 48);
    for _ in 0..60 {
        strings.note(0, 3, 55, 60, 1_920);
    }

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Drums");
    for _ in 0..120 {
        drums.note(0, 9, 36, 100, 120);
        drums.note(360, 9, 38, 90, 120);
    }

    vec![
        melody.finish(),
        piano.finish(),
        bass.finish(),
        strings.finish(),
        drums.finish(),
    ]
}

/// Lyrics timed against some arrangement other than the one they are in.
///
/// Nothing about the quantity or the granularity is wrong: 240 timing points, one per syllable,
/// spanning the song. They simply do not fall where the singing does — every one of them sits half
/// a beat off, squarely between two melody notes, which is what lyrics timed to another cut of the
/// same song look like.
///
/// The strummed chords are what make the file worth a fixture. They put a note onset every
/// sixteenth, which is close enough that *any* timing at all falls within the sync window of one of
/// them. Asked about the arrangement the words are perfectly synced; asked about the line being
/// sung, not one of them lands. The second question is the one a singer is asking.
pub fn lyrics_against_another_arrangement() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.track_name(0, b"Another One").tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for i in 0..240usize {
        let text: &[u8] = match (i % 8, i % 2) {
            (0, _) => b"/la",
            (_, 0) => b"la",
            _ => b"la ",
        };
        words.lyric(if i == 0 { 240 } else { 480 }, text);
    }

    // Chords rather than single notes, so this is never mistaken for the melody itself.
    let mut comp = TrackWriter::new();
    comp.track_name(0, b"Comp").program_change(0, 4, 25);
    for _ in 0..960 {
        comp.note_on(0, 4, 55, 60);
        comp.note_on(0, 4, 59, 60);
        comp.note_on(0, 4, 62, 60);
        comp.note_off(120, 4, 55);
        comp.note_off(0, 4, 59);
        comp.note_off(0, 4, 62);
    }

    let mut tracks = vec![conductor.finish(), words.finish(), comp.finish()];
    tracks.extend(backing());
    smf(tracks)
}

/// A track named `Melody` that plays only where nobody is singing.
///
/// Two verses of words, one syllable to the beat, and a monophonic line in singing range named for
/// the melody. The line is a riff: it plays the intro, the break between the verses and the outro,
/// and rests under every sung word. Its name is the only thing tying it to the singing, and the name
/// is wrong.
///
/// Measured against that line, the words land on nothing, and a detector that trusts the name marks
/// a well-timed song as badly synced. The chords under the verses strike every beat, so against the
/// arrangement the words are timed exactly.
pub fn named_melody_silent_under_the_words() -> Vec<u8> {
    const BEAT: u32 = 480;

    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"Riff Named Melody")
        .tempo(0, TEMPO_120);

    // Verses on beats 16 to 119 and 136 to 239.
    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    let mut last = 0;
    for beat in (16..120u32).chain(136..240) {
        let text: &[u8] = match (beat % 8, beat % 2) {
            (0, _) => b"/la",
            (_, 0) => b"la",
            _ => b"la ",
        };
        words.lyric((beat - last) * BEAT, text);
        last = beat;
    }

    // Intro, break and outro: beats 0 to 15, 120 to 135 and 240 to 255.
    let mut riff = TrackWriter::new();
    riff.track_name(0, b"Melody").program_change(0, 0, 81);
    let mut rest = 0;
    for beat in (0..16u32).chain(120..136).chain(240..256) {
        let key = 64 + u8::try_from(beat % 4).unwrap_or(0);
        riff.note((beat * BEAT).saturating_sub(rest), 0, key, 100, BEAT);
        rest = (beat + 1) * BEAT;
    }

    // Chords rather than single notes, so nothing here stands in for the tune.
    let mut comp = TrackWriter::new();
    comp.track_name(0, b"Comp").program_change(0, 1, 4);
    for _ in 0..256 {
        comp.note_on(0, 1, 55, 70);
        comp.note_on(0, 1, 59, 70);
        comp.note_on(0, 1, 62, 70);
        comp.note_off(BEAT, 1, 55);
        comp.note_off(0, 1, 59);
        comp.note_off(0, 1, 62);
    }

    let mut pad = TrackWriter::new();
    pad.track_name(0, b"Pad").program_change(0, 3, 89);
    for _ in 0..64 {
        pad.note_on(0, 3, 48, 60);
        pad.note_on(0, 3, 52, 60);
        pad.note_off(4 * BEAT, 3, 48);
        pad.note_off(0, 3, 52);
    }

    let mut bass = TrackWriter::new();
    bass.track_name(0, b"Bass").program_change(0, 2, 33);
    for i in 0..128u32 {
        let key = 36 + u8::try_from(i % 5).unwrap_or(0);
        bass.note(0, 2, key, 90, 2 * BEAT);
    }

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Drums");
    for _ in 0..256 {
        drums.note(0, 9, 36, 100, BEAT / 2);
        drums.note(0, 9, 42, 80, BEAT / 2);
    }

    smf(vec![
        conductor.finish(),
        words.finish(),
        riff.finish(),
        comp.finish(),
        pad.finish(),
        bass.finish(),
        drums.finish(),
    ])
}

/// A Soft Karaoke file carrying two timings of the same song, the wrong one longer.
///
/// `Words` is the timing the file means: 240 syllables, one to the beat, each on a melody note. The
/// unnamed track is a working copy left behind — 300 finer timing points, half a beat apart and
/// three ticks short of it, so it opens on the beat, slides away within a few syllables and runs out
/// three fifths of the way through the song.
///
/// The stale track is the longer of the two, which is what makes this shape worth a fixture: a
/// reader counting events picks it, and every word after the first line is drawn away from the note
/// being sung.
pub fn two_lyric_tracks_disagreeing() -> Vec<u8> {
    let mut header = TrackWriter::new();
    header
        .track_name(0, b"Soft Karaoke")
        .text(0, b"@KMIDI KARAOKE FILE")
        .text(0, b"@V0100")
        .text(0, b"@LENGL")
        .text(0, b"@TTwo Timings")
        .text(0, b"@TThe Test Fixtures")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for i in 0..240usize {
        let text: &[u8] = match (i % 8, i % 2) {
            (0, _) => b"\\la",
            (_, 0) => b"la",
            _ => b"la ",
        };
        words.text(if i == 0 { 0 } else { 480 }, text);
    }

    // Half a beat apart and losing three ticks each time, written as deltas so the drift is the
    // spacing rather than a correction applied on top of it.
    let mut stale = TrackWriter::new();
    for i in 0..300usize {
        let text: &[u8] = match (i % 8, i % 2) {
            (0, _) => b"\\la",
            (_, 0) => b"la",
            _ => b"la ",
        };
        stale.text(if i == 0 { 0 } else { 237 }, text);
    }

    let mut tracks = vec![header.finish(), words.finish(), stale.finish()];
    tracks.extend(backing());
    smf(tracks)
}

/// Lyrics whose timing points all sit at tick 0: present, but useless for highlighting.
///
/// 16 files in the local corpus look like this. It is a hard defect, not a low suitability.
pub fn lyrics_all_at_zero() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track.track_name(0, b"Dumped Lyrics").tempo(0, TEMPO_120);
    for word in ["all ", "at ", "once ", "no ", "timing"] {
        track.lyric(0, word.as_bytes());
    }
    for key in [60u8, 62, 64, 65] {
        track.note(0, 0, key, 90, 480);
    }
    smf(vec![track.finish()])
}

/// A melody and a bass line that are *both* monophonic and both land on the syllables.
///
/// Distilled from a real corpus file (a Brazilian `.kar` whose guide track is named `Tema`). Both
/// channels are monophonic, and the bass hits most syllables because it follows the rhythm, so
/// lyric alignment alone cannot separate them -- detection called it ambiguous until singable range
/// became a gate. The melody sits in vocal register; the bass is two octaves below it.
pub fn melody_with_monophonic_bass() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"Melody Versus Bass")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in ["A", "mor", " da", " vi", "da", " mi", "nha", " sim"]
        .into_iter()
        .enumerate()
    {
        words.lyric(if i == 0 { 0 } else { 480 }, syllable.as_bytes());
    }

    // Named only in Portuguese, so the name signal is exercised in a non-English form.
    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Tema").program_change(0, 4, 56);
    for key in [71u8, 69, 67, 69, 71, 72, 71, 69] {
        melody.note(0, 4, key, 100, 480);
    }

    // Same rhythm, same alignment, but nobody sings these notes.
    let mut bass = TrackWriter::new();
    bass.track_name(0, b"Baixo eletrico")
        .program_change(0, 1, 34);
    for key in [35u8, 35, 40, 40, 38, 38, 35, 35] {
        bass.note(0, 1, key, 90, 480);
    }

    smf(vec![
        conductor.finish(),
        words.finish(),
        melody.finish(),
        bass.finish(),
    ])
}

/// A whole song in the layout real Soft Karaoke files actually use, long enough to browse.
///
/// [`soft_karaoke_real_layout`] establishes the *shape* in four syllables; this is that shape
/// carrying a song. Thirty-two lines of eight syllables, a guide melody named `Tema` on **channel
/// 5**, a monophonic bass two octaves under it on channel 1 hitting every one of the same
/// syllables, and drums on 9.
///
/// The bass is what makes it worth having. It is as monophonic as the melody and aligns with the
/// words just as well, so nothing but singable register separates them -- which is the case that
/// forced vocal range to be a gate rather than a bonus. Getting that wrong mutes the tune and leaves
/// the bass playing, which is the worst outcome the melody detector has.
pub fn soft_karaoke_header_on_words_track() -> Vec<u8> {
    // Four lines, cycled eight times. Deliberately about nothing: a fixture that quotes a real song
    // is a fixture that cannot be shipped.
    const VERSE: [[&str; 8]; 4] = [
        [
            "This ", "song ", "was ", "made ", "up ", "for ", "a ", "test",
        ],
        [
            "No ", "sing", "er ", "ev", "er ", "sang ", "these ", "words",
        ],
        ["The ", "notes ", "be", "low ", "are ", "not ", "a ", "tune"],
        [
            "They ", "on", "ly ", "prove ", "the ", "read", "er ", "works",
        ],
    ];
    const LINES: usize = 32;

    let mut conductor = TrackWriter::new();
    conductor.tempo(0, TEMPO_120);

    // The magic alone on its own track, which is where real files put it -- and nowhere near the
    // header lines the format says accompany it.
    let mut announce = TrackWriter::new();
    announce
        .track_name(0, b"Soft karaoke")
        .text(0, b"@KMIDI KARAOKE FILE");

    let mut words = TrackWriter::new();
    words
        .track_name(0, b"Words")
        .text(0, b"@LENGL")
        .text(0, b"@TThe Long One")
        .text(0, b"@T(Karaoke by Somebody)");
    for line in 0..LINES {
        for (i, syllable) in VERSE[line % VERSE.len()].iter().enumerate() {
            let delta = if line == 0 && i == 0 { 0 } else { 480 };
            let marked = match (line, i) {
                (0, 0) => format!("\\{syllable}"),
                (_, 0) => format!("/{syllable}"),
                _ => (*syllable).to_owned(),
            };
            words.text(delta, marked.as_bytes());
        }
    }

    // Named in Portuguese, as the file this is distilled from was: the name signal cannot carry
    // this one, so alignment, monophony and register have to.
    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Tema").program_change(0, 5, 56);
    for i in 0..LINES * 8 {
        let key = [67u8, 69, 71, 72, 71, 69, 67, 69][i % 8];
        melody.note(0, 5, key, 100, 480);
    }

    // Same rhythm, same alignment, two octaves down. Nobody sings these.
    let mut bass = TrackWriter::new();
    bass.track_name(0, b"Baixo eletrico")
        .program_change(0, 1, 34);
    for i in 0..LINES * 8 {
        let key = [35u8, 35, 40, 40, 38, 38, 35, 35][i % 8];
        bass.note(0, 1, key, 90, 480);
    }

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Bateria");
    for _ in 0..LINES * 4 {
        drums.note(0, 9, 36, 100, 120);
        drums.note(360, 9, 38, 90, 120);
    }

    smf(vec![
        conductor.finish(),
        announce.finish(),
        words.finish(),
        melody.finish(),
        bass.finish(),
        drums.finish(),
    ])
}

/// A Soft Karaoke file whose **first** `@T` line is the producer rather than the song.
///
/// The header convention is positional -- first `@T` the title, second the performer -- so a studio
/// in front of them puts a studio in the title and shifts everything else along by one. Hundreds of
/// files in a real corpus open this way, which is why credits are partitioned out *before* the two
/// names are assigned rather than filtered afterwards.
pub fn soft_karaoke_producer_credit_first() -> Vec<u8> {
    let mut header = TrackWriter::new();
    header
        .track_name(0, b"Soft Karaoke")
        .text(0, b"@KMIDI KARAOKE FILE")
        .text(0, b"@V0100")
        .text(0, b"@LENGL")
        .text(0, b"@TKaraoke Fixture Studios - 2001")
        .text(0, b"@TA Made Up Song")
        .text(0, b"@TA Made Up Singer")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in [
        "\\No", "bo", "dy ", "wrote ", "/this ", "one ", "at ", "all",
    ]
    .into_iter()
    .enumerate()
    {
        words.text(if i == 0 { 0 } else { 240 }, syllable.as_bytes());
    }

    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Melody").program_change(0, 0, 73);
    for key in [62u8, 64, 65, 67, 67, 65, 64, 62] {
        melody.note(0, 0, key, 100, 240);
    }

    smf(vec![header.finish(), words.finish(), melody.finish()])
}

/// A file whose `@T` lines are separator rows rather than names.
///
/// The shape a corpus arrives in when whoever typed the file had no title to type: a row of `=`
/// where the name goes and a row of `<>-` under it. Both are refused, so the song reaches curation
/// with no title and no artist and browses under its file name.
pub fn soft_karaoke_titles_of_marks() -> Vec<u8> {
    let mut header = TrackWriter::new();
    header
        .track_name(0, b"Soft Karaoke")
        .text(0, b"@KMIDI KARAOKE FILE")
        .text(0, b"@V0100")
        .text(0, b"@LENGL")
        .text(0, b"@T====================")
        .text(0, b"@T<>-<>-<>-<>")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in [
        "\\No", "bo", "dy ", "named ", "/this ", "one ", "at ", "all",
    ]
    .into_iter()
    .enumerate()
    {
        words.text(if i == 0 { 0 } else { 240 }, syllable.as_bytes());
    }

    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Melody").program_change(0, 0, 73);
    for key in [62u8, 64, 65, 67, 67, 65, 64, 62] {
        melody.note(0, 0, key, 100, 240);
    }

    smf(vec![header.finish(), words.finish(), melody.finish()])
}

/// A melody on **channel 15**, against chords on 2 and drums on 9.
///
/// [`melody_and_accompaniment`] re-voiced and nothing else. It exists because a channel is a
/// four-bit field and code that indexes an array of ten, or that reads a channel as a decimal digit,
/// is wrong in a way that only the top of the range shows.
pub fn melody_on_channel_fifteen() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor.track_name(0, b"High Channel").tempo(0, TEMPO_120);

    let mut melody = TrackWriter::new();
    melody.track_name(0, b"Melody").program_change(0, 15, 73);
    for key in [62u8, 64, 65, 67, 65, 64, 62, 60] {
        melody.note(0, 15, key, 100, 480);
    }

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in ["The ", "tune ", "is ", "up/", "at ", "the ", "top ", "here"]
        .into_iter()
        .enumerate()
    {
        words.lyric(if i == 0 { 0 } else { 480 }, syllable.as_bytes());
    }

    let mut chords = TrackWriter::new();
    chords.track_name(0, b"Piano").program_change(0, 2, 0);
    for _ in 0..4 {
        chords.note_on(0, 2, 48, 80);
        chords.note_on(0, 2, 52, 80);
        chords.note_on(0, 2, 55, 80);
        chords.note_off(960, 2, 48);
        chords.note_off(0, 2, 52);
        chords.note_off(0, 2, 55);
    }

    let mut drums = TrackWriter::new();
    drums.track_name(0, b"Drums");
    for _ in 0..8 {
        drums.note(0, 9, 36, 100, 120);
        drums.note(240, 9, 38, 90, 120);
    }

    smf(vec![
        conductor.finish(),
        melody.finish(),
        words.finish(),
        chords.finish(),
        drums.finish(),
    ])
}

/// A file whose transcription credit is inside the *track name* the title comes from.
///
/// The credit filter partitions `@T` lines and deliberately does not reach here: a file with no Soft
/// Karaoke header has only its track names to offer, and an ugly-but-complete title beats no title
/// at all. Curation is where somebody tidies it.
pub fn named_text_track_credit_in_the_name() -> Vec<u8> {
    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"A Made Up Song - Kar by Somebody")
        .tempo(0, TEMPO_120);

    let mut words = TrackWriter::new();
    words.track_name(0, b"Words");
    for (i, syllable) in [
        "The ", "name ", "up ", "there/", "is ", "not ", "tid", "ied",
    ]
    .into_iter()
    .enumerate()
    {
        words.text(if i == 0 { 0 } else { 240 }, syllable.as_bytes());
    }

    smf(vec![conductor.finish(), words.finish()])
}

// --- files that must be refused -----------------------------------------------------------------
//
// A parser is judged as much by what it declines as by what it reads, and a real corpus is full of
// files that are not what their extension says. These are the shapes it has to survive: never a
// panic, always an error somebody can act on.

/// Bytes that are not a MIDI file at all, under a `.kar` name.
///
/// A fixed-seed linear congruential generator rather than anything captured, so the same bytes come
/// out on every machine and [`crate::Song::parse`] is deterministic over them. The constants are
/// Numerical Recipes'; nothing here depends on the quality of the randomness, only on its
/// repeatability and on the result not accidentally beginning `MThd`, which the first byte being
/// forced away from `M` guarantees.
pub fn not_midi_random_bytes() -> Vec<u8> {
    let mut state: u32 = 0x1234_5678;
    let mut out = Vec::with_capacity(2_048);
    while out.len() < 2_048 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((state >> 16) as u8);
    }
    out[0] = b'?';
    out
}

/// A text file under a `.mid` name.
///
/// The corpus has thousands of these: web-authoring metadata, playlists and readme files that were
/// renamed or downloaded wrong. The shape is what matters -- printable ASCII with no chunk header
/// anywhere in it -- not what any particular one said.
pub fn not_midi_text_file() -> Vec<u8> {
    b"vti_encoding:SR|utf8-nl\r\nvti_timelastmodified:TR|01 Jan 2001 00:00:00 -0000\r\n".to_vec()
}

/// A file damaged by an FTP transfer in ASCII mode: every `0d` became `0d 0a`.
///
/// **Thirteen tracks, which is what makes it fatal.** The damage is applied to a well-formed file
/// byte for byte, and the byte that matters is in the header: thirteen is `0x0d`, so `MThd`'s own
/// `ntracks` field grows a `0x0a` and the six bytes the header declares become seven. Everything
/// after it is one byte out, so where `MTrk` should be there is a stray newline -- the header reads,
/// and no track chunk is found. Twenty-two files in a real corpus are in exactly this state, and
/// they are all files with a `0x0d` somewhere structural.
///
/// Refusing it is the honest outcome; which error says so is midly's business, so nothing pins one.
pub fn crlf_corrupted_no_tracks() -> Vec<u8> {
    let mut tracks = Vec::new();
    let mut conductor = TrackWriter::new();
    conductor
        .track_name(0, b"Damaged")
        .tempo(0, TEMPO_120)
        // Two more 0x0d, one inside a payload and one as a delta, because a real file has them
        // scattered through the music as well as in the header.
        .text(0, b"a line ending the way DOS ends one\r")
        .note(13, 0, 60, 100, 240);
    tracks.push(conductor.finish());
    for channel in 1..13u8 {
        let mut part = TrackWriter::new();
        part.track_name(0, b"Part");
        part.note(0, channel & 0x0F, 60, 90, 240);
        tracks.push(part.finish());
    }
    let intact = smf(tracks);
    debug_assert_eq!(
        intact[11], 13,
        "the track count is the byte that has to be 0x0d"
    );

    // `MThd` and its length survive -- neither holds a 0x0d -- so the file still looks like a MIDI
    // file for exactly eight bytes.
    let (header, body) = intact.split_at(8);
    let mut out = header.to_vec();
    for byte in body {
        out.push(*byte);
        if *byte == 0x0D {
            out.push(0x0A);
        }
    }
    out
}

/// A header that declares no tracks, and then has none.
///
/// The `NoTracks` path, reached deliberately rather than by damage. Everything about the file is
/// well-formed; there is simply no music in it, and a song with no track is not a song.
pub fn header_declares_no_tracks() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // format 0
    out.extend_from_slice(&0u16.to_be_bytes()); // and no tracks at all
    out.extend_from_slice(&TPQN.to_be_bytes());
    out
}

/// A song that leans on channel pressure, which is not the rarity it was taken for.
///
/// Aftertouch was dropped on the floor for years on the grounds that it is "rare in karaoke files
/// and the synthesizer ignores it". The second half stopped being true when the synthesizer gained
/// modulators, and the first half turns out to be wrong for **channel** pressure: 1,915 of 25,000
/// corpus files carry it, 7.7%, and where it appears there are hundreds of events. Poly pressure
/// really is rare — 0.67% — and is still dropped, so this fixture carries both and the sequencer
/// tests assert that exactly one of them arrives.
pub fn channel_pressure() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Pressure")
        .tempo(0, TEMPO_120)
        .program_change(0, 0, 73)
        .note_on(0, 0, 60, 100)
        .channel_aftertouch(10, 0, 64)
        .poly_aftertouch(10, 0, 60, 90)
        .channel_aftertouch(10, 0, 127)
        .note_off(10, 0, 60)
        // A second note after the pressure, so a seek landing here has state to restore.
        .note(TPQN as u32, 0, 62, 100, TPQN as u32)
        .lyric(0, b"pressed");
    smf(vec![track.finish()])
}

/// A part whose melody is carried in pitch bends against a twelve-semitone bend range.
///
/// **The shape a General MIDI default cannot play.** The channel sets RPN 0 to 12 and then writes
/// its tune as bends around held notes, so a bend of −2730 means four semitones down. Read at the
/// default range of 2 the same bend is two thirds of one semitone, and every note of the part is
/// out of tune by a different fraction — audible immediately, and not fixed by changing key.
///
/// A second channel bends without ever setting a range, so a test can tell a part that asks for
/// something from a part that takes the default.
pub fn pitch_bend_range() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Bent")
        .tempo(0, TEMPO_120)
        .program_change(0, 8, 75)
        .registered_parameter(0, 8, (0, 0), 12)
        .program_change(0, 2, 0)
        .note_on(TPQN as u32, 8, 76, 99)
        // Four semitones down at a range of twelve, two thirds of one at the default.
        .pitch_bend(TPQN as u32 / 2, 8, 5461)
        .note_off(TPQN as u32, 8, 76)
        .note_on(0, 2, 60, 90)
        .pitch_bend(TPQN as u32 / 2, 2, 9557)
        .note_off(TPQN as u32, 2, 60)
        // A note after everything, so a seek landing here has state to restore.
        .note(TPQN as u32, 8, 74, 99, TPQN as u32)
        .lyric(0, b"bent");
    smf(vec![track.finish()])
}

/// A drum kit with two keys retuned by Roland GS non-registered parameter 18H.
///
/// The parameter's LSB is a key number rather than half a parameter id, so one channel carries one
/// of these per key and a channel-wide tune cannot express what the file asks for.
pub fn drum_key_tune() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Kit")
        .tempo(0, TEMPO_120)
        // NRPN 18H, key 36, nine semitones down; then key 38, three up.
        .controller(0, 9, 99, 0x18)
        .controller(0, 9, 98, 36)
        .controller(0, 9, 6, 64 - 9)
        .controller(0, 9, 99, 0x18)
        .controller(0, 9, 98, 38)
        .controller(0, 9, 6, 64 + 3)
        .controller(0, 9, 99, 127)
        .controller(0, 9, 98, 127)
        .note(TPQN as u32, 9, 36, 100, TPQN as u32)
        .note(0, 9, 38, 100, TPQN as u32)
        .note(TPQN as u32, 9, 36, 100, TPQN as u32)
        .lyric(0, b"kit");
    smf(vec![track.finish()])
}

/// A track with no end-of-track meta event, running right up to its own chunk boundary.
///
/// **The shape came from somebody else's bug.** The synthesizer's own MIDI reader was fixed for
/// exactly this — "a track without an end-of-track meta event was parsed past the end of its own
/// chunk" — and although this project never uses that reader, `km-song` had no fixture for the shape
/// at all: `TrackWriter::finish` always appends the mandatory `0x2F`, so every fixture was
/// well-formed in precisely the way real files are not. The `MTrk` length is honest here; only the
/// terminator is missing, which is the case a parser can get wrong silently by reading on into the
/// next chunk.
pub fn track_without_end_of_track() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"No EOT")
        .tempo(0, TEMPO_120)
        .note(0, 0, 60, 100, TPQN as u32)
        .lyric(0, b"unterminated");
    // Deliberately not `finish`, which is the only thing that writes the terminator.
    let data = std::mem::take(&mut track.data);
    let mut chunk = Vec::with_capacity(data.len() + 8);
    chunk.extend_from_slice(b"MTrk");
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(&data);
    smf(vec![chunk])
}

/// A track midly stops reading part way through, silently.
///
/// **This is the shape that manufactures a stuck note.** A system-realtime byte is illegal in a
/// standard MIDI file — midly refuses `0xF8`–`0xFE` outright — and because the crate is deliberately
/// not in `strict` mode, refusing one event means abandoning the whole rest of the track with no
/// error returned. The note-off after the `0xFE` here is therefore unreachable: without a repair the
/// note sounds until the process ends.
///
/// Active sensing is not an invented threat. Older Windows sequencers leak it, and clock (`0xF8`),
/// into files they export, because both are ordinary bytes on a MIDI cable and the export path
/// forgets that a file is not a cable.
pub fn truncated_by_realtime_byte() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Truncated")
        .tempo(0, TEMPO_120)
        // One clean note first, so a reader can tell "stopped early" from "read nothing".
        .note(0, 0, 60, 100, TPQN as u32)
        // Down when the data runs out, at the same tick the clean note ended.
        .note_on(0, 0, 67, 100);
    // A delta, then the illegal byte. The event fails, so this delta is never applied and everything
    // after it is lost.
    track.push_varlen(TPQN as u32);
    track
        .raw(&[0xFE])
        .note_off(0, 0, 67)
        .lyric(0, b"never read");
    smf(vec![track.finish()])
}

/// A setup track that selects a bank of drum kits on channels meant to carry instruments.
///
/// **Four channels, each a different answer.** Channel 4 takes Bank Select MSB 127 — XG's drum
/// bank — and then a program change and sustained chords, which is the defect: on a SoundFont that
/// has a bank 127 the lookup succeeds and those chords come out as percussion. Channel 2 takes an
/// ordinary variation bank and must be left alone, because a file asking for bank 8 is asking for a
/// different piano and a font without one already falls back correctly. Channel 9 takes the same
/// 127 and is also left alone, being the one channel where a kit is what the file means. Channel 5
/// takes it and sounds no note, so there is nothing to correct.
///
/// The track is named for the convention real files write this in, and the selects sit in a block
/// at the head of the file ahead of every program change, which is where a module expects them.
pub fn kit_bank_on_a_melodic_channel() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"GM+GS")
        .tempo(0, TEMPO_120)
        // The defect: a drum bank on a channel the file then plays chords on.
        .controller(0, 4, 0, 127)
        .program_change(0, 4, 17)
        // An ordinary variation bank, which is not a defect.
        .controller(0, 2, 0, 8)
        .program_change(0, 2, 0)
        // The drum channel, where a kit bank is the file's plain meaning.
        .controller(0, 9, 0, 127)
        // A channel that selects a kit bank and never sounds.
        .controller(0, 5, 0, 127)
        .program_change(0, 5, 48)
        // Chords on the flagged channel, held long enough to ring.
        .note_on(TPQN as u32, 4, 60, 100)
        .note_on(0, 4, 64, 100)
        .note_on(0, 4, 67, 100)
        .note_off(TPQN as u32 * 2, 4, 60)
        .note_off(0, 4, 64)
        .note_off(0, 4, 67)
        .note(0, 2, 48, 90, TPQN as u32)
        .note(0, 9, 36, 100, TPQN as u32)
        .lyric(0, b"kit");
    smf(vec![track.finish()])
}

/// A bend gesture whose return stops one step short of centre, and the notes played on it after.
///
/// Channel 4 bends a note down two semitones and ramps back up, and the ramp ends at 6784 — 34
/// cents flat at the default range — with nothing afterwards to finish it. The two notes it plays a
/// beat later sound on that leftover bend. Channel 2 makes the same gesture and finishes it at
/// centre. Channel 6 sets one bend before its first note and never moves it, which is a channel
/// detuned on purpose rather than a gesture left behind.
pub fn bend_left_off_centre() -> Vec<u8> {
    let step = TPQN as u32 / 4;
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Bend")
        .tempo(0, TEMPO_120)
        .program_change(0, 4, 29)
        .registered_parameter(0, 4, (0, 0), 2)
        .program_change(0, 2, 29)
        .program_change(0, 6, 25)
        .pitch_bend(0, 6, 7000)
        // The gesture, on both channels.
        .note_on(TPQN as u32, 4, 52, 100)
        .note_on(0, 2, 52, 100)
        .note_on(0, 6, 60, 90)
        .pitch_bend(step, 4, 4096)
        .pitch_bend(0, 2, 4096)
        .pitch_bend(step, 4, 0)
        .pitch_bend(0, 2, 0)
        .pitch_bend(step, 4, 4096)
        .pitch_bend(0, 2, 4096)
        // The defect: channel 4's last step is missing.
        .pitch_bend(step, 4, 6784)
        .pitch_bend(0, 2, 8192)
        .note_off(0, 4, 52)
        .note_off(0, 2, 52)
        .note_off(0, 6, 60)
        // Two note starts a beat and more after the last bend.
        .note_on(TPQN as u32, 4, 55, 100)
        .note_on(0, 2, 55, 100)
        .note_on(0, 6, 62, 90)
        .note_off(TPQN as u32, 4, 55)
        .note_off(0, 2, 55)
        .note_off(0, 6, 62)
        .note_on(0, 4, 57, 100)
        .note_on(0, 2, 57, 100)
        .note_on(0, 6, 64, 90)
        .note_off(TPQN as u32, 4, 57)
        .note_off(0, 2, 57)
        .note_off(0, 6, 64)
        .lyric(0, b"bend");
    smf(vec![track.finish()])
}

/// A well-formed file whose note-on simply never gets a note-off.
///
/// Nothing is malformed here — every byte parses, the `MTrk` length is honest and the terminator is
/// present. The file just ends holding a note, which 0.5% of the real corpus does, and which no
/// parser can detect as an error because it is not one.
pub fn unbalanced_note_on() -> Vec<u8> {
    let mut track = TrackWriter::new();
    track
        .track_name(0, b"Unbalanced")
        .tempo(0, TEMPO_120)
        .note(0, 0, 60, 100, TPQN as u32)
        // Down from here to the end of the file, and never lifted.
        .note_on(0, 0, 67, 100)
        // A quarter later, so the track outlasts the dangling note-on and the repair has somewhere
        // honest to put its note-off.
        .lyric(TPQN as u32, b"held");
    smf(vec![track.finish()])
}

/// A fixture: a filename and the function that produces its bytes.
pub type Fixture = (&'static str, fn() -> Vec<u8>);

/// Every fixture, with the filename each should be written under.
///
/// The extension matters: `.kar` files are Soft Karaoke by convention, and using the right one
/// keeps manual testing honest.
pub const FIXTURES: &[Fixture] = &[
    ("soft_karaoke.kar", soft_karaoke),
    ("soft_karaoke_real_layout.kar", soft_karaoke_real_layout),
    (
        "soft_karaoke_header_on_words_track.kar",
        soft_karaoke_header_on_words_track,
    ),
    (
        "soft_karaoke_producer_credit_first.kar",
        soft_karaoke_producer_credit_first,
    ),
    (
        "soft_karaoke_titles_of_marks.kar",
        soft_karaoke_titles_of_marks,
    ),
    (
        "two_lyric_tracks_disagreeing.kar",
        two_lyric_tracks_disagreeing,
    ),
    (
        "lyrics_against_another_arrangement.mid",
        lyrics_against_another_arrangement,
    ),
    (
        "named_melody_silent_under_the_words.mid",
        named_melody_silent_under_the_words,
    ),
    ("lyric_events.mid", lyric_events),
    ("named_text_track.mid", named_text_track),
    (
        "named_text_track_credit_in_the_name.mid",
        named_text_track_credit_in_the_name,
    ),
    ("unmarked_lyrics.mid", unmarked_lyrics),
    ("underscore_spacing.mid", underscore_spacing),
    ("instrumental.mid", instrumental),
    ("untitled_instrumental.mid", untitled_instrumental),
    ("tempo_change.mid", tempo_change),
    ("superseded_initial_tempo.mid", superseded_initial_tempo),
    ("velocity_zero_note_offs.mid", velocity_zero_note_offs),
    ("channel_pressure.mid", channel_pressure),
    ("pitch_bend_range.mid", pitch_bend_range),
    ("drum_key_tune.mid", drum_key_tune),
    (
        "kit_bank_on_a_melodic_channel.mid",
        kit_bank_on_a_melodic_channel,
    ),
    ("bend_left_off_centre.mid", bend_left_off_centre),
    ("track_without_end_of_track.mid", track_without_end_of_track),
    ("melody_and_accompaniment.mid", melody_and_accompaniment),
    ("melody_on_channel_fifteen.mid", melody_on_channel_fifteen),
    ("legacy_encoded_lyrics.mid", legacy_encoded_lyrics),
    ("smpte_timed.mid", smpte_timed),
    ("ambiguous_melody.mid", ambiguous_melody),
    (
        "melody_with_monophonic_bass.mid",
        melody_with_monophonic_bass,
    ),
    ("chords_only.mid", chords_only),
    ("drums_and_lyrics.mid", drums_and_lyrics),
    ("high_quality_song.mid", high_quality_song),
    ("lyrics_all_at_zero.mid", lyrics_all_at_zero),
    ("credits_in_the_lyric_track.mid", credits_in_the_lyric_track),
    ("lyrics_that_stop_early.mid", lyrics_that_stop_early),
    ("chord_names_as_lyrics.mid", chord_names_as_lyrics),
    ("word_ends_unmarked.mid", word_ends_unmarked),
    (
        "word_ends_unmarked_lines_marked.mid",
        word_ends_unmarked_lines_marked,
    ),
    (
        "word_ends_unmarked_short_words.mid",
        word_ends_unmarked_short_words,
    ),
    ("word_boundaries_unmarked.mid", word_boundaries_unmarked),
    ("truncated_by_realtime_byte.mid", truncated_by_realtime_byte),
    ("unbalanced_note_on.mid", unbalanced_note_on),
];

/// Every fixture that must be **refused**, with the filename each should be written under.
///
/// Deliberately a second list rather than entries in [`FIXTURES`]: four sweeps across this crate and
/// `km-suitability` require every fixture in that one to parse, which is the exact opposite of what
/// these are for. Keeping them apart is what lets both claims be made by iterating a list.
pub const UNREADABLE_FIXTURES: &[Fixture] = &[
    ("not_midi_random_bytes.kar", not_midi_random_bytes),
    ("not_midi_text_file.mid", not_midi_text_file),
    ("crlf_corrupted_no_tracks.mid", crlf_corrupted_no_tracks),
    ("header_declares_no_tracks.mid", header_declares_no_tracks),
];
