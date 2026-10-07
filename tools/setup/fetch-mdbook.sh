#!/usr/bin/env bash
#
# Fetches the pinned `mdbook` into the asset cache, once per machine.
#
#   bash tools/setup/fetch-mdbook.sh            # fetch it if it is not there; or: task mdbook
#   bash tools/setup/fetch-mdbook.sh --force    # fetch it again
#   bash tools/setup/fetch-mdbook.sh --path     # print where it is and fetch nothing
#
# `mdbook` is what `tools/dist/site.sh` uses to turn `docs/manual/` into the pages under `docs/` on
# the site. It is a **build-time** tool on the same footing as `abc2midi` and Inno Setup: nothing
# links it and nothing ships it.
#
# ** It is downloaded rather than built, because the project publishes a binary per platform. ** A
# `cargo install mdbook` compiles for minutes and needs a Rust toolchain, which the pages workflow
# deliberately has none of. The release archive is one file and takes seconds.
#
# ** One version, and the cache is the only place `site.sh` looks. ** A newer `mdbook` on PATH would
# render the same chapters into different markup, and the checks `site.sh` runs over the result would
# then pass on one machine and fail on another. The version is in the cached folder's name, so a
# moved pin is a new download rather than a stale binary.
#
# Pinned by version **and** by the checksum of each archive, so a re-upload or a man-in-the-middle
# fails loudly. The digests are the ones the release page publishes beside each asset.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || {
  echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

# Where a machine keeps the large things a checkout does not carry. Outside the repository, so
# `cargo clean` and a fresh clone leave it alone and every worktree shares one copy.
. tools/setup/asset-cache.sh

# -- the pin ---------------------------------------------------------------------------------------
#
# Move the version and every digest together.
MDBOOK_VERSION="0.5.4"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    MDBOOK_ASSET="mdbook-v$MDBOOK_VERSION-x86_64-unknown-linux-musl.tar.gz"
    MDBOOK_SHA256="5222beabd3e37dc5be0d18ff99b79058469354db5c220153a1b92db5ba12be89" ;;
  Linux-aarch64|Linux-arm64)
    MDBOOK_ASSET="mdbook-v$MDBOOK_VERSION-aarch64-unknown-linux-musl.tar.gz"
    MDBOOK_SHA256="753e5c5c363ee8a56972344dcf91466f005a51db84a7aeffe427ae3ef83d6d44" ;;
  Darwin-arm64)
    MDBOOK_ASSET="mdbook-v$MDBOOK_VERSION-aarch64-apple-darwin.tar.gz"
    MDBOOK_SHA256="03e8a6d8b13a2971e0b3280affd03b388373c1485e26f73407c3a76b0b1838df" ;;
  Darwin-x86_64)
    MDBOOK_ASSET="mdbook-v$MDBOOK_VERSION-x86_64-apple-darwin.tar.gz"
    MDBOOK_SHA256="a47d7bf0d5d670cff9ee6cce95537cbeb62dc10704d9e7131ffbd13e2b59a5de" ;;
  MINGW*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64)
    MDBOOK_ASSET="mdbook-v$MDBOOK_VERSION-x86_64-pc-windows-msvc.zip"
    MDBOOK_SHA256="8a6b2421aa522de06d746871d2b8fff9c8d71773467e310a4516d2566e7f2de4" ;;
  *)
    echo "${0##*/}: mdbook publishes no binary for $(uname -s) on $(uname -m)" >&2
    echo "  \`cargo install mdbook --locked --version $MDBOOK_VERSION\` builds one. Copy it to the" >&2
    echo "  path \`${0##*/} --path\` prints." >&2
    exit 1 ;;
esac

MDBOOK_URL="https://github.com/rust-lang/mdBook/releases/download/v$MDBOOK_VERSION/$MDBOOK_ASSET"

# -- arguments -------------------------------------------------------------------------------------

FORCE=0
PATH_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --force)   FORCE=1 ;;
    --path)    PATH_ONLY=1 ;;
    -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
    *)         echo "${0##*/}: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;;
  *)                    EXE="" ;;
esac

DEST="$CACHE/mdbook-$MDBOOK_VERSION"
BIN="$DEST/mdbook$EXE"

# `--path` is for a caller that wants the location and not the work, so the one thing on stdout is
# the path. `tools/setup/fetch-abcmidi.sh --path` makes the same contract.
if [ "$PATH_ONLY" -eq 1 ]; then
  printf '%s\n' "$BIN"
  exit 0
fi

if [ -x "$BIN" ] && [ "$FORCE" -eq 0 ]; then
  echo "cached    $("$BIN" --version 2>/dev/null | head -1)"
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
ARCHIVE="$CACHE/$MDBOOK_ASSET"

if [ -f "$ARCHIVE" ] && [ "$(sha256 "$ARCHIVE")" = "$MDBOOK_SHA256" ]; then
  echo "cached    $MDBOOK_ASSET"
else
  echo "fetching  $MDBOOK_ASSET"
  curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$ARCHIVE.part" "$MDBOOK_URL"
  got="$(sha256 "$ARCHIVE.part")"
  if [ "$got" != "$MDBOOK_SHA256" ]; then
    rm -f "$ARCHIVE.part"
    echo "${0##*/}: $MDBOOK_ASSET failed verification" >&2
    echo "  expected $MDBOOK_SHA256" >&2
    echo "  got      $got" >&2
    exit 1
  fi
  mv "$ARCHIVE.part" "$ARCHIVE"
fi

# The archive holds the one executable at its root.
case "$MDBOOK_ASSET" in
  *.zip) unzip -o -q "$ARCHIVE" -d "$DEST" ;;
  *)     tar xzf "$ARCHIVE" -C "$DEST" ;;
esac
chmod 755 "$BIN"

echo "fetched   $("$BIN" --version 2>/dev/null | head -1)"
echo "          $BIN"
