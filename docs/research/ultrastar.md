# Research: UltraStar (`.txt`) as a song source

**Research only.** What was decided is `UltraStar as a song source` and the four entries after it in
[`docs/decisions/song-sources.md`](../decisions/song-sources.md#ultrastar-as-a-song-source), and how
it is built is [`docs/architecture/ultrastar.md`](../architecture/ultrastar.md). Where this note and
either of those disagree, they hold.

**Scope: the synced lyrics only.** UltraStar is the file format of singing games, and each note
carries a pitch and a type that the games score against. Scoring singers is a non-goal, so this note
reads a file for its syllables and their times and discards the rest.

**Summary.** A plain-text file of syllables timed in beats, beside an audio file that it names,
gives syllable-level highlighting as good as a well-made `.kar`. The format has a published
specification, and turning it into this project's lyric timeline is a small parser. The playback
side reuses the MP3+G audio path and the MIDI lyric renderer, and needs the renderer to follow the
audio clock. A local collection of these songs, each with its MP3 beside it, was measured for the
dialects a parser meets. Public content is text only: the owner supplies every song's audio.

| Marker | Meaning |
|---|---|
| **[bytes]** | Counted from the files of a local collection. High confidence for that collection. |
| **[web]** | Public sources: the specification, the UltraStar Deluxe source code, community sites. |
| **[code]** | Verified in this repository. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. The format, reduced to lyrics

**[web]** The specification is `github.com/UltraStar-Deluxe/format`, MIT-licensed, in three
documents: unversioned files, v1 and v2. A file declares its version in a `#VERSION` header, and a
file without one is unversioned. An application refuses a major version it does not know; a minor
version adds only backward-compatible tags.

| Version | Status |
|---|---|
| unversioned | Everything written before 2023. Most files in circulation. |
| 1.0.0 | Published 2023. The specification states that 1.1.0 is equivalent to it. |
| 1.2.0 | Adds `#AUDIOURL`, `#VIDEOURL`, `#COVERURL` and `#BACKGROUNDURL`. Named by third-party tools, not by the specification. |
| 2.0.0 | Unpublished and marked as liable to change. |

A file is a header of `#TAG:value` lines followed by one event per line:

```
#VERSION:1.0.0
#TITLE:Song
#ARTIST:Someone
#AUDIO:Someone - Song.mp3
#BPM:280
#GAP:12500
: 0 4 60 Hel
: 4 2 62 lo
- 8
* 10 6 64  world
E
```

### The header tags a lyric reader needs

| Tag | Use |
|---|---|
| `#TITLE`, `#ARTIST`, `#LANGUAGE` | The song's name and language. |
| `#AUDIO` | The audio file, relative to the `.txt`. An application ignores `#MP3` when `#AUDIO` is present. |
| `#MP3` | The same, in unversioned files. |
| `#BPM` | The beat rate. Accepts `.` or `,` as the decimal separator in v1. |
| `#GAP` | Milliseconds from the start of the audio to beat 0. |
| `#ENCODING` | Unversioned files only: `UTF8`, `CP1252` or `CP1250`. v1 requires UTF-8. |
| `#RELATIVE` | Unversioned files only. See below. |

`#VIDEO`, `#VIDEOGAP`, `#COVER`, `#BACKGROUND`, `#START`, `#END`, the medley and preview tags, and
`#VOCALS`/`#INSTRUMENTAL` have no use here. `#START` and `#END` do not move notes.

### Event lines

| Line | Meaning | Kept |
|---|---|---|
| `: start length pitch text` | A normal note. | start, length, text |
| `*`, `F`, `R`, `G` with the same fields | Golden, freestyle, rap and golden rap notes. | start, length, text |
| `- beat` | The end of a phrase, which players show as a line break. | beat, as a line break |
| `P1`, `P2` | The notes after it belong to that voice of a duet. | see below |
| `E` | The end of the song. Anything after it is ignored. | — |

**Timing.** The file's BPM is a quarter of the beat rate, so the time of a beat is
`ms = GAP + beat × 60000 / (BPM × 4)`. UltraStar Deluxe computes exactly this: `USong.pas` multiplies
the header value by 4, and `UNote.pas` converts a beat as `GAP / 1000 + beat × 60 / BPM` seconds. A
syllable starts at `start` and ends at `start + length`.

**Words.** The note text is one syllable. A space at the start or end of the text marks a word
boundary, and `Hello` + ` World` is the same as `Hello ` + `World`. Syllables with no space between
them form one word. A syllable written as `~` is a community convention for a sustained vowel across
several notes. It has no text of its own and extends the syllable before it.

**Pitch** is in semitones from C4 and may be negative. It is discarded.

**Relative mode.** In an unversioned file with `#RELATIVE:yes`, beats restart at each phrase, and a
`-` line carries a second number that offsets the following beats. v1 removed it and UltraStar
Deluxe refuses it in a versioned file. The community database converts such files to absolute
beats, and so can a parser. It adds a `-` line's second number to every beat that follows it,
cumulatively, until the next `-` line adds its own.

**Tempo changes.** Unversioned files can carry `B` lines. UltraStar Deluxe logs and ignores them, and
no specification defines them.

**Duets.** `P1` and `P2` give two voices, each with its own notes and phrase breaks. A lyric display
has one stream, so a duet needs a rule. It could show voice 1 only, or merge both voices by time,
which interleaves two phrasings. **Open.**

## 2. Dialects and failure shapes

**[web]** What a parser meets in files from the wild:

- **Encoding.** Unversioned files are often CP1252 or CP1250, with or without `#ENCODING`. Some forums
  tell authors to save as "ANSI" with CRLF line endings. A UTF-8 BOM appears and is ignored.
- **Decimal comma** in `#BPM` and `#GAP`.
- **Audio that is not MP3.** `#AUDIO` names OGG, M4A and Opus files as well.
- **Video-only songs**, whose `#AUDIO` or `#MP3` names the video file.
- **The two-number `-` line** in absolute files, where older editors wrote when the line disappears
  and when the next appears. Only the first number marks the break.
- **Line endings** CR, LF or CRLF.

**[bytes]** What a local collection holds, as shares of its UltraStar files:

| Feature | Share | What it means for a parser |
|---|---|---|
| No `#VERSION` | 99.5% | Unversioned rules are the normal case. |
| `#VERSION:1.1` or `#VERSION:1,00` | 0.5% | A version string can carry a decimal comma, which the specification does not allow. |
| `#MP3` names the audio | 100% | No file has `#AUDIO`. Every file names an `.mp3`, and the file is present. |
| `#RELATIVE:yes` | 18% | Common enough that refusing it drops a fifth of the songs. Every such file uses the two-number `-` line. |
| Two-number `-` line in an absolute file | 12% | Only the first number is read. |
| Decimal comma in `#BPM` | 55% | The normal case, not an edge case. |
| Decimal comma in `#GAP` | 8% | |
| `#ENCODING` | 0% | No file declares its encoding. |
| Pure ASCII | 76% | |
| Not valid UTF-8 | 24% | A legacy code page, found only by detection. No file has non-ASCII UTF-8. |
| UTF-8 BOM | 0% | |
| LF line endings only | 8% | The rest are CRLF. No CR-only file. |
| `P1`/`P2` duets | 0% | |
| `B` tempo lines | 0% | |
| `E` at the end | 100% | |
| `#VIDEO` beside `#MP3` | 5% | Mostly MPEG. The audio file is still the song. |

Note types are almost all `:`, with a few `*` golden and `F` freestyle notes, and no `R` or `G`. The
`~` sustained syllable appears in 8% of files. The collection folders also hold `.sco` high-score
files that the game writes. They hold `ReadMe!.txt` files with no `#TITLE` too, so a `.txt` without
a `#TITLE` header is not a song. About half of the files repeat a song from another folder
of the same collection.

## 3. Where content comes from

**[web]**

- **USDB** (`usdb.animux.de`) is the community database, running since 2009 with more than 18,000
  songs. It hosts `.txt` files only, never audio. A download appears to need an account. No terms of
  use, scraping policy or API were found.
- **`usdb_syncer`** (GPL) is the common way songs are assembled. It downloads a USDB file and fetches
  the audio, video and cover from the links in its header, mostly YouTube. The audio of every song is
  therefore something the user sources.
- **Other databases**: `ultrastar-es.org` and `usdb.eu`, both indexed at `usdb.hehoe.de`.
- **Lyrics are copyrighted.** US courts have held that showing lyrics in time with music needs
  synchronisation and reprint licences. An UltraStar file carries the lyric text, so it is not
  redistributable content in its own right.
- **Karaoke software outside the games** barely uses the format. Karaoke Mugen imports it by
  converting to ASS subtitles, and My Little Karaoke reads it. No karaoke hardware was found that
  accepts it.

**[bytes]** The local collection holds songs as a player's game folder keeps them. Each song has one
folder, holding the `.txt`, its MP3, a cover image, sometimes a video, and the game's score files. The files
are not redistributable either, so fixtures stay synthetic and reproduce the shapes in section 2.

## 4. How it would fit

**[code]** except where noted.

### The song kind

`SongKind` in `crates/song/km-kmpkg/src/manifest.rs:53` gains a variant, and the package format
version rises with it. `Unknown` already exists so that an older build refuses the new kind
cleanly. Three other places match on the kind:

- the package builder's own `SongKind` in `tools/cmd/km-package-builder/src/model.rs:21`;
- the machine's `Media` enum in `crates/machine/karaokemachine/src/machine.rs:68`;
- the catalog's row reader in `crates/song/km-catalog/src/lib.rs:1209`. It reads an unrecognised
  kind as MIDI by design, so the new kind must be named there or its songs fail to load.

### Audio

`km-cdg`'s `AudioReader` decodes on a thread into `km_audio::track::TrackPlayer`, which resamples,
seeks and reports `position_ms` from the samples actually played. The audio is the master clock, which
is what lyric highlighting needs. An UltraStar song reuses this without the graphics half.

`symphonia` is built with the `mp3` feature only (root `Cargo.toml:314`). Every song in the local
collection is MP3 **[bytes]**. OGG Vorbis, which community files name **[web]**, needs the `ogg` and
`vorbis` features; M4A and Opus are larger additions **[inferred]**.

### Lyrics

The timeline is `LyricTimeline`, built by `build_timeline` in `crates/song/km-song/src/timeline.rs:637`
from raw syllables, with `LineInference` for songs without line marks. It counts time in MIDI ticks,
but `TempoMap` has a `Timebase::Smpte { ticks_per_second }` (`crates/song/km-song/src/tempo.rs:33`), so
a timeline at 1,000 ticks a second counts milliseconds. The renderer, `LyricView::frame` in
`crates/playback/km-display/src/lyrics.rs:104`, draws the syllable wipe from a timeline and a tick.

UltraStar's `-` lines are explicit line breaks, so no inference runs on a well-formed file.

Two things block this today:

1. `Player::position_ticks` (`crates/playback/km-audio/src/player.rs:150`) returns 0 for every track
   song. The lyric wipe needs the track's `position_ms` instead.
2. The machine treats every non-MIDI kind as a song that brings its own picture:
   `has_own_picture = !now.kind.is_midi()` in `crates/machine/karaokemachine/src/display.rs:2382`, and
   `announce_lyric_line` (`machine.rs:1759`) reads only `Media::Midi`. An UltraStar song draws its words
   over the still wallpaper, as a MIDI song does, so both become a question about the kind.

`Lyric timing offset` (`docs/decisions/audio.md`) applies unchanged, since it moves the display and
not the audio.

### What the song does not have

Transpose, tempo and the guide melody answer a 409 with code `unavailable`, as they do for MP3+G.
Transposing an audio file is a standing non-goal. The pitch data in the file does not change that,
because it is a sung melody, not a backing track.

### Suitability

A flat 10, as `Suitability, for a song that was made to be sung to` in `docs/decisions/songs.md`
gives video and MP3+G. A person timed the words for this recording. Whether that holds for
community files of uneven quality is part of the decision **[inferred]**.

### Pairing and packaging

The scanner finds an MP3+G pair by stem: `pair_for` in `crates/song/km-kmpkg/src/lib.rs:778` looks
for the `.cdg` beside an MP3. An UltraStar file names its audio in `#AUDIO` or `#MP3`. The `.txt` is
therefore the file the scanner starts from, and the header rather than the stem leads to the audio.
In the local collection the header is always `#MP3`. In 16% of files the audio's stem differs from
the `.txt` stem, so matching by stem would miss those songs **[bytes]**. The
scanner's dispatch in `tools/cmd/km-package-builder/src/scan.rs`, `missing_entries` (`lib.rs:358`) and
`pair_content_hash` (`lib.rs:824`) each need the second rule.

Packaging renames a pair to `media/<number>.*`. The `.txt` can be stored the same way, so the machine
derives both names from the number and never reads the original `#AUDIO` value.

**A lyric timeline could instead be computed at packaging time** and stored in the package, so the
machine never parses UltraStar at all **[inferred]**. That keeps the dialect handling in the builder,
which is where the corpus's other messy pairing already lives.

### Encoding

`TextDecoder::resolve` (`crates/song/km-song/src/encoding.rs:66`) takes a declared label first, then
valid UTF-8, then detection, then windows-1252. An `#ENCODING` tag maps to a declared label, and a
v1 file is UTF-8 by rule.

### What it gains over MP3+G

- Title, artist and language come from the header.
- The songbook's first line (`SongRow.first_line` in `crates/song/km-songbook/src/lib.rs:127`) and
  the lyric-line announcement work, because the words are text.

### A parser

The Rust crate `ultrastar-txt` is MIT-licensed and unmaintained for seven years. The Go package
`codello.dev/ultrastar` (MIT) covers every version, relative mode and the legacy encodings, and is
the better reference for dialects. A reader for the lyric subset is small enough to write in
`km-song` **[inferred]**.

## 5. No conversion path

The cheaper path for `.st3` was to convert to MIDI. It does not exist here: a `.kar` carries its own
music, and an UltraStar song's music is a recording.

## 6. Effort and risks

| Item | Estimate **[inferred]** | Confidence |
|---|---|---|
| Lyric-subset parser, dialects, synthetic fixtures | 2–3 days | medium |
| Song kind through the package format, catalog, `km-pack` and the builder's scan | 3–5 days | medium |
| Lyric wipe on the track clock, and the MIDI-only gates | 2–3 days | medium |
| Relative mode, converted to absolute beats | half a day | high |
| OGG Vorbis audio, which the local collection does not need | 1 day | high |

1. **One collection is not the community database.** The shares in section 2 come from one player's
   game folder. Files downloaded from USDB may carry `#VERSION`, `#AUDIO` and OGG audio far more
   often **[inferred]**.
2. **The audio is the user's to find.** No database distributes it, so a song exists only when
   somebody pairs a text file with a recording that matches its `#GAP` and `#BPM`. A mismatched
   recording plays with its words in the wrong place, and nothing detects that.
3. **Duets need a rule** before a duet file can play. The local collection has none, so refusing
   them costs nothing there.
4. **Video-only songs** are a video song with a lyric timeline, which `What a video song does not
   have` rules out.
5. **Community timing is uneven.** A flat suitability of 10 may overrate some files.

## 7. Recommendation

If somebody takes it up: **accept v1 and unversioned files, convert relative mode to absolute beats,
and read one voice and MP3 audio**. OGG is a later addition. Read a decimal comma in any number,
and detect the encoding when no `#ENCODING` is present. Refuse duets and video-only songs, and find
the audio by the header, never by the stem. Build the timeline at packaging time. Record the
decision in `docs/decisions/song-sources.md` beside `MP3+G as a song source` before writing code.

## 8. Found in passing

- `What an MP3+G song does not have` in `docs/decisions/song-sources.md` says an MP3+G song's
  suitability is absent. `Suitability, for a song that was made to be sung to` in
  `docs/decisions/songs.md` gives it a flat 10. The second is the one the code follows (`km-pack`
  stamps 10).
- `MP3+G as a song source` cites `No audio-file pitch shifting`, which is a non-goal in `CLAUDE.md`
  and has no heading in `docs/decisions/`.
