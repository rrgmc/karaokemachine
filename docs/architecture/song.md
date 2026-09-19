# Songs — parsing, suitability, language

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## `km-song` — parsing and lyric normalization

The crate that determines whether the product feels right, and the one with the most real-world
messiness.

**The timeline is in ticks, not milliseconds.** Precomputed ms would break the moment the user changes
tempo mid-song. The sequencer publishes the current tick and the display interpolates the syllable
wipe from ticks, so highlighting stays exact under tempo changes and embedded tempo maps alike;
`duration_ms` is derived once from the tempo map, for display only.

**Two tempo events at one tick: the last wins**, and it is not an edge case — Soft Karaoke sequencers
routinely write a placeholder 120 BPM and immediately state the real tempo, both at tick 0.
`TempoMap::new` used `Vec::dedup_by_key`, which keeps the **first** of a run, so the placeholder
governed: one real file played its intro 1.766× too fast, 223.6 s against a correct 261.8 s. Changes
arrive track by track and the sort is stable, so a tie between *tracks* resolves to the higher index.

Karaoke-flavor detection, in order: **Soft Karaoke** (a magic Text meta event, lyrics as Text events
on a *Words* track); **standard MIDI karaoke** (`Lyric` events, with line breaks inferred where the
markers are absent); a **named-text-track fallback**; and **no lyrics**, where the song still plays
and the display shows title and artist only.

**Inference reads gaps and a width, and asks about punctuation only to find a word boundary.** A file
marking nothing is broken at a 1.2 second silence or at 40 characters, whichever comes first. A file
marking some of its lines keeps every marker it wrote, and a run past 100 characters between two of
them is cut at the widest pause inside it, never inside a word; where that run holds no pause, it
stays whole and the display shrinks the face. The reasoning and what set each number is
[`A file is trusted for the lines it marks`](../decisions/songs.md#a-file-is-trusted-for-the-lines-it-marks-and-not-for-the-verse-it-says-nothing-about).

**Four corrections found against real files**, none of which the format documents:

- **The header is *not* on the track carrying the magic string.** Real files put it alone on a track
  named `Soft karaoke` and the rest of the header at the top of the *Words* track. So every track is
  scanned for control lines.
- **A file can carry two timings of the same song, and the longer one is not the one to sing.**
  `pick_soft_karaoke_lyric_track` ranks candidates on `(named Words, event count)`, the name first.
  Of 1,198 corpus files 36 hold two or more lyric tracks and 35 of those name one `Words`; ranking on
  the count alone takes the wrong track in three of them, one landing 15% of its syllables on the
  melody where the `Words` track lands 100%. PyKaraoke sorts on the same pair, and picking the
  longest track alone is what most readers do.
- **The second title line is frequently a transcription credit** rather than a performer, so
  credit-shaped values go to `info` and the search for an artist continues.
- **Only the first two tracks are eligible as a title fallback**, because later track names are
  instrument names — and an instrument name as a song title is worse than no title.

Both marker conventions occur, leading and trailing, so both ends of a payload are inspected.

**A fourth thing real files carry is the underscore**, and `km_song::spacing` reads it in
`collect_raws` once the break markers are off. It is markup about a space with two opposite meanings
separated by position — an elided space between two words on one note, a cancelled space where a
writer who ends every syllable with one needs to say *not this one*. The decision is
[`An underscore in the words is markup about a space`](../decisions/songs.md#an-underscore-in-the-words-is-markup-about-a-space);
`spacing::marks` is the same test with no rewrite behind it, and counting it in `km-lyrics scan` is
what holds the rule against a corpus rather than against fixtures. **Only the full corpus separates
the two conventions by weight**: a folder sample puts the cancelled space at one file in 500, where
every parsed file puts it at 9,783 marks against 30,186 elided — nearly a quarter, and far too many to
answer with the commoner reading. `space marks left` reads nothing once the rule runs, which is what
that line is for: 2,486 files before, none after, over the same corpus.

**The same pair of runs prices the one side effect.** Dropping a syllable that was nothing but a mark
moves four counters and no more — one file to no lyrics at all, two from syllable-level to
line-level, three suitabilities by a point — which is the check that a rule touching syllable text
did not quietly move suitabilities on a corpus.

**A file may mark none of it: a space after every syllable and no underscores.**
That claims each fragment is a whole word and puts six words on the screen where the line has two.
`build_timeline` narrows those spaces to `SYLLABLE_DIVIDER` and the file loses a point of its
suitability, under
[`A file that marks no word ends draws a narrower space`](../decisions/songs.md#a-file-that-marks-no-word-ends-draws-a-narrower-space).
**The cancelled space above is what such a file is missing**, which is why the two rules meet in the
right order: `spacing` resolves the marks in `collect_raws`, so a file that cancels the spaces it
does not mean arrives with the word-final ones already distinguished and is left alone here. What
reaches this rule marks no word, and two measurements keep it narrow — a stream with a single
leading space is untouched, and fragment length separates a file whose syllables are spaced from
one whose whole words are. The mean decides below 3.0 characters; up to 3.65, where English syllable
files and short-word files overlap, the share of fragments of seven characters or more decides as
well. **Break markers are not among them**: a file can place every line with
`\r` and still space every syllable, and only fragments with text are counted, so a marker standing
alone does not dilute the share.

**A file may mark none of it the other way round: no space anywhere.** Sheet music read by an optical
scanner writes one event per note and no separator at all, so a whole verse joins into one run of
letters — the claim above turned around, and answered with the same divider and the same point off,
under
[`A file that marks no word boundary at all draws the same divider`](../decisions/songs.md#a-file-that-marks-no-word-boundary-at-all-draws-the-same-divider).
`LyricTimeline::word_ends` carries which of the two a file was, so a sweep can say how many files each
rule claims without hunting for the divider in text the rule has already rewritten. The guard that
keeps this one narrow is a script: Han, kana, Hangul, Thai, Lao, Khmer and Myanmar write without
spaces on purpose, and one character of any of them leaves the file joined.

**Some files write in a notation of their own, and `collect_raws` measures before it reads.** Two
habits are looked for over the events one flavor accepts, before any of them becomes a syllable: a
file that opens every line with `<`, and a file that prefixes a quarter or more of its events with
`%`. The first makes `<` a leading line mark, the second drops the marked events — chord symbols, in
every file sampled — and both are answered per file rather than per event for the reason the space
rule above is: punctuation is markup only where a writer used it that way throughout. **The mark is
recognised rather than the notation after it**, because the notation varies where the structure does
not: one file spells its chords `%SOL` `%FA7+` and another `%Bm7` `%A7+/D`. The second rule is read
only in a file that also marks its lines, which is the second signal that keeps `100% PURE LOVE` a
lyric. The decision is
[`A chord is not a word, and a bracket that opens every line is a mark`](../decisions/songs.md#a-chord-is-not-a-word-and-a-bracket-that-opens-every-line-is-a-mark).

**Where it sits is what makes it safe.** The measurement reads past the ordinary break marks with the
dialect switched off, so `/%LA-` is recognized as the chord it is while `<` is left in place to be
counted — it is the thing being decided. The drop then happens after `spacing::resolve` and
`clean_lyric_text`, on the final text, and hands back the break the dropped event was carrying.
`Dialect` travels on `Song` beside `flavor` and the decoder, so what was decided about a file is
answered rather than worked out again; `km-lyrics scan` counts both, which is how the shares were
set.

**Text encoding matters more than it looks.** Real `.kar` files are frequently Shift-JIS, CP949,
CP874, GB18030 or CP1252, **never tagged**. Resolution order: the manifest's `lyric_encoding` wins,
then valid UTF-8, then detection, then CP1252. Correct *decoding* is cheap and worth doing even though
*rendering* non-Latin scripts is deferred — it keeps the stored data right for later.

### The preview, and five functions that must stay five

`looks_like_a_banner` skips the sequencer's advertisement so a package carries the first lines
somebody would actually sing.

**It and `is_credit_line` in `km-suitability` share two rules and must stay separate.** Merging them
looks like an obvious deduplication and is the trap: `is_credit_line` decides whether a line counts
towards *how much lyric a file has*, which is an input to the stored 0–10 suitability — **so a rule
added there moves suitabilities in packages that already exist.** Both doc comments say so, pointing
at each other.

**`is_only_a_legal_notice` is a fourth, and the widest consequence of the four carries the tightest
test.** It shares the banner rule's `LEGAL` marks and asks something stricter of them: not whether the
line *contains* one, but whether every word of the line is a word a notice is made of. A line that
passes is dropped from the timeline in `build_timeline`, so it reaches nothing — where a banner is
only kept out of a preview. `Copyright 1994 Some Publisher` keeps a publisher and is therefore a
banner and not this. Dropping a line takes its syllables, so the affected files lose lyric coverage
and their stored suitabilities move; that is the correction, since they were counted for words nobody
sings.

**`redact::contact_spans` is the third, and it answers a different question again**: not "is this
line a credit" but "*which characters* are contact details", so the rest of a line can survive. It
returns byte ranges rather than a verdict and is the only one of the three that rewrites anything —
`build_timeline` applies it to the syllables as they are built, which is why the address is gone from
the screen, the wipe, the `lyric_line` event, the preview, the catalog, the book and the search
index in one edit. The decision is
[`A stranger's contact details are taken out of the words`](../decisions/songs.md#a-strangers-contact-details-are-taken-out-of-the-words).

**`names_something` is the fifth, and it is the only one asked about a *name* rather than a line.**
It decides whether a title or an artist says anything — two letters or digits in it, or one character
of a script that writes a word in one — where the other four judge a line of words. Its consequence is one row's title, so it can be blunt where
`is_credit_line` cannot; and it shares none of the phrase lists, because a song may perfectly well be
called `Karaoke` while no lyric line is. `clean_meta_name` pairs it with the character cleaning, and
every kind of song passes through that one gate — a MIDI title, a container tag and an ID3 frame
alike. The decision is
[`A name made of marks is not a name`](../decisions/songs.md#a-name-made-of-marks-is-not-a-name).

**The syllable-divider narrowing is the second rewrite in the same function, and it runs first**, so
the spans are computed over the text the screen will draw. It rewrites a syllable's *body* and never
`text()`, which keeps that a join with no separator between syllables — the property the byte-offset
walk here and the API's `line.text` contract both stand on. `is_credit_line` reads any whitespace as
the end of an address for the same reason: the same line spaced two ways must reach one verdict.

**It reaches the other two from behind, and that is the sharp edge.** Once an address is a dash,
both stop recognizing the line they were classifying. `is_credit_line` would let every business card
count as lyric, moving stored suitabilities — exactly the trap the paragraph above exists to prevent.
And `looks_like_a_banner` would let one *into the preview*: `0**17 3463-1150` is a telephone by the
digit ratio, while `0**17 —` is three digits and reads as words, so the card the skip exists to hide
becomes the thing it stores. `LyricLine::contact_redacted` closes both — each reads the flag before
the text, so the old verdict survives without a rule being added anywhere.

**Folding does the bracket-stripping for free**: mapping punctuation to spaces makes `(Intro)`,
`{Words}` and `[ VOCALS ]` all arrive at the label check as a bare word, and that check is exact-match
— a substring rule there would eat `Words of love`.

**The measurement is `km-lyrics preview`**, and its useful half is the table headed *first kept line,
where more than one file shares it*. Real lyrics differ per song, so a first line hundreds of files
share is boilerplate that got through. That list is what most of the rules were written from — **and
it killed one**: skipping a line matching the song's title, which has a test named after it so the
idea does not come back on the same plausible reasoning.

**Three things only the full run could show**, and they are the argument for running it rather than a
sample. The phrase list was English and looked for a credit after *by*; half this corpus is Portuguese
and Spanish and puts it after *por*. A `exclusive by: tomson` shape matched nothing because the verb
in front is not enumerable — **the colon is the signal**. And the general lesson: **a banner is
usually a block, so removing its first line reveals its second.** Real lyrics begin at rank four of
the survivors table, which is the stopping condition — above them sit a person's name, a studio name,
mojibake and a version banner, none of which has a shape a rule could key on without eating song
titles.

## What the real corpus looks like

Measured over 44,355 `.kar` files. These are the baseline to compare against after any parser change.

| Measure | Result |
|---|---|
| Parsed / failed / panicked | 99.81% / 0.19% / **0%** |
| Soft Karaoke / named text track / `Lyric` events / none | 84.75% / 10.78% / 3.19% / 1.28% |
| Syllable-level timing | 98.57% |
| Encoding: ASCII/UTF-8 / CP1252 fallback / positively detected | 59.25% / 37.64% / 3.11% |
| Title / artist recovered | 88.12% / 67.73% |

Conclusions that shaped the code:

- **Every one of the 83 failures is a file that is not MIDI at all** — random bytes under a `.KAR`
  extension. Rejecting them is the correct outcome, and the corpus confirms nothing makes the parser
  panic.
- **The named-text-track fallback is not an edge case at 10.78%**; dropping it would lose ~4,800
  songs.
- **Detection alone would mislabel a lot**: 37.64% land on the CP1252 fallback, which is right for
  Western European text but is a guess — exactly why the manifest overrides it, since packaging is
  where a human can correct it once per song rather than every play.

**The `.mid` files matter too.** Over every one of them: 99.49% parsed, again zero panics, and 87.21%
have no lyrics as expected — **but 12.78% do, which is a large further set of karaoke songs outside
the `.kar` set.** Packaging must therefore treat `.mid` as a source. Of the failures, 22 show `0d 0a`
where `0d` belongs, the signature of a file transferred in FTP ASCII mode; at 0.026% repairing that is
not worth the code, **but it is worth knowing the cause rather than filing it under "mystery"**.

## `km-suitability` — melody detection and the rubric

Runs **only at packaging time**, never during playback. Results are written into the manifest, so the
machine reads a fact instead of guessing and a packager can override it. This is the direct answer to
"do not guess anything every time".

**Detection feeds the guide-melody toggle and not the suitability.** What the rubric asks of a
file's channels is whether the backing is spread across them or piled onto one, which is a property
of the file; whether *this* detector could name the tune is a property of the detector. So an
abstention costs nothing and is reported, and the two points go to the file that has its parts
apart.

**Melody-channel detection is confident or nothing.** Each non-drum channel is scored on independent
signals — a name matching a melody vocabulary, lyric alignment, monophony, vocal pitch range, and the
common channel-4 convention as a tiebreaker only. **The gates are applied in order and each failure
has its own reported reason**, so an abstention is diagnostic rather than a shrug. A channel is
claimed only if it clears every gate **and** beats the runner-up by at least 1.5×; otherwise the
toggle is hidden rather than offering a wrong one.

**Vocal range is a gate, not a bonus — corrected against a real file.** A `.kar` whose guide track is
plainly named was being called ambiguous, because **its bass line is also monophonic, and a bass
follows the rhythm so it lands on nearly every syllable too.** Lyric alignment therefore cannot
separate a bass from a tune, and with range worth a mere bonus the two stayed inside the margin.
Register separates them decisively — nobody sings a line two octaves below the voice. It is measured
as a *fraction* of notes in range rather than a min/max test, so one outlier cannot rule out a real
melody.

**Presence under the words is the gate after range, and a track name cannot pass it.** A channel in
range must have an onset within `melody_presence_window_ms` (1 s) of `melody_min_lyric_presence`
(half) of the syllables, or detection abstains as `SilentUnderTheWords`. This is what stops a riff
named `Melody` from being claimed and then judging the words it rests under. The window is far wider
than either alignment window because it asks whether the channel is there at all, so words timed a
beat off a real melody still pass. `rank` applies the same test to `eligible`. The rule and its
measurement are the decision
[`A channel named for the melody must play while the words are sung`](../decisions/songs.md#a-channel-named-for-the-melody-must-play-while-the-words-are-sung).

All thresholds live in one module so they can be revised from evidence, and `km-pack reanalyze`
recomputes a package after a revision.

### How much lyric there is — added after a 10/10 with no song in it

A file scored **10/10** with three lines of lyrics that were the arranger's name, two telephone
numbers and an email address. Every other measurement was right: syllable-timed, 91% aligned, melody
found, nine channels, three minutes long. **The rubric simply never asked *how much* there was to
sing**, so eleven syllables of business card scored exactly as well as four hundred syllables of song
— and on a corpus of hundreds of thousands of files, that sorts the useless ones to the top.

Two measures, and a file has to pass both: **syllables**, with obvious credit lines discounted first
and deliberately narrowly (a *name* is not discounted, because a name can be a lyric and quantity
settles those files anyway); and **coverage**, the span from first counted syllable to last over the
song's length, which is by far the stronger signal and the one that cannot be faked by a wordy credit
block.

Below either threshold both the lyrics and sync components go to **0**. Sync goes to zero because
"every syllable lands near a note" is not a measurement when there are eleven syllables and 4,489
notes to land near.

**Two failures, two names, because they read differently to a person.** One is the business card. The
other is a *real song* whose words are timed for the first verse and then stop. Both are unsingable
and both score the same — but telling somebody "there is nothing to sing along to" about a file with a
verse plainly in it is how a warning stops being believed.

**The thresholds come from the corpus, not from taste.** Over 198 files: credit blocks carried 1–49
syllables covering 0–8%; the songs carried 144 or more covering 56–90%. **There is nothing in
between**, which is why the thresholds are safe. Re-scoring moved 23 files and **not one of the 177
real songs changed**. A revision should be made against a fresh measurement rather than by argument.

### A chord chart in the lyric track

**Some files put chord names where the words go**: keyboard style demos and guitar instrumentals,
one chord to the bar. Quantity cannot see them. The text runs the length of the song, and it lands
on every chord change, so sync scores it at the top. It is marked `chord_names_only`, a hard defect,
and scores zero for lyrics and sync.

`is_chord_symbol` accepts a line only when every word is part of a chord: a root `A` to `G` with `#`
or `b`, then quality pieces (`maj`, `min`, `m`, `dim`, `aug`, `sus`, `add`, `th`, digits, `+`, `-`)
and a slash bass, written joined (`F#m7`) or spread over words (`A  min 7th    /G`). Roots are
uppercase, so `a` and `be` never qualify.

**`LyricContent::is_chord_chart` judges the file.** At least 8 lines, and at least 80% of them, must
be chord names. The share comes from a probe over the whole corpus with the gate lowered to one line
at 10%:

| Share of lines | What the files were |
|---|---|
| 0.81 to 1.00 | chord charts, every one; the lines that failed were `H7+`, `NC`, `same` and `CHOR BEGIN` |
| 0.75 | a chord chart whose title line runs into its words, and the one chart the gate misses |
| 0.49 and below | real songs: `Ave Maria` sung as `A` `ve`, the alphabet song's `A B C D`, lyric sheets with a chord above each line |

Nothing fell between 0.49 and 0.75, which is why the gate is safe.

### Which notes the sync question is asked about

**A full arrangement answers yes to everything.** Sync counts syllables landing within 120 ms of a
note onset, and across every non-drum channel of a real backing there is an onset almost everywhere:
one file drawing words that run eleven seconds early by the last chorus put **94%** of them inside
the window and took all three points. An eight-channel arrangement with 3,651 notes has no gaps for a
badly timed syllable to fall into.

**The melody's channel is the one that separates them.** The same words score **36%** against the
channel carrying the tune, and the timing the file actually means scores **100%**. `sung_line` picks
that channel out of the measured stats, and only when `MelodySignal::LyricAlignment` is absent from
the signals that chose it — a channel found *by* the lyrics cannot then be asked about them. Every
other case keeps the measurement across all non-drum channels, so an abstention still costs nothing.

**`sync_window_ms` stays at 120 and `note_align_window_ms` stays at 60.** They answer different
questions — *is this note being sung* against *were these words timed to this music* — and narrowing
the channels is not a reason to tighten the window.

### What detection and scoring do on real files

**Melody found on 65.89%.** The remaining third abstains, and the reasons are the useful part: no
supporting evidence 14.04%, outside singing range 9.30%, ambiguous 9.17%, nothing monophonic 1.49%.
**A ~66% rate is the shape to expect** — a guide-melody track is a common convention but far from
universal, and the ~9% ambiguity is honest abstention rather than a coin flip.

**Suitability: mean 9.04/10, and this is the component with a known weakness.** Hard defects are
caught, but 86% score 8 or above, so **suitability sorts *broken from working* far better than it
sorts *good from excellent*.** That is largely inherent: these files were made for karaoke, so they
satisfy the four criteria the rubric was asked to measure. Improving discrimination means measuring
something the rubric cannot see — narrowing sync to the melody is one such measurement, and these
figures were taken before it, so a fresh sweep is what they should be compared against.

### Which faults mean there is nothing on screen to follow

`Suitability::words_cannot_be_followed` is a second question over the same warnings, asked about the
*display* rather than about the file: it decides whether a build tells the machine to draw the words
at all. Three answer yes — `lyrics_all_at_zero`, `negligible_lyrics` and `chord_names_only` — and in
each the text on screen either never moves or is not words, so withholding it takes nothing away.

It is narrower than `has_hard_defect` on purpose. `partial_lyrics` is a hard defect and is left out,
because those are the song's own words timed for as long as they last; `no_lyrics` is left out
because such a file draws none already, and reporting it would claim a decision nobody took.

**No new warning, no new threshold, no new field.** The judgement is a function over warnings a scan
already produced, which is what keeps `ANALYSIS_REVISION` still and means nothing has to be rescanned
for it — and `km-lyrics scan` already tallies warning codes, so how much of a corpus it reaches is
measurable with the tool as it stands.

## `km-fixes` — corrections for what a file gets wrong about itself

One module per defect, holding its detector, what the correction does, and the shape it addresses.
That co-location is the whole reason the crate exists in one piece rather than as a detector beside
the scorer and an effect beside the sequencer: the question anybody asks of it is *what does this
machine repair*, and the answer should be a directory listing.

**Two readers that share no other ancestor**, which is what earns a crate here: packaging detects
into a manifest, playback resolves into a channel table. Putting the list in `km-audio` would make
`km-pack` link a synthesizer to name a defect; putting it in `km-kmpkg` would put the sequencer's own
filter inside the container format.

**The wire is open downward and closed upward.** `Fix` is a closed set of shapes this build can act
on, and an unreadable one is held as the JSON it arrived in rather than as a variant remembering only
that there was something. `SongKind::Unknown` and `EditedField::Unknown` are unit variants because
what they stand in for is a scalar and a name is all there is to lose; a fix carries arguments, and a
curator's hand-set list is copied whole by `inherit_edits_from` — so a forgetful catch-all would turn
somebody's channel mute into `{"fix":"unknown"}` the first time an older build rewrote the manifest it
sat in. Key order is not preserved, which nothing reads by byte.

**`automatic` is what a package records, `suggested` is what a person is offered, and `detect` is
the two together.** The split is the applies-itself rule: a stored list means *the corrections in
force*, which playback resolves and applies entire, so a fix that must be offered cannot be in it
until somebody has agreed. Filtering at playback instead would overrule the curator the offering
exists to serve. Each runs only its own detectors, because a package build and a song load call
`automatic` once per song and a check whose answers it would discard is wasted work.

**`ChannelFixes` is flat, `Copy` and heap-free** — five arrays of sixteen, one per kind of effect.
A new kind of fix adds one array and one arm in `resolve`, and the sequencer never learns the
vocabulary. It is shaped that way because it is read inside the audio callback; see
[`audio.md`](audio.md) for what happens to it there.

### The bank select that turns an organ into a drum kit

A setup track sends Bank Select MSB 126 or 127 — XG's effect and drum banks — on a channel the same
file then gives a melodic program and sustained chords. **What that sounds like depends entirely on
the bank, which is why nothing measured at packaging time can hear it.** A SoundFont with no bank 127
falls back to bank 0 and plays some instrument, wrong but harmless; one that has a bank 127 succeeds,
and every chord note lands on whatever percussion sits at that key, at the velocity the chord was
written with.

Both halves of the message are dropped, coarse and fine: a select passed on with its partner
suppressed still names a bank. A bank below 126 is left alone — a file asking for bank 8 is asking
for a different piano, and on a font without one the fallback is already right. The drum channel is
left alone, being the one place a kit bank is what the file means. **A channel that sounds no note is
left alone too**, so the manifest and the log carry a line only where something can be heard.

### The bend a return left short of centre

A part bends a note or a chord and ramps back towards centre in steps, and the last step is missing.
Nothing else moves the bend, so the channel plays the following bars a fraction of a semitone out
against the parts doubling it. **Every bank renders it the same way**, and so does the module the
file was written for, so this is a defect in the file rather than in a synthesizer.

**One rule is shared by the detector and the sequencer**: a note is *stranded* when it starts a beat
or more after the channel's last bend event while that bend is off centre. `is_stranded` states it
and `stranded_from` gives the same answer as one tick, which is the form the sequencer keeps.

**The detector proposes only the clear cases**, by adding three conditions to the rule:

- the offset is at least 20 cents, at the channel's own bend range read from RPN 0;
- the bend ended a return: its last step moved towards centre, and it sits at no more than half of
  the furthest point of the movement, a movement being bend events each within a beat of the one
  before;
- the channel starts at least two notes on it, a chord counting once.

The return condition is what separates the defect from deliberate shapes. A bend that climbs to full
deflection and is held, a vibrato written around a held bend, and a channel detuned once before its
first note all fail it. The drum channel is left alone, and Reset All Controllers returns the bend to
centre as it does on a synthesizer.

`fix_census` sweeps both thresholds and names flagged files, and `stranded` lists every stranded note
start with its bend and offset, so a flagged file can be checked by listening.

### The two a person decides

**A channel mute and a forced instrument have no detector**, which is the whole of what their modules
hold: a `describe` for the log, and the reasoning for why a rule that proposed either would be wrong
about somebody's song. A part that spoils a song can be deleted or re-voiced, and which of those is
wanted is a judgement about an arrangement.

**A forced instrument is the one fix carrying an argument**, and that reaches three places. The wire
spells it `{"fix":"force_program","channel":4,"program":52}`; a program above 127 is refused on
reading, being a shape this build knows with a value no program change could carry; and the curation
tool's control is a select rather than a checkbox, because on-or-off cannot say which of a hundred and
twenty-eight. The drum channel is offered none, a program change there naming a kit.

**`melody::rank` is `detect` without the choosing**, and the curation tool's channel table is what it
is for. Detection answers one channel or none, which is what a machine needs; a person auditing a song
that abstained is asking which channel came closest, so `rank` reports the field with the gates
labeled rather than applied. Its `evidence` is the half of `MelodyChannel::confidence` that belongs to
a channel rather than to the song, which is what lets every channel carry one.

The General MIDI names live in `km-fixes` for the reason `DRUM_CHANNEL` does: they are a fact about
General MIDI, and a crate whose dependencies are `km-song` and `serde` is where a fact of that kind
costs nothing to reach.

## Language, as a code

`Language` is the whole ISO 639-1 list compiled in, plus `und` and `zxx`.

**The type is closed, the wire is not.** The manifest field stays a `String`: a package written before
this carries a raw `ENGL`, and one written by a later build may carry a code this one has never heard
of — **both must open**, showing the value as it stands, rather than failing. Everything that *writes*
one goes through the type, so a package built by this version only ever holds a real code.

> **Hazard — do not add a language rule to `Manifest::problems`.** It is the obvious place and it is
> wrong. `Package::open` calls it as well as the writer does, so anything it refuses is a package that
> cannot be *opened* — and every package built before the change would qualify. **The catalog of
> every machine in service would empty on the next start.** The requirement is a curation rule and
> lives in the two packagers. There is a regression test and a note above the function saying so.

**Two sources, strongest first, resolved in one place** so the two packagers cannot disagree about
precedence: the lyric encoding wins where it speaks, the Soft Karaoke header is the fallback. **Only
unambiguous encodings map** — an entry naming a *region* rather than a language is absent, since it is
one language or another depending on who wrote the file.

**A third source reads the words, and it is a crate of its own.** `km-langguess` takes the lyrics,
falling back to the title, and answers with a `Language` and a confidence; `km_kmpkg::Language::detect`
knows nothing about it, and the ordering between the three is `eff_language()` in the curation tool.
The crate exists rather than a module or a feature because the detector compiles a trigram model of
every language it knows into the binary, `km-kmpkg` is linked by the machine, both remotes and every
application shell, and only the packaging tools ask the question — a crate nothing else names is a
crate nothing else carries.

Its answer maps ISO 639-3 onto ISO 639-1, which is transcription; a test walks every language the
detector can return and fails on one nothing maps.

**Folding strips accents as well as case**, which is why every lookup table can stay ASCII. That is
not fussiness: a non-ASCII key is a key that a careless `perl -i` silently replaces with `U+FFFD`,
which is exactly how the function came to be written. 194 corpus files declare `Français`.

**In the curation tool the code is a *second* detected column, not a rewrite of the first.** Two
alternatives are refused. Rewriting in place breaks the schema's own contract —
`det_*` is *what the file said* — and destroys the mapping's input, so a table change could never be
re-applied. A SQLite user-defined function avoids the column but puts an expression index over a UDF
in front of the planner, and a function is code the database file cannot carry to whoever opens it in
`sqlite3`.

**What the corpus actually looks like**, worth knowing before anybody is surprised by the packaging
gate: only **5% of songs carry a language header at all** — the format is Soft Karaoke's
and most of the corpus is plain `.mid` — and being UTF-8 implies nothing. So **8% are classified
and 92% are not.** That is why the bulk set exists and why it shipped before the gate.
