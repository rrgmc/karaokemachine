//! An audio track fed from another thread — the audio half of a video song.
//!
//! A MIDI song is *generated* on the audio thread: the sequencer dispatches events into the
//! synthesizer and audio comes out with no I/O anywhere. A video song is the other shape entirely —
//! the samples already exist, in a file, and somebody has to decode them. That decoding cannot
//! happen on the audio callback, so it happens on a thread of its own and the samples arrive here
//! through a lock-free ring.
//!
//! Nothing in this module knows what a video is, or names ffmpeg. It is a producer, a consumer and
//! the protocol between them, which is why the whole audio half of video playback is testable with a
//! synthetic feed and no decoder installed. `km-video` is what fills one from a real file.
//!
//! **The consumer half runs on the audio callback**, so it obeys the same hard rules as everything
//! else there: no allocation, no locking, no I/O. Buffers are sized once at construction and the
//! ring is wait-free on both ends.
//!
//! # The device rate never reaches the decoder
//!
//! The output device chooses its own sample rate, and chooses again every time it is reopened after
//! an idle release. Handing that rate to a decoder would couple it to a decision made much later and
//! somewhere else, so the feed carries samples at **the file's own rate** and [`TrackPlayer`]
//! resamples on the way out. The decoder is then a pure function of the file, and a device that
//! comes back at 44.1 kHz where it was 48 kHz costs nothing but a different step size.
//!
//! # Seeking, without either side blocking
//!
//! A seek has to discard whatever is already in the ring, and the awkward part is that the producer
//! cannot remove what it has written. So it says how much to throw away instead: on noticing a
//! request it repositions, publishes the running total of samples it had written *before* the seek
//! as `stale_until`, acknowledges, and only then writes fresh data. The consumer discards until its
//! own running total reaches that mark. Neither side ever waits for the other, which matters because
//! one of them is a real-time callback and the other is doing file I/O.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Channels a feed carries. Interleaved stereo, always.
///
/// Fixed rather than negotiated: a decoder can downmix or upmix to stereo far better than this
/// module could, and every extra shape here is a shape to get wrong on the audio thread.
pub const FEED_CHANNELS: usize = 2;

/// Shared between the producer and the consumer. Every field is written by exactly one side.
#[derive(Debug, Default)]
struct FeedState {
    /// Set by the producer when the file has no more samples.
    eof: AtomicBool,
    /// Bumped by the consumer to ask for a seek.
    seek_request: AtomicU32,
    /// Where the consumer wants to go, in milliseconds.
    seek_target_ms: AtomicU32,
    /// Echoed back by the producer once it has repositioned.
    seek_acked: AtomicU32,
    /// Samples written before the acknowledged seek; the consumer discards up to this.
    stale_until: AtomicU64,
}

/// Builds a feed, returning the producer and consumer halves.
///
/// `capacity_frames` is how much audio the ring holds. Enough to cover a decoder hiccup and no more:
/// every frame of it is latency on a seek, and a second or two is the useful range.
#[must_use]
pub fn audio_feed(sample_rate: u32, capacity_frames: usize) -> (AudioFeedWriter, AudioFeed) {
    let state = Arc::new(FeedState::default());
    let (tx, rx) = rtrb::RingBuffer::new(capacity_frames.max(1) * FEED_CHANNELS);
    let writer = AudioFeedWriter {
        tx,
        state: Arc::clone(&state),
        written: 0,
        seen_request: 0,
    };
    let feed = AudioFeed {
        rx,
        state,
        sample_rate: sample_rate.max(1),
        read: 0,
    };
    (writer, feed)
}

/// The producing half of a feed, owned by whatever is decoding.
pub struct AudioFeedWriter {
    tx: rtrb::Producer<f32>,
    state: Arc<FeedState>,
    /// Samples pushed since the feed was created. Only this side reads it.
    written: u64,
    /// The last seek request this side noticed.
    seen_request: u32,
}

impl AudioFeedWriter {
    /// Pushes interleaved stereo samples, returning how many were taken.
    ///
    /// A short return means the ring is full and the rest should be offered again later; it is the
    /// normal way a decoder learns to stop running ahead.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let (pushed, _remainder) = self.tx.push_partial_slice(samples);
        self.written += pushed.len() as u64;
        pushed.len()
    }

    /// How many samples can be pushed right now without a short write.
    #[must_use]
    pub fn space(&self) -> usize {
        self.tx.slots()
    }

    /// Declares the file exhausted. The consumer finishes once it has drained what is left.
    pub fn finish(&mut self) {
        self.state.eof.store(true, Ordering::Release);
    }

    /// Where the consumer has asked to seek to, in milliseconds, if it has.
    ///
    /// After repositioning, the caller **must** call [`AudioFeedWriter::seek_complete`] before
    /// pushing anything else, or the consumer cannot tell the new samples from the old.
    pub fn pending_seek(&mut self) -> Option<u32> {
        let request = self.state.seek_request.load(Ordering::Acquire);
        if request == self.seen_request {
            return None;
        }
        self.seen_request = request;
        Some(self.state.seek_target_ms.load(Ordering::Relaxed))
    }

    /// Declares the decoder repositioned, marking everything written so far as stale.
    pub fn seek_complete(&mut self) {
        self.state
            .stale_until
            .store(self.written, Ordering::Relaxed);
        self.state
            .seek_acked
            .store(self.seen_request, Ordering::Release);
    }

    /// Whether the consuming half has been dropped, so decoding is pointless.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.tx.is_abandoned()
    }
}

/// The consuming half of a feed, owned by the player on the audio thread.
#[derive(Debug)]
pub struct AudioFeed {
    rx: rtrb::Consumer<f32>,
    state: Arc<FeedState>,
    sample_rate: u32,
    /// Samples taken since the feed was created, discarded ones included.
    read: u64,
}

impl AudioFeed {
    /// The rate the samples arrive at — the file's, not the device's.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Asks the producer to reposition. Samples already in the ring are discarded.
    fn request_seek(&mut self, ms: u32) {
        self.state.seek_target_ms.store(ms, Ordering::Relaxed);
        // Fetch-add rather than a store: two seeks in quick succession must not collapse into one,
        // or the producer would ack the first and the consumer would go on discarding for the
        // second one for ever.
        self.state.seek_request.fetch_add(1, Ordering::AcqRel);
    }

    /// Whether a requested seek has yet to be acknowledged.
    fn seek_pending(&self) -> bool {
        self.state.seek_acked.load(Ordering::Acquire)
            != self.state.seek_request.load(Ordering::Relaxed)
    }

    /// Throws away up to `limit` samples, returning how many went.
    fn discard(&mut self, limit: usize) -> usize {
        let n = limit.min(self.rx.slots());
        if n == 0 {
            return 0;
        }
        match self.rx.read_chunk(n) {
            Ok(chunk) => {
                chunk.commit_all();
                self.read += n as u64;
                n
            }
            Err(_) => 0,
        }
    }

    /// Discards whatever the producer marked stale after the last acknowledged seek.
    fn drop_stale(&mut self) {
        let mark = self.state.stale_until.load(Ordering::Relaxed);
        while self.read < mark {
            let wanted = usize::try_from(mark - self.read).unwrap_or(usize::MAX);
            if self.discard(wanted) == 0 {
                return;
            }
        }
    }

    /// Takes one interleaved stereo frame, or `None` if none is ready.
    fn pop_frame(&mut self) -> Option<[f32; FEED_CHANNELS]> {
        if self.rx.slots() < FEED_CHANNELS {
            return None;
        }
        let mut frame = [0.0; FEED_CHANNELS];
        for slot in &mut frame {
            *slot = self.rx.pop().ok()?;
            self.read += 1;
        }
        Some(frame)
    }

    /// Whether the producer has finished and everything it wrote has been taken.
    fn drained(&self) -> bool {
        self.state.eof.load(Ordering::Acquire) && self.rx.slots() < FEED_CHANNELS
    }
}

/// Plays an [`AudioFeed`] into the output, resampling to the device's rate.
///
/// This is the video-song counterpart of [`crate::sequencer::Sequencer`], and deliberately has the
/// same shape: it owns the position, it is advanced from the audio callback, and it reports when it
/// has finished. What it does *not* have is a tick timeline, a transposition or a channel to mute —
/// a video carries none of those, which is why a video song's controls are reported unavailable
/// rather than silently doing nothing.
#[derive(Debug)]
pub struct TrackPlayer {
    feed: AudioFeed,
    /// Device rate, which is what the position is measured in.
    out_rate: u32,
    /// Source frames advanced per output frame.
    step: f64,
    /// Fractional position between `prev` and `next`.
    cursor: f64,
    prev: [f32; FEED_CHANNELS],
    next: [f32; FEED_CHANNELS],
    /// Whether `prev`/`next` hold real samples yet.
    primed: bool,
    /// Output frames emitted, which is the position. Frozen while starved.
    rendered: u64,
    /// Where a pending seek will land, in output frames.
    seek_base: u64,
    /// Output frames dropped to a starved feed, for one summary log per song.
    starved: u64,
    finished: bool,
}

impl TrackPlayer {
    /// Builds a player for a feed, rendering at `out_rate`.
    #[must_use]
    pub fn new(feed: AudioFeed, out_rate: u32) -> Self {
        let out_rate = out_rate.max(1);
        let step = f64::from(feed.sample_rate()) / f64::from(out_rate);
        Self {
            feed,
            out_rate,
            step,
            cursor: 0.0,
            prev: [0.0; FEED_CHANNELS],
            next: [0.0; FEED_CHANNELS],
            primed: false,
            rendered: 0,
            seek_base: 0,
            starved: 0,
            finished: false,
        }
    }

    /// Position in milliseconds.
    #[must_use]
    pub fn position_ms(&self) -> u32 {
        let ms = self.rendered.saturating_mul(1000) / u64::from(self.out_rate);
        u32::try_from(ms).unwrap_or(u32::MAX)
    }

    /// Whether the track has played to its end.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Output frames that had no samples to play. Zero on a healthy decode.
    #[must_use]
    pub fn starved_frames(&self) -> u64 {
        self.starved
    }

    /// The same, as the time somebody sat through. Zero on a healthy decode.
    ///
    /// Converted here rather than by the caller because this is what knows `out_rate`, and it is the
    /// unit the number is ever read in: a stall is reported as "the song stopped for 3.1 s", never
    /// as 148,000 frames.
    ///
    /// **Not the same as the position falling behind the wall clock.** [`Self::stall`] freezes the
    /// position while this grows, so a song that starved for three seconds ends three seconds late
    /// having played every sample it had — the sound hesitated, it did not skip.
    #[must_use]
    pub fn starved_ms(&self) -> u64 {
        self.starved.saturating_mul(1000) / u64::from(self.out_rate)
    }

    /// Jumps to a position in milliseconds.
    ///
    /// Renders silence until the decoder reports back, which is a fraction of a second and is what
    /// every player does across a seek.
    pub fn seek_ms(&mut self, ms: u32) {
        self.seek_base = u64::from(ms) * u64::from(self.out_rate) / 1000;
        self.rendered = self.seek_base;
        self.primed = false;
        self.cursor = 0.0;
        self.finished = false;
        self.feed.request_seek(ms);
    }

    /// Returns to the start.
    pub fn restart(&mut self) {
        self.seek_ms(0);
    }

    /// Fills two mono buffers, advancing the position by their length.
    ///
    /// `advancing` is false when the transport is paused: the buffers are still filled, with
    /// silence, and a pending seek is still serviced, so pausing never wedges the decoder.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], advancing: bool) {
        let frames = left.len().min(right.len());

        if self.feed.seek_pending() {
            // Everything in the ring predates the seek. Throw away what is there; the rest goes
            // when the producer says how much there was.
            self.feed.discard(usize::MAX);
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return;
        }
        self.feed.drop_stale();

        if !advancing {
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return;
        }

        for i in 0..frames {
            if !self.primed {
                let Some(first) = self.feed.pop_frame() else {
                    self.stall(&mut left[i..frames], &mut right[i..frames]);
                    return;
                };
                self.prev = first;
                self.next = self.feed.pop_frame().unwrap_or(first);
                self.primed = true;
                self.cursor = 0.0;
            }

            while self.cursor >= 1.0 {
                let Some(frame) = self.feed.pop_frame() else {
                    self.stall(&mut left[i..frames], &mut right[i..frames]);
                    return;
                };
                self.prev = self.next;
                self.next = frame;
                self.cursor -= 1.0;
            }

            // Linear interpolation. Between 44.1 and 48 kHz on a backing track this is inaudible;
            // if a measurement ever says otherwise, this is the one function to replace.
            let t = self.cursor as f32;
            left[i] = self.prev[0] + (self.next[0] - self.prev[0]) * t;
            right[i] = self.prev[1] + (self.next[1] - self.prev[1]) * t;
            self.cursor += self.step;
            self.rendered += 1;
        }
    }

    /// Nothing to play: fill silence and hold the position, so the picture waits with the sound.
    ///
    /// Holding rather than advancing is deliberate. If the position ran on through a stall the
    /// video would be asked for frames that the same stall has not produced either, and the two
    /// would come back misaligned; freezing both keeps them together and costs a hesitation.
    fn stall(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.feed.drained() {
            self.finished = true;
        } else {
            self.starved += left.len() as u64;
        }
        left.fill(0.0);
        right.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A feed with room for a tenth of a second.
    fn feed(rate: u32) -> (AudioFeedWriter, AudioFeed) {
        audio_feed(rate, rate as usize / 10)
    }

    /// Interleaves a ramp so each frame is distinguishable.
    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|i| [i as f32, -(i as f32)]).collect()
    }

    fn render(player: &mut TrackPlayer, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        player.render(&mut left, &mut right, true);
        (left, right)
    }

    #[test]
    fn same_rate_passes_samples_through_unchanged() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(8));
        let mut player = TrackPlayer::new(feed, RATE);

        let (left, right) = render(&mut player, 4);
        assert_eq!(left, vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(right, vec![0.0, -1.0, -2.0, -3.0]);
    }

    #[test]
    fn position_counts_output_frames_not_source_frames() {
        let (mut writer, feed) = feed(24_000);
        writer.push(&ramp(1_000));
        // Source is half the device rate, so 480 output frames is 10 ms either way.
        let mut player = TrackPlayer::new(feed, RATE);

        render(&mut player, 480);
        assert_eq!(player.position_ms(), 10);
    }

    #[test]
    fn upsampling_interpolates_between_source_frames() {
        let (mut writer, feed) = feed(24_000);
        writer.push(&ramp(8));
        let mut player = TrackPlayer::new(feed, 48_000);

        let (left, _) = render(&mut player, 4);
        // step = 0.5, so every other output frame is a midpoint.
        assert_eq!(left, vec![0.0, 0.5, 1.0, 1.5]);
    }

    #[test]
    fn downsampling_skips_source_frames() {
        let (mut writer, feed) = feed(96_000);
        writer.push(&ramp(16));
        let mut player = TrackPlayer::new(feed, 48_000);

        let (left, _) = render(&mut player, 4);
        assert_eq!(left, vec![0.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    fn starvation_holds_the_position_instead_of_advancing_past_it() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(4));
        let mut player = TrackPlayer::new(feed, RATE);

        let (left, _) = render(&mut player, 16);
        // Real samples first, then silence, and the position stops where the audio did.
        assert_eq!(&left[..3], &[0.0, 1.0, 2.0]);
        assert!(left[8..].iter().all(|s| *s == 0.0));
        assert!(player.position_ms() < 1);
        assert!(player.starved_frames() > 0);
        assert!(!player.is_finished(), "starved is not finished");
    }

    #[test]
    fn starvation_is_reported_as_the_time_it_lasted() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(4));
        let mut player = TrackPlayer::new(feed, RATE);

        // Four frames of audio yield three of output -- the fourth is consumed priming `next` and
        // is heard as the endpoint of the third rather than as a frame of its own -- so 4,797 of
        // the 4,800 are silence. At 48 kHz that is 99 ms, rounded down as integer division does.
        render(&mut player, 4_800);
        assert_eq!(player.starved_frames(), 4_797);
        assert_eq!(player.starved_ms(), 99);
    }

    #[test]
    fn a_healthy_song_starves_for_no_time_at_all() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(4_800));
        let mut player = TrackPlayer::new(feed, RATE);

        render(&mut player, 4_800);
        assert_eq!(player.starved_ms(), 0, "nothing to report on a fed song");
    }

    #[test]
    fn the_same_silence_is_the_same_milliseconds_at_any_device_rate() {
        // The counter is in output frames, so the two players below starve by numbers that differ
        // by a factor of two. What somebody sat through is a second either way, and that is what
        // the conversion has to protect -- a device rate is not something a stall report may vary
        // with.
        let mut ms = Vec::new();
        for rate in [RATE, 24_000] {
            let (_writer, feed) = feed(rate);
            let mut player = TrackPlayer::new(feed, rate);
            render(&mut player, rate as usize);
            ms.push(player.starved_ms());
        }
        assert_eq!(ms, vec![1_000, 1_000]);
    }

    #[test]
    fn finishes_only_once_the_producer_says_so_and_the_ring_is_empty() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(4));
        let mut player = TrackPlayer::new(feed, RATE);

        render(&mut player, 2);
        assert!(!player.is_finished());

        writer.finish();
        render(&mut player, 16);
        assert!(player.is_finished());
    }

    #[test]
    fn paused_renders_silence_without_consuming_the_feed() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(8));
        let mut player = TrackPlayer::new(feed, RATE);

        let mut left = vec![9.0; 4];
        let mut right = vec![9.0; 4];
        player.render(&mut left, &mut right, false);
        assert!(left.iter().all(|s| *s == 0.0));
        assert_eq!(player.position_ms(), 0);

        // The samples are still there once it resumes.
        let (left, _) = render(&mut player, 4);
        assert_eq!(left, vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn seek_discards_everything_written_before_the_acknowledgment() {
        let (mut writer, feed) = feed(RATE);
        // Stale audio, already in the ring when the seek is asked for.
        writer.push(&ramp(64));
        let mut player = TrackPlayer::new(feed, RATE);

        player.seek_ms(1_000);
        assert_eq!(player.position_ms(), 1_000, "position lands immediately");

        // Silence while the decoder is still repositioning, and the stale audio never plays.
        let (left, _) = render(&mut player, 32);
        assert!(left.iter().all(|s| *s == 0.0));

        // The decoder writes a little more stale audio before it notices, as it really would.
        writer.push(&ramp(8));
        let target = writer
            .pending_seek()
            .expect("the seek is visible to the producer");
        assert_eq!(target, 1_000);
        writer.seek_complete();

        // Now fresh audio, marked so it is distinguishable from the ramp above. Three frames for
        // two of output: interpolation always holds the frame after the one it is emitting.
        writer.push(&[0.5, -0.5, 0.25, -0.25, 0.125, -0.125]);
        let (left, right) = render(&mut player, 2);
        assert_eq!(left, vec![0.5, 0.25]);
        assert_eq!(right, vec![-0.5, -0.25]);
        assert_eq!(
            player.position_ms(),
            1_000,
            "the seek target is where time resumes"
        );
    }

    #[test]
    fn two_seeks_in_a_row_do_not_collapse_into_one() {
        let (mut writer, feed) = feed(RATE);
        let mut player = TrackPlayer::new(feed, RATE);

        player.seek_ms(1_000);
        player.seek_ms(2_000);

        // The producer sees the *second* target, and one acknowledgment clears both.
        assert_eq!(writer.pending_seek(), Some(2_000));
        writer.seek_complete();
        assert_eq!(writer.pending_seek(), None);

        writer.push(&[1.0, 1.0]);
        let (left, _) = render(&mut player, 1);
        assert_eq!(left, vec![1.0]);
        assert_eq!(player.position_ms(), 2_000);
    }

    #[test]
    fn restart_goes_back_to_the_start() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(64));
        let mut player = TrackPlayer::new(feed, RATE);
        render(&mut player, 32);

        player.restart();
        assert_eq!(player.position_ms(), 0);
        assert_eq!(writer.pending_seek(), Some(0));
    }

    #[test]
    fn a_dropped_consumer_tells_the_producer_to_stop() {
        let (writer, feed) = feed(RATE);
        assert!(!writer.is_abandoned());
        drop(feed);
        assert!(writer.is_abandoned());
    }
}
