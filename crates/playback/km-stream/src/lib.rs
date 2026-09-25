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
//! The clients are televisions. A television's own player plays HLS through the set's own media
//! pipeline, and so does VLC, Kodi or a set-top box. The playlist is then the whole interface, so
//! anything that follows a URL plays it without knowing this exists. Files also make it ordinary
//! request-and-response.
//!
//! # And fragments beside it
//!
//! The watch page takes the same packets as fragmented MP4 over a socket, which puts it under half a
//! second behind. That is [`fragments`] and `Stream::open_with_fragments`: a copy of each packet,
//! not a second encode.
//!
//! # The parts
//!
//! * [`pixels`] converts a drawn screen into the planes an encoder wants. No ffmpeg, because
//!   swscale is switched off by build decision and there is no scaling to do anyway.
//! * [`encode`] drives the encoder, the `hls` muxer and the fragment muxer beside it. Behind the
//!   `ffmpeg` feature, so a build without one still compiles and tests the conversion above.
//! * [`fragments`] cuts what the fragment muxer writes into whole pieces a browser can append. No
//!   ffmpeg, for the reason `pixels` has none.

#[cfg(feature = "ffmpeg")]
pub mod encode;
pub mod fragments;
pub mod pixels;

#[cfg(feature = "ffmpeg")]
pub use crate::encode::{Config, PLAYLIST, Stream, StreamError};
pub use crate::fragments::{Piece, Sink};
pub use crate::pixels::{Plane, Source, bgra_to_yuv420p, yuv420p_to_rgba};
