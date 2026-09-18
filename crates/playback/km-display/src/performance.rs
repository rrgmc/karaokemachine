//! What the frame meter measured, in the shape the screen wants it.
//!
//! **The measuring lives in `km-app` and only the reporting is here**, which is the same seam every
//! other resolved value on [`crate::draw::Frame`] uses: this crate is handed finished numbers and
//! knows nothing about audio callbacks, video decoders or the clock they were taken against. What
//! it owns is the decision about which of them are worth a person's attention, which is
//! [`FrameStats::strained`].
//!
//! It exists because `--frame-stats` put these numbers somewhere the person who needs them is not.
//! "The screen looks choppy" is reported from a sofa, and the answer was a line in a log on a box
//! under the television -- reachable over ssh, from another room, after the stutter had stopped.

/// One second of frames, as the meter finished counting them.
///
/// `Copy`, so [`crate::draw::Frame`] holds it by value rather than by reference. Every other
/// resolved field there is a borrow, and this one is not for a reason worth stating: it is rebuilt
/// once a second and read once a frame, so a borrow would force the caller to keep the meter alive
/// across the frame it is drawing and would buy nothing for the sixty bytes it saved.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameStats {
    /// Frames in the window this describes.
    ///
    /// **Zero is the honest "not yet" state, and it is why the panel can be drawn the instant the
    /// key is pressed.** The meter reports once a second, so a panel that waited for real numbers
    /// would come up blank for up to a second and read as a key that had not worked. Zero draws
    /// `measuring...` instead.
    ///
    /// The alternative -- shortening the meter's window while the panel is up -- was rejected
    /// because the drawn numbers and the logged numbers would then mean different things, and the
    /// whole point of the panel is that it shows what `--frame-stats` shows.
    pub frames: u32,
    /// Frames a second, as somebody watching would count them.
    pub fps: f32,
    /// Mean time to *build* a frame, up to but not including `present`.
    pub draw_ms: f32,
    /// The worst single one, because a stutter is a tail property: thirty good frames and one bad
    /// one average out to something that looks fine and is not.
    pub draw_worst_ms: f32,
    /// Mean time in `present`, which under vsync is the wait for the next refresh and therefore the
    /// **slack** in the frame. Watching it fall towards zero is watching the machine run out of room.
    pub present_ms: f32,
    /// The worst single one.
    pub present_worst_ms: f32,
    /// Mean wall time between frames: the two above plus any padding.
    pub interval_ms: f32,
    /// The worst single one.
    pub interval_worst_ms: f32,
    /// Milliseconds the sound ran dry in this window.
    pub starved_ms: u32,
    /// Pictures thrown away before anything could draw them.
    pub dropped: u32,
    /// Pictures that arrived after their moment had passed.
    pub late: u32,
    /// Device underruns -- the **only** one of the four a MIDI song can move, because nothing else
    /// here describes a decoder and `rustysynth` renders inside the audio callback.
    pub xruns: u32,
}

impl FrameStats {
    /// How close to the edge a frame may get before the panel says so, as a fraction of the interval.
    ///
    /// Draw and present together fill one interval by definition, so the useful question is not
    /// "are they large" but "has draw eaten the slack". At 90% there is a tenth of a frame left.
    const TIGHT: f32 = 0.9;

    /// How many rows this block takes.
    ///
    /// **Here rather than in the drawing**, so the panel's height is a thing a test can ask about
    /// without a font or a screen. A window that has measured nothing says so in one row; a healthy
    /// second is the heading and three timings; a decoder that complained adds its four counters.
    pub fn rows(&self) -> u32 {
        if self.frames == 0 {
            1
        } else if self.decoder_complained() {
            8
        } else {
            4
        }
    }

    /// Whether these numbers are worth coloring.
    ///
    /// Two independent ways to be in trouble, and they fail in opposite directions -- which is why
    /// this is not a single threshold. The display can be at 60 fps exactly while the *picture* has
    /// stopped, because what stopped is the decoder; that is the case the four counters catch and
    /// the three timings cannot. The reverse -- a machine that cannot build a frame in time -- moves
    /// the timings and leaves the counters at zero on every MIDI song.
    pub fn strained(&self) -> bool {
        self.decoder_complained() || self.frame_is_tight()
    }

    /// Whether anything downstream of the clock went wrong in this window.
    pub fn decoder_complained(&self) -> bool {
        self.starved_ms > 0 || self.dropped > 0 || self.late > 0 || self.xruns > 0
    }

    /// Whether building a frame has eaten the slack that `present` should be waiting in.
    ///
    /// Guarded against a zero interval, which is what a window holding one frame reports.
    pub fn frame_is_tight(&self) -> bool {
        self.interval_ms > 0.0 && self.draw_ms > self.interval_ms * Self::TIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_measured_is_not_a_complaint() {
        // The state the panel is in for its first second. It must not come up red.
        assert!(!FrameStats::default().strained());
    }

    #[test]
    fn a_healthy_second_is_not_a_complaint() {
        let stats = FrameStats {
            frames: 60,
            fps: 60.0,
            draw_ms: 3.0,
            present_ms: 13.0,
            interval_ms: 16.7,
            ..FrameStats::default()
        };
        assert!(!stats.strained());
    }

    #[test]
    fn a_display_keeping_up_perfectly_still_reports_a_starved_decoder() {
        // The case the counters exist for, and the one the three timings cannot see: 60 fps exactly
        // while the picture has stopped. This is what was actually observed on the appliance.
        let stats = FrameStats {
            frames: 60,
            fps: 60.0,
            draw_ms: 3.0,
            present_ms: 13.0,
            interval_ms: 16.7,
            starved_ms: 400,
            ..FrameStats::default()
        };
        assert!(stats.strained());
        assert!(stats.decoder_complained());
        assert!(!stats.frame_is_tight());
    }

    #[test]
    fn drawing_that_has_eaten_the_slack_is_a_complaint() {
        let stats = FrameStats {
            frames: 60,
            fps: 60.0,
            draw_ms: 16.0,
            present_ms: 0.5,
            interval_ms: 16.7,
            ..FrameStats::default()
        };
        assert!(stats.frame_is_tight());
        assert!(stats.strained());
    }

    #[test]
    fn a_window_of_one_frame_does_not_divide_by_a_zero_interval() {
        let stats = FrameStats {
            frames: 1,
            draw_ms: 8.0,
            ..FrameStats::default()
        };
        assert!(!stats.frame_is_tight());
    }
}
