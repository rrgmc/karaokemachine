#!/usr/bin/env bash
#
# Builds `abc2midi` into the asset cache, once per machine.
#
#   tools/setup/fetch-abcmidi.sh            # build it if it is not there; or: task abcmidi
#   tools/setup/fetch-abcmidi.sh --force    # build it again
#   tools/setup/fetch-abcmidi.sh --path     # print where it is and build nothing
#
# `abc2midi` is what `tools/dist/carols.sh` uses to turn the Open Hymnal's ABC into karaoke MIDI.
# It is a **build-time** tool on the same footing as `ffmpeg` the command and Inno Setup: nothing
# links it, nothing ships it, and its GPL therefore reaches no released artifact.
#
# ** It is built rather than downloaded, and that is not a preference. ** There is nothing official
# to download: the project publishes no GitHub releases, its SourceForge file area lists no
# binaries, and the author's own site -- which the GitHub repository still points at -- is 404. So
# the choice was between an unofficial third-party binary of unknown provenance and eight C files
# with no dependencies, and this repository has already answered that question once, when surveying
# instrument banks: a permissive claim with no provenance is not evidence. Building
# takes a couple of seconds.
#
# Pinned by tag **and** by the checksum of the archive, so a re-tag, a re-upload or a
# man-in-the-middle fails loudly rather than landing in a release. Note the failure mode this shares
# with every archive pin here: GitHub generates these tarballs rather than storing them, and has
# changed how once. If the digest stops matching and the tag has not moved, that is what happened --
# re-pin deliberately, do not delete the check.
#
# ** How the compiler is chosen: by trying, not by guessing. ** Each candidate is asked to build the
# thing, and the first that produces a working binary wins. That is the whole rule, and it is
# deliberately not a table of platforms and toolchains -- such a table would encode whatever the
# author had installed, which is exactly what this must not do. Set `CC` to force one.
#
# What is worth knowing before "improving" that loop by preferring clang on Windows: **a
# clang that targets MSVC cannot build this**, and it fails in a way that looks like a bug in this
# script. abcMIDI carries `#define snprintf _snprintf` under an `_MSC_VER` guard for the benefit of
# twenty-year-old compilers, and a modern UCRT `stdio.h` answers that with `#error Macro definition
# of snprintf conflicts with Standard Library function declaration`. A MinGW gcc or clang has no
# `_MSC_VER`, never takes that branch, and builds it in one line. The loop below discovers that by
# itself, which is why it is a loop.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || {
  echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

# Where a machine keeps the large things a checkout does not carry. Outside the repository, so
# `cargo clean` and a fresh clone leave it alone and every worktree shares one copy.
. tools/setup/asset-cache.sh

# -- the pin ---------------------------------------------------------------------------------------
#
# The project tags by date and publishes no releases. Move all three lines together.
ABCMIDI_TAG="2026.06.16"
ABCMIDI_URL="https://codeload.github.com/sshlien/abcmidi/tar.gz/refs/tags/2026.06.16"
ABCMIDI_SHA256="eb5f57f5e42356a7eaf3d283cf8846543a25756719bbceda7a7fc083609e9262"

# The eight translation units `abc2midi` is made of. The project's own makefile is the source of
# this list; it builds several other programs from the same directory and we want one.
ABCMIDI_SOURCES=(parseabc.c store.c genmidi.c midifile.c queues.c parser2.c stresspat.c music_utils.c)

# `ANSILIBS` tells the source it may include the standard headers. Without it `parseabc.c` declares
# `char *malloc()` in the pre-ANSI style and every compiler made this century refuses it.
ABCMIDI_CFLAGS=(-O2 -DANSILIBS)

# -- arguments -------------------------------------------------------------------------------------

FORCE=0
PATH_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --force)   FORCE=1 ;;
    --path)    PATH_ONLY=1 ;;
    -h|--help) sed -n '2,42p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)         echo "${0##*/}: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;;
  *)                    EXE="" ;;
esac

DEST="$CACHE/abcmidi"
BIN="$DEST/abc2midi$EXE"

# `--path` is for a caller that wants the location and not the work. Everything else this script
# prints goes to stderr under it, so the one thing on stdout is the path -- the same contract
# `tools/setup/fetch-assets.sh --cache-only` makes.
if [ "$PATH_ONLY" -eq 1 ]; then
  printf '%s\n' "$BIN"
  exit 0
fi

if [ -x "$BIN" ] && [ "$FORCE" -eq 0 ]; then
  echo "cached    $("$BIN" -ver 2>/dev/null | head -1)"
  echo "          $BIN"
  exit 0
fi

# -- fetch ------------------------------------------------------------------------------------------

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  echo "${0##*/}: need sha256sum or shasum to verify the download" >&2
  exit 1
fi

mkdir -p "$CACHE" "$DEST"
ARCHIVE="$CACHE/abcmidi-$ABCMIDI_TAG.tar.gz"

if [ -f "$ARCHIVE" ] && [ "$(sha256 "$ARCHIVE")" = "$ABCMIDI_SHA256" ]; then
  echo "cached    abcmidi-$ABCMIDI_TAG.tar.gz"
else
  echo "fetching  abcmidi-$ABCMIDI_TAG.tar.gz"
  curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$ARCHIVE.part" "$ABCMIDI_URL"
  got="$(sha256 "$ARCHIVE.part")"
  if [ "$got" != "$ABCMIDI_SHA256" ]; then
    rm -f "$ARCHIVE.part"
    echo "${0##*/}: abcmidi-$ABCMIDI_TAG.tar.gz failed verification" >&2
    echo "  expected $ABCMIDI_SHA256" >&2
    echo "  got      $got" >&2
    exit 1
  fi
  mv "$ARCHIVE.part" "$ARCHIVE"
fi

# -- build ------------------------------------------------------------------------------------------

BUILD="$(mktemp -d)"
trap 'rm -rf "$BUILD"' EXIT
tar xzf "$ARCHIVE" -C "$BUILD"

SRC="$BUILD/abcmidi-$ABCMIDI_TAG"
[ -d "$SRC" ] || SRC="$(find "$BUILD" -maxdepth 1 -mindepth 1 -type d | head -1)"
[ -d "$SRC" ] || { echo "${0##*/}: the archive held no source directory" >&2; exit 1; }

# The candidates, in the order they are tried. `CC` first so anything unusual can be named; the rest
# are the three names a C compiler answers to. Whichever builds it, wins -- see the header.
CANDIDATES=()
[ -n "${CC:-}" ] && CANDIDATES+=("$CC")
CANDIDATES+=(cc gcc clang)

built=""
log="$(mktemp)"
for candidate in "${CANDIDATES[@]}"; do
  command -v "$candidate" >/dev/null 2>&1 || continue
  echo "building  with $candidate"
  if ( cd "$SRC" && "$candidate" "${ABCMIDI_CFLAGS[@]}" -o "abc2midi$EXE" "${ABCMIDI_SOURCES[@]}" ) \
       >"$log" 2>&1 && [ -x "$SRC/abc2midi$EXE" ]; then
    built="$candidate"
    break
  fi
  echo "          $candidate could not build it; trying the next"
done

if [ -z "$built" ]; then
  echo >&2
  echo "${0##*/}: no C compiler here could build abc2midi." >&2
  if [ -s "$log" ]; then
    echo "The last one said:" >&2
    tail -20 "$log" >&2
    echo >&2
  fi
  cat >&2 <<'MISSING'
It needs an ordinary C compiler and nothing else -- no libraries, no build system.

  Debian/Ubuntu   sudo apt install build-essential
  Fedora          sudo dnf install gcc
  Arch            sudo pacman -S gcc
  macOS           xcode-select --install
  Windows         winget install BrechtSanders.WinLibs.POSIX.UCRT
                  ...then open a new shell so gcc is on PATH.

On Windows, note that a clang targeting MSVC will NOT do: abcMIDI defines `snprintf` for the
benefit of very old compilers and a modern UCRT header refuses that. A MinGW gcc is the answer,
which is what the winget package above installs.

`CC=/path/to/compiler tools/setup/fetch-abcmidi.sh` names one that is not on PATH.
MISSING
  rm -f "$log"
  exit 1
fi
rm -f "$log"

install -m 755 "$SRC/abc2midi$EXE" "$BIN" 2>/dev/null || { cp "$SRC/abc2midi$EXE" "$BIN"; chmod 755 "$BIN"; }

echo "built     $("$BIN" -ver 2>/dev/null | head -1) with $built"
echo "          $BIN"
