# shellcheck shell=bash
#
# Finding the Android NDK, and the three paths inside it every caller needs.
#
#   . tools/port/ndk.sh          # from the repository root, which both callers cd to first
#   ndk_require                     # then this, which exports the variables below
#
# Sourced, never executed -- no shebang and no `set -euo pipefail`, for the reason
# tools/dist/common.sh, tools/setup/features.sh and tools/setup/ffmpeg-pin.sh all give: a sourced file that sets
# shell options changes the caller's shell in ways the caller did not ask for.
#
# **This exists because there are now two callers.** `tools/port/machine/android/build.sh` found the NDK
# itself from the day it was written, which was right while it was the only script that needed one;
# `tools/port/machine/android/ffmpeg.sh` needs the same NDK, the same version-sorted "newest installed" rule
# and the same "is this really an NDK?" check, and a second copy of that would be a second thing to
# fix when the layout changes. The discovery below is `build.sh`'s, moved rather than rewritten.
#
# What it sets, on success:
#
#   ANDROID_NDK_HOME   the NDK root (also exported as ANDROID_NDK_ROOT; some tooling reads only one)
#   NDK_HOST           the prebuilt host tag -- `windows-x86_64`, `linux-x86_64`, `darwin-x86_64`
#   NDK_BIN            the LLVM toolchain's bin directory: clang, llvm-ar, llvm-strip, llvm-readelf
#   NDK_SYSROOT        the unified sysroot every Android target compiles against
#   NDK_TOOLCHAIN      build/cmake/android.toolchain.cmake, which the SDL `-sys` crates want
#   NDK_MAKE           a GNU make -- see below, this is the interesting one

# The API level floor, shared so the two callers cannot disagree about what they are building for.
#
# **26 is a hard floor, not a preference**: `libaaudio.so` does not exist below it and cpal's Android
# backend links against it unconditionally, so a lower level fails at the link with
# `unable to find library -laaudio`. ffmpeg does not care, but it must be built for the same level as
# the code that loads it, so it reads the same variable.
ANDROID_PLATFORM="${ANDROID_PLATFORM:-26}"

# Locates the NDK and everything inside it, or explains what to install and returns 1.
#
# A function rather than top-level code so that sourcing this file is free of side effects and a
# caller decides when -- and whether -- it needs an NDK at all.
ndk_require() {
  if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-${LOCALAPPDATA:-$HOME}/Android/Sdk}}"
    if [ -d "$sdk/ndk" ]; then
      # Newest installed version, so a side-by-side upgrade needs no edit here.
      ANDROID_NDK_HOME="$sdk/ndk/$(ls -1 "$sdk/ndk" | sort -V | tail -1)"
    fi
  fi
  if [ -z "${ANDROID_NDK_HOME:-}" ] || [ ! -d "$ANDROID_NDK_HOME" ]; then
    echo "error: no Android NDK found. Install it from Android Studio's SDK Manager," >&2
    echo "       or set ANDROID_NDK_HOME." >&2
    return 1
  fi

  # **Converted to a POSIX path before anything is built from it.** On Windows the fallback above
  # comes from $LOCALAPPDATA, which is `C:\Users\...` -- and a backslash path concatenated into a
  # `--sysroot=` argument produces an error from ffmpeg's configure that names neither the NDK nor
  # the backslash. Harmless everywhere else: without cygpath the value is already POSIX.
  if command -v cygpath >/dev/null 2>&1; then
    ANDROID_NDK_HOME="$(cygpath -u "$ANDROID_NDK_HOME")"
  fi

  export ANDROID_NDK_HOME
  # The CMake toolchain file reads this one; some tooling only sets the other.
  export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"

  NDK_TOOLCHAIN="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake"
  if [ ! -f "$NDK_TOOLCHAIN" ]; then
    echo "error: $NDK_TOOLCHAIN is missing; is ANDROID_NDK_HOME really an NDK?" >&2
    return 1
  fi

  # One entry, whatever this host is. Listed rather than derived from `uname`, because the NDK's own
  # spelling (`windows-x86_64`) is not what `uname -s` says on any of the three platforms, and the
  # directory is the authority on which prebuilt is actually installed.
  NDK_HOST="$(ls -1 "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" 2>/dev/null | head -1)"
  if [ -z "$NDK_HOST" ]; then
    echo "error: no LLVM prebuilt under $ANDROID_NDK_HOME/toolchains/llvm/prebuilt" >&2
    return 1
  fi

  NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$NDK_HOST/bin"
  NDK_SYSROOT="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$NDK_HOST/sysroot"

  # **The NDK ships a GNU make, and on Windows it is the only one.** ffmpeg is an autotools-shaped
  # build: it needs `make`, and Git Bash -- which is where this project is largely developed -- has
  # `sh`, `awk`, `sed` and `perl` but no `make` at all. The NDK's is GNU Make 4.3, which is ample,
  # so an ffmpeg cross-build needs no MSYS2, no WSL and no new prerequisite on this platform. Prefer
  # it everywhere rather than only on Windows, so that all three platforms run the same program.
  if [ -x "$ANDROID_NDK_HOME/prebuilt/$NDK_HOST/bin/make" ]; then
    NDK_MAKE="$ANDROID_NDK_HOME/prebuilt/$NDK_HOST/bin/make"
  elif [ -x "$ANDROID_NDK_HOME/prebuilt/$NDK_HOST/bin/make.exe" ]; then
    NDK_MAKE="$ANDROID_NDK_HOME/prebuilt/$NDK_HOST/bin/make.exe"
  elif command -v make >/dev/null 2>&1; then
    NDK_MAKE="make"
  else
    echo "error: no make found -- not in the NDK at" >&2
    echo "       $ANDROID_NDK_HOME/prebuilt/$NDK_HOST/bin, and not on PATH." >&2
    return 1
  fi

  export NDK_HOST NDK_BIN NDK_SYSROOT NDK_TOOLCHAIN NDK_MAKE
}

# The Rust target triple for an ABI, which is also the stem of the clang driver that targets it.
#
# Both callers need this mapping and neither should spell it twice: `build.sh` turns an ABI into the
# directory cargo built into, and `ffmpeg.sh` turns it into `--cc=<triple><api>-clang`.
ndk_triple() {
  case "$1" in
    arm64-v8a) echo "aarch64-linux-android" ;;
    armeabi-v7a) echo "armv7-linux-androideabi" ;;
    x86_64) echo "x86_64-linux-android" ;;
    x86) echo "i686-linux-android" ;;
    *) return 1 ;;
  esac
}

# The clang driver's target for an ABI, which is *not* always the Rust triple.
#
# 32-bit ARM is the exception and the reason this is a second function: rustc says
# `armv7-linux-androideabi` where clang says `armv7a-linux-androideabi` -- `armv7a`, with the `a`.
# Getting this wrong produces a clang that reports "unknown target", which reads like a broken NDK.
ndk_clang_target() {
  case "$1" in
    arm64-v8a) echo "aarch64-linux-android" ;;
    armeabi-v7a) echo "armv7a-linux-androideabi" ;;
    x86_64) echo "x86_64-linux-android" ;;
    x86) echo "i686-linux-android" ;;
    *) return 1 ;;
  esac
}
