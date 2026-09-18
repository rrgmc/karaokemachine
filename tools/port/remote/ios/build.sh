#!/usr/bin/env bash
#
# Cross-compiles the offline remote for iOS and generates the Xcode project around it.
#
#   tools/port/remote/ios/build.sh                  # device + simulator, debug
#   tools/port/remote/ios/build.sh --release        # device + simulator, release
#   tools/port/remote/ios/build.sh --device-only    # skip the simulator slice
#   tools/port/remote/ios/build.sh --no-app         # stop at the project; do not compile the app
#   tools/port/remote/ios/build.sh --release --ipa  # ...and package that as a release carrier
#
#   dist/km-remote/ios/km-remote-<version>-ios-unsigned.ipa   (--ipa)
#
# **One script where Android has two**, and the missing one is `stage.sh`. There is nothing to stage:
# an `.xcframework` is what Xcode consumes directly, so the archives are assembled into one rather
# than copied into a source set the way a `.so` is copied into `jniLibs`.
#
# What is shared with the rest of `tools/` is sourced rather than copied: `dist_step`, `dist_detail`
# and `dist_target_dir`.
#
# Prerequisites, all of which this script checks before it builds anything:
#   - Xcode, with the iOS platform installed (not just the Command Line Tools)
#   - rustup target add aarch64-apple-ios aarch64-apple-ios-sim
#   - brew install xcodegen

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
. tools/setup/features.sh
DIST_SCRIPT=build

# **Must match `deploymentTarget.iOS` in ports/remote/ios/project.yml.** `cc` compiles the SQLite
# amalgamation that `rusqlite` bundles, and it obeys this variable; Xcode compiles the Swift against
# its own setting. A mismatch is not caught by either -- it is a link error inside Xcode naming
# object files, long after cargo said `Finished`.
MIN_IOS=15.0
export IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS"

OUT=ports/remote/ios/Frameworks
BUILD=ports/remote/ios/build/lib
HEADER=crates/remote/km-remote-ios/include/km_remote.h

# Both slices by default. **`--device-only` is a debugging shortcut and not a thing to ship**, in the
# sense that a simulator-less xcframework simply cannot be run in a simulator; it is here because the
# second `cargo build` costs a minute and a device-only iteration is a real thing to want.
SLICES=("device" "sim")
# **The app is compiled by default, and that is the whole point of the flag being a negative one.**
# For most of this script's life it stopped at generating the project, so it reported success over a
# target that did not build -- and did so for weeks, because the one command anybody runs for "the
# iOS build" is this one. `--no-app` exists for the case where only the framework is wanted and
# `build:ios:remote:native` is too little, not as the normal path.
APP=1
# **`--ipa` is a release carrier, asked for by name the way `--notarize` is one platform over.** It
# packages the app this script has just compiled, unsigned, which is the only kind of `.ipa` anything
# here can produce; `dist_ipa` in tools/dist/common.sh says why there is no signed path to one.
IPA=0
CARGO_ARGS=()
for arg in "$@"; do
  case "$arg" in
    --device-only) SLICES=("device") ;;
    --no-app) APP=0 ;;
    --ipa) IPA=1 ;;
    *) CARGO_ARGS+=("$arg") ;;
  esac
done

triple_of() {
  case "$1" in
    device) echo "aarch64-apple-ios" ;;
    sim) echo "aarch64-apple-ios-sim" ;;
  esac
}

# -- Xcode ---------------------------------------------------------------------------------------
#
# **`DEVELOPER_DIR` is exported here rather than asking for `sudo xcode-select -s`.** `xcode-select`
# is a machine-wide setting and is very often pointing at the Command Line Tools, which have no
# `xcodebuild` and no iOS SDKs at all. Setting it for this process changes nothing else and needs no
# password. A real `DEVELOPER_DIR` in the environment still wins, because `XCODE` is only a default.
XCODE="${XCODE:-/Applications/Xcode.app}"
if [ ! -d "$XCODE" ]; then
  echo "$DIST_SCRIPT: no Xcode at $XCODE -- install it, or set XCODE to its path." >&2
  exit 1
fi
export DEVELOPER_DIR="${DEVELOPER_DIR:-$XCODE/Contents/Developer}"

# The iOS *platform* is a separate download from the SDK, and without it every `-destination` is
# reported ineligible with an error that blames the destination rather than the missing platform.
if ! xcodebuild -showsdks 2>/dev/null | grep -q -- '-sdk iphoneos'; then
  echo "$DIST_SCRIPT: no iOS platform in $XCODE." >&2
  echo "       Run: xcodebuild -downloadPlatform iOS" >&2
  exit 1
fi

# -- The Rust targets ----------------------------------------------------------------------------
#
# Named rather than added to `rust-toolchain.toml`: that file has no `targets` key, and adding one
# would make every developer on Windows and Linux download an iOS standard library for nothing.
for slice in "${SLICES[@]}"; do
  triple="$(triple_of "$slice")"
  if ! rustup target list --installed | grep -qx "$triple"; then
    echo "$DIST_SCRIPT: the $triple target is not installed." >&2
    echo "       Run: rustup target add aarch64-apple-ios aarch64-apple-ios-sim" >&2
    exit 1
  fi
done

XCODEGEN="${XCODEGEN:-$(command -v xcodegen || true)}"
if [ -z "$XCODEGEN" ]; then
  echo "$DIST_SCRIPT: no xcodegen found -- brew install xcodegen, or set XCODEGEN." >&2
  exit 1
fi

# -- Build ---------------------------------------------------------------------------------------

profile="debug"
for arg in ${CARGO_ARGS+"${CARGO_ARGS[@]}"}; do [ "$arg" = "--release" ] && profile="release"; done

# **The refusals are about what goes on a release page.** A debug build is slow and chatty, and a
# file name says nothing about either.
if [ "$IPA" = "1" ]; then
  if [ "$APP" != "1" ]; then
    echo "$DIST_SCRIPT: --ipa packages the app, which --no-app declines to build." >&2
    exit 2
  fi
  if [ "$profile" != "release" ]; then
    echo "$DIST_SCRIPT: --ipa needs --release; a debug build is not a carrier." >&2
    exit 2
  fi
fi

dist_step "building the offline remote for ${SLICES[*]} (iOS $MIN_IOS, $profile)"
dist_detail "features: ${KM_FEATURES_IOS_REMOTE:-<none>}"
dist_detail "developer dir: $DEVELOPER_DIR"

for slice in "${SLICES[@]}"; do
  # `--lib` because this package has no binary at all: an app links the archive, nothing runs it.
  cargo build -p km-remote-ios --lib --target "$(triple_of "$slice")" \
    ${CARGO_ARGS+"${CARGO_ARGS[@]}"}
done

# -- Assemble ------------------------------------------------------------------------------------

# Asked of cargo rather than spelled `target/`. Nothing in tools/ may assume that directory: a
# worktree, a container and another machine all put it somewhere else, and the failure from guessing
# is a script reporting a missing file on the line after cargo said `Finished`.
target_dir="$(dist_target_dir)"

# `-create-xcframework` refuses an output that already exists, so both are cleared rather than
# written over.
rm -rf "$OUT" "$BUILD"
mkdir -p "$OUT"

args=()
for slice in "${SLICES[@]}"; do
  triple="$(triple_of "$slice")"
  archive="$target_dir/$triple/$profile/libkm_remote_ios.a"
  if [ ! -f "$archive" ]; then
    # The directory is named because "the build failed" is only one of the two reasons to get here,
    # and it is the one a person will believe. The other is that cargo built somewhere this script
    # did not look, which is unsayable when the path is a literal `target/`.
    echo "$DIST_SCRIPT: $slice produced no archive at $archive" >&2
    exit 1
  fi
  mkdir -p "$BUILD/$slice/inc"
  cp "$archive" "$BUILD/$slice/"
  cp "$HEADER" "$BUILD/$slice/inc/"
  args+=(-library "$BUILD/$slice/libkm_remote_ios.a" -headers "$BUILD/$slice/inc")
done

dist_step "assembling KmRemote.xcframework"
# **The two archives are separate `-library` arguments and never one fat archive.** Both slices are
# arm64, and `lipo` would happily merge them into something no linker can tell apart -- which is the
# problem the xcframework format exists to solve.
xcodebuild -create-xcframework "${args[@]}" -output "$OUT/KmRemote.xcframework" >/dev/null

dist_step "generating the Xcode project"
(cd ports/remote/ios && "$XCODEGEN" generate --quiet)

# -- Compile the app -----------------------------------------------------------------------------
#
# **Generating a project is not compiling it, and the gap is not academic.** `xcodegen` writes a
# project out of `project.yml` without reading a line of Swift, so everything above this can succeed
# with `RootViewController.swift` in a state no compiler will accept. That happened: a rename sweep
# turned Foundation's `NSURLErrorCancelled` into `NSURLErrorCanceled`, and the app did not build for
# as long as it took somebody to open Xcode. Nothing reported it -- `cargo km-test` never sees the
# Swift, `task check` never sees this target, and `task build:ios:remote` carries
# `platforms: [darwin]`, so off macOS it is *skipped rather than failed* and prints a green line.
#
# **`CODE_SIGNING_ALLOWED=NO`, because the question here is whether the code compiles.** Signing
# needs a certificate in the keychain and a provisioning profile, and asking for them would turn a
# compile check into a credentials check -- failing on a machine that has the source, the toolchain
# and every reason to want to know whether the app builds. Xcode signs when somebody presses Run,
# which is where signing belongs and where a missing certificate is a sentence a person can act on.
#
# **`generic/platform=iOS` rather than a named device or simulator**, so this needs nothing plugged
# in and nothing booted. It is the device slice either way; `--device-only` does not change what is
# checked here, only what the xcframework can additionally run on.
#
# The output goes to a file and is printed only on failure. A successful `xcodebuild` says several
# hundred lines of nothing anybody reads, and the one line that matters when it fails is buried in
# them.
if [ "$APP" = "1" ]; then
  case "$profile" in
    release) configuration=Release ;;
    *)       configuration=Debug   ;;
  esac
  dist_step "compiling the app ($configuration)"
  app_log="ports/remote/ios/build/xcodebuild.log"
  mkdir -p "$(dirname "$app_log")"
  if ! (cd ports/remote/ios && xcodebuild \
          -project KaraokeRemote.xcodeproj \
          -scheme KaraokeRemote \
          -configuration "$configuration" \
          -destination 'generic/platform=iOS' \
          -derivedDataPath build/dd \
          CODE_SIGNING_ALLOWED=NO \
          build) > "$app_log" 2>&1; then
    echo >&2
    echo "$DIST_SCRIPT: the app did not compile. The errors, from $app_log:" >&2
    grep -E 'error:' "$app_log" >&2 || tail -30 "$app_log" >&2
    exit 1
  fi
  APP_BUNDLE="ports/remote/ios/build/dd/Build/Products/$configuration-iphoneos/KaraokeRemote.app"
fi

# -- The carrier -----------------------------------------------------------------------------------
#
# **Named from the manifest and checked against the bundle.** `tools/dist/release.sh` goes looking for
# this file under the manifest's version, so that is the name it takes; the bundle's own
# `CFBundleShortVersionString` is a copy nothing else reconciles, and a carrier is the one place the
# two disagreeing costs anything.
if [ "$IPA" = "1" ]; then
  version="$(dist_manifest_version)"
  bundled="$(dist_bundle_version "$APP_BUNDLE")"
  if [ "$bundled" != "$version" ]; then
    echo "$DIST_SCRIPT: the bundle says $bundled where the manifest says $version." >&2
    echo "       CFBundleShortVersionString in ports/remote/ios/project.yml is the copy to fix." >&2
    exit 1
  fi
  IPA_PATH="$(dist_dir km-remote ios)/km-remote-$version-ios-unsigned.ipa"
  mkdir -p "$(dirname "$IPA_PATH")"
  dist_step "packaging the .ipa"
  dist_ipa "$APP_BUNDLE" "$IPA_PATH"
fi

echo
echo "built:"
for slice in "${SLICES[@]}"; do
  triple="$(triple_of "$slice")"
  printf '  %-8s %s\n' "$slice" "$(du -h "$BUILD/$slice/libkm_remote_ios.a" | cut -f1)"
done

if [ "$APP" = "1" ]; then
  # `du -sh`, not `du -h`: a `.app` is a directory, and without `-s` this prints a line per folder
  # inside it. The loop above gets away with `du -h` because a `.a` is one file.
  printf '  %-8s %s\n' "app" "$(du -sh "$APP_BUNDLE" | cut -f1) (unsigned; Xcode signs it on Run)"
else
  printf '  %-8s %s\n' "app" "not compiled -- --no-app was given"
fi

if [ "$IPA" = "1" ]; then
  printf '  %-8s %s\n' "ipa" "$(du -h "$IPA_PATH" | cut -f1) $IPA_PATH"
fi

echo
echo "now:  open ports/remote/ios/KaraokeRemote.xcodeproj  and press Run"
