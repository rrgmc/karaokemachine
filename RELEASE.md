# Cutting a release

The order to do it in. Each step names the document that holds the detail, because a second copy of
a command list goes stale against the first. [`BUILDING.md`](BUILDING.md) is the command reference,
and this is the sequence.

A release raises the **minor** number and zeroes the patch: one number covers every program here, so
they all move together. See
[`One version number for the whole repository`](docs/decisions/repository.md#one-version-number-for-the-whole-repository).

## 1. Write down what changed

[`CHANGELOG.md`](CHANGELOG.md) — retitle `## [Unreleased]` to `## [X.Y.0] - <today>`, and open an
empty `## [Unreleased]` above it. An entry names what somebody with the machine gets, in one
sentence, consequence first. A crate, a route, a thread, a build flag or a format's internals is the
shape to catch. It names what changed rather than what the change gives.

**Do this first**, while the range is still `git log --merges v<previous>..HEAD` rather than
something to reconstruct after the tag exists.

**A release with no predecessor is the one case that skips the range.** Where no `v<previous>`
resolves, write the entry from the branch's own commits and from what the machine does. It goes into
the empty `## [Unreleased]`, rather than into a retitled one that already carries a release's worth
of entries.

## 2. Write the download page's body

[`tools/dist/release-notes.md`](tools/dist/release-notes.md) — its `## What changed` list is this
release's few highlights, taken from the section just written. A release with no predecessor heads
that list `## What it does` instead, and says what the machine is. A page whose reader cannot reach
the version before it has nothing to state a difference against. The tracked file names no
version; the run substitutes one. Its register is
[`What a release page says, and to whom`](docs/decisions/distribution.md#what-a-release-page-says-and-to-whom),
and the narration rule reaches it like any other published word —
[`How a document in this repository is written`](docs/decisions/repository.md#how-a-document-in-this-repository-is-written).

**Edit the file, never the GitHub form.** Neither `check-prose.sh` nor `check-no-local-refs.sh` reads
a body typed at the point of upload, because both choose what to read through `git ls-files`.

## 3. Raise the number

The two manifests, then both lockfiles, then the eight places written by hand that no script checks —
[`Bumping the version`](BUILDING.md#bumping-the-version) has the commands and the table.

`git grep -F <old version>` afterwards is the check that exists. The changelog's headings are hits it
is meant to find and leave alone. They record a release that happened, rather than a number that
advances. A release with no predecessor has no such heading yet, so every hit it reports is one to
move.

## 4. Prove the tree

```sh
task check
```

**A failure stops the release.** Nothing below is worth doing over a tree that does not pass.

## 5. Commit, merge, and tag

One commit carrying the bump, the changelog and the notes, on a branch of its own. `master` takes
pull requests only, so the release reaches it the way every other change does:

```sh
git checkout -b release-X.Y.0
git commit -m "chore(release): X.Y.0, <short phrase naming the release>"
git push -u origin release-X.Y.0
gh pr create --fill
gh pr merge --merge --delete-branch   # once `CI ok` has passed
```

**The tag goes on the merge commit, after the merge.** That commit is the one on `master`, so it is
what the tag names and what the release is built from:

```sh
git checkout master
git pull --ff-only
git tag -a vX.Y.0 -m "karaokemachine X.Y.0"
git push origin vX.Y.0
```

**Pushing the tag starts the release build**, so it is pushed last and alone. Annotated, never
lightweight. A published tag is never moved or deleted; a mistake is the next patch version.

## 6. Stage the carriers, and upload

**The pushed tag does this.** `.github/workflows/release.yml` builds Windows, Linux, Android, Meta Quest and iOS
from the tag, checks each carrier, and fills the draft. The draft's text already names the two macOS
packages. Watch it with `gh run watch`. A job that failed on something outside the tree is re-run
with `gh workflow run release.yml -f tag=vX.Y.0`.

## 7. Add the macOS packages, on a Mac

Once the draft exists, on a Mac:

```sh
git fetch --tags && git checkout vX.Y.0
task release:macos
```

That is three commands, which are the same step without `task`:

```sh
tools/platform/macos/installer.sh --notarize
tools/platform/macos/installer-remote.sh --notarize
tools/dist/release.sh --add --platforms macos
```

It first checks that the checkout is at the tag with no changed tracked file, so the packages are
built from what the tag names. `--add` uploads the two notarized packages to the draft and leaves
its text alone.

**The Mac can start as soon as the tag is pushed.** Its builds take about ten minutes and CI's about
forty, so the draft is usually not there yet when they finish. The run then uploads nothing and
prints the command that uploads what it built, `tools/dist/release.sh --add --platforms macos`. Run
that once the draft exists; it does not build again. See
[`CI builds the release, and a Mac adds its packages`](docs/decisions/distribution.md#ci-builds-the-release-and-a-mac-adds-its-packages).

## By hand, from a desk

Steps 6 and 7 without CI are:

```sh
tools/dist/release.sh           # gather what is staged into dist/release/<version>/
tools/dist/release.sh --upload  # ...and fill the draft release
```

**It gathers and never builds**, so every carrier has to be staged first.
[`Releases`](BUILDING.md#releases) lists the script behind each one, and `release.sh` names any that
is missing and stops.

**Name the platforms where a Mac is not to hand.** `--platforms windows,linux,android` carries those
and leaves the two `.pkg` files and the two `.ipa` files out of the count and off the page. It does
that rather than failing on four carriers the machine cannot build. Everything else holds: a named
platform whose carrier is missing still stops the run. See
[`A release page carries the platforms the machine cutting it can build`](docs/decisions/distribution.md#a-release-page-carries-the-platforms-the-machine-cutting-it-can-build).

## 8. Publish

Either way, the result is a **draft**. Somebody opens the page, sees all thirteen files on it, reads
it, and only then types this:

```sh
gh release edit vX.Y.0 --draft=false
```
