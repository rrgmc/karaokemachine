//! Rendering a song to a buffer or a WAV file, with no audio device.
//!
//! Two uses. It makes the render path testable in CI, where there is neither a sound card nor a
//! SoundFont. And it gives a way to hear what the engine produces without setting up playback —
//! useful for checking a real file, and for the `render_wav` example.

use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use km_song::Song;

use crate::player::{Player, PlayerEvent};
use crate::sequencer::PlaybackSettings;
use crate::source::AudioSource;

/// How to render offline.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    /// Playback settings to apply.
    pub settings: PlaybackSettings,
    /// Music volume, 0.0 to 1.0.
    pub volume: f32,
    /// Extra time rendered after the last event, so reverb tails are not cut off.
    pub tail_ms: u32,
    /// Refuse to render longer than this, so a file with an absurd duration cannot fill a disk.
    pub max_ms: u32,
    /// Corrections to apply to the song's own events.
    ///
    /// Empty by default, so a render says what the file says. This is what lets one command render
    /// the same song with and without a fix and hand somebody the two to compare.
    pub fixes: km_fixes::ChannelFixes,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            settings: PlaybackSettings::default(),
            volume: 1.0,
            tail_ms: 2_000,
            max_ms: 15 * 60 * 1_000,
            fixes: km_fixes::ChannelFixes::default(),
        }
    }
}

/// Interleaved stereo audio and how it was produced.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// Interleaved stereo samples.
    pub samples: Vec<f32>,
    /// Sample rate.
    pub sample_rate: u32,
    /// Whether the song reached its own end rather than hitting the length limit.
    pub reached_end: bool,
}

impl Rendered {
    /// Number of stereo frames.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Duration in milliseconds.
    pub fn duration_ms(&self) -> u32 {
        let frames = self.frames() as u64;
        u32::try_from(frames * 1_000 / u64::from(self.sample_rate.max(1))).unwrap_or(u32::MAX)
    }

    /// Largest absolute sample value, for checking a render is neither silent nor clipping.
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
    }

    /// Whether anything at all was produced.
    pub fn is_silent(&self) -> bool {
        self.peak() == 0.0
    }
}

/// Renders a whole song through any audio source.
pub fn render<S: AudioSource>(
    source: S,
    song: Arc<Song>,
    melody_channel: Option<u8>,
    options: &RenderOptions,
) -> Rendered {
    render_keeping_source(source, song, melody_channel, options).0
}

/// Renders a whole song and hands the source back, for a caller that wants to inspect it after.
///
/// A synthesizer's channel state is what says whether a file's setup was acted on — a pitch bend
/// range of 12 where the file asked for 12 — and no amount of looking at the samples will say it.
pub fn render_keeping_source<S: AudioSource>(
    source: S,
    song: Arc<Song>,
    melody_channel: Option<u8>,
    options: &RenderOptions,
) -> (Rendered, S) {
    let sample_rate = source.sample_rate();
    let mut player = Player::new(source);
    player.set_transpose(options.settings.transpose);
    player.set_tempo_ratio(options.settings.tempo_ratio);
    player.set_music_volume(options.volume);
    player.load(song, melody_channel, options.fixes);
    player.set_melody_enabled(options.settings.melody_enabled);
    player.play();

    let chunk_frames = 1_024usize;
    let max_frames = (u64::from(options.max_ms) * u64::from(sample_rate) / 1_000) as usize;
    let tail_frames = (u64::from(options.tail_ms) * u64::from(sample_rate) / 1_000) as usize;

    let mut samples: Vec<f32> = Vec::new();
    let mut chunk = vec![0.0f32; chunk_frames * 2];
    let mut reached_end = false;
    let mut tail_remaining = None;

    while samples.len() / 2 < max_frames {
        if let Some(PlayerEvent::SongEnded) = player.fill(&mut chunk, 2) {
            reached_end = true;
            tail_remaining = Some(tail_frames);
        }
        samples.extend_from_slice(&chunk);

        if let Some(remaining) = &mut tail_remaining {
            *remaining = remaining.saturating_sub(chunk_frames);
            if *remaining == 0 {
                break;
            }
        }
    }

    (
        Rendered {
            samples,
            sample_rate,
            reached_end,
        },
        player.into_source(),
    )
}

/// Writes interleaved stereo `f32` samples as a 16-bit PCM WAV file.
///
/// Hand-written rather than pulling in a WAV crate: the header is 44 bytes and this is the only
/// audio file the project ever writes.
pub fn write_wav(path: impl AsRef<Path>, rendered: &Rendered) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut out = io::BufWriter::new(file);

    const CHANNELS: u16 = 2;
    const BITS: u16 = 16;
    let sample_rate = rendered.sample_rate;
    let byte_rate = sample_rate * u32::from(CHANNELS) * u32::from(BITS / 8);
    let data_len = u32::try_from(rendered.samples.len() * 2).unwrap_or(u32::MAX);

    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVE")?;

    out.write_all(b"fmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    // 1 = uncompressed PCM.
    out.write_all(&1u16.to_le_bytes())?;
    out.write_all(&CHANNELS.to_le_bytes())?;
    out.write_all(&sample_rate.to_le_bytes())?;
    out.write_all(&byte_rate.to_le_bytes())?;
    out.write_all(&(CHANNELS * BITS / 8).to_le_bytes())?;
    out.write_all(&BITS.to_le_bytes())?;

    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for sample in &rendered.samples {
        // Clamp before converting: a sample above full scale would wrap to the opposite polarity
        // and produce a loud click rather than a quiet distortion.
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * f32::from(i16::MAX)) as i16;
        out.write_all(&value.to_le_bytes())?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, testing};

    use super::*;
    use crate::source::TestToneSource;

    fn song(bytes: &[u8]) -> Arc<Song> {
        Arc::new(Song::parse(bytes, &ParseOptions::default()).expect("fixture parses"))
    }

    fn render_fixture(bytes: &[u8], options: &RenderOptions) -> Rendered {
        render(TestToneSource::new(22_050), song(bytes), None, options)
    }

    #[test]
    fn a_song_renders_to_audible_audio_of_about_the_right_length() {
        let rendered = render_fixture(&testing::soft_karaoke(), &RenderOptions::default());
        assert!(rendered.reached_end, "the song should run to its end");
        assert!(!rendered.is_silent(), "the render should be audible");
        // The fixture is 4 seconds plus a 2 second tail.
        assert!(
            (5_000..=7_000).contains(&rendered.duration_ms()),
            "unexpected length {} ms",
            rendered.duration_ms()
        );
    }

    #[test]
    fn rendering_never_clips() {
        let rendered = render_fixture(&testing::high_quality_song(), &RenderOptions::default());
        assert!(rendered.peak() <= 1.0, "peak was {}", rendered.peak());
    }

    #[test]
    fn a_faster_tempo_produces_a_shorter_render() {
        let mut fast = RenderOptions::default();
        fast.settings.tempo_ratio = 1.25;
        let normal = render_fixture(&testing::high_quality_song(), &RenderOptions::default());
        let quick = render_fixture(&testing::high_quality_song(), &fast);
        assert!(
            quick.duration_ms() < normal.duration_ms(),
            "{} should be shorter than {}",
            quick.duration_ms(),
            normal.duration_ms()
        );
    }

    #[test]
    fn the_length_limit_is_respected() {
        let options = RenderOptions {
            max_ms: 500,
            ..Default::default()
        };
        let rendered = render_fixture(&testing::high_quality_song(), &options);
        assert!(!rendered.reached_end, "the limit should have stopped it");
        assert!(
            rendered.duration_ms() <= 600,
            "got {} ms",
            rendered.duration_ms()
        );
    }

    #[test]
    fn an_instrumental_with_no_lyrics_still_renders() {
        let rendered = render_fixture(&testing::instrumental(), &RenderOptions::default());
        assert!(!rendered.is_silent());
    }

    #[test]
    fn silence_is_rendered_at_zero_volume() {
        let options = RenderOptions {
            volume: 0.0,
            ..Default::default()
        };
        let rendered = render_fixture(&testing::soft_karaoke(), &options);
        assert!(rendered.is_silent());
        assert!(rendered.frames() > 0, "silent, but still the right length");
    }

    #[test]
    fn a_wav_file_is_written_with_a_valid_header() {
        let rendered = render_fixture(&testing::soft_karaoke(), &RenderOptions::default());
        let dir = std::env::temp_dir().join("km-audio-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("render.wav");

        write_wav(&path, &rendered).expect("wav should be written");
        let bytes = std::fs::read(&path).expect("read back");

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[36..40], b"data");
        // Header plus two bytes per sample.
        assert_eq!(bytes.len(), 44 + rendered.samples.len() * 2);
        // The declared data length must match what was written.
        let declared = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(declared as usize, rendered.samples.len() * 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn out_of_range_samples_are_clamped_rather_than_wrapping() {
        // A sample above full scale would wrap to the opposite polarity and click loudly.
        let rendered = Rendered {
            samples: vec![2.0, -2.0, 0.5, -0.5],
            sample_rate: 44_100,
            reached_end: true,
        };
        let dir = std::env::temp_dir().join("km-audio-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("clamp.wav");
        write_wav(&path, &rendered).expect("write");
        let bytes = std::fs::read(&path).expect("read");

        let sample =
            |index: usize| i16::from_le_bytes([bytes[44 + index * 2], bytes[44 + index * 2 + 1]]);
        assert_eq!(sample(0), i16::MAX);
        assert_eq!(sample(1), -i16::MAX);
        assert!(sample(2) > 0 && sample(3) < 0);

        let _ = std::fs::remove_file(&path);
    }
}
