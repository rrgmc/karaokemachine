#!/usr/bin/env bash
#
# Copies the built native libraries where Gradle expects them.
#
#   tools/port/machine/android/stage.sh               # debug, both ABIs
#   tools/port/machine/android/stage.sh --release
#   tools/port/machine/android/stage.sh --arm64-only  # arm64 alone, and armv7 removed if present
#
# Run after tools/port/machine/android/build.sh. Three libraries per ABI: SDL3 and SDL3_ttf, which cargo built
# from source into a target directory nobody would think to look in, and km_app, which is the machine.
#
# Kept separate from build.sh because it is a copy rather than a build, and because it is the step
# that tells you what is actually going into the APK.
#
# **`--arm64-only` mirrors build.sh's flag and has to be passed here as well.** `build.sh
# --arm64-only` skips armv7; it does not delete the armv7 library an earlier both-ABI build left in
# the target directory, so a staging step that copies whatever it finds packages that stale library.
# **On this APK that is not merely wasted space.** Both Google TV devices run a 32-bit OS and load
# `armeabi-v7a` alone, so the stale library is the one the television executes -- a build that looks
# current on a phone and is months old on the television, which is the hardest kind of wrong to see.
# It also silences the warning at the bottom of this file, which asks whether armv7 was staged and
# cannot tell a fresh library from a leftover.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

# Sourced for `dist_target_dir` alone -- nothing here stages a release. "a target directory nobody
# would think to look in" above is truer than it reads: the directory is not necessarily `target/`
# either, and when it is not, every check below turns into "nothing to stage", which is the same
# thing this script says when the build simply has not been run.
. tools/dist/common.sh
# For `NDK_BIN`, so the ffmpeg step below can read a cross-built ELF's DT_NEEDED with the NDK's own
# llvm-readelf rather than a host tool that may not understand an ARM object.
. tools/port/ndk.sh
ndk_require || exit 1
DIST_SCRIPT=stage

PROFILE="debug"
PAIRS=("arm64-v8a:aarch64-linux-android" "armeabi-v7a:armv7-linux-androideabi")
for arg in "$@"; do
  case "$arg" in
    --release) PROFILE="release" ;;
    --arm64-only) PAIRS=("arm64-v8a:aarch64-linux-android") ;;
    *) echo "error: unknown argument $arg" >&2; exit 2 ;;
  esac
done

# Every ABI this script knows how to stage, which is what the pruning below is allowed to touch --
# the list rather than "whatever is in $DEST", so a directory somebody put there by hand is left
# alone rather than silently deleted.
KNOWN_ABIS=("arm64-v8a" "armeabi-v7a")

DEST="ports/machine/android/app/src/main/jniLibs"
TARGET_DIR="$(dist_target_dir)"

staged=0
staged_abis=()
for pair in "${PAIRS[@]}"; do
  abi="${pair%%:*}"
  triple="${pair##*:}"
  out="$TARGET_DIR/$triple/$PROFILE"

  [ -f "$out/libkm_app.so" ] || continue

  mkdir -p "$DEST/$abi"
  cp "$out/libkm_app.so" "$DEST/$abi/"
  echo "$abi"
  printf '  %-16s %s\n' "libkm_app.so" "$(du -h "$DEST/$abi/libkm_app.so" | cut -f1)"

  # SDL3 and SDL3_ttf are built by their -sys crates into a build-script output directory whose name
  # contains a hash, so they have to be found rather than named.
  for lib in libSDL3.so libSDL3_ttf.so; do
    found=$(find "$TARGET_DIR/$triple" -path "*/out/lib/$lib" -print -quit 2>/dev/null || true)
    if [ -n "$found" ]; then
      cp "$found" "$DEST/$abi/"
      printf '  %-16s %s\n' "$lib" "$(du -h "$DEST/$abi/$lib" | cut -f1)"
    else
      # Worth stopping for: the APK would build and then fail to load at launch.
      echo "  error: $lib not found under $TARGET_DIR/$triple — was SDL built shared for Android?" >&2
      exit 1
    fi
  done

  # ffmpeg's four, and only when this build actually links them.
  #
  # **The library itself is asked, rather than a flag being passed down from build.sh.** A
  # `--no-video` build and a video one differ in exactly one observable way here -- whether
  # `libkm_app.so` carries a DT_NEEDED on libavcodec -- and reading that is what makes this step
  # unable to disagree with the build that produced it. A flag could; the ELF cannot.
  #
  # These are *named*, where SDL's two are *found*: they come from a prefix
  # tools/port/machine/android/ffmpeg.sh will tell us, not from a hashed build-script directory.
  # **Removed first, unconditionally, and that is not belt and braces.** This directory is not
  # cleaned between runs, so a video build followed by a `--no-video` one would leave four ffmpeg
  # libraries sitting here and Gradle would package them -- an APK carrying libraries nothing links,
  # and the `--no-video` build silently no smaller than the ordinary one. Staging what is wanted is
  # only half the job when the destination persists.
  rm -f "$DEST/$abi"/libav*.so "$DEST/$abi"/libsw*.so

  if "$NDK_BIN/llvm-readelf" -d "$DEST/$abi/libkm_app.so" 2>/dev/null | grep -q 'libavcodec\.so'; then
    ff="$(tools/port/machine/android/ffmpeg.sh --print-dir "$abi")"
    for lib in libavutil.so libswresample.so libavcodec.so libavformat.so; do
      if [ -f "$ff/lib/$lib" ]; then
        cp "$ff/lib/$lib" "$DEST/$abi/"
        printf '  %-16s %s\n' "$lib" "$(du -h "$DEST/$abi/$lib" | cut -f1)"
      else
        # Also worth stopping for, and for a nastier reason than SDL's: the APK would build, install
        # and start, and fail only when somebody queued a video song.
        echo "  error: $lib missing from $ff/lib, but libkm_app.so links it." >&2
        echo "         Rebuild it with: tools/port/machine/android/ffmpeg.sh $abi" >&2
        exit 1
      fi
    done
    # **The license is deliberately NOT copied here**, although here is where it belongs logically.
    # AGP packages only `*.so` out of jniLibs, so anything else put in this directory is dropped
    # from the APK silently -- the same rule that makes a versioned soname vanish. It is copied into
    # the assets tree by tools/port/machine/android/assets.sh instead, which is packaged wholesale.
  fi
  staged=$((staged + 1))
  staged_abis+=("$abi")
done

if [ "$staged" -eq 0 ]; then
  # The directory is named because "run build.sh first" is only one of the two reasons to get here,
  # and it is the reason a person will believe. The other is that cargo builds somewhere else than
  # this script looked, which a literal `target/` in the message cannot say.
  echo "error: nothing to stage -- no libkm_app.so under $TARGET_DIR/<triple>/$PROFILE." >&2
  echo "       Run tools/port/machine/android/build.sh first." >&2
  exit 1
fi

# **An ABI directory survives only if this run put libraries in it.** This is the rule stated for
# ffmpeg's four libraries above -- staging what is wanted is only half the job when the destination
# persists -- applied one level up, to the ABI directories themselves. It has to be, because the
# `rm -f` above only reaches inside an ABI directory the loop visited: a `--arm64-only` run never
# enters `armeabi-v7a`, so nothing there was ever cleaned by it.
#
# Placed after the check above on purpose: a run that staged nothing has already exited, so
# forgetting to build leaves the previous staging intact rather than emptying the source set.
for abi in "${KNOWN_ABIS[@]}"; do
  case " ${staged_abis[*]} " in *" $abi "*) continue ;; esac
  [ -d "$DEST/$abi" ] || continue
  rm -rf "${DEST:?}/$abi"
  echo "$abi"
  printf '  %-16s %s\n' "removed" "not part of this build"
done

echo
echo "staged $staged ABI(s) into $DEST"

# Said loudly. Both Google TV devices run a **32-bit** Android build on 64-bit-capable
# chips, so they load armeabi-v7a and ignore arm64-v8a entirely. An arm64-only APK installs on a
# phone and fails on a television, and the error there says nothing about the cause.
#
# **The pruning above is what makes this question answerable.** It asks whether a file is present
# and cannot tell a library staged a minute ago from one an old build left behind -- and such a
# leftover silences this warning in exactly the case it is for.
if [ ! -f "$DEST/armeabi-v7a/libkm_app.so" ]; then
  echo
  echo "warning: no armeabi-v7a build staged."
  echo "         Phones are fine, but a Google TV Streamer or Chromecast with Google TV runs a"
  echo "         32-bit OS and loads *only* armeabi-v7a. See the armv7 note in"
  echo "         docs/architecture/android.md."
fi

echo "now:  cd ports/machine/android && ./gradlew assembleDebug"
