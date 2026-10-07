#!/usr/bin/env bash
#
# Regenerates the two pictures of putting words on a MIDI file that the manual publishes.
#
#   tools/dev/sync-pictures.sh              # both
#   tools/dev/sync-pictures.sh --editor     # the editor's picture only: no server, no browser
#   tools/dev/sync-pictures.sh --page       # the page's picture only
#
#   song-sync-page.png        the song sync page, with a song selected and its words in the box
#   sync-editor-tapping.png   the lyric sync editor, part-way through tapping a line
#
# **Both show a carol from the released pack, and neither needs a corpus.** The page's box holds a
# whole song's words and the editor shows three lines of them, so the song must be one whose words
# anybody may publish. tools/dev/carol-pack.sh fetches the pack, and KM_CAROLS names another one.
#
# The editor's picture is drawn with no window and no sound, by `examples/sync_picture.rs` in the
# machine's crate. The page's picture is taken by a headless browser from the running program.
#
# **A published picture may not name anybody's folders.** The page prints the folder it lists, so
# this script chooses that folder, and the scratch tree is not where the songs go.
#
#   KM_CAROLS_DIR  where the carols are written   (default: a neutral path. Whatever this is gets
#                                                  published in a picture)
#   KM_CHROME      the browser binary             (default: Chrome, then Edge)
#
# **A run does not touch the operator's own settings.** The page remembers the folder it was on
# last, so the run is given a settings file of its own through KM_SONG_SYNC_SETTINGS.
#
# The port is 8282, and not the program's own 8182, so a run cannot collide with a page somebody
# has open.
#
# The pictures depend on the system font, like every other published picture. Regenerate them when
# the page or the editor changes, and not routinely.

set -euo pipefail

cd "$(dirname "$0")/../.."
REPO="$PWD"

. tools/dist/common.sh
DIST_SCRIPT=sync-pictures

OUT="docs/images"
DO_EDITOR=1
DO_PAGE=1

for arg in "$@"; do
  case "$arg" in
    --editor) DO_PAGE=0 ;;
    --page) DO_EDITOR=0 ;;
    -h | --help)
      sed -n '3,33p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "$DIST_SCRIPT: unknown argument $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

# The carol the pictures are of, by its number in the pack: "Angels From the Realms of Glory", which
# is the one the animated picture sings. The page lists it among the others named here.
CAROL=2
OTHERS="7 12 14"

# The line the editor is part-way through, counted from zero. The fifth line has one before it and
# one after it, and enough taps before it for the editor to have read the vocal line out of them.
LINE=4

PORT=8282
PAGE="http://127.0.0.1:$PORT"

# Spelled the way the operating system spells it. km-song-sync is a native program, and a Git Bash
# `/c/...` path is not one a Windows program can open.
case "$(uname -s 2>/dev/null)" in
  MINGW* | MSYS* | CYGWIN*) CAROLS_DEFAULT="C:/Users/Public/karaoke/carols" ;;
  *) CAROLS_DEFAULT="/tmp/karaoke/carols" ;;
esac
CAROLS_DIR="${KM_CAROLS_DIR:-$CAROLS_DEFAULT}"
CAROLS_DIR="${CAROLS_DIR%/}"

WORK="$(dist_target_dir)/sync-pictures"
PROFILE="$WORK/chrome"
SETTINGS="$WORK/settings.json"
WORDS="$WORK/words.txt"

FAILURES=0
SERVER=""

note() { printf '  %s\n' "$*"; }
warn() {
  printf '%s: %s\n' "$DIST_SCRIPT" "$*" >&2
  FAILURES=$((FAILURES + 1))
}

# The server is stopped on the way out, and the carols go with it. The folder is a shared one by
# construction, and nothing should be left sitting in it.
cleanup() {
  [ -n "$SERVER" ] && kill "$SERVER" 2>/dev/null
  [ -n "${STAGED:-}" ] && rm -rf "$CAROLS_DIR" 2>/dev/null
  return 0
}
trap cleanup EXIT INT TERM

. tools/dev/browser-shot.sh
. tools/dev/carol-pack.sh

mkdir -p "$OUT"
dist_clear "$WORK"
mkdir -p "$PROFILE"

# ------------------------------------------------------------------ the editor

if [ "$DO_EDITOR" = 1 ]; then
  dist_step "drawing the editor"
  cargo run -q -p karaokemachine --example sync_picture -- \
    picture "$PACK" "$CAROL" "$LINE" "$OUT/sync-editor-tapping.png" ||
    warn "the editor's picture failed"
fi

# -------------------------------------------------------------------- the page

page_picture() {
  CHROME=$(find_chrome) || {
    warn "no Chrome or Edge found; skipping the page's picture. Set KM_CHROME."
    return 0
  }
  note "browser    $CHROME"

  dist_step "writing the carols into $CAROLS_DIR"
  rm -rf "$CAROLS_DIR"
  mkdir -p "$CAROLS_DIR"
  STAGED=1
  # The carol the picture selects comes first, because its words are the ones written out.
  # shellcheck disable=SC2086
  cargo run -q -p karaokemachine --example sync_picture -- \
    stage "$PACK" "$CAROLS_DIR" "$WORDS" "$CAROL" $OTHERS >"$WORK/staged.txt"
  sed 's/^/  /' "$WORK/staged.txt"
  selected=$(sed -n '1s/^staged //p' "$WORK/staged.txt")
  # The words go into the page as markup, so they must hold nothing markup reads.
  if grep -q '[<&]' "$WORDS"; then
    warn "the carol's words hold a < or an &, which the page copy cannot carry"
    return 0
  fi

  # The page finds the machine beside itself, which is where cargo builds both.
  dist_step "starting km-song-sync (:$PORT)"
  cargo build -q -p karaokemachine --bin karaokemachine
  printf '{"locale":"en"}' >"$SETTINGS"
  KM_SONG_SYNC_SETTINGS="$SETTINGS" \
    cargo run -q -p km-song-sync -- "$CAROLS_DIR" --port "$PORT" >"$WORK/km-song-sync.log" 2>&1 &
  SERVER=$!
  for _ in $(seq 1 200); do
    curl -sf "$PAGE/" >/dev/null 2>&1 && break
    sleep 0.3
  done

  # **A copy of the page is pictured, with one script added.** Selecting a song is a press of a
  # button, and no address reaches that state. The script presses Select on the carol's row and puts
  # its words in the box, as a person does. The stylesheet and the page's own script still load from
  # the program, through the <base>.
  raw="$WORK/page.raw.html"
  page="$WORK/page.html"
  curl -sS -H 'Accept-Language: en' "$PAGE/" -o "$raw" || {
    warn "km-song-sync never answered"
    return 0
  }
  # A taken port sends the program to another one without a word, and something else answers here.
  grep -q 'data-select=' "$raw" || {
    warn "the page at $PAGE lists no songs; is another program on the port?"
    return 0
  }
  grep -q 'class="error"' "$raw" && warn "the page says the machine was not found"

  {
    sed -e "s|<head>|<head><base href=\"$PAGE/\">|" -e '/<\/body>/,$d' "$raw"
    printf '<script type="text/plain" id="picture-words">'
    cat "$WORDS"
    printf '</script>\n<script type="text/plain" id="picture-song">%s</script>\n<script>\n' "$selected"
    cat <<'EOF'
window.addEventListener("load", () => {
  // The press scrolls the panel into view, and the picture is of the whole page.
  Element.prototype.scrollIntoView = () => {};
  const name = document.getElementById("picture-song").textContent;
  [...document.querySelectorAll("[data-select]")].find((row) => row.dataset.name === name).click();
  const box = document.getElementById("words-box");
  box.value = document.getElementById("picture-words").textContent;
  box.dispatchEvent(new Event("input", { bubbles: true }));
});
</script>
</body>
</html>
EOF
  } >"$page"

  dist_step "capturing"
  shot "$OUT/song-sync-page.png" 1440 1100 1 "file:///$page"
}

[ "$DO_PAGE" = 1 ] && page_picture

# ------------------------------------------------------------------ the report

# The width and height of a PNG, read from its header. A page laid out at the wrong width is a valid
# file of the right name and the wrong picture.
png_size() {
  od -An -tu1 -j16 -N8 "$1" |
    awk '{printf "%dx%d", $1*16777216+$2*65536+$3*256+$4, $5*16777216+$6*65536+$7*256+$8}'
}

echo
echo "produced"
for pair in "song-sync-page.png 1440x1100" "sync-editor-tapping.png 1920x1080"; do
  f="$OUT/${pair% *}"
  want="${pair#* }"
  [ -f "$f" ] || continue
  got=$(png_size "$f")
  [ "$got" = "$want" ] || warn "$(basename "$f") is $got, expected $want"
  printf '  %-34s %5s KB\n' "$(basename "$f")" "$(($(wc -c <"$f") / 1024))"
done

if [ "$FAILURES" -gt 0 ]; then
  echo
  echo "$DIST_SCRIPT: $FAILURES step(s) did not produce a picture (see above)" >&2
  exit 1
fi
