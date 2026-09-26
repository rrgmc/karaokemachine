# shellcheck shell=bash
#
# The released carol pack that the published moving pictures are made from: which file, which bytes,
# and how to get it.
#
# Sourced, never executed -- no shebang and no `set -euo pipefail`, for the reason
# tools/dist/common.sh gives. The caller sources tools/dist/common.sh and tools/setup/asset-cache.sh
# first, because the fetch uses `dist_run` and `$CACHE`.
#
#   . tools/setup/carols-pin.sh
#   PACK="$(carols_pack)"               # the pack's path, downloaded and verified if not cached
#
# Its two callers:
#
#   tools/dev/screen-animation.sh       docs/images/screen-singing.webp, the README's animated picture
#   tools/dev/promo-video.sh            the promotional video
#
# **The pack is a release asset, pinned by digest.** So both regenerate on any machine with the
# network, and neither needs a corpus or `abc2midi`. See `The animated picture is of a public-domain
# carol` in docs/decisions/repository.md.
#
#   KM_CAROLS   a carol pack to read instead of the pinned download, such as the one
#               tools/dist/carols.sh writes into dist/carols/
#
# The pack changes rarely and has its own version, so the pin moves only when the pack does. Change
# the three lines together.

CAROLS_NAME="christmas-carols-1.0.0.kmpkg"
CAROLS_URL="https://github.com/rrgmc/karaokemachine/releases/download/v1.18.0/$CAROLS_NAME"
CAROLS_SHA256="cbe1145ffe43c6a134cf826cf69dfc3676d1aab298ed9c76e015cbdaef9410db"

carols_sha256() { # <file>  -> prints the digest
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    echo "${DIST_SCRIPT:-carols}: need sha256sum or shasum to verify the download" >&2
    return 1
  fi
}

carols_pack() { # -> prints the pack's path, or fails having said why
  if [ -n "${KM_CAROLS:-}" ]; then
    [ -f "$KM_CAROLS" ] || {
      echo "${DIST_SCRIPT:-carols}: KM_CAROLS is $KM_CAROLS, which is not a file" >&2
      return 1
    }
    printf '%s' "$KM_CAROLS"
    return 0
  fi

  local pack="$CACHE/$CAROLS_NAME" got
  mkdir -p "$CACHE"
  # The function runs in a command substitution, where the caller's `set -e` does not reach. So each
  # step that can fail says so itself.
  if [ ! -f "$pack" ] || [ "$(carols_sha256 "$pack")" != "$CAROLS_SHA256" ]; then
    # Downloaded to a temporary name and moved into place, so an interrupted run cannot leave a
    # half file looking cached. The progress goes to stderr, because stdout carries the path.
    dist_run "fetching $CAROLS_NAME" \
      curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$pack.part" "$CAROLS_URL" >&2 \
      || return 1
    got="$(carols_sha256 "$pack.part")" || return 1
    if [ "$got" != "$CAROLS_SHA256" ]; then
      rm -f "$pack.part"
      echo "${DIST_SCRIPT:-carols}: $CAROLS_NAME failed verification" >&2
      echo "  expected $CAROLS_SHA256" >&2
      echo "  got      $got" >&2
      return 1
    fi
    mv "$pack.part" "$pack" || return 1
  fi
  printf '%s' "$pack"
}
