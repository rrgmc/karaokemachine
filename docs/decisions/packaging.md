# Packages and song numbers

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## A package is a document too

**A package is described by a YAML file, and `km-pack build` builds from one and from nothing else.**
`km-pack spec <folder>` writes it, a person edits it, `build` consumes it. Packaging a folder
directly can correct nothing and cannot say *which files are in the package*; the curation database
describes one and is hundreds of megabytes of SQLite, and the built `.kmpkg` describes one and is the
output. The description is the missing middle, and it is text because the point is that it is *edited*
rather than merely opened.

**The provenance is derived, not stored.** There is no `edited:` key: the build constructs the entry
a fresh parse of each file implies and lays the description's values over it through `apply_edits`,
marking a field only where the two disagree. So a generated description says back what detection
found and marks nothing, a person who corrects a title gets the flag for free, and `edited` is a
fact about a disagreement rather than a claim somebody maintains. **A stored list can lie about
itself and a comparison cannot.**

**The selection flags live on `spec`** — `--min-suitability`, `--require-lyrics`, `--limit`, `--index`
— because they decide what is *in* the package, and a file they exclude simply has no row.
`spec --from <existing.kmpkg>` seeds the hand-edited fields into a description rather than carrying
them invisibly.

**The curation tool builds through the same description**, in memory, so the two tools cannot produce
different packages from the same songs.

YAML rather than JSON because a person edits it and JSON has no comments. Every written file opens
with a header saying how to build it and warning about the two values YAML reads as something else,
one of which is that `no` is Norwegian.

## A package can write a listing beside itself

**A build can put a plain-text page next to the `.kmpkg` saying what is in it**, at the package's own
name with `.txt` on the end. Answering *what songs are in this?* otherwise means having `km-pack` and
knowing to run it, and the person most likely to ask is the one who was handed the package and has
neither.

**The description is the editable middle and this is the readable end.** The YAML is what a build
*reads*; the listing is generated from the manifest and nothing ever reads it back, so a build
overwrites whatever is there and editing one changes nothing. Two text files beside a package would
be confusing if either could be mistaken for the other, which is why this one is a page of prose
rather than a second syntax.

**It says what the manifest says and nothing else**: the package's name, version, publisher, id and
creation moment, then a line per song with the number, title, artist, length and language. No source
path, no output folder, no host, no build clock. This file travels to a stranger exactly as the
archive does, so [`A package says nothing about the machine that built it`](#a-package-says-nothing-about-the-machine-that-built-it)
reaches it unchanged, and reading every value off the manifest is what makes that structural rather
than remembered.

**Off at the command line and on in the curation tool**, which is the one place the two front ends
differ about a build on purpose. A description is built from over and over while it is being edited,
and a second file rewritten each time is noise in the folder. A package built in the curation tool is
built to be handed over, and what it is handed over with is the page saying what is in it.

**Songs in number order**, which is the order a singer meets them in and the order the book prints,
rather than the order they went into the archive.

## A package is not an archive a file manager can open

**A `.kmpkg` is a container of this project's own: magic bytes, the entries, a directory, a footer.**
Double-clicking one opens nothing, and no unarchiver offers to take it apart.

**A package is the one thing built here that is handed to a stranger**, and a file that opens as a
folder is a file somebody edits. Renaming a song, swapping an audio file for another, deleting the
manifest to see what happens: each is a click away in a format every desktop already knows, and each
produces a package that is still a package and no longer says what it holds. A commercial machine's
songs do not arrive as a folder, and these should not either.

**It is not protection, and nothing here should ever be described as protection.** The directory is
in clear, the names are readable, and anybody who means to read a package will read one. What the
format buys is that nobody does it *by accident*. There is no encryption, no scrambling and no key,
because a key that ships inside the player protects nothing and would only make a damaged package
harder to diagnose.

**The shape is what the seeking-in-place design already required**, so it costs nothing to hold:

- **The directory is the only table**, so no second copy of a length can disagree with the first.
- **The footer is a fixed size at a fixed end**, so opening is two seeks and no search, and a file
  that lost its tail is refused rather than found by a signature that also occurs inside stored media.
- **The manifest is written last**, so correcting a title can be a truncation and a re-append.
- **Every offset and length is 64-bit**, so one video crossing four gibibytes is ordinary rather than
  a threshold something has to be told about.
- **A package records no moment, no host and no file mode**, because the container has nowhere to put
  one. [`A package says nothing about the machine that built it`](#a-package-says-nothing-about-the-machine-that-built-it)
  therefore holds by construction.
- **Each entry carries a checksum**, so a package damaged in transit is diagnosed by
  `km-pack check --verify` rather than played wrong.

**What it gives up is that `unzip -l` no longer answers *what is in this?*** That question is
answered by `km-pack describe`, and for somebody who was handed a package and has no tools, by
[the listing beside it](#a-package-can-write-a-listing-beside-itself). The first eight bytes still
say what the file is, which is the rest of what being openable was worth.

**A package opens only at a manifest format this build writes — 1, 4 or 5 — and any other is
refused, naming the formats it reads.** A file that does not begin with the package magic is *not a
package*. Neither answer tries to recognise an older shape, by the rule in
[`A store opens at its current version or is refused`](foundations.md#a-store-opens-at-its-current-version-or-is-refused):
the description a package was built from builds it again.

## Where a song's media lives

**Inside the `.kmpkg`, as an uncompressed entry the decoder seeks into. A package is one file.**

A stored entry is a byte range and the directory says where it begins, so a video is played by
opening the package once to learn `(start, len)`, dropping it, and reading the package **as a file**
from that offset. Nothing is extracted and nothing is held. `ffmpeg-next` takes a `Read + Seek`
through `StreamIo::from_read_seek`, and symphonia's `MediaSource` is that plus `Sync`; both are safe,
public and already in the tree, so this costs no new dependency, no `unsafe` and no ffmpeg rebuild —
which matters, because ffmpeg's own `subfile:` protocol is absent from the Android build.

**What it buys is one file to move, to send, to install and to lose.** There is no pairing rule left
to get wrong: no folder to leave behind on a stick, no `--include='*.media/**'` in a deploy script,
and `missing_entries` is one set lookup instead of a filesystem walk.

**What it costs:** `km-pack apply`, `edit` and `reanalyze` rewrite the package, so correcting one
title in a 20 GB package moves 20 GB. It is a byte copy — no decode, no re-encode, flat memory, and
the checksum carried across rather than recomputed — it happens on a curation workstation rather than
on the machine, and the temp-and-rename means the disk needs twice the package's size while it runs.

**`--dry-run` is on all five commands that write one** — `spec`, `build`, `apply`, `edit`,
`reanalyze` — so somebody can see that a correction changes one field before spending a 20 GB copy
on it. `edit` is the command most likely to be run on a hunch.

Old formats are **refused, and there is no converter**: formats 2 and 3 name themselves and say to
rebuild from the description, and so does a package written into the older container. A manifest
format number alone never stops one opening — its id still has to satisfy
[`A package's id is generated, not typed`](#a-packages-id-is-generated-not-typed), which is a rule
about the manifest rather than about the container, and which a package of any version is held to.

## Which entries are compressed

**An entry a decoder *seeks into* is stored; everything else is deflated.** That is the video and the
MP3+G song's audio stored, and the manifest, the MIDI and the `.cdg` deflated.

- **The saving is large and a person can see it.** Across 60 corpus files CD+G deflates to **14.7%** —
  mean 1.85 MB down to 272 KB. On a real three-song package the whole `.kmpkg` went from **19.5 MB to
  15.0 MB, 23% smaller**, with the MP3s untouched beside it. That is roughly a quarter off an MP3+G
  library of any size, and nearer 6.3 GB across this corpus.
- **It is not a latency trade.** Inflating a 1.67 MB `.cdg` measured *below the cost of spawning the
  process that did it* — 72.5 ms against a 77.7 ms spawn-only baseline — while removing ~1.5 MB from
  the read. On anything slower than about 150 MB/s, which is an SD card, the Android target or a
  spinning disk, **the deflated entry reaches the screen sooner**.
- **It is one rule and it is checkable.** `is_seekable_entry` is the single place the distinction is
  made, shared by the writer, `Package::media_entries` and `km-pack check`, so the three cannot drift.
  A compressed `.mp4` or `.mp3` is a fault that fails the run.

**A package holding a stored `.cdg` still reads.** `Package::graphics_bytes` reads the entry through
the directory rather than a byte window, so it takes either method, and a rebuild carries across
whatever it found rather than imposing the rule afresh.

**What stays stored is not negotiable:** a compressed entry has no byte range to seek into.
Compressing video or MP3 would break playback outright rather than merely cost something — and would
save nothing anyway, both being entropy-coded already.

## A package says nothing about the machine that built it

**A package records what the songs are and who published them. It records no build time, no operating
system, no path, and no name taken from a folder.** It is the one thing built here that is handed to a
stranger, so it is held to what `CLAUDE.md` asks of a tracked file, pointed outward. Nothing
mechanical enforces that half: `tools/dev/check-no-local-refs.sh` reads `git ls-files` and skips
binaries, so it can see neither a package nor anything else a build writes.

**The container has nowhere to put any of it.** A directory record is a name, a method, two lengths,
an offset and a checksum, so there is no date field to pin, no host byte to overwrite and no mode to
agree about. A rebuild has nothing to carry forward and nothing to stamp afresh, and the test that
says so is that rebuilding a package without changing it reproduces it byte for byte.

**A package's id is generated**, so it cannot carry the name of the folder somebody scanned. That has
a section of its own, because an id collision is a worse failure than a leak.

**What a package does still say, and why each is a statement rather than a leak:**

- **`publisher` and `name`** are typed by a person who means them.
- **`created`** is the build moment, UTC to the second, so it says *when* and not *where*. It is the
  one field a person may pin in the description, and pinning it makes a build reproducible: the same
  songs then produce the same bytes on any machine on any day, which is the only way to check a
  package against the description it claims to come from.
- **A song's title falls back to its source file's stem.** Most of a real corpus carries no title meta
  event, so the alternative is not a tidier package but tens of thousands of unnamed songs. It is song
  data a curator sees in the description and corrects — see
  [`Where an MP3+G song's title and artist come from`](song-sources.md) — where a path or a host is
  something nobody chose and nobody can see.
- **Media carries its own tags.** ID3 frames, MP4 atoms and MIDI text events go in byte for byte.
  They belong to the files' publishers rather than to the person packaging them, and the stored bytes
  being the source's exact bytes is what `content_hash` means. A `lyric_preview` is the exception,
  and it is redacted for the *file's* author rather than for the packager's sake.

**The id is the opposite case and is refused**, because there the fault is one the machine cannot work
around.

## Where packages live

**A `packages` folder in the data directory, beside `library.sqlite`, scanned at every start.** Drop a
package in and the songs are there.

**Not under `assets/`**, despite that being the obvious parallel with the wallpapers and the
SoundFont: that tree ships with the build and is read-only where it counts — root-owned under `/opt`
on Debian, inside a signed bundle on macOS, unpacked from the APK on Android — whereas packages are
the owner's own files, they accumulate, and `km-catalog` reads each one *in place*.

**Resolved by rule and never by a setting.** A second way to say where packages are is a second thing
that can disagree with the paths the catalog has already stored. `package_dirs` names somewhere else
and is **folder-granular on purpose**: a folder is stable, and a package taken out of one simply stops
being installed, where a named file renamed inside its own folder becomes a failure somebody has to go
and clear. `debug.packages` is what remains for naming a single file, and it lives behind `debug.`
precisely because that fragility is what it is for.

**On Android there are two such folders, and that is a second *rule* rather than a first setting.**
The private directory is joined by the app's *external* files directory, scanned second so nothing
dropped onto shared storage can displace what the machine already installed. Nothing in
`settings.json` names it and it cannot be pointed anywhere else.

`getExternalFilesDir` is chosen over two alternatives on the same grounds: `MANAGE_EXTERNAL_STORAGE`
needs an *All files access* prompt, hostile on a television and refused outright on some builds, and
the Storage Access Framework returns a content URI that every path in the application would have to
learn about. This one needs **no permission on any API level** and is an ordinary path.

**Android 11 closed `/Android/data/` to other applications**, so a third-party file manager cannot
browse into it, the stock Files app declines, and MTP over USB generally hides it. What survives is
`adb push`, which the Android TV research note itself calls *"a test convenience, not a product
answer"*. The **choice** stands, because the alternatives are worse and this is the only writable
folder needing no permission; making that folder reachable by somebody holding a remote is unfinished
business.

**The folders are the truth.** A scan says what is installed, at every pass, and the catalog is
reconciled against what the scan found — no list of individual paths anywhere.

**The uninstall route deletes the file**, rather than adding it to an ignore list: a folder scanned
at every start would put back anything merely marked, and a deleted file cannot come back. The
sharpness is answered rather than waved away — removing a package is **behind the admin password**,
the deletion is logged at `warn` because it is the only account anywhere of a destroyed file, and a
package reached through `debug.packages` is refused outright, because a file the owner keeps
somewhere of their own is not the machine's to remove.

Both pages that offer to uninstall ask first, and `PackageDto::removable` lets a page leave the
control off a package the machine would refuse. The route itself does not ask: a 200 is a 200, and the
page is where the person is. See
[`A page asks before it deletes a file; the API does not`](api-and-network.md#a-page-asks-before-it-deletes-a-file-the-api-does-not).

## What an installed package file is called

**`<name>-<id>.kmpkg`, from the manifest, and never the name the file arrived under.** A package
handed to the machine is copied in under a name the machine derives — `PackageMeta::file_stem`, one
answer for every route in.

**The id is the half that has to be there.** An install is keyed on it, so a rebuilt package must
land on the file it replaces however the sender named it — and a name built from the *sender's* file
name cannot do that. It used to work by accident: the builder wrote `<id>.kmpkg`, so the file that
arrived was already named the way the machine would have named it. A name carrying anything else,
the version most obviously, ends that at once.

**And what it ends is not untidiness.** Two files of one package in one folder is a scan that sorts by
file name and keeps the first id it meets, so `brasil1-1.0.0.kmpkg` beats `brasil1-1.0.1.kmpkg` and
the machine serves the older build for ever, saying nothing. A package the owner has just rebuilt and
installed, going on playing what it held last month, is the kind of wrong nobody thinks to look for.

**The name is the half a person needs**, because `F10` opens that folder for somebody to look at, and
sixteen hexadecimal characters are not something to pick a volume out of a shelf by. It is slugged
through the fold `km_kmpkg::name_slug` holds, and a name that folds away to nothing leaves the id
alone rather than nothing at all.

**No version in it, deliberately** — the opposite of the file the builder writes, and the two answer
different questions. A curator wants to hold two builds and tell them apart; a machine holds one
package and wants the newer build to take the older one's place. See
[`What the tool calls the package it writes`](curation.md#what-the-tool-calls-the-package-it-writes).

**A file of the same package left in the write folder is deleted, and logged at `warn`.** The derived
name means the ordinary case never reaches this: a rebuild lands on its predecessor and there is
nothing left over. What is left over is a file from before this rule, or one delivered under a name of
somebody else's choosing — and leaving it is the two-files failure above. The `warn` is the same
account the uninstall route keeps, and for the same reason: this is the machine destroying a file.

**What the sweep may take is narrow**, and each bound is a rule already in force. Only the folder the
machine *writes* to, which is one it owns — never a `package_dirs` folder, which is somewhere the
owner keeps their own files. Never a file named in `debug.packages`, which is the one place an owner
names a single file and is not the machine's to remove. And never a file that will not open, which is
a standing fault the Problems tab reports and which taking away would hide.

**A package already sitting in a scanned folder is installed where it lies, under whatever it is
called.** Copying it would leave two files of one package, and renaming somebody's file where it
lies is not what taking a package in means.

**The limit, said rather than hidden:** a file copied into the folder by hand reaches none of this. Two
files of one package can still sit there, and the scan will still take the one that sorts first. That
is the owner's own doing with their own file manager, and the answer to it is the folder being theirs
to look in.

## A rescan picks up what was dropped in

**`POST /api/v1/packages/rescan`, and `Ctrl+F10` at the machine, read the packages folders again and
make the catalog agree with them — no restart.**

**It is one code path with the startup pass**, not a second implementation, so a rescan does what a
restart does. `install_startup_packages` is a wrapper that logs the
report and throws it away.

**The gate is on whether there is anything to *remove*, not on whether the machine is busy.** Three
positions:

- *Refuse while anything is playing or queued.* Rejected: it guts the feature at exactly the moment it
  is wanted. The occasion for a rescan is a party, and a route usable only when a restart would also
  have been fine is not a route.
- *Prune live but protect rows a queued number points at.* The worst of the three: `catalog_version`
  would describe a set that is neither the old one nor the scan, so a mirror downloads a catalog
  containing packages the machine intends to drop, and nothing re-prunes when the queue drains.
- **Split by `doomed`.** Adding always runs, gated on nothing: a new package takes a free bank and
  disturbs no queue. Removing waits for an idle machine, and *only when there is something to remove*.
  Somebody who has just put a file in the folder is never made to wait.

**What deferring costs.** A package whose file was taken away mid-party keeps its rows until the
machine is idle, so its numbers stay dialable and fail at load. That is already what happens when a
file is deleted under a running machine; the rescan declines to fix it live rather than causing it,
and the alternative removes rows a queue entry points at — a singer losing their turn in silence is
worse than a number that says out loud it cannot be opened. The ids come back as `deferred` rather
than being held back quietly, because a client otherwise cannot tell *nothing was missing* from *not
yet*.

**Not a 409 when work is deferred.** The additive half succeeded. And no event is published — a
catalog change is learned from `catalog_version`, which a mirror already polls, and the event stream
is about a performance.

**Behind the admin password**, which needs no migration to reach machines in service: the route is
declared under `/api/v1/admin/` and that is the whole of its permission, so a machine that has been
running for a year is gated exactly as a fresh one is. There is no permission map for an existing
settings file to be out of step with.

**`Ctrl+F10`, beside the key that opens the folder** — `F10` shows you where songs go, `Ctrl+F10` picks
up what you put there. A modifier because there is no bare function key left. The pairing is the
justification rather than the digit, so if the function row is rearranged this should follow the
folder key. It joins the auto-repeat suppression list for the most expensive reason on it: a held key
would queue thirty rescans, each re-indexing every package, and the cap would then refuse the one the
owner meant.

**The work is handed to the installer's worker**, the same one a dropped file goes to. One queue
rather than two, so a rescan and a drop cannot run at once and each wait on the other's transaction —
and the toast, the queue cap and the started/finished reporting all come for free.

## Where a handed-in file is written is not where it is looked for first

**A content file the machine is *handed* goes to the roomiest writable folder it owns — on Android the
external files directory rather than the private one — while the private folder stays the *first* one
scanned.** Two questions, two answers.

**Why they separate.** Scanning the private folder first is about *precedence*: nothing dropped onto
shared storage can displace what the machine already installed. But Android's private directory is
**internal** storage, the smaller volume on most devices, and a package reaches tens of gigabytes — so
putting a copied-in file there because it happens to be scanned first answers a question about room
with an answer about precedence. `Paths::write_dir` is the second answer; `packages_dirs` keeps the
first.

**A copied package therefore lands in the folder scanned second, and that is correct.** Deduplication
is by package id, so a newly copied package has nothing to collide with; and if the owner later puts
the same package in the private folder by hand, the private copy wins.

**It applies to banks too**, for the sharper version of the same reason: the offered SoundFonts run to
301 MiB, and `Removing a bank` in [`audio.md`](audio.md) records what it cost to put them somewhere an
owner could not reach.

**The write bit is read rather than logged.** Android reports read and write access to shared storage
in one bitmask, and ignoring the write half is harmless for a folder that is only scanned and a fault
for one that is written to. Read and write are kept apart: a folder
mounted read-only still contributes the packages it holds and only stops being where new files go. A
folder that is absent is neither.

**The shipped asset tree is never written to or deleted from.** It is root-owned under `/opt`,
inside a signed bundle whose seal `codesign --verify` checks, and unpacked from the APK — so a write
there fails on three platforms and succeeds misleadingly on the fourth. `Paths::is_mine_to_delete`
is the guard, and it is on the delete path rather than at each call site so it holds for whatever
route somebody adds next.

## The catalog is what the folders hold

**After a startup pass the catalog contains exactly the packages that pass installed, and nothing
else.** A `.kmpkg` taken out of a folder loses its rows, its songs and its place in the search index.

Without it, a package whose file has gone keeps its rows for ever: those songs stay in the count on
the idle screen, stay dialable, and fail at the moment somebody picks one — **which is the worst
way for a karaoke machine to be wrong, because the failure lands on a person standing up to sing.**

**Reconciled against what *installed*, not against what is on disk.** A package that is present but
will not open is not in the kept set, so its rows go too. The machine cannot serve songs out of an
archive it cannot read, so rows that survived would be exactly the phantoms this exists to remove —
and the fault is not silent, because the file's name and the reason are above the title, on the
machine's own remote and in `GET /api/v1/packages`. The cost is real and accepted: songs *disappear* from
the catalog rather than lingering as rows nothing can play.

**The set is the scan plus the debug extras**, never the scan alone — otherwise every pass would prune
precisely the packages `debug.packages` had just installed.

**It is only safe because there is no removable media to model** (see
[`No removable media`](foundations.md#no-removable-media)). The argument that a file missing this
morning is usually a drive that is not plugged in applies to a *file*; this reconciles against
*folders*, and a folder that is not there contributes nothing and takes nothing away.

**A dropped package keeps its bank.** `settings.package_banks` is keyed by package id and is not
touched here, so a package that comes back — a folder unreadable for one pass, a file being mended —
comes back with the song numbers it had. Without that, prune-then-reinstall would renumber, and the
reconcile would become the thing that stales every printed book in the house. Only an explicit
uninstall releases a bank.

**It runs where nothing is playing.** The pass happens before the API binds and before the display
thread exists, so the queue is empty by construction and there is no analogue of the 409-while-playing
rule. Anything that reconciles while the machine is running has to earn that separately.

## A package's default language

**A package names a language its unclassified songs go in under, prefilled `en`, and it fills the
package and nothing else.**

The danger is a corpus whose language column is filled with a value nobody checked, and the answer is
not to refuse the packager an answer — it is to keep the answer **out of the corpus**. So the default
lives on the *package*, is written into the manifest and never into the `songs` table, and is not
marked hand-edited, because nobody edited it. The corpus goes on saying `nobody has said` about a song
no one has looked at.

Without it, on a real corpus most of a package's songs have no language the first time somebody
tries, so the tool answers a click with a list of two hundred songs and no way forward.

**Prefilled `en` rather than blank**, and the prefill is the migration —
`ALTER TABLE packages ADD COLUMN default_language TEXT DEFAULT 'en'` fills every package that already
exists, so the first build after the upgrade does not refuse. `en` because it is the commonest answer
and a wrong one is visible and correctable in the package, whereas a build that refuses is a person
stuck.

**Clearing the box asks for the strict rule**: a song with no language stops the build and is
listed. The refusal names both ways out — the package default and the bulk set — because they answer
different questions: one is "this volume is Portuguese", the other is "these files are Portuguese",
and only the second is worth writing down for every later package.

## The manifest format is chosen by content, and an unknown kind is kept

**A package declares the oldest reader that can handle it.** `required_format` returns 1 for an
all-MIDI package and 4 for one holding any media, so the number says what a reader needs rather than
when the package was built. It is a fact about the manifest, and it is separate from the container
version, which says how to find the entries.

**Raising the version does not improve the message a binary already in service gives**, and that is
why `SongKind` carries a serde catch-all. Without one, a `kind` a build does not know fails to
deserialize the whole manifest and `Package::open` returns a JSON error *before* `problems()` can say
`UnsupportedFormat`, and no value written into the file can fix that retroactively. The catch-all is
the durable half: an unknown kind degrades to a clean refusal naming the song rather than to a parse
error naming a byte offset.

## A clash warns rather than only logging

**A package the machine refuses is remembered and said out loud — on the idle screen, in
`GET /packages`, and on the online remote.** A refusal that is one line in a log, on an appliance under
a television, is a catalog quietly missing an album with nothing anywhere accounting for it.

What is left to say is one sentence with one remedy: a file that will not open, or a bank another
package holds.

**The television counts rather than saying it** — `1 problem: packages` — and the sentence lives on
the three surfaces that can act on it. See
[`A fault says how much is wrong, not what`](interface.md#a-fault-says-how-much-is-wrong-not-what).
A refusal is still *said out loud* rather than logged, and a machine standing in front of somebody
with an album missing owes them an account of it somewhere they will find.

**A problem is a fault, not a decision.** It stays out of `packages_ignored`, which records something
the owner chose, and it lives in memory only, so a package fixed between two runs leaves no trace. The
offline remote is deliberately excluded and the trait default says why: it holds a mirror of a
*catalog*, which cannot describe a package that never entered one, and the person who can act on this
is standing at the machine.

**A fourth surface can do something about it.** `/admin/`'s Problems tab lists the same refusals and
offers to delete the file behind each — see
[`The Problems tab, and deleting a package that never installed`](api-and-network.md#the-problems-tab-and-deleting-a-package-that-never-installed).
That does not soften the sentence above about the person being at the machine; it widens what *at
the machine* means to any browser that can reach it, which on a box with no shell is the difference
between a fault being reported and a fault being fixable.

**The reason beside the name carries no path.** Every surface that prints a reason shows the file's
*name* deliberately, and a reason that smuggles the whole path back defeats them: the online remote
and `/admin/`'s Problems tab both give it a row, and a row spent on `C:\Users\…\Downloads\` is a row
not spent on what is wrong.

**A path inside a wrapped sentence eats the reason.** A Windows path is one unbreakable word, so a
notice wrapped on whitespace into two lines spends a line on the path and drops the reason off the
end, leaving a sentence that ends in a bare colon and reads as a machine that will not say what is
wrong. The reason is stored stripped — one place, so no surface can reintroduce a path by forgetting.

**The television's own cut is marked with an ellipsis rather than made in silence**, and
`NOTICE_MAX_LINES` is two. Both are headroom rather than a budget: the television carries a count and
an area, which fit one line on any screen a television is, and the cap is what a longer translation on
a phone in portrait wraps against.

## A song number is a bank and a slot

**`bank * 1000 + slot`, and the number a singer dials carries its package inside it.** The slot is the
song's number within its package, 1 to 999; the bank is 0 to 9999 and says which package's block of a
thousand the song sits in. So `3500` is bank 3's five hundredth song.

Splitting the number costs nothing at the keypad, because every code is still digits — where a letter
prefix would say the same thing with characters a D-pad under a television does not have.

**One spelling on the wire, always a string**: `{"number": "3500"}`, and a bare integer is *refused*
rather than accepted as well. The reason is not ambiguity, since a bare integer names exactly one
song, but that there is one spelling at all: every route, element id, template and script sends a
string.

In the database the identity is `UNIQUE (number)`, with `id` a surrogate that exists only because
FTS5 needs an integer rowid. **The identity is not packed into one string column**: two places read
that column *as a number* and neither stops compiling. None of that applies to a bank, which *is* a
number, so `read_song` reads the whole code as one integer.

**Inside a package a number is a bare `u32` slot, bounded at 999.** The catalog's export cursor is
`number > ?` and the derived `Ord` is `u32`'s.

## A package holds at most 999 songs

**A statement about curation before it is one about arithmetic.** Packages here are put together by
hand, and a cap is what discourages pointing the builder at a corpus and importing all of it; that it
also makes a bank exactly a thousand wide is what makes it cheap.

Enforced in `Manifest::problems`, so such a package can neither be written nor opened — a deliberate
exception to the standing hazard on that function: a slot above 999 does not merely fail to dial,
banked it **is** a number belonging to the next package's block, so the package names songs that are
not its own wherever it is read. It rests on the same grounds as `NumberTooLarge`: no `.kmpkg` has
been released, and it states what the machine *can do* rather than a preference.

**Two problems and not one** — `TooManySongs` says the package is too big and `NumberTooLarge` names
the song that will not fit, and a package of a thousand songs numbered 1 to 1000 trips both.

The packagers refuse earlier, where somebody can still act: `km-pack spec` refuses to describe a folder
that overflows, naming the count and pointing at `--min-suitability`, `--require-lyrics` and
`--limit`, and `km-package-builder` refuses a hand add past the cap and offers **Re-flow**.

**A curated set larger than 999 songs is several packages, and the curation tool divides it.** A
package there holds volumes, each a `.kmpkg` under this cap, and a package sourced from favorites
starts the next volume when its lists outgrow the last — see
[`A package holds volumes`](curation.md#a-package-holds-volumes). The cap's purpose holds, because a
volume is filled from lists a person curated rather than from a folder.

## A package file can say which set it is a volume of

**A manifest may carry `volume: { of, name, number }`, and only the tool that wrote it reads it.** A
curated set larger than one package is divided by the curation tool into volumes, and each volume is a
package in every sense the machine has: its own id, its own bank, its own install and its own removal.
The key exists so that a built volume imported back into the tool rejoins the set it came from rather
than arriving as an unrelated package.

**`Manifest::problems` never checks it.** Whatever that function refuses, a machine already in use
cannot open, and nothing about which set a file belongs to changes whether its songs can be played.
It is optional, skipped when absent and read past by an older reader, so the format version does not
move. `km-pack inspect` prints it, and a description names it under `package` as `volume:`.

## A package's id asks for a bank; the machine assigns one

**The machine decides, and it takes a free request.** A bank is a **slot** with no meaning — the
machine has to fill it either way, and honoring the one a package's id implies takes nothing from
anybody and changes nothing already installed.

`Machine::ensure_bank` reads, in order: the bank recorded in settings; the bank the *catalog* holds,
written back into settings; then `choose_bank`, which takes the bank the package asks for when it is
free and the next one after it when it is not.

**A colliding package is never refused** — it lands at the next bank instead of being silently missing
until somebody reads the idle screen. The second step is load-bearing: a `settings.json` that was lost
or rescued into `settings.json.bad` starts from an empty map, and without it an already-installed
package could be moved under a live queue. With it the invariant is flat: **automatic banking never
moves a package that already has a bank, from either source.** A recorded bank of **0** is the one
value that reads past both steps, because 0 is not a bank a package may hold — it is a stale value
rather than an assignment, and the package is banked as though nothing had been recorded, with a
`warn` naming where it went.

The assignment lives in `settings.package_banks`, keyed by package **id** and not by path, and must be
knowable *before* the install, because the install writes the bank into every song's number.

`PUT /api/v1/packages/{id}/bank` is the only thing that ever moves an installed package: **admin by
default**, since it re-keys every song and a guest could break the number somebody is reading out of a
printed book, and **refused with a 409 while anything is playing or queued**, because the queue holds
numbers. It reaches 1 to `MAX_BANK` — see
[`Bank 0 is the machine's own`](#bank-0-is-the-machines-own).

## A package's bank comes from its id, and from nothing else

**The same package gets the same numbers on every machine.** A
package is banked at `1 + SHA-256(id) % MAX_BANK`, so installing a volume here and at a friend's
house — in any order, beside anything else — puts it in the same thousand both times, and a song list
printed from the file travels with it.

**A package cannot name its own bank.** There is no `bank:` key in a manifest and none in a
description, `km-package-builder` has no field for one, and `SpecPackage` sets `deny_unknown_fields`,
so a curator who types one is told the key does not exist rather than building a package that lands
somewhere other than the number they wrote down. The reason is the same one the derivation exists
for, pointed at the person instead of the code: a number a curator chooses is true on the machine
they chose it for and a **guess** about every other one. Any other machine that already holds that
thousand puts the package in the next free bank — silently, correctly, and leaving the book printed
from the file wrong. The id is the only thing about a package every machine reads the same way, so it
is the only thing allowed to decide.

**SHA-256 and never `DefaultHasher`**: `DefaultHasher` is explicitly unstable
across Rust releases, and this answer has to be the same number next year and on somebody else's
machine.

**Where the guarantee does not hold, something else catches it.** *Usually* the same thousand is not
*always*: a machine that finds the bank taken uses the next free one, and the owner may move a package
outright. Both leave a number meaning one song here and another there. A phone's favorites are filed
under the package and the content hash as well as the number — see
[`A favorite rejoins its song by content`](remotes.md#a-favorite-rejoins-its-song-by-content) — so a
collection survives the cases this derivation cannot cover. Nothing can rejoin ink to a renumbered
catalog, which is what the derivation is for.

**`MAX_BANK` is 9999, sized against collisions rather than against how many packages fit.** Two
packages wanting the same thousand is a birthday problem over the bank space, counted over the
packages installed *together* — so a large ecosystem of third-party packages does not drive it and a
large personal library does. A thousand banks put it at 17% for twenty packages on one machine and 35%
for thirty; ten thousand put it at 1.9% and 4.3%. Nothing fails when it happens, since `choose_bank`
takes the next free bank, but the loser's already-printed book is then wrong, which is the single
thing this derivation exists to prevent.

**9999 and not 99999**: the next step up costs an eighth digit *and* widens every bank in the
workspace from `u16` to `u32`, for a gain that only tells past about fifty packages on one machine. **A
1000–9999 overflow tier was also rejected** — using the fourth digit only when a package collides
would keep most numbers at six digits and reduces the collision *rate* not at all, which is the number
that decides whether a book is right.

**The spread takes four digest bytes, not two.** `65536 % 9999` is 3,940, so two bytes would give
banks 1 to 3,940 a 17% edge over the rest — and a bias towards the low banks is a bias towards
collisions, since it crowds packages into a third of the range.
`derived_banks_are_spread_evenly_over_the_range` walks 50,000 ids and checks the thirds.

**The question has one home, `PackageMeta::wanted_bank`**, which both the book and the machine call.
Two copies of a question get two answers — a package printing one number in a book and installing
under another.

**Bank 0 is outside the hash space rather than skipped by the allocator.** If the hash could land on
0 and only the allocator stepped over it, that package would print bank-0 numbers and install
somewhere else — the book-against-machine disagreement reintroduced for one package in ten thousand.
So `suggested_bank` returns 1 to `MAX_BANK` and the derivation agrees with
[`Bank 0 is the machine's own`](#bank-0-is-the-machines-own) rather than merely obeying it.

**A tie probes forward from the bank asked for**, not from the bottom: the loser lands beside where
other machines put it, so a book printed from the file is wrong by one thousand rather than by
wherever the range happened to be empty.

**Nothing stamps a derived bank into a manifest.** `wanted_bank` derives the same answer on read, so a
`.kmpkg` says nothing about which thousand it lands in and cannot contradict the machine that
installed it. A stamped bank would pin an already-built package against a future change to
`suggested_bank`, splitting packages built before such a change from those built after it. A `bank:`
key left over in an older manifest is read past — `PackageMeta` sets no `deny_unknown_fields` — and
decides nothing.

**`km-pack book --bank <PACKAGE>=<N>` is the one place a number is typed, and it is about a machine
rather than a package.** An owner who has moved an installed volume with
`PUT /api/v1/packages/{id}/bank` needs the book they print next to follow it. That is a statement
about one machine's state, made by the person who can see it, and it reaches only the paper.

**The allocator is `choose_bank`, pure and beside the `Machine` rather than inside it**, because
building a `Machine` needs an audio device and an instrument bank, so branching logic put inside one
goes unexercised.

## Bank 0 is the machine's own

**No package may be given bank 0, and no surface offers it.** It is the block whose songs would dial
in three digits, and those three digits are the machine's: what a singer types below 1000 is for the
machine to answer, not for a volume of songs to occupy.

**Impossible rather than refused.** `SongCode::in_bank` returns `None` for bank 0 exactly as it does
for slot 0, so a code in that block cannot be constructed anywhere in the workspace, and
`Library::install` and `Library::set_package_bank` refuse it with `BankReserved`. Every surface that
takes a number sits on top of that and writes the sentence: `PUT /api/v1/packages/{id}/bank` answers
a 400, the owner's page and the dev remote refuse before they ask, and `km-pack book --bank` takes 1
to `MAX_BANK`. A check per surface would be four rules free to disagree, and the one that forgot
would be the one that mattered.

**Parsing is untouched, and that half is deliberate.** `"500"` is still a `SongCode` — the keypad
answers `no song 500` as it does for any number nobody holds, rather than refusing the keys as they
are pressed. What a number below 1000 comes to mean is a separate decision; this one only keeps the
block free for it.

**A recorded 0 is a stale value.** `Machine::ensure_bank` reads past a bank of 0 in settings or in
the catalog and allocates as though nothing were recorded, logging where the package went. The
alternative is a package refused at every start by a rule the machine itself now holds — a machine
that quietly lost songs, over a number nobody typed.

## A package's id is generated, not typed

**`km-package-builder` generates sixteen hexadecimal characters from OS entropy** —
`PackageMeta::new_id`, the same shape as `km_api::discover::new_instance_id`.

**The failure this closes is the worst one the catalog has**, and it is not a bank collision. Two
packages sharing an id is not a clash the machine can report: it keys an install on the id precisely
so that reinstalling a package *upgrades* it, so a second package arriving under an id the first
already used **replaces a volume that has nothing to do with it**. `karaoke-vol1` is a name two
packagers would each pick unprompted, so a typed id makes that a matter of luck. Sixty-four bits
makes it a matter of arithmetic.

**This does not reduce bank collisions.** SHA-256 destroys whatever structure an id had, so two
*distinct* ids land in the same thousand with probability `1/MAX_BANK` whether they are readable or
random. The bank space is what governs that.

**The shape is required, and `Manifest::problems` refuses anything else** — `PackageMeta::is_generated_id`,
sixteen lowercase hexadecimal characters. `problems` is the one gate every route in shares, so a
package carrying a typed id cannot be opened at all.

**The second reason for the shape is that a typed id is a leak.** People name a folder after a client,
a party, or whose collection it was, and an id is what the installed file's name is built from — see
[`A package says nothing about the machine that built it`](#a-package-says-nothing-about-the-machine-that-built-it).
`km-pack spec` generates one for a scanned folder and takes only the *name* from it.

**The shape is all that is checkable, and it is enough.** Randomness cannot be verified from a value:
`0000000000000000` satisfies this. What it buys is that nothing a person types *as a label* does.

**A package whose id is not one of these is refused rather than repaired**, and the remedy is a
rebuild, which the message names. Repairing it is not available: the id is what an install is keyed
on, so a machine that invented a new one would install a second package beside the first rather than
replacing it. `build::import` re-opens a built `.kmpkg` and carries its id back in unchanged, which is
the recovery path if a `.kmbuild` is lost — for a package that satisfies the shape.

**A curation database is brought to the rule when it is opened**, because the id is not a field the
build form offers and a package described under a typed one would otherwise refuse every build it was
asked for, with nothing a person could do from the page. What that costs is the id's whole purpose:
an install is keyed on it and `suggested_bank` hashes it, so a rebuilt package arrives on a machine
as a *new* package in a different thousand rather than as an upgrade of the one already there. Its
predecessor no longer opens, so the machine has already stopped serving it; removing the file, and
reprinting any book that named the old numbers, is the owner's to do.

**A published package spells its id out rather than generating one.** `km-carols` is the one built
here, and every copy has to carry the same value or a reinstall would put a second package beside the
first and a book printed last year would name the wrong thousand. What it writes down is a value of
the generated shape, so it keeps this rule rather than escaping it.

**The readable half is `PackageMeta::name`** — what a person types and what every surface shows — so
the form requires a name, and the fallback that fills a blank name from the id stays only so that
importing a `.kmpkg` whose manifest names nothing refuses nothing.

## A package's id and version are names, because the file is named from them

**A package is written to disk under a name its own manifest decides**, so the id and the version are
not only labels — they are the machine's answer to *where does this file go*. A package holding
`../../../../evil` as its id is a package asking to be written outside the packages folder, and
joining an absolute path onto a folder discards the folder outright.

**The refusal is at `Manifest::problems`**, so it is one gate for every route in and a package that
says this cannot be opened at all. That matters more here than anywhere else the manifest is
checked: **a package the operating system hands the machine is taken in before anybody has typed a
password**, so the two routes behind the admin password are not the ones this protects.

**A name, not a shape.** `km_kmpkg::is_safe_name` refuses separators, colons, control characters and
values that are nothing but dots and spaces — `..` among them, since Win32 strips a component's
trailing dots and spaces and what reaches the filesystem is then not what the name said. Everything
a person would actually choose is still accepted: `Músicas`, `2024-spring`, `1.0.0-rc1`.

**It stands beside the id's own shape rather than behind it**, and the pair is not redundant. An id
of sixteen hexadecimal characters satisfies `is_safe_name` by construction, so that check earns
nothing today — and it is the one that stops a file being written outside the packages folder, which
is reached before anybody has typed a password. A later change to either shape cannot quietly open
that while both are in force. The version has only this check, being free-form.

**Length is not a reason to refuse.** A long version makes a long file name and nothing worse, and
`file_stem` bounds what it builds. Refusing one would stop a package opening that has installed
correctly for as long as it has existed, which is the cost the
[refusal rule](../architecture/packaging.md) sets against a fault that is not a fault.

**The derivation is made safe as well as checked, and the pair is not redundant.** `file_stem` folds
an id that is not a name rather than trusting that it was checked, because `read_manifest_unchecked`
exists to *diagnose* a manifest and so reaches the derivation with the gate deliberately skipped.
`free_name` refuses one at the line that joins. Three guards for one rule, at the two ends and the
middle, because the middle is where a route added later would arrive.

**The version is in because the curation tool names a build from it.** `default_out_path` builds
`<name>-<version>.kmpkg`, and a version is carried across an import from a manifest the tool did not
write. The typed version was already three numbers; the imported one was not checked at all.

**Not derived from the name, the songs or the time.** A name is exactly the thing two packagers
collide on. A content hash would change the id every time a song was added, which is the one property
an id must not have — a rebuild with fifty more songs in it is still the same package, in the same
thousand. A timestamp collides between two people working the same afternoon.

## A list of installed packages names the build

**Every surface that lists installed packages carries the version, and the removal confirmation names
it too.** The machine writes an installed package to
[`<name>-<id>.kmpkg`](#what-an-installed-package-file-is-called) so that a rebuild lands on the file
it replaces, which leaves the file name unable to say which build is in it. The list is therefore the
only place on a machine where two builds of one package can be told apart, and telling them apart is
[the version's whole job](curation.md#a-rebuild-raises-the-version-and-it-is-the-patch-that-moves).

**The question it answers is asked from the tool, not from the machine.** Somebody who has just
rebuilt a volume and sent it wants to know that the build which landed is the one they made. So it is
on `km-admin`'s table for the reason it is on the machine's own, and one template draws both — the
`installed_packages` capability is on for both hosts, and a fact one surface withheld would make the
tool the weaker place to ask from.

**A column, matching the curation tool's.** The two programs are open side by side while a volume is
being built and sent, so the packages they each list name one fact one way.

**The confirmation names it first**, above the song count, the numbers and the size. That page may be
standing in front of the only copy of tens of gigabytes, and the heading above it names the package
where what is wanted is the build.

**The television still says only how much and where.** A version is something somebody goes looking
for deliberately, which is the distinction
[`Every program says which build it is`](interface.md#every-program-says-which-build-it-is) draws
about the machine's own number, and
[`A fault says how much is wrong, not what`](interface.md#a-fault-says-how-much-is-wrong-not-what)
already keeps a package's detail off a screen nobody can act from.

**Nothing compares two of them.** The string is free-form at the manifest, an install is keyed on the
id, and the reader is a person — so a surface shows the version and never sorts, ranks or upgrades by
it.

## Number collisions are impossible, not refused

**`UNIQUE (bank)` on `packages` is *total*** — every package has exactly one bank — so two packages
are never in the same thousand and two songs can never share a number. There is no collision refusal
to write, and no collision report to render.

Nothing renumbers silently either: changing a song's identity loses whatever is printed in a book,
and overwriting loses the song.

## A song number stops at 9999999

**Song numbers run 1 to 9999999, and the limit comes from the keypad rather than from storage.**

A number exists to be dialled, and the machine's number entry takes `MAX_DIGITS` digits — so a song
numbered above this would sit in the catalog, answer to search, and be unaskable at the one surface
the number is for. That ceiling is real where a number is *typed* and imaginary where it is
*assigned*, so the limit is one constant in `km-songcode` and it bites at every point a number is
claimed: parsing a code, opening a package, describing a folder, building, and typing one into either
tool.

**Two bounds, not one.** `MAX_NUMBER` bounds a whole *code*, where one is parsed; `MAX_SLOT` bounds a
song's number *inside its package*, where one is assigned. `MAX_BANK`, `MAX_SLOT` and `MAX_NUMBER` are
one statement written three ways and a test says so.

**The seventh digit is charged to the television.** A song has been dialled on a Google TV Streamer
with the remote alone at several D-pad presses a digit, and the idle number pad is on for every
Android precisely so a television is self-sufficient without a phone. Seven digits makes that a
third more work on the one surface where a number is laborious to enter. The judgment is that the
remote app is how a number is normally entered and the keypad is the fallback that must keep
working; the collision arithmetic that bought the digit is in `A package's bank comes from its id`.

**The song book absorbs it for nothing**: the CODE column is 36.6 pt and seven digits is 27.2 pt of
it, so no column moves, and `the_widest_song_number_fits_the_code_column` asserts that rather than
leaving it to be re-measured.

**A package carrying a larger number is refused rather than tolerated**, a deliberate exception to the
rule that a manifest check must never refuse a package in service: there are none, and unlike a
curation preference this states what the machine *can do* — such a package is broken wherever it is
read from. `10000000` and `99999999999` are one error and not two, because they are the same mistake.

**The storage constrains nothing**: the column is an unconstrained SQLite `INTEGER` and the field is
a `u32`, so this is a rule about what may be written and never a narrowing of what can be read back
— which is why widening the bank space needs no migration.