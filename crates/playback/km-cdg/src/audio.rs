//! The MP3 half of an MP3+G song: a decoder thread feeding `km-audio`'s ring.
//!
//! This is `km-video`'s audio path with the video removed, and deliberately so — the two produce the
//! same thing for the same consumer, and the seek protocol they both obey is `km-audio`'s, not
//! theirs. Samples go in **interleaved stereo at the file's own rate**; the output device's rate
//! never reaches here, because `TrackPlayer` resamples on the way out and a decoder that knew the
//! device rate would be coupled to a decision made much later and somewhere else.
//!
//! What is different from video is only what is absent. There is no second stream, so there is no
//! picture queue that must never stall the demuxer, no frame pool and no lookahead balancing act:
//! this thread has exactly one consumer and blocking on it is correct.
//!
//! # Trimming is ours to do
//!
//! An MP3 begins with encoder delay and ends with padding — samples the encoder added that were
//! never in the recording. Symphonia reports them per packet as `trim_start` and `trim_end` and does
//! **not** remove them, so this module does. Without it every song would begin a few milliseconds
//! late, and since CD+G timing is locked to the audio position that error would land straight on the
//! words.

#[cfg(test)]
mod tests;

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration as StdDuration;

use km_audio::{AudioFeed, AudioFeedWriter, FEED_CHANNELS, audio_feed};
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, StandardTag};
use symphonia::core::units::{Time, TimeBase};

use crate::CdgError;

/// How much audio the ring holds.
///
/// Enough to cover a decode hiccup and no more: every frame of it is latency on a seek. A quarter of
/// a second is the same figure `km-video` settled on, for the same reason.
const LOOKAHEAD_MS: u32 = 250;

/// How long to wait when the ring is full. Ordinary backpressure, not a fault.
const BACKPRESSURE_NAP: StdDuration = StdDuration::from_millis(4);

/// What a probe found in the audio file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    /// Length of the recording. **This is the song's length** — see
    /// [`crate::GraphicsStream::duration_ms`] for why the graphics are not.
    pub duration_ms: u32,
    /// Sample rate, which is what the feed carries and what `TrackPlayer` resamples from.
    pub sample_rate: u32,
    /// Channels in the file, before the downmix to stereo.
    pub channels: u16,
    /// Codec name, for reporting.
    pub codec: String,
    /// The title the file's own tags claim, if any.
    ///
    /// **Offered, not trusted.** Measured over 2,851 real tracks, ID3 is present on about half and
    /// is frequently wrong — artist and title swapped, titles that are literally `Track  6`. What to
    /// do about that is a packaging decision and lives there, not here; this reports what the file
    /// says. See the `Where an MP3+G song's title and artist come from` decision in `docs/decisions/`.
    pub title: Option<String>,
    /// The artist the file's own tags claim, if any. The same warning applies.
    pub artist: Option<String>,
}

/// Reads an audio file's shape without decoding it.
pub fn probe_audio(path: &Path) -> Result<AudioInfo, CdgError> {
    let name = path.display().to_string();
    let (format, track_id) = open_format(path)?;
    audio_info_of(format, track_id, &name)
}

/// The same, from anything seekable — a file, or a window into a package.
///
/// This one keeps its own reader rather than sharing the one `open_from` uses, and it has to:
/// counting the duration walks every packet to the end of the file, which would leave a shared
/// reader with nothing left to decode.
pub fn probe_audio_from<R: Read + Seek + Send + Sync + 'static>(
    reader: R,
    name: &str,
) -> Result<AudioInfo, CdgError> {
    let (format, track_id) = open_format_from(reader, name)?;
    audio_info_of(format, track_id, name)
}

/// Reads the shape off an already-open container.
fn audio_info_of(
    mut format: Box<dyn FormatReader>,
    track_id: u32,
    name: &str,
) -> Result<AudioInfo, CdgError> {
    let track = format
        .tracks()
        .iter()
        .find(|track| track.id == track_id)
        .ok_or_else(|| CdgError::no_audio_track(name))?;

    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| CdgError::no_audio_track(name))?;
    let spec = params
        .sample_rate
        .zip(params.channels.as_ref())
        .ok_or_else(|| CdgError::no_audio_track(name))?;
    let (sample_rate, channels) = (spec.0, spec.1.count());

    let stated = track
        .num_frames
        .zip(track.time_base)
        .map(|(frames, base)| ms_from(base, frames));
    let time_base = track.time_base;
    let codec = format.format_info().short_name.to_owned();

    let (title, artist) = tags(format.as_mut());

    // **The length is counted, never taken from the header, and that is a decision the corpus
    // forced.** An MP3 states its length in a Xing or Info header, and one file in the measured
    // corpus states it wrongly by a factor of seven: the header claims 1,646 s, ffmpeg's own
    // bitrate estimate says 1,032 s, and the audio actually in the file is **242 s** — which is,
    // to a tenth of a second, exactly as long as the `.cdg` beside it.
    //
    // That is not a curiosity. This number goes into the manifest, drives the progress bar, and
    // decides when the machine moves to the next song; a song that claims to be twenty-seven
    // minutes long leaves the room staring at a finished song for twenty-three of them. So the file
    // is walked and its real frame durations summed. That parses frame headers and decodes nothing,
    // it costs one sequential read, and it is the only answer that cannot be a lie.
    let duration_ms = count_duration_ms(format.as_mut(), track_id, time_base)?;
    if let Some(stated) = stated
        && stated.abs_diff(duration_ms) > 2_000
    {
        tracing::debug!(
            audio = %name,
            stated, counted = duration_ms,
            "the file's own header disagrees with the audio in it; counted wins"
        );
    }

    Ok(AudioInfo {
        duration_ms,
        sample_rate,
        channels: u16::try_from(channels).unwrap_or(u16::MAX),
        codec,
        title,
        artist,
    })
}

/// Measures how loud an MP3 is, for a package being built.
///
/// See [`measure_loudness_from`] for everything that matters about it; this is the same over a path.
pub fn measure_loudness(path: &Path) -> Result<Option<km_loudness::Loudness>, CdgError> {
    let name = path.display().to_string();
    let (format, track_id) = open_format(path)?;
    loudness_of(format, track_id, &name)
}

/// The same, from anything seekable — a file, or a window into a package.
///
/// Shaped like [`probe_audio_from`] beside it rather than taking an already-open container: a caller
/// with a `.kmpkg` entry has a reader, and opening the format is this module's business.
pub fn measure_loudness_from<R: Read + Seek + Send + Sync + 'static>(
    reader: R,
    name: &str,
) -> Result<Option<km_loudness::Loudness>, CdgError> {
    let (format, track_id) = open_format_from(reader, name)?;
    loudness_of(format, track_id, name)
}

/// Decodes an audio track into a loudness meter.
///
/// **Synchronous, and deliberately not through a `TrackPlayer`.** The live path decodes on a thread
/// into a quarter-second ring that a real-time callback drains, and driving that faster than real
/// time wins the race the ring exists to lose: the player answers an empty feed with silence and a
/// frozen position, so an unpaced loop measures a six-minute song as five seconds of gaps. See
/// `Judging it by ear, and one trap in doing so` in `docs/architecture/cdg.md`. So this has no
/// thread, no ring and no backpressure — it pulls packets and hands the samples straight to a meter.
///
/// **Nothing is lost by reading earlier than the player does.** The same note puts the player's own
/// output within 0.017 dB RMS of ffmpeg's decode of the same file, and [`append`] below is what
/// produces the samples either way — including the encoder trims, so what is measured is the
/// recording rather than the encoder's padding.
///
/// `Ok(None)` is a file with less audio in it than R128 integrates over, or one that is silent. That
/// is not an error: the caller writes no measurement and the song plays at gain 1.0, exactly as it
/// does today.
fn loudness_of(
    mut format: Box<dyn FormatReader>,
    track_id: u32,
    name: &str,
) -> Result<Option<km_loudness::Loudness>, CdgError> {
    let track = format
        .tracks()
        .iter()
        .find(|track| track.id == track_id)
        .cloned()
        .ok_or_else(|| CdgError::no_audio_track(name))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .cloned()
        .ok_or_else(|| CdgError::no_audio_track(name))?;
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| CdgError::no_audio_track(name))?;
    let channels = params.channels.as_ref().map_or(2, |set| set.count()).max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|source| CdgError::undecodable(name, &source))?;

    // The meter takes the feed's rate, not the device's -- there is no device here, and the file's
    // own rate is what the samples are at.
    let Some(mut meter) = km_loudness::Meter::stereo(sample_rate) else {
        return Ok(None);
    };

    let mut spare: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    while let Some(packet) = format
        .next_packet()
        .map_err(|source| CdgError::undecodable(name, &source))?
    {
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                spare.clear();
                append(
                    &mut spare,
                    &mut scratch,
                    &decoded,
                    channels,
                    packet.trim_start.get(),
                    packet.trim_end.get(),
                );
                meter.add(&spare);
            }
            // Skipped rather than fatal, the same judgment `run_decode` makes: one damaged frame
            // otherwise costs the measurement of the whole recording, and a song whose level was
            // measured over all but one frame of it is measured.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(source) => return Err(CdgError::undecodable(name, &source)),
        }
    }

    Ok(meter.finish())
}

/// Opens an audio file and starts decoding it into a feed.
///
/// The returned reader owns the thread: dropping it stops decoding and **joins**, so it must be
/// dropped where blocking is allowed and never on the audio callback.
pub(crate) fn open_audio(path: &Path) -> Result<(AudioReader, AudioFeed), CdgError> {
    let name = path.display().to_string();
    let (format, track_id) = open_format(path)?;
    open_audio_from(format, track_id, &name)
}

/// The same, from a container the caller has already opened.
///
/// **Opened once.** Reading the track parameters through a `sample_rate_of` that opens the file and
/// drops it leaves the decoder thread opening the same file again. A `Box<dyn FormatReader>` is
/// `Send + Sync`, so the measured one moves to the thread instead — which is both a saving and the
/// only shape that works for a package entry, where "open it again" would mean re-opening the
/// archive and finding the entry a second time.
///
/// Deliberately **not** a full probe: counting the duration means walking every packet to the end of
/// the file, which would leave the reader with nothing left to decode, and would put a
/// multi-megabyte read on the path between pressing a number and hearing anything. Whoever wants the
/// length asks for it separately, or takes it from the manifest.
pub(crate) fn open_audio_from(
    format: Box<dyn FormatReader>,
    track_id: u32,
    name: &str,
) -> Result<(AudioReader, AudioFeed), CdgError> {
    let sample_rate = format
        .tracks()
        .iter()
        .find(|track| track.id == track_id)
        .and_then(|track| track.codec_params.as_ref())
        .and_then(|params| params.audio())
        .and_then(|params| params.sample_rate)
        .ok_or_else(|| CdgError::no_audio_track(name))?;

    let ring_frames = (sample_rate as usize).saturating_mul(LOOKAHEAD_MS as usize) / 1_000;
    let (mut writer, feed) = audio_feed(sample_rate, ring_frames.max(1));

    let stop = Arc::new(AtomicBool::new(false));
    let thread = std::thread::Builder::new()
        .name("km-cdg-decode".to_owned())
        .spawn({
            let stop = Arc::clone(&stop);
            let owned = name.to_owned();
            move || {
                if let Err(error) = run_decode(format, track_id, &mut writer, &stop) {
                    tracing::warn!(audio = %owned, %error, "MP3 decoding stopped");
                }
                // On **every** exit path, including an error. Without it the player waits for
                // samples that are never coming and the song hangs where it stood instead of
                // ending.
                writer.finish();
            }
        })
        .map_err(|source| CdgError::io(name, source))?;

    Ok((
        AudioReader {
            stop,
            thread: Some(thread),
        },
        feed,
    ))
}

/// Keeps a decoder thread alive; dropping it stops and joins.
#[derive(Debug)]
pub struct AudioReader {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for AudioReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// symphonia's [`MediaSource`], for anything seekable.
///
/// **A newtype because of the orphan rule**, and that is the whole reason it exists: `MediaSource`
/// is symphonia's trait and a package window is `km-kmpkg`'s type, so neither this crate nor that
/// one may write the impl. Wrapping it here is what lets a package's bytes reach a decoder without
/// either crate learning the other's name — which is why `km-cdg` still depends on neither `zip` nor
/// `serde`, and why its Cargo.toml's boast about having no optional anything stays true.
///
/// `byte_len` is measured once at construction, because the trait takes `&self` and so cannot seek.
/// That is what symphonia's own `File` impl does with `metadata()`.
///
/// `is_seekable` **must** be true, or `format.seek` — which is how a rewind mid-song works — stops
/// working silently rather than failing.
struct Seekable<R> {
    inner: R,
    len: Option<u64>,
}

impl<R: Read + Seek + Send + Sync> Seekable<R> {
    fn new(mut inner: R) -> std::io::Result<Self> {
        let len = inner.seek(SeekFrom::End(0)).ok();
        inner.seek(SeekFrom::Start(0))?;
        Ok(Self { inner, len })
    }
}

impl<R: Read> Read for Seekable<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Seekable<R> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(from)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for Seekable<R> {
    fn is_seekable(&self) -> bool {
        self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

/// Opens an audio file and finds its audio track.
fn open_format(path: &Path) -> Result<(Box<dyn FormatReader>, u32), CdgError> {
    let name = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|source| CdgError::io(&name, source))?;
    open_format_from(file, &name)
}

/// The same, from anything seekable — a file, or a window into a package.
///
/// `name` is the subject of the error messages and, through [`Hint`], the extension symphonia is
/// told to expect. A package entry is called `media/<number>.mp3` so that this keeps working.
pub(crate) fn open_format_from<R: Read + Seek + Send + Sync + 'static>(
    reader: R,
    name: &str,
) -> Result<(Box<dyn FormatReader>, u32), CdgError> {
    let source = Seekable::new(reader).map_err(|source| CdgError::io(name, source))?;
    let stream = MediaSourceStream::new(Box::new(source), Default::default());

    let mut hint = Hint::new();
    if let Some((_, extension)) = name.rsplit_once('.') {
        hint.with_extension(extension);
    }

    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|source| CdgError::undecodable(name, &source))?;

    // `first_track_known_codec` rather than `default_track`: an MP3 from this corpus can carry cover
    // art, which arrives as a track this build has no decoder for, and picking it would fail on a
    // file that plays perfectly.
    let track_id = format
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| CdgError::no_audio_track(name))?
        .id;
    Ok((format, track_id))
}

/// Walks the file summing packet durations. Parses frame headers; decodes nothing.
fn count_duration_ms(
    format: &mut dyn FormatReader,
    track_id: u32,
    time_base: Option<TimeBase>,
) -> Result<u32, CdgError> {
    let Some(base) = time_base else {
        return Ok(0);
    };
    let mut frames = 0u64;
    while let Some(packet) = format.next_packet().unwrap_or(None) {
        if packet.track_id == track_id {
            frames = frames.saturating_add(packet.dur.get());
        }
    }
    Ok(ms_from(base, frames))
}

fn ms_from(base: TimeBase, ticks: u64) -> u32 {
    let ms = u64::from(base.numer.get())
        .saturating_mul(ticks)
        .saturating_mul(1000)
        / u64::from(base.denom.get()).max(1);
    u32::try_from(ms).unwrap_or(u32::MAX)
}

/// The title and artist the file's own tags claim.
fn tags(format: &mut dyn FormatReader) -> (Option<String>, Option<String>) {
    let mut title = None;
    let mut artist = None;
    if let Some(revision) = format.metadata().current() {
        for tag in &revision.media.tags {
            match &tag.std {
                Some(StandardTag::TrackTitle(value)) if title.is_none() => {
                    title = Some(value.trim().to_owned());
                }
                Some(StandardTag::Artist(value)) if artist.is_none() => {
                    artist = Some(value.trim().to_owned());
                }
                _ => {}
            }
        }
    }
    // A blank tag is worse than an absent one: it looks like an answer. `km_video::tag` already
    // drops these, for the same reason.
    (
        title.filter(|value| !value.is_empty()),
        artist.filter(|value| !value.is_empty()),
    )
}

fn run_decode(
    mut format: Box<dyn FormatReader>,
    track_id: u32,
    writer: &mut AudioFeedWriter,
    stop: &AtomicBool,
) -> Result<(), SymphoniaError> {
    let track = format
        .tracks()
        .iter()
        .find(|track| track.id == track_id)
        .cloned();
    let Some(track) = track else { return Ok(()) };
    let Some(params) = track.codec_params.as_ref().and_then(|p| p.audio()).cloned() else {
        return Ok(());
    };
    let channels = params.channels.as_ref().map_or(2, |set| set.count()).max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())?;

    let mut spare: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    // Set by a seek: everything decoded before this belongs to the old position.
    let mut discard_before: Option<u64> = None;

    loop {
        if stop.load(Ordering::Acquire) || writer.is_abandoned() {
            return Ok(());
        }

        if let Some(target_ms) = writer.pending_seek() {
            let time = Time::try_new(i64::from(target_ms / 1000), (target_ms % 1000) * 1_000_000);
            let _ = format.seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time: time.unwrap_or(Time::ZERO),
                    track_id: Some(track_id),
                },
            );
            decoder.reset();
            spare.clear();
            // A coarse seek lands at or before the target, so what arrives first is from before it.
            // Thrown away by timestamp as it comes.
            discard_before = track
                .time_base
                .map(|base| ticks_for_ms(base, target_ms))
                .filter(|_| target_ms > 0);
            // Only after the decoder is clear, so nothing written before this can be mistaken for
            // the new position's audio.
            writer.seek_complete();
        }

        // Hand over what is already decoded before asking for more.
        if !spare.is_empty() {
            let taken = writer.push(&spare);
            spare.drain(..taken);
            if !spare.is_empty() {
                std::thread::sleep(BACKPRESSURE_NAP);
                continue;
            }
        }

        let Some(packet) = format.next_packet()? else {
            writer.finish();
            return Ok(());
        };
        if packet.track_id != track_id {
            continue;
        }
        if let Some(limit) = discard_before {
            let end = packet.pts.get().max(0) as u64 + packet.dur.get();
            if end <= limit {
                continue;
            }
            discard_before = None;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => append(
                &mut spare,
                &mut scratch,
                &decoded,
                channels,
                packet.trim_start.get(),
                packet.trim_end.get(),
            ),
            // A damaged frame in the middle of a song is worth skipping, not stopping for: the
            // alternative is that one bad packet silences the rest of the recording.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn ticks_for_ms(base: TimeBase, ms: u32) -> u64 {
    u64::from(ms)
        .saturating_mul(u64::from(base.denom.get()))
        .checked_div(u64::from(base.numer.get()).saturating_mul(1000))
        .unwrap_or(0)
}

/// Converts one decoded buffer to interleaved stereo and appends it, honoring the encoder trims.
fn append(
    spare: &mut Vec<f32>,
    scratch: &mut Vec<f32>,
    decoded: &GenericAudioBufferRef<'_>,
    channels: usize,
    trim_start: u64,
    trim_end: u64,
) {
    scratch.clear();
    decoded.copy_to_vec_interleaved(scratch);

    let frames = scratch.len() / channels;
    let from = usize::try_from(trim_start).unwrap_or(0).min(frames);
    let to = frames
        .saturating_sub(usize::try_from(trim_end).unwrap_or(0))
        .max(from);

    spare.reserve((to - from) * FEED_CHANNELS);
    for frame in scratch[from * channels..to * channels].chunks_exact(channels) {
        // The feed is stereo, always. A mono file is duplicated; anything wider takes its first two
        // channels, which is not a downmix and does not pretend to be — the corpus is uniformly
        // stereo, and inventing a matrix for a case that does not occur is how a wrong one ships.
        let (left, right) = match channels {
            1 => (frame[0], frame[0]),
            _ => (frame[0], frame[1]),
        };
        spare.push(left);
        spare.push(right);
    }
}
