//! The shape every video song is normalized to at packaging time, and the re-encode that gets it
//! there.
//!
//! # Why normalize at all
//!
//! So the appliance's decoder only ever meets one thing instead of whatever codec and container mix
//! a download happened to arrive in. That is the same instinct that put melody detection and the
//! suitability score at packaging time — the machine reads a fact rather than coping at run time —
//! and it costs nothing, because whatever machine did the downloading already has ffmpeg.
//!
//! # Why the check comes first
//!
//! Most files are already in the profile. A yt-dlp download asked for AVC and AAC arrives as H.264
//! in yuv420p with AAC stereo, which is exactly what is wanted, and re-encoding it would spend an
//! hour of CPU to produce a slightly worse picture. So [`Profile::check`] runs first and an empty
//! result means *copy the bytes*.
//!
//! # Two kinds of finding, and the difference matters
//!
//! [`Severity::Blocking`] means the machine cannot play the file at all. There is exactly one such
//! thing, and it is not obvious from the outside: the **pixel format**. `km-video` copies three
//! planes with the chroma at half height and contains no `swscale`, so anything that is not 8-bit
//! planar 4:2:0 draws wrongly — which is why that crate now refuses it outright.
//!
//! [`Severity::Preference`] means the file plays, but is outside the normalized shape: VP9 instead
//! of H.264, 4K instead of 1080p, 60 fps instead of 30. Worth re-encoding for predictability and
//! disk, and never worth *refusing* over.
//!
//! Everything about the **audio** is a preference, and that is a deliberate consequence of how
//! `km-video` decodes: it runs a `swresample` context to interleaved stereo `f32`, so every codec,
//! sample rate and channel count already plays. The profile therefore says nothing about the sample
//! rate at all — the first real song packaged here is 44.1 kHz, and re-encoding it to 48 would
//! degrade it to fix a problem that does not exist.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use km_video::VideoInfo;

/// The one shape a packaged video song is stored in.
///
/// A constant rather than a setting. The point of normalizing is that there is one answer, and a
/// knob here would mean the appliance meets whatever each packager chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// The video codec, spelled as [`km_video::VideoInfo::video_codec`] spells it.
    pub video_codec: &'static str,
    /// Widest picture kept without scaling.
    pub max_width: u32,
    /// Tallest picture kept without scaling.
    pub max_height: u32,
    /// Fastest frame rate kept, in the milli-fps [`km_video::VideoInfo`] reports.
    pub max_frame_rate_milli: u32,
    /// The audio codec.
    pub audio_codec: &'static str,
    /// The container, by file extension.
    pub container: &'static str,
}

/// H.264 in 8-bit 4:2:0, at most 1080p30, with AAC, in MP4.
///
/// 1080p because that is what the source material is and the appliance's Kaby Lake decodes it many
/// times faster than real time. 30 fps because a karaoke video is a caption over a picture and 60
/// buys nothing for twice the bitrate.
///
/// **The frame-rate ceiling is compared in milli-fps**, so the ubiquitous 29.97 — which arrives as
/// 29970 — passes a 30 fps test instead of failing it by a rounding error.
pub const DEFAULT: Profile = Profile {
    video_codec: "h264",
    max_width: 1920,
    max_height: 1080,
    max_frame_rate_milli: 30_000,
    audio_codec: "aac",
    container: "mp4",
};

/// How much a mismatch matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The machine cannot play this file. Only the pixel format can be this.
    Blocking,
    /// It plays, but it is not the shape packaging aims for.
    Preference,
}

/// One way a file falls outside the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// Whether this stops the file working or merely makes it irregular.
    pub severity: Severity,
    /// What is wrong, in the form `found, wanted` — written to be shown to a person deciding
    /// whether to spend twenty minutes re-encoding.
    pub detail: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl Profile {
    /// Everything about `info` that falls outside this profile.
    ///
    /// An empty result means the file is already in profile and packaging should copy its bytes.
    /// `path` is read only for its extension, which is the only thing that says what container the
    /// file is in.
    #[must_use]
    pub fn check(&self, info: &VideoInfo, path: &Path) -> Vec<Mismatch> {
        let mut found = Vec::new();

        // The one thing that stops a file working. Asked of `km-video` rather than compared here, so
        // that what can be drawn is recorded in the crate that does the drawing.
        if !km_video::supports_pixel_format(&info.pixel_format) {
            found.push(Mismatch {
                severity: Severity::Blocking,
                detail: format!(
                    "{} video, and only 8-bit planar 4:2:0 can be drawn",
                    info.pixel_format
                ),
            });
        }

        if info.video_codec != self.video_codec {
            found.push(Mismatch {
                severity: Severity::Preference,
                detail: format!(
                    "{} video, wanted {}",
                    info.video_codec.to_uppercase(),
                    self.video_codec.to_uppercase()
                ),
            });
        }
        if info.width > self.max_width || info.height > self.max_height {
            found.push(Mismatch {
                severity: Severity::Preference,
                detail: format!(
                    "{}x{}, wanted at most {}x{}",
                    info.width, info.height, self.max_width, self.max_height
                ),
            });
        }
        if info.frame_rate_milli > self.max_frame_rate_milli {
            found.push(Mismatch {
                severity: Severity::Preference,
                detail: format!(
                    "{:.3} fps, wanted at most {}",
                    f64::from(info.frame_rate_milli) / 1000.0,
                    self.max_frame_rate_milli / 1000
                ),
            });
        }
        if info.audio_codec != self.audio_codec {
            found.push(Mismatch {
                severity: Severity::Preference,
                detail: format!(
                    "{} audio, wanted {}",
                    info.audio_codec.to_uppercase(),
                    self.audio_codec.to_uppercase()
                ),
            });
        }
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extension != self.container {
            found.push(Mismatch {
                severity: Severity::Preference,
                detail: format!("a .{extension} container, wanted .{}", self.container),
            });
        }

        found
    }
}

/// Whether any mismatch in the list stops the file being playable.
#[must_use]
pub fn is_blocking(mismatches: &[Mismatch]) -> bool {
    mismatches.iter().any(|m| m.severity == Severity::Blocking)
}

/// The H.264 encoders this tool will use, best first.
///
/// **`libx264` is not always there, and that surprised this project once already.** x264 is GPL, so
/// an LGPL ffmpeg build — which is what the development machine deliberately runs, and what the
/// notes in `CLAUDE.local.md` explain the reasoning for — ships without it. Such a build carries
/// Cisco's `libopenh264` instead, which is BSD-licensed and perfectly adequate here.
///
/// Hardware encoders (`h264_nvenc`, `h264_qsv`, `h264_amf`) are deliberately **not** in this list
/// even though an LGPL build lists all three. `ffmpeg -encoders` reports what was compiled in, not
/// what the machine's GPU and driver can actually do, so choosing one automatically turns a missing
/// graphics card into a failure in the middle of a batch. They also take different rate-control
/// options, so a single profile could not aim at the same quality through them.
const H264_ENCODERS: [&str; 2] = ["libx264", "libopenh264"];

/// Which ffmpeg binary to run.
///
/// Whatever is on `PATH`. Not configurable and not bundled: the packaging machine is somebody's
/// desktop and already has one, and shelling out to the user's own ffmpeg is also what keeps this
/// project's own linking story unchanged — nothing new is linked or distributed.
const FFMPEG: &str = "ffmpeg";

/// Start a child process without giving it a console window of its own.
///
/// **Not cosmetic, and not only about this crate.** `km-package-builder` is a GUI-subsystem
/// executable on Windows once it has a window, so it has no console for a child to inherit and every
/// ffmpeg Windows would otherwise hand one — one black window per file, through a batch that may be
/// hundreds of them. Both call sites already capture what ffmpeg says, so the window was never
/// showing anybody anything. `km-pack` run from a terminal is unaffected: this hides a console that
/// would have been *created*, never the one that is already there.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Applies [`CREATE_NO_WINDOW`], and is a no-op everywhere else.
fn without_a_console_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// What the local ffmpeg can do, looked up once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoders {
    /// The H.264 encoder to use, the first of [`H264_ENCODERS`] this ffmpeg has.
    pub h264: &'static str,
}

/// Finds ffmpeg and asks it what it can encode.
///
/// Fails with a message naming what is missing, rather than letting a re-encode die halfway with
/// ffmpeg's own wording. The two failures are genuinely different — no ffmpeg at all is "install
/// one", an ffmpeg with no H.264 encoder is "this is an unusual build" — so they say different
/// things.
pub fn encoders() -> Result<Encoders> {
    let output = without_a_console_window(&mut Command::new(FFMPEG))
        .args(["-hide_banner", "-loglevel", "error", "-encoders"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| {
            format!("could not run `{FFMPEG}` — video packaging needs ffmpeg on the PATH")
        })?;
    let listing = String::from_utf8_lossy(&output.stdout);

    // The listing is one encoder per line with a capability column first, so the name is matched
    // surrounded by spaces rather than merely contained: `libopenh264` must not match a line that
    // only mentions it in its description.
    let h264 = H264_ENCODERS.into_iter().find(|name| {
        listing
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    });

    match h264 {
        Some(h264) => Ok(Encoders { h264 }),
        None => bail!(
            "this ffmpeg has none of {} — it cannot encode H.264, so videos outside the profile \
             cannot be re-encoded",
            H264_ENCODERS.join(" or ")
        ),
    }
}

/// How far a re-encode has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Milliseconds of the source encoded so far.
    pub done_ms: u32,
    /// Milliseconds in the source altogether, from the probe.
    pub total_ms: u32,
}

impl Progress {
    /// How far along, 0 to 100. Zero when the duration is unknown, rather than a division by zero.
    #[must_use]
    pub fn percent(self) -> u8 {
        if self.total_ms == 0 {
            return 0;
        }
        let pct = u64::from(self.done_ms) * 100 / u64::from(self.total_ms);
        u8::try_from(pct.min(100)).unwrap_or(100)
    }
}

/// Re-encodes `source` into `destination` in this profile.
///
/// `info` is the source's probe, which is what decides whether scaling and frame-rate conversion are
/// needed at all — neither filter is applied to a file already within the ceiling, because scaling a
/// picture to its own size is a lossy no-op.
///
/// `on_progress` is called as ffmpeg reports, so a caller can show a bar during what is minutes of
/// work on a long song. It is called from this thread, between reads.
pub fn transcode(
    source: &Path,
    destination: &Path,
    profile: &Profile,
    info: &VideoInfo,
    encoders: &Encoders,
    mut on_progress: impl FnMut(Progress),
) -> Result<()> {
    let mut command = Command::new(FFMPEG);
    without_a_console_window(&mut command);
    command.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"]);
    command.arg("-i").arg(source);

    if let Some(filter) = video_filter(profile, info) {
        command.args(["-vf", &filter]);
    }

    command.args(["-c:v", encoders.h264]);
    // Rate control differs between the two encoders and there is no common spelling. x264's CRF is
    // quality-targeted and the right tool; openh264 has no equivalent, so it gets a bitrate chosen
    // to look similar at 1080p.
    if encoders.h264 == "libx264" {
        command.args(["-preset", "medium", "-crf", "20"]);
    } else {
        command.args(["-b:v", "4M"]);
    }
    // Named explicitly rather than left to the encoder's default. This is the one setting the
    // machine cannot cope without, so it does not get to be implied.
    command.args(["-pix_fmt", "yuv420p"]);
    command.args(["-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
    // Puts the index at the front, so the machine can start playing without reading to the end.
    command.args(["-movflags", "+faststart"]);
    // The container, said out loud rather than inferred from the output's name. ffmpeg chooses a
    // muxer by file extension, and this writes through a temporary `.part` name — so leaving it
    // implicit fails with `Unable to choose an output format`, which reads like a broken input
    // rather than a naming detail. Found by running it.
    command.args(["-f", profile.container]);
    // Progress on stdout, so it can be read without competing with ffmpeg's diagnostics.
    command.args(["-progress", "pipe:1"]);
    command.arg(destination);

    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("could not run `{FFMPEG}`"))?;

    // Drained on a thread of its own. With `-loglevel error` there is very little of it, but a
    // stderr pipe that fills while nothing reads it deadlocks the child, and "the re-encode hangs on
    // some files" is not a bug worth discovering later.
    let stderr = child.stderr.take();
    let drain = std::thread::spawn(move || {
        let mut buffer = String::new();
        if let Some(mut stderr) = stderr {
            use std::io::Read;
            let _ = stderr.read_to_string(&mut buffer);
        }
        buffer
    });

    if let Some(stdout) = child.stdout.take() {
        use std::io::{BufRead, BufReader};
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(done_ms) = progress_line_ms(&line) {
                on_progress(Progress {
                    done_ms,
                    total_ms: info.duration_ms,
                });
            }
        }
    }

    let status = child.wait().context("waiting for ffmpeg")?;
    let errors = drain.join().unwrap_or_default();
    if !status.success() {
        let detail = errors.trim();
        let detail = if detail.is_empty() {
            format!("ffmpeg exited with {status}")
        } else {
            detail.to_owned()
        };
        bail!("re-encoding {} failed: {detail}", source.display());
    }

    on_progress(Progress {
        done_ms: info.duration_ms,
        total_ms: info.duration_ms,
    });
    Ok(())
}

/// The `-vf` chain, or `None` when the source is already within every ceiling.
///
/// Built from the probe rather than written once and always applied, because `scale` and `fps` are
/// not free: scaling a picture to the size it already is still resamples it, and asking for 30 fps
/// from a 25 fps source *invents* frames.
fn video_filter(profile: &Profile, info: &VideoInfo) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if info.width > profile.max_width || info.height > profile.max_height {
        // `decrease` keeps the aspect ratio and only ever shrinks; `force_divisible_by=2` keeps both
        // sides even, which 4:2:0 requires and which an odd source height would otherwise break.
        parts.push(format!(
            "scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=decrease:\
             force_divisible_by=2",
            w = profile.max_width,
            h = profile.max_height
        ));
    }
    if info.frame_rate_milli > profile.max_frame_rate_milli {
        parts.push(format!("fps={}", profile.max_frame_rate_milli / 1000));
    }

    (!parts.is_empty()).then(|| parts.join(","))
}

/// Reads one `-progress` line, returning milliseconds encoded when it carries them.
///
/// **`out_time_ms` is a misnomer in ffmpeg itself**: it has always carried *microseconds*, and the
/// correctly named `out_time_us` was added beside it later. Both are read, both are divided by a
/// thousand, and a build that emits only the old one still reports honestly.
fn progress_line_ms(line: &str) -> Option<u32> {
    let (key, value) = line.split_once('=')?;
    if !matches!(key.trim(), "out_time_us" | "out_time_ms") {
        return None;
    }
    let micros: u64 = value.trim().parse().ok()?;
    u32::try_from(micros / 1_000).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real sample song's shape: what a yt-dlp download asked for AVC and AAC actually arrives
    /// as. Written down here because the whole probe-first design rests on this being in profile.
    fn in_profile() -> VideoInfo {
        VideoInfo {
            duration_ms: 273_020,
            width: 1920,
            height: 1080,
            frame_rate_milli: 29_970,
            audio_sample_rate: 44_100,
            video_codec: "h264".to_owned(),
            audio_codec: "aac".to_owned(),
            pixel_format: "yuv420p".to_owned(),
            audio_channels: 2,
            // What a download tagged with yt-dlp's `--embed-metadata` writes into the container,
            // and what one run without it leaves absent. The profile reads neither — they are
            // carried here so the fixture stays the shape of a real download rather than a subset
            // of one.
            title: Some("Aeng Moo Sae".to_owned()),
            artist: Some("Howl".to_owned()),
        }
    }

    #[test]
    fn the_real_download_needs_no_re_encoding() {
        let found = DEFAULT.check(&in_profile(), Path::new("Howl - Aeng Moo Sae.mp4"));
        assert!(found.is_empty(), "wanted nothing to fix, got {found:?}");
    }

    /// 29.97 is what nearly every downloaded video reports, and comparing it as whole frames per
    /// second would round it to 30 or truncate it to 29 depending on which way the arithmetic fell.
    #[test]
    fn twenty_nine_ninety_seven_is_within_a_thirty_fps_ceiling() {
        let mut info = in_profile();
        info.frame_rate_milli = 29_970;
        assert!(DEFAULT.check(&info, Path::new("a.mp4")).is_empty());

        info.frame_rate_milli = 60_000;
        let found = DEFAULT.check(&info, Path::new("a.mp4"));
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].severity,
            Severity::Preference,
            "60 fps still plays"
        );
    }

    /// The distinction the whole module is built around: one finding stops the file working and the
    /// rest do not.
    #[test]
    fn only_the_pixel_format_blocks() {
        let mut info = in_profile();
        info.pixel_format = "yuv444p".to_owned();
        let found = DEFAULT.check(&info, Path::new("a.mp4"));
        assert!(is_blocking(&found));

        let mut info = in_profile();
        info.video_codec = "vp9".to_owned();
        info.audio_codec = "opus".to_owned();
        info.width = 3840;
        info.height = 2160;
        let found = DEFAULT.check(&info, Path::new("a.webm"));
        assert_eq!(found.len(), 4, "codec, size, audio codec, container");
        assert!(!is_blocking(&found), "all of that still plays");
    }

    /// Audio says nothing about the sample rate on purpose: the decoder resamples, and the first
    /// real song packaged here is 44.1 kHz.
    #[test]
    fn an_odd_sample_rate_is_not_a_mismatch() {
        let mut info = in_profile();
        info.audio_sample_rate = 22_050;
        info.audio_channels = 1;
        assert!(DEFAULT.check(&info, Path::new("a.mp4")).is_empty());
    }

    #[test]
    fn a_file_already_within_the_ceilings_gets_no_filters() {
        assert_eq!(video_filter(&DEFAULT, &in_profile()), None);
    }

    #[test]
    fn an_oversized_source_is_scaled_and_a_fast_one_is_slowed() {
        let mut info = in_profile();
        info.width = 3840;
        info.height = 2160;
        info.frame_rate_milli = 60_000;
        let filter = video_filter(&DEFAULT, &info).expect("both filters apply");
        assert!(filter.contains("scale="), "{filter}");
        assert!(
            filter.contains("force_original_aspect_ratio=decrease"),
            "{filter}"
        );
        assert!(filter.ends_with("fps=30"), "{filter}");
    }

    #[test]
    fn progress_reads_microseconds_under_either_name() {
        assert_eq!(progress_line_ms("out_time_us=2500000"), Some(2_500));
        assert_eq!(progress_line_ms("out_time_ms=2500000"), Some(2_500));
        assert_eq!(progress_line_ms("frame=42"), None);
        assert_eq!(progress_line_ms("out_time_us=N/A"), None);
    }

    #[test]
    fn percent_survives_an_unknown_duration() {
        assert_eq!(
            Progress {
                done_ms: 10,
                total_ms: 0
            }
            .percent(),
            0
        );
        assert_eq!(
            Progress {
                done_ms: 50,
                total_ms: 100
            }
            .percent(),
            50
        );
        assert_eq!(
            Progress {
                done_ms: 200,
                total_ms: 100
            }
            .percent(),
            100,
            "a container that under-reports its duration must not exceed 100"
        );
    }
}
