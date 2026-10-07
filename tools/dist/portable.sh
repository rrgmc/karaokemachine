#!/usr/bin/env bash
#
# Stages the portable copy: every program in one folder, with everything it writes kept beside it.
#
#   tools/dist/portable.sh               # stage dist/bin{,-console}, then build the archive
#   tools/dist/portable.sh --no-build    # build the archive from what is already gathered
#   tools/dist/portable.sh --no-video    # the smaller build, throughout
#   tools/dist/portable.sh -v            # watch the builds; quiet is the default
#
#   dist/portable/<platform>/karaokemachine-portable-<version>-<system>-<arch>/        the folder
#   dist/portable/<platform>/karaokemachine-portable-<version>-<system>-<arch>.zip     Windows
#   dist/portable/<platform>/karaokemachine-portable-<version>-<system>-<arch>.tar.gz  Linux
#
# **The marker file is what makes the folder portable.** `km-dirs` reads it beside the executable,
# and every program then keeps its files under `data/` there. The decision is
# `A portable copy keeps its state beside its programs` in docs/decisions/distribution.md.
#
# **The marker goes into this folder and no other.** `dist/bin/<platform>` is the payload of the
# setup program, which must install an ordinary copy. tools/dist/bin.sh also empties that folder on
# every run, which would delete the `data/` folder of anybody working out of it.
#
# **It gathers rather than builds.** tools/dist/bin.sh stages both folders and proves each program
# starts. This script copies them, so no fact about a product is written here a second time.
#
# **Both forms of a program are here.** The windowed form is the one to double-click, and the
# console form is the one that prints. A portable copy has no second folder to reach for.
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
# constant; the check at the bottom fails when the two disagree.
MARKER="karaokemachine-portable.txt"
DATA="data"

if [ "$BUILD" -eq 1 ]; then
  FLAGS=()
  if [ "$VIDEO" -ne 1 ]; then FLAGS+=(--no-video); fi
  if dist_verbose; then FLAGS+=(--verbose); fi
  tools/dist/bin.sh "${FLAGS[@]+"${FLAGS[@]}"}"
  echo
fi

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

MACHINE="$CONSOLE/karaokemachine-console$EXT"
[ -f "$MACHINE" ] || MACHINE="$CONSOLE/karaokemachine$EXT"
VERSION="$(dist_version "$MACHINE")"

# The system ahead of the architecture, which is the rule every file on a release page keeps.
#
# The marker goes on the build that declined video, and the folder answers that: the libraries are
# there or they are not. Two globs asked one at a time, because `ls` fails when either is empty.
SUFFIX="-no-video"
for lib in "$BIN"/avcodec-*.dll "$BIN"/lib/libav*; do
  if [ -e "$lib" ]; then SUFFIX=""; fi
done
NAME="karaokemachine-portable-$VERSION-$PLATFORM-${TARGET%%-*}$SUFFIX"
PARENT="dist/portable/$PLATFORM"
OUT="$PARENT/$NAME"

dist_step "staging $OUT"
mkdir -p "$OUT"
dist_clear "$OUT"

# The console folder first and the windowed one over it. A file in both is the same file, because
# tools/dist/bin.sh gathered both from one staged build. A release signs the windowed folder's
# copy of a single-form program, so that folder goes last.
cp -R "$CONSOLE"/. "$OUT"/
cp -R "$BIN"/. "$OUT"/

# The tarball's `install.sh` registers the machine in the home directory, which a portable copy
# never writes to.
rm -f "$OUT/install.sh"

# The tarball's folder names the machine's document `README.txt`, which is this folder's own name
# for its own. The machine's keeps the name every other program's has.
if [ -f "$OUT/README.txt" ] && [ ! -f "$OUT/README-karaokemachine.txt" ]; then
  mv "$OUT/README.txt" "$OUT/README-karaokemachine.txt"
fi

cat > "$OUT/$MARKER" <<'MARKER_TEXT'
This file makes this folder a portable copy of KaraokeMachine.

While it is here, every program in this folder keeps its settings, songs and
everything else it writes in the data folder beside it. Nothing is read from
your user profile or home directory, and nothing is written there.

Delete this file and the programs use your user profile again, as an
installed copy does. The data folder then stays where it is and is not read.
MARKER_TEXT

# -- the folders somebody puts files into ------------------------------------------------------------

# Made here so the folder shows where things go before the machine has run once. Each name is the
# one `Paths` in crates/machine/karaokemachine/src/settings/paths.rs gives it.
OWN="$OUT/$DATA/karaokemachine"
mkdir -p "$OWN/packages" "$OWN/soundfonts" "$OWN/wallpapers"

cat > "$OWN/packages/README.txt" <<'TEXT'
Song packages go here.

A song package is a file whose name ends in .kmpkg. Copy one into this folder
and start the machine: its songs are then in the catalog.

To make a package from your own karaoke files, use KM Package Builder or
KM Simple Package in the folder above.

The machine reads this text file and nothing else that is not a package.
TEXT

cat > "$OWN/soundfonts/README.txt" <<'TEXT'
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
printf '{\n  "bank": "%s"\n}\n' "$BANK_ID" > "$OWN/$REQUEST.example"
dist_detail "soundfont  $BANK_ID -> $BANK_NAME ($BANK_SIZE), offered and not requested"

# -- the settings somebody is most likely to change ---------------------------------------------------

# Under a name the machine does not read, for a reason of its own: a new archive is unpacked over an
# old folder, and a `settings.json` in it would replace the one somebody edited. A test in
# crates/machine/karaokemachine/src/settings.rs holds the file to keys the machine has, at their
# defaults.
SETTINGS_EXAMPLE="settings.example.json"
cp tools/dist/portable-settings.example.json "$OWN/$SETTINGS_EXAMPLE"

cat > "$OWN/wallpapers/README.txt" <<'TEXT'
Your own background pictures go here.

A wallpaper is a still picture: a .jpg, .png or .webp file. The machine shows
one behind the words of a song.

The machine already has a set of pictures, in the assets folder beside the
programs. Pictures in this folder replace that set. Pictures you upload from
the machine's page are also kept here.
TEXT

# -- the folder's own document -----------------------------------------------------------------------

{
  cat <<'HEAD'
KaraokeMachine -- a portable copy
=================================

This folder holds every program, and everything the programs write stays in
this folder. Nothing is installed, and nothing is read from or written to your
user profile or home directory. A copy that is installed on the same computer
keeps its own settings and songs, and this one does not touch them.

What is here
------------

HEAD
  for exe in "$OUT"/*; do
    [ -f "$exe" ] || continue
    base="$(basename "$exe")"
    case "$base" in
      *.exe) [ -n "$EXT" ] && printf '    %s\n' "$base" ;;
      *.*) ;;
      *) [ -z "$EXT" ] && printf '    %s\n' "$base" ;;
    esac
  done
  if [ -n "$EXT" ]; then
    cat <<'BODY'

Where a program has two names, the plain one opens a window and the one
ending in -console prints to a terminal. Use the plain one. Use the -console
one when a program does not start and you want to read why.
BODY
  fi
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

        karaokemachine --first-run-soundfont recommended

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
holds the settings people change most, each at its usual value:

    machine.name         the name a phone shows for this machine
    machine.locale       the language of the screen: en or pt-BR
    api.bind             the address and port the machine listens on
    display.fullscreen   true to fill the screen, false for a window
    package_dirs         more folders to read song packages from

Stop the machine before you edit settings.json. It writes the file again
while it runs.

Ports
-----

Each program serves its pages on a port of its own. The machine takes its
port from api.bind in settings.json. The others take --port when you start
them:

    karaokemachine       8177   api.bind, or --api-bind 8277 for one run
    km-package-builder   8178   km-package-builder --port 8278
    km-remote            8179   km-remote --port 8279
    km-admin             8180   km-admin --port 8280
    km-package-simple    8181   km-package-simple --port 8281

If an installed machine is running on this computer too, give this one
another port. A program that sends songs to the machine has to be told the
machine's address when it is not the usual one, with --machine:

    km-remote --machine 127.0.0.1:8277

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
} > "$OUT/README.txt"

# -- what this folder claims, tested rather than asserted --------------------------------------------

# The claim is that the machine names nothing outside this folder. `--show-paths` prints where it
# keeps things and writes nothing, so it is asked from the folder itself.
#
# **Only the machine has such a report.** The other programs take their folders from the same
# `km-dirs` function, and `clippy.toml` refuses any other source, so the machine's answer stands for
# theirs. On Linux the home directory is also pointed at an empty folder, which must stay empty.
SCRATCH="$PARENT/.proof"
rm -rf "$SCRATCH"
mkdir -p "$SCRATCH"

# A folder from `--from` has not been through tools/dist/bin.sh, so its programs are started here,
# with nothing inherited from this shell.
if [ -n "$FROM" ]; then
  for exe in "$OUT"/*; do
    [ -f "$exe" ] || continue
    base="$(basename "$exe")"
    case "$base" in *.*) continue ;; esac
    ( cd "$OUT" && env -i HOME="$PWD/$SCRATCH" PATH=/usr/bin:/bin "./$base" --version >/dev/null 2>&1 ) || {
      echo "dist-portable: $base would not start with nothing inherited from this shell." >&2
      exit 1
    }
  done
fi
PROBE="karaokemachine-console$EXT"
[ -f "$OUT/$PROBE" ] || PROBE="karaokemachine$EXT"
if [ "$PLATFORM" = windows ]; then
  SHOWN="$(cd "$OUT" && PATH="/c/Windows/System32:/c/Windows" "./$PROBE" --show-paths 2>&1)" || {
    echo "dist-portable: $PROBE --show-paths did not run." >&2; exit 1; }
else
  SHOWN="$(cd "$OUT" && env -i HOME="$PWD/$SCRATCH" XDG_CONFIG_HOME="$PWD/$SCRATCH/config" \
    XDG_DATA_HOME="$PWD/$SCRATCH/data" XDG_CACHE_HOME="$PWD/$SCRATCH/cache" PATH=/usr/bin:/bin \
    "./$PROBE" --show-paths 2>&1)" || {
    echo "dist-portable: $PROBE --show-paths did not run." >&2; exit 1; }
fi
SHOWN="$(printf '%s\n' "$SHOWN" | tr '\\' '/' | tr -d '\r')"

fail=0
printf '%s\n' "$SHOWN" | grep -q '^portable  *yes' || {
  echo "dist-portable: the machine does not read $MARKER as the marker." >&2; fail=1; }
for line in settings catalog packages; do
  printf '%s\n' "$SHOWN" | grep "^$line " | grep -q "/$NAME/$DATA/karaokemachine" || {
    echo "dist-portable: the machine keeps its $line outside $DATA/karaokemachine." >&2; fail=1; }
done
if [ -n "$(find "$SCRATCH" -type f 2>/dev/null)" ]; then
  echo "dist-portable: the machine wrote into the home directory:" >&2
  find "$SCRATCH" -type f >&2
  fail=1
fi
rm -rf "$SCRATCH"
# The archive carries the five files written above and nothing a run left behind. A live request
# among them would make every copy download a bank nobody asked for, and a live settings file would
# replace the one in a folder it is unpacked over.
LEFT="$(cd "$OUT/$DATA" && find . -type f | LC_ALL=C sort | tr '\n' ' ')"
WANT="./karaokemachine/$REQUEST.example ./karaokemachine/packages/README.txt ./karaokemachine/$SETTINGS_EXAMPLE ./karaokemachine/soundfonts/README.txt ./karaokemachine/wallpapers/README.txt "
if [ "$LEFT" != "$WANT" ]; then
  echo "dist-portable: $OUT/$DATA does not hold exactly its five files: $LEFT" >&2
  fail=1
fi
if [ "$fail" -ne 0 ]; then
  printf '%s\n' "$SHOWN" >&2
  exit 1
fi
echo "verified: the machine keeps its settings, catalog and packages inside the folder."

# -- the archive -------------------------------------------------------------------------------------

bytes="$(dist_bytes "$OUT")"
printf 'staged %s\n' "$OUT"
printf '  %s bytes total (~%s MiB)\n' "$bytes" "$((bytes / 1024 / 1024))"

if [ "$PLATFORM" = windows ]; then
  rm -f "$PARENT/$NAME.zip"
  dist_zip "$PARENT" "$NAME"
else
  # The flags the machine's own tarball takes, so two runs over one build give one archive.
  tar -C "$PARENT" --owner=0 --group=0 --numeric-owner --sort=name -czf "$PARENT/$NAME.tar.gz" "$NAME"
  printf 'wrote %s\n' "$PARENT/$NAME.tar.gz"
fi
