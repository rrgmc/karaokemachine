# Song files and their formats

- **MIDI and KAR**, in all three karaoke conventions: Soft Karaoke `@`-headers, `Lyric` meta-events,
  and a named text track.
- **A video song** is H.264 with AAC audio, in an MP4.
- **An MP3+G song** is an MP3 with a `.cdg` of the same stem. The machine draws the CD+G graphics
  itself, in Rust.
- **An UltraStar song** is a `.txt` beside the MP3 it names. The machine reads its timed words and
  discards its pitches.
- **An LRC song** is an `.lrc` beside the MP3 of the same name. A file that times each word gets the
  word wipe. A file that times only its lines lights a whole line at a time, and counts you back in
  after a break.
- **A song's language is an ISO 639-1 code.**

## Text encodings and writing systems

- **The machine detects a legacy encoding**: Shift-JIS and the Windows code pages.
- **CJK works**: a Japanese or Chinese song uses a font from the system.
- **No shaped scripts, and no right-to-left.** Thai, Arabic and Indic need a text shaper, and this
  build does not include one.
