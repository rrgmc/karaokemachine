# shellcheck shell=bash
#
# The released carol pack, fetched once into the asset cache and verified.
#
#   . tools/dev/carol-pack.sh     # after tools/dist/common.sh and a DIST_SCRIPT; sets PACK
#
# Sourced by tools/dev/screen-animation.sh and tools/dev/sync-pictures.sh. No shebang and no
# `set -euo pipefail`, for the reason tools/dist/common.sh gives: a sourced file must not change the
# caller's shell.
#
# **A picture that publishes whole lines of words shows a carol from this pack.** Anybody may
# publish its words, and it is a release asset, so a script that reads it needs the network and no
# corpus. See `The animated picture is of a public-domain carol` in docs/decisions/.
#
#   KM_CAROLS   a carol pack to read instead of the pinned download, such as the one
#               tools/dist/carols.sh writes into dist/carols/

. tools/setup/asset-cache.sh

# -- the pin ---------------------------------------------------------------------------------------
#
# One release asset, by name and by digest. The pack changes rarely and has its own version, so the
# pin moves only when the pack does. Change the three lines together.
CAROLS_NAME="christmas-carols-1.0.0.kmpkg"
CAROLS_URL="https://github.com/rrgmc/karaokemachine/releases/download/v1.18.0/$CAROLS_NAME"
CAROLS_SHA256="cbe1145ffe43c6a134cf826cf69dfc3676d1aab298ed9c76e015cbdaef9410db"

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  echo "$DIST_SCRIPT: need sha256sum or shasum to verify the download" >&2
  exit 1
fi

if [ -n "${KM_CAROLS:-}" ]; then
  PACK="$KM_CAROLS"
  [ -f "$PACK" ] || { echo "$DIST_SCRIPT: KM_CAROLS is $PACK, which is not a file" >&2; exit 1; }
else
  mkdir -p "$CACHE"
  PACK="$CACHE/$CAROLS_NAME"
  if [ ! -f "$PACK" ] || [ "$(sha256 "$PACK")" != "$CAROLS_SHA256" ]; then
    # Downloaded to a temporary name and moved into place, so an interrupted run cannot leave a
    # half file looking cached.
    dist_run "fetching $CAROLS_NAME" \
      curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$PACK.part" "$CAROLS_URL"
    got="$(sha256 "$PACK.part")"
    if [ "$got" != "$CAROLS_SHA256" ]; then
      rm -f "$PACK.part"
      echo "$DIST_SCRIPT: $CAROLS_NAME failed verification" >&2
      echo "  expected $CAROLS_SHA256" >&2
      echo "  got      $got" >&2
      exit 1
    fi
    mv "$PACK.part" "$PACK"
  fi
fi
