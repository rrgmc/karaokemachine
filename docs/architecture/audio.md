# Audio — synth, sequencer, output

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## `km-audio` — sequencer, synth, transport

Our own sequencer rather than `rustysynth::MidiFileSequencer`, because the machine needs lyric-aware
position reporting, transpose, tempo scaling, seek, melody muting and queue-driven song changes.

- Step in units of `Synthesizer::get_block_size()` (64 samples, ~1.45 ms at 44.1 kHz): dispatch every
  event due in the block, render, interleave into the cpal buffer. Far tighter than any perceptual
  threshold for *when* a message lands — but a controller that changes and changes back inside one
  block is a change the synthesizer can only see if it counts, which is what the hold pedal does.
- **Tone adjustment** is a semitone transpose, ±6, applied to note keys and **skipping channel 9** —
  transposing drums changes which drum plays. A per-channel map of the transposition applied to each
  sounding key is kept so a `NoteOff` matches even if the transpose changes mid-note.
- **The guide-melody toggle** drops or admits `NoteOn` on the declared melody channel, always passing
  `NoteOff` and controllers so nothing sticks. Reset from the stored setting at the start of every
  song, so a toggle within a session does not carry to the next one while a *stated* preference does.
  `Machine::start` ands it with whether the song has a confidently detected channel, so with none
  declared the command is a no-op.
- **Seek replays controller, parameter and program state up to the target tick**, so instruments are
  not wrong after a jump. It replays only what the file states *before* the target, which is why the
  reset below is what makes it whole: a hold pedal pressed before the point being seeked away from and
  never mentioned again is invisible to the replay.
- **A registered or non-registered parameter is replayed as a parameter, not as six controllers.**
  Those six carry no value one at a time — a data entry means whatever the selectors in front of it
  last chose — so the scan tracks what each one *meant* and the replay states the parameter, the
  value, and then the selector state the file left. `Parameters` in `sequencer.rs` is that state
  machine, and it is the same one a synthesizer runs.
- **...and it collects that state into a fixed `[[Option<u8>; 128]; 16]`, because the replay runs in
  the callback.** `Command::SeekMs` is applied inside the cpal data callback, so a seek is subject to
  the hard rule at the top of this file's subject: no allocation. What was there grew a `Vec` and
  searched it linearly per controller event, making a seek O(events × controllers) *with
  reallocations in it* — tens of thousands of comparisons on a dense file, on the one thread that
  must return before the buffer runs out. That is an xrun rather than a glitch, and `xruns` was
  already counting them with nothing pointing at the cause. The table is 4 KiB of stack, O(events),
  and bounded by the MIDI standard itself. `Sequencer::sounding` had the same fault in smaller
  print: `Vec::with_capacity(32)` against a 256-voice pool, so the first dense passage of any song
  reallocated it on the audio thread. It is sized at `MAX_POLYPHONY` now.
- **Load, unload, restart and seek `reset` the synthesizer; pause and the end of a song only
  `all_notes_off`.** Not interchangeable: `note_off_all` kills voices and touches no channel state,
  so volume, pan, expression, hold pedal, patch and bend all survive into the next song. Measured on
  one real file that fades out on CC7 — channels 5, 6, 7 and 9 end at 1 or 2 of 127, about −42 dB —
  the next song was inaudible on four channels, drums included, unless it set CC7 itself. `reset`
  also mutes the reverb and chorus, which is why neither a pause nor the end of a song may use it:
  both let the tail render so it does not click.
- **The end of a song silences what is still sounding**, and does not move the transport out of
  `Playing`. A note with no note-off would otherwise sound for the life of the process: the sequencer
  early-returns for ever once it is finished, and a bare `Player` — `offline.rs`, `render_wav` — has
  no watchdog behind it to load or unload. The parser repairs most of these before they get here; the
  backstop is for songs that reached the player another way.
- **The queue state machine lives on the control thread.** The audio thread only raises "song ended";
  the control thread pops the next entry and loads it.
- Behind a `PlayerEngine` trait, so `km-api` can be tested against a stub with no audio device.

## A song's corrections, in the two places they have to be applied

The table `km_fixes::resolve` produces rides on `Load::Midi` beside the melody channel, and for the
same reason: `PlaybackSettings` carries over from one song to the next, and a correction belongs to
one file. The sequencer holds it and acts in four arms of `dispatch`: bank selects dropped on a
channel, a program substituted, a channel silenced beside the melody channel `is_muted` already
handles, and a centred bend sent before a note that would start on a bend the file left behind.

**It is carried inline where `Load::Track` beside it is boxed**, which looks backwards until
`Player::retire` is read: that drops the sequencer itself on the audio thread and hands back only the
`Arc<Song>`, on the stated grounds that a sequencer is small. A `Box` reaching the sequencer is
therefore a free in the callback — the thing `Vec::with_capacity(MAX_POLYPHONY)` and `seek_ticks`'
fixed tables exist to prevent. Flat and ninety-six bytes, it costs the command ring
`COMMAND_CAPACITY × the largest variant`: about 1.5 KB becoming about 7.7 KB, once, at startup. Buying
six kilobytes back with a deallocation in the audio callback is the wrong trade.

**The recentre fix keeps one tick per channel in the sequencer, and not the bend and its tick.** The
tick is `km_fixes::recentre_bend::stranded_from`: the first tick at which a note would start on a
stranded bend, or none while the bend is at centre. `Player` holds the sequencer inline in its
`Program` enum beside a video song's much smaller track. Clippy's `large_enum_variant` refuses a
difference above 200 bytes, and the answer it suggests, boxing the sequencer, is an allocation in
the callback. A bend value beside each tick crosses that line; the tick alone does not.

**`seek_ticks` bypasses `dispatch` entirely and every suppression has to be applied a second time
there.** It scans the event list itself, fills last-value-wins tables and emits into the sink
directly, so a filter written only in `dispatch` survives exactly until the first seek — which puts
back the bank select the fix removed and leaves the rest of the song playing the drum kit that
playing straight through never reaches. Switching the bank mid-song goes through the same replay, and
carries the loaded song's corrections rather than finding them again, so a hand-set mute is not
quietly lifted by a bank change. The recentre fix is restored the same way: the replay computes each
channel's stranded tick from the last bend it replays, so the first note after a seek is judged as it
would have been playing straight through.

That second application is what `a_seek_does_not_restore_a_suppressed_bank_select` asserts, with
`a_seek_replays_a_bank_select_that_is_not_suppressed` beside it so the absence is known to be caused
rather than vacuous.
## The display is only as smooth as the audio period

Reported from the sofa as "the lyrics scrolling seems a little slower than before, looks like low
FPS". It is not the renderer — the machine is a locked 60.0 fps drawing in 16.6 ms, idle *and* while
presenting a 1080p video, with not one dropped frame. What is chunky is the **position everything is
drawn from**:

```
period_size: 8192 @ 48000 Hz = 170.7 ms per audio callback
position_ms: 37113 → 37284 → 37455 → 37625   (steps of ~171 ms)
```

The `process` closure stores the position **once per callback**, and that store is the only thing that
moves it. So on a device handing over 8192 frames at a time the clock the display reads advances 5.9
times a second while the display redraws sixty times a second. The lyric wipe steps instead of
gliding, and the video picks the same picture for ten consecutive frames and then jumps — which is
exactly what "looks like low FPS" looks like, and is nothing of the kind.

**The two wrong attempts are the useful part.**

The first interpolated *forward* from the reported position by wall time. It is smooth and it is too
early, because the callback reports the position at the **end** of the buffer it has just filled and
that audio has not been heard yet — it plays over the *following* period. The report already leads the
sound, so sweeping forward adds a second period on top: up to **340 ms ahead of the music**.

The second smoothed the wrong counter. `position_ms` is what the *video* picks a frame with, but the
lyric wipe rides on `position_ticks` — a separate atomic with the same staircase. Smoothing only the
first left the scroll stepping exactly as before while the change looked correct in every log.

What works, applied to **both** counters:

```text
shown(t) = reported - step/2 + step * (t - t_report)/period
```

Three decisions in that, each of which an alternative got wrong:

- **The step is measured, not calculated.** Ticks per callback depends on the song's tempo map and the
  tempo ratio; milliseconds per callback depends on the ratio. Deriving either invites arithmetic
  nobody can check — `as_secs_f64()` is seconds, so an obvious derivation truncates to zero and still
  looks plausible in the log. The difference between two consecutive reports *is* the step, in
  whatever unit, at whatever tempo, so one type serves both counters.
- **Centered on the report, not forward from it.** The staircase showed `reported` for a whole period,
  so its mean was `reported`; a sweep from half a step below to half above has the same mean. That is
  what makes an offset judged by eye against a real television **still valid** — the change fixes
  motion without moving timing.
- **It joins up.** At `t_report + period` it shows `reported + step/2`, and the report arriving then
  starts at the same number, so there is no seam. A test asserts exactly this, because it is the
  property the whole design rests on.

A seek is caught by comparing the jump against **the step already established** rather than a
constant — the unit differs between the two counters, so a fixed threshold is meaningful in neither,
and the first version let a 3,830-unit jump through as a step. Backwards is always a seek.

**On a Mac this does nothing at all, correctly.** CoreAudio hands over 512 frames at a time, so the
period is 11 ms against the appliance's 170 — the staircase is about a third of one frame tall, and a
period that rounds to zero takes the early return and shows the raw report.

Worth knowing: **cpal reports this ALSA backend's true output latency as zero**, so there is nothing
to correct timing *with*. That is a property of ALSA here rather than of cpal — CoreAudio answers
honestly (194 ms beside an 11 ms period) — so on such a platform there would be a real number and the
offset would not have to be judged by eye at all.

### The instrumentation this needed, and why it stays

`km-display` could say nothing about its own frame rate, so a question asked from a sofa had no answer
short of squinting. `FrameMeter` reports once a second — fps, mean and **worst** draw, present and
interval times — plus a line naming the SDL renderer backend actually obtained and whether it took
vsync.

**The meter is asked for by name, and that correction is the point.** It was written as a `debug!`
under a default filter that had `km_app=debug` in it, which is to say it was on: every shipped build
wrote a frame report every second, forever, until somebody read a log and asked why. **The mistake was
not the level; it was reaching for a level at all.** A log level says *how much detail* you want about
what the program is doing; whether to run a measurement is a different question, and answering it with
a filter directive means the diagnostic is something you discover you have been collecting rather than
something you switched on. So the meter is built only when `--frame-stats` says so, and reports at
`info` — having asked for it by name, you should not then have to work out which level it hides
behind. Not building it also means nothing is measured, so the off case costs nothing.

**The worst-case columns are deliberate**: a stutter is a tail property, and thirty good frames plus
one bad one average out to a number that looks fine.

**Draw stops before `present`, and `present` is reported separately.** Folding them together on a
backend that waits for vsync inside `present()` makes the draw figure almost the whole frame
interval, and what it mostly measures is the wait. Vsync is requested, so that is every backend, and
the figure would be a number pinned to the frame interval on every healthy frame, which is exactly
the reading that means the machine cannot keep up. **A metric whose alarming value is also its
resting value reports nothing.** So the clock stops before `present`, `draw` is the build alone, and
`present_ms` carries the wait — which under vsync is the **slack** in the frame, since the two
together fill one interval. Watching `present` fall towards zero is watching the room run out. The
frame-padding sleep is still excluded from both, and the padding decision still uses their sum.

**Neither column can see a decoder, and that gap is what the decode fields fill.** On the Streamer
the display held an exact 60 fps through a stall somebody heard: `fps`, `draw` and `interval` were
all healthy, because they measure the loop and the loop was fine — what stopped was the thread
feeding it. So the `frames` line also carries `starved_ms`, `pictures_dropped`, `pictures_late` and
`xruns` as per-window deltas, and `starved_ms` is additionally reported at `warn` **with no flag**,
since a song that audibly stopped is a fault rather than a measurement.

**`xruns` is the one that belongs to this file's subject.** The other three describe a decoder on
another thread, which a MIDI song does not have — the synthesizer renders in the callback measured
above. So a MIDI song falling behind cannot starve; it misses the deadline and the device underruns,
and `xruns` counts that. **The 82% peak in the table above is the reason to watch it**: that is less
headroom than the video decoder had when it stalled, on the one thread that cannot be handed more
cores. Full account in
[`video.md`](video.md#what-decoding-costs-and-what-says-when-it-stopped).

**What that headroom was competing with, until recently, was the display.** During a MIDI song the
drawing thread took **88% of a core against the synthesizer's 20.7%** — four times the cost of the
thing it was starving, and almost all of it re-rasterising text that had not changed. Caching the
rendered strings took the display to 10.7%, so the margin the callback needs is no longer being spent
somewhere else. Measured across ~15 minutes of MIDI afterwards: **zero underruns.** See
[`video.md`](video.md#a-rendered-string-is-kept-not-remade-every-frame).

### What the three binaries share, and what they deliberately do not

All three ship a quiet default plus a `-v` ladder, with `RUST_LOG` winning over it, and the
`RUST_LOG` check is spelled out rather than left to `try_from_default_env` because the two differ on a
malformed filter and one behavior across three binaries is the point.

**The filter is duplicated on purpose, and so is `QUIET_DEPENDENCIES`.** Ten lines of `match` in three
binaries with no common dependency is cheaper than a workspace member existing to hold them. **A third
caller earns a crate, two do not** — and that rule was written down here in advance and then simply
applied: `km-osopen` became a crate when the machine's `F12` was the third caller. Folding it into
`km-console` was rejected on a boundary rather than on size: that crate is the workspace's single
`unsafe` exception and its scope is "whether anything printed will be read", so a process launcher in
it would be a *second exception* rather than a second caller.

**Size was never the axis — how badly a divergence would hurt is.** Ten lines of `match` fall one side
of it; a function with a platform-specific command line and a shell-quoting rule in it falls the
other. The bargain had already been paid once: the Windows quoting bug was found through one button in
one program and had to be fixed in the other from a report about a different button.

**For the package builder the rung was the smaller half of the win.** Its own debug stream is ten
sites on edge paths. What actually buried a scan was `symphonia`'s MP3 demuxer, which announces itself
at `info` on every MP3+G pair opened — and `info` is a level no verbosity change reaches. So it takes
the `QUIET_DEPENDENCIES` list too, and `km-remote` does not, rather than naming a crate that is not
there. **It holds that list one level lower than the machine does**, at `error`: the machine opens a
file per song and this opens every file in a folder, where the same crate's warnings are a line per
file over a meter that rewrites one line.

**Two formatting defects, both visible only on the appliance.** `tracing`'s formatter writes its own
timestamp and its own ANSI color, and journald stamps every line it receives — so each journal line
carried two times and a scattering of escape bytes. The two are decided separately because they are
different questions: **color** follows `stdout().is_terminal()`, so a deliberate console keeps it and
a pipe does not; **the timestamp** is dropped only when systemd says it owns the stream. Keying the
timestamp on "is a terminal" would have been wrong — a plain `> log.txt` is not one either and very
much wants the time.

### …and it can go to a file

`What a shipped build says out loud` is about **how much** a shipped build says. It never asked
**where**, and where is the half that was actually broken: all three binaries ship a GUI-subsystem
executable on Windows, and a process with no console has a null standard output handle — so `tracing`'s
writer does not fail, it *discards*. The whole run's log, silently. A machine that came up on a test
tone said why into nowhere, and the answer has been "run it from a terminal", which is advice for the
one state in which nobody can.

**`km-logfile` is a second layer and not a second writer**, and that is what buys the formatting
corrections. `MakeWriterExt::and` would have been one line and would have sent the console's bytes to
the file — escapes included, and under systemd without a timestamp, because the machine drops its
clock when journald will supply one. Right for the journal and wrong for a file, where nothing else
will stamp it.

**`log_internal_errors` must never be turned on**, and this is where that stops being theoretical: it
reports a failed write with `eprintln!`, which panics with no stderr, and a full disk is now something
that can fail a write in a double-clicked build.

**It is a crate where the log filter is not**, on the same axis: three copies of a retention policy is
three answers to "how many are kept?", and the one thing nobody would notice diverging is the one that
quietly fills a disk. It takes no dependency but `tracing-subscriber` — `tracing-appender` would be a
version to track and a policy to configure, and it rotates on the **clock** where this rotates per
**run**, which is the unit somebody actually asks about.

Two smaller things that had to be got right rather than chosen. The data directory is resolved
**before** the subscriber in all three, because that is where the file goes. And the timestamp in the
filename is UTC with a `Z`, because a local one needs a time zone database; the fifteen-line date
conversion is tested at 2000-02-29 and 2100-03-01, which are the two dates a wrong leap rule gets
wrong.

**A panic reaches neither of those destinations, so it gets a third.** The default hook writes to
stderr, which is the same null handle stdout is, and it does not travel through `tracing` — so the
file above stops mid-sentence at the last ordinary event, looking complete. `report_panics` installs
a hook that writes `<stem>-<UTC>.crash` beside the logs whether or not one was asked for, and emits
the same facts as an event for whoever has a console. Nothing in it may panic: a panic inside a
panic aborts, which would take the report with it. See the `A panic writes a file even when nothing
else does` decision.

**Retention is a count and `all` is one of its values**, through `--log-keep` on the machine and
`KM_LOG_KEEP` everywhere. Crash reports are counted separately from runs — `prune` matches on the
extension — so an evening of starting and stopping cannot push out the report of the panic that
ended one of those runs.

## Nothing leaves the callback outside full scale

`audio.rs`'s `limit` brings every block into range before cpal sees it, and it is there because the
second SoundFont survey found the failure it prevents rather than because anyone reasoned about it.

**The path almost everybody takes was the one with no limiter.** The `I16` and `U16` branches of
`Output::open` clamp on their way out because they have to scale anyway. `F32` handed the buffer to
the driver exactly as `Player::fill` left it — and `F32` is the native format on Windows, and on most
current ALSA and CoreAudio configurations.

**What can arrive there is not "a bit loud".** `Roland_SC-55.sf2` loads cleanly, plays five of the
research note's seven songs at an ordinary level, and on the other two reaches a peak of **2.2e19**.
That is a divergence, not a hot mix, and a machine wired into a PA plays it at whatever the amplifier
will do. `check_plays` does not catch it and could not: it renders one song, and five of the seven
are fine.

Two details worth keeping:

- **`clamp` alone would not have been enough.** `f32::clamp` returns NaN for a NaN input, so the same
  divergence arriving as a non-finite sample would pass straight through it. A sample that is not
  finite becomes silence, which is the only safe reading of a number that is not a number.
- **It is done once, after `fill`, for every format**, rather than in the two branches that already
  scaled. The two clamps below it are now belt-and-braces and are left alone; the cost here is a
  compare and a select per sample, against a stream that is 96,000 samples a second.

Offline rendering was never exposed to this: `offline.rs` clamps at the point it converts to `i16`,
which is why the survey could measure a 2.2e19 peak and report it as a number instead of producing it.

## `km-audio` could not play through ALSA's `dmix`

Pointing ALSA's `default` at a shared card — which is what a stock `default` *is* — made the machine
open the device, report an underrun within about 200 ms, and give up. It looked like an audio problem
and it was an error-handling problem.

**cpal recovers from an xrun itself**, and says so in its ALSA backend: an xrun is reported and then
repaired, and only `DeviceNotAvailable` ends a stream. Our error callback set `stream_failed` on
**every** error, so the control thread dropped a perfectly live stream over a transient that had
already been fixed. `dmix` underruns once while it primes, so the machine could not play through it at
all — and `dmix` is not an exotic path, it is the ordinary one.

The callback matches on `error.kind()`. Verified against the configuration that fails without it: two
xruns while dmix primes, both shrugged off, then zero in the following thirty seconds.

**The second bug was worse and is fixed with it.** Dropping a stream mid-song left the transport
atomic frozen at `Playing` — it is only ever written from inside the callback, and a dropped stream
leaves no callback — with the position stuck beside it. The machine reported a song playing forever,
at a standstill, silently: indistinguishable from a hang, in front of a room. `publish_stopped` is
called before an abnormal close. **The position is deliberately left where it was**: it is the last
true thing known about the song, and inventing a new one would be no improvement on the lie it
replaces.

**A useful consequence: the coarse audio period was self-inflicted.** The 170 ms period that made the
display step came from a `plug`-over-bare-`hw` workaround written to dodge this bug. With `dmix`
working, the stock `default` negotiates 21 ms. **The right fix removed the need for the workaround
*and* the symptom it caused.**

## The queue and the synthesizer are two different crates

`km-queue` holds what the machine is *doing* — queue, mics, transport, limits — where `km-audio` holds
what makes it happen. It takes `km-songcode` and `thiserror` and nothing else.

The reason is the same one `km-songcode` exists for one layer down: **a type that the API, the
display and both remotes all have to name cannot live inside any one of them.** With the queue in
`km-audio`, both remotes and the Android shell compile `rustysynth`, `cpal` and `rtrb` in order to
render a queue, and so does `km-display`, whose entire use of `km-audio` is two items.

- **`km-audio` re-exports none of it.** A `pub use` there would have been a one-line change with no
  call sites to touch, and it would have put the manifests back to lying. **A caller that wants a
  queue declares `km-queue`.**
- **Two dependencies disappeared rather than moving.** `km-display` dropped `km-audio` outright, and
  `km-audio` dropped `km-songcode`: the queue was the only thing in the engine that ever identified a
  song, and nothing names a song code to a synthesizer.
- **`DRUM_CHANNEL` deliberately stayed behind** while the transpose and tempo limits went. It is a
  fact about General MIDI that only something emitting MIDI messages has a use for; the other two are
  product decisions about what the buttons offer, which is why four crates validate against them.
- **The APK does not get much smaller, and that was never the claim.** The linker was already
  garbage-collecting cpal out of the remote's `.so`. What this buys is compile time, a dependency
  graph that says what it means, and the guarantee that the next thing added to `km-audio` cannot
  silently reach a phone.

## Which SoundFont, and what `rustysynth` leaves out

**Two facts about the synthesizer come first, because they change how the banks are read.**
`rustysynth` **implements SF2 modulators from 1.4.0**, which is one of the two reasons this workspace
takes a fork rather than the published crate — see
[`The synthesizer is a fork`](../decisions/audio.md#the-synthesizer-is-a-fork). And `km-audio` sets
exactly one field of `SynthesizerSettings` — `maximum_polyphony`, 256 (below) — taking block size
(64) and reverb and chorus (on) from the crate. Sample data is held as `Vec<i16>` and not widened, so
a bank's RAM cost is its `smpl` chunk.

**Everything measured in this section predates the fork**, and that has to be read into every number
in it: the published 1.3.6 read `pmod` and `imod` and threw them away, so these are banks with their
velocity, CC and filter response removed. The figures were also taken **at polyphony 64**, before it
was raised — and *that* half costs nothing: 1.3.6 at 256 reproduces the whole table to the decimal,
so the polyphony rise moves none of these aggregates. **The absolute numbers do re-run exactly**;
what moves them is the synthesizer.

The modulator gap was not neutral between the candidates, which is why it mattered: the bank whose
design leans hardest on modulators is the one the synthesizer ignored hardest. GeneralUser GS's
per-patch *level* balance survived — that lives in `initialAttenuation` generators, not modulators —
but its CC, velocity and filter response did not. **We were not hearing GeneralUser GS as its author
tuned it**, which was the one real argument in FluidR3's favor. **The measurement settles it**: a
bank moves if and only if its author wrote preset-level modulators, GeneralUser GS has 1,194 of them
and moved 1.1 LU of song-to-song spread, and the four banks with none — FluidR3 among them — did not
move at all. See [`The synthesizer is a fork`](../decisions/audio.md#the-synthesizer-is-a-fork) and
§11 of the research note.

Then it was measured: seven songs rendered 90 s each through three banks, through `ebur128` and
`astats`.

| bank | mean LUFS | range across songs | worst true peak |
|---|---|---|---|
| GeneralUser GS 2.0.3 | −22.2 | **6.7 LU** | −2.6 dBTP |
| GeneralUser GS 1.471 | −19.2 | 8.2 LU | −0.2 dBTP |
| FluidR3_GM | −18.2 | **12.0 LU** | **+0.2 dBTP** |

**FluidR3 clipped at the shipped default volume** — 3,787 samples pinned to full scale in 90 seconds
of one song. That alone settles the default.

**But the deciding number is the range, not the peak**, and it survives turning FluidR3 down. A
karaoke machine has its music-to-microphone balance set once, in hardware, and then plays a hundred
different songs at it. Song-to-song spread is therefore the property that matters, and FluidR3's is
nearly twice GeneralUser GS 2.0.3's. **A bank that needs the volume knob between songs is the wrong
bank here however good any one song sounds.**

**Set once is where the machine's own level control belongs**, which is why it is on `/admin/` behind
the owner's password and not among the four things a guest's phone may change. Setting it once means
being able to set it at all: the music side of that balance is the sound card's playback control, and
`Microphones — state, not audio` below is why the other side stays hardware.

## Two gains, and why they are two

`Player::fill` multiplies by `music_volume * song_gain`. The first is the owner's level; the second
brings one song to the reference — see
[`Video and MP3+G play at the MIDI reference level`](../decisions/audio.md#video-and-mp3g-play-at-the-midi-reference-level).

**Two in the callback, and a third outside it.** The level in the operating system's mixer is a gain
on the same signal and is not one of these: it is applied by the sound card, downstream of both
factors and of `limit`, and `km_audio::level` reaches it by asking the mixer rather than by
multiplying anything. `Nothing leaves the callback outside full scale` is a statement about the
samples, not about what arrives at an amplifier. See
[`The machine sets the level its output runs at`](../decisions/audio.md#the-machine-sets-the-level-its-output-runs-at).

**Folding them into one field was the obvious shape and is wrong.** `music_volume` is reported by
`GET /api/v1/settings`, mirrored into `ApiSettings`, and drawn as a slider on every remote, so a
per-song correction living in it would make that slider jump between songs and would write a
*measurement* into a key an owner sets by hand. Two fields and one product keeps each settable by
whatever owns it.

**`set_song_gain` clamps at `MAX_SONG_GAIN`, which is +12 dB, and the ceiling is what a MIDI song
needs.** A quiet MIDI song is the fault the levelling exists for and only a boost answers it; the
measured headroom is there, because a quiet MIDI song is sparse rather than compressed. It is still a
clamp rather than a trust: a wrong estimate must not reach the callback as an arbitrary multiplier,
and `limit` downstream would flatten an overshoot rather than report it.

**A media song never reaches past 1.0, and `gain_for` is what holds it there rather than the clamp.**
The two kinds of song have two rules for one reason: a finished master has no headroom to give. See
[`Every song plays at the level its bank renders the corpus at`](../decisions/audio.md#every-song-plays-at-the-level-its-bank-renders-the-corpus-at).

**A MIDI song's level is read from its events at load, and no bank enters the arithmetic.**
`km_song::Song::estimated_loudness_db` costs 0.19 ms a song and 1.6 ms at worst on a song already
parsed, and `km_loudness::midi_gain` turns it into a gain against one corpus constant. The estimate
predicts a song's level as `bank mean + (estimate − corpus mean)` and the target is the bank's mean, so
the bank appears on both sides and cancels. That is why `reference_lufs` is read for a media song and
not for a MIDI one.

**`Machine::start` sends it on every song, `1.0` included.** `Sticky` replays the last value it saw
into each stream it builds, so a start that sent nothing would leave the previous song's gain in
place; the case that breaks is the ordinary one, a song after a video playing 16 dB quiet for its
whole length. This is the same fault `volume_for_bank` exists for, in a second place — which is why
`Sticky` carries five commands rather than four. It matters more now that a gain can be above 1.0: a
stale boost would arrive on a song that had not asked for one.

**The reference is read from the bank that is sounding, not from one resolved at startup**, because
`Ctrl+1`…`Ctrl+9` can have replaced it since. `reference_lufs` joins a bank to its table row by
filename, exactly as `measured_level` does, and **falls back where `measured_level` abstains**: an
unknown bank's `music_volume` must be left alone, where an unknown bank's *reference* has to be
guessed at, because abstaining would mean no levelling.

### What the reference numbers are

Every figure below describes what leaves the callback, so it assumes unity in the sound card's own
mixer. A card sitting 20 dB down renders the whole table 20 dB quieter without changing one number in
it, which is why the machine reports that level rather than leaving it to be found with `amixer`.

`soundfont-banks.conf` carries a `lufs` per row. Re-measured for this at `rustysynth` `3e5ef8bd` with
`tools/dev/soundfont-measure.sh` over the asset cache — **the same run reproduced every row's recorded
`spread` to the decimal**, which is what says the loudness beside them was taken with the synthesizer
the machine renders with rather than with the one the note used.

| bank | mean LUFS | spread |
|---|---|---|
| generaluser (bundled) | **−21.9** | 7.8 LU |
| colombogmgs2 | −21.0 | 7.4 LU |
| sc55-v37 | −18.2 | 10.6 LU |
| somsak | **−12.4** | 6.1 LU |
| sc55 | **−26.6** | 9.3 LU |

**Fifteen banks span −12.4 to −26.6**, which is the whole argument for the reference following the
bank: a machine on `somsak` renders MIDI as loud as a commercial karaoke MP3, and one on `sc55`
renders it 14 dB quieter. A fixed target would be wrong by that much the moment anybody chose one.

**Measured at `music_volume: 1.0`, so a bank's own `volume` does not enter into it.** The owner's
level multiplies a MIDI song and a media song alike, so it cancels out of the difference between them;
counting a hot bank's reduction into the reference as well would apply it twice.

### What it costs, and what it is worth

Measuring is a full audio decode per media song at packaging time — **about 0.6 s each**, measured on
a 4.6-minute video and a 3.5 MB MP3: a build of the two went from 0.9 s to 2.1 s. A video's *picture*
is never decoded; its packets are dropped unread.

**That figure is optimized, and the gap to a debug build is the width of a mistake.** symphonia
decodes roughly thirty times slower unoptimized, which turns a 223-song MP3+G package from about two
minutes into seventy, with nothing on stdout while it runs because the output is block-buffered into
a pipe. `BUILDING.md` spells a measuring run in full for that reason rather than through the
`km-pack` alias, which is a debug build.

**That is why there is no `Measuring` progress event**, although one was planned. `BuildEvent::Encoding`
exists because a transcode takes *minutes* per song and a progress view that ticks per song looks
hung; sub-second work per song is invisible beside the tick it already gets.

**Neither measurement goes through a `TrackPlayer`**, and the trap is documented where it was found —
`Judging it by ear, and one trap in doing so` in [`cdg.md`](cdg.md). The live path decodes into a
quarter-second ring a real-time callback drains, and driving it faster than real time wins the race
the ring exists to lose: silence with a frozen position, so an unpaced loop measures a six-minute
song as five seconds of gaps. Both read the decoder instead, which the same note puts within
0.017 dB RMS of ffmpeg.

**The meter is `ebur128`, a port of libebur128** — the library ffmpeg's own `ebur128` filter is built
on, and that filter produced every LUFS figure in this file. So these numbers and those are the same
measurement rather than two that ought to agree. **libavfilter is not the alternative even though
ffmpeg is linked**: `tools/setup/ffmpeg-pin.sh` configures `--disable-avfilter`, so the filter is in no
library this workspace links and in no carrier it ships.

## Voice stealing, and why polyphony is 256

**`rustysynth`'s default of 64 counts voices, not notes.** `note_on` starts one voice per matching
instrument region, and each lives through its release envelope long after the note-off — so a
dense arrangement on the sustain pedal wants far more than its note count.
`rustysynth_regress diagnose voices <bank> <file> <pool>`, over one 28-track file with 334 CC64
events and about 33 notes sounding at once:

| Bank | peak voices at a pool of 256 | steals at 64 | at 128 |
|---|---:|---:|---:|
| TimGM6mb | **139** | 39 note-ons | none |
| FluidR3_GM | 111 | 57 | none |
| GeneralUser GS 1.471 | 83 | 5 | none |

**The densest bank wants more than 128**, which is what settles the constant: 64 saturates on every
bank measured, and a pool of 128 sits under the peak of the one that asks for most. How much a note
costs in voices is the bank's property, not the file's, and a machine plays whichever bank it is
pointed at.

**256, as a constant.** It costs one allocation (256 `Voice` × a 64-`f32` block, ~128 KB) and no
per-block CPU — `VoiceCollection::process` iterates only `active_voice_count`, so work tracks demand
and the cap merely bounds it. Nothing to tune, so no settings key. Only a constrained target failing
to keep up would reopen it: the appliance and the 32-bit `armeabi-v7a` build are where that shows.

**What a pool has to hold is bounded by the hold pedal being honored.** A pedal that lifts and
presses again inside one render block is a lift the synthesizer can still see, so the voices its
note-offs asked to release do not accumulate — see
[`Nothing is left sounding`](../decisions/audio.md#nothing-is-left-sounding). Without that, the same
file asks for 161 voices on GeneralUser GS and saturates 256 on the other two banks, which is demand
no pool size answers.

**And each of those voices got about 7.6% dearer** — the fork's 1.4.0 puts modulator support at
roughly that, by its own changelog. **Both Android targets have now been asked; the appliance has
not.** The audio callback thread's share of one core *is* the fraction of realtime the synthesizer
consumes, so it reads straight against the stream period. Peak of 2-second windows, GeneralUser GS at
48 kHz, over thirteen songs on a build carrying `274e2c6`:

| Target | Stream period | Peak | Headroom |
|---|---|---|---|
| `arm64-v8a` phone | 1600 frames / 33 ms | 34% of one core | ~2.9× |
| `armeabi-v7a` Streamer, Cortex-A55 | 5643 frames / 117 ms | **82%** of one core | **~1.2×** |

**No underruns on either** — zero `audio stream error`, and zero at the AudioFlinger track row —
including two full play-throughs. So 256 stands, and the television is where it would reopen: it
plays today with about a fifth of realtime to spare, and an A55 core costs roughly 2.4× what the
phone's big core does for this work.

**Track count is not the proxy for that peak.** The densest file in the sampled corpus, 49 tracks,
took 37%; the worst case was a 25-track arrangement carried on the sustain pedal — the same property
the table above turns on, and the reason a stress corpus has to be picked by sustain rather
than by size.

**None of that measurement needs a patched synthesizer.** `rustysynth_regress diagnose voices` reads
peak and mean demand off the pool it is given, and the fork exports
`Synthesizer::get_active_voice_count()`, so the same figure can be read off a running machine on
either target without building a second synthesizer to do it.

Two findings from the wider bank survey are worth knowing: several well-regarded banks **did not
load in `rustysynth` at all** — four of the fifteen surveyed — so "point `audio.soundfont` at a
better bank" was not the reliable escape hatch it reads as; and the one bank that beat the bundled
one on song-to-song spread is
206 MiB as `.sf2` and 38 MiB as `.sf3` — so **SF3 support is the single change that would reopen the
choice of bundled bank**.

**The first of those has been re-measured, and it changed.** Lenient loading (1.5.0) drops a
defective record instead of the bank, and re-opening all fifteen under it found **four of the banks
that could not be played now play**, ColomboGMGS2 having been refused over two empty loops out of 891
presets. Two files still fail and should — an SF3's Ogg `smpl` and a `sfpk` RIFF form are refused at
the container, before there is a record to drop — so `--set-soundfont`'s check below is unchanged in
purpose, and **SF3 is still the single change that would reopen the bundled bank**. Crisis General
Midi is the one file that could not be re-run, having no recorded source. The banks that now load are
still not *usable* by default: both Timbres of Heaven files clip all seven songs at `music_volume`
1.0, and the best of them cannot be shipped or pinned.

## Switching the SoundFont

The overlay could not do this job, and the reason is the safety property working as intended: **the
run somebody actually wants to listen to is the staged one**, and a staged binary has an `assets/`
sibling, so it gets no overlay. A bank in `local/assets/soundfont/` played under `cargo run` and was
silent everywhere else, saying nothing either way.

So the switch is `audio.soundfont` in `settings.json`. It holds a bank **id** now rather than a
path — `soundfont::resolve` looks it up in the folder and `resolve_soundfont` then picks between that
answer and the bundled candidates — so a name that no longer matches anything falls back to bundled
with a reason instead of refusing and leaving the machine on a test tone. That file is per-machine, is written by no staging script and is in no carrier, so it is local
by construction — the property `assets/` cannot have, and `dist/` cannot have either since it *is* the
deliverable.

**`--set-soundfont`'s order is the design**: absolutise, check it is a file, range-check the level,
**open the bank**, and only then write anything. Four of fifteen surveyed banks did not load, and a
bank can still fail its sanity check under the fork, so without that check the failure mode is a
machine that comes up on a sine test tone with the reason in a log — and the refusal quotes the
synthesizer rather than paraphrasing it, because "the RIFF chunk was not found" is a fact about this
synthesizer and not about the file being broken.

**A bank and a level are one decision**, since several banks exceed full scale at 1.0. The previous
level is stashed in a sidecar rather than a settings key, on the recurring argument that a
`music_volume_before_override` key would be a key an owner could set and it means nothing on its own.
Three rules that took a round of getting wrong:

- The stash is written **once** and only the applied value moves afterwards, so switching twice and
  then clearing restores the level from before the first switch.
- Restoring returns nothing when the current level differs from the applied one, because somebody has
  tuned it by hand since and it is theirs.
- **`--set-soundfont` consults the stash too**, which is the easy half to miss. Without it, switching
  from a bank needing 0.8 to one needing nothing leaves 0.8 behind — a property of the previous bank
  silently inherited by the next.

**The bank table is one file read by two scripts**, the same bargain `features.sh` and `apt-deps.sh`
make. Most rows are pinned by the host's own published sha1 rather than a computed sha256, which
attests the same bytes over the same origin; and two are published only inside a zip, which is why the
archive-member path exists — it verifies the archive and then the extracted member, because the second
check is what says `unzip` picked the file the table meant.

**The speed argument for that sha1 convention does not hold.** It rests on archive.org serving at
about 30 KB/s, which would put hashing 2 GiB locally at hours; §13 of the research note pulled
3.6 GiB from one item at **1.5 MB/s**. The
convention stands on its second leg alone, and two other places that decline to fetch something
because that host is slow are relying on a number nobody should plan around without re-measuring.

**The table is the survey now, and `rank` is what separates a shortlist from it.** It holds
sixty-three rows; nine carry a rank and are what `/dev/` opens on, and the rest are behind
`?all=true` there or reached from a shell. (Keeping the survey off a phone is not a job `rank` has:
the singer's remote has no Setup tab and lists no banks at all.) The file grew from 9 KB to 42 KB in
the binary, which is the price of not making the next person repeat a four-gigabyte download to
answer a question that has been answered. See `Which banks the machine offers` in
`docs/decisions/repository.md`.

**Every row was confirmed by byte count against the research note** before being written down. That is
not belt-and-braces over the digest: a digest says the bytes did not change in transit, and the size
says *this is the bank that note measured* — which matters because several of these names are shared
by unrelated soundfonts.

**An override is cached and never installed into `assets/soundfont/`**, which is the part to preserve:
that directory is copied into every carrier, so installing a 206 MiB override there would add it to
every release — and the bundled-name list does not include it, so it would not even be found.

**`--show-paths` gained a `soundfont` line**, closing a real gap: the report named the asset directory
and the overlay and never the file that won, which is the only thing that answers "why does this sound
wrong?". It calls `resolve_soundfont` rather than restating the order, and prints the failing case
too, since a machine with no bank plays a test tone and nothing else there says so.

## ...and switching it while a song plays

Everything above changes what the machine plays *at the next start*. The switcher —
[`Switching the bank while it plays`](../decisions/audio.md#switching-the-bank-while-it-plays) —
changes what this process is playing right now, from `Ctrl+1`…`Ctrl+9`, and it needed four things
that were not there.

**The synthesizer cannot be re-banked in place.** `rustysynth`'s `Synthesizer` holds its
`sound_font` as a private field set only by `new`, and builds its preset lookup from that bank's
presets — so there is no setter to add from outside the crate, even in [the
fork](../decisions/audio.md#the-synthesizer-is-a-fork). And the synthesizer is *moved into the cpal
callback*: `OutputStream::open` builds the source, `Player` owns it, and the closure owns the player.
Nothing on the control thread can reach it. So a swap is a new `Synthesizer`, which is a new stream,
which is `Held::close` and a reopen — the same path an idle release already takes several times an
evening.

**`Bank::load` stays on the control thread**, and `Job::SetSoundFont` carries the parsed `Bank`. The
audio thread has a 250 ms housekeeping cadence to keep and parsing a bank is tens of megabytes of
work; the same ordering also means a bank that will not open changes nothing, since it fails before
the job is ever sent.

**The song survives the rebuild because two mechanisms already existed.** `Machine` keeps the parsed
`Arc<Song>` in `state.loaded`, so re-sending it is one `Arc::clone` and one ring push — no disk, no
re-parse, no archive reopen. And `Sequencer::seek_ticks` replays the surviving program change, CC,
registered parameter, pitch bend and channel pressure for all 16 channels onto the sink, which is
what stops a virgin synthesizer coming back as sixteen grand pianos at default volume — and, since a
bank swap is the one seek nobody asked for, what stops swapping a bank mid-song retuning the parts
that lean on a pitch bend range. `Sticky::replay` covers the four
playback settings as it does after any reopen. So the sequence is `Load`, `SeekMs(position)`, `Play`,
and the position comes from `SharedState`, which outlives the stream.

**There is one swap and two callers.** `switch_soundfont(path, volume)` is the whole delicate part —
open the bank on the calling thread before anything changes, snapshot position and transport before
the stream goes, reload and seek after, level last — and it returns whether the stream was rebuilt so
the caller can tell "chosen" from "sounding". `switch_debug_soundfont` resolves a slot and calls it;
`Machine::select_soundfont` resolves an id from `data_dir/soundfonts/`, writes that **id** into
`audio.soundfont` through `--set-soundfont`'s own level protocol, and calls it. The two differ in *what they persist*
and in nothing else, which is the property that made a second copy of that ordering not worth having.

**`km_api::ops::set_soundfont` is where choosing happens for both API callers**, for the reason
`km-app`'s `api_failed` gives: the machine serving its own remote pages reaches the controller without
going through HTTP, so without a shared `ops` function there would be two translations of
`ControlError` into `ApiError` and two places publishing the event.

**The level is sent on every swap, and that is a fix rather than a tidy-up.** `Sticky` records the
last `SetMusicVolume` it saw and replays it into each new stream, so a swap that sends a volume only
when the slot has one leaves the previous bank's reduction in force over every slot that has none —
including slot 1, whose whole job is to be the reference the others are judged against. One leveled
slot therefore silently mis-leveled four of the eight on this box. `volume_for_bank` resolves the
slot's level or `audio.music_volume`, and the send is unconditional. Reading the machine's own level
back out of settings is sound here precisely because this path never writes it: the switcher sends
`SetMusicVolume` straight to the engine and never through `apply_settings`, so `audio.music_volume`
still holds what the owner chose, including a level moved from a remote, which is mirrored into it.

**Two things must *not* happen on this path**, and both are the difference between a swap and a
fault:

- **No `publish_stopped()`.** The transport atomic stays at `Playing` across the gap. The abnormal
  path calls it because a stream that vanishes mid-song would otherwise freeze the position and
  report a song playing for ever at a standstill; a deliberate swap is putting the song straight
  back, and reporting a stop would make the display and the API describe a machine that had lost it.
- **No queue interaction.** `Machine::poll` advances on `songs_ended` alone, which lives in
  `SharedState` and is untouched by a close, so nothing needs suppressing.

**`Engine.sound` moved behind a mutex**, joining `output` in being control-thread-written and
API-read. It was a plain field for a good reason — the bank was decided once at startup — and leaving
it one would have meant the startup log line, `GET /audio/soundfont`, `describe_soundfont` and the
on-screen label all going on naming the bank the machine started with. `Engine::sound()` returns a
clone rather than a borrow now, which is a `PathBuf` and a short defect list per call, and every
caller either formats it once or matches it once.

**A video or MP3+G song is the sharp case.** Its audio is a `TrackPlayer`, *moved* into
the audio thread, and `VideoSong::open` binds the feed writer to the decoder at open time — there is
no accessor to mint a second one from a live decoder. So dropping the stream would end that song
irrecoverably. But those songs run `Program::Track` and use no synthesizer at all, so there is nothing
to hear differently either: the bank is assigned with `close: false`, the stream is not touched, and
`Machine::settle_pending_soundfont` clears the pending flag once a MIDI song is playing through the
new bank. The label says which of the two states it is in, because "chosen" and "sounding" are
different claims.

**The keys needed a modifier where nothing else did.** `action_for` took a bare `Keycode`, and
`Ctrl+1` shares a keycode with the song-number keypad's `1` — so the function takes a `ctrl: bool`
and the Ctrl arm sits above the digit arms. A second entry point checked first in the event loop
would have left that precedence implicit and split across two files. A `bool` rather than SDL's
`Mod`, because Control is the only modifier any binding consults and shift is already spoken for by
the layout: `Plus` and `Less` arrive as themselves.

**`SelectSoundFont` is in the auto-repeat exclusion list**, beside `ToggleFullscreen`, `Escape`,
`Back` and `OpenPackagesFolder`. A held `Ctrl+2` would drop and rebuild the audio stream at the
repeat rate, and `ensure_open` gives up on a song after three failed opens — so the worst case is not
a stutter but a song that is simply gone.

**Which banks reach the eight slots is now a choice, not a consequence of the table's order.**
`tools/dev/soundfont-debug.sh` filled them by walking the table top-down and keeping the first eight
already cached — and the table is in rank order, so the slots were always the highest-ranked cached
banks and the other fifty-four were unreachable without hand-written specs. `--choose` draws a
checkbox list of every row through `tools/dev/km-pick`, which wraps `inquire`'s `MultiSelect` and
knows nothing about SoundFonts, and fetches what is ticked and missing through `fetch-assets.sh`. The
slot *order* is unchanged and deliberately so: the table's, so the two commands cannot disagree.

**`--show-debug-soundfonts` is what makes it a list you edit rather than one you retype.** It prints
each slot in exactly the form `--set-debug-soundfonts` reads — one flag, printing what the other one
parses — so the picker opens with the current slots ticked without any shell script learning to read
`settings.json`. A round-trip test holds the pair together. See `Choosing the debug banks from a list`
in `docs/decisions/repository.md`.

## What a bank costs on the television, measured

The whole picker — fetch, choose, swap, remove — had never been run on Android until it was, on a
Google TV Streamer: `armeabi-v7a`, API 34, 3.87 GB of RAM, and **no 64-bit ABI at all**, so this is a
32-bit process or nothing. Release build, one MIDI package installed.

This retires a caveat rather than adding one. An earlier estimate put a 200 MB bank out of the
question on Android, reasoning from 313 MB resident with FluidR3 — **and that
figure was taken from a debug build**, whose `libkm_app.so` is 227 MB against a release one's 11.8 MB.

| bank | file | resident | choosing it took |
|---|---|---|---|
| GeneralUser GS, bundled | 30.8 MiB | 158.7 MB | — |
| Aspirin 160 GMGS | 15.9 MiB | 143.3 MB | 0.39 s |
| ColomboGMGS2 Vanilla | 261.9 MiB | 394.9 MB | 3.75 s |
| SGM v2.01 GuitsPlusBass | 300.8 MiB | 427.9 MB | 4.38 s |

**Resident memory is the bank's file size plus about 125 MB**, which is what "sample data is held as
`Vec<i16>` and not widened" predicts. The largest bank the machine offers fits with room to spare;
nothing was killed, and no swap was refused.

**The transient is both banks at once, and it is the number that matters.** Switching between the two
largest — 261.9 MiB to 300.8 MiB, the worst case the offered nine can produce — peaked at
**674.2 MB**. `Bank::load` runs on the control thread while the old bank is still live inside the
audio callback, so a swap is never cheaper than the sum of the two.

**The trace also found a defect, which is why it is worth sampling rather than reading two
endpoints.** Resident memory climbed to 674 MB, fell back to 407, and climbed to 674 **again**:

```
394.9 … 394.9  414  436  457  480  502  524  551  565  583  606  629  642  656  670  674
407  412  462  515  573  608  643  672  673   521  412 … 410
```

Two full load-and-free cycles for one choice. `select_soundfont` called `check_plays`, which parses
the bank, takes its defect list and drops it — and then the swap parsed the same file again. The
check was right and the second parse was waste: `switch_loaded_soundfont` now takes the bank the
check already built.

**That swap is 4.38 s before and 1.45 s after**, and the trace has one climb where it had two:

```
382.4 … 382.4  397  455  517  575  627  646  664  674.6  672   411.7  411.7 … 410.8
```

**The peak is unchanged at 674.6 MB, and was never going to change** — it is the sum of the two banks
either way, because the new one is parsed while the old one is still live in the audio callback. What
went is the stall and the reading. Three times rather than the two the parse count implies, which is
the ordinary shape of this: the second parse was competing with the first for page cache on a device
whose storage is not fast.

### Buffering the bank read makes it slower — do not try it again

`Bank::load` hands `SoundFont::new` a bare `File`, and that looks like an obvious oversight: an SF2 is
parsed a field at a time, with `read_exact` on one-, two- and four-byte slices, so a bank with
thousands of records is tens of thousands of reads straight at the operating system. Wrapping it in a
`BufReader` is one line.

**It costs about 23%.** Four swaps each between the two largest banks, same build, same device:

| | to 261.9 MiB | to 300.8 MiB |
|---|---|---|
| bare `File` | 0.907, 0.928 s | 1.008, 1.006 s |
| `BufReader`, 64 KiB | 1.189, 1.128 s | 1.231, 1.187 s |

The reason is in `binary_reader.rs`: `read_wave_data` already reads the sample data in **64 KiB
blocks**. So the bulk of the file — all 300 MB of it — was never going one byte at a time, and the
only thing a buffer adds is a second copy of every sample. `BufReader` bypasses its buffer only for a
read at least as large as its capacity *and* only when the buffer is empty, which after header
parsing it is not; so the blocks are served through it and copied twice.

**A bigger buffer is worse, not better**, which is the trap worth naming: at any capacity above
64 KiB every block read is smaller than the buffer and can never take the bypass at all. The few
thousand header fields a buffer would help with are not worth one extra copy of the samples.

### A big bank is not just resident, it is playable

Fitting in memory would settle nothing on its own — the synthesizer already peaks near 82% of one
core on this A55 with the *bundled* bank, so the question is whether a bigger one leaves any. Playing
through the 300.8 MiB SGM, `top -H` puts the audio callback thread at **22.5% of one core** against a
stream period of 5,643 frames / 117 ms, which is the same period this device negotiates with any
bank.

**Zero underruns** — no `audio stream error` at all through playback and through swaps. Nine `Xrun`
lines were logged by an *earlier* run of the process, all of them around stream rebuilds, which is
the priming underrun `km-audio` was taught to shrug off rather than treat as a dead stream.

**A mid-song swap costs 1.01 s and the song keeps playing**, which is the promise
[`Switching the bank while it plays`](../decisions/audio.md#switching-the-bank-while-it-plays)
makes and the one that had never been tested on a television. The audible hole is a device close and
open, as designed; the transport never leaves `playing`.

Read the 22.5% against the 82% carefully rather than as an improvement: that figure is the peak of
2-second windows over thirteen songs chosen for sustain-pedal density, and this is one ordinary song.
What it supports is the narrow claim — **a bank four times the size of the bundled one does not
change the shape of the audio budget**, because a bank's cost is its resident samples and voice
count, not its file size.

### Fetching one, and the host that is not slow after all

The downloader works unchanged on Android — TLS through `ureq` with rustls and bundled
`webpki-roots`, so nothing depends on the platform trust store, and progress is reported throughout.
Every host in the table delivers megabytes a second to the device:

| source | rate to the device |
|---|---|
| huggingface.co | 5.4 MB/s |
| sourceforge.net | 6.8 MB/s |
| archive.org, cold file | 5.4 MB/s (40.4 MiB), 5.9 MB/s (52.4 MiB) |

**Nothing about the bank table needs changing on speed grounds**, which matters because a
single bad measurement says the opposite: 51 of the 63 rows come from one archive.org item, so a
throughput figure forty times too low makes the 103.4 MiB SC-55 look half an hour away and the table
look like a problem worth solving.

Three ways that measurement goes wrong, and the third is the one worth remembering:

- **n = 1.** One download of a session, never repeated. Re-run later, *the same file on the same
  device from the same host* fetches at about 2 MB/s — a fortyfold difference that no property of the
  host, the file or the device explains. One sample of a variable quantity is not a measurement.
- **A cross-check can be contaminated.** Timing a desktop against the same URL "at the same time" and
  getting 83 KB/s reads as independent corroboration. It is the opposite: the desktop and the
  television share one public IP, and the television was mid-download of that very file — the two
  measurements were competing with each other. `CLAUDE.local.md` carries this lesson for Docker
  benchmarks: *check for a peer before benchmarking, not afterwards*.
- **The obvious alternative explanation has to be tested rather than argued.** Once the re-runs come
  back fast the natural reading is caching, both re-tested files having been downloaded before. So
  cold files were timed — banks in the unranked survey never fetched from this network, one from a
  different archive.org item entirely. Cold runs at 745–884 KB/s from the desktop against a warm
  file's 887 KB/s, and 5.4–5.9 MB/s to the device. **A plausible mechanism that predicts the result
  is still not the cause**, which is the same trap the `autoMemoryReclaim` entry in
  `CLAUDE.local.md` records losing an afternoon to.

**And this file had already said so, 240 lines up.** [`The speed argument for that sha1 convention is
retired`](#switching-the-soundfont) records the same host measured at 30 KB/s and then at 1.5 MB/s,
and ends by warning that *two other places that decline to fetch something because that host is slow
are relying on a number nobody should plan around without re-measuring*. A third place was then added
below it. §13 of the research note makes the same point in the same words. The correction is
therefore not new information about archive.org; it is the existing note being read.

What survives is narrow and worth keeping: a download can still stall or die, and on a television
nobody can clear what it leaves behind. That is an argument for the `.part` sweep above, which is
already there, and it would be an argument for resume — but not one about any particular host.

### …and the second caller: a tick box from a setup program

`crates/machine/karaokemachine/src/firstrun.rs` is the whole of it: a `{bank, attempts}` file named
`first-run-soundfont.json`, written beside `settings.json` by the Windows setup program and the
macOS `.pkg`, read once per start by `Machine::start_first_run_soundfont` and deleted when it has
been carried out. The decision is
[`Offering the recommended bank at install time`](../decisions/distribution.md#offering-the-recommended-bank-at-install-time).

| | |
|---|---|
| Where | `config_dir` — `%APPDATA%\karaokemachine\config` on Windows, `~/Library/Application Support/karaokemachine` on macOS |
| Read by | `run()` in `lib.rs`, straight after `install_startup_packages`, before the API binds |
| Followed by | `Machine::settle_first_run_soundfont`, in the 50 ms poll loop beside `settle_pending_soundfont` |
| Ends in | `select_soundfont` — the same path the `/dev/` page takes, so the level protocol has one implementation |
| Attempts | three, counted in the file **before** the download starts, so a start that dies mid-download still spends one |

**It reuses `Downloader` unchanged**, which is what makes the whole feature small: the pinning, the
archive-then-member verification, the `check_plays` before the rename and the `.part` sweep are the
ones above, not a second set. The only new machinery is a `Mutex<FirstRun>` holding which bank this
start asked for, the last percentage put on screen, and one sentence for the display to draw —
`firstrun::Notice`, rendered through the same `Flash` as a dropped package.

**The id is checked against the downloader's status on every poll**, because the downloader is
shared with `POST /audio/soundfont/fetch`. Without that, somebody fetching a second bank from the
`/dev/` page while a first-start download runs would have theirs selected, or its failure reported,
as though it were the one the tick box asked for.

**The percentage can overshoot and is clamped.** For a bank published inside a zip — which the
recommended one is — the bytes counted are the archive's while the total is the extracted member's.

## Microphones — state, not audio

Mic audio is mixed in hardware, so `km-audio` has **no input stream**. It keeps a registry —
`{ id, name, device_hint, gain, reverb, echo, muted }` — persisted and exposed over the API with
change events. This is honest configuration state an external mixer can act on; it applies no DSP.
Kept behind a `MicBus` trait so a future opt-in passthrough feature could implement the same API
unchanged.

**The output level is the one mixer the machine does move**, and the boundary holds because the two
are different halves of the balance. `km_audio::level` reads and writes the sound card's own playback
control, which is the music side; the microphone side stays hardware the machine describes and never
touches. What `A karaoke machine has its music-to-microphone balance set once, in hardware` asks for
is that the ratio be set once and left, and an owner who cannot reach either half is an owner who
cannot set it at all.
