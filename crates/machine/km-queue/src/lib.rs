//! What a karaoke machine is doing: the queue, the microphones, the transport and the limits.
//!
//! This is the vocabulary an *observer* of the machine needs — the API describing it over HTTP, a
//! remote rendering it, the screen drawing it — as opposed to the machinery that makes any of it
//! happen, which is [`km_audio`](https://docs.rs/km-audio). Four things live here:
//!
//! * [`queue`] — the waiting songs. Pure state, held on the control thread; it knows song codes and
//!   titles and nothing about how a song is loaded or played.
//! * [`mics`] — the microphone registry. Configuration state only: mic audio is mixed in hardware,
//!   a standing decision in `docs/decisions/`, so there is no input stream here and no DSP.
//! * [`transport`] — whether a song is playing, paused, stopped or absent.
//! * [`limits`] — how far the transpose and tempo controls go.
//!
//! **Why it is a crate and not a module in `km-audio`.** Living beside the sequencer and the
//! synthesizer, these four would make describing a queue over HTTP link `rustysynth`, `cpal` and
//! `rtrb` — and so would both remotes and the Android application built out of them. `km-api` never
//! touches the engine, and this split is what lets that be true. The same
//! argument [`km_songcode`](https://docs.rs/km-songcode) was split out on, one layer up: a type that the
//! catalog, the queue, the API, the display and both remotes all have to name cannot live inside
//! any one of them.
//!
//! It takes `km-songcode` and `thiserror` and nothing else, and the absence of anything heavier is the
//! property worth protecting — see the note at the foot of `Cargo.toml`.

pub mod limits;
pub mod mics;
pub mod queue;
pub mod transport;

pub use crate::limits::{MAX_TEMPO_RATIO, MAX_TRANSPOSE, MIN_TEMPO_RATIO};
pub use crate::mics::{MAX_GAIN, MAX_MICS, MicBus, MicChannel, MicError, MicPatch, MicRegistry};
pub use crate::queue::{MAX_QUEUED, Queue, QueueEntry, QueueFull, QueueRequest};
pub use crate::transport::Transport;
