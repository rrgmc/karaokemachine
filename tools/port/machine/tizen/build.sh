#!/usr/bin/env bash
#
# Cross-builds the machine for a Samsung Tizen television, as libkm_app.so, in Docker.
#
#   tools/port/machine/tizen/build.sh              # debug
#   tools/port/machine/tizen/build.sh --release    # release; other arguments go to cargo too
#   tools/port/machine/tizen/build.sh --clippy     # clippy over the Tizen code, -D warnings
#   tools/port/machine/tizen/build.sh --app --release   # the library for the app (package.sh)
#
# The development build carries the font, the wallpapers and the dev console inside the .so, so
# load.sh beside this is all it takes to run it in the probe app on the set (M2 to M4). `--app`
# leaves them out, for package.sh, whose .tpk carries them in res/assets.
#
# Builds the km-tizen image first (Dockerfile beside this; cached after the first time), then runs
# build-in-container.sh in it with this checkout mounted. Video is always on: the set has ffmpeg.
#
# The container builds into its own target directory, local/tizen/target, never the host's: the
# host's target/ holds Windows or macOS artefacts that a Linux cargo must not share. Crate downloads
# are kept in two named volumes so a warm build fetches nothing.
#
# What the image is built from is read here, so nothing is copied into it: the Rust version from
# rust-toolchain.toml, the SDL3 and SDL3_ttf source crates' versions and checksums from Cargo.lock,
# and the ffmpeg pin from tools/setup/ffmpeg-pin.sh.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

IMAGE=km-tizen
HERE=tools/port/machine/tizen

command -v docker >/dev/null 2>&1 || { echo "tizen: docker is not on PATH" >&2; exit 1; }

# MSYS (Git Bash) rewrites anything that looks like a Unix path before Docker sees it; every path
# below is a container path, so it is switched off. See tools/platform/linux/deb.sh.
export MSYS2_ARG_CONV_EXCL='*'
host_path() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

# The version and checksum Cargo.lock records for one registry crate.
lock_field() { # crate field
  awk -v name="$1" -v field="$2" '
    $0 == "name = \"" name "\"" { hit = 1; next }
    hit && $1 == field { gsub(/"/, "", $3); print $3; exit }
    /^\[\[package\]\]/ { hit = 0 }
  ' Cargo.lock
}

RUST_VERSION="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)"
build_args=(
  --build-arg "RUST_VERSION=$RUST_VERSION"
  --build-arg "SDL3_SRC_VERSION=$(lock_field sdl3-src version)"
  --build-arg "SDL3_SRC_SHA256=$(lock_field sdl3-src checksum)"
  --build-arg "SDL3_TTF_SRC_VERSION=$(lock_field sdl3-ttf-src version)"
  --build-arg "SDL3_TTF_SRC_SHA256=$(lock_field sdl3-ttf-src checksum)"
)
for a in "${build_args[@]}"; do
  case "$a" in *=) echo "tizen: could not read $a from Cargo.lock or rust-toolchain.toml" >&2; exit 1 ;; esac
done

echo "== image $IMAGE"
docker build "${build_args[@]}" --build-context "setup=$(host_path "$PWD/tools/setup")" \
  -t "$IMAGE" "$(host_path "$PWD/$HERE")"

echo "== libkm_app.so for armv7-unknown-linux-gnueabi"
mkdir -p local/tizen/target
docker run --rm -i \
  -v "$(host_path "$PWD")":/src \
  -v km-tizen-cargo-registry:/opt/cargo/registry \
  -v km-tizen-cargo-git:/opt/cargo/git \
  -e CARGO_TARGET_DIR=/src/local/tizen/target \
  -w /src \
  "$IMAGE" bash "$HERE/build-in-container.sh" "$@"
