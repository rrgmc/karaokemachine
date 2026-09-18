#!/usr/bin/env bash
#
# Regenerates the pictures in docs/images that README.md publishes.
#
#   tools/dev/screenshots.sh              # everything this machine can do
#   tools/dev/screenshots.sh --display    # the three SDL pictures only: no servers, no browser
#   tools/dev/screenshots.sh --web        # the four browser pictures only
#   tools/dev/screenshots.sh --keep       # leave the servers up afterwards, for a manual look
#
# Eight pictures, from four surfaces:
#
#   screen-playing.png       screen-idle-connect.png   screen-queue.png     the television
#   remote-browse.png        remote-now.png            remote-queue.png     the singer's remote
#   remote-offline.png                                                      the offline remote
#   package-builder-songs.png                                               km-package-builder
#
# Why a script and not a paragraph in the README: a screenshot is a build product with no build. Left
# to hand-driven capture these drift from the code within one release and nobody can tell which ones,
# because a stale picture looks exactly like a fresh one. This is the only thing that makes them
# checkable.
#
# **It needs a corpus, and most machines do not have one.** The pictures show a real catalog --
# real titles, real artists, a real lyric on screen -- because a remote listing "Song 1001 / Even
# Artist" advertises a demo rather than a product. That material is machine-local, so on a machine
# without it this degrades: it renders the display pictures from whatever song it is given into
# `<the cargo target dir>/screenshots`, and **never** writes
# docs/images. A clone that cannot reach the corpus must not be able to overwrite a published
# picture with a worse one.
#
# **Two rules about what the corpus may contain, both about the reader rather than the owner.**
#
# *Every song a picture SHOWS is in English*, because the README is. A television full of lyrics most
# of its readers cannot read demonstrates the wipe and nothing else. The catalog itself is **not**
# English-only, deliberately: a handful of songs in other languages are in it so the remote's
# language picker is a control with something to choose between rather than a select with one option,
# and `remote-browse.png` is captured with that filter applied (`?language=en`) so the picture shows it
# doing its job. Everything else keeps them out of frame by ordering -- the lists are sorted by
# title, so a non-English song wants a title that sorts past the visible rows.
#
# *And not the songs everybody has already heard.* This is curation and not code: nothing here
# filters by fame, and there is nothing it could read if it wanted to. But a catalog of the five
# most-covered songs on earth reads as a stock demo rather than as somebody's collection, which is
# the same objection that rules out "Song 1001 / Even Artist" one step further along. See the
# `What the README may show of a catalog` decision in docs/decisions/.
#
# **A published picture may not name anybody's folders.** km-package-builder prints the absolute path
# it is scanning across its own header, and that header is in `package-builder-songs.png` -- so the
# folder it scans is chosen by this script, below, and is not wherever KM_SHOT_DIR happens to point.
#
# **And a screenshot run does not touch the operator's recent-folder list.** It opens a real corpus
# through the real binary, which is where a `cfg(test)` guard reaches nothing, so the run is given a
# list of its own under KM_SHOT_DIR -- see KM_PACKAGE_BUILDER_RECENT where it is set below, and
# KM_PACKAGE_BUILDER_PASSWORDS beside it, which is the same rule over the file that holds a machine's
# admin password.
#
# Set KM_CORPUS to a folder of .kar/.mid files. Everything else has a default:
#
#   KM_CORPUS     folder of source songs                (no default -- without it, degraded mode)
#   KM_SONG       the song the television is playing    (default: a named one, then the first .kar)
#   KM_SHOT_DIR   scratch: package, data dirs, HTML     (default: a temp folder outside the repo)
#   KM_SONGS_DIR  where the songs are copied to         (default: a neutral path -- SEE ABOVE:
#                                                        whatever this is gets published in a picture)
#   KM_LANGUAGE   what an unnamed song is filed as      (default: en -- an ISO 639-1 code, not a
#                                                        name; the label the television draws is
#                                                        read off the song being drawn)
#   KM_CHROME     the browser binary                    (default: Chrome, then Edge)
#
# The scratch tree is deliberately **outside the repository**. `scratch/` is gitignored, but the
# corpus does not belong in a checkout and a gitignore is one `git add -f` away from not mattering.
#
# Ports are 8277/8278/8279, not the usual 8177/8178/8179, so a run cannot collide with a machine or a
# curation tool somebody already has open. See "Working in parallel" in CLAUDE.md.

set -euo pipefail

cd "$(dirname "$0")/../.."
REPO="$PWD"

# Sourced for `dist_target_dir` alone. Nothing here is staged for release, but the scratch output
# below belongs in the directory cargo is building into rather than in a `target/` conjured beside
# the checkout -- which is what a literal path produced once CARGO_TARGET_DIR pointed elsewhere, and
# it is a stray folder in a place `.gitignore` covers and nobody sweeps.
. tools/dist/common.sh
DIST_SCRIPT=screenshots

OUT="docs/images"
DEGRADED_OUT="$(dist_target_dir)/screenshots"
DO_DISPLAY=1
DO_WEB=1
KEEP=0

for arg in "$@"; do
  case "$arg" in
    --display) DO_WEB=0 ;;
    --web) DO_DISPLAY=0 ;;
    --keep) KEEP=1 ;;
    -h | --help)
      sed -n '3,64p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "screenshots: unknown argument $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

CORPUS="${KM_CORPUS:-}"
# What a song the index does not name gets filed as. An ISO 639-1 code, because that is what the
# manifest holds and what `?language=` matches -- **not** a display name. Two independent
# statements -- `--default-language pt` at the packaging step and the literal word "Portuguese" at
# the display call -- are each free to disagree with the other and with the song actually on screen,
# so the label is read off that song instead; see `language_name` below.
PACK_LANGUAGE="${KM_LANGUAGE:-en}"
SHOT_DIR="${KM_SHOT_DIR:-${TMPDIR:-${TEMP:-/tmp}}/km-shots}"
SHOT_DIR="${SHOT_DIR%/}"
# **This one is not under KM_SHOT_DIR, and that is the whole point of it.** km-package-builder prints
# the absolute path it is scanning across its own header, and that header is inside
# `package-builder-songs.png` -- so this path is published, and a published picture may not name
# anybody's folders. A `$SHOT_DIR/songs` would name whatever the operator set their scratch tree
# to; on Windows even the temp-folder default carries a user name in it.
#
# So the script chooses, rather than the operator: a standard shared account on Windows, /tmp
# elsewhere, neither of which says anything about whose machine this is. KM_SONGS_DIR overrides it
# for a box where neither is writable -- and whatever is put there is what the picture will read.
#
# Spelled the way the *operating system* spells it rather than the way this shell does: everything
# downstream of here -- km-pack, km-package-builder -- is a native binary, and a Git Bash `/c/...`
# path is not something a Windows program can open.
case "$(uname -s 2>/dev/null)" in
  MINGW* | MSYS* | CYGWIN*) SONGS_DEFAULT="C:/Users/Public/karaoke/songs" ;;
  *) SONGS_DEFAULT="/tmp/karaoke/songs" ;;
esac
SONGS="${KM_SONGS_DIR:-$SONGS_DEFAULT}"
SONGS="${SONGS%/}"
MACHINE_DIR="$SHOT_DIR/machine"
OFFLINE_DIR="$SHOT_DIR/offline"
HTML_DIR="$SHOT_DIR/html"
PROFILE="$SHOT_DIR/chrome"
# The third server's version of the two `--data-dir`s above. km-package-builder's recent-folder list
# is twelve entries of somebody's own curation, in their config directory, newest first -- and it is
# deliberately never pruned of folders that have gone away, so anything that gets in stays. A run
# that remembered `$SONGS` would evict a real corpus and then offer, on the next startup, a folder
# this script deletes on its way out. There is no flag for it; the variable is it.
PKGBUILD_RECENT="$SHOT_DIR/km-package-builder-recent.json"
# The same rule one file over, and the stricter half of it: this run never signs in to a machine,
# but a store of admin passwords is not something a screenshot run should be able to reach at all.
PKGBUILD_PASSWORDS="$SHOT_DIR/km-package-builder-passwords.json"
# ...and the third file the same rule reaches, for a reason of its own: that program's settings hold
# which language it draws its pages in, so a run reading the owner's would publish a picture in
# whatever language they curate in. Pinned to English, which is what the published pictures are.
PKGBUILD_SETTINGS="$SHOT_DIR/km-package-builder-settings.json"

API="http://127.0.0.1:8277/api/v1"
REMOTE="http://127.0.0.1:8277"
PKGBUILD="http://127.0.0.1:8278"
OFFLINE="http://127.0.0.1:8279"

FAILURES=0
PIDS=""

note() { printf '  %s\n' "$*"; }
warn() {
  printf 'screenshots: %s\n' "$*" >&2
  FAILURES=$((FAILURES + 1))
}

# Every server this starts is killed on the way out, including on Ctrl-C. Three servers and a browser
# profile is exactly the sort of thing that gets left running for a week.
#
# The copied songs go with them. `$SONGS` is somewhere shared by construction -- see where it is set
# -- and somebody's own collection should not be left sitting in a folder every account on the box
# can read. `--keep` keeps it, because the servers it keeps up are serving it.
cleanup() {
  [ "$KEEP" = 1 ] && {
    printf '\nleft running (--keep): %s  %s  %s\n' "$REMOTE" "$PKGBUILD" "$OFFLINE"
    printf 'left in place (--keep): %s\n' "$SONGS"
    return
  }
  for pid in $PIDS; do kill "$pid" 2>/dev/null || true; done
  # A moment for them to let go first. km-package-builder holds its database open inside the folder
  # about to be deleted, and on Windows an open handle makes the file undeletable rather than merely
  # busy -- so without this the run ends in three `Device or resource busy` lines every time.
  if [ -n "${SONGS_STAGED:-}" ]; then
    for _ in 1 2 3 4 5; do
      rm -rf "$SONGS" 2>/dev/null && break
      sleep 1
    done
    [ -d "$SONGS" ] && printf 'screenshots: could not remove %s; delete it by hand\n' "$SONGS" >&2
  fi
  return 0
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------- the browser

find_chrome() {
  if [ -n "${KM_CHROME:-}" ]; then
    printf '%s' "$KM_CHROME"
    return 0
  fi
  for candidate in \
    "/c/Program Files/Google/Chrome/Application/chrome.exe" \
    "/c/Program Files (x86)/Google/Chrome/Application/chrome.exe" \
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"; do
    [ -x "$candidate" ] && {
      printf '%s' "$candidate"
      return 0
    }
  done
  for name in google-chrome chromium chromium-browser microsoft-edge; do
    command -v "$name" >/dev/null 2>&1 && {
      command -v "$name"
      return 0
    }
  done
  return 1
}

# shot <out.png> <css-width> <css-height> <scale> <url>
#
# Two of these flags each fix a specific way this silently produces nothing:
#
#   --user-data-dir  without it, an already-running Chrome adopts the invocation, and the headless
#                    process exits 0 having written no file at all. The commonest failure on Windows.
#   --run-all-compositor-stages-before-draw
#                    or the tab bar and star transitions are caught mid-animation.
#
# There is deliberately **no --blink-settings=preferredColorScheme flag**. One pinned to 0 answers
# a single surface: headless defaults to light, so a page leading with dark comes out in a palette
# the product does not lead with.
#
# The two browser surfaces disagree, and that is precisely why no flag belongs here. The remote is
# pinned light (see the head of km-remote-pages/static/app.css) and km-package-builder is pinned dark (the
# head of its own style.css), each for its own reason, so there is no one value this flag could take
# that would be right for both -- which was always the objection to it: setting a color scheme for
# every page from one place asserts they agree, and each page already knows its own. Nothing is lost
# by leaving it out, because the flag drives `prefers-color-scheme` and neither stylesheet has such a
# branch. Each declares `color-scheme` itself, which is what the UA-drawn scrollbars and `<select>`
# follow, so headless's own default reaches nothing either page cares about.
#
# Note the width floor: headless Chrome will not give a viewport narrower than 500 CSS px, so asking
# for 390 silently lays the page out at 500 and crops it, which looks like a broken stylesheet. The
# phone pictures are therefore taken at 500. That is not a meaningful infidelity here -- the only
# thing below km-remote-pages's 27rem breakpoint is the result count.
#
# `out` is made absolute before it is handed over: Chrome resolves a relative --screenshot path
# against something other than this shell's working directory and then writes nothing, silently,
# exiting 0. Every capture failed exactly once for this reason.
shot() {
  out="$1" w="$2" h="$3" scale="$4" url="$5"
  case "$out" in
    /* | ?:*) ;;
    *) out="$REPO/$out" ;;
  esac
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars \
    --window-size="$w,$h" --force-device-scale-factor="$scale" \
    --virtual-time-budget=4000 --run-all-compositor-stages-before-draw \
    --user-data-dir="$PROFILE" --no-first-run --no-default-browser-check \
    --screenshot="$out" "$url" >/dev/null 2>&1 || true
  if [ -s "$out" ]; then
    note "wrote $out"
  else
    warn "could not capture $url -> $out (capture it by hand, ${w}x${h})"
  fi
}

# freeze <name> <url> <base> [curl args...]
#
# Fetches a page and screenshots the copy rather than the URL, because **the remote holds an
# EventSource open forever** (`static/live.js`) and headless Chrome therefore never decides the page
# has finished loading -- it hangs until killed, having written nothing. `--virtual-time-budget` does
# not save it: the stream is a real socket, not virtual time.
#
# Dropping that one script tag is safe and is not a fake: the server renders the whole page, and SSE
# only patches it afterwards. What is captured is exactly what a phone is sent. A <base> is injected
# so the stylesheet and icons still load from the server.
#
# km-package-builder has no SSE, so it is captured straight from its URL.
freeze() {
  name="$1" url="$2" base="$3"
  shift 3
  raw="$HTML_DIR/$name.raw.html"
  page="$HTML_DIR/$name.html"
  # **`Accept-Language: en` on every capture**, for the same reason `machine.locale` is pinned where
  # the settings file is written: a page is drawn in the language the request asked for, `curl`
  # inherits none from the person running this, and English by luck is not the same as English by
  # instruction. One header against a picture nobody would notice was wrong until it was published.
  curl -sS -H 'Accept-Language: en' "$@" "$url" -o "$raw" || {
    warn "could not fetch $url"
    return 1
  }
  # `live.js[^"]*` rather than the bare name, because the tag carries a cache-busting stamp --
  # `/static/live.js?v=26bd7aab`, from `chrome.assets` in km-remote-pages' layout.html. The exact
  # match had silently stopped matching, and the four browser pictures were being taken with the
  # script still in the page: they came out right only because `live.js` stopped opening its
  # connection while the page is not on screen, which a headless capture never is. That is luck
  # holding the door, not the mechanism this line exists to be.
  sed -e 's|<script src="/static/live.js[^"]*" defer></script>||' \
    -e "s|<head>|<head><base href=\"$base\">|" "$raw" >"$page"
  printf '%s' "$page"
}

api() { curl -sS "$@"; }
api_post() { curl -sS -X POST "$API$1" -H 'content-type: application/json' -d "${2:-}" >/dev/null; }

wait_for() {
  for _ in $(seq 1 100); do
    curl -sf "$1" >/dev/null 2>&1 && return 0
    sleep 0.3
  done
  return 1
}

# ------------------------------------------------------------------ the songs

if [ -z "$CORPUS" ]; then
  echo "screenshots: KM_CORPUS is not set, so there is no catalog to photograph."
  echo "             Rendering the display pictures only, into $DEGRADED_OUT."
  echo "             docs/images is left alone: a picture of a test fixture is worse than a stale one."
  echo
  [ -z "${KM_SONG:-}" ] && {
    echo "screenshots: set KM_SONG to a .kar/.mid file with lyrics, or KM_CORPUS to a folder." >&2
    exit 1
  }
  mkdir -p "$DEGRADED_OUT"
  # No language is claimed, deliberately. There is no catalog here to read one off, and KM_LANGUAGE
  # holds a code rather than a name, so passing it would paint "en" across a television. The
  # example's language is an Option precisely so a caller who does not know can say so.
  cargo run -q -p km-display --example screenshots -- "$KM_SONG" "$DEGRADED_OUT" ""
  exit 0
fi

[ -d "$CORPUS" ] || {
  echo "screenshots: KM_CORPUS is $CORPUS, which is not a directory" >&2
  exit 1
}

mkdir -p "$SONGS" "$MACHINE_DIR/packages" "$OFFLINE_DIR" "$HTML_DIR" "$PROFILE" "$OUT"

echo "screenshots"
note "corpus     $CORPUS"
note "scratch    $SHOT_DIR"
note "out        $OUT"
echo

# Copied rather than curated in place: `km-package-builder --init` writes its database into the
# folder it scans, and the corpus is not ours to write into. It also puts the path
# km-package-builder prints in its header somewhere neutral, which matters because that header is in
# a published picture -- see where $SONGS is set, which is the whole of how that is arranged.
#
# **Cleared first, and that is not tidiness.** `mkdir -p` with no wipe meant a run *added* to
# whatever an earlier one left, so a scratch folder drifted to more files than the corpus held and
# every published song number was off by an amount nothing recorded. It matters more now that the
# catalog is English: a leftover from an older corpus is a song in a published picture.
#
# The clear is scoped rather than a `rm -rf`, because this deletes files in a folder somebody could
# have pointed anywhere: songs by extension at depth 1, and km-package-builder's own database, which
# would otherwise keep rows for files that are no longer on disk and publish them. It also refuses
# outright when $SONGS is the corpus itself, which is the one mistake that would delete the thing
# being photographed.
if [ "$(cd "$SONGS" 2>/dev/null && pwd -P)" = "$(cd "$CORPUS" && pwd -P)" ]; then
  echo "screenshots: the staging folder is the corpus itself; refusing to clear it." >&2
  echo "             Unset KM_SONGS_DIR, or point it somewhere that is not KM_CORPUS." >&2
  exit 1
fi

echo "copying the source songs"
note "staging    $SONGS"
mkdir -p "$SONGS"
SONGS_STAGED=1
find "$SONGS" -maxdepth 1 -type f \
  \( -iname '*.kar' -o -iname '*.mid' -o -iname '*.midi' -o -iname '*.kmbuild*' \) -delete
find "$CORPUS" -maxdepth 1 -type f \( -iname '*.kar' -o -iname '*.mid' -o -iname '*.midi' \) \
  -exec cp {} "$SONGS/" \;
note "$(find "$SONGS" -type f | wc -l | tr -d ' ') file(s)"

# --index is what turns a folder of filenames into a catalog that reads like one. Most karaoke
# MIDIs carry a title only as the file name, and a browse screenshot full of
# `Barao_Vermelho-Por_Voce` shows the tool having failed rather than the product working. If the
# corpus has an index.csv beside it, it is used.
#
# It is also the only trustworthy statement of what language a song is in. `@LENGL` is the Soft
# Karaoke editor's *default* rather than an assertion -- most non-English .kar files declare it, see
# crates/song/km-kmpkg/src/language.rs -- so nothing here may infer a language from a file. Which is
# why a missing index now says so out loud instead of degrading in silence: without one the browse
# list reads as file names, the artist column is whatever the file happened to carry, and the
# catalog's languages are whatever the editors left behind.
INDEX_ARGS=""
[ -f "$CORPUS/../index.csv" ] && INDEX_ARGS="--index $CORPUS/../index.csv"
[ -f "$SHOT_DIR/index.csv" ] && INDEX_ARGS="--index $SHOT_DIR/index.csv"
[ -n "$INDEX_ARGS" ] ||
  warn "no index.csv beside the corpus or in the scratch tree: titles will read as file names,
        artists will be whatever the files carry, and no song's language is stated by anybody"

# Two steps since `km-pack build` stopped taking a folder: describe what is there, then build the
# description. The description is written into the scratch tree rather than beside the songs, so a
# run leaves the picture corpus exactly as it found it.
echo "describing the demo songs"
# **`--start-number 1`, and the pictures still say 1019.** A song number is `bank * 1000 + slot`
# rather than a flat serial: a package numbers its own songs 1 to 999 and the machine supplies the
# thousands, which `package_banks` pins to 1 below. So the 1 is written out rather than left to the
# default, because the number it is *not* is the thing worth seeing here -- `--start-number 1001`
# is refused with `1001 is not in 1..=999`. The index's `number` column is a slot for the same
# reason: 19, not 1019.
# shellcheck disable=SC2086
cargo run -q -p km-pack -- spec "$SONGS" \
  --out "$SHOT_DIR/favorites.kmspec.yaml" \
  --name "Favorites" \
  --start-number 1 --default-language "$PACK_LANGUAGE" --require-lyrics $INDEX_ARGS

# The id the description was given, read back out of it. A build generates one and refuses a typed
# one -- see `A package says nothing about the machine that built it` in docs/decisions/packaging.md
# -- so the id is sixteen random hexadecimal characters and is different every run. The bank below
# is keyed on it, and a bank keyed on a name nobody issues would pin nothing.
PACKAGE_ID=$(sed -n 's/^  id: //p' "$SHOT_DIR/favorites.kmspec.yaml" | head -1)
case "$PACKAGE_ID" in
  [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *)
    echo "screenshots: no package id in $SHOT_DIR/favorites.kmspec.yaml (got \"$PACKAGE_ID\")" >&2
    exit 1
    ;;
esac

echo "building the demo package"
# The folder holds this run's package and nothing else. A package left by an earlier run is a
# second copy of the same catalog at best, and at worst a build the current manifest can no longer
# read -- which the machine reports on the remote's own page as a red banner across the top of
# `remote-browse.png`. Scoped to the extension so a stray file is left where somebody put it.
find "$MACHINE_DIR/packages" -maxdepth 1 -type f -iname '*.kmpkg' -delete
cargo run -q -p km-pack -- build "$SHOT_DIR/favorites.kmspec.yaml" \
  --out "$MACHINE_DIR/packages/favorites.kmpkg"

# ---------------------------------------------------------------- the machine

# Written rather than defaulted, for three reasons the pictures depend on: a port that cannot collide
# with a machine somebody already has running; mDNS off, so a screenshot run does not advertise
# itself to the house; and the SoundFont named absolutely, because `--data-dir` moves the asset
# directory with it and the engine would otherwise fall back to a test tone.
#
# **`locale` is pinned rather than defaulted**, and it is the one line here guarding against the
# person running the script rather than against the machine. The README these pictures go into is in
# English — see `What the README may show of a catalog` — so a developer whose own machine speaks
# Portuguese would otherwise republish Portuguese screenshots into it, and a stale picture looks
# exactly like a fresh one. `freeze` sends `Accept-Language: en` for the browser captures.
#
# **`package_banks` puts the demo songs in bank 1, so the pictures say 1019.** A song number is
# `bank * 1000 + slot`: the index's `number` column is the slot (19), and the machine supplies the
# thousands. Nothing about a package names a bank -- it comes from the id, which is generated, so
# an unpinned bank is a different thousand every run and `screenshots.rs` would draw 1019 while the
# machine dialled whatever this run's id hashed to. This is the machine's own reservation, which
# `Machine::ensure_bank` reads before anything else, so it is the same act as an owner moving a
# package with `PUT /api/v1/packages/{id}/bank` and needs no password to perform. `library.sqlite`
# is deleted below, so bank 1 is always free to claim.
cat >"$MACHINE_DIR/settings.json" <<JSON
{
  "api": { "bind": "127.0.0.1:8277", "advertise_mdns": false },
  "audio": { "soundfont": "$REPO/assets/soundfont/GeneralUser-GS.sf2", "music_volume": 0.0 },
  "display": { "enabled": false },
  "machine": { "name": "KaraokeMachine", "locale": "en" },
  "package_banks": { "$PACKAGE_ID": 1 }
}
JSON

# A stale catalog would keep song numbers from a previous run and the pictures would disagree
# with each other.
rm -f "$MACHINE_DIR/library.sqlite"

echo "starting the machine (headless, :8277)"
cargo run -q -p karaokemachine -- --headless --data-dir "$MACHINE_DIR" \
  >"$SHOT_DIR/machine.log" 2>&1 &
PIDS="$PIDS $!"
wait_for "$API/discover" || {
  echo "screenshots: the machine never answered; see $SHOT_DIR/machine.log" >&2
  exit 1
}
SONG_COUNT=$(api "$API/discover" | tr ',' '\n' | sed -n 's/.*"song_count":\([0-9]*\).*/\1/p')
note "$SONG_COUNT songs"

# Every song the machine has filed as English, lowest number first. One request, doing two jobs: it
# is the membership test that says a song named below really is English -- because a picture may not
# show one that is not -- and it is the pool a name falls back into when the corpus is edited under
# it. `?language=` is an exact match on the catalog's own column, which came from index.csv, which
# is the only thing here that actually knows.
#
# A song number is a `SongCode`, and it crosses the wire as a **string** -- `"number":"1001"`. Every
# scrape and every POST below spells the quotes, and that is not pedantry: the pattern here used to
# read `"number":\([0-9]*\)`, which silently matched the empty string once codes stopped being
# integers, and the queue was posted as `{"number":1019}`, which the API refuses with a 400 that
# nothing looked at. A run then photographed an empty queue and said nothing.
#
# **The pattern below allows digits only**, because a code is digits only. Allowing letters is the
# same class of bug pointing the other way, and it is said here rather than quietly narrowed: a
# pattern that matches more than the wire can hold is a pattern that will one day match nothing and
# report success.
#
# Flattened to one space-separated line, because the membership test below is a `case` glob over
# `" $ENGLISH "` and a list separated by newlines has no spaces in it to match against.
ENGLISH=$(api "$API/songs?language=en&sort=number&limit=500" |
  tr ',' '\n' | sed -n 's/.*"number":"\([0-9][0-9]*\)".*/\1/p' | tr '\n' ' ')
if [ -z "$ENGLISH" ]; then
  echo "screenshots: nothing in the catalog is filed as English. Check the index.csv's" >&2
  echo "             language column; there is nothing here worth photographing." >&2
  exit 1
fi
# Not fatal, but it silently costs two pictures a feature: km-remote-pages draws the Language picker only
# where there is more than one language to pick between, so an English-only catalog publishes a
# browse page with the control missing and README.md's alt text stops being true. Counted against
# the catalog total rather than asked for -- there is no languages endpoint on the API, and every
# song has a language here because `--default-language` gave one to any the index did not name.
ENGLISH_COUNT=$(printf '%s\n' $ENGLISH | grep -c . || true)
note "$ENGLISH_COUNT of $SONG_COUNT songs are English"
[ "$ENGLISH_COUNT" != "$SONG_COUNT" ] ||
  warn "every song is English, so the Language picker will not be drawn at all --
        the corpus wants a few songs in other languages"

CHOSEN=""

# Named, not derived: two runs a month apart must produce the same pictures, and "whatever sorted
# first" is not a promise -- adding one song to the corpus would repaint six of the eight. Terms are
# %20-encoded by hand because this script deliberately depends on no jq and no python.
english_song() { case " $ENGLISH " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

first_unused() {
  for candidate in $ENGLISH; do
    case " $CHOSEN " in *" $candidate "*) continue ;; esac
    printf '%s' "$candidate"
    return 0
  done
  return 1
}

number_for() {
  api "$API/songs?q=$1&limit=1" |
    sed -n 's/.*"number":"\([0-9][0-9]*\)".*/\1/p' | head -1
}

# pick <term> <what it is, for the message>
#
# A miss must not be silent: a run that quietly photographs five queued songs where six were asked
# for looks exactly like a run that worked, which is the wrong trade for a published picture. The
# fallback is the lowest-numbered English song nothing else has claimed, so an edited corpus degrades
# to a different but valid picture with a message, rather than to a blank one without.
pick() {
  n=$(number_for "$1")
  if [ -z "$n" ]; then
    n=$(first_unused || true)
    warn "nothing matches \"$1\" ($2); falling back to #${n:-nothing}"
  elif ! english_song "$n"; then
    # The one thing the fallback cannot rescue: a term that matched, but matched a song in another
    # language. Every song in a published picture is English.
    warn "\"$1\" ($2) matched #$n, which is not filed as English"
  fi
  [ -n "$n" ] && CHOSEN="$CHOSEN $n"
  printf '%s' "$n"
}

# The song on the television *and* on the remote's Now page -- one song, rather than two different
# ones with nothing saying so. Named twice because it is reached two ways: by search, for
# the machine, and by file name, for the display example, which is handed a path and never a number.
HERO_FILE='Dire_Straits-Sultans_of_Swing.kar'
PLAYING=$(pick 'sultans%20of%20swing' 'the song playing')

# `term:singer`, six of them, because README.md's alt text for remote-queue.png says six. One has no
# singer on purpose: an anonymous queue entry looks different from a named one and both happen at a
# party, so a picture showing only one of the two is showing half the feature.
QUEUE_TERMS='superstition:Ana
fields%20of%20gold:Beto
englishman%20in%20new%20york:
devil%20went%20down:Marina
raspberry%20beret:Nina
save%20tonight:Ana'

echo "setting up a party"
# Guarded, unlike before: an empty $PLAYING posted `{"number":,"singer":"Nina"}` -- malformed JSON,
# which api_post discards along with its output and no status check, so nothing played and the only
# sign was remote-now.png quietly coming back as the nothing-is-playing page.
if [ -n "$PLAYING" ]; then
  api_post /queue "{\"number\":\"$PLAYING\",\"singer\":\"Nina\"}"
else
  warn "no song to play; remote-now.png will show the idle page"
fi

# A here-document rather than a pipe into `while`: a pipeline would run the loop in a subshell, and
# every `warn` inside it -- and therefore the exit code this script is judged by -- would be thrown
# away with it.
QUEUED=""
while IFS= read -r entry; do
  [ -n "$entry" ] || continue
  term="${entry%%:*}"
  who="${entry#*:}"
  n=$(pick "$term" 'a queued song')
  [ -n "$n" ] || continue
  QUEUED="$QUEUED $n"
  if [ -n "$who" ]; then
    api_post /queue "{\"number\":\"$n\",\"singer\":\"$who\"}"
  else
    api_post /queue "{\"number\":\"$n\"}"
  fi
done <<QUEUE_LIST
$QUEUE_TERMS
QUEUE_LIST

# Seek then **pause**, on purpose. A running position makes every regeneration a different picture of
# the same thing, and a paused machine is silent -- this runs on somebody's desk. The volume is set
# after the pause so the slider reads like a machine in use without anything being heard.
api_post /transport/seek '{"ms":72000}'
api_post /transport/pause
curl -sS -X PUT "$API/settings" -H 'content-type: application/json' \
  -d '{"music_volume":0.8}' >/dev/null
curl -sS -X PUT "$API/settings" -H 'content-type: application/json' \
  -d '{"transpose":-2}' >/dev/null
note "playing #$PLAYING, paused at 1:12, key -2"

# ------------------------------------------------------- the display pictures

if [ "$DO_DISPLAY" = 1 ]; then
  echo
  echo "rendering the display pictures"
  # The same song the machine is playing, so `screen-playing.png` and `remote-now.png` are two
  # pictures of one evening rather than of two. Named by file, because this runs before -- and
  # without -- the catalog in `--display` mode; the fallback is what it always was, but it now
  # says so, since falling back publishes an arbitrary song under a language read off another one.
  SONG="${KM_SONG:-}"
  if [ -z "$SONG" ]; then
    SONG=$(find "$SONGS" -type f -iname "$HERO_FILE" | head -1)
  fi
  if [ -z "$SONG" ]; then
    SONG=$(find "$SONGS" -type f -iname '*.kar' | head -1)
    [ -z "$SONG" ] || warn "$HERO_FILE is not in the corpus; the television will show $SONG instead"
  fi
  if [ -n "$SONG" ]; then
    # The idle picture states how much is installed, and it states the truth: these come off the
    # machine this script has just loaded, not out of the air. Only the LAN addresses and the queue
    # are fabricated -- see `What the README may show of a catalog` in docs/decisions/repository.md. Scraped with
    # sed, like `number_for` above, because this script deliberately depends on no jq.
    PACKAGES_JSON=$(api "$API/packages")
    # `],"song_count":N` -- the envelope's own total, which serde writes straight after the packages
    # array. Anchored on the `]` because every package object carries a `song_count` of its own, and
    # an unanchored match would pick one of those instead.
    SONG_TOTAL=$(printf '%s' "$PACKAGES_JSON" | sed -n 's/.*\],"song_count":\([0-9]*\).*/\1/p')
    # `installed_at` appears once per package and nowhere else in this response.
    PACKAGE_TOTAL=$(printf '%s' "$PACKAGES_JSON" | grep -o '"installed_at"' | wc -l | tr -d ' ')
    COUNTS=""
    if [ -n "$SONG_TOTAL" ] && [ -n "$PACKAGE_TOTAL" ]; then
      COUNTS="$SONG_TOTAL,$PACKAGE_TOTAL"
      note "the idle picture will say $SONG_TOTAL song(s) in $PACKAGE_TOTAL package(s)"
    else
      # Not fatal: the example draws no summary line at all rather than a number nobody counted.
      warn "could not read the catalog size; the idle picture will show no summary"
    fi
    # The language *name* on the television, taken from the song being drawn rather than from the
    # package default beside it. They are different questions -- the default is what an unclassified
    # song is filed as, this is what *this* song is sung in -- and until now the second was a literal
    # word in this file, free to say Portuguese under an English title with nothing to catch it.
    # `SongInfo::language` is a name and not a tag so that km-display needs no km-kmpkg dependency
    # (examples/screenshots.rs); the case below is the caller's half of that bargain.
    language_name() {
      case "$1" in
        en) printf 'English' ;;
        pt) printf 'Portuguese' ;;
        es) printf 'Spanish' ;;
        it) printf 'Italian' ;;
        fr) printf 'French' ;;
        de) printf 'German' ;;
        ja) printf 'Japanese' ;;
        *) return 1 ;;
      esac
    }
    # The title comes off the catalog for the same reason, and it is not cosmetic: real karaoke
    # MIDIs carry `SULTANS OF SWING` in their meta as often as a readable name, and every other
    # published picture shows the corrected title from the index. Left to the file, two pictures of
    # one song disagree about what it is called.
    SONG_LANGUAGE=""
    SONG_TITLE=""
    if [ -n "$PLAYING" ]; then
      PLAYING_JSON=$(api "$API/songs/$PLAYING")
      PLAYING_LANGUAGE=$(printf '%s' "$PLAYING_JSON" |
        sed -n 's/.*"language":"\([a-z][a-z]*\)".*/\1/p' | head -1)
      SONG_TITLE=$(printf '%s' "$PLAYING_JSON" | sed -n 's/.*"title":"\([^"]*\)".*/\1/p' | head -1)
      SONG_LANGUAGE=$(language_name "$PLAYING_LANGUAGE" || true)
      # Not fatal, and the same choice the counts above make: the example draws no language line at
      # all rather than one nobody stood behind.
      [ -n "$SONG_LANGUAGE" ] ||
        warn "#$PLAYING is filed as ${PLAYING_LANGUAGE:-nothing}, which this script has no name
              for; the playing picture will show no language"
    fi
    # examples/screenshots.rs draws 1019 on the television, and index.csv pins the playing song to
    # that number so the two agree. Asserted rather than assumed: if they part company, one published
    # picture gives a number to a song that another published picture gives to a different one.
    [ -z "$PLAYING" ] || [ "$PLAYING" = 1019 ] ||
      warn "the song playing is #$PLAYING but the display example draws 1019 -- pin it in index.csv
            or change the example"
    cargo run -q -p km-display --example screenshots -- \
      "$SONG" "$OUT" "$SONG_LANGUAGE" "$COUNTS" "$SONG_TITLE" ||
      warn "the display pictures failed"
  else
    warn "no song to render the display pictures from"
  fi
fi

# ----------------------------------------------------------- the web pictures

if [ "$DO_WEB" = 1 ]; then
  echo
  if ! CHROME=$(find_chrome); then
    warn "no Chrome or Edge found; skipping the four browser pictures. Set KM_CHROME."
    echo "  they would have been: $REMOTE/  $REMOTE/now  $REMOTE/queue  $OFFLINE/  $PKGBUILD/songs"
  else
    note "browser    $CHROME"

    # The offline remote's two databases go the way of the machine's catalog above, and for one
    # more reason each. `catalog.sqlite` holds song numbers from the previous run and is refreshed
    # only when the machine's `catalog_version` says to, which a machine whose library was just
    # deleted cannot be relied on to say. `favorites.sqlite` is worse, because it fails *silently*:
    # the create-a-folder route files its song with `toggle`, so the three POSTs below **un-star**
    # what a previous run starred, and `remote-offline.png` came back with and without its stars on
    # alternate runs. Nothing in the picture says which one you got.
    rm -f "$OFFLINE_DIR"/catalog.sqlite* "$OFFLINE_DIR"/favorites.sqlite*

    echo "starting the offline remote (:8279)"
    cargo run -q -p km-remote -- --machine 127.0.0.1:8277 \
      --data-dir "$OFFLINE_DIR" --port 8279 >"$SHOT_DIR/offline.log" 2>&1 &
    PIDS="$PIDS $!"
    wait_for "$OFFLINE/" || warn "the offline remote never answered"

    # A star on a row is the whole point of the offline remote, so a few are filed. `create_folder`
    # takes a name and a song together and is idempotent by name, so no folder id has to be found.
    # Read off $QUEUED rather than out of three variables, so a seventh song in the party list needs
    # no fourth name here.
    for n in $(printf '%s\n' $QUEUED | head -2) $PLAYING; do
      curl -sS -X POST "$OFFLINE/favorites/folders" --data "name=Party&song=$n" >/dev/null || true
    done

    echo "starting km-package-builder (:8278)"
    printf '{"locale":"en"}' >"$PKGBUILD_SETTINGS"
    KM_PACKAGE_BUILDER_RECENT="$PKGBUILD_RECENT" \
      KM_PACKAGE_BUILDER_PASSWORDS="$PKGBUILD_PASSWORDS" \
      KM_PACKAGE_BUILDER_SETTINGS="$PKGBUILD_SETTINGS" \
      cargo run -q -p km-package-builder -- "$SONGS" --init --scan --port 8278 \
      --machine "$REMOTE" >"$SHOT_DIR/package-builder.log" 2>&1 &
    PIDS="$PIDS $!"
    wait_for "$PKGBUILD/songs" || warn "km-package-builder never answered"
    sleep 12 # the scan is asynchronous; an empty table is not worth photographing

    echo "capturing"
    # 500x540 for all four phone pictures so they line up in the README table. See the width floor above.
    #
    # **`?language=en` on the two list pages, and it is doing two jobs at once.** It guarantees that
    # every row above the fold is English -- the catalog is not English-only, and a curated corpus
    # plus alphabetical order is luck rather than a mechanism -- and it makes the picture show the
    # Language picker *in use*, reading English, instead of sitting on All demonstrating nothing.
    # The parameter is the page's own (`lang`, crates/remote/km-remote-pages/src/handlers.rs), applied server-side.
    p=$(freeze browse "$REMOTE/?language=en" "$REMOTE/") &&
      shot "$OUT/remote-browse.png" 500 540 2 "file:///$p"
    p=$(freeze now "$REMOTE/now" "$REMOTE/") && shot "$OUT/remote-now.png" 500 540 2 "file:///$p"
    p=$(freeze queue "$REMOTE/queue" "$REMOTE/" -H 'Cookie: km_singer=Nina') &&
      shot "$OUT/remote-queue.png" 500 540 2 "file:///$p"
    # `remote-offline.png` is the offline remote's own songs list, and every difference from
    # `remote-browse.png` beside it is the point: a favorites mode where the machine's remote offers
    # the printed Book, the A–Z picker, and a star on every row. Both remotes carry three mode
    # buttons, so the count is not the difference; **the A–Z is a dropdown rather than a strip of
    # letters**, which is what the alt text in README.md and site/index.html calls it.
    # **The stars in frame are empty and that is expected** -- the three
    # songs starred just above are the party's, and they sort well past the visible rows of an
    # alphabetical list. What the picture shows is the star *column*, which the machine's own remote
    # does not have at all; a filled one would mean filtering the list to find it, which would make
    # this a picture of a search instead of a picture of the catalog.
    # `?language=en` for the same two reasons the other list picture takes it: it guarantees English
    # above the fold, and it shows the language picker reading something rather than sitting on All.
    p=$(freeze offline "$OFFLINE/?language=en" "$OFFLINE/") &&
      shot "$OUT/remote-offline.png" 500 540 2 "file:///$p"
    shot "$OUT/package-builder-songs.png" 1440 900 1 "$PKGBUILD/songs"
  fi
fi

# ----------------------------------------------------------------- the report

# A PNG's width and height are big-endian u32s at offset 16, right after the IHDR tag. Worth
# checking rather than trusting: the failure this catches is a page laid out at the wrong width and
# then cropped, which produces a perfectly valid file of the right name and the wrong picture. It is
# how the phone captures were found to be coming out cropped at 390 when Chrome had used 500.
png_size() {
  od -An -tu1 -j16 -N8 "$1" |
    awk '{printf "%dx%d", $1*16777216+$2*65536+$3*256+$4, $5*16777216+$6*65536+$7*256+$8}'
}

expected_size() {
  case "$(basename "$1")" in
    screen-*) printf '1920x1080' ;;
    package-builder-*) printf '1440x900' ;;
    remote-*) printf '1000x1080' ;;
  esac
}

echo
echo "produced"
total=0
for f in "$OUT"/screen-*.png "$OUT"/remote-*.png "$OUT"/package-builder-*.png; do
  [ -f "$f" ] || continue
  got=$(png_size "$f")
  want=$(expected_size "$f")
  [ -n "$want" ] && [ "$got" != "$want" ] &&
    warn "$(basename "$f") is ${got}, expected ${want}"
  bytes=$(wc -c <"$f" | tr -d ' ')
  total=$((total + bytes))
  kb=$((bytes / 1024))
  flag=""
  # 1200 KB, measured rather than guessed, and **raised from 600 when the television pictures moved
  # off the generated gradient onto the shipped photographs**. A smooth gradient is the easiest thing
  # a PNG ever compresses and the playing picture landed near 460 KB; a starfield does not deflate at
  # all, and the same picture is now near 1105 KB with the idle one at 1043. That is the cost of the
  # README showing the background the product actually ships, and it was paid deliberately -- see
  # `Which wallpaper the published pictures are taken over`. The browser pictures are unmoved near
  # 100. The hero's full size is still worth paying: GitHub renders a README column about 830 px
  # wide, so 1920 is what keeps it sharp on a high-DPI screen. Anything meaningfully past this is a
  # new problem, not this one.
  [ "$kb" -gt 1200 ] && flag="   <-- unexpectedly large"
  printf '  %-34s %5s KB%s\n' "$(basename "$f")" "$kb" "$flag"
done
printf '  %-34s %5s KB\n' "TOTAL" "$((total / 1024))"
# 3 MB, up from 2 for the same reason as the per-file figure above. A run lands near 2884 KB, so this
# leaves a little room and still says something if a picture doubles.
[ "$((total / 1024))" -gt 3072 ] &&
  echo "  note: over the 3 MB total budget -- every regeneration rewrites all of these."

if [ "$FAILURES" -gt 0 ]; then
  echo
  echo "screenshots: $FAILURES step(s) did not produce a picture (see above)" >&2
  exit 1
fi
