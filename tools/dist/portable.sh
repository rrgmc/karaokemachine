#!/usr/bin/env bash
#
# Stages the portable copy: every program in one folder, with everything it writes kept beside it.
#
#   tools/dist/portable.sh               # stage dist/bin{,-console}, then build both copies
#   tools/dist/portable.sh --no-build    # build both copies from what is already gathered
#   tools/dist/portable.sh --no-video    # the smaller build, throughout
#   tools/dist/portable.sh -v            # watch the builds; quiet is the default
#
#   dist/portable/<platform>/karaokemachine-portable-<version>-<system>-<arch>/        the folder
#   dist/portable/<platform>/karaokemachine-portable-<version>-<system>-<arch>.zip     Windows
#   dist/portable/<platform>/karaokemachine-portable-<version>-<system>-<arch>.tar.gz  Linux
#   dist/portable-console/<platform>/karaokemachine-portable-console-<version>-<system>-<arch>/
#
# **The marker file is what makes a folder portable.** `km-dirs` reads it beside the executable,
# and every program then keeps its files under `data/` there. The decision is
# `A portable copy keeps its state beside its programs` in docs/decisions/distribution.md.
#
# **The marker goes into these two folders and no other.** `dist/bin/<platform>` is the payload of
# the setup program, which must install an ordinary copy. tools/dist/bin.sh also empties that folder
# on every run, which would delete the `data/` folder of anybody working out of it.
#
# **It gathers rather than builds.** tools/dist/bin.sh stages both folders and proves each program
# starts. This script copies them, so no fact about a product is written here a second time.
#
# **Each copy holds one form of each program.** `portable/` copies `dist/bin/<platform>`, the folder
# a person opens to double-click a program. `portable-console/` copies `dist/bin-console/<platform>`,
# whose programs print to the console that started them.
#
# **The console copy is a folder and has no archive.** A release page carries the windowed copy
# alone, and an archive is the thing a release hands over.
#
# **Windows and Linux.** A macOS bundle cannot hold files that change, so `km-dirs` reads no marker
# there and this script refuses the platform.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=dist-portable

VIDEO=1
BUILD=1
FROM=""
ARGS=("$@")
while [ $# -gt 0 ]; do
  case "$1" in
    --no-video) VIDEO=0 ;;
    --no-build) BUILD=0 ;;
    # One folder that already holds every program, in place of the two tools/dist/bin.sh gathers.
    # tools/platform/linux/portable-in-container.sh passes the folder it built.
    --from)
      shift
      [ $# -gt 0 ] || { echo "dist-portable: --from needs a folder" >&2; exit 2; }
      FROM="$1"; BUILD=0
      ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/dist/portable.sh [--no-build] [--no-video] [--from <folder>] [-v]"
      exit 0
      ;;
    *) echo "dist-portable: unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

TARGET="$(dist_host_triple)"
PLATFORM="$(dist_platform "$TARGET")"
EXT="$(dist_exe_ext "$TARGET")"

case "$PLATFORM" in
  windows|linux) ;;
  *) echo "dist-portable: no portable copy on $PLATFORM; a bundle cannot keep files beside itself." >&2
     exit 1 ;;
esac

# **On Linux the folder is built in a container**, so every program links one glibc and the tools
# take the machine's ffmpeg. That script calls back here with `--from`.
if [ "$PLATFORM" = linux ] && [ -z "$FROM" ]; then
  exec tools/platform/linux/portable.sh "${ARGS[@]+"${ARGS[@]}"}"
fi

# The names `km-dirs` reads. Spelled a second time here because a shell script cannot read a Rust
# constant; the check in `stage_copy` fails when the two disagree.
MARKER="karaokemachine-portable.txt"
DATA="data"

if [ "$BUILD" -eq 1 ]; then
  FLAGS=()
  if [ "$VIDEO" -ne 1 ]; then FLAGS+=(--no-video); fi
  if dist_verbose; then FLAGS+=(--verbose); fi
  tools/dist/bin.sh "${FLAGS[@]+"${FLAGS[@]}"}"
  echo
fi

# A folder from `--from` holds one form of each program, so both copies are made from it.
BIN="dist/bin/$PLATFORM"
CONSOLE="dist/bin-console/$PLATFORM"
if [ -n "$FROM" ]; then BIN="$FROM"; CONSOLE="$FROM"; fi
for dir in "$BIN" "$CONSOLE"; do
  if [ ! -f "$dir/README.txt" ]; then
    echo "dist-portable: $dir is not gathered." >&2
    echo "               build it with: tools/dist/bin.sh" >&2
    exit 1
  fi
done

VERSION="$(dist_version "$BIN/karaokemachine$EXT")"

# The system ahead of the architecture, which is the rule every file on a release page keeps.
#
# The marker goes on the build that declined video, and the folder answers that: the libraries are
# there or they are not. Two globs asked one at a time, because `ls` fails when either is empty.
SUFFIX="-no-video"
for lib in "$BIN"/avcodec-*.dll "$BIN"/lib/libav*; do
  if [ -e "$lib" ]; then SUFFIX=""; fi
done

# -- the bank a setup program's tick box offers -------------------------------------------------------

# A zip has no tick box, so the request ships under a name the machine does not read. Renaming it
# is the asking that `Nothing downloads` in docs/decisions/ wants before anything is fetched.
# `crates/machine/karaokemachine/src/firstrun.rs` reads the renamed file on the next start.
#
# The row comes from the table, as tools/platform/windows/installer.sh takes it, and the concrete
# id is written for that script's reason: the bank named in the document is the bank that arrives.
. tools/setup/soundfont-banks.sh
BANK_ID=""
for name in $KM_BANKS; do
  km_bank "$name"
  [ "$SF_RECOMMENDED" = "1" ] || continue
  BANK_ID="$name"
  BANK_NAME="$SF_NAME"
  BANK_SIZE="$SF_SIZE"
  BANK_STATUS="$SF_STATUS"
done
if [ -z "$BANK_ID" ] || [ "$BANK_STATUS" = "manual" ]; then
  echo "dist-portable: the table recommends no bank the machine can download." >&2
  exit 1
fi
REQUEST="first-run-soundfont.json"
dist_detail "soundfont  $BANK_ID -> $BANK_NAME ($BANK_SIZE), offered and not requested"

# Under a name the machine does not read, for a reason of its own: a new archive is unpacked over an
# old folder, and a `settings.json` in it would replace the one somebody edited. A test in
# crates/machine/karaokemachine/src/settings.rs holds the file to keys the machine has. Each is at
# its default but `api.room_access`, which gives the room the control level.
SETTINGS_EXAMPLE="settings.example.json"

# One copy: the folder, its marker, its `data` folders, its document and its proof. The windowed
# copy also gets the archive.
stage_copy() { # <gathered folder> <kind: gui|console>
  local src="$1" kind="$2"
  local name parent out own twin scratch probe shown fail twins left want bytes exe base wide

  # The suffix a program's name takes in this copy. A console twin is a Windows file, so the folder
  # answers whether there is one.
  twin=""
  if [ "$kind" = console ] && [ -f "$src/karaokemachine-console$EXT" ]; then twin="-console"; fi

  if [ "$kind" = gui ]; then
    name="karaokemachine-portable-$VERSION-$PLATFORM-${TARGET%%-*}$SUFFIX"
    parent="dist/portable/$PLATFORM"
  else
    name="karaokemachine-portable-console-$VERSION-$PLATFORM-${TARGET%%-*}$SUFFIX"
    parent="dist/portable-console/$PLATFORM"
  fi
  out="$parent/$name"

  dist_step "staging $out"
  mkdir -p "$out"
  dist_clear "$out"

  cp -R "$src"/. "$out"/

  # The tarball's `install.sh` registers the machine in the home directory, which a portable copy
  # never writes to.
  rm -f "$out/install.sh"

  # The tarball's folder names the machine's document `README.txt`, which is this folder's own name
  # for its own. The machine's keeps the name every other program's has.
  if [ -f "$out/README.txt" ] && [ ! -f "$out/README-karaokemachine.txt" ]; then
    mv "$out/README.txt" "$out/README-karaokemachine.txt"
  fi

  cat > "$out/$MARKER" <<'MARKER_TEXT'
This file makes this folder a portable copy of KaraokeMachine.

While it is here, every program in this folder keeps its settings, songs and
everything else it writes in the data folder beside it. Nothing is read from
your user profile or home directory, and nothing is written there.

Delete this file and the programs use your user profile again, as an
installed copy does. The data folder then stays where it is and is not read.
MARKER_TEXT

  # -- the folders somebody puts files into ----------------------------------------------------------

  # Made here so the folder shows where things go before the machine has run once. Each name is the
  # one `Paths` in crates/machine/karaokemachine/src/settings/paths.rs gives it.
  own="$out/$DATA/karaokemachine"
  mkdir -p "$own/packages" "$own/soundfonts" "$own/wallpapers"

  cat > "$own/packages/README.txt" <<'TEXT'
Song packages go here.

A song package is a file whose name ends in .kmpkg. Copy one into this folder
and start the machine: its songs are then in the catalog.

To make a package from your own karaoke files, use KM Package Builder or
KM Simple Package in the folder above.

The machine reads this text file and nothing else that is not a package.
TEXT

  cat > "$own/soundfonts/README.txt" <<'TEXT'
Instrument banks go here.

An instrument bank is a SoundFont file, whose name ends in .sf2. It decides
how the instruments of a MIDI song sound. Copy one into this folder, then
choose it on the machine's Sound page.

The machine already has one bank, in the assets folder beside the programs.
A bank here is offered beside it. Banks the machine downloads are also kept
here.

The machine can download the bank it recommends, the first time it starts.
The README.txt beside the programs says how to ask for that.
TEXT

  printf '{\n  "bank": "%s"\n}\n' "$BANK_ID" > "$own/$REQUEST.example"
  cp tools/dist/portable-settings.example.json "$own/$SETTINGS_EXAMPLE"

  cat > "$own/wallpapers/README.txt" <<'TEXT'
Your own background pictures go here.

A wallpaper is a still picture: a .jpg, .png or .webp file. The machine shows
one behind the words of a song.

The machine already has a set of pictures, in the assets folder beside the
programs. Pictures in this folder replace that set. Pictures you upload from
the machine's page are also kept here.
TEXT

  # -- the folder's own document ---------------------------------------------------------------------

  # The name column of the Ports table is as wide as the longest name in this copy.
  wide=$((21 + ${#twin}))
  {
    if [ "$kind" = gui ]; then
      cat <<'HEAD'
KaraokeMachine -- a portable copy
=================================
HEAD
    else
      cat <<'HEAD'
KaraokeMachine -- a portable copy (console)
===========================================
HEAD
    fi
    cat <<'HEAD'

This folder holds every program, and everything the programs write stays in
this folder. Nothing is installed, and nothing is read from or written to your
user profile or home directory. A copy that is installed on the same computer
keeps its own settings and songs, and this one does not touch them.
HEAD
    if [ "$kind" = console ]; then
      cat <<'HEAD'

Each program here prints to the console that starts it. Use this copy to read
why a program does not start. The portable copy with no console in its name
holds the programs you double-click.
HEAD
    fi
    cat <<'HEAD'

What is here
------------

HEAD
    for exe in "$out"/*; do
      [ -f "$exe" ] || continue
      base="$(basename "$exe")"
      case "$base" in
        *.exe) [ -n "$EXT" ] && printf '    %s\n' "$base" ;;
        *.*) ;;
        *) [ -z "$EXT" ] && printf '    %s\n' "$base" ;;
      esac
    done
    cat <<'BODY'

Where your files are
--------------------

    data/karaokemachine/packages     song packages (.kmpkg)
    data/karaokemachine/soundfonts   instrument banks (.sf2)
    data/karaokemachine/wallpapers   your own background pictures
    data/karaokemachine              the machine's settings and catalog
    data/km-remote                   the remote's songs list and favorites
    data/km-package-builder          the package builder's settings
    data/km-package-simple           the simple package tool's settings
    data/km-song-sync                the song sync page's settings
    data/km-admin                    pictures and banks KM Admin downloaded

The first three folders are made already, each with a text file that says
what goes in it. A program makes its own folder the first time it runs.

A document here whose name begins README- belongs to one program, and was
written for an ordinary folder. Where one says your files are under your user
profile or home directory, in this copy they are in the data folder.

The rules about this folder
---------------------------

Keep it together. The programs find assets, data and the file
karaokemachine-portable.txt beside themselves. Move or copy the whole folder,
not files out of it.

Keep it somewhere you can write to. A program that cannot write to the data
folder stops and says so.

The file karaokemachine-portable.txt is what makes this copy portable. Open it
to read what happens without it.

This copy does not register file types, so double-clicking a .kmpkg file does
not open it. Put the file in data/karaokemachine/packages.
BODY
    cat <<BODY

A better instrument bank
------------------------

The machine comes with one instrument bank. It recommends a larger one,
$BANK_NAME, which is a download of $BANK_SIZE. Nothing is downloaded unless
you ask. To ask, do one of these before you start the machine:

  - In data/karaokemachine, rename $REQUEST.example to
    $REQUEST.
  - Or run this once:

        karaokemachine$twin --first-run-soundfont recommended

The machine downloads the bank the next time it starts, into
data/karaokemachine/soundfonts, and then uses it.

BODY
    dist_bank_by_hand "data/karaokemachine/soundfonts"
    cat <<'BODY'

Changing the machine's settings
-------------------------------

The machine keeps its settings in data/karaokemachine/settings.json, and
writes that file the first time it starts. To set things before then, copy
settings.example.json in that folder to settings.json and edit the copy. It
holds the settings people change most:

    machine.name         the name a phone shows for this machine
    machine.locale       the language of the screen: en or pt-BR
    api.bind             the address and port the machine listens on
    api.room_access      what a phone may do with no code: view, queue or control
    display.fullscreen   true to fill the screen, false for a window
    package_dirs         more folders to read song packages from

Each is at its usual value except api.room_access. The file sets it to
control, so any phone on your network can skip a song, play one now and
change the queue. The usual value is queue, which lets a phone add a song
and nothing more. The machine's Admin pages ask for the password either way.

Stop the machine before you edit settings.json. It writes the file again
while it runs.

Ports
-----

Each program serves its pages on a port of its own. The machine takes its
port from api.bind in settings.json. The others take --port when you start
them:

BODY
    printf "    %-${wide}s%s   %s\n" \
      "karaokemachine$twin"     8177 "api.bind, or --api-bind 8277 for one run" \
      "km-package-builder$twin" 8178 "km-package-builder$twin --port 8278" \
      "km-remote$twin"          8179 "km-remote$twin --port 8279" \
      "km-admin$twin"           8180 "km-admin$twin --port 8280" \
      "km-package-simple$twin"  8181 "km-package-simple$twin --port 8281" \
      "km-song-sync$twin"       8182 "km-song-sync$twin --port 8282"
    cat <<BODY

If an installed machine is running on this computer too, give this one
another port. A program that sends songs to the machine has to be told the
machine's address when it is not the usual one, with --machine:

    km-remote$twin --machine 127.0.0.1:8277

Getting rid of it
-----------------

Delete the folder. Your songs and settings in the data folder go with it, so
copy the data folder out first if you want to keep them.
BODY
    if [ "$PLATFORM" = windows ]; then
      cat <<'BODY'

What Windows needs
------------------

The Microsoft Visual C++ 2015-2022 Redistributable, and the WebView2 Runtime
for the programs that open a window. Windows 11 has both. Without WebView2 a
program opens its page in your browser.
BODY
    fi
    if [ -z "$SUFFIX" ]; then
      printf '\nThis build plays video songs.\n'
    else
      printf '\nThis build does not play video songs. They are listed and queued, and do not play.\n'
    fi
  } > "$out/README.txt"

  # -- what this folder claims, tested rather than asserted ------------------------------------------

  # The claim is that the machine names nothing outside this folder. `--show-paths` prints where it
  # keeps things and writes nothing, so it is asked from the folder itself.
  #
  # **Only the machine has such a report.** The other programs take their folders from the same
  # `km-dirs` function, and `clippy.toml` refuses any other source, so the machine's answer stands
  # for theirs. On Linux the home directory is also pointed at an empty folder, which must stay
  # empty.
  scratch="$parent/.proof"
  rm -rf "$scratch"
  mkdir -p "$scratch"

  # A folder from `--from` has not been through tools/dist/bin.sh, so its programs are started here,
  # with nothing inherited from this shell.
  if [ -n "$FROM" ]; then
    for exe in "$out"/*; do
      [ -f "$exe" ] || continue
      base="$(basename "$exe")"
      case "$base" in *.*) continue ;; esac
      ( cd "$out" && env -i HOME="$PWD/$scratch" PATH=/usr/bin:/bin "./$base" --version >/dev/null 2>&1 ) || {
        echo "dist-portable: $base would not start with nothing inherited from this shell." >&2
        exit 1
      }
    done
  fi
  # The program asked is the machine this copy ships. The plain name answers into a pipe whatever
  # its subsystem.
  probe="karaokemachine$twin$EXT"
  if [ "$PLATFORM" = windows ]; then
    shown="$(cd "$out" && PATH="/c/Windows/System32:/c/Windows" "./$probe" --show-paths 2>&1)" || {
      echo "dist-portable: $probe --show-paths did not run." >&2; exit 1; }
  else
    shown="$(cd "$out" && env -i HOME="$PWD/$scratch" XDG_CONFIG_HOME="$PWD/$scratch/config" \
      XDG_DATA_HOME="$PWD/$scratch/data" XDG_CACHE_HOME="$PWD/$scratch/cache" PATH=/usr/bin:/bin \
      "./$probe" --show-paths 2>&1)" || {
      echo "dist-portable: $probe --show-paths did not run." >&2; exit 1; }
  fi
  shown="$(printf '%s\n' "$shown" | tr '\\' '/' | tr -d '\r')"

  fail=0
  printf '%s\n' "$shown" | grep -q '^portable  *yes' || {
    echo "dist-portable: the machine does not read $MARKER as the marker." >&2; fail=1; }
  for line in settings catalog packages; do
    printf '%s\n' "$shown" | grep "^$line " | grep -q "/$name/$DATA/karaokemachine" || {
      echo "dist-portable: the machine keeps its $line outside $DATA/karaokemachine." >&2; fail=1; }
  done
  if [ -n "$(find "$scratch" -type f 2>/dev/null)" ]; then
    echo "dist-portable: the machine wrote into the home directory:" >&2
    find "$scratch" -type f >&2
    fail=1
  fi
  rm -rf "$scratch"
  # One form of each program. The windowed copy holds no twin, and the console copy holds no
  # program beside its own twin.
  twins="$(cd "$out" && find . -maxdepth 1 -name '*-console*' | LC_ALL=C sort | tr '\n' ' ')"
  if [ "$kind" = gui ] && [ -n "$twins" ]; then
    echo "dist-portable: $out holds a console twin: $twins" >&2
    fail=1
  fi
  if [ "$kind" = console ]; then
    for exe in $twins; do
      base="${exe%"$EXT"}"
      base="${base%-console}$EXT"
      if [ -e "$out/$base" ]; then
        echo "dist-portable: $out holds both forms of one program: $base and $exe" >&2
        fail=1
      fi
    done
  fi
  # The copy carries the five files written above and nothing a run left behind. A live request
  # among them would make every copy download a bank nobody asked for, and a live settings file
  # would replace the one in a folder it is unpacked over.
  left="$(cd "$out/$DATA" && find . -type f | LC_ALL=C sort | tr '\n' ' ')"
  want="./karaokemachine/$REQUEST.example ./karaokemachine/packages/README.txt ./karaokemachine/$SETTINGS_EXAMPLE ./karaokemachine/soundfonts/README.txt ./karaokemachine/wallpapers/README.txt "
  if [ "$left" != "$want" ]; then
    echo "dist-portable: $out/$DATA does not hold exactly its five files: $left" >&2
    fail=1
  fi
  if [ "$fail" -ne 0 ]; then
    printf '%s\n' "$shown" >&2
    exit 1
  fi
  echo "verified: the machine keeps its settings, catalog and packages inside the folder."

  bytes="$(dist_bytes "$out")"
  printf 'staged %s\n' "$out"
  printf '  %s bytes total (~%s MiB)\n' "$bytes" "$((bytes / 1024 / 1024))"

  if [ "$kind" = console ] && [ -z "$twin" ]; then
    echo "  note: on $PLATFORM nothing has two forms, so the two copies hold the same programs."
  fi

  # -- the archive -----------------------------------------------------------------------------------

  if [ "$kind" != gui ]; then return 0; fi
  if [ "$PLATFORM" = windows ]; then
    rm -f "$parent/$name.zip"
    dist_zip "$parent" "$name"
  else
    # The flags the machine's own tarball takes, so two runs over one build give one archive.
    tar -C "$parent" --owner=0 --group=0 --numeric-owner --sort=name -czf "$parent/$name.tar.gz" "$name"
    printf 'wrote %s\n' "$parent/$name.tar.gz"
  fi
}

stage_copy "$BIN" gui
echo
stage_copy "$CONSOLE" console
