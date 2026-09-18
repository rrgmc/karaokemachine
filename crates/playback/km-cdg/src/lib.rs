//! MP3+G songs — a CD+G graphics renderer and an MP3 feed, both in pure Rust.
//!
//! An MP3+G song is two files with the same stem: an `.mp3` carrying the backing track and a `.cdg`
//! carrying the words as graphics. It is the format most commercial karaoke discs actually hold, and
//! it is a song source here alongside MIDI and video — see the `MP3+G as a song source`
//! decision in `docs/decisions/`.
//!
//! # Why this crate has no cargo feature
//!
//! `km-video` has one because ffmpeg is a C dependency, and a `cargo build --workspace` that
//! compiled every member must not require it. Nothing here is optional in that sense: the renderer
//! is ours and the MP3 decoder is pure Rust, so **there is no build that can catalog an MP3+G song
//! and not play it**. That is the whole reason this route was taken over pre-rendering each pair to
//! an MP4 — which ffmpeg can do, and cheaply — because that would need the `video` feature and so
//! would exclude a `--no-video` build and Android, which is where this is meant to work.
//!
//! # Shape, and how it differs from video
//!
//! The audio half is the same as a video's: a thread decodes the file and pushes interleaved stereo
//! at the **file's own rate** into km-audio's lock-free audio feed, and `km-audio` resamples on
//! the way out. The position a song reports is samples the device has actually consumed, so audio is
//! the master clock here exactly as it is everywhere else.
//!
//! The graphics half is not like a video's at all, and the differences are all simplifications:
//!
//! * **A CD+G surface is cumulative.** Packets mutate a screen that persists; they cannot be
//!   skipped, only played through. That sounds like the harder problem and is not — see below.
//! * **There is no decoder thread and no frame queue.** The screen is advanced by whoever asks for a
//!   picture, inside [`FrameReader::take_frame_for`]. Nothing is produced unless somebody asks, so a
//!   headless run simply never advances it. `km-video` needs a bounded queue, a frame pool and a
//!   lookahead precisely because a full picture queue there would stall the demuxer that also
//!   carries the audio; there is no such hazard when nothing pushes.
//! * **A seek is a replay from packet zero.** The whole `.cdg` is held in memory — 2.6 MB for a
//!   six-minute song — and replaying every packet of one is under ten million byte writes, which is
//!   single-digit milliseconds. That measurement deletes keyframes, snapshots and a whole class of
//!   "which snapshot was that" bug, so there are none of them here.
//!
//! # What it assumes about its input
//!
//! Very little, deliberately. Real `.cdg` files off a real corpus are not clean: they retain the P
//! and Q subchannel bits in the top two bits of every byte, one measured file is not a whole number
//! of packets long, and one is three-quarters packs belonging to some other subcode application —
//! which it turns out is not damage at all, and that file plays perfectly. So every byte is masked
//! with `0x3F`, a trailing partial packet is dropped, and **anything unrecognized is counted and
//! skipped, never raised as an error**. That rule is load-bearing: a renderer that failed on a
//! stray packet would refuse files that play. [`GraphicsStats`] is how a packaging tool tells a
//! disc worth having from one with no words in it.

mod audio;
mod graphics;

pub use crate::audio::{
    AudioInfo, AudioReader, measure_loudness, measure_loudness_from, probe_audio, probe_audio_from,
};
pub use crate::graphics::{
    Applied, Frame, FrameReader, GraphicsStats, GraphicsStream, REWIND_SLACK_MS, Screen,
};

/// Bytes in one CD+G packet.
pub const PACKET_BYTES: usize = 24;

/// Packets in one second of a CD+G stream.
///
/// Fixed by the medium rather than by the file: a CD carries 75 sectors a second and each holds four
/// subcode packs. So a `.cdg`'s length is its size divided by 24 and then by this, and nothing in
/// the file states it.
pub const PACKETS_PER_SECOND: u32 = 300;

/// Width of the whole CD+G screen, border included.
pub const WIDTH: u32 = 300;

/// Height of the whole CD+G screen, border included.
pub const HEIGHT: u32 = 216;

/// Width of the area a television actually shows.
pub const VISIBLE_WIDTH: u32 = 288;

/// Height of the area a television actually shows.
pub const VISIBLE_HEIGHT: u32 = 192;

/// Left and right border, in pixels.
pub const BORDER_X: u32 = 6;

/// Top and bottom border, in pixels.
pub const BORDER_Y: u32 = 12;

/// The shape the picture must be presented at, as width to height.
///
/// **Not the same as its pixel dimensions, and that is the whole reason this constant exists.** The
/// visible area is 288x192, which is 3:2, but it was drawn for a 4:3 television — CD+G pixels are
/// not square. Presenting it at its pixel size, the way a video is presented at its own, would
/// stretch every word about 12% too wide. See the `A CD+G pixel is not square` decision in
/// `docs/decisions/song-sources.md`.
pub const DISPLAY_ASPECT: (u32, u32) = (4, 3);

/// Anything that can go wrong reading an MP3+G song.
#[derive(Debug, thiserror::Error)]
pub enum CdgError {
    /// A file could not be read.
    #[error("{path}: {source}")]
    Io {
        /// The file being read.
        path: String,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The graphics file holds no whole packet at all.
    #[error("{path}: not a CD+G stream — it holds no whole 24-byte packet")]
    NoPackets {
        /// The graphics file.
        path: String,
    },
    /// The audio file could not be opened as audio at all.
    #[error("{path}: not a readable audio file — {message}")]
    Undecodable {
        /// The audio file.
        path: String,
        /// What the decoder said.
        message: String,
    },
    /// The file opened, but holds no audio track this build can decode.
    #[error("{path}: holds no audio track that can be decoded")]
    NoAudioTrack {
        /// The audio file.
        path: String,
    },
}

impl CdgError {
    /// Wraps an I/O error with the path it happened to.
    /// Named by a `&str` rather than a `Path`, because since media moved into the `.kmpkg` the
    /// subject is as often an archive entry as a file, and `media/0012.mp3` is not a path on
    /// anybody's disk.
    pub(crate) fn io(name: &str, source: std::io::Error) -> Self {
        Self::Io {
            path: name.to_owned(),
            source,
        }
    }

    pub(crate) fn undecodable(name: &str, source: &dyn std::fmt::Display) -> Self {
        Self::Undecodable {
            path: name.to_owned(),
            message: source.to_string(),
        }
    }

    pub(crate) fn no_audio_track(name: &str) -> Self {
        Self::NoAudioTrack {
            path: name.to_owned(),
        }
    }
}

/// Everything a probe found about an MP3+G song: both files, in one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdgInfo {
    /// What the audio file holds, including the song's length.
    pub audio: AudioInfo,
    /// What replaying the whole graphics stream found.
    pub graphics: GraphicsStats,
}

impl CdgInfo {
    /// How far the graphics stop short of the audio, in milliseconds.
    ///
    /// Zero on most files — over 2,847 real pairs the mean gap is a tenth of a second — and a few
    /// seconds on the rest, because the words end before the outro does. **A gap of a minute or
    /// more is the signal that something is wrong with the pair**, and it is the one thing this
    /// number is good for: in the measured corpus exactly one file trips it, and that same file is
    /// the only one whose `.cdg` is not a whole number of packets. A truncated graphics file is
    /// precisely what it looks like.
    ///
    /// Saturating, because it also runs the other way — see [`Self::graphics_overrun_ms`].
    #[must_use]
    pub fn graphics_short_by_ms(&self) -> u32 {
        self.audio
            .duration_ms
            .saturating_sub(self.graphics.duration_ms)
    }

    /// How far the graphics stream runs *past* the end of the audio, in milliseconds.
    ///
    /// **This happens, which was a surprise and is worth stating plainly**: 34 of 2,847 measured
    /// pairs overrun, by up to 142 seconds. An early sample of twenty files showed none of it and
    /// the conclusion drawn — that a `.cdg` is never longer than its audio — was simply wrong.
    ///
    /// It is harmless. The tail is filler packets after the last tile is drawn, the song still ends
    /// with the sound, and the reader clamps to the packets it has. It is reported rather than
    /// acted on.
    #[must_use]
    pub fn graphics_overrun_ms(&self) -> u32 {
        self.graphics
            .duration_ms
            .saturating_sub(self.audio.duration_ms)
    }
}

/// Reads both halves of an MP3+G song without decoding the audio.
pub fn probe(audio: &std::path::Path, graphics: &std::path::Path) -> Result<CdgInfo, CdgError> {
    Ok(CdgInfo {
        audio: probe_audio(audio)?,
        graphics: read_graphics(graphics)?.stats(),
    })
}

/// The same, from bytes a caller already holds.
///
/// [`probe`] opens both files itself, which is right for a caller that has only names — but the
/// curation scan needs both halves in memory anyway, to hash them together into the song's identity
/// (`km_kmpkg::pair_content_hash`). Given only [`probe`] it read each file twice: once for the
/// probe, once for the hash. On a corpus of sixteen thousand pairs that is the whole corpus read a
/// second time for nothing.
///
/// The asymmetry between the two arguments is [`open_from`]'s and has the same reason: a `.cdg` is
/// held whole however it arrives, and an MP3 is a stream. `audio_name` and `graphics_name` are the
/// subjects of the error messages and the extension hints symphonia is given.
pub fn probe_from<R: std::io::Read + std::io::Seek + Send + Sync + 'static>(
    audio: R,
    audio_name: &str,
    graphics: &[u8],
    graphics_name: &str,
) -> Result<CdgInfo, CdgError> {
    Ok(CdgInfo {
        audio: crate::audio::probe_audio_from(audio, audio_name)?,
        graphics: graphics_from_bytes(graphics, graphics_name)?.stats(),
    })
}

/// Opens an MP3+G song: starts decoding the audio, and readies the graphics.
///
/// Returns the decoder to keep alive, the feed for `km-audio`, and the picture source for the
/// display. Dropping the [`AudioReader`] stops the decoder and **joins** it, so it must be dropped
/// on a thread where blocking is allowed and never on the audio callback.
pub fn open(
    audio: &std::path::Path,
    graphics: &std::path::Path,
) -> Result<(AudioReader, km_audio::AudioFeed, FrameReader), CdgError> {
    // Graphics first: it is the cheap half and the one that can fail on a missing file, and failing
    // before a thread exists is simpler than stopping one that already started.
    let stream = read_graphics(graphics)?;
    let (reader, feed) = crate::audio::open_audio(audio)?;
    Ok((reader, feed, FrameReader::new(stream)))
}

/// Opens an MP3+G song whose halves came out of a package rather than off the disk.
///
/// The audio arrives as something seekable and the graphics as bytes, which is the asymmetry the
/// crate documentation already argues for: a `.cdg` is 2.6 MB and is replayed from packet zero on
/// every seek, so it is held whole either way, and an MP3 is not.
///
/// Both names are the subjects of the error messages and the extensions symphonia is hinted with.
pub fn open_from<R: std::io::Read + std::io::Seek + Send + Sync + 'static>(
    audio: R,
    audio_name: &str,
    graphics: &[u8],
    graphics_name: &str,
) -> Result<(AudioReader, km_audio::AudioFeed, FrameReader), CdgError> {
    // Graphics first, for the reason above.
    let stream = graphics_from_bytes(graphics, graphics_name)?;
    let (format, track_id) = crate::audio::open_format_from(audio, audio_name)?;
    let (reader, feed) = crate::audio::open_audio_from(format, track_id, audio_name)?;
    Ok((reader, feed, FrameReader::new(stream)))
}

/// Starts decoding an MP3 with no graphics beside it, which is how an UltraStar song's audio plays.
///
/// **The audio half of [`open_from`], and nothing else.** An UltraStar song's words are a lyric
/// timeline the machine draws, so there is no picture source to ready; the decoder, the feed and the
/// clock the words follow are the ones an MP3+G song already uses.
pub fn open_audio_from<R: std::io::Read + std::io::Seek + Send + Sync + 'static>(
    audio: R,
    audio_name: &str,
) -> Result<(AudioReader, km_audio::AudioFeed), CdgError> {
    let (format, track_id) = crate::audio::open_format_from(audio, audio_name)?;
    crate::audio::open_audio_from(format, track_id, audio_name)
}

/// Reads a `.cdg` file and parses its packets.
///
/// The whole file, in one go: see the crate documentation for why holding it is right here and would
/// not be for a video.
pub fn read_graphics(path: &std::path::Path) -> Result<GraphicsStream, CdgError> {
    let name = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|source| CdgError::io(&name, source))?;
    graphics_from_bytes(&bytes, &name)
}

/// Parses a `.cdg` already in memory, which is how one comes out of a package.
///
/// `name` is only for the error message.
pub fn graphics_from_bytes(bytes: &[u8], name: &str) -> Result<GraphicsStream, CdgError> {
    let stream = GraphicsStream::from_bytes(bytes);
    if stream.packets() == 0 {
        return Err(CdgError::NoPackets {
            path: name.to_owned(),
        });
    }
    Ok(stream)
}
