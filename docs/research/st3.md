# Research: Star 3 (`.st3`) karaoke format

**Research only. Nothing implemented, nothing decided.** Supporting `.st3` is a requirements change —
`Song sources` in `docs/decisions/song-sources.md` names the song sources a package may hold — so it
needs a decision recorded there first.

Investigated 2026-08-23 against the local corpus.

**Summary.** A re-encoded, deliberately obfuscated MIDI file. Two thirds decode; one third is
compressed or encrypted with no known algorithm. After duplicates and songs already held as MIDI, a
decoder adds roughly **2,130 distinct songs**, not the 12,489 the file count suggests. Cost 1.5–3
weeks with one unsolved blocker (tempo). Two cheaper paths exist and should be tried first.

| Marker | Meaning |
|---|---|
| **[bytes]** | Verified from the files. High confidence. |
| **[web]** | Public sources. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. Star 3 files, not Scream Tracker modules

`.st3` is also Scream Tracker 3; that ambiguity is settled. **[bytes]** No file carries `SCRM` at
0x2C. Whole corpus, 12,489 files:

| Magic | Files | Share |
|---|---|---|
| `STAR DATA V3.50` | 8,376 | 67.1% |
| `STAR DATA V3.00` | 4,096 | 32.8% |
| neither | 17 | 0.1% |

A 2,142-file folder sample split 71/28, close enough to be a property of the format rather than of
one directory. Sections 3 and 4 come from that sample; a decoder must be validated against all
12,489.

The trailing `0x1A` is a DOS EOF marker, so `type file.st3` prints only the magic line.
`Brasil/120_150_200Km.st3` is doubly corrupted (CRLF-injected and CP1252-transliterated) and
duplicates a clean file; everything else is intact.

## 2. The content is MIDI

**[bytes]** Byte-exact evidence: Soft Karaoke strings (`@KMIDI KARAOKE FILE`, `@L`/`@T` tags);
`FF 21 01 00` (SMF MIDI Port); `F0 41 10 42 12 40 00 7F 00 41 F7` (Roland GS Reset); canonical setup
runs (`B0 5B 00`, `C0 21`, `B0 07 64`, `B0 0A 40`); note runs across `0x90`-`0x9D` with channel 9 the
most frequent status byte. **No `MThd` or `MTrk`** — re-encoded, not wrapped. Median 34 KB, max
199 KB.

Header text names the source file converted from, e.g. `Kar\Brasil\Emilio_Santiago-Saigon.kar`. 212
V3.50 and 155 V3.00 headers name a `.kar`; 354 and 134 name a `.mid`.

**[web]** MultimediaWiki attributes the format to Creative Labs Korea, players "Star3 Karaoke" and
"Real Orchestra", late 1990s.

## 3. Layout (V3.50)

**[bytes]** except where noted.

```
0x0000  "STAR DATA V3.50" 0x1A
0x0010  0x11 (V3.50) / 0x0E (V3.00), 0x01, 0x00 0x00
0x0014  fixed-width, NUL-padded Latin-1/CP1252 text fields:
        title, artist, source .kar/.mid path, credits, e-mail/URL
0x0122  u32 (observed 0 or 5000)
0x0139  u16 = ticks per quarter note
        observed 120, 480, 96, 384, 192, 240 -- the standard SMF divisions
0x0165  16 x u32 little-endian  [inferred] per-channel event or note counts
~0x01A3 event stream (below)
 EOF-n  "LYRICS" section: u32 length, "LYRICS", padding, then the complete lyric
        text in Latin-1, with " > > > > " page markers
```

### Event grammar

**[bytes]**, roughly 80% decoded:

```
channel events:  <0x00> <MIDI status> <data bytes> <duration VLQ> <delta VLQ>
tagged records:  <tag> <len u16 LE> <payload>
                 tag = 0x40 | metatype   (0x41 -> FF 01 Text, 0x43 -> FF 03 Track name)
                 0x0D = SysEx, 0x0F = raw meta
```

**Notes carry a duration instead of a paired note-off** — the key difference from SMF.

A first-pass parser covers a median 78% and maximum 98% of the stream over 60 files. The residue is
unidentified tags in `0x01`-`0x14`. **[inferred]** Likely the display events (image change, line
change, prepare-next-line) a Brazilian tutorial describes.

## 4. The blocker: 28% is compressed or encrypted

**[bytes]** Measured on the 2,142-file sample.

| | Files | Body entropy | Deflate ratio | Verdict |
|---|---|---|---|---|
| V3.50 | 1,524 (71.4%) | ~5.5 bits/byte | 0.285 | plaintext, decodable |
| V3.00 | 596 (27.9%) | 7.89 bits/byte | 0.970 | compressed or encrypted |
| V3.50 outliers | 15 (0.7%) | >7.5 | — | as V3.00 |

V3.00 bodies are near-incompressible with no readable text, and no zlib, gzip or deflate stream was
found at any probed offset. **No lead on the algorithm.** **[web]** GNMIDI imports only uncompressed
files, so the boundary is a property of the format.

## 5. What exists publicly

**[web]** MultimediaWiki (origin only, no byte detail); **GNMIDI**, live commercial shareware that
imports `.st3` to MIDI, closed source, no spec; Brazilian players RealOrche and Microke, sites dead.

**Not found:** no byte-level specification anywhere, no entry on Wikipedia, Just Solve, TrID or
Kessler, and **no open-source parser or converter in any language**. Section 3 is our own analysis.

## 6. How it would fit

**Not a new `KaraokeFlavor` branch.** `Song::parse` opens with `Smf::parse(bytes)`, hard-wired to SMF,
so ST3 needs a separate decoder producing the same `Song` — a `km-song-st3` crate or `Song::from_st3`,
plus a `KaraokeFlavor::Star3` variant.

**No new audio path.** The output is MIDI channel events, so `km-audio`, `rustysynth`, the sequencer
and `km-suitability` need zero changes.

Three adaptations: emit `NoteOn` plus `NoteOff` at `tick + duration` and sort by tick; where only the
trailing `LYRICS` block exists it is untimed line text, which `LyricGranularity` and `LineInference`
already handle; encoding is Latin-1/CP1252, which `TextDecoder` already covers.

## 7. Effort and risks

| Item | Estimate | Confidence |
|---|---|---|
| V3.50 decoder plus corpus validation harness | 1.5–3 weeks | medium |
| V3.00 decompression or decryption | open-ended, may be impossible | — |

1. **~28.6% is undecodable** (596 V3.00 plus 15 outliers), no algorithm hint, no reference
   implementation.
2. **Tempo recovery is unsolved.** `FF 51 03` appears in **0 of 200** V3.50 bodies. Ticks-per-quarter
   is confirmed at 0x139, but where BPM lives is open, and without it absolute timing is wrong.
   **Solve this before writing a decoder.**
3. **~20% of the grammar is unmapped.** A mis-parse desynchronizes the whole stream.
4. **The format is deliberately obfuscated** — reverse engineering for interoperability is normally
   defensible, but deserves a conscious decision.
5. **It is a requirements change**, cascading into `KaraokeFlavor`, suitability and packaging.

## 8. The cheaper paths

These files are derived from `.kar`/`.mid`, and many headers name the original.

1. **Convert offline at packaging time** with GNMIDI, then feed the existing `km-song` path. Keeps the
   runtime MIDI-only. Coverage ~71%, the same a native decoder reaches.
2. **Recover the originals by name.** 554 distinct source filenames were extractable from headers and
   155 (28%) already exist as `.kar`/`.mid`; extraction was imperfect, so the true rate is higher.

## 9. How many songs this adds

**[bytes]** The deciding number. Names normalized (lowercased, spaces/underscores/dots/hyphens
stripped) against every distinct name in the `.kar`/`.mid` corpus:

| Measure | Count |
|---|---|
| `.st3` files | 12,489 |
| Distinct song names | **5,117** — 59% of files are duplicates across folders |
| Distinct names in the decodable V3.50 set | **3,731** |
| ...already present as `.kar`/`.mid` | **1,601 (43%)** |
| **...with no MIDI counterpart — the real gain** | **2,130** |

**[inferred]** Name matching is fuzzy both ways; treat 2,130 as ±20%, not precise.

The 43% overlap makes path 2 more attractive: nearly half the decodable catalog is playable today
with no new code.

## 10. Recommendation

Try the cheap paths first. If a decoder is built anyway: **scope it to V3.50**, solve tempo first,
treat V3.00 as out of scope, and validate against all 12,489 files rather than the 2,142 sample.
