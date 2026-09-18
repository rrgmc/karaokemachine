# The printed song book

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

A commercial home karaoke machine ships with a ring binder. This is that binder: every installed song
on paper, five columns wide, so a room full of people can find something without queueing at the one
screen.

**The reference was measured, not imagined** — a real machine's book, A4 portrait, 233 pages for
11,999 songs, so 52 rows a page at about 7pt, with artist, code, title and the first line of the
words. Every one of those columns already existed here, which is what made this a small piece of work
rather than a large one.

## Why there is no PDF crate

**The base-14 fonts.** A conforming reader supplies the glyphs *and the metrics* for `/Helvetica`
itself, so a font object is four keys and no stream: no `/FontDescriptor`, no `/Widths`, no
`/FontFile`, and therefore no font parsing, no glyph subsetting and no CID machinery — which is where
the bulk of any PDF library lives. What is left is a page tree, a content stream and a cross-reference
table, in about 180 lines. `km-songbook` takes `km-songcode` and **no external crate at all**.

**A test asserts `/FontFile`, `/FontDescriptor` and `/Widths` never appear** — the negative assertion
is what proves the claim, since everything else would still work if a font crept in.

Three things in the writer worth not rediscovering:

- **Offsets are recorded, never revised.** The offset is stored immediately before the object is
  written and nothing patches the file afterwards, which removes the whole class of fault where an
  offset was correct at the moment it was taken.
- **Xref records are exactly twenty bytes**, trailing space included. Readers are entitled to seek by
  multiplying, so nineteen or twenty-one produces a file that opens in a forgiving viewer and fails in
  a strict one.
- **A `q`/`Q` pair restores the text state**, so a rule drawn between two rows resets the font and the
  next row must re-emit it. Without that, rows after a rule draw in the reader's default font — and
  only on pages that happen to carry one.

**Streams are uncompressed**, which is a dependency decision: `flate2`
reaches this workspace only through `zip` and no member names it directly, so compression would mean a
new workspace dependency. About 3 MB for twelve thousand songs against the reference's own 6.7 MB.

## Measuring without a font file

Two 256-entry width tables from Adobe's AFM files, **indexed by the WinAnsi byte** so measuring and
drawing use one index and cannot disagree. The AFM is keyed by glyph *name* against StandardEncoding,
so building the tables was a transposition — **the step that goes subtly wrong and is impossible to
notice** — and the module header writes that mapping down.

Anchor widths are asserted, and so is the invariant the code column rests on: **every digit is 556 in
both faces**, which is what lets a right-aligned number line up without zero-padding.

**The WinAnsi bargain, said out loud.** ASCII and Latin-1 pass through, twenty-seven typographic
characters live in the band Latin-1 leaves as controls, Latin letters cp1252 lacks are transliterated,
and everything else becomes `?` **and is counted**. The count rides out of *pagination* rather than
rendering — encoding happens there because measuring is what truncation needs — so the command line
can report the cost and the book can note it on its own first page before a byte is drawn. The
transliteration table is strictly what cp1252 *lacks*, which is why it is thirty rows and not three
hundred.

## The layout, and the slot model

**Every page holds 52 rows, the first page of a section included** — asserted exactly, because that
arithmetic is the thing most likely to drift. A heading in the body would cost two of them on the page
a section starts on, running 50, 52, 52…; as a running header in the masthead it costs no body row
anywhere, which is worth two pages in a book of two hundred.

A section always starts a new page: it restarts the alphabet, and two alphabets on one sheet is a page
somebody reads as one run and gets wrong. That is also what makes the running header honest — no page
holds two languages, so the one in the corner is true of every row under it.

**Every column's `x` is its left edge**, including the code column, which is the one that invites a
right-aligned `x`. Four columns measured one way and one the other leaves the fifth column's left edge
existing nowhere in the data — liveable while the only thing drawn is text, and untenable the moment a
vertical rule has to be placed.

**Every cell is padded off its rules by half a gutter** — and the first cut got the two outer columns
wrong, **and the mistake is structurally invisible until something is
drawn**. The outer rules were placed at the margins, which are the first column's own left edge and
the last column's own right edge, so the artist name was set *on* the line while the columns between
cleared theirs. One cause, two symptoms, neither detectable while the only thing on the page was text.

Two departures from the reference, both deliberate. **A repeated artist is blanked but reprinted at
the top of every page**, because a page is what somebody reads on its own. And **a song with no artist
prints an em dash**: a blank in that column already means "the same as above", so a blank there would
file an unattributed song under whoever preceded it — a wrong answer rather than a missing one.

## What the reference actually draws

Making it a ruled table meant reading the reference's own content stream rather than looking at a
picture of it, and three of these are not what you would guess:

- it draws **thin filled rectangles, not strokes** — a word processor's idiom, and the reason a naive
  search for a stroke operator finds nothing;
- a horizontal under *every* row, so every cell is boxed, and the column-heading row is **repeated and
  boxed on every page**;
- the grid **stops at the last row** rather than ruling empty cells down to the bottom margin;
- **one of the six verticals is shorter than the others**, starting below the header row — so even the
  reference draws its verticals as segments rather than full-height lines.

That last point is the one a **section heading** would raise: a band across all four columns needs the
interior verticals stopping above it and resuming below, so that it reads as one merged cell rather
than four empty ones with a word in the first. **There is no band** — the section is a running header
in the masthead — so every vertical runs the full height and every page of the book is ruled
identically, which is what makes the geometry a function of one number. The geometry is still
returned as a list of segments
instead of being drawn, which is what lets the tests read the lines that were asked for rather than
parsing a content stream back out.

**The weight is not copied.** The reference draws 0.72 pt in solid black against 11 pt type; the same
weight against this book's 7 pt would compete with the words instead of framing them.

**One method serves all of it**: a batch of segments stroked as **one** path. That is about file size
and not tidiness — a page of grid is around 58 segments, and a `q`/`Q` pair each would cost roughly
five times the bytes on streams this crate deliberately leaves uncompressed, which over a 233-page
book is more than half a megabyte. The two traps apply unchanged: `q`/`Q` restores the text state, so
the cached font must be invalidated, and it restores the non-stroking color, so the gray must be
re-issued afterwards.

**The grid is emitted before any text on the page.** A content stream has no z-order, so it is drawn
at the top of the stream rather than relying on hairlines being too thin to matter.

## Three strings at the top of the page

A `name` at the left, a `title` in the middle and the **section** at the right, on one baseline, with
the subtitle a line under the third of them.

The first two are separate fields rather than one because they answer different questions: whose
machine the book belongs to, and what the document is. **The case that makes it worth a parameter is a
house with a machine in two rooms**, where two books that look identical are two books somebody mixes
up — which is also why the machine's own name is the *default* for it now, composed as
`KaraokeMachine - Living Room` rather than substituted. That composition happens in
`km_api::book::book_name_for`, called where `?name=` is read rather than inside `style`, so the
composed string reaches `BookQuery::name_tag` and a rename changes the `ETag`.

The third is per page rather than per book, which is the only field here that is: `Page` carries the
section it belongs to, every page of a section carries it, and `chrome` draws it right-aligned.

All three are encoded with every other string, so a name outside cp1252 is transliterated or counted
like anything else rather than being a second encoding path.

Three API decisions that are easy to get wrong:

- **The name goes beside the filter, not as a sixth field on it.** That struct is a filter and every
  field narrows — two of its methods say so, and the download filename reads it for the one segment a
  filter contributes. A name narrows nothing, so putting it there would make two honest methods start
  lying.
- **The `ETag` hashes the name rather than interpolating it.** Every other value reaching that header
  is a closed set, but a name is free text out of a query string and a `"` in one closes the entity tag
  early. **Omitting it is not the alternative**: the body varies with it, which is the cache bug that
  header's own comment was written to prevent.
- **The download filename is `KaraokeMachine - Living Room - Song Book (Portuguese).pdf`**, composed
  in `book_filename` from three sources that are each already decided elsewhere: `book_name_for` for
  the masthead half, so the file is called what the page inside it is called; `named_language` for
  the parenthesis, so a book headed `Português` cannot arrive as `Portuguese`; and the filter for
  whether there is a parenthesis at all. It reads `state.machine_name()` rather than `?name=` — a
  caller renaming one download is titling *that copy*, and keeping caller text out of the header is
  worth more than honouring the override twice.

  **A machine name is the first free text an owner types that reaches a header**, and a `HeaderValue`
  is visible ASCII, so `Salão` would have been a 500 with nothing in the log to explain it. The
  header is therefore RFC 6266's two-parameter form — an underscored ASCII rendering in `filename`,
  the real one percent-encoded per RFC 8187 in `filename*`. The encoder is fourteen lines in
  `handlers.rs` rather than a dependency, for the reason `km-songbook` takes none.

## Two adapters, one ordering

`arrange` groups and sorts; it does **not** fold. The fold has to be `km_song::text::fold` — the one
the catalog's FTS5 configuration implies, so the book's alphabet and the search box's are the same
alphabet — and that crate drags three parsing dependencies. So **the caller folds and `arrange`
sorts**, which is also what lets one ordering serve both sources: the catalog (where both callers
can see it, so there is one keyset paging loop rather than two) and a stack of manifests.

The manifest adapter holds the **manifest** rather than the package, because a book is entirely
metadata and never opens a song's bytes: no file handle across a build, no `video` feature, no ffmpeg,
and tests that need no package on disk.

**Sections are keyed by the raw language string and only *named* through the language table**, so the
several spellings a real corpus carries print as themselves rather than merging into each other or
into the unclassified bucket. Sections sort alphabetically by heading, **not** by size: the catalog
orders languages by count, which is right for a picker and wrong for a book somebody flips through.

**`?tags=` narrows which songs go in and never sections the book**, and both adapters drop the
non-matching rows before `arrange` sees them. Sectioning by tag was considered and is not available:
sections need an order and a heading, and a language has both — a table with a name per row — where an
open vocabulary has neither. A song carrying three tags would also have to appear in three sections or
arbitrarily in one. The OR is the same one every other surface makes, so a book printed from
`?tags=rock,brasil` holds exactly the songs a remote showing that filter lists. Both adapters check
the list is not empty before asking, because *any of none* is none where *all of none* is everything.

## What is deliberately not built

- **A five-column catalog query.** Collecting walks the export and holds the book in memory — about
  200 bytes a row, so a few megabytes at twelve thousand songs, which is inherent to a book. A query
  would be the answer at six figures, and would also mean a catalog method and a test double for it.
- **A book from the offline remote's mirror.** Different schema, different sort key, so a second
  adapter to keep in step — and that app exists for browsing with the machine switched off, which a
  printed book is the *alternative* to rather than an instance of.


## The book's own words

**`km-songbook` holds none of them and gained no dependency.** `BookStyle` was always a struct the
caller fills, so translating the book was a caller that fills it from a catalog instead of from
`Default` — the crate that draws rectangles goes on taking `km-songcode` and nothing else.

The catalog is `km-api/i18n/`, because that is where the book's English already lived. It took
`UNCLASSIFIED` with it: that constant was written out character for character in `km-api` and again
in `km-pack`, which is the "one sentence, two spellings" fault `Both roads to an install say the same
sentence` names. `km-pack` is a command line and out of the translated scope, so its copy stays and
says so.

**Sections order by a folded heading.** `Entry::section_sort`, folded by the caller with
`km_song::text::fold`, for exactly the reason `SortKey` already is. A byte compare was correct only
while every heading was an English language name; `Índico` and `Português` are headings a reader
expects between `Alemão` and `Polonês`, and this was the one place in the product nothing was folding.

**Five language names are translated and 181 are not.** The catalog carries the handful a real machine
has packages in; anything else falls back to `km_kmpkg::Language::name()`, which is the answer a code
the ISO table does not know already gets — shown as itself rather than dropped.

**`?locale=` varies the body, so it is in the `ETag`.** It is the parameter that changes no song in the
book and every word around them, which makes it the easy one to forget; two books of identical rows
with `ARTIST` and `ARTISTA` at the top are different bodies.

**cp1252 is the ceiling and always was.** A test asserts every message in every catalog survives
`winansi::encoded` without a replacement, so a translation can never trip the counter the book reports
on its own first page.
