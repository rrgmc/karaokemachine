//! SoundFont synthesis, sequencing and the audio path.
//!
//! The engine is split so that the interesting behavior is testable without hardware:
//!
//! * [`sequencer`] turns a song into MIDI messages at the right moment, applying transposition and
//!   the guide-melody mute. It emits into a trait, so timing, transposition, muting and seeking are
//!   all verifiable with no audio device and no SoundFont.
//! * [`source`] turns MIDI messages into audio -- `rustysynth` for playback, a sine synthesizer for
//!   tests.
//! * [`player`] joins the two and fills audio buffers, absorbing the mismatch between the device's
//!   arbitrary buffer size and the synthesizer's fixed render block.
//! * [`offline`] renders to memory or a WAV file with no device at all.
//! * [`device`] decides which output device to play through, and remembers it.
//! * [`audio`] runs a [`player::Player`] on a real output stream.
//!
//! **What is deliberately no longer here.** The queue, the microphone registry, the transport enum
//! and the two playback limits are [`km_queue`] and were this crate's `queue`, `mics`,
//! `player::Transport` and `sequencer`'s constants until they moved. Nothing about them needed a
//! synthesizer, and while they lived here every crate that merely *describes* a machine -- `km-api`
//! above all, and both remotes and the Android application behind it -- linked `rustysynth`, `cpal`
//! and `rtrb` to name one. This crate still uses all four, and takes them the way everybody else
//! does. They are **not re-exported**: a caller that wants a queue says so.
//!
//! **Real-time rules.** Everything reached from the audio callback avoids allocation, file I/O,
//! locking and parsing. Songs are parsed elsewhere and handed over as `Arc<Song>`; scratch buffers
//! are sized once at construction. See `docs/ARCHITECTURE.md`.

pub mod audio;
pub mod device;
pub mod level;
pub mod offline;
pub mod player;
pub mod sequencer;
pub mod source;
pub mod track;

pub use crate::audio::{AudioError, DeviceInfo, OutputStream, Renderer};
pub use crate::device::{Chosen, OutputDevice, SYSTEM_DEFAULT};
pub use crate::offline::{RenderOptions, Rendered, render, render_keeping_source, write_wav};
pub use crate::player::{MAX_SONG_GAIN, Player, PlayerEvent, Retired};
pub use crate::sequencer::{DRUM_CHANNEL, MidiSink, PlaybackSettings, Sequencer};
pub use crate::source::{
    AudioSource, Bank, BankDefects, SoundFontSource, SourceError, TestToneSource,
};
pub use crate::track::{AudioFeed, AudioFeedWriter, FEED_CHANNELS, TrackPlayer, audio_feed};
