#!/usr/bin/env bash
#
# Gathers this version's carriers under one folder of release names, and uploads them to the draft
# GitHub release for the tag.
#
#   tools/dist/release.sh                      # gather into dist/release/<version>/ and report
#   tools/dist/release.sh --upload             # ...and create or update the draft release
#   tools/dist/release.sh --platforms windows,linux,android   # ...the carriers one machine builds
#   tools/dist/release.sh --upload --platforms windows,linux,android,ios --elsewhere macos
#                                              # ...and a body that also names what a Mac adds later
#   tools/dist/release.sh --add --platforms macos   # upload these carriers to the draft, body untouched
#   tools/dist/release.sh --check-tag          # is this checkout at the version's tag, unchanged?
#   tools/dist/release.sh --notes-file <path>  # a body other than tools/dist/release-notes.md
#   tools/dist/release.sh -v
#
#   dist/release/<version>/            every asset under the name it is published as
#   dist/release/<version>-notes.md    the body, rendered from tools/dist/release-notes.md
#
# **It gathers; it does not build.** Twelve carriers, six build systems, three of them in Docker, one
# needing a JDK and two a Mac: a script that ran all of them would be a release cut by whoever typed
# one word, from artifacts nobody had looked at. So each row below names the command that produces
# it, a missing artifact is reported by name rather than skipped, and the run stops. `BUILDING.md`
# has the twelve commands in order.
#
# **`--platforms` is for the release no one machine can cut.** The two `.pkg` files and the two
# `.ipa` files are built on a Mac and the rest are not, so a machine without one has four carriers it
# is not going to produce and a refusal it can do nothing about. Naming the platforms this run
# carries takes those rows out of the table, out of the count, and out of the body's download table,
# and leaves every other refusal exactly as it was: a carrier of a platform that *was* named and is
# not staged still stops the run.
#
# **The body names what the run gathered, plus what `--elsewhere` says is coming, so one run writes
# the page.** The release workflow builds everything but the two `.pkg` files, and a Mac adds those
# to the same draft. `--elsewhere macos` keeps their rows and their prose in the body, and the check
# that every row names an asset accepts a row of an elsewhere platform as one still to come.
#
# **`--add` is the second machine's half.** It gathers the named platforms through the same table,
# so the macOS row still takes only the notarized package, and uploads them to the existing draft
# without rewriting the body. It never gathers the carol pack, which the first run already uploaded,
# and it refuses when there is no draft to add to. The decisions are
# `A release page carries the platforms the machine cutting it can build` and
# `CI builds the release, and a Mac adds its packages` in docs/decisions/distribution.md.
#
# **The table is the point of the file.** A release name is written down once, here, and the copy
# into `dist/release/<version>/` happens before anything is uploaded -- so the folder can be read,
# and an asset's name can be wrong in a place that costs nothing to correct rather than on a page
# somebody has already been sent.
#
# **The body is a tracked file**, `tools/dist/release-notes.md`, rendered beside the folder of
# assets. A release page is read by somebody choosing a download, so it is held to the register in
# `What a release page says, and to whom` in docs/decisions/distribution.md -- and a tracked file is
# what puts it in front of `check-prose.sh` and `check-no-local-refs.sh`, both of which select what
# they read through `git ls-files`. A body typed at the point of upload is read by neither.
#
# `@VERSION@` and `@CAROLS@` are substituted from this run, so the file itself carries no version
# number and no asset name that a rebuild can change.
#
# **Both APKs are called `app-release.apk`**, one per Gradle project, which is the reason renaming is
# a step and not a convenience: two files of one name cannot both be assets, and a `.deb` that is
# already named for its package and version can travel as it is.
#
# **Both `.ipa`s say `unsigned` in the name, and that is the product rather than a lesser build of
# it.** iOS has no signature this repository can put on a file a stranger installs, so the person
# installing signs it with their own Apple ID; `README.md` is where that is written for them.
#
# **The macOS row takes the notarized package and no other.** `tools/platform/macos/installer.sh`
# writes the signed-only and the ad-hoc build into the same folder, under names ending in
# `-unnotarized` and `-unsigned`, and a release carries neither: a package Gatekeeper refuses on the
# machine that downloaded it is worse than no package at all.
#
# **The draft flag is never cleared here.** `--upload` creates a draft and fills it; publishing is
# `gh release edit v<version> --draft=false`, typed by somebody who has opened the page. A release
# that already exists and is *not* a draft is refused outright rather than clobbered, because
# replacing an asset on a page people have been sent is not something a staging script should be able
# to do by accident.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=dist-release

ALL_PLATFORMS="windows macos linux android ios any"

UPLOAD=0
ADD=0
CHECK_TAG=0
NOTES_FILE="tools/dist/release-notes.md"
PLATFORMS="$ALL_PLATFORMS"
PLATFORMS_NAMED=0
ELSEWHERE=""

# A comma-separated list of platform names, checked against the ones this file knows.
platform_list() { # <option> <list>  -> prints the names space-separated, or fails
  local p list
  list="$(printf '%s' "$2" | tr ',' ' ')"
  for p in $list; do
    case " $ALL_PLATFORMS " in
      *" $p "*) ;;
      *) echo "dist-release: $1 names no such platform -- $p" >&2
         echo "              one or more of: $ALL_PLATFORMS" >&2
         return 1 ;;
    esac
  done
  printf '%s' "$list"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --upload) UPLOAD=1 ;;
    --add) ADD=1; UPLOAD=1 ;;
    --check-tag) CHECK_TAG=1 ;;
    --elsewhere)
      shift
      [ $# -gt 0 ] || { echo "dist-release: --elsewhere needs a list" >&2; exit 2; }
      ELSEWHERE="$(platform_list --elsewhere "$1")" || exit 2
      ;;
    --notes-file)
      shift
      [ $# -gt 0 ] || { echo "dist-release: --notes-file needs a path" >&2; exit 2; }
      NOTES_FILE="$1"
      ;;
    --platforms)
      shift
      [ $# -gt 0 ] || { echo "dist-release: --platforms needs a list" >&2; exit 2; }
      PLATFORMS="$(platform_list --platforms "$1")" || exit 2
      PLATFORMS_NAMED=1
      ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/dist/release.sh [--upload] [--platforms <list>] [--elsewhere <list>] [--notes-file <path>] [-v]"
      echo "       tools/dist/release.sh --add --platforms <list> [-v]"
      echo "       tools/dist/release.sh --check-tag"
      exit 0
      ;;
    *) echo "dist-release: unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

# `any` carries what belongs to no platform, so a run that writes the page never opts out of it. An
# `--add` run is the one that leaves it out: the carol pack went up with the page.
if [ "$ADD" -eq 1 ]; then
  if [ "$PLATFORMS_NAMED" -eq 0 ]; then
    echo "dist-release: --add needs --platforms, naming what this machine adds" >&2
    exit 2
  fi
  if [ -n "$ELSEWHERE" ]; then
    echo "dist-release: --add leaves the body alone, so --elsewhere has nothing to say" >&2
    exit 2
  fi
  PLATFORMS="$(printf ' %s ' "$PLATFORMS" | sed 's/ any / /g; s/^ *//; s/ *$//')"
else
  case " $PLATFORMS " in *" any "*) ;; *) PLATFORMS="$PLATFORMS any" ;; esac
fi

# `selected <platform>` and `elsewhere <platform>` are the two questions the rest of the file asks
# about a platform.
selected() { case " $PLATFORMS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
elsewhere() { case " $ELSEWHERE " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

for p in $ELSEWHERE; do
  if [ "$p" = "any" ]; then
    echo "dist-release: the carol pack belongs to no platform, so no other machine adds it" >&2
    exit 2
  fi
  if selected "$p"; then
    echo "dist-release: $p is both gathered here and added elsewhere; name it in one list" >&2
    exit 2
  fi
done

if [ ! -f "$NOTES_FILE" ]; then
  echo "dist-release: no such notes file -- $NOTES_FILE" >&2
  exit 2
fi

# From the manifest, not from an artifact, and for the reason tools/dist/bin.sh gives: this script is
# looking for the artifacts and so has nothing to run yet. One number covers every product; see
# `One version number for the whole repository` in docs/decisions/repository.md.
VERSION="$(dist_manifest_version)"
TAG="v$VERSION"
OUT="dist/release/$VERSION"

# -- the checkout a second machine builds from -----------------------------------------------------

# **What `--add` uploads is built from what the tag names, or not at all.** The first run's carriers
# come from a checkout of the tag, and a package built from a commit after it would sit on the same
# page under the same version. So the checkout has to be at the tag, with no change to a tracked
# file. `--check-tag` asks this alone, so `task release:macos` can ask before ten minutes of builds.
at_tag() {
  local want
  if ! want="$(git rev-parse -q --verify "refs/tags/$TAG^{commit}")"; then
    echo "dist-release: there is no tag $TAG in this checkout -- git fetch --tags" >&2
    return 1
  fi
  if [ "$(git rev-parse HEAD)" != "$want" ]; then
    echo "dist-release: this checkout is not at $TAG, so what it builds is not what $TAG names." >&2
    echo "              git checkout $TAG" >&2
    return 1
  fi
  if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "dist-release: this checkout changes tracked files, so what it builds is not $TAG" >&2
    return 1
  fi
}

if [ "$CHECK_TAG" -eq 1 ]; then
  at_tag || exit 2
  dist_step "at $TAG"
  exit 0
fi
[ "$ADD" -eq 0 ] || at_tag || exit 2

# -- the table -------------------------------------------------------------------------------------

# Five fields: the platform the carrier belongs to, the directory to look in, the name to look for
# there, the name it is published under, and the command that produces it. A `=` in the fourth field
# keeps the source's own name, which is right wherever the build already names the file for its
# product and version.
#
# **The platform is what `--platforms` selects on**, and `any` is the carrier that belongs to none --
# the carol pack is a song package and a Debian box downloads the same one a phone does, so it is
# never what a release leaves out. A row whose platform is not selected is not gathered and is not
# missing: the four fields after it describe a file this run was never going to look for.
#
# **The fifth field is a command somebody can type**, which is the whole of its value: it is printed
# beside a carrier that is not staged, to a reader who is about to run it. The per-platform staging
# tasks are `internal:` in `Taskfile.yml` and Task refuses one by name, so `dist:setup` is what a
# Windows row names -- it picks the host's installer, which on the machine missing that `.exe` is the
# one that makes it.
#
# The patterns end in a glob where the part after the version is the build's to choose: an
# architecture the installer names, the triple the container built, the carol pack's own version.
# Two matches is an error rather than a choice, which is what `one_match` is for -- a `dist/carols`
# holding two packs would otherwise upload whichever sorted first, for ever.
#
# **The macOS pattern ends at that glob rather than after it**, because what follows the architecture
# on the other two packages is a signing marker. Both architectures the installer names end in a
# digit and no marker does, so `[0-9]` is what keeps the ad-hoc build off a release page.
release_rows() {
  cat <<ROWS
windows|dist/setup/windows|karaokemachine-setup-$VERSION-windows-*.exe|=|task dist:setup
macos|dist/setup/macos|karaokemachine-setup-$VERSION-macos-*[0-9].pkg|=|task dist:setup:notarized
windows|dist/setup/windows|km-remote-setup-$VERSION-windows-*.exe|=|task dist:setup:remote
macos|dist/setup/macos|km-remote-setup-$VERSION-macos-*[0-9].pkg|=|task dist:setup:remote:notarized
linux|dist/karaokemachine/linux|karaokemachine_$VERSION-1_*.deb|=|task dist:deb
linux|dist/karaokemachine-tools/linux|karaokemachine-tools_$VERSION-1_*.deb|=|task dist:deb:tools
linux|dist/karaokemachine/linux|karaokemachine-$VERSION-*.tar.gz|=|task dist:tarball
android|ports/machine/android/app/build/outputs/apk/release|app-release.apk|karaokemachine-$VERSION-android.apk|task build:android RELEASE=1
android|ports/remote/android/app/build/outputs/apk/release|app-release.apk|km-remote-$VERSION-android.apk|task build:android:remote RELEASE=1
ios|dist/karaokemachine/ios|karaokemachine-$VERSION-ios-unsigned.ipa|=|task build:ios RELEASE=1 DEVICE=1 IPA=1
ios|dist/km-remote/ios|km-remote-$VERSION-ios-unsigned.ipa|=|task build:ios:remote RELEASE=1 DEVICE=1 IPA=1
any|dist/carols|*.kmpkg|=|task carols
ROWS
}

# The one matching file in a directory, or a failure that says which of the two ways it went wrong.
# `find -maxdepth 1` rather than a bare glob so that a missing directory and a missing file are the
# same answer, and so nothing depends on `nullglob` being set.
one_match() { # <dir> <name pattern>  -> prints the path, or fails
  local dir="$1" pat="$2" found n
  [ -d "$dir" ] || return 1
  found="$(find "$dir" -maxdepth 1 -type f -name "$pat" | sort)"
  [ -n "$found" ] || return 1
  n="$(printf '%s\n' "$found" | wc -l | tr -d ' ')"
  if [ "$n" -ne 1 ]; then
    echo "dist-release: $dir/$pat matched $n files; expected one" >&2
    printf '  %s\n' "$found" >&2
    return 1
  fi
  printf '%s' "$found"
}

# KiB below a megabyte, because the carol pack is tens of kilobytes and `0.0 MiB` beside it reads as
# a carrier that failed to build.
human() { # <bytes>  -> prints a size
  awk -v b="$1" 'BEGIN {
    if (b < 1048576) { printf "%.0f KiB", b / 1024 } else { printf "%.1f MiB", b / 1048576 }
  }'
}

# -- gather ----------------------------------------------------------------------------------------

dist_step "gathering $TAG"
dist_detail "out   $OUT"
[ "$PLATFORMS" = "$ALL_PLATFORMS" ] || dist_detail "for   $(printf '%s' "$PLATFORMS" | sed 's/ any$//')"

mkdir -p "$OUT"
dist_clear "$OUT"

missing=0
count=0
skipped=0
# What the carriers of a platform this run leaves out would have been called, as globs: the body's
# download table is filtered against these below, so a row naming one comes out with it.
#
# **A glob rather than a name, because the file is not there to ask.** A row publishing under the
# source's own name knows only the pattern until it has matched something on disk, and the carrier
# of a platform this machine cannot build never will. The pattern is what the body's name has to
# match anyway -- `karaokemachine-setup-1.2.0-macos-*[0-9].pkg` covers the architecture the table
# spells out.
#
# **A row of an `--elsewhere` platform is neither gathered nor omitted.** Its glob goes into
# `awaited_globs` instead, which keeps its line in the body and lets the check below accept a name
# that has no file here yet.
omitted_globs=""
awaited_globs=""
awaited=0
while IFS='|' read -r platform dir pat name cmd; do
  [ -n "$dir" ] || continue
  if ! selected "$platform"; then
    [ "$name" = "=" ] && name="$pat"
    if elsewhere "$platform"; then
      awaited_globs="$awaited_globs|$name"
      awaited=$((awaited + 1))
    else
      omitted_globs="$omitted_globs|$name"
      skipped=$((skipped + 1))
    fi
    continue
  fi
  if ! src="$(one_match "$dir" "$pat")"; then
    echo "dist-release: not staged -- $dir/$pat" >&2
    echo "              build it with: $cmd" >&2
    missing=$((missing + 1))
    continue
  fi
  [ "$name" = "=" ] && name="$(basename "$src")"
  cp "$src" "$OUT/$name"
  count=$((count + 1))
  printf '   %-54s %s\n' "$name" "$(human "$(wc -c < "$src" | tr -d ' ')")"
done <<EOF
$(release_rows)
EOF

if [ "$missing" -gt 0 ]; then
  echo "dist-release: $missing of $((count + missing)) assets are not staged; nothing was uploaded" >&2
  exit 2
fi

# -- the key the APKs are signed with ----------------------------------------------------------------

# **A debug-signed APK is refused here and nowhere else.** The build only reports which key it used,
# because a build is a thing somebody watches; a release is a page people are sent, and the debug key
# is per machine, so an APK signed with it installs over nothing and anybody's own debug key replaces
# it. `KM_ANDROID_KEYSTORE` unset is the state that produces one -- see `How the Android applications
# are signed` in docs/decisions/remotes.md.
#
# **A machine with no apksigner says so rather than passing.** It lives in the Android SDK's
# build-tools and a release can be gathered on a machine that never builds an APK, so silence here
# would read as a check that ran. The same message covers an apksigner whose output names no
# certificate, which is the other way `--cn` fails.
#
# Through `bash`, because the script's executable bit does not survive a commit made on Windows.
for apk in "$OUT"/*.apk; do
  [ -f "$apk" ] || continue
  if ! cn="$(bash tools/port/apk-signer.sh --cn "$apk")"; then
    echo "dist-release: no apksigner, or none that named a certificate, so $(basename "$apk")" >&2
    echo "              was not checked for its signing key" >&2
    continue
  fi
  if [ "$cn" = "Android Debug" ]; then
    echo "dist-release: $(basename "$apk") is signed with the debug key." >&2
    echo "              Set KM_ANDROID_KEYSTORE and build it again; nothing was uploaded." >&2
    exit 2
  fi
  dist_detail "signed $(basename "$apk") -- $cn"
done

need_gh() {
  command -v gh >/dev/null 2>&1 || {
    echo "dist-release: gh is not installed -- https://cli.github.com" >&2
    exit 2
  }
}

# -- adding to a draft another run made ------------------------------------------------------------

# **The draft has to be there already, and still a draft.** The run that wrote the body made it;
# an `--add` that created one would make a page with no text and nothing but this machine's files.
if [ "$ADD" -eq 1 ]; then
  need_gh
  dist_step "adding to draft release $TAG"
  if ! state="$(gh release view "$TAG" --json isDraft --jq .isDraft 2>/dev/null)"; then
    echo "dist-release: there is no release $TAG to add to." >&2
    echo "              The run that writes its body creates it; --add comes after that." >&2
    exit 2
  fi
  if [ "$state" != "true" ]; then
    echo "dist-release: $TAG is already published, so its assets are not replaced here." >&2
    echo "              Upload to a published release by hand, deliberately." >&2
    exit 2
  fi
  dist_run "gh release upload" gh release upload "$TAG" --clobber "$OUT"/*
  dist_step "added $count assets to $TAG"
  echo "   gh release view $TAG --web"
  echo "   (still a draft: gh release edit $TAG --draft=false publishes it)"
  exit 0
fi

# -- the body --------------------------------------------------------------------------------------

# Rendered beside the folder it describes, and for the reason the table is written down once: a
# sentence can be wrong in a place that costs nothing to correct rather than on a page somebody has
# already been sent.
#
# **Beside `$OUT` and not inside it**, because everything in `$OUT` is uploaded as an asset and a
# release page carrying its own text as a download is not what any of this is for.
#
# The carol pack's name is read back out of `$OUT` rather than carried out of the loop, so the body
# names the file that is about to be uploaded and not the one a row expected. `one_match` gives the
# same refusal here as it does there.
NOTES="dist/release/$VERSION-notes.md"
carols="$(basename "$(one_match "$OUT" '*.kmpkg')")"

# **A page never names a download it does not have**, so what the body says about a platform this
# run leaves out comes out of it. Two shapes carry that, and neither is a second copy of the table
# above:
#
# * A download table row is matched against the globs of the rows that were skipped. The body spells
#   an architecture out where the table globs it, which is why this is a glob match and not a set
#   membership test.
# * Prose is wrapped in `<!-- platform: <name> -->` and `<!-- /platform -->`. A comment is invisible
#   wherever Markdown is rendered, so the tracked file reads as the whole page to somebody editing
#   it, and a section added for a platform later is dropped by the marker rather than by this script
#   learning its heading.
#
# The markers come out on every run, selected or not: they are how the file is written, not
# something a reader of the page has any use for. An `--elsewhere` platform's prose stays, because
# its files are on the page before anybody publishes it.
#
# **The substitution runs first**, because the globs being matched against carry this run's version
# and a body still holding `@VERSION@` matches none of them.
sed -e "s/@VERSION@/$VERSION/g" -e "s/@CAROLS@/$carols/g" "$NOTES_FILE" |
awk -v selected="$PLATFORMS $ELSEWHERE" -v omitted="$omitted_globs" '
  function is_selected(p,   i, n, a) {
    n = split(selected, a, " ")
    for (i = 1; i <= n; i++) if (a[i] == p) return 1
    return 0
  }
  # A row naming a carrier of a platform this run skipped. The first cell is a file name when it
  # holds a dot, which is the same test the check below the render makes.
  function row_is_omitted(line,   name, i, n, g) {
    if (line !~ /^\| `[^`]*\.[A-Za-z0-9]*` \|/) return 0
    name = line
    sub(/^\| `/, "", name)
    sub(/`.*$/, "", name)
    n = split(omitted, g, "|")
    for (i = 1; i <= n; i++) if (g[i] != "" && name ~ "^" glob2re(g[i]) "$") return 1
    return 0
  }
  # A star and a bracket range are the whole of what a release name globs.
  function glob2re(g,   out, i, c) {
    out = ""
    for (i = 1; i <= length(g); i++) {
      c = substr(g, i, 1)
      if (c == "*") out = out ".*"
      else if (c == "[" || c == "]" || c == "-") out = out c
      else if (c ~ /[A-Za-z0-9_]/) out = out c
      else out = out "\\" c
    }
    return out
  }
  /^<!-- platform: [a-z]+ -->$/ {
    p = $0; sub(/^<!-- platform: /, "", p); sub(/ -->$/, "", p)
    skip = !is_selected(p)
    next
  }
  /^<!-- \/platform -->$/ { skip = 0; next }
  skip { next }
  row_is_omitted($0) { next }
  # What a dropped block leaves behind is the blank line on each side of it, so a run of them
  # collapses to one and the page reads as though the section was never written.
  /^[[:space:]]*$/ { if (blank) next; blank = 1; print; next }
  { blank = 0; print }
' > "$NOTES"

if grep -q '@[A-Z]*@' "$NOTES"; then
  echo "dist-release: $NOTES_FILE has a placeholder this script does not substitute" >&2
  grep -n '@[A-Z]*@' "$NOTES" >&2
  exit 2
fi

# **The folder and the body name the same files, and each is checked against the other.** The table
# above decides what is uploaded; the download table in the body decides what a reader is told to
# download. Neither is derived from the other, so a carrier that reaches one and not the other
# arrives on the page either as a file nobody is told about or as a name that downloads nothing.
# Both directions are checked, because the two failures read nothing alike to somebody choosing a
# download and cost the same to catch here.
#
# A row's first cell is a file name when it holds a dot. The carol pack is named in the prose under
# the table rather than in it, and the first direction is what covers it.
#
# **A row naming an `--elsewhere` carrier is not a mismatch**: its file is the one `--add` uploads.
# The match is a glob for the reason the omitted rows' is, and `case` is bash's own glob match.
is_awaited() { # <name>
  local g
  local IFS='|'
  for g in $awaited_globs; do
    [ -n "$g" ] || continue
    # shellcheck disable=SC2254
    case "$1" in $g) return 0 ;; esac
  done
  return 1
}

mismatch=0

for asset in "$OUT"/*; do
  [ -f "$asset" ] || continue
  if ! grep -qF "$(basename "$asset")" "$NOTES"; then
    echo "dist-release: $(basename "$asset") is an asset the body never names" >&2
    mismatch=$((mismatch + 1))
  fi
done

while read -r named; do
  [ -n "$named" ] || continue
  if [ ! -f "$OUT/$named" ] && ! is_awaited "$named"; then
    echo "dist-release: the body's download table names $named, which is not an asset" >&2
    mismatch=$((mismatch + 1))
  fi
done <<EOF
$(sed -n 's/^| `\([^`]*\.[A-Za-z0-9]*\)` |.*/\1/p' "$NOTES")
EOF

if [ "$mismatch" -gt 0 ]; then
  echo "dist-release: $NOTES_FILE and $OUT disagree about $mismatch file(s); nothing was uploaded" >&2
  exit 2
fi

dist_detail "body  $NOTES"

summary="$count assets, $(human "$(dist_bytes "$OUT")")"
[ "$awaited" -eq 0 ] || summary="$summary -- $awaited to be added from another machine"
[ "$skipped" -eq 0 ] || summary="$summary -- $skipped left off the page"
dist_step "$summary"

if [ "$UPLOAD" -eq 0 ]; then
  echo "   (--upload creates or updates the draft release for $TAG)"
  exit 0
fi

# -- upload ----------------------------------------------------------------------------------------

need_gh

dist_step "draft release $TAG"

# A release that is already published is refused rather than clobbered. `gh release view` exits
# non-zero when there is none, which is the create path.
if state="$(gh release view "$TAG" --json isDraft --jq .isDraft 2>/dev/null)"; then
  if [ "$state" != "true" ]; then
    echo "dist-release: $TAG is already published, so its assets are not replaced here." >&2
    echo "              Upload to a published release by hand, deliberately." >&2
    exit 2
  fi
  dist_detail "exists, still a draft"
  # A draft's body is rewritten on every run, so correcting a sentence is a re-run rather than a
  # visit to the web page. The published release above is the one this script will not touch.
  dist_run "gh release edit" gh release edit "$TAG" --notes-file "$NOTES"
else
  dist_run "gh release create" \
    gh release create "$TAG" --draft --verify-tag \
      --title "karaokemachine $VERSION" --notes-file "$NOTES"
  dist_detail "created"
fi

# `--clobber` so a re-run after rebuilding one carrier replaces that asset instead of failing on
# every other one already up there.
dist_run "gh release upload" gh release upload "$TAG" --clobber "$OUT"/*

dist_step "uploaded $count assets to $TAG"
echo "   gh release view $TAG --web"
if [ -n "$ELSEWHERE" ]; then
  echo "   (the page names $awaited more: tools/dist/release.sh --add --platforms $(printf '%s' "$ELSEWHERE" | tr ' ' ','))"
fi
echo "   (still a draft: gh release edit $TAG --draft=false publishes it)"
