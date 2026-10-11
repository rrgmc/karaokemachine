# Research: key and tempo for a recorded song

**Research only.** What was decided is
[`A recorded song changes key and tempo`](../decisions/audio.md#a-recorded-song-changes-key-and-tempo),
and how it is built is in [`docs/architecture/audio.md`](../architecture/audio.md). Where this note
and either of those disagree, they hold.

A recorded song is a video, an MP3+G pair, or an UltraStar or LRC file with its audio.

Measured on 2026-10-11 with `crates/playback/km-cdg/examples/stretch`. **One quick listen found no
render bad, and nothing here ran on a television or an appliance.**

**Summary.** The change is possible in pure Rust, with no C dependency. One stretcher serves both
controls, and the player already holds the other half of the work. The `timestretch` crate can do
it from an audio callback at a few per cent of one desktop core. Its cheaper mode stops at four
semitones, so the machine's range of six needs its phase vocoder. The `wsola` crate costs more and
allocates on every call.

| Marker | Meaning |
|---|---|
| **[measured]** | A figure the example printed, or a test asserts. |
| **[source]** | Read from the crate's source at the version named. |
| **[docs]** | The crate's own documentation. Second-hand. |
| **[web]** | A crate's registry page. Second-hand. |
| **[heard]** | A person listened to the renders. |
| **[repo]** | Read from this repository. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. One stretcher gives both controls

A stretcher changes the length of the audio and keeps its pitch. A resampler changes the length and
the pitch together. A song at tempo `T` and pitch ratio `P` is therefore the audio stretched to
rate `T / P` and then resampled by `P`. A key change of `n` semitones has `P = 2^(n/12)`.

**[repo]** `TrackPlayer::render` in `crates/playback/km-audio/src/track.rs` already resamples every
recorded song, from the file's rate to the device's rate. Its step is one number. A key change
multiplies that step by `P`, so the stretcher is the only new stage.

**[repo]** Every recorded kind plays through that one function. The video decoder and the MP3
decoder both write a feed, and `TrackPlayer` reads it.

The machine's limits set the range a stretcher must cover. `MAX_TRANSPOSE` is six semitones, so `P`
runs from 0.707 to 1.414. The tempo runs from 0.75 to 1.25. The stretch rate `T / P` therefore
runs from 0.53 to 1.77 when both controls sit at their limits.

## 2. The candidates

| Candidate | Method | Licence | Callable from the audio callback |
|---|---|---|---|
| `wsola` 0.1.0 | Overlap-add in the time domain | MIT | No. **[source]** `push` grows a `Vec`, and `pull` returns a new one. |
| `timestretch` 0.15.0, `Keylock` | A sinc resampler, then a time-domain pitch corrector in two bands | MIT | Yes. **[docs]** `EngineProcessor::process` is allocation-free and lock-free. |
| `timestretch` 0.15.0, `WideKeylock` | A phase vocoder over the whole spectrum | MIT | Yes, by the same statement. |
| `timestretch` 0.15.0, `stretch` and `pitch_shift` | The same graph over a whole file, with formant correction on the pitch shift | MIT | No. Each takes the whole file. |

**[source]** `timestretch` forbids `unsafe` code. Its real-time engine has three halves: a feed, a
controller and a processor. The feed is a lock-free ring, which is the shape `AudioFeed` has.

**[source]** The real-time engine has a tempo control and no pitch control. A key change is that
tempo control plus the resampling in section 1. The whole-file `pitch_shift` works the same way.

**[measured]** `timestretch` brings six crates the workspace does not hold: `rustfft` and five
under it or beside it. It also carries beat tracking, key detection and file readers that playback
would not call.

**[web]** Three other crates were set aside without a render. `ssstretch` and
`signalsmith-stretch` wrap a C++ library. `pitch_shift` has had no release since 2022.

## 3. What each candidate did

The source was a song of 232 seconds, synthesized from a MIDI file and encoded to MP3 at 192 kbit/s.
It has no voice in it. Each render ran in a release build on one core of an arm64 desktop
processor.

**[measured]** Processor time, as a share of the length of the audio rendered:

| Candidate | Key −6 to +6, tempo 1.0 | Tempo 0.75 to 1.25, key 0 | Delay |
|---|---|---|---|
| `wsola` | 8.7% to 17.5% | 12.3% to 12.5% | 0 ms |
| `timestretch`, `Keylock` | 1.8% to 2.9% | 1.9% to 2.4% | 12.7 ms |
| `timestretch`, `WideKeylock` | 4.6% to 5.0% | 3.7% to 6.2% | 0 ms |
| `timestretch`, whole file | 4.2% to 6.9% | 2.5% to 6.6% | none |

**[measured]** Every render came out within 0.02 seconds of the length its tempo asks for. A
position counted in source frames would therefore stay true over a whole song.

**[measured]** `Keylock` does not reach the machine's key range. At six semitones either way a
440 Hz tone came out at 440 Hz. At four semitones it came out within 0.5 Hz of the right pitch, and
at tempo 0.75 and 1.25 it held 440 Hz. **[docs]** The crate states full correction to 20% of the
tempo and none beyond 35%, and six semitones is a stretch rate of 41%.

**[measured]** `wsola`, `WideKeylock` and the whole-file functions put a 440 Hz tone within 2% of
the right pitch at six semitones either way. `crates/playback/km-cdg/tests/stretch.rs` asserts it.

**A tone says nothing about how music sounds.** A recording's key change moves the drums and any
backing voice with the melody.

**[heard]** One quick listen across the renders found none of them bad. It compared no engine
against another, and it judged no setting on its own.

## 4. What is not measured

- **The television and the appliance.** [`audio.md`](../architecture/audio.md) records the
  synthesizer at 82% of one core on the television, so a MIDI song is the tight case there.
  **[inferred]** A recorded song has that core nearly free, which is the budget a stretcher would
  spend. No figure here says how much of it.
- **A video song at a faster tempo.** The picture follows the audio position, so tempo 1.25 asks
  the decoder for 25% more frames each second. [`video.md`](../architecture/video.md) records
  1080p decoding at 80% of one core on the television.
- **Allocation.** The statement for `timestretch` comes from its documentation. Nobody ran its
  processor under an allocator that counts.
- **A voice.** The source song has none.
- **A change during playback.** Every render held one key and one tempo from start to end.

## 5. What a decision would have to settle

1. **Whether the sound is good enough, and over what range.** A recording could take a narrower
   key range than a MIDI song.
2. **A setting that turns the feature off.** A machine with a slow processor needs one. With the
   setting off, a recorded song answers as it does today.
3. **The song with neither control moved.** At key 0 and tempo 1.0 the audio can skip the
   stretcher. The song then sounds and costs exactly what it does today. **[measured]** `Keylock`
   adds 12.7 ms, so entering and leaving it during a song moves the audio by that much.
4. **The position clock.** **[repo]** `TrackPlayer::position_ms` counts output frames. With a
   tempo change it must count source frames. The picture, the CD+G graphics, the word highlight
   and `quiet_from_ms` all read it as a position in the song.
5. **The lyric offset.** **[repo]** `shift_ticks` scales the offset by the tempo for a MIDI song
   and holds it at 1.0 for a timed recording.
6. **The refusal codes.** **[repo]** `no_key_*` and `no_tempo_*` exist for each recorded kind, and
   `transpose_available` and `tempo_available` are true for a MIDI song only.
7. **The dependency.** `timestretch` would enter the audio path of every build, the `--no-video`
   build and Android included, because MP3+G has no feature to hang it on.
8. **The missing entry.** Three entries in `song-sources.md` cite `No audio-file pitch shifting`,
   and `docs/decisions/` has no entry with that heading.

## Reproducing

```sh
cargo run --release -p km-cdg --example stretch -- <song.mp3> out.wav --engine wide --key -3
cargo run --release -p km-cdg --example stretch -- <song.mp3> out.wav --engine keylock --tempo 1.1
cargo test -p km-cdg --test stretch
```

The engines are `wsola`, `keylock`, `wide` and `batch`. The example prints the length, the
processor time and the delay of each render.
