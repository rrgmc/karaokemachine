# Songs and suitability

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Suitability

**0–10, computed at packaging time**, stored in the package with a per-component breakdown and
warnings so a low one is explainable. Four components: lyrics 0–3, sync 0–3, channels 0–2,
arrangement 0–2.

**A component has to measure the file and not the tool.** Whether a melody channel could be picked
out with confidence is a fact about how a file was named and voiced and about what this build's
detector can see — a good arrangement whose guide track is called `Track 3` sings exactly as well as
the same arrangement with the track called `MELODY`. So *no melody channel* is a **warning and not a
deduction**: it says what the machine will not be able to offer, which is worth knowing before a
package is built, and it takes nothing off the number.

**Every instrument on one channel is the channel fault that does count.** Two parts sharing a
channel share a patch, a volume and a transposition, so nothing downstream can act on either
separately — which is what the guide-melody toggle and the key change both need. It is worth the
full two points, because no other measurement can make up for it.

**A revision leaves stored numbers stale, and correcting them is asked for rather than automatic.**
`km-pack reanalyze` recomputes a package; *Recalculate suitability* on the Songs page and
*Re-analyze everything* on the Scan page do it for a corpus.

## Suitability is not called a score

**The 0–10 number a file gets is `suitability` everywhere — in Rust, in SQL, in the package manifest
and on the wire. Nothing calls it `score`.** This project's first non-goal is *no scoring of singers*,
and a quantity called "the score" sitting in the middle of it reads as the thing the product refuses
to do.

**The hand-set column keeps its name**: `user_score` really is a user's score. It reads less
ambiguously when nothing beside it is called `score` with nothing qualifying it.

**Verbs stay.** A file *scores* 8 out of 10; the noun is its suitability. Nobody reads "this file
scored 8" as a singer being judged.

## No compatibility aliases

**A rename carries no alias and no shim.** The one published package is rebuilt from a description in
this tree, and nothing outside it queries the API, so a shim would keep a second spelling alive for
nobody — and it is not free: it is a second spelling that new writing drifts back onto. A shim that
still works is a shim a demonstration script goes on teaching.

**Two exceptions.** A rename inside a store holding somebody's work is a numbered migration step
rather than an alias, by
[`A store opens at its current version or is refused`](foundations.md#a-store-opens-at-its-current-version-or-is-refused).
And `user_score` keeps its name, because it really is somebody's score.

**How a removal fails, which differs by kind.** An unknown
*query field* is ignored, because `SearchParams` sets no `deny_unknown_fields`, so a stale filter
silently does not apply. An unknown *enum variant* is refused, so a stale sort key is a 400. And a
`.kmpkg` written before a rename **fails to open**: `value` has no serde default, deliberately,
because a default of 0 would let such a package through reading as the worst possible file and sort it
to the bottom of every list without a word. That is the only one with a real artifact behind it, and
its remedy is one `km-pack build` over the description the package came from.

## Non-Latin text

**The television draws CJK. Thai, Arabic, Indic and right-to-left stay deferred.** The line between
them is not how foreign a script looks, it is whether drawing it needs a *shaper*: Han, Kana and
Hangul are one codepoint to one glyph, with no reordering, no cursive joining and no mark
positioning, so SDL3_ttf renders them with HarfBuzz off exactly as it renders Latin. The other three
do not, and `no-sdlttf-harfbuzz` is a build decision with a linker failure of its own behind it, so
they wait. Lyric *decoding* handles the legacy encodings either way.

**Glyphs come from a fallback chain, and the chain is borrowed rather than shipped.** No font is
bundled for this: Windows, macOS and Android carry CJK faces already, a Linux desktop has them
wherever its distribution's Noto CJK package is installed, `display.font_cjk` names one where a
system hides it, and adding a 20 MB font to every carrier for 0.3% of a corpus is not a trade worth
making. See
[`A CJK face is borrowed, never bundled, and opened only when asked for`](distribution.md#a-cjk-face-is-borrowed-never-bundled-and-opened-only-when-asked-for).

**A chain rather than a font, because coverage is per-font and not per-script.** MS Gothic has no
Hangul, so Korean stays a row of boxes behind it however good the Japanese looks. Two faces are
opened at most, which is the cap that keeps the memory cost bounded on Android.

**So the list says what each file answers for, and a slot is never spent twice on one script.** That
is the cap and the per-font coverage taken together: two files chosen by position give a stock Windows
box two Japanese faces and a Mac Japanese and Korean, and in both cases the machine reports a CJK font
loaded while a lyric line draws boxes. Which script comes *first* is still the corpus — Japanese, then
Chinese. See
[`A CJK face is borrowed, never bundled, and opened only when asked for`](distribution.md#a-cjk-face-is-borrowed-never-bundled-and-opened-only-when-asked-for).

**Nothing is opened until a song asks.** A machine whose catalog is Portuguese never loads a Japanese
font, because the cost is real — a face is bound to its point size, so each fallback file is six more
`TTF_Font`s. The text cache notices the first CJK string, being the one place every drawn string
passes through, and the display rebuilds its fonts the way a resize already makes it. That rebuild is
required rather than tidy: **SDL3_ttf caches rasterized glyphs per face**, so attaching a fallback to
a face that has already drawn a box returns success and goes on drawing the box.

**What this does not do.** The song book still prints `?` for a Japanese title and counts it — that is
`The song book` in [`interface.md`](interface.md#the-song-book), and a PDF needs an embedded CID font
rather than a fallback chain. Search and the A–Z strip are unchanged: SQLite's `unicode61` tokenizer
cannot segment a language with no spaces between its words, so a Japanese title is found by its number
and by browsing, not by typing part of it. And the interface itself has no CJK locale — `is_drawable`
still holds every shipped catalog to Latin-1, deliberately.

**The corpus is why this is small.** A scan of every `.kar` file in it found 189 in Shift-JIS and 28 in
Big5, and no EUC-KR or GB18030 at all: 0.3%, against 2.7% in the Central European encodings that
[`One alphabet, everywhere`](#one-alphabet-everywhere) answers. That ratio is the argument for
borrowing a font rather than bundling one, and for opening it late rather than at boot.

## One alphabet, everywhere

**Every list of songs in this product is ordered by `km_song::text::fold` — accents stripped, case
folded, punctuation collapsed — and there is exactly one implementation of it.** The machine's
screens, both remotes, the printed book and the package builder's browse list all sort by what that
function returns, so `Águas de Março` files under `A` on every surface.

SQLite's default collation sorts every accented character *after* `Z`, and its `NOCASE` collation
is ASCII-only and does the same — which on a corpus that is largely Portuguese is a hundred songs past
the end of the alphabet. The fault is worse when it is *partial*: one list folding its A–Z bar and not
its sort, another folding titles and not artists.
**One list running two alphabets is harder to notice, and harder to trust, than one running the wrong
alphabet consistently.**

**The fold is stored, not computed at query time.** No SQL spelling of it exists that is not a second
copy of its accent table, and two accent tables free to disagree produce a fault that reads as bad
data rather than as a bug — a song found by the search box failing to appear where the same song
sorts. So each catalog carries folded columns beside the names, written in Rust when a name is
written. The cost is a column per name per database and a one-time pass over what is installed.

**Which makes changing the table a migration, and it is one the catalog performs on itself.** A
`fold_version` in the catalog's `meta` table records which spelling wrote its keys; a machine that
opens a catalog folded by an older one refolds it there and then, rather than waiting for every
package to happen to be reinstalled. It also bumps `catalog_version`, because the offline remote
copies `sort_key` and orders by it — a refold nobody was told about would leave every mirror in the
house sorting by the previous alphabet with nothing to notice.

**Leading punctuation does not put a song in the `#` bucket.** Folding strips it, so `¿Y ahora qué?`
files under `Y`. A Spanish song is not a symbol.

**This is about *ordering*, and it does not give the A–Z strip to an online remote.** That needs the
folded *initial* as an indexed column — a different column and a different decision, in
`The two remotes` in [`remotes.md`](remotes.md).

**The whole Latin range, and the table is SQLite's rather than anybody's opinion.** Every character
in Latin-1 Supplement, Latin Extended-A and Latin Extended-B was put through
`unicode61 remove_diacritics 2` and the answers transcribed, so `fold` cannot drift from the index it
has to agree with; `fold_and_the_search_index_agree` walks the same range and fails if it ever does.
That is a stronger guarantee than a Unicode normalization dependency would give, and a smaller one:
the question is never "what is the right fold" but "what does the index already do".

**Czech, Polish, Hungarian and the Baltic languages file under their own letters.** The fonts are
not the obstacle: measured 2026-09-08, Segoe UI draws `Příliš žluťoučký kůň Łódź Tükörfúrógép` with
every glyph present and no fallback consulted, and DejaVu Sans — the tarball's font — covers the
same range. A scan of every `.kar` file in the corpus puts 2.7% of them in these encodings against
0.3% in CJK, so this is also where the songs are.

**A stroke is not an accent, and neither is a ligature.** `remove_diacritics` takes combining marks
off and leaves `ł ø æ đ ß ı` alone, because they are letters rather than decorated ones — so this
leaves them alone too, and a Polish title files under `Ł`. Tidying that away would look like finishing
the table and would put the browse strip and the search box on two alphabets.

**None of this reopens `Non-Latin text`.** These are Latin scripts, drawn by fonts the product
already reaches. Greek, Cyrillic and CJK still sort by whatever their codepoints do, and still will
until something renders them.

## Within one title, by performer

**A list ordered by title orders the songs that share one by performer.** A corpus holds five songs
called *Goodbye* and a catalog holds three, so what a title order decides most often is not which
title comes next but which of a title's recordings does. The keys underneath — a content hash in the
curation tool, a song number in both catalogs — are answers to *which file* and *which slot*, and
neither is an order a person can read down a column. Interleaving one performer's rows with another's
is the visible cost of ordering by one.

**A song nobody named a performer for comes last within its title.** It is not somebody to look up,
so it goes after the named ones — the placement the artist order and the printed book already give
it. The term that buys this differs with the column and must not be tidied into the other: the
curation database's `sort_artist` is nullable, where NULL is *nobody recorded one* and `''` is *one
recorded as blank*, so the leading term is `sort_artist IS NULL`; both catalogs' columns are
`NOT NULL DEFAULT ''`, so theirs is `sort_artist = ''`. Dropping either term does not fail — SQLite
sorts NULL and `''` first, which files the unnamed songs at the *head* of every title.

**The artist order is not the same question and does not change.** There, a song with no performer
keeps its place at the top of the list, because *the songs nobody attributed* is a group somebody is
looking at rather than a gap at the end of one.

**This reaches every order that gets as far as a title**, and in the curation tool that is six of the
nine: by suitability, by personal rating, by language, by when a song was last edited and by when it
was added all break
their tie on the title, and now break the title's tie on the performer. Length and copies never reach
a title at all. **Each of them is an index key as much as an order**, so the terms are named once and
the pair asserted against each other: an index that stops matching the order it serves does not fail,
it goes back to sorting the corpus, which nothing a page shows can reveal.

## User score

**0–10 or unset, stored per song, separate from the automatic suitability score.** Unset is not
zero — a song nobody has rated and a song rated unusable must not sort together. Like the automatic
suitability it rates a *file*, never a performance.

**One hand-set rating and not two.** A second one asking *how good is this to sing*, beside this one
asking *how good is this file*, is a distinction a curator makes in their head with a single number:
both are one person's taste, both are unset on almost every row of a real corpus, and telling them
apart on screen cost a column of the browse table, a select in every row, a band of the filter bar
and a sort nobody reached for. What a rating means here is *how much do I want this song in a
package*, which is the question the second one was asking in different words.

**It stops at the curation database because a package is the thing you hand to somebody else.** A
rating is one person's taste in a way the automatic suitability is not: whether a file has synced
lyrics is a fact about the bytes. A package carrying it hands a stranger somebody else's taste
wearing the same clothes as the measurements beside it, and the receiving machine has no way to tell
them apart. So it is a **curation aid** — filter and sort the corpus by it when deciding what goes in
a package — and never a property of one.

The kmspec has no `suitability:` key, so a description carrying one is refused by
`deny_unknown_fields`. A `.kmpkg` or a backup carrying a key this build does not know opens unchanged
with the key read past, neither setting `deny_unknown_fields`, so nothing stale is written over what
this corpus says now.

## How much lyric counts as a song

**A file has to carry a good amount of lyrics to score for having them.** One corpus file scored
**10/10** on three lines that were the arranger's name, two phone numbers and an email —
syllable-timed, well synced, melody found, full arrangement, and nothing to sing.

So the suitability measures *how much* there is: syllables (with obvious contact details discounted,
never names), and how much of the song's length the words span. Too little of either and the lyrics
and sync components are both zero and the file is marked defective, because a lyric covering four
seconds of three minutes is not a karaoke file however well those four seconds are timed. A middle
band scores 2 of 3 and says the lyrics are thin.

**The two ways of failing are named separately**: a business card in the lyric track, and a real
song timed only for its first verse — same verdict, different thing to tell somebody. Thresholds are
set from measurements of the local corpus, where the two clusters are separated by a wide gap, and
revising them means measuring again. Suitabilities already stored are not rewritten until a
re-analysis runs.

**A lyric track of chord names counts as no lyric.** Keyboard style demos put the chord of each bar
where the words go (`A# 7th`, `D# 6`, `C  min 7th /G`). The text is plentiful, spans the whole song
and lands on every chord change, so every other measurement passes it. The file is marked defective
with its own warning, `chord_names_only`, and scores zero for lyrics and sync.

**The file is judged, never the line.** A line reading `A` or `E` can be a word, so no single line is
discounted. A file is a chord chart when at least 8 of its lines, and at least 80% of them, are made
of chord names and nothing else.

## The sync component asks about the melody, not about the arrangement

**Whether the words were timed to this music is a question about the line being sung.** Asked of the
whole arrangement it is barely a question at all: a full backing puts a note onset almost everywhere,
so a syllable falls near one whatever it does. One corpus file drawing words that run eleven seconds
early by the last chorus still had **94%** of them within the window of some note, and took the full
three points. Against the channel carrying the tune, the same words score **36%**.

**So the melody's channel is the one measured, whenever the melody was identified without the
lyrics.** Where lyric alignment is among the signals that chose the channel, the channel agrees with
the lyrics by construction and the file marks its own work — there the wider measurement across every
non-drum channel stands instead, as it does when no melody is claimed at all. Melody detection still
costs nothing: a file whose melody cannot be found is measured the way it always was.

**The two windows stay different and both are deliberate.** A note counts as *being sung* at 60 ms
and counts as *evidence the words were timed to the music* at 120 ms, because the second question is
looser than the first. Narrowing which notes are asked about is not a reason to tighten how close
they must be.

## A channel named for the melody must play while the words are sung

**A track name does not make a channel the melody on its own.** A candidate must have a note onset
within one second of at least half the syllables. A channel that fails is not the melody, whatever
its track is called, and when no channel passes, detection abstains as `silent_under_the_words`.

**The name is often wrong.** One corpus file names a synth riff `Melody`. The riff plays the intro and
the break and rests under nearly every sung word. Detection trusted the name, the sync component then
asked the riff about the words, and a well-timed song lost all three sync points because only 1% of its
syllables landed on a riff note. The same claim puts the guide-melody toggle on a riff, and muting a riff takes nothing off
the singer's line.

**The name-only path stays for the case it exists for.** Words timed half a beat off a real melody
land on none of its notes, and the melody still plays under every one of them. One second is wide
enough for that and narrow enough to fail a part that plays only between the verses. Where alignment
already qualifies a channel, the gate passes by construction. A song with no syllables skips it.

**Measured over 4,000 corpus files sampled at random**, 2.9% of the claimed melodies were withdrawn
and none moved to another channel. Of those withdrawn, three quarters had no syllable within 60 ms of
a note. The rest carried the complaint that prompted the rule: well-timed words marked down against a
riff, and those files rise by two or three points. Where a riff was the runner-up, removing it let
eight songs abstaining as ambiguous find a melody aligned with 76% or more of the words.

## A file's own name for its lyric track outranks how long the track is

**`Words` wins, even when another text track holds more.** A second text track is a second timing of
the same song — a working copy, an earlier sync, a translation — and its length says nothing about
which of them the file means to sing. Counting events is the common shortcut and it is the one that
fails: one corpus file pairs a 243-syllable `Words` track landing on every melody note with an
unnamed 347-syllable track that runs steadily early and stops eleven seconds before the music does.
The words start right and are gone within a line.

**Length still decides among tracks the name does not separate**, because convention puts the words
on the track after the header and files disagree with that often enough that the order is worth less
than the count.

**`Words` and nothing else.** The wider list of names a non-Soft-Karaoke file is searched for takes
`melody`, and files carrying both a `Words` and a `Melody` text track are common enough that the
wider list would separate nothing. The name is matched whole rather than by its opening, which is
what keeps `Words & Music By ...` out: that is somebody's name in the place a name goes.

**Measured over 1,198 corpus files, all of which carry lyrics**: 36 hold two or more lyric tracks and
35 of those name one `Words`. Preferring the name changes the pick in 22 of them. Taking the share of
syllables falling within the sync window of a note on the best-fitting instrument channel: three
improve — 78% to 100%, 82% to 100%, 95% to 100% — eighteen are unchanged, and one falls from 100% to
97%, which moves no score and leaves the file at 10/10.

## Suitability, for a song that was made to be sung to

**A flat 10, for a video song, an MP3+G song and an UltraStar song alike — by what the file is, not by measurement.**

The suitability exists to answer "how good is this file as a karaoke source", and a commercial
karaoke disc or a purpose-made karaoke video is the best possible answer: the words are there, they
were timed by whoever authored them, and the backing is a real studio arrangement. There is nothing
in doubt for a number to resolve, and leaving it absent sorts professionally produced karaoke
*below* a mediocre MIDI file — the opposite of the truth, and exactly what the suitability exists to
prevent.

The stored breakdown is filled to match rather than left at zero, so it cannot contradict the
suitability it explains: it is a derivation that does not apply here, not a measurement that came
out full.

Two things this does not change: the hand-set **user score** is separate and about what one person
wants rather than about the file; and `melody_available` stays false, because a guide melody really
is absent rather than merely unmeasured.

## Song language

**An ISO 639-1 code from a table compiled into the binary, and a song cannot be packaged without
one.** A free string that nothing validates gives a column that can be *stored* and never *asked
about* — no filter, no sort, and no honest answer to "what Japanese have we got?", which is the
question that made video songs worth having. The local corpus holds `ENGL`, `PORT`, `ITAL` and
`ITALIANO` for what is one language.

**Neither witness exists for a video or an MP3+G song**, and more absolutely for MP3+G: a container
could in principle carry a subtitle track, whereas CD+G words are one-bit tiles with no character
data, so there is no lyric encoding to infer from and no `@L` header to read. ID3's `TLAN` frame
appears in *none* of the sampled corpus. Both populations are caught by the packaging gate with the
same two ways past it: `--default-language`, or the curation tool's bulk set over the current filter.
The practical shape is 25 album folders and a tool that already filters by folder, so classifying
2,849 songs is 25 actions.

**The whole standard is in the table**, all 184 of them, which makes it a *transcription* rather than
a judgment: there is never a question about whether some language has earned a row. Two entries are
deliberately not ISO 639-1 — `und`, because the standard offers no way to say *somebody looked and
could not tell*, and `zxx`, "no linguistic content", because an instrumental backing track honestly
has none and that is a different fact from nobody having looked. Both are three letters, which stops
them being mistaken for a language. `mul` is **not** offered: "several" is neither a language anybody
filters to nor a statement that nobody has said.

**Two letters were chosen over BCP 47** (`pt-BR`, `zh-Hant`, `yue-HK`), which can say things two
letters cannot — Brazilian against European Portuguese, Simplified against Traditional, and Cantonese
at all, which lands under `zh`. Preferred for how it reads and types, and the door is not shut: every
code here is a valid BCP 47 primary subtag, so nothing stored would have to change, only gain
suffixes.

**Three witnesses say what a song is in, and they are read in order of what stands behind them.**
What a person typed comes first: somebody looked at the song. Then what the file said about itself.
Then what the song's own words read as.

**The file's two witnesses are transcription rather than inference**, and the stronger of them is the
lyrics' text **encoding** — a Shift-JIS track is Japanese whatever the header claims. Only what is
unambiguous is mapped, so `windows-1252` (a dozen languages), `windows-1251` (a region) and UTF-8 all
say nothing. The Soft Karaoke `@L` header is the weaker: the file made a statement and this reads it.

**The words are read last because a reading is worth less than a statement**, and it is admitted at
all because over 90% of a real corpus has neither witness above it — a column that is empty on nine
songs in ten answers no question. What reads them is a trigram and alphabet model over the lyrics,
falling back to the title, in `km-langguess`.

**A reading is stored only when it is certain enough to be a fact**, above a named confidence, and
below that nothing is written. That gate is what makes the ordering safe rather than merely tidy: a
guess counts as the song's language, so it satisfies the packaging gate, and one admitted on a
balance of probabilities would put a fabricated language into a package. The confidence is not a
proxy for how much text there was — it already holds that, a two-word title in a shared alphabet
scoring around 0.49 where a verse of 262 letters is certain, and a title carrying characters only one
language writes is certain at ten letters.

**How sure it was is shown wherever a guessed language is.** It is the one witness that can be wrong
about a song it read correctly, so a close call has to look different from a certainty to a curator
deciding what to check.

**`ENGL` is taken at face value, and the measurements say that is uncomfortable.** Sampling 60
`.kar` files from each language-named folder: `Ingles/` says `ENGL` 59 times and is right; `Brasil/`
says it 35 times and is wrong 35 times out of 36; `Musicas Italianas/` says it 30 times against 28
that say Italian. It is the editor's default, not a statement. Discarding it is worse: it leaves
over 90% of a real corpus with no language at all — **a column that is empty on nine songs in ten
answers no question, whereas a populated one that is sometimes wrong can be corrected in one
action.**

**When no witness speaks, nothing is written** — not `und`, which is a claim somebody made, and
writing it automatically would satisfy the packaging rule on every song in a corpus nobody had looked
at. A song whose words nothing could place confidently reaches the end of all three and is honestly
unclassified, which is what the browse list's `unset` filter is for.

**A package build refuses a song with no language**, in `km-pack build` and in the curation tool's
build, and **deliberately not in the manifest's own validity check**: `Package::open` runs that same
check, so a rule there would refuse to open every package built without a language and empty the
catalog of every machine in service. `--default-language <code>` is the way past it rather than an
`--allow-missing-language`, because a flag that turns a rule off ends up in a build script forever
and produces exactly the artifact the rule forbids. `--default-language und` produces songs that
*say* undetermined, which is a fact and is queryable.

The curation tool also gets a **bulk action**, setting the language of every song matching the current
filter, because on the local corpus 92% of songs have none and classifying those one page
at a time is not a thing anybody finishes. It reuses the browse list's own `WHERE`, so what is listed
and what is written cannot diverge, and it confirms with the count and the filters first.

## Song tags

**A song may be filed under any number of open-vocabulary words — `rock`, `anime`, `brasil` — and
every browsing surface can narrow by all of them at once.** A language is the only dimension a
catalog could be narrowed on, and it answers *what Portuguese have we got?* and nothing else. The
questions a room full of people actually asks a karaoke machine are about kind, and no closed table
can hold them.

**Deliberately unlike a language in four ways.**

- **The vocabulary is open, but a tag is a slug.** A language is one of 184 rows compiled into the
  binary; a tag exists because somebody typed it, so there is nothing to validate *against*.
  `Tag::parse` therefore **normalizes** first: it runs `km_song::text::fold` — the alphabet of the
  whole product, the same one the sort keys and FTS5 use — and joins the words with `-`. `Forró`,
  `forro` and `FORRÓ` are one tag; `Rock & Roll` is `rock-roll`. A control that answered three
  spellings of one word with *that is not a valid tag* would teach people to type the fourth.

  **And then it refuses what is not `[a-z0-9-]`.** `fold` maps the Latin-1 accents to ASCII and
  passes through every alphanumeric it has no mapping for, so this is what stops `日本`, `рок` and
  `Straße` becoming tags. Two reasons: a slug is an ASCII thing — typed into a URL, read back off
  one, shown in a chip a few characters wide, and a vocabulary somebody cannot type on the keyboard
  in front of them is one they cannot filter by. And **a tag is not a title**: this is the one place
  in the product where a title may hold a character a tag may not — a title is the song's own name
  and is stored as it is, while a tag is a word chosen to file things under.

  **Refused rather than stripped**: stripping would turn
  `rock日本` into `rock`, a tag that looks right and filters wrongly. `km-pack` and the curation tool
  both refuse it by name; a `?tags=` on a browsing surface drops it, exactly as it drops a typo. The
  Latin letters `fold` has no entry for — `ß`, `ø`, `æ`, `ł` — go the same way, and extending that
  table is not the fix: it is the alphabet of every title, sort key and search box, so widening it
  for tags would change all four.
- **A song has many and nothing reconciles them, so the filter is OR.** One kind of song arrives
  filed under several words: `rock` on the rows one person tagged and `rock-nacional` on another's,
  `xmas` beside `natal`, `mpb` beside `brasil`. Picking both means *either of these*. An
  intersection of words nobody reconciled is empty far more often than it is useful, and a second
  pick that empties the list is what teaches somebody the filter is broken.

  **A tag narrows the list against the catalog; a second tag widens it against the first**, and
  that is the whole shape. The closed dimension beside it goes the other way for reasons this one
  has neither of: a song has exactly one language, and the 184 rows are a table somebody compiled.
- **Nothing detects one.** A language has a detected half and a hand-set half; a tag has only the
  second. So there is no *nobody has said* state, no `eff_` expression, and no scan interaction — and
  it must survive a backup and a rebuild from source, because it is nothing but hand curation.
- **They are optional where a language is compulsory.** A build refuses a song with no language, so
  `default_language` exists to fill the gap. Nothing refuses a song for having no tag, and there is
  therefore **no package-level `default_tags`**: it would be a value nobody checked, written onto
  every song for no reason.

**One spelling on every surface: `?tags=rock,brasil`, comma-joined, never a repeated key.** That is
forced rather than preferred — `axum::extract::Query` is `serde_urlencoded`, which answers a repeated
known key with a 400, and htmx does not swap on an error, so the control would stop working with
nothing said anywhere. A comma is unambiguous by construction, because `Tag::parse` folds one to a
word break.

**The remotes filter by tags and never draw them on a song row.** A phone is narrow, and a run of
chips beside every row would cost more width than a filter is worth. The curation tool draws them,
because that is where curation is checked and a tool has to show what it wrote.

**The printed book takes tags as a filter and never as a section.** It is sectioned by language,
which works because a language is a closed table with a name per row; an open vocabulary has neither
an order nor a heading, and a song carrying three tags would have to appear three times or
arbitrarily once.

**The machine's catalog stores them twice on purpose**, and only one is the truth — see
`docs/architecture/packaging.md`. The packed `songs.tags` column rides `SONG_COLUMNS`, which is what
makes `package_digest` notice a package rebuilt with nothing changed but its tags; without it every
mirrored phone would keep the tags it downloaded once, for ever, with nothing reporting a fault.

## A song's first lines travel with it

**A package carries the first two lines of each song's words, skipping the banner most files open
with.** A package could say a song's number, title, artist, suitability and language and carried
nothing anybody could *read* to recognize it; two lines tells one `Tempo Perdido` from another and
is short enough that a four-thousand-song manifest does not become a lyric database. Each line is
cut at 120 characters, which is not a hypothetical bound: break markers are stripped during parsing,
so a file carrying none yields **one** line holding the entire song, and the corpus has those.

**The banner is what the skip is for.** A great many real files open with the sequencer's
advertisement rather than the song — a studio name, a telephone number, a web address, a row of
asterisks — and a preview of that says only that the file came from somebody.

`km_song::looks_like_a_banner` holds the rules, and **it is deliberately a second function rather
than a widening of `km_suitability`'s `is_credit_line`**, which shares two of them. That one decides
whether a line counts towards *how much lyric a file has*, feeding a 0–10 suitability stored in
every package ever built, so widening it would silently move suitabilities across a whole corpus.
This one decides whether a line is worth showing, where being wrong costs one line. **Two functions,
two blast radii.** Only *leading* lines are skipped and at most eight, so a false positive costs a
line rather than the song.

**The rules are measured**, by `km-lyrics preview` over the real corpus.

**Do not skip a line because it repeats the song's title.** Popular music names itself after its own
opening line far more often than a sequencer prints a header, so such a rule turns
`Good golly Miss Molly` and `Ain't no sunshine when she's gone` into each song's *second* line.

Two rules come out of the same pass: the wrapped legal notice commercial discs carry, whose second
line became the preview of 97 files in 30,000 because only its first line matched; and an email rule
that takes the token after the `@` rather than the rest of the line, because an address with a
telephone number after it is the *shape* of one line in this corpus.

The table that finds such gaps is **first kept line, where more than one file shares it**: real lyrics
differ per song, so a line hundreds of files open with is boilerplate that got through.

**Measured over the entire corpus**: 99.5% parse, 22.9% of those carry words, and of those
**95.4% end up with two lines and 90.1% needed nothing skipped at all**. 0.85% get one line, 3.7% get
none — a file whose whole short lyric track is credits, which is the honest answer rather than a
failure. 52 of the songs carrying words exhausted the eight-line budget.

**Running it on everything rather than on a sample found two more**, both about language and
punctuation: the phrase list was English and put a credit after *by*, while half this corpus is
Portuguese and Spanish and puts it after *por* (223 files); and `exclusive by: tomson` (88 files)
matched nothing, because the verb in front of the colon is not one anybody could enumerate, so the
**colon** is what to key on.

**A banner is usually a block, and removing its first line reveals its second.** The unfilled
template `song title` / `artist` is two lines, and catching the first promoted the second to being
the preview of the same 110 files.

What is left uncaught, in order, is a person's name (160 files), a studio name with no shape to key on
(136), mojibake from a misdetected encoding (136) and one program's version banner (108) — and **real
lyrics begin at rank four of that table**, which is where to stop. A rule catching a name would catch
song titles, and mojibake is a decoding problem wearing a preview's clothes.

**Empty for a video song and an MP3+G song**, which is what they are rather than a gap: the words in
both are pixels. **Detected, never edited** — a fact about the bytes like the duration, so a rebuild
re-derives it and there is nothing to correct.

**The manifest format version does not move for this field.** The version is chosen by content and
nothing in the manifest refuses unknown keys, so a package built without a preview opens normally. The
consequence is the other direction: a package **gains** a preview only by being rebuilt.

It reaches the catalog as one nullable column and the API as `lyric_preview`, absent entirely when
there is none. It is **not** in the FTS index — searching the words is a different feature, and FTS5
cannot `ALTER`. The API field carries `serde(default)` as an obligation rather than a habit:
`km-remote-core` deserializes that same struct out of the NDJSON export and refuses a page if one row
will not parse, so without it an updated phone would reject every row from a machine that had not been
updated.

## A stranger's contact details are taken out of the words

**An email address, a web address or a telephone number inside a song's lyrics is replaced by a dash
before anything reads them — the television, the wire, the package, the book, the search index.** The
line survives; only the address goes.

**The problem is not that it is untidy, it is that it is somebody's.** A great many files carry the
sequencer's business card inside the lyric track, and a karaoke machine does with it what it does
with lyrics: draws it across the television, wipes it in time with the music, publishes it on the
`lyric_line` event, stores it in the package preview, prints it in the song book and indexes it for
search. That is a private individual's address traveling into every machine and every printed book a
package reaches. The repository's own rule that
[no committed file may name a person](repository.md#what-a-committed-file-may-say-about-the-machine-it-was-written-on)
is the same judgment pointed at ourselves; this is it pointed at the corpus.

**Masked, not dropped, because the two shapes are different.** The pure business card is one shape and
losing the whole line costs nothing; an address sitting inside something otherwise singable is the
other, and dropping that line takes words the singer needed. One rule handles both if it removes
characters rather than lines, and a false positive then costs a word instead of a verse.

**It happens where the syllable is written, not where the line is read**, which is the only place that
works. `LyricLine::text()` looks like the tidier seam and is a trap: the display draws the *current*
line from the syllables directly so it can wipe one at a time, and the API puts the same field on the
wire — so masking in `text()` would leave the address on the television and in the API while hiding it
from everything nobody was looking at. Written once in `build_timeline`, it reaches every consumer.
Timing is untouched: a syllable whose text goes away keeps its ticks.

**This is the third contact-detail rule, and the three are not one.** `looks_like_a_banner`
decides whether a *leading* line is worth showing; `is_credit_line` decides whether a line counts
towards how much lyric a file has; this one decides *which characters* are contact details. They must
stay separate for the reason [the preview decision gives](#a-songs-first-lines-travel-with-it).

**Masking blinds the rules that were reading the text to find what has just been removed** — twice:

- `is_credit_line` stops recognizing the line, so every business card in the corpus would begin
  counting as lyric and every affected suitability would move.
- `looks_like_a_banner` stops recognizing it too, and worse: a telephone banner masked to three digits
  reads as words, so **the business card would have become the package's stored preview** — the
  precise outcome the banner skip exists to prevent.

The line carries a `contact_redacted` flag and both read it first, rather than re-deriving from text
that no longer says so. **A redaction is not finished when the text is clean; it is
finished when everything that was classifying that text still gets the same answer.**

**The telephone rule asks about the line, where the other two ask only about the token.** A number is
masked only when the line already carries an address, or holds no word outside its contact details.
`867-5309` is a lyric, and `is_credit_line`'s digit *ratio* is no defense — `867-5309 I got it` passes
it exactly. That rule is right where it lives, because setting a line aside costs a syllable count;
here it would cost the hook of a song somebody came to sing.

## A line that is only a legal notice is not sung

**A lyric line that is the publisher's notice and nothing else is dropped before anything reads it.**
`ALL rights reserved. Not for broadcast or transmission of any kind. DO NOT DUPLICATE. NOT FOR
RENTAL.` is not a line of a song, and a machine that draws it across the television wipes it in time
with the music to whoever is standing at the microphone.

**The header form was never the problem.** A notice carried as an `@W` text event is metadata and is
already left out with every other `@`. The same sentence also arrives as an ordinary text event with
no `@` on it, and as a lyric event, and those reach the words — which is where 55 songs of the local
corpus got a lyric that is only this.

**Dropped, where a stranger's contact details are masked**, and the two are different for the reason
[that entry](#a-strangers-contact-details-are-taken-out-of-the-words) gives: an address can sit inside
something otherwise singable, so removing the line would take words the singer needed. A line that is
*only* the notice is the other shape, the one where losing the whole line costs nothing. `NOT FOR
RENTAL.` has no reading as a lyric.

**This is a third rule and not a widening of either of the other two**, which is the constraint
[`A song's first lines travel with it`](#a-songs-first-lines-travel-with-it) states. `looks_like_a_banner`
decides whether a *leading* line is worth showing, where being wrong costs one line of a preview;
`is_credit_line` decides whether a line counts towards how much lyric a file has. This one decides
whether a line may be thrown away anywhere in a song, which is the widest consequence of the three
and so carries the tightest test: the marks are shared with the banner rule, but a mark has to be
there **and every word of the line has to be a word a notice is made of** — the marks, the words that
join them, and a year. `Copyright 1994 Some Publisher` keeps a publisher and stays a banner;
`All rights reserved, TUNE 1000 CORP.` keeps a company and stays one too.

**Word by word and never by substring**, because taking `or` out of `world` is how a rule like this
eats a verse. The joining words are reached only on a line that already holds a mark, so `and` alone
is still a lyric, and so is `100% PURE LOVE`.

**A file whose whole lyric track is the notice ends up with no words**, which is the honest answer
rather than a failure — the same answer `A song's first lines travel with it` already gives for a
file whose track is all credits.

**Suitabilities move, and that is a correction.** Dropping a line takes its syllables, so the
affected files lose lyric coverage they were counted for. They were scoring for words nobody sings.
`km-pack reanalyze` recomputes a built package without rebuilding it, and a curation database
refreshes on a `--force` scan.

## An underscore in the words is markup about a space

**An underscore inside a lyric event says something about a space rather than being a character
anybody sings, and none of them reaches the screen.** Left alone it is drawn across the television,
wiped in time with the music, published on the `lyric_line` event, stored in the package's preview,
printed in the song book and indexed for search, so `Se apronta` arrives at a singer as
`Se_apronta`.

**It carries two opposite meanings, and position is what separates them.** Both shapes are in the
corpus, so a rule keyed on the character alone spells one of them wrong:

- **An elided space**, where the word carries on either side of the mark. Portuguese sings two words
  across one note and the writer joins them so the syllable stays one timing point: `Se_a` + `pron` +
  `ta ` is `Se apronta`. The mark becomes a space.
- **A cancelled space**, where nothing but whitespace follows the mark. A writer who ends every
  syllable with a space needs a way to say *not this one*: `MEL_ ` + `O_ ` + `DY ` is `MELODY`, and
  reading that mark as a space gives `MEL O DY`. The mark and the space it cancels go together.

A mark asking for neither — `do ` + `_a ` + `ve` + `jo `, where the space it wants is already
there — loses the mark and keeps the spacing around it, which is the reading that costs nothing
whichever was meant. A run of marks is one mark.

**1.77% of the parsed corpus files that carry any lyric carry a mark**, and
41,505 marks between them: 30,186 elided spaces, 9,783 cancelled and 1,536 stray. **The cancelled
space is nearly a quarter of them**, which is what settles the shape of the rule: reading every mark
as a space would spell one syllable in four of the affected words wrong, and reading every mark as
nothing would run the other three together.

**It happens where the syllable is written**, for the reason
[the contact-detail rule gives](#a-strangers-contact-details-are-taken-out-of-the-words): the display
draws the current line from the syllables directly so it can wipe one at a time, and the API puts the
same field on the wire, so a rule applied at the joining would leave the mark on the television and
hide it only from whatever nobody was looking at. Timing is untouched — a syllable that loses a
character keeps its ticks. Break markers come off first, so a payload closing `MEL_ \r` still ends in
the space its mark cancels.

**The machine parses a song's file when it loads it, so the television answers for every song in the
catalog.** A package's `lyric_preview` and the search text beside it hold what the package was built
from, which is what building it again carries this into.

**A syllable that is nothing but a mark goes, and that is the one thing here that changes what a file
counts as rather than what it reads as.** An empty syllable is not drawn, so it is not kept, and a
file therefore has fewer of them than its events. Over the whole corpus that is one file whose words
were marks and nothing else and now counts as having none, two more that fall from syllable-level to
line-level timing, and three suitabilities that move by a point. The measurement is worth stating
because a syllable count is an input to the stored 0–10 suitability, and
`docs/architecture/song.md` is emphatic that a rule reaching it does so knowingly.

**The three line classifiers reach the same verdicts, which is why this needs no flag of its own.**
`looks_like_a_banner` reads `km_song::text::fold` for its label and single-character rules, and
folding already maps an underscore to a space; a row of underscores has no alphanumeric in it before
and is empty after, so the ornament rule catches it either way; and both telephone ratios only get
easier to satisfy. That is the difference from a redaction, which blinds the rules reading the text
it changes.

**Two things this deliberately does not do.** A title is not a sung syllable, so `clean_meta_text`
is untouched and a drawn title keeps its underscores; sorting and search are unaffected there because
they fold. And an underscore somebody meant, `AC_DC`, draws as a space, which is the price of a rule
that leaves none behind.

## A control character in the words is padding, and none of it reaches the screen

**A lyric carries text or it carries nothing, so every control character comes out of a syllable
before the syllable becomes one.** A great deal of software writes MIDI text events as fixed-length
fields and puts the terminator in with the payload, so a five-byte field holding `%SOL` arrives as
`%SOL\0`. That is the same padding `clean_meta_text` already takes out of a title, arriving by the
other door — and a syllable that is nothing but padding is dropped, exactly as a bare space mark is.

**Left in, one of them ends the machine rather than the frame.** SDL_ttf is asked for a C string,
and a Rust string with a NUL inside it cannot become one; the failure lands on the display thread,
which is the one thread whose panic stops the process. So a corpus file that plays perfectly well
everywhere else takes the machine down between one lyric line and the next, on a television, in
front of a room.

**The order is what makes it safe**: the break markers and the space marks are read first, so `\r`
and `\n` have already been understood as the line breaks the timeline is built out of by the time
anything is removed. What is left after them is a character somebody sings or it is nothing.

**The display refuses one anyway**, and that is not redundancy. Words are cleaned where they are
read, which is the fix; the floor under it covers everything else that reaches a font and no single
parser owns — a title out of a package, a name somebody typed into a phone, a file name off a disk.
The measuring and drawing paths were already written to shrug off a string they could not use, and
that was never reachable, because the dependency panics before it can refuse.

## A file that marks no word ends draws a narrower space

**A file that puts a space after every syllable is drawn with a narrow space between its syllables
rather than a full one, and loses a point of its suitability for it.** Such a file claims each of its
fragments is a whole word, so `cantaremos juntos` reaches the television as `can ta re mos jun tos` —
six words where there are two.

**No word boundary is invented, because none is recoverable.** The fragments themselves straddle the
words, so the file cannot even be re-joined by machine: a run reading `mos`, `jun`, `tosa` is
`mos junto sa` as readily as `mos juntos a`, and no rule gets the right one back. The timing does not
hold the answer either — the gap between two syllables of one word measures the same as the gap
between two words. Only a dictionary of the song's language could do it, and that is not what a
karaoke machine is.

**Narrowed rather than removed.** Joining the fragments would draw `cantaremosjuntos`, which is
faithful to the fact that the spaces say nothing and harder to sing from than the defect. A thinner
gap keeps the fragments apart, which the singer needs to follow the highlight, while no longer
reading as a word end. It is a sixth of an em, which draws a 12 pixel gap at a 72 pixel face against
a space's 20, and which is the one narrow space whose width does not move between faces. Narrower
still closes the gap rather than narrowing it.

**The effect is per gap, not on the line.** A whole line comes out only 4 to 7% shorter, so the face
the line is fitted into is usually the same one. What changes is that each gap stops reading as a
word end.

**Only a file that marks no word is touched.** A single leading space is the file saying where a
word begins, and a file that says so anywhere is trusted completely. **A break marker does not count
as saying so.** It says where a line ends and nothing about the words inside it, and a file can place
every one of its lines and still space every syllable. The file is judged on its words and not on
its line markers, on two measurements, and the second matters as much as the first:
a file with one event per *whole word* also spaces every event and marks no ends, and its words are
exactly where it says they are — so fragment length is what separates the two. **Below a mean of 3.0
characters the corpus holds syllable files only**, and they are judged on that alone. **Between 3.0
and 3.65 the two kinds overlap**, because English is mostly words of one syllable and a file that
spaces them averages what a file of short whole words does. There the share of fragments of seven
characters or more decides: the syllable files hold 0 to 3.4% of them and the word files mostly 4.3%
and above, so a file under 4% is judged. The few word files that fall under it draw their words a
divider apart, which still reads as words; the syllable files it reaches would otherwise draw every
syllable as a word. Above 3.65 the files are whole words almost without exception, and none is
touched.

**The point off is the words, not the timing.** These files are syllable-timed and the highlight
follows the singing, so this is nothing like line-level timing and does not score like it. What is
lost is that the words as drawn are not the words, which is the same order of loss as lyrics thinner
than a song's — and it is scored the same. The file stays singable and is not a hard defect.

**The point exists to sort duplicates.** The corpus carries the same song many times over, and a copy
that marks its word ends is the better file. A suitability that says so is how the package builder
offers that one first, which is the only real remedy: nothing improves the affected file itself.

**The drawing changes on upgrade; the number waits.** A package stores its suitability but not its
lyric timeline, so the narrower space reaches every package already built as soon as the machine
re-parses, while the point comes off only under a reanalyze or a rebuild, and the curation database
only under a forced rescan.

## A file that marks no word boundary at all draws the same divider

**A file that carries no space in its lyrics is drawn with a narrow space between its fragments, and
loses the same point of its suitability as a file that spaces every one.** Such a file says nothing
whatever about its words, so `and this is crazy but heres my number` reaches the television as
`andthisiscrazybutheresmynumber` — one word where the line has seven.

**It is the claim [`A file that marks no word ends`](#a-file-that-marks-no-word-ends-draws-a-narrower-space)
answers, made from the other side.** One file says each of its fragments is a whole word and the
other that a whole verse is one, and neither is true. What the two share is that no boundary is
recoverable: a dictionary of the song's language could find them and nothing else could, and that is
not what a karaoke machine is. So the drawn answer is the same one, for the same reason — a gap that
keeps the fragments apart, which the singer needs to follow the highlight, and does not read as a
word end.

**Joining is what that decision already refused**, and this is the shape it refused it for. A file of
this kind arrives joined, so the run of letters is the defect rather than the faithful reading of it.

**Sheet music read by an optical scanner is where these come from.** Such a program writes one lyric
event per note and no separator of any kind, because a printed score marks its words by where they
sit under the staff. The fragments are a mixture of whole words and syllables, and nothing in the
file says which is which.

**0.07% of the files the corpus parses**, against 0.03% for the file that spaces
every syllable. The shape is rarer than a wrong encoding and commoner than the one already answered.

**A file needs to mark nothing at all, and four measurements keep it there.** A break marker anywhere
is the file placing a line, and a file that places anything is trusted completely. Fewer than
thirty-two fragments is not a verse. **A character of a script that writes without spaces disqualifies
the file outright** — Han, kana, Hangul, Thai, Lao, Khmer and Myanmar run their words together because
that is how they are written, so joining them is correct and a divider between them would be this
same error committed the other way. That guard is not hypothetical: the corpus decodes 2,145 files as
Big5, GBK or EUC-KR.

**The fourth is that the file spaces no more than one fragment in twenty, and not that it spaces
none.** A space arrives from places other than a convention: an underscore meaning an elided space
resolves to one, and the corpus file this rule was written for holds exactly one such space among 275
fragments. Requiring none left that file drawn as a single run of letters. A file that marks word
boundaries at all marks most of them, so a share this low is noise rather than a file speaking, and
the handful of fragments that do carry a space keep it and take no divider beside it.

**No mean fragment length is measured**, where the rule for a file that spaces everything measures
one. That measurement is there to spare a file whose one event per whole word carries a real space,
and here there is no space to spare: joining is wrong whether the fragments are words or syllables.

**The lines of an affected file are re-broken, and no other file's are.** The divider is a character,
so it counts toward the width a line is broken at, and the fragments of these files are short — the
file above goes from 27 lines to 34, each narrower than before and none of them a run of letters. A
file the rule does not claim is untouched.

**The drawing changes on upgrade; the number waits**, which is the pair of speeds
[`A file that marks no word ends`](#a-file-that-marks-no-word-ends-draws-a-narrower-space) has and
for the same reason.

## A file is trusted for the lines it marks, and not for the verse it says nothing about

**Every break marker a file writes is honored, and a run of words it marked nothing across for more
than 100 characters is broken at the pauses its own timing shows.** A file that marks where its lines
go is telling the truth about the lines it marks. It is saying nothing at all about the stretch
between two of them, and a verse with no marker in it is not a line the file placed.

The corpus holds files marking a chorus perfectly and leaving whole verses unmarked, so one file
yields twenty-two lines of 16 to 79 characters beside three of 418, 394 and 119 — the 418 running
44 seconds. **0.65% of the corpus files that place their own lines hold one past 100
characters**, which is what re-breaking at that bound reaches.

**A cut lands where the file paused, never where a width ran out.** A singer reads phrases, and in a
file that has lost its markers the silence between two of them is the only evidence left of where one
ended. The widest gap in a run is the cut, then each half is judged the same way. Cutting at a
character budget instead lands mid-phrase every time, and is worse than that in the files this
reaches: their phrase gaps frequently sit just under the 1.2 s the unmarked-stream heuristic asks
for, so the threshold that serves a file marking nothing finds nothing here.

**A pause is read against the run's own syllable step rather than in milliseconds**, which is the
only form that survives a corpus of every tempo and timebase somebody has sequenced in. A run's
median gap is what one syllable following another costs in it, and a gap three times that is a rest.

**Three sits between the first quartile of what a real line break is worth and the median**, measured
by `km-lyrics scan` over every line those corpus files place: a break a file writes is 1.0×
its own step at the 10th percentile, 2.0× at the 25th, 4.0× at the 50th and 13.0× at the 90th. Above
the quartile on purpose. A quarter of real breaks are written at twice the step or less, which
nothing separates from one syllable following another, so the two errors are different sizes —
missing a break leaves a line wider than it might have been, which
[the face ladder](interface.md#a-lyric-line-that-will-not-fit) was built for, while taking one that
is not there cuts a word in half in front of somebody singing it.

**No cut lands inside a word.** A file marks word ends with a trailing space or a leading one, and a
cut may only open a word. A file that marks neither offers nothing to respect, and there the gaps
decide alone — which is the same answer
[`A file that marks no word ends`](#a-file-that-marks-no-word-ends-draws-a-narrower-space) gives, for
the same reason: no rule recovers a boundary the file did not write.

**A run with no pause left in it stays exactly as wide as it came in**, however far past the bound
that is. This is where the cutting stops, and the width is not: a segment nothing may be cut inside
is one long line, the face ladder answers it, and every alternative is a different way of being
wrong. The same holds for a run of one unbroken word.

**The bound is not `COMFORTABLE_LINE_CHARS`.** Forty characters is what reads comfortably and is what
a line is cut *to*; a hundred is where trusting a file stops describing anything a sequencer did.
Re-breaking every marked line past forty would take lines a file wrote deliberately and fit on the
screen, which is second-guessing: the corpus puts the 90th percentile of a placed line at 35
characters and the 95th at 40, so forty is where ordinary lines *end*, and a hundred is past the
99.9th percentile of 75.

**The cutting happens before the words are read, so what reads them sees the lines the screen will
draw.** Three things downstream ask about a whole line: an address is masked in one, a line that is
only a publisher's notice is dropped, and `km_suitability` counts a line that looks like a credit as
no lyric at all. Each of those was answering for a whole verse where the run held one address, which
is the wrong blast radius for all three — a mask now covers the line that carries the address, and the
verse around it counts as the lyric it is.

**So an affected file's suitability moves, in both directions, and only an affected file's.** Over
the whole corpus that is a few dozen files: thirteen leave 10/10 and ten arrive at
9/10, with single figures either way below that. Both directions are the same mechanism seen from
two ends. A verse stops being discarded for the one address inside it, which is a point gained; and
the address, now a line of its own, is a credit line that counts as no lyric, which takes syllables
off a file that had few — fourteen more files read as sparse. Four cross into line-level timing,
because a file's timing points per line falls when it has more lines and that is what line-level
means.

A package stores its suitability and not its timeline, so the drawing changes as soon as the machine
re-parses while the number waits for a reanalyze or a rebuild — the same two speeds
[`A file that marks no word ends`](#a-file-that-marks-no-word-ends-draws-a-narrower-space) has.

**Nothing an ordinary file draws moves at all.** Over the same corpus the width of a placed line is
unchanged at every percentile through the 99.9th, and 9,383 lines are added across it. What moves
is the tail it was aimed at: the files holding a line past 100 characters fall from 865 to 511, past
200 from 388 to 149, and past 300 from 262 to 77. The 511 that remain are runs holding no pause to
cut at, which is the answer above, and many of them are narrower than they were.

## A half-read file plays, and says which track was lost

**A track the parser could not finish is recorded, not refused, and any note left sounding is turned
off where the data stopped.** Two separate defects with one symptom: a note that never stops.

**midly discards the rest of a track on any malformed event and returns no error.** Its `strict`
feature would turn that into a refusal and is deliberately not enabled: files
that today play imperfectly would stop playing at all, and **2.01% of the parsed corpus files is
this shape**, 22,049 tracks abandoned between them. So `Song::parse` reads through the
lower-level API and watches `unread()` before each `next()`, because the iterator empties its own
buffer *before* reporting exhaustion and sampling afterwards always says zero. `truncated_tracks` and
`missing_tracks` carry the answer; neither is an error.

**The repair is unconditional rather than limited to truncated tracks**, because truncation is one way
to arrive at a hanging note and not the commonest. **2.91% of that corpus ends holding a note** —
17,864 files, 96,646 note-offs synthesized — much of it in data that is perfectly well-formed, which
no parser can reject because it is not wrong. A note still down when its track's data runs out gets a
note-off at that track's last tick.

**It leaves nothing behind, checked on the corpus rather than on fixtures.** `truncation_census`
asserts the invariant on every file it reads — no note-on without a later note-off. Across every one
of them it fires **zero** times.

**Per track, and that is measured rather than assumed.** Pairing within a track would cut a note short
if a file put its note-off on a different one. It does not: over a 5,785-file folder with the repair
disabled, the number of files unbalanced when paired *globally* is 138 — the same 138 the per-track
pass finds — so the two sets coincide exactly. What per-track buys is where the note-off lands: at the
point of truncation, rather than at the end of a song the track never reached.

**`km-suitability` keeps the opposite convention and is deliberately untouched.**
`ChannelStats::finish` holds a still-sounding note to the end of the file rather than dropping it,
which is right for measuring how much of a song a channel occupies and wrong for playing it.

**Reading the tracks in parallel is midly's own behavior.**
`collect_tracks` splits above 3 KiB and karaoke files are tens of those, so reading them serially cost
19% on a warm 4,000-file pass — 2.24 s against 2.66 s — for no gain. The parallel and serial versions
produce byte-identical censuses.

## A chord is not a word, and a bracket that opens every line is a mark

**Some files write their lines one event at a time, opened with `<`, and spend most of their lyric
events on chord symbols: `%SOL`, `%FA7+`, `%MI-7` — Latin note names, minor as `-`, major seventh as
`7+`.** Neither is anything anybody sings. The brackets are read as line marks and the chords are
dropped, so the television shows `LET THE MUSIC PLAY ON` and an instrumental shows nothing at all
rather than two lines of chord symbols.

**Rare, and in the part of the corpus that gets curated.** Over the 11,857 files of the local corpus
the neighbouring rule was measured on: 5 files open their lines with a bracket and 2 of those also
write in marks. A wider sample of 1,019 files with words in them, drawn across the whole corpus,
found the same order — about one in a hundred. It is a small number of files reached by an owner
picking songs rather than by a sweep, which is how the first one was found.

**Chords are dropped rather than drawn somewhere else.** A chord row over the words is a real thing a
songbook does and a genuinely useful one with a guitar in the room — and it is a new surface for the
television, the book, the remote and the packaging profile each to answer for. This machine is for
singing. The door is not locked: the file's chords are in the file, and a decision that wanted them
would start here.

**Two rules, and the bracket is the one that stands alone.** Files marking lines with `<` and
carrying no chords at all outnumber files doing both, so the bracket earns its own verdict. The mark
does not: it is read only in a file that also marks its lines, and no corpus file was found using
either habit without the other.

**Both are read from the file's habits, never from one event.** Punctuation becomes markup only
where a file uses it that way throughout — the same argument
[`A file that marks no word ends draws a narrower space`](#a-file-that-marks-no-word-ends-draws-a-narrower-space)
is built on. The shares are a quarter of the events for marks and nine tenths of the unmarked ones
for brackets, over at least sixteen events; below that a file has no habit to read.

**The mark is what is recognised, not the notation after it.** The two sampled files spell their
chords differently — `%SOL` `%LA-` `%FA7+` in Latin note names, `%Bm7` `%F#m7` `%A7+/D` in English
ones, with slash chords and accidentals — and a test for the notation would have read one file and
left the other showing chords. The structure is identical in both: 92 marked events of 138 in one
and 145 of 187 in the other, every remaining event opening a line, and nothing else present at all.

**What makes that safe is the second habit rather than a narrower test.** `%` on its own is a
percent sign, and `100% PURE LOVE` must stay a lyric. A file that opens every line with `<` *and*
prefixes a quarter of its events with `%` is not a file whose words begin with a percent sign a
quarter of the time.

**A dropped event still gives back the break it was carrying.** A writer that closes its lines on the
chord sends `/%LA-`, and a rule that left with the text would take the line break with it — the one
way this could quietly change a file it was only meant to tidy. The same reason the whole-file
measurement reads past the ordinary marks before it judges a payload: measuring the raw bytes would
count `/%LA-` as neither a chord nor a line and talk the file out of a dialect it plainly has.

**This moves stored suitability, and that is the cost to weigh.** Measured on the file the rule was
written from:

| | lines | syllables | granularity | suitability |
|---|---|---|---|---|
| before | 84 | 138 | syllable-level | 8/10 — lyrics 3, sync 3 |
| after | 44 | 44 | line-level | 5/10 — lyrics 1, sync 2 |

**The 8 was the markup flattering the file.** Its chords were being counted as timing points, so a
file timed one line at a time passed for one timed syllable by syllable, and the sync measure was
reading chord changes as words landing on the beat. Five is what the file is.

**Part of that drop is a proxy rather than a judgement.** Lyric quantity is measured in syllables,
and a line-timed file has one syllable per line however many words are on it — so 44 full lines
count as 44, where the same words timed syllable by syllable count as several hundred. That is how
every line-level file is scored, and this rule moves these files into that population rather than
changing it. A measure that counted *words* would serve both, and is a decision of its own.

A curated corpus therefore wants a reanalysis, and packages built from an affected file carry a
`<`-prefixed first line until they are rebuilt.

## A harmonica tab is not a word

**Harmonica play-along files write the hole to play into the lyric events, and the tabs are dropped.**
A tab is a hole number, drawn with `-`, bent with `b` or `'`, overblown with `o`: `6`, `-6`, `5b`.
Left in, the television draws `6Do6you-6wan-6na5feel` and a solo as a line of numbers.

**Two shapes, and each is read from the file's habits rather than from one event.**

- **Stacked:** an event is rows divided by newlines, a tab on one and the syllable on another —
  `6\nDo`, `4\nIt's\n7`. Where at least 5% of a file's lyric events stack a tab over a word, every
  tab row in the file goes, and so does every event that holds only a tab. The share is low because
  a play-along often tabs only its choruses, and the evidence is specific: sampled harp files stack a
  tab over anywhere from 9% to nearly all of their events, and not one event in 21,064 files sampled
  across the rest of the corpus stacks one.
- **A track of tabs:** the words are on one track and a tab for each note on another. A track whose
  lyric events are at least nine tenths tabs, over at least sixteen of them, is dropped when another
  track carries lyrics.

**A bare number is not evidence.** A count-in is `1\n`, and a credit splits a telephone number into
`47` `9996` `90`. Both look like a tab beside an empty row, so a stacked event counts only when its
other row has a letter in it. The telephone number is on the same track as the credit, so it never
makes a track of tabs.

**In a stacked file a newline divides rows and never ends a line.** An event of two empty rows sits in
the middle of a word as readily as between two phrases, so the lines are inferred, as in any file that
marks none.

**A file whose only lyrics are tabs is left alone.** It is an instrumental written for the harmonica,
and there are no words for the tabs to be separated from.

**Held notes stay spelled the way the writer spelled them.** This writer repeats letters across the
notes of one syllable, `an` `n` `d` and `fo` `or`, and no rule tells that apart from ordinary spelling.

**This moves stored suitability.** A tab left in counts as a syllable, and its newlines as the lines
the file placed. The file the rule was written from:

| | lines | syllables | suitability |
|---|---|---|---|
| before | 9 | 128 | 10/10 — lyrics 3 |
| after | 15 | 116 | 9/10 — lyrics 2 |

A curated corpus therefore wants a reanalysis, and packages built from an affected file keep their
tabs until they are rebuilt.

## A declared lyric encoding must name a real one, and packaging is where that is said

**`km-pack check` and the package builder refuse a `lyric_encoding` that names no encoding; playback
does not.** The two have different jobs.

Playback must stay lenient. `TextDecoder::resolve` falls through to ordinary detection when a
declared label is unrecognised, and it has to: refusing to play a song because somebody mistyped a
field in its manifest is a worse failure than singing it in a guessed encoding, and there is a test
pinning that behaviour.

Leniency alone reports nothing: a packager who writes `cp-1252` gets detection's guess silently — no
problem from `km-pack check`, no warning in the builder, nothing in the log — and the symptom is
mojibake on exactly the songs the field was set to fix. The field exists to stop a wrong guess, so a
wrong *field* falling back to the guess is the one outcome it must not have.

**The vocabulary is the WHATWG Encoding Standard's, not the platform's codepage names**, and the
near misses are what people actually type: `cp1252` is a label and `cp-1252` is not; `euc-kr` and
`windows-949` are labels and `cp949` is not. That last one is not hypothetical: `km-song`'s own
documentation offered `cp949` as an example of a valid label.

**A package already in service is unaffected.** At playback a known label is honored and an unknown
one is decoded by detection, either way. What building adds is that it says so.
## A name made of marks is not a name

**A detected title or artist is kept only where two letters or digits are in it.** A corpus writes
something in the title field whether or not whoever typed the file had a title to put there, and what
it writes is a separator row: `====================`, `<>-<>-<>-<>`, `****`, `???`, `-` and `.`.
Taken at face value those become the names of songs, and because they fold to an empty browse key
they sort ahead of every real name — so the first page anybody opens is marks. 600 songs in the
local corpus are named this way.

This is the same class as the padding
[`A control character in the words is padding`](#a-control-character-in-the-words-is-padding-and-none-of-it-reaches-the-screen)
answers, reached by a different door: a string that survives cleaning and still says nothing. It is
refused in the same place, so a MIDI title, a container tag and an ID3 frame are all held to it and
cannot come to disagree about what a name is.

**Two rather than one, because a frame with one character in it is still a frame.** `====== X ======`
and `** 2` name a song no more than `======` does, and both are in the corpus.

**A count of letters, not a ratio of letters to marks, because decoration must cost a name nothing.**
`***** I Love You *****`, `----- TAKE FIVE -----` and `- - -X-FILES- - -` are titles somebody framed,
and a rule weighing marks against letters refuses every one of them — 203 in the local corpus. What
is inside the frame is the whole question, and counting letters cannot see a frame. The count is over
letters and digits in any script, so an accented title and a Japanese one are names.

**A name of a single character is refused**, which is the call the preview filter already makes about
a one-character lyric line: it is an event that escaped rather than a word. Here it takes `1` through
`9`, `A`, `D#` and `*2` — 629 rows of track index and key signature, with no song's name among them.

**Unless that character is a word, which Han, Kana and Hangul make it.** `虹`, `脈` and `비` are
rainbow, pulse and rain, and a script that writes a word in one character cannot be held to a rule
counting two. Seven rows, each a real song. The list of blocks that carry a word is **not** the one
that decides whether a CJK font must be opened: that question is about blocks and takes in the
ideographic punctuation, and an ideographic comma is no more a word than a Latin one is.

**What is refused is recorded nowhere.** A producer credit is kept as information because it is
often the only record of where a file came from, and this is the opposite of that — a row of marks
records nothing, so there is nothing to keep. **The file's own name is untouched**, so a file called
`----.kar` still browses under that: it is the only name that song has, and the alternative is the
blank unclickable row [`A song with no title`](curation.md#a-song-with-no-title) exists to prevent.

**Nothing a person typed is touched**, and no artist is invented. A refused name leaves the song
exactly where a song whose file said nothing already stands.
