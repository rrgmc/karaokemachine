# LRC songs

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

An LRC song is an `.lrc` of timed lines and the MP3 of the same stem. The decisions are `LRC as a
song source` and the four entries after it in
[`song-sources.md`](../decisions/song-sources.md#lrc-as-a-song-source). The display's half is
[`A line-timed song lights a line at a time`](../decisions/interface.md#a-line-timed-song-lights-a-line-at-a-time).

**After the parser, an LRC song is an UltraStar song under another name.** Both are an MP3 and a
millisecond `LyricTimeline`, stored, loaded and played by the same code. Read
[`ultrastar.md`](ultrastar.md) for everything past the parser: the package entries, the audio clock
and the four traps. `SongKind::carries_timeline` is the question that code asks, and `is_lrc` is
asked only where a label differs.

## The parser

`km_song::lrc::parse` turns bytes into a `LyricTimeline` in milliseconds, one raw syllable per line
or per word.

| Shape | What the parser does |
|---|---|
| `[ti:]`, `[ar:]`, `[offset:]` | Reads them. Every other tag is ignored. |
| Several timestamps on a line | Emits the line once per timestamp, with its word tags moved by the same amount. |
| A blank timestamped line | Sets the end of the line before it. |
| `<mm:ss.xx>` before a word | Times the word. A tag with nothing after it ends the word before it. |
| A space between words | Moves to the start of the next word. |
| `M:`, `F:`, `D:` | Drops the part marker and keeps the words. |
| Two lines at one time | Keeps the first. |
| A UTF-16 byte-order mark | Re-encodes the file as UTF-8 before reading it. |

**The markup is found in the raw bytes.** `[`, `]`, `<` and `>` never occur inside a multibyte
character in UTF-8, Shift-JIS, GBK or Big5. So the brackets are cut before the encoding is known,
and the whole file's text then decides it once.

**The space moves because of `marks_no_word_ends`.** It reads a trailing space on almost every
fragment as a file that spaces its syllables, and draws dividers between them. Whole words written
`we go up` would trip it. One leading space disqualifies the rule, so the words keep their spacing.

**A line-timed file holds its last line for `LAST_LINE_HOLD_MS`**, three seconds. A word-timed file
holds its last word for the nominal half-second beat, as an UltraStar file does.

## Finding the song on disk

**A song starts from its `.lrc`, and `km_pack::read_lrc` finds its MP3 by stem.**
`km_kmpkg::sibling_with_extension` makes the search `pair_for` makes for an MP3+G pair, with a
different extension. `km_pack::read_lrc` refuses an MP3 that `pair_for` or `ultrastar_naming` already
claims, so the precedence is one check in one place.

**The claimed MP3 is folded into the scan's skip test**, as an UltraStar song's is. `lrc_naming`
finds the `.lrc` beside an unpaired MP3. A row that read the MP3 as half a pair differs in size once
the `.lrc` counts, and is read again once.

## Suitability

`km_pack::purpose_made_suitability_for` takes the timeline's granularity. A line-level timeline gets
`SuitabilityRecord::purpose_made_line_timed`, an 8 with the `linelevellyrics` warning, unless the
singing is too brief. The curation scan and `km-pack reanalyze` call the same function.
