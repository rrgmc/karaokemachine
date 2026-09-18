#!/usr/bin/env bash
#
# The half of tools/platform/linux/deb.sh that runs inside the build image. Not meant to be run directly.
#
# Expects /src (the source tree), /build (the cargo target dir) and /out (where the .deb goes).
# KM_VIDEO selects the optional video feature; the image already has what it needs, and deb.sh sends
# 1 unless it was told --no-video. The default here is 0 rather than 1 on purpose: this script is
# reached through deb.sh, which always sets the variable, so an unset one means somebody is running
# it by hand and the smaller build is the safer thing to give them.

set -euo pipefail

cd /src

# Sourced for the output helpers rather than for the staging ones -- this script stages nothing, and
# builds its report out of `dpkg-deb`. DIST_VERBOSE arrives from deb.sh as an environment variable,
# because this script has no argv at all: it is configured entirely by KM_* variables, so that
# `deb.sh --shell` puts you in a container that would build the same thing the automatic run did.
. tools/dist/common.sh
DIST_SCRIPT=deb-in-container

# The pin, for `$FF_SRC_ID` -- the same prefix the tarball builds into, so the two carriers share one
# ffmpeg per pin rather than building it twice into the same volume.
. tools/setup/ffmpeg-pin.sh

VIDEO="${KM_VIDEO:-0}"

# **Which ffmpeg a video package carries.** 0 bundles the LGPL build this repository makes, which is
# the default and the one that leaves an appliance with no X on it; 1 links the distribution's, which
# is what a packager rebuilding this for Debian wants and what `deb.sh --system-ffmpeg` asks for.
# Meaningless without video, and deb.sh refuses the combination rather than quietly ignoring it.
SYSTEM_FFMPEG="${KM_SYSTEM_FFMPEG:-0}"

# **Which package this run builds.** 0 is the machine; 1 is `karaokemachine-tools`, the three
# web-backed tools in one package that the machine's Recommends. They share this script because they
# share everything around the build -- the image, the shared target volume, the checkout guard and
# the report -- and differ in which manifest cargo-deb is pointed at.
TOOLS="${KM_TOOLS:-0}"

# The prefix carries the build's identity, exactly as it does for the tarball: `$FF_SRC_ID` is a hash
# of the release and the configure flags, so one pin resolves to one directory and two pins do not
# delete each other's work.
FFPREFIX="/build/ffmpeg-lgpl/$FF_SRC_ID"

# The staging directory the `bundled-ffmpeg` variant globs. Under `target/` rather than beside the
# manifest because cargo-deb resolves asset sources against the workspace root and this is build
# output, not a tracked file.
FFSTAGE="${CARGO_TARGET_DIR:-/build/target}/release/kmffmpeg"

# -- the assets ----------------------------------------------------------------------------------

# The same release-step rule as the Windows folder: the package ships the instrument bank because the
# build put it there, not because somebody remembered to fetch it. The script caches per machine, and
# /src is the developer's own checkout, so this normally prints "cached" and copies nothing.
if ! ls assets/soundfont/*.sf2 >/dev/null 2>&1; then
  dist_step "assets (none found; fetching)"
  dist_run "fetch-assets" tools/setup/fetch-assets.sh
  echo
fi

# The .deb never calls `dist_stage_assets` -- cargo-deb globs the tree straight out of
# crates/machine/karaokemachine/Cargo.toml -- so the check that every other carrier inherits from that helper has to
# be asked for here. This is the carrier the two-bank rule is really about: `assets/soundfont/*.sf2`
# would ship every bank it matched.
tools/dist/check-assets.sh

# -- the package ---------------------------------------------------------------------------------

# **This checkout's crates are cleared out of the shared cache first**, for the reason
# clean-checkout.sh gives at length: every checkout mounts at `/src`, so the volume cannot tell two of
# them apart, and the failure mode here is not a confusing compiler error but a **package built from
# another checkout's source** -- which deploy.sh would then install on the appliance, with nothing
# downstream to catch it. The tarball has done this since e14350f; the .deb is the carrier that
# actually reaches the television, so it wanted it more.
tools/platform/linux/clean-checkout.sh
echo

# cargo-deb builds --release itself, then derives Depends with dpkg-shlibdeps from what the binary
# actually links. --locked so a package build cannot silently resolve a different dependency tree
# than the one committed in Cargo.lock.
#
# `--features` is cargo-deb's own flag rather than something passed through after `--`. That matters:
# cargo-deb has to know the feature set to build with it *and* to run shlibdeps against the result,
# and this is the path where `$auto` earns its keep -- km-video links the four ffmpeg libraries
# normally, so unlike everything SDL dlopens, shlibdeps really can see them and the hand-kept list in
# Cargo.toml needs no video entries at all.
FEATURES=()
[ "$VIDEO" -eq 1 ] && FEATURES+=(--features video)

# **Which variant, and why there is one at all.** The three builds differ in what they depend on and
# what they carry, and `[package.metadata.deb]` is static TOML that a flag here cannot reach into.
# cargo-deb's variants are that mechanism; both pin `name`, so all three produce `karaokemachine`.
# A no-video build takes no variant, the section in Cargo.toml already being right for a package with
# no ffmpeg in it at all.
VARIANT=()
if [ "$VIDEO" -eq 1 ]; then
  if [ "$SYSTEM_FFMPEG" -eq 1 ]; then
    VARIANT+=(--variant system-ffmpeg)
  else
    VARIANT+=(--variant bundled-ffmpeg)
  fi
fi

# -- ffmpeg, when the package carries its own ------------------------------------------------------

# Built rather than borrowed, and the argument is tools/platform/linux/ffmpeg-lgpl.sh's: Debian's is
# GPL-2+ where this workspace is MIT OR Apache-2.0, and its libavutil has libX11, libva-x11, libvdpau
# and libOpenCL as direct NEEDED entries -- so linking it puts an X client library into the process
# on a box with no X server. The one this builds links nothing but libc and its siblings, which is
# asserted by that script rather than hoped for.
#
# PKG_CONFIG_PATH points ffmpeg-sys-next at the prefix, so the binary is compiled against exactly the
# headers of the libraries it ships with. Building against Debian's and shipping ours would probably
# work, and "probably" there means an undefined symbol at load time on somebody else's machine.
if [ "$VIDEO" -eq 1 ] && [ "$SYSTEM_FFMPEG" -eq 0 ]; then
  tools/platform/linux/ffmpeg-lgpl.sh "$FFPREFIX"
  export PKG_CONFIG_PATH="$FFPREFIX/lib/pkgconfig"
  echo
fi

dist_step "build$( [ "$VIDEO" -eq 1 ] && printf ' (video%s)' "$( [ "$SYSTEM_FFMPEG" -eq 1 ] && printf ', system ffmpeg' )" )"
build_started=$SECONDS
# Wrapped rather than given cargo-deb's own `--quiet`, which is a different flag doing a different
# job: cargo's `-q` drops its status lines and leaves rustc's diagnostics, while cargo-deb's is
# documented as "don't print warnings" -- and dpkg-shlibdeps' warnings are the ones worth keeping
# under `-v`, since they are how a missing Depends first shows itself. `dist_run` holds all of it and
# replays every line if the build fails.
# **cargo builds and cargo-deb packages what was built, for all three rather than for one.** Letting
# cargo-deb build itself is one step fewer, and it leaves nowhere to stand between compiling and
# packaging -- which is exactly where the bundled build has to rewrite the binary's RPATH. A
# `--no-build` taken by only some runs would be a second path that only some runs test, so every run
# takes it.
T="${CARGO_TARGET_DIR:-/build/target}"

if [ "$TOOLS" -eq 1 ]; then
  # **Two cargo invocations, because km-admin lives in the second workspace.** One
  # CARGO_TARGET_DIR covers both, which is what lets the asset list in
  # tools/cmd/km-package-builder/Cargo.toml name all three under `target/release`.
  dist_run "cargo build (tools)" \
    cargo build --release --locked -p km-package-builder -p km-remote "${FEATURES[@]+"${FEATURES[@]}"}"
  dist_run "cargo build (km-admin)" \
    cargo build --release --locked --manifest-path tools/cmd/assets/Cargo.toml -p km-admin

  for t in km-package-builder km-remote km-admin; do
    if [ ! -x "$T/release/$t" ]; then
      echo "deb-in-container: the build produced no $T/release/$t" >&2
      exit 1
    fi
    # **The console twin must not be here.** It is `required-features = ["desktop"]`, which a Linux
    # build never turns on -- and a twin appearing would mean the feature reached this build, which
    # is the fault `tools/setup/features.sh` exists to prevent.
    if [ -e "$T/release/$t-console" ]; then
      echo "deb-in-container: $t-console exists, so the desktop feature reached a Linux build." >&2
      exit 1
    fi
  done

  # km-package-builder is the one with `video`, so it is the one that links ffmpeg and the one whose
  # rpath has to reach the machine's copy. The other two link nothing but libc and are left alone.
  if [ "$VIDEO" -eq 1 ]; then
    patchelf --set-rpath '$ORIGIN/../lib' "$T/release/km-package-builder"
  fi
else
  # **cargo builds and cargo-deb packages what was built, for all three rather than for one.**
  # Letting cargo-deb build itself is one step fewer, and it leaves nowhere to stand between
  # compiling and packaging -- which is exactly where the bundled build has to rewrite the binary's
  # RPATH. A `--no-build` taken by only some runs would be a second path that only some runs test, so
  # every run takes it.
  dist_run "cargo build" \
    cargo build --release --locked -p karaokemachine "${FEATURES[@]+"${FEATURES[@]}"}"
fi

EXE="$T/release/karaokemachine"
if [ "$TOOLS" -eq 0 ] && [ ! -x "$EXE" ]; then
  echo "deb-in-container: the build produced no $EXE" >&2
  exit 1
fi

# -- the libraries, when the package carries them ---------------------------------------------------

# The machine's package carries the libraries; the tools package reaches them through it, so this
# whole block is the machine's.
if [ "$TOOLS" -eq 0 ] && [ "$VIDEO" -eq 1 ] && [ "$SYSTEM_FFMPEG" -eq 0 ]; then
  # Cleared rather than written over, so a pin change cannot leave last pin's soname behind for the
  # glob to find and ship beside the new one.
  rm -rf "$FFSTAGE"
  mkdir -p "$FFSTAGE"
  # Copied under their sonames as real files rather than as the versioned file plus a symlink chain:
  # DT_NEEDED records the soname and nothing looks for the longer name, and dpkg would have to own
  # every link in the chain for the sake of pointing at the file beside it.
  for lib in avcodec avformat avutil swresample; do
    real="$(readlink -f "$FFPREFIX/lib/lib$lib.so")"
    soname="$(patchelf --print-soname "$real")"
    cp "$real" "$FFSTAGE/$soname"
    chmod 644 "$FFSTAGE/$soname"
    # $ORIGIN is resolved against the object's own directory, so the package can be relocated and the
    # libraries still find each other. Two separate rewrites because RUNPATH -- which is what a modern
    # linker emits -- is *not* inherited by an object's own dependencies: the binary finding lib/ says
    # nothing about libavformat finding libavcodec beside it.
    patchelf --set-rpath '$ORIGIN' "$FFSTAGE/$soname"
  done
  # libopenh264, which libavcodec links for the one thing ffmpeg cannot do itself: encode H.264, which
  # is what a machine streaming its screen uses. Named separately because it is the one library here
  # whose name is not libav*, and taken from the prefix rather than the system for the reason the
  # tarball gives -- ffmpeg-lgpl.sh put it there, so the prefix is the single description of what a
  # video build carries. Missing it ships a libavcodec that cannot load.
  for real in "$FFPREFIX"/lib/libopenh264.so.*; do
    soname="$(patchelf --print-soname "$real")"
    cp "$real" "$FFSTAGE/$soname"
    chmod 644 "$FFSTAGE/$soname"
    patchelf --set-rpath '$ORIGIN' "$FFSTAGE/$soname"
  done
  patchelf --set-rpath '$ORIGIN/lib' "$EXE"

  # **The licence text travels with the libraries, because shipping them is what makes it required.**
  # The tarball puts these in LICENSES/; a package's home for them is its own doc directory. Both are
  # written by ffmpeg-lgpl.sh into the prefix, so the text and the version it describes cannot drift.
  cp "$FFPREFIX/COPYING.LGPLv2.1" "$FFSTAGE/ffmpeg-COPYING.LGPLv2.1.txt"
  cp "$FFPREFIX/openh264-COPYRIGHT" "$FFSTAGE/openh264-LICENSE.txt"
  chmod 644 "$FFSTAGE"/ffmpeg-COPYING.LGPLv2.1.txt "$FFSTAGE"/openh264-LICENSE.txt
fi

if [ "$TOOLS" -eq 1 ]; then
  # `-p km-package-builder` is where the metadata lives; the package it produces is called
  # `karaokemachine-tools` and holds all three binaries. No variant: this package has one shape.
  dist_run "cargo deb" \
    cargo deb --no-build -p km-package-builder \
      "${FEATURES[@]+"${FEATURES[@]}"}" --output /out
else
  dist_run "cargo deb" \
    cargo deb --no-build -p karaokemachine \
      "${FEATURES[@]+"${FEATURES[@]}"}" "${VARIANT[@]+"${VARIANT[@]}"}" --output /out
fi
printf '   built in %s\n' "$(dist_elapsed "$build_started")"

# -- report --------------------------------------------------------------------------------------

deb=$(ls -t /out/*.deb | head -1)
echo
echo "== $deb"
dpkg-deb --field "$deb" Package Version Architecture Installed-Size Depends

# Asserted rather than eyeballed. A video package and a plain one have the same name, the same
# version and the same file name -- the only thing that distinguishes them from the outside is what
# they Depend on. If the feature had silently not been applied, this is the difference, and shipping
# a package called "video" that cannot play one is exactly the failure worth refusing to produce.
#
# Asserted in both directions since video became the default. The one that matters most is still the
# first -- a package that says video and cannot play one -- but the second is no longer paranoia: the
# unusual build is now the one somebody asked for specially, and a `no-video/` package that quietly
# came out with ffmpeg in it would be the same mistake pointing the other way.
# **Where ffmpeg shows itself moved with the bundling**, and the assertion followed it. A package
# carrying its own libraries Depends on none, so `Depends` says nothing about whether video reached
# the binary; what says it is `lib/libavcodec.so.*` in the package's own contents. The system build
# is the one still answered by `Depends`, and it is asserted the other way round so that a run which
# silently took the wrong variant cannot pass either test.
#
# **Each match captures its producer's output and searches it afterwards**, which is the rule
# tools/dist/common.sh spells out and which this file has to follow for the same reason: these
# scripts run under `pipefail`, and a `grep -q` that matches kills its producer with SIGPIPE, so the
# pipeline's 141 makes the `if` read a match as no match. Over `dpkg-deb --contents` of a
# seventy-megabyte package the producer loses that race every time, and the direction is the danger:
# every check here would wave through exactly what it exists to stop.
deb_contents="$(dpkg-deb --contents "$deb")"
deb_depends="$(dpkg-deb --field "$deb" Depends)"

deb_carries() { # <soname regex> -> 0 if the package ships opt/karaokemachine/lib/<that>
  printf '%s\n' "$deb_contents" | grep -qE "opt/karaokemachine/lib/$1"
}
deb_depends_ffmpeg() { printf '%s\n' "$deb_depends" | grep -q libavcodec; }

if [ "$TOOLS" -eq 1 ]; then
  # **What this package is, asserted rather than assumed.** Three binaries, no libraries of its own,
  # and a Depends on the machine -- which is what makes `$ORIGIN/../lib` resolve to anything.
  for t in km-package-builder km-remote km-admin; do
    if ! printf '%s\n' "$deb_contents" | grep -qE "opt/karaokemachine/tools/$t\$"; then
      echo >&2
      echo "deb-in-container: the tools package carries no $t." >&2
      exit 1
    fi
  done
  if ! printf '%s\n' "$deb_depends" | grep -q 'karaokemachine'; then
    echo >&2
    echo "deb-in-container: the tools package does not Depend on karaokemachine." >&2
    echo "              km-package-builder's rpath reaches its ffmpeg through that package." >&2
    exit 1
  fi
  if deb_carries '.'; then
    echo >&2
    echo "deb-in-container: the tools package carries libraries of its own." >&2
    echo "              It is meant to share the machine's. Refusing to ship it." >&2
    exit 1
  fi
  echo
  echo "-- tools: three binaries, no libraries, and a Depends on the machine that holds them --"
elif [ "$VIDEO" -eq 1 ] && [ "$SYSTEM_FFMPEG" -eq 0 ]; then
  if ! deb_carries 'libavcodec\.so\.'; then
    echo >&2
    echo "deb-in-container: KM_VIDEO=1 but the package carries no opt/karaokemachine/lib/libavcodec.so.*" >&2
    echo "              The video feature did not reach the binary, or the staging step did not run." >&2
    echo "              Refusing to ship it." >&2
    exit 1
  fi
  if deb_depends_ffmpeg; then
    echo >&2
    echo "deb-in-container: the package carries its own ffmpeg and Depends on the distribution's." >&2
    echo "              Something put \$auto back. That is the dependency the bundling removes." >&2
    exit 1
  fi
  # **Named as its own failure, because it is the one that reads as something else.** libavcodec links
  # libopenh264 for the H.264 encoder a streaming run uses; a package carrying the four libav
  # libraries and not this one installs cleanly and fails at the loader, which looks like a broken
  # build rather than a missing file.
  if ! deb_carries 'libopenh264\.so\.'; then
    echo >&2
    echo "deb-in-container: the package carries ffmpeg and no libopenh264." >&2
    echo "              libavcodec links it, so this package's own libavcodec will not load." >&2
    exit 1
  fi
  echo
  echo "-- video: the package carries its own ffmpeg and openh264, and Depends on none --"
elif [ "$VIDEO" -eq 1 ]; then
  if ! deb_depends_ffmpeg; then
    echo >&2
    echo "deb-in-container: KM_SYSTEM_FFMPEG=1 but the package does not Depend on libavcodec." >&2
    echo "              The video feature did not reach the binary. Refusing to ship it." >&2
    exit 1
  fi
  if deb_carries 'libavcodec\.so\.'; then
    echo >&2
    echo "deb-in-container: the system-ffmpeg package carries ffmpeg libraries of its own." >&2
    echo "              This build is the one that ships none. Refusing to ship it." >&2
    exit 1
  fi
  echo
  echo "-- video: the package Depends on the distribution's ffmpeg and carries none --"
else
  if deb_depends_ffmpeg || deb_carries 'libavcodec\.so\.'; then
    echo >&2
    echo "deb-in-container: KM_VIDEO=0 but the package reaches ffmpeg all the same." >&2
    echo "              This is a video build wearing the --no-video name. Refusing to ship it." >&2
    exit 1
  fi
  echo
  echo "-- no video: the package Depends on no ffmpeg and carries none --"
fi

# **The requirement, stated as a check.** The appliance must install with no X library on the box,
# and what would silently undo that is a link-time dependency appearing in the binary -- a crate that
# links X, or an ffmpeg that was borrowed rather than built. So the shipped binary's DT_NEEDED is read
# against an allowlist rather than trusted, which is the same shape ffmpeg-lgpl.sh uses on its own
# output. The system build is exempt by construction: Debian's libavutil pulls libX11 in and saying so
# is that build's whole point.
#
# **The tools are read the same way, and gtk is on their list rather than X.** A Linux build of any
# of the three has no window at all, so `libgtk`, `libwebkit` or `libayatana` appearing in one would
# mean the `desktop` feature reached a Linux build -- the fault `tools/setup/features.sh` exists to
# prevent, and one that shows up as a program that will not start rather than as a build error.
if [ "$TOOLS" -eq 1 ]; then
  CHECK=("$T/release/km-package-builder" "$T/release/km-remote" "$T/release/km-admin")
else
  CHECK=("$EXE")
fi

if [ "$SYSTEM_FFMPEG" -eq 0 ]; then
  bad=0
  for obj in "${CHECK[@]}"; do
    while read -r dep; do
      case "$dep" in
        libX*|libxcb*|libwayland*|libva*|libvdpau*|libOpenCL*|libgtk*|libwebkit*|libayatana*|libsoup*)
          echo "deb-in-container: $(basename "$obj") links $dep" >&2
          bad=1
          ;;
      esac
    done < <(objdump -p "$obj" | awk '/NEEDED/ {print $2}' | sort -u)
  done
  if [ "$bad" -eq 1 ]; then
    echo >&2
    echo "              A package whose binary links these cannot be installed without them, so an" >&2
    echo "              appliance would carry X whatever Recommends says. Refusing to ship it." >&2
    exit 1
  fi
  echo "-- nothing shipped links X, Wayland, VA-API, VDPAU, OpenCL, gtk or webkit --"
fi
# Twenty lines of directory listing, and the one thing a reader wants from it -- did the assets go in
# -- is already implied by the byte total below and asserted by tools/platform/linux/verify-deb.sh. So it is
# kept back with the build logs: it answers "what is in there", which is a question you go looking
# for, rather than "what was produced", which the report above already says.
if dist_verbose; then
  echo
  echo "-- contents (directories collapsed) --"
  dpkg-deb --contents "$deb" | awk '{print $1, $NF}' | sed 's|/[^/]*$|/|' | sort -u | head -20
fi
echo
printf -- "-- %s bytes (~%s MiB) --\n" "$(stat -c %s "$deb")" "$(( $(stat -c %s "$deb") / 1024 / 1024 ))"
echo
echo "install with:  sudo apt-get install ./$(basename "$deb")"
