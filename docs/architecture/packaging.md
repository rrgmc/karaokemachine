# Packages and the catalog

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## A package is one file

`.kmpkg` is a container of this project's own. `km-kmpkg`'s `container` module writes and reads it,
and nothing else does. It holds `manifest.json`, the MIDI files, and every song's media as **stored**
entries under `media/`. MIDI comes back whole, which is right for a few kilobytes. Media is reached
through an `EntryWindow`: the package's own file handle with an offset and a length. A reader seeks
into the window and never extracts it.

```text
0                header: magic `KMPKG\x1a\0\0`, container version (u16), flags (u32), two zero bytes
16               the entries, back to back
                 the directory: name, method, offset, stored length, real length, crc32
                 footer: directory offset, directory length, `KMPKGEND`
```

**The flags word is read with the header and kept whole.** `Container::open` reads it,
`Package::flags` returns it, and `read_flags` reads it for a file whose manifest does not validate.
A bit this build does not know passes through unchanged. `PackageFlags::names` is the one map from a
bit to a word.

**The directory is the only table**, so no second copy of a length can disagree with it, and there
is nothing to cross-check. A stored entry is a contiguous byte range, and the directory says where it
starts. So a decoder reads the package *as a file* from an offset.

**Opening is two seeks and no search.** The footer is a fixed 24 bytes at a fixed end. A ZIP's
end-of-central-directory record has to be found by a backward scan for a signature, and that
signature can also occur inside stored media. The reader keeps the handle it opened. A window
**has no gap in which the file could be replaced between learning the offset and reading from it**.
The window also outlives everything.

**The refusals are each for a fault that would otherwise be found late**, and they all happen at
`Container::open`. A deflated entry has no byte range to seek into. A stored entry whose two lengths
disagree would make the window read on into whatever follows. And the reader checks every entry
against where the directory begins, rather than against the file's length. An entry reaching *into*
the directory is as wrong as one reaching past the end. Without that check, a directory read into
memory makes a half-copied package open perfectly and **break only when somebody sings.**

**What is read whole is read against a ceiling, and the ceiling is on the bytes rather than on the
claim.** `MAX_MANIFEST_BYTES`, `MAX_SONG_BYTES` and `MAX_GRAPHICS_BYTES` bound the three reads that
put an entry in memory. `read_capped` counts what arrives instead of trusting what the directory
says. `MAX_DIRECTORY_BYTES` bounds the directory itself for the same reason, one step earlier. Three
properties follow, and each is load-bearing:

- **A declared length sizes nothing on its own.** `Vec::with_capacity` on a claimed `2^63` reaches
  `handle_alloc_error`, which **aborts the process and cannot be caught**. So the reader clamps the
  claim to the ceiling before it uses the claim as a hint. It still uses the hint, because the claim
  is right almost always, and one allocation beats growing a buffer twenty times.
- **Deflate reaches about a thousand to one**, so a package inside the upload limit can carry a
  manifest that exhausts memory. The manifest matters most, because the reader inflates it *before
  anything has decided the package is real*. And **every start opens every package in the scanned
  folders**. So one such file otherwise makes a machine that will not boot, rather than a song that
  will not play. The inflating read writes into a sink that refuses to grow past the ceiling. The
  read stops where a decompressor with a plain `Vec` would go on filling it.
- **Reading `limit + 1`** separates a file that exactly fills the ceiling from one that runs past it,
  and it never holds the overrun.

The ceilings are per kind, because a MIDI file and a `.cdg` differ by three orders of magnitude. One
number covering both would have to be the larger of the two.

**`EntryWindow` needs none of this**, and the contrast is the point. It never reads an entry into
memory, so its guard is the size cross-check above rather than a ceiling.

**Both decoders opened their file twice, and the fix makes them simpler than before.** Each probed the
shape, dropped the reader, and opened the path again on the decoder thread. One reader cannot serve
both, and that looked like an obstacle. But the reader types are `Send`, so the decoder opens the file
once, measures it in place, and moves the open thing to the thread.

**The name is load-bearing under custom I/O.** ffmpeg has no filename to look at, so it probes the
container from the hint it is given. That is why entries are `media/<number>.<ext>`, and why a test
pins it. A name with no extension leaves ffmpeg sniffing bytes. For a fragmented MP4 or a short read,
that can decide wrong, and quietly.

**`Seekable<R>` exists because of the orphan rule and nothing else.** The media-source trait is
symphonia's and the window is the package crate's, so neither crate may write the impl. Wrapping a
generic `R` lets a package's bytes reach the decoder **without either crate learning the other's
name**. That keeps `km-cdg` free of the package crate and `serde` on Android. It also keeps
`km-video` the only crate that decodes with ffmpeg.

**There is no size at which an entry needs anything else.** Every offset and length is `u64` in the
directory, on the wire and in the reader. So one video crossing four gibibytes is ordinary, rather
than a threshold that something has to be told about.

**`u64` discipline, for one target.** `armeabi-v7a` is 32-bit and is what the television runs. The
only narrowing is in `read`, where the `min` happens **before** the conversion and never after.
Without the clamp, a reader runs out of its entry and into whatever follows it. **That presents as a
video that plays and then shows garbage.**

**What it costs, and where.** The build pays nothing extra: it reads every video whole and writes it
out again anyway. **The cost is on the *editing* commands.** Correcting one title in a 20 GB package
moves 20 GB. The temp-and-rename means the disk needs **twice** the package's size while it runs.
This is accepted, because it happens on a curation workstation rather than on the machine.

**The manifest is written last, which is what leaves the cheap edit available.** An edit that changes
only the manifest can truncate the file at the manifest's offset. It then re-appends the manifest,
the directory and the footer, and no video moves. It is not built, because the cost falls on a
curation workstation and nobody has yet been waiting on it. But the layout had to be decided in
advance. The first eight bytes identify the file, so the manifest gives up nothing by not being
first.

**Reproducibility rests on one pinned number.** Two builds of the same songs are byte-identical,
because the container records no moment, host or mode. They are also identical because
`DEFLATE_LEVEL` is named rather than taken from `Compression::default()`. A default that moved
underneath the writer would change every package's bytes at once, quietly.

**The `.cdg` is deflated, and the two arguments against deflating it are both wrong.** Both are
worth stating, because both are the obvious thing to think.

- **The saving is not small.** CD+G measures at **14.7%** over 60 corpus files, a mean of 1.85 MB
  to 272 KB. A real three-song package goes **19.5 MB → 15.0 MB**. The `.cdg` saving across 4,000
  songs is about 6.3 GB.
- **The latency argument runs backwards.** "The machine reads the whole thing at load, on the path
  between pressing a number and hearing something" sounds decisive. But inflating a 1.67 MB `.cdg`
  costs single-digit milliseconds. That measures *below* the cost of spawning the process doing the
  measuring, and it saves ~1.5 MB of reading. Below about 150 MB/s of storage the deflated entry is
  the *faster* one. An SD card and the Android target are both below it.
- **"One rule for every media entry" survives, restated.** The rule is not "media is stored". It is
  "what a decoder seeks into is stored", and the `.cdg` is in the media set only by accident of being
  media. `is_seekable_entry` is that rule, in one function. The writer, `media_entries` and
  `km-pack check` share it, so there is exactly one place to get it wrong.

No converter and no format bump is needed. `graphics_bytes` reads through the directory rather than
a byte window, so it takes either method. A raw copy carries a stored `.cdg` across a rebuild
untouched, rather than imposing the rule afresh.

**Refusing the two superseded format versions needed justifying, not merely doing.** The standing
warning is that anything added to `problems` is a package that cannot be *opened*. The refusal is
allowed on two conditions that a language rule could not meet. **The refused set is finite and
named** rather than open-ended, and **the remedy is one command the message spells out**.

It is a distinct problem type rather than a reuse of "unsupported format", because the two say
opposite things. That one means *I do not know what this is*. This one means the build knows exactly,
and it refuses because nothing reads the shape that the package describes.

**The manifest format is a fact about the manifest and says nothing about the container.** A MIDI-only
package still declares 1, which is what choosing a version by content is for. The number says what a
reader needs to understand the songs. How to find them is the container's own version in the header.

### Nothing in a package records when or where it was built

The container has nowhere to put it. A directory record is a name, a method, two lengths, an offset
and a checksum. It has no date, no host and no file mode. So the rule
[`A package says nothing about the machine that built it`](../decisions/packaging.md) holds by
construction, rather than by a pass that sets fields back to a fixed value.

**A rebuild is the case that proves it.** Copying an entry from another package moves its bytes, its
method, its lengths and its checksum. A record holds nothing else to carry forward or to stamp afresh.
`a_rebuild_of_an_unchanged_package_is_byte_identical_to_it` is the assertion: a rebuild that changed
nothing produces the same file, whenever and wherever it runs.

**A build is reproducible, and it rests on one pinned number.** With `created` pinned in the
description, the same songs produce the same bytes. Entry order is the order they were added. The
manifest's JSON is plain structs in declaration order. And `DEFLATE_LEVEL` is named rather than taken
from `Compression::default()`, because a default that moved underneath the writer would change every
package's bytes at once, quietly. `two_builds_of_the_same_songs_are_byte_identical` is what holds it.

**Integrity is a checksum per entry, and it is not a signature.** The CRC says a package arrived as
it was built. It says nothing about who built it, and anybody editing a package can recompute one.
`km-pack check --verify` reads them. It is a flag rather than part of every check, because it reads
the whole file. Playback does not verify, because a decoder streaming a video cannot.

## The catalog, and why collisions are impossible

The catalog is `rusqlite` with bundled SQLite, so it behaves identically on Android. It holds
`packages` and `songs`, an FTS5 index over title and artist, and direct number lookup for the keypad.

Song numbers are the user's identity for a song, so **two songs must never share one. The catalog
enforces that by shape rather than with a check.** Each package installs into a bank of a thousand,
and `UNIQUE (bank)` gives it one of its own. A number is `bank * 1000 + slot`. Two packages that both
number a song 500 cannot clash, so **the install has no conflict to refuse.**

**`songs` also carries `sort_key` and `sort_artist`, folded from the title and artist by
`km_song::text::fold` at install.** SQLite's `COLLATE NOCASE` is ASCII-only. So ordering by the raw
names put every accented one after `Z`. On a Portuguese corpus, that put a hundred songs past the
end of the alphabet. It did so on the machine's screens and on the online remote it serves.

The fold is stored, because every SQL spelling of it is a second copy of that function's accent
table. Two of those disagreeing looks like bad data. The rationale is `One alphabet, everywhere` in
[`docs/decisions/songs.md`](../decisions/songs.md).

**Which fold wrote them is recorded, in `meta.fold_version`.** It holds
`km_song::text::FOLD_REVISION`. That is one constant, and it lives beside the table it describes.
Both databases that store folded columns read it. So the edit that changes the folding is the edit
that invalidates every key that the old folding wrote.

`prepare_existing` refolds a catalog whose number is behind, and the refold moves
`catalog_version`. Changing the table moves songs that were
already in a believable order, and `km-remote-core` orders by the column it copies.

**A `library.sqlite` that is not the current shape is dropped and rebuilt** from the installed
packages. The rebuild keeps `meta`, so the catalog version goes on counting up and every mirror
fetches again. See `A store opens at its current version or is refused` in
[`docs/decisions/foundations.md`](../decisions/foundations.md).

**`lyric_preview` is the one manifest field that carries the song itself.** Four notes on the
plumbing:

- **It did not move the format version**, on the same reasoning. The content chooses the version,
  and nothing in the manifest denies unknown fields, so an older reader ignores the key. Tests cover
  both directions: a new manifest read through a struct with the field removed, and an old one read
  through the new struct.
- **The catalog holds it as one nullable column**, newline-joined, because a preview line cannot
  contain a newline. It is **not** in the FTS index. FTS5 cannot `ALTER`, so adding a column means
  dropping and rebuilding, and searching the words is a different feature.
- **`serde(default)` on the DTO is an obligation, not a habit.** The offline remote also deserializes
  the export into that struct, and the client fails a whole page on one unparsable row. Without the
  default, an updated phone would refuse every row from a machine that had not been updated.
- **A package gains one only by being rebuilt.** Reinstalling an old package cannot conjure words its
  manifest does not contain.

**`tags` copies all four of those and diverges on one**, and the one is the interesting half.

- `Vec<String>` with `serde(default, skip_serializing_if)`, no `FORMAT_VERSION` bump, and an untagged
  package byte-identical to what an earlier build wrote. All of that is for `lyric_preview`'s
  reasons, and all of it is under test.
- **Unlike it, `tags` is in `fields` and in `inherit_edits_from`.** A preview is *detected*: a rebuild
  re-derives it from the same bytes, so there is nothing for a person to correct and nothing to
  inherit. Nothing anywhere detects a tag, so a rebuild from source has nothing to re-derive. A tag
  left out of that list would sit in the manifest and vanish at the next rebuild. It would vanish
  quietly, and only from the songs somebody had bothered to curate.
- **The catalog holds it twice, and only the packed column is the truth.** `songs.tags` is the sorted
  comma-joined value. A tag cannot contain a comma, so one column is enough, which is
  `lyric_preview`'s own argument about newlines. It is in `SONG_COLUMNS`, and that is the load-bearing
  part. `package_digest` selects through that list, so a package rebuilt with nothing changed but its
  tags moves `catalog_version`, and every mirror refetches.
- `song_tags` beside it is a derived index in the sense that `songs_fts` is one. So the tag filter and
  the vocabulary list are seeks, rather than a scan of a six-figure catalog. `Library::install` fills
  it, rather than a trigger, because a trigger cannot split a string without a recursive CTE.
  **A join table alone would have left the digest identical.** Every phone in the house would then
  keep stale tags for ever, with nothing reporting a fault. A test therefore fails when the column is
  taken out of the list.

**`loudness` copies `lyric_preview` exactly, and diverges on where it can be filled from.** The
product decision is
[`Video and MP3+G play at the MIDI reference level`](../decisions/audio.md#video-and-mp3g-play-at-the-midi-reference-level);
the plumbing is what follows.

- `Option<LoudnessRecord>`, holding `lufs` and a `peak_dbtp`, with `serde(default, skip_serializing_if)`
  and no `FORMAT_VERSION` bump. A package without one is byte-identical to what an earlier build
  wrote. **It is a wire type of its own rather than `km_loudness::Loudness`**, on this module's
  standing rule. A manifest that reused an analysis type would turn a change in measuring code into a
  format change.
- **Detected, so it is in neither `fields` nor `inherit_edits_from`.** That is `lyric_preview`'s side
  of the divergence above, not `tags`'. A rebuild *replaces* a measurement, which is what should
  happen to one. It reads like the defect that paragraph warns about, so the field's own comment says
  which it is.
- **The peak is carried and never read.** Levelling only attenuates, so nothing needs a peak to know
  a gain is safe. It is four bytes a song. It is also the only number that could answer whether a
  *quiet* song could safely be raised. Having measured it, that question can be settled from the
  packages people already have.
- **The catalog holds the loudness only**, as a nullable `REAL` in `SONG_COLUMNS`. Being in that list
  is required rather than chosen: `CatalogSong` is read through it, and the machine reads the value
  at song start.
- **What it costs is the `tags` consequence in reverse.** `package_digest` selects through the same
  list, so a package re-analysed for nothing but its levels moves `catalog_version`, and every mirror
  refetches. That is the honest answer rather than a price to dodge, because the package really did
  change. A second list could keep it out of the digest. That list would be a second thing to keep in
  step, only to tell a mirror less than the truth.
- The peak stays out of the catalog entirely, since nothing on the machine reads it.
- **A package gains one by being rebuilt *or* re-analysed**, which is the one place this is easier
  than a preview. `km-pack reanalyze` reads the media through a window into the package, the same
  bytes that playback reads. So nobody needs the sources back to gain levels, and no media is
  re-encoded. A re-analysed package comes out the same size as a freshly built one, carrying the same
  numbers. Both `km-cdg` and `km-video` hold a test that their reader and path forms measure a file
  identically. That test makes the command a shortcut rather than a second answer.

## Curation: how packages are really meant to be made

Scanning a folder is a **bootstrap**, not the workflow. A person curates real packages by hand, and
the corpus makes the reason obvious. Detection puts a studio's name where a title belongs, leaves the
artist empty on most files, and yields titles truncated from a filename.

So **detected metadata is a suggestion, never the final word**, and the format treats it that way. An
`edited` list names the fields a person set by hand. That list makes corrections durable across
re-analysis and rebuilds.

### The description's language reached only one of the three song kinds

**The shape of mistake this invites.** A description carries a per-song `language`.

Suppose `VideoFields` and `CdgFields` lack that field. Then `entry_from_video` and `entry_from_cdg`
write `None`, however specific the description was. `settle_languages` then fills the package's
`default_language` over the top. The build reports `accepted 9`, and the catalog is wrong. A
description naming `ko` for one song and `en` for six produces seven songs filed under `und`.

Three things make that kind of fault last:

- **The kind that works is the kind that is tested.** `apply_edits` lays a MIDI entry over a fresh
  parse, and it carries the field. Video and MP3+G bypass that path entirely. Each builds its entry
  from a plain struct of chosen values.
- **It is invisible by construction.** There is nothing to compare against. No video container and no
  MP3+G pair states the language of the singing. So a wrong answer looks exactly like the honest
  "nobody said" that the default exists to cover.
- **The `--index` CSV goes inert with it**, because it feeds the same `SpecSong.language`. A
  spreadsheet's language column appears to work as long as the corpus it is written for is all
  `.kar`.

A person saying so is the *only* source of a language for these two kinds. `@LENGL` makes most `.kar`
files claim English, so nothing may infer it from a file. The value is threaded through both requests
and both field structs. `settle_languages` fills only what is genuinely unstated. The regression test
asserts the boring direction, that a named language survives, because the failure is a silent wrong
answer rather than an error.

**`build` takes no folder at all.** `spec <folder>` walks it and records every decision that a build
would otherwise make in silence. The decisions are which files are songs, what each is called, what
number it gets, and which copies are duplicates. `build <spec>` does what the file says.

The selection flags moved to `spec`, **because selecting is what a description records**. Seeding corrections out of an existing package
is `spec --from` rather than a merge flag. The matching is identical, by content hash. The difference
is that **the corrections land in a file somebody can read**, rather than travelling invisibly into a
package.

**One module decides what a package contains.** The curation tool calls it with a description that it
builds in memory from its own database. Shared per-song primitives do **not** buy the claim "The two
tools cannot produce different packages from the same songs". Numbering, duplicate detection, ordering
and the language gate all live above them, and two loops kept in step by hand give two answers.

**Provenance is derived rather than stored**, and this is the part to understand before changing
anything here. **The description has no `edited:` key.** The build constructs the entry a fresh parse
implies, then lays the description's values over it. It marks a field only where the two disagree.
There are three consequences:

- A generated description says back what detection found. So building it marks nothing, and
  regenerating is safe to repeat.
- Correcting a title in the file gets the flag, with no second key to remember.
- `edited` becomes **a fact about a disagreement rather than a claim somebody has to keep true**. An
  `edited:` list can lie about itself, and a comparison cannot.

The one cost is a source file changed after its description was written. It compares stale against
fresh and marks a correction nobody made. **That errs towards keeping what a person reviewed**, which
is the right direction for it to err.

### A marker over a value a fresh parse would produce anyway

`lyrics_hidden` is the one field where the comparison above is not enough on its own. Know why before
adding another like it. The field is a boolean with a measured half: a fresh parse sets it from the
three faults that `Suitability::words_cannot_be_followed` names. So *hide these words* on a file the
measurement was content with marks itself, as a corrected title does. But *draw these words* on a
file the measurement silences writes `false`. An untouched song already carries that value, so a
comparison sees no disagreement at the next rebuild.

What closes it is that the builder holds three states where the manifest holds two. Its column is
nullable: null is nobody has said, and a stored `false` is somebody's answer. `spec_for` carries that
`Option<bool>` into the description. `apply_edits` compares it against the measurement rather than
against nothing, and the marker lands because the two genuinely disagree. **A build from a
description with the key absent is what re-derives the measurement.** So the key is written only
where somebody spoke.

The field does not move `FORMAT_VERSION`, on `tags`' and `loudness`' terms. `skip_serializing_if`
leaves the key out of a package whose songs all draw their words. Such a package is byte-identical to
what a build predating the field wrote. The field *is* in `km-catalog`'s `SONG_COLUMNS`, because the
machine reads it at song start. That puts it in `package_digest`, so a package rebuilt for nothing but
this moves `catalog_version`, and every mirror re-reads once. That is the honest price: what a song
puts on a television is as much part of the package as how loud it is.

**The preview travels with the words.** `lyric_preview` is the words on every surface that the
television is not: the song book, the remote and the export. So the build writes none for a song
whose words are withheld, in `preview_for` and in `inherit_edits_from`. One rule at the point the
flag is settled keeps every reader downstream from needing a rule of its own.

**Progress is an event handed to a callback rather than a shared struct.** The three consumers want
different things, and a progress struct would make the command line poll its own atomics in order to
print. The callback returns a control flow checked *between songs*. That lets a web UI cancel without
waiting out an ffmpeg re-encode. The package write raises an event of its own. Otherwise a
four-thousand-song package is a silent minute that looks like a hang at 100%.

**One YAML wart worth knowing.** `no` is a language code **and** `false` to any YAML 1.1 reader. Our
own reader follows 1.2 and is safe either way. But the file exists to be opened by people and run
through other tools, so the writer puts the quotes back on the way out. It is one code and not a
general escaper, because **one code is the whole problem**. Of YAML 1.1's boolean spellings, `no` is
the only one that is also an ISO 639-1 language.

## One bug that predates all of this

Four commands called `read_song` for every entry. That fails for anything whose media lived beside
the package. So **all four had failed outright on any package containing a video since videos
existed**, and nothing noticed, because nobody had run them on one. A third song kind did not cause
this. **It doubled the population who would meet it, which is how it was found.**
