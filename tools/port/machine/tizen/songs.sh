#!/usr/bin/env bash
#
# Installs a song package into the machine running on a Samsung television, through its API.
#
#   tools/port/machine/tizen/songs.sh <address>                  # the synthetic test package
#   tools/port/machine/tizen/songs.sh <address> <package.kmpkg>  # any package
#
# With no package, builds local/tizen/songs/tizen-test-songs.kmpkg first, from synthetic MIDI songs
# only (km_song::testing, written by km-song's write_fixtures example) and with km-pack:
#
#   cargo run -p km-song --features testing --example write_fixtures -- local/tizen/songs/fixtures
#   cargo run -p km-pack -- spec local/tizen/songs/midi ...
#   cargo run -p km-pack -- build local/tizen/songs/tizen-test-songs.kmspec.yaml ...
#
# `high_quality_song` is two minutes of melody, piano, bass, strings and drums under timed words,
# which is what M3's "heard clean" listens to. The package id is fixed, so installing it again
# replaces it.
#
# Then signs in on http://<address>:8277 and POSTs the package to /api/v1/admin/packages/upload.
# The admin password is KM_ADMIN_PASSWORD, or asked for. A machine still on its generated PIN shows
# it on the television's screen.

set -euo pipefail

cd "$(dirname "$0")/../../../.."
export PATH="$HOME/.cargo/bin:$PATH"

[ $# -ge 1 ] || { sed -n '3,6p' "$0" >&2; exit 2; }
address="$1"
package="${2:-}"
api="http://$address:${KM_TIZEN_API_PORT:-8277}/api/v1"

# The songs, and the package id they always go under.
SONGS="high_quality_song.mid melody_and_accompaniment.mid a_complete_short_song.mid soft_karaoke.kar"
TEST_ID=7e57501165000001

if [ -z "$package" ]; then
  out=local/tizen/songs
  package="$out/tizen-test-songs.kmpkg"
  rm -rf "$out/fixtures" "$out/midi"
  mkdir -p "$out/midi"
  cargo run -q -p km-song --features testing --example write_fixtures -- "$out/fixtures" >/dev/null
  for song in $SONGS; do cp "$out/fixtures/$song" "$out/midi/"; done
  cargo run -q -p km-pack -- spec "$out/midi" --id "$TEST_ID" --name "Tizen test songs" \
    --default-language en --out "$out/tizen-test-songs.kmspec.yaml"
  cargo run -q -p km-pack -- build "$out/tizen-test-songs.kmspec.yaml" --out "$package"
fi
[ -f "$package" ] || { echo "songs: $package is not a file" >&2; exit 1; }

password="${KM_ADMIN_PASSWORD:-}"
if [ -z "$password" ]; then
  read -r -s -p "admin password for $address: " password
  echo >&2
fi

# The password goes in a request body on stdin, never on a command line another process can read.
token="$(printf '{"password":"%s"}' "$password" \
  | curl --silent --show-error --fail --max-time 20 -X POST "$api/admin/login" \
      -H 'content-type: application/json' --data-binary @- \
  | tr ',' '\n' | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')"
[ -n "$token" ] || { echo "songs: could not sign in to $api" >&2; exit 1; }

echo "songs: sending $package ($(du -h "$package" | cut -f1)) to $address" >&2
curl --silent --show-error --fail --max-time 300 -X POST "$api/admin/packages/upload" \
  -H "authorization: Bearer $token" -F "file=@$package"
echo
