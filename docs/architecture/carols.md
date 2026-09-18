# The Christmas carol pack

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

`km-carols` turns a pinned edition of the Open Hymnal Project's ABC into sixteen Christmas carols, and
a script fetches, converts, packages and reports.

**Nothing about the machine changed.** `assets/` is untouched, so every carrier's asset handling and
the Android size budget are unchanged, and the pack is built into `dist/` rather than committed. There
is no first-run install path and no second package source.

## The pipeline

ABC → `km-carols` → `.kar` + a description → `km-pack build` → `.kmpkg`. The last arrow is the rule the
curation tool already follows: **this tool describes and never packages**, so no second code path can
disagree about what a package holds.

**Verse expansion is the part that is not obvious.** A hymnal writes the music once and stacks the
verses beneath it as consecutive lyric lines, which is right for a printed score and useless for a
karaoke file: `abc2midi` takes one set of lyrics, so the naive conversion of *Silent Night* is verse
one alone — 49 syllables, about thirty seconds, **below the duration the suitability score credits at
all**.

The expansion regroups the body into *systems* — a run of voice lines plus the lyric lines under them,
a new one beginning when a voice the current system has already written reappears — and emits every
system once per verse, carrying only that verse's words. *Silent Night* becomes 3 systems × 4 verses,
171 syllables and 3:54. **The grouping rule is what makes it independent of voice count**, which
matters because the sixteen range from two voices to four.

## Three things found by running it

- **abc2midi writes Soft Karaoke natively**, which is why no MIDI *writer* was needed — only a
  rewriter.
- **The Open Hymnal's published MIDI zip is useless here**: zero lyric events, two tracks, voices
  already merged. Their build never asked for karaoke.
- **Two upstream notations abc2midi rejects outright** — a staccato before a tie, and a nested staccato
  slur. Both are accepted by the typesetter the hymnal is set with, and both **drop music rather than
  warning**. Normalizing them is what makes all sixteen convert with zero errors.

## The MIDI rewrite, and the trap inside it

**A hymn setting is homophonic** — four voices moving together — so every voice aligns with the lyrics
exactly as well as every other. Melody detection needs the winner to beat the runner-up by half again,
and the only tiebreak available is the bonus for a melody-shaped **track name**. ABC carries a voice
name, `abc2midi` parses it for the typesetter, and **it never reaches the MIDI**. So the pass names the
first track `Melody` itself. Measured: 7/10 for all sixteen before, **9/10 for fifteen after**.

The same pass writes the header into the words track — so a `.kar` lifted out of the package still says
what it is — and drops abc2midi's own track labels, which are plain text events in the lyric stream and
**therefore arrive as the song's first line on screen**.

**It expands running status on the way through.** The pass *deletes* events, and deleting one that
carries a status byte **silently changes the meaning of every running-status event after it** — the
class of corruption that plays almost correctly. Deltas are carried forward on a deletion, so nothing
after a dropped event moves.

## The license gate

It reads each tune's own copyright line and requires public domain in **all four layers a hymn divides
into**: music, setting, words, translation. Shaped after the wallpaper tool's gate and keeping both of
its rules — the allow-list is a **constant in the code and never a config key**, and it **fails
closed**.

**It earned its keep in the first edition it was pointed at.** One carol is public domain in its words
and music and CPDL's in its **setting**, whose default license this project declines. It is not in the
pack, and two tests guard against it drifting back in.

A second shape it has to catch is *music and setting public domain. Words: Copyright 2010, …* —
**which contains the words "public domain" and is not what may be taken.** Hence whole-statement
matching plus a restricted-marker sweep rather than a substring search.

## The floor, and the carol that sits on it

Every generated file is parsed and scored **before it is written**, and the two components that decide
whether a song can be sung are required **exactly**: syllable-level lyrics, and timings that land on
the notes. Melody and arrangement are worth points and not worth failing a build over.

*Away In A Manger* is why the floor is 6 rather than 9: it is set for two voices with chords in the
treble staff, **so nothing in it is monophonic** and melody detection abstains whatever the tracks are
called. It loses four points and is perfectly singable. The other fifteen score 9 — the missing point
everywhere is the drum channel, which a hymn setting will never have.

## Lines that ran off the screen

Measuring found **two independent faults**, one introduced by this pack and one in the display since it
was written. Neither would have been enough alone.

**The pack's lines were far longer than any real karaoke file.** A hymnal writes one lyric line per
musical *system*, and the conversion mapped one system to one displayed line: mean 57.8 characters, 92%
past 40. Real karaoke, over 152,228 lines from 3,795 corpus files, averages **26.7** with p90 39.
Re-wrapping at the comfortable width — `km-song`'s own constant, promoted from a literal — gives mean
27.7 with none over 40.

**`km-song` could not have caught it, and must not be changed to.** The timeline builder trusts a file
that supplies its own break markers absolutely, and abc2midi's output is marked. **That rule is right**
— second-guessing a file that said where its lines go would make things worse — so the comfortable
width is reachable only for an *unmarked* stream, and the fix had to be where the file is made.

The display half is in [`display.md`](display.md).

## `abc2midi` is built here, and the compiler is found by trying

**Built rather than downloaded, and there was no choice.** The project publishes no binaries: its
releases list is empty, its file area lists none, and the site its own repository points at answers
404. So the alternatives were an unofficial third-party binary of unknown provenance, or eight C files
with no dependencies. Pinned by tag *and* by the archive's SHA-256, with the failure mode every archive
pin here has: **GitHub generates these tarballs rather than storing them**, so a digest that stops
matching while the tag has not moved means re-pin deliberately rather than delete the check.

**The compiler is chosen by trying, not by guessing.** Each of `$CC`, `cc`, `gcc` and `clang` is asked
to build the thing and the first that succeeds wins. **A table of platforms and toolchains was the
obvious alternative and is exactly wrong**: it would encode whatever the author happened to have
installed, on a machine whose toolchain is an accident of history.

It also discovers the one fact that would otherwise be learned the hard way: **a clang targeting MSVC
cannot build abcMIDI.** The source carries a `snprintf` redefinition behind an `_MSC_VER` guard for
compilers of twenty years ago, and a modern UCRT header answers with a hard `#error`. A MinGW gcc never
takes that branch and builds it in one line. **Measured on a box that has LLVM installed for
`libclang`** — the obvious "we already require clang, use that" shortcut does not work, and the loop
finds a working gcc instead of failing.

The build script looks in three places in order: an explicit path, `PATH`, then the cache. **The cache
is last** so a system package or a hand-built one already on the machine wins.

## Two smaller decisions

**`dist/carols/`, not `dist/<app>/<platform>/`.** The layout rule exists so every build of one product
sits together, and a `.kmpkg` has no platform axis. **An `any/` level to satisfy the shape would say
something untrue about the file.**

**The description carries no root.** The output path resolves against the description's base, and the
base *is* the root where there is one — so a root of `songs` would put the built package inside the
songs folder. Each song names its folder instead.
