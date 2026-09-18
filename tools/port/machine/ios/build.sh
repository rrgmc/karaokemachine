#!/usr/bin/env bash
#
# Builds the machine for iOS, assembles the xcframework, generates the Xcode project and compiles
# the app.
#
#   tools/port/machine/ios/build.sh [--release] [--device-only] [--no-app] [--no-video]
#   tools/port/machine/ios/build.sh --release --ipa   # ...and package that as a release carrier
#
#   dist/karaokemachine/ios/karaokemachine-<version>-ios-unsigned.ipa   (--ipa)
#
# **Two scripts where Android has four**, and the arrangement is the offline remote's iOS one rather
# than the machine's Android one. There is nothing to stage: an `.xcframework` is what Xcode consumes
# directly, so this script assembles one instead of copying a library into a source set, and it
# generates the project because the project is generated. `assets.sh` beside it is the one step that
# does survive from Android, because a bundle still has to be given the bank, the wallpapers and the
# font.
#
# Prerequisites, all four checked before anything is built:
#   - Xcode, with the iOS platform (not just the Command Line Tools)
#   - rustup target add aarch64-apple-ios aarch64-apple-ios-sim
#   - brew install xcodegen
#   - tools/setup/fetch-assets.sh, for the bank

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/dist/common.sh
. tools/setup/features.sh
DIST_SCRIPT=build

# **Must match `deploymentTarget.iOS` in ports/machine/ios/project.yml.** `cc` compiles the SQLite
# amalgamation that `rusqlite` bundles, and it obeys this variable; Xcode compiles `main.m` against
# its own setting. A mismatch is not caught by either -- it is a link error inside Xcode naming
# object files, long after cargo said `Finished`.
MIN_IOS=15.0
export IPHONEOS_DEPLOYMENT_TARGET="$MIN_IOS"

# **CMake 4 rejects the FreeType vendored inside SDL3_ttf, and the panic names neither.** The offline
# remote's iOS build needs nothing like this because it links no SDL; this one builds SDL3 and
# SDL3_ttf from source for the phone, which is where that policy is read. `Taskfile.yml` sets the
# same variable in its global `env:`, so this line is what makes the script work when it is run
# directly.
export CMAKE_POLICY_VERSION_MINIMUM="${CMAKE_POLICY_VERSION_MINIMUM:-3.5}"

OUT=ports/machine/ios/Frameworks
BUILD=ports/machine/ios/build/lib
HEADER=crates/machine/km-machine-ios/include/km_machine.h

# Both slices by default. **`--device-only` is a debugging shortcut and not a thing to ship**, in the
# sense that a simulator-less xcframework simply cannot be run in a simulator; it is here because the
# second `cargo build` is a whole second SDL and a device-only iteration is a real thing to want.
SLICES=("device" "sim")
# **The app is compiled by default, and that is the whole point of the flag being a negative one.**
# `xcodegen` writes a project without reading a line of the shell it is generating a target for, so a
# script that stopped at generation would report success over a target that does not build -- which
# is exactly what happened to the offline remote's shell, for as long as it took somebody to open
# Xcode.
APP=1
# **Video is on by default, matching the Android build**, and `--no-video` is what a fresh clone or a
# quick iteration uses. With it off the app carries no ffmpeg at all: no frameworks are wrapped, none
# are embedded, and `km-video` compiles to nothing.
VIDEO=1
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
    --no-video) VIDEO=0 ;;
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

# **The refusals are about what goes on a release page.** A debug build is slow and chatty and a
# video-less one lists songs it cannot play, and a file name says nothing about either.
if [ "$IPA" = "1" ]; then
  if [ "$APP" != "1" ]; then
    echo "$DIST_SCRIPT: --ipa packages the app, which --no-app declines to build." >&2
    exit 2
  fi
  if [ "$profile" != "release" ]; then
    echo "$DIST_SCRIPT: --ipa needs --release; a debug build is not a carrier." >&2
    exit 2
  fi
  if [ "$VIDEO" != "1" ]; then
    echo "$DIST_SCRIPT: --ipa carries the video decoder; --no-video is not a carrier." >&2
    exit 2
  fi
fi

# -- ffmpeg, when there is video ------------------------------------------------------------------
#
# Before cargo, because `ffmpeg-sys-next` reads `FFMPEG_DIR` in its build script and runs bindgen
# against the headers there. Each slice gets its own prefix: a device library cannot be linked into
# a simulator build, and the two have different `LC_BUILD_VERSION` platforms.
if [ "$VIDEO" = "1" ]; then
  for slice in "${SLICES[@]}"; do
    tools/port/machine/ios/ffmpeg.sh "$slice"
  done
fi

# Named rather than spelled out, and empty rather than absent -- see `KM_FEATURES_IOS` in
# tools/setup/features.sh. `--features ""` is accepted by cargo and means nothing extra.
features=""
[ "$VIDEO" = "1" ] && features="$KM_FEATURES_IOS"

dist_step "building the machine for ${SLICES[*]} (iOS $MIN_IOS, $profile)"
dist_detail "features: ${features:-<none>}"
dist_detail "developer dir: $DEVELOPER_DIR"

for slice in "${SLICES[@]}"; do
  triple="$(triple_of "$slice")"
  # **`FFMPEG_DIR` and bindgen's sysroot are per slice and set here rather than in the environment.**
  # `ffmpeg-sys-next` finds the libraries through the first; the second is what stops bindgen parsing
  # iOS headers with the *host* clang's idea of what a target is, which fails on `TargetConditionals.h`
  # long before it reaches anything of ffmpeg's. The variable's name carries the triple with
  # underscores, which is the spelling bindgen looks for -- the same one
  # `tools/port/machine/android/ffmpeg.sh` writes for its two.
  if [ "$VIDEO" = "1" ]; then
    prefix="$(tools/port/machine/ios/ffmpeg.sh --print-dir "$slice")"
    [ -n "$prefix" ] || { echo "$DIST_SCRIPT: ffmpeg for $slice did not build" >&2; exit 1; }
    case "$slice" in
      device) sdk=iphoneos; target="arm64-apple-ios$MIN_IOS" ;;
      sim) sdk=iphonesimulator; target="arm64-apple-ios$MIN_IOS-simulator" ;;
    esac
    sysroot="$(xcrun --sdk "$sdk" --show-sdk-path)"
    export FFMPEG_DIR="$prefix"
    export "BINDGEN_EXTRA_CLANG_ARGS_${triple//-/_}=--target=$target -isysroot $sysroot"
  fi
  # `--lib` because this package has no binary at all: an app links the archive, nothing runs it.
  cargo build -p km-machine-ios --lib --target "$triple" \
    --features "$features" ${CARGO_ARGS+"${CARGO_ARGS[@]}"}
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
  archive="$target_dir/$triple/$profile/libkm_machine_ios.a"
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
  args+=(-library "$BUILD/$slice/libkm_machine_ios.a" -headers "$BUILD/$slice/inc")
done

dist_step "assembling KmMachine.xcframework"
# **The two archives are separate `-library` arguments and never one fat archive.** Both slices are
# arm64, and `lipo` would happily merge them into something no linker can tell apart -- which is the
# problem the xcframework format exists to solve.
xcodebuild -create-xcframework "${args[@]}" -output "$OUT/KmMachine.xcframework" >/dev/null

# -- ffmpeg's frameworks ---------------------------------------------------------------------------
#
# Before the assets, because this is what puts ffmpeg's license into the asset tree -- and the assets
# step clears that directory before it copies.
if [ "$VIDEO" = "1" ]; then
  tools/port/machine/ios/frameworks.sh "${SLICES[@]}"
else
  # Cleared rather than left, so that a `--no-video` build after a video one does not embed four
  # libraries nothing in the binary asks for.
  rm -rf ports/machine/ios/Frameworks/av*.xcframework ports/machine/ios/Frameworks/swresample.xcframework
fi

# **The fragment `project.yml` includes, written on every run.** It cannot list these four itself:
# xcodegen refuses a dependency whose xcframework is not on disk, so a committed list would make
# `--no-video` fail with four `There is no XCFramework found` errors naming a decoder nobody asked
# for. xcodegen concatenates arrays across an include, so an empty list here adds nothing.
{
  echo "# Written by tools/port/machine/ios/build.sh. Do not edit; the next build overwrites it."
  echo "targets:"
  echo "  KaraokeMachine:"
  echo "    dependencies:"
  if [ "$VIDEO" = "1" ]; then
    for lib in avcodec avformat avutil swresample; do
      # `embed: true` is what the LGPL posture rests on -- the libraries travel beside the binary
      # rather than inside it -- and `codeSign: true` because an embedded framework is signed with
      # the app, without which a device refuses to launch and blames the executable.
      echo "      - framework: Frameworks/$lib.xcframework"
      echo "        embed: true"
      echo "        codeSign: true"
    done
  else
    echo "      []"
  fi
} > ports/machine/ios/ffmpeg.yml

# -- The assets ----------------------------------------------------------------------------------
#
# Before the project is generated, because the folder reference `project.yml` declares has to exist
# for xcodegen to write it into the project.
ASSET_DEST=ports/machine/ios/KaraokeMachine/assets   # the folder tools/port/machine/ios/assets.sh writes
tools/port/machine/ios/assets.sh

# **A carrier with no instrument bank answers every MIDI song with a test tone.** The staging step
# reports that as a warning, which is right for an iteration and not for something handed over, and
# the file name says nothing about it -- the same argument as the two refusals above, made here
# because this is the first point at which the answer is known.
if [ "$IPA" = "1" ] && ! find "$ASSET_DEST" -name '*.sf2' | grep -q .; then
  echo "$DIST_SCRIPT: --ipa needs the instrument bank -- run tools/setup/fetch-assets.sh first." >&2
  exit 1
fi

dist_step "generating the Xcode project"
(cd ports/machine/ios && "$XCODEGEN" generate --quiet)

# -- Compile the app -----------------------------------------------------------------------------
#
# **Generating a project is not compiling it, and the gap is not academic.** See the note on `APP`
# above, and `tools/port/remote/ios/build.sh`, where a broken source file shipped for weeks because
# the build stopped one step short of this one.
#
# **`CODE_SIGNING_ALLOWED=NO`, because the question here is whether the code compiles.** Signing
# needs a certificate in the keychain and a provisioning profile, and asking for them would turn a
# compile check into a credentials check. Xcode signs when somebody presses Run, which is where a
# missing certificate is a sentence a person can act on.
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
  app_log="ports/machine/ios/build/xcodebuild.log"
  mkdir -p "$(dirname "$app_log")"
  if ! (cd ports/machine/ios && xcodebuild \
          -project KaraokeMachine.xcodeproj \
          -scheme KaraokeMachine \
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
  APP_BUNDLE="ports/machine/ios/build/dd/Build/Products/$configuration-iphoneos/KaraokeMachine.app"
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
    echo "       CFBundleShortVersionString in ports/machine/ios/project.yml is the copy to fix." >&2
    exit 1
  fi
  IPA_PATH="$(dist_dir karaokemachine ios)/karaokemachine-$version-ios-unsigned.ipa"
  mkdir -p "$(dirname "$IPA_PATH")"
  dist_step "packaging the .ipa"
  dist_ipa "$APP_BUNDLE" "$IPA_PATH"
fi

echo
echo "built:"
for slice in "${SLICES[@]}"; do
  printf '  %-8s %s\n' "$slice" "$(du -h "$BUILD/$slice/libkm_machine_ios.a" | cut -f1)"
done

if [ "$APP" = "1" ]; then
  # `du -sh`, not `du -h`: a `.app` is a directory, and without `-s` this prints a line per folder
  # inside it. The loop above gets away with `du -h` because a `.a` is one file.
  printf '  %-8s %s\n' "app" "$(du -sh "$APP_BUNDLE" | cut -f1) (unsigned; Xcode signs it on Run)"
  # **Counted in the bundle rather than counted towards it**, which is the check Android's assets
  # step exists to make: a file copied into a source tree is not the same claim as a file that
  # arrived in the package.
  staged=$(find "$APP_BUNDLE/assets" -type f 2>/dev/null | wc -l | tr -d ' ')
  printf '  %-8s %s\n' "assets" "$staged file(s) in the bundle"
else
  printf '  %-8s %s\n' "app" "not compiled -- --no-app was given"
fi

if [ "$IPA" = "1" ]; then
  printf '  %-8s %s\n' "ipa" "$(du -h "$IPA_PATH" | cut -f1) $IPA_PATH"
fi

echo
echo "now:  open ports/machine/ios/KaraokeMachine.xcodeproj  and press Run"
