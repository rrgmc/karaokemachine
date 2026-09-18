#!/usr/bin/env bash
#
# Bundles the machine's assets into the APK.
#
#   tools/port/machine/android/assets.sh
#
# Run before `gradlew assembleDebug`, alongside build.sh and stage.sh. Kept separate from stage.sh for
# the same reason stage.sh is separate from build.sh: this one is a copy rather than a build, and it is
# the step that decides how big the APK is.
#
# Everything here ends up in `ports/machine/android/app/src/main/assets/`, which Gradle packages as
# Android assets. Those are not files on the device — they live inside the APK — so `km_app`'s
# `androidassets` module unpacks them into the app-private directory on first run. It reads them
# through SDL, whose `SDL_IOFromFile` falls back to the APK's asset system for a relative path, so
# nothing here needs JNI.
#
# The MANIFEST this writes is what makes that work: SDL can open an asset by name but cannot *list* a
# directory, so the unpacker needs to be told what is in there. Sizes are included so that changing a
# file's contents without changing the set of files still triggers a re-unpack.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

DEST="ports/machine/android/app/src/main/assets"

# Before anything is removed or copied. This script has the same "what counts as an asset" loop as
# `dist_stage_assets` and deliberately does not share it (see the note there), so it calls the check
# itself rather than inheriting it. Plain stdout here -- nothing captures this script's output.
tools/dist/check-assets.sh

rm -rf "$DEST"
mkdir -p "$DEST"

# -- the asset tree ------------------------------------------------------------------------------

# `.gitkeep` exists to keep empty directories in git and would only be dead weight on a device.
copied=0
while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  mkdir -p "$DEST/$(dirname "$rel")"
  cp "assets/$rel" "$DEST/$rel"
  copied=$((copied + 1))
done < <(cd assets && find . -type f ! -name '.gitkeep' | sed 's|^\./||' | sort)

# -- the development remote ----------------------------------------------------------------------

# One static page, and worth the 30 KB: without it `/dev/` on a device serves the landing page and the
# whole API is only reachable with curl. `km-app` looks for it under the asset directory.
if [ -f tools/dev/remote/index.html ]; then
  mkdir -p "$DEST/remote-dev"
  cp tools/dev/remote/index.html "$DEST/remote-dev/index.html"
  copied=$((copied + 1))
fi

# -- ffmpeg's license, when ffmpeg is in the APK --------------------------------------------------

# **It goes in the assets tree and not beside the libraries, and that is not a preference.** The
# obvious home is `jniLibs/<abi>/`, next to the four `.so` files it covers -- and the Android Gradle
# plugin packages only `*.so` from that directory, so a `COPYING.LGPLv2.1.txt` put there is dropped
# without a word. That is the same trap as a versioned soname, discovered the same way: by counting
# what actually arrived in the APK rather than what was copied towards it.
#
# It matters because LGPL-2.1 is the whole reason those libraries may be shipped beside this
# application at all -- see the header of tools/port/machine/android/ffmpeg.sh -- and the terms have to travel
# with them.
#
# Keyed off the staged libraries rather than a flag, for the reason stage.sh gives: a `--no-video`
# build has no libavcodec.so and should carry no license for one.
for abi in ports/machine/android/app/src/main/jniLibs/*/; do
  [ -f "$abi/libavcodec.so" ] || continue
  ff="$(tools/port/machine/android/ffmpeg.sh --print-dir "$(basename "$abi")" 2>/dev/null || true)"
  if [ -n "$ff" ] && [ -f "$ff/COPYING.LGPLv2.1" ]; then
    cp "$ff/COPYING.LGPLv2.1" "$DEST/ffmpeg-COPYING.LGPLv2.1.txt"
    copied=$((copied + 1))
  fi
  break   # one copy; both ABIs are built from the same source under the same terms
done

# -- the manifest --------------------------------------------------------------------------------

# `size<TAB>path`, sorted, so the file is stable and a diff means something actually changed.
# `wc -c` rather than `stat`, whose size flag differs between GNU and BSD.
(
  cd "$DEST"
  find . -type f ! -name MANIFEST | sed 's|^\./||' | sort | while IFS= read -r p; do
    printf '%s\t%s\n' "$(wc -c < "$p" | tr -d ' ')" "$p"
  done
) > "$DEST/MANIFEST"

# -- report --------------------------------------------------------------------------------------

echo "bundled $copied file(s) into $DEST"
while IFS=$'\t' read -r size path; do
  printf '  %10s  %s\n' "$size" "$path"
done < "$DEST/MANIFEST"

total=$(awk -F'\t' '{s+=$1} END {print s+0}' "$DEST/MANIFEST")
printf '\ntotal %s bytes (~%s MiB) of assets\n' "$total" "$((total / 1024 / 1024))"

# Said rather than left to be discovered on a device: no bank means the machine comes up
# on its sine test tone, which sounds broken to anyone who does not know it is the documented
# fallback.
if ! grep -q '\.sf2$' "$DEST/MANIFEST"; then
  echo
  echo "warning: no SoundFont bundled — the app will fall back to a test tone."
  echo "         Run tools/setup/fetch-assets.sh first."
fi

# The assets are unpacked to app-private storage on first run, so they cost their own size twice on
# the device. A 32 MiB bank is fine; a 148 MiB one is worth thinking about, especially on a 32-bit
# television with 32 GB of storage.
if [ "$total" -gt $((64 * 1024 * 1024)) ]; then
  echo
  echo "note: over 64 MiB of assets. These are unpacked on first run, so the device pays"
  echo "      roughly twice this in storage. GeneralUser GS (~31 MiB) is the intended default;"
  echo "      FluidR3_GM (~142 MiB) is an override, not something to ship."
fi

echo
echo "now:  cd ports/machine/android && ./gradlew assembleDebug"
