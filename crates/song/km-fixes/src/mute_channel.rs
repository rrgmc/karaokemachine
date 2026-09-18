//! Silencing one channel of a song.
//!
//! **There is no detector here, and that is what the module is for.** A channel somebody wants gone
//! is a judgement about the arrangement — a guide track doubling the vocal an octave up, a lead that
//! fights the singer, a part a particular bank renders badly. Nothing in the file distinguishes
//! those from a channel the arranger meant, so no rule could emit this without being wrong about
//! somebody's song.
//!
//! **It is therefore offered rather than applied**, and it is the reason that half of the rule
//! exists at all. A fix that makes every bank agree may turn itself on; a fix that deletes music
//! some banks render correctly waits for a person.
//!
//! The sequencer already suppresses one channel for the guide melody toggle, so this costs it a
//! second term in the same test rather than a new mechanism.

/// The log line this fix writes when a song starts.
pub fn describe(channel: u8) -> String {
    format!("channel {channel}: muted")
}

#[cfg(test)]
mod tests {
    use crate::{Fix, resolve};

    #[test]
    fn a_mute_never_arrives_by_itself() {
        assert!(!Fix::MuteChannel { channel: 3 }.applies_itself());
    }

    #[test]
    fn muting_marks_only_the_channel_named() {
        let resolved = resolve(&[Fix::MuteChannel { channel: 3 }]);
        assert!(resolved.mute[3]);
        assert_eq!(resolved.mute.iter().filter(|set| **set).count(), 1);
    }
}
