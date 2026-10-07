# Song files and their formats

The machine plays five kinds of song. Each one is a file, or a pair of files, that holds both the
music and the timed words. An MP3 on its own is not a song, because it has no words in it.

## The five kinds of song

| Kind | The files | What the machine does with it |
|---|---|---|
| **MIDI and KAR** | One `.mid` or `.kar` | Reads all three karaoke conventions: Soft Karaoke `@`-headers, `Lyric` meta-events, and a named text track. |
| **Video** | One MP4, with H.264 video and AAC audio | Plays it as it is. |
| **MP3+G** | An MP3 with a `.cdg` of the same stem | Draws the CD+G graphics itself, in Rust. |
| **UltraStar** | A `.txt` beside the MP3 it names | Reads its timed words and discards its pitches. |
| **LRC** | An `.lrc` beside the MP3 of the same name | Fills each word, or lights each line. |

**An LRC file decides its own highlight.** A file that times each word gets the word wipe. A file
that times only its lines lights a whole line at a time, and counts you back in after a break.

**A song's language is an ISO 639-1 code**, such as `pt` or `en`.

These files reach the machine inside a package. See [Getting songs in](songs.md).

## Text encodings and writing systems

Many karaoke files are old, and their words are not in Unicode.

- **The machine detects a legacy encoding**: Shift-JIS and the Windows code pages.
- **CJK works**: a Japanese or Chinese song uses a font from the system.
- **No shaped scripts, and no right-to-left.** Thai, Arabic and Indic need a text shaper, and this
  build does not include one.
