# MP3+G songs

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

An MP3+G song is two files with the same stem: an `.mp3` with the backing track and a `.cdg` with the
words as graphics.

**This crate has no cargo feature, and the absence is the design.** `km-video` has one because ffmpeg
is a C dependency a workspace build must not require. Nothing here is optional in that sense — the
renderer is ours and the MP3 decoder is pure Rust — **so there is no build that can catalog an
MP3+G song and refuse to play it.** That is what makes MP3+G work in a `--no-video` build and on
Android. A future reader will otherwise take the missing feature gate for an oversight, so it is
commented in three places.

**Both halves come out of the package, and the asymmetry between them is this crate's own**: the MP3
is seeked into through a window and the `.cdg` is read whole, because 2.6 MB is held anyway when every
seek replays from packet zero.

## The format, and three things a document will not tell you

24-byte packets at exactly 300 a second — 75 CD sectors times four subcode packs — so a stream's
length is its size over 24 over 300, and **nothing in the file states it**.

- **Every byte is masked with `0x3F`.** A `.cdg` is a copy of the R–W subchannel and real rips keep
  the P and Q bits in the top two. Every file examined here had them set, **so masking is the format
  and not a defensive measure**. A test feeds a stream twice, once with the bits forced on, and
  demands identical output.
- **The surface holds palette indices, not colors.** Loading a palette recolors everything already
  drawn, which is how many discs fade words in and out without redrawing a tile. Resolving pixels at
  draw time would make the instruction do nothing, **and it would present as "the fades are missing"
  rather than as a bug.**
- **A four-bit channel expands by multiplying by 17, not by shifting left four.** 15 has to become
  255; `15 << 4` is 240, and **a palette whose white is 94% white looks like a tired display and gets
  blamed on one.**

## Pulled by the display, not pushed by a thread

`km-video` needs a decoder thread, a bounded frame queue, a pool and a lookahead, because a full
picture queue must never stall the demuxer — that demuxer also carries the audio, and a headless run
takes no pictures at all.

**None of that applies here, because nothing pushes.** The screen is advanced by whoever wants a
picture, so a headless run simply never advances it. There is no queue to fill, no frame to drop and
no backpressure to get wrong.

That leaves the awkward-looking part: **a CD+G surface is cumulative**, so packets cannot be skipped
and a backward seek has nowhere to start from but the beginning. **The measurement is what makes that
a non-problem.** The whole corpus — 2,849 files, 208 million packets — replays in **4.5 seconds warm,
about 46 million packets a second**, which puts a six-minute song's complete rebuild at roughly **two
milliseconds**. So there are no keyframes, no snapshots and no "which snapshot was that" bugs: the
whole file is read into memory at load and a rewind replays from zero.

**The rewind tolerance is the one trap.** The display smooths the position between audio callbacks, so
it can overshoot and step back. A rewind check with no tolerance would rebuild on that wobble — a
hitch on every drawn frame, **appearing as a flicker nobody could place.**

## What the corpus said, and the two assumptions it destroyed

Over 2,849 files and 208 million packets: **nothing unreadable, nothing that drew no words, and no
panic.**

The value of that run was not the pass. It was that **both intuitive definitions of "corrupt" turned
out to be wrong**, and each would have shipped a packaging check that refused songs that work:

- **A command byte that is not 9 is not damage — it is another subcode application.** One file looks
  like garbage in a hex dump, is 74% such packs, and renders three clean lines of karaoke.
- **A CD+G instruction nobody implements is not damage either.** 223 files carry some, the worst is
  29% of its packets, and every one renders perfectly — so it is a manufacturer's extension.

What *is* real rip damage is a tile addressed off the screen, and it is still not worth refusing a
song over. So all three are counted separately and **the only signal that condemns a file is that no
tiles were written**: no tiles means no words, whatever else is in it.

**A third assumption dies the same way, and it is ours rather than the format's.** A sample of twenty
pairs says a `.cdg` is never longer than its audio; over all 2,847 pairs, 34 overrun, by up to 142
seconds. Harmless — a tail of filler — and not a fact.

**The general lesson**, arrived at from the other direction to the MIDI work: a real corpus does not
tell you your parser is right, it tells you **which of your definitions of "wrong" were made up**.
Three of the four here came from a twenty-file sample, which is the size at which everything looks
consistent.

Once the audio was probed too:

| | |
|---|---|
| Paired | **2,847 of 2,849**, with two orphan `.cdg` and none in reverse |
| Audio that would not probe | **0** |
| Sample rate / channels | **44,100 Hz stereo, every single one** — which is why there is no resampling or downmix worth tuning for at packaging time |
| Carrying usable tags | 1,472, or 52% |
| Mean gap, graphics to audio | **0.1 s** |
| Graphics stopping over a minute early | **1** — the same file whose `.cdg` is not a whole number of packets |

**That mean gap was 3.2 s before the duration fix**, and two files were flagged as probable
mispairings that were nothing of the kind — their MP3 headers were lying. Counting the frames moved
the corpus-wide average by a factor of thirty and removed both false alarms.

## The audio half, and three things that look optional

`symphonia`, pure Rust, no build script and no libclang — which is the whole reason this crate needs
no cargo feature and therefore the reason MP3+G plays where video cannot. Default features off, so
only MP3 and the two ID3 versions are on.

The decode thread is `km-video`'s with the video removed: interleaved stereo at the **file's own
rate**, the same non-blocking seek protocol, and `finish()` on every exit path including errors —
without which the player waits for samples that never come and the song hangs where it stood.

Three things a first implementation would leave out, each of which the corpus made mandatory:

- **The encoder trims are applied by us.** Symphonia reports them per packet and does not remove them.
  Skipping that starts every song a few milliseconds late, and **since the CD+G clock is the audio
  position, that error lands straight on the words.**
- **The length is counted, not read.** One corpus file's Xing header overstates its length **by a
  factor of seven**, and the counted answer agrees with the `.cdg` beside it to a tenth of a second.
  ffmpeg gets this file wrong too, differently.
- **A decode error mid-song is skipped, not fatal.** One bad frame otherwise silences the rest of a
  recording.

## Judging it by ear, and one trap in doing so

The offline render goes through **a real player over a real feed** — the ring, the seek protocol and
the resampler included — so what lands in the WAV has been through the path the machine plays through.
Its levels sit within 0.017 dB RMS of ffmpeg's own decode of the same file.

**The trap is that the loop has to be paced.** On the machine the audio callback pulls at exactly real
time while the decoder is a hundred times faster, so the ring never runs dry. An offline loop that
pulls flat out wins that race instead, and the player answers an empty feed the only way it safely can
— **silence, with the position frozen**. That is correct for a live machine and produces a WAV full of
gaps here: unpaced, a six-minute song came out five seconds long with 466,697 starved frames.
**Anything else that drives a player faster than real time will meet this.**

**`measure_loudness` is the first thing that had to obey that warning, and it obeys it by not
playing.** It has no thread, no ring, no seek and no backpressure: it pulls packets and hands the
samples straight to a meter, which is a decode as fast as the disk allows and cannot starve because
there is nothing waiting on real time. Pacing it would have worked and would have made measuring a
six-minute song take six minutes.

**Nothing is lost by reading earlier than the player does.** The 0.017 dB RMS above is exactly the
claim that matters — the player's own output and the decoder's are the same signal — and `append` is
what produces the samples either way, encoder trims included, so what is measured is the recording
rather than the encoder's padding. It also means a **mono** file is measured after this crate's
full-amplitude duplication into both channels, which reads about 3 dB above ffmpeg's own reading of
the file and is right: that is what the machine will play, and the gain is applied to the same
signal. `km-video`'s swresample scales instead of duplicating, so the two paths differ there and each
measures its own. See
[`Video and MP3+G play at the MIDI reference level`](../decisions/audio.md#video-and-mp3g-play-at-the-midi-reference-level).

## Judging it by eye

The stills example writes PNGs from several points in a song — the fastest way to tell a working
renderer from a plausible one. **A palette read wrong, a tile bitmap reversed and a scroll offset
misapplied all produce output that looks like output** until somebody looks at it. The first real
render is what proved the XOR tile instruction: the wipe highlight was visibly part-way across a word.

**It is `stills` and not `preview`, and the rename was forced rather than chosen.** Cargo builds every
example in a workspace to one directory, so two targets of the same name collide on one output path —
which cargo reports on every test run and says may become a hard error. Whichever built last won the
file.

**Previews come out at 4:3 and not at the pixel size, because CD+G pixels are not square.** A preview
at the pixel size would be 12% too wide and would hide the very mistake it exists to catch.

## Packaging and curating

Both halves go into the package; **the manifest names the audio and the graphics are found by rule at
the same stem**, because a field that can only ever hold one value is a field a hand-edited manifest
can set wrong and every writer has to agree about. Both are checked, because half a pair is not a
degraded song.

**The renaming is the quiet win.** Real corpus names are a mess — mixed-case extensions inside one
folder, a stem with a trailing space — and all of it is normalized **once, at the packager**. The
messy-pairing code never ships to the machine.

**The check is two lines long, and the corpus is why.** There is no packaging profile, which departs
from video deliberately: that profile exists because the video decoder copies three planes and carries
no swscale, so a pixel format it cannot read genuinely blocks. Nothing here has an equivalent limit.
So a pair is refused for exactly two reasons — **the audio will not decode, or the graphics never draw
a tile** — and everything else is reported. The list is that short because every other candidate was
tried against 2,849 real files and none survived. **The lesson generalises: a quality gate wants a
corpus before it wants a threshold.**

**Title and artist take the stem first, which is the inverse of the video rule**, and the corpus is
the entire argument. A downloaded video's container tags are the best thing about it; a karaoke MP3's
are measurably not — present on about half, and when present often wrong: `Track  6` as a title, and
in one album several tracks carrying the *whole file name* in the artist field.

**A pair is one song, filed under its audio, and both halves still get a file row.** Neither half of
that is obvious: the song must not appear twice, **and the number of files in the folder must still
reconcile with the number in the tool**, or the scan page stops being something anybody trusts. Four
statuses rather than one, because they send a person somewhere different.

**The trap is the incremental scan.** A song is filed under its audio, so replacing only the `.cdg`
leaves that audio's size and modification time untouched — and the song silently keeps the graphics
facts of a file that is not there any more, with a re-scan doing nothing. The fix folds the graphics
file's size and mtime into the audio's before the skip test: no schema change, no second lookup, and
**it makes the pair rather than the file the unit that is compared.**
