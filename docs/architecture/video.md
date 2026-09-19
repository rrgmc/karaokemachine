# Video songs

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## `km-video` — decoding

The only crate in the workspace that *decodes* with ffmpeg, for the same reason `km-audio` is the
only one that names `rustysynth`: every other crate's manifest goes on showing its real footprint,
and one feature turns the whole C dependency off.

**`km-stream` is the other one that names it**, and the split is by direction rather than by
convenience: this decodes a song's own picture into planes bound for a texture, and that encodes the
screen the machine drew into something a television elsewhere can play. Neither takes the other, and
each carries its own `ffmpeg` feature — so a build that wants one does not compile the other.

**It decodes from a path or from anything seekable**, which is how a packaged song reaches it.
`ffmpeg-next` wraps the custom-I/O context in a safe API, so that costs this workspace no `unsafe` and
needs no ffmpeg rebuild anywhere — **which matters, because ffmpeg's own `subfile:` protocol is absent
from the Android build**. With custom I/O there is no filename to probe from, so the entry's name is
passed as the format hint.

**One thread per loaded song** demuxes the file and drives both decoders, feeding two very different
consumers: audio into a lock-free ring, pictures into a bounded channel of pooled YUV420 frames.
**The video decoder itself is threaded**, though — `Type::Frame` with `count: 0`, so ffmpeg spawns a
worker per core; without that it runs on one, which is the fault the measurement section below is
about. **The audio is what carries time** — a video song's position comes from samples the device has actually
consumed, exactly as a MIDI song's comes from the sequencer. Audio-as-master-clock falls out of the
machine's existing design rather than being imported alongside it.

Five things worth knowing before changing any of it:

- **The device's sample rate never reaches the decoder.** Samples are pushed at the *file's* rate and
  the player resamples on the way out. This is what makes the decoder a pure function of the file: the
  output device chooses its rate at open time and chooses again after every idle release, and coupling
  a decoder to a decision made that much later and elsewhere is the risk the research note recorded as
  needing a resampler. **It needs one, but on the player's side**, where the rate is known and the
  buffers are already sized.
- **Both buffers are sized from one lookahead constant, and they have to be.** One thread demuxing
  both streams reads them in timestamp order, so if the two queues hold different amounts of *time*
  the shallower fills first and the demuxer stops to wait for it. When that was video it
  **deadlocked**: audio is the clock, a starved ring freezes the position, a frozen position means
  every queued picture is still in the future, and the display only takes pictures whose time has
  come. **Nothing drained.** Found by the end-to-end test rather than by reading, and fixed by making
  it unrepresentable rather than by tuning.
- **Frames are recycled, never reallocated.** A 1080p YUV420 frame is 3.1 MB; allocating one thirty
  times a second and freeing it on whichever thread drops it is the kind of churn that surfaces later
  as a stutter nobody can place.
- **Nothing is converted to RGB.** The three planes upload to an SDL `IYUV` streaming texture and the
  GPU converts while it draws. Converting on the CPU would cost a full-frame pass to arrive somewhere
  worse.
- **Seeking decodes forward and discards**, so the position cannot lie about where the sound is. That
  is also why the packaging profile does *not* need a short keyframe interval for accuracy: the
  interval only decides how much is decoded and discarded, which is tens of milliseconds of CPU.

**`measure_loudness` is a third entry point beside `probe` and `open`, and it decodes the audio and
nothing else.** Video packets are recognised by index and dropped unread, so it costs a demux and an
audio decode rather than a play-through — about 0.6 s for a 4.6-minute song. It reuses the same
`swresample` conversion to interleaved stereo `f32` that playback does, which is what makes the
recorded number the one the gain will be applied to; a mono file matters here, because swresample
scales on the way to stereo where `km-cdg` duplicates, and each path measures whatever it will play.
See
[`Video and MP3+G play at the MIDI reference level`](../decisions/audio.md#video-and-mp3g-play-at-the-midi-reference-level).

**`probe`'s promise to read the shape without decoding anything is intact** — this is a separate
call, made only where a level is wanted, and a test still holds that `probe` decodes nothing. **The
decoder is flushed with `send_eof` at the end**, which is easy to leave out and costs the tail of
every song: the decoder holds frames back, and what it holds is small on a four-minute file and is
not nothing.

## What decoding costs, and what says when it stopped

Measured on a Google TV Streamer (MediaTek MT8696, four Cortex-A55 at 2.0 GHz, `armeabi-v7a`,
1920x1080 at 60 Hz), by sampling `/proc/<pid>/task/*/stat` and `dumpsys SurfaceFlinger --latency`
from outside the process — so the figures are of an uninstrumented build, not of one measuring
itself.

| | Share of one core |
|---|---|
| `km-video-decode`, 640x360 H.264 Main 30 fps at 313 kbps | 10% |
| `km-video-decode`, **1920x1080 H.264 High 29.97 fps at 926 kbps** | **80%** |
| the display loop, **whatever is playing** | 58% |
| the whole process | 132% of the four cores' 400% |

**Those are the single-threaded numbers**, taken before frame threading — the section below is the
correction and the reason. They are kept because they are what a decoder confined to one core costs,
and because the 80% is what makes the fault inevitable.

**1080p30 holds real time on this box either way.** That is the measurement that was always going to
decide the decoder, and it decides it in favor
of software: `h264_mediacodec` is not needed. On one thread it holds with a fifth of a core to spare,
which is not enough; spread over the four the box has, it is not close.

Three things follow.

- **The display loop's cost is not the picture.** 58% of a core on a 360p file and on a 1080p one
  alike, so it is the lyric and wallpaper drawing rather than the texture upload — which is what
  "nothing is converted to RGB" above buys. Presentation held **exactly 60 fps with no dropped
  vsync** across 252 sampled frames; note that this is what the *compositor* showed, and the loop was
  meanwhile drawing about twice that. See the run-3 bullet below.
- **Decode scales with bitrate more than with pixels.** Eight times the pixels cost eight times the
  CPU here, but only because the bitrate rose with them; 926 kbps is *low* for 1080p — which a
  karaoke video, being a caption over a fairly static picture, tends to be. **The shape of the file
  matters more than its resolution**, so a resolution ceiling is the wrong lever to reach for first.
- **The margin is `LOOKAHEAD_MS`, and it is 250 ms.** One core at 80% left a fifth of a core for
  every hard passage in the file, and a passage exceeding it for a quarter of a second emptied the
  audio ring. `TrackPlayer::stall` then freezes the position, deliberately, so the picture waits with
  the sound instead of drifting out of sync — which is what the stall looked like from the sofa.

### The decoder was single-threaded, and that was the whole fault

A stall was reported on the appliance: sound and picture freezing together, several times, in one
1080p song. Four runs of the same file settled it.

| run | decode | display | `starved_ms` | `skipped` |
|---|---|---|---|---|
| 1 | 1 thread | 118 fps | **871** | 134 |
| 2 | 1 thread | 118 fps | **452** | 146 |
| 3 | 1 thread | **62 fps** | **260** | 120 |
| 4 | **frame threading** | 118 fps | **0** | 188 |
| 5–8 | frame threading | 118 fps | **0, 0, 0, 0** | — |

Runs 5 to 8 are four further plays of the same file on a clean build with the meter off, watched by
somebody in the room as well as logged; nobody saw a stall and nothing warned.

**`km-video` never asked ffmpeg for threads**, and ffmpeg's own default is `thread_count = 1` — the
command line turns frame threading on for itself, a library caller does not get it. So one Cortex-A55
carried 1080p H.264 at 80% while three sat idle, and any passage harder than average emptied the
250 ms ring. Setting `Type::Frame` with `count: 0` spawns `av:h264:df0…df4`, five workers at ~16% of
a core each: the same total work, a five-fold drop in the per-core peak, and **no starvation at all**
across a full song.

**`count: 0` means "ask the machine", so the answer differs per machine — and it is logged for that
reason.** `the video decoder's threading threads=N kind=Frame`, once per song at `info` -- `debug` is
unreachable on a retail Android TV, which was found by printing it there and seeing nothing. Five on the
appliance's four cores; **sixteen on a 24-core desktop**, ffmpeg capping automatic frame threads
there. It is the only number in this crate the code does not decide, and without it printed, a
question about behavior on another machine cannot be answered from the source. One was asked and
could not be, which is why it is there.

**Frame threading does not lag a big machine, and the argument that says it does is the sort that
gets made again.** It delays output by roughly `thread_count` frames, so sixteen threads at 30 fps
reads as ~500 ms — twice `LOOKAHEAD_MS` — and therefore as a picture running a quarter-second behind
the sound on any desktop. Measured on a 24-core box with 16 threads: `pictures_late` **0**,
`starved_ms` **0**, three mid-song seeks clean, a steady 60 fps. **The error is confusing pipeline
*frames* with wall-clock *time*.** A pipeline delays output by N frames *of input*, but the decoder
does not run at 1× — it runs flat out until audio backpressure stops it, so those sixteen frames cost
milliseconds. Steady-state throughput is 1:1 and there is no lag at any thread count.

**Seven runs of the one file settle it: 3/3 starved single-threaded, 0/4 threaded.**

**And an eighth run settles the device the 80% was measured on** — the four Cortex-A55s where the
fault is inevitable, rather than the appliance or the 24-core desktop. The same file, packaged the
same way, on a **Google TV Streamer** over `armeabi-v7a`: `threads=5 kind=Frame`, the five workers
`av:h264:df0`…`df4` present in `/proc/<pid>/task`, all 273 seconds played, **`starved_ms` 0** and
nothing warned. Presentation held 59.5 fps with draw at 2.6 ms.

That is the run that matters most of the eight, because it is the only one taken where a single core
was demonstrably not enough.

| build | runs | `starved_ms` |
|---|---|---|
| single-threaded | 3 | 871, 452, 260 — every run |
| frame-threaded | 4 | 0, 0, 0, 0 |

The repetition is not ceremony. Of roughly fifteen songs played on the appliance single-threaded only
one stalled, so **the base rate of the fault is about one song in fifteen** and a single clean run
proves almost nothing — most songs do not stall either way. Repeating *the file that always stalls* is
what makes the result mean something: it is the one input known to provoke the fault, it did so on
every single-threaded run, and it stopped doing so on every threaded one.

**All of those songs are the same kind of file**, which is what makes the comparison fair rather than
flattering: every one came from `km-pack`'s own transcode, and so did the one that stalled — its
`libx264` High profile, `yuv420p` and faststart index are exactly what `-preset medium -crf 20`
emits.

**What the profile does not bound is decode cost, and the reason is CRF.** `-crf 20` targets
*quality*, so bitrate follows content: the file here is 926 kbps because a karaoke video is mostly a
caption over a near-static picture, and a busy one from the same command would be several times that.
The packaging profile guarantees the decoder meets one *format* and says nothing about how much work
per second it will be handed — which is why the one file at the hard end of the corpus was the one
that stalled. Threading turned that from a 250 ms cliff into headroom; it did not make the cost
predictable.

Two things about how that was found are worth keeping, because both were wrong first.

- **The display was the wrong suspect, and run 3 is the evidence.** The loop drew at ~118 fps into a
  60 Hz screen — `MIN_FRAME` is 8 ms and `into_canvas()` did not request vsync — so roughly half of
  what it drew was discarded by the compositor. Capping it at 62 fps did free ~20% of a core, and
  starvation continued. It could not have helped: with four cores and ~1.3 in use, CPU was never the
  scarce resource. **A single-threaded bottleneck does not care how much of the other three cores you
  hand it.**
- **Run 3 is not evidence the cap is worthless, either.** 260 against 452 looks like an improvement
  until you notice runs 1 and 2 were the same build and differed by a factor of two. One run per
  condition cannot separate that. The waste was real and was worth its own measurement, which it has
  since had — see [the display's own rate](#the-display-draws-at-the-screens-rate-and-idle-costs-more-than-video)
  below. Threading is what fixed the stall.

**Three counters say it happened**, and were added because a stall was heard and nothing in the
machine could confirm it:

| Counter | Where | Means |
|---|---|---|
| `starved_ms` | `TrackPlayer`, reported by `OutputStream::collect_retired` | the sound ran dry — the song audibly stopped for this long. **The one that tracks the fault** |
| `dropped` | `VideoCounters`, at the decoder's `try_send` | the queue was full, so the display was not taking pictures. **Normal and constant in a headless run** |
| `skipped` | `VideoCounters`, in `FrameReader::take_frame_for` | a picture came due while it waited. **Not a fault — see below** |

**`starved_ms` warns with no flag at all**, because a song that audibly stopped is a fault the owner
already heard rather than a diagnostic they opted into; a healthy song reports zero and says nothing.
It earned that: it read 871, 452, 260 and 0 across the four runs above, tracking the fault exactly.

**`skipped` deliberately warns about nothing**, and that is the measurement here most likely to be
rediscovered the hard way. The position it compares against advances one audio callback at a time,
and that period is 117 ms on the appliance — so about three and a half frames of a 30 fps video come
due at every step, `take_frame_for` keeps the newest, and the rest are counted. It runs at **0.44 to
0.65 a second on a healthy song** and did not move when the audible fault went from 871 ms to zero.
Gating a warning on it, which this did at first, would have warned about every video song ever
played. It stays as a *rate to compare against that baseline*, never a count to react to. The
picture-side warning is gated on `dropped` instead, with `skipped` used only to prove a display was
attached at all — a headless run takes no frames, so it can never have skipped one.

All three ride the once-a-second `frames` line when `--frame-stats` asks for it, as per-window
deltas, alongside a fourth that belongs to the other song kinds.

### The same counters cover MP3+G, and MIDI needs a different one

**MP3+G is the same machinery and needs nothing added.** `km-cdg` spawns `km-cdg-decode`, fills the
same `audio_feed` sized from the same `LOOKAHEAD_MS`, and is played by the same `TrackPlayer` — so a
pair that could not be decoded in real time reports `starved_ms` exactly as a video song does. It is
in no danger of the fault above: MP3 decode costs a per cent or two of a core against H.264's 80, so
there is roughly fifty times the headroom and nothing to parallelise. *(The warning said "the video
song's audio ran dry" until an MP3+G reading of it was noticed; it names no kind now, because this
side cannot tell them apart.)*

**MIDI is the exception, and it has the least headroom of the three.** There is no decoder thread and
no feed — `rustysynth` renders inside the audio callback — so a MIDI song cannot starve and
`Player::starved_ms` returns 0 for one by construction. What it does instead is miss the callback's
deadline, and the device underruns. That is counted by `xruns`, which exists because the underrun was
already *logged* once per event and a log line cannot distinguish three in a second from three in an
hour.

Read it against [`audio.md`](audio.md): the synthesizer peaks at **82% of one core on the appliance's
Cortex-A55 against a 117 ms period, about 1.2× headroom** — *less* than the 80% that made a video song
stall. And unlike the video decoder it cannot be given more cores, because the work is in the
real-time callback by design. **MIDI is therefore the tightest of the three song kinds on this
hardware**, and `xruns` is the only number that will say so.

## The display draws at the screen's rate, and idle costs more than video

`window.into_canvas()` takes SDL's default, which is **vsync off**, and SDL3 dropped SDL2's
`SDL_RENDERER_PRESENTVSYNC` creation flag — so a renderer presents as fast as it is asked to unless
something says otherwise. Nothing did. `MIN_FRAME`'s doc comment claimed vsync was requested and was
simply wrong, for years, which is easy to see how: at idle a frame costs about 15 ms on the appliance
and the loop settles near 65 fps by itself, so the claim matched the observation everywhere except
where it mattered.

`display.rs` now asks, in `request_vsync`. `sdl3` 0.18.4 wraps no vsync setting for a renderer, so it
is one FFI call to `SDL_SetRenderVSync(renderer, 1)` whose result is reported beside the backend name:

```
the SDL renderer backend renderer=opengles2 vsync=true
```

**Plain vsync rather than `SDL_RENDERER_VSYNC_ADAPTIVE`.** Adaptive tears instead of waiting when a
frame runs late, and on a television showing words over a picture a torn frame is worse than a
repeated one — somebody is reading it. **A refusal is not an error**: the call returns `false` on a
driver that will not take the value, that is logged, and `MIN_FRAME` goes back to being the only
pacing there is. That is the fallback role its doc always claimed and had never actually had.

**Granted on both renderers it has met**: `opengles2` on the appliance and `direct3d11` on a Windows
desktop, where a frame reads `draw_ms=0.3 present_ms=16.3 interval_ms=16.7` — 2% of the budget spent
building and the rest waiting, which is the split doing its job. **The untested case is a display
faster than 125 Hz**: vsync does not cap the loop where a `MIN_FRAME` floor would, so on a 144 Hz
panel this *raises* the draw rate rather than lowering it. Nobody has run one.

### What it saved, and what it did not

Measured on the Streamer by differencing `SDLThread`'s jiffies in `/proc`, same method and same file
either side, on release builds:

| | before | after | saved |
|---|---|---|---|
| playing a 1080p30 song | 40.5% of one core | **28.9%** | 11.6 points, −29% |
| **idle, showing nothing** | 95.9% of one core | **87.4%** | 8.5 points, −9% |

**The second row is the finding, and it was not what this change went looking for.** The idle screen
costs *more than twice* what decoding and drawing a 1080p video costs, and vsync barely touches it —
because idle was never the case running away with the frame rate. A frame takes ~15 ms to build there
against ~5.5 ms during a song, so idle was already near 65 fps and vsync only trims it to 60; video
was at 118 and is now at 60. **The machine's largest steady cost was drawing an ordinary screen**, and
nothing in this section addresses that. It is addressed in
[the text cache](#a-rendered-string-is-kept-not-remade-every-frame) below, which came out of asking
what a 15 ms frame is made of.

### The frame meter had to change to survive this

**`draw` must mean build alone, not build *and* present.** With vsync granted `present` blocks until
the next refresh, so a frame that took 2 ms to build and one that took 12 ms leave it at the same
moment: a combined figure sits at the frame interval on every healthy frame, which is exactly the
reading that means "cannot keep up". **A metric whose alarming value is also its resting value
reports nothing** — and this one has a job, being what clears the display of blame during a stall
hunt.

So the clock stops before `present`, and `present_ms` / `present_worst_ms` join the line. Under vsync `present` is the **slack**: draw and present together
fill one interval, so watching present fall towards zero is watching the machine run out of room.
`MIN_FRAME`'s padding decision still uses both together, because the question it asks is whether the
whole loop body came in under the floor.

## A rendered string is kept, not remade every frame

The naive path rasterises every string with SDL_ttf, uploads it as a GPU texture, draws it and
destroys it — **every frame, for every string, at 60 fps.** `free`'s own doc describes it ("a texture
that was made for one frame") without saying what it costs.

**It costs most of the display.**

| | before | after |
|---|---|---|
| a MIDI song | 88% of one core | **10.7%** |
| the idle screen | 87.4% | **29.4%** |

### The asymmetry that found it, and the profile that shaped it

Nobody profiled first. The four song kinds were measured on the appliance and two of them were cheap:

| what is on screen | display CPU | the words are… |
|---|---|---|
| MIDI song | 88% | rendered **text** |
| idle screen | 87% | rendered **text** |
| 1080p video | 29–32% | **pixels** in the video |
| MP3+G | 23% | **pixels** in the CD+G bitmap |

All four draw a full-screen background texture every frame — a wallpaper, a video frame, a CD+G
bitmap. The only structural difference is that the top two rasterise text. **The ~60-point gap is the
text**, and that is a within-machine comparison with the background cost held constant, which is
stronger than the desktop-to-appliance ratio that first suggested it.

`simpleperf` then said *which half* to attack, and its answer is the reason this caches what it does.
During a MIDI song `make_texture` was about 38% of the app's CPU, split two to one the way round
nobody guesses:

| | share of app CPU |
|---|---|
| `SDL_CreateTextureFromSurface` → `glTexSubImage2D` → `ioctl` | **25%** |
| `TTF_RenderText_Blended` | **13%** |

**Uploading the texture costs twice what rasterising the glyphs does.** So the cache holds
`Texture`s. A cache of rendered *surfaces* — the obvious design, and the one that was nearly built —
would have recovered the smaller third and left the larger alone.

### Why MIDI gains more than idle

The lyric line is the biggest texture on the screen and now lives as long as the line does. **The
wipe was the worst of it**: it draws the line twice, once pending and once sung, and the sung copy
was a whole second rasterisation and upload every frame. It is a clip over a cached texture now, so
the highlight crossing a line costs nothing at all.

Idle keeps 29% because it **crossfades a wallpaper** every frame, which no text cache touches. That is
now the largest single display cost, and unlike the text it is real drawing rather than waste.

The effect on the audio is the point of the whole exercise: the display was **four times the
synthesiser's cost** (88% against 20.7%) on a box where the synthesiser has about 1.2× headroom inside
a real-time callback. That margin now belongs to the audio.

### The two things that make it safe

**It is bounded, because the unbounded version is a crash this project has already had.** The
per-frame textures leaked once: `GL mtrack` reached 2.6 GB and Android's low-memory killer took the
app down about ninety seconds into a song. The cache is that same failure with a slower fuse, so it
evicts least-recently-wanted above four megapixels and frees what it drops through the same `free`.
Measured after: `GL mtrack` **70 MB and steady**, and RSS *lower* than before — constant
create-and-destroy churn cost more than a bounded cache holds.

**Font identity is the sharp edge, and the compiler cannot hold it.** `sdl3`'s `Font` keeps its
`TTF_Font` handle private with no accessor, so the key carries the *address* of the font that drew the
string; two faces of the same size are otherwise indistinguishable, and keying on metrics would draw
one font's glyphs for another. An address is an identity only while the fonts stay put, and **they do
not**: the display rebuilds `Fonts` on a resize, freeing faces whose addresses the new ones can be
handed straight back. The cache would then answer with the old size's glyphs — text that is subtly
wrong, on a path nobody exercises often. Borrowing `&Fonts` would have made that unrepresentable and
is impossible, because the display owns its fonts by value and reassigns them. **So the guard is a
call, not a type**: `TextCache::clear` at the rebuild site, said at both ends. It is the first thing
to check if this code is ever moved or reused.

## The profile is checked before it is enforced

A packaged video is H.264 in 8-bit 4:2:0, at most 1080p30, AAC, in MP4 — the point being that the
appliance's decoder only ever meets one thing. **But most files already are that**: a download asked
for AVC and AAC arrives in the profile, and the first real song needed nothing done to it. So the
check runs first and an empty result means *copy the bytes*. **Had the design said "always transcode",
the very first song would have been re-encoded to produce a slightly worse picture.**

**One constraint is a requirement and the rest are preferences**, and keeping them apart is what makes
the reporting useful:

- **The pixel format blocks.** The frame copy takes three planes with the chroma at half height — 8-bit
  planar 4:2:0 and nothing else — and there is no `swscale` and no color conversion of any kind. A
  4:4:4 source has its chroma truncated; a 10-bit one is read a byte per sample. **Neither fails: both
  draw a wrong picture, which is the hardest kind of fault to place.** So anything outside the
  supported set is refused, and re-encoding is the remedy rather than the normalizer.
- **The refusal is made twice, because the first one reads a claim.** `open` takes the format from
  the stream's `AVCodecParameters`, which are written by whoever made the file and are settled
  **before a single frame has been decoded**. A bitstream may disagree with them — an H.264 sequence
  header declaring no chroma decodes to a one-plane picture inside a stream whose parameters say
  `yuv420p` — so `Frame::fill_from` asks the *decoded* picture what it is and refuses there too.
  What that second check is worth is specific: `ffmpeg_next`'s `Video::data` **panics** on a plane the
  frame does not have, and the panic unwinds the decoder thread **past `writer.finish()`**, so the
  audio feed never reaches its end and the machine sits on a song that cannot finish. A refusal ends
  the song; a panic hangs it. `fill_from` also reads each plane's `stride` *before* its data, because
  a stride is `linesize` cast to `usize` and ffmpeg spells a bottom-upwards picture with a negative
  one — which arrives as a number near `usize::MAX` and is the only thing here that could make a
  slice out of a raw pointer go wrong.
- **Everything else is a preference.** VP9, 60 fps, 4K, a `.webm` container — all play, and are
  re-encoded for predictability and disk rather than refused. `--no-transcode` keeps such a file as it
  is and **still refuses an unplayable one**: switching off a re-encode is a statement about CPU, not
  a license to package a song the machine will skip while somebody is holding the microphone.

**`probe` stays permissive and `open` refuses**, which pull in opposite directions on purpose:
packaging has to read a file's shape in order to decide it needs re-encoding, so a probe that failed
on precisely the files needing a transcode would put that decision out of reach.

**The profile says nothing about the audio sample rate**, and that is a deliberate consequence of how
the decoder works: it resamples to interleaved stereo `f32`, so every codec, rate and channel count
already plays. Requiring 48 kHz would re-encode a good file to fix a problem the runtime does not
have.

**The frame-rate ceiling is compared in milli-fps**, so the ubiquitous 29.97 passes a 30 fps test
instead of failing it by rounding.

### `libx264` is not always there

x264 is GPL, so the LGPL ffmpeg this project deliberately develops against does not carry it; such a
build ships `libopenh264` instead. **A profile that named `libx264` could not re-encode anything on
the development machine, and the failure would have looked like a broken transcoder.** Both are tried
in order and the one used is reported.

**Hardware encoders are excluded** even though an LGPL build lists several. `ffmpeg -encoders` reports
what was compiled in, **not what the machine's GPU and driver can actually do**, so choosing one
automatically turns a missing graphics card into a failure in the middle of a batch — and each takes
different rate-control options, so one profile could not aim at the same quality through them.

Two defects the first real run found, neither visible by reading: **ffmpeg picks its muxer from the
output's extension**, and the transcoder writes through a temporary `.part` name, so every re-encode
died with `Unable to choose an output format` — which reads like a broken input rather than a naming
detail. And **a progress line that rewrites itself only means anything on a terminal**; piped to a log
it was a hundred copies of itself.

## Rebuilding the songs table

Thirteen columns were `NOT NULL` while every song was a MIDI file, and a video has none of them.
SQLite cannot relax `NOT NULL` with `ALTER TABLE`, so the table is rewritten once: **the only
migration in the tool that is not an `ADD COLUMN`.**

**The suitability columns are not among the thirteen.** A scan fills them for every kind, because a
song made to be sung to is answered by what it is and by how long it is sung for. Storing rather than
inventing is what keeps the browse list, the band filter and the suitability sort reading one number:
the latter two read the column, so a number worked out on the way out shows on the page and means
nothing in the `WHERE` clause beside it.

The rewrite is split around `schema.sql` — rename before, copy after — so the new table comes from the
**one** definition rather than a second copy of thirty columns free to drift. Four details, each of
which would quietly do damage:

- **`legacy_alter_table` is on for the rename.** Modern SQLite helpfully rewrites *other* tables'
  foreign keys to follow a renamed table, which here would repoint four tables at the set-aside copy —
  precisely the opposite of what is wanted.
- **The old table is renamed, never dropped.** With foreign keys enforced, a `DROP TABLE` counts as
  deleting every row: it would blank every file's link to its song and empty every favorite.
- **The copy's column list is derived from `pragma_table_info`, not written out.** A hand-written list
  is how this kind of migration loses somebody's corrections — **one forgotten column and every
  hand-typed title is gone, with nothing to say so.**
- **`rowid` is carried across, and that is what keeps both FTS indexes untouched.** The first version
  took "a rebuilt table hands out fresh rowids" as a fact rather than a choice, emptying both indexes
  and letting triggers refill them — on the real corpus, every title and all of its lyric text
  retokenised to produce an index whose contents did not change. Nothing outside FTS refers to that
  rowid. **The test for it is load-bearing**: without it a later tidy-up that dropped `rowid` from the
  list would leave a corpus that browses perfectly and cannot be searched, with every other test
  passing.

**The indexes are dropped twice, for two different reasons.** Once because they follow the rename
keeping their names, so `CREATE INDEX IF NOT EXISTS` would find the name taken and **quietly skip
it**, leaving the new table unindexed. And once around the copy itself, because otherwise it inserts
a whole corpus through nine secondary B-trees in content-hash order, which is random I/O.
Both lists are **derived from `sqlite_master`** rather than written out, for the same reason the column
list is.

**The resume path is the easy one to get wrong.** An interrupted rebuild leaves the rows set aside
and no `songs` table at all — because the migration runs *before* `schema.sql` — so a recovery branch
that counts rows in `songs` is counting a table that does not exist yet.

## What the model shape says

MIDI-only fields group into one `Option` around thirteen fields that arrive and are absent together,
rather than thirteen that let a caller invent a combination which cannot happen. The browse row keeps
its two flat, because **the grouping is worth its indirection at thirteen fields and not at two.**

A distinct "not a video" scan status joins "not MIDI" because the two are reached by different routes
and **read differently to a person**: telling somebody their `.mp4` is "not a readable MIDI file"
sends them looking for a problem that is not there.

In the browse list a video's suitability column shows its stored number in the verdict color every
other song gets, because it is the same number answering the same question. The song page prints the
four components beside it under a line saying the file is native karaoke, which is what tells a
reader they are a derivation rather than a reading.

**Auditioning needed the debug play route to stop parsing MIDI unconditionally.** Judging a video
otherwise meant packaging it first, which is the wrong way round for a tool whose whole purpose is
deciding what deserves to become a song. The extension test moved into the package crate, which is
where all three crates that ask can reach it, and is deliberately **outside every `#[cfg]`** — so a
build without the video feature recognizes the file and says it cannot play video rather than
reporting a perfectly good MP4 as an unreadable MIDI file.

## Container tags

`VideoInfo` gained a title and artist read from the container, **mapped to `None` when blank** —
blank rather than absent is the common case, because a muxer asked to embed metadata it does not have
writes the key with an empty value, and an empty string propagated onward is a song titled nothing at
all.

The fallback chain lives in **one place**, the only place a video enters a package, and resolves
person → tag → stem.

**One consequence**: an incremental scan settles a file by its size and modification time, so a corpus
scanned by a build that did not read container tags keeps its blank artists until those rows are
scanned again. There is nothing to fix — **but it is the kind of thing that reads as a bug when a folder is
rescanned and nothing changes.**