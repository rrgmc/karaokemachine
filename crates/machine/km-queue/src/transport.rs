//! What the transport is doing.
//!
//! Four states and one question about them. It lives here rather than beside the player that owns
//! it because everything that *reports* the transport — the API, both remotes, the screen — needs
//! the word without needing the synthesizer, and while this enum sat in `km_audio::player` asking
//! for it meant linking `rustysynth` and `cpal`. See this crate's `lib.rs`.

/// What the transport is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// No song loaded.
    #[default]
    Idle,
    /// Playing.
    Playing,
    /// A song is loaded and positioned, but time is not advancing.
    Paused,
    /// A song is loaded and positioned at the start.
    Stopped,
}

impl Transport {
    /// Whether song time should advance.
    pub fn is_advancing(self) -> bool {
        matches!(self, Self::Playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_playing_advances() {
        assert!(Transport::Playing.is_advancing());
        for transport in [Transport::Idle, Transport::Paused, Transport::Stopped] {
            assert!(!transport.is_advancing());
        }
    }

    #[test]
    fn idle_is_the_default() {
        assert_eq!(Transport::default(), Transport::Idle);
    }
}
