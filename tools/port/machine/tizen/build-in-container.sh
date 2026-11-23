#!/usr/bin/env bash
#
# Runs inside the km-tizen image, from tools/port/machine/tizen/build.sh, with the checkout at /src.
# Builds libkm_app.so for armv7-unknown-linux-gnueabi with video, then reads it back: the softfp
# ABI, and every needed library checked against what the television is known to carry.

set -euo pipefail

cd /src
. tools/dist/common.sh
. tools/setup/features.sh
DIST_SCRIPT=tizen
TRIPLE=armv7-unknown-linux-gnueabi

cargo build --locked -p karaokemachine --lib --target "$TRIPLE" --features "$KM_FEATURES_TIZEN" "$@"

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
"$readelf" -h "$so" | grep -E 'Class|Machine|Flags'
"$readelf" -A "$so" | grep -E 'Tag_CPU_arch|Tag_FP_arch|Tag_Advanced_SIMD_arch|Tag_ABI_VFP_args'

# Measured on the set (docs/research/tizen.md): glibc 2.40 and the libraries in /usr/lib. libgcc_s
# comes with every glibc system. libswresample is ffmpeg's, but not known to be on the set.
echo "needed:"
unknown=0
while read -r lib; do
  case "$lib" in
    libc.so.6|libm.so.6|libdl.so.2|libpthread.so.0|librt.so.1|libutil.so.1|ld-linux.so.3|libgcc_s.so.1|\
    libasound.so.2|libavcodec.so.61|libavformat.so.61|libavutil.so.59)
      echo "  $lib" ;;
    *) echo "  $lib   <- not known to be on the set"; unknown=$((unknown + 1)) ;;
  esac
done < <("$readelf" -d "$so" | sed -n 's/.*(NEEDED).*Shared library: \[\(.*\)\]/\1/p')
newest_glibc="$("$readelf" -V "$so" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
echo "newest glibc symbol: $newest_glibc (the set has 2.40)"
[ "$unknown" -eq 0 ] || echo "tizen: $unknown needed librar(ies) not known to be on the set"
