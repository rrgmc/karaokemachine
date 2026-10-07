#!/usr/bin/env bash
#
# Regenerates docs/images/screen-singing.webp, the one animated picture README.md publishes.
#
#   tools/dev/screen-animation.sh
#
# Two lines of a carol, sung on the playing screen over the shipped wallpaper. The frames come from
# `examples/screen_animation.rs` in km-display, and `img2webp` joins them into one animated WebP.
#
# **The song is a released package, not a corpus.** A clip publishes whole lines of words in motion,
# so the song must be one whose words anybody may publish. The carol pack is the only such package.
# It is a release asset, so this script runs on any machine with the network, and needs no corpus
# and no `abc2midi`. See `The animated picture is of a public-domain carol` in docs/decisions/.
# tools/dev/carol-pack.sh holds the pin and fetches the pack, and KM_CAROLS names another one.
#
# It needs `img2webp` (`apt install webp`, `brew install webp`), or an ffmpeg built with libwebp.
#
# The picture depends on the system font the display discovers, like every television picture. So a
# run on another platform rewrites it for no product change. Regenerate it when the playing screen
# changes, and not routinely.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=screen-animation

# The carol, by its number in the pack: "Angels From the Realms of Glory". Not one of the carols
# everybody has already heard, for the reason `What the README may show of a catalog` gives.
CAROL=2

OUT="docs/images/screen-singing.webp"

# The budget per picture that tools/dev/screenshots.sh sets. Advice, like the budget there.
BUDGET_KB=1200

if command -v img2webp >/dev/null 2>&1; then
  ENCODER=img2webp
elif command -v ffmpeg >/dev/null 2>&1 && ffmpeg -hide_banner -encoders 2>/dev/null | grep -q libwebp_anim; then
  ENCODER=ffmpeg
else
  echo "$DIST_SCRIPT: need img2webp (apt install webp, brew install webp) or ffmpeg with libwebp" >&2
  exit 1
fi

# -- the pack --------------------------------------------------------------------------------------

. tools/dev/carol-pack.sh

# -- the frames ------------------------------------------------------------------------------------

FRAMES="$(dist_target_dir)/screen-animation"
dist_clear "$FRAMES"

dist_step "rendering the frames"
cargo run -q -p km-display --example screen_animation -- "$PACK" "$CAROL" "$FRAMES"

# The interval the example rendered at. It writes it beside the frames, so the rate is stated once.
FRAME_MS="$(tr -d '[:space:]' <"$FRAMES/frame-ms")"

# -- the picture -----------------------------------------------------------------------------------

dist_step "encoding $OUT with $ENCODER"
# Lossy at quality 75. The frames are a photograph behind text, which lossless WebP stores at
# several times the size and no visible gain.
case "$ENCODER" in
  img2webp)
    img2webp -loop 0 -lossy -q 75 -m 6 -d "$FRAME_MS" "$FRAMES"/frame-*.png -o "$OUT" >/dev/null
    ;;
  ffmpeg)
    ffmpeg -hide_banner -loglevel error -y -framerate "$((1000 / FRAME_MS))" \
      -i "$FRAMES/frame-%04d.png" -c:v libwebp_anim -lossless 0 -q:v 75 -loop 0 "$OUT"
    ;;
esac

kb=$(($(wc -c <"$OUT") / 1024))
printf '  %-34s %5s KB\n' "$(basename "$OUT")" "$kb"
if [ "$kb" -gt "$BUDGET_KB" ]; then
  echo "  note: over the $BUDGET_KB KB budget for one picture"
fi
