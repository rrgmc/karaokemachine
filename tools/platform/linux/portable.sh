#!/usr/bin/env bash
#
# Builds the portable copy for Linux: the machine and every tool in one folder, in a container.
#
#   tools/platform/linux/portable.sh             # stage the machine's tarball folder, then the copy
#   tools/platform/linux/portable.sh --no-build  # use the tarball folder that is already staged
#   tools/platform/linux/portable.sh --no-video  # ...with the optional video feature turned off
#   tools/platform/linux/portable.sh -v          # watch the builds
#
#   dist/portable/linux/karaokemachine-portable-<version>-linux-<arch>/        the folder
#   dist/portable/linux/karaokemachine-portable-<version>-linux-<arch>.tar.gz  the archive
#   dist/portable-console/linux/karaokemachine-portable-console-<version>-linux-<arch>/
#
# **The second folder holds the same programs.** Nothing has two forms on Linux, and
# tools/dist/portable.sh stages both copies on every platform it serves.
#
# **Every program is built in the image the tarball is built in.** A program links the glibc of the
# system that built it, so tools built on the host would ask for a newer one than the machine does.
# The image is the compatibility floor for the whole folder; tools/platform/linux/tarball.sh says
# which distributions that is.
#
# **The tools take the machine's ffmpeg.** tools/dist/cmd.sh links the host's, which a folder cannot
# carry. Here the three tools that read video link the LGPL build in `lib/` beside them.
#
# The machine's folder comes from tools/platform/linux/tarball.sh and is copied, not built again.
# What makes the folder portable is tools/dist/portable.sh, which the container runs last.
#
# Requirements: Docker. On Windows that means Docker Desktop running.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=portable

. tools/platform/linux/image-tag.sh          # sets IMAGE, the one tarball.sh and deb.sh build
VOLUME="karaokemachine-deb-build"
VIDEO=1
BUILD=1

for arg in "$@"; do
  case "$arg" in
    --no-video) VIDEO=0 ;;
    --no-build) BUILD=0 ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/platform/linux/portable.sh [--no-build] [--no-video] [-v]"
      exit 0
      ;;
    *) echo "portable: unknown option $arg" >&2; exit 2 ;;
  esac
done

# The same guard as tarball.sh: MSYS would rewrite `/src` into a Windows path.
export MSYS2_ARG_CONV_EXCL='*'

if ! docker version >/dev/null 2>&1; then
  echo "portable: cannot reach the Docker daemon." >&2
  echo "          On Windows, start Docker Desktop and wait for it to say it is running." >&2
  exit 1
fi

# tarball.sh builds the image when it is missing, and stages the folder this copies.
if [ "$BUILD" -eq 1 ]; then
  FLAGS=()
  if [ "$VIDEO" -ne 1 ]; then FLAGS+=(--no-video); fi
  if dist_verbose; then FLAGS+=(--verbose); fi
  tools/platform/linux/tarball.sh "${FLAGS[@]+"${FLAGS[@]}"}"
  echo
fi

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "portable: the build image $IMAGE is not here." >&2
  echo "          build it with: tools/platform/linux/tarball.sh" >&2
  exit 1
fi

dist_step "portable copy"
# `/src` is read-write because the folders and the archive go to `dist/portable{,-console}/linux`
# inside it.
# The build writes to `/build`, as the tarball's does. KM_CHECKOUT: see the note in deb.sh.
docker run --rm -i \
  -e "KM_VIDEO=$VIDEO" \
  -e "DIST_VERBOSE=$DIST_VERBOSE" \
  -e "KM_CHECKOUT=$(host_path "$PWD")" \
  -v "$(host_path "$PWD")":/src \
  -v "$VOLUME":/build \
  "$IMAGE" tools/platform/linux/portable-in-container.sh
