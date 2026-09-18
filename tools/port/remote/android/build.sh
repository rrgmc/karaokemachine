#!/usr/bin/env bash
#
# Cross-compiles the offline remote for Android.
#
#   tools/port/remote/android/build.sh                 # arm64 + armv7, debug
#   tools/port/remote/android/build.sh --release       # arm64 + armv7, release
#   tools/port/remote/android/build.sh --arm64-only    # skip armv7
#
# **This is deliberately not `tools/port/machine/android/build.sh` with a flag.** That script is a hundred and
# ninety lines of CMake generator selection, SDL toolchain files, a ninja check, per-ABI ffmpeg
# prefixes and a libclang validation, none of which applies to a crate whose only C dependency is the
# SQLite amalgamation `rusqlite` bundles. Sharing would have meant a flag to turn each of them off.
#
# What *is* shared is sourced rather than copied: the NDK discovery, the API floor and
# `dist_target_dir`.
#
# Prerequisites -- the same ones the machine already needs, and nothing more. No ninja, no ffmpeg, no
# libclang:
#   - Android NDK, via Android Studio's SDK Manager
#   - cargo install cargo-ndk
#   - rustup target add aarch64-linux-android armv7-linux-androideabi

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
. tools/port/ndk.sh
. tools/setup/features.sh
DIST_SCRIPT=build

# API 26, from `ndk.sh`, so the remote and the machine cannot drift. The *reason* differs and is
# worth knowing: the machine's floor is `libaaudio.so`, which cpal links unconditionally, and this
# crate links no audio at all. 26 is kept for consistency rather than inherited necessity.
PLATFORM="$ANDROID_PLATFORM"

# **Both ABIs by default, and `--arm64-only` is a real option here rather than a debugging shortcut.**
# The machine builds both because every Google TV device runs a 32-bit OS and loads `armeabi-v7a`
# alone, so an arm64-only APK installs on a phone and fails on a television. **No such device exists
# for a remote** -- it is a phone application, and every phone shipped for years is arm64. Both stay
# the default because a second `cargo ndk` costs about a minute; the machine's "NOT a thing to ship"
# warning is deliberately *not* repeated, because it names a television this APK never reaches.
ABIS=("arm64-v8a" "armeabi-v7a")
CARGO_ARGS=()
# Kept so the `next:` line at the bottom can repeat it. **stage.sh needs this flag too**, because
# skipping an ABI here does not remove the library an earlier build left in the target directory --
# see the header of stage.sh. A hint that printed the command without it would be telling somebody to
# reintroduce the exact bug.
ARM64_ONLY=""
for arg in "$@"; do
  case "$arg" in
    --arm64-only) ABIS=("arm64-v8a"); ARM64_ONLY="--arm64-only" ;;
    *) CARGO_ARGS+=("$arg") ;;
  esac
done

ndk_require

dist_step "building the offline remote for ${ABIS[*]}"
dist_detail "features: ${KM_FEATURES_ANDROID_REMOTE:-<none>}"

# armv7 first when both are asked for, for the reason `tools/port/machine/android/build.sh` gives: it is
# where the 32-bit surprises live -- the `LIBC_N` version-node fight was one -- so a failure there
# should arrive before several minutes of arm64 work rather than after. It is also where the JNI
# exports meet `tools/port/libc_n.map`, which `.cargo/config.toml` applies workspace-wide.
order=("${ABIS[@]}")
if [ ${#order[@]} -gt 1 ]; then
  order=("armeabi-v7a" "arm64-v8a")
fi

for abi in "${order[@]}"; do
  # `--lib` because the APK ships exactly one `.so` and this package has no binary at all.
  cargo ndk -t "$abi" -P "$PLATFORM" build -p km-remote-android --lib "${CARGO_ARGS[@]}"
done

profile="debug"
for arg in "${CARGO_ARGS[@]:-}"; do [ "$arg" = "--release" ] && profile="release"; done

echo
echo "built:"
# Asked of cargo rather than spelled `target/`. Nothing in tools/ may assume that directory: a
# worktree, a container and another machine all put it somewhere else, and the failure from guessing
# is a script reporting a missing file on the line after cargo said `Finished`.
target_dir="$(dist_target_dir)"
for abi in "${ABIS[@]}"; do
  case "$abi" in
    arm64-v8a) triple="aarch64-linux-android" ;;
    armeabi-v7a) triple="armv7-linux-androideabi" ;;
  esac
  path="$target_dir/$triple/$profile/libkm_remote_android.so"
  if [ -f "$path" ]; then
    echo "  $abi  $path"
  else
    echo "$DIST_SCRIPT: $abi produced no library at $path" >&2
    exit 1
  fi
done

echo
echo "next: tools/port/remote/android/stage.sh${CARGO_ARGS:+ ${CARGO_ARGS[*]}}${ARM64_ONLY:+ $ARM64_ONLY}"
