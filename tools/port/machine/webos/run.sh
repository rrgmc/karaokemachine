#!/usr/bin/env bash
#
# Installs the probe build.sh packed and starts it on a set in Developer Mode, then prints its report.
#
#   tools/port/machine/webos/run.sh <device> [params-json]
#   tools/port/machine/webos/run.sh mytv '{"secs":120}'
#
# <device> is the name given to ares-setup-device. The report is read back over HTTP on port 8177,
# from the address ares has for that device.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

DEVICE="${1:?usage: run.sh <device> [params-json]}"
PARAMS="${2:-{\}}"
APP_ID=com.rrgmc.kmprobe

ipk="$(ls -t local/webos/*.ipk 2>/dev/null | head -1)"
[ -n "$ipk" ] || { echo "webos: no .ipk in local/webos; run build.sh first" >&2; exit 1; }

ares-install -d "$DEVICE" "$ipk"
ares-launch -d "$DEVICE" "$APP_ID" -p "$PARAMS"

host="$(ares-setup-device --list | awk -v d="$DEVICE" '$1 == d { for (i = 2; i <= NF; i++) if ($i ~ /@/) { split($i, a, "[@:]"); print a[2]; exit } }')"
[ -n "$host" ] || { echo "webos: no address for $DEVICE" >&2; exit 1; }
for _ in $(seq 1 20); do
  if curl -fsS --max-time 2 "http://$host:8177/report"; then exit 0; fi
  sleep 1
done
echo "webos: the probe did not answer on $host:8177" >&2
exit 1
