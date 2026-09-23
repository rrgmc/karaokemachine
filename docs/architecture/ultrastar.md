# UltraStar songs

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

An UltraStar song is a `.txt` of syllables timed in beats and the MP3 its header names. The decisions
are `UltraStar as a song source` and the four entries after it in
[`song-sources.md`](../decisions/song-sources.md#ultrastar-as-a-song-source); the format and the
measured dialects are [`docs/research/ultrastar.md`](../research/ultrastar.md).

**Three crates read the file, and the machine is not one of them.** `km_song::ultrastar` parses it,
`km-pack` and `km-package-builder` call that parser, and a package carries the result. The machine
reads a lyric timeline and an MP3.

## The parser

`km_song::ultrastar::parse` turns bytes into a `LyricTimeline` whose ticks are milliseconds from the
start of the audio: `ms = GAP + beat × 60000 / (BPM × 4)`.

| Shape | What the parser does |
|---|---|
| `#RELATIVE:yes` | Adds each `-` line's second number to the beats after it. Refused in a versioned file. |
| Two-number `-` in an absolute file | Reads the first number only. |
| Decimal comma | Reads `,` as `.` in every number, `#VERSION` included. |
| `~` syllable | Extends the syllable before it and draws nothing. |
| `P1`/`P2` | Refuses the file as a duet. |
| A space at a line's edge | Trims it, so a centred line is not off-centre by a space. |
| CR, LF or CRLF | Reads all three. |

**Encoding is the part real files disagree with the specification about.** Three rules came out of
the local collection:

- **Every header value is sampled for detection, not only the words.** A file with ASCII words can
  name its audio `DIE ÄRZTE - ….mp3`. A decoder chosen from the words alone reads that name as UTF-8
  and finds no file.
- **A versioned file is not declared UTF-8.** Editors write `#VERSION:1.1` on CP1252 files. Valid
  UTF-8 is still read as UTF-8, because `TextDecoder::resolve` checks validity before detecting.
- **`#LANGUAGE` is the domain hint `chardetng` takes**, through
  `TextDecoder::resolve_for_domain`. Five accented letters in a Portuguese file read as well in
  windows-1250 as in windows-1252, and without the hint `ê` becomes `ę`.

**A syllable keeps its note's length.** `RawSyllable::end_tick` is `Some` for an UltraStar note and
`None` for a MIDI lyric. `build_timeline` ends a syllable at the earlier of it and the next timing
point. It leaves a MIDI file's timeline unchanged, so no stored row moves and `ANALYSIS_REVISION`
stays where it was.

`tests/ultrastar_corpus.rs` sweeps a real folder, ignored by default:

```sh
KM_ULTRASTAR_CORPUS=<your UltraStar folder> cargo test -p km-song --test ultrastar_corpus -- --ignored --nocapture
```

## In a package

| Entry | Holds | Stored |
|---|---|---|
| `media/<n>.mp3` | the audio, byte for byte | uncompressed, so it is seeked into |
| `media/<n>.json` | the serialized `LyricTimeline` | deflated, read whole |

**A rule names the second entry**, as it names a CD+G file's. `km_kmpkg::companion_entry_for` is the one
place it is named, so `missing_entries`, `media_entries` and `add_media_copied` cannot disagree about
which songs have one. A manifest holding an UltraStar song is format 5; a package without one keeps
the version its content gave it.

**The content hash covers the MP3 and the `.txt` together.** A retimed `.txt` over the same recording
is a different song.

## Finding the song on disk

**A `.txt` is a candidate, and reading it decides.** `km_pack::read_ultrastar` parses the file, finds
the audio by its header, case-insensitively, and refuses a video, a non-MP3 and a missing file.
Readme files are neither songs nor failures.

**The MP3 and the `#VIDEO` a song names are part of it.** `UltraStarSource::claimed_media` lists them,
and two walks use it:

- `km-pack spec` drops them from the video list and from the MP3s with no `.cdg`.
- The curation scan gives each one a file row with no song and no failure, through
  `km_pack::ultrastar_naming`.

**The skip test folds the naming `.txt` into an unpaired MP3's size and mtime.** It does the same for
a video, as it folds a `.cdg` into its MP3's. A row written before the `.txt` counted then differs, and the
file is read again once. A `.txt` is compared on its own size and mtime. Reading every text file in a
corpus to find its audio would cost a disk seek per file on every scan.

## Playing it

**The audio is an MP3+G song's audio without the graphics.** `km_cdg::open_audio_from` starts the
same decoder thread and feed, and the song reaches the engine as `Load::Track`.

**The words are a `km_song::Song` with nothing to play.** `recording::song_from_timeline` builds one
with no events and a timecode tempo map at 1,000 ticks a second. It does so because the lyric view,
the `lyric_line` events and `LyricsDto::from_song` all read a `Song`. The machine holds it in
`TimedSong`, which an LRC song shares: see [`lrc.md`](lrc.md).

Four things differ from a MIDI song, and each is a trap:

- **`Loaded::song()` stays MIDI only, and `Loaded::lyric_song()` is the words of either kind.** A bank
  switch reloads whatever `song()` returns into the synthesizer.
- **The clock is the audio's position**, through the song's tempo map, in the display, the stream
  and `announce_lyric_line`. `Player::position_ticks` is 0 for every track. The lyric offset is
  not scaled by the tempo setting, which belongs to the next MIDI song.
- **The lyric view's lead-in is eight beats of `Song::beat_ticks`**, half a second for a timecode
  song. Built from `ticks_per_quarter`, which is 0 for one, it read ahead eight ticks.
- **The key and tempo badges read neutral values for any kind but MIDI**, through
  `display::drawn_adjustments`. The frame's `picture` flag is false for an UltraStar song, and that
  flag is what hid them for a video.
