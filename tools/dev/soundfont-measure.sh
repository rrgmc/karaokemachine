#!/usr/bin/env bash
#
# Measures General MIDI banks the way soundfont-banks.conf was measured -- seven songs of 90 s each
# at music_volume 1.0, levels deliberately NOT normalised -- and prints the two tables as markdown.
#
#   KM_CORPUS=<folder> tools/dev/soundfont-measure.sh                  # every cached bank
#   KM_CORPUS=<folder> tools/dev/soundfont-measure.sh musescore sgm    # named ones
#   KM_CORPUS=<folder> tools/dev/soundfont-measure.sh --file <path>.sf2
#   KM_CORPUS=<folder> tools/dev/soundfont-measure.sh --verdicts-only  # §3 only, one render each
#
# ## Why this exists
#
# The note's own §2 records the four commands per render and says "**Reproducing the measurements**",
# but nothing was ever committed that runs them -- so the fifteen-bank survey was 77 renders driven
# by hand, and re-running it after a synthesizer change meant doing that again. The note asks for
# exactly that re-run in its own opening paragraph. This is the loop, and nothing more: it makes no
# judgments, changes no settings and writes nothing outside its temporary directory.
#
# ## It never downloads
#
# `tools/setup/fetch-assets.sh` is the downloader and `tools/dev/soundfont.sh` is what plays a bank;
# this only *measures*, over what the asset cache already holds. Six of the eleven pinned banks come
# from a host the note measured at about 30 KB/s, so a measurement run that quietly fetched 2 GiB
# would be a surprise rather than a convenience. Banks that are not cached are named and skipped.
#
# ## What it checks before trusting a file
#
# `SF_BYTES` in tools/setup/soundfont-banks.sh is the exact size of the file the note measured, and
# it was put there for this and read by nothing until now: "SC-55" alone names several unrelated
# soundfonts, so a same-named different file is the realistic way to produce numbers that look fine
# and mean nothing. A mismatch is reported and measured anyway, flagged in the table -- refusing
# would be worse, since a newer release of a bank is still worth a number, just not a *comparable*
# one.

set -uo pipefail

cd "$(dirname "$0")/../.."

# The eleven banks, their sources and what each was measured at.
. tools/setup/soundfont-banks.sh
# CACHE -- outside the repository, shared between worktrees.
. tools/setup/asset-cache.sh

CORPUS="${KM_CORPUS:-}"
MAX_MS="${KM_MAX_MS:-90000}"
VOLUME="${KM_MUSIC_VOLUME:-1.0}"
VERDICTS_ONLY=""
FILES=()
NAMED=()
FAILURES=0

warn() {
  echo "soundfont-measure: $*" >&2
  FAILURES=$((FAILURES + 1))
}

die() {
  echo "soundfont-measure: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --file) FILES+=("${2:?--file needs a path}"); shift 2 ;;
    --file=*) FILES+=("${1#*=}"); shift ;;
    --verdicts-only) VERDICTS_ONLY=1; shift ;;
    -h | --help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) NAMED+=("$1"); shift ;;
  esac
done

# The seven songs of §4, named by basename. The corpus root is a local folder and is recorded in
# CLAUDE.local.md rather than here, which is why this takes KM_CORPUS and refuses to guess.
SONGS=(
  "Cazuza - Exagerado.kar"
  "Crazy Little Thing Called Love.kar"
  "In My Life.kar"
  "Bonus/25OR6TO4.KAR"
  "Bonus/1999.KAR"
  "Bonus/HotelCalifornia.kar"
  "Bonus/Legião Urbana - Tempo perdido.kar"
)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

RENDER=(cargo run -q --release -p km-audio --example render_wav --)

echo "soundfont-measure: building the renderer once, in release -- a debug render is minutes"
if ! cargo build -q --release -p km-audio --example render_wav; then
  die "the renderer would not build"
fi

# ---------------------------------------------------------------------------------------------------
# Which files to measure
# ---------------------------------------------------------------------------------------------------
#
# A bank named on the command line, or in the table when nothing was named, is looked up in the cache
# and skipped if it is not there. A `--file` is taken as given: that is how a bank the table does not
# know about -- including every one of the four the note found unopenable -- gets measured at all.

LABELS=()
PATHS=()
EXPECTED=()

add() { # label path expected_bytes
  LABELS+=("$1")
  PATHS+=("$2")
  EXPECTED+=("$3")
}

if [ ${#NAMED[@]} -eq 0 ] && [ ${#FILES[@]} -eq 0 ]; then
  NAMED=($KM_BANKS)
fi

for name in ${NAMED[@]+"${NAMED[@]}"}; do
  if ! km_bank "$name"; then
    warn "$name is not a bank this knows about; tools/dev/soundfont.sh --list has the names"
    continue
  fi
  if [ -z "${SF_NAME:-}" ]; then
    warn "$name has no file name in the table"
    continue
  fi
  if [ ! -f "$CACHE/$SF_NAME" ]; then
    echo "soundfont-measure: $name is not cached, skipping ($SF_NAME)"
    continue
  fi
  add "$name" "$CACHE/$SF_NAME" "${SF_BYTES:-}"
done

for file in ${FILES[@]+"${FILES[@]}"}; do
  if [ ! -f "$file" ]; then
    warn "$file is not a file"
    continue
  fi
  add "$(basename "$file")" "$file" ""
done

if [ ${#PATHS[@]} -eq 0 ]; then
  die "nothing to measure"
fi

# ---------------------------------------------------------------------------------------------------
# §3 -- does it load, and what did loading cost
# ---------------------------------------------------------------------------------------------------
#
# One render of one song is enough to answer this: the bank is parsed before a sample is produced, so
# a file that loads says so on `render_wav`'s own `synth`/`defects` lines and a file that does not
# fails the command. The song is whichever of the seven is present.

probe_song=""
if [ -n "$CORPUS" ]; then
  for song in "${SONGS[@]}"; do
    if [ -f "$CORPUS/$song" ]; then
      probe_song="$CORPUS/$song"
      break
    fi
  done
fi
if [ -z "$probe_song" ]; then
  die "KM_CORPUS is unset or holds none of the seven songs, so nothing can be rendered"
fi

LOADS=()
echo
echo "| Bank | Loads | Records dropped | What was dropped |"
echo "|---|---|---|---|"
for i in "${!PATHS[@]}"; do
  out="$("${RENDER[@]}" "$probe_song" "$TMP/probe.wav" "${PATHS[$i]}" --max-ms 2000 2>&1)"
  if printf '%s' "$out" | grep -q '^wrote '; then
    LOADS+=(1)
    defects="$(printf '%s' "$out" | sed -n 's/^  defects  *//p')"
    if [ -z "$defects" ]; then
      echo "| ${LABELS[$i]} | **OK** | 0 | — |"
    else
      count="$(printf '%s' "$defects" | sed -n 's/^\([0-9]*\) defective.*/\1/p')"
      # The first example only. The whole point of the cap is that a table row is one line.
      first="$(printf '%s' "$defects" | sed 's/^[0-9]* defective records\{0,1\} dropped: //; s/;.*//')"
      echo "| ${LABELS[$i]} | **OK** | ${count:-?} | \`$first\` |"
    fi
  else
    LOADS+=(0)
    reason="$(printf '%s' "$out" | sed -n 's/.*SoundFont(\"\(.*\)\").*/\1/p' | head -1)"
    echo "| ${LABELS[$i]} | **FAIL** | — | \`${reason:-see the command output}\` |"
  fi
  if [ -n "${EXPECTED[$i]}" ]; then
    actual="$(wc -c <"${PATHS[$i]}" | tr -d ' ')"
    if [ "$actual" != "${EXPECTED[$i]}" ]; then
      warn "${LABELS[$i]} is $actual bytes, not the ${EXPECTED[$i]} the note measured -- the numbers below are not comparable with its tables"
    fi
  fi
done

if [ -n "$VERDICTS_ONLY" ]; then
  echo
  echo "soundfont-measure: $FAILURES warning(s)"
  exit 0
fi

# ---------------------------------------------------------------------------------------------------
# §4 -- the seven songs, and the spread across them
# ---------------------------------------------------------------------------------------------------
#
# Song-to-song spread is the metric this project decided the bundled bank on: a karaoke machine has
# its music-to-microphone balance set once in hardware and then plays a hundred songs at it, so a
# bank that needs the volume knob between songs is the wrong bank however good any one song is.
#
# **The sd column is the population standard deviation**, which the note does not state anywhere --
# but its published 2.2 for GeneralUser GS is 2.15 by the population formula against 2.33 by the
# sample one, so that is what it used and that is what keeps the two comparable.
#
# ## The last two columns are §12's method, and they were a second pass by hand
#
# §12 derives the `music_volume` a hot bank wants from **the largest pre-clamp peak across the seven
# songs**, and is explicit that the `Worst true peak` column cannot do it: `write_wav` clamps before
# anything is written, so ffmpeg is reading a signal whose overshoot has already been flattened, and
# a bank reduced by what that column implies goes on clipping. This loop already reads the pre-clamp
# peak -- it is where the clipping count comes from -- and used to throw it away, so deriving a level
# meant rendering everything a second time by hand. It is kept and printed now, with the largest
# tenth that clears it beside it, which is the arithmetic §12 does and the number that goes in the
# table's `volume` field.

missing=0
for song in "${SONGS[@]}"; do
  [ -f "$CORPUS/$song" ] || { warn "KM_CORPUS has no $song"; missing=$((missing + 1)); }
done
if [ "$missing" -eq ${#SONGS[@]} ]; then
  die "none of the seven songs is under $CORPUS"
fi

echo
echo "| Bank | Mean LUFS | Range across the seven | sd | Worst true peak | Songs that clipped | Largest pre-clamp peak | Level |"
echo "|---|---|---|---|---|---|---|---|"

for i in "${!PATHS[@]}"; do
  [ "${LOADS[$i]}" = "1" ] || continue
  values=""
  peaks=""
  prepeaks=""
  clipped=0
  measured=0
  for song in "${SONGS[@]}"; do
    [ -f "$CORPUS/$song" ] || continue
    out="$("${RENDER[@]}" "$CORPUS/$song" "$TMP/m.wav" "${PATHS[$i]}" \
             --max-ms "$MAX_MS" --volume "$VOLUME" 2>&1)" || continue
    # render_wav's own peak is measured BEFORE write_wav clamps, so over 1.000 is the honest
    # clipping signal -- a WAV that has already been clamped cannot tell you it was.
    prepeak="$(printf '%s' "$out" | sed -n 's/^wrote .*peak \([0-9.]*\).*/\1/p')"
    [ -n "$prepeak" ] || continue
    summary="$(ffmpeg -hide_banner -nostats -i "$TMP/m.wav" \
                 -filter_complex ebur128=peak=true -f null - 2>&1 | sed -n '/Summary:/,$p')"
    lufs="$(printf '%s' "$summary" | sed -n 's/^ *I: *\(-*[0-9.]*\) LUFS/\1/p' | head -1)"
    tpk="$(printf '%s' "$summary" | sed -n 's/^ *Peak: *\(-*[0-9.]*\) dBFS/\1/p' | head -1)"
    [ -n "$lufs" ] || continue
    values="$values $lufs"
    peaks="$peaks $tpk"
    prepeaks="$prepeaks $prepeak"
    measured=$((measured + 1))
    if awk -v p="$prepeak" 'BEGIN { exit !(p > 1.0) }'; then
      clipped=$((clipped + 1))
    fi
  done
  if [ "$measured" -eq 0 ]; then
    warn "${LABELS[$i]} produced no measurable render"
    continue
  fi
  awk -v bank="${LABELS[$i]}" -v clipped="$clipped" -v n="$measured" \
      -v vals="$values" -v pks="$peaks" -v pres="$prepeaks" 'BEGIN {
    c = split(vals, v, " "); s = 0
    for (i = 1; i <= c; i++) s += v[i]
    mean = s / c
    lo = v[1]; hi = v[1]; ss = 0
    for (i = 1; i <= c; i++) {
      if (v[i] < lo) lo = v[i]
      if (v[i] > hi) hi = v[i]
      ss += (v[i] - mean) ^ 2
    }
    pc = split(pks, p, " "); worst = p[1]
    for (i = 1; i <= pc; i++) if (p[i] > worst) worst = p[i]

    # §12: the level is the largest tenth that keeps the largest pre-clamp peak under full scale.
    # A bank that never exceeded 1.0 gets none -- "0 of 7 clipped means the largest pre-clamp peak
    # is already under 1.0, so there is nothing for a reduction to buy".
    rc = split(pres, r, " "); top = r[1]
    for (i = 1; i <= rc; i++) if (r[i] > top) top = r[i]
    level = "—"
    if (top > 1.0) {
      # **Hundredths, not tenths, and the difference is audible.** The rule is the same -- the largest
      # step that keeps the peak under full scale -- but a tenth throws away up to a whole step of
      # headroom for nothing. Chorium needs 0.69 and a tenth gives it 0.6: 1.2 dB quieter than it has
      # to be, which is enough to lose an A/B against a bank that got a kinder rounding.
      steps = int(100 / top)
      # **A bank can exceed full scale by more than a level can rescue.** `Roland_SC-55.sf2` reaches
      # 2.2e19 on two of the seven songs -- a divergence rather than a hot mix -- and the arithmetic
      # returns 0 for it, which is not a level, it is silence. Saying so is the honest answer;
      # printing it would put a number in the bank table that would mute the bank.
      level = (steps >= 1) ? sprintf("**%.2f**", steps / 100) : "**none rescues it**"
    }

    printf "| %s | %.1f | **%.1f LU** | %.1f | %+.1f dBTP | %d of %d | %.3f | %s |\n",
      bank, mean, hi - lo, sqrt(ss / c), worst, clipped, n, top, level
  }'
done

echo
echo "soundfont-measure: $FAILURES warning(s)"
[ "$FAILURES" -eq 0 ]
