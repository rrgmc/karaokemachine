# Song sources — video, MP3+G and UltraStar

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Video as a song source

**A song may be a video file.** The words are pixels in the picture, so nothing draws them and nothing
needs a font for them.

Video songs queue, search and mix freely with MIDI ones. A mixed queue costs no device churn, because
the machine opens the audio output stream once and only the *loaded thing* varies.

**A video song is not a video wallpaper** — wallpapers are still images.

## What a video song does not have

**No transpose, no tempo, no guide melody, no syllable wipe, and no lyric timeline.** All five measure
or manipulate things a MIDI file has and a video does not. There are no channels to mute, no key to
shift and no tick timeline to warp, and the words are in somebody else's picture.

Asking for one is **reported as unavailable rather than hidden**. The answer is a 409 with code
`unavailable`, the same answer the melody toggle gives on a file where detection abstained. A control
that silently does nothing is worse than one that says why.

`display.lyric_offset_ms` is inert for the same reason: nothing is being drawn, so there is nothing to
offset. The television's picture delay that it compensates for already applies to the video's own
frames.

## Where a video's title and artist come from

**The container's own tags first, the file's stem second, and a person's answer over both.**
`km_video::probe` reads `title` and `artist` from the container, and the scan records them as
*detected* values. `km-pack build` uses them in place of the stem.

**Detected, not authoritative**: they land in the same `det_*` columns a MIDI file's parsed title
lands in. So a typed correction wins, and curation is where a title is finally decided. The stem is the
fallback and stays. Most of a real corpus has no tags at all. A title with a `/` in it survives in a
tag where it cannot survive in a path.

**Language is not inferred for a video**, and a video song arrives with none. No container records
what language the singing is in, and a guess from a title would be a guess wearing a fact's clothes.
`Song language` infers one for a MIDI file from the encoding its lyrics are written in: a title is a
guess, and Shift-JIS bytes are evidence.

**An incremental scan settles a file by its size and modification time.** So a corpus scanned by a
build that did not read container tags keeps its blank artists until those rows are scanned again.

## What video costs the build

**An optional `video` cargo feature, off by default, and one crate that names ffmpeg.** A build
without it still catalogs, searches and queues video songs. It refuses to *play* them with a clear
message. That is the shape already used for "no SoundFont, so this is a test tone", said out loud
rather than failing silently.

This is the only C dependency outside SDL. It costs development libraries on three platforms, DLLs
beside the Windows executable, and a crate version coupled to the appliance's ffmpeg. The feature
localizes all of it, so a build without it needs neither ffmpeg nor libclang.

**Video builds and ships on Android too.** Whether Android *TV* is a target remains open and is a
separate question, since the machine builds `armeabi-v7a` for a television either way.

**Building ffmpeg for Android is where the cost is, not decoding it.**
`tools/port/machine/android/ffmpeg.sh` cross-builds the same pinned LGPL 7.1.5 for both ABIs, past
three traps:

- a `TMPDIR` whose backslashes ffmpeg's configure eats;
- a `cargo-ndk` that names clang without the `.exe`, so bindgen never finds `stddef.h`;
- an ffmpeg that installs `hwcontext_vulkan.h` whatever was configured. That makes `ffmpeg-sys-next`
  bind a Vulkan context through a stub whose size assertion is wrong on 32-bit. The build then fails
  on the one ABI a television loads.

**The APK carries 5 MB of ffmpeg rather than 25**, because the decoder set is narrowed — see
`What the Android build's ffmpeg can decode`.

## The packaging profile for a video

**H.264 in 8-bit 4:2:0, at most 1080p30, AAC, in MP4 — and checked before it is enforced.**
Normalizing at packaging time means the appliance's decoder only ever meets one thing. Checking first
means a file already in that shape is copied byte-for-byte rather than re-encoded. That matters,
because a `yt-dlp` download asked for AVC and AAC already *is* in the profile.

**One constraint is a requirement and the rest are preferences.** `km-video` copies three planes with
the chroma at half height and carries no color conversion. So anything that is not 8-bit planar 4:2:0
draws a wrong picture rather than failing. It is *refused*, and re-encoding is the remedy. VP9,
60 fps, 4K and a `.webm` container all play perfectly well. They are re-encoded for predictability and
disk, never refused.

The profile says nothing about the audio sample rate. The decoder resamples whatever it finds to
stereo, and the real sample file is 44.1 kHz. Requiring 48 would degrade a good file to fix a problem
that does not exist.

`--no-transcode` keeps an irregular file as it is and still refuses an unplayable one. Switching off a
re-encode is a statement about CPU. It is not a license to package a song the machine will skip while
somebody is holding the microphone.

## What the Android build's ffmpeg can decode

**H.264 and AAC, with MP3 and 16-bit PCM beside them, and nothing else — on Android alone.** Every
other platform keeps the rule that the build carries *every decoder ffmpeg implements itself*. That
lets `--play ./clip.mp4` audition an arbitrary download before anybody packages it.

**Android abandons it because the full set cannot be linked on Windows at all.** libavcodec's object
list alone is over 25 KB of paths, and Windows caps a command line at 32,767 characters. The link line
is truncated mid-argument, and clang reports `no such file or directory:
'libavcodec/bsf/mpeg4_unpack_bframe'`. That is `mpeg4_unpack_bframes.o` with three characters cut off,
and it reads like a missing source file. The size saving is a consequence rather than the motive. It
is **2.2 MB per ABI against an estimated 10–13 MB,
about 5 MB in the APK instead of 22–30 MB.**

What it costs is close to nothing, and only because `The packaging profile for a video` fixes what the
machine can ever *meet* in a package. An ffmpeg that is not this one re-encodes anything else at
packaging time. So **no song a package can hold becomes unplayable**. What is lost is auditioning an
arbitrary loose file through `debug.play_file` on the device, which refuses cleanly.

`h264_mediacodec`, `--enable-jni` and the two bitstream filters it needs are compiled in and never
selected. So the hardware decoder is a change in Rust rather than a second ffmpeg build on every
machine.

## Which H.264 encoder

**Whatever the local ffmpeg has: `libx264` if present, else `libopenh264`.** x264 is GPL, so the LGPL
ffmpeg this project develops against does not carry it. A profile that named `libx264` could not
re-encode anything on the development machine.

**Hardware encoders are deliberately excluded** even though an LGPL build lists three.
`ffmpeg -encoders` reports what was compiled in, and not what the machine's GPU and driver can
actually do. So choosing one automatically turns a missing graphics card into a failure in the middle
of a batch. Each also takes different rate-control options, so one profile could not aim at the same
quality through them.

Nothing new is linked or shipped either way: the transcoder shells out to the packager's own ffmpeg.

## Auditioning a video before packaging it

**The debug play-file path takes a video by absolute path.** Otherwise, judging a video song means
packaging it first. That is the wrong way round for a curation tool whose purpose is deciding what
deserves to become a song. The rule that a video song's media is found *relative to a package* is
about resolving a manifest's file name. It does not apply to a path handed to the machine directly,
and the route keeps the same `debug.play_file` permission and allowed-roots check.

A build without the `video` feature recognizes the file by extension and says it cannot play video. It
does not report a perfectly good MP4 as an unreadable MIDI file.

**A machine that is not the curator's own box is sent the video instead**, through
`debug/play-upload`. An absolute path is only a name over there, so the alternative was auditioning
nothing. The video is staged as a real file rather than streamed at the decoder, because the decoder
wants bytes it can seek. The codec limits are untouched, so the Android machine's cut-down ffmpeg
refuses the same files it refuses everywhere else.

## Searching a video's words

**Not done.** Extracting an embedded subtitle track would let the Lyrics page find a video by a line
somebody half-remembers. That is the question that page exists to answer. It is deferred because the
material does not carry one.

A `yt-dlp` download has no
subtitle stream unless `--embed-subs` was passed. YouTube's automatic captions are unpunctuated,
mistimed and frequently wrong. An index of those would make the lyric search less trustworthy for
every song in it, MIDI ones included. Video songs stay findable by title, artist and file name.

Revisit if a source of real subtitles appears.

## Video in a release build

**Every staging script builds video by default; `--no-video` is how you ask for the smaller one.**
This is about what the *release* commands produce, where `What video costs the build` is about the
cargo feature. A fresh clone still builds the whole workspace with neither ffmpeg nor libclang.

The two defaults answer different questions. The cargo one is "what must somebody install to compile
this at all". The release one is "what does a person receiving a folder get to play". Consider a staged
`km-package-builder` that indexes every MP4 as "a video, and this build has no `video` feature to read
it with". That is correct behavior for a build somebody asked for, and the wrong product to get from
the easy command.

**A missing ffmpeg stops the script before it builds anything**, naming both remedies, because a
release must not quietly degrade into the lesser build. The exception
is a run selecting only tools that have no such feature.

**The marker goes on the declined build** — `-no-video` folders, a `no-video/` subfolder for the
`.deb`. The plain name should name what the plain command produces, and the two still cannot overwrite
each other.

Four carriers, each paying a different price:

- **Windows** stages four DLLs beside the exe and proves the folder starts with ffmpeg off `PATH`.
- **Linux** names four `Depends` that Debian supplies and stages nothing. But a second Linux carrier
  pays a fourth price: the libraries Debian merely *names* turn out to be libraries a folder may not
  carry.
- **macOS** costs the most work. The linker records each dylib by the absolute path it was linked at,
  so staging copies them in and rewrites every load command. See `Video in a macOS release`.
- **Android** stages four `.so` files in `jniLibs/<abi>/`, exactly as SDL's two already are. It adds
  one Android-only trap: the Gradle plugin packages **only** `*.so` out of that directory. So a
  versioned soname like `libavcodec.so.61` is dropped silently, and the app dies at launch with
  nothing wrong in the build log. ffmpeg's own `android)` case emits unversioned names, so nothing
  patches it. But `ffmpeg.sh` asserts it, because a later version bump could take it away. The same
  rule is why the LGPL text ships in the **assets** tree rather than beside the libraries it covers.

**`--no-video` is deprecated as a thing to *produce*, not as a thing that compiles.** The flag stays
and the build behind it stays supported. It is what a person who does not
want a C dependency asks for. It is also what CI exercises when it checks that a video-less build
still catalogs and refuses politely.

What is deprecated is generating one *alongside* the ordinary
build. A release pass should not produce a `-no-video` folder or a video-less APK unless somebody
typed the flag.

## Video is the default build

**The unsuffixed command name is the video one, and declining video is spelled `-no-video`.**
`km-build`, `km-test`, `km-lint` and `km` are the video ones, and `task build`/`test`/`lint` follow
them. The `-no-video` / `:no-video` twins are what a machine with no C toolchain runs.

**The short name matters because of the build cache.** Alternating between the two feature sets
invalidates every crate downstream of `km-video`: `karaokemachine`, `km-pack` and
`km-package-builder`. So a scheme whose short name is the one nobody wants produces exactly the
flip-flopping that is expensive on this workspace.

**Two things keep their plain spelling.** The per-tool aliases `km-pack` and `km-pkgbuild` do.
`km-pack book` and `km-pack check` need no `video` feature and no ffmpeg at all, because a book is
manifest metadata and never opens a song's bytes. A MIDI or MP3+G corpus also scans without one.

And **CI builds video on Linux only**, in `debian:13-slim` beside the appliance's ffmpeg. Its Windows and
macOS jobs run the `-no-video` twins. The code behind the feature has no platform-specific paths, and
finding ffmpeg there is a download `tools/setup/fetch-ffmpeg.sh` owns. `check.sh` covers the video
build locally.

**What it costs is one prerequisite**: `task check` wants `task ffmpeg` to have been run, which is one
command once per machine.

## Video in a macOS release

**A macOS video build carries ffmpeg inside it, plays video on a Mac that has no ffmpeg installed, and
is LGPL like the Windows one.**

*Portability*: a Mach-O names each library by the absolute path it was linked at. So staging copies
the closure in, rewrites every load command to `@rpath` and **re-signs each file**.
`install_name_tool` invalidates the ad-hoc signature, and dyld then refuses to load the file.
`dist_stage_ffmpeg_macos` does this once, and it reaches two bundles (`Contents/Frameworks`) and the
`km-pack` and `km-package-builder` folders (`lib/`). `km-package-builder` is staged both ways, so the
same build's libraries are laid in twice under two rpaths. Each time they come from the untouched
executable, not from the copy the other has already rewritten.

*License*: bundling means redistributing, and Homebrew's ffmpeg is `--enable-gpl --enable-version3`
with x264 and x265. So a bundle built against it is GPL. It also carries 25 MB of encoders this
application cannot call, since `km-video` only decodes and `km-pack` re-encodes by shelling out. So
`tools/setup/fetch-ffmpeg.sh` builds a pinned LGPL ffmpeg from source on macOS. Nobody publishes a
prebuilt shared LGPL macOS ffmpeg with headers, so source is the only route, and it costs about a
minute once per machine. The result is 4 dylibs and 15 MB instead of 13 and 33.

**7.1.5 is pinned deliberately** — exactly Debian trixie's version, so the development machine is the
lower bound rather than ahead of the appliance. Homebrew remains available through `--homebrew`, and
an `FFMPEG_DIR` in the environment wins. A staged note labels a build made either way, and it works
out which case it is in from the libraries actually copied.

## MP3+G as a song source

**A song may be an MP3 paired with a `.cdg` of the same stem.** A bare MP3 is not a song, because it
has no words.

**CD+G the *disc* format is not supported.** There is not one `.bin`/`.cue` or raw subcode rip in the
corpus, because the file pair is what the world actually trades.

**It needs no C dependency and no cargo feature.** The renderer is ours, in Rust, and the MP3 decoder
is `symphonia`. So an MP3+G song plays in a `--no-video` build and on Android, and
`km-app/src/cdg.rs` has no `AVAILABLE` const and no second `mod imp`.

See `No audio-file pitch shifting` for why the control refusals below are permanent.

## What an MP3+G song does not have

**The same five a video lacks, answered the same way.** There is no transpose, no tempo, no guide
melody, no syllable wipe and no lyric timeline. Each is a 409 with code `unavailable` naming which.

One difference looks like it should change the answer and does not. A video's words are pixels in
*somebody else's* picture. This application draws an MP3+G song's words itself, from data in the
file. That does not give it a timeline. CD+G words are one-bit 6x12 tiles with no character data
behind them, so there is not even a character to have an encoding. There is nothing to highlight and
nothing to search.

Suitability is **full marks, by what the file is**, for the same reason it is for a video. A
commercial karaoke disc was made to be sung to, so there is nothing for the rubric's four
measurements to resolve. The one thing in doubt is how long it is sung for. The pair is answered on
its own length, because CD+G words carry no timing to read. See
[`Suitability, for a song that was made to be sung to`](songs.md#suitability-for-a-song-that-was-made-to-be-sung-to).

## How an MP3+G pair is stored

**Both files go inside the package as stored entries, named `media/<number>.mp3` and
`media/<number>.cdg`. The manifest names the audio, and a rule finds the graphics from its name.**
No second manifest field, for the reason `Where a song's media lives` gives. A field that can only
ever hold one value is a field a hand-edited manifest can set wrong and every writer has to agree
about.

The pair is *renamed* to the number at packaging time. So the corpus's mixed-case extensions and its
one trailing-space stem are normalized **once, at the packager**, and the messy-pairing code never
ships to the machine.

`missing_entries()` checks **both** halves. Half a pair is not a degraded song; it is an MP3 with no
words.

The machine seeks into the audio and reads the graphics whole: 2.6 MB for six minutes. `km-cdg`
replays from packet zero on every seek, so it holds all of it anyway. **Being read whole is why the
graphics are the one media entry that is deflated**. Nothing seeks into a `.cdg`, so nothing needs a
byte range into it, and CD+G measures at 14.7% of its size.

## Rendering CD+G, not converting it

**Ours, in Rust, ~500 lines, and the surface is CLUT *indices* rather than resolved colors.** That is
not an optimization. A `LOAD_CLUT` recolors everything already drawn, which is how many discs fade
words in and out. Stored resolved pixels would make the instruction do nothing.

**Anything unrecognized is counted and skipped, never an error**, and the corpus is emphatic. 2,849
files and 208 million packets replayed with **nothing unreadable, nothing that drew no words, and no
panic**. A command byte that is not 9 is another subcode application, not damage. An unimplemented
CD+G instruction is a manufacturer's extension, not damage. **The rule is load-bearing and must
never be "improved" into a hard failure.**

**The whole `.cdg` is held in memory** — 2.6 MB for six minutes. So a seek is a replay from packet 0
with no I/O, no keyframes and no snapshots. The whole corpus replays in **4.5 seconds warm, 46 million
packets a second**. That puts a six-minute song's full rebuild at about **two milliseconds**.

**The display pulls the graphics; no thread pushes them.** Nothing is produced unless somebody asks,
so a headless run simply never advances the screen. That would deadlock for video, so `km-video` has a
queue, a pool and a lookahead, and this has none of the three.

Every byte is masked with `0x3F`, because real files retain the P and Q subchannel bits in the top two.
`SCROLL_PRESET` and `SCROLL_COPY` never appeared in an 800,000-packet sample and are implemented
anyway: "never seen in 48 files" is not "absent from 2,849".

## A CD+G pixel is not square

**The picture is presented at 4:3, not at the 3:2 its 288x192 would imply.** Invisible until somebody
notices the words look fat. CD+G was drawn for a television, so presenting its pixel size would
stretch every word about 12% too wide.

`Background`'s letterbox field is a **shape**, a width-to-height ratio, rather than a *size*. A video
passes its pixel size, and an MP3+G song passes 4:3. It works for video only because video pixels
happen to be square, and an anamorphic video would want the same treatment.

The picture is scaled **nearest-neighbor**, deliberately. A 288x192 surface blown up to a television
should look crisp and blocky the way a real machine does. The default bilinear filter would soften it.

## Where an MP3+G song's title and artist come from

**The file's own tags, *filtered* before they are believed, then the stem, then a person over both.**
The precedence is the same three steps as a video's, and what is added is a filter. The tags here
cannot be taken at face value. Measured over this corpus, ID3v2 is present on roughly half the files.
When present, it is frequently wrong: artist and title swapped, and titles that are literally
`Track  6`.

So a tag that is blank **or** that matches `track <n>` is discarded as a placeholder. The stem,
reliably `Artist - Title` here, is what remains. The residual swapped-fields case cannot be detected
reliably and is left to curation, where
[`Taking the artist out of the title it was filed inside`](curation.md#taking-the-artist-out-of-the-title-it-was-filed-inside)
is the button that answers it over ticked rows.

## Nothing transcodes an MP3+G song

**There is no packaging profile, no encoder table and no `--no-transcode` flag; packaging copies both
files byte for byte and *checks* them instead.**

The video profile exists because `km-video` copies three planes and carries no swscale, so a pixel
format it cannot read is genuinely blocking. Nothing here has an analogous limit. `symphonia` decodes
any Layer III at any rate and `TrackPlayer` already resamples. So there is nothing a re-encode could
fix, and lossy-to-lossy would be a pure quality loss.

A check replaces the profile. It is free, because the full graphics replay it needs is about two
milliseconds. **It refuses a song for exactly two reasons**: the MP3 will not decode, or the `.cdg`
draws no tile at all. A CD+G with no words in it is an MP3.

**The list is short because the corpus made it short.** Every intuitive quality signal was tried
against 2,849 real files, and every one was wrong:

- A packet whose command byte is not 9 is another subcode application. One file is 74% of them and
  renders three clean lines.
- A CD+G instruction nobody implements is a manufacturer's extension. 223 files carry some, and the
  worst is 29% of its packets. All render.
- A tile addressed off the screen *is* rip damage, but it is a blemish nobody sees. 129 files carry
  between one and fourteen.

**A check built on any of those would have refused songs that work.**

What is reported:

- those three counts;
- a size that is not a multiple of 24. 2,848 of 2,849 are, so the count is rounded down and said out
  loud;
- graphics that stop more than a minute before the audio. This check catches a `.cdg` paired with the
  wrong song, and 9 to 37 seconds is ordinary.

Rate, channels and bitrate are recorded and never enforced.

## An MP3+G song's length is its audio's, and is counted rather than read

**Duration comes from the MP3, never from the graphics — and never from the MP3's own header either.**

The graphics are not the song. The CD+G stream agrees with the audio to within 0.2 s on most files.
On many, it stops seconds to minutes early, because the words end before the outro does. Taking that
as the length would cut the progress bar short on exactly the files where somebody is still holding a
note.

**An MP3 states its own length in a Xing header, and one in the corpus states it wrongly by a factor
of seven.** The header claims 1,646 s, and ffmpeg's own estimate says 1,032 s. The audio actually in
the file is **242 s**, which is, to a tenth of a second, exactly as long as the `.cdg` beside it.
That number goes into the manifest, drives the progress bar and decides when the machine moves on.
So a song claiming twenty-seven minutes leaves the room staring at a finished song for twenty-three of
them.

So the file is walked and its real frame durations summed. The walk parses frame headers, decodes
nothing and costs one sequential read, and it is the only answer that cannot be a lie. The graphics
freeze on their last picture and the song ends with the sound. The measured gap is its own field, and
the mispairing check reads it.

## A song that stopped says so without being asked

**A starved decoder is reported at `warn` unconditionally, not behind `--frame-stats`.** It is not a
diagnostic, so the frame meter's rule that a diagnostic is asked for by name does not reach it.
When the audio feed runs dry, the position freezes and the song audibly stops. So the owner has
already experienced the event. The only question is whether the machine can confirm it afterwards.

Zero costs nothing: a healthy song reports zero and prints nothing. That makes an always-on fault
report affordable where an always-on measurement would not be. The per-second numbers stay behind
`--frame-stats`, because *those* are a diagnostic.

**Only one counter may raise the alarm.** `skipped` — pictures
that came due while they waited — looks like the right gate and is not. The position a picture is
judged against advances one audio callback at a time, and the appliance's period is 117 ms. So three
and a half frames of a 30 fps video fall due at every step, and all but the newest are counted.

It runs at **half a picture a second on a song with nothing wrong with it**. Across four measured runs
it sat between 0.44 and 0.65 a second, while the audible fault went from 871 ms to zero. It does not
detect the fault and does not track its severity.

So the alarm belongs to `starved_ms`, which measures the thing the owner experienced. The picture-side
line is gated on `dropped`, the queue backing up, which on a machine with a display should never
happen. `skipped` is kept as a rate to compare against its own baseline. It is also proof that a
display was attached at all: a headless run takes no frames, so it cannot have skipped one.

**A counter that is never zero cannot gate a warning.** Whether it is ever zero is a measurement,
not something to reason out from what it means. The numbers are in
[`docs/architecture/video.md`](../architecture/video.md#what-decoding-costs-and-what-says-when-it-stopped).

## Video decodes on every core, not on one

**`km-video` asks ffmpeg for frame threading; without that it gets one thread.** ffmpeg's
`thread_count` defaults to 1 for a library caller. The command-line tool turns threading on for
itself. So decoding through the API is quietly several times slower than the same file through
`ffmpeg` on the same machine.

**CPU is not the scarce resource — a single-threaded bottleneck does not care how much of the other
three cores you hand it.** A 1080p30 song cost 80% of one Cortex-A55 while the other three sat idle.
So any passage harder than average emptied the 250 ms ring and froze picture and sound together.

Capping the display gave the decoder 20% of a core back and changed nothing. The display was drawing
at twice the screen's rate and wasting about half a core. Frame threading spawns a worker per core
and drops the per-core peak five-fold. It takes starvation from 871 ms in a song to none at all.

`count: 0` is ffmpeg's own "decide from the machine", which is right for hardware this has never seen.
`Frame` rather than `Slice`, because slice threading needs the encoder to have emitted slices, and
most files carry one per picture. Frame threading works on anything, at the cost of a few frames of
latency the 250 ms lookahead already covers.

## UltraStar as a song source

**A song may be an UltraStar `.txt` together with the audio file its header names.** The file gives
each syllable a start and a length, so its words are text with timing. An audio file with one beside
it has words, where a bare audio file has none.

**The file is read for its words and their timing, and for nothing else.** Every note also carries a
pitch and a type that singing games score a singer against. Both are discarded, because scoring
singers is a non-goal and the pitch of a sung melody is not a backing track anyone could transpose.

See `No audio-file pitch shifting` for why the control refusals below are permanent.

## What an UltraStar song has, and what it does not

**It has a syllable wipe and a lyric timeline.** The machine draws them over the wallpaper as it draws
a MIDI song's words. They are searchable, and they give the songbook its first line. That is what
separates it from MP3+G, whose words are pixels.

**It has no transpose, no tempo and no guide melody.** Each is a 409 with code `unavailable` naming
which, for the reasons an MP3+G song has none.

**Its suitability is a flat 10**, under `Suitability, for a song that was made to be sung to`: a
person timed its words to this recording. The recording is usually the original with its vocals,
and community timing is uneven. A file that sings badly is curation's to hide, not a number's to
guess. **How long it is sung for is the exception.** It is the one media kind measured on its words
rather than on its length. The timing a person wrote says exactly where the singing starts and
stops.

`display.lyric_offset_ms` applies unchanged, because it moves the words and not the sound.

## Which UltraStar files are songs

**A file is a song when it has a `#TITLE`, one voice, and audio the machine can decode.** Measured
against a real collection in
[`docs/research/ultrastar.md`](../research/ultrastar.md#2-dialects-and-failure-shapes), these are
the rules that keep what exists and refuse what cannot play:

- **Unversioned files and version 1 files are read.** Almost every file in circulation has no
  version, and an unknown major version is refused, as the specification says.
- **Relative mode is converted to absolute beats**, not refused. It is almost a fifth of a real
  collection, and the conversion is exact.
- **A decimal comma is read in any number**, `#VERSION` included. It is the normal case in `#BPM`.
- **The encoding is `#ENCODING` in an unversioned file, and detected otherwise, with `#LANGUAGE` as
  the hint.** No file in the measured collection declares one, and a quarter are not UTF-8. A
  version 1 file is UTF-8 by rule, and the detector is not told so. Editors write a version on CP1252
  files, and detection reads valid UTF-8 as UTF-8 anyway. A short Portuguese file reads as Polish
  without its language to steer it.
- **The reader finds the audio through `#AUDIO`, then `#MP3`, and never through the stem.** One file
  in six names audio whose stem differs from its own.
- **The audio is MP3.** Another audio format is a new decoder feature and a change to this entry.
- **Duets are refused.** A lyric display has one stream of words, and two voices merged by time
  interleave two phrasings.
- **A file whose only media is a video is refused**, for the reasons in `What a video song does not
  have`. A `#VIDEO` beside audio is ignored, and the audio is the song. The video is not a video
  song of its own: a singing game's music video has no words in its picture.
- **A `.txt` with no `#TITLE` is not a song**, and is skipped rather than reported: a song folder
  holds readme files too.

## Where an UltraStar song's title, artist and language come from

**`#TITLE` and `#ARTIST` first, the stem second, and a person's answer over both**, as detected
values in the same columns every other kind uses. `#LANGUAGE` names a language in words, such as
`English`. It becomes the song's ISO 639-1 code when the table knows the name. Otherwise the song
has none until curation gives it one.

## The machine never reads an UltraStar file

**The package builder turns the file into a lyric timeline, and the package stores the timeline and
the audio.** The dialects above are the kind of messy input the builder already handles for MIDI
encodings and MP3+G pairing. Keeping them there means none of that code ships to the machine.
The audio is stored as `media/<number>.mp3`, as an MP3+G song's is, and is copied byte for byte.

**A preview follows the same rule.** The package builder's Play button sends the machine the MP3 and
the timeline it read, over either debug route, and never the `.txt`.
