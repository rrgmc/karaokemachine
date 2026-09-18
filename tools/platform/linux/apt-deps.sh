#!/usr/bin/env bash
#
# Installs the system libraries a Linux build of this workspace needs.
#
#   tools/platform/linux/apt-deps.sh           # what every build needs
#   tools/platform/linux/apt-deps.sh --video   # ...plus what the optional `video` feature needs
#
# **Why this is a script rather than a list repeated in three files.** It was in three: CI's Linux
# job, `tools/platform/linux/Dockerfile`, and the README. The comment justifying that said a wrong list would
# be wrong in three places at once and CI would say so -- which is true, and is still three places to
# edit and two chances to forget. One list, read by CI and baked into the image, cannot disagree with
# itself.
#
# The first group is what any build of this workspace links: SDL3 and SDL3_ttf are compiled from
# source (`build-from-source-static`), `cpal` binds ALSA, `rusqlite` is `bundled`. It is SDL's own
# README-linux list rather than a minimal one, deliberately -- SDL's configure aborts on the *first*
# missing dependency and names only that one, so trimming it costs a full build per package to
# rediscover. XSCRNSAVER was followed immediately by XTEST once already.
#
# The second group is `--video` only, and is the whole reason the feature is off by default.
# `libclang-dev` is not optional among them: `ffmpeg-sys-next` runs bindgen at build time and ships
# no pre-generated bindings.

set -euo pipefail

VIDEO=0
for arg in "$@"; do
  case "$arg" in
    --video) VIDEO=1 ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "apt-deps: unknown option $arg" >&2; exit 2 ;;
  esac
done

# Root in a container, an unprivileged user with passwordless sudo on a CI runner. Asking `sudo` to
# exist in the image would be a package installed for nothing.
SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
  else
    echo "apt-deps: not root and no sudo; run this as root or install sudo." >&2
    exit 1
  fi
fi

BASE=(
  cmake pkg-config
  libasound2-dev libpulse-dev libudev-dev
  libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxi-dev libxfixes-dev
  libwayland-dev libxkbcommon-dev libegl1-mesa-dev libgl1-mesa-dev
  libxss-dev libxtst-dev libxrender-dev libxinerama-dev
  libibus-1.0-dev libdbus-1-dev libdrm-dev libgbm-dev libgles2-mesa-dev
  libdecor-0-dev libpipewire-0.3-dev libsamplerate0-dev
)

# Exactly the four ffmpeg libraries `km-video` links -- avdevice, avfilter and swscale are never
# built, because `ffmpeg-next` is taken with `default-features = false` and only
# codec/format/software-resampling. Their -dev packages carry the headers and the pkg-config files
# that `ffmpeg-sys-next` finds; FFMPEG_DIR is deliberately left unset on Linux, so that pkg-config
# answers rather than a guessed path.
VIDEO_PKGS=(
  libavcodec-dev libavformat-dev libavutil-dev libswresample-dev
  libclang-dev
  # **For building ffmpeg, not for building this workspace.** `tools/platform/linux/ffmpeg-lgpl.sh`
  # configures the pinned source with `--enable-libopenh264`, which is the only external library it
  # asks for and the only H.264 *encoder* an LGPL build can have — see `tools/setup/ffmpeg-pin.sh`
  # for why a streaming machine needs one. The four `-dev` packages above are what `km-video` links
  # against; this one is what that build needs to find.
  libopenh264-dev
)

PKGS=("${BASE[@]}")
if [ "$VIDEO" -eq 1 ]; then
  PKGS+=("${VIDEO_PKGS[@]}")
fi

echo "apt-deps: installing ${#PKGS[@]} package(s)$( [ "$VIDEO" -eq 1 ] && printf ', video included' )"

$SUDO apt-get update
$SUDO apt-get install -y --no-install-recommends "${PKGS[@]}"
