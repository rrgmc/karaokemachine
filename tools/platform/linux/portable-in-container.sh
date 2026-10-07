#!/usr/bin/env bash
#
# The half of tools/platform/linux/portable.sh that runs inside the build image. Not meant to be run
# directly.
#
# Expects /src (the source tree, read-write) and /build (the cargo target dir and the ffmpeg
# prefix). KM_VIDEO selects the optional video feature, and the default is 0 for the reason
# tarball-in-container.sh gives.
#
# It gathers the machine's staged tarball folder, builds every tool beside it, and hands the
# folder to tools/dist/portable.sh. That script adds the marker, the `data` folders and the
# document, proves the claim and writes the archive.

set -euo pipefail

cd /src

. tools/dist/common.sh
DIST_SCRIPT=portable

. tools/setup/ffmpeg-pin.sh

VIDEO="${KM_VIDEO:-0}"
FFPREFIX="/build/ffmpeg-lgpl/$FF_SRC_ID"
T="${CARGO_TARGET_DIR:-/build/target}"

# The same tools, and the same two facts about them, that tools/dist/cmd.sh keeps: which take
# `video`, and which live in the second workspace. That script cannot be run here, because it
# links the host's ffmpeg and stages one folder per tool.
TOOLS=(km-pack km-lyrics km-site-pack km-package-builder km-package-simple km-remote km-admin km-wallpaper-pack)
video_capable() { case "$1" in km-pack|km-package-builder|km-package-simple) return 0 ;; *) return 1 ;; esac; }
tool_build_args() {
  case "$1" in
    km-wallpaper-pack|km-admin) printf -- '--manifest-path tools/cmd/assets/Cargo.toml -p %s' "$1" ;;
    *) printf -- '-p %s' "$1" ;;
  esac
}

# -- the machine's folder ------------------------------------------------------------------------

VERSION="$(dist_manifest_version)"
TARGET="$(dist_host_triple)"
MACHINE="dist/karaokemachine/linux/karaokemachine-$VERSION-$TARGET"
if [ "$VIDEO" -eq 0 ]; then MACHINE="$MACHINE-no-video"; fi
if [ ! -x "$MACHINE/karaokemachine" ]; then
  echo "portable: $MACHINE is not staged." >&2
  echo "          build it with: tools/platform/linux/tarball.sh$( [ "$VIDEO" -eq 0 ] && printf ' --no-video' )" >&2
  exit 1
fi

# -- the tools -----------------------------------------------------------------------------------

if [ "$VIDEO" -eq 1 ]; then
  # Already built by the tarball's run, so this reports the prefix and builds nothing.
  tools/platform/linux/ffmpeg-lgpl.sh "$FFPREFIX"
  export PKG_CONFIG_PATH="$FFPREFIX/lib/pkgconfig"
  echo
fi

# The shared volume cannot tell two checkouts apart; clean-checkout.sh gives the argument.
tools/platform/linux/clean-checkout.sh

dist_step "build the tools$( [ "$VIDEO" -eq 1 ] && printf ' (video)' )"
build_started=$SECONDS
for tool in "${TOOLS[@]}"; do
  FEATURES=()
  if [ "$VIDEO" -eq 1 ] && video_capable "$tool"; then FEATURES+=(--features video); fi
  # shellcheck disable=SC2046  # tool_build_args prints separate words on purpose
  dist_run "cargo build ($tool)" \
    cargo build --release --locked $(tool_build_args "$tool") "${FEATURES[@]+"${FEATURES[@]}"}"
  if [ ! -x "$T/release/$tool" ]; then
    echo "portable: the build produced no $T/release/$tool" >&2
    exit 1
  fi
done
printf '   built in %s\n' "$(dist_elapsed "$build_started")"

# -- one folder ----------------------------------------------------------------------------------

STAGE="/build/portable-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$MACHINE"/. "$STAGE"/

for tool in "${TOOLS[@]}"; do
  cp "$T/release/$tool" "$STAGE/$tool"
  chmod 755 "$STAGE/$tool"
  strip --strip-unneeded "$STAGE/$tool"
  # The machine's `lib/` is beside every program here, so one rpath serves all of them.
  if [ "$VIDEO" -eq 1 ] && video_capable "$tool"; then
    patchelf --set-rpath '$ORIGIN/lib' "$STAGE/$tool"
  fi
done

echo
tools/dist/portable.sh --from "$STAGE"
rm -rf "$STAGE"
