#!/usr/bin/env bash
#
# Runs inside the km-tizen image, from tools/port/machine/tizen/build.sh, with the checkout at /src.
# Builds libkm_app.so for armv7-unknown-linux-gnueabi with video, then reads it back: the softfp
# ABI, the exported entry points, and every needed library checked against what the television is
# known to carry.
#
#   build-in-container.sh [cargo args]         # the library, assets carried inside it (probe app)
#   build-in-container.sh --app [cargo args]   # the library for the app, assets left to its res/
#   build-in-container.sh --clippy       # clippy over the Tizen code, -D warnings, and nothing built

set -euo pipefail

cd /src
. tools/dist/common.sh
. tools/setup/features.sh
DIST_SCRIPT=tizen
TRIPLE=armv7-unknown-linux-gnueabi

# For the probe app the assets ride inside the .so (the crate's `tizen-embed-assets` feature), so
# that its /load is all that reaches the set. The app's package carries them in res/assets instead,
# from the same staged tree (package.sh), and its library goes without.
FEATURES="$KM_FEATURES_TIZEN,tizen-embed-assets"
if [ "${1:-}" = "--app" ]; then
  shift
  FEATURES="$KM_FEATURES_TIZEN"
fi

# The asset tree as the set should see it, staged where build.rs reads it. Replaced only when its
# contents differ, so an unchanged tree keeps its timestamps and cargo does not rebuild the crate.
#
# The bundled bank rides too, so a MIDI song has instruments (M3): soundfont/GeneralUser-GS.sf2,
# where every carrier puts it, leaving gm.sf2 free for an override. Any other bank stays out. The
# font is the image's DejaVu Sans, because the repository bundles none and the set's own are unknown.
stage_assets() {
  local dest=local/tizen/assets next=local/tizen/assets.next
  [ -f assets/soundfont/GeneralUser-GS.sf2 ] || {
    echo "tizen: assets/soundfont/GeneralUser-GS.sf2 is missing; run tools/setup/fetch-assets.sh" >&2
    exit 1
  }
  rm -rf "$next"
  mkdir -p "$next"
  (cd assets && find . -type f ! -name '.gitkeep' ! -name 'wallpapers-*.zip' \
      \( ! -name '*.sf2' -o -name 'GeneralUser-GS.sf2' \)) \
    | sed 's|^\./||' | while IFS= read -r rel; do
        mkdir -p "$next/$(dirname "$rel")"
        cp "assets/$rel" "$next/$rel"
      done
  mkdir -p "$next/fonts" "$next/remote-dev"
  cp /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf "$next/fonts/karaoke.ttf"
  cp /usr/share/doc/fonts-dejavu-core/copyright "$next/fonts/karaoke.ttf.LICENSE.txt"
  cp tools/dev/remote/index.html "$next/remote-dev/index.html"
  if [ -d "$dest" ] && diff -r "$dest" "$next" >/dev/null 2>&1; then
    rm -rf "$next"
  else
    rm -rf "$dest"
    mv "$next" "$dest"
  fi
  echo "assets staged in $dest ($(du -sh "$dest" | cut -f1)):"
  (cd "$dest" && find . -type f | sed 's|^\./|  |' | sort)
}

stage_assets
export KM_TIZEN_ASSETS=/src/local/tizen/assets

if [ "${1:-}" = "--clippy" ]; then
  shift
  # Both feature sets: the display half is linted without the carried assets, and the entry point's
  # embedding with them.
  for features in "$KM_FEATURES_TIZEN" "$FEATURES"; do
    echo "== clippy --features $features"
    cargo clippy --locked --target "$TRIPLE" -p karaokemachine --features "$features" "$@" -- -D warnings
  done
  exit 0
fi

cargo build --locked -p karaokemachine --lib --target "$TRIPLE" --features "$FEATURES" "$@"

profile=debug
for arg in "$@"; do [ "$arg" = "--release" ] && profile=release; done
so="$(dist_target_dir)/$TRIPLE/$profile/libkm_app.so"
[ -f "$so" ] || { echo "tizen: cargo finished but $so was not produced" >&2; exit 1; }

readelf=arm-linux-gnueabi-readelf
echo
echo "built: $so ($(du -h "$so" | cut -f1))"

# softfp: hardware float inside, integer registers across calls. A hard-float object would say
# "VFP registers" here and fail to load on the set.
if "$readelf" -A "$so" | grep -q 'Tag_ABI_VFP_args: VFP registers'; then
  echo "tizen: $so uses the hard-float calling convention; the set is softfp" >&2
  exit 1
fi

# What load.sh sends: debug information off, the symbol table kept so a backtrace still names its
# functions. A debug build is hundreds of megabytes before this, nearly all of it DWARF.
stripped="${so%.so}.stripped.so"
arm-linux-gnueabi-strip --strip-debug -o "$stripped" "$so"
echo "for load.sh: $stripped ($(du -h "$stripped" | cut -f1))"
"$readelf" -h "$so" | grep -E 'Class|Machine|Flags'
"$readelf" -A "$so" | grep -E 'Tag_CPU_arch|Tag_FP_arch|Tag_Advanced_SIMD_arch|Tag_ABI_VFP_args'

# The entry points the hosts look up by name: the probe app's, M5's, and the app's pause and resume.
echo "exported:"
for symbol in km_probe_run km_tizen_run km_tizen_set_visible; do
  if "$readelf" --dyn-syms -W "$so" | awk '$4 == "FUNC" && $7 != "UND" { print $8 }' | grep -qx "$symbol"; then
    echo "  $symbol"
  else
    echo "tizen: $so does not export $symbol" >&2
    exit 1
  fi
done

# Measured on the set (docs/research/tizen.md): glibc 2.40 and the libraries in /usr/lib. libgcc_s
# comes with every glibc system. ffmpeg 7.1 is on the set whole, libswresample and libswscale included.
echo "needed:"
unknown=0
while read -r lib; do
  case "$lib" in
    libc.so.6|libm.so.6|libdl.so.2|libpthread.so.0|librt.so.1|libutil.so.1|ld-linux.so.3|libgcc_s.so.1|\
    libasound.so.2|libavcodec.so.61|libavformat.so.61|libavutil.so.59|libswresample.so.5|libswscale.so.8)
      echo "  $lib" ;;
    *) echo "  $lib   <- not known to be on the set"; unknown=$((unknown + 1)) ;;
  esac
done < <("$readelf" -d "$so" | sed -n 's/.*(NEEDED).*Shared library: \[\(.*\)\]/\1/p')
newest_glibc="$("$readelf" -V "$so" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
echo "newest glibc symbol: $newest_glibc (the set has 2.40)"
[ "$unknown" -eq 0 ] || echo "tizen: $unknown needed librar(ies) not known to be on the set"
