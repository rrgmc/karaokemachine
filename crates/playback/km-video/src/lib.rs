//! Decoding video songs — the only crate in the workspace that decodes with ffmpeg.
//!
//! This exists as a crate of its own for the same reason `km-audio` isolates `rustysynth`: exactly
//! one `Cargo.toml` mentions the dependency, so every other crate goes on showing its real
//! footprint, and the C dependency can be turned off with one feature.
//!
//! That feature is this crate's own `ffmpeg`, and it is **off by default**. Being a workspace member
//! means `cargo build --workspace` compiles this crate whether or not anything wants video, so
//! without the switch ffmpeg and libclang would be needed to build the workspace at all. With it off
//! the crate is empty; nothing has to ask for it by name, because the root `Cargo.toml` puts
//! `features = ["ffmpeg"]` on the workspace dependency entry that every consumer inherits.
//!
//! # Shape
//!
//! One thread demuxes a file and decodes both of its streams, feeding two very different consumers:
//!
//! * **Audio** goes into a [`km_audio::AudioFeedWriter`] — a lock-free ring the audio callback
//!   reads from. Nothing here knows the output device's sample rate; samples are pushed at the
//!   file's own rate and `km-audio` resamples them on the way out.
//! * **Video** goes into a bounded channel of [`Frame`]s, which the display thread takes from once
//!   per drawn frame and uploads to a texture.
//!
//! The audio is what carries time. A video song's position comes from samples the device has
//! actually consumed, exactly as a MIDI song's comes from the sequencer, and the display picks
//! whichever decoded frame is nearest that position. That is audio-as-master-clock, and it falls out
//! of the machine's existing design rather than being imported alongside it.
//!
//! # Frames are recycled, never reallocated
//!
//! A 1080p YUV420 frame is 3.1 MB. Allocating one thirty times a second, and freeing it on whichever
//! thread happens to drop it, is exactly the kind of churn that shows up later as a stutter nobody
//! can place. Frames therefore come from a small pool: the display hands each one back with
//! [`FrameReader::recycle`] after uploading it, and the decoder refills it in place.
//!
//! # What it assumes about its input
//!
//! That the file was normalized at packaging time — H.264 in yuv420p, constant frame rate, short
//! keyframe intervals. The appliance is a small Intel box and this is what keeps its job easy:
//! `yuv420p` uploads straight to an SDL `IYUV` texture with no color conversion, and a short
//! keyframe interval is what keeps a seek near where it was asked for.
//!
//! **One of those is a requirement rather than a preference.** [`Frame::fill_from`] copies three
//! planes with the chroma at half height — 8-bit planar 4:2:0 and nothing else — and this crate
//! contains no `swscale` and no conversion of any kind. Handed a `yuv444p` file it would take half
//! the chroma rows the picture has; handed a 10-bit one it would read a byte per sample. Both draw a
//! wrong picture rather than failing, which is the hardest kind of fault to place. So [`open`]
//! refuses anything [`supports_pixel_format`] does not know, and names the format it found.
//!
//! [`probe`] deliberately does **not** refuse. Packaging has to read a file's shape in order to
//! decide that it must be re-encoded, and a probe that failed on precisely the files needing a
//! transcode would make that decision unreachable.

// Everything below is ffmpeg, so the whole crate goes rather than each item carrying its own `cfg`.
// Without the feature this compiles to an empty library in about a second, which is exactly what a
// workspace build that never asked for video should pay.
#![cfg(feature = "ffmpeg")]

use std::io::{Read, Seek};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use ffmpeg_next as ff;
use km_audio::{AudioFeed, AudioFeedWriter, FEED_CHANNELS, audio_feed};

/// Planes in a YUV420 frame: luma and two chroma.
const PLANES: usize = 3;

/// The widest row this crate will copy out of a decoded picture, in bytes.
///
/// **Not a limit on picture size — a sign check that `usize` has thrown away.** `Video::stride` is
/// ffmpeg's `linesize` cast to `usize`, and ffmpeg spells a bottom-upwards picture with a *negative*
/// linesize, which arrives here as a number near `usize::MAX`. Anything above this is that, or a
/// field this crate has no way to read correctly either way.
///
/// A row of 8K video is 7,680 bytes before padding, so a mebibyte is three orders of magnitude of
/// headroom.
const MAX_STRIDE: usize = 1024 * 1024;

/// How far ahead of the picture on screen the decoder is allowed to work.
///
/// **This one number sizes both buffers.** One thread demuxes both streams in timestamp order, so by
/// the time it has read a quarter-second of audio it has also read a quarter-second of video. Giving
/// the two queues the same amount of *time* means audio is what fills first and therefore what paces
/// the demuxer — which is the right way round, because audio is the clock.
///
/// It is a tuning choice, not the safety net. What actually guarantees the audio keeps flowing is
/// that a full picture queue never stalls the demuxer: the picture is dropped instead. That rule is
/// what makes a headless run work at all, where nothing is drawing and no picture is ever taken —
/// and before it existed, this froze: the queue filled, the demuxer stopped, the ring drained, and
/// the position stuck where it stood.
///
/// Matching the two sizes is what keeps that drop path from firing during ordinary playback.
///
/// A quarter of a second is far more than the decoder needs — it runs at many times real time — and
/// small enough that a seek is not waiting on it.
const LOOKAHEAD_MS: u32 = 250;

/// Frames to hold, whatever the frame rate says, so a still image or a broken rate still buffers.
const MIN_FRAME_QUEUE: usize = 4;

/// How long the decoder rests when its outputs are full.
///
/// It is not idle work — the thread has decoded audio in hand and nowhere to put it — so this is a
/// backpressure wait, not a poll interval.
const BACKPRESSURE_NAP: std::time::Duration = std::time::Duration::from_millis(4);

/// Sizes the picture queue so it holds at least as much *time* as the audio ring does.
///
/// The two extra frames are slack: the queue must not become the binding constraint through
/// rounding, because the invariant in [`LOOKAHEAD_MS`] only holds while audio fills first.
fn frame_queue_len(frame_rate_milli: u32) -> usize {
    let frames = (u64::from(frame_rate_milli) * u64::from(LOOKAHEAD_MS)) / 1_000_000;
    (usize::try_from(frames).unwrap_or(MIN_FRAME_QUEUE) + 2).max(MIN_FRAME_QUEUE)
}

/// Why a video file could not be opened or decoded.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// The file could not be opened, or is not a container ffmpeg understands.
    #[error("could not open video {path}: {source}")]
    Open {
        /// The file that failed.
        path: String,
        /// What ffmpeg said.
        source: ff::Error,
    },
    /// The file has no video stream, so it is not a video song.
    #[error("{path} has no video stream")]
    NoVideoStream {
        /// The file that failed.
        path: String,
    },
    /// The file has no audio stream, so there would be nothing to sing over.
    #[error("{path} has no audio stream")]
    NoAudioStream {
        /// The file that failed.
        path: String,
    },
    /// The picture is not 8-bit planar 4:2:0, which is the only layout this crate can copy.
    ///
    /// Refused rather than drawn: see the module documentation. Packaging's answer is to re-encode
    /// the file, which is what the transcode profile is for.
    #[error("{path} is {format}, and only 8-bit planar 4:2:0 video can be played")]
    UnsupportedPixelFormat {
        /// The file that failed.
        path: String,
        /// The pixel format it turned out to be, spelled as ffmpeg spells it.
        format: String,
    },
    /// A decoder could not be built for one of the streams.
    #[error("could not decode {path}: {source}")]
    Decoder {
        /// The file that failed.
        path: String,
        /// What ffmpeg said.
        source: ff::Error,
    },
}

/// What a video file says about itself, read without decoding it.
///
/// This is what packaging records, so the machine reads a fact rather than opening a file to find
/// out how long a song is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInfo {
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Frames per second, times 1000, so the common 29.97 survives being written down.
    pub frame_rate_milli: u32,
    /// The audio stream's sample rate.
    pub audio_sample_rate: u32,
    /// The video codec, spelled as ffmpeg spells it: `h264`, `vp9`, `av1`.
    ///
    /// A string rather than ffmpeg's own enum, deliberately. This struct is what packaging reads,
    /// and keeping ffmpeg types out of it is what lets exactly one `Cargo.toml` in the workspace
    /// name the dependency — the reason this crate exists at all.
    pub video_codec: String,
    /// The audio codec, spelled the same way: `aac`, `opus`, `mp3`.
    pub audio_codec: String,
    /// How the picture is laid out: `yuv420p`, `yuv444p`, `yuv420p10le`.
    ///
    /// The one field here that decides whether the file can be *played* rather than merely how well
    /// it is suited — see [`supports_pixel_format`].
    pub pixel_format: String,
    /// Audio channels in the file, before conversion.
    ///
    /// Reported for packaging's benefit rather than the player's: the decoder runs a `swresample`
    /// context that converts whatever the file has to interleaved stereo `f32`, so mono, 5.1 and
    /// anything else all play. Contrast the pixel format, which nothing converts.
    pub audio_channels: u16,
    /// The container's own title tag, if it carries one.
    ///
    /// Read here rather than guessed from the file name, because a downloader that knows the song's
    /// real title can write it down and a file name cannot always carry it — a title with a `/` or a
    /// `:` in it survives in a tag and does not survive in a path. The file stem remains the
    /// fallback everywhere this is `None`, which is most of the corpus.
    pub title: Option<String>,
    /// The container's own artist tag, if it carries one.
    ///
    /// The one fact a video song has never had. A MIDI file's artist comes out of its lyrics header;
    /// a video's has to come from whoever downloaded it, and this is where they put it.
    pub artist: Option<String>,
}

/// Whether a picture in this format can be copied by [`Frame::fill_from`] and drawn.
///
/// True for exactly the 8-bit planar 4:2:0 layouts. `yuvj420p` is the deprecated full-range
/// spelling of `yuv420p` — the same three planes at the same sizes, differing only in how the
/// values are interpreted, which the GPU's conversion handles — so refusing it would reject a large
/// number of perfectly ordinary files for no reason this crate can act on.
///
/// Public because packaging asks the same question in order to decide what to re-encode, and the
/// answer must not be written down in two places that can drift apart.
pub fn supports_pixel_format(format: &str) -> bool {
    matches!(format, "yuv420p" | "yuvj420p")
}

/// Initializes ffmpeg once per process.
///
/// `ffmpeg_next::init` is idempotent and cheap after the first call; this wrapper exists so callers
/// do not have to know that.
fn init() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Failing here means the libraries are unusable, which the next call will report far more
        // specifically than we could.
        let _ = ff::init();
        // ffmpeg's own logging is chatty and goes to stderr behind our back. Warnings and worse
        // only; a corrupt frame is worth knowing about, and the rest is not.
        ff::util::log::set_level(ff::util::log::Level::Warning);
    });
}

/// Opens a container from anything seekable.
///
/// **This is the whole of what a package's media costs the decoder.** `ffmpeg-next` 9.0 wraps
/// `avio_alloc_context` in `StreamIo::from_read_seek`, so a window into a `.kmpkg` reaches ffmpeg
/// through safe, public API and this crate writes no `unsafe`. It also means no ffmpeg rebuild
/// anywhere — the alternative, ffmpeg's own `subfile:` protocol, is missing from the Android build,
/// which is configured `--disable-everything --enable-protocol=file`.
///
/// `name` is the format hint. Custom I/O gives ffmpeg no filename to look at, so a name that carries
/// the right extension is what lets it recognize the container.
fn stream_input<R: Read + Seek + Send + 'static>(
    reader: R,
    name: &str,
) -> Result<ff::format::context::Input, VideoError> {
    init();
    let io = ff::format::context::StreamIo::from_read_seek(reader).map_err(|source| {
        VideoError::Open {
            path: name.to_owned(),
            source,
        }
    })?;
    ff::format::input_from_stream(io, Some(name), None).map_err(|source| VideoError::Open {
        path: name.to_owned(),
        source,
    })
}

/// Opens a container from a path.
fn path_input(path: &Path) -> Result<(ff::format::context::Input, String), VideoError> {
    init();
    let name = path.display().to_string();
    let input = ff::format::input(path).map_err(|source| VideoError::Open {
        path: name.clone(),
        source,
    })?;
    Ok((input, name))
}

/// Reads a video file's shape without decoding any of it.
pub fn probe(path: &Path) -> Result<VideoInfo, VideoError> {
    let (input, name) = path_input(path)?;
    info_of(&input, &name)
}

/// The same, from anything seekable — a file, or a window into a package.
///
/// `name` is both the subject of the error messages and ffmpeg's format hint; pass something with
/// the right extension on it, such as `media/0007.mp4`.
pub fn probe_from<R: Read + Seek + Send + 'static>(
    reader: R,
    name: &str,
) -> Result<VideoInfo, VideoError> {
    let input = stream_input(reader, name)?;
    info_of(&input, name)
}

/// Measures how loud a video song's audio is, for a package being built.
///
/// See [`measure_loudness_from`] for everything that matters about it; this is the same over a path.
pub fn measure_loudness(path: &Path) -> Result<Option<km_loudness::Loudness>, VideoError> {
    let (input, name) = path_input(path)?;
    loudness_of(input, &name)
}

/// The same, from anything seekable — a file, or a window into a package.
///
/// `name` is both the subject of the error messages and ffmpeg's format hint; pass something with
/// the right extension on it, such as `media/0007.mp4`.
pub fn measure_loudness_from<R: Read + Seek + Send + 'static>(
    reader: R,
    name: &str,
) -> Result<Option<km_loudness::Loudness>, VideoError> {
    let input = stream_input(reader, name)?;
    loudness_of(input, name)
}

/// Decodes a container's audio stream, and only that, into a loudness meter.
///
/// **The video stream is never decoded.** Its packets are recognised by index and dropped unread, so
/// this costs an audio decode and a demux rather than a play-through — which is most of the work of
/// a video gone, and why measuring a four-minute song is seconds rather than minutes.
///
/// **Deliberately not through a `TrackPlayer`, and [`probe`]'s promise is left intact.** The live
/// path decodes on a thread into a ring a real-time callback drains; driving that faster than real
/// time makes the player answer an empty feed with silence and a frozen position, which measures
/// gaps rather than a song (`Judging it by ear, and one trap in doing so` in
/// `docs/architecture/cdg.md`). And `probe` still reads the shape without decoding anything — this
/// is a separate call, made only where a level is wanted.
///
/// What reaches the meter is what the machine plays: the same `swresample` conversion to interleaved
/// stereo `f32` that [`run_decode`] sets up, at the file's own rate.
///
/// `Ok(None)` is a file with less audio in it than R128 integrates over, or one that is silent —
/// not an error. The caller writes no measurement and the song plays at gain 1.0, as it does today.
fn loudness_of(
    mut input: ff::format::context::Input,
    name: &str,
) -> Result<Option<km_loudness::Loudness>, VideoError> {
    let decoder_error = |source: ff::Error| VideoError::Decoder {
        path: name.to_owned(),
        source,
    };

    let audio_index = input
        .streams()
        .best(ff::media::Type::Audio)
        .map(|stream| stream.index())
        .ok_or_else(|| VideoError::NoAudioStream {
            path: name.to_owned(),
        })?;
    let audio_time_base = input
        .stream(audio_index)
        .ok_or_else(|| decoder_error(ff::Error::StreamNotFound))?
        .time_base();

    let mut audio = ff::codec::context::Context::from_parameters(
        input
            .stream(audio_index)
            .ok_or_else(|| decoder_error(ff::Error::StreamNotFound))?
            .parameters(),
    )
    .map_err(decoder_error)?
    .decoder()
    .audio()
    .map_err(decoder_error)?;

    let rate = audio.rate();
    let mut resampler = ff::software::resampling::Context::get(
        audio.format(),
        audio.channel_layout(),
        rate,
        ff::format::Sample::F32(ff::format::sample::Type::Packed),
        ff::channel_layout::ChannelLayout::STEREO,
        rate,
    )
    .map_err(decoder_error)?;

    let Some(mut meter) = km_loudness::Meter::stereo(rate) else {
        return Ok(None);
    };

    let mut samples: Vec<f32> = Vec::new();
    // `None` is what `drain_audio` wants when there is no seek to discard past, and there never is
    // one here: this reads the file once, start to end.
    let mut discard_before_ms: Option<u32> = None;

    for (stream, packet) in input.packets() {
        if stream.index() != audio_index {
            continue;
        }
        if audio.send_packet(&packet).is_err() {
            // A packet the decoder will not take is skipped rather than fatal, the same judgment
            // `km-cdg` makes: one damaged frame otherwise costs the measurement of the whole song.
            continue;
        }
        samples.clear();
        drain_audio(
            &mut audio,
            &mut resampler,
            &mut samples,
            audio_time_base,
            &mut discard_before_ms,
        );
        meter.add(&samples);
    }

    // Flushed, because the decoder holds frames back. Without this the tail of every song is missing
    // from the measurement -- which is small on a four-minute song and is not nothing.
    if audio.send_eof().is_ok() {
        samples.clear();
        drain_audio(
            &mut audio,
            &mut resampler,
            &mut samples,
            audio_time_base,
            &mut discard_before_ms,
        );
        meter.add(&samples);
    }

    Ok(meter.finish())
}

/// Reads the shape off an already-open container.
///
/// Split out so that opening and measuring happen once. A `probe` that opens the file, reads this
/// and drops it leaves the decoder thread opening the same file again. Nothing here consumes
/// packets, so the input can be measured and then handed straight to the thread.
fn info_of(input: &ff::format::context::Input, name: &str) -> Result<VideoInfo, VideoError> {
    let video = input
        .streams()
        .best(ff::media::Type::Video)
        .ok_or_else(|| VideoError::NoVideoStream {
            path: name.to_owned(),
        })?;
    let audio = input
        .streams()
        .best(ff::media::Type::Audio)
        .ok_or_else(|| VideoError::NoAudioStream {
            path: name.to_owned(),
        })?;

    let decoder = ff::codec::context::Context::from_parameters(video.parameters())
        .map(ff::codec::context::Context::decoder)
        .and_then(ff::codec::decoder::Decoder::video)
        .map_err(|source| VideoError::Decoder {
            path: name.to_owned(),
            source,
        })?;
    let audio_decoder = ff::codec::context::Context::from_parameters(audio.parameters())
        .map(ff::codec::context::Context::decoder)
        .and_then(ff::codec::decoder::Decoder::audio)
        .map_err(|source| VideoError::Decoder {
            path: name.to_owned(),
            source,
        })?;

    let rate = video.avg_frame_rate();
    let frame_rate_milli = if rate.denominator() > 0 {
        u32::try_from(i64::from(rate.numerator()) * 1000 / i64::from(rate.denominator()))
            .unwrap_or(0)
    } else {
        0
    };

    // `Input::duration` is in AV_TIME_BASE units, which is microseconds.
    let duration_ms = u32::try_from(input.duration().max(0) / 1_000).unwrap_or(u32::MAX);

    // ffmpeg normalizes each container's own spelling into these keys, which is the whole reason to
    // ask it rather than to parse atoms here: MP4's `©nam`/`©ART`, Matroska's `TITLE`/`ARTIST` and
    // an ID3 tag all arrive as `title` and `artist`. `album_artist` and `author` are consulted after
    // `artist` because a muxer that had only one of the three writes whichever it prefers.
    let tags = input.metadata();
    let title = tag(&tags, &["title"]);
    let artist = tag(&tags, &["artist", "album_artist", "author"]);

    Ok(VideoInfo {
        duration_ms,
        width: decoder.width(),
        height: decoder.height(),
        frame_rate_milli,
        audio_sample_rate: audio_decoder.rate(),
        video_codec: codec_name(video.parameters().id()),
        audio_codec: codec_name(audio.parameters().id()),
        pixel_format: pixel_name(decoder.format()),
        audio_channels: audio_decoder.channels(),
        title,
        artist,
    })
}

/// The first of `keys` the container actually carries, trimmed, ignoring blanks.
///
/// Blank rather than absent is the common case and not a hypothetical: a muxer asked to embed
/// metadata it does not have writes the key with an empty value, and `Some("")` propagated onwards
/// would become a song titled nothing at all — which is precisely the shape `km-song` already had to
/// grow a guard against for MIDI titles made of padding.
fn tag(tags: &ff::util::dictionary::Ref<'_>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| tags.get(key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Spells a codec the way ffmpeg's own tools do.
///
/// Taken from the enum's `Debug`, lowercased. That is not a shortcut: `ffmpeg_next` models codec ids
/// as a Rust enum whose variants are named after ffmpeg's own identifiers, so `Id::H264` is `h264`
/// and `Id::VP9` is `vp9`, which is exactly what `ffprobe -show_entries stream=codec_name` prints.
/// Anything unrecognized still comes out as a readable name rather than a number, which matters
/// because these strings are shown to a person deciding whether to re-encode a file.
fn codec_name(id: ff::codec::Id) -> String {
    format!("{id:?}").to_lowercase()
}

/// Spells a pixel format the same way, for the same reason.
fn pixel_name(format: ff::format::Pixel) -> String {
    format!("{format:?}").to_lowercase()
}

/// One decoded picture, in the planar YUV the decoder produced it in.
///
/// Deliberately not converted to RGB: SDL uploads these three planes to an `IYUV` texture and the
/// GPU does the color conversion for free. Converting here would cost a full-frame pass on the CPU
/// to arrive somewhere worse.
#[derive(Debug)]
pub struct Frame {
    /// Presentation time in milliseconds from the start of the file.
    pub pts_ms: u32,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Y, U and V planes, each packed to its own stride.
    planes: [Vec<u8>; PLANES],
    /// Bytes per row in each plane.
    strides: [usize; PLANES],
}

impl Frame {
    /// The luma plane and its stride.
    #[must_use]
    pub fn y(&self) -> (&[u8], usize) {
        (&self.planes[0], self.strides[0])
    }

    /// The U chroma plane and its stride.
    #[must_use]
    pub fn u(&self) -> (&[u8], usize) {
        (&self.planes[1], self.strides[1])
    }

    /// The V chroma plane and its stride.
    #[must_use]
    pub fn v(&self) -> (&[u8], usize) {
        (&self.planes[2], self.strides[2])
    }

    /// An empty frame, for seeding the pool.
    fn empty() -> Self {
        Self {
            pts_ms: 0,
            width: 0,
            height: 0,
            planes: [Vec::new(), Vec::new(), Vec::new()],
            strides: [0; PLANES],
        }
    }

    /// Refills this frame from a decoded one, reusing its buffers.
    ///
    /// **The decoded frame is asked what it is, rather than the container.** [`open`] refuses a
    /// format [`supports_pixel_format`] does not know, but it reads that from the stream's
    /// `AVCodecParameters` — written by whoever made the file, and settled *before a single frame
    /// has been decoded*. A bitstream may disagree with them: an H.264 sequence header declaring no
    /// chroma decodes to a one-plane picture inside a stream whose parameters say `yuv420p`.
    ///
    /// What that costs is why this returns something rather than trusting the check upstream.
    /// `ffmpeg_next`'s `Video::data` **panics** when asked for a plane the frame does not have, and
    /// the panic unwinds the decoder thread past `writer.finish()` — so the audio feed never reaches
    /// its end and the machine sits on a song that cannot finish. A refusal ends the song; a panic
    /// hangs it.
    ///
    /// **The stride is read before the data**, which is the order that matters. `stride` is
    /// `linesize` cast to `usize`, and ffmpeg uses a *negative* linesize for a picture stored bottom
    /// upwards — which arrives here as a number near `usize::MAX` and would make `data` build a
    /// slice of that length out of a raw pointer. Reading the length first costs nothing and is the
    /// only point at which this crate can see it coming.
    fn fill_from(&mut self, decoded: &ff::frame::Video, pts_ms: u32) -> bool {
        if decoded.planes() < PLANES || !supports_pixel_format(&pixel_name(decoded.format())) {
            return false;
        }

        let height = decoded.height() as usize;
        for plane in 0..PLANES {
            let stride = decoded.stride(plane);
            if stride == 0 || stride > MAX_STRIDE {
                return false;
            }
            let rows = if plane == 0 {
                height
            } else {
                // 4:2:0 chroma is half height, rounded up for odd sizes.
                height.div_ceil(2)
            };
            let Some(wanted) = stride.checked_mul(rows) else {
                return false;
            };
            let source = decoded.data(plane);
            let buffer = &mut self.planes[plane];
            buffer.clear();
            buffer.extend_from_slice(&source[..wanted.min(source.len())]);
            self.strides[plane] = stride;
        }

        self.pts_ms = pts_ms;
        self.width = decoded.width();
        self.height = decoded.height();
        true
    }
}

/// Pictures this song threw away, and where.
///
/// Both of these were already happening and neither said so. Dropping video under pressure is the
/// correct behavior and is argued at each site — the point of counting is that "correct" and
/// "happening" are different claims, and only the second one explains a hesitation somebody saw.
///
/// Written by the decoder thread and the display thread, one each, and read by neither: the machine
/// reads them when the song ends. `Relaxed` throughout for that reason — nothing is ordered against
/// these, and a count that is one behind is a count that is right a millisecond later.
///
/// **A headless run fills `dropped` at the video's full frame rate** and that is not a fault: with
/// nothing drawing, the picture queue is full for ever and every frame decoded is a frame nobody
/// wanted. Anything reporting these has to know whether a display was attached.
#[derive(Debug, Default)]
pub struct VideoCounters {
    dropped: AtomicU32,
    skipped: AtomicU32,
}

impl VideoCounters {
    /// Pictures the decoder could not hand over because the queue was full.
    #[must_use]
    pub fn dropped(&self) -> u32 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Pictures that reached the display already too old to draw.
    ///
    /// **Not a fault, and reading it as one is a mistake this made first.** The obvious reading is
    /// that a picture only comes due late if the display was late — but the position it is compared
    /// against advances in steps of one audio callback, and on the appliance that is 117 ms. Three
    /// and a half frames of a 30 fps video come due at every step, so this counts about half a
    /// picture a second on a song with nothing wrong with it, and measured runs put it at 0.44 to
    /// 0.65 a second while the audible fault ranged from 871 ms to zero.
    ///
    /// What it is good for is *comparison*: a rate far above that baseline, for a known audio
    /// period and frame rate, is the display genuinely failing to keep up. The absolute number says
    /// nothing.
    #[must_use]
    pub fn skipped(&self) -> u32 {
        self.skipped.load(Ordering::Relaxed)
    }
}

/// Takes decoded pictures, newest-first, and hands their buffers back.
///
/// Lives on the display thread. Every method is non-blocking: a drawn frame must never wait for a
/// decoder.
#[derive(Debug)]
pub struct FrameReader {
    /// The receiver, and a frame taken from it before its time.
    ///
    /// Behind a mutex because this is reachable from the machine, which is shared across the
    /// control, API and display threads and must be `Sync` — a bare `Receiver` is not. In practice
    /// only the display thread ever takes it, once per drawn frame, and it is never held across
    /// anything slow, so there is nothing here to contend for.
    ///
    /// The held frame exists because a channel cannot be un-read and the decoder runs ahead on
    /// purpose: the first picture whose time has not come has to wait somewhere.
    inner: std::sync::Mutex<Pending>,
    recycle: SyncSender<Frame>,
    counters: Arc<VideoCounters>,
}

#[derive(Debug)]
struct Pending {
    frames: Receiver<Frame>,
    held: Option<Frame>,
}

impl FrameReader {
    /// Takes the newest frame that should already be on screen at `position_ms`.
    ///
    /// Frames older than that are skipped and recycled rather than drawn: if the display has fallen
    /// behind, showing every frame it missed in a rush would be worse than showing the right one
    /// now. Returns `None` when there is nothing new, which is the ordinary case — the display draws
    /// faster than the video's frame rate and simply keeps the picture it has.
    pub fn take_frame_for(&self, position_ms: u32) -> Option<Frame> {
        let mut best: Option<Frame> = None;
        // A poisoned lock here means a previous caller panicked mid-frame; recovering keeps the
        // picture moving rather than taking the whole display down with it.
        let mut pending = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        loop {
            let frame = match pending.held.take() {
                Some(frame) => frame,
                None => match pending.frames.try_recv() {
                    Ok(frame) => frame,
                    Err(_) => break,
                },
            };

            if frame.pts_ms > position_ms {
                // Not due. Keep it for a later call rather than showing it early or losing it.
                pending.held = Some(frame);
                break;
            }

            if let Some(previous) = best.replace(frame) {
                // Only counted here, in the branch that replaces one due picture with a newer due
                // one. The loop reaches this a second time only when two frames came due between
                // one drawn frame and the next, which is the display being late by definition.
                self.counters.skipped.fetch_add(1, Ordering::Relaxed);
                let _ = self.recycle.try_send(previous);
            }
        }
        best
    }

    /// What this song threw away, and where. See [`VideoCounters`].
    #[must_use]
    pub fn counters(&self) -> &Arc<VideoCounters> {
        &self.counters
    }

    /// Hands a frame's buffers back to the decoder.
    ///
    /// Dropping one instead is safe and merely wasteful — the pool refills by allocating.
    pub fn recycle(&self, frame: Frame) {
        let _ = self.recycle.try_send(frame);
    }
}

/// A running decoder: one thread, reading one file.
///
/// Dropping this stops the thread and waits for it, which is why it must not be dropped on the audio
/// callback. `km-audio`'s retirement queue exists for exactly that.
#[derive(Debug)]
pub struct MediaReader {
    name: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MediaReader {
    /// What is being decoded: a file path, or an entry inside a package.
    ///
    /// A name rather than a path, because since media moved into the `.kmpkg` there is not always a
    /// path to give — `media/0007.mp4` is a real answer and `PathBuf` would make it a lie.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for MediaReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens a video file and starts decoding it.
///
/// Returns the shape as well as the three handles, because the container is opened **once** and
/// measured in place. Opening it, measuring, dropping it and opening it again on the decoder thread
/// is the alternative; `ffmpeg_next::format::context::Input` is `Send`, so the measured one can
/// simply be moved to the thread instead.
///
/// The feed and the reader keep working until the [`MediaReader`] is dropped, and dropping either of
/// them tells the decoder to stop.
pub fn open(path: &Path) -> Result<(VideoInfo, MediaReader, AudioFeed, FrameReader), VideoError> {
    let (input, name) = path_input(path)?;
    start(input, &name)
}

/// The same, from anything seekable — a file, or a window into a package.
///
/// `name` is both the subject of the error messages and ffmpeg's format hint; with custom I/O there
/// is no filename for it to probe from, so pass something carrying the right extension.
pub fn open_from<R: Read + Seek + Send + 'static>(
    reader: R,
    name: &str,
) -> Result<(VideoInfo, MediaReader, AudioFeed, FrameReader), VideoError> {
    let input = stream_input(reader, name)?;
    start(input, name)
}

/// Measures an open container and starts a decoder thread on it.
fn start(
    input: ff::format::context::Input,
    name: &str,
) -> Result<(VideoInfo, MediaReader, AudioFeed, FrameReader), VideoError> {
    // Measured first, so a file that is not a video song is an error the caller can report rather
    // than a thread that starts and immediately dies where nobody is looking. It also answers the
    // two questions the buffers are sized from.
    let info = info_of(&input, name)?;

    // Refused here rather than in `probe`, which packaging needs to stay permissive so it can read
    // the shape of a file in order to decide it must be re-encoded. This is the last point before a
    // decoder thread starts filling frames nobody can draw correctly.
    if !supports_pixel_format(&info.pixel_format) {
        return Err(VideoError::UnsupportedPixelFormat {
            path: name.to_owned(),
            format: info.pixel_format,
        });
    }

    // Both sized from one lookahead, for the reason spelled out on `LOOKAHEAD_MS`: a picture queue
    // holding less *time* than the audio ring deadlocks the pair of them.
    let audio_rate = info.audio_sample_rate.max(1);
    let ring_frames = (audio_rate as usize).saturating_mul(LOOKAHEAD_MS as usize) / 1_000;
    let queue = frame_queue_len(info.frame_rate_milli);

    let (writer, feed) = audio_feed(audio_rate, ring_frames.max(1));
    let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel(queue);
    let (recycle_tx, recycle_rx) = std::sync::mpsc::sync_channel(queue + 2);
    for _ in 0..queue {
        let _ = recycle_tx.try_send(Frame::empty());
    }

    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(VideoCounters::default());
    let thread = {
        let stop = Arc::clone(&stop);
        let counters = Arc::clone(&counters);
        let owned = name.to_owned();
        std::thread::Builder::new()
            .name("km-video-decode".to_owned())
            .spawn(move || {
                decode_loop(
                    input, &owned, writer, frame_tx, recycle_rx, &stop, &counters,
                );
            })
            .map_err(|error| VideoError::Open {
                path: name.to_owned(),
                source: ff::Error::Other {
                    errno: error.raw_os_error().unwrap_or(0),
                },
            })?
    };

    Ok((
        info.clone(),
        MediaReader {
            name: name.to_owned(),
            stop,
            thread: Some(thread),
        },
        feed,
        FrameReader {
            inner: std::sync::Mutex::new(Pending {
                frames: frame_rx,
                held: None,
            }),
            recycle: recycle_tx,
            counters,
        },
    ))
}

/// The decoder thread's whole life.
///
/// Any error ends decoding rather than being retried: a file that has stopped being readable
/// mid-song is not going to start again, and the feed's end-of-file is what tells the machine to
/// move on to the next song. That is the same path a song reaching its natural end takes, which is
/// why a truncated file behaves like a short song rather than like a crash.
fn decode_loop(
    input: ff::format::context::Input,
    name: &str,
    mut writer: AudioFeedWriter,
    frames: SyncSender<Frame>,
    recycle: Receiver<Frame>,
    stop: &AtomicBool,
    counters: &VideoCounters,
) {
    if let Err(error) = run_decode(input, &mut writer, &frames, &recycle, stop, counters) {
        tracing::warn!(video = %name, %error, "video decoding stopped early");
    }
    // Always, including on the error path: without it the player would starve for ever instead of
    // finishing, and the queue would never advance.
    writer.finish();
}

/// Demuxes and decodes until the file ends, the consumer goes away, or something breaks.
///
/// Takes the container the caller already opened rather than opening one. The same input was
/// measured a moment ago, so re-opening was a second open — and for a package entry it is not
/// something a path could express at all.
fn run_decode(
    mut input: ff::format::context::Input,
    writer: &mut AudioFeedWriter,
    frames: &SyncSender<Frame>,
    recycle: &Receiver<Frame>,
    stop: &AtomicBool,
    counters: &VideoCounters,
) -> Result<(), ff::Error> {
    let video_index = input
        .streams()
        .best(ff::media::Type::Video)
        .map(|stream| stream.index())
        .ok_or(ff::Error::StreamNotFound)?;
    let audio_index = input
        .streams()
        .best(ff::media::Type::Audio)
        .map(|stream| stream.index())
        .ok_or(ff::Error::StreamNotFound)?;

    let video_time_base = input
        .stream(video_index)
        .ok_or(ff::Error::StreamNotFound)?
        .time_base();
    let audio_time_base = input
        .stream(audio_index)
        .ok_or(ff::Error::StreamNotFound)?
        .time_base();

    let mut video = {
        let mut context = ff::codec::context::Context::from_parameters(
            input
                .stream(video_index)
                .ok_or(ff::Error::StreamNotFound)?
                .parameters(),
        )?;
        // **Decode on every core the machine has, not on one.** ffmpeg defaults `thread_count` to 1,
        // and a 1080p30 H.264 song costs 80% of a single Cortex-A55 on the appliance — which starves
        // the moment one passage is harder than average, while three cores sit idle beside it. Frame
        // threading is what the ffmpeg command line turns on for itself by default; nothing here was
        // asking for it.
        //
        // `count: 0` is ffmpeg's own "decide from the machine" rather than a number this has to keep
        // right on hardware it has never seen. `Frame` rather than `Slice` because slice threading
        // needs the encoder to have produced slices and most files carry one per picture, where
        // frame threading works on anything at the cost of a few frames of latency -- which the
        // 250 ms lookahead already covers.
        context.set_threading(ff::threading::Config {
            kind: ff::threading::Type::Frame,
            count: 0,
        });
        let opened = context.decoder().video()?;
        // **What `count: 0` actually resolved to, said once per song.** It is the one number in this
        // crate that is decided by the machine rather than by the code -- five workers on the
        // appliance's four cores, sixteen on a desktop -- so a question about decode behavior "on
        // another machine" cannot be answered from the source. It was asked and could not be, which
        // is why this is here.
        //
        // **`info` rather than `debug`, because `debug` cannot be reached where this matters most.**
        // Android's filter is `info` and the only way past it is `RUST_LOG`, which needs a `wrap.`
        // system property that a retail Google TV's SELinux policy refuses outright — so on the
        // appliance a `debug!` is written for nobody. It was written as one, and the first play on a
        // television printed nothing. Once per song is the same cadence as "playing a video file
        // directly" beside it, and costs a line an hour of ordinary use.
        let threads = opened.threading();
        tracing::info!(
            threads = threads.count,
            kind = ?threads.kind,
            "the video decoder's threading"
        );
        opened
    };
    let mut audio = ff::codec::context::Context::from_parameters(
        input
            .stream(audio_index)
            .ok_or(ff::Error::StreamNotFound)?
            .parameters(),
    )?
    .decoder()
    .audio()?;

    // Native rate in, native rate out: only the layout and sample format change. The device's rate
    // is deliberately not known here -- see the module docs, and `km_audio::track`.
    let rate = audio.rate();
    let mut resampler = ff::software::resampling::Context::get(
        audio.format(),
        audio.channel_layout(),
        rate,
        ff::format::Sample::F32(ff::format::sample::Type::Packed),
        ff::channel_layout::ChannelLayout::STEREO,
        rate,
    )?;

    // Carried between iterations when a consumer is full, so nothing decoded is ever dropped.
    let mut spare_audio: Vec<f32> = Vec::new();
    let mut spare_frame: Option<Frame> = None;
    // A picture that had nowhere to go, kept so its buffers are reused rather than freed.
    let mut dropped: Option<Frame> = None;
    // Set by a seek: everything decoded before this belongs to the old position.
    let mut discard_before_ms: Option<u32> = None;

    loop {
        if stop.load(Ordering::Acquire) || writer.is_abandoned() {
            return Ok(());
        }

        if let Some(target_ms) = writer.pending_seek() {
            // `Input::seek` works in AV_TIME_BASE units, which are microseconds. Seeking backwards
            // to the keyframe at or before the target is what makes the picture correct on arrival;
            // it is also why a seek lands within one keyframe interval rather than exactly.
            let target = i64::from(target_ms) * 1_000;
            let _ = input.seek(target, ..target);
            video.flush();
            audio.flush();
            spare_audio.clear();
            // Decoding resumes at the keyframe before the target, so what comes out first is from
            // before it. Thrown away as it arrives -- see `drain_audio`.
            discard_before_ms = Some(target_ms);
            if let Some(frame) = spare_frame.take() {
                let _ = recycle.try_iter().count();
                drop(frame);
            }
            // Only after the decoders are clear, so nothing written before this point can be
            // mistaken for the new position's audio.
            writer.seek_complete();
        }

        // Hand over what is already decoded before asking for more. Either consumer being full is
        // ordinary backpressure, not a fault: the decoder is simply ahead.
        if !spare_audio.is_empty() {
            let taken = writer.push(&spare_audio);
            spare_audio.drain(..taken);
            if !spare_audio.is_empty() {
                std::thread::sleep(BACKPRESSURE_NAP);
                continue;
            }
        }
        if let Some(frame) = spare_frame.take() {
            match frames.try_send(frame) {
                Ok(()) => {}
                // **Never wait for the picture queue.** Audio is the clock: if the demuxer stopped
                // here, the ring would drain, the position would freeze, and the song would hang
                // where it stood. Which is exactly what a headless run does — nothing is drawing, so
                // nothing ever takes a picture and the queue stays full for ever.
                //
                // So the picture is dropped and decoding carries on. Dropping video under pressure
                // is the right way round: a missing frame is a frame nobody sees, and a missing
                // sample is a song that stops. Audio backpressure alone paces the demuxer, and while
                // a display is keeping up this never fires — the two queues hold the same amount of
                // time, so audio fills first.
                Err(TrySendError::Full(frame)) => {
                    // Kept, not freed: its buffers are exactly what the next picture needs, and
                    // this path runs every frame in a headless run.
                    counters.dropped.fetch_add(1, Ordering::Relaxed);
                    dropped = Some(frame);
                }
                Err(TrySendError::Disconnected(_)) => return Ok(()),
            }
        }

        let mut packet = ff::Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {}
            // The file is done. Flushing the decoders lets out whatever they were holding.
            Err(ff::Error::Eof) => {
                video.send_eof()?;
                audio.send_eof()?;
                drain_audio(
                    &mut audio,
                    &mut resampler,
                    &mut spare_audio,
                    audio_time_base,
                    &mut discard_before_ms,
                );
                let _ = writer.push(&spare_audio);
                return Ok(());
            }
            Err(error) => return Err(error),
        }

        if packet.stream() == audio_index {
            audio.send_packet(&packet)?;
            drain_audio(
                &mut audio,
                &mut resampler,
                &mut spare_audio,
                audio_time_base,
                &mut discard_before_ms,
            );
        } else if packet.stream() == video_index {
            video.send_packet(&packet)?;
            let mut decoded = ff::frame::Video::empty();
            while video.receive_frame(&mut decoded).is_ok() {
                let pts_ms = decoded
                    .pts()
                    .map_or(0, |pts| stamp_ms(pts, video_time_base));

                // Reuse a returned buffer, or make one if the pool has run dry.
                let mut frame = dropped
                    .take()
                    .or_else(|| recycle.try_recv().ok())
                    .unwrap_or_else(Frame::empty);
                if !frame.fill_from(&decoded, pts_ms) {
                    // The stream's parameters said one thing and its pictures are another, so every
                    // frame after this one is the same picture wrongly described. Ending the song
                    // runs `writer.finish()` on the way out, which is what lets the queue advance —
                    // the alternative is a song that plays no picture and never ends.
                    tracing::warn!(
                        format = %pixel_name(decoded.format()),
                        planes = decoded.planes(),
                        "the decoded picture is not the planar 4:2:0 the stream declared"
                    );
                    return Err(ff::Error::InvalidData);
                }
                match frames.try_send(frame) {
                    Ok(()) => {}
                    Err(TrySendError::Full(frame)) => {
                        spare_frame = Some(frame);
                        break;
                    }
                    Err(TrySendError::Disconnected(_)) => return Ok(()),
                }
            }
        }
    }
}

/// Pulls every ready audio frame out of the decoder, resampled to interleaved stereo `f32`.
fn drain_audio(
    audio: &mut ff::codec::decoder::Audio,
    resampler: &mut ff::software::resampling::Context,
    out: &mut Vec<f32>,
    time_base: ff::Rational,
    discard_before_ms: &mut Option<u32>,
) {
    let mut decoded = ff::frame::Audio::empty();
    while audio.receive_frame(&mut decoded).is_ok() {
        // After a seek, decoding resumes at the keyframe *at or before* the target, so the first
        // audio out of the decoder is from before where the seek asked for. Without this the
        // position would claim the target while the sound played from earlier, and the two would
        // stay that far apart for the rest of the song.
        //
        // Whole frames only. One is around twenty milliseconds, which is far below anything a
        // listener can place, and discarding part of one would mean splitting a resampled buffer to
        // save nothing.
        if let Some(target) = *discard_before_ms {
            let pts_ms = decoded.pts().map_or(0, |pts| stamp_ms(pts, time_base));
            if pts_ms < target {
                continue;
            }
            *discard_before_ms = None;
        }

        let mut converted = ff::frame::Audio::empty();
        if resampler.run(&decoded, &mut converted).is_err() {
            continue;
        }
        append_samples(&converted, out);
    }
}

/// A stream timestamp in milliseconds.
fn stamp_ms(stamp: i64, time_base: ff::Rational) -> u32 {
    if time_base.denominator() == 0 {
        return 0;
    }
    let seconds =
        stamp as f64 * f64::from(time_base.numerator()) / f64::from(time_base.denominator());
    (seconds * 1000.0).max(0.0) as u32
}

/// Appends a packed-`f32` audio frame's samples to `out`.
///
/// Read as bytes rather than through a typed plane accessor: the frame is *packed*, so both channels
/// live in plane 0 and its length in samples is per channel, which is precisely the arithmetic that
/// is easy to get wrong by one factor of two.
fn append_samples(frame: &ff::frame::Audio, out: &mut Vec<f32>) {
    let wanted = frame.samples() * FEED_CHANNELS * std::mem::size_of::<f32>();
    let bytes = frame.data(0);
    let bytes = &bytes[..wanted.min(bytes.len())];
    // `as_chunks` yields `&[u8; 4]`, which is exactly what `from_ne_bytes` wants — so the
    // four-element array literal spelling out the indices goes away rather than being rewritten.
    out.extend(
        bytes
            .as_chunks::<{ std::mem::size_of::<f32>() }>()
            .0
            .iter()
            .copied()
            .map(f32::from_ne_bytes),
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    /// A two-second 160x120 test pattern with a 440 Hz tone, encoded to the same profile packaging
    /// produces. Synthetic on purpose: it is generated by ffmpeg from `testsrc` and `sine`, so it
    /// carries nobody's content and is small enough to commit.
    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("tone.mp4")
    }

    /// The same two seconds, remuxed with a title and an artist in its container.
    ///
    /// A stream copy of [`fixture`] with two atoms added, so it is the same picture and the same
    /// tone and differs in exactly the thing under test. This is the shape a download tagged at
    /// fetch time leaves behind: yt-dlp's `--embed-metadata` writes `©nam` and `©ART`, and ffmpeg
    /// presents them as `title` and `artist`.
    fn tagged_fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("tagged.mp4")
    }

    /// A one-second 64x48 solid color in **4:4:4**, which is the layout this crate cannot copy.
    ///
    /// Lossless FFV1 rather than H.264 because an LGPL ffmpeg build has no `libx264` — x264 is GPL —
    /// and `libopenh264` does not do 4:4:4 either. The codec is beside the point: what this fixture
    /// exists to carry is a pixel format with full-height chroma, so that the refusal is tested
    /// against a real file rather than against a string.
    fn chroma444_fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("chroma444.mkv")
    }

    /// The tags are what carry a video song's title and artist from the download all the way to
    /// curation — there is nowhere else for them to live.
    #[test]
    fn probe_reads_the_containers_own_title_and_artist() {
        let info = probe(&tagged_fixture()).expect("the tagged fixture probes");
        assert_eq!(info.title.as_deref(), Some("Aeng Moo Sae"));
        assert_eq!(info.artist.as_deref(), Some("Howl"));

        // Same picture, same tone: the fixture differs in the tags and in nothing else.
        let plain = probe(&fixture()).expect("the fixture probes");
        assert_eq!(info.width, plain.width);
        assert_eq!(info.video_codec, plain.video_codec);
    }

    /// The ordinary case, and the reason the file stem is still the title of last resort: a video
    /// downloaded by hand carries no tags at all.
    #[test]
    fn an_untagged_file_reports_no_title_rather_than_an_empty_one() {
        let info = probe(&fixture()).expect("the fixture probes");
        assert_eq!(info.title, None);
        assert_eq!(info.artist, None);
    }

    /// The meter measures the audio the machine will play, and the video is never decoded.
    ///
    /// `tone.mp4` carries **mono** aac, which is what makes this worth pinning rather than a
    /// formality: what reaches the meter has been through the same `swresample` conversion to
    /// interleaved stereo that playback uses, so the number recorded is the number the gain will be
    /// applied to. ffmpeg reading the file directly agrees:
    ///
    /// ```sh
    /// ffmpeg -hide_banner -nostats -i tone.mp4 -filter_complex ebur128=peak=true -f null -
    /// #   I: -6.2 LUFS      Peak: -0.1 dBFS
    /// ```
    #[test]
    fn the_meter_measures_a_videos_audio() {
        let measured = measure_loudness(&fixture())
            .expect("the fixture decodes")
            .expect("two seconds is enough to integrate");
        assert!(
            (measured.lufs - (-6.2)).abs() < 0.5,
            "measured {} LUFS, ffmpeg says -6.2",
            measured.lufs
        );
    }

    /// A path and a window into a package measure the same file identically.
    ///
    /// The two entry points are what `km-pack` uses for a build and for a re-analysis, and a
    /// re-analysis reads the media out of an archive rather than off the disk. A number that
    /// depended on which way in it came would make a rebuilt package disagree with a re-analysed
    /// one.
    #[test]
    fn measuring_through_a_reader_matches_measuring_a_path() {
        let by_path = measure_loudness(&fixture())
            .expect("decodes")
            .expect("measures");

        let file = std::fs::File::open(fixture()).expect("opens");
        let by_reader = measure_loudness_from(file, "media/0001.mp4")
            .expect("decodes")
            .expect("measures");

        assert_eq!(by_path, by_reader);
    }

    #[test]
    fn probe_reads_the_shape_without_decoding() {
        let info = probe(&fixture()).expect("the fixture probes");
        assert_eq!(info.width, 160);
        assert_eq!(info.height, 120);
        assert_eq!(info.audio_sample_rate, 48_000);
        assert_eq!(info.frame_rate_milli, 30_000, "30 fps, constant");
        assert_eq!(info.video_codec, "h264");
        assert_eq!(info.audio_codec, "aac");
        assert_eq!(info.pixel_format, "yuv420p");
        // Mono, and it plays anyway: the decoder resamples to stereo. Pinned rather than corrected
        // because a fixture that is not already in the shape packaging aims for is what proves the
        // field reports the file instead of the profile.
        assert_eq!(info.audio_channels, 1);
        // Two seconds, give or take the container's rounding.
        assert!(
            (1_900..=2_100).contains(&info.duration_ms),
            "duration was {}",
            info.duration_ms
        );
    }

    /// The two halves of the pixel-format rule, which pull in opposite directions on purpose.
    ///
    /// `probe` must succeed, because packaging reads a file's shape in order to decide it needs
    /// re-encoding — a probe that failed here would make that decision unreachable. `open` must
    /// refuse, because `Frame::fill_from` would otherwise copy half the chroma rows this picture has
    /// and draw the result without complaint.
    #[test]
    fn a_444_source_probes_but_is_refused_rather_than_drawn() {
        let info = probe(&chroma444_fixture()).expect("a 4:4:4 file still probes");
        assert_eq!(info.pixel_format, "yuv444p");
        assert!(!supports_pixel_format(&info.pixel_format));

        let error = open(&chroma444_fixture()).expect_err("but it does not open");
        let VideoError::UnsupportedPixelFormat { format, .. } = error else {
            panic!("wanted an UnsupportedPixelFormat, got {error:?}");
        };
        assert_eq!(
            format, "yuv444p",
            "the message names what it actually found"
        );
    }

    /// `yuvj420p` is the deprecated full-range spelling of the same three planes at the same sizes.
    /// Refusing it would reject a large number of ordinary files over a difference the GPU's color
    /// conversion already handles.
    #[test]
    fn the_full_range_spelling_of_420_is_still_420() {
        assert!(supports_pixel_format("yuv420p"));
        assert!(supports_pixel_format("yuvj420p"));
        assert!(!supports_pixel_format("yuv422p"));
        assert!(
            !supports_pixel_format("yuv420p10le"),
            "10-bit is two bytes a sample"
        );
    }

    #[test]
    fn a_missing_file_is_an_error_rather_than_a_panic() {
        let error = probe(Path::new("no/such/video.mp4")).expect_err("missing files fail");
        assert!(matches!(error, VideoError::Open { .. }));
    }

    /// A video reads the same whether it arrives as a path or as bytes somebody hands over.
    ///
    /// This is the whole of what moving media into the `.kmpkg` asks of this crate: the packaged
    /// route supplies a window into an archive instead of a file, and everything downstream has to
    /// be unable to tell.
    #[test]
    fn a_reader_probes_to_the_same_shape_as_a_path() {
        let by_path = probe(&fixture()).expect("path");
        let file = std::fs::File::open(fixture()).expect("open");
        let by_reader = probe_from(file, "media/0007.mp4").expect("reader");
        assert_eq!(by_path, by_reader);
    }

    /// With custom I/O there is no filename for ffmpeg to look at, so the name is doing real work.
    ///
    /// The failure this guards against is quiet: a name with no extension leaves probing to sniff
    /// the bytes, which for a fragmented MP4 or a short read can decide wrong. An entry is called
    /// `media/<number>.mp4` precisely so this keeps working.
    #[test]
    fn the_name_is_what_a_reader_is_probed_as() {
        let file = std::fs::File::open(fixture()).expect("open");
        let info = probe_from(file, "media/0007.mp4").expect("probe");
        assert_eq!(info.video_codec, "h264");
        assert!(info.duration_ms > 0);
    }

    /// And a video **decodes** from a reader, not merely probes.
    ///
    /// Sound coming out is the assertion: it means the decoder thread read from the window, which is
    /// the part a probe cannot exercise because probing happens before the thread starts.
    #[test]
    fn a_video_decodes_from_a_reader() {
        let file = std::fs::File::open(fixture()).expect("open");
        let (info, reader, feed, frames) =
            open_from(file, "media/0007.mp4").expect("the fixture opens");
        assert_eq!(reader.name(), "media/0007.mp4");
        assert_eq!(info.width, 160);

        let mut player = km_audio::TrackPlayer::new(feed, 48_000);
        let mut left = vec![0.0; 512];
        let mut right = vec![0.0; 512];
        let mut peak = 0.0f32;
        let mut picture = None;
        for _ in 0..400 {
            player.render(&mut left, &mut right, true);
            peak = peak.max(left.iter().fold(0.0f32, |acc, s| acc.max(s.abs())));
            if picture.is_none() {
                picture = frames.take_frame_for(u32::MAX);
            }
            if peak > 0.01 && picture.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(peak > 0.01, "no audio came out of a reader-backed video");
        let picture = picture.expect("a picture came out of a reader-backed video");
        let (y, y_stride) = picture.y();
        assert!(y_stride >= 160, "luma stride was {y_stride}");
        assert_eq!(y.len(), y_stride * 120, "the whole luma plane is present");
        frames.recycle(picture);
    }

    /// The whole chain: ffmpeg decodes, km-audio's feed carries it, the player renders it.
    #[test]
    fn a_real_file_decodes_to_both_audio_and_pictures() {
        let (_info, reader, feed, frames) = open(&fixture()).expect("the fixture opens");
        let mut player = km_audio::TrackPlayer::new(feed, 48_000);

        let mut left = vec![0.0; 512];
        let mut right = vec![0.0; 512];
        let mut peak = 0.0f32;
        let mut picture = None;

        // The decoder is a thread that has just started, so this is a wait, not a poll loop with a
        // guess in it: both conditions are what the test is actually about.
        for _ in 0..400 {
            player.render(&mut left, &mut right, true);
            peak = peak.max(left.iter().fold(0.0f32, |acc, s| acc.max(s.abs())));
            if picture.is_none() {
                picture = frames.take_frame_for(player.position_ms());
            }
            if peak > 0.3 && picture.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(
            peak > 0.3,
            "a near-full-scale 440 Hz tone should reach the output audibly"
        );

        let picture = picture.expect("a decoded picture arrived");
        assert_eq!((picture.width, picture.height), (160, 120));
        let (y, y_stride) = picture.y();
        assert!(y_stride >= 160, "luma stride was {y_stride}");
        assert_eq!(y.len(), y_stride * 120, "the whole luma plane is present");
        let (u, u_stride) = picture.u();
        assert_eq!(u.len(), u_stride * 60, "4:2:0 chroma is half height");

        frames.recycle(picture);
        assert_eq!(
            reader.name(),
            fixture().display().to_string(),
            "the reader knows what it is playing"
        );
    }

    #[test]
    fn dropping_the_reader_stops_the_decoder() {
        let (_info, reader, feed, frames) = open(&fixture()).expect("the fixture opens");
        drop(reader);
        // The thread is joined by the drop, so nothing is still running by the time we get here.
        drop(frames);
        drop(feed);
    }
}

#[cfg(test)]
mod counter_tests {
    use super::*;

    /// A reader with a queue somebody else fills, and no decoder behind it.
    fn reader(depth: usize) -> (SyncSender<Frame>, FrameReader) {
        let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel(depth);
        let (recycle_tx, _recycle_rx) = std::sync::mpsc::sync_channel(depth + 2);
        (
            frame_tx,
            FrameReader {
                inner: std::sync::Mutex::new(Pending {
                    frames: frame_rx,
                    held: None,
                }),
                recycle: recycle_tx,
                counters: Arc::new(VideoCounters::default()),
            },
        )
    }

    fn frame_at(pts_ms: u32) -> Frame {
        Frame {
            pts_ms,
            ..Frame::empty()
        }
    }

    #[test]
    fn taking_one_due_picture_skips_nothing() {
        let (tx, reader) = reader(4);
        tx.send(frame_at(0)).unwrap();
        tx.send(frame_at(100)).unwrap();

        // The display is keeping up: it asks at 40 ms, gets the frame due at 0, and the one at 100
        // is held for later rather than skipped.
        assert_eq!(reader.take_frame_for(40).map(|f| f.pts_ms), Some(0));
        assert_eq!(reader.counters().skipped(), 0);
    }

    #[test]
    fn a_late_display_skips_the_pictures_it_missed_and_counts_them() {
        let (tx, reader) = reader(8);
        for pts in [0, 33, 66, 100] {
            tx.send(frame_at(pts)).unwrap();
        }

        // Asked for the first time at 100 ms: three pictures came due while it waited, so the
        // newest is drawn and the three before it are counted rather than shown in a rush.
        assert_eq!(reader.take_frame_for(100).map(|f| f.pts_ms), Some(100));
        assert_eq!(reader.counters().skipped(), 3);
    }

    #[test]
    fn nothing_due_is_not_a_skip() {
        let (tx, reader) = reader(4);
        tx.send(frame_at(500)).unwrap();

        // The ordinary case, sixty times a second: the display redraws faster than the video's
        // frame rate and simply keeps the picture it has. That is not the decoder falling behind
        // and must never be counted as one -- if it were, every healthy video song would report a
        // fault, which is the whole reason `skipped` rather than `dropped` gates the warning.
        assert!(reader.take_frame_for(10).is_none());
        assert!(reader.take_frame_for(20).is_none());
        assert_eq!(reader.counters().skipped(), 0);
    }

    /// A frame that arrived early is handed out when it comes due, not thrown away.
    ///
    /// The three tests above all assert `held` from the *outside* — that an early frame is not
    /// returned yet — and every one of them would still pass if the held branch dropped the frame
    /// instead of keeping it. That failure is a video that plays at the decoder's mercy: any frame
    /// arriving before its time disappears, which at a normal lookahead is most of them.
    #[test]
    fn a_picture_held_back_is_the_one_handed_out_when_it_comes_due() {
        let (tx, reader) = reader(4);
        tx.send(frame_at(500)).unwrap();

        assert!(reader.take_frame_for(100).is_none(), "not due yet");
        // Nothing is sent between the two calls: the frame this returns can only be the held one.
        assert_eq!(reader.take_frame_for(500).map(|f| f.pts_ms), Some(500));
        assert_eq!(reader.counters().skipped(), 0);
    }

    /// A skipped picture's buffers go back to the decoder rather than being freed.
    ///
    /// The pool is the reason `Frame` is refilled rather than allocated, and a skip is the one place
    /// a frame is discarded *without* the display having drawn it — so it is the path where a
    /// forgotten `recycle` would leak the pool away one frame at a time and put the decoder back to
    /// allocating a picture per frame. Nothing checked it, because the harness dropped the receiver.
    #[test]
    fn a_skipped_picture_hands_its_buffers_back() {
        let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel(8);
        let (recycle_tx, recycle_rx) = std::sync::mpsc::sync_channel(8);
        let reader = FrameReader {
            inner: std::sync::Mutex::new(Pending {
                frames: frame_rx,
                held: None,
            }),
            recycle: recycle_tx,
            counters: Arc::new(VideoCounters::default()),
        };
        for pts in [0, 33, 66, 100] {
            frame_tx.send(frame_at(pts)).unwrap();
        }

        assert_eq!(reader.take_frame_for(100).map(|f| f.pts_ms), Some(100));

        let handed_back: Vec<u32> = recycle_rx.try_iter().map(|f| f.pts_ms).collect();
        assert_eq!(
            handed_back,
            [0, 33, 66],
            "every skipped picture, and in the order it was skipped"
        );

        // And the one that *was* drawn goes back only when the display says so.
        reader.recycle(frame_at(100));
        assert_eq!(recycle_rx.try_recv().map(|f| f.pts_ms), Ok(100));
    }
}

#[cfg(test)]
mod plane_tests {
    use super::*;

    /// The picture queue holds at least a lookahead's worth of time, at every frame rate.
    ///
    /// That sentence is the whole contract — [`LOOKAHEAD_MS`] argues that the audio ring must be the
    /// binding constraint, and it only holds while the picture queue is not. The `+ 2` is slack
    /// against integer division, and the floor is what covers the rates where the division reaches
    /// zero. Nothing asserted any of it.
    #[test]
    fn the_picture_queue_always_holds_a_lookahead_of_time() {
        // Milli-fps, as ffmpeg reports a rate: silent film through a high-refresh capture.
        for rate in [
            1_000u32, 12_000, 23_976, 24_000, 25_000, 30_000, 50_000, 60_000, 120_000,
        ] {
            let frames = frame_queue_len(rate) as u64;
            let held_ms = frames * 1_000_000 / u64::from(rate);
            assert!(
                held_ms >= u64::from(LOOKAHEAD_MS),
                "at {rate} milli-fps the queue holds {held_ms} ms, short of {LOOKAHEAD_MS}"
            );
            assert!(frames >= MIN_FRAME_QUEUE as u64, "at {rate} milli-fps");
        }

        // A rate of zero is what a container with no frame rate at all reports, and must not give a
        // queue of zero — a `sync_channel(0)` is a rendezvous, which would stall the decoder on
        // every frame.
        assert_eq!(frame_queue_len(0), MIN_FRAME_QUEUE);
    }

    /// A timestamp converts through its own time base, and nonsense becomes zero rather than a spike.
    ///
    /// A single bad `pts_ms` is not a cosmetic fault: it is compared against the audio position to
    /// decide what is due, so one enormous value holds a picture back for the rest of the song and
    /// one negative value would make everything after it look overdue.
    #[test]
    fn a_timestamp_converts_through_its_time_base() {
        // The two bases nearly every file uses: milliseconds, and MPEG-TS' 90 kHz clock.
        assert_eq!(stamp_ms(1_500, ff::Rational(1, 1_000)), 1_500);
        assert_eq!(stamp_ms(90_000, ff::Rational(1, 90_000)), 1_000);
        // A base with a numerator, which Matroska writes.
        assert_eq!(stamp_ms(40, ff::Rational(1, 25)), 1_600);

        // A negative stamp is ordinary: B-frames before the first keyframe carry one.
        assert_eq!(stamp_ms(-5_000, ff::Rational(1, 1_000)), 0);
        // And a zero denominator is a corrupt header, not a division.
        assert_eq!(stamp_ms(1_000, ff::Rational(1, 0)), 0);
    }

    /// Chroma planes are half height, rounded **up**, and an odd height is where that shows.
    ///
    /// `fill_from`'s doc says planar 4:2:0 is a requirement rather than a preference, and the
    /// rounding is the half of that a test can hold: at 241 rows the chroma planes have 121, and
    /// taking 120 would copy one row short and leave the bottom of the picture holding whatever the
    /// recycled frame had there before — a stripe of the previous song's colour.
    #[test]
    fn an_odd_height_rounds_the_chroma_planes_up() {
        let mut decoded = ff::frame::Video::new(ff::format::Pixel::YUV420P, 320, 241);
        for plane in 0..PLANES {
            decoded.data_mut(plane).fill(0x5A);
        }

        let mut frame = Frame::empty();
        frame.fill_from(&decoded, 1_234);

        assert_eq!((frame.pts_ms, frame.width, frame.height), (1_234, 320, 241));
        let (y, y_stride) = frame.y();
        assert_eq!(y.len(), y_stride * 241, "luma is every row");
        for (plane, rows) in [(frame.u(), 121), (frame.v(), 121)] {
            let (data, stride) = plane;
            assert_eq!(data.len(), stride * rows, "chroma rounds 241/2 up to 121");
            assert!(data.iter().all(|b| *b == 0x5A), "and is copied, not zeroed");
        }
    }

    /// A picture that is not what its stream declared is refused rather than copied.
    ///
    /// **The container is not the authority on this and cannot be.** `open` reads the pixel format
    /// from `AVCodecParameters`, which is settled before a frame has been decoded, so a bitstream
    /// that decodes to something else reaches `fill_from` having passed every check upstream. Asking
    /// `ffmpeg_next` for plane 1 of a one-plane picture panics, and the panic unwinds the decoder
    /// thread past `writer.finish()` — leaving a song that plays no picture and never ends, which is
    /// worse than one that stops.
    #[test]
    fn a_picture_with_fewer_planes_than_declared_is_refused_rather_than_panicking() {
        let mut frame = Frame::empty();

        // Grey is one plane. Nothing here reads plane 1, which is the whole assertion.
        let grey = ff::frame::Video::new(ff::format::Pixel::GRAY8, 320, 240);
        assert!(
            !frame.fill_from(&grey, 0),
            "a one-plane picture must be refused"
        );

        // 4:4:4 has three planes at full height, so it passes the plane count and fails the format:
        // copying it would take half the chroma rows the picture has and draw a wrong picture.
        let full = ff::frame::Video::new(ff::format::Pixel::YUV444P, 320, 240);
        assert!(
            !frame.fill_from(&full, 0),
            "three planes is not enough on its own; the layout has to be 4:2:0"
        );

        // And the frame is left alone by a refusal, so a recycled buffer cannot be half overwritten
        // with a picture that was rejected.
        assert_eq!((frame.width, frame.height), (0, 0));
    }

    /// Refilling a frame from a smaller picture leaves nothing of the larger one behind.
    ///
    /// The whole point of the pool is that these buffers are reused, so the sizes must come from the
    /// *new* picture rather than from whatever capacity the `Vec` happens to have. A `resize` where
    /// the code has `clear` then `extend` would keep the tail of the previous frame and draw it.
    #[test]
    fn a_reused_frame_takes_the_new_pictures_size_and_none_of_the_old() {
        let mut frame = Frame::empty();

        let mut large = ff::frame::Video::new(ff::format::Pixel::YUV420P, 640, 480);
        for plane in 0..PLANES {
            large.data_mut(plane).fill(0xFF);
        }
        frame.fill_from(&large, 0);
        let was = frame.y().0.len();

        let mut small = ff::frame::Video::new(ff::format::Pixel::YUV420P, 320, 240);
        for plane in 0..PLANES {
            small.data_mut(plane).fill(0x11);
        }
        frame.fill_from(&small, 40);

        assert_eq!((frame.width, frame.height), (320, 240));
        assert!(
            frame.y().0.len() < was,
            "the luma plane shrank with the picture"
        );
        assert_eq!(frame.y().0.len(), frame.y().1 * 240);
        assert!(
            frame.y().0.iter().all(|b| *b == 0x11),
            "not one byte of the larger picture survived"
        );
    }

    /// Packed stereo `f32` yields two samples per frame-sample, not one and not four.
    ///
    /// `append_samples`' own doc calls this "precisely the arithmetic that is easy to get wrong by
    /// one factor of two" — `frame.samples()` is per channel while plane 0 holds both — and then
    /// nothing checked the factor.
    #[test]
    fn packed_stereo_yields_two_floats_a_sample() {
        const SAMPLES: usize = 64;
        let mut audio = ff::frame::Audio::new(
            ff::format::Sample::F32(ff::format::sample::Type::Packed),
            SAMPLES,
            ff::ChannelLayout::STEREO,
        );
        // Interleaved left/right, each channel a constant, so a factor of two shows up as a value.
        let pattern: Vec<u8> = (0..SAMPLES)
            .flat_map(|_| [0.25f32, -0.25f32])
            .flat_map(f32::to_ne_bytes)
            .collect();
        audio.data_mut(0)[..pattern.len()].copy_from_slice(&pattern);

        let mut out = Vec::new();
        append_samples(&audio, &mut out);

        assert_eq!(out.len(), SAMPLES * FEED_CHANNELS, "one pair per sample");
        assert_eq!(out[0], 0.25);
        assert_eq!(out[1], -0.25);
        assert_eq!(out[out.len() - 1], -0.25, "and the last pair is whole");
    }
}
