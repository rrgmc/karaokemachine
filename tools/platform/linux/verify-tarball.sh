#!/usr/bin/env bash
#
# Unpacks the portable tarball in a clean container and proves it runs there.
#
#   tools/platform/linux/verify-tarball.sh                    # the video tarball, on debian:13-slim
#   tools/platform/linux/verify-tarball.sh --no-video         # the other one
#   tools/platform/linux/verify-tarball.sh --image fedora:42  # somewhere that is not Debian at all
#   tools/platform/linux/verify-tarball.sh --bare             # install nothing first; see what it needs
#   tools/platform/linux/verify-tarball.sh --no-prewarm       # install at run time, as this always used to
#   tools/platform/linux/verify-tarball.sh --refresh          # rebuild the prewarmed image against the archive
#
# **A clean container, not the build image**, for the reason tools/platform/linux/verify-deb.sh gives about
# the package: the build image has the whole toolchain and every development library in it, so a
# tarball missing something would run there and fail on a real machine. This one starts from a base
# image with nothing in it and installs only the runtime libraries README.txt tells a user to
# install -- so a pass means that list is both correct and complete, which is the part of a tarball
# that has no dpkg to check it.
#
# **--image is the whole point of the exercise.** The .deb has Depends, and dpkg refuses to install
# it where they are not met; a tarball has a paragraph of prose in a README, and prose is not
# checked by anything. Running this against a distribution from another family is how that paragraph
# stops being a guess -- which matters most for the two lines nobody here can test by reading, the
# Fedora and Arch package names. Each family's list lives in the case statement below, beside the
# others, so the README and this script can be compared line by line.
#
# The images are not pulled unless you ask for them; the default costs nothing beyond what deb.sh
# already fetched.
#
# **The runtime libraries are installed once, into a cached image, not on every run.** That install
# was the dominant cost of this script -- 17 packages including libgl1-mesa-dri, and on Windows the
# unpack onto Docker's overlay stalls for minutes. tools/platform/linux/verify-image.sh builds a derivative of
# the base image with exactly the list in it, tagged by a hash of the generated Dockerfile, so a
# changed list or a changed base cannot silently reuse a stale one.
#
# **Does that weaken what this proves? No, provided four things, and each is enforced rather than
# hoped for.** The assertion is: *on a stock <distro> with exactly the README's list installed and
# nothing else, the tarball resolves, runs, finds its own assets and ffmpeg, and installs and removes
# a menu entry.* Prewarming moves the "with exactly the list installed" clause from run time to image
# build time, and the predicate is unchanged as long as it is the same list (one source of truth in
# runtime-deps.sh, and it is hashed into the tag), the same base (also hashed), the same install
# command character for character (generated, not retyped -- note in particular that the Debian one
# still has no `--no-install-recommends`, exactly as it always has), and nothing else in the image
# (no package name is written anywhere in verify-image.sh).
#
# **What is genuinely lost is freshness**, and that is worth stating plainly. A run-time install
# resolves against the archive today; the image resolved against it on the day it was built. A
# package renamed or dropped upstream stops being caught until the image is rebuilt. So the header
# prints the image's age on every run, `--refresh` rebuilds it, and `--no-prewarm` goes back to
# installing at run time. Use `--refresh` before tagging a release. `--no-prewarm` also exists to
# keep the run-time path exercised -- a branch nothing ever takes is a branch that has quietly
# stopped working.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=verify

# The one place the three families' package lists live, shared with the README generator in
# tarball-in-container.sh, and the prewarmed-image machinery that installs them.
. tools/platform/linux/runtime-deps.sh
. tools/platform/linux/verify-image.sh

VIDEO=1
IMAGE="debian:13-slim"
BARE=0
PREWARM=1
REFRESH=""

while [ $# -gt 0 ]; do
  case "$1" in
    --no-video) VIDEO=0 ;;
    --bare) BARE=1 ;;
    --no-prewarm) PREWARM=0 ;;
    --refresh) REFRESH="--refresh" ;;
    --image) shift; IMAGE="${1:?--image needs an image name}" ;;
    *) echo "verify: unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

export MSYS2_ARG_CONV_EXCL='*'

if ! docker version >/dev/null 2>&1; then
  echo "verify: cannot reach the Docker daemon." >&2
  echo "        On Windows, start Docker Desktop and wait for it to say it is running." >&2
  exit 1
fi

OUT="$(dist_dir karaokemachine linux)"

# Newest wins, the same rule every consumer of dist/ here uses. The two builds differ in the name, so
# the pattern picks the right one rather than needing a subfolder.
if [ "$VIDEO" -eq 1 ]; then
  tar="$(ls -t "$OUT"/karaokemachine-*.tar.gz 2>/dev/null | grep -v -- '-no-video' | head -1 || true)"
else
  tar="$(ls -t "$OUT"/karaokemachine-*-no-video.tar.gz 2>/dev/null | head -1 || true)"
fi

if [ -z "$tar" ]; then
  echo "verify: no$( [ "$VIDEO" -eq 0 ] && printf ' --no-video' ) tarball in $OUT" >&2
  echo "        build one with: tools/platform/linux/tarball.sh$( [ "$VIDEO" -eq 0 ] && printf ' --no-video' )" >&2
  exit 1
fi

# **--bare must never be handed a prewarmed image**, and the guard is that `family` is forced empty
# rather than that some later branch remembers to check. --bare exists to prove what the tarball
# needs from a bare system -- running it against an image with the list already baked in would invert
# its purpose and turn a real check into a tautology that always passes. There must be no code path
# on which BARE=1 and a derivative image coexist, so the two are made mutually exclusive here, once.
#
# An unrecognized `--image` also yields an empty family, and that is a fast path missing rather than
# an error: --image is the whole point of this script, so an image verify_image_family has never
# heard of has to keep working exactly as it did before prewarming existed.
family=""
RUN_IMAGE="$IMAGE"
PREWARMED=0
if [ "$BARE" -eq 0 ] && [ "$PREWARM" -eq 1 ]; then
  family="$(verify_image_family "$IMAGE")"
fi
if [ -n "$family" ]; then
  RUN_IMAGE="$(verify_image_ensure "$family" "$IMAGE" "$REFRESH")" || exit 1
  PREWARMED=1
fi

# How old the prewarmed image is, said out loud on every run. Staleness is the one thing prewarming
# costs, so it should be visible rather than something you have to think to ask about.
image_age() {
  local created
  created="$(docker image inspect -f '{{.Created}}' "$1" 2>/dev/null)" || return 1
  local secs=$(( $(date +%s) - $(date -d "$created" +%s 2>/dev/null || echo 0) ))
  if   [ "$secs" -lt 172800 ];  then printf 'built %dh ago' $(( secs / 3600 ))
  else printf 'built %dd ago' $(( secs / 86400 )); fi
}

echo "== $(basename "$tar")"
if [ "$PREWARMED" -eq 1 ]; then
  echo "   image   $RUN_IMAGE"
  echo "           $IMAGE plus the README's list, $(image_age "$RUN_IMAGE")"
  echo "   deps    already in the image (--no-prewarm to install at run time)"
else
  echo "   image   $IMAGE"
  echo "   deps    $( [ "$BARE" -eq 1 ] && printf 'none (--bare)' || printf "the README's list, installed at run time" )"
fi
echo

# The three install commands, built on the host from tools/platform/linux/runtime-deps.sh -- the same file the
# README generator and the prewarmed images read, so none of them can describe a different list from
# the others. ffmpeg is deliberately absent from all three because the tarball carries its own, and
# so is a font, for the same reason. Deriving them from crates/machine/karaokemachine/Cargo.toml's `depends` instead
# would not work: that field is Debian's, has $auto in it, and two of these three families do not use
# Debian's names for anything.
#
# All three are passed in and the *container* chooses between them by which package manager it has,
# rather than the host deciding. That is what keeps an unrecognized `--image` working: the host
# cannot classify it, but the image can still say what it is.
export KM_INSTALL_DEBIAN="$(runtime_deps_install_cmd debian)"
export KM_INSTALL_FEDORA="$(runtime_deps_install_cmd fedora)"
export KM_INSTALL_ARCH="$(runtime_deps_install_cmd arch)"

REMOTE=$(cat <<'SCRIPT'
set -eu

# This script used to arrive on stdin, and apt-get's debconf read it -- reporting "<STDIN> line 51",
# which was a line of this file it had just swallowed. It survived one run on block-buffering luck
# and hung the next. The fix is at the `docker run` below, where the script is now passed as an
# argument instead; nothing here has to redirect anything. DEBIAN_FRONTEND stays because a container
# has no tty and debconf should not go looking for one.
export DEBIAN_FRONTEND=noninteractive

if [ "${KM_BARE:-0}" = "1" ]; then
  echo "-- installing nothing (--bare) --"
elif [ "${KM_PREWARMED:-0}" = "1" ]; then
  # The image already carries exactly this list, installed by exactly this command -- both come from
  # the same runtime-deps.sh, and the tag is a hash of the generated Dockerfile, so a stale image
  # cannot be silently reused. See the header for why this does not weaken the check.
  echo "-- the README's list is already in this image --"
elif command -v apt-get >/dev/null 2>&1; then
  echo "-- apt-get: the Debian/Ubuntu list --"
  eval "$KM_INSTALL_DEBIAN"
elif command -v dnf >/dev/null 2>&1; then
  echo "-- dnf: the Fedora list --"
  eval "$KM_INSTALL_FEDORA"
elif command -v pacman >/dev/null 2>&1; then
  echo "-- pacman: the Arch list --"
  eval "$KM_INSTALL_ARCH"
else
  echo "verify: this image has none of apt-get, dnf or pacman; nothing installed" >&2
fi

echo
echo "-- glibc --"
(ldd --version 2>/dev/null || true) | head -1

tarball=$(ls /in/*.tar.gz | head -1)
mkdir -p /tmp/km
tar -xzf "$tarball" -C /tmp/km
dir=$(ls -d /tmp/km/*/ | head -1)
cd "$dir"
echo
echo "-- unpacked into $dir --"
ls

# 1. Does it resolve? This is the failure a user would meet first, and `ldd` names what is missing
#    where the loader would only name the first one.
echo
echo "-- what it links, and from where --"
if ldd ./karaokemachine | grep -i 'not found'; then
  echo "verify: the binary has unresolved libraries in this image." >&2
  exit 1
fi
ldd ./karaokemachine | sed 's/^[[:space:]]*//' | sort

# 2. Is the bundled ffmpeg the one it actually uses? A host with its own ffmpeg installed would
#    otherwise hide a broken RPATH.
if [ -d lib ]; then
  echo
  echo "-- ffmpeg --"
  ldd ./karaokemachine | grep libav || true
  if ! ldd ./karaokemachine | grep -q "$dir"; then
    echo "verify: nothing resolves inside the unpacked folder; RPATH is not doing its job." >&2
    exit 1
  fi
  echo "the libav* above resolve inside the folder, not to anything this image has."
fi

# 3. Does it run, and does it find its own assets? --show-paths prints and exits, which makes it the
#    cheapest proof that the process got as far as running its own code.
echo
echo "-- --version --"
./karaokemachine --version

echo
echo "-- --show-paths --"
./karaokemachine --show-paths

assets=$(./karaokemachine --show-paths | awk '/^assets/ {print $2}')
if [ "$assets" != "${dir%/}/assets" ]; then
  echo "verify: assets resolved to '$assets', not to the folder the binary is in." >&2
  exit 1
fi
echo
echo "assets resolve beside the binary, as discover_asset_dir intends."

for f in assets/fonts/karaoke.ttf; do
  [ -f "$f" ] || { echo "verify: $f is missing; on a distribution with no DejaVu at a Debian path the screen stays black." >&2; exit 1; }
done
echo "a font is bundled, so this needs no font package installed."

if ! ls assets/soundfont/*.sf2 >/dev/null 2>&1; then
  echo "verify: no SoundFont; the machine would come up on its sine test tone." >&2
  exit 1
fi
echo "an instrument bank is bundled."

# 4. Does it get through a start? Reported rather than asserted -- a container has no sound device
#    and no display, so the interesting outcomes here are the ones it *does* reach.
echo
echo "-- a headless start (8s) --"
# No RUST_LOG, for the reason verify-deb.sh gives: every line grepped for below is info or warn, so
# the shipped default finds them all, and pinning a filter here would test a configuration nobody
# runs -- which matters more since the default became plain `info`.
timeout 8 ./karaokemachine --headless --data-dir /tmp/kmdata 2>&1 \
  | grep -E "SoundFont:|no SoundFont|no audio output|the API is listening|wallpaper|video" \
  | head -12 || true

echo
echo "-- install.sh, for this user --"
share=/tmp/kmhome/.local/share
entry=$share/applications/karaokemachine.desktop
mime=$share/mime/packages/karaokemachine-package.xml
icon=$share/icons/hicolor/256x256/apps/karaokemachine.png
# The badged mark the entry's stream action names. Checked beside the one above rather than trusted
# to it, because the two are separate lists in register.rs and the way that fails is one-sided: a
# mark written and never removed stays in every application menu after an uninstall.
stream_icon=$share/icons/hicolor/256x256/apps/karaokemachine-stream.png

HOME=/tmp/kmhome XDG_DATA_HOME=$share ./install.sh
test -f "$entry"
grep -q "^Exec=$dir" "$entry" \
  || { echo "verify: install.sh did not rewrite Exec to an absolute path." >&2; exit 1; }
# Carried from the file the .deb installs rather than written out again, which is what stops the
# two disagreeing. StartupWMClass is the line a hand-written copy loses first, and losing it is a
# running window with a generic icon that nothing connects to this entry.
grep -q "^StartupWMClass=karaokemachine" "$entry" \
  || { echo "verify: the entry is not the shipped one; something is writing its own." >&2; exit 1; }
# The half that is easy to leave out, and its absence is silent: without it a file manager has no
# name, no icon and no default application for a .kmpkg, and never reads the entry's MimeType line.
test -f "$mime" \
  || { echo "verify: no MIME definition, so .kmpkg is a type the desktop has never heard of." >&2; exit 1; }
grep -q 'application/x-km-package' "$mime" \
  || { echo "verify: the MIME definition does not declare the type the entry claims." >&2; exit 1; }
test -f "$icon" \
  || { echo "verify: the icon theme entries were not written." >&2; exit 1; }
test -f "$stream_icon" \
  || { echo "verify: the stream action's mark was not written, so it draws the machine's." >&2; exit 1; }
grep -q "^Icon=karaokemachine-stream" "$entry" \
  || { echo "verify: the stream action does not name its own mark." >&2; exit 1; }

HOME=/tmp/kmhome XDG_DATA_HOME=$share ./install.sh --uninstall
test ! -f "$entry"
test ! -f "$mime"
test ! -f "$icon"
test ! -f "$stream_icon"
echo "the entry, the type and the icons were written for this user, and removed again."

echo
echo "-- verified on this image --"
SCRIPT
)

# Mounted read-only: the verification must not be able to repair what it is verifying.
#
# The script goes in as an **argument**, not down stdin, and `-i` is gone with it. That is the same
# fix tools/platform/linux/verify-deb.sh carries and it is a bug fix rather than a style choice: piped, the
# script *is* the container's stdin, the shell reads it as it runs, and any command that reads stdin
# eats the rest of it. `apt-get` does exactly that. Passing it as an argument leaves stdin free for
# whatever wants it, so no command in the remote half has to remember to redirect.
docker run --rm \
  -e "KM_BARE=$BARE" \
  -e "KM_PREWARMED=$PREWARMED" \
  -e "KM_INSTALL_DEBIAN=$KM_INSTALL_DEBIAN" \
  -e "KM_INSTALL_FEDORA=$KM_INSTALL_FEDORA" \
  -e "KM_INSTALL_ARCH=$KM_INSTALL_ARCH" \
  -v "$(host_path "$PWD/$OUT")":/in:ro \
  "$RUN_IMAGE" sh -c "$REMOTE" verify-tarball
