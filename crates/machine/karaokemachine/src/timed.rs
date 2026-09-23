//! UltraStar and LRC songs, loaded and kept alive.
//!
//! The counterpart of [`crate::cdg`], and the same shape for the same reason. The audio is an MP3
//! decoded by `km-cdg` exactly as an MP3+G song's is; the words are a lyric timeline the machine
//! draws over the wallpaper, as it draws a MIDI song's. The two kinds differ only in the file the
//! package builder read the timeline from. See `UltraStar as a song source` and `LRC as a song
//! source` in `docs/decisions/song-sources.md`.

use std::sync::Arc;

use km_audio::TrackPlayer;
use km_song::{LyricTimeline, Song};

/// A loaded UltraStar or LRC song: the decoder to keep running, and the words to draw.
///
/// Dropping this stops the decoder thread and waits for it, as dropping a
/// [`CdgSong`](crate::cdg::CdgSong) does, so it must be dropped on the control thread.
#[derive(Debug)]
pub struct TimedSong {
    /// Held only to keep the decoder running. Dropping it is how playback stops.
    _audio: km_cdg::AudioReader,
    /// The timeline, in a `Song` with a millisecond tempo map and nothing to play, because the lyric
    /// view, the lyric-line events and the lyrics endpoint all read one.
    song: Arc<Song>,
}

impl TimedSong {
    /// Starts decoding the audio of a packaged UltraStar or LRC song.
    ///
    /// The length is not measured here, for the reason
    /// [`CdgSong::open_from`](crate::cdg::CdgSong::open_from) gives: the catalog row already has it.
    pub fn open_from<R: std::io::Read + std::io::Seek + Send + Sync + 'static>(
        audio: R,
        audio_name: &str,
        timeline: LyricTimeline,
        out_rate: u32,
    ) -> anyhow::Result<(Self, TrackPlayer)> {
        let (reader, feed) = km_cdg::open_audio_from(audio, audio_name)?;
        let track = TrackPlayer::new(feed, out_rate);
        Ok((
            Self {
                _audio: reader,
                song: Arc::new(km_song::recording::song_from_timeline(timeline)),
            },
            track,
        ))
    }

    /// The words, for the display and the lyric-line events.
    pub fn song(&self) -> &Arc<Song> {
        &self.song
    }
}
