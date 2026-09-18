//! MP3+G songs, loaded and kept alive.
//!
//! The counterpart of [`crate::video`], and deliberately the same shape so that `machine.rs` and
//! `display.rs` treat the two alike. **What is missing here is the whole point**: there is no
//! `AVAILABLE` const, no `#[cfg]` and no second `mod imp`, because nothing about this is optional.
//! `km-cdg` is pure Rust with no C dependency, so every build that can list an MP3+G song can also
//! play it — including a `--no-video` build and the Android one, where a video song cannot be played
//! at all. See the `MP3+G as a song source` decision in `docs/decisions/`.

use std::path::Path;
use std::sync::Arc;

use km_audio::TrackPlayer;

/// A loaded MP3+G song: everything that has to stay alive for it to keep playing.
///
/// Dropping this stops the decoder thread and waits for it, exactly as dropping a [`VideoSong`]
/// does, so it must be dropped on the control thread and never on the audio callback.
///
/// [`VideoSong`]: crate::video::VideoSong
#[derive(Debug)]
pub struct CdgSong {
    /// Held only to keep the decoder running. Dropping it is how playback stops.
    _audio: km_cdg::AudioReader,
    /// Shared with the display thread, which takes pictures from it once per drawn frame.
    frames: Arc<km_cdg::FrameReader>,
    /// Length in milliseconds, from the audio — never from the graphics.
    duration_ms: u32,
}

impl CdgSong {
    /// Opens both halves of an MP3+G song and starts decoding the audio.
    ///
    /// `out_rate` is the audio device's sample rate and is the *only* thing about the device that
    /// reaches this side; the decoder feeds samples at the file's own rate and [`TrackPlayer`]
    /// resamples them.
    ///
    /// `known_duration_ms` is the length from the catalog, when the song came from one. **Pass it
    /// whenever it is known**, because the alternative is measuring: an MP3's own header cannot be
    /// trusted (see the `An MP3+G song's length is its audio's, and is counted rather than read`
    /// decision), so the honest answer needs the whole file read, and doing that here would put a
    /// multi-megabyte read between pressing a number and hearing anything. Packaging has already
    /// paid that cost once and written the answer down. `None` — the loose-pair debug path, which
    /// has no manifest to ask — measures it now.
    pub fn open(
        audio: &Path,
        graphics: &Path,
        out_rate: u32,
        known_duration_ms: Option<u32>,
    ) -> anyhow::Result<(Self, TrackPlayer)> {
        let duration_ms = match known_duration_ms {
            Some(known) => known,
            None => km_cdg::probe_audio(audio)?.duration_ms,
        };
        let (reader, feed, frames) = km_cdg::open(audio, graphics)?;
        let track = TrackPlayer::new(feed, out_rate);
        Ok((
            Self {
                _audio: reader,
                frames: Arc::new(frames),
                duration_ms,
            },
            track,
        ))
    }

    /// The same, from a package: the audio as something seekable and the graphics as bytes.
    ///
    /// `duration_ms` is **not** optional here, and cannot be.
    /// Measuring one means walking every packet to the end of the audio, which would leave the
    /// reader at EOF with nothing left to decode — so the only sound version is to be told, and a
    /// packaged song always has the manifest's answer. The loose-pair path keeps [`Self::open`],
    /// which has no manifest and can afford to measure because it opens the file twice anyway.
    pub fn open_from<R: std::io::Read + std::io::Seek + Send + Sync + 'static>(
        audio: R,
        audio_name: &str,
        graphics: &[u8],
        graphics_name: &str,
        out_rate: u32,
        duration_ms: u32,
    ) -> anyhow::Result<(Self, TrackPlayer)> {
        let (reader, feed, frames) = km_cdg::open_from(audio, audio_name, graphics, graphics_name)?;
        let track = TrackPlayer::new(feed, out_rate);
        Ok((
            Self {
                _audio: reader,
                frames: Arc::new(frames),
                duration_ms,
            },
            track,
        ))
    }

    /// How long the audio says the song is.
    pub fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    /// The picture source, for the display thread.
    pub fn frames(&self) -> Arc<km_cdg::FrameReader> {
        Arc::clone(&self.frames)
    }
}
