#!/usr/bin/env bash
#
# Wraps the four ffmpeg libraries as embedded xcframeworks.
#
#   tools/port/machine/ios/frameworks.sh [device] [sim]
#
# Run after `ffmpeg.sh` and before `xcodebuild`; `build.sh` calls both. This is a repackage rather
# than a build, and it is kept out of `ffmpeg.sh` for the reason the assets step is kept out of
# `build.sh`: that one writes into the shared asset cache, where several worktrees read one copy,
# and this one writes into a checkout's own gitignored `Frameworks/`.
#
# # Why frameworks, and not the dylibs as they come
#
# **The license is why these are shared libraries at all**: shipping them beside the binary is
# compliant under LGPL where linking them into it would not be. iOS permits that -- an app bundle
# may carry dynamic libraries -- and refuses only a library loaded from *outside* the bundle. What
# it wants them wrapped in is a framework: a bare `.dylib` dropped in `Frameworks/` is loadable on a
# development build and is not a shape to ship.
#
# # The naming is what makes the linker find them
#
# A framework called `avcodec.framework` holding a binary called `avcodec` is what resolves the
# undefined `_avcodec_*` symbols in the Rust staticlib. The libraries arrive as `libavcodec.61.dylib`,
# so the binary is renamed on the way in and every reference to the old name is rewritten -- both the
# library's own install name and the references its siblings hold, since libavformat is linked
# against libavcodec and would otherwise ask the loader for a file that is not in the bundle.
#
# # An xcframework each, for the reason the Rust staticlib gets one
#
# A device framework cannot run in a simulator, and a checkout has one `Frameworks/`. Wrapping each
# slice's framework in an `.xcframework` lets Xcode pick by SDK, the same arrangement
# `KmMachine.xcframework` already has -- where a per-slice directory would work until somebody
# pressed Run against a simulator and got a link error naming an architecture.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
DIST_SCRIPT=frameworks

# `-create-xcframework` is `xcodebuild`, so this script needs Xcode as much as the others do --
# `xcode-select` very often points at the Command Line Tools, which have no `xcodebuild` at all.
# Exported for this process rather than asking for a machine-wide `sudo`.
XCODE="${XCODE:-/Applications/Xcode.app}"
[ -d "$XCODE" ] || { echo "$DIST_SCRIPT: no Xcode at $XCODE -- set XCODE to its path." >&2; exit 1; }
export DEVELOPER_DIR="${DEVELOPER_DIR:-$XCODE/Contents/Developer}"

MIN_IOS=15.0
LIBS="avcodec avformat avutil swresample"
OUT=ports/machine/ios/Frameworks
STAGE=ports/machine/ios/build/fw

SLICES=("$@")
[ ${#SLICES[@]} -gt 0 ] || SLICES=("device")

rm -rf "$STAGE"

for slice in "${SLICES[@]}"; do
case "$slice" in
  device) PLATFORM=iPhoneOS ;;
  sim) PLATFORM=iPhoneSimulator ;;
  *) echo "$DIST_SCRIPT: unknown slice $slice" >&2; exit 2 ;;
esac

PREFIX="$(tools/port/machine/ios/ffmpeg.sh --print-dir "$slice")"
[ -n "$PREFIX" ] || {
  echo "$DIST_SCRIPT: ffmpeg for $slice is not built." >&2
  echo "       Run: tools/port/machine/ios/ffmpeg.sh $slice" >&2
  exit 1
}

here="$STAGE/$slice"
mkdir -p "$here"
dist_step "wrapping ffmpeg as frameworks ($slice)"

for lib in $LIBS; do
  src="$(find "$PREFIX/lib" -name "lib$lib.*.dylib" -type f | head -1)"
  [ -n "$src" ] || { echo "$DIST_SCRIPT: no lib$lib in $PREFIX/lib" >&2; exit 1; }

  fw="$here/$lib.framework"
  rm -rf "$fw"
  mkdir -p "$fw"
  # **Flat, not versioned.** A `Versions/A/` layout with symlinks is macOS's; iOS wants the binary
  # and the manifest at the top of the bundle, and a symlink in an embedded framework is rejected
  # when the app is signed.
  cp "$src" "$fw/$lib"
  chmod 644 "$fw/$lib"

  install_name_tool -id "@rpath/$lib.framework/$lib" "$fw/$lib"
done

# The references between them, done after every framework exists so that a library can be rewritten
# to point at a sibling this loop has already created.
for lib in $LIBS; do
  fw="$here/$lib.framework/$lib"
  for other in $LIBS; do
    # `otool -L` lists what this library will ask the loader for. The name carries an ABI number
    # that changes with the release, so it is read off the binary rather than spelled here.
    old="$(otool -L "$fw" | awk -v o="$other" '$1 ~ ("lib" o "\\.[0-9]+\\.dylib$") {print $1; exit}')"
    [ -n "$old" ] || continue
    install_name_tool -change "$old" "@rpath/$other.framework/$other" "$fw"
  done
done

# The manifest each framework needs to be embedded and signed. Written per framework rather than
# copied from a template, because three of its values differ per library.
for lib in $LIBS; do
  cat > "$here/$lib.framework/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>$lib</string>
    <key>CFBundleIdentifier</key><string>org.ffmpeg.$lib</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>$lib</string>
    <key>CFBundlePackageType</key><string>FMWK</string>
    <key>CFBundleShortVersionString</key><string>1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>CFBundleSupportedPlatforms</key><array><string>$PLATFORM</string></array>
    <key>MinimumOSVersion</key><string>$MIN_IOS</string>
</dict>
</plist>
PLIST
done
done

# -- one xcframework per library -------------------------------------------------------------------

mkdir -p "$OUT"
for lib in $LIBS; do
  args=()
  for slice in "${SLICES[@]}"; do
    args+=(-framework "$PWD/$STAGE/$slice/$lib.framework")
  done
  # Refuses an output that already exists, exactly as the Rust one does.
  rm -rf "${OUT:?}/$lib.xcframework"
  xcodebuild -create-xcframework "${args[@]}" -output "$OUT/$lib.xcframework" >/dev/null
done

# The license is **not** copied here, and that is deliberate: `assets.sh` clears the asset tree
# before it fills it, so a file written here would be deleted by the step that runs next. It copies
# ffmpeg's terms itself, keyed off these xcframeworks existing -- the same way Android's assets step
# keys off its staged `.so` files rather than off a flag.

echo
echo "wrapped (${SLICES[*]}):"
for lib in $LIBS; do
  printf '  %-12s %s\n' "$lib" "$(du -sh "$OUT/$lib.xcframework" | cut -f1)"
done
