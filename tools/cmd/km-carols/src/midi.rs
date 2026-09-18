//! The pass that turns abc2midi's output into a karaoke file the machine can make the most of.
//!
//! abc2midi writes Soft Karaoke natively — `@KMIDI KARAOKE FILE`, the syllables as text events with
//! `\` and `/` line markers, a `Words` track under `-STFW`. What it does not write is a **track
//! name**, and that one omission costs two points out of ten.
//!
//! The reason is worth stating, because it is not obvious and it is not abc2midi's fault. A hymn
//! setting is homophonic: soprano, alto, tenor and bass move together, so every voice's note onsets
//! line up with the lyrics exactly as well as every other's. `km_suitability::melody` scores a
//! candidate on `4 × lyric_alignment + 3 if the track is named like a melody + monophony`, and
//! requires the winner to beat the runner-up by half again. Four voices that tie on alignment and
//! tie on monophony produce `MelodyOutcome::Ambiguous`, and the machine's guide-melody toggle goes
//! away. The `+3` for a name is the only thing that separates them — and abc2midi parses ABC's
//! `V: … name=` for the typesetter without ever writing it to the MIDI.
//!
//! So this pass:
//!
//! 1. names the track carrying channel 0 — the first voice, which is the soprano in every tune in
//!    the pack — `Melody`;
//! 2. writes `@T<title>`, `@T<artist>` and `@L<language>` into the words track, so the `.kar` is
//!    self-describing rather than relying on the package manifest to say what it is;
//! 3. drops abc2midi's own annotations, which are plain text events in the lyric stream and
//!    therefore arrive as the song's first line on screen;
//! 4. subdivides lyric lines that are too long to read, because a hymnal writes one `w:` line per
//!    musical *system* and a system is a whole phrase. See [`rewrap_lines`], which is where the
//!    measurement behind that lives.
//!
//! **Running status is expanded on the way through.** It has to be: dropping an event that carries
//! a status byte would silently change the meaning of every running-status event after it, which is
//! the kind of corruption that plays almost correctly.

use anyhow::{Result, bail};
use km_song::COMFORTABLE_LINE_CHARS;

/// The words abc2midi labels its tracks with, as ordinary text events.
///
/// Written unconditionally by `genmidi.c`; no flag suppresses them, `-NCOM` included. They are in
/// the same text stream as the syllables, so km-song reads them as the first line of the song.
const ANNOTATIONS: [&str; 6] = [
    "note track",
    "lyric track",
    "notes/lyric track",
    "gchord track",
    "drum track",
    "drone track",
];

/// What the rewritten file should say about itself.
#[derive(Debug, Clone)]
pub struct Headers {
    /// Soft Karaoke's first `@T` line.
    pub title: String,
    /// Soft Karaoke's second `@T` line.
    pub artist: String,
    /// The `@L` line, in the four-letter form real files use (`ENGL`).
    pub language: String,
}

/// The channel whose track is named as the melody. The first voice of every tune in the pack.
const MELODY_CHANNEL: u8 = 0;

/// The track name `km_suitability::melody` looks for.
const MELODY_NAME: &[u8] = b"Melody";

/// Rewrites one abc2midi file. See the module header for what changes and why.
pub fn rewrite(bytes: &[u8], headers: &Headers) -> Result<Vec<u8>> {
    let (header, chunks) = split_chunks(bytes)?;

    let mut tracks: Vec<Vec<Event>> = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        tracks.push(parse_track(chunk)?);
    }

    let words = tracks.iter().position(|t| has_annotation(t, "lyric track"));
    let melody = tracks.iter().position(|t| plays_channel(t, MELODY_CHANNEL));

    for (index, track) in tracks.iter_mut().enumerate() {
        drop_annotations(track);

        if Some(index) == melody && !track.iter().any(|e| e.is_meta(0x03)) {
            track.insert(0, Event::meta(0x03, MELODY_NAME));
        }
        if Some(index) == words {
            // Behind any existing header text, so `@KMIDI KARAOKE FILE` keeps its place if the
            // words happen to share track 0.
            let at = track.iter().position(|e| !e.is_meta(0x01)).unwrap_or(0);
            for (offset, line) in soft_karaoke_headers(headers).into_iter().enumerate() {
                track.insert(at + offset, Event::meta(0x01, line.as_bytes()));
            }
            // After the headers, so the `@` lines are in place to be skipped rather than counted.
            rewrap_lines(track, COMFORTABLE_LINE_CHARS);
        }
    }

    if melody.is_none() {
        bail!("no track plays channel {MELODY_CHANNEL}, so the melody cannot be named");
    }
    if words.is_none() {
        bail!("no lyric track found; was abc2midi run with -STFW?");
    }

    let mut out = header;
    for track in &tracks {
        out.extend_from_slice(&emit_track(track));
    }
    Ok(out)
}

/// The `@` lines to put at the head of the words track.
fn soft_karaoke_headers(headers: &Headers) -> Vec<String> {
    vec![
        format!("@L{}", headers.language),
        format!("@T{}", headers.title),
        format!("@T{}", headers.artist),
    ]
}

/// One MIDI event, with its status byte always present.
#[derive(Debug, Clone)]
struct Event {
    delta: u32,
    bytes: Vec<u8>,
}

impl Event {
    /// A meta event of this type with this payload, at delta zero.
    fn meta(kind: u8, payload: &[u8]) -> Self {
        let mut bytes = vec![0xFF, kind];
        push_vlq(&mut bytes, u32::try_from(payload.len()).unwrap_or(u32::MAX));
        bytes.extend_from_slice(payload);
        Self { delta: 0, bytes }
    }

    /// Whether this is a meta event of the given type.
    fn is_meta(&self, kind: u8) -> bool {
        self.bytes.first() == Some(&0xFF) && self.bytes.get(1) == Some(&kind)
    }

    /// A meta event's payload.
    fn payload(&self) -> Option<&[u8]> {
        if self.bytes.first() != Some(&0xFF) {
            return None;
        }
        let mut at = 2;
        let len = read_vlq(&self.bytes, &mut at).ok()?;
        self.bytes.get(at..at + usize::try_from(len).ok()?)
    }

    /// Replaces a meta event's payload, keeping its type and its delta.
    ///
    /// The length is a variable-length quantity, so a payload that crosses 127 bytes changes the
    /// event's size — which is why this rebuilds rather than writing in place.
    fn set_payload(&mut self, payload: &[u8]) {
        let Some(&kind) = self.bytes.get(1) else {
            return;
        };
        let delta = self.delta;
        *self = Self {
            delta,
            ..Self::meta(kind, payload)
        };
    }
}

/// Splits a standard MIDI file into its header chunk and its track chunks.
fn split_chunks(bytes: &[u8]) -> Result<(Vec<u8>, Vec<&[u8]>)> {
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
        bail!("not a standard MIDI file");
    }
    let header_len =
        8 + usize::try_from(u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]))?;
    let mut at = header_len;
    let mut chunks = Vec::new();
    while at + 8 <= bytes.len() {
        let len = usize::try_from(u32::from_be_bytes([
            bytes[at + 4],
            bytes[at + 5],
            bytes[at + 6],
            bytes[at + 7],
        ]))?;
        let body = bytes
            .get(at + 8..at + 8 + len)
            .ok_or_else(|| anyhow::anyhow!("truncated chunk at byte {at}"))?;
        if &bytes[at..at + 4] == b"MTrk" {
            chunks.push(body);
        }
        at += 8 + len;
    }
    if chunks.is_empty() {
        bail!("no tracks");
    }
    Ok((bytes[..header_len].to_vec(), chunks))
}

/// Walks one track's bytes into events, expanding running status.
fn parse_track(body: &[u8]) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut at = 0usize;
    let mut running: Option<u8> = None;

    while at < body.len() {
        let delta = read_vlq(body, &mut at)?;
        let Some(&first) = body.get(at) else { break };

        let bytes = match first {
            0xFF => {
                let start = at;
                at += 2;
                let len = usize::try_from(read_vlq(body, &mut at)?)?;
                at += len;
                running = None;
                slice(body, start, at)?
            }
            0xF0 | 0xF7 => {
                let start = at;
                at += 1;
                let len = usize::try_from(read_vlq(body, &mut at)?)?;
                at += len;
                running = None;
                slice(body, start, at)?
            }
            status if status >= 0x80 => {
                running = Some(status);
                let start = at;
                at += 1 + data_len(status);
                slice(body, start, at)?
            }
            _ => {
                // Running status: the status byte is implied, so write it back out explicitly.
                let Some(status) = running else {
                    bail!("running status with no status byte")
                };
                let start = at;
                at += data_len(status);
                let mut bytes = vec![status];
                bytes.extend_from_slice(&slice(body, start, at)?);
                bytes
            }
        };
        events.push(Event { delta, bytes });
    }
    Ok(events)
}

/// Re-emits a track as an `MTrk` chunk.
fn emit_track(events: &[Event]) -> Vec<u8> {
    let mut data = Vec::new();
    for event in events {
        push_vlq(&mut data, event.delta);
        data.extend_from_slice(&event.bytes);
    }
    let mut chunk = Vec::with_capacity(data.len() + 8);
    chunk.extend_from_slice(b"MTrk");
    chunk.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    chunk.extend_from_slice(&data);
    chunk
}

/// Whether the track carries this abc2midi label.
fn has_annotation(events: &[Event], label: &str) -> bool {
    events
        .iter()
        .any(|e| e.is_meta(0x01) && e.payload() == Some(label.as_bytes()))
}

/// Whether the track puts notes on this channel.
fn plays_channel(events: &[Event], channel: u8) -> bool {
    events.iter().any(|e| {
        matches!(e.bytes.first(), Some(&status) if status & 0xF0 == 0x90 && status & 0x0F == channel)
            && e.bytes.get(2).is_some_and(|velocity| *velocity > 0)
    })
}

/// Removes abc2midi's track labels, keeping the timeline intact.
///
/// A dropped event's delta is added to the next one's, so nothing after it moves.
fn drop_annotations(events: &mut Vec<Event>) {
    let mut carried = 0u32;
    events.retain_mut(|event| {
        let junk = event.is_meta(0x01)
            && event.payload().is_some_and(|p| {
                let text = String::from_utf8_lossy(p);
                ANNOTATIONS.contains(&text.as_ref())
                    || text
                        .strip_prefix("X:")
                        .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            });
        if junk {
            carried += event.delta;
            return false;
        }
        event.delta += carried;
        carried = 0;
        true
    });
}

/// The characters that ask for a line break, on either end of a syllable.
///
/// The same set `km_song::karaoke` reads, and it has to stay that way: a marker this pass does
/// not recognize is a line boundary it measures straight through.
const MARKERS: [char; 4] = ['\\', '/', '\r', '\n'];

/// The punctuation that ends a clause, and so makes the best place to break a long line.
const CLAUSE_ENDS: [char; 6] = [',', ';', ':', '!', '?', '.'];

/// Subdivides over-long lyric lines, so a hymnal's phrases become karaoke lines.
///
/// **This is a display decision made on the syllable stream, which is why it is here and not in the
/// ABC.** A hymnal writes one `w:` line per *system* — a whole musical phrase — and the conversion
/// maps one system to one displayed line, so the pack's lines averaged 58 characters against a
/// corpus average of 27, and 92% of them were past [`COMFORTABLE_LINE_CHARS`]. Re-marking the
/// stream moves no timing and needs no note counting; re-cutting the ABC could not be done at all,
/// because consecutive `w:` lines there are *verses* rather than continuations.
///
/// Three rules, and the first is the one that keeps this honest:
///
/// 1. **Existing markers are kept and never recomputed.** A `\` is a verse start and the hymnal's
///    own `/` is a phrase end; both are better than anything inferrable here. Long lines are
///    subdivided, and nothing is ever joined.
/// 2. **A break goes only before a syllable that starts a word**, which in Soft Karaoke means one
///    whose text begins with a space. Breaking mid-word (`Emman-` / `uel`) reads worse than
///    overflowing. The space is *replaced* by the marker rather than kept behind it, or the next
///    line would be drawn one space off center.
/// 3. **The latest clause boundary that still fits wins, else the latest word boundary.** One rule
///    and not two: also breaking eagerly at any comma past half the budget was tried, and phrased
///    worse.
///
/// A line with no legal break point — one unbroken word longer than the budget — is left alone.
/// There is nothing to be done for it here, and [`crate::check`] exempts the same case.
fn rewrap_lines(events: &mut [Event], budget: usize) {
    // The syllables, in order, as (index into `events`, text). `@` lines are Soft Karaoke headers
    // rather than words, and abc2midi's annotations are already gone by now.
    let syllables: Vec<(usize, String)> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.is_meta(0x01))
        .filter_map(|(index, event)| {
            let payload = event.payload()?;
            (!payload.starts_with(b"@"))
                .then(|| (index, String::from_utf8_lossy(payload).into_owned()))
        })
        .collect();
    let count = syllables.len();

    // Split each syllable the way `km_song::karaoke::split_break_markers` does. **Both ends have to
    // be inspected**: abc2midi opens most lines with a marker but closes some by appending one, and
    // a pass that saw only the first kind would measure two lines as one.
    let parts: Vec<(&str, &str, &str)> = syllables
        .iter()
        .map(|(_, text)| {
            let after_lead = text.trim_start_matches(MARKERS);
            let body = after_lead.trim_end_matches(MARKERS);
            (
                &text[..text.len() - after_lead.len()],
                body,
                &after_lead[body.len()..],
            )
        })
        .collect();
    let body = |at: usize| parts[at].1;

    // **A marker often arrives as an event of its own**, with no text on it at all — abc2midi
    // writes a bare `/` and then ` wa`. So "the first syllable of the line" and "the syllable that
    // opens the line" are not the same index, and the leading space to drop belongs to the former.
    let visible = |at: usize| !body(at).trim().is_empty();

    // Whether a line begins here: the source asked for one on either side, or it is the first.
    let mut opens: Vec<bool> = (0..count)
        .map(|at| at == 0 || !parts[at].0.is_empty() || !parts[at - 1].2.is_empty())
        .collect();

    // What this syllable puts on screen. The line's first visible one loses the whitespace behind
    // its marker; where we are the ones breaking, that whitespace is the word separator it stands
    // in for.
    let width = |at: usize, opening: bool| {
        if opening {
            body(at).trim_start().chars().count()
        } else {
            body(at).chars().count()
        }
    };

    // The nearest earlier syllable carrying text, for asking what the previous word ended with.
    let previous = |at: usize| (0..at).rev().find(|&i| visible(i));

    // The width of a line running from `from` through `to`.
    let run = |from: usize, to: usize| {
        let mut total = 0;
        let mut opening = true;
        for at in from..=to {
            total += width(at, opening);
            opening &= !visible(at);
        }
        total
    };

    let mut len = 0usize;
    // Whether we are still before the line's first visible syllable, and so have nothing to break.
    let mut opening = true;
    // The latest legal break point since the line opened. Both are cleared whenever a line starts,
    // and both were recorded while the line still fitted, so breaking at either is guaranteed to
    // leave a line within budget.
    let mut clause: Option<usize> = None;
    let mut word: Option<usize> = None;
    let mut cuts: Vec<usize> = Vec::new();

    for (at, &starts_a_line) in opens.iter().enumerate() {
        if starts_a_line {
            len = 0;
            opening = true;
            clause = None;
            word = None;
        }

        if !opening {
            if body(at).starts_with(' ') {
                word = Some(at);
                if previous(at).is_some_and(|p| body(p).trim_end().ends_with(CLAUSE_ENDS)) {
                    clause = Some(at);
                }
            }

            if len + width(at, false) > budget
                && let Some(cut) = clause.or(word)
            {
                cuts.push(cut);
                len = run(cut, at);
                // `at` is still a candidate for the *next* break, unless it is the cut itself.
                let carry = at > cut && body(at).starts_with(' ');
                word = carry.then_some(at);
                clause = carry
                    .then(|| previous(at))
                    .flatten()
                    .filter(|&p| body(p).trim_end().ends_with(CLAUSE_ENDS))
                    .map(|_| at);
                continue;
            }
        }

        len += width(at, opening);
        opening &= !visible(at);
    }

    // Rebalance an orphaned tail. Filling each line as far as it will go now and then leaves a
    // fragment alone on the last one -- "In the bleak midwinter a stable place" / "sufficed".
    // Moving the *last* cut earlier splits the remaining words evenly instead. This is one
    // adjustment to a break already made rather than a second rule about where breaks go, and it
    // is deliberately confined to a tail that is genuinely short: an early cut at a clause is a
    // choice, and only a stranded ending is a mistake.
    let mut from = 0;
    while from < count {
        let mut to = from + 1;
        while to < count && !opens[to] {
            to += 1;
        }
        let inside: Vec<usize> = cuts
            .iter()
            .copied()
            .filter(|&c| c > from && c < to)
            .collect();
        if let Some(&last) = inside.last()
            && run(last, to - 1) < budget / 3
        {
            let end = to - 1;
            let start = inside.len().checked_sub(2).map_or(from, |i| inside[i]);
            let best = (start + 1..=end)
                .filter(|&c| body(c).starts_with(' '))
                .filter(|&c| run(start, c - 1) <= budget && run(c, end) <= budget)
                .max_by_key(|&c| {
                    let clause =
                        previous(c).is_some_and(|p| body(p).trim_end().ends_with(CLAUSE_ENDS));
                    (run(start, c - 1).min(run(c, end)), usize::from(clause))
                });
            if let Some(best) = best
                && let Some(position) = cuts.iter().position(|&c| c == last)
            {
                cuts[position] = best;
            }
        }
        from = to;
    }

    for &cut in &cuts {
        opens[cut] = true;
    }

    // Which syllables carry a line's first and last text, so their outer whitespace can go. A line
    // drawn with a leading space sits a space off center, and abc2midi writes plenty of them. This
    // is whitespace at a line's edge and never a second opinion about where the line goes, so the
    // rule that existing markers are kept is intact.
    let mut trim_left = vec![false; count];
    let mut trim_right = vec![false; count];
    let mut from = 0;
    while from < count {
        let mut to = from + 1;
        while to < count && !opens[to] {
            to += 1;
        }
        // Everything up to the first text and everything after the last, so a marker-only or
        // space-only event at either end is emptied rather than left to pad the line.
        let first = (from..to).find(|&at| visible(at));
        let last = (from..to).rev().find(|&at| visible(at));
        trim_left[from..first.map_or(to, |f| f + 1)].fill(true);
        trim_right[last.unwrap_or(from)..to].fill(true);
        from = to;
    }

    for at in 0..count {
        let (index, ref text) = syllables[at];
        let mut body = parts[at].1;
        if trim_left[at] {
            body = body.trim_start();
        }
        if trim_right[at] {
            body = body.trim_end();
        }
        let lead = if cuts.contains(&at) { "/" } else { parts[at].0 };
        let trail = parts[at].2;
        let rewritten = format!("{lead}{body}{trail}");
        if rewritten != *text {
            events[index].set_payload(rewritten.as_bytes());
        }
    }
}

/// How many data bytes a channel status takes.
fn data_len(status: u8) -> usize {
    match status & 0xF0 {
        0xC0 | 0xD0 => 1,
        _ => 2,
    }
}

/// Copies a range, or reports the truncation.
fn slice(body: &[u8], from: usize, to: usize) -> Result<Vec<u8>> {
    body.get(from..to)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| anyhow::anyhow!("event runs past the end of its track"))
}

/// Reads a variable-length quantity, advancing `at`.
fn read_vlq(body: &[u8], at: &mut usize) -> Result<u32> {
    let mut value = 0u32;
    for _ in 0..4 {
        let Some(&byte) = body.get(*at) else {
            bail!("a length runs past the end of its track")
        };
        *at += 1;
        value = (value << 7) | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("a variable-length quantity longer than four bytes")
}

/// Writes a variable-length quantity.
fn push_vlq(out: &mut Vec<u8>, mut value: u32) {
    let mut buffer = [0u8; 4];
    let mut len = 0;
    loop {
        buffer[len] = u8::try_from(value & 0x7F).unwrap_or(0);
        len += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    for index in (0..len).rev() {
        out.push(buffer[index] | if index == 0 { 0x00 } else { 0x80 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> Headers {
        Headers {
            title: "Silent Night".to_owned(),
            artist: "Traditional".to_owned(),
            language: "ENGL".to_owned(),
        }
    }

    /// A two-track file shaped the way abc2midi's `-STFW` output is: one note track on channel 0
    /// carrying its label, one lyric track carrying its label and a syllable.
    fn abc2midi_shaped() -> Vec<u8> {
        let mut notes = vec![Event::meta(0x01, b"note track")];
        notes.push(Event {
            delta: 0,
            bytes: vec![0x90, 60, 100],
        });
        notes.push(Event {
            delta: 240,
            bytes: vec![0x80, 60, 64],
        });
        notes.push(Event::meta(0x2F, b""));

        let words = vec![
            Event::meta(0x01, b"@KMIDI KARAOKE FILE"),
            Event::meta(0x01, b"lyric track"),
            Event::meta(0x01, b"X:68"),
            Event {
                delta: 0,
                bytes: vec![0xFF, 0x01, 5, b'\\', b'S', b'i', b'l', b'e'],
            },
            Event::meta(0x2F, b""),
        ];

        let mut out = b"MThd\x00\x00\x00\x06\x00\x01\x00\x02\x01\xE0".to_vec();
        out.extend_from_slice(&emit_track(&notes));
        out.extend_from_slice(&emit_track(&words));
        out
    }

    fn text_events(bytes: &[u8]) -> Vec<String> {
        let (_, chunks) = split_chunks(bytes).unwrap();
        chunks
            .iter()
            .flat_map(|c| parse_track(c).unwrap())
            .filter(|e| e.is_meta(0x01))
            .filter_map(|e| e.payload().map(|p| String::from_utf8_lossy(p).into_owned()))
            .collect()
    }

    /// Builds a words track from these syllables, rewraps it, and reads the payloads back.
    fn wrapped(syllables: &[&str], budget: usize) -> Vec<String> {
        let mut events: Vec<Event> = vec![Event::meta(0x01, b"@KMIDI KARAOKE FILE")];
        events.extend(syllables.iter().map(|s| Event::meta(0x01, s.as_bytes())));
        events.push(Event::meta(0x2F, b""));
        rewrap_lines(&mut events, budget);
        events
            .iter()
            .filter(|e| e.is_meta(0x01))
            .filter_map(|e| e.payload().map(|p| String::from_utf8_lossy(p).into_owned()))
            .filter(|t| !t.starts_with('@'))
            .collect()
    }

    #[test]
    fn a_line_within_the_budget_is_left_exactly_as_it_was() {
        let input = ["Joy", " to", " the", " world"];
        assert_eq!(wrapped(&input, 40), input);
    }

    #[test]
    fn a_long_line_is_broken_at_a_word_boundary_and_never_inside_a_word() {
        // "Joy to the world and" is 20; " more" would make 25.
        let out = wrapped(
            &["Joy", " to", " the", " world", " and", " more", " words"],
            20,
        );
        assert_eq!(
            out,
            ["Joy", " to", " the", " world", " and", "/more", " words"],
            "{out:?}"
        );
        // Every break replaced the space that separated two words, so none landed mid-word.
        assert!(
            out.iter().all(|s| !s.starts_with("/-")),
            "a break landed inside a word: {out:?}"
        );
    }

    #[test]
    fn a_syllable_that_does_not_open_a_word_is_never_broken_before() {
        // "Emman-u-el" is one word in three syllables. Cutting before "u-" or "el" would put half
        // a word on each line, which reads worse than letting the line overflow.
        let input = ["Oh", " come", " Emman-", "u-", "el", " shall", " ransom"];
        let out = wrapped(&input, 12);
        for (before, after) in input.iter().zip(&out) {
            assert!(
                before.starts_with(' ') || !after.starts_with('/'),
                "broke inside a word, before {before:?}: {out:?}"
            );
        }
        assert!(out.iter().any(|s| s.starts_with('/')), "{out:?}");
    }

    #[test]
    fn a_clause_boundary_is_preferred_to_a_bare_word_boundary() {
        // Both fit, and the comma is the musical phrase end even though it is the earlier cut.
        let out = wrapped(
            &["Oh", " say,", " can", " you", " see", " the", " dawn"],
            20,
        );
        assert_eq!(
            out,
            ["Oh", " say,", "/can", " you", " see", " the", " dawn"],
            "{out:?}"
        );
    }

    #[test]
    fn a_line_closed_by_a_trailing_marker_is_still_a_line() {
        // abc2midi closes some lines by appending the marker instead of opening the next with one.
        // Seen only from the leading end, these two lines measure as one 33-character line -- and
        // the second would keep the space that the marker stands in for.
        let out = wrapped(
            &[
                "Earth", " stood", " hard/", " water", " like", " a", " stone",
            ],
            40,
        );
        assert_eq!(
            out,
            [
                "Earth", " stood", " hard/", "water", " like", " a", " stone"
            ],
            "{out:?}"
        );
    }

    #[test]
    fn the_hymnals_own_markers_survive_untouched() {
        let out = wrapped(&["\\Joy", " a", " b", "/Second", " c"], 40);
        assert_eq!(out[0], "\\Joy", "{out:?}");
        assert_eq!(out[3], "/Second", "{out:?}");
    }

    #[test]
    fn one_unbreakable_word_is_left_alone_rather_than_cut() {
        let input = ["Supercalifragilisticexpialidocious"];
        assert_eq!(wrapped(&input, 10), input);
    }

    #[test]
    fn only_the_markers_change_so_no_timing_can_have_moved() {
        let input = [
            "Hark", " the", " her-", "ald", " an-", "gels", " sing,", " glo-", "ry", " to", " the",
            " new-", "born", " King",
        ];
        let out = wrapped(&input, 20);
        assert_eq!(out.len(), input.len(), "{out:?}");
        let words = |s: &str| s.trim_start_matches(['\\', '/']).trim_start().to_owned();
        assert_eq!(
            out.iter().map(|s| words(s)).collect::<Vec<_>>(),
            input.iter().map(|s| words(s)).collect::<Vec<_>>(),
            "{out:?}"
        );
        assert!(out.iter().any(|s| s.starts_with('/')), "{out:?}");
    }

    #[test]
    fn the_melody_track_is_named() {
        let out = rewrite(&abc2midi_shaped(), &headers()).unwrap();
        let (_, chunks) = split_chunks(&out).unwrap();
        let names: Vec<String> = chunks
            .iter()
            .flat_map(|c| parse_track(c).unwrap())
            .filter(|e| e.is_meta(0x03))
            .filter_map(|e| e.payload().map(|p| String::from_utf8_lossy(p).into_owned()))
            .collect();
        assert_eq!(names, vec!["Melody".to_owned()]);
    }

    #[test]
    fn the_annotations_are_gone_and_the_syllables_are_not() {
        let out = rewrite(&abc2midi_shaped(), &headers()).unwrap();
        let text = text_events(&out);
        assert!(!text.iter().any(|t| t == "note track"), "{text:?}");
        assert!(!text.iter().any(|t| t == "lyric track"), "{text:?}");
        assert!(!text.iter().any(|t| t == "X:68"), "{text:?}");
        assert!(text.iter().any(|t| t == "\\Sile"), "{text:?}");
        assert!(text.iter().any(|t| t == "@KMIDI KARAOKE FILE"), "{text:?}");
    }

    #[test]
    fn the_soft_karaoke_headers_are_written() {
        let out = rewrite(&abc2midi_shaped(), &headers()).unwrap();
        let text = text_events(&out);
        assert!(text.iter().any(|t| t == "@LENGL"), "{text:?}");
        assert!(text.iter().any(|t| t == "@TSilent Night"), "{text:?}");
        assert!(text.iter().any(|t| t == "@TTraditional"), "{text:?}");
    }

    /// Dropping an event must not move anything after it.
    #[test]
    fn dropping_an_annotation_keeps_the_timeline() {
        let mut events = vec![
            Event {
                delta: 10,
                bytes: Event::meta(0x01, b"note track").bytes,
            },
            Event {
                delta: 20,
                bytes: vec![0x90, 60, 100],
            },
        ];
        drop_annotations(&mut events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta, 30);
    }

    /// The hazard the module header names: a dropped status byte must not orphan what follows.
    #[test]
    fn running_status_is_expanded() {
        let mut data = Vec::new();
        push_vlq(&mut data, 0);
        data.extend_from_slice(&[0x90, 60, 100]);
        push_vlq(&mut data, 5);
        data.extend_from_slice(&[62, 100]); // running status
        let events = parse_track(&data).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].bytes, vec![0x90, 62, 100]);
    }

    #[test]
    fn variable_length_quantities_round_trip() {
        for value in [0u32, 1, 127, 128, 8192, 100_000, 0x0FFF_FFFF] {
            let mut out = Vec::new();
            push_vlq(&mut out, value);
            let mut at = 0;
            assert_eq!(read_vlq(&out, &mut at).unwrap(), value);
            assert_eq!(at, out.len());
        }
    }

    #[test]
    fn a_file_with_no_lyric_track_is_refused() {
        let mut out = b"MThd\x00\x00\x00\x06\x00\x00\x00\x01\x01\xE0".to_vec();
        out.extend_from_slice(&emit_track(&[
            Event {
                delta: 0,
                bytes: vec![0x90, 60, 100],
            },
            Event::meta(0x2F, b""),
        ]));
        let err = rewrite(&out, &headers()).unwrap_err().to_string();
        assert!(err.contains("-STFW"), "{err}");
    }
}
