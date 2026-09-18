# Research: what a `.kar` file puts in its lyric stream

**Research only. Nothing here is a commitment.** The parser handles what it needs to; this note
exists so the next surprise is looked up rather than hit, and so a rule added to it can be weighed
against what the corpus actually holds.

Surveyed 2026-09-10 against 4,000 `.kar` files sampled from the local corpus,
1,357,319 text and lyric meta events between them.

| Marker | Meaning |
|---|---|
| **[bytes]** | Counted from the files. High confidence. |
| **[web]** | Published sources. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

**Summary.** Five formats share the extension. Only Soft Karaoke matters here by volume, and its
documented markers are `@` tags plus `/` and `\`. What the documentation does not say is that the
lyric stream is also where sequencers put their business cards, publishers put their legal notices
and arrangers put their section labels — carried three different ways, only one of which announces
itself. `km_song::looks_like_a_banner` is the rule that knows them.

## 1. The five formats, and their markers

**[web]** Every one of these has been seen in the wild under a `.kar` or `.mid` extension.

| Format | Lyrics carried in | Markers |
|---|---|---|
| **Soft Karaoke** (Tune 1000, 1993) | text meta events, `FF 01` | `@K` file type, `@V` version, `@I` information, `@L` language, `@T` title; `/` next line, `\` clear screen |
| **Tune1000** | lyric meta events, `FF 05` | one syllable per event, a space after a word's last syllable; `FF 05 01 0D` ends a line, `FF 05 01 0A` clears the screen; chords in text events prefixed `%` |
| **Solton** (Keytron) | lyric meta events, `FF 05` | `<` opens a lyric line, `%` opens a chord; controller 31 on channel 1 drives the highlight |
| **MidiSoft** | system exclusive, maker `00 20 24` | `00 04`–`00 07` are four lines, `00 08` syncs, `00 01` is a chord |
| **Yamaha XF** | sysex chord control code | `NOTE` + accidental + chord type, from a fixed list of 35 |

**[bytes]** The `%` chord prefix and the `<` line opener both appear in this corpus, and both are
handled — see `A marked event is not a word` in `docs/decisions/songs.md`. The MidiSoft and Yamaha
sysex forms were not looked for.

## 2. What the sampled events actually start with

**[bytes]** Leading character of every text or lyric meta event, as a share of 1,357,319:

| Opens with | Share | What it is |
|---|---|---|
| space | 38.6% | the syllable separator — a space *before* the syllable rather than after |
| `/` | 8.7% | next line |
| `\` | 3.2% | clear the screen |
| `@` | 1.6% | a tag, or prose that happens to start with `@` — see below |
| `\r` | 1.4% | a line ending that reached the payload |
| a letter | the rest | a syllable |

**[bytes]** 32,845 control bytes sit inside lyric text, 31,081 of them NUL. They come from
fixed-length text fields in the writing tool. `clean_meta_text` and the syllable cleaner take them
out; see `A lyric field's terminator no longer reaches a font` in the decisions.

## 3. Most "undocumented tags" are not tags

**[bytes]** Beyond the five documented tags, the sample holds `@W` 573 times, `@E` 272, `@M` 196,
`@U` 85, and a dozen others once or twice each. Reading their payloads settles what they are:

- `@E` is `@E-mail:`, `@U` is `@Universal Lyrics Editor`, `@M` is `@Midi Songs Karaoke System`,
  `@Y` is `@You Like Me Too Much` and `@F` is `@Filhos da madrugada`. **The `@` is the first
  character of a sentence, not a tag.** Two of those are the song's title, lost because nothing reads
  a tag it does not know.
- `@W` is the exception and behaves like a real tag: 523 of its 573 events carry one publisher's
  legal notice, split across two or three events.

**[inferred]** A writer emitting `@Y` for a title is likelier to be a person typing into an editor
than a tool with a specification. Each such tag stands in one or two files of the 4,000.

## 4. The legal notice is carried three ways, and only one announces itself

**[bytes]** The same sentence, `ALL rights reserved. Not for broadcast or transmission of any kind.
DO NOT DUPLICATE. NOT FOR RENTAL.`, appears as:

| Carried as | Announces itself |
|---|---|
| an `@W` text event | **yes** — every `@` event is metadata and is not sung |
| a bare text event with no `@` | no |
| a lyric event `FF 05` with no `@` | no |

The last two reach the words. On the curation corpus, 55 songs had that notice and nothing else as
their entire lyric text.

**Nothing in it is an address, a web address or a telephone number**, so a rule that looks for
contact details does not see it. `km_song::looks_like_a_banner` does, and carries the continuation
`transmission of any kind` for the case where the sentence wraps onto a second event.

**[bytes]** Other non-lyric shapes in the same sample: 1,537 sequencer credits, 1,328 email
addresses, 570 copyright lines, 446 lines of legal boilerplate, 304 telephone numbers, 209 web
addresses. 365 events are longer than 60 characters, which is a whole line written as one event
rather than as syllables.

## 5. What follows from it

**Nothing is proposed here.** Two things are worth knowing:

- **The rule to reuse is `looks_like_a_banner`**, not a narrower contact-detail test. It already
  holds the legal phrases in two languages, the credits, the addresses and the section labels, and
  it is public. `lyric_key` in `km-package-builder` uses it for exactly this reason.
- **A line that is only the notice is dropped from the timeline**, by a third rule narrower than
  either of the other two — see `A line that is only a legal notice is not sung` in the decisions.
  A notice that names a publisher is not dropped: it keeps a name, so it is a banner rather than
  only a notice, and the preview skip is what handles it.

## Sources

- [Karaoke Formats — Mixage Software](https://www.mixagesoftware.com/en/midikit/help/HTML/karaoke_formats.html),
  the only page found that catalogues all five formats and their markers together.
- [How many MIDI karaoke formats exist? — MIDI Association](https://midi.org/community/midi-specifications/how-many-midi-karaoke-formats-exist)
- [Working with Karaoke (.kar) Files — Notation Software](https://www.notation.com/MusicianDocs5/viewing_and_editing_karaoke.htm)
