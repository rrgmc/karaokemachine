#!/usr/bin/env bash
#
# Stages a macOS application bundle.
#
#   tools/platform/macos/app-bundle.sh              # fetch assets, build release, stage the .app
#   tools/platform/macos/app-bundle.sh --no-fetch   # skip the SoundFont step (it is already cached)
#   tools/platform/macos/app-bundle.sh --zip        # also produce the .zip beside the bundle
#   tools/platform/macos/app-bundle.sh --no-video   # build without the `video` feature (needs no ffmpeg)
#   tools/platform/macos/app-bundle.sh -v           # watch the build; quiet is the default
#
# **Quiet by default.** The phases, the report at the end and every warning are printed; cargo's
# compile stream is not. A step that fails replays everything it held back, so `-v` is for watching
# a build rather than for diagnosing one afterwards.
#
# **Video is on by default**, and needs ffmpeg and libclang to *build*; both are found by this script.
# The bundle it produces needs neither: ffmpeg's libraries are copied into Contents/Frameworks and
# every load command is rewritten to `@rpath`, so the .app plays video on a Mac that has never heard of
# Homebrew. That was not true until this script learned to do it -- macOS resolves a dylib by the
# absolute path it was linked at, so the bundle used to play video only on the machine that built it.
# See `dist_stage_ffmpeg_macos` in tools/dist/common.sh, which does the same job for the tool folders.
# `--no-video` builds the smaller one that reads no video at all.
#
# This is a *release* default and nothing else: the cargo feature is still off by default, so
# `cargo build -p karaokemachine` needs neither ffmpeg nor libclang.
#
# Output: dist/karaokemachine/macos/Karaoke Machine.app
#
#   Karaoke Machine.app/
#     Contents/
#       Info.plist
#       PkgInfo
#       MacOS/karaokemachine
#       Resources/karaokemachine.icns
#       Resources/assets/soundfont/GeneralUser-GS.sf2
#       Resources/assets/wallpapers/*.png
#       Frameworks/libav*.dylib, and the thirteen-library closure they pull in  (video builds only)
#       Frameworks/COPYING.*, LICENSE.md                                       (their terms)
#
# Why a bundle rather than the bare executable that `cargo build` produces. A bare Mach-O binary on
# macOS is a terminal program: it has no icon in the Dock or the Finder, it cannot be double-clicked
# out of a download, and it has no Info.plist -- so the window manager has nothing to name it and no
# `CFBundleIconFile` to draw. The icon work is the reason this script exists at all; without a bundle
# there is nowhere on macOS for an icon to live.
#
# Assets go in Contents/Resources, which is where Apple's layout says things the app reads belong;
# `Paths::discover_asset_dir` in km-app knows to look there when the executable is inside a
# Contents/MacOS. Contents/MacOS is for executables only, and a bundle that ignores that works right
# up until something tries to sign it.
#
# **Ad-hoc signed by default, and Developer ID signed if you have one.** With KM_SIGN_IDENTITY unset
# this is NOT signed and NOT notarized, so Gatekeeper refuses to open it on a machine it was not
# built on until somebody clears the quarantine attribute:
#
#   xattr -dr com.apple.quarantine "dist/karaokemachine/macos/Karaoke Machine.app"
#
# Set KM_SIGN_IDENTITY to a Developer ID Application identity and the bundle, its executable and
# every dylib in Contents/Frameworks are signed with it, with the hardened runtime. The report says
# which you got. Notarizing is the installer's job, not this script's: `tools/platform/macos/installer.sh
# --notarize` submits the .pkg, and one submission covers every bundle inside it.
#
# Prerequisites: the Command Line Tools and CMake -- SDL3 and SDL3_ttf are built from source. See
# "Building on macOS" in docs/ARCHITECTURE.md; CMake 4 needs CMAKE_POLICY_VERSION_MINIMUM=3.5.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=app-bundle

FETCH=1
ZIP=0
VIDEO=1
for arg in "$@"; do
  case "$arg" in
    --no-fetch) FETCH=0 ;;
    --zip) ZIP=1 ;;
    --no-video) VIDEO=0 ;;
    # The build logs, which are quiet by default. A failure replays whatever was held back, so this
    # is for watching a build rather than for diagnosing one after the fact.
    -v|--verbose) DIST_VERBOSE=1 ;;
    *) echo "app-bundle: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  echo "app-bundle: this builds a macOS bundle and has to run on macOS." >&2
  echo "            The binary inside it is a Mach-O executable; there is no cross-compile here." >&2
  exit 1
fi

# -- the assets ----------------------------------------------------------------------------------

# The same release-step rule as tools/platform/windows/dist.sh: the bundle ships the instrument bank because
# the build put it there, not because somebody remembered to fetch it first.
#
# Wrapped rather than taught a flag of its own, so running it directly -- which is what CLAUDE.md
# tells somebody to do once per machine -- is unchanged.
if [ "$FETCH" -eq 1 ]; then
  dist_step assets
  dist_run "fetch-assets" tools/setup/fetch-assets.sh
fi

# -- the video feature ----------------------------------------------------------------------------

# On unless declined. `ffmpeg-sys-next` needs FFMPEG_DIR *and* LIBCLANG_PATH, and fails in ways that
# name neither if either is missing -- so they are resolved and checked here, before anything is
# built, rather than left to the build script.
#
# Resolved rather than hard-coded, and the ffmpeg half is delegated to `tools/setup/fetch-ffmpeg.sh`, which
# is the one thing that knows the order: the environment first, then the pinned LGPL build it keeps in
# the cache, then Homebrew. **Which of those a release gets built against is a licensing question, not
# just a path** -- Homebrew's ffmpeg is the GPL configuration -- so it is worth it being one decision
# in one place. libclang ships inside the Command Line Tools this build already requires, so that path
# is probed here; an FFMPEG_DIR or LIBCLANG_PATH already in the environment wins either way.
FEATURES=()
if [ "$VIDEO" -eq 1 ]; then
  if [ -z "${FFMPEG_DIR:-}" ]; then
    FFMPEG_DIR="$(dist_ffmpeg_dir_macos)"
  fi
  if [ ! -d "$FFMPEG_DIR/include" ] || [ ! -d "$FFMPEG_DIR/lib" ]; then
    echo "app-bundle: FFMPEG_DIR=$FFMPEG_DIR has no include/ and lib/ under it." >&2
    echo "            Point it at a real ffmpeg prefix, or pass --no-video." >&2
    exit 1
  fi

  if [ -z "${LIBCLANG_PATH:-}" ]; then
    for candidate in "$(xcode-select -p 2>/dev/null)/usr/lib" \
                     "$(brew --prefix llvm 2>/dev/null)/lib"; do
      if [ -f "$candidate/libclang.dylib" ]; then LIBCLANG_PATH="$candidate"; break; fi
    done
  fi
  if [ -z "${LIBCLANG_PATH:-}" ] || [ ! -f "$LIBCLANG_PATH/libclang.dylib" ]; then
    echo "app-bundle: video is on by default and needs libclang; none was found." >&2
    echo "            xcode-select --install, or set LIBCLANG_PATH to a directory holding" >&2
    echo "            libclang.dylib, or pass --no-video." >&2
    exit 1
  fi
  export FFMPEG_DIR LIBCLANG_PATH

  dist_step video
  # Which ffmpeg and which libclang a release was built against is a licensing question as much as a
  # path -- see the block above -- so both are kept, one `-v` away, rather than dropped.
  dist_detail "FFMPEG_DIR     $FFMPEG_DIR"
  dist_detail "LIBCLANG_PATH  $LIBCLANG_PATH"

  # `tray` rides with `video`, on the terms `tools/platform/windows/dist.sh` sets out: the icon is
  # switched on per carrier because its Linux backend links a library the two Linux carriers refuse,
  # and it is joined to `video` because the run that wants an icon is the streaming one.
  FEATURES=(--features video,tray)
fi

# -- the build -----------------------------------------------------------------------------------

dist_detail "signing   $(dist_signing_note)"

dist_step build
build_started=$SECONDS
# CMake 4 rejects the vendored FreeType inside SDL3_ttf. Harmless on CMake 3, so it is set
# unconditionally rather than probed for.
# shellcheck disable=SC2046  # dist_cargo_quiet prints one flag or nothing at all
CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo build --release $(dist_cargo_quiet) \
  -p karaokemachine "${FEATURES[@]+"${FEATURES[@]}"}"
printf '   built in %s\n' "$(dist_elapsed "$build_started")"
# **The build also produces `karaokemachine-console`, and this bundle deliberately ignores it.** The
# twin exists because Windows has a subsystem to choose; macOS has none, so a `.app` already has no
# terminal and a bare executable run from Terminal still prints, which makes the second file the same
# program under a second name. Naming this one explicitly is what keeps it out. See the `The machine's
# console window` decision in docs/decisions/.
BIN="$(dist_target_dir)/release/karaokemachine"
if [ ! -f "$BIN" ]; then
  echo "app-bundle: $BIN was not produced" >&2
  exit 1
fi
echo

VERSION="$(dist_version "$BIN")"

# -- the bundle ----------------------------------------------------------------------------------

# App, then platform, then the thing -- the layout rule lives in tools/dist/common.sh. The bundle
# carries no version in its own name, unlike the Windows folder: a `.app` is what the Finder shows
# and `KaraokeMachine 1.1.0.app` would read as a different application every release. The version is
# in Info.plist, which is where macOS looks for it, and in the zip's name.
MACOS_DIST="$(dist_dir karaokemachine macos)"
APP="$MACOS_DIST/Karaoke Machine.app"
CONTENTS="$APP/Contents"

# The second bundle, and what it is for is in `Info.stream.plist`: a macOS bundle carries no launch
# argument anywhere in its manifest, so a mode reached by one needs a bundle of its own. Declared
# before either is staged, because each staging sweeps the fossils of a rename out of this folder and
# would otherwise take the other for one.
#
# Not exported, and bash could not export it anyway: `common.sh` is sourced, so it reads this shell's
# own variables.
STREAM_APP="$MACOS_DIST/KM Stream.app"
DIST_MACOS_ALSO_STAGES=("$(basename "$APP")" "$(basename "$STREAM_APP")")

# The executable, the icon, Info.plist and PkgInfo -- the parts every bundle here has, which is why
# they live in tools/dist/common.sh rather than in this script. `km-package-builder` stages its own
# through the same two functions.
dist_stage_macos_bundle "$APP" tools/platform/macos/Info.plist \
                        "$BIN" karaokemachine \
                        icon/karaokemachine.icns "$VERSION"

# A bundle keeps its assets under Contents/Resources rather than beside the executable, so the shared
# helper is pointed at Resources and lands them in `Resources/assets/...`, which is where
# `Paths::discover_asset_dir` looks inside a bundle.
assets="$(dist_stage_assets "$CONTENTS/Resources")"

# The application's own terms, in Resources for the same reason ffmpeg's go there: Contents/Frameworks
# holds code and `codesign` refuses to seal a bundle with a text file in it. Staged before the seal
# below, which is what makes them part of what is signed.
dist_stage_app_licenses "$CONTENTS/Resources"

# -- ffmpeg, copied in and pointed at itself -------------------------------------------------------

# Contents/Frameworks is where Apple's layout puts libraries a bundle carries, and `@rpath` resolved
# through `@executable_path/../Frameworks` is how the executable in Contents/MacOS reaches them. Both
# are conventions rather than requirements, and both are what any tool that later signs or notarizes
# this bundle will expect to find.
DYLIBS=0
if [ "$VIDEO" -eq 1 ]; then
  DYLIBS="$(dist_stage_ffmpeg_macos "$CONTENTS/MacOS/karaokemachine" \
                                    "$CONTENTS/Frameworks" \
                                    "@executable_path/../Frameworks")"
  # Terms in Resources, libraries in Frameworks. Apple's layout says Frameworks holds code, and
  # `codesign` enforces it: a text file in there makes it refuse to seal the bundle.
  # Made here rather than left to the function: the shell opens the redirect below before the function
  # runs, so the directory has to exist first.
  mkdir -p "$CONTENTS/Resources/ffmpeg"
  dist_stage_ffmpeg_license_macos "$FFMPEG_DIR" "$CONTENTS/Frameworks" "$CONTENTS/Resources/ffmpeg" \
    > "$CONTENTS/Resources/ffmpeg/README.txt"
fi

# **Last, after everything else is in place**, and for every bundle rather than only a video one. The
# seal, the no-absolute-paths check and the Finder's icon nudge are all in here; see
# `dist_seal_macos_bundle`. Inside the `if` above, a `--no-video` bundle would be neither signed nor
# checked -- and the check is exactly what catches a stray absolute load path in a build that
# carries no ffmpeg and still links SDL.
dist_seal_macos_bundle "$APP"

# -- the same machine, started streaming -----------------------------------------------------------

# **Four lines and an icon, and it carries no copy of anything.** What it starts is the bundle beside
# it, so there is one machine, one set of assets and one set of ffmpeg libraries however it was
# started -- and nothing here to fall out of step with the bundle above.
#
# **The icon is the one thing the two bundles do not share.** They sit next to each other in
# /Applications and in the Dock, where the same mark on both would leave the Finder showing two
# entries that differ only by name; the badged mark says which of the two is the streaming one.
#
# **`open` on the bundle beside it, and never the binary inside that bundle.** A process that execs
# its way from one bundle into another keeps the identity LaunchServices launched it under and gains
# the one its new image belongs to, and the two disagree for the rest of the run. What that costs is
# not obvious and is not an error: the machine starts, streams, and answers its API, but a status
# item it creates in the menu bar is never drawn -- `NSStatusBar` hands one back and nothing appears.
# Asking LaunchServices to start the other bundle is what keeps one identity: the machine comes up as
# `Karaoke Machine.app`, which is what it is.
#
# **It also settles the question `exec` was here for.** `current_exe()` on Apple is
# `_NSGetExecutablePath` with no realpath, so a machine that finds this script anywhere in its own
# path takes neither the sibling-assets branch nor the Contents/MacOS one, and comes up on a sine
# test tone over a plain gradient with nothing on screen saying why. Launched this way it is not in
# the path at all: the kernel records the real binary at its real place.
#
# **`--args` is last, and everything after it goes to the machine.** A run started from the Finder
# passes none, so `"$@"` is usually empty; it is forwarded so that `open --args` against this bundle
# reaches the machine the way it reaches any other.
#
# **Resolved from this script's own location rather than from /Applications**, so the pair works
# unzipped into a Downloads folder as well as installed -- which is how `--zip` hands it over.
STREAM_EXE="$(mktemp)"
cat > "$STREAM_EXE" <<'LAUNCH'
#!/bin/sh
# Starts the machine in Karaoke Machine.app beside this bundle, drawing for an encoder rather than for
# a television. Staged by tools/platform/macos/app-bundle.sh; see the comment there for why it asks
# LaunchServices to start the other bundle rather than running the binary inside it.
set -eu
beside=$(cd -- "$(dirname -- "$0")/../../.." && pwd)
exec /usr/bin/open -a "$beside/Karaoke Machine.app" --args --stream "$@"
LAUNCH

dist_stage_macos_bundle "$STREAM_APP" tools/platform/macos/Info.stream.plist \
                        "$STREAM_EXE" karaokemachine-stream \
                        icon/karaokemachine-stream.icns "$VERSION"
rm -f "$STREAM_EXE"

# The licenses and nothing else: it reads no assets and links no libraries, and both of those live in
# the bundle it starts. Sealed on the same terms as the one above, which for a bundle holding no
# Mach-O is the signature and the resource seal -- `dist_verify_macho_portable` finds nothing to walk
# and says so by passing.
dist_stage_app_licenses "$STREAM_APP/Contents/Resources"
dist_seal_macos_bundle "$STREAM_APP"

# -- report --------------------------------------------------------------------------------------

echo "== $APP"
echo "   version   $VERSION"
echo "   assets    $assets file(s) in Contents/Resources/assets"
if [ "$VIDEO" -eq 1 ]; then
  echo "   video     yes -- $DYLIBS dylib(s) in Contents/Frameworks, from $FFMPEG_DIR/lib"
  # Said every time. There is no `-no-video` marker in the path to tell the two bundles apart
  # -- `--no-video` stages over the same `Karaoke Machine.app`, so the last run of this script is the
  # bundle you have -- and this line is the only thing in the output that says which one that was.
  echo "             verified: nothing loads by absolute path, so it plays video on a Mac"
  echo "             that has no ffmpeg installed."
else
  echo "   video     no  -- video songs catalog and queue, but will not play"
fi
echo "   size      $(du -sh "$APP" | awk '{print $1}')"
echo
echo "== $STREAM_APP"
echo "   starts    the machine in $(basename "$APP") with --stream"
echo "   size      $(du -sh "$STREAM_APP" | awk '{print $1}')"
echo

if [ "$ZIP" -eq 1 ]; then
  ZIPFILE="$MACOS_DIST/karaokemachine-$VERSION-macos.zip"
  dist_zip_macos_bundle "$APP" "$ZIPFILE" >/dev/null
  echo "== $ZIPFILE ($(du -h "$ZIPFILE" | awk '{print $1}'))"
  echo
fi

echo "open it with:  open \"$APP\""
echo
# Only for the build it is true of. See the note in tools/dist/cmd.sh: a signed bundle needs
# no quarantine strip, and printing the line anyway teaches somebody to run it by reflex.
if dist_signing; then
  echo "signed $(dist_signing_note)"
  echo "  Not notarized here -- a downloaded copy is still refused until it is."
  echo "  tools/platform/macos/installer.sh --notarize does that, for the .pkg and everything in it."
else
  echo "On another machine Gatekeeper will refuse it until the quarantine flag is cleared:"
  echo "  xattr -dr com.apple.quarantine \"$APP\""
fi
