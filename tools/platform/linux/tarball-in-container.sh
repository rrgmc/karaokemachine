#!/usr/bin/env bash
#
# The half of tools/platform/linux/tarball.sh that runs inside the build image. Not meant to be run directly.
#
# Expects /src (the source tree), /build (the cargo target dir and the ffmpeg prefix) and /out (where
# the folder and the tarball go). KM_VIDEO selects the optional video feature; tarball.sh always sets
# it, and the default here is 0 for the same reason deb-in-container.sh gives -- an unset variable means
# somebody is running this by hand, and the smaller build is the safer thing to give them.
#
# The staged layout is the Windows one, and that is the point:
#
#   karaokemachine-<version>-<triple>/
#     karaokemachine            the binary, RPATH $ORIGIN/lib
#     lib/                      ffmpeg and openh264, RPATH $ORIGIN          (video build only)
#     assets/                   soundfont, wallpapers, and a font
#     install.sh                menus and the .kmpkg type for this user; --uninstall takes it out
#     README.txt
#     LICENSES/
#
# `Paths::discover_asset_dir` takes the directory beside the executable when it has an `assets`
# child, so this needs no code change -- it is the layout that function was written for, which is
# what "Shipping a desktop build" in docs/ARCHITECTURE.md says about the Windows folder. Everything
# sits at the top level rather than under bin/ and lib/ for that one reason: put the binary in bin/
# and the assets have to follow it there, which is a stranger tree than this one.

set -euo pipefail

cd /src

. tools/dist/common.sh
DIST_SCRIPT=tarball

# The ffmpeg pin, for the license note staged below: the release it points at and the exact configure
# line, taken from the one file that decides them rather than retyped into a heredoc where nobody
# would notice them going stale.
. tools/setup/ffmpeg-pin.sh

# The runtime dependency lists README.txt tells the user to install, taken from the one file that
# also drives tools/platform/linux/verify-tarball.sh -- so the README cannot promise a list the verifier does
# not actually test.
. tools/platform/linux/runtime-deps.sh

VIDEO="${KM_VIDEO:-0}"

# **The prefix carries the build's identity**, so this volume can hold more than one and every
# checkout sharing it gets the right one. `$FF_SRC_ID` is a hash of the release and the configure
# flags, from the pin sourced above: the same pin resolves to the same directory and is reused, a
# different pin resolves to a different one, and neither run has any reason to delete the other's.
# One fixed path plus a stamp is what makes two worktrees on different pins delete and rebuild each
# other's ffmpeg in turn -- see the header of tools/platform/linux/ffmpeg-lgpl.sh.
FFPREFIX="/build/ffmpeg-lgpl/$FF_SRC_ID"

# -- the assets ----------------------------------------------------------------------------------

# The same release-step rule as everywhere else: the tarball ships the instrument bank because the
# build put it there, not because somebody remembered to fetch it.
if ! ls assets/soundfont/*.sf2 >/dev/null 2>&1; then
  dist_step "assets (none found; fetching)"
  dist_run "fetch-assets" tools/setup/fetch-assets.sh
  echo
fi

# -- ffmpeg --------------------------------------------------------------------------------------

# Built rather than borrowed, and the whole argument is in tools/platform/linux/ffmpeg-lgpl.sh: Debian's is
# GPL-2+ and this workspace is MIT OR Apache-2.0.
#
# PKG_CONFIG_PATH points ffmpeg-sys-next at that prefix, so the binary is compiled against exactly
# the headers of the libraries it will be shipped with. Building against Debian's and shipping ours
# would *probably* work -- same upstream version, same soname -- but "probably" is doing real work in
# that sentence, and the failure would be an undefined symbol at load time on somebody else's
# machine.
FEATURES=()
if [ "$VIDEO" -eq 1 ]; then
  tools/platform/linux/ffmpeg-lgpl.sh "$FFPREFIX"
  export PKG_CONFIG_PATH="$FFPREFIX/lib/pkgconfig"
  FEATURES+=(--features video)
  echo
fi

# -- the build -----------------------------------------------------------------------------------

# **The workspace's own crates are cleaned first, and that is not paranoia**: the shared build volume
# cannot tell two checkouts apart, and a release artifact built from another one's source is a failure
# nothing downstream would catch. The whole argument, and the crate list, live in one place now --
# deb-in-container.sh does exactly the same before building the .deb.
tools/platform/linux/clean-checkout.sh

# --locked so a release build cannot silently resolve a different dependency tree than the one
# committed in Cargo.lock, which is the same reason deb-in-container.sh passes it to cargo-deb.
dist_step "build$( [ "$VIDEO" -eq 1 ] && printf ' (video)' )"
build_started=$SECONDS
# shellcheck disable=SC2046  # dist_cargo_quiet prints one flag or nothing at all
cargo build --release --locked $(dist_cargo_quiet) -p karaokemachine "${FEATURES[@]+"${FEATURES[@]}"}"
printf '   built in %s\n' "$(dist_elapsed "$build_started")"

# Named explicitly, which is what keeps `karaokemachine-console` out of the tarball. That binary is
# built on every platform and wanted on one: Linux has no subsystem to choose, so it is the same
# program under a second name here. See the `The machine's console window` decision in docs/decisions/.
EXE="${CARGO_TARGET_DIR:-/build/target}/release/karaokemachine"
if [ ! -x "$EXE" ]; then
  echo "tarball: the build produced no $EXE" >&2
  exit 1
fi

# -- the folder ----------------------------------------------------------------------------------

# The version comes out of the binary, which for a video build has to be able to find its libraries
# to start at all -- they are not staged yet, so this is the one moment LD_LIBRARY_PATH is legitimate.
# It is unset again immediately, because everything after this point is meant to prove the staged
# tree stands on its own and a leftover variable would prove it while helping.
if [ "$VIDEO" -eq 1 ]; then
  export LD_LIBRARY_PATH="$FFPREFIX/lib"
fi
VERSION="$(dist_version "$EXE")"
unset LD_LIBRARY_PATH

TARGET="$(dist_host_triple)"
NAME="karaokemachine-$VERSION-$TARGET"
# The marker goes on the declined build, exactly as it does in the Windows folder name: the plain
# name should name what the plain command produces. Unlike the .deb this needs no subfolder, because
# the name is ours to choose and two tarballs can differ in it.
if [ "$VIDEO" -eq 0 ]; then
  NAME="$NAME-no-video"
fi
DEST="/out/$NAME"

echo
dist_step "stage $NAME"
dist_clear "$DEST"

cp "$EXE" "$DEST/karaokemachine"
chmod 755 "$DEST/karaokemachine"
# cargo-deb strips the binary itself, so the .deb has always shipped a stripped one; nothing does
# that here. It is worth about two thirds of the file.
strip --strip-unneeded "$DEST/karaokemachine"

# -- the libraries -------------------------------------------------------------------------------

libs=0
if [ "$VIDEO" -eq 1 ]; then
  mkdir -p "$DEST/lib"
  # Copied under their sonames as real files rather than as the versioned file plus a symlink chain.
  # DT_NEEDED records the soname and nothing looks for the longer name, so the chain would be two
  # more things in the tarball that only exist to point at the third -- and a symlink is the one kind
  # of file an unpacking tool on a strange filesystem can get wrong.
  for lib in avcodec avformat avutil swresample; do
    real="$(readlink -f "$FFPREFIX/lib/lib$lib.so")"
    soname="$(patchelf --print-soname "$real")"
    cp "$real" "$DEST/lib/$soname"
    chmod 644 "$DEST/lib/$soname"
    libs=$((libs + 1))
  done

  # libopenh264, which libavcodec links for the one thing ffmpeg cannot do itself: encode H.264.
  # Taken by soname from the prefix rather than from the system, because ffmpeg-lgpl.sh put it
  # there and that is what makes the prefix the single description of what a video build carries.
  # Named separately because it is the one library here whose name is not libav*.
  for real in "$FFPREFIX"/lib/libopenh264.so.*; do
    soname="$(patchelf --print-soname "$real")"
    cp "$real" "$DEST/lib/$soname"
    chmod 644 "$DEST/lib/$soname"
    libs=$((libs + 1))
  done

  # $ORIGIN is resolved by the loader against the object's own directory, so both of these are
  # relative and the folder can be unpacked anywhere. Two separate rewrites because RUNPATH -- which
  # is what a modern linker emits -- is *not* inherited by an object's own dependencies: the binary
  # finding lib/ says nothing about libavformat finding libavcodec beside it. The alternative is the
  # deprecated DT_RPATH, which is inherited, or a launcher script setting LD_LIBRARY_PATH; this is
  # the version with no wrapper and nothing deprecated in it.
  patchelf --set-rpath '$ORIGIN/lib' "$DEST/karaokemachine"
  for so in "$DEST"/lib/*.so.*; do
    patchelf --set-rpath '$ORIGIN' "$so"
  done
fi

# -- the assets ----------------------------------------------------------------------------------

assets="$(dist_stage_assets "$DEST")"

# The font, which the .deb gets by naming fonts-dejavu-core in Depends and a tarball cannot. Without
# it km-display walks a hard-coded list of system paths (crates/playback/km-display/src/text.rs) that is
# Debian- and Arch-shaped: on Fedora, DejaVu is at /usr/share/fonts/dejavu-sans-fonts/ and none of
# the four candidates match, so `find_font` returns None, km-app treats a display that will not start
# as non-fatal, and the television stays black with the reason in the log. `fonts/karaoke.ttf` is the
# bundled path km-app already passes as `bundled_font` -- so this is the file the code has been
# looking for since M4, finally present.
#
# `mkdir` first because the directory will not be there: `assets/fonts/` in the repository holds
# nothing but a `.gitkeep`, which dist_stage_assets skips by design, so nothing has created it.
mkdir -p "$DEST/assets/fonts"
cp /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf "$DEST/assets/fonts/karaoke.ttf"
chmod 644 "$DEST/assets/fonts/karaoke.ttf"
assets=$((assets + 1))

# -- desktop integration -------------------------------------------------------------------------
#
# Nothing is staged for it. The desktop entry, the type definition and the seven icons are compiled
# into the binary -- `register.rs` embeds the same files this repository gives the `.deb` -- so
# install.sh has nothing to copy and the folder has no `share/` for a move to leave behind.
# Unpacking still writes nothing outside the folder: `--register` is what writes, and running it is
# the user's choice.

# -- licenses ------------------------------------------------------------------------------------

mkdir -p "$DEST/LICENSES"
# The application's own terms, carried rather than merely named. README.txt below has always said
# "MIT OR Apache-2.0. See LICENSES/", and until these two files existed in the tree that sentence
# pointed at a folder holding everybody's license but ours.
dist_stage_app_licenses "$DEST/LICENSES" karaokemachine-
if [ "$VIDEO" -eq 1 ]; then
  # Both written by ffmpeg-lgpl.sh into the prefix, so the license text and the version cannot drift
  # from the libraries they describe.
  ffversion="$FF_SRC_VER"
  cp "$FFPREFIX/COPYING.LGPLv2.1" "$DEST/LICENSES/ffmpeg-COPYING.LGPLv2.1.txt"
  cat > "$DEST/LICENSES/ffmpeg-README.txt" <<LGPL
The four libav* libraries in ../lib are FFmpeg, used under the GNU Lesser General
Public License version 2.1 or later. The full text is beside this file.
libopenh264.so beside them is not FFmpeg and has terms of its own, in
openh264-LICENSE.txt.

They are not the FFmpeg your distribution ships. This is a build made from the
unmodified FFmpeg $ffversion release, configured with GPL components switched off
and autodetection of external libraries disabled:

$(printf '    %s\n' "${FF_SRC_CONFIGURE[@]}")

so it contains no x264 and no x265 -- every decoder in it is one FFmpeg
implements itself. Two external libraries are asked for by name: zlib, which
several demuxers need, and openh264, which is the H.264 encoder a machine
streaming its screen uses and the only one an LGPL build may have. The same line
builds the FFmpeg inside this project's macOS release; it lives in
tools/setup/ffmpeg-pin.sh, with the source URL and its checksum.

Your LGPL rights, concretely: the source is the unmodified release at

    $FF_SRC_URL

and because these libraries are dynamically linked and sit in a directory of
their own, replacing them with your own build of FFmpeg 7.1 is a matter of
overwriting the files in ../lib. Nothing here is statically linked to them.
LGPL
  cp "$FFPREFIX/openh264-COPYRIGHT" "$DEST/LICENSES/openh264-LICENSE.txt"
fi
cat > "$DEST/LICENSES/font-README.txt" <<'FONT'
assets/fonts/karaoke.ttf is DejaVu Sans, unmodified and renamed. DejaVu is in the
public domain in part and licensed under the Bitstream Vera Fonts Copyright in
part; both permit redistribution, with or without modification, including as part
of a software distribution.

It is bundled because this is a tarball: the Debian package of this application
names fonts-dejavu-core as a dependency instead, and a folder cannot name
anything. Setting `display.font` in settings.json to any other TTF overrides it.
FONT

# -- install.sh ----------------------------------------------------------------------------------

cat > "$DEST/install.sh" <<'INSTALL'
#!/bin/sh
#
# Puts this folder in the desktop's menus and makes .kmpkg files open with it -- for this user,
# without root, and without moving anything. The application keeps running from where you unpacked
# it; what this writes is a desktop entry pointing at it, the icons a menu draws beside the name,
# and the definition that tells the desktop what a song package is.
#
#   ./install.sh              add the menu entry, the icons and the file type
#   ./install.sh --uninstall  take them away again; the folder is untouched
#
# There is no system-wide mode on purpose. Installing to /usr or /opt as root is what the .deb is
# for, and it does it properly -- with a symlink in /usr/bin, a systemd unit and an uninstall that
# dpkg remembers. A tarball that wrote into /usr would be a package manager with no records.
#
# The work is the machine's own --register, which this passes straight to. Doing it here as well
# would be a second copy of the desktop entry and the type definition, and the copy that lived here
# had one of the two: it wrote the entry claiming the type without ever declaring the type, so a
# file manager had no idea what a .kmpkg was and never read the line that would have told it.

set -eu

here=$(cd "$(dirname "$0")" && pwd)

if [ ! -x "$here/karaokemachine" ]; then
  echo "install: $here/karaokemachine is missing or not executable" >&2
  exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
  "$here/karaokemachine" --unregister
  echo
  echo "The folder itself is still here. Delete it to finish, and note that your songs and"
  echo "settings are not in it -- run ./karaokemachine --show-paths to see where they are."
  exit 0
fi

"$here/karaokemachine" --register

echo
echo "KaraokeMachine should now be in your menus, and a .kmpkg should open with it. If it is"
echo "not there, log out and back in -- some desktops only read that directory at session start."
INSTALL
chmod 755 "$DEST/install.sh"

# -- README.txt ----------------------------------------------------------------------------------

{
  printf 'KaraokeMachine %s\n' "$VERSION"
  printf '================%s\n\n' "$(printf '%*s' ${#VERSION} '' | tr ' ' '=')"
  if [ "$VIDEO" -eq 1 ]; then
    printf 'A portable build for Linux, with video songs. Unpack it anywhere and run it; nothing\n'
    printf 'here has to be installed and nothing writes outside this folder until you play something.\n\n'
  else
    printf 'A portable build for Linux, without the video feature -- MIDI and KAR songs only. Unpack\n'
    printf 'it anywhere and run it; nothing here has to be installed.\n\n'
  fi
} > "$DEST/README.txt"

cat >> "$DEST/README.txt" <<'README'
Running it
----------

    ./karaokemachine                    the machine, in a window
    ./karaokemachine --fullscreen       ...filling the screen, for this run
    ./karaokemachine --show-paths       where it keeps settings, songs and assets
    ./karaokemachine --headless         no window: the API and the engine only
    ./karaokemachine --set-password s   set the admin password (or POST /api/v1/admin/password)
    ./karaokemachine --song-book b.pdf  every installed song, as a PDF to print
    ./karaokemachine --song-book b.pdf --book-name "Sitting room"   ...with a name on it

F makes it fullscreen and takes it back out, Esc leaves fullscreen or quits. To start fullscreen
every time, set "fullscreen": true under "display" in settings.json, which --show-paths locates. The
.deb does that for you; a folder like this one cannot, because it does not know whether it is the
machine under your television or a copy you are trying out.

The screen shows the address to point a phone at;
everything else -- searching, queueing, tone, skipping -- happens there, at http://<this machine>:8177.

The machine gives itself a six-digit password the first time it starts, and shows it on its own
screen beside the address. Anything that reconfigures the machine needs it -- adding and removing
songs, the sound output, the machine's name, the pictures -- and nothing a singer does needs
anything, which is the design: whoever is in the room can queue a song.

Change it at http://<this machine>:8177/admin/, or with --set-password. Until you do, that page
says so on every tab. The generated PIN is fine for a room and is not a secret from the internet,
so change it before this machine is reachable from outside your house -- and note that setting
api.bind to 127.0.0.1:8177 in settings.json shuts it in entirely.

Songs
-----

Songs come in packages built with km-pack. Drop a .kmpkg into the packages folder that
--show-paths names and restart; it is scanned at every start. A package is one file -- video and
MP3+G songs are inside it too -- so there is nothing beside it that can be left behind.

Your settings, catalog and packages live under your home directory, not in this folder, so
deleting this folder loses nothing but the application. --data-dir DIR moves all of it somewhere
else, which is how to keep a trial run out of the way.

Adding it to the menus
----------------------

    ./install.sh                a menu entry, icons, and .kmpkg files that open with this
    ./install.sh --uninstall    takes all of it away again

Double-clicking a package installs it once that has been run. The application still runs from this
folder; the entry points at it. Move the folder and you have to run install.sh again. If you would
rather have a proper system-wide install -- /usr/bin, a systemd unit for a machine that comes up as
an appliance under a television, and an uninstall the system remembers -- use the Debian package
instead of this tarball.

install.sh is a wrapper around the machine's own --register, which does the same job and can be run
directly. Neither needs root.

What this needs from your system
--------------------------------

Less than you might expect. SDL3, SDL3_ttf and SQLite are compiled into the binary, the instrument
bank and the wallpapers are in assets/, and a font is bundled too, so there is no package to install
for any of those. What is left is the graphics, input and sound libraries that SDL opens at run time,
which every desktop Linux install already has:

README

# The lists come from tools/platform/linux/runtime-deps.sh, which is also what verify-tarball.sh installs to
# prove this README is true. Writing them out here as well makes "the README and the verifier can be
# compared line by line" a claim discharged by a human comparing two heredocs.
# The heredoc is split around this call rather than made interpolating: the rest of the text is
# literal and should stay that way.
runtime_deps_readme_block >> "$DEST/README.txt"

cat >> "$DEST/README.txt" <<'README'

Every one of those lists was checked by installing exactly it into a clean container of that
distribution and starting this application there -- they are not transcribed from a wiki.

If one is missing the application does not start and says which library it could not find. It needs
glibc 2.38 or newer -- Debian 13, Ubuntu 24.04, Fedora 39 and anything more recent.

Licenses
--------

The application is MIT OR Apache-2.0, at your option; both texts are in LICENSES/. See there also for
what travels beside it: the instrument bank in assets/soundfont/ under the GeneralUser GS license,
DejaVu Sans as assets/fonts/karaoke.ttf, and -- in a video build -- FFmpeg under the LGPL v2.1, built
without any GPL component, with openh264 under the BSD 2-clause license beside it. LICENSES/ has the
details and, for FFmpeg, what your LGPL rights amount to in practice.
README

# -- the proof -----------------------------------------------------------------------------------

# The Linux half of the check tools/platform/windows/dist.sh does by starting the exe with ffmpeg off PATH:
# does this folder stand on its own? `env -i` is stronger than a stripped PATH -- no LD_LIBRARY_PATH,
# no LD_PRELOAD, nothing inherited from a shell that has the build's prefix in it. If the RPATH
# rewrites above had not taken, this is where it shows, here, rather than on somebody's Fedora box.
echo
echo "-- self-contained? --"
if ( cd "$DEST" && env -i HOME=/tmp PATH=/usr/bin:/bin ./karaokemachine --version >/dev/null 2>&1 ); then
  echo "yes: it starts with nothing inherited from this shell."
else
  echo >&2
  echo "tarball: the staged binary would not start in an empty environment." >&2
  echo "         Something it links is not in the folder; it will fail on another machine." >&2
  ( cd "$DEST" && env -i PATH=/usr/bin:/bin ldd ./karaokemachine | grep -i 'not found' >&2 ) || true
  exit 1
fi

# Asserted rather than eyeballed, the same way deb-in-container.sh asserts the .deb's Depends and for the
# same reason: a video tarball and a plain one differ in almost nothing you can see from outside.
# Both directions, because the unusual build is the one somebody asked for specially.
if [ "$VIDEO" -eq 1 ]; then
  # **Each of the three matches below captures its producer's output and searches it afterwards**,
  # which is what `dist_verify_macho_portable` in tools/dist/common.sh spells out and why: these
  # scripts run under `pipefail`, and a `grep -q` that matches kills its producer with SIGPIPE, so
  # the pipeline's 141 makes the `if` read a match as no match. Over a short `ldd` listing that is a
  # race the producer usually wins; over `strings` of a ten-megabyte library it loses every time.
  # The direction is the danger: every check here would wave through exactly what it exists to stop.
  loads="$( cd "$DEST" && env -i PATH=/usr/bin:/bin ldd ./karaokemachine | grep -F 'lib/libavcodec' || true )"
  if [ -n "$loads" ]; then
    echo "video: the binary resolves libavcodec out of ./lib, so it really is bundled."
  else
    echo >&2
    echo "tarball: KM_VIDEO=1 but the binary does not load libavcodec from ./lib." >&2
    echo "         Either the feature did not reach it or the RPATH did. Refusing to ship it." >&2
    exit 1
  fi
  # The license claim, checked rather than trusted. x264 and x265 are what make Debian's ffmpeg
  # GPL-2+, and a stray -dev package in the image plus a configure that quietly autodetected it is
  # the one plausible way they could appear here.
  gpl="$( cd "$DEST" && ldd lib/*.so.* 2>/dev/null | grep -E 'libx26[45]|librav1e|libjxl' || true )"
  if [ -n "$gpl" ]; then
    echo >&2
    echo "tarball: a bundled library links a GPL codec. This is not the build ffmpeg-lgpl.sh" >&2
    echo "         describes, and it cannot be redistributed with an Apache-2.0 binary." >&2
    exit 1
  fi
  echo "video: no GPL codec library is linked, so the LGPL claim in LICENSES/ holds."
  # The H.264 encoder, asserted the same way and for the same reason. `--enable-libopenh264` is one
  # flag in a shared list, and a configure that dropped it would leave a tarball that decodes and
  # plays perfectly and cannot stream -- discovered by whoever first ran --stream out of a folder,
  # as a machine that starts and then says it has no encoder.
  h264="$( cd "$DEST" && strings lib/libavcodec.so.* | grep -x libopenh264 || true )"
  if [ -n "$h264" ]; then
    echo "video: libavcodec carries the H.264 encoder, so --stream works out of this folder."
  else
    echo >&2
    echo "tarball: the bundled libavcodec has no libopenh264 encoder in it." >&2
    echo "         --stream would start and then fail. Refusing to ship it." >&2
    exit 1
  fi
else
  if [ -d "$DEST/lib" ]; then
    echo >&2
    echo "tarball: KM_VIDEO=0 but a lib/ folder was staged. Refusing to ship it." >&2
    exit 1
  fi
  echo "no video: nothing is bundled and nothing links ffmpeg."
fi

# -- the tarball ---------------------------------------------------------------------------------

echo
dist_step tar
rm -f "/out/$NAME.tar.gz"
# --owner/--group so the archive does not carry this container's root, --sort=name so two builds of
# the same tree produce the same bytes in the same order.
tar -czf "/out/$NAME.tar.gz" -C /out \
    --owner=0 --group=0 --numeric-owner --sort=name \
    "$NAME"

# -- report --------------------------------------------------------------------------------------

folder_bytes="$(dist_bytes "$DEST")"
tar_bytes="$(wc -c < "/out/$NAME.tar.gz" | tr -d ' ')"

echo
echo "-- $NAME --"
printf '   version   %s\n' "$VERSION"
printf '   target    %s\n' "$TARGET"
printf '   assets    %s files\n' "$assets"
printf '   ffmpeg    %s libraries%s\n' "$libs" "$( [ "$libs" -eq 0 ] && printf ' (no video)' || printf ' in lib/, LGPL and BSD' )"
printf '   folder    %s bytes (~%s MiB)\n' "$folder_bytes" "$((folder_bytes / 1024 / 1024))"
printf '   tarball   %s bytes (~%s MiB)\n' "$tar_bytes" "$((tar_bytes / 1024 / 1024))"

if ! find "$DEST/assets" -name '*.sf2' | grep -q .; then
  echo
  echo "warning: no SoundFont in the staged assets. The machine will come up on its sine test" >&2
  echo "         tone, which sounds broken to anybody who does not know it is the fallback." >&2
fi

echo
echo "unpack with:  tar -xzf $NAME.tar.gz && cd $NAME && ./karaokemachine"
