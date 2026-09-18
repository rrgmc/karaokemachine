#!/usr/bin/env bash
#
# Builds a portable Linux folder of the machine, and a .tar.gz of it, in a container.
#
#   tools/platform/linux/tarball.sh                 # build the image if needed, then the tarball
#   tools/platform/linux/tarball.sh --no-video      # ...with the optional video feature turned off
#   tools/platform/linux/tarball.sh --rebuild       # rebuild the image from scratch first
#   tools/platform/linux/tarball.sh --shell         # a shell in the build image, for poking at it
#   tools/platform/linux/tarball.sh -v              # watch the image build, the ffmpeg build and cargo
#
# **Quiet by default.** The phases, the self-contained checks and the folder/tarball report are
# printed; the build streams are not. A step that fails replays everything it held back.
#
# Output: dist/karaokemachine/linux/karaokemachine-<version>-<triple>/          (the folder)
#         dist/karaokemachine/linux/karaokemachine-<version>-<triple>.tar.gz    (the same, packed)
#         ...and both with -no-video in the name, for that build.
#
# **What this is for, given the .deb already exists.** The .deb is the supported Linux deployment and
# nothing here changes that -- `What the machine is, on Linux` in docs/decisions/distribution.md says the product is a
# Debian appliance, and the package is what makes one: a systemd unit, a system user, an uninstall
# the system remembers. This is the other thing people want from a Linux release and a .deb cannot
# be: a folder. Unpack it in your home directory, run it, delete it. No root, no package manager, no
# opinion about your distribution. It is the Windows folder, on Linux, and deliberately the same
# layout so that anybody who has seen one recognizes the other.
#
# **It is not an appliance and does not try to be.** No unit file, no `karaoke` user, no DRM master.
# install.sh inside it does what one user may do without root -- a menu entry and the `.kmpkg` type
# -- and that is the whole of its ambition; the header of that file says so and points at the
# package.
#
# **The ffmpeg it bundles is built here, not borrowed from Debian**, and that is the one genuinely
# surprising thing in this script. Debian's libavcodec61 is GPL-2+ by their own copyright file, this
# workspace is MIT OR Apache-2.0, and Apache-2.0 is not GPL-2-compatible -- so the four libraries a
# .deb may simply *name* are libraries a tarball may not carry. tools/platform/linux/ffmpeg-lgpl.sh builds an
# LGPL one instead, out of the release Debian itself packages, with every external library switched
# off. It is a fifth the size and it costs the profile nothing. The full argument is in that file.
#
# Why a container, same as the .deb: the binary has to link against the glibc of the oldest
# distribution it is meant for, and this is developed on Windows. tools/platform/linux/Dockerfile names that
# distribution in its FROM line and that line is the compatibility floor -- today Debian 13, which
# leaves the tarball needing glibc 2.38, so Debian 13, Ubuntu 24.04 and Fedora 39 and up.
#
# Requirements: Docker. On Windows that means Docker Desktop running.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=tarball

# The same image and the same cache volume as tools/platform/linux/deb.sh, deliberately. Both build the same
# workspace with the same toolchain against the same glibc, so a second image would be a second thing
# to keep in step for no gain -- and sharing the volume means a tarball built after a package reuses
# its compilation rather than starting from cold. The tarball needs four more packages in the image
# (patchelf, nasm, xz-utils and a font); they are appended in a layer of their own so that adding
# them did not invalidate anything the package build had already cached.
. tools/platform/linux/image-tag.sh          # sets IMAGE + RUST_VERSION, hashed from Dockerfile, apt-deps.sh, rust-toolchain.toml
VOLUME="karaokemachine-deb-build"
BUILD_ARGS=()
REBUILD=0
CMD=(tools/platform/linux/tarball-in-container.sh)
VIDEO=1

for arg in "$@"; do
  case "$arg" in
    --rebuild) REBUILD=1; BUILD_ARGS+=(--no-cache --pull) ;;
    --shell) CMD=(bash) ;;
    --no-video) VIDEO=0 ;;
    # The build logs, quiet by default -- `docker build`'s layer stream and, inside the container,
    # cargo's and ffmpeg's. Travels in as an environment variable below, as KM_VIDEO does.
    -v|--verbose) DIST_VERBOSE=1 ;;
    *) echo "tarball: unknown option $arg" >&2; exit 2 ;;
  esac
done

# MSYS (Git Bash) rewrites anything that looks like a Unix path before handing it to a Windows
# executable, which turns `/src` into `C:/Program Files/Git/src`. Turned off wholesale; host paths
# are converted deliberately with `host_path` below.
export MSYS2_ARG_CONV_EXCL='*'

if ! docker version >/dev/null 2>&1; then
  echo "tarball: cannot reach the Docker daemon." >&2
  echo "         On Windows, start Docker Desktop and wait for it to say it is running." >&2
  exit 1
fi

# Built only when it is missing -- the same guard, and the same reasoning, as deb.sh. `--rebuild`
# still builds unconditionally.
if [ "$REBUILD" = "1" ] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  dist_step "image (building $IMAGE)"
  image_started=$SECONDS
  dist_run "docker build" docker build "${IMAGE_BUILD_ARGS[@]}" "${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"}" -t "$IMAGE" tools/platform/linux
  printf '   built in %s\n' "$(dist_elapsed "$image_started")"
else
  dist_step "image $IMAGE (cached)"
fi
echo

# App, then platform -- the layout rule is in tools/dist/common.sh. Unlike the .deb the declined
# build needs no subfolder of its own: cargo-deb names its file and a feature changes neither the
# name nor the version, whereas the folder name here is ours and carries the marker.
OUT="$(dist_dir karaokemachine linux)"
mkdir -p "$OUT"

dist_step folder
# Read-write on /src because cargo writes Cargo.lock timestamps and the asset check may fetch the
# SoundFont; the build itself writes to /build via CARGO_TARGET_DIR, so this cannot disturb the
# Windows target/ directory in the same checkout. The ffmpeg prefix lives in /build too, which is why
# it is built once per volume rather than once per run.
# KM_CHECKOUT: see the note in deb.sh. clean-checkout.sh reads it.
# Not wrapped in `dist_run`: what the container prints here is the report -- the self-contained
# checks and the folder/tarball totals -- and only cargo's and ffmpeg's streams inside it are a log.
# So the quieting happens in there, where the two can be told apart. See the same note in deb.sh.
docker run --rm -i \
  -e "KM_VIDEO=$VIDEO" \
  -e "DIST_VERBOSE=$DIST_VERBOSE" \
  -e "KM_CHECKOUT=$(host_path "$PWD")" \
  -v "$(host_path "$PWD")":/src \
  -v "$VOLUME":/build \
  -v "$(host_path "$PWD/$OUT")":/out \
  "$IMAGE" "${CMD[@]}"

echo
echo "verify it in a clean container with:  tools/platform/linux/verify-tarball.sh$( [ "$VIDEO" -eq 0 ] && printf ' --no-video' )"
