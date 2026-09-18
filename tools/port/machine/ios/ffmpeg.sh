#!/usr/bin/env bash
#
# Builds the pinned LGPL ffmpeg for iOS, once per slice, into the shared asset cache.
#
#   tools/port/machine/ios/ffmpeg.sh [device|sim]      build it, or say it is already built
#   tools/port/machine/ios/ffmpeg.sh --print-dir device   print the prefix, build nothing
#
# **A fourth caller of tools/setup/ffmpeg-pin.sh**, after the developer's Mac build, the Debian
# container's and Android's. What is shared is the *definition* -- the release, the bytes and the
# flags -- and never the procedure, because the four environments genuinely differ. See that file's
# header.
#
# # Shared libraries, and the reason is the license
#
# The whole ffmpeg posture here rests on shipping the shared libraries *beside* the binary being
# compliant under LGPL where linking them into it would not be, which is why the APK links none of
# it statically. **iOS does not force a static link.** An app bundle may carry dynamic libraries in
# `Frameworks/`, signed and embedded; what the platform refuses is a library loaded from *outside*
# the bundle. So the four libraries become four `.framework` bundles and the pin's
# `--enable-shared --disable-static` stands unchanged.
#
# `--install-name-dir=@rpath` is what makes that work without rewriting every library afterwards:
# ffmpeg's own configure stamps the install name, so each library announces itself as
# `@rpath/lib<name>.<abi>.dylib` from the moment it is linked. `frameworks.sh` beside this file is
# what turns them into bundles and fixes the references between them.
#
# # The decoder set is not narrowed
#
# Android narrows it to nine decoders because libavcodec's link line runs past Windows'
# 32,767-character command line and fails mid-argument. **This build only ever runs on a Mac**, so
# there is no such limit and the pin's own rule holds: every decoder ffmpeg implements itself. A
# video song that plays on the appliance therefore plays here, which is the property that rule
# exists to keep.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
. tools/setup/ffmpeg-pin.sh
. tools/setup/asset-cache.sh
DIST_SCRIPT=ffmpeg

MIN_IOS=15.0

PRINT_DIR=0
SLICE="device"
for arg in "$@"; do
  case "$arg" in
    --print-dir) PRINT_DIR=1 ;;
    device | sim) SLICE="$arg" ;;
    *) echo "$DIST_SCRIPT: unknown argument $arg" >&2; exit 2 ;;
  esac
done

case "$SLICE" in
  device) SDK=iphoneos; TRIPLE_MIN="-mios-version-min=$MIN_IOS" ;;
  sim) SDK=iphonesimulator; TRIPLE_MIN="-mios-simulator-version-min=$MIN_IOS" ;;
esac

# **Content-addressed by the pin, so a changed flag is a different directory** rather than a stale
# prefix nobody notices. `FF_SRC_ID_CROSS` is a hash of the version and the configure line this takes;
# the slice is in the name because a device library cannot be linked into a simulator build.
#
# **The cross line, because iOS has no openh264 package to link.** `--enable-libopenh264` is asked for
# by name on a line with autodetection off, so configure refuses outright rather than building
# without it, and the id has to name the line that was actually used.
DEST="$CACHE/ffmpeg-$FF_SRC_VER-lgpl-ios-$SLICE-$FF_SRC_ID_CROSS"
# Written last, and checked instead of `lib/`: a build interrupted halfway leaves a prefix with some
# libraries in it, which every check by directory existence would call finished. Android's ffmpeg.sh
# uses the same marker for the same reason.
MARKER="$DEST/.km-complete"

if [ "$PRINT_DIR" -eq 1 ]; then
  [ -f "$MARKER" ] && printf '%s\n' "$DEST"
  exit 0
fi

if [ -f "$MARKER" ]; then
  dist_step "ffmpeg $FF_SRC_VER for $SLICE is already built"
  dist_detail "$DEST"
  exit 0
fi

# -- Xcode -----------------------------------------------------------------------------------------

XCODE="${XCODE:-/Applications/Xcode.app}"
[ -d "$XCODE" ] || { echo "$DIST_SCRIPT: no Xcode at $XCODE -- set XCODE to its path." >&2; exit 1; }
export DEVELOPER_DIR="${DEVELOPER_DIR:-$XCODE/Contents/Developer}"

SYSROOT="$(xcrun --sdk "$SDK" --show-sdk-path)"
[ -n "$SYSROOT" ] && [ -d "$SYSROOT" ] || {
  echo "$DIST_SCRIPT: no $SDK SDK in $XCODE." >&2
  echo "       Run: xcodebuild -downloadPlatform iOS" >&2
  exit 1
}
CC="$(xcrun --sdk "$SDK" -f clang)"

# -- the source ------------------------------------------------------------------------------------

TAR="$CACHE/$FF_SRC_NAME.tar.xz"
mkdir -p "$CACHE"

if command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
fi

# Shared with the developer's own macOS build, which caches the same tarball under the same name --
# so a machine that has run `tools/setup/fetch-ffmpeg.sh` re-downloads nothing here.
if [ -f "$TAR" ] && [ "$(sha256 "$TAR")" = "$FF_SRC_SHA256" ]; then
  dist_step "cached $FF_SRC_NAME.tar.xz"
else
  dist_step "fetching $FF_SRC_NAME.tar.xz (11 MB)"
  curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$TAR.part" "$FF_SRC_URL"
  got="$(sha256 "$TAR.part")"
  if [ "$got" != "$FF_SRC_SHA256" ]; then
    rm -f "$TAR.part"
    echo "$DIST_SCRIPT: $FF_SRC_NAME.tar.xz failed verification" >&2
    echo "  expected $FF_SRC_SHA256" >&2
    echo "  got      $got" >&2
    exit 1
  fi
  mv "$TAR.part" "$TAR"
fi

SRC="$CACHE/src-ios-$SLICE"
rm -rf "$SRC" "$DEST"
mkdir -p "$SRC"
dist_step "unpacking $FF_SRC_NAME"
tar -xJf "$TAR" -C "$SRC"

# -- configure and build ---------------------------------------------------------------------------

# **`--enable-cross-compile` and `--target-os=darwin`, with the arch named separately**, because
# configure runs its test programs with `$CC` and would otherwise link them against the host and
# conclude that the target is macOS. `--sysroot` alone is not enough: the version-min flag is what
# stops the libraries claiming a macOS deployment target, which is the shape of failure that surfaces
# as `building for iOS but linking object built for macOS` when Xcode links the app.
#
# `--install-name-dir=@rpath` for the reason this file's header gives.
#
# `--disable-asm` is deliberately *not* here: arm64 assembles with the toolchain already present, and
# turning it off would be materially slower at exactly the thing this is for.
CFLAGS="-arch arm64 -isysroot $SYSROOT $TRIPLE_MIN"
jobs="$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"

dist_step "building ffmpeg $FF_SRC_VER for $SLICE (LGPL, decode only) -- a few minutes, once"
dist_detail "sysroot: $SYSROOT"
dist_detail "prefix:  $DEST"
(
  cd "$SRC/$FF_SRC_NAME"
  ./configure \
    --prefix="$DEST" \
    "${FF_SRC_CONFIGURE_CROSS[@]}" \
    --enable-cross-compile \
    --target-os=darwin \
    --arch=arm64 \
    --cc="$CC" \
    --sysroot="$SYSROOT" \
    --extra-cflags="$CFLAGS" \
    --extra-ldflags="$CFLAGS" \
    --install-name-dir=@rpath \
    > "$SRC/configure.log" 2>&1 ||
    { tail -25 "$SRC/configure.log" >&2
      echo "$DIST_SCRIPT: ffmpeg's configure failed; the tail is above. Whole log: $SRC/configure.log" >&2
      exit 1; }
  make -j"$jobs" > "$SRC/make.log" 2>&1 ||
    { tail -25 "$SRC/make.log" >&2
      echo "$DIST_SCRIPT: building ffmpeg failed; the tail is above. Whole log: $SRC/make.log" >&2
      exit 1; }
  make install > "$SRC/install.log" 2>&1 ||
    { tail -25 "$SRC/install.log" >&2
      echo "$DIST_SCRIPT: installing into $DEST failed" >&2
      exit 1; }
)

# `make install` does not install the license texts, and the terms are what make shipping these
# libraries legitimate. Copied into the prefix so the staging step finds them in the same place the
# other carriers' do.
cp "$SRC/$FF_SRC_NAME"/COPYING.* "$SRC/$FF_SRC_NAME"/LICENSE.md "$DEST/" 2>/dev/null ||
  echo "$DIST_SCRIPT: warning -- the source carried no COPYING files; the staged terms will be missing" >&2

# -- prove it is what it claims --------------------------------------------------------------------
#
# **Checked rather than assumed, on Android's reasoning**: a cross-compile that quietly produced host
# libraries builds, installs, and fails only when Xcode links the app -- and the message then names
# an architecture rather than this script.
for lib in avcodec avformat avutil swresample; do
  dylib="$(find "$DEST/lib" -name "lib$lib.*.dylib" -type f | head -1)"
  [ -n "$dylib" ] || { echo "$DIST_SCRIPT: no lib$lib in $DEST/lib" >&2; exit 1; }

  arch="$(lipo -archs "$dylib" 2>/dev/null || echo unknown)"
  [ "$arch" = "arm64" ] || { echo "$DIST_SCRIPT: lib$lib is $arch, not arm64" >&2; exit 1; }

  # Platform 2 is iOS and 7 is the iOS simulator; anything else means the version-min flag did not
  # take and the library is a macOS one wearing an iOS sysroot.
  want=2
  [ "$SLICE" = "sim" ] && want=7
  got="$(otool -l "$dylib" | awk '/LC_BUILD_VERSION/{f=1} f&&/platform/{print $2; exit}')"
  [ "$got" = "$want" ] || {
    echo "$DIST_SCRIPT: lib$lib says platform $got, wanted $want ($SLICE)" >&2
    exit 1
  }

  # The install name is what the app's `@rpath` resolves against, and a wrong one is a dynamic
  # loader failure at launch rather than a link error.
  case "$(otool -D "$dylib" | tail -1)" in
    @rpath/*) ;;
    *) echo "$DIST_SCRIPT: lib$lib's install name is not under @rpath" >&2; exit 1 ;;
  esac
done

: > "$MARKER"
# The tree is a gigabyte of objects and the tarball is cached, so a rebuild costs a re-unpack rather
# than a re-download. The logs go with it; they were only ever for the failure paths above.
rm -rf "$SRC"

dist_step "built ffmpeg $FF_SRC_VER for $SLICE"
dist_detail "$DEST"
