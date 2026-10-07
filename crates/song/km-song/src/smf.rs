//! A standard MIDI file as bytes: its chunks, and each track's events with their bytes intact.
//!
//! [`crate::Song`] is what a player needs and cannot be written back, because it keeps the channel
//! voice events and drops the rest. This module is for a program that changes a file's text and
//! must leave every other byte alone, so an event is its delta and its bytes and nothing is
//! interpreted that does not have to be.
//!
//! **Running status is expanded on the way through.** It has to be: dropping an event that carries
//! a status byte would silently change the meaning of every running-status event after it, which is
//! the kind of corruption that plays almost correctly.

/// Why a file's bytes could not be walked, or could not be rewritten.
#[derive(Debug, thiserror::Error)]
pub enum SmfError {
    /// The bytes do not open with a header chunk.
    #[error("not a standard MIDI file")]
    NotMidi,
    /// A chunk says it is longer than the bytes that are left.
    #[error("truncated chunk at byte {0}")]
    TruncatedChunk(usize),
    /// The file has a header and no track.
    #[error("no tracks")]
    NoTracks,
    /// A data byte arrived before any status byte.
    #[error("running status with no status byte")]
    RunningStatus,
    /// An event says it is longer than the track that holds it.
    #[error("event runs past the end of its track")]
    EventPastEnd,
    /// A variable-length quantity is cut off by the end of the track.
    #[error("a length runs past the end of its track")]
    LengthPastEnd,
    /// A variable-length quantity has more than the four bytes the format allows.
    #[error("a variable-length quantity longer than four bytes")]
    LongQuantity,
    /// A format 2 file holds independent patterns, so a track added to it is not part of any song.
    #[error("a format 2 MIDI file cannot take another track")]
    Format2,
    /// The header counts tracks in sixteen bits.
    #[error("more tracks than a MIDI file can count")]
    TooManyTracks,
}

/// One MIDI event, with its status byte always present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Ticks since the event before it in the same track.
    pub delta: u32,
    /// The event as the file holds it, from the status byte on.
    pub bytes: Vec<u8>,
}

impl Event {
    /// A meta event of this type with this payload, at delta zero.
    pub fn meta(kind: u8, payload: &[u8]) -> Self {
        let mut bytes = vec![0xFF, kind];
        push_vlq(&mut bytes, u32::try_from(payload.len()).unwrap_or(u32::MAX));
        bytes.extend_from_slice(payload);
        Self { delta: 0, bytes }
    }

    /// Whether this is a meta event of the given type.
    pub fn is_meta(&self, kind: u8) -> bool {
        self.bytes.first() == Some(&0xFF) && self.bytes.get(1) == Some(&kind)
    }

    /// A meta event's payload.
    pub fn payload(&self) -> Option<&[u8]> {
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
    pub fn set_payload(&mut self, payload: &[u8]) {
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

/// Splits a standard MIDI file into its header chunk and the bodies of its track chunks.
pub fn split_chunks(bytes: &[u8]) -> Result<(Vec<u8>, Vec<&[u8]>), SmfError> {
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
        return Err(SmfError::NotMidi);
    }
    let header_len = 8 + be_len(&bytes[4..8]);
    if header_len > bytes.len() {
        return Err(SmfError::TruncatedChunk(0));
    }
    let mut at = header_len;
    let mut chunks = Vec::new();
    while at + 8 <= bytes.len() {
        let len = be_len(&bytes[at + 4..at + 8]);
        let body = at
            .checked_add(8 + len)
            .and_then(|end| bytes.get(at + 8..end))
            .ok_or(SmfError::TruncatedChunk(at))?;
        if &bytes[at..at + 4] == b"MTrk" {
            chunks.push(body);
        }
        at += 8 + len;
    }
    if chunks.is_empty() {
        return Err(SmfError::NoTracks);
    }
    Ok((bytes[..header_len].to_vec(), chunks))
}

/// Walks one track's bytes into events, expanding running status.
pub fn parse_track(body: &[u8]) -> Result<Vec<Event>, SmfError> {
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
                let len = vlq_len(body, &mut at)?;
                at = at.saturating_add(len);
                running = None;
                slice(body, start, at)?
            }
            0xF0 | 0xF7 => {
                let start = at;
                at += 1;
                let len = vlq_len(body, &mut at)?;
                at = at.saturating_add(len);
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
                    return Err(SmfError::RunningStatus);
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
pub fn emit_track(events: &[Event]) -> Vec<u8> {
    let mut data = Vec::new();
    for event in events {
        push_vlq(&mut data, event.delta);
        data.extend_from_slice(&event.bytes);
    }
    emit_chunk(&data)
}

/// Wraps a track body that is already bytes as an `MTrk` chunk.
pub fn emit_chunk(body: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(body.len() + 8);
    chunk.extend_from_slice(b"MTrk");
    chunk.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_be_bytes());
    chunk.extend_from_slice(body);
    chunk
}

/// Removes the events `drop` picks, keeping the timeline intact.
///
/// A dropped event's delta is added to the next one's, so nothing after it moves. `drop` is given
/// each event's absolute tick with it.
pub fn drop_where(events: &mut Vec<Event>, mut drop: impl FnMut(u32, &Event) -> bool) {
    let mut carried = 0u32;
    let mut tick = 0u32;
    events.retain_mut(|event| {
        tick = tick.saturating_add(event.delta);
        if drop(tick, event) {
            carried = carried.saturating_add(event.delta);
            return false;
        }
        event.delta = event.delta.saturating_add(carried);
        carried = 0;
        true
    });
}

/// Builds a whole track from events placed at absolute ticks.
///
/// The track opens with its name and closes with the end-of-track event. Events sharing a tick keep
/// the order they were given in.
pub fn track_at(name: &str, mut placed: Vec<(u32, Event)>) -> Vec<Event> {
    placed.sort_by_key(|(tick, _)| *tick);
    let mut events = Vec::with_capacity(placed.len() + 2);
    events.push(Event::meta(0x03, name.as_bytes()));
    let mut last = 0u32;
    for (tick, mut event) in placed {
        event.delta = tick - last;
        last = tick;
        events.push(event);
    }
    events.push(Event::meta(0x2F, b""));
    events
}

/// The header chunk for the same file holding `tracks` tracks.
///
/// A format 0 file holds exactly one track, so it becomes format 1 when it is given more. Its one
/// track is then the first, which is where format 1 keeps the tempo.
pub fn header_for(header: &[u8], tracks: usize) -> Result<Vec<u8>, SmfError> {
    if header.len() < 14 {
        return Err(SmfError::NotMidi);
    }
    let mut out = header.to_vec();
    let format = u16::from_be_bytes([out[8], out[9]]);
    if format == 2 {
        return Err(SmfError::Format2);
    }
    if format == 0 && tracks > 1 {
        out[8..10].copy_from_slice(&1u16.to_be_bytes());
    }
    let count = u16::try_from(tracks).map_err(|_| SmfError::TooManyTracks)?;
    out[10..12].copy_from_slice(&count.to_be_bytes());
    Ok(out)
}

/// Reads a variable-length quantity, advancing `at`.
pub fn read_vlq(body: &[u8], at: &mut usize) -> Result<u32, SmfError> {
    let mut value = 0u32;
    for _ in 0..4 {
        let Some(&byte) = body.get(*at) else {
            return Err(SmfError::LengthPastEnd);
        };
        *at += 1;
        value = (value << 7) | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(SmfError::LongQuantity)
}

/// Writes a variable-length quantity.
pub fn push_vlq(out: &mut Vec<u8>, mut value: u32) {
    let mut buffer = [0u8; 4];
    let mut len = 0;
    loop {
        buffer[len] = u8::try_from(value & 0x7F).unwrap_or(0);
        len += 1;
        value >>= 7;
        if value == 0 || len == buffer.len() {
            break;
        }
    }
    for index in (0..len).rev() {
        out.push(buffer[index] | if index == 0 { 0x00 } else { 0x80 });
    }
}

/// A big-endian chunk length. One too large for the platform is past the end of any file.
fn be_len(four: &[u8]) -> usize {
    let value = u32::from_be_bytes([four[0], four[1], four[2], four[3]]);
    usize::try_from(value).unwrap_or(usize::MAX / 2)
}

/// A variable-length quantity used as a length.
fn vlq_len(body: &[u8], at: &mut usize) -> Result<usize, SmfError> {
    let len = read_vlq(body, at)?;
    usize::try_from(len).map_err(|_| SmfError::EventPastEnd)
}

/// How many data bytes a channel status takes.
fn data_len(status: u8) -> usize {
    match status & 0xF0 {
        0xC0 | 0xD0 => 1,
        _ => 2,
    }
}

/// Copies a range, or reports the truncation.
fn slice(body: &[u8], from: usize, to: usize) -> Result<Vec<u8>, SmfError> {
    body.get(from..to)
        .map(<[u8]>::to_vec)
        .ok_or(SmfError::EventPastEnd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dropping an event must not move anything after it.
    #[test]
    fn dropping_an_event_keeps_the_timeline() {
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
        drop_where(&mut events, |_, event| event.is_meta(0x01));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta, 30);
    }

    #[test]
    fn the_predicate_is_given_each_events_absolute_tick() {
        let mut events = vec![
            Event {
                delta: 10,
                ..Event::meta(0x01, b"a")
            },
            Event {
                delta: 20,
                ..Event::meta(0x01, b"b")
            },
        ];
        let mut seen = Vec::new();
        drop_where(&mut events, |tick, _| {
            seen.push(tick);
            false
        });
        assert_eq!(seen, [10, 30]);
    }

    /// A dropped status byte must not orphan what follows.
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
    fn a_placed_track_is_named_ordered_and_closed() {
        let track = track_at(
            "Words",
            vec![
                (480, Event::meta(0x01, b"b")),
                (0, Event::meta(0x01, b"a")),
                (480, Event::meta(0x01, b"c")),
            ],
        );
        let texts: Vec<_> = track
            .iter()
            .map(|event| (event.delta, event.payload().unwrap().to_vec()))
            .collect();
        assert_eq!(
            texts,
            [
                (0, b"Words".to_vec()),
                (0, b"a".to_vec()),
                (480, b"b".to_vec()),
                (0, b"c".to_vec()),
                (0, Vec::new()),
            ]
        );
        assert!(track[0].is_meta(0x03));
        assert!(track[4].is_meta(0x2F));
    }

    #[test]
    fn a_format_0_header_becomes_format_1_when_it_gains_a_track() {
        let header = b"MThd\x00\x00\x00\x06\x00\x00\x00\x01\x01\xE0";
        let out = header_for(header, 3).unwrap();
        assert_eq!(&out[8..12], &[0, 1, 0, 3]);
        assert_eq!(&out[12..], &header[12..]);
    }

    #[test]
    fn a_format_2_header_is_refused() {
        let header = b"MThd\x00\x00\x00\x06\x00\x02\x00\x01\x01\xE0";
        assert!(matches!(header_for(header, 2), Err(SmfError::Format2)));
    }

    #[test]
    fn a_chunk_longer_than_the_file_is_reported() {
        let mut bytes = b"MThd\x00\x00\x00\x06\x00\x01\x00\x01\x01\xE0".to_vec();
        bytes.extend_from_slice(b"MTrk\x00\x00\x00\x40\x00\xFF\x2F\x00");
        assert!(matches!(
            split_chunks(&bytes),
            Err(SmfError::TruncatedChunk(14))
        ));
    }
}
