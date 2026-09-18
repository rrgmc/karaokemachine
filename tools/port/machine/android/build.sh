#!/usr/bin/env bash
#
# Cross-compiles the machine for Android.
#
#   tools/port/machine/android/build.sh                     # arm64 + armv7, debug
#   tools/port/machine/android/build.sh --release           # arm64 + armv7, release
#   tools/port/machine/android/build.sh --arm64-only        # skip armv7, for a quick phone-only iteration
#
# Both ABIs are the default because an APK is only useful with both: every Google TV device runs a
# 32-bit OS and loads `armeabi-v7a` alone, so an arm64-only APK installs on a phone and fails on a
# television.
#
# This exists because the working combination is not obvious and took several failed attempts to
# find. Each line below is one of those attempts.
#
# Prerequisites (see "Android prerequisites" in docs/ARCHITECTURE.md):
#   - Android NDK, via Android Studio's SDK Manager
#   - cargo install cargo-ndk
#   - rustup target add aarch64-linux-android armv7-linux-androideabi
#   - ninja on PATH

set -euo pipefail

# This script always assumed it was being run from the repository root -- the `target/` path in its
# closing report said so -- without ever arranging it. Both are settled here, the same way
# tools/port/machine/android/stage.sh and tools/port/machine/android/assets.sh already do it, so the report below can ask
# `dist_target_dir` where cargo actually built rather than guessing.
cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
. tools/port/ndk.sh
. tools/setup/features.sh
DIST_SCRIPT=build

# API 26 is a hard floor, not a preference: `libaaudio.so` does not exist below it, and cpal's
# Android backend links against it unconditionally. cargo-ndk defaults to 21, where the link fails
# with `unable to find library -laaudio`. Named once, in tools/port/ndk.sh, because the ffmpeg
# built for this platform has to target the same level as the code that loads it.
PLATFORM="$ANDROID_PLATFORM"

ABIS=("arm64-v8a" "armeabi-v7a")
CARGO_ARGS=()
# **Video is on by default here, and `--no-video` is how the smaller build is asked for.** That is
# the same shape every other carrier already has -- see the `Video in a release build` decision in
# docs/decisions/song-sources.md, whose reasoning is that the cargo default answers "what must somebody install to
# compile this at all" while a *release* default answers "what does a person receiving this get to
# play". Android follows that rule like every other carrier, having a video path of its own to
# decline.
WANT_VIDEO=1
for arg in "$@"; do
  case "$arg" in
    --arm64-only) ABIS=("arm64-v8a") ;;
    --no-video) WANT_VIDEO=0 ;;
    *) CARGO_ARGS+=("$arg") ;;
  esac
done

# -- find the NDK ---------------------------------------------------------------------------------

# The discovery lives in tools/port/ndk.sh rather than inline, because
# tools/port/machine/android/ffmpeg.sh needs exactly the same NDK, the same "newest installed" rule
# and the same "is this really an NDK?" check.
ndk_require || exit 1
toolchain="$NDK_TOOLCHAIN"

# -- CMake, for SDL3 and SDL3_ttf -----------------------------------------------------------------

# Two settings, and both are needed.
#
# `Ninja`, because CMake on Windows defaults to the Visual Studio generator: without this, building
# SDL3 for Android tries to compile a `.vcxproj` with MSBuild for `Platform=x64` and fails somewhere
# deep inside `Microsoft.Common.CurrentVersion.targets`.
#
# The NDK's own toolchain file, because the `cmake` crate otherwise guesses `CMAKE_SYSTEM_PROCESSOR`
# and gets `arm64`, which CMake's built-in Android support rejects — it wants `aarch64`. Handing it
# the NDK's toolchain file skips the guessing entirely, and cargo-ndk already passes the `ANDROID_ABI`
# that file needs.
export CMAKE_GENERATOR="${CMAKE_GENERATOR:-Ninja}"
export CMAKE_TOOLCHAIN_FILE="${CMAKE_TOOLCHAIN_FILE:-$toolchain}"

if ! command -v ninja >/dev/null 2>&1; then
  echo "error: ninja is not on PATH, and the Ninja generator needs it." >&2
  echo "       Install it, or add Android SDK's cmake package which bundles one." >&2
  exit 1
fi

for tool in cargo-ndk; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "error: $tool is not installed. cargo install cargo-ndk" >&2
    exit 1
  }
done

# -- video, if it was asked for -------------------------------------------------------------------

# **A missing ffmpeg stops the build before it compiles anything**, naming the remedy, rather than
# quietly producing the APK that lists video songs and refuses to read them. That is the rule the
# desktop staging scripts already follow, and the failure it prevents is the one the
# `Video in a release build` decision was written about.
FEATURE_ARGS=()
if [ "$WANT_VIDEO" -eq 1 ]; then
  for abi in "${ABIS[@]}"; do
    if [ ! -f "$(tools/port/machine/android/ffmpeg.sh --print-dir "$abi")/.km-complete" ]; then
      echo "error: no ffmpeg built for $abi, and this build wants video." >&2
      echo "       Build it once with:  tools/port/machine/android/ffmpeg.sh" >&2
      echo "       ...or ask for the smaller build with:  $0 --no-video" >&2
      exit 1
    fi
  done
  # ffmpeg-sys-next runs bindgen, which needs libclang. The NDK does not ship one, so this is the
  # host LLVM's -- checked here rather than left to fail inside a build script, where the error
  # names neither libclang nor Android.
  if [ -z "${LIBCLANG_PATH:-}" ] && ! grep -q 'LIBCLANG_PATH' "${CARGO_HOME:-$HOME/.cargo}/config.toml" 2>/dev/null; then
    echo "error: LIBCLANG_PATH is not set and cargo has no [env] entry for it." >&2
    echo "       ffmpeg-sys-next runs bindgen and ships no pre-generated bindings." >&2
    echo "       Run:  tools/setup/fetch-ffmpeg.sh" >&2
    exit 1
  fi
  FEATURE_ARGS=(--features "$KM_FEATURES_ANDROID")
fi

# -- build ---------------------------------------------------------------------------------------

echo "NDK       $ANDROID_NDK_HOME"
echo "platform  android-$PLATFORM"
echo "abis      ${ABIS[*]}"
echo "generator $CMAKE_GENERATOR"
if [ "$WANT_VIDEO" -eq 1 ]; then
  echo "video     on ($KM_FEATURES_ANDROID)"
  echo "ffmpeg    $(dirname "$(tools/port/machine/android/ffmpeg.sh --print-dir "${ABIS[0]}")")/<abi>"
else
  echo "video     off (--no-video)"
fi
echo

# `-P` is the platform. Note the capital: lowercase `-p` is passed through to cargo as `--package`,
# which fails with "unknown package: 26".
#
# **`--lib` is not a tidy-up.** The APK ships `libkm_app.so` and nothing else -- tools/port/machine/android/stage.sh
# copies exactly that -- so the package's binaries were always dead weight here, Android having no way
# to execute one. It stopped being free when the machine gained a console twin: without this the NDK
# linked *two* full executables per ABI, four in all, none of which any APK has ever contained. See
# the `The machine's console window` decision in docs/decisions/ for why there are two.
#
# **One `cargo ndk` per ABI, rather than one invocation with two `-t` flags, and that is forced by
# ffmpeg.** `FFMPEG_DIR` is a single plain variable that `ffmpeg-sys-next` reads once per build
# script run; there is no per-target spelling of it. Two ABIs mean two different ffmpeg prefixes, so
# they cannot be described to one process. Splitting the invocation is safe because that crate emits
# `rerun-if-env-changed=FFMPEG_DIR` and because cargo keeps build-script output per target, so the
# two ABIs' bindings never share a cell.
#
# **armv7 is built first when both are asked for.** It is the ABI a television loads and the one
# where 32-bit surprises live -- the `LIBC_N` version-node fight was one -- so a failure there should
# arrive before several minutes of arm64 work rather than after.
order=("${ABIS[@]}")
if [ ${#order[@]} -gt 1 ]; then
  order=("armeabi-v7a" "arm64-v8a")
fi

for abi in "${order[@]}"; do
  if [ "$WANT_VIDEO" -eq 1 ]; then
    # A native Windows program reads this, so it needs a Windows-shaped path: a POSIX one reaches
    # ffmpeg-sys-next as a directory it cannot open, and the error names a missing header rather
    # than a missing directory.
    ff="$(tools/port/machine/android/ffmpeg.sh --print-dir "$abi")"
    command -v cygpath >/dev/null 2>&1 && ff="$(cygpath -m "$ff")"
    export FFMPEG_DIR="$ff"
  fi
  cargo ndk -t "$abi" -P "$PLATFORM" build -p karaokemachine --lib \
    "${FEATURE_ARGS[@]}" "${CARGO_ARGS[@]}"
done

echo
echo "built:"
target_dir="$(dist_target_dir)"
for abi in "${ABIS[@]}"; do
  case "$abi" in
    arm64-v8a) triple="aarch64-linux-android" ;;
    armeabi-v7a) triple="armv7-linux-androideabi" ;;
  esac
  profile="debug"
  for arg in "${CARGO_ARGS[@]:-}"; do [ "$arg" = "--release" ] && profile="release"; done
  # The shared library, not the binary. Naming `karaokemachine` here is the wrong artifact even
  # where it exists: it reports success by finding a file no APK has ever carried, so a build that
  # produced no `.so` at all still prints a reassuring line, and `--lib` above would make it
  # silently report nothing. Naming what is actually shipped answers both.
  path="$target_dir/$triple/$profile/libkm_app.so"
  [ -f "$path" ] && echo "  $abi  $path"
done
