//! Encoding a drawn screen and the sound beside it into one HLS stream.
//!
//! **It knows nothing about karaoke.** What arrives is a drawn screen as packed BGRA and stereo
//! samples as `f32`; what leaves is a rolling playlist and its segments in a directory. That is what
//! lets the pipeline be driven by a synthetic source in a test, and it is the same division
//! `km-video` makes in the other direction — one crate that names ffmpeg, with the machine on the
//! other side of it.
//!
//! # Why HLS, and why files
//!
//! The clients are televisions. A smart television's browser plays HLS through the set's own media
//! pipeline, where a stream fed from JavaScript reaches only the newer ones — and the playlist is
//! then the whole interface, so anything that follows a URL plays it without knowing this exists.
//!
//! Files also make this ordinary request-and-response rather than a response held open for the life
//! of a song, which a machine whose other job is playing audio should not be doing.
//!
//! # The parts
//!
//! * [`pixels`] converts a drawn screen into the planes an encoder wants. No ffmpeg, because
//!   swscale is switched off by build decision and there is no scaling to do anyway.
//! * [`encode`] drives the encoder and the `hls` muxer. Behind the `ffmpeg` feature, so a build
//!   without one still compiles and tests the conversion above.

#[cfg(feature = "ffmpeg")]
pub mod encode;
pub mod pixels;

#[cfg(feature = "ffmpeg")]
pub use crate::encode::{Config, PLAYLIST, Stream, StreamError};
pub use crate::pixels::{Plane, Source, bgra_to_yuv420p, yuv420p_to_rgba};
