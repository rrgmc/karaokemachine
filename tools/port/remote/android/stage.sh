#!/usr/bin/env bash
#
# Copies what tools/port/remote/android/build.sh produced into the Gradle source set.
#
#   tools/port/remote/android/stage.sh                 # the debug build, both ABIs
#   tools/port/remote/android/stage.sh --release       # the release build
#   tools/port/remote/android/stage.sh --arm64-only    # arm64 alone, and armv7 removed if present
#
# One file per ABI, and that is the whole job. The machine's equivalent also has to find SDL's two
# libraries under hashed build-script output directories and read `libkm_app.so`'s DT_NEEDED to
# decide whether ffmpeg goes beside it; this crate links `liblog`, `libdl`, `libm` and `libc`, all of
# which are on the device already.
#
# **`--arm64-only` mirrors build.sh's flag and has to be passed here as well**, which is worth the
# apparent redundancy. `build.sh --arm64-only` does not *delete* the armv7 library it skipped, so
# after any earlier both-ABI build one is still sitting in the target directory -- and a staging step
# that copies whatever it finds would put that stale library into the APK, months old, while the
# build that produced the APK never touched it. The ABI selection is a property of the build, not
# something the target directory can be asked about, so it has to be told to both halves.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
DIST_SCRIPT=stage

PROFILE="debug"
ABIS=("arm64-v8a" "armeabi-v7a")
for arg in "$@"; do
  case "$arg" in
    --release) PROFILE="release" ;;
    --arm64-only) ABIS=("arm64-v8a") ;;
    *) echo "$DIST_SCRIPT: unknown argument $arg" >&2; exit 2 ;;
  esac
done

# Every ABI this script knows how to stage, which is what the pruning below is allowed to touch. It
# is deliberately this list rather than "whatever directories are in $DEST": removing an unknown
# directory somebody put there by hand would be a surprise, and there is no ABI beyond these two.
KNOWN_ABIS=("arm64-v8a" "armeabi-v7a")

DEST="ports/remote/android/app/src/main/jniLibs"

# Asked of cargo, never spelled. See the same comment in build.sh: a worktree, a container and
# another machine all put this somewhere else, and guessing produces a script that reports a missing
# file on the line after cargo said `Finished`.
TARGET_DIR="$(dist_target_dir)"

dist_step "staging libkm_remote_android.so ($PROFILE) into $DEST"

staged=0
staged_abis=()
for abi in "${ABIS[@]}"; do
  case "$abi" in
    arm64-v8a) triple="aarch64-linux-android" ;;
    armeabi-v7a) triple="armv7-linux-androideabi" ;;
  esac
  src="$TARGET_DIR/$triple/$PROFILE/libkm_remote_android.so"
  [ -f "$src" ] || continue

  mkdir -p "$DEST/$abi"
  cp "$src" "$DEST/$abi/"
  printf '  %-12s %s\n' "$abi" "$(du -h "$DEST/$abi/libkm_remote_android.so" | cut -f1)"
  staged=$((staged + 1))
  staged_abis+=("$abi")
done

if [ "$staged" -eq 0 ]; then
  # The directory is named because "run build.sh first" is only one of the two reasons to get here,
  # and it is the one a person will believe. The other is that cargo built somewhere this script did
  # not look, which is unsayable when the path is a literal `target/`.
  echo "$DIST_SCRIPT: nothing to stage -- no libkm_remote_android.so under $TARGET_DIR/<triple>/$PROFILE." >&2
  echo "       Run tools/port/remote/android/build.sh first." >&2
  exit 1
fi

# **An ABI directory survives only if this run put a library in it.** Staging what is wanted is only
# half the job when the destination persists -- the same rule the machine's staging script states for
# ffmpeg's four libraries, applied one level up to the ABI directories themselves. AGP packages every
# ABI it finds under jniLibs, so a directory left behind by an earlier build is not inert: it is in
# the APK.
#
# This is placed after the check above on purpose. A run that staged nothing has already exited, so a
# build that was never run leaves the previous staging alone rather than emptying the source set --
# destroying a good staging is a worse answer to "you forgot to build" than the error already given.
for abi in "${KNOWN_ABIS[@]}"; do
  case " ${staged_abis[*]} " in *" $abi "*) continue ;; esac
  [ -d "$DEST/$abi" ] || continue
  rm -rf "${DEST:?}/$abi"
  printf '  %-12s %s\n' "$abi" "removed -- not part of this build"
done

# **Nothing but `*.so` may go in here.** AGP packages only shared libraries out of jniLibs and drops
# everything else silently, so a license or a README placed beside the library would simply not be
# in the APK. The machine's staging script records the same rule, having found it the hard way with
# a versioned soname.

echo
echo "staged $staged ABI(s) into $DEST"

# **No warning about a missing armeabi-v7a**, and the silence is deliberate.
# The machine's staging script warns loudly there, because every Google TV device runs a 32-bit OS
# and loads that ABI alone -- so an arm64-only APK installs on a phone and fails on a television.
# This is a phone application. It reaches no television, and repeating that warning here would send
# somebody to read about a device this APK never runs on.

echo "now:  cd ports/remote/android && ./gradlew assembleDebug"
