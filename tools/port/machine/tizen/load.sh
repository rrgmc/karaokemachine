#!/usr/bin/env bash
#
# Runs the built machine inside the probe app on a Samsung television, over the probe app's port.
#
#   tools/port/machine/tizen/load.sh <address> [args] [--release]
#   tools/port/machine/tizen/load.sh 192.168.1.x 'data=<the probe app's data folder>,log=info'
#
# POSTs libkm_app.so to http://<address>:8177/load?a=<args>. The probe app writes it into its data
# folder, loads it and calls km_probe_run on its UI thread, then answers with what was logged: after
# the machine returns, or after 60 s if it is still running, which is the normal case. GET /log on the
# same port reads the rest later; the machine's own API is on port= (8277 unless told otherwise).
#
# args is km_probe_run's `key=value,...`: data= (required, the probe app's data folder, which its
# own screen prints), port=, log= (a tracing filter, `;` for `,`) and dev=1 (serve /dev/).
#
# **Run it once per load.** Each POST loads a fresh copy and starts it on the UI thread, so a second
# one while a machine runs starts another inside it. Hence one curl, in the foreground, no retry.
#
# The address is always given: nothing here knows where the set is.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

[ $# -ge 1 ] || { sed -n '3,6p' "$0" >&2; exit 2; }
address="$1"
shift
args=""
profile=debug
for arg in "$@"; do
  case "$arg" in
    --release) profile=release ;;
    *) args="$arg" ;;
  esac
done

so="local/tizen/target/armv7-unknown-linux-gnueabi/$profile/libkm_app.so"
[ -f "$so" ] || { echo "load: $so is not built; run tools/port/machine/tizen/build.sh first" >&2; exit 1; }

# Percent-encodes everything but the unreserved characters, byte by byte.
urlencode() {
  local LC_ALL=C text="$1" out="" c i
  for ((i = 0; i < ${#text}; i++)); do
    c="${text:i:1}"
    case "$c" in
      [A-Za-z0-9._~-]) out+="$c" ;;
      *) out+="$(printf '%%%02X' "'$c")" ;;
    esac
  done
  printf '%s' "$out"
}

echo "load: $so ($(du -h "$so" | cut -f1)) to $address, args '$args'" >&2
# 70 s: the probe app waits up to 60 s for km_probe_run before it answers. `Expect:` is emptied
# because the probe app's server never sends 100 Continue, which would cost a second per load.
curl --silent --show-error --max-time 70 \
  --data-binary "@$so" -H 'Content-Type: application/octet-stream' \
  -H 'Expect:' \
  "http://$address:8177/load?a=$(urlencode "$args")"
echo
