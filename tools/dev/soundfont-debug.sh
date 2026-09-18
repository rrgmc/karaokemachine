#!/usr/bin/env bash
#
# Fills the machine's Ctrl+2..Ctrl+9 SoundFont slots with the banks already on this box.
#
#   tools/dev/soundfont-debug.sh                 # find them and fill the slots
#   tools/dev/soundfont-debug.sh --choose        # tick the ones you want, from all of the table
#   tools/dev/soundfont-debug.sh --list          # what it would use, without writing anything
#   tools/dev/soundfont-debug.sh --clear         # empty the slots, turning the switcher off
#
# `task soundfont:debug`, `task soundfont:debug:choose`, `task soundfont:debug:list` and
# `task soundfont:debug:clear` are these.
#
# ## What the slots are for
#
# `Ctrl+1`..`Ctrl+9` play a numbered bank from now on, keeping the song and its position -- so the
# same bar can be heard under two banks a second apart, which is the only way a comparison means
# anything. Ctrl+1 is always the bank the machine resolved for itself, so it needs no configuration
# and is the reference the rest are judged against; this fills 2 upwards.
#
# A non-empty list is what turns the switcher on, keys and on-screen label together, so `--clear`
# turns it off. See the `Switching the SoundFont while it plays` decision in docs/decisions/audio.md.
#
# ## `--choose`, and why the plain command is not enough
#
# **The plain command can only ever reach the top of the table.** It walks the bank table top-down
# and keeps the first eight rows whose file is already cached -- and the table is in rank order, the
# nine ranked banks first and then the other fifty-four by ascending spread. So the slots are
# structurally always the highest-ranked cached banks, and there is no way to say "compare
# merlin-orchestra against opl3fm" short of typing eight `<path>=<name>=<volume>` specs whose paths
# are cache filenames like `41.8mg_saphyr_two_thousand_gm_gs_bank.sf2`.
#
# `--choose` opens a checkbox list of **every** bank in the table, with whatever is in the slots
# already ticked, and fetches the ones that are ticked and missing. Space ticks a row, typing filters
# the list, Enter confirms and Esc changes nothing. The list is `tools/dev/km-pick`, which is a
# hundred lines around `inquire`'s `MultiSelect` and knows nothing about SoundFonts -- everything
# here that is about banks is in this file, where the rest of it already was.
#
# Two things it does that are worth knowing before you tick something:
#
#   * **Order is the table's, not the order you tick.** The highest-ranked bank you choose is Ctrl+2.
#     Same rule the plain command uses, so the two cannot disagree about what a slot means.
#   * **A bank that is not cached is downloaded**, through tools/setup/fetch-assets.sh and its pinned
#     digests -- but not before it has told you which ones and how much, and asked. Some rows in that
#     table are over a gigabyte.
#   * **A `manual` bank cannot be ticked**, because nothing may point a downloader at it. It is drawn
#     all the same and refused at the prompt, with the list still up and everything else still
#     ticked -- one unavailable row must not cost seven good decisions.
#
# ## Where the banks come from
#
# The asset cache, which is where `tools/setup/fetch-assets.sh --bank <name>` puts them -- outside
# the repository, shared between worktrees, and never installed into `assets/` because everything
# there is copied into every release. So this finds what has already been fetched rather than
# fetching: `task soundfont:list` says what there is, and `fetch-assets.sh --bank musescore` gets
# one.
#
# **It asks the bank table rather than scanning a folder.** tools/setup/soundfont-banks.sh knows every
# bank, its filename and the music_volume each was measured to want, so every slot gets a short name
# for the label and a level that makes the comparison fair -- where a `find` would turn up whatever
# else is in the folder and have nothing to call it. `KM_SF2_DIRS` is for a bank the table does not
# know.
#
# ## Environment
#
#   KM_SF2_DIRS   extra folders to look in, `;`-separated. Every `.sf2` directly inside one is
#                 offered, named by its filename stem. Unset by default and deliberately not
#                 guessed at: a folder of banks is a local path, and a local path may not appear in
#                 a committed file -- keep yours in CLAUDE.local.md. `;` rather than `:` because a
#                 Windows path starts with a drive letter and a colon.
#   KM_ASSET_CACHE  moves the asset cache; see tools/setup/asset-cache.sh.
#
# ## What it does not do
#
# It does not check that a bank loads -- **the machine does that**, before it writes anything, and
# refuses the lot if any one of them fails. Four of the fifteen banks in
# crates/machine/km-banks/data/soundfont-banks.conf do not open in `rustysynth` at all, and a slot that fails in front of a room is the thing this
# ordering exists to prevent.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=soundfont-debug

# The bank table -- every bank, its filename and what each was measured at.
. tools/setup/soundfont-banks.sh

# CACHE: where a fetched bank was put.
. tools/setup/asset-cache.sh

# How many slots there are to fill. Ctrl+1 is the bundled bank and is not one of them, so this is
# nine keys minus that one. The machine refuses more than this too -- it is the number of keys, and
# neither side may be the only one that knows it.
SLOTS=8

ACTION="set"

# The header block above is the help text, printed to the first line that is not a comment. The same
# form tools/dev/worktree.sh uses, and for the reason it gives: a hardcoded `sed` range starts
# printing the argument parser the moment somebody adds a paragraph.
usage() {
  awk 'NR > 1 { if (!/^#/) exit; sub(/^# ?/, ""); print }' "$0"
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --list) ACTION="list" ;;
    --choose) ACTION="choose" ;;
    --clear) ACTION="clear" ;;
    -h | --help) usage 0 ;;
    *)
      echo "$DIST_SCRIPT: unknown argument $1 (try --help)" >&2
      exit 2
      ;;
  esac
  shift
done

# **The machine is what writes settings.json**, so the whole of this script is finding one to run.
# Both helpers are tools/dev/soundfont.sh's, unchanged -- including the `bin-console` preference, for
# the reason it gives: on Windows the executable in `dist/bin` is GUI-subsystem and can print nothing
# at all, which would make a command whose whole output is a report silently empty.
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

run_machine() {
  local exe
  exe="$(machine)"
  if [ -n "$exe" ]; then
    "$exe" "$@"
  else
    echo "$DIST_SCRIPT: nothing staged in dist/bin-console -- building a machine to do it with." >&2
    echo "      'task dist:bin' stages one, and is much faster to reach for afterwards." >&2
    cargo run -q -p karaokemachine --bin karaokemachine-console -- "$@"
  fi
}

# Absolute, and in the form the machine will read back. `cygpath -m` on Windows, because a settings
# file holding `/c/Users/...` names nothing a Rust `Path` can open.
host_path() {
  if command -v cygpath > /dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

# The bundled bank, so a cached copy of the same file does not spend a slot saying what Ctrl+1
# already says. Empty in a checkout that has never fetched assets, in which case nothing is skipped
# and the cached copy is genuinely worth a slot.
#
# Duplicated from `SOUNDFONT_SUBPATHS` in crates/machine/karaokemachine/src/settings.rs, which is
# the source of truth -- a shell script cannot read a Rust constant. The same duplication
# tools/dist/check-assets.sh carries, and it is named there too.
#
# Its result is held in `BUNDLED_BANK` and **not** in `BUNDLED`, which looks like the obvious name
# and is already taken: `km_bank` in tools/setup/soundfont-banks.sh sets `BUNDLED` as one of its
# output variables, so the first call in the loop below silently emptied it and every bank came out
# looking unlike the bundled one.
bundled_bank() {
  local name
  for name in gm.sf2 GeneralUser-GS.sf2 FluidR3_GM.sf2; do
    if [ -f "assets/soundfont/$name" ]; then
      printf '%s' "assets/soundfont/$name"
      return 0
    fi
  done
  printf ''
}

# Whether two paths hold the same bank.
#
# Two tests, and the second is the one that does the work. `-ef` catches a symlink or a hard link --
# which is how tools/dev/worktree.sh seeds a worktree's banks. But the cache and `assets/soundfont/`
# hold **separate copies** of the same download, so `-ef` alone says they differ and slot 2 comes out
# as a second Ctrl+1.
#
# Comparing filenames is exact rather than a heuristic here: every name on both sides comes from
# `SF_NAME` in the pinned bank table, so `GeneralUser-GS.sf2` is one download by construction. A
# bank somebody renamed is the case it gets wrong, and getting that wrong costs a duplicate slot.
same_bank() {
  if [ -e "$1" ] && [ -e "$2" ] && [ "$1" -ef "$2" ]; then
    return 0
  fi
  [ "$(basename "$1")" = "$(basename "$2")" ]
}

# -- what is here ------------------------------------------------------------------------------
#
# Index-aligned arrays: the name a label will show, the file, the level the bank wants, and enough
# about each row for `--choose` to draw it. Filled in KM_BANKS order, which is the research note's
# ranking, so a box with more banks than slots keeps the ones worth hearing first.
#
# **Every bank in the table goes in, cached or not, and `CACHED` says which.** The plain command and
# `--list` throw the uncached ones away immediately afterwards -- they can only offer what is here --
# where `--choose` offers all of them and fetches what is ticked. Building the full set either way
# keeps one loop that knows how a row becomes a slot, rather than two that have to agree.
IDS=()
NAMES=()
PATHS=()
VOLUMES=()
CACHED=()
STATUSES=()
SIZES=()
SPREADS=()
BYTES=()
NOTES=()
SKIPPED_BUNDLED=""

BUNDLED_BANK="$(bundled_bank)"

for key in $KM_BANKS; do
  km_bank "$key" || continue
  [ -n "$SF_NAME" ] || continue
  file="$CACHE/$SF_NAME"
  # The bundled bank is Ctrl+1 whatever happens, so a copy of it may never spend a slot saying the
  # same thing -- and there is certainly no point offering to download one. `same_bank` compares
  # filenames when neither `-ef` test applies, so this works before the file is there.
  if [ -n "$BUNDLED_BANK" ] && same_bank "$file" "$BUNDLED_BANK"; then
    # Only worth remarking on when the copy is really here; otherwise nothing was skipped.
    [ -f "$file" ] && SKIPPED_BUNDLED="$key"
    continue
  fi
  IDS+=("$key")
  NAMES+=("$key")
  PATHS+=("$file")
  VOLUMES+=("${SF_VOLUME:-}")
  if [ -f "$file" ]; then CACHED+=(1); else CACHED+=(0); fi
  STATUSES+=("${SF_STATUS:-}")
  SIZES+=("${SF_SIZE:--}")
  SPREADS+=("${SF_SPREAD:--}")
  BYTES+=("${SF_BYTES:-0}")
  NOTES+=("${SF_NOTE:-}")
done

# Then anything in the folders somebody named. After the table's own, because these have no measured
# level and no name but a filename -- so where there are more banks than slots, a known bank is the
# better use of one.
if [ -n "${KM_SF2_DIRS:-}" ]; then
  # `;`-separated, and read with IFS rather than word splitting so a folder may contain spaces.
  old_ifs="$IFS"
  IFS=';'
  read -r -a sf2_dirs <<< "$KM_SF2_DIRS"
  IFS="$old_ifs"

  for dir in "${sf2_dirs[@]}"; do
    [ -n "$dir" ] || continue
    if [ ! -d "$dir" ]; then
      echo "$DIST_SCRIPT: warning -- KM_SF2_DIRS names $dir, which is not a directory" >&2
      continue
    fi
    # `-maxdepth 1`: a folder of banks, not a tree to walk. A corpus root pointed at this by mistake
    # should turn up what is loose in it and no more.
    while IFS= read -r file; do
      [ -n "$file" ] || continue
      if [ -n "$BUNDLED_BANK" ] && same_bank "$file" "$BUNDLED_BANK"; then continue; fi
      # Not twice, if it is also the cached copy of a bank the table knows.
      #
      # **Against the cached rows only**, which matters now that the loop above keeps uncached ones
      # too: a bank the table names but has not fetched is not this file, and dropping the folder's
      # copy because a row exists for it would lose the one that is actually on disk.
      already=0
      index=0
      while [ "$index" -lt "${#PATHS[@]}" ]; do
        if [ "${CACHED[$index]}" -eq 1 ] && same_bank "$file" "${PATHS[$index]}"; then
          already=1
          break
        fi
        index=$((index + 1))
      done
      [ "$already" -eq 0 ] || continue

      stem="$(basename "$file")"
      stem="${stem%.*}"
      # `file:` and the path, because a folder's bank has no id in the table and the picker hands
      # back whatever key it was given.
      IDS+=("file:$file")
      NAMES+=("$stem")
      PATHS+=("$file")
      VOLUMES+=("")
      CACHED+=(1)
      STATUSES+=("local")
      SIZES+=("-")
      SPREADS+=("-")
      BYTES+=(0)
      NOTES+=("from KM_SF2_DIRS")
    done < <(find "$dir" -maxdepth 1 -type f \( -iname '*.sf2' \) | sort)
  done
fi

# The plain command and `--list` can only offer what is already on the box, so the uncached rows go
# now. `--choose` keeps them: offering to fetch one is the whole of what it adds.
if [ "$ACTION" != "choose" ]; then
  keep_ids=() keep_names=() keep_paths=() keep_volumes=()
  keep_cached=() keep_statuses=() keep_sizes=() keep_spreads=() keep_bytes=() keep_notes=()
  index=0
  while [ "$index" -lt "${#PATHS[@]}" ]; do
    if [ "${CACHED[$index]}" -eq 1 ]; then
      keep_ids+=("${IDS[$index]}")
      keep_names+=("${NAMES[$index]}")
      keep_paths+=("${PATHS[$index]}")
      keep_volumes+=("${VOLUMES[$index]}")
      keep_cached+=(1)
      keep_statuses+=("${STATUSES[$index]}")
      keep_sizes+=("${SIZES[$index]}")
      keep_spreads+=("${SPREADS[$index]}")
      keep_bytes+=("${BYTES[$index]}")
      keep_notes+=("${NOTES[$index]}")
    fi
    index=$((index + 1))
  done
  IDS=(${keep_ids[@]+"${keep_ids[@]}"})
  NAMES=(${keep_names[@]+"${keep_names[@]}"})
  PATHS=(${keep_paths[@]+"${keep_paths[@]}"})
  VOLUMES=(${keep_volumes[@]+"${keep_volumes[@]}"})
  CACHED=(${keep_cached[@]+"${keep_cached[@]}"})
  STATUSES=(${keep_statuses[@]+"${keep_statuses[@]}"})
  SIZES=(${keep_sizes[@]+"${keep_sizes[@]}"})
  SPREADS=(${keep_spreads[@]+"${keep_spreads[@]}"})
  BYTES=(${keep_bytes[@]+"${keep_bytes[@]}"})
  NOTES=(${keep_notes[@]+"${keep_notes[@]}"})
fi

FOUND=${#PATHS[@]}

# -- clear -------------------------------------------------------------------------------------

if [ "$ACTION" = "clear" ]; then
  run_machine --clear-debug-soundfonts
  exit 0
fi

# -- nothing to do -----------------------------------------------------------------------------

if [ "$FOUND" -eq 0 ]; then
  echo "$DIST_SCRIPT: no banks found, so there is nothing to put in a slot." >&2
  echo "      The asset cache is $CACHE" >&2
  echo "      'task soundfont:list' says which banks there are; 'tools/setup/fetch-assets.sh" >&2
  echo "      --bank musescore' fetches one without installing it anywhere." >&2
  echo "      KM_SF2_DIRS adds folders of your own -- see --help." >&2
  exit 1
fi

# -- choose ------------------------------------------------------------------------------------
#
# The one mode that can reach past the top of the table, and the only one that downloads anything.

# A byte count the way the table writes one. `awk` and not `bc`, which is not installed here.
human_bytes() {
  awk -v b="$1" 'BEGIN {
    if (b >= 1073741824) printf "%.1f GiB", b / 1073741824
    else if (b >= 1048576) printf "%.1f MiB", b / 1048576
    else printf "%.1f KiB", b / 1024
  }'
}

# Which row a key belongs to. Prints the index, or nothing.
row_index() {
  local want="$1" index=0
  while [ "$index" -lt "$FOUND" ]; do
    if [ "${IDS[$index]}" = "$want" ]; then
      printf '%s' "$index"
      return 0
    fi
    index=$((index + 1))
  done
  return 1
}

if [ "$ACTION" = "choose" ]; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT

  # What is in the slots now, so the list opens on the current answer rather than on nothing --
  # which is the difference between choosing a list and re-choosing one from scratch.
  #
  # `--show-debug-soundfonts` prints one slot a line in exactly the form `--set-debug-soundfonts`
  # reads. A machine that cannot be run or has no slots set leaves the file empty, and an empty file
  # is a perfectly good "nothing is ticked yet".
  : > "$work/current"
  run_machine --show-debug-soundfonts > "$work/current" || : > "$work/current"

  # Matched back to rows by filename, which is the join the table already guarantees is unique --
  # `name` is unique across it, and `Machine::measured_level` relies on the same fact.
  #
  # **A slot that is already set keeps its own name and level**, held here and used instead of the
  # table's when the spec is built again. This box's slot 2 is called `colombo` rather than
  # `colombogmgs2` and its sgm-guits sits at 0.97 against the table's 0.93 -- somebody tuned those,
  # and re-ticking a row they did not touch must not quietly undo it. The same rule
  # `--set-soundfont` already follows: a level edited by hand is kept rather than overwritten.
  # Taken as a pair, so a slot deliberately left with no level does not acquire the table's.
  TICKED=""
  KEEP_NAME=()
  KEEP_VOLUME=()
  index=0
  while [ "$index" -lt "$FOUND" ]; do
    KEEP_NAME+=("")
    KEEP_VOLUME+=("")
    index=$((index + 1))
  done

  while IFS= read -r line; do
    line="${line%$'\r'}"
    [ -n "$line" ] || continue
    current_base="$(basename "${line%%=*}")"
    index=0
    while [ "$index" -lt "$FOUND" ]; do
      if [ "$(basename "${PATHS[$index]}")" = "$current_base" ]; then
        TICKED="$TICKED $index "
        # `<path>=<name>[=<volume>]`, and a path holds no `=` -- a Windows one starts with a drive
        # letter and a colon.
        rest="${line#*=}"
        if [ "$rest" != "$line" ]; then
          KEEP_NAME[$index]="${rest%%=*}"
          volume="${rest#*=}"
          [ "$volume" = "$rest" ] || KEEP_VOLUME[$index]="$volume"
        fi
        break
      fi
      index=$((index + 1))
    done
  done < "$work/current"

  # One row per bank: the key the picker hands back, whether it starts ticked, and a label that is
  # already aligned. The picker draws the label as it stands and has no opinion about what is in it.
  : > "$work/rows"
  index=0
  while [ "$index" -lt "$FOUND" ]; do
    state="cached"
    if [ "${CACHED[$index]}" -eq 0 ]; then
      # A `manual` bank has no URL anybody may point a downloader at -- see `Where a bank may be
      # fetched from` in docs/decisions/repository.md. Saying so in the row is cheaper than letting
      # somebody tick it and find out at the confirmation.
      if [ "${STATUSES[$index]}" = "manual" ]; then state="manual"; else state="fetch"; fi
    fi
    flags=""
    case "$TICKED" in *" $index "*) flags="on" ;; esac
    # A `manual` bank that is not already here cannot be ticked at all -- the picker draws it,
    # refuses it and keeps everything else ticked. Accepting it and rejecting it after the list has
    # closed throws away seven other decisions to say one thing.
    if [ "$state" = "manual" ]; then
      [ -z "$flags" ] && flags="no" || flags="$flags,no"
    fi
    note="${NOTES[$index]}"
    [ "${#note}" -le 44 ] || note="${note:0:41}..."
    label="$(printf '%-17s %10s %6s  %-6s %s' \
      "${NAMES[$index]}" "${SIZES[$index]}" "${SPREADS[$index]}" "$state" "$note")"
    printf '%s\t%s\t%s\n' "${IDS[$index]}" "$flags" "$label" >> "$work/rows"
    index=$((index + 1))
  done

  # Six spaces so the header lands under the labels rather than under the `> [x] ` the picker draws
  # in front of each one.
  title="$(printf 'Choose up to %s banks for Ctrl+2..Ctrl+9 (Ctrl+1 is always the bundled bank)\n      %-17s %10s %6s  %-6s %s' \
    "$SLOTS" BANK SIZE SPREAD STATE NOTE)"

  # `cargo run` rather than a staged binary, deliberately: nothing may assume cargo builds into
  # `target/` (see BUILDING.md), and this is a development command whose crate is a workspace member
  # anyway. `-q` so a warm build prints nothing above the list.
  #
  # **`--rows` and never a pipe.** On Windows the prompt reads its keys from stdin, so rows sent
  # down it are typed into the list -- which is not a theory: it ticked seven banks nobody asked for
  # and confirmed them, in one run, silently.
  status=0
  cargo run -q -p km-pick -- \
    --rows "$work/rows" --out "$work/chosen" \
    --title "$title" --max "$SLOTS" --page-size 18 \
    --refusal "{} is manual: its terms forbid mirroring, so it has to be downloaded by hand first" \
    || status=$?

  case "$status" in
    0) ;;
    1)
      echo "Nothing was changed."
      exit 0
      ;;
    3)
      echo "$DIST_SCRIPT: --choose needs a terminal to draw its list on." >&2
      echo "      On Windows, try it from Windows Terminal rather than through a pipe." >&2
      exit 1
      ;;
    *)
      echo "$DIST_SCRIPT: the picker failed ($status)." >&2
      exit 1
      ;;
  esac

  CHOSEN=()
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    CHOSEN+=("$id")
  done < "$work/chosen"

  # Unticking everything is the natural way to turn the switcher off, so it is that rather than an
  # error -- but not silently, because it is also what a mis-keyed Enter looks like.
  if [ "${#CHOSEN[@]}" -eq 0 ]; then
    printf 'No banks are ticked. Empty the slots and turn the switcher off? [y/N] '
    read -r reply < /dev/tty
    case "$reply" in
      [yY]*) run_machine --clear-debug-soundfonts ;;
      *) echo "Nothing was changed." ;;
    esac
    exit 0
  fi

  # In the order the picker was given them, which is the table's -- so the highest-ranked bank
  # ticked is Ctrl+2, exactly as the plain command would have put it there.
  # Every chosen key resolved to a row, and refused by name if it is not one -- a key the picker
  # invented would otherwise become a slot pointing at nothing.
  wanted=""
  for id in "${CHOSEN[@]}"; do
    if ! index="$(row_index "$id")"; then
      echo "$DIST_SCRIPT: the picker returned '$id', which is not a row it was given." >&2
      exit 1
    fi
    wanted="$wanted $index "
  done

  # **The slots are filled by walking the rows in order**, not by walking what came back -- so the
  # order is the table's whatever order the picker returns, and the highest-ranked bank ticked is
  # Ctrl+2. km-pick does sort by input order, but that would make this file's one visible promise
  # depend on a detail of another program.
  PICKED=()
  TO_FETCH=()
  TOTAL=0
  index=0
  while [ "$index" -lt "$FOUND" ]; do
    case "$wanted" in
      *" $index "*)
        if [ "${CACHED[$index]}" -eq 0 ]; then
          # The picker refuses these while its list is still up, so reaching here means a row was
          # marked `no` and chosen anyway. Kept because the alternative to a message is a slot
          # pointing at a file that was never downloaded.
          if [ "${STATUSES[$index]}" = "manual" ]; then
            km_bank "${IDS[$index]}"
            echo "$DIST_SCRIPT: ${IDS[$index]} has no direct download address" >&2
            echo "      Get it from $SF_PAGE ($SF_SIZE)," >&2
            echo "      put it in $(host_path "$CACHE"), and tick it again." >&2
            echo "      No slot was changed." >&2
            exit 2
          fi
          TO_FETCH+=("$index")
          TOTAL=$((TOTAL + ${BYTES[$index]}))
        fi
        PICKED+=("$index")
        ;;
    esac
    index=$((index + 1))
  done

  # Told first, asked second, and only then downloaded. Some rows in this table are over a
  # gigabyte, and the archive.org ones serve at about 30 KB/s.
  if [ "${#TO_FETCH[@]}" -gt 0 ]; then
    echo
    echo "${#TO_FETCH[@]} bank(s) are not here yet:"
    for index in "${TO_FETCH[@]}"; do
      printf '  %-17s %10s  %s\n' "${NAMES[$index]}" "${SIZES[$index]}" "${NOTES[$index]}"
    done
    printf 'Fetch them (%s in total)? [y/N] ' "$(human_bytes "$TOTAL")"
    read -r reply < /dev/tty
    case "$reply" in
      [yY]*) ;;
      *)
        echo "Nothing was changed."
        exit 0
        ;;
    esac
    for index in "${TO_FETCH[@]}"; do
      dist_step "fetching ${NAMES[$index]} (${SIZES[$index]})"
      # Downloading lives in fetch-assets.sh and nowhere else, so there is one place that knows
      # about the cache, the digests and the banks published inside a zip. It prints the path and
      # nothing else on stdout, and that is the path used rather than one rebuilt here.
      PATHS[$index]="$(tools/setup/fetch-assets.sh --bank "${IDS[$index]}" --cache-only)"
    done
  fi

  SPECS=()
  for index in "${PICKED[@]}"; do
    # A slot that was already set keeps whatever it was called and whatever level it was given; only
    # a row that is new to the slots takes the table's.
    name="${NAMES[$index]}"
    volume="${VOLUMES[$index]}"
    if [ -n "${KEEP_NAME[$index]}" ]; then
      name="${KEEP_NAME[$index]}"
      volume="${KEEP_VOLUME[$index]}"
    fi
    spec="$(host_path "${PATHS[$index]}")=$name"
    if [ -n "$volume" ]; then
      spec="$spec=$volume"
    fi
    SPECS+=("$spec")
  done

  echo
  run_machine --set-debug-soundfonts "${SPECS[@]}"
  exit 0
fi

# -- the report, which --list stops after ------------------------------------------------------

dist_step "$FOUND bank(s) found for the switcher's $SLOTS slots"
printf '%-6s %-14s %6s  %s\n' SLOT NAME VOLUME FILE
index=0
while [ "$index" -lt "$FOUND" ]; do
  slot=$((index + 2))
  if [ "$index" -lt "$SLOTS" ]; then
    printf 'Ctrl+%-1s %-14s %6s  %s\n' \
      "$slot" "${NAMES[$index]}" "${VOLUMES[$index]:--}" "${PATHS[$index]}"
  else
    printf '%-6s %-14s %6s  %s\n' \
      "--" "${NAMES[$index]}" "${VOLUMES[$index]:--}" "${PATHS[$index]}"
  fi
  index=$((index + 1))
done

echo
echo "Ctrl+1 is the bundled bank and is always there, whatever is in the slots."
if [ -n "$SKIPPED_BUNDLED" ]; then
  echo "The cached '$SKIPPED_BUNDLED' is that same file, so it is not given a slot of its own."
fi
if [ "$FOUND" -gt "$SLOTS" ]; then
  echo "$DIST_SCRIPT: warning -- $((FOUND - SLOTS)) bank(s) past Ctrl+9 will not be set." >&2
  echo "      The order is the ranking in soundfont-banks.conf, so the ones left out" >&2
  echo "      are the ones it ranks lowest. Name the rest by hand with --set-debug-soundfonts." >&2
fi

if [ "$ACTION" = "list" ]; then
  echo
  echo "Nothing was written. Run it without --list to fill the slots."
  exit 0
fi

# -- fill the slots ----------------------------------------------------------------------------
#
# `<path>=<name>=<volume>`, which the machine parses. The volume travels with the bank because banks
# differ enough in level to clip, and an A/B in which one is simply louder answers a question nobody
# asked.
SPECS=()
index=0
while [ "$index" -lt "$FOUND" ] && [ "$index" -lt "$SLOTS" ]; do
  spec="$(host_path "${PATHS[$index]}")=${NAMES[$index]}"
  if [ -n "${VOLUMES[$index]}" ]; then
    spec="$spec=${VOLUMES[$index]}"
  fi
  SPECS+=("$spec")
  index=$((index + 1))
done

echo
run_machine --set-debug-soundfonts "${SPECS[@]}"
