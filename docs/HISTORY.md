# Eighteen years, and then fourteen days

> The origin story, and the one document here that is chronological on purpose. The style rule in
> [`docs/decisions/repository.md`](decisions/repository.md) excludes chronology and what something
> used to be; this file is exempt because the arc *is* the content, as it is for
> [`docs/learning-rust.md`](learning-rust.md). It binds nothing — product decisions live in
> [`docs/decisions/`](decisions/) and this document is not one.

This machine was built in fourteen days. It was attempted five times over the eighteen years before
that, and every attempt failed.

That is not an interesting sentence on its own. What makes it worth writing down is *where* the five
failed, because it is not where anybody would guess. Every one of them succeeded at the part that
sounds hard. Every one of them could open a karaoke MIDI file, find the lyric events buried in it,
and put a timestamp on each syllable. Four of them played audio while doing it. Not one of them ever
drew a word on a screen in time with the music.

---

## The five

| Year | Project | Stack | Lasted | How far it got |
|---|---|---|---|---|
| 2008 | KaraokeMachine | SDL 1.2, TSE3, libtimidity, boost | Feb–Sep, 52 commits | Packages, playlist, keypad, threaded playback, **a working screen**, a handheld port |
| 2014 | karmac | FluidSynth, declared but never called | 8 days, 2 commits | A package format that writes and cannot read |
| 2016 | tse3 | TSE3, freshly imported | **2 days**, 11 commits | Two karaoke patches, rewritten from scratch onto nothing |
| 2018 | fluidsynthpp, libkarmac | A privately patched FluidSynth | 8 days, 36 commits | Timed lyric lines, printed to a console |
| 2020 | karmac2 | TinySoundFont, miniaudio, midifile | 3 days, 8 commits | A song playing, lyrics printed to stdout in sync |

---

## 2008 — the one that had a screen

The first attempt is also the furthest any of them got, which is its own kind of joke. It ran from
February to September 2008 — seven months, against one-to-three-day bursts for everything that
followed.

It was C++98 with Code::Blocks project files, three of them: Win32, unix, and a **GP2X**, the Linux
handheld with two ARM cores that briefly looked like the future of portable open-source gaming. The
handheld build was not a token port. It swapped the display to 320×240 at 12pt where the desktop ran
800×600 at 22pt, dropped the audio to 22050 Hz with 64-sample buffers, scaled the backgrounds down to
three-tenths, and read a joystick instead of a keyboard. Somebody meant to carry this around.

The shape of it is recognisable from where you are standing now. Songs lived in `.kms` package files
and background images in `.kmi` ones, both `#pragma pack(1)` structs with fixed-width character
fields and byte offsets into a file handle held open for the life of the process. Typing digits on
the keypad built a code of the form `package*song`, which was split on the asterisk and appended to a
playlist. A song was never played from a path — the bytes were copied out of the package into memory
first. There was a separate wxWidgets application whose only job was authoring those packages. Two
screenshots were committed, `v0.1` and `v0.2`.

It also had the bugs of something nobody ever finished. Song-end detection is *present and commented
out*, so a finished song leaves the playlist stalled until a human presses a key. The endianness
helpers that every multi-byte read goes through have their swap bodies commented out and are identity
functions, so the header's claim that packages are big-endian is false and a package built on the
handheld will not open on a desktop. There is no `try`/`catch` anywhere, `main()` included, in code
that throws freely. The MIDI device is a static class member constructed before `main()` runs, which
is why only one song can ever be playing.

And then it stops. The last three commits are "add DL library", "MSVC project and fixes", "MSVC
fixes" — a week of build plumbing, and then silence for six years.

## 2014 — the format, from nothing, again

The second attempt is two commits, eight days apart, and it is the most instructive of the five
because of what it chose to do first.

It did not start with playback. It started by designing the package format again, from nothing, with
no reference to the one that already existed and worked. New magic bytes, `KMPK`; a new header
carrying a name, an author, a producer code, a collection code and a song count; a new per-song record
under `KMSG` with a title, an artist and a length. The writer is complete — header, then every song
header, then every blob. The reader checks the magic bytes and stops. It cannot read back a single
file it wrote.

The build script requires FluidSynth. No source file in the project includes it, mentions it, or
calls it.

What *did* get written that month was the FluidSynth patch, and that is where the effort actually
went — which is the pattern the next twelve years repeat.

## The fork you have to write before you can start

Upstream FluidSynth threw karaoke lyrics away. The switch case for a lyric meta-event was an empty
`break;` — parsed, recognised, discarded. Every karaoke feature in every attempt from 2014 onward
begins by fixing that, and not one of those fixes was ever upstreamed.

The patch starts at 121 lines across six files in January 2014: keep the text, allocate an event for
it, publish a callback that fires once the file has finished loading so an application can walk the
tracks. Then it grows, a feature at a time, over two years. Static linking on Windows. A hundred and
eighty-one lines of internal MIDI constants moved into the public header, because a client program
could not otherwise name the events it was being handed. A getter and setter for an event's delta
time, without which you cannot compute when a syllable lands. Tempo changes reaching the playback
callback. An event type that carries its player, its track and its absolute tick position. A periodic
tick callback.

In November 2018 the whole series is replayed onto a modern FluidSynth. That branch's final state is
**fifteen commits ahead of upstream and one thousand seven hundred and twenty-two behind**.

It happens to the other stacks too. The single-file synth picked up in 2020 had a MIDI loader that
kept no text at all, so April 2020 opens with a patch adding a text pointer to its message struct and
two cases to its parser. The MIDI reader chosen the following month needed a pitch-bend accessor,
added on a branch of its own — and never wired back into the project that wanted it, which went on
reassembling the fourteen-bit value by hand.

Three synth stacks. Three sets of private patches. All of them for the same two things: let lyrics
survive parsing, and do not transpose the drum channel when a singer changes key. **Every stack was
abandoned for the reason the next one was chosen, and the next one turned out to need the same
surgery.**

## 2016 — two days on the library, none on the machine

December 2016 is the purest expression of that. Over two days, the 2005-vintage sequencer from 2008
is imported fresh, converted to CMake, made to compile on Linux, and then given back the two patches
it had been given in 2008 — the drum-channel-aware transpose, and a transport callback carrying the
whole MIDI event instead of just its command byte.

Both had already been written, eight years earlier, in a tree that was still on disk. Both were
written again from scratch.

Nothing was built on top of them. The last commits are compile fixes, and that is the entire attempt.

## 2018 — eight days in November

The fourth attempt is the one that looks most like it is going to work, and it escalates over eight
days in a way that is painful to read in order.

It opens with a print statement: intercept a text event, print it, confirm the patched library hands
it over. Then a test harness exercising the newly published API — with commented-out experiments in
transposing, and a commented-out line muting a channel's volume, which is the melody guide, which is
the single most karaoke feature there is. Then something genuinely good: a handler that builds a model
of the song. Per track, the lowest and highest note; the dominant instrument and channel, decided by
histogram rather than by trusting the first event; every note with a start and an end; every lyric
with a start and an end. That is exactly the analysis you need to find the vocal line and time a wipe
across it.

Then three days and thirty-two commits go into a C++ wrapper for the synthesizer — fifteen classes,
a shared pointer alias and a factory function on every one, private implementation pointers
throughout, an enumerable typed settings tree. Every lyric-related method on it is behind a build
option named after the private fork it requires.

And then, on one afternoon, four commits: a library. It reads the karaoke conventions correctly —
the header fields for file type, language, title and information, and the two prefix characters that
start a new line — and assembles the file into timed lyric lines held in an ordered map. It declares
two fields for how the words are meant to appear:

```cpp
int _lines_shown{2};
int _lines_preload{3};
```

Two lines on screen, three buffered ahead. Neither field is ever read. The events that would tell a
display when the lines changed are declared and never fired. And the demo program that ties it all
together ends with a bare `return;` inserted immediately in front of the playback call, leaving
everything after it — the callbacks, the play, the wait — as dead code.

It got as far as parsing a karaoke file into timed lyric lines and printing them. That is the
afternoon it stopped.

## 2020 — a clean reset, and the same wall

The fifth attempt gets the diagnosis right and reaches the same place anyway.

It throws out FluidSynth entirely, and with it the patched fork, the wrapper, and the two days of
build system in front of every experiment. In its place: a SoundFont synthesizer in a single header,
an audio backend in a single header, and a MIDI file reader — three submodules, no installed
dependencies, nothing to configure.

And it works. An audio callback advances the file by wall-clock time and drives the synth directly:
program changes, notes on and off, controllers, and a pitch wheel reassembled by hand from its two
seven-bit halves. Lyrics print as they arrive, with their timestamps, in sync.

The library header the whole thing is supposedly for is **zero bytes long**. All of the code is in a
directory called `tryout`. The main loop is `while(true) { Sleep(100); }` with the exit condition
commented out. The final commit is named `debug`, and what it contains is one unused variable
capturing a start time and an edit to a commented-out print — somebody three days in, beginning to
look at timing drift in the audio callback, who did not come back.

## Every attempt died in the same place

Read the five together and the shape is unmistakable.

**The hard part was never hard.** A lyric is a string and a tick. All five got that far; four of them
got audio out of a speaker at the same time. The decoding, the synthesis, the timing — the parts that
sound like the engineering — were done, repeatedly, by one person in an evening.

**Only the first one ever had a screen.** After 2008, every attempt printed lyrics to a console.

**Every one was a burst.** Two days, three days, eight days, eight days. Only 2008 sustained months,
and only 2008 had a running application at the end of it. Each burst spends its first half on library
surgery and its second half on the thing it wanted to build, and stops before the second half is
finished.

**The package format was designed twice, from nothing, six years apart**, and neither version could
read back what it wrote.

**Nothing ever grew a file chooser.** All five were bench-tested against a single karaoke file at a
hardcoded absolute path, changed only when the folder moved.

What none of them ever reached is not a technique. It is a catalog. A number typed on a pad that
finds a song. A queue somebody else can add to. A package format that reads back what it wrote. A
screen that draws a wipe across a syllable. A remote. An installer. Something that survives being
handed to a person who did not write it.

That layer is large, it is dull, and there is nothing in it worth an evening. It is where five
attempts went to die.

## 2026 — fourteen days

| | |
|---|---|
| Span | 2026-08-23 to 2026-09-05, **fourteen consecutive days** |
| Commits | **1,080** — a mean of 77 a day, peaking at 118 |
| Releases | eight tags in thirteen days, the first on day two |
| Crates | 34 |
| Rust | 155,803 lines across 244 files |
| Tests | **2,206** |
| Decisions | 250, across eleven topic files |
| Documents | 41 markdown files under `docs/`, 17,631 lines |

The construction ran on numbered milestones, M0 to M38, and the numbering stops on the tenth day —
not because the work did, but because the plan document was split into
[`ARCHITECTURE.md`](ARCHITECTURE.md) and [`docs/architecture/`](architecture/) and the project stopped
narrating its own construction. Growth in the test count is the better spine anyway: 62 after the
parser, 577 once the machine ran at all, 1,074 at the first remote, 1,654 at the last numbered
milestone, 2,206 now.

Two measurements from that stretch matter more than the totals, because they are the thing eighteen
years of single-file benches never had. The parser was validated against **44,355 real karaoke files:
99.81% parsed, zero panics.** The melody detector was measured against 52,833 of them and found a
melody in 65.9%.

What it reaches, that none of the five did: three kinds of song rather than one — karaoke MIDI, video,
and MP3+G. A catalog, and a printed song book to go beside it. An HTTP API with an event stream and
per-endpoint access control, and machines that find each other on the network. Two remotes, one served
by the machine and one that works when the machine is switched off, over a shared core with five
different hosts. Applications for Android and iOS. Installers for Windows, macOS and Debian. And an
appliance mode that draws straight to the framebuffer from a virtual terminal on a box with no display
server on it at all.

## What actually changed

Not the language, and not the libraries. The 2020 stack — a SoundFont synthesizer, a MIDI reader, an
audio callback advancing by wall clock — is recognisably the stack this machine runs on. That attempt
had the right idea about its dependencies and died anyway, three days in, on a commit named `debug`.

What changed is that the layer that killed all five stopped costing evenings. The package format that
had to be designed twice; the second day of build plumbing standing in front of every experiment; the
wrapper around the wrapper; the catalog, the queue, the keypad, the installer — the work that is
large and obvious and has no problem in it worth solving. None of that got easier to *think* about. It
got cheap to *do*, and that turns out to be the whole difference between five attempts and a machine.

Two disciplines carried it, and neither is about writing code faster. The first is that every product
decision was written down as it was made — 250 of them, in topic files, in the same commit as the code
they justify. That is not documentation of the work. At 77 commits a day it is the only reason the
work stayed reversible: a decision on disk is a decision nobody re-litigates on day nine. The second
is the corpus. Hundreds of thousands of real files, and a parser measured against tens of thousands of
them, is what replaced twelve years of testing against the same one song at a hardcoded path.

The five attempts were not short of skill or short of ideas. Every one of them proved it could do the
part that looked hard, and then ran out of evenings in front of the part that was merely long.
