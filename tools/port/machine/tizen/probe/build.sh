#!/usr/bin/env bash
# Builds the M1 nested-loop probe, nested_loop.c, into one armel shared library exporting
# km_probe_run, inside the Tizen build image. See README.md beside this file.
#
#   tools/port/machine/tizen/probe/build.sh
#
# KM_TIZEN_IMAGE  the image to build in; default km-tizen. It needs arm-linux-gnueabi-gcc and the
#                 armel Wayland client headers and library.
# SDL3_PREFIX     a host folder holding the patched static SDL3 (include/ and lib/libSDL3.a). When
#                 set it is mounted into the container; when unset, the image's own /opt/sdl3 is used.
# KM_PROBE_OUT    the output folder on the host; default local/tizen/probe in the checkout, which git
#                 ignores.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../../.." && pwd)"
image="${KM_TIZEN_IMAGE:-km-tizen}"
out="${KM_PROBE_OUT:-$repo/local/tizen/probe}"
mkdir -p "$out"

# MSYS rewrites Unix-looking arguments before a Windows docker.exe sees them, which mounts the wrong
# folder; and docker wants the host side of a mount in Windows form there.
export MSYS2_ARG_CONV_EXCL='*'
host_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

if ! docker image inspect "$image" >/dev/null 2>&1; then
  echo "probe: no image '$image'. Build the Tizen image first, or name another with KM_TIZEN_IMAGE." >&2
  exit 1
fi

mounts=(-v "$(host_path "$here")":/probe:ro -v "$(host_path "$out")":/out)
sdl=/opt/sdl3
if [ -n "${SDL3_PREFIX:-}" ]; then
  if [ ! -f "$SDL3_PREFIX/lib/libSDL3.a" ]; then
    echo "probe: SDL3_PREFIX has no lib/libSDL3.a" >&2
    exit 1
  fi
  mounts+=(-v "$(host_path "$(cd "$SDL3_PREFIX" && pwd)")":/sdl3:ro)
  sdl=/sdl3
fi

script='
set -eu
test -f "$SDL/lib/libSDL3.a" || { echo "probe: no $SDL/lib/libSDL3.a in the image" >&2; exit 1; }
arm-linux-gnueabi-gcc -march=armv7-a -mfloat-abi=softfp -mfpu=neon -fPIC -O2 -Wall -Wextra -shared \
  -I"$SDL/include" -o /out/nested-loop.so /probe/nested_loop.c \
  -Wl,--whole-archive "$SDL/lib/libSDL3.a" -Wl,--no-whole-archive -lwayland-client -ldl -lpthread -lm
arm-linux-gnueabi-readelf -d /out/nested-loop.so | grep NEEDED
arm-linux-gnueabi-readelf -A /out/nested-loop.so | grep -E "Tag_ABI_VFP_args|Tag_FP_arch|Tag_CPU_arch:" || true
arm-linux-gnueabi-nm -D --defined-only /out/nested-loop.so | grep " km_probe_run$"
'
docker run --rm "${mounts[@]}" -e SDL="$sdl" "$image" bash -c "$script"
echo "probe: $(host_path "$out")/nested-loop.so"
