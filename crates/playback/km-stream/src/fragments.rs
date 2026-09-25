//! Cutting a fragmented MP4 into the pieces a browser appends one at a time.
//!
//! **The page's socket carries whole boxes, and the muxer writes bytes.** ffmpeg's `mp4` muxer,
//! told to fragment on every packet, writes an initialisation segment and then a `moof` and an
//! `mdat` for each packet. It hands them to its output in whatever chunks its buffer fills, so
//! [`Splitter`] puts the boxes back together. A browser can append an initialisation segment and
//! then any run of whole fragments, and nothing else.
//!
//! No ffmpeg, like [`crate::pixels`], so the box reading is tested on a machine that has none.
//!
//! # Which fragment a viewer may start on
//!
//! **A viewer joining late starts on a fragment holding a video keyframe**, because a decoder
//! handed anything else shows nothing until the next one. The muxer says which fragment that is in
//! the fragment's own sample flags, and [`Splitter`] reads them rather than being told. So the
//! answer is right however the muxer groups sound and picture.

use std::io;

/// One piece of the stream, whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// The `ftyp` and `moov` boxes. Every viewer needs this first, and it does not change for the
    /// life of a stream.
    Init(Vec<u8>),
    /// A `moof` and the `mdat` it describes.
    Fragment {
        /// The two boxes, back to back.
        bytes: Vec<u8>,
        /// Whether this fragment's first video sample is a keyframe, so a viewer may start here.
        /// A fragment holding only sound is never one.
        key: bool,
    },
}

/// Where whole pieces go.
pub type Sink = Box<dyn FnMut(Piece) + Send>;

/// The ISO BMFF sample flag set on a sample that is *not* a sync sample.
const NON_SYNC_SAMPLE: u32 = 0x0001_0000;

/// Reassembles top-level boxes from what the muxer writes, and hands on whole pieces.
pub struct Splitter {
    sink: Sink,
    /// Bytes written but not yet a whole box.
    pending: Vec<u8>,
    /// The `ftyp`, held until the `moov` completes the initialisation segment.
    file_type: Option<Vec<u8>>,
    /// A `moof`, held until its `mdat` arrives.
    fragment: Option<Vec<u8>>,
    /// The video track's number, and its default sample flags from `trex`, read from the `moov`.
    video: Option<Track>,
}

#[derive(Debug, Clone, Copy)]
struct Track {
    id: u32,
    default_flags: Option<u32>,
}

impl Splitter {
    /// A splitter handing each whole piece to `sink`.
    pub fn new(sink: Sink) -> Self {
        Self {
            sink,
            pending: Vec::new(),
            file_type: None,
            fragment: None,
            video: None,
        }
    }

    /// Takes one whole top-level box.
    fn take(&mut self, kind: [u8; 4], bytes: Vec<u8>) {
        match &kind {
            b"ftyp" => self.file_type = Some(bytes),
            b"moov" => {
                self.video = video_track(body(&bytes));
                let mut init = self.file_type.take().unwrap_or_default();
                init.extend_from_slice(&bytes);
                (self.sink)(Piece::Init(init));
            }
            b"moof" => self.fragment = Some(bytes),
            b"mdat" => {
                if let Some(mut fragment) = self.fragment.take() {
                    let key = self
                        .video
                        .is_some_and(|video| starts_on_keyframe(body(&fragment), video));
                    fragment.extend_from_slice(&bytes);
                    (self.sink)(Piece::Fragment {
                        bytes: fragment,
                        key,
                    });
                }
            }
            // `mfra` at the end, and anything else a muxer may add, is for a file on a disk.
            _ => {}
        }
    }
}

impl io::Write for Splitter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some((kind, size)) = header(&self.pending)? {
            if self.pending.len() < size {
                break;
            }
            let rest = self.pending.split_off(size);
            let whole = std::mem::replace(&mut self.pending, rest);
            self.take(kind, whole);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The type and whole size of the box starting at `bytes`, once enough of it is there to say.
fn header(bytes: &[u8]) -> io::Result<Option<([u8; 4], usize)>> {
    let Some((size, kind)) = bytes
        .get(..8)
        .map(|h| (be32(&h[..4]), [h[4], h[5], h[6], h[7]]))
    else {
        return Ok(None);
    };
    let size = match size {
        // A 64-bit size follows the type.
        1 => match bytes.get(8..16) {
            Some(large) => {
                usize::try_from(u64::from_be_bytes(large.try_into().expect("eight bytes")))
                    .map_err(|_| invalid("a box larger than memory"))?
            }
            None => return Ok(None),
        },
        // "To the end of the file" has no end on a stream that never stops.
        0 => return Err(invalid("a box that runs to the end of the stream")),
        size => size as usize,
    };
    if size < 8 {
        return Err(invalid("a box shorter than its own header"));
    }
    Ok(Some((kind, size)))
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().expect("four bytes"))
}

/// The contents of a whole box, after its header.
fn body(bytes: &[u8]) -> &[u8] {
    let skip = if be32(bytes) == 1 { 16 } else { 8 };
    &bytes[skip.min(bytes.len())..]
}

/// The boxes directly inside `bytes`, as their type and their contents. Stops at anything that
/// does not parse, because a box read wrongly is worse than one not read.
fn children(mut bytes: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let (kind, size) = header(bytes).ok().flatten()?;
        let whole = bytes.get(..size)?;
        bytes = &bytes[size..];
        Some((kind, body(whole)))
    })
}

fn child<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(bytes).find(|(k, _)| k == kind).map(|(_, b)| b)
}

/// The video track in a `moov`: the `trak` whose handler is `vide`, and its `trex` defaults.
fn video_track(moov: &[u8]) -> Option<Track> {
    let id = children(moov)
        .filter(|(kind, _)| kind == b"trak")
        .find_map(|(_, trak)| {
            let handler = child(child(trak, b"mdia")?, b"hdlr")?;
            // Version and flags, then `pre_defined`, then the handler type.
            (handler.get(8..12)? == b"vide").then_some(())?;
            let header = child(trak, b"tkhd")?;
            // Version 1 carries 64-bit times before the track number.
            let at = if header.first()? == &1 { 20 } else { 12 };
            Some(be32(header.get(at..at + 4)?))
        })?;
    let default_flags = child(moov, b"mvex").and_then(|mvex| {
        children(mvex)
            .filter(|(kind, _)| kind == b"trex")
            // Version and flags, track, description index, duration, size, flags.
            .find(|(_, trex)| trex.get(4..8).map(be32) == Some(id))
            .and_then(|(_, trex)| trex.get(20..24).map(be32))
    });
    Some(Track { id, default_flags })
}

/// Whether the video track's first sample in a `moof` is a keyframe.
///
/// **The first sample's flags come from the most specific place that states them**: the `trun`'s
/// first-sample flags, then its per-sample flags, then the `tfhd` default, then the `trex` default
/// in the initialisation segment. A fragment with no video in it is not a place to start.
fn starts_on_keyframe(moof: &[u8], video: Track) -> bool {
    children(moof)
        .filter(|(kind, _)| kind == b"traf")
        .find_map(|(_, traf)| {
            let tfhd = child(traf, b"tfhd")?;
            let flags = be32(tfhd.get(..4)?) & 0x00FF_FFFF;
            (be32(tfhd.get(4..8)?) == video.id).then_some(())?;
            // Base data offset, description index, default duration, default size, then flags.
            let mut at = 8;
            for (bit, width) in [(0x01, 8), (0x02, 4), (0x08, 4), (0x10, 4)] {
                if flags & bit != 0 {
                    at += width;
                }
            }
            let fragment_default = (flags & 0x20 != 0)
                .then(|| tfhd.get(at..at + 4).map(be32))
                .flatten();
            let sample = child(traf, b"trun")
                .and_then(first_sample_flags)
                .or(fragment_default)
                .or(video.default_flags)?;
            Some(sample & NON_SYNC_SAMPLE == 0)
        })
        .unwrap_or(false)
}

/// The first sample's flags as a `trun` states them, if it does.
fn first_sample_flags(trun: &[u8]) -> Option<u32> {
    let flags = be32(trun.get(..4)?) & 0x00FF_FFFF;
    if be32(trun.get(4..8)?) == 0 {
        return None;
    }
    // Sample count, then the optional data offset.
    let mut at = 8 + if flags & 0x01 != 0 { 4 } else { 0 };
    if flags & 0x04 != 0 {
        return trun.get(at..at + 4).map(be32);
    }
    // Otherwise the first sample's own entry: duration and size come before its flags.
    if flags & 0x400 != 0 {
        for bit in [0x100, 0x200] {
            if flags & bit != 0 {
                at += 4;
            }
        }
        return trun.get(at..at + 4).map(be32);
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A box of `kind` holding `body`.
    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn full(version_and_flags: u32, rest: &[u8]) -> Vec<u8> {
        let mut out = version_and_flags.to_be_bytes().to_vec();
        out.extend_from_slice(rest);
        out
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// A `trak` numbered `id` whose handler is `handler`.
    fn trak(id: u32, handler: &[u8; 4]) -> Vec<u8> {
        // Version 0: creation, modification, then the track number.
        let tkhd = mp4_box(b"tkhd", &full(0, &words(&[0, 0, id, 0, 0])));
        let mut hdlr = full(0, &words(&[0]));
        hdlr.extend_from_slice(handler);
        let mdia = mp4_box(b"mdia", &mp4_box(b"hdlr", &hdlr));
        mp4_box(b"trak", &[tkhd, mdia].concat())
    }

    fn trex(id: u32, default_flags: u32) -> Vec<u8> {
        mp4_box(b"trex", &full(0, &words(&[id, 1, 0, 0, default_flags])))
    }

    /// An initialisation segment with sound on track 1 and picture on track 2, so nothing can pass
    /// by assuming the picture is first.
    fn init(video_default: u32) -> Vec<u8> {
        let moov = [
            trak(1, b"soun"),
            trak(2, b"vide"),
            mp4_box(
                b"mvex",
                &[trex(1, 0x0200_0000), trex(2, video_default)].concat(),
            ),
        ]
        .concat();
        [mp4_box(b"ftyp", b"isom"), mp4_box(b"moov", &moov)].concat()
    }

    /// A fragment for `track`, its flags stated where `how` says.
    fn fragment(track: u32, how: Flags) -> Vec<u8> {
        let (tfhd, trun) = match how {
            Flags::FirstSample(flags) => (
                full(0x20000, &words(&[track])),
                full(0x05, &words(&[1, 0, flags])),
            ),
            Flags::PerSample(flags) => (
                full(0x20000, &words(&[track])),
                full(0x401, &words(&[1, 0, flags])),
            ),
            Flags::FragmentDefault(flags) => (
                full(0x20020, &words(&[track, flags])),
                full(0x01, &words(&[1, 0])),
            ),
            Flags::TrackDefault => (full(0x20000, &words(&[track])), full(0x01, &words(&[1, 0]))),
        };
        let traf = mp4_box(
            b"traf",
            &[mp4_box(b"tfhd", &tfhd), mp4_box(b"trun", &trun)].concat(),
        );
        [mp4_box(b"moof", &traf), mp4_box(b"mdat", &[track as u8; 5])].concat()
    }

    #[derive(Clone, Copy)]
    enum Flags {
        FirstSample(u32),
        PerSample(u32),
        FragmentDefault(u32),
        TrackDefault,
    }

    const SYNC: u32 = 0x0200_0000;
    const NOT_SYNC: u32 = 0x0101_0000;

    /// Feeds `bytes` through a splitter in chunks of `chunk` bytes.
    fn split(bytes: &[u8], chunk: usize) -> Vec<Piece> {
        let pieces = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&pieces);
        let mut splitter = Splitter::new(Box::new(move |piece| {
            seen.lock().expect("unpoisoned").push(piece);
        }));
        for part in bytes.chunks(chunk) {
            assert_eq!(splitter.write(part).expect("valid boxes"), part.len());
        }
        pieces.lock().expect("unpoisoned").clone()
    }

    fn keys(pieces: &[Piece]) -> Vec<bool> {
        pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Fragment { key, .. } => Some(*key),
                Piece::Init(_) => None,
            })
            .collect()
    }

    #[test]
    fn boxes_are_whole_however_the_bytes_arrive() {
        let stream = [
            init(NOT_SYNC),
            fragment(2, Flags::FirstSample(SYNC)),
            fragment(1, Flags::TrackDefault),
            mp4_box(b"mfra", b"end"),
        ]
        .concat();
        let whole = split(&stream, stream.len());
        for chunk in [1, 3, 7, 64] {
            assert_eq!(split(&stream, chunk), whole, "chunks of {chunk}");
        }
        assert_eq!(whole.len(), 3, "an init and two fragments, and no mfra");
        let Piece::Init(first) = &whole[0] else {
            panic!("the initialisation segment comes first");
        };
        assert_eq!(&first[4..8], b"ftyp");
    }

    #[test]
    fn a_keyframe_is_read_from_wherever_the_fragment_states_it() {
        let stream = [
            init(NOT_SYNC),
            fragment(2, Flags::FirstSample(SYNC)),
            fragment(2, Flags::FirstSample(NOT_SYNC)),
            fragment(2, Flags::PerSample(SYNC)),
            fragment(2, Flags::FragmentDefault(SYNC)),
            fragment(2, Flags::FragmentDefault(NOT_SYNC)),
            fragment(2, Flags::TrackDefault),
        ]
        .concat();
        assert_eq!(
            keys(&split(&stream, 5)),
            [true, false, true, true, false, false]
        );
    }

    #[test]
    fn a_track_default_of_sync_makes_every_fragment_a_start() {
        let stream = [init(SYNC), fragment(2, Flags::TrackDefault)].concat();
        assert_eq!(keys(&split(&stream, 5)), [true]);
    }

    /// Sound is always a sync sample, and still never a place for a viewer to start.
    #[test]
    fn a_fragment_of_sound_alone_is_not_a_start() {
        let stream = [init(NOT_SYNC), fragment(1, Flags::FirstSample(SYNC))].concat();
        assert_eq!(keys(&split(&stream, 5)), [false]);
    }

    #[test]
    fn a_box_that_cannot_be_a_box_is_refused() {
        let mut splitter = Splitter::new(Box::new(|_| {}));
        let error = splitter
            .write(&[0, 0, 0, 4, b'm', b'o', b'o', b'f'])
            .expect_err("a size below the header's own");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
