#!/usr/bin/env bash
#
# Builds the Christmas carol pack -- sixteen public-domain carols, as one `.kmpkg` somebody can
# download and drop into their packages folder.
#
#   tools/dist/carols.sh                 # fetch, convert, build, report
#   tools/dist/carols.sh -v              # ...and watch the conversion go past
#   tools/dist/carols.sh --keep-work     # keep the generated ABC and the .kar files
#
# ** This pack is a separate download and is never bundled with the machine. ** Nothing here writes
# into `assets/`, so no carrier changes and a fresh install still starts with an empty catalog on
# purpose. See the `A downloadable song pack` decision in docs/decisions/.
#
# ** The repository holds the recipe and the release holds the bytes. ** The carols themselves are
# not committed: they are generated from a pinned edition of somebody else's ABC sources, and
# `dist/` is gitignored like every other carrier's output. That also means `.gitignore`'s unanchored
# `*.kmpkg` needs no exception -- see the note there about how the wallpapers' shipped zip had to
# earn one.
#
# What it needs that a checkout does not carry:
#
#   * the network, once. The hymnal is fetched into the asset cache and verified against the digest
#     below, so a re-upload or a man-in-the-middle fails loudly rather than landing in a release.
#   * `abc2midi`. It is a **build-time** tool: nothing links it and nothing ships it, so its GPL
#     reaches no released artifact, exactly as `ffmpeg` the command does for `km-pack --features
#     video` and as Inno Setup does for the Windows installer.
#
#     **`task abcmidi` builds one into the asset cache, on any platform, once per machine** -- it
#     needs an ordinary C compiler and nothing else. A system package is preferred where there is
#     one (`apt install abcmidi`, `brew install abcmidi`) and is found first; there is no winget
#     package, which is the case the script exists for. `--abc2midi` names one anywhere else.
#
# The conversion itself, and why it is a Rust program rather than more of this script, is
# `tools/cmd/km-carols`. The short version: it checks every file it makes with the machine's own
# parser and scorer before writing it, so a carol that could not be sung from stops the build by
# name instead of reaching somebody's television.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=dist-carols
dist_assert_root

# Where the machine keeps large files a checkout does not carry. Outside the repository, so
# `cargo clean` and a fresh clone both leave it alone and sibling worktrees share one copy.
. tools/setup/asset-cache.sh

# -- the pin ---------------------------------------------------------------------------------------
#
# One edition, by name and by digest. The Open Hymnal restarted in October 2025 and later editions
# exist; this is deliberately not "the newest", because the pack selects its carols by `X:` reference
# number and a later edition may renumber. `km-carols` asserts every selected tune's title as well,
# so a re-pin that moved one stops the build rather than quietly swapping a carol.
#
# To move it: change all three lines together, run the build, and read the report.
CAROLS_SOURCE_NAME="OpenHymnal2014.06.abc"
CAROLS_SOURCE_URL="http://openhymnal.org/OpenHymnal2014.06.abc"
CAROLS_SOURCE_SHA256="f75551ce21cfe9439545f3c0d97b4e737a4689ca4c4594a5a51f76fb164daa46"

# The pack's own version, which is not the workspace's. It moves when the songs change, which is
# rarely and for its own reasons -- the same argument `tools/cmd/assets/km-wallpaper-pack` makes for keeping
# its own.
CAROLS_VERSION="1.0.0"

# The package identifier. Must match `selection::PACKAGE_ID`.
CAROLS_ID="c5a201f8e6b4d379"

# The stem of what comes out: `selection::PACKAGE_NAME` through the fold `km_kmpkg::name_slug`
# applies. Sixteen hexadecimal characters are not something to pick a download out of a browser's
# list by, which is the argument `What an installed package file is called` already makes about the
# machine's own folder.
CAROLS_SLUG="christmas-carols"

# -- arguments -------------------------------------------------------------------------------------

ABC2MIDI=""
KEEP_WORK=0

while [ $# -gt 0 ]; do
  case "$1" in
    -v|--verbose)  DIST_VERBOSE=1 ;;
    --abc2midi)    shift; ABC2MIDI="${1:-}" ;;
    --keep-work)   KEEP_WORK=1 ;;
    -h|--help)     sed -n '2,36p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)             echo "$DIST_SCRIPT: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

# -- what has to be here before anything is built ---------------------------------------------------

# Three places, in this order: what was named, what is on PATH, and what
# `tools/setup/fetch-abcmidi.sh` built into the asset cache. The cache is last so that a system
# package or a hand-built one already on PATH wins -- the cached copy is this repository's fallback
# for a machine that has none, not a preference over the machine's own.
if [ -z "$ABC2MIDI" ]; then
  ABC2MIDI="$(command -v abc2midi || true)"
fi
if [ -z "$ABC2MIDI" ] || [ ! -x "$ABC2MIDI" ]; then
  ABC2MIDI="$(bash tools/setup/fetch-abcmidi.sh --path)"
fi
if [ -z "$ABC2MIDI" ] || [ ! -x "$ABC2MIDI" ]; then
  cat >&2 <<'MISSING'
dist-carols: abc2midi is not here, and it is what turns the hymnal's ABC into MIDI.

  task abcmidi        (or: tools/setup/fetch-abcmidi.sh)

builds it into the asset cache, once per machine. It needs an ordinary C compiler and nothing
else -- no libraries and no build system -- and that script says what to install if there is none.
It is built rather than downloaded because the project publishes no binaries at all.

A system package works just as well and is preferred when it is there: `apt install abcmidi`,
`brew install abcmidi`. There is no winget package, which is why the script exists.

Or name one: tools/dist/carols.sh --abc2midi /path/to/abc2midi
MISSING
  exit 1
fi

# `shasum` on macOS and the BSDs, `sha256sum` on most Linuxes. One set is always there. The same
# two-way check tools/setup/fetch-assets.sh makes, for the same reason.
if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  echo "$DIST_SCRIPT: need sha256sum or shasum to verify the download" >&2
  exit 1
fi

# -- fetch ------------------------------------------------------------------------------------------

mkdir -p "$CACHE"
SOURCE="$CACHE/$CAROLS_SOURCE_NAME"

dist_step "the hymnal"
if [ -f "$SOURCE" ] && [ "$(sha256 "$SOURCE")" = "$CAROLS_SOURCE_SHA256" ]; then
  dist_detail "cached $SOURCE"
else
  # Downloaded to a temporary name and moved into place, so an interrupted run cannot leave a half
  # file looking cached.
  dist_run "fetching $CAROLS_SOURCE_NAME" \
    curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$SOURCE.part" "$CAROLS_SOURCE_URL"
  got="$(sha256 "$SOURCE.part")"
  if [ "$got" != "$CAROLS_SOURCE_SHA256" ]; then
    rm -f "$SOURCE.part"
    echo "$DIST_SCRIPT: $CAROLS_SOURCE_NAME failed verification" >&2
    echo "  expected $CAROLS_SOURCE_SHA256" >&2
    echo "  got      $got" >&2
    exit 1
  fi
  mv "$SOURCE.part" "$SOURCE"
fi
dist_detail "abc2midi $ABC2MIDI"

# -- convert ----------------------------------------------------------------------------------------

# `dist/carols/` and not `dist/<app>/<platform>/`, and the exception is deliberate rather than an
# oversight: the layout rule exists so every build of one product sits together, and this artifact
# has **no platform axis at all**. A `.kmpkg` is the same bytes on Windows, macOS, Linux and Android.
# Inventing an `any/` level to satisfy the shape would say something untrue about the file.
OUT="dist/carols"
WORK="$OUT/work"

dist_clear "$OUT"
mkdir -p "$WORK"

started=$SECONDS
dist_step "building the songs"
cargo run $(dist_cargo_quiet) -p km-carols -- \
  --source "$SOURCE" \
  --out "$WORK" \
  --abc2midi "$ABC2MIDI" \
  --pack-version "$CAROLS_VERSION" \
  $([ "$KEEP_WORK" -eq 1 ] && printf -- '--keep-abc')
dist_detail "converted in $(dist_elapsed "$started")"

# -- package -----------------------------------------------------------------------------------------
#
# `km-pack build` and never a second path of our own: km-carols writes the same `.kmspec.yaml` a
# person would write by hand, and this builds it exactly as `km-pack build` would build theirs. Two
# tools writing subtly different manifests from the same songs is the defect that arrangement exists
# to make impossible.
PKG="$OUT/$CAROLS_SLUG-$CAROLS_VERSION.kmpkg"

dist_step "building the package"
dist_run "km-pack build" \
  cargo run $(dist_cargo_quiet) -p km-pack -- build "$WORK/$CAROLS_ID.kmspec.yaml" --out "$PKG"

cp "$WORK/CREDITS.md" "$OUT/CREDITS.md"

if [ "$KEEP_WORK" -eq 0 ]; then
  rm -rf "$WORK"
else
  dist_detail "kept the working files in $WORK"
fi

# -- report -------------------------------------------------------------------------------------------

dist_step "what came out"
cargo run $(dist_cargo_quiet) -p km-pack -- check "$PKG"

printf '\n%s\n' "wrote $PKG ($(( $(dist_bytes "$OUT") / 1024 )) KiB) with CREDITS.md beside it"
printf '%s\n' "Install it by dropping it in the folder \`karaokemachine --show-paths\` calls packages,"
printf '%s\n' "or without a restart by uploading it -- which is an owner's act and wants the password:"
printf '%s\n' "  TOKEN=\$(curl -s -H 'content-type: application/json' -d '{\"password\": \"<the machine password>\"}' \\"
printf '%s\n' "          http://<machine>:8177/api/v1/admin/login | sed -n 's/.*\"token\":\"\\([^\"]*\\)\".*/\\1/p')"
printf '%s\n' "  curl -H \"authorization: Bearer \$TOKEN\" -F file=@$PKG \\"
printf '%s\n' "       http://<machine>:8177/api/v1/admin/packages/upload"
