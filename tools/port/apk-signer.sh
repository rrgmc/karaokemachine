#!/usr/bin/env bash
#
# Says which key signed an APK.
#
#   tools/port/apk-signer.sh <apk>            # prints one line naming the signer
#   tools/port/apk-signer.sh --cn <apk>       # prints the certificate's CN alone, for a test to read
#
# **A report, not a gate.** It runs after `assembleRelease` in both Android tasks so the answer is
# visible at the moment of building, which is the rule `KM_SIGN_IDENTITY` follows on macOS: absent
# means the lesser build, and every report says which one it just made. What refuses to *publish* a
# debug-signed APK is `tools/dist/release.sh`, because a release is the place the answer has to be
# acted on rather than read.
#
# **A missing apksigner is said rather than swallowed.** It lives in the Android SDK's build-tools
# and nothing else here needs it, so a machine that builds APKs perfectly well may not have one; a
# report that silently said nothing would read like a build that signed nothing.

set -euo pipefail

CN_ONLY=0
APK=""
while [ $# -gt 0 ]; do
  case "$1" in
    --cn) CN_ONLY=1 ;;
    -h|--help) echo "usage: tools/port/apk-signer.sh [--cn] <apk>"; exit 0 ;;
    *) APK="$1" ;;
  esac
  shift
done

[ -n "$APK" ] || { echo "apk-signer: name an APK" >&2; exit 2; }
[ -f "$APK" ] || { echo "apk-signer: no such APK -- $APK" >&2; exit 2; }

# The newest build-tools that has one. `sort -V` so 36.1.0 beats 9.0.0, which a lexical sort does not.
find_apksigner() {
  local sdk dir
  if command -v apksigner >/dev/null 2>&1; then command -v apksigner; return 0; fi
  sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-${LOCALAPPDATA:-$HOME}/Android/Sdk}}"
  [ -d "$sdk/build-tools" ] || return 1
  while read -r dir; do
    for name in apksigner.bat apksigner; do
      [ -f "$dir/$name" ] && { printf '%s' "$dir/$name"; return 0; }
    done
  done < <(find "$sdk/build-tools" -maxdepth 1 -mindepth 1 -type d | sort -Vr)
  return 1
}

if ! SIGNER="$(find_apksigner)"; then
  [ "$CN_ONLY" -eq 1 ] && exit 1
  echo "   signed  unknown -- no apksigner in the Android SDK's build-tools"
  exit 0
fi

# apksigner prints one `Signer #N certificate DN:` line per signer; there is one here.
CERT="$("$SIGNER" verify --print-certs "$APK" 2>/dev/null | sed -n 's/^Signer #1 certificate DN: //p' | head -1)"

if [ -z "$CERT" ]; then
  [ "$CN_ONLY" -eq 1 ] && exit 1
  echo "   signed  nothing apksigner could read a certificate from"
  exit 0
fi

CN="$(printf '%s' "$CERT" | sed -n 's/.*CN=\([^,]*\).*/\1/p')"

if [ "$CN_ONLY" -eq 1 ]; then
  printf '%s\n' "$CN"
  exit 0
fi

if [ "$CN" = "Android Debug" ]; then
  echo "   signed  the debug key -- KM_ANDROID_KEYSTORE is unset, so this APK installs over nothing"
else
  echo "   signed  $CN"
fi
