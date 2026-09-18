//! How far the two playback controls may be pushed.
//!
//! Two ranges, and both of them are *product* decisions rather than facts about the synthesizer:
//! the transposition range is what the tone buttons offer, and the tempo range is what still sounds
//! like the song. So the API validates a request against them, `km-app` clamps to them, the screen
//! draws them and the sequencer applies them — four callers, of which exactly one owns a
//! synthesizer.
//!
//! They lived in `km_audio::sequencer` beside the code that applies them, which is the natural
//! home right up to the point where validating a `PUT /settings` meant linking `rustysynth`. See
//! this crate's `lib.rs`.

/// Widest transposition offered, in semitones either way.
pub const MAX_TRANSPOSE: i8 = 6;

/// Slowest and fastest playback, as a multiple of the written tempo.
pub const MIN_TEMPO_RATIO: f32 = 0.75;
/// See [`MIN_TEMPO_RATIO`].
pub const MAX_TEMPO_RATIO: f32 = 1.25;
