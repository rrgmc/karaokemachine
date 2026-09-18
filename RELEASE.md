# Cutting a release

The order to do it in. Each step names the document that holds the detail, because a second copy of
a command list goes stale against the first — [`BUILDING.md`](BUILDING.md) is the command reference
and this is the sequence.

A release raises the **minor** number and zeroes the patch: one number covers every program here, so
they all move together. See
[`One version number for the whole repository`](docs/decisions/repository.md#one-version-number-for-the-whole-repository).

## 1. Write down what changed

[`CHANGELOG.md`](CHANGELOG.md) — retitle `## [Unreleased]` to `## [X.Y.0] - <today>`, and open an
empty `## [Unreleased]` above it. An entry names what somebody with the machine gets, in one
sentence, consequence first. A crate, a route, a thread, a build flag or a file format's internals
in an entry is the shape to catch: it names what was changed rather than what the change gives.

**Do this first**, while the range is still `git log --merges v<previous>..HEAD` rather than
something to reconstruct after the tag exists.

**A release with no predecessor is the one case that skips the range.** Where there is no
`v<previous>` to resolve, the entry is written from the branch's own commits and from what the
machine does, and it is written into the empty `## [Unreleased]` rather than by retitling one that
already carries a release's worth of entries.

## 2. Write the download page's body

[`tools/dist/release-notes.md`](tools/dist/release-notes.md) — its `## What changed` list is this
release's few highlights, taken from the section just written. A release with no predecessor heads
that list `## What it does` instead and says what the machine is, because a page whose reader cannot
reach the version before it has nothing to state a difference against. The tracked file names no
version; the run substitutes one. Its register is
[`What a release page says, and to whom`](docs/decisions/distribution.md#what-a-release-page-says-and-to-whom),
and the narration rule reaches it like any other published word —
[`How a document in this repository is written`](docs/decisions/repository.md#how-a-document-in-this-repository-is-written).

**Edit the file, never the GitHub form.** A body typed at the point of upload is read by neither
`check-prose.sh` nor `check-no-local-refs.sh`, both of which choose what to read through
`git ls-files`.

## 3. Raise the number

The two manifests, then both lockfiles, then the eight places written by hand that no script checks —
[`Bumping the version`](BUILDING.md#bumping-the-version) has the commands and the table.

`git grep -F <old version>` afterwards is the check that exists. The changelog's headings are hits it
is meant to find and leave alone: they are a record of a release that happened, not a number that
advances. A release with no predecessor has no such heading yet, so every hit it reports is one to
move.

## 4. Prove the tree

```sh
task check
```

**A failure stops the release.** Nothing below is worth doing over a tree that does not pass.

## 5. Commit and tag

One commit carrying the bump, the changelog and the notes:

```sh
git commit -m "chore(release): X.Y.0, <short phrase naming the release>"
git tag -a vX.Y.0 -m "karaokemachine X.Y.0"
git push origin master --follow-tags
```

Annotated, never lightweight — `--follow-tags` carries no other kind. A published tag is never moved
or deleted; a mistake is the next patch version.

## 6. Stage the carriers, and upload

```sh
tools/dist/release.sh           # gather what is staged into dist/release/<version>/
tools/dist/release.sh --upload  # ...and fill the draft release
```

**It gathers and never builds**, so every carrier has to be staged first —
[`Releases`](BUILDING.md#releases) lists the script behind each one, and `release.sh` names any that
is missing and stops.

**Name the platforms where a Mac is not to hand.** `--platforms windows,linux,android` carries those
and leaves the two `.pkg` files and the two `.ipa` files out of the count and off the page, rather
than failing on four carriers the machine cannot build. Everything else holds: a named platform
whose carrier is missing still stops the run. See
[`A release page carries the platforms the machine cutting it can build`](docs/decisions/distribution.md#a-release-page-carries-the-platforms-the-machine-cutting-it-can-build).

`--upload` creates a **draft**. Publishing is typed by somebody who has opened the page and looked at
it:

```sh
gh release edit vX.Y.0 --draft=false
```
