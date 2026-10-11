//! What each candidate stretcher does to a tone, checked against what was asked of it.
//!
//! The stretchers are development dependencies of the `stretch` example, and this reads the
//! example's own code, so the two cannot disagree about how a candidate is driven.

#[path = "../examples/stretch/engines.rs"]
mod engines;

use engines::Engine;

const RATE: u32 = 44_100;
const TONE_HZ: f64 = 440.0;
const SECONDS: usize = 2;

/// A stereo sine, interleaved.
fn tone() -> Vec<f32> {
    (0..RATE as usize * SECONDS)
        .flat_map(|i| {
            let phase = 2.0 * std::f64::consts::PI * TONE_HZ * i as f64 / f64::from(RATE);
            let sample = (phase.sin() * 0.5) as f32;
            [sample, sample]
        })
        .collect()
}

/// The frequency of the left channel's middle half, from its rising zero crossings.
fn frequency(samples: &[f32]) -> f64 {
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    let middle = &left[left.len() / 4..left.len() * 3 / 4];
    let crossings = middle
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    crossings as f64 * f64::from(RATE) / middle.len() as f64
}

/// Asserts a render has the length and the pitch its key and tempo ask for.
fn assert_renders(engine: Engine, semitones: i8, tempo: f32) {
    let input = tone();
    let (samples, _delay) = engines::render(engine, &input, RATE, semitones, tempo)
        .unwrap_or_else(|error| panic!("{engine:?} failed: {error}"));

    let expected_len = input.len() as f64 / f64::from(tempo);
    let len_error = (samples.len() as f64 - expected_len).abs() / expected_len;
    assert!(
        len_error < 0.03,
        "{engine:?} key {semitones} tempo {tempo}: {} samples, expected {expected_len:.0}",
        samples.len()
    );

    let expected_hz = TONE_HZ * engines::pitch_ratio(semitones);
    let hz = frequency(&samples);
    assert!(
        (hz - expected_hz).abs() / expected_hz < 0.02,
        "{engine:?} key {semitones} tempo {tempo}: {hz:.1} Hz, expected {expected_hz:.1}"
    );
}

#[test]
fn a_key_change_moves_the_pitch_and_keeps_the_length() {
    for engine in [Engine::Wsola, Engine::Wide, Engine::Batch] {
        assert_renders(engine, 6, 1.0);
        assert_renders(engine, -6, 1.0);
    }
}

#[test]
fn a_tempo_change_moves_the_length_and_keeps_the_pitch() {
    for engine in [Engine::Wsola, Engine::Wide, Engine::Batch] {
        assert_renders(engine, 0, 1.25);
        assert_renders(engine, 0, 0.75);
    }
}

#[test]
fn the_time_domain_corrector_holds_the_pitch_inside_a_fifth_of_the_tempo() {
    assert_renders(Engine::Keylock, 0, 1.2);
    assert_renders(Engine::Keylock, 3, 1.0);
}
