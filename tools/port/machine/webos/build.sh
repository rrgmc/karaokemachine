#!/usr/bin/env bash
#
# Cross-builds the webOS probe in Docker and packs it as an .ipk of type "native".
#
#   tools/port/machine/webos/build.sh
#
# Builds the km-webos image first (Dockerfile beside this; cached after the first time), then the
# probe in it, into local/webos/target, never the host's target/. The .ipk is packed on the host with
# ares-package (`npm install -g @webos-tools/cli`) and lands in local/webos/.
# docs/plans/webos-probe.md is what this is for; run.sh installs and starts the result.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

IMAGE=km-webos
HERE=tools/port/machine/webos
APP_ID=com.rrgmc.kmprobe
OUT=local/webos

for c in docker ares-package; do
  command -v "$c" >/dev/null 2>&1 || { echo "webos: $c is not on PATH" >&2; exit 1; }
done

# MSYS (Git Bash) rewrites anything that looks like a Unix path before Docker sees it; every path
# below is a container path, so it is switched off.
export MSYS2_ARG_CONV_EXCL='*'
host_path() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

RUST_VERSION="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)"
[ -n "$RUST_VERSION" ] || { echo "webos: could not read the channel from rust-toolchain.toml" >&2; exit 1; }

# The version and checksum Cargo.lock records for one registry crate, so the SDL3 in the image is
# the one the sdl3-sys crate expects.
lock_field() { # crate field
  awk -v name="$1" -v field="$2" '
    $0 == "name = \"" name "\"" { hit = 1; next }
    hit && $1 == field { gsub(/"/, "", $3); print $3; exit }
    /^\[\[package\]\]/ { hit = 0 }
  ' Cargo.lock
}
SDL3_SRC_VERSION="$(lock_field sdl3-src version)"
SDL3_SRC_SHA256="$(lock_field sdl3-src checksum)"
SDL3_TTF_SRC_VERSION="$(lock_field sdl3-ttf-src version)"
SDL3_TTF_SRC_SHA256="$(lock_field sdl3-ttf-src checksum)"
for v in SDL3_SRC_VERSION SDL3_SRC_SHA256 SDL3_TTF_SRC_VERSION SDL3_TTF_SRC_SHA256; do
  [ -n "${!v}" ] || { echo "webos: could not read $v from Cargo.lock" >&2; exit 1; }
done

echo "== image $IMAGE"
docker build -q -t "$IMAGE" --build-arg "RUST_VERSION=$RUST_VERSION" \
  --build-arg "SDL3_SRC_VERSION=$SDL3_SRC_VERSION" --build-arg "SDL3_SRC_SHA256=$SDL3_SRC_SHA256" \
  --build-arg "SDL3_TTF_SRC_VERSION=$SDL3_TTF_SRC_VERSION" \
  --build-arg "SDL3_TTF_SRC_SHA256=$SDL3_TTF_SRC_SHA256" \
  "$HERE" >/dev/null

echo "== probe"
mkdir -p "$OUT"
docker run --rm \
  -v "$(host_path "$PWD"):/src" -w /src/$HERE/probe \
  -v km-webos-cargo-registry:/opt/cargo/registry \
  -e CARGO_TARGET_DIR=/src/$OUT/target \
  "$IMAGE" bash -c 'cargo build --release --target armv7-unknown-linux-gnueabi &&
    file /src/'"$OUT"'/target/armv7-unknown-linux-gnueabi/release/km-webos-probe'

# The SDL3 test, linked against the image's patched static SDL3. The probe starts it for "sdl": 1.
echo "== km-sdl-test"
docker run --rm -v "$(host_path "$PWD"):/src" -w /src "$IMAGE" bash -c \
  'arm-linux-gnueabi-gcc -O2 -march=armv7-a -mfloat-abi=softfp -mfpu=neon -Wall \
     -I/opt/sdl3/include '"$HERE"'/sdl/km_sdl_test.c -o '"$OUT"'/km-sdl-test \
     /opt/sdl3/lib/libSDL3.a -ldl -lm -lpthread -lrt &&
   arm-linux-gnueabi-strip '"$OUT"'/km-sdl-test && file '"$OUT"'/km-sdl-test'

# The machine, as an executable with the webos feature, against the image's prebuilt SDL3 (the
# target triple chooses it; see crates/machine/karaokemachine/Cargo.toml). The probe starts it
# for "machine": 1.
echo "== karaokemachine"
. tools/setup/features.sh
docker run --rm \
  -v "$(host_path "$PWD"):/src" -w /src \
  -v km-webos-cargo-registry:/opt/cargo/registry \
  -e CARGO_TARGET_DIR=/src/$OUT/machine-target \
  "$IMAGE" bash -c 'cargo build --locked --release -p karaokemachine --bin karaokemachine \
      --target armv7-unknown-linux-gnueabi --features '"$KM_FEATURES_WEBOS"' &&
    file /src/'"$OUT"'/machine-target/armv7-unknown-linux-gnueabi/release/karaokemachine'

echo "== package"
app="$OUT/app"
rm -rf "$app" && mkdir -p "$app"
cp "$OUT/target/armv7-unknown-linux-gnueabi/release/km-webos-probe" "$OUT/km-sdl-test" \
  "$OUT/machine-target/armv7-unknown-linux-gnueabi/release/karaokemachine" "$app/"
# The machine looks for assets/ beside its executable: the SoundFont, the wallpapers, and a font.
mkdir -p "$app/assets/fonts" "$app/assets/soundfont" "$app/assets/wallpapers"
cp assets/soundfont/*.sf2 assets/soundfont/LICENSE.txt "$app/assets/soundfont/"
cp assets/wallpapers/default-wallpapers.zip assets/wallpapers/CREDITS.md "$app/assets/wallpapers/"
docker run --rm -v "$(host_path "$PWD/$app"):/app" "$IMAGE" \
  cp /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf /app/assets/fonts/karaoke.ttf
# A package to try the machine with, carried beside it; the probe installs it for "package". It
# stays in local/, since a package of real songs is not the repository's to carry.
if [ -n "${KM_WEBOS_PACKAGE:-}" ]; then
  cp "$KM_WEBOS_PACKAGE" "$app/"
fi
cp icon/icon-128.png "$app/icon.png"
cat > "$app/appinfo.json" <<EOF
{
  "id": "$APP_ID",
  "version": "0.0.1",
  "vendor": "karaokemachine",
  "type": "native",
  "main": "km-webos-probe",
  "title": "KM Probe",
  "icon": "icon.png",
  "requiredACG": []
}
EOF
rm -f "$OUT"/*.ipk
ares-package --no-minify -o "$OUT" "$app"

# ares-package on Windows writes every file 0666, so the set installs an executable it cannot run.
# Repack the data tarball in the image with real modes. An .ipk is an ar archive whose members must
# stay in this order, and the set's installer refuses GNU ar's `name/` headers, so the archive is
# written by hand in the form ares-package writes.
ipk="$(cd "$OUT" && ls *.ipk)"
docker run --rm -v "$(host_path "$PWD/$OUT"):/w" "$IMAGE" bash -euc '
  t=$(mktemp -d) && cd "$t"
  arm-linux-gnueabi-ar x "/w/'"$ipk"'"
  mkdir data && tar -xzf data.tar.gz -C data
  find data -type d -exec chmod 755 {} + && find data -type f -exec chmod 644 {} +
  chmod 755 data/usr/palm/applications/*/km-webos-probe data/usr/palm/applications/*/km-sdl-test \
    data/usr/palm/applications/*/karaokemachine
  tar -czf data.tar.gz -C data --owner=0 --group=0 --numeric-owner usr
  out="/w/'"$ipk"'"
  printf "!<arch>\n" > "$out"
  for m in debian-binary control.tar.gz data.tar.gz; do
    size=$(stat -c %s "$m")
    printf "%-16s%-12s%-6s%-6s%-8s%-10s\140\n" "$m" "$(date +%s)" 0 0 100644 "$size" >> "$out"
    cat "$m" >> "$out"
    [ $((size % 2)) -eq 0 ] || printf "\n" >> "$out"
  done'
ls -l "$OUT"/*.ipk
