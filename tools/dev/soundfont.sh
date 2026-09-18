#!/usr/bin/env bash
#
# Plays a different General MIDI bank on this machine, locally and undoably.
#
#   tools/dev/soundfont.sh                       # which bank is playing, and where it came from
#   tools/dev/soundfont.sh --list                # the banks this knows about
#   tools/dev/soundfont.sh --bank musescore      # fetch it and point this machine at it
#   tools/dev/soundfont.sh --file <path>.sf2     # ...a bank already on disk
#   tools/dev/soundfont.sh --bank musescore --music-volume 0.7
#   tools/dev/soundfont.sh --clear               # back to the bundled bank
#
# `task soundfont`, `task soundfont:list` and `task soundfont:clear` are these.
#
# ## What it actually does, and why that reaches dist/bin
#
# It runs the machine's own `--set-soundfont`, which **installs the bank into the machine's SoundFont
# folder** and then writes its **bank id** into `audio.soundfont`. The folder is what says which
# banks exist, so a bank left only in the shared download cache would be named and then not found;
# the install is a hard link where the filesystem allows one, so nothing is duplicated per worktree.
# That setting is
# the only override that reaches **every** build on the box, and the reason is structural rather than
# incidental: `Paths::asset_dirs_from` returns the directory beside the executable the moment there
# is one, and returns **no overlay** with it. So `dist/bin/<platform>/karaokemachine` -- which has an
# `assets/` sibling -- can never see the checkout's `local/assets/`, however many banks are put
# there. A settings key can be seen by all of them.
#
# **Nothing here touches `assets/`, `dist/` or the checkout.** `assets/` is copied wholesale into
# every carrier, so a 206 MiB bank left there would be added to every release; `dist/bin` is the
# thing being handed over and is rebuilt by the next staging run anyway. The bank stays in the asset
# cache -- outside the repository, shared between worktrees -- and settings.json, which is
# per-machine and in no carrier at all, says which one to play. See the `Switching the SoundFont
# locally` decision in docs/decisions/.
#
# ## Two things it does not do
#
# It does not check that the bank is any good, and it does not check that it loads -- **the machine
# does the second**, before it writes anything, which is the point of `--set-soundfont` being a flag
# rather than a line somebody edits into settings.json. Four of the fifteen banks in
# crates/machine/km-banks/data/soundfont-banks.conf do not open in `rustysynth` at all.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=soundfont

# The eleven banks, their sources and what each was measured at.
. tools/setup/soundfont-banks.sh

BANK=""
FILE=""
VOLUME=""
ACTION="status"

while [ $# -gt 0 ]; do
  case "$1" in
    --bank) shift; BANK="${1:-}"; ACTION="set"
      [ -n "$BANK" ] || { echo "soundfont: --bank needs a name (try --list)" >&2; exit 2; } ;;
    --bank=*) BANK="${1#--bank=}"; ACTION="set" ;;
    --file) shift; FILE="${1:-}"; ACTION="set"
      [ -n "$FILE" ] || { echo "soundfont: --file needs a path" >&2; exit 2; } ;;
    --file=*) FILE="${1#--file=}"; ACTION="set" ;;
    --music-volume) shift; VOLUME="${1:-}" ;;
    --music-volume=*) VOLUME="${1#--music-volume=}" ;;
    --clear) ACTION="clear" ;;
    --list) ACTION="list" ;;
    -h|--help)
      sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "soundfont: unknown argument $1 (try --help)" >&2; exit 2 ;;
  esac
  shift
done

if [ -n "$BANK" ] && [ -n "$FILE" ]; then
  echo "soundfont: name a bank or a file, not both" >&2
  exit 2
fi

# **The machine is what writes settings.json**, so the whole of this script is finding one to run.
#
# Any of them writes the same file -- every desktop build asks `directories` for the same config
# directory -- so the preference is not about correctness. It is about `--show-paths`, which resolves
# the bundled bank relative to whichever executable answers: the interesting answer is the staged
# build's, because that is the one somebody is about to go and listen to.
#
# `bin-console` and never `bin`: on Windows the executable in `dist/bin` is GUI-subsystem and can
# print nothing at all, which would make the status command silently empty.
machine() {
  local triple
  triple="$(dist_host_triple)"
  local dir="dist/bin-console/$(dist_platform "$triple")"
  local ext
  ext="$(dist_exe_ext "$triple")"

  if [ -x "$dir/karaokemachine-console$ext" ]; then
    printf '%s' "$dir/karaokemachine-console$ext"
  elif [ -x "$dir/karaokemachine$ext" ]; then
    printf '%s' "$dir/karaokemachine$ext"
  else
    printf ''
  fi
}

# Runs the machine with whatever arguments were given. Falls back to building one, which is right
# rather than merely convenient: somebody in a fresh checkout who has not staged anything still has a
# machine, and it is the same settings file either way.
run_machine() {
  local exe
  exe="$(machine)"
  if [ -n "$exe" ]; then
    "$exe" "$@"
  else
    # To stderr and not through `dist_step`: the status branch captures this function's stdout, and a
    # phase line inside a captured pipeline is a phase line nobody ever sees.
    echo "$DIST_SCRIPT: nothing staged in dist/bin-console -- building a machine to do it with." >&2
    echo "      'task dist:bin' stages one, and is much faster to reach for afterwards." >&2
    cargo run -q -p karaokemachine --bin karaokemachine-console -- "$@"
  fi
}

case "$ACTION" in
  list)
    printf '%-12s %10s  %-7s %6s %6s  %s\n' BANK SIZE SOURCE SPREAD VOLUME NOTES
    for name in $KM_BANKS; do
      km_bank "$name"
      printf '%-12s %10s  %-7s %6s %6s  %s\n' \
        "$name" "$SF_SIZE" "$SF_STATUS" "${SF_SPREAD:--}" "${SF_VOLUME:--}" "$SF_LICENSE"
    done
    cat <<'EOF'

SPREAD is song-to-song loudness spread in LU, which is the metric this project decided on -- lower is
better, and the bundled bank is 6.7. VOLUME is the music_volume a bank needs because it exceeds full
scale at 1.0; it is set for you. Both come from crates/machine/km-banks/data/soundfont-banks.conf,
which records fifteen banks and the four that do not load at all.

  task soundfont BANK=musescore      fetch it and play it
  task soundfont FILE=<a bank>.sf2   anything already on disk, including a `manual` one
  task soundfont:clear               back to the bundled bank

Nothing here is installed into assets/ or dist/, and nothing reaches a release: the bank stays in the
asset cache and settings.json names it.
EOF
    ;;

  status)
    # One line out of `--show-paths`, which is where the rule that picks a bank already lives. The
    # rest of that report is about directories and is not what was asked.
    line="$(run_machine --show-paths | grep '^soundfont ' || true)"
    if [ -z "$line" ]; then
      echo "soundfont: the machine did not say which bank it would play" >&2
      echo "      Run it directly to see why: $(machine) --show-paths" >&2
      exit 1
    fi
    echo "$line"
    # ...and which of the eleven that is, where it is one of them. The path says the filename and the
    # table is the only thing that can turn that back into a name somebody typed.
    base="${line##*[\\/]}"
    base="${base%% *}"
    for name in $KM_BANKS; do
      km_bank "$name"
      if [ "$SF_NAME" = "$base" ]; then
        echo "           $name -- $SF_NOTE"
        break
      fi
    done
    ;;

  clear)
    run_machine --clear-soundfont
    ;;

  set)
    if [ -n "$BANK" ]; then
      if ! km_bank "$BANK"; then
        echo "soundfont: no bank called '$BANK'" >&2
        echo "      task soundfont:list names the eleven this knows about." >&2
        exit 2
      fi
      if [ "$SF_STATUS" = "manual" ]; then
        # Not a gap in the plumbing. arachnosoft.com serves the file through a page rather than a
        # URL, so there is no address to pin a digest against and nothing for this to fetch.
        echo "soundfont: $BANK has no direct download address" >&2
        echo "      Get it from $SF_PAGE ($SF_SIZE)," >&2
        echo "      then: task soundfont FILE=<the file you downloaded>" >&2
        [ -z "$SF_VOLUME" ] || echo "      It wants music_volume $SF_VOLUME; add MUSIC_VOLUME=$SF_VOLUME." >&2
        exit 2
      fi
      # Downloading lives in fetch-assets.sh and nowhere else, so there is one place that knows about
      # the cache, the digests and the one bank published inside a zip. It prints the path and
      # nothing else on stdout.
      dist_step "fetching $BANK ($SF_SIZE)"
      FILE="$(tools/setup/fetch-assets.sh --bank "$BANK" --cache-only)"
      [ -z "$VOLUME" ] && VOLUME="$SF_VOLUME"
    fi

    # `audio.soundfont` beats the overlay outright, so a bank sitting in the checkout's
    # local/assets/soundfont would go silent from here on with nothing to say why. Nothing writes one
    # any more -- fetch-assets.sh stopped -- but a checkout that predates that still has one.
    if [ -d local/assets/soundfont ]; then
      echo "$DIST_SCRIPT: warning -- local/assets/soundfont/ exists, and audio.soundfont wins over it." >&2
      echo "      That folder is the old override and nothing writes one any more." >&2
      echo "      rm -rf local/assets/soundfont once you are done comparing." >&2
    fi

    if [ -n "$VOLUME" ]; then
      run_machine --set-soundfont "$FILE" --music-volume "$VOLUME"
    else
      run_machine --set-soundfont "$FILE"
    fi
    ;;
esac
