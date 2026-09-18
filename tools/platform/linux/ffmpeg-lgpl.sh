#!/usr/bin/env bash
#
# Builds the ffmpeg the portable tarball bundles. Runs inside the build image; called by
# tools/platform/linux/tarball-in-container.sh, not by hand.
#
#   tools/platform/linux/ffmpeg-lgpl.sh <prefix>      # builds into <prefix> if it is not already there
#
# **Why this exists at all**, when the machine is developed against a perfectly good ffmpeg and the
# .deb simply names Debian's: because the .deb is the only Linux carrier that can *name* anything.
# A tarball is a folder somebody unpacks, so whatever it needs travels with it -- and Debian's
# libavcodec61 cannot travel. Two independent reasons, either of which is enough:
#
#   * **License.** Debian builds ffmpeg with --enable-gpl. /usr/share/doc/libavcodec61/copyright says
#     so in as many words: "For building the default Debian packages some of the GPL licensed files
#     are used, so the resulting binaries are licensed under GPL v2+." This workspace is
#     `MIT OR Apache-2.0`, and Apache-2.0 is not compatible with GPL-2. Putting those libraries in a
#     folder beside this binary would be shipping a combined work this project cannot redistribute.
#     The project already decided to develop against an LGPL ffmpeg for exactly this reason -- see
#     "Building km-video on this box" in CLAUDE.local.md, which chose BtbN's LGPL build over the GPL
#     one on the grounds that "we only ever decode, and shipping the DLLs beside the executable is
#     compliant under LGPL and would not be under GPL". This is the same sentence, on Linux.
#
#   * **Size, and what comes with it.** Debian's libavcodec61 pulls 93 shared libraries and 97 MiB
#     behind it -- x264, x265, rav1e, jxl, and then, because those want text and images, harfbuzz,
#     fontconfig, pango, cairo, gdk-pixbuf and glib. Bundling a second glib and a second fontconfig
#     underneath a host's own GL stack is the classic way to make an application that starts
#     everywhere except where somebody actually runs it. Measured, not guessed.
#
# What this builds instead is described once, in tools/setup/ffmpeg-pin.sh, which this shares with
# tools/setup/fetch-ffmpeg.sh -- the same release, the same checksum and the same configure line as the
# macOS bundle gets. In one sentence: **every decoder ffmpeg implements itself, and none that needs a
# third-party library.** The result links libc, libm and libz and nothing else, is LGPL-2.1+ with no
# GPL component, and is a fifth the size of Debian's.
#
# The version matches Debian trixie's on purpose, which is the same rule "Develop against the version
# the appliance has, not the newest" already states in docs/ARCHITECTURE.md: the tarball and the .deb
# should be the same machine, and a soname that differs between the two carriers would be a
# difference nobody asked for.

set -euo pipefail

# Both callers run from the repository root, so the shared pin is reachable by a relative path -- but
# say so explicitly rather than inheriting whatever the caller happened to be standing in.
cd "$(dirname "$0")/../../.."

# **Which release, which bytes and which flags are not decided here.** They are in
# tools/setup/ffmpeg-pin.sh, shared with tools/setup/fetch-ffmpeg.sh, which builds the same ffmpeg on macOS. What
# is decided here is everything about *this* environment: the prefix in the build volume, the stamp
# that makes it cacheable, and the check at the end that reads DT_NEEDED off the result.
. tools/setup/ffmpeg-pin.sh

PREFIX="${1:?usage: ffmpeg-lgpl.sh <prefix>}"

# **The cache is a fixed path, not one derived from the prefix.** A
# `$(dirname "$PREFIX")/ffmpeg-cache` follows a content-addressed prefix a level deeper, once per
# id. The downloaded source is the same bytes for every configuration of a given release, so it
# stays shared rather than duplicated.
CACHE="/build/ffmpeg-cache"

# **The prefix is content-addressed, and that is what makes this safe to share.**
#
# This runs in the `karaokemachine-deb-build` volume, which every checkout and every git worktree
# shares. The previous shape was one fixed prefix plus a stamp naming the version and flags: a run
# whose stamp disagreed did `rm -rf "$PREFIX"` and rebuilt. With one checkout that is right. With two
# it is destructive in both directions -- the delete happens with no lock while a peer container may
# be compiling against `$PREFIX/lib/pkgconfig` or running a binary with `LD_LIBRARY_PATH` pointed
# there -- and, because each checkout keeps judging the other's prefix stale, the two ping-pong
# through a multi-minute rebuild every time either one runs.
#
# So the caller passes a prefix that already names the build's identity (`$FF_SRC_ID`, from the shared
# pin). Two checkouts on the same pin share one directory, which is the caching this wants; two on
# different pins use different directories and neither can touch the other's. Nothing has to be
# deleted to be correct, so nothing is.
if [ -z "${FF_SRC_ID:-}" ]; then
  echo "ffmpeg-lgpl: no sha256 tool, so the build has no identity -- install coreutils" >&2
  exit 1
fi
case "$PREFIX" in
  */"$FF_SRC_ID") ;;
  # Not fatal for a hand-run, but say so: a prefix that does not carry the id is a prefix two
  # checkouts can collide in, which is the whole point of the change above.
  *) echo "ffmpeg-lgpl: warning -- prefix does not end in $FF_SRC_ID; it is not collision-safe" >&2 ;;
esac

# The caller's prefix is the only flag this adds to the shared list.
CONFIGURE=("--prefix=$PREFIX" "${FF_SRC_CONFIGURE[@]}")

# A marker rather than a stamp. The path already says *what* this is, so the only question left is
# whether the install finished -- an interrupted one must not be mistaken for a usable prefix. Written
# last, after the move below, so it can only exist on a complete tree.
DONE="$PREFIX/.km-complete"

if [ -f "$DONE" ]; then
  echo "-- ffmpeg $FF_SRC_VER (LGPL): cached at $PREFIX"
  exit 0
fi

echo "== ffmpeg $FF_SRC_VER (LGPL, native decoders only)"
echo "   this happens once per build volume; it takes a few minutes"

mkdir -p "$CACHE"
tarball="$CACHE/ffmpeg-$FF_SRC_VER.tar.xz"

if [ -f "$tarball" ] && [ "$(sha256sum "$tarball" | cut -d' ' -f1)" = "$FF_SRC_SHA256" ]; then
  echo "-- source: cached"
else
  echo "-- source: fetching $FF_SRC_URL"
  # Downloaded to a name of this process's own and moved into place only once the checksum passes.
  # A fixed `$tarball.part` was a second thing two concurrent runs wrote to at the same time; the
  # rename is atomic, so a peer either sees no tarball or sees a whole verified one.
  part="$tarball.part.$$"
  curl --proto '=https' --tlsv1.2 -sSfL -o "$part" "$FF_SRC_URL"
  got="$(sha256sum "$part" | cut -d' ' -f1)"
  if [ "$got" != "$FF_SRC_SHA256" ]; then
    echo "ffmpeg-lgpl: checksum mismatch for ffmpeg-$FF_SRC_VER.tar.xz" >&2
    echo "             expected $FF_SRC_SHA256" >&2
    echo "             got      $got" >&2
    rm -f "$part"
    exit 1
  fi
  mv "$part" "$tarball"
fi

# Unpacked into this run's own directory for the same reason: one fixed path for the extracted tree
# is a path a peer's `rm -rf` can remove mid-compile. It is scratch, so it is thrown away afterwards
# rather than cached -- the tarball is the thing to keep.
work="$CACHE/build.$$"
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
tar -xJf "$tarball" -C "$work"
src="$work/ffmpeg-$FF_SRC_VER"

# **Installed through DESTDIR and moved into place, rather than built straight into the prefix.**
# `--prefix` stays the final path so the .pc files and every baked-in path are right; DESTDIR just
# changes where `make install` puts the tree. The move is what makes the result appear all at once:
# a peer never sees a half-populated prefix, and there is no window in which the old one is gone and
# the new one is not there yet -- because nothing deletes an old one at all.
staged="$work/install"
mkdir -p "$(dirname "$PREFIX")"

(
  cd "$src"
  ./configure "${CONFIGURE[@]}" >configure.log 2>&1 || {
    echo "ffmpeg-lgpl: configure failed; last 40 lines of its log:" >&2
    tail -40 configure.log >&2
    exit 1
  }
  # **stderr too, which it was not.** `>/dev/null` alone left an ffmpeg compile's steady stream of gcc
  # warnings going straight to the console -- so the one command in this file that looked quiet was
  # the loudest thing a video tarball build printed. Held back the same way `configure` is, and the
  # log is replayed on failure for the same reason.
  make -j"$(nproc)" >make.log 2>&1 || {
    echo "ffmpeg-lgpl: make failed; last 40 lines of its log:" >&2
    tail -40 make.log >&2
    exit 1
  }
  make install DESTDIR="$staged" >>make.log 2>&1 || {
    echo "ffmpeg-lgpl: make install failed; last 40 lines of its log:" >&2
    tail -40 make.log >&2
    exit 1
  }
)

# `mv -T` refuses an existing directory rather than moving inside it, which is exactly the check
# wanted: if a peer built the same id while we were compiling, its tree is already there and is
# byte-for-byte what ours would be, so keeping theirs is correct and ours is discarded.
if ! mv -T "$staged$PREFIX" "$PREFIX" 2>/dev/null; then
  if [ -f "$DONE" ]; then
    echo "-- another build finished $FF_SRC_ID first; keeping it"
    exit 0
  fi
  echo "ffmpeg-lgpl: could not move the build into $PREFIX" >&2
  exit 1
fi

# -- the one external library, carried rather than depended on -----------------------------------
#
# `--enable-libopenh264` in the shared pin asks for the only H.264 *encoder* an LGPL configure line
# can have, because a machine streaming its screen has to produce H.264 and ffmpeg implements no
# encoder for it. The flag makes libavcodec link libopenh264, which is a library this prefix does not
# yet hold -- so a tarball built from it would want a package on the user's machine, and the whole
# point of the prefix is that it wants nothing.
#
# So it is copied in beside the libraries that link it, under its soname, exactly as the tarball
# copies those. BSD-2, so carrying it is a matter of carrying the text beside it; the copy of that
# text goes in for the same reason COPYING.LGPLv2.1 does, so that whatever stages these has one
# place to take every license from.
sysopenh264="$(ldconfig -p | awk '/libopenh264\.so\.[0-9]+$/ { print $NF; exit }')"
if [ -z "$sysopenh264" ] || [ ! -f "$sysopenh264" ]; then
  echo "ffmpeg-lgpl: the build asked for --enable-libopenh264 and no libopenh264.so.N is here." >&2
  echo "             tools/platform/linux/apt-deps.sh installs libopenh264-dev for it." >&2
  exit 1
fi
cp "$sysopenh264" "$PREFIX/lib/$(patchelf --print-soname "$sysopenh264")"
chmod 644 "$PREFIX/lib/libopenh264.so."*
for doc in /usr/share/doc/libopenh264-[0-9]*/copyright; do
  cp "$doc" "$PREFIX/openh264-COPYRIGHT"
  break
done
[ -f "$PREFIX/openh264-COPYRIGHT" ] || {
  echo "ffmpeg-lgpl: libopenh264 is being carried and its license text was not found." >&2
  exit 1
}

# Assert the thing this file exists for, rather than trusting the flags to have meant what they say.
# A configure that silently found a system library -- somebody adds a -dev package to the image, and
# --disable-autodetect stops being the only thing standing between this and a GPL dependency -- would
# otherwise be discovered by a user on another distribution, as a missing library at load time.
#
# **The check always runs; only its display is held back.** They read as one thing and are not: the
# loop below refuses the build, which no verbosity may skip, while the `ldd` at the end of it is a
# picture of a result that has just been asserted correct. DIST_VERBOSE is read straight out of the
# environment rather than by sourcing tools/dist/common.sh, because this script takes a prefix as its
# only argument and may be run from any directory -- and inside the container the variable is there,
# put on `docker run` by tarball.sh.
missing=0
for lib in "$PREFIX"/lib/lib*.so.*; do
  case "$lib" in *.so.*.*) continue ;; esac   # the fully-versioned real files; the sonames suffice
  while read -r dep; do
    case "$dep" in
      libc.so.*|libm.so.*|libgcc_s.so.*|libpthread.so.*|libdl.so.*|librt.so.*) ;;
      # zlib is the one external library the shared pin asks for by name -- several demuxers want it,
      # and libz.so.1 is on every Linux that can run any of this. Allowed here rather than silently
      # tolerated, so that anything *else* appearing still stops the build.
      libz.so.*) ;;
      # libstdc++ is here for libopenh264, which is C++ where every other library in this prefix is
      # C. It is not carried: a bundled libstdc++ that is older than the system's own breaks any C++
      # the process loads afterwards, and it is on every distribution the tarball claims -- the
      # floor being the one in tools/platform/linux/Dockerfile's FROM line.
      libstdc++.so.*) ;;
      libav*|libsw*|libopenh264.so.*) ;;       # our own siblings, the last one copied in above
      *)
        echo "ffmpeg-lgpl: $(basename "$lib") links $dep, which is not libc and not one of ours." >&2
        missing=1
        ;;
    esac
  done < <(patchelf --print-needed "$lib")
done
[ "$missing" -eq 0 ] || {
  echo "ffmpeg-lgpl: refusing a build with external dependencies -- the tarball would not be" >&2
  echo "             self-contained, and the license argument in this file's header assumed it was." >&2
  exit 1
}
# LD_LIBRARY_PATH so that `ldd` resolves our own siblings to *ours*. Without it the loader finds
# Debian's libavutil and libswresample on the system path, and the report then lists their
# dependencies -- libsoxr and the rest -- which looks exactly like the failure the check above just
# ruled out. The check is right and the display was lying; this makes them agree.
if [ "${DIST_VERBOSE:-0}" = "1" ]; then
  echo
  echo "-- what the result links --"
  LD_LIBRARY_PATH="$PREFIX/lib" ldd "$PREFIX"/lib/libavcodec.so.* 2>/dev/null | grep -v '^lib' | head -8
fi

strip --strip-unneeded "$PREFIX"/lib/lib*.so.*.* 2>/dev/null || true

# The license text kept beside the libraries rather than left in a source tree that gets deleted, so
# whatever stages these has one place to take both from. Naming the version here too: the tarball's
# LICENSES/ has to say which release the source offer points at.
cp "$src/COPYING.LGPLv2.1" "$PREFIX/COPYING.LGPLv2.1"
printf '%s' "$FF_SRC_VER" > "$PREFIX/VERSION"

# Last, so it can only ever mark a tree that got all the way here -- including the closure check
# above, which is the one that would refuse a build that quietly found a system library.
printf '%s' "$FF_SRC_ID" > "$DONE"

echo
echo "-- ffmpeg $FF_SRC_VER built into $PREFIX --"
