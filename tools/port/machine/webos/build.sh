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

echo "== image $IMAGE"
docker build -q -t "$IMAGE" --build-arg "RUST_VERSION=$RUST_VERSION" "$HERE" >/dev/null

echo "== probe"
mkdir -p "$OUT"
docker run --rm \
  -v "$(host_path "$PWD"):/src" -w /src/$HERE/probe \
  -v km-webos-cargo-registry:/opt/cargo/registry \
  -e CARGO_TARGET_DIR=/src/$OUT/target \
  "$IMAGE" bash -c 'cargo build --release --target armv7-unknown-linux-gnueabi &&
    file /src/'"$OUT"'/target/armv7-unknown-linux-gnueabi/release/km-webos-probe'

echo "== package"
app="$OUT/app"
rm -rf "$app" && mkdir -p "$app"
cp "$OUT/target/armv7-unknown-linux-gnueabi/release/km-webos-probe" "$app/"
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
  chmod 755 data/usr/palm/applications/*/km-webos-probe
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
