#!/usr/bin/env bash
#
# Builds the promotional video, dist/promo/karaokemachine-promo.mp4: one minute at 1920x1080 and
# thirty frames a second, with sound.
#
#   tools/dev/promo-video.sh
#
# **The machine draws and plays its own part.** A carol from the released pack is the music from
# the first frame to the last. `examples/promo.rs` in km-display renders the playing screen from it,
# and `examples/render_wav.rs` in km-audio renders its sound through the bundled SoundFont. Both read
# one song file and one tempo map, so the words keep time with the music.
#
# **Everything else is published already.** The stills come from docs/images, the icon from icon/,
# and the palette from site/style.css. `tools/dev/promo/promo.html` lays them out over time, and
# `tools/dev/promo/capture.cjs` captures it frame by frame into ffmpeg.
#
# It needs:
#
#   * the network once, for the carol pack. tools/setup/carols-pin.sh pins it by digest.
#   * the SoundFont, which tools/setup/fetch-assets.sh installs.
#   * ffmpeg with libx264, and Node with Playwright (`npm install -g playwright`).
#
#   KM_CAROLS   a carol pack to read instead of the pinned download
#   KM_CHROME   a browser to capture with, instead of Playwright's own
#
# The video is a build product, so it lands in dist/ and is never committed. It depends on the
# system font the display finds, like every television picture.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=promo-video
dist_assert_root
. tools/setup/asset-cache.sh
. tools/setup/carols-pin.sh

# The carol, by its number in the pack: "Angels From the Realms of Glory", as in the README's
# animated picture. `The animated picture is of a public-domain carol` gives the reason.
CAROL=2

FPS=30

# The span the playing screen is rendered over, in milliseconds. It covers the video's full length,
# which `DURATION` in promo.html sets, and the audio runs a little past it for the fade.
FRAMES_MS=64000
AUDIO_MS=66000

SOUNDFONT="assets/soundfont/GeneralUser-GS.sf2"
OUT_DIR="dist/promo"
OUT="$OUT_DIR/karaokemachine-promo.mp4"

# The stills the page shows, from docs/images.
STILLS=(screen-idle-connect.png screen-queue.png remote-browse.png remote-now.png remote-queue.png)

[ -f "$SOUNDFONT" ] || {
  echo "$DIST_SCRIPT: no $SOUNDFONT; run tools/setup/fetch-assets.sh" >&2
  exit 1
}
command -v ffmpeg >/dev/null 2>&1 || { echo "$DIST_SCRIPT: need ffmpeg on PATH" >&2; exit 1; }
# Read whole before the test: under pipefail, `grep -q` quitting early fails ffmpeg on SIGPIPE.
encoders="$(ffmpeg -hide_banner -encoders 2>/dev/null || true)"
case "$encoders" in *libx264*) ;; *)
  echo "$DIST_SCRIPT: this ffmpeg has no libx264 encoder" >&2
  exit 1
  ;;
esac
command -v node >/dev/null 2>&1 || { echo "$DIST_SCRIPT: need Node on PATH" >&2; exit 1; }
# A global Playwright is found through NODE_PATH, which `require` reads and `import` does not.
NODE_PATH="$(npm root -g 2>/dev/null)${NODE_PATH:+:$NODE_PATH}"
export NODE_PATH
node -e 'require("playwright")' 2>/dev/null || {
  echo "$DIST_SCRIPT: need Playwright for Node (npm install -g playwright)" >&2
  exit 1
}

PACK="$(carols_pack)"

WORK="$(dist_target_dir)/promo"
dist_clear "$WORK"
mkdir -p "$WORK/stage/images" "$OUT_DIR"

dist_step "rendering the playing screen and the music"
cargo build $(dist_cargo_quiet) --release -p km-display --example promo -p km-audio --example render_wav
cargo run $(dist_cargo_quiet) --release -p km-display --example promo -- \
  "$PACK" "$CAROL" "$WORK/stage/frames" 0 "$FRAMES_MS" "$FPS"
cargo run $(dist_cargo_quiet) --release -p km-audio --example render_wav -- \
  "$WORK/stage/frames/song.kar" "$WORK/song.wav" "$SOUNDFONT" --melody on --max-ms "$AUDIO_MS" \
  >/dev/null

dist_step "staging the page"
cp tools/dev/promo/promo.html "$WORK/stage/"
cp icon/icon-512.png "$WORK/stage/"
for still in "${STILLS[@]}"; do
  cp "docs/images/$still" "$WORK/stage/images/"
done

dist_step "capturing $OUT"
node tools/dev/promo/capture.cjs "$WORK/stage/promo.html" "$WORK/song.wav" "$OUT" "$FPS"

mb=$(($(wc -c <"$OUT") / 1024 / 1024))
printf '  %-34s %5s MB\n' "$(basename "$OUT")" "$mb"
