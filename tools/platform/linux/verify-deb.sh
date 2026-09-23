#!/usr/bin/env bash
#
# Installs the built .deb in a clean container and checks it actually works.
#
#   tools/platform/linux/verify-deb.sh
#   tools/platform/linux/verify-deb.sh --no-video       # the package from the no-video/ subfolder instead
#   tools/platform/linux/verify-deb.sh --system-ffmpeg  # ...from system-ffmpeg/
#
# With no argument this verifies the package tools/platform/linux/deb.sh produces by default, which is the
# video one -- and that is the more valuable test of the two, because it is the build whose Depends
# apt has to satisfy from the archive. `--no-video` points at the other folder.
#
# Deliberately a *clean* image rather than the build image: the build image has the whole toolchain
# and every dev library in it, so a package with a missing dependency would install and run there and
# fail on a real machine. Here apt has to satisfy Depends from the archive, which is the real test of
# the $auto field.
#
# What this can and cannot prove. It proves the layout, that the /usr/bin symlink works, and -- the
# thing most likely to be silently wrong -- that asset discovery resolves through that symlink to
# /opt/karaokemachine/assets. It cannot prove the instrument bank is *opened*: a container has no
# sound device, so the engine reports "no audio output" and never gets as far as loading the bank.
# That step needs a real Debian machine with a sound card.
#
# **And it proves the box gets no X**, which is the requirement the default package exists to meet:
# a machine under a television draws through kmsdrm and opens no X connection ever, so an install
# that leaves X client libraries behind has failed whatever it can still run. That is counted rather
# than argued, because the ways X arrives are transitive -- Debian's libavutil has libX11 as a direct
# NEEDED, which is why the package carries its own ffmpeg. `--system-ffmpeg` asserts the mirror
# image, so a run pointed at the wrong folder cannot pass the wrong test.
#
# **And it proves the appliance service ships disabled**, which is the claim with the widest blast
# radius and was the one thing here nothing checked. `What the machine *is*, on Linux` in
# docs/decisions/distribution.md turns on it, README.md now promises it to anybody installing the package, and the
# whole of what enforces it is a comment in debian/postinst saying not to enable -- one
# `[package.metadata.deb.systemd-units]` section in Cargo.toml would have cargo-deb generate the
# enable fragments, and nothing would have said so. The symptom is not subtle on the machine it
# happens to (a desktop that loses tty1 on the next boot) and completely invisible here, which is
# the combination a verifier is for.
#
# It is checked on the filesystem rather than with `systemctl`, and that is not a workaround: a
# unit's enabled state *is* a symlink under a `.wants` directory, so the file is the fact and
# `systemctl is-enabled` is a reading of it. debian:13-slim has no `systemctl` at all -- verified,
# not assumed -- and a container has no running systemd to answer even if it did.
#
# **Two things are cached between runs, and neither of them is a package.** That distinction is the
# whole design here, so it is worth being explicit about, because the obvious speed-up is the one
# thing that must not be done.
#
# This script asserts *two* properties, and the second is the one people forget:
#
#   1. apt can satisfy the package's Depends from the archive -- a wrong or nonexistent package name
#      is an error here rather than a mystery on somebody's machine. Hence `apt-get install` on the
#      .deb rather than `dpkg -i`.
#   2. the Depends list is **complete** -- because the image starts with nothing, an *omitted*
#      library makes `karaokemachine --version` fail below. This is why a clean image is used and not
#      the build image.
#
# Property 2 does not survive prewarming the Depends closure. Pre-install libx11-6, libasound2t64 and
# the ffmpeg libraries and an omitted Depends becomes invisible -- and an omitted Depends is
# precisely what `$auto`/dpkg-shlibdeps can get wrong, since it cannot see what SDL dlopens. That is
# why crates/machine/karaokemachine/Cargo.toml keeps a hand-written list beside `$auto` at all.
#
# **So the closure is never prewarmed. Not as a mode, not behind a flag, not "just for iteration".**
# A step called `verify` that cannot detect a missing dependency is worse than no step, because it
# consumes the attention a real check would have got.
#
# What *is* cached installs nothing, and so touches neither property:
#
#   * **the apt index**, as a one-line derivative image (`FROM debian:13-slim` + `apt-get update`).
#     The `apt-get update` below is kept: against a populated lists directory apt does conditional
#     GETs and usually transfers nothing, so this is the saving with none of the staleness that
#     pinning a snapshot would bring.
#   * **the downloaded .deb files**, in a named volume over /var/cache/apt/archives. The closure
#     changes rarely between builds, and re-downloading it into Docker's overlay was costing more
#     than the resolution it was proving.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=verify

# runtime-deps.sh is sourced only because verify-image.sh's other kinds need it; the `index` kind
# this script uses installs nothing and reads no list.
. tools/platform/linux/runtime-deps.sh
. tools/platform/linux/verify-image.sh

export MSYS2_ARG_CONV_EXCL='*'

BASE="debian:13-slim"           # the base the package targets; see the note at the docker run below
APT_CACHE="karaokemachine-apt-cache"

VIDEO=1
SYSTEM_FFMPEG=0
TOOLS=0
REFRESH=""
for arg in "$@"; do
  case "$arg" in
    --no-video) VIDEO=0 ;;
    --system-ffmpeg) SYSTEM_FFMPEG=1 ;;   # the package from the system-ffmpeg/ subfolder
    --tools) TOOLS=1 ;;                   # karaokemachine-tools, installed beside the machine
    --refresh) REFRESH="--refresh" ;;   # rebuild the index image against the archive
    *) echo "verify: unknown option $arg" >&2; exit 2 ;;
  esac
done
if [ "$VIDEO" -eq 0 ] && [ "$SYSTEM_FFMPEG" -eq 1 ]; then
  echo "verify: --system-ffmpeg needs video; deb.sh refuses to build that combination." >&2
  exit 2
fi

# Mirrors tools/platform/linux/deb.sh: the declined build is the one in a subfolder, because both files carry
# the same name. `ls` here is not recursive, so the default really does pick the ordinary package and
# not whichever of the two happens to be newest.
OUT="$(dist_dir karaokemachine linux)"
[ "$TOOLS" -eq 1 ] && OUT="$(dist_dir karaokemachine-tools linux)"
[ "$VIDEO" -eq 0 ] && OUT="$OUT/no-video"
[ "$SYSTEM_FFMPEG" -eq 1 ] && OUT="$OUT/system-ffmpeg"

# **The tools package cannot be installed alone**, and that is the design rather than a limitation:
# it Depends on `karaokemachine` for the ffmpeg its builder links. So the machine's package has to be
# in the same folder apt is pointed at, and a missing one is a clearer failure here than an
# unresolvable Depends inside the container.
MACHINE_DEB=""
MACHINE_MOUNT=()
if [ "$TOOLS" -eq 1 ]; then
  MACHINE_DEB=$(ls -t "$(dist_dir karaokemachine linux)"/*.deb 2>/dev/null | head -1 || true)
  if [ -z "$MACHINE_DEB" ]; then
    echo "verify: --tools needs the machine's .deb beside it -- run tools/platform/linux/deb.sh first" >&2
    exit 1
  fi
  # A second read-only mount rather than copying: the two packages live in folders of their own, and
  # apt has to be handed both in one call so it can satisfy the Depends from files rather than from
  # an archive that has never heard of either.
  MACHINE_MOUNT=(-v "$(host_path "$PWD/$(dist_dir karaokemachine linux)")":/machine-deb:ro)
fi
deb=$(ls -t "$OUT"/*.deb 2>/dev/null | head -1 || true)
if [ -z "$deb" ]; then
  echo "verify: no .deb in $OUT -- run tools/platform/linux/deb.sh $( [ "$VIDEO" -eq 0 ] && printf -- '--no-video ' )$( [ "$SYSTEM_FFMPEG" -eq 1 ] && printf -- '--system-ffmpeg ' )first" >&2
  exit 1
fi
echo "verifying $(basename "$deb")"
echo

# The script is collected into a variable and passed as an argument rather than piped to `bash -s`,
# and that is a bug fix rather than a style preference. Piped, the script *is* the container's stdin,
# and bash reads it a line at a time as it runs -- so any command that reads stdin eats the rest of
# the script. `apt-get install` does exactly that. The symptom is the worst kind: everything up to
# and including the install runs, every check after it silently never happens, and the whole thing
# **exits 0**, because bash reaching the end of its input is a successful end of script. It looked
# like a passing verification that had merely gone quiet. Passing the script as an argument leaves
# stdin free for whatever wants it, and there is then no reason for `-i` either.
script=$(cat <<'SCRIPT'
set -e
deb="$1"
system_ffmpeg="$2"
tools="${3:-0}"
machine_deb="${4:-}"

echo "== install"
# Official Debian images ship /etc/apt/apt.conf.d/docker-clean, whose DPkg::Post-Invoke deletes
# /var/cache/apt/archives/*.deb after every install -- which would empty the cache volume on the way
# out and make it pointless. Removing it affects caching only and never resolution: apt still asks
# the archive what satisfies Depends, and still fails here if something does not.
rm -f /etc/apt/apt.conf.d/docker-clean

# Kept, and cheap: against the prewarmed index this is conditional GETs that usually transfer
# nothing. Dropping it would pin a snapshot of the archive, which is exactly the staleness the
# index image is designed *not* to introduce.
apt-get update -qq
# apt rather than dpkg -i, so Depends are resolved from the archive and a missing one is an error
# here rather than a mystery on somebody's machine.
#
# **`--no-install-recommends`, which is what a deploy passes and what makes this the appliance's
# install rather than a desktop's.** It is also the stricter reading of property 2 above: a library
# the machine genuinely needs cannot hide in Recommends and be installed here by accident. A desktop
# install differs from this one by taking Recommends, which apt does without being asked.
if [ "$tools" = "1" ]; then
  # Both in one call, so apt satisfies `Depends: karaokemachine` from the file beside it.
  # `--no-install-recommends` because the machine Recommends the very package being installed, and a
  # desktop's worth of X libraries would tell us nothing here.
  apt-get install -y -qq --no-install-recommends "/machine-deb/$machine_deb" "/debs/$deb" 2>&1 | tail -5
  echo

  echo "== the four names are on PATH =="
  for t in km-package-builder km-package-simple km-remote km-admin; do
    link="/usr/bin/$t"
    target="/opt/karaokemachine/tools/$t"
    if [ ! -L "$link" ]; then echo "  FAIL: $link is not a symlink"; exit 1; fi
    if [ "$(readlink "$link")" != "$target" ]; then
      echo "  FAIL: $link points at $(readlink "$link"), not $target"; exit 1
    fi
    echo "  ok   $t -> $target"
  done
  echo

  echo "== the builder finds the machine's ffmpeg (the load-bearing check) =="
  # This is what `$ORIGIN/../lib` buys, and the only thing that proves it: km-package-builder links
  # the four ffmpeg libraries and carries none, so if the rpath does not resolve into the machine's
  # package it does not start at all.
  km-package-builder --version
  # **The path is canonicalised before it is judged.** `ldd` prints the rpath as written, so what
  # comes back is `/opt/karaokemachine/tools/../lib/libavcodec.so.61` -- the right file under a
  # spelling no literal match would accept. Comparing the text rather than the file is how a check
  # comes to fail on a package that is correct.
  resolved=$(ldd /opt/karaokemachine/tools/km-package-builder | grep -F 'libavcodec' || true)
  if [ -z "$resolved" ]; then
    echo "  FAIL: the builder links no libavcodec, so this is not a video build"; exit 1
  fi
  real=$(readlink -f "$(printf '%s' "$resolved" | sed -E 's/.*=> ([^ ]+).*/\1/')")
  case "$real" in
    /opt/karaokemachine/lib/libavcodec.so.*)
      echo "  PASS: resolved to $real, which the machine's package owns" ;;
    *)
      echo "  FAIL: libavcodec resolved outside the machine's package: $real"; exit 1 ;;
  esac
  echo

  echo "== the other three run =="
  km-package-simple --version
  km-remote --version
  km-admin --version
  echo

  echo "== removing the tools leaves the machine, and takes the names with them =="
  apt-get remove -y -qq karaokemachine-tools >/dev/null 2>&1
  for t in km-package-builder km-package-simple km-remote km-admin; do
    if [ -e "/usr/bin/$t" ]; then echo "  FAIL: /usr/bin/$t survived the removal"; exit 1; fi
  done
  if [ ! -x /usr/bin/karaokemachine ]; then echo "  FAIL: removing the tools took the machine too"; exit 1; fi
  echo "  PASS: the four names are gone and karaokemachine is still installed"
  echo
  echo "== every check ran"
  exit 0
fi

apt-get install -y -qq --no-install-recommends "/debs/$deb" 2>&1 | tail -5
echo

echo "== what dpkg thinks it installed"
dpkg -L karaokemachine | grep -vE '^/(usr|opt)$' | sort
echo
echo "== derived dependencies"
dpkg-query -W -f='${Depends}\n' karaokemachine | tr ',' '\n' | sed 's/^ */  /'
echo

echo "== the /usr/bin entry"
ls -l /usr/bin/karaokemachine
karaokemachine --version
echo

echo "== asset discovery (the load-bearing check)"
karaokemachine --show-paths
assets=$(karaokemachine --show-paths | awk '/^assets/ {print $2}')
case "$assets" in
  /opt/karaokemachine/assets)
    if [ -f "$assets/soundfont/GeneralUser-GS.sf2" ]; then
      echo "  PASS: resolved through the symlink, and the bank is there"
    else
      echo "  FAIL: right directory, no SoundFont in it"; exit 1
    fi ;;
  *) echo "  FAIL: expected /opt/karaokemachine/assets, got '$assets'"; exit 1 ;;
esac
echo

echo "== no display-server stack on the box (the appliance's whole requirement)"
# **The one assertion this file exists for now.** A box under a television runs kmsdrm and opens no X
# connection ever, and an install that puts X client libraries on it anyway is the thing `What the
# machine *is*, on Linux` says the deployment is not. Counted rather than reasoned about, because the
# ways X arrives are transitive: Debian's libavutil has libX11 as a direct NEEDED, so the package
# carrying its own ffmpeg is what makes this number reachable at all.
#
# The system-ffmpeg build asserts the mirror image. It links the distribution's libavutil, so X *must*
# be here -- and a run that silently verified the wrong folder would otherwise pass the wrong test.
#
# **`libX11` itself is not the test, and cannot be.** Mesa's EGL and gallium packages -- which kmsdrm
# draws through and cannot be declined -- hard-depend on `libx11-xcb1`, so `libX11` and the `libxcb`
# family arrive on any box that renders at all. What the requirement is really about is whether the
# box carries a *display-server stack*, and the answer to that is these eight: SDL's x11 driver opens
# them by soname, and without them it cannot start whatever else is present.
backend=""
for lib in libXext.so.6 libXrandr.so.2 libXcursor.so.1 libXi.so.6 \
           libXfixes.so.3 libXss.so.1 libXtst.so.6 libXrender.so.1; do
  found="$(find /usr/lib -name "$lib" 2>/dev/null | head -1)"
  [ -n "$found" ] && backend="$backend $lib"
done
# The acceleration stacks are the other half, and they are the ones Debian's ffmpeg brought: a box
# carrying libva or libvdpau is a box linking a libavutil this package is supposed to have replaced.
accel=""
for lib in libva.so.2 libva-x11.so.2 libva-drm.so.2 libvdpau.so.1 libOpenCL.so.1; do
  found="$(find /usr/lib -name "$lib" 2>/dev/null | head -1)"
  [ -n "$found" ] && accel="$accel $lib"
done

if [ "$system_ffmpeg" = "1" ]; then
  if [ -z "$accel" ]; then
    echo "  FAIL: the system-ffmpeg package brought no libva or libvdpau, so it is not linking Debian's ffmpeg"
    exit 1
  fi
  echo "  PASS:$accel, which is what linking the distribution's ffmpeg costs"
else
  if [ -n "$backend" ]; then
    echo "  FAIL: the box carries SDL's X11 backend libraries, so something asked for X:$backend"
    exit 1
  fi
  if [ -n "$accel" ]; then
    echo "  FAIL: the box carries video-acceleration libraries this package should not reach:$accel"
    echo "  That is Debian's libavutil. See the DT_NEEDED allowlist in deb-in-container.sh."
    exit 1
  fi
  echo "  PASS: no X11 backend library and no acceleration stack"
  echo "        (libX11 and libxcb are present through Mesa, which kmsdrm draws through)"
fi
echo

echo "== the appliance service ships disabled"
unit=/lib/systemd/system/karaokemachine.service
if [ ! -f "$unit" ]; then
  echo "  FAIL: no unit at $unit -- the package should carry it, just not switch it on"; exit 1
fi
# Enabling a unit is exactly this symlink: `systemctl enable` writes one, and so would the
# deb-systemd-helper fragments cargo-deb generates for a package that asks for systemd integration.
# So an empty result here is the assertion, and any hit is the regression -- named rather than
# counted, because which target it was linked into is the first thing anybody would want to know.
links=$(find /etc/systemd/system -name 'karaokemachine.service' 2>/dev/null || true)
if [ -n "$links" ]; then
  echo "  FAIL: installing the package ENABLED the service. Linked from:"
  echo "$links" | sed 's/^/    /'
  echo "  See 'What the machine *is*, on Linux' in docs/decisions/distribution.md: the package"
  echo "  ships this unit"
  echo "  disabled, and enabling it is the appliance decision rather than a consequence of"
  echo "  installing an application."
  exit 1
fi
echo "  PASS: unit installed at $unit, and nothing links it into a target"
echo

echo "== the boot splash ships unselected"
# The same claim as the unit above, one layer earlier in the boot, and it fails the same way if
# anybody ever "helpfully" makes postinst run plymouth-set-default-theme: installing a karaoke
# player would repaint a stranger's boot and rebuild their initramfs.
#
# The logo is checked separately from the two text files because it is the one that arrives by a
# different route -- it is icon/icon-1024.png renamed by the asset list, so a generator that stops
# producing 1024 breaks *this* and nothing else, and the splash it breaks is a bare ground with no
# mark on it.
theme=/usr/share/plymouth/themes/karaokemachine
for f in karaokemachine.plymouth karaokemachine.script logo.png; do
  if [ ! -f "$theme/$f" ]; then
    echo "  FAIL: $theme/$f is missing from the package"; exit 1
  fi
done
# ImageDir is an absolute path written into the theme by hand, and Plymouth resolves Image("logo.png")
# against it -- so this is the one line that catches the theme and the asset list disagreeing about
# where the theme lives.
if ! grep -qx "ImageDir=$theme" "$theme/karaokemachine.plymouth"; then
  echo "  FAIL: the theme's ImageDir is not $theme, so it will find no logo"; exit 1
fi
# The selection lives in /etc, which is what makes it a decision rather than a file. Plymouth is not
# installed here at all -- it is a Suggests -- so this directory should not even exist; if some
# future maintainer script creates it, this says so before a box does.
if [ -e /etc/plymouth/plymouthd.conf ] || [ -L /usr/share/plymouth/themes/default.plymouth ]; then
  echo "  FAIL: installing the package SELECTED a boot theme."
  echo "  See 'What the box shows before the machine does' in docs/decisions/distribution.md:"
  echo "  the package ships the theme, and tools/platform/linux/appliance-boot.sh selects it."
  exit 1
fi
echo "  PASS: theme installed at $theme, and nothing has selected it"
echo

echo "== the power-button drop-in ships inert"
# The same assertion as the one above, over a file that has no "shipped but off" state of its own.
# A unit on disk does nothing until something enables it; a file under /etc/systemd/logind.conf.d/
# is live the moment it exists -- so for this one, *where it is* is the whole of the decision.
# Installing the application on somebody's desktop must not repurpose that desktop's power button.
inert=/opt/karaokemachine/logind-powerkey.conf
if [ ! -f "$inert" ]; then
  echo "  FAIL: no drop-in at $inert -- the package should carry it for deploy.sh to install"; exit 1
fi
live=$(find /etc/systemd/logind.conf.d -name '*karaokemachine*' 2>/dev/null || true)
if [ -n "$live" ]; then
  echo "  FAIL: installing the package CHANGED the power button. Found:"
  echo "$live" | sed 's/^/    /'
  echo "  See \"The appliance's power button is a deploy decision, not an install one\" in"
  echo "  docs/decisions/distribution.md: the package ships the text, and deploy.sh is what puts"
  echo "  it into /etc."
  exit 1
fi
echo "  PASS: drop-in carried at $inert, and nothing is live in logind.conf.d"
echo

echo "== it starts (headless, no sound device in a container)"
mkdir -p /tmp/km
# Reports whichever of the three audio outcomes it reached, rather than asserting one: "no audio
# output" is the honest and expected answer in a container, "SoundFont:" would be a bonus, and
# "no SoundFont" would mean asset discovery lied.
# No RUST_LOG: every line grepped for below is info or warn, so the shipped default finds them all.
# Setting one here would test a configuration nobody runs.
timeout 8 karaokemachine --headless --data-dir /tmp/km 2>&1 \
  | grep -E "SoundFont:|no SoundFont|no audio output|the API is listening|wallpaper" || true

# Said out loud so that a run which stops early cannot be mistaken for one that passed -- the failure
# this script had until now was precisely a silent, successful-looking truncation.
echo
echo "== every check ran"
SCRIPT
)

# The same base image the package was built for -- anything newer would not prove the glibc floor --
# plus an apt index and **nothing else**. See the header for why that is the only thing safe to bake
# in, and why the Depends closure is deliberately not.
RUN_IMAGE="$(verify_image_ensure index "$BASE" "$REFRESH")" || exit 1

# The cache volume holds downloaded .deb files, not installed packages, so the container still starts
# with nothing installed and both properties in the header hold. Concurrent runs serialize on apt's
# own lock over the shared directory.
docker volume create "$APT_CACHE" >/dev/null

echo "   image   $RUN_IMAGE ($BASE plus an apt index)"
echo "   cache   $APT_CACHE (downloaded .debs only; nothing is pre-installed)"
echo

docker run --rm \
  -v "$(host_path "$PWD/$OUT")":/debs:ro \
  "${MACHINE_MOUNT[@]+"${MACHINE_MOUNT[@]}"}" \
  -v "$APT_CACHE":/var/cache/apt/archives \
  "$RUN_IMAGE" bash -c "$script" verify-deb "$(basename "$deb")" "$SYSTEM_FFMPEG" \
    "$TOOLS" "$( [ -n "$MACHINE_DEB" ] && basename "$MACHINE_DEB" )"

echo
echo "verified $(basename "$deb")"
