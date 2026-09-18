#!/usr/bin/env sh
#
# The whole API, one call at a time, with curl.
#
# The companion to index.html: that page is for driving the machine by hand, this is for reading.
# Every endpoint appears once, in the order a client would actually use them, so "what does a remote
# have to do?" is answered by scrolling rather than by reading Rust.
#
# Usage:
#   ./api-walkthrough.sh                        # against http://127.0.0.1:8177
#   BASE=http://192.168.1.42:8177 ./api-walkthrough.sh
#   PASSWORD=hunter2 ./api-walkthrough.sh       # also exercises admin mode
#
# It is read-only by default. Set MUTATE=1 to let it queue, transpose and skip.

set -eu

BASE="${BASE:-http://127.0.0.1:8177}"
API="$BASE/api/v1"
MUTATE="${MUTATE:-0}"
PASSWORD="${PASSWORD:-}"
TOKEN=""

# `-sS` so a failure is reported but the body is not decorated; `--fail-with-body` so a 4xx still
# prints the error document, which is the interesting part.
call() {
  method="$1"
  path="$2"
  body="${3:-}"
  printf '\n\033[1m%s %s\033[0m\n' "$method" "$path"
  if [ -n "$body" ]; then
    curl -sS --fail-with-body -X "$method" "$API$path" \
      -H 'content-type: application/json' \
      ${TOKEN:+-H "authorization: Bearer $TOKEN"} \
      -d "$body" || true
  else
    curl -sS --fail-with-body -X "$method" "$API$path" \
      ${TOKEN:+-H "authorization: Bearer $TOKEN"} || true
  fi
  printf '\n'
}

section() { printf '\n\033[36m── %s ─────────────────────────────\033[0m\n' "$1"; }

section 'Finding the machine'
# Always public, always unauthenticated. This is what a phone asks first, and what the QR code on
# screen leads to.
call GET /discover
# Where the machine thinks it is reachable, and honestly why not when it is not.
call GET /connect

section 'What is happening'
call GET /state
call GET /queue
call GET /settings
call GET /mics
# Every output the machine could play through, plus a synthetic "system" entry -- ALSA cannot
# enumerate its own default device, so "follow the system" has to be added rather than found.
# `selected` is what settings ask for and `active_id` is what is actually open: they disagree when a
# saved device has been unplugged, and the machine deliberately keeps the setting so it comes back.
call GET /audio/outputs
# ...and what the sound coming out of it is *made of*: the bank chosen at startup, or the reason
# there is none. `playing` is soundfont / test_tone / silent, and only the last refuses a song --
# a machine on a test tone plays every song with the lyrics in time and the instruments wrong,
# which is otherwise invisible from off the box. The same answer `--show-paths` prints.
call GET /audio/soundfont
# ...and every bank it could be switched to, which is a different question again: this one says what
# the *setting* names, and during a `Ctrl+1`..`Ctrl+9` A/B the two disagree on purpose. `offers` is
# what the machine would download if asked -- the nine it offers, minus the bundled one and minus
# whatever is already installed -- and `fetching` is how the last download went.
call GET /audio/soundfonts
# The same answer at the other width: the whole survey, sixty-odd banks, each row marked `offered`
# or not. It is a width and not a permission -- `audio.read` either way -- and it is what the `/dev/`
# page asks for. A phone gets the nine; nobody browses a catalog on one.
call GET '/audio/soundfonts?all=true'
# Choosing and fetching are `audio.write`, so admin by default, and they are deliberately NOT run
# here. Both take one id and answer with the whole list:
#
#   curl -sS -X PUT  "$API/audio/soundfont"       -H 'content-type: application/json' -d '{"id":"..."}'
#   curl -sS -X POST "$API/audio/soundfont/fetch" -H 'content-type: application/json' -d '{"id":"..."}'
#
# The first rebuilds the audio stream, which puts a hole in whatever is playing. The second answers
# when the download has *started* and then pulls up to a gigabyte in the background, which is not
# something a walkthrough should do to somebody's connection; watch `fetching` above for how it goes.
# `pictures` is one row per *file*, so a zip says how many pictures it holds and is removed whole —
# the unit is the file, exactly as a package is. `DELETE /wallpapers/<id>` takes one; it is not
# called here because a walkthrough should not delete somebody's picture, and `removable: false` on
# a row is the machine saying it would refuse anyway.
call GET /wallpapers
# What the machine does when nobody is singing. Public, unlike its `PUT` twin below, and that split
# is the whole design: a remote has to be able to tell a singer that a demo song is on the deck and
# that *skipping* — not queueing — is what takes the machine, but turning the mode on reconfigures
# the machine rather than adjusting a performance. `starts_in_secs` is null rather than zero once the
# deadline has passed, and null again whenever something is loaded or queued: both mean "no
# countdown to show", which is not the same as "nothing is coming".
call GET /demo
call GET /packages
call GET /debug

section 'Searching the catalog'
call GET '/songs?limit=3'
call GET '/songs?q=love&sort=suitability&limit=3'
call GET '/songs?min_suitability=8&melody_only=true&limit=3'
# Paging: `more` says whether another page may exist, without a COUNT(*) over the whole catalog.
call GET '/songs?limit=2&offset=2'

# The whole catalog, for a client keeping its own copy. NDJSON -- one song per line, no wrapping
# array -- paged by `after=<the last number of the previous page>` rather than by an offset, because
# SQLite walks an offset row by row and a six-figure catalog paged that way costs time proportional
# to the square of its size. Keep asking until a page comes back shorter than the limit.
#
# The response carries `X-Km-Catalog-Version`, which also appears in `/discover`. It moves on every
# install and uninstall and on nothing else, so a client that stored it can ask `/discover` -- always
# public, always cheap -- and skip the download entirely when nothing has changed.
section 'The whole catalog, for a client that mirrors it'
printf '\n$ curl -sS -D- "%s/songs/export?limit=3"\n' "$API"
curl -sS -D- "$API/songs/export?limit=3" | sed -n '1,12p'

# The first number in the catalog, so the rest of the walkthrough has something real to act on.
NUMBER="$(curl -sS "$API/songs?limit=1" | sed -n 's/.*"number":\([0-9]*\).*/\1/p' | head -1)"
if [ -n "${NUMBER:-}" ]; then
  section "One song (number $NUMBER)"
  call GET "/songs/$NUMBER"
  # Milliseconds, not ticks: the server owns the tempo map, including tempo changes inside the song,
  # so no client has to reimplement it.
  call GET "/songs/$NUMBER/lyrics"
else
  printf '\n(no songs catalogd — install a package to exercise the rest)\n'
fi

section 'Errors, on purpose'
# A number nobody has: 404 with the stable code `not_found`.
call GET /songs/999999999
# A misspelled field: rejected rather than silently ignored, so a client learns it made a typo.
call PUT /settings '{"transpoze": 2}'
# An output device that is not there. 404 rather than a silent no-op, because a remote sending a
# stale id -- one saved before the interface was unplugged -- has to be told.
#
# Without a token this answers 401 instead, whoever is asking and from wherever — it is under
# `/api/v1/admin/`, and that is the whole of the rule. Set PASSWORD to see the 404.
call PUT /admin/audio/output '{"id": "alsa:plughw:CARD=Nothing,DEV=0"}'
# An unversioned path.
printf '\n\033[1mGET /api/nonsense\033[0m\n'
curl -sS --fail-with-body "$BASE/api/nonsense" || true
printf '\n'

if [ -n "$PASSWORD" ]; then
  section 'Admin mode'
  call POST /admin/login "{\"password\": \"$PASSWORD\"}"
  TOKEN="$(curl -sS -X POST "$API/admin/login" -H 'content-type: application/json' \
    -d "{\"password\": \"$PASSWORD\"}" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')"
  if [ -n "$TOKEN" ]; then
    printf 'token: %s…\n' "$(printf '%s' "$TOKEN" | cut -c1-12)"
    # Everything under /api/v1/admin/ needs this token, and nothing outside it does. There is no
    # per-route map any more: the prefix is the permission.


    # Two of the routes that ship admin rather than public, and the only places in the walkthrough
    # that *need* the token. Choosing the machine's output socket is installation configuration; a
    # guest who can queue a song has no business moving the sound out of the room.
    #
    # "system" always exists, so this is safe to run anywhere -- and it is a real change: the sentinel
    # is stored rather than clearing the setting, because "follow the system, deliberately" and
    # "nothing has ever been chosen" are different states and only the second re-runs the first-run
    # rule on the next start.
    call PUT /admin/audio/output '{"id": "system"}'
    # **Off, and un-persisted, and both halves of that are deliberate.** `enabled: false` is the
    # direction that cannot surprise anybody: a walkthrough that switched demo mode *on* would start
    # music on somebody's machine two minutes later with no obvious cause. Omitting `persist` — the
    # default — means this touches the running machine and not the settings file, so a restart
    # restores whatever the owner actually chose. Run against a machine that was demoing, it does
    # stop the mode; it does not stop the song already playing, which is a mode-not-a-transport
    # distinction the decision spells out.
    call PUT /admin/demo '{"enabled": false}'
    # Sign out everywhere: the epoch moves and every token stops verifying, this one included.
    # Commented out so the walkthrough can go on using its own token.
    # call POST /admin/sessions/reset
    call POST /admin/logout
    TOKEN=""
  fi
  # A wrong password, five times, is a lockout. Uncomment to see the 429 and its Retry-After.
  # for i in 1 2 3 4 5 6; do call POST /admin/login '{"password": "wrong"}'; done
else
  printf '\n(set PASSWORD=… to exercise admin mode)\n'
fi

if [ "$MUTATE" = "1" ] && [ -n "${NUMBER:-}" ]; then
  section 'Queueing and playing'
  call POST /queue "{\"number\": $NUMBER, \"singer\": \"walkthrough\"}"
  ENTRY="$(curl -sS "$API/queue" | sed -n 's/.*"id":\([0-9]*\).*/\1/p' | head -1)"
  # Moves and removals are by entry id, never by position: another phone queueing a song shifts
  # every position, and acting on a stale one hits the wrong song.
  [ -n "${ENTRY:-}" ] && call POST "/queue/$ENTRY/move" '{"to_index": 0}'
  call POST /transport/play
  call PUT /settings '{"transpose": -2}'
  call PUT /settings '{"tempo_ratio": 0.9, "music_volume": 0.8}'
  # One field on its own, which is the point of it: the response has to come back with the tempo and
  # volume set just above still at 0.9 and 0.8. A patch that quietly restated the other four would
  # undo whatever another phone had just set.
  #
  # Clamped rather than refused, unlike the transpose and tempo above -- it is a calibration dial, so
  # 9999 comes back as the 500 ms limit and a 200, not a 400. And it is never refused for a video
  # song: it calibrates the screen, not the notes.
  call PUT /settings '{"lyric_offset_ms": 40}'
  call PUT /settings '{"lyric_offset_ms": 9999}'
  # Unavailable rather than refused when detection abstained: a 409, not a 400.
  call PUT /settings '{"melody_enabled": true}'
  call POST /transport/seek '{"ms": 30000}'
  call POST /transport/pause
  call POST /transport/skip
  call POST /transport/stop
  call PUT /settings '{"transpose": 0}'
  call PUT /settings '{"lyric_offset_ms": 0}'
  call PUT /mics/mic1 '{"muted": true}'
  call PUT /mics/mic1 '{"muted": false, "reverb": 0.3}'
  call POST /wallpapers/next
  call DELETE /queue
else
  printf '\n(set MUTATE=1 to exercise queueing, transport and settings)\n'
fi

section 'The event stream'
cat <<'EOF'
The one thing curl cannot do. With websocat:

    websocat ws://127.0.0.1:8177/api/v1/events

The first message is always a `state`, sent immediately so a client is synchronized the instant it
connects rather than up to 250 ms later. After that: `state` four times a second, plus
`queue_changed`, `song_started`, `song_ended`, `lyric_line`, `settings_changed`, `mics_changed` and
`wallpaper_changed` as they happen.

Per-syllable position is never streamed. Fetch /songs/{number}/lyrics once and interpolate against
the position on the `state` event; a song has thousands of syllables and pushing each one would be a
thousandfold traffic increase to say something the client can already work out.

A `desync` message means this connection fell far enough behind that events were dropped. Re-fetch;
do not assume the queue you are holding is current.
EOF
