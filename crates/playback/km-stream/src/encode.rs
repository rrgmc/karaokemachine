//! The encoder and the muxer: a drawn screen in, a rolling playlist out.
//!
//! **The whole of ffmpeg's part is here**, and it is the library rather than the program: the
//! `hls` muxer and every encoder below are in libavformat and libavcodec, which this workspace
//! already links for decoding. Driving the `ffmpeg` command instead would mean a child process to
//! supervise, two pipes to carry a picture and its sound into one muxer, and a program that has to
//! be on the machine — which is more moving parts, not fewer.
//!
//! # What is produced
//!
//! A directory holding `live.m3u8`, an fMP4 initialisation segment, and the segments the playlist
//! names. The muxer deletes segments as they fall out of the window, so the directory is bounded by
//! the playlist length and nothing has to sweep it.

use std::path::{Path, PathBuf};

use ff::{Dictionary, Rational, codec, encoder, format, frame, util};
use ffmpeg_next as ff;

use crate::pixels::{Plane, bgra_to_yuv420p};

/// The playlist a client opens. Named once, because the server serves it by this name.
pub const PLAYLIST: &str = "live.m3u8";

/// The encoder name that means "whichever software H.264 encoder this build has".
pub const AUTO_ENCODER: &str = "auto";

/// What [`AUTO_ENCODER`] tries, in order.
///
/// **Two names because ffmpeg implements no H.264 encoder of its own**, and which external one a
/// build carries is settled by whoever built it. An ffmpeg this project builds carries
/// `libopenh264`, the only H.264 encoder an LGPL configure line can have. A distribution's own
/// ffmpeg is built `--enable-gpl` and carries `libx264` instead, so a machine installed from a
/// package finds that one and no other.
///
/// **Only the default falls back.** A name somebody wrote down is used exactly as written, and is
/// reported if the build does not have it — which is what makes a hardware encoder testable by
/// setting one.
pub const SOFTWARE_ENCODERS: [&str; 2] = ["libopenh264", "libx264"];

/// What the stream looks like.
#[derive(Debug, Clone)]
pub struct Config {
    /// Frame size, which is also the size the screen is drawn at. Both must be even.
    pub width: u32,
    /// See [`Config::width`].
    pub height: u32,
    /// Frames a second.
    pub fps: u32,
    /// Video bits a second.
    pub bitrate: usize,
    /// Which encoder to use, by ffmpeg's own name for it.
    ///
    /// **A name rather than a choice made here**, so that a hardware encoder can be tried by
    /// setting one and measuring, without a change to this code. `h264_nvenc`, `h264_qsv`,
    /// `h264_amf` and `h264_mf` are the hardware ones a build may also have. A name the build does
    /// not have is reported when the stream is opened, which is the point at which somebody can
    /// still do something about it.
    ///
    /// [`AUTO_ENCODER`] is the one value that is not a name: it takes the first of
    /// [`SOFTWARE_ENCODERS`] the build has, because which of the two an ffmpeg carries is decided
    /// by whoever configured it rather than by anything here.
    pub encoder: String,
    /// Seconds of video in each segment.
    pub segment_seconds: u32,
    /// How many segments the playlist names at once.
    pub playlist_size: u32,
    /// Samples a second, per channel. The stream is always stereo.
    ///
    /// **What the machine renders at, not what a device asked for.** There is no device, so nothing
    /// else has an opinion and the rate is chosen here.
    pub sample_rate: u32,
    /// Audio bits a second.
    pub audio_bitrate: usize,
}

/// Channels in the stream, everywhere.
///
/// **Stereo, and not a setting.** The machine renders stereo, a karaoke mix is stereo, and a number
/// here that disagreed with what the caller hands over would be a silent channel or a crash rather
/// than a choice anybody wanted.
pub const CHANNELS: usize = 2;

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            // Generous on purpose. The clients are on the same house network, so there is no
            // reason to compress hard, and a high bitrate is what keeps one generation of
            // re-encoding away from anything a person can see.
            bitrate: 12_000_000,
            encoder: AUTO_ENCODER.to_owned(),
            segment_seconds: 2,
            playlist_size: 6,
            sample_rate: 48_000,
            // Transparent for a stereo mix, and a rounding error beside the picture beside it.
            audio_bitrate: 192_000,
        }
    }
}

/// Why a stream could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// The build has no encoder by that name.
    #[error(
        "this build of ffmpeg has no encoder called '{name}'; \
         `ffmpeg -encoders` lists the ones it does have"
    )]
    NoSuchEncoder {
        /// What was asked for.
        name: String,
    },
    /// The build has none of the software H.264 encoders.
    #[error(
        "this build of ffmpeg has no H.264 encoder: none of {tried} is in it. \
         Name a hardware one in the stream settings, or use a build that carries one"
    )]
    NoSoftwareEncoder {
        /// The names that were tried, for the message.
        tried: String,
    },
    /// The frame size cannot be encoded.
    #[error("a {width}x{height} frame cannot be encoded: 4:2:0 needs even width and height")]
    OddSize {
        /// Width asked for.
        width: u32,
        /// Height asked for.
        height: u32,
    },
    /// The directory the segments go in could not be prepared.
    #[error("preparing {}: {source}", path.display())]
    Directory {
        /// Where the segments were to go.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// ffmpeg refused something, carrying its own message.
    #[error("{what}: {source}")]
    Ffmpeg {
        /// The stage that failed, so the message says which call it was.
        what: &'static str,
        /// ffmpeg's own error.
        source: ff::Error,
    },
}

/// The encoder a [`Config::encoder`] asks for.
///
/// **A written name is taken literally and never falls back.** Somebody who set `h264_nvenc` and
/// gets software H.264 instead has been told their graphics card is working when it is not, so the
/// absence is reported. [`AUTO_ENCODER`] is the one value that searches, and it searches only
/// [`SOFTWARE_ENCODERS`].
fn find_encoder(name: &str) -> Result<ff::Codec, StreamError> {
    if name != AUTO_ENCODER {
        return encoder::find_by_name(name).ok_or_else(|| StreamError::NoSuchEncoder {
            name: name.to_owned(),
        });
    }
    SOFTWARE_ENCODERS
        .iter()
        .find_map(|candidate| encoder::find_by_name(candidate))
        .ok_or_else(|| StreamError::NoSoftwareEncoder {
            tried: SOFTWARE_ENCODERS.join(", "),
        })
}

/// The options an encoder opens with, by the name it resolved to.
///
/// **A frame leaves the encoder in the order it arrived, with nothing held back.** A segment is
/// published only once its last frame is out. So frames held back for lookahead delay every segment
/// by that much. At its default preset `libx264` holds about forty frames and reorders for B-frames.
/// `zerolatency` turns both off. `libopenh264` holds nothing back, and a hardware encoder takes
/// whatever its own defaults are.
fn encoder_options(codec_name: &str) -> Dictionary<'static> {
    let mut options = Dictionary::new();
    if codec_name == "libx264" {
        options.set("tune", "zerolatency");
    }
    options
}

/// Attaches the stage to an ffmpeg failure.
fn at(what: &'static str) -> impl FnOnce(ff::Error) -> StreamError {
    move |source| StreamError::Ffmpeg { what, source }
}

/// Closes the file `ffmpeg-next` opened for a muxer that does its own I/O.
///
/// **The HLS muxer is `AVFMT_NOFILE`**: it opens, writes and renames every file it produces, and
/// `AVFormatContext.pb` is documented to stay null for such a muxer. `format::output_as` opens one
/// anyway — it does not test the flag, where `format::output_to_stream` beside it does and refuses
/// outright — so the playlist is held open by this process while the muxer tries to rename its
/// freshly written copy onto that same name.
///
/// On Windows that rename fails, and nothing reports it: the segments are correct, the temporary
/// playlist beside them is correct, and `live.m3u8` stays empty, so every client fetches an empty
/// playlist and shows nothing. On a platform where renaming over an open file is permitted it
/// works, which is what makes this worth stating rather than leaving to be rediscovered.
///
/// `avio_closep` closes the handle and nulls the field, which is what the muxer expects to find and
/// what the context's own destructor tolerates — it passes `pb` to `avio_close`, and a null one is a
/// no-op there.
#[expect(
    unsafe_code,
    reason = "undoing an `avio_open` the binding makes for a muxer that must not have one"
)]
fn release_playlist_handle(output: &mut format::context::Output) {
    // SAFETY: `as_mut_ptr` hands back the context this function's caller owns and has not yet
    // written a header to, so nothing is reading `pb`. `avio_closep` is null-safe and leaves the
    // field null, which is the state an `AVFMT_NOFILE` muxer is specified to be given.
    unsafe {
        let context = output.as_mut_ptr();
        ff::ffi::avio_closep(&raw mut (*context).pb);
    }
}

/// Initializes ffmpeg once per process.
///
/// A `Once` of its own rather than one shared with `km-video`: a dependency edge between two
/// playback crates to reach one idempotent call would cost more than it saves, and `av_register`
/// has been safe to call from anywhere for years.
fn init() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ff::init();
        // ffmpeg's own logging goes to stderr behind our back, and an encoder is chattier than a
        // decoder. Warnings and worse only.
        ff::util::log::set_level(ff::util::log::Level::Warning);
    });
}

/// A picture and its sound, encoded into a directory of HLS segments.
///
/// **Both halves are numbered by what has been handed over rather than by a clock**, which is what
/// makes them agree: a video frame's time is the count of frames before it, an audio packet's is the
/// count of samples, and a caller that hands over one frame for every `sample_rate / fps` samples
/// has produced a stream in which the words are on the beat by arithmetic. Nothing here reads a
/// wall clock, so nothing here can drift against one.
pub struct Stream {
    output: format::context::Output,
    encoder: encoder::Video,
    stream_index: usize,
    encoder_time_base: Rational,
    stream_time_base: Rational,
    next_pts: i64,
    width: u32,
    height: u32,
    /// Converted planes, kept so a running stream allocates none per frame. See [`Stream::push`].
    luma: Vec<u8>,
    blue: Vec<u8>,
    red: Vec<u8>,
    audio: Audio,
}

/// The sound beside the picture.
struct Audio {
    encoder: encoder::Audio,
    stream_index: usize,
    time_base: Rational,
    stream_time_base: Rational,
    /// How many samples per channel one encoded packet holds.
    frame_samples: usize,
    /// Interleaved samples handed over but not yet making a whole encoder frame.
    ///
    /// **An encoder takes a fixed number of samples and the caller hands over whatever a video
    /// frame's worth is**, and the two do not divide: at 48 kHz and 30 fps a frame is 1600 samples
    /// against AAC's 1024. So what is left over waits here for the next handful.
    pending: Vec<f32>,
    /// Samples per channel already encoded, which is the next packet's presentation time.
    next_pts: i64,
}

impl Stream {
    /// Opens a stream writing its playlist and segments into `dir`.
    ///
    /// The directory is created if it is not there. Anything already in it belonging to an earlier
    /// run is left for the muxer to overwrite, because a playlist is rewritten from its first
    /// segment and a client that was watching has already been told the stream restarted.
    pub fn open(dir: &Path, config: &Config) -> Result<Self, StreamError> {
        if !config.width.is_multiple_of(2) || !config.height.is_multiple_of(2) {
            return Err(StreamError::OddSize {
                width: config.width,
                height: config.height,
            });
        }
        init();

        std::fs::create_dir_all(dir).map_err(|source| StreamError::Directory {
            path: dir.to_path_buf(),
            source,
        })?;

        let codec = find_encoder(&config.encoder)?;

        let mut output = format::output_as(&for_ffmpeg(&dir.join(PLAYLIST)), "hls")
            .map_err(at("opening the playlist"))?;
        release_playlist_handle(&mut output);

        // The time base is one tick per frame, so a frame's presentation time is its own number and
        // nothing has to be rescaled on the way in.
        let encoder_time_base = Rational::new(1, config.fps as i32);

        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(at("preparing the encoder"))?;
        video.set_width(config.width);
        video.set_height(config.height);
        video.set_format(format::Pixel::YUV420P);
        video.set_time_base(encoder_time_base);
        video.set_frame_rate(Some(Rational::new(config.fps as i32, 1)));
        video.set_bit_rate(config.bitrate);
        // A keyframe every segment, so the muxer can cut where it says it will. Without this the
        // segments run long and a client waiting for the next one waits past the duration the
        // playlist promised.
        video.set_gop(config.fps * config.segment_seconds);
        // Said out loud so the picture and its tag agree. `pixels` converts with the Rec. 709
        // matrix at limited range, and a stream tagged as anything else is shown washed out or
        // crushed — which looks like a display problem rather than a conversion one.
        video.set_colorspace(util::color::Space::BT709);
        video.set_color_range(util::color::Range::MPEG);
        // fMP4 carries the codec's own header in the initialisation segment rather than in every
        // frame, and the muxer says so through this flag.
        if output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER)
        {
            video.set_flags(codec::Flags::GLOBAL_HEADER);
        }

        let encoder = video.open_with(encoder_options(codec.name())).map_err(at(
            "opening the encoder; the settings asked for may be beyond what it supports",
        ))?;

        let mut stream = output
            .add_stream(codec)
            .map_err(at("adding the video stream"))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(encoder_time_base);
        let stream_index = stream.index();

        // **The native AAC encoder, named by codec rather than by setting.** Every ffmpeg carries
        // it and it needs no external library, so unlike the picture there is nothing for an owner
        // to choose between and nothing that can be absent.
        let aac = encoder::find(codec::Id::AAC).ok_or_else(|| StreamError::NoSuchEncoder {
            name: "aac".to_owned(),
        })?;
        let audio_time_base = Rational::new(1, config.sample_rate as i32);
        let mut audio = codec::context::Context::new_with_codec(aac)
            .encoder()
            .audio()
            .map_err(at("preparing the audio encoder"))?;
        audio.set_rate(config.sample_rate as i32);
        audio.set_channel_layout(util::channel_layout::ChannelLayout::STEREO);
        // Planar float, which is what this encoder takes. The machine renders interleaved, so
        // `push_audio` splits the channels apart on the way in.
        audio.set_format(format::Sample::F32(format::sample::Type::Planar));
        audio.set_bit_rate(config.audio_bitrate);
        audio.set_time_base(audio_time_base);
        if output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER)
        {
            audio.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let audio_encoder = audio
            .open_with(Dictionary::new())
            .map_err(at("opening the audio encoder"))?;
        let mut audio_stream = output
            .add_stream(aac)
            .map_err(at("adding the audio stream"))?;
        audio_stream.set_parameters(&audio_encoder);
        audio_stream.set_time_base(audio_time_base);
        let audio_index = audio_stream.index();
        // **Asked after opening, because the encoder decides it.** AAC takes 1024 samples a frame;
        // an encoder that said otherwise and was fed 1024 would produce packets nothing lines up
        // with.
        let frame_samples = audio_encoder.frame_size().max(1) as usize;

        output
            .write_header_with(hls_options(dir, config))
            .map_err(at("writing the playlist header"))?;

        // Read after the header is written, because a muxer may choose its own and everything
        // written afterwards has to be rescaled into whatever it settled on.
        let stream_time_base = output
            .stream(stream_index)
            .map_or(encoder_time_base, |s| s.time_base());
        let audio_stream_time_base = output
            .stream(audio_index)
            .map_or(audio_time_base, |s| s.time_base());

        Ok(Self {
            output,
            encoder,
            stream_index,
            encoder_time_base,
            stream_time_base,
            next_pts: 0,
            width: config.width,
            height: config.height,
            luma: Vec::new(),
            blue: Vec::new(),
            red: Vec::new(),
            audio: Audio {
                encoder: audio_encoder,
                stream_index: audio_index,
                time_base: audio_time_base,
                stream_time_base: audio_stream_time_base,
                frame_samples,
                pending: Vec::new(),
                next_pts: 0,
            },
        })
    }

    /// Encodes however much sound is ready, in interleaved stereo.
    ///
    /// **Whatever arrives, rather than a fixed amount.** An encoder frame is a fixed number of
    /// samples and a video frame's worth of sound is a different number, so what does not fill a
    /// frame waits for the next handful. A caller hands over one video frame for every
    /// `sample_rate / fps` samples and the two streams stay in step by counting alone.
    ///
    /// # Panics
    ///
    /// If `samples` does not hold whole stereo frames.
    pub fn push_audio(&mut self, samples: &[f32]) -> Result<(), StreamError> {
        assert!(
            samples.len().is_multiple_of(CHANNELS),
            "interleaved stereo comes in pairs; {} samples do not",
            samples.len()
        );
        self.audio.pending.extend_from_slice(samples);

        let per_frame = self.audio.frame_samples * CHANNELS;
        let mut taken = 0;
        while self.audio.pending.len() - taken >= per_frame {
            let block = &self.audio.pending[taken..taken + per_frame];
            let mut picture = frame::Audio::new(
                format::Sample::F32(format::sample::Type::Planar),
                self.audio.frame_samples,
                util::channel_layout::ChannelLayout::STEREO,
            );
            // Deinterleaved into the encoder's own planes. `plane_mut` may hand back more room than
            // was asked for, so the write is bounded by the sample count rather than by its length.
            for channel in 0..CHANNELS {
                let plane = picture.plane_mut::<f32>(channel);
                for (index, slot) in plane.iter_mut().take(self.audio.frame_samples).enumerate() {
                    *slot = block[index * CHANNELS + channel];
                }
            }
            picture.set_pts(Some(self.audio.next_pts));
            self.audio.next_pts += self.audio.frame_samples as i64;
            taken += per_frame;

            self.audio
                .encoder
                .send_frame(&picture)
                .map_err(at("handing sound to the encoder"))?;
            self.drain_audio()?;
        }
        self.audio.pending.drain(..taken);
        Ok(())
    }

    /// Moves whatever the audio encoder has finished into the muxer.
    fn drain_audio(&mut self) -> Result<(), StreamError> {
        let mut packet = codec::packet::Packet::empty();
        while self.audio.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.audio.stream_index);
            packet.rescale_ts(self.audio.time_base, self.audio.stream_time_base);
            packet
                .write_interleaved(&mut self.output)
                .map_err(at("writing sound into a segment"))?;
        }
        Ok(())
    }

    /// Encodes one drawn screen.
    ///
    /// `bgra` is the screen in memory order, which is what an `ARGB8888` surface holds. Frames are
    /// numbered in the order they arrive, so a caller that wants a steady rate hands one over per
    /// tick rather than telling this what time it is.
    pub fn push(&mut self, bgra: &[u8]) -> Result<(), StreamError> {
        let mut picture = frame::Video::new(format::Pixel::YUV420P, self.width, self.height);
        let (width, height) = (self.width as usize, self.height as usize);

        // **Converted into buffers this owns and then copied in, rather than written straight into
        // the frame.** A frame's three planes are reached one at a time through `data_mut`, and each
        // of those borrows the whole frame — so writing all three in one pass over the source is not
        // expressible against that API. Converting three times instead would read the screen three
        // times, where this reads it once and copies the result.
        //
        // The buffers belong to the stream so that a running stream allocates none of this per
        // frame. At 1080p a frame's planes are about three megabytes, and allocating and freeing
        // that thirty times a second is the churn that surfaces later as a stutter nobody can place.
        let strides = [picture.stride(0), picture.stride(1), picture.stride(2)];
        self.luma.resize(strides[0] * height, 0);
        self.blue.resize(strides[1] * height / 2, 0);
        self.red.resize(strides[2] * height / 2, 0);
        bgra_to_yuv420p(
            bgra,
            width,
            height,
            &mut Plane {
                bytes: &mut self.luma,
                stride: strides[0],
            },
            &mut Plane {
                bytes: &mut self.blue,
                stride: strides[1],
            },
            &mut Plane {
                bytes: &mut self.red,
                stride: strides[2],
            },
        );
        picture.data_mut(0)[..self.luma.len()].copy_from_slice(&self.luma);
        picture.data_mut(1)[..self.blue.len()].copy_from_slice(&self.blue);
        picture.data_mut(2)[..self.red.len()].copy_from_slice(&self.red);

        picture.set_pts(Some(self.next_pts));
        self.next_pts += 1;

        self.encoder
            .send_frame(&picture)
            .map_err(at("handing a frame to the encoder"))?;
        self.drain()
    }

    /// Flushes the encoder and closes the playlist.
    ///
    /// **Taking `self` by value is what makes the trailer unmissable.** A playlist without its
    /// closing tag leaves every client waiting for a segment that is never coming.
    pub fn finish(mut self) -> Result<(), StreamError> {
        self.encoder
            .send_eof()
            .map_err(at("telling the encoder there is no more"))?;
        self.drain()?;
        // **Whatever is left over is dropped rather than padded out.** A part-filled encoder frame
        // is a fraction of a hundredth of a second at the very end of a stream nobody is still
        // watching, and padding it with silence would put samples into the timeline that the
        // machine never rendered.
        self.audio
            .encoder
            .send_eof()
            .map_err(at("telling the audio encoder there is no more"))?;
        self.drain_audio()?;
        self.output
            .write_trailer()
            .map_err(at("closing the playlist"))
    }

    /// Moves whatever the encoder has finished into the muxer.
    fn drain(&mut self) -> Result<(), StreamError> {
        let mut packet = codec::packet::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream_index);
            // **One tick, because the encoder's time base is one tick per frame.** An encoder is
            // not obliged to fill this in and this one does not, which leaves the muxer deciding
            // where a segment ends from the gap to the next packet — so it cuts a frame late and
            // says so on every packet it writes.
            packet.set_duration(1);
            packet.rescale_ts(self.encoder_time_base, self.stream_time_base);
            packet
                .write_interleaved(&mut self.output)
                .map_err(at("writing a segment"))?;
        }
        Ok(())
    }
}

/// The name of the fMP4 initialisation segment, which every client fetches before any other.
pub const INIT_SEGMENT: &str = "init.mp4";

/// Renders a path for ffmpeg, with one separator throughout.
///
/// **A mixed path is the trap this exists for.** Joining onto a directory somebody typed with
/// forward slashes produces `C:/somewhere/stream\live.m3u8`, and the muxer's own idea of which part
/// of that is a directory keeps only what is before the *last slash it recognises* — so the
/// initialisation segment is written one directory above the playlist that names it, and every
/// client fetches a file that is not there. Nothing reports it: the playlist is correct, the
/// segments are correct, and the stream simply does not play.
fn for_ffmpeg(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The muxer options that make this a live HLS stream rather than a file.
///
/// **Every path here is absolute**, because the muxer resolves a relative one against the working
/// directory rather than against the playlist it is writing beside.
fn hls_options(dir: &Path, config: &Config) -> Dictionary<'static> {
    let mut options = Dictionary::new();
    options.set("hls_time", &config.segment_seconds.to_string());
    options.set("hls_list_size", &config.playlist_size.to_string());
    // `delete_segments` is what bounds the directory; `independent_segments` tells a client every
    // segment starts on a keyframe, which is what lets it join at the live edge rather than at the
    // start of the window.
    options.set("hls_flags", "delete_segments+independent_segments");
    // fMP4 rather than MPEG-TS. Both play on a television, and fMP4 is what a browser's own media
    // pipeline is happiest with.
    options.set("hls_segment_type", "fmp4");
    // **A bare name, unlike the segment pattern below, and the two are not interchangeable.** The
    // muxer joins this one to the playlist's own directory and writes the segment pattern as given,
    // so an absolute path here is appended to that directory and refused as a doubled path, while a
    // bare name there is written to the working directory instead. What makes the bare name land
    // correctly is `for_ffmpeg` above: the directory it is joined to is the playlist's, and that is
    // only right when the playlist's separators are all of one kind.
    options.set("hls_fmp4_init_filename", INIT_SEGMENT);
    options.set("hls_segment_filename", &for_ffmpeg(&dir.join("seg-%d.m4s")));
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_no_build_has_is_reported_as_the_name_that_was_asked_for() {
        let Err(StreamError::NoSuchEncoder { name }) = find_encoder("h264_imaginary") else {
            panic!("a name nothing carries must be refused rather than substituted");
        };
        assert_eq!(name, "h264_imaginary");
    }

    #[test]
    fn auto_takes_a_software_encoder_this_build_actually_has() {
        let codec = find_encoder(AUTO_ENCODER).expect(
            "every ffmpeg this project builds or links carries one of the software H.264 encoders",
        );
        assert!(
            SOFTWARE_ENCODERS.contains(&codec.name()),
            "auto resolved to {}, which is not one of the names it searches",
            codec.name()
        );
    }

    #[test]
    fn only_libx264_is_told_to_hold_no_frames_back() {
        assert_eq!(encoder_options("libx264").get("tune"), Some("zerolatency"));
        for other in ["libopenh264", "h264_nvenc"] {
            assert_eq!(
                encoder_options(other).get("tune"),
                None,
                "{other} opens with its own defaults"
            );
        }
    }

    /// The search is the default's alone: `libx264` written down stays `libx264`.
    ///
    /// A build carrying only one of the two would otherwise let the other name quietly resolve to
    /// it, and somebody measuring two encoders against each other would be measuring one twice.
    #[test]
    fn a_written_name_is_not_searched_for_among_the_others() {
        for candidate in SOFTWARE_ENCODERS {
            match find_encoder(candidate) {
                Ok(codec) => assert_eq!(
                    codec.name(),
                    candidate,
                    "a written name must resolve to itself or to nothing"
                ),
                Err(StreamError::NoSuchEncoder { name }) => assert_eq!(name, candidate),
                Err(other) => panic!("unexpected refusal of {candidate}: {other}"),
            }
        }
    }
}
