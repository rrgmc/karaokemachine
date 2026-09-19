# MP3+G songs

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

An MP3+G song is two files with the same stem: an `.mp3` with the backing track and a `.cdg` with the
words as graphics.

**This crate has no cargo feature, and the absence is the design.** `km-video` has one because ffmpeg
is a C dependency a workspace build must not require. Nothing here is optional in that sense. The
renderer is ours and the MP3 decoder is pure Rust. **No build can catalog an MP3+G song and refuse
to play it.**

That is what makes MP3+G work in a `--no-video` build and on Android. A
future reader will otherwise take the missing feature gate for an oversight, so three places carry a
comment about it.

**Both halves come out of the package, and the asymmetry between them is this crate's own.** The
player seeks into the MP3 through a window and reads the `.cdg` whole. The reason is that every seek
replays from packet zero, so the player holds the 2.6 MB anyway.

## The format, and three things a document will not tell you

A stream is 24-byte packets at exactly 300 a second: 75 CD sectors times four subcode packs. So a
stream's length is its size over 24 over 300, and **nothing in the file states it**.

- **Every byte is masked with `0x3F`.** A `.cdg` is a copy of the R–W subchannel, and real rips keep
  the P and Q bits in the top two. Every file examined here had them set, **so masking is the format
  and not a defensive measure**. A test feeds a stream twice, once with the bits forced on, and
  demands identical output.
- **The surface holds palette indices, not colors.** Loading a palette recolors everything already
  drawn, and many discs fade words in and out that way without redrawing a tile. Resolving pixels at
  draw time would make the instruction do nothing. **It would present as "the fades are missing"
  rather than as a bug.**
- **A four-bit channel expands by multiplying by 17, not by shifting left four.** 15 has to become
  255; `15 << 4` is 240. **A palette whose white is 94% white looks like a tired display, and a
  person blames the display.**

## Pulled by the display, not pushed by a thread

`km-video` needs a decoder thread, a bounded frame queue, a pool and a lookahead. A full picture queue
must never stall the demuxer, because that demuxer also carries the audio. A headless run takes no
pictures at all.

**None of that applies here, because nothing pushes.** Whoever wants a picture advances the screen,
so a headless run simply never advances it. There is no queue to fill, no frame to drop and no
backpressure to get wrong.

That leaves the awkward-looking part: **a CD+G surface is cumulative**. The player cannot skip
packets, and a backward seek has nowhere to start from but the beginning. **The measurement is what
makes that a non-problem.** The whole corpus, 2,849 files and 208 million packets, replays in **4.5
seconds warm, about 46 million packets a second**. That puts a six-minute song's complete rebuild at
roughly **two milliseconds**.

So there are no keyframes, no snapshots and no "which snapshot was that" bugs. The player reads the whole file into memory at load, and a rewind replays from zero.

**The rewind tolerance is the one trap.** The display smooths the position between audio callbacks,
so it can overshoot and step back. A rewind check with no tolerance would rebuild on that wobble. That
is a hitch on every drawn frame, **appearing as a flicker nobody could place.**

## What the corpus said, and the two assumptions it destroyed

Over 2,849 files and 208 million packets: **nothing unreadable, nothing that drew no words, and no
panic.**

The value of that run was not the pass. **Both intuitive definitions of "corrupt" turned out to be
wrong**, and each would have shipped a packaging check that refused songs that work:

- **A command byte that is not 9 is not damage: it is another subcode application.** One file looks
  like garbage in a hex dump and is 74% such packs. It renders three clean lines of karaoke.
- **A CD+G instruction nobody implements is not damage either.** 223 files carry some, and the worst
  is 29% of its packets. Every one renders perfectly, so it is a manufacturer's extension.

What *is* real rip damage is a tile addressed off the screen, and it is still not worth refusing a
song over. So the scan counts all three separately. **The only signal that condemns a file is that no
tiles were written**: no tiles means no words, whatever else is in it.

**A third assumption dies the same way, and it is ours rather than the format's.** A sample of twenty
pairs says a `.cdg` is never longer than its audio. Over all 2,847 pairs, 34 overrun, by up to 142
seconds. The overrun is harmless, a tail of filler, and the assumption is not a fact.

**The general lesson** comes from the other direction to the MIDI work. A real corpus does not tell
you your parser is right; it tells you **which of your definitions of "wrong" were made up**. Three of
the four here came from a twenty-file sample, which is the size at which everything looks consistent.

Once the audio was probed too:

| | |
|---|---|
| Paired | **2,847 of 2,849**, with two orphan `.cdg` and none in reverse |
| Audio that would not probe | **0** |
| Sample rate / channels | **44,100 Hz stereo, every single one** — which is why there is no resampling or downmix worth tuning for at packaging time |
| Carrying usable tags | 1,472, or 52% |
| Mean gap, graphics to audio | **0.1 s** |
| Graphics stopping over a minute early | **1** — the same file whose `.cdg` is not a whole number of packets |

**Without the duration fix, that mean gap is 3.2 s**, and two files show as probable mispairings that
are nothing of the kind. Their MP3 headers lie about the length. Counting the frames moves the
corpus-wide average by a factor of thirty and removes both false alarms.

## The audio half, and three things that look optional

The decoder is `symphonia`: pure Rust, no build script and no libclang. That is the whole reason this
crate needs no cargo feature, and therefore the reason MP3+G plays where video cannot. Its default
features are off, so only MP3 and the two ID3 versions are on.

The decode thread is `km-video`'s with the video removed. It produces interleaved stereo at the
**file's own rate**, and it uses the same non-blocking seek protocol. It calls `finish()` on every
exit path, errors included. Without that call, the player waits for samples that never come, and the
song hangs where it stood.

A first implementation would leave out three things, and the corpus made each of them mandatory:

- **This crate applies the encoder trims.** Symphonia reports them per packet and does not remove
  them. Skipping that starts every song a few milliseconds late. **The CD+G clock is the audio
  position, so that error lands straight on the words.**
- **The length is counted, not read.** One corpus file's Xing header overstates its length **by a
  factor of seven**. The counted answer agrees with the `.cdg` beside it to a tenth of a second.
  ffmpeg gets this file wrong too, differently.
- **A decode error mid-song is skipped, not fatal.** One bad frame otherwise silences the rest of a
  recording.

## Judging it by ear, and one trap in doing so

The offline render goes through **a real player over a real feed**, the ring, the seek protocol and
the resampler included. So what lands in the WAV has been through the path the machine plays through.
Its levels sit within 0.017 dB RMS of ffmpeg's own decode of the same file.

**The trap is that the loop has to be paced.** On the machine, the audio callback pulls at exactly
real time while the decoder is a hundred times faster, so the ring never runs dry. An offline loop
that pulls flat out wins that race instead. The player answers an empty feed the only way it safely
can: **silence, with the position frozen**. That is correct for a live machine, and it produces a WAV
full of gaps here. Unpaced, a six-minute song came out five seconds long with 466,697 starved frames.

**Anything else that drives a player faster than real time will meet this.**

**`measure_loudness` is the first thing that had to obey that warning, and it obeys it by not
playing.** It has no thread, no ring, no seek and no backpressure. It pulls packets and hands the
samples straight to a meter. That is a decode as fast as the disk allows, and it cannot starve,
because nothing waits on real time. Pacing it would work, and it would make measuring a six-minute
song take six minutes.

**Reading earlier than the player does loses nothing.** The 0.017 dB RMS above is exactly the claim
that matters: the player's own output and the decoder's are the same signal. `append` produces the
samples either way, encoder trims included. So the meter measures the recording rather than the
encoder's padding.

The meter measures a **mono** file after this crate's full-amplitude duplication into both channels.
That reads about 3 dB above ffmpeg's own reading of the file, and it is right. It is what the machine
will play, and the gain applies to the same signal. `km-video`'s swresample scales instead of
duplicating, so the two paths differ there, and each measures its own. See
[`Video and MP3+G play at the MIDI reference level`](../decisions/audio.md#video-and-mp3g-play-at-the-midi-reference-level).

## Judging it by eye

The stills example writes PNGs from several points in a song. It is the fastest way to tell a working
renderer from a plausible one. **A palette read wrong, a tile bitmap reversed and a scroll offset
misapplied all produce output that looks like output** until somebody looks at it. The first real
render is what proved the XOR tile instruction: the wipe highlight was visibly part-way across a word.

**It is `stills` and not `preview`, and cargo forces that name.** Cargo builds every example in a
workspace to one directory, so two targets of the same name collide on one output path. Cargo reports
that on every test run, and says it may become a hard error. Whichever target builds last wins the
file.

**Previews come out at 4:3 and not at the pixel size, because CD+G pixels are not square.** A preview
at the pixel size would be 12% too wide and would hide the very mistake it exists to catch.

## Packaging and curating

Both halves go into the package. **The manifest names the audio, and the loader finds the graphics by
rule at the same stem.** A hand-edited manifest can set a one-value field wrong, and every writer has
to agree about it. The packager checks both halves, because half
a pair is not a degraded song.

**The renaming is the quiet win.** Real corpus names are a mess: mixed-case extensions inside one
folder, and a stem with a trailing space. The packager normalizes all of it **once**. The
messy-pairing code never ships to the machine.

**The check is two lines long, and the corpus is why.** There is no packaging profile, and that
departs from video deliberately. The video profile exists because the video decoder copies three
planes and carries no swscale, so a pixel format it cannot read genuinely blocks. Nothing here has an
equivalent limit.

So the packager refuses a pair for exactly two reasons: **the audio will not
decode, or the graphics never draw a tile**. It reports everything else. The list is that short
because every other candidate met 2,849 real files and none survived.

**The lesson generalises: a quality gate wants a corpus before it wants a threshold.**

**Title and artist take the stem first, which is the inverse of the video rule**, and the corpus is
the entire argument. A downloaded video's container tags are the best thing about it. A karaoke MP3's
tags are measurably not: about half the files have them, and they are often wrong. One file has
`Track  6` as a title, and in one album several tracks carry the *whole file name* in the artist
field.

**A pair is one song, filed under its audio, and both halves still get a file row.** Neither half of
that is obvious. The song must not appear twice. **The number of files in the folder must still
reconcile with the number in the tool**, or nobody trusts the scan page. There are four statuses
rather than one, because they send a person somewhere different.

**The trap is the incremental scan.** A song is filed under its audio, so replacing only the `.cdg`
leaves that audio's size and modification time untouched. The song then silently keeps the graphics
facts of a file that is gone, and a re-scan does nothing. The fix folds the graphics file's size and
mtime into the audio's before the skip test. It needs no schema change and no second lookup, and **it
makes the pair rather than the file the unit that is compared.**
