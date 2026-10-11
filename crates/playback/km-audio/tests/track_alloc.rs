//! A recorded song's player allocates nothing on the audio thread, its stretcher included.
//!
//! The stretcher is a dependency, and its promise of an allocation-free `process` is second-hand
//! until something here counts. This file holds one test, because a global allocator belongs to the
//! whole test binary.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use km_audio::track::{TrackPlayer, audio_feed};

thread_local! {
    /// Whether this thread is inside the part of the test that stands for the audio callback.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /// Allocations this thread made while counting.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// The system allocator, counting what the test thread asks for while it is told to.
struct Counting;

#[expect(
    unsafe_code,
    reason = "a global allocator is an unsafe trait, and this one only forwards to the system's"
)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        // SAFETY: the caller's contract is the system allocator's own, passed through unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from `System.alloc` above with this layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const RATE: u32 = 44_100;
const SECONDS: usize = 8;

/// A tone, which has few spectral peaks.
fn tone() -> Vec<f32> {
    (0..RATE as usize * SECONDS)
        .flat_map(|i| {
            let s = (i as f32 * 0.06).sin() * 0.4;
            [s, s]
        })
        .collect()
}

/// Noise, which has about as many spectral peaks as a spectrum can hold.
fn noise() -> Vec<f32> {
    let mut seed: u32 = 12_345;
    (0..RATE as usize * SECONDS * 2)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            ((seed >> 9) as f32 / 4_194_304.0 - 1.0) * 0.4
        })
        .collect()
}

fn play(player: &mut TrackPlayer, blocks: usize) {
    let mut left = [0.0; 64];
    let mut right = [0.0; 64];
    for _ in 0..blocks {
        player.render(&mut left, &mut right, true);
    }
}

#[test]
fn a_key_and_tempo_change_allocates_nothing_on_the_audio_thread() {
    for (name, samples) in [("a tone", tone()), ("noise", noise())] {
        // Everything that may allocate happens first, as it does on the control thread.
        let (mut writer, feed) = audio_feed(RATE, RATE as usize * SECONDS);
        assert_eq!(writer.push(&samples), samples.len());
        let mut player = TrackPlayer::new(feed, 48_000).with_stretch();

        ALLOCATIONS.with(|count| count.set(0));
        COUNTING.with(|counting| counting.set(true));

        play(&mut player, 50);
        player.set_adjustments(3, 1.0);
        play(&mut player, 200);
        player.set_adjustments(3, 1.25);
        play(&mut player, 200);
        player.set_adjustments(-6, 0.75);
        play(&mut player, 200);
        player.set_adjustments(0, 1.0);
        play(&mut player, 50);
        player.set_adjustments(6, 1.25);
        play(&mut player, 200);
        player.set_adjustments(-6, 1.25);
        play(&mut player, 200);
        player.set_adjustments(6, 0.75);
        play(&mut player, 200);
        player.seek_ms(1_000);
        play(&mut player, 10);

        COUNTING.with(|counting| counting.set(false));
        let allocations = ALLOCATIONS.with(Cell::get);

        assert!(player.position_ms() >= 1_000, "{name}: the song played");
        assert_eq!(allocations, 0, "{name}: the audio thread allocated");
    }
}
