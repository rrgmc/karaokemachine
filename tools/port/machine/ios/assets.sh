#!/usr/bin/env bash
#
# Stages the machine's assets into the iOS application.
#
#   tools/port/machine/ios/assets.sh
#
# Run before `xcodebuild`; `build.sh` calls it. Kept separate from build.sh for the reason Android's
# is separate from its own: this one is a copy rather than a build, and it is the step that decides
# how big the bundle is.
#
# Everything here ends up in `ports/machine/ios/KaraokeMachine/assets/`, which `project.yml` includes
# as a *folder reference* -- so Xcode copies the tree wholesale to the bundle root, beside the
# executable, where `Paths::asset_dirs_from` already looks for it.
#
# **There is no MANIFEST and no unpacker, which is the whole difference from Android.** That platform
# needs both because an APK's assets are not files: only the asset API can read them, and SDL can
# open one by name but cannot list a directory, so the unpacker has to be told what is in there. An
# `.app` is an ordinary filesystem. Nothing is unpacked, nothing is counted twice on the device, and
# the tree is readable exactly where it was copied.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
DIST_SCRIPT=assets

DEST=ports/machine/ios/KaraokeMachine/assets

# Before anything is removed or copied, so that a missing bank is reported once rather than after a
# minute of copying. The same check Android's assets.sh and `dist_stage_assets` call.
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

# -- the lyric font -------------------------------------------------------------------------------

# **A bundle carries its own Latin face, and that is the treatment the Linux tarball gets.** Neither
# carrier can name a font package, and `km-display`'s list of system paths is Debian- and
# Arch-shaped: it has no iOS entry, and the paths this platform keeps its faces at are undocumented,
# which is exactly the way `PingFang.ttc` was learned the hard way. So `find_font`'s bundled arm is
# the one that has to answer here, and `fonts/karaoke.ttf` is where it looks.
#
# `fetch-assets.sh --font` pins DejaVu Sans and puts it in the shared cache. It prints the font and
# then its license, one per line; the license is copied beside the font because the terms are what
# make shipping it legitimate.
#
# `display.font` still overrides all of this.
font_lines="$(tools/setup/fetch-assets.sh --font)"
font="$(echo "$font_lines" | sed -n '1p')"
font_license="$(echo "$font_lines" | sed -n '2p')"
if [ -n "$font" ] && [ -f "$font" ]; then
  mkdir -p "$DEST/fonts"
  cp "$font" "$DEST/fonts/karaoke.ttf"
  copied=$((copied + 1))
  if [ -n "$font_license" ] && [ -f "$font_license" ]; then
    cp "$font_license" "$DEST/fonts/karaoke-LICENSE.txt"
    copied=$((copied + 1))
  fi
else
  echo "$DIST_SCRIPT: no font was fetched; the machine will find none and draw no words." >&2
  exit 1
fi

# -- the development remote ----------------------------------------------------------------------

# One static page, and worth the 30 KB: without it `/dev/` on a device serves the landing page and
# the whole API is only reachable with curl. `km-app` looks for it under the asset directory.
if [ -f tools/dev/remote/index.html ]; then
  mkdir -p "$DEST/remote-dev"
  cp tools/dev/remote/index.html "$DEST/remote-dev/index.html"
  copied=$((copied + 1))
fi

# -- ffmpeg's license, when ffmpeg is in the bundle ------------------------------------------------

# **The terms are what make shipping those four libraries legitimate**, so they travel with them.
# Android lost this once: the obvious home is beside the libraries, and the Android Gradle plugin
# packages only `*.so` out of `jniLibs`, so a `COPYING.LGPLv2.1` put there was dropped without a
# word. Here the asset tree is the home that actually arrives, because it is a folder reference
# copied whole -- and `build.sh` counts what is in the built bundle rather than what was copied
# towards it.
#
# **Keyed off the wrapped frameworks rather than a flag**, on the reasoning Android's own step
# gives: a `--no-video` build carries no libavcodec and should carry no license for one.
if [ -d ports/machine/ios/Frameworks/avcodec.xcframework ]; then
  ff="$(tools/port/machine/ios/ffmpeg.sh --print-dir device)"
  if [ -n "$ff" ] && [ -f "$ff/COPYING.LGPLv2.1" ]; then
    cp "$ff/COPYING.LGPLv2.1" "$DEST/ffmpeg-COPYING.LGPLv2.1.txt"
    copied=$((copied + 1))
  else
    echo "$DIST_SCRIPT: warning -- ffmpeg is in the bundle and its terms are not." >&2
  fi
fi

# -- report --------------------------------------------------------------------------------------

total=$(find "$DEST" -type f -exec cat {} + | wc -c | tr -d ' ')
dist_step "staged $copied file(s) into $DEST"
dist_detail "$total bytes (~$((total / 1024 / 1024)) MiB)"

# Said rather than left to be discovered on a device: no bank means the machine comes up on its sine
# test tone, which sounds broken to anyone who does not know it is the documented fallback.
if ! find "$DEST" -name '*.sf2' | grep -q .; then
  echo
  echo "warning: no SoundFont staged -- the app will fall back to a test tone."
  echo "         Run tools/setup/fetch-assets.sh first."
fi
