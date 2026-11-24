#!/usr/bin/env bash
#
# Installs a song package into the machine running on a Samsung television, through its API.
#
#   tools/port/machine/tizen/songs.sh <address>                  # the synthetic test package
#   tools/port/machine/tizen/songs.sh <address> <package.kmpkg>  # any package
#
# With no package, builds local/tizen/songs/tizen-test-songs.kmpkg first, from synthetic songs only,
# with km-pack and its `video` feature:
#
#   - four MIDI songs from km_song::testing, written by km-song's write_fixtures example.
#     `high_quality_song` is two minutes of melody, piano, bass, strings and drums under timed
#     words, which is what M3's "heard clean" listens to;
#   - "Flash and Beep", a video song: ffmpeg's testsrc2 at 1920x1080, 30 fps and 8 Mbit/s H.264 High,
#     with 48 kHz AAC. A white square flashes and a beep sounds at the start of each second;
#   - "Words on the Beep", an MP3+G song: a 44.1 kHz MP3 that beeps at the start of each second,
#     and a .cdg from km-cdg's sync_song example whose word blocks light one per second, on the beep.
#
# The video and the MP3 need ffmpeg on PATH, built with libx264, aac and libmp3lame. The package id
# is fixed, so installing it again replaces it. The songs' numbers are printed at the end.
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

# The songs, how long the synthetic media lasts, and the package id they always go under.
SONGS="high_quality_song.mid melody_and_accompaniment.mid a_complete_short_song.mid soft_karaoke.kar"
SECONDS_LONG=60
TEST_ID=7e57501165000001

if [ -z "$package" ]; then
  command -v ffmpeg >/dev/null || { echo "songs: ffmpeg is not on PATH" >&2; exit 1; }
  out=local/tizen/songs
  package="$out/tizen-test-songs.kmpkg"
  spec="$out/tizen-test-songs.kmspec.yaml"
  rm -rf "$out/fixtures" "$out/midi" "$out/songs"
  mkdir -p "$out/songs"
  cargo run -q -p km-song --features testing --example write_fixtures -- "$out/fixtures" >/dev/null
  for song in $SONGS; do cp "$out/fixtures/$song" "$out/songs/"; done

  # A flash and a beep at the start of every second: the picture and the sound agree or they don't.
  ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "testsrc2=size=1920x1080:rate=30:duration=$SECONDS_LONG" \
    -f lavfi -i "sine=frequency=440:beep_factor=4:sample_rate=48000:duration=$SECONDS_LONG" \
    -vf "drawbox=x=iw/2-120:y=ih/2-120:w=240:h=240:color=white:t=fill:enable='lt(mod(t\,1)\,0.1)'" \
    -c:v libx264 -profile:v high -pix_fmt yuv420p -b:v 8M -c:a aac -b:a 128k -ac 2 \
    -metadata title="Flash and Beep" -metadata artist="Test Pattern" -shortest \
    "$out/songs/Test Pattern - Flash and Beep.mp4"

  # The same beep as an MP3, and a .cdg whose words light on it.
  stem="$out/songs/Sine Wave - Words on the Beep"
  ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=440:beep_factor=4:sample_rate=44100:duration=$SECONDS_LONG" \
    -af volume=4 -ac 2 -c:a libmp3lame -b:a 128k \
    -metadata title="Words on the Beep" -metadata artist="Sine Wave" "$stem.mp3"
  cargo run -q -p km-cdg --example sync_song -- "$stem.cdg" --seconds "$SECONDS_LONG" >/dev/null

  cargo run -q -p km-pack --features video -- spec "$out/songs" --id "$TEST_ID" \
    --name "Tizen test songs" --default-language en --out "$spec"
  cargo run -q -p km-pack --features video -- build "$spec" --out "$package"
  echo "songs: numbers in $package:" >&2
  sed -n 's/^- file: \(.*\)$/\1/p; s/^  number: \(.*\)$/  number \1/p' "$spec" | paste -d' ' - - >&2
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
