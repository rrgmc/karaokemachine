//! Video songs, and the one place that knows whether this build can play them.
//!
//! Everything conditional about video lives here. The rest of `km-app` calls [`VideoSong::open`] and
//! either gets a song or gets an error saying why not — which is the same shape whether the reason is
//! a missing file, a broken container, or a build with the `video` feature turned off.
//!
//! That last case is deliberately not silent. A build without the feature still **catalogs,
//! searches and queues** video songs, because the catalog is a record of what the owner has rather
//! than of what this particular binary can do; it just cannot play them, and it says so. This is the
//! shape `km-app` already uses when no SoundFont is found and it falls back to a test tone: say it
//! out loud rather than behave mysteriously.

use std::path::Path;

use km_audio::TrackPlayer;

/// Whether this build can decode video at all.
///
/// Reported at startup beside the audio device, so an owner who wonders why a video song is being
/// skipped has already been told.
pub const AVAILABLE: bool = cfg!(feature = "video");

#[cfg(feature = "video")]
mod imp {
    use super::{Path, TrackPlayer};
    use std::sync::Arc;

    /// A loaded video song: everything that has to stay alive for it to keep playing.
    ///
    /// Dropping this stops the decoder thread and waits for it, so it must be dropped on the control
    /// thread and never on the audio callback. `km-audio`'s retirement queue is what keeps the
    /// audio half of the same song off that thread; this half never goes near it.
    #[derive(Debug)]
    pub struct VideoSong {
        /// Keeps the decoder running — dropping it is how playback stops — and names the song for
        /// the summary below.
        reader: km_video::MediaReader,
        /// Shared with the display thread, which takes pictures from it once per drawn frame.
        frames: Arc<km_video::FrameReader>,
        /// Length in milliseconds, from the probe.
        duration_ms: u32,
    }

    impl VideoSong {
        /// Opens a video file and starts decoding it.
        ///
        /// `out_rate` is the audio device's sample rate, and is the *only* thing about the device
        /// that reaches this side. The decoder itself never learns it: it feeds samples at the
        /// file's own rate and [`TrackPlayer`] resamples them.
        pub fn open(path: &Path, out_rate: u32) -> anyhow::Result<(Self, TrackPlayer)> {
            // The length comes back from the open itself. A song loaded from a package could take it
            // from the manifest, but a loose file auditioned through the debug path has none — and a
            // progress bar with no length cannot move. `km_video::open` measures the container it has
            // already opened, so nothing here opens one a second time to probe it.
            let (info, reader, feed, frames) = km_video::open(path)?;
            let duration_ms = info.duration_ms;
            let track = TrackPlayer::new(feed, out_rate);
            Ok((
                Self {
                    reader,
                    frames: Arc::new(frames),
                    duration_ms,
                },
                track,
            ))
        }

        /// The same, from anything seekable — which is how a packaged song arrives.
        ///
        /// Generic rather than naming `km_kmpkg::EntryWindow`, deliberately. `km-app` depends on
        /// `km-kmpkg` and could name it, but a bound keeps `--play` on a loose file and the
        /// catalog's packaged route on **one** call, and keeps the stub below honest about what it
        /// is refusing rather than about which type it was handed.
        pub fn open_from<R: std::io::Read + std::io::Seek + Send + 'static>(
            reader: R,
            name: &str,
            out_rate: u32,
        ) -> anyhow::Result<(Self, TrackPlayer)> {
            let (info, reader, feed, frames) = km_video::open_from(reader, name)?;
            let track = TrackPlayer::new(feed, out_rate);
            Ok((
                Self {
                    reader,
                    frames: Arc::new(frames),
                    duration_ms: info.duration_ms,
                },
                track,
            ))
        }

        /// How long the file says it is.
        pub fn duration_ms(&self) -> u32 {
            self.duration_ms
        }

        /// The frame source, for the display thread.
        pub fn frames(&self) -> Arc<km_video::FrameReader> {
            Arc::clone(&self.frames)
        }
    }

    /// Says so if the decoder could not hand pictures over fast enough, once, as the song ends.
    ///
    /// Here rather than in the machine because this is the one place every ending passes through —
    /// finished, skipped, stopped and shut down are four code paths and one `drop`.
    ///
    /// **`skipped` is deliberately not a reason to speak, and that cost a measurement to learn.**
    /// It was the gate when this was written, on the reasoning that a picture can only come due late
    /// if something was drawing and was late. That reasoning is wrong, and the counter runs at
    /// roughly **half a picture a second on a perfectly healthy song** — because the audio callback
    /// period is 117 ms on the appliance, so `position_ms` advances in 117 ms steps and about three
    /// and a half frames of a 30 fps video come due at each one. `take_frame_for` keeps the newest
    /// and skips the rest, for ever, on every video song. Gating on it would have warned about every
    /// video ever played. Three measured runs put it at 0.44, 0.53 and 0.65 a second while the
    /// audible fault varied from 871 ms to nothing, so it does not even track severity.
    ///
    /// `dropped` is the one worth a word: on a machine with a display the queue should never back
    /// up, so anything here is the display failing to take pictures rather than the decoder failing
    /// to make them. **A headless run fills it at the video's frame rate by design** and must not
    /// warn, which is why this asks whether anything was ever drawn.
    ///
    /// Both numbers are on the `--frame-stats` line for somebody who asked for them.
    impl Drop for VideoSong {
        fn drop(&mut self) {
            let counters = self.frames.counters();
            let dropped = counters.dropped();
            // `skipped` non-zero is the proof a display was attached: a headless run never takes a
            // frame at all, so it cannot have skipped one, and its `dropped` is expected.
            if dropped > 0 && counters.skipped() > 0 {
                tracing::warn!(
                    video = %self.reader.name(),
                    dropped,
                    skipped = counters.skipped(),
                    "video pictures were thrown away before anything could draw them"
                );
            }
        }
    }
}

#[cfg(not(feature = "video"))]
mod imp {
    use super::{Path, TrackPlayer};

    /// What to say when a video song is reached by a build that cannot play one.
    const UNAVAILABLE: &str =
        "this build cannot play video songs; it was built without the `video` feature";

    /// A loaded video song. Never constructed in a build without the `video` feature.
    ///
    /// It exists so that nothing outside this module needs `#[cfg]`: the machine has one code path
    /// for loading a song of either kind, and only [`VideoSong::open`] differs.
    #[derive(Debug)]
    pub struct VideoSong {}

    impl VideoSong {
        /// Always fails, with the reason.
        pub fn open(path: &Path, _out_rate: u32) -> anyhow::Result<(Self, TrackPlayer)> {
            Err(anyhow::anyhow!("{}: {UNAVAILABLE}", path.display()))
        }

        /// Always fails too, naming the entry rather than a path.
        ///
        /// The signature matches its twin above exactly, which is the only thing keeping the two
        /// `mod imp` blocks in step — nothing outside this module has a `#[cfg]` to notice a drift
        /// with, so it would show up as a `--no-default-features` build that will not compile.
        pub fn open_from<R: std::io::Read + std::io::Seek + Send + 'static>(
            _reader: R,
            name: &str,
            _out_rate: u32,
        ) -> anyhow::Result<(Self, TrackPlayer)> {
            Err(anyhow::anyhow!("{name}: {UNAVAILABLE}"))
        }

        /// Never reached: `open` never returns one of these, so nothing can ask.
        pub fn duration_ms(&self) -> u32 {
            0
        }
    }
}

pub use imp::VideoSong;
