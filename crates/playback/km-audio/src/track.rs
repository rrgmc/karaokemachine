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
//! # Key and tempo
//!
//! A stretcher changes the length of the audio and keeps its pitch. The resampler changes both
//! together. A song at tempo `T` and pitch ratio `P` is therefore the audio stretched to rate
//! `T / P` and then resampled by `P`, so the stretcher is the one stage a key change adds.
//!
//! **A song at its own key and tempo does not pass through the stretcher.** Its samples reach the
//! resampler straight from the feed, so an unchanged song costs and sounds what it does without the
//! stage. A player built without [`TrackPlayer::with_stretch`] has no stage and ignores both
//! controls.
//!
//! **The position is a position in the song, not in the output.** At tempo 1.25 one second of output
//! is 1.25 seconds of song. The picture, the graphics and the words all read the position as a place
//! in the file.
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

use km_queue::{MAX_TEMPO_RATIO, MAX_TRANSPOSE, MIN_TEMPO_RATIO};
use timestretch::engine::{
    Engine, EngineConfig, EngineController, EngineProcessor, EngineProfile, SourceProducer,
};

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

    /// Takes whole frames into `out`, returning how many frames went.
    fn pop_frames(&mut self, out: &mut [f32]) -> usize {
        let frames = (out.len() / FEED_CHANNELS).min(self.rx.slots() / FEED_CHANNELS);
        for slot in &mut out[..frames * FEED_CHANNELS] {
            let Ok(sample) = self.rx.pop() else {
                return 0;
            };
            *slot = sample;
            self.read += 1;
        }
        frames
    }

    /// Whether the producer has finished and everything it wrote has been taken.
    fn drained(&self) -> bool {
        self.state.eof.load(Ordering::Acquire) && self.rx.slots() < FEED_CHANNELS
    }
}

/// Frames the stretcher renders at a time, which is the smallest block its engine takes.
const STRETCH_BLOCK: usize = 64;

/// Frames the stretcher's own ring holds. Enough for a few blocks at the fastest rate, and no more:
/// the feed is the buffer, and every frame held here is a frame a return to bypass skips.
const STRETCH_RING_FRAMES: usize = 4096;

/// Blocks one call may render while the engine fills its first window. The engine needs about ten at
/// the fastest rate, and the bound keeps a call on the audio thread finite.
const STRETCH_FILL_BLOCKS: usize = 64;

/// Blocks of silence the engine runs when it is built, which carries it past its first window.
const STRETCH_WARM_UP_BLOCKS: usize = 128;

/// The stage that changes a song's length without changing its pitch.
///
/// Built on the control thread, because building allocates. Everything it does afterwards runs on
/// the audio callback and allocates nothing.
#[derive(Debug)]
struct Stretch {
    controller: EngineController,
    processor: EngineProcessor,
    source: SourceProducer,
    /// One rendered block, interleaved, and how much of it the resampler has taken.
    block: [f32; STRETCH_BLOCK * FEED_CHANNELS],
    len: usize,
    at: usize,
    /// Whether the song passes through the stage. False at the song's own key and tempo.
    engaged: bool,
    /// Feed frames given to the engine since it was last reset.
    fed: u64,
    /// Whether the end of the feed has been flushed through the engine.
    flushed: bool,
    /// Whether the engine has played the last of the song.
    ended: bool,
}

impl Stretch {
    fn new(sample_rate: u32) -> Option<Box<Self>> {
        let handles = Engine::build(EngineConfig {
            sample_rate,
            channels: FEED_CHANNELS,
            profile: EngineProfile::WideKeylock,
            max_block_frames: STRETCH_BLOCK,
            source_capacity_frames: STRETCH_RING_FRAMES,
            ..EngineConfig::default()
        })
        .ok()?;
        let mut stretch = Box::new(Self {
            controller: handles.controller,
            processor: handles.processor,
            source: handles.source,
            block: [0.0; STRETCH_BLOCK * FEED_CHANNELS],
            len: 0,
            at: 0,
            engaged: false,
            fed: 0,
            flushed: false,
            ended: false,
        });
        stretch.warm_up();
        Some(stretch)
    }

    /// Runs the engine over silence at each end of its rate range, here on the control thread.
    ///
    /// The engine grows its buffers on the first blocks at a rate it has not run at, and never
    /// after them. `tests/track_alloc.rs` counts, and fails where this is taken out.
    ///
    /// **Silence is enough, and only on the fork.** The published crate also grows a list of
    /// spectral peaks on dense music, which no silence reaches. The fork in the root `Cargo.toml`
    /// sizes that list when it builds the engine, and the same test fails on the published crate.
    fn warm_up(&mut self) {
        let fastest = f64::from(MAX_TEMPO_RATIO) * 2f64.powf(f64::from(MAX_TRANSPOSE) / 12.0);
        let slowest = f64::from(MIN_TEMPO_RATIO) / 2f64.powf(f64::from(MAX_TRANSPOSE) / 12.0);
        let silence = [0.0; STRETCH_BLOCK * FEED_CHANNELS];
        for rate in [fastest, slowest, 1.0] {
            self.controller.set_tempo_rate(rate);
            for _ in 0..STRETCH_WARM_UP_BLOCKS {
                while self.source.free_frames() >= STRETCH_BLOCK
                    && self.source.occupied_frames() < STRETCH_RING_FRAMES / 2
                {
                    self.source.push(&silence);
                }
                self.processor.process(&mut self.block);
            }
        }
        self.restart();
    }

    /// Empties the engine, so the next frame it plays is the next frame it is given.
    fn restart(&mut self) {
        self.processor.reset();
        self.len = 0;
        self.at = 0;
        self.fed = 0;
        self.flushed = false;
        self.ended = false;
    }

    /// The next stretched frame, or `None` when the feed has too little for a block.
    fn next_frame(&mut self, feed: &mut AudioFeed) -> Option<[f32; FEED_CHANNELS]> {
        if self.at == self.len && !self.refill(feed) {
            return None;
        }
        let i = self.at * FEED_CHANNELS;
        self.at += 1;
        Some([self.block[i], self.block[i + 1]])
    }

    /// Renders one block. False when the feed cannot cover it yet, or the song has ended.
    fn refill(&mut self, feed: &mut AudioFeed) -> bool {
        if self.ended {
            return false;
        }
        let need = self
            .source
            .demand_hint(STRETCH_BLOCK, self.controller.tempo_rate_target());
        let mut chunk = [0.0; STRETCH_BLOCK * FEED_CHANNELS];
        for _ in 0..STRETCH_FILL_BLOCKS {
            while self.source.occupied_frames() < need {
                let room = self.source.free_frames().min(STRETCH_BLOCK);
                let frames = feed.pop_frames(&mut chunk[..room * FEED_CHANNELS]);
                if frames == 0 {
                    break;
                }
                self.source.push(&chunk[..frames * FEED_CHANNELS]);
                self.fed += frames as u64;
            }

            let short = self.source.occupied_frames() < need;
            if short {
                // The engine plays silence for what it does not have. A block is therefore
                // rendered only when the feed covers it, and a short feed stalls the song as it
                // does in bypass.
                if !feed.drained() {
                    return false;
                }
                if !self.flushed {
                    self.flushed = self.source.finish();
                }
            }

            let dry_before = self.controller.underrun_frames();
            self.processor.process(&mut self.block);
            let dry = self.controller.underrun_frames() - dry_before;
            if short && self.flushed {
                if dry > 0 {
                    // The engine ran out inside this block, so the block holds the song's last
                    // frames.
                    self.ended = true;
                }
            } else if dry >= STRETCH_BLOCK as u64 {
                // The engine is still filling its first window and gave a block of its own
                // silence. That block is not part of the song, so it is dropped.
                continue;
            }
            self.len = STRETCH_BLOCK;
            self.at = 0;
            return true;
        }
        false
    }
}

/// Plays an [`AudioFeed`] into the output, resampling to the device's rate.
///
/// This is the video-song counterpart of [`crate::sequencer::Sequencer`], and deliberately has the
/// same shape: it owns the position, it is advanced from the audio callback, and it reports when it
/// has finished. It has no tick timeline and no channel to mute, because a recording carries
/// neither. Its key and tempo come from the stage [`TrackPlayer::with_stretch`] adds.
#[derive(Debug)]
pub struct TrackPlayer {
    feed: AudioFeed,
    /// Device rate.
    out_rate: u32,
    /// Feed frames per output frame at the song's own key.
    base_step: f64,
    /// Frames the resampler advances per output frame: `base_step` times the pitch ratio.
    step: f64,
    /// Feed frames the song advances per output frame: `base_step` times the tempo.
    advance: f64,
    /// The stretcher, where the song may change key and tempo.
    stretch: Option<Box<Stretch>>,
    /// Fractional position between `prev` and `next`.
    cursor: f64,
    prev: [f32; FEED_CHANNELS],
    next: [f32; FEED_CHANNELS],
    /// Whether `prev`/`next` hold real samples yet.
    primed: bool,
    /// The position at the last seek or change of tempo, in feed frames.
    base: f64,
    /// Output frames emitted since `base`. Frozen while starved.
    rendered: u64,
    /// Output frames dropped to a starved feed, for one summary log per song.
    starved: u64,
    /// The position from which silence may end the song, or `None` where it never may.
    quiet_from_ms: Option<u32>,
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
            base_step: step,
            step,
            advance: step,
            stretch: None,
            cursor: 0.0,
            prev: [0.0; FEED_CHANNELS],
            next: [0.0; FEED_CHANNELS],
            primed: false,
            base: 0.0,
            rendered: 0,
            starved: 0,
            quiet_from_ms: None,
            finished: false,
        }
    }

    /// Adds the stage that lets the song change key and tempo.
    ///
    /// Call it on the control thread: building the stage allocates, and nothing after it does. A
    /// player without the stage ignores [`TrackPlayer::set_adjustments`].
    #[must_use]
    pub fn with_stretch(mut self) -> Self {
        self.add_stretch();
        self
    }

    /// [`TrackPlayer::with_stretch`], for a player that is already inside something else.
    pub fn add_stretch(&mut self) {
        self.stretch = Stretch::new(self.feed.sample_rate());
    }

    /// Whether the song can change key and tempo.
    #[must_use]
    pub fn can_adjust(&self) -> bool {
        self.stretch.is_some()
    }

    /// Sets the key in semitones and the tempo as a ratio of the song's own.
    ///
    /// Both are clamped to the machine's limits. Key 0 and tempo 1.0 take the stretcher out of the
    /// path. Does nothing on a player without the stage.
    pub fn set_adjustments(&mut self, semitones: i8, tempo_ratio: f32) {
        let Some(stretch) = self.stretch.as_deref_mut() else {
            return;
        };
        let semitones = semitones.clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE);
        let tempo = f64::from(tempo_ratio.clamp(MIN_TEMPO_RATIO, MAX_TEMPO_RATIO));
        let pitch = 2f64.powf(f64::from(semitones) / 12.0);
        let engage = semitones != 0 || (tempo - 1.0).abs() > 0.0005;

        // The position holds what the old tempo counted, and counts on from here at the new one.
        self.base += self.rendered as f64 * self.advance;
        self.rendered = 0;

        if engage {
            stretch.controller.set_tempo_rate(tempo / pitch);
            if !stretch.engaged {
                stretch.restart();
                stretch.engaged = true;
            }
            self.step = self.base_step * pitch;
            self.advance = self.base_step * tempo;
        } else {
            if stretch.engaged {
                // The engine holds frames it took from the feed and has not played. Bypass resumes
                // at the feed's next frame, so the position moves over the frames left behind.
                let played = stretch.controller.source_position();
                self.base += (stretch.fed as f64 - played).max(0.0);
                stretch.engaged = false;
            }
            self.step = self.base_step;
            self.advance = self.base_step;
        }
    }

    /// Names the position from which a run of silence ends the song.
    ///
    /// The caller knows where that is and this does not: the last word of a timed song, or the final
    /// part of one with no timeline. `None` leaves the song to end with its file.
    pub fn set_quiet_from_ms(&mut self, quiet_from_ms: Option<u32>) {
        self.quiet_from_ms = quiet_from_ms;
    }

    /// Whether the position has reached the point from which silence may end the song.
    #[must_use]
    pub fn is_past_quiet_point(&self) -> bool {
        self.quiet_from_ms
            .is_some_and(|from| self.position_ms() >= from)
    }

    /// Position in the song, in milliseconds.
    ///
    /// It counts the song and not the output: at tempo 1.25 a second of output moves it 1,250.
    #[must_use]
    pub fn position_ms(&self) -> u32 {
        let frames = self.base + self.rendered as f64 * self.advance;
        // The small term keeps a whole millisecond whole after the multiplication above.
        let ms = frames * 1000.0 / f64::from(self.feed.sample_rate()) + 1e-6;
        ms.clamp(0.0, f64::from(u32::MAX)) as u32
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
        self.base = f64::from(ms) * f64::from(self.feed.sample_rate()) / 1000.0;
        self.rendered = 0;
        self.primed = false;
        self.cursor = 0.0;
        self.finished = false;
        if let Some(stretch) = self.stretch.as_deref_mut() {
            stretch.restart();
        }
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
    ///
    /// Returns whether every frame came from the song. A paused block, a block waiting on a seek and
    /// a block the decoder left short are silent without the song being silent, and the caller that
    /// listens for a silent tail must not count them.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], advancing: bool) -> bool {
        let frames = left.len().min(right.len());

        if self.feed.seek_pending() {
            // Everything in the ring predates the seek. Throw away what is there; the rest goes
            // when the producer says how much there was.
            self.feed.discard(usize::MAX);
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return false;
        }
        self.feed.drop_stale();

        if !advancing {
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return false;
        }

        for i in 0..frames {
            if !self.primed {
                let Some(first) = self.next_frame() else {
                    self.stall(&mut left[i..frames], &mut right[i..frames]);
                    return false;
                };
                self.prev = first;
                self.next = self.next_frame().unwrap_or(first);
                self.primed = true;
                self.cursor = 0.0;
            }

            while self.cursor >= 1.0 {
                let Some(frame) = self.next_frame() else {
                    self.stall(&mut left[i..frames], &mut right[i..frames]);
                    return false;
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
        true
    }

    /// The next frame for the resampler: from the stretcher when it is engaged, else from the feed.
    fn next_frame(&mut self) -> Option<[f32; FEED_CHANNELS]> {
        match self.stretch.as_deref_mut() {
            Some(stretch) if stretch.engaged => stretch.next_frame(&mut self.feed),
            _ => self.feed.pop_frame(),
        }
    }

    /// Nothing to play: fill silence and hold the position, so the picture waits with the sound.
    ///
    /// Holding rather than advancing is deliberate. If the position ran on through a stall the
    /// video would be asked for frames that the same stall has not produced either, and the two
    /// would come back misaligned; freezing both keeps them together and costs a hesitation.
    fn stall(&mut self, left: &mut [f32], right: &mut [f32]) {
        let playing_out = self
            .stretch
            .as_deref()
            .is_some_and(|stretch| stretch.engaged && !stretch.ended);
        if self.feed.drained() && !playing_out {
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
    fn the_position_is_the_same_at_any_device_rate() {
        let (mut writer, feed) = feed(24_000);
        writer.push(&ramp(1_000));
        // Source is half the device rate, so 480 output frames is 240 of the song, and 10 ms.
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

    const TONE_HZ: f32 = 440.0;
    const TONE_FRAMES: usize = 2 * 44_100;

    /// A player with the stage, over a whole two-second tone the producer has finished.
    fn tone_player() -> TrackPlayer {
        let (mut writer, feed) = audio_feed(44_100, TONE_FRAMES);
        let tone: Vec<f32> = (0..TONE_FRAMES)
            .flat_map(|i| {
                let s = (i as f32 * TONE_HZ * std::f32::consts::TAU / 44_100.0).sin() * 0.5;
                [s, s]
            })
            .collect();
        assert_eq!(writer.push(&tone), tone.len());
        writer.finish();
        TrackPlayer::new(feed, 44_100).with_stretch()
    }

    /// Renders to the end of the song, returning the left channel of every block that played.
    fn play_out(player: &mut TrackPlayer) -> Vec<f32> {
        let mut out = Vec::new();
        let mut left = [0.0; 64];
        let mut right = [0.0; 64];
        for _ in 0..10_000 {
            player.render(&mut left, &mut right, true);
            if player.is_finished() {
                return out;
            }
            out.extend_from_slice(&left);
        }
        panic!("the song never finished");
    }

    /// The pitch of the middle half of a tone, from its upward zero crossings.
    fn pitch_hz(samples: &[f32]) -> f32 {
        let middle = &samples[samples.len() / 4..samples.len() * 3 / 4];
        let crossings = middle
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f32 * 44_100.0 / middle.len() as f32
    }

    fn close(got: f32, want: f32, tolerance: f32) -> bool {
        (got - want).abs() <= want * tolerance
    }

    #[test]
    fn a_player_without_the_stage_ignores_key_and_tempo() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(8));
        let mut player = TrackPlayer::new(feed, RATE);
        assert!(!player.can_adjust());

        player.set_adjustments(6, 1.25);
        let (left, _) = render(&mut player, 4);
        assert_eq!(left, vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_song_at_its_own_key_and_tempo_bypasses_the_stage() {
        let (mut writer, feed) = feed(RATE);
        writer.push(&ramp(8));
        let mut player = TrackPlayer::new(feed, RATE).with_stretch();
        assert!(player.can_adjust());

        player.set_adjustments(0, 1.0);
        let (left, right) = render(&mut player, 4);
        assert_eq!(left, vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(right, vec![0.0, -1.0, -2.0, -3.0]);
    }

    #[test]
    fn a_key_change_moves_the_pitch_and_keeps_the_length() {
        for semitones in [-6_i8, 6] {
            let mut player = tone_player();
            player.set_adjustments(semitones, 1.0);
            let out = play_out(&mut player);

            let want = TONE_HZ * 2f32.powf(f32::from(semitones) / 12.0);
            assert!(
                close(pitch_hz(&out), want, 0.02),
                "key {semitones}: {} Hz, wanted {want}",
                pitch_hz(&out)
            );
            assert!(
                close(out.len() as f32, TONE_FRAMES as f32, 0.03),
                "key {semitones}: {} frames",
                out.len()
            );
            assert!(close(player.position_ms() as f32, 2_000.0, 0.03));
        }
    }

    #[test]
    fn a_tempo_change_moves_the_length_and_keeps_the_pitch() {
        for tempo in [0.75_f32, 1.25] {
            let mut player = tone_player();
            player.set_adjustments(0, tempo);
            let out = play_out(&mut player);

            assert!(
                close(pitch_hz(&out), TONE_HZ, 0.02),
                "tempo {tempo}: {} Hz",
                pitch_hz(&out)
            );
            assert!(
                close(out.len() as f32, TONE_FRAMES as f32 / tempo, 0.03),
                "tempo {tempo}: {} frames",
                out.len()
            );
            // The position is a place in the song, so it ends at the song's own length.
            assert!(
                close(player.position_ms() as f32, 2_000.0, 0.03),
                "tempo {tempo}: ended at {} ms",
                player.position_ms()
            );
        }
    }

    #[test]
    fn a_fed_song_never_reaches_the_engines_own_silence() {
        for (semitones, tempo) in [(6_i8, 1.25_f32), (-6, 0.75), (6, 0.75), (-6, 1.25)] {
            let mut player = tone_player();
            player.set_adjustments(semitones, tempo);
            let mut left = [0.0; 64];
            let mut right = [0.0; 64];
            let dry = |player: &TrackPlayer| {
                player
                    .stretch
                    .as_deref()
                    .map(|stretch| stretch.controller.underrun_frames())
            };
            // The first block carries the engine past its first window.
            assert!(player.render(&mut left, &mut right, true));
            assert!(left.iter().any(|s| *s != 0.0), "the first block is song");
            let warm = dry(&player);
            for _ in 0..400 {
                assert!(player.render(&mut left, &mut right, true));
            }
            assert_eq!(dry(&player), warm, "key {semitones}, tempo {tempo}");
        }
    }

    #[test]
    fn the_position_runs_at_the_tempo() {
        let mut player = tone_player();
        player.set_adjustments(0, 1.25);
        let mut left = [0.0; 441];
        let mut right = [0.0; 441];
        for _ in 0..50 {
            player.render(&mut left, &mut right, true);
        }
        // Half a second of output at tempo 1.25 is 625 ms of song.
        assert_eq!(player.position_ms(), 625);
    }

    #[test]
    fn a_change_during_the_song_keeps_the_position_it_had() {
        let mut player = tone_player();
        let mut left = [0.0; 441];
        let mut right = [0.0; 441];
        for _ in 0..50 {
            player.render(&mut left, &mut right, true);
        }
        assert_eq!(player.position_ms(), 500);

        player.set_adjustments(3, 1.25);
        assert_eq!(player.position_ms(), 500);
        for _ in 0..40 {
            player.render(&mut left, &mut right, true);
        }
        assert_eq!(player.position_ms(), 1_000);

        // Back in bypass the position steps over the frames the engine held, and no further.
        player.set_adjustments(0, 1.0);
        let resumed = player.position_ms();
        assert!((1_000..1_100).contains(&resumed), "resumed at {resumed}");
        assert!(player.render(&mut left, &mut right, true));
        assert_eq!(player.position_ms(), resumed + 10);
    }

    #[test]
    fn a_short_feed_stalls_an_engaged_song_instead_of_playing_silence_as_song() {
        let (mut writer, feed) = audio_feed(44_100, 44_100);
        writer.push(&vec![0.25; 2 * 2_000]);
        let mut player = TrackPlayer::new(feed, 44_100).with_stretch();
        player.set_adjustments(2, 1.0);

        let mut left = [0.0; 64];
        let mut right = [0.0; 64];
        for _ in 0..200 {
            player.render(&mut left, &mut right, true);
        }
        assert!(!player.is_finished(), "starved is not finished");
        assert!(player.starved_frames() > 0);
        assert!(
            player.position_ms() < 50,
            "the position waits with the sound"
        );
    }

    #[test]
    fn a_seek_while_engaged_lands_on_the_target_and_plays_on() {
        let (mut writer, feed) = audio_feed(44_100, 44_100);
        writer.push(&vec![0.25; 2 * 20_000]);
        let mut player = TrackPlayer::new(feed, 44_100).with_stretch();
        player.set_adjustments(0, 1.25);
        let mut left = [0.0; 64];
        let mut right = [0.0; 64];
        for _ in 0..20 {
            player.render(&mut left, &mut right, true);
        }

        player.seek_ms(1_000);
        assert_eq!(player.position_ms(), 1_000);
        player.render(&mut left, &mut right, true);
        assert_eq!(writer.pending_seek(), Some(1_000));
        writer.seek_complete();
        writer.push(&vec![0.5; 2 * 20_000]);

        let mut heard = false;
        for _ in 0..100 {
            heard |= player.render(&mut left, &mut right, true);
        }
        assert!(heard, "the song plays after the seek");
        assert!(player.position_ms() > 1_000);
        assert!(left.iter().any(|s| (*s - 0.5).abs() < 0.05));
    }

    #[test]
    fn a_dropped_consumer_tells_the_producer_to_stop() {
        let (writer, feed) = feed(RATE);
        assert!(!writer.is_abandoned());
        drop(feed);
        assert!(writer.is_abandoned());
    }
}
