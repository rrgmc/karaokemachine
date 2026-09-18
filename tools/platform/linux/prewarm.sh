#!/usr/bin/env bash
#
# Builds every Docker image the Linux tooling uses, so a cold machine pays for them once and
# deliberately rather than discovering the cost in the middle of something else.
#
#   tools/platform/linux/prewarm.sh            # the build image, the two Debian verifiers, the volumes
#   tools/platform/linux/prewarm.sh --all      # ...and the Fedora and Arch verifier images too
#   tools/platform/linux/prewarm.sh --check    # report what is cold and build nothing; exit 1 if any is
#   tools/platform/linux/prewarm.sh --refresh  # rebuild everything against the archive, --pull --no-cache
#
# **This is not a required step and must never become one.** Every consumer keeps its own on-demand
# guard -- deb.sh and tarball.sh check `docker image inspect`, the verifiers call
# `verify_image_ensure` -- and this script calls exactly the same helpers. One build path, two entry
# points. A prewarm that became mandatory would just be a new way for the build to fail.
#
# **Images only; the cargo cache is deliberately left cold.** The genuinely expensive first run is
# SDL3, SDL3_ttf and the bundled SQLite compiling into /build/target, which is minutes. That is not
# "Docker use", it warms itself on the first real build, and folding it in here would make this a
# twenty-minute command -- and a twenty-minute command is one nobody runs.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/platform/linux/image-tag.sh          # sets IMAGE + RUST_VERSION, hashed from Dockerfile, apt-deps.sh, rust-toolchain.toml
. tools/platform/linux/runtime-deps.sh
. tools/platform/linux/verify-image.sh

ALL=0
CHECK=0
REFRESH=""
for arg in "$@"; do
  case "$arg" in
    --all) ALL=1 ;;
    --check) CHECK=1 ;;
    --refresh) REFRESH="--refresh" ;;
    *) echo "prewarm: unknown option $arg" >&2; exit 2 ;;
  esac
done

export MSYS2_ARG_CONV_EXCL='*'

if ! docker version >/dev/null 2>&1; then
  echo "prewarm: cannot reach the Docker daemon." >&2
  echo "         On Windows, start Docker Desktop and wait for it to say it is running." >&2
  exit 1
fi

BASE_DEB="debian:13-slim"
VOLUME="karaokemachine-deb-build"
APT_CACHE="karaokemachine-apt-cache"

START=$(date +%s)
COLD=0

# One table row: label, tag, what happened, age, size. The action column is the useful part -- it is
# how you tell "already warm" from "just spent four minutes".
row() { # <label> <tag> <action> [<elapsed seconds>]
  local label="$1" tag="$2" action="$3" secs="${4:-}"
  local size age
  size="$(docker image inspect -f '{{.Size}}' "$tag" 2>/dev/null || echo 0)"
  size="$(awk -v b="$size" 'BEGIN{ if (b>=1073741824) printf "%.1f GB", b/1073741824; else printf "%.0f MB", b/1048576 }')"
  age="$(docker image inspect -f '{{.Created}}' "$tag" 2>/dev/null || true)"
  if [ -n "$age" ]; then
    local s=$(( $(date +%s) - $(date -d "$age" +%s 2>/dev/null || date +%s) ))
    if [ "$s" -lt 172800 ]; then age="$(( s / 3600 ))h"; else age="$(( s / 86400 ))d"; fi
  else
    age="-"
  fi
  [ -n "$secs" ] && action="$action ${secs}s"
  printf '  %-16s %-46s %-14s %5s %9s\n' "$label" "$tag" "$action" "$age" "$size"
}

# Builds one image unless --check, and reports which. Sets COLD when something was (or would be)
# missing, which is what --check's exit status is derived from.
warm() { # <label> <tag> <build command...>
  local label="$1" tag="$2"; shift 2
  if [ -z "$REFRESH" ] && docker image inspect "$tag" >/dev/null 2>&1; then
    row "$label" "$tag" "present"
    return 0
  fi
  COLD=$(( COLD + 1 ))
  if [ "$CHECK" = "1" ]; then
    row "$label" "$tag" "MISSING"
    return 0
  fi
  local t0 t1 log
  t0=$(date +%s)
  # The build's own output is captured rather than discarded, and **printed when it fails**. It was
  # sent to /dev/null at first, which meant a failure said only "failed to build <tag>" and the
  # reason -- the one thing you need -- was gone. That is the same mistake as a verification that
  # exits 0 having run nothing: it looks like a clean report and is an absence of information.
  log="$(mktemp)"
  if ! "$@" >"$log" 2>&1; then
    echo "prewarm: failed to build $tag" >&2
    sed 's/^/    /' "$log" | tail -25 >&2
    rm -f "$log"
    return 1
  fi
  rm -f "$log"
  t1=$(date +%s)
  row "$label" "$tag" "built" "$(( t1 - t0 ))"
}

build_main_image() { docker build ${REFRESH:+--pull --no-cache} "${IMAGE_BUILD_ARGS[@]}" -t "$IMAGE" tools/platform/linux; }

echo "== prewarm$( [ "$CHECK" = "1" ] && printf ' (check only)' )"

warm "build image" "$IMAGE" build_main_image

# The verifiers' base has to exist before anything can be derived from it. Pulled rather than built,
# and only when absent.
if ! docker image inspect "$BASE_DEB" >/dev/null 2>&1; then
  if [ "$CHECK" = "1" ]; then
    COLD=$(( COLD + 1 )); row "base" "$BASE_DEB" "MISSING"
  else
    t0=$(date +%s); docker pull -q "$BASE_DEB" >/dev/null; t1=$(date +%s)
    row "base" "$BASE_DEB" "pulled" "$(( t1 - t0 ))"
  fi
else
  row "base" "$BASE_DEB" "present"
fi

# `verify_image_ensure` is the same function the verifiers call, so an image warmed here is exactly
# the image they will look for -- a prewarm that computed its own tags would warm images nothing
# else uses, which is worse than no prewarm at all.
warm_verify() { # <label> <kind> <base>
  local label="$1" kind="$2" base="$3" tag
  tag="$(verify_image_tag "$kind" "$base")"
  warm "$label" "$tag" verify_image_ensure "$kind" "$base" "$REFRESH"
}

warm_verify "deb verifier"  index  "$BASE_DEB"
warm_verify "tarball deps"  debian "$BASE_DEB"

if [ "$ALL" = "1" ]; then
  for spec in "fedora deps:fedora:fedora:42" "arch deps:arch:archlinux:latest"; do
    label="${spec%%:*}"; rest="${spec#*:}"; kind="${rest%%:*}"; base="${rest#*:}"
    if ! docker image inspect "$base" >/dev/null 2>&1 && [ "$CHECK" != "1" ]; then
      docker pull -q "$base" >/dev/null || { echo "prewarm: could not pull $base" >&2; continue; }
    fi
    warm_verify "$label" "$kind" "$base"
  done
else
  printf '  %-16s %s\n' "fedora deps" "-- not built (pass --all)"
  printf '  %-16s %s\n' "arch deps"   "-- not built (pass --all)"
fi

# The volumes. Creating one is idempotent and costs nothing; it is listed so the table is the whole
# picture of what a run will and will not have to make.
for v in "$VOLUME" "$APT_CACHE"; do
  if docker volume inspect "$v" >/dev/null 2>&1; then
    printf '  %-16s %-46s %s\n' "volume" "$v" "present"
  elif [ "$CHECK" = "1" ]; then
    COLD=$(( COLD + 1 )); printf '  %-16s %-46s %s\n' "volume" "$v" "MISSING"
  else
    docker volume create "$v" >/dev/null
    printf '  %-16s %-46s %s\n' "volume" "$v" "created"
  fi
done

echo
if [ "$CHECK" = "1" ]; then
  if [ "$COLD" -gt 0 ]; then
    echo "$COLD cold. Run tools/platform/linux/prewarm.sh to build them."
    exit 1
  fi
  echo "everything is warm."
  exit 0
fi

# Says what is now free, rather than leaving the table to be interpreted. The cargo cache is named
# explicitly as *not* covered, because "prewarmed" would otherwise imply the first build is quick and
# it is not.
echo "$(( $(date +%s) - START ))s. deb.sh, tarball.sh and check.sh will not build an image;"
echo "verify-tarball.sh and verify-deb.sh will not install anything."
echo "The cargo cache is still cold -- the first build compiles SDL3 and SQLite, which is minutes."
